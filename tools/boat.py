"""Compile this worktree for Windows on a Boat VM (Python 3.11+, Windows).

    python tools/boat.py check -p <crate>... --all-targets
    python tools/boat.py check --workspace --all-targets
    python tools/boat.py ui
    python tools/boat.py build --out <dir> --release -p viewer --bin viewer
    python tools/boat.py reconcile

Run it inside the worktree. Its tracked and unignored files, uncommitted changes included, are
synchronized to a VM cloned from the `hoi4-check` image (tools/boat_setup.sh) and compiled with
cargo-xwin for x86_64-pc-windows-msvc. Nothing runs tests or Windows programs. Cargo's exit code
passes through (101 for compile errors); exit 75 means Boat failed, so run the work locally.

`build` downloads the named exe/dll/pdb outputs, checks their SHA-256 and records them with the
source digest in `<dir>/boat-artifacts.json`; `--refresh-image` then re-saves `hoi4-check` from
a clean worktree.

Worktrees share VMs, locks and `logs/timings.jsonl` under the main checkout's `captures/boat`
(BOAT_STATE_DIR overrides). A run keeps its VM alive with a short lease it renews, and leaves
it with a 15-minute idle tail, so the next run reuses it warm and an interrupted run strands it
for minutes, not hours.
`reconcile`, which every run also does, ends long lifetimes on VMs no run holds.
"""

from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
import msvcrt
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tarfile
import threading
import time
import uuid

TOOLS = Path(__file__).resolve().parent
SNAPSHOT = "hoi4-check"
REMOTE = "/home/user/hoi4"
TARGET = "x86_64-pc-windows-msvc"
RUN_SH = b'#!/usr/bin/env bash\nset -euo pipefail\nsource /home/user/hoi4/env.sh\nexec python3 /home/user/hoi4/remote.py "$@"'
LEASE_SECONDS = 900
RENEW_SECONDS = 240
# Owner, 26 Oct: a VM stays warm 15 min after its run, so the next one skips the multi-minute resume.
IDLE_SECONDS = 900
# Give slow resumed images up to 30 min to finish hydration.
HYDRATION_SECONDS = 1800
COOLDOWN_SECONDS = 1800
CREATE_SPACING_SECONDS = 5.1
UNAVAILABLE = 75
USABLE = {"ready", "idle", "running", "stopped"}
# The runner refuses to start on a VM holding another run's claim.
REMOTE_CLAIMED = 73


class Failure(Exception):
    pass


def say(text):
    print(text, file=sys.stderr, flush=True)


def atomic_json(path, data):
    temp = path.with_suffix("." + uuid.uuid4().hex + ".tmp")
    temp.write_text(json.dumps(data), encoding="utf-8")
    temp.replace(path)


class Lock:
    """An OS byte lock on a file in the shared state, released if this process dies."""

    def __init__(self, path, wait=False):
        self.file = path.open("a+b")
        self.file.seek(0)
        self.file.write(b"0")
        self.file.flush()
        self.file.seek(0)
        try:
            while True:
                try:
                    msvcrt.locking(self.file.fileno(), msvcrt.LK_NBLCK, 1)
                    break
                except OSError:
                    if not wait:
                        raise
                    time.sleep(0.2)
        except BaseException:
            self.file.close()
            raise

    def close(self):
        if self.file:
            self.file.seek(0)
            msvcrt.locking(self.file.fileno(), msvcrt.LK_UNLCK, 1)
            self.file.close()
            self.file = None

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()


def boat_exe():
    return os.environ.get("BOAT_EXE") or shutil.which("boat") or str(Path.home() / ".ascii" / "bin" / "boat.exe")


def boat(*args, check=True):
    result = subprocess.run([boat_exe(), "--json", *args], capture_output=True, text=True, encoding="utf-8", errors="replace")
    events = []
    for line in result.stdout.splitlines():
        try:
            events.append(json.loads(line))
        except ValueError:
            pass
    if check and result.returncode:
        raise Failure(f"Boat {args[0]} failed ({result.returncode}): {result.stderr.strip()} {result.stdout.strip()}")
    return result, events


