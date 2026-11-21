"""Test `main` once a day on this machine (Python 3.11+, Windows).

    python tools/daily_tests.py install --ci-dir <runner folder> --game-dir <the game folder> --fixture <recording folder> --recorded-replies <file> [--at 03:00]
    python tools/daily_tests.py run --ci-dir <runner folder> --game-dir <the game folder> --fixture <recording folder> --recorded-replies <file>

`--ci-dir` (or BATTLE_PLAN_CI) names the runner's own folder: its worktree is main there, the
acceptance app is built into e2e-app beside it, and CAPTURES is captures there unless
BATTLE_PLAN_CAPTURES names another. `install` makes that worktree of origin/main, or moves an
existing one of this clone there and refuses any other folder, and registers the Windows task that runs it daily at `--at`, or as soon as it can after a missed
start. `run` moves that worktree to the newest origin/main and holds TESTS_RUNNING while it runs, in GROUPS, the repository checks, the UI
typecheck and tests, every Rust test and the acceptance suite (tools/e2e/suite.py: the release
app, built on Boat, on a copy of the `--fixture` recording). RUST_BUILD says where the Rust tests
compile: "boat" builds them on Boat and runs them here (tools/boat.py test); "local" runs nextest
here, without incremental compilation, and once the build finishes removes from its Cargo target
whatever that build neither produced nor used, so the target holds one build. Either runs only
with DISK_FLOOR free, removing the runner's own Cargo target first when that brings it there.
Rust tests are counted passed, failed, skipped or missing (tools/ci_results.py). `--recorded-replies` names the replies the acceptance sweep
recorded (`--record-replies`) from the `--fixture` recording, which the browser tests replay
(BATTLE_PLAN_RECORDED_REPLIES). Its report goes to CAPTURES\\ci: latest.md, rewritten
as each group ends, a folder per run with each step's log, kept 30 days, and history.tsv, kept
whole. The acceptance suite's report is e2e/e2e.md in the run's folder; its visual baseline and
last work stay in E2E_STATE.

On GitHub, through tools/ci_results.py, each group's result is the commit status
`STATUS/<group>` on the tested commit as the group ends, the whole run's is `STATUS`, and one
issue stays open while runs fail. A run that never reported leaves TESTS_RUNNING behind; the
next run records it in history.tsv and on GitHub as unfinished.
"""

import argparse
import datetime
import json
import msvcrt
import os
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import time
import tomllib
import traceback
from pathlib import Path
from xml.sax.saxutils import escape

import ci_results

ROOT = Path(__file__).resolve().parents[1]
WORKTREE = CAPTURES = REPORTS = MARKER = E2E_APP = E2E_STATE = None
TASK = "Battle Plan daily tests"
DEADLINE = datetime.timedelta(hours=18)
GIT_TIMEOUT = 600
CLEANUP = 120
REPORT_MARGIN = 2 * CLEANUP + 60
NO_WINDOW = subprocess.CREATE_NO_WINDOW
KEEP = datetime.timedelta(days=30)
RUN_FOLDER = re.compile(r"^\d{4}-\d{2}-\d{2}-\d{4}-[0-9a-f]{10}(?:-\d+)?$")
RUN_MARK = "daily-tests-run.tsv"
OPEN_GROUPS = "open-groups.json"
OUTCOME = "outcome.json"
CHECKOUT_LOCK = "checkout.lock"
STATUS = "daily-tests"
ISSUE_TITLE = "Daily tests are failing on main"
GB = 2**30
DISK_FLOOR = 60 * GB
UNIT_HASH = re.compile(r"-([0-9a-f]{16})(?=\.|$)")
UNIT_FOLDER = re.compile(r"^(.+)-([0-9a-f]{16})$")
UNHASHED_EXTENSIONS = re.compile(r"[A-Za-z]+(?:\.[A-Za-z]+)?")


RUST_BUILD = "boat"
BOAT_FAILED = 75
BOAT_RUST_LIMIT = datetime.timedelta(hours=6)
NEXTEST_PROFILE = "daily"
RUST_LIST = [
    "cargo", "nextest", "list", "--workspace", "--profile", NEXTEST_PROFILE, "--message-format", "json",
]
RUST_TESTS = [
    "cargo", "nextest", "run", "--workspace", "--profile", NEXTEST_PROFILE, "--no-fail-fast",
    "--final-status-level", "fail", "--cargo-message-format", "json-render-diagnostics",
]


def steps():
    """Each step: its name, its log's name, its command or a function of the run's `Build` and the
    log giving its exit code and result, and the step or steps it needs to have passed. A group's
    steps are together, the quick groups first, so their results come before the long ones'."""
    npm = shutil.which("npm.cmd") or "npm.cmd"
    python = str(Path(sys.executable).with_name("python.exe"))
    check = lambda script: [python, f"tools/{script}.py"]
    return [
        ("Build dependencies", "build-dependencies", check("check-build-dependencies"), None),
        ("Content dependencies", "content-dependencies", check("check-content-dependencies"), None),
        ("Feature unification", "feature-unification", check("check-feature-unification"), None),
        ("Nextest config", "nextest-config", check("check-nextest-config"), None),
        ("Machine paths", "machine-paths", check("check-machine-paths"), None),
        ("Citations", "citations", check("check-citations"), None),
        ("Agent context", "agent-context", check("check-agent-context"), None),
        ("UI install", "ui-install", [npm, "ci"], None),
        ("UI typecheck", "ui-check", [npm, "run", "check:ui"], "ui-install"),
        ("UI tests", "ui-tests", [npm, "run", "test:ui"], "ui-install"),
        ("Disk space", "disk", check_disk, None),
        *([
            ("Rust tests", "rust", rust_tests, "disk"),
            ("Rust target sweep", "rust-sweep", sweep_target, "rust-build"),
        ] if RUST_BUILD == "local" else [
            ("Rust tests on Boat", "rust", boat_rust_tests, "disk"),
        ]),
        ("Acceptance app build", "e2e-build", [python, "tools/e2e/suite.py", "build", "--out", str(E2E_APP)], None),
        ("Acceptance suite", "e2e", acceptance, ("e2e-build", "ui-install")),
    ]


