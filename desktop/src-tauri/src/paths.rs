//! Filesystem layout resolution + small file utilities.
//!
//! Layout (production):
//!   <install>\Vira.exe
//!   <install>\<resources dir>\payload\...       (bundled payload)
//!   %LOCALAPPDATA%\Vira Translator Studio\      (user data: db, media, secret)
//!   %LOCALAPPDATA%\Vira\                        (rebuildable: logs, site-packages)

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub const PAYLOAD_MARKER: &str = "backend/manage.py";

/// Locate the payload directory by probing candidates and checking for
/// `backend/manage.py`. Order matters: strict NSIS resource layout first,
/// then common variants, then the developer machine (checked out source tree).
pub fn find_payload(exe_dir: &Path, resource_dir: Option<&Path>) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(res) = resource_dir {
        candidates.push(res.join("payload"));
        candidates.push(res.join("_up_").join("payload"));
    }
    candidates.push(exe_dir.join("payload"));
    if let Some(parent) = exe_dir.parent() {
        candidates.push(parent.join("payload"));
        candidates.push(parent.parent().map(|p| p.join("payload")).unwrap_or_else(|| parent.to_path_buf()));
    }
    #[cfg(debug_assertions)]
    {
        // Dev runs: the cargo manifest dir is src-tauri, payload sits next to it.
        if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
            let manifest = PathBuf::from(manifest);
            candidates.push(manifest.parent().unwrap_or(&manifest).join("payload"));
        }
        // Optional explicit override for special setups.
        if let Ok(over) = std::env::var("VIRA_PAYLOAD_DIR") {
            candidates.push(PathBuf::from(over));
        }
    }
    for c in &candidates {
        if c.join(PAYLOAD_MARKER).is_file() {
            return Some(c.clone());
        }
    }
    if let Ok(over) = std::env::var("VIRA_PAYLOAD_DIR") {
        let p = PathBuf::from(over);
        if p.join(PAYLOAD_MARKER).is_file() {
            return Some(p);
        }
    }
    None
}

/// A stable per-process cache of resolved roots.
pub struct Roots {
    pub payload: PathBuf,
    pub data: PathBuf,      // %LOCALAPPDATA%\Vira Translator Studio
    pub vira_home: PathBuf, // %LOCALAPPDATA%\Vira
    pub logs: PathBuf,      // vira_home\logs
    pub site_packages: PathBuf, // vira_home\site-packages
}

pub fn user_data_dir() -> PathBuf {
    crate::state::dirs_local()
        .unwrap_or_else(|| std::env::temp_dir())
        .join("Vira Translator Studio")
}

pub fn vira_home_dir() -> PathBuf {
    crate::state::dirs_local()
        .unwrap_or_else(|| std::env::temp_dir())
        .join("Vira")
}

pub fn resolve_roots(payload: PathBuf) -> Roots {
    let data = user_data_dir();
    let vira_home = vira_home_dir();
    let logs = vira_home.join("logs");
    let site_packages = vira_home.join("site-packages");
    Roots { payload, data, vira_home, logs, site_packages }
}

static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();

pub fn init_log_file(logs_dir: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(logs_dir)?;
    let name = format!(
        "launcher-{}.log",
        chrono_now_filename()
    );
    let path = logs_dir.join(name);
    LOG_PATH.get_or_init(|| path.clone());
    Ok(path)
}

pub fn log_file_path() -> Option<PathBuf> {
    LOG_PATH.get().cloned()
}

pub fn append_log_line(line: &str) {
    if let Some(path) = log_file_path() {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            let _ = f.write_all(
                format!(
                    "{} {}\n",
                    humantime_now(),
                    line.replace('\n', " | ")
                )
                .as_bytes(),
            );
        }
    }
}

fn humantime_now() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    // Cheap UTC timestamp without chrono.
    let days = secs / 86400;
    let rem = secs % 86400;
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let (y, mo, d) = civil_from_days(days as i64);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

fn chrono_now_filename() -> String {
    humantime_now().replace(':', "-")
}

