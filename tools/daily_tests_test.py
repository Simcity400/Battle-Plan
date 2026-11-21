"""Tests for tools/daily_tests.py: python tools/daily_tests_test.py"""

import contextlib
import io
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import daily_tests as dt


class FakeGitHub:
    """Records what would be published; never calls GitHub."""

    def __init__(self):
        self.statuses, self.tracked = [], []

    def status(self, commit, context, state, description):
        self.statuses.append((commit, context, state, description))

    def track(self, failing, title, body):
        self.tracked.append((failing, title, body))


class GameDir(unittest.TestCase):
    def test_a_relative_game_dir_means_the_folder_it_names_where_it_is_given(self):
        previous = os.getcwd()
        with tempfile.TemporaryDirectory() as here:
            os.chdir(here)
            try:
                given = os.path.join(os.getcwd(), "game")
                with mock.patch.dict(os.environ, {"BATTLE_PLAN_GAME_DIR": ""}):
                    for command in ("install", "run"):
                        self.assertEqual(dt.parse([command, "--game-dir", "game"]).game_dir, given)
                with mock.patch.dict(os.environ, {"BATTLE_PLAN_GAME_DIR": "game"}):
                    self.assertEqual(dt.parse(["run"]).game_dir, given)
            finally:
                os.chdir(previous)

    def test_an_empty_variable_names_no_game_dir(self):
        with mock.patch.dict(os.environ, {"BATTLE_PLAN_GAME_DIR": ""}), \
                contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            dt.parse(["run"])


