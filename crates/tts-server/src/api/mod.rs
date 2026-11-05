//! HTTP API of the TTS subsystem.
//!
//! The API offers
//!
//! * an OpenAI compatible endpoint (`POST /v1/audio/speech`),
//! * internal extension endpoints for voice design, voice cloning and the
//!   advanced synthesis features that do not fit into the OpenAI schema, and
//! * lifecycle endpoints to load, unload, pin and inspect the models.

pub mod dto;
pub mod runtimes;

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use tts_core::error::TtsError;
use tts_core::registry::TTSModelSpec;
use tts_core::service::TtsService;
use tts_core::types::{AudioReference, EngineSpecificParameters, FishDrama3Parameters, MockParameters, Qwen3TtsParameters, Qwen3TtsTaskType, RuntimeState, TTSRequest, TtsFeature};
use tts_core::voice::{NewVoiceProfile, VoiceFilter, VoicePatch};

use dto::*;

/// Shared state of the API.
#[derive(Clone)]
pub struct ApiState {
    /// The TTS subsystem.
    pub service: Arc<TtsService>,
}

/// Builds the router.
pub fn router(service: Arc<TtsService>) -> Router {
    let state = ApiState { service };
    Router::new()
        .route("/health", get(health))
        .route("/v1/models", get(list_models))
        .route("/v1/tts/models", get(list_catalogue))
        .route("/v1/tts/status", get(status))
        .route("/v1/audio/speech", post(speech))
        .route("/v1/tts/speak", post(speak))
        .route("/v1/tts/voices", get(list_voices).post(create_voice))
        .route("/v1/tts/voices/design", post(design_voice))
        .route("/v1/tts/voices/clone", post(clone_voice))
        .route(
            "/v1/tts/voices/{id}",
            get(get_voice).patch(update_voice).delete(delete_voice),
        )
        .route("/v1/tts/voices/{id}/samples", get(list_samples))
        .route("/v1/tts/voices/{id}/samples/{*name}", get(get_sample))
        .route("/v1/tts/models/{id}/load", post(load_model))
        .route("/v1/tts/models/{id}/unload", post(unload_model))
        .route("/v1/tts/models/{id}/pin", post(pin_model))
        .route("/v1/tts/models/{id}/unpin", post(unpin_model))
        .route("/v1/tts/models/{id}/install", post(install_model))
        .route("/v1/tts/models/{id}/install", get(install_progress))
        .route("/v1/tts/models/{id}/uninstall", post(uninstall_model))
        .route("/v1/tts/models/{id}/logs", get(model_logs))
        .route("/v1/runtimes", get(runtimes::list_runtimes))
        .route("/v1/runtimes/{id}", get(runtimes::get_runtime))
        .route("/v1/runtimes/{id}/versions", get(runtimes::list_versions))
        .route("/v1/runtimes/{id}/install", post(runtimes::install))
        .route(
            "/v1/runtimes/{id}/install-progress",
            get(runtimes::install_progress),
        )
        .route("/v1/runtimes/{id}/activate", post(runtimes::activate))
        .route("/v1/runtimes/{id}/uninstall", post(runtimes::uninstall))
        .route("/v1/runtimes/{id}/health", post(runtimes::health))
        .route("/v1/audio/{file}", get(get_audio))
        .layer(tower_http::cors::CorsLayer::permissive())
        .layer(
            tower_http::trace::TraceLayer::new_for_http()
                .make_span_with(|request: &axum::http::Request<_>| {
                    tracing::info!(
                        method = %request.method(),
                        path = %request.uri().path(),
                        "request"
                    );
                    tracing::Span::current()
                })
                .on_response(
                    |response: &axum::http::Response<_>,
                     latency: std::time::Duration,
                     _span: &tracing::Span| {
                        tracing::info!(status = response.status().as_u16(), ?latency, "response");
                    },
                ),
        )
        .with_state(state)
}

/// Error type of the API.
pub struct ApiError(pub TtsError);

