"""Tests for tools/boat.py against a fake Boat: python tools/boat_test.py"""

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
import types
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import boat as bt  # noqa: E402

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
        self.renewed = threading.Event()

    def boat(self, *args, check=True):
        self.calls.append(args)
        command = args[0]
        if command == "list":
            return OK, [{"sandboxes": [dict(vm) for vm in self.vms.values()]}]
        if command == "info":
            return OK, [{"sandbox": dict(self.vms[args[1]])}]
        if command == "extend":
            self.vms[args[1]]["ttlSeconds"] = int(args[3])
            if int(args[3]) == bt.LEASE_SECONDS:
                self.renewed.set()
            return OK, []
        if command == "resume":
            vm = self.vms[args[1]]
            vm.update(state="idle", ttlSeconds=int(args[3]))
            if "--type" in args:
                vm["type"] = args[args.index("--type") + 1]
            return OK, []
        if command == "push":
            files = [path for path in Path(args[2]).rglob("*") if path.is_file() and ".git" not in path.parts]
            return OK, [{"files": len(files)}]
        if command == "exec" and "--timeout" in args:
            if self.fail_downloads and any("dd if=" in arg for arg in args):
                raise bt.Failure("Boat exec failed")
            return OK, []
        raise AssertionError(f"unexpected boat call {args}")

    def remote(self, vmid, *args, stream=False):
        self.remote_calls.append((vmid, *args))
        mode = args[0]
        if mode == "acquire":
            return self.acquire_status.get(vmid, 0), self.acquire_output.get(vmid, "{}")
        if mode == "release":
            return 0, ""
        if mode in {"check", "build"} and args[2] == "--sync":
            self.synced.append(args[3])
            if self.sync_status:
                return self.sync_status, ""
        if mode == "check":
            return self.cargo_exit, ""
        if mode == "build":
            artifact = {"path": f"{bt.REMOTE}/target/{bt.TARGET}/release/viewer.exe", "sha256": "0" * 64}
            bundle = {"path": f"{bt.REMOTE}/artifacts-x.tar.xz", "size": 10, "sha256": "0" * 64}
            return 0, "BOAT_ARTIFACTS=" + json.dumps([artifact]) + "\nBOAT_BUNDLE=" + json.dumps(bundle) + "\n"
        raise AssertionError(f"unexpected remote call {args}")

    def ttl_calls(self, vmid):
        return [int(call[3]) for call in self.calls if call[0] == "extend" and call[1] == vmid]


def vm(vmid, state="idle", type="large", ttl=bt.IDLE_SECONDS, hydrated=True):
    return {"id": vmid, "state": state, "type": type, "ttlSeconds": ttl, "hydrated": hydrated}


