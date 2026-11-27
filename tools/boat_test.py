"""Tests for tools/boat.py against a fake Boat: python tools/boat_test.py"""

import ast
import contextlib
import io
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import threading
import time
import types
import unittest
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import boat as bt

OK = types.SimpleNamespace(returncode=0, stdout="", stderr="")

class FakeBoat:
    """VMs as Boat lists them; records every call and lets a test fail chosen ones."""

    def __init__(self, vms):
        self.vms = {vm["id"]: dict(vm) for vm in vms}
        self.calls = []
        self.remote_calls = []
        self.acquire_status = {}
        self.acquire_output = {}
        self.sync_status = 0
        self.synced = []
        self.fail_downloads = False
        self.cargo_exit = 0
        self.outcome = None
        self.renewed = threading.Event()
        self.missing = set()
        self.failing = set()
        self.refused = set()
        self.starting = {}
        self.hydrating = {}
        self.too_small = set()
        self.single = []

    def boat(self, *args, check=True, idempotent=None, shielded=False, retry=True):
        self.calls.append(args)
        if not retry:
            self.single.append(args[0])
        command = args[0]
        if command == "list":
            return OK, [{"sandboxes": [dict(vm) for vm in self.vms.values()]}]
        if command == "info":
            vm = self.vms[args[1]]
            if vm["state"] == "starting":
                self.starting[vm["id"]] -= 1
                vm["state"] = "starting" if self.starting[vm["id"]] > 0 else "ready"
            elif vm["id"] in self.hydrating:
                vm["hydration"] = {"entriesDone": vm.get("hydration", {}).get("entriesDone", 0) + 1}
                self.hydrating[vm["id"]] -= 1
                vm["hydrated"] = self.hydrating[vm["id"]] <= 0
            return OK, [{"sandbox": dict(vm)}]
        if command == "extend":
            self.vms[args[1]]["ttlSeconds"] = int(args[3])
            if int(args[3]) == bt.LEASE_SECONDS:
                self.renewed.set()
            return OK, []
        if command == "resume":
            vm = self.vms[args[1]]
            if vm["id"] in self.refused:
                raise bt.Failure("Boat resume failed (1): Please reduce your request rate")
            vm.update(state="starting" if vm["id"] in self.starting else "idle", ttlSeconds=int(args[3]))
            if "--type" in args:
                vm["type"] = args[args.index("--type") + 1]
            return OK, []
        if command == "new":
            source = args[args.index("--from") + 1]
            if source in self.missing:
                return types.SimpleNamespace(returncode=1, stdout="", stderr=f'No saved snapshot "{source}".'), []
            kind = args[args.index("--type") + 1]
            if kind in self.too_small:
                return types.SimpleNamespace(returncode=1, stdout="", stderr=""), [{"code": "deploy_failed", "error": f"This sandbox uses 87.3 GB of disk, which does not fit any available '{kind}' machine (largest budget: 76.0 GB).", "event": "error"}]
            self.vms["n"] = {"id": "n", "state": "ready", "type": kind, "ttlSeconds": bt.LEASE_SECONDS, "hydrated": True}
            return OK, [{"event": "created", "id": "n"}]
        if command == "snapshot" or (command == "exec" and "source-revision" in args[-1]):
            return OK, []
        if command == "push":
            files = [path for path in Path(args[2]).rglob("*") if path.is_file() and ".git" not in path.parts]
            return OK, [{"files": len(files)}]
        if command == "exec" and "--timeout" in args:
            if self.fail_downloads and any("dd if=" in arg for arg in args):
                raise bt.Failure("Boat exec failed")
            return OK, []
        raise AssertionError(f"unexpected boat call {args}")

    def quick(self, vmid, *args):
        self.remote_calls.append((vmid, *args))
        mode = args[0]
        if mode == "acquire":
            return self.acquire_status.get(vmid, 0), self.acquire_output.get(vmid, "{}")
        if mode == "release":
            return 0, ""
        if mode == "outcome" and self.outcome is not None:
            return 0, json.dumps(self.outcome)
        if mode == "outcome":
            artifact = {"path": f"{bt.REMOTE}/target/{bt.TARGET}/release/viewer.exe", "sha256": "0" * 64}
            bundle = {"path": f"{bt.REMOTE}/artifacts-x.tar.xz", "size": 10, "sha256": "0" * 64}
            return 0, json.dumps({"exit": 0, "artifacts": [artifact], "bundle": bundle})
        raise AssertionError(f"unexpected quick call {args}")

    def remote(self, vmid, mode, token, *args):
        self.remote_calls.append((vmid, mode, token, *args))
        if args[:1] == ("--sync",):
            self.synced.append(args[1])
            if self.sync_status:
                return self.sync_status, "j"
        if mode == "check":
            return self.cargo_exit, "j"
        if mode in {"build", "test"}:
            return self.cargo_exit, "j"
        if mode in {"ui", "provenance", "prune", "clear"}:
            return int(mode in self.failing), "j"
        raise AssertionError(f"unexpected remote call {mode} {args}")

    def ttl_calls(self, vmid):
        return [int(call[3]) for call in self.calls if call[0] == "extend" and call[1] == vmid]

def vm(vmid, state="idle", type="large", ttl=bt.IDLE_SECONDS, hydrated=True):
    return {"id": vmid, "state": state, "type": type, "ttlSeconds": ttl, "hydrated": hydrated}

