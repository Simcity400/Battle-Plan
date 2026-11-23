"""Refuse Clippy findings on the lines a change writes (Python 3.11+).

usage: python tools/check-clippy.py --against REF [--head REV] [--input FILE]
       python tools/check-clippy.py --against REF [--head REV] -- COMMAND...

The warning lints in Cargo.toml do not stop a build on their own, while denied lints fail the
build. This reads Cargo's human-readable Clippy output from FILE, from stdin, or from COMMAND
(run here, its output passed through, as in `-- python tools/boat.py clippy --all-targets -p
<crate>`), and fails on each finding whose primary line REV (default: the working tree,
untracked files included) adds or changes against its merge base with REF. Findings on other
lines are counted as existing. A COMMAND that exits nonzero fails too; a Boat exit of 75 is
Boat itself failing: run it again. Exits 1 on any failure.
"""
import argparse
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
HEADER = re.compile(r"^(warning|error)(?:\[\w+\])?: (.*)$")
LOCATION = re.compile(r"^\s*--> (.+?):(\d+):(\d+)\s*$")
LINT = re.compile(r"rust-clippy/[^#\s]*#([a-z0-9_]+)")
ANSI = re.compile(r"\x1b\[[0-9;]*m")
HUNK = re.compile(r"^@@ -\d+(?:,\d+)? \+(\d+)(?:,(\d+))? @@")


def findings(lines):
    """[(path, line, lint, level, message)] for each Clippy diagnostic in Cargo's output, once
    each however many targets reported it."""
    found, current = [], None

    def close():
        if current and current["lint"] and current["path"]:
            found.append((current["path"], current["line"], current["lint"], current["level"], current["message"]))

    for raw in lines:
        line = ANSI.sub("", raw.rstrip("\r\n"))
        if header := HEADER.match(line):
            close()
            current = {"level": header.group(1), "message": header.group(2), "path": None, "line": 0, "lint": None}
        elif current is None:
            continue
        elif current["path"] is None and (location := LOCATION.match(line)):
            current["path"] = normal(location.group(1))
            current["line"] = int(location.group(2))
        elif current["lint"] is None and (lint := LINT.search(line)):
            current["lint"] = lint.group(1)
    close()
    return list(dict.fromkeys(found))


def normal(path):
    """A diagnostic path as a repository path."""
    path = path.replace("\\", "/")
    for root in ("crates/", "fuzz/", "vendor/"):
        if path.startswith(root):
            return path
        if f"/{root}" in path:
            return path[path.index(f"/{root}") + 1:]
    return path


def written(base, head):
    """{path: lines} that `head` adds or changes against `base`; None for a whole file."""
    diff = subprocess.run(["git", "-c", "core.quotePath=false", "diff", "-U0", "--no-color", "--no-ext-diff", "--src-prefix=a/", "--dst-prefix=b/", "-M", base, *([head] if head else []), "--"],
                          cwd=ROOT, capture_output=True, text=True, encoding="utf-8", errors="replace", check=True).stdout
    lines, path = {}, None
    for line in diff.splitlines():
        if line.startswith("+++ "):
            path = None if line == "+++ /dev/null" else line[6:].removesuffix("\t")
        elif path and (hunk := HUNK.match(line)):
            start, count = int(hunk.group(1)), int(hunk.group(2) if hunk.group(2) is not None else 1)
            lines.setdefault(path, set()).update(range(start, start + count))
    if head is None:
        untracked = subprocess.run(["git", "ls-files", "-z", "--others", "--exclude-standard"], cwd=ROOT, capture_output=True, text=True, encoding="utf-8", check=True).stdout
        lines.update({name: None for name in untracked.split("\0") if name})
    return lines


def judge(found, changed):
    """(findings on written lines, how many other findings)."""
    new = [entry for entry in found if entry[0] in changed and (changed[entry[0]] is None or entry[1] in changed[entry[0]])]
    return new, len(found) - len(new)


def run(command):
    """The command's exit code and its output lines, passed through as they come."""
    process = subprocess.Popen(command, cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, encoding="utf-8", errors="replace")
    lines = []
    for line in process.stdout:
        sys.stdout.write(line)
        sys.stdout.flush()
        lines.append(line)
    return process.wait(), lines


def main():
    argv = sys.argv[1:]
    command = argv[argv.index("--") + 1:] if "--" in argv else None
    parser = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    parser.add_argument("--against", required=True, metavar="REF")
    parser.add_argument("--head", metavar="REV", help="judge a revision instead of the working tree")
    parser.add_argument("--input", metavar="FILE", help="Clippy output to read instead of stdin")
    args = parser.parse_args(argv[:argv.index("--")] if command is not None else argv)
    base = subprocess.run(["git", "merge-base", args.head or "HEAD", args.against], cwd=ROOT, capture_output=True, text=True)
    if base.returncode:
        sys.exit(f"No merge base with {args.against}: fetch it, or pass another --against.")
    exit_code = 0
    if command:
        exit_code, lines = run(command)
    elif args.input:
        lines = Path(args.input).read_text(encoding="utf-8", errors="replace").splitlines()
    else:
        lines = sys.stdin.read().splitlines()
    new, existing = judge(findings(lines), written(base.stdout.strip(), args.head))
    for path, line, lint, level, message in new:
        print(f"{path}:{line}: clippy::{lint} ({level}): {message}")
    print(f"Clippy: {len(new)} finding(s) on changed lines, {existing} elsewhere left as they are")
    if exit_code:
        print(f"The command exited {exit_code}" + (": Boat itself failed, run it again" if exit_code == 75 else ""))
    return 1 if new or exit_code else 0


if __name__ == "__main__":
    sys.exit(main())
