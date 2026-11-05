//! Fish Audio Drama 3 adapter.
//!
//! Drama 3 is the model for expressive, multi-character speech. Its
//! capabilities are **not** mapped onto the Qwen3-TTS voice design
//! parameters, the adapter therefore has its own protocol:
//!
//! * references are indexed, `<|speaker:0|>` refers to `references[0]`,
//! * point tags such as `[sighs]` stay part of the text,
//! * the native Apple Silicon runtime only accepts base64 encoded mono
//!   44.1 kHz WAV references, so local files are converted before they are
//!   sent.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Map, Value};

use crate::adapter::engine::SGLangOmniAdapter;
use crate::adapter::protocol::{references_to_json, voice_hints, RequestContext};
use crate::adapter::{EngineAdapter, ModelProtocol, TTSAdapter};
use crate::error::{TtsError, TtsResult};
use crate::types::{
    AudioFormat, AudioOutput, AudioReference, EngineMode, HealthStatus, ModelFamily,
    RawAudioResponse, RuntimeHandle, TTSRequest, TTSRuntimeConfig, TtsFeature,
};
use crate::voice::VoiceProfile;

/// Protocol implementation for the Fish Audio Drama 3 family.
#[derive(Debug, Default, Clone, Copy)]
pub struct FishDrama3Protocol;

impl FishDrama3Protocol {
    /// Converts a local reference clip into the shape the MLX runtime expects.
    fn to_mlx_reference(reference: &AudioReference) -> TtsResult<AudioReference> {
        let Some(path) = &reference.audio_path else {
            return Ok(reference.clone());
        };
        let path_ref = Path::new(path);
        if !path_ref.exists() {
            // Remote references are refused by the native runtime.
            return Err(TtsError::InvalidRequest(format!(
                "the native MLX runtime only accepts local reference clips, got `{path}`"
            )));
        }
        let data = crate::audio::wav_to_mono_44k_base64(path_ref)?;
        Ok(AudioReference {
            audio_path: None,
            data: Some(data),
            text: reference.text.clone(),
            speaker_embedding: None,
        })
    }
}

impl ModelProtocol for FishDrama3Protocol {
    fn family(&self) -> ModelFamily {
        ModelFamily::FishDrama3
    }

    fn features(&self) -> Vec<TtsFeature> {
        vec![
            TtsFeature::TextToSpeech,
            TtsFeature::VoiceCloning,
            TtsFeature::MultiSpeaker,
            TtsFeature::EmotionalControl,
            TtsFeature::PointTags,
            TtsFeature::AudioEditing,
            TtsFeature::Streaming,
        ]
    }

