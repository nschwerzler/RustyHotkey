import json
import os
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from unittest import TestCase
from unittest.mock import patch

ROOT = Path(__file__).parents[1]
sys.path.insert(0, str(ROOT))
from rustyhotkey import Client, Database
from rustyhotkey.script_telemetry import TelemetryForwarder, _valid_key


FAKE_RPC = r'''
import json, sys
mode = sys.argv[1]
for line in sys.stdin:
    request = json.loads(line)
    op = request.get("op")
    if op == "input_events":
        after = request.get("after", 0)
        if mode == "overflow":
            events = ([{"sequence": 9, "kind": "down", "key": "KeyB"}]
                      if after < 9 else [])
            result = {"events": events, "last_sequence": 9, "oldest_sequence": 9,
                      "truncated": after < 9, "held": ["KeyA"]}
        elif mode == "large":
            events = [{"sequence": i, "kind": "down", "key": "KeyA"}
                      for i in range(after + 1, 301)]
            result = {"events": events, "last_sequence": 300, "oldest_sequence": 1,
                      "truncated": False, "held": ["KeyA"]}
        else:
            all_events = [
                {"sequence": 1, "kind": "down", "key": "KeyA"},
                {"sequence": 2, "kind": "text", "text": "SENSITIVE_TEXT"},
                {"sequence": 3, "kind": "up", "key": "KeyA", "text": "SENSITIVE_TEXT"},
            ]
            result = {"events": [event for event in all_events if event["sequence"] > after],
                      "last_sequence": 3, "oldest_sequence": 1,
                      "truncated": False, "held": []}
    else:
        result = {"op": op}
    print(json.dumps({"id": request.get("id"), "ok": True, "result": result,
                      "error": None}), flush=True)
'''


class EventServer:
    def __init__(self):
        self.packets = []
        self.tokens = []

        owner = self

        class Handler(BaseHTTPRequestHandler):
            def do_POST(self):
                length = int(self.headers.get("Content-Length", "0"))
                body = self.rfile.read(length)
                owner.packets.append(json.loads(body))
                owner.tokens.append(self.headers.get("X-RHK-Token"))
                self.send_response(204)
                self.send_header("Content-Length", "0")
                self.end_headers()

            def log_message(self, *_):
                pass

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.url = f"http://127.0.0.1:{self.server.server_port}/api/script-input-events"

    def close(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=2)


def wait_for(predicate, timeout=3):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return True
        time.sleep(.01)
    return predicate()