/// Howard Hinnant's days-to-civil conversion (UTC).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Generate/persist the Django secret key exactly like the previous launcher.
pub fn resolve_secret_key(data_dir: &Path) -> String {
    if let Ok(key) = std::env::var("DJANGO_SECRET_KEY") {
        if !key.is_empty() {
            return key;
        }
    }
    let file = data_dir.join("secret_key.txt");
    if file.is_file() {
        if let Ok(text) = std::fs::read_to_string(&file) {
            let key = text.trim().to_string();
            if !key.is_empty() {
                return key;
            }
        }
    }
    let key = random_urlsafe(64);
    let _ = std::fs::write(&file, &key);
    key
}

/// OS-random token (no extra crate): CryptGenRandom via advapi32.
pub fn random_urlsafe(len: usize) -> String {
    #[cfg(windows)]
    {
        #[repr(C)]
        struct Out { dw: u32 }
        extern "system" {
            fn CryptAcquireContextW(ph: *mut usize, psz: *const u16, prov: *const u16, typ: u32, flags: u32) -> i32;
            fn CryptGenRandom(h: usize, len: u32, buf: *mut u8) -> i32;
            fn CryptReleaseContext(h: usize, flags: u32) -> i32;
        }
        unsafe {
            let mut prov = 0usize;
            const PROV_RSA_FULL: u32 = 1;
            const CRYPT_VERIFYCONTEXT: u32 = 0xF000_0000;
            if CryptAcquireContextW(&mut prov, std::ptr::null(), std::ptr::null(), PROV_RSA_FULL, CRYPT_VERIFYCONTEXT) != 0 {
                let mut bytes = vec![0u8; len];
                if CryptGenRandom(prov, len as u32, bytes.as_mut_ptr()) != 0 {
                    let _ = CryptReleaseContext(prov, 0);
                    return bytes.iter().map(|b| URLSAFE[*b as usize % 64] as char).collect();
                }
                let _ = CryptReleaseContext(prov, 0);
            }
        }
    }
    // Fallback: process-id + time based pseudo entropy (never used on Windows normally).
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as u64
        ^ (std::process::id() as u64) << 32;
    (0..len)
        .map(|i| {
            let x = (seed.wrapping_mul(6364136223846793005).wrapping_add(i as u64 + 1442695)) >> 33;
            URLSAFE[(x % 64) as usize] as char
        })
        .collect()
}

const URLSAFE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Patch the Python runtime `python310._pth` so the embedded interpreter can
/// import from our site-packages dir and the packaged backend dir. Idempotent:
/// only rewrites when the content would change.
pub fn patch_python_pth(runtime_python_dir: &Path, site_packages: &Path, backend_dir: &Path) -> std::io::Result<()> {
    let pth = runtime_python_dir.join("python310._pth");
    let content = format!(
        // Relative paths (python310.zip, Lib\site-packages) resolve next to
        // the ._pth file, i.e. the runtime directory.
        "python310.zip\n.\nLib\\site-packages\n{}\n{}\nimport site\n",
        site_packages.display(),
        backend_dir.display()
    );
    let existing = std::fs::read_to_string(&pth).unwrap_or_default();
    if existing.replace("\r\n", "\n") != content {
        std::fs::write(&pth, content)?;
    }
    Ok(())
}

/// Patch the Next standalone `routes-manifest.json` so `/api/*` and `/media/*`
/// rewrites point at the *actual* backend port chosen at runtime. The baked
/// build-time origin is `http://127.0.0.1:<BAKED_BACKEND_PORT>`.
pub const BAKED_BACKEND_PORT: u16 = 8642;

pub fn patch_next_manifest(standalone: &Path, backend_port: u16) -> Result<usize, String> {
    let manifest = standalone.join(".next").join("routes-manifest.json");
    let text = std::fs::read_to_string(&manifest)
        .map_err(|e| format!("cannot read {}: {e}", manifest.display()))?;
    let baked = format!("http://127.0.0.1:{BAKED_BACKEND_PORT}");
    let actual = format!("http://127.0.0.1:{backend_port}");
    if !text.contains(&baked) {
        // Already patched or built with the runtime port: accept as-is when the
        // actual port is present, otherwise surface a clear error.
        if text.contains(&actual) {
            return Ok(0);
        }
        return Err(format!(
            "routes-manifest.json does not contain the expected backend origin ({baked}). \
             The frontend payload may have been built with a different port."
        ));
    }
    let patched = text.replace(&baked, &actual);
    let count = text.matches(&baked).count();
    std::fs::write(&manifest, patched)
        .map_err(|e| format!("cannot write {}: {e}", manifest.display()))?;
    Ok(count)
}
