#!/usr/bin/env python3
"""Probe only mission/decision state not already covered by domains 16, 45, 51.

Known CDecisionStatus vectors are summarized but not promoted here. Unknown
mission-like containers remain candidates until a controlled fixture gives
owner, timer, and UI/save semantics.
"""

from __future__ import annotations

import argparse
import json
import time
from pathlib import Path
from typing import Any

import probe_common as pc


PROBE = "probe_870ebf_mission_decision_state"
RTTI_TERMS = ("Mission", "Decision", "Timed", "Cooldown", "Event")
MISSION_LIKE_TERMS = ("mission", "decision", "timed", "cooldown")

CDECISION_STATUS = 0xFA0
CDECISION_STATUS_VTABLE_RVA = 0x27D50F0

KNOWN_DECISION_LISTS = {
    0x40: "decisions_taken_bucket",
    0x88: "active_targeted_decision",
    0xA0: "active_timed_decision",
    0xE8: "active_targeted_timed_decision",
    0x130: "decision_to_re_enable",
    0x148: "random_item",
}

DOMAIN51_ROOTS = {
    0x218: "country_variables_domain51",
    0x230: "country_flags_domain51",
}


def summarize_vector(p, status_ptr: int, offset: int, interesting_vtables: set[int]) -> dict[str, Any]:
    shapes = pc.vector_shapes_at(p, status_ptr, offset)
    if not shapes:
        return {
            "offset": pc.hx(offset),
            "status": "no_vector_shape",
        }
    shape = max(shapes, key=lambda row: int(row.get("count", 0)))
    sample = pc.sample_pointer_vector(
        p,
        int(shape["data"]),
        int(shape["count"]),
        interesting_vtables,
        max_entries=32,
    )
    return {
        "offset": pc.hx(offset),
        "shape": {
            "kind": shape["kind"],
            "data": pc.hx(int(shape["data"])),
            "count": shape["count"],
            "capacity": shape["capacity"],
        },
        "pointer_sample": sample,
        "status": "observed_vector",
    }


def read_decision_status_roots(p, roots: dict[str, Any], max_countries: int) -> tuple[list[dict[str, Any]], dict[str, int]]:
    out: list[dict[str, Any]] = []
    invalid = {"null_or_nonheap": 0, "vtable_mismatch": 0}
    expected_vt = p.module_base + CDECISION_STATUS_VTABLE_RVA
    for row in roots["validated_countries"][:max_countries]:
        ptr = pc.try_u64(p, int(row["country_ptr"]) + CDECISION_STATUS)
        if not pc.heap_ok(ptr):
            invalid["null_or_nonheap"] += 1
            continue
        vt = pc.try_u64(p, int(ptr))
        if vt != expected_vt:
            invalid["vtable_mismatch"] += 1
            continue
        out.append(
            {
                "tag_db_id": row["tag_db_id"],
                "tag": row.get("tag"),
                "country_ptr": int(row["country_ptr"]),
                "decision_status": int(ptr),
            }
        )
    return out, invalid


def summarize_known_lists(p, status_rows: list[dict[str, Any]], interesting_vtables: set[int]) -> dict[str, Any]:
    totals: dict[str, Any] = {}
    for offset, name in KNOWN_DECISION_LISTS.items():
        count_total = 0
        owner_count = 0
        samples: list[dict[str, Any]] = []
        rtti_counts: dict[str, int] = {}
        for row in status_rows:
            summary = summarize_vector(p, int(row["decision_status"]), offset, interesting_vtables)
            shape = summary.get("shape")
            if not shape:
                continue
            owner_count += 1
            count_total += int(shape.get("count", 0))
            for rtti, count in summary.get("pointer_sample", {}).get("rtti_counts", {}).items():
                rtti_counts[rtti] = rtti_counts.get(rtti, 0) + int(count)
            if len(samples) < 8:
                samples.append(
                    {
                        "tag_db_id": row["tag_db_id"],
                        "tag": row.get("tag"),
                        "decision_status": pc.hx(int(row["decision_status"])),
                        "summary": summary,
                    }
                )
        totals[name] = {
            "offset": pc.hx(offset),
            "owners_with_vector": owner_count,
            "total_rows": count_total,
            "rtti_counts": dict(sorted(rtti_counts.items(), key=lambda item: (-item[1], item[0]))[:16]),
            "samples": samples,
            "coverage": "already_captured_or_badged_by_domain_16_45",
        }
    return totals


