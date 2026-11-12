import unittest
from rustyhotkey.tools import ToolError, ToolRegistry, default_registry


class FakeDatabase:
    def __init__(self):
        self.values = {}
        self.fetch_calls = []

    def cache_get(self, namespace, key): return self.values.get((namespace, key))
    def cache_put(self, namespace, key, value, ttl_ms):
        self.values[(namespace, key)] = value
        return {"stored": True}
    def cache_invalidate(self, namespace, key): return self.values.pop((namespace, key), None) is not None
    def fetch(self, url, **kwargs):
        self.fetch_calls.append((url, kwargs))
        return {"body": "example"}


class ToolTests(unittest.TestCase):
    def test_schema_and_mutation_guard(self):
        r = ToolRegistry(); r.register("read", "read", {"type": "object", "properties": {"x": {"type": "string"}}, "required": ["x"], "additionalProperties": False}, lambda x: x["x"])
        r.register("write", "write", {"type": "object", "additionalProperties": False}, lambda x: "ok", mutating=True)
        self.assertEqual(r.call("read", {"x": "ok"}), "ok")
        with self.assertRaises(ToolError): r.call("read", {"x": "ok", "evil": True})
        with self.assertRaises(ToolError): r.call("write", {})
        self.assertEqual(r.call("write", {}, allow_mutating=True), "ok")

    def test_unknown_and_bad_json_are_rejected(self):
        r = ToolRegistry()
        with self.assertRaises(ToolError): r.call("unknown", {})
        with self.assertRaises(ToolError): r.call("unknown", "not-json")

    def test_default_fetch_is_bounded_https_and_callback_overrides_database(self):
        db = FakeDatabase()
        r = default_registry(db)
        self.assertEqual(r.call("fetch", {"url": "https://example.test/data"}), {"body": "example"})
        self.assertEqual(db.fetch_calls, [("https://example.test/data", {"timeout_ms": 10_000, "max_bytes": 65_536, "ttl_ms": 60_000, "allow_http": False})])
        with self.assertRaises(ToolError): r.call("fetch", {"url": "http://example.test/"})
        with self.assertRaises(ToolError): r.call("fetch", {"url": "https://user:secret@example.test/"})

        called = []
        overridden = default_registry(db, lambda url: called.append(url) or "custom")
        self.assertEqual(overridden.call("fetch", {"url": "https://example.test/"}), "custom")
        self.assertEqual(called, ["https://example.test/"])
        self.assertEqual(len(db.fetch_calls), 1)

    def test_cache_tools_are_bot_scoped_bounded_and_mutation_guarded(self):
        db = FakeDatabase()
        r = default_registry(db)
        args = {"namespace": "notes.v1", "key": "answer", "value": {"ok": True}, "ttl_ms": 86_400_000}
        with self.assertRaises(ToolError): r.call("cache_put", args)
        r.call("cache_put", args, allow_mutating=True)
        self.assertEqual(db.values[("rhk.tools.notes.v1", "answer")], {"ok": True})
        self.assertEqual(r.call("cache_get", {"namespace": "notes.v1", "key": "answer"}), {"ok": True})
        with self.assertRaises(ToolError): r.call("cache_invalidate", {"namespace": "notes.v1", "key": "answer"})
        self.assertTrue(r.call("cache_invalidate", {"namespace": "notes.v1", "key": "answer"}, allow_mutating=True))
        self.assertNotIn(("rhk.bots", "answer"), db.values)
        self.assertNotIn("cache_put", [x["function"]["name"] for x in r.schemas(include_mutating=False)])

    def test_cache_rejects_invalid_scope_ttl_and_oversized_values(self):
        r = default_registry(FakeDatabase())
        with self.assertRaises(ToolError): r.call("cache_get", {"namespace": "../rhk.bots", "key": "x"})
        for ttl in (0, 86_400_001):
            with self.assertRaises(ToolError): r.call("cache_put", {"namespace": "test", "key": "x", "value": 1, "ttl_ms": ttl}, allow_mutating=True)
        with self.assertRaises(ToolError): r.call("cache_put", {"namespace": "test", "key": "x", "value": "x" * 65_537, "ttl_ms": 1000}, allow_mutating=True)
        with self.assertRaises(ToolError): r.call("cache_put", {"namespace": "test", "key": "x", "value": float("nan"), "ttl_ms": 1000}, allow_mutating=True)

    def test_schema_integer_and_array_bounds_are_enforced(self):
        r = ToolRegistry()
        r.register("bounded", "bounded", {"type": "object", "properties": {"count": {"type": "integer", "minimum": 1, "maximum": 3}, "items": {"type": "array", "maxItems": 2, "items": {"type": "string"}}}, "required": ["count", "items"], "additionalProperties": False}, lambda x: True)
        self.assertTrue(r.call("bounded", {"count": 3, "items": ["x", "y"]}))
        for args in ({"count": 4, "items": []}, {"count": True, "items": []}, {"count": 1, "items": ["a", "b", "c"]}, {"count": 1, "items": [1]}):
            with self.assertRaises(ToolError): r.call("bounded", args)

    def test_enum_nonfinite_and_nullable_type_union_validation(self):
        r = ToolRegistry()
        r.register("choice", "choice", {"type": "object", "properties": {
            "mode": {"type": "string", "enum": ["fast", "safe"]},
            "score": {"type": "number", "minimum": 0, "maximum": 1},
            "note": {"type": ["string", "null"]},
        }, "required": ["mode", "score", "note"], "additionalProperties": False}, lambda x: x)
        self.assertEqual(r.call("choice", {"mode": "safe", "score": 0.5, "note": None})["note"], None)
        for args in (
            {"mode": "unsafe", "score": 0.5, "note": None},
            {"mode": "fast", "score": float("nan"), "note": "ok"},
            {"mode": "fast", "score": float("inf"), "note": "ok"},
            {"mode": "fast", "score": 0.5, "note": 7},
        ):
            with self.assertRaises(ToolError): r.call("choice", args)

    def test_unsupported_type_union_fails_as_tool_error(self):
        r = ToolRegistry()
        r.register("bad", "bad", {"type": ["string", "mystery"]}, lambda x: x)
        with self.assertRaises(ToolError): r.call("bad", "ok")


if __name__ == "__main__": unittest.main()
