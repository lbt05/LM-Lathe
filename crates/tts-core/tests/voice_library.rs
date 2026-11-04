//! Tests of the voice library.

mod common;

use common::TempDir;
use tts_core::types::{AudioFormat, AudioOutput};
use tts_core::voice::fs::FsVoiceLibrary;
use tts_core::voice::{NewVoiceProfile, VoiceFilter, VoiceLibrary, VoicePatch};

fn library(dir: &TempDir) -> FsVoiceLibrary {
    FsVoiceLibrary::new(dir.path().join("voices"))
}

fn sample() -> AudioOutput {
    AudioOutput {
        data: vec![0u8; 128],
        format: AudioFormat::Wav,
        sample_rate: Some(24_000),
        channels: Some(1),
        duration_ms: Some(500),
        finish_reason: None,
    }
}

#[tokio::test]
async fn a_voice_is_created_and_stored() {
    let dir = TempDir::new("voices");
    let library = library(&dir);

    let voice = library
        .create(NewVoiceProfile {
            name: Some("府中贴身婢女".to_string()),
            description: Some("青年女性，稚嫩、害羞".to_string()),
            language: Some("Chinese".to_string()),
            engine: Some("qwen3-tts".to_string()),
            model_id: Some("qwen3-tts-1.7b-voicedesign".to_string()),
            engine_config: serde_json::json!({
                "voice_design_prompt": "青年女性，稚嫩、害羞、略带怯意"
            }),
            ..Default::default()
        })
        .await
        .unwrap();

    assert!(voice.id.starts_with("voice_"));
    assert_eq!(voice.name, "府中贴身婢女");
    assert_eq!(voice.language.as_deref(), Some("Chinese"));
    assert!(voice.is_voice_design());

    // The profile is stored as JSON inside the voice directory.
    let profile = dir
        .path()
        .join("voices")
        .join(&voice.id)
        .join("profile.json");
    assert!(profile.exists());
    let raw = std::fs::read_to_string(&profile).unwrap();
    assert!(raw.contains("voice_design_prompt"));

    let loaded = library.get(&voice.id).await.unwrap().unwrap();
    assert_eq!(loaded.name, voice.name);
    assert_eq!(loaded.engine_config, voice.engine_config);
}

#[tokio::test]
async fn voices_can_be_filtered() {
    let dir = TempDir::new("voices");
    let library = library(&dir);
    library
        .create(NewVoiceProfile {
            name: Some("A".to_string()),
            model_id: Some("qwen3-tts-1.7b-voicedesign".to_string()),
            engine: Some("qwen3-tts".to_string()),
            language: Some("Chinese".to_string()),
            ..Default::default()
        })
        .await
        .unwrap();
    library
        .create(NewVoiceProfile {
            name: Some("B".to_string()),
            model_id: Some("drama-3".to_string()),
            engine: Some("fish-drama-3".to_string()),
            language: Some("English".to_string()),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(library.list(VoiceFilter::default()).await.unwrap().len(), 2);
    assert_eq!(
        library
            .list(VoiceFilter {
                model_id: Some("drama-3".to_string()),
                ..Default::default()
            })
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        library
            .list(VoiceFilter {
                language: Some("Chinese".to_string()),
                ..Default::default()
            })
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        library
            .list(VoiceFilter {
                query: Some("B".to_string()),
                ..Default::default()
            })
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn a_voice_can_be_updated_and_deleted() {
    let dir = TempDir::new("voices");
    let library = library(&dir);
    let voice = library.create(NewVoiceProfile::default()).await.unwrap();

    let updated = library
        .update(
            &voice.id,
            VoicePatch {
                name: Some("new name".to_string()),
                description: Some("new description".to_string()),
                engine_config: Some(serde_json::json!({"speaker": "Vivian"})),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.name, "new name");
    assert_eq!(updated.description, "new description");
    assert!(updated.updated_at >= updated.created_at);

    library.delete(&voice.id).await.unwrap();
    assert!(library.get(&voice.id).await.unwrap().is_none());
}

#[tokio::test]
async fn samples_are_stored_with_the_voice() {
    let dir = TempDir::new("voices");
    let library = library(&dir);
    let voice = library.create(NewVoiceProfile::default()).await.unwrap();

    let sample = library
        .add_sample(&voice.id, &sample(), None)
        .await
        .unwrap();
    assert!(sample.name.starts_with("samples/"));
    assert!(sample.duration_ms == Some(500));

    let stored = library.get(&voice.id).await.unwrap().unwrap();
    assert_eq!(stored.samples.len(), 1);

    let path = library.sample_path(&voice.id, &sample.name).await.unwrap();
    assert!(path.exists());

    let samples = library.samples(&voice.id).await.unwrap();
    assert_eq!(samples.len(), 1);

    library
        .remove_sample(&voice.id, &sample.name)
        .await
        .unwrap();
    let stored = library.get(&voice.id).await.unwrap().unwrap();
    assert!(stored.samples.is_empty());
}

#[tokio::test]
async fn a_missing_voice_is_reported() {
    let dir = TempDir::new("voices");
    let library = library(&dir);
    let error = library.get("voice_missing").await.unwrap();
    assert!(error.is_none());
    assert!(library.delete("voice_missing").await.is_err());
}