def scan_unknown_decision_vectors(
    p,
    status_rows: list[dict[str, Any]],
    scan_bytes: int,
    interesting_vtables: set[int],
    max_candidates: int,
) -> list[dict[str, Any]]:
    grouped: dict[tuple[int, str], dict[str, Any]] = {}
    known_offsets = set(KNOWN_DECISION_LISTS)
    for row in status_rows:
        status_ptr = int(row["decision_status"])
        for offset in range(0, scan_bytes, 8):
            if offset in known_offsets:
                continue
            for shape in pc.vector_shapes_at(p, status_ptr, offset):
                sample = pc.sample_pointer_vector(
                    p,
                    int(shape["data"]),
                    int(shape["count"]),
                    interesting_vtables,
                    max_entries=24,
                )
                if sample["heap_pointer_entries"] == 0:
                    continue
                interesting = sample.get("interesting_rtti_entries", 0)
                rtti_key = ",".join(sorted(sample.get("rtti_counts", {}).keys())[:3])
                key = (offset, rtti_key)
                rec = grouped.setdefault(
                    key,
                    {
                        "offset": pc.hx(offset),
                        "owners": 0,
                        "total_rows": 0,
                        "interesting_rtti_entries": 0,
                        "rtti_counts": {},
                        "samples": [],
                        "status": "candidate_requires_fixture" if interesting else "blocked_no_mission_like_rtti",
                        "promotion": "not_promoted_candidate_only",
                    },
                )
                rec["owners"] += 1
                rec["total_rows"] += int(shape["count"])
                rec["interesting_rtti_entries"] += int(interesting)
                for rtti, count in sample.get("rtti_counts", {}).items():
                    rec["rtti_counts"][rtti] = rec["rtti_counts"].get(rtti, 0) + int(count)
                if len(rec["samples"]) < 10:
                    rec["samples"].append(
                        {
                            "tag_db_id": row["tag_db_id"],
                            "tag": row.get("tag"),
                            "decision_status": pc.hx(status_ptr),
                            "shape": {
                                "kind": shape["kind"],
                                "data": pc.hx(int(shape["data"])),
                                "count": shape["count"],
                                "capacity": shape["capacity"],
                            },
                            "pointer_sample": sample,
                        }
                    )
    rows = list(grouped.values())
    for row in rows:
        row["rtti_counts"] = dict(sorted(row["rtti_counts"].items(), key=lambda item: (-item[1], item[0]))[:16])
        names = " ".join(row["rtti_counts"].keys()).lower()
        if any(term in names for term in MISSION_LIKE_TERMS):
            row["status"] = "candidate_requires_fixture"
    rows.sort(
        key=lambda row: (
            -int(row.get("interesting_rtti_entries", 0)),
            -int(row.get("owners", 0)),
            -int(row.get("total_rows", 0)),
            row.get("offset") or "",
        )
    )
    return rows[:max_candidates]


def scan_country_mission_pointers(
    p,
    country_rows: list[dict[str, Any]],
    scan_bytes: int,
    interesting_vtables: set[int],
    max_candidates: int,
) -> list[dict[str, Any]]:
    grouped: dict[str, dict[str, Any]] = {}
    excluded = set(DOMAIN51_ROOTS) | {CDECISION_STATUS}
    for row in country_rows:
        country = int(row["country_ptr"])
        for offset in range(0, scan_bytes, 8):
            if offset in excluded:
                continue
            ptr = pc.try_u64(p, country + offset)
            if not pc.heap_ok(ptr):
                continue
            vt = pc.try_u64(p, int(ptr))
            name = pc.rtti_name_for_vtable(p, vt)
            if not name or not any(term in name.lower() for term in MISSION_LIKE_TERMS):
                continue
            key = "%s:%s" % (pc.hx(offset), name)
            rec = grouped.setdefault(
                key,
                {
                    "country_offset": pc.hx(offset),
                    "rtti": name,
                    "vtable_rva": pc.hx(vt - p.module_base) if isinstance(vt, int) else None,
                    "owners": 0,
                    "samples": [],
                    "status": "candidate_requires_fixture",
                    "promotion": "not_promoted_candidate_only",
                },
            )
            rec["owners"] += 1
            if len(rec["samples"]) < 10:
                rec["samples"].append(
                    {
                        "tag_db_id": row["tag_db_id"],
                        "tag": row.get("tag"),
                        "country_ptr": pc.hx(country),
                        "ptr": pc.hx(int(ptr)),
                    }
                )
    rows = list(grouped.values())
    rows.sort(key=lambda row: (-int(row.get("owners", 0)), row.get("country_offset") or ""))
    return rows[:max_candidates]


