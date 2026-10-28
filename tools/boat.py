"""Compile this worktree for Windows on a Boat VM (Python 3.11+, Windows).

    python tools/boat.py check -p <crate>... --all-targets
    python tools/boat.py check --workspace --all-targets
    python tools/boat.py ui
    python tools/boat.py build --out <dir> --release -p viewer --bin viewer
    python tools/boat.py reconcile

Run it inside the worktree. Its tracked and unignored files, uncommitted changes included, are
synchronized to a VM cloned from the `hoi4-check` image (tools/boat_setup.sh) and compiled with
cargo-xwin for x86_64-pc-windows-msvc. Nothing runs tests or Windows programs. Cargo's exit code
passes through (101 for compile errors); exit 75 means Boat failed: run it again, never locally.

`build` downloads the named exe/dll/pdb outputs, checks their SHA-256 and records them with the
source digest in `<dir>/boat-artifacts.json`; `--refresh-image` then re-saves `hoi4-check` from
a clean worktree.
GUI builds with `custom-protocol` build and embed the current UI on the VM.

Worktrees share VMs, locks and `logs/timings.jsonl` under the main checkout's `captures/boat`
(BOAT_STATE_DIR overrides). A run keeps its VM alive with a short lease it renews, and leaves
it with a 5-minute idle tail, so the next run reuses it warm and an interrupted run strands it
for minutes, not hours.
`reconcile`, which every run also does, ends long lifetimes on VMs no run holds.
"""

import base64
from concurrent.futures import ThreadPoolExecutor
import hashlib
import io
import json
import msvcrt
import os
from pathlib import Path
import shutil
import signal
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
# Warm 5 min after a run, skipping the next run's multi-minute resume (27 Oct: 86% of reuses came within 5 min).
IDLE_SECONDS = 300
# Give slow resumed images up to 30 min to finish hydration.
HYDRATION_SECONDS = 1800
COOLDOWN_SECONDS = 1800
CREATE_SPACING_SECONDS = 5.1
UNAVAILABLE = 75
USABLE = {"ready", "idle", "running", "stopped"}
# The runner refuses to start on a VM holding another run's claim.
REMOTE_CLAIMED = 73
# Waits between attempts when the Boat API drops a connection (bursts of TLS handshake failures, 26 Oct).
RETRY_SECONDS = (2, 5, 10, 20, 40, 60)
# STATUS_DLL_INIT_FAILED: Windows could not start boat.exe under load, so nothing reached Boat.
LAUNCH_FAILED = 0xC0000142
IDEMPOTENT = {"list", "info", "extend", "scp", "push"}
# Uploads go as base64 command arguments, inside Windows' 32 767-character command line.
CHUNK_CHARS = 24000
# Up to about 560 KB compressed goes up in commands; more goes up with one `boat push`.
UPLOAD_CHUNKS = 32
UPLOAD_WORKERS = 8
# 4 MiB parts download as base64 command output; a `boat scp` part cost about 13 s before moving a byte.
DOWNLOAD_BYTES = 4 * 1024 * 1024
DOWNLOAD_WORKERS = 8
# The remote job's exit when the source could not be synchronized.
SYNC_FAILED = 79


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


def parse_events(stdout):
    events = []
    for line in stdout.splitlines():
        try:
            events.append(json.loads(line))
        except ValueError:
            pass
    return events


def transient(result, idempotent):
    # Nothing shows whether a failed create, resume, exec or snapshot took effect, so only those that never ran repeat.
    if result.returncode & 0xFFFFFFFF == LAUNCH_FAILED:
        return True
    text = result.stdout + result.stderr
    return idempotent and ("could not reach the Boat API" in text or '"status":5' in text)


class Held:
    """Holds Ctrl-C until the block ends, so a remote job is never left started or stopped halfway."""

    def __enter__(self):
        self.interrupted = False
        self.main = threading.current_thread() is threading.main_thread()
        if self.main:
            self.previous = signal.signal(signal.SIGINT, self.hold)
        return self

    def hold(self, *_):
        self.interrupted = True

    def __exit__(self, kind, error, trace):
        if self.main:
            signal.signal(signal.SIGINT, self.previous)
        if self.interrupted and kind is not None and not issubclass(kind, KeyboardInterrupt):
            raise KeyboardInterrupt from error


