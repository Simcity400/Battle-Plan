#!/usr/bin/env python3
"""Fail-closed 870ebf country resource-flow candidate probe.

The probe enumerates compact vector-like containers reachable from CCountry and
CProductionManager, then optionally compares known resource-id rows against a
same-frame UI oracle. All outputs are candidates unless the oracle comparison is
complete across the sampled countries and directly answers prior rejections.
"""

from __future__ import annotations

import argparse
import json
import struct
import sys
import time
from collections import Counter, defaultdict
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

MEMORY_READER = Path(__file__).resolve().parents[1]
if str(MEMORY_READER) not in sys.path:
    sys.path.insert(0, str(MEMORY_READER))

import winmem  # type: ignore  # noqa: E402
import market_trade_live_probe as mtp  # type: ignore  # noqa: E402


PROC_NAME = "hoi4.exe"
EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"

CCOUNTRY_SCAN_START = 0x400
CCOUNTRY_SCAN_END = 0x2400
PROD_MANAGER_SCAN_START = 0x0
PROD_MANAGER_SCAN_END = 0x1200
OBJ_PRODUCTION_MANAGER = 0xF68
MAX_VECTOR_ROWS = 128
MAX_AMOUNT_ABS = 10_000_000_000

RESOURCE_ID_NAMES = {
    0x3E03: "steel",
    0x3E05: "rubber",
    0x3E06: "tungsten",
    0x3E07: "chromium",
    0x4E1F: "aluminium",
    0x4E25: "coal",
    0x4E37: "oil",
}
UI_CATEGORIES = ("produced", "imported", "exported", "used", "deficit", "to_export", "available")
SCALES = (1, 1000, 100000)
PRIOR_REJECTED_CCOUNTRY_OFFSETS = {
    0x5B8: "Rejected by market_trade_live resource_flow_root_probe v1: no production-ready mapping.",
    0x1218: "Rejected by market_trade_live resource_flow_state_shape_probe v1: fewer ids than owned state resources.",
}


def hx(value: int | None) -> str | None:
    return "0x%X" % value if isinstance(value, int) else None


def read_u32(blob: bytes, off: int) -> int | None:
    if off < 0 or off + 4 > len(blob):
        return None
    return struct.unpack_from("<I", blob, off)[0]


def read_i32(blob: bytes, off: int) -> int | None:
    if off < 0 or off + 4 > len(blob):
        return None
    return struct.unpack_from("<i", blob, off)[0]


def read_u64(blob: bytes, off: int) -> int | None:
    if off < 0 or off + 8 > len(blob):
        return None
    return struct.unpack_from("<Q", blob, off)[0]


def read_i64(blob: bytes, off: int) -> int | None:
    if off < 0 or off + 8 > len(blob):
        return None
    return struct.unpack_from("<q", blob, off)[0]


def safe_u64(p: Any, addr: int) -> int | None:
    try:
        return p.u64(addr)
    except OSError:
        return None


def plausible_resource_id(value: int | None) -> bool:
    return isinstance(value, int) and (value in RESOURCE_ID_NAMES or 0 <= value <= 64)


def plausible_amount(value: int | None) -> bool:
    return isinstance(value, int) and abs(value) <= MAX_AMOUNT_ABS


def country_maps_with_dynamic_fallback(p: Any) -> dict[str, Any]:
    maps = mtp.read_country_maps(p)
    id_to_country = dict(maps["id_to_country"])
    table = maps.get("country_table")
    tag_count = maps.get("tag_count") or 0
    country_count = maps.get("country_count") or 0
    country_vtable = p.module_base + mtp.COUNTRY_VTABLE_RVA
    if mtp.is_heap(table):
        for tag_id in range(1, min(tag_count, country_count, mtp.MAX_TAG_ROWS)):
            ptr = safe_u64(p, table + tag_id * 8)
            if (
                mtp.is_heap(ptr)
                and safe_u64(p, ptr) == country_vtable
                and mtp.safe_u32(p, ptr + mtp.OBJ_TAG_DB_ID) == tag_id
            ):
                id_to_country.setdefault(tag_id, ptr)
    maps["id_to_country_dynamic"] = id_to_country
    return maps


