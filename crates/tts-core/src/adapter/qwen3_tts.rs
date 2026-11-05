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

/// The kind of a Qwen3-TTS checkpoint.
///
/// The three kinds are told apart by the name of the repository, because the
/// checkpoint itself is the only thing that knows what it can do: a voice
/// design checkpoint writes a voice from a description, a custom voice
/// checkpoint speaks with a built-in speaker and a base checkpoint clones
/// the voice of a reference clip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CheckpointKind {
    VoiceDesign,
    CustomVoice,
    Base,
}

impl CheckpointKind {
    /// The kind implied by the name of the repository.
    fn of(repo: &str) -> Self {
        if repo.contains("VoiceDesign") {
            CheckpointKind::VoiceDesign
        } else if repo.contains("Base") {
            CheckpointKind::Base
        } else {
            CheckpointKind::CustomVoice
        }
    }
}

impl Qwen3TTSProtocol {
    /// Task type implied by the checkpoint.
    fn default_task_type(repo: &str) -> Qwen3TtsTaskType {
        if repo.contains("VoiceDesign") {
            Qwen3TtsTaskType::VoiceDesign
        } else {
            Qwen3TtsTaskType::CustomVoice
        }
    }

    /// The language name mlx-audio expects.
    ///
    /// The checkpoint carries a table of languages that are named in
    /// English; every other spelling is left to the automatic detection.
    fn mlx_language(language: &str) -> String {
        let name = match language.trim().to_lowercase().as_str() {
            "zh" | "zh-cn" | "zh-hans" | "zh-tw" | "chinese" | "中文" => "chinese",
            "en" | "en-us" | "en-gb" | "english" | "英文" => "english",
            "de" | "de-de" | "german" => "german",
            "it" | "it-it" | "italian" => "italian",
            "pt" | "pt-br" | "portuguese" => "portuguese",
            "es" | "es-es" | "spanish" => "spanish",
            "ja" | "ja-jp" | "japanese" => "japanese",
            "ko" | "ko-kr" | "korean" => "korean",
            "fr" | "fr-fr" | "french" => "french",
            "ru" | "ru-ru" | "russian" => "russian",
            _ => "auto",
        };
        name.to_string()
    }

    /// Payload of the mlx-audio server.
    ///
    /// The server reads the checkpoint that the `model` field names and
    /// routes the request by the kind of that checkpoint: a voice design
    /// checkpoint needs `instruct`, a custom voice checkpoint needs `voice`
    /// and a base checkpoint clones with `ref_audio` and `ref_text`.
    fn build_mlx_audio_payload(
        request: &TTSRequest,
        ctx: &RequestContext,
        params: &crate::types::Qwen3TtsParameters,
    ) -> TtsResult<Value> {
        let mut payload = Map::new();
        // The server loads the checkpoint of the request, so the path on the
        // disk is the name of the model.
        let model = ctx
            .model_path
            .clone()
            .unwrap_or_else(|| ctx.model_repo.clone());
        payload.insert("model".to_string(), Value::String(model));
        payload.insert("input".to_string(), Value::String(request.text.clone()));

        let kind = match params.task_type {
            Some(Qwen3TtsTaskType::VoiceDesign) => CheckpointKind::VoiceDesign,
            Some(Qwen3TtsTaskType::CustomVoice) => CheckpointKind::CustomVoice,
            None => CheckpointKind::of(&ctx.model_repo),
        };
        let instructions = params
            .instructions
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());

