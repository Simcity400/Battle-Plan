"""Focused live probe for the remaining construction target-state candidates.

The broad probe narrows candidates to construction subobject offsets +0xC0 and
+0xC8. This probe records those offsets for every live construction line and
checks whether the interpreted state id is owned or controlled by the line owner.
That owner/controller read is validation-only evidence; the lane still emits only
state/province join keys.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import time
from typing import Any

PROBE_DIR = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, PROBE_DIR)

import live_probe_870ebf as live  # type: ignore  # noqa: E402

STATE_OFF_OWNER = 0xC8
STATE_OFF_CTRL = 0xCC
FOCUS_OFFSETS = (0xC0, 0xC8)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", required=True)
    parser.add_argument("--max-regions", type=int, default=None)
    args = parser.parse_args()

    started = time.time()
    p = live.winmem.open_process("hoi4.exe")
    if p is None:
        raise SystemExit("hoi4.exe not running")
    with p:
        fingerprint = p.fingerprint()
        country_roots, country_meta = live.read_country_roots(p, max_regions=args.max_regions)
        lines = live.read_construction_lines(p, country_roots)
        topology = live.scan_cprovince_roots(p, max_regions=args.max_regions)
        state_details = read_state_details(p, topology)
        analyses = analyze_offsets(p, lines, state_details)
        fixture_request = build_fixture_request(p, lines, state_details)

    result = {
        "schema": "construction_buildings_state_damage_live_target_candidate_focus_v1",
        "build_profile": live.BUILD_PROFILE,
        "duration_ms": round((time.time() - started) * 1000),
        "fingerprint": fingerprint,
        "country_roots": {
            "resolver": country_meta.get("resolver"),
            "root_count": country_meta.get("root_count"),
            "country_vtable_rva": country_meta.get("country_vtable_rva"),
            "ok": country_meta.get("ok"),
        },
        "construction_queue": {
            "line_count": len(lines),
            "tag_db_id_count": len({line["tag_db_id"] for line in lines}),
            "template_id_count": len(
                {
                    line["template_id"]
                    for line in lines
                    if line.get("template_id") is not None
                }
            ),
        },
        "state_details": {
            "state_count": len(state_details),
            "owner_offset": hex(STATE_OFF_OWNER),
            "controller_offset": hex(STATE_OFF_CTRL),
        },
        "candidate_offsets": analyses,
        "fixture_request": fixture_request,
        "verdict": focus_verdict(analyses),
    }
    os.makedirs(os.path.dirname(os.path.abspath(args.out)), exist_ok=True)
    with open(args.out, "w", encoding="utf-8") as handle:
        json.dump(result, handle, indent=2, sort_keys=True)
        handle.write("\n")
    print(
        json.dumps(
            {
                "out": args.out,
                "construction_lines": len(lines),
                "verdict": result["verdict"],
                "duration_ms": result["duration_ms"],
            },
            sort_keys=True,
        )
    )
    return 0


def read_state_details(p: Any, topology: dict[str, Any]) -> dict[int, dict[str, int | str | None]]:
    out: dict[int, dict[str, int | str | None]] = {}
    for state_raw, ptr_raw in topology.get("state_by_id", {}).items():
        state_id = int(state_raw)
        state_ptr = int(ptr_raw, 16)
        out[state_id] = {
            "state_ptr": ptr_raw,
            "owner_tag_db_id": live.try_u32(p, state_ptr + STATE_OFF_OWNER),
            "controller_tag_db_id": live.try_u32(p, state_ptr + STATE_OFF_CTRL),
        }
    return out


def analyze_offsets(
    p: Any,
    lines: list[dict[str, Any]],
    state_details: dict[int, dict[str, int | str | None]],
) -> list[dict[str, Any]]:
    analyses: list[dict[str, Any]] = []
    for offset in FOCUS_OFFSETS:
        rows = []
        for line in lines:
            sub_ptr_raw = line.get("sub_ptr")
            if not sub_ptr_raw:
                continue
            sub_ptr = int(sub_ptr_raw, 16)
            raw = live.try_u32(p, sub_ptr + offset)
            if raw is None:
                continue
            detail = state_details.get(raw)
            owner = detail.get("owner_tag_db_id") if detail else None
            controller = detail.get("controller_tag_db_id") if detail else None
            rows.append(
                {
                    "tag_db_id": line["tag_db_id"],
                    "line_id": line["line_id"],
                    "priority": line["priority"],
                    "template_id": line.get("template_id"),
                    "raw_value": raw,
                    "state_exists": detail is not None,
                    "candidate_state_id": raw if detail else None,
                    "candidate_owner_tag_db_id": owner,
                    "candidate_controller_tag_db_id": controller,
                    "owner_matches_line_tag": owner == line["tag_db_id"],
                    "controller_matches_line_tag": controller == line["tag_db_id"],
                    "owner_or_controller_matches_line_tag": (
                        owner == line["tag_db_id"] or controller == line["tag_db_id"]
                    ),
                    "raw_equals_line_tag_db_id": raw == line["tag_db_id"],
                }
            )
        analyses.append(summarize_offset(offset, rows))
    return analyses


def summarize_offset(offset: int, rows: list[dict[str, Any]]) -> dict[str, Any]:
    state_rows = [row for row in rows if row["state_exists"]]
    match_rows = [row for row in state_rows if row["owner_or_controller_matches_line_tag"]]
    raw_equals_tag = [row for row in rows if row["raw_equals_line_tag_db_id"]]
    unique_raw = sorted({row["raw_value"] for row in rows})
    unique_state = sorted({row["candidate_state_id"] for row in state_rows if row["candidate_state_id"]})
    unique_owner = sorted(
        {
            row["candidate_owner_tag_db_id"]
            for row in state_rows
            if row["candidate_owner_tag_db_id"] is not None
        }
    )
    unique_controller = sorted(
        {
            row["candidate_controller_tag_db_id"]
            for row in state_rows
            if row["candidate_controller_tag_db_id"] is not None
        }
    )
    verdict = "fixture_blocked"
    if state_rows and not match_rows:
        verdict = "rejected_no_owner_or_controller_match"
    elif raw_equals_tag and len(raw_equals_tag) == len(rows):
        verdict = "rejected_raw_equals_line_tag_db_id"
    return {
        "base": "sub",
        "offset": hex(offset),
        "kind": "raw_state_id_or_u64_low_state_id",
        "line_count": len(rows),
        "state_existing_line_count": len(state_rows),
        "owner_or_controller_match_count": len(match_rows),
        "raw_equals_line_tag_count": len(raw_equals_tag),
        "unique_raw_count": len(unique_raw),
        "unique_state_count": len(unique_state),
        "unique_owner_tag_count": len(unique_owner),
        "unique_controller_tag_count": len(unique_controller),
        "unique_raw_values": unique_raw[:64],
        "unique_state_ids": unique_state[:64],
        "unique_owner_tag_ids": unique_owner[:64],
        "unique_controller_tag_ids": unique_controller[:64],
        "mismatch_samples": [
            row
            for row in state_rows
            if not row["owner_or_controller_matches_line_tag"]
        ][:16],
        "match_samples": match_rows[:16],
        "verdict": verdict,
        "rejection_reason": (
            "candidate state id never has owner/controller equal to the construction line owner"
            if verdict == "rejected_no_owner_or_controller_match"
            else None
        ),
    }


def focus_verdict(analyses: list[dict[str, Any]]) -> str:
    rejected = [item for item in analyses if str(item.get("verdict", "")).startswith("rejected")]
    if len(rejected) == len(analyses) and analyses:
        return "remaining_target_state_candidates_rejected"
    if rejected:
        return "remaining_target_state_candidates_partially_rejected"
    return "remaining_target_state_candidates_fixture_blocked"


def build_fixture_request(
    p: Any,
    lines: list[dict[str, Any]],
    state_details: dict[int, dict[str, int | str | None]],
) -> dict[str, Any]:
    rows = []
    seen: set[tuple[int, int, int]] = set()
    for selector in (
        lambda row: not row["candidates"]["sub+0xC0"]["owner_or_controller_matches_line_tag"]
        and not row["candidates"]["sub+0xC8"]["owner_or_controller_matches_line_tag"],
        lambda row: row["candidates"]["sub+0xC0"]["owner_or_controller_matches_line_tag"],
        lambda row: row["candidates"]["sub+0xC8"]["owner_or_controller_matches_line_tag"],
    ):
        for row in focus_rows(p, lines, state_details):
            key = (row["tag_db_id"], row["line_id"], row["priority"])
            if key in seen or not selector(row):
                continue
            seen.add(key)
            rows.append(row)
            if len(rows) >= 8:
                break
        if len(rows) >= 8:
            break
    return {
        "schema_to_fill": "construction_buildings_state_damage_live_fixture_contract_v1",
        "purpose": "validate_or_reject_remaining_construction_target_state_candidates",
        "required_truth_source": (
            "time-aligned save production/construction queue or equivalent UI oracle with "
            "expected target state for each listed tag_db_id/line_id/priority/template row"
        ),
        "acceptance_rule": (
            "For each row, fill expected_target_state_id. If expected_target_state_id matches "
            "sub+0xC0 on distinguishing rows, promote +0xC0; if it matches sub+0xC8, promote "
            "+0xC8; if neither candidate matches across the fixture, reject the remaining "
            "candidate interpretations and keep target_state_id absent."
        ),
        "minimum_rows": 4,
        "candidate_offsets": ["sub+0xC0", "sub+0xC8"],
        "rows": rows,
    }


def focus_rows(
    p: Any,
    lines: list[dict[str, Any]],
    state_details: dict[int, dict[str, int | str | None]],
) -> list[dict[str, Any]]:
    out = []
    for line in lines:
        sub_ptr_raw = line.get("sub_ptr")
        if not sub_ptr_raw:
            continue
        sub_ptr = int(sub_ptr_raw, 16)
        candidates = {}
        for offset in FOCUS_OFFSETS:
            candidates[f"sub+0x{offset:X}"] = candidate_detail(
                live.try_u32(p, sub_ptr + offset),
                line["tag_db_id"],
                state_details,
            )
        out.append(
            {
                "tag_db_id": line["tag_db_id"],
                "line_id": line["line_id"],
                "priority": line["priority"],
                "template_id": line.get("template_id"),
                "active_factories": line.get("active_factories"),
                "cost_raw": line.get("cost_raw"),
                "produced_raw": line.get("produced_raw"),
                "line_ptr": line.get("line_ptr"),
                "sub_ptr": line.get("sub_ptr"),
                "candidates": candidates,
                "expected_target_state_id": "FILL_FROM_TIME_ALIGNED_SAVE_OR_UI_ORACLE",
            }
        )
    return out


def candidate_detail(
    raw: int | None,
    line_tag_db_id: int,
    state_details: dict[int, dict[str, int | str | None]],
) -> dict[str, Any]:
    detail = state_details.get(raw) if raw is not None else None
    owner = detail.get("owner_tag_db_id") if detail else None
    controller = detail.get("controller_tag_db_id") if detail else None
    return {
        "raw_value": raw,
        "candidate_state_id": raw if detail else None,
        "state_exists": detail is not None,
        "candidate_owner_tag_db_id": owner,
        "candidate_controller_tag_db_id": controller,
        "owner_or_controller_matches_line_tag": (
            owner == line_tag_db_id or controller == line_tag_db_id
        ),
    }


if __name__ == "__main__":
    raise SystemExit(main())
