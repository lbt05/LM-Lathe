//! Service level adapter for the SGLang-Omni runtime.
//!
//! The adapter owns the inference process: it starts `sgl-omni serve` on the
//! CUDA path or `fishaudio_drama3_server` on the Apple Silicon path, waits
//! for the engine to become healthy, proxies inference calls and exposes the
//! engine log file.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::process::{Child, Command};

use crate::adapter::protocol::{raw_to_audio_output, ModelProtocol, RequestContext};
use crate::adapter::{EngineAdapter, TTSAdapter};
use crate::error::{TtsError, TtsResult};
use crate::types::{
    AudioFormat, AudioOutput, EngineMode, HealthStatus, RawAudioResponse, RuntimeHandle,
    TTSRequest, TTSRuntimeConfig,
};

/// Tuning knobs of the engine adapter.
#[derive(Clone)]
pub struct EngineSettings {
    /// Binary used for the CUDA path.
    pub sglang_binary: String,
    /// Binary used for the native Apple Silicon path.
    pub mlx_binary: String,
    /// Binary used for the mlx-audio path.
    pub mlx_audio_binary: String,
    /// Directory the engine logs are written to.
    pub logs_dir: PathBuf,
    /// Timeout of a single inference request.
    pub request_timeout: Duration,
    /// Interval between two health probes.
    pub health_interval: Duration,
    /// Resolves the program of an engine.
    ///
    /// The callback receives the engine id of a model (`sglang-omni`,
    /// `fishaudio-drama3-mlx`) and returns the binary that serves it. The
    /// managed engine installations win over the `PATH`.
    pub binary_resolver: Option<Arc<dyn Fn(&str) -> Option<PathBuf> + Send + Sync>>,
}

impl Default for EngineSettings {
    fn default() -> Self {
        Self {
            sglang_binary: "sgl-omni".to_string(),
            mlx_binary: "fishaudio_drama3_server".to_string(),
            mlx_audio_binary: "mlx_audio.server".to_string(),
            logs_dir: PathBuf::from("logs/tts"),
            request_timeout: Duration::from_secs(600),
            health_interval: Duration::from_millis(750),
            binary_resolver: None,
        }
    }
}

impl EngineSettings {
    /// Resolves the binary that serves the given engine id.
    fn resolve_binary(&self, engine_id: &str) -> Option<PathBuf> {
        self.binary_resolver.as_ref().and_then(|resolver| resolver(engine_id))
    }

    /// The program that serves a model, when it is there.
    ///
    /// The managed installations win over the `PATH`. A model whose engine is
    /// not installed has no program: it cannot be loaded before the engine
    /// is there.
    pub fn program_for(&self, engine_id: &str, fallback: &str) -> Option<PathBuf> {
        self.resolve_binary(engine_id).or_else(|| {
            crate::engine_assets::find_in_path(fallback)
        })
    }
}

impl std::fmt::Debug for EngineSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineSettings")
            .field("sglang_binary", &self.sglang_binary)
            .field("mlx_binary", &self.mlx_binary)
            .field("mlx_audio_binary", &self.mlx_audio_binary)
            .field("logs_dir", &self.logs_dir)
            .field("request_timeout", &self.request_timeout)
            .field("health_interval", &self.health_interval)
            .finish()
    }
}

struct ProcessEntry {
    child: Child,
    log_path: PathBuf,
}

/// The last 64 kilobytes of a log file.
fn tail_of(path: &Path) -> String {
    let Ok(bytes) = std::fs::read(path) else {
        return String::new();
    };
    let tail = bytes.len().saturating_sub(64 * 1024);
    String::from_utf8_lossy(&bytes[tail..]).to_string()
}

/// The last lines of a log, without the empty ones.
fn last_lines(log: &str, count: usize) -> String {
    let lines: Vec<&str> = log
        .lines()
        .map(|line| line.trim_end())
        .filter(|line| !line.is_empty())
        .collect();
    let start = lines.len().saturating_sub(count);
    lines[start..].join("\n")
}

/// Adapter that manages the SGLang-Omni inference service.
pub struct SGLangOmniAdapter {
    client: reqwest::Client,
    settings: EngineSettings,
    processes: Mutex<HashMap<String, ProcessEntry>>,
    current: RwLock<Option<RuntimeHandle>>,
    protocols: RwLock<HashMap<crate::types::ModelFamily, Arc<dyn ModelProtocol>>>,
    registry: RwLock<Option<Arc<crate::registry::TTSModelRegistry>>>,
    /// The log of every start attempt, also the ones that failed.
    log_files: Mutex<HashMap<String, PathBuf>>,
}