impl From<TtsError> for ApiError {
    fn from(value: TtsError) -> Self {
        ApiError(value)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status =
            StatusCode::from_u16(self.0.status_code()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let body = serde_json::to_vec(&ErrorDto {
            kind: kind_of(&self.0),
            message: self.0.to_string(),
        })
        .unwrap_or_default();
        (status, [(header::CONTENT_TYPE, "application/json")], body).into_response()
    }
}

/// Machine readable name of an error.
fn kind_of(error: &TtsError) -> String {
    match error {
        TtsError::ModelNotFound(_) => "model_not_found".to_string(),
        TtsError::VoiceNotFound(_) => "voice_not_found".to_string(),
        TtsError::VoiceExists(_) => "voice_exists".to_string(),
        TtsError::InvalidRequest(_) => "invalid_request".to_string(),
        TtsError::UnsupportedFeature { .. } => "unsupported_feature".to_string(),
        TtsError::EngineBusy(_) => "engine_busy".to_string(),
        TtsError::IncompatibleHardware { .. } => "incompatible_hardware".to_string(),
        TtsError::UnsupportedFormat(_) => "unsupported_format".to_string(),
        TtsError::Timeout(_) => "timeout".to_string(),
        TtsError::Cancelled(_) => "cancelled".to_string(),
        TtsError::Scheduler(_) => "scheduler_error".to_string(),
        TtsError::Engine(_) => "engine_error".to_string(),
        TtsError::HealthCheckFailed(_) => "health_check_failed".to_string(),
        _ => "internal_error".to_string(),
    }
}

/// `GET /health`
async fn health(State(state): State<ApiState>) -> impl IntoResponse {
    let status = state.service.manager.status();
    axum::Json(serde_json::json!({
        "status": "ok",
        "active_model": status.active_model,
        "available_bytes": status.scheduler.available_bytes,
    }))
}

/// `GET /v1/tts/models`
async fn list_models(State(state): State<ApiState>) -> Result<impl IntoResponse, ApiError> {
    let runtimes = state.service.manager.runtimes();
    let models: Vec<ModelDto> = state
        .service
        .registry
        .list()
        .into_iter()
        .filter(|spec| is_servable(&state, spec))
        .map(|spec| model_dto(&state, &spec, &runtimes))
        .collect();
    Ok(axum::Json(models))
}

/// `GET /v1/tts/models`
///
/// The full catalogue of the known models, including the ones that are not
/// on the disk yet.
async fn list_catalogue(State(state): State<ApiState>) -> Result<impl IntoResponse, ApiError> {
    let runtimes = state.service.manager.runtimes();
    let models: Vec<ModelDto> = state
        .service
        .registry
        .list()
        .into_iter()
        .map(|spec| model_dto(&state, &spec, &runtimes))
        .collect();
    Ok(axum::Json(models))
}

/// `true` when the checkpoint of the model is on the disk and the machine can
/// run it, so that the server is able to serve it.
fn is_servable(state: &ApiState, spec: &TTSModelSpec) -> bool {
    if !state.service.registry.is_installed(&spec.id) {
        return false;
    }
    state.service.manager.compatibility(spec).compatible
}

/// Maps a model to its payload.
fn model_dto(
    state: &ApiState,
    spec: &TTSModelSpec,
    runtimes: &[tts_core::runtime::TTSRuntime],
) -> ModelDto {
    let report = state.service.manager.compatibility(spec);
    let runtime = runtimes
        .iter()
        .find(|runtime| runtime.model_id == spec.id);
    ModelDto {
        id: spec.id.clone(),
        name: spec.name.clone(),
        family: spec.family.as_str().to_string(),
        engine_id: spec.engine_id.clone(),
        hf_repo: spec.hf_repo.clone(),
        version: spec.version.clone(),
        format: spec.format.clone(),
        size_bytes: spec.size_bytes,
        estimated_memory_bytes: spec.estimated_memory_bytes,
        features: spec
            .features
            .iter()
            .map(|feature| feature.as_str().to_string())
            .collect(),
        compatible: report.compatible,
        compatibility: report.level.as_str().to_string(),
        notes: report.notes,
        installed: state.service.registry.is_installed(&spec.id),
        path: state.service.registry.install_path(&spec.id),
        state: runtime
            .map(|runtime| runtime.state.as_str().to_string())
            .unwrap_or_else(|| "unloaded".to_string()),
        pinned: runtime.map(|runtime| runtime.pinned).unwrap_or(false),
    }
}

/// `GET /v1/tts/status`
async fn status(State(state): State<ApiState>) -> Result<impl IntoResponse, ApiError> {
    Ok(axum::Json(state.service.manager.status()))
}

/// `POST /v1/audio/speech`
///
/// OpenAI compatible speech synthesis.
async fn speech(
    State(state): State<ApiState>,
    body: axum::Json<SpeechRequest>,
) -> Result<Response, ApiError> {
    let request = body.0;
    let spec = resolve_model(&state.service, &request.model)?;
    let format = request.format();

    let mut tts_request = TTSRequest {
        model_id: spec.id.clone(),
        voice_id: None,
        text: request.input.clone(),
        language: request.language.clone(),
        parameters: parameters_for(&spec, &request),
        response_format: format,
        speed: request.speed,
    };

    // The `voice` field carries a voice profile id. When it is unknown the
    // value is treated as a built-in speaker name.
    if let Some(voice) = &request.voice {
        if state.service.library.get(voice).await?.is_some() {
            tts_request.voice_id = Some(voice.clone());
        } else if let EngineSpecificParameters::Qwen3Tts(params) = &mut tts_request.parameters {
            params.speaker = Some(voice.clone());
        }
    }

    let result = state.service.manager.synthesize(tts_request).await?;
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, result.output.mime_type().to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!(
                    "inline; filename=\"speech.{}\"",
                    result.output.format.extension()
                ),
            ),
        ],
        Body::from(result.output.data),
    )
        .into_response())
}

