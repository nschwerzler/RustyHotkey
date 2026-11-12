"""Managed-script telemetry against a mock RPC process; never emits OS input."""
import json
import os
from pathlib import Path
import tempfile
import threading
import time
import unittest
from http.server import ThreadingHTTPServer

from rustyhotkey.bot_server import BotService, CacheStore, MemoryVault, make_handler


class DB:
    def cache_get(self, *args): return None
    def cache_put(self, *args, **kwargs): pass


PROTOCOL = '''import sys,json
for line in sys.stdin:
    request=json.loads(line)
    if request['op']=='input_events':
        events=[{'sequence':1,'kind':'down','key':'KeyA','text':'must be omitted'},
                {'sequence':2,'kind':'up','key':'KeyA'}]
        result={'events':[e for e in events if e['sequence']>request.get('after',0)],
                'last_sequence':2,'oldest_sequence':1,'held':[],'truncated':False}
    else: result={}
    print(json.dumps({'id':request.get('id'),'ok':True,'result':result}),flush=True)
'''
SCRIPT = '''import sys,time
from rustyhotkey import Client
with Client('unused.json',command=[sys.executable,'protocol.py'],observe_only=True,no_tray=True) as client:
    client.status()
    time.sleep(.6)
'''


class ManagedInputTests(unittest.TestCase):
    def test_managed_mock_rpc_events_reach_keyboard_and_omit_text(self):
        root = Path(os.environ.get('APPDATA',tempfile.gettempdir()))/'RustyHotkey'/'tests'
        root.mkdir(parents=True,exist_ok=True)
        with tempfile.TemporaryDirectory(dir=root) as directory:
            service = BotService(CacheStore(DB()),MemoryVault())
            service.configure_workspace(Path(directory)/'scripts')
            server = ThreadingHTTPServer(('127.0.0.1',0),make_handler(service,'test-token'))
            server.daemon_threads = False
            worker = threading.Thread(target=server.serve_forever,daemon=True); worker.start()
            try:
                service.script_runner.set_telemetry(f'http://127.0.0.1:{server.server_port}/api/script-input-events','test-token')
                service.workspace.save_script('protocol','py',PROTOCOL)
                draft = service.workspace.save_script('example','py',SCRIPT)
                run = service.script_runner.run('example',draft['revision'])
                deadline = time.monotonic()+8
                while time.monotonic()<deadline:
                    run = service.script_runner.status(run['id'])
                    if run['state'] not in {'running','stopping'}: break
                    time.sleep(.02)
                self.assertEqual(run['exit_code'],0,run['output'])
                service.script_runs()
                snapshot = service.input_monitor.snapshot()
                source = service._script_input_source(run)
                events = [x for x in snapshot['events'] if x['source']==source and x['kind'] in {'down','up'}]
                self.assertEqual([(x['kind'],x['key']) for x in events],[('down','KeyA'),('up','KeyA')])
                self.assertEqual(snapshot['held'],[])
                self.assertNotIn('must be omitted',json.dumps(snapshot))
                self.assertNotIn('test-token',json.dumps(snapshot))
            finally:
                server.shutdown(); server.server_close(); service.close(); worker.join(timeout=2)


if __name__ == '__main__': unittest.main()
