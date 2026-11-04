//! Service level adapter for the SGLang-Omni runtime.
//!
//! The adapter owns the inference process: it starts `sgl-omni serve` on the
//! CUDA path or `fishaudio_drama3_server` on the Apple Silicon path, waits
//! for the engine to become healthy, proxies inference calls and exposes the
//! engine log file.

use std::collections::HashMap;
use std::path::PathBuf;
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
#[derive(Debug, Clone)]
pub struct EngineSettings {
    /// Binary used for the CUDA path.
    pub sglang_binary: String,
    /// Binary used for the native Apple Silicon path.
    pub mlx_binary: String,
    /// Directory the engine logs are written to.
    pub logs_dir: PathBuf,
    /// Timeout of a single inference request.
    pub request_timeout: Duration,
    /// Interval between two health probes.
    pub health_interval: Duration,
}

impl Default for EngineSettings {
    fn default() -> Self {
        Self {
            sglang_binary: "sgl-omni".to_string(),
            mlx_binary: "fishaudio_drama3_server".to_string(),
            logs_dir: PathBuf::from("logs/tts"),
            request_timeout: Duration::from_secs(600),
            health_interval: Duration::from_millis(750),
        }
    }
}

struct ProcessEntry {
    child: Child,
    log_path: PathBuf,
}

/// Adapter that manages the SGLang-Omni inference service.
pub struct SGLangOmniAdapter {
    client: reqwest::Client,
    settings: EngineSettings,
    processes: Mutex<HashMap<String, ProcessEntry>>,
    current: RwLock<Option<RuntimeHandle>>,
    protocols: RwLock<HashMap<crate::types::ModelFamily, Arc<dyn ModelProtocol>>>,
    registry: RwLock<Option<Arc<crate::registry::TTSModelRegistry>>>,
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
        })
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
                (self.settings.sglang_binary.clone(), args)
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
                (self.settings.mlx_binary.clone(), args)
            }
            EngineMode::Mock => ("mock".to_string(), Vec::new()),
        }
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
    ) -> TtsResult<()> {
        let mut status;
        loop {
            match child.try_wait() {
                Ok(Some(exit)) => {
                    return Err(TtsError::Engine(format!(
                        "engine exited during startup with {exit}"
                    )));
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

        tracing::info!(model = %config.model_id, binary = %binary, ?args, ?pid, "starting engine");

        let deadline = Instant::now() + config.health_timeout;
        match self
            .wait_until_healthy(&mut child, &endpoint, deadline)
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
