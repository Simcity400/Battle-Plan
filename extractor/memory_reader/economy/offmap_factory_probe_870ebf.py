#!/usr/bin/env python3
"""Fail-closed 870ebf probe for country offmap factory levels.

This is discovery evidence only. It scans CCountry and CProductionManager
objects for small scalar/triple candidates that match the same-save offmap
factory oracle. A match is still only a candidate until a live before/after
delta proves the field moves by the expected isolated offmap change.
"""

from __future__ import annotations

import argparse
import json
import struct
import sys
import time
from collections import defaultdict
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
PROD_MANAGER_SCAN_END = 0x1000
OBJ_PRODUCTION_MANAGER = 0xF68

FIELD_KEYS = {
    "civilian": "civilian_offmap_building_levels",
    "military": "military_offmap_building_levels",
    "dockyard": "dockyard_offmap_building_levels",
}
NONZERO_MIN = {"civilian": 3, "military": 2, "dockyard": 1}

KNOWN_REJECTIONS = [
    {
        "path": "CCountry+0x1750",
        "reason": "Prior v4 raw oracle matched IRE civilian total only; ENG/SOV/FRA read pointer-like garbage.",
    },
    {
        "path": "CCountry+0x17E0",
        "reason": "Prior v4 raw oracle matched GER civilian total only; most sampled countries read zero or garbage.",
    },
    {
        "path": "CCountry+0x1038",
        "reason": "Prior v3/v4 dockyard candidate was near-constant/capped and matched only GER.",
    },
    {
        "path": "CCountry+0xF08",
        "reason": "Prior v3/v4 showed real per-country data but no derived/raw oracle field match.",
    },
]
KNOWN_REJECTED_CCOUNTRY_OFFSETS = {0xF08, 0x1038, 0x1750, 0x17E0}


def hx(value: int | None) -> str | None:
    return "0x%X" % value if isinstance(value, int) else None


def read_u64_blob(blob: bytes, offset: int) -> int | None:
    if offset < 0 or offset + 8 > len(blob):
        return None
    return struct.unpack_from("<Q", blob, offset)[0]


def read_i32_blob(blob: bytes, offset: int) -> int | None:
    if offset < 0 or offset + 4 > len(blob):
        return None
    return struct.unpack_from("<i", blob, offset)[0]


def safe_u64(p: Any, addr: int) -> int | None:
    try:
        return p.u64(addr)
    except OSError:
        return None


def country_maps_with_dynamic_fallback(p: Any) -> dict[str, Any]:
    maps = mtp.read_country_maps(p)
    id_to_country = dict(maps["id_to_country"])
    table = maps.get("country_table")
    tag_count = maps.get("tag_count") or 0
    country_count = maps.get("country_count") or 0
    country_vtable = p.module_base + mtp.COUNTRY_VTABLE_RVA
    if mtp.is_heap(table):
        for tag_id in range(1, min(tag_count, country_count, mtp.MAX_TAG_ROWS)):
            tag = maps["id_to_tag"].get(tag_id)
            if not tag:
                continue
            ptr = safe_u64(p, table + tag_id * 8)
            if (
                mtp.is_heap(ptr)
                and safe_u64(p, ptr) == country_vtable
                and mtp.safe_u32(p, ptr + mtp.OBJ_TAG_DB_ID) == tag_id
            ):
                id_to_country.setdefault(tag_id, ptr)
    maps["id_to_country_dynamic"] = id_to_country
    return maps


def load_offmap_oracle(path: str) -> dict[str, dict[str, int | None]]:
    data = json.loads(Path(path).read_text(encoding="utf-8"))
    countries = data.get("countries", {})
    out: dict[str, dict[str, int | None]] = {}
    for tag, row in countries.items():
        raw = row.get("factories_raw", {}) if isinstance(row, dict) else {}
        if not isinstance(raw, dict):
            continue
        out[tag] = {kind: raw.get(key) for kind, key in FIELD_KEYS.items()}
    return out


def select_tags(
    oracle: dict[str, dict[str, int | None]],
    live_tags: dict[int, str],
    requested: list[str],
    max_countries: int,
) -> list[tuple[int, str]]:
    live_by_tag = {tag: tag_id for tag_id, tag in live_tags.items()}
    selected: list[str] = []

    def add(tag: str) -> None:
        if tag in oracle and tag in live_by_tag and tag not in selected:
            selected.append(tag)

    for tag in requested:
        add(tag.upper())
    for tag, fields in sorted(oracle.items()):
        if any(isinstance(value, int) and value != 0 for value in fields.values()):
            add(tag)
    for tag, fields in sorted(oracle.items()):
        if all((value or 0) == 0 for value in fields.values()):
            add(tag)
        if len(selected) >= max_countries:
            break
    return [(live_by_tag[tag], tag) for tag in selected[:max_countries]]


