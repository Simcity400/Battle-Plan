#!/usr/bin/env python3
"""Discover delayed-event queue candidates in a live 870ebf the game process.

This is a bounded read-only probe. It can identify candidates and reject obvious
non-matches, but it must not promote a root without a positive save oracle and a
follow-up exact decoder/proof.
"""

from __future__ import annotations

import argparse
import json
import time
from pathlib import Path
from typing import Any

import probe_common as pc


PROBE = "probe_870ebf_delayed_event_queues"
RTTI_TERMS = ("Event", "Delayed", "Queue", "OnAction", "Mission", "Decision")
EVENT_LIKE_TERMS = ("event", "delayed", "queue", "onaction")


def oracle_positive_rows(oracle: dict[str, Any] | None) -> int:
    if not isinstance(oracle, dict):
        return 0
    summary = oracle.get("summary", {})
    value = summary.get("country_delayed_event_count")
    return value if isinstance(value, int) else 0


def oracle_owner_keys(oracle: dict[str, Any] | None) -> set[str]:
    out: set[str] = set()
    if not isinstance(oracle, dict):
        return out
    for row in oracle.get("country_delayed_events", []):
        if not isinstance(row, dict):
            continue
        if int(row.get("record_count") or 0) > 0:
            out.add(str(row.get("owner_key")))
    return out


def candidate_from_vector(
    p,
    owner_kind: str,
    owner_label: str,
    owner_addr: int,
    header_owner_addr: int,
    offset: int,
    shape: dict[str, Any],
    interesting_vtables: set[int],
    positive_rows: int,
) -> dict[str, Any]:
    sample = pc.sample_pointer_vector(
        p,
        int(shape["data"]),
        int(shape["count"]),
        interesting_vtables,
        max_entries=48,
    )
    status = pc.candidate_status_from_sample(sample, positive_rows)
    return {
        "owner_kind": owner_kind,
        "owner_label": owner_label,
        "owner_addr": pc.hx(owner_addr),
        "header_owner_addr": pc.hx(header_owner_addr),
        "header_offset": pc.hx(offset),
        "shape": {
            "kind": shape["kind"],
            "data": pc.hx(int(shape["data"])),
            "count": shape["count"],
            "capacity": shape["capacity"],
        },
        "pointer_sample": sample,
        "status": status,
        "promotion": "not_promoted_candidate_only",
    }


def scan_owner_vectors(
    p,
    owner_kind: str,
    owner_label: str,
    owner_addr: int,
    scan_bytes: int,
    interesting_vtables: set[int],
    positive_rows: int,
    max_candidates: int,
) -> list[dict[str, Any]]:
    candidates: list[dict[str, Any]] = []
    for offset in range(0, scan_bytes, 8):
        for shape in pc.vector_shapes_at(p, owner_addr, offset):
            cand = candidate_from_vector(
                p,
                owner_kind,
                owner_label,
                owner_addr,
                owner_addr,
                offset,
                shape,
                interesting_vtables,
                positive_rows,
            )
            if cand["pointer_sample"]["heap_pointer_entries"] > 0:
                candidates.append(cand)
        ptr = pc.try_u64(p, owner_addr + offset)
        if not pc.heap_ok(ptr):
            continue
        ptr_vt = pc.try_u64(p, int(ptr))
        ptr_rtti = pc.rtti_name_for_vtable(p, ptr_vt)
        inner_has_interesting_name = ptr_rtti and any(term in ptr_rtti.lower() for term in EVENT_LIKE_TERMS)
        for inner_offset in range(0, 0x180, 8):
            for shape in pc.vector_shapes_at(p, int(ptr), inner_offset):
                cand = candidate_from_vector(
                    p,
                    owner_kind,
                    owner_label,
                    owner_addr,
                    int(ptr),
                    inner_offset,
                    shape,
                    interesting_vtables,
                    positive_rows,
                )
                cand["source_pointer_field"] = pc.hx(offset)
                cand["source_pointer_rtti"] = ptr_rtti
                cand["source_pointer_vtable_rva"] = pc.hx(ptr_vt - p.module_base) if isinstance(ptr_vt, int) else None
                if cand["pointer_sample"]["interesting_rtti_entries"] > 0 or inner_has_interesting_name:
                    candidates.append(cand)
        if len(candidates) >= max_candidates:
            break
    candidates.sort(
        key=lambda row: (
            -int(row["pointer_sample"].get("interesting_rtti_entries", 0)),
            -int(row["shape"].get("count", 0)),
            row["owner_kind"],
            row["owner_label"],
            row["header_offset"] or "",
        )
    )
    return candidates[:max_candidates]


