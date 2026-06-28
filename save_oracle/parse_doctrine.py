#!/usr/bin/env python3
"""the game save -> doctrine manager state (``doctrine.json``).

The 1.19 doctrine tree's static catalog lives in ``game_defs/out/doctrines.json``;
that file tells us which tokens are doctrine nodes and how they render. This
parser reads the save's top-level ``doctrine={ countries={...} }`` manager and
preserves the observed per-country state tree without inventing researched or
unresearched status from the static catalog alone.

Because the checked-in fixture set does not include a real melted doctrine block,
the parser is deliberately conservative:

* TAG-keyed country entries are exposed under ``countries``.
* non-TAG country keys are preserved under ``countries_by_key`` and are not folded
  onto country snapshots without a proven TAG bridge.
* ``completed`` and ``in_progress`` summaries are derived only from explicit
  save containers such as ``completed`` / ``researched`` / ``in_progress`` and
  only for tokens present in the static doctrine catalog when that catalog exists.
* the raw per-country doctrine tree is always kept, so unknown newer save shapes
  remain truthful data rather than a lossy guess.
"""

from __future__ import annotations

import json
import re
from pathlib import Path
from typing import Any, Dict, Iterable, List, Set, Tuple

import deep_common as dc
from deep_common import (
    LBRACE, RBRACE, EQUALS, atom_value, to_float, to_int, run_cli, top_level_pass,
)


TAG_RE = re.compile(r"^[A-Z][A-Z0-9]{2}$")

COMPLETED_HINTS = {
    "completed", "completed_doctrines", "unlocked", "unlocked_doctrines",
    "researched", "researched_doctrines",
}
IN_PROGRESS_HINTS = {
    "in_progress", "in_progress_doctrine", "in_progress_doctrines",
    "research", "researching", "current_research",
}


def _add_repeated(out: Dict[str, Any], key: str, value: Any) -> None:
    """Insert key preserving repeated Clausewitz keys as a list."""
    if key not in out:
        out[key] = value
        return
    cur = out[key]
    if isinstance(cur, list):
        cur.append(value)
    else:
        out[key] = [cur, value]


def _read_block_tree(tz) -> Any:
    """Read a block into a JSON-compatible dict/list, preserving repeated keys.

    This uses the raw tokenizer instead of iter_block_entries so anonymous list
    blocks are not skipped. A block containing only bare atoms becomes a list;
    a mixed block becomes a dict with ``_list`` for bare atoms and ``_blocks`` for
    anonymous child blocks.
    """
    out: Dict[str, Any] = {}
    atoms: List[Any] = []
    blocks: List[Any] = []
    while True:
        tok = tz.next()
        if tok is None or tok is RBRACE:
            break
        if tok is LBRACE:
            blocks.append(_read_block_tree(tz))
            continue
        key = atom_value(tok)
        if key is None:
            continue
        nxt = tz.peek()
        if nxt is EQUALS:
            tz.next()
            val = tz.next()
            if val is LBRACE:
                _add_repeated(out, key, _read_block_tree(tz))
            elif val is RBRACE:
                break
            else:
                _add_repeated(out, key, atom_value(val))
        else:
            atoms.append(key)

    if not out:
        if blocks:
            return blocks if not atoms else {"_list": atoms, "_blocks": blocks}
        return atoms
    if atoms:
        out["_list"] = atoms
    if blocks:
        out["_blocks"] = blocks
    return out


def _valid_tag(value: Any) -> bool:
    return isinstance(value, str) and TAG_RE.match(value) is not None


def _load_known_doctrines() -> Tuple[Set[str], bool]:
    """Return static doctrine ids if game_defs/out/doctrines.json is available."""
    p = Path(__file__).resolve().parents[1] / "game_defs" / "out" / "doctrines.json"
    try:
        data = json.loads(p.read_text(encoding="utf-8"))
        docs = data.get("doctrines")
        if isinstance(docs, dict):
            return set(docs.keys()), True
    except Exception:
        pass
    return set(), False


def _walk(node: Any, path: Tuple[str, ...] = ()):
    if isinstance(node, dict):
        for key, val in node.items():
            yield path, key, val
            yield from _walk(val, path + (key,))
    elif isinstance(node, list):
        for item in node:
            yield from _walk(item, path)


def _path_has_hint(path: Iterable[str], hints: Set[str]) -> bool:
    return any(str(p).lower() in hints for p in path)


def _record_from_value(token: str, value: Any) -> Dict[str, Any]:
    rec = {"id": token, "level": None, "date": None, "research_points": None}
    if isinstance(value, dict):
        if value.get("level") is not None:
            rec["level"] = to_int(value.get("level"))
        if value.get("date") is not None:
            rec["date"] = value.get("date")
        if value.get("research_points") is not None:
            rec["research_points"] = to_float(value.get("research_points"))
    elif str(value).lower() in ("yes", "true", "1"):
        rec["level"] = 1
    return rec


def _progress_record_from_value(token: str, value: Any) -> Dict[str, Any]:
    rec = {"id": token, "progress": None}
    if isinstance(value, dict):
        for key in ("progress", "progress_units", "research_points"):
            if value.get(key) is not None:
                rec["progress"] = to_float(value.get(key))
                break
    else:
        rec["progress"] = to_float(value)
    return rec


