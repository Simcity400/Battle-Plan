#!/usr/bin/env python3
"""Fail closed when a source citation cannot be resolved, repo-wide.

WHAT A CITATION IS.  A reference written in a comment or a design document that
names a Rust source file and a place inside it.  Two written forms:

    `attach_log.rs:<line>`          a LINE NUMBER
    `attach_log.rs::dll_main`       an ITEM NAME

(The line form is written with a placeholder here on purpose: this file is
inside the scan roots, and a worked example spelt with real digits would be a
citation this gate then demanded be grandfathered.  The item form needs no such
care -- it is spelt exactly as it would be written, and it resolves.)

The first rots.  A line number is a position in a file whose own line numbers
move on the next insertion above them; nothing compiles a comment, so nothing
notices.  Lane HK measured the damage on one tree -- 1,143 line citations in the
dNN wire modules, of a sample of ten, six landing on unrelated text -- and lane
`runs/prose_refs/` had already converted the citations of ONE file
(`world_fold.rs`) to the item form and guarded the result with
``verify_world_fold_citations.py``.  This is that guard generalized to the whole
repository.

The second form does not rot the same way.  An item name survives every
insertion above it, and its one failure mode -- the item was renamed or deleted
-- is exactly what a checker can detect.  That is the whole argument for the
conversion: it moves citations from a class nothing can check into a class this
script checks on every run.

THREE RULES.

  L (line references are a ratchet).  Every line citation must already be listed
    in ``citation_integrity_allowance.json`` under its citing file.  A NEW one
    fails.  The list may only shrink -- an unused entry is reported, never an
    error, so deleting a citation never forces an allowance edit in the same
    commit, while adding one always does.

  U (a grandfathered citation must still land).  A line citation whose file does
    not exist, whose basename is carried by several files, or whose line is past
    the end of the file, fails -- unless it is listed under ``unresolved`` with a
    reason.  This is the half that catches rot as it happens rather than
    freezing it: a grandfathered citation is permitted to EXIST, not permitted to
    stop resolving.

    Note what U does NOT claim.  A citation that lands inside its file lands
    SOMEWHERE, not necessarily on what the citing sentence means -- no checker
    can read that.  That is why L exists: the only durable fix is the item form,
    and L is what stops the population growing while it is converted.

  I (item references must check out).  An item citation must name exactly one
    file that defines the item, and a multi-segment path must be a real route
    through it: each segment has to be declared inside the body of the one
    before it.  There is no ratchet here and no budget -- an item citation is
    checkable, so it is checked.  The few that do not resolve today are listed
    under ``items`` with a reason.

WHAT IS NOT SCANNED.  Evidence trees -- any ``runs/`` directory and any
``re_evidence/`` directory.  AGENTS.md makes those append-only: a completed run
records what was true when it ran, and a stale citation inside one is frozen
history, not a defect.  Editing one to satisfy a gate would be falsifying the
record.  ``census.py`` in this lane's run directory counts them so the size of
that frozen population is known (633 citations) rather than merely excluded.

EXTERNAL SOURCES.  ``libc-0.2.189/src/psp.rs:505`` is a real citation of a real
file that is not in this repository.  Those are listed under
``external_sources`` by path and skipped -- a checker that demanded they resolve
would be demanding the vendor tree be checked in.

OVERLAP WITH ``verify_world_fold_citations.py`` IS DELIBERATE.  That gate is
stricter inside its own territory: it bans line numbers in the dNN modules
outright, pins the fold's line range with a digest, and pins the callee closure.
This one is broader and weaker: it covers every file in the scan roots and asks
only that a citation resolve.  Both ratchets only ever fail on a citation being
ADDED, and both accept an unused allowance entry, so the cost of the overlap is
one extra allowance edit on the rare occasion a new world_fold line number is
deliberately grandfathered -- and the benefit is that neither gate's coverage
depends on the other still existing.

WHY THERE IS A CACHE.  Lane JD timed a full battery and found this gate, as run
by ``recorder_host``'s ``citation_integrity`` test, was 70.8 s of the 106.6 s the
Extractor serial half holds the machine-wide mutex -- 66% of it, one test.  Lane
JN profiled where that goes: the three citation patterns swept over the 98.2 MB
of text in the scan set cost 61.4 s of a 64.5 s run, and everything else --
walking the tree, sniffing it for binaries, reading every byte and hashing it --
costs 0.7 s together.  The whole of the expense is re-asking a question about
files that did not change.

WHAT MAKES SKIPPING IT SOUND.  Verifying ONE citing file is a deterministic
function of that file's bytes, of the allowance, of this script, and of a set of
QUESTIONS ABOUT THE REST OF THE TREE -- which files carry a basename, how long
one is, what names it declares.  ``Tree`` is the only route to those questions,
so every one of them is recorded, with the answer it gave, while the file is
verified.  A cached result is reused only when the file's bytes hash the same AND
every recorded question still gives the recorded answer; by determinism the run
would then replay identically, so the recorded result IS this run's result.
Anything else -- one recorded answer that moved, a cache written by another
allowance or another copy of this script, a cache that will not parse, an entry
whose own digest does not match its contents -- verifies the file again.  The
cache is therefore incapable of hiding a citation: it can only be slower than it
could have been.

Two consequences worth stating plainly.  A changed ALLOWANCE or a changed SCRIPT
re-verifies the whole tree, because both are inputs to every file's result and
both are hashed into the cache's key.  And ``--full`` ignores the cache for
reading while still refreshing it, which is what the central validation runner
passes: every landing gets one verification that consulted no cache at all, and
it leaves the cache warm for the in-lane runs that follow.  ``--full`` is not a
second implementation of the rules -- both paths call the same ``verify_one``,
and the only difference between them is whether a recorded result is consulted.
"""

from __future__ import annotations

import argparse
import bisect
import hashlib
import importlib.util
import io
import json
import marshal
import os
import re
import sys
import types
import unicodedata
from collections import Counter
from pathlib import Path


BINARY_SUFFIXES = frozenset(
    """
    .png .jpg .jpeg .gif .bmp .ico .tga .dds .webp .svgz
    .zip .gz .xz .zst .7z .rar .tar .cab .msi
    .exe .dll .pdb .lib .obj .so .dylib .a .o .bin .dat .pak
    .ttf .otf .woff .woff2 .wav .mp3 .ogg .mp4 .avi .mkv
    .pyc .pyd .class .jar .rlib .rmeta .db .sqlite .parquet
    """.split()
)
BINARY_SNIFF_BYTES = 8192

UNSCANNED = (
    "extractor/native/scripts/citation_integrity_allowance.json",
    "target/citation_integrity_cache.json",
)

ALLOWANCE = Path("extractor/native/scripts/citation_integrity_allowance.json")

CACHE = Path("target/citation_integrity_cache.json")
CACHE_SCHEMA = "battle-plan.citation_integrity_cache.v1"

SKIP_DIRS = frozenset(
    {"target", ".git", ".claude", "node_modules", ".cargo-target", ".venv",
     ".pnpm-store", ".reference", ".t3-worktrees", "captures"}
)
FROZEN_DIRS = frozenset({"runs", "re_evidence"})



def _class_body(codepoints: list[int]) -> str:
    """Sorted codepoints -> the interior of a regex character class, as ranges."""
    parts: list[str] = []
    start = previous = None
    for codepoint in codepoints:
        if start is None:
            start = previous = codepoint
        elif codepoint == previous + 1:
            previous = codepoint
        else:
            parts.append((start, previous))
            start = previous = codepoint
    if start is not None:
        parts.append((start, previous))
    return "".join(
        f"\\U{low:08x}" if low == high else f"\\U{low:08x}-\\U{high:08x}"
        for low, high in parts
    )


def _xid_extras() -> tuple[str, str]:
    """The XID characters Python's `\\w` / `[^\\W\\d]` classes do not already cover.

    Continuation first, one sweep of the code space; the START extras are a
    subset of it, because a character that starts an identifier also continues
    one, and every character `[^\\W\\d]` misses is by definition not a `\\w`
    character (a `\\w` digit cannot start an identifier at all).
    """
    word = re.compile(r"\w")
    word_not_digit = re.compile(r"[^\W\d]")
    continuation = [
        codepoint
        for codepoint in range(0x110000)
        if not word.match(chr(codepoint)) and ("_" + chr(codepoint)).isidentifier()
    ]
    beginning = [
        codepoint
        for codepoint in continuation
        if chr(codepoint).isidentifier() and not word_not_digit.match(chr(codepoint))
    ]
    return _class_body(beginning), _class_body(continuation)