def info(vmid):
    _, events = boat("info", vmid)
    return next(event["sandbox"] for event in events if "sandbox" in event)


def remote(vmid, *args, stream=False):
    command = [boat_exe(), "--json", "exec", vmid, "--cwd", REMOTE, "--timeout", "0", "--stream", "--", "bash", REMOTE + "/run.sh", *args]
    process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, encoding="utf-8", errors="replace")
    output, errors, exit_code = [], [], None
    for line in process.stdout:
        try:
            event = json.loads(line)
        except ValueError:
            if stream:
                say(line.rstrip())
            continue
        if event.get("type") in {"stdout", "stderr"}:
            data = event.get("data", "")
            (output if event["type"] == "stdout" else errors).append(data)
            if stream:
                print(data, end="", file=sys.stderr if event["type"] == "stderr" else sys.stdout, flush=True)
        elif event.get("type") == "exit":
            exit_code = event.get("exitCode")
            if exit_code:
                say("Boat remote exit: " + json.dumps(event))
        elif event.get("error"):
            say(str(event))
    process.wait()
    if exit_code is None:
        raise Failure(f"Boat exec lost its terminal result (CLI exit {process.returncode}); the remote claim stays so nothing overlaps a surviving job")
    if exit_code and errors and not stream:
        say("".join(errors).rstrip())
    return exit_code, "".join(output)


def git(worktree, *args):
    return subprocess.run(["git", "-C", str(worktree), *args], check=True, capture_output=True, text=True).stdout.strip()


class State:
    """The VM pool shared by every worktree of this repository."""

    def __init__(self, root):
        self.root = root
        for name in ("claims", "pool", "lanes", "jobs", "logs"):
            (root / name).mkdir(parents=True, exist_ok=True)
        self.claims, self.pool, self.lanes, self.jobs, self.logs = (root / name for name in ("claims", "pool", "lanes", "jobs", "logs"))

    @staticmethod
    def for_worktree(worktree):
        if os.environ.get("BOAT_STATE_DIR"):
            return State(Path(os.environ["BOAT_STATE_DIR"]))
        return State(Path(git(worktree, "rev-parse", "--path-format=absolute", "--git-common-dir")).parent / "captures" / "boat")

    def lock(self, vmid, wait=False):
        return Lock(self.claims / (vmid + ".lock"), wait)

    def allocator(self):
        return Lock(self.claims / "allocator.lock", wait=True)

    def registered(self):
        return sorted(path.stem for path in self.pool.glob("*.json"))

    def quarantined(self):
        return sorted(path.name.split(".")[0] for path in self.claims.glob("*.quarantined.json"))

    def cooling(self, vmid, now):
        path = self.claims / (vmid + ".cooldown.json")
        return path.exists() and json.loads(path.read_text()).get("until", 0) > now

    def cool(self, vmid, reason):
        atomic_json(self.claims / (vmid + ".cooldown.json"), {"until": time.time() + COOLDOWN_SECONDS, "reason": reason})

    def quarantine(self, vmid):
        with self.allocator():
            entry = self.pool / (vmid + ".json")
            if entry.exists():
                entry.replace(self.claims / (vmid + ".quarantined.json"))

    def lane_vm(self, lane):
        path = self.lanes / (lane + ".json")
        return json.loads(path.read_text()).get("id") if path.exists() else None


def rank(registered, states, lane_vm, size, cooling):
    """Usable VMs, best first: running before stopped, then the lane's own, then the right size."""
    ranked = []
    for vmid in registered:
        vm = states.get(vmid)
        # A stopping VM can take minutes to become resumable; another VM or a clone is quicker.
        if not vm or vm.get("state") not in USABLE or cooling(vmid):
            continue
        stopped = vm["state"] == "stopped"
        if size == "large" and vm.get("type") != "large" and not stopped:
            continue
        ranked.append((stopped, vmid != lane_vm, vm.get("type") != size, vmid))
    return [row[-1] for row in sorted(ranked)]