def boat(*args, check=True, idempotent=None, shielded=False):
    idempotent = args[0] in IDEMPOTENT if idempotent is None else idempotent
    # A shielded call runs in its own process group, so the console's Ctrl-C cannot cut its request short.
    flags = subprocess.CREATE_NEW_PROCESS_GROUP if shielded else 0
    for wait in (*RETRY_SECONDS, None):
        result = subprocess.run([boat_exe(), "--json", *args], capture_output=True, text=True, encoding="utf-8", errors="replace", creationflags=flags)
        events = parse_events(result.stdout)
        if not result.returncode or wait is None or not transient(result, idempotent):
            break
        say(f"Boat {args[0]} failed ({result.returncode}), retrying in {wait} s: {(result.stderr or result.stdout).strip()[:300]}")
        time.sleep(wait)
    if check and result.returncode:
        raise Failure(f"Boat {args[0]} failed ({result.returncode}): {result.stderr.strip()} {result.stdout.strip()}")
    return result, events


def info(vmid):
    _, events = boat("info", vmid)
    return next(event["sandbox"] for event in events if "sandbox" in event)


def attach(vmid, pid, seen, stream):
    """One `exec --attach` pass: the job's exit code (None if the stream dropped first) and this pass's whole replay."""
    process = subprocess.Popen([boat_exe(), "--json", "exec", vmid, "--attach", str(pid)], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, encoding="utf-8", errors="replace")
    start, replay, exit_code, headers = {"stdout": 0, "stderr": 0}, {"stdout": [], "stderr": []}, None, 0
    try:
        for line in process.stdout:
            try:
                event = json.loads(line)
            except ValueError:
                if stream:
                    say(line.rstrip())
                continue
            kind = event.get("type")
            if kind == "attached":
                # The CLI reconnects on its own with a header at the current offsets; one that skips or rewinds breaks the replay.
                offsets = {"stdout": event.get("stdoutOffset", 0), "stderr": event.get("stderrOffset", 0)}
                if offsets != (start if headers else {"stdout": 0, "stderr": 0}):
                    replay = None
                start, headers = offsets, headers + 1
            elif kind in seen:
                text, end = event.get("data", ""), event["nextOffset"]
                begin, start[kind] = start[kind], end
                if replay is not None:
                    replay[kind].append(text)
                if end <= seen[kind]:
                    continue
                if begin < seen[kind]:
                    raw = text.encode("utf-8")
                    if len(raw) == end - begin:
                        text = raw[seen[kind] - begin:].decode("utf-8", errors="replace")
                    else:
                        # The CLI replaced undecodable bytes, so the overlap cannot be cut exactly: repeat it, never drop it.
                        say(f"Boat attach: up to {seen[kind] - begin} bytes of {kind} repeat below")
                seen[kind] = end
                if stream:
                    print(text, end="", file=sys.stderr if kind == "stderr" else sys.stdout, flush=True)
            elif kind == "exit":
                if event.get("exitCode") is None:
                    raise Failure(f"Boat remote job {pid} on {vmid} ended without an exit code: {json.dumps(event)}")
                exit_code = event["exitCode"]
                if exit_code:
                    say("Boat remote exit: " + json.dumps(event))
            elif event.get("error"):
                say(str(event))
    finally:
        if process.poll() is None:
            process.kill()
        process.wait()
    return exit_code, replay


def stop(vmid, pid):
    # A detached job outlives Ctrl-C; the foreground exec it replaced died with its caller, so end it the same way.
    with Held():
        boat("exec", vmid, "--kill", str(pid), check=False, idempotent=True, shielded=True)
        for wait in (*RETRY_SECONDS, None):
            _, events = boat("exec", vmid, "--status", str(pid), check=False, idempotent=True, shielded=True)
            if any(event.get("pid") == pid and event.get("running") is False for event in events):
                return
            if wait is None:
                break
            time.sleep(wait)
    say(f"WARNING: Boat job {pid} on {vmid} may still be running; its remote claim stays so nothing overlaps it")