XID_START_EXTRA, XID_CONT_EXTRA = _xid_extras()
IDENT_START = f"(?:[^\\W\\d]|[{XID_START_EXTRA}])"
IDENT_CONT = f"[\\w{XID_CONT_EXTRA}]"
IDENT = f"{IDENT_START}{IDENT_CONT}*"
PATH_CHAR = f"[\\w.\\-{XID_CONT_EXTRA}]"
NOT_AFTER = f"(?<![\\w.\\-{XID_CONT_EXTRA}/\\\\])"


def normalized(name: str) -> str:
    """NFC, so a decomposed citation and a composed declaration compare equal."""
    return unicodedata.normalize("NFC", name)


LINE_CITATION = re.compile(
    rf"{NOT_AFTER}"
    rf"(?P<path>(?:{PATH_CHAR}+[/\\])*)"
    rf"(?P<base>{PATH_CHAR}+\.rs)"
    r":(?P<line>\d+)"
    r"(?:-(?P<end>\d+))?"
    r"(?![0-9])"
)
SPLIT_RANGE = re.compile(
    r"`?[ \t]*(?:\r?\n[ \t]*(?://[/!]?|\#+|\*)?[ \t]*)?"
    r"[-–—]"
    r"[ \t]*(?:\r?\n[ \t]*(?://[/!]?|\#+|\*)?[ \t]*)?"
    r"`?:(?P<end>\d+)(?![0-9])"
)
MALFORMED_ITEM = re.compile(
    rf"{NOT_AFTER}"
    rf"(?P<path>(?:{PATH_CHAR}+[/\\])*)"
    rf"(?P<base>{PATH_CHAR}+\.rs)"
    rf"::(?P<item>(?:(?:r#)?{IDENT}::)*(?:r#)?)"
    rf"(?!{IDENT_CONT}|<)"
)
ITEM_CITATION = re.compile(
    rf"{NOT_AFTER}"
    rf"(?P<path>(?:{PATH_CHAR}+[/\\])*)"
    rf"(?P<base>{PATH_CHAR}+\.rs)"
    rf"::(?P<item>(?:r#)?{IDENT}(?:::(?:r#)?{IDENT})*)"
    rf"(?!{IDENT_CONT}|#|::)"
)


def citation_matches(
    pattern: "re.Pattern[str]", text: str
) -> "Iterator[re.Match[str]]":
    """`pattern.finditer(text)`, restricted to the lines that contain `.rs`.

    Identical output, by two properties every citation pattern here has: a
    match always contains the literal `.rs` (each pattern's `base` group ends
    `\\.rs`), and a match never spans a newline (no character class in them
    admits one).  So only lines containing `.rs` can hold a match, and scanning
    with `pos`/`endpos` on the WHOLE string -- never a slice -- keeps the
    look-behind reading the true preceding character and makes the trailing
    look-aheads see end-of-region exactly where the full scan saw a newline,
    which no citation pattern's look-ahead distinguishes from one.

    What it buys: `(?:{PATH_CHAR}+[/\\\\])*{PATH_CHAR}+\\.rs` backtracks
    quadratically over every long run of path characters that does NOT end in
    `.rs`, which is nearly all of a large generated file.  Skipping the lines
    that cannot match turned a 190-second restamp into seconds; the gate's own
    scan is the same three patterns over the same files.
    """
    pos = 0
    length = len(text)
    while True:
        hit = text.find(".rs", pos)
        if hit == -1:
            return
        line_start = text.rfind("\n", 0, hit) + 1
        line_end = text.find("\n", hit)
        if line_end == -1:
            line_end = length
        yield from pattern.finditer(text, line_start, line_end)
        pos = line_end + 1


class VerificationError(RuntimeError):
    """A citation in this tree does not resolve, or is not grandfathered."""


_BLANKABLE = re.compile(
    r"(?P<raw>(?:b?r)(?P<hashes>\#*)\"(?s:.*?)\"(?P=hashes))"
    r"|(?P<string>b?\"(?:\\(?s:.)|[^\"\\])*\")"
    r"|(?P<char>b?\'(?:\\(?s:.)|[^\'\\])\')"
    r"|(?P<line>//[^\n]*)"
    r"|(?P<block>/\*(?s:.*?)\*/)"
)


def _blank(match: re.Match[str]) -> str:
    """Replace a span with spaces, keeping its length and its newlines."""
    text = match.group(0)
    if match.group("block") and "/*" in text[2:]:
        raise VerificationError(
            "a nested block comment was found; this scanner cannot tell where "
            "it ends, so the scope index it feeds cannot be trusted. Unnest it, "
            "or teach the scanner to count."
        )
    return "".join("\n" if char == "\n" else " " for char in text)


def _sanitize_rust(source: str) -> str:
    """Blank comments and literals, preserving length and newlines.

    `attach_log.rs` embeds JSON in string literals, so a brace walk over raw
    source does not balance, and Rust-looking text inside a comment would
    otherwise declare items that no citation can reach.

    This is a SECOND implementation of a scanner this repository already has
    (`verify_world_fold_citations.sanitize_rust`), and that is deliberate: the
    original walks character by character in Python, which costs three minutes
    over the ~40 MB of Rust this gate indexes, against three seconds here.  The
    two are held to the same behaviour by `test_verify_citation_integrity.py`,
    which runs both over the same tricky fixtures -- nested-free block comments,
    hashed raw strings, escaped quotes, char literals, lifetimes -- and requires
    identical output.  A divergence fails that test rather than silently
    changing what this gate can see.
    """
    return _BLANKABLE.sub(_blank, source)


def _impl_subject(tail: str) -> str:
    """The type an `impl` header applies to, generics and trait stripped."""
    tail = " " + " ".join(tail.split()) + " "
    subject = tail.split(" for ", 1)[1] if " for " in tail else tail
    subject = subject.split(" where ", 1)[0].strip()
    if subject.startswith("<"):
        depth = 0
        for index, char in enumerate(subject):
            if char == "<":
                depth += 1
            elif char == ">":
                depth -= 1
                if depth == 0:
                    subject = subject[index + 1 :].strip()
                    break
    subject = subject.split("<")[0].strip().strip("&").strip()
    return subject.split("::")[-1].strip()


def _immediate_depth_spans(body: str) -> list[tuple[int, int]]:
    """The stretches of ``body`` that sit directly inside its outer braces.

    ``body`` starts at the owner's ``{``, so everything at brace depth exactly 1
    is a declaration the owner makes itself; anything deeper belongs to whatever
    nested block opened around it.  Returned as sorted spans rather than a
    per-character depth array, because the owner here can be a 5 MB file's
    module body.
    """
    spans: list[tuple[int, int]] = []
    depth = 0
    start: int | None = None
    for index, char in enumerate(body):
        if char in "{([":
            depth += 1
            if depth == 1:
                start = index + 1
            elif depth == 2 and start is not None:
                spans.append((start, index))
                start = None
        elif char in "})]":
            depth -= 1
            if depth == 1:
                start = index + 1
            elif depth == 0 and start is not None:
                spans.append((start, index))
                start = None
    if start is not None:
        spans.append((start, len(body)))
    return spans