def resume_type(current, size):
    # Boat refuses to shrink a VM whose data does not fit, so only ever grow one.
    return "large" if size == "large" and current != "large" else None


def reconcile(state, states, held=()):
    """Shortens to the idle tail the lifetime of every pool VM that no run holds but still has a long one."""
    for vmid in sorted({*state.registered(), *state.quarantined()} - set(held)):
        vm = states.get(vmid)
        if not vm or vm.get("state") in {"stopped", "stopping"} or (vm.get("ttlSeconds") or 0) <= LEASE_SECONDS:
            continue
        try:
            lock = state.lock(vmid)
        except OSError:
            continue
        try:
            boat("extend", vmid, "--ttl", str(IDLE_SECONDS))
            say(f"{vmid}: no run holds it; lifetime shortened from {vm.get('ttlSeconds')} s to {IDLE_SECONDS} s")
        except Failure as error:
            say(f"WARNING: {vmid}: could not shorten its lifetime: {error}")
        finally:
            lock.close()


def listing():
    _, events = boat("list", "--all")
    return {vm["id"]: vm for event in events for vm in event.get("sandboxes", [])}


class Lease:
    """Keeps a VM alive only while this process renews it."""

    def __init__(self, vmid):
        self.vmid = vmid
        self.stopped = threading.Event()
        self.thread = threading.Thread(target=self.renew, daemon=True)
        self.thread.start()

    def renew(self):
        while not self.stopped.wait(RENEW_SECONDS):
            try:
                boat("extend", self.vmid, "--ttl", str(LEASE_SECONDS))
            except Failure as error:
                say(f"WARNING: {self.vmid}: lease renewal failed: {error}")

    def end(self):
        self.stopped.set()
        self.thread.join()
        try:
            boat("extend", self.vmid, "--ttl", str(IDLE_SECONDS))
        except Failure as error:
            say(f"WARNING: {self.vmid} stops when its {LEASE_SECONDS} s lease runs out: {error}")


def choose(state, lane, size):
    """Locks the best free VM, or clones a new one; returns (vmid, vm, lock, created)."""
    allocator = state.allocator()
    try:
        states = listing()
        reconcile(state, states)
        now = time.time()
        for vmid in rank(state.registered(), states, state.lane_vm(lane), size, lambda vmid: state.cooling(vmid, now)):
            try:
                return vmid, states[vmid], state.lock(vmid), False
            except OSError:
                continue
        say(f"No free prepared VM: cloning a {size} VM from {SNAPSHOT}")
        schedule = state.claims / "create-schedule.json"
        slot = max(now, json.loads(schedule.read_text()).get("next", now) if schedule.exists() else now)
        atomic_json(schedule, {"next": slot + CREATE_SPACING_SECONDS})
        allocator.close()
        if slot > now:
            time.sleep(slot - now)
        creation, events = boat("new", "--from", SNAPSHOT, "--type", size, "--name", "hoi4-check-pool-" + lane[:10], "--ttl", str(LEASE_SECONDS), "--no-env", check=False)
        vmid = next((event["id"] for event in events if event.get("event") == "created"), None)
        if creation.returncode or not vmid:
            if vmid:
                atomic_json(state.pool / (vmid + ".json"), {"id": vmid, "snapshot": SNAPSHOT})
                boat("extend", vmid, "--ttl", str(IDLE_SECONDS), check=False)
            raise Failure(f"Boat could not clone {SNAPSHOT} ({creation.returncode}), sandbox {vmid}: {creation.stderr.strip()}")
        lock = state.lock(vmid)
        atomic_json(state.pool / (vmid + ".json"), {"id": vmid, "snapshot": SNAPSHOT})
        return vmid, {"state": "ready", "type": size}, lock, True
    finally:
        allocator.close()


