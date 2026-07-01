#!/usr/bin/env python3
"""Fixture-independent naval root audit for domains 49/50.

This audits naval roots that do not require an active CNavalCombat body:
CShip damage/counter telemetry, persistent CShip -> CTaskForce/CFleet linkage,
static region derivation, and bounded mission/theatre/home-base candidates.
It writes a lane-owned proof JSON and does not edit shared entrypoints.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import sys
from collections import Counter
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional

LANE_DIR = Path(__file__).resolve().parents[1]
REPO_ROOT = Path(__file__).resolve().parents[5]
MEMORY_READER = REPO_ROOT / "extractor" / "memory_reader"
NAVAL_OOB_LANE = REPO_ROOT / "extractor/native" / "lanes" / "naval_oob_fleets_ships_live"

for path in (REPO_ROOT, MEMORY_READER, NAVAL_OOB_LANE):
    text = str(path)
    if text not in sys.path:
        sys.path.insert(0, text)

import winmem as wm  # noqa: E402
import naval_oob_gap_probe as naval_oob  # noqa: E402


def utc_now() -> str:
    return dt.datetime.utcnow().replace(microsecond=0).isoformat() + "Z"


def hx(value: Optional[int]) -> Optional[str]:
    if value is None:
        return None
    return "0x%X" % int(value)


def safe_u32(proc: wm.Process, addr: int) -> Optional[int]:
    return naval_oob.safe_u32(proc, addr)


def write_json(path: Path, payload: Dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def summarize_ships(ships: List[Dict[str, Any]]) -> Dict[str, Any]:
    instance_ids = [int(row["instance_id"]) for row in ships]
    duplicate_ids = [
        instance_id for instance_id, count in Counter(instance_ids).items() if count > 1
    ]
    missing_task_force = [row for row in ships if not row.get("task_force_id")]
    missing_fleet = [row for row in ships if not row.get("fleet_id")]
    missing_strength = [row for row in ships if row.get("strength_raw") is None]
    missing_org = [row for row in ships if row.get("organisation_raw") is None]
    missing_xp = [row for row in ships if row.get("experience_raw") is None]
    missing_sunk = [row for row in ships if row.get("sunk_convoys") is None]
    sunk_nonzero = [row for row in ships if int(row.get("sunk_convoys") or 0) > 0]
    xp_nonzero = [row for row in ships if int(row.get("experience_raw") or 0) > 0]
    strength_values = [int(row["strength_raw"]) for row in ships if row.get("strength_raw") is not None]
    org_values = [int(row["organisation_raw"]) for row in ships if row.get("organisation_raw") is not None]
    categories = Counter(int(row.get("category_enum") or 0) for row in ships)
    return {
        "ship_count": len(ships),
        "unique_instance_id_count": len(set(instance_ids)),
        "duplicate_instance_ids": duplicate_ids[:32],
        "task_force_linked_ship_count": len(ships) - len(missing_task_force),
        "fleet_linked_ship_count": len(ships) - len(missing_fleet),
        "missing_task_force_count": len(missing_task_force),
        "missing_fleet_count": len(missing_fleet),
        "strength_present_count": len(ships) - len(missing_strength),
        "organisation_present_count": len(ships) - len(missing_org),
        "experience_present_count": len(ships) - len(missing_xp),
        "sunk_convoys_present_count": len(ships) - len(missing_sunk),
        "experience_nonzero_count": len(xp_nonzero),
        "sunk_convoys_nonzero_count": len(sunk_nonzero),
        "strength_min_raw": min(strength_values) if strength_values else None,
        "strength_max_raw": max(strength_values) if strength_values else None,
        "organisation_min_raw": min(org_values) if org_values else None,
        "organisation_max_raw": max(org_values) if org_values else None,
        "category_counts": dict(sorted(categories.items())),
        "sample_sunk_convoys_nonzero": sample_ship_rows(sunk_nonzero, 16),
        "sample_experience_nonzero": sample_ship_rows(xp_nonzero, 16),
        "sample_low_strength": sample_ship_rows(
            sorted(ships, key=lambda row: int(row.get("strength_raw") or 0))[:16],
            16,
        ),
    }


def sample_ship_rows(rows: Iterable[Dict[str, Any]], limit: int) -> List[Dict[str, Any]]:
    out = []
    for row in list(rows)[:limit]:
        out.append(
            {
                "instance_id": row.get("instance_id"),
                "task_force_id": row.get("task_force_id"),
                "fleet_id": row.get("fleet_id"),
                "owner_tag_db_id": row.get("owner_tag_db_id"),
                "body_ptr": hx(row.get("body_ptr")),
                "category_enum": row.get("category_enum"),
                "strength_raw": row.get("strength_raw"),
                "organisation_raw": row.get("organisation_raw"),
                "experience_raw": row.get("experience_raw"),
                "sunk_convoys": row.get("sunk_convoys"),
            }
        )
    return out


def summarize_mission_candidates(mission: Dict[str, Any]) -> Dict[str, Any]:
    candidates = mission.get("candidate_offsets") or []
    all_zero = [
        row
        for row in candidates
        if row.get("unique_value_count") == 1
        and row.get("top_values")
        and int(row["top_values"][0][0]) == 0
    ]
    varied = [row for row in candidates if row not in all_zero]
    return {
        "status": mission.get("status"),
        "verdict": mission.get("verdict"),
        "task_force_count": mission.get("task_force_count"),
        "scan_range": mission.get("scan_range"),
        "candidate_offset_count": len(candidates),
        "all_zero_candidate_count": len(all_zero),
        "varied_candidate_count": len(varied),
        "sample_varied_candidates": varied[:24],
        "sample_all_zero_candidates": all_zero[:24],
    }


def summarize_home_scan(scan: Dict[str, Any]) -> Dict[str, Any]:
    return {
        "object_kind": scan.get("object_kind"),
        "object_count": scan.get("object_count"),
        "scan_span": scan.get("scan_span"),
        "province_pointer_offset_count": len(scan.get("province_pointer_offsets") or []),
        "province_id_u32_offset_count": len(scan.get("province_id_u32_offsets") or []),
        "sample_province_pointer_offsets": (scan.get("province_pointer_offsets") or [])[:16],
        "sample_province_id_u32_offsets": (scan.get("province_id_u32_offsets") or [])[:16],
    }


def summarize_theatre_scan(scan: Dict[str, Any]) -> Dict[str, Any]:
    return {
        "object_kind": scan.get("object_kind"),
        "object_count": scan.get("object_count"),
        "scan_span": scan.get("scan_span"),
        "target_type": scan.get("target_type"),
        "target_vtable_rva": scan.get("target_vtable_rva"),
        "offset_count": len(scan.get("offsets") or []),
        "offsets": (scan.get("offsets") or [])[:16],
    }


def validate_task_force_locations(proc: wm.Process, roots: Dict[str, Any]) -> Dict[str, Any]:
    rows = []
    missing = []
    for task_force in roots.get("task_forces") or []:
        body = int(task_force["body_ptr"])
        location = safe_u32(proc, body + naval_oob.TASK_FORCE_OFF_LOCATION_PROVINCE)
        if not location:
            missing.append(task_force.get("task_force_id"))
            continue
        rows.append(
            {
                "task_force_id": task_force.get("task_force_id"),
                "fleet_id": task_force.get("fleet_id"),
                "owner_tag_db_id": task_force.get("owner_tag_db_id"),
                "body_ptr": hx(body),
                "location_province_id": location,
            }
        )
    return {
        "offset": "CTaskForce+0x%X" % naval_oob.TASK_FORCE_OFF_LOCATION_PROVINCE,
        "task_force_count": len(roots.get("task_forces") or []),
        "location_present_count": len(rows),
        "missing_location_count": len(missing),
        "sample_rows": rows[:32],
        "missing_task_force_ids": missing[:32],
    }


def run(args: argparse.Namespace) -> Dict[str, Any]:
    proc = wm.Process(args.pid, "hoi4.exe")
    try:
        roots = naval_oob.discover_compact_naval_roots(proc)
        ships = naval_oob.sample_ship_rows(proc, roots)
        mission = naval_oob.scan_mission_candidates(proc, roots)
        home_tf = naval_oob.scan_province_candidates(
            proc,
            roots["module_base"],
            roots["task_forces"],
            "task_force",
            naval_oob.TASK_FORCE_SCAN_SPAN,
        )
        home_fleet = naval_oob.scan_province_candidates(
            proc,
            roots["module_base"],
            roots["fleets"],
            "fleet",
            naval_oob.COMPACT_FLEET_SCAN_SPAN,
        )
        theatre_fleet = naval_oob.scan_vtable_pointer_candidates(
            proc,
            roots["module_base"],
            roots["fleets"],
            "fleet",
            naval_oob.COMPACT_FLEET_SCAN_SPAN,
            naval_oob.THEATRE_VTABLE_RVA,
            naval_oob.OOB_TYPE_THEATRE,
        )
        theatre_tf = naval_oob.scan_vtable_pointer_candidates(
            proc,
            roots["module_base"],
            roots["task_forces"],
            "task_force",
            naval_oob.TASK_FORCE_SCAN_SPAN,
            naval_oob.THEATRE_VTABLE_RVA,
            naval_oob.OOB_TYPE_THEATRE,
        )
        region_rollups = naval_oob.derive_region_rollups(proc, roots, ships)
        task_force_locations = validate_task_force_locations(proc, roots)
        ship_summary = summarize_ships(ships)
        theatre_rejected = not theatre_fleet.get("offsets") and not theatre_tf.get("offsets")
        direct_home_rejected = not home_tf.get("province_pointer_offsets") and not home_fleet.get(
            "province_pointer_offsets"
        )
        return {
            "audit": "naval_combat_live_fixture_independent_roots_v1",
            "created_utc": utc_now(),
            "status": "passed",
            "pid": args.pid,
            "process": {
                "module": proc.module_name,
                "module_base": hx(proc.module_base),
                "module_size": proc.module_size,
                "module_path": proc.module_path,
                "sha256": proc.fingerprint().get("sha256"),
            },
            "validated_roots": {
                "CGameState": "module+0x%X -> %s"
                % (naval_oob.CG_STATE_DATE_OFFSET, hx(roots["game_state"])),
                "country_table": "CGameState+0x%X -> %s count=%s"
                % (
                    naval_oob.CGAMESTATE_COUNTRY_TABLE,
                    hx(roots["country_table"]),
                    roots["country_count"],
                ),
                "CCountry_vtable": "module+0x%X" % naval_oob.COUNTRY_VTABLE_RVA,
                "CFleet_vtable": "module+0x%X" % naval_oob.FLEET_VTABLE_RVA,
                "CTaskForce_vtable": "module+0x%X" % naval_oob.TASK_FORCE_VTABLE_RVA,
                "CShip_vtable": "module+0x%X" % naval_oob.CSHIP_VTABLE_RVA,
                "CShip_handle": "CShip+0x%X packed {type=51,instance_id}"
                % naval_oob.CSHIP_OFF_HANDLE,
                "CShip_strength_raw": "CShip+0x%X" % naval_oob.CSHIP_OFF_STRENGTH,
                "CShip_organisation_raw": "CShip+0x%X" % naval_oob.CSHIP_OFF_ORGANISATION,
                "CShip_experience_raw": "CShip+0x%X" % naval_oob.CSHIP_OFF_EXPERIENCE,
                "CShip_sunk_convoys": "CShip+0x%X" % naval_oob.CSHIP_OFF_SUNK_CONVOYS,
                "CTaskForce_location": "CTaskForce+0x%X"
                % naval_oob.TASK_FORCE_OFF_LOCATION_PROVINCE,
                "CTaskForce_parent_fleet": "CTaskForce+0x%X"
                % naval_oob.TASK_FORCE_OFF_FLEET_PARENT,
                "CTaskForce_ship_vector": "CTaskForce+0x%X/+0x%X"
                % (naval_oob.TASK_FORCE_OFF_SHIP_BEGIN, naval_oob.TASK_FORCE_OFF_SHIP_COUNT),
            },
            "counts": {
                "countries": len(roots.get("countries") or []),
                "fleets": len(roots.get("fleets") or []),
                "task_forces": len(roots.get("task_forces") or []),
                "ships": len(ships),
            },
            "ship_damage_and_counter_roots": ship_summary,
            "task_force_locations": task_force_locations,
            "strategic_region_rollups": {
                "status": region_rollups.get("status"),
                "task_force_count": region_rollups.get("task_force_count"),
                "task_forces_with_region": region_rollups.get("task_forces_with_region"),
                "missing_region_task_forces": region_rollups.get("missing_region_task_forces"),
                "sample_task_force_region_rows": (region_rollups.get("task_force_region_rows") or [])[
                    :32
                ],
            },
            "mission_order_candidates": summarize_mission_candidates(mission),
            "home_base_port_candidates": {
                "status": "rejected_direct_province_pointer_scan"
                if direct_home_rejected
                else "blocked_fixture_required",
                "verdict": "no direct CProvince* home-base pointer found in CFleet[0x0..0x1800] or CTaskForce[0x0..0x800]; raw province-id candidates need a port-change fixture"
                if direct_home_rejected
                else "direct CProvince* candidate exists but needs a port-change fixture",
                "task_force_scan": summarize_home_scan(home_tf),
                "fleet_scan": summarize_home_scan(home_fleet),
            },
            "theatre_linkage": {
                "status": "rejected_direct_pointer_scan" if theatre_rejected else "blocked_fixture_required",
                "verdict": "no direct CTheatre pointer found in CFleet[0x0..0x1800] or CTaskForce[0x0..0x800]"
                if theatre_rejected
                else "direct CTheatre pointer candidate exists but needs theatre assignment semantics fixture",
                "fleet_scan": summarize_theatre_scan(theatre_fleet),
                "task_force_scan": summarize_theatre_scan(theatre_tf),
            },
            "remaining_fixture_requirements": [
                {
                    "field": "CNavalCombat lifecycle/members/engagement participants",
                    "fixture": "visible active two-sided naval battle with at least one non-convoy warship member",
                    "reason": "no CNavalCombat body or CFEXMember rows exist outside an active battle",
                },
                {
                    "field": "ship sinking/deletion from battle",
                    "fixture": "active naval battle where a tracked combat member sinks or leaves before/at battle end",
                    "reason": "persistent CShip list exposes live ships and sunk-convoy counters, not battle-scoped deletion semantics",
                },
                {
                    "field": "task-force mission/repair/retreat/order enum semantics",
                    "fixture": "before/after controlled task-force mission, repair, retreat, or hold-order change on the same task_force_id",
                    "reason": "single snapshot produces bounded small-u32 candidates but cannot assign replay semantics",
                },
                {
                    "field": "home-base/port semantics",
                    "fixture": "before/after controlled home-base or port reassignment on the same fleet/task_force_id",
                    "reason": "direct CProvince* scan is rejected; raw province-id candidates need a transition fixture",
                },
            ],
        }
    finally:
        proc.close()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pid", type=int, required=True)
    parser.add_argument("--out-dir", type=Path, required=True)
    args = parser.parse_args()
    args.out_dir.mkdir(parents=True, exist_ok=True)
    report = run(args)
    out_path = args.out_dir / "fixture_independent_roots_audit.json"
    write_json(out_path, report)
    print(
        json.dumps(
            {
                "out_path": str(out_path),
                "status": report.get("status"),
                "ships": report.get("counts", {}).get("ships"),
                "task_forces": report.get("counts", {}).get("task_forces"),
                "sunk_convoys_nonzero_count": report.get(
                    "ship_damage_and_counter_roots", {}
                ).get("sunk_convoys_nonzero_count"),
                "theatre_status": report.get("theatre_linkage", {}).get("status"),
                "home_base_status": report.get("home_base_port_candidates", {}).get("status"),
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0 if report.get("status") == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
