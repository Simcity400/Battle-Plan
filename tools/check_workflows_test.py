"""Tests for tools/check-workflows.py: python tools/check_workflows_test.py"""

import importlib.util
import os
import unittest

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SPEC = importlib.util.spec_from_file_location("check_workflows", os.path.join(ROOT, "tools", "check-workflows.py"))
cw = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(cw)

GOOD = """name: Comments
on:
  pull_request:
jobs:
  comments:
    runs-on: ubuntu-24.04
    steps:
      - uses: actions/checkout@v4
      - run: "python -m pip install --only-binary=:all: -r requirements.txt"
"""


class Workflows(unittest.TestCase):
    def test_a_runnable_workflow_passes(self):
        self.assertEqual(cw.problems("w.yml", GOOD), [])

    def test_an_unquoted_colon_in_a_command_does_not_parse(self):
        text = GOOD.replace('"python -m pip install --only-binary=:all: -r requirements.txt"', "python -m pip install --only-binary=:all: -r requirements.txt")
        found = cw.problems("w.yml", text)
        self.assertEqual(len(found), 1)
        self.assertIn("does not parse", found[0])

    def test_a_repeated_key_is_refused(self):
        self.assertIn("repeated key 'runs-on'", cw.problems("w.yml", GOOD.replace("    steps:", "    runs-on: windows-latest\n    steps:"))[0])

    def test_shapes_github_cannot_run_are_named(self):
        self.assertEqual(cw.problems("w.yml", "jobs: {}\n"), ["w.yml: no `on` trigger", "w.yml: `jobs` is not a non-empty mapping"])
        text = "on: push\njobs:\n  a:\n    steps:\n      - name: nothing\n      - run:\n          x: y\n"
        self.assertEqual(cw.problems("w.yml", text), [
            "w.yml: job a: neither `runs-on` nor `uses`",
            "w.yml: job a step 1: needs exactly one of `run` or `uses`",
            "w.yml: job a step 2: `run` is not a non-empty string",
        ])

    def test_an_empty_uses_or_run_is_refused(self):
        text = "on: push\njobs:\n  a:\n    runs-on: x\n    steps:\n      - uses: ''\n      - run: ' '\n  b:\n    uses: ''\n"
        self.assertEqual(cw.problems("w.yml", text), [
            "w.yml: job a step 1: `uses` names no action",
            "w.yml: job a step 2: `run` is not a non-empty string",
            "w.yml: job b: `uses` names no workflow",
        ])

    def test_a_job_needs_steps_and_a_reusable_call_takes_none(self):
        text = "on: push\njobs:\n  a:\n    runs-on: x\n    steps: []\n  b:\n    uses: ./.github/workflows/w.yml\n    steps:\n      - run: x\n  c:\n    uses: ./.github/workflows/w.yml\n"
        self.assertEqual(cw.problems("w.yml", text), [
            "w.yml: job a: `steps` is not a non-empty list",
            "w.yml: job b: a reusable-workflow call takes neither `steps` nor `runs-on`",
        ])

    def test_only_true_and_false_are_booleans_as_github_reads_them(self):
        self.assertEqual(cw.problems("w.yml", GOOD.replace("    runs-on:", "    env:\n      ON: 1\n      YES: 2\n    runs-on:")), [])
        self.assertEqual(cw.problems("w.yml", GOOD.replace("on:\n", "yes:\n")), ["w.yml: no `on` trigger"])

    def test_a_revision_that_does_not_exist_fails(self):
        with self.assertRaises(SystemExit) as stopped:
            cw.files("missing-revision-for-check-workflows-test")
        self.assertIn("Cannot list", str(stopped.exception.code))


if __name__ == "__main__":
    unittest.main()