def _derive_doctrine_lists(raw: Any, known: Set[str]) -> Tuple[List[Dict[str, Any]], List[Dict[str, Any]]]:
    """Derive explicit completed/in-progress summaries, or [] when unprovable."""
    if not known:
        return [], []
    completed: Dict[str, Dict[str, Any]] = {}
    in_progress: Dict[str, Dict[str, Any]] = {}

    for path, key, val in _walk(raw):
        key_l = str(key).lower()
        completed_here = _path_has_hint(path, COMPLETED_HINTS) or key_l in COMPLETED_HINTS
        progress_here = _path_has_hint(path, IN_PROGRESS_HINTS) or key_l in IN_PROGRESS_HINTS

        if key in known and completed_here:
            completed.setdefault(key, _record_from_value(key, val))
        if key in known and progress_here:
            in_progress.setdefault(key, _progress_record_from_value(key, val))

        if isinstance(val, str) and val in known:
            if completed_here:
                completed.setdefault(val, _record_from_value(val, "yes"))
            if progress_here:
                rec = in_progress.setdefault(val, {"id": val, "progress": None})
                if isinstance(raw, dict):
                    prog = raw.get("progress") or raw.get("progress_units")
                    rec["progress"] = to_float(prog) if prog is not None else rec["progress"]

        if isinstance(val, list):
            for item in val:
                if isinstance(item, str) and item in known:
                    if completed_here:
                        completed.setdefault(item, _record_from_value(item, "yes"))
                    if progress_here:
                        in_progress.setdefault(item, {"id": item, "progress": None})

    return (
        sorted(completed.values(), key=lambda r: r["id"]),
        sorted(in_progress.values(), key=lambda r: r["id"]),
    )


def _parse_doctrine_countries(tz, countries: Dict[str, Any],
                              countries_by_key: Dict[str, Any],
                              known: Set[str]) -> None:
    tree = _read_block_tree(tz)
    if not isinstance(tree, dict):
        return
    for key, raw in tree.items():
        if key.startswith("_"):
            continue
        completed, in_progress = _derive_doctrine_lists(raw, known)
        rec = {
            "raw": raw,
            "completed": completed,
            "completed_count": len(completed),
            "in_progress": in_progress,
        }
        if _valid_tag(key):
            countries[key] = rec
        else:
            countries_by_key[str(key)] = rec


def _parse_doctrine_root(tz, countries: Dict[str, Any],
                         countries_by_key: Dict[str, Any],
                         known: Set[str]) -> None:
    while True:
        tok = tz.next()
        if tok is None or tok is RBRACE:
            return
        if tok is LBRACE:
            _read_block_tree(tz)
            continue
        key = atom_value(tok)
        if key is None:
            continue
        if tz.peek() is EQUALS:
            tz.next()
            val = tz.next()
            if val is LBRACE:
                if key == "countries":
                    _parse_doctrine_countries(tz, countries, countries_by_key, known)
                else:
                    _read_block_tree(tz)
            elif val is RBRACE:
                return


def parse_doctrine_text(text: str):
    countries: Dict[str, Any] = {}
    countries_by_key: Dict[str, Any] = {}
    known, static_loaded = _load_known_doctrines()
    meta = top_level_pass(
        text,
        {"doctrine": lambda tz: _parse_doctrine_root(
            tz, countries, countries_by_key, known)},
        scalar_keys=("date", "version", "player"),
    )
    return countries, countries_by_key, meta, static_loaded, len(known)


def build(text, source, was_melted):
    countries, countries_by_key, meta, static_loaded, known_count = parse_doctrine_text(text)
    total_completed = sum((c.get("completed_count") or 0) for c in countries.values())
    total_in_progress = sum(len(c.get("in_progress") or []) for c in countries.values())
    out = {
        "meta": {
            "source_filename": source.name,
            "source_path_redacted": dc.ps._redact(source),
            "was_melted": was_melted,
            "save_date": meta.get("date"),
            "save_version": meta.get("version"),
            "player": meta.get("player"),
            "static_doctrine_catalog_loaded": static_loaded,
            "static_doctrine_count": known_count,
            "notes": {
                "fail_closed": "raw doctrine.countries tree is preserved; completed/in_progress "
                "summaries are derived only from explicit save containers and known static doctrine ids.",
                "country_keys": "TAG-keyed entries fold to countries; non-TAG keys stay in countries_by_key "
                "until a TAG bridge is proven.",
            },
            "stats": {
                "tag_countries_with_doctrine": len(countries),
                "non_tag_country_keys": len(countries_by_key),
                "derived_completed_doctrines": total_completed,
                "derived_in_progress_doctrines": total_in_progress,
            },
        },
        "countries": dict(sorted(countries.items())),
        "countries_by_key": dict(sorted(countries_by_key.items())),
    }

    lines = ["=== DOCTRINE ORACLE VALIDATION ==="]
    lines.append(f"source             : {source.name} (melted={was_melted})")
    lines.append(f"save date          : {meta.get('date')}")
    lines.append(f"static catalog     : {'loaded' if static_loaded else 'missing'} ({known_count} ids)")
    lines.append(f"TAG country rows   : {len(countries)}")
    lines.append(f"non-TAG country rows: {len(countries_by_key)}")
    lines.append(f"derived completed  : {total_completed}")
    lines.append(f"derived in-progress: {total_in_progress}")
    return {"doctrine.json": out}, "\n".join(lines)


if __name__ == "__main__":
    raise SystemExit(run_cli(
        "doctrine", "the game save -> doctrine manager oracle.",
        build, ["doctrine.json"]))
