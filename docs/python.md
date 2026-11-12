# Python client

RustyHotkey exposes a dependency-free Python client in `python/`. It starts
the x64 Rust binary as `rustyhotkey rpc <profile>` and exchanges one JSON object
per line. Applications own profiles and workflows; this package contains no
game-specific bindings or settings.

```python
from rustyhotkey import Action, Client

with Client("profile.json", heartbeat_interval=1.0) as engine:
    engine.arm()
    engine.execute([Action.tap(42), Action.wait(50)])
    print(engine.status())
```

Install locally with `python -m pip install ./python`, or add `python/` to an
application's package path. The client uses only the Python standard library.
The context manager pauses the engine before closing its process so held input
is released by the Rust runtime.

The client also exposes generic Lua widget control. `load_widgets(paths,
state)` loads zero or more application-owned scripts, `update_widget_state`
updates their shared state, and `clear_widgets` removes them. Loading no Lua
scripts produces no overlay. `notify` remains an engine notification request
and does not implicitly create an operating-system overlay.

Use `MockClient` to run the same Python orchestration logic against the real
Rust engine with a scripted backend when the target application is closed. It
has the `Client` RPC methods plus `observe(observation)`, `advance(ms)`,
`virtual_time()`, and `trace()`. The mock process cannot fall back to live RPC
or send operating-system input. Its `trace()` result can be passed to
`InputMonitor.simulation()`:

```python
from rustyhotkey import InputMonitor, MockClient

profile = {
    "schema_version": 1,
    "target": {"window_class": "FixtureWindow", "window_title": None},
    "controls": {"abort": 112, "reset": 113, "pause": 114, "resume": 115},
    "tick_ms": 20,
    "rules": [],
}
observation = {
    "target_present": True,
    "focused": True,
    "pixels": [{"point": {"x": 0.5, "y": 0.5}, "color": 1122867}],
    "ocr": [{"text": "Ready", "region": {"x": 0.1, "y": 0.1, "width": 0.4, "height": 0.2}}],
    "screenshot": {"width": 640, "height": 480, "solid_color": 0},
}

with MockClient(profile, observation) as engine:
    engine.arm()
    engine.execute([{"type": "tap", "key": 65}])
    engine.advance(100)
    trace = engine.trace()

monitor = InputMonitor()
monitor.simulation(trace, source="offline-plan")
```

The profile can be a mapping or a JSON file; observations are mappings.
Screenshot fixtures may use an RGB `solid_color` or a complete BGRA byte array;
screenshot, OCR and pixel reads then use only that fixture. Fixtures are staged
under the application data directory and removed on close. Screenshot region
arguments crop the fixture using normalized coordinates. `MockClient` also
supports `render_widgets()` to return a simulated overlay specification and
widget diagnostics without creating an operating-system overlay. It accepts
the usual script and observe-only options while remaining in mock mode. See
[simulation](simulation.md) for observation fields and size limits.

The same package exposes `Database` for the Rust Turso bridge. It accepts
parameter sequences and never embeds values into SQL:

```python
from rustyhotkey import Database

with Database("state.db") as db:
    rows = db.query("select value from cache where key = ?", ["key"])
    db.cache_put("prices", "key", {"value": 1}, ttl_ms=60_000)
```

Cache namespaces are explicit. Expired entries are treated as misses by the
Rust service; callers should refresh them rather than relying on stale data.

`Database.fetch` routes bounded web retrieval through the Rust service and
accepts `timeout_ms`, `max_bytes`, optional `ttl_ms`, and explicit
`allow_http=False` controls. `Client.hotkeys(enabled)` toggles framework
hotkey handling, while `script_path` and `no_tray=True` pass per-script CLI
options to the native process.
