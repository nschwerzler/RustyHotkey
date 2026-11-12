# Scenario simulation

The Rust `simulation` module runs the real `Engine` against a scripted
`Backend`. The backend never sends operating-system input. It records keyboard,
mouse, and text `InputEvent`s, scripted notifications, per-operation status,
and the input effects of each rule tick. It does not install a keyboard hook.

A scenario contains a normal version 1 profile and a bounded sequence of
observation frames. Each frame replaces the observed target presence, focus,
keys, pixel samples, clipboard text, and cursor position, then runs its
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
the repository. The simulator supports observations used by conditions and
input safety. Screenshot and OCR operations return an explicit unsupported
error, and notifications and sound cues are returned as simulated effects.
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
