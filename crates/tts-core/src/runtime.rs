//! TTS runtime manager.
//!
//! The runtime manager owns the lifecycle of every TTS model:
//!
//! * it asks the global scheduler for resources before a model is started,
//! * it starts and stops the engine through the model adapter,
//! * it enforces the product policy *one model per engine instance*,
//! * it tracks pin state, idle time and the number of in-flight requests, and
//! * it exposes the runtime state to the desktop UI.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ai_scheduler::{
    evictor_from_fn, Evictor, MemoryLease, Scheduler, SchedulerSnapshot, TenantKind, TenantState,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::adapter::factory::AdapterFactory;
use crate::adapter::TTSAdapter;
use crate::audio::AudioManager;
use crate::error::{TtsError, TtsResult};
use crate::hardware::HardwareProfile;
use crate::registry::{CompatibilityReport, TTSModelRegistry, TTSModelSpec};
use crate::types::{AudioOutput, RuntimeState, TTSRequest};
#[cfg(feature = "mock-engine")]
use crate::types::EngineMode;
use crate::voice::VoiceLibrary;

/// State of one TTS model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TTSRuntime {
    /// Model this runtime serves.
    pub model_id: String,
    /// Engine that hosts the model.
    pub engine_id: String,
    /// Lifecycle state.
    pub state: RuntimeState,
    /// Pinned runtimes are never unloaded automatically.
    pub pinned: bool,
    /// Process id of the engine.
    pub process_id: Option<u32>,
    /// Base URL of the engine.
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
    pub checkpoint_bytes: Option<u64>,
}

/// Result of a synthesis request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynthesisResult {
    /// Model that produced the audio.
    pub model_id: String,
    /// Voice profile that was used, when any.
    pub voice_id: Option<String>,
    /// The audio payload.
    pub output: AudioOutput,
    /// File the audio was written to.
    pub file_path: Option<String>,
}

/// Aggregated status of the TTS subsystem.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeStatus {
    /// State of every known model.
    pub runtimes: Vec<TTSRuntime>,
    /// State of every language model.
    #[serde(default)]
    pub llm_runtimes: Vec<crate::llm::LlmRuntime>,
    /// Model that currently occupies the engine.
    pub active_model: Option<String>,
    /// State of the global scheduler.
    pub scheduler: SchedulerSnapshot,
    /// Hardware of the machine.
    pub hardware: HardwareProfile,
}

/// Tunable behaviour of the runtime manager.
#[derive(Debug, Clone)]
pub struct RuntimeSettings {
    /// Idle time after which a model is unloaded automatically.
    pub default_ttl: Duration,
    /// Timeout for acquiring memory from the global scheduler.
    pub acquire_timeout: Duration,
    /// How long to wait for in-flight requests before an unload is aborted.
    pub unload_timeout: Duration,
    /// Whether generated audio is written to disk.
    pub save_audio: bool,
    /// Whether a missing model is loaded on demand.
    pub auto_load: bool,
    /// How long the engine may take to become healthy.
    pub health_timeout: Duration,
}

impl Default for RuntimeSettings {
    fn default() -> Self {
        Self {
            default_ttl: Duration::from_secs(30 * 60),
            acquire_timeout: Duration::from_secs(120),
            unload_timeout: Duration::from_secs(30),
            save_audio: true,
            auto_load: true,
            health_timeout: Duration::from_secs(600),
        }
    }
}

struct ActiveRuntime {
    model_id: String,
    adapter: Arc<dyn TTSAdapter>,
    handle: crate::types::RuntimeHandle,
    lease: MemoryLease,
}

impl Clone for ActiveRuntime {
    fn clone(&self) -> Self {
        Self {
            model_id: self.model_id.clone(),
            adapter: self.adapter.clone(),
            handle: self.handle.clone(),
            lease: self.lease.clone(),
        }
    }
}

