//! Tests of the language models that the native runtime of Apple serves.
//!
//! The MLX checkpoints of the community are safetensors files, not GGUF. The
//! hub used to drop them, because the language runtime of llama.cpp reads
//! nothing else — but the application owns the native runtime of Apple as
//! well, and it is the fastest one on a Mac.

mod common;

use common::TempDir;
use tts_core::engine_assets::{engine_definition, engine_reads_gguf, server_arguments};
use tts_core::paths::AppPaths;
use tts_core::service::TtsService;
use tts_core::types::{EngineMode, ModelFamily};

/// Writes an MLX checkpoint below a directory.
fn write_mlx(directory: &std::path::Path, name: &str) {
    std::fs::create_dir_all(directory).expect("failed to create the directory");
    std::fs::write(
        directory.join("config.json"),
        br#"{"architectures":["Qwen3ForCausalLM"],"quantization":{"group_size":64,"bits":4}}"#,
    )
    .expect("failed to write the config");
    std::fs::write(directory.join("model.safetensors"), [0_u8; 256])
        .expect("failed to write the weights");
    std::fs::write(directory.join("tokenizer.json"), b"{}").expect("failed to write the tokenizer");
    let _ = name;
}

/// A checkpoint of the native runtime is adopted into the catalogue.
#[tokio::test]
async fn an_mlx_checkpoint_is_adopted() {
    let dir = TempDir::new("mlx");
    let checkpoint = dir.path().join("models/llm/mlx-community--Qwen3-4B-4bit");
    write_mlx(&checkpoint, "mlx-community--Qwen3-4B-4bit");
    let service = TtsService::with_paths(AppPaths::with_root(dir.path())).expect("the service");
    let spec = service
        .registry
        .get("mlx-community--Qwen3-4B-4bit")
        .expect("the checkpoint was not adopted");

    // The native runtime of Apple serves it, not llama.cpp.
    assert_eq!(spec.family, ModelFamily::Llm);
    assert_eq!(spec.engine_id, "mlx");
    assert_eq!(spec.mode, EngineMode::MlxLlm);
    assert!(service.registry.is_installed(&spec.id));
    // The context of the MLX server is the one of the model, the flag of
    // llama.cpp is not understood by it.
    assert!(spec.launch_args.is_empty());
}

/// A checkpoint that is not an MLX one stays with the language runtime that
/// reads GGUF.
#[tokio::test]
async fn a_gguf_checkpoint_stays_with_llama_cpp() {
    let dir = TempDir::new("mlx");
    let checkpoint = dir.path().join("models/llm/unsloth--Qwen3.8-27B-GGUF");
    std::fs::create_dir_all(&checkpoint).expect("failed to create the directory");
    std::fs::write(checkpoint.join("model-Q4_K_M.gguf"), [0_u8; 256])
        .expect("failed to write the weights");
    let service = TtsService::with_paths(AppPaths::with_root(dir.path())).expect("the service");
    let spec = service
        .registry
        .get("unsloth--Qwen3.8-27B-GGUF")
        .expect("the checkpoint was not adopted");

    assert_eq!(spec.engine_id, "llama.cpp");
    assert_eq!(spec.mode, EngineMode::LlamaCpp);
    assert_eq!(spec.launch_args, vec!["--ctx-size", "8192"]);
}

/// The engines are told what they can read.
#[test]
fn the_engines_read_different_formats() {
    assert!(engine_reads_gguf("llama.cpp"));
    assert!(!engine_reads_gguf("mlx"));
    assert!(!engine_reads_gguf("mlx-audio"));
}

/// The arguments of the servers are the ones of their engine.
#[test]
fn every_engine_has_its_own_arguments() {
    let llama = server_arguments(
        "llama.cpp",
        std::path::Path::new("/models/llm/model.gguf"),
        "qwen3-8b-gguf",
        54321,
    );
    assert!(llama.contains(&"--model".to_string()));
    assert!(llama.contains(&"/models/llm/model.gguf".to_string()));
    // The name of the catalogue is the name of the model for llama.cpp.
    assert!(llama.contains(&"--alias".to_string()));
    assert!(llama.contains(&"qwen3-8b-gguf".to_string()));
    assert!(llama.contains(&"54321".to_string()));

    let mlx = server_arguments(
        "mlx",
        std::path::Path::new("/models/llm/mlx-community--Qwen3-4B-4bit"),
        "mlx-community--Qwen3-4B-4bit",
        54322,
    );
    assert!(mlx.contains(&"--model".to_string()));
    assert!(mlx.contains(&"/models/llm/mlx-community--Qwen3-4B-4bit".to_string()));
    // The MLX server knows the model by its path, it has no alias.
    assert!(!mlx.contains(&"--alias".to_string()));
}

/// The native runtime is wired to the models it serves.
#[test]
fn the_native_runtime_is_wired_to_its_models() {
    let definition =
        engine_definition("mlx").expect("the native runtime of Apple is not defined");
    // The engine serves the models whose engine id is `mlx`.
    assert!(definition.serves("mlx"));
    // The managed installation is the one that is used: the server is a
    // Python program of the environment of the engine.
    assert!(definition
        .managed_model_engine_ids
        .iter()
        .any(|id| id == "mlx"));
    assert!(definition.capabilities.iter().any(|item| item == "llm"));
}
