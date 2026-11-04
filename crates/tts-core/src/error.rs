//! Errors of the TTS subsystem.

use std::path::PathBuf;

use crate::types::AudioFormat;

/// Result alias used across the TTS subsystem.
pub type TtsResult<T> = Result<T, TtsError>;

/// Everything that can go wrong in the TTS subsystem.
#[derive(Debug, thiserror::Error)]
pub enum TtsError {
    /// The requested model is not in the registry.
    #[error("unknown TTS model `{0}`")]
    ModelNotFound(String),

    /// No adapter is registered for the model family.
    #[error("no engine adapter for model family `{0}`")]
    AdapterNotFound(String),

    /// The model does not implement the requested feature.
    #[error("model `{model}` does not support `{feature}`")]
    UnsupportedFeature { model: String, feature: String },

    /// The request is malformed.
    #[error("invalid request: {0}")]
    InvalidRequest(String),

    /// The voice profile does not exist.
    #[error("unknown voice profile `{0}`")]
    VoiceNotFound(String),

    /// A voice profile with the same id already exists.
    #[error("voice profile `{0}` already exists")]
    VoiceExists(String),

    /// The engine is busy with another model that is pinned.
    #[error("engine is occupied by pinned model `{0}`")]
    EngineBusy(String),

    /// The engine failed to start or crashed.
    #[error("engine error: {0}")]
    Engine(String),

    /// The health check did not succeed.
    #[error("health check failed: {0}")]
    HealthCheckFailed(String),

    /// The global scheduler rejected the request.
    #[error("scheduler error: {0}")]
    Scheduler(String),

    /// The hardware cannot run the model.
    #[error("model `{model}` is not supported on this machine: {reason}")]
    IncompatibleHardware { model: String, reason: String },

    /// A file system error.
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// A (de)serialisation error.
    #[error("serialization error: {0}")]
    Serialization(String),

    /// The requested audio format cannot be produced.
    #[error("cannot convert audio to {0}")]
    UnsupportedFormat(AudioFormat),

    /// The operation timed out.
    #[error("operation timed out: {0}")]
    Timeout(String),

    /// The operation was cancelled.
    #[error("operation cancelled: {0}")]
    Cancelled(String),

    /// Catch-all for unexpected failures.
    #[error("{0}")]
    Internal(String),
}

impl TtsError {
    /// Wraps an [`std::io::Error`] together with the path it happened on.
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        TtsError::Io {
            path: path.into(),
            source,
        }
    }

    /// Wraps a [`serde_json::Error`].
    pub fn serialization(source: serde_json::Error) -> Self {
        TtsError::Serialization(source.to_string())
    }

    /// HTTP status code that best represents the error.
    pub fn status_code(&self) -> u16 {
        match self {
            TtsError::ModelNotFound(_) | TtsError::VoiceNotFound(_) => 404,
            TtsError::VoiceExists(_) => 409,
            TtsError::InvalidRequest(_) => 400,
            TtsError::UnsupportedFeature { .. } => 400,
            TtsError::EngineBusy(_) => 409,
            TtsError::IncompatibleHardware { .. } => 409,
            TtsError::UnsupportedFormat(_) => 415,
            TtsError::Timeout(_) => 504,
            TtsError::Cancelled(_) => 499,
            TtsError::Scheduler(_) => 503,
            _ => 500,
        }
    }
}

impl From<crate::types::AudioFormat> for TtsError {
    fn from(value: AudioFormat) -> Self {
        TtsError::UnsupportedFormat(value)
    }
}

impl From<ai_scheduler::SchedulerError> for TtsError {
    fn from(value: ai_scheduler::SchedulerError) -> Self {
        TtsError::Scheduler(value.to_string())
    }
}
