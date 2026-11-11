import hashlib
import importlib.util
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

GUARD = Path(__file__).with_name('check_public.py').resolve()
spec = importlib.util.spec_from_file_location('check_public', GUARD)
guard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(guard)


class GuardTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.repo = Path(self.temp.name)
        self.git('init', '-q')
        self.git('config', 'user.name', 'Test Author')
        self.git('config', 'user.email', 'test@example.invalid')
        self.write('note.txt', 'generic content\n')
        self.git('add', 'note.txt')
        self.git('commit', '-qm', 'initial')

    def tearDown(self):
        self.temp.cleanup()

    def git(self, *args):
        return subprocess.run(['git', *args], cwd=self.repo, check=True, capture_output=True)

    def write(self, path, content):
        p = self.repo / path
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(content, encoding='utf-8')

    def run_guard(self, *args, env=None):
        return subprocess.run([sys.executable, str(GUARD), *args], cwd=self.repo, capture_output=True, env=env)

    def test_staged_content_is_scanned_not_worktree(self):
        private = 'C' + ':\\' + 'example\\notes.txt'
        self.write('note.txt', private)
        self.git('add', 'note.txt')
        self.write('note.txt', 'clean worktree\n')
        self.assertEqual(self.run_guard().returncode, 1)
        self.write('note.txt', 'clean staged\n')
        self.git('add', 'note.txt')
        self.write('note.txt', private)
        self.assertEqual(self.run_guard().returncode, 0)

    def test_generic_code_and_loopback_pass(self):
        self.write('note.txt', 'endpoint = "http://127.0.0.1:8080"\nrelative = "assets/icon"\n')
        self.git('add', 'note.txt')
        self.assertEqual(self.run_guard().returncode, 0)

    def test_renamed_file_is_scanned(self):
        self.git('mv', 'note.txt', 'renamed.txt')
        self.write('renamed.txt', '/home/' + 'someone/data')
        self.git('add', 'renamed.txt')
        self.assertEqual(self.run_guard().returncode, 1)

    def test_private_address_and_runtime_paths_fail(self):
        self.write('note.txt', 'endpoint = "http://' + '.'.join(['192', '168', '77', '12']) + '"')
        self.git('add', 'note.txt')
        self.assertEqual(self.run_guard().returncode, 1)
        self.write('note.txt', 'generic')
        self.write('runtime/session.db', 'test')
        self.git('add', 'note.txt', 'runtime/session.db')
        self.assertEqual(self.run_guard().returncode, 1)

    def test_local_hashes_extend_policy(self):
        word = 'ConfidentialExample'
        hashes = self.repo.parent / (self.repo.name + '.hashes')
        self.addCleanup(lambda: hashes.unlink(missing_ok=True))
        hashes.write_text(hashlib.sha256(word.lower().encode()).hexdigest(), encoding='ascii')
        env = dict(os.environ, RHK_PRIVATE_MARKER_HASHES_FILE=str(hashes))
        self.write('note.txt', word)
        self.git('add', 'note.txt')
        self.assertEqual(self.run_guard(env=env).returncode, 1)

    def test_binary_allowlist_requires_exact_content(self):
        data = b'\x89PNG\x00example'
        path = 'approved/test.png'
        original = guard.APPROVED_BINARY.copy()
        self.addCleanup(lambda: (guard.APPROVED_BINARY.clear(), guard.APPROVED_BINARY.update(original)))
        guard.APPROVED_BINARY[path] = hashlib.sha256(data).hexdigest()
        self.assertIsNone(guard.inspect(path, data, set()))
        self.assertIsNotNone(guard.inspect(path, data + b'changed', set()))
        self.assertIsNotNone(guard.inspect('unknown.png', data, set()))

    def test_all_scans_unchanged_tracked_index(self):
        self.write('note.txt', 'a ' + 'C' + ':\\' + 'example\\file')
        self.git('add', 'note.txt')
        self.git('commit', '-qm', 'fixture')
        self.assertEqual(self.run_guard().returncode, 0)
        self.assertEqual(self.run_guard('--all').returncode, 1)


if __name__ == '__main__':
    unittest.main()