def wait_hydrated(vmid):
    # Boat reports ready before the clone's files are all present; cached mtimes must wait for them.
    deadline = time.monotonic() + HYDRATION_SECONDS
    while True:
        vm = info(vmid)
        if vm.get("hydrated") is not False:
            return vm
        if time.monotonic() >= deadline:
            raise Failure(f"{vmid}: filesystem hydration did not finish within {HYDRATION_SECONDS} s")
        time.sleep(3)


def acquire(state, lane, size, token, record):
    """Claims a VM locally and remotely; returns (vmid, lock, lease, remote source manifest)."""
    for _ in range(3):
        vmid, vm, lock, created = choose(state, lane, size)
        record.update(vm=vmid, vm_type=vm.get("type"), vm_state=vm.get("state"), created=created)
        say(f"Boat VM {vmid} ({vm.get('type')}, {vm.get('state')}), lane {lane[:10]}")
        lease = None
        try:
            if vm["state"] == "stopped":
                grow = resume_type(vm.get("type"), size)
                boat("resume", vmid, "--ttl", str(LEASE_SECONDS), *(["--type", grow] if grow else []))
                record["resumed"] = True
            elif not created:
                boat("extend", vmid, "--ttl", str(LEASE_SECONDS))
            lease = Lease(vmid)
            try:
                wait_hydrated(vmid)
            except Failure:
                state.cool(vmid, "hydration")
                raise
            status, manifest = remote(vmid, "acquire", token)
            if status == 0:
                return vmid, lock, lease, json.loads(manifest)
            if status != REMOTE_CLAIMED:
                remote(vmid, "release", token)
                raise Failure(f"{vmid}: initialization failed ({status}); see the remote output above")
            state.quarantine(vmid)
            say(f"{vmid} holds another run's remote claim; quarantined")
        except BaseException:
            if lease:
                lease.end()
            lock.close()
            raise
        lease.end()
        lock.close()
    raise Failure("Could not claim a VM after three attempts")


def source_manifest(worktree):
    names = subprocess.run(["git", "-C", str(worktree), "ls-files", "-z", "--cached", "--others", "--exclude-standard"], capture_output=True, check=True).stdout
    files = {}
    safe_dirs = {worktree}
    for raw in set(names.split(b"\0")) - {b""}:
        name = os.fsdecode(raw).replace("\\", "/")
        path = worktree / name
        try:
            status = path.lstat()
        except FileNotFoundError:
            continue
        if not stat.S_ISREG(status.st_mode) or getattr(status, "st_file_attributes", 0) & stat.FILE_ATTRIBUTE_REPARSE_POINT:
            raise Failure(f"Unsupported non-regular source file: {name}")
        parent = path.parent
        while parent not in safe_dirs:
            if parent.is_symlink() or getattr(parent.lstat(), "st_file_attributes", 0) & stat.FILE_ATTRIBUTE_REPARSE_POINT:
                raise Failure(f"Unsupported linked source directory: {parent}")
            safe_dirs.add(parent)
            parent = parent.parent
        files[name] = hashlib.sha256(path.read_bytes()).hexdigest()
    if "__boat_manifest__.json" in files:
        raise Failure("Reserved upload manifest filename is present in source")
    return files


def remove_stage(state, path):
    # Refuse links: the staging tree must never let removal reach outside jobs/.
    root = state.jobs.resolve()
    target = path.resolve()
    if root not in target.parents:
        say(f"Upload staging cleanup refused outside {root}: {target}")
        return
    for base, dirs, files in os.walk(target):
        for name in [*dirs, *files]:
            if (Path(base) / name).is_symlink() or getattr((Path(base) / name).lstat(), "st_file_attributes", 0) & stat.FILE_ATTRIBUTE_REPARSE_POINT:
                say(f"Upload staging cleanup refused: {Path(base) / name} is a link")
                return
    shutil.rmtree(target, ignore_errors=True)
    if target.exists():
        say(f"Upload staging left behind: {target}")


