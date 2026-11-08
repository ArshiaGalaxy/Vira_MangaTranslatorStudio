# Vira Desktop — full packaging pipeline.
#
# Stages a self-contained payload (Django backend + fonts + Next standalone +
# portable Python/Node runtimes + offline core wheels), then builds the Tauri
# NSIS installer.
#
# Usage:  powershell -ExecutionPolicy Bypass -File scripts\build-app.ps1
# Optional: -SkipPayload  (reuse an already-staged payload)

[CmdletBinding()]
param(
    [switch]$SkipPayload,
    [switch]$SkipBundle
)

$ErrorActionPreference = "Stop"
$stopwatch = [System.Diagnostics.Stopwatch]::StartNew()

function Stage($name) { Write-Host "`n=== $name ===" -ForegroundColor Cyan }
function Info($msg) { Write-Host "    $msg" -ForegroundColor DarkGray }

# pip download with an automatic mirror fallback: some networks block
# files.pythonhosted.org while pypi mirrors stay reachable. The fallback only
# affects where wheels are fetched from at build time — payload contents are
# identical.
function Invoke-PipDownload {
    param([string[]]$PipArgs, [string]$FailureMessage)
    & $venvPy -m pip @PipArgs
    if ($LASTEXITCODE -ne 0) {
        Info "PyPI file host unreachable - retrying via mirror (pypi.tuna.tsinghua.edu.cn)..."
        & $venvPy -m pip @PipArgs --index-url https://pypi.tuna.tsinghua.edu.cn/simple
        if ($LASTEXITCODE -ne 0) { throw $FailureMessage }
    }
}

$root      = Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $PSCommandPath))   # repo root
$desktop   = Join-Path $root "desktop"
$payload   = Join-Path $desktop "payload"
$cache     = Join-Path $desktop ".cache"
$srcTauri  = Join-Path $desktop "src-tauri"
$venvPy    = Join-Path $root "backend\.venv\Scripts\python.exe"

# Build-time toolchain: GNU binutils (as/dlltool/windres) for the Rust build.
. (Join-Path (Split-Path -Parent $PSCommandPath) "mingw-env.ps1")
Ensure-MingwBinutils

# Versions — treat like a lockfile: change deliberately, not casually.
$PythonVersion = "3.10.11"
$NodeVersion   = "22.14.0"
# The frontend build BAKES the backend origin into routes-manifest.json.
# This must match BAKED_BACKEND_PORT in desktop\src-tauri\src\paths.rs.
$BakedBackendPort = 8642

New-Item -ItemType Directory -Force -Path $cache | Out-Null

if (-not (Test-Path $venvPy)) {
    throw "backend\.venv not found at $venvPy — this pipeline uses it (Python 3.10) to fetch wheels."
}

# ── Icons ──────────────────────────────────────────────────────────────────────
if (-not (Test-Path (Join-Path $srcTauri "icons\icon.ico"))) {
    Stage "Icons"
    & $venvPy (Join-Path $desktop "scripts\make_icons.py")
    if (-not (Test-Path (Join-Path $srcTauri "icons\128x128.png"))) { throw "icon generation failed" }
    Push-Location $srcTauri
    & npx --yes @tauri-apps/cli@2 icon (Join-Path $srcTauri "icons\base-1024.png") "--output" (Join-Path $srcTauri "icons")
    Pop-Location
    if (-not (Test-Path (Join-Path $srcTauri "icons\icon.ico"))) { throw "tauri icon did not produce icon.ico" }
}

