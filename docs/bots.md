# RustyHotkey bot workspace

`python -m rustyhotkey.bot_server --data-dir <app-data> --port 0 --no-open`
starts a generic loopback bot workspace. The launcher prints one URL containing
a runtime-only fragment token. API requests must send that value as
`X-RHK-Token`; requests with an invalid Host or Origin are rejected. The server
never enables wildcard CORS and never logs request bodies.

Providers are OpenAI-compatible (`/v1/chat/completions`), Anthropic
(` /v1/messages`), or a custom OpenAI-compatible endpoint. External endpoints
must use HTTPS; plain HTTP is limited to loopback. API keys are write-only and
stored in Windows Credential Manager. Non-Windows callers must inject a vault
for tests or an application-owned memory vault; keys are never persisted in the
RustyHotkey database, bot state, tasks, or transcripts.

Bot definitions, thread metadata, and task receipts use the RustyHotkey
Database cache API under the `rhk.bots` namespace. The master bot can explain
the installed APIs and draft scripts. Generated scripts are returned as text
and require a separate, explicit caller action to save or execute. Bot handoffs
are typed and bounded to two nested handoffs and eight turns.

The UI contract exposes `GET /api/templates` for a small generic starter
gallery, `POST /api/providers`, `POST /api/bots`, `DELETE /api/bots/<id>`,
`POST /api/chat`, and `GET /api/tasks/<id>`. Provider and bot objects returned
by the API never contain keys or vault identifiers.
