"""
Vira Translator Studio — Automatic Update System

Checks for new versions using a version manifest, downloads updates with
hash verification, and applies them without touching user data.

Design:
  * Version manifest hosted at a configurable URL (GitHub releases by default)
  * Manifest contains: version, download URL, SHA-256 hash, release notes,
    minimum version, mandatory flag
  * Downloads are verified against SHA-256 before installation
  * User data (db.sqlite3, media/) is never touched
  * Users can skip, postpone, or disable updates
  * Update state is stored in Django's GlobalSetting (persisted in SQLite)
"""

from __future__ import annotations

import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import urllib.request
import urllib.error
from datetime import datetime, timedelta
from pathlib import Path

from django.conf import settings
from django.db import transaction

from .models import GlobalSetting

# ── Configuration ───────────────────────────────────────────────────────────

CURRENT_VERSION = "1.0.1"

# Default manifest URL — can be overridden via STUDIO_UPDATE_MANIFEST_URL
DEFAULT_MANIFEST_URL = (
    "https://raw.githubusercontent.com/ArshiaGalaxy/"
    "Vira_MangaTranslatorStudio/master/desktop/version-manifest.json"
)

# How often to check for updates (seconds)
UPDATE_CHECK_INTERVAL = 24 * 60 * 60  # 24 hours

# Download timeout
DOWNLOAD_TIMEOUT = 300  # 5 minutes

# Update state keys in GlobalSetting
STATE_KEY = "studio_update_state"


class UpdateError(Exception):
    pass


# ── Version manifest ────────────────────────────────────────────────────────

class VersionManifest:
    """Represents a version manifest entry."""

    def __init__(self, data: dict):
        self.version = data.get("version", "")
        self.download_url = data.get("download_url", "")
        self.sha256 = data.get("sha256", "")
        self.release_notes = data.get("release_notes", "")
        self.published_at = data.get("published_at", "")
        self.minimum_version = data.get("minimum_version", "0.0.0")
        self.mandatory = data.get("mandatory", False)
        self.file_size = data.get("file_size", 0)
        self.platform = data.get("platform", "windows-x64")


def fetch_manifest(url: str, timeout: int = 15) -> VersionManifest | None:
    """Download and parse the version manifest. Returns None on failure."""
    try:
        req = urllib.request.Request(url, headers={"User-Agent": "Vira-Updater/1.0"})
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            if resp.status != 200:
                return None
            data = json.loads(resp.read().decode("utf-8"))
            return VersionManifest(data)
    except Exception:
        return None


def parse_version(v: str) -> tuple[int, ...]:
    """Parse a version string into a comparable tuple."""
    parts = []
    for p in v.strip().lstrip("v").split("."):
        try:
            parts.append(int(p))
        except ValueError:
            parts.append(0)
    return tuple(parts)


def is_newer(candidate: str, current: str) -> bool:
    return parse_version(candidate) > parse_version(current)


def meets_minimum(candidate: str, minimum: str) -> bool:
    return parse_version(candidate) >= parse_version(minimum)


# ── Update state ────────────────────────────────────────────────────────────

def _get_update_state() -> dict:
    obj, _ = GlobalSetting.objects.get_or_create(key=STATE_KEY)
    if not isinstance(obj.value, dict):
        obj.value = {}
        obj.save()
    return obj.value


def _save_update_state(state: dict) -> None:
    obj, _ = GlobalSetting.objects.get_or_create(key=STATE_KEY)
    obj.value = state
    obj.save()


def get_update_prefs() -> dict:
    """Return user update preferences."""
    state = _get_update_state()
    return {
        "enabled": state.get("enabled", True),
        "last_check": state.get("last_check"),
        "last_version": state.get("last_version"),
        "skipped_version": state.get("skipped_version"),
        "postpone_until": state.get("postpone_until"),
        "download_path": state.get("download_path"),
        "update_ready": state.get("update_ready", False),
        "pending_version": state.get("pending_version"),
        "pending_url": state.get("pending_url"),
        "pending_hash": state.get("pending_hash"),
        "pending_notes": state.get("pending_notes", ""),
    }


