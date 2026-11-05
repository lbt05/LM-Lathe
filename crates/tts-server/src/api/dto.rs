//! Request and response payloads of the HTTP API.

use serde::{Deserialize, Serialize};
use tts_core::types::{AudioFormat, Qwen3TtsTaskType};

/// A reference clip in a request.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReferenceDto {
    /// Path or URL of the clip.
    #[serde(default)]
    pub audio_path: Option<String>,
    /// Base64 encoded clip.
    #[serde(default)]
    pub data: Option<String>,
    /// Transcript of the clip.
    #[serde(default)]
    pub text: Option<String>,
}

/// OpenAI compatible speech request.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SpeechRequest {
    /// Model id or alias.
    #[serde(default)]
    pub model: String,
    /// The text to speak.
    #[serde(default)]
    pub input: String,
    /// Voice profile id.
    #[serde(default)]
    pub voice: Option<String>,
    /// `wav`, `mp3`, `flac`, `opus`, `aac` or `pcm`.
    #[serde(default)]
    pub response_format: Option<String>,
    /// Speaking rate.
    #[serde(default)]
    pub speed: Option<f32>,
    /// Language of the text.
    #[serde(default)]
    pub language: Option<String>,
    /// Natural language instructions, e.g. a voice description.
    #[serde(default)]
    pub instructions: Option<String>,
    /// `CustomVoice` or `VoiceDesign`.
    #[serde(default)]
    pub task_type: Option<String>,
    /// Reference clips for voice cloning.
    #[serde(default)]
    pub references: Option<Vec<ReferenceDto>>,
    /// Sampling seed.
    #[serde(default)]
    pub seed: Option<u64>,
    /// Use the speaker embedding only.
    #[serde(default)]
    pub x_vector_only_mode: Option<bool>,
    /// Previous takes for speech editing (Drama 3).
    #[serde(default)]
    pub previous_takes: Option<Vec<String>>,
}

impl SpeechRequest {
    /// The requested output format, defaulting to WAV.
    pub fn format(&self) -> AudioFormat {
        self.response_format
            .as_deref()
            .and_then(AudioFormat::parse)
            .unwrap_or(AudioFormat::Wav)
    }

    /// The requested task type, when the caller named one.
    pub fn task(&self) -> Option<Qwen3TtsTaskType> {
        self.task_type.as_deref().and_then(|value| match value {
            "VoiceDesign" | "voice_design" | "voicedesign" => Some(Qwen3TtsTaskType::VoiceDesign),
            "CustomVoice" | "custom_voice" | "customvoice" => Some(Qwen3TtsTaskType::CustomVoice),
            _ => None,
        })
    }
}

/// Extended synthesis request with the engine specific parameters.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SpeakRequest {
    /// Model id or alias.
    #[serde(default)]
    pub model: String,
    /// The text to speak.
    #[serde(default)]
    pub input: String,
    /// Voice profile id.
    #[serde(default)]
    pub voice_id: Option<String>,
    /// Language of the text.
    #[serde(default)]
    pub language: Option<String>,
    /// Requested output format.
    #[serde(default)]
    pub response_format: Option<String>,
    /// Engine specific parameters, kept verbatim.
    #[serde(default)]
    pub parameters: Option<serde_json::Value>,
}

/// Voice design request.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct VoiceDesignRequest {
    /// Model that creates the voice.
    #[serde(default)]
    pub model: String,
    /// Natural language description of the wanted voice.
    #[serde(default)]
    pub prompt: String,
    /// Display name.
    #[serde(default)]
    pub name: Option<String>,
    /// Language of the voice.
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
    /// Text used for the preview sample.
    #[serde(default)]
    pub sample_text: Option<String>,
    /// Voice profile the new voice is based on.
    #[serde(default)]
    pub base_voice_id: Option<String>,
}

/// Voice cloning request.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct VoiceCloneRequest {
    /// Model that clones the voice.
    #[serde(default)]
    pub model: String,
    /// Reference clip.
    #[serde(default)]
    pub reference_audio: Option<String>,
    /// Transcript of the reference clip.
    #[serde(default)]
    pub reference_text: Option<String>,
    /// Display name.
    #[serde(default)]
    pub name: Option<String>,
    /// Description.
    #[serde(default)]
    pub description: Option<String>,
    /// Language of the voice.
    #[serde(default)]
    pub language: Option<String>,
    /// Text used for the preview sample.
    #[serde(default)]
    pub sample_text: Option<String>,
}

