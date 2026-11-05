//! Tests of the runtime manager and its integration with the global
//! scheduler.

mod common;

use common::TempDir;
use tts_core::paths::AppPaths;
use tts_core::registry::TTSModelSpec;
use tts_core::service::TtsService;
use tts_core::types::{EngineMode, ModelFamily, RuntimeState, TTSRequest, TtsFeature};
use tts_core::voice::NewVoiceProfile;

fn service(dir: &TempDir) -> std::sync::Arc<TtsService> {
    let service = TtsService::with_paths(AppPaths::with_root(dir.path())).unwrap();
    // The synthetic model is not part of the catalogue of the application,
    // the test registers it itself.
    service.registry.insert(TTSModelSpec::mock_spec());
    service
}

/// A second mock model, used to test the one-model-per-instance policy.
fn second_mock_model() -> TTSModelSpec {
    TTSModelSpec {
        id: "mock-echo-2".to_string(),
        name: "Mock Echo 2".to_string(),
        family: ModelFamily::Mock,
        engine_id: "mock".to_string(),
        hf_repo: "mock://echo-2".to_string(),
        version: "1".to_string(),
        format: "none".to_string(),
        size_bytes: 0,
        estimated_memory_bytes: 64 * 1024 * 1024,
        parameters: None,
        features: vec![TtsFeature::TextToSpeech],
        llm_features: vec![],
        files: vec![],
        hardware: Default::default(),
        mode: EngineMode::Mock,
        default_ttl_secs: None,
        aliases: vec![],
        launch_args: vec![],
        local_path: None,
        notes: None,
    }
}

#[tokio::test]
async fn a_model_is_loaded_and_unloaded() {
    let dir = TempDir::new("runtime");
    let service = service(&dir);

    let runtime = service.manager.load_model("mock-echo").await.unwrap();
    assert_eq!(runtime.model_id, "mock-echo");
    assert_eq!(runtime.state, RuntimeState::Running);
    assert!(runtime.endpoint.is_some());

    // The model occupies the engine and the memory of the global scheduler.
    let status = service.manager.status();
    assert_eq!(status.active_model.as_deref(), Some("mock-echo"));
    assert_eq!(status.scheduler.used_bytes, 64 * 1024 * 1024);
    assert!(status
        .scheduler
        .tenants
        .iter()
        .any(|tenant| tenant.key == "tts:mock-echo"));

    service.manager.unload_model("mock-echo").await.unwrap();
    let status = service.manager.status();
    assert_eq!(status.active_model, None);
    assert_eq!(status.scheduler.used_bytes, 0);
    assert_eq!(
        service.manager.runtime("mock-echo").unwrap().state,
        RuntimeState::Unloaded
    );
}

#[tokio::test]
async fn synthesis_writes_an_audio_file() {
    let dir = TempDir::new("runtime");
    let service = service(&dir);

    let request = TTSRequest::new("mock-echo", "你好，世界");
    let result = service.manager.synthesize(request).await.unwrap();
    assert!(!result.output.data.is_empty());
    assert_eq!(result.model_id, "mock-echo");

    let path = result.file_path.expect("the audio is stored on disk");
    let info = service
        .audio
        .inspect_file(std::path::Path::new(&path))
        .unwrap();
    assert_eq!(info.sample_rate, 24_000);
    assert_eq!(info.channels, 1);
    assert!(info.duration_ms > 0);
}

#[tokio::test]
async fn a_voice_profile_is_used_for_the_synthesis() {
    let dir = TempDir::new("runtime");
    let service = service(&dir);

    let voice = service
        .library
        .create(NewVoiceProfile {
            name: Some("测试声音".to_string()),
            model_id: Some("mock-echo".to_string()),
            engine: Some("mock".to_string()),
            engine_config: serde_json::json!({"voice_design_prompt": "温柔的声音"}),
            ..Default::default()
        })
        .await
        .unwrap();

    let request = TTSRequest::new("mock-echo", "测试").with_voice(voice.id.clone());
    let result = service.manager.synthesize(request).await.unwrap();
    assert_eq!(result.voice_id.as_deref(), Some(voice.id.as_str()));
}

#[tokio::test]
async fn an_unknown_voice_is_reported() {
    let dir = TempDir::new("runtime");
    let service = service(&dir);
    let request = TTSRequest::new("mock-echo", "测试").with_voice("voice_missing");
    let error = service.manager.synthesize(request).await.unwrap_err();
    assert!(error.to_string().contains("voice_missing"));
}

#[tokio::test]
async fn loading_a_second_model_unloads_the_first_one() {
    let dir = TempDir::new("runtime");
    let service = service(&dir);
    service.registry.insert(second_mock_model());

    service.manager.load_model("mock-echo").await.unwrap();
    service.manager.load_model("mock-echo-2").await.unwrap();

    let status = service.manager.status();
    assert_eq!(status.active_model.as_deref(), Some("mock-echo-2"));
    assert_eq!(
        service.manager.runtime("mock-echo").unwrap().state,
        RuntimeState::Unloaded
    );
    // Only one model holds memory at a time.
    assert_eq!(status.scheduler.used_bytes, 64 * 1024 * 1024);
}

#[tokio::test]
async fn a_pinned_model_may_not_be_replaced() {
    let dir = TempDir::new("runtime");
    let service = service(&dir);
    service.registry.insert(second_mock_model());

    service.manager.load_model("mock-echo").await.unwrap();
    service.manager.set_pinned("mock-echo", true).unwrap();

    let error = service.manager.load_model("mock-echo-2").await.unwrap_err();
    assert!(error.to_string().contains("pinned"));
    assert_eq!(
        service.manager.status().active_model.as_deref(),
        Some("mock-echo")
    );
}

#[tokio::test]
async fn an_incompatible_model_is_refused() {
    let dir = TempDir::new("runtime");
    let service = service(&dir);
    // Drama 3 needs CUDA, the test machine is a Mac.
    let error = service.manager.load_model("drama-3").await.unwrap_err();
    assert!(error.to_string().contains("not supported on this machine"));
}

#[tokio::test]
async fn an_unknown_model_is_refused() {
    let dir = TempDir::new("runtime");
    let service = service(&dir);
    let error = service
        .manager
        .load_model("does-not-exist")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("unknown TTS model"));
}

#[tokio::test]
async fn the_runtime_reports_its_state() {
    let dir = TempDir::new("runtime");
    let service = service(&dir);
    service.manager.load_model("mock-echo").await.unwrap();
    let status = service.manager.status();
    assert_eq!(status.runtimes.len(), 1);
    assert_eq!(status.runtimes[0].model_id, "mock-echo");
    assert_eq!(status.runtimes[0].state, RuntimeState::Running);
    assert!(status.hardware.total_ram_bytes > 0);
}

#[tokio::test]
async fn a_model_that_is_not_installed_is_refused_immediately() {
    let dir = TempDir::new("runtime");
    let service = service(&dir);
    // The checkpoint was never downloaded, so the engine must not be started.
    let error = service
        .manager
        .load_model("qwen3-tts-1.7b-voicedesign")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("not installed"));
}
