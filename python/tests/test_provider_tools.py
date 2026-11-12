import io
import json
import unittest
from unittest.mock import patch

from rustyhotkey.bot_server import MemoryVault, Provider
from rustyhotkey.tools import ToolRegistry


class ProviderToolTests(unittest.TestCase):
    def test_anthropic_preserves_assistant_calls_and_groups_results(self):
        blocks = [{'type': 'text', 'text': 'Checking.'},
                  {'type': 'tool_use', 'id': 'one', 'name': 'read', 'input': {}},
                  {'type': 'tool_use', 'id': 'two', 'name': 'read', 'input': {}}]
        requests = []
        class Opener:
            def open(inner, request, timeout):
                requests.append(json.loads(request.data))
                result = {'content': blocks if len(requests) == 1 else [{'type': 'text', 'text': 'Done.'}]}
                return io.BytesIO(json.dumps(result).encode())
        registry = ToolRegistry()
        registry.register('read', 'Read evidence.', {'type': 'object'}, lambda _: {'value': 42})
        provider = Provider({'kind': 'anthropic', 'endpoint': 'https://example.invalid/v1', 'model': 'example'}, MemoryVault())
        with patch('rustyhotkey.bot_server.urllib.request.build_opener', return_value=Opener()):
            answer = provider.ask_tools([{'role': 'system', 'content': 'Use evidence.'},
                                         {'role': 'user', 'content': 'Check both.'}], registry)
        self.assertEqual(answer, 'Done.')
        messages = requests[1]['messages']
        self.assertEqual(messages[-2], {'role': 'assistant', 'content': blocks})
        self.assertEqual(messages[-1]['role'], 'user')
        self.assertEqual([x['tool_use_id'] for x in messages[-1]['content']], ['one', 'two'])
        self.assertTrue(all(json.loads(x['content']) == {'value': 42} for x in messages[-1]['content']))


if __name__ == '__main__': unittest.main()
