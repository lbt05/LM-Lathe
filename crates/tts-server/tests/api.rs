//! Integration tests of the HTTP API.

use std::path::PathBuf;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;
use tts_core::paths::AppPaths;
use tts_core::service::TtsService;

/// Makes the names of the temporary directories unique.
///
/// The clock of the operating system is not precise enough: two threads can
/// read the same timestamp, and the tests that run in parallel would then
/// share one directory and delete it from under each other.
fn unique_suffix() -> String {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sequence = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{now}-{sequence}")
}

/// A temporary directory that removes itself when it is dropped.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> Self {
        let path = std::env::temp_dir().join(format!("{prefix}-{}", unique_suffix()));
        std::fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn router(dir: &TempDir) -> axum::Router {
    let service = TtsService::with_paths(AppPaths::with_root(dir.path())).unwrap();
    tts_server::api::router(service)
}

async fn get(router: axum::Router, uri: &str) -> (StatusCode, axum::body::Body) {
    let request = Request::builder().uri(uri).body(Body::empty()).unwrap();
    let response = router.oneshot(request).await.unwrap();
    let status = response.status();
    (status, response.into_body())
}

async fn post_json(
    router: axum::Router,
    uri: &str,
    body: serde_json::Value,
) -> (StatusCode, axum::body::Body) {
    let request = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    let status = response.status();
    (status, response.into_body())
}

async fn patch_json(
    router: axum::Router,
    uri: &str,
    body: serde_json::Value,
) -> (StatusCode, axum::body::Body) {
    let request = Request::builder()
        .method("PATCH")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    let status = response.status();
    (status, response.into_body())
}

