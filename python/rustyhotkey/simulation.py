"""Run bounded native what-if scenarios without creating a Windows backend."""
from __future__ import annotations
import json
import os
from pathlib import Path
import subprocess
import tempfile
from typing import Any, Mapping, Sequence

from .client import Client, Profile, RustyHotkeyError


_MAX_FIXTURE_BYTES = 4 * 1024 * 1024
_MAX_OBSERVATION_BYTES = 64 * 1024 * 1024
_MAX_OBSERVE_RPC_BYTES = 3 * 1024 * 1024


def _encoded_json(value: Any, *, name: str, limit: int) -> bytes:
    try:
        raw = json.dumps(value, allow_nan=False, separators=(",", ":")).encode("utf-8")
    except (TypeError, ValueError) as error:
        raise ValueError(f"{name} must be finite JSON data") from error
    if len(raw) > limit:
        raise ValueError(f"{name} exceeds the {limit // (1024 * 1024)} MiB limit")
    return raw


class MockClient(Client):
    """Client-compatible RPC session backed only by the native mock backend.

    ``profile`` may be a JSON path, :class:`Profile`, or profile mapping.
    ``observation`` is an initial scripted observation mapping. No error or
    missing mock binary can fall back to the live ``rpc`` command.
    """

    def __init__(
        self,
        profile: str | Path | Profile | Mapping[str, Any],
        observation: Mapping[str, Any],
        *,
        command: Sequence[str] | None = None,
        data_dir: str | Path | None = None,
        **client_options: Any,
    ) -> None:
        if not isinstance(observation, Mapping):
            raise ValueError("observation must be a JSON object")
        conflicting_options = {"mode"} & set(client_options)
        if conflicting_options:
            options = ", ".join(sorted(conflicting_options))
            raise ValueError(f"MockClient controls its mock mode and does not accept {options}")
        observation_raw = _encoded_json(
            observation, name="observation", limit=_MAX_OBSERVATION_BYTES
        )
        profile_value = profile.path if isinstance(profile, Profile) else profile
        profile_path: Path
        if isinstance(profile_value, Mapping):
            profile_raw = _encoded_json(profile_value, name="profile", limit=_MAX_FIXTURE_BYTES)
            profile_data = profile_raw
            staged_profile = True
        elif isinstance(profile_value, (str, Path)):
            profile_path = Path(profile_value)
            profile_data = None
            staged_profile = False
        else:
            raise ValueError("profile must be a path, Profile, or JSON object")

        if data_dir is None:
            root_value = os.environ.get("RHK_DATA_DIR") or os.environ.get("APPDATA")
            root = Path(root_value or Path.home() / ".cache")
            if not os.environ.get("RHK_DATA_DIR"):
                root /= "RustyHotkey"
        else:
            root = Path(data_dir)
        fixture_root = root / "mock-clients"
        fixture_root.mkdir(parents=True, exist_ok=True)
        self._fixture = tempfile.TemporaryDirectory(prefix="session-", dir=fixture_root)
        fixture_dir = Path(self._fixture.name)
        if staged_profile:
            profile_path = fixture_dir / "profile.json"
            profile_path.write_bytes(profile_data)
        observation_path = fixture_dir / "observation.json"
        observation_path.write_bytes(observation_raw)
        self._fixture_dir = fixture_dir
        self._final_trace: dict[str, Any] | None = None
        self._trace_failed = False
        self._virtual_time_ms = 0

        executable = list(command or [os.environ.get("RHK_BINARY", "rhk")])
        if not executable:
            self._fixture.cleanup()
            raise ValueError("mock command must contain an executable")
        options = dict(client_options)
        # Mock sessions always clean up through their explicit pause path.
        options["close_op"] = None
        try:
            super().__init__(
                profile_path,
                command=executable,
                mode="mock-rpc",
                **options,
            )
            # The mock CLI reads profile and observation from fixed positions;
            # Client appends optional flags after the required profile.
            self.command.insert(len(executable) + 2, str(observation_path))
        except Exception:
            self._fixture.cleanup()
            raise

    def start(self) -> None:
        try:
            super().start()
        except Exception:
            self._fixture.cleanup()
            raise

    def observe(self, observation: Mapping[str, Any]) -> Any:
        if not isinstance(observation, Mapping):
            raise ValueError("observation must be a JSON object")
        _encoded_json(observation, name="observation", limit=_MAX_OBSERVE_RPC_BYTES)
        return self.request("observe", observation=observation)

    def advance(self, ms: int) -> int:
        if isinstance(ms, bool) or not isinstance(ms, int) or not 0 <= ms <= 86_400_000:
            raise ValueError("advance must be an integer from 0 through 86400000 ms")
        result = self.request("advance", ms=ms)
        now = result.get("now_ms") if isinstance(result, dict) else None
        if isinstance(now, bool) or not isinstance(now, int) or not 0 <= now <= 86_400_000:
            raise RustyHotkeyError("mock advance returned an invalid virtual time")
        self._virtual_time_ms = now
        return self._virtual_time_ms

    def virtual_time(self) -> int:
        if self._closed:
            return self._virtual_time_ms
        result = self.request("virtual_time")
        now = result.get("now_ms") if isinstance(result, dict) else None
        if isinstance(now, bool) or not isinstance(now, int) or not 0 <= now <= 86_400_000:
            raise RustyHotkeyError("mock backend returned an invalid virtual time")
        self._virtual_time_ms = now
        return self._virtual_time_ms

    def trace(self) -> dict[str, Any]:
        """Return the complete bounded virtual-time event trace."""
        if self._closed:
            if self._trace_failed:
                raise RustyHotkeyError("mock client could not collect its final trace")
            return json.loads(json.dumps(self._final_trace or {"events": []}))
        result = self.request("trace")
        if not isinstance(result, dict) or not isinstance(result.get("events"), list):
            raise RustyHotkeyError("mock backend returned an invalid event trace")
        return result

    def render_widgets(self) -> Any:
        """Return the mock Lua overlay specification and diagnostics."""
        return self.request("render_widgets")

    def close(self) -> None:
        try:
            if self._process is not None and not self._closed:
                try:
                    self.pause()
                    self._final_trace = self.trace()
                    try:
                        self.virtual_time()
                    except Exception:
                        pass
                except Exception:
                    self._final_trace = None
                    self._trace_failed = True
            super().close()
        finally:
            self._fixture.cleanup()


