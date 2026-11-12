# Keyboard and mouse monitor

Open **Keyboard** in RHK BOT to see a keyboard graphic, mouse buttons, held
inputs and a recent event timeline. Presses glow while held; releases flash.
The display distinguishes live script output from simulated actions and local
page input. The bounded feed reports event gaps and restores current held state.

**Play simulated demo** exercises the display without sending native input.
**Capture focused page input** is opt-in and local to the browser page. Select
the keyboard surface to try it. Text fields and controls are excluded; changing
focus, hiding the page or closing the monitor releases local highlights. This
is not a system-wide keyboard listener.

The dialog also contains a collapsible **Scenario what-if simulation** editor.
Paste a version 1 scenario or load its generic sample, then choose **Run
what-if**. The editor checks JSON and caps requests at 512 KiB. The authenticated
`POST /api/simulate` endpoint runs the bounded Rust simulator against its mock
backend and returns the operation trace and final status. On success, the
browser starts a playback of that result using its virtual timestamps. The
timeline marks each frame's focus/target state and each key/button event; Play,
Pause, Restart, and the speed selector control the playback. Same-time taps get
a short visual hold, the display is capped at 600 rows, and playback clears all
simulated held keys/buttons when it finishes. Playback starts automatically
unless the browser requests reduced motion. Simulation telemetry may also
appear in the shared feed as `sim:scenario`. Playback only changes this page's
display and never sends operating-system input. The CLI and Rust scenario API
are documented in [simulation.md](simulation.md).

## Script integration

Python scripts launched through the app's **Run saved Python** action attach
their RHK clients automatically. Each run has a separate live source. Closing
a client or finishing a managed run clears that source's displayed held keys.
The authenticated loopback bridge carries bounded key/button events only;
it does not install a global keyboard hook or forward typed text. Standalone
CLI runs can use the explicit integration below in an embedding application.

Embedding applications share the bot service's `input_monitor` with their
script clients:

```python
monitor = service.input_monitor
monitor.attach(client, source="example")  # observe successful native output
monitor.actions("example", [            # visualize intention; no execution
    {"type": "tap", "key": 65},
    {"type": "click", "button": 1},
])
```

`attach` polls `Client.input_events(after=0)` and labels output `live:example`.
`actions` labels intended events `sim:example`. Neither call creates a global
input hook. Native text contents are omitted; mouse movement is not displayed.
Close an owned `InputMonitor` during application shutdown.

Native RPC `input_events` accepts a nonnegative `after` cursor. Its snapshot
contains `events`, `last_sequence`, `oldest_sequence`, `truncated`, `held` and
`limit` (512). Events contain `sequence`, `kind`, `key` and `at_ms`; `kind` is
`down` or `up` and `key` uses DOM-style codes such as `KeyA`, `ShiftLeft`,
`Digit1` and `Mouse1`. The native held snapshot contains key-code strings.

The authenticated UI API `GET /api/input-events?after=0` adds source identifiers
and returns held entries as `{source, key}`. `POST /api/input-events` accepts
`{source, events: [{kind, key}]}` for telemetry only: it always labels these
events simulated and never emits operating-system input. A source `reset` event
clears its held state. Batches are bounded to 256 entries. This feed is transient
in memory, never a database of typed text.

Scenario simulation also scripts target presence, focus, pressed keys, pixel
samples, clipboard text and cursor position between operations. The scenario
playback timeline marks frame focus/target state and input events. The shared
monitor feed displays simulated key/button events; text contents and pointer
movement remain omitted.
