"""Run bounded native what-if scenarios without creating a Windows backend."""
from __future__ import annotations
import json
import os
from pathlib import Path
import subprocess
import tempfile
from typing import Any


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
