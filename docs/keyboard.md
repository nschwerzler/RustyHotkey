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

## Script integration

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

The monitor visualizes output; it does not simulate a target application's
responses, pixel state, focus or workflow conditions. A preview remains distinct
from a full scenario simulator.