/// Create voice request.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CreateVoiceRequest {
    /// Display name.
    #[serde(default)]
    pub name: Option<String>,
    /// Description.
    #[serde(default)]
    pub description: Option<String>,
    /// Language.
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
    /// Engine.
    #[serde(default)]
    pub engine: Option<String>,
    /// Model.
    #[serde(default)]
    pub model_id: Option<String>,
    /// Engine specific configuration.
    #[serde(default)]
    pub engine_config: Option<serde_json::Value>,
}

/// Update voice request.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct UpdateVoiceRequest {
    /// Display name.
    #[serde(default)]
    pub name: Option<String>,
    /// Description.
    #[serde(default)]
    pub description: Option<String>,
    /// Language.
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
    /// Engine specific configuration.
    #[serde(default)]
    pub engine_config: Option<serde_json::Value>,
}

/// Filter for listing voices.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ListVoicesQuery {
    /// Only voices of this model.
    #[serde(default)]
    pub model: Option<String>,
    /// Only voices of this engine.
    #[serde(default)]
    pub engine: Option<String>,
    /// Only voices of this language.
    #[serde(default)]
    pub language: Option<String>,
    /// Free text search.
    #[serde(default)]
    pub query: Option<String>,
}

/// A model entry of the registry.
#[derive(Debug, Clone, Serialize)]
pub struct ModelDto {
    /// Model id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Model family.
    pub family: String,
    /// Engine that serves the model.
    pub engine_id: String,
    /// Repository id.
    pub hf_repo: String,
    /// Version.
    pub version: String,
    /// Checkpoint format.
    pub format: String,
    /// Size on disk.
    pub size_bytes: u64,
    /// Expected memory usage.
    pub estimated_memory_bytes: u64,
    /// Capabilities.
    pub features: Vec<String>,
    /// Capabilities of a language model.
    #[serde(default)]
    pub llm_features: Vec<String>,
    /// Compatibility with the current machine.
    pub compatible: bool,
    /// Compatibility level.
    pub compatibility: String,
    /// Compatibility notes.
    pub notes: Vec<String>,
    /// `true` when the checkpoint is present on disk.
    pub installed: bool,
    /// Where the checkpoint is stored.
    pub path: Option<std::path::PathBuf>,
    /// Runtime state.
    pub state: String,
    /// `true` when the model is pinned.
    pub pinned: bool,
}

/// A model of the Hugging Face Hub that the runtime can serve.
#[derive(Debug, Clone, Serialize)]
pub struct DiscoveredModelDto {
    /// The id the model is known by in the API and on disk.
    pub id: String,
    /// The `organisation/repository` id of the hub.
    pub repo: String,
    /// Display name.
    pub name: String,
    /// Model family, `llm` or `qwen3-tts`.
    pub family: String,
    /// Checkpoint format.
    pub format: String,
    /// The engine that serves the model.
    pub engine_id: String,
    /// Downloads in the last month.
    pub downloads: u64,
    /// How many users marked the model.
    pub likes: u64,
    /// When the repository was touched the last time.
    pub last_modified: Option<String>,
    /// The licence of the repository.
    pub license: Option<String>,
    /// The quantisation of the checkpoint that would be downloaded.
    pub quant: Option<String>,
    /// The checkpoint files of the repository.
    pub files: Vec<String>,
    /// `true` when the checkpoint is already on the disk.
    pub installed: bool,
}

/// The answer of a search of the hub.
#[derive(Debug, Clone, Serialize)]
pub struct DiscoveredModelsDto {
    /// The models the runtime of the application can serve.
    pub models: Vec<DiscoveredModelDto>,
    /// The repositories that were found but dropped, because the runtime
    /// cannot serve them.
    pub dropped: usize,
}

/// A voice profile.
#[derive(Debug, Clone, Serialize)]
pub struct VoiceDto {
    /// Voice id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Description.
    pub description: String,
    /// Language.
    pub language: Option<String>,
    /// Gender presentation.
    pub gender_presentation: Option<String>,
    /// Age impression.
    pub age_style: Option<String>,
    /// Personality traits.
    pub personality: Option<String>,
    /// Engine.
    pub engine: String,
    /// Model.
    pub model_id: String,
    /// Engine specific configuration.
    pub engine_config: serde_json::Value,
    /// Stored samples.
    pub samples: Vec<String>,
    /// Creation timestamp.
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Last update timestamp.
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Result of a voice design or cloning request.
#[derive(Debug, Clone, Serialize)]
pub struct VoiceCreationResult {
    /// The stored profile.
    pub voice: VoiceDto,
    /// The preview sample.
    pub sample: Option<String>,
    /// File the preview sample was written to.
    pub sample_path: Option<String>,
}

/// Error body of the API.
#[derive(Debug, Clone, Serialize)]
pub struct ErrorDto {
    /// Machine readable error type.
    pub kind: String,
    /// Human readable message.
    pub message: String,
}
