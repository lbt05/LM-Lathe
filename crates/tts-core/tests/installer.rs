//! Tests of the checkpoint download.

mod common;

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

use common::TempDir;
use tts_core::installer::{InstallTracker, ModelInstaller};
use tts_core::registry::TTSModelSpec;

/// Starts a tiny HTTP server that mimics the model hub.
///
/// It answers the repository tree and the file downloads of one repository.
fn fake_hub() -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("failed to bind the fake hub");
    let address = listener.local_addr().expect("failed to read the address");
    let handle = thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = match stream {
                Ok(stream) => stream,
                Err(_) => break,
            };
            let mut buffer = [0u8; 4096];
            let read = stream.read(&mut buffer).unwrap_or(0);
            let request = String::from_utf8_lossy(&buffer[..read]).to_string();
            let target = request.split_whitespace().nth(1).unwrap_or("/").to_string();
            let range = request
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    if name.trim().eq_ignore_ascii_case("range") {
                        Some(value.trim().to_string())
                    } else {
                        None
                    }
                });

            let (status, content_type, body) = if target.contains("does/not-exist") {
                ("404 Not Found", "text/plain", "missing".to_string())
            } else if target.contains("/tree/main") {
                (
                    "200 OK",
                    "application/json",
                    r#"[{"type":"file","path":"config.json","size":7},{"type":"file","path":"model.safetensors","size":5},{"type":"file","path":"demo.mp4","size":11},{"type":"directory","path":"extra"}]"#.to_string(),
                )
            } else if target.ends_with("/config.json") {
                ("200 OK", "application/json", "{\"a\":1}".to_string())
            } else if target.ends_with("/model.safetensors") {
                // The hub continues a download when the client asks for a
                // range.
                let body = "hello";
                match range.as_deref().and_then(|value| value.strip_prefix("bytes=")) {
                    Some(start) => {
                        let start: usize = start.trim_end_matches('-').parse().unwrap_or(0);
                        (
                            "206 Partial Content",
                            "application/octet-stream",
                            body[start.min(body.len())..].to_string(),
                        )
                    }
                    None => ("200 OK", "application/octet-stream", body.to_string()),
                }
            } else {
                ("404 Not Found", "text/plain", "missing".to_string())
            };

            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });

    (format!("http://{address}"), handle)
}

fn spec() -> TTSModelSpec {
    TTSModelSpec {
        id: "qwen3-tts-1.7b-voicedesign".to_string(),
        name: "Qwen3-TTS 1.7B VoiceDesign".to_string(),
        family: tts_core::types::ModelFamily::Qwen3Tts,
        engine_id: "sglang-omni".to_string(),
        hf_repo: "Qwen/Qwen3-TTS-12Hz-1.7B-VoiceDesign".to_string(),
        version: "1.7B".to_string(),
        format: "safetensors".to_string(),
        size_bytes: 3_833_402_552,
        estimated_memory_bytes: 8 * 1024 * 1024 * 1024,
        features: vec![tts_core::types::TtsFeature::TextToSpeech],
        hardware: Default::default(),
        mode: tts_core::types::EngineMode::SglangOmni,
        default_ttl_secs: None,
        aliases: vec![],
        launch_args: vec![],
        local_path: None,
        notes: None,
    }
}

