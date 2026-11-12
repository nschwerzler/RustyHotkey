"""Best-effort loopback bridge for input events from managed RPC scripts."""

from __future__ import annotations

import http.client
import json
import re
import threading
from urllib.parse import urlsplit
from typing import Any


_PATH = "/api/script-input-events"
_MAX_PACKET_EVENTS = 256
_POLL_INTERVAL = 0.1
_HTTP_TIMEOUT = 0.8
_STOP_JOIN_TIMEOUT = 0.9
_KEY = re.compile(r"(?:Key[A-Z]|Digit[0-9]|Mouse[1-5]|F(?:[1-9]|1[0-9]|2[0-4])|"
                  r"Backspace|Tab|Enter|Shift(?:Left|Right)|Control(?:Left|Right)|"
                  r"Alt(?:Left|Right)|Meta(?:Left|Right)|Pause|CapsLock|Space|Escape|"
                  r"PageUp|PageDown|End|Home|Arrow(?:Left|Up|Right|Down)|PrintScreen|"
                  r"Insert|Delete|ContextMenu|Numpad(?:[0-9]|Multiply|Add|Subtract|Decimal|Divide)|"
                  r"NumLock|ScrollLock|Semicolon|Equal|Comma|Minus|Period|Slash|Backquote|"
                  r"BracketLeft|Backslash|BracketRight|Quote|VK_[0-9A-F]{2,4})\Z")


def _configuration() -> tuple[str, int, str, str] | None:
    """Return parsed loopback destination and credentials, without diagnostics."""
    import os

    run_id = os.environ.get("RHK_SCRIPT_RUN_ID", "")
    url = os.environ.get("RHK_TELEMETRY_URL", "")
    token = os.environ.get("RHK_TELEMETRY_TOKEN", "")
    if (not run_id or len(run_id) > 256 or any(ord(ch) < 32 or ord(ch) == 127 for ch in run_id)
            or not token or len(token) > 512
            or any(ord(ch) < 33 or ord(ch) > 126 for ch in token)
            or not url or len(url) > 128
            or any(ord(ch) <= 32 or ord(ch) == 127 for ch in url)):
        return None
    try:
        parsed = urlsplit(url)
        port = parsed.port
    except ValueError:
        return None
    if (parsed.scheme != "http" or parsed.hostname != "127.0.0.1"
            or parsed.username is not None or parsed.password is not None
            or parsed.path != _PATH or parsed.query or parsed.fragment
            or port is None or not 1 <= port <= 65535
            or parsed.netloc != f"127.0.0.1:{port}"):
        return None
    return parsed.hostname, port, run_id, token


def _valid_key(value: Any) -> str | None:
    if isinstance(value, str) and len(value) <= 20 and _KEY.fullmatch(value):
        return value
    return None


class TelemetryForwarder:
    """Poll a Client's native input history and publish bounded local batches."""

    def __init__(self, client: Any, host: str, port: int, run_id: str, token: str) -> None:
        self._client = client
        self._host = host
        self._port = port
        self._run_id = run_id
        self._token = token
        self._stop = threading.Event()
        self._post_lock = threading.Lock()
        self._thread = threading.Thread(
            target=self._run,
            name="rhk-script-telemetry",
            daemon=True,
        )
        self._stopped = False
        self._stop_lock = threading.Lock()
        self._thread.start()

    def _post(self, events: list[dict[str, str]]) -> bool:
        if not events or len(events) > _MAX_PACKET_EVENTS:
            return not events
        with self._post_lock:
            is_reset = events == [{"kind": "reset"}]
            if self._stop.is_set() and not is_reset:
                return False
            body = json.dumps(
                {"run_id": self._run_id, "events": events},
                separators=(",", ":"),
            ).encode("utf-8")
            connection = http.client.HTTPConnection(
                self._host,
                self._port,
                timeout=_HTTP_TIMEOUT,
            )
            try:
                connection.request(
                    "POST",
                    _PATH,
                    body=body,
                    headers={
                        "Content-Type": "application/json",
                        "Content-Length": str(len(body)),
                        "X-RHK-Token": self._token,
                        "Connection": "close",
                    },
                )
                response = connection.getresponse()
                response.read(4096)
                return 200 <= response.status < 300
            except (OSError, http.client.HTTPException, TimeoutError):
                return False
            finally:
                connection.close()

    def _reset_to(self, held: Any) -> bool:
        if not self._post([{"kind": "reset"}]):
            return False
        keys = sorted({key for item in held if (key := _valid_key(item)) is not None}) \
            if isinstance(held, list) else []
        for start in range(0, len(keys), _MAX_PACKET_EVENTS):
            batch = [{"kind": "down", "key": key}
                     for key in keys[start:start + _MAX_PACKET_EVENTS]]
            if not self._post(batch):
                return False
        return True

    def _consume(self, snapshot: Any, cursor: int) -> int:
        if not isinstance(snapshot, dict):
            return cursor
        events = snapshot.get("events")
        last = snapshot.get("last_sequence")
        oldest = snapshot.get("oldest_sequence")
        held = snapshot.get("held", [])
        if (not isinstance(events, list) or not isinstance(last, int) or isinstance(last, bool)
                or last < 0 or not isinstance(oldest, int) or isinstance(oldest, bool)
                or oldest < 0):
            return cursor
        normalized: list[tuple[int, dict[str, str]]] = []
        previous = cursor
        gap = bool(snapshot.get("truncated")) or cursor > last
        for raw in events:
            if not isinstance(raw, dict):
                gap = True
                break
            sequence = raw.get("sequence")
            kind = raw.get("kind")
            if (not isinstance(sequence, int) or isinstance(sequence, bool)
                    or sequence != previous + 1):
                gap = True
                break
            if kind in ("down", "up"):
                key = _valid_key(raw.get("key"))
                if key is None:
                    gap = True
                    break
                normalized.append((sequence, {"kind": kind, "key": key}))
            previous = sequence
        if previous < last and not gap:
            gap = True
        if gap:
            if self._stop.is_set():
                return cursor
            return last if self._reset_to(held) else cursor
        if not normalized:
            return last if last == cursor else cursor
        batch = normalized[:_MAX_PACKET_EVENTS]
        if self._stop.is_set():
            return cursor
        if not self._post([event for _, event in batch]):
            return cursor
        return batch[-1][0]

    def _run(self) -> None:
        cursor = 0
        while not self._stop.is_set():
            try:
                snapshot = self._client.input_events(after=cursor)
            except Exception:
                self._stop.wait(_POLL_INTERVAL)
                continue
            # Stop may race an in-flight native RPC; never publish its response
            # after stop() has sent the final reset packet.
            if self._stop.is_set():
                return
            try:
                cursor = self._consume(snapshot, cursor)
            except Exception:
                # Telemetry must not affect script execution.
                pass
            self._stop.wait(_POLL_INTERVAL)

    def stop(self) -> None:
        with self._stop_lock:
            if self._stopped:
                return
            self._stopped = True
            self._stop.set()
        if threading.current_thread() is not self._thread:
            self._thread.join(_STOP_JOIN_TIMEOUT)
        try:
            self._post([{"kind": "reset"}])
        except Exception:
            pass


def attach(client: Any) -> TelemetryForwarder | None:
    """Attach only when all managed-run telemetry environment values validate."""
    try:
        configuration = _configuration()
        if configuration is None:
            return None
        return TelemetryForwarder(client, *configuration)
    except Exception:
        # Invalid local telemetry configuration never prevents a script running.
        return None