def index_scopes(source: str) -> dict[tuple[str, ...], tuple[frozenset[str], frozenset[str]]]:
    """route -> (what it DECLARES, what it lists as MEMBERS).

    Four properties, each of which cost a review round to learn:

    * built from SANITIZED source, so Rust-looking text inside a comment or a
      string literal declares nothing (round 6);
    * keyed by the FULL LEXICAL ROUTE, so two modules that each define a `Row`
      do not lend each other members (round 5);
    * collected only at each body's IMMEDIATE item depth, so a `fn` -- or a
      `struct` -- declared inside a function body is local to it and is neither
      a member nor a route child (rounds 4 and 6).  A function body is a route
      barrier: nothing inside one is addressable from outside it;
    * every pattern is run over the file ONCE, not once per scope.  The
      item-level spans of different scopes are disjoint by construction (a
      position at one scope's immediate depth is inside no deeper brace), so one
      sorted span list answers "which route owns this match" for every match.

    Deliberately lexical.  A scope form it cannot parse contributes no members,
    and the citation naming one is then reported rather than silently accepted --
    the failure direction that asks a human to look.
    """
    sanitized = _sanitize_rust(source)

    found: list[tuple[int, int, int, str]] = []
    for header in SCOPE_HEADER.finditer(sanitized):
        open_brace = sanitized.find("{", header.end())
        semicolon = sanitized.find(";", header.end())
        if open_brace < 0 or (0 <= semicolon < open_brace):
            continue
        depth = 0
        close_brace = -1
        for index in range(open_brace, len(sanitized)):
            char = sanitized[index]
            if char == "{":
                depth += 1
            elif char == "}":
                depth -= 1
                if depth == 0:
                    close_brace = index
                    break
        if close_brace < 0:
            continue
        owner = header.group("name") or _impl_subject(
            sanitized[header.end() : open_brace]
        )
        if owner:
            found.append(
                (header.start(), open_brace, close_brace, owner, header.group("kind"))
            )
    found.sort()

    file_level = _outermost_spans(sanitized)
    spans: list[tuple[int, int, tuple[str, ...], bool]] = [
        (span_start, span_end, (), False) for span_start, span_end in file_level
    ]
    declarations: dict[tuple[str, ...], set[str]] = {(): set()}
    members: dict[tuple[str, ...], set[str]] = {(): set()}

    stack: list[tuple[int, tuple[str, ...] | None, list[tuple[int, int]]]] = []
    field_bodies: list[tuple[tuple[str, ...], str]] = []
    for header_start, body_start, body_end, owner, kind in found:
        while stack and header_start > stack[-1][0]:
            stack.pop()
        parent_route: tuple[str, ...] | None
        if stack:
            parent_end, parent_route, parent_level = stack[-1]
        else:
            parent_route, parent_level = (), file_level
        addressable = _within(parent_level, header_start)
        if parent_route is None or not addressable:
            stack.append((body_end, None, []))
            continue
        owner = normalized(owner)
        route = parent_route + (owner,)
        declarations.setdefault(parent_route, set()).add(owner)
        level = [
            (body_start + span_start, body_start + span_end)
            for span_start, span_end in _immediate_depth_spans(
                sanitized[body_start:body_end]
            )
        ]
        declarations.setdefault(route, set())
        members.setdefault(route, set())
        carries_fields = kind in ("struct", "enum", "union")
        if carries_fields:
            field_bodies.append((route, sanitized[body_start:body_end]))
        spans.extend(
            (span_start, span_end, route, carries_fields)
            for span_start, span_end in level
        )
        stack.append((body_end, route, level))

    spans.sort()
    starts = [span_start for span_start, _end, _route, _fields in spans]

    def span_at(position: int):
        """-> (route, declares-fields), or ``(None, False)`` outside every body."""
        index = bisect.bisect_right(starts, position) - 1
        if index < 0:
            return None, False
        span_start, span_end, route, carries = spans[index]
        if span_start <= position < span_end:
            return route, carries
        return None, False

    for route, body in field_bodies:
        members.setdefault(route, set()).update(entry_names(body))

    for pattern, target in ((DEFINITION_RE, declarations),):
        for match in pattern.finditer(sanitized):
            route, _carries_fields = span_at(match.start())
            if route is None:
                continue
            name = match.group("name") or (
                match.group("macro") if "macro" in match.re.groupindex else None
            )
            if name and name != "_":
                target.setdefault(route, set()).add(normalized(name))
    return {
        route: (
            frozenset(declarations.get(route, ())),
            frozenset(members.get(route, ())),
        )
        for route in set(declarations) | set(members)
    }


def _within(spans: list[tuple[int, int]], position: int) -> bool:
    """Is ``position`` inside one of these sorted, disjoint spans?"""
    index = bisect.bisect_right([start for start, _end in spans], position) - 1
    return index >= 0 and spans[index][0] <= position < spans[index][1]


def _outermost_spans(source: str) -> list[tuple[int, int]]:
    """The stretches of a whole file that sit outside every brace."""
    spans: list[tuple[int, int]] = []
    depth = 0
    start = 0
    for index, char in enumerate(source):
        if char in "{([":
            if depth == 0:
                spans.append((start, index))
            depth += 1
        elif char in "})]":
            depth -= 1
            if depth == 0:
                start = index + 1
    if depth == 0:
        spans.append((start, len(source)))
    return spans


DEFINITION_RE = re.compile(
    r"^[ \t]*(?:(?:pub(?:\([^)\r\n]*\))?|unsafe|async|const|default|"
    r"extern(?:[ \t]+\"[^\"]+\")?)[ \t]+)*"
    r"(?:fn|struct|enum|union|trait|mod|const|static|type)[ \t]+"
    rf"(?:mut[ \t]+)?(?:r#)?(?P<name>{IDENT})(?!{IDENT_CONT})"
    rf"|^[ \t]*macro_rules![ \t]*(?P<macro>{IDENT})(?!{IDENT_CONT})",
    re.MULTILINE,
)

ENTRY_NAME = re.compile(
    r"\A(?:\s*\#\s*\[[^\]]*\])*\s*"
    r"(?:pub(?:\([^)]*\))?\s+)*"
    rf"(?:r#)?(?P<name>{IDENT})"
)


def entry_names(body: str) -> set[str]:
    """The names a `{ .. }` body declares as entries, in declaration order.

    ``body`` starts at the opening brace.  Splitting on depth-one commas is what
    makes a wrapped field one entry rather than two, and what keeps a nested
    `Beta { x: u32 }` variant's inner field out of the OUTER type's member set --
    that field belongs to `Beta`, which this index does not model, so it is left
    out rather than attributed to the wrong owner.
    """
    names: set[str] = set()
    depth = 0
    angle = 0
    segment_start = None
    for index, char in enumerate(body):
        if char in "{([":
            depth += 1
            if depth == 1:
                segment_start = index + 1
            continue
        if char in "})]":
            depth -= 1
            if depth == 0 and segment_start is not None:
                _take_entry(body[segment_start:index], names)
                segment_start = None
            continue
        if char == "<" and not (
            body[index : index + 2] == "<<"
            or (index > 0 and body[index - 1] == "<")
        ):
            angle += 1
            continue
        if char == ">" and index > 0 and body[index - 1] not in "-=":
            angle = max(0, angle - 1)
            continue
        if char == "," and depth == 1 and angle == 0 and segment_start is not None:
            _take_entry(body[segment_start:index], names)
            segment_start = index + 1
    return names


def _take_entry(segment: str, names: set[str]) -> None:
    match = ENTRY_NAME.match(segment)
    if match:
        names.add(normalized(match.group("name")))


SCOPE_HEADER = re.compile(
    r"^[ \t]*(?:(?:pub(?:\([^)\r\n]*\))?|unsafe|default)[ \t]+)*"
    r"(?:(?P<impl>impl)\b"
    r"|(?:(?P<kind>struct|enum|union|trait|mod)[ \t]+(?:r#)?"
    rf"(?P<name>{IDENT})(?!{IDENT_CONT})))",
    re.MULTILINE,
)


class Dependencies:
    """Every question one file's verification asked about the rest of the tree.

    Two kinds, because the tree answers two kinds:

    * ``candidates`` -- "which indexed files could this citation name?", whose
      answer depends on the SET of `.rs` paths.  A new file appearing, or an old
      one being deleted or moved, changes an answer here and nothing else would
      notice: no file's CONTENT changed when `lib.rs` number 72 was added, yet
      every bare citation of that basename just became ambiguous.
    * ``files`` -- "how long is this file / what does it declare?", whose answer
      depends on that file's CONTENT, recorded as the digest it had.

    Recorded by `Tree` itself rather than by its callers, so a new question
    cannot be added to the gate and forgotten here: the three content methods and
    the candidate lookup are the whole of the tree's surface, and each records
    itself.
    """

    def __init__(self) -> None:
        self.candidates: dict[tuple[str, str], list[str]] = {}
        self.files: dict[str, str] = {}


