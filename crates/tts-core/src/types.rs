//! Core types of the TTS subsystem.

use std::path::PathBuf;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Model families supported by the subsystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelFamily {
    /// Qwen3-TTS (Base / CustomVoice / VoiceDesign checkpoints).
    Qwen3Tts,
    /// Fish Audio Drama 3.
    FishDrama3,
    /// A language model served by llama.cpp.
    Llm,
    /// Synthetic model used for tests and for running the UI without a GPU.
    #[cfg(feature = "mock-engine")]
    Mock,
}

impl ModelFamily {
    /// The memory a checkpoint of this family needs, when it is known.
    ///
    /// The value is the same as the one of the built-in checkpoints, so that
    /// a model of the hub is judged by the same rule.
    pub fn memory_requirement(&self) -> u64 {
        match self {
            ModelFamily::Qwen3Tts => 16 * 1024 * 1024 * 1024,
            ModelFamily::FishDrama3 => 48 * 1024 * 1024 * 1024,
            ModelFamily::Llm => 0,
            #[cfg(feature = "mock-engine")]
            ModelFamily::Mock => 0,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            ModelFamily::Qwen3Tts => "qwen3-tts",
            ModelFamily::FishDrama3 => "fish-drama-3",
            ModelFamily::Llm => "llm",
            #[cfg(feature = "mock-engine")]
            ModelFamily::Mock => "mock",
        }
    }
}

impl std::fmt::Display for ModelFamily {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A capability a TTS model may provide.
///
/// The Voice Design Studio only offers options a model actually supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TtsFeature {
    /// Plain text to speech.
    TextToSpeech,
    /// Voice design from a natural-language description.
    VoiceDesign,
    /// Voice cloning from a reference clip.
    VoiceCloning,
    /// Built-in speakers ("Custom Voice").
    CustomVoice,
    /// Multiple characters in one request.
    MultiSpeaker,
    /// Emotional / performance control.
    EmotionalControl,
    /// Natural-language instruction control.
    InstructionControl,
    /// Inline point tags such as `[sighs]`.
    PointTags,
    /// Streaming output.
    Streaming,
    /// Speech editing / "previous takes".
    AudioEditing,
}

impl TtsFeature {
    pub fn as_str(&self) -> &'static str {
        match self {
            TtsFeature::TextToSpeech => "text_to_speech",
            TtsFeature::VoiceDesign => "voice_design",
            TtsFeature::VoiceCloning => "voice_cloning",
            TtsFeature::CustomVoice => "custom_voice",
            TtsFeature::MultiSpeaker => "multi_speaker",
            TtsFeature::EmotionalControl => "emotional_control",
            TtsFeature::InstructionControl => "instruction_control",
            TtsFeature::PointTags => "point_tags",
            TtsFeature::Streaming => "streaming",
            TtsFeature::AudioEditing => "audio_editing",
        }
    }
}

/// How the inference engine is launched.
///
/// `SglangOmni` is the CUDA path (`sgl-omni serve`), `MlxNative` is the
/// Apple Silicon path (`fishaudio_drama3_server`) and `LlamaCpp` serves the
/// GGUF checkpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineMode {
    #[default]
    SglangOmni,
    MlxNative,
    /// A checkpoint served by the mlx-audio server on Apple Silicon.
    ///
    /// The native MLX runtime only speaks the 0.6B CustomVoice checkpoint,
    /// while mlx-audio reads the 1.7B checkpoints as well and adds the voice
    /// cloning of the Base checkpoints.
    MlxAudio,
    /// A GGUF checkpoint served by llama.cpp.
    LlamaCpp,
    /// The synthetic engine of the tests.
    #[cfg(feature = "mock-engine")]
    Mock,
}

impl EngineMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            EngineMode::SglangOmni => "sglang_omni",
            EngineMode::MlxNative => "mlx_native",
            EngineMode::MlxAudio => "mlx_audio",
            EngineMode::LlamaCpp => "llama_cpp",
            #[cfg(feature = "mock-engine")]
            EngineMode::Mock => "mock",
        }
    }
}

/// A capability a language model may provide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmFeature {
    /// Chat completions.
    Chat,
    /// Plain text completions.
    Completion,
    /// Tool / function calling.
    ToolUse,
    /// Image input.
    Vision,
    /// Reasoning traces.
    Reasoning,
    /// Long context window.
    LongContext,
}

