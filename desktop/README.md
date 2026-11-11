# Vira Desktop (Tauri + NSIS)

The Windows desktop app is a Tauri 2 shell that launches the bundled Django backend and the Next.js standalone frontend. It bundles portable Python and Node runtimes plus offline core wheels — end users install **no** Python/Node/dev tools.

## Prerequisites (build machine)

- **Windows 10/11 x64** (NSIS target)
- **Rust toolchain** (stable) + Cargo — <https://rustup.rs/>
- **MSVC Build Tools** (C++ workload) — required by Tauri on Windows
- **Node.js 18.18+** and npm
- A working **`backend\.venv`** (Python 3.10 recommended, matching the bundled runtime) — `desktop/scripts/build-app.ps1` uses it to fetch/build wheels and generate icons.
- Network access the first time you stage runtimes/wheels (Python embeddable, Node zip, get-pip, PyPI wheels). Subsequent builds reuse `desktop/.cache/`.

> `scripts/mingw-env.ps1` locates/activates GNU binutils (`as`/`dlltool`/`windres`) for the Rust build when MSVC/MinGW setup needs it. Run it in your shell or let `build-app.ps1` source it.

## Build the installer

From the repo root (PowerShell):

```powershell
# 1) Stage the self-contained payload (backend + frontend standalone + Python/Node + core wheels)
powershell -ExecutionPolicy Bypass -File desktop\scripts\build-app.ps1

# The script stages payload, then runs: npx @tauri-apps/cli@2 build
# Output: desktop\src-tauri\target\release\bundle\nsis\Vira_1.0.1_x64-setup.exe
```

Useful flags (see the script header):
- `-SkipPayload` — rebuild the NSIS bundle reusing an already-staged `desktop/payload/`
- `-SkipBundle` — stage payload only

What `build-app.ps1` does (read it before changing): copies backend source + fonts (excluding `.venv`/media/db), builds Next.js in `STUDIO_DESKTOP=1` standalone mode with `BACKEND_URL` baked to the desktop backend port, assembles the standalone tree (`server.js` + `.next/static` + `public`), bundles Python 3.10.11 embeddable + `node.exe` 22.14.0, builds/downloads core wheels for offline first-run install, and writes `get-pip.py`. Build outputs (`desktop/payload/`, `.cache/`, `src-tauri/target/`) are gitignored.

## Dev loop

```powershell
powershell -ExecutionPolicy Bypass -File desktop\scripts\dev.ps1
```

## Icons

```powershell
backend\.venv\Scripts\python.exe desktop\scripts\make_icons.py
# then regenerate platform icons if needed:
cd desktop\src-tauri
npx @tauri-apps/cli@2 icon icons\base-1024.png --output icons
```

## Release checklist

1. **Bump versions (keep in sync):** `desktop/src-tauri/tauri.conf.json` (`version`), `desktop/src-tauri/Cargo.toml` (`version`), `backend/studio/update_system.py` (`CURRENT_VERSION`), `frontend/package.json` (+ lockfile root `version` fields), and the README installer filename.
2. **Build** a clean installer (`build-app.ps1` without `-SkipPayload`).
3. **Compute hash/size** of the produced `*_x64-setup.exe`:
   ```powershell
   Get-FileHash .\desktop\src-tauri\target\release\bundle\nsis\Vira_<ver>_x64-setup.exe -Algorithm SHA256
   (Get-Item ...\Vira_<ver>_x64-setup.exe).Length
   ```
4. **Update `desktop/version-manifest.json`** (the auto-updater reads it from the repo's default branch): set `version`, `download_url` (`https://github.com/ArshiaGalaxy/Vira_MangaTranslatorStudio/releases/download/v<ver>/Vira_<ver>_x64-setup.exe`), `sha256`, `file_size`, `published_at`, `release_notes`, `minimum_version`, `mandatory`, `platform` (`windows-x64`). Commit it.
5. **Tag and publish the GitHub Release** `v<ver>` and upload the installer asset (filename must match `download_url`).
6. **Verify the updater**: `https://raw.githubusercontent.com/ArshiaGalaxy/Vira_MangaTranslatorStudio/master/desktop/version-manifest.json` returns the new manifest JSON.

User data lives in `%LOCALAPPDATA%\Vira Translator Studio` and is never removed by updates or uninstall — the manifest/update flow must preserve that behavior.