GROUPS = {
    "repository": ("build-dependencies", "content-dependencies", "feature-unification", "nextest-config",
                   "machine-paths", "citations", "agent-context"),
    "ui": ("ui-install", "ui-check", "ui-tests"),
    "rust": ("disk", "rust", "rust-sweep"),
    "acceptance": ("e2e-build", "e2e"),
}


def bounded(command, stdout, stderr, seconds, environment=None):
    """Run `command` into open files, never pipes; its exit code, or None when it ran out of
    time and its process tree was stopped."""
    process = subprocess.Popen(
        command,
        cwd=WORKTREE,
        stdout=stdout,
        stderr=stderr,
        env=environment,
        creationflags=NO_WINDOW,
    )
    try:
        return process.wait(timeout=max(seconds, 1))
    except subprocess.TimeoutExpired:
        try:
            subprocess.run(
                ["taskkill", "/T", "/F", "/PID", str(process.pid)],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                creationflags=NO_WINDOW,
                timeout=CLEANUP,
            )
            process.wait(timeout=CLEANUP)
        except subprocess.TimeoutExpired:
            pass
        return None


def git(*args):
    with tempfile.TemporaryFile() as stdout, tempfile.TemporaryFile() as stderr:
        code = bounded(["git", "-C", str(WORKTREE), *args], stdout, stderr, GIT_TIMEOUT)
        read = lambda output: (output.seek(0), output.read())[1].decode(errors="replace").strip()
        if code is None:
            raise RuntimeError(f"git {' '.join(args)}: timed out")
        if code != 0:
            raise RuntimeError(f"git {' '.join(args)}: {read(stderr)}")
        return read(stdout)


def lock_runner(name="runner.lock"):
    """Lock out other runs for this process's life; None while another run holds the lock."""
    REPORTS.mkdir(parents=True, exist_ok=True)
    descriptor = os.open(REPORTS / name, os.O_RDWR | os.O_CREAT)
    try:
        msvcrt.locking(descriptor, msvcrt.LK_NBLCK, 1)
    except OSError:
        os.close(descriptor)
        return None
    return descriptor


def hold_checkout():
    """The checked-out runner's hold on the worktree for its life, or None while another holds it:
    a runner whose parent died must not see the next run move its checkout."""
    return lock_runner(CHECKOUT_LOCK)


def checkout_free():
    """Whether no checked-out runner holds the worktree."""
    held = lock_runner(CHECKOUT_LOCK)
    if held is None:
        return False
    os.close(held)
    return True


def duration(seconds):
    return f"{seconds / 60:.0f} min" if seconds >= 60 else f"{seconds:.0f} s"


def report_text(heading, lines):
    return "\n".join([f"# Daily tests: {heading}", "", *lines, ""])


def write_report(heading, lines):
    text = report_text(heading, lines)
    temporary = REPORTS / "latest.md.tmp"
    temporary.write_text(text, encoding="utf-8")
    os.replace(temporary, REPORTS / "latest.md")
    return text


def record_history(started, commit, result, failed):
    """One line per run; the failing test count is blank when no test results were read."""
    with open(REPORTS / "history.tsv", "a", encoding="utf-8") as history:
        count = "" if failed is None else failed
        history.write(f"{started:%Y-%m-%d %H:%M}\t{(commit or '')[:10]}\t{result}\t{count}\n")


def write_marker(text):
    """Replaces TESTS_RUNNING whole, so an interrupted rewrite keeps the run it named."""
    (REPORTS / (MARKER.name + ".tmp")).write_text(text, encoding="utf-8")
    os.replace(REPORTS / (MARKER.name + ".tmp"), MARKER)


def run_step(command, log, environment, seconds, stdout=None):
    """The step's exit code, or None when it ran out of time and was stopped. Its stdout goes to
    `stdout` when given, else to its log with its stderr."""
    with open(log, "wb") as output:
        errors = subprocess.STDOUT if stdout is None else output
        return bounded(command, stdout or output, errors, seconds, environment)


def configured_target():
    """The runner worktree's own Cargo target: its config's build.target-dir, else `target`."""
    try:
        with open(WORKTREE / ".cargo" / "config.toml", "rb") as config:
            configured = tomllib.load(config).get("build", {}).get("target-dir")
    except FileNotFoundError:
        configured = None
    return WORKTREE / (configured or "target")


def target_dir():
    """The Cargo target a local build uses: CARGO_TARGET_DIR, else the worktree's own."""
    configured = os.environ.get("CARGO_TARGET_DIR")
    return WORKTREE / configured if configured else configured_target()


def rust_target():
    """Where the Rust tests' files go: Boat's test programs under the worktree's own `.cargo-target`
    (tools/boat.py test), else the Cargo target."""
    return WORKTREE / ".cargo-target" if RUST_BUILD == "boat" else target_dir()


def nextest_junit():
    """Where nextest writes the daily profile's JUnit report: its store under the workspace root."""
    try:
        with open(WORKTREE / ".config" / "nextest.toml", "rb") as config:
            store = tomllib.load(config).get("store", {}).get("dir")
    except (OSError, tomllib.TOMLDecodeError):
        store = None
    return WORKTREE / (store or "target/nextest") / NEXTEST_PROFILE / "junit.xml"


def size_of(folder):
    """Bytes under `folder`, a hard-linked file once (Cargo links the binaries it copies up), or
    None when it does not exist."""
    if not folder.is_dir():
        return None
    total, seen = 0, set()
    for directory, _, files in os.walk(folder):
        for name in files:
            try:
                info = os.lstat(os.path.join(directory, name))
            except OSError:
                continue
            if info.st_nlink > 1:
                if (info.st_dev, info.st_ino) in seen:
                    continue
                seen.add((info.st_dev, info.st_ino))
            total += info.st_size
    return total


