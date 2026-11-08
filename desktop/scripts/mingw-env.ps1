# Ensures the GNU toolchain (binutils + GCC + runtime libs) for x86_64-pc-windows-gnu builds.
#
# Why this exists: the rustup GNU toolchain ships dlltool but no assembler, and
# rustc's raw-dylib import-library generation needs a working dlltool + as pair
# (windows-sys 0.61+ uses raw-dylib). tauri-build also needs windres.exe, which
# shells out to `gcc -E` for preprocessing, and the final link uses `gcc`.
# If the machine already has a working gcc + as on PATH, nothing is downloaded.
# Otherwise the MSYS2 mingw64 packages (+ their DLL deps) are fetched once
# into desktop\.cache and prepended to PATH for this process tree.

$script:MingwPackages = @(
    # binutils: as / dlltool / windres / ld / objdump
    "mingw-w64-x86_64-binutils-2.47-4-any.pkg.tar.zst",
    # mingw-w64 runtime headers + CRT (link target)
    "mingw-w64-x86_64-headers-14.0.0.r598.ge3d18511f-1-any.pkg.tar.zst",
    "mingw-w64-x86_64-crt-14.0.0.r598.ge3d18511f-1-any.pkg.tar.zst",
    "mingw-w64-x86_64-libwinpthread-14.0.0.r598.ge3d18511f-1-any.pkg.tar.zst",
    "mingw-w64-x86_64-winpthreads-14.0.0.r598.ge3d18511f-1-any.pkg.tar.zst",
    # compiler runtime libraries used by gcc at build time
    "mingw-w64-x86_64-gmp-6.3.0-2-any.pkg.tar.zst",
    "mingw-w64-x86_64-mpfr-4.2.2-3-any.pkg.tar.zst",
    "mingw-w64-x86_64-mpc-1.4.1-1-any.pkg.tar.zst",
    "mingw-w64-x86_64-isl-0.28-1-any.pkg.tar.zst",
    # shared library deps of binutils/gcc
    "mingw-w64-x86_64-zlib-1.4-2-any.pkg.tar.zst",
    "mingw-w64-x86_64-libiconv-1.19.1-1-any.pkg.tar.zst",
    "mingw-w64-x86_64-gettext-0.22.4-3-any.pkg.tar.zst",
    "mingw-w64-x86_64-zstd-1.5.7-2-any.pkg.tar.zst",
    # GCC 16.2 runtime libraries (libgcc/libstdc++ DLLs and friends)
    "mingw-w64-x86_64-gcc-libs-16.2.0-6-any.pkg.tar.zst",
    "mingw-w64-x86_64-cc-libs-16.2.0-6-any.pkg.tar.zst",
    "mingw-w64-x86_64-libgcc-16.2.0-6-any.pkg.tar.zst",
    "mingw-w64-x86_64-libstdc%2B%2B-16.2.0-6-any.pkg.tar.zst",
    "mingw-w64-x86_64-libatomic-16.2.0-6-any.pkg.tar.zst",
    "mingw-w64-x86_64-libgomp-16.2.0-6-any.pkg.tar.zst",
    "mingw-w64-x86_64-libquadmath-16.2.0-6-any.pkg.tar.zst",
    "mingw-w64-x86_64-windows-default-manifest-20260815-1-any.pkg.tar.zst",
    # the compiler itself (must stay last: it depends on everything above)
    "mingw-w64-x86_64-gcc-16.2.0-6-any.pkg.tar.zst"
)
$script:MingwBase = "https://repo.msys2.org/mingw/mingw64/"

function Ensure-MingwToolchain {
    # 1) A working gcc + assembler already on PATH: reuse it (requirement:
    #    detect and reuse existing valid environments).
    $haveGcc = [bool](Get-Command gcc.exe -ErrorAction SilentlyContinue)
    $haveAs  = [bool](Get-Command as.exe -ErrorAction SilentlyContinue)
    if ($haveGcc -and $haveAs) { return }

    $desktop    = Split-Path -Parent (Split-Path -Parent $PSCommandPath)  # desktop\
    $cache      = Join-Path $desktop ".cache"
    $pkgDir     = Join-Path $cache "msys"
    $extract    = Join-Path $cache "mingw-extract"
    $bin        = Join-Path $extract "mingw64\bin"

    # 2) Previously extracted: just put it on PATH.
    if ((Test-Path (Join-Path $bin "as.exe")) -and (Test-Path (Join-Path $bin "gcc.exe"))) {
        $env:PATH = "$bin;$env:PATH"
        return
    }

    Write-Host "    preparing build toolchain (GNU binutils + GCC)..." -ForegroundColor DarkGray
    New-Item -ItemType Directory -Force -Path $pkgDir | Out-Null
    New-Item -ItemType Directory -Force -Path $extract | Out-Null

    foreach ($pkg in $script:MingwPackages) {
        $file = Join-Path $pkgDir $pkg
        if (-not (Test-Path $file)) {
            Invoke-WebRequest -UseBasicParsing ($script:MingwBase + $pkg) -OutFile $file -TimeoutSec 600
        }
        tar -xf $file -C $extract
        if ($LASTEXITCODE -ne 0) { throw "failed to extract $pkg" }
    }
    if (-not (Test-Path (Join-Path $bin "as.exe")))   { throw "toolchain extraction failed (as.exe missing)" }
    if (-not (Test-Path (Join-Path $bin "gcc.exe")))  { throw "toolchain extraction failed (gcc.exe missing)" }
    $env:PATH = "$bin;$env:PATH"
}

# Backwards-compatible alias used by older call sites.
function Ensure-MingwBinutils { Ensure-MingwToolchain }