class ScriptTelemetryTests(TestCase):
    def test_native_key_names_cover_dom_codes_and_bounded_vk_fallback(self):
        for key in ("ShiftRight", "ControlRight", "AltRight", "Backspace", "Delete",
                    "Numpad0", "NumpadMultiply", "NumpadDivide", "VK_08", "VK_100"):
            with self.subTest(key=key):
                self.assertEqual(_valid_key(key), key)
        for key in ("VK_10000", "VK_zz", "KeyAA", "Numpad10", "text"):
            with self.subTest(key=key):
                self.assertIsNone(_valid_key(key))

    def client(self, directory, mode="normal", client_type=Client):
        fake = Path(directory) / "fake_rpc.py"
        fake.write_text(FAKE_RPC, encoding="utf-8")
        command = [sys.executable, str(fake), mode]
        if client_type is Database:
            return Database("fixture", command=command)
        return Client("fixture", command=command, heartbeat_interval=None, close_op=None)

    def telemetry_env(self, server):
        return {
            "RHK_SCRIPT_RUN_ID": "run-fixture-1",
            "RHK_TELEMETRY_URL": server.url,
            "RHK_TELEMETRY_TOKEN": "test-secret-token",
        }

    def test_client_forwards_only_key_edges_and_resets_on_close(self):
        server = EventServer()
        try:
            with tempfile.TemporaryDirectory() as directory, patch.dict(
                    os.environ, self.telemetry_env(server)):
                client = self.client(directory)
                client.start()
                self.assertTrue(wait_for(lambda: bool(server.packets)))
                client.close()
                self.assertEqual(server.tokens[-1], "test-secret-token")
                self.assertEqual(server.packets[0], {
                    "run_id": "run-fixture-1",
                    "events": [
                        {"kind": "down", "key": "KeyA"},
                        {"kind": "up", "key": "KeyA"},
                    ],
                })
                self.assertEqual(server.packets[-1]["events"], [{"kind": "reset"}])
                self.assertNotIn("SENSITIVE_TEXT", json.dumps(server.packets))
                self.assertNotIn("test-secret-token", json.dumps(server.packets))
                self.assertTrue(client._stderr_tail.empty())
        finally:
            server.close()

    def test_overflow_resets_then_reconstructs_held_keys(self):
        server = EventServer()
        try:
            with tempfile.TemporaryDirectory() as directory, patch.dict(
                    os.environ, self.telemetry_env(server)):
                client = self.client(directory, mode="overflow")
                client.start()
                self.assertTrue(wait_for(lambda: len(server.packets) >= 2))
                client.close()
            self.assertEqual(server.packets[0]["events"], [{"kind": "reset"}])
            self.assertEqual(server.packets[1]["events"], [{"kind": "down", "key": "KeyA"}])
            self.assertEqual(server.packets[-1]["events"], [{"kind": "reset"}])
        finally:
            server.close()

    def test_packets_are_capped_at_256_events(self):
        server = EventServer()
        try:
            with tempfile.TemporaryDirectory() as directory, patch.dict(
                    os.environ, self.telemetry_env(server)):
                client = self.client(directory, mode="large")
                client.start()
                self.assertTrue(wait_for(
                    lambda: sum(len(packet["events"]) for packet in server.packets) >= 300
                ))
                client.close()
            self.assertTrue(all(len(packet["events"]) <= 256 for packet in server.packets))
        finally:
            server.close()

    def test_database_mode_and_malformed_url_do_not_attach(self):
        server = EventServer()
        try:
            bad_env = self.telemetry_env(server)
            bad_env["RHK_TELEMETRY_URL"] = server.url.replace("127.0.0.1", "localhost")
            with tempfile.TemporaryDirectory() as directory, patch.dict(os.environ, bad_env):
                client = self.client(directory)
                client.start()
                self.assertIsNone(client._script_telemetry)
                client.close()
            with tempfile.TemporaryDirectory() as directory, patch.dict(
                    os.environ, self.telemetry_env(server)):
                database = self.client(directory, client_type=Database)
                database.start()
                self.assertIsNone(database._script_telemetry)
                database.close()
            self.assertEqual(server.packets, [])
        finally:
            server.close()

    def test_stop_during_native_poll_does_not_publish_delayed_edges(self):
        server = EventServer()
        entered = threading.Event()
        release = threading.Event()

        class DelayedClient:
            def input_events(self, after=0):
                entered.set()
                release.wait(timeout=3)
                return {
                    "events": [{"sequence": 1, "kind": "down", "key": "KeyA"}],
                    "last_sequence": 1,
                    "oldest_sequence": 1,
                    "truncated": False,
                    "held": ["KeyA"],
                }

        try:
            forwarder = TelemetryForwarder(
                DelayedClient(), "127.0.0.1", server.server.server_port,
                "run-delayed", "test-secret-token",
            )
            self.assertTrue(entered.wait(timeout=1))
            forwarder.stop()
            self.assertEqual(len(server.packets), 1)
            self.assertEqual(server.packets[0]["events"], [{"kind": "reset"}])
            release.set()
            forwarder._thread.join(timeout=1)
            self.assertEqual(len(server.packets), 1)
        finally:
            release.set()
            server.close()


if __name__ == "__main__":
    import unittest
    unittest.main()