/// `POST /v1/tts/speak`
///
/// Extended synthesis endpoint that accepts the engine specific parameters.
async fn speak(
    State(state): State<ApiState>,
    body: axum::Json<SpeakRequest>,
) -> Result<Response, ApiError> {
    let request = body.0;
    let format = request
        .response_format
        .as_deref()
        .and_then(tts_core::types::AudioFormat::parse)
        .unwrap_or(tts_core::types::AudioFormat::Wav);

    let spec = resolve_model(&state.service, &request.model)?;

    let parameters = match request.parameters.clone() {
        Some(value) => {
            serde_json::from_value(value).unwrap_or_else(|_| default_parameters(&spec))
        }
        None => default_parameters(&spec),
    };

    let tts_request = TTSRequest {
        model_id: spec.id.clone(),
        voice_id: request.voice_id.clone(),
        text: request.input.clone(),
        language: request.language.clone(),
        parameters,
        response_format: format,
        speed: None,
    };
    let result = state.service.manager.synthesize(tts_request).await?;
    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, result.output.mime_type().to_string())],
        Body::from(result.output.data),
    )
        .into_response())
}

/// `GET /v1/tts/voices`
async fn list_voices(
    State(state): State<ApiState>,
    Query(query): Query<ListVoicesQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let filter = VoiceFilter {
        model_id: query.model,
        engine: query.engine,
        language: query.language,
        query: query.query,
    };
    let voices = state.service.library.list(filter).await?;
    let voices: Vec<VoiceDto> = voices.into_iter().map(VoiceDto::from).collect();
    Ok(axum::Json(voices))
}

/// `POST /v1/tts/voices`
async fn create_voice(
    State(state): State<ApiState>,
    body: axum::Json<CreateVoiceRequest>,
) -> Result<Response, ApiError> {
    let request = body.0;
    let profile = state
        .service
        .library
        .create(NewVoiceProfile {
            name: request.name,
            description: request.description,
            language: request.language,
            gender_presentation: request.gender_presentation,
            age_style: request.age_style,
            personality: request.personality,
            engine: request.engine,
            model_id: request.model_id,
            engine_config: request
                .engine_config
                .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new())),
        })
        .await?;
    Ok((StatusCode::CREATED, axum::Json(VoiceDto::from(profile))).into_response())
}

/// `GET /v1/tts/voices/{id}`
async fn get_voice(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let voice = state
        .service
        .library
        .get(&id)
        .await?
        .ok_or_else(|| TtsError::VoiceNotFound(id.clone()))?;
    Ok(axum::Json(VoiceDto::from(voice)))
}

/// `PATCH /v1/tts/voices/{id}`
async fn update_voice(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    body: axum::Json<UpdateVoiceRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let request = body.0;
    let voice = state
        .service
        .library
        .update(
            &id,
            VoicePatch {
                name: request.name,
                description: request.description,
                language: request.language,
                gender_presentation: request.gender_presentation,
                age_style: request.age_style,
                personality: request.personality,
                engine_config: request.engine_config,
            },
        )
        .await?;
    Ok(axum::Json(VoiceDto::from(voice)))
}

