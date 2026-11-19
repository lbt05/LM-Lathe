//! Runtime of the language models.
//!
//! The language models are served by llama.cpp: the runtime starts the
//! server for a GGUF checkpoint and forwards the requests of the gateway to
//! it. The server speaks the OpenAI API, so the gateway offers the same
//! routes for the chat and the plain completions.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use ai_scheduler::{evictor_from_fn, Evictor, MemoryLease, Scheduler, TenantKind, TenantState};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::active_request::ActiveRequest;
use crate::engine_assets::EngineAssetManager;
use crate::error::{TtsError, TtsResult};
use crate::registry::TTSModelRegistry;
use crate::types::RuntimeState;

/// How long the runtime waits for the server to answer.
const HEALTH_TIMEOUT: Duration = Duration::from_secs(180);

/// How long a request may take before the runtime gives up on it.
///
/// A request that is never answered would keep the tenant busy forever, and
/// a busy tenant is never taken away by the scheduler: its memory would stay
/// booked and no bigger model would fit any more.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(600);

/// State of one language model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmRuntime {
    /// Model this runtime serves.
    pub model_id: String,
    /// Engine that hosts the model.
    pub engine_id: String,
    /// Lifecycle state.
    pub state: RuntimeState,
    /// Pinned runtimes are never unloaded automatically.
    pub pinned: bool,
    /// Process id of the server.
    pub process_id: Option<u32>,
    /// Base URL of the server.
    pub endpoint: Option<String>,
    /// Last time the model was used.
    pub last_used_at: Option<DateTime<Utc>>,
    /// Number of in-flight requests.
    pub active_requests: u32,
    /// Memory the runtime is estimated to need.
    pub estimated_memory: u64,
    /// Memory the runtime actually uses.
    pub actual_memory: Option<u64>,
    /// The size of the weights on the disk.
    ///
    /// The engines map the checkpoint into their address space instead of
    /// copying it, so the memory of the process does not hold the weights
    /// until they are used. This is the number that tells how big the model
    /// really is.
    #[serde(default)]
    pub checkpoint_bytes: Option<u64>,
    /// The name the server knows the model by.
    ///
    /// It is the name of the catalogue for llama.cpp, and the path of the
    /// checkpoint for the MLX server, which reads it from the request.
    #[serde(default)]
    pub server_model: Option<String>,
}

/// Manages the lifecycle of the language models.
pub struct LlmRuntimeManager {
    registry: Arc<TTSModelRegistry>,
    scheduler: Arc<Scheduler>,
    engines: Arc<EngineAssetManager>,
    logs_dir: PathBuf,
    runtimes: Mutex<HashMap<String, LlmRuntime>>,
    children: Mutex<HashMap<String, tokio::process::Child>>,
    leases: Mutex<HashMap<String, MemoryLease>>,
    logs: Mutex<HashMap<String, PathBuf>>,
    /// The manager itself, for the evictor of the scheduler.
    self_weak: Weak<LlmRuntimeManager>,
}

impl LlmRuntimeManager {
    /// Creates a runtime manager.
    pub fn new(
        registry: Arc<TTSModelRegistry>,
        scheduler: Arc<Scheduler>,
        engines: Arc<EngineAssetManager>,
        logs_dir: impl Into<PathBuf>,
    ) -> Arc<Self> {
        Arc::new_cyclic(|weak: &Weak<Self>| Self {
            registry,
            scheduler,
            engines,
            logs_dir: logs_dir.into(),
            runtimes: Mutex::new(HashMap::new()),
            children: Mutex::new(HashMap::new()),
            leases: Mutex::new(HashMap::new()),
            logs: Mutex::new(HashMap::new()),
            self_weak: weak.clone(),
        })
    }

    /// The key of the tenant of a model in the scheduler.
    fn tenant_key(model_id: &str) -> String {
        format!("llm:{}", model_id)
    }

