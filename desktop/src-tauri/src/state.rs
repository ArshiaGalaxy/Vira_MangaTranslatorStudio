use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::Mutex;
use tauri::AppHandle;

use crate::proc;

/// Bootstrap stages shown in the launcher UI. Serialized as snake_case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Init,
    Paths,
    Data,
    Secrets,
    Runtimes,
    PythonEnv,
    Ready,
    Starting,
    Ports,
    Migrate,
    Backend,
    Frontend,
    App,
    Failed,
    AiInstall,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AiState {
    Unknown,
    NotInstalled,
    Installing,
    Installed,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Event {
    pub stage: Stage,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<bool>,
}

impl Event {
    pub fn new(stage: Stage, message: impl Into<String>) -> Self {
        Self { stage, message: message.into(), detail: None, progress: None, error: None }
    }
    pub fn detail(mut self, d: impl Into<String>) -> Self {
        self.detail = Some(d.into());
        self
    }
    pub fn progress(mut self, p: u8) -> Self {
        self.progress = Some(p);
        self
    }
    pub fn error(mut self, e: bool) -> Self {
        self.error = Some(e);
        self
    }
}

#[derive(Clone, serde::Serialize)]
pub struct Snapshot {
    pub stage: Stage,
    pub stage_message: Option<String>,
    pub ready: bool,
    pub failed: bool,
    pub app_url: Option<String>,
    pub data_dir: Option<String>,
    pub logs_dir: Option<String>,
    pub ai_state: AiState,
    pub backend_port: Option<u16>,
    pub frontend_port: Option<u16>,
}

pub struct SharedState {
    pub app: AppHandle,
    pub stage: Mutex<Stage>,
    pub stage_message: Mutex<String>,
    pub failed: AtomicBool,
    pub ready: AtomicBool,
    pub app_url: Mutex<Option<String>>,
    pub payload_root: Mutex<Option<std::path::PathBuf>>,
    pub logs_dir: Mutex<Option<std::path::PathBuf>>,
    pub data_dir: Mutex<Option<std::path::PathBuf>>,
    pub vira_home: Mutex<Option<std::path::PathBuf>>,
    pub site_packages: Mutex<Option<std::path::PathBuf>>,
    pub secret_key: Mutex<String>,
    pub ai_state: Mutex<AiState>,
    pub backend_port: AtomicU32,
    pub frontend_port: AtomicU32,
    /// Job object handle for all spawned children (Windows).
    pub job_handle: AtomicUsize,
    /// Child processes in tree-kill order.
    pub spawned: Mutex<Vec<proc::Tracked>>,
    setup_running: AtomicBool,
    starting: AtomicBool,
}

impl SharedState {
    pub fn new(app: AppHandle) -> Self {
        Self {
            app,
            stage: Mutex::new(Stage::Init),
            stage_message: Mutex::new("Starting up…".into()),
            failed: AtomicBool::new(false),
            ready: AtomicBool::new(false),
            app_url: Mutex::new(None),
            payload_root: Mutex::new(None),
            logs_dir: Mutex::new(None),
            data_dir: Mutex::new(None),
            vira_home: Mutex::new(None),
            site_packages: Mutex::new(None),
            secret_key: Mutex::new(String::new()),
            ai_state: Mutex::new(AiState::Unknown),
            backend_port: AtomicU32::new(0),
            frontend_port: AtomicU32::new(0),
            job_handle: AtomicUsize::new(0),
            spawned: Mutex::new(Vec::new()),
            setup_running: AtomicBool::new(false),
            starting: AtomicBool::new(false),
        }
    }

    /// Build the real state once AppHandle is available (called in setup hook).
    pub fn bootstrap(app: AppHandle) -> Self {
        Self::new(app)
    }

    pub fn snapshot(state: &tauri::State<'_, Mutex<SharedState>>) -> Snapshot {
        let guard = state.lock().unwrap();
        // Bind to a local so the temporary MutexGuards inside the literal are
        // dropped before `guard` goes out of scope.
        let snap = Snapshot {
            stage: *guard.stage.lock().unwrap(),
            stage_message: Some(guard.stage_message.lock().unwrap().clone()),
            ready: guard.ready.load(Ordering::SeqCst),
            failed: guard.failed.load(Ordering::SeqCst),
            app_url: guard.app_url.lock().unwrap().clone(),
            data_dir: guard.data_dir.lock().unwrap().as_ref().map(|p| p.display().to_string()),
            logs_dir: guard.logs_dir.lock().unwrap().as_ref().map(|p| p.display().to_string()),
            ai_state: *guard.ai_state.lock().unwrap(),
            backend_port: { let p = guard.backend_port.load(Ordering::SeqCst); if p == 0 { None } else { Some(p as u16) } },
            frontend_port: { let p = guard.frontend_port.load(Ordering::SeqCst); if p == 0 { None } else { Some(p as u16) } },
        };
        snap
    }

    pub fn emit(&self, ev: &Event) {
        *self.stage.lock().unwrap() = ev.stage;
        *self.stage_message.lock().unwrap() = ev.message.clone();
        if ev.error == Some(true) {
            self.failed.store(true, Ordering::SeqCst);
        }
        if ev.stage == Stage::Ready {
            self.ready.store(true, Ordering::SeqCst);
        }
        let app = self.app.clone();
        let payload = ev.clone();
        // Emit from a short-lived task; emit itself is cheap and thread-safe.
        std::thread::spawn(move || {
            use tauri::Emitter;
            let _ = app.emit("vira://event", payload);
        });
    }

    pub fn log_dir() -> Option<std::path::PathBuf> {
        dirs_local()
    }

    pub fn begin_setup(&self) -> bool {
        self.setup_running.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_ok()
    }
    pub fn end_setup(&self) {
        self.setup_running.store(false, Ordering::SeqCst);
    }
    pub fn begin_start(&self) -> bool {
        self.starting.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_ok()
    }
    pub fn end_start(&self) {
        self.starting.store(false, Ordering::SeqCst);
    }
}

/// `%LOCALAPPDATA%` without the `dirs` crate: resolve via env with a shell-folder
/// fallback (same approach backend/settings.py uses).
pub fn dirs_local() -> Option<std::path::PathBuf> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt;
        if let Some(v) = std::env::var_os("LOCALAPPDATA") {
            let s = std::path::PathBuf::from(v);
            if s.is_absolute() {
                return Some(s);
            }
        }
        // Fallback: read the User Shell Folders registry value (REG_EXPAND_SZ).
        const KEY_PATH: &[u8] = b"Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\User Shell Folders";
        const VALUE: &[u8] = b"Local AppData";
        let key_path: Vec<u16> = KEY_PATH.iter().chain(std::iter::once(&0u8)).map(|&b| b as u16).collect();
        let value_name: Vec<u16> = VALUE.iter().chain(std::iter::once(&0u8)).map(|&b| b as u16).collect();
        #[repr(C)]
        struct RegKeyH(u64);
        extern "system" {
            fn RegOpenKeyExW(h: usize, sub: *const u16, opt: u32, access: u32, result: *mut usize) -> i32;
            fn RegQueryValueExW(h: usize, name: *const u16, res: *mut u32, typ: *mut u32, data: *mut u8, len: *mut u32) -> i32;
            fn RegCloseKey(h: usize) -> i32;
        }
        unsafe {
            let mut key = 0usize;
            if RegOpenKeyExW(0x8000_0000u32 as usize /* HKEY_CURRENT_USER */, key_path.as_ptr(), 0, 0x20019 /* KEY_READ */, &mut key) != 0 {
                return None;
            }
            let mut ty = 0u32;
            let mut buf = vec![0u8; 1024];
            let mut len = buf.len() as u32;
            let ok = RegQueryValueExW(key, value_name.as_ptr(), std::ptr::null_mut(), &mut ty, buf.as_mut_ptr(), &mut len);
            let _ = RegCloseKey(key);
            if ok != 0 || len < 2 {
                return None;
            }
            let wide: Vec<u16> = buf[..(len as usize) & !1]
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            let s = std::ffi::OsString::from_wide(&wide);
            let expanded = expand_env(&s.to_string_lossy());
            let p = std::path::PathBuf::from(expanded.trim_end_matches('\0'));
            if p.is_absolute() { Some(p) } else { None }
        }
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from)
    }
}

#[cfg(windows)]
fn expand_env(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.char_indices().peekable();
    while let Some((_, ch)) = chars.next() {
        if ch == '%' {
            let mut name = String::new();
            let mut closed = false;
            let mut copy = chars.clone();
            while let Some((_, c2)) = copy.next() {
                if c2 == '%' {
                    chars = copy;
                    closed = true;
                    break;
                }
                name.push(c2);
            }
            if closed && !name.is_empty() {
                if let Ok(v) = std::env::var(&name) {
                    out.push_str(&v);
                    continue;
                }
            }
            out.push('%');
        }
        out.push(ch);
    }
    out
}
