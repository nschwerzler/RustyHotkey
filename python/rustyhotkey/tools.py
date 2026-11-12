"""Typed tool registry used by RHK BOT and embedding applications."""
from __future__ import annotations

import json
import math
import re
from typing import Any, Callable
from urllib.parse import urlsplit


class ToolError(ValueError):
    pass


def _validate(schema: dict[str, Any], value: Any, where: str = "input") -> None:
    typ = schema.get("type")
    matches = {
        "object": lambda x: isinstance(x, dict),
        "array": lambda x: isinstance(x, list),
        "string": lambda x: isinstance(x, str),
        "number": lambda x: isinstance(x, (int, float)) and not isinstance(x, bool),
        "integer": lambda x: isinstance(x, int) and not isinstance(x, bool),
        "boolean": lambda x: isinstance(x, bool),
        "null": lambda x: x is None,
    }
    if typ is not None:
        types = typ if isinstance(typ, list) else [typ]
        if not types or any(not isinstance(item, str) or item not in matches for item in types):
            raise ToolError(f"{where} uses an unsupported type declaration")
        if not any(matches[item](value) for item in types):
            raise ToolError(f"{where} must be one of {', '.join(types)}")
    if isinstance(value, float) and not math.isfinite(value):
        raise ToolError(f"{where} must be finite")
    if "enum" in schema and not any(
        (isinstance(value, (int, float)) and not isinstance(value, bool)
         and isinstance(option, (int, float)) and not isinstance(option, bool) and value == option)
        or (type(value) is type(option) and value == option)
        for option in schema["enum"]
    ):
        raise ToolError(f"{where} is not an allowed value")
    if typ is not None and any(item in types for item in ("integer", "number")) and isinstance(value, (int, float)) and not isinstance(value, bool):
        if "minimum" in schema and value < schema["minimum"]: raise ToolError(f"{where} is below its minimum")
        if "maximum" in schema and value > schema["maximum"]: raise ToolError(f"{where} is above its maximum")
    if isinstance(value, str):
        if "minLength" in schema and len(value) < schema["minLength"]: raise ToolError(f"{where} is too short")
        if "maxLength" in schema and len(value) > schema["maxLength"]: raise ToolError(f"{where} is too long")
    if isinstance(value, list):
        if "minItems" in schema and len(value) < schema["minItems"]: raise ToolError(f"{where} has too few items")
        if "maxItems" in schema and len(value) > schema["maxItems"]: raise ToolError(f"{where} has too many items")
        if "items" in schema:
            for i, item in enumerate(value): _validate(schema["items"], item, f"{where}[{i}]")
    if isinstance(value, dict):
        for key in schema.get("required", []):
            if key not in value: raise ToolError(f"{where}.{key} is required")
        props = schema.get("properties", {})
        if schema.get("additionalProperties") is False and any(k not in props for k in value): raise ToolError(f"{where} contains an unknown field")
        for key, child in props.items():
            if key in value: _validate(child, value[key], f"{where}.{key}")


class ToolRegistry:
    def __init__(self):
        self._tools: dict[str, dict[str, Any]] = {}
        self._lifecycle: dict[str, Any] = {'closed':False, 'cleanup':[]}
    def register(self, name: str, description: str, input_schema: dict[str, Any], handler: Callable[[dict[str, Any]], Any], mutating: bool = False) -> None:
        if not name or name in self._tools or not callable(handler): raise ToolError("invalid or duplicate tool")
        self._tools[name] = {"name": name, "description": description, "input_schema": input_schema, "handler": handler, "mutating": bool(mutating)}
    def schemas(self, include_mutating: bool = True) -> list[dict[str, Any]]:
        return [{"type": "function", "function": {"name": x["name"], "description": x["description"], "parameters": x["input_schema"]}} for x in self._tools.values() if include_mutating or not x["mutating"]]
    def call(self, name: str, arguments: Any, allow_mutating: bool = False) -> Any:
        if self._lifecycle['closed']: raise ToolError('tool registry is closed')
        tool = self._tools.get(name)
        if not tool: raise ToolError("unknown tool")
        if tool["mutating"] and not allow_mutating: raise ToolError("mutating tools are disabled")
        if isinstance(arguments, str):
            try: arguments = json.loads(arguments)
            except ValueError as e: raise ToolError("tool arguments must be JSON") from e
        _validate(tool["input_schema"], arguments)
        return tool["handler"](arguments)
    def has(self, name: str) -> bool: return name in self._tools
    def clone(self) -> "ToolRegistry":
        other = ToolRegistry(); other._tools = dict(self._tools); other._lifecycle = self._lifecycle; return other
    def add_cleanup(self, callback: Callable[[], Any]) -> None:
        if self._lifecycle['closed'] or not callable(callback): raise ToolError('invalid cleanup callback')
        self._lifecycle['cleanup'].append(callback)
    def close(self) -> list[str]:
        if self._lifecycle['closed']: return []
        self._lifecycle['closed'] = True
        errors = []
        for callback in reversed(self._lifecycle['cleanup']):
            try: callback()
            except Exception as error: errors.append(str(error)[:240])
        self._lifecycle['cleanup'].clear()
        return errors