def gigabytes(size):
    return "absent" if size is None else f"{size / GB:.1f} GB"


class Build:
    """What the steps share: the Rust target, the `Units` of its build once it finished, the counted
    Rust `ci_results.Tests` once they ran, and the run's folder, environment, deadline and
    acceptance fixture."""

    def __init__(self, target, folder=None, environment=None, deadline=None, fixture=None):
        self.target = target
        self.units = self.tests = None
        self.folder, self.environment, self.deadline, self.fixture = folder, environment, deadline, fixture


def acceptance(build, log):
    """The acceptance suite on the night's app build; its report goes to the run's folder."""
    if not build.fixture:
        log.write_text("No fixture: install the task with --fixture.\n", encoding="utf-8")
        return 1, "failed: no fixture configured (install with --fixture)"
    out = build.folder / "e2e"
    command = [
        str(Path(sys.executable).with_name("python.exe")), "tools/e2e/suite.py", "run", "--app", str(E2E_APP),
        "--fixture", build.fixture, "--game-dir", build.environment["BATTLE_PLAN_GAME_DIR"], "--out", str(out),
        "--state", str(E2E_STATE),
    ]
    code = run_step(command, log, build.environment, build.deadline - time.monotonic())
    try:
        report = json.loads((out / "e2e.json").read_text(encoding="utf-8"))
    except (OSError, ValueError):
        report = None
    if code is None:
        return 1, "stopped: out of time"
    if report is None or code not in (0, 1):
        return 1, f"failed (exit {code}): the suite could not run"
    if report["passed"] and code == 0:
        return 0, "passed: e2e/e2e.md"
    return 1, f"failed: {report['failed']} failing rows, e2e/e2e.md"


LOCKS = {".boat-test.lock", ".cargo-lock"}


def not_owned(target):
    """Why `target` is not the runner's to remove, or None: it must be the runner worktree's own
    Cargo target, a plain folder holding no link, ignored by Git and holding nothing Git tracks, with
    no Boat test run or Cargo build holding a lock in it. Nothing else builds in the runner's
    worktree, and the runner lock keeps out other runs."""
    try:
        if target.resolve() != configured_target().resolve():
            return "it is not the runner worktree's own Cargo target"
        attributes = getattr(os.lstat(target), "st_file_attributes", 0)
        if not target.is_dir() or target.is_symlink() or attributes & stat.FILE_ATTRIBUTE_REPARSE_POINT:
            return "it is not a plain folder"
        relative = target.resolve().relative_to(WORKTREE.resolve()).as_posix()
        if git("ls-files", "--", relative):
            return "Git tracks files in it"
        try:
            git("check-ignore", "--quiet", "--no-index", "--", relative)
        except RuntimeError:
            return "Git does not ignore it"
        for directory, folders, files in os.walk(target):
            for name in folders:
                if getattr(os.lstat(os.path.join(directory, name)), "st_file_attributes", 0) & stat.FILE_ATTRIBUTE_REPARSE_POINT:
                    return f"it links elsewhere at {os.path.relpath(os.path.join(directory, name), target)}"
            for name in LOCKS.intersection(files):
                with open(os.path.join(directory, name), "a+b") as held:
                    held.seek(0)
                    try:
                        msvcrt.locking(held.fileno(), msvcrt.LK_NBLCK, 1)
                    except OSError:
                        return f"a build holds {os.path.relpath(os.path.join(directory, name), target)}"
                    held.seek(0)
                    msvcrt.locking(held.fileno(), msvcrt.LK_UNLCK, 1)
    except (OSError, RuntimeError) as error:
        return f"it could not be checked: {error}"
    return None


def check_disk(build, log):
    """Passes with DISK_FLOOR free on the target's drive. Below it, removes the runner's own Cargo
    target when that alone brings the drive to the floor, and keeps it when it would not."""
    drive = build.target.anchor
    try:
        free = shutil.disk_usage(drive).free
    except OSError as error:
        log.write_text(f"{error}\n", encoding="utf-8")
        return 1, "failed: the free space could not be read"
    floor = f"the {gigabytes(DISK_FLOOR)} floor"
    notes = [f"{gigabytes(free)} free on {drive}; {floor}"]
    reclaimed = ""
    refused = not_owned(build.target) if free < DISK_FLOOR and build.target.exists() else None
    if refused:
        notes.append(f"kept {build.target}: {refused}")
    elif free < DISK_FLOOR and build.target.exists():
        size = size_of(build.target) or 0
        if free + size < DISK_FLOOR:
            notes.append(f"kept the runner's Cargo target {build.target} ({gigabytes(size)}): "
                         f"removing it would not reach the floor")
        else:
            try:
                shutil.rmtree(build.target)
            except OSError as error:
                notes.append(f"could not remove all of {build.target}: {error}")
            try:
                free = shutil.disk_usage(drive).free
            except OSError as error:
                notes.append(f"{error}")
                log.write_text("\n".join(notes) + "\n", encoding="utf-8")
                return 1, "failed: the free space could not be read"
            notes.append(f"removed the runner's Cargo target {build.target} ({gigabytes(size)}); "
                         f"{gigabytes(free)} free")
            reclaimed = f", after removing the runner's {gigabytes(size)} Cargo target"
    log.write_text("\n".join(notes) + "\n", encoding="utf-8")
    if free < DISK_FLOOR:
        return 1, f"failed: {gigabytes(free)} free{reclaimed}, below {floor}"
    return 0, f"passed: {gigabytes(free)} free{reclaimed}"


