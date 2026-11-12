import json
import os
from pathlib import Path
import tempfile
import threading
import unittest
from http.client import HTTPConnection
from http.server import ThreadingHTTPServer

from rustyhotkey.bot_server import BotService, CacheStore, MemoryVault, make_handler
from rustyhotkey.tools import ToolError


class DB:
    def cache_get(self, namespace, key): return None
    def cache_put(self, *args, **kwargs): pass


class ScriptApiTests(unittest.TestCase):
    def test_authenticated_save_read_conflict_and_bot_permission(self):
        root = Path(os.environ.get('APPDATA', tempfile.gettempdir())) / 'RustyHotkey' / 'tests'
        root.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=root) as directory:
            service = BotService(CacheStore(DB()), MemoryVault())
            service.configure_workspace(Path(directory))
            server = ThreadingHTTPServer(('127.0.0.1', 0), make_handler(service, 'test-token'))
            thread = threading.Thread(target=server.serve_forever, daemon=True); thread.start()
            def request(method, url, data=None, auth=True):
                conn = HTTPConnection('127.0.0.1', server.server_port, timeout=3)
                headers = {'Content-Type': 'application/json'}
                if auth: headers['X-RHK-Token'] = 'test-token'
                conn.request(method, url, json.dumps(data) if data is not None else None, headers)
                response = conn.getresponse(); raw = response.read(); status = response.status; conn.close()
                return status, json.loads(raw) if raw.startswith(b'{') else None
            try:
                self.assertEqual(request('GET', '/api/scripts', auth=False)[0], 403)
                draft = {'id': 'example', 'extension': 'py', 'text': 'value = 1\n'}
                status, saved = request('POST', '/api/scripts', draft)
                self.assertEqual(status, 201)
                self.assertEqual(request('GET', '/api/scripts/example.py')[1]['text'], draft['text'])
                self.assertEqual(request('POST', '/api/scripts', draft)[0], 409)
                self.assertEqual(request('POST', '/api/scripts', {**draft, 'expected_revision': saved['revision'], 'text': 'value = 2\n'})[0], 201)
                self.assertEqual(request('POST', '/api/scripts', {**draft, 'expected_revision': saved['revision']})[0], 409)
                self.assertEqual(request('POST', '/api/scripts', {**draft, 'id': '../escape'})[0], 400)
                with self.assertRaises(ToolError): service.registry.call('save_script', draft)
                self.assertEqual(service.registry.call('read_script', {'id':'example','extension':'py'})['text'], 'value = 2\n')
            finally:
                server.shutdown(); server.server_close(); service.close(); thread.join(timeout=2)


if __name__ == '__main__': unittest.main()