class Tree:
    """The cited side: every `.rs` file in the repository, read at most once."""

    def __init__(self, repo_root: Path, store: "FileStore | None" = None) -> None:
        self.repo_root = repo_root
        self.store = store if store is not None else FileStore(repo_root)
        self.recording: Dependencies | None = None
        self.by_basename: dict[str, list[str]] = {}
        self._lines: dict[str, int] = {}
        self._defined: dict[str, frozenset[str]] = {}
        self._scopes: dict[str, dict[tuple[str, ...], frozenset[str]]] = {}
        for dirpath, dirs, files in os.walk(repo_root, onerror=_walk_error):
            dirs[:] = [name for name in dirs if name not in SKIP_DIRS]
            for name in files:
                if not name.endswith(".rs"):
                    continue
                relative = (
                    os.path.relpath(os.path.join(dirpath, name), repo_root)
                    .replace("\\", "/")
                )
                self.by_basename.setdefault(name, []).append(relative)
        for paths in self.by_basename.values():
            paths.sort()

    def _text(self, relative: str) -> str:
        return self.store.text(relative)

    def _consulted(self, relative: str) -> None:
        """Note that this run's answer depended on ``relative``'s content."""
        if self.recording is not None and relative not in self.recording.files:
            self.recording.files[relative] = self.store.digest(relative)

    def line_count(self, relative: str) -> int:
        """PHYSICAL lines.

        `count("\\n") + 1` invents a line past the end of every
        newline-terminated file, which made a citation one line too far resolve,
        and reported 1 for an empty file.  (Codex review round 2, finding 3.)
        The text is not retained: `attach_log.rs` alone is 5 MB.
        """
        self._consulted(relative)
        if relative not in self._lines:
            self._lines[relative] = len(self._text(relative).splitlines())
        return self._lines[relative]

    def defined_names(self, relative: str) -> frozenset[str]:
        """Every name the file declares at an ADDRESSABLE place.

        Derived from the scope index rather than from a scan of raw text: a
        `fn` inside a function body, and Rust-looking text inside a comment or a
        string literal, declare nothing a citation can reach, and all three used
        to satisfy one.  (Codex review round 6, finding 3.)
        """
        self._consulted(relative)
        if relative not in self._defined:
            names: set[str] = set()
            for declared, _members in self.scope_members(relative).values():
                names |= declared
            self._defined[relative] = frozenset(names)
        return self._defined[relative]

    def scope_members(self, relative: str) -> dict[tuple[str, ...], frozenset[str]]:
        """route -> what that route declares.  Built once per file; see `index_scopes`."""
        self._consulted(relative)
        if relative not in self._scopes:
            self._scopes[relative] = index_scopes(self._text(relative))
        return self._scopes[relative]

    def candidates(self, path: str, base: str) -> list[str]:
        """Files a citation's `path`/`base` could name.

        A citation may be written from the repository root, from a crate root,
        or from the citing file's own directory, so a written path is matched as
        a SUFFIX of an indexed path.  A suffix that matches several indexed
        files is exactly as ambiguous as a bare basename and is returned as
        such.
        """
        indexed = self.by_basename.get(base, [])
        if not path:
            answer = list(indexed)
        else:
            wanted = (path + base).replace("\\", "/")
            answer = [
                relative
                for relative in indexed
                if relative == wanted or relative.endswith("/" + wanted)
            ]
        if self.recording is not None:
            self.recording.candidates[(path, base)] = answer
        return answer

    def defines(self, relative: str, item_path: str) -> bool:
        """Is ``item_path`` a real route through ``relative``?

        A single segment has only to be declared in the file.  A longer path has
        to be a ROUTE: some scope whose lexical route ends with the citation's
        leading segments must declare the last one.  Suffix-matching the route
        is what lets a citation write `Row::method` without naming the module
        while still rejecting `alpha::Row::beta_only`, where the member belongs
        to a different module's `Row`.

        Getting here took four rounds of review.  Accepting the last segment
        alone admitted `MissingType::existing`; accepting the first alone
        admitted every member of a known type; accepting both independently
        admitted a member of a *different* type and a local `let` in a function
        body.  A citation names a path, and a path that does not lead anywhere
        misdirects exactly like a drifted line number.
        """
        segments = [
            normalized(segment[2:] if segment.startswith("r#") else segment)
            for segment in item_path.split("::")
        ]
        if segments[0] not in self.defined_names(relative):
            return False
        if len(segments) == 1:
            return True
        prefix, last = tuple(segments[:-1]), segments[-1]
        for route, (declared, listed) in self.scope_members(relative).items():
            if len(route) < len(prefix):
                continue
            if route[len(route) - len(prefix) :] != prefix:
                continue
            if last in declared or last in listed:
                return True
        return False


_SPACED_PATH_TAIL = re.compile(rf"(?:{PATH_CHAR}|[ \t/\\])*\Z")
EXTENSION_WINDOW = 256


def cited_path(text: str, match: re.Match[str], tree: "Tree") -> tuple[str, str]:
    """The `(path, base)` a citation names, spaces in its components included.

    Longest first, so `a/my file.rs` is preferred to `my file.rs` to `file.rs`;
    the first spelling that names an indexed file wins.  Failing all of them, a
    backticked path the author wrote whole is returned whole -- so that it is
    reported as naming no file, rather than silently shortened to a tail that
    names a different one.
    """
    path, base = match.group("path"), match.group("base")
    if not text[: match.start()].endswith((" ", "\t")):
        return path, base
    window = text[max(0, match.start() - EXTENSION_WINDOW) : match.start()]
    run = _SPACED_PATH_TAIL.search(window).group(0)
    starts = [0] + [index + 1 for index, char in enumerate(run) if char in " \t"]
    for start in starts:
        if start < len(run) and run[start] in " \t":
            continue
        wider = _split_path(run[start:] + path + base)
        if wider == (path, base):
            continue
        if tree.candidates(*wider):
            return wider
    opening = len(window) - len(run) - 1
    if ("/" in run or "\\" in run) and opening >= 0 and window[opening] == "`":
        return _split_path(run + path + base)
    return path, base


def _split_path(written: str) -> tuple[str, str]:
    """A written path -> its directory prefix and its final component."""
    separator = max(written.rfind("/"), written.rfind("\\"))
    return written[: separator + 1], written[separator + 1 :]


def is_frozen(relative: str) -> bool:
    return any(part in FROZEN_DIRS for part in relative.split("/"))


def scan_files(repo_root: Path) -> list[str]:
    """Every citing-side file in the repository.

    One walk from the root, so a new tree is covered the day it is added rather
    than the day someone remembers to add it to a list.
    """
    found: list[str] = []
    for dirpath, dirs, files in os.walk(repo_root, onerror=_walk_error):
        dirs[:] = [
            name for name in dirs if name not in SKIP_DIRS and name not in FROZEN_DIRS
        ]
        for name in files:
            full = os.path.join(dirpath, name)
            relative = os.path.relpath(full, repo_root).replace("\\", "/")
            if relative in UNSCANNED:
                continue
            if not is_text_file(full, name):
                continue
            found.append(relative)
    return sorted(set(found))


def _walk_error(error: OSError) -> None:
    """`os.walk` swallows a directory it cannot list.  This refuses instead.

    A directory silently skipped is a set of citations silently unchecked, which
    is the same shape as the gate reporting a smaller count nobody compares
    against anything.
    """
    raise VerificationError(f"cannot list {error.filename}: {error}")


def read_bytes(path) -> bytes:
    """Read a file's bytes, or say which one could not be read.

    An unguarded read raised a bare `OSError` that `main` did not catch, so a
    permission problem crashed the gate with a traceback instead of naming the
    file. (Codex review round 9, finding 4.) A file the gate cannot read is a
    file whose citations it has not checked, so this is an error, never a skip.
    """
    try:
        with io.open(path, "rb") as handle:
            return handle.read()
    except OSError as error:
        raise VerificationError(f"cannot read {path}: {error}") from error


