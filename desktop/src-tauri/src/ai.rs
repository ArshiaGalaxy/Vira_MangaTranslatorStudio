//! AI extras installer: fetches requirements-ai.txt (torch CPU, paddleocr, …)
//! on demand. This is the only stage that uses the network, shown with live
//! progress; it never touches existing user data or the core environment.

use crate::paths::append_log_line;
use crate::proc;
use crate::runtime::MARKER_AI;
use crate::state::{AiState, Event, SharedState, Stage};
use std::path::PathBuf;
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter, Manager};

/// Marker detection for "AI extras installed".
pub fn ai_marker_exists(site_packages: &PathBuf) -> bool {
    site_packages.join(MARKER_AI).is_file()
}

/// Run the AI installation in a worker thread that streams progress events.
pub fn install_async(app: AppHandle) {
    std::thread::spawn(move || {
        if crate::setup::AI_RUNNING
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            emit_ev(&app, Event::new(Stage::AiInstall, "AI installation already in progress"));
            return;
        }
        let outcome = run_install(&app);
        if let Err(msg) = outcome {
            fail(&app, msg);
        }
        crate::setup::AI_RUNNING.store(false, Ordering::SeqCst);
    });
}

fn run_install(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<std::sync::Mutex<SharedState>>();
    {
        let guard = state.lock().unwrap();
        if *guard.ai_state.lock().unwrap() == AiState::Installed {
            return Ok(());
        }
        guard.failed.store(false, Ordering::SeqCst);
        let _ = std::mem::replace(&mut *guard.stage.lock().unwrap(), Stage::AiInstall);
        let _ = std::mem::replace(&mut *guard.stage_message.lock().unwrap(), "Installing AI libraries…".into());
        let _ = std::mem::replace(&mut *guard.ai_state.lock().unwrap(), AiState::Installing);
        guard.emit(&Event::new(
            Stage::AiInstall,
            "Downloading AI libraries (one-time, honouring your connection speed)",
        )
        .progress(1));
    }

    let (payload, site_packages) = {
        let guard = state.lock().unwrap();
        // Bind inside the block so temporary guards drop before `guard`.
        let pair = (
            guard.payload_root.lock().unwrap().clone(),
            guard.site_packages.lock().unwrap().clone(),
        );
        pair
    };
    let Some(payload) = payload else { return Err("application payload not found".into()) };
    let Some(sp) = site_packages else { return Err("python environment not prepared".into()) };
    let py = payload.join("runtimes").join("python").join("python.exe");
    if !py.is_file() {
        return Err("bundled python runtime missing".into());
    }
    let req = payload.join("backend").join("requirements-ai.txt");
    if !req.is_file() {
        return Err(format!("requirements-ai.txt missing: {}", req.display()));
    }

    let args: Vec<String> = vec![
        "-m".into(),
        "pip".into(),
        "install".into(),
        "--target".into(),
        sp.to_string_lossy().into_owned(),
        "--no-warn-script-location".into(),
        "--extra-index-url".into(),
        "https://download.pytorch.org/whl/cpu".into(),
        "--progress-bar".into(),
        "on".into(),
        "-r".into(),
        req.to_string_lossy().into_owned(),
    ];

    let stopped = std::sync::Arc::new(AtomicBool::new(false));
    let stopped_for_lines = std::sync::Arc::clone(&stopped);
    let app_for_lines = app.clone();
    let spawn_result = proc::spawn_tracked(
        &proc::SpawnSpec {
            program: py,
            args,
            cwd: payload.clone(),
            env: vec![
                ("PYTHONDONTWRITEBYTECODE".into(), "1".into()),
                ("PYTHONNOUSERSITE".into(), "1".into()),
            ],
            label: "pip-ai".into(),
        },
        move |line| {
            if !stopped_for_lines.load(Ordering::Relaxed) {
                handle_pip_line(&app_for_lines, &line);
            }
        },
    );
    match spawn_result {
        Err(e) => Err(format!("Failed to start pip: {e}")),
        Ok(mut child) => {
            // Heartbeat progress (pip has no total progress we can trust).
            let app_hb = app.clone();
            let hb_stop = stopped.clone();
            let hb = std::thread::spawn(move || {
                for i in 0.. {
                    if hb_stop.load(Ordering::Relaxed) || i > 90 {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_secs(10));
                    emit_ev(&app_hb, Event::new(
                        Stage::AiInstall,
                        "Downloading large AI libraries (torch ~2 GB, paddle — please wait)",
                    ).progress(((i + 1) * 10u32).min(95) as u8));
                }
            });
            // Track the child for cleanup + job assignment happened in spawn.
            {
                let guard = state.lock().unwrap();
                track_spawned(&guard, &mut child, "pip-ai");
            }
            let status = child.wait();
            stopped.store(true, Ordering::Relaxed);
            let ok = matches!(&status, Ok(s) if s.success());
            let _ = hb.join().ok();
            if ok {
                append_log_line("AI extras installed successfully");
                std::fs::write(sp.join(MARKER_AI), "ok").map_err(|e| format!("cannot write marker: {e}"))?;
                let guard = state.lock().unwrap();
                *guard.ai_state.lock().unwrap() = AiState::Installed;
                guard.emit(&Event::new(Stage::Ready, "AI libraries installed"));
                Ok(())
            } else {
                Err("pip exited with an error — see the launcher log for details".into())
            }
        }
    }
}

fn handle_pip_line(app: &AppHandle, line: &str) {
    if line.is_empty() {
        return;
    }
    append_log_line(line);
    let text = line.trim();
    // Lines arrive prefixed like "[pip-ai] Collecting paddleocr==..."
    if let Some(rest) = text.strip_prefix("[pip-ai] ") {
        if let Some(name) = rest.strip_prefix("Collecting ") {
            emit_ev(app, Event::new(Stage::AiInstall, format!("Fetching {name}")));
        } else if rest.starts_with("Installing collected packages") {
            emit_ev(app, Event::new(Stage::AiInstall, "Installing packages…"));
        } else if let Some(s) = rest.strip_prefix("Successfully installed ") {
            emit_ev(app, Event::new(Stage::AiInstall, format!("Installed {s}")).progress(95));
        }
    }
}

fn emit_ev(app: &AppHandle, ev: Event) {
    let _ = app.emit("vira://event", ev);
}

fn fail(app: &AppHandle, msg: String) {
    let state = app.state::<std::sync::Mutex<SharedState>>();
    let guard = state.lock().unwrap();
    guard.emit(
        &Event::new(
            Stage::AiInstall,
            "AI library installation failed",
        )
        .detail(format!("{msg}\nCore Vira features still work without AI extras."))
        .error(true),
    );
    *guard.ai_state.lock().unwrap() = AiState::NotInstalled;
}

fn track_spawned(state: &std::sync::MutexGuard<'_, SharedState>, child: &mut Child, label: &str) {
    state
        .spawned
        .lock()
        .unwrap()
        .push(proc::Tracked::new(child.id(), label));
}
