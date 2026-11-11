//! AI extras installer — three-phase resumable pipeline.
//!
//!   P0  ai_fetch.py   — Range-resume + SHA-256 download of the pinned big
//!                       wheels (torch, paddlepaddle, …) into a persistent
//!                       cache (`%LOCALAPPDATA%\Vira\ai-cache\wheels`).
//!                       Partial `.part` files survive retries AND app
//!                       restarts; verified files are never re-downloaded.
//!   P1  pip download  — fetches the remaining wheels into the same cache.
//!                       `--find-links` points at the cache so P0's wheels
//!                       (and their metadata) never touch the network again.
//!                       pip `--retries 10` plus an outer backoff loop.
//!   P2  pip install   — fully offline from the cache into site-packages.
//!
//! Failures are classified for the user: network connectivity vs dependency
//! compatibility vs package installation vs SHA-256 verification. The launcher
//! UI re-enables the button as "Retry install"; because the cache persists,
//! retrying never re-downloads verified packages.

use crate::paths::{append_log_line, vira_home_dir};
use crate::proc;
use crate::runtime::MARKER_AI;
use crate::state::{AiState, Event, SharedState, Stage};
use std::path::PathBuf;
use std::process::Child;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager};

/// Outer retry attempts for the pip-managed (small) downloads. Each attempt
/// is a fresh process (fresh %TEMP% — sidesteps pip's Windows resume bug
/// where it crashes with WinError 32 removing its own locked partial) and
/// already-completed wheels are skipped via `--find-links`.
const PHASE1_ATTEMPTS: u32 = 6;
/// pip's own per-request retries (default 5 — raised per requirement #2).
const PIP_RETRIES: &str = "10";
const PIP_TIMEOUT: &str = "60";

/// (user-facing headline, explanatory detail) for the launcher error box.
type Fail = (String, String);

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
        if let Err((title, detail)) = run_install(&app) {
            fail(&app, title, detail);
        }
        crate::setup::AI_RUNNING.store(false, Ordering::SeqCst);
    });
}

