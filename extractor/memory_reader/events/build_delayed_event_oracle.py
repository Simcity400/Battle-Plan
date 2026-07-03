#!/usr/bin/env python3
"""Build a save-side oracle for delayed event queues.

This parser is fixture tooling only. It reads a melted/plaintext save or asks
rakaly to melt a binary save via the existing save_oracle helpers, then extracts
country delayed-event blocks and top-level pending_events blocks without
claiming any live root.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parents[3]
SAVE_ORACLE = REPO_ROOT / "extractor" / "save_oracle"
if str(SAVE_ORACLE) not in sys.path:
    sys.path.insert(0, str(SAVE_ORACLE))

import deep_common as dc  # type: ignore
import parse_save as ps  # type: ignore


MAX_BLOCK_NODES = 20000
INTERESTING_KEYS = {"event", "id", "name", "days", "hours", "months", "originator", "scope", "random"}


def atom(opener: Any) -> str | None:
    return dc.atom_value(opener)


def coerce_scalar(value: str | None) -> Any:
    if value is None:
        return None
    if value in ("yes", "no"):
        return value == "yes"
    try:
        if "." not in value:
            return int(value, 10)
    except ValueError:
        pass
    try:
        return float(value)
    except ValueError:
        return value


def merge_key(target: dict[str, Any], key: str, value: Any) -> None:
    if key not in target:
        target[key] = value
    elif isinstance(target[key], list):
        target[key].append(value)
    else:
        target[key] = [target[key], value]


def collect_block(tz: dc.Tokenizer, depth: int = 0, budget: list[int] | None = None) -> Any:
    if budget is None:
        budget = [MAX_BLOCK_NODES]
    out: dict[str, Any] = {}
    bare: list[Any] = []
    for key, opener in dc.iter_block_entries(tz):
        budget[0] -= 1
        if budget[0] <= 0:
            if opener is dc.LBRACE:
                dc._skip_block_body(tz)
            merge_key(out, "_truncated", True)
            continue
        if opener is dc.LBRACE:
            merge_key(out, key, collect_block(tz, depth + 1, budget))
        elif opener is None:
            bare.append(key)
        else:
            merge_key(out, key, coerce_scalar(atom(opener)))
    if bare:
        out["_bare"] = bare
    return out


def flatten_records(value: Any, path: list[str] | None = None) -> list[dict[str, Any]]:
    path = path or []
    records: list[dict[str, Any]] = []
    if isinstance(value, dict):
        if any(key in value for key in INTERESTING_KEYS):
            records.append(
                {
                    "path": ".".join(path) if path else ".",
                    "event": value.get("event") or value.get("id") or value.get("name"),
                    "days": value.get("days"),
                    "hours": value.get("hours"),
                    "months": value.get("months"),
                    "originator": value.get("originator"),
                    "scope": value.get("scope"),
                    "random": value.get("random"),
                    "raw": value,
                }
            )
        for key, child in value.items():
            if key == "raw":
                continue
            if isinstance(child, list):
                for index, item in enumerate(child):
                    records.extend(flatten_records(item, path + [key, str(index)]))
            elif isinstance(child, dict):
                records.extend(flatten_records(child, path + [key]))
    elif isinstance(value, list):
        for index, item in enumerate(value):
            records.extend(flatten_records(item, path + [str(index)]))
    return records


def parse_country_block(tz: dc.Tokenizer, owner_key: str, out: list[dict[str, Any]]) -> None:
    for key, opener in dc.iter_block_entries(tz):
        if opener is dc.LBRACE:
            if key == "delayed_event":
                tree = collect_block(tz)
                records = flatten_records(tree, ["delayed_event"])
                out.append(
                    {
                        "owner_key": owner_key,
                        "record_count": len(records),
                        "records": records,
                        "raw": tree,
                    }
                )
            else:
                dc._skip_block_body(tz)


def parse_countries(tz: dc.Tokenizer, out: list[dict[str, Any]]) -> None:
    for owner_key, opener in dc.iter_block_entries(tz):
        if opener is dc.LBRACE:
            parse_country_block(tz, owner_key, out)


def parse_pending_events(tz: dc.Tokenizer, out: list[dict[str, Any]]) -> None:
    tree = collect_block(tz)
    out.extend(flatten_records(tree, ["pending_events"]))


def build_oracle(text: str, source: Path, was_melted: bool) -> dict[str, Any]:
    country_events: list[dict[str, Any]] = []
    pending_events: list[dict[str, Any]] = []
    meta = dc.top_level_pass(
        text,
        {
            "countries": lambda tz: parse_countries(tz, country_events),
            "pending_events": lambda tz: parse_pending_events(tz, pending_events),
        },
        scalar_keys=("date", "version", "player"),
    )
    nonempty_country_events = [row for row in country_events if row["record_count"] > 0]
    total_country_records = sum(row["record_count"] for row in country_events)
    return {
        "schema": "event_delayed_event_save_oracle/v1",
        "source": {
            "filename": source.name,
            "path_redacted": ps._redact(source),
            "was_melted": was_melted,
            "save_date": meta.get("date"),
            "save_version": meta.get("version"),
            "player": meta.get("player"),
        },
        "summary": {
            "country_blocks_with_delayed_event_key": len(country_events),
            "country_blocks_with_positive_records": len(nonempty_country_events),
            "country_delayed_event_count": total_country_records,
            "top_level_pending_event_count": len(pending_events),
            "validation_status": "positive_fixture" if total_country_records > 0 else "blocked_no_country_delayed_event_rows",
        },
        "country_delayed_events": country_events,
        "top_level_pending_events": pending_events,
        "rejected_proxies": [
            {
                "candidate": "saved_event_target",
                "status": "not_used_by_this_oracle",
                "reason": "top-level saved scopes are not per-country delayed_event queue rows",
            },
            {
                "candidate": "empty pending_events",
                "status": "not_a_positive_country_delayed_event_fixture",
                "reason": "top-level pending_events is retained for comparison but cannot validate countries.*.delayed_event roots when empty",
            },
        ],
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--save", required=True, help="Path to .hoi4 or melted plaintext save.")
    parser.add_argument("--rakaly", default=None, help="Optional rakaly executable for binary saves.")
    parser.add_argument("--out", required=True, help="Output oracle JSON.")
    args = parser.parse_args()

    save_path = Path(args.save)
    if not save_path.exists():
        raise SystemExit("save not found: %s" % save_path)
    rakaly = ps.find_rakaly(args.rakaly)
    text, was_melted = ps.melt_if_needed(save_path, rakaly, Path(args.out).parent)
    oracle = build_oracle(text, save_path, was_melted)
    Path(args.out).parent.mkdir(parents=True, exist_ok=True)
    Path(args.out).write_text(json.dumps(oracle, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(oracle["summary"], indent=2, sort_keys=True))
    print(args.out)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
