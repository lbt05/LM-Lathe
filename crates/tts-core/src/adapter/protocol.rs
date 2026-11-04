//! Model level protocol translation.
//!
//! Every model family turns a [`TTSRequest`] into the JSON payload its
//! engine expects and parses the answer back into an [`AudioOutput`].

use async_trait::async_trait;
use serde_json::Value;

use crate::error::{TtsError, TtsResult};
use crate::types::{
    AudioFormat, AudioOutput, ModelFamily, RawAudioResponse, TTSRequest, TtsFeature,
};

/// Context handed to a protocol while a request is built.
#[derive(Debug, Clone, Default)]
pub struct RequestContext {
    /// How the engine was launched.
    pub mode: crate::types::EngineMode,
    /// Repository id or path of the model.
    pub model_repo: String,
}

/// Model specific parameter translation.
#[async_trait]
pub trait ModelProtocol: Send + Sync {
    /// Family this protocol serves.
    fn family(&self) -> ModelFamily;

    /// Capabilities of the model family.
    fn features(&self) -> Vec<TtsFeature>;

    /// `true` when the family supports the capability.
    fn supports(&self, feature: TtsFeature) -> bool {
        self.features().contains(&feature)
    }

    /// Builds the engine payload for a request.
    fn build_payload(&self, request: &TTSRequest, ctx: &RequestContext) -> TtsResult<Value>;

    /// Merges a voice profile into the request parameters.
    ///
    /// The runtime manager resolves the voice profile and hands it to the
    /// protocol, which translates the engine specific part of the profile
    /// into request parameters.
    fn hydrate(
        &self,
        request: &mut TTSRequest,
        voice: &crate::voice::VoiceProfile,
    ) -> TtsResult<()>;

    /// Parses the engine answer.
    fn parse_output(&self, response: RawAudioResponse) -> TtsResult<AudioOutput>;
}

/// Normalised view of the engine specific part of a voice profile.
///
/// Voice profiles store their engine specific configuration as free-form
/// JSON so a profile survives model upgrades. The protocols read the keys
/// they understand from it.
#[derive(Debug, Clone, Default)]
pub struct VoiceHints {
    /// Natural language voice description or instruction.
    pub instructions: Option<String>,
    /// Built-in speaker name.
    pub speaker: Option<String>,
    /// Reference clips, as `(path, transcript)` pairs.
    pub references: Vec<(Option<String>, Option<String>)>,
    /// Additional values the protocol passes through verbatim.
    pub extra: Value,
}

/// Extracts the well known keys of a voice profile's `engine_config`.
pub fn voice_hints(config: &Value) -> VoiceHints {
    let mut hints = VoiceHints::default();
    let Some(map) = config.as_object() else {
        return hints;
    };
    for key in [
        "voice_design_prompt",
        "instructions",
        "prompt",
        "voice_prompt",
    ] {
        if let Some(value) = map.get(key).and_then(Value::as_str) {
            if !value.trim().is_empty() {
                hints.instructions = Some(value.to_string());
                break;
            }
        }
    }
    for key in ["speaker", "voice", "speaker_name", "default_speaker"] {
        if let Some(value) = map.get(key).and_then(Value::as_str) {
            if !value.trim().is_empty() {
                hints.speaker = Some(value.to_string());
                break;
            }
        }
    }
    if let Some(references) = map.get("references").and_then(Value::as_array) {
        for reference in references {
            match reference {
                Value::String(path) => hints.references.push((Some(path.clone()), None)),
                Value::Object(obj) => {
                    let path = obj
                        .get("audio_path")
                        .or_else(|| obj.get("path"))
                        .or_else(|| obj.get("audio"))
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    let text = obj
                        .get("text")
                        .or_else(|| obj.get("transcript"))
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    hints.references.push((path, text));
                }
                _ => {}
            }
        }
    }
    for key in ["reference_audio", "ref_audio", "reference_clip"] {
        if let Some(path) = map.get(key).and_then(Value::as_str) {
            let text = map
                .get("reference_text")
                .or_else(|| map.get("ref_text"))
                .and_then(Value::as_str)
                .map(str::to_string);
            hints.references.push((Some(path.to_string()), text));
            break;
        }
    }
    hints.extra = Value::Object(map.clone());
    hints
}

/// Turns the engine answer into an [`AudioOutput`].
///
/// WAV payloads are inspected so the caller knows the sample rate, the channel
/// count and the duration.
pub fn raw_to_audio_output(
    response: RawAudioResponse,
    fallback: AudioFormat,
) -> TtsResult<AudioOutput> {
    let format = match response.content_type.as_deref() {
        Some(content_type) if content_type.contains("wav") => AudioFormat::Wav,
        Some(content_type) if content_type.contains("mpeg") => AudioFormat::Mp3,
        Some(content_type) if content_type.contains("flac") => AudioFormat::Flac,
        Some(content_type) if content_type.contains("opus") => AudioFormat::Opus,
        Some(content_type) if content_type.contains("aac") => AudioFormat::Aac,
        Some(content_type) if content_type.contains("ogg") => AudioFormat::Ogg,
        _ => fallback,
    };
    let mut output = AudioOutput::new(response.bytes, format);
    output.finish_reason = response.finish_reason;
    if let Some(rate) = response.sample_rate {
        output.sample_rate = Some(rate);
    }
    if let Some(channels) = response.channels {
        output.channels = Some(channels);
    }
    if output.format == AudioFormat::Wav {
        if let Some(info) = crate::audio::inspect_wav_bytes(&output.data) {
            output.sample_rate = Some(info.sample_rate);
            output.channels = Some(info.channels);
            output.duration_ms = Some(info.duration_ms);
        }
    }
    Ok(output)
}

/// Extracts the reference clips of a request payload.
pub fn references_to_json(references: &[crate::types::AudioReference]) -> Vec<Value> {
    references
        .iter()
        .map(|reference| {
            let mut map = serde_json::Map::new();
            if let Some(path) = &reference.audio_path {
                map.insert("audio_path".to_string(), Value::String(path.clone()));
            }
            if let Some(data) = &reference.data {
                map.insert("data".to_string(), Value::String(data.clone()));
            }
            if let Some(text) = &reference.text {
                map.insert("text".to_string(), Value::String(text.clone()));
            }
            if let Some(embedding) = &reference.speaker_embedding {
                map.insert(
                    "speaker_embedding".to_string(),
                    serde_json::to_value(embedding).unwrap_or(Value::Null),
                );
            }
            Value::Object(map)
        })
        .collect()
}

/// Rejects requests the model cannot serve.
pub fn ensure_supported(protocol: &dyn ModelProtocol, request: &TTSRequest) -> TtsResult<()> {
    if request.text.trim().is_empty() {
        return Err(TtsError::InvalidRequest(
            "text must not be empty".to_string(),
        ));
    }
    let _ = protocol;
    Ok(())
}