def load_ui_oracle(path: str | None) -> dict[str, dict[str, dict[str, float]]]:
    if not path:
        return {}
    raw = json.loads(Path(path).read_text(encoding="utf-8"))
    countries = raw.get("countries", raw)
    out: dict[str, dict[str, dict[str, float]]] = {}
    for tag, row in countries.items():
        resources = row.get("resources", {}) if isinstance(row, dict) else {}
        if not isinstance(resources, dict):
            continue
        tag_out: dict[str, dict[str, float]] = defaultdict(dict)
        for res_name, values in resources.items():
            if not isinstance(values, dict):
                continue
            for category, value in values.items():
                if category in UI_CATEGORIES and isinstance(value, (int, float)):
                    tag_out[category][res_name] = float(value)
        out[tag.upper()] = dict(tag_out)
    return out


def selected_countries(
    maps: dict[str, Any],
    ui_oracle: dict[str, Any],
    requested: list[str],
    max_countries: int,
    min_countries: int,
) -> list[tuple[int, str, int]]:
    tag_to_id = {tag: tag_id for tag_id, tag in maps["id_to_tag"].items()}
    selected_tags: list[str] = []

    def add(tag: str) -> None:
        if tag in tag_to_id and tag_to_id[tag] in maps["id_to_country_dynamic"] and tag not in selected_tags:
            selected_tags.append(tag)

    for tag in requested:
        add(tag.upper())
    for tag in sorted(ui_oracle):
        add(tag)
    for tag_id, tag in sorted(maps["id_to_tag"].items()):
        if len(selected_tags) >= max(max_countries, min_countries):
            break
        add(tag)
    return [
        (tag_to_id[tag], tag, maps["id_to_country_dynamic"][tag_to_id[tag]])
        for tag in selected_tags[:max_countries]
    ]


def score_vector(raw: bytes, size: int, stride: int, resource_off: int, amount_off: int, amount_width: int) -> dict[str, Any] | None:
    if size <= 0 or size > MAX_VECTOR_ROWS or len(raw) < size * stride:
        return None
    ids = []
    preview = []
    nonzero = 0
    for idx in range(size):
        base = idx * stride
        rid = read_u32(raw, base + resource_off)
        amount = read_i64(raw, base + amount_off) if amount_width == 8 else read_i32(raw, base + amount_off)
        if not plausible_resource_id(rid) or not plausible_amount(amount):
            return None
        ids.append(rid)
        if amount:
            nonzero += 1
        if len(preview) < 12:
            preview.append(
                {
                    "index": idx,
                    "resource_id": rid,
                    "resource_name_if_known": RESOURCE_ID_NAMES.get(rid),
                    "amount_raw": amount,
                }
            )
    return {
        "resource_ids": ids,
        "known_resource_ids": [rid for rid in ids if rid in RESOURCE_ID_NAMES],
        "nonzero_amount_rows": nonzero,
        "duplicate_resource_ids": len(ids) - len(set(ids)),
        "rows_preview": preview,
    }


def scan_object_vectors(p: Any, obj: int | None, label: str, tag: str, start: int, end: int) -> list[dict[str, Any]]:
    if not mtp.is_heap(obj):
        return []
    blob = p.try_read(obj + start, end - start)
    if len(blob) < 0x40:
        return []
    layouts = ((0, 4, 4), (4, 0, 4), (0, 8, 8), (8, 0, 8), (8, 16, 8), (16, 8, 8))
    rows = []
    for rel in range(0, len(blob) - 16, 8):
        begin = read_u64(blob, rel)
        cap = read_u32(blob, rel + 8)
        size = read_u32(blob, rel + 12)
        if not mtp.is_heap(begin) or not isinstance(size, int) or not isinstance(cap, int):
            continue
        if size == 0 or size > cap or cap > MAX_VECTOR_ROWS:
            continue
        for stride in (8, 12, 16, 24, 32, 40):
            raw = p.try_read(begin, size * stride)
            if len(raw) < size * stride:
                continue
            for amount_off, resource_off, amount_width in layouts:
                if max(amount_off + amount_width, resource_off + 4) > stride:
                    continue
                scored = score_vector(raw, size, stride, resource_off, amount_off, amount_width)
                if scored is None:
                    continue
                rows.append(
                    {
                        "tag": tag,
                        "object": label,
                        "object_offset": start + rel,
                        "object_offset_hex": hx(start + rel),
                        "begin": hx(begin),
                        "size": size,
                        "cap": cap,
                        "stride": stride,
                        "amount_off": amount_off,
                        "amount_width": amount_width,
                        "resource_off": resource_off,
                        **scored,
                    }
                )
    return rows


