"""Tests for tools/check-citations.py: python tools/check_citations_test.py"""

import importlib.util
import os
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

    def test_a_scan_that_matches_nothing_is_clean(self):
        self.assertEqual(list(cc.candidate_files(None, ['tools/no-such-file-for-this-test'])), [])


if __name__ == '__main__':
    unittest.main()