def decode(data: bytes) -> str:
    """A file's bytes as this gate reads them: UTF-8, replacing, universal newlines.

    THE ONE DECODER.  The bytes are needed anyway -- the cache keys a file's
    result on their digest -- so reading text a second way would be a second
    answer to "what does this file say", and the two would drift over a BOM, a
    lone CR, or an invalid sequence. `io.TextIOWrapper` over the bytes is not a
    re-implementation of `io.open`'s text mode: it is the same class `io.open`
    itself layers on the file, with the same defaults, so the two cannot decode
    differently. `test_verify_citation_integrity.py` holds them to that over
    CRLF, a BOM, a lone CR and invalid UTF-8.
    """
    return io.TextIOWrapper(io.BytesIO(data), encoding="utf-8", errors="replace").read()


def digest_bytes(data: bytes) -> str:
    """The content identity a cached result is keyed on."""
    return hashlib.blake2b(data, digest_size=32).hexdigest()


def read_text(path) -> str:
    """Read a file as text, or say which one could not be read."""
    return decode(read_bytes(path))


def _script_digest() -> "str | None":
    """This script's identity: the CODE THAT IS RUNNING, not a file on disk.

    Every cached result is filed under a key containing this, so that editing the
    gate re-verifies the tree rather than leaving anyone to judge whether their
    edit was cache-safe.

    READING THE FILE CANNOT ESTABLISH THIS, at any moment.  Python compiles the
    source once, at import; a read taken afterwards -- however soon afterwards --
    can return different bytes, and the run would then verify under the compiled
    rules while filing its results under the key of rules it never ran.  A later
    process running those newer rules would reuse them.  Moving the read earlier
    narrows that window without closing it. (Codex review rounds 1 and 2, finding
    1 both times.)

    So the digest is taken over the module's own CODE OBJECT, reached through the
    importing frame -- the object CPython compiled and is now executing, which no
    edit to the file can change underneath it.  `marshal` carries a code object's
    bytecode, its constants and every function and class body defined in it
    (each is a constant of the module's code), and it sorts frozenset constants
    precisely so that its output is reproducible between processes.

    WHAT THIS DELIBERATELY DOES NOT NOTICE: a change to this file that leaves the
    compiled code identical -- a `#` comment, blank lines, reformatting. Such an
    edit cannot change how any OTHER file is verified, because the rules are the
    code. It does change THIS file's own citing-side content, and that is caught
    the ordinary way: this file is in the scan set, so its entry is keyed on its
    bytes like everyone else's.

    The direction that has to hold is: a change to how this gate BEHAVES always
    moves the digest.  It does -- behaviour lives in the bytecode, the constants
    and the names, and all three are marshalled.  The converse is not claimed and
    does not need to be: an edit that moves code around without changing it moves
    the line table and re-verifies the tree for nothing, which costs one cold run.

    An interpreter that cannot supply its own frame, or a code object that will
    not marshal, returns ``None`` -- and a run without an identity for its own
    rules neither reads nor writes the cache, and says so.  It must not fall back
    to a per-process invention: that reads as an ordinary cold run every time,
    which is precisely how a cache silently stops working.
    """
    try:
        return code_digest(sys._getframe(1).f_code)
    except (AttributeError, ValueError, TypeError, RecursionError):
        return None


CODE_MARSHAL_VERSION = 2


def code_digest(code) -> str:
    """A digest of what a code object DOES, identical in every process."""
    return digest_bytes(marshal.dumps(_anonymous(code), CODE_MARSHAL_VERSION))


def _anonymous(code):
    """A code object with every filename in it removed, nested ones included.

    `co_filename` is marshalled with the rest, so without this the digest would
    be an identity for "this gate, at this path" rather than for this gate -- and
    the thing being claimed is that the RULES decide the key, not where the file
    happens to sit.  The nested walk is what makes it true of every function and
    class body as well as of the module.
    """
    constants = tuple(
        _anonymous(constant) if isinstance(constant, types.CodeType) else constant
        for constant in code.co_consts
    )
    return code.replace(co_filename="", co_consts=constants)


SCRIPT_DIGEST = _script_digest()


class FileStore:
    """Reads a file once, and remembers only its digest.

    The digest is what the cache compares; the TEXT is deliberately not
    retained, because the scan set is 98 MB and `attach_log.rs` alone is 6 MB --
    the pre-cache gate held one file's text at a time and this must not regress
    that. Digests are small and are kept for the whole run, so a file that is
    both cited and citing is read once and hashed once.
    """

    def __init__(self, repo_root: Path) -> None:
        self.repo_root = repo_root
        self._digests: dict[str, str] = {}

    def text(self, relative: str) -> str:
        """The file's text; its digest is memoized on the way past."""
        return decode(self._read(relative))

    def digest(self, relative: str) -> str:
        """The digest of the file's bytes, reading it if it has not been read.

        DECODING IS NOT DONE HERE.  A warm run wants the digest of every file in
        the scan set and the text of none of them, and decoding 98 MB it will not
        look at was a third of that run.
        """
        if relative not in self._digests:
            self._read(relative)
        return self._digests[relative]

    def digest_of_present(self, relative: str) -> str | None:
        """The digest, or ``None`` for a file that is no longer there.

        Used only when re-checking a cached dependency: a dependency that has
        been DELETED is a changed answer, not an error, and the file that
        depended on it is verified again -- which is where its disappearance is
        reported, if it matters.
        """
        if relative in self._digests:
            return self._digests[relative]
        if not (self.repo_root / relative).is_file():
            return None
        try:
            return self.digest(relative)
        except VerificationError:
            return None

    def _read(self, relative: str) -> bytes:
        """The file's bytes, digesting them on the way past.

        The bytes are not retained -- see the class docstring -- so a file whose
        TEXT is wanted after its digest has been taken is read a second time.
        That is one file in the scan set per stale cache entry, and keeping 98 MB
        alive to save it would trade a bounded read for an unbounded footprint.

        ONE FILE HAS ONE CONTENT FOR THE WHOLE RUN, and a second read that
        disagrees with the first is refused rather than absorbed.  Without this,
        a file edited WHILE the gate ran could be analysed at one version and
        recorded at another: `Tree` memoizes a file's declarations the first time
        a citation resolves against it, a later read as a CITING file would move
        the remembered digest to the new bytes, and a citation resolved after
        that would be recorded as depending on bytes it was never checked
        against -- an entry that then looks valid for ever. (Codex review round 4,
        finding 2.)  A tree that changes underneath a verification cannot be
        verified; the run says so and the next one is clean.
        """
        data = read_bytes(self.repo_root / relative)
        digest = digest_bytes(data)
        known = self._digests.get(relative)
        if known is not None and known != digest:
            raise VerificationError(
                f"{relative} changed while this run was reading the tree, so "
                f"what was checked and what would be recorded are two different "
                f"files. Nothing has been recorded; run the gate again."
            )
        self._digests[relative] = digest
        return data


def is_text_file(full_path: str, name: str) -> bool:
    """Is this a file whose bytes a human wrote comments into?

    Extension first because it is free, then a NUL sniff, which is what decides
    for anything the extension list has never heard of.  An unreadable file is
    reported rather than skipped: a citation the gate could not read is not a
    citation the gate has checked.
    """
    try:
        with open(full_path, "rb") as handle:
            head = handle.read(BINARY_SNIFF_BYTES)
    except OSError as error:
        raise VerificationError(f"cannot read {full_path}: {error}") from error
    return b"\x00" not in head


def read_allowance(repo_root: Path) -> tuple[bytes, dict]:
    """The allowance's BYTES and the object they parse to, from ONE read.

    THE TWO MUST COME FROM THE SAME READ.  The bytes are hashed into the cache
    key and the object decides what is grandfathered, so reading the file twice
    admits a run that verifies under one allowance and RECORDS the result under
    the key of another -- and unlike an ordinary read-while-changing race, that
    one persists: the next run parses the new allowance, computes the key the
    stale results were filed under, and reuses them. (Codex review round 1,
    finding 1.) One read, one truth, and a caller that cannot get them apart.
    """
    path = repo_root / ALLOWANCE
    if not path.is_file():
        raise VerificationError(
            f"the citation allowance {ALLOWANCE.as_posix()} is missing; it is "
            f"the record of which line citations are grandfathered, and without "
            f"it this gate cannot tell an old citation from a new one"
        )
    data = read_bytes(path)
    return data, _parse_allowance(path, data)