fn run_install(app: &AppHandle) -> Result<(), Fail> {
    let state = app.state::<Mutex<SharedState>>();
    {
        let guard = state.lock().unwrap();
        if *guard.ai_state.lock().unwrap() == AiState::Installed {
            return Ok(());
        }
        guard.failed.store(false, Ordering::SeqCst);
        *guard.stage.lock().unwrap() = Stage::AiInstall;
        *guard.stage_message.lock().unwrap() = "Installing AI libraries…".into();
        *guard.ai_state.lock().unwrap() = AiState::Installing;
        guard.emit(
            &Event::new(Stage::AiInstall, "Checking the AI package list…").progress(2),
        );
    }

    let (payload, site_packages) = {
        let guard = state.lock().unwrap();
        // Bind inside the block so temporary mutex guards drop before `guard`.
        let pair = (
            guard.payload_root.lock().unwrap().clone(),
            guard.site_packages.lock().unwrap().clone(),
        );
        pair
    };
    let Some(payload) = payload else {
        return Err(("Installation setup problem".into(), "Application payload not found.".into()));
    };
    let Some(sp) = site_packages else {
        return Err(("Installation setup problem".into(), "Python environment not prepared.".into()));
    };
    let py = payload.join("runtimes").join("python").join("python.exe");
    if !py.is_file() {
        return Err(("Installation setup problem".into(), "Bundled Python runtime missing.".into()));
    }
    let req = payload.join("backend").join("requirements-ai.txt");
    if !req.is_file() {
        return Err(("Installation setup problem".into(), "requirements-ai.txt missing.".into()));
    }
    let fetcher = payload.join("backend").join("ai_fetch.py");
    if !fetcher.is_file() {
        return Err((
            "Installation setup problem".into(),
            "ai_fetch.py missing from the application payload.".into(),
        ));
    }

    // Effective proxy (env var, else Windows system/WinINET registry).
    // pip never reads the registry — without propagating this, users behind
    // local proxy clients (v2rayN/xray, Clash, …) get dead-DNS failures on
    // every download while their browser works fine. Probe failures fall
    // back to a direct connection (phase 0 reports the proxy it actually
    // uses either way).
    let proxy = match resolve_proxy(app, &state, &py, &fetcher, &payload) {
        Ok(p) => p,
        Err((title, detail)) => {
            append_log_line(&format!("proxy probe failed ({title}): {detail}"));
            None
        }
    };
    let mut env: Vec<(String, String)> = vec![
        ("PYTHONDONTWRITEBYTECODE".into(), "1".into()),
        ("PYTHONNOUSERSITE".into(), "1".into()),
    ];
    if let Some((p, bypass)) = &proxy {
        env.push(("HTTP_PROXY".into(), p.clone()));
        env.push(("HTTPS_PROXY".into(), p.clone()));
        if !bypass.is_empty() {
            // Hosts with working DNS skip the tunnel (both spellings so
            // urllib and requests agree); keep any inherited entries.
            let existing = std::env::var("NO_PROXY")
                .or_else(|_| std::env::var("no_proxy"))
                .unwrap_or_default();
            let merged = if existing.is_empty() {
                bypass.clone()
            } else {
                format!("{existing},{bypass}")
            };
            env.push(("NO_PROXY".into(), merged.clone()));
            env.push(("no_proxy".into(), merged));
        }
        append_log_line(&format!(
            "AI downloads: proxy {p} (direct: {bypass})",
            bypass = if bypass.is_empty() { "none" } else { bypass }
        ));
    }

    let cache = vira_home_dir().join("ai-cache");
    let wheels = cache.join("wheels");
    if let Err(e) = std::fs::create_dir_all(&wheels) {
        return Err((
            "Installation setup problem".into(),
            format!("Cannot create the package cache at {}: {e}", wheels.display()),
        ));
    }

    phase0_fetch(app, &state, &py, &fetcher, &req, &wheels, &payload, &env)?;
    phase1_pip_download(app, &state, &py, &req, &wheels, &payload, &env)?;
    phase2_offline_install(app, &state, &py, &req, &wheels, &sp, &payload, &env)?;

    // Success: marker + snapshot state + a progress(100) event so the
    // launcher switches the button to "AI ready" (see ui/app.js).
    append_log_line("AI extras installed successfully");
    std::fs::write(sp.join(MARKER_AI), "ok")
        .map_err(|e| ("Installation setup problem".into(), format!("Cannot write marker: {e}")))?;
    let guard = state.lock().unwrap();
    *guard.ai_state.lock().unwrap() = AiState::Installed;
    guard.emit(&Event::new(Stage::AiInstall, "AI libraries installed").progress(100));
    Ok(())
}

// ── Phase 0: resumable, verified pre-fetch of the pinned wheels ──────────────

/// Ask `ai_fetch.py --print-proxy` for the effective proxy configuration:
/// line 1 = proxy URL (env var first, else the Windows system proxy from
/// the WinINET registry), line 2 = hosts whose DNS resolves directly (they
/// should bypass the tunnel — long transfers are more stable that way).
/// `Ok(None)` = direct connection. Non-fatal: any probe problem falls back
/// to direct and is logged.
fn resolve_proxy(
    app: &AppHandle,
    state: &tauri::State<'_, Mutex<SharedState>>,
    py: &PathBuf,
    fetcher: &PathBuf,
    payload: &PathBuf,
) -> Result<Option<(String, String)>, Fail> {
    let spec = proc::SpawnSpec {
        program: py.clone(),
        args: vec![fetcher.to_string_lossy().into_owned(), "--print-proxy".into()],
        cwd: payload.clone(),
        env: vec![("PYTHONNOUSERSITE".into(), "1".into())],
        label: "ai-proxy".into(),
    };
    let run = run_proc(app, state, &spec, |_app, _line| {})?;
    if !run.ok {
        return Err((
            "Installation setup problem".into(),
            "Could not determine the network proxy configuration.".into(),
        ));
    }
    let lines: Vec<&str> = run
        .lines
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    let val = lines.first().copied().unwrap_or("");
    if val.is_empty() || val.eq_ignore_ascii_case("none") {
        return Ok(None);
    }
    let bypass = lines.get(1).copied().unwrap_or("").to_string();
    Ok(Some((val.to_string(), bypass)))
}