    /// Registers the tenant of a model in the scheduler.
    ///
    /// The tenant carries the evictor, so that the scheduler can take the
    /// memory of the model back when a bigger one needs it.
    fn register_tenant(&self, spec: &crate::registry::TTSModelSpec) {
        self.scheduler.register_tenant(
            &Self::tenant_key(&spec.id),
            TenantKind::Llm,
            spec.estimated_memory_bytes,
            spec.default_ttl_secs.map(Duration::from_secs),
            Some(Self::evictor(self.self_weak.clone())),
        );
    }

    /// The callback that lets the scheduler take the memory of a model back.
    ///
    /// Without it the language models hold their memory until they are
    /// unloaded by hand, and a bigger model does not fit any more.
    fn evictor(weak: Weak<LlmRuntimeManager>) -> Evictor {
        evictor_from_fn(move |key: String| {
            let weak = weak.upgrade();
            async move {
                let Some(manager) = weak else {
                    return Ok(());
                };
                let model_id = key.strip_prefix("llm:").unwrap_or(&key).to_string();
                manager
                    .unload_model(&model_id)
                    .await
                    .map_err(|err| ai_scheduler::SchedulerError::EvictionFailed {
                        tenant: key,
                        reason: err.to_string(),
                    })
            }
        })
    }

    /// The state of every known language model.
    pub fn runtimes(&self) -> Vec<LlmRuntime> {
        let mut list: Vec<LlmRuntime> = self
            .runtimes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .cloned()
            .collect();
        // The memory of a running engine is read again, so that the numbers
        // follow the process.
        for runtime in &mut list {
            if runtime.state == RuntimeState::Running {
                runtime.actual_memory = runtime
                    .process_id
                    .and_then(crate::hardware::process_memory);
            }
            runtime.checkpoint_bytes = self
                .registry
                .get(&runtime.model_id)
                .map(|spec| spec.size_bytes);
        }
        list.sort_by(|a, b| a.model_id.cmp(&b.model_id));
        list
    }

    /// The state of one language model.
    pub fn runtime(&self, model_id: &str) -> Option<LlmRuntime> {
        self.runtimes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(model_id)
            .cloned()
    }