impl LlmFeature {
    pub fn as_str(&self) -> &'static str {
        match self {
            LlmFeature::Chat => "chat",
            LlmFeature::Completion => "completion",
            LlmFeature::ToolUse => "tool_use",
            LlmFeature::Vision => "vision",
            LlmFeature::Reasoning => "reasoning",
            LlmFeature::LongContext => "long_context",
        }
    }
}

/// Audio container / encoding of a synthesis result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioFormat {
    Wav,
    Pcm,
    Mp3,
    Flac,
    Ogg,
    Opus,
    Aac,
}

impl AudioFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            AudioFormat::Wav => "wav",
            AudioFormat::Pcm => "pcm",
            AudioFormat::Mp3 => "mp3",
            AudioFormat::Flac => "flac",
            AudioFormat::Ogg => "ogg",
            AudioFormat::Opus => "opus",
            AudioFormat::Aac => "aac",
        }
    }

    pub fn extension(&self) -> &'static str {
        match self {
            AudioFormat::Wav => "wav",
            AudioFormat::Pcm => "pcm",
            AudioFormat::Mp3 => "mp3",
            AudioFormat::Flac => "flac",
            AudioFormat::Ogg => "ogg",
            AudioFormat::Opus => "opus",
            AudioFormat::Aac => "aac",
        }
    }

    pub fn mime_type(&self) -> &'static str {
        match self {
            AudioFormat::Wav => "audio/wav",
            AudioFormat::Pcm => "audio/pcm",
            AudioFormat::Mp3 => "audio/mpeg",
            AudioFormat::Flac => "audio/flac",
            AudioFormat::Ogg => "audio/ogg",
            AudioFormat::Opus => "audio/opus",
            AudioFormat::Aac => "audio/aac",
        }
    }

    /// Parses a format name as used by the OpenAI-compatible API.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "wav" | "wave" => Some(AudioFormat::Wav),
            "pcm" | "raw" => Some(AudioFormat::Pcm),
            "mp3" => Some(AudioFormat::Mp3),
            "flac" => Some(AudioFormat::Flac),
            "ogg" | "vorbis" => Some(AudioFormat::Ogg),
            "opus" => Some(AudioFormat::Opus),
            "aac" => Some(AudioFormat::Aac),
            _ => None,
        }
    }
}

impl std::fmt::Display for AudioFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A reference clip used for voice cloning.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AudioReference {
    /// Path or URL of the reference clip.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub audio_path: Option<String>,
    /// Base64 encoded WAV payload (required by the MLX native engine).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub data: Option<String>,
    /// Transcript of the reference clip; improves cloning quality a lot.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub text: Option<String>,
    /// Precomputed speaker embedding (x-vector) instead of raw audio.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub speaker_embedding: Option<Vec<f32>>,
}

/// Task selector for Qwen3-TTS checkpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Qwen3TtsTaskType {
    CustomVoice,
    VoiceDesign,
}

impl Qwen3TtsTaskType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Qwen3TtsTaskType::CustomVoice => "CustomVoice",
            Qwen3TtsTaskType::VoiceDesign => "VoiceDesign",
        }
    }
}

/// Qwen3-TTS specific parameters.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Qwen3TtsParameters {
    /// `CustomVoice` (built-in speakers) or `VoiceDesign` (natural-language
    /// voice creation). Inferred from the checkpoint when omitted.
    pub task_type: Option<Qwen3TtsTaskType>,
    /// Natural-language instructions. Required for `VoiceDesign`.
    pub instructions: Option<String>,
    /// Built-in speaker name, e.g. `Vivian` or `Ryan`.
    pub speaker: Option<String>,
    /// Reference clips for voice cloning.
    pub references: Vec<AudioReference>,
    /// Sampling seed.
    pub seed: Option<u64>,
    /// Use the speaker embedding only (no reference audio decoding).
    pub x_vector_only_mode: Option<bool>,
}