struct Inner {
    registry: Arc<TTSModelRegistry>,
    scheduler: Arc<Scheduler>,
    library: Arc<dyn VoiceLibrary>,
    audio: Arc<AudioManager>,
    factory: AdapterFactory,
    hardware: HardwareProfile,
    settings: RuntimeSettings,
    runtimes: Mutex<HashMap<String, TTSRuntime>>,
    active: Mutex<Option<ActiveRuntime>>,
    adapters: Mutex<HashMap<String, Arc<dyn TTSAdapter>>>,
    engine_gate: tokio::sync::Mutex<()>,
    /// Weak handle to ourselves, used by the eviction callbacks.
    self_weak: std::sync::Weak<Inner>,
}

/// Manages the lifecycle of every TTS model.
pub struct TTSRuntimeManager {
    inner: Arc<Inner>,
}

impl TTSRuntimeManager {
    /// Creates a runtime manager.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        registry: Arc<TTSModelRegistry>,
        scheduler: Arc<Scheduler>,
        library: Arc<dyn VoiceLibrary>,
        audio: Arc<AudioManager>,
        factory: AdapterFactory,
        hardware: HardwareProfile,
        settings: RuntimeSettings,
    ) -> Arc<Self> {
        let inner = Arc::new_cyclic(|weak: &std::sync::Weak<Inner>| Inner {
            registry,
            scheduler,
            library,
            audio,
            factory,
            hardware,
            settings,
            runtimes: Mutex::new(HashMap::new()),
            active: Mutex::new(None),
            adapters: Mutex::new(HashMap::new()),
            engine_gate: tokio::sync::Mutex::new(()),
            self_weak: weak.clone(),
        });
        Arc::new(Self { inner })
    }

    /// The model registry.
    pub fn registry(&self) -> &Arc<TTSModelRegistry> {
        &self.inner.registry
    }

    /// The global scheduler.
    pub fn scheduler(&self) -> &Arc<Scheduler> {
        &self.inner.scheduler
    }

    /// The voice library.
    pub fn library(&self) -> &Arc<dyn VoiceLibrary> {
        &self.inner.library
    }

    /// The audio manager.
    pub fn audio(&self) -> &Arc<AudioManager> {
        &self.inner.audio
    }

    /// The detected hardware profile.
    pub fn hardware(&self) -> &HardwareProfile {
        &self.inner.hardware
    }

    /// Returns the runtime state of a model.
    pub fn runtime(&self, model_id: &str) -> Option<TTSRuntime> {
        self.inner.runtime(model_id)
    }

    /// Returns the runtime state of every known model.
    pub fn runtimes(&self) -> Vec<TTSRuntime> {
        self.inner.runtimes()
    }

    /// Loads a model and makes it the active engine instance.
    pub async fn load_model(&self, model_id: &str) -> TtsResult<TTSRuntime> {
        self.inner.load_model(model_id).await
    }

    /// Unloads a model.
    pub async fn unload_model(&self, model_id: &str) -> TtsResult<()> {
        self.inner.unload_model(model_id).await
    }

    /// Synthesises speech.
    ///
    /// The model is loaded on demand when `auto_load` is enabled.
    pub async fn synthesize(&self, request: TTSRequest) -> TtsResult<SynthesisResult> {
        self.inner.synthesize(request).await
    }

    /// Pins or unpins a model.
    pub fn set_pinned(&self, model_id: &str, pinned: bool) -> TtsResult<()> {
        self.inner.set_pinned(model_id, pinned)
    }

    /// `true` when the model is pinned.
    pub fn is_pinned(&self, model_id: &str) -> bool {
        self.inner.is_pinned(model_id)
    }

    /// Returns the engine logs of a model.
    pub async fn logs(&self, model_id: &str) -> TtsResult<String> {
        self.inner.logs(model_id).await
    }

    /// Compatibility report of a model.
    pub fn compatibility(&self, spec: &TTSModelSpec) -> CompatibilityReport {
        self.inner.compatibility(spec)
    }

    /// Aggregated status of the subsystem.
    pub fn status(&self) -> RuntimeStatus {
        self.inner.status()
    }

    /// Stops every runtime.
    pub async fn shutdown(&self) {
        self.inner.shutdown().await;
    }
}

