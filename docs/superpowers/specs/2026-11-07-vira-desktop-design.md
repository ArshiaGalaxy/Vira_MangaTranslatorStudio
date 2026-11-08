# Vira Desktop â€” Tauri 2 Launcher & Packaging Design

Date: 2026-11-07
Status: Approved (direction fixed by the owner; "build and test the actual solution")

## Intent

Give Vira Manga Translator Studio a professional Windows desktop experience:
**Install â†’ Open Vira â†’ Click Start â†’ Use the existing studio.** Users never
see Python, Node, Django, ports, or virtualenvs. Existing components are
untouched and treated as black boxes.

## Ground truth (measured, not assumed)

- Django (`backend/`, port 127.0.0.1:8000 dev) embeds the translation engine in-process (`backend/studio_engine/`) â†’ only **two processes** to manage (Django + Next).
- Frontend (`frontend/`, Next 15, port 3000 dev) proxies `/api/*` and `/media/*`
  to Django via `next.config.ts` rewrites. **`BACKEND_URL` is evaluated at build
  time and baked into `.next/routes-manifest.json`** (no middleware file).
- Desktop mode already exists in `backend/settings.py`, keyed on `STUDIO_DESKTOP=1`:
  DB + media relocate to `%LOCALAPPDATA%\Vira Translator Studio`; fonts via
  `STUDIO_FONTS_DIR`; CORS/CSRF origins include `STUDIO_FRONTEND_PORT`;
  `DJANGO_SECRET_KEY` required when `DEBUG != 1`.
- Prior art: `desktop-packaging/` (PyInstaller + pywebview + Inno) proves the
  env-var contract: `STUDIO_DESKTOP, STUDIO_BACKEND_PORT, STUDIO_FRONTEND_PORT,
  STUDIO_DATA_DIR, DJANGO_SECRET_KEY, BACKEND_URL, DJANGO_DEBUG, PYTHONUNBUFFERED`.
- Deps: `requirements-core.txt` (light, pip wheels ~150MB total) and
  `requirements-ai.txt` (torch 2.2.2+cpu, paddleocr â€” ~600MB+, CPU-only).
  Python 3.10â€“3.12 (venv here: 3.10.11). Node â‰¥ 18.18.
- Machine state: no Rust, no MSVC â†’ use `x86_64-pc-windows-gnu` toolchain.

## Layout

```
desktop/                       â† new, self-contained layer
â”œâ”€â”€ src-tauri/                 Rust crate: launcher logic
â”‚   â”œâ”€â”€ src/                   paths, ports, runtime setup, process manager,
â”‚   â”‚                          health polling, AI installer, commands, events
â”‚   â”œâ”€â”€ ui/                    launcher/bootstrap window (plain HTML+JS, no framework)
â”‚   â”œâ”€â”€ icons/                 generated app icons
â”‚   â”œâ”€â”€ tauri.conf.json        per-user NSIS install, WebView2 bootstrapper
â”‚   â””â”€â”€ capabilities/
â”œâ”€â”€ scripts/
â”‚   â”œâ”€â”€ build-app.ps1          full packaging pipeline (payload â†’ installer)
â”‚   â””â”€â”€ dev.ps1                run launcher against the source tree (no payload)
â””â”€â”€ payload/                   staged at build time (gitignored)
    â”œâ”€â”€ backend/               Django + engine source (no .venv/db/media)
    â”œâ”€â”€ frontend/              Next standalone output (+ static, public)
    â”œâ”€â”€ fonts/                 Persian fonts
    â”œâ”€â”€ runtimes/python/       Python 3.10.11 embeddable (unpacked, ._pth patched)
    â”œâ”€â”€ runtimes/node/         node.exe (LTS win-x64)
    â””â”€â”€ wheels/                requirements-core wheels for cp310 win_amd64
```

## Behaviour

### Run model
- Single instance (tauri-plugin-single-instance).
- Launcher window shows the bootstrap: ensure data dirs â†’ verify runtimes â†’
  pip install core from bundled wheels (offline, `--target` into
  `%LOCALAPPDATA%\Vira\runtime\site-packages`) â†’ `migrate --noinput` â€”
  all idempotent, with markers so *working environments are never recreated*.