impl SGLangOmniAdapter {
    /// Creates an adapter with the given settings.
    pub fn new(settings: EngineSettings) -> Arc<Self> {
        let client = reqwest::Client::builder()
            .timeout(settings.request_timeout)
            .build()
            .unwrap_or_default();
        Arc::new(Self {
            client,
            settings,
            processes: Mutex::new(HashMap::new()),
            current: RwLock::new(None),
            protocols: RwLock::new(HashMap::new()),
            registry: RwLock::new(None),
            log_files: Mutex::new(HashMap::new()),
        })
    }

    /// The settings of the engine.
    pub fn settings(&self) -> &EngineSettings {
        &self.settings
    }

    /// Registers the model registry used to resolve protocols.
    pub fn with_registry(
        self: &Arc<Self>,
        registry: Arc<crate::registry::TTSModelRegistry>,
    ) -> Arc<Self> {
        *self.registry.write().unwrap_or_else(|e| e.into_inner()) = Some(registry);
        self.clone()
    }

    /// Registers a protocol for a model family.
    pub fn register_protocol(
        &self,
        family: crate::types::ModelFamily,
        protocol: Arc<dyn ModelProtocol>,
    ) {
        self.protocols
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(family, protocol);
    }

    /// The handle of the engine instance that is currently running.
    pub fn current_handle(&self) -> Option<RuntimeHandle> {
        self.current
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Builds the command line for the given configuration.
    fn build_command(&self, config: &TTSRuntimeConfig, port: u16) -> (String, Vec<String>) {
        let mut args = Vec::new();
        match config.mode {
            EngineMode::SglangOmni => {
                args.push("serve".to_string());
                args.push("--model-path".to_string());
                args.push(config.model_path.clone());
                args.push("--port".to_string());
                args.push(port.to_string());
                args.extend(config.extra_args.iter().cloned());
                let binary = self
                    .settings
                    .resolve_binary("sglang-omni")
                    .map(|path| path.to_string_lossy().to_string())
                    .unwrap_or_else(|| self.settings.sglang_binary.clone());
                (binary, args)
            }
            EngineMode::MlxNative => {
                args.push("--model-path".to_string());
                args.push(config.model_path.clone());
                if let Some(name) = &config.model_name {
                    args.push("--model-name".to_string());
                    args.push(name.clone());
                }
                args.push("--port".to_string());
                args.push(port.to_string());
                args.extend(config.extra_args.iter().cloned());
                // The engine id of the model decides which server of the
                // native runtime is started: the drama3 server or the
                // Qwen3-TTS one.
                let binary = self
                    .settings
                    .resolve_binary(&config.engine_id)
                    .or_else(|| self.settings.resolve_binary("fishaudio-drama3-mlx"))
                    .map(|path| path.to_string_lossy().to_string())
                    .unwrap_or_else(|| self.settings.mlx_binary.clone());
                (binary, args)
            }
            EngineMode::MlxAudio => {
                // mlx-audio serves an OpenAI compatible API and loads the
                // checkpoint that the request names, so the server only gets
                // the address it listens on.
                args.push("--host".to_string());
                args.push("127.0.0.1".to_string());
                args.push("--port".to_string());
                args.push(port.to_string());
                // The server writes its own log into a directory. The
                // default is relative, and the working directory of the
                // process is not always writable: the application starts the
                // engine from inside its bundle.
                args.push("--log-dir".to_string());
                args.push(
                    self.settings
                        .logs_dir
                        .join("mlx-audio")
                        .to_string_lossy()
                        .to_string(),
                );
                args.extend(config.extra_args.iter().cloned());
                let binary = self
                    .settings
                    .resolve_binary("mlx-audio")
                    .map(|path| path.to_string_lossy().to_string())
                    .unwrap_or_else(|| self.settings.mlx_audio_binary.clone());
                (binary, args)
            }
            EngineMode::LlamaCpp => {
                // The language models run in their own runtime, they never
                // reach the speech engine.
                ("llama-server".to_string(), Vec::new())
            }
            EngineMode::MlxLlm => {
                // The language models run in their own runtime, they never
                // reach the speech engine.
                ("mlx_lm.server".to_string(), Vec::new())
            }
            #[cfg(feature = "mock-engine")]
            EngineMode::Mock => ("mock".to_string(), Vec::new()),
        }
    }

    /// The log of the last start attempt of a model.
    ///
    /// A model that could not be loaded has no runtime, but the engine wrote
    /// the reason into its log. The file is found even when the server was
    /// restarted in between, because the logs are named after the model.
    pub fn logs_for(&self, model_id: &str) -> String {
        let path = self
            .log_files
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(model_id)
            .cloned()
            .or_else(|| self.newest_log(model_id));
        match path {
            Some(path) => tail_of(&path),
            None => String::new(),
        }
    }

    /// The newest log file of a model.
    fn newest_log(&self, model_id: &str) -> Option<PathBuf> {
        let entries = std::fs::read_dir(&self.settings.logs_dir).ok()?;
        let prefix = format!("{model_id}-");
        let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if !name.starts_with(&prefix) {
                continue;
            }
            let modified = entry
                .metadata()
                .and_then(|meta| meta.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            if newest.as_ref().map(|(time, _)| modified > *time).unwrap_or(true) {
                newest = Some((modified, path));
            }
        }
        newest.map(|(_, path)| path)
    }

    /// Probes the health endpoint of a running engine.
    async fn probe(&self, endpoint: &str) -> HealthStatus {
        match self
            .client
            .get(format!("{endpoint}/health"))
            .timeout(Duration::from_secs(10))
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => HealthStatus::Healthy,
            Ok(response) => {
                let status = response.status();
                match self
                    .client
                    .get(format!("{endpoint}/v1/models"))
                    .timeout(Duration::from_secs(10))
                    .send()
                    .await
                {
                    Ok(models) if models.status().is_success() => HealthStatus::Healthy,
                    Ok(_) => HealthStatus::Degraded {
                        reason: format!("health endpoint returned HTTP {status}"),
                    },
                    Err(err) => HealthStatus::Unhealthy {
                        reason: err.to_string(),
                    },
                }
            }
            Err(err) => HealthStatus::Unhealthy {
                reason: err.to_string(),
            },
        }
    }

