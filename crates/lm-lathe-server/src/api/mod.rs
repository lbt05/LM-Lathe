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
use axum::extract::{Json, Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use serde::Deserialize;
use tts_core::error::TtsError;
use tts_core::hub::HubClient;
use tts_core::registry::TTSModelSpec;
use tts_core::service::TtsService;
use tts_core::types::{
    AudioReference, EngineSpecificParameters, FishDrama3Parameters, ModelFamily, Qwen3TtsParameters,
    Qwen3TtsTaskType, RuntimeState, TTSRequest, TtsFeature,
};
use tts_core::voice::{NewVoiceProfile, VoiceFilter, VoicePatch};

use dto::*;

/// The query of a hub search.
#[derive(Debug, Clone, Deserialize)]
pub struct DiscoverParams {
    /// The text that is searched for.
    #[serde(default)]
    pub q: String,
    /// How many models are returned.
    #[serde(default)]
    pub limit: Option<usize>,
}

/// The body of the installation of a discovered model.
#[derive(Debug, Clone, Deserialize)]
pub struct DiscoverInstallRequest {
    /// The `organisation/repository` id of the hub.
    pub repo: String,
}

/// The body of the setting of the download concurrency.
#[derive(Debug, Clone, Deserialize)]
pub struct DownloadConcurrencyRequest {
    /// How many files of a repository are fetched at the same time.
    #[serde(default = "one")]
    pub concurrency: u8,
}

/// The number of the files that are fetched at the same time.
fn one() -> u8 {
    1
}

/// Shared state of the API.
#[derive(Clone)]
pub struct ApiState {
    /// The TTS subsystem.
    pub service: Arc<TtsService>,
    /// The client of the Hugging Face Hub.
    pub hub: Arc<HubClient>,
    /// The address the service listens on.
    pub bind: String,
}

/// One route of the local gateway.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RouteInfo {
    /// The HTTP method.
    pub method: &'static str,
    /// The path of the route.
    pub path: &'static str,
    /// What the route does.
    pub summary: &'static str,
}

