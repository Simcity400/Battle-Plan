#!/usr/bin/env python3
"""Compare two domain-58 economy root probe snapshots.

Use this after a controlled UI/save delta, for example changing one production
line's assigned/requested factories or recording a before/after resource UI
change. The output is still evidence, not validation by itself.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


def load(path: str) -> dict[str, Any]:
    with open(path, "r", encoding="utf-8") as handle:
        return json.load(handle)


def diff_focus_scalars(before: dict[str, Any], after: dict[str, Any], tag: str) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    btag = before.get("focus_scalar_samples", {}).get(tag, {})
    atag = after.get("focus_scalar_samples", {}).get(tag, {})
    for obj in sorted(set(btag) | set(atag)):
        bobj = btag.get(obj, {})
        aobj = atag.get(obj, {})
        for typ in ("i32", "f32"):
            bvals = bobj.get(typ, {})
            avals = aobj.get(typ, {})
            for offset in sorted(set(bvals) | set(avals)):
                old = bvals.get(offset)
                new = avals.get(offset)
                if old != new:
                    rows.append(
                        {
                            "tag": tag,
                            "object": obj,
                            "type": typ,
                            "offset": offset,
                            "before": old,
                            "after": new,
                            "status": "CANDIDATE_CHANGED_BY_DELTA",
                        }
                    )
    return rows


def line_key(row: dict[str, Any]) -> tuple[Any, Any, Any]:
    return row.get("tag"), row.get("kind"), row.get("line_id")


def diff_production_lines(before: dict[str, Any], after: dict[str, Any], tag: str) -> list[dict[str, Any]]:
    brows = {
        line_key(row): row
        for row in before.get("production_lines", {}).get("sample_rows", [])
        if row.get("tag") == tag
    }
    arows = {
        line_key(row): row
        for row in after.get("production_lines", {}).get("sample_rows", [])
        if row.get("tag") == tag
    }
    rows: list[dict[str, Any]] = []
    fields = ["active_factories", "cost_raw", "speed_raw", "produced_raw", "amount"]
    for key in sorted(set(brows) | set(arows)):
        b = brows.get(key, {})
        a = arows.get(key, {})
        changes = {
            field: {"before": b.get(field), "after": a.get(field)}
            for field in fields
            if b.get(field) != a.get(field)
        }
        if changes:
            rows.append(
                {
                    "tag": key[0],
                    "kind": key[1],
                    "line_id": key[2],
                    "changes": changes,
                    "status": "VALIDATED_DOMAIN11_OVERLAP_OR_DELTA_ANCHOR",
                }
            )
    return rows


def summarize(before: dict[str, Any], after: dict[str, Any], tags: list[str]) -> dict[str, Any]:
    scalar_changes: list[dict[str, Any]] = []
    line_changes: list[dict[str, Any]] = []
    for tag in tags:
        scalar_changes.extend(diff_focus_scalars(before, after, tag))
        line_changes.extend(diff_production_lines(before, after, tag))
    return {
        "schema": "battle-plan.country_economy_counters_live.probe_compare.v1",
        "before": {
            "snapshot_label": before.get("snapshot_label"),
            "process": before.get("process", {}),
            "status": before.get("status"),
        },
        "after": {
            "snapshot_label": after.get("snapshot_label"),
            "process": after.get("process", {}),
            "status": after.get("status"),
        },
        "tags": tags,
        "scalar_change_count": len(scalar_changes),
        "line_change_count": len(line_changes),
        "scalar_changes": scalar_changes[:400],
        "production_line_changes": line_changes[:120],
        "interpretation": (
            "Changed offsets are candidates only. Promote an offset only when the "
            "operator action was isolated, the changed value maps to the expected "
            "UI/save delta, and unrelated countries/lines remain stable."
        ),
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--before", required=True)
    parser.add_argument("--after", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--tags", nargs="+", default=["IRE"])
    args = parser.parse_args(argv)

    report = summarize(load(args.before), load(args.after), [tag.upper() for tag in args.tags])
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    with out.open("w", encoding="utf-8") as handle:
        json.dump(report, handle, indent=2, sort_keys=True)
        handle.write("\n")
    print(
        "compare_870ebf_country_economy_probes: "
        f"scalar_changes={report['scalar_change_count']} "
        f"line_changes={report['line_change_count']} out={out}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