def rust_tests(build, log):
    """Lists the workspace's tests, which builds them, then runs them with nextest; counts each test
    the filters select as passed, failed or missing, and each they leave out as skipped. Keeps the
    listing and JUnit report in the run's folder."""
    junit = nextest_junit()
    listing = build.folder / "rust-list.json"
    with open(log, "wb") as output:
        try:
            junit.unlink(missing_ok=True)
        except OSError as error:
            output.write(f"could not remove the last run's {junit}: {error}\n".encode())
            return 1, "failed: the last run's JUnit report could not be removed"
        with open(listing, "wb") as listed:
            code = bounded(RUST_LIST, listed, output, build.deadline - time.monotonic(), build.environment)
        if code == 0:
            output.write(b"\n")
            output.flush()
            with tempfile.TemporaryFile() as messages:
                code = bounded(RUST_TESTS, messages, output, build.deadline - time.monotonic(), build.environment)
                units, finished = cargo_units(messages)
            if code is not None and (finished or (finished is None and junit.is_file())):
                build.units = units
    report = None
    if junit.is_file():
        shutil.copyfile(junit, build.folder / "rust-junit.xml")
        report = junit.read_text(encoding="utf-8", errors="replace")
    listed_text = listing.read_text(encoding="utf-8", errors="replace") if listing.is_file() else None
    build.tests = ci_results.tests(listed_text, report)
    if code is None:
        return 1, f"stopped: out of time; {build.tests.describe()}"
    if code == 0 and build.tests.complete():
        return 0, f"passed: {build.tests.describe()}"
    return 1, f"failed (exit {code}): {build.tests.describe()}"


def boat_rust_tests(build, log):
    """Builds every workspace crate's tests on Boat and runs them here (tools/boat.py test), once
    again when Boat itself failed; counts them from the programs' output. Stopped after
    BOAT_RUST_LIMIT, since libtest stops no test that hangs."""
    import boat
    packages = sorted(boat.workspace(WORKTREE)[0])
    command = [
        str(Path(sys.executable).with_name("python.exe")), "tools/boat.py", "test", "--no-fail-fast",
        *(argument for package in packages for argument in ("-p", package)),
    ]
    stop = min(build.deadline, time.monotonic() + BOAT_RUST_LIMIT.total_seconds())
    with open(log, "wb") as output:
        for attempt in range(2):
            began = output.tell()
            code = bounded(command, output, subprocess.STDOUT, stop - time.monotonic(), build.environment)
            output.seek(0, os.SEEK_END)
            if code != BOAT_FAILED or attempt or time.monotonic() >= stop:
                break
            output.write(b"\nBoat itself failed (exit 75); running it again.\n\n")
            output.flush()
    build.tests = ci_results.libtest(log.read_bytes()[began:].decode("utf-8", errors="replace"))
    if code is None:
        return 1, f"stopped: out of time; {build.tests.describe()}"
    if code == BOAT_FAILED:
        return 1, f"failed: Boat itself failed twice (exit 75); {build.tests.describe()}"
    if code == 0 and build.tests.complete():
        return 0, f"passed: {build.tests.describe()}"
    return 1, f"failed (exit {code}): {build.tests.describe()}"


class Units:
    """What a finished Cargo build produced or used: its units' hashes, the files it named, and the
    fingerprint names of units whose files carry no hash (binaries and examples on MSVC)."""

    def __init__(self):
        self.hashes, self.paths, self.unhashed = set(), set(), set()


def cargo_units(messages):
    """The `Units` in Cargo's JSON messages (a binary file), and whether its build finished: None
    without Cargo's message saying so either way."""
    units, finished = Units(), None
    messages.seek(0)
    for line in messages:
        try:
            message = json.loads(line)
        except ValueError:
            continue
        if not isinstance(message, dict):
            continue
        reason = message.get("reason")
        if reason == "compiler-artifact":
            target = message.get("target") or {}
            files = [*(message.get("filenames") or []), *filter(None, [message.get("executable")])]
            for file in map(Path, files):
                units.paths.add(os.path.normcase(file))
                hashed = UNIT_HASH.search(file.name) or UNIT_FOLDER.match(file.parent.name)
                if hashed:
                    units.hashes.add(hashed.group(hashed.lastindex))
                else:
                    kinds = target.get("kind") or []
                    kind = "bin" if "bin" in kinds else "example" if "example" in kinds else "lib"
                    units.unhashed.add(f"{kind}-{target.get('name')}")
        elif reason == "build-script-executed":
            hashed = UNIT_FOLDER.match(Path(message.get("out_dir") or "").parent.name)
            if hashed:
                units.hashes.add(hashed.group(2))
        elif reason == "build-finished":
            finished = message.get("success") is True
    return units, finished


class Refused(Exception):
    pass


def entries(folder):
    try:
        return list(os.scandir(folder))
    except FileNotFoundError:
        return []


def stale(profile, units):
    """Every path in the build's profile folder that the build neither produced nor used. Refused
    when a unit Cargo named has no fingerprint there: the layout is not the one this reads."""
    if not units.hashes:
        raise Refused("Cargo named no units")
    removals, found, owners = [], set(), {}
    for entry in entries(profile / ".fingerprint"):
        named = UNIT_FOLDER.match(entry.name)
        if named and named.group(2) in units.hashes:
            found.add(named.group(2))
            continue
        mine = set()
        if named and entry.is_dir():
            mine = units.unhashed.intersection(os.listdir(entry.path))
        if not mine:
            removals.append(entry.path)
        for name in mine:
            written = os.stat(os.path.join(entry.path, name)).st_mtime_ns
            owners.setdefault((named.group(1), name), []).append((written, entry.path))
    newest = {max(candidates)[1] for candidates in owners.values()}
    removals += sorted({path for candidates in owners.values() for _, path in candidates} - newest)
    missing = sorted(units.hashes - found) + sorted(units.unhashed - {name for _, name in owners})
    if missing:
        raise Refused(f"units Cargo named have no fingerprint in {profile}: {', '.join(missing)}")
    for entry in entries(profile / "build"):
        named = UNIT_FOLDER.match(entry.name)
        if not (named and named.group(2) in units.hashes):
            removals.append(entry.path)
    stems = {name.split("-", 1)[1].replace("-", "_") for name in units.unhashed}
    for entry in entries(profile / "deps") + entries(profile / "examples"):
        if entry.is_file() and not produced(entry, units, stems):
            removals.append(entry.path)
    for entry in entries(profile):
        if entry.is_file() and not entry.name.startswith("."):
            if os.path.normcase(entry.path) not in units.paths:
                removals.append(entry.path)
    if (profile / "incremental").exists():
        removals.append(str(profile / "incremental"))
    return removals