class Harness(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.state = bt.State(self.root / "state")
        self.saved = {name: getattr(bt, name) for name in ("boat", "remote", "quick", "RENEW_SECONDS", "HYDRATION_SECONDS", "UPLOAD_CHUNKS")}

    def tearDown(self):
        for name, value in self.saved.items():
            setattr(bt, name, value)
        self.temp.cleanup()

    def use(self, fake):
        bt.boat = fake.boat
        bt.remote = fake.remote
        bt.quick = fake.remote
        for vmid in fake.vms:
            bt.atomic_json(self.state.pool / (vmid + ".json"), {"id": vmid, "snapshot": bt.SNAPSHOT})
        return fake


class Rank(unittest.TestCase):
    def test_running_before_stopped_then_lane_then_size(self):
        states = {vmid: vm(vmid, state, type) for vmid, state, type in [("a", "stopped", "large"), ("b", "idle", "default"), ("c", "idle", "large"), ("d", "idle", "large")]}
        self.assertEqual(bt.rank(sorted(states), states, "d", "default", lambda _: False), ["d", "b", "c", "a"])

    def test_takes_the_vm_another_lane_left_longest_ago(self):
        states = {vmid: vm(vmid) for vmid in "abc"}
        used = {"a": 300, "b": 100, "c": 200}.get
        self.assertEqual(bt.rank(sorted(states), states, "c", "large", lambda _: False, used), ["c", "b", "a"])

    def test_never_picks_stopping_cooling_or_unlisted_vms(self):
        states = {"a": vm("a", "stopping"), "b": vm("b"), "c": vm("c", "stopped")}
        self.assertEqual(bt.rank(["a", "b", "c", "gone"], states, None, "large", lambda vmid: vmid == "b"), ["c"])

    def test_large_work_skips_running_default_vms_but_can_grow_stopped_ones(self):
        states = {"a": vm("a", "idle", "default"), "b": vm("b", "stopped", "default")}
        self.assertEqual(bt.rank(["a", "b"], states, None, "large", lambda _: False), ["b"])

    def test_the_running_vm_with_the_least_to_compile_again_comes_first(self):
        states = {vmid: vm(vmid, state) for vmid, state in [("a", "stopped"), ("b", "idle"), ("c", "idle"), ("d", "idle")]}
        rebuilds = {"a": (0, 0), "b": (60, 0), "c": (3, 0), "d": (60, 0)}.get
        self.assertEqual(bt.rank(sorted(states), states, "d", "large", lambda _: False, rebuilds=rebuilds), ["c", "d", "b", "a"])

    def test_resume_only_grows(self):
        self.assertEqual(bt.resume_type("default", "large"), "large")
        self.assertIsNone(bt.resume_type("large", "default"))
        self.assertIsNone(bt.resume_type("large", "large"))
        self.assertIsNone(bt.resume_type("default", "default"))


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
        # The VM's target holds the other tree, its cache both.
        self.assertEqual((first.rebuilds(entry), other.rebuilds(entry)), ((0, 3), (0, 0)))
        # Another helper's run since voids the tree, never the cache; checks have none.
        self.assertEqual(first.rebuilds({**entry, "used": 3.0}), (0, 3))
        self.assertEqual(self.plan("check", "-p", "top").rebuilds(entry), (3, 0))

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


class Parse(unittest.TestCase):
    def test_check_and_build(self):
        self.assertEqual(bt.parse(["check", "-p", "viewer", "--all-targets"]), ("check", ["check", "-p", "viewer", "--all-targets"], None, False))
        mode, cargo, outdir, refresh = bt.parse(["build", "--refresh-image", "--out", "x", "--release", "-p", "viewer"])
        self.assertEqual((mode, cargo, outdir, refresh), ("build", ["build", "--release", "-p", "viewer"], Path("x"), True))
        self.assertEqual(bt.parse(["ui"]), ("ui", [], None, False))
        mode, cargo, outdir, _ = bt.parse(["build", "--out", "x", "--profile", "release-lto", "-p", "collector", "--lib"])
        self.assertEqual((mode, cargo, outdir, bt.profile(cargo)), ("build", ["build", "--profile", "release-lto", "-p", "collector", "--lib"], Path("x"), "release-lto"))

    def test_refuses_what_the_tool_owns(self):
        for args in (["check", "-j", "3"], ["check", "-j3"], ["check", "--jobs=2"], ["check", "--target", "x"], ["check", "--target-dir=x"], ["check", "--message-format=json"], ["build", "--out", "x", "-p", "viewer"], ["build", "--out", "x", "--profile", "dev"], ["build", "--out", "x", "--profile=test"], ["build", "--release"], ["ui", "x"], ["test"], []):
            with self.subTest(args=args), self.assertRaises(bt.Failure):
                bt.parse(args)


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
    def test_hydration_timeout_cools_the_vm_and_ends_its_lease(self):
        fake = self.use(FakeBoat([vm("a", "stopped", hydrated=False)]))
        bt.HYDRATION_SECONDS = 0
        with self.assertRaises(bt.Failure):
            bt.acquire(self.state, "lane", "large", "t", {})
        self.assertEqual(fake.ttl_calls("a")[-1], bt.IDLE_SECONDS)
        self.assertTrue(self.state.cooling("a", 0))
        self.state.lock("a").close()

    def test_resume_never_shrinks_a_large_vm(self):
        fake = self.use(FakeBoat([vm("a", "stopped", "large")]))
        vmid, lock, lease, _ = bt.acquire(self.state, "lane", "default", "t", {})
        lease.end()
        lock.close()
        resume = next(call for call in fake.calls if call[0] == "resume")
        self.assertNotIn("--type", resume)
        self.assertEqual(fake.vms["a"]["type"], "large")

    def test_the_runner_is_installed_before_the_claim(self):
        fake = self.use(FakeBoat([vm("a")]))
        installed = []

        def quick(vmid, *args):
            installed.append(any(call[0] == "exec" and "runner-remote.py.b64-000" in " ".join(call) for call in fake.calls))
            return fake.remote(vmid, *args)

        bt.quick = quick
        vmid, lock, lease, _ = bt.acquire(self.state, "lane", "large", "t", {})
        lease.end()
        lock.close()
        self.assertEqual((fake.remote_calls, installed), ([("a", "acquire", "t")], [True]))

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

    def test_a_vm_still_finishing_a_job_is_passed_by_and_stays_in_the_pool(self):
        fake = self.use(FakeBoat([vm("a"), vm("b")]))
        fake.acquire_status["a"] = bt.REMOTE_CLAIMED
        fake.acquire_output["a"] = "busy\n"
        vmid, lock, lease, _ = bt.acquire(self.state, "lane", "large", "t", {})
        lease.end()
        lock.close()
        self.assertEqual(vmid, "b")
        self.assertEqual(self.state.quarantined(), [])
        self.assertEqual(self.state.registered(), ["a", "b"])

    def test_lease_renews_until_ended(self):
        fake = self.use(FakeBoat([vm("a")]))
        bt.RENEW_SECONDS = 0.01
        lease = bt.Lease("a")
        self.assertTrue(fake.renewed.wait(10))
        lease.end()
        count = len(fake.calls)
        self.assertEqual(fake.ttl_calls("a")[-1], bt.IDLE_SECONDS)
        lease.thread.join()
        self.assertEqual(len(fake.calls), count)


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
        fake = self.use(FakeBoat([vm("a")]))
        fake.cargo_exit = 101
        self.assertEqual(bt.main(["check", "-p", "x", "-p", "y"]), 101)
        record = self.last_record()
        self.assertEqual((record["exit"], record["cargo_exit"], record["completed"], record["vm"]), (101, 101, True, "a"))
        self.assertEqual(set(record["phases"]), {"allocate", "sync", "run"})
        self.assertEqual(fake.ttl_calls("a")[-1], bt.IDLE_SECONDS)
        self.state.lock("a").close()

    def test_a_failed_sync_is_a_boat_failure_whose_job_released_the_claim(self):
        fake = self.use(FakeBoat([vm("a")]))
        fake.sync_status = bt.SYNC_FAILED
        self.assertEqual(bt.main(["check", "-p", "x"]), bt.UNAVAILABLE)
        record = self.last_record()
        self.assertIn("synchronization", record["failure"])
        self.assertIsNone(record["cargo_exit"])
        self.assertNotIn("release", [call[1] for call in fake.remote_calls])
        self.assertGreater(self.state.entry("a")["used"], 0)

    def test_a_small_source_goes_up_in_commands_and_a_large_one_in_one_push(self):
        fake = self.use(FakeBoat([vm("a")]))
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


TLS = '{"error":"could not reach the Boat API at https://boat.dev: error sending request for url (https://boat.dev/api/boat/sandboxes): client error (Connect): tls handshake eof","event":"error"}'
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
        self.saved = {name: getattr(bt, name) for name in ("subprocess", "RETRY_SECONDS", "say")}
        bt.RETRY_SECONDS = (0, 0, 0)
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
        self.assertEqual(bt.remote("a", "check"), (7, "one\ntwo\n"))
        self.assertIn("é\nx", self.said)
        self.assertIn("--detach", self.runs[0])
        self.assertEqual([command[-1] for command in self.attaches], ["42", "42"])

    def test_undecodable_output_is_kept_whole_and_never_cut(self):
        lossy = {"type": "stdout", "data": "AB�CD\n", "nextOffset": 6}
        exit = {"type": "exit", "exitCode": 0, "signal": None, "status": "exited"}
        self.fake([result(0, '{"pid":1}')], [Attach([{"type": "attached"}, lossy, exit], 0)])
        self.assertEqual(bt.remote("a", "check"), (0, "AB�CD\n"))
        first = Attach([{"type": "attached"}, {"type": "stdout", "data": "AB", "nextOffset": 2}], 1)
        self.fake([result(0, '{"pid":2}')], [first, Attach([{"type": "attached"}, lossy, exit], 0)])
        self.assertEqual(bt.remote("a", "check"), (0, "AB�CD\n"))
        self.assertTrue(any("repeat" in line for line in self.said))

    def test_parsed_output_comes_from_the_whole_final_replay(self):
        first = Attach([{"type": "attached"}, {"type": "stdout", "data": "diag�", "nextOffset": 5}], 1)
        whole = {"type": "stdout", "data": "diag�\nBOAT_ARTIFACTS=[]\n", "nextOffset": 24}
        self.fake([result(0, '{"pid":3}')], [first, Attach([{"type": "attached"}, whole, {"type": "exit", "exitCode": 0}], 0)])
        self.assertEqual(bt.remote("a", "build"), (0, "diag�\nBOAT_ARTIFACTS=[]\n"))

    def test_interrupting_stops_the_remote_job(self):
        self.fake([result(0, '{"pid":42}'), result(0), result(0, '{"pid":42,"running":false}')], [Attach([{"type": "attached"}], None, interrupt=True)])
        with self.assertRaises(KeyboardInterrupt):
            bt.remote("a", "check")
        self.assertIn(["--kill", "42"], [command[-2:] for command in self.runs])
        self.assertTrue(self.attaches and self.runs[-1][-2:] == ["--status", "42"])
        self.assertFalse(any("may still be running" in line for line in self.said))

    def test_a_stop_that_cannot_run_keeps_the_interrupt(self):
        def unlaunchable(command):
            raise OSError("insufficient system resources")

        self.fake([result(0, '{"pid":42}'), unlaunchable], [Attach([{"type": "attached"}], None, interrupt=True)])
        with self.assertRaises(KeyboardInterrupt):
            bt.remote("a", "check")
        self.assertTrue(any("could not stop Boat job 42" in line for line in self.said))

    def test_a_status_error_is_not_taken_as_stopped(self):
        error = result(1, '{"event":"error","error":"could not reach the Boat API"}')
        self.fake([result(0, '{"pid":42}'), result(0)] + [error] * 16, [Attach([{"type": "attached"}], None, interrupt=True)])
        with self.assertRaises(KeyboardInterrupt):
            bt.remote("a", "check")
        self.assertTrue(any("may still be running" in line for line in self.said))

    def test_ctrl_c_while_the_job_starts_still_stops_it(self):
        def started(command):
            signal.raise_signal(signal.SIGINT)
            return result(0, '{"pid":42}')

        self.fake([started, result(0), result(0, '{"pid":42,"running":false}')])
        with self.assertRaises(KeyboardInterrupt):
            bt.remote("a", "check")
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
                bt.remote("a", "check")
            self.assertEqual(len(self.runs), 1)
            self.assertTrue(any("interrupted while starting" in line for line in self.said))

    def test_ctrl_c_is_kept_when_boat_exe_cannot_start(self):
        def started(command):
            signal.raise_signal(signal.SIGINT)
            raise OSError("insufficient system resources")

        self.fake([started])
        with self.assertRaises(KeyboardInterrupt):
            bt.remote("a", "check")
        self.assertTrue(any("interrupted while starting" in line for line in self.said))

    def test_a_reconnect_within_one_pass_keeps_the_whole_replay(self):
        frames = [{"type": "attached"}, {"type": "stdout", "data": "ab", "nextOffset": 2}, {"type": "attached", "stdoutOffset": 2, "stderrOffset": 0}, {"type": "stdout", "data": "cd", "nextOffset": 4}, {"type": "exit", "exitCode": 0}]
        for stream in (False, True):
            self.fake([result(0, '{"pid":6}')], [Attach(frames, 0)])
            shown = io.StringIO()
            with contextlib.redirect_stdout(shown):
                self.assertEqual(bt.remote("a", "check", stream=stream), (0, "abcd"))
            self.assertEqual(shown.getvalue(), "abcd" if stream else "")

    def test_a_reconnect_that_skips_output_is_refused(self):
        frames = [{"type": "attached"}, {"type": "stdout", "data": "ab", "nextOffset": 2}, {"type": "attached", "stdoutOffset": 3, "stderrOffset": 0}, {"type": "stdout", "data": "d", "nextOffset": 4}, {"type": "exit", "exitCode": 0}]
        self.fake([result(0, '{"pid":7}')], [Attach(frames, 0)])
        with self.assertRaises(bt.Failure):
            bt.remote("a", "check")

    def test_a_replay_that_does_not_start_at_the_beginning_is_refused(self):
        first = Attach([{"type": "attached"}, {"type": "stdout", "data": '["a"', "nextOffset": 4}], 1)
        late = Attach([{"type": "attached", "stdoutOffset": 8, "stderrOffset": 0}, {"type": "stdout", "data": "]", "nextOffset": 9}, {"type": "exit", "exitCode": 0}], 0)
        self.fake([result(0, '{"pid":5}')], [first, late])
        with self.assertRaises(bt.Failure):
            bt.remote("a", "acquire")

    def test_a_job_that_never_reports_an_exit_keeps_failing_closed(self):
        self.fake([result(0, '{"pid":42}')], [Attach([{"type": "attached"}], 1) for _ in range(len(bt.RETRY_SECONDS) + 1)])
        with self.assertRaises(bt.Failure):
            bt.remote("a", "check")
        self.assertEqual(len(self.attaches), len(bt.RETRY_SECONDS) + 1)


if __name__ == "__main__":
    unittest.main()
