#!/usr/bin/env python3
"""Shared helpers for deterministic static-definition artifacts.

This module intentionally stays in ``extractor/game_defs`` so lane-local gates
can share canonical JSON, source inventory, and Clausewitz node conversion
without importing native recorder code.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any

from paradox_parser import Node, tokenize


REPO_ROOT = Path(__file__).resolve().parents[2]
_OPERATOR_TOKENS = {">", "<", "!", "=", ">=", "<=", "!=", "=="}


CLAUSEWITZ_SYNTAX_QUIRK_ALLOWLIST = {
    "common/ideas/persia.txt": {
        "error_contains": "unbalanced braces at EOF (depth 1)",
        "reason": (
            "Installed vanilla the game common/ideas/persia.txt is missing the final "
            "top-level ideas brace; the tolerant parser still recovers the "
            "complete ideas block before EOF."
        ),
    },
    "common/ideas/switzerland.txt": {
        "error_contains": "unbalanced braces at EOF (depth 1)",
        "reason": (
            "Installed vanilla the game common/ideas/switzerland.txt is missing the "
            "final top-level ideas brace; the tolerant parser still recovers the "
            "complete ideas block before EOF."
        ),
    },
    "interface/powerbalanceview.gfx": {
        "error_contains": "unbalanced braces at EOF (depth 1)",
        "reason": (
            "Installed vanilla the game interface/powerbalanceview.gfx is missing "
            "the final top-level spriteTypes brace; the tolerant parser still "
            "recovers sprite definitions before EOF."
        ),
    },
}


def canonical_json(data: Any) -> bytes:
    return json.dumps(
        data,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")


def pretty_json(data: Any) -> bytes:
    return json.dumps(data, ensure_ascii=False, sort_keys=True, indent=2).encode("utf-8")


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def scalar(value: Any) -> str | None:
    return value if isinstance(value, str) else None


def boolish(value: Any) -> bool | None:
    if not isinstance(value, str):
        return None
    low = value.lower()
    if low in {"yes", "true"}:
        return True
    if low in {"no", "false"}:
        return False
    return None


def _syntax_allowlist_hit(rel: str, error: ValueError, allowlist: dict[str, dict[str, str]]) -> dict[str, str] | None:
    entry = allowlist.get(rel)
    if not entry:
        return None
    message = str(error)
    expected = entry.get("error_contains")
    # Exact-suffix match, NOT substring: the error messages end with the
    # documented quirk (e.g. "(depth 1)"), and a substring test would let a
    # more-broken file matching "(depth 1)" as a prefix of "(depth 1X)" slip
    # through. endswith pins the exact depth. The downstream wrapper-presence
    # gate (idea/tech/decisions/GFX all require their top-level block) is the
    # completeness backstop if a file loses real content while still ending at
    # the same brace depth.
    if expected and not message.endswith(expected):
        return None
    return {
        "source_file": rel,
        "matched_error": message,
        "reason": entry.get("reason") or "documented Clausewitz syntax quirk",
    }


def _unclosed_top_level_wrapper_key(text: str) -> str | None:
    """Return the key for ``key = { ... EOF`` when only that root brace is open.

    Some shipped the game definition files omit the final closing brace for their
    document wrapper while all nested blocks inside the wrapper are complete.
    The tolerant parser can recover those files exactly; this precheck should
    only reject cases where a nested block is still open or a closing brace is
    unmatched.
    """

    stack: list[int] = []
    tokens = tokenize(text)
    for idx, (kind, _value) in enumerate(tokens):
        if kind == "lbrace":
            stack.append(idx)
            continue
        if kind == "rbrace":
            if not stack:
                return None
            stack.pop()
    if len(stack) != 1:
        return None
    lbrace_idx = stack[0]
    if lbrace_idx < 2 or tokens[lbrace_idx - 1][0] != "eq":
        return None
    key_kind, key_value = tokens[lbrace_idx - 2]
    if key_kind in {"str", "bare"} and key_value not in _OPERATOR_TOKENS:
        return key_value
    return None


def _missing_final_wrapper_hit(rel: str, error: ValueError, text: str) -> dict[str, str] | None:
    key = _unclosed_top_level_wrapper_key(text)
    if not key:
        return None
    return {
        "source_file": rel,
        "matched_error": str(error),
        "reason": (
            f"Missing final top-level {key} brace; nested braces are balanced "
            "and the tolerant parser recovers the complete wrapper before EOF."
        ),
    }


def validate_clausewitz_text(
    text: str,
    rel: str,
    *,
    allowlist: dict[str, dict[str, str]] | None = None,
) -> dict[str, str] | None:
    """Fail-closed precheck for the repo's tolerant Clausewitz parser.

    ``paradox_parser.parse`` is documented as permissive and does not expose an
    unconsumed-token cursor to callers, so static ground-truth parsers do this
    quote/comment-aware balanced-brace validation before parsing.
    """

    depth = 0
    line = 1
    col = 0
    in_quote = False
    escaped = False
    in_comment = False
    quote_line = 0
    quote_col = 0
    for ch in text:
        if ch == "\n":
            line += 1
            col = 0
            in_comment = False
            if in_quote:
                # the game quoted strings can contain escaped newlines as text, but
                # a literal newline before a closing quote is a broken file for
                # these static definition inputs.
                raise ValueError(f"{rel}: unterminated quote starting at {quote_line}:{quote_col}")
            continue
        col += 1
        if in_comment:
            continue
        if in_quote:
            if escaped:
                escaped = False
                continue
            if ch == "\\":
                escaped = True
                continue
            if ch == '"':
                in_quote = False
            continue
        if ch == "#":
            in_comment = True
            continue
        if ch == '"':
            in_quote = True
            quote_line = line
            quote_col = col
            continue
        if ch == "{":
            depth += 1
            continue
        if ch == "}":
            depth -= 1
            if depth < 0:
                raise ValueError(f"{rel}: unmatched closing brace at {line}:{col}")
    try:
        if in_quote:
            raise ValueError(f"{rel}: unterminated quote starting at {quote_line}:{quote_col}")
        if depth:
            raise ValueError(f"{rel}: unbalanced braces at EOF (depth {depth})")
    except ValueError as exc:
        if allowlist:
            hit = _syntax_allowlist_hit(rel, exc, allowlist)
            if hit:
                return hit
        hit = _missing_final_wrapper_hit(rel, exc, text)
        if hit:
            return hit
        raise
    return None


def atoms(node: Any) -> list[str]:
    if not isinstance(node, Node):
        return []
    return [value for key, value in node.entries if key == "" and isinstance(value, str)]


def node_to_data(value: Any) -> Any:
    """Convert a parsed Clausewitz node to deterministic JSON-safe data.

    Repeated keys are preserved as lists, and anonymous list values are stored
    under ``_values``. This avoids losing repeated blocks like
    ``can_convert_from`` or ``module_count_limit``.
    """

    if not isinstance(value, Node):
        return value
    out: dict[str, Any] = {}
    anonymous: list[Any] = []
    for key, child in value.entries:
        converted = node_to_data(child)
        if key == "":
            anonymous.append(converted)
            continue
        if key in out:
            if not isinstance(out[key], list):
                out[key] = [out[key]]
            out[key].append(converted)
        else:
            out[key] = converted
    if anonymous:
        out["_values"] = anonymous
    return out


def first_scalar(node: Node, field: str) -> str | None:
    for value in node.get_all(field):
        if isinstance(value, str):
            return value
    return None


def field_values(node: Node, field: str) -> list[Any]:
    return [node_to_data(value) for value in node.get_all(field)]


def single_or_many(values: list[Any]) -> Any:
    if not values:
        return None
    if len(values) == 1:
        return values[0]
    return values


def put_present(row: dict[str, Any], key: str, value: Any) -> None:
    if value is not None:
        row[key] = value


def direct_files(ordered: list[tuple[str, Path]], subdir: str) -> list[tuple[str, Path]]:
    wanted = Path(subdir).as_posix()
    return [
        (rel, path)
        for rel, path in ordered
        if Path(rel).parent.as_posix() == wanted
    ]


def source_inventory(files: list[tuple[str, Path]]) -> list[dict[str, Any]]:
    return [
        {
            "relpath": rel,
            "bytes": path.stat().st_size,
            "sha256": sha256_bytes(path.read_bytes()),
        }
        for rel, path in sorted(files, key=lambda item: item[0])
    ]


def add_layer_args(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--vanilla", default=None, help="the game install dir.")
    parser.add_argument(
        "--mods",
        default=None,
        help="Comma-separated .mod descriptor paths/tokens. Defaults to active_modset.json.",
    )
    parser.add_argument(
        "--userdata",
        default=None,
        help="the game user-data dir used to resolve relative mod descriptors.",
    )


def mods_from_arg(raw: str | None) -> list[str] | None:
    if not raw:
        return None
    return [part.strip() for part in raw.split(",") if part.strip()]


def layer_summary(layers: list[Any]) -> list[dict[str, Any]]:
    return [
        {
            "label": layer.label,
            "root_name": Path(layer.root).name,
            "replace_path": list(getattr(layer, "replace_paths", []) or []),
        }
        for layer in layers
    ]


def repo_relative(path: Path) -> str:
    resolved = Path(path).resolve()
    try:
        return resolved.relative_to(REPO_ROOT.resolve()).as_posix()
    except ValueError:
        return resolved.name


def write_static_store(
    out_dir: Path,
    *,
    game_id: str,
    catalog_name: str,
    catalog: dict[str, Any],
    snapshot_schema: str,
    index_schema: str,
    manifest_role: str,
    payload_role: str,
    snapshot_extra: dict[str, Any] | None = None,
) -> dict[str, Path]:
    game_dir = out_dir / game_id
    game_dir.mkdir(parents=True, exist_ok=True)

    catalog_path = game_dir / catalog_name
    catalog_bytes = pretty_json(catalog)
    catalog_path.write_bytes(catalog_bytes)

    snapshot = {
        "schema_version": snapshot_schema,
        "catalog_file": catalog_path.name,
        "catalog_sha256": sha256_bytes(catalog_bytes),
    }
    if snapshot_extra:
        snapshot.update(snapshot_extra)
    snapshot_path = game_dir / "snapshot_000000.json"
    snapshot_bytes = pretty_json(snapshot)
    snapshot_path.write_bytes(snapshot_bytes)

    index = {
        "schema": index_schema,
        "game_id": game_id,
        "snapshots": [
            {
                "hour_index": 0,
                "file": snapshot_path.name,
                "role": manifest_role,
                "sha256": sha256_bytes(snapshot_bytes),
            },
            {
                "hour_index": 0,
                "file": catalog_path.name,
                "role": payload_role,
                "sha256": sha256_bytes(catalog_bytes),
            },
        ],
    }
    index_path = game_dir / "index.json"
    index_path.write_bytes(pretty_json(index))
    return {"game_dir": game_dir, "catalog": catalog_path, "snapshot": snapshot_path, "index": index_path}


def verify_store(paths: dict[str, Path]) -> dict[str, Any]:
    index = json.loads(paths["index"].read_text(encoding="utf-8"))
    game_dir = paths["game_dir"]
    checks = []
    for entry in index["snapshots"]:
        path = game_dir / entry["file"]
        actual = sha256_bytes(path.read_bytes())
        checks.append(
            {
                "file": entry["file"],
                "role": entry["role"],
                "expected_sha256": entry["sha256"],
                "actual_sha256": actual,
                "match": actual == entry["sha256"],
            }
        )
    return {
        "index_sha256": sha256_bytes(paths["index"].read_bytes()),
        "checks": checks,
        "passed": all(check["match"] for check in checks),
    }


def load_localisation_catalog(path: Path) -> dict[str, Any]:
    data = json.loads(path.read_text(encoding="utf-8"))
    keys = data.get("keys")
    if not isinstance(keys, dict):
        raise ValueError(f"localisation catalog missing keys object: {path}")
    return data


def localisation_name(keys: dict[str, Any], loc_key: str | None) -> str | None:
    if not loc_key:
        return None
    entry = keys.get(loc_key)
    if not isinstance(entry, dict):
        return None
    value = entry.get("name")
    return value if isinstance(value, str) else None


def simple_localisation_coverage(
    category: str,
    items: list[dict[str, Any]],
    loc_keys: dict[str, Any],
) -> dict[str, Any]:
    unresolved = []
    resolved = 0
    literal = 0
    for item in sorted(items, key=lambda row: row["id"]):
        loc_key = item.get("loc_key")
        literal_name = item.get("literal_name")
        if loc_key and loc_key in loc_keys:
            resolved += 1
            continue
        if literal_name:
            resolved += 1
            literal += 1
            continue
        unresolved.append(
            {
                "id": item["id"],
                "loc_key": loc_key,
                "reason": item.get("reason") or (
                    "no_derivable_loc_key" if not loc_key else "missing_localisation_key"
                ),
                "source_file": item.get("source_file"),
                "source_layer": item.get("source_layer"),
            }
        )
    total = len(items)
    out: dict[str, Any] = {
        "category": category,
        "total_seen": total,
        "resolved": resolved,
        "resolved_by_literal": literal,
        "unresolved": len(unresolved),
        "loc_resolvable_percent": round((resolved * 100.0 / total), 2) if total else 100.0,
        "missing_localisation_key": sum(1 for row in unresolved if row.get("loc_key")),
        "no_derivable_loc_key": sum(1 for row in unresolved if not row.get("loc_key")),
    }
    out["unresolvable_details"] = unresolved
    return out