fn phase0_fetch(
    app: &AppHandle,
    state: &tauri::State<'_, Mutex<SharedState>>,
    py: &PathBuf,
    fetcher: &PathBuf,
    req: &PathBuf,
    wheels: &PathBuf,
    payload: &PathBuf,
    env: &[(String, String)],
) -> Result<(), Fail> {
    let spec = proc::SpawnSpec {
        program: py.clone(),
        args: vec![
            fetcher.to_string_lossy().into_owned(),
            "--requirements".into(),
            req.to_string_lossy().into_owned(),
            "--dest".into(),
            wheels.to_string_lossy().into_owned(),
            // Direct assets (e.g. the ~200 MB LaMa ONNX model) go into the
            // app data dir, where the engine searches for them. Not wheels:
            // a Core install cannot load them without onnxruntime anyway.
            "--assets".into(),
            payload.join("backend").join("requirements-ai-assets.txt")
                .to_string_lossy().into_owned(),
            "--asset-dest".into(),
            crate::paths::user_data_dir().join("models").to_string_lossy().into_owned(),
        ],
        cwd: payload.clone(),
        env: env.to_vec(),
        label: "ai-fetch".into(),
    };

    let proto_err: Arc<Mutex<Option<Fail>>> = Arc::new(Mutex::new(None));
    let err_slot = Arc::clone(&proto_err);

    let run = run_proc(app, state, &spec, move |app, line| {
        let parts: Vec<&str> = line.split('|').collect();
        match parts.first().copied().unwrap_or("") {
            "PROXY" => {
                if let Some(p) = parts.get(1) {
                    append_log_line(&format!("download proxy: {p}"));
                }
            }
            "TOTAL" => {
                let pending = parts.get(1).unwrap_or(&"?");
                let saved = parts.get(2).unwrap_or(&"0");
                let bytes: u64 = parts.get(3).and_then(|v| v.parse().ok()).unwrap_or(0);
                let msg = if saved.parse::<u64>().unwrap_or(0) > 0 {
                    format!(
                        "Preparing {pending} packages — {} total ({saved} already saved)",
                        mb(bytes)
                    )
                } else {
                    format!("Preparing {pending} packages — {} to download", mb(bytes))
                };
                emit_ev(app, Event::new(Stage::AiInstall, msg).progress(3));
            }
            "PKG" => {
                let (i, n, file) = (parts.get(1), parts.get(2), parts.get(3));
                if let (Some(i), Some(n), Some(f)) = (i, n, file) {
                    emit_ev(app, Event::new(Stage::AiInstall, format!("Downloading ({i}/{n}) {f}")));
                }
            }
            "RETRY" => {
                // Requirement #2: visible backoff. "file | attempt | max | delay | reason"
                if let (Some(f), Some(a), Some(m), Some(d)) =
                    (parts.get(1), parts.get(2), parts.get(3), parts.get(4))
                {
                    emit_ev(app, Event::new(
                        Stage::AiInstall,
                        format!("Connection trouble on {f} — retry {a}/{m} in {d}s (download resumes)"),
                    ));
                }
            }
            "PROGRESS" => {
                // pct | i | n | filename | got | size
                let pct: u8 = parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
                let idx = parts.get(2).and_then(|v| v.parse::<u8>().ok()).unwrap_or(0);
                let n = parts.get(3).and_then(|v| v.parse::<u8>().ok()).unwrap_or(0);
                let file = parts.get(4).copied().unwrap_or("package");
                let got: u64 = parts.get(5).and_then(|v| v.parse().ok()).unwrap_or(0);
                let size: u64 = parts.get(6).and_then(|v| v.parse().ok()).unwrap_or(0);
                let mut msg = if size > 0 {
                    format!("Downloading ({idx}/{n}) {file} — {} / {} ({pct}%)", mb(got), mb(size))
                } else {
                    format!("Downloading ({idx}/{n}) {file} — {}", mb(got))
                };
                if msg.len() > 160 {
                    msg.truncate(157);
                    msg.push('…');
                }
                let mapped = 2u8.saturating_add(((pct as u32 * 68) / 100) as u8);
                emit_ev(app, Event::new(Stage::AiInstall, msg).progress(mapped));
            }
            "FILE_DONE" => {
                if let Some(f) = parts.get(3) {
                    append_log_line(&format!("verified: {f}"));
                }
            }
            "ALL_DONE" => {
                emit_ev(app, Event::new(
                    Stage::AiInstall,
                    "All large packages downloaded and verified",
                ).progress(70));
            }
            "WARN" => {
                if let (Some(p), Some(r)) = (parts.get(1), parts.get(2)) {
                    append_log_line(&format!("fetch warning: {p} — {r}"));
                }
            }
            "ERR" => {
                let kind = parts.get(1).unwrap_or(&"network");
                let subject = parts.get(2).unwrap_or(&"package");
                let msg = parts[3.min(parts.len() - 1)..].join("|");
                let mut slot = err_slot.lock().unwrap();
                *slot = Some(classify_fetch_error(kind, subject, &msg));
            }
            _ => {}
        }
    })?;

    if let Some(e) = proto_err.lock().unwrap().take() {
        return Err(e);
    }
    if !run.ok {
        // Crashed without a protocol ERR record — classify from captured output.
        return Err(classify_pip(&run.lines));
    }
    Ok(())
}