/// The routes of the local gateway.
///
/// The desktop application shows this list on the 服务 page, so it has to
/// describe the real gateway and not a hand-written copy of it.
pub const ROUTES: &[RouteInfo] = &[
    RouteInfo { method: "GET", path: "/health", summary: "服务是否在运行，以及机器还剩多少内存" },
    RouteInfo { method: "GET", path: "/v1/routes", summary: "网关路由表" },
    RouteInfo { method: "GET", path: "/v1/paths", summary: "应用使用的目录与下载设置" },
    RouteInfo { method: "POST", path: "/v1/settings/download-concurrency", summary: "设置同时下载的文件数" },
    RouteInfo { method: "GET", path: "/v1/models", summary: "可以服务的模型（已下载且机器能跑）" },
    RouteInfo { method: "GET", path: "/v1/tts/models", summary: "完整模型目录，含未下载的模型" },
    RouteInfo { method: "GET", path: "/v1/tts/status", summary: "运行时状态与调度器快照" },
    RouteInfo { method: "POST", path: "/v1/audio/speech", summary: "OpenAI 兼容的语音合成" },
    RouteInfo { method: "POST", path: "/v1/chat/completions", summary: "OpenAI 兼容的对话补全" },
    RouteInfo { method: "POST", path: "/v1/completions", summary: "OpenAI 兼容的文本补全" },
    RouteInfo { method: "POST", path: "/v1/tts/speak", summary: "语音合成，可带引擎参数" },
    RouteInfo { method: "GET", path: "/v1/tts/voices", summary: "声音库列表" },
    RouteInfo { method: "POST", path: "/v1/tts/voices", summary: "新建声音" },
    RouteInfo { method: "POST", path: "/v1/tts/voices/design", summary: "声音设计" },
    RouteInfo { method: "POST", path: "/v1/tts/voices/clone", summary: "声音克隆" },
    RouteInfo { method: "GET", path: "/v1/tts/voices/{id}", summary: "声音详情" },
    RouteInfo { method: "PATCH", path: "/v1/tts/voices/{id}", summary: "修改声音" },
    RouteInfo { method: "DELETE", path: "/v1/tts/voices/{id}", summary: "删除声音" },
    RouteInfo { method: "GET", path: "/v1/tts/voices/{id}/samples", summary: "声音样例列表" },
    RouteInfo { method: "GET", path: "/v1/tts/voices/{id}/samples/{*name}", summary: "声音样例文件" },
    RouteInfo { method: "POST", path: "/v1/tts/models/{id}/load", summary: "加载模型" },
    RouteInfo { method: "POST", path: "/v1/tts/models/{id}/unload", summary: "卸载模型" },
    RouteInfo { method: "POST", path: "/v1/tts/models/{id}/pin", summary: "固定模型，常驻显存" },
    RouteInfo { method: "POST", path: "/v1/tts/models/{id}/unpin", summary: "取消固定" },
    RouteInfo { method: "POST", path: "/v1/tts/models/{id}/install", summary: "下载模型" },
    RouteInfo { method: "GET", path: "/v1/tts/models/{id}/install", summary: "下载进度" },
    RouteInfo { method: "POST", path: "/v1/tts/models/{id}/install/cancel", summary: "取消下载" },
    RouteInfo { method: "POST", path: "/v1/tts/models/{id}/install/discard", summary: "丢弃中止的下载" },
    RouteInfo { method: "GET", path: "/v1/tts/installs", summary: "所有下载的进度" },
    RouteInfo { method: "POST", path: "/v1/tts/models/{id}/uninstall", summary: "删除本地模型" },
    RouteInfo { method: "GET", path: "/v1/tts/models/{id}/logs", summary: "引擎日志" },
    RouteInfo { method: "GET", path: "/v1/tts/discover", summary: "在 Hugging Face 上搜索模型" },
    RouteInfo { method: "POST", path: "/v1/tts/discover/install", summary: "下载搜索到的模型" },
    RouteInfo { method: "GET", path: "/v1/runtimes", summary: "推理运行时列表" },
    RouteInfo { method: "GET", path: "/v1/runtimes/{id}", summary: "单个推理运行时" },
    RouteInfo { method: "GET", path: "/v1/runtimes/{id}/versions", summary: "可安装的版本" },
    RouteInfo { method: "POST", path: "/v1/runtimes/{id}/install", summary: "安装引擎" },
    RouteInfo { method: "GET", path: "/v1/runtimes/{id}/install-progress", summary: "安装进度" },
    RouteInfo { method: "POST", path: "/v1/runtimes/{id}/activate", summary: "切换当前版本" },
    RouteInfo { method: "POST", path: "/v1/runtimes/{id}/uninstall", summary: "卸载引擎版本" },
    RouteInfo { method: "POST", path: "/v1/runtimes/{id}/health", summary: "引擎健康检查" },
    RouteInfo { method: "GET", path: "/v1/audio/{file}", summary: "生成的音频文件" },
];

/// `GET /v1/routes`
async fn list_routes() -> impl IntoResponse {
    axum::Json(ROUTES)
}