if (-not $SkipPayload) {
    # ── Stage payload ────────────────────────────────────────────────────────────
    Stage "Payload: clean staging dir"
    if (Test-Path $payload) { Remove-Item $payload -Recurse -Force }
    New-Item -ItemType Directory -Force -Path $payload | Out-Null

    Stage "Payload: Django backend source + fonts"
    $beDest = Join-Path $payload "backend"
    New-Item -ItemType Directory -Force -Path $beDest | Out-Null
    Get-ChildItem $root\backend -Force | Where-Object {
        $_.Name -notin @(".venv", "media", "staticfiles", "db.sqlite3", ".idea", "%USERPROFILE%") -and $_.Name -notlike "*__pycache__*"
    } | ForEach-Object {
        Copy-Item $_.FullName -Destination $beDest -Recurse -Force -Exclude *.pyc
    }
    # Remove any nested __pycache__ copies that slipped through.
    Get-ChildItem $beDest -Recurse -Directory -Force | Where-Object { $_.Name -like "*__pycache__*" } |
        Remove-Item -Recurse -Force
    if (-not (Test-Path "$beDest\manage.py")) { throw "backend staging failed (manage.py missing)" }
    if (-not (Test-Path "$beDest\requirements-core.txt")) { throw "requirements-core.txt missing from staging" }
    # Launcher-only helper for the resumable AI-package download (used by ai.rs).
    Copy-Item (Join-Path $desktop "scripts\ai_fetch.py") -Destination $beDest -Force
    if (-not (Test-Path "$beDest\ai_fetch.py")) { throw "ai_fetch.py staging failed" }

    Stage "Payload: Next.js frontend (production standalone)"
    Push-Location $root\frontend
    if (-not (Test-Path "node_modules")) {
        Info "installing frontend dependencies (first build on this machine)…"
        & npm install --no-audit --no-fund
        if ($LASTEXITCODE -ne 0) { Pop-Location; throw "npm install failed" }
    }
    Info "building Next.js standalone (STUDIO_DESKTOP=1, BACKEND_URL baked to port $BakedBackendPort)…"
    $env:STUDIO_DESKTOP = "1"
    $env:BACKEND_URL = "http://127.0.0.1:$BakedBackendPort"
    $env:NEXT_TELEMETRY_DISABLED = "1"
    & npm run build
    $env:STUDIO_DESKTOP = $null; $env:BACKEND_URL = $null
    if ($LASTEXITCODE -ne 0) { Pop-Location; throw "next build failed" }
    Pop-Location

    $standalone = Join-Path $root "frontend\.next\standalone"
    if (-not (Test-Path "$standalone\server.js")) { throw "standalone output missing: $standalone" }

    Info "assembling standalone tree (server + static + public)…"
    $feDest = Join-Path $payload "frontend\standalone"
    New-Item -ItemType Directory -Force -Path (Split-Path $feDest) | Out-Null
    Copy-Item $standalone -Destination $feDest -Recurse -Force
    Copy-Item $root\frontend\.next\static -Destination "$feDest\.next\static" -Recurse -Force
    if (Test-Path $root\frontend\public) {
        Copy-Item $root\frontend\public -Destination "$feDest\public" -Recurse -Force
    }

    Stage "Payload: Python runtime (embeddable $PythonVersion)"
    $pyZip = Join-Path $cache "python-$PythonVersion-embed-amd64.zip"
    if (-not (Test-Path $pyZip)) {
        Info "downloading python embeddable zip…"
        Invoke-WebRequest -UseBasicParsing "https://www.python.org/ftp/python/$PythonVersion/python-$PythonVersion-embed-amd64.zip" -OutFile $pyZip
    }
    New-Item -ItemType Directory -Force -Path "$payload\runtimes\python" | Out-Null
    Expand-Archive $pyZip "$payload\runtimes\python" -Force
    # The launcher rewrites python310._pth at runtime; make sure a stock copy exists.
    if (-not (Test-Path "$payload\runtimes\python\python310._pth")) { throw "python310._pth missing after expand" }

    # Defensive: the embeddable distribution sometimes lacks the ssl module.
    # AI dependency installation needs TLS, so harvest the modules from the
    # same-version venv if the embedded copy cannot import ssl.
    Info "checking ssl availability in bundled python…"
    $sslProbe = & "$payload\runtimes\python\python.exe" -c "import ssl; print('ssl-ok')" 2>&1
    if ("$sslProbe" -notmatch "ssl-ok") {
        Info "ssl missing in embeddable python — copying ssl support from venv"
        foreach ($f in @("_ssl.pyd", "_hashlib.pyd", "_socket.pyd", "libcrypto-1_1.dll", "libssl-1_1.dll", "libcrypto-3.dll", "libssl-3.dll")) {
            $src = Join-Path $root "backend\.venv\Scripts\$f"
            $src2 = Join-Path $root "backend\.venv\DLLs\$f"
            if (Test-Path $src)  { Copy-Item $src  "$payload\runtimes\python\" -Force }
            if (Test-Path $src2) { Copy-Item $src2 "$payload\runtimes\python\" -Force }
        }
        $sslProbe2 = & "$payload\runtimes\python\python.exe" -c "import ssl; print('ssl-ok')" 2>&1
        if ("$sslProbe2" -notmatch "ssl-ok") { throw "ssl still unavailable in bundled python — cannot install AI deps later" }
    }

    Stage "Payload: Node.js runtime (node.exe $NodeVersion)"
    $nodeZip = Join-Path $cache "node-v$NodeVersion-win-x64.zip"
    if (-not (Test-Path $nodeZip)) {
        Info "downloading node zip…"
        Invoke-WebRequest -UseBasicParsing "https://nodejs.org/dist/v$NodeVersion/node-v$NodeVersion-win-x64.zip" -OutFile $nodeZip
    }
    $tmpNode = Join-Path $cache "node-extract-$NodeVersion"
    if (-not (Test-Path "$tmpNode\node.exe")) {
        if (Test-Path $tmpNode) { Remove-Item $tmpNode -Recurse -Force }
        Expand-Archive $nodeZip $tmpNode -Force
        # zip contains node-vX-win-x64/node.exe
        $innerNode = Get-ChildItem $tmpNode -Recurse -Filter "node.exe" | Select-Object -First 1
        if (-not $innerNode) { throw "node.exe not found in zip" }
        Copy-Item $innerNode.FullName -Destination $tmpNode\node.exe -Force
    }
    New-Item -ItemType Directory -Force -Path "$payload\runtimes\node" | Out-Null
    Copy-Item "$tmpNode\node.exe" "$payload\runtimes\node\node.exe" -Force

    Stage "Payload: core wheels (offline install, first run)"
    # pip wheel (not pip download): a few transitive deps (e.g. docopt for
    # psd-tools) ship sdist only — wheel builds them here so the payload can
    # still install fully offline at runtime.
    Invoke-PipDownload -PipArgs @("wheel", "-r", "$root\backend\requirements-core.txt", "--wheel-dir", "$payload\wheels") -FailureMessage "pip wheel (core) failed"
    Invoke-PipDownload -PipArgs @("download", "pip", "setuptools", "wheel", "--dest", "$payload\wheels", "--only-binary=:all:") -FailureMessage "pip download (pip tooling) failed"

    Stage "Payload: get-pip bootstrap"
    $getpip = Join-Path $payload "get-pip.py"
    Invoke-WebRequest -UseBasicParsing "https://bootstrap.pypa.io/get-pip.py" -OutFile $getpip

    $sizeMb = [math]::Round((Get-ChildItem $payload -Recurse | Measure-Object Length -Sum).Sum / 1MB)
    Info "payload size: $sizeMb MB"
}

if (-not $SkipBundle) {
    Stage "Building Vira installer (Tauri NSIS)"
    Push-Location $srcTauri
    & npx --yes @tauri-apps/cli@2 build
    $code = $LASTEXITCODE
    Pop-Location
    if ($code -ne 0) { throw "tauri build failed (exit $code)" }

    $nsis = Get-ChildItem "$srcTauri\target\release\bundle\nsis" -Filter "*-setup.exe" |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1
    if (-not $nsis) { throw "installer bundle not produced" }

    Stage "Done"
    Write-Host "    installer: $($nsis.FullName)" -ForegroundColor Green
    Write-Host ("    elapsed:   {0:mm\:ss}" -f $stopwatch.Elapsed)
}

Write-Host ""
