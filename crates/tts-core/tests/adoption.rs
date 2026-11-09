//! Tests of the adoption of the checkpoints that are on the disk.
//!
//! The catalogue of the application lives in the memory of the service, so
//! a model that was found in a search of the hub is forgotten when the
//! service stops. The checkpoints that are on the disk are adopted into the
//! catalogue again, so that they do not disappear from the interface.

mod common;

use common::TempDir;
use tts_core::paths::AppPaths;
use tts_core::service::TtsService;
use tts_core::types::{EngineMode, ModelFamily};

/// Writes a file below a directory of a checkpoint.
fn write(directory: &std::path::Path, name: &str, bytes: &[u8]) {
    let path = directory.join(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("failed to create the directory");
    }
    std::fs::write(path, bytes).expect("failed to write the file");
}

/// A checkpoint that was downloaded from a search of the hub is adopted.
#[tokio::test]
async fn a_checkpoint_on_the_disk_is_adopted() {
    let dir = TempDir::new("adopt");
    let checkpoint = dir.path().join("models/llm/unsloth--Qwen3.8-27B-GGUF");
    write(&checkpoint, "Qwen3.8-27B-UD-Q4_K_M.gguf", &[0_u8; 4096]);
    let service = TtsService::with_paths(AppPaths::with_root(dir.path())).expect("the service");
    let spec = service
        .registry
        .get("unsloth--Qwen3.8-27B-GGUF")
        .expect("the checkpoint was not adopted");

    assert_eq!(spec.family, ModelFamily::Llm);
    assert_eq!(spec.engine_id, "llama.cpp");
    assert_eq!(spec.mode, EngineMode::LlamaCpp);
    assert_eq!(spec.hf_repo, "unsloth/Qwen3.8-27B-GGUF");
    assert!(service.registry.is_installed(&spec.id));
    assert_eq!(
        service.registry.install_path(&spec.id).as_deref(),
        Some(checkpoint.as_path())
    );
    // The size is the one of the files on the disk.
    assert_eq!(spec.size_bytes, 4096);
    // The file that is on the disk is the one the checkpoint is made of.
    assert_eq!(spec.files, vec!["Qwen3.8-27B-UD-Q4_K_M.gguf"]);
}

/// A speech checkpoint that was downloaded from a search of the hub is
/// adopted as well.
#[tokio::test]
async fn a_speech_checkpoint_on_the_disk_is_adopted() {
    let dir = TempDir::new("adopt");
    let checkpoint = dir
        .path()
        .join("models/tts/mlx-community--Qwen3-TTS-12Hz-0.6B-Base-bf16");
    write(&checkpoint, "model.safetensors", &[0_u8; 512]);
    write(&checkpoint, "config.json", b"{}");
    let service = TtsService::with_paths(AppPaths::with_root(dir.path())).expect("the service");
    let spec = service
        .registry
        .get("mlx-community--Qwen3-TTS-12Hz-0.6B-Base-bf16")
        .expect("the checkpoint was not adopted");

    assert_eq!(spec.family, ModelFamily::Qwen3Tts);
    // The MLX checkpoints are served by mlx-audio.
    assert_eq!(spec.engine_id, "mlx-audio");
    assert_eq!(spec.mode, EngineMode::MlxAudio);
    assert!(service.registry.is_installed(&spec.id));
}

/// A checkpoint that the catalogue already knows is not adopted a second
/// time: the model of the built-in catalogue keeps its own name.
#[tokio::test]
async fn a_known_checkpoint_is_not_adopted_twice() {
    let dir = TempDir::new("adopt");
    let checkpoint = dir
        .path()
        .join("models/tts/mlx-community--Qwen3-TTS-12Hz-1.7B-VoiceDesign-4bit");
    write(&checkpoint, "model.safetensors", &[0_u8; 512]);
    let service = TtsService::with_paths(AppPaths::with_root(dir.path())).expect("the service");

    assert!(service
        .registry
        .get("qwen3-tts-1.7b-voicedesign-mlx")
        .is_some());
    assert!(service
        .registry
        .get("mlx-community--Qwen3-TTS-12Hz-1.7B-VoiceDesign-4bit")
        .is_none());
    // The known model reports that it is on the disk.
    assert!(service
        .registry
        .is_installed("qwen3-tts-1.7b-voicedesign-mlx"));
}

/// A download that was interrupted is not a model: the progress of the
/// download knows about it, the catalogue does not.
#[tokio::test]
async fn an_interrupted_download_is_not_adopted() {
    let dir = TempDir::new("adopt");
    let checkpoint = dir.path().join("models/llm/some--Interrupted-GGUF");
    write(&checkpoint, "Model-Q4_K_M.gguf.part", &[0_u8; 128]);
    let service = TtsService::with_paths(AppPaths::with_root(dir.path())).expect("the service");

    assert!(service.registry.get("some--Interrupted-GGUF").is_none());
    assert!(service.adopted.is_empty());
}

/// A directory that holds no weights of a known kind stays a directory.
#[tokio::test]
async fn an_unknown_directory_is_not_adopted() {
    let dir = TempDir::new("adopt");
    let checkpoint = dir.path().join("models/llm/some--Notes");
    write(&checkpoint, "readme.md", b"hello");
    let service = TtsService::with_paths(AppPaths::with_root(dir.path())).expect("the service");

    assert!(service.registry.get("some--Notes").is_none());
    assert!(service.adopted.is_empty());
}
