#!/usr/bin/env python3
"""Fail closed when wire-format payload-authority citations drift.

The dNN wire modules transcribe the shipping fold rather than calling it.  Their
top-level authority marker therefore pins four things to the source it cites:

* the domain-to-fold dispatch in ``WorldFold::fold_domain_data``;
* the exact lexical function envelope;
* a SHA-256 of the cited source bytes after newline normalization; and
* a SHA-256 of the cited functions' CALLEE CLOSURE -- themselves plus every
  item in ``world_fold.rs`` they can reach, functions AND the constants,
  statics, enums, structs and type aliases they read.  ``DIVISION_FLAG_SUPPLY_
  GAIN`` is a ``const``; re-spelling it ``1 << 9`` changes which bit the
  division fold reads while every ``fn`` stays byte-identical, so a
  function-only digest covered nothing that mattered about it.

THE CLOSURE IS WHAT SURVIVES A REFACTOR.  An envelope digest covers a function
where it stands and nothing else, so lifting a helper out of a cited fold narrows
the authority to the residual: the moved logic leaves every digest, the marker is
re-stamped over the smaller span, and the gate keeps printing PASS while the
rendering it claims to pin can be changed freely.  That is not hypothetical --
d20's document build was moved into a row method and the gate noticed nothing.
The closure covers the moved code wherever it lands in this file, and
``world_fold_authority_closure.json`` records WHICH functions each domain covers
so that a set that shrinks is a diff somebody has to justify rather than a digest
that silently means less.

The closure is computed from call sites, exactly where the call names its owner
(``self.f()``, ``Type::f()``, a bare ``f()``).  A method call on any other
receiver names no type: if exactly one local function carries that name it is
taken, and if several do the gate REFUSES rather than guess, and the calling
function declares the receiver with ``// @fold-callee Type::method`` (or
``// @fold-callee-external name`` when the receiver is not defined here at all).
Refusing is the load-bearing part -- it is why there is no call from a cited fold
to a local function that the closure can quietly omit.

THE SINGLE-CANDIDATE FALLBACK OVER-COVERS ON PURPOSE.  ``country.insert(..)`` on a
`serde_json` map resolves to ``MapJournal::insert`` because that is the only
``insert`` this file defines, so that helper sits in all 45 closures and changing
it re-stamps all 45.  The alternative -- resolve only what the call site spells --
would miss the single most likely shape of the refactor this gate exists to
survive: a helper lifted into a uniquely named method on a row struct, reachable
only as ``row.helper()``.  A redundant member costs a re-stamp; a missing one
costs the coverage.

WHAT IS STILL OUTSIDE.  A helper moved to ANOTHER FILE (the membership record
names it, the re-stamper refuses it, but the closure cannot follow it); trait
dispatch through ``Display``/``Debug``/operators, which names no method at the
call site; and a callee reached through a function pointer or ``dyn`` value
assigned in a function no domain covers.  Where a hole could be turned into a
refusal it has been: a ``macro_rules!`` definition, an unrecognized item scope,
and an unreadable path qualifier all stop the gate rather than cover less.

That marker owns exactly one line range.  Every OTHER line number in these
modules is free text: it drifts on the next insertion into ``world_fold.rs`` and
nothing notices.  Those have been rewritten to name the item they point at, and
the ones that could not be are ratcheted -- see ``verify_prose_references``.

The DESIGN DOCUMENTS cite ``world_fold.rs`` by line number too, and until this
was added nothing checked them at all -- ``DOMAIN_SCHEMA_RECIPE.md``, the
document every domain lane reads before writing a schema, carries eighteen bare
line numbers on its own.  A drifted line number in prose is worse than one in a
module: nothing compiles it, nobody reviews it against the source, and a lane
that follows it lands on whatever item moved into that position.  Documents are
therefore ratcheted exactly like the modules -- see
``verify_document_references``.

The two ports which also transcribe a capture-lane payload decoder (d51 and d52)
carry a second marker.  That marker hashes the original decoder, not the
transcription, so a changed production authority cannot be approved by tests of
the stale copy.
"""

from __future__ import annotations

import argparse
import bisect
import hashlib
import json
import re
import sys
from collections import Counter
from dataclasses import dataclass
from functools import lru_cache
from pathlib import Path
from typing import Iterable


WORLD_FOLD = Path("extractor/native/recorder_host/src/world_fold.rs")
WIRE_SRC = Path("extractor/native/wire_format/src")
# Both design-document trees.  `replay_interface/documents` was found citing
# `world_fold.rs` by line 28 times while only `extractor/documents` was being
# scanned, which is exactly the hole a one-directory scan leaves.
DOCUMENT_ROOTS = (Path("extractor/documents"), Path("replay_interface/documents"))
PROSE_ALLOWANCE = Path(
    "extractor/native/scripts/world_fold_prose_reference_allowance.json"
)
# The membership half of the authority.  The marker's `closure=` digest proves
# the covered SOURCE did not change; this file proves the covered SET did not
# quietly shrink -- a digest alone cannot, because the re-stamper recomputes it
# from whatever the closure happens to be after the edit.
CLOSURE_INVENTORY = Path(
    "extractor/native/scripts/world_fold_authority_closure.json"
)
PROSE_REFERENCE_RE = re.compile(r"world_fold\.rs:(\d+)")

WORLD_MARKER_RE = re.compile(
    r"^// @world-fold-authority "
    r"domain=(?P<domain>\d+) "
    r"functions=(?P<functions>fold_[A-Za-z0-9_]+(?:,fold_[A-Za-z0-9_]+)*) "
    r"lines=(?P<start>\d+)-(?P<end>\d+) "
    r"sha256=(?P<sha256>[0-9a-f]{64}) "
    r"closure=(?P<closure>[0-9a-f]{64})$",
    re.MULTILINE,
)
LANE_MARKER_RE = re.compile(
    r"^// @lane-payload-authority "
    r"domain=(?P<domain>\d+) "
    r"path=(?P<path>[A-Za-z0-9_./-]+) "
    r"function=(?P<function>[A-Za-z_][A-Za-z0-9_]*) "
    r"lines=(?P<start>\d+)-(?P<end>\d+) "
    r"sha256=(?P<sha256>[0-9a-f]{64})$",
    re.MULTILINE,
)
FUNCTION_RE = re.compile(
    r"(?m)^[ \t]*(?:(?:pub(?:\([^)\r\n]*\))?|unsafe|async|const|extern"
    r"(?:[ \t]+\"[^\"]+\")?)[ \t]+)*fn[ \t]+"
    r"(?P<name>[A-Za-z_][A-Za-z0-9_]*)\b"
)
DISPATCH_RE = re.compile(
    r"(?m)^[ \t]*(?P<domain>\d+)[ \t]*=>[ \t]*"
    r"self\.(?P<function>fold_[A-Za-z0-9_]+)[ \t]*\("
)
RAW_STRING_PREFIX_RE = re.compile(r"(?:b)?r(?P<hashes>#{0,255})\"")
CHAR_LITERAL_RE = re.compile(
    r"(?:b)?'(?:\\(?:x[0-9A-Fa-f]{2}|u\{[0-9A-Fa-f_]+\}|.)|[^\\'\r\n])'"
)

REQUIRED_LANE_AUTHORITIES = frozenset(
    {
        (
            51,
            "extractor/native/lanes/scripted_state_flags_variables_live/"
            "scripted_state_compact.rs",
            "decode_payload",
        ),
        (
            52,
            "extractor/native/lanes/occupation_resistance_compliance_live/"
            "domain44_compact.rs",
            "decode_payload",
        ),
        # d99's wire module carries no `compact_contract` dependency (the
        # `compact_contract_dependency_policy` boundary admits only d81), so it
        # transcribes the DCS1 decoder instead of calling it. This marker pins the
        # ORIGINAL, exactly as d51's and d52's do.
        (
            99,
            "extractor/native/compact_contract/src/division_combat_stats.rs",
            "decode_payload",
        ),
    }
)


class VerificationError(RuntimeError):
    """The checked-in citation authority does not match production source."""


@dataclass(frozen=True)
class Span:
    start: int
    end: int


def _blank_except_newlines(value: str) -> str:
    return "".join("\n" if char == "\n" else " " for char in value)


