//! Bootstrap (first-run / every-run preparation) orchestrator.
//!
//! Stages: paths → data dirs → secret → runtimes → python env → ready.
//! Every stage is idempotent; a working environment is detected and reused.

use crate::paths::{append_log_line, resolve_roots};
use crate::runtime;
use crate::state::{AiState, Event, SharedState, Stage};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::Manager;

/// Guards concurrent AI installations.
pub static AI_RUNNING: AtomicBool = AtomicBool::new(false);

fn fail(state: &Mutex<SharedState>, msg: String, detail: Option<String>) {
    let guard = state.lock().unwrap();
    guard.failed.store(true, Ordering::SeqCst);
    let mut ev = Event::new(Stage::Failed, msg).error(true);
    if let Some(d) = detail {
        ev = ev.detail(d);
    }
    guard.emit(&ev);
}

/// Run the whole bootstrap on a worker thread; emits vira://event messages.
pub fn run_bootstrap(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        // Initialize the log file (best effort) before anything else.
        let _ = crate::paths::init_log_file(&crate::paths::vira_home_dir().join("logs"));
        append_log_line("── Vira launching ──");
        let state = app.state::<Mutex<SharedState>>();
        {
            let guard = state.lock().unwrap();
            if !guard.begin_setup() {
                return; // already running
            }
            guard.failed.store(false, Ordering::SeqCst);
            guard.emit(&Event::new(Stage::Init, "Starting Vira…").progress(1));
        }

        let result = (|| -> Result<(), String> {
            // ── Stage: locate payload ──────────────────────────────────────
            {
                let guard = state.lock().unwrap();
                guard.emit(&Event::new(Stage::Paths, "Locating application files"));
            }
            let exe_dir = std::env::current_exe()
                .map_err(|e| format!("cannot resolve executable path: {e}"))?
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| std::env::temp_dir());
            let resource_dir = app.path().resource_dir().ok();
            let payload = crate::paths::find_payload(&exe_dir, resource_dir.as_deref())
                .ok_or_else(|| {
                    "Application files not found. The installation appears to be damaged — please reinstall Vira.".to_string()
                })?;
            append_log_line(&format!("payload resolved: {}", payload.display()));

            let roots = resolve_roots(payload);
            {
                let guard = state.lock().unwrap();
                *guard.payload_root.lock().unwrap() = Some(roots.payload.clone());
                *guard.site_packages.lock().unwrap() = Some(roots.site_packages.clone());
                guard
                    .emit(&Event::new(Stage::Paths, "Application files found").progress(10));
            }

            // ── Stage: user data dirs ──────────────────────────────────────
            if let Err(e) = std::fs::create_dir_all(&roots.data) {
                return Err(format!("Cannot create data folder {}: {e}", roots.data.display()));
            }
            if let Err(e) = std::fs::create_dir_all(&roots.site_packages) {
                return Err(format!("Cannot create {}: {e}", roots.site_packages.display()));
            }
            let _ = std::fs::create_dir_all(&roots.logs);
            {
                let guard = state.lock().unwrap();
                *guard.data_dir.lock().unwrap() = Some(roots.data.clone());
                *guard.vira_home.lock().unwrap() = Some(roots.vira_home.clone());
                *guard.logs_dir.lock().unwrap() = Some(roots.logs.clone());
                guard.emit(&Event::new(Stage::Data, "Data folder ready").progress(20));
            }
            append_log_line(&format!("data dir: {}", roots.data.display()));

            // ── Stage: secret key ──────────────────────────────────────────
            let secret = crate::paths::resolve_secret_key(&roots.data);
            {
                let guard = state.lock().unwrap();
                *guard.secret_key.lock().unwrap() = secret.clone();
                guard.emit(&Event::new(Stage::Secrets, "Settings stored").progress(30));
            }

            // ── Stage: runtimes ────────────────────────────────────────────
            {
                let guard = state.lock().unwrap();
                guard.emit(&Event::new(Stage::Runtimes, "Checking bundled runtimes").progress(40));
            }
            if let Err(e) = runtime::verify_runtimes(&state.lock().unwrap(), &roots) {
                return Err(format!("Bundled runtime check failed: {e}"));
            }

            // ── Stage: python deps ─────────────────────────────────────────
            {
                let guard = state.lock().unwrap();
                guard.emit(&Event::new(Stage::PythonEnv, "Checking Python libraries").progress(50));
            }
            if let Err(e) = runtime::ensure_python_env(&state.lock().unwrap(), &roots) {
                return Err(format!("Python environment setup failed: {e}"));
            }

            // ── Ready ──────────────────────────────────────────────────────
            {
                let guard = state.lock().unwrap();
                *guard.ai_state.lock().unwrap() = AiState::NotInstalled;
                if runtime::ai_installed(&roots.site_packages) {
                    *guard.ai_state.lock().unwrap() = AiState::Installed;
                }
                guard.emit(&Event::new(Stage::Ready, "Ready — Press Start").progress(100));
            }
            Ok(())
        })();

        let guard = state.lock().unwrap();
        guard.end_setup();
        drop(guard);

        if let Err(e) = result {
            fail(
                &state,
                "Vira could not finish preparing".into(),
                Some(format!("{e}\n\nTip: open the logs folder for details.")),
            );
        }
    });
}
