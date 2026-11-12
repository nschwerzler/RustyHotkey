import json
import os
import sys
import tempfile
from pathlib import Path
from unittest import TestCase
from unittest.mock import patch

ROOT = Path(__file__).parents[1]
sys.path.insert(0, str(ROOT))
from rustyhotkey import InputMonitor, MockClient


MOCK_RPC = r'''
import json, sys
assert sys.argv[1] == "mock-rpc"
profile = json.load(open(sys.argv[2], encoding="utf-8"))
observation = json.load(open(sys.argv[3], encoding="utf-8"))
now = 0
events = []
sequence = 0
def record(kind, key):
    global sequence
    sequence += 1
    events.append({"sequence": sequence, "kind": kind, "key": key, "at_ms": now})
for line in sys.stdin:
    request = json.loads(line)
    op = request.get("op")
    if op == "advance":
        now += request["ms"]
        result = {"now_ms": now}
    elif op == "virtual_time":
        result = {"now_ms": now}
    elif op == "observe":
        observation = request["observation"]
        result = {"updated": True}
    elif op == "execute":
        record("down", "KeyA")
        result = {"op": op}
    elif op == "render_widgets":
        result = {"simulated": True, "spec": [], "diagnostics": []}
    elif op == "pause":
        if events and events[-1]["kind"] == "down": record("up", "KeyA")
        result = {"armed": False}
    elif op == "input_events":
        after = request.get("after", 0)
        result = {"events": [event for event in events if event["sequence"] > after],
                  "last_sequence": sequence, "oldest_sequence": 1 if events else sequence + 1,
                  "truncated": False, "held": []}
    elif op == "trace":
        result = {"events": [
            {"frame": 0, "operation": i, "at_ms": event["at_ms"],
             "event": {"type": "key", "key": 65, "down": event["kind"] == "down"}}
            for i, event in enumerate(events)
        ]}
    else:
        result = {"op": op, "profile": profile, "observation": observation}
    print(json.dumps({"id": request.get("id"), "ok": True, "result": result,
                      "error": None}), flush=True)
'''


class MockClientTests(TestCase):
    def test_uses_only_mock_rpc_and_keeps_cleanup_trace(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fake = root / "mock_rpc.py"
            fake.write_text(MOCK_RPC, encoding="utf-8")
            env = {"APPDATA": str(root / "appdata")}
            profile = {
                "schema_version": 1,
                "target": {"window_class": "FixtureWindow", "window_title": None},
                "controls": {"abort": 112, "reset": 113, "pause": 114, "resume": 115},
                "tick_ms": 20,
                "rules": [],
            }
            observation = {
                "target_present": True,
                "focused": True,
                "keys_down": [],
                "pixels": [{"point": {"x": 0.5, "y": 0.5}, "color": 1122867}],
                "ocr": [{"text": "Ready", "region": {"x": 0, "y": 0, "width": 1, "height": 1}}],
                "clipboard": "fixture",
                "cursor": {"x": 0.5, "y": 0.5},
                "screenshot": {"width": 1, "height": 1, "bgra": [0, 0, 0, 255]},
            }
            with patch.dict(os.environ, env, clear=False):
                client = MockClient(profile, observation, command=[sys.executable, str(fake)])
                self.assertEqual(client.command[2], "mock-rpc")
                self.assertTrue(client.command[-2].endswith("profile.json"))
                self.assertTrue(client.command[-1].endswith("observation.json"))
                profile_file = Path(client.command[-2])
                fixture_dir = profile_file.parent
                self.assertEqual(json.loads(profile_file.read_text(encoding="utf-8")), profile)
                client.start()
                self.assertEqual(client.advance(25), 25)
                self.assertEqual(client.virtual_time(), 25)
                self.assertEqual(client.observe(observation), {"updated": True})
                self.assertEqual(client.render_widgets(),
                                 {"simulated": True, "spec": [], "diagnostics": []})
                client.execute([])
                client.close()
                self.assertFalse(fixture_dir.exists())
                events = client.trace()["events"]
                self.assertEqual([event["event"]["down"] for event in events], [True, False])
                self.assertEqual(events[-1]["event"]["key"], 65)
                monitor = InputMonitor()
                monitor.simulation(client.trace(), source="fixture")
                self.assertEqual(monitor.snapshot()["held"], [])
                self.assertEqual([event["key"] for event in monitor.snapshot()["events"]
                                  if event["kind"] != "reset"], ["KeyA", "KeyA"])

    def test_path_profile_is_passed_directly_and_no_live_mode_is_used(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fake = root / "mock_rpc.py"
            fake.write_text(MOCK_RPC, encoding="utf-8")
            profile = root / "profile.json"
            profile.write_text("{}", encoding="utf-8")
            observation = {"target_present": False, "focused": False}
            with patch.dict(os.environ, {"APPDATA": str(root / "appdata")}, clear=False):
                client = MockClient(profile, observation, command=[sys.executable, str(fake)],
                                    script_path="widgets.lua", no_tray=True,
                                    observe_only=True)
                self.assertEqual(client.command[2], "mock-rpc")
                self.assertEqual(client.command[3], str(profile))
                self.assertTrue(client.command[4].endswith("observation.json"))
                self.assertEqual(client.command[5:], ["--script", "widgets.lua",
                                                      "--no-tray", "--observe-only"])
                client.close()


if __name__ == "__main__":
    import unittest
    unittest.main()