fn classify_fetch_error(kind: &str, subject: &str, msg: &str) -> Fail {
    let resume_hint = "Press \"Retry install\" — downloads continue where they stopped and already-verified packages are not fetched again.";
    match kind {
        "verify" => (
            format!("Package verification problem — {subject} failed its SHA-256 check"),
            format!("{msg}\n\n{resume_hint}"),
        ),
        "setup" => (
            format!("Installation setup problem — {subject}"),
            format!("{msg}"),
        ),
        _ => (
            format!("Network connectivity problem — {subject} could not be downloaded"),
            format!(
                "{msg}\n\nThe download was interrupted repeatedly. {resume_hint}"
            ),
        ),
    }
}

// ── Phase 1: pip-managed remainder with outer exponential backoff ────────────

fn phase1_pip_download(
    app: &AppHandle,
    state: &tauri::State<'_, Mutex<SharedState>>,
    py: &PathBuf,
    req: &PathBuf,
    wheels: &PathBuf,
    payload: &PathBuf,
    env: &[(String, String)],
) -> Result<(), Fail> {
    let counter = Arc::new(AtomicU32::new(0));
    let mut last_lines: Vec<String> = Vec::new();

    for attempt in 1..=PHASE1_ATTEMPTS {
        emit_ev(app, Event::new(
            Stage::AiInstall,
            if attempt == 1 {
                "Fetching the remaining AI packages…".into()
            } else {
                format!("Retrying package downloads (attempt {attempt}/{PHASE1_ATTEMPTS})…")
            },
        ).progress(72));

        let spec = proc::SpawnSpec {
            program: py.clone(),
            args: vec![
                "-m".into(),
                "pip".into(),
                "download".into(),
                "-r".into(),
                req.to_string_lossy().into_owned(),
                "--dest".into(),
                wheels.to_string_lossy().into_owned(),
                "--find-links".into(),
                wheels.to_string_lossy().into_owned(),
                "--retries".into(),
                PIP_RETRIES.into(),
                "--timeout".into(),
                PIP_TIMEOUT.into(),
                "--progress-bar".into(),
                "off".into(),
                // NOTE: `pip download` has no --no-warn-script-location
                // (verified empirically — it exits with "no such option").
            ],
            cwd: payload.clone(),
            env: env.to_vec(),
            label: "pip-ai-dl".into(),
        };

        let n_counter = Arc::clone(&counter);
        let run = run_proc(app, state, &spec, move |app, line| {
            if let Some(rest) = strip_once(line, "Downloading ") {
                let n = n_counter.fetch_add(1, Ordering::Relaxed) + 1;
                // "file.whl (200.8 MB)" — show package + size (requirement #6).
                let pkg = rest.split(" (from").next().unwrap_or(rest).trim();
                let progress = 73u8.saturating_add((n.min(14)) as u8);
                emit_ev(app, Event::new(
                    Stage::AiInstall,
                    format!("Downloading {n}: {pkg}"),
                ).progress(progress));
            }
        })?;

        if run.ok {
            emit_ev(app, Event::new(
                Stage::AiInstall,
                "All packages downloaded",
            ).progress(88));
            return Ok(());
        }

        let dependency_conflict = is_dependency_failure(&run.lines);
        last_lines = run.lines;
        if dependency_conflict {
            return Err(classify_pip(&last_lines));
        }
        if attempt < PHASE1_ATTEMPTS {
            // 5s, 15s, 45s, 60s, 60s — exponential backoff, capped so a
            // flaky connection can't turn one install into a 20-min wait.
            let delay = (5u64 * 3u64.pow(attempt - 1)).min(60);
            emit_ev(app, Event::new(
                Stage::AiInstall,
                format!("Network trouble — retrying in {delay}s (attempt {attempt}/{PHASE1_ATTEMPTS})…"),
            ).progress(72));
            std::thread::sleep(std::time::Duration::from_secs(delay));
        }
    }
    Err(classify_pip(&last_lines))
}

