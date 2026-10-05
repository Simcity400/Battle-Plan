"""Check a candidate Git tree for private artifacts and common sensitive literals.

This is a small publication guard, not a complete secret scanner or a rights audit.
It reads Git blobs, never untracked files, and reports locations without values.
"""
import argparse
import re
import subprocess
from pathlib import PurePosixPath

PRIVATE_ROOTS = (
    "captures/", "tools/re/data/", "ghidra_project/", ".claude/", ".codex/",
    ".agents/", ".aws/", ".ssh/", ".pnpm-store/", "build_slots/",
    "build_slots_check/", ".cargo-target/", "target/", "node_modules/", "ui/dist/",
    "crates/viewer/examples/map_shader_probe/",
)
PRIVATE_FILES = {"collector.json", "viewer.json", ".mcp.json",
                 "crates/viewer/examples/map_shader_probe.rs"}
PRIVATE_SUFFIXES = {".battleplan", ".hoi4", ".h4s", ".h4cp", ".dmp", ".gbf",
                    ".sqlite", ".sqlite3", ".db", ".log", ".exe", ".dll", ".pdb",
                    ".obj", ".spv", ".dxil", ".cso", ".dds", ".tga", ".ttf", ".otf"}
# Original application art and synthetic codec/archive vectors, reviewed separately.
# Changing any bytes requires a new provenance review, not just reusing a filename.
REVIEWED_BINARY_BLOBS = {
    'crates/desktop/icons/icon.png': '0feac6da5527a4eb3b4cc1c73d5a0dadfad8364c',
    'crates/desktop/icons/icon.ico': 'c16a625c6d85df5530b129ec70ed4ef4e9bbf837',
    'crates/installed-content/src/content/testdata/zipcrypto_deflated.zip': '21fc9c5c294a841c2a9b5580b98223fa5829444a',
    'crates/installed-content/src/content/testdata/zipcrypto_mixed.zip': 'f13d70c08741397ab2b2d41a03e834d6cc20b932',
    'crates/installed-content/src/content/testdata/zipcrypto_stored.zip': 'e38c928b6bc802ba08188725031e8f58787546e7',
    'crates/installed-content/src/image/native/testdata/adobe_cmyk.jpg': 'd6f49f2030ef337bc415c165589b7d9b0099daa4',
    'crates/installed-content/src/image/native/testdata/arithmetic.jpg': '51fce0bc81104347a11677de3a98b7636ebff514',
    'crates/installed-content/src/image/native/testdata/baseline_420.jpg': '4db2bb1b822a9d60938e9dbb7fafdbdc9d7383b8',
    'crates/installed-content/src/image/native/testdata/progressive.jpg': 'a29705f82792cf4a0f24c2d2750a3a18fa03cb8e',
    'crates/installed-content/src/image/native/testdata/smartscale_4.jpg': 'e8567367372caea12b1d0f72272337e8bef48f78',
}
LITERALS = {
    "private-key header": re.compile(r"-----BEGIN (?:[A-Z]+ )?PRIVATE KEY-----"),
    "provider-token shape": re.compile(
        r"\b(?:gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{60,}|"
        r"(?:AKIA|ASIA)[A-Z0-9]{16}|xox[baprs]-[A-Za-z0-9-]{20,}|"
        r"sk-(?:proj|svcacct)-[A-Za-z0-9_-]{20,})\b"),
    "URL credentials": re.compile(r"https?://[^/\s:@]+:[^/\s@]+@", re.I),
}
PROFILE = re.compile(r"(?:[A-Z]:[\\/]+Users[\\/]+|/(?:home|Users)/)([^\\/\s\"']+)", re.I)
EXAMPLE_PROFILES = {"public", "default", "user", "username", "example", "<user>",
                    "<username>", "[user]", "..."}


def git(*args):
    return subprocess.check_output(["git", *args])


def entries(revision):
    if revision:
        commit = git("rev-parse", "--verify", "--end-of-options", revision + "^{commit}").decode().strip()
        raw = git("ls-tree", "-rz", commit)
    else:
        raw = git("ls-files", "--stage", "-z")
    for record in raw.split(b"\0"):
        if not record:
            continue
        metadata, path = record.split(b"\t", 1)
        fields = metadata.split()
        if revision:
            mode, kind, oid = fields
            stage = b"0"
        else:
            mode, oid, stage = fields
            kind = b"blob" if mode != b"160000" else b"commit"
        yield path.decode("utf-8", "surrogateescape"), mode, kind, oid, stage


def path_reason(path):
    normalized = path.lower()
    name = PurePosixPath(normalized).name
    if normalized.startswith(PRIVATE_ROOTS) or normalized in PRIVATE_FILES:
        return "private/generated path"
    if name.startswith(".env") and not name.endswith(".example"):
        return "local environment file"
    if ".bak" in PurePosixPath(normalized).suffixes or ".bak-" in name:
        return "local backup"
    if PurePosixPath(normalized).suffix in PRIVATE_SUFFIXES:
        return "private/game/generated file type"
    if PurePosixPath(normalized).suffix in {".png", ".ico", ".jpg", ".jpeg", ".webp", ".zip", ".7z", ".rar"}:
        return "binary artwork/archive requires provenance review"
    return None


def text_findings(text):
    for number, line in enumerate(text.splitlines(), 1):
        for label, pattern in LITERALS.items():
            if pattern.search(line):
                yield number, label
        for match in PROFILE.finditer(line):
            profile = match[1].lower()
            if profile not in EXAMPLE_PROFILES and not profile.startswith(("<", "$", "%", "{")):
                yield number, "personal profile path"
                break


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--revision", help="Scan a committed tree instead of the staged index.")
    args = parser.parse_args()
    failures = 0
    checked = 0
    process = subprocess.Popen(["git", "cat-file", "--batch"], stdin=subprocess.PIPE, stdout=subprocess.PIPE)
    try:
        for path, mode, kind, oid, stage in entries(args.revision):
            checked += 1
            reasons = []
            if stage != b"0" or mode == b"120000" or kind != b"blob":
                reasons.append("unresolved index or indirect content requires review")
            reviewed_binary = REVIEWED_BINARY_BLOBS.get(path) == oid.decode("ascii")
            reason = path_reason(path)
            if reviewed_binary and reason == "binary artwork/archive requires provenance review":
                reason = None
            if reason:
                reasons.append(reason)
            if kind == b"blob":
                process.stdin.write(oid + b"\n")
                process.stdin.flush()
                header = process.stdout.readline().split()
                if len(header) != 3 or header[1] != b"blob":
                    raise RuntimeError("Cannot read candidate Git blob")
                data = process.stdout.read(int(header[2]))
                if process.stdout.read(1) != b"\n":
                    raise RuntimeError("Invalid Git blob boundary")
                if not reviewed_binary:
                    if b"\0" in data:
                        reasons.append("unreviewed binary content")
                    else:
                        for line, label in text_findings(data.decode("utf-8", "replace")):
                            reasons.append(f"{label} at line {line}")
            for reason in reasons:
                print(f"{path}: {reason}")
                failures += 1
    finally:
        process.stdin.close()
        process.stdout.close()
        process.wait()
    print(f"Checked {checked} candidate files; {failures} findings. History, metadata, hosted content and rights require separate review.")
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