def set_update_prefs(prefs: dict) -> None:
    state = _get_update_state()
    state.update(prefs)
    _save_update_state(state)


def should_check_now() -> bool:
    """Return True if enough time has passed since the last check."""
    prefs = get_update_prefs()
    if not prefs.get("enabled", True):
        return False
    last = prefs.get("last_check")
    if not last:
        return True
    try:
        last_dt = datetime.fromisoformat(last)
    except (ValueError, TypeError):
        return True
    return datetime.now() - last_dt > timedelta(seconds=UPDATE_CHECK_INTERVAL)


def is_postponed() -> bool:
    """Return True if updates are postponed until a future date."""
    prefs = get_update_prefs()
    until = prefs.get("postpone_until")
    if not until:
        return False
    try:
        until_dt = datetime.fromisoformat(until)
    except (ValueError, TypeError):
        return False
    return datetime.now() < until_dt


def is_skipped(version: str) -> bool:
    """Return True if this version was explicitly skipped."""
    prefs = get_update_prefs()
    return prefs.get("skipped_version") == version


# ── Update check ────────────────────────────────────────────────────────────

def check_for_updates(manifest_url: str | None = None,
                      force: bool = False) -> dict:
    """
    Check for a new version. Returns a result dict:

      {"available": False}  — no update
      {"available": True, "manifest": VersionManifest, ...}  — update available

    Never raises; failures are logged and returned as {"available": False, "error": ...}.
    """
    if not force and not should_check_now():
        return {"available": False, "reason": "not_due"}

    if is_postponed():
        return {"available": False, "reason": "postponed"}

    url = manifest_url or os.environ.get(
        "STUDIO_UPDATE_MANIFEST_URL", DEFAULT_MANIFEST_URL)

    manifest = fetch_manifest(url)
    if manifest is None:
        return {"available": False, "error": "manifest_fetch_failed"}

    # Record the check
    prefs = get_update_prefs()
    prefs["last_check"] = datetime.now().isoformat()
    prefs["last_version"] = CURRENT_VERSION
    _save_update_state(prefs)

    # Version comparison
    if not is_newer(manifest.version, CURRENT_VERSION):
        return {"available": False, "reason": "up_to_date", "current": CURRENT_VERSION}

    if not meets_minimum(CURRENT_VERSION, manifest.minimum_version):
        return {"available": False, "reason": "below_minimum",
                "current": CURRENT_VERSION, "minimum": manifest.minimum_version}

    if is_skipped(manifest.version):
        return {"available": False, "reason": "skipped",
                "version": manifest.version}

    # Update is available — stage it
    prefs.update({
        "update_ready": True,
        "pending_version": manifest.version,
        "pending_url": manifest.download_url,
        "pending_hash": manifest.sha256,
        "pending_notes": manifest.release_notes,
        "pending_mandatory": manifest.mandatory,
        "pending_size": manifest.file_size,
    })
    _save_update_state(prefs)

    return {
        "available": True,
        "version": manifest.version,
        "current": CURRENT_VERSION,
        "release_notes": manifest.release_notes,
        "mandatory": manifest.mandatory,
        "file_size": manifest.file_size,
        "published_at": manifest.published_at,
        "download_url": manifest.download_url,
        "sha256": manifest.sha256,
    }


# ── Download ────────────────────────────────────────────────────────────────