class Continuation(unittest.TestCase):
    def setUp(self):
        root = Path(self.enterContext(tempfile.TemporaryDirectory()))
        self.worktree, self.reports = root / "worktree", root / "reports"
        self.worktree.mkdir()
        self.reports.mkdir()
        self.marker = self.reports / "TESTS_RUNNING"
        self.commit = "a" * 40
        self.enterContext(mock.patch.multiple(
            dt, WORKTREE=self.worktree, REPORTS=self.reports, MARKER=self.marker,
        ))
        self.lock = self.enterContext(mock.patch.object(dt, "lock_runner", return_value=41))
        self.free = self.enterContext(mock.patch.object(dt, "checkout_free", return_value=True))
        self.hold = self.enterContext(mock.patch.object(dt, "hold_checkout", return_value=43))
        self.close = self.enterContext(mock.patch.object(dt.os, "close"))
        self.git = self.enterContext(mock.patch.object(dt, "git", side_effect=self.git_command))
        self.clock = self.enterContext(mock.patch.object(dt.time, "monotonic", return_value=100.0))
        self.step = self.enterContext(mock.patch.object(dt, "run_step", side_effect=self.complete))
        self.record_history = dt.record_history
        self.mark = self.enterContext(mock.patch.object(dt, "write_marker", wraps=dt.write_marker))
        self.history = self.enterContext(mock.patch.object(dt, "record_history", wraps=self.record_history))
        self.write_report = dt.write_report
        self.report = self.enterContext(mock.patch.object(dt, "write_report", wraps=self.write_report))
        self.enterContext(mock.patch.object(dt, "target_dir", return_value=self.worktree / "target"))
        self.github = FakeGitHub()
        self.enterContext(mock.patch.object(dt, "publisher", return_value=self.github))
        self.args = dt.parse(["run", "--game-dir", str(self.worktree / "game")])

    def git_command(self, *args):
        return "synthetic commit" if args[0] == "log" else self.commit

    def complete(self, command, log, environment, seconds):
        self.assertTrue(self.marker.is_file())
        self.close.assert_not_called()
        child = dt.parse(command[2:])
        folder = Path(child.continue_run[3])
        (folder / "result.json").write_text(json.dumps({
            "result": "passed", "failed": None, "lines": ["synthetic report"],
        }), encoding="utf-8")
        log.write_text("synthetic output", encoding="utf-8")
        return 0

    def child_args(self):
        folder = self.reports / "child"
        folder.mkdir()
        self.marker.write_text("parent holds the lock", encoding="utf-8")
        return dt.parse([
            "run", "--game-dir", self.args.game_dir, "--continue-run", self.commit,
            "2026-11-03T03:00:00", "200.0", str(folder),
        ])

    def test_the_parent_runs_the_checked_out_script_once_and_keeps_exclusion_until_reporting(self):
        def record(*args):
            self.assertTrue(self.marker.is_file())
            self.close.assert_not_called()
            return self.record_history(*args)

        self.history.side_effect = record
        self.assertEqual(dt.run(self.args), 0)
        self.step.assert_called_once()
        command, log, environment, seconds = self.step.call_args.args
        self.assertEqual(command[:6], [
            sys.executable, str(self.worktree / "tools" / "daily_tests.py"), "run",
            "--game-dir", self.args.game_dir, "--continue-run",
        ])
        self.assertEqual(command[6], self.commit)
        self.assertIsNone(environment)
        self.assertEqual(log.name, "runner.log")
        self.assertEqual(self.git.call_args_list, [
            mock.call("fetch", "--quiet", "origin", "main"),
            mock.call("checkout", "--quiet", "--detach", "--force", "origin/main"),
            mock.call("rev-parse", "HEAD"),
        ])
        self.lock.assert_called_once_with()
        self.close.assert_called_once_with(41)
        self.assertEqual(self.mark.call_count, 2)
        self.assertTrue(self.mark.call_args.args[0].endswith(f"{self.step.call_args.args[1].parent.name}\n"))
        self.assertFalse(self.marker.exists())
        self.history.assert_called_once()
        self.assertEqual([call.args[0] for call in self.report.call_args_list], ["RUNNING", "PASSED"])
        self.assertEqual(self.report.call_args, mock.call("PASSED", ["synthetic report"]))
        self.assertEqual(len((self.reports / "history.tsv").read_text().splitlines()), 1)

    def test_an_interrupted_marker_rewrite_keeps_the_run_it_named(self):
        dt.write_marker("4242 2026-11-20 03:00 2026-11-20-0300-aaaaaaaaaa\n")

        def stopped(path, *args, **kwargs):
            path.write_bytes(b"")
            raise OSError("stopped mid-write")

        with mock.patch.object(Path, "write_text", stopped), self.assertRaises(OSError):
            dt.write_marker("4242 2026-11-21 03:00\n")
        self.assertEqual(self.marker.read_text(encoding="utf-8"), "4242 2026-11-20 03:00 2026-11-20-0300-aaaaaaaaaa\n")

    def test_an_interrupted_open_groups_write_still_closes_the_groups_listed_before(self):
        folder = self.reports / "run"
        folder.mkdir()
        dt.write_open_groups(folder, ["checks", "rust"])

        def stopped(path, *args, **kwargs):
            path.write_bytes(b"")
            raise OSError("stopped mid-write")

        with mock.patch.object(Path, "write_text", stopped), self.assertRaises(OSError):
            dt.write_open_groups(folder, ["rust"])
        publishing = mock.Mock()
        dt.close_open_groups(folder, self.commit, publishing, "unfinished")
        self.assertEqual(publishing.status.call_args_list, [
            mock.call(self.commit, f"{dt.STATUS}/checks", "error", "unfinished"),
            mock.call(self.commit, f"{dt.STATUS}/rust", "error", "unfinished"),
        ])

    def test_running_is_written_before_fetch_and_replaced_by_the_final_report(self):
        latest = self.reports / "latest.md"
        latest.write_text("yesterday's result", encoding="utf-8")

        def git_command(*args):
            if args[0] == "fetch":
                self.assertTrue(self.marker.is_file())
                self.assertTrue(latest.read_text().startswith("# Daily tests: RUNNING\n"))
                self.report.assert_called_once()
                self.history.assert_not_called()
                self.step.assert_not_called()
            return self.git_command(*args)

        self.git.side_effect = git_command
        self.assertEqual(dt.run(self.args), 0)
        self.assertEqual([call.args[0] for call in self.report.call_args_list], ["RUNNING", "PASSED"])
        self.assertEqual(latest.read_text(), "# Daily tests: PASSED\n\nsynthetic report\n")
        folder = self.step.call_args.args[1].parent
        self.assertEqual((folder / "report.md").read_text(), latest.read_text())

    def test_the_guarded_child_verifies_head_without_locking_fetching_or_reexecuting(self):
        args = self.child_args()
        with mock.patch.object(dt, "steps", return_value=[]):
            self.assertEqual(dt.run(args), 0)
        self.assertEqual(self.git.call_args_list, [
            mock.call("rev-parse", "HEAD"), mock.call("log", "-1", "--format=%s"),
        ])
        self.lock.assert_not_called()
        self.close.assert_not_called()
        self.step.assert_not_called()
        self.history.assert_not_called()
        self.report.assert_not_called()
        self.assertEqual(self.marker.read_text(), "parent holds the lock")
        completed = json.loads((Path(args.continue_run[3]) / "result.json").read_text())
        self.assertEqual(completed["result"], "passed")

    def test_recorded_replies_reach_the_checked_out_runner_the_task_and_every_step_alone(self):
        replies = str(self.worktree / "replies.jsonl.gz")
        with mock.patch.dict(os.environ, {"BATTLE_PLAN_RECORDED_REPLIES": ""}):
            args = dt.parse(["run", "--game-dir", self.args.game_dir, "--recorded-replies", replies])
            self.assertIsNone(dt.parse(["install", "--game-dir", self.args.game_dir]).recorded_replies)
        self.assertEqual(dt.fixtures(args), ["--recorded-replies", replies])
        self.assertEqual(dt.run(args), 0)
        self.assertEqual(dt.parse(self.step.call_args.args[0][2:]).recorded_replies, replies)

        environments = []
        self.step.side_effect = lambda command, log, environment, seconds: environments.append(environment) or 0
        for name, given in (("named", replies), ("unnamed", None)):
            folder = self.reports / name
            folder.mkdir()
            child = dt.parse([
                "run", "--game-dir", self.args.game_dir, *(["--recorded-replies", given] if given else []),
                "--continue-run", self.commit, "2026-11-03T03:00:00", "200.0", str(folder),
            ])
            with mock.patch.object(dt, "steps", return_value=[("UI", "ui", ["npm"], None)]), \
                    mock.patch.dict(os.environ, {"BATTLE_PLAN_RECORDED_REPLIES": "inherited"}):
                dt.run(child)
        self.assertEqual(environments[0]["BATTLE_PLAN_RECORDED_REPLIES"], replies)
        self.assertNotIn("BATTLE_PLAN_RECORDED_REPLIES", environments[1])

    def test_the_guarded_child_refuses_a_different_head(self):
        args = self.child_args()
        self.git.side_effect = lambda *args: "b" * 40
        with self.assertRaisesRegex(RuntimeError, "HEAD is " + "b" * 40):
            dt.run(args)
        self.git.assert_called_once_with("rev-parse", "HEAD")
        self.lock.assert_not_called()
        self.step.assert_not_called()
        self.history.assert_not_called()

    def test_child_errors_timeouts_and_missing_results_are_reported_once_per_run(self):
        cases = [(1, "SyntaxError: invalid syntax"), (3, "ModuleNotFoundError: synthetic"),
                 (None, "output before the deadline"), (0, "exited without a result")]
        for count, (code, output) in enumerate(cases, 1):
            with self.subTest(code=code):
                def fail(command, log, environment, seconds):
                    self.assertTrue(self.marker.is_file())
                    log.write_text(output, encoding="utf-8")
                    return code

                self.step.side_effect = fail
                self.assertEqual(dt.run(self.args), 2)
                self.assertEqual(self.history.call_count, count)
                self.assertEqual([call.args[0] for call in self.report.call_args_list],
                                 ["RUNNING", "RUNNER ERROR"] * count)
                history = (self.reports / "history.tsv").read_text().splitlines()
                self.assertEqual(len(history), count)
                self.assertEqual(history[-1].split("\t")[1:], [self.commit[:10], "error", ""])
                report = (self.reports / "latest.md").read_text()
                self.assertTrue(report.startswith("# Daily tests: RUNNER ERROR\n"))
                self.assertIn(output, report)
                if code is None:
                    self.assertIn("stopped: out of time", report)
                elif code:
                    self.assertIn(f"failed (exit {code})", report)
                self.assertFalse(self.marker.exists())

    def test_a_completed_child_with_failing_tests_records_a_failed_run(self):
        def completed(command, log, environment, seconds):
            self.complete(command, log, environment, seconds)
            (log.parent / "result.json").write_text(json.dumps({
                "result": "failed", "failed": 2, "lines": ["two synthetic failures"],
            }), encoding="utf-8")
            return 0

        self.step.side_effect = completed
        self.assertEqual(dt.run(self.args), 1)
        self.history.assert_called_once()
        self.assertEqual(self.history.call_args.args[1:], (self.commit, "failed", 2))
        self.assertEqual([call.args[0] for call in self.report.call_args_list], ["RUNNING", "FAILED"])
        self.assertEqual(self.report.call_args, mock.call("FAILED", ["two synthetic failures"]))

    def test_the_child_deadline_reserves_cleanup_and_reporting_before_the_parent_deadline(self):
        self.clock.side_effect = [100.0, 137.0]
        self.assertEqual(dt.run(self.args), 0)
        command, _, _, seconds = self.step.call_args.args
        child = dt.parse(command[2:])
        deadline = 100.0 + dt.DEADLINE.total_seconds()
        self.assertGreater(dt.REPORT_MARGIN, 2 * dt.CLEANUP)
        self.assertEqual(float(child.continue_run[2]), deadline - dt.REPORT_MARGIN)
        self.assertEqual(seconds, dt.DEADLINE.total_seconds() - 37.0)

    def test_a_child_cannot_start_without_its_cleanup_and_reporting_margin(self):
        deadline = 100.0 + dt.DEADLINE.total_seconds()
        self.clock.side_effect = [100.0, deadline - dt.REPORT_MARGIN]
        self.assertEqual(dt.run(self.args), 2)
        self.step.assert_not_called()
        self.history.assert_called_once()
        self.assertIn("out of time before starting", (self.reports / "latest.md").read_text())

    def test_a_history_write_failure_still_writes_both_reports_and_prunes(self):
        self.history.side_effect = PermissionError("synthetic history is read-only")
        with mock.patch.object(dt, "prune") as prune:
            self.assertEqual(dt.run(self.args), 2)
        self.history.assert_called_once()
        self.assertEqual(self.history.call_args.args[1:], (self.commit, "passed", None))
        self.assertEqual([call.args[0] for call in self.report.call_args_list],
                         ["RUNNING", "RUNNER ERROR"])
        report = (self.reports / "latest.md").read_text()
        self.assertTrue(report.startswith("# Daily tests: RUNNER ERROR\n"))
        self.assertIn("history.tsv", report)
        self.assertIn("synthetic history is read-only", report)
        self.assertIn("synthetic report", report)
        folder = self.step.call_args.args[1].parent
        self.assertEqual((folder / "report.md").read_text(), report)
        prune.assert_called_once_with(self.history.call_args.args[0])
        self.assertFalse(self.marker.exists())
        self.close.assert_called_once_with(41)

    def test_a_latest_report_failure_still_writes_the_run_report_and_prunes(self):
        def write_report(heading, lines):
            if heading != "RUNNING":
                raise PermissionError("latest is locked")
            return self.write_report(heading, lines)

        self.report.side_effect = write_report
        with mock.patch.object(dt, "prune") as prune:
            self.assertEqual(dt.run(self.args), 2)
        self.history.assert_called_once()
        folder = self.step.call_args.args[1].parent
        self.assertEqual((folder / "report.md").read_text(),
                         "# Daily tests: PASSED\n\nsynthetic report\n")
        prune.assert_called_once_with(self.history.call_args.args[0])
        self.assertFalse(self.marker.exists())

    def test_a_run_report_failure_keeps_the_latest_report_and_still_prunes(self):
        write_text = Path.write_text

        def write(path, *args, **kwargs):
            if path.name == "report.md":
                raise PermissionError("run report is read-only")
            return write_text(path, *args, **kwargs)

        with mock.patch.object(Path, "write_text", write), mock.patch.object(dt, "prune") as prune:
            self.assertEqual(dt.run(self.args), 2)
        self.history.assert_called_once()
        self.assertEqual((self.reports / "latest.md").read_text(),
                         "# Daily tests: PASSED\n\nsynthetic report\n")
        prune.assert_called_once_with(self.history.call_args.args[0])
        self.assertFalse(self.marker.exists())

    def test_the_run_publishes_pending_then_its_result_and_tracks_failure(self):
        self.assertEqual(dt.run(self.args), 0)
        self.assertEqual([status[:3] for status in self.github.statuses], [
            (self.commit, dt.STATUS, "pending"), (self.commit, dt.STATUS, "success"),
        ])
        self.assertEqual(len(self.github.tracked), 1)
        failing, title, body = self.github.tracked[0]
        self.assertEqual((failing, title), (False, dt.ISSUE_TITLE))
        self.assertTrue(body.startswith("# Daily tests: PASSED\n"))

        self.github.statuses.clear()
        self.step.side_effect = lambda command, log, environment, seconds: 1
        self.assertEqual(dt.run(self.args), 2)
        self.assertEqual(self.github.statuses[-1][1:3], (dt.STATUS, "error"))
        failing, _, body = self.github.tracked[-1]
        self.assertTrue(failing)
        self.assertIn("checked-out runner failed (exit 1)", body)
        self.assertNotIn(str(self.worktree), body)

    def test_a_run_that_recorded_its_result_is_published_not_called_unfinished_by_the_next(self):
        def power_loss(*args):
            raise KeyboardInterrupt
        track, self.github.track = self.github.track, power_loss
        with self.assertRaises(KeyboardInterrupt):
            dt.run(self.args)
        self.assertTrue(self.marker.exists())
        self.github.track, self.github.statuses = track, []
        self.close.reset_mock()
        self.assertEqual(dt.run(self.args), 0)
        history = [line.split("\t")[2] for line in (self.reports / "history.tsv").read_text().splitlines()]
        self.assertEqual(history, ["passed", "passed"])
        self.assertEqual(self.github.statuses[0][:3], (self.commit, dt.STATUS, "success"))
        self.assertEqual(self.github.tracked[0][0], False)
        self.assertIn("saved its result but stopped before publishing it", (self.reports / "latest.md").read_text())

    def test_a_saved_outcome_missing_from_history_is_recorded_once_and_an_error_closes_its_groups(self):
        folder = self.reports / f"2026-11-20-0300-{'e' * 10}"
        folder.mkdir()
        (folder / dt.RUN_MARK).write_text(f"2026-11-20 03:00\t{'e' * 40}\n", encoding="utf-8")
        dt.write_open_groups(folder, ["acceptance"])
        dt.save_outcome(folder, {"result": "error", "failed": None, "recorded": False, "state": "error",
                                 "description": "error 2026-11-20 03:00", "failing": True, "body": "body"})
        self.marker.write_text(f"4242 2026-11-20 03:00 {folder.name}\n", encoding="utf-8")
        self.assertEqual(dt.run(self.args), 0)
        history = (self.reports / "history.tsv").read_text().splitlines()
        self.assertEqual(history[0], f"2026-11-20 03:00\t{'e' * 10}\terror\t")
        self.assertEqual(len(history), 2)
        self.assertEqual([status[1:3] for status in self.github.statuses[:2]], [
            (f"{dt.STATUS}/acceptance", "error"), (dt.STATUS, "error"),
        ])

    def test_a_run_interrupted_while_its_tests_run_is_recovered_by_the_next(self):
        def interrupted(command, log, environment, seconds):
            raise KeyboardInterrupt
        self.step.side_effect = interrupted
        with self.assertRaises(KeyboardInterrupt):
            dt.run(self.args)
        self.assertTrue(self.marker.exists())
        self.assertIsNotNone(dt.unfinished_run())
        self.step.side_effect, self.github.statuses = self.complete, []
        self.close.reset_mock()
        self.assertEqual(dt.run(self.args), 0)
        self.assertEqual(self.github.statuses[0][:3], (self.commit, dt.STATUS, "error"))
        self.assertEqual((self.reports / "history.tsv").read_text().splitlines()[0].split("\t")[1:3],
                         [self.commit[:10], "unfinished"])

    def test_a_run_whose_parent_alone_stopped_is_published_from_its_runners_result(self):
        folder = self.reports / f"2026-11-20-0300-{'e' * 10}"
        folder.mkdir()
        (folder / dt.RUN_MARK).write_text(f"2026-11-20 03:00\t{'e' * 40}\n", encoding="utf-8")
        (folder / "result.json").write_text(json.dumps({
            "result": "passed", "failed": 0, "lines": ["local"], "public": ["all passed"],
        }), encoding="utf-8")
        self.marker.write_text(f"4242 2026-11-20 03:00 {folder.name}\n", encoding="utf-8")
        self.assertEqual(dt.run(self.args), 0)
        self.assertEqual((self.reports / "history.tsv").read_text().splitlines()[0],
                         f"2026-11-20 03:00\t{'e' * 10}\tpassed\t0")
        self.assertEqual(self.github.statuses[0][:3], ("e" * 40, dt.STATUS, "success"))
        self.assertEqual(self.github.tracked[0][0], False)
        self.assertIn("all passed", self.github.tracked[0][2])

    def test_a_history_that_cannot_be_appended_still_closes_the_earlier_runs_statuses(self):
        folder = self.reports / f"2026-11-20-0300-{'e' * 10}"
        folder.mkdir()
        (folder / dt.RUN_MARK).write_text(f"2026-11-20 03:00\t{'e' * 40}\n", encoding="utf-8")
        dt.write_open_groups(folder, ["rust"])
        self.marker.write_text(f"4242 2026-11-20 03:00 {folder.name}\n", encoding="utf-8")

        def locked(started, commit, result, failed):
            if result == "unfinished":
                raise PermissionError("history.tsv is locked")
            return self.record_history(started, commit, result, failed)
        self.history.side_effect = locked
        self.assertEqual(dt.run(self.args), 0)
        self.assertEqual([status[:3] for status in self.github.statuses[:2]], [
            ("e" * 40, dt.STATUS, "error"), ("e" * 40, f"{dt.STATUS}/rust", "error"),
        ])
        self.assertIn("history.tsv is locked", (self.reports / "latest.md").read_text())

    def test_the_earlier_run_is_recovered_before_this_run_can_fail(self):
        self.marker.write_text("4242 2026-11-20 03:00\n", encoding="utf-8")

        def write_report(heading, lines):
            raise PermissionError("latest is locked")
        self.report.side_effect = write_report
        self.assertEqual(dt.run(self.args), 2)
        history = (self.reports / "history.tsv").read_text().splitlines()
        self.assertEqual(history[0], "2026-11-20 03:00\t\tunfinished\t")

    def test_a_child_that_dies_leaves_no_group_pending(self):
        def die(command, log, environment, seconds):
            dt.write_open_groups(Path(dt.parse(command[2:]).continue_run[3]), ["rust", "acceptance"])
            log.write_text("killed", encoding="utf-8")
            return 1
        self.step.side_effect = die
        self.assertEqual(dt.run(self.args), 2)
        self.assertEqual([status[1:3] for status in self.github.statuses], [
            (dt.STATUS, "pending"), (f"{dt.STATUS}/rust", "error"), (f"{dt.STATUS}/acceptance", "error"),
            (dt.STATUS, "error"),
        ])
        self.assertIn("The runner failed: checked-out runner failed (exit 1).", self.github.tracked[-1][2])

    def test_a_child_without_a_result_publishes_no_local_path(self):
        self.step.side_effect = lambda command, log, environment, seconds: 0
        self.assertEqual(dt.run(self.args), 2)
        body = self.github.tracked[-1][2]
        self.assertIn("left no readable result", body)
        self.assertNotIn(str(self.reports), body)

    def test_publishing_failures_are_reported_and_never_fail_the_run(self):
        self.github.status = lambda *args: "status daily-tests: HTTP 401"
        self.github.track = lambda *args: "tracking issue: gh auth login"
        self.assertEqual(dt.run(self.args), 0)
        report = (self.reports / "latest.md").read_text()
        self.assertTrue(report.startswith("# Daily tests: PASSED\n"))
        self.assertIn("Not published: status daily-tests: HTTP 401", report)
        self.assertIn("Not published: tracking issue: gh auth login", report)

    def test_a_run_that_never_reported_is_recorded_unfinished_by_the_next(self):
        folder = self.reports / f"2026-11-20-0300-{'e' * 10}"
        folder.mkdir()
        (folder / dt.RUN_MARK).write_text(f"2026-11-20 03:00\t{'e' * 40}\n", encoding="utf-8")
        dt.write_open_groups(folder, ["rust", "acceptance"])
        self.marker.write_text(f"4242 2026-11-20 03:00 {folder.name}\n", encoding="utf-8")
        self.assertEqual(dt.run(self.args), 0)
        history = (self.reports / "history.tsv").read_text().splitlines()
        self.assertEqual(history[0], f"2026-11-20 03:00\t{'e' * 10}\tunfinished\t")
        self.assertEqual(history[1].split("\t")[2], "passed")
        self.assertEqual([status[:3] for status in self.github.statuses[:3]], [
            ("e" * 40, dt.STATUS, "error"), ("e" * 40, f"{dt.STATUS}/rust", "error"),
            ("e" * 40, f"{dt.STATUS}/acceptance", "error"),
        ])
        self.assertTrue(self.github.tracked[0][0])
        self.assertIn("never reported", self.github.tracked[0][2])
        self.assertIn("never reported", (self.reports / "latest.md").read_text())
        self.assertFalse(self.marker.exists())

        self.marker.write_text("4243 2026-11-21 03:00\n", encoding="utf-8")
        self.github.statuses.clear()
        self.close.reset_mock()
        self.assertEqual(dt.run(self.args), 0)
        self.assertEqual((self.reports / "history.tsv").read_text().splitlines()[2], "2026-11-21 03:00\t\tunfinished\t")
        self.assertEqual([status[2] for status in self.github.statuses], ["pending", "success"])

    def test_each_group_is_published_and_reported_as_it_ends(self):
        args = self.child_args()
        seen = []

        def step(outcome):
            def run(build, log):
                latest = self.reports / "latest.md"
                seen.append(([status[1:3] for status in self.github.statuses],
                             latest.read_text() if latest.exists() else ""))
                return outcome
            return run

        plan = [
            ("Machine paths", "machine-paths", step((0, "passed")), None),
            ("UI install", "ui-install", step((1, "failed")), None),
            ("UI tests", "ui-tests", step((0, "passed")), "ui-install"),
            ("Disk space", "disk", step((0, "passed")), None),
            ("Rust tests", "rust", step((1, "failed")), "disk"),
        ]
        with mock.patch.object(dt, "steps", return_value=plan):
            self.assertEqual(dt.run(args), 0)
        self.assertEqual(len(seen), 4)
        self.assertEqual(seen[0][0], [(f"{dt.STATUS}/{group}", "pending") for group in ("repository", "ui", "rust")])
        self.assertEqual(seen[1][0][-1], (f"{dt.STATUS}/repository", "success"))
        self.assertIn("| Machine paths | passed |", seen[1][1])
        self.assertIn("Still running: ui, rust.", seen[1][1])
        descriptions = {status[1]: (status[2], status[3]) for status in self.github.statuses}
        self.assertEqual(descriptions[f"{dt.STATUS}/ui"], ("failure", "failed: UI install, UI tests"))
        state, description = descriptions[f"{dt.STATUS}/rust"]
        self.assertEqual(state, "failure")
        self.assertTrue(description.startswith("failed: Rust tests; passed not recorded"), description)
        completed = json.loads((Path(args.continue_run[3]) / "result.json").read_text())
        self.assertEqual(completed["result"], "failed")
        self.assertIn("Rust tests: passed not recorded, failed not recorded, skipped not recorded, missing not recorded.",
                      completed["lines"])
        self.assertFalse(any(args.continue_run[3] in line for line in completed["public"]))


