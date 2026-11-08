# Production Readiness & Public Release Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prepare Vira MangaTranslatorStudio for public GitHub release under MIT — standardized repo identity, clean dependency installs for outsiders, no broken/misleading UI, protected credentials, synchronized versions, professional repo docs, and a working Windows installer built from the existing Tauri architecture.

**Architecture:** No architectural changes. Work happens on a new `production-readiness` branch, preserving all current uncommitted work as the first commit. Changes are localized: URL/text replacements, lockfile regeneration, deletion of a dead frontend auth layer, API-response masking in the existing Django config endpoint, version string sync, documentation/templates, and an end-to-end desktop installer build using the existing `desktop/scripts/build-app.ps1` + Tauri NSIS pipeline.

**Tech Stack:** Django 5.2 backend (`backend/`), Next.js 15 frontend (`frontend/`), Tauri 2 desktop shell (`desktop/src-tauri`), PowerShell build scripts (`desktop/scripts/`), Git/GitHub.

**Spec:** `docs/superpowers/specs/2026-11-08-production-readiness-design.md` (owner-provided; every requirement below cites it by section number)

## Global Constraints (verbatim from the spec)

- Repository name: `Vira_MangaTranslatorStudio`; URL: `https://github.com/ArshiaGalaxy/Vira_MangaTranslatorStudio` (§1)
- Lockfile must resolve exclusively against `https://registry.npmjs.org/` (§2)
- Preserve the entire `desktop/` directory and all Tauri-related files; no essential file may be lost (§3)
- No automated tests will be added or restored (Constraints) — verification is via lint/typecheck/build/manual commands, not new test files
- Do not modify LAN functionality or address LAN-related issues (Constraints)
- Do not modify, remove, or replace showcase images/manga pages — they are AI-generated (Constraints)
- Do not remove existing application features (Constraints)
- Keep Tauri + Django + Next.js architecture; no rewrites of working components (§9)
- No mandatory paid APIs or cloud services (§9)
- Version everywhere: `1.0.0` (matches `README*.md` installer name `Vira_1.0.0_x64-setup.exe`, §6)
- Users must not install Python/Node manually; services start automatically; Start Menu + desktop shortcuts + proper uninstall (§8)
- One `production-readiness` branch; commit after each task; no destructive ops without preservation (§10)
- Final deliverable includes a detailed report of completed changes and remaining issues (§10)

## Review Focus

Conditions the spec implies but no command in the plan fully proves on its own; each is pinned to a task step:

1. **Stored API keys must survive the settings round-trip after masking** — a user opens Settings, saves without retyping the key, and the real key must still work. Pinned in Task 5's verification (shell-based GET→PUT→re-read with a sentinel key).
2. **A third-party machine must be able to `npm ci` from a clean clone** — no Liara URLs, no machine-level npmrc dependency. Pinned in Task 3: lockfile grep = 0 Liara matches + fresh `npm ci` + lint + typecheck.
3. **`pip install -r requirements.txt` must resolve with the PyMuPDF 1.28.2 fix preserved** — the old freeze pins `pymupdf4llm==2.0`/`pymupdf_layout==2.0` which may require PyMuPDF 2.x; both packages are imported nowhere in the codebase (only `fitz`). Pinned in Task 6: `pip install --dry-run -r requirements.txt` resolves; if the pair conflicts with PyMuPDF==1.28.2, they are dropped (unused, not a feature removal).
4. **The installed app must run on a machine without Python/Node** — payload runtimes must be what the app executes, not system interpreters. Pinned in Task 8: inspect `build-app.ps1` runtime wiring, then smoke-test the silently-installed app.
5. **The auto-updater must reach its manifest after the rename** — correct repo, correct branch (`master`, the default; there is no `desktop` branch), and NSIS-compatible silent flags (current code uses Inno Setup's `/VERYSILENT`, which NSIS ignores). Pinned in Task 2 (URL) and Task 8 (flags + manifest file + curl-able URL pattern).
6. **Auth removal must not leave pages stuck on their loading gates** — `settings/page.tsx:473` and `projects/[id]/page.tsx:1144` gate rendering on `authLoading`. Pinned in Task 4: typecheck + dev-server page render.

---

## File Structure

| Path | Change |
|---|---|
| `.gitignore` | Modify — add `.freebuff/`, `backend/staticfiles/`, `Thumbs.db`, `.DS_Store` |
| `README.md`, `README.en.md`, `CONTRIBUTING.md` | Modify — repo URLs, stale captions, docs accuracy, dev-setup content |
| `backend/studio/update_system.py` | Modify — manifest URL (repo + branch), NSIS silent flags |
| `frontend/package-lock.json` | Regenerate — npmjs.org only |
| `frontend/lib/auth.tsx` | Delete — phantom auth layer |
| `frontend/app/layout.tsx`, `app/projects/page.tsx`, `app/projects/[id]/page.tsx`, `app/settings/page.tsx`, `lib/i18n.tsx` | Modify — remove auth wiring/dead copy |
| `backend/studio/services.py`, `backend/studio/views.py` | Modify — mask API keys on config read, preserve on masked write |
| `frontend/app/settings/page.tsx` | Modify — api_key inputs show/round-trip the mask |
| `frontend/package.json` + lockfile | Modify — version 0.1.0 → 1.0.0 |
| `backend/requirements.txt` | Regenerate — core + ai pins as source of truth |
| `desktop/README.md` | Create — installer build + release checklist |
| `desktop/version-manifest.json` | Create — updater manifest (real hash after build) |
| `.github/ISSUE_TEMPLATE/bug_report.yml`, `feature_request.yml`, `config.yml`, `.github/pull_request_template.md` | Create |
| `docs/superpowers/specs/2026-11-08-production-readiness-design.md` | Created (spec, already committed in Task 1) |

Untouched by design: `start.bat`, `start-lan.bat`, all showcase images, OCR/translation/inpainting/typesetting code, Tauri architecture, auth-free Django settings.

---

### Task 1: Preserve all work and create the `production-readiness` branch

**Files:**
- Modify: `.gitignore`
- Stage: everything currently modified/untracked (`desktop/`, `backend/studio/update_system.py`, README edits, settings UI, `docs/`)

**Interfaces:**
- Produces: a clean working tree on branch `production-readiness`; all subsequent tasks assume `git status --short` is empty at their start and end.

- [ ] **Step 1: Harden `.gitignore`**

Append, keeping the existing sections: `.freebuff/`, `backend/staticfiles/`, `Thumbs.db`, `.DS_Store`. (`.freebuff/` is currently ignored only by the local-only `.git/info/exclude`, which a fresh clone does not have.)

- [ ] **Step 2: Create the branch before any edit**

```powershell
git checkout -b production-readiness
```

- [ ] **Step 3: Stage everything and audit what gets staged**

```powershell
git add -A
git status --porcelain | Select-String "payload|\.cache|target/"   # Expected: no matches
git diff --cached --stat | Select-Object -Last 1                   # Expected: ~78 desktop files, total well under 5 MB
```

If any match in the first check appears, unstage it and extend `.gitignore` — `desktop/payload/` (254 MB), `desktop/.cache/` (816 MB), and `desktop/src-tauri/target/` (4.4 GB) must never be staged.

- [ ] **Step 4: Commit the preservation baseline**

```powershell
git commit -m "chore: preserve working state - desktop app, auto-updater, desktop-mode settings, docs"
```

- [ ] **Step 5: Verify nothing was lost**

```powershell
git status --short                                   # Expected: empty
git ls-files desktop | Measure-Object -Count         # Expected: ~78 tracked files
git ls-files | Select-String "\.env"                 # Expected: no matches
```

---

### Task 2: Standardize repository identity and updater URL (§1)

**Files:**
- Modify: `README.md` (lines ≈72, 88–89, 104–105, 228), `README.en.md` (lines ≈72, 88–89, 104–105, 216), `CONTRIBUTING.md` (lines ≈9, 30, 39)
- Modify: `backend/studio/update_system.py:43-46`
- Config: `git remote set-url origin …`

**Interfaces:**
- Produces: canonical string `https://github.com/ArshiaGalaxy/Vira_MangaTranslatorStudio` used by Task 8's manifest and by all docs.

- [ ] **Step 1: Replace the old repo name everywhere**

In `README.md`, `README.en.md`, `CONTRIBUTING.md`: replace `Vira_TranslatorStudio` → `Vira_MangaTranslatorStudio` (covers both the clone URLs and `cd Vira_TranslatorStudio` lines). Do NOT touch strings that already read `Vira_MangaTranslatorStudio`.

- [ ] **Step 2: Fix the updater manifest URL and its branch**

In `backend/studio/update_system.py:43-46` set the default to the official repo **and the `master` branch** (the repo's default; the current path implies a `desktop` branch that does not exist):

```python
DEFAULT_MANIFEST_URL = (
    "https://raw.githubusercontent.com/ArshiaGalaxy/"
    "Vira_MangaTranslatorStudio/master/desktop/version-manifest.json"
)
```

- [ ] **Step 3: Point the git remote at the official repo**

```powershell
git remote set-url origin https://github.com/ArshiaGalaxy/Vira_MangaTranslatorStudio.git
git remote -v   # Expected: single origin with the new URL
```

- [ ] **Step 4: Verify zero stale references**

```powershell
Get-ChildItem -Recurse -Include *.md,*.py,*.ts,*.tsx,*.json,*.bat -File |
  Select-String -Pattern "Vira_TranslatorStudio" |
  Where-Object { $_.Path -notmatch "\.git\\|node_modules|\.next|target" }
# Expected: no matches (Vira_MangaTranslatorStudio does not match this pattern)
```

- [ ] **Step 5: Backend still boots**

```powershell
cd backend; python manage.py check
```
Expected: no issues. Then commit: `docs: standardize repository name and fix updater manifest URL`

---

### Task 3: Regenerate the npm lockfile against registry.npmjs.org (§2)

**Files:**
- Regenerate: `frontend/package-lock.json`

**Interfaces:**
- Consumes: machine global npm config points at `https://package-mirror.liara.ir/repository/npm/` — every npm command in this task must pass `--registry=https://registry.npmjs.org/` explicitly.

- [ ] **Step 1: Regenerate the lockfile with the official registry**

```powershell
cd frontend
Remove-Item package-lock.json
npm install --package-lock-only --registry=https://registry.npmjs.org/
```

- [ ] **Step 2: Verify no mirror references remain**

```powershell
(Select-String package-lock.json -Pattern "liara" | Measure-Object).Count   # Expected: 0
(Select-String package-lock.json -Pattern "registry.npmjs.org" | Measure-Object).Count  # Expected: > 0
(Select-String package-lock.json -Pattern "package-mirror" | Measure-Object).Count      # Expected: 0
```

- [ ] **Step 3: Prove a clean install works (Review Focus #2)**

```powershell
npm ci --registry=https://registry.npmjs.org/
npm run lint
npm run typecheck
```
Expected: all succeed. (The lockfile's `resolved` URLs now point at npmjs, so third parties need no flag.)

- [ ] **Step 4: Commit**

```powershell
git add frontend/package-lock.json
git commit -m "fix: regenerate package-lock.json against registry.npmjs.org"
```

---

### Task 4: Remove the phantom authentication layer (§4)

**Files:**
- Delete: `frontend/lib/auth.tsx`
- Modify: `frontend/app/layout.tsx` (import line 4, `<AuthProvider>` line ≈55)
- Modify: `frontend/app/projects/page.tsx` (import line 8, `useAuth` line 26, sign-out `IconBtn` line ≈122, `LogOut` import if now unused)
- Modify: `frontend/app/projects/[id]/page.tsx` (import line 31, `useAuth` line 64, `if (authLoading)` gate line ≈1144 — keep the gate's sibling data-loading behavior, drop only the auth condition)
- Modify: `frontend/app/settings/page.tsx` (import line 25, `useAuth` line 257, gate line ≈473 → `if (!config)`)
- Modify: `frontend/lib/i18n.tsx` — remove dead keys: `nav.signOut` (line 22), `login.tagline`/`login.welcome`/`login.subtitle`/`login.username`/`login.password`/`login.submit`/`login.submitting` (lines ≈243, 260–265), `toast.loginFailed` (line 769)
- Modify: `README.md:219` and `README.en.md:207` — the troubleshooting row claiming `/media` requires sign-in

**Interfaces:**
- Produces: `frontend` with no `@/lib/auth` import anywhere; pages render without auth context.

- [ ] **Step 1: Map every consumer before deleting**

```powershell
Get-ChildItem frontend -Recurse -Include *.ts,*.tsx -File | Select-String "useAuth|AuthProvider|/api/auth|nav\.signOut|login\."
```
Expected afterwards: no matches. Use this list as the work order.

- [ ] **Step 2: Remove auth wiring from the four pages + layout, then delete `frontend/lib/auth.tsx`**

Leave all non-auth loading states intact (e.g. keep `if (!config)`); remove only conditions sourced from `useAuth`.

- [ ] **Step 3: Remove dead i18n keys** listed above.

- [ ] **Step 4: Fix the misleading docs**

In `README.md:219` / `README.en.md:207`, replace the "signed-in users" claim with the truth (files under `media/` are served to any local client; the app has no accounts — settings are stored per-machine). Keep the row's troubleshooting value or drop the row; do not add new features.

- [ ] **Step 5: Verify (Review Focus #6)**

```powershell
Get-ChildItem frontend -Recurse -Include *.ts,*.tsx -File | Select-String "useAuth|AuthProvider|/api/auth"  # Expected: 0 matches
cd frontend; npm run lint; npm run typecheck                                                                  # Expected: pass
```
Then `npm run dev`, open `/`, `/projects`, `/settings` — all render; no Log out button anywhere.

- [ ] **Step 6: Commit** — `refactor: remove non-functional auth layer and fix login references in docs`

---

### Task 5: Protect stored API keys in config responses (§5)

**Files:**
- Modify: `backend/studio/views.py` (GET config handler ≈line 571) and/or `backend/studio/services.py` (`get_global_config()` ≈284–296, update whitelist ≈91–93, update path ≈272–340)
- Modify: `frontend/app/settings/page.tsx` (api_key inputs for openrouter/gemini/custom — round-trip the mask)

**Interfaces:**
- Produces: mask constant (use the literal `"********"`) honored by both backend read/write and the Settings UI.
- Behavior contract: **GET** `/api/config` returns `"********"` in place of any non-empty `translator.{openrouter,gemini,custom}.api_key` (empty stays `""`); **PUT/POST** of a config where an `api_key` equals `"********"` leaves the stored value unchanged; any other value overwrites it as today. The mask must never be persisted to `studio_engine/config.json`.

- [ ] **Step 1: Read the exact read/write paths first**

Read `services.py` around lines 85–100 (whitelist), 272–340 (update logic incl. the existing "must not clobber" comment) and `views.py` 565–600. The mask goes at the API boundary — `comic_translator/config.py` and `config.json` stay untouched.

- [ ] **Step 2: Implement masking on read and mask-awareness on write**

Backend: in the GET path, replace each non-empty `api_key` with `"********"`; in the update path, drop fields whose value is `"********"` before merging (so stored keys survive). Frontend: ensure the api_key inputs simply carry the value through (they will show `********`; typing replaces it) — no new UI elements.

- [ ] **Step 3: Verify the round-trip with a sentinel key (Review Focus #1)**

```powershell
cd backend
python manage.py shell
```
```python
from studio import services
from django.test import Client
services.update_global_config({"translator": {"openrouter": {"model": "test-model", "api_key": "sk-SENTINEL-REAL"}}})
c = Client()
r = c.get("/api/config").json()
assert r["translator"]["openrouter"]["api_key"] == "********", r["translator"]["openrouter"]
c.put("/api/config", data=json.dumps(r), content_type="application/json")
assert services.get_global_config()["translator"]["openrouter"]["api_key"] == "sk-SENTINEL-REAL"   # not clobbered
# restore original config afterwards (re-read it before overwriting)
```
Also confirm `backend/studio_engine/config.json` contains no `"********"` after the PUT. Note: `manage.py shell` needs `import json`.

- [ ] **Step 4: Lint/typecheck + Settings page save works end-to-end**

`npm run lint && npm run typecheck`; manually: Settings → change a non-key field → Save → reopen → key still functional (masked), model change persisted.

- [ ] **Step 5: Commit** — `security: mask provider API keys in config API responses`

---

### Task 6: Synchronize versions and fix `requirements.txt` (§6)

**Files:**
- Modify: `frontend/package.json:3` (`0.1.0` → `1.0.0`), `frontend/package-lock.json` (root `version` fields ≈lines 3 and 9)
- Regenerate: `backend/requirements.txt`

**Interfaces:**
- Consumes: `backend/requirements-core.txt` (includes the pending fix `PyMuPDF==1.28.2`) and `backend/requirements-ai.txt` are the source of truth for any package they share.

- [ ] **Step 1: Version sync**

Set `frontend/package.json` version to `1.0.0` and update the lockfile's two root `version` fields to match (do not re-run npm install for this; hand-edit both lockfile occurrences). Verify the full set:

```powershell
# Expected: every one prints 1.0.0
(Get-Content frontend\package.json | ConvertFrom-Json).version
Select-String frontend\package-lock.json -Pattern '"version": "1.0.0"' | Select-Object -First 2
Select-String desktop\src-tauri\tauri.conf.json -Pattern '"version"'
Select-String desktop\src-tauri\Cargo.toml -Pattern '^version'
Select-String backend\studio\update_system.py -Pattern 'CURRENT_VERSION'
```

- [ ] **Step 2: Rebuild `backend/requirements.txt`**

Rules, in precedence order:
1. Every pin from `requirements-core.txt` verbatim (thus `PyMuPDF==1.28.2`, `psd-tools==1.8.24` override the old freeze's `pymupdf==2.0` / `psd-tools==1.8.32`).
2. Every pin from `requirements-ai.txt` verbatim (including the `--extra-index-url` line, placed under a comment header).
3. All remaining packages from the old freeze that neither file lists, unchanged.
4. `pymupdf4llm==2.0` and `pymupdf_layout==2.0`: keep only if Step 3 resolves with `PyMuPDF==1.28.2`; if pip reports a conflict, drop both — they are imported nowhere in the codebase (verified: only `fitz`/PyMuPDF is used), so this removes no feature.
5. Keep the two file-header comments explaining Core vs Optional AI tiers.

- [ ] **Step 3: Verify pip resolution (Review Focus #3)**

```powershell
cd backend
python -m venv .venv-reqcheck
.venv-reqcheck\Scripts\pip install --dry-run -r requirements.txt
```
Expected: "Would install …" with no `ERROR: Resolution Impossible`. Delete `.venv-reqcheck` afterwards (add to nothing — it matches existing `.venv/`? no: ensure deletion).

- [ ] **Step 4: Commit** — `chore: sync versions to 1.0.0 and regenerate requirements.txt from core+ai pins`

---

### Task 7: Repository presentation — README, desktop build guide, CONTRIBUTING, issue templates (§7)

**Files:**
- Create: `desktop/README.md`
- Create: `.github/ISSUE_TEMPLATE/bug_report.yml`, `.github/ISSUE_TEMPLATE/feature_request.yml`, `.github/ISSUE_TEMPLATE/config.yml`, `.github/pull_request_template.md`
- Modify: `README.md`, `README.en.md`, `CONTRIBUTING.md`

**Interfaces:**
- Consumes: Task 2's canonical URLs, Task 8's build procedure (write `desktop/README.md` from `desktop/scripts/build-app.ps1` — read it first, document what it actually does).

- [ ] **Step 1: Create `desktop/README.md`**

Content (bilingual not required; English): prerequisites (Rust toolchain, MSVC build tools, Node, backend `.venv`), how to stage the payload (`scripts/build-app.ps1`, usage comment at its lines 7–8), `mingw-env.ps1` note, how to build the installer (`npx @tauri-apps/cli build` → NSIS output under `desktop/src-tauri/target/release/bundle/nsis/`), icon regeneration (`scripts/make_icons.py`), dev loop (`scripts/dev.ps1`), and a **Release checklist**: bump the four version locations (Task 6 list) → build → compute SHA-256 of the installer → update `desktop/version-manifest.json` (version, download_url `https://github.com/ArshiaGalaxy/Vira_MangaTranslatorStudio/releases/download/vX.Y.Z/Vira_X.Y.Z_x64-setup.exe`, sha256, size) → tag `vX.Y.Z` → upload installer as the release asset.

- [ ] **Step 2: Polish both READMEs**

- Fix stale screenshot captions (`README.md:19`, `README.en.md:19` — the real screenshots at lines 17/37 already exist; remove "real screenshot goes here" phrasing in both languages).
- Add one line under "Option 0" linking to `desktop/README.md` ("want to build the installer yourself?").
- Keep both languages structurally in sync (every EN change gets its FA counterpart and vice versa).
- Do not touch showcase images, LAN sections, or the AI tiers.

- [ ] **Step 3: Improve `CONTRIBUTING.md`**

Fork-based workflow (replace "create a branch on this repo"), dev setup commands (backend: venv + `pip install -r requirements-core.txt` + `manage.py migrate` + `runserver`; frontend: `npm ci` + `npm run lint` + `npm run typecheck`), note that the project currently has no automated test suite (do not promise tests), keep the existing bilingual FA/EN structure.

- [ ] **Step 4: Create issue templates**

`bug_report.yml`: OS, install path (installer vs from source), Python/Node versions (source only), steps, expected/actual, logs. `feature_request.yml`: problem, proposed solution, alternatives. `config.yml`: `blank_issues_enabled: true`. PR template: what/why/how-tested (manual steps).

- [ ] **Step 5: Verify docs integrity**

```powershell
# every referenced local file exists
Select-String README.md, README.en.md -Pattern '\]\((?!http)([^)#]+)' -AllMatches |
  ForEach-Object { $_.Matches } | ForEach-Object { $_.Groups[1].Value } | Sort-Object -Unique
# then check each path exists; also confirm no old repo name (Task 2 grep) and no "goes here" captions remain
```

- [ ] **Step 6: Commit** — `docs: add desktop build guide, issue templates, and README/CONTRIBUTING polish`

---

### Task 8: Build and verify the Windows installer + updater wiring (§8)

**Files:**
- Modify: `backend/studio/update_system.py:337-345` (`apply_update` silent flags)
- Create: `desktop/version-manifest.json`
- Read/verify: `desktop/scripts/build-app.ps1`, `desktop/src-tauri/tauri.conf.json`

**Interfaces:**
- Consumes: Task 2's manifest URL, Task 6's synchronized versions.
- Produces: `desktop/version-manifest.json` shape: `{"version", "download_url", "sha256", "release_notes", "published_at", "minimum_version", "mandatory", "file_size", "platform": "windows-x64"}` (fields per `VersionManifest`, `update_system.py:64-77`).

- [ ] **Step 1: Fix the silent-install flags for NSIS**

`apply_update` currently passes `/VERYSILENT /SUPPRESSMSGBOXES /NORESTART` (Inno Setup syntax). NSIS accepts `/S /NCRC /NORESTART`. Replace the flag list accordingly (keep `CREATE_NO_WINDOW`, keep exiting the app).

- [ ] **Step 2: Inspect the payload staging**

Read `build-app.ps1`: confirm it stages Python embed + Node + **core** wheels (not the multi-GB AI stack), Next standalone build, backend, and that the launched services use payload runtimes rather than system Python/Node (Review Focus #4). Note anything obviously oversized; optimize only obvious wins (deleted caches, excluded dev files) — no rewrites.

- [ ] **Step 3: Build the payload and the installer**

```powershell
cd desktop
powershell -ExecutionPolicy Bypass -File scripts\build-app.ps1    # stages ../payload (read its header for options)
cd src-tauri
npx @tauri-apps/cli build                                         # NSIS installer output: target\release\bundle\nsis\*_x64-setup.exe
```
Expected: `Vira_1.0.0_x64-setup.exe` produced. Record its size for the report (compare against the previous release if available).

- [ ] **Step 4: Silent-install and smoke-test (Review Focus #4)**

```powershell
& ".\target\release\bundle\nsis\Vira_1.0.0_x64-setup.exe" /S /NCRC /NORESTART
Start-Sleep 20
Get-Process Vira -ErrorAction SilentlyContinue     # Expected: running
# verify: Start Menu shortcut exists under %APPDATA%\Microsoft\Windows\Start Menu\Programs; desktop shortcut present?
# verify app window/backend up: the studio UI loads and a page renders
```
If the **desktop** shortcut is missing (Tauri NSIS creates Start Menu + uninstaller by default), add it via `tauri.conf.json` NSIS configuration / installer hooks — spec §8 requires both. Then test uninstall: locate `Uninstall Vira.exe` under the install dir (or Apps & Features), run `/S`, confirm the app and shortcuts are gone while `%LOCALAPPDATA%\Vira Translator Studio` data remains.

- [ ] **Step 5: Create `desktop/version-manifest.json`**

```powershell
(Get-FileHash ".\target\release\bundle\nsis\Vira_1.0.0_x64-setup.exe" -Algorithm SHA256).Hash
(Get-Item ".\target\release\bundle\nsis\Vira_1.0.0_x64-setup.exe").Length
```
Write the manifest with version `1.0.0`, the GitHub release download URL (`.../releases/download/v1.0.0/Vira_1.0.0_x64-setup.exe`), the real hash and size, `minimum_version: "0.0.0"`, `mandatory: false`, `platform: "windows-x64"`. Commit it at `desktop/version-manifest.json` — exactly where Task 2's URL points (on `master`).

- [ ] **Step 6: Verify updater wiring**

In a shell: `from studio.update_system import DEFAULT_MANIFEST_URL; print(DEFAULT_MANIFEST_URL)` → must equal `https://raw.githubusercontent.com/ArshiaGalaxy/Vira_MangaTranslatorStudio/master/desktop/version-manifest.json`. After push, the same URL must return the manifest JSON (post-push check, record for the report).

- [ ] **Step 7: Commit** — `feat: fix NSIS silent-update flags, add version manifest, desktop shortcut` (include any tauri.conf changes)

---

### Task 9: Final verification and report (§10)

- [ ] **Step 1: Run the full verification sweep**

```powershell
# Identity
Get-ChildItem -Recurse -Include *.md,*.py,*.ts,*.tsx,*.json -File | Select-String "Vira_TranslatorStudio" | Where-Object Path -notmatch "\.git\\|node_modules|target"  # 0
# Lockfile
Select-String frontend\package-lock.json -Pattern "liara|package-mirror"   # 0
# Auth
Get-ChildItem frontend -Recurse -Include *.ts,*.tsx -File | Select-String "/api/auth|useAuth|AuthProvider"  # 0
# Secrets
git ls-files | Select-String "\.env"                                        # 0
Get-ChildItem -Recurse -Include *.py,*.ts,*.tsx,*.json -File | Select-String "sk-or-[a-zA-Z0-9]{10}|AIza[a-zA-Z0-9_-]{20}" | Where-Object Path -notmatch "\.git\\|node_modules|README"  # 0
# Build health
cd frontend; npm ci; npm run lint; npm run typecheck                        # pass
cd ..\backend; python manage.py check                                       # no issues
# Versions all 1.0.0 (Task 6 Step 1 command)                                 # all 1.0.0
git status --short                                                           # empty
```

- [ ] **Step 2: Manual end-to-end pass**

From source: `start.bat` → dashboard → open a project → Settings save. Installer: app launches, services auto-start, uninstall clean. Showcase images untouched: `git status` shows no changes under `frontend/public/showcase/`. LAN files untouched: `git diff master -- start-lan.bat start.bat` empty.

- [ ] **Step 3: Write the final report**

Deliver to the owner: completed changes per spec section (1–10), verification results for each Review Focus item, installer size and startup observations, and remaining issues/decisions (e.g. GitHub repo must be created before push, release assets to upload, post-push manifest URL check).