#[tokio::test]
async fn the_checkpoint_is_downloaded_into_the_model_directory() {
    let dir = TempDir::new("installer");
    let (endpoint, _hub) = fake_hub();
    let installer = ModelInstaller::new(dir.path().join("models/tts")).with_endpoint(&endpoint);
    let tracker = InstallTracker::default();

    assert!(!installer.is_installed(&spec()));

    tracker.start("qwen3-tts-1.7b-voicedesign");
    let report = installer
        .install(&spec(), Some(&tracker))
        .await
        .expect("the installation failed");

    assert_eq!(report.downloaded.len(), 2);
    assert!(report.skipped.is_empty());
    assert_eq!(report.bytes, 12);

    let target = installer.target_dir(&spec());
    assert_eq!(
        target,
        dir.path()
            .join("models/tts")
            .join("Qwen--Qwen3-TTS-12Hz-1.7B-VoiceDesign")
    );
    assert_eq!(
        std::fs::read_to_string(target.join("config.json")).unwrap(),
        "{\"a\":1}"
    );
    assert_eq!(
        std::fs::read_to_string(target.join("model.safetensors")).unwrap(),
        "hello"
    );
    // Directories are not files and are therefore not downloaded.
    assert!(!target.join("extra").exists());
    // Demonstration material is skipped.
    assert!(!target.join("demo.mp4").exists());

    assert!(installer.is_installed(&spec()));
    assert_eq!(installer.size_on_disk(&spec()), 12);

    let progress = tracker.get("qwen3-tts-1.7b-voicedesign").unwrap();
    assert_eq!(progress.files_done, 2);
    assert_eq!(progress.files_total, 2);
    assert_eq!(progress.bytes, 12);
}

#[tokio::test]
async fn an_existing_checkpoint_is_not_downloaded_again() {
    let dir = TempDir::new("installer");
    let (endpoint, _hub) = fake_hub();
    let installer = ModelInstaller::new(dir.path().join("models/tts")).with_endpoint(&endpoint);

    installer.install(&spec(), None).await.unwrap();
    let second = installer.install(&spec(), None).await.unwrap();

    assert!(second.downloaded.is_empty());
    assert_eq!(second.skipped.len(), 2);
    assert_eq!(second.bytes, 0);
}

#[tokio::test]
async fn the_progress_is_measured_in_bytes() {
    let dir = TempDir::new("installer");
    let (endpoint, _hub) = fake_hub();
    let installer = ModelInstaller::new(dir.path().join("models/tts")).with_endpoint(&endpoint);
    let tracker = InstallTracker::default();

    tracker.start("qwen3-tts-1.7b-voicedesign");
    installer
        .install(&spec(), Some(&tracker))
        .await
        .expect("the installation failed");

    let progress = tracker.get("qwen3-tts-1.7b-voicedesign").unwrap();
    // The demo video is skipped, so the two remaining files count.
    assert_eq!(progress.files_total, 2);
    assert_eq!(progress.files_done, 2);
    // `config.json` holds 7 bytes, `model.safetensors` 5.
    assert_eq!(progress.bytes_total, 12);
    assert_eq!(progress.bytes, 12);
}

#[tokio::test]
async fn an_interrupted_download_is_continued() {
    let dir = TempDir::new("installer");
    let (endpoint, _hub) = fake_hub();
    let installer = ModelInstaller::new(dir.path().join("models/tts")).with_endpoint(&endpoint);

    installer.install(&spec(), None).await.unwrap();
    let target = installer.target_dir(&spec());

    // The download of the weights stopped after three bytes.
    std::fs::remove_file(target.join("model.safetensors")).unwrap();
    std::fs::write(target.join("model.safetensors.part"), b"hel").unwrap();

    installer.install(&spec(), None).await.unwrap();

    // The file was continued and not downloaded a second time.
    assert_eq!(
        std::fs::read(target.join("model.safetensors")).unwrap(),
        b"hello"
    );
    assert!(!target.join("model.safetensors.part").exists());
}

#[tokio::test]
async fn the_checkpoint_can_be_removed_again() {
    let dir = TempDir::new("installer");
    let (endpoint, _hub) = fake_hub();
    let installer = ModelInstaller::new(dir.path().join("models/tts")).with_endpoint(&endpoint);

    installer.install(&spec(), None).await.unwrap();
    assert!(installer.is_installed(&spec()));

    installer.uninstall(&spec()).unwrap();
    assert!(!installer.is_installed(&spec()));
}

#[tokio::test]
async fn an_unknown_repository_is_reported() {
    let dir = TempDir::new("installer");
    let (endpoint, _hub) = fake_hub();
    let installer = ModelInstaller::new(dir.path().join("models/tts")).with_endpoint(&endpoint);

    let mut unknown = spec();
    unknown.hf_repo = "does/not-exist".to_string();
    let error = installer.install(&unknown, None).await.unwrap_err();
    assert!(error.to_string().contains("404"));
}
