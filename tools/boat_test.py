"""Tests for tools/boat.py against a fake Boat: python tools/boat_test.py"""

import json
import os
from pathlib import Path
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
        if command == "scp":
            if self.fail_downloads and "artifacts-" in args[1]:
                raise bt.Failure("Boat scp failed")
            return OK, []
        raise AssertionError(f"unexpected boat call {args}")

    def remote(self, vmid, *args, stream=False):
        self.remote_calls.append((vmid, *args))
        mode = args[0]
        if mode == "acquire":
            return self.acquire_status.get(vmid, 0), "{}"
        if mode in {"release", "sync", "discard-bundle"}:
            return 0, ""
        if mode == "check":
            return self.cargo_exit, ""
        if mode == "build":
            artifact = {"path": f"{bt.REMOTE}/target/{bt.TARGET}/release/viewer.exe", "sha256": "0" * 64}
            return 0, "BOAT_ARTIFACTS=" + json.dumps([artifact]) + f"\nBOAT_BUNDLE={bt.REMOTE}/artifacts-x.tar.xz\n"
        if mode == "split-bundle":
            return 0, json.dumps([f"{bt.REMOTE}/artifacts-x.part-0"])
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
        self.saved = {name: getattr(bt, name) for name in ("boat", "remote", "RENEW_SECONDS", "HYDRATION_SECONDS")}

    def tearDown(self):
        for name, value in self.saved.items():
            setattr(bt, name, value)
        self.temp.cleanup()

    def use(self, fake):
        bt.boat = fake.boat
        bt.remote = fake.remote
        for vmid in fake.vms:
            bt.atomic_json(self.state.pool / (vmid + ".json"), {"id": vmid, "snapshot": bt.SNAPSHOT})
        return fake


class Rank(unittest.TestCase):
    def test_running_before_stopped_then_lane_then_size(self):
        states = {vmid: vm(vmid, state, type) for vmid, state, type in [("a", "stopped", "large"), ("b", "idle", "default"), ("c", "idle", "large"), ("d", "idle", "large")]}
        self.assertEqual(bt.rank(sorted(states), states, "d", "default", lambda _: False), ["d", "b", "c", "a"])

    def test_never_picks_stopping_cooling_or_unlisted_vms(self):
        states = {"a": vm("a", "stopping"), "b": vm("b"), "c": vm("c", "stopped")}
        self.assertEqual(bt.rank(["a", "b", "c", "gone"], states, None, "large", lambda vmid: vmid == "b"), ["c"])

    def test_large_work_skips_running_default_vms_but_can_grow_stopped_ones(self):
        states = {"a": vm("a", "idle", "default"), "b": vm("b", "stopped", "default")}
        self.assertEqual(bt.rank(["a", "b"], states, None, "large", lambda _: False), ["b"])

    def test_resume_only_grows(self):
        self.assertEqual(bt.resume_type("default", "large"), "large")
        self.assertIsNone(bt.resume_type("large", "default"))
        self.assertIsNone(bt.resume_type("large", "large"))
        self.assertIsNone(bt.resume_type("default", "default"))


class Parse(unittest.TestCase):
    def test_check_and_build(self):
        self.assertEqual(bt.parse(["check", "-p", "viewer", "--all-targets"]), ("check", ["check", "-p", "viewer", "--all-targets"], None, False))
        mode, cargo, outdir, refresh = bt.parse(["build", "--refresh-image", "--out", "x", "--release", "-p", "viewer"])
        self.assertEqual((mode, cargo, outdir, refresh), ("build", ["build", "--release", "-p", "viewer"], Path("x"), True))
        self.assertEqual(bt.parse(["ui"]), ("ui", [], None, False))

    def test_refuses_what_the_tool_owns(self):
        for args in (["check", "-j", "3"], ["check", "-j3"], ["check", "--jobs=2"], ["check", "--target", "x"], ["check", "--target-dir=x"], ["check", "--message-format=json"], ["build", "--out", "x", "-p", "viewer"], ["build", "--release"], ["ui", "x"], ["test"], []):
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

    def test_failed_download_is_not_a_success(self):
        fake = self.use(FakeBoat([vm("a")]))
        fake.fail_downloads = True
        self.assertEqual(bt.main(["build", "--out", str(self.root / "out"), "--release", "-p", "viewer"]), bt.UNAVAILABLE)
        record = self.last_record()
        self.assertEqual((record["exit"], record["cargo_exit"], record["completed"], record["failure_phase"]), (bt.UNAVAILABLE, 0, False, "deliver"))
        self.assertNotIn("artifacts_verified", record)
        self.assertEqual(fake.ttl_calls("a")[-1], bt.IDLE_SECONDS)


if __name__ == "__main__":
    unittest.main()
