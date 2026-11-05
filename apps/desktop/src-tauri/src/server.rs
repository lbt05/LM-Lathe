//! Management of the `tts-server` backend.
//!
//! The desktop application owns the lifecycle of the backend: it starts the
//! server on a free port when the window opens and stops it when the
//! application quits. The frontend then talks to the server over HTTP.

use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager, State};

/// The state of the backend process.
#[derive(Default)]
pub struct ServerState {
    /// The backend process.
    pub child: Mutex<Option<Child>>,
    /// The write end of the pipe that keeps the backend alive. The backend
    /// stops when it reaches the end of the pipe, which happens as soon as
    /// this process is gone.
    pub stdin: Mutex<Option<ChildStdin>>,
    /// The base URL of the backend.
    pub base: Mutex<Option<String>>,
}

/// The name of the backend binary.
const BINARY: &str = "tts-server";

/// Finds the backend binary.
///
/// The order is: the `TTS_SERVER_BIN` environment variable, the binary that
/// the workspace produced and finally the sidecar that Tauri bundled.
fn server_binary(app: &AppHandle) -> Result<PathBuf, String> {
    if let Ok(path) = std::env::var("TTS_SERVER_BIN") {
        let path = PathBuf::from(path);
        if path.exists() {
            return Ok(path);
        }
        return Err(format!(
            "TTS_SERVER_BIN points to {}, which does not exist",
            path.display()
        ));
    }

    // Development: the workspace target directory. Several copies can exist —
    // the one the backend workspace produced and the one Tauri copied next to
    // the application — so the newest one wins.
    let mut development: Vec<PathBuf> = Vec::new();
    if let Ok(current) = std::env::current_exe() {
        let mut root = current.as_path();
        while let Some(parent) = root.parent() {
            for profile in ["release", "debug"] {
                development.push(parent.join("target").join(profile).join(BINARY));
                development.push(parent.join(profile).join(BINARY));
            }
            root = parent;
        }
    }
    development.sort_by_key(|candidate| {
        std::fs::metadata(candidate)
            .and_then(|meta| meta.modified())
            .ok()
    });
    development.reverse();
    development.retain(|candidate| candidate.exists());
    let mut candidates: Vec<PathBuf> = development;

    // Packaged application: the sidecar that Tauri bundled. Depending on the
    // platform it sits next to the executable or inside the resource
    // directory, with or without the target triple in its name.
    let triple = format!("{}-{}", std::env::consts::ARCH, platform_triple());
    if let Ok(directory) = app.path().resource_dir() {
        candidates.push(
            directory
                .join("binaries")
                .join(format!("{BINARY}-{triple}")),
        );
        candidates.push(directory.join("binaries").join(BINARY));
        candidates.push(directory.join(format!("{BINARY}-{triple}")));
        candidates.push(directory.join(BINARY));
        if let Some(parent) = directory.parent() {
            candidates.push(parent.join("MacOS").join(BINARY));
        }
    }
    if let Ok(executable) = std::env::current_exe() {
        if let Some(directory) = executable.parent() {
            candidates.push(directory.join(BINARY));
            candidates.push(directory.join(format!("{BINARY}-{triple}")));
        }
    }

    // The newest copy wins: the backend workspace and the desktop directory
    // can both hold a build, and only one of them is current.
    for candidate in candidates {
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    Err(format!(
        "the `{BINARY}` binary was not found — build the workspace or set TTS_SERVER_BIN"
    ))
}

/// The operating system part of the Rust target triple.
fn platform_triple() -> &'static str {
    match std::env::consts::OS {
        "macos" => "apple-darwin",
        "windows" => "windows-msvc",
        "linux" => "unknown-linux-gnu",
        other => other,
    }
}

/// Picks a free TCP port.
fn free_port() -> Result<u16, String> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|err| format!("failed to reserve a port: {err}"))?;
    let port = listener
        .local_addr()
        .map_err(|err| format!("failed to read the port: {err}"))?
        .port();
    drop(listener);
    Ok(port)
}

/// Waits until the backend accepts connections.
fn wait_for_port(port: u16, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(120));
    }
    Err("the backend did not start in time".to_string())
}

/// The directory that holds the models, voices and generated audio.
fn data_directory() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("LOCAL_AI_MANAGER_HOME") {
        return Some(PathBuf::from(path));
    }
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".local-ai-manager"))
}

/// Starts the backend and returns its base URL.
#[tauri::command]
pub async fn start_server(app: AppHandle, state: State<'_, ServerState>) -> Result<String, String> {
    // The server is already running.
    if let Some(base) = state.base.lock().unwrap().clone() {
        return Ok(base);
    }

    let binary = server_binary(&app)?;
    let port = free_port()?;
    let base = format!("http://127.0.0.1:{port}");

    let mut command = Command::new(&binary);
    command
        .arg("serve")
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .stdin(Stdio::piped());

    // The backend follows the life cycle of this process.
    command.env("TTS_SERVER_EXIT_ON_STDIN_EOF", "1");

    // The output of the backend ends up in the log directory, so the user can
    // see what the server does.
    let directory = data_directory();
    if let Some(directory) = &directory {
        let logs = directory.join("logs").join("tts");
        if std::fs::create_dir_all(&logs).is_ok() {
            let path = logs.join("desktop-server.log");
            if let Ok(file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
            {
                if let Ok(errors) = file.try_clone() {
                    command
                        .stdout(Stdio::from(file))
                        .stderr(Stdio::from(errors));
                }
            }
        }
    }
    if directory.is_none() {
        command.stdout(Stdio::null()).stderr(Stdio::null());
    }

    if let Some(directory) = directory {
        command.env("LOCAL_AI_MANAGER_HOME", directory);
    }

    let mut child = command
        .spawn()
        .map_err(|err| format!("failed to start {}: {err}", binary.display()))?;

    // The write end of the pipe stays open as long as this process lives.
    let stdin = child.stdin.take();

    if let Err(err) = wait_for_port(port, Duration::from_secs(30)) {
        let mut child = child;
        let _ = child.kill();
        return Err(err);
    }

    *state.stdin.lock().unwrap() = stdin;
    *state.child.lock().unwrap() = Some(child);
    *state.base.lock().unwrap() = Some(base.clone());
    Ok(base)
}

/// Stops the backend.
#[tauri::command]
pub fn stop_server(state: State<'_, ServerState>) -> Result<(), String> {
    let child = state.child.lock().unwrap().take();
    // Closing the pipe asks the backend to stop.
    let _ = state.stdin.lock().unwrap().take();
    if let Some(mut child) = child {
        let _ = child.kill();
        let _ = child.wait();
    }
    *state.base.lock().unwrap() = None;
    Ok(())
}

/// The base URL of the running backend.
#[tauri::command]
pub fn api_base(state: State<'_, ServerState>) -> Option<String> {
    state.base.lock().unwrap().clone()
}