/// Fish Audio Drama 3 specific parameters.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FishDrama3Parameters {
    /// Reference clips; index 0 is `<|speaker:0|>`.
    pub references: Vec<AudioReference>,
    /// Previous takes used for speech editing.
    pub previous_takes: Vec<String>,
    /// Sampling seed.
    pub seed: Option<u64>,
}

/// Parameters of the mock engine.
#[cfg(feature = "mock-engine")]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MockParameters {
    /// Frequency of the generated sine wave in Hz.
    pub frequency_hz: Option<f32>,
    /// Duration of the generated audio in milliseconds.
    pub duration_ms: Option<u64>,
}

/// Engine specific parameters of a synthesis request.
///
/// The interpretation of every variant is the responsibility of the matching
/// adapter.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "family", rename_all = "snake_case")]
pub enum EngineSpecificParameters {
    Qwen3Tts(Qwen3TtsParameters),
    FishDrama3(FishDrama3Parameters),
    /// The parameters of the synthetic engine of the tests.
    #[cfg(feature = "mock-engine")]
    Mock(MockParameters),
    /// Escape hatch for engines that are not modelled yet.
    Generic {
        params: serde_json::Value,
    },
}

impl Default for EngineSpecificParameters {
    fn default() -> Self {
        EngineSpecificParameters::Generic {
            params: serde_json::Value::Null,
        }
    }
}

impl EngineSpecificParameters {
    /// Returns the Qwen3-TTS parameters, if any.
    pub fn as_qwen3_tts(&self) -> Option<&Qwen3TtsParameters> {
        match self {
            EngineSpecificParameters::Qwen3Tts(p) => Some(p),
            _ => None,
        }
    }

    /// Returns the Drama 3 parameters, if any.
    pub fn as_fish_drama3(&self) -> Option<&FishDrama3Parameters> {
        match self {
            EngineSpecificParameters::FishDrama3(p) => Some(p),
            _ => None,
        }
    }
}

/// A synthesis request handed to the TTS subsystem.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TTSRequest {
    /// Registry id of the model that should produce the audio.
    pub model_id: String,
    /// Optional voice profile to use.
    pub voice_id: Option<String>,
    /// The text to speak.
    pub text: String,
    /// Language of the text, e.g. `Chinese` or `English`.
    pub language: Option<String>,
    /// Engine specific parameters.
    pub parameters: EngineSpecificParameters,
    /// Requested output format.
    pub response_format: AudioFormat,
    /// Speaking rate multiplier.
    pub speed: Option<f32>,
}

impl TTSRequest {
    /// Creates a plain text-to-speech request.
    pub fn new(model_id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            model_id: model_id.into(),
            voice_id: None,
            text: text.into(),
            language: None,
            parameters: EngineSpecificParameters::default(),
            response_format: AudioFormat::Wav,
            speed: None,
        }
    }

    /// Attaches a voice profile.
    pub fn with_voice(mut self, voice_id: impl Into<String>) -> Self {
        self.voice_id = Some(voice_id.into());
        self
    }

    /// Attaches a language.
    pub fn with_language(mut self, language: impl Into<String>) -> Self {
        self.language = Some(language.into());
        self
    }

    /// Attaches engine specific parameters.
    pub fn with_parameters(mut self, parameters: EngineSpecificParameters) -> Self {
        self.parameters = parameters;
        self
    }
}

/// The raw result of an inference call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioOutput {
    /// Encoded audio bytes.
    #[serde(default, skip_serializing)]
    pub data: Vec<u8>,
    /// Container of `data`.
    pub format: AudioFormat,
    /// Sample rate in Hz, when known.
    pub sample_rate: Option<u32>,
    /// Number of channels, when known.
    pub channels: Option<u16>,
    /// Duration in milliseconds, when known.
    pub duration_ms: Option<u64>,
    /// How the generation finished (`stop`, `length`, `unknown`).
    pub finish_reason: Option<String>,
}

impl AudioOutput {
    /// Creates an output for encoded bytes without metadata.
    pub fn new(data: Vec<u8>, format: AudioFormat) -> Self {
        Self {
            data,
            format,
            sample_rate: None,
            channels: None,
            duration_ms: None,
            finish_reason: None,
        }
    }

    /// MIME type of the payload.
    pub fn mime_type(&self) -> &'static str {
        self.format.mime_type()
    }
}