KEPT, OLD, SCRIPT, RUN = "a" * 16, "b" * 16, "c" * 16, "d" * 16


def messages(profile, finished=True):
    """Cargo's messages for a synthetic build: a library, a build script and its run, and a binary
    whose files carry no hash."""
    build = profile / "build"
    lines = [
        {"reason": "compiler-artifact", "target": {"kind": ["lib"], "name": "alpha"},
         "filenames": [str(profile / "deps" / f"libalpha-{KEPT}.rlib")], "executable": None},
        {"reason": "compiler-artifact",
         "target": {"kind": ["custom-build"], "name": "build-script-build"},
         "filenames": [str(build / f"alpha-{SCRIPT}" / f"build_script_build-{SCRIPT}.exe")]},
        {"reason": "build-script-executed", "out_dir": str(build / f"alpha-{RUN}" / "out")},
        {"reason": "compiler-artifact", "target": {"kind": ["bin"], "name": "map-seek"},
         "filenames": [str(profile / "map-seek.exe"), str(profile / "map_seek.pdb")],
         "executable": str(profile / "map-seek.exe")},
    ]
    if finished is not None:
        lines.append({"reason": "build-finished", "success": finished})
    stream = io.BytesIO()
    stream.write(b"Compiling is not JSON\n")
    for line in lines:
        stream.write(json.dumps(line).encode() + b"\n")
    return stream