@lru_cache(maxsize=4)
def sanitize_rust(source: str) -> str:
    """Blank comments and literals while preserving byte count and newlines.

    This is deliberately a small lexical scanner, not a Rust parser.  It knows
    nested block comments, escaped ordinary/byte strings, raw/byte-raw strings,
    and character literals.  That is sufficient to make braces in diagnostics,
    JSON snippets, and comments invisible to the function-envelope walk.

    Cached because the span walk, the item index and the closure walk each want
    the same sanitized copy of the same 1.2 MB file, and building it is a
    character-at-a-time scan.
    """

    return _blank(source, comments=True)


def _blank(source: str, *, comments: bool) -> str:
    """The scanner behind both views.  ``comments=False`` keeps comment text."""

    out: list[str] = []
    index = 0
    length = len(source)
    while index < length:
        if source.startswith("//", index):
            end = source.find("\n", index)
            if end < 0:
                end = length
            segment = source[index:end]
            out.append(segment if not comments else _blank_except_newlines(segment))
            index = end
            continue

        if source.startswith("/*", index):
            start = index
            depth = 1
            index += 2
            while index < length and depth:
                if source.startswith("/*", index):
                    depth += 1
                    index += 2
                elif source.startswith("*/", index):
                    depth -= 1
                    index += 2
                else:
                    index += 1
            segment = source[start:index]
            out.append(segment if not comments else _blank_except_newlines(segment))
            continue

        raw = RAW_STRING_PREFIX_RE.match(source, index)
        if raw:
            hashes = raw.group("hashes")
            close = '"' + hashes
            start = index
            index = raw.end()
            end = source.find(close, index)
            index = length if end < 0 else end + len(close)
            out.append(_blank_except_newlines(source[start:index]))
            continue

        string_prefix = 2 if source.startswith('b"', index) else (
            1 if source[index] == '"' else 0
        )
        if string_prefix:
            start = index
            index += string_prefix
            escaped = False
            while index < length:
                char = source[index]
                index += 1
                if escaped:
                    escaped = False
                elif char == "\\":
                    escaped = True
                elif char == '"':
                    break
            out.append(_blank_except_newlines(source[start:index]))
            continue

        char_literal = CHAR_LITERAL_RE.match(source, index)
        if char_literal:
            out.append(_blank_except_newlines(source[index : char_literal.end()]))
            index = char_literal.end()
            continue

        out.append(source[index])
        index += 1

    return "".join(out)


def function_spans(source: str) -> dict[str, Span]:
    sanitized = sanitize_rust(source)
    spans: dict[str, Span] = {}
    for match in FUNCTION_RE.finditer(sanitized):
        name = match.group("name")
        # Only authority boundaries are indexed. world_fold.rs also contains
        # ordinary helpers/tests whose names may repeat in different scopes.
        if not (name.startswith("fold_") or name == "decode_payload"):
            continue
        open_brace = sanitized.find("{", match.end())
        if open_brace < 0:
            raise VerificationError(f"{name}: function has no opening brace")
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
            raise VerificationError(f"{name}: function has no closing brace")
        span = Span(
            start=source.count("\n", 0, match.start()) + 1,
            end=source.count("\n", 0, close_brace) + 1,
        )
        if name in spans:
            raise VerificationError(f"duplicate function name cannot be pinned: {name}")
        spans[name] = span
    return spans


@dataclass(frozen=True)
class Item:
    """One item defined in the cited file, with the scope that owns it."""

    ident: str
    owner: str
    name: str
    span: Span
    start: int
    end: int
    # "fn", or the value-item keyword (`const`/`static`/`struct`/`enum`/
    # `union`/`type`).  A value item can be NAMED by a fold but never CALLED, so
    # it is a closure member without being a candidate for a method call.
    kind: str = "fn"


class ItemIndex:
    """Every item in one file, addressable by owning scope and by bare name."""

    def __init__(
        self, items: list[Item], source: str, aliases: dict[str, str] | None = None
    ) -> None:
        # The source this index was built from, so the walk cannot be run
        # against a different one and hit the callee cache for answers that
        # describe text no longer present.
        self.source = source
        self.sanitized = sanitize_rust(source)
        self.items = {item.ident: item for item in items}
        # `type Alias = Row;` -- `Alias::helper()` executes `Row::helper`.
        self.aliases = aliases or {}
        self.by_owner: dict[tuple[str, str], list[str]] = {}
        self.by_name: dict[str, list[str]] = {}
        # Only callable items are candidates for an unresolved method call.
        self.callable_by_name: dict[str, list[str]] = {}
        # One index serves all 45 domains and their closures overlap heavily, so
        # resolving each function's callees once is the difference between a
        # gate that runs in well under a second and one that runs in five.
        self.callees: dict[str, set[str]] = {}
        for item in items:
            self.by_owner.setdefault((item.owner, item.name), []).append(item.ident)
            self.by_name.setdefault(item.name, []).append(item.ident)
            if item.kind == "fn":
                self.callable_by_name.setdefault(item.name, []).append(item.ident)

    def owned(self, owner: str, name: str) -> list[str]:
        """`(owner, name)`, following a chain of local type aliases.

        `type Alias = Mid; type Mid = Row;` needs both hops, and a cycle in
        a malformed file must not spin forever.
        """
        found = self.by_owner.get((owner, name))
        seen = {owner}
        while found is None and owner in self.aliases:
            owner = self.aliases[owner]
            if owner in seen:
                break
            seen.add(owner)
            found = self.by_owner.get((owner, name))
        return found or []


# The opening brace is deliberately NOT required on the header line: a header
# that wraps (`impl<'a> From<&'a Row>` / newline / `    for Projection<'a>` /
# newline / `{`) is the shape that made this scanner silently mis-own the `fn`
# inside it, and a mis-owned function is one whose `self.f()` callers no longer
# resolve -- silent UNDER-coverage, the exact failure this gate exists to stop.
SCOPE_RE = re.compile(
    r"(?m)^[ \t]*(?:(?:pub(?:\([^)\r\n]*\))?|unsafe)[ \t]+)*"
    r"(?:(?P<impl>impl)\b|(?:(?P<kind>mod|trait)[ \t]+(?:r#)?"
    r"(?P<name>[A-Za-z_][A-Za-z0-9_]*)\b))"
)
# Every line whose first token opens an item scope must be one SCOPE_RE matched.
# Return-position `impl Trait` and `f: impl Fn(..)` are mid-line and are not
# candidates; an item is not.
SCOPE_LINE_RE = re.compile(
    r"(?m)^[ \t]*(?:(?:pub(?:\([^)\r\n]*\))?|unsafe)[ \t]+)*(?:impl|mod|trait)\b"
)
# ...unless the line is the CONTINUATION of something else. A wrapped signature
# can put `impl Trait` at the start of a line (`) -> ` / newline / `impl Iterator
# <..> {`), and that is a return type, not an item.
CONTINUATION_CHARS = frozenset(",(+->:=&|<")
ANY_FUNCTION_RE = re.compile(
    r"(?m)^[ \t]*(?:(?:pub(?:\([^)\r\n]*\))?|unsafe|async|const|default|extern"
    r"(?:[ \t]+\"[^\"]+\")?)[ \t]+)*fn[ \t]+"
    r"(?:r#)?(?P<name>[A-Za-z_][A-Za-z0-9_]*)\b"
)
# A function is not the only thing a fold executes.  `DIVISION_FLAG_SUPPLY_GAIN`
# is a `const`, and re-spelling it `1 << 9` changes which bit the division fold
# reads while every `fn` in the closure stays byte-identical.  Constants,
# statics, enums, structs and type aliases are therefore indexed as items too,
# and a covered function that names one pulls it in through the same walk.
# A `use` binding decides WHICH implementation a bare name reaches.
# `hour_index_from_raw` is imported, not defined here, and re-pointing that
# import at another module changes what the covered helper executes while every
# indexed byte of this file stays the same.  The binding is therefore an item.
USE_ITEM_RE = re.compile(r"(?m)^[ \t]*(?:pub(?:\([^)\r\n]*\))?[ \t]+)?use[ \t]+")
USE_BINDING_RE = re.compile(
    r"(?:as[ \t\r\n]+(?:r#)?(?P<alias>[A-Za-z_][A-Za-z0-9_]*))"
    r"|(?:(?:r#)?(?P<segment>[A-Za-z_][A-Za-z0-9_]*)[ \t\r\n]*(?=[,}；;]|$))"
)
VALUE_ITEM_RE = re.compile(
    r"(?m)^[ \t]*(?:(?:pub(?:\([^)\r\n]*\))?|unsafe|default)[ \t]+)*"
    r"(?P<kind>const|static|enum|struct|union|type)[ \t]+(?:mut[ \t]+)?"
    r"(?!fn\b)(?:r#)?(?P<name>[A-Za-z_][A-Za-z0-9_]*)\b"
)
# EVERY identifier, not only the ones followed by `(`.  A function reached as a
# VALUE transfers control just as a call does -- `fold_divisions` reaches
# `DivisionFoldRow::equipment_join_key` only as
# `.and_then(DivisionFoldRow::equipment_join_key)`, and a call-shaped scan leaves
# that logic outside the closure entirely.  Whether a token is a call is decided
# from what FOLLOWS it, because that is what separates a method call from a field
# access of the same name.
IDENTIFIER_RE = re.compile(r"(?<![A-Za-z0-9_])(?:r#)?(?P<name>[A-Za-z_][A-Za-z0-9_]*)")
# Only `(` -- or a turbofish before it -- makes `x.name` a CALL.  `row.items[0]`
# and `if row.ready {` are field reads, and treating them as calls would fire the
# ambiguity refusal on every struct field that shares a method's name.
METHOD_CALL_SUFFIX_RE = re.compile(r"[ \t\r\n]*(?:::[ \t\r\n]*<|\()")
# An item keyword that is NOT the first thing on its line.  `impl` is excluded:
# it appears mid-line legitimately as `-> impl Iterator` and `f: impl Fn(..)`,
# and a line-leading one is already covered by the scope guard.
SECOND_ITEM_ON_A_LINE_RE = re.compile(
    r"(?m)^[^\r\n]*[}\];][ \t]*(?:pub(?:\([^)\r\n]*\))?[ \t]+)?"
    r"(?:fn|struct|enum|union|trait|mod|impl)\b"
)
MACRO_DEFINITION_RE = re.compile(
    r"macro_rules[ \t\r\n]*![ \t\r\n]*([A-Za-z_][A-Za-z0-9_]*)"
)
CALLEE_ANNOTATION_RE = re.compile(
    r"@fold-callee[ \t]+(?P<owner>[A-Za-z_][A-Za-z0-9_]*)"
    r"::(?P<name>[A-Za-z_][A-Za-z0-9_]*)"
)
# The other half of the declaration: the receivers of `.name()` in this function
# are types this file does not define (`String::as_str`), so the same-named local
# functions are NOT reachable.  Without it the only way to satisfy the gate would
# be to name a local type that is not the callee, which is a worse record than an
# explicit "not from here".
EXTERNAL_ANNOTATION_RE = re.compile(
    r"@fold-callee-external[ \t]+(?P<name>[A-Za-z_][A-Za-z0-9_]*)"
)


