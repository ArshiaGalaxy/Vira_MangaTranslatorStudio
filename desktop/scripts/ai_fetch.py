#!/usr/bin/env python3
"""Resumable, hash-verified wheel pre-fetcher for Vira's AI extras (P0 phase).

Downloads the *directly pinned* wheels from requirements-ai.txt (paddlepaddle,
onnxruntime, …) with HTTP Range resume, exponential backoff, and SHA-256
verification into a persistent cache directory. Partial downloads are kept
as ``<file>.part`` so interrupted transfers continue across retries AND
application restarts. Verified files are never downloaded again.

Stdlib only — must run on the bundled embeddable Python (no pip packages).

stdout protocol (one record per line, ``|``-separated):
  VIRA_FETCH|1
  PROXY|<url or none>
  TOTAL|<pending>|<already_ok>|<bytes_total>
  PKG|<i>|<n>|<filename>
  RETRY|<filename>|<attempt>|<max>|<delay_s>|<reason>
  PROGRESS|<pct>|<i>|<n>|<filename>|<got>|<size>
  FILE_DONE|<i>|<n>|<filename>|<bytes>
  WARN|<package>|<reason>
  ALL_DONE|<n_ok>|<bytes>
  ERR|<kind>|<subject>|<message>          kinds: network | verify | setup

Exit codes: 0 = success, 2 = ERR emitted, 3 = usage/internal error.
"""

import argparse
import hashlib
import os
import random
import re
import socket
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import zipfile

UA = "Vira-ai-fetch/1.0"
SOCKET_TIMEOUT = 30
MAX_ATTEMPTS = 24  # per file per run; runs themselves are resumable (req #5)
CHUNK = 64 * 1024
PROGRESS_INTERVAL = 0.3  # seconds between PROGRESS records per file
MAX_BACKOFF = 60.0

# Index base only (PyPI) — torch wheels are no longer needed since LaMa
# moved to onnxruntime (1.0.1).
INDEXES = [
    "https://pypi.org/simple",
]


def out(*parts):
    print("|".join(str(p) for p in parts), flush=True)


def resolve_proxy():
    """Effective proxy URL for this run, or "" for direct connection.

    Order: explicit env vars first, then the Windows system proxy from the
    WinINET registry. The registry lookup matters because
    ``urllib.request.getproxies()`` short-circuits: a bare ``NO_PROXY``
    entry makes ``getproxies_environment()`` truthy, so the registry proxy
    is never consulted — which silently breaks users behind local proxy
    clients (v2rayN/xray, Clash, …) whose browsers work fine but whose
    pip/urllib traffic hits dead DNS. pip has the same gap (env-only), so
    the Rust orchestrator shells out with ``--print-proxy`` to learn this
    value and exports it for the pip phases too.
    """
    for key in ("HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy"):
        v = (os.environ.get(key) or "").strip()
        if v:
            return v
    try:
        reg = urllib.request.getproxies_registry()
    except Exception:  # noqa: BLE001 - non-Windows or registry unavailable
        reg = {}
    v = (reg.get("https") or reg.get("http") or "").strip()
    if v and "://" not in v:
        v = "http://" + v
    return v


def print_proxy():
    """`--print-proxy`: line 1 = proxy URL (or "none"), line 2 = comma-
    separated hosts that resolve via direct DNS (candidates for NO_PROXY)."""
    p = resolve_proxy()
    print(p if p else "none", flush=True)
    if p:
        direct = [h for h in PROXY_PROBE_HOSTS if host_resolves(h)]
        print(",".join(direct), flush=True)
    return 0


# ── Selective proxying (per host) ────────────────────────────────────────────
# The system proxy exists because *some* hosts are unreachable directly
# (DNS responses dropped → getaddrinfo fails), but routing EVERYTHING
# through the tunnel hurts hosts that do resolve: long transfers die every
# few MB inside the tunnel, while the same
# URL streamed fine over a direct connection. So: probe each host's DNS
# once, go direct when it resolves, proxy only when it doesn't; if a
# direct host later fails with network errors repeatedly, fall back to
# the proxy for it (adaptive both ways, no configuration needed).

PROXY_PROBE_HOSTS = [
    "pypi.org",
    "files.pythonhosted.org",
]
DIRECT_FAILS_BEFORE_PROXY = 2  # consecutive network failures → route via proxy