def produced(entry, units, stems):
    """Whether a file in deps or examples belongs to a unit of the build."""
    if os.path.normcase(entry.path) in units.paths:
        return True
    if entry.name.endswith(".rcgu.o"):
        return False
    hashed = UNIT_HASH.search(entry.name)
    if hashed:
        return hashed.group(1) in units.hashes
    stem, _, extensions = entry.name.partition(".")
    return stem in stems and UNHASHED_EXTENSIONS.fullmatch(extensions) is not None


def sweep_target(build, log):
    """Remove from the target what its finished build neither produced nor used, so it holds one
    build: the units of earlier builds, incremental files and objects of failed links."""
    try:
        removals = stale(build.target / "debug", build.units)
    except Refused as error:
        log.write_text(f"Refused: {error}\n", encoding="utf-8")
        return 1, "failed: refused, the target is not laid out as Cargo reported"
    except OSError as error:
        log.write_text(f"{error}\n", encoding="utf-8")
        return 1, "failed: the target could not be read"
    kept = 0
    with open(log, "w", encoding="utf-8") as out:
        for path in removals:
            try:
                if os.path.isdir(path) and not os.path.islink(path):
                    shutil.rmtree(path)
                else:
                    os.remove(path)
                out.write(f"removed {path}\n")
            except OSError as error:
                kept += 1
                out.write(f"could not remove {path}: {error}\n")
    if kept:
        return 1, f"failed: {kept} of {len(removals)} stale entries could not be removed"
    return 0, f"passed: removed {len(removals)} stale entries"


def run_folder(started, commit):
    """A folder this run creates for its logs, marked as the runner's; never an existing one."""
    name = f"{started:%Y-%m-%d-%H%M}-{commit[:10]}"
    for attempt in range(1, 100):
        folder = REPORTS / (name if attempt == 1 else f"{name}-{attempt}")
        try:
            folder.mkdir()
        except FileExistsError:
            continue
        (folder / RUN_MARK).write_text(f"{started:%Y-%m-%d %H:%M}\t{commit}\n", encoding="utf-8")
        return folder
    raise RuntimeError(f"every run folder name beside {name} is taken")


def prune(now):
    """Delete run folders this runner marked as its own, older than KEEP; keep anything else, and
    a folder that cannot be read or deleted until the next night."""
    try:
        folders = list(REPORTS.iterdir())
    except OSError:
        return
    for folder in folders:
        try:
            if not RUN_FOLDER.match(folder.name) or not folder.is_dir():
                continue
            mark = (folder / RUN_MARK).read_text(encoding="utf-8").split("\t")[0].strip()
            if now - datetime.datetime.strptime(mark, "%Y-%m-%d %H:%M") > KEEP:
                shutil.rmtree(folder)
        except (OSError, ValueError):
            continue


def publisher():
    """Where results are published: the repository the runner's worktree is a clone of."""
    return ci_results.GitHub(WORKTREE, shutil.which("gh") or "gh")


class Publishing:
    """Statuses and the tracking issue through a `ci_results.GitHub`, with what failed kept to report."""

    def __init__(self, github):
        self.github, self.failures = github, []

    def status(self, commit, context, state, description):
        if commit:
            self.note(self.github.status(commit, context, state, description))

    def track(self, failing, body):
        self.note(self.github.track(failing, ISSUE_TITLE, body))

    def note(self, failure):
        if failure:
            self.failures.append(f"Not published: {failure}")


class RunFailed(RuntimeError):
    """A failure of the run itself, named without local paths."""


def write_open_groups(folder, groups):
    (folder / (OPEN_GROUPS + ".tmp")).write_text(json.dumps({"open": groups}), encoding="utf-8")
    os.replace(folder / (OPEN_GROUPS + ".tmp"), folder / OPEN_GROUPS)


def close_open_groups(folder, commit, publishing, description):
    """Sets each group the run's folder names as not yet ended to error on `commit`."""
    try:
        groups = json.loads((folder / OPEN_GROUPS).read_text(encoding="utf-8"))["open"]
    except (OSError, ValueError, KeyError, TypeError):
        return
    for group in groups:
        publishing.status(commit, f"{STATUS}/{group}", "error", description)


def run_step_row(build, name, slug, command, needs, passed, log):
    """Runs one step unless a step it needs did not pass or time is up; its report row."""
    missing = [need for need in ((needs,) if isinstance(needs, str) else needs or ()) if need not in passed]
    if missing:
        return f"| {name} | not run: {', '.join(missing)} failed | | |"
    if time.monotonic() >= build.deadline:
        return f"| {name} | not run: out of time | | |"
    began = time.monotonic()
    if callable(command):
        code, outcome = command(build, log)
    else:
        code = run_step(command, log, build.environment, build.deadline - began)
        outcome = "passed" if code == 0 else "stopped: out of time" if code is None else f"failed (exit {code})"
    if code == 0:
        passed.add(slug)
    if slug == "rust" and build.units is not None:
        passed.add("rust-build")
    return f"| {name} | {outcome} | {duration(time.monotonic() - began)} | {log.name} |"


