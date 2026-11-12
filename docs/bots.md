# RustyHotkey bot workspace

`python -m rustyhotkey.bot_server --data-dir <app-data> --port 0 --no-open`
starts a generic loopback bot workspace. The launcher prints one URL containing
a runtime-only fragment token. API requests must send that value as
`X-RHK-Token`; requests with an invalid Host or Origin are rejected. The server
never enables wildcard CORS and never logs request bodies.

Providers are OpenAI-compatible (`/v1/chat/completions`), Anthropic
(` /v1/messages`), or a custom OpenAI-compatible endpoint. External endpoints
must use HTTPS by default; plain HTTP requires explicit opt-in and is limited
to private, loopback, or link-local addresses. API keys are write-only and
stored in Windows Credential Manager. Non-Windows callers must inject a vault
for tests or an application-owned memory vault; keys are never persisted in the
RustyHotkey database, bot state, tasks, or transcripts.

Bot definitions, thread metadata, and handoff receipts use the RustyHotkey
Database cache API under the `rhk.bots` namespace. The service registers two
built-in tools: `framework_reference` reports the installed Python interfaces
and native schema/simulation commands; `simulate_profile` runs a bounded
scenario against the mock backend, returns event/failure/final-status counts,
and publishes its trace to Keyboard. Simulation never emits operating-system
input. The master bot can explain the installed APIs and draft scripts.
Generated scripts are returned as text and require a separate, explicit caller
action to save or execute.

Bot tool calls use the typed `ToolRegistry`, which validates arguments against
each registered JSON schema. Mutating tools are excluded unless the bot's
action-tools setting enables them. Handoffs are bounded to two nested levels;
provider tool-use loops are capped at eight rounds. Handoffs record their
source, target, message and response, and treat the incoming message as
untrusted context. A handoff can call read-only tools but never inherits
permission to emit input. The registry and its clones share one lifecycle;
`close()` runs registered cleanup callbacks once, in reverse registration
order.

The authenticated UI contract exposes `GET /api/state`, `GET /api/templates`,
`GET /api/tasks/<id>`, `POST /api/providers`, `POST /api/bots`,
`POST /api/chat`, `POST /api/simulate`, and `DELETE /api/providers/<id>` or
`DELETE /api/bots/<id>`. `POST /api/chat` returns a task ID; task state moves
from `queued` to `done` or `error`, and an unknown task returns `missing`.
Threads retain at most 64 messages per bot; handoff receipts retain the most
recent 100. Task state is process-local, while bot configuration, threads and
handoff receipts use the database cache.

The **Scripts** editor and `list_scripts`/`read_script`/`save_script` tools share
a file-backed draft workspace under the app data directory. Updates require
the current revision and saving never executes code. See [scripts.md](scripts.md)
for the API, syntax checks and bounds.

`POST /api/input-events` is a separate telemetry-only endpoint. All API routes
require the runtime token and reject invalid Host or Origin values. Provider
and bot objects returned by the API never contain keys or vault identifiers.
When the service stops, it closes the input monitor and tool registry and shuts
down the task pool, cancelling work that has not started.

The default tool registry also provides HTTPS fetch and a tool-owned cache under
`rhk.tools.<namespace>`. Cache writes and invalidation are guarded as mutating
tools; fetch has fixed time, response-size, and cache bounds. See
[tools.md](tools.md) for the tool schemas and limits.