    /// Starts the server of a model and returns its runtime.
    pub async fn load_model(&self, model_id: &str) -> TtsResult<LlmRuntime> {
        if let Some(runtime) = self.runtime(model_id) {
            if runtime.state == RuntimeState::Running {
                return Ok(runtime);
            }
        }

        let spec = self
            .registry
            .resolve(model_id)
            .ok_or_else(|| TtsError::ModelNotFound(model_id.to_string()))?;
        if spec.family != crate::types::ModelFamily::Llm {
            return Err(TtsError::InvalidRequest(format!(
                "`{model_id}` is not a language model"
            )));
        }

        let hardware = crate::hardware::HardwareProfile::detect();
        let report = self.registry.check_compatibility(&spec, &hardware);
        if !report.compatible {
            return Err(TtsError::IncompatibleHardware {
                model: spec.id.clone(),
                reason: report.notes.join("; "),
            });
        }

        let checkpoint = self.checkpoint_of(&spec)?;
        let binary = self
            .engines
            .resolve_program(&spec.engine_id)
            .ok_or_else(|| {
                TtsError::Engine(format!(
                    "the {} runtime is not installed, install it on the runtime page first",
                    spec.engine_id
                ))
            })?;

        // The server of the model occupies memory of the machine, the same
        // pool the speech models use. The evictor lets the scheduler take the
        // memory back when a bigger model needs it.
        let tenant_key = Self::tenant_key(&spec.id);
        self.register_tenant(&spec);
        let lease = self
            .scheduler
            .acquire(&tenant_key, spec.estimated_memory_bytes, None)
            .await
            .map_err(|err| TtsError::Engine(format!("no memory for the model: {err}")))?;

        let port = free_port()?;
        let log_path = self.log_path(&spec.id);
        let mut command = tokio::process::Command::new(&binary);
        // The arguments come from the definition of the engine: the servers
        // do not speak the same dialect.
        command
            .args(crate::engine_assets::server_arguments(
                &spec.engine_id,
                &checkpoint,
                &spec.id,
                port,
            ))
            .args(spec.launch_args.clone())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command.kill_on_drop(true);

        let mut child = command.spawn().map_err(|err| {
            TtsError::Engine(format!("the llama.cpp server did not start: {err}"))
        })?;
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let log_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .ok();
        if let Some(file) = log_file {
            if let (Some(stdout), Some(clone)) = (stdout, file.try_clone().ok()) {
                pump(stdout, clone);
            }
            if let Some(stderr) = stderr {
                pump(stderr, file);
            }
        }

        let endpoint = format!("http://127.0.0.1:{port}");
        let pid = child.id();
        self.children
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(spec.id.clone(), child);
        self.logs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(spec.id.clone(), log_path.clone());
        self.leases
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(spec.id.clone(), lease);

        let runtime = LlmRuntime {
            model_id: spec.id.clone(),
            engine_id: spec.engine_id.clone(),
            state: RuntimeState::Loading,
            pinned: false,
            process_id: pid,
            endpoint: Some(endpoint.clone()),
            last_used_at: None,
            active_requests: 0,
            estimated_memory: spec.estimated_memory_bytes,
            actual_memory: None,
            checkpoint_bytes: None,
            server_model: Some(self.server_model_name(&spec)),
        };
        self.set_runtime(runtime.clone());
        let _ = self.scheduler.set_state(&tenant_key, TenantState::Loading);

        if let Err(err) = self.wait_until_healthy(&endpoint, &spec.id).await {
            self.unload_model(&spec.id).await.ok();
            return Err(err);
        }

        let mut runtime = runtime;
        runtime.state = RuntimeState::Running;
        self.set_runtime(runtime.clone());
        let _ = self.scheduler.set_state(&tenant_key, TenantState::Running);
        Ok(runtime)
    }

    /// Stops the server of a model.
    pub async fn unload_model(&self, model_id: &str) -> TtsResult<()> {
        let tenant_key = Self::tenant_key(model_id);
        let _ = self.scheduler.set_state(&tenant_key, TenantState::Unloading);
        // The locks are left before the process is stopped: a guard that is
        // held over an await makes the future unusable in the server.
        let child = {
            self.children
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(model_id)
        };
        if let Some(mut child) = child {
            let _ = child.kill().await;
        }
        let lease = {
            self.leases
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(model_id)
        };
        if let Some(lease) = lease {
            let key = lease.key().to_string();
            let bytes = lease.bytes();
            drop(lease);
            self.scheduler.release(&key, bytes);
        }
        self.runtimes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(model_id);
        self.scheduler.unregister_tenant(&tenant_key);
        Ok(())
    }

    /// Stops every server.
    pub async fn shutdown(&self) {
        let ids: Vec<String> = self
            .children
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .cloned()
            .collect();
        for id in ids {
            self.unload_model(&id).await.ok();
        }
    }

    /// Forwards a chat request to the server of the model.
    pub async fn chat(
        &self,
        model_id: &str,
        body: serde_json::Value,
    ) -> TtsResult<serde_json::Value> {
        self.forward(model_id, "/v1/chat/completions", body).await
    }

    /// Forwards a plain completion request to the server of the model.
    pub async fn completion(
        &self,
        model_id: &str,
        body: serde_json::Value,
    ) -> TtsResult<serde_json::Value> {
        self.forward(model_id, "/v1/completions", body).await
    }

    /// The output of the last start attempt of a model.
    pub fn logs(&self, model_id: &str) -> String {
        let path = self
            .logs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(model_id)
            .cloned();
        match path {
            Some(path) => tail_of(&path, 64 * 1024),
            None => String::new(),
        }
    }