def default_registry(database: Any | None = None, fetch: Callable[[str], Any] | None = None) -> ToolRegistry:
    """Build safe generic tools. Application-specific tools are registered by callers."""
    r = ToolRegistry()
    namespace_schema = {"type": "string", "minLength": 1, "maxLength": 64}
    key_schema = {"type": "string", "minLength": 1, "maxLength": 128}
    ttl_schema = {"type": "integer", "minimum": 1, "maximum": 86_400_000}

    def scoped(namespace: str, key: str) -> tuple[str, str]:
        if not re.fullmatch(r"[A-Za-z0-9_-]+(?:\.[A-Za-z0-9_-]+)*", namespace):
            raise ToolError("namespace must contain only letters, digits, dots, underscores, or hyphens")
        if not re.fullmatch(r"[A-Za-z0-9_.:-]+", key):
            raise ToolError("key must contain only letters, digits, dots, underscores, colons, or hyphens")
        return f"rhk.tools.{namespace}", key

    if database is not None:
        def cache_get(x: dict[str, Any]) -> Any:
            ns, key = scoped(x["namespace"], x["key"])
            return database.cache_get(ns, key)

        def cache_put(x: dict[str, Any]) -> Any:
            ns, key = scoped(x["namespace"], x["key"])
            try:
                encoded = json.dumps(x["value"], ensure_ascii=False, allow_nan=False, separators=(",", ":")).encode("utf-8")
            except (TypeError, ValueError, UnicodeError) as error:
                raise ToolError("cache value must be JSON serializable") from error
            if len(encoded) > 65_536: raise ToolError("cache value exceeds 64 KiB")
            return database.cache_put(ns, key, x["value"], x["ttl_ms"])

        def cache_invalidate(x: dict[str, Any]) -> Any:
            ns, key = scoped(x["namespace"], x["key"])
            return database.cache_invalidate(ns, key)

        r.register("cache_get", "Read a value from the tool-owned cache namespace.", {"type": "object", "properties": {"namespace": namespace_schema, "key": key_schema}, "required": ["namespace", "key"], "additionalProperties": False}, cache_get)
        r.register("cache_put", "Write a JSON value to the tool-owned cache for at most one day.", {"type": "object", "properties": {"namespace": namespace_schema, "key": key_schema, "value": {}, "ttl_ms": ttl_schema}, "required": ["namespace", "key", "value", "ttl_ms"], "additionalProperties": False}, cache_put, mutating=True)
        r.register("cache_invalidate", "Remove a value from the tool-owned cache.", {"type": "object", "properties": {"namespace": namespace_schema, "key": key_schema}, "required": ["namespace", "key"], "additionalProperties": False}, cache_invalidate, mutating=True)

    if fetch is not None or database is not None:
        def do_fetch(x: dict[str, Any]) -> Any:
            url = x["url"]
            try: parsed = urlsplit(url)
            except ValueError as error: raise ToolError("fetch URL is invalid") from error
            if parsed.scheme.lower() != "https" or not parsed.hostname or parsed.username is not None or parsed.password is not None:
                raise ToolError("fetch requires an HTTPS URL without embedded credentials")
            if fetch is not None:
                return fetch(url)
            return database.fetch(url, timeout_ms=10_000, max_bytes=65_536, ttl_ms=60_000, allow_http=False)

        r.register("fetch", "Fetch an HTTPS URL with bounded time, size, and a 60-second cache.", {"type": "object", "properties": {"url": {"type": "string", "minLength": 1, "maxLength": 4096}}, "required": ["url"], "additionalProperties": False}, do_fetch)
    return r