_dns_cache = {}   # hostname -> bool (resolves directly)
_route = {}       # hostname -> "direct" | "proxy" (explicit override)
_openers = {}     # use_proxy -> OpenerDirector
_proxy_url = ""   # set by main()


def host_resolves(host):
    """One-shot direct-DNS probe per host (cached; failures take ~12s once)."""
    if host not in _dns_cache:
        try:
            socket.getaddrinfo(host, 443, proto=socket.IPPROTO_TCP)
            _dns_cache[host] = True
        except OSError:
            _dns_cache[host] = False
    return _dns_cache[host]


def opener_for(host):
    """Opener for this host: direct when DNS works, else the system proxy."""
    route = _route.get(host)
    if route is None:
        route = "proxy" if (_proxy_url and not host_resolves(host)) else "direct"
    use_proxy = route == "proxy" and bool(_proxy_url)
    if use_proxy not in _openers:
        if use_proxy:
            ph = urllib.request.ProxyHandler(
                {"http": _proxy_url, "https": _proxy_url})
        else:
            ph = urllib.request.ProxyHandler({})  # force direct
        _openers[use_proxy] = urllib.request.build_opener(ph)
    return _openers[use_proxy]


def open_url(req, timeout=SOCKET_TIMEOUT):
    """urlopen replacement that routes per-host (see opener_for)."""
    host = urllib.parse.urlparse(req.full_url).hostname or ""
    return opener_for(host).open(req, timeout=timeout)


def note_direct_failure(host):
    """Count network failures on a direct-routed host; after
    DIRECT_FAILS_BEFORE_PROXY consecutive ones, route it via the proxy."""
    if _route.get(host) == "proxy" or not _proxy_url:
        return
    _direct_fail_count[host] = _direct_fail_count.get(host, 0) + 1
    if _direct_fail_count[host] >= DIRECT_FAILS_BEFORE_PROXY:
        _route[host] = "proxy"
        _openers.pop(False, None)  # force opener re-evaluation for this host
        out("ROUTE", host, "direct-failures->proxy")


_direct_fail_count = {}


def note_direct_success(host):
    _direct_fail_count.pop(host, None)


def norm(s):
    return re.sub(r"[-_.]+", "-", s).lower()


def parse_requirements(path):
    """Extract direct `name==version` pins (comments/`-r`/options skipped)."""
    pins = []
    with open(path, encoding="utf-8-sig", errors="replace") as fh:
        for raw in fh:
            line = raw.split("#", 1)[0].strip()
            m = re.match(r"^([A-Za-z0-9][A-Za-z0-9._-]*)==([^\s;]+)$", line)
            if m:
                pins.append((m.group(1), m.group(2)))
    return pins


def parse_assets(path):
    """Parse direct-asset spec lines: `url|filename|sha256` (comments skipped).

    Unlike wheels, assets are explicit downloads with a pinned SHA-256 —
    used for big model files that are not pip packages (e.g. lama.onnx).
    """
    assets = []
    with open(path, encoding="utf-8-sig", errors="replace") as fh:
        for raw in fh:
            line = raw.split("#", 1)[0].strip()
            if not line:
                continue
            parts = [p.strip() for p in line.split("|")]
            if len(parts) != 3 or not all(parts):
                out("WARN", path, f"skipping malformed asset line: {line[:120]}")
                continue
            url, filename, sha = parts
            assets.append({
                "pkg": filename, "version": "", "filename": filename,
                "url": url, "sha256": sha, "size": 0,
            })
    return assets


