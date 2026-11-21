"""Tests for tools/daily_tests.py: python tools/daily_tests_test.py"""

import contextlib
import io
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import daily_tests as dt


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
        self.close = self.enterContext(mock.patch.object(dt.os, "close"))
        self.git = self.enterContext(mock.patch.object(dt, "git", side_effect=self.git_command))
        self.clock = self.enterContext(mock.patch.object(dt.time, "monotonic", return_value=100.0))
        self.step = self.enterContext(mock.patch.object(dt, "run_step", side_effect=self.complete))
        self.record_history = dt.record_history
        self.history = self.enterContext(mock.patch.object(dt, "record_history", wraps=self.record_history))
        self.write_report = dt.write_report
        self.report = self.enterContext(mock.patch.object(dt, "write_report", wraps=self.write_report))
        self.enterContext(mock.patch.object(dt, "target_dir", return_value=self.worktree / "target"))
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
        with mock.patch.object(dt, "MARKER", wraps=self.marker) as marker:
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
        marker.write_text.assert_called_once()
        marker.unlink.assert_called_once_with(missing_ok=True)
        self.assertFalse(self.marker.exists())
        self.history.assert_called_once()
        self.assertEqual([call.args[0] for call in self.report.call_args_list], ["RUNNING", "PASSED"])
        self.assertEqual(self.report.call_args, mock.call("PASSED", ["synthetic report"]))
        self.assertEqual(len((self.reports / "history.tsv").read_text().splitlines()), 1)

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
        with mock.patch.multiple(dt, WORKTREE=root), mock.patch.object(dt, "steps", return_value=plan),                 mock.patch.object(dt, "git", return_value="c" * 40), mock.patch.object(dt, "target_dir", return_value=root / "target"):
            dt.run(args)
        lines = json.loads((folder / "result.json").read_text(encoding="utf-8"))["lines"]
        self.assertEqual(ran, [])
        self.assertIn("| Both | not run: second failed | | |", lines)


if __name__ == "__main__":
    unittest.main()
