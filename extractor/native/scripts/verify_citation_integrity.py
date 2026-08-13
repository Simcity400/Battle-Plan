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
"""

from __future__ import annotations

import argparse
import bisect
import importlib.util
import io
import json
import os
import re
import sys
import unicodedata
from collections import Counter
from pathlib import Path


# Citing side: THE WHOLE REPOSITORY, minus the directories below.
#
# An enumerated root list was tried first and is the wrong shape for this gate.
# It scanned six trees and therefore silently exempted everything else -- the
# repository-root documents (`ACCEPTANCE.md` alone carries an ambiguous
# `lib.rs:<line>`), `tools/`, `extractor/coverage_audit/`, the fixture trees --
# and nothing about the list said which files were outside it.  The world_fold
# gate had already been bitten by exactly this: `replay_interface/documents` was
# found citing `world_fold.rs` 28 times while only `extractor/documents` was
# being scanned.  Scanning everything and naming the exclusions makes the
# exemptions visible and finite.
# EVERY TEXT FILE, not a suffix whitelist.  A whitelist was tried and kept
# leaking: `.sh`, then `.json`, then `.txt`, then `.toml`, then `.yml` -- and a
# case-sensitive one still missed `.C` and left `seh_read.c`, `StringXref.java`
# and `f1_stress.hlsl` unscanned, all of which carry comments.  (Codex review
# rounds 6, 8 and 9.)  What the gate claims is whole-repository coverage, so the
# rule is: read it unless it is binary.
#
# BINARY_SUFFIXES is a speed filter, not the decision -- the decision is the NUL
# sniff below, which is what a file with an unlisted binary extension hits.
BINARY_SUFFIXES = frozenset(
    """
    .png .jpg .jpeg .gif .bmp .ico .tga .dds .webp .svgz
    .zip .gz .xz .zst .7z .rar .tar .cab .msi
    .exe .dll .pdb .lib .obj .so .dylib .a .o .bin .dat .pak
    .ttf .otf .woff .woff2 .wav .mp3 .ogg .mp4 .avi .mkv
    .pyc .pyd .class .jar .rlib .rmeta .db .sqlite .parquet
    """.split()
)
# How much of a file to sniff for a NUL byte before calling it text.
BINARY_SNIFF_BYTES = 8192

# The one file that must NOT be scanned: this gate's own allowance, whose KEYS
# are citations.  Scanning it would read every grandfathered citation back as a
# fresh citation of the allowance file, and the ratchet would demand the
# allowance grandfather itself.  Named by path, not by suffix, so `.json`
# documents (the dossiers carry real item citations) are still covered.
UNSCANNED = ("extractor/native/scripts/citation_integrity_allowance.json",)

ALLOWANCE = Path("extractor/native/scripts/citation_integrity_allowance.json")

# Directory names that never contain a file worth indexing or scanning.
# `target` and `.cargo-target` are build outputs: a basename that resolves into
# one resolves to a copy nobody edits, which would make an ambiguous citation
# look resolvable.
SKIP_DIRS = frozenset(
    {"target", ".git", ".claude", "node_modules", ".cargo-target", ".venv"}
)
# Append-only evidence.  Named as directory components, matched at any depth.
FROZEN_DIRS = frozenset({"runs", "re_evidence"})

# Rust identifiers follow Unicode XID rules and are compared after NFC
# normalization.  Python's `\w` excludes COMBINING MARKS, so a decomposed name
# (`cafe` + U+0301) was truncated to its ASCII head -- the citation
# `<file>::cafe\u0301` matched as `<file>::cafe` and resolved against a
# different function, and a decomposed FILENAME let the match restart inside the
# component.  (Codex review round 12, findings 3 and 4.)
#
# The first repair ENUMERATED six combining-mark blocks by hand, and a hand
# enumeration of a Unicode property is wrong the moment the property is wider
# than the hand that wrote it: U+0483 (COMBINING CYRILLIC TITLO) is a valid XID
# continuation character in none of those six blocks, so `<file>::a\u0483` was
# still truncated to `a` and still resolved against the wrong function.  (Codex
# review round 13, findings 1 and 2.)  The classes below are DERIVED, from the
# only XID tables that are certainly the same ones Rust-shaped names are compared
# with here -- Python's own, reached through `str.isidentifier`, whose rule is
# XID_Start (plus `_`) followed by XID_Continue.  The sweep costs ~0.2 s once per
# process and can never disagree with the interpreter running it.
#
# The derived set is a SUPERSET of XID_Continue, not an equality: `\w` is kept in
# the class even though it admits a few characters XID excludes (`\u00bd`, say).
# The directions are not symmetric.  A class that is too WIDE captures the whole
# of a malformed name, which then resolves against nothing and is REPORTED; a
# class that is too NARROW truncates a real name to a shorter one that resolves
# against something else, which is silent and is the defect itself.


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
# `[^\W\d]` is a NEGATED class and cannot be unioned with more members, so the
# start class is an alternation; the rest are ordinary positive classes.
IDENT_START = f"(?:[^\\W\\d]|[{XID_START_EXTRA}])"
IDENT_CONT = f"[\\w{XID_CONT_EXTRA}]"
IDENT = f"{IDENT_START}{IDENT_CONT}*"
PATH_CHAR = f"[\\w.\\-{XID_CONT_EXTRA}]"
# The left boundary shared by all three citation patterns: a match may not start
# in the middle of a path or of a name.  One definition, because three copies of
# it drifted from the identifier class they are supposed to mirror.
NOT_AFTER = f"(?<![\\w.\\-{XID_CONT_EXTRA}/\\\\])"


def normalized(name: str) -> str:
    """NFC, so a decomposed citation and a composed declaration compare equal."""
    return unicodedata.normalize("NFC", name)


# Matches `dir/file.rs:<line>`, `file.rs:<line>` and `file.rs:<line>-<line>`.
# The path prefix is captured separately: a qualified citation is resolved by
# PATH SUFFIX (which is exact where it matches one indexed file) and a bare one
# by BASENAME (the form that gives 71 candidates for `lib.rs`).
#
# The trailing `(?![0-9])` stops a four-digit line number being read as its
# first three digits; the leading look-behind stops a match starting in the
# middle of a path, so a two-directory citation is one citation of the full path
# and not also one of its tail.
LINE_CITATION = re.compile(
    rf"{NOT_AFTER}"
    rf"(?P<path>(?:{PATH_CHAR}+[/\\])*)"
    rf"(?P<base>{PATH_CHAR}+\.rs)"
    r":(?P<line>\d+)"
    r"(?:-(?P<end>\d+))?"
    r"(?![0-9])"
)
# THE SPLIT RANGE.  This repository writes most ranges with the endpoint in its
# own backticks -- ``file.rs:<start>`-`:<end>`` -- across 102 files, and an endpoint that
# is not part of the citation is an endpoint nothing keys a budget on and nothing
# range-checks.  (Codex review round 2, finding 1.)  Matched immediately after a
# `LINE_CITATION`, allowing the closing backtick, either dash the tree uses, and
# the reopening backtick.
# The gap may contain a line break and the next line's comment prefix: a range
# written at the end of a `///` line continues on the following one, and reading
# only horizontal whitespace lost the endpoint entirely.  (Codex review round 9,
# finding 2.)
SPLIT_RANGE = re.compile(
    r"`?[ \t]*(?:\r?\n[ \t]*(?://[/!]?|\#+|\*)?[ \t]*)?"
    r"[-–—]"
    r"[ \t]*(?:\r?\n[ \t]*(?://[/!]?|\#+|\*)?[ \t]*)?"
    r"`?:(?P<end>\d+)(?![0-9])"
)
# Matches `<file>::<item>` and `<file>::<Type>::<method>` (a real one names a
# `.rs` file; the placeholders here deliberately do not, because this file is
# inside the scan roots).  Matched FIRST and
# its spans are excluded from the line scan, so an item citation is never also
# read as a malformed line citation.  (Placeholders again: a worked example here
# would be an item citation this gate then had to resolve.)
# An item-shaped string that does NOT tokenize completely.  `<file>::okay::`
# has a trailing separator, so it names no item -- and before this it simply did
# not match, which meant the gate said nothing at all about it.  (Codex review
# round 6, finding 1.)  Silence about a malformed citation is the same failure as
# silence about a broken one.
MALFORMED_ITEM = re.compile(
    rf"{NOT_AFTER}"
    rf"(?P<path>(?:{PATH_CHAR}+[/\\])*)"
    rf"(?P<base>{PATH_CHAR}+\.rs)"
    # The trailing `(?:r#)?` catches an item path that stops on a raw-identifier
    # prefix (`...::r#`), which is as unfinished as one that stops on the `::`
    # and matched neither pattern before.  (Codex review round 7, finding 1.)
    rf"::(?P<item>(?:(?:r#)?{IDENT}::)*(?:r#)?)"
    # `<` excluded: `world_fold.rs::<item_name>` is a PLACEHOLDER in prose about
    # the citation form, not an unfinished citation, and the world_fold gate's
    # own files write several.
    rf"(?!{IDENT_CONT}|<)"
)
ITEM_CITATION = re.compile(
    rf"{NOT_AFTER}"
    rf"(?P<path>(?:{PATH_CHAR}+[/\\])*)"
    rf"(?P<base>{PATH_CHAR}+\.rs)"
    rf"::(?P<item>(?:r#)?{IDENT}(?:::(?:r#)?{IDENT})*)"
    # A RAW IDENTIFIER IS ONE IDENTIFIER.  Without the `r#` alternative and this
    # boundary, a citation of a raw identifier `r#<name>` parsed as just `r` and
    # resolved against a
    # function called `r`.  (Codex review round 4, finding 2.)
    # A partial match is a different citation.  The boundary rejects a further
    # identifier character (so a Unicode name is not truncated to its ASCII
    # head), a `#` (so `r#name` is one identifier), and a trailing `::` (so an
    # unfinished path is not read as the prefix that happens to resolve).
    rf"(?!{IDENT_CONT}|#|::)"
)


class VerificationError(RuntimeError):
    """A citation in this tree does not resolve, or is not grandfathered."""


# Comments and literals, in the order they must be recognized: a `//` inside a
# string is not a comment, and a `"` inside a comment is not a string.  Nested
# block comments are legal Rust and this pattern cannot express them, so a
# nested one is REFUSED below rather than mis-scanned.
_BLANKABLE = re.compile(
    r"(?P<raw>(?:b?r)(?P<hashes>\#*)\"(?s:.*?)\"(?P=hashes))"
    # `\<newline>` is a Rust string CONTINUATION, so the escape has to match a
    # newline; a non-DOTALL `\\.` ended the string early and every literal after
    # it in the file went unblanked.
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
        # ALL THREE DELIMITERS.  A brace-only walk left the inside of `(..)` and
        # `[..]` at the enclosing item depth, so item-shaped tokens handed to a
        # macro that discards them -- `discard!(fn ghost() {});` -- were indexed
        # as real items.  (Codex review round 9, finding 1.)
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

    # Every scope with a body, in source order: (header start, body start, body
    # end, owner).
    found: list[tuple[int, int, int, str]] = []
    for header in SCOPE_HEADER.finditer(sanitized):
        open_brace = sanitized.find("{", header.end())
        semicolon = sanitized.find(";", header.end())
        if open_brace < 0 or (0 <= semicolon < open_brace):
            continue  # `mod tests;` -- a declaration, not a body.
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

    # The file is the empty route; its "immediate depth" is brace depth zero.
    # Computed ONCE: recomputing it per top-level scope made the index O(scopes
    # x file size) and the gate three minutes long.
    file_level = _outermost_spans(sanitized)
    # (start, end, route, does this body declare FIELDS).  The last flag is
    # per BODY, not per route: `struct Row { .. }` and `impl Row { .. }` share
    # the route `("Row",)`, and only the first of them declares fields.
    spans: list[tuple[int, int, tuple[str, ...], bool]] = [
        (span_start, span_end, (), False) for span_start, span_end in file_level
    ]
    # DECLARATIONS and MEMBERS are kept apart.  Flattened together, a struct
    # field satisfied an ownerless citation of its bare name -- `<file>::field`
    # resolved because `Row` happened to have one.  (Codex review round 7,
    # finding 2.)  A member is reachable only as the LAST segment of its owner's
    # route; a declaration is reachable on its own.
    declarations: dict[tuple[str, ...], set[str]] = {(): set()}
    members: dict[tuple[str, ...], set[str]] = {(): set()}

    # One outward-to-inward sweep with a stack, so a scope's route costs nothing
    # to find.  A scope whose header does not sit at its parent's item level is
    # inside a function body: it gets no route, and neither does anything under
    # it, because it never goes on the stack.
    stack: list[tuple[int, tuple[str, ...] | None, list[tuple[int, int]]]] = []
    field_bodies: list[tuple[tuple[str, ...], str]] = []
    # Only a body that DECLARES FIELDS may contribute `MEMBER_RE` names.  An
    # `impl` body's `Other { .. }` struct-literal expression is a type being
    # constructed, not an associated item, and admitting it made `Row::Other`
    # resolve.  (Codex review round 10, finding 2.)  `impl`/`trait`/`mod` bodies
    # contribute through `DEFINITION_RE`, which is what actually declares there.
    for header_start, body_start, body_end, owner, kind in found:
        while stack and header_start > stack[-1][0]:
            stack.pop()
        parent_route: tuple[str, ...] | None
        if stack:
            parent_end, parent_route, parent_level = stack[-1]
        else:
            parent_route, parent_level = (), file_level
        # Bisect on the span STARTS, not on the span tuples: a probe tuple
        # `(position, position)` sorts before a span that begins at the same
        # position, so a scope declared at offset zero read as unaddressable and
        # the whole file's first item vanished from the index.
        addressable = _within(parent_level, header_start)
        if parent_route is None or not addressable:
            # Not addressable from outside: record it as a barrier so its own
            # children are excluded too, and contribute nothing.
            stack.append((body_end, None, []))
            continue
        owner = normalized(owner)
        route = parent_route + (owner,)
        # An `impl Row` block declares the route `Row` even when the `struct`
        # itself lives in another file, so a route owner counts as declared --
        # without which removing `impl` from DEFINITION_RE would make
        # `<file>::ForeignType::method` unresolvable.
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

    # ONLY A FIELD-CARRYING BODY CONTRIBUTES MEMBERS, and it contributes its
    # parsed ENTRIES rather than anything line-shaped inside it.  An `impl`
    # body's `Other { .. }` struct-literal expression is a type being
    # constructed, not an associated item (round 10), and a wrapped field's type
    # on its own line is not an entry (round 11).  `impl`/`trait`/`mod` bodies
    # contribute through `DEFINITION_RE`, which is what declares anything there.
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
            # `const _: u32 = 0;` is an anonymous item: `_` is the ABSENCE of a
            # name, not a name, so a citation of it names nothing.  (Codex
            # review round 8, finding 3.)
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
        # All three delimiters, for the reason given in `_immediate_depth_spans`.
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


# Every item DEFINITION in one line of Rust, name captured.
#
# Deliberately lexical and deliberately generous about the keyword set: this
# decides which of several same-named files an item citation means, not whether
# the item is spelt correctly.  Requiring the definition keyword is what makes it
# discriminating -- `stream_fold.rs` and `bin/stream_fold.rs` both MENTION
# `fold_compact_stream_file`, and only one defines it.
#
#
# THE DECLARATION SIDE USES THE SAME IDENTIFIER CLASS AS THE CITATION SIDE.  It
# had its own, `[^\W\d]\w*`, and a class that truncates a DECLARATION is the
# citation defect running backwards: `pub fn café() {}` was indexed as `cafe`,
# so a citation of the shorter, nonexistent `cafe` resolved against it.  (Codex
# review round 13, finding 2.)  `\b` is likewise `\w`-based and cannot end a name
# that ends in a combining mark, so the terminator is the identifier class too.
DEFINITION_RE = re.compile(
    r"^[ \t]*(?:(?:pub(?:\([^)\r\n]*\))?|unsafe|async|const|default|"
    r"extern(?:[ \t]+\"[^\"]+\")?)[ \t]+)*"
    # `impl` is NOT here.  `impl std::fmt::Display for Row` would declare `std`,
    # and a citation of `<file>::std` then resolved against nothing.  (Codex
    # review round 7, finding 3.)  An `impl` block still contributes its SUBJECT
    # through `SCOPE_HEADER`/`_impl_subject`, which reads `Row` from that header.
    r"(?:fn|struct|enum|union|trait|mod|const|static|type)[ \t]+"
    rf"(?:mut[ \t]+)?(?:r#)?(?P<name>{IDENT})(?!{IDENT_CONT})"
    rf"|^[ \t]*macro_rules![ \t]*(?P<macro>{IDENT})(?!{IDENT_CONT})",
    re.MULTILINE,
)

# A MEMBER: an enum variant or a struct field.  Neither carries a declaration
# keyword, so `DEFINITION_RE` cannot see them, and `Row::VERSION`-shaped
# citations are legitimate and common.  The shapes matched are `Variant,`,
# `Variant = 3,`, `Variant(u32)`, `Variant {`, and `pub field: Type`.
#
# A struct/enum/union body's ENTRIES.  Not a line pattern: a line pattern read a
# wrapped field's type as an entry of its own -- `pub actual:` / `    Other,`
# made `Row::Other` resolve.  (Codex review round 11, finding 1.)  Entries are
# separated by commas at the body's own depth, and each one's name is its
# leading identifier, after any attributes and visibility.
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
        # GENERIC ARGUMENTS ARE NOT ENTRIES.  `pub actual: Pair<Other, Ghost>,`
        # is one field, and splitting on its inner comma invented a member
        # called `Ghost`.  (Codex review round 12, finding 2.)  `<<` is a shift
        # and `->`/`=>` end in `>` without opening anything, so neither moves
        # the angle depth -- the same three exceptions the sibling gate's
        # scanner makes.
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


# An item scope: the `struct`/`enum`/`union`/`trait`/`mod` that owns a body, or
# an `impl` block whose subject owns one.  Used to answer "does `B` belong to
# `A`", which a file-wide name set cannot.
SCOPE_HEADER = re.compile(
    r"^[ \t]*(?:(?:pub(?:\([^)\r\n]*\))?|unsafe|default)[ \t]+)*"
    r"(?:(?P<impl>impl)\b"
    r"|(?:(?P<kind>struct|enum|union|trait|mod)[ \t]+(?:r#)?"
    rf"(?P<name>{IDENT})(?!{IDENT_CONT})))",
    re.MULTILINE,
)


class Tree:
    """The cited side: every `.rs` file in the repository, read at most once."""

    def __init__(self, repo_root: Path) -> None:
        self.repo_root = repo_root
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
        return read_text(self.repo_root / relative)

    def line_count(self, relative: str) -> int:
        """PHYSICAL lines.

        `count("\\n") + 1` invents a line past the end of every
        newline-terminated file, which made a citation one line too far resolve,
        and reported 1 for an empty file.  (Codex review round 2, finding 3.)
        The text is not retained: `attach_log.rs` alone is 5 MB.
        """
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
        if relative not in self._defined:
            names: set[str] = set()
            for declared, _members in self.scope_members(relative).values():
                names |= declared
            self._defined[relative] = frozenset(names)
        return self._defined[relative]

    def scope_members(self, relative: str) -> dict[tuple[str, ...], frozenset[str]]:
        """route -> what that route declares.  Built once per file; see `index_scopes`."""
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
            return list(indexed)
        wanted = (path + base).replace("\\", "/")
        return [
            relative
            for relative in indexed
            if relative == wanted or relative.endswith("/" + wanted)
        ]

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
        # `r#type` and `type` name the same item; the index stores the bare
        # name, so the citation is normalized to match it.
        # `r#type` and `type` name the same item, and a DECOMPOSED citation names
        # the same item as a composed declaration, so both are folded before
        # lookup.  (Codex review round 12, finding 3.)
        segments = [
            normalized(segment[2:] if segment.startswith("r#") else segment)
            for segment in item_path.split("::")
        ]
        if segments[0] not in self.defined_names(relative):
            return False
        if len(segments) == 1:
            return True
        # Built only now: a single-segment citation needs no route, and scope
        # indexing sanitizes the whole file, which is expensive on the 5 MB one.
        prefix, last = tuple(segments[:-1]), segments[-1]
        for route, (declared, listed) in self.scope_members(relative).items():
            if len(route) < len(prefix):
                continue
            if route[len(route) - len(prefix) :] != prefix:
                continue
            # A MEMBER is only reachable as the last segment: a struct field
            # owns nothing, so it can never be an intermediate step, and the
            # route walk above has already required every earlier segment to be
            # a declared scope.
            if last in declared or last in listed:
                return True
        return False


# A FILENAME MAY CONTAIN A SPACE; `PATH_CHAR` MAY NOT.  A path class that
# admitted spaces would swallow the prose on both sides of every citation --
# `see <file>::<item_name>` would be read as naming a file called `see <file>`
# -- so a citation of `dir/my <file>` matched only its TAIL, `<file>`, and that
# tail then resolved happily against an unrelated `file.rs` somewhere else in
# the tree.  (Codex review round 13, finding 3.)
#
# The match is therefore extended leftwards over spaces, but only as far as an
# extension that names a file THIS TREE ACTUALLY INDEXES.  That is what makes the
# extension safe to do unconditionally: where no such file exists -- which is the
# whole repository today, 0 `.rs` files with a space in the name -- every
# candidate fails the lookup and the original match stands unchanged, so no
# allowance key and no resolution moves.  Where such a file does exist, the
# citation is read as the whole component the author wrote.
#
# AND WHEN THE SPACED FILE DOES NOT EXIST AT ALL?  Resolving by existence alone
# left the case the gate is for: a citation of `dir/missing <file>` -- a typo, or
# a file since deleted -- extended to nothing, fell back to its tail, and the
# tail resolved against an unrelated file of that name.  A citation that names a
# file this tree does not have must say so.  (Codex review round 14, finding 1.)
#
# The discriminator is what the author DELIMITED and what they WROTE.  Inside a
# backtick span -- how this repository writes a citation -- a run that reaches
# the opening backtick and carries a path SEPARATOR is a path the author wrote
# whole, so the whole of it is the citation, and it is reported when it names
# nothing.  A run with no separator in it is prose (`see <file>`), and a
# separator-less prefix drops no path, so the narrow match stands.  Outside
# backticks nothing is assumed: prose runs on, and a run that reaches back
# through an earlier sentence would make an ordinary citation fail.
_SPACED_PATH_TAIL = re.compile(rf"(?:{PATH_CHAR}|[ \t/\\])*\Z")
# How far back to look.  A path component is short and a citation sits in prose;
# scanning the whole preceding text of a 5 MB file to find one would cost more
# than the defect.
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
        # The pattern's own left boundary already refused a path character here,
        # so with no space there is nothing a longer spelling could reach.
        return path, base
    window = text[max(0, match.start() - EXTENSION_WINDOW) : match.start()]
    run = _SPACED_PATH_TAIL.search(window).group(0)
    starts = [0] + [index + 1 for index, char in enumerate(run) if char in " \t"]
    for start in starts:
        if start < len(run) and run[start] in " \t":
            continue  # a start inside a run of spaces is the next start
        wider = _split_path(run[start:] + path + base)
        if wider == (path, base):
            continue
        if tree.candidates(*wider):
            return wider
    # No spelling names a file.  Inside a code span, a run that reaches the
    # opening backtick and contains a separator is a path written whole.
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


def read_text(path) -> str:
    """Read a file, or say which one could not be read.

    An unguarded read raised a bare `OSError` that `main` did not catch, so a
    permission problem crashed the gate with a traceback instead of naming the
    file. (Codex review round 9, finding 4.) A file the gate cannot read is a
    file whose citations it has not checked, so this is an error, never a skip.
    """
    try:
        with io.open(path, encoding="utf-8", errors="replace") as handle:
            return handle.read()
    except OSError as error:
        raise VerificationError(f"cannot read {path}: {error}") from error


def is_text_file(full_path: str, name: str) -> bool:
    """Is this a file whose bytes a human wrote comments into?

    Extension first because it is free, then a NUL sniff, which is what decides
    for anything the extension list has never heard of.  An unreadable file is
    reported rather than skipped: a citation the gate could not read is not a
    citation the gate has checked.
    """
    # THE BYTES DECIDE, ALWAYS.  An earlier version returned False on a
    # known-binary SUFFIX without reading, which contradicted this function's own
    # contract: a UTF-8 file named `probe.png` was skipped unread and a citation
    # inside it was never checked.  (Codex review round 10, finding 1.)  A sniff
    # is one 8 KB read, so there is nothing to buy by guessing.
    try:
        with open(full_path, "rb") as handle:
            head = handle.read(BINARY_SNIFF_BYTES)
    except OSError as error:
        raise VerificationError(f"cannot read {full_path}: {error}") from error
    return b"\x00" not in head


def load_allowance(repo_root: Path) -> dict:
    """The grandfathered set, or a hard failure.

    A MISSING allowance is an error, not an empty one.  An empty allowance would
    fail every citation in the tree, which someone would then "fix" by deleting
    the file -- and a deleted file that grandfathers everything is the shape of
    gate this repository has already been burned by.  Failing on absence makes
    the two states distinguishable.
    """
    path = repo_root / ALLOWANCE
    if not path.is_file():
        raise VerificationError(
            f"the citation allowance {ALLOWANCE.as_posix()} is missing; it is "
            f"the record of which line citations are grandfathered, and without "
            f"it this gate cannot tell an old citation from a new one"
        )
    try:
        loaded = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
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
            # A REASON MUST SAY SOMETHING.  `""` satisfied `isinstance(_, str)`
            # and bought the same exemption as a written justification, which is
            # the standing-permission-nobody-read shape these sections exist to
            # prevent.  (Codex review round 2, finding 5.)
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


def verify_repo(repo_root: Path) -> tuple[int, int, int, int]:
    """-> (files scanned, line citations, item citations, unused allowance entries)."""
    repo_root = repo_root.resolve()
    tree = Tree(repo_root)
    allowance = load_allowance(repo_root)
    permitted: dict[str, dict[str, int]] = allowance.get("line_references", {})
    unresolved_ok: dict[str, dict[str, str]] = allowance.get("unresolved", {})
    items_ok: dict[str, dict[str, str]] = allowance.get("items", {})
    external: list[str] = allowance.get("external_sources", [])

    # An `unresolved` entry that is not also a `line_references` entry
    # grandfathers a citation this gate would reject at rule L anyway, so it is
    # a record of a permission that does nothing -- the kind of entry that
    # accumulates and then gets cited as precedent.
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

    failures: list[str] = []
    used: Counter[tuple[str, str]] = Counter()
    line_total = 0
    item_total = 0

    scanned = scan_files(repo_root)
    for citing in scanned:
        text = read_text(repo_root / citing)
        budget = Counter(permitted.get(citing, {}))
        item_spans: list[tuple[int, int]] = []

        # A malformed item citation is REPORTED, not ignored.  `<file>::okay::`
        # names no item; before this it simply did not match any pattern, so the
        # gate said nothing about it at all, which is the same silence as a
        # citation that resolves wrongly.  (Codex review round 6, finding 1.)
        for match in MALFORMED_ITEM.finditer(text):
            line = text.count("\n", 0, match.start()) + 1
            path, base = cited_path(text, match, tree)
            written = f"{path}{base}::{match.group('item')}"
            failures.append(
                f"{citing}:{line}: `{written}` is not a complete item citation -- "
                f"it ends on a `::` and names no item. Finish the path, or drop "
                f"the separator."
            )

        for match in ITEM_CITATION.finditer(text):
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
                    used[(citing, written)] += 1
                continue
            if reason is not None:
                used[(citing, written)] += 1
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

        for match in LINE_CITATION.finditer(text):
            if any(start <= match.start() < end for start, end in item_spans):
                continue
            path, base = cited_path(text, match, tree)
            cited_line = int(match.group("line"))
            # THE RANGE ENDPOINT IS PART OF THE CITATION.  Dropping it made
            # a `file.rs:<start>-<end>` citation key the budget on its START
            # alone and range-check that alone: the endpoint could be moved to
            # any number, existing or not,
            # without changing a budget key or being checked against the file.
            # (Codex review 019fdd05-0dd0-7833-8b8b-cc0bfa4e3aa2, finding 3.)
            cited_end = citation_end(text, match)
            written = written_citation(text, match, tree)
            if _external(external, path, base):
                continue
            # Counted AFTER the external skip, so the reported total is the
            # number of citations the allowance actually has to carry and a
            # reader can compare it against the file.
            line_total += 1
            line = text.count("\n", 0, match.start()) + 1

            # Rule L first: whether the citation is ALLOWED TO EXIST is
            # independent of whether it currently resolves, and reporting the
            # resolution failure of a citation that should not have been written
            # at all buries the instruction that matters.
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
            used[(citing, written)] += 1

            reason = unresolved_ok.get(citing, {}).get(written)
            candidates = tree.candidates(path, base)
            if len(candidates) == 1:
                total = tree.line_count(candidates[0])
                last = cited_end if cited_end is not None else cited_line
                # `1 <= start <= end <= total`.  Line zero does not exist and an
                # end before its start denotes nothing; both read as resolvable
                # while only the start was bounds-checked.
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

    if failures:
        raise VerificationError(
            f"{len(failures)} unresolved citation(s):\n  " + "\n  ".join(failures)
        )

    unused = sum(
        1
        for citing, entries in permitted.items()
        for citation, count in entries.items()
        if used[(citing, citation)] < count
    )
    return len(scanned), line_total, item_total, unused


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Repo-wide citation integrity gate")
    parser.add_argument(
        "--repo-root", type=Path, default=Path(__file__).resolve().parents[3]
    )
    args = parser.parse_args(argv)
    # A REPORT MUST SURVIVE THE CONSOLE IT IS PRINTED TO.  Now that a name is
    # captured whole, a failure message can carry any character a Rust identifier
    # or a filename may carry -- and on the Windows console this gate runs on,
    # `sys.stdout` is cp1252, where printing one raises `UnicodeEncodeError`.
    # That turns "citation X does not resolve" into a traceback with no citation
    # in it, which is the reporting failure this gate exists to prevent, one
    # level up.  Unrepresentable characters are escaped instead.
    for stream in (sys.stdout, sys.stderr):
        if hasattr(stream, "reconfigure"):
            try:
                stream.reconfigure(errors="backslashreplace")
            except (OSError, ValueError):  # a stream that cannot be reconfigured
                pass
    try:
        files, lines, items, unused = verify_repo(args.repo_root)
    except VerificationError as error:
        print(f"citation integrity check: FAIL: {error}", file=sys.stderr)
        return 1
    print(
        f"citation integrity check: PASS ({files} files scanned, "
        f"{lines} line citations grandfathered, {items} item citations resolved, "
        f"{unused} unused allowance entries)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
