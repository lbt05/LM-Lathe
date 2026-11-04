//! Qwen3-TTS adapter.
//!
//! The adapter translates a [`TTSRequest`] into the OpenAI compatible payload
//! the SGLang-Omni runtime expects:
//!
//! * `CustomVoice` checkpoints use the built-in speaker in `voice`,
//! * `VoiceDesign` checkpoints require `task_type=VoiceDesign` together with
//!   a non-empty `instructions` field,
//! * `Base` checkpoints clone a voice from `references`.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Map, Value};

use crate::adapter::engine::SGLangOmniAdapter;
use crate::adapter::protocol::{references_to_json, voice_hints, RequestContext};
use crate::adapter::{EngineAdapter, ModelProtocol, TTSAdapter};
use crate::error::{TtsError, TtsResult};
use crate::types::{
    AudioFormat, AudioOutput, EngineMode, HealthStatus, ModelFamily, Qwen3TtsTaskType,
    RawAudioResponse, RuntimeHandle, TTSRequest, TTSRuntimeConfig, TtsFeature,
};
use crate::voice::VoiceProfile;

/// Protocol implementation for the Qwen3-TTS family.
#[derive(Debug, Default, Clone, Copy)]
pub struct Qwen3TTSProtocol;

impl Qwen3TTSProtocol {
    /// Task type implied by the checkpoint.
    fn default_task_type(repo: &str) -> Qwen3TtsTaskType {
        if repo.contains("VoiceDesign") {
            Qwen3TtsTaskType::VoiceDesign
        } else {
            Qwen3TtsTaskType::CustomVoice
        }
    }
}

impl ModelProtocol for Qwen3TTSProtocol {
    fn family(&self) -> ModelFamily {
        ModelFamily::Qwen3Tts
    }

    fn features(&self) -> Vec<TtsFeature> {
        vec![
            TtsFeature::TextToSpeech,
            TtsFeature::VoiceDesign,
            TtsFeature::VoiceCloning,
            TtsFeature::CustomVoice,
            TtsFeature::InstructionControl,
            TtsFeature::Streaming,
        ]
    }

    fn hydrate(&self, request: &mut TTSRequest, voice: &VoiceProfile) -> TtsResult<()> {
        let hints = voice_hints(&voice.engine_config);
        let mut params = request
            .parameters
            .as_qwen3_tts()
            .cloned()
            .unwrap_or_default();
        if params.instructions.is_none() {
            params.instructions = hints.instructions;
        }
        if params.speaker.is_none() {
            params.speaker = hints.speaker;
        }
        for (path, text) in hints.references {
            if params.references.iter().any(|r| r.audio_path == path) {
                continue;
            }
            params.references.push(crate::types::AudioReference {
                audio_path: path,
                text,
                ..Default::default()
            });
        }
        request.parameters = crate::types::EngineSpecificParameters::Qwen3Tts(params);
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
            .as_qwen3_tts()
            .cloned()
            .unwrap_or_default();
        let mut payload = Map::new();

        if ctx.mode != EngineMode::MlxNative {
            payload.insert("model".to_string(), Value::String(ctx.model_repo.clone()));
        }
        payload.insert("input".to_string(), Value::String(request.text.clone()));
        payload.insert(
            "voice".to_string(),
            Value::String(
                params
                    .speaker
                    .clone()
                    .unwrap_or_else(|| "default".to_string()),
            ),
        );
        if let Some(language) = &request.language {
            payload.insert("language".to_string(), Value::String(language.clone()));
        }

        let task = params
            .task_type
            .unwrap_or_else(|| Self::default_task_type(&ctx.model_repo));
        match task {
            Qwen3TtsTaskType::VoiceDesign => {
                let instructions = params
                    .instructions
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| {
                        TtsError::InvalidRequest(
                            "voice design requires a non-empty `instructions` description"
                                .to_string(),
                        )
                    })?;
                payload.insert(
                    "task_type".to_string(),
                    Value::String("VoiceDesign".to_string()),
                );
                payload.insert(
                    "instructions".to_string(),
                    Value::String(instructions.to_string()),
                );
            }
            Qwen3TtsTaskType::CustomVoice => {
                // The runtime accepts an omitted `task_type` for CustomVoice.
                if let Some(instructions) = params.instructions.as_deref() {
                    if !instructions.trim().is_empty() {
                        payload.insert(
                            "instructions".to_string(),
                            Value::String(instructions.to_string()),
                        );
                    }
                }
            }
        }

        if !params.references.is_empty() {
            payload.insert(
                "references".to_string(),
                Value::Array(references_to_json(&params.references)),
            );
        }
        if let Some(x_vector) = params.x_vector_only_mode {
            payload.insert("x_vector_only_mode".to_string(), Value::Bool(x_vector));
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

/// Adapter that serves the Qwen3-TTS family.
pub struct Qwen3TTSAdapter {
    engine: Arc<SGLangOmniAdapter>,
    protocol: Qwen3TTSProtocol,
}

impl Qwen3TTSAdapter {
    /// Creates an adapter on top of an engine.
    pub fn new(engine: Arc<SGLangOmniAdapter>) -> Self {
        Self {
            engine,
            protocol: Qwen3TTSProtocol,
        }
    }

    /// The protocol used by this adapter.
    pub fn protocol(&self) -> &Qwen3TTSProtocol {
        &self.protocol
    }
}

#[async_trait]
impl TTSAdapter for Qwen3TTSAdapter {
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