def group_result(slugs, names, passed, tests):
    """A group's commit status state and description."""
    failed = [names[slug] for slug in slugs if slug not in passed]
    description = "passed" if not failed else f"failed: {', '.join(failed)}"
    if "rust" in slugs:
        description += f"; {(tests or ci_results.Tests()).describe()}"
    return ("failure" if failed else "success"), description


def run_checked_out(args):
    commit, started, deadline, folder = args.continue_run
    started = datetime.datetime.fromisoformat(started)
    deadline, folder = float(deadline), Path(folder)
    if hold_checkout() is None:
        raise RuntimeError("another checked-out runner holds the worktree")
    actual = git("rev-parse", "HEAD")
    if actual != commit:
        raise RuntimeError(f"checked-out runner expected {commit}, HEAD is {actual}")
    heading = f"main {commit[:10]} {git('log', '-1', '--format=%s')}"
    environment = dict(
        os.environ, BATTLE_PLAN_GAME_DIR=args.game_dir, CARGO_TERM_COLOR="never", NO_COLOR="1",
        CARGO_INCREMENTAL="0",
    )
    environment.pop("BATTLE_PLAN_RECORDED_REPLIES", None)
    if args.recorded_replies:
        environment["BATTLE_PLAN_RECORDED_REPLIES"] = args.recorded_replies
    build = Build(rust_target(), folder, environment, deadline, args.fixture)
    size_at_start = size_of(build.target)
    plan = steps()
    names = {slug: name for name, slug, _, _ in plan}
    groups = {group: [slug for slug in slugs if slug in names] for group, slugs in GROUPS.items()}
    groups = {group: slugs for group, slugs in groups.items() if slugs}
    publishing = Publishing(publisher())
    write_open_groups(folder, list(groups))
    for group in groups:
        publishing.status(commit, f"{STATUS}/{group}", "pending", "running")
    table = ["| Step | Result | Time | Log |", "|---|---|---|---|"]
    rows, passed, done, ended = [], set(), set(), []
    for name, slug, command, needs in plan:
        rows.append(run_step_row(build, name, slug, command, needs, passed, folder / f"{slug}.log"))
        done.add(slug)
        for group, slugs in groups.items():
            if group not in ended and done.issuperset(slugs):
                ended.append(group)
                state, description = group_result(slugs, names, passed, build.tests)
                publishing.status(commit, f"{STATUS}/{group}", state, description)
                running = [other for other in groups if other not in ended]
                write_open_groups(folder, running)
                try:
                    write_report("RUNNING", [
                        heading, f"Started {started:%Y-%m-%d %H:%M}. Logs: {folder}", "", *table, *rows, "",
                        f"Still running: {', '.join(running)}." if running else "Reporting.",
                    ])
                except OSError:
                    pass
    result = "passed" if all(slug in passed for _, slug, _, _ in plan) else "failed"
    tests = build.tests or ci_results.Tests()
    counted = [f"Rust tests: {tests.describe()}."]
    if tests.failures:
        counted += ["", "## Tests that did not pass", "", *(f"- {test}" for test in tests.failures)]
    lines = [
        heading,
        f"{started:%Y-%m-%d %H:%M} to {datetime.datetime.now():%H:%M}. Logs: {folder}",
        f"Rust target {build.target}: {gigabytes(size_at_start)} at start, "
        f"{gigabytes(size_of(build.target))} at end.",
        "", *table, *rows, "", *counted,
    ]
    if publishing.failures:
        lines += ["", *publishing.failures]
    public = [heading, "", *table, *rows, "", *counted]
    (folder / "result.json").write_text(json.dumps({
        "result": result, "failed": tests.failed, "lines": lines, "public": public,
    }), encoding="utf-8")
    return 0


def unfinished_run():
    """The start, commit and folder of the run that left MARKER behind without recording its
    result, or None; the commit and folder are None when it never made its folder."""
    try:
        parts = MARKER.read_text(encoding="utf-8").split()
        started = datetime.datetime.strptime(" ".join(parts[1:3]), "%Y-%m-%d %H:%M")
    except (OSError, ValueError):
        return None
    folder = REPORTS / parts[3] if len(parts) > 3 and RUN_FOLDER.match(parts[3]) else None
    try:
        commit = (folder / RUN_MARK).read_text(encoding="utf-8").strip().split("\t")[1] if folder else None
    except (OSError, IndexError):
        commit = None
    return started, commit, folder


def save_outcome(folder, outcome):
    """Saves a run's outcome in its folder whole or not at all; whether it was saved."""
    if folder is None:
        return True
    try:
        (folder / (OUTCOME + ".tmp")).write_text(json.dumps(outcome), encoding="utf-8")
        os.replace(folder / (OUTCOME + ".tmp"), folder / OUTCOME)
    except OSError:
        return False
    return True


def outcome_of(started, result, failed, public):
    """What a run publishes and records once its result is known."""
    heading = "RUNNER ERROR" if result == "error" else result.upper()
    return {"result": result, "failed": failed, "recorded": False,
            "state": {"passed": "success", "failed": "failure"}.get(result, "error"),
            "description": f"{result} {started:%Y-%m-%d %H:%M}", "failing": result != "passed",
            "body": report_text(heading, public)}


def saved_outcome(started, folder):
    """The outcome a stopped run saved, else the one its checked-out runner's result gives when only
    the parent stopped; None without either."""
    for name in (OUTCOME, "result.json"):
        try:
            saved = json.loads((folder / name).read_text(encoding="utf-8"))
            if name == OUTCOME:
                return saved
            public = saved.get("public") or [f"Result: {saved['result']}."]
            return outcome_of(started, saved["result"], saved["failed"], public)
        except (OSError, ValueError, KeyError, TypeError, AttributeError):
            continue
    return None


