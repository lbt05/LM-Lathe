//! TTS subsystem of the local AI runtime manager.
//!
//! The subsystem is built around five pieces:
//!
//! * [`registry`] knows every TTS model, its capabilities and whether the
//!   current machine can run it,
//! * [`adapter`] hides the inference engine behind a single interface,
//! * [`engine_assets`] owns the engines themselves: their versions, their
//!   installation and the version that is active,
//! * [`runtime`] owns the model lifecycle and talks to the global scheduler,
//! * [`voice`] stores the voice assets independently of any runtime, and
//! * [`audio`] stores, inspects and converts the generated audio.

pub mod adapter;
pub mod adopt;
pub mod audio;
pub mod engine_assets;
pub mod error;
pub mod hardware;
pub mod hub;
pub mod installer;
pub mod llm;
pub mod paths;
pub mod registry;
pub mod runtime;
pub mod service;
pub mod types;
pub mod voice;

pub use adopt::AdoptedCheckpoint;
pub use audio::{AudioInfo, AudioManager};
pub use engine_assets::{
    engine_definition, engine_definitions, EngineAsset, EngineDefinition, EngineDistribution,
    EngineHealth, EngineInstallProgress, EngineVersion, EngineAssetManager, InstalledEngine,
};
pub use error::{TtsError, TtsResult};
pub use hardware::HardwareProfile;
pub use hub::{HubClient, HubModel};
pub use installer::{
    directory_size, InstallProgress, InstallReport, InstallTracker, ModelInstaller,
};
pub use llm::{gguf_checkpoint, LlmRuntime, LlmRuntimeManager};
pub use paths::AppPaths;
pub use registry::{CompatibilityLevel, CompatibilityReport, TTSModelRegistry, TTSModelSpec};
pub use runtime::{RuntimeSettings, RuntimeStatus, SynthesisResult, TTSRuntime, TTSRuntimeManager};
pub use service::TtsService;
pub use types::{
    AudioFormat, AudioOutput, AudioReference, EngineMode, EngineSpecificParameters,
    FishDrama3Parameters, HealthStatus, ModelFamily, Qwen3TtsParameters, Qwen3TtsTaskType,
    LlmFeature, RuntimeHandle, RuntimeState, TTSRequest, TTSRuntimeConfig, TtsFeature,
};
pub use voice::{
    NewVoiceProfile, VoiceFilter, VoiceLibrary, VoicePatch, VoiceProfile, VoiceSample,
};