def runner_files():
    return {"remote.py": (TOOLS / "boat_remote.py").read_bytes().replace(b"\r\n", b"\n"), "run.sh": RUN_SH}


def install_runner(state, vmid, stage):
    files = runner_files()
    digest = hashlib.sha256(files["remote.py"] + files["run.sh"]).hexdigest()
    path = state.pool / (vmid + ".json")
    entry = json.loads(path.read_text())
    if entry.get("runner_hash") == digest:
        return
    for name, data in files.items():
        local = stage / ("runner-" + name)
        local.write_bytes(data)
        boat("scp", str(local), f"{vmid}:{REMOTE}/{name}")
    entry["runner_hash"] = digest
    atomic_json(path, entry)


def push_source(vmid, worktree, manifest, old, stage, token):
    changed = [name for name, digest in manifest.items() if old.get(name) != digest]
    say(f"Source: {len(manifest)} files; uploading {len(changed)} changed, removing {len(old.keys() - manifest.keys())}")
    upload = stage / "upload"
    upload.mkdir()
    for name in changed:
        destination = upload / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(worktree / name, destination)
    (upload / "__boat_manifest__.json").write_text(json.dumps(manifest), encoding="utf-8")
    # captures/ is ignored, and boat push sends only what Git would; give the upload its own index.
    subprocess.run(["git", "-c", "core.hideDotFiles=false", "init", "--quiet", str(upload)], check=True)
    names = [*changed, "__boat_manifest__.json"]
    index = b"".join(b"100644 e69de29bb2d1d6434b8b29ae775ad8c2e48c5391\t" + os.fsencode(name) + b"\0" for name in names)
    subprocess.run(["git", "-C", str(upload), "update-index", "--add", "--info-only", "-z", "--index-info"], input=index, check=True)
    incoming = f"{REMOTE}/incoming-{token}"
    _, pushed = boat("push", vmid, str(upload), "--to", incoming)
    if not any(event.get("files") == len(names) for event in pushed):
        raise Failure(f"Boat push did not confirm all {len(names)} files: {pushed}")
    return incoming


def deliver(vmid, token, output, outdir, stage):
    line = next((line for line in output.splitlines() if line.startswith("BOAT_ARTIFACTS=")), None)
    if not line:
        raise Failure("The build returned no artifact manifest")
    artifacts = json.loads(line.split("=", 1)[1])
    if not artifacts:
        raise Failure("The build produced no exe, dll or pdb")
    status, parts_json = remote(vmid, "split-bundle", token)
    if status:
        raise Failure("Could not split the artifact bundle for download")

    def fetch(item):
        index, source = item
        destination = stage / f"artifact-part-{index}"
        boat("scp", f"{vmid}:{source}", str(destination))
        return destination

    with ThreadPoolExecutor(max_workers=4) as downloads:
        parts = list(downloads.map(fetch, enumerate(json.loads(parts_json))))
    bundle = stage / "artifacts.tar.xz"
    with bundle.open("wb") as assembled:
        for part in parts:
            with part.open("rb") as source:
                shutil.copyfileobj(source, assembled)
    expected = {Path(artifact["path"]).name for artifact in artifacts}
    with tarfile.open(bundle, "r:xz") as archive:
        members = archive.getmembers()
        if {member.name for member in members} != expected or len(members) != len(expected) or any(not member.isfile() or Path(member.name).name != member.name for member in members):
            raise Failure("The artifact bundle holds unexpected names or entry types")
        for member in members:
            with archive.extractfile(member) as source, (outdir / member.name).open("wb") as destination:
                shutil.copyfileobj(source, destination)
    for artifact in artifacts:
        destination = outdir / Path(artifact["path"]).name
        digest = hashlib.sha256(destination.read_bytes()).hexdigest()
        if digest != artifact["sha256"]:
            raise Failure(f"Artifact checksum mismatch: {destination}")
        print(f"{destination}\nSHA-256 {digest}", flush=True)
    remote(vmid, "discard-bundle", token)
    return artifacts


