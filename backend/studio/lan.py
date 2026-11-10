"""LAN exposure state: opt-in access for phone/tablet on the local network.

The desktop app is single-user and unauthenticated, so LAN access is gated
by a 6-digit PIN at the frontend gateway (frontend/lan-gate.js) — the only
component that ever listens on 0.0.0.0. Django stays on loopback; LAN
clients reach the API through the Next.js proxy like the desktop does.

State lives in GlobalSetting("lan") for the settings UI, and is mirrored to
``<USER_DATA_DIR>/lan.json`` for the launcher, which reads it at startup
(binding and the PIN are fixed at process start, so changes apply after the
app restarts).
"""

import hmac
import json
import os
import secrets
import socket
import tempfile
from pathlib import Path

from django.conf import settings

from .models import GlobalSetting

LAN_FILE_NAME = "lan.json"
STATE_KEY = "lan"

ENABLED_KEY = "enabled"
PIN_KEY = "pin"


def _random_pin() -> str:
    # 6 decimal digits; secrets guarantees unbiased sampling.
    return f"{secrets.randbelow(1_000_000):06d}"


def _valid_pin(pin: str) -> bool:
    return isinstance(pin, str) and len(pin) == 6 and pin.isdigit()


def get_lan_state() -> dict:
    """Persisted state with defaults: {"enabled": False, "pin": ""}."""
    obj = GlobalSetting.objects.filter(key=STATE_KEY).first()
    raw = (obj.value or {}) if obj else {}
    enabled = bool(raw.get(ENABLED_KEY, False))
    pin = raw.get(PIN_KEY, "")
    if not _valid_pin(pin):
        pin = ""
    return {ENABLED_KEY: enabled, PIN_KEY: pin}


def _write_launcher_file(state: dict) -> None:
    """Mirror the state for the launcher (atomic replace)."""
    base = Path(str(settings.USER_DATA_DIR))
    base.mkdir(parents=True, exist_ok=True)
    target = base / LAN_FILE_NAME
    fd, tmp = tempfile.mkstemp(dir=str(base), suffix=".tmp")
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump(
                {ENABLED_KEY: bool(state[ENABLED_KEY]),
                 PIN_KEY: state[PIN_KEY]},
                f,
            )
        os.replace(tmp, target)
    except OSError:
        # Best-effort: the launcher simply keeps loopback binding when the
        # file is missing; settings stay authoritative in the DB.
        try:
            os.unlink(tmp)
        except OSError:
            pass


def set_lan_state(enabled: bool, pin: str | None = None) -> dict:
    """Validate + persist the LAN state; returns the new state.

    ``pin=None`` keeps the stored pin (generating one when enabling without
    a stored pin); an invalid provided pin raises ValueError.
    """
    current = get_lan_state()
    new_pin = current[PIN_KEY]
    if pin is not None:
        if not _valid_pin(pin):
            raise ValueError("PIN must be exactly 6 digits")
        new_pin = pin
    if enabled and not new_pin:
        new_pin = _random_pin()

    state = {ENABLED_KEY: bool(enabled), PIN_KEY: new_pin}
    obj, _ = GlobalSetting.objects.get_or_create(key=STATE_KEY)
    obj.value = state
    obj.save(update_fields=["value", "updated_at"])
    _write_launcher_file(state)
    return state


def rotate_pin() -> dict:
    """Replace the pin with a fresh random one (keeps enabled as-is)."""
    current = get_lan_state()
    return set_lan_state(current[ENABLED_KEY], _random_pin())


def verify_pin(candidate: str, state: dict | None = None) -> bool:
    """Timing-safe pin check; empty stored pin never matches."""
    if state is None:
        state = get_lan_state()
    stored = state.get(PIN_KEY, "")
    if not _valid_pin(stored) or not isinstance(candidate, str):
        return False
    return hmac.compare_digest(stored, candidate)


def lan_ip() -> str | None:
    """Best-effort primary LAN IPv4 (UDP connect sends no packets)."""
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    try:
        sock.connect(("10.255.255.255", 1))
        return sock.getsockname()[0]
    except OSError:
        return None
    finally:
        sock.close()


def lan_url() -> str:
    """URL phones on the same network would use (frontend gateway port)."""
    port = os.environ.get("STUDIO_FRONTEND_PORT", "3000")
    ip = lan_ip()
    host = ip if ip else "<this-computer-ip>"
    return f"http://{host}:{port}"