def _matching_brace(sanitized: str, from_index: int, label: str) -> tuple[int, int]:
    open_brace = sanitized.find("{", from_index)
    if open_brace < 0:
        raise VerificationError(f"{label}: no opening brace")
    depth = 0
    for index in range(open_brace, len(sanitized)):
        char = sanitized[index]
        if char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
            if depth == 0:
                return open_brace, index
    raise VerificationError(f"{label}: no closing brace")


def _impl_subject(tail: str) -> str:
    """The type an `impl` header applies to, generics and trait stripped.

    ``tail`` is everything between ``impl`` and the block's ``{``, which may
    span lines, so it is whitespace-normalized before the ``for``/``where``
    splits -- those are word boundaries in a wrapped header just as much as in a
    single-line one.
    """
    tail = " " + " ".join(tail.split()) + " "
    subject = tail.split(" for ", 1)[1] if " for " in tail else tail
    subject = subject.split(" where ", 1)[0].strip()
    # `<K: Ord, V>` immediately after `impl` belongs to the header, not the type.
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
    return subject.split("<")[0].strip().strip("&").strip()


def index_items(source: str) -> ItemIndex:
    """Index every `fn` in ``source`` under the scope that owns it.

    The owner is the innermost enclosing ``impl`` type, else the innermost
    enclosing ``mod``/``trait`` name, else the empty string for a free function.
    That is deliberately NOT a Rust name resolver: it is the coarsest key under
    which ``self.f()``, ``Type::f()``, and a bare ``f()`` can all be looked up,
    and every call it cannot key exactly is handled by the fallback in
    ``resolve_callees`` rather than by guessing.
    """

    sanitized = sanitize_rust(source)

    # A macro can expand to a call this scanner cannot see, so a file that
    # defines one is a file whose closure would silently under-cover.  There are
    # none today; the day someone adds one, the gate says so instead of quietly
    # covering less.
    macros = sorted(
        {match.group(1) for match in MACRO_DEFINITION_RE.finditer(sanitized)}
    )
    if macros:
        raise VerificationError(
            f"this file now defines macro(s) {macros}; a macro body can call a "
            f"function without naming it at the call site, so the authority "
            f"closure can no longer be trusted to cover what a cited fold runs"
        )

    def continues_a_previous_line(position: int) -> bool:
        cursor = position
        while cursor > 0 and sanitized[cursor - 1] in " \t\r\n":
            cursor -= 1
        return cursor > 0 and sanitized[cursor - 1] in CONTINUATION_CHARS

    scopes: list[tuple[int, int, str]] = []
    recognized: set[int] = set()
    for match in SCOPE_RE.finditer(sanitized):
        if match.group("impl") and continues_a_previous_line(match.start()):
            # `) -> ` / newline / `impl Iterator<..> {` is a return type wrapped
            # onto its own line, not an item.
            recognized.add(match.start())
            continue
        brace = sanitized.find("{", match.end())
        semicolon = sanitized.find(";", match.end())
        if brace < 0 or (0 <= semicolon < brace):
            # `mod tests;` -- a declaration, not a scope, and its contents are in
            # another file which this gate does not claim to cover.
            recognized.add(match.start())
            continue
        open_brace, close_brace = _matching_brace(
            sanitized, match.end(), "scope header"
        )
        if match.group("impl"):
            owner = _impl_subject(sanitized[match.end() : open_brace])
        else:
            owner = match.group("name")
        recognized.add(match.start())
        if owner:
            scopes.append((match.start(), close_brace, owner))

    # A scope form this scanner does not recognize is not a harmless gap: the
    # functions inside it get attributed to whatever encloses it, their callers'
    # `self.f()` stops resolving, and the closure quietly covers less while the
    # gate still passes.  Refuse instead.
    unrecognized = [
        source.count("\n", 0, match.start()) + 1
        for match in SCOPE_LINE_RE.finditer(sanitized)
        if match.start() not in recognized
    ]
    if unrecognized:
        raise VerificationError(
            f"unrecognized item-scope header(s) at line(s) {unrecognized}; the "
            f"functions they own would be attributed to the wrong scope"
        )

    # Every item pattern is anchored at a line start, so a SECOND item on the
    # same line (`fn first() {} fn helper() {}`, or `#[cfg(x)] impl Row {`) is
    # neither indexed nor refused, and a cited fold could call the one that was
    # missed.  Supporting the layout would mean unanchoring every pattern; the
    # file does not use it, so this refuses it instead of covering less.
    crowded_items = sorted(
        source.count("\n", 0, match.start()) + 1
        for match in SECOND_ITEM_ON_A_LINE_RE.finditer(sanitized)
    )
    if crowded_items:
        raise VerificationError(
            f"line(s) {crowded_items} declare more than one item; this scanner "
            f"indexes one item per line, so the later ones would be invisible to "
            f"the authority closure. Put each item on its own line."
        )

    def owning_scope(position: int) -> str:
        innermost = ""
        best = -1
        for start, end, owner in scopes:
            if start <= position <= end and start > best:
                best, innermost = start, owner
        return innermost

    # `source.count("\n", 0, position)` per item is a scan of the whole file per
    # lookup, and there are two lookups for each of 460 functions.  One sorted
    # list of newline offsets answers all of them.
    newlines = [index for index, char in enumerate(source) if char == "\n"]

    def line_of(position: int) -> int:
        return bisect.bisect_left(newlines, position) + 1

    # SANITIZED lines: a comment that happens to end in `]` reads as the tail of
    # a multi-line attribute and walks the span upward past it.  Blanked, a
    # comment terminates that walk exactly as an empty line does.
    lines = _source_lines(sanitized)
    items: list[Item] = []
    seen: Counter[str] = Counter()

    def add(name: str, start: int, end_index: int, kind: str = "fn") -> None:
        owner = owning_scope(start)
        ident = f"{owner}::{name}" if owner else name
        seen[ident] += 1
        if seen[ident] > 1:
            # Two items under one key: different traits implemented for one type
            # (`Display::fmt` and `Debug::fmt`), or one name in two `impl`
            # blocks.  Suffixing keeps both in the closure; dropping either
            # would silently uncover it.
            ident = f"{ident}#{seen[ident]}"
        items.append(
            Item(
                ident=ident,
                owner=owner,
                name=name,
                span=Span(
                    start=_attribute_start(lines, line_of(start)),
                    end=line_of(end_index),
                ),
                start=start,
                end=end_index + 1,
                kind=kind,
            )
        )

    for match in ANY_FUNCTION_RE.finditer(sanitized):
        kind, position = _item_terminator(sanitized, match.end())
        if kind != "brace":
            # `fn f(&self);` -- a trait method DECLARATION with no body.  Walking
            # to the next brace would hand it some later function's body.
            #
            # The scan is DEPTH-AWARE because a signature can carry a semicolon
            # of its own: `fn helper(values: [i32; 5]) { .. }` is a definition,
            # and reading that `;` as a terminator dropped the whole function
            # from the index -- so a cited fold could call it and the closure
            # would contain only the fold.
            continue
        _, close_brace = _matching_brace(sanitized, position, match.group("name"))
        add(match.group("name"), match.start(), close_brace)

    # A `const` declared INSIDE a function body is already inside that
    # function's span, so indexing it again would hash it twice -- and, because
    # such names repeat freely across bodies (`HEADER_LEN` thirteen times), it
    # would key them `#2`..`#13` by source ORDER, so inserting one earlier in the
    # file renames the rest and churns membership that did not change.
    enclosing_extents = [(item.start, item.end) for item in items]

    def inside_an_item(position: int) -> bool:
        return any(start <= position < end for start, end in enclosing_extents)

    aliases: dict[str, str] = {}
    for match in VALUE_ITEM_RE.finditer(sanitized):
        # Nested inside a function OR inside another value item: either way
        # its parent's span already hashes it, and indexing it again gives a
        # second same-named item that makes an unambiguous `crate::FLAG`
        # refuse for an ambiguity Rust does not have.
        if inside_an_item(match.start()):
            continue
        item_kind = match.group("kind")
        if item_kind in ("struct", "enum", "union"):
            # A braced body, or `struct Foo;` / `struct Foo(u32);`.
            kind, position = _item_terminator(sanitized, match.end())
            if kind == "brace":
                # From the terminator's OWN brace, not the first `{` after the
                # name: a braced const-generic default (`struct Row<const N:
                # usize = { 1 }>`) puts one in the header, and matching from
                # there ended the item before its fields.
                _, end_index = _matching_brace(
                    sanitized, position, match.group("name")
                )
            elif kind == "semicolon":
                end_index = position
            else:
                continue
        else:
            # `const`/`static`/`type` end at their OWN semicolon, which is the
            # first one at depth zero.  Taking the first `{` instead truncated
            # `const PICK: i32 = if c { A } else { B };` after the first branch,
            # and taking the first `;` truncated `static X: [u8; 4] = ..` inside
            # the array type.
            kind, position = _item_terminator(
                sanitized, match.end(), stop_at_brace=False
            )
            if kind != "semicolon":
                continue
            end_index = position
            if item_kind == "type":
                target = _alias_target(sanitized[match.end() : end_index])
                if target:
                    aliases[match.group("name")] = target
        add(match.group("name"), match.start(), end_index, item_kind)
        enclosing_extents.append((match.start(), end_index + 1))

    # LAST, so a block-local import inside a constant is already covered
    # by that constant's extent and is not globalized into `by_name`,
    # where it would make an unambiguous `crate::FLAG` refuse.
    for match in USE_ITEM_RE.finditer(sanitized):
        if inside_an_item(match.start()):
            continue
        kind, position = _item_terminator(
            sanitized, match.end(), stop_at_brace=False
        )
        if kind != "semicolon":
            continue
        declaration = sanitized[match.end() : position]
        # `use a::b::{c, d as e};` binds `c` and `e`.  Each binding is indexed
        # over the WHOLE statement, so re-pointing any of them changes the
        # digest of every closure that names one.
        for bound in USE_BINDING_RE.finditer(declaration):
            name = bound.group("alias") or bound.group("segment")
            if name and name not in ("self", "crate", "super"):
                add(name, match.start(), position, "use")

    return ItemIndex(items, source, aliases)


