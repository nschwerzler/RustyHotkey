"""Small, dependency-free client for a RustyHotkey RPC process.

The client deliberately treats profile and framework payloads as JSON data. A
consumer can use the dataclasses for common geometry and actions, or pass plain
mappings when the Rust schema gains a new operation.
"""

from __future__ import annotations

import json
import os
import queue
import subprocess
import threading
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Any, Iterable, Mapping, Sequence


class RustyHotkeyError(RuntimeError):
    """Base exception raised by the client."""


class RpcError(RustyHotkeyError):
    """The engine rejected an RPC request."""


@dataclass(frozen=True)
class Point:
    x: float
    y: float


@dataclass(frozen=True)
class Region:
    x: float
    y: float
    width: float
    height: float


@dataclass(frozen=True)
class Profile:
    """A profile reference accepted by ``Client``.

    ``path`` is passed to the Rust CLI as its required profile argument. The
    SDK does not inspect or copy profile contents.
    """

    path: str | Path


@dataclass(frozen=True)
class Action:
    """Convenience constructors for the generic action wire format."""

    type: str
    values: Mapping[str, Any] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        return {"type": self.type, **dict(self.values)}

    @classmethod
    def tap(cls, key: int) -> "Action":
        return cls("tap", {"key": key})

    @classmethod
    def key(cls, key: int, down: bool) -> "Action":
        return cls("key", {"key": key, "down": down})

    @classmethod
    def move(cls, x: float, y: float) -> "Action":
        return cls("move", {"x": x, "y": y})

    @classmethod
    def click(cls, button: int) -> "Action":
        return cls("click", {"button": button})

    @classmethod
    def scroll(cls, delta: int, horizontal: bool = False) -> "Action":
        return cls("scroll", {"delta": delta, "horizontal": horizontal})

    @classmethod
    def wait(cls, ms: int) -> "Action":
        return cls("wait", {"ms": ms})

    @classmethod
    def random_wait(cls, min_ms: int, max_ms: int) -> "Action":
        return cls("random_wait", {"min_ms": min_ms, "max_ms": max_ms})

    @classmethod
    def text(cls, text: str) -> "Action":
        return cls("text", {"text": text})


def _json_value(value: Any) -> Any:
    if hasattr(value, "to_dict"):
        return value.to_dict()
    if isinstance(value, (Point, Region)):
        return asdict(value)
    if isinstance(value, Mapping):
        return {str(k): _json_value(v) for k, v in value.items()}
    if isinstance(value, (list, tuple)):
        return [_json_value(v) for v in value]
    return value


