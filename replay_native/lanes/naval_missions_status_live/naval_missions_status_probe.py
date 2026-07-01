#!/usr/bin/env python3
"""Lane-local live proof probe for naval mission/status domain 56.

This probe is bounded discovery/validation only. It reuses the already
validated naval OOB root traversal and does not claim hot-path extraction for
mission, home-base, repair, retreat, or convoy-route semantics without the
required fixture diffs.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import time
from collections import Counter, defaultdict
from typing import Any, Dict, Iterable, List, Optional, Tuple

LANE_DIR = os.path.dirname(os.path.abspath(__file__))
REPO_ROOT = os.path.abspath(os.path.join(LANE_DIR, "..", "..", ".."))
NAVAL_OOB_LANE = os.path.join(REPO_ROOT, "replay_native", "lanes", "naval_oob_fleets_ships_live")
if NAVAL_OOB_LANE not in sys.path:
    sys.path.insert(0, NAVAL_OOB_LANE)

import naval_oob_gap_probe as oob  # noqa: E402


DOMAIN_ID = 56
DOMAIN_NAME = "naval_missions_status_live"

MISSION_HOME_REPAIR_RETREAT_SCAN_SPAN = oob.TASK_FORCE_SCAN_SPAN
MAX_SCALAR_VALUE = 256


def now_ms() -> int:
    return int(time.time() * 1000)


def classify_scalar_offset(offset: int) -> str:
    if offset in (0x08, 0x0C):
        return "object_handle"
    if offset == oob.TASK_FORCE_OFF_LOCATION_PROVINCE:
        return "validated_current_location_province"
    if offset == oob.TASK_FORCE_OFF_SHIP_COUNT:
        return "validated_ship_vector_count"
    return "unclassified_status_or_counter_candidate"


def scan_task_force_status_scalars(proc: oob.wm.Process, roots: Dict[str, Any]) -> Dict[str, Any]:
    offsets: Dict[int, Counter] = defaultdict(Counter)
    examples: Dict[int, List[Dict[str, Any]]] = defaultdict(list)
    object_count = 0
    for task_force in roots["task_forces"]:
        object_count += 1
        body = task_force["body_ptr"]
        blob = oob.safe_read(proc, body, MISSION_HOME_REPAIR_RETREAT_SCAN_SPAN)
        if blob is None:
            continue
        for offset in range(0, max(0, len(blob) - 4), 4):
            value = oob.u32_from(blob, offset)
            if value is None or value > MAX_SCALAR_VALUE:
                continue
            offsets[offset][value] += 1
            if len(examples[offset]) < 6:
                examples[offset].append(
                    {
                        "task_force_id": task_force["task_force_id"],
                        "fleet_id": task_force["fleet_id"],
                        "offset": "0x%X" % offset,
                        "value": value,
                    }
                )

    rows = []
    for offset, counts in offsets.items():
        total = sum(counts.values())
        if total < max(2, object_count // 4):
            continue
        classification = classify_scalar_offset(offset)
        rows.append(
            {
                "offset": "0x%X" % offset,
                "classification": classification,
                "coverage_ratio": round(total / object_count, 4) if object_count else 0.0,
                "count": total,
                "unique_value_count": len(counts),
                "top_values": counts.most_common(10),
                "examples": examples[offset],
            }
        )
    rows.sort(
        key=lambda row: (
            0 if row["classification"].startswith("validated_") else 1,
            -row["coverage_ratio"],
            row["unique_value_count"],
            row["offset"],
        )
    )
    return {
        "status": "fixture_required_for_semantics",
        "verdict": (
            "bounded CTaskForce scalar scan found candidates, but mission, repair, and "
            "retreat meaning requires a controlled before/after UI fixture"
        ),
        "object_count": object_count,
        "scan_range": "CTaskForce[0x0..0x%X] u32 step 4" % MISSION_HOME_REPAIR_RETREAT_SCAN_SPAN,
        "candidate_offsets": rows[:60],
    }


def aggregate_sunk_convoys_by_task_force(ships: Iterable[Dict[str, Any]]) -> Dict[int, int]:
    totals: Dict[int, int] = defaultdict(int)
    for ship in ships:
        sunk = ship.get("sunk_convoys")
        if isinstance(sunk, int) and sunk > 0:
            totals[ship["task_force_id"]] += sunk
    return dict(totals)


def summarize_convoy_route_context(
    roots: Dict[str, Any], ships: List[Dict[str, Any]]
) -> Dict[str, Any]:
    sunk_by_tf = aggregate_sunk_convoys_by_task_force(ships)
    rows = []
    task_forces_by_id = {row["task_force_id"]: row for row in roots["task_forces"]}
    for task_force_id, sunk_convoys in sorted(sunk_by_tf.items()):
        task_force = task_forces_by_id.get(task_force_id, {})
        rows.append(
            {
                "task_force_id": task_force_id,
                "fleet_id": task_force.get("fleet_id"),
                "owner_tag_db_id": task_force.get("owner_tag_db_id"),
                "sunk_convoys": sunk_convoys,
            }
        )
    return {
        "status": "narrowed_to_existing_ship_counter_only",
        "verdict": (
            "CShip+0x3C sunk-convoy counts are reachable through the production ship roots; "
            "route/loss context was not reachable from fixture-independent fleet/task-force/ship roots"
        ),
        "ship_rows_sampled": len(ships),
        "task_forces_with_sunk_convoys": len(rows),
        "total_sunk_convoys_observed": sum(row["sunk_convoys"] for row in rows),
        "route_context_blocker": (
            "active convoy combat or a controlled convoy-loss route fixture is required to map "
            "losses to route/mission context rather than aggregate CShip counters"
        ),
        "sunk_convoy_rows": rows[:80],
    }


def domain56_registration_patch_plan() -> List[str]:
    return [
        "In replay_native/collector/src/domain_registry.rs, insert domain(56, \"naval_missions_status_live\", \"naval_missions_status_live\", CompactDomainStatus::Reserved or ActiveIntegration while wiring, false until collector hook is registered, true, &[]) between ids 54 and 1000.",
        "In replay_native/recorder_host/src/compact_domain_registry.rs, insert the matching descriptor for id 56 with canonical and wire name \"naval_missions_status_live\" and requires_data_record=true.",
        "In the shared collector, register one lane-owned domain module that emits domain 56 rows from the existing DLL coordinator after the current naval OOB roots are available; do not create a second collector or DLL.",
        "Use domain56_compact.rs as the wire contract: 44-byte header plus 64-byte task-force rows keyed by task_force_id with badged fixture-required fields until mission/home-base/repair/retreat offsets are selected by before/after fixture proof.",
        "In recorder_host/src/main.rs, add a domain 56 data decoder that records data_units_total keys emitted by domain56_compact.rs and preserves the raw compact stream inside .battleplan exactly like existing compact domains.",
    ]


def make_fixture_contracts() -> List[Dict[str, Any]]:
    probe = "python replay_native/lanes/naval_missions_status_live/naval_missions_status_probe.py"
    return [
        {
            "id": "domain56_task_force_mission_diff_v1",
            "purpose": "Select mission enum/state offsets from validated CTaskForce roots.",
            "commands": [
                probe + " --out replay_native/runs/naval_missions_status_live_870ebf_20260701_fixture_before/naval_missions_status_probe.json --snapshot-label before_mission_change",
                probe + " --out replay_native/runs/naval_missions_status_live_870ebf_20260701_fixture_after/naval_missions_status_probe.json --snapshot-label after_mission_change",
            ],
            "operator_steps_between_commands": [
                "Record one task_force_id and fleet_id from the before JSON.",
                "Change only that task force mission in the UI.",
                "Do not split, merge, rebase, or delete that task force between snapshots.",
            ],
            "acceptance": [
                "The same task_force_id and fleet_id appear in both snapshots.",
                "A candidate scalar offset changes only for the edited task force or the edited fleet group.",
                "The new value maps to the expected UI mission while unedited task forces remain stable.",
            ],
        },
        {
            "id": "domain56_home_base_repair_retreat_diff_v1",
            "purpose": "Resolve home-base, repair-base/progress, and retreat-state semantics.",
            "commands": [
                probe + " --out replay_native/runs/naval_missions_status_live_870ebf_20260701_base_repair_before/naval_missions_status_probe.json --snapshot-label before_base_repair_change",
                probe + " --out replay_native/runs/naval_missions_status_live_870ebf_20260701_base_repair_after/naval_missions_status_probe.json --snapshot-label after_base_repair_change",
            ],
            "operator_steps_between_commands": [
                "Change one selected task force or fleet home base to a known different naval-base province.",
                "If repair state is the target, damage or select a damaged ship and send it to repair without changing task-force identity.",
                "If retreat is the target, trigger a retreat state with a known selected task force and keep the task_force_id stable.",
            ],
            "acceptance": [
                "The selected task_force_id persists across both snapshots.",
                "A home-base candidate becomes the expected port/naval-base province or CProvince pointer, distinct from CTaskForce+0x18 current location.",
                "Repair/retreat candidates change only for the selected damaged/retreating task force and remain stable elsewhere.",
            ],
        },
        {
            "id": "domain56_convoy_route_context_fixture_v1",
            "purpose": "Map convoy losses to route/mission context instead of only aggregate ship sunk-convoy counters.",
            "commands": [
                probe + " --out replay_native/runs/naval_missions_status_live_870ebf_20260701_convoy_before/naval_missions_status_probe.json --snapshot-label before_convoy_loss",
                probe + " --out replay_native/runs/naval_missions_status_live_870ebf_20260701_convoy_after/naval_missions_status_probe.json --snapshot-label after_convoy_loss",
            ],
            "operator_steps_between_commands": [
                "Create or load an active convoy battle/loss fixture with known source and destination route context.",
                "Keep the involved task force and fleet identity stable when possible.",
            ],
            "acceptance": [
                "A route or battle context root changes with the known convoy loss event.",
                "The result links loss count to route/mission context, not only CShip+0x3C aggregate counters.",
            ],
        },
    ]


def build_summary(proc: oob.wm.Process, args: argparse.Namespace) -> Dict[str, Any]:
    started = now_ms()
    roots = oob.discover_compact_naval_roots(proc)
    ships = oob.sample_ship_rows(proc, roots)
    module_base = roots["module_base"]

    mission_candidates = oob.scan_mission_candidates(proc, roots)
    scalar_candidates = scan_task_force_status_scalars(proc, roots)
    home_base_tf = oob.scan_province_candidates(
        proc,
        module_base,
        roots["task_forces"],
        "task_force",
        oob.TASK_FORCE_SCAN_SPAN,
    )
    home_base_fleet = oob.scan_province_candidates(
        proc,
        module_base,
        roots["fleets"],
        "fleet",
        oob.COMPACT_FLEET_SCAN_SPAN,
    )
    region_rollups = oob.derive_region_rollups(proc, roots, ships)
    convoy_context = summarize_convoy_route_context(roots, ships)

    task_force_rows = []
    sunk_by_tf = aggregate_sunk_convoys_by_task_force(ships)
    region_by_tf = {
        row["task_force_id"]: row
        for row in region_rollups.get("task_force_region_rows", [])
        if isinstance(row.get("task_force_id"), int)
    }
    for task_force in roots["task_forces"][:120]:
        task_force_id = task_force["task_force_id"]
        location = oob.safe_u32(proc, task_force["body_ptr"] + oob.TASK_FORCE_OFF_LOCATION_PROVINCE)
        region_row = region_by_tf.get(task_force_id)
        flags = ["HAS_LOCATION", "VALIDATED_DIRECT", "FIXTURE_REQUIRED"]
        if region_row:
            flags.extend(["HAS_REGION", "DERIVED_REGION"])
        if sunk_by_tf.get(task_force_id):
            flags.append("HAS_SUNK_CONVOY_COUNT")
        task_force_rows.append(
            {
                "task_force_id": task_force_id,
                "fleet_id": task_force["fleet_id"],
                "owner_tag_db_id": task_force.get("owner_tag_db_id"),
                "location_province_id": location,
                "strategic_region_id": region_row.get("strategic_region_id") if region_row else None,
                "sunk_convoys": sunk_by_tf.get(task_force_id, 0),
                "domain56_flags": flags,
            }
        )

    home_base_pointer_hits = (
        len(home_base_tf.get("province_pointer_offsets", []))
        + len(home_base_fleet.get("province_pointer_offsets", []))
    )
    home_base_status = (
        "blocked_fixture_required"
        if home_base_pointer_hits
        else "rejected_direct_cprovince_pointer_scan_fixture_still_required_for_u32"
    )

    return {
        "schema_version": 1,
        "domain": {"id": DOMAIN_ID, "name": DOMAIN_NAME},
        "snapshot_label": args.snapshot_label,
        "generated_unix_ms": started,
        "duration_ms": now_ms() - started,
        "process": {
            "pid": proc.pid,
            "module": proc.module_name,
            "module_base": oob.hx(proc.module_base),
            "module_size": proc.module_size,
            "module_path": proc.module_path,
            "sha256": proc.fingerprint().get("sha256"),
        },
        "validated_roots": {
            "CGameState": "module+0x%X" % oob.CG_STATE_DATE_OFFSET,
            "country_table": "CGameState+0x%X count CGameState+0x%X"
            % (oob.CGAMESTATE_COUNTRY_TABLE, oob.CGAMESTATE_COUNTRY_COUNT),
            "fleet_vector": "CCountry+0x%X/+0x%X/+0x%X"
            % (
                oob.COUNTRY_FLEET_VECTOR_DATA,
                oob.COUNTRY_FLEET_VECTOR_CAPACITY,
                oob.COUNTRY_FLEET_VECTOR_COUNT,
            ),
            "CFleet_vtable": "module+0x%X" % oob.FLEET_VTABLE_RVA,
            "CTaskForce_vtable": "module+0x%X" % oob.TASK_FORCE_VTABLE_RVA,
            "CTaskForce_current_location": "CTaskForce+0x%X u32 current province"
            % oob.TASK_FORCE_OFF_LOCATION_PROVINCE,
            "CTaskForce_ship_vector": "CTaskForce+0x%X/+0x%X"
            % (oob.TASK_FORCE_OFF_SHIP_BEGIN, oob.TASK_FORCE_OFF_SHIP_COUNT),
            "CShip_sunk_convoys": "CShip+0x%X u32 aggregate sunk-convoy count"
            % oob.CSHIP_OFF_SUNK_CONVOYS,
        },
        "counts": {
            "countries": len(roots["countries"]),
            "fleets": len(roots["fleets"]),
            "task_forces": len(roots["task_forces"]),
            "ships": len(ships),
        },
        "domain56_compact_schema": {
            "file": os.path.join(LANE_DIR, "domain56_compact.rs"),
            "payload": "44-byte header plus 64-byte task-force rows",
            "row_key": "task_force_id",
            "validated_now": [
                "task_force_id",
                "fleet_id",
                "owner_tag_db_id",
                "location_province_id",
                "derived strategic_region_id when province map covers CTaskForce+0x18",
                "aggregate sunk_convoys from CShip+0x3C",
            ],
            "fixture_required": [
                "mission_enum",
                "mission_state",
                "home_base_province_id",
                "repair_base_province_id",
                "repair_progress_milli",
                "retreat_state",
                "convoy_route_context_id",
            ],
        },
        "field_results": {
            "naval_missions": {
                "status": "blocked_fixture_required",
                "verdict": mission_candidates["verdict"],
                "candidate_offsets": mission_candidates["candidate_offsets"][:30],
                "scalar_status_scan": scalar_candidates,
            },
            "home_base": {
                "status": home_base_status,
                "verdict": (
                    "CTaskForce+0x18 is validated current location; no home-base offset is promoted "
                    "without a controlled port-change fixture"
                ),
                "task_force_scan": home_base_tf,
                "fleet_scan": home_base_fleet,
            },
            "repair_retreat": {
                "status": "blocked_fixture_required",
                "verdict": (
                    "repair and retreat semantics need a damaged/repairing or retreating task-force "
                    "fixture; scalar candidates are not promoted from a single live snapshot"
                ),
                "scalar_status_scan": scalar_candidates,
            },
            "convoy_combat_loss_route_context": convoy_context,
            "strategic_region_rollups": region_rollups,
        },
        "domain56_sample_rows": task_force_rows,
        "master_row_change_proposals": [
            {
                "row": "Naval missions",
                "proposed_status": "FIXTURE_BLOCKED",
                "proposed_evidence_domain": "`naval_missions_status_live` / 56 lane proof",
                "proposed_remaining_closure_condition": (
                    "Controlled task-force mission before/after fixture to select exact mission enum/state offsets; "
                    "domain-56 compact schema and validated task-force/location roots are lane-local."
                ),
            },
            {
                "row": "Home base/repair/retreat",
                "proposed_status": "FIXTURE_BLOCKED",
                "proposed_evidence_domain": "`naval_missions_status_live` / 56 lane proof",
                "proposed_remaining_closure_condition": (
                    "Controlled port-change, damaged-repairing, and retreating task-force fixtures; "
                    "do not promote CTaskForce+0x18 because it is current location, not home base."
                ),
            },
            {
                "row": "Convoy combat/losses",
                "proposed_status": "FIXTURE_BLOCKED",
                "proposed_evidence_domain": "`naval_missions_status_live` / 56 lane proof plus `ships_live` sunk-convoy counter",
                "proposed_remaining_closure_condition": (
                    "Active convoy battle/loss route fixture to link losses to route/mission context; "
                    "CShip+0x3C remains only aggregate sunk-convoy count."
                ),
            },
        ],
        "fixture_contracts": make_fixture_contracts(),
        "registration_patch_plan": domain56_registration_patch_plan(),
    }


def write_json(path: str, value: Dict[str, Any]) -> None:
    os.makedirs(os.path.dirname(os.path.abspath(path)), exist_ok=True)
    with open(path, "w", encoding="utf-8") as handle:
        json.dump(value, handle, indent=2, sort_keys=True)
        handle.write("\n")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", default=None)
    parser.add_argument("--snapshot-label", default="single_live_snapshot")
    args = parser.parse_args()

    proc = oob.wm.open_process("hoi4.exe")
    if proc is None:
        raise SystemExit("hoi4.exe is not running")
    try:
        summary = build_summary(proc, args)
    finally:
        proc.close()

    if args.out:
        write_json(args.out, summary)
        print("wrote %s" % args.out)
    else:
        print(json.dumps(summary, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
