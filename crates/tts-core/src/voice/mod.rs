//! Voice library: the persistent catalogue of the voices a user created.
//!
//! Voice profiles are independent of the inference runtime. They survive the
//! unloading of every model and they are never bound to a running process.

pub mod fs;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::TtsResult;
use crate::types::AudioOutput;

/// A voice asset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceProfile {
    /// Unique identifier.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Natural language description of the voice.
    #[serde(default)]
    pub description: String,
    /// Main language of the voice.
    #[serde(default)]
    pub language: Option<String>,
    /// Gender presentation of the voice.
    #[serde(default)]
    pub gender_presentation: Option<String>,
    /// Age impression of the voice.
    #[serde(default)]
    pub age_style: Option<String>,
    /// Personality and emotional traits.
    #[serde(default)]
    pub personality: Option<String>,
    /// Engine the voice was created with.
    pub engine: String,
    /// Model the voice was created with.
    pub model_id: String,
    /// Engine specific configuration, stored verbatim.
    #[serde(default)]
    pub engine_config: serde_json::Value,
    /// Generated samples, relative to the voice directory.
    #[serde(default)]
    pub samples: Vec<String>,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Last update timestamp.
    pub updated_at: DateTime<Utc>,
}

impl VoiceProfile {
    /// `true` when the voice was created with a voice design workflow.
    pub fn is_voice_design(&self) -> bool {
        self.engine_config
            .get("voice_design_prompt")
            .and_then(serde_json::Value::as_str)
            .is_some()
    }
}

/// Fields used to create a voice profile.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NewVoiceProfile {
    /// Display name. Generated when missing.
    #[serde(default)]
    pub name: Option<String>,
    /// Natural language description.
    #[serde(default)]
    pub description: Option<String>,
    /// Main language.
    #[serde(default)]
    pub language: Option<String>,
    /// Gender presentation.
    #[serde(default)]
    pub gender_presentation: Option<String>,
    /// Age impression.
    #[serde(default)]
    pub age_style: Option<String>,
    /// Personality traits.
    #[serde(default)]
    pub personality: Option<String>,
    /// Engine the voice belongs to.
    #[serde(default)]
    pub engine: Option<String>,
    /// Model the voice belongs to.
    #[serde(default)]
    pub model_id: Option<String>,
    /// Engine specific configuration.
    #[serde(default)]
    pub engine_config: serde_json::Value,
}

impl NewVoiceProfile {
    /// Creates a profile description for the given model.
    pub fn for_model(model_id: impl Into<String>, engine: impl Into<String>) -> Self {
        Self {
            engine: Some(engine.into()),
            model_id: Some(model_id.into()),
            engine_config: serde_json::Value::Null,
            ..Default::default()
        }
    }
}

/// Fields that may be changed on an existing profile.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VoicePatch {
    /// New display name.
    #[serde(default)]
    pub name: Option<String>,
    /// New description.
    #[serde(default)]
    pub description: Option<String>,
    /// New language.
    #[serde(default)]
    pub language: Option<String>,
    /// New gender presentation.
    #[serde(default)]
    pub gender_presentation: Option<String>,
    /// New age impression.
    #[serde(default)]
    pub age_style: Option<String>,
    /// New personality traits.
    #[serde(default)]
    pub personality: Option<String>,
    /// New engine configuration.
    #[serde(default)]
    pub engine_config: Option<serde_json::Value>,
}

/// Filter for listing voices.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VoiceFilter {
    /// Only voices of this model.
    #[serde(default)]
    pub model_id: Option<String>,
    /// Only voices of this engine.
    #[serde(default)]
    pub engine: Option<String>,
    /// Only voices of this language.
    #[serde(default)]
    pub language: Option<String>,
    /// Free text search over name and description.
    #[serde(default)]
    pub query: Option<String>,
}

/// A generated sample of a voice.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceSample {
    /// File name relative to the voice directory.
    pub name: String,
    /// Absolute path on disk.
    pub path: String,
    /// Duration in milliseconds.
    #[serde(default)]
    pub duration_ms: Option<u64>,
}

/// Storage backend of the voice library.
#[async_trait]
pub trait VoiceLibrary: Send + Sync {
    /// Creates and stores a new voice profile.
    async fn create(&self, profile: NewVoiceProfile) -> TtsResult<VoiceProfile>;

    /// Returns a profile.
    async fn get(&self, id: &str) -> TtsResult<Option<VoiceProfile>>;

    /// Lists the profiles matching the filter.
    async fn list(&self, filter: VoiceFilter) -> TtsResult<Vec<VoiceProfile>>;

    /// Updates a profile.
    async fn update(&self, id: &str, patch: VoicePatch) -> TtsResult<VoiceProfile>;

    /// Deletes a profile and all its samples.
    async fn delete(&self, id: &str) -> TtsResult<()>;

    /// Stores a generated sample with the profile.
    async fn add_sample(
        &self,
        id: &str,
        audio: &AudioOutput,
        label: Option<&str>,
    ) -> TtsResult<VoiceSample>;

    /// Removes a sample.
    async fn remove_sample(&self, id: &str, sample: &str) -> TtsResult<()>;

    /// Absolute path of a sample.
    async fn sample_path(&self, id: &str, sample: &str) -> TtsResult<std::path::PathBuf>;

    /// Lists the samples of a profile.
    async fn samples(&self, id: &str) -> TtsResult<Vec<VoiceSample>>;
}
