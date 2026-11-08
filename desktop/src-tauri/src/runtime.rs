//! Runtime environment bootstrap: data dirs, runtime verification, and the
//! offline pip install of core dependencies from bundled wheels.
//!
//! Contract: every stage is idempotent and keyed on durable markers so a
//! *working* environment is detected and reused, never recreated.

use crate::paths::{append_log_line, Roots};
use crate::proc;
use crate::state::{Event, SharedState, Stage};
use std::path::Path;
use std::sync::{Arc, Mutex};

pub const MARKER_CORE: &str = ".vira-core-ok";
pub const MARKER_AI: &str = ".vira-ai-ok";

pub fn python_exe(roots: &Roots) -> Option<std::path::PathBuf> {
    let exe = roots
        .payload
        .join("runtimes")
        .join("python")
        .join("python.exe");
    exe.is_file().then_some(exe)
}

pub fn node_exe(roots: &Roots) -> Option<std::path::PathBuf> {
    let exe = roots.payload.join("runtimes").join("node").join("node.exe");
    exe.is_file().then_some(exe)
}

pub fn backend_dir(roots: &Roots) -> std::path::PathBuf {
    roots.payload.join("backend")
}

/// Location of the Next.js standalone server, if the build is present.
/// Returns `None` when neither layout exists (damaged installation).
pub fn standalone_dir(roots: &Roots) -> Option<std::path::PathBuf> {
    let direct = roots.payload.join("frontend").join(".next").join("standalone");
    if direct.join("server.js").is_file() {
        return Some(direct);
    }
    let fallback = roots.payload.join("frontend").join("standalone");
    fallback.join("server.js").is_file().then_some(fallback)
}

/// Child-process environment for Python one-shots and (superset for) servers.
pub fn python_env(_roots: &Roots, secret: &str, backend_port: u16, frontend_port: u16) -> Vec<(String, String)> {
    vec![
        ("STUDIO_DESKTOP".into(), "1".into()),
        ("STUDIO_BACKEND_PORT".into(), backend_port.to_string()),
        ("STUDIO_FRONTEND_PORT".into(), frontend_port.to_string()),
        ("DJANGO_SETTINGS_MODULE".into(), "backend.settings".into()),
        ("DJANGO_DEBUG".into(), "0".into()),
        ("DJANGO_ALLOWED_HOSTS".into(), "localhost,127.0.0.1".into()),
        ("DJANGO_SECRET_KEY".into(), secret.into()),
        ("PYTHONUNBUFFERED".into(), "1".into()),
        ("PYTHONDONTWRITEBYTECODE".into(), "1".into()),
        ("NEXT_TELEMETRY_DISABLED".into(), "1".into()),
        // Keep the user's PATH out of the equation: runtimes are explicit.
        ("PYTHONNOUSERSITE".into(), "1".into()),
    ]
}

fn wheels_dir(roots: &Roots) -> std::path::PathBuf {
    roots.payload.join("wheels")
}

/// Verify the bundled runtimes are present and functional (cheap probes).
pub fn verify_runtimes(state: &SharedState, roots: &Roots) -> Result<(), String> {
    let py = python_exe(roots)
        .ok_or_else(|| format!("Bundled Python runtime missing (expected {})", roots.payload.join("runtimes\\python\\python.exe").display()))?;
    let node = node_exe(roots)
        .ok_or_else(|| format!("Bundled Node runtime missing (expected {})", roots.payload.join("runtimes\\node\\node.exe").display()))?;

    let env: Vec<(String, String)> = vec![("PYTHONDONTWRITEBYTECODE".into(), "1".into())];
    let out = proc::run_oneshot(&py, &["-c", "import sys; print(sys.version_info[:2])"], roots.payload.as_path(), &env, "python-check")
        .map_err(|e| format!("could not run bundled Python: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "Bundled Python failed to run (code {:?}): {}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr).chars().take(400).collect::<String>()
        ));
    }
    let version = String::from_utf8_lossy(&out.stdout);
    if !version.contains("(3, 10)") {
        return Err(format!("Bundled Python reported wrong version: {version}"));
    }

    let out = proc::run_oneshot(&node, &["--version"], roots.payload.as_path(), &[], "node-check")
        .map_err(|e| format!("could not run bundled Node: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "Bundled Node failed to run (code {:?})",
            out.status.code()
        ));
    }
    state.emit(&Event::new(Stage::Runtimes, "Runtimes verified (Python 3.10, Node bundled)"));
    Ok(())
}

