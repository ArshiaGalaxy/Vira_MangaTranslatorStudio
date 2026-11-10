//! Start-flow: allocate ports → migrate → launch Django + Next → health check
//! → navigate the launcher window into the studio UI.

use crate::paths::{append_log_line, patch_next_manifest};
use crate::ports;
use crate::proc::{self, SpawnSpec};
use crate::runtime;
use crate::state::{Event, SharedState, Stage};
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use tauri::Manager;

type StageErr = (Stage, String, String);

fn fail(state: &Mutex<SharedState>, stage: Stage, title: String, detail: String) {
    let guard = state.lock().unwrap();
    guard.failed.store(true, Ordering::SeqCst);
    guard.emit(&Event::new(stage, title).detail(detail).error(true));
}

/// Preferred ports; bumped upward automatically when occupied.
const PREFERRED_BACKEND: u16 = 8642;
const PREFERRED_FRONTEND: u16 = 3142;

/// Entry point (spawns worker thread).
pub fn run_start(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        let state = app.state::<Mutex<SharedState>>();
        {
            let guard = state.lock().unwrap();
            if !guard.begin_start() {
                return;
            }
            guard.failed.store(false, Ordering::SeqCst);
            guard.emit(&Event::new(Stage::Ports, "Preparing to start…").progress(2));
        }

        let result = start_flow(&app, &state);

        {
            let guard = state.lock().unwrap();
            guard.end_start();
        }
        if let Err((stage, title, detail)) = result {
            // Tear down whatever already started before reporting the error.
            proc::cleanup_all(&state);
            fail(&state, stage, title, detail);
        }
    });
}

