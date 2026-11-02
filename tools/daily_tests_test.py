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
import daily_tests as dt  # noqa: E402


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
            # Newest for the example though older for the binary: still this build's.
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


if __name__ == "__main__":
    unittest.main()