def simulate(scenario: dict[str, Any], *, timeout: float = 15.0) -> dict[str, Any]:
    if not isinstance(scenario, dict): raise ValueError('scenario must be an object')
    if not 1 <= timeout <= 30: raise ValueError('simulation timeout must be 1 to 30 seconds')
    raw = json.dumps(scenario, allow_nan=False)
    if len(raw.encode('utf-8')) > 4 * 1024 * 1024: raise ValueError('scenario exceeds 4 MiB')
    root = Path(os.environ.get('APPDATA', Path.home() / '.cache')) / 'RustyHotkey' / 'simulations'
    root.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='scenario-', dir=root) as directory:
        path = Path(directory) / 'scenario.json'
        path.write_text(raw, encoding='utf-8')
        try:
            result = subprocess.run([os.environ.get('RHK_BINARY', 'rhk'), 'simulate', str(path)],
                                    capture_output=True, text=True, encoding='utf-8', timeout=timeout,
                                    shell=False)
        except subprocess.TimeoutExpired as error:
            raise ValueError('simulation exceeded its wall-time budget') from error
        if result.returncode: raise ValueError(result.stderr.strip()[:500] or 'simulation failed')
        if len(result.stdout.encode('utf-8')) > 8 * 1024 * 1024:
            raise ValueError('simulation result exceeds 8 MiB; reduce the scenario')
        value = json.loads(result.stdout)
        if not isinstance(value, dict): raise ValueError('invalid simulation result')
        return value