    /// Forwards a request to the server of the model.
    async fn forward(
        &self,
        model_id: &str,
        path: &str,
        body: serde_json::Value,
    ) -> TtsResult<serde_json::Value> {
        let runtime = match self.runtime(model_id) {
            Some(runtime) if runtime.state == RuntimeState::Running => {
                if self.engine_alive(model_id) {
                    runtime
                } else {
                    // The engine died on its own. The old runtime is cleaned
                    // up and the model is started again, instead of sending
                    // the request to an address that nobody listens on.
                    self.unload_model(model_id).await.ok();
                    self.load_model(model_id).await?
                }
            }
            _ => self.load_model(model_id).await?,
        };

        // The request is registered with the scheduler. It refreshes the idle
        // timer of the tenant and keeps the model from being taken away
        // while it answers. Without it the scheduler counts the idle time
        // from the moment the model was loaded, and takes a model that is in
        // use away in the middle of a request.
        //
        // The guard gives the mark back when it is dropped, also when the
        // request is cancelled: a client that hangs up must not leave the
        // tenant busy, or the scheduler would never take the model back.
        let tenant_key = Self::tenant_key(model_id);
        let guard = self.begin_active_request(model_id, &tenant_key);

        let result = self.forward_to(&runtime, path, body).await;

        drop(guard);
        result
    }

    /// Marks the tenant of a model as busy and returns the guard that gives
    /// the mark back.
    ///
    /// The mark keeps the model from being taken away while it answers. The
    /// guard gives it back when it is dropped, also when the request is
    /// cancelled: a client that hangs up in the middle of a request must not
    /// leave the tenant busy forever, or the scheduler would never evict the
    /// model and its memory would stay booked.
    fn begin_active_request(&self, model_id: &str, tenant_key: &str) -> Option<ActiveRequest> {
        if self.scheduler.begin_request(tenant_key).is_err() {
            return None;
        }
        self.update_runtime(model_id, |runtime| {
            runtime.active_requests = runtime.active_requests.saturating_add(1);
            runtime.last_used_at = Some(Utc::now());
        });
        let scheduler = self.scheduler.clone();
        let tenant_key = tenant_key.to_string();
        let model_id = model_id.to_string();
        let manager = self.self_weak.upgrade();
        Some(ActiveRequest::new(move || {
            if let Some(manager) = manager {
                manager.update_runtime(&model_id, |runtime| {
                    runtime.active_requests = runtime.active_requests.saturating_sub(1);
                    runtime.last_used_at = Some(Utc::now());
                });
            }
            scheduler.end_request(&tenant_key).ok();
        }))
    }