def load_allowance(repo_root: Path) -> dict:
    """The grandfathered set, or a hard failure.

    A MISSING allowance is an error, not an empty one.  An empty allowance would
    fail every citation in the tree, which someone would then "fix" by deleting
    the file -- and a deleted file that grandfathers everything is the shape of
    gate this repository has already been burned by.  Failing on absence makes
    the two states distinguishable.
    """
    return read_allowance(repo_root)[1]


def _parse_allowance(path: Path, data: bytes) -> dict:
    """Validate the allowance's shape, given the bytes it was read as."""
    try:
        loaded = json.loads(decode(data))
    except (ValueError, RecursionError) as error:
        raise VerificationError(f"cannot read {path}: {error}") from error
    if not isinstance(loaded, dict):
        raise VerificationError(f"{ALLOWANCE.as_posix()}: expected a JSON object")
    for section, value_kind in (
        ("line_references", int),
        ("unresolved", str),
        ("items", str),
    ):
        section_value = loaded.get(section, {})
        if not isinstance(section_value, dict):
            raise VerificationError(
                f"{ALLOWANCE.as_posix()}: \"{section}\" must be an object"
            )
        for citing, entries in section_value.items():
            if not isinstance(entries, dict) or not all(
                isinstance(value, value_kind) for value in entries.values()
            ):
                raise VerificationError(
                    f"{ALLOWANCE.as_posix()}: \"{section}\".\"{citing}\" must map "
                    f"each citation to a {value_kind.__name__}"
                )
            if value_kind is str:
                blank = sorted(
                    citation
                    for citation, value in entries.items()
                    if not value.strip()
                )
                if blank:
                    raise VerificationError(
                        f"{ALLOWANCE.as_posix()}: \"{section}\".\"{citing}\" "
                        f"exempts {blank} with an empty reason. An exemption "
                        f"without a reason is a permission nobody has read; "
                        f"write why the citation may stay broken, or repair it."
                    )
    external = loaded.get("external_sources", [])
    if not isinstance(external, list) or not all(
        isinstance(entry, str) for entry in external
    ):
        raise VerificationError(
            f"{ALLOWANCE.as_posix()}: \"external_sources\" must be a list of paths"
        )
    return loaded


def citation_end(text: str, match: re.Match[str]) -> int | None:
    """The last line a citation covers, or ``None`` when it names one line.

    Two spellings, both real in this tree: `file.rs:<a>-<b>` in one token, and
    the split ``file.rs:<a>`-`:<b>`` form the documents prefer.
    """
    if match.group("end"):
        return int(match.group("end"))
    split = SPLIT_RANGE.match(text, match.end())
    return int(split.group("end")) if split else None


def written_citation(text: str, match: re.Match[str], tree: "Tree") -> str:
    """A `LINE_CITATION` match, canonicalized with its endpoint if it has one.

    One function so the gate's budget key and the restamper's cannot disagree --
    they did once, over the range endpoint, and an allowance whose keys differ
    from the gate's is an allowance that grandfathers nothing it names.  The
    split-range endpoint is folded in here rather than left in the source text,
    so the two spellings of one range produce one key.

    The tree is a parameter for the same reason: the path a citation names is
    the one `cited_path` resolves, spaces included, and a key that spelt only the
    tail of a spaced filename would be a key for a citation nobody wrote.
    """
    path, base = cited_path(text, match, tree)
    written = f"{path}{base}:{int(match.group('line'))}"
    end = citation_end(text, match)
    if end is not None:
        written += f"-{end}"
    return written


def _external(external: list[str], path: str, base: str) -> bool:
    written = (path + base).replace("\\", "/")
    return any(written == entry or written.endswith("/" + entry) for entry in external)


def verify_one(
    citing: str,
    text: str,
    tree: "Tree",
    permitted: dict,
    unresolved_ok: dict,
    items_ok: dict,
    external: list[str],
) -> tuple[list[str], int, int, "Counter[str]"]:
    """Apply all three rules to ONE citing file.

    -> (the failures it produced, its line citations, its item citations, the
    allowance entries it consumed and how often).

    THE ONLY IMPLEMENTATION OF THE RULES.  A full run and an incremental run
    differ in which files reach this function, never in what happens when one
    does, so there is nothing here for a "fast path" to have got wrong.

    Deterministic in its arguments and in what `tree` answers, which is the whole
    premise of the cache: recording every answer `tree` gave is recording
    everything about the rest of the repository that this result depended on.
    Nothing here reads the clock, the environment, or any file directly -- the
    text arrives as an argument and the tree is the only oracle.
    """
    failures: list[str] = []
    used: "Counter[str]" = Counter()
    line_total = 0
    item_total = 0
    budget = Counter(permitted.get(citing, {}))
    item_spans: list[tuple[int, int]] = []

    for match in citation_matches(MALFORMED_ITEM, text):
        line = text.count("\n", 0, match.start()) + 1
        path, base = cited_path(text, match, tree)
        written = f"{path}{base}::{match.group('item')}"
        failures.append(
            f"{citing}:{line}: `{written}` is not a complete item citation -- "
            f"it ends on a `::` and names no item. Finish the path, or drop "
            f"the separator."
        )

    for match in citation_matches(ITEM_CITATION, text):
        item_spans.append((match.start(), match.end()))
        path, base = cited_path(text, match, tree)
        item_path = match.group("item")
        written = f"{path}{base}::{item_path}"
        if _external(external, path, base):
            continue
        item_total += 1
        reason = items_ok.get(citing, {}).get(written)
        candidates = tree.candidates(path, base)
        defining = [
            relative
            for relative in candidates
            if tree.defines(relative, item_path)
        ]
        if len(defining) == 1:
            if reason is not None:
                used[written] += 1
            continue
        if reason is not None:
            used[written] += 1
            continue
        line = text.count("\n", 0, match.start()) + 1
        if not candidates:
            failures.append(
                f"{citing}:{line}: `{written}` names no file in this tree"
            )
        elif not defining:
            failures.append(
                f"{citing}:{line}: `{written}` -- none of {candidates} "
                f"defines `{item_path}`; the item was renamed or deleted, so "
                f"re-point the citation at what it is called now"
            )
        else:
            failures.append(
                f"{citing}:{line}: `{written}` is ambiguous -- {defining} all "
                f"define `{item_path}`; qualify the citation with enough of "
                f"its path to name one"
            )

    item_span_starts = [start for start, _end in item_spans]
    for match in citation_matches(LINE_CITATION, text):
        at = bisect.bisect_right(item_span_starts, match.start()) - 1
        if at >= 0 and match.start() < item_spans[at][1]:
            continue
        path, base = cited_path(text, match, tree)
        cited_line = int(match.group("line"))
        cited_end = citation_end(text, match)
        written = written_citation(text, match, tree)
        if _external(external, path, base):
            continue
        line_total += 1
        line = text.count("\n", 0, match.start()) + 1

        if budget[written] <= 0:
            failures.append(
                f"{citing}:{line}: `{written}` is a new free-text line "
                f"number. Line numbers drift on the next insertion into the "
                f"cited file and nothing checks them; cite the item instead, "
                f"as `{base}::<item_name>`. (If this reference genuinely "
                f"cannot name an item, add it to {ALLOWANCE.name} under "
                f'"line_references" with the reason.)'
            )
            continue
        budget[written] -= 1
        used[written] += 1

        reason = unresolved_ok.get(citing, {}).get(written)
        candidates = tree.candidates(path, base)
        if len(candidates) == 1:
            total = tree.line_count(candidates[0])
            last = cited_end if cited_end is not None else cited_line
            if 1 <= cited_line <= last <= total:
                continue
            if reason is None and cited_end is not None and cited_end < cited_line:
                failures.append(
                    f"{citing}:{line}: `{written}` is a backwards range -- it "
                    f"ends before it starts, so it cites nothing."
                )
                continue
            if reason is None and cited_line < 1:
                failures.append(
                    f"{citing}:{line}: `{written}` cites line {cited_line}; "
                    f"files start at line 1."
                )
                continue
        if reason is not None:
            continue
        if not candidates:
            failures.append(
                f"{citing}:{line}: `{written}` names no file in this tree. "
                f"Find what it meant (`git log -S`), then re-point it at the "
                f"item -- or correct the sentence and delete it."
            )
        elif len(candidates) > 1:
            failures.append(
                f"{citing}:{line}: `{written}` is ambiguous -- {len(candidates)} "
                f"files carry that name ({candidates[:4]}...). Qualify it with "
                f"enough of its path to name one."
            )
        else:
            failures.append(
                f"{citing}:{line}: `{written}` is past the end of "
                f"{candidates[0]} ({tree.line_count(candidates[0])} lines). "
                f"The cited text moved or was deleted; re-point the citation "
                f"at the item it meant."
            )
    return failures, line_total, item_total, used



