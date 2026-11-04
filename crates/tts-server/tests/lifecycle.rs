//! Tests of the life cycle of the server process.

use std::process::{Command, Stdio};
use std::time::Duration;

/// Waits until the server answers.
async fn wait_for_health(port: u16) -> bool {
    let client = reqwest::Client::new();
    for _ in 0..100 {
        if client
            .get(format!("http://127.0.0.1:{port}/health"))
            .send()
            .await
            .is_ok()
        {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

#[tokio::test]
async fn the_server_stops_when_the_parent_closes_the_pipe() {
    // The desktop application holds the write end of the pipe; when it dies
    // the server sees the end and stops.
    let mut child = Command::new(env!("CARGO_BIN_EXE_tts-server"))
        .args(["serve", "--host", "127.0.0.1", "--port", "8091"])
        .env("TTS_SERVER_EXIT_ON_STDIN_EOF", "1")
        .env("LOCAL_AI_MANAGER_HOME", "/tmp/lm-lathe-lifecycle-test")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to start the server");

    assert!(wait_for_health(8091).await, "the server did not start");

    // Closing the pipe is what the desktop application does when it exits.
    drop(child.stdin.take());

    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        match child.try_wait().expect("failed to wait for the server") {
            Some(status) => {
                assert!(status.success(), "the server exited with {status}");
                break;
            }
            None => {
                if std::time::Instant::now() > deadline {
                    let _ = child.kill();
                    panic!("the server did not stop after the pipe was closed");
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

#[tokio::test]
async fn the_server_keeps_running_without_the_pipe_watch() {
    // Without the environment variable the server ignores the standard
    // input, so a redirected stdin does not stop it.
    let mut child = Command::new(env!("CARGO_BIN_EXE_tts-server"))
        .args(["serve", "--host", "127.0.0.1", "--port", "8092"])
        .env("LOCAL_AI_MANAGER_HOME", "/tmp/lm-lathe-lifecycle-test")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to start the server");

    assert!(wait_for_health(8092).await, "the server did not start");
    assert!(
        child.try_wait().expect("failed to wait").is_none(),
        "the server stopped although nobody asked it to"
    );

    let _ = child.kill();
    let _ = child.wait();
}