class Harness(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.state = bt.State(self.root / "state")
        self.saved = {name: getattr(bt, name) for name in ("boat", "remote", "quick", "deliver", "run_program", "wait_ready", "info", "RENEW_SECONDS", "UPLOAD_CHUNKS")}
        self.now = 0.0

        def pause(seconds):
            self.now += seconds

        bt.wait_ready = lambda vmid, spent=None: self.saved["wait_ready"](vmid, spent, clock=lambda: self.now, pause=pause)

    def tearDown(self):
        for name, value in self.saved.items():
            setattr(bt, name, value)
        self.temp.cleanup()

    def use(self, fake):
        bt.boat = fake.boat
        bt.remote = fake.remote
        bt.quick = fake.quick
        for vmid in fake.vms:
            bt.atomic_json(self.state.pool / (vmid + ".json"), {"id": vmid, "snapshot": bt.SNAPSHOT})
        return fake

class Rank(unittest.TestCase):
    def test_running_before_stopped_then_lane_then_size(self):
        states = {vmid: vm(vmid, state, type) for vmid, state, type in [("a", "stopped", "default"), ("b", "stopped", "large"), ("c", "idle", "large"), ("d", "idle", "large")]}
        self.assertEqual(bt.rank(sorted(states), states, "d", "large", lambda _: False), ["d", "c", "b", "a"])

    def test_takes_the_vm_another_lane_left_longest_ago(self):
        states = {vmid: vm(vmid) for vmid in "abc"}
        used = {"a": 300, "b": 100, "c": 200}.get
        self.assertEqual(bt.rank(sorted(states), states, "c", "large", lambda _: False, used), ["c", "b", "a"])

    def test_never_picks_stopping_cooling_or_unlisted_vms(self):
        states = {"a": vm("a", "stopping"), "b": vm("b"), "c": vm("c", "stopped")}
        self.assertEqual(bt.rank(["a", "b", "c", "gone"], states, None, "large", lambda vmid: vmid == "b"), ["c"])

    def test_a_running_vm_still_hydrating_comes_after_the_hydrated_ones(self):
        states = {"a": vm("a", hydrated=False), "b": vm("b"), "c": vm("c", "stopped", hydrated=False), "d": vm("d", "stopped")}
        self.assertEqual(bt.rank(sorted(states), states, "c", "large", lambda _: False), ["b", "a", "c", "d"])

    def test_a_vm_still_starting_comes_after_the_running_ones_and_before_the_stopped(self):
        states = {"a": vm("a", "stopped"), "b": vm("b", "starting"), "c": vm("c"), "d": vm("d", "resuming"), "e": vm("e", "provisioning"), "f": vm("f", "cloning")}
        self.assertEqual(bt.rank(sorted(states), states, None, "large", lambda _: False), ["c", "b", "d", "e", "f", "a"])

    def test_large_work_skips_running_default_vms_but_can_grow_stopped_ones(self):
        states = {"a": vm("a", "idle", "default"), "b": vm("b", "stopped", "default")}
        self.assertEqual(bt.rank(["a", "b"], states, None, "large", lambda _: False), ["b"])

    def test_default_work_skips_large_vms_unless_cloned_as_a_default_fallback(self):
        states = {"a": vm("a", "stopped", "default"), "b": vm("b", "idle", "large"), "c": vm("c", "stopped", "large")}
        self.assertEqual(bt.rank(["a", "b", "c"], states, "b", "default", lambda _: False), ["a"])
        large_only = {"b": vm("b", "idle", "large"), "c": vm("c", "stopped", "large")}
        self.assertEqual(bt.rank(["b", "c"], large_only, "b", "default", lambda _: False), [])
        self.assertEqual(bt.rank(["b", "c"], large_only, None, "default", lambda _: False, cloned_for={"b": "default"}.get), ["b"])
        both = {"a": vm("a", "idle", "default"), "b": vm("b", "idle", "large")}
        self.assertEqual(bt.rank(["a", "b"], both, None, "default", lambda _: False, cloned_for={"b": "default"}.get), ["a", "b"])

    def test_the_running_vm_with_the_least_to_compile_again_comes_first(self):
        states = {vmid: vm(vmid, state) for vmid, state in [("a", "stopped"), ("b", "idle"), ("c", "idle"), ("d", "idle")]}
        rebuilds = {"a": (0, 0), "b": (60, 0), "c": (3, 0), "d": (60, 0)}.get
        self.assertEqual(bt.rank(sorted(states), states, "d", "large", lambda _: False, rebuilds=rebuilds), ["c", "d", "b", "a"])

class Plan(unittest.TestCase):
    """A workspace where top depends on middle, middle on base, and top's tests on probe."""

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        (self.root / "Cargo.toml").write_text('[workspace]\nmembers = ["crates/*"]\n[workspace.dependencies]\nbase = { path = "crates/base" }\nserde = "1"\n')
        crates = {
            "base": "",
            "middle": "[dependencies]\nbase.workspace = true\nserde.workspace = true\n",
            "top": '[dependencies]\nmid = { path = "../middle", package = "middle" }\n[dev-dependencies]\nprobe = { path = "../probe" }\n',
            "probe": "",
        }
        for name, body in crates.items():
            (self.root / "crates" / name).mkdir(parents=True)
            (self.root / "crates" / name / "Cargo.toml").write_text(f'[package]\nname = "{name}"\n{body}')
        self.manifest = {f"crates/{name}/src/lib.rs": name for name in crates} | {"Cargo.lock": "lock", "README.md": "readme"}

    def tearDown(self):
        self.temp.cleanup()

    def plan(self, *args, manifest=None):
        return bt.Plan(self.root, manifest or self.manifest, list(args))

    def built(self, plan, used=1.0):
        return {"used": used, **bt.left_trees({}, {plan.key: plan.compiled_tree({}, True)}, used)}

    def test_compiles_the_named_crates_with_their_dependencies_and_only_the_named_crates_tests(self):
        self.assertEqual(self.plan("build", "--release", "-p", "top").compiled, {"top", "middle", "base"})
        self.assertEqual(self.plan("check", "--all-targets", "-ptop").compiled, {"top", "middle", "base", "probe"})
        self.assertEqual(self.plan("check", "-p", "top", "--test", "dump").compiled, {"top", "middle", "base", "probe"})
        self.assertEqual(self.plan("check", "-p", "top", "--example=demo").compiled, {"top", "middle", "base", "probe"})
        self.assertEqual(self.plan("check", "--package=middle").compiled, {"middle", "base"})
        self.assertEqual(self.plan("check", "--workspace").compiled, {"top", "middle", "base", "probe"})

    def test_a_changed_crate_compiles_again_with_everything_depending_on_it(self):
        entry = self.built(self.plan("build", "--release", "-p", "top"))
        for changed, expected in [("base", 3), ("middle", 2), ("top", 1), ("probe", 0)]:
            plan = self.plan("build", "--release", "-p", "top", manifest={**self.manifest, f"crates/{changed}/src/lib.rs": "new"})
            self.assertEqual(plan.rebuilds(entry), (expected, 0), changed)
        self.assertEqual(self.plan("build", "--release", "-p", "top", manifest={**self.manifest, "README.md": "new"}).rebuilds(entry), (0, 0))
        self.assertEqual(self.plan("build", "--release", "-p", "top", manifest={**self.manifest, "Cargo.lock": "new"}).rebuilds(entry), (3, 0))

    def test_crates_the_cache_compiled_before_come_back_from_it(self):
        first = self.plan("build", "--release", "-p", "top")
        other = self.plan("build", "--release", "-p", "top", manifest={**self.manifest, "crates/base/src/lib.rs": "new"})
        entry = {"used": 1.0, **bt.left_trees({}, {first.key: first.compiled_tree({}, True)}, 1.0), first.cache: first.cached_after({})}
        entry = {**entry, "used": 2.0, **bt.left_trees(entry, {other.key: other.compiled_tree(entry, True)}, 2.0), other.cache: other.cached_after(entry)}
        self.assertEqual((first.rebuilds(entry), other.rebuilds(entry)), ((0, 3), (0, 0)))
        self.assertEqual(first.rebuilds({**entry, "used": 3.0}), (0, 3))
        self.assertEqual(self.plan("check", "-p", "top").rebuilds(entry), (3, 0))

    def test_only_tests_in_a_profile_without_incremental_sessions_count_on_the_cache(self):
        self.assertIsNone(self.plan("test", "--no-run", "-p", "top").cache)
        self.assertEqual(self.plan("test", "--no-run", "--release", "-p", "top").cache, "cache_tests_release")
        self.assertEqual(self.plan("build", "--release", "-p", "top").cache, "cache_build_release")

    def test_a_tree_stands_only_until_another_run_uses_the_vm_and_only_for_its_profile(self):
        release = self.plan("build", "--release", "-p", "top")
        entry = self.built(release)
        self.assertEqual(release.rebuilds({**entry, "used": 2.0}), (3, 0))
        self.assertEqual(self.plan("check", "-p", "top").rebuilds(entry), (3, 0))
        self.assertEqual(self.plan("build", "--profile", "release-lto", "-p", "top").rebuilds(entry), (3, 0))
        check = self.plan("check", "-p", "middle")
        after = {"used": 2.0, **bt.left_trees(entry, {check.key: check.compiled_tree(entry, True)}, 2.0)}
        self.assertEqual((release.rebuilds(after), check.rebuilds(after)), ((0, 0), (0, 0)))
        failed = {"used": 3.0, **bt.left_trees(after, {release.key: release.compiled_tree(after, False)}, 3.0)}
        self.assertEqual(release.rebuilds(failed), (3, 0))
        lost = {"used": 4.0, **bt.left_trees(after, {release.key: None}, 4.0)}
        self.assertEqual((release.rebuilds(lost), check.rebuilds(lost)), ((3, 0), (0, 0)))

    def test_a_feature_named_through_a_member_that_dev_depends_on_itself_reaches_the_build(self):
        crates = {"solo": '[features]\nx = []\ny = []\n[dev-dependencies]\nsolo = { path = ".", features = ["y"] }\n', "pair": "[features]\nx = []\n"}
        for name, body in crates.items():
            (self.root / "crates" / name).mkdir(parents=True)
            (self.root / "crates" / name / "Cargo.toml").write_text(f'[package]\nname = "{name}"\n{body}')
        cases = [
            (["-p", "solo", "--features", "solo/x,solo/y"], ["-p", "solo", "--features", "x,y"]),
            (["-p", "solo", "--features=solo/x", "-Fsolo/y", "-F", "solo/x"], ["-p", "solo", "--features=x", "-Fy", "-F", "x"]),
            (["-p", "solo", "-p", "middle", "--no-default-features", "--features", "solo/x"], ["-p", "solo", "-p", "middle", "--no-default-features", "--features", "x"]),
            (["-p", "solo", "--features", "x"], ["-p", "solo", "--features", "x"]),
            (["-p", "pair", "--features", "pair/x"], ["-p", "pair", "--features", "pair/x"]),
            (["-p", "top", "--features", "solo/x"], ["-p", "top", "--features", "solo/x"]),
        ]
        for args, expected in cases:
            with self.subTest(args=args):
                self.assertEqual(bt.member_features(self.root, ["build", "--release", *args]), ["build", "--release", *expected])
        with self.assertRaises(bt.Failure):
            bt.member_features(self.root, ["build", "--release", "-p", "solo", "-p", "pair", "--features", "solo/x"])

class Parse(unittest.TestCase):
    def test_check_and_build(self):
        self.assertEqual(bt.parse(["check", "-p", "viewer", "--all-targets"]), ("check", ["check", "-p", "viewer", "--all-targets"], None, None))
        mode, cargo, outdir, _ = bt.parse(["build", "--out", "x", "--release", "-p", "viewer"])
        self.assertEqual((mode, cargo, outdir), ("build", ["build", "--release", "-p", "viewer"], Path("x")))
        self.assertEqual(bt.parse(["snapshot"]), ("snapshot", [], None, None))
        self.assertEqual(bt.parse(["snapshot", "--from", "seed"]), ("snapshot", ["seed"], None, None))
        with self.assertRaises(bt.Failure):
            bt.parse(["snapshot", "--workspace"])
        self.assertEqual(bt.parse(["ui"]), ("ui", [], None, None))
        mode, cargo, outdir, _ = bt.parse(["build", "--out", "x", "--profile", "release-lto", "-p", "collector", "--lib"])
        self.assertEqual((mode, cargo, outdir, bt.profile(cargo)), ("build", ["build", "--profile", "release-lto", "-p", "collector", "--lib"], Path("x"), "release-lto"))

    def test_refuses_what_the_tool_owns(self):
        for args in (["check", "-j", "3"], ["check", "-j3"], ["check", "--jobs=2"], ["check", "--target", "x"], ["check", "--target-dir=x"], ["check", "--message-format=json"], ["build", "--out", "x", "-p", "viewer"], ["build", "--out", "x", "--profile", "dev"], ["build", "--out", "x", "--profile=test"], ["build", "--release"], ["ui", "x"], ["test", "-p", "x", "-j", "2"], ["test", "-p", "x", "--target", "y"], ["test", "-p", "x", "--message-format=json"], []):
            with self.subTest(args=args), self.assertRaises(bt.Failure):
                bt.parse(args)

    def test_test_takes_cargo_test_arguments_and_names_before_or_after_the_separator(self):
        mode, cargo, outdir, run = bt.parse(["test", "-p", "replay", "--features", "a,b", "--test", "codec", "first", "--out", "o", "--no-fail-fast", "--pdb", "--release", "--", "second", "--exact", "--nocapture"])
        self.assertEqual((mode, cargo, outdir), ("test", ["test", "--no-run", "-p", "replay", "--features", "a,b", "--test", "codec", "--release"], Path("o")))
        self.assertEqual((run.program, run.fail_fast, run.pdb), (["first", "second", "--exact", "--nocapture"], False, True))
        _, cargo, outdir, run = bt.parse(["test", "-preplay-columnar", "--lib", "name"])
        self.assertEqual((cargo, outdir, bt.profile(cargo), run.program, run.fail_fast, run.pdb), (["test", "--no-run", "-preplay-columnar", "--lib"], None, "dev", ["name"], True, False))
        self.assertEqual(bt.parse(["test", "-p", "x", "--", "--test-threads", "1", "-p"])[3].program, ["--test-threads", "1", "-p"])

    def test_test_refuses_by_name_what_cannot_run_here(self):
        for args, reason in ((["test"], "-p"), (["test", "name"], "-p"), (["test", "--workspace"], "suite"), (["test", "-p", "x", "--doc"], "rustdoc"), (["test", "-p", "x", "--no-run"], "check"), (["test", "-p", "x", "--out"], "directory")):
            with self.subTest(args=args), self.assertRaisesRegex(bt.Refused, reason):
                bt.parse(args)

    def test_machine_size_is_large_only_for_build_unless_overridden(self):
        with mock.patch.dict(os.environ, {}, clear=False):
            os.environ.pop("BOAT_MACHINE_TYPE", None)
            self.assertEqual(bt.machine_size("build"), "large")
            for mode in ("check", "clippy", "test", "ui", "snapshot"):
                self.assertEqual(bt.machine_size(mode), "default", mode)
        with mock.patch.dict(os.environ, {"BOAT_MACHINE_TYPE": "large"}):
            for mode in ("check", "clippy", "test", "ui", "snapshot", "build"):
                self.assertEqual(bt.machine_size(mode), "large", mode)
        with mock.patch.dict(os.environ, {"BOAT_MACHINE_TYPE": "default"}):
            self.assertEqual(bt.machine_size("build"), "default")
        with mock.patch.dict(os.environ, {"BOAT_MACHINE_TYPE": "huge"}), self.assertRaises(bt.Failure):
            bt.machine_size("check")

def remote_function(name):
    """A function of the VM's runner, whose module runs on import and only on Linux."""
    tree = ast.parse((Path(bt.TOOLS) / "boat_remote.py").read_text(encoding="utf-8"))
    node = next(node for node in tree.body if isinstance(node, ast.FunctionDef) and node.name == name)
    namespace = {"Path": Path, "SRC": Path("/home/user/hoi4/src")}
    exec(compile(ast.Module([node], []), "boat_remote.py", "exec"), namespace)
    return namespace[name]

def artifact(package, name, kind, test, executable):
    return {"reason": "compiler-artifact", "package_id": package, "target": {"name": name, "kind": kind}, "profile": {"test": test}, "executable": executable}

class TestPrograms(unittest.TestCase):
    """The runner's reading of Cargo's messages: which executable is which test program."""

    def test_boats_own_cargo_options_go_ahead_of_the_programs_arguments(self):
        command = remote_function("cargo_command")
        self.assertEqual(command(["clippy", "-p", "a", "--", "-W", "clippy::perf"], "t", False), ["cargo", "xwin", "clippy", "-p", "a", "--target", "t", "--", "-W", "clippy::perf"])
        self.assertEqual(command(["build", "-p", "a", "--"], "t", True), ["cargo", "xwin", "build", "-p", "a", "--target", "t", "--message-format=json-render-diagnostics", "--"])
        self.assertEqual(command(["test", "-p", "a"], "t", True), ["cargo", "xwin", "test", "-p", "a", "--target", "t", "--message-format=json-render-diagnostics"])

    def test_each_test_program_comes_with_its_package_and_only_named_binaries_come_too(self):
        deps = f"/home/user/hoi4/target/{bt.TARGET}/debug"
        viewer, other = "path+file:///home/user/hoi4/src/crates/viewer#0.1.0", "path+file:///home/user/hoi4/src/crates/viewer-core#0.2.0-rc.1"
        events = [
            {"reason": "build-script-executed", "package_id": viewer, "env": [["BATTLEPLAN_STAMP", "abc"]], "out_dir": "/x"},
            artifact(other, "viewer_core", ["lib"], False, None),
            artifact(viewer, "viewer", ["lib"], True, f"{deps}/deps/viewer-1a.exe"),
            artifact(viewer, "viewer", ["bin"], True, f"{deps}/deps/viewer-2b.exe"),
            artifact(viewer, "viewer", ["bin"], False, f"{deps}/viewer.exe"),
            artifact(viewer, "tool", ["bin"], False, f"{deps}/tool.exe"),
            artifact(viewer, "dump_stack", ["test"], True, f"{deps}/deps/dump_stack-3c.exe"),
            artifact(viewer, "demo", ["example"], False, f"{deps}/examples/demo.exe"),
            artifact(other, "viewer_core", ["lib"], True, f"{deps}/deps/viewer_core-4d.exe"),
            {"reason": "compiler-artifact", "package_id": viewer, "target": {"name": "build-script-build", "kind": ["custom-build"]}, "profile": {"test": False}, "executable": None},
        ]
        packages = {
            viewer: {"id": viewer, "name": "viewer", "version": "0.1.0", "authors": [], "manifest_path": "/home/user/hoi4/src/crates/viewer/Cargo.toml", "description": None},
            other: {"id": other, "name": "viewer-core", "version": "0.2.0-rc.1", "authors": ["a", "b"], "manifest_path": "/home/user/hoi4/src/crates/viewer-core/Cargo.toml", "license": "MIT"},
        }
        files, tests, binaries = remote_function("test_programs")(events, packages, {"viewer/viewer"}, False)
        self.assertEqual([(test["package"]["name"], test["target"], test["kind"], test["executable"]) for test in tests], [("viewer", "viewer", ["lib"], "viewer-1a.exe"), ("viewer", "viewer", ["bin"], "viewer-2b.exe"), ("viewer", "dump_stack", ["test"], "dump_stack-3c.exe"), ("viewer-core", "viewer_core", ["lib"], "viewer_core-4d.exe")])
        self.assertEqual(tests[0]["package"]["manifest"], "crates/viewer/Cargo.toml")
        self.assertEqual((tests[0]["package"]["env"], tests[3]["package"]["env"], tests[3]["package"]["license"]), ([["BATTLEPLAN_STAMP", "abc"]], [], "MIT"))
        self.assertEqual(binaries, [{"package": "viewer", "name": "viewer", "executable": "viewer.exe", "path": f"{deps}/viewer.exe"}])
        self.assertEqual(files, [f"{deps}/deps/viewer-1a.exe", f"{deps}/deps/viewer-2b.exe", f"{deps}/deps/dump_stack-3c.exe", f"{deps}/deps/viewer_core-4d.exe", f"{deps}/viewer.exe"])

def program(name="replay", kind=("lib",), target="replay", executable="replay-1a.exe", env=(), version="1.2.3"):
    package = {"name": name, "version": version, "authors": ["a", "b"], "manifest": f"crates/{name}/Cargo.toml", "env": [list(pair) for pair in env], "description": None, "rust_version": "1.80"}
    return {"package": package, "target": target, "kind": list(kind), "executable": executable}

class TestEnvironment(unittest.TestCase):
    def test_a_program_gets_what_cargo_test_sets_with_this_machines_paths(self):
        worktree, folder = Path("C:/w"), Path("C:/w/.cargo-target/boat-test")
        base = {"Path": "C:/bin", "BATTLEPLAN_CAPTURES": "D:/c", "CARGO_PKG_NAME": "stale"}
        binaries = [{"package": "viewer", "name": "viewer", "executable": "viewer.exe"}, {"package": "other", "name": "x", "executable": "x.exe"}]
        unit = bt.test_environment(base, worktree, folder, program("viewer", env=[("BATTLEPLAN_STAMP", "abc")], version="0.4.1-rc.2+build.5"), binaries)
        keys = ("CARGO_MANIFEST_DIR", "CARGO_MANIFEST_PATH", "CARGO_PKG_NAME", "CARGO_PKG_VERSION", "CARGO_PKG_VERSION_MAJOR", "CARGO_PKG_VERSION_MINOR", "CARGO_PKG_VERSION_PATCH", "CARGO_PKG_VERSION_PRE", "CARGO_PKG_AUTHORS", "CARGO_PKG_DESCRIPTION", "CARGO_PKG_RUST_VERSION", "BATTLEPLAN_STAMP", "BATTLEPLAN_CAPTURES", "Path")
        self.assertEqual({key: unit[key] for key in keys}, {
            "CARGO_MANIFEST_DIR": str(worktree / "crates/viewer"), "CARGO_MANIFEST_PATH": str(worktree / "crates/viewer/Cargo.toml"), "CARGO_PKG_NAME": "viewer", "CARGO_PKG_VERSION": "0.4.1-rc.2+build.5",
            "CARGO_PKG_VERSION_MAJOR": "0", "CARGO_PKG_VERSION_MINOR": "4", "CARGO_PKG_VERSION_PATCH": "1", "CARGO_PKG_VERSION_PRE": "rc.2", "CARGO_PKG_AUTHORS": "a:b", "CARGO_PKG_DESCRIPTION": "",
            "CARGO_PKG_RUST_VERSION": "1.80", "BATTLEPLAN_STAMP": "abc", "BATTLEPLAN_CAPTURES": "D:/c", "Path": str(folder) + os.pathsep + "C:/bin"})
        self.assertNotIn("PATH", unit)
        self.assertFalse({"CARGO_BIN_EXE_viewer", "CARGO_TARGET_TMPDIR"} & unit.keys())
        integration = bt.test_environment(base, worktree, folder, program("viewer", ("test",), "dump_stack"), binaries)
        self.assertEqual((integration["CARGO_BIN_EXE_viewer"], integration["CARGO_TARGET_TMPDIR"]), (str(folder / "viewer.exe"), str(folder / "tmp")))
        self.assertNotIn("CARGO_BIN_EXE_x", integration)

class TestSources(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        (self.root / "Cargo.toml").write_text('[workspace]\nmembers = ["crates/*"]\n')
        files = {
            "crates/app/Cargo.toml": '[package]\nname = "app"\n',
            "crates/app/tests/run.rs": 'fn a() { Command::new(env!("CARGO_BIN_EXE_app-cli")); }\nconst B: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/x"));\n',
            "crates/app/src/lib.rs": 'fn f() { let root = env!("CARGO_MANIFEST_DIR"); let o = env!("OUT_DIR"); }\n',
            "crates/app/src/fixed.rs": 'fn g() { std::env::var_os("CARGO_MANIFEST_DIR").unwrap_or(env!("CARGO_MANIFEST_DIR").into()); }\n',
            "crates/app/src/bin/cli.rs": 'fn main() { env!("CARGO_MANIFEST_DIR"); }\n',
            "crates/app/build.rs": 'fn main() { env!("CARGO_MANIFEST_DIR"); }\n',
            "crates/other/Cargo.toml": '[package]\nname = "other"\n',
            "crates/other/tests/t.rs": 'fn h() { env!("CARGO_BIN_EXE_other"); }\n',
        }
        for name, text in files.items():
            (self.root / name).parent.mkdir(parents=True, exist_ok=True)
            (self.root / name).write_text(text)

    def tearDown(self):
        self.temp.cleanup()

    def test_binaries_named_by_a_selected_crates_code_come_with_its_tests(self):
        self.assertEqual(bt.named_binaries(self.root, ["test", "--no-run", "-p", "app"]), ["app/app-cli"])
        self.assertEqual(bt.named_binaries(self.root, ["test", "--no-run", "-p", "app", "-p", "other"]), ["app/app-cli", "other/other"])

    def test_paths_compiled_in_without_a_run_time_reading_are_named(self):
        self.assertEqual(bt.compiled_paths(self.root, ["test", "--no-run", "-p", "app"]), ['crates/app/src/lib.rs:1: env!("CARGO_MANIFEST_DIR")', 'crates/app/src/lib.rs:1: env!("OUT_DIR")', 'crates/app/tests/run.rs:1: env!("CARGO_BIN_EXE_app-cli")'])

    def test_a_named_out_folder_is_replaced_only_when_a_test_run_left_it(self):
        mine, theirs, empty = self.root / "mine", self.root / "theirs", self.root / "empty"
        for folder in (mine, theirs, empty):
            folder.mkdir()
        (mine / "boat-artifacts.json").write_text(json.dumps({"cargo_arguments": ["test", "--no-run"]}))
        (theirs / "viewer.exe").write_text("")
        (mine / "old-1a.exe").write_text("")
        claimed, lock = bt.test_folder(self.root, mine)
        self.assertEqual(claimed, mine.resolve())
        with self.assertRaisesRegex(bt.Refused, "in use"):
            bt.test_folder(self.root, mine)
        bt.replace_folder(mine)
        self.assertEqual([entry.name for entry in mine.iterdir()], [bt.TEST_LOCK])
        lock.close()
        bt.test_folder(self.root, mine)[1].close()
        bt.test_folder(self.root, empty)[1].close()
        with self.assertRaisesRegex(bt.Refused, "no earlier"):
            bt.test_folder(self.root, theirs)

    def test_each_run_has_a_folder_of_its_own_and_removes_only_ended_runs_folders(self):
        base = self.root / ".cargo-target" / "boat-test"
        live, live_lock = bt.test_folder(self.root, None)
        ended, ended_lock = bt.test_folder(self.root, None)
        (live / "a.exe").write_text("")
        (ended / "b.exe").write_text("")
        ended_lock.close()
        base.joinpath("stray.exe").write_text("")
        third, third_lock = bt.test_folder(self.root, None)
        self.assertEqual({live.parent, third.parent}, {base.resolve()})
        self.assertEqual(sorted(path.name for path in base.iterdir()), sorted([live.name, third.name]))
        self.assertTrue((live / "a.exe").exists())
        live_lock.close()
        third_lock.close()

class TestRuns(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.folder = Path(self.temp.name)
        self.saved = bt.run_program
        self.said = []
        self.ran = []

    def tearDown(self):
        bt.run_program = self.saved
        self.temp.cleanup()

    def run_with(self, results, args=("name",), fail_fast=True):
        programs = [program("b", ("test",), "z", "z.exe"), program("b", ("lib",), "b", "b.exe"), program("a", ("test",), "t", "t.exe"), program("b", ("bin",), "b", "bb.exe")]

        def run_program(executable, program_args, cwd, env):
            self.ran.append((executable.name, cwd.name, env["CARGO_PKG_NAME"], program_args))
            return results[executable.name]

        bt.run_program = run_program
        say, bt.say = bt.say, self.said.append
        placed, bt.place_shader_compiler = bt.place_shader_compiler, lambda worktree, folder: self.said.append(f"placed {folder}")
        try:
            return bt.run_tests(Path("C:/w"), self.folder, {"tests": programs}, bt.TestRun(list(args), fail_fast), 0)
        finally:
            bt.say = say
            bt.place_shader_compiler = placed

    def test_the_programs_get_the_pinned_shader_compiler_beside_them_from_the_worktrees_copy(self):
        worktree = self.folder / "worktree"
        copy = worktree / ".cargo-target" / "shader-compiler"
        copy.mkdir(parents=True)
        for name in bt.SHADER_COMPILER:
            (copy / name).write_text(name)
        programs = self.folder / "programs"
        programs.mkdir()
        with mock.patch.object(bt.subprocess, "run") as installed:
            bt.place_shader_compiler(worktree, programs)
        installed.assert_not_called()
        self.assertEqual({name: (programs / name).read_text() for name in bt.SHADER_COMPILER}, {name: name for name in bt.SHADER_COMPILER})
        self.assertEqual(self.run_with({name: (0, 1, 0) for name in ("t.exe", "b.exe", "bb.exe", "z.exe")}), 0)
        self.assertIn(f"placed {self.folder}", self.said)

    def test_programs_run_in_cargos_order_and_the_worst_exit_wins(self):
        results = {"t.exe": (0, 1, 3), "b.exe": (0, 0, 2), "bb.exe": (101, 1, 0), "z.exe": (0xC0000005, None, None)}
        self.assertEqual(self.run_with(results, fail_fast=False), 0xC0000005 - 2 ** 32)
        self.assertEqual(self.ran, [("t.exe", "a", "a", ["name"]), ("b.exe", "b", "b", ["name"]), ("bb.exe", "b", "b", ["name"]), ("z.exe", "b", "b", ["name"])])
        self.assertIn(f"Test program: b bin b: {self.folder / 'bb.exe'}: exit 101", self.said)
        self.assertIn(f"Test program: b test z: {self.folder / 'z.exe'}: exit 0xc0000005", self.said)

    def test_the_first_failing_program_stops_the_run_as_in_cargo(self):
        results = {"t.exe": (101, 1, 0), "b.exe": (0, 1, 0), "bb.exe": (0, 1, 0), "z.exe": (0, 1, 0)}
        self.assertEqual(self.run_with(results), 101)
        self.assertEqual([name for name, *_ in self.ran], ["t.exe"])
        self.assertTrue(any("3 not run" in line for line in self.said))

    def test_names_that_match_nothing_fail_where_cargo_would_pass(self):
        nothing = {name: (0, 0, 4) for name in ("t.exe", "b.exe", "bb.exe", "z.exe")}
        self.assertEqual(self.run_with(nothing), 1)
        self.assertTrue(any(line.startswith("No test matched name: 16 filtered out in 4 programs") for line in self.said))
        self.assertEqual(self.run_with(nothing, args=("name", "--list")), 0)
        self.said.clear()
        self.assertEqual(self.run_with({name: (0, 0, 0) for name in nothing}, args=()), 0)
        self.assertIn("No tests ran: the selected programs hold none", self.said)

    def test_a_program_streams_its_output_and_its_counts_are_read(self):
        script = "import sys; print('running 2 tests'); print('test a ... ok', file=sys.stderr); print('test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 0.00s'); sys.exit(3)"
        out = io.BytesIO()
        wrapper = io.TextIOWrapper(out)
        with contextlib.redirect_stdout(wrapper):
            result = self.saved(Path(sys.executable), ["-c", script], self.folder, dict(os.environ))
        wrapper.flush()
        self.assertEqual(result, (3, 2, 5))
        self.assertIn(b"test a ... ok", out.getvalue())

class Reconcile(Harness):
    def test_shortens_only_long_lived_vms_no_run_holds(self):
        fake = self.use(FakeBoat([vm("free", ttl=14400), vm("short"), vm("held", ttl=14400), vm("stopped", "stopped", ttl=14400), vm("busy", "running", ttl=14400)]))
        fake.vms["orphan"] = vm("orphan", "running", ttl=14400)
        bt.atomic_json(self.state.claims / "orphan.quarantined.json", {"id": "orphan"})
        with self.state.lock("held"):
            bt.reconcile(self.state, bt.listing())
        self.assertEqual(fake.ttl_calls("free"), [bt.IDLE_SECONDS])
        self.assertEqual(fake.ttl_calls("orphan"), [bt.IDLE_SECONDS])
        self.assertEqual(fake.ttl_calls("busy"), [bt.IDLE_SECONDS])
        for vmid in ("short", "held", "stopped"):
            self.assertEqual(fake.ttl_calls(vmid), [], vmid)

class Acquire(Harness):
    def test_a_vm_that_does_not_start_or_stops_hydrating_is_cooled_and_passed_by(self):
        fake = self.use(FakeBoat([vm("a", "stopped"), vm("b", "stopped", hydrated=False), vm("c", "stopped", hydrated=False)]))
        fake.starting["a"] = 10 ** 6
        fake.starting["c"] = fake.hydrating["c"] = bt.STALL_SECONDS // bt.POLL_SECONDS + 5
        record = {}
        vmid, lock, lease, _ = bt.acquire(self.state, "lane", "large", "t", record)
        lease.end()
        lock.close()
        self.assertEqual(vmid, "c")
        self.assertEqual([item["vm"] for item in record["passed_over"]], ["a", "b"])
        self.assertIn("not running", record["passed_over"][0]["reason"])
        self.assertIn("stalled", record["passed_over"][1]["reason"])
        for vmid in ("a", "b"):
            self.assertTrue(self.state.cooling(vmid, 0))
            self.assertEqual(fake.ttl_calls(vmid)[-1], bt.IDLE_SECONDS)
            self.state.lock(vmid).close()
        self.assertTrue(all("--no-wait" in call for call in fake.calls if call[0] == "resume"))

    def test_hydration_spends_its_stages_by_name_and_counts_its_entries(self):
        phases = iter([("starting", {"phase": "folding"}), ("idle", {"phase": "hydrating", "percent": 40, "entriesTotal": 9}), ("idle", {"phase": "hydrating", "percent": 95, "entriesTotal": 9}), ("idle", None)])
        bt.info = lambda vmid: (lambda state, hydration: {"state": state, "hydration": hydration, "hydrated": hydration is None})(*next(phases))
        spent, pauses = {}, []
        self.saved["wait_ready"]("a", spent, clock=lambda: self.now, pause=lambda seconds: (pauses.append(seconds), setattr(self, "now", self.now + seconds)))
        self.assertEqual(spent, {"boot": 0, "folding": bt.POLL_SECONDS, "hydrating": bt.POLL_SECONDS + bt.FINAL_POLL_SECONDS, "entries": 9})
        self.assertEqual(pauses, [bt.POLL_SECONDS, bt.POLL_SECONDS, bt.FINAL_POLL_SECONDS])

    def test_hydration_that_crawls_fails_as_a_stall_while_one_that_changes_phase_does_not(self):
        crawl = iter(range(10 ** 6))
        bt.info = lambda vmid: {"state": "idle", "hydrated": False, "hydration": {"phase": "hydrating", "entriesDone": 10 * next(crawl), "entriesTotal": 100000}}
        with self.assertRaisesRegex(bt.Failure, "stalled: .* entries in 180 s, under 0.5% of 100000, in its hydrating phase"):
            self.saved["wait_ready"]("a", clock=lambda: self.now, pause=lambda seconds: setattr(self, "now", self.now + seconds))
        self.assertLessEqual(self.now, bt.STALL_SECONDS + bt.POLL_SECONDS)
        phases = iter(["sysdelta", "hydrating", "degraded", "reconciling", None])
        bt.info = lambda vmid: (lambda phase: {"state": "idle", "hydrated": phase is None, "hydration": {"phase": phase, "entriesDone": 5, "entriesTotal": 100000}})(next(phases))
        self.now = 0.0
        self.saved["wait_ready"]("a", clock=lambda: self.now, pause=lambda seconds: setattr(self, "now", self.now + bt.STALL_SECONDS - 1))

    def test_a_vm_woken_before_the_run_is_taken_while_it_starts_and_waited_for(self):
        fake = self.use(FakeBoat([vm("a", "starting"), vm("b", "stopped")]))
        fake.starting["a"] = 2
        self.state.update("a", woken=time.time() - 30)
        record = {}
        vmid, lock, lease, _ = bt.acquire(self.state, "lane", "large", "t", record)
        lease.end()
        lock.close()
        self.assertEqual(vmid, "a")
        self.assertNotIn("resume", [call[0] for call in fake.calls])
        self.assertGreaterEqual(record["woken"], 30)
        self.assertIsNone(self.state.entry("a")["woken"])
        self.assertNotIn("resumed", record)
        self.assertIn("boot", record["readiness"])

    def test_a_refused_resume_passes_the_vm_by_without_cooling_it(self):
        fake = self.use(FakeBoat([vm("a", "stopped"), vm("b", "stopped")]))
        fake.refused.add("a")
        record = {}
        vmid, lock, lease, _ = bt.acquire(self.state, "lane", "large", "t", record)
        lease.end()
        lock.close()
        self.assertEqual((vmid, [item["vm"] for item in record["passed_over"]]), ("b", ["a"]))
        self.assertFalse(self.state.cooling("a", 0))

    def test_an_image_too_big_for_the_size_asked_is_cloned_large_and_a_refusal_names_boats_reason(self):
        fake = self.use(FakeBoat([]))
        fake.too_small = {"default"}
        record = {}
        held = []
        create = fake.boat

        def boat(*args, **kwargs):
            if args[0] == "new":
                try:
                    self.state.waking("lane").close()
                    held.append(False)
                except OSError:
                    held.append(True)
            return create(*args, **kwargs)

        bt.boat = boat
        with mock.patch.object(bt.time, "sleep") as sleep, mock.patch.object(bt.time, "time", lambda: 1000.0):
            vmid, lock, lease, _ = bt.acquire(self.state, "lane", "default", "t", record)
            lease.end()
            lock.close()
            self.assertEqual((vmid, record["vm_type"], record["created"]), ("n", "large", True))
            self.assertEqual(self.state.entry("n")["cloned_for"], "default")
            self.assertEqual([call[call.index("--type") + 1] for call in fake.calls if call[0] == "new"], ["default", "large"])
            self.assertEqual(held, [True, True])
            self.assertEqual(len(sleep.call_args_list), 1)
            self.assertAlmostEqual(sleep.call_args.args[0], bt.CREATE_SPACING_SECONDS, places=6)
            said, reuse = [], {}
            fake.vms["b"] = vm("b", "ready", "large")
            bt.atomic_json(self.state.pool / "b.json", {"id": "b", "snapshot": bt.SNAPSHOT, "cloned_for": "default", "used": 0})
            self.state.update("n", used=1)
            fake.acquire_status["b"] = bt.REMOTE_CLAIMED
            fake.acquire_output["b"] = "busy\n"
            with mock.patch.object(bt, "say", side_effect=said.append):
                vmid2, lock2, lease2, _ = bt.acquire(self.state, "lane", "default", "t2", reuse)
            lease2.end()
            lock2.close()
            self.assertEqual((vmid2, reuse["vm_type"], reuse["created"]), ("n", "large", False))
            self.assertEqual(sum(call[0] == "new" for call in fake.calls), 2)
            notices = [line for line in said if "does not fit a default VM" in line]
            self.assertEqual(len(notices), 1, said)
            self.assertIn("large VM n", notices[0])
            del fake.vms["b"]
            self.assertEqual(bt.wake(self.state, "lane", "default"), ("n", "ready"))
            self.assertEqual(sum(call[0] == "new" for call in fake.calls), 2)
            fake = self.use(FakeBoat([]))
            fake.too_small = {"default", "large"}
            with self.assertRaisesRegex(bt.Failure, "as a large VM .*does not fit any available 'large' machine"):
                bt.acquire(self.state, "lane", "default", "t", {})

    def test_resume_never_shrinks_a_large_vm(self):
        fake = self.use(FakeBoat([vm("a", "stopped", "large")]))
        self.state.update("a", cloned_for="default")
        vmid, lock, lease, _ = bt.acquire(self.state, "lane", "default", "t", {})
        lease.end()
        lock.close()
        self.assertEqual(vmid, "a")
        resume = next(call for call in fake.calls if call[0] == "resume")
        self.assertNotIn("--type", resume)
        self.assertEqual(fake.vms["a"]["type"], "large")
        fake = self.use(FakeBoat([vm("b", "stopped", "large")]))
        vmid, lock, lease, _ = bt.acquire(self.state, "lane", "default", "t2", {})
        lease.end()
        lock.close()
        self.assertEqual((vmid, fake.vms["n"]["type"]), ("n", "default"))
        self.assertNotIn("resume", [call[0] for call in fake.calls])

    def test_a_vm_holding_another_claim_is_quarantined_and_released(self):
        fake = self.use(FakeBoat([vm("a"), vm("b")]))
        fake.acquire_status["a"] = bt.REMOTE_CLAIMED
        record = {}
        vmid, lock, lease, _ = bt.acquire(self.state, "lane", "large", "t", record)
        lease.end()
        lock.close()
        self.assertEqual(vmid, "b")
        self.assertEqual(record["vm"], "b")
        self.assertEqual(self.state.quarantined(), ["a"])
        self.assertEqual(self.state.registered(), ["b"])
        self.assertEqual(fake.ttl_calls("a"), [bt.LEASE_SECONDS, bt.IDLE_SECONDS])

    def test_a_vm_whose_toolchain_does_not_run_is_quarantined_and_one_that_fails_to_initialize_is_cooled(self):
        fake = self.use(FakeBoat([vm("a"), vm("b"), vm("c")]))
        for vmid in "abc":
            self.state.update(vmid, used={"a": 0, "b": 1, "c": 2}[vmid])
        fake.acquire_status.update(a=bt.BROKEN_TOOLCHAIN, b=127)
        record = {}
        vmid, lock, lease, _ = bt.acquire(self.state, "lane", "large", "t", record)
        lease.end()
        lock.close()
        self.assertEqual(vmid, "c")
        self.assertEqual([item["vm"] for item in record["passed_over"]], ["a", "b"])
        self.assertTrue(all("claim" in item["readiness"] for item in record["passed_over"]))
        self.assertEqual((self.state.quarantined(), self.state.registered()), (["a"], ["b", "c"]))
        self.assertTrue(self.state.cooling("b", time.time()))
        self.assertEqual([call[1:] for call in fake.remote_calls if call[1] == "release"], [("release", "t")])

    def test_a_woken_vm_passed_by_keeps_its_wake_lead(self):
        fake = self.use(FakeBoat([vm("a"), vm("b")]))
        self.state.update("a", woken=time.time() - 30, used=0)
        self.state.update("b", used=1)
        fake.acquire_status["a"] = 127
        record = {}
        vmid, lock, lease, _ = bt.acquire(self.state, "lane", "large", "t", record)
        lease.end()
        lock.close()
        self.assertEqual(vmid, "b")
        self.assertGreaterEqual(record["passed_over"][0]["woken"], 30)
        self.assertNotIn("woken", record)

    def test_a_claim_step_that_fails_or_answers_garbage_passes_the_vm_by(self):
        fake = self.use(FakeBoat([vm("a"), vm("b"), vm("c")]))
        for index, vmid in enumerate("abc"):
            self.state.update(vmid, used=index)
        fake.acquire_output["b"] = "null"
        install = bt.install_runner

        def install_runner(state, vmid):
            if vmid == "a":
                raise bt.Failure("Boat exec failed (1): upload lost")
            install(state, vmid)

        record = {}
        with mock.patch.object(bt, "install_runner", install_runner):
            vmid, lock, lease, _ = bt.acquire(self.state, "lane", "large", "t", record)
        lease.end()
        lock.close()
        self.assertEqual(vmid, "c")
        self.assertEqual([item["vm"] for item in record["passed_over"]], ["a", "b"])
        self.assertIn("upload lost", record["passed_over"][0]["reason"])
        self.assertTrue(all("claim" in item["readiness"] for item in record["passed_over"]))
        self.assertTrue(self.state.cooling("a", time.time()) and self.state.cooling("b", time.time()))
        self.assertEqual([call[0] for call in fake.remote_calls if call[1] == "release"], ["a", "b"])

    def test_an_image_whose_clones_cannot_build_is_cloned_three_times_at_most(self):
        fake = self.use(FakeBoat([]))
        fake.acquire_status["n"] = bt.BROKEN_TOOLCHAIN
        with mock.patch.object(bt.time, "sleep") as sleep, mock.patch.object(bt.time, "time", lambda: 1000.0), self.assertRaisesRegex(bt.Failure, "three attempts"):
            bt.acquire(self.state, "lane", "large", "t", {})
        self.assertEqual(sum(call[0] == "new" for call in fake.calls), 3)
        self.assertEqual(len(sleep.call_args_list), 2)

    def test_a_clone_its_wake_made_counts_like_one_the_run_made(self):
        fake = self.use(FakeBoat([]))
        fake.acquire_status["n"] = bt.BROKEN_TOOLCHAIN
        with mock.patch.object(bt.time, "sleep"), mock.patch.object(bt.time, "time", lambda: 1000.0):
            self.assertEqual(bt.wake(self.state, "lane", "large"), ("n", "cloned"))
            with self.assertRaisesRegex(bt.Failure, "three attempts"):
                bt.acquire(self.state, "lane", "large", "t", {})
        self.assertEqual(sum(call[0] == "new" for call in fake.calls), 3)

    def test_quarantined_vms_do_not_use_up_the_attempts(self):
        fake = self.use(FakeBoat([vm(vmid) for vmid in "abcd"]))
        for index, vmid in enumerate("abcd"):
            self.state.update(vmid, used=index)
        fake.acquire_status.update(a=bt.BROKEN_TOOLCHAIN, b=bt.BROKEN_TOOLCHAIN, c=bt.BROKEN_TOOLCHAIN)
        vmid, lock, lease, _ = bt.acquire(self.state, "lane", "large", "t", {})
        lease.end()
        lock.close()
        self.assertEqual((vmid, self.state.quarantined()), ("d", ["a", "b", "c"]))

    def test_a_vm_still_finishing_a_job_is_passed_by_and_stays_in_the_pool(self):
        fake = self.use(FakeBoat([vm("a"), vm("b")]))
        fake.acquire_status["a"] = bt.REMOTE_CLAIMED
        fake.acquire_output["a"] = "busy\n"
        record = {}
        vmid, lock, lease, _ = bt.acquire(self.state, "lane", "large", "t", record)
        lease.end()
        lock.close()
        self.assertEqual(vmid, "b")
        self.assertEqual([(item["vm"], "claim" in item["readiness"]) for item in record["passed_over"]], [("a", True)])
        self.assertEqual(self.state.quarantined(), [])
        self.assertEqual(self.state.registered(), ["a", "b"])

class Wake(Harness):
    def test_resumes_the_vm_the_run_would_take_without_claiming_or_waiting(self):
        fake = self.use(FakeBoat([vm("a", "stopped"), vm("b", "stopped"), vm("c", "stopped")]))
        rebuilds = {"a": (5, 0), "b": (0, 0), "c": (0, 0)}.get
        with self.state.lock("b"):
            self.assertEqual(bt.wake(self.state, "lane", "large", rebuilds), ("c", "resuming"))
        resume = next(call for call in fake.calls if call[0] == "resume")
        self.assertEqual(resume[:4], ("resume", "c", "--ttl", str(bt.LEASE_SECONDS)))
        self.assertEqual(fake.single, ["resume"])
        self.assertIn("--no-wait", resume)
        self.assertNotIn("info", [call[0] for call in fake.calls])
        self.assertEqual(fake.remote_calls, [])
        self.assertGreater(self.state.entry("c")["woken"], 0)
        self.state.lock("c").close()

    def test_clones_the_vm_the_run_would_clone_and_leaves_it_free(self):
        fake = self.use(FakeBoat([vm("a", "stopping")]))
        clock = [1000.0]
        bt.atomic_json(self.state.claims / "create-schedule.json", {"next": 1120.0})
        with mock.patch.object(bt.time, "time", lambda: clock[0]), mock.patch.object(bt.time, "sleep", lambda seconds: clock.__setitem__(0, clock[0] + seconds)):
            self.assertEqual(bt.wake(self.state, "lane", "default"), ("n", "cloned"))
        self.assertEqual(self.state.entry("n")["woken"], 1120.0)
        self.state.lock("n").close()
        self.state.allocator().close()
        self.assertEqual(sum(call[0] == "new" for call in fake.calls), 1)

    def test_a_run_that_would_clone_waits_for_the_vm_its_wake_is_starting(self):
        fake = self.use(FakeBoat([]))
        waking = self.state.waking("lane")

        waited = []

        def finish(seconds):
            waited.append(seconds)
            fake.vms["w"] = vm("w", hydrated=False)
            bt.atomic_json(self.state.pool / "w.json", {"id": "w", "snapshot": bt.SNAPSHOT})
            waking.close()

        with mock.patch.object(bt.time, "sleep", finish):
            vmid, _, lock, created = bt.choose(self.state, "lane", "large")
        lock.close()
        self.assertEqual(len(waited), 1)
        self.assertEqual((vmid, created), ("w", False))
        self.assertNotIn("new", [call[0] for call in fake.calls])

    def test_a_wake_after_its_worktrees_run_began_starts_nothing(self):
        fake = self.use(FakeBoat([vm("a", "stopped")]))
        first, second = self.state.running("lane", "1"), self.state.running("lane", "2")
        first.close()
        self.assertEqual(bt.wake(self.state, "lane", "large"), (None, "a run of this worktree is under way"))
        second.close()
        self.assertEqual(fake.calls, [])
        self.assertFalse(self.state.run_active("lane"))
        self.assertEqual(list(self.state.lanes.glob("lane.running-*")), [])

    def test_a_wake_whose_worktree_ran_since_it_began_starts_nothing(self):
        fake = self.use(FakeBoat([vm("a")]))
        bt.atomic_json(self.state.lanes / "lane.ended.json", {"at": 2000.0})
        self.state.update("a", used=5000.0)
        self.assertEqual(bt.wake(self.state, "lane", "large", begun=1000.0), (None, "a run of this worktree ended since the wake began"))
        self.assertEqual(bt.wake(self.state, "lane", "large", begun=3000.0), ("a", "idle"))
        self.assertEqual(fake.ttl_calls("a"), [bt.LEASE_SECONDS])
        self.assertEqual(fake.single, ["extend"])

    def test_a_run_takes_its_marker_from_a_check_holding_it_for_a_moment(self):
        held = bt.Lock(self.state.lanes / "lane.running-t")
        with mock.patch.object(bt.time, "sleep", lambda seconds: held.close()) :
            self.state.running("lane", "t").close()

    def test_a_run_about_to_resume_a_vm_waits_for_its_worktrees_wake(self):
        fake = self.use(FakeBoat([vm("s", "stopped")]))
        waking = self.state.waking("lane")

        def finish(seconds):
            fake.vms["w"] = vm("w", hydrated=False)
            bt.atomic_json(self.state.pool / "w.json", {"id": "w", "snapshot": bt.SNAPSHOT})
            waking.close()

        with mock.patch.object(bt.time, "sleep", finish):
            vmid, _, lock, created = bt.choose(self.state, "lane", "large")
        lock.close()
        self.assertEqual((vmid, created), ("w", False))

    def test_a_refused_resume_wakes_the_next_vm(self):
        fake = self.use(FakeBoat([vm("a", "stopped"), vm("b", "stopped")]))
        self.state.update("a", used=0)
        self.state.update("b", used=1)
        fake.refused.add("a")
        self.assertEqual(bt.wake(self.state, "lane", "large"), ("b", "resuming"))

    def test_a_refused_extend_wakes_the_next_vm(self):
        fake = self.use(FakeBoat([vm("a"), vm("b", "stopped")]))
        extend = fake.boat

        def boat(*args, **kwargs):
            if args[0] == "extend" and args[1] == "a":
                raise bt.Failure("Boat extend failed (1): refused")
            return extend(*args, **kwargs)

        bt.boat = boat
        self.assertEqual(bt.wake(self.state, "lane", "large"), ("b", "resuming"))

    def test_leaves_alone_a_vm_that_already_runs_or_starts(self):
        for state in ("idle", "starting"):
            fake = self.use(FakeBoat([vm("a", state), vm("b", "stopped")]))
            self.assertEqual(bt.wake(self.state, "lane", "large"), ("a", state))
            self.assertNotIn("resume", [call[0] for call in fake.calls])
            self.assertEqual(fake.ttl_calls("a"), [bt.LEASE_SECONDS])

    def test_the_command_takes_the_arguments_of_the_run_that_follows(self):
        fake = self.use(FakeBoat([vm("a", "stopped", "default")]))
        repo = self.root / "repo"
        (repo / "crates" / "x").mkdir(parents=True)
        (repo / "Cargo.toml").write_text('[workspace]\nmembers = ["crates/*"]\n')
        (repo / "crates" / "x" / "Cargo.toml").write_text('[package]\nname = "x"\n')
        subprocess.run(["git", "init", "-q", str(repo)], check=True)
        cwd = os.getcwd()
        os.chdir(repo)
        os.environ["BOAT_STATE_DIR"] = str(self.state.root)
        try:
            self.assertEqual(bt.main(["wake", "check", "--all-targets", "-p", "x"]), 0)
            self.assertEqual(bt.main(["wake", "snapshot"]), 2)
            self.assertEqual(bt.main(["wake", "check", "-j", "4"]), 2)
            with mock.patch.dict(os.environ, {"BOAT_MACHINE_TYPE": "larg"}):
                self.assertEqual(bt.main(["wake", "ui"]), 2)
        finally:
            os.chdir(cwd)
            del os.environ["BOAT_STATE_DIR"]
        self.assertEqual([call[1] for call in fake.calls if call[0] == "resume"], ["a"])
        self.assertFalse((self.state.logs / "timings.jsonl").exists())

class ToolchainChannel(unittest.TestCase):
    def test_the_channel_is_read_and_a_file_that_cannot_be_read_names_none(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "rust-toolchain.toml"
            path.write_text('[toolchain]\nchannel = "1.96.0"\n', encoding="utf-8")
            self.assertEqual(bt.toolchain_channel(Path(folder)), "1.96.0")
            path.write_text('[toolchain]\nchannel = "1.96.0"\n', encoding="utf-16")
            self.assertEqual(bt.toolchain_channel(Path(folder)), "")
            path.unlink()
            self.assertEqual(bt.toolchain_channel(Path(folder)), "")

class AtomicJson(unittest.TestCase):
    def test_a_replace_a_reader_blocks_for_a_moment_still_lands(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "entry.json"
            replace, refusals = Path.replace, [PermissionError(5, "Access is denied")] * 2

            def blocked(self, target):
                if refusals:
                    raise refusals.pop()
                return replace(self, target)

            with mock.patch.object(Path, "replace", blocked), mock.patch.object(bt.time, "sleep") as sleep:
                bt.atomic_json(path, {"a": 1})
            self.assertEqual((json.loads(path.read_text()), len(sleep.call_args_list)), ({"a": 1}, 2))

class RemoteToolchain(unittest.TestCase):
    """The claim's Cargo probe, from boat_remote.py's own definitions."""

    def setUp(self):
        text = (bt.TOOLS / "boat_remote.py").read_text(encoding="utf-8")
        self.remote = {"__name__": "boat_remote_definitions"}
        with mock.patch.dict(sys.modules, {"fcntl": types.ModuleType("fcntl")}):
            exec(compile(text.split("\nmode = sys.argv[1]\n")[0], "boat_remote.py", "exec"), self.remote)
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        (Path(self.temp.name) / "toolchains" / "1.96.0-x86_64-unknown-linux-gnu").mkdir(parents=True)

    def probe(self, channel, returncode=0, stderr="", raises=None):
        result = types.SimpleNamespace(returncode=returncode, stdout="", stderr=stderr)
        with mock.patch.dict(os.environ, {"RUSTUP_HOME": self.temp.name}), mock.patch.object(subprocess, "run", return_value=result, side_effect=raises) as run:
            fault = self.remote["toolchain_fault"](channel)
        self.assertTrue(all(call.args[0] == ["cargo", "--version"] for call in run.call_args_list))
        return fault, [call.kwargs["env"]["RUSTUP_TOOLCHAIN"] for call in run.call_args_list]

    def test_an_installed_toolchain_whose_cargo_fails_is_named_and_one_the_vm_lacks_is_rustups(self):
        missing = "error: 'cargo' is not installed for the toolchain '1.96.0-x86_64-unknown-linux-gnu'."
        broken = bt.BROKEN_TOOLCHAIN
        self.assertEqual(self.probe("1.96.0", 1, missing), ((broken, missing), ["1.96.0"]))
        self.assertEqual(self.probe("1.96.0-x86_64-unknown-linux-gnu", 1, missing), ((broken, missing), ["1.96.0-x86_64-unknown-linux-gnu"]))
        self.assertEqual(self.probe("1.96.0", 1), ((broken, "exit 1"), ["1.96.0"]))
        self.assertEqual(self.probe("1.96.0"), ((None, None), ["1.96.0"]))
        self.assertEqual(self.probe("1.97.0", 1), ((None, None), []))
        self.assertEqual(self.probe("", 1), ((None, None), []))

    def test_a_linked_toolchain_is_probed_and_a_launch_that_fails_for_now_quarantines_nothing(self):
        (Path(self.temp.name) / "toolchains" / "local-ci").mkdir()
        broken, unproven = bt.BROKEN_TOOLCHAIN, self.remote["UNPROVEN_TOOLCHAIN"]
        self.assertEqual(self.probe("local-ci", 1, "broken"), ((broken, "broken"), ["local-ci"]))
        for error in (FileNotFoundError(2, "No such file", "cargo"), PermissionError(13, "Permission denied", "cargo"), OSError(8, "Exec format error", "cargo")):
            self.assertEqual(self.probe("1.96.0", raises=error)[0], (broken, repr(error)))
        for error in (BlockingIOError(11, "Resource temporarily unavailable"), OSError(12, "Cannot allocate memory")):
            self.assertEqual(self.probe("1.96.0", raises=error)[0], (unproven, repr(error)))
        self.assertEqual(self.probe("1.96.0", raises=subprocess.TimeoutExpired("cargo", 60))[0][0], unproven)
        self.assertEqual(self.probe("1.96.0", -11), ((unproven, "cargo --version was killed by signal 11"), ["1.96.0"]))
        self.assertNotIn(unproven, {bt.BROKEN_TOOLCHAIN, bt.REMOTE_CLAIMED, 0})

    def test_an_exec_reply_without_output_reads_as_empty(self):
        with mock.patch.object(bt, "boat", return_value=(OK, [{"exitCode": 0, "stdout": None}])):
            self.assertEqual(bt.quick("a", "acquire", "t"), (0, ""))

class Run(Harness):
    def setUp(self):
        super().setUp()
        self.repo = self.root / "repo"
        (self.repo / "src").mkdir(parents=True)
        (self.repo / "src" / "lib.rs").write_text("")
        subprocess.run(["git", "init", "-q", str(self.repo)], check=True)
        subprocess.run(["git", "-C", str(self.repo), "add", "."], check=True)
        subprocess.run(["git", "-C", str(self.repo), "-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "t"], check=True)
        self.cwd = os.getcwd()
        os.chdir(self.repo)
        os.environ["BOAT_STATE_DIR"] = str(self.state.root)

    def tearDown(self):
        os.chdir(self.cwd)
        del os.environ["BOAT_STATE_DIR"]
        super().tearDown()

    def last_record(self):
        return json.loads((self.state.logs / "timings.jsonl").read_text().splitlines()[-1])

    def test_compile_errors_pass_through_and_leave_the_vm_idle(self):
        fake = self.use(FakeBoat([vm("a", type="default")]))
        fake.cargo_exit = 101
        (self.repo / "rust-toolchain.toml").write_text('[toolchain]\nchannel = "1.96.0"\n')
        self.assertEqual(bt.main(["check", "-p", "x", "-p", "y"]), 101)
        self.assertIn(("a", "acquire", fake.remote_calls[0][2], "1.96.0"), fake.remote_calls)
        self.assertEqual(list(self.state.lanes.glob("*.running-*")), [])
        record = self.last_record()
        self.assertEqual((record["exit"], record["cargo_exit"], record["completed"], record["vm"]), (101, 101, True, "a"))
        self.assertEqual(set(record["phases"]), {"allocate", "sync", "run"})
        self.assertEqual(fake.ttl_calls("a")[-1], bt.IDLE_SECONDS)
        self.state.lock("a").close()
        fake.remote_calls.clear()
        self.assertEqual(bt.main(["ui"]), 0)
        self.assertEqual([len(call) for call in fake.remote_calls if call[1] == "acquire"], [3])

    def test_a_failed_sync_is_a_boat_failure_whose_job_released_the_claim(self):
        fake = self.use(FakeBoat([vm("a", type="default")]))
        fake.sync_status = bt.SYNC_FAILED
        self.assertEqual(bt.main(["check", "-p", "x"]), bt.UNAVAILABLE)
        record = self.last_record()
        self.assertIn("synchronization", record["failure"])
        self.assertIsNone(record["cargo_exit"])
        self.assertNotIn("release", [call[1] for call in fake.remote_calls])
        self.assertGreater(self.state.entry("a")["used"], 0)

    def test_an_untracked_asset_is_refused_before_a_vm_is_claimed(self):
        fake = self.use(FakeBoat([vm("a", type="default")]))
        (self.repo / "copied.dds").write_bytes(b"DDS " + bytes(range(256)))
        (self.repo / "second.ogg").write_bytes(b"trial")
        with mock.patch.object(bt, "say") as output:
            self.assertEqual(bt.main(["check", "-p", "x"]), 2)
        refusal = output.call_args.args[0]
        self.assertIn("copied.dds", refusal)
        self.assertIn("second.ogg", refusal)
        self.assertEqual(fake.remote_calls, [])
        (self.repo / ".gitignore").write_text("*.dds\n*.ogg\n")
        tracked = self.repo / "src" / "lib.rs"
        tracked.write_bytes(b"trial\0")
        self.assertEqual(bt.main(["check", "-p", "x"]), 2)
        self.assertEqual(fake.synced, [])
        tracked.write_text("fn trial() {}")
        self.assertEqual(bt.main(["check", "-p", "x"]), 0)
        crate_src = self.repo / "crates" / "a" / "src"
        crate_src.mkdir(parents=True)
        (crate_src / "lib.rs").write_text("#[cfg(test)]\nmod tests;\n")
        (crate_src / "tests.rs").write_text("const VANILLA: i32 = 5;\n")
        (self.repo / ".gitignore").write_text("*.dds\n*.ogg\ncrates/a/examples/\n")
        examples = self.repo / "crates" / "a" / "examples"
        examples.mkdir(parents=True)
        (examples / "local.rs").write_text('#[path = "../src/tests.rs"]\nmod tests;\n')
        before = list(fake.remote_calls)
        self.assertEqual(bt.main(["check", "-p", "x"]), 2)
        self.assertEqual(fake.remote_calls, before)
        (crate_src / "tests.rs").write_text("const FIXTURE: i32 = 5;\n")
        subprocess.run(["git", "-C", str(self.repo), "add", "crates/a/src/lib.rs", "crates/a/src/tests.rs"], check=True)
        (crate_src / "tests.rs").write_text("const VANILLA: i32 = 5;\n")
        self.assertEqual(bt.main(["check", "-p", "x"]), 2)

    def test_a_small_source_goes_up_in_commands_and_a_large_one_in_one_push(self):
        fake = self.use(FakeBoat([vm("a", type="default")]))
        self.assertEqual(bt.main(["check", "-p", "x"]), 0)
        self.assertTrue(fake.synced[-1].startswith("parts:"))
        self.assertNotIn("push", [call[0] for call in fake.calls])
        bt.UPLOAD_CHUNKS = 0
        self.assertEqual(bt.main(["check", "-p", "x"]), 0)
        self.assertTrue(fake.synced[-1].startswith("file:"))
        self.assertIn("push", [call[0] for call in fake.calls])

    def test_failed_download_is_not_a_success(self):
        fake = self.use(FakeBoat([vm("a")]))
        fake.fail_downloads = True
        self.assertEqual(bt.main(["build", "--out", str(self.root / "out"), "--release", "-p", "viewer"]), bt.UNAVAILABLE)
        record = self.last_record()
        self.assertEqual((record["exit"], record["cargo_exit"], record["completed"], record["failure_phase"]), (bt.UNAVAILABLE, 0, False, "deliver"))
        self.assertNotIn("artifacts_verified", record)
        self.assertEqual(fake.ttl_calls("a")[-1], bt.IDLE_SECONDS)

    def test_a_test_run_lets_the_vm_go_then_runs_what_it_delivered_with_the_runner_told_what_to_send(self):
        fake = self.use(FakeBoat([vm("a", type="default")]))
        (self.repo / "Cargo.toml").write_text('[workspace]\nmembers = ["crates/*"]\n')
        (self.repo / "crates" / "app" / "tests").mkdir(parents=True)
        (self.repo / "crates" / "app" / "Cargo.toml").write_text('[package]\nname = "app"\n')
        (self.repo / "crates" / "app" / "tests" / "cli.rs").write_text('fn f() { std::env::var_os("CARGO_BIN_EXE_app"); env!("CARGO_BIN_EXE_app"); }\n')
        out = self.root / "out"
        out.mkdir()
        (out / "boat-artifacts.json").write_text(json.dumps({"cargo_arguments": ["test"]}))
        (out / "old-1a.exe").write_text("")
        fake.outcome = {"exit": 0, "tests": [program("app", ("test",), "cli", "cli-2b.exe")], "binaries": [{"package": "app", "name": "app", "executable": "app.exe"}], "artifacts": [{"path": "x"}], "bundle": {}}
        delivered, ran = [], []

        def deliver(vmid, job, outdir):
            delivered.append((sorted(path.name for path in outdir.iterdir()), job["tests"][0]["executable"]))
            return [{"path": f"/t/{name}", "sha256": "0" * 64} for name in ("cli-2b.exe", "app.exe")]

        def run_program(executable, args, cwd, env):
            self.state.lock("a").close()
            ran.append((executable, args, cwd, env["CARGO_BIN_EXE_app"]))
            return 0, 1, 0

        bt.deliver, bt.run_program = deliver, run_program
        self.state.update("a", used=1.0, tree_build_release={"app": "x"}, tree_build_release_used=1.0)
        self.assertEqual(bt.main(["test", "--out", str(out), "-p", "app", "--test", "cli", "--", "name"]), 0)
        test = next(call for call in fake.remote_calls if call[1] == "test")
        self.assertEqual(test[3:5], ("--sync", fake.synced[-1]))
        self.assertEqual(list(test[5:]), ["--bin-exe=app/app", "--", "test", "--no-run", "-p", "app", "--test", "cli"])
        self.assertEqual(delivered, [([bt.TEST_LOCK], "cli-2b.exe")])
        self.assertEqual(ran, [(out.resolve() / "cli-2b.exe", ["name"], self.repo.resolve() / "crates" / "app", str(out.resolve() / "app.exe"))])
        self.assertEqual(json.loads((out / "boat-artifacts.json").read_text())["cargo_arguments"][:2], ["test", "--no-run"])
        record = self.last_record()
        self.assertEqual((record["mode"], record["exit"], set(record["phases"])), ("test", 0, {"allocate", "sync", "run", "deliver"}))
        entry = self.state.entry("a")
        self.assertEqual((list(entry["tree_tests_dev"]), entry["tree_build_release"]), (["app"], {"app": "x"}))

    def test_an_image_is_saved_after_the_build_steps_from_the_selected_source(self):
        self.saved.update(SNAPSHOT=bt.SNAPSHOT)
        bt.SNAPSHOT = "image"
        fake = self.use(FakeBoat([]))
        fake.missing = {"image"}
        self.assertEqual(bt.main(["snapshot"]), bt.UNAVAILABLE)
        self.assertEqual([call[2] for call in fake.calls if call[0] == "new"], ["image"])
        fake = self.use(FakeBoat([]))
        with mock.patch.dict(os.environ, {"BOAT_MACHINE_TYPE": ""}):
            self.assertEqual(bt.main(["snapshot", "--from", "seed"]), 0)
        news = [call for call in fake.calls if call[0] == "new"]
        self.assertEqual(len(news), 1)
        self.assertEqual(news[0][:7], ("new", "--from", "seed", "--type", "default", "--name", "battleplan-image-image"))
        steps = [call for call in fake.remote_calls if len(call) > 2 and call[1] in {"clear", "ui", "check", "test", "build", "provenance", "prune"}]
        self.assertEqual([call[1] for call in steps], ["clear", "ui", "check", "provenance", "prune"])
        self.assertEqual([call[3:4] == ("--sync",) for call in steps], [False, True, False, False, False])
        self.assertEqual(fake.calls[-2][0:3], ("snapshot", "n", "image"))
        self.assertEqual(fake.ttl_calls("n")[-1], bt.IDLE_SECONDS)
        fake = self.use(FakeBoat([]))
        fake.cargo_exit = 101
        self.assertEqual(bt.main(["snapshot"]), 0)
        fake = self.use(FakeBoat([]))
        fake.cargo_exit = 1
        self.assertEqual(bt.main(["snapshot"]), bt.UNAVAILABLE)
        self.assertNotIn("snapshot", [call[0] for call in fake.calls])
        self.assertEqual(fake.ttl_calls("n")[-1], bt.IDLE_SECONDS)

TLS = '{"error":"could not reach the Boat API at https://boat.dev: error sending request for url (https://boat.dev/api/boat/sandboxes): client error (Connect): tls handshake eof","event":"error"}'
NOT_READY = '{"code":"sandbox_not_ready","error":"Sandbox a is not ready (state: updating)","event":"error"}'
LOST = '{"error":"could not reach the Boat API at https://boat.dev: error sending request for url (https://boat.dev/api/boat/sandboxes): connection closed before message completed","event":"error"}'

def result(returncode=0, stdout=""):
    return types.SimpleNamespace(returncode=returncode, stdout=stdout, stderr="")

class Attach:
    """A fake `boat exec --attach` process that prints the given JSON frames, then optionally raises."""

    def __init__(self, frames, returncode, interrupt=False):
        self.lines = [json.dumps(frame) + "\n" for frame in frames]
        self.returncode, self.interrupt, self.killed = returncode, interrupt, False

    @property
    def stdout(self):
        yield from self.lines
        if self.interrupt:
            raise KeyboardInterrupt

    def poll(self):
        return None if self.interrupt and not self.killed else self.returncode

    def kill(self):
        self.killed = True

    def wait(self):
        return self.returncode

class Transport(unittest.TestCase):
    def setUp(self):
        self.saved = {name: getattr(bt, name) for name in ("subprocess", "RETRY_SECONDS", "OUTCOME_SECONDS", "say")}
        bt.RETRY_SECONDS = (0, 0, 0)
        bt.OUTCOME_SECONDS = 0
        self.runs = []
        self.attaches = []
        self.said = []
        bt.say = self.said.append

    def tearDown(self):
        for name, value in self.saved.items():
            setattr(bt, name, value)

    def fake(self, results, attaches=()):
        results, attaches = list(results), list(attaches)

        def run(command, **_):
            self.runs.append(command)
            outcome = results.pop(0)
            return outcome(command) if callable(outcome) else outcome

        def popen(command, **_):
            self.attaches.append(command)
            return attaches.pop(0)

        bt.subprocess = types.SimpleNamespace(run=run, Popen=popen, PIPE=subprocess.PIPE, STDOUT=subprocess.STDOUT, CREATE_NEW_PROCESS_GROUP=getattr(subprocess, "CREATE_NEW_PROCESS_GROUP", 0x200))

    def transient(self, returncode, stdout, command):
        return bt.transient(result(returncode, stdout), command in bt.IDEMPOTENT)

    def test_only_calls_that_never_ran_or_repeat_safely_are_transient(self):
        self.assertTrue(self.transient(1, TLS, "list"))
        self.assertTrue(self.transient(bt.LAUNCH_FAILED, "", "resume"))
        self.assertTrue(self.transient(-1073741502, "", "exec"))
        self.assertTrue(self.transient(1, LOST, "info"))
        self.assertTrue(self.transient(1, '{"event":"error","status":503}', "extend"))
        self.assertTrue(self.transient(1, NOT_READY, "info"))
        self.assertFalse(self.transient(1, NOT_READY, "resume"))
        for command in ("new", "resume", "exec", "snapshot"):
            self.assertFalse(self.transient(1, TLS, command), command)
        self.assertFalse(self.transient(1, '{"code":"snapshot_not_found","event":"error","status":404}', "info"))

    def test_never_repeats_a_create_whatever_it_reported(self):
        for stdout in (TLS, '{"event":"created","id":"bx_1"}\n' + TLS, '{"event":"crea\n' + TLS):
            self.runs.clear()
            self.fake([result(1, stdout)])
            outcome, _ = bt.boat("new", "--from", bt.SNAPSHOT, check=False)
            self.assertEqual((outcome.returncode, len(self.runs)), (1, 1), stdout)

    def test_retries_a_dropped_connection_then_succeeds(self):
        self.fake([result(1, TLS), result(1, TLS), result(0, '{"sandboxes":[]}')])
        outcome, events = bt.boat("list", "--all")
        self.assertEqual((outcome.returncode, events, len(self.runs)), (0, [{"sandboxes": []}], 3))

    def test_never_repeats_a_create_whose_reply_was_lost(self):
        self.fake([result(1, LOST)])
        with self.assertRaises(bt.Failure):
            bt.boat("new", "--from", bt.SNAPSHOT)
        self.assertEqual(len(self.runs), 1)

    def test_gives_up_after_the_last_wait(self):
        self.fake([result(1, TLS)] * (len(bt.RETRY_SECONDS) + 1))
        with self.assertRaises(bt.Failure):
            bt.boat("info", "a")
        self.assertEqual(len(self.runs), len(bt.RETRY_SECONDS) + 1)

    def test_reattaching_after_a_drop_neither_repeats_nor_loses_output(self):
        first = Attach([{"type": "attached"}, {"type": "stdout", "data": "one\n", "nextOffset": 4}, {"type": "stderr", "data": "é\n", "nextOffset": 3}], 1)
        second = Attach([
            {"type": "attached"},
            {"type": "stdout", "data": "one\ntwo\n", "nextOffset": 8},
            {"type": "stderr", "data": "é\nx\n", "nextOffset": 5},
            {"type": "exit", "exitCode": 7, "signal": None, "status": "exited"},
        ], 7)
        self.fake([result(0, '{"pid":42,"processId":42}')], [first, second])
        shown, errors = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(shown), contextlib.redirect_stderr(errors):
            self.assertEqual(bt.remote("a", "check", "t")[0], 7)
        self.assertEqual((shown.getvalue(), errors.getvalue()), ("one\ntwo\n", "é\nx\n"))
        self.assertIn("--detach", self.runs[0])
        self.assertEqual([command[-1] for command in self.attaches], ["42", "42"])

    def test_undecodable_output_is_kept_whole_and_never_cut(self):
        lossy = {"type": "stdout", "data": "AB�CD\n", "nextOffset": 6}
        exit = {"type": "exit", "exitCode": 0, "signal": None, "status": "exited"}
        self.fake([result(0, '{"pid":1}')], [Attach([{"type": "attached"}, lossy, exit], 0)])
        shown = io.StringIO()
        with contextlib.redirect_stdout(shown):
            self.assertEqual(bt.remote("a", "check", "t")[0], 0)
        self.assertEqual(shown.getvalue(), "AB�CD\n")
        first = Attach([{"type": "attached"}, {"type": "stdout", "data": "AB", "nextOffset": 2}], 1)
        self.fake([result(0, '{"pid":2}')], [first, Attach([{"type": "attached"}, lossy, exit], 0)])
        shown = io.StringIO()
        with contextlib.redirect_stdout(shown):
            self.assertEqual(bt.remote("a", "check", "t")[0], 0)
        self.assertEqual(shown.getvalue(), "ABAB�CD\n")
        self.assertTrue(any("repeat" in line for line in self.said))

    def test_interrupting_stops_the_remote_job(self):
        self.fake([result(0, '{"pid":42}'), result(0), result(0, '{"pid":42,"running":false}')], [Attach([{"type": "attached"}], None, interrupt=True)])
        with self.assertRaises(KeyboardInterrupt):
            bt.remote("a", "check", "t")
        self.assertIn(["--kill", "42"], [command[-2:] for command in self.runs])
        self.assertTrue(self.attaches and self.runs[-1][-2:] == ["--status", "42"])
        self.assertFalse(any("may still be running" in line for line in self.said))

    def test_a_stop_that_cannot_run_keeps_the_interrupt(self):
        def unlaunchable(command):
            raise OSError("insufficient system resources")

        self.fake([result(0, '{"pid":42}'), unlaunchable], [Attach([{"type": "attached"}], None, interrupt=True)])
        with self.assertRaises(KeyboardInterrupt):
            bt.remote("a", "check", "t")
        self.assertTrue(any("could not stop Boat job 42" in line for line in self.said))

    def test_a_status_error_is_not_taken_as_stopped(self):
        error = result(1, '{"event":"error","error":"could not reach the Boat API"}')
        self.fake([result(0, '{"pid":42}'), result(0)] + [error] * 16, [Attach([{"type": "attached"}], None, interrupt=True)])
        with self.assertRaises(KeyboardInterrupt):
            bt.remote("a", "check", "t")
        self.assertTrue(any("may still be running" in line for line in self.said))

    def test_ctrl_c_while_the_job_starts_still_stops_it(self):
        def started(command):
            signal.raise_signal(signal.SIGINT)
            return result(0, '{"pid":42}')

        self.fake([started, result(0), result(0, '{"pid":42,"running":false}')])
        with self.assertRaises(KeyboardInterrupt):
            bt.remote("a", "check", "t")
        self.assertEqual([command[-2:] for command in self.runs[1:]], [["--kill", "42"], ["--status", "42"]])
        self.assertEqual(self.attaches, [])

    def test_ctrl_c_is_kept_when_the_start_fails_or_names_no_job(self):
        for reply in (result(1, '{"event":"error","error":"refused"}'), result(0, "{}")):
            def started(command, reply=reply):
                signal.raise_signal(signal.SIGINT)
                return reply

            self.runs.clear()
            self.said.clear()
            self.fake([started])
            with self.assertRaises(KeyboardInterrupt):
                bt.remote("a", "check", "t")
            self.assertEqual(len(self.runs), 1)
            self.assertTrue(any("interrupted while starting" in line for line in self.said))

    def test_ctrl_c_is_kept_when_boat_exe_cannot_start(self):
        def started(command):
            signal.raise_signal(signal.SIGINT)
            raise OSError("insufficient system resources")

        self.fake([started])
        with self.assertRaises(KeyboardInterrupt):
            bt.remote("a", "check", "t")
        self.assertTrue(any("interrupted while starting" in line for line in self.said))

    def test_a_reconnect_within_one_pass_shows_the_whole_output(self):
        frames = [{"type": "attached"}, {"type": "stdout", "data": "ab", "nextOffset": 2}, {"type": "attached", "stdoutOffset": 2, "stderrOffset": 0}, {"type": "stdout", "data": "cd", "nextOffset": 4}, {"type": "exit", "exitCode": 0}]
        self.fake([result(0, '{"pid":6}')], [Attach(frames, 0)])
        shown = io.StringIO()
        with contextlib.redirect_stdout(shown):
            self.assertEqual(bt.remote("a", "check", "t")[0], 0)
        self.assertEqual(shown.getvalue(), "abcd")

    def test_a_job_boat_loses_is_waited_for_and_its_outcome_read_from_the_vm(self):
        lost = Attach([{"type": "attached"}, {"type": "exit", "exitCode": None, "signal": None, "status": "lost"}], 0)
        starting = result(0, json.dumps({"exitCode": bt.NOT_STARTED, "stdout": "", "stderr": ""}))
        running = result(0, json.dumps({"exitCode": bt.STILL_RUNNING, "stdout": "", "stderr": ""}))
        done = result(0, json.dumps({"exitCode": 0, "stdout": json.dumps({"exit": 7}), "stderr": ""}))
        self.fake([result(0, '{"pid":42}'), starting, running, done], [lost])
        code, job = bt.remote("a", "build", "t")
        self.assertEqual((code, self.runs[0][-1]), (7, job))
        self.assertEqual(len(self.attaches), 1)
        self.assertEqual([command[-2:] for command in self.runs[1:]], [["outcome", job]] * 3)

    def test_a_job_that_never_starts_leaves_no_outcome(self):
        starting = result(0, json.dumps({"exitCode": bt.NOT_STARTED, "stdout": "", "stderr": "The job has not started"}))
        lost = Attach([{"type": "attached"}, {"type": "exit", "exitCode": None, "signal": None, "status": "lost"}], 0)
        self.fake([result(0, '{"pid":42}')] + [starting] * (len(bt.RETRY_SECONDS) + 1), [lost])
        with self.assertRaises(bt.Failure):
            bt.remote("a", "check", "t")
        self.assertEqual(len(self.runs), len(bt.RETRY_SECONDS) + 2)

    def test_a_job_that_never_reports_an_exit_nor_leaves_an_outcome_fails_closed(self):
        gone = result(0, json.dumps({"exitCode": 1, "stdout": "", "stderr": "The job ended without leaving its outcome"}))
        self.fake([result(0, '{"pid":42}'), gone], [Attach([{"type": "attached"}], 1) for _ in range(len(bt.RETRY_SECONDS) + 1)])
        with self.assertRaises(bt.Failure):
            bt.remote("a", "check", "t")
        self.assertEqual(len(self.attaches), len(bt.RETRY_SECONDS) + 1)
        self.assertEqual(self.runs[-1][-2:], ["outcome", self.runs[0][-1]])

if __name__ == "__main__":
    unittest.main()
