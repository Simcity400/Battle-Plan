"""Test `main` once a day on this machine (Python 3.11+, Windows).

    python tools/daily_tests.py install --game-dir <the game folder> [--at 03:00]
    python tools/daily_tests.py run --game-dir <the game folder>

`install` makes the runner's own worktree of origin/main and registers the Windows task that runs
it daily at `--at`, or as soon as it can after a missed start. `run` moves that worktree to the
newest origin/main and holds TESTS_RUNNING while it runs every Rust test, the UI typecheck and
tests and the dependency checks. Its report goes to
captures\\ci: latest.md, a folder per run with each step's log, kept 30 days, and
history.tsv, kept whole.
"""

import argparse
import datetime
import msvcrt
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
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


RUST_TESTS = [
    "cargo", "nextest", "run", "--workspace", "--no-fail-fast", "--final-status-level", "fail"
]


def steps():
    """Each step: its name, its log's name, its command, and the step it needs to have passed."""
    npm = shutil.which("npm.cmd") or "npm.cmd"
    python = str(Path(sys.executable).with_name("python.exe"))
    check = lambda script: [python, f"tools/{script}.py"]
    return [
        ("Rust tests", "rust", RUST_TESTS, None),
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


def run_step(command, log, environment, seconds):
    """The step's exit code, or None when it ran out of time and was stopped."""
    with open(log, "wb") as output:
        return bounded(command, output, subprocess.STDOUT, seconds, environment)


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
        environment = dict(
            os.environ, BATTLE_PLAN_GAME_DIR=args.game_dir, CARGO_TERM_COLOR="never", NO_COLOR="1"
        )
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
            code = run_step(command, log, environment, deadline - began)
            took = duration(time.monotonic() - began)
            if slug == "rust":
                failures = not_passed(log)
                text = log.read_text(encoding="utf-8", errors="replace").splitlines()
                summary = next((line.strip() for line in reversed(text) if SUMMARY.match(line)), None)
            if code == 0:
                passed.add(slug)
                rows.append(f"| {name} | passed | {took} | {log.name} |")
            elif code is None:
                rows.append(f"| {name} | stopped: out of time | {took} | {log.name} |")
            else:
                rows.append(f"| {name} | failed (exit {code}) | {took} | {log.name} |")
        result = "passed" if len(passed) == len(plan) else "failed"
        lines = [
            heading,
            f"{started:%Y-%m-%d %H:%M} to {datetime.datetime.now():%H:%M}. Logs: {folder}",
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


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    commands = parser.add_subparsers(dest="command", required=True)
    game_dir = os.environ.get("BATTLE_PLAN_GAME_DIR", os.environ.get("HOI4_GAME_DIR"))
    for name in ("install", "run"):
        command = commands.add_parser(name)
        command.add_argument("--game-dir", default=game_dir, required=game_dir is None)
        if name == "install":
            command.add_argument("--at", default="03:00", help="local start time, HH:MM")
    args = parser.parse_args()
    return install(args) if args.command == "install" else run(args)


if __name__ == "__main__":
    sys.exit(main())
