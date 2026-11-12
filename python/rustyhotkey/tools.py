"""Typed tool registry used by RHK BOT and embedding applications."""
from __future__ import annotations

import json
from typing import Any, Callable


class ToolError(ValueError):
    pass


def _validate(schema: dict[str, Any], value: Any, where: str = "input") -> None:
    typ = schema.get("type")
    ok = {"object": isinstance(value, dict), "array": isinstance(value, list), "string": isinstance(value, str), "number": isinstance(value, (int, float)) and not isinstance(value, bool), "integer": isinstance(value, int) and not isinstance(value, bool), "boolean": isinstance(value, bool)}
    if typ in ok and not ok[typ]: raise ToolError(f"{where} must be {typ}")
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
    if database is not None:
        r.register("cache_get", "Read a namespaced RHK cache value.", {"type": "object", "properties": {"namespace": {"type": "string"}, "key": {"type": "string"}}, "required": ["namespace", "key"], "additionalProperties": False}, lambda x: database.cache_get(x["namespace"], x["key"]))
    if fetch is not None:
        r.register("fetch", "Fetch an application-approved URL.", {"type": "object", "properties": {"url": {"type": "string"}}, "required": ["url"], "additionalProperties": False}, lambda x: fetch(x["url"]))
    return r
