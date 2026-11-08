//! Child process spawning with logging + job-object supervision.

use crate::jobobj;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

/// A spawned child we may need to terminate by PID tree.
pub struct Tracked {
    pub pid: u32,
    pub label: String,
    handle: usize,
    exited: Mutex<bool>,
}

impl Tracked {
    pub fn new(pid: u32, label: &str) -> Self {
        Self { pid, label: label.to_string(), handle: 0, exited: Mutex::new(false) }
    }

    pub fn kill_tree(&self) {
        let mut exited = self.exited.lock().unwrap();
        if *exited {
            return;
        }
        // Preferred: force-kill the exact tree by PID (taskkill /T), scoped to
        // the PIDs Vira spawned itself. The Job Object below is a belt-and-
        // braces guarantee (children that outlive taskkill races).
        #[cfg(windows)]
        {
            let _ = Command::new("taskkill")
                .args(["/PID", &self.pid.to_string(), "/T", "/F"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            // Explicit handle close is implicit on drop of the raw handle.
        }
        *exited = true;
    }
}

/// Spawn options: working dir, env, and per-process label for logging.
pub struct SpawnSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    pub label: String,
}

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(windows)]
const CREATE_SUSPENDED: u32 = 0x0000_0004;

/// Resume every thread of a process created with CREATE_SUSPENDED.
/// `std::process::Child` only exposes the process handle, and ResumeThread
/// needs a thread handle — so enumerate the process's threads with
/// Toolhelp32, open them with THREAD_SUSPEND_RESUME and resume them.
#[cfg(windows)]
fn resume_process_threads(pid: u32) -> bool {
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct ThreadEntry32 {
        dw_size: u32,
        cnt_usage: u32,
        th32_thread_id: u32,
        th32_owner_process_id: u32,
        tp_base_priority: i32,
        tp_delta_priority: i32,
        dw_flags: u32,
    }
    extern "system" {
        fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> isize;
        fn Thread32First(snap: isize, entry: *mut ThreadEntry32) -> i32;
        fn Thread32Next(snap: isize, entry: *mut ThreadEntry32) -> i32;
        fn OpenThread(access: u32, inherit: i32, tid: u32) -> isize;
        fn ResumeThread(handle: isize) -> u32;
        fn CloseHandle(handle: isize) -> i32;
    }
    const TH32CS_SNAPTHREAD: u32 = 0x0000_0004;
    const THREAD_SUSPEND_RESUME: u32 = 0x0002;
    const INVALID_HANDLE_VALUE: isize = -1;

    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if snap == 0 || snap == INVALID_HANDLE_VALUE {
            return false;
        }
        let mut entry: ThreadEntry32 = std::mem::zeroed();
        entry.dw_size = std::mem::size_of::<ThreadEntry32>() as u32;
        let mut resumed = false;
        if Thread32First(snap, &mut entry) != 0 {
            loop {
                if entry.th32_owner_process_id == pid {
                    let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32_thread_id);
                    if thread != 0 {
                        if ResumeThread(thread) != 0xFFFF_FFFF {
                            resumed = true;
                        }
                        CloseHandle(thread);
                    }
                }
                if Thread32Next(snap, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
        resumed
    }
}

/// Spawn a hidden child process assigned to Vira's job object, and stream
/// (prefix, stdout+stderr lines) to `on_line` as they arrive.
/// Returns Ok(child) once the process is running (already resumed).
pub fn spawn_tracked(
    spec: &SpawnSpec,
    on_line: impl Fn(String) + Send + 'static,
) -> std::io::Result<Child> {
    let mut cmd = Command::new(&spec.program);
    cmd.args(&spec.args)
        .current_dir(&spec.cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("PYTHONDONTWRITEBYTECODE");
    for (k, v) in &spec.env {
        cmd.env(k, v);
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_SUSPENDED: the process exists but cannot have spawned
        // grandchildren before we attach it to the job object.
        cmd.creation_flags(CREATE_NO_WINDOW | CREATE_SUSPENDED);
    }

    let mut child = cmd.spawn()?;

    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        let handle = child.as_raw_handle() as usize;
        if !jobobj::assign_to_job(handle) {
            // Fall through: process runs un-supervised but is still killed
            // explicitly via taskkill on exit.
        }
        // ResumeThread must be given a *thread* handle, not the process
        // handle (which is what Child exposes). Enumerate the process's
        // threads via the Toolhelp snapshot and resume them — a freshly
        // created suspended process has exactly one (the main thread).
        if !resume_process_threads(child.id()) {
            log(format!(
                "[{}] warning: could not resume main thread after CREATE_SUSPENDED spawn",
                spec.label
            ));
        }
    }

    // Drain stdout/stderr on separate threads sharing the same line sink.
    // Arc<Mutex<F>> lets both reader threads own a handle without requiring
    // the callback to be Sync (only Send, which the bound already guarantees).
    let on_line = std::sync::Arc::new(std::sync::Mutex::new(on_line));
    let label = spec.label.clone();
    if let Some(stdout) = child.stdout.take() {
        let cb = std::sync::Arc::clone(&on_line);
        std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                match line {
                    Ok(l) => (*cb.lock().unwrap())(format!("[{label}] {l}")),
                    Err(_) => break,
                }
            }
        });
    }
    let label = spec.label.clone();
    if let Some(stderr) = child.stderr.take() {
        std::thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines() {
                match line {
                    Ok(l) => (*on_line.lock().unwrap())(format!("[{label}:err] {l}")),
                    Err(_) => break,
                }
            }
        });
    }

    Ok(child)
}

/// Run a short one-shot command to completion, returning captured output.
pub fn run_oneshot(
    program: &Path,
    args: &[&str],
    cwd: &Path,
    env: &[(String, String)],
    label: &str,
) -> std::io::Result<std::process::Output> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = cmd.output();
    match &out {
        Ok(_) => {}
        Err(e) => log(format!("{label} spawn failed: {e}")),
    }
    out
}

pub fn log(msg: impl AsRef<str>) {
    // Logging is wired by state/log.rs at call sites via with_log_file; this
    // appends to the launcher log best-effort.
    crate::paths::append_log_line(msg.as_ref());
}

pub fn track(
    state: &crate::state::SharedState,
    pid: u32,
    label: &str,
    handle: usize,
    exited: bool,
) {
    let mut spawned = state.spawned.lock().unwrap();
    spawned.push(Tracked {
        pid,
        label: label.to_string(),
        handle,
        exited: Mutex::new(exited),
    });
}

/// Kill every tracked child (by PID tree) and terminate the job object.
pub fn cleanup_all(state: &Mutex<crate::state::SharedState>) {
    let guard = state.lock().unwrap();
    {
        let spawned = guard.spawned.lock().unwrap();
        for t in spawned.iter() {
            t.kill_tree();
        }
    }
    jobobj::terminate_job();
    guard.spawned.lock().unwrap().clear();
}
