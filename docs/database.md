# Embedded database

RustyHotkey exposes [`store::Store`](../src/store.rs) as a small synchronous
facade over the embedded [Turso](https://github.com/tursodatabase/turso)
engine. It supports bounded parameterized SQL, atomic batches, numbered
migrations, and a generic persistent JSON cache. Applications choose the
database path; keep it outside the repository and keep application schemas in
the consuming application.

`cache_put`, `cache_get`, `cache_invalidate`, `cache_clear`, and `cache_prune`
use the framework-owned `rhk_cache` table. Cache values are JSON and expired
entries are treated as misses and removed. The wrapper uses Turso `0.9.0-pre.5`
and is synchronous so CLI and Python bindings can call it without owning an
async runtime.

`Database.fetch` uses the native HTTP `timeout_ms` as its transport budget and
adds one second to the RPC wait for dispatch, cache, and serialization work. A
fetch can therefore use the full native timeout even when the `Database`
client's general RPC timeout is shorter.