def _item_terminator(
    sanitized: str, start: int, *, stop_at_brace: bool = True
) -> tuple[str, int]:
    """Find where an item's header ends: its body brace, or its semicolon.

    Depth-aware over `()`, `[]` and `{}`, because a semicolon inside an array
    type (`[i32; 5]`) or a brace inside an initializer (`if c { A } else { B }`)
    belongs to the item, not after it.  Returns ``("brace"|"semicolon"|"", index)``.
    """
    depth = 0
    angle = 0
    for index in range(start, len(sanitized)):
        char = sanitized[index]
        if char in "([":
            depth += 1
        elif char in ")]":
            depth -= 1
        # An adjacent `<<` is a SHIFT, not two generic opens: `{ 1 << 2 }`
        # as a const-generic default otherwise left the angle depth stuck
        # open and the real body brace looked nested.  `>>` still
        # decrements twice, because that is how `Vec<Vec<u32>>` closes --
        # guarding both sides broke every nested generic in the file.
        elif char == "<" and not (
            sanitized[index : index + 2] == "<<"
            or (index > start and sanitized[index - 1] == "<")
        ):
            # Generic parameter lists can carry a braced default
            # (`struct Row<const N: usize = { 1 }>`), whose `{` is NOT the item
            # body.  `->` and `=>` are excluded below so a return type does not
            # unbalance the count.
            angle += 1
        elif char == ">" and index > start and sanitized[index - 1] not in "-=":
            angle = max(0, angle - 1)
        elif char == "{":
            if depth == 0 and angle == 0 and stop_at_brace:
                return "brace", index
            depth += 1
        elif char == "}":
            depth -= 1
        elif char == ";" and depth == 0:
            # A `;` cannot appear inside a real generic list, so it terminates
            # regardless of `angle` -- which also unwedges a comparison
            # (`const X: bool = 1 < 2;`) that inflated the count.
            return "semicolon", index
    return "", -1


def _alias_target(declaration: str) -> str:
    """The base type a `type A = B<..>;` alias points at, or "".

    `Alias::helper()` executes `Row::helper`, so an index that only knows
    `(Alias, helper)` silently resolves nothing and the method leaves the
    closure while the alias and the underlying type both stay in it.
    """
    if "=" not in declaration:
        return ""
    target = declaration.split("=", 1)[1].strip()
    target = target.split("<")[0].strip().lstrip("&").strip()
    # `type Alias = m::Row;` aliases `Row`, not the module `m`.
    target = target.split("::")[-1].strip()
    match = re.match(r"(?:r#)?([A-Za-z_][A-Za-z0-9_]*)", target)
    return match.group(1) if match else ""


def _attribute_above(lines: tuple[str, ...], line: int) -> bool:
    """Is the next non-blank line above ``line`` a single-line attribute?"""
    cursor = line - 1
    while cursor >= 1:
        text = lines[cursor - 1].strip()
        if text:
            return text.startswith("#") and text.endswith("]")
        cursor -= 1
    return False


def _attribute_start(lines: tuple[str, ...], fn_line: int) -> int:
    """The first line of the attribute block above ``fn_line``, or ``fn_line``.

    An attribute is part of the function it decorates and can decide whether it
    is the one compiled at all: ``world_fold.rs`` carries two
    ``injected_record_fault`` bodies selected by ``#[cfg(not(test))]`` /
    ``#[cfg(test)]``.  A span that starts at ``fn`` leaves those selectors
    outside every digest, so swapping which body ships changes no membership and
    no digest.  Doc comments are deliberately NOT included: they change no
    behaviour, and hashing them would fail the gate on a typo fix.
    """
    start = fn_line
    cursor = fn_line - 1
    pending = 0
    while cursor >= 1:
        text = lines[cursor - 1].strip()
        if not text:
            # Rust allows `#[cfg(..)]`, a blank line or comment, then the
            # item.  Keep climbing: a blank line does not end the attribute
            # block, only a line that is neither blank nor an attribute does.
            # (Comments are blank in the sanitized view this walks.)
            if pending == 0 and not _attribute_above(lines, cursor):
                break
            cursor -= 1
            continue
        if pending == 0:
            if not text.endswith("]"):
                break
            if text.startswith("#"):
                start = cursor
                cursor -= 1
                continue
            # The tail of an attribute spread over several lines.
            pending = text.count("]") - text.count("[")
            cursor -= 1
            continue
        pending += text.count("]") - text.count("[")
        if pending <= 0:
            if not text.startswith("#"):
                # Balanced, but never an attribute -- do not swallow code.
                break
            start = cursor
            pending = 0
        cursor -= 1
    return start