ENTRY_FIELDS = (
    "digest",
    "candidates",
    "files",
    "failures",
    "line_total",
    "item_total",
    "used",
)


def cache_key(repo_root: Path, allowance_bytes: bytes) -> str:
    """What every cached result depends on, beyond its own file and the tree.

    All three are hashed WHOLE rather than consulted for the part that matters:

    * this script, because a changed rule is a changed result and no reviewer
      should have to decide whether their edit was cache-safe;
    * the allowance, because it is an input to every file's verification --
      `external_sources` alone is consulted for every citation in the tree -- and
      because widening it while an old cache said PASS is exactly the shape of
      staleness that would let an exemption go unread;
    * the interpreter, because `IDENT` and `PATH_CHAR` are DERIVED at import from
      the running interpreter's XID tables.  A different Python can read the same
      bytes as a different name, so a cache written by one is not evidence for
      the other.

    THE ALLOWANCE ARRIVES AS BYTES, from the same read that produced the object
    the run is verifying against -- never re-read here.  See `read_allowance`.

    ``repo_root`` is unused by the digest and is a parameter only so that a
    caller cannot mistake this for a key that is portable between checkouts: it
    is not, because the allowance it hashes belongs to one.

    Never called without an identified script: `verify_repo` checks first and
    runs cache-free rather than inventing a key nothing can match.
    """
    if SCRIPT_DIGEST is None:
        raise VerificationError(
            "this gate cannot establish the identity of its own code, so there "
            "is no key under which a result could be recorded"
        )
    digest = hashlib.blake2b(digest_size=32)
    for part in (
        CACHE_SCHEMA,
        SCRIPT_DIGEST,
        digest_bytes(allowance_bytes),
        sys.version,
        unicodedata.unidata_version,
    ):
        digest.update(part.encode("utf-8"))
        digest.update(b"\x00")
    return digest.hexdigest()


def entry_tag(key: str, entry: dict) -> str:
    """A digest over an entry's own contents, bound to the cache key.

    This is what makes a HAND-EDITED entry a miss rather than a lie.  The
    dependency checks below compare the entry's record of the world against the
    world; they cannot, on their own, notice an entry whose recorded RESULT was
    altered -- deleting a cached failure would otherwise buy a green run.  The
    tag covers the result and the record together, so altering either without
    recomputing it is detected and the file is verified again.

    It is a digest, not a signature: someone who can write the cache can also
    recompute a tag, exactly as they could edit this script.  What it defends
    against is the reachable case -- an entry edited by hand or damaged in
    place -- and there it is decisive.
    """
    payload = json.dumps(
        [entry.get(field) for field in ENTRY_FIELDS],
        sort_keys=True,
        ensure_ascii=True,
        separators=(",", ":"),
    )
    return hashlib.blake2b(
        (key + "\x00" + payload).encode("utf-8"), digest_size=32
    ).hexdigest()


def well_formed(entry: object) -> bool:
    """Is this entry shaped the way the aggregation below will read it?

    A cache is a file on disk and may be truncated, half-written by a killed
    process, or edited.  Reading a result out of a mis-shaped entry would raise a
    traceback at best; at worst a wrong type would sum into a reported count.
    """
    if not isinstance(entry, dict):
        return False
    if not isinstance(entry.get("digest"), str):
        return False
    if not isinstance(entry.get("files"), dict) or not all(
        isinstance(name, str) and isinstance(value, str)
        for name, value in entry["files"].items()
    ):
        return False
    if not isinstance(entry.get("candidates"), list) or not all(
        isinstance(record, list)
        and len(record) == 3
        and isinstance(record[0], str)
        and isinstance(record[1], str)
        and isinstance(record[2], list)
        and all(isinstance(name, str) for name in record[2])
        for record in entry["candidates"]
    ):
        return False
    if not isinstance(entry.get("failures"), list) or not all(
        isinstance(message, str) for message in entry["failures"]
    ):
        return False
    for field in ("line_total", "item_total"):
        value = entry.get(field)
        if not isinstance(value, int) or isinstance(value, bool) or value < 0:
            return False
    if not isinstance(entry.get("used"), list) or not all(
        isinstance(record, list)
        and len(record) == 2
        and isinstance(record[0], str)
        and isinstance(record[1], int)
        and not isinstance(record[1], bool)
        for record in entry["used"]
    ):
        return False
    return True


def reusable(entry: object, relative: str, tree: "Tree", store: "FileStore", key: str) -> bool:
    """May this recorded result stand in for verifying ``relative`` again?

    Four questions, all of which must answer yes:

    1. is the entry shaped like an entry, and does its tag match its contents;
    2. does the citing file still hash to what it hashed to;
    3. does every recorded candidate lookup still return the recorded answer --
       which is what notices a file added, deleted or moved elsewhere in the
       tree, none of which changes any file's content;
    4. does every file whose content was consulted still hash the same.

    A file that has been deleted since -- the citing file itself, or one it
    depended on -- reads as a changed answer and re-verifies, which is where its
    absence gets reported if it matters.
    """
    if not well_formed(entry) or entry.get("tag") != entry_tag(key, entry):
        return False
    if entry["digest"] != store.digest_of_present(relative):
        return False
    for path, base, answer in entry["candidates"]:
        if tree.candidates(path, base) != answer:
            return False
    for name, digest in entry["files"].items():
        if store.digest_of_present(name) != digest:
            return False
    return True


def verify_and_record(
    citing: str,
    tree: "Tree",
    store: "FileStore",
    permitted: dict,
    unresolved_ok: dict,
    items_ok: dict,
    external: list[str],
    key: str,
) -> dict:
    """Verify one file for real, recording what its result depended on."""
    deps = Dependencies()
    tree.recording = deps
    try:
        text = store.text(citing)
        failures, line_total, item_total, used = verify_one(
            citing, text, tree, permitted, unresolved_ok, items_ok, external
        )
    finally:
        tree.recording = None
    entry = {
        "digest": store.digest(citing),
        "candidates": sorted(
            [path, base, answer] for (path, base), answer in deps.candidates.items()
        ),
        "files": dict(sorted(deps.files.items())),
        "failures": failures,
        "line_total": line_total,
        "item_total": item_total,
        "used": sorted([written, count] for written, count in used.items()),
    }
    entry["tag"] = entry_tag(key, entry)
    return entry


def cache_location(
    repo_root: Path, given: "Path | None", notices: "list[str] | None" = None
) -> "Path | None":
    """Where the cache lives, or ``None`` when there is nowhere usable.

    A cache inside the scan set is not a small problem: it records the citations
    every file carries and spells out any failure message, so scanning it would
    invent citations nobody wrote and fail the gate on them.  The default sits
    under `target/`, which `SKIP_DIRS` excludes; an override may live outside the
    repository (which is what the tests do), but inside it must be somewhere the
    scan does not reach -- and THAT is refused outright, because it is a
    misconfiguration that would corrupt what the gate reports.

    Being unable to REACH a location is the other case entirely, and returns
    ``None``: the run simply proceeds without a cache.
    """
    if notices is None:
        notices = []
    path = (repo_root / CACHE) if given is None else Path(given)
    try:
        resolved = path.resolve()
    except OSError as error:
        notices.append(
            f"citation integrity cache: {path} cannot be reached ({error}); "
            f"verifying every file and recording nothing"
        )
        return None
    for spelling in (Path(os.path.abspath(path)), resolved):
        inside = _within_repo(spelling, repo_root)
        if inside is None:
            continue
        written = "/".join(inside)
        if any(part in SKIP_DIRS for part in inside):
            continue
        raise VerificationError(
            f"the cache path {written} is inside the tree this gate scans. Its "
            f"contents are citations and failure messages, so scanning it would "
            f"fail the gate on citations nobody wrote. Put it under one of "
            f"{sorted(SKIP_DIRS)}, or outside the repository."
        )
    return resolved


