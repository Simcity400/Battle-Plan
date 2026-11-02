"""Test `main` once a day on this machine (Python 3.11+, Windows).

    python tools/daily_tests.py install --game-dir <the game folder> [--at 03:00]
    python tools/daily_tests.py run --game-dir <the game folder>

`install` makes the runner's own worktree of origin/main and registers the Windows task that runs
it daily at `--at`, or as soon as it can after a missed start. `run` moves that worktree to the
newest origin/main and holds TESTS_RUNNING while it runs every Rust test, the UI typecheck and
tests and the dependency checks. It compiles only with DISK_FLOOR free, without incremental
compilation, and once the build finishes removes from its Cargo target whatever that build neither
produced nor used, so the target holds one build. Its report goes to
captures\\ci: latest.md, a folder per run with each step's log, kept 30 days, and
history.tsv, kept whole.
"""

import argparse
import datetime
import json
import msvcrt
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
import tomllib
import traceback
from pathlib import Path
from xml.sax.saxutils import escape

ROOT = Path(__file__).resolve().parents[1]
WORKTREE = Path(r"ci\main")
CAPTURES = Path(os.environ.get("BATTLE_PLAN_CAPTURES", os.environ.get("HOI4_CAPTURES", ROOT / "captures")))
REPORTS = CAPTURES / "ci"
MARKER = REPORTS / "TESTS_RUNNING"
TASK = "Battle Plan daily tests"
# From taking the lock, below the task's 20-hour limit, so a run that overruns still reports and
# cleans up.
DEADLINE = datetime.timedelta(hours=18)
# Seconds a git command, or stopping a step that overran, may take.
GIT_TIMEOUT = 600
CLEANUP = 120
NO_WINDOW = subprocess.CREATE_NO_WINDOW
# Owner decision 2026-10-23: run folders are kept 30 days; history.tsv keeps every run's line.
KEEP = datetime.timedelta(days=30)
RUN_FOLDER = re.compile(r"^\d{4}-\d{2}-\d{2}-\d{4}-[0-9a-f]{10}(?:-\d+)?$")
# Written in each run folder as it is made: its start and commit, proving the runner made it.
RUN_MARK = "daily-tests-run.tsv"
SUMMARY = re.compile(r"^\s*Summary \[")
# After the summary nextest lists each failing test (final status level "fail"): its status, crate
# and name.
LISTED = re.compile(r"^\s*(\S.*?)\s+\[\s*[\d.]+s\]\s+(?:\(\s*\d+/\d+\)\s+)?(\S+\s+\S+)\s*$")
GB = 2**30
# Free space the target's drive needs to compile: a whole rebuild (one build was 10 to 20 GB on
# 2026-11-02) beside the build it replaces, which stays until the sweep, and the tests' own files.
DISK_FLOOR = 60 * GB
# Cargo's unit hash, closing a unit's file name before its extensions, or its folder's name.
UNIT_HASH = re.compile(r"-([0-9a-f]{16})(?=\.|$)")
UNIT_FOLDER = re.compile(r"^(.+)-([0-9a-f]{16})$")
UNHASHED_EXTENSIONS = re.compile(r"[A-Za-z]+(?:\.[A-Za-z]+)?")


# Cargo's JSON messages go to stdout for the sweep; nextest's report and diagnostics to stderr.
RUST_TESTS = [
    "cargo", "nextest", "run", "--workspace", "--no-fail-fast", "--final-status-level", "fail",
    "--cargo-message-format", "json-render-diagnostics",
]


def steps():
    """Each step: its name, its log's name, its command or a function of the run's `Build` and the
    log giving its exit code and result, and the step it needs to have passed."""
    npm = shutil.which("npm.cmd") or "npm.cmd"
    python = str(Path(sys.executable).with_name("python.exe"))
    check = lambda script: [python, f"tools/{script}.py"]
    return [
        ("Disk space", "disk", check_disk, None),
        ("Rust tests", "rust", RUST_TESTS, "disk"),
        ("Rust target sweep", "rust-sweep", sweep_target, "rust-build"),
        ("UI install", "ui-install", [npm, "ci"], None),
        ("UI typecheck", "ui-check", [npm, "run", "check:ui"], "ui-install"),
        ("UI tests", "ui-tests", [npm, "run", "test:ui"], "ui-install"),
        ("Build dependencies", "build-dependencies", check("check-build-dependencies"), None),
        ("Content dependencies", "content-dependencies", check("check-content-dependencies"), None),
        ("Feature unification", "feature-unification", check("check-feature-unification"), None),
    ]


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