def read_object_blob(p: Any, base: int | None, start: int, end: int) -> bytes:
    if not mtp.is_heap(base):
        return b""
    blob = p.try_read(base + start, end - start)
    return blob if len(blob) >= 0x40 else b""


def collect_scalar_groups(
    p: Any,
    selected: list[tuple[int, str]],
    id_to_country: dict[int, int],
) -> dict[tuple[str, int], dict[str, int]]:
    groups: dict[tuple[str, int], dict[str, int]] = defaultdict(dict)
    for tag_id, tag in selected:
        country = id_to_country.get(tag_id)
        country_blob = read_object_blob(p, country, CCOUNTRY_SCAN_START, CCOUNTRY_SCAN_END)
        for rel in range(0, len(country_blob) - 3, 4):
            offset = CCOUNTRY_SCAN_START + rel
            if offset in KNOWN_REJECTED_CCOUNTRY_OFFSETS:
                continue
            value = read_i32_blob(country_blob, rel)
            if value is not None and -1000 <= value <= 1000:
                groups[("CCountry", offset)][tag] = value
        manager = safe_u64(p, country + OBJ_PRODUCTION_MANAGER) if mtp.is_heap(country) else None
        manager_blob = read_object_blob(p, manager, PROD_MANAGER_SCAN_START, PROD_MANAGER_SCAN_END)
        for rel in range(0, len(manager_blob) - 3, 4):
            value = read_i32_blob(manager_blob, rel)
            if value is not None and -1000 <= value <= 1000:
                groups[("CProductionManager", PROD_MANAGER_SCAN_START + rel)][tag] = value
    return groups


def summarize_field_candidates(
    groups: dict[tuple[str, int], dict[str, int]],
    oracle: dict[str, dict[str, int | None]],
    selected_tags: list[str],
) -> dict[str, list[dict[str, Any]]]:
    by_field: dict[str, list[dict[str, Any]]] = {}
    for field in FIELD_KEYS:
        rows = []
        for (obj, offset), values in groups.items():
            eligible = []
            matches = []
            mismatches = []
            for tag in selected_tags:
                target = oracle.get(tag, {}).get(field)
                observed = values.get(tag)
                if target is None or observed is None:
                    continue
                eligible.append(tag)
                if observed == target:
                    matches.append(tag)
                elif len(mismatches) < 8:
                    mismatches.append({"tag": tag, "observed": observed, "target": target})
            if not eligible:
                continue
            nonzero_matches = sum(1 for tag in matches if oracle[tag].get(field))
            all_selected = len(matches) == len(eligible) == len(selected_tags)
            if all_selected and nonzero_matches >= NONZERO_MIN[field]:
                status = "CANDIDATE_ALL_SELECTED_EXACT_NEEDS_LIVE_DELTA"
            elif all_selected:
                status = "REJECTED_ZERO_DOMINATED_AMBIGUOUS"
            elif matches:
                status = "CANDIDATE_PARTIAL_ORACLE_MATCH"
            else:
                status = "REJECTED_NO_ORACLE_MATCH"
            value_vector = [
                {
                    "tag": tag,
                    "observed": values.get(tag),
                    "target": oracle.get(tag, {}).get(field),
                }
                for tag in selected_tags[:32]
            ]
            rows.append(
                {
                    "object": obj,
                    "offset": hx(offset),
                    "path": f"{obj}+{hx(offset)}",
                    "eligible_count": len(eligible),
                    "match_count": len(matches),
                    "nonzero_match_count": nonzero_matches,
                    "mismatch_examples": mismatches,
                    "value_vector_sample": value_vector,
                    "status": status,
                }
            )
        rows.sort(
            key=lambda row: (
                0 if row["status"] == "CANDIDATE_ALL_SELECTED_EXACT_NEEDS_LIVE_DELTA" else 1,
                -row["match_count"],
                -row["nonzero_match_count"],
                row["path"],
            )
        )
        by_field[field] = rows[:40]
    return by_field