def touch(path, written=None):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(b"x")
    if written is not None:
        os.utime(path, (written, written))


class Sweep(unittest.TestCase):
    def profile(self, root):
        profile = Path(root) / "debug"
        for unit in (f"alpha-{KEPT}", f"alpha-{SCRIPT}", f"alpha-{RUN}", f"alpha-{OLD}"):
            touch(profile / ".fingerprint" / unit / "lib-alpha")
        touch(profile / ".fingerprint" / ("map-seek-" + "e" * 16) / "bin-map-seek", 1_000)
        touch(profile / ".fingerprint" / ("map-seek-" + "f" * 16) / "bin-map-seek", 2_000)
        for name in (f"libalpha-{KEPT}.rlib", f"alpha-{KEPT}.d", f"libalpha-{OLD}.rlib",
                     f"alpha-{KEPT}.alpha.0abc-cgu.0.rcgu.o", "map_seek.exe", "map_seek.pdb",
                     "map_seek.d", "map_seek.0j6cv75.rcgu.o", "gone.exe"):
            touch(profile / "deps" / name)
        touch(profile / "build" / f"alpha-{SCRIPT}" / f"build_script_build-{SCRIPT}.exe")
        touch(profile / "build" / f"alpha-{RUN}" / "out" / "generated.rs")
        touch(profile / "build" / f"alpha-{OLD}" / "out" / "generated.rs")
        for name in ("map-seek.exe", "map_seek.pdb", "viewer.exe", ".cargo-lock"):
            touch(profile / name)
        touch(profile / "incremental" / "alpha-1" / "s-1" / "work")
        return profile

    def test_a_finished_build_keeps_its_units_and_removes_everything_else(self):
        with tempfile.TemporaryDirectory() as root:
            profile = self.profile(root)
            units, finished = dt.cargo_units(messages(profile))
            self.assertTrue(finished)
            removed = {os.path.relpath(path, profile) for path in dt.stale(profile, units)}
            self.assertEqual(removed, {
                os.path.join(".fingerprint", f"alpha-{OLD}"),
                os.path.join(".fingerprint", "map-seek-" + "e" * 16),
                os.path.join("deps", f"libalpha-{OLD}.rlib"),
                os.path.join("deps", f"alpha-{KEPT}.alpha.0abc-cgu.0.rcgu.o"),
                os.path.join("deps", "map_seek.0j6cv75.rcgu.o"),
                os.path.join("deps", "gone.exe"),
                os.path.join("build", f"alpha-{OLD}"),
                "viewer.exe",
                "incremental",
            })

    def test_the_sweep_step_removes_the_stale_entries_and_reports_them(self):
        with tempfile.TemporaryDirectory() as root:
            profile = self.profile(root)
            build = dt.Build(Path(root))
            build.units, _ = dt.cargo_units(messages(profile))
            log = Path(root) / "rust-sweep.log"
            self.assertEqual(dt.sweep_target(build, log), (0, "passed: removed 9 stale entries"))
            self.assertFalse((profile / "incremental").exists())
            self.assertFalse((profile / "deps" / f"libalpha-{OLD}.rlib").exists())
            self.assertTrue((profile / "deps" / f"libalpha-{KEPT}.rlib").exists())
            self.assertTrue((profile / "deps" / "map_seek.exe").exists())
            self.assertEqual(len(log.read_text(encoding="utf-8").splitlines()), 9)

    def test_a_unit_cargo_named_without_its_fingerprint_refuses_the_sweep(self):
        with tempfile.TemporaryDirectory() as root:
            profile = self.profile(root)
            units, _ = dt.cargo_units(messages(profile))
            units.hashes.add("9" * 16)
            with self.assertRaises(dt.Refused):
                dt.stale(profile, units)
            build = dt.Build(Path(root))
            build.units = units
            code, outcome = dt.sweep_target(build, Path(root) / "rust-sweep.log")
            self.assertEqual(code, 1)
            self.assertTrue(outcome.startswith("failed: refused"))
            self.assertTrue((profile / "incremental").exists())

    def test_a_fingerprint_folder_shared_by_unhashed_targets_is_removed_once_or_kept(self):
        with tempfile.TemporaryDirectory() as root:
            profile = self.profile(root)
            old, new = ("map-seek-" + "e" * 16), ("map-seek-" + "f" * 16)
            touch(profile / ".fingerprint" / old / "example-fixture", 1_000)
            touch(profile / ".fingerprint" / new / "example-fixture", 2_000)
            units, _ = dt.cargo_units(messages(profile))
            units.unhashed.add("example-fixture")
            removed = [os.path.relpath(path, profile) for path in dt.stale(profile, units)]
            self.assertEqual(removed.count(os.path.join(".fingerprint", old)), 1)
            self.assertNotIn(os.path.join(".fingerprint", new), removed)
            touch(profile / ".fingerprint" / old / "example-fixture", 3_000)
            removed = [os.path.relpath(path, profile) for path in dt.stale(profile, units)]
            self.assertNotIn(os.path.join(".fingerprint", old), removed)
            self.assertNotIn(os.path.join(".fingerprint", new), removed)

    def test_a_build_with_no_units_is_never_swept(self):
        with tempfile.TemporaryDirectory() as root:
            with self.assertRaises(dt.Refused):
                dt.stale(self.profile(root), dt.Units())

    def test_whether_the_build_finished_is_cargos_message(self):
        profile = Path("C:/target/debug")
        self.assertIs(dt.cargo_units(messages(profile, finished=False))[1], False)
        self.assertIsNone(dt.cargo_units(messages(profile, finished=None))[1])
        units, _ = dt.cargo_units(messages(profile))
        self.assertEqual(units.hashes, {KEPT, SCRIPT, RUN})
        self.assertEqual(units.unhashed, {"bin-map-seek"})