def build_summary(report: dict[str, Any], positive_rows: int) -> dict[str, Any]:
    candidates = report.get("candidates", [])
    interesting = [
        row for row in candidates
        if int(row.get("pointer_sample", {}).get("interesting_rtti_entries", 0)) > 0
    ]
    if positive_rows <= 0:
        status = "blocked_no_positive_oracle"
    elif interesting:
        status = "candidate_found_needs_fixture_match"
    elif candidates:
        status = "candidates_rejected_no_event_like_rtti"
    else:
        status = "no_candidate_found"
    return {
        "status": status,
        "positive_oracle_country_delayed_events": positive_rows,
        "candidate_count": len(candidates),
        "event_like_candidate_count": len(interesting),
        "promotion": "none",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--oracle", default=None, help="Delayed-event oracle JSON from build_delayed_event_oracle.py")
    parser.add_argument("--out", required=True)
    parser.add_argument("--max-countries", type=int, default=512)
    parser.add_argument("--scan-country-bytes", type=pc.parse_int, default=0x4000)
    parser.add_argument("--scan-gamestate-bytes", type=pc.parse_int, default=0x2000)
    parser.add_argument("--max-candidates", type=int, default=160)
    args = parser.parse_args()

    p, fail_report, code = pc.open_process_with_sha_or_report(PROBE)
    if fail_report is not None:
        pc.write_json(Path(args.out), fail_report)
        return code
    assert p is not None

    oracle = pc.read_json(Path(args.oracle)) if args.oracle else None
    positive_rows = oracle_positive_rows(oracle)
    owner_keys = oracle_owner_keys(oracle)

    try:
        started = time.time()
        info = pc.process_info(p)
        roots = pc.resolve_country_roots(p, max_countries=args.max_countries)
        rtti = pc.rediscover_rtti_by_terms(p, RTTI_TERMS)
        interesting_vtables = pc.rtti_vtable_values(rtti, EVENT_LIKE_TERMS)

        candidates: list[dict[str, Any]] = []
        candidates.extend(
            scan_owner_vectors(
                p,
                "CGameState",
                "world",
                roots["game_state"],
                args.scan_gamestate_bytes,
                interesting_vtables,
                positive_rows,
                args.max_candidates,
            )
        )
        per_country_limit = max(4, args.max_candidates // max(1, len(roots["validated_countries"])))
        for row in roots["validated_countries"]:
            label = "%s:%s" % (row["tag_db_id"], row.get("tag") or "?")
            country_candidates = scan_owner_vectors(
                p,
                "CCountry",
                label,
                int(row["country_ptr"]),
                args.scan_country_bytes,
                interesting_vtables,
                positive_rows,
                per_country_limit,
            )
            for cand in country_candidates:
                cand["oracle_owner_key_match"] = (
                    str(row["tag_db_id"]) in owner_keys or str(row.get("tag") or "") in owner_keys
                )
            candidates.extend(country_candidates)
            if len(candidates) >= args.max_candidates:
                break

        candidates.sort(
            key=lambda row: (
                -int(row.get("pointer_sample", {}).get("interesting_rtti_entries", 0)),
                0 if row.get("oracle_owner_key_match") else 1,
                -int(row.get("shape", {}).get("count", 0)),
            )
        )
        report: dict[str, Any] = {
            "schema": "event_delayed_event_queue_live_probe/v1",
            "probe": PROBE,
            "expected_sha256": pc.EXPECTED_SHA256,
            "process": info,
            "fail_closed": False,
            "started_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(started)),
            "elapsed_seconds": round(time.time() - started, 3),
            "source_roots": {
                "CGameState": pc.hx(roots["game_state"]),
                "CGameState_global": "module+0x%X" % pc.CGAMESTATE_GLOBAL_RVA,
                "country_table": pc.hx(roots["country_table"]),
                "country_count": roots["country_count"],
                "validated_country_rows": len(roots["validated_countries"]),
                "country_vtable": pc.hx(roots["country_vtable"]),
            },
            "oracle": {
                "path": args.oracle,
                "positive_country_delayed_events": positive_rows,
                "positive_owner_keys": sorted(owner_keys),
            },
            "rtti": pc.compact_rtti_report(rtti),
            "scan_policy": {
                "gamestate_bytes": pc.hx(args.scan_gamestate_bytes),
                "country_bytes": pc.hx(args.scan_country_bytes),
                "promotion_policy": "candidate_only_no_root_promotion",
            },
            "candidates": candidates[: args.max_candidates],
            "rejected_proxies": [
                {
                    "candidate": "saved_event_target",
                    "status": "rejected_proxy",
                    "reason": "not a per-country delayed_event queue",
                },
                {
                    "candidate": "zero-row fixture",
                    "status": "blocked_not_rejected",
                    "reason": "empty fixture cannot disprove delayed-event queues",
                },
            ],
        }
        report["summary"] = build_summary(report, positive_rows)
        report["completed_utc"] = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
        pc.write_json(Path(args.out), report)
        print(json.dumps(report["summary"], indent=2, sort_keys=True))
        print(args.out)
        return 0
    finally:
        p.close()


if __name__ == "__main__":
    raise SystemExit(main())