    fn hydrate(&self, request: &mut TTSRequest, voice: &VoiceProfile) -> TtsResult<()> {
        let hints = voice_hints(&voice.engine_config);
        let mut params = request
            .parameters
            .as_fish_drama3()
            .cloned()
            .unwrap_or_default();
        for (path, text) in hints.references {
            if params.references.iter().any(|r| r.audio_path == path) {
                continue;
            }
            params.references.push(AudioReference {
                audio_path: path,
                text,
                ..Default::default()
            });
        }
        // `character_config.speakers` describes the cast of a scene. Every
        // entry becomes one reference, the position in the list is the
        // speaker index used by `<|speaker:N|>`.
        if let Some(speakers) = voice
            .engine_config
            .get("character_config")
            .and_then(|config| config.get("speakers"))
            .and_then(Value::as_array)
        {
            for speaker in speakers {
                let path = speaker
                    .get("audio_path")
                    .or_else(|| speaker.get("path"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let text = speaker
                    .get("text")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                if path.is_none() {
                    continue;
                }
                params.references.push(AudioReference {
                    audio_path: path,
                    text,
                    ..Default::default()
                });
            }
        }
        request.parameters = crate::types::EngineSpecificParameters::FishDrama3(params);
        Ok(())
    }

    fn build_payload(&self, request: &TTSRequest, ctx: &RequestContext) -> TtsResult<Value> {
        if request.text.trim().is_empty() {
            return Err(TtsError::InvalidRequest(
                "text must not be empty".to_string(),
            ));
        }
        let params = request
            .parameters
            .as_fish_drama3()
            .cloned()
            .unwrap_or_default();
        let mut payload = Map::new();
        if ctx.mode != EngineMode::MlxNative {
            payload.insert("model".to_string(), Value::String(ctx.model_repo.clone()));
        }
        payload.insert("input".to_string(), Value::String(request.text.clone()));

        let references = if ctx.mode == EngineMode::MlxNative {
            params
                .references
                .iter()
                .map(Self::to_mlx_reference)
                .collect::<TtsResult<Vec<_>>>()?
        } else {
            params.references.clone()
        };
        if !references.is_empty() {
            payload.insert(
                "references".to_string(),
                Value::Array(references_to_json(&references)),
            );
        }
        if !params.previous_takes.is_empty() {
            payload.insert(
                "previous_takes".to_string(),
                Value::Array(
                    params
                        .previous_takes
                        .iter()
                        .map(|take| Value::String(take.clone()))
                        .collect(),
                ),
            );
        }
        if let Some(seed) = params.seed {
            if ctx.mode != EngineMode::MlxNative {
                payload.insert("seed".to_string(), Value::from(seed));
            }
        }
        let format = match request.response_format {
            AudioFormat::Pcm => "pcm",
            _ => "wav",
        };
        payload.insert(
            "response_format".to_string(),
            Value::String(format.to_string()),
        );
        payload.insert("stream".to_string(), Value::Bool(false));
        Ok(Value::Object(payload))
    }

    fn parse_output(&self, response: RawAudioResponse) -> TtsResult<AudioOutput> {
        crate::adapter::protocol::raw_to_audio_output(response, AudioFormat::Wav)
    }
}

/// Adapter that serves the Fish Audio Drama 3 family.
pub struct FishDrama3Adapter {
    engine: Arc<SGLangOmniAdapter>,
    protocol: FishDrama3Protocol,
}

impl FishDrama3Adapter {
    /// Creates an adapter on top of an engine.
    pub fn new(engine: Arc<SGLangOmniAdapter>) -> Self {
        Self {
            engine,
            protocol: FishDrama3Protocol,
        }
    }

    /// The protocol used by this adapter.
    pub fn protocol(&self) -> &FishDrama3Protocol {
        &self.protocol
    }
}

#[async_trait]
impl TTSAdapter for FishDrama3Adapter {
    fn logs_for(&self, model_id: &str) -> String {
        self.engine.logs_for(model_id)
    }

    async fn start(&self, config: TTSRuntimeConfig) -> TtsResult<RuntimeHandle> {
        EngineAdapter::start(self.engine.as_ref(), &config).await
    }

    async fn stop(&self, handle: RuntimeHandle) -> TtsResult<()> {
        EngineAdapter::stop(self.engine.as_ref(), &handle).await
    }

    async fn health_check(&self, handle: &RuntimeHandle) -> TtsResult<HealthStatus> {
        EngineAdapter::health_check(self.engine.as_ref(), handle).await
    }

    async fn synthesize(&self, request: TTSRequest) -> TtsResult<AudioOutput> {
        let handle = self
            .engine
            .current_handle()
            .ok_or_else(|| TtsError::Engine("no engine instance is running".to_string()))?;
        let ctx = RequestContext {
            mode: handle.mode,
            model_repo: handle
                .model_repo
                .clone()
                .unwrap_or_else(|| handle.model_id.clone()),
        };
        let payload = self.protocol.build_payload(&request, &ctx)?;
        let raw = EngineAdapter::synthesize_raw(self.engine.as_ref(), &handle, &payload).await?;
        self.protocol.parse_output(raw)
    }

    async fn apply_voice(
        &self,
        request: &mut TTSRequest,
        voice: &crate::voice::VoiceProfile,
    ) -> TtsResult<()> {
        self.protocol.hydrate(request, voice)
    }

    async fn unload_model(&self, handle: &RuntimeHandle) -> TtsResult<()> {
        EngineAdapter::unload_model(self.engine.as_ref(), handle).await
    }

    async fn get_logs(&self, handle: &RuntimeHandle) -> TtsResult<String> {
        EngineAdapter::get_logs(self.engine.as_ref(), handle).await
    }
}
