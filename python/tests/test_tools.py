import unittest
from rustyhotkey.tools import ToolError, ToolRegistry


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


if __name__ == "__main__": unittest.main()
