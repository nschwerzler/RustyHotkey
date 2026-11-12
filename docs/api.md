# API contract

This is the public wire contract currently visible in `src/cli.rs`, `src/model.rs`, `src/store.rs`, and the Python package. Rust modules may expose more types than the CLI currently routes.

## Common data

`Point` is `{x:number,y:number}`. `Region` is `{x:number,y:number,width:number,height:number}`. A `Target` has optional `window_class` and `window_title`; at least one must be non-empty in a valid profile. A profile is `{schema_version:1,target,controls,tick_ms,rules?}`. Controls are distinct numeric `abort`, `reset`, `pause`, and `resume` keys. A rule has `name`, `condition`, `actions`, `cooldown_ms`, optional `delay_ms` (default `0`), and optional `enabled` (default `true`).

Actions are tagged objects: `tap {key}`, `key {key,down}`, `move {x,y}`, `click {button}`, `button {button,down}`, `click_burst {button,count,interval_ms,lock_position}`, `scroll {delta,horizontal?}`, `text {text}`, `wait {ms}`, `pause`, `set_hotkeys {enabled}`, `toggle_hotkeys`, `sound {cue}`, or `notify {message,ttl_ms}`. Keys are virtual-key values in `1..=255`; mouse buttons are `1..=5`. A click burst has `1..=1000` clicks and a positive interval, with total duration capped at 24 hours. Scroll delta must be nonzero and at most 120,000 in magnitude; `horizontal` defaults to `false`. Sound cues are `default`, `info`, `warning`, and `error`. Rule delays and action waits are bounded to 24 hours.

Conditions are `always`, `focused`, `key_down {key}`, `modifiers {shift?,control?,alt?,meta?}`, `pixel {point,color,tolerance}`, `pixel_pattern {point,pattern}`, `clipboard_contains {text}`, or boolean `all {conditions}`, `any {conditions}`, `not {condition}`. A modifier condition must specify at least one boolean state. A pixel pattern has six characters, optionally prefixed by `#`; each character is a hex digit or `?`/`*` wildcard, for example `"A?**0F"`. A workflow is `{schema_version:1,steps:[...]}`; steps are `actions {actions}`, `wait_for {condition,timeout_ms,poll_ms}`, `assert {condition}`, or `repeat {count,steps}`. A recording is `{schema_version:1,events:[{at_ms,action}]}`.

## RPC operations

Start with `rhk rpc PROFILE`. Each request needs `op`; include an `id` to correlate responses.

| op | additional fields | result |
|---|---|---|
| `status` | none | runtime state plus `focused` |
| `arm`, `pause`, `abort`, `tick` | none | runtime state |
| `heartbeat` | none | `{alive:true}` |
| `input_events` | optional `after` (nonnegative sequence) | bounded event snapshot and held keys |
| `execute` | `actions` | runtime state |
| `evaluate` | `condition` | `{matches:boolean}` |
| `pixel` | `point` | `{color:number}` |
| `clipboard` | none | `{text:string}` |
| `screenshot` | optional `region`, `path` | `{width,height,path}` |
| `ocr` | optional `region` | `{words:[{text,region}]}` |
| `notify` | `message`, `ttl_ms` | `{shown:true}` |
| `sound` | `cue` | `{played:true}` |
| `hotkeys` | `enabled` | runtime state |
| `workflow` | `workflow` | `{complete:true}` |

Runtime status includes `running` (loaded), `executing`, `armed`, `paused`,
`abort_latched`, `held_keys`, `held_buttons`, `dormant`, `hotkeys_enabled`
(effective), `requested_hotkeys`, `observe_only`, `pending_rules`, and `rules`.
Each pending rule reports its name and remaining delay; each rule reports its
name, enabled state, and remaining cooldown. Delayed rules are canceled when
their condition stops matching, the rule is disabled, or the engine pauses or
loses its target/focus. Unknown operations fail.
`rhk rpc PROFILE --observe-only` and `rhk run PROFILE --observe-only` allow
status and observation while refusing arm and input-producing operations.
Python `Client(..., observe_only=True)` selects this mode. Widget operations
are `widgets` with `{paths, state}`, `widget_state` with `{state}`, and
`clear_widgets`. `hotkeys` accepts `{enabled}`. No selected Lua files means no
overlay. The native tool catalog is available through `rhk tools`. The separate
`rhk simulate SCENARIO.json` command runs a version 1 scenario against a mock
backend with no operating-system input; see [simulation.md](simulation.md).

## Database JSON-lines

Start `rhk db DATABASE`; use the same envelope. Operations are `execute {sql,params?}` → affected row count, `query {sql,params?}` → row objects, `transaction {statements:[{sql,params?}]}` → counts, `migrate {version,sql}` → boolean, `cache_put {namespace,key,value,ttl_ms}`, `cache_get {namespace,key}` → value or `null`, `cache_invalidate`, `cache_clear`, `cache_prune` → count, and `status` → `{engine:"turso"}`. Parameters are JSON scalar values; use placeholders, never string interpolation. The framework cache uses `rhk_cache`; HTTP cache uses namespace `rhk.http`.

## Rust web module

`FetchRequest` has `url`, optional `timeout_ms` (default 15000), `max_bytes` (default 1048576), optional `cache_ttl_ms`, and `allow_http` (default false). `FetchResponse` has `url`, `status`, optional `content_type`, UTF-8 `body`, `fetched_at_ms`, and `cached`. Bounds are 1–60000 ms, 1–8388608 bytes, and URL length 1–8192 bytes. HTTPS is required unless `allow_http`; redirects, cookies, authentication, URL fragments, embedded credentials, and non-UTF-8 bodies are rejected. Use `rhk fetch URL` or the database RPC `fetch` operation with `{request: FetchRequest}`; both can use the HTTP cache.

## RHK BOT tools

The bot service provides `framework_reference` to report installed Python
interfaces and the native schema/simulation commands, and `simulate_profile`
with `{scenario}` to run a bounded scenario and return a compact failure/status
summary. It reports `native_input:false`; the trace is also published to the
Keyboard monitor as simulated events. Tool calls pass through the typed
registry. Mutating tools are disabled unless the bot's action-tools setting
allows them; handoffs never inherit that permission. See [bots.md](bots.md) and
[simulation.md](simulation.md).
