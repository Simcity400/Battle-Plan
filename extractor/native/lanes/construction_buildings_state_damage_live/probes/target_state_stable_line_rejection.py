"""Reject construction target-state candidates using stable construction-line IDs.

This is deliberately separate from the same-frame oracle. It accepts a later
save only for rows where the construction line id still exists as a type-59
building line, the cost matches, and the save exposes a numeric
building.location. Under the game construction model, a construction line target
location is fixed for the lifetime of that line; progress/factory/priority can
change while the target does not.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


SCHEMA = "construction_buildings_state_damage_live_target_state_stable_line_rejection_v2"

STATE_LOCATION_TEMPLATES = {
    "air_base",
    "anti_air_building",
    "arms_factory",
    "dockyard",
    "fuel_silo",
    "industrial_complex",
    "infrastructure",
    "nuclear_reactor",
    "radar_station",
    "rocket_site",
    "synthetic_refinery",
}

PROVINCE_LOCATION_TEMPLATES = {
    "bunker",
    "coastal_bunker",
    "naval_base",
    "rail_way",
    "supply_node",
}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--focus-proof", required=True)
    parser.add_argument("--oracle", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--min-rows", type=int, default=4)
    parser.add_argument("--min-unique-targets", type=int, default=2)
    args = parser.parse_args()

    focus_path = Path(args.focus_proof)
    oracle_path = Path(args.oracle)
    with focus_path.open(encoding="utf-8") as handle:
        focus = json.load(handle)
    with oracle_path.open(encoding="utf-8") as handle:
        oracle = json.load(handle)

    proof = build_proof(
        focus=focus,
        focus_path=focus_path,
        oracle=oracle,
        oracle_path=oracle_path,
        min_rows=args.min_rows,
        min_unique_targets=args.min_unique_targets,
    )
    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    with out_path.open("w", encoding="utf-8") as handle:
        json.dump(proof, handle, indent=2, sort_keys=True)
        handle.write("\n")

    print(
        json.dumps(
            {
                "out": str(out_path),
                "status": proof["status"],
                "accepted_stable_rows": proof["accepted_stable_rows"],
                "state_target_rejected_rows": proof["state_target_rejection"]["row_count"],
                "unique_state_building_targets": proof["state_target_rejection"][
                    "unique_state_building_targets"
                ],
                "exact_location_rejected_rows": proof["exact_location_rejection"]["row_count"],
            },
            sort_keys=True,
        )
    )
    return 0 if proof["status"].startswith("rejected_") else 2


def build_proof(
    *,
    focus: dict[str, Any],
    focus_path: Path,
    oracle: dict[str, Any],
    oracle_path: Path,
    min_rows: int,
    min_unique_targets: int,
) -> dict[str, Any]:
    if focus.get("schema") != "construction_buildings_state_damage_live_target_candidate_focus_v1":
        raise SystemExit(f"unexpected focus proof schema {focus.get('schema')!r}")

    accepted_rows = []
    state_target_rejected = []
    exact_location_rejected = []
    province_target_unmapped = []
    inconclusive = []
    for row in oracle.get("unmatched_required_rows") or []:
        save = row.get("save_match") or {}
        accepted, reason = stable_line_accepted(row, save)
        if not accepted:
            inconclusive.append(row_summary(row, save, reason, [], []))
            continue
        accepted_rows.append(row)
        target_state_candidates = candidate_state_ids(row)
        exact_location_candidates = candidate_exact_location_ids(row)
        save_location = save.get("location_state")

        if (
            isinstance(save_location, int)
            and save_location > 0
            and save_location not in exact_location_candidates
        ):
            exact_location_rejected.append(
                row_summary(
                    row,
                    save,
                    "save_location_differs_from_all_candidate_raw_values",
                    target_state_candidates,
                    exact_location_candidates,
                )
            )
        else:
            inconclusive.append(
                row_summary(
                    row,
                    save,
                    "save_location_matches_candidate_or_missing",
                    target_state_candidates,
                    exact_location_candidates,
                )
            )

        template = save.get("template")
        if template in STATE_LOCATION_TEMPLATES:
            if save_location not in target_state_candidates:
                state_target_rejected.append(
                    row_summary(
                        row,
                        save,
                        "state_building_target_state_differs_from_all_candidates",
                        target_state_candidates,
                        exact_location_candidates,
                    )
                )
            else:
                inconclusive.append(
                    row_summary(
                        row,
                        save,
                        "state_building_target_state_matches_candidate",
                        target_state_candidates,
                        exact_location_candidates,
                    )
                )
        elif template in PROVINCE_LOCATION_TEMPLATES:
            province_target_unmapped.append(
                row_summary(
                    row,
                    save,
                    "province_building_location_requires_province_to_state_map",
                    target_state_candidates,
                    exact_location_candidates,
                )
            )
        else:
            inconclusive.append(
                row_summary(
                    row,
                    save,
                    "unknown_building_location_semantics",
                    target_state_candidates,
                    exact_location_candidates,
                )
            )

    unique_state_targets = sorted(
        {
            item["save_location_state"]
            for item in state_target_rejected
            if isinstance(item.get("save_location_state"), int)
        }
    )
    exact_unique_locations = sorted(
        {
            item["save_location_state"]
            for item in exact_location_rejected
            if isinstance(item.get("save_location_state"), int)
        }
    )
    state_rejection_passed = (
        len(state_target_rejected) >= min_rows
        and len(unique_state_targets) >= min_unique_targets
        and not any(
            item["verdict"] == "state_building_target_state_matches_candidate"
            for item in inconclusive
        )
    )
    exact_location_rejection_passed = (
        len(exact_location_rejected) == len(accepted_rows)
        and len(exact_unique_locations) >= min_unique_targets
    )
    status = (
        "rejected_state_building_target_candidates_and_exact_location_candidates"
        if state_rejection_passed and exact_location_rejection_passed
        else "inconclusive"
    )
    return {
        "schema": SCHEMA,
        "status": status,
        "source_focus_proof": str(focus_path).replace("\\", "/"),
        "source_oracle": str(oracle_path).replace("\\", "/"),
        "oracle_kind": "stable_line_later_save",
        "assumption": (
            "For a type-59 construction line, building.location is fixed for the "
            "line lifetime. Produced progress, assigned factories, and priority may "
            "change between the live proof and later save."
        ),
        "save_location_semantics": {
            "state_location_templates": sorted(STATE_LOCATION_TEMPLATES),
            "province_location_templates": sorted(PROVINCE_LOCATION_TEMPLATES),
            "note": (
                "Save construction building.location is a state id for state-building "
                "templates and a province id for province-building templates. This "
                "proof therefore rejects target-state candidates only from state-building "
                "rows, while exact building.location roots can be rejected for every "
                "accepted stable row."
            ),
        },
        "requirements": {
            "minimum_rows": min_rows,
            "minimum_unique_state_building_targets": min_unique_targets,
            "accepted_identity": [
                "line_id equal by oracle row match",
                "save line_type == 59",
                "cost_raw equal",
                "save building.location is numeric",
            ],
            "candidate_interpretations_checked": [
                "sub+0xC0.raw_state_id",
                "sub+0xC0.raw_province_id_mapped_state",
                "sub+0xC8.raw_state_id",
                "sub+0xC8.raw_province_id_mapped_state",
            ],
        },
        "focus_summary": {
            "build_profile": focus.get("build_profile"),
            "construction_lines": focus.get("construction_queue", {}).get("line_count"),
            "tag_db_id_count": focus.get("construction_queue", {}).get("tag_db_id_count"),
            "template_id_count": focus.get("construction_queue", {}).get("template_id_count"),
            "candidate_offsets": summarize_candidate_offsets(focus),
        },
        "accepted_stable_rows": len(accepted_rows),
        "state_target_rejection": {
            "passed": state_rejection_passed,
            "row_count": len(state_target_rejected),
            "unique_state_building_targets": unique_state_targets,
            "rows": state_target_rejected,
        },
        "exact_location_rejection": {
            "passed": exact_location_rejection_passed,
            "row_count": len(exact_location_rejected),
            "unique_save_locations": exact_unique_locations,
            "rows": exact_location_rejected,
        },
        "province_target_state_unmapped_rows": province_target_unmapped,
        "inconclusive_rows": inconclusive,
        "result_detail": (
            "Accepted state-building rows have save building.location state ids "
            "different from every emitted +0xC0/+0xC8 state interpretation, and all "
            "accepted stable rows have save building.location values different from "
            "the raw +0xC0/+0xC8 candidate values."
            if status.startswith("rejected_")
            else "Stable-line oracle did not meet row/target thresholds or had inconclusive rows."
        ),
    }


def stable_line_accepted(row: dict[str, Any], save: dict[str, Any]) -> tuple[bool, str]:
    if save.get("line_type") != 59:
        return False, "save_line_type_not_59"
    if not isinstance(save.get("location_state"), int) or save["location_state"] <= 0:
        return False, "save_missing_numeric_location_state"
    if save.get("cost_raw") != row.get("cost_raw"):
        return False, "cost_raw_mismatch"
    if save.get("line_id") != row.get("line_id"):
        return False, "line_id_mismatch"
    return True, "accepted_stable_line"


def candidate_state_ids(row: dict[str, Any]) -> list[int]:
    states = []
    for key in ("sub+0xC0", "sub+0xC8"):
        candidate = (row.get("candidates") or {}).get(key) or {}
        for field in ("candidate_state_id", "candidate_mapped_state_id"):
            value = candidate.get(field)
            if isinstance(value, int) and value > 0 and value not in states:
                states.append(value)
        interpretations = candidate.get("candidate_interpretations") or {}
        for item in interpretations.values():
            value = item.get("candidate_state_id") if isinstance(item, dict) else None
            if isinstance(value, int) and value > 0 and value not in states:
                states.append(value)
    return states


def candidate_exact_location_ids(row: dict[str, Any]) -> list[int]:
    locations = []
    for key in ("sub+0xC0", "sub+0xC8"):
        candidate = (row.get("candidates") or {}).get(key) or {}
        for field in ("raw_value", "candidate_state_id", "candidate_province_id"):
            value = candidate.get(field)
            if isinstance(value, int) and value > 0 and value not in locations:
                locations.append(value)
    return locations


def row_summary(
    row: dict[str, Any],
    save: dict[str, Any],
    verdict: str,
    target_state_candidates: list[int],
    exact_location_candidates: list[int],
) -> dict[str, Any]:
    return {
        "verdict": verdict,
        "tag": row.get("tag"),
        "tag_db_id": row.get("tag_db_id"),
        "line_id": row.get("line_id"),
        "priority_live": row.get("priority"),
        "priority_save": save.get("priority"),
        "template_id": row.get("template_id"),
        "save_template": save.get("template"),
        "cost_raw": row.get("cost_raw"),
        "produced_raw_live": row.get("produced_raw"),
        "produced_raw_save": save.get("produced_raw"),
        "active_factories_live": row.get("active_factories"),
        "active_factories_save": save.get("active_factories"),
        "save_location_state": save.get("location_state"),
        "candidate_state_ids_checked": target_state_candidates,
        "candidate_exact_location_ids_checked": exact_location_candidates,
        "candidate_detail": row.get("candidates"),
    }


def summarize_candidate_offsets(focus: dict[str, Any]) -> list[dict[str, Any]]:
    out = []
    for item in focus.get("candidate_offsets") or []:
        out.append(
            {
                "offset": item.get("offset"),
                "line_count": item.get("line_count"),
                "state_existing_line_count": item.get("state_existing_line_count"),
                "province_existing_line_count": item.get("province_existing_line_count"),
                "owner_or_controller_match_count": item.get("owner_or_controller_match_count"),
                "province_mapped_owner_or_controller_match_count": item.get(
                    "province_mapped_owner_or_controller_match_count"
                ),
                "unique_state_ids": item.get("unique_state_ids"),
                "unique_mapped_state_ids": item.get("unique_mapped_state_ids"),
                "verdict": item.get("verdict"),
            }
        )
    return out


if __name__ == "__main__":
    raise SystemExit(main())
