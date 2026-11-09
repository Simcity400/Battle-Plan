"""Refuse tracked files that name this machine's folders (Python 3.11+).

usage: python tools/check-machine-paths.py

Finds, in every tracked text file:
- `checkout`: a drive path, Windows or MSYS style, into the maintainer's checkout or build root;
- `home`: a drive path into a user's profile folder whose name is not a placeholder (`Player`,
  `User`, `UserName`, `Public`, `Default`, `Name`, `You`, `Example`).
Take such paths from arguments, settings or the environment instead.

tools/machine-paths.allow exempts path globs from one kind each, for scripts that exist to
configure this machine. It prints `<file>:<line>: <kind>` and exits 1 on any finding; the private
terms themselves are tools/leak-gate.py's, and never appear here.
"""
import fnmatch
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ALLOW = ROOT / "tools" / "machine-paths.allow"
DRIVE = r"(?:(?<![A-Za-z0-9])[A-Za-z]:[\\/]+|(?<![A-Za-z0-9_.])/[a-z]/)"
CHECKOUT = re.compile(DRIVE + r"hoi4_(?:mp_stats|build)\b", re.IGNORECASE)
HOME = re.compile(
    DRIVE + r"Users[\\/]+(?!(?:Player|User|UserName|Public|Default|Name|You|Example)\b)[A-Za-z0-9][A-Za-z0-9._-]*",
    re.IGNORECASE,
)
KINDS = (("checkout", CHECKOUT), ("home", HOME))
SKIPPED = ("Cargo.lock", "package-lock.json")


def allowed(text):
    """Each (glob, kind) the allow file lists, ignoring blank and `#` lines."""
    rules = []
    for line in text.splitlines():
        line = line.strip()
        if line and not line.startswith("#"):
            glob, kind = line.rsplit(None, 1)
            rules.append((glob, kind))
    return rules


def scan(name, text, rules):
    """The `(line, kind)` of each finding in one file's text."""
    exempt = {kind for glob, kind in rules if fnmatch.fnmatch(name, glob)}
    found = []
    for number, line in enumerate(text.splitlines(), 1):
        for kind, pattern in KINDS:
            if kind not in exempt and pattern.search(line):
                found.append((number, kind))
    return found


def tracked():
    listed = subprocess.run(["git", "ls-files", "-z"], cwd=ROOT, capture_output=True, check=True)
    return [n for n in listed.stdout.decode("utf-8").split("\0") if n and Path(n).name not in SKIPPED]


def main():
    rules = allowed(ALLOW.read_text(encoding="utf-8")) if ALLOW.exists() else []
    failures = []
    for name in tracked():
        path = ROOT / name
        if not path.is_file():
            continue
        data = path.read_bytes()
        if b"\0" in data:
            continue
        for number, kind in scan(name, data.decode("utf-8", errors="replace"), rules):
            failures.append(f"{name}:{number}: {kind}")
    if failures:
        print("\n".join(failures))
        print(f"{len(failures)} path(s) name this machine; take them from arguments, settings or the environment.")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
