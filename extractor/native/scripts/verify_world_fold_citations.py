#!/usr/bin/env python3
"""Fail closed when wire-format payload-authority citations drift.

The dNN wire modules transcribe the shipping fold rather than calling it.  Their
top-level authority marker therefore pins three things to the source it cites:

* the domain-to-fold dispatch in ``WorldFold::fold_domain_data``;
* the exact lexical function envelope; and
* a SHA-256 of the cited source bytes after newline normalization.

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
import hashlib
import json
import re
import sys
from collections import Counter
from dataclasses import dataclass
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
PROSE_REFERENCE_RE = re.compile(r"world_fold\.rs:(\d+)")

WORLD_MARKER_RE = re.compile(
    r"^// @world-fold-authority "
    r"domain=(?P<domain>\d+) "
    r"functions=(?P<functions>fold_[A-Za-z0-9_]+(?:,fold_[A-Za-z0-9_]+)*) "
    r"lines=(?P<start>\d+)-(?P<end>\d+) "
    r"sha256=(?P<sha256>[0-9a-f]{64})$",
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


def sanitize_rust(source: str) -> str:
    """Blank comments and literals while preserving byte count and newlines.

    This is deliberately a small lexical scanner, not a Rust parser.  It knows
    nested block comments, escaped ordinary/byte strings, raw/byte-raw strings,
    and character literals.  That is sufficient to make braces in diagnostics,
    JSON snippets, and comments invisible to the function-envelope walk.
    """

    out: list[str] = []
    index = 0
    length = len(source)
    while index < length:
        if source.startswith("//", index):
            end = source.find("\n", index)
            if end < 0:
                end = length
            out.append(_blank_except_newlines(source[index:end]))
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
            out.append(_blank_except_newlines(source[start:index]))
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


def normalized_span_hash(source: str, span: Span) -> str:
    lines = source.splitlines()
    if span.start < 1 or span.end < span.start or span.end > len(lines):
        raise VerificationError(
            f"invalid source span {span.start}-{span.end} for {len(lines)} lines"
        )
    normalized = "\n".join(lines[span.start - 1 : span.end]) + "\n"
    return hashlib.sha256(normalized.encode("utf-8")).hexdigest()


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
        return json.loads(allowance_path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        raise VerificationError(
            f"cannot read the prose-reference allowance {allowance_path}: {error}"
        ) from error


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
            expected_hash = normalized_span_hash(world_source, expected)
            if marker.group("sha256") != expected_hash:
                raise VerificationError(
                    f"{module.name}: authority digest changed for "
                    f"{expected.start}-{expected.end}"
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
    except VerificationError as error:
        print(f"world-fold citation authority check: FAIL: {error}", file=sys.stderr)
        return 1
    print(
        "world-fold citation authority check: PASS "
        f"({module_count} dNN modules, {lane_count} lane authorities, "
        f"{grandfathered} grandfathered prose line references, "
        f"{documented} grandfathered document line references)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
