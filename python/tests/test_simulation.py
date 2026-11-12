import json
from pathlib import Path
import subprocess
import unittest
from unittest.mock import patch

from rustyhotkey import InputMonitor, simulate
from rustyhotkey.bot_server import BotService, CacheStore, MemoryVault
from rustyhotkey.tools import ToolError, ToolRegistry


class DB:
    def __init__(self): self.values = {}
    def cache_get(self,n,k): return self.values.get((n,k))
    def cache_put(self,n,k,v,ttl_ms): self.values[n,k] = v


class SimulationTests(unittest.TestCase):
    def test_bridge_uses_data_file_and_no_shell(self):
        paths = []
        def run(command, **options):
            self.assertEqual(command[1], 'simulate'); self.assertFalse(options['shell'])
            path = Path(command[2]); paths.append(path)
            self.assertEqual(json.loads(path.read_text()), {'schema_version':1})
            return subprocess.CompletedProcess(command,0,json.dumps({'events':[],'final_status':{}}),'')
        with patch('rustyhotkey.simulation.subprocess.run',side_effect=run):
            self.assertEqual(simulate({'schema_version':1})['events'],[])
        self.assertFalse(paths[0].exists())

    def test_simulation_trace_omits_text(self):
        monitor = InputMonitor()
        monitor.simulation({'events':[{'event':{'type':'key','key':65,'down':True}},
                                      {'event':{'type':'text','text':'not logged'}},
                                      {'event':{'type':'key','key':65,'down':False}}]})
        snapshot = monitor.snapshot(); self.assertEqual(snapshot['held'],[])
        self.assertNotIn('not logged',json.dumps(snapshot))
        self.assertEqual({event['source'] for event in snapshot['events']},{'sim:scenario'})

    def test_registry_clones_close_with_owned_resources(self):
        registry = ToolRegistry(); cleaned=[]
        registry.register('read','read',{'type':'object'},lambda _:1)
        registry.add_cleanup(lambda:cleaned.append(True)); clone = registry.clone()
        self.assertEqual(registry.close(),[]); self.assertEqual(registry.close(),[])
        self.assertEqual(cleaned,[True])
        with self.assertRaises(ToolError): clone.call('read',{})

    def test_handoff_uses_read_tools_without_action_permission(self):
        service = BotService(CacheStore(DB()),MemoryVault())
        service.provider({'id':'local','kind':'custom','endpoint':'https://example.invalid','model':'example'})
        for name in ('a','b'): service.bot({'id':name,'name':name,'provider_id':'local','allow_mutating':True})
        def ask(provider,messages,registry,allow_mutating=False):
            self.assertFalse(allow_mutating)
            reference=registry.call('framework_reference',{})
            self.assertIn('Client',reference['python'])
            return 'grounded answer'
        try:
            with patch('rustyhotkey.bot_server.Provider.ask_tools',autospec=True,side_effect=ask):
                self.assertEqual(service.handoff('a','b','explain the API'),'grounded answer')
            record=service.state()['handoffs'][0]
            self.assertEqual((record['source_bot_id'],record['target_bot_id']),('a','b'))
        finally: service.close()
