import concurrent.futures
import tempfile
import threading
import unittest
from pathlib import Path

from rustyhotkey.scripts import (
    RevisionConflictError, ScriptWorkspace, ScriptWorkspaceError,
)


class ScriptWorkspaceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.workspace = ScriptWorkspace(Path(self.temp.name) / "drafts", max_file_bytes=512)

    def test_round_trip_and_revision_conflict(self):
        created = self.workspace.save_script("sample", "py", "answer = 42\n")
        self.assertEqual(created["draft_status"], "draft")
        self.assertEqual(created["validation"], "python_ast_valid")
        read = self.workspace.read_script("sample", "py")
        self.assertEqual(read["text"], "answer = 42\n")
        updated = self.workspace.save_script("sample", "py", "answer = 43\n", read["revision"])
        self.assertNotEqual(updated["revision"], read["revision"])
        with self.assertRaises(RevisionConflictError):
            self.workspace.save_script("sample", "py", "answer = 0\n", read["revision"])
        with self.assertRaises(RevisionConflictError):
            self.workspace.save_script("sample", "py", "blind = True\n")

    def test_safe_ids_size_and_syntax_validation(self):
        for script_id in ("../escape", "a/b", "", ".", "x" * 65):
            with self.subTest(script_id=script_id), self.assertRaises(ScriptWorkspaceError):
                self.workspace.save_script(script_id, "py", "x = 1")
        with self.assertRaises(ScriptWorkspaceError):
            self.workspace.save_script("broken", "py", "def nope(:\n")
        with self.assertRaises(ScriptWorkspaceError):
            self.workspace.save_script("broken", "json", "{")
        with self.assertRaises(ScriptWorkspaceError):
            self.workspace.save_script("large", "lua", "x" * 513)
        self.assertFalse((Path(self.temp.name) / "escape.py").exists())

    def test_symlink_is_never_read_or_overwritten(self):
        outside = Path(self.temp.name) / "outside.py"
        outside.write_text("safe = True", encoding="utf-8")
        link = self.workspace.root / "linked.py"
        try:
            link.symlink_to(outside)
        except (OSError, NotImplementedError):
            self.skipTest("symlinks unavailable")
        with self.assertRaises(ScriptWorkspaceError):
            self.workspace.read_script("linked", "py")
        with self.assertRaises(ScriptWorkspaceError):
            self.workspace.save_script("linked", "py", "changed = True")
        self.assertEqual(outside.read_text(encoding="utf-8"), "safe = True")
        self.assertNotIn("linked", [item["id"] for item in self.workspace.list_scripts()])

    def test_concurrent_updates_allow_only_one_revision_winner(self):
        original = self.workspace.save_script("race", "json", '{"v":0}')
        gate = threading.Barrier(2)

        def update(value):
            gate.wait()
            try:
                return self.workspace.save_script("race", "json", '{"v":%d}' % value,
                                                  original["revision"])
            except RevisionConflictError:
                return "conflict"

        with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
            results = list(pool.map(update, (1, 2)))
        self.assertEqual(sum(result == "conflict" for result in results), 1)
        self.assertEqual(self.workspace.read_script("race", "json")["text"] in ('{"v":1}', '{"v":2}'), True)

    def test_python_draft_is_compiled_but_never_executed(self):
        marker = Path(self.temp.name) / "executed.txt"
        code = f"open({str(marker)!r}, 'w').write('ran')\n"
        self.workspace.save_script("side_effect", "py", code)
        self.assertFalse(marker.exists())

    def test_script_count_is_bounded_for_listing_and_creation(self):
        bounded = ScriptWorkspace(Path(self.temp.name) / "bounded", max_scripts=1)
        bounded.save_script("first", "lua", "return {}")
        with self.assertRaises(ScriptWorkspaceError):
            bounded.save_script("second", "lua", "return {}")
        (bounded.root / "second.lua").write_text("return {}", encoding="utf-8")
        with self.assertRaises(ScriptWorkspaceError):
            bounded.list_scripts()


if __name__ == "__main__":
    unittest.main()
