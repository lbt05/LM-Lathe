//! Management of the `lm-lathe-server` backend.
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
    /// Held while the backend starts, so that two callers do not start two
    /// backends.
    pub startup: Mutex<()>,
}

/// The name of the backend binary.
const BINARY: &str = "lm-lathe-server";

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
    // Packaged application: the sidecar that Tauri bundled. Depending on the
    // platform it sits next to the executable or inside the resource
    // directory, with or without the target triple in its name.
    let mut packaged: Vec<PathBuf> = Vec::new();
    let triple = format!("{}-{}", std::env::consts::ARCH, platform_triple());
    if let Ok(directory) = app.path().resource_dir() {
        packaged.push(
            directory
                .join("binaries")
                .join(format!("{BINARY}-{triple}")),
        );
        packaged.push(directory.join("binaries").join(BINARY));
        packaged.push(directory.join(format!("{BINARY}-{triple}")));
        packaged.push(directory.join(BINARY));
        if let Some(parent) = directory.parent() {
            packaged.push(parent.join("MacOS").join(BINARY));
        }
    }
    if let Ok(executable) = std::env::current_exe() {
        if let Some(directory) = executable.parent() {
            packaged.push(directory.join(BINARY));
            packaged.push(directory.join(format!("{BINARY}-{triple}")));
        }
    }

    // An installed application uses the backend it carries; a development
    // build uses the newest one of the workspace.
    let mut candidates: Vec<PathBuf> = Vec::new();
    if packaged.iter().any(|candidate| candidate.exists()) {
        candidates.extend(packaged);
        candidates.extend(development);
    } else {
        candidates.extend(development);
        candidates.extend(packaged);
    }

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

/// Finds a tool that travels with the application.
///
/// The tools are sidecars: they sit next to the executable of the
/// application, and depending on the platform also in the resource
/// directory.
fn bundled_tool(app: &AppHandle, name: &str) -> Option<PathBuf> {
    let triple = format!("{}-{}", std::env::consts::ARCH, platform_triple());
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(directory) = app.path().resource_dir() {
        candidates.push(directory.join("binaries").join(format!("{name}-{triple}")));
        candidates.push(directory.join("binaries").join(name));
        candidates.push(directory.join(format!("{name}-{triple}")));
        candidates.push(directory.join(name));
        if let Some(parent) = directory.parent() {
            candidates.push(parent.join("MacOS").join(name));
        }
    }
    if let Ok(executable) = std::env::current_exe() {
        if let Some(directory) = executable.parent() {
            candidates.push(directory.join(name));
            candidates.push(directory.join(format!("{name}-{triple}")));
        }
    }
    candidates.into_iter().find(|candidate| candidate.exists())
}

/// The directory that holds the tools of the application.
///
/// It is put in front of the `PATH` of the backend, so that the tools that
/// travel with the application win over the ones of the system.
fn tool_directory() -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?;
    let directory = executable.parent()?.to_path_buf();
    if directory.join("uv").exists() {
        return Some(directory);
    }
    None
}

/// Picks a free TCP port.
/// The port the backend listens on.
///
/// The port is stable, so the URL of the local gateway does not change with
/// every restart. `LMLATHE_PORT` overrides it.
const DEFAULT_PORT: u16 = 54321;

/// The number of ports the application tries before it gives up.
const PORT_RANGE: u16 = 64;

/// The time the backend gets to stop by itself.
const STOP_GRACE: Duration = Duration::from_secs(20);

/// What the application does with a port.
enum PortChoice {
    /// The port is free, the backend is started there.
    Start(u16),
    /// The port already serves the build the application expects.
    Reuse(u16),
}

/// The port the application wants to use.
fn preferred_port() -> u16 {
    std::env::var("LMLATHE_PORT")
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .filter(|port| *port > 0)
        .unwrap_or(DEFAULT_PORT)
}

/// The fingerprint of a binary: its path and the time it was written.
fn build_fingerprint(path: &std::path::Path) -> String {
    let Ok(meta) = std::fs::metadata(path) else {
        return "unknown".to_string();
    };
    let modified = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|since| since.as_secs())
        .unwrap_or_default();
    format!("{}:{}", path.display(), modified)
}