def lock_runner():
    """Lock out other runs for this process's life; None while another run holds the lock."""
    REPORTS.mkdir(parents=True, exist_ok=True)
    descriptor = os.open(REPORTS / "runner.lock", os.O_RDWR | os.O_CREAT)
    try:
        msvcrt.locking(descriptor, msvcrt.LK_NBLCK, 1)
    except OSError:
        os.close(descriptor)
        return None
    return descriptor


def duration(seconds):
    return f"{seconds / 60:.0f} min" if seconds >= 60 else f"{seconds:.0f} s"


def write_report(heading, lines):
    text = "\n".join([f"# Daily tests: {heading}", "", *lines, ""])
    temporary = REPORTS / "latest.md.tmp"
    temporary.write_text(text, encoding="utf-8")
    os.replace(temporary, REPORTS / "latest.md")
    return text


def record_history(started, commit, result, failed):
    """One line per run; the failing test count is blank when no test results were read."""
    with open(REPORTS / "history.tsv", "a", encoding="utf-8") as history:
        count = "" if failed is None else failed
        history.write(f"{started:%Y-%m-%d %H:%M}\t{(commit or '')[:10]}\t{result}\t{count}\n")


def run_step(command, log, environment, seconds, stdout=None):
    """The step's exit code, or None when it ran out of time and was stopped. Its stdout goes to
    `stdout` when given, else to its log with its stderr."""
    with open(log, "wb") as output:
        errors = subprocess.STDOUT if stdout is None else output
        return bounded(command, stdout or output, errors, seconds, environment)


def target_dir():
    """The runner worktree's Cargo target: CARGO_TARGET_DIR, else its config's build.target-dir."""
    configured = os.environ.get("CARGO_TARGET_DIR")
    if not configured:
        try:
            with open(WORKTREE / ".cargo" / "config.toml", "rb") as config:
                configured = tomllib.load(config).get("build", {}).get("target-dir")
        except FileNotFoundError:
            pass
    return WORKTREE / (configured or "target")


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
    """What the Rust steps share: the target, and the `Units` of the build once it finished."""

    def __init__(self, target):
        self.target = target
        self.units = None


def check_disk(build, log):
    drive = build.target.anchor
    try:
        free = shutil.disk_usage(drive).free
    except OSError as error:
        log.write_text(f"{error}\n", encoding="utf-8")
        return 1, f"failed: the free space on {drive} could not be read"
    floor = f"the {gigabytes(DISK_FLOOR)} floor"
    log.write_text(f"{gigabytes(free)} free on {drive}; {floor}\n", encoding="utf-8")
    if free < DISK_FLOOR:
        return 1, f"failed: {gigabytes(free)} free on {drive}, below {floor}"
    return 0, f"passed: {gigabytes(free)} free on {drive}"


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
        # Each build of an unhashed unit writes its fingerprint, so its newest is this build's.
        for name in mine:
            written = os.stat(os.path.join(entry.path, name)).st_mtime_ns
            owners.setdefault((named.group(1), name), []).append((written, entry.path))
    # One fingerprint folder can hold several unhashed targets: kept if it is newest for any.
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
    # Objects a failed link left behind; a finished build has packed or removed its own.
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


def not_passed(log):
    """Each test nextest lists as failing, with its status; None without its summary."""
    text = log.read_text(encoding="utf-8", errors="replace").splitlines()
    summaries = [index for index, line in enumerate(text) if SUMMARY.match(line)]
    if not summaries:
        return None
    listed = (LISTED.match(line) for line in text[summaries[-1] + 1 :])
    return [f"{match.group(1)} {match.group(2)}" for match in listed if match]


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