def build_summary(report: dict[str, Any]) -> dict[str, Any]:
    unknown = report.get("unknown_decision_status_vectors", [])
    country_ptrs = report.get("country_mission_like_pointers", [])
    candidates = [
        row for row in unknown + country_ptrs
        if row.get("status") == "candidate_requires_fixture"
    ]
    return {
        "status": "candidate_found_requires_fixture" if candidates else "no_separate_mission_container_found",
        "known_decision_lists": len(report.get("known_decision_lists", {})),
        "unknown_vector_candidate_count": len(unknown),
        "country_pointer_candidate_count": len(country_ptrs),
        "candidate_requires_fixture_count": len(candidates),
        "promotion": "none",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", required=True)
    parser.add_argument("--max-countries", type=int, default=512)
    parser.add_argument("--scan-decision-status-bytes", type=pc.parse_int, default=0x220)
    parser.add_argument("--scan-country-bytes", type=pc.parse_int, default=0x1800)
    parser.add_argument("--max-candidates", type=int, default=120)
    args = parser.parse_args()

    p, fail_report, code = pc.open_process_with_sha_or_report(PROBE)
    if fail_report is not None:
        pc.write_json(Path(args.out), fail_report)
        return code
    assert p is not None

    try:
        started = time.time()
        info = pc.process_info(p)
        roots = pc.resolve_country_roots(p, max_countries=args.max_countries)
        rtti = pc.rediscover_rtti_by_terms(p, RTTI_TERMS)
        interesting_vtables = pc.rtti_vtable_values(rtti, MISSION_LIKE_TERMS)
        status_rows, invalid_status = read_decision_status_roots(
            p, roots, max_countries=args.max_countries
        )
        known = summarize_known_lists(p, status_rows, interesting_vtables)
        unknown = scan_unknown_decision_vectors(
            p,
            status_rows,
            args.scan_decision_status_bytes,
            interesting_vtables,
            args.max_candidates,
        )
        country_ptrs = scan_country_mission_pointers(
            p,
            roots["validated_countries"],
            args.scan_country_bytes,
            interesting_vtables,
            args.max_candidates,
        )
        report: dict[str, Any] = {
            "schema": "event_mission_decision_state_probe/v1",
            "probe": PROBE,
            "expected_sha256": pc.EXPECTED_SHA256,
            "process": info,
            "fail_closed": False,
            "started_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(started)),
            "completed_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
            "elapsed_seconds": round(time.time() - started, 3),
            "source_roots": {
                "CGameState": pc.hx(roots["game_state"]),
                "country_table": pc.hx(roots["country_table"]),
                "country_count": roots["country_count"],
                "validated_country_rows": len(roots["validated_countries"]),
                "country_vtable": pc.hx(roots["country_vtable"]),
                "CDecisionStatus": "CCountry+0x%X" % CDECISION_STATUS,
                "CDecisionStatus_vtable": "module+0x%X" % CDECISION_STATUS_VTABLE_RVA,
                "validated_decision_status_rows": len(status_rows),
                "invalid_decision_status_rows": invalid_status,
            },
            "rtti": pc.compact_rtti_report(rtti),
            "known_decision_lists": known,
            "unknown_decision_status_vectors": unknown,
            "country_mission_like_pointers": country_ptrs,
            "explicit_rejections": [
                {
                    "candidate": "CCountry+0x218 CVariables",
                    "status": "rejected_for_this_probe_already_captured",
                    "evidence": "domain 51 owns country variables",
                },
                {
                    "candidate": "CCountry+0x230 CFlagManager",
                    "status": "rejected_for_this_probe_already_captured",
                    "evidence": "domain 51 owns country flags",
                },
                {
                    "candidate": "known CDecisionStatus vectors",
                    "status": "not_promoted_here",
                    "evidence": "domain 16/domain 45 own or badge known decision list families",
                },
            ],
            "scan_policy": {
                "decision_status_bytes": pc.hx(args.scan_decision_status_bytes),
                "country_bytes": pc.hx(args.scan_country_bytes),
                "promotion_policy": "candidate_only_no_root_promotion",
            },
        }
        report["summary"] = build_summary(report)
        pc.write_json(Path(args.out), report)
        print(json.dumps(report["summary"], indent=2, sort_keys=True))
        print(args.out)
        return 0
    finally:
        p.close()


if __name__ == "__main__":
    raise SystemExit(main())