def remote(vmid, *args, stream=False):
    # Detached, the job outlives a dropped connection; each attach replays from the start, so offsets skip what was seen.
    # Detached exec takes --cwd relative to the sandbox's work directory, so the job changes directory itself.
    pid, seen, exit_code, replay = None, {"stdout": 0, "stderr": 0}, None, None
    try:
        with Held() as held:
            _, events = boat("exec", vmid, "--detach", "--", "bash", "-c", 'cd "$0" && exec bash run.sh "$@"', REMOTE, *args, shielded=True)
            pid = next((event["pid"] for event in events if "pid" in event), None)
        if held.interrupted:
            raise KeyboardInterrupt
        if pid is None:
            raise Failure(f"Boat exec --detach on {vmid} returned no process id")
        for wait in (*RETRY_SECONDS, None):
            exit_code, replay = attach(vmid, pid, seen, stream)
            if exit_code is not None or wait is None:
                break
            say(f"Boat attach to job {pid} on {vmid} dropped; re-attaching in {wait} s")
            time.sleep(wait)
    except KeyboardInterrupt:
        if pid is None:
            say(f"WARNING: interrupted while starting a Boat job on {vmid}; if it started, its remote claim stays so nothing overlaps it")
        else:
            try:
                stop(vmid, pid)
            except Exception as error:
                say(f"WARNING: could not stop Boat job {pid} on {vmid} ({error}); its remote claim stays so nothing overlaps it")
        raise
    if exit_code is None:
        raise Failure(f"Boat lost job {pid} on {vmid} after {len(RETRY_SECONDS) + 1} attaches; the remote claim stays so nothing overlaps it")
    # Callers parse what is returned, so it must be the whole log from offset 0; the displayed text may repeat or be cut.
    if replay is None:
        raise Failure(f"Boat replay of job {pid} on {vmid} was not one unbroken log from offset 0, so its output cannot be trusted")
    stdout, stderr = "".join(replay["stdout"]), "".join(replay["stderr"])
    if exit_code and stderr and not stream:
        say(stderr.rstrip())
    return exit_code, stdout


def quick(vmid, *args):
    """A short runner step in the foreground, about a second against four detached; returns (exit code, stdout).

    Only steps safe to repeat use it: a repeated acquire takes over the claim its lost reply left.
    """
    result, events = boat("exec", vmid, "--timeout", "120", "--", "bash", "-c", 'cd "$0" && exec bash run.sh "$@"', REMOTE, *args, check=False, idempotent=True)
    event = next((event for event in events if "exitCode" in event), None)
    if event is None or event["exitCode"] is None:
        raise Failure(f"Boat exec on {vmid} returned no exit code ({result.returncode}): {(result.stderr or result.stdout).strip()[:300]}")
    if event["exitCode"] and event.get("stderr"):
        say(event["stderr"].rstrip())
    return event["exitCode"], event.get("stdout", "")


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

    def entry(self, vmid):
        path = self.pool / (vmid + ".json")
        try:
            return json.loads(path.read_text())
        except (FileNotFoundError, ValueError):
            return {}

    def update(self, vmid, **fields):
        # Only the holder of the VM's lock writes its entry.
        if (self.pool / (vmid + ".json")).exists():
            atomic_json(self.pool / (vmid + ".json"), {**self.entry(vmid), **fields})


def rank(registered, states, lane_vm, size, cooling, used=lambda vmid: 0):
    """Usable VMs, best first: running before stopped, then the lane's own, then the right size, then the least recently used."""
    ranked = []
    for vmid in registered:
        vm = states.get(vmid)
        # A stopping VM can take minutes to become resumable; another VM or a clone is quicker.
        if not vm or vm.get("state") not in USABLE or cooling(vmid):
            continue
        stopped = vm["state"] == "stopped"
        if size == "large" and vm.get("type") != "large" and not stopped:
            continue
        # Another lane's VM holds that lane's build cache: take the one whose lane has been away longest.
        ranked.append((stopped, vmid != lane_vm, vm.get("type") != size, used(vmid), vmid))
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