def run(args):
    started = datetime.datetime.now()
    lock = lock_runner()
    if lock is None:
        return 0
    deadline = time.monotonic() + DEADLINE.total_seconds()
    commit, recorded = None, False
    try:
        MARKER.write_text(f"{os.getpid()} {started:%Y-%m-%d %H:%M}\n", encoding="utf-8")
        write_report("RUNNING", [f"Started {started:%Y-%m-%d %H:%M}. Earlier runs: history.tsv."])
        git("fetch", "--quiet", "origin", "main")
        git("checkout", "--quiet", "--detach", "--force", "origin/main")
        commit = git("rev-parse", "HEAD")
        heading = f"main {commit[:10]} {git('log', '-1', '--format=%s')}"
        folder = run_folder(started, commit)
        # A daily full build gains nothing from incremental files; they were most of the target.
        environment = dict(
            os.environ, BATTLE_PLAN_GAME_DIR=args.game_dir, CARGO_TERM_COLOR="never", NO_COLOR="1",
            CARGO_INCREMENTAL="0",
        )
        build = Build(target_dir())
        size_at_start = size_of(build.target)
        plan = steps()
        rows, failures, passed, summary = [], None, set(), None
        for name, slug, command, needs in plan:
            log = folder / f"{slug}.log"
            if needs is not None and needs not in passed:
                rows.append(f"| {name} | not run: {needs} failed | | |")
                continue
            if time.monotonic() >= deadline:
                rows.append(f"| {name} | not run: out of time | | |")
                continue
            began = time.monotonic()
            if callable(command):
                code, outcome = command(build, log)
                if code == 0:
                    passed.add(slug)
                took = duration(time.monotonic() - began)
                rows.append(f"| {name} | {outcome} | {took} | {log.name} |")
                continue
            if slug == "rust":
                with tempfile.TemporaryFile() as messages:
                    code = run_step(command, log, environment, deadline - began, messages)
                    units, finished = cargo_units(messages)
            else:
                code = run_step(command, log, environment, deadline - began)
            took = duration(time.monotonic() - began)
            if slug == "rust":
                failures = not_passed(log)
                text = log.read_text(encoding="utf-8", errors="replace").splitlines()
                summary = next((line.strip() for line in reversed(text) if SUMMARY.match(line)), None)
                # nextest runs tests, and prints their summary, only once the build finished.
                if code is not None and (finished or (finished is None and summary)):
                    build.units = units
                    passed.add("rust-build")
            if code == 0:
                passed.add(slug)
                rows.append(f"| {name} | passed | {took} | {log.name} |")
            elif code is None:
                rows.append(f"| {name} | stopped: out of time | {took} | {log.name} |")
            else:
                rows.append(f"| {name} | failed (exit {code}) | {took} | {log.name} |")
        result = "passed" if all(slug in passed for _, slug, _, _ in plan) else "failed"
        lines = [
            heading,
            f"{started:%Y-%m-%d %H:%M} to {datetime.datetime.now():%H:%M}. Logs: {folder}",
            f"Rust target {build.target}: {gigabytes(size_at_start)} at start, "
            f"{gigabytes(size_of(build.target))} at end.",
            "",
            "| Step | Result | Time | Log |",
            "|---|---|---|---|",
            *rows,
        ]
        if summary:
            lines += ["", summary]
        if failures:
            lines += ["", "## Tests that did not pass", "", *(f"- {test}" for test in failures)]
        record_history(started, commit, result, None if failures is None else len(failures))
        recorded = True
        report = write_report(result.upper(), lines)
        (folder / "report.md").write_text(report, encoding="utf-8")
        prune(started)
        return 0 if result == "passed" else 1
    except Exception:
        # History first: it outlives a latest.md this run cannot replace.
        error = traceback.format_exc().rstrip()
        if not recorded:
            try:
                record_history(started, commit, "error", None)
            except OSError:
                pass
        try:
            write_report("RUNNER ERROR", ["```", error, "```"])
        except OSError:
            pass
        return 2
    finally:
        MARKER.unlink(missing_ok=True)
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
    if not WORKTREE.exists():
        subprocess.run(["git", "-C", str(ROOT), "fetch", "--quiet", "origin", "main"], check=True)
        subprocess.run(
            ["git", "-C", str(ROOT), "worktree", "add", "--detach", str(WORKTREE), "origin/main"],
            check=True,
        )
    script = WORKTREE / "tools" / "daily_tests.py"
    if not script.is_file():
        sys.exit(f"{script} is missing: the runner must be on origin/main first")
    start = datetime.datetime.combine(datetime.date.today(), at)
    if start <= datetime.datetime.now():
        start += datetime.timedelta(days=1)
    arguments = subprocess.list2cmdline([str(script), "run", "--game-dir", args.game_dir])
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


def parse(argv):
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    commands = parser.add_subparsers(dest="command", required=True)
    game_dir = os.environ.get("BATTLE_PLAN_GAME_DIR", os.environ.get("HOI4_GAME_DIR")) or None
    for name in ("install", "run"):
        command = commands.add_parser(name)
        # Made absolute where it is given: the task and every step run from the runner's worktree.
        command.add_argument(
            "--game-dir", type=os.path.abspath, default=game_dir, required=game_dir is None
        )
        if name == "install":
            command.add_argument("--at", default="03:00", help="local start time, HH:MM")
    return parser.parse_args(argv)


def main():
    args = parse(sys.argv[1:])
    return install(args) if args.command == "install" else run(args)


if __name__ == "__main__":
    sys.exit(main())
