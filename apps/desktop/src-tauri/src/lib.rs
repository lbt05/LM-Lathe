//! The desktop shell of the local AI manager.
//!
//! The window hosts the TTS subsystem: the model hub, the voice library and
//! the voice design studio. The backend runs as a sidecar that this process
//! starts and stops.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

pub mod server;

use tauri::Manager;

/// Starts the desktop application.
pub fn run() {
    let app = tauri::Builder::default()
        .manage(server::ServerState::default())
        .invoke_handler(tauri::generate_handler![
            server::start_server,
            server::stop_server,
            server::api_base,
        ])
        .build(tauri::generate_context!())
        .expect("failed to build the desktop application");

    app.run(|handle, event| {
        // The backend dies together with the application.
        if let tauri::RunEvent::Exit = event {
            if let Some(state) = handle.try_state::<server::ServerState>() {
                // Closing the pipe asks the backend to stop.
                let _ = state.stdin.lock().unwrap().take();
                let child = state.child.lock().unwrap().take();
                if let Some(mut child) = child {
                    let _ = child.kill();
                    let _ = child.wait();
                }
            }
        }
    });
}
