"""Tests for tools/commit_status.py: python tools/commit_status_test.py"""

import importlib.util
import os
import subprocess
import unittest
from unittest import mock

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SPEC = importlib.util.spec_from_file_location("commit_status", os.path.join(ROOT, "tools", "commit_status.py"))
cs = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(cs)

SHA = "a" * 40


def fake_run(api):
    """A subprocess.run that resolves any revision to SHA and answers `gh api` with `api`."""
    calls = []

    def run(command, **_):
        calls.append(command)
        if command[0] == "git":
            return subprocess.CompletedProcess(command, 0, SHA + "\n", "")
        return api(command)

    return run, calls


class Post(unittest.TestCase):
    def test_without_gh_it_is_skipped(self):
        with mock.patch.object(cs.shutil, "which", return_value=None):
            self.assertEqual(cs.post("success", "x"), (False, "skipped: gh is not installed"))

    def test_it_posts_the_state_context_and_a_cut_description_on_the_resolved_commit(self):
        run, calls = fake_run(lambda command: subprocess.CompletedProcess(command, 0, "{}", ""))
        with mock.patch.object(cs.shutil, "which", return_value="gh"), mock.patch.object(cs.subprocess, "run", run):
            self.assertEqual(cs.post("failure", "d" * 200), (True, f"posted failure as local/check_changed on {SHA[:10]}"))
        command = calls[-1]
        self.assertEqual(command[:5], ["gh", "api", "--method", "POST", f"repos/{{owner}}/{{repo}}/statuses/{SHA}"])
        fields = command[6::2]
        self.assertEqual(fields[:2], ["state=failure", "context=local/check_changed"])
        self.assertEqual(len(fields[2].removeprefix("description=")), cs.LIMIT)

    def test_an_unpushed_commit_or_no_network_is_skipped_with_the_reason(self):
        unknown = lambda command: subprocess.CompletedProcess(command, 1, "", "gh: No commit found for SHA: x (HTTP 422)")
        run, _ = fake_run(unknown)
        with mock.patch.object(cs.shutil, "which", return_value="gh"), mock.patch.object(cs.subprocess, "run", run):
            posted, message = cs.post("success", "x")
        self.assertFalse(posted)
        self.assertTrue(message.endswith("(push the commit first)"))

        def offline(command, **_):
            raise subprocess.TimeoutExpired(command, 30)

        run, _ = fake_run(offline)
        with mock.patch.object(cs.shutil, "which", return_value="gh"), mock.patch.object(cs.subprocess, "run", run):
            self.assertEqual(cs.post("success", "x"), (False, "skipped: GitHub did not answer in 30 s"))


if __name__ == "__main__":
    unittest.main()