- Restore previous settings (API keys live in the desktop DB already).
- Allocate free ports (backend default 8642, frontend 3142); **patch
  `<standalone>/.next/routes-manifest.json` rewrite destinations to the actual
  backend port** before spawning Next (transparently robust to collisions).
- Start Django: `<install>\runtimes\python\python.exe -u manage.py runserver
  127.0.0.1:<bport> --noreload` with the proven env-var contract + `STUDIO_DESKTOP=1`,
  `DJANGO_DEBUG=0`, per-install persisted `DJANGO_SECRET_KEY`
  (`%LOCALAPPDATA%\Vira\secret_key.txt`), `PYTHONPATH` = backend + site-packages.
- Health poll `/api/fonts` (200/404 ok) with process-exit early-fail;
  then Next via `node server.js` (standalone, `.next/static` + `public` shipped);
  then poll `/`.
- On failure: show real error + log tail + Open-logs button; never a dead window.
- Window closes â†’ children killed by PID tree (`taskkill /T /F` on tracked PIDs only).
  Crash of the launcher â†’ children outlive single instance guard... (mitigated by
  --noreload + dedicated processes; hold: also use Windows Job Object per child
  so OS reaps them if Vira dies).

### Data
- User data: `%LOCALAPPDATA%\Vira Translator Studio\` (db.sqlite3, media/, secret_key.txt) â€” never deleted by installer.
- Rebuildable state: `%LOCALAPPDATA%\Vira\` (logs, site-packages, pip cache) â€” safe to wipe for repair.
- Installer (per-user, NSIS currentUser): uninstall removes only install dir.

### AI extras (hybrid decision)
- Core works offline from bundled wheels.
- "AI support" button in launcher settings installs `requirements-ai.txt`
  on demand from the network with streamed progress (`pip install --extra-index-url
  https://download.pytorch.org/whl/cpu`). Detection = marker + import probe.
- PaddleOCR/LaMa models download to `~/.paddleocr`, `~/.simple-lama` by the libs
  (same as today); already-present models are never replaced.

### Installer
- Tauri NSIS, `installMode: currentUser` â†’ `%LOCALAPPDATA%\Programs\Vira`,
  no admin rights. All payload (backend, frontend build, fonts, runtimes,
  wheels) shipped as resources â€” no network needed for core install.
- WebView2: `downloadBootstrapper` fallback for machines without it.
- Updates out of scope for v1 (backend has an update-check API; revisit later).

## Open calls I made (owner can veto)
- Ports 8642/3142 (uncommon) instead of 8000/3000 to cut collision risk.
- Pillow-generated glyph icon (repo has none) â€” placeholder good enough, replaceable.
- GNU toolchain now; switch to MSVC if it blocks (Tauri 2 supports both).
- Frontend runs as production standalone (not `next dev`): faster, stable, memory-light.
  Same UI, zero frontend code changes; dev.bat remains the dev-mode path.
- Standalone `routes-manifest.json` patching replaced runtime env `BACKEND_URL`
  (which Next ignores post-build â€” old launcher's port bump was dead code).

## Task checklist
[x] Recon + design doc
[x] Scaffold Tauri crate + launcher UI
[x] Implement paths/config/ports/runtime/process/health modules
[x] Implement commands + events + windowed flow incl. error UX
[x] Icon generation
[x] dev.ps1 run against source tree (real smoke test — bootstrap, offline wheels, Django+Next health, navigate to studio, close kills only spawned children)
[x] build-app.ps1: payload assembly + wheels + node/python fetch + next standalone build
[x] cargo tauri build -> NSIS installer (Vira_1.0.0_x64-setup.exe, 132 MiB; fixed invalid category -> GraphicsAndDesign)
[x] End-to-end test of installed app + uninstall + data-preservation check (silent /S install, first-run offline env 44s, full start healthy, reinstall over leftovers 20s with env reuse, 2.6s subsequent launch, uninstall preserves db.sqlite3 + secret_key)
