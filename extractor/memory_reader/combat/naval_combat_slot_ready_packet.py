"""Build a slot-ready packet for the naval_combat_live integration slot.

This stays lane-local. It consumes accepted proof artifacts and emits an exact
collector/recorder patch plan, fixture blocker, and proof command list. It does
not edit shared collector or recorder entrypoints.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import re
import sys
from pathlib import Path
from typing import Any, Dict, List, Optional

THIS_DIR = Path(__file__).resolve().parent
REPO_ROOT = THIS_DIR.parent.parent
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from memory_reader.combat.naval_combat_compact_lane import schema_manifest  # noqa: E402

EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
CSHIP_VTABLE_RVA = "module+0x2941748"
STALE_CSHIP_VTABLE_RVA = "module+0x2931998"
DOMAIN_NAVAL_COMBAT_LIFECYCLE = 49
DOMAIN_NAVAL_COMBAT_MEMBERS = 50
DOMAIN_NAVAL_COMBAT_LIFECYCLE_NAME = "naval_combat_lifecycle_live"
DOMAIN_NAVAL_COMBAT_MEMBERS_NAME = "naval_combat_members_live"


def load_json(path: Path) -> Dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def validate_cship_route(report: Dict[str, Any]) -> Dict[str, Any]:
    errors: List[str] = []
    root = report.get("root") or {}
    old_candidate = report.get("old_candidate") or {}
    current_candidate = report.get("current_candidate") or {}
    old_validation = old_candidate.get("validation") or {}
    old_scan = old_candidate.get("scan") or {}
    current_validation = current_candidate.get("validation") or {}
    current_scan = current_candidate.get("scan") or {}

    if root.get("sha256") != EXPECTED_SHA256 or not root.get("sha256_ok"):
        errors.append("cship proof build sha mismatch")
    if old_validation.get("verdict") != "rejected_no_vtable_hits":
        errors.append("stale cship rva was not rejected")
    if int(old_scan.get("hit_count") if old_scan.get("hit_count") is not None else -1) != 0:
        errors.append("stale cship rva had live hits")
    if current_validation.get("verdict") != "validated":
        errors.append("current cship route was not validated")
    hit_count = int(current_scan.get("hit_count") or 0)
    validated_count = int(current_validation.get("validated_count") or 0)
    unique_count = int(current_validation.get("validated_unique_instance_ids") or 0)
    invalid_count = int(current_validation.get("invalid_count") or 0)
    if hit_count <= 0:
        errors.append("current cship scan found no rows")
    if validated_count != hit_count or unique_count != validated_count:
        errors.append("current cship row/id counts do not match")
    if invalid_count != 0:
        errors.append("current cship route has invalid rows")
    if current_validation.get("duplicate_instance_ids"):
        errors.append("current cship route has duplicate instance ids")
    if current_validation.get("error_counts"):
        errors.append("current cship route has validation errors")

    return {
        "ok": not errors,
        "errors": errors,
        "root": root,
        "stale_cship_rva": {
            "rva": STALE_CSHIP_VTABLE_RVA,
            "hit_count": old_scan.get("hit_count"),
            "verdict": old_validation.get("verdict"),
        },
        "current_cship_route": {
            "rva": CSHIP_VTABLE_RVA,
            "hit_count": hit_count,
            "validated_count": validated_count,
            "validated_unique_instance_ids": unique_count,
            "invalid_count": invalid_count,
            "duplicate_instance_ids": current_validation.get("duplicate_instance_ids") or [],
        },
        "offsets": report.get("offsets_tested") or {},
    }


def fixture_status(path: Optional[Path]) -> Dict[str, Any]:
    if path is None:
        return {
            "status": "blocked_fixture_not_supplied",
            "ready": False,
            "bridge_row_count": 0,
            "requirements": fixture_requirements(),
        }
    if not path.exists():
        return {
            "status": "blocked_fixture_report_missing",
            "ready": False,
            "path": str(path),
            "bridge_row_count": 0,
            "requirements": fixture_requirements(),
        }
    report = load_json(path)
    ready = report.get("status") == "validated_active_fixture" and int(report.get("bridge_row_count") or 0) > 0
    return {
        "status": "fixture_ready" if ready else "blocked_fixture_absent_or_unvalidated",
        "ready": ready,
        "path": str(path),
        "fixture_probe_status": report.get("status"),
        "body_hit_count": report.get("body_hit_count"),
        "validated_combat_count": report.get("validated_combat_count"),
        "bridge_row_count": report.get("bridge_row_count"),
        "requirements": fixture_requirements(),
    }


def fixture_requirements() -> List[str]:
    return [
        "Use current 870ebf hoi4.exe process.",
        "Create an active visible fleet-vs-fleet naval battle before recorder attach.",
        "Require non-empty attacker and defender CFEXMember rows.",
        "Require at least one non-convoy warship member with unit_type_tag=51 and ship_instance_id>0 to exercise the CShip join.",
        "Convoy-only defender rows can validate lifecycle/member enumeration but are insufficient to validate the CShip member resolver for that side.",
        "Fixture probe must return status=validated_active_fixture and bridge_row_count>=1.",
        "Fixture probe must emit member_bridge.jsonl with member_bridge_row_count>=2 and cship_unresolved_member_count==0.",
    ]


def domain_allocation_audit() -> Dict[str, Any]:
    shared_files = [
        REPO_ROOT / "extractor/native" / "collector" / "src" / "lib.rs",
        REPO_ROOT / "extractor/native" / "recorder_host" / "src" / "main.rs",
    ]
    domain_ids = [44, 45, 46, 47, 48, 49, 50]
    usage: Dict[int, List[Dict[str, Any]]] = {domain_id: [] for domain_id in domain_ids}
    id_pattern = "|".join(str(domain_id) for domain_id in domain_ids)
    compact_patterns = [
        re.compile(r"COMPACT_DOMAIN_[A-Z0-9_]+\s*:\s*u16\s*=\s*(%s)\b" % id_pattern),
        re.compile(r"^\s*(%s)\s*=>\s*decode_compact" % id_pattern),
        re.compile(r"add_compact_domain_data_unit\(summary,\s*(%s)\b" % id_pattern),
        re.compile(r"per_domain\.get_mut\(&\s*(%s)\)" % id_pattern),
    ]
    for path in shared_files:
        if not path.exists():
            continue
        lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
        for line_no, line in enumerate(lines, 1):
            for pattern in compact_patterns:
                match = pattern.search(line)
                if match:
                    domain_id = int(match.group(1))
                    usage[domain_id].append({
                        "file": str(path.relative_to(REPO_ROOT)),
                        "line": line_no,
                        "text": line.strip(),
                    })
                    break
    return {
        "source": "shared-file compact-domain scan plus orchestrator allocation update",
        "shared_files_scanned": [str(path.relative_to(REPO_ROOT)) for path in shared_files],
        "occupied": {
            "44": {
                "owner": "supply_logistics_attrition_live",
                "evidence": usage[44],
            },
            "45": {
                "owner": "focus_research_decisions_depth_live",
                "evidence": usage[45],
            },
        },
        "reserved_by_orchestrator": {
            "46": "air",
            "47": "characters",
            "48": "construction",
        },
        "free_in_current_shared_files": {
            "49": len(usage[49]) == 0,
            "50": len(usage[50]) == 0,
        },
        "current_shared_usage_49": usage[49],
        "current_shared_usage_50": usage[50],
        "proposed": {
            str(DOMAIN_NAVAL_COMBAT_LIFECYCLE): DOMAIN_NAVAL_COMBAT_LIFECYCLE_NAME,
            str(DOMAIN_NAVAL_COMBAT_MEMBERS): DOMAIN_NAVAL_COMBAT_MEMBERS_NAME,
        },
    }


def build_packet(
    *,
    cship_proof_path: Path,
    fixture_probe_path: Optional[Path],
    out_dir: Path,
) -> Dict[str, Any]:
    cship_report = load_json(cship_proof_path)
    cship = validate_cship_route(cship_report)
    fixture = fixture_status(fixture_probe_path)
    schema = schema_manifest()
    domain_audit = domain_allocation_audit()
    errors = list(cship["errors"])
    if not domain_audit["free_in_current_shared_files"]["49"]:
        errors.append("proposed domain 49 is not free in current shared files")
    if not domain_audit["free_in_current_shared_files"]["50"]:
        errors.append("proposed domain 50 is not free in current shared files")
    status = "slot_ready_fixture_blocked" if cship["ok"] and not fixture["ready"] else "slot_ready_fixture_ready"
    if not cship["ok"]:
        status = "blocked_cship_route_not_validated"
    if any(error.startswith("proposed domain") for error in errors):
        status = "blocked_domain_allocation_conflict"

    packet: Dict[str, Any] = {
        "generated_at_utc": dt.datetime.utcnow().replace(microsecond=0).isoformat() + "Z",
        "status": status,
        "ok_for_integration_slot": cship["ok"] and not errors,
        "blocking_errors": errors,
        "fixture": fixture,
        "domain_allocation": domain_audit,
        "cship_route": cship,
        "compact_schema": {
            "domains": schema["domains"],
            "schema_version": schema["schema_version"],
            "combat_row_len": schema["combat_row_len"],
            "member_row_len": schema["member_row_len"],
            "member_join_policy": (
                "naval_combat_members_live keeps ship_instance_id in the 72-byte member row; "
                "current hull telemetry stays in existing compact ships domain keyed by the same id"
            ),
        },
        "collector_patch_plan": collector_patch_plan(),
        "recorder_patch_plan": recorder_patch_plan(),
        "slot_proof_commands": slot_proof_commands(out_dir),
        "rejection_criteria": rejection_criteria(),
    }
    return packet


def collector_patch_plan() -> List[Dict[str, Any]]:
    return [
        {
            "step": "add_domains",
            "details": [
                "Add/schedule compact domain 49 naval_combat_lifecycle_live and domain 50 naval_combat_members_live.",
                "Keep shared entrypoint edits gated behind the orchestrator integration slot.",
            ],
        },
        {
            "step": "lifecycle_roots",
            "details": [
                "Use validated bridge body rows from naval_combat_live_fixture_probe.py only.",
                "Require the fixture probe's member_bridge.jsonl to have two-sided CFEXMember rows before shared DLL proof starts.",
                "Validate CNavalCombat body vtable module+0x29B6660, packed handle at +0x08, sides at +0x18/+0x20.",
                "Validate registry vtable module+0x29B6588, registry+0x18 handle, +0x28 attacker, +0x30 defender, +0x38 front.",
                "Fail closed with source-unavailable/partial status instead of guessed lifecycle rows.",
            ],
        },
        {
            "step": "member_roots",
            "details": [
                "Walk side+0xC8 group vector and CFEXGroup+0x08/+0x14 member vector.",
                "Validate each CFEXMember vtable module+0x29F9608 before emitting a member row.",
                "Emit unit_type_tag and ship_instance_id from CFEXMember+0x08.",
                "Emit owner tag from +0x20, designed max strength from +0x30, state from +0xE0, engagement weight from +0xF8, slot from +0x120.",
            ],
        },
        {
            "step": "cship_join",
            "details": [
                "For non-convoy member rows where ship_instance_id>0, require a current CShip row resolving through module+0x2941748 before status=validated_active_fixture.",
                "Validate CShip+0x08 type=51 and matching instance id, +0x0C alias, +0x710 ref100 sentinel.",
                "Use existing compact ships domain 29 for CShip hull telemetry: +0x6F8 strength, +0x700 organisation, +0x708 experience, +0x3C sunk convoys, +0xE0 category.",
                "Do not duplicate CShip telemetry into domain 50 in the first integration patch; join by ship_instance_id.",
                "If a non-convoy member id cannot resolve through current CShip, mark the naval combat member source partial and fail the proof gate.",
            ],
        },
    ]


def recorder_patch_plan() -> List[Dict[str, Any]]:
    return [
        {
            "step": "decode_domains",
            "details": [
                "Add decoder names for domain 49 naval_combat_lifecycle_live and domain 50 naval_combat_members_live.",
                "Use the lane-local schema: lifecycle row 72 bytes, member row 72 bytes, schema version 1.",
                "Require strict H4CR header/order/status checks equivalent to memory_reader/combat/naval_combat_compact_gate.rs.",
            ],
        },
        {
            "step": "gate_join",
            "details": [
                "When ships domain 29 is present in the same replay, allow lookup by member ship_instance_id for hull telemetry.",
                "Do not require a CShip join for convoy members with ship_instance_id=0.",
                "For the live proof, require at least one non-convoy member id to resolve against the current CShip route and no unresolved non-convoy member ids.",
            ],
        },
        {
            "step": "acceptance",
            "details": [
                "Compact stream must include domains 49 and 50 on scheduled frame hours.",
                ".battleplan must verify.",
                "Speed-five/current-equivalent hourly-or-lower capture must skip zero hours and report no partial/source/worker/bridge failures.",
            ],
        },
    ]


def slot_proof_commands(out_dir: Path) -> List[str]:
    live_dir = out_dir / "live_fixture_proof"
    return [
        "python -m py_compile memory_reader\\combat\\naval_combat_slot_ready_packet.py",
        "python memory_reader\\combat\\naval_combat_cship_route_probe.py --pid 45880 --workers 4 --out-dir extractor\native\\runs\\naval_combat_live_cship_route_870ebf_20260701_v1",
        "python memory_reader\\combat\\naval_combat_live_watch_gate.py --pid 45880 --timeout-s 900 --poll-s 10 --workers 4 --require-cship-join --collector-dll extractor\native\\target\\release\\collector.dll --run-recorder --out-dir extractor\native\\runs\\naval_combat_live_watch_gate_870ebf_<fixture>",
        "python memory_reader\\combat\\naval_combat_compact_proof.py --bridge-file extractor\native\\runs\\naval_combat_live_watch_gate_870ebf_<fixture>\\bridge.txt --out-dir %s" % live_dir,
        "rustc --edition=2021 --test memory_reader\\combat\\naval_combat_compact_gate.rs -o %s\\naval_combat_compact_gate_tests.exe" % live_dir,
        "%s\\naval_combat_compact_gate_tests.exe" % live_dir,
        "rustc --edition=2021 memory_reader\\combat\\naval_combat_compact_gate.rs -o %s\\naval_combat_compact_gate.exe" % live_dir,
        "%s\\naval_combat_compact_gate.exe %s\\naval_combat_lane_local_compact.bin --json-out %s\\rust_gate_live_fixture.json" % (live_dir, live_dir, live_dir),
        "After shared-slot patch: run DLL compact capture and live_capture.collector_proof_gate with domains naval_combat_lifecycle_live, naval_combat_members_live, ships.",
    ]


def rejection_criteria() -> List[str]:
    return [
        "Active fixture produces CNavalCombat body hits but no row passes body/side/registry validation.",
        "Body/side/registry validates but either side has zero CFEXMember rows in a fleet-vs-fleet fixture.",
        "Fixture emits member_bridge.jsonl but has no non-convoy ship_instance_id rows to exercise current CShip.",
        "Any non-convoy CFEXMember ship_instance_id fails to resolve to current CShip vtable module+0x2941748 with matching CShip+0x08/+0x0C id.",
        "Lane-local compact proof emits source-unavailable/partial flags or lacks two-sided member rows.",
        "Shared-slot DLL/Rust proof emits compact gaps, fails .battleplan verification, skips game hours, or reports partial/source/worker/bridge failures.",
    ]


def write_markdown(packet: Dict[str, Any], path: Path) -> None:
    lines = [
        "# naval_combat_live slot-ready packet",
        "",
        "## Status",
        "",
        "- status: `%s`" % packet["status"],
        "- ok_for_integration_slot: `%s`" % str(packet["ok_for_integration_slot"]).lower(),
        "- fixture: `%s`" % packet["fixture"]["status"],
        "",
        "## Domain Allocation",
        "",
        "- occupied `44`: supply_logistics_attrition_live",
        "- occupied `45`: focus_research_decisions_depth_live",
        "- orchestrator-reserved `46`: air",
        "- orchestrator-reserved `47`: characters",
        "- orchestrator-reserved `48`: construction",
        "- proposed `49`: naval_combat_lifecycle_live",
        "- proposed `50`: naval_combat_members_live",
        "- shared-file scan: 49 free `%s`, 50 free `%s`" % (
            str(packet["domain_allocation"]["free_in_current_shared_files"]["49"]).lower(),
            str(packet["domain_allocation"]["free_in_current_shared_files"]["50"]).lower(),
        ),
        "",
        "## Validated CShip Route",
        "",
        "- stale candidate `%s`: `%s`, hits `%s`" % (
            STALE_CSHIP_VTABLE_RVA,
            packet["cship_route"]["stale_cship_rva"]["verdict"],
            packet["cship_route"]["stale_cship_rva"]["hit_count"],
        ),
        "- current route `%s`: `%s`, hits `%s`, valid rows `%s`, unique ids `%s`" % (
            CSHIP_VTABLE_RVA,
            "validated" if packet["cship_route"]["ok"] else "not_validated",
            packet["cship_route"]["current_cship_route"]["hit_count"],
            packet["cship_route"]["current_cship_route"]["validated_count"],
            packet["cship_route"]["current_cship_route"]["validated_unique_instance_ids"],
        ),
        "",
        "## Compact Contract",
        "",
        "- domains: `%s`" % packet["compact_schema"]["domains"],
        "- lifecycle row length: `%s`" % packet["compact_schema"]["combat_row_len"],
        "- member row length: `%s`" % packet["compact_schema"]["member_row_len"],
        "- member join policy: %s" % packet["compact_schema"]["member_join_policy"],
        "",
        "## Fixture Blocker",
        "",
    ]
    lines.extend("- %s" % item for item in packet["fixture"]["requirements"])
    lines.extend([
        "",
        "## Collector Patch Plan",
        "",
    ])
    for step in packet["collector_patch_plan"]:
        lines.append("### %s" % step["step"])
        lines.extend("- %s" % item for item in step["details"])
        lines.append("")
    lines.extend(["## Recorder Patch Plan", ""])
    for step in packet["recorder_patch_plan"]:
        lines.append("### %s" % step["step"])
        lines.extend("- %s" % item for item in step["details"])
        lines.append("")
    lines.extend(["## Slot Proof Commands", ""])
    lines.append("```powershell")
    lines.extend(packet["slot_proof_commands"])
    lines.append("```")
    lines.extend(["", "## Rejection Criteria", ""])
    lines.extend("- %s" % item for item in packet["rejection_criteria"])
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cship-proof", type=Path, required=True)
    parser.add_argument("--fixture-probe", type=Path)
    parser.add_argument("--out-dir", type=Path, required=True)
    args = parser.parse_args()

    args.out_dir.mkdir(parents=True, exist_ok=True)
    packet = build_packet(
        cship_proof_path=args.cship_proof,
        fixture_probe_path=args.fixture_probe,
        out_dir=args.out_dir,
    )
    json_path = args.out_dir / "naval_combat_slot_ready_packet.json"
    md_path = args.out_dir / "SLOT_READY_PACKET.md"
    json_path.write_text(json.dumps(packet, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    write_markdown(packet, md_path)
    print(json.dumps({
        "status": packet["status"],
        "ok_for_integration_slot": packet["ok_for_integration_slot"],
        "fixture": packet["fixture"]["status"],
        "out_json": str(json_path),
        "out_md": str(md_path),
    }, indent=2, sort_keys=True))
    return 0 if packet["ok_for_integration_slot"] else 2


if __name__ == "__main__":
    raise SystemExit(main())
