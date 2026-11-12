import json
import ipaddress
import threading
import unittest
from http.client import HTTPConnection
from unittest.mock import patch

from rustyhotkey.bot_server import BotError, BotService, CacheStore, CredentialVault, MemoryVault, make_handler


class DB:
    def __init__(self): self.v = {}
    def cache_get(self, n, k): return self.v.get((n, k))
    def cache_put(self, n, k, v, ttl_ms): self.v[n, k] = v


class BotTests(unittest.TestCase):
    def setUp(self):
        self.vault = MemoryVault(); self.service = BotService(CacheStore(DB()), self.vault, provider_call=lambda p, m: "answer")
        self.provider = self.service.provider({"id": "p", "kind": "custom", "name": "local", "endpoint": "http://127.0.0.1:9/v1", "model": "m", "key": "secret", "allow_insecure_http": True})

    def test_key_never_in_state(self):
        self.assertNotIn("secret", json.dumps(self.service.state()))
        self.assertNotIn("secret", json.dumps(self.provider))

    def test_fresh_state_seeds_unconfigured_master(self):
        fresh = BotService(CacheStore(DB()), self.vault, provider_call=lambda p, m: "answer")
        master = fresh.state()["bots"][0]
        self.assertEqual(master["id"], "master"); self.assertEqual(master["provider_id"], "")
        with self.assertRaises(BotError): fresh.chat("master", "hello")

    def test_threads_persist_between_services(self):
        db = DB(); first = BotService(CacheStore(db), self.vault, provider_call=lambda p, m: "answer"); first.provider({"id": "p", "kind": "custom", "endpoint": "https://example.invalid", "model": "m", "key": "secret"})
        bot = first.bot({"name": "a", "provider_id": "p"}); tid = first.chat(bot["id"], "hello")
        for _ in range(100):
            if first.task(tid)["status"] != "queued": break
        second = BotService(CacheStore(db), self.vault, provider_call=lambda p, m: "answer")
        self.assertTrue(second.state()["threads"][bot["id"]])

    def test_crud_and_async_task(self):
        bot = self.service.bot({"name": "master", "role": "expert", "provider_id": "p"})
        tid = self.service.chat(bot["id"], "draft a script")
        for _ in range(100):
            task = self.service.task(tid)
            if task["status"] != "queued": break
        self.assertEqual(task["status"], "done")

    def test_handoff_bounds(self):
        a = self.service.bot({"name": "a", "provider_id": "p"}); b = self.service.bot({"name": "b", "provider_id": "p"})
        self.assertEqual(self.service.handoff(a["id"], b["id"], "handoff"), "answer")
        with self.assertRaises(BotError): self.service.handoff(a["id"], b["id"], "x", depth=2)

    def test_provider_http_guard(self):
        lan_host = str(ipaddress.IPv4Address(0x0A000001))
        with self.assertRaises(BotError): self.service.provider({"kind": "custom", "endpoint": f"http://{lan_host}", "key": "x"})
        with self.assertRaises(BotError): self.service.provider({"kind": "custom", "endpoint": "file:///tmp", "key": "x"})

    def test_provider_removal_rejects_in_use_provider(self):
        bot = self.service.bot({"name": "uses provider", "provider_id": "p"})
        with self.assertRaisesRegex(BotError, "in use") as error:
            self.service.delete_provider("p")
        self.assertNotIn("secret", str(error.exception))
        self.assertIn("p", self.service._providers())
        self.assertEqual(self.vault.get("RustyHotkey/provider/p"), "secret")
        self.assertEqual(self.service._bots()[bot["id"]]["provider_id"], "p")

    def test_provider_removal_deletes_unreferenced_secret_and_config(self):
        self.service.delete_provider("p")
        self.assertNotIn("p", self.service._providers())
        self.assertNotIn("RustyHotkey/provider/p", self.vault.values)

    def test_provider_removal_store_failure_keeps_config_and_secret(self):
        original_put = self.service.store.put
        def fail_provider_write(namespace, key, value, ttl_ms=30 * 86_400_000):
            if namespace == "rhk.bots" and key == "providers":
                raise OSError("simulated store failure")
            return original_put(namespace, key, value, ttl_ms)
        self.service.store.put = fail_provider_write
        with self.assertRaisesRegex(BotError, "configuration and credential remain"):
            self.service.delete_provider("p")
        self.assertIn("p", self.service._providers())
        self.assertEqual(self.vault.get("RustyHotkey/provider/p"), "secret")

    def test_provider_removal_vault_failure_restores_config_and_secret(self):
        def fail_delete(_key): raise RuntimeError("simulated vault failure")
        self.vault.delete = fail_delete
        with self.assertRaisesRegex(BotError, "configuration was restored") as error:
            self.service.delete_provider("p")
        self.assertNotIn("secret", str(error.exception))
        self.assertIn("p", self.service._providers())
        self.assertEqual(self.vault.get("RustyHotkey/provider/p"), "secret")

    def test_provider_removal_tolerates_missing_credential(self):
        self.vault.delete("RustyHotkey/provider/p")
        self.service.delete_provider("p")
        self.assertNotIn("p", self.service._providers())
        self.assertNotIn("RustyHotkey/provider/p", self.vault.values)

    def test_credential_vault_delete_checks_windows_error(self):
        class Advapi:
            def __init__(self, result): self.result = result; self.calls = []
            def CredDeleteW(self, *args): self.calls.append(args); return self.result

        key = "RustyHotkey/provider/test"
        vault = CredentialVault.__new__(CredentialVault)
        vault._adv = Advapi(1)
        vault.delete(key)
        self.assertEqual(vault._adv.calls, [(key, 1, 0)])

        vault._adv = Advapi(0)
        with patch("rustyhotkey.bot_server.ctypes.get_last_error", return_value=1168):
            vault.delete(key)  # Missing credentials are idempotent success.
        with patch("rustyhotkey.bot_server.ctypes.get_last_error", return_value=5):
            with self.assertRaisesRegex(BotError, "Credential Manager delete failed") as error:
                vault.delete(key)
        self.assertNotIn(key, str(error.exception))

    def test_http_csrf_and_state(self):
        token = "test-token"; server = __import__("http.server").server.ThreadingHTTPServer(("127.0.0.1", 0), make_handler(self.service, token))
        thread = threading.Thread(target=server.serve_forever, daemon=True); thread.start()
        try:
            c = HTTPConnection("127.0.0.1", server.server_port); c.request("GET", "/api/state"); self.assertEqual(c.getresponse().status, 403)
            c.request("GET", "/api/state", headers={"Host": f"127.0.0.1:{server.server_port}", "X-RHK-Token": token}); r = c.getresponse(); self.assertEqual(r.status, 200); self.assertNotIn("secret", r.read().decode())
        finally: server.shutdown(); server.server_close()


if __name__ == "__main__": unittest.main()
