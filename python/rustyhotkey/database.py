"""Turso-backed database client exposed by the RustyHotkey CLI."""

from __future__ import annotations

from pathlib import Path
from typing import Any, Mapping, Sequence

from .client import Client


class Database(Client):
    """Use the Rust database bridge without installing a Python DB driver."""

    def __init__(self, path: str | Path, *, command: Sequence[str] | None = None,
                 timeout: float = 5.0, close_timeout: float = 1.0) -> None:
        super().__init__(path, command=command, timeout=timeout,
                         close_timeout=close_timeout, mode="db", close_op=None,
                         heartbeat_interval=None)

    def execute(self, sql: str, params: Sequence[Any] = ()) -> Any:
        return self.request("execute", sql=sql, params=list(params))

    def query(self, sql: str, params: Sequence[Any] = ()) -> Any:
        return self.request("query", sql=sql, params=list(params))

    def transaction(self, statements: Sequence[Mapping[str, Any]]) -> Any:
        return self.request("transaction", statements=list(statements))

    def migrate(self, version: int, sql: str) -> Any:
        return self.request("migrate", version=version, sql=sql)

    def fetch(self, url: str, *, timeout_ms: int = 10_000, max_bytes: int = 1_000_000,
              ttl_ms: int | None = None, allow_http: bool = False) -> Any:
        request = {"url": url, "timeout_ms": timeout_ms, "max_bytes": max_bytes,
                   "allow_http": allow_http}
        if ttl_ms is not None:
            request["cache_ttl_ms"] = ttl_ms
        # Native HTTP may run for the full requested timeout. Leave a small
        # allowance for RPC dispatch, cache work, and JSON serialization.
        rpc_timeout = max(self.timeout, timeout_ms / 1000 + 1.0)
        return self.request("fetch", timeout=rpc_timeout, request=request)

    def cache_put(self, namespace: str, key: str, value: Any, ttl_ms: int) -> Any:
        return self.request("cache_put", namespace=namespace, key=key, value=value, ttl_ms=ttl_ms)

    def cache_get(self, namespace: str, key: str) -> Any:
        return self.request("cache_get", namespace=namespace, key=key)

    def cache_invalidate(self, namespace: str, key: str) -> Any:
        return self.request("cache_invalidate", namespace=namespace, key=key)

    def cache_clear(self, namespace: str) -> Any:
        return self.request("cache_clear", namespace=namespace)

    def cache_prune(self) -> Any:
        return self.request("cache_prune")