def choose(state, lane, size, passed=()):
    """Locks the best free VM not passed by, or clones a new one; returns (vmid, vm, lock, created)."""
    allocator = state.allocator()
    try:
        states = listing()
        reconcile(state, states)
        now = time.time()
        registered = [vmid for vmid in state.registered() if vmid not in passed]
        for vmid in rank(registered, states, state.lane_vm(lane), size, lambda vmid: state.cooling(vmid, now), lambda vmid: state.entry(vmid).get("used", 0)):
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
                # Shorten before publishing: a run that acquires it afterwards keeps its own full lease.
                boat("extend", vmid, "--ttl", str(IDLE_SECONDS), check=False)
                atomic_json(state.pool / (vmid + ".json"), {"id": vmid, "snapshot": SNAPSHOT})
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
    passed = set()
    for _ in range(3):
        vmid, vm, lock, created = choose(state, lane, size, passed)
        record.update(vm=vmid, vm_type=vm.get("type"), vm_state=vm.get("state"), created=created)
        say(f"Boat VM {vmid} ({vm.get('type')}, {vm.get('state')}), lane {lane[:10]}")
        lease = None
        try:
            started = time.monotonic()
            if vm["state"] == "stopped":
                grow = resume_type(vm.get("type"), size)
                boat("resume", vmid, "--ttl", str(LEASE_SECONDS), *(["--type", grow] if grow else []))
                record["resumed"] = True
            elif not created:
                boat("extend", vmid, "--ttl", str(LEASE_SECONDS))
            lease = Lease(vmid)
            # A running VM the listing shows hydrated stays so; a resumed or cloned one hydrates again.
            if vm["state"] == "stopped" or created or vm.get("hydrated") is False:
                try:
                    wait_hydrated(vmid)
                except Failure:
                    state.cool(vmid, "hydration")
                    raise
            record["ready_seconds"] = round(time.monotonic() - started, 2)
            # Before the claim, so every VM decides claims by this runner's rules.
            install_runner(state, vmid)
            status, manifest = quick(vmid, "acquire", token)
            if status == 0:
                return vmid, lock, lease, json.loads(manifest)
            if status != REMOTE_CLAIMED:
                quick(vmid, "release", token)
                raise Failure(f"{vmid}: initialization failed ({status}); see the remote output above")
            if manifest.strip() == "busy":
                passed.add(vmid)
                say(f"{vmid} is still finishing a job whose run ended; passing it by")
            else:
                state.quarantine(vmid)
                say(f"{vmid} holds processes a job could not stop; quarantined")
        except BaseException:
            if lease:
                lease.end()
            lock.close()
            raise
        lease.end()
        lock.close()
    raise Failure("Could not claim a VM after three attempts")


def source_manifest(worktree, cache=None):
    """Hashes of the files Git would send; `cache` keeps them for files whose size and mtime stay put."""
    names = subprocess.run(["git", "-C", str(worktree), "ls-files", "-z", "--cached", "--others", "--exclude-standard"], capture_output=True, check=True).stdout
    try:
        known = json.loads(cache.read_text()) if cache else {}
    except (FileNotFoundError, ValueError):
        known = {}
    # As Git does, a hash is kept only for a file last written well before it was hashed, so a rewrite within one timestamp tick is seen.
    settled = time.time_ns() - 2_000_000_000
    files, kept = {}, {}
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
        stamp = [status.st_size, status.st_mtime_ns]
        previous = known.get(name)
        files[name] = previous[2] if previous and previous[:2] == stamp else hashlib.sha256(path.read_bytes()).hexdigest()
        if status.st_mtime_ns < settled:
            kept[name] = [*stamp, files[name]]
    if cache:
        cache.parent.mkdir(parents=True, exist_ok=True)
        atomic_json(cache, kept)
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


