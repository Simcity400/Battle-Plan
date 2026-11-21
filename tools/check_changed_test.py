"""Tests for tools/check_changed.py: python tools/check_changed_test.py"""

import contextlib
import importlib.util
import io
import os
import sys
import types
import unittest
from unittest import mock

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SPEC = importlib.util.spec_from_file_location("check_changed", os.path.join(ROOT, "tools", "check_changed.py"))
cc = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(cc)


def package_of(folder):
    return None if folder == "gone" else folder


def plan(files):
    return cc.plan(files, "BASE", package_of, "py", "npm")


def names(steps):
    return [name for name, _ in steps]


class Plan(unittest.TestCase):
    def test_a_change_with_no_code_runs_only_the_always_steps(self):
        steps, notes = plan(["AGENTS.md"])
        self.assertEqual(names(steps), ["Machine paths", "Agent context", "Comments", "Patterns"])
        self.assertEqual(dict(steps)["Comments"], ["py", "tools/check-comments.py", "--against", "BASE"])
        self.assertEqual(notes, [])

    def test_changed_crates_are_checked_once_each_and_never_their_dependents(self):
        steps, _ = plan(["crates/b/src/lib.rs", "crates/a/src/x.rs", "crates/a/src/y.rs"])
        boat = dict(steps)["Rust check"]
        self.assertEqual(boat[2:], ["check", "--all-targets", "-p", "a", "-p", "b"])
        self.assertEqual(names(steps)[-1], "Rust check")
        self.assertIn("Suppression reasons", names(steps))

    def test_a_manifest_change_adds_the_dependency_checks_and_a_non_rust_one_skips_the_rust_scans(self):
        steps, _ = plan(["crates/a/Cargo.toml"])
        self.assertEqual(
            names(steps)[4:],
            ["Build dependencies", "Content dependencies", "Feature unification", "Nextest config", "Rust check"],
        )
        for name in ("Cargo.toml", ".config/nextest.toml"):
            self.assertIn("Nextest config", names(plan([name])[0]), name)
        self.assertNotIn("Nextest config", names(plan(["crates/a/src/lib.rs"])[0]))

    def test_the_ui_runs_for_ui_files_its_manifests_and_tools_tests(self):
        for name in ("ui/src/a.ts", "package.json", "tools/test-ui.mjs", "tools/e2e/verdicts.test.mjs"):
            self.assertEqual(names(plan([name])[0])[-1], "UI typecheck", name)

    def test_a_tools_change_runs_the_python_tool_tests_and_a_workflow_change_the_workflow_check(self):
        self.assertEqual(names(plan(["tools/release.py"])[0])[4:], ["Python tool tests"])
        self.assertEqual(names(plan([".github/workflows/comments.yml"])[0])[4:], ["Workflows"])

    def test_a_crate_without_a_manifest_and_files_outside_the_workspace_are_noted_not_checked(self):
        steps, notes = plan(["crates/gone/src/a.rs", "vendor/eframe/src/a.rs"])
        self.assertNotIn("Rust check", names(steps))
        self.assertEqual(len(notes), 2)

    def test_a_workspace_wide_file_is_noted_and_checks_no_crate(self):
        steps, notes = plan(["Cargo.lock"])
        self.assertNotIn("Rust check", names(steps))
        self.assertEqual(len(notes), 1)

    def test_boat_failing_itself_is_called_out_apart_from_a_compile_error(self):
        self.assertIn("run it again", cc.describe(cc.BOAT_FAILED))
        self.assertEqual(cc.describe(101), "FAIL (exit 101)")


class Report(unittest.TestCase):
    def posted(self, checked, now, base=cc.DEFAULT_BASE):
        calls = []
        fake = types.SimpleNamespace(post=lambda state, description, sha: calls.append((state, description, sha)) or (True, "posted"))
        with mock.patch.object(cc, "clean_head", return_value=now), mock.patch.dict(sys.modules, {"commit_status": fake}):
            with contextlib.redirect_stdout(io.StringIO()):
                cc.report([("Comments", 0), ("Rust check", 101)], checked, base)
        return calls

    def test_the_status_goes_on_the_clean_commit_the_run_began_on(self):
        self.assertEqual(self.posted("A", "A"), [("failure", "FAIL: Rust check; pass: Comments", "A")])

    def test_no_status_when_the_tree_was_dirty_head_moved_or_the_base_was_narrower(self):
        self.assertEqual(self.posted(None, "A"), [])
        self.assertEqual(self.posted("A", "B"), [])
        self.assertEqual(self.posted("A", None), [])
        self.assertEqual(self.posted("A", "A", base="HEAD"), [])


if __name__ == "__main__":
    unittest.main()