def _within_repo(path: Path, repo_root: Path) -> "list[str] | None":
    """``path``'s components below ``repo_root``, or ``None`` if it is outside.

    BY FILE IDENTITY, not by comparing spellings.  Windows has several names for
    one directory -- an extended-length prefix, a `\\\\localhost\\C$` share, a
    junction, a different case -- and none of them can be made relative to the
    ordinary spelling by text, so a lexical check accepted every one of them and
    would have let the cache be written into the scanned tree, for the next run
    to read back as citations nobody wrote. (Codex review round 2, finding 2.)
    Walking up to a directory that IS the repository root, asked of the
    filesystem rather than of the string, has no such aliases to miss.

    The path itself need not exist -- the cache usually does not yet -- so only
    the directories that do exist are compared, and each component that is not
    the root is collected on the way up.
    """
    components: list[str] = []
    current = path
    for _ in range(len(path.parts) + 1):
        try:
            if current.is_dir() and os.path.samefile(current, repo_root):
                return components
        except OSError:
            pass
        parent = current.parent
        if parent == current:
            return None
        components.insert(0, current.name)
        current = parent
    return None


def load_cache(path: Path, key: str, notices: list[str]) -> dict:
    """The recorded entries, or none of them.

    EVERY WAY OF NOT UNDERSTANDING THIS FILE RETURNS NOTHING, which verifies the
    whole tree.  A cache that cannot be read is a slow run; a cache half-believed
    is an unchecked citation, so there is no partial acceptance here and no
    repair attempt.  Only a first run is silent about it -- an absent cache is
    the normal state of a fresh checkout, not a fault.
    """
    try:
        raw = json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError:
        return {}
    except (OSError, ValueError, UnicodeDecodeError, RecursionError) as error:
        notices.append(
            f"citation integrity cache: {path.name} could not be read ({error}); "
            f"verifying every file"
        )
        return {}
    if not isinstance(raw, dict) or raw.get("schema") != CACHE_SCHEMA:
        notices.append(
            f"citation integrity cache: {path.name} is not a v1 cache; "
            f"verifying every file"
        )
        return {}
    if raw.get("key") != key:
        notices.append(
            f"citation integrity cache: this script, the allowance or the "
            f"interpreter changed; verifying every file"
        )
        return {}
    entries = raw.get("entries")
    if not isinstance(entries, dict):
        notices.append(
            f"citation integrity cache: {path.name} carries no entries object; "
            f"verifying every file"
        )
        return {}
    return entries


def save_cache(path: Path, key: str, entries: dict, notices: list[str]) -> None:
    """Write the cache, atomically, and never fail the gate over it.

    Written through a temporary and `os.replace` because several checkouts of
    this repository run their batteries at once and a half-written cache read by
    the next run would be a miss at best.  A write that cannot happen at all --
    a read-only tree, a full disk -- is reported and ignored: it costs the NEXT
    run its speed and costs this one's correctness nothing.
    """
    document = {"schema": CACHE_SCHEMA, "key": key, "entries": entries}
    temporary = path.with_name(f"{path.name}.{os.getpid()}.tmp")
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        temporary.write_text(
            json.dumps(document, sort_keys=True, separators=(",", ":")) + "\n",
            encoding="utf-8",
            newline="\n",
        )
        os.replace(temporary, path)
    except OSError as error:
        notices.append(
            f"citation integrity cache: not written ({error}); the next run "
            f"verifies every file"
        )
        try:
            temporary.unlink()
        except OSError:
            pass


def verify_repo(
    repo_root: Path,
    *,
    use_cache: bool = True,
    cache_path: "Path | None" = None,
    notices: "list[str] | None" = None,
) -> tuple[int, int, int, int]:
    """-> (files scanned, line citations, item citations, unused allowance entries).

    ``use_cache=False`` is `--full`: every file is verified from its bytes, with
    no recorded result consulted.  The cache is still WRITTEN either way, so the
    full run a landing performs leaves it warm for the in-lane runs that follow.
    """
    repo_root = repo_root.resolve()
    if notices is None:
        notices = []
    store = FileStore(repo_root)
    tree = Tree(repo_root, store)
    allowance_bytes, allowance = read_allowance(repo_root)
    permitted: dict[str, dict[str, int]] = allowance.get("line_references", {})
    unresolved_ok: dict[str, dict[str, str]] = allowance.get("unresolved", {})
    items_ok: dict[str, dict[str, str]] = allowance.get("items", {})
    external: list[str] = allowance.get("external_sources", [])

    orphaned = sorted(
        f"{citing} :: {citation}"
        for citing, entries in unresolved_ok.items()
        for citation in entries
        if citation not in permitted.get(citing, {})
    )
    if orphaned:
        raise VerificationError(
            f"{ALLOWANCE.name}: these \"unresolved\" entries have no matching "
            f"\"line_references\" entry, so they permit nothing: {orphaned}"
        )

    location = cache_location(repo_root, cache_path, notices)
    identified = SCRIPT_DIGEST is not None and location is not None
    if SCRIPT_DIGEST is None:
        notices.append(
            "citation integrity: this gate could not establish the identity of "
            "its own code, so every file is verified and nothing is recorded"
        )
    key = cache_key(repo_root, allowance_bytes) if identified else ""
    recorded = load_cache(location, key, notices) if use_cache and identified else {}

    failures: list[str] = []
    used: Counter[tuple[str, str]] = Counter()
    line_total = 0
    item_total = 0
    reused = 0
    entries: dict[str, dict] = {}

    scanned = scan_files(repo_root)
    for citing in scanned:
        entry = recorded.get(citing)
        if entry is not None and reusable(entry, citing, tree, store, key):
            reused += 1
        else:
            entry = verify_and_record(
                citing,
                tree,
                store,
                permitted,
                unresolved_ok,
                items_ok,
                external,
                key,
            )
        entries[citing] = entry
        failures.extend(entry["failures"])
        line_total += entry["line_total"]
        item_total += entry["item_total"]
        for written, count in entry["used"]:
            used[(citing, written)] += count

    if identified:
        save_cache(location, key, entries, notices)
    notices.append(
        f"citation integrity: {reused} file(s) reused from cache, "
        f"{len(scanned) - reused} verified"
    )

    if failures:
        raise VerificationError(
            f"{len(failures)} unresolved citation(s):\n  " + "\n  ".join(failures)
        )

    unused = sum(
        1
        for citing, entries_for_file in permitted.items()
        for citation, count in entries_for_file.items()
        if used[(citing, citation)] < count
    )
    return len(scanned), line_total, item_total, unused


def _report(notices: list[str]) -> None:
    """Say what the run did, on stderr, where it cannot disturb the report."""
    for notice in notices:
        print(notice, file=sys.stderr)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Repo-wide citation integrity gate")
    parser.add_argument(
        "--repo-root", type=Path, default=Path(__file__).resolve().parents[3]
    )
    parser.add_argument(
        "--full",
        action="store_true",
        help=(
            "verify every file from its bytes, consulting no recorded result. "
            "The central validation runner passes this, so every landing gets "
            "one cache-free verification; the cache is still refreshed, leaving "
            "it warm for the in-lane runs that follow."
        ),
    )
    parser.add_argument(
        "--cache",
        type=Path,
        default=None,
        help=(
            f"where results are remembered (default: {CACHE.as_posix()}). Must "
            f"not be anywhere this gate scans."
        ),
    )
    args = parser.parse_args(argv)
    notices: list[str] = []
    for stream in (sys.stdout, sys.stderr):
        if hasattr(stream, "reconfigure"):
            try:
                stream.reconfigure(errors="backslashreplace")
            except (OSError, ValueError):
                pass
    try:
        files, lines, items, unused = verify_repo(
            args.repo_root,
            use_cache=not args.full,
            cache_path=args.cache,
            notices=notices,
        )
    except VerificationError as error:
        _report(notices)
        print(f"citation integrity check: FAIL: {error}", file=sys.stderr)
        return 1
    _report(notices)
    print(
        f"citation integrity check: PASS ({files} files scanned, "
        f"{lines} line citations grandfathered, {items} item citations resolved, "
        f"{unused} unused allowance entries)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
