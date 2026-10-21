"""Tests for tools/warm_target.py: python tools/warm_target_test.py

The end-to-end tests build a small workspace with cargo and only run with
WARM_TARGET_END_TO_END=1.
"""

import _winapi
import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import warm_target as wt  # noqa: E402

HASH = "0123456789abcdef"
OTHER = "fedcba9876543210"
THIRD = "00112233445566ff"


def write(path, data, mtime=None):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as handle:
        handle.write(data if isinstance(data, bytes) else data.encode())
    if mtime is not None:
        os.utime(path, ns=(mtime, mtime))


def read(path):
    with open(path, "rb") as handle:
        return handle.read()


class DepInfo(unittest.TestCase):
    def test_round_trip(self):
        files = [(wt.PACKAGE_RELATIVE, "src\\lib.rs"), (wt.PACKAGE_RELATIVE, "C:\\w\\other\\x.md"),
                 (wt.TARGET_RELATIVE, "debug\\build\\a-1\\out\\gen.rs")]
        env = [("CLIPPY_ARGS", "-D__CLIPPY_HACKERY__warnings"), ("UNSET", None)]
        data = wt.encode_dep_info(files, env)
        self.assertTrue(data.startswith(b"\x01\x00\x00\x00\xff\x01"))
        self.assertEqual(wt.parse_dep_info(data), (files, env))

    def test_rejects_unknown_layouts(self):
        good = wt.encode_dep_info([(0, "src\\lib.rs")], [])
        for bad in (b"\x00" + good[1:], good[:5] + b"\x02" + good[6:], good + b"\x00",
                    good[:-5] + b"\x01" + good[-4:]):
            with self.assertRaises(wt.WarmError):
                wt.parse_dep_info(bad)


class Paths(unittest.TestCase):
    def test_shaped_checkout_matches_length_and_depth(self):
        like = "C:\\Users\\Example\\.t3\\worktrees\\battle-plan\\t3-44dbad32"
        path = wt.shaped_checkout("C:\\seed", like)
        self.assertEqual((len(path), path.count("\\")), (len(like), like.count("\\")))
        self.assertTrue(path.startswith("C:\\seed\\"))
        with self.assertRaises(wt.WarmError):
            wt.shaped_checkout("C:\\a\\very\\deep\\seed\\root\\x", like)

    def test_render_forms(self):
        path = "C:\\Seed\\x"
        self.assertEqual(wt.render(path, ("/", "exact", "ascii")), b"C:/Seed/x")
        self.assertEqual(wt.render(path, ("\\\\", "lower", "ascii")), b"c:\\\\seed\\\\x")
        self.assertEqual(wt.render(path, ("\\", "drive", "utf-16-le")), "c:\\Seed\\x".encode("utf-16-le"))

    def test_map_text_keeps_each_form(self):
        mapping = [("C:\\seed\\t", "D:\\tgt"), ("C:\\seed", "D:\\work")]
        text = 'C:\\seed\\t\\debug "C:\\\\seed\\\\src" c:/SEED/a C:\\seedling'
        self.assertEqual(wt.map_text(text, mapping),
                         'D:\\tgt\\debug "D:\\\\work\\\\src" D:/work/a D:\\workling')

    def test_layout_rules(self):
        self.assertTrue(wt.skipped("debug/incremental", True))
        self.assertTrue(wt.skipped("debug/viewer.exe", False))
        self.assertTrue(wt.skipped(".cargo-lock", False))
        self.assertFalse(wt.skipped("debug/deps", True))
        self.assertFalse(wt.skipped("debug/deps/viewer-%s.exe" % HASH, False))
        self.assertEqual(wt.file_kind("debug\\.fingerprint\\a-%s\\dep-lib-a" % HASH), "dep-info")
        self.assertEqual(wt.file_kind("debug\\.fingerprint\\a-%s\\lib-a.json" % HASH), "text")
        self.assertEqual(wt.file_kind("debug\\build\\a-%s\\output" % HASH), "text")
        self.assertEqual(wt.file_kind("debug\\build\\a-%s\\out\\gen.rs" % HASH), "binary")
        self.assertEqual(wt.file_kind("debug\\deps\\a-%s.d" % HASH), "text")
        units = {f"a-{HASH}": ["lib-a", "dep-lib-a"], f"tool-{OTHER}": ["bin-my-tool", "bin-my-tool.json"],
                 f"tool-{THIRD}": ["test-bin-my-tool"]}
        self.assertEqual(wt.owning_units("debug\\deps\\liba-%s.rmeta" % HASH, units), [f"a-{HASH}"])
        self.assertEqual(wt.owning_units("debug\\build\\a-%s\\out\\x.lib" % HASH, units), [f"a-{HASH}"])
        self.assertEqual(wt.owning_units("debug\\deps\\my_tool.pdb", units), [f"tool-{OTHER}"])
        self.assertEqual(wt.owning_units("debug\\deps\\a.dll", units), [f"a-{HASH}"])  # a cdylib
        self.assertEqual(wt.owning_units("debug\\deps\\other.exe", units), [])
        self.assertEqual(wt.owning_units("debug\\viewer.exe", units), [])


