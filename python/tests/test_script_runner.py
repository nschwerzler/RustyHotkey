import json
import os
import tempfile
import time
import unittest
from pathlib import Path

from rustyhotkey.script_runner import (
    MAX_OUTPUT_BYTES, ScriptRunner, ScriptRunnerError,
)
from rustyhotkey.scripts import ScriptConflict, ScriptWorkspace


class ScriptRunnerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / "scripts"
        self.workspace = ScriptWorkspace(self.root)
        self.runner = ScriptRunner(self.workspace)
        self.addCleanup(self.runner.close)

    def wait_for_terminal(self, run_id, timeout=5):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            receipt = self.runner.status(run_id)
            if receipt["state"] in ("exited", "failed"):
                return receipt
            time.sleep(0.02)
        self.fail("script process did not finish before timeout")

    @staticmethod
    def text(receipt, stream=None):
        chunks = receipt["output"]
        if stream is not None:
            chunks = [chunk for chunk in chunks if chunk["stream"] == stream]
        return "".join(chunk["text"] for chunk in chunks)

    def test_runs_exact_revision_with_runtime_environment_and_workspace_cwd(self):
        (self.root / "asset.txt").write_text("caller asset", encoding="utf-8")
        source = (
            "import json, os, pathlib, time\n"
            "time.sleep(0.15)\n"
            "print(json.dumps({'path': os.environ['RHK_SCRIPT_PATH'], 'rev': os.environ['RHK_SCRIPT_REVISION'], "
            "'python': os.environ['RHK_PYTHON'], 'cwd': os.getcwd(), 'asset': pathlib.Path('asset.txt').read_text(), "
            "'snapshot': __file__, 'pythonpath': os.environ['PYTHONPATH']}))\n"
            "print('stderr marker', file=__import__('sys').stderr)\n"
        )
        draft = self.workspace.save_script("sample", "py", source)
        receipt = self.runner.run("sample", draft["revision"])
        self.assertIn(receipt["state"], ("running", "exited"))
        snapshot = self.runner.run_root / receipt["id"] / "sample.py"
        self.assertEqual(snapshot.read_text(encoding="utf-8"), source)
        self.workspace.save_script("sample", "py", "print('edited after run started')\n", draft["revision"])
        finished = self.wait_for_terminal(receipt["id"])
        self.assertEqual(finished["state"], "exited")
        self.assertEqual(finished["exit_code"], 0)
        output = json.loads(self.text(finished, "stdout"))
        self.assertEqual(output["path"], str(self.root / "sample.py"))
        self.assertEqual(output["rev"], draft["revision"])
        self.assertTrue(output["python"])
        self.assertEqual(output["cwd"], str(self.root))
        self.assertEqual(output["asset"], "caller asset")
        self.assertTrue(output["snapshot"].startswith(str(self.runner.run_root)))
        self.assertIn(str(self.root), output["pythonpath"].split(os.pathsep))
        self.assertIn("stderr marker", self.text(finished, "stderr"))

    def test_stale_revision_and_non_python_drafts_cannot_run(self):
        first = self.workspace.save_script("sample", "py", "print('old')\n")
        self.workspace.save_script("sample", "py", "print('new')\n", first["revision"])
        with self.assertRaises(ScriptConflict):
            self.runner.run("sample", first["revision"])
        lua = self.workspace.save_script("widget", "lua", "return {}")
        with self.assertRaises(ScriptRunnerError):
            self.runner.run("widget", lua["revision"])

    def test_exception_is_reported_as_failed_with_bounded_receipt_output(self):
        draft = self.workspace.save_script(
            "failure", "py", "print('before error')\nraise RuntimeError('expected test error')\n"
        )
        receipt = self.wait_for_terminal(self.runner.run("failure", draft["revision"])["id"])
        self.assertEqual(receipt["state"], "failed")
        self.assertNotEqual(receipt["exit_code"], 0)
        self.assertIn("expected test error", self.text(receipt, "stderr"))

    def test_large_output_is_drained_but_receipt_stays_bounded(self):
        draft = self.workspace.save_script("output", "py", "print('x' * 200000)\n")
        receipt = self.wait_for_terminal(self.runner.run("output", draft["revision"])["id"])
        self.assertEqual(receipt["state"], "exited")
        self.assertLessEqual(sum(len(c["text"].encode("utf-8")) for c in receipt["output"]), MAX_OUTPUT_BYTES)
        self.assertTrue(receipt["output_truncated"])

    def test_stop_terminates_only_the_direct_script_process(self):
        draft = self.workspace.save_script("sleeper", "py", "import time\ntime.sleep(30)\n")
        receipt = self.runner.run("sleeper", draft["revision"])
        stopped = self.runner.stop(receipt["id"])
        self.assertIn(stopped["state"], ("exited", "failed"))
        self.assertIsNotNone(stopped["exit_code"])

    def test_four_process_limit_and_close_stop_children(self):
        draft = self.workspace.save_script("sleeper", "py", "import time\ntime.sleep(30)\n")
        receipts = [self.runner.run("sleeper", draft["revision"]) for _ in range(4)]
        with self.assertRaises(ScriptRunnerError):
            self.runner.run("sleeper", draft["revision"])
        self.runner.close()
        self.assertTrue(all(self.runner.status(item["id"])["state"] in ("exited", "failed") for item in receipts))
        with self.assertRaises(ScriptRunnerError):
            self.runner.run("sleeper", draft["revision"])

    def test_completed_history_is_capped_and_run_ids_are_type_checked(self):
        draft = self.workspace.save_script("quick", "py", "print('done')\n")
        run_ids = []
        for _ in range(35):
            run_id = self.runner.run("quick", draft["revision"])["id"]
            run_ids.append(run_id)
            self.wait_for_terminal(run_id)
        self.assertEqual(len(self.runner.list_runs()), 32)
        with self.assertRaises(ScriptRunnerError):
            self.runner.status([run_ids[-1]])
        with self.assertRaises(ScriptRunnerError):
            self.runner.status(run_ids[0])


if __name__ == "__main__":
    unittest.main()
