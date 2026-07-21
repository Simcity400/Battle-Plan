#!/usr/bin/env python3
"""Bounded active-fixture harness for naval combat domains 49/50.

This lane-owned wrapper waits for the first validated CNavalCombat fixture,
promotes the validated front+0xA4 battle location (4/4 match vs the same-tick
D04 save oracle) to the compact lifecycle location_id, joins it to region_id
via the game's static province->strategic-region map, enriches member rows with
persistent CShip task-force/fleet data, then hands an exact JSON fixture to the
Rust compact proof binary.
Shared collector/recorder entrypoints are intentionally untouched.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import re
import subprocess
import sys
import time
from pathlib import Path
from types import SimpleNamespace
from typing import Any, Dict, Iterable, List, Optional, Tuple

LANE_DIR = Path(__file__).resolve().parents[1]
REPO_ROOT = Path(__file__).resolve().parents[5]
MEMORY_READER = REPO_ROOT / "extractor" / "memory_reader"
COMBAT_READER = MEMORY_READER / "combat"
NAVAL_OOB_LANE = REPO_ROOT / "extractor/native" / "lanes" / "naval_oob_fleets_ships_live"

for path in (REPO_ROOT, MEMORY_READER, COMBAT_READER, NAVAL_OOB_LANE):
    text = str(path)
    if text not in sys.path:
        sys.path.insert(0, text)

import winmem as wm  # noqa: E402
from naval_combat_live_fixture_probe import (  # noqa: E402
    encode_bridge,
    encode_member_bridge_jsonl,
    run as run_fixture_probe,
)
import naval_oob_gap_probe as naval_oob  # noqa: E402


TERMINAL_NON_FIXTURE_STATUSES = {
    "build_mismatch",
    "vtable_rtti_mismatch",
    "scan_failed",
    "scan_truncated",
    "candidate_rejected",
    "candidate_rejected_cship_member_join",
    "validated_member_rows_missing_cship_fixture",
}


def utc_now() -> str:
    return dt.datetime.utcnow().replace(microsecond=0).isoformat() + "Z"


def write_json(path: Path, payload: Dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def compact_poll_summary(report: Dict[str, Any], path: Path) -> Dict[str, Any]:
    scan = report.get("scan") or {}
    root = report.get("root") or {}
    return {
        "path": str(path),
        "created_utc": report.get("created_utc"),
        "status": report.get("status"),
        "sha256_ok": root.get("sha256_ok"),
        "body_hit_count": int(report.get("body_hit_count") or 0),
        "validated_combat_count": int(report.get("validated_combat_count") or 0),
        "bridge_row_count": int(report.get("bridge_row_count") or 0),
        "member_bridge_row_count": int(report.get("member_bridge_row_count") or 0),
        "cship_resolved_member_count": int(report.get("cship_resolved_member_count") or 0),
        "cship_unresolved_member_count": int(report.get("cship_unresolved_member_count") or 0),
        "scan_status": scan.get("status"),
        "scan_elapsed_s": scan.get("elapsed_s"),
    }


def run_probe(args: argparse.Namespace) -> Dict[str, Any]:
    probe_args = SimpleNamespace(
        pid=args.pid,
        workers=args.workers,
        require_two_sided=args.require_two_sided,
        require_cship_join=args.require_cship_join,
        out_dir=None,
    )
    return run_fixture_probe(probe_args)


def detect_game_root(report: Dict[str, Any], override: Optional[Path]) -> Path:
    if override is not None:
        return override
    module_path = ((report.get("root") or {}).get("module_path") or "").strip()
    if not module_path:
        raise ValueError("fixture report does not include root.module_path; pass --game-root")
    return Path(module_path).resolve().parent


def load_province_regions(game_root: Path) -> Dict[int, int]:
    strategic_dir = game_root / "map" / "strategicregions"
    if not strategic_dir.is_dir():
        raise ValueError(f"strategic region directory not found: {strategic_dir}")
    mapping: Dict[int, int] = {}
    for path in strategic_dir.glob("*.txt"):
        text = strip_comments(path.read_text(encoding="utf-8", errors="ignore"))
        region_match = re.search(r"\bid\s*=\s*(\d+)", text)
        provinces_match = re.search(r"\bprovinces\s*=\s*\{([^}]*)\}", text, re.S)
        if not region_match or not provinces_match:
            continue
        region_id = int(region_match.group(1))
        for province_text in re.findall(r"\d+", provinces_match.group(1)):
            mapping.setdefault(int(province_text), region_id)
    if not mapping:
        raise ValueError(f"no province-to-region rows parsed from {strategic_dir}")
    return mapping


def strip_comments(text: str) -> str:
    return "\n".join(line.split("#", 1)[0] for line in text.splitlines())


def collect_oob_ships(pid: int) -> Tuple[List[Dict[str, Any]], Dict[str, Any]]:
    proc = wm.Process(pid, "hoi4.exe")
    try:
        roots = naval_oob.discover_compact_naval_roots(proc)
        ships = naval_oob.sample_ship_rows(proc, roots)
        summary = {
            "validated_roots": {
                "CGameState": "module+0x%X" % naval_oob.CG_STATE_DATE_OFFSET,
                "country_table": "CGameState+0x%X count=%s"
                % (naval_oob.CGAMESTATE_COUNTRY_TABLE, roots.get("country_count")),
                "CFleet_vtable": "module+0x%X" % naval_oob.FLEET_VTABLE_RVA,
                "CTaskForce_vtable": "module+0x%X" % naval_oob.TASK_FORCE_VTABLE_RVA,
                "CShip_vtable": "module+0x%X" % naval_oob.CSHIP_VTABLE_RVA,
                "CTaskForce_ship_vector": "CTaskForce+0x%X/+0x%X"
                % (naval_oob.TASK_FORCE_OFF_SHIP_BEGIN, naval_oob.TASK_FORCE_OFF_SHIP_COUNT),
            },
            "counts": {
                "countries": len(roots.get("countries") or []),
                "fleets": len(roots.get("fleets") or []),
                "task_forces": len(roots.get("task_forces") or []),
                "ships": len(ships),
            },
        }
        return ships, summary
    finally:
        proc.close()


def build_fixture_json(
    report: Dict[str, Any],
    *,
    oob_ships: Iterable[Dict[str, Any]],
    oob_summary: Dict[str, Any],
    province_regions: Dict[int, int],
    game_hour_raw: int,
) -> Tuple[Dict[str, Any], List[str]]:
    errors: List[str] = []
    ships_by_id = {int(row["instance_id"]): row for row in oob_ships}
    member_rows = report.get("member_bridge_rows") or []
    body_ptr_by_combat = parse_bridge_text(str(report.get("bridge") or ""))
    members_by_combat: Dict[int, List[Dict[str, Any]]] = {}
    for row in member_rows:
        combat_id = int(row.get("combat_id") or 0)
        members_by_combat.setdefault(combat_id, []).append(row)
        if row.get("body_ptr") is not None:
            body_ptr_by_combat.setdefault(combat_id, int(row["body_ptr"]))

    snapshots: List[Dict[str, Any]] = []
    cships_by_id: Dict[int, Dict[str, Any]] = {}
    location_rows: List[Dict[str, Any]] = []
    for combat in report.get("validated_combats") or []:
        if combat.get("status") != "valid":
            continue
        combat_id = int(combat["combat_id"])
        # PROMOTED: front+0xA4 battle location (validated 4/4 vs the same-tick
        # D04 save oracle). Absent/zero location or an unmapped province is a
        # fail-closed error, never a silently absent compact field.
        location_id = optional_int(combat.get("location_front_0xA4"))
        region_id: Optional[int] = None
        if location_id is None:
            errors.append(f"combat {combat_id} is missing promoted front+0xA4 location")
        else:
            region_id = province_regions.get(location_id)
            if region_id is None:
                errors.append(
                    f"combat {combat_id} location province {location_id} has no strategic-region mapping"
                )
        location_rows.append(
            {
                "combat_id": combat_id,
                "body_ptr": body_ptr_by_combat.get(combat_id),
                "location_front_0xA4": location_id,
                "region_id": region_id,
                "validated": True,
            }
        )
        body_ptr = body_ptr_by_combat.get(combat_id)
        if body_ptr is None:
            errors.append(f"combat {combat_id} is missing body pointer in bridge/member manifest")
        attacker = combat.get("attacker") or {}
        defender = combat.get("defender") or {}
        rows = members_by_combat.get(combat_id, [])
        if not rows:
            errors.append(f"combat {combat_id} has no flattened member_bridge rows")
        snapshots.append(
            {
                "lifecycle": {
                    "combat_id": combat_id,
                    "generation": 1,
                    "body_ptr": body_ptr or 0,
                    "location_id": location_id,
                    "region_id": region_id,
                    "attacker_member_count": int(attacker.get("member_count") or 0),
                    "defender_member_count": int(defender.get("member_count") or 0),
                    "attacker_positioning_raw": optional_i32(
                        attacker.get("positioning_raw")
                    ),
                    "defender_positioning_raw": optional_i32(
                        defender.get("positioning_raw")
                    ),
                    "attacker_anti_air_raw": optional_i64(
                        attacker.get("anti_air_raw")
                    ),
                    "defender_anti_air_raw": optional_i64(
                        defender.get("anti_air_raw")
                    ),
                },
                "members": [member_fixture_row(row) for row in rows],
            }
        )
        for row in rows:
            if bool(row.get("is_convoy")):
                continue
            ship_instance_id = optional_int(row.get("ship_instance_id"))
            if not ship_instance_id:
                errors.append(f"combat {combat_id} non-convoy member has no ship_instance_id")
                continue
            if row.get("cship_status") != "resolved":
                errors.append(
                    f"combat {combat_id} ship_instance_id {ship_instance_id} cship_status={row.get('cship_status')}"
                )
                continue
            oob_ship = ships_by_id.get(ship_instance_id)
            if oob_ship is None:
                errors.append(
                    f"combat {combat_id} ship_instance_id {ship_instance_id} missing from CShip task-force index"
                )
                continue
            cships_by_id.setdefault(ship_instance_id, cship_fixture_row(row, oob_ship))

    payload = {
        "source": {
            "harness": "naval_combat_live_active_fixture_proof_v1",
            "created_utc": utc_now(),
            "fixture_report_status": report.get("status"),
            "fixture_report_created_utc": report.get("created_utc"),
            "fixture_probe": report.get("probe"),
            "root": report.get("root"),
            "build_sha256": (report.get("root") or {}).get("sha256"),
            "oob_summary": oob_summary,
            "location_front_0xA4": {
                "validated": True,
                "provenance": "front+0xA4 == save naval_combat.location for all 4 battles of the same-tick D04 1944-02-11 oracle (6434/2804/1032/8819); region_id joined via map/strategicregions",
                "rows": location_rows,
            },
        },
        "game_hour_raw": game_hour_raw,
        "elapsed_wall_ms": 1000,
        "snapshots": snapshots,
        "cships": sorted(cships_by_id.values(), key=lambda row: row["instance_id"]),
    }
    if not snapshots:
        errors.append("fixture report has no valid combat snapshots")
    if not cships_by_id:
        errors.append("fixture report has no non-convoy CShip rows with task-force/fleet linkage")
    return payload, errors


def parse_bridge_text(text: str) -> Dict[int, int]:
    rows: Dict[int, int] = {}
    for part in text.strip().split(";"):
        if not part:
            continue
        if ":" not in part:
            continue
        combat_text, ptr_text = part.split(":", 1)
        try:
            rows[int(combat_text, 10)] = int(ptr_text, 0)
        except ValueError:
            continue
    return rows


def member_fixture_row(row: Dict[str, Any]) -> Dict[str, Any]:
    ship_instance_id = optional_int(row.get("ship_instance_id"))
    if bool(row.get("is_convoy")) and ship_instance_id == 0:
        ship_instance_id = None
    return {
        "combat_id": int(row["combat_id"]),
        "generation": 1,
        "side": str(row["side"]),
        "group_index": int(row.get("group_index") or 0),
        "member_index": int(row.get("member_index") or 0),
        "member_ptr": int(row.get("member_ptr") or 0),
        "unit_type_tag": int(row.get("unit_type_tag") or 0),
        "ship_instance_id": ship_instance_id,
        "is_convoy": bool(row.get("is_convoy")),
        "owner_tag_db_id": optional_int(row.get("owner_tag_db_id")),
        "designed_max_strength_raw": optional_int(row.get("max_strength_raw")),
        "battle_slot_index_raw": int(row.get("slot_index") or 0),
        "state_bitmask": optional_int(row.get("state_bitmask")),
        "engagement_weight_raw": optional_i32(row.get("engagement_weight")),
    }


def cship_fixture_row(member_row: Dict[str, Any], oob_ship: Dict[str, Any]) -> Dict[str, Any]:
    instance_id = int(member_row["ship_instance_id"])
    return {
        "instance_id": instance_id,
        "body_ptr": int(oob_ship.get("body_ptr") or member_row.get("cship_ptr") or 0),
        "vtable_rva": naval_oob.CSHIP_VTABLE_RVA,
        "handle_type_tag": naval_oob.CSHIP_TYPE,
        "alias_instance_id": instance_id,
        "ref100_sentinel_raw": naval_oob.CSHIP_REF100_RAW,
        "task_force_id": optional_int(oob_ship.get("task_force_id")),
        "fleet_id": optional_int(oob_ship.get("fleet_id")),
        "strength_raw": int(member_row.get("cship_strength_raw") or oob_ship.get("strength_raw") or 0),
        "organisation_raw": int(
            member_row.get("cship_organisation_raw") or oob_ship.get("organisation_raw") or 0
        ),
        "experience_raw": int(member_row.get("cship_experience_raw") or oob_ship.get("experience_raw") or 0),
        "sunk_convoys": int(member_row.get("cship_sunk_convoys") or oob_ship.get("sunk_convoys") or 0),
        "category_enum": int(member_row.get("cship_category_enum") or oob_ship.get("category_enum") or 0),
    }


def optional_int(value: Any) -> Optional[int]:
    if value is None:
        return None
    try:
        parsed = int(value)
    except (TypeError, ValueError):
        return None
    return parsed if parsed != 0 else None


def optional_i32(value: Any) -> Optional[int]:
    parsed = optional_int(value)
    if parsed is None:
        return None
    if parsed >= 2**31:
        parsed -= 2**32
    return parsed


def optional_i64(value: Any) -> Optional[int]:
    parsed = optional_int(value)
    if parsed is None:
        return None
    if parsed >= 2**63:
        parsed -= 2**64
    return parsed


def run_rust_fixture_proof(fixture_json: Path, rust_out_dir: Path) -> Dict[str, Any]:
    command = [
        "cargo",
        "run",
        "--manifest-path",
        str(LANE_DIR / "Cargo.toml"),
        "--bin",
        "naval_combat_lane_proof",
        "--",
        "--out-dir",
        str(rust_out_dir),
        "--fixture-json",
        str(fixture_json),
    ]
    completed = subprocess.run(
        command,
        cwd=str(REPO_ROOT),
        text=True,
        capture_output=True,
        check=False,
    )
    return {
        "command": command,
        "returncode": completed.returncode,
        "stdout": completed.stdout,
        "stderr": completed.stderr,
        "out_dir": str(rust_out_dir),
        "proof_path": str(rust_out_dir / "naval_combat_live_lane_stack_proof.json"),
        "status": "passed" if completed.returncode == 0 else "failed",
    }


def run(args: argparse.Namespace) -> Dict[str, Any]:
    args.out_dir.mkdir(parents=True, exist_ok=True)
    deadline = time.monotonic() + args.timeout_s
    polls: List[Dict[str, Any]] = []
    poll_index = 0
    final_report: Optional[Dict[str, Any]] = None
    while True:
        report = run_probe(args)
        final_report = report
        poll_path = args.out_dir / ("poll_%04d.json" % poll_index)
        write_json(poll_path, report)
        polls.append(compact_poll_summary(report, poll_path))

        status = str(report.get("status") or "unknown")
        if status == "validated_active_fixture":
            return finish_validated_fixture(args, report, polls)
        if status in TERMINAL_NON_FIXTURE_STATUSES:
            break
        if time.monotonic() >= deadline:
            break
        poll_index += 1
        time.sleep(min(args.poll_s, max(0.0, deadline - time.monotonic())))

    return {
        "harness": "naval_combat_live_active_fixture_proof_v1",
        "created_utc": utc_now(),
        "status": "timeout_no_validated_fixture"
        if (final_report or {}).get("status") == "fixture_absent"
        else "stopped_without_validated_fixture",
        "pid": args.pid,
        "timeout_s": args.timeout_s,
        "poll_s": args.poll_s,
        "poll_count": len(polls),
        "polls": polls,
        "final_fixture_status": (final_report or {}).get("status"),
        "bridge_path": None,
        "member_bridge_path": None,
        "fixture_json_path": None,
        "rust_fixture_proof": {"status": "not_run", "reason": "no validated active fixture"},
    }


def finish_validated_fixture(
    args: argparse.Namespace,
    report: Dict[str, Any],
    polls: List[Dict[str, Any]],
) -> Dict[str, Any]:
    bridge_text = str(report.get("bridge") or "")
    if not bridge_text:
        bridge_text = encode_bridge(report.get("bridge_rows") or [])
    member_rows = report.get("member_bridge_rows") or []
    bridge_path = args.out_dir / "bridge.txt"
    member_bridge_path = args.out_dir / "member_bridge.jsonl"

    oob_ships, oob_summary = collect_oob_ships(args.pid)
    cship_index_path = args.out_dir / "cship_task_force_index.json"
    write_json(
        cship_index_path,
        {
            "created_utc": utc_now(),
            "status": "captured",
            "ship_count": len(oob_ships),
            "ships": oob_ships,
            "summary": oob_summary,
        },
    )

    # Static province -> strategic-region join for the promoted location field.
    # Any failure here must fail the fixture (fail-closed), not skip the field.
    try:
        game_root = detect_game_root(report, args.game_root)
        province_regions = load_province_regions(game_root)
        region_join_error: Optional[str] = None
    except (ValueError, OSError) as exc:
        province_regions = {}
        region_join_error = f"province-region join unavailable: {exc}"

    fixture_payload, fixture_errors = build_fixture_json(
        report,
        oob_ships=oob_ships,
        oob_summary=oob_summary,
        province_regions=province_regions,
        game_hour_raw=args.game_hour_raw,
    )
    if region_join_error:
        fixture_errors.append(region_join_error)
    fixture_json_path = args.out_dir / "active_fixture_input.json"
    write_json(fixture_json_path, fixture_payload)
    if fixture_errors:
        bridge_rejected_path = args.out_dir / "bridge.REJECTED.txt"
        member_bridge_rejected_path = args.out_dir / "member_bridge.REJECTED.jsonl"
        if bridge_text:
            bridge_rejected_path.write_text(bridge_text, encoding="ascii")
        if member_rows:
            member_bridge_rejected_path.write_text(
                encode_member_bridge_jsonl(member_rows),
                encoding="utf-8",
            )
        return {
            "harness": "naval_combat_live_active_fixture_proof_v1",
            "created_utc": utc_now(),
            "status": "validated_fixture_enrichment_failed",
            "pid": args.pid,
            "poll_count": len(polls),
            "polls": polls,
            "bridge_path": None,
            "member_bridge_path": None,
            "bridge_rejected_path": str(bridge_rejected_path) if bridge_text else None,
            "member_bridge_rejected_path": str(member_bridge_rejected_path) if member_rows else None,
            "fixture_json_path": str(fixture_json_path),
            "cship_task_force_index_path": str(cship_index_path),
            "fixture_errors": fixture_errors,
            "rust_fixture_proof": {"status": "not_run", "reason": "fixture enrichment failed"},
        }

    rust_out_dir = args.out_dir / "rust_fixture_proof"
    rust_report = run_rust_fixture_proof(fixture_json_path, rust_out_dir)
    if rust_report["returncode"] == 0:
        bridge_path.write_text(bridge_text, encoding="ascii")
        member_bridge_path.write_text(encode_member_bridge_jsonl(member_rows), encoding="utf-8")
        bridge_path_value: Optional[str] = str(bridge_path)
        member_bridge_path_value: Optional[str] = str(member_bridge_path)
        bridge_rejected_path_value: Optional[str] = None
        member_bridge_rejected_path_value: Optional[str] = None
    else:
        bridge_rejected_path = args.out_dir / "bridge.REJECTED.txt"
        member_bridge_rejected_path = args.out_dir / "member_bridge.REJECTED.jsonl"
        bridge_rejected_path.write_text(bridge_text, encoding="ascii")
        member_bridge_rejected_path.write_text(
            encode_member_bridge_jsonl(member_rows),
            encoding="utf-8",
        )
        bridge_path_value = None
        member_bridge_path_value = None
        bridge_rejected_path_value = str(bridge_rejected_path)
        member_bridge_rejected_path_value = str(member_bridge_rejected_path)
    return {
        "harness": "naval_combat_live_active_fixture_proof_v1",
        "created_utc": utc_now(),
        "status": "passed" if rust_report["returncode"] == 0 else "rust_fixture_proof_failed",
        "pid": args.pid,
        "timeout_s": args.timeout_s,
        "poll_s": args.poll_s,
        "poll_count": len(polls),
        "polls": polls,
        "bridge_path": bridge_path_value,
        "member_bridge_path": member_bridge_path_value,
        "bridge_rejected_path": bridge_rejected_path_value,
        "member_bridge_rejected_path": member_bridge_rejected_path_value,
        "fixture_json_path": str(fixture_json_path),
        "cship_task_force_index_path": str(cship_index_path),
        "rust_fixture_proof": rust_report,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pid", type=int, required=True)
    parser.add_argument("--workers", type=int, default=4)
    parser.add_argument("--timeout-s", type=float, default=900.0)
    parser.add_argument("--poll-s", type=float, default=10.0)
    parser.add_argument("--require-two-sided", action="store_true", default=True)
    parser.add_argument("--allow-one-sided", dest="require_two_sided", action="store_false")
    parser.add_argument("--require-cship-join", action=argparse.BooleanOptionalAction, default=True)
    parser.add_argument("--game-root", type=Path, default=None)
    parser.add_argument("--game-hour-raw", type=int, default=0)
    parser.add_argument("--out-dir", type=Path, required=True)
    args = parser.parse_args()

    if args.timeout_s <= 0:
        raise SystemExit("--timeout-s must be > 0")
    if args.poll_s <= 0:
        raise SystemExit("--poll-s must be > 0")

    report = run(args)
    summary_path = args.out_dir / "active_fixture_harness_summary.json"
    write_json(summary_path, report)
    print(
        json.dumps(
            {
                "summary_path": str(summary_path),
                "status": report.get("status"),
                "final_fixture_status": report.get("final_fixture_status"),
                "poll_count": report.get("poll_count"),
                "fixture_json_path": report.get("fixture_json_path"),
                "rust_fixture_proof": (report.get("rust_fixture_proof") or {}).get("status"),
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0 if report.get("status") in {"passed", "timeout_no_validated_fixture"} else 1


if __name__ == "__main__":
    raise SystemExit(main())