    /// Polls the engine until it answers health checks.
    async fn wait_until_healthy(
        &self,
        child: &mut Child,
        endpoint: &str,
        deadline: Instant,
        log_path: &Path,
    ) -> TtsResult<()> {
        let mut status;
        loop {
            match child.try_wait() {
                Ok(Some(exit)) => {
                    // The engine wrote the reason into its log, the user has
                    // to see it.
                    let log = tail_of(log_path);
                    let reason = last_lines(&log, 12);
                    return Err(TtsError::Engine(if reason.is_empty() {
                        format!("engine exited during startup with {exit}")
                    } else {
                        format!("engine exited during startup with {exit}:\n{reason}")
                    }));
                }
                Ok(None) => {}
                Err(err) => {
                    return Err(TtsError::Engine(format!(
                        "failed to poll engine process: {err}"
                    )));
                }
            }
            status = self.probe(endpoint).await;
            if status.is_healthy() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(TtsError::HealthCheckFailed(format!(
                    "engine did not become healthy before the deadline: {}",
                    status.as_str()
                )));
            }
            tokio::time::sleep(self.settings.health_interval).await;
        }
    }

    /// Resolves the protocol of a model family.
    fn protocol_for(&self, model_id: &str) -> Option<Arc<dyn ModelProtocol>> {
        let protocols = self.protocols.read().unwrap_or_else(|e| e.into_inner());
        if let Some(protocol) = protocols.get(&crate::types::ModelFamily::Qwen3Tts) {
            return Some(protocol.clone());
        }
        let registry = self
            .registry
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()?;
        let spec = registry.resolve(model_id)?;
        protocols.get(&spec.family).cloned()
    }
}