def _token_before(body: str, index: int) -> tuple[str, int]:
    """The identifier ending at or before ``index``, and where it starts."""
    end = index
    while end > 0 and body[end - 1] in " \t\r\n":
        end -= 1
    start = end
    while start > 0 and (body[start - 1].isalnum() or body[start - 1] == "_"):
        start -= 1
    return body[start:end], start


def _turbofish_owner(body: str, index: int) -> str:
    """The type in front of a `Type::<..>::` turbofish ending at ``index``.

    Returns "" when the angle brackets do not balance or nothing precedes them
    (``<[u8; 4]>::try_from``), which the caller treats as an owner it could not
    read rather than as no owner at all.
    """
    cursor = index
    while cursor > 0 and body[cursor - 1] in " \t\r\n":
        cursor -= 1
    if cursor == 0 or body[cursor - 1] != ">":
        return ""
    depth = 0
    while cursor > 0:
        char = body[cursor - 1]
        if char == ">":
            depth += 1
        elif char == "<":
            depth -= 1
            if depth == 0:
                cursor -= 1
                break
        cursor -= 1
    else:
        return ""
    if cursor >= 2 and body[cursor - 2 : cursor] == "::":
        cursor -= 2
    owner, _ = _token_before(body, cursor)
    return owner


def _call_sites(body: str) -> Iterable[tuple[str, str | None, str, int]]:
    """Classify each identifier in a sanitized function body.

    Yields ``(kind, qualifier, name)`` where kind is ``path`` (``A::b``),
    ``self`` (``self.b(``), ``method`` (any other ``expr.b(``), or ``free``.

    A PATH is yielded whether or not it is followed by ``(``, because
    ``.and_then(Type::method)`` hands control to that method exactly as a call
    does, and a call-shaped scan leaves it outside the closure.  A ``.name``
    that is NOT followed by ``(`` is yielded for nothing: Rust has no way to
    take a method by value through a receiver, so those are field accesses, and
    treating them as calls would drag the ambiguity refusal onto every struct
    field that shares a method's name.

    The classification reads backwards a character at a time rather than
    matching a regex against the whole preceding body: it is exact regardless of
    how the call is wrapped, and it does not make the walk quadratic in the
    length of a 600-line fold function.
    """

    for match in IDENTIFIER_RE.finditer(body):
        name = match.group("name")
        cursor = match.start()
        while cursor > 0 and body[cursor - 1] in " \t\r\n":
            cursor -= 1
        if cursor >= 2 and body[cursor - 2 : cursor] == "::":
            segment, _ = _token_before(body, cursor - 2)
            if not segment:
                segment = _turbofish_owner(body, cursor - 2)
            if segment in ("crate", "super", "self"):
                # A module path this scanner does not model.  Treated as an
                # unread owner so it reaches the refusal rather than resolving
                # to a scope named "crate" that does not exist.
                segment = ""
            if segment:
                yield "path", segment, name, match.start()
                continue
            # A qualifier this cannot read is the same unknown as a receiver it
            # cannot read, and must reach the same refusal rather than resolve to
            # nothing quietly.  Yielded whether or not it is called, because
            # `<Row as Key>::join_key` hands over control as a value too.
            #
            # Its own kind, not "method": `crate::FLAG` can name a CONSTANT,
            # and the method rules consult callable items only, so a value
            # behind an unreadable qualifier fell through and vanished.
            yield "qualified", None, name, match.start()
            continue
        if cursor >= 1 and body[cursor - 1] == ".":
            if METHOD_CALL_SUFFIX_RE.match(body, match.end()):
                receiver, _ = _token_before(body, cursor - 1)
                # `foo().bar(` and `x[0].bar(` leave no receiver token, so the
                # receiver type is not something the call site spells.
                yield (
                    ("self" if receiver == "self" else "method"),
                    None,
                    name,
                    match.start(),
                )
            continue
        yield "free", None, name, match.start()


def _line_offsets(text: str):
    """A 0-based line number for any offset inside ``text``."""
    starts = [0]
    for index, char in enumerate(text):
        if char == "\n":
            starts.append(index + 1)

    def line_of(position: int) -> int:
        return bisect.bisect_right(starts, position) - 1

    return line_of


def resolve_callees(index: ItemIndex, item: Item) -> set[str]:
    """The locally defined items ``item`` can transfer control to.

    Resolution is exact where the call spells its owner (``self.f()``,
    ``Type::f()``, ``module::f()``, a bare ``f()``).  A method call on any other
    receiver names no type, so:

    * exactly one local ``fn`` carries that name -- take it.  Over-inclusion
      (the receiver was really a `std` type with a same-named method) costs one
      redundant closure member; exclusion would cost coverage.
    * several do -- REFUSE.  Picking one would under-cover, taking all would
      chain every domain's digest to every other renderer named ``to_json``.
      The calling function declares the receiver with a
      ``// @fold-callee Type::method`` comment instead.

    Refusing rather than guessing is what makes the closure sound: there is no
    path by which a call reaches a local function that the closure omits without
    the gate saying so.
    """

    cached = index.callees.get(item.ident)
    if cached is not None:
        return cached
    body = index.sanitized[item.start : item.end]
    # Declarations are read from the view where comments survive but string
    # literals do not, so `"// @fold-callee Type::method"` inside a diagnostic
    # cannot act as authority.
    raw_body = blank_literals_only(index.source)[item.start : item.end]
    # (name, declaration line) -> the idents it declares.  Keyed by LINE as
    # well as name so a declaration governs the call it was written for and
    # not every later call that happens to share the method name.
    declared: dict[tuple[str, int], set[str]] = {}
    body_line = _line_offsets(raw_body)
    for annotation in CALLEE_ANNOTATION_RE.finditer(raw_body):
        owner, name = annotation.group("owner"), annotation.group("name")
        idents = [
            ident
            for ident in index.owned(owner, name)
            if index.items[ident].kind == "fn"
        ]
        if not idents:
            # A declaration naming a CONSTANT would be accepted and would
            # then suppress the real method's ambiguity, omitting the body it
            # claims to describe.
            raise VerificationError(
                f"{item.ident}: `@fold-callee {owner}::{name}` names no function "
                f"defined in this file"
            )
        declared.setdefault((name, body_line(annotation.start())), set()).update(
            idents
        )
    for annotation in EXTERNAL_ANNOTATION_RE.finditer(raw_body):
        name = annotation.group("name")
        if name not in index.by_name:
            raise VerificationError(
                f"{item.ident}: `@fold-callee-external {name}` declares away a "
                f"name this file never defines, so it declares nothing; remove it"
            )
        declared.setdefault((name, body_line(annotation.start())), set())

    resolved: set[str] = set()
    governed_calls: dict[tuple[str, int], int] = {}
    unresolved: dict[str, tuple[list[str], list[int]]] = {}
    used: set[tuple[str, int]] = set()
    for kind, qualifier, name, position in _call_sites(body):
        if kind == "path":
            owner = item.owner if qualifier == "Self" else qualifier
            resolved.update(index.owned(owner or "", name))
            continue
        if kind == "self":
            resolved.update(index.owned(item.owner, name))
            continue
        if kind == "free":
            local = index.owned(item.owner, name) or index.owned("", name)
            resolved.update(local)
            continue
        # kind == "method" or "qualified"
        # A declaration governs its own line and the line after it, which is
        # how it is written: the comment sits on the call, or immediately
        # above it.
        call_line = body_line(position)
        governing = [
            key
            for key in ((name, call_line), (name, call_line - 1))
            if key in declared
        ]
        if governing:
            for key in governing:
                resolved.update(declared[key])
                used.add(key)
            governed_calls[(name, call_line)] = (
                governed_calls.get((name, call_line), 0) + 1
            )
            continue
        # A method call can only reach a callable item; an unreadable PATH
        # (`crate::FLAG`, `<Row as Key>::JOIN`) can equally name a constant, a
        # static or a type, so it consults every item of that name.
        candidates = (
            index.by_name.get(name)
            if kind == "qualified"
            else index.callable_by_name.get(name)
        )
        if not candidates:
            continue
        if len(candidates) == 1:
            resolved.update(candidates)
            continue
        unresolved.setdefault(name, (candidates, []))[1].append(
            item.span.start + call_line
        )

    # A declaration that resolves nothing is a claim about a call that is no
    # longer here: the call was renamed, moved, or deleted, and what is left is a
    # standing permission to omit a callee nobody has looked at since.
    # A declaration is bound to a LINE, so two same-named calls on one line
    # share it and the second receiver is silently assumed to be the first.
    crowded = sorted(
        f"{name} (line {item.span.start + line})"
        for (name, line), count in governed_calls.items()
        if count > 1
    )
    if crowded:
        raise VerificationError(
            f"{item.ident}: a declaration governs a LINE, and these carry more "
            f"than one call of the same name so the receivers cannot be told "
            f"apart: {crowded}. Put each call on its own line with its own "
            f"declaration."
        )

    stale = sorted(name for name, _ in set(declared) - used)
    if stale:
        raise VerificationError(
            f"{item.ident}: `@fold-callee` declaration(s) for {stale} govern no "
            f"call on their own line or the next one; move each declaration "
            f"onto the call it describes, or delete it"
        )

    if unresolved:
        detail = "; ".join(
            f"`{name}` at world_fold.rs line(s) {lines} could be "
            f"{sorted(candidates)}"
            for name, (candidates, lines) in sorted(unresolved.items())
        )
        raise VerificationError(
            f"{item.ident}: a reference reaches a locally defined item whose "
            f"owner is not written at the site, and more than one local item "
            f"carries that name, so the authority closure cannot be computed: "
            f"{detail}. Declare it on the line above with a "
            f"`// @fold-callee Type::name` comment."
        )
    resolved.discard(item.ident)
    index.callees[item.ident] = resolved
    return resolved