def fetch_index(index_base, pkg):
    """Return ([(filename, url, sha256|None)], error) from a PEP 503 index.

    Retries transient failures (DNS blips) 3× before giving up on the index.
    """
    url = f"{index_base}/{pkg}/"
    last_err = None
    for attempt in range(3):
        try:
            req = urllib.request.Request(
                url, headers={"User-Agent": UA, "Accept": "text/html"})
            with open_url(req, timeout=SOCKET_TIMEOUT) as resp:
                html = resp.read().decode("utf-8", "replace")
        except urllib.error.HTTPError as e:
            if e.code == 404:
                return [], f"HTTP 404"  # package simply not on this index
            last_err = f"HTTP {e.code}"
        except Exception as e:  # noqa: BLE001 - classify and retry
            last_err = f"{type(e).__name__}: {e}"
        else:
            results = []
            for m in re.finditer(r'<a\s[^>]*href="([^"]+)"', html, re.I):
                href = m.group(1)
                parsed = urllib.parse.urlparse(href)
                sha = None
                if parsed.fragment.startswith("sha256="):
                    sha = parsed.fragment[len("sha256="):].lower()
                path = urllib.parse.unquote(parsed.path)
                filename = path.rstrip("/").split("/")[-1]
                if not filename:
                    continue
                results.append(
                    (filename, urllib.parse.urljoin(url, href).split("#")[0], sha))
            if results:
                return results, None
            last_err = "index page had no links"
        if attempt < 2:
            time.sleep(1.5)
    return [], last_err


def pick_wheel(files, pkg, version):
    """Pick the best wheel for cp310/win_amd64 (or a universal fallback)."""
    n_pkg, n_ver = norm(pkg), version
    candidates = []
    for filename, url, sha in files:
        if not filename.endswith(".whl"):
            continue
        stem = filename[:-4]
        parts = stem.split("-")
        if len(parts) < 5:
            continue
        f_pkg = "-".join(parts[:-4])
        f_ver = parts[-4]
        if norm(f_pkg) != n_pkg or f_ver != n_ver:
            continue
        py, abi, plat = parts[-3], parts[-2], parts[-1]
        # Preference: CPython 3.10 Windows > universal > any Windows.
        if py == "cp310" and "win_amd64" in plat:
            rank = 0
        elif py in ("py3", "py2.py3") and plat in ("any", "py3-none-any"):
            rank = 1
        elif py in ("py3", "py2.py3"):
            rank = 2
        elif "win_amd64" in plat:
            rank = 3
        else:
            rank = 4
        candidates.append((rank, filename, url, sha))
    if not candidates:
        return None
    candidates.sort(key=lambda c: c[0])
    _, filename, url, sha = candidates[0]
    return filename, url, sha


def probe_size(url):
    """Total size via 1-byte Range request (HEAD is blocked on some CDNs)."""
    for attempt in range(3):
        try:
            req = urllib.request.Request(
                url, headers={"User-Agent": UA, "Range": "bytes=0-0"})
            with open_url(req, timeout=SOCKET_TIMEOUT) as resp:
                cr = resp.headers.get("Content-Range", "")
                if "/" in cr:
                    total = cr.rsplit("/", 1)[-1]
                    if total.isdigit():
                        return int(total)
                cl = resp.headers.get("Content-Length")
                if cl and cl.isdigit():
                    return int(cl)
            return 0
        except Exception:  # noqa: BLE001 - size is best-effort (DNS blips pass)
            if attempt < 2:
                time.sleep(1.5)
    return 0


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as fh:
        while True:
            chunk = fh.read(1024 * 1024)
            if not chunk:
                break
            h.update(chunk)
    return h.hexdigest()


def is_valid_wheel(path, sha, size):
    """SHA-256 when the index provided one, else structural wheel check."""
    try:
        if size and os.path.getsize(path) != size:
            return False
        if sha:
            return sha256_file(path).lower() == sha.lower()
        return zipfile.is_zipfile(path)
    except OSError:
        return False


class Progress:
    """Throttled overall-progress emitter (bytes-based with count fallback).

    ``total_box`` is a one-element list so the grand total can grow when a
    file's real size is discovered after a failed size probe.
    """

    def __init__(self, base_bytes, total_box, idx, n):
        self.base = base_bytes
        self.total_box = total_box
        self.idx = idx
        self.n = n
        self.last_emit = 0.0

    def emit(self, filename, got, file_total, force=False):
        now = time.time()
        if not force and now - self.last_emit < PROGRESS_INTERVAL:
            return
        self.last_emit = now
        total = self.total_box[0]
        done = self.base + got
        if total > 0:
            pct = int(min(99, 100 * done / total))
        else:
            frac = (self.idx - 1) + (got / file_total if file_total else 0)
            pct = int(min(99, 100 * frac / max(1, self.n)))
        out("PROGRESS", pct, self.idx, self.n, filename, got, file_total)


