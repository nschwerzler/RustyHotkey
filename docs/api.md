# API contract

This is the public wire contract currently visible in `src/cli.rs`, `src/model.rs`, `src/store.rs`, and the Python package. Rust modules may expose more types than the CLI currently routes.

## Common data

`Point` is `{x:number,y:number}`. `Region` is `{x:number,y:number,width:number,height:number}`. A `Target` has optional `window_class` and `window_title`; at least one must be non-empty in a valid profile. A profile is `{schema_version:1,target,controls,tick_ms,rules?}`. Controls are numeric `abort`, `reset`, `pause`, and `resume` keys. Actions are tagged objects: `tap {key}`, `key {key,down}`, `move {x,y}`, `click {button}`, `button {button,down}`, `text {text}`, `wait {ms}`, `pause`, or `notify {message,ttl_ms}`.

Conditions are `always`, `focused`, `key_down {key}`, `pixel {point,color,tolerance}`, `clipboard_contains {text}`, or boolean `all {conditions}`, `any {conditions}`, `not {condition}`. A workflow is `{schema_version:1,steps:[...]}`; steps are `actions {actions}`, `wait_for {condition,timeout_ms,poll_ms}`, `assert {condition}`, or `repeat {count,steps}`. A recording is `{schema_version:1,events:[{at_ms,action}]}`.

## RPC operations

Start with `rhk rpc PROFILE`. Each request needs `op`; include an `id` to correlate responses.

| op | additional fields | result |
|---|---|---|
| `status` | none | runtime state plus `focused` |
| `arm`, `pause`, `abort`, `tick` | none | runtime state |
| `heartbeat` | none | `{alive:true}` |
| `execute` | `actions` | runtime state |
| `evaluate` | `condition` | `{matches:boolean}` |
| `pixel` | `point` | `{color:number}` |
| `clipboard` | none | `{text:string}` |
| `screenshot` | optional `region`, `path` | `{width,height,path}` |
| `ocr` | optional `region` | `{words:[{text,region}]}` |
| `notify` | `message`, `ttl_ms` | `{shown:true}` |
| `workflow` | `workflow` | `{complete:true}` |

Runtime status includes `running` (loaded), `executing`, `armed`, `paused`,
`abort_latched`, `held_keys`, `held_buttons`, `dormant`, `hotkeys_enabled`
(effective) and `requested_hotkeys`. Unknown operations fail. Widget operations
are `widgets` with `{paths, state}`, `widget_state` with `{state}`, and
`clear_widgets`. `hotkeys` accepts `{enabled}`. No selected Lua files means no
overlay. The native tool catalog is available through `rhk tools`.

## Database JSON-lines

Start `rhk db DATABASE`; use the same envelope. Operations are `execute {sql,params?}` → affected row count, `query {sql,params?}` → row objects, `transaction {statements:[{sql,params?}]}` → counts, `migrate {version,sql}` → boolean, `cache_put {namespace,key,value,ttl_ms}`, `cache_get {namespace,key}` → value or `null`, `cache_invalidate`, `cache_clear`, `cache_prune` → count, and `status` → `{engine:"turso"}`. Parameters are JSON scalar values; use placeholders, never string interpolation. The framework cache uses `rhk_cache`; HTTP cache uses namespace `rhk.http`.

## Rust web module

`FetchRequest` has `url`, optional `timeout_ms` (default 15000), `max_bytes` (default 1048576), optional `cache_ttl_ms`, and `allow_http` (default false). `FetchResponse` has `url`, `status`, optional `content_type`, UTF-8 `body`, `fetched_at_ms`, and `cached`. Bounds are 1–60000 ms, 1–8388608 bytes, and URL length 1–8192 bytes. HTTPS is required unless `allow_http`; redirects, cookies, authentication, URL fragments, embedded credentials, and non-UTF-8 bodies are rejected. The module is not currently routed through CLI/RPC.