class Disk(unittest.TestCase):
    def test_free_space_below_the_floor_fails_by_name_before_compiling(self):
        with tempfile.TemporaryDirectory() as root:
            log = Path(root) / "disk.log"
            usage = mock.Mock(free=dt.DISK_FLOOR - 1)
            with mock.patch.object(dt.shutil, "disk_usage", return_value=usage):
                code, outcome = dt.check_disk(dt.Build(Path(root) / "target"), log)
            self.assertEqual(code, 1)
            self.assertIn("below the 60.0 GB floor", outcome)
            usage = mock.Mock(free=dt.DISK_FLOOR)
            with mock.patch.object(dt.shutil, "disk_usage", return_value=usage):
                self.assertEqual(dt.check_disk(dt.Build(Path(root) / "target"), log)[0], 0)

    def test_a_hard_linked_file_counts_once_and_a_missing_target_is_absent(self):
        with tempfile.TemporaryDirectory() as root:
            (Path(root) / "deps").mkdir()
            (Path(root) / "deps" / "tool.exe").write_bytes(b"x" * 10)
            os.link(Path(root) / "deps" / "tool.exe", Path(root) / "tool.exe")
            (Path(root) / "other").write_bytes(b"y" * 5)
            self.assertEqual(dt.size_of(Path(root)), 15)
            self.assertIsNone(dt.size_of(Path(root) / "missing"))
            self.assertEqual(dt.gigabytes(None), "absent")

    def setUp(self):
        self.tracked, self.ignored = "", True

        def git(*args):
            if args[0] == "ls-files":
                return self.tracked
            if not self.ignored:
                raise RuntimeError("git check-ignore: ")
            return ""
        self.enterContext(mock.patch.object(dt, "git", side_effect=git))

    def reclaiming(self, root, free, target_size):
        """A runner worktree under `root` whose Cargo target holds `target_size` bytes, on a drive
        with `free` bytes free until the target is removed."""
        worktree = Path(root) / "worktree"
        target = worktree / ".cargo-target"
        (target / "debug").mkdir(parents=True)
        (worktree / ".cargo").mkdir()
        (worktree / ".cargo" / "config.toml").write_text('[build]\ntarget-dir = ".cargo-target"\n', encoding="utf-8")
        (target / "debug" / "lib.rlib").write_bytes(b"x" * target_size)
        usage = lambda drive: mock.Mock(free=free + (0 if target.exists() else target_size))
        return worktree, target, usage

    def test_below_the_floor_the_runners_own_target_is_removed_when_that_reaches_it(self):
        with tempfile.TemporaryDirectory() as root:
            worktree, target, usage = self.reclaiming(root, dt.DISK_FLOOR - 5, 10)
            log = Path(root) / "disk.log"
            with mock.patch.object(dt, "WORKTREE", worktree), mock.patch.object(dt.shutil, "disk_usage", side_effect=usage):
                code, outcome = dt.check_disk(dt.Build(target), log)
            self.assertEqual(code, 0, outcome)
            self.assertIn("after removing the runner's", outcome)
            self.assertFalse(target.exists())
            self.assertIn("removed the runner's Cargo target", log.read_text(encoding="utf-8"))

    def test_a_target_whose_removal_would_not_reach_the_floor_is_kept(self):
        with tempfile.TemporaryDirectory() as root:
            worktree, target, usage = self.reclaiming(root, dt.DISK_FLOOR - 50, 10)
            log = Path(root) / "disk.log"
            with mock.patch.object(dt, "WORKTREE", worktree), mock.patch.object(dt.shutil, "disk_usage", side_effect=usage):
                code, outcome = dt.check_disk(dt.Build(target), log)
            self.assertEqual(code, 1)
            self.assertIn("below the 60.0 GB floor", outcome)
            self.assertTrue(target.exists())
            self.assertIn("would not reach the floor", log.read_text(encoding="utf-8"))

    def test_a_target_outside_the_runners_worktree_or_behind_a_link_is_never_removed(self):
        with tempfile.TemporaryDirectory() as root:
            worktree, target, usage = self.reclaiming(root, dt.DISK_FLOOR - 5, 10)
            elsewhere = Path(root) / "elsewhere"
            elsewhere.mkdir()
            with mock.patch.object(dt, "WORKTREE", elsewhere), mock.patch.object(dt.shutil, "disk_usage", side_effect=usage):
                self.assertEqual(dt.check_disk(dt.Build(target), Path(root) / "disk.log")[0], 1)
            self.assertTrue(target.exists())
            linked = worktree / "linked-target"
            subprocess.run(["cmd", "/c", "mklink", "/J", str(linked), str(target)], check=True, capture_output=True)
            try:
                with mock.patch.object(dt, "WORKTREE", worktree):
                    self.assertEqual(dt.not_owned(linked), "it is not a plain folder")
                    self.assertEqual(dt.not_owned(worktree), "it is not the runner worktree's own Cargo target")
                    captures = worktree / "captures"
                    captures.mkdir()
                    self.assertEqual(dt.not_owned(captures), "it is not the runner worktree's own Cargo target")
                    self.assertIsNone(dt.not_owned(target))
            finally:
                os.rmdir(linked)
            self.assertTrue((target / "debug" / "lib.rlib").exists())

    def test_a_target_holding_a_link_to_elsewhere_is_never_removed(self):
        with tempfile.TemporaryDirectory() as root:
            worktree, target, usage = self.reclaiming(root, dt.DISK_FLOOR - 5, 10)
            elsewhere = Path(root) / "elsewhere"
            elsewhere.mkdir()
            (elsewhere / "big.rlib").write_bytes(b"x" * 100)
            linked = target / "release"
            subprocess.run(["cmd", "/c", "mklink", "/J", str(linked), str(elsewhere)], check=True, capture_output=True)
            try:
                with mock.patch.object(dt, "WORKTREE", worktree), mock.patch.object(dt.shutil, "disk_usage", side_effect=usage):
                    self.assertEqual(dt.not_owned(target), "it links elsewhere at release")
                    self.assertEqual(dt.check_disk(dt.Build(target), Path(root) / "disk.log")[0], 1)
                self.assertTrue((elsewhere / "big.rlib").exists())
                self.assertTrue(target.exists())
            finally:
                os.rmdir(linked)

    def test_a_tracked_or_unignored_folder_or_one_a_boat_run_holds_is_never_removed(self):
        with tempfile.TemporaryDirectory() as root:
            worktree, target, usage = self.reclaiming(root, dt.DISK_FLOOR - 5, 10)
            with mock.patch.object(dt, "WORKTREE", worktree), mock.patch.object(dt.shutil, "disk_usage", side_effect=usage):
                self.tracked = "ui/src/app.ts"
                self.assertEqual(dt.check_disk(dt.Build(target), Path(root) / "disk.log")[0], 1)
                self.assertIn("Git tracks files in it", (Path(root) / "disk.log").read_text(encoding="utf-8"))
                self.tracked, self.ignored = "", False
                self.assertEqual(dt.not_owned(target), "Git does not ignore it")
                self.ignored = True
                (target / "boat-test" / "run").mkdir(parents=True)
                held = open(target / "boat-test" / "run" / ".boat-test.lock", "a+b")
                try:
                    held.write(b"0")
                    held.flush()
                    held.seek(0)
                    dt.msvcrt.locking(held.fileno(), dt.msvcrt.LK_NBLCK, 1)
                    self.assertEqual(dt.not_owned(target), f"a build holds {os.path.join('boat-test', 'run', '.boat-test.lock')}")
                    held.seek(0)
                    dt.msvcrt.locking(held.fileno(), dt.msvcrt.LK_UNLCK, 1)
                finally:
                    held.close()
                self.assertIsNone(dt.not_owned(target))
            self.assertTrue(target.exists())


