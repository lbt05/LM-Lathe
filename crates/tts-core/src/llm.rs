//! Runtime of the language models.
//!
//! The language models are served by llama.cpp: the runtime starts the
//! server for a GGUF checkpoint and forwards the requests of the gateway to
//! it. The server speaks the OpenAI API, so the gateway offers the same
//! routes for the chat and the plain completions.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ai_scheduler::{MemoryLease, Scheduler, TenantKind};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::engine_assets::EngineAssetManager;
use crate::error::{TtsError, TtsResult};
use crate::registry::TTSModelRegistry;
use crate::types::RuntimeState;

/// How long the runtime waits for the server to answer.
const HEALTH_TIMEOUT: Duration = Duration::from_secs(180);

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
}

impl LlmRuntimeManager {
    /// Creates a runtime manager.
    pub fn new(
        registry: Arc<TTSModelRegistry>,
        scheduler: Arc<Scheduler>,
        engines: Arc<EngineAssetManager>,
        logs_dir: impl Into<PathBuf>,
    ) -> Arc<Self> {
        Arc::new(Self {
            registry,
            scheduler,
            engines,
            logs_dir: logs_dir.into(),
            runtimes: Mutex::new(HashMap::new()),
            children: Mutex::new(HashMap::new()),
            leases: Mutex::new(HashMap::new()),
            logs: Mutex::new(HashMap::new()),
        })
    }

    /// The state of every known language model.
    pub fn runtimes(&self) -> Vec<LlmRuntime> {
        let runtimes = self.runtimes.lock().unwrap_or_else(|e| e.into_inner());
        let mut list: Vec<LlmRuntime> = runtimes.values().cloned().collect();
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
                TtsError::Engine(
                    "the llama.cpp runtime is not installed, install it on the runtime page first"
                        .to_string(),
                )
            })?;

        // The server of the model occupies memory of the machine, the same
        // pool the speech models use.
        let tenant_key = format!("llm:{}", spec.id);
        self.scheduler.register_tenant(
            &tenant_key,
            TenantKind::Llm,
            spec.estimated_memory_bytes,
            spec.default_ttl_secs.map(Duration::from_secs),
            None,
        );
        let lease = self
            .scheduler
            .acquire(&tenant_key, spec.estimated_memory_bytes, None)
            .await
            .map_err(|err| TtsError::Engine(format!("no memory for the model: {err}")))?;

        let port = free_port()?;
        let log_path = self.log_path(&spec.id);
        let mut command = tokio::process::Command::new(&binary);
        command
            .arg("--model")
            .arg(&checkpoint)
            .arg("--host")
            .arg("127.0.0.1")
            .arg("--port")
            .arg(port.to_string())
            .arg("--alias")
            .arg(&spec.id)
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
        };
        self.set_runtime(runtime.clone());

        if let Err(err) = self.wait_until_healthy(&endpoint, &spec.id).await {
            self.unload_model(&spec.id).await.ok();
            return Err(err);
        }

        let mut runtime = runtime;
        runtime.state = RuntimeState::Running;
        self.set_runtime(runtime.clone());
        Ok(runtime)
    }

    /// Stops the server of a model.
    pub async fn unload_model(&self, model_id: &str) -> TtsResult<()> {
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
        mut body: serde_json::Value,
    ) -> TtsResult<serde_json::Value> {
        let runtime = match self.runtime(model_id) {
            Some(runtime) if runtime.state == RuntimeState::Running => runtime,
            _ => self.load_model(model_id).await?,
        };
        let endpoint = runtime
            .endpoint
            .ok_or_else(|| TtsError::Engine("the model has no server".to_string()))?;

        // The server knows one model, the name in the request is only a label.
        if let Some(object) = body.as_object_mut() {
            object.insert("model".to_string(), serde_json::json!(model_id));
        }

        let client = reqwest::Client::new();
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
        serde_json::from_str(&text).map_err(|err| TtsError::Serialization(err.to_string()))
    }

    /// The GGUF file of a model.
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
