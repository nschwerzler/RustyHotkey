import json
import threading
import time
import unittest
from http.client import HTTPConnection
from http.server import ThreadingHTTPServer

from rustyhotkey.input_monitor import InputMonitor, key_code
from rustyhotkey.bot_server import BotService, CacheStore, MemoryVault, make_handler


class DB:
    def cache_get(self, *args): return None


class InputMonitorTests(unittest.TestCase):
    def test_source_isolation_and_reset(self):
        monitor = InputMonitor()
        monitor.publish('one', [{'kind': 'down', 'key': 'KeyA'}])
        monitor.publish('two', [{'kind': 'down', 'key': 'KeyA'}])
        monitor.publish('one', [{'kind': 'reset'}])
        self.assertEqual(monitor.snapshot()['held'], [{'source': 'two', 'key': 'KeyA'}])

    def test_ring_overflow_retains_held_snapshot(self):
        monitor = InputMonitor()
        for _ in range(3): monitor.publish('one', [{'kind': 'down', 'key': 'KeyA'}] * 200)
        snapshot = monitor.snapshot(1)
        self.assertEqual(len(snapshot['events']), 512)
        self.assertEqual(snapshot['last_sequence'], 600)
        self.assertTrue(snapshot['truncated'])
        self.assertEqual(snapshot['held'], [{'source': 'one', 'key': 'KeyA'}])
        self.assertEqual(monitor.snapshot(600)['events'], [])

    def test_invalid_batch_is_atomic_and_rejects_text(self):
        monitor = InputMonitor()
        with self.assertRaises(ValueError):
            monitor.publish('one', [{'kind': 'down', 'key': 'KeyA'}, {'kind': 'text', 'key': 'secret'}])
        self.assertEqual(monitor.snapshot()['last_sequence'], 0)
        with self.assertRaises(ValueError): monitor.publish('one', [{'kind': 'down', 'key': 'KeyA', 'text': 'secret'}])
        with self.assertRaises(ValueError): monitor.publish('one', [{'kind': 'down', 'key': 'KeyA'}] * 257)
        with self.assertRaises(ValueError): monitor.snapshot(-1)

    def test_intended_actions_are_simulated_and_text_is_omitted(self):
        monitor = InputMonitor()
        monitor.actions('example', [{'type': 'tap', 'key': 65}, {'type': 'click', 'button': 2},
                                    {'type': 'text', 'text': 'do not log this'}])
        snapshot = monitor.snapshot()
        self.assertEqual([e['key'] for e in snapshot['events']], ['KeyA', 'KeyA', 'Mouse2', 'Mouse2'])
        self.assertEqual({e['source'] for e in snapshot['events']}, {'sim:example'})
        self.assertEqual(snapshot['held'], [])
        self.assertNotIn('do not log this', json.dumps(snapshot))
        self.assertEqual(key_code(163), 'ControlRight')

    def test_client_disconnect_clears_keys(self):
        class Client:
            _closed = False
            def input_events(self, after):
                return {'events': [{'kind': 'down', 'key': 'KeyA'}] if after == 0 else [], 'last_sequence': 1}
        client = Client(); monitor = InputMonitor()
        try:
            monitor.attach(client, 'one')
            for _ in range(100):
                if monitor.snapshot()['held']: break
                time.sleep(.01)
            self.assertEqual(monitor.snapshot()['held'], [{'source': 'live:one', 'key': 'KeyA'}])
            client._closed = True
            for _ in range(100):
                if not monitor.snapshot()['held']: break
                time.sleep(.01)
            self.assertEqual(monitor.snapshot()['held'], [])
        finally: monitor.close()

    def test_http_requires_auth_and_accepts_only_simulation(self):
        service = BotService(CacheStore(DB()), MemoryVault())
        server = ThreadingHTTPServer(('127.0.0.1', 0), make_handler(service, 'test-token'))
        thread = threading.Thread(target=server.serve_forever, daemon=True); thread.start()
        connection = HTTPConnection('127.0.0.1', server.server_port)
        headers = {'X-RHK-Token': 'test-token', 'Content-Type': 'application/json'}
        try:
            connection.request('POST', '/api/input-events', json.dumps({'source': 'demo', 'events': []}))
            response = connection.getresponse(); self.assertEqual(response.status, 403); response.read()
            connection.request('POST', '/api/input-events', json.dumps({'source': 'demo', 'events': [{'kind': 'down', 'key': 'KeyA'}]}), headers)
            response = connection.getresponse(); self.assertEqual(response.status, 201); response.read()
            connection.request('GET', '/api/input-events?after=0', headers=headers)
            response = connection.getresponse(); body = json.loads(response.read())
            self.assertEqual(body['held'], [{'source': 'sim:demo', 'key': 'KeyA'}])
            connection.request('GET', '/api/input-events?after=bad', headers=headers)
            response = connection.getresponse(); self.assertEqual(response.status, 400); response.read()
        finally:
            connection.close(); server.shutdown(); server.server_close(); thread.join()
            service.input_monitor.close(); service.pool.shutdown()
