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