def record_artifacts(outdir, artifacts, cargo, vmid, worktree, revision, source_digest):
    path = outdir / "boat-artifacts.json"
    previous = json.loads(path.read_text()) if path.exists() else {}
    combined = {}
    if previous.get("source_manifest_sha256") == source_digest:
        combined = {Path(item["path"]).name: item for item in previous.get("artifacts", [])}
    for item in artifacts:
        combined[Path(item["path"]).name] = {**item, "cargo_arguments": cargo}
    atomic_json(path, {"vm": vmid, "lane": str(worktree), "revision": revision, "cargo_arguments": cargo, "target": TARGET, "source_manifest_sha256": source_digest, "artifacts": list(combined.values())})


def refresh_image(vmid, token, worktree, revision):
    if git(worktree, "status", "--porcelain"):
        raise Failure("Refusing to save the shared image from a dirty worktree")
    for step in (["check", "check", "--workspace", "--all-targets"], ["ui"]):
        status, _ = remote(vmid, "acquire", token)
        if status:
            raise Failure("Could not reclaim the VM before saving the image")
        status, _ = remote(vmid, step[0], token, *step[1:], stream=True)
        if status:
            raise Failure(f"{' '.join(step)} failed; the image was not saved")
    status, _ = remote(vmid, "provenance")
    if status:
        raise Failure("Could not record the toolchain provenance")
    boat("exec", vmid, "--", "sh", "-c", f"echo {revision} > {REMOTE}/source-revision")
    boat("snapshot", vmid, SNAPSHOT)
    say(f"Saved {SNAPSHOT} from {revision}")


def parse(argv):
    """Returns (mode, cargo arguments, output directory, refresh image)."""
    if not argv or argv[0] not in {"check", "build", "ui", "reconcile"}:
        raise Failure("Usage: boat.py check <cargo args> | build --out <dir> [--refresh-image] --release <cargo args> | ui | reconcile")
    mode, rest = argv[0], list(argv[1:])
    outdir, refresh = None, False
    if mode in {"ui", "reconcile"}:
        if rest:
            raise Failure(f"{mode} takes no arguments")
        return mode, [], None, False
    if mode == "build":
        while rest[:1] in (["--out"], ["--refresh-image"]):
            if rest[0] == "--refresh-image":
                refresh = True
                rest.pop(0)
            elif len(rest) < 2:
                raise Failure("--out needs a directory")
            else:
                outdir = Path(rest[1])
                del rest[:2]
        if outdir is None or "--release" not in rest:
            raise Failure("build needs --out <dir> and --release")
    for arg in rest:
        if arg in {"--target", "--target-dir"} or arg.startswith(("--target=", "--target-dir=", "--message-format")):
            raise Failure("The target, target directory and message format belong to the tool")
        if arg in {"-j", "--jobs"} or arg.startswith(("--jobs=", "-j")):
            raise Failure("The VM sets the job count for its size; drop -j")
    return mode, [mode, *rest], outdir, refresh


def machine_size(mode, cargo):
    if os.environ.get("BOAT_MACHINE_TYPE"):
        if os.environ["BOAT_MACHINE_TYPE"] not in {"default", "large"}:
            raise Failure("BOAT_MACHINE_TYPE must be default or large")
        return os.environ["BOAT_MACHINE_TYPE"]
    packages = sum(arg in {"-p", "--package"} or arg.startswith("--package=") or (arg.startswith("-p") and len(arg) > 2) for arg in cargo)
    return "default" if mode == "ui" or (mode == "check" and packages == 1 and "--workspace" not in cargo) else "large"


