//! Convenience assembly of the TTS subsystem.
//!
//! [`TtsService`] wires the registry, the global scheduler, the voice library,
//! the audio manager and the runtime manager together. The desktop
//! application and the CLI both use it.

use std::sync::Arc;
use std::time::Duration;

use ai_scheduler::{Scheduler, SchedulerConfig};

use crate::adapter::engine::EngineSettings;
use crate::adapter::factory::AdapterFactory;
use crate::adopt::{self, AdoptedCheckpoint};
use crate::audio::AudioManager;
use crate::engine_assets::EngineAssetManager;
use crate::error::TtsResult;
use crate::hardware::HardwareProfile;
use crate::installer::{InstallTracker, ModelInstaller};
use crate::llm::LlmRuntimeManager;
use crate::paths::AppPaths;
use crate::registry::TTSModelRegistry;
use crate::runtime::{RuntimeSettings, TTSRuntimeManager};
use crate::voice::fs::FsVoiceLibrary;
use crate::voice::VoiceLibrary;

/// The assembled TTS subsystem.
pub struct TtsService {
    /// Runtime manager.
    pub manager: Arc<TTSRuntimeManager>,
    /// Voice library.
    pub library: Arc<dyn VoiceLibrary>,
    /// Audio manager.
    pub audio: Arc<AudioManager>,
    /// Model registry.
    pub registry: Arc<TTSModelRegistry>,
    /// Global scheduler.
    pub scheduler: Arc<Scheduler>,
    /// Filesystem layout.
    pub paths: AppPaths,
    /// Hardware profile of the machine.
    pub hardware: HardwareProfile,
    /// Downloads the model checkpoints.
    pub installer: Arc<ModelInstaller>,
    /// Progress of the running installations.
    pub installs: InstallTracker,
    /// Owns the installed inference engines.
    pub engines: Arc<EngineAssetManager>,
    /// Runtime of the language models.
    pub llm: Arc<LlmRuntimeManager>,
    /// The checkpoints that were found on the disk and are not in the
    /// catalogue of the application.
    ///
    /// The catalogue is built in the memory, so a model of a search of the
    /// hub is forgotten when the service stops. The checkpoints that were
    /// adopted from the disk are looked up in the hub afterwards, to learn
    /// the things that only the hub knows.
    pub adopted: Vec<AdoptedCheckpoint>,
}

impl TtsService {
    /// Builds the service with the default configuration.
    pub fn new() -> TtsResult<Arc<Self>> {
        Self::with_paths(AppPaths::discover())
    }

    /// Builds the service with an explicit data directory.
    pub fn with_paths(paths: AppPaths) -> TtsResult<Arc<Self>> {
        paths.ensure()?;
        let registry = Arc::new(
            TTSModelRegistry::builtin()
                .with_models_root(paths.tts_models_root())
                .with_llm_models_root(paths.llm_models_root()),
        );
        let hardware = HardwareProfile::detect();
        let installer = Arc::new(
            ModelInstaller::new(paths.tts_models_root()).with_llm_root(paths.llm_models_root()),
        );

        // The checkpoints that are on the disk are adopted into the
        // catalogue: a model that was downloaded from a search of the hub
        // would be forgotten by the catalogue otherwise, even though its
        // files are still there.
        let adopted = adopt::scan(&registry);
        if !adopted.is_empty() {
            tracing::info!(
                count = adopted.len(),
                "the checkpoints on the disk were adopted into the catalogue"
            );
        }
        adopt::apply(&registry, &adopted);

        // The pool covers the unified memory of the machine; a quarter stays
        // available for the operating system and for transient allocations.
        let total = if hardware.total_ram_bytes > 0 {
            hardware.total_ram_bytes
        } else {
            8 * 1024 * 1024 * 1024
        };
        let scheduler = Scheduler::new(SchedulerConfig {
            total_bytes: total,
            reserved_bytes: total / 4,
            default_ttl: Duration::from_secs(30 * 60),
            sweep_interval: Duration::from_secs(60),
            acquire_timeout: Duration::from_secs(120),
        });

        let library: Arc<dyn VoiceLibrary> = Arc::new(FsVoiceLibrary::new(paths.voices_root()));
        let audio = Arc::new(AudioManager::new(&paths));

        let mut engine_settings = EngineSettings {
            logs_dir: paths.tts_logs_root(),
            ..Default::default()
        };
        engine_settings.request_timeout = Duration::from_secs(600);

        // The managed engine installations win over the binaries on the PATH.
        let engines = EngineAssetManager::new(paths.clone())?;
        let resolver = engines.clone();
        engine_settings.binary_resolver = Some(Arc::new(move |engine_id: &str| {
            resolver.resolve_program(engine_id)
        }));

        let factory = AdapterFactory::new(engine_settings);
        let manager = TTSRuntimeManager::new(
            registry.clone(),
            scheduler.clone(),
            library.clone(),
            audio.clone(),
            factory,
            hardware.clone(),
            RuntimeSettings::default(),
        );
        manager.scheduler().spawn_sweeper();

        let llm = LlmRuntimeManager::new(
            registry.clone(),
            scheduler.clone(),
            engines.clone(),
            paths.llm_logs_root(),
        );

        Ok(Arc::new(Self {
            manager,
            library,
            audio,
            registry,
            scheduler,
            paths,
            hardware,
            installer,
            installs: InstallTracker::default(),
            engines,
            llm,
            adopted,
        }))
    }

    /// Stops every runtime.
    pub async fn shutdown(&self) {
        self.manager.shutdown().await;
        self.llm.shutdown().await;
        self.scheduler.shutdown();
    }
}
