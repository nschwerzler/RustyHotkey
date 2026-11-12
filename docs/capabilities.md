# Capabilities and status

RHK is a generic Windows x64 framework. These interfaces are implemented;
automated checks use fake input backends and harmless owned test windows rather
than controlling a user's application.

| Capability | Current contract | Status |
|---|---|---|
| Target selection/focus | caller-supplied class/title; focus checks gate input | implemented in Rust backend/runtime |
| Keyboard, mouse, text, scroll | typed actions include key/button control, click bursts, wheel scroll, sound, and hotkey toggles | implemented; live native use requires an explicit profile |
| Pixels, clipboard, capture, OCR | RPC inspection operations; capture writes BMP | implemented in runtime/backend |
| Conditions and workflows | modifiers, pixel masks/patterns, delayed rules, typed workflow steps; validate and dry-run | implemented |
| Observe-only session | `run`/`rpc --observe-only` and Python `Client(observe_only=True)` refuse arm and input-producing operations | implemented; status and observation remain available |
| Scenario simulation | version 1 JSON scenarios run the real engine against a scripted mock backend; bounded trace, rule effects, notifications and sounds | implemented; no operating-system input; see [simulation.md](simulation.md) |
| Recording/replay | CLI `record`/`replay`, schema-versioned JSON | implemented |
| Control | `arm`, `pause`, `abort`, `heartbeat`, `hotkeys`, status | implemented; script, execution, pause, hotkey and dormant states are separate |
| Turso Store/cache | Rust `Store`, CLI `db`, Python `Database` | implemented |
| Web GET/cache | Rust, `rhk fetch`, DB RPC `fetch`, Python `Database.fetch` | native loopback HTTP/cache tests |
| Lua widgets | selected files, `widgets`/`widget_state`/`clear_widgets`, Python helpers | sandbox, reload and bounds tests; no files means no overlay |
| Native overlay | target-relative click-through, focus-aware, capture-excluded | implemented on Windows when explicitly constructed |
| Tray | per-script icon, independent pause/edit/exit/hotkeys controls, dormant/running colors | started by `run`/`rpc`; `--no-tray` disables it; `rhk tray` provides global controls |
| Bot workspace | loopback UI, `framework_reference` and `simulate_profile` tools, bounded handoffs, provider adapters, vault-backed keys | implemented in Python package; see [bots.md](bots.md) |
| Input monitor | keyboard/mouse graphic, successful native output, simulated actions and scenarios, optional focused-page capture | bounded memory-only telemetry; see [keyboard.md](keyboard.md) |
| Script workspace | Python/Lua/JSON draft editor, revision checks, CLI and bot tools | saves never execute drafts; see [scripts.md](scripts.md) |

No capability implies a default target, default HUD, private profile, macro, calibration, or game-specific behavior. Do not claim a native/live check passed unless you ran it on the relevant Windows setup; repository tests and dry runs do not prove live input safety.

For source-level changes, preserve the generic boundary: no process memory inspection, no credentials in state or transcripts, and no live keyboard/mouse input in tests. Use fake backends or non-destructive checks as described in [CONTRIBUTING.md](../CONTRIBUTING.md).
