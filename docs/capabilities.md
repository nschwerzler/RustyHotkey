# Capabilities and status

RHK is a generic Windows x64 framework. These interfaces are implemented;
automated checks use fake input backends and harmless owned test windows rather
than controlling a user's application.

| Capability | Current contract | Status |
|---|---|---|
| Target selection/focus | caller-supplied class/title; focus checks gate input | implemented in Rust backend/runtime |
| Keyboard, mouse, text | typed `Action` values and backend input events | implemented; live native use requires an explicit profile |
| Pixels, clipboard, capture, OCR | RPC inspection operations; capture writes BMP | implemented in runtime/backend |
| Conditions and workflows | typed conditions/steps; validate and dry-run | implemented |
| Recording/replay | CLI `record`/`replay`, schema-versioned JSON | implemented |
| Control | `arm`, `pause`, `abort`, `heartbeat`, `hotkeys`, status | implemented; script, execution, pause, hotkey and dormant states are separate |
| Turso Store/cache | Rust `Store`, CLI `db`, Python `Database` | implemented |
| Web GET/cache | Rust, `rhk fetch`, DB RPC `fetch`, Python `Database.fetch` | native loopback HTTP/cache tests |
| Lua widgets | selected files, `widgets`/`widget_state`/`clear_widgets`, Python helpers | sandbox, reload and bounds tests; no files means no overlay |
| Native overlay | target-relative click-through, focus-aware, capture-excluded | implemented on Windows when explicitly constructed |
| Tray | per-script icon, independent pause/edit/exit/hotkeys controls, dormant/running colors | started by `run`/`rpc`; `--no-tray` disables it; `rhk tray` provides global controls |
| Bot workspace | loopback UI, bounded handoffs, provider adapters, vault-backed keys | implemented in Python package; see [bots.md](bots.md) |

No capability implies a default target, default HUD, private profile, macro, calibration, or game-specific behavior. Do not claim a native/live check passed unless you ran it on the relevant Windows setup; repository tests and dry runs do not prove live input safety.

For source-level changes, preserve the generic boundary: no process memory inspection, no credentials in state or transcripts, and no live keyboard/mouse input in tests. Use fake backends or non-destructive checks as described in [CONTRIBUTING.md](../CONTRIBUTING.md).
