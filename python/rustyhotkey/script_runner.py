"""Operator-controlled execution of revision-checked Python script drafts."""

from __future__ import annotations

import codecs
import os
import shutil
import subprocess
import sys
import threading
import time
import uuid
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any
from urllib.parse import urlsplit

from .scripts import ScriptConflict, ScriptWorkspace, ScriptWorkspaceError


MAX_RUNNING = 4
MAX_COMPLETED_HISTORY = 32
MAX_OUTPUT_BYTES = 64 * 1024
READ_CHUNK_BYTES = 4096
STOP_WAIT_SECONDS = 1.0


class ScriptRunnerError(ValueError):
    """A draft could not be launched or controlled safely."""


@dataclass
class _Run:
    id: str
    script_id: str
    revision: str
    process: subprocess.Popen[bytes]
    started_at: float
    snapshot_dir: Path
    state: str = "running"
    exit_code: int | None = None
    finished_at: float | None = None
    output: list[dict[str, str]] = field(default_factory=list)
    output_bytes: int = 0
    output_truncated: bool = False
    readers: list[threading.Thread] = field(default_factory=list)
    lock: threading.Lock = field(default_factory=threading.Lock)


class ScriptRunner:
    """Launch saved Python drafts as bounded, separately controlled processes.

    The caller must explicitly supply the current draft revision to ``run``.
    A private snapshot of that exact revision is executed; later edits cannot
    change a running program. Processes inherit the RHK SDK environment and
    use the workspace as their working directory for caller-owned assets.
    """

    def __init__(self, workspace: ScriptWorkspace, run_root: str | os.PathLike[str] | None = None):
        if not isinstance(workspace, ScriptWorkspace):
            raise TypeError("workspace must be a ScriptWorkspace")
        self.workspace = workspace
        self.run_root = Path(run_root) if run_root is not None else workspace.root.parent / "script-runs"
        self.run_root = self.run_root.expanduser().resolve()
        self.run_root.mkdir(parents=True, exist_ok=True)
        self._lock = threading.RLock()
        self._runs: dict[str, _Run] = {}
        self._closed = False
        self._telemetry: tuple[str,str] | None = None

    def set_telemetry(self, url: str, token: str) -> None:
        if not isinstance(url,str) or len(url) > 128 or any(ord(ch) <= 32 or ord(ch) == 127 for ch in url):
            raise ScriptRunnerError('telemetry requires the local application endpoint')
        try:
            parsed = urlsplit(url)
            port = parsed.port
        except ValueError as error:
            raise ScriptRunnerError('telemetry requires the local application endpoint') from error
        if parsed.scheme != 'http' or parsed.hostname != '127.0.0.1' or not port or parsed.netloc != f'127.0.0.1:{port}' or parsed.path != '/api/script-input-events' or parsed.query or parsed.fragment:
            raise ScriptRunnerError('telemetry requires the local application endpoint')
        if not isinstance(token,str) or not 1 <= len(token) <= 256 or any(ord(ch) < 33 or ord(ch) > 126 for ch in token):
            raise ScriptRunnerError('telemetry token is invalid')
        with self._lock: self._telemetry = (url,token)

    def run(self, script_id: str, expected_revision: str) -> dict[str, Any]:
        """Start one exact Python draft revision and return its run receipt."""
        if not isinstance(expected_revision, str) or not expected_revision:
            raise ScriptRunnerError("expected_revision is required")
        with self._lock:
            if self._closed:
                raise ScriptRunnerError("script runner is closed")
            self._refresh_all()
            if sum(run.state in ("running", "stopping") for run in self._runs.values()) >= MAX_RUNNING:
                raise ScriptRunnerError("at most four scripts may run at once")
            try:
                draft = self.workspace.read_script(script_id, "py")
            except ScriptWorkspaceError as exc:
                raise ScriptRunnerError(str(exc)) from None
            if draft["revision"] != expected_revision:
                raise ScriptConflict("draft revision changed; reload before running")

            run_id = uuid.uuid4().hex
            snapshot_dir = self.run_root / run_id
            snapshot_dir.mkdir(mode=0o700)
            snapshot = snapshot_dir / f"{script_id}.py"
            try:
                snapshot.write_text(draft["text"], encoding="utf-8", newline="")
                kwargs: dict[str, Any] = {
                    "args": [sys.executable, "-B", str(snapshot)],
                    "stdin": subprocess.DEVNULL,
                    "stdout": subprocess.PIPE,
                    "stderr": subprocess.PIPE,
                    "cwd": str(self.workspace.root),
                    "shell": False,
                    "bufsize": 0,
                }
                child_env = os.environ.copy()
                child_env["RHK_SCRIPT_PATH"] = str(self.workspace.root / f"{script_id}.py")
                child_env["RHK_SCRIPT_REVISION"] = expected_revision
                child_env['RHK_SCRIPT_RUN_ID'] = run_id
                child_env.pop('RHK_TELEMETRY_URL',None); child_env.pop('RHK_TELEMETRY_TOKEN',None)
                if self._telemetry is not None:
                    child_env['RHK_TELEMETRY_URL'],child_env['RHK_TELEMETRY_TOKEN'] = self._telemetry
                child_env["RHK_PYTHON"] = sys.executable
                child_env["PYTHONIOENCODING"] = "utf-8"
                existing_pythonpath = child_env.get("PYTHONPATH", "")
                paths = [entry for entry in existing_pythonpath.split(os.pathsep) if entry]
                workspace_path = str(self.workspace.root)
                if workspace_path not in paths:
                    paths.append(workspace_path)
                child_env["PYTHONPATH"] = os.pathsep.join(paths)
                kwargs["env"] = child_env
                if os.name == "nt":
                    kwargs["creationflags"] = getattr(subprocess, "CREATE_NO_WINDOW", 0x08000000)
                process = subprocess.Popen(**kwargs)
            except Exception as exc:
                shutil.rmtree(snapshot_dir, ignore_errors=True)
                raise ScriptRunnerError(f"could not start script process: {exc}") from None

            record = _Run(
                id=run_id,
                script_id=script_id,
                revision=expected_revision,
                process=process,
                started_at=time.time(),
                snapshot_dir=snapshot_dir,
            )
            self._runs[run_id] = record
            for stream_name, stream in (("stdout", process.stdout), ("stderr", process.stderr)):
                assert stream is not None
                reader = threading.Thread(
                    target=self._read_output,
                    args=(record, stream_name, stream),
                    name=f"rhk-script-{stream_name}-{run_id[:8]}",
                    daemon=True,
                )
                record.readers.append(reader)
                reader.start()
            self._refresh(record)
            return self._receipt(record)

    @staticmethod
    def _read_output(record: _Run, stream_name: str, stream: Any) -> None:
        decoder = codecs.getincrementaldecoder("utf-8")(errors="replace")
        try:
            while True:
                chunk = stream.read(READ_CHUNK_BYTES)
                if not chunk:
                    break
                ScriptRunner._append_output(record, stream_name, decoder.decode(chunk))
            ScriptRunner._append_output(record, stream_name, decoder.decode(b"", final=True))
        finally:
            stream.close()

    @staticmethod
    def _append_output(record: _Run, stream_name: str, text: str) -> None:
        if not text:
            return
        size = len(text.encode("utf-8"))
        with record.lock:
            if record.output_bytes >= MAX_OUTPUT_BYTES:
                record.output_truncated = True
                return
            remaining = MAX_OUTPUT_BYTES - record.output_bytes
            if size > remaining:
                text = text.encode("utf-8")[:remaining].decode("utf-8", errors="ignore")
                size = len(text.encode("utf-8"))
                record.output_truncated = True
            record.output.append({"stream": stream_name, "text": text})
            record.output_bytes += size

    def _refresh(self, record: _Run) -> None:
        code = record.process.poll()
        if code is None:
            return
        if record.exit_code is None:
            record.exit_code = code
            record.state = "exited" if code == 0 else "failed"
            record.finished_at = time.time()
            for reader in record.readers:
                reader.join(timeout=0.1)
            shutil.rmtree(record.snapshot_dir, ignore_errors=True)

    def _refresh_all(self) -> None:
        for record in self._runs.values():
            self._refresh(record)
        self._prune_history()

    def _prune_history(self) -> None:
        completed = sorted(
            (record for record in self._runs.values() if record.state in ("exited", "failed")),
            key=lambda record: record.finished_at or record.started_at,
        )
        for record in completed[: max(0, len(completed) - MAX_COMPLETED_HISTORY)]:
            self._runs.pop(record.id, None)

    @staticmethod
    def _receipt(record: _Run) -> dict[str, Any]:
        with record.lock:
            output = [dict(item) for item in record.output]
            truncated = record.output_truncated
        return {
            "id": record.id,
            "script_id": record.script_id,
            "revision": record.revision,
            "state": record.state,
            "exit_code": record.exit_code,
            "output": output,
            "output_truncated": truncated,
            "started_at": record.started_at,
            "finished_at": record.finished_at,
        }

    def list_runs(self) -> list[dict[str, Any]]:
        with self._lock:
            self._refresh_all()
            return [self._receipt(record) for record in self._runs.values()]

    def status(self, run_id: str) -> dict[str, Any]:
        if not isinstance(run_id, str):
            raise ScriptRunnerError("run id must be a string")
        with self._lock:
            record = self._runs.get(run_id)
            if record is None:
                raise ScriptRunnerError("run not found")
            self._refresh(record)
            receipt = self._receipt(record)
            self._prune_history()
            return receipt

    def stop(self, run_id: str) -> dict[str, Any]:
        if not isinstance(run_id, str):
            raise ScriptRunnerError("run id must be a string")
        with self._lock:
            record = self._runs.get(run_id)
            if record is None:
                raise ScriptRunnerError("run not found")
            self._refresh(record)
            if record.process.poll() is None:
                record.state = "stopping"
                try:
                    record.process.terminate()
                except OSError:
                    self._refresh(record)
                if record.process.poll() is None:
                    try:
                        record.process.wait(timeout=STOP_WAIT_SECONDS)
                    except subprocess.TimeoutExpired:
                        try:
                            record.process.kill()
                        except OSError:
                            self._refresh(record)
                        if record.process.poll() is None:
                            try:
                                record.process.wait(timeout=STOP_WAIT_SECONDS)
                            except subprocess.TimeoutExpired:
                                return self._receipt(record)
            self._refresh(record)
            receipt = self._receipt(record)
            self._prune_history()
            return receipt

    def close(self) -> None:
        """Stop direct interpreter children and wait/kill with bounded timeouts."""
        with self._lock:
            if self._closed:
                return
            self._closed = True
            active = [run.id for run in self._runs.values() if run.process.poll() is None]
        for run_id in active:
            try:
                self.stop(run_id)
            except ScriptRunnerError:
                continue

    def __enter__(self) -> "ScriptRunner":
        return self

    def __exit__(self, *_: object) -> None:
        self.close()