class Acceptance(unittest.TestCase):
    def setUp(self):
        self.root = Path(self.enterContext(tempfile.TemporaryDirectory()))
        self.log = self.root / "e2e.log"
        self.build = dt.Build(self.root / "target", self.root, {"BATTLE_PLAN_GAME_DIR": "G:/game"}, 1e9, "F:/fixture")

    def reporting(self, code, report):
        def step(command, log, environment, seconds):
            self.command = command
            if report is not None:
                (self.root / "e2e").mkdir(exist_ok=True)
                (self.root / "e2e" / "e2e.json").write_text(json.dumps(report), encoding="utf-8")
            return code
        return mock.patch.object(dt, "run_step", side_effect=step)

    def test_the_fixture_reaches_the_checked_out_runner_and_the_task(self):
        with mock.patch.dict(os.environ, {"BATTLE_PLAN_E2E_FIXTURE": ""}):
            args = dt.parse(["run", "--game-dir", "game", "--fixture", "fixture"])
        self.assertEqual(args.fixture, os.path.abspath("fixture"))
        with mock.patch.dict(os.environ, {"BATTLE_PLAN_E2E_FIXTURE": "named"}):
            self.assertEqual(dt.parse(["install", "--game-dir", "game"]).fixture, os.path.abspath("named"))

    def test_no_fixture_fails_the_step_by_name_without_running_it(self):
        self.build.fixture = None
        with self.reporting(0, None) as step:
            code, outcome = dt.acceptance(self.build, self.log)
        step.assert_not_called()
        self.assertEqual(code, 1)
        self.assertIn("no fixture configured", outcome)

    def test_the_suite_runs_on_the_nights_build_into_the_run_folder(self):
        with self.reporting(0, {"passed": True, "failed": 0}):
            self.assertEqual(dt.acceptance(self.build, self.log), (0, "passed: e2e/e2e.md"))
        self.assertEqual(self.command[1:4], ["tools/e2e/suite.py", "run", "--app"])
        for flag, value in (("--app", str(dt.E2E_APP)), ("--fixture", "F:/fixture"), ("--game-dir", "G:/game"),
                            ("--out", str(self.root / "e2e")), ("--state", str(dt.E2E_STATE))):
            self.assertEqual(self.command[self.command.index(flag) + 1], value)

    def test_failing_rows_a_suite_that_could_not_run_and_a_timeout_each_fail(self):
        with self.reporting(1, {"passed": False, "failed": 7}):
            self.assertEqual(dt.acceptance(self.build, self.log), (1, "failed: 7 failing rows, e2e/e2e.md"))
        with self.reporting(2, None):
            self.assertEqual(dt.acceptance(self.build, self.log)[1], "failed (exit 2): the suite could not run")
        with self.reporting(None, None):
            self.assertEqual(dt.acceptance(self.build, self.log), (1, "stopped: out of time"))

    def test_the_suite_runs_after_the_build_and_ui_install_it_needs(self):
        slugs = [slug for _, slug, _, _ in dt.steps()]
        needs = dict((slug, needs) for _, slug, _, needs in dt.steps())
        self.assertEqual(needs["e2e"], ("e2e-build", "ui-install"))
        for need in needs["e2e"]:
            self.assertLess(slugs.index(need), slugs.index("e2e"))

    def test_a_step_needing_several_is_not_run_when_any_failed_and_names_them(self):
        root = Path(self.enterContext(tempfile.TemporaryDirectory()))
        folder = root / "run"
        folder.mkdir()
        ran = []
        plan = [
            ("First", "first", lambda build, log: (0, "passed"), None),
            ("Second", "second", lambda build, log: (1, "failed"), None),
            ("Both", "both", lambda build, log: ran.append("both") or (0, "passed"), ("first", "second")),
        ]
        args = dt.parse(["run", "--game-dir", "game", "--continue-run", "c" * 40, "2026-11-08T03:00:00", "1e12", str(folder)])
        with mock.patch.multiple(dt, WORKTREE=root, REPORTS=root / "reports"), \
                mock.patch.object(dt, "steps", return_value=plan), \
                mock.patch.object(dt, "git", return_value="c" * 40), \
                mock.patch.object(dt, "target_dir", return_value=root / "target"), \
                mock.patch.object(dt, "hold_checkout", return_value=43), \
                mock.patch.object(dt, "publisher", return_value=FakeGitHub()):
            dt.run(args)
        lines = json.loads((folder / "result.json").read_text(encoding="utf-8"))["lines"]
        self.assertEqual(ran, [])
        self.assertIn("| Both | not run: second failed | | |", lines)