/// Builds the router.
pub fn router(service: Arc<TtsService>, bind: &str) -> Router {
    let state = ApiState {
        service,
        hub: Arc::new(HubClient::new()),
        bind: bind.to_string(),
    };
    Router::new()
        .route("/health", get(health))
        .route("/v1/routes", get(list_routes))
        .route("/v1/paths", get(paths))
        .route("/v1/settings/download-concurrency", post(set_download_concurrency))
        .route("/v1/models", get(list_models))
        .route("/v1/tts/models", get(list_catalogue))
        .route("/v1/tts/status", get(status))
        .route("/v1/audio/speech", post(speech))
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/completions", post(completions))
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
        .route("/v1/tts/models/{id}/install/cancel", post(cancel_install))
        .route("/v1/tts/models/{id}/install/discard", post(discard_install))
        .route("/v1/tts/installs", get(list_installs))
        .route("/v1/tts/models/{id}/uninstall", post(uninstall_model))
        .route("/v1/tts/models/{id}/logs", get(model_logs))
        .route("/v1/tts/discover", get(discover_models))
        .route("/v1/tts/discover/install", post(install_discovered))
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

#[cfg(test)]
mod tests {
    use super::ROUTES;

    /// The route table of the gateway must not drift away from the router.
    ///
    /// Every registered route has to appear in the table, and the table must
    /// not describe a route that does not exist.
    #[test]
    fn the_route_table_matches_the_router() {
        let source = include_str!("mod.rs");
        let mut registered: Vec<String> = Vec::new();
        // The needle is assembled from two pieces, so that the test itself
        // does not contain the pattern it looks for.
        let needle = [".ro", "ute("].concat();
        let mut rest = source;
        while let Some(index) = rest.find(&needle) {
            rest = &rest[index + needle.len()..];
            let trimmed = rest.trim_start();
            if !trimmed.starts_with('"') {
                continue;
            }
            let body = &trimmed[1..];
            if let Some(end) = body.find('"') {
                registered.push(body[..end].to_string());
                rest = &body[end..];
            }
        }

        let mut described: Vec<String> = ROUTES
            .iter()
            .map(|route| route.path.to_string())
            .collect();
        described.sort();
        described.dedup();
        registered.sort();
        registered.dedup();

        assert_eq!(
            described, registered,
            "the route table of the gateway does not match the router"
        );
    }
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
///
/// The answer tells whether the service runs and how much memory the machine
/// has left. It does not ask the engine: a model that is loaded has its own
/// endpoint under `/v1/tts/status`.
async fn health(State(state): State<ApiState>) -> impl IntoResponse {
    let status = state.service.manager.status();
    // The service is only healthy when it can reach its own directory.
    let healthy = state.service.paths.root().is_dir();
    axum::Json(serde_json::json!({
        "status": if healthy { "ok" } else { "degraded" },
        "active_model": status.active_model,
        "available_bytes": status.scheduler.available_bytes,
        "free_memory_bytes": status.hardware.available_ram_bytes,
        "build": build_fingerprint(),
        "bind": state.bind,
    }))
}

/// `GET /v1/paths`
///
/// The directories of the application and the settings that belong to them.
async fn paths(State(state): State<ApiState>) -> impl IntoResponse {
    let paths = &state.service.paths;
    let status = state.service.manager.status();
    axum::Json(dto::PathsDto {
        root: paths.root().display().to_string(),
        tts_models_root: paths.tts_models_root().display().to_string(),
        llm_models_root: paths.llm_models_root().display().to_string(),
        runtimes_root: paths.runtimes_root().display().to_string(),
        engines_root: paths.engines_root().display().to_string(),
        voices_root: paths.voices_root().display().to_string(),
        logs_root: paths.logs_root().display().to_string(),
        download_concurrency: state.service.installer.concurrency(),
        available_bytes: status.scheduler.available_bytes,
    })
}

/// `POST /v1/settings/download-concurrency`
///
/// Sets how many files of a repository are fetched at the same time.
async fn set_download_concurrency(
    State(state): State<ApiState>,
    body: Option<Json<DownloadConcurrencyRequest>>,
) -> Result<impl IntoResponse, ApiError> {
    let request = body.map(|Json(body)| body).unwrap_or(DownloadConcurrencyRequest {
        concurrency: 1,
    });
    let value = state.service.installer.set_concurrency(request.concurrency as usize);
    Ok(axum::Json(serde_json::json!({ "concurrency": value })))
}

/// A fingerprint of the running binary.
///
/// The desktop application compares it with the fingerprint of the binary it
/// would start. If both are the same, the server on the port is the build the
/// application expects and it can be reused.
pub fn build_fingerprint() -> String {
    let Ok(exe) = std::env::current_exe() else {
        return "unknown".to_string();
    };
    let Ok(meta) = std::fs::metadata(&exe) else {
        return "unknown".to_string();
    };
    let modified = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|since| since.as_secs())
        .unwrap_or_default();
    format!("{}:{}", exe.display(), modified)
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

/// `GET /v1/tts/discover`
///
/// Searches the Hugging Face Hub for models the runtime can serve. The
/// registry only knows the models that ship with the application, so this
/// is where the rest of the hub comes from.
/// The warnings of a model of the hub.
///
/// The model is described by the files of its repository, so the same rules
/// as for the built-in catalogue apply.
fn discovered_warnings(
    model: &tts_core::hub::HubModel,
    hardware: &tts_core::hardware::HardwareProfile,
) -> Vec<String> {
    let family = match model.family() {
        Some(family) => family,
        None => return Vec::new(),
    };
    let engine_id = model.engine_id();
    let mut warnings = Vec::new();

    let needed = family.memory_requirement().max(
        model
            .size_bytes
            .map(|size| size + size / 4)
            .unwrap_or_default(),
    );
    if needed > 0 && hardware.total_ram_bytes < needed {
        warnings.push(format!(
            "这个检查点需要约 {:.1} GiB 内存，这台机器只有 {:.1} GiB。换一个更小的量化版本，或者换一台机器。",
            needed as f64 / 1024.0 / 1024.0 / 1024.0,
            hardware.total_ram_bytes as f64 / 1024.0 / 1024.0 / 1024.0
        ));
    }

    if hardware.apple_silicon
        && engine_id == "sglang-omni"
        && family == ModelFamily::Qwen3Tts
        && model.parameters.unwrap_or_default() > 1_000_000_000
    {
        warnings.push(
            "在 Apple Silicon 上，SGLang-Omni 的 MPS 路径已经停用（Torch 加载这个检查点时会崩溃），请改用同系列的 MLX 转换版。"
                .to_string(),
        );
    }

    warnings
}

async fn discover_models(
    State(state): State<ApiState>,
    Query(params): Query<DiscoverParams>,
) -> Result<impl IntoResponse, ApiError> {
    let search = state
        .hub
        .search(&params.q, params.limit.unwrap_or(20))
        .await?;
    let known: Vec<TTSModelSpec> = state.service.registry.list();
    let payload: Vec<DiscoveredModelDto> = search
        .models
        .iter()
        .map(|model| {
            // A repository that the application already knows keeps the id
            // of that model, so the download is reported under it.
            let known_spec = known.iter().find(|spec| spec.hf_repo == model.id);
            DiscoveredModelDto {
                id: known_spec
                    .map(|spec| spec.id.clone())
                    .unwrap_or_else(|| model.id.replace('/', "--")),
                repo: model.id.clone(),
                name: model.id.clone(),
                family: model
                    .family()
                    .map(|family| family.as_str().to_string())
                    .unwrap_or_default(),
                format: model.format().to_string(),
                engine_id: model.engine_id().to_string(),
                downloads: model.downloads,
                likes: model.likes,
                last_modified: model.last_modified.clone(),
                license: model.license(),
                quant: model.quant(),
                files: model.gguf_files(),
                installed: known_spec
                    .map(|spec| spec.is_installed())
                    .unwrap_or(false),
                size_bytes: model.size_bytes,
                parameters: model.parameters,
                warnings: discovered_warnings(model, &state.service.hardware),
            }
        })
        .collect();
    Ok(axum::Json(DiscoveredModelsDto {
        models: payload,
        dropped: search.dropped,
    }))
}

/// `POST /v1/tts/models/install`
///
/// Downloads a model of the Hugging Face Hub that was found by the search.
/// The model is described by its repository, the specification is built
/// from the hub and the model joins the catalogue, so the progress of the
/// download is reported like the progress of every other model.
async fn install_discovered(
    State(state): State<ApiState>,
    Json(body): Json<DiscoverInstallRequest>,
) -> Result<Response, ApiError> {
    let repo = body.repo.trim();
    if repo.is_empty() {
        return Err(ApiError(TtsError::InvalidRequest(
            "缺少模型仓库".to_string(),
        )));
    }

    // A repository that is already known keeps its own specification, so
    // the checkpoint is not written twice.
    if let Some(spec) = state
        .service
        .registry
        .list()
        .into_iter()
        .find(|spec| spec.hf_repo == repo)
    {
        return start_install(&state, spec);
    }

    let model = state.hub.model(repo).await?;
    if !model.is_runnable() {
        return Err(ApiError(TtsError::InvalidRequest(format!(
            "`{repo}` 不是本应用能运行的模型"
        ))));
    }
    let files = state.hub.files(repo).await?;
    let spec = model.to_spec(&files)?;
    state.service.registry.insert(spec.clone());
    start_install(&state, spec)
}

/// Starts the download of a model and answers with its id.
fn start_install(state: &ApiState, spec: TTSModelSpec) -> Result<Response, ApiError> {
    if spec.is_builtin() {
        return Err(ApiError(TtsError::InvalidRequest(format!(
            "`{}` 内置于应用，无需下载",
            spec.id
        ))));
    }
    if state
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
    let model_id = spec.id.clone();
    let name = spec.name.clone();
    tokio::spawn(async move {
        let result = service
            .installer
            .install(&spec, Some(&service.installs))
            .await;
        finish_install(&service, &spec.id, result);
    });
    Ok((
        StatusCode::ACCEPTED,
        axum::Json(serde_json::json!({ "model_id": model_id, "name": name })),
    )
        .into_response())
}

/// Puts the end of an installation into the log: a download that the user
/// stopped is not a failure, the files that are already there stay and the
/// next attempt continues them.
fn finish_install(
    service: &TtsService,
    model_id: &str,
    result: Result<tts_core::installer::InstallReport, TtsError>,
) {
    match result {
        Ok(_) => service.installs.finish(model_id, None),
        Err(err) => {
            if matches!(err, TtsError::Cancelled(_)) {
                service.installs.cancel(model_id);
                tracing::info!(model = %model_id, "the download was cancelled");
            } else {
                service.installs.finish(model_id, Some(err.to_string()));
            }
        }
    }
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
/// The warnings of a model for the reader of the interface.
///
/// They name the things that are known to fail before they happen: a
/// checkpoint that does not fit into the memory of the machine, or an
/// engine that crashes on this platform.
fn warnings_for(
    spec: &TTSModelSpec,
    hardware: &tts_core::hardware::HardwareProfile,
    report: &tts_core::registry::CompatibilityReport,
) -> Vec<String> {
    let mut warnings = Vec::new();

    // The checkpoint does not fit into the memory of this machine.
    let needed = if spec.hardware.min_total_ram_bytes > 0 {
        spec.hardware.min_total_ram_bytes
    } else {
        spec.estimated_memory_bytes
    };
    if needed > 0 && hardware.total_ram_bytes < needed {
        warnings.push(format!(
            "这个检查点需要约 {:.1} GiB 内存，这台机器只有 {:.1} GiB。换一个更小的量化版本，或者换一台机器。",
            needed as f64 / 1024.0 / 1024.0 / 1024.0,
            hardware.total_ram_bytes as f64 / 1024.0 / 1024.0 / 1024.0
        ));
    } else if !report.compatible {
        // Something else is missing: the note of the report explains it.
        warnings.push(format!(
            "这台机器不能运行这个模型：{}。",
            report.notes.join("；")
        ));
    }

    // The Torch path of SGLang-Omni crashes on Apple Silicon while it loads
    // the bigger Qwen3-TTS checkpoints.
    if hardware.apple_silicon
        && spec.engine_id == "sglang-omni"
        && spec.family == ModelFamily::Qwen3Tts
        && spec.parameters.unwrap_or_default() > 1_000_000_000
    {
        warnings.push(
            "在 Apple Silicon 上，SGLang-Omni 的 MPS 路径已经停用（Torch 加载这个检查点时会崩溃），请改用同系列的 MLX 转换版。"
                .to_string(),
        );
    }

    warnings
}

fn model_dto(
    state: &ApiState,
    spec: &TTSModelSpec,
    runtimes: &[tts_core::runtime::TTSRuntime],
) -> ModelDto {
    let report = state.service.manager.compatibility(spec);
    let runtime = runtimes
        .iter()
        .find(|runtime| runtime.model_id == spec.id);
    let llm_runtime = state
        .service
        .llm
        .runtime(&spec.id)
        .filter(|_| spec.family == ModelFamily::Llm);
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
        parameters: spec.parameters,
        features: spec
            .features
            .iter()
            .map(|feature| feature.as_str().to_string())
            .collect(),
        llm_features: spec
            .llm_features
            .iter()
            .map(|feature| feature.as_str().to_string())
            .collect(),
        compatible: report.compatible,
        compatibility: report.level.as_str().to_string(),
        notes: report.notes.clone(),
        warnings: warnings_for(spec, &state.service.hardware, &report),
        installed: state.service.registry.is_installed(&spec.id),
        path: state.service.registry.install_path(&spec.id),
        state: runtime
            .map(|runtime| runtime.state.as_str().to_string())
            .or_else(|| {
                llm_runtime
                    .as_ref()
                    .map(|runtime| runtime.state.as_str().to_string())
            })
            .unwrap_or_else(|| "unloaded".to_string()),
        pinned: runtime
            .map(|runtime| runtime.pinned)
            .or_else(|| llm_runtime.as_ref().map(|runtime| runtime.pinned))
            .unwrap_or(false),
    }
}

/// `GET /v1/tts/status`
async fn status(State(state): State<ApiState>) -> Result<impl IntoResponse, ApiError> {
    let mut status = state.service.manager.status();
    status.llm_runtimes = state.service.llm.runtimes();
    Ok(axum::Json(status))
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
///
/// Starts the model. The language models run in their own runtime, the
/// speech models in the speech runtime.
async fn load_model(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    if is_language_model(&state, &id) {
        let runtime = state.service.llm.load_model(&id).await?;
        let payload = serde_json::to_value(runtime)
            .map_err(|err| ApiError(TtsError::Serialization(err.to_string())))?;
        return Ok(axum::Json(payload));
    }
    let runtime = state.service.manager.load_model(&id).await?;
    let payload = serde_json::to_value(runtime)
        .map_err(|err| ApiError(TtsError::Serialization(err.to_string())))?;
    Ok(axum::Json(payload))
}

/// `POST /v1/tts/models/{id}/unload`
async fn unload_model(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    if is_language_model(&state, &id) {
        state.service.llm.unload_model(&id).await?;
    } else {
        state.service.manager.unload_model(&id).await?;
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `true` when the model is a language model.
fn is_language_model(state: &ApiState, id: &str) -> bool {
    state
        .service
        .registry
        .resolve(id)
        .map(|spec| spec.family == ModelFamily::Llm)
        .unwrap_or(false)
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

    // A model that is part of the application needs no download.
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
        finish_install(&service, &spec.id, result);
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

/// `POST /v1/tts/models/{id}/install/cancel`
///
/// Stops a running download. The files that are already on the disk stay
/// there, the next attempt continues them.
async fn cancel_install(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let spec = resolve_model(&state.service, &id)?;
    state.service.installer.cancel(&spec.id);
    state.service.installs.cancel(&spec.id);
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `POST /v1/tts/models/{id}/install/discard`
///
/// Throws away the files of a download that was stopped: the space on the
/// disk is free again and the next attempt starts from zero.
async fn discard_install(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    // The model is known: the files are in the directory of the checkpoint.
    if let Ok(spec) = resolve_model(&state.service, &id) {
        if state.service.registry.is_installed(&spec.id) {
            return Err(ApiError::from(TtsError::InvalidRequest(format!(
                "`{}` 已经下载好了，要删除它请用删除模型",
                spec.id
            ))));
        }
        state.service.installer.cancel(&spec.id);
        let bytes = state.service.installer.discard(&spec)?;
        state.service.installs.remove(&spec.id);
        tracing::info!(model = %spec.id, bytes, "the files of the download were thrown away");
        return Ok(StatusCode::NO_CONTENT.into_response());
    }

    // The model is not in the catalogue any more (the service was restarted
    // and the model came from a search): the directory of the download is
    // looked for by its name.
    let Some(name) = safe_directory_name(&id) else {
        return Err(ApiError::from(TtsError::InvalidRequest(
            "这个名称不是有效的模型".to_string(),
        )));
    };
    let mut found = None;
    for root in [
        state.service.registry.models_root(),
        state.service.registry.llm_models_root(),
    ] {
        let directory = root.join(name);
        if directory.is_dir() {
            found = Some(directory);
            break;
        }
    }
    let Some(directory) = found else {
        return Err(ApiError::from(TtsError::ModelNotFound(format!(
            "unknown TTS model `{id}`"
        ))));
    };
    // Only a download that was interrupted has a part file. A directory
    // without one holds a model, and that is not thrown away here.
    if part_files_size(&directory) == 0 {
        return Err(ApiError::from(TtsError::InvalidRequest(format!(
            "`{id}` 不是中断的下载，请用删除模型来清理它"
        ))));
    }
    let bytes = tts_core::installer::directory_size(&directory);
    std::fs::remove_dir_all(&directory).map_err(|source| TtsError::Io {
        path: directory.clone(),
        source,
    })?;
    state.service.installs.remove(&id);
    tracing::info!(model = %id, bytes, "the files of the download were thrown away");
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// The name of a directory of a download, when it is safe to use.
fn safe_directory_name(id: &str) -> Option<&str> {
    if id.is_empty() || id.len() > 200 || id.contains("..") {
        return None;
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return None;
    }
    Some(id)
}

/// The progress of one download, with the name of the model.
#[derive(Debug, serde::Serialize)]
struct InstallDto {
    model_id: String,
    name: String,
    state: String,
    files_done: usize,
    files_total: usize,
    bytes: u64,
    bytes_total: u64,
    error: Option<String>,
}

/// `GET /v1/tts/installs`
///
/// The progress of every download the service knows about. The interface
/// uses it to show the downloads that are still running after a restart.
async fn list_installs(State(state): State<ApiState>) -> Result<Response, ApiError> {
    let mut list: Vec<InstallDto> = state
        .service
        .installs
        .list()
        .into_iter()
        .map(|progress| {
            let name = state
                .service
                .registry
                .get(&progress.model_id)
                .map(|spec| spec.name.clone())
                .unwrap_or_else(|| progress.model_id.clone());
            InstallDto {
                model_id: progress.model_id,
                name,
                state: progress.state,
                files_done: progress.files_done,
                files_total: progress.files_total,
                bytes: progress.bytes,
                bytes_total: progress.bytes_total,
                error: progress.error,
            }
        })
        .collect();

    // The downloads that were interrupted by a restart of the service: the
    // part files are still on the disk, the next attempt continues them.
    for spec in state.service.registry.list() {
        if list.iter().any(|entry| entry.model_id == spec.id) {
            continue;
        }
        let directory = state.service.installer.target_dir(&spec);
        let bytes = part_files_size(&directory);
        if bytes == 0 {
            continue;
        }
        let files = state
            .service
            .installer
            .list_files(&spec)
            .await
            .unwrap_or_default();
        let bytes_total: u64 = files.iter().filter_map(|file| file.size).sum();
        list.push(InstallDto {
            model_id: spec.id.clone(),
            name: spec.name.clone(),
            state: "partial".to_string(),
            files_done: 0,
            files_total: files.len(),
            bytes,
            bytes_total,
            error: None,
        });
    }
    // The downloads that no model of the catalogue belongs to: the service
    // was restarted and the model came from a search. The directories with
    // part files tell which ones they are.
    let known: Vec<String> = state
        .service
        .registry
        .list()
        .iter()
        .flat_map(|spec| {
            [
                spec.id.clone(),
                spec.hf_repo.replace('/', "--"),
            ]
        })
        .collect();
    for root in [
        state.service.registry.models_root(),
        state.service.registry.llm_models_root(),
    ] {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if known.iter().any(|item| item == name) {
                continue;
            }
            let bytes = part_files_size(&path);
            if bytes == 0 {
                continue;
            }
            list.push(InstallDto {
                model_id: name.to_string(),
                name: name.to_string(),
                state: "partial".to_string(),
                files_done: 0,
                files_total: 0,
                bytes,
                bytes_total: 0,
                error: None,
            });
        }
    }
    Ok(axum::Json(list).into_response())
}

/// The size of the files that a download left behind.
fn part_files_size(directory: &std::path::Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .map(|name| name.ends_with(".part"))
                .unwrap_or(false)
        })
        .filter_map(|entry| entry.metadata().ok())
        .map(|meta| meta.len())
        .sum()
}

/// `POST /v1/tts/models/{id}/uninstall`
async fn uninstall_model(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    body: Option<Json<UninstallRequest>>,
) -> Result<Response, ApiError> {
    let spec = resolve_model(&state.service, &id)?;
    // Deleting a checkpoint removes gigabytes from the disk, so the caller
    // has to say that it means it. A client that does not know this rule —
    // an old window that is still open, for example — is refused.
    let confirmed = body.map(|Json(body)| body.confirm).unwrap_or(false);
    if !confirmed {
        return Err(ApiError(TtsError::InvalidRequest(
            "删除模型需要确认：请在请求里带上 {\"confirm\": true}".to_string(),
        )));
    }
    if spec.is_builtin() {
        return Err(ApiError(TtsError::InvalidRequest(format!(
            "`{}` 内置于应用，无法删除",
            spec.id
        ))));
    }
    // A checkpoint that is in use must not disappear under the running
    // engine. The engine is stopped first, because the user asked for the
    // model to go.
    if is_language_model(&state, &spec.id) {
        let running = state
            .service
            .llm
            .runtimes()
            .iter()
            .any(|runtime| runtime.model_id == spec.id && runtime.state == RuntimeState::Running);
        if running {
            tracing::info!(model = %spec.id, "the engine is stopped before the checkpoint is removed");
            state.service.llm.unload_model(&spec.id).await?;
        }
    } else {
        let running = state
            .service
            .manager
            .status()
            .runtimes
            .iter()
            .any(|runtime| runtime.model_id == spec.id && runtime.state == RuntimeState::Running);
        if running {
            tracing::info!(model = %spec.id, "the engine is stopped before the checkpoint is removed");
            state.service.manager.unload_model(&spec.id).await?;
        }
    }
    state.service.installer.uninstall(&spec)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// The body of a request that removes a checkpoint.
#[derive(Debug, Deserialize)]
struct UninstallRequest {
    /// The caller confirms that the checkpoint may be removed.
    #[serde(default)]
    confirm: bool,
}

/// `GET /v1/tts/models/{id}/logs`
async fn model_logs(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let logs = if is_language_model(&state, &id) {
        state.service.llm.logs(&id)
    } else {
        state.service.manager.logs(&id).await?
    };
    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        Body::from(logs),
    )
        .into_response())
}

/// `POST /v1/chat/completions`
///
/// OpenAI compatible chat completion. The request is forwarded to the
/// server of the language model, which is started when it is not running.
async fn chat_completions(
    State(state): State<ApiState>,
    body: axum::Json<serde_json::Value>,
) -> Result<impl IntoResponse, ApiError> {
    let model = body
        .0
        .get("model")
        .and_then(|value| value.as_str())
        .ok_or_else(|| {
            ApiError(TtsError::InvalidRequest(
                "the request needs a `model`".to_string(),
            ))
        })?
        .to_string();
    let spec = resolve_model(&state.service, &model)?;
    if spec.family != ModelFamily::Llm {
        return Err(ApiError(TtsError::InvalidRequest(format!(
            "`{model}` is not a language model"
        ))));
    }
    let answer = state.service.llm.chat(&spec.id, body.0).await?;
    Ok(axum::Json(answer))
}

/// `POST /v1/completions`
///
/// OpenAI compatible plain completion.
async fn completions(
    State(state): State<ApiState>,
    body: axum::Json<serde_json::Value>,
) -> Result<impl IntoResponse, ApiError> {
    let model = body
        .0
        .get("model")
        .and_then(|value| value.as_str())
        .ok_or_else(|| {
            ApiError(TtsError::InvalidRequest(
                "the request needs a `model`".to_string(),
            ))
        })?
        .to_string();
    let spec = resolve_model(&state.service, &model)?;
    if spec.family != ModelFamily::Llm {
        return Err(ApiError(TtsError::InvalidRequest(format!(
            "`{model}` is not a language model"
        ))));
    }
    let answer = state.service.llm.completion(&spec.id, body.0).await?;
    Ok(axum::Json(answer))
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
        // The task type follows the profile: a designed voice needs the
        // description, a cloned one needs the reference clips and a custom
        // voice needs the speaker. Only the designed voice knows its task
        // type, the others let the checkpoint decide.
        let design = voice
            .engine_config
            .get("voice_design_prompt")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        match design {
            Some(prompt) => {
                params.task_type = Some(tts_core::types::Qwen3TtsTaskType::VoiceDesign);
                if params.instructions.is_none() {
                    params.instructions = Some(prompt);
                }
            }
            None => {
                params.task_type = None;
            }
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
        tts_core::types::ModelFamily::Llm => {
            EngineSpecificParameters::Generic {
                params: serde_json::Value::Null,
            }
        }
        // Every other family speaks a protocol that is not modelled yet.
        #[allow(unreachable_patterns)]
        _ => EngineSpecificParameters::Generic {
            params: serde_json::Value::Null,
        },
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
        tts_core::types::ModelFamily::Llm => {
            EngineSpecificParameters::Generic {
                params: serde_json::Value::Null,
            }
        }
        // Every other family speaks a protocol that is not modelled yet.
        #[allow(unreachable_patterns)]
        _ => EngineSpecificParameters::Generic {
            params: serde_json::Value::Null,
        },
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