def report_unfinished(unfinished, publishing):
    """Records a run that never reported, in history.tsv and on GitHub, or publishes the result it
    saved before it stopped; the report lines it adds. A history.tsv it cannot append to is said,
    and its statuses are published still."""
    started, commit, folder = unfinished
    outcome = saved_outcome(started, folder) if folder else None
    lines = []

    def history(result, failed):
        try:
            record_history(started, commit, result, failed)
        except OSError as error:
            lines.append(f"Could not append the run started {started:%Y-%m-%d %H:%M} to history.tsv: {error}")

    if outcome:
        if not outcome.get("recorded"):
            history(outcome["result"], outcome["failed"])
        if outcome["state"] == "error":
            close_open_groups(folder, commit, publishing, "the run ended before this group did")
        publishing.status(commit, STATUS, outcome["state"], outcome["description"])
        publishing.track(outcome["failing"], outcome["body"])
        said = f"The daily run started {started:%Y-%m-%d %H:%M} saved its result but stopped before publishing it; published now."
        return [said, *lines, ""]
    history("unfinished", None)
    said = f"The daily run started {started:%Y-%m-%d %H:%M}" + (f" on {commit[:10]}" if commit else "")
    said += " never reported: it was stopped, or the machine went down, before it finished."
    publishing.status(commit, STATUS, "error", f"unfinished: the run started {started:%Y-%m-%d %H:%M} never reported")
    if folder:
        close_open_groups(folder, commit, publishing, "unfinished: the run never reached this group's end")
    publishing.track(True, report_text("UNFINISHED", [said]))
    return [said, *lines, ""]


def run(args):
    if args.continue_run is not None:
        return run_checked_out(args)
    lock = lock_runner()
    if lock is None:
        return 0
    if not checkout_free():
        os.close(lock)
        return 0
    deadline = time.monotonic() + DEADLINE.total_seconds()
    started = datetime.datetime.now()
    commit, folder, log = None, None, None
    publishing = Publishing(publisher())
    earlier = []
    try:
        try:
            unfinished = unfinished_run()
            if unfinished:
                earlier = report_unfinished(unfinished, publishing)
            write_marker(f"{os.getpid()} {started:%Y-%m-%d %H:%M}\n")
            write_report("RUNNING", [f"Started {started:%Y-%m-%d %H:%M}. Earlier runs: history.tsv."])
            owner = Path(git("rev-parse", "--show-toplevel"))
            if owner.resolve() != WORKTREE.resolve():
                raise RunFailed(f"{WORKTREE} is not a worktree of its own: git there works on {owner}")
            git("fetch", "--quiet", "origin", "main")
            git("checkout", "--quiet", "--detach", "--force", "origin/main")
            commit = git("rev-parse", "HEAD")
            folder = run_folder(started, commit)
            write_marker(f"{os.getpid()} {started:%Y-%m-%d %H:%M} {folder.name}\n")
            publishing.status(commit, STATUS, "pending", f"running since {started:%Y-%m-%d %H:%M}")
            log = folder / "runner.log"
            command = [
                sys.executable, str(WORKTREE / "tools" / "daily_tests.py"), "run",
                "--game-dir", args.game_dir, *fixtures(args),
                "--continue-run", commit, started.isoformat(),
                str(deadline - REPORT_MARGIN), str(folder), "--ci-dir", args.ci_dir,
            ]
            remaining = deadline - time.monotonic()
            if remaining <= REPORT_MARGIN:
                raise RunFailed("out of time before starting the checked-out runner")
            code = run_step(command, log, None, remaining)
            if code is None:
                raise RunFailed("checked-out runner stopped: out of time")
            if code != 0:
                raise RunFailed(f"checked-out runner failed (exit {code})")
            try:
                completed = json.loads((folder / "result.json").read_text(encoding="utf-8"))
            except (OSError, ValueError):
                raise RunFailed("the checked-out runner left no readable result") from None
            result, failed, lines = completed["result"], completed["failed"], completed["lines"]
            public = completed.get("public") or [f"main {commit[:10]}: {result}.", "Its report is latest.md on the test machine."]
            code = {"passed": 0, "failed": 1}[result]
        except Exception as failure:
            error = traceback.format_exc().rstrip()
            if log is not None:
                try:
                    error += "\n\n" + log.read_text(encoding="utf-8", errors="replace")
                except OSError:
                    pass
            result, failed, code = "error", None, 2
            lines = ["```", error, "```"]
            said = failure if isinstance(failure, RunFailed) else type(failure).__name__
            public = [f"The runner failed: {said}.", "Its report is latest.md on the test machine."]
        heading = "RUNNER ERROR" if result == "error" else result.upper()
        outcome = outcome_of(started, result, failed, public)
        state = outcome["state"]
        if not save_outcome(folder, outcome):
            code = 2
        try:
            record_history(started, commit, result, failed)
            outcome["recorded"] = True
            save_outcome(folder, outcome)
        except OSError:
            heading, code = "RUNNER ERROR", 2
            lines += ["", "Could not append history.tsv:", "```", traceback.format_exc().rstrip(), "```"]
        if result == "error" and folder is not None:
            close_open_groups(folder, commit, publishing, "the run ended before this group did")
        publishing.status(commit, STATUS, state, outcome["description"])
        publishing.track(outcome["failing"], outcome["body"])
        lines = [*earlier, *lines, *([""] + publishing.failures if publishing.failures else [])]
        report = report_text(heading, lines)
        try:
            write_report(heading, lines)
        except OSError:
            code = 2
        if folder is not None:
            try:
                (folder / "report.md").write_text(report, encoding="utf-8")
            except OSError:
                code = 2
        try:
            prune(started)
        except OSError:
            code = 2
        try:
            MARKER.unlink(missing_ok=True)
        except OSError:
            code = 2
        return code
    finally:
        os.close(lock)


