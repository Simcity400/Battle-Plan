"""Refuses anything that would carry what belongs to the machine it was made on (Python 3.11+).

The one leak scanner: release artifacts use the artifact walk below; the repository's commit and
pull-request checks add git front ends over the same core, `Matcher.scan_bytes`.

It looks, in bytes, UTF-8 and UTF-16, case-insensitively and for each kind on its own, for these kinds:
- term: the private terms (names, home folders, private paths, private hosts), never kept in this
  repository. In order: the environment variable LEAK_GATE_TERMS (a JSON list, from the CI secret
  of that name); else `[leak] terms` in the dev config at BATTLEPLAN_DEV_CONFIG, or at
  %LOCALAPPDATA%\\BattlePlan\\dev\\config.toml ($XDG_CONFIG_HOME/battleplan/dev/config.toml
  elsewhere); with none, it refuses to run;
- user-path, steam-path: user folders on any drive, SteamLibrary folders;
- email: addresses other than GitHub noreply ones;
- github-token, private-key, aws-key, slack-token, secret-key: credentials, including minisign
  and Tauri secret keys; updater-key: the updater key's private half (TAURI_SIGNING_PRIVATE_KEY);
- pdb, source-map: debug databases and source maps, which never ship.

tools/leak-gate.allow exempts path globs from one kind each; term and key kinds are never exempt.
It prints only `<file>: <kind> xN` and a total, never what matched, and exits 1 on any match.

usage: leak-gate.py PATH...   (folders whole; installers and archives opened with 7-Zip)
"""
import argparse
import base64
import fnmatch
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ALLOW = ROOT / "tools" / "leak-gate.allow"
LOCAL = b"abcdefghijklmnopqrstuvwxyz0123456789._%+-"
EMAIL = re.compile(rb"@(?<=[a-z0-9._%+-]{2}@)[a-z0-9-]{2,}(?:\.[a-z0-9-]+)*\.(?:com|org|net|io|dev|edu|gov|info|biz|me|co|uk|de|fr|"
                   rb"au|ca|us|eu|ru|jp|cn|nl|se|no|fi|ch|at|be|it|es|pl|cz|br|in|nz|app|xyz|ai|gg|sh|cc)(?![a-z0-9-])")
NOREPLY = re.compile(rb"@users\.noreply\.github\.com$|^noreply@github\.com$")
USER_TAIL = rb"[\\/]+users[\\/]+[^\\/\x00\s\"']+"
RULES = {
    "user-path": [rb":(?<=[a-z]:)" + USER_TAIL, rb"c(?<=[\\/]c)" + USER_TAIL],
    "steam-path": [rb"steamlibrary"],
    "github-token": [rb"gh[pousr]_[a-z0-9]{30,}", rb"github_pat_[a-z0-9_]{30,}"],
    "private-key": [rb"-----begin[a-z ]*private key-----"],
    "aws-key": [rb"akia[0-9a-z]{16}"],
    "slack-token": [rb"xox[abprs]-[a-z0-9-]{10,}"],
    "secret-key": [rb"encrypted secret key",
                   *(re.escape(base64.b64encode(b"untrusted comment: %s encrypted secret key" % tool)[:40].lower())
                     for tool in (b"rsign", b"minisign"))],
}
NEVER_EXEMPT = {"term", "updater-key", "github-token", "private-key", "aws-key", "slack-token", "secret-key"}
OPENED = {"Nsis", "zip", "7z", "Cab", "Rar", "gzip", "tar", "xz"}
BINARY = {"PE"}
ARCHIVES = {".exe", ".zip", ".7z", ".cab", ".gz", ".tar", ".xz"}


def dev_config():
    if configured := os.environ.get("BATTLEPLAN_DEV_CONFIG"):
        return Path(configured)
    if os.name == "nt" and os.environ.get("LOCALAPPDATA"):
        return Path(os.environ["LOCALAPPDATA"]) / "BattlePlan" / "dev" / "config.toml"
    base = os.environ.get("XDG_CONFIG_HOME") or (Path.home() / ".config")
    return Path(base) / "battleplan" / "dev" / "config.toml"


def private_terms():
    if os.environ.get("LEAK_GATE_TERMS"):
        source = "LEAK_GATE_TERMS"
        try:
            terms = json.loads(os.environ["LEAK_GATE_TERMS"])
        except ValueError:
            sys.exit("LEAK_GATE_TERMS is not a JSON list of strings.")
    else:
        config = dev_config()
        source = f"[leak] terms in {config}"
        if not config.is_file():
            sys.exit(f"No private terms: set LEAK_GATE_TERMS, or configure {source}.")
        try:
            terms = tomllib.loads(config.read_text(encoding="utf-8")).get("leak", {}).get("terms")
        except tomllib.TOMLDecodeError as error:
            sys.exit(f"{config} is not TOML: {error}")
    if not isinstance(terms, list) or not terms or not all(isinstance(term, str) and len(term) >= 3 for term in terms):
        sys.exit(f"{source} must be a non-empty list of strings of 3 characters or more.")
    return terms


def updater_key():
    secret = os.environ.get("TAURI_SIGNING_PRIVATE_KEY", "").strip()
    if not secret:
        return []
    forms = [secret]
    try:
        forms += [line for line in base64.b64decode(secret).decode("utf-8").splitlines() if len(line) >= 16]
    except ValueError:
        pass
    return forms


