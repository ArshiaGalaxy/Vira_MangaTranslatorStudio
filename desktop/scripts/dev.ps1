# Vira Desktop - dev runner.
#
# Builds and launches the Tauri launcher against the staged payload
# (desktop\payload) without installing anything. Equivalent to running the
# installed app, but from source with debug symbols.
#
# Usage:  powershell -ExecutionPolicy Bypass -File scripts\dev.ps1
#         powershell -ExecutionPolicy Bypass -File scripts\dev.ps1 -Release

[CmdletBinding()]
param([switch]$Release)

$ErrorActionPreference = "Stop"

$scriptsDir = Split-Path -Parent $PSCommandPath   # desktop\scripts
$desktop    = Split-Path -Parent $scriptsDir
$srcTauri   = Join-Path $desktop "src-tauri"

. (Join-Path $scriptsDir "mingw-env.ps1")
Ensure-MingwBinutils

if (-not (Test-Path (Join-Path $desktop "payload\backend\manage.py"))) {
    throw "payload not staged - run: powershell -ExecutionPolicy Bypass -File scripts\build-app.ps1 -SkipBundle"
}
if (-not (Test-Path (Join-Path $srcTauri "icons\icon.ico"))) {
    throw "icons not generated - run: powershell -ExecutionPolicy Bypass -File scripts\build-app.ps1 -SkipBundle (or just build-app.ps1)"
}

Push-Location $srcTauri
try {
    # cargo/app stderr must not abort the script (EAP=Stop + redirected
    # native stderr in some hosts raises NativeCommandError).
    $prevEap = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    if ($Release) { cargo run --release } else { cargo run }
    $code = $LASTEXITCODE
    $ErrorActionPreference = $prevEap
    exit $code
} finally {
    Pop-Location
}
