//! TTS subsystem of the local AI runtime manager.
//!
//! The subsystem is built around five pieces:
//!
//! * [`registry`] knows every TTS model, its capabilities and whether the
//!   current machine can run it,
//! * [`adapter`] hides the inference engine behind a single interface,
//! * [`runtime`] owns the model lifecycle and talks to the global scheduler,
//! * [`voice`] stores the voice assets independently of any runtime, and
//! * [`audio`] stores, inspects and converts the generated audio.

pub mod adapter;
pub mod audio;
pub mod error;
pub mod hardware;
pub mod installer;
pub mod paths;
pub mod registry;
pub mod runtime;
pub mod service;
pub mod types;
pub mod voice;

pub use audio::{AudioInfo, AudioManager};
pub use error::{TtsError, TtsResult};
pub use hardware::HardwareProfile;
pub use installer::{InstallProgress, InstallReport, InstallTracker, ModelInstaller};
pub use paths::AppPaths;
pub use registry::{CompatibilityLevel, CompatibilityReport, TTSModelRegistry, TTSModelSpec};
pub use runtime::{RuntimeSettings, RuntimeStatus, SynthesisResult, TTSRuntime, TTSRuntimeManager};
pub use service::TtsService;
pub use types::{
    AudioFormat, AudioOutput, AudioReference, EngineMode, EngineSpecificParameters,
    FishDrama3Parameters, HealthStatus, ModelFamily, Qwen3TtsParameters, Qwen3TtsTaskType,
    RuntimeHandle, RuntimeState, TTSRequest, TTSRuntimeConfig, TtsFeature,
};
pub use voice::{
    NewVoiceProfile, VoiceFilter, VoiceLibrary, VoicePatch, VoiceProfile, VoiceSample,
};