// ── Phase 2: fully offline install from the verified cache ───────────────────

fn phase2_offline_install(
    app: &AppHandle,
    state: &tauri::State<'_, Mutex<SharedState>>,
    py: &PathBuf,
    req: &PathBuf,
    wheels: &PathBuf,
    site_packages: &PathBuf,
    payload: &PathBuf,
    env: &[(String, String)],
) -> Result<(), Fail> {
    emit_ev(app, Event::new(Stage::AiInstall, "Installing packages…").progress(92));

    let spec = proc::SpawnSpec {
        program: py.clone(),
        args: vec![
            "-m".into(),
            "pip".into(),
            "install".into(),
            "--no-index".into(),
            "--find-links".into(),
            wheels.to_string_lossy().into_owned(),
            "--target".into(),
            site_packages.to_string_lossy().into_owned(),
            "--no-warn-script-location".into(),
            "-r".into(),
            req.to_string_lossy().into_owned(),
        ],
        cwd: payload.clone(),
        env: env.to_vec(),
        label: "pip-ai".into(),
    };

    let run = run_proc(app, state, &spec, |_app, _line| {})?;
    if run.ok {
        emit_ev(app, Event::new(Stage::AiInstall, "Installing packages… done").progress(99));
        return Ok(());
    }
    Err(classify_pip(&run.lines))
}

// ── Failure classification (requirement #8) ──────────────────────────────────

const NETWORK_MARKERS: &[&str] = &[
    "could not fetch",
    "connectionerror",
    "connection refused",
    "newconnectionerror",
    "connectionreseterror",
    "nameresolution",
    "getaddrinfo",
    "incomplete-download",
    "read timed out",
    "timed out",
    "max retries exceeded",
    "temporary failure",
    "proxyerror",
    "sslerror",
    "network is unreachable",
    "winerror 10054",
    "remote end closed connection",
    "connection aborted",
    // pip on Windows crashes like this while resuming a download that was
    // interrupted mid-transfer (it removes its own locked %TEMP% partial) —
    // the root cause is the interrupted network transfer, not the install.
    "winerror 32",
    "being used by another process",
];

const DEPENDENCY_MARKERS: &[&str] = &[
    "resolutionimpossible",
    "conflicting dependencies",
    "conflict is caused by",
    "cannot install",
];

fn is_dependency_failure(lines: &[String]) -> bool {
    let low = lines.join("\n").to_lowercase();
    DEPENDENCY_MARKERS.iter().any(|m| low.contains(m))
}