def upload(vmid, name, data):
    """Writes base64 parts of data to REMOTE/<name>.b64-NNN, several commands at a time."""
    text = base64.b64encode(data).decode()
    chunks = [text[start:start + CHUNK_CHARS] for start in range(0, len(text), CHUNK_CHARS)] or [""]

    def put(item):
        index, chunk = item
        boat("exec", vmid, "--timeout", "60", "--", "sh", "-c", 'printf %s "$1" > "$0"', f"{REMOTE}/{name}.b64-{index:03d}", chunk, idempotent=True)

    with ThreadPoolExecutor(max_workers=UPLOAD_WORKERS) as uploads:
        list(uploads.map(put, enumerate(chunks)))


def install_runner(state, vmid):
    files = runner_files()
    digest = hashlib.sha256(files["remote.py"] + files["run.sh"]).hexdigest()
    if state.entry(vmid).get("runner_hash") == digest:
        return
    steps = []
    for name, data in files.items():
        upload(vmid, "runner-" + name, data)
        steps.append(f'cat runner-{name}.b64-* | base64 -d > {name}.new && echo "{hashlib.sha256(data).hexdigest()}  {name}.new" | sha256sum -c --quiet && mv {name}.new {name} && rm runner-{name}.b64-*')
    boat("exec", vmid, "--timeout", "60", "--", "sh", "-c", f"cd {REMOTE} && " + " && ".join(steps), idempotent=True)
    state.update(vmid, runner_hash=digest)


def send_source(vmid, worktree, manifest, old, stage, token):
    """Sends the files that differ from the VM's tree; returns how the remote job finds them."""
    changed = [name for name, digest in manifest.items() if old.get(name) != digest]
    deleted = sorted(old.keys() - manifest.keys())
    say(f"Source: {len(manifest)} files; uploading {len(changed)} changed, removing {len(deleted)}")
    bundle = io.BytesIO()
    with tarfile.open(fileobj=bundle, mode="w:xz", preset=2) as archive:
        for name in changed:
            data = (worktree / name).read_bytes()
            # A file edited since it was hashed is sent as read, so the remote checksums always match.
            manifest[name] = hashlib.sha256(data).hexdigest()
            entry = tarfile.TarInfo("files/" + name)
            entry.size = len(data)
            archive.addfile(entry, io.BytesIO(data))
        delta = json.dumps({"changed": {name: manifest[name] for name in changed}, "deleted": deleted, "digest": manifest_digest(manifest)}).encode()
        entry = tarfile.TarInfo("delta.json")
        entry.size = len(delta)
        archive.addfile(entry, io.BytesIO(delta))
    data = bundle.getvalue()
    digest = hashlib.sha256(data).hexdigest()
    name = f"incoming-{token}.tar.xz"
    # Each `boat push` or `scp` costs 13-17 s however small; a few kilobytes per exec go up in parallel in about two.
    if len(data) * 4 // 3 <= UPLOAD_CHUNKS * CHUNK_CHARS:
        upload(vmid, name, data)
        return f"parts:{name}:{digest}"
    folder = stage / "upload"
    folder.mkdir()
    (folder / name).write_bytes(data)
    # captures/ is ignored, and boat push sends only what Git would; give the upload its own index.
    subprocess.run(["git", "-c", "core.hideDotFiles=false", "init", "--quiet", str(folder)], check=True)
    index = b"100644 e69de29bb2d1d6434b8b29ae775ad8c2e48c5391\t" + os.fsencode(name) + b"\0"
    subprocess.run(["git", "-C", str(folder), "update-index", "--add", "--info-only", "-z", "--index-info"], input=index, check=True)
    _, pushed = boat("push", vmid, str(folder), "--to", f"{REMOTE}/incoming-{token}")
    if not any(event.get("files") == 1 for event in pushed):
        raise Failure(f"Boat push did not confirm the source bundle: {pushed}")
    return f"file:incoming-{token}/{name}:{digest}"


def manifest_digest(manifest):
    return hashlib.sha256(json.dumps(manifest, sort_keys=True).encode()).hexdigest()


