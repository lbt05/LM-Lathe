//! Endpoints that manage the inference engines.
//!
//! The runtime manager starts engines, these endpoints own the engines
//! themselves: which versions exist, which one is installed and which one is
//! active.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;

use tts_core::engine_assets::{
    engine_definition, probe_engine, EngineAsset, EngineHealth, EngineInstallProgress,
    EngineVersion,
};
use tts_core::error::TtsError;

use super::ApiError;
use super::ApiState;

/// Body of the install request.
#[derive(Debug, Default, Deserialize)]
pub struct InstallRequest {
    /// The version to install. The newest one is used when it is missing.
    #[serde(default)]
    pub version: Option<String>,
}

/// Body of the version request.
#[derive(Debug, Default, Deserialize)]
pub struct VersionRequest {
    /// The version to activate or to remove.
    pub version: String,
}

/// `GET /v1/runtimes`
pub async fn list_runtimes(State(state): State<ApiState>) -> Json<Vec<EngineAsset>> {
    Json(state.service.engines.list())
}

/// `GET /v1/runtimes/{id}`
pub async fn get_runtime(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Json<EngineAsset>, ApiError> {
    let engine = state
        .service
        .engines
        .get(&id)
        .ok_or_else(|| TtsError::InvalidRequest(format!("未知引擎 `{id}`")))?;
    Ok(Json(engine))
}

/// `GET /v1/runtimes/{id}/versions`
pub async fn list_versions(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<EngineVersion>>, ApiError> {
    Ok(Json(state.service.engines.versions(&id).await?))
}

/// `POST /v1/runtimes/{id}/install`
pub async fn install(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    body: Option<Json<InstallRequest>>,
) -> Result<Response, ApiError> {
    let version = body.and_then(|Json(body)| body.version);
    state.service.engines.start_install(&id, version)?;
    Ok((StatusCode::ACCEPTED, "the installation is running").into_response())
}

/// `GET /v1/runtimes/{id}/install-progress`
pub async fn install_progress(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Json<EngineInstallProgress> {
    Json(
        state
            .service
            .engines
            .install_progress(&id)
            .unwrap_or_else(|| EngineInstallProgress {
                engine_id: id,
                version: String::new(),
                state: "idle".to_string(),
                percent: None,
                bytes: 0,
                total_bytes: None,
                message: "尚未开始安装".to_string(),
                log: Vec::new(),
                error: None,
            }),
    )
}

/// `POST /v1/runtimes/{id}/activate`
pub async fn activate(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(body): Json<VersionRequest>,
) -> Result<Response, ApiError> {
    state.service.engines.activate(&id, &body.version)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `POST /v1/runtimes/{id}/uninstall`
pub async fn uninstall(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(body): Json<VersionRequest>,
) -> Result<Response, ApiError> {
    state.service.engines.uninstall(&id, &body.version)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `POST /v1/runtimes/{id}/health`
///
/// The engine service is started and its `/health` endpoint is probed. When
/// the engine already serves a model, the running instance is used.
pub async fn health(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Json<EngineHealth>, ApiError> {
    let definition =
        engine_definition(&id).ok_or_else(|| TtsError::InvalidRequest(format!("未知引擎 `{id}`")))?;

    // A model that the engine is able to serve is needed to start the service.
    let spec = state.service.registry.list().into_iter().find(|spec| {
        definition.serves(&spec.engine_id) && state.service.registry.is_installed(&spec.id)
    });
    let model = spec.as_ref().and_then(|spec| {
        state
            .service
            .registry
            .install_path(&spec.id)
            .map(|path| {
                // The engines that read GGUF want the file, the others the
                // directory with the weights.
                let path = if tts_core::engine_assets::engine_reads_gguf(&spec.engine_id) {
                    tts_core::gguf_checkpoint(&path).unwrap_or(path)
                } else {
                    path
                };
                (path, spec.hf_repo.clone())
            })
    });

    // The program depends on the model: the Apple path of the subsystem is
    // served by a build that the managed distribution does not contain.
    let entry = match &spec {
        Some(spec) => state.service.engines.resolve_program(&spec.engine_id),
        None => state.service.engines.resolve_entry(&id),
    };

    let Some(entry) = entry else {
        return Ok(Json(EngineHealth {
            engine_id: id,
            status: "missing".to_string(),
            detail: format!("{} 尚未安装，也没有系统级安装。", definition.name),
            entry: None,
            endpoint: None,
            duration_ms: 0,
        }));
    };

    Ok(Json(
        probe_engine(
            &definition,
            &entry,
            model
                .as_ref()
                .map(|(path, name)| (path.as_path(), name.as_str())),
        )
        .await,
    ))
}