def run(argv):
    started = time.monotonic()
    mode, cargo, outdir, refresh = parse(argv)
    worktree = Path(git(Path.cwd(), "rev-parse", "--show-toplevel")).resolve()
    state = State.for_worktree(worktree)
    if mode == "reconcile":
        with state.allocator():
            reconcile(state, listing())
        return 0
    if outdir:
        outdir = outdir.resolve()
        outdir.mkdir(parents=True, exist_ok=True)
    revision = git(worktree, "rev-parse", "HEAD")
    manifest = source_manifest(worktree)
    source_digest = hashlib.sha256(json.dumps(manifest, sort_keys=True).encode()).hexdigest()
    lane = hashlib.sha256(os.path.normcase(str(worktree)).encode()).hexdigest()
    token = uuid.uuid4().hex
    record = {"helper": "tools/boat.py", "mode": mode, "worktree": str(worktree), "revision": revision, "source_sha256": source_digest, "args": cargo, "vm": None, "phases": {}}
    phase = "allocate"
    mark = time.monotonic()

    def enter(name):
        nonlocal phase, mark
        record["phases"][phase] = round(time.monotonic() - mark, 2)
        phase, mark = name, time.monotonic()

    lock = lease = stage = vmid = None
    claimed = sync_started = sync_ok = running = False
    cargo_exit = None
    failure = None
    try:
        vmid, lock, lease, old = acquire(state, lane, machine_size(mode, cargo), token, record)
        claimed = True
        enter("sync")
        stage = state.jobs / token
        stage.mkdir()
        install_runner(state, vmid, stage)
        incoming = push_source(vmid, worktree, manifest, old, stage, token)
        sync_started = True
        status, _ = remote(vmid, "sync", token, incoming, stream=True)
        if status:
            raise Failure(f"Remote source synchronization failed ({status}); the claim stays to quarantine the partial tree")
        sync_ok = True
        atomic_json(state.lanes / (lane + ".json"), {"id": vmid, "worktree": str(worktree)})
        enter("run")
        running = True
        cargo_exit, output = remote(vmid, "ui" if mode == "ui" else mode, token, *([] if mode == "ui" else cargo), stream=True)
        claimed = False
        if mode == "build" and cargo_exit == 0:
            enter("deliver")
            artifacts = deliver(vmid, token, output, outdir, stage)
            record["artifacts_verified"] = True
            record_artifacts(outdir, artifacts, cargo, vmid, worktree, revision, source_digest)
            if refresh:
                enter("refresh")
                refresh_image(vmid, token, worktree, revision)
        return cargo_exit
    except BaseException as error:
        failure = (phase, "interrupted" if isinstance(error, KeyboardInterrupt) else str(error))
        raise
    finally:
        if vmid and claimed and not running and (not sync_started or sync_ok):
            try:
                remote(vmid, "release", token)
            except Failure as error:
                say(f"Remote claim kept: {error}")
        if lease:
            lease.end()
        if lock:
            lock.close()
        if stage:
            remove_stage(state, stage)
        enter("done")
        elapsed = time.monotonic() - started
        record.update(time=time.strftime("%Y-%m-%dT%H:%M:%S%z"), seconds=round(elapsed, 2), cargo_exit=cargo_exit)
        if failure is None:
            record.update(exit=cargo_exit, completed=True)
        else:
            record.update(exit=UNAVAILABLE, completed=False, failure_phase=failure[0], failure=failure[1])
        with (state.logs / "timings.jsonl").open("a", encoding="utf-8") as log:
            log.write(json.dumps(record) + "\n")
        say(f"Boat wall time {elapsed:.2f} s; cargo exit {cargo_exit}; VM {vmid or 'not allocated'}")


def main(argv):
    os.environ["BOAT_NO_UPDATE"] = "1"
    try:
        return run(argv)
    except KeyboardInterrupt:
        say("Interrupted; a surviving remote job keeps its claim, and the VM stops when its lease ends.")
        return 130
    except Exception as error:
        say(f"BOAT UNAVAILABLE: {error}\nRun the same work locally instead.")
        return UNAVAILABLE


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