/// Asks a running backend for its build fingerprint and its address.
///
/// The request is a plain HTTP/1.1 GET, the shell has no HTTP client.
fn probe_health(port: u16) -> Option<(String, String)> {
    use std::io::{Read, Write};

    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream.set_read_timeout(Some(Duration::from_millis(500))).ok()?;
    stream
        .write_all(b"GET /health HTTP/1.0\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .ok()?;
    let mut answer = String::new();
    stream.read_to_string(&mut answer).ok()?;
    Some((field(&answer, "build")?, field(&answer, "bind").unwrap_or_default()))
}

/// Reads one string field of a JSON answer.
fn field(answer: &str, name: &str) -> Option<String> {
    let start = answer.find(&format!("\"{name}\""))?;
    let rest = &answer[start + name.len() + 2..];
    let start = rest.find(':')? + 1;
    let rest = &rest[start..];
    let start = rest.find('"')? + 1;
    let end = rest[start..].find('"')? + start;
    Some(rest[start..end].to_string())
}

/// Picks the port for the backend.
///
/// The preferred port is used when it is free. When it is taken, the running
/// server is asked for its build: if it is the build the application expects
/// and it listens on the address the user chose, the application reuses it,
/// otherwise the next port is tried.
fn select_port(preferred: u16, fingerprint: &str, bind: &str) -> Result<PortChoice, String> {
    for offset in 0..PORT_RANGE {
        let port = preferred.saturating_add(offset);
        if port == 0 {
            break;
        }
        match std::net::TcpListener::bind(("127.0.0.1", port)) {
            Ok(listener) => {
                drop(listener);
                return Ok(PortChoice::Start(port));
            }
            Err(_) => {
                if let Some((build, address)) = probe_health(port) {
                    if build == fingerprint && (bind == "0.0.0.0" || address == bind) {
                        return Ok(PortChoice::Reuse(port));
                    }
                }
            }
        }
    }
    Err(format!(
        "no free port for the backend, {preferred} and the ports after it are taken"
    ))
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
    if let Ok(path) = std::env::var("LM_LATHE_HOME") {
        return Some(PathBuf::from(path));
    }
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".lm-lathe"))
}

/// Starts the backend and returns its base URL.
///
/// The address the backend listens on comes from the settings of the
/// application: `127.0.0.1` keeps the service inside the machine,
/// `0.0.0.0` lets the other machines of the network reach it.
#[tauri::command]
pub async fn start_server(
    app: AppHandle,
    state: State<'_, ServerState>,
    host: Option<String>,
) -> Result<String, String> {
    // Only one backend may start at a time, the window can ask twice.
    let _startup = state.startup.lock().unwrap();

    // The server is already running.
    if let Some(base) = state.base.lock().unwrap().clone() {
        return Ok(base);
    }

    let bind = match host.as_deref() {
        Some("0.0.0.0") => "0.0.0.0".to_string(),
        _ => "127.0.0.1".to_string(),
    };

    let binary = server_binary(&app)?;
    let fingerprint = build_fingerprint(&binary);
    let (port, reuse) = match select_port(preferred_port(), &fingerprint, &bind)? {
        PortChoice::Start(port) => (port, false),
        PortChoice::Reuse(port) => (port, true),
    };
    let base = format!("http://127.0.0.1:{port}");

    // The port already serves the build the application expects: the running
    // server is used, the application does not start a second one.
    if reuse {
        if let Ok(mut slot) = state.base.lock() {
            *slot = Some(base.clone());
        }
        return Ok(base);
    }

    let mut command = Command::new(&binary);
    command
        .arg("serve")
        .arg("--host")
        .arg(&bind)
        .arg("--port")
        .arg(port.to_string())
        .stdin(Stdio::piped());

    // The backend leads a group of its own, so that the engines it starts
    // can be reached together with it.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    // The backend follows the life cycle of this process.
    command.env("TTS_SERVER_EXIT_ON_STDIN_EOF", "1");

    // The tools that travel with the application are used first: the user
    // does not have to install anything before an engine can be installed.
    if let Some(uv) = bundled_tool(&app, "uv") {
        command.env("LM_LATHE_UV", &uv);
    }
    if let Some(directory) = tool_directory() {
        let path = std::env::var("PATH").unwrap_or_default();
        let path = if path.is_empty() {
            directory.display().to_string()
        } else {
            format!("{}:{}", directory.display(), path)
        };
        command.env("PATH", path);
    }

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
        command.env("LM_LATHE_HOME", directory);
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
///
/// The pipe is closed first: the backend sees the end of it, stops by itself
/// and takes the engine that holds the model with it. Only when the backend
/// does not go away by itself, it is killed.
pub fn stop(state: &ServerState) {
    let child = state.child.lock().unwrap().take();
    // Closing the pipe asks the backend to stop.
    let _ = state.stdin.lock().unwrap().take();
    *state.base.lock().unwrap() = None;

    let Some(mut child) = child else {
        return;
    };

    // The backend stops by itself when the pipe reaches its end. On the way
    // out it stops the engine that holds the model.
    let deadline = Instant::now() + STOP_GRACE;
    while Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(_) => break,
        }
    }

    // It did not go away: the whole group is killed, so that no engine is
    // left behind.
    terminate(&mut child);
}

/// Kills the backend and the engines it started.
fn terminate(child: &mut Child) {
    #[cfg(unix)]
    {
        // The backend leads the group, the engines it started are in it.
        let group = -(child.id() as i32);
        unsafe {
            libc::kill(group, libc::SIGTERM);
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            match child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) => std::thread::sleep(Duration::from_millis(100)),
                Err(_) => break,
            }
        }
        unsafe {
            libc::kill(group, libc::SIGKILL);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
    }
    let _ = child.wait();
}

/// Stops the backend.
#[tauri::command]
pub fn stop_server(state: State<'_, ServerState>) -> Result<(), String> {
    stop(&state);
    Ok(())
}

/// The base URL of the running backend.
#[tauri::command]
pub fn api_base(state: State<'_, ServerState>) -> Option<String> {
    state.base.lock().unwrap().clone()
}
