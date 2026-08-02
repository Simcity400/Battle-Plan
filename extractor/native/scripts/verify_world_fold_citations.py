#!/usr/bin/env python3
"""Fail closed when wire-format payload-authority citations drift.

The dNN wire modules transcribe the shipping fold rather than calling it.  Their
top-level authority marker therefore pins three things to the source it cites:

* the domain-to-fold dispatch in ``WorldFold::fold_domain_data``;
* the exact lexical function envelope; and
* a SHA-256 of the cited source bytes after newline normalization.

The two ports which also transcribe a capture-lane payload decoder (d51 and d52)
carry a second marker.  That marker hashes the original decoder, not the
transcription, so a changed production authority cannot be approved by tests of
the stale copy.
"""

from __future__ import annotations

import argparse
import hashlib
import re
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable


WORLD_FOLD = Path("extractor/native/recorder_host/src/world_fold.rs")
WIRE_SRC = Path("extractor/native/wire_format/src")

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


def verify_repo(
    repo_root: Path,
    *,
    required_lane_authorities: Iterable[tuple[int, str, str]] = (
        REQUIRED_LANE_AUTHORITIES
    ),
) -> tuple[int, int]:
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
    module_failures: list[str] = []
    for module in modules:
        domain = int(module.stem[1:])
        module_source = _read_utf8(module)
        try:
            marker = _only_match(
                WORLD_MARKER_RE, module_source, module.relative_to(repo_root).as_posix()
            )
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
            human_citation = (
                f"`world_fold.rs:{expected.start}`-`:{expected.end}`"
            )
            if human_citation not in module_source:
                raise VerificationError(
                    f"{module.name}: human authority citation is not {human_citation}"
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

    return len(modules), len(lane_markers)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--repo-root",
        type=Path,
        default=Path(__file__).resolve().parents[3],
    )
    args = parser.parse_args(argv)
    try:
        module_count, lane_count = verify_repo(args.repo_root)
    except VerificationError as error:
        print(f"world-fold citation authority check: FAIL: {error}", file=sys.stderr)
        return 1
    print(
        "world-fold citation authority check: PASS "
        f"({module_count} dNN modules, {lane_count} lane authorities)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