def summarize_triples(
    groups: dict[tuple[str, int], dict[str, int]],
    oracle: dict[str, dict[str, int | None]],
    selected_tags: list[str],
) -> list[dict[str, Any]]:
    by_obj = defaultdict(dict)
    for (obj, offset), values in groups.items():
        by_obj[obj][offset] = values
    rows = []
    for obj, offsets in by_obj.items():
        for offset in sorted(offsets):
            if offset + 4 not in offsets or offset + 8 not in offsets:
                continue
            matches = []
            mismatches = []
            for tag in selected_tags:
                target = tuple(oracle.get(tag, {}).get(k) for k in ("civilian", "military", "dockyard"))
                observed = (
                    offsets[offset].get(tag),
                    offsets[offset + 4].get(tag),
                    offsets[offset + 8].get(tag),
                )
                if any(value is None for value in target + observed):
                    continue
                if observed == target:
                    matches.append(tag)
                elif len(mismatches) < 8:
                    mismatches.append({"tag": tag, "observed": observed, "target": target})
            nonzero_matches = sum(
                1 for tag in matches if any(oracle[tag].get(k) for k in ("civilian", "military", "dockyard"))
            )
            if matches:
                value_vector = [
                    {
                        "tag": tag,
                        "observed": (
                            offsets[offset].get(tag),
                            offsets[offset + 4].get(tag),
                            offsets[offset + 8].get(tag),
                        ),
                        "target": tuple(
                            oracle.get(tag, {}).get(k)
                            for k in ("civilian", "military", "dockyard")
                        ),
                    }
                    for tag in selected_tags[:32]
                ]
                rows.append(
                    {
                        "object": obj,
                        "offsets": [hx(offset), hx(offset + 4), hx(offset + 8)],
                        "path": f"{obj}+{hx(offset)} triple i32",
                        "match_count": len(matches),
                        "nonzero_match_count": nonzero_matches,
                        "mismatch_examples": mismatches,
                        "value_vector_sample": value_vector,
                        "status": (
                            "CANDIDATE_TRIPLE_ALL_SELECTED_EXACT_NEEDS_LIVE_DELTA"
                            if len(matches) == len(selected_tags) and nonzero_matches >= 3
                            else "CANDIDATE_TRIPLE_PARTIAL_OR_AMBIGUOUS"
                        ),
                    }
                )
    rows.sort(key=lambda row: (0 if "ALL_SELECTED" in row["status"] else 1, -row["match_count"], row["path"]))
    return rows[:40]


def build_report(args: argparse.Namespace) -> tuple[dict[str, Any], int]:
    pid = args.pid or winmem.find_pid(PROC_NAME)
    if not pid:
        return {"status": "process_not_found", "process": PROC_NAME, "fail_closed": True}, 3
    oracle = load_offmap_oracle(args.economy_raw)
    with winmem.Process(pid, PROC_NAME) as p:
        fp = p.fingerprint()
        sha = str(fp.get("sha256") or "").lower()
        base_report: dict[str, Any] = {
            "schema": "battle-plan.offmap_factory_probe_870ebf.v1",
            "generated_utc": datetime.now(timezone.utc).isoformat(timespec="seconds"),
            "expected_sha256": EXPECTED_SHA256,
            "process": {
                "pid": pid,
                "module_base": hx(p.module_base),
                "module_size": p.module_size,
                "path": fp.get("path"),
                "sha256": sha,
            },
            "oracle": args.economy_raw,
            "fail_closed": sha != EXPECTED_SHA256,
        }
        if sha != EXPECTED_SHA256:
            base_report["status"] = "build_gate_failed"
            return base_report, 2
        maps = country_maps_with_dynamic_fallback(p)
        selected = select_tags(oracle, maps["id_to_tag"], args.sample_tags, args.max_countries)
        selected_tags = [tag for _tag_id, tag in selected]
        started = time.perf_counter()
        groups = collect_scalar_groups(p, selected, maps["id_to_country_dynamic"])
        report = {
            **base_report,
            "status": "probe_completed_candidates_only",
            "scan": {
                "selected_country_count": len(selected),
                "selected_tags": selected_tags,
                "ccountry_range": f"0x{CCOUNTRY_SCAN_START:X}..0x{CCOUNTRY_SCAN_END:X}",
                "production_manager_range": f"0x{PROD_MANAGER_SCAN_START:X}..0x{PROD_MANAGER_SCAN_END:X}",
                "excluded_ccountry_offsets": [hx(value) for value in sorted(KNOWN_REJECTED_CCOUNTRY_OFFSETS)],
            },
            "field_candidates": summarize_field_candidates(groups, oracle, selected_tags),
            "triple_candidates": summarize_triples(groups, oracle, selected_tags),
            "known_rejections_not_retried": KNOWN_REJECTIONS,
            "promotion_rule": (
                "Do not promote from this static oracle scan. A candidate must move by an isolated "
                "offmap add/remove delta for the edited country while control countries remain stable."
            ),
            "elapsed_ms": round((time.perf_counter() - started) * 1000, 3),
        }
        return report, 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pid", type=int)
    parser.add_argument("--economy-raw", required=True)
    parser.add_argument("--out-json", required=True)
    parser.add_argument("--max-countries", type=int, default=128)
    parser.add_argument(
        "--sample-tags",
        nargs="*",
        default=["IRE", "ENG", "GER", "AST", "FRA", "EST", "NOR", "USA", "SOV", "JAP", "ITA", "CAN"],
    )
    args = parser.parse_args()
    report, code = build_report(args)
    out = Path(args.out_json)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({"status": report.get("status"), "out": str(out)}, sort_keys=True))
    return code


if __name__ == "__main__":
    raise SystemExit(main())