    /// Sends the request to the server of the model.
    async fn forward_to(
        &self,
        runtime: &LlmRuntime,
        path: &str,
        mut body: serde_json::Value,
    ) -> TtsResult<serde_json::Value> {
        let endpoint = runtime
            .endpoint
            .clone()
            .ok_or_else(|| TtsError::Engine("the model has no server".to_string()))?;

        // The server knows one model, the name in the request is only a label.
        if let Some(object) = body.as_object_mut() {
            let name = runtime
                .server_model
                .clone()
                .unwrap_or_else(|| runtime.model_id.clone());
            object.insert("model".to_string(), serde_json::json!(name));
        }

        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|err| TtsError::Engine(format!("the client is broken: {err}")))?;
        let response = client
            .post(format!("{endpoint}{path}"))
            .json(&body)
            .send()
            .await
            .map_err(|err| TtsError::Engine(format!("the model did not answer: {err}")))?;

        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|err| TtsError::Engine(format!("the answer was cut off: {err}")))?;
        if !status.is_success() {
            return Err(TtsError::Engine(format!(
                "the model answered with {status}: {}",
                text.chars().take(400).collect::<String>()
            )));
        }
        let mut answer: serde_json::Value =
            serde_json::from_str(&text).map_err(|err| TtsError::Serialization(err.to_string()))?;
        // The answer carries the name the server knows the model by; the
        // caller asked for the model of the catalogue.
        if let Some(object) = answer.as_object_mut() {
            object.insert("model".to_string(), serde_json::json!(runtime.model_id));
        }
        Ok(answer)
    }

    /// The name the server of a model knows the model by.
    ///
    /// llama.cpp is told the name of the catalogue through `--alias`, and it
    /// answers to it. The MLX server reads the checkpoint from the request
    /// instead, and it only loads what is on the disk — so it gets the path
    /// of the checkpoint.
    fn server_model_name(&self, spec: &crate::registry::TTSModelSpec) -> String {
        if crate::engine_assets::engine_reads_gguf(&spec.engine_id) {
            return spec.id.clone();
        }
        self.registry
            .install_path(&spec.id)
            .map(|path| path.to_string_lossy().to_string())
            .unwrap_or_else(|| spec.id.clone())
    }

    /// The path the server of the model is given.
    ///
    /// The engines that read GGUF want the file itself, the ones that read
    /// safetensors want the directory with the weights, the config and the
    /// tokenizer.
    fn checkpoint_of(&self, spec: &crate::registry::TTSModelSpec) -> TtsResult<PathBuf> {
        let dir = self
            .registry
            .install_path(&spec.id)
            .ok_or_else(|| TtsError::ModelNotFound(spec.id.clone()))?;
        if !dir.is_dir() {
            return Err(TtsError::InvalidRequest(format!(
                "`{}` is not downloaded yet",
                spec.id
            )));
        }
        if !crate::engine_assets::engine_reads_gguf(&spec.engine_id) {
            return Ok(dir);
        }
        gguf_checkpoint(&dir).ok_or_else(|| {
            TtsError::InvalidRequest(format!("`{}` has no GGUF checkpoint", spec.id))
        })
    }

    /// The file the output of the server goes to.
    fn log_path(&self, model_id: &str) -> PathBuf {
        let stamp = chrono::Local::now().format("%Y%m%dT%H%M%S");
        self.logs_dir.join(format!("{model_id}-{stamp}.log"))
    }

    /// Waits until the server answers.
    async fn wait_until_healthy(&self, endpoint: &str, model_id: &str) -> TtsResult<()> {
        let client = reqwest::Client::new();
        let deadline = std::time::Instant::now() + HEALTH_TIMEOUT;
        loop {
            if let Ok(response) = client.get(format!("{endpoint}/health")).send().await {
                if response.status().is_success() {
                    return Ok(());
                }
            }
            if std::time::Instant::now() >= deadline {
                let log = self.logs(model_id);
                let tail: String = log.lines().rev().take(12).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n");
                return Err(TtsError::Engine(format!(
                    "the llama.cpp server did not become ready: {tail}"
                )));
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// Stores the state of a model.
    fn set_runtime(&self, runtime: LlmRuntime) {
        self.runtimes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(runtime.model_id.clone(), runtime);
    }

    /// Changes the state of one runtime.
    fn update_runtime(&self, model_id: &str, update: impl FnOnce(&mut LlmRuntime)) {
        let mut runtimes = self.runtimes.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(runtime) = runtimes.get_mut(model_id) {
            update(runtime);
        }
    }

    /// `true` when the process of the engine is still running.
    ///
    /// The runtime keeps the state of the model, but the process of the
    /// engine can die on its own. A model whose engine is gone is started
    /// again, instead of answering with an error.
    fn engine_alive(&self, model_id: &str) -> bool {
        let mut children = self.children.lock().unwrap_or_else(|e| e.into_inner());
        match children.get_mut(model_id) {
            Some(child) => matches!(child.try_wait(), Ok(None)),
            None => false,
        }
    }

    /// Pins or unpins a model.
    ///
    /// A pinned model is never taken away by the scheduler, so it stays in
    /// memory even when it is idle.
    pub fn set_pinned(&self, model_id: &str, pinned: bool) -> TtsResult<()> {
        let tenant_key = Self::tenant_key(model_id);
        self.scheduler.set_pinned(&tenant_key, pinned)?;
        self.update_runtime(model_id, |runtime| runtime.pinned = pinned);
        Ok(())
    }
}

/// Copies the output of the server into the log file.
fn pump<R>(reader: R, file: std::fs::File)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        use tokio::io::AsyncReadExt;
        let mut reader = reader;
        let mut file = file;
        let mut buffer = vec![0_u8; 4096];
        loop {
            match reader.read(&mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    use std::io::Write;
                    let _ = file.write_all(&buffer[..read]);
                    let _ = file.flush();
                }
            }
        }
    });
}