/// Lifecycle state of a runtime.
///
/// ```text
/// Unloaded -> Loading -> Running -> Unloading -> Unloaded
///                          \-> Failed
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeState {
    Unloaded,
    Loading,
    Running,
    Unloading,
    Failed,
}

impl RuntimeState {
    pub fn as_str(&self) -> &'static str {
        match self {
            RuntimeState::Unloaded => "unloaded",
            RuntimeState::Loading => "loading",
            RuntimeState::Running => "running",
            RuntimeState::Unloading => "unloading",
            RuntimeState::Failed => "failed",
        }
    }
}

/// Health of a running engine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum HealthStatus {
    /// The engine answers requests.
    Healthy,
    /// The engine is still booting.
    Starting,
    /// The engine answers, but something is off.
    Degraded { reason: String },
    /// The engine does not answer.
    Unhealthy { reason: String },
}

impl HealthStatus {
    pub fn is_healthy(&self) -> bool {
        matches!(self, HealthStatus::Healthy)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            HealthStatus::Healthy => "healthy",
            HealthStatus::Starting => "starting",
            HealthStatus::Degraded { .. } => "degraded",
            HealthStatus::Unhealthy { .. } => "unhealthy",
        }
    }
}

/// Handle to a running engine instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeHandle {
    /// Model the instance serves.
    pub model_id: String,
    /// Public repository id of the model, sent to the engine as `model`.
    pub model_repo: Option<String>,
    /// Local directory of the checkpoint.
    ///
    /// The engines that load the checkpoint on their own — mlx-audio names
    /// the directory in the request — need the path on the disk.
    pub model_path: Option<String>,
    /// Engine that hosts the model.
    pub engine_id: String,
    /// OS process id of the engine, when it is a separate process.
    pub process_id: Option<u32>,
    /// Base URL of the engine.
    pub endpoint: Option<String>,
    /// When the instance was started.
    pub started_at: DateTime<Utc>,
    /// File the engine logs are written to.
    pub log_path: Option<PathBuf>,
    /// How the engine was launched.
    pub mode: EngineMode,
}

impl RuntimeHandle {
    /// Endpoint with a guaranteed scheme.
    pub fn base_url(&self) -> Option<&str> {
        self.endpoint.as_deref()
    }
}

/// Configuration used to launch an engine instance.
#[derive(Debug, Clone)]
pub struct TTSRuntimeConfig {
    /// Registry id of the model.
    pub model_id: String,
    /// Engine that hosts the model.
    pub engine_id: String,
    /// Model path or Hugging Face repository id.
    pub model_path: String,
    /// Public model name, used by the native MLX runtime (`--model-name`).
    pub model_name: Option<String>,
    /// Launch mode.
    pub mode: EngineMode,
    /// Port for the engine. `None` picks a free port.
    pub port: Option<u16>,
    /// Extra command line arguments.
    pub extra_args: Vec<String>,
    /// Extra environment variables.
    pub env: Vec<(String, String)>,
    /// How long to wait for the engine to become healthy.
    pub health_timeout: Duration,
    /// Grace period before a forced kill after SIGTERM.
    pub shutdown_timeout: Duration,
}

impl TTSRuntimeConfig {
    /// Creates a configuration with sensible defaults.
    pub fn new(
        model_id: impl Into<String>,
        engine_id: impl Into<String>,
        model_path: impl Into<String>,
        mode: EngineMode,
    ) -> Self {
        Self {
            model_id: model_id.into(),
            engine_id: engine_id.into(),
            model_path: model_path.into(),
            model_name: None,
            mode,
            port: None,
            extra_args: Vec::new(),
            env: Vec::new(),
            health_timeout: Duration::from_secs(600),
            shutdown_timeout: Duration::from_secs(20),
        }
    }
}

/// Raw HTTP answer of the inference engine.
#[derive(Debug, Clone)]
pub struct RawAudioResponse {
    /// Response body.
    pub bytes: Vec<u8>,
    /// Content type reported by the engine.
    pub content_type: Option<String>,
    /// `X-Finish-Reason` header, when present.
    pub finish_reason: Option<String>,
    /// Sample rate header, when present (PCM responses only).
    pub sample_rate: Option<u32>,
    /// Channel count header, when present.
    pub channels: Option<u16>,
}