/// `DELETE /v1/tts/voices/{id}`
async fn delete_voice(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    state.service.library.delete(&id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `POST /v1/tts/voices/design`
///
/// Creates a voice from a natural language description and renders a preview.
async fn design_voice(
    State(state): State<ApiState>,
    body: axum::Json<VoiceDesignRequest>,
) -> Result<Response, ApiError> {
    let request = body.0;
    if request.prompt.trim().is_empty() {
        return Err(ApiError(TtsError::InvalidRequest(
            "the voice description must not be empty".to_string(),
        )));
    }
    let spec = resolve_model(&state.service, &request.model)?;
    if !spec.supports(TtsFeature::VoiceDesign) {
        return Err(ApiError(TtsError::UnsupportedFeature {
            model: spec.id.clone(),
            feature: TtsFeature::VoiceDesign.as_str().to_string(),
        }));
    }

    let mut engine_config = serde_json::Value::Object(serde_json::Map::new());
    if let Some(base_id) = &request.base_voice_id {
        let base = state
            .service
            .library
            .get(base_id)
            .await?
            .ok_or_else(|| TtsError::VoiceNotFound(base_id.clone()))?;
        engine_config = base.engine_config;
    }
    if let Some(map) = engine_config.as_object_mut() {
        map.insert(
            "voice_design_prompt".to_string(),
            serde_json::Value::String(request.prompt.clone()),
        );
    }

    let voice = state
        .service
        .library
        .create(NewVoiceProfile {
            name: request.name.clone(),
            description: Some(request.prompt.clone()),
            language: request.language.clone(),
            gender_presentation: request.gender_presentation.clone(),
            age_style: request.age_style.clone(),
            personality: request.personality.clone(),
            engine: Some(spec.family.as_str().to_string()),
            model_id: Some(spec.id.clone()),
            engine_config: engine_config.clone(),
        })
        .await?;

    let (sample, sample_path) =
        render_preview(&state, &spec, &voice, request.sample_text.as_deref()).await?;
    Ok((
        StatusCode::CREATED,
        axum::Json(VoiceCreationResult {
            voice: VoiceDto::from(voice),
            sample,
            sample_path,
        }),
    )
        .into_response())
}

/// `POST /v1/tts/voices/clone`
///
/// Creates a voice from a reference clip and renders a preview.
async fn clone_voice(
    State(state): State<ApiState>,
    body: axum::Json<VoiceCloneRequest>,
) -> Result<Response, ApiError> {
    let request = body.0;
    let Some(reference) = request.reference_audio.clone() else {
        return Err(ApiError(TtsError::InvalidRequest(
            "a reference clip is required".to_string(),
        )));
    };
    let spec = resolve_model(&state.service, &request.model)?;
    if !spec.supports(TtsFeature::VoiceCloning) {
        return Err(ApiError(TtsError::UnsupportedFeature {
            model: spec.id.clone(),
            feature: TtsFeature::VoiceCloning.as_str().to_string(),
        }));
    }

    let mut engine_config = serde_json::Map::new();
    engine_config.insert(
        "reference_audio".to_string(),
        serde_json::Value::String(reference),
    );
    if let Some(text) = &request.reference_text {
        engine_config.insert(
            "reference_text".to_string(),
            serde_json::Value::String(text.clone()),
        );
    }

    let voice = state
        .service
        .library
        .create(NewVoiceProfile {
            name: request.name.clone(),
            description: request.description.clone(),
            language: request.language.clone(),
            engine: Some(spec.family.as_str().to_string()),
            model_id: Some(spec.id.clone()),
            engine_config: serde_json::Value::Object(engine_config),
            ..Default::default()
        })
        .await?;

    let (sample, sample_path) =
        render_preview(&state, &spec, &voice, request.sample_text.as_deref()).await?;
    Ok((
        StatusCode::CREATED,
        axum::Json(VoiceCreationResult {
            voice: VoiceDto::from(voice),
            sample,
            sample_path,
        }),
    )
        .into_response())
}

/// `GET /v1/tts/voices/{id}/samples`
async fn list_samples(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let samples = state.service.library.samples(&id).await?;
    Ok(axum::Json(samples))
}

/// `GET /v1/tts/voices/{id}/samples/{name}`
async fn get_sample(
    State(state): State<ApiState>,
    Path((id, name)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let path = state.service.library.sample_path(&id, &name).await?;
    let bytes = std::fs::read(&path).map_err(|e| TtsError::io(&path, e))?;
    let mime = mime_of(&path);
    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, mime)],
        Body::from(bytes),
    )
        .into_response())
}

