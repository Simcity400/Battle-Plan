#!/usr/bin/env python3
"""Read-only structural audit for the repository's Rust tests.

This does not decide that a test is useless. It identifies mechanically reviewable
candidates: exact body duplicates, obvious tautologies, external-resource tests
that can return success without exercising their assertions, and sleep/large-loop
tests that should not silently enter the fast tier.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import re
from collections import defaultdict


REPO = pathlib.Path(__file__).resolve().parents[3]
ROOTS = (
    REPO / "extractor" / "native",
    REPO / "replay_interface" / "native",
    REPO / "portable_replay" / "native",
)

TEST_HEADER = re.compile(
    r"#\s*\[\s*(?:tokio::)?test(?:\s*\([^]]*\))?\s*\]"
    r"(?P<attrs>(?:\s*#\s*\[[^]]+\])*)\s*"
    r"(?:async\s+)?fn\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)"
    r"\s*\([^)]*\)[^{]*\{",
    re.S,
)
EXTERNAL_HINT = re.compile(
    r"env::var|var_os|\.exists\(|\.is_dir\(|vanilla_dir\(|"
    r"SHADER_CAPTURE_DIR|HOI4_[A-Z0-9_]+'?",
)
SLOW_HINT = re.compile(
    r"thread::sleep|tokio::time::sleep|Duration::from_secs|"
    r"BATTLE_PLAN_SOAK_|\b(?:20_000|26_280|2000)\b",
)
TAUTOLOGY = re.compile(
    r"assert!\s*\(\s*true\s*\)|"
    r"assert_eq!\s*\(\s*([A-Za-z_][A-Za-z0-9_.]*)\s*,\s*\1\s*\)",
)
TEST_ANNOTATION = re.compile(r"#\s*\[\s*(?:tokio::)?test")
IGNORE_ANNOTATION = re.compile(r"#\s*\[\s*ignore(?:\s*=|\s*\])")


def closing_brace(text: str, start: int) -> int | None:
    depth = 0
    index = start
    state = "code"
    block_depth = 0
    while index < len(text):
        char = text[index]
        following = text[index + 1] if index + 1 < len(text) else ""
        if state == "code":
            if char == "/" and following == "/":
                state = "line-comment"
                index += 2
                continue
            if char == "/" and following == "*":
                state = "block-comment"
                block_depth = 1
                index += 2
                continue
            if char == '"':
                state = "string"
                index += 1
                continue
            if char == "'":
                # Rust lifetimes such as `'static` are not character literals.
                looks_like_char = following == "\\" or (
                    index + 2 < len(text) and text[index + 2] == "'"
                )
                if looks_like_char:
                    state = "char"
                index += 1
                continue
            if char == "{":
                depth += 1
            elif char == "}":
                depth -= 1
                if depth == 0:
                    return index
            index += 1
            continue
        if state == "line-comment":
            if char == "\n":
                state = "code"
            index += 1
            continue
        if state == "block-comment":
            if char == "/" and following == "*":
                block_depth += 1
                index += 2
                continue
            if char == "*" and following == "/":
                block_depth -= 1
                index += 2
                if block_depth == 0:
                    state = "code"
                continue
            index += 1
            continue
        if state in {"string", "char"}:
            if char == "\\":
                index += 2
                continue
            if (state == "string" and char == '"') or (state == "char" and char == "'"):
                state = "code"
            index += 1
    return None


def normalized_body(body: str) -> str:
    body = re.sub(r"//[^\n]*|/\*.*?\*/", "", body, flags=re.S)
    return re.sub(r"\s+", " ", body).strip()


def relative(path: pathlib.Path) -> str:
    return path.relative_to(REPO).as_posix()


def collect() -> list[dict[str, object]]:
    tests: list[dict[str, object]] = []
    for root in ROOTS:
        for path in root.rglob("*.rs"):
            try:
                text = path.read_text(encoding="utf-8")
            except (OSError, UnicodeError):
                continue
            for match in TEST_HEADER.finditer(text):
                end = closing_brace(text, match.end() - 1)
                if end is None:
                    continue
                body = text[match.end() : end]
                tests.append(
                    {
                        "path": relative(path),
                        "line": text.count("\n", 0, match.start()) + 1,
                        "name": match.group("name"),
                        "ignored": "ignore" in (match.group("attrs") or ""),
                        "body": body,
                        "normalized": normalized_body(body),
                    }
                )
    return tests


def reference(test: dict[str, object]) -> str:
    return f"{test['path']}:{test['line']}:{test['name']}"


def build_report(tests: list[dict[str, object]]) -> dict[str, object]:
    annotation_counts: dict[str, int] = {}
    ignored_annotations = 0
    for root in ROOTS:
        test_count = 0
        for path in root.rglob("*.rs"):
            try:
                text = path.read_text(encoding="utf-8")
            except (OSError, UnicodeError):
                continue
            test_count += len(TEST_ANNOTATION.findall(text))
            ignored_annotations += len(IGNORE_ANNOTATION.findall(text))
        annotation_counts[relative(root)] = test_count
    duplicates: dict[str, list[dict[str, object]]] = defaultdict(list)
    for test in tests:
        normalized = str(test["normalized"])
        if len(normalized) >= 80:
            duplicates[hashlib.sha256(normalized.encode("utf-8")).hexdigest()].append(test)

    duplicate_groups = [
        [reference(test) for test in group]
        for group in duplicates.values()
        if len(group) > 1
    ]
    tautologies = [reference(test) for test in tests if TAUTOLOGY.search(str(test["body"]))]
    resource_returns = [
        reference(test)
        for test in tests
        if not bool(test["ignored"])
        and "return;" in str(test["body"])
        and EXTERNAL_HINT.search(str(test["body"]))
    ]
    slow_default_candidates = [
        reference(test)
        for test in tests
        if not bool(test["ignored"]) and SLOW_HINT.search(str(test["body"]))
    ]
    return {
        "schema": "hoi4-rust-test-structural-audit-v1",
        "test_annotations": sum(annotation_counts.values()),
        "test_annotations_by_root": annotation_counts,
        "parsed_tests": len(tests),
        "ignored_annotations": ignored_annotations,
        "duplicate_body_groups": sorted(duplicate_groups),
        "obvious_tautologies": sorted(tautologies),
        "external_resource_early_return_candidates": sorted(resource_returns),
        "slow_default_candidates": sorted(slow_default_candidates),
        "limitations": [
            "structural candidates require human semantic review before deletion",
            "helper functions can contain assertions not visible in a test body",
            "macro-generated tests are outside this source-function parser",
        ],
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--json", action="store_true", help="emit the complete JSON report")
    args = parser.parse_args()
    report = build_report(collect())
    if args.json:
        print(json.dumps(report, indent=2, sort_keys=True))
    else:
        print(f"test annotations: {report['test_annotations']}")
        print(f"parsed test functions: {report['parsed_tests']}")
        print(f"ignored annotations: {report['ignored_annotations']}")
        print(f"exact duplicate-body groups: {len(report['duplicate_body_groups'])}")
        print(f"obvious tautologies: {len(report['obvious_tautologies'])}")
        print(
            "external-resource early-return candidates: "
            f"{len(report['external_resource_early_return_candidates'])}"
        )
        print(f"slow default candidates: {len(report['slow_default_candidates'])}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