def deliver(vmid, output, outdir):
    lines = {key: value for key, _, value in (line.partition("=") for line in output.splitlines()) if key in {"BOAT_ARTIFACTS", "BOAT_BUNDLE"}}
    if len(lines) != 2:
        raise Failure("The build returned no artifact manifest")
    artifacts = json.loads(lines["BOAT_ARTIFACTS"])
    if not artifacts:
        raise Failure("The build produced no exe, dll or pdb")
    bundle = json.loads(lines["BOAT_BUNDLE"])

    def fetch(index):
        # An exec returns at most 8 MiB of output: a part's base64 stays under it.
        _, events = boat("exec", vmid, "--timeout", "120", "--", "sh", "-c", 'dd if="$0" bs="$1" skip="$2" count=1 status=none | base64 -w0', bundle["path"], str(DOWNLOAD_BYTES), str(index), idempotent=True)
        data = base64.b64decode("".join(event.get("stdout", "") for event in events))
        if len(data) != min(DOWNLOAD_BYTES, bundle["size"] - index * DOWNLOAD_BYTES):
            raise Failure(f"Artifact bundle part {index} arrived with {len(data)} bytes")
        return data

    with ThreadPoolExecutor(max_workers=DOWNLOAD_WORKERS) as downloads:
        data = b"".join(downloads.map(fetch, range(max(1, -(-bundle["size"] // DOWNLOAD_BYTES)))))
    if hashlib.sha256(data).hexdigest() != bundle["sha256"]:
        raise Failure("Artifact bundle checksum mismatch")
    expected = {Path(artifact["path"]).name for artifact in artifacts}
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:xz") as archive:
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
        status, _ = quick(vmid, "acquire", token)
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
    lane = hashlib.sha256(os.path.normcase(str(worktree)).encode()).hexdigest()
    manifest = source_manifest(worktree, state.root / "hashes" / (lane + ".json"))
    source_digest = manifest_digest(manifest)
    token = uuid.uuid4().hex
    record = {"helper": "tools/boat.py", "mode": mode, "worktree": str(worktree), "revision": revision, "source_sha256": source_digest, "args": cargo, "vm": None, "phases": {}}
    phase = "allocate"
    mark = time.monotonic()

    def enter(name):
        nonlocal phase, mark
        record["phases"][phase] = round(time.monotonic() - mark, 2)
        phase, mark = name, time.monotonic()

    lock = lease = stage = vmid = None
    claimed = running = False
    cargo_exit = None
    failure = None
    try:
        stage = state.jobs / token
        stage.mkdir()
        vmid, lock, lease, old = acquire(state, lane, machine_size(mode, cargo), token, record)
        claimed = True
        enter("sync")
        source = send_source(vmid, worktree, manifest, old, stage, token)
        source_digest = record["source_sha256"] = manifest_digest(manifest)
        enter("run")
        running = True
        # The job synchronizes the tree first; it releases its claim whether that or Cargo fails.
        status, output = remote(vmid, "ui" if mode == "ui" else mode, token, "--sync", source, *([] if mode == "ui" else cargo), stream=True)
        claimed = False
        if status == SYNC_FAILED:
            raise Failure("Remote source synchronization failed; see the remote output above")
        cargo_exit = status
        atomic_json(state.lanes / (lane + ".json"), {"id": vmid, "worktree": str(worktree)})
        if mode == "build" and cargo_exit == 0:
            enter("deliver")
            artifacts = deliver(vmid, output, outdir)
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
        # A job whose result was lost may still run, so it keeps the claim.
        if vmid and claimed and not running:
            try:
                quick(vmid, "release", token)
            except Failure as error:
                say(f"Remote claim kept: {error}")
        if lease:
            lease.end()
        if lock:
            state.update(vmid, used=time.time())
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
    for stream in (sys.stdout, sys.stderr):
        if hasattr(stream, "reconfigure"):
            stream.reconfigure(errors="replace")
    os.environ["BOAT_NO_UPDATE"] = "1"
    try:
        return run(argv)
    except KeyboardInterrupt:
        say("Interrupted; a surviving remote job keeps its claim, and the VM stops when its lease ends.")
        return 130
    except Exception as error:
        say(f"BOAT UNAVAILABLE: {error}\nRun it on Boat again; never compile locally.")
        return UNAVAILABLE


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
