"""Bounded, file-backed drafts for Python, Lua, and JSON scripts."""

from __future__ import annotations

import ast
import contextlib
import hashlib
import json
import os
import re
import tempfile
import threading
from pathlib import Path
from typing import Any


class ScriptWorkspaceError(ValueError):
    """A draft could not be read or saved safely."""


class RevisionConflictError(ScriptWorkspaceError):
    """The draft changed since the caller last read it."""


# Stable, concise name for HTTP/service integrations.
ScriptConflict = RevisionConflictError


_ID = re.compile(r"[A-Za-z0-9][A-Za-z0-9_-]{0,63}\Z")
_EXTENSIONS = {"py", "lua", "json"}
_LOCKS_GUARD = threading.Lock()
_LOCKS: dict[str, threading.RLock] = {}


def _revision(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


class ScriptWorkspace:
    """Manage text drafts beneath a caller-owned directory.

    The default directory is ``%APPDATA%/RustyHotkey/scripts``. Saves are
    atomic, bounded, and use a SHA-256 revision for optimistic concurrency.
    Python drafts are parsed and compiled to an AST, but are never executed.
    """

    def __init__(self, root: str | os.PathLike[str] | None = None, *, max_file_bytes: int = 256_000,
                 max_scripts: int = 256):
        if not isinstance(max_file_bytes, int) or isinstance(max_file_bytes, bool) or not 1 <= max_file_bytes <= 1_000_000:
            raise ValueError("max_file_bytes must be between 1 and 1000000")
        if not isinstance(max_scripts, int) or isinstance(max_scripts, bool) or not 1 <= max_scripts <= 4096:
            raise ValueError("max_scripts must be between 1 and 4096")
        if root is None:
            appdata = os.environ.get("APPDATA")
            if not appdata:
                raise ScriptWorkspaceError("APPDATA is unavailable; provide a workspace directory")
            root = Path(appdata) / "RustyHotkey" / "scripts"
        self.root = Path(root).expanduser().resolve()
        self.max_file_bytes = max_file_bytes
        self.max_scripts = max_scripts
        self.root.mkdir(parents=True, exist_ok=True)
        key = os.path.normcase(str(self.root))
        with _LOCKS_GUARD:
            self._lock = _LOCKS.setdefault(key, threading.RLock())

    @staticmethod
    def _parts(script_id: str, extension: str) -> tuple[str, str]:
        if not isinstance(script_id, str) or not _ID.fullmatch(script_id):
            raise ScriptWorkspaceError("script_id must be a simple identifier")
        if not isinstance(extension, str):
            raise ScriptWorkspaceError("extension must be py, lua, or json")
        extension = extension.lower().removeprefix(".")
        if extension not in _EXTENSIONS:
            raise ScriptWorkspaceError("extension must be py, lua, or json")
        return script_id, extension

    def _path(self, script_id: str, extension: str) -> Path:
        script_id, extension = self._parts(script_id, extension)
        return self.root / f"{script_id}.{extension}"

    def _read_bytes(self, path: Path) -> bytes:
        if path.is_symlink():
            raise ScriptWorkspaceError("symbolic links are not allowed")
        try:
            with path.open("rb") as source:
                data = source.read(self.max_file_bytes + 1)
        except FileNotFoundError:
            raise ScriptWorkspaceError("draft not found") from None
        if len(data) > self.max_file_bytes:
            raise ScriptWorkspaceError("draft exceeds the configured size limit")
        return data

    @contextlib.contextmanager
    def _file_lock(self):
        """Serialize revisions across workspace objects and processes."""
        lock_path = self.root / ".workspace.lock"
        with open(lock_path, "a+b") as lock_file:
            lock_file.seek(0, os.SEEK_END)
            if lock_file.tell() == 0:
                lock_file.write(b"0")
                lock_file.flush()
            lock_file.seek(0)
            if os.name == "nt":
                import msvcrt
                msvcrt.locking(lock_file.fileno(), msvcrt.LK_LOCK, 1)
                try:
                    yield
                finally:
                    lock_file.seek(0)
                    msvcrt.locking(lock_file.fileno(), msvcrt.LK_UNLCK, 1)
            else:
                import fcntl
                fcntl.flock(lock_file.fileno(), fcntl.LOCK_EX)
                try:
                    yield
                finally:
                    fcntl.flock(lock_file.fileno(), fcntl.LOCK_UN)

    def list_scripts(self) -> list[dict[str, Any]]:
        """Return metadata for drafts; malformed names and links are omitted."""
        entries = []
        with self._lock, self._file_lock():
            for path in self.root.iterdir():
                match = re.fullmatch(r"([A-Za-z0-9][A-Za-z0-9_-]{0,63})\.(py|lua|json)", path.name)
                if not match or path.is_symlink() or not path.is_file():
                    continue
                if len(entries) >= self.max_scripts:
                    raise ScriptWorkspaceError("workspace exceeds the configured script limit")
                try:
                    data = self._read_bytes(path)
                except (OSError, ScriptWorkspaceError):
                    continue
                entries.append({"id": match.group(1), "extension": match.group(2),
                                "revision": _revision(data), "draft_status": "draft"})
        return sorted(entries, key=lambda item: (item["id"].casefold(), item["extension"]))

    def read_script(self, script_id: str, extension: str) -> dict[str, Any]:
        path = self._path(script_id, extension)
        with self._lock, self._file_lock():
            data = self._read_bytes(path)
        try:
            text = data.decode("utf-8")
        except UnicodeDecodeError:
            raise ScriptWorkspaceError("draft is not valid UTF-8") from None
        return {"id": script_id, "extension": extension.lower().removeprefix("."),
                "text": text, "revision": _revision(data), "draft_status": "draft"}

    def save_script(self, script_id: str, extension: str, text: str,
                    expected_revision: str | None = None) -> dict[str, Any]:
        path = self._path(script_id, extension)
        extension = extension.lower().removeprefix(".")
        if not isinstance(text, str):
            raise ScriptWorkspaceError("text must be a string")
        try:
            data = text.encode("utf-8")
        except UnicodeEncodeError:
            raise ScriptWorkspaceError("text must be valid UTF-8") from None
        if len(data) > self.max_file_bytes:
            raise ScriptWorkspaceError("text exceeds the configured size limit")
        validation = "text_saved"
        if extension == "py":
            try:
                tree = ast.parse(text, filename=f"{script_id}.py", mode="exec")
                compile(tree, f"{script_id}.py", "exec")
            except (SyntaxError, ValueError, TypeError, RecursionError) as exc:
                raise ScriptWorkspaceError(f"invalid Python syntax: {exc}") from None
            validation = "python_ast_valid"
        elif extension == "json":
            try:
                json.loads(text)
            except (json.JSONDecodeError, ValueError, RecursionError) as exc:
                raise ScriptWorkspaceError(f"invalid JSON syntax: {exc}") from None
            validation = "json_valid"

        with self._lock, self._file_lock():
            if path.is_symlink():
                raise ScriptWorkspaceError("symbolic links are not allowed")
            if expected_revision is None:
                if path.exists():
                    raise RevisionConflictError("draft already exists; provide its revision to update")
                named_scripts = sum(
                    bool(re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_-]{0,63}\.(?:py|lua|json)", item.name))
                    and not item.is_symlink() and item.is_file()
                    for item in self.root.iterdir()
                )
                if named_scripts >= self.max_scripts:
                    raise ScriptWorkspaceError("workspace exceeds the configured script limit")
                current = None
            else:
                if not isinstance(expected_revision, str):
                    raise ScriptWorkspaceError("expected_revision must be a string")
                try:
                    current = self._read_bytes(path)
                except ScriptWorkspaceError as exc:
                    if str(exc) == "draft not found":
                        current = None
                    else:
                        raise
                actual = _revision(current) if current is not None else None
                if actual != expected_revision:
                    raise RevisionConflictError("draft revision changed")
            fd, temp_name = tempfile.mkstemp(prefix=".draft-", suffix=".tmp", dir=self.root)
            try:
                with os.fdopen(fd, "wb") as temp:
                    temp.write(data)
                    temp.flush()
                    os.fsync(temp.fileno())
                if path.is_symlink():
                    raise ScriptWorkspaceError("symbolic links are not allowed")
                if expected_revision is None:
                    # Hard-link publication is atomic and refuses to replace a
                    # draft created by another process after the exists check.
                    os.link(temp_name, path)
                    os.unlink(temp_name)
                else:
                    os.replace(temp_name, path)
            finally:
                try:
                    os.unlink(temp_name)
                except FileNotFoundError:
                    pass
        return {"id": script_id, "extension": extension, "revision": _revision(data),
                "text": text, "draft_status": "draft", "validation": validation}
