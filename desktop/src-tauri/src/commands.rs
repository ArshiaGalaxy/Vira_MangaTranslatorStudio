//! Tauri command surface exposed to the launcher UI.

use crate::ai;
use crate::services;
use crate::setup;
use crate::state::{AiState, Event, SharedState, Snapshot, Stage};
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use tauri::Manager;

#[tauri::command]
pub fn get_snapshot(state: tauri::State<'_, Mutex<SharedState>>) -> Snapshot {
    SharedState::snapshot(&state)
}

#[tauri::command]
pub fn retry_setup(app: tauri::AppHandle) {
    // Reset visible failure state, then rerun the bootstrap pipeline.
    let state = app.state::<Mutex<SharedState>>();
    let guard = state.lock().unwrap();
    guard.failed.store(false, Ordering::SeqCst);
    guard.emit(&Event::new(Stage::Init, "Retrying setup…"));
    drop(guard);
    setup::run_bootstrap(app);
}

#[tauri::command]
pub fn start_studio(app: tauri::AppHandle) {
    services::run_start(app);
}

#[tauri::command]
pub fn install_ai(app: tauri::AppHandle) {
    ai::install_async(app);
}

#[tauri::command]
pub fn ai_state(state: tauri::State<'_, Mutex<SharedState>>) -> AiState {
    let guard = state.lock().unwrap();
    let current = *guard.ai_state.lock().unwrap();
    current
}

#[tauri::command]
pub fn open_logs(state: tauri::State<'_, Mutex<SharedState>>) {
    let guard = state.lock().unwrap();
    let logs_dir = guard.logs_dir.lock().unwrap().clone();
    if let Some(dir) = logs_dir {
        let _ = std::fs::create_dir_all(&dir);
        // The user asked for the logs folder: opening an Explorer window on
        // their own directory is harmless and admin-free.
        let _ = std::process::Command::new("explorer")
            .arg(dir.as_os_str())
            .status();
    }
}

/// Open an https link from the launcher footer in the user's default browser.
/// Restricted to plain https URLs so the command can never be used to launch
/// file:// or custom-protocol targets.
#[tauri::command]
pub fn open_url(url: String) {
    let url = url.trim();
    if !(url.starts_with("https://") && !url.contains('"') && !url.contains('\\')) {
        return;
    }
    let _ = std::process::Command::new("rundll32")
        .arg("url.dll,FileProtocolHandler")
        .arg(url)
        .output();
}
