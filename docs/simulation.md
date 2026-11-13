# Scenario simulation

The Rust `simulation` module runs the real `Engine` against a scripted
`Backend`. The backend never sends operating-system input. It records keyboard,
mouse, and text `InputEvent`s, scripted notifications, per-operation status,
and the input effects of each rule tick. It does not install a keyboard hook.

A scenario contains a normal version 1 profile and a bounded sequence of
observation frames. Each frame replaces the observed target presence, focus,
keys, pixel samples, clipboard text, OCR word boxes, and cursor position, then runs its
operations. Frame times are virtual milliseconds, strictly increasing from
zero and limited to 24 hours. Output input events include their virtual
timestamp. During an action wait or click burst, the clock advances in bounded
steps, so later frames can change focus or target presence before more input is
sent. Rule cooldowns and the watchdog use the same virtual clock.

The following example shows the shape of the input. A production profile can
contain ordinary rules; the small profile here keeps the example self
contained.

```json
{
  "schema_version": 1,
  "profile": {
    "schema_version": 1,
    "target": { "window_class": "ExampleWindow", "window_title": null },
    "controls": { "abort": 112, "reset": 113, "pause": 114, "resume": 115 },
    "tick_ms": 20,
    "rules": []
  },
  "frames": [
    {
      "at_ms": 0,
      "observation": {
        "target_present": true,
        "focused": true,
        "keys_down": [],
        "pixels": [{ "point": { "x": 0.5, "y": 0.5 }, "color": 3368601 }],
        "clipboard": "ready",
        "ocr": [{ "text": "Ready", "region": { "x": 0.1, "y": 0.2, "width": 0.2, "height": 0.1 } }],
        "cursor": { "x": 0.5, "y": 0.5 }
      },
      "operations": [
        { "op": "arm" },
        { "op": "assert_condition", "condition": { "type": "clipboard_contains", "text": "ready" }, "expected": true },
        { "op": "execute", "actions": [{ "type": "tap", "key": 65 }] },
        { "op": "assert_state", "expected": { "armed": true, "held_keys": 0 } }
      ]
    }
  ]
}
```

Available operations are `arm`, `tick`, `execute`, `pause`, `abort`,
`heartbeat`, `watchdog`, `assert_condition`, and `assert_state`. Failed
operations and failed assertions are returned in the operation trace; later
operations still run, making a scenario useful for inspecting recovery paths.
An assertion checks only the state fields it supplies. Tick effects report
input events, notifications, and sounds produced while rules run, without
guessing which rule matched when the engine does not expose that detail.
At scenario end, the runner records a `cleanup_pause` step so held inputs are
released and `final_status` reflects the safe stopped state.

Call `run_scenario(scenario, state_dir)` from Rust. The supplied state directory
is used by `Engine` for its abort latch and pause state; callers should create a
unique scenario directory under `%APPDATA%/RustyHotkey/simulation` and remove it
after the engine has been dropped. Keep scenario files and build output outside
the repository. OCR conditions read the frame's scripted word boxes, join
their text in order, and optionally filter boxes to those overlapping the
requested normalized region. Each box has
non-empty text up to 4,096 bytes and a valid normalized region; each frame
supports at most 10,000 boxes within the existing combined observation and
result limits. Notifications and sound cues are returned as simulated effects.
The simulator bounds a scenario to 10,000 frames and operations, a 24 hour
virtual timeline, a combined input trace of 100,000 events/1 MiB, capped
notifications and sounds, and an 8 MiB serialized result.

```rust,no_run
use rustyhotkey::simulation::{run_scenario, SimulationScenario};
use std::path::Path;

fn run(input: &str, state_dir: &Path) -> rustyhotkey::Result<()> {
    let scenario: SimulationScenario = serde_json::from_str(input).map_err(|e| e.to_string())?;
    let result = run_scenario(scenario, state_dir)?;
    println!("{}", serde_json::to_string_pretty(&result).map_err(|e| e.to_string())?);
    Ok(())
}
```

OCR is opt-in through `ocr_contains` conditions. Live OCR calls may be
expensive, so workflows should choose polling intervals that fit their latency
and resource budget instead of checking at every engine tick.

## Interactive mock RPC

`rhk mock-rpc PROFILE [OBSERVATION.json]` starts a JSON-lines session on the
same Rust `Engine` with `SimulationBackend`. It accepts the normal RPC
operations, but all observations and input effects are simulated. `observe`
replaces the current observation; `advance {"ms":N}` advances virtual time;
`virtual_time` returns the current clock; and `trace` returns the bounded full
input event trace, including cleanup releases. The mock engine is a separate
CLI mode and never falls back to `rpc`.

Python applications can inject `MockClient` wherever their orchestration
accepts the normal `Client` API. It accepts a profile path or JSON mapping and
an initial observation mapping, and adds `observe()`, `advance()`,
`virtual_time()`, and `trace()`. `advance()` returns the new virtual time.
`trace()` returns the native trace shape and can be published through
`InputMonitor.simulation()`. The context manager pauses before collecting the
final trace and closing the mock process.

An interactive observation uses the same fields as a batch frame:
`target_present`, `focused`, optional `keys_down`, `pixels`, `clipboard`,
`ocr`, and `cursor`. It also accepts an optional `screenshot` fixture:
`{"width":640,"height":480,"solid_color":0}` paints an RGB solid color,
or `{"width":1,"height":1,"bgra":[0,0,0,255]}` supplies the complete BGRA
image. A caller-supplied file can instead be referenced as
`{"width":640,"height":480,"bmp_path":"fixture.bmp"}`. The file must be
a 32-bit uncompressed BMP with matching dimensions (top-down or bottom-up),
and cannot be combined with `bgra` or `solid_color`. It is read when the
observation is installed. Captures are bounded to 16,777,216 pixels and 64 MiB
of image data. Explicit pixel observations take precedence; other pixel queries
sample the image using normalized coordinates.
Screenshot writes, pixel queries, and OCR reads use only the active fixture;
they do not inspect a desktop or target process. Screenshot region arguments
crop the fixture using normalized coordinates. OCR boxes are filtered by
region overlap. Dynamic `observe()` payloads are bounded to 3 MiB, while
staged fixtures remain under the application data directory and are removed on
close. Mock sessions can load and update application-owned Lua widget files;
`render_widgets()` returns `{simulated, spec, diagnostics}` without creating a
real overlay.

```python
from rustyhotkey import InputMonitor, MockClient

with MockClient("profile.json", {
    "target_present": True,
    "focused": True,
    "pixels": [{"point": {"x": 0.5, "y": 0.5}, "color": 1122867}],
    "ocr": [{"text": "Ready", "region": {"x": 0.1, "y": 0.1, "width": 0.4, "height": 0.2}}],
    "screenshot": {"width": 640, "height": 480, "solid_color": 0},
}) as engine:
    engine.arm()
    engine.execute([{"type": "tap", "key": 65}])
    engine.advance(100)
    trace = engine.trace()

InputMonitor().simulation(trace, source="offline-plan")
```

The existing `rhk simulate SCENARIO.json` remains the batch format for
deterministic profile-rule tests; `MockClient` lets Python orchestration call
the same Rust APIs against the interactive fixture.
