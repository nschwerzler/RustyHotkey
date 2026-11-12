import json
import os
import subprocess
import sys
import tempfile
import threading
from pathlib import Path

import unittest

ROOT = Path(__file__).parents[1]
sys.path.insert(0, str(ROOT))
from rustyhotkey import Action, Client, Database, Point


FAKE = r'''
import json, sys, time
for line in sys.stdin:
    req = json.loads(line)
    op = req.get("op")
    if op == "sleep": time.sleep(.3)
    result = {"op": op}
    if op == "pixel": result["point"] = req.get("point")
    print(json.dumps({"id": req.get("id"), "ok": op != "bad", "result": result,
                      "error": "rejected" if op == "bad" else None}), flush=True)
'''


def client(tmp_path, **kwargs):
    fake = tmp_path / "fake.py"
    fake.write_text(FAKE, encoding="utf-8")
    return Client(tmp_path / "profile.json", command=[sys.executable, str(fake)], **kwargs)


class ClientTests(unittest.TestCase):
  def test_helpers_and_request_round_trip(self):
    with tempfile.TemporaryDirectory() as d:
      tmp_path = Path(d)
      with client(tmp_path) as c:
          self.assertEqual(c.execute([Action.tap(7), Action.wait(1)])["op"], "execute")
          self.assertEqual(c.pixel(Point(.5, .25))["point"], {"x": .5, "y": .25})


  def test_errors_and_close_pause(self):
    with tempfile.TemporaryDirectory() as d:
      c = client(Path(d)); c.start()
      with self.assertRaisesRegex(Exception, "rejected"): c.request("bad")
      c.close(); self.assertIsNone(c._process)


  def test_concurrent_requests_are_matched(self):
    with tempfile.TemporaryDirectory() as d, client(Path(d)) as c:
      out = []; threads = [threading.Thread(target=lambda: out.append(c.status())) for _ in range(8)]
      for t in threads: t.start()
      for t in threads: t.join()
      self.assertEqual(len(out), 8)

  def test_concurrent_first_requests_start_one_process(self):
    from unittest.mock import patch
    import time
    with tempfile.TemporaryDirectory() as d:
      c = client(Path(d), heartbeat_interval=None)
      barrier = threading.Barrier(8); out = []; errors = []; starts = []
      popen = subprocess.Popen
      def slow_start(*args, **kwargs):
        starts.append(1); time.sleep(.03); return popen(*args, **kwargs)
      def request():
        barrier.wait()
        try: out.append(c.status())
        except Exception as error: errors.append(error)
      try:
        with patch('rustyhotkey.client.subprocess.Popen', side_effect=slow_start):
          threads = [threading.Thread(target=request) for _ in range(8)]
          for t in threads: t.start()
          for t in threads: t.join(timeout=5)
        self.assertEqual(errors, []); self.assertEqual(len(out), 8)
        self.assertEqual(len(starts), 1)
      finally: c.close()


  def test_timeout_does_not_poison_next_request(self):
    with tempfile.TemporaryDirectory() as d, client(Path(d), timeout=.03) as c:
      with self.assertRaisesRegex(Exception, "timeout"): c.request("sleep")
      # The engine consumes requests in order; allow the timed-out operation
      # to finish before asserting that the transport still matches replies.
      import time; time.sleep(.35)
      self.assertEqual(c.status()["op"], "status")

  def test_widgets_and_database_wire_shapes(self):
    with tempfile.TemporaryDirectory() as d, client(Path(d)) as c:
      self.assertEqual(c.load_widgets(["one.lua"], {"visible": True})["op"], "widgets")
      self.assertEqual(c.update_widget_state({"visible": False})["op"], "widget_state")
      self.assertEqual(c.clear_widgets()["op"], "clear_widgets")

  def test_script_flags_and_hotkeys(self):
    with tempfile.TemporaryDirectory() as d:
      c = client(Path(d), script_path=Path(d) / "script.lua", no_tray=True, observe_only=True)
      self.assertIn("--script", c.command); self.assertIn("--no-tray", c.command)
      self.assertIn('--observe-only',c.command)
      with c:
        self.assertEqual(c.hotkeys(False)["op"], "hotkeys")

  def test_database_fetch_request_is_structured(self):
    with tempfile.TemporaryDirectory() as d:
      db = Database(Path(d) / "state.db", command=[sys.executable, str(Path(d) / "fake.py")])
      # Reuse the protocol fixture without requiring a database driver.
      Path(d, "fake.py").write_text(FAKE, encoding="utf-8")
      with db:
        result = db.fetch("https://example.invalid/data", timeout_ms=50,
                          max_bytes=200, ttl_ms=1000)
        self.assertEqual(result["op"], "fetch")


if __name__ == "__main__": unittest.main()