#[tokio::test]
async fn the_health_endpoint_answers() {
    let dir = TempDir::new("api");
    let (status, body) = get(router(&dir), "/health").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value =
        serde_json::from_slice(&body.collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(json["status"], "ok");
}

#[tokio::test]
async fn the_registry_is_exposed() {
    let dir = TempDir::new("api");
    let (status, body) = get(router(&dir), "/v1/tts/models").await;
    assert_eq!(status, StatusCode::OK);
    let models: Vec<serde_json::Value> =
        serde_json::from_slice(&body.collect().await.unwrap().to_bytes()).unwrap();
    assert!(models.iter().any(|model| model["id"] == "mock-echo"));
    assert!(models.iter().any(|model| model["id"] == "drama-3"));

    // The compatibility verdict is part of the answer.
    let drama = models
        .iter()
        .find(|model| model["id"] == "drama-3")
        .unwrap();
    assert!(drama["features"]
        .as_array()
        .unwrap()
        .iter()
        .any(|feature| feature == "multi_speaker"));
}

#[tokio::test]
async fn the_openai_compatible_endpoint_synthesises_speech() {
    let dir = TempDir::new("api");
    let router = router(&dir);
    let (status, body) = post_json(
        router.clone(),
        "/v1/audio/speech",
        serde_json::json!({
            "model": "mock",
            "input": "你好，欢迎使用本地 AI 工作台。",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let bytes = body.collect().await.unwrap().to_bytes();
    assert!(!bytes.is_empty());
    // The payload is a RIFF/WAVE file.
    assert_eq!(&bytes[0..4], b"RIFF");
    assert_eq!(&bytes[8..12], b"WAVE");
}

#[tokio::test]
async fn an_unknown_model_is_reported_as_not_found() {
    let dir = TempDir::new("api");
    let (status, body) = post_json(
        router(&dir),
        "/v1/audio/speech",
        serde_json::json!({"model": "does-not-exist", "input": "x"}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let json: serde_json::Value =
        serde_json::from_slice(&body.collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(json["kind"], "model_not_found");
}

#[tokio::test]
async fn a_voice_is_designed_and_stored() {
    let dir = TempDir::new("api");
    let router = router(&dir);
    let (status, body) = post_json(
        router.clone(),
        "/v1/tts/voices/design",
        serde_json::json!({
            "model": "mock",
            "prompt": "青年女性，稚嫩、害羞、略带怯意",
            "name": "府中贴身婢女",
            "language": "Chinese",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let json: serde_json::Value =
        serde_json::from_slice(&body.collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(json["voice"]["name"], "府中贴身婢女");
    assert_eq!(
        json["voice"]["engine_config"]["voice_design_prompt"],
        "青年女性，稚嫩、害羞、略带怯意"
    );
    assert!(json["sample"].as_str().unwrap().starts_with("samples/"));

    // The voice is listed and can be updated.
    let (status, body) = get(router.clone(), "/v1/tts/voices").await;
    assert_eq!(status, StatusCode::OK);
    let voices: Vec<serde_json::Value> =
        serde_json::from_slice(&body.collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(voices.len(), 1);
    let id = voices[0]["id"].as_str().unwrap().to_string();

    let (status, _) = patch_json(
        router.clone(),
        &format!("/v1/tts/voices/{id}"),
        serde_json::json!({"name": "新名字"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = get(router, &format!("/v1/tts/voices/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value =
        serde_json::from_slice(&body.collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(json["name"], "新名字");
}

#[tokio::test]
async fn a_model_can_be_loaded_and_pinned() {
    let dir = TempDir::new("api");
    let router = router(&dir);

    let (status, body) = post_json(
        router.clone(),
        "/v1/tts/models/mock-echo/load",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value =
        serde_json::from_slice(&body.collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(json["state"], "running");

    let (status, _) = post_json(
        router.clone(),
        "/v1/tts/models/mock-echo/pin",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, body) = get(router.clone(), "/v1/tts/status").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value =
        serde_json::from_slice(&body.collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(json["active_model"], "mock-echo");
    assert_eq!(json["scheduler"]["tenants"][0]["pinned"], true);

    let (status, _) = post_json(
        router.clone(),
        "/v1/tts/models/mock-echo/unload",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn a_builtin_model_cannot_be_deleted() {
    let dir = TempDir::new("api");
    let router = router(&dir);

    let (status, body) = post_json(
        router.clone(),
        "/v1/tts/models/mock-echo/uninstall",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let json: serde_json::Value =
        serde_json::from_slice(&body.collect().await.unwrap().to_bytes()).unwrap();
    assert!(
        json["message"]
            .as_str()
            .unwrap_or_default()
            .contains("内置于应用"),
        "the message does not explain the refusal: {json}"
    );
}

#[tokio::test]
async fn the_mlx_checkpoint_of_qwen3_tts_is_part_of_the_catalogue() {
    let dir = TempDir::new("api");
    let router = router(&dir);

    let (status, body) = get(router, "/v1/tts/models").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value =
        serde_json::from_slice(&body.collect().await.unwrap().to_bytes()).unwrap();
    let mlx = json
        .as_array()
        .unwrap()
        .iter()
        .find(|model| model["id"] == "qwen3-tts-0.6b-customvoice-mlx")
        .expect("the MLX checkpoint of Qwen3-TTS is missing in the catalogue");
    assert_eq!(mlx["engine_id"], "sglang-omni-mlx");
    assert_eq!(mlx["hf_repo"], "mlx-community/Qwen3-TTS-12Hz-0.6B-CustomVoice-bf16");
}

#[tokio::test]
async fn a_generated_sample_can_be_downloaded() {
    let dir = TempDir::new("api");
    let router = router(&dir);
    let (_, body) = post_json(
        router.clone(),
        "/v1/tts/voices/design",
        serde_json::json!({"model": "mock", "prompt": "低沉的声音", "sample_text": "试听"}),
    )
    .await;
    let json: serde_json::Value =
        serde_json::from_slice(&body.collect().await.unwrap().to_bytes()).unwrap();
    let id = json["voice"]["id"].as_str().unwrap().to_string();
    let sample = json["sample"].as_str().unwrap().to_string();

    let (status, body) = get(router, &format!("/v1/tts/voices/{id}/samples/{sample}")).await;
    assert_eq!(status, StatusCode::OK);
    let bytes = body.collect().await.unwrap().to_bytes();
    assert_eq!(&bytes[0..4], b"RIFF");
}

#[tokio::test]
async fn only_the_models_that_can_be_served_are_listed() {
    let dir = TempDir::new("api");
    let (status, body) = get(router(&dir), "/v1/models").await;
    assert_eq!(status, StatusCode::OK);
    let models: Vec<serde_json::Value> =
        serde_json::from_slice(&body.collect().await.unwrap().to_bytes()).unwrap();

    // The mock engine is part of the application, so the server can serve it.
    assert!(models.iter().any(|model| model["id"] == "mock-echo"));

    // The checkpoints of the other models are not on the disk yet.
    assert!(!models
        .iter()
        .any(|model| model["id"] == "qwen3-tts-1.7b-voicedesign"));

    // The catalogue keeps them.
    let (status, body) = get(router(&dir), "/v1/tts/models").await;
    assert_eq!(status, StatusCode::OK);
    let catalogue: Vec<serde_json::Value> =
        serde_json::from_slice(&body.collect().await.unwrap().to_bytes()).unwrap();
    assert!(catalogue
        .iter()
        .any(|model| model["id"] == "qwen3-tts-1.7b-voicedesign"));
}

#[tokio::test]
async fn the_model_list_reports_the_installation_state() {
    let dir = TempDir::new("api");
    let (status, body) = get(router(&dir), "/v1/tts/models").await;
    assert_eq!(status, StatusCode::OK);
    let models: Vec<serde_json::Value> =
        serde_json::from_slice(&body.collect().await.unwrap().to_bytes()).unwrap();
    let qwen = models
        .iter()
        .find(|model| model["id"] == "qwen3-tts-1.7b-voicedesign")
        .unwrap();
    assert_eq!(qwen["installed"], false);
    assert!(qwen["path"]
        .as_str()
        .unwrap()
        .ends_with("Qwen--Qwen3-TTS-12Hz-1.7B-VoiceDesign"));
}

#[tokio::test]
async fn an_incompatible_model_is_not_installed() {
    let dir = TempDir::new("api");
    // Drama 3 needs CUDA, the test machine is a Mac.
    let (status, body) = post_json(
        router(&dir),
        "/v1/tts/models/drama-3/install",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let json: serde_json::Value =
        serde_json::from_slice(&body.collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(json["kind"], "incompatible_hardware");
}

#[tokio::test]
async fn the_installation_progress_is_reported() {
    let dir = TempDir::new("api");
    let (status, body) = get(router(&dir), "/v1/tts/models/mock-echo/install").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value =
        serde_json::from_slice(&body.collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(json["state"], "idle");
    assert_eq!(json["model_id"], "mock-echo");
}

#[tokio::test]
async fn the_catalogue_knows_the_language_models() {
    let dir = TempDir::new("api");
    let (status, body) = get(router(&dir), "/v1/tts/models").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value =
        serde_json::from_slice(&body.collect().await.unwrap().to_bytes()).unwrap();
    let models = json.as_array().unwrap();

    let llama = models
        .iter()
        .find(|model| model["id"] == "qwen3-4b-gguf")
        .expect("the language model is missing in the catalogue");
    assert_eq!(llama["family"], "llm");
    assert_eq!(llama["engine_id"], "llama.cpp");
    assert_eq!(llama["format"], "gguf");
    assert!(llama["llm_features"]
        .as_array()
        .unwrap()
        .iter()
        .any(|feature| feature == "chat"));

    // The speech models keep their family.
    let speech = models
        .iter()
        .find(|model| model["id"] == "qwen3-tts-1.7b-base")
        .expect("the speech model is missing in the catalogue");
    assert_eq!(speech["family"], "qwen3-tts");
}

#[tokio::test]
async fn a_chat_request_needs_a_language_model() {
    let dir = TempDir::new("api");
    let (status, body) = post_json(
        router(&dir),
        "/v1/chat/completions",
        serde_json::json!({
            "model": "mock-echo",
            "messages": [{"role": "user", "content": "hello"}],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let json: serde_json::Value =
        serde_json::from_slice(&body.collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(json["kind"], "invalid_request");
}

#[tokio::test]
async fn a_chat_request_without_a_model_is_rejected() {
    let dir = TempDir::new("api");
    let (status, body) = post_json(
        router(&dir),
        "/v1/chat/completions",
        serde_json::json!({"messages": [{"role": "user", "content": "hello"}]}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let json: serde_json::Value =
        serde_json::from_slice(&body.collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(json["kind"], "invalid_request");
}

/// The search of the hub answers with the models of the query, and a
/// query without a text is empty.
#[tokio::test]
async fn the_hub_is_searched_for_models() {
    let dir = TempDir::new("api");
    let (status, body) = get(router(&dir), "/v1/tts/discover?q=Qwen3-TTS").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value =
        serde_json::from_slice(&body.collect().await.unwrap().to_bytes()).unwrap();
    let models = json["models"].as_array().unwrap();

    // The hub answers with the repositories of the query; every one of them
    // has to be a model the runtime can serve.
    assert!(!models.is_empty());
    for model in models {
        assert!(model["repo"].as_str().unwrap().contains('/'));
        assert!(model["family"] == "llm" || model["family"] == "qwen3-tts");
        assert!(!model["id"].as_str().unwrap().contains('/'));
    }
}