class Translate(unittest.TestCase):
    """A seed target with every kind of reference, brought over to a worktree."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        base = self.tmp.name
        self.root = os.path.join(base, "seed")
        self.checkout = os.path.join(self.root, "checkout_")
        self.seed_target = os.path.join(self.checkout, ".cargo-target")
        self.worktree = os.path.join(base, "wtre", "t3-abcdef")  # same length as the checkout
        self.assertEqual(len(self.worktree), len(self.checkout))
        self.target = os.path.join(self.worktree, ".cargo-target")
        self.old = [self.seed_target, self.checkout]
        self.new = [self.target, self.worktree]
        profile = os.path.join(self.seed_target, "debug")
        self.mtime = 1_600_000_000_000_000_000
        include = os.path.join(self.checkout, "crates", "b", "notes.md")
        write(os.path.join(profile, ".fingerprint", f"a-{HASH}", "dep-lib-a"),
              wt.encode_dep_info([(0, "src\\lib.rs"), (0, include)], []), self.mtime)
        write(os.path.join(profile, ".fingerprint", f"a-{HASH}", "lib-a"), "1234", self.mtime)
        rmeta = (b"\x00" + self.checkout.encode() + b"\\crates\\a\\src\\lib.rs\x00"
                 + self.checkout.replace("\\", "/").encode() + b"/x\x00"
                 + self.checkout.replace("\\", "\\\\").encode() + b"\\\\y\x00"
                 + (self.checkout + "\\z").encode("utf-16-le"))
        write(os.path.join(profile, "deps", f"liba-{HASH}.rmeta"), rmeta, self.mtime)
        write(os.path.join(profile, "deps", f"a-{HASH}.d"),
              f"{self.seed_target}\\debug\\deps\\liba-{HASH}.rmeta: crates\\a\\src\\lib.rs\n", self.mtime)
        write(os.path.join(profile, "build", f"c-{OTHER}", "output"),
              f"cargo:rustc-link-search=native={self.seed_target}\\debug\\build\\c-{OTHER}\\out\n",
              self.mtime)
        write(os.path.join(profile, ".fingerprint", f"c-{OTHER}", "run-build-script-build"), "5678")
        # A name the scan does not recognise as a whole path: its unit must be rebuilt.
        write(os.path.join(profile, "build", f"c-{OTHER}", "out", "blob.bin"),
              b"..\\checkout_\\thing", self.mtime)
        # A linker resolving `..` against the checkout reaches the seed's own directory.
        write(os.path.join(profile, ".fingerprint", f"d-{THIRD}", "lib-d"), "9abc")
        write(os.path.join(profile, "deps", f"libd-{THIRD}.rlib"),
              (self.root + "\\third_party\\x.h").encode(), self.mtime)
        write(os.path.join(profile, "deps", f"libd-{THIRD}.rmeta"), b"clean", self.mtime)

    def tearDown(self):
        self.tmp.cleanup()

    def index(self, target):
        files, _ = wt.target_files(target)
        return wt.scan_target(target, [self.seed_target, self.checkout], files, 2,
                              wt.seed_markers(self.checkout, self.root))

    def test_everything_rewritten_or_rebuilt(self):
        index = self.index(self.seed_target)
        self.assertEqual(index[f"debug\\build\\c-{OTHER}\\out\\blob.bin"]["unrecognized"], 1)
        self.assertEqual(index[f"debug\\deps\\libd-{THIRD}.rlib"]["unrecognized"], 1)
        os.makedirs(self.worktree)
        wt.copy_robocopy(self.seed_target, self.target, 4)
        counts = wt.translate(self.target, index, self.old, self.new, set())
        self.assertEqual(counts["invalidated units"], 2)
        profile = os.path.join(self.target, "debug")
        self.assertFalse(os.path.exists(os.path.join(profile, ".fingerprint", f"c-{OTHER}")))
        self.assertFalse(os.path.exists(os.path.join(profile, "build", f"c-{OTHER}")))
        self.assertFalse(os.path.exists(os.path.join(profile, ".fingerprint", f"d-{THIRD}")))
        self.assertFalse(os.path.exists(os.path.join(profile, "deps", f"libd-{THIRD}.rmeta")))
        self.assertEqual(self.index(self.target), {})
        rmeta = os.path.join(profile, "deps", f"liba-{HASH}.rmeta")
        self.assertIn(self.worktree.replace("\\", "/").encode() + b"/x", read(rmeta))
        self.assertIn((self.worktree + "\\z").encode("utf-16-le"), read(rmeta))
        files, _ = wt.parse_dep_info(read(os.path.join(profile, ".fingerprint", f"a-{HASH}", "dep-lib-a")))
        self.assertEqual(files[1][1], os.path.join(self.worktree, "crates", "b", "notes.md"))
        for rel in (f"deps\\liba-{HASH}.rmeta", f".fingerprint\\a-{HASH}\\dep-lib-a", f"deps\\a-{HASH}.d"):
            self.assertEqual(os.stat(os.path.join(profile, rel)).st_mtime_ns, self.mtime)

    def test_unequal_lengths_rebuild_the_unit(self):
        index = self.index(self.seed_target)
        longer = self.worktree + "x"
        os.makedirs(longer)
        target = os.path.join(longer, ".cargo-target")
        wt.copy_robocopy(self.seed_target, target, 4)
        counts = wt.translate(target, index, self.old, [target, longer], set())
        profile = os.path.join(target, "debug")
        self.assertEqual(counts["invalidated units"], 3)
        self.assertFalse(os.path.exists(os.path.join(profile, ".fingerprint", f"a-{HASH}")))
        self.assertFalse(os.path.exists(os.path.join(profile, "deps", f"liba-{HASH}.rmeta")))
        self.assertEqual(self.index(target), {})

    def test_environment_naming_the_seed_rebuilds_the_unit(self):
        dep_info = os.path.join(self.seed_target, "debug", ".fingerprint", f"a-{HASH}", "dep-lib-a")
        write(dep_info, wt.encode_dep_info([(0, "src\\lib.rs")], [("DATA", self.checkout + "\\d")]))
        index = self.index(self.seed_target)
        os.makedirs(self.worktree)
        wt.copy_robocopy(self.seed_target, self.target, 4)
        wt.translate(self.target, index, self.old, self.new, set())
        self.assertFalse(os.path.exists(os.path.join(self.target, "debug", ".fingerprint", f"a-{HASH}")))

    def test_seed_paths_written_verbatim_rebuild_the_unit(self):
        dep_info = os.path.join(self.seed_target, "debug", ".fingerprint", f"a-{HASH}", "dep-lib-a")
        write(dep_info, wt.encode_dep_info([(1, "\\\\?\\" + self.checkout + "\\crates\\b\\notes.md")], []))
        index = self.index(self.seed_target)
        os.makedirs(self.worktree)
        wt.copy_robocopy(self.seed_target, self.target, 4)
        wt.translate(self.target, index, self.old, self.new, set())
        self.assertFalse(os.path.exists(os.path.join(self.target, "debug", ".fingerprint", f"a-{HASH}")))

    def test_forced_packages_rebuild(self):
        os.makedirs(self.worktree)
        wt.copy_robocopy(self.seed_target, self.target, 4)
        wt.translate(self.target, {}, self.old, self.new, {"a"})
        self.assertFalse(os.path.exists(os.path.join(self.target, "debug", ".fingerprint", f"a-{HASH}")))
        self.assertTrue(os.path.exists(os.path.join(self.target, "debug", ".fingerprint", f"c-{OTHER}")))

    def test_copy_keeps_every_file_and_mtime(self):
        files, stamp = wt.target_files(self.seed_target)
        wt.copy_seed(self.seed_target, self.target, files, 4)
        self.assertEqual(wt.target_files(self.target)[1], stamp)
        rel = f"debug\\deps\\liba-{HASH}.rmeta"
        self.assertNotEqual(os.stat(os.path.join(self.target, rel)).st_ino,
                            os.stat(os.path.join(self.seed_target, rel)).st_ino)
        self.assertEqual(os.stat(os.path.join(self.target, rel)).st_mtime_ns, self.mtime)


class Pdb(unittest.TestCase):
    def test_paths_split_across_pages_are_reported(self):
        root = "C:\\seed\\checkout_"
        page = 512

        def pdb(pages):
            header = wt.MSF_MAGIC + page.to_bytes(4, "little")
            data = bytearray(header.ljust(page, b"\x00"))
            for content, at_end in pages:
                block = bytearray(page)
                if at_end:
                    block[page - len(content):] = content
                else:
                    block[:len(content)] = content
                data += block
            return bytes(data)

        with tempfile.TemporaryDirectory() as target:
            whole = root.encode() + b"\\a.o"
            cases = {
                "split.pdb": [(b"C:\\seed\\chec", True), (b"zz", False), (b"kout_\\a.o", False)],
                "whole.pdb": [(whole, False), (b"C", True), (b"\\other", False)],
            }
            for name, pages in cases.items():
                write(os.path.join(target, "debug", "deps", name), pdb(pages))
            found = {rel.rsplit("\\", 1)[-1]: (len(at), bad) for rel, at, bad in
                     (wt.scan_file((target, os.path.join("debug", "deps", n), [root], ["checkout_"]))
                      for n in cases)}
            self.assertEqual(found, {"split.pdb": (0, 1), "whole.pdb": (1, 0)})


class Backdate(unittest.TestCase):
    def test_only_identical_inputs_are_backdated(self):
        with tempfile.TemporaryDirectory() as base:
            seed, mine = os.path.join(base, "seed"), os.path.join(base, "mine")
            old = 1_500_000_000_000_000_000
            for root in (seed, mine):
                write(os.path.join(root, "a", "same.rs"), "fn a() {}")
                write(os.path.join(root, "a", "dir", "x.txt"), "x")
            write(os.path.join(seed, "a", "changed.rs"), "fn b() {}", old)
            write(os.path.join(mine, "a", "changed.rs"), "fn b() { 1 }", old)
            write(os.path.join(mine, "a", "dir2", "x.txt"), "x")
            write(os.path.join(seed, "a", "dir2", "x.txt"), "x")
            write(os.path.join(mine, "a", "dir2", "new.txt"), "new")
            for path in ("a\\same.rs", "a\\dir\\x.txt", "a\\dir", "a\\dir2", "a\\dir2\\x.txt"):
                os.utime(os.path.join(seed, path), ns=(old, old))
            write(os.path.join(seed, "a", "late.rs"), "fn c() {}", old)
            write(os.path.join(base, "outside.txt"), "outside", old)
            write(os.path.join(base, "seed_outside.txt"), "outside", old)
            started = time.time_ns()
            time.sleep(0.01)
            write(os.path.join(mine, "a", "late.rs"), "fn c() {}")  # a rebase during the warm
            late = os.stat(os.path.join(mine, "a", "late.rs")).st_mtime_ns
            before = time.time_ns()
            inputs = {"a\\same.rs", "a\\changed.rs", "a\\dir", "a\\dir2", "a\\missing.rs",
                      os.path.join(mine, "a", "dir", "x.txt"), "a\\late.rs", "a\\..\\..\\outside.txt"}
            self.assertEqual(wt.backdate(inputs, mine, seed, started), (3, 2))
            mtime = lambda rel: os.stat(os.path.join(mine, rel)).st_mtime_ns
            self.assertEqual(mtime("a\\late.rs"), late)
            self.assertEqual(os.stat(os.path.join(base, "outside.txt")).st_mtime_ns, old)
            self.assertEqual(mtime("a\\same.rs"), old)
            self.assertEqual(mtime("a\\dir\\x.txt"), old)
            self.assertEqual(mtime("a\\dir"), old)
            self.assertGreaterEqual(mtime("a\\changed.rs"), before)
            self.assertGreaterEqual(mtime("a\\dir2"), before)
            self.assertGreaterEqual(mtime("a\\dir2\\x.txt"), before)

    def test_source_inputs_resolve_packages_and_build_scripts(self):
        with tempfile.TemporaryDirectory() as staging:
            fingerprints = os.path.join(staging, "debug", ".fingerprint")
            write(os.path.join(fingerprints, f"a-{HASH}", "dep-lib-a"),
                  wt.encode_dep_info([(0, "src\\lib.rs"), (1, "debug\\build\\x\\out\\g.rs"),
                                      (0, "C:\\w\\crates\\b\\n.md"),
                                      (1, "C:\\w\\crates\\a\\src\\../../../ui/x.ts")], []))
            write(os.path.join(fingerprints, f"a-{OTHER}", "run-build-script-build-script-build.json"),
                  json.dumps({"local": [{"RerunIfChanged": {"output": "o", "paths": ["src\\x.c"]}}]}))
            write(os.path.join(fingerprints, f"serde-{HASH}", "dep-lib-serde"),
                  wt.encode_dep_info([(1, "debug\\build\\serde-1\\out\\private.rs")], []))
            inputs = wt.source_inputs(staging, {"a": "crates\\a"})
            self.assertEqual(inputs, {"crates\\a\\src\\lib.rs", "C:\\w\\crates\\b\\n.md", "crates\\a\\src\\x.c",
                                      "C:\\w\\crates\\a\\src\\../../../ui/x.ts"})
            write(os.path.join(fingerprints, f"serde-{HASH}", "dep-lib-serde"),
                  wt.encode_dep_info([(0, "src\\lib.rs")], []))
            with self.assertRaises(wt.WarmError):
                wt.source_inputs(staging, {"a": "crates\\a"})


class Precalculated(unittest.TestCase):
    def test_build_scripts_watching_their_whole_package_rerun_when_files_come_or_go(self):
        with tempfile.TemporaryDirectory() as base:
            seed, mine, staging = (os.path.join(base, n) for n in ("seed", "mine", "staging"))
            for root in (seed, mine):
                write(os.path.join(root, "crates", "u", "build.rs"), "fn main() {}")
                write(os.path.join(root, "crates", "w", "build.rs"), "fn main() {}")
                write(os.path.join(root, ".gitignore"), "/.cargo-target/\n")
                subprocess.run(["git", "init", "-q"], cwd=root, check=True)
                subprocess.run(["git", "-c", "core.autocrlf=false", "add", "."], cwd=root, check=True)
            write(os.path.join(mine, "crates", "w", "new.rs"), "// untracked")
            write(os.path.join(mine, ".cargo-target", "x"), "ignored")
            fingerprints = os.path.join(staging, "debug", ".fingerprint")
            for unit in (f"u-{HASH}", f"w-{HASH}"):
                write(os.path.join(fingerprints, unit, "run-build-script-build-script-build.json"),
                      json.dumps({"local": [{"Precalculated": "1.0s (build.rs)"}]}))
            write(os.path.join(fingerprints, f"w-{OTHER}", "run-build-script-build-script-build.json"),
                  json.dumps({"local": [{"RerunIfChanged": {"output": "o", "paths": ["build.rs"]}}]}))
            packages = {"u": "crates\\u", "w": "crates\\w"}
            self.assertEqual(wt.precalculated_units(staging, packages, mine, seed), [f"w-{HASH}"])
            self.assertEqual(wt.checkout_files(mine) - wt.checkout_files(seed), {"crates\\w\\new.rs"})


class UpdateLock(unittest.TestCase):
    def test_a_killed_update_does_not_hold_the_seed(self):
        with tempfile.TemporaryDirectory() as root:
            lock = wt.take_update_lock(root)
            with self.assertRaises(wt.WarmError):  # this process is alive
                wt.take_update_lock(root)
            exited = subprocess.Popen([sys.executable, "-c", "pass"])
            exited.wait()
            write(os.path.join(lock, "pid"), str(exited.pid))
            self.assertEqual(wt.take_update_lock(root), lock)
            self.assertEqual(read(os.path.join(lock, "pid")).decode(), str(os.getpid()))
            os.remove(os.path.join(lock, "pid"))  # killed before writing its pid
            with self.assertRaises(wt.WarmError):
                wt.take_update_lock(root)
            old = time.time() - 120
            os.utime(lock, (old, old))
            self.assertEqual(wt.take_update_lock(root), lock)


class Staging(unittest.TestCase):
    def test_a_killed_warm_leaves_nothing_behind_for_long(self):
        with tempfile.TemporaryDirectory() as base:
            root, target = os.path.join(base, "seed"), os.path.join(base, "target")
            exited = subprocess.Popen([sys.executable, "-c", "pass"])
            exited.wait()
            dead = os.path.join(target, f"{wt.STAGING_PREFIX}{exited.pid}")
            write(os.path.join(dead, "debug", "deps", "big.rlib"), "x")
            write(os.path.join(root, wt.STAGING_RECORDS, str(exited.pid)), dead)
            live = os.path.join(target, f"{wt.STAGING_PREFIX}{os.getpid()}")
            record = wt.register_staging(root, live)
            os.makedirs(live)
            wt.sweep_staging(root)
            self.assertFalse(os.path.exists(dead))
            self.assertTrue(os.path.exists(live) and os.path.exists(record))
            self.assertEqual(os.listdir(os.path.join(root, wt.STAGING_RECORDS)), [str(os.getpid())])
            self.assertEqual(wt.staging_owner(".warming-123"), 123)
            self.assertIsNone(wt.staging_owner(".warming-abc"))
            self.assertIsNone(wt.staging_owner("debug"))


class External(unittest.TestCase):
    def test_sweep_keeps_owned_young_and_filling_directories(self):
        with tempfile.TemporaryDirectory() as base:
            root = os.path.join(base, "root")
            names = {n: os.path.join(root, f"{n}{wt.EXTERNAL_SUFFIX}") for n in
                     ("owned", "orphan", "young", "filling", "moved")}
            for directory in names.values():
                os.makedirs(os.path.join(directory, "debug"))
            junction = os.path.join(base, "worktree", ".cargo-target", "debug")
            os.makedirs(os.path.dirname(junction))
            _winapi.CreateJunction(os.path.join(names["owned"], "debug"), junction)
            write(os.path.join(names["owned"], wt.OWNER_FILE), junction)
            write(os.path.join(names["moved"], wt.OWNER_FILE), junction)  # now points elsewhere
            os.makedirs(os.path.join(names["filling"], f"{wt.STAGING_PREFIX}{os.getpid()}"))
            for name in ("owned", "orphan", "filling", "moved"):
                os.utime(names[name], (0, 0))
            wt.sweep_external([root, os.path.join(base, "missing")])
            self.assertEqual(sorted(os.listdir(root)),
                             sorted(f"{n}{wt.EXTERNAL_SUFFIX}" for n in ("owned", "young", "filling")))
            self.assertEqual(os.path.normcase(wt.junction_target(junction)),
                             os.path.normcase(os.path.join(names["owned"], "debug")))
            self.assertIsNone(wt.junction_target(names["young"]))

    def test_profile_directories_are_named_per_worktree_path(self):
        a = wt.external_profile_dir("G:\\dev", "C:\\one\\t3-1234")
        b = wt.external_profile_dir("G:\\dev", "C:\\two\\t3-1234")
        self.assertNotEqual(a, b)
        self.assertTrue(a.startswith("G:\\dev\\t3-1234-") and a.endswith(wt.EXTERNAL_SUFFIX))


class Garbage(unittest.TestCase):
    def test_units_outside_the_last_build_are_removed(self):
        with tempfile.TemporaryDirectory() as profile:
            live_lib = os.path.join(profile, "deps", f"liba-{HASH}.rlib")
            write(live_lib, "lib")
            write(os.path.join(profile, "deps", "viewer.exe"), "exe")
            # Two units of one bin share deps\viewer.exe; the one that linked it last owns it.
            write(os.path.join(profile, ".fingerprint", f"viewer-{THIRD}", "bin-viewer"), "bin")
            write(os.path.join(profile, ".fingerprint", f"viewer-{THIRD}", "dep-bin-viewer"), "", 2_000)
            write(os.path.join(profile, ".fingerprint", f"viewer-{OTHER}", "bin-viewer"), "bin")
            write(os.path.join(profile, ".fingerprint", f"viewer-{OTHER}", "dep-bin-viewer"), "", 1_000)
            os.makedirs(os.path.join(profile, ".fingerprint", f"empty-{OTHER}"))
            write(os.path.join(profile, "deps", f"liba-{OTHER}.rlib"), "old")
            write(os.path.join(profile, ".fingerprint", f"a-{OTHER}", "lib-a"), "old")
            write(os.path.join(profile, ".fingerprint", f"a-{HASH}", "lib-a"), "new")
            write(os.path.join(profile, "build", f"c-{HASH}", "out", "x"), "out")
            log = os.path.join(profile, "log.json")
            messages = [{"reason": "compiler-artifact", "filenames": [live_lib], "executable": None},
                        {"reason": "compiler-artifact", "filenames": [os.path.join(profile, "viewer.exe")],
                         "executable": os.path.join(profile, "viewer.exe")},
                        {"reason": "build-script-executed", "out_dir": os.path.join(profile, "build", f"c-{HASH}", "out")}]
            write(log, "\n".join(json.dumps(m) for m in messages) + "\n   Compiling x\n")
            failed = os.path.join(profile, "failed.json")
            write(failed, "error: unexpected argument\n")
            self.assertIsNone(wt.live_units([log, failed], profile))
            live = wt.live_units([log], profile)
            self.assertEqual(live, {HASH})
            wt.collect_garbage(profile, live)
            self.assertFalse(os.path.exists(os.path.join(profile, "deps", f"liba-{OTHER}.rlib")))
            self.assertFalse(os.path.exists(os.path.join(profile, ".fingerprint", f"a-{OTHER}")))
            self.assertFalse(os.path.exists(os.path.join(profile, ".fingerprint", f"viewer-{OTHER}")))
            self.assertFalse(os.path.exists(os.path.join(profile, ".fingerprint", f"empty-{OTHER}")))
            for kept in (live_lib, os.path.join(profile, ".fingerprint", f"a-{HASH}"),
                         os.path.join(profile, ".fingerprint", f"viewer-{THIRD}"),
                         os.path.join(profile, "deps", "viewer.exe"),
                         os.path.join(profile, "build", f"c-{HASH}")):
                self.assertTrue(os.path.exists(kept))


@unittest.skipUnless(os.environ.get("WARM_TARGET_END_TO_END"),
                     "builds a workspace with cargo; set WARM_TARGET_END_TO_END=1")
class EndToEnd(unittest.TestCase):
    """Seeds a small synthetic workspace with real cargo and checks what each change rebuilds.

    Crate `a` includes a file outside its package and bakes in `CARGO_MANIFEST_DIR`; `b`
    depends on `a` and has a build script without `rerun-if-changed` paths; `c` is a bin
    whose test runs on `b`.
    """

    TOOL = os.path.join(os.path.dirname(os.path.abspath(__file__)), "warm_target.py")

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        base = self.tmp.name
        self.repo = os.path.join(base, "repo")
        files = {
            "Cargo.toml": '[workspace]\nresolver = "3"\nmembers = ["crates/*"]\n',
            "rust-toolchain.toml": read(os.path.join(wt.ROOT, "rust-toolchain.toml")).decode(),
            ".cargo/config.toml": '[build]\ntarget-dir = ".cargo-target"\n',
            ".gitignore": "/.cargo-target/\n",
            "shared/data.txt": "seed data",
            "crates/a/Cargo.toml": '[package]\nname = "a"\nversion = "0.1.0"\nedition = "2024"\n',
            "crates/a/src/lib.rs": (
                'pub fn data() -> &\'static str {\n    include_str!("../../../shared/data.txt")\n}\n'
                'pub const DIR: &str = env!("CARGO_MANIFEST_DIR");\n'
                '#[test]\nfn built_for_this_checkout() {\n'
                '    assert_eq!(DIR, std::env::var("CARGO_MANIFEST_DIR").unwrap());\n}\n'),
            "crates/b/Cargo.toml": ('[package]\nname = "b"\nversion = "0.1.0"\nedition = "2024"\n'
                                    '[dependencies]\na = { path = "../a" }\n'),
            "crates/b/build.rs": "fn main() {}\n",
            "crates/b/notes.txt": "notes\n",
            "crates/b/src/lib.rs": "pub fn size() -> usize {\n    a::data().len()\n}\n",
            "crates/c/Cargo.toml": ('[package]\nname = "c"\nversion = "0.1.0"\nedition = "2024"\n'
                                    '[dependencies]\nb = { path = "../b" }\n'),
            "crates/c/src/main.rs": 'fn main() {\n    println!("{}", b::size());\n}\n',
        }
        for rel, text in files.items():
            write(os.path.join(self.repo, rel), text)
        self.git(self.repo, "init", "-q")
        self.cargo(self.repo, "generate-lockfile", "--offline")
        self.commit("base")
        write(os.path.join(self.repo, "crates", "a", "src", "lib.rs"),
              files["crates/a/src/lib.rs"] + "// the seed's version\n")
        self.commit("seed")
        self.worktree = os.path.join(base, "w", "t3-" + "a" * 24)
        self.root = os.path.join(base, "s")
        self.tool("seed", "--root", self.root, "--like", self.worktree, "--repo", self.repo,
                  "--no-fetch", "--commit", "HEAD", "-j", "4")

    def tearDown(self):
        self.tmp.cleanup()

    def git(self, cwd, *args):
        # Checkouts use the user's own settings, as the tool's do: seed and worktree must
        # agree on line endings for their files to compare equal.
        subprocess.run(["git", *args], cwd=cwd, check=True, capture_output=True)

    def commit(self, message):
        self.git(self.repo, "-c", "core.safecrlf=false", "add", "-A")
        self.git(self.repo, "-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", message)

    def cargo(self, cwd, *args):
        env = dict(os.environ)
        env.pop("CARGO_TARGET_DIR", None)
        return subprocess.run(["cargo", *args], cwd=cwd, env=env, capture_output=True, text=True,
                              encoding="utf-8", errors="replace")

    def tool(self, *args, cwd=None):
        result = subprocess.run([sys.executable, self.TOOL, *args], cwd=cwd, capture_output=True,
                                text=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return result.stdout

    def fresh_worktree(self, commit="HEAD"):
        if os.path.exists(self.worktree):
            self.git(self.repo, "worktree", "remove", "--force", self.worktree)
        self.git(self.repo, "worktree", "add", "--detach", self.worktree, commit)

    def rebuilt(self, *args):
        """Workspace packages cargo compiled (not just checked as fresh) for `cargo <args>`."""
        result = self.cargo(self.worktree, *args, "--message-format=json")
        self.assertEqual(result.returncode, 0, result.stderr)
        names = set()
        for line in result.stdout.splitlines():
            message = json.loads(line) if line.startswith("{") else {}
            if message.get("reason") == "compiler-artifact" and not message["fresh"]:
                names.add(message["target"]["name"])
        return names

    def warm(self, edit=None):
        self.fresh_worktree()
        if edit:
            edit()
        out = self.tool("warm", "--root", self.root, cwd=self.worktree)
        self.assertIn("seeded from", out)
        self.tool("check", "--root", self.root, "--target-dir",
                  os.path.join(self.worktree, ".cargo-target"))

    def test_unchanged_worktree_rebuilds_nothing_and_runs_with_its_own_paths(self):
        self.warm()
        self.assertEqual(self.rebuilt("check", "--workspace", "--all-targets"), set())
        self.assertEqual(self.rebuilt("test", "--workspace", "--no-run"), set())
        result = self.cargo(self.worktree, "test", "--workspace")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_changes_rebuild_exactly_their_dependents(self):
        seed_target = wt.read_seed(self.root)["target"]
        seed_files = wt.target_files(seed_target)[1]
        append = lambda rel: lambda: write(
            os.path.join(self.worktree, rel), read(os.path.join(self.worktree, rel)) + b"\n// edit\n")
        self.warm(append(os.path.join("crates", "b", "src", "lib.rs")))
        self.assertEqual(self.rebuilt("check", "--workspace", "--all-targets"), {"b", "c"})
        self.warm(append(os.path.join("shared", "data.txt")))
        self.assertEqual(self.rebuilt("check", "--workspace", "--all-targets"), {"a", "b", "c"})

        def revert():
            path = os.path.join(self.worktree, "crates", "b", "src", "lib.rs")
            original = read(path)
            write(path, original + b"// edit\n")
            write(path, original)  # same bytes, new mtime
        self.warm(revert)
        self.assertEqual(self.rebuilt("check", "--workspace", "--all-targets"), set())

        # `b`'s build script watches every file of its package, so a file leaving or joining
        # it reruns the script and rebuilds `b`.
        self.warm(lambda: os.remove(os.path.join(self.worktree, "crates", "b", "notes.txt")))
        self.assertEqual(self.rebuilt("check", "--workspace", "--all-targets"), {"b", "c"})
        self.warm(lambda: write(os.path.join(self.worktree, "crates", "b", "new.txt"), "untracked"))
        self.assertEqual(self.rebuilt("check", "--workspace", "--all-targets"), {"b", "c"})
        # Nothing a worktree builds reaches the seed.
        self.assertEqual(self.rebuilt("test", "--workspace", "--no-run"), {"b", "c"})
        self.assertEqual(wt.target_files(seed_target)[1], seed_files)

    def test_cargo_building_during_the_warm_keeps_its_own_build(self):
        # The worktree starts at the base commit, where `a` differs from the seed. Cargo builds
        # it while the copy is made, then a rebase makes `a` equal the seed's.
        self.fresh_worktree("HEAD~1")
        seed_a = read(os.path.join(self.repo, "crates", "a", "src", "lib.rs"))
        translate = wt.translate

        def racing(*args, **kwargs):
            counts = translate(*args, **kwargs)
            self.assertEqual(self.cargo(self.worktree, "check", "--workspace").returncode, 0)
            write(os.path.join(self.worktree, "crates", "a", "src", "lib.rs"), seed_a)
            return counts

        wt.translate = racing
        try:
            args = argparse.Namespace(root=self.root, worktree=self.worktree, target_dir=None,
                                      copy="copyfile2", threads=4, target_roots=None, min_free_gb=60)
            self.assertEqual(wt.command_warm(args), 0)
        finally:
            wt.translate = translate
        self.assertIn("a", self.rebuilt("check", "--workspace"))

    def test_profile_directory_on_another_root_behind_a_junction(self):
        roots = [os.path.join(self.tmp.name, "drive1"), os.path.join(self.tmp.name, "drive2")]
        joined = ";".join(roots)
        self.fresh_worktree()
        out = self.tool("warm", "--root", self.root, "--target-roots", joined, "--min-free-gb", "1",
                        cwd=self.worktree)
        self.assertIn("profile directory on", out)
        profile = os.path.join(self.worktree, ".cargo-target", "debug")
        external = wt.junction_target(profile)
        self.assertTrue(any(os.path.normcase(external).startswith(os.path.normcase(r)) for r in roots))
        self.assertEqual(self.rebuilt("check", "--workspace", "--all-targets"), set())
        self.tool("check", "--root", self.root, "--target-dir", os.path.join(self.worktree, ".cargo-target"))
        # A junction whose directory is gone is replaced; one that resolves is kept.
        out = self.tool("warm", "--root", self.root, "--target-roots", joined, cwd=self.worktree)
        self.assertIn("already holds a build", out)
        shutil.rmtree(os.path.dirname(external))
        out = self.tool("warm", "--root", self.root, "--target-roots", joined, "--min-free-gb", "1",
                        cwd=self.worktree)
        self.assertIn("seeded from", out)
        self.assertEqual(self.rebuilt("check", "--workspace", "--all-targets"), set())
        # Removing the worktree leaves its profile directory for the next warm to sweep.
        external = os.path.dirname(wt.junction_target(profile))
        self.fresh_worktree()
        os.utime(external, (0, 0))
        out = self.tool("warm", "--root", self.root, "--target-roots", joined, "--min-free-gb", "100000",
                        cwd=self.worktree)
        self.assertIn("floor", out)
        self.assertFalse(os.path.exists(external))
        self.assertEqual(os.listdir(os.path.join(self.worktree, ".cargo-target")), [])


if __name__ == "__main__":
    unittest.main()