def callee_closure(index: ItemIndex, roots: Iterable[str]) -> set[str]:
    """Every local item reachable from ``roots``, roots included."""

    members: set[str] = set()
    pending = list(roots)
    while pending:
        ident = pending.pop()
        if ident in members:
            continue
        members.add(ident)
        item = index.items.get(ident)
        if item is None:
            raise VerificationError(f"unknown authority function {ident}")
        if item.kind in ("const", "static"):
            # A constant built from other constants carries them:
            # `KNOWN_FLAGS = A | B | C` means nothing without A, B and C, and
            # changing one of them changes what `KNOWN_FLAGS` admits while its
            # own bytes stay identical.  Only VALUES are followed, never the
            # field types a struct names -- see below.
            pending.extend(
                ident
                for ident in resolve_callees(index, item)
                if index.items[ident].kind in ("const", "static", "fn")
            )
            continue
        if item.kind != "fn":
            # A TYPE IS A LEAF.  Its own text is hashed, which is what catches a
            # widened field or a changed enum discriminant, but the walk is not
            # followed through the types its fields name: doing that made
            # `WorldFold` drag all 138 state structs into all 45 closures, so
            # every domain's digest moved whenever any other domain's state
            # changed and the membership list stopped describing the domain.
            continue
        pending.extend(resolve_callees(index, item))
        if item.owner:
            # The TYPE a method belongs to is part of what that method operates
            # on: every cited fold reads `WorldFold`'s fields directly, so
            # widening one changes what the fold does while the method's own
            # bytes stay identical.  The owner is a lookup key everywhere else;
            # here it is a dependency.
            pending.extend(index.owned("", item.owner))
    return members


def closure_digest(index: ItemIndex, members: Iterable[str]) -> str:
    """A position-independent digest of the closure's source.

    Keyed by qualified name and ordered by it, so inserting an unrelated
    function ABOVE a covered one does not change the digest -- only the covered
    text, or the set of covered names, does.  That is what separates "the file
    moved" (re-stamp mechanically) from "the covered logic changed" (review it).
    """

    digest = hashlib.sha256()
    for ident in sorted(members):
        item = index.items[ident]
        digest.update(ident.encode("utf-8"))
        digest.update(b"\n")
        digest.update(
            _normalized_span_text(index.source, item.span).encode("utf-8")
        )
    return digest.hexdigest()


@lru_cache(maxsize=4)
def blank_literals_only(source: str) -> str:
    """Like ``sanitize_rust`` but KEEPING comments, byte count preserved.

    Receiver declarations live in comments, so they cannot be read from the
    sanitized copy; read from the RAW source, a string literal containing
    ``// @fold-callee Type::method`` would act as authority.  This is the one
    view where a declaration is exactly a declaration.
    """
    return _blank(source, comments=False)


@lru_cache(maxsize=4)
def _source_lines(source: str) -> tuple[str, ...]:
    """`splitlines()` of a 1.2 MB file, once instead of once per span.

    A closure digest reads one span per member, and 45 domains share members, so
    an uncached split here dominated the whole gate's runtime.
    """
    return tuple(source.splitlines())


def _normalized_span_text(source: str, span: Span) -> str:
    lines = _source_lines(source)
    if span.start < 1 or span.end < span.start or span.end > len(lines):
        raise VerificationError(
            f"invalid source span {span.start}-{span.end} for {len(lines)} lines"
        )
    return "\n".join(lines[span.start - 1 : span.end]) + "\n"


def normalized_span_hash(source: str, span: Span) -> str:
    return hashlib.sha256(
        _normalized_span_text(source, span).encode("utf-8")
    ).hexdigest()


def domain_dispatch(source: str, spans: dict[str, Span]) -> dict[int, str]:
    dispatch_span = spans.get("fold_domain_data")
    if dispatch_span is None:
        raise VerificationError("world_fold.rs has no fold_domain_data")
    lines = source.splitlines()
    dispatch_source = "\n".join(
        lines[dispatch_span.start - 1 : dispatch_span.end]
    )
    dispatch: dict[int, str] = {}
    for match in DISPATCH_RE.finditer(dispatch_source):
        domain = int(match.group("domain"))
        function = match.group("function")
        prior = dispatch.setdefault(domain, function)
        if prior != function:
            raise VerificationError(
                f"domain {domain} dispatches to both {prior} and {function}"
            )
    return dispatch