/// `POST /v1/tts/models/{id}/load`
async fn load_model(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let runtime = state.service.manager.load_model(&id).await?;
    Ok(axum::Json(runtime))
}

/// `POST /v1/tts/models/{id}/unload`
async fn unload_model(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    state.service.manager.unload_model(&id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `POST /v1/tts/models/{id}/pin`
async fn pin_model(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    state.service.manager.set_pinned(&id, true)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `POST /v1/tts/models/{id}/unpin`
async fn unpin_model(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    state.service.manager.set_pinned(&id, false)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `POST /v1/tts/models/{id}/install`
///
/// Starts the download of the checkpoint in the background and answers with
/// `202 Accepted`. The progress is polled through the same path with `GET`.
async fn install_model(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let spec = resolve_model(&state.service, &id)?;

    // The mock engine is part of the application, there is nothing to
    // download for it.
    if spec.is_builtin() {
        return Err(ApiError(TtsError::InvalidRequest(format!(
            "`{}` 内置于应用，无需下载",
            spec.id
        ))));
    }

    // The specification demands the compatibility check before the model is
    // installed.
    let report = state.service.manager.compatibility(&spec);
    if !report.compatible {
        return Err(ApiError::from(TtsError::IncompatibleHardware {
            model: spec.id.clone(),
            reason: report.notes.join("; "),
        }));
    }

    if state.service.installs.get(&spec.id).is_some()
        && state
            .service
            .installs
            .get(&spec.id)
            .map(|progress| progress.state == "running")
            .unwrap_or(false)
    {
        return Ok((StatusCode::ACCEPTED, "the installation is already running").into_response());
    }

    let service = state.service.clone();
    state.service.installs.start(&spec.id);
    tokio::spawn(async move {
        let result = service
            .installer
            .install(&spec, Some(&service.installs))
            .await;
        service
            .installs
            .finish(&spec.id, result.as_ref().err().map(|err| err.to_string()));
    });

    Ok((StatusCode::ACCEPTED, "the installation has started").into_response())
}

/// `GET /v1/tts/models/{id}/install`
async fn install_progress(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let spec = resolve_model(&state.service, &id)?;
    let progress = state.service.installs.get(&spec.id).unwrap_or_else(|| {
        tts_core::installer::InstallProgress {
            model_id: spec.id.clone(),
            state: "idle".to_string(),
            files_done: 0,
            files_total: 0,
            bytes: 0,
            bytes_total: 0,
            error: None,
        }
    });
    Ok(axum::Json(progress).into_response())
}

/// `POST /v1/tts/models/{id}/uninstall`
async fn uninstall_model(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let spec = resolve_model(&state.service, &id)?;
    if spec.is_builtin() {
        return Err(ApiError(TtsError::InvalidRequest(format!(
            "`{}` 内置于应用，无法删除",
            spec.id
        ))));
    }
    // A checkpoint that is in use must not disappear under the running
    // engine.
    if state
        .service
        .manager
        .status()
        .runtimes
        .iter()
        .any(|runtime| runtime.model_id == spec.id && runtime.state == RuntimeState::Running)
    {
        return Err(ApiError(TtsError::InvalidRequest(format!(
            "`{}` 正在运行，请先卸载后再删除",
            spec.id
        ))));
    }
    state.service.installer.uninstall(&spec)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `GET /v1/tts/models/{id}/logs`
async fn model_logs(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let logs = state.service.manager.logs(&id).await?;
    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        Body::from(logs),
    )
        .into_response())
}

/// `GET /v1/audio/{file}`
async fn get_audio(
    State(state): State<ApiState>,
    Path(file): Path<String>,
) -> Result<Response, ApiError> {
    if file.contains('/') || file.contains("..") {
        return Err(ApiError(TtsError::InvalidRequest(
            "invalid file name".to_string(),
        )));
    }
    let path = state.service.audio.generated_root().join(&file);
    if !path.exists() {
        return Err(ApiError(TtsError::InvalidRequest(format!(
            "audio file `{file}` does not exist"
        ))));
    }
    let bytes = std::fs::read(&path).map_err(|e| TtsError::io(&path, e))?;
    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, mime_of(&path))],
        Body::from(bytes),
    )
        .into_response())
}