class Client:
    """Thread-safe RPC client around ``rustyhotkey rpc <profile>``.

    ``command`` can be supplied for tests or for a separately installed binary;
    it is always executed without a shell. The default command is the binary
    named ``rhk`` on PATH.
    """

    def __init__(self, profile: str | Path | Profile, *, command: Sequence[str] | None = None,
                 timeout: float = 5.0, close_timeout: float = 1.0,
                 heartbeat_interval: float | None = 0.5, mode: str = "rpc",
                 close_op: str | None = "pause", script_path: str | Path | None = None,
                 no_tray: bool = False, observe_only: bool = False) -> None:
        self.profile = str(profile.path if isinstance(profile, Profile) else profile)
        executable = os.environ.get("RHK_BINARY", "rhk") if command is None else None
        self.command = list(command or (executable,)) + [mode, self.profile]
        if script_path is None and mode == 'rpc':
            script_path = os.environ.get('RHK_SCRIPT_PATH') or None
        if script_path is not None:
            self.command += ["--script", str(script_path)]
        if no_tray:
            self.command.append("--no-tray")
        if observe_only:
            self.command.append("--observe-only")
        self.timeout = timeout
        self.close_timeout = close_timeout
        self.mode = mode
        self._process: subprocess.Popen[str] | None = None
        self._responses: dict[str, queue.Queue[dict[str, Any]]] = {}
        self._lock = threading.Lock()
        self._write_lock = threading.Lock()
        self._start_lock = threading.Lock()
        self._closed = False
        self._counter = 0
        self._reader: threading.Thread | None = None
        self._stderr: threading.Thread | None = None
        self._heartbeat_stop: threading.Event | None = None
        self._heartbeat: threading.Thread | None = None
        self._error: BaseException | None = None
        self._stderr_tail: queue.Queue[str] = queue.Queue(maxsize=32)
        self.heartbeat_interval = heartbeat_interval
        self.close_op = close_op
        self._script_telemetry: Any = None
        self._script_telemetry_checked = False

    def __enter__(self) -> "Client":
        self.start()
        return self

    def __exit__(self, *_: Any) -> None:
        self.close()

    def start(self) -> None:
        with self._start_lock:
            if self._closed: raise RustyHotkeyError("client is closed")
            self._start_process()
            if not self._script_telemetry_checked:
                self._script_telemetry_checked = True
                if self.mode == "rpc":
                    try:
                        from .script_telemetry import attach
                        self._script_telemetry = attach(self)
                    except Exception:
                        self._script_telemetry = None

    def _start_process(self) -> None:
        if self._process is not None:
            return
        self._process = subprocess.Popen(self.command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                         stderr=subprocess.PIPE, text=True, encoding="utf-8",
                                         bufsize=1, close_fds=True)
        self._reader = threading.Thread(target=self._read_stdout, name="rustyhotkey-rpc", daemon=True)
        self._stderr = threading.Thread(target=self._read_stderr, name="rustyhotkey-stderr", daemon=True)
        self._reader.start(); self._stderr.start()
        if self.heartbeat_interval is not None:
            self._heartbeat_stop = threading.Event()
            self._heartbeat = threading.Thread(target=self._heartbeat_loop, daemon=True)
            self._heartbeat.start()

    def _read_stdout(self) -> None:
        assert self._process and self._process.stdout
        try:
            for line in self._process.stdout:
                try:
                    response = json.loads(line)
                    request_id = str(response.get("id"))
                    with self._lock:
                        waiter = self._responses.get(request_id)
                    if waiter is not None:
                        waiter.put(response)
                except (ValueError, TypeError) as exc:
                    self._error = RustyHotkeyError(f"invalid RPC response: {exc}")
        finally:
            self._error = self._error or RustyHotkeyError("RustyHotkey process exited")
            with self._lock:
                waiters = list(self._responses.values())
            for waiter in waiters:
                waiter.put({"ok": False, "error": str(self._error)})

    def _read_stderr(self) -> None:
        assert self._process and self._process.stderr
        for line in self._process.stderr:
            try:
                self._stderr_tail.put_nowait(line.rstrip())
            except queue.Full:
                try: self._stderr_tail.get_nowait()
                except queue.Empty: pass

    def _heartbeat_loop(self) -> None:
        assert self._heartbeat_stop
        while not self._heartbeat_stop.wait(self.heartbeat_interval):
            try: self.heartbeat()
            except RustyHotkeyError: return

    def request(self, op: str, *, timeout: float | None = None, **payload: Any) -> Any:
        self.start()
        if self._closed or not self._process or not self._process.stdin:
            raise RustyHotkeyError("client is closed")
        with self._lock:
            self._counter += 1
            request_id = str(self._counter)
            waiter: queue.Queue[dict[str, Any]] = queue.Queue(maxsize=1)
            self._responses[request_id] = waiter
        message = {"id": request_id, "op": op, **{k: _json_value(v) for k, v in payload.items()}}
        try:
            with self._write_lock:
                self._process.stdin.write(json.dumps(message, separators=(",", ":")) + "\n")
                self._process.stdin.flush()
            try: response = waiter.get(timeout=self.timeout if timeout is None else timeout)
            except queue.Empty as exc: raise RustyHotkeyError(f"RPC timeout for {op}") from exc
            if not response.get("ok", False):
                raise RpcError(str(response.get("error", "RPC request failed")))
            return response.get("result")
        finally:
            with self._lock: self._responses.pop(request_id, None)

    def status(self) -> Any: return self.request("status")
    def geometry(self) -> Any: return self.request("geometry")
    def observe_state(self, points: Iterable[Point | Mapping[str, Any]] = ()) -> Any:
        return self.request("observe_state", points=[_json_value(p) for p in points])
    def input_events(self, after: int = 0) -> Any: return self.request("input_events", after=after)
    def arm(self) -> Any: return self.request("arm")
    def focus(self) -> Any: return self.request("focus")
    def pause(self) -> Any: return self.request("pause")
    def abort(self) -> Any: return self.request("abort")
    def heartbeat(self) -> Any: return self.request("heartbeat")
    def tick(self) -> Any: return self.request("tick")
    def execute(self, actions: Iterable[Action | Mapping[str, Any]]) -> Any:
        return self.request("execute", actions=[_json_value(a) for a in actions])
    def evaluate(self, condition: Mapping[str, Any]) -> Any: return self.request("evaluate", condition=condition)
    def screenshot(self, region: Region | Mapping[str, Any] | None = None, *, path: str | Path | None = None) -> Any:
        return self.request("screenshot", region=region, path=str(path) if path is not None else None)

    def debug_screenshot(self, *, path: str | Path | None = None) -> Any:
        """Save a full capture with the current Lua widgets drawn into it."""
        return self.request("debug_screenshot", path=str(path) if path is not None else None)
    def ocr(self, region: Region | Mapping[str, Any] | None = None) -> Any: return self.request("ocr", region=region)
    def pixel(self, point: Point | Mapping[str, Any]) -> Any: return self.request("pixel", point=point)
    def clipboard(self) -> Any: return self.request("clipboard")
    def notify(self, message: str, ttl_ms: int = 0) -> Any: return self.request("notify", message=message, ttl_ms=ttl_ms)
    def workflow(self, workflow: Mapping[str, Any]) -> Any: return self.request("workflow", workflow=workflow)
    def hotkeys(self, enabled: bool) -> Any: return self.request("hotkeys", enabled=enabled)
    def sound(self, cue: str = "default") -> Any: return self.request("sound", cue=cue)
    def load_widgets(self, paths: Iterable[str], state: Mapping[str, Any] | None = None) -> Any:
        return self.request("widgets", paths=list(paths), state=dict(state or {}))
    def update_widget_state(self, state: Mapping[str, Any]) -> Any:
        return self.request("widget_state", state=dict(state))
    def clear_widgets(self) -> Any: return self.request("clear_widgets")

    def close(self) -> None:
        if self._process is None or self._closed: return
        telemetry = self._script_telemetry
        self._script_telemetry = None
        if self._heartbeat_stop: self._heartbeat_stop.set()
        if self.close_op is not None:
            try: self.request(self.close_op, timeout=self.close_timeout)
            except (RustyHotkeyError, OSError): pass
        # Release native owned input before waiting for display telemetry.
        if telemetry is not None:
            try: telemetry.stop()
            except Exception: pass
        self._closed = True
        process = self._process
        try:
            if process.stdin: process.stdin.close()
            process.wait(timeout=self.close_timeout)
        except (subprocess.TimeoutExpired, OSError):
            process.terminate()
            try: process.wait(timeout=self.close_timeout)
            except subprocess.TimeoutExpired: process.kill(); process.wait()
        for stream in (process.stdout, process.stderr):
            if stream is not None:
                try: stream.close()
                except OSError: pass
        for thread in (self._reader, self._stderr):
            if thread is not None: thread.join(timeout=self.close_timeout)
        self._process = None