def compare_group_to_ui(rows: list[dict[str, Any]], ui_oracle: dict[str, Any]) -> list[dict[str, Any]]:
    if not ui_oracle:
        return []
    out = []
    for category in UI_CATEGORIES:
        for scale in SCALES:
            eligible = set()
            matched = set()
            examples = []
            mismatches = []
            for row in rows:
                tag = row["tag"]
                targets = ui_oracle.get(tag, {}).get(category, {})
                if not targets:
                    continue
                observed_by_name = {
                    item["resource_name_if_known"]: item["amount_raw"]
                    for item in row["rows_preview"]
                    if item.get("resource_name_if_known")
                }
                comparable = []
                failed = []
                for name, target in targets.items():
                    if name not in observed_by_name:
                        continue
                    observed = observed_by_name[name]
                    comparable.append({"resource": name, "observed_raw": observed, "target": target})
                    if abs((observed / scale) - target) > 0.0001:
                        failed.append({"resource": name, "observed_raw": observed, "target": target, "scale": scale})
                if not comparable:
                    continue
                eligible.add(tag)
                if not failed:
                    matched.add(tag)
                    if len(examples) < 8:
                        examples.append({"tag": tag, "comparisons": comparable[:8]})
                elif len(mismatches) < 8:
                    mismatches.append({"tag": tag, "failed": failed[:8]})
            if eligible:
                out.append(
                    {
                        "category": category,
                        "scale": scale,
                        "eligible_tag_count": len(eligible),
                        "matched_tag_count": len(matched),
                        "missing_tags": sorted(eligible - matched),
                        "examples": examples,
                        "mismatch_examples": mismatches,
                        "status": (
                            "CANDIDATE_UI_ORACLE_ALL_ELIGIBLE_MATCH"
                            if eligible == matched and len(matched) >= 2
                            else "CANDIDATE_UI_ORACLE_PARTIAL_OR_INSUFFICIENT"
                        ),
                    }
                )
    out.sort(key=lambda row: (0 if row["status"].endswith("MATCH") else 1, -row["matched_tag_count"], row["category"], row["scale"]))
    return out[:24]


def summarize_candidates(candidates: list[dict[str, Any]], ui_oracle: dict[str, Any]) -> list[dict[str, Any]]:
    groups: dict[tuple[Any, ...], list[dict[str, Any]]] = defaultdict(list)
    for row in candidates:
        key = (row["object"], row["object_offset"], row["stride"], row["amount_off"], row["amount_width"], row["resource_off"])
        groups[key].append(row)
    summaries = []
    for (obj, offset, stride, amount_off, amount_width, resource_off), rows in groups.items():
        sequences = Counter(tuple(row["resource_ids"]) for row in rows)
        prior = obj == "CCountry" and offset in PRIOR_REJECTED_CCOUNTRY_OFFSETS
        summaries.append(
            {
                "object": obj,
                "offset": hx(offset),
                "path": f"{obj}+{hx(offset)}",
                "stride": stride,
                "amount_off": amount_off,
                "amount_width": amount_width,
                "resource_off": resource_off,
                "tag_count": len({row["tag"] for row in rows}),
                "nonzero_tag_count": sum(1 for row in rows if row["nonzero_amount_rows"] > 0),
                "known_resource_tag_count": sum(1 for row in rows if row["known_resource_ids"]),
                "top_resource_id_sequences": [
                    {"resource_ids": list(ids), "count": count} for ids, count in sequences.most_common(5)
                ],
                "examples": rows[:8],
                "ui_oracle_matches": [] if prior else compare_group_to_ui(rows, ui_oracle),
                "status": "REJECTED_PRIOR_EVIDENCE_NO_RETRY" if prior else "CANDIDATE_VECTOR_SHAPE_ONLY_OR_UI_MATCH",
                "prior_rejection_reason": PRIOR_REJECTED_CCOUNTRY_OFFSETS.get(offset) if prior else None,
            }
        )
    summaries.sort(key=lambda row: (row["status"].startswith("REJECTED"), -row["tag_count"], -row["known_resource_tag_count"], row["path"]))
    return summaries[:80]


