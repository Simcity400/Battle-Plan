"""Set up a fresh worktree so it can run the UI (Python 3.11+, Windows).

usage: python tools/setup_worktree.py   (T3 Code runs it when it creates a worktree, see t3.json)

Runs `npm ci` in the worktree and leaves a marker in `.t3/`: `setup-running` holds this process's
PID while it runs, then `setup-done`, or `setup-failed` with the tail of the error. While
`setup-running` names a live process, wait for it; with no marker, setup never ran, so run this.
Rust is not built here: it compiles on Boat (tools/boat.py).
"""
import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MARKERS = ROOT / ".t3"
TAIL = 4000


def main():
    MARKERS.mkdir(exist_ok=True)
    for name in ("setup-running", "setup-done", "setup-failed"):
        (MARKERS / name).unlink(missing_ok=True)
    (MARKERS / "setup-running").write_text(str(os.getpid()), encoding="utf-8")
    npm = shutil.which("npm.cmd") or "npm.cmd"
    try:
        result = subprocess.run([npm, "ci"], cwd=ROOT, capture_output=True, text=True, encoding="utf-8", errors="replace")
        failure = None if result.returncode == 0 else (result.stdout + result.stderr)[-TAIL:]
    except OSError as error:
        failure = str(error)
    (MARKERS / "setup-running").unlink(missing_ok=True)
    if failure is None:
        (MARKERS / "setup-done").write_text("", encoding="utf-8")
        return 0
    (MARKERS / "setup-failed").write_text(failure, encoding="utf-8")
    print(failure, file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main())