def fetch_one(item, dest_dir, prog, total_box):
    """Download one file (wheel or asset) with resume+backoff."""
    filename, url, sha, size = item["filename"], item["url"], item["sha256"], item["size"]
    host = urllib.parse.urlparse(url).hostname or ""
    final = os.path.join(dest_dir, filename)
    part = final + ".part"
    last_err = "unknown"

    if os.path.exists(final):
        if is_valid_wheel(final, sha, size):
            prog.emit(filename, os.path.getsize(final), size or os.path.getsize(final), force=True)
            return os.path.getsize(final)
        os.remove(final)  # corrupt/partial leftover — refetch

    last_size = os.path.getsize(part) if os.path.exists(part) else 0
    for attempt in range(1, MAX_ATTEMPTS + 1):
        try:
            offset = os.path.getsize(part) if os.path.exists(part) else 0
            headers = {"User-Agent": UA}
            if offset:
                headers["Range"] = f"bytes={offset}-"
            req = urllib.request.Request(url, headers=headers)
            try:
                resp = open_url(req, timeout=SOCKET_TIMEOUT)
            except urllib.error.HTTPError as e:
                if e.code == 416 and offset:
                    # Range not satisfiable → part is (probably) complete.
                    if is_valid_wheel(part, sha, size or offset):
                        os.replace(part, final)
                        prog.emit(filename, os.path.getsize(final), os.path.getsize(final), force=True)
                        return os.path.getsize(final)
                    os.remove(part)
                    raise IOError("partial file failed verification after 416") from e
                raise
            with resp:
                status = getattr(resp, "status", 200)
                if status == 206 and offset:
                    mode, got = "ab", offset
                    cr = resp.headers.get("Content-Range", "")
                    total = size or 0
                    if "/" in cr:
                        tail = cr.rsplit("/", 1)[-1]
                        if tail.isdigit():
                            total = int(tail)
                else:
                    mode, got = "wb", 0
                    cl = resp.headers.get("Content-Length")
                    total = int(cl) if cl and cl.isdigit() else size or 0
                # Keep the grand total honest when the size probe failed (DNS).
                if total and total != item.get("size", 0):
                    total_box[0] += total - item.get("size", 0)
                    item["size"] = total
                    size = total
                with open(part, mode) as fh:
                    while True:
                        chunk = resp.read(CHUNK)
                        if not chunk:
                            break
                        fh.write(chunk)
                        got += len(chunk)
                        prog.emit(filename, got, total)
            if total and got < total:
                raise IOError(f"incomplete-download ({got}/{total} bytes)")
            if not is_valid_wheel(part, sha, total or (os.path.getsize(part))):
                os.remove(part)
                raise IOError(
                    "SHA-256 verification failed" if sha else "not a valid wheel archive")
            os.replace(part, final)
            final_size = os.path.getsize(final)
            note_direct_success(host)
            prog.emit(filename, final_size, final_size, force=True)
            return final_size
        except Exception as e:  # noqa: BLE001 - classify and retry
            last_err = f"{type(e).__name__}: {e}" if str(e) else type(e).__name__
            if "SHA-256" not in last_err and "valid wheel" not in last_err:
                note_direct_failure(host)  # dead direct host → fall back to proxy
            if attempt >= MAX_ATTEMPTS:
                break
            cur = os.path.getsize(part) if os.path.exists(part) else 0
            progressed = cur - last_size
            last_size = max(last_size, cur)
            if progressed >= 512 * 1024:
                # The connection was demonstrably alive (it transferred
                # >0.5 MB before dying) — keep momentum with a short delay
                # instead of sitting out a full exponential penalty.
                delay = 2.0 + attempt * 1.5 + random.random()
            else:
                # Stall/dead connection → exponential backoff (capped).
                delay = min(MAX_BACKOFF, 2.0 ** attempt) + random.random() * 1.5
            out("RETRY", filename, attempt, MAX_ATTEMPTS, f"{delay:.0f}", last_err[:180])
            time.sleep(delay)

    kind = "verify" if "SHA-256" in last_err or "valid wheel" in last_err else "network"
    raise RuntimeError(f"{kind}|{filename}|{last_err}")


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--requirements")
    ap.add_argument("--dest")
    ap.add_argument("--assets",
                    help="direct-asset spec file (url|filename|sha256 lines)")
    ap.add_argument("--asset-dest",
                    help="directory for the files listed by --assets")
    ap.add_argument("--print-proxy", action="store_true",
                    help="print the effective proxy URL (or 'none') and exit")
    args = ap.parse_args()

    if args.print_proxy:
        return print_proxy()
    if args.requirements and not args.dest:
        ap.error("--dest is required together with --requirements")
    if not args.requirements and not args.assets:
        ap.error("pass --requirements and/or --assets (unless --print-proxy)")

    # Selective proxying: per-host DNS probe decides direct vs tunnel
    # (see opener_for / host_resolves). The pip phases learn the same
    # picture from `--print-proxy` (line 1 proxy, line 2 bypass hosts).
    global _proxy_url
    _proxy_url = resolve_proxy()

    out("VIRA_FETCH", 1)
    out("PROXY", _proxy_url or "none")
    if args.requirements and not os.path.isfile(args.requirements):
        out("ERR", "setup", args.requirements, "requirements file not found")
        return 2
    if args.assets and not os.path.isfile(args.assets):
        out("ERR", "setup", args.assets, "assets spec file not found")
        return 2
    if args.requirements:
        os.makedirs(args.dest, exist_ok=True)
    if args.asset_dest:
        os.makedirs(args.asset_dest, exist_ok=True)

    pins = parse_requirements(args.requirements) if args.requirements else []
    if args.requirements and not pins:
        out("ERR", "setup", args.requirements, "no name==version pins found")
        return 2

    items = []
    for pkg, version in pins:
        picked = None
        errors = []
        for index in INDEXES:
            files, err = fetch_index(index, pkg)
            if err:
                errors.append(f"{index}: {err}")
            if files:
                picked = pick_wheel(files, pkg, version)
                if picked:
                    break
        if not picked:
            out("WARN", f"{pkg}=={version}",
                ("no matching wheel for cp310/win_amd64" if not errors
                 else "; ".join(errors[:2])))
            continue
        filename, url, sha = picked
        items.append({
            "pkg": pkg, "version": version, "filename": filename,
            "url": url, "sha256": sha, "size": 0, "dest": args.dest,
        })

    asset_items = parse_assets(args.assets) if args.assets else []
    for it in asset_items:
        it["dest"] = args.asset_dest
    items.extend(asset_items)

    if not items:
        out("ERR", "setup", args.requirements or args.assets,
            "nothing to fetch: no wheels resolved and no assets specified "
            "(network unreachable?)")
        return 2

    # Skip already-verified files (requirement: never re-download).
    pending, ok_bytes = [], 0
    for it in items:
        final = os.path.join(it["dest"], it["filename"])
        size = it.get("size") or 0
        if os.path.exists(final) and is_valid_wheel(final, it["sha256"], 0):
            it["size"] = os.path.getsize(final)
            it["verified"] = True
            ok_bytes += it["size"]
        else:
            if os.path.exists(final):
                os.remove(final)
            pending.append(it)

    # Probe sizes of pending files (Range 0-0 → exact total, no full GET).
    total_bytes = ok_bytes
    for it in pending:
        it["size"] = probe_size(it["url"])
        total_bytes += it["size"]
    total_box = [total_bytes]

    out("TOTAL", len(pending), len(items) - len(pending), total_bytes)

    done_bytes = ok_bytes
    n = len(pending)
    for i, it in enumerate(pending, start=1):
        out("PKG", i, n, it["filename"])
        prog = Progress(done_bytes, total_box, i, n)
        try:
            got = fetch_one(it, it["dest"], prog, total_box)
        except RuntimeError as e:
            kind, subject, msg = str(e).split("|", 2)
            out("ERR", kind, subject, msg)
            return 2
        done_bytes += got
        out("FILE_DONE", i, n, it["filename"], got)

    out("ALL_DONE", len(items), done_bytes)
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        out("ERR", "network", "user", "interrupted")
        sys.exit(2)
    except Exception as e:  # noqa: BLE001 - last-resort classification
        out("ERR", "setup", "fetcher", f"{type(e).__name__}: {e}"[:400])
        sys.exit(3)
