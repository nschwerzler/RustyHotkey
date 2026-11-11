# Usage patterns

Set `RHK_DATA_DIR` to an application data directory outside this repository when `APPDATA` is not suitable. Build the Windows x64 binary, then use `rhk` on `PATH`.

```powershell
rhk capabilities
rhk tools
rhk validate profile profile.json
rhk validate workflow workflow.json
rhk preview workflow.json
rhk rpc profile.json
rhk db state.db
rhk ui
rhk tray
rhk fetch https://example.com --database state.db --ttl-ms 60000
```

RPC and DB commands read JSON-lines from standard input and write one JSON response per line. For example:

```powershell
'{"id":"s1","op":"status"}' | rhk rpc profile.json
'{"id":"q1","op":"query","sql":"select ?1 as value","params":["synthetic"]}' | rhk db state.db
```

The minimal Python shape is:

```python
from rhk import Action, Client

profile = "profile.json"  # caller-owned explicit target profile
with Client(profile, heartbeat_interval=1.0) as engine:
    engine.status()
    engine.arm()
    engine.execute([Action.tap(42), Action.wait(50)])
```

For storage:

```python
from rhk import Database

with Database("state.db") as db:
    db.execute("create table if not exists notes (value text)")
    db.execute("insert into notes values (?1)", ["synthetic"])
    print(db.query("select value from notes"))
```

Use `Client.evaluate`, `pixel`, `clipboard`, `screenshot`, `ocr`, `notify`, and `workflow` for inspection and bounded orchestration. Keep live actions behind a caller-owned profile and focus check. Use `Client.pause()` or `Client.abort()` when cancellation is needed. `abort` is the emergency path; its physical control cannot be disabled.

Lua is optional and explicit:

```python
engine.load_widgets(["widgets/status.lua"], {"label": "ready"})
```

An application must choose those files. No selected Lua files means no overlay. See [python.md](python.md), [lua.md](lua.md), and [database.md](database.md) for focused details.

`run` and `rpc` create a per-script tray icon. Use `--script` to identify a
customization script, or `--no-tray` for headless use. A script can remain
loaded while dormant or while normal hotkeys are disabled; pause and emergency
abort are separate controls.
