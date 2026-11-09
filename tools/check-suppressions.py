"""Refuse new lint suppressions that give no reason (Python 3.11+).

usage: python tools/check-suppressions.py --against REF
       python tools/check-suppressions.py --count

A suppression is `#[allow(..)]`, `#![allow(..)]`, `#[expect(..)]` or `#![expect(..)]`, also
inside `cfg_attr`, in a Rust file under crates/, fuzz/ or vendor/. It needs `reason = "..."`,
saying why the lint is wrong here, so the next reader can tell a fix from a silence.

Most of the tree predates this rule, so `--against REF` compares, per Rust file the working tree
changes or adds against REF, the suppressions without a reason with the same file at REF (at its
old name when renamed). It fails when any file has more. It never asks for a reason on a
suppression a change only moved or left alone. `--count` prints how many suppressions in the
tree still lack one. Comments and string literals are never read as attributes.
Exits 1 on any failure.
"""
import argparse
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TREES = ("crates", "fuzz", "vendor")
START = re.compile(r"#!?\[\s*(?:r#)?(allow|expect|cfg_attr)\s*\(")
REASON = re.compile(r"\breason\s*=")
CHAR_LITERAL = re.compile(r"'(?:\\(?:x[0-9A-Fa-f]{2}|u\{[0-9A-Fa-f_]+\}|.)|[^\\'\n])'")
RAW_STRING = re.compile(r'[bc]?r(#*)"')


def mask(text):
    """The text with comments and the insides of string and char literals blanked, newlines and
    length kept, so what is left is code."""
    out = list(text)
    length = len(text)
    index = 0

    def blank(start, end):
        for position in range(start, end):
            if out[position] != "\n":
                out[position] = " "

    while index < length:
        if text.startswith("//", index):
            end = text.find("\n", index)
            end = length if end < 0 else end
            blank(index, end)
            index = end
        elif text.startswith("/*", index):
            depth, end = 1, index + 2
            while end < length and depth:
                if text.startswith("/*", end):
                    depth, end = depth + 1, end + 2
                elif text.startswith("*/", end):
                    depth, end = depth - 1, end + 2
                else:
                    end += 1
            blank(index, end)
            index = end
        elif (raw := RAW_STRING.match(text, index)) and not (index and (text[index - 1].isalnum() or text[index - 1] == "_")):
            close = '"' + raw.group(1)
            end = text.find(close, raw.end())
            end = length if end < 0 else end + len(close)
            blank(raw.end(), max(raw.end(), end - len(close)))
            index = end
        elif text[index] == '"':
            end = index + 1
            while end < length and text[end] != '"':
                end += 2 if text[end] == "\\" else 1
            blank(index + 1, min(end, length))
            index = end + 1
        elif text[index] == "'" and (literal := CHAR_LITERAL.match(text, index)):
            blank(index + 1, literal.end() - 1)
            index = literal.end()
        else:
            index += 1
    return "".join(out)


def attribute_end(code, start):
    """The index just past the `)` closing the attribute whose `(` is at `start`, in masked code."""
    depth = 0
    for index in range(start, len(code)):
        if code[index] == "(":
            depth += 1
        elif code[index] == ")":
            depth -= 1
            if depth == 0:
                return index + 1
    return len(code)


def split_items(inner):
    """The comma-separated items of an attribute's arguments, at the top nesting level."""
    items, depth, begin = [], 0, 0
    for index, char in enumerate(inner):
        if char in "([{":
            depth += 1
        elif char in ")]}":
            depth -= 1
        elif char == "," and depth == 0:
            items.append(inner[begin:index])
            begin = index + 1
    items.append(inner[begin:])
    return items


def missing_reasons(name, body):
    """How many suppressions in `name(body)` have no reason. A `cfg_attr` holds one per
    suppression among the items after its condition, each needing its own."""
    if name != "cfg_attr":
        return 0 if REASON.search(body) else 1
    total = 0
    for item in split_items(body)[1:]:
        item = item.strip()
        match = re.match(r"(?:r#)?(allow|expect|cfg_attr)\s*\(", item)
        if match:
            total += missing_reasons(match.group(1), item[match.end():item.rfind(")")])
    return total


def unreasoned(text):
    """The 1-based line of each suppression with no reason, once per suppression."""
    code = mask(text)
    found = []
    for match in START.finditer(code):
        end = attribute_end(code, match.end() - 1)
        line = code.count("\n", 0, match.start()) + 1
        found += [line] * missing_reasons(match.group(1), code[match.end(): end - 1])
    return found


def git(*args):
    return subprocess.run(["git", *args], cwd=ROOT, capture_output=True, text=True, encoding="utf-8")


def rust_files(names):
    return [n for n in names if n.endswith(".rs") and n.split("/", 1)[0] in TREES]


def parse_name_status(output):
    """`(name, the name to read at the revision)` for each added, copied, modified or renamed
    entry of `git diff --name-status -z -M`. A name the revision lacks reads as empty."""
    fields = output.split("\0")
    pairs, index = [], 0
    while index < len(fields) and fields[index]:
        if fields[index][0] in "RC":
            pairs.append((fields[index + 2], fields[index + 1]))
            index += 3
        else:
            pairs.append((fields[index + 1], fields[index + 1]))
            index += 2
    return pairs


def changed_files(revision):
    """The Rust files the working tree adds, changes or renames, with their names at the revision;
    untracked files too, which may be tracked there."""
    diff = git("diff", "--name-status", "-z", "-M", "--diff-filter=ACMR", revision).stdout
    pairs = parse_name_status(diff)
    for name in git("ls-files", "-z", "--others", "--exclude-standard").stdout.split("\0"):
        if name:
            pairs.append((name, name))
    return sorted((new, old) for new, old in pairs if rust_files([new]))


def at_revision(revision, name):
    shown = git("show", f"{revision}:{name}")
    return shown.stdout if shown.returncode == 0 else ""


def compare(revision):
    failures = []
    for name, old in changed_files(revision):
        path = ROOT / name
        if not path.is_file():
            continue
        now = unreasoned(path.read_text(encoding="utf-8", errors="replace"))
        before = unreasoned(at_revision(revision, old))
        if len(now) > len(before):
            lines = ", ".join(str(n) for n in now)
            failures.append(f"{name}: {len(now)} without a reason (was {len(before)}), at lines {lines}")
    return failures


def count():
    total = 0
    for name in rust_files(git("ls-files", "-z").stdout.split("\0")):
        path = ROOT / name
        if path.is_file():
            total += len(unreasoned(path.read_text(encoding="utf-8", errors="replace")))
    return total


def main():
    parser = argparse.ArgumentParser()
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--against", metavar="REF")
    group.add_argument("--count", action="store_true")
    args = parser.parse_args()
    if args.count:
        print(count())
        return 0
    failures = compare(args.against)
    if failures:
        print("New lint suppressions need `reason = \"...\"`:")
        print("\n".join(failures))
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