#[async_trait]
impl EngineAdapter for SGLangOmniAdapter {
    async fn start(&self, config: &TTSRuntimeConfig) -> TtsResult<RuntimeHandle> {
        #[cfg(feature = "mock-engine")]
        if config.mode == EngineMode::Mock {
            return Err(TtsError::InvalidRequest(
                "the mock engine does not spawn a process".to_string(),
            ));
        }
        let port = match config.port {
            Some(port) => port,
            None => free_port()?,
        };
        let (binary, args) = self.build_command(config, port);
        std::fs::create_dir_all(&self.settings.logs_dir)
            .map_err(|e| TtsError::io(&self.settings.logs_dir, e))?;
        let log_path = self.settings.logs_dir.join(format!(
            "{}-{}.log",
            sanitize(&config.model_id),
            chrono::Utc::now().format("%Y%m%dT%H%M%S")
        ));
        let stdout = std::fs::File::create(&log_path).map_err(|e| TtsError::io(&log_path, e))?;
        let stderr = stdout.try_clone().map_err(|e| TtsError::io(&log_path, e))?;

        let mut command = Command::new(&binary);
        command
            .args(&args)
            // The engine runs in the directory of its logs, so that a
            // relative path of the engine always has a writable place to go
            // to.
            .current_dir(&self.settings.logs_dir)
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .kill_on_drop(false);
        for (key, value) in &config.env {
            command.env(key, value);
        }
        let mut child = command.spawn().map_err(|e| {
            TtsError::Engine(format!(
                "failed to start `{binary}` for model `{}`: {e}",
                config.model_id
            ))
        })?;
        let pid = child.id();
        let endpoint = format!("http://127.0.0.1:{port}");
        self.log_files
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(config.model_id.clone(), log_path.clone());

        tracing::info!(model = %config.model_id, binary = %binary, ?args, ?pid, "starting engine");

        let deadline = Instant::now() + config.health_timeout;
        match self
            .wait_until_healthy(&mut child, &endpoint, deadline, &log_path)
            .await
        {
            Ok(()) => {}
            Err(err) => {
                let _ = terminate(&mut child, config.shutdown_timeout).await;
                return Err(err);
            }
        }

        let handle = RuntimeHandle {
            model_id: config.model_id.clone(),
            model_repo: config
                .model_name
                .clone()
                .or_else(|| Some(config.model_path.clone())),
            model_path: Some(config.model_path.clone()),
            engine_id: config.engine_id.clone(),
            process_id: pid,
            endpoint: Some(endpoint),
            started_at: chrono::Utc::now(),
            log_path: Some(log_path.clone()),
            mode: config.mode,
        };
        self.processes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(config.model_id.clone(), ProcessEntry { child, log_path });
        *self.current.write().unwrap_or_else(|e| e.into_inner()) = Some(handle.clone());
        Ok(handle)
    }

    async fn stop(&self, handle: &RuntimeHandle) -> TtsResult<()> {
        let entry = self
            .processes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&handle.model_id);
        if let Some(mut entry) = entry {
            terminate(&mut entry.child, Duration::from_secs(20)).await?;
        }
        let mut current = self.current.write().unwrap_or_else(|e| e.into_inner());
        if current
            .as_ref()
            .map(|h| h.model_id == handle.model_id)
            .unwrap_or(false)
        {
            *current = None;
        }
        Ok(())
    }

    async fn health_check(&self, handle: &RuntimeHandle) -> TtsResult<HealthStatus> {
        let Some(endpoint) = handle.endpoint.as_deref() else {
            return Ok(HealthStatus::Unhealthy {
                reason: "runtime has no endpoint".to_string(),
            });
        };
        Ok(self.probe(endpoint).await)
    }

    async fn unload_model(&self, handle: &RuntimeHandle) -> TtsResult<()> {
        // The product policy is one model per engine instance, so unloading
        // the model means stopping the instance.
        EngineAdapter::stop(self, handle).await
    }

    async fn get_logs(&self, handle: &RuntimeHandle) -> TtsResult<String> {
        let path = handle.log_path.clone().or_else(|| {
            self.processes
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&handle.model_id)
                .map(|e| e.log_path.clone())
        });
        let Some(path) = path else {
            return Ok(String::new());
        };
        if !path.exists() {
            return Ok(String::new());
        }
        let bytes = std::fs::read(&path).map_err(|e| TtsError::io(&path, e))?;
        let tail = bytes.len().saturating_sub(64 * 1024);
        Ok(String::from_utf8_lossy(&bytes[tail..]).to_string())
    }

    async fn synthesize_raw(
        &self,
        handle: &RuntimeHandle,
        payload: &serde_json::Value,
    ) -> TtsResult<RawAudioResponse> {
        let Some(endpoint) = handle.endpoint.as_deref() else {
            return Err(TtsError::Engine("runtime has no endpoint".to_string()));
        };
        let url = format!("{endpoint}/v1/audio/speech");
        let response = self
            .client
            .post(&url)
            .json(payload)
            .send()
            .await
            .map_err(|e| TtsError::Engine(format!("inference request failed: {e}")))?;
        let status = response.status();
        let headers = response.headers().clone();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(TtsError::Engine(format!(
                "engine returned HTTP {status}: {}",
                body.trim()
            )));
        }
        let content_type = headers
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let finish_reason = headers
            .get("x-finish-reason")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let sample_rate = ["x-sample-rate", "sample-rate", "x-audio-sample-rate"]
            .iter()
            .find_map(|name| {
                headers
                    .get(*name)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u32>().ok())
            });
        let channels = ["x-channels", "channels", "x-audio-channels"]
            .iter()
            .find_map(|name| {
                headers
                    .get(*name)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u16>().ok())
            });
        let bytes = response
            .bytes()
            .await
            .map_err(|e| TtsError::Engine(format!("failed to read engine response: {e}")))?;
        Ok(RawAudioResponse {
            bytes: bytes.to_vec(),
            content_type,
            finish_reason,
            sample_rate,
            channels,
        })
    }
}

