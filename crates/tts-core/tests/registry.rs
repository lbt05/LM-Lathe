//! Tests of the model registry and the hardware compatibility check.

use tts_core::hardware::HardwareProfile;
use tts_core::registry::{CompatibilityLevel, TTSModelRegistry};
use tts_core::types::{ModelFamily, TtsFeature};

fn apple_silicon() -> HardwareProfile {
    HardwareProfile::new("macos", "aarch64", true, false, 64 * 1024 * 1024 * 1024)
}

fn cuda_linux() -> HardwareProfile {
    HardwareProfile::new("linux", "x86_64", false, true, 128 * 1024 * 1024 * 1024)
}

#[test]
fn builtin_catalog_contains_the_m1_models() {
    let registry = TTSModelRegistry::builtin();
    assert!(registry.get("qwen3-tts-1.7b-voicedesign").is_some());
    assert!(registry.get("qwen3-tts-1.7b-customvoice").is_some());
    assert!(registry.get("qwen3-tts-1.7b-base").is_some());
    assert!(registry.get("drama-3").is_some());
    assert!(registry.get("drama-3-int4").is_some());
    assert!(registry.get("drama-3-mlx-4bit").is_some());
}

#[test]
fn aliases_resolve_to_the_model() {
    let registry = TTSModelRegistry::builtin();
    let spec = registry.resolve("qwen3-tts").expect("the alias resolves");
    assert_eq!(spec.id, "qwen3-tts-1.7b-voicedesign");
    assert_eq!(spec.family, ModelFamily::Qwen3Tts);

    let drama = registry
        .resolve("fish-drama-3")
        .expect("the alias resolves");
    assert_eq!(drama.id, "drama-3");
    assert_eq!(drama.family, ModelFamily::FishDrama3);

    assert!(registry.resolve("does-not-exist").is_none());
}

#[test]
fn voice_design_is_offered_by_the_qwen_voicedesign_checkpoint() {
    let registry = TTSModelRegistry::builtin();
    let spec = registry.get("qwen3-tts-1.7b-voicedesign").unwrap();
    assert!(spec.supports(TtsFeature::VoiceDesign));
    assert!(spec.supports(TtsFeature::TextToSpeech));
    assert!(!spec.supports(TtsFeature::MultiSpeaker));

    let drama = registry.get("drama-3").unwrap();
    assert!(drama.supports(TtsFeature::MultiSpeaker));
    assert!(drama.supports(TtsFeature::PointTags));
    assert!(!drama.supports(TtsFeature::VoiceDesign));
}

#[test]
fn drama_3_is_rejected_on_apple_silicon() {
    let registry = TTSModelRegistry::builtin();
    let spec = registry.get("drama-3").unwrap();
    let report = registry.check_compatibility(&spec, &apple_silicon());
    assert_eq!(report.level, CompatibilityLevel::Unsupported);
    assert!(!report.compatible);
    assert!(report.notes.iter().any(|note| note.contains("CUDA")));
}

#[test]
fn drama_3_is_supported_on_a_cuda_machine() {
    let registry = TTSModelRegistry::builtin();
    let spec = registry.get("drama-3").unwrap();
    let report = registry.check_compatibility(&spec, &cuda_linux());
    assert_eq!(report.level, CompatibilityLevel::Supported);
    assert!(report.compatible);
}

#[test]
fn the_mlx_variant_requires_enough_memory() {
    let registry = TTSModelRegistry::builtin();
    let spec = registry.get("drama-3-mlx-4bit").unwrap();
    let small = HardwareProfile::new("macos", "aarch64", true, false, 32 * 1024 * 1024 * 1024);
    let report = registry.check_compatibility(&spec, &small);
    assert!(!report.compatible);
    assert!(report.notes.iter().any(|note| note.contains("memory")));

    let large = HardwareProfile::new("macos", "aarch64", true, false, 64 * 1024 * 1024 * 1024);
    let report = registry.check_compatibility(&spec, &large);
    assert!(report.compatible);
}

#[test]
fn the_catalog_round_trips_through_a_file() {
    let registry = TTSModelRegistry::builtin();
    let dir = std::env::temp_dir().join(format!("registry-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("models.json");
    registry.save_catalog(&path).unwrap();

    let other = TTSModelRegistry::empty();
    let count = other.load_catalog(&path).unwrap();
    assert_eq!(count, registry.len());
    assert_eq!(other.get("drama-3").unwrap().id, "drama-3");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_install_path_follows_the_directory_layout() {
    let registry = TTSModelRegistry::builtin().with_models_root("/tmp/models/tts");
    let path = registry.install_path("qwen3-tts-1.7b-voicedesign").unwrap();
    assert_eq!(
        path,
        std::path::Path::new("/tmp/models/tts/Qwen--Qwen3-TTS-12Hz-1.7B-VoiceDesign")
    );
    // An alias resolves to the same directory.
    assert_eq!(registry.install_path("qwen3-tts").unwrap(), path);
    // A model that is not on the disk is not installed.
    assert!(!registry.is_installed("qwen3-tts-1.7b-voicedesign"));
}