        match kind {
            CheckpointKind::VoiceDesign => {
                let instructions = instructions.ok_or_else(|| {
                    TtsError::InvalidRequest(
                        "voice design requires a non-empty `instructions` description".to_string(),
                    )
                })?;
                payload.insert(
                    "instruct".to_string(),
                    Value::String(instructions.to_string()),
                );
            }
            CheckpointKind::CustomVoice => {
                let speaker = params
                    .speaker
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty() && *value != "default");
                let speaker = speaker.ok_or_else(|| {
                    TtsError::InvalidRequest(
                        "a custom voice checkpoint needs a speaker, e.g. `Vivian`, `Ryan`, \
                         `Dylan` or `Eric`"
                            .to_string(),
                    )
                })?;
                payload.insert("voice".to_string(), Value::String(speaker.to_string()));
                // The custom voice checkpoint takes the instruction as a
                // control of the style.
                if let Some(instructions) = instructions {
                    payload.insert(
                        "instruct".to_string(),
                        Value::String(instructions.to_string()),
                    );
                }
            }
            CheckpointKind::Base => {
                // The base checkpoint has no built-in speaker: it clones the
                // voice of a reference clip.
                let reference = params.references.first().ok_or_else(|| {
                    TtsError::InvalidRequest(
                        "a base checkpoint clones a voice and needs a reference clip".to_string(),
                    )
                })?;
                payload.insert(
                    "ref_audio".to_string(),
                    Value::String(Self::reference_path(reference)?),
                );
                if let Some(text) = reference.text.as_deref().map(str::trim) {
                    if !text.is_empty() {
                        payload.insert("ref_text".to_string(), Value::String(text.to_string()));
                    }
                }
            }
        }

        if let Some(language) = &request.language {
            payload.insert(
                "lang_code".to_string(),
                Value::String(Self::mlx_language(language)),
            );
        }
        payload.insert("response_format".to_string(), Value::String("wav".to_string()));
        Ok(Value::Object(payload))
    }

    /// The path of a reference clip on the disk.
    ///
    /// mlx-audio reads the file itself, so a clip that is only known as
    /// base64 is written next to the other temporary files first.
    fn reference_path(reference: &crate::types::AudioReference) -> TtsResult<String> {
        if let Some(path) = reference.audio_path.as_deref().map(str::trim) {
            if !path.is_empty() {
                if path.starts_with("http://") || path.starts_with("https://") {
                    return Err(TtsError::InvalidRequest(format!(
                        "the mlx-audio engine needs a local reference clip, but the request \
                         names the URL `{path}`"
                    )));
                }
                return Ok(path.to_string());
            }
        }
        let data = reference.data.as_deref().ok_or_else(|| {
            TtsError::InvalidRequest(
                "the reference clip has neither a path nor a payload".to_string(),
            )
        })?;
        let bytes = base64_decode(data)
            .ok_or_else(|| TtsError::InvalidRequest("the reference clip is not base64".to_string()))?;
        let directory = std::env::temp_dir().join("lm-lathe-references");
        std::fs::create_dir_all(&directory)
            .map_err(|err| TtsError::io(&directory, err))?;
        let path = directory.join(format!("{}.wav", fnv1a(data)));
        if !path.exists() {
            std::fs::write(&path, bytes).map_err(|err| TtsError::io(&path, err))?;
        }
        Ok(path.to_string_lossy().to_string())
    }
}

/// The 64 bit FNV-1a hash of a string, used to name temporary files.
fn fnv1a(value: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// Decodes a base64 payload, ignoring the whitespace of the lines.
fn base64_decode(value: &str) -> Option<Vec<u8>> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut lookup = [255u8; 256];
    for (index, byte) in TABLE.iter().enumerate() {
        lookup[*byte as usize] = index as u8;
    }
    let mut out = Vec::new();
    let mut buffer: u32 = 0;
    let mut bits = 0;
    for byte in value.bytes() {
        if byte.is_ascii_whitespace() || byte == b'=' {
            continue;
        }
        let index = lookup[byte as usize];
        if index == 255 {
            return None;
        }
        buffer = (buffer << 6) | u32::from(index);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Some(out)
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
        if ctx.mode == EngineMode::MlxAudio {
            return Self::build_mlx_audio_payload(request, ctx, &params);
        }
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
            model_path: handle.model_path.clone(),
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