impl Inner {
    /// Tenant key of a model in the global scheduler.
    fn tenant_key(model_id: &str) -> String {
        format!("tts:{model_id}")
    }

    /// Creates the eviction callback of a TTS tenant.
    fn evictor(weak: std::sync::Weak<Inner>) -> Evictor {
        evictor_from_fn(move |key: String| {
            let weak = weak.clone();
            async move {
                let Some(inner) = weak.upgrade() else {
                    return Ok(());
                };
                let model_id = key.strip_prefix("tts:").unwrap_or(&key).to_string();
                inner.unload_model(&model_id).await.map_err(|err| {
                    ai_scheduler::SchedulerError::EvictionFailed {
                        tenant: key.clone(),
                        reason: err.to_string(),
                    }
                })
            }
        })
    }
    /// Returns the runtime state of a model.
    fn runtime(&self, model_id: &str) -> Option<TTSRuntime> {
        self.runtimes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(model_id)
            .cloned()
    }

    /// Returns the runtime state of every known model.
    fn runtimes(&self) -> Vec<TTSRuntime> {
        let mut runtimes: Vec<TTSRuntime> = self
            .runtimes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .cloned()
            .collect();
        // The memory of a running engine is read again, so that the numbers
        // follow the process.
        for runtime in &mut runtimes {
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
        runtimes.sort_by(|a, b| a.model_id.cmp(&b.model_id));
        runtimes
    }

    /// Updates the tracked state of a model.
    fn update_runtime(&self, model_id: &str, update: impl FnOnce(&mut TTSRuntime)) {
        let mut runtimes = self.runtimes.lock().unwrap_or_else(|e| e.into_inner());
        let runtime = runtimes
            .entry(model_id.to_string())
            .or_insert_with(|| TTSRuntime {
                model_id: model_id.to_string(),
                engine_id: String::new(),
                state: RuntimeState::Unloaded,
                pinned: false,
                process_id: None,
                endpoint: None,
                last_used_at: None,
                active_requests: 0,
                estimated_memory: 0,
                actual_memory: None,
                checkpoint_bytes: None,
            });
        update(runtime);
    }

    /// The runtime that currently occupies the engine.
    fn active_runtime(&self) -> Option<ActiveRuntime> {
        self.active
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Loads a model and makes it the active engine instance.
    async fn load_model(&self, model_id: &str) -> TtsResult<TTSRuntime> {
        // Only one model may occupy the engine at a time.
        let _gate = self.engine_gate.lock().await;
        self.load_model_locked(model_id).await
    }

    /// Loads a model. The caller must hold the engine gate.
    async fn load_model_locked(&self, model_id: &str) -> TtsResult<TTSRuntime> {
        let spec = self
            .registry
            .resolve(model_id)
            .ok_or_else(|| TtsError::ModelNotFound(model_id.to_string()))?;

        let report = self.compatibility(&spec);
        if !report.compatible {
            return Err(TtsError::IncompatibleHardware {
                model: spec.id.clone(),
                reason: report.notes.join("; "),
            });
        }

        // The checkpoint has to be on the disk before the engine is started.
        #[cfg(feature = "mock-engine")]
        let needs_checkpoint = spec.mode != EngineMode::Mock;
        #[cfg(not(feature = "mock-engine"))]
        let needs_checkpoint = true;
        if needs_checkpoint {
            let path = self.registry.install_path(&spec.id).unwrap_or_default();
            if !path.is_dir() {
                return Err(TtsError::Engine(format!(
                    "model `{}` is not installed at {} — run `lm-lathe-server models install {}` first",
                    spec.id,
                    path.display(),
                    spec.id
                )));
            }
        }

        if let Some(active) = self.active_runtime() {
            if active.model_id == spec.id {
                return Ok(self
                    .runtime(&spec.id)
                    .unwrap_or_else(|| empty_runtime(&spec)));
            }
        }

        // The currently running model has to go first. A pinned model may not
        // be evicted automatically.
        if let Some(current) = self.active_runtime() {
            if self.is_pinned(&current.model_id) {
                return Err(TtsError::EngineBusy(current.model_id.clone()));
            }
            self.unload_active(current).await?;
        }

        let tenant_key = Self::tenant_key(&spec.id);
        let ttl = spec
            .default_ttl_secs
            .map(Duration::from_secs)
            .unwrap_or(self.settings.default_ttl);
        self.scheduler.register_tenant(
            &tenant_key,
            TenantKind::Tts,
            spec.estimated_memory_bytes,
            Some(ttl),
            Some(Self::evictor(self.self_weak.clone())),
        );

        // The engine has to be there before the memory is taken: the
        // scheduler would evict the other models to make room for a model
        // whose engine cannot even start.
        if self.factory.engine_program(&spec).is_none() {
            self.scheduler.unregister_tenant(&tenant_key);
            return Err(TtsError::Engine(format!(
                "the `{}` runtime is not installed, install it on the runtime page first",
                spec.engine_id
            )));
        }

        let lease = self
            .scheduler
            .acquire(
                &tenant_key,
                spec.estimated_memory_bytes,
                Some(self.settings.acquire_timeout),
            )
            .await?;

        let adapter = self.adapter_for(&spec)?;
        let config = crate::types::TTSRuntimeConfig {
            model_id: spec.id.clone(),
            engine_id: spec.engine_id.clone(),
            model_path: self
                .registry
                .install_path(&spec.id)
                .map(|path| path.to_string_lossy().to_string())
                .unwrap_or_else(|| spec.hf_repo.clone()),
            model_name: Some(spec.hf_repo.clone()),
            mode: spec.mode,
            port: None,
            extra_args: spec.launch_args.clone(),
            env: Vec::new(),
            health_timeout: self.settings.health_timeout,
            shutdown_timeout: Duration::from_secs(20),
        };

        self.update_runtime(&spec.id, |runtime| {
            runtime.engine_id = spec.engine_id.clone();
            runtime.state = RuntimeState::Loading;
            runtime.estimated_memory = spec.estimated_memory_bytes;
        });
        self.scheduler
            .set_state(&tenant_key, TenantState::Loading)
            .ok();

        tracing::info!(model = %spec.id, "loading TTS model");
        let handle = match adapter.start(config).await {
            Ok(handle) => handle,
            Err(err) => {
                lease.release();
                self.scheduler.unregister_tenant(&tenant_key);
                self.update_runtime(&spec.id, |runtime| {
                    runtime.state = RuntimeState::Failed;
                    runtime.endpoint = None;
                    runtime.process_id = None;
                });
                return Err(err);
            }
        };

        let active = ActiveRuntime {
            model_id: spec.id.clone(),
            adapter,
            handle: handle.clone(),
            lease,
        };
        *self.active.lock().unwrap_or_else(|e| e.into_inner()) = Some(active);
        self.scheduler
            .set_state(&tenant_key, TenantState::Running)
            .ok();

        self.update_runtime(&spec.id, |runtime| {
            runtime.state = RuntimeState::Running;
            runtime.process_id = handle.process_id;
            runtime.endpoint = handle.endpoint.clone();
            runtime.last_used_at = Some(Utc::now());
            // The memory of the running engine is measured, not estimated.
            runtime.actual_memory = handle
                .process_id
                .and_then(crate::hardware::process_memory);
        });
        tracing::info!(model = %spec.id, "TTS model is running");
        Ok(self
            .runtime(&spec.id)
            .unwrap_or_else(|| empty_runtime(&spec)))
    }

    /// Unloads a model.
    async fn unload_model(&self, model_id: &str) -> TtsResult<()> {
        let Some(active) = self.active_runtime() else {
            self.update_runtime(model_id, |runtime| {
                if runtime.state != RuntimeState::Failed {
                    runtime.state = RuntimeState::Unloaded;
                }
            });
            return Ok(());
        };
        if active.model_id != model_id {
            self.update_runtime(model_id, |runtime| {
                runtime.state = RuntimeState::Unloaded;
            });
            return Ok(());
        }
        self.unload_active(active).await
    }

    /// Stops the engine and releases every resource.
    async fn unload_active(&self, active: ActiveRuntime) -> TtsResult<()> {
        let tenant_key = Self::tenant_key(&active.model_id);
        self.update_runtime(&active.model_id, |runtime| {
            runtime.state = RuntimeState::Unloading;
        });
        self.scheduler
            .set_state(&tenant_key, TenantState::Unloading)
            .ok();

        // Never force-unload a model that still has work in flight.
        self.wait_for_idle(&active.model_id).await?;

        if let Err(err) = active.adapter.stop(active.handle.clone()).await {
            tracing::warn!(model = %active.model_id, error = %err, "failed to stop engine cleanly");
        }
        active.lease.release();
        self.scheduler.unregister_tenant(&tenant_key);
        *self.active.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.update_runtime(&active.model_id, |runtime| {
            runtime.state = RuntimeState::Unloaded;
            runtime.process_id = None;
            runtime.endpoint = None;
            runtime.active_requests = 0;
        });
        tracing::info!(model = %active.model_id, "TTS model unloaded");
        Ok(())
    }

    /// Waits until the model has no in-flight request left.
    async fn wait_for_idle(&self, model_id: &str) -> TtsResult<()> {
        let deadline = std::time::Instant::now() + self.settings.unload_timeout;
        loop {
            let busy = self
                .runtime(model_id)
                .map(|runtime| runtime.active_requests > 0)
                .unwrap_or(false);
            if !busy {
                return Ok(());
            }
            if std::time::Instant::now() >= deadline {
                return Err(TtsError::Engine(format!(
                    "model `{model_id}` still has in-flight requests and was not unloaded"
                )));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Returns the adapter of a model, creating it on first use.
    fn adapter_for(&self, spec: &TTSModelSpec) -> TtsResult<Arc<dyn TTSAdapter>> {
        let mut adapters = self.adapters.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(adapter) = adapters.get(&spec.id) {
            return Ok(adapter.clone());
        }
        let adapter = self.factory.create(spec)?;
        adapters.insert(spec.id.clone(), adapter.clone());
        Ok(adapter)
    }

    /// Synthesises speech.
    ///
    /// The model is loaded on demand when `auto_load` is enabled.
    async fn synthesize(&self, mut request: TTSRequest) -> TtsResult<SynthesisResult> {
        let spec = self
            .registry
            .resolve(&request.model_id)
            .ok_or_else(|| TtsError::ModelNotFound(request.model_id.clone()))?;
        request.model_id = spec.id.clone();

        // Resolve the voice profile and let the adapter translate it into
        // engine specific parameters. This happens before the engine is
        // touched so a broken profile never blocks the engine gate.
        let adapter = self.adapter_for(&spec)?;
        let voice = match &request.voice_id {
            Some(voice_id) => Some(
                self.library
                    .get(voice_id)
                    .await?
                    .ok_or_else(|| TtsError::VoiceNotFound(voice_id.clone()))?,
            ),
            None => None,
        };
        if let Some(voice) = &voice {
            adapter.apply_voice(&mut request, voice).await?;
        }

        // The engine gate is held while the model is selected and while the
        // request is registered. From that moment on the model may not be
        // unloaded, which protects the request from a concurrent model switch.
        let tenant_key = Self::tenant_key(&spec.id);
        let active = {
            let _gate = self.engine_gate.lock().await;
            let active = self.ensure_active_locked(&spec).await?;
            self.scheduler.begin_request(&tenant_key)?;
            self.update_runtime(&spec.id, |runtime| {
                runtime.active_requests = runtime.active_requests.saturating_add(1);
                runtime.last_used_at = Some(Utc::now());
            });
            active
        };

        let result = active.adapter.synthesize(request.clone()).await;

        self.update_runtime(&spec.id, |runtime| {
            runtime.active_requests = runtime.active_requests.saturating_sub(1);
            runtime.last_used_at = Some(Utc::now());
        });
        self.scheduler.end_request(&tenant_key).ok();

        let mut output = result?;
        let mut file_path = None;
        if self.settings.save_audio {
            let stem = voice
                .as_ref()
                .map(|voice| voice.id.clone())
                .unwrap_or_else(|| spec.id.clone());
            let path = self.audio.save_generated(&output, &stem)?;
            file_path = Some(path.to_string_lossy().to_string());
            if output.format != request.response_format {
                if let Ok(converted) =
                    self.audio
                        .convert(&path, request.response_format, None, None)
                {
                    file_path = Some(converted.to_string_lossy().to_string());
                    output.format = request.response_format;
                }
            }
        }

        Ok(SynthesisResult {
            model_id: spec.id.clone(),
            voice_id: request.voice_id.clone(),
            output,
            file_path,
        })
    }

    /// Makes sure the given model occupies the engine.
    ///
    /// The caller must hold the engine gate.
    async fn ensure_active_locked(&self, spec: &TTSModelSpec) -> TtsResult<ActiveRuntime> {
        if let Some(active) = self.active_runtime() {
            if active.model_id == spec.id {
                return Ok(active);
            }
        }
        if !self.settings.auto_load {
            return Err(TtsError::Engine(format!(
                "model `{}` is not loaded and auto loading is disabled",
                spec.id
            )));
        }
        self.load_model_locked(&spec.id).await?;
        self.active_runtime()
            .ok_or_else(|| TtsError::Engine("failed to load model".to_string()))
    }

    /// Pins or unpins a model.
    fn set_pinned(&self, model_id: &str, pinned: bool) -> TtsResult<()> {
        let tenant_key = Self::tenant_key(model_id);
        self.scheduler.set_pinned(&tenant_key, pinned)?;
        self.update_runtime(model_id, |runtime| runtime.pinned = pinned);
        Ok(())
    }

    /// `true` when the model is pinned.
    fn is_pinned(&self, model_id: &str) -> bool {
        self.runtime(model_id).map(|r| r.pinned).unwrap_or(false)
    }

    /// Returns the engine logs of a model.
    async fn logs(&self, model_id: &str) -> TtsResult<String> {
        match self.active_runtime() {
            Some(active) if active.model_id == model_id => {
                active.adapter.get_logs(&active.handle).await
            }
            // The model is not running: the log of the last start attempt
            // still holds the reason of the failure.
            _ => Ok(self.factory.logs_for(model_id)),
        }
    }

    /// Compatibility report of a model.
    fn compatibility(&self, spec: &TTSModelSpec) -> CompatibilityReport {
        self.registry.check_compatibility(spec, &self.hardware)
    }

    /// Aggregated status of the subsystem.
    fn status(&self) -> RuntimeStatus {
        RuntimeStatus {
            runtimes: self.runtimes(),
            llm_runtimes: Vec::new(),
            active_model: self.active_runtime().map(|active| active.model_id),
            scheduler: self.scheduler.snapshot(),
            hardware: self.hardware.clone(),
        }
    }

    /// Stops every runtime.
    async fn shutdown(&self) {
        let active = { self.active.lock().unwrap_or_else(|e| e.into_inner()).take() };
        if let Some(active) = active {
            let _ = active.adapter.stop(active.handle.clone()).await;
            active.lease.release();
        }
    }
}

/// Builds the runtime entry of a model that was never loaded.
fn empty_runtime(spec: &TTSModelSpec) -> TTSRuntime {
    TTSRuntime {
        model_id: spec.id.clone(),
        engine_id: spec.engine_id.clone(),
        state: RuntimeState::Unloaded,
        pinned: false,
        process_id: None,
        endpoint: None,
        last_used_at: None,
        active_requests: 0,
        estimated_memory: spec.estimated_memory_bytes,
        actual_memory: None,
        checkpoint_bytes: None,
    }
}
