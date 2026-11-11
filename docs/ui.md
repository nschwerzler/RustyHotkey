# RHK BOT UI contract

The server serves the package assets at `/`, `/app.js`, `/style.css`, and
`/portraits.png`. The browser reads the runtime token from `#token=<value>` and
sends it as `X-RHK-Token`; the launcher may remove the fragment after loading.

The JSON API is:

- `GET /api/state` → `{providers, bots, threads}` with no keys or vault IDs.
- `GET /api/templates` → generic starter bot gallery.
- `POST /api/providers` with `name`, `kind`, `base_url`/`endpoint`, `model`,
  `api_key` and optional `allow_insecure_http`.
- `POST /api/bots` with `name`, `role`, `provider_id`, `model`, `avatar_id`
  (0–5), and `accent_color`.
- `DELETE /api/providers/<id>` and `DELETE /api/bots/<id>`.
- `POST /api/chat` with `bot_id` and `message` → `task_id`.
- `GET /api/tasks/<task_id>` → queued, done, or error task state.

LAN HTTP providers require an explicit opt-in and resolve only to private,
loopback, or link-local addresses. Public providers require HTTPS. Redirects
are rejected so provider credentials cannot be forwarded to a new origin.
