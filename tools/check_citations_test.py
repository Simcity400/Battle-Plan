"""Tests for tools/check-citations.py: python tools/check_citations_test.py"""

import importlib.util
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SPEC = importlib.util.spec_from_file_location('check_citations', os.path.join(ROOT, 'tools', 'check-citations.py'))
cc = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(cc)


class CandidateFiles(unittest.TestCase):
    def setUp(self):
        self.cwd = os.getcwd()
        os.chdir(ROOT)

    def tearDown(self):
        os.chdir(self.cwd)

    def test_a_revision_git_rejects_fails_the_check_instead_of_scanning_nothing(self):
        with self.assertRaises(SystemExit) as raised:
            list(cc.candidate_files('refs/heads/no-such-revision-for-this-test', ['tools']))
        self.assertIn('git grep failed', str(raised.exception))


    def test_untracked_files_are_scanned_whatever_their_names(self):
        with tempfile.TemporaryDirectory() as folder:
            subprocess.run(['git', 'init', '-q', folder], check=True)
            os.chdir(folder)
            names = ['untracked.md', 'caf\u00e9.md']
            for name in names:
                Path(folder, name).write_text('Offset only', encoding='utf-8')
            self.assertEqual(sorted(cc.candidate_files(None, ['.'])), sorted(names))
            os.chdir(self.cwd)


if __name__ == '__main__':
    unittest.main()