/// Renders a preview sample for a freshly created voice.
async fn render_preview(
    state: &ApiState,
    spec: &TTSModelSpec,
    voice: &tts_core::voice::VoiceProfile,
    sample_text: Option<&str>,
) -> Result<(Option<String>, Option<String>), ApiError> {
    let sample_text = sample_text.unwrap_or("你好，这是这个声音的试听样本。");
    let mut request = TTSRequest {
        model_id: spec.id.clone(),
        voice_id: Some(voice.id.clone()),
        text: sample_text.to_string(),
        language: voice.language.clone(),
        parameters: default_parameters(spec),
        response_format: tts_core::types::AudioFormat::Wav,
        speed: None,
    };
    if let EngineSpecificParameters::Qwen3Tts(params) = &mut request.parameters {
        params.task_type = Some(tts_core::types::Qwen3TtsTaskType::VoiceDesign);
        if params.instructions.is_none() {
            params.instructions = voice
                .engine_config
                .get("voice_design_prompt")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
        }
    }
    let result = state.service.manager.synthesize(request).await?;
    let sample = state
        .service
        .library
        .add_sample(&voice.id, &result.output, None)
        .await?;
    Ok((Some(sample.name), sample.path.into()))
}

/// Resolves a model id or an alias.
fn resolve_model(service: &TtsService, model: &str) -> Result<TTSModelSpec, ApiError> {
    service
        .registry
        .resolve(model)
        .ok_or_else(|| ApiError(TtsError::ModelNotFound(model.to_string())))
}

/// Builds the engine specific parameters of a request.
///
/// The fields of the request are translated into the parameters of the
/// adapter that serves the model.
fn parameters_for(
    spec: &TTSModelSpec,
    request: &SpeechRequest,
) -> EngineSpecificParameters {
    let references: Vec<AudioReference> = request
        .references
        .as_ref()
        .map(|list| {
            list.iter()
                .map(|reference| AudioReference {
                    audio_path: reference.audio_path.clone(),
                    data: reference.data.clone(),
                    text: reference.text.clone(),
                    speaker_embedding: None,
                })
                .collect()
        })
        .unwrap_or_default();

    match spec.family {
        tts_core::types::ModelFamily::Qwen3Tts => {
            let task_type = match request.task_type.as_deref() {
                Some("VoiceDesign") | Some("voice_design") | Some("design") => {
                    Some(Qwen3TtsTaskType::VoiceDesign)
                }
                Some("CustomVoice") | Some("custom_voice") => {
                    Some(Qwen3TtsTaskType::CustomVoice)
                }
                _ => None,
            };
            EngineSpecificParameters::Qwen3Tts(Qwen3TtsParameters {
                task_type,
                instructions: request.instructions.clone(),
                speaker: None,
                references,
                seed: request.seed,
                x_vector_only_mode: request.x_vector_only_mode,
            })
        }
        tts_core::types::ModelFamily::FishDrama3 => {
            EngineSpecificParameters::FishDrama3(FishDrama3Parameters {
                references,
                previous_takes: request.previous_takes.clone().unwrap_or_default(),
                seed: request.seed,
            })
        }
        tts_core::types::ModelFamily::Mock => {
            EngineSpecificParameters::Mock(MockParameters::default())
        }
    }
}

fn default_parameters(spec: &TTSModelSpec) -> EngineSpecificParameters {
    match spec.family {
        tts_core::types::ModelFamily::Qwen3Tts => {
            EngineSpecificParameters::Qwen3Tts(Qwen3TtsParameters::default())
        }
        tts_core::types::ModelFamily::FishDrama3 => {
            EngineSpecificParameters::FishDrama3(FishDrama3Parameters::default())
        }
        tts_core::types::ModelFamily::Mock => {
            EngineSpecificParameters::Mock(MockParameters::default())
        }
    }
}

/// MIME type of an audio file.
fn mime_of(path: &std::path::Path) -> String {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("wav");
    tts_core::types::AudioFormat::parse(extension)
        .map(|format| format.mime_type().to_string())
        .unwrap_or_else(|| "application/octet-stream".to_string())
}

impl From<tts_core::voice::VoiceProfile> for VoiceDto {
    fn from(value: tts_core::voice::VoiceProfile) -> Self {
        VoiceDto {
            id: value.id,
            name: value.name,
            description: value.description,
            language: value.language,
            gender_presentation: value.gender_presentation,
            age_style: value.age_style,
            personality: value.personality,
            engine: value.engine,
            model_id: value.model_id,
            engine_config: value.engine_config,
            samples: value.samples,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}