def _read_utf8(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except (OSError, UnicodeError) as error:
        raise VerificationError(f"cannot read {path}: {error}") from error


def _only_match(pattern: re.Pattern[str], source: str, label: str) -> re.Match[str]:
    matches = list(pattern.finditer(source))
    if len(matches) != 1:
        raise VerificationError(f"{label}: expected one marker, found {len(matches)}")
    return matches[0]


def gated_citation(marker: re.Match[str]) -> str:
    """The one human citation the ``lines=``/sha256 marker backs."""
    return f"`world_fold.rs:{marker.group('start')}`-`:{marker.group('end')}`"


def ungated_prose_references(source: str, marker: re.Match[str]) -> list[int]:
    """Every `world_fold.rs:N` in a module that the digest gate does not own.

    The gated citation -- the ``lines=`` marker's paired human form -- is the one
    place a line number belongs, because the sha256 beside it fails the moment
    those bytes move.  Every OTHER line number is free text that nothing checks,
    drifts silently on the next insertion, and should name the item instead.

    ONE occurrence is owned, not every occurrence.  Treating them all as owned
    would let a second, unpaired copy of the gated text smuggle two unchecked
    line numbers past the ratchet, and the digest backs a citation, not a string
    that happens to look like one.  ``verify_repo`` separately requires the
    citation to appear exactly once, so the first match is that one occurrence.
    """
    gated = gated_citation(marker)
    found = re.search(re.escape(gated), source)
    owned = (found.start(), found.end()) if found else (-1, -1)
    return sorted(
        int(reference.group(1))
        for reference in PROSE_REFERENCE_RE.finditer(source)
        if not owned[0] <= reference.start() < owned[1]
    )


def load_prose_allowance(repo_root: Path) -> dict:
    """The grandfathered free-text line numbers, or nothing.

    A missing file grandfathers NOTHING, which fails closed: a tree that lost
    the file fails loudly instead of quietly permitting everything.  A corrupt
    one is an error rather than an empty allowance, for the same reason.
    """
    allowance_path = repo_root / PROSE_ALLOWANCE
    if not allowance_path.is_file():
        return {}
    try:
        loaded = json.loads(allowance_path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        raise VerificationError(
            f"cannot read the prose-reference allowance {allowance_path}: {error}"
        ) from error
    if not isinstance(loaded, dict):
        # Valid JSON of the wrong shape reached `.get` and raised an
        # AttributeError outside the VerificationError handler.
        raise VerificationError(
            f"{allowance_path}: expected a JSON object at the root"
        )
    return loaded


def verify_document_references(repo_root: Path, allowance: dict) -> int:
    """Fail when a design document gains a ``world_fold.rs`` line number.

    Same ratchet as ``verify_prose_references`` and for a sharper reason.  A
    module's stale line number sits next to a digest-gated marker that a reader
    can fall back on; a document's does not.  Nothing compiles prose, no gate
    read these before this one, and the citation's only reader is a human who
    opens ``world_fold.rs`` at that line and believes whatever is there --
    ``DOMAIN_SCHEMA_RECIPE.md`` alone carried eighteen of them.

    The budget is per (document, line number), exactly as for modules, with the
    same accepted hole: deleting one occurrence and citing the SAME number again
    elsewhere in the SAME document passes.  Binding to a position would mean
    recording a line number in a file whose own line numbers move.

    Absence of a document root is not failure -- the module fixtures under test
    have no document tree at all -- but absence of the ALLOWANCE still
    grandfathers nothing.

    KNOWN LIMIT: ``*.md`` only, and only under ``DOCUMENT_ROOTS``.  No other file
    under those trees cites ``world_fold.rs`` by line today (the dossier JSON
    does not), and the code that does -- ``collector/src/attach_log.rs``,
    ``recorder_host/src/world_fold/tests.rs``, the ``wire_format`` bench
    binaries -- sits outside the dNN module set the marker gate scans and is not
    claimed here.
    """
    permitted: dict[str, list[int]] = allowance.get("documents", {})
    documents: list[Path] = []
    for root in DOCUMENT_ROOTS:
        document_root = repo_root / root
        if document_root.is_dir():
            documents.extend(document_root.rglob("*.md"))
    scanned = {document.relative_to(repo_root).as_posix() for document in documents}

    # A DOCUMENT THE ALLOWANCE NAMES, THAT EXISTS, AND THAT THIS SCAN DID NOT
    # REACH, DISABLES THE CHECK FOR IT SILENTLY.  Renaming a configured root,
    # moving a document out of one, or changing its extension all leave the
    # citations on disk and the gate reporting a smaller count -- and a count
    # nobody compares against anything is not a check.  Absence is still fine:
    # deleting a document must not force an allowance edit in the same commit.
    # (Codex review 019fdc8a-6a81-7422-af0f-82edb26a4e31, finding 8.)
    unreachable = sorted(
        relative
        for relative in permitted
        if relative not in scanned and (repo_root / relative).is_file()
    )
    if unreachable:
        raise VerificationError(
            "these documents are grandfathered but were not scanned, so their "
            f"`world_fold.rs:<line>` references are unchecked: {unreachable}. "
            f"Either bring them under one of {[str(r) for r in DOCUMENT_ROOTS]} "
            "as `*.md`, or add their location to DOCUMENT_ROOTS."
        )

    total = 0
    for document in sorted(documents):
        relative = document.relative_to(repo_root).as_posix()
        found = sorted(
            int(reference.group(1))
            for reference in PROSE_REFERENCE_RE.finditer(_read_utf8(document))
        )
        total += len(found)
        budget = Counter(permitted.get(relative, []))
        for line in found:
            if budget[line] <= 0:
                raise VerificationError(
                    f"{relative}: `world_fold.rs:{line}` is a free-text line "
                    f"number nothing checks. Line numbers drift on the next "
                    f"insertion into world_fold.rs, and a document's reader has "
                    f"no digest beside it to notice; cite the item instead, as "
                    f"`world_fold.rs::<item_name>`. (If this reference genuinely "
                    f"cannot be named, add it to {PROSE_ALLOWANCE.name} under "
                    f'"documents" with the reason.)'
                )
            budget[line] -= 1
    return total


def verify_prose_references(
    repo_root: Path, modules: Iterable[Path], markers: dict[str, re.Match[str]]
) -> int:
    """Fail when a module gains a world_fold.rs line number outside the gate.

    This is a ratchet, not a clean-tree assertion.  The references that survive
    today are the ones no tool could resolve to an item with total confidence --
    ranges spanning several items, struct fields, and citations that were already
    off by one when they were written.  They are listed per module in
    ``world_fold_prose_reference_allowance.json``, and the list may only shrink:
    citing a NEW line number, or swapping one for another, fails here.

    Known and accepted limit: the budget is per (module, line number), not per
    site.  Deleting one grandfathered occurrence and citing the SAME number again
    elsewhere in the SAME module passes.  Binding to a site would mean recording
    a position in a file whose own line numbers move -- the very failure this
    check exists to stop -- and the hole admits no line number that module was
    not already allowed to cite.
    """
    allowance = load_prose_allowance(repo_root)
    permitted: dict[str, list[int]] = allowance.get("modules", {})

    total = 0
    for module in modules:
        name = module.name
        found = ungated_prose_references(_read_utf8(module), markers[name])
        total += len(found)
        budget = Counter(permitted.get(name, []))
        for line in found:
            if budget[line] <= 0:
                raise VerificationError(
                    f"{name}: `world_fold.rs:{line}` is a free-text line number "
                    f"the digest gate does not own. Line numbers drift on the "
                    f"next insertion into world_fold.rs and nothing catches it; "
                    f"cite the item instead, as `world_fold.rs::<item_name>`. "
                    f"(If this reference genuinely cannot be named, add it to "
                    f"{PROSE_ALLOWANCE.name} with the reason.)"
                )
            budget[line] -= 1
    return total


def authority_roots(index: ItemIndex, functions: Iterable[str]) -> list[str]:
    """The indexed idents for a marker's ``functions=`` list."""
    roots: list[str] = []
    for name in functions:
        idents = index.by_name.get(name)
        if not idents:
            raise VerificationError(f"missing world-fold function {name}")
        if len(idents) > 1:
            raise VerificationError(
                f"authority function {name} is defined {len(idents)} times "
                f"({sorted(idents)}); an authority must name one function"
            )
        roots.append(idents[0])
    return roots


def load_closure_inventory(repo_root: Path) -> dict[str, list[str]]:
    """The recorded closure membership, or a hard failure.

    A missing inventory is NOT an empty one.  An empty inventory would make the
    membership check vacuous while the run still printed PASS, which is the
    shape of the defect this whole gate exists to stop.
    """
    path = repo_root / CLOSURE_INVENTORY
    if not path.is_file():
        raise VerificationError(
            f"the authority closure inventory {CLOSURE_INVENTORY} is missing; "
            f"regenerate it with extractor/native/scripts/restamp_world_fold_citations.py"
        )
    try:
        loaded = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        raise VerificationError(
            f"cannot read the authority closure inventory {path}: {error}"
        ) from error
    if not isinstance(loaded, dict):
        raise VerificationError(
            f"{CLOSURE_INVENTORY}: expected a JSON object at the root"
        )
    domains = loaded.get("domains")
    if not isinstance(domains, dict):
        raise VerificationError(
            f"{CLOSURE_INVENTORY}: expected an object under \"domains\""
        )
    # A domain whose value is not a list of names reaches `sorted()` and
    # `set()` later and raises a TypeError there instead of saying what is
    # wrong with the file.
    malformed = sorted(
        key
        for key, members in domains.items()
        if not isinstance(members, list)
        or not all(isinstance(member, str) for member in members)
    )
    if malformed:
        raise VerificationError(
            f"{CLOSURE_INVENTORY.name}: domain(s) {malformed} do not "
            f"record a list of member names"
        )
    return domains


def verify_repo(
    repo_root: Path,
    *,
    required_lane_authorities: Iterable[tuple[int, str, str]] = (
        REQUIRED_LANE_AUTHORITIES
    ),
    # (dNN modules, lane authorities, grandfathered module line references,
    # grandfathered document line references)
) -> tuple[int, int, int, int]:
    repo_root = repo_root.resolve()
    world_path = repo_root / WORLD_FOLD
    world_source = _read_utf8(world_path)
    spans = function_spans(world_source)
    dispatch = domain_dispatch(world_source, spans)
    world_index = index_items(world_source)
    recorded_closures = load_closure_inventory(repo_root)

    wire_dir = repo_root / WIRE_SRC
    modules = sorted(
        (
            path
            for path in wire_dir.glob("d*.rs")
            if re.fullmatch(r"d[0-9]+\.rs", path.name)
        ),
        key=lambda path: int(path.stem[1:]),
    )
    if not modules:
        raise VerificationError(f"no dNN modules found under {wire_dir}")

    # An inventory key with no module is a membership record nothing
    # compares against anything.  Left alone it accumulates coverage
    # claims for domains that no longer exist -- the same shape as a
    # grandfathered document no scan reaches, which this gate refuses.
    module_domains = {str(int(path.stem[1:])) for path in modules}
    orphaned = sorted(set(recorded_closures) - module_domains)
    if orphaned:
        raise VerificationError(
            f"{CLOSURE_INVENTORY.name} records closure membership for "
            f"domain(s) {orphaned}, which have no dNN module; nothing "
            f"checks those entries, so they are a coverage claim no run "
            f"verifies. Re-stamp to drop them."
        )

    lane_markers: dict[tuple[int, str, str], tuple[Span, str]] = {}
    # Every module is checked on every run and each failure is retained. A
    # checker that stops at the FIRST drifted module reports one defect and
    # hides the rest: one stale d11 range masked 31 further stale ranges for
    # days, and because this check runs before Cargo, the whole central gate
    # was unreachable the entire time. Report the complete set so one fix pass
    # can close it.
    #
    # The marker checks below are deliberately NOT also run as a fail-fast
    # pre-pass: the same checks reporting twice would restore exactly the
    # stop-at-the-first-defect behaviour this accumulator exists to remove.
    module_failures: list[str] = []
    world_markers: dict[str, re.Match[str]] = {}
    for module in modules:
        domain = int(module.stem[1:])
        module_source = _read_utf8(module)
        try:
            marker = _only_match(
                WORLD_MARKER_RE, module_source, module.relative_to(repo_root).as_posix()
            )
            world_markers[module.name] = marker
            marker_domain = int(marker.group("domain"))
            if marker_domain != domain:
                raise VerificationError(
                    f"{module.name}: marker domain {marker_domain} != filename domain {domain}"
                )
            functions = marker.group("functions").split(",")
            dispatched = dispatch.get(domain)
            if dispatched is None:
                raise VerificationError(f"{module.name}: domain absent from fold_domain_data")
            if functions[0] != dispatched:
                raise VerificationError(
                    f"{module.name}: first authority {functions[0]} != dispatch {dispatched}"
                )
            try:
                function_ranges = [spans[name] for name in functions]
            except KeyError as error:
                raise VerificationError(
                    f"{module.name}: missing world-fold function {error.args[0]}"
                ) from error
            expected = Span(
                min(span.start for span in function_ranges),
                max(span.end for span in function_ranges),
            )
            declared = Span(int(marker.group("start")), int(marker.group("end")))
            if declared != expected:
                raise VerificationError(
                    f"{module.name}: declared lines {declared.start}-{declared.end}, "
                    f"actual {expected.start}-{expected.end}"
                )
            # Membership is checked BEFORE the envelope digest, because the two
            # can fail together and this is the one that says something the
            # other does not: the envelope reports that bytes moved, which a
            # re-stamp answers, while this reports that a function stopped being
            # covered, which a re-stamp must not.
            #
            # The envelope digest covers the cited functions where they stand
            # and nothing else.  It does NOT survive a helper being lifted out
            # of one: the envelope narrows to the residual, the re-stamp records
            # the narrower bytes, and the moved logic is outside every digest
            # while the gate still prints PASS.  The closure covers the cited
            # functions AND everything in this file they call, wherever it sits.
            members = callee_closure(
                world_index, authority_roots(world_index, functions)
            )
            recorded = recorded_closures.get(str(domain))
            if recorded is None:
                raise VerificationError(
                    f"{module.name}: no closure membership recorded in "
                    f"{CLOSURE_INVENTORY.name}"
                )
            if sorted(recorded) != sorted(members):
                dropped = sorted(set(recorded) - members)
                gained = sorted(members - set(recorded))
                raise VerificationError(
                    f"{module.name}: authority closure membership changed; "
                    f"no longer covered={dropped}, newly covered={gained}. "
                    f"A name leaving this list is coverage leaving the gate -- "
                    f"confirm the logic moved somewhere still cited before "
                    f"re-stamping."
                )
            expected_hash = normalized_span_hash(world_source, expected)
            if marker.group("sha256") != expected_hash:
                raise VerificationError(
                    f"{module.name}: authority digest changed for "
                    f"{expected.start}-{expected.end}"
                )
            expected_closure = closure_digest(world_index, members)
            if marker.group("closure") != expected_closure:
                raise VerificationError(
                    f"{module.name}: authority closure digest changed over "
                    f"{len(members)} function(s) reachable from "
                    f"{','.join(functions)}"
                )
            human_citation = gated_citation(marker)
            # Exactly once, not at least once.  The digest backs one citation; a
            # second copy of the same text is two more unchecked line numbers,
            # and the free-text ratchet would read them as gated.
            occurrences = module_source.count(human_citation)
            if occurrences != 1:
                raise VerificationError(
                    f"{module.name}: expected exactly one human authority citation "
                    f"{human_citation}, found {occurrences}"
                )
        except VerificationError as error:
            module_failures.append(str(error))

        # Lane markers are an independent authority (they pin a capture-lane
        # decoder, not the fold envelope), so they are still collected for a
        # module whose world-fold citation drifted. Skipping them would turn one
        # stale line range into a spurious "lane authority marker set drifted".
        for lane in LANE_MARKER_RE.finditer(module_source):
            lane_domain = int(lane.group("domain"))
            if lane_domain != domain:
                module_failures.append(
                    f"{module.name}: lane marker domain {lane_domain} != {domain}"
                )
                continue
            key = (lane_domain, lane.group("path"), lane.group("function"))
            if key in lane_markers:
                module_failures.append(f"duplicate lane authority marker: {key}")
                continue
            lane_markers[key] = (
                Span(int(lane.group("start")), int(lane.group("end"))),
                lane.group("sha256"),
            )

    required = set(required_lane_authorities)
    if set(lane_markers) != required:
        missing = sorted(required - set(lane_markers))
        unexpected = sorted(set(lane_markers) - required)
        module_failures.append(
            f"lane authority marker set drifted; missing={missing}, "
            f"unexpected={unexpected}"
        )

    for (domain, relative, function), (declared, digest) in lane_markers.items():
        try:
            authority_path = (repo_root / relative).resolve()
            try:
                authority_path.relative_to(repo_root)
            except ValueError as error:
                raise VerificationError(
                    f"d{domain}: lane authority escapes repository: {relative}"
                ) from error
            authority_source = _read_utf8(authority_path)
            authority_spans = function_spans(authority_source)
            actual = authority_spans.get(function)
            if actual is None:
                raise VerificationError(
                    f"d{domain}: {relative} has no function {function}"
                )
            if declared != actual:
                raise VerificationError(
                    f"d{domain}: lane authority {function} declared "
                    f"{declared.start}-{declared.end}, actual "
                    f"{actual.start}-{actual.end}"
                )
            actual_hash = normalized_span_hash(authority_source, actual)
            if digest != actual_hash:
                raise VerificationError(
                    f"d{domain}: lane authority digest changed for {relative}::{function}"
                )
        except VerificationError as error:
            module_failures.append(str(error))

    if module_failures:
        raise VerificationError(
            f"{len(module_failures)} citation authority failure(s): "
            + "; ".join(module_failures)
        )

    grandfathered = verify_prose_references(repo_root, modules, world_markers)
    documented = verify_document_references(
        repo_root, load_prose_allowance(repo_root)
    )

    return len(modules), len(lane_markers), grandfathered, documented


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--repo-root",
        type=Path,
        default=Path(__file__).resolve().parents[3],
    )
    args = parser.parse_args(argv)
    try:
        module_count, lane_count, grandfathered, documented = verify_repo(
            args.repo_root
        )
        # The inventory is only trustworthy to report from BECAUSE verify_repo
        # just proved it equals the computed closure, domain by domain.
        covered = {
            ident
            for members in load_closure_inventory(args.repo_root.resolve()).values()
            for ident in members
        }
    except VerificationError as error:
        print(f"world-fold citation authority check: FAIL: {error}", file=sys.stderr)
        return 1
    print(
        "world-fold citation authority check: PASS "
        f"({module_count} dNN modules, {lane_count} lane authorities, "
        f"{len(covered)} world_fold.rs items inside an authority closure, "
        f"{grandfathered} grandfathered prose line references, "
        f"{documented} grandfathered document line references)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