#[async_trait]
impl TTSAdapter for SGLangOmniAdapter {
    async fn start(&self, config: TTSRuntimeConfig) -> TtsResult<RuntimeHandle> {
        EngineAdapter::start(self, &config).await
    }

    async fn stop(&self, handle: RuntimeHandle) -> TtsResult<()> {
        EngineAdapter::stop(self, &handle).await
    }

    async fn health_check(&self, handle: &RuntimeHandle) -> TtsResult<HealthStatus> {
        EngineAdapter::health_check(self, handle).await
    }

    async fn synthesize(&self, request: TTSRequest) -> TtsResult<AudioOutput> {
        let handle = self
            .current_handle()
            .ok_or_else(|| TtsError::Engine("no engine instance is running".to_string()))?;
        let protocol = self.protocol_for(&handle.model_id).ok_or_else(|| {
            TtsError::AdapterNotFound(format!("no protocol for model `{}`", handle.model_id))
        })?;
        let ctx = RequestContext {
            mode: handle.mode,
            model_repo: handle
                .model_repo
                .clone()
                .unwrap_or_else(|| handle.model_id.clone()),
            model_path: handle.model_path.clone(),
        };
        let payload = protocol.build_payload(&request, &ctx)?;
        let raw = EngineAdapter::synthesize_raw(self, &handle, &payload).await?;
        protocol.parse_output(raw)
    }

    async fn unload_model(&self, handle: &RuntimeHandle) -> TtsResult<()> {
        EngineAdapter::unload_model(self, handle).await
    }

    async fn get_logs(&self, handle: &RuntimeHandle) -> TtsResult<String> {
        EngineAdapter::get_logs(self, handle).await
    }
}

/// Terminates a child process gracefully and forcefully if needed.
async fn terminate(child: &mut Child, grace: Duration) -> TtsResult<()> {
    if let Some(pid) = child.id() {
        #[cfg(unix)]
        unsafe {
            libc::kill(pid as i32, libc::SIGTERM);
        }
        #[cfg(not(unix))]
        let _ = pid;
    }
    let deadline = Instant::now() + grace;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return Ok(()),
            Ok(None) => {}
            Err(err) => {
                return Err(TtsError::Engine(format!(
                    "failed to wait for engine process: {err}"
                )))
            }
        }
        if Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    child
        .kill()
        .await
        .map_err(|e| TtsError::Engine(format!("failed to kill engine process: {e}")))?;
    Ok(())
}

/// Picks a free TCP port on the loopback interface.
pub fn free_port() -> TtsResult<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|e| TtsError::Internal(format!("failed to allocate a port: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| TtsError::Internal(format!("failed to read the allocated port: {e}")))?
        .port();
    drop(listener);
    Ok(port)
}

/// Removes characters that are unsafe in file names.
fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// Helper used by the model adapters: converts a raw answer into an output.
pub fn parse_engine_output(
    response: RawAudioResponse,
    fallback: AudioFormat,
) -> TtsResult<AudioOutput> {
    raw_to_audio_output(response, fallback)
}

#[cfg(test)]
mod command_tests {
    use super::{EngineSettings, SGLangOmniAdapter};
    use crate::types::{EngineMode, TTSRuntimeConfig};
    use std::path::{Path, PathBuf};

    /// The mlx-audio server writes a log into a directory of its own. The
    /// working directory of the process is not always writable: the
    /// application starts the engine from inside its bundle.
    #[test]
    fn the_mlx_audio_server_gets_a_writable_log_directory() {
        let adapter = SGLangOmniAdapter::new(EngineSettings {
            logs_dir: PathBuf::from("/tmp/lm-lathe-logs"),
            ..Default::default()
        });
        let config = TTSRuntimeConfig::new(
            "qwen3-tts-1.7b-voicedesign-mlx",
            "mlx-audio",
            "/models/voicedesign",
            EngineMode::MlxAudio,
        );
        let (_, args) = adapter.build_command(&config, 54321);

        let index = args
            .iter()
            .position(|argument| argument == "--log-dir")
            .expect("the server is told where to write its log");
        let directory = Path::new(&args[index + 1]);
        assert!(directory.is_absolute(), "{directory:?} is a relative path");
        assert!(directory.starts_with("/tmp/lm-lathe-logs"));
    }
}