def download_update(progress_cb=None) -> dict:
    """
    Download the staged update to a temp file and verify its SHA-256 hash.
    Returns {"path": str, "version": str} or raises UpdateError.
    """
    prefs = get_update_prefs()
    if not prefs.get("update_ready"):
        raise UpdateError("No update staged for download")

    url = prefs["pending_url"]
    expected_hash = prefs["pending_hash"]
    version = prefs["pending_version"]

    if not url:
        raise UpdateError("No download URL in update state")

    # Download to a temp file in the user data directory (writable)
    data_dir = getattr(settings, "USER_DATA_DIR", None)
    if not data_dir:
        from pathlib import Path
        data_dir = Path.home() / "Vira Translator Studio"
    download_dir = Path(data_dir) / "updates"
    download_dir.mkdir(parents=True, exist_ok=True)

    tmp_path = download_dir / f"vira-update-{version}.tmp"
    final_path = download_dir / f"Vira-Setup-{version}.exe"

    try:
        req = urllib.request.Request(url, headers={"User-Agent": "Vira-Updater/1.0"})
        with urllib.request.urlopen(req, timeout=DOWNLOAD_TIMEOUT) as resp:
            total = int(resp.headers.get("Content-Length", 0) or 0)
            downloaded = 0
            sha256 = hashlib.sha256()
            with open(tmp_path, "wb") as f:
                while True:
                    chunk = resp.read(65536)
                    if not chunk:
                        break
                    f.write(chunk)
                    downloaded += len(chunk)
                    sha256.update(chunk)
                    if progress_cb and total:
                        progress_cb(downloaded, total)

        actual_hash = sha256.hexdigest()
        if actual_hash.lower() != expected_hash.lower():
            tmp_path.unlink(missing_ok=True)
            raise UpdateError(
                f"Hash mismatch: expected {expected_hash}, got {actual_hash}")

        # Rename to final name
        if final_path.exists():
            final_path.unlink()
        tmp_path.rename(final_path)

        prefs["download_path"] = str(final_path)
        prefs["download_complete"] = True
        prefs["download_verified"] = True
        _save_update_state(prefs)

        return {"path": str(final_path), "version": version}

    except UpdateError:
        raise
    except Exception as e:
        tmp_path.unlink(missing_ok=True)
        raise UpdateError(f"Download failed: {e}")


# ── Apply ───────────────────────────────────────────────────────────────────

def apply_update(installer_path: str) -> None:
    """
    Launch the downloaded installer. The installer handles replacing
    application files while preserving the user data directory.

    The current process exits after launching the installer.
    """
    path = Path(installer_path)
    if not path.exists():
        raise UpdateError(f"Installer not found: {installer_path}")

    # Run the NSIS installer silently (it replaces app files, not user data)
    subprocess.Popen(
        [str(path), "/S", "/NCRC", "/NORESTART"],
        creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0,
    )

    # Exit the current app so the installer can replace files
    sys.exit(0)


# ── User preference setters ─────────────────────────────────────────────────

def skip_version(version: str) -> None:
    prefs = get_update_prefs()
    prefs["skipped_version"] = version
    prefs["update_ready"] = False
    _save_update_state(prefs)


def postpone(hours: int = 24) -> None:
    prefs = get_update_prefs()
    until = datetime.now() + timedelta(hours=hours)
    prefs["postpone_until"] = until.isoformat()
    prefs["update_ready"] = False
    _save_update_state(prefs)


def enable_updates(enabled: bool = True) -> None:
    prefs = get_update_prefs()
    prefs["enabled"] = enabled
    _save_update_state(prefs)


def dismiss_update() -> None:
    prefs = get_update_prefs()
    prefs["update_ready"] = False
    prefs["pending_version"] = None
    prefs["pending_url"] = None
    prefs["pending_hash"] = None
    _save_update_state(prefs)


# ── Background checker ──────────────────────────────────────────────────────

class UpdateChecker(threading.Thread):
    """Background thread that periodically checks for updates."""

    def __init__(self, interval: int = UPDATE_CHECK_INTERVAL,
                 manifest_url: str | None = None):
        super().__init__(daemon=True, name="vira-update-checker")
        self.interval = interval
        self.manifest_url = manifest_url
        self._stop = threading.Event()

    def run(self):
        while not self._stop.wait(self.interval):
            try:
                check_for_updates(manifest_url=self.manifest_url, force=True)
            except Exception:
                pass  # Never let the update checker crash the app

    def stop(self):
        self._stop.set()


# Global checker instance (started by the Django ready() or launcher)
_update_checker: UpdateChecker | None = None


def start_background_checker(manifest_url: str | None = None,
                             interval: int = UPDATE_CHECK_INTERVAL):
    global _update_checker
    if _update_checker is None:
        _update_checker = UpdateChecker(interval=interval,
                                         manifest_url=manifest_url)
        _update_checker.start()


def stop_background_checker():
    global _update_checker
    if _update_checker is not None:
        _update_checker.stop()
        _update_checker = None