LISTING = json.dumps({"rust-suites": {"replay": {"testcases": {
    "reads": {"filter-match": {"status": "matches"}},
    "writes": {"filter-match": {"status": "matches"}},
    "slow": {"filter-match": {"status": "mismatch", "reason": "ignored"}},
}}}})


class RustTests(unittest.TestCase):
    def setUp(self):
        self.root = Path(self.enterContext(tempfile.TemporaryDirectory()))
        self.build = dt.Build(self.root / "cargo-target", self.root, {}, 1e12)
        self.worktree = self.root / "worktree"
        self.enterContext(mock.patch.object(dt, "WORKTREE", self.worktree))
        self.junit = dt.nextest_junit()
        self.assertEqual(self.junit, self.worktree / "target" / "nextest" / dt.NEXTEST_PROFILE / "junit.xml")
        self.junit.parent.mkdir(parents=True)
        self.junit.write_text("<testsuites/>", encoding="utf-8")

    def bounded(self, listed, junit, run_code=0):
        def run(command, stdout, stderr, seconds, environment=None):
            if command == dt.RUST_LIST:
                if listed is None:
                    return 101
                stdout.write(listed.encode())
                return 0
            self.assertEqual(command, dt.RUST_TESTS)
            self.assertFalse(self.junit.exists(), "the last run's report is removed first")
            stdout.write(messages(self.build.target / "debug").getvalue())
            if junit is not None:
                self.junit.write_text(junit, encoding="utf-8")
            return run_code
        return mock.patch.object(dt, "bounded", side_effect=run)

    def test_a_run_counts_its_tests_and_keeps_its_listing_and_report(self):
        junit = ('<testsuites><testsuite name="replay"><testcase name="reads"/>'
                 '<testcase name="writes"><failure/></testcase></testsuite></testsuites>')
        with self.bounded(LISTING, junit, 100):
            code, outcome = dt.rust_tests(self.build, self.root / "rust.log")
        self.assertEqual((code, outcome), (1, "failed (exit 100): 1 passed, 1 failed, 1 skipped, 0 missing"))
        self.assertEqual(self.build.tests.failures, ["replay writes"])
        self.assertIsNotNone(self.build.units)
        self.assertEqual((self.root / "rust-junit.xml").read_text(encoding="utf-8"), junit)
        self.assertEqual((self.root / "rust-list.json").read_text(encoding="utf-8"), LISTING)

    def test_the_report_is_read_from_the_nextest_store_the_config_names(self):
        (self.worktree / ".config").mkdir()
        (self.worktree / ".config" / "nextest.toml").write_text('[store]\ndir = "out/nextest"\n', encoding="utf-8")
        self.assertEqual(dt.nextest_junit(), self.worktree / "out" / "nextest" / dt.NEXTEST_PROFILE / "junit.xml")

    def test_a_clean_exit_with_a_test_unreported_still_fails(self):
        junit = '<testsuites><testsuite name="replay"><testcase name="reads"/></testsuite></testsuites>'
        with self.bounded(LISTING, junit):
            self.assertEqual(dt.rust_tests(self.build, self.root / "rust.log"),
                             (1, "failed (exit 0): 1 passed, 0 failed, 1 skipped, 1 missing"))
        junit = ('<testsuites><testsuite name="replay"><testcase name="reads"/>'
                 '<testcase name="writes"/></testsuite></testsuites>')
        with self.bounded(LISTING, junit):
            self.assertEqual(dt.rust_tests(self.build, self.root / "rust.log"),
                             (0, "passed: 2 passed, 0 failed, 1 skipped, 0 missing"))

    def boat_run(self, codes, output):
        worktree = self.root / "worktree"
        for name in ("replay", "viewer"):
            (worktree / "crates" / name).mkdir(parents=True, exist_ok=True)
            (worktree / "crates" / name / "Cargo.toml").write_text(f'[package]\nname = "{name}"\n')
        (worktree / "Cargo.toml").write_text('[workspace]\nmembers = ["crates/*"]\n')
        commands = []

        def run(command, stdout, stderr, seconds, environment=None):
            commands.append((command, stderr, seconds))
            stdout.write(output.encode())
            return codes.pop(0)
        with mock.patch.object(dt, "WORKTREE", worktree), mock.patch.object(dt, "bounded", side_effect=run):
            return dt.boat_rust_tests(self.build, self.root / "rust.log"), commands

    ONE = ("running 1 test\ntest a ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured\n"
           "Test program: replay lib replay: x: exit 0\nTests: 1 of 1 programs run\n")

    def test_boat_builds_every_member_and_its_programs_are_counted(self):
        output = ("running 2 tests\ntest a ... ok\ntest b ... ignored\n"
                  "test result: ok. 1 passed; 0 failed; 1 ignored; 0 measured\n"
                  "Test program: replay lib replay: x: exit 0\nTests: 1 of 1 programs run\n")
        (code, outcome), commands = self.boat_run([0], output)
        self.assertEqual((code, outcome), (0, "passed: 1 passed, 0 failed, 1 skipped, 0 missing"))
        command, stderr, seconds = commands[0]
        self.assertEqual(command[1:4], ["tools/boat.py", "test", "--no-fail-fast"])
        self.assertEqual(command[4:], ["-p", "replay", "-p", "viewer"])
        self.assertEqual(stderr, subprocess.STDOUT)
        self.assertLessEqual(seconds, dt.BOAT_RUST_LIMIT.total_seconds())
        self.assertIsNone(self.build.units)

    def test_boat_failing_itself_is_run_once_more_then_named(self):
        (code, outcome), commands = self.boat_run([dt.BOAT_FAILED, 0], self.ONE)
        self.assertEqual(len(commands), 2)
        self.assertEqual((code, outcome), (0, "passed: 1 passed, 0 failed, 0 skipped, 0 missing"))
        self.assertEqual((self.root / "rust.log").read_text(encoding="utf-8").count("test a ... ok"), 2)
        (code, outcome), commands = self.boat_run([dt.BOAT_FAILED, dt.BOAT_FAILED], "BOAT UNAVAILABLE\n")
        self.assertEqual(len(commands), 2)
        self.assertEqual(outcome, "failed: Boat itself failed twice (exit 75); passed not recorded, "
                                  "failed not recorded, skipped not recorded, missing not recorded")

    def test_the_rust_setting_chooses_boat_or_local_and_only_local_sweeps(self):
        for setting, rust, sweep in (("boat", dt.boat_rust_tests, False), ("local", dt.rust_tests, True)):
            with mock.patch.object(dt, "RUST_BUILD", setting):
                plan = {slug: command for _, slug, command, _ in dt.steps()}
            self.assertIs(plan["rust"], rust)
            self.assertEqual("rust-sweep" in plan, sweep)
        with mock.patch.dict(os.environ, {"CARGO_TARGET_DIR": "D:/elsewhere"}):
            with mock.patch.object(dt, "RUST_BUILD", "boat"):
                self.assertEqual(dt.rust_target(), self.worktree / ".cargo-target")
            with mock.patch.object(dt, "RUST_BUILD", "local"):
                self.assertEqual(dt.rust_target(), Path("D:/elsewhere"))

    def test_a_build_that_fails_records_no_counts_and_runs_nothing(self):
        with self.bounded(None, None) as bounded:
            code, outcome = dt.rust_tests(self.build, self.root / "rust.log")
        self.assertEqual(bounded.call_count, 1)
        self.assertEqual(code, 1)
        self.assertEqual(outcome, "failed (exit 101): passed not recorded, failed not recorded, "
                                  "skipped not recorded, missing not recorded")
        self.assertIsNone(self.build.units)


if __name__ == "__main__":
    unittest.main()
