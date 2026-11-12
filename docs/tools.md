# Built-in BOT tools

`default_registry(database)` adds generic database tools when a database is
available. `cache_get`, `cache_put`, and `cache_invalidate` address only the
`rhk.tools.<namespace>` cache space. The namespace must use letters, digits,
underscores, hyphens, and dot-separated components; keys are limited to 128
characters. Cache values must be JSON serializable and at most 64 KiB. Cache
write and invalidate calls are mutating tools, so the BOT must have
`allow_mutating` enabled and the call must pass the registry mutation guard.
TTL values are required and range from 1 ms through one day.

When a database is available, `fetch` is registered automatically. It accepts
an HTTPS URL without embedded credentials and uses the native `Database.fetch`
path with a 10-second timeout, a 64 KiB response limit, a 60-second cache TTL,
and HTTP disabled. Native fetch does not send caller credentials or cookies and
does not follow redirects. A supplied `fetch(url)` callback takes precedence
over the database fallback and retains its existing one-argument signature;
it still receives only HTTPS URLs, while its own transport policy belongs to
the callback owner.

The registry checks tool inputs against their JSON schema, including integer
minimum/maximum values and array item limits. It checks again in Python before
calling each handler. Custom tools can be registered on a `ToolRegistry`; mark
any persistent writes or other state changes `mutating=True` so they remain
behind the same guard.