def build_report(args: argparse.Namespace) -> tuple[dict[str, Any], int]:
    pid = args.pid or winmem.find_pid(PROC_NAME)
    if not pid:
        return {"status": "process_not_found", "process": PROC_NAME, "fail_closed": True}, 3
    ui_oracle = load_ui_oracle(args.ui_oracle)
    with winmem.Process(pid, PROC_NAME) as p:
        fp = p.fingerprint()
        sha = str(fp.get("sha256") or "").lower()
        base_report: dict[str, Any] = {
            "schema": "battle-plan.resource_flow_delta_probe_870ebf.v1",
            "generated_utc": datetime.now(timezone.utc).isoformat(timespec="seconds"),
            "expected_sha256": EXPECTED_SHA256,
            "process": {"pid": pid, "module_base": hx(p.module_base), "path": fp.get("path"), "sha256": sha},
            "ui_oracle": args.ui_oracle,
            "fail_closed": sha != EXPECTED_SHA256,
        }
        if sha != EXPECTED_SHA256:
            base_report["status"] = "build_gate_failed"
            return base_report, 2
        started = time.perf_counter()
        maps = country_maps_with_dynamic_fallback(p)
        selected = selected_countries(maps, ui_oracle, args.sample_tags, args.max_countries, args.min_countries)
        candidates = []
        for _tag_id, tag, country in selected:
            candidates.extend(scan_object_vectors(p, country, "CCountry", tag, CCOUNTRY_SCAN_START, CCOUNTRY_SCAN_END))
            manager = safe_u64(p, country + OBJ_PRODUCTION_MANAGER)
            candidates.extend(scan_object_vectors(p, manager, "CProductionManager", tag, PROD_MANAGER_SCAN_START, PROD_MANAGER_SCAN_END))
        report = {
            **base_report,
            "status": "probe_completed_candidates_only",
            "scan": {
                "selected_country_count": len(selected),
                "selected_tags": [tag for _tag_id, tag, _ptr in selected],
                "resource_id_names": {hx(k): v for k, v in RESOURCE_ID_NAMES.items()},
                "prior_rejected_ccountry_offsets": {hx(k): v for k, v in PRIOR_REJECTED_CCOUNTRY_OFFSETS.items()},
            },
            "candidate_group_count": len(candidates),
            "candidate_groups": summarize_candidates(candidates, ui_oracle),
            "promotion_rule": (
                "Promote only a non-prior-rejected path that matches a same-frame UI oracle "
                "for produced/imported/exported/used/deficit across multiple countries or moves "
                "by an isolated controlled delta with stable controls."
            ),
            "elapsed_ms": round((time.perf_counter() - started) * 1000, 3),
        }
        return report, 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pid", type=int)
    parser.add_argument("--ui-oracle")
    parser.add_argument("--out-json", required=True)
    parser.add_argument("--max-countries", type=int, default=96)
    parser.add_argument("--min-countries", type=int, default=32)
    parser.add_argument("--sample-tags", nargs="*", default=["IRE", "ENG", "GER", "USA", "SOV", "JAP", "ITA", "FRA", "SWE", "TUR"])
    args = parser.parse_args()
    report, code = build_report(args)
    out = Path(args.out_json)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({"status": report.get("status"), "out": str(out)}, sort_keys=True))
    return code


if __name__ == "__main__":
    raise SystemExit(main())