def literal(strings):
    """One pattern for each string, also with its slashes turned the other way."""
    forms = set()
    for value in strings:
        forms |= {value, value.replace("\\", "/"), value.replace("/", "\\")}
    lowered = {form.encode("utf-8").lower() for form in forms}
    return b"|".join(re.escape(form) for form in sorted(lowered, key=lambda form: (-len(form), form)))


def load_allow(path=ALLOW):
    allow = []
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        line = line.split("#", 1)[0].strip()
        if not line:
            continue
        parts = line.split()
        if len(parts) != 2:
            sys.exit(f"{path}:{number}: write one path glob and the one kind it exempts.")
        glob, kind = parts
        if kind in NEVER_EXEMPT:
            sys.exit(f"{path}:{number}: {kind} is never exempt.")
        allow.append((glob, kind))
    return allow


class Matcher:
    """The kinds to look for, compiled once."""

    def __init__(self, terms, key_forms, allow):
        rules = {"term": [literal(terms)], **RULES}
        if key_forms:
            rules["updater-key"] = [literal(key_forms)]
        self.patterns = [(kind, re.compile(pattern)) for kind, patterns in rules.items() for pattern in patterns]
        self.allow = allow

    def exempt(self, name, kind):
        return kind not in NEVER_EXEMPT and any(kind == allowed and fnmatch.fnmatchcase(name, glob) for glob, allowed in self.allow)

    def scan_bytes(self, name, data):
        """(kind, count) for each kind found in `data`, the contents of `name`."""
        counts = {}
        lowered = name.lower()
        if lowered.endswith(".pdb"):
            counts["pdb"] = 1
        if lowered.endswith((".js.map", ".css.map")):
            counts["source-map"] = 1
        data = data.lower()
        views = [data, *(data[offset:len(data) - ((len(data) - offset) % 2)].decode("utf-16-le", errors="replace")
                         .encode("latin-1", errors="replace") for offset in (0, 1))]
        for view in views:
            for kind, pattern in self.patterns:
                if found := sum(1 for _ in pattern.finditer(view)):
                    counts[kind] = counts.get(kind, 0) + found
            if found := addresses(view):
                counts["email"] = counts.get("email", 0) + found
        return [(kind, count) for kind, count in counts.items() if not self.exempt(name, kind)]


def addresses(view):
    """The email addresses in `view`, one per @ and each judged whole, less GitHub noreply ones."""
    count = 0
    for match in EMAIL.finditer(view):
        before = view[max(0, match.start() - 8):match.start()]
        count += not NOREPLY.search(before[len(before.rstrip(LOCAL)):] + match.group())
    return count



def seven_zip():
    found = shutil.which("7z") or (r"C:\Program Files\7-Zip\7z.exe" if os.name == "nt" else None)
    if not found or not Path(found).exists():
        sys.exit("7-Zip was not found, so installers could not be opened.")
    return found


def archive_type(tool, path, label):
    listed = subprocess.run([tool, "l", "-slt", str(path)], capture_output=True)
    found = re.search(rb"^Type = (\S+)", listed.stdout, re.MULTILINE) if listed.returncode == 0 else None
    if not found:
        sys.exit(f"{label} could not be identified (7-Zip exit {listed.returncode}), so it was not checked.")
    return found.group(1).decode()


def files(paths):
    for path in paths:
        if path.is_dir():
            yield from (entry for entry in sorted(path.rglob("*")) if entry.is_file())
        elif path.is_file():
            yield path
        else:
            sys.exit(f"{path} does not exist.")


def walk(path, label, matcher, tool, scratch, depth, found):
    found.extend((label, kind, count) for kind, count in matcher.scan_bytes(label, path.read_bytes()))
    if path.suffix.lower() not in ARCHIVES:
        return
    kind = archive_type(tool, path, label)
    if kind in BINARY and not path.name.lower().endswith("-setup.exe"):
        return
    if kind not in OPENED:
        sys.exit(f"{label} is a {kind} file the gate cannot open, so it was not checked.")
    if depth >= 4:
        sys.exit(f"{label} nests archives too deep to open.")
    inside = Path(tempfile.mkdtemp(dir=scratch))
    opened = subprocess.run([tool, "x", "-y", f"-o{inside}", str(path)], capture_output=True)
    if opened.returncode != 0:
        sys.exit(f"{label} could not be opened ({kind}, 7-Zip exit {opened.returncode}), so it was not checked.")
    for entry in files([inside]):
        walk(entry, f"{label}/{entry.relative_to(inside).as_posix()}", matcher, tool, scratch, depth + 1, found)


def report(found, checked):
    for label, kind, count in found:
        print(f"{label}: {kind} x{count}")
    total = sum(count for _, _, count in found)
    print(f"Leak gate: {checked} files checked, {total} matches.")
    return 1 if total else 0


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("paths", nargs="+", type=Path)
    args = parser.parse_args()
    matcher = Matcher(private_terms(), updater_key(), load_allow())
    tool = seven_zip()
    found = []
    with tempfile.TemporaryDirectory() as scratch:
        given = list(files(args.paths))
        for path in given:
            try:
                label = path.resolve().relative_to(Path.cwd().resolve()).as_posix()
            except ValueError:
                label = path.name
            walk(path, label, matcher, tool, scratch, 0, found)
    sys.exit(report(found, len(given)))


if __name__ == "__main__":
    main()