fn error_excerpt(lines: &[String]) -> String {
    // Prefer lines around "error", else the tail of the output.
    let mut picked: Vec<&str> = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        let low = l.to_lowercase();
        if low.contains("error") || low.contains("failed") {
            for j in i..(i + 3).min(lines.len()) {
                let s: &str = &lines[j];
                if !picked.contains(&s) {
                    picked.push(s);
                }
            }
        }
    }
    if picked.is_empty() {
        picked = lines.iter().rev().take(6).rev().map(|s| s.as_str()).collect();
    }
    let mut text = picked.join("\n");
    if text.len() > 700 {
        text.truncate(697);
        text.push('…');
    }
    text
}

fn classify_pip(lines: &[String]) -> Fail {
    let low = lines.join("\n").to_lowercase();
    let excerpt = error_excerpt(lines);
    if DEPENDENCY_MARKERS.iter().any(|m| low.contains(m)) {
        (
            "Dependency compatibility problem — the AI packages cannot be resolved together".into(),
            format!(
                "{excerpt}\n\nThis is a package-compatibility issue, not a download problem."
            ),
        )
    } else if NETWORK_MARKERS.iter().any(|m| low.contains(m)) {
        (
            "Network connectivity problem — could not fetch the AI packages".into(),
            format!(
                "{excerpt}\n\nCheck your internet connection and press \"Retry install\" — progress already made is kept and verified packages are not re-downloaded."
            ),
        )
    } else {
        (
            "Package installation problem — files were downloaded but installation failed".into(),
            format!(
                "{excerpt}\n\nPress \"Retry install\" to continue from the saved package cache."
            ),
        )
    }
}

// ── Process plumbing ─────────────────────────────────────────────────────────

struct ProcRun {
    ok: bool,
    lines: Vec<String>,
}

/// Spawn a tracked child, log + collect its lines, forward each line to
/// `on_line` as it arrives. Collects at most 1200 lines for classification.
fn run_proc(
    app: &AppHandle,
    state: &tauri::State<'_, Mutex<SharedState>>,
    spec: &proc::SpawnSpec,
    on_line: impl Fn(&AppHandle, &str) + Send + 'static,
) -> Result<ProcRun, Fail> {
    let lines: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let cb = {
        let lines = Arc::clone(&lines);
        let app = app.clone();
        let label = spec.label.clone();
        move |raw: String| {
            append_log_line(&raw);
            let cleaned = strip_label(&raw, &label);
            if let Ok(mut v) = lines.lock() {
                if v.len() < 1200 {
                    v.push(cleaned.to_string());
                }
            }
            on_line(&app, cleaned);
        }
    };

    let mut child: Child = proc::spawn_tracked(spec, cb).map_err(|e| {
        (
            "Installation setup problem".into(),
            format!("Failed to start {}: {e}", spec.label),
        )
    })?;
    {
        let guard = state.lock().unwrap();
        guard
            .spawned
            .lock()
            .unwrap()
            .push(proc::Tracked::new(child.id(), &spec.label));
    }
    let status = child.wait().map_err(|e| {
        (
            "Installation setup problem".into(),
            format!("Waiting for {} failed: {e}", spec.label),
        )
    })?;
    let ok = status.success();
    let captured = lines.lock().map(|v| v.clone()).unwrap_or_default();
    Ok(ProcRun { ok, lines: captured })
}

fn strip_label<'a>(raw: &'a str, label: &str) -> &'a str {
    let with = format!("[{label}] ");
    if let Some(r) = raw.strip_prefix(with.as_str()) {
        return r;
    }
    let err = format!("[{label}:err] ");
    if let Some(r) = raw.strip_prefix(err.as_str()) {
        return r;
    }
    raw
}

fn strip_once<'a>(line: &'a str, prefix: &str) -> Option<&'a str> {
    line.strip_prefix(prefix)
}

fn mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_048_576.0)
}

fn emit_ev(app: &AppHandle, ev: Event) {
    let _ = app.emit("vira://event", ev);
}

fn fail(app: &AppHandle, title: String, detail: String) {
    let state = app.state::<Mutex<SharedState>>();
    let guard = state.lock().unwrap();
    guard.emit(
        &Event::new(Stage::AiInstall, title)
            .detail(format!("{detail}\n\nCore Vira features still work without AI extras."))
            .error(true),
    );
    *guard.ai_state.lock().unwrap() = AiState::NotInstalled;
}
