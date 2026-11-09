//! The desktop shell of the local AI manager.
//!
//! The window hosts the TTS subsystem: the model hub, the voice library and
//! the voice design studio. The backend runs as a sidecar that this process
//! starts and stops.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

pub mod server;

use serde::Serialize;
use tauri::Manager;

/// Where a drag of the window starts.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct DragOrigin {
    /// The position of the window, in physical pixels.
    pub x: i32,
    /// The position of the window, in physical pixels.
    pub y: i32,
    /// The ratio between the physical pixels and the points of the screen.
    pub scale: f64,
}

/// Remembers where the window is, so that a drag can move it.
///
/// macOS does not let the webview hand the drag over to the window: the
/// native drag needs the mouse-down event itself, and by the time the
/// request of the interface reaches the Rust side the event is gone
/// (tauri#9503). The window follows the mouse on its own instead.
#[tauri::command]
fn begin_drag(window: tauri::Window) -> Result<DragOrigin, String> {
    let position = window.outer_position().map_err(|err| err.to_string())?;
    let scale = window.scale_factor().map_err(|err| err.to_string())?;
    Ok(DragOrigin {
        x: position.x,
        y: position.y,
        scale,
    })
}

/// Moves the window to a place, in physical pixels.
#[tauri::command]
fn drag_window_to(window: tauri::Window, x: i32, y: i32) -> Result<(), String> {
    window
        .set_position(tauri::PhysicalPosition::new(x, y))
        .map_err(|err| err.to_string())
}

/// Zooms the window, or puts it back, when the bar is clicked twice.
///
/// The window is told to do it itself, because the system only zooms a
/// window when the title bar of the system is clicked twice — and that bar
/// is not there.
#[tauri::command]
fn toggle_zoom(window: tauri::Window) -> Result<(), String> {
    if window.is_maximized().map_err(|err| err.to_string())? {
        window.unmaximize().map_err(|err| err.to_string())
    } else {
        window.maximize().map_err(|err| err.to_string())
    }
}

/// Starts the desktop application.
pub fn run() {
    let app = tauri::Builder::default()
        .manage(server::ServerState::default())
        .invoke_handler(tauri::generate_handler![
            server::start_server,
            server::stop_server,
            server::api_base,
            begin_drag,
            drag_window_to,
            toggle_zoom,
        ])
        .build(tauri::generate_context!())
        .expect("failed to build the desktop application");

    app.run(|handle, event| {
        // The backend dies together with the application.
        if let tauri::RunEvent::Exit = event {
            if let Some(state) = handle.try_state::<server::ServerState>() {
                // The backend stops by itself and takes the engine that
                // holds the model with it.
                server::stop(&state);
            }
        }
    });
}