fn marker_current(marker: &Path, expect_subdir: &str) -> bool {
    marker.is_file() && std::fs::read_to_string(marker)
        .map(|f| f.trim() == expect_subdir)
        .unwrap_or(false)
}

/// Install core Python dependencies offline from bundled wheels into the
/// user-local site-packages. Skipped entirely when the marker exists and the
/// directory is populated — a working environment is never recreated.
pub fn ensure_python_env(state: &SharedState, roots: &Roots) -> Result<(), String> {
    let sp = &roots.site_packages;
    std::fs::create_dir_all(sp).map_err(|e| format!("cannot create {}: {e}", sp.display()))?;

    let marker = sp.join(MARKER_CORE);
    // Marker records which backend dir expectation it was prepared for.
    let prepared_for = backend_dir(roots).display().to_string();
    if marker_current(&marker, &prepared_for) {
        state.emit(&Event::new(Stage::PythonEnv, "Python environment already prepared — reusing"));
        return Ok(());
    }

    let py = python_exe(roots).ok_or("python runtime missing")?;
    let wheels = wheels_dir(roots);
    if !wheels.is_dir() {
        return Err(format!("Bundled wheels directory missing: {}", wheels.display()));
    }

    // Make pip + site-packages importable for the embedded interpreter.
    crate::paths::patch_python_pth(
        py.parent().unwrap_or_else(|| roots.payload.as_path()),
        &roots.site_packages,
        &backend_dir(roots),
    )
    .map_err(|e| format!("cannot patch python runtime paths: {e}"))?;

    state.emit(&Event::new(Stage::PythonEnv, "Preparing Python environment (first run only)"));

    // 1) Bootstrap pip from the bundled get-pip.py + wheels (no network).
    let getpip = roots.payload.join("get-pip.py");
    if !getpip.is_file() {
        return Err(format!("Bundled get-pip.py missing: {}", getpip.display()));
    }
    let env: Vec<(String, String)> = vec![
        ("PYTHONDONTWRITEBYTECODE".into(), "1".into()),
        ("PYTHONNOUSERSITE".into(), "1".into()),
    ];
    let out = proc::run_oneshot(
        &py,
        &[
            getpip.to_string_lossy().as_ref(),
            "--no-index".as_ref(),
            "--find-links".as_ref(),
            wheels.to_string_lossy().as_ref(),
            "--no-warn-script-location".as_ref(),
        ],
        roots.payload.as_path(),
        &env,
        "pip-bootstrap",
    )
    .map_err(|e| format!("pip bootstrap failed to start: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "pip bootstrap failed:\n{}",
            tail(&out, 1200)
        ));
    }

    // 2) Install core requirements --target site-packages (no network).
    let req = backend_dir(roots).join("requirements-core.txt");
    let out = proc::run_oneshot(
        &py,
        &[
            "-m", "pip", "install", "--no-index",
            "--find-links", wheels.to_string_lossy().as_ref(),
            "--target", sp.to_string_lossy().as_ref(),
            "--no-warn-script-location", "--no-compile",
            "--progress-bar", "off",
            "-r", req.to_string_lossy().as_ref(),
        ],
        roots.payload.as_path(),
        &env,
        "pip-core",
    )
    .map_err(|e| format!("pip install failed to start: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "Dependency installation failed:\n{}",
            tail(&out, 2000)
        ));
    }
    std::fs::write(&marker, &prepared_for).map_err(|e| format!("cannot write marker: {e}"))?;
    append_log_line("core python env installed offline from bundled wheels");
    state.emit(&Event::new(Stage::PythonEnv, "Python environment ready"));
    Ok(())
}

fn tail(out: &std::process::Output, limit: usize) -> String {
    let mut text = format!(
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    if text.len() > limit {
        text = text.split_off(text.len() - limit);
        text = format!("…{text}");
    }
    text
}

/// Detect whether the AI extras are installed (marker file check; the marker
/// is written by ai.rs after a successful install).
pub fn ai_installed(sp: &Path) -> bool {
    sp.is_dir() && sp.join(MARKER_AI).is_file()
}

/// Shared emit helper used by long-running orchestrators.
pub type Shared = Arc<Mutex<SharedState>>;

pub fn emit(shared: &Shared, ev: Event) {
    let guard = shared.lock().unwrap();
    guard.emit(&ev);
}