TASK_XML = """<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>Tests Battle Plan's main branch once a day (tools/daily_tests.py).</Description>
  </RegistrationInfo>
  <Triggers>
    <CalendarTrigger>
      <StartBoundary>{start}</StartBoundary>
      <ScheduleByDay>
        <DaysInterval>1</DaysInterval>
      </ScheduleByDay>
    </CalendarTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <StartWhenAvailable>true</StartWhenAvailable>
    <ExecutionTimeLimit>PT20H</ExecutionTimeLimit>
    <Priority>7</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{command}</Command>
      <Arguments>{arguments}</Arguments>
      <WorkingDirectory>{worktree}</WorkingDirectory>
    </Exec>
  </Actions>
</Task>
"""


def install(args):
    if not (Path(args.game_dir) / "hoi4.exe").is_file():
        sys.exit(f"{args.game_dir} holds no hoi4.exe")
    at = datetime.datetime.strptime(args.at, "%H:%M").time()
    if WORKTREE.exists():
        refresh_runner()
    else:
        git_at(ROOT, "fetch", "--quiet", "origin", "main")
        git_at(ROOT, "worktree", "add", "--detach", str(WORKTREE), "origin/main")
    script = WORKTREE / "tools" / "daily_tests.py"
    if not script.is_file():
        sys.exit(f"{script} is missing: the runner must be on origin/main first")
    start = datetime.datetime.combine(datetime.date.today(), at)
    if start <= datetime.datetime.now():
        start += datetime.timedelta(days=1)
    arguments = subprocess.list2cmdline([
        str(script), "run", "--ci-dir", args.ci_dir, "--game-dir", args.game_dir, *fixtures(args),
    ])
    xml = TASK_XML.format(
        start=start.isoformat(timespec="seconds"),
        user=escape(f"{os.environ['USERDOMAIN']}\\{os.environ['USERNAME']}"),
        command=escape(str(Path(sys.executable).with_name("pythonw.exe"))),
        arguments=escape(arguments),
        worktree=escape(str(WORKTREE)),
    )
    with tempfile.TemporaryDirectory() as folder:
        definition = Path(folder) / "task.xml"
        definition.write_text(xml, encoding="utf-16")
        subprocess.run(
            ["schtasks", "/Create", "/TN", TASK, "/XML", str(definition), "/F"], check=True
        )
    print(f"{TASK}: daily at {args.at}, first {start:%Y-%m-%d %H:%M}; reports in {REPORTS}")
    return 0


def git_at(folder, *args):
    return subprocess.run(
        ["git", "-C", str(folder), *args], check=True, capture_output=True, text=True
    ).stdout.strip()


def refresh_runner():
    """Moves an existing runner worktree of this clone to origin/main, so the task never starts a
    runner too old for its arguments; exits naming the folder when it is not one, or is in use."""
    def common(folder):
        return Path(git_at(folder, "rev-parse", "--path-format=absolute", "--git-common-dir")).resolve()

    try:
        owner = Path(git_at(WORKTREE, "rev-parse", "--show-toplevel")).resolve()
        foreign = owner != WORKTREE.resolve() or common(WORKTREE) != common(ROOT)
    except subprocess.CalledProcessError:
        foreign = True
    if foreign:
        sys.exit(f"{WORKTREE} is not a worktree of {ROOT}: name another --ci-dir or remove it")
    lock = lock_runner()
    if lock is None:
        sys.exit(f"a run holds {WORKTREE}: install again when it ends")
    try:
        if not checkout_free():
            sys.exit(f"a run holds {WORKTREE}: install again when it ends")
        git_at(ROOT, "fetch", "--quiet", "origin", "main")
        git_at(WORKTREE, "checkout", "--quiet", "--detach", "--force", "origin/main")
    finally:
        os.close(lock)


def fixtures(args):
    """The fixture arguments a run passes on: to the checked-out runner, and in the task."""
    return [
        *(["--fixture", args.fixture] if args.fixture else []),
        *(["--recorded-replies", args.recorded_replies] if args.recorded_replies else []),
    ]


def parse(argv):
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    commands = parser.add_subparsers(dest="command", required=True)
    game_dir = os.environ.get("BATTLE_PLAN_GAME_DIR", os.environ.get("HOI4_GAME_DIR")) or None
    ci_dir = os.environ.get("BATTLE_PLAN_CI") or None
    for name in ("install", "run"):
        command = commands.add_parser(name)
        command.add_argument("--ci-dir", type=os.path.abspath, default=ci_dir, required=ci_dir is None)
        command.add_argument(
            "--game-dir", type=os.path.abspath, default=game_dir, required=game_dir is None
        )
        command.add_argument("--fixture", type=os.path.abspath, default=os.environ.get("BATTLE_PLAN_E2E_FIXTURE", os.environ.get("HOI4_E2E_FIXTURE")))
        command.add_argument(
            "--recorded-replies", type=os.path.abspath, default=os.environ.get("BATTLE_PLAN_RECORDED_REPLIES", os.environ.get("HOI4_RECORDED_REPLIES")) or None
        )
        if name == "install":
            command.add_argument("--at", default="03:00", help="local start time, HH:MM")
        else:
            command.add_argument("--continue-run", nargs=4, help=argparse.SUPPRESS)
    return parser.parse_args(argv)


def configure(ci_dir):
    """Points the runner at its own folder: the worktree in main, the acceptance app beside it and,
    unless BATTLE_PLAN_CAPTURES names another, the reports and locks in captures there."""
    global WORKTREE, E2E_APP, CAPTURES, REPORTS, MARKER, E2E_STATE
    ci = Path(ci_dir)
    WORKTREE, E2E_APP = ci / "main", ci / "e2e-app"
    CAPTURES = Path(os.environ.get("BATTLE_PLAN_CAPTURES") or os.environ.get("HOI4_CAPTURES") or ci / "captures")
    REPORTS = CAPTURES / "ci"
    MARKER, E2E_STATE = REPORTS / "TESTS_RUNNING", REPORTS / "e2e"


def main():
    args = parse(sys.argv[1:])
    configure(args.ci_dir)
    return install(args) if args.command == "install" else run(args)


if __name__ == "__main__":
    sys.exit(main())