/// The largest GGUF file below a directory.
///
/// A repository of quantised checkpoints holds one file per quantisation, the
/// biggest one is the one the model asks for.
pub fn gguf_checkpoint(dir: &Path) -> Option<PathBuf> {
    let mut best: Option<(u64, PathBuf)> = None;
    collect_gguf(dir, &mut best);
    best.map(|(_, path)| path)
}

/// Collects the GGUF files below a directory.
fn collect_gguf(dir: &Path, best: &mut Option<(u64, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_gguf(&path, best);
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("gguf") {
            continue;
        }
        let size = entry.metadata().map(|meta| meta.len()).unwrap_or(0);
        if best.as_ref().map(|(largest, _)| size > *largest).unwrap_or(true) {
            *best = Some((size, path));
        }
    }
}

/// A free port on the loopback interface.
fn free_port() -> TtsResult<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|err| TtsError::Engine(format!("no free port: {err}")))?;
    let port = listener
        .local_addr()
        .map_err(|err| TtsError::Engine(format!("no free port: {err}")))?
        .port();
    drop(listener);
    Ok(port)
}

/// The last bytes of a file.
fn tail_of(path: &Path, bytes: usize) -> String {
    let Ok(data) = std::fs::read(path) else {
        return String::new();
    };
    let start = data.len().saturating_sub(bytes);
    String::from_utf8_lossy(&data[start..]).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::AppPaths;
    use crate::registry::TTSModelSpec;
    use crate::types::ModelFamily;

    /// A model of the catalogue, without a checkpoint.
    fn spec(id: &str) -> TTSModelSpec {
        TTSModelSpec {
            id: id.to_string(),
            name: id.to_string(),
            family: ModelFamily::Llm,
            engine_id: "llama.cpp".to_string(),
            hf_repo: format!("some/{id}"),
            version: "1".to_string(),
            format: "gguf".to_string(),
            size_bytes: 1024,
            estimated_memory_bytes: 1024,
            parameters: None,
            features: Vec::new(),
            llm_features: Vec::new(),
            files: Vec::new(),
            hardware: Default::default(),
            mode: crate::types::EngineMode::LlamaCpp,
            default_ttl_secs: None,
            aliases: Vec::new(),
            launch_args: Vec::new(),
            local_path: None,
            notes: None,
        }
    }

    /// The language models give their memory back when a bigger model needs
    /// it.
    ///
    /// The scheduler can only take the memory of a tenant back that carries
    /// an evictor. Without it the models of the language runtime hold their
    /// memory until they are unloaded by hand, and nothing bigger fits any
    /// more.
    #[tokio::test]
    async fn a_language_model_can_be_evicted() {
        let root = std::env::temp_dir().join(format!("lm-lathe-llm-{}", std::process::id()));
        let paths = AppPaths::with_root(&root);
        paths.ensure().expect("the directories are there");
        let scheduler = Scheduler::new(ai_scheduler::SchedulerConfig {
            total_bytes: 8 * 1024 * 1024 * 1024,
            reserved_bytes: 0,
            ..Default::default()
        });
        let manager = LlmRuntimeManager::new(
            Arc::new(TTSModelRegistry::builtin()),
            scheduler.clone(),
            EngineAssetManager::new(paths.clone()).expect("the engines are known"),
            paths.llm_logs_root(),
        );
        let spec = spec("a-language-model");
        manager.register_tenant(&spec);
        assert!(
            scheduler.tenant_is_evictable(&LlmRuntimeManager::tenant_key(&spec.id)),
            "the memory of a language model can be taken back"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
