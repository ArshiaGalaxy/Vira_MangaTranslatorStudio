//! Vira Translator Studio — desktop launcher entry point.
//!
//! Runs the bootstrap logic on a worker thread, reports progress to the
//! launcher UI via the `vira://event` channel, and manages the Django /
//! Next.js child processes for the lifetime of the window.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ai;
mod commands;
mod health;
mod jobobj;
mod paths;
mod ports;
mod proc;
mod runtime;
mod services;
mod setup;
mod state;

use std::sync::Mutex;
use tauri::Manager;
use state::SharedState;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // A second instance was launched: focus the existing window.
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.set_focus();
            }
        }))
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshot,
            commands::start_studio,
            commands::retry_setup,
            commands::install_ai,
            commands::open_logs,
        ])
        .setup(|app| {
            // Job object: all child processes Vira spawns are reaped if Vira dies.
            jobobj::init_vira_job();
            app.manage(Mutex::new(SharedState::bootstrap(app.handle().clone())));
            setup::run_bootstrap(app.handle().clone());
            Ok(())
        })
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::CloseRequested { .. }) {
                let app = window.app_handle().clone();
                proc::cleanup_all(&app.state::<Mutex<SharedState>>());
            } else if let tauri::WindowEvent::Destroyed = event {
                let app = window.app_handle().clone();
                proc::cleanup_all(&app.state::<Mutex<SharedState>>());
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running Vira");
}