fn start_flow(app: &tauri::AppHandle, state: &Mutex<SharedState>) -> Result<(), StageErr> {
    let (payload, site_packages, secret) = {
        let guard = state.lock().unwrap();
        // Bind inside the block so temporary guards drop before `guard`.
        let triple = (
            guard.payload_root.lock().unwrap().clone(),
            guard.site_packages.lock().unwrap().clone(),
            guard.secret_key.lock().unwrap().clone(),
        );
        triple
    };
    let payload =
        payload.ok_or((Stage::Ports, "Setup not finished".into(), "Wait for the setup to finish, then press Start.".into()))?;
    let site_packages =
        site_packages.ok_or((Stage::Ports, "Setup not finished".into(), "Python environment is missing.".into()))?;

    let roots = crate::paths::resolve_roots(payload);
    let backend_dir = runtime::backend_dir(&roots);
    let standalone = runtime::standalone_dir(&roots).ok_or((
        Stage::Frontend,
        "Studio interface missing".into(),
        format!(
            "The Next.js build was not found under {}. The installation appears damaged — please reinstall Vira.",
            roots.payload.join("frontend").display()
        ),
    ))?;

    // ── Ports ──────────────────────────────────────────────────────────────
    let backend_port = ports::find_free_port(PREFERRED_BACKEND);
    let frontend_port = ports::find_free_port(PREFERRED_FRONTEND);

    // ── LAN exposure (opt-in) ─────────────────────────────────────────────
    // The backend mirrors the user's choice to <data>\lan.json; binding and
    // the PIN are fixed at process start, so this is the single read point.
    // When enabled, Next.js moves to an internal loopback port and the PIN
    // gateway takes the public one (0.0.0.0) — the ONLY listener on the LAN.
    let (lan_enabled, lan_pin) = crate::paths::read_lan_config();
    let next_port = if lan_enabled {
        ports::find_free_port(frontend_port.saturating_add(1))
    } else {
        frontend_port
    };

    {
        let guard = state.lock().unwrap();
        guard.backend_port.store(backend_port as u32, Ordering::SeqCst);
        guard.frontend_port.store(frontend_port as u32, Ordering::SeqCst);
        let summary = if lan_enabled {
            "LAN access enabled: PIN gateway on the interface port"
        } else {
            "loopback only"
        };
        guard.emit(&Event::new(Stage::Ports, format!("Using backend port {backend_port}, interface port {frontend_port} ({summary})")).progress(5));
    }

    // Configure the embedded interpreter's search paths (idempotent).
    let py = runtime::python_exe(&roots)
        .ok_or((Stage::Backend, "Runtime missing".into(), "Bundled Python runtime missing.".into()))?;
    if let Err(e) = crate::paths::patch_python_pth(
        py.parent().unwrap_or_else(|| roots.payload.as_path()),
        &site_packages,
        &backend_dir,
    ) {
        return Err((Stage::Backend, "Runtime configuration failed".into(), e.to_string()));
    }

    // ── Database migrations (one-shot) ─────────────────────────────────────
    {
        let guard = state.lock().unwrap();
        guard.emit(&Event::new(Stage::Migrate, "Updating database…").progress(8));
    }
    let env = runtime::python_env(&roots, &secret, backend_port, frontend_port);
    let out = proc::run_oneshot(
        &py,
        &["-u", "manage.py", "migrate", "--noinput"],
        &backend_dir,
        &env,
        "migrate",
    )
    .map_err(|e| (Stage::Migrate, "Database update failed".into(), format!("could not run migrate: {e}")))?;
    if !out.status.success() {
        return Err((Stage::Migrate, "Database update failed".into(), truncate_out(&out, 1500)));
    }

    // recover_jobs: mark jobs orphaned by a previous session as failed.
    let _ = proc::run_oneshot(
        &py,
        &["-u", "manage.py", "recover_jobs"],
        &backend_dir,
        &env,
        "recover-jobs",
    );

    // ── Django backend ─────────────────────────────────────────────────────
    {
        let guard = state.lock().unwrap();
        guard.emit(&Event::new(Stage::Backend, "Starting translator service…").progress(12));
    }
    let mut django = proc::spawn_tracked(
        &SpawnSpec {
            program: py.clone(),
            args: vec![
                "-u".into(),
                "manage.py".into(),
                "runserver".into(),
                "--noreload".into(),
                format!("127.0.0.1:{backend_port}"),
            ],
            cwd: backend_dir.clone(),
            env: env.clone(),
            label: "Django".into(),
        },
        |line| append_log_line(&line),
    )
    .map_err(|e| (Stage::Backend, "Translator service failed to start".into(), format!("spawn error: {e}")))?;
    {
        let guard = state.lock().unwrap();
        guard.spawned.lock().unwrap().push(proc::Tracked::new(django.id(), "Django"));
    }
    {
        let pid = django.id();
        wait_health(
            backend_port,
            "/api/fonts",
            "Translator service",
            &mut django,
        )
        .map_err(|e| {
            append_log_line(&format!("[Django] pid {pid} failed health: {e}"));
            (Stage::Backend, "Translator service did not respond".into(), e)
        })?;
    }
    {
        let guard = state.lock().unwrap();
        guard.emit(&Event::new(Stage::Backend, "Translator service is running").progress(15));
    }

    // ── Next.js frontend (standalone server via bundled Node) ─────────────
    {
        let guard = state.lock().unwrap();
        guard.emit(&Event::new(Stage::Frontend, "Starting studio interface…").progress(20));
    }
    let node = runtime::node_exe(&roots)
        .ok_or((Stage::Frontend, "Runtime missing".into(), "Bundled Node runtime missing.".into()))?;
    if let Err(e) = patch_next_manifest(&standalone, backend_port) {
        return Err((Stage::Frontend, "Interface configuration failed".into(), e));
    }

    let mut node_env: Vec<(String, String)> = env
        .iter()
        .filter(|(k, _)| k != "DJANGO_SETTINGS_MODULE")
        .cloned()
        .collect();
    // In LAN mode Next listens on an internal loopback port; the PIN gateway
    // owns the public one and forwards to it.
    node_env.push(("PORT".into(), next_port.to_string()));
    node_env.push(("HOSTNAME".into(), "127.0.0.1".into()));
    node_env.push(("NODE_ENV".into(), "production".into()));
    node_env.push(("BACKEND_URL".into(), format!("http://127.0.0.1:{backend_port}")));

    let mut frontend = proc::spawn_tracked(
        &SpawnSpec {
            program: node,
            args: vec!["server.js".into()],
            cwd: standalone.clone(),
            env: node_env,
            label: "Next.js".into(),
        },
        |line| append_log_line(&line),
    )
    .map_err(|e| (Stage::Frontend, "Studio interface failed to start".into(), format!("spawn error: {e}")))?;
    {
        let guard = state.lock().unwrap();
        guard.spawned.lock().unwrap().push(proc::Tracked::new(frontend.id(), "Next.js"));
    }
    {
        let pid = frontend.id();
        wait_health(next_port, "/", "Studio interface", &mut frontend).map_err(|e| {
            append_log_line(&format!("[Next.js] pid {pid} failed health: {e}"));
            (Stage::Frontend, "Studio interface did not respond".into(), e)
        })?;
    }

    // ── LAN gateway (only when exposure is opted in) ─────────────────────
    // Listens on 0.0.0.0:{frontend_port} and forwards to Next on loopback;
    // non-loopback clients must present the 6-digit PIN (cookie after the
    // first entry). The desktop webview connects via loopback and is exempt.
    if lan_enabled {
        let node_gate = runtime::node_exe(&roots).ok_or((
            Stage::Frontend,
            "Runtime missing".into(),
            "Bundled Node runtime missing.".into(),
        ))?;
        let gate_env: Vec<(String, String)> = vec![
            ("PORT".into(), frontend_port.to_string()),
            ("UPSTREAM_PORT".into(), next_port.to_string()),
            ("VIRA_LAN_PIN".into(), lan_pin.clone()),
            ("NODE_ENV".into(), "production".into()),
            ("NEXT_TELEMETRY_DISABLED".into(), "1".into()),
        ];
        let mut gateway = proc::spawn_tracked(
            &SpawnSpec {
                program: node_gate,
                args: vec!["lan-gate.js".into()],
                cwd: standalone.clone(),
                env: gate_env,
                label: "LAN gateway".into(),
            },
            |line| append_log_line(&line),
        )
        .map_err(|e| (Stage::Frontend, "LAN gateway failed to start".into(), format!("spawn error: {e}")))?;
        {
            let guard = state.lock().unwrap();
            guard.spawned.lock().unwrap().push(proc::Tracked::new(gateway.id(), "LAN gateway"));
        }
        {
            let pid = gateway.id();
            wait_health(frontend_port, "/", "LAN gateway", &mut gateway).map_err(|e| {
                append_log_line(&format!("[LAN gateway] pid {pid} failed health: {e}"));
                (Stage::Frontend, "LAN gateway did not respond".into(), e)
            })?;
        }
    }

    // ── Navigate the launcher window into the studio ──────────────────────
    let url = format!("http://127.0.0.1:{frontend_port}/");
    {
        let guard = state.lock().unwrap();
        *guard.app_url.lock().unwrap() = Some(url.clone());
        guard.emit(&Event::new(Stage::App, "Opening the studio…").progress(60));
    }
    navigate_to(app, &url);

    {
        let guard = state.lock().unwrap();
        guard.emit(&Event::new(
            Stage::App,
            format!("Studio is running. Closing this window stops all Vira services."),
        )
        .progress(100));
    }
    Ok(())
}

fn truncate_out(out: &std::process::Output, limit: usize) -> String {
    let mut text = format!(
        "output tail:\n{}{}",
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    );
    if text.len() > limit {
        text = format!("…{}", text.split_off(text.len() - limit));
    }
    text
}

fn wait_health(
    port: u16,
    path: &str,
    label: &str,
    child: &mut std::process::Child,
) -> Result<(), String> {
    let is_alive = || matches!(child.try_wait(), Ok(None));
    crate::health::wait_for(port, path, label, if label.contains("service") { 90 } else { 60 }, is_alive)
}

/// Navigate the launcher webview into the studio web app.
fn navigate_to(app: &tauri::AppHandle, url: &str) {
    if let Some(win) = app.get_webview_window("main") {
        let js = format!("location.replace('{}')", url);
        let _ = win.eval(&js);
    }
}
