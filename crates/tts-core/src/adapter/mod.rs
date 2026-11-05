//! Engine and model adapters.
//!
//! Two levels of abstraction keep the desktop application free of engine
//! specifics:
//!
//! * [`EngineAdapter`] owns the *service* level: starting the inference
//!   process, health checks, log access and the raw HTTP transport.
//! * [`ModelProtocol`] owns the *model* level: translating a [`TTSRequest`]
//!   into the payload the engine expects and parsing the answer back.
//!
//! [`TTSAdapter`] combines both and is the trait the runtime manager talks
//! to.

pub mod engine;
pub mod factory;
pub mod fish_drama3;
pub mod mock;
pub mod protocol;
pub mod qwen3_tts;

use async_trait::async_trait;

use crate::error::TtsResult;
use crate::types::{
    AudioOutput, HealthStatus, RawAudioResponse, RuntimeHandle, TTSRequest, TTSRuntimeConfig,
};

pub use engine::{EngineSettings, SGLangOmniAdapter};
pub use factory::AdapterFactory;
pub use fish_drama3::FishDrama3Adapter;
pub use mock::MockAdapter;
pub use protocol::{ModelProtocol, RequestContext, VoiceHints};
pub use qwen3_tts::Qwen3TTSAdapter;

/// Unified interface to a TTS runtime.
///
/// The desktop application never talks to SGLang-Omni directly, it always
/// goes through an adapter.
#[async_trait]
pub trait TTSAdapter: Send + Sync {
    /// Starts the engine for the given model.
    async fn start(&self, config: TTSRuntimeConfig) -> TtsResult<RuntimeHandle>;

    /// Stops the engine.
    async fn stop(&self, handle: RuntimeHandle) -> TtsResult<()>;

    /// Checks whether the engine answers requests.
    async fn health_check(&self, handle: &RuntimeHandle) -> TtsResult<HealthStatus>;

    /// Synthesises speech.
    async fn synthesize(&self, request: TTSRequest) -> TtsResult<AudioOutput>;

    /// Merges a voice profile into a request.
    ///
    /// The runtime manager resolves the voice profile and hands it to the
    /// adapter, which translates the engine specific part of the profile into
    /// request parameters.
    async fn apply_voice(
        &self,
        _request: &mut TTSRequest,
        _voice: &crate::voice::VoiceProfile,
    ) -> TtsResult<()> {
        Ok(())
    }

    /// Unloads the model from a running engine.
    async fn unload_model(&self, handle: &RuntimeHandle) -> TtsResult<()>;

    /// Returns the engine logs.
    async fn get_logs(&self, handle: &RuntimeHandle) -> TtsResult<String>;

    /// Returns the log of the last start attempt of a model.
    ///
    /// A model that could not be loaded has no runtime, but the engine wrote
    /// the reason of the failure into its log.
    fn logs_for(&self, _model_id: &str) -> String {
        String::new()
    }
}

/// Service level operations of an inference engine.
#[async_trait]
pub trait EngineAdapter: Send + Sync {
    /// Starts the engine process.
    async fn start(&self, config: &TTSRuntimeConfig) -> TtsResult<RuntimeHandle>;

    /// Stops the engine process and waits for the resources to be released.
    async fn stop(&self, handle: &RuntimeHandle) -> TtsResult<()>;

    /// Checks the health of the engine.
    async fn health_check(&self, handle: &RuntimeHandle) -> TtsResult<HealthStatus>;

    /// Unloads the served model.
    async fn unload_model(&self, handle: &RuntimeHandle) -> TtsResult<()>;

    /// Returns the engine logs.
    async fn get_logs(&self, handle: &RuntimeHandle) -> TtsResult<String>;

    /// Performs a raw inference call against the running engine.
    async fn synthesize_raw(
        &self,
        handle: &RuntimeHandle,
        payload: &serde_json::Value,
    ) -> TtsResult<RawAudioResponse>;
}
