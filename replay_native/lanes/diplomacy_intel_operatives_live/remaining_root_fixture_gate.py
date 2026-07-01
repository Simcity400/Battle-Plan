#!/usr/bin/env python3
"""Fail-closed remaining-root gate for diplomacy/intel/operatives domain 53.

This lane-local gate does not discover new roots. It turns current proof
artifacts plus optional fixture evidence into an executable go/no-go contract
for the unresolved production surfaces: war rows, running operation detail,
intel-map percentages, and incoming diplomatic actions.

Exit codes:
  0: all remaining roots have accepted current-build evidence.
  1: invalid input or unreadable proof.
  2: blocker evidence was written; the lane must stay fail-closed.
"""

from __future__ import annotations

import argparse
import json
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


LANE = "diplomacy_intel_operatives_live"
DOMAIN_ID = 53
EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"

REQUIRED_WAR_FIXTURE_FIELDS = (
    "build_sha256",
    "same_tick_utc",
    "war_name_or_id",
    "attacker_tags",
    "defender_tags",
    "ui_or_save_at_war_evidence",
)

REQUIRED_WAR_LIFECYCLE_FIXTURE_FIELDS = (
    "build_sha256",
    "same_tick_utc",
    "war_name_or_id",
    "attacker_tags",
    "defender_tags",
    "start_date_scalar",
    "war_grouping_or_handle",
    "ui_or_save_war_lifecycle_evidence",
)

REQUIRED_OPERATION_FIXTURE_FIELDS = (
    "build_sha256",
    "same_tick_utc",
    "owner_tag",
    "target_tag_or_state",
    "operation_token",
    "operation_name",
    "phase_or_progress",
    "assigned_operative_ids",
)

REQUIRED_INTEL_MAP_FIXTURE_FIELDS = (
    "build_sha256",
    "same_tick_utc",
    "viewer_tag",
    "target_tag",
    "civilian_percent",
    "army_percent",
    "navy_percent",
    "air_percent",
    "ui_or_save_intel_evidence",
)

REQUIRED_INCOMING_ACTION_FIXTURE_FIELDS = (
    "build_sha256",
    "same_tick_utc",
    "actor_tag",
    "target_tag",
    "action_kind",
    "visible_queue_evidence",
)

FIXTURE_CONTRACTS = {
    "war_lifecycle_detail": {
        "positive_fixture": {
            "setup": "current 870ebf live tick with at least one active war whose UI/save identity and start date are visible or save-corroborated same-tick",
            "must_identify": [
                "war_name_or_id",
                "attacker_tags",
                "defender_tags",
                "start_date_scalar",
                "war_grouping_or_handle",
            ],
            "acceptance_rule": (
                "a candidate must join the accepted tag-pair/casualty row pool to "
                "a stable war grouping/name and decode the embedded date scalar "
                "from live memory, then warm-revalidate the same owner sets and "
                "start date against the fixture"
            ),
        },
        "rejection_rule": (
            "tag-pair casualty rows alone are not war lifecycle detail; embedded "
            "date RTTI without scalar date fields and relation-entry scans without "
            "a stable war grouping/name remain a validated slice, not complete war replay"
        ),
    },
    "operation_detail": {
        "positive_fixture": {
            "setup": "current 870ebf live tick with at least one running intelligence operation visible for an owner country",
            "must_identify": [
                "owner_tag",
                "target_tag_or_state",
                "operation_token",
                "operation_name",
                "phase_or_progress",
                "assigned_operative_ids",
            ],
            "acceptance_rule": (
                "a current-build probe must map a running COperation/rooted structure "
                "to the fixture fields and warm-revalidate owner, target, progress/phase, "
                "and operative slots from live memory"
            ),
        },
        "rejection_rule": (
            "definition-only RTTI/databases are not operation detail; stale 16022851 "
            "operation RVAs remain rejected unless the probe maps live running-operation "
            "fields and passes warm validation"
        ),
    },
    "intel_map_percentages": {
        "positive_fixture": {
            "setup": "current 870ebf live tick with a viewer/target pair whose civilian, army, navy, and air intel percentages are visible or save-corroborated same-tick",
            "must_identify": [
                "viewer_tag",
                "target_tag",
                "civilian_percent",
                "army_percent",
                "navy_percent",
                "air_percent",
            ],
            "acceptance_rule": (
                "a candidate must be keyed by viewer and target country, decode all four "
                "percentage lanes, match the fixture values at the expected scale, and "
                "warm-revalidate without relying on positional save rows alone"
            ),
        },
        "rejection_rule": (
            "reject unkeyed multisets, positional-only save rows, or values that cannot "
            "bind both viewer and target in live memory"
        ),
    },
    "incoming_diplomatic_action_queue": {
        "positive_fixture": {
            "setup": "current 870ebf live tick with at least one visible pending incoming diplomatic action",
            "must_identify": [
                "actor_tag",
                "target_tag",
                "action_kind",
                "visible_queue_evidence",
            ],
            "acceptance_rule": (
                "a candidate must locate a pending-action queue/root, not just settled "
                "relation flags, and warm-revalidate actor, target, kind, and queue "
                "presence against the visible same-tick fixture"
            ),
        },
        "rejection_rule": (
            "relation-state flags, accepted/settled diplomacy, or UI modal definitions "
            "without a pending queue root are not incoming-action queue extraction"
        ),
    },
}


def load_json(path: Path) -> dict[str, Any]:
    try:
        with path.open("r", encoding="utf-8") as f:
            value = json.load(f)
    except OSError as exc:
        raise ValueError(f"{path}: {exc}") from exc
    except json.JSONDecodeError as exc:
        raise ValueError(f"{path}: invalid JSON: {exc}") from exc
    if not isinstance(value, dict):
        raise ValueError(f"{path}: top-level JSON value must be an object")
    return value


def process_sha(report: dict[str, Any]) -> str | None:
    process = report.get("process")
    if isinstance(process, dict):
        value = process.get("sha256")
        return value if isinstance(value, str) else None
    return None


def proof_process(report: dict[str, Any]) -> dict[str, Any]:
    process = report.get("process")
    return process if isinstance(process, dict) else {}


def missing_fields(row: dict[str, Any] | None, required: tuple[str, ...]) -> list[str]:
    if not isinstance(row, dict):
        return list(required)
    return [field for field in required if row.get(field) in (None, "", [])]


def fixture_section(fixture: dict[str, Any] | None, name: str) -> dict[str, Any] | None:
    if not fixture:
        return None
    value = fixture.get(name)
    return value if isinstance(value, dict) else None


def summarize_war_blocker(path: Path, report: dict[str, Any], fixture: dict[str, Any] | None) -> dict[str, Any] | None:
    if report.get("probe") == "probe_870ebf_war_shape":
        accepted = bool(report.get("accepted_for_production_war_root"))
        summary = report.get("summary") if isinstance(report.get("summary"), dict) else {}
        scan = report.get("scan") if isinstance(report.get("scan"), dict) else {}
        warm = report.get("warm_validation") if isinstance(report.get("warm_validation"), dict) else {}
        if accepted and process_sha(report) == EXPECTED_SHA256:
            return None
        return {
            "id": "war_root",
            "status": "war_shape_unaccepted",
            "proof_path": str(path),
            "reason": "current-build war-shape probe did not accept a production war root",
            "candidate": {
                "status": report.get("status"),
                "accepted_for_production_war_root": accepted,
                "current_date_typeptr_rows": summary.get("current_date_typeptr_rows"),
                "current_date_unique_pairs": summary.get("current_date_unique_pairs"),
                "old_16022851_date_typeptr_rows": summary.get("old_16022851_date_typeptr_rows"),
                "warm_checked_rows": summary.get("warm_checked_rows"),
                "warm_valid_rows": summary.get("warm_valid_rows"),
                "warm_invalid_rows": summary.get("warm_invalid_rows"),
                "scan_timed_out": scan.get("timed_out"),
                "read_failures": scan.get("read_failures"),
                "warm_status": warm.get("status"),
            },
        }

    wars = report.get("wars")
    if not isinstance(wars, dict):
        return {
            "id": "war_root",
            "status": "blocked_missing_war_proof",
            "proof_path": str(path),
            "reason": "diplomacy proof has no wars object",
        }

    fixture_row = fixture_section(fixture, "war_root")
    fixture_missing = missing_fields(fixture_row, REQUIRED_WAR_FIXTURE_FIELDS)
    valid_rows = int(wars.get("valid_rows") or 0)
    typeptr_hits = int(wars.get("typeptr_hits") or 0)
    status = wars.get("status")

    if status == "validated" and valid_rows > 0 and not fixture_missing:
        return None

    return {
        "id": "war_root",
        "status": "candidate_rejected_fixture_missing",
        "proof_path": str(path),
        "reason": (
            "old 16022851 date-typeptr war route was not observed in the current "
            "870ebf bounded heap scan, and no active-war fixture was supplied"
        ),
        "candidate_rejected": {
            "route": "record+0x08 in {module+0x26F2398,module+0x26F21A0}",
            "status": status,
            "regions_scanned": wars.get("regions_scanned"),
            "bytes_scanned": wars.get("bytes_scanned"),
            "typeptr_hits": typeptr_hits,
            "valid_rows": valid_rows,
            "read_failures": wars.get("read_failures"),
            "war_relation_offsets": wars.get("war_relation_offsets"),
            "typeptr_vtable_rvas": wars.get("typeptr_vtable_rvas"),
        },
        "required_fixture": {
            "section": "war_root",
            "missing_fields": fixture_missing,
            "fields": list(REQUIRED_WAR_FIXTURE_FIELDS),
        },
    }


def summarize_operation_blocker(path: Path, report: dict[str, Any], fixture: dict[str, Any] | None) -> dict[str, Any] | None:
    summary = report.get("summary")
    if not isinstance(summary, dict):
        return {
            "id": "operation_detail",
            "status": "blocked_missing_operation_summary",
            "proof_path": str(path),
            "reason": "operation proof has no summary object",
        }

    fixture_row = fixture_section(fixture, "operation_detail")
    fixture_missing = missing_fields(fixture_row, REQUIRED_OPERATION_FIXTURE_FIELDS)
    accepted = bool(summary.get("accepted_for_operation_detail"))
    if accepted and not fixture_missing:
        return None

    targets = report.get("rtti_targets") if isinstance(report.get("rtti_targets"), dict) else {}
    current_rvas = {
        name: value.get("derived_vtable_rvas")
        for name, value in targets.items()
        if isinstance(value, dict)
    }

    return {
        "id": "operation_detail",
        "status": "definition_rtti_only_fixture_missing",
        "proof_path": str(path),
        "reason": summary.get(
            "reason",
            "no current running COperation detail root accepted",
        ),
        "accepted_for_operation_detail": accepted,
        "current_definition_rtti_vtable_rvas": current_rvas,
        "old_16022851_operation_rvas": report.get("old_16022851_operation_rvas"),
        "old_16022851_rva_classification": report.get("old_16022851_rva_classification"),
        "summary": summary,
        "required_fixture": {
            "section": "operation_detail",
            "missing_fields": fixture_missing,
            "fields": list(REQUIRED_OPERATION_FIXTURE_FIELDS),
        },
        "positive_fixture_contract": FIXTURE_CONTRACTS["operation_detail"]["positive_fixture"],
        "rejection_rule": FIXTURE_CONTRACTS["operation_detail"]["rejection_rule"],
    }


def summarize_war_lifecycle_blocker(
    fixture: dict[str, Any] | None,
) -> dict[str, Any] | None:
    row = fixture_section(fixture, "war_lifecycle_detail")
    missing = missing_fields(row, REQUIRED_WAR_LIFECYCLE_FIXTURE_FIELDS)
    if not missing:
        return None
    return {
        "id": "war_lifecycle_detail",
        "status": "fixture_missing",
        "reason": (
            "current war tag-pair/casualty rows are accepted, but embedded date "
            "scalar decode and higher-level war grouping/name are not validated"
        ),
        "accepted_slice": {
            "war_root": "current 870ebf global row pool keyed by tag pair",
            "fields": [
                "first_tag_id",
                "second_tag_id",
                "first_casualties",
                "second_casualties",
                "echo_tags",
                "current date RTTI pointer presence",
            ],
        },
        "required_fixture": {
            "section": "war_lifecycle_detail",
            "missing_fields": missing,
            "fields": list(REQUIRED_WAR_LIFECYCLE_FIXTURE_FIELDS),
        },
        "positive_fixture_contract": FIXTURE_CONTRACTS["war_lifecycle_detail"][
            "positive_fixture"
        ],
        "rejection_rule": FIXTURE_CONTRACTS["war_lifecycle_detail"]["rejection_rule"],
    }


def summarize_fixture_only_blocker(
    fixture: dict[str, Any] | None,
    section: str,
    blocker_id: str,
    required: tuple[str, ...],
    reason: str,
) -> dict[str, Any] | None:
    row = fixture_section(fixture, section)
    missing = missing_fields(row, required)
    if not missing:
        return None
    return {
        "id": blocker_id,
        "status": "fixture_missing",
        "reason": reason,
        "required_fixture": {
            "section": section,
            "missing_fields": missing,
            "fields": list(required),
        },
        "positive_fixture_contract": FIXTURE_CONTRACTS.get(blocker_id, {}).get(
            "positive_fixture"
        ),
        "rejection_rule": FIXTURE_CONTRACTS.get(blocker_id, {}).get("rejection_rule"),
    }


def build_report(args: argparse.Namespace) -> tuple[dict[str, Any], int]:
    war_path = Path(args.war_proof)
    operation_path = Path(args.operation_proof)
    fixture_path = Path(args.fixture) if args.fixture else None

    war_report = load_json(war_path)
    operation_report = load_json(operation_path)
    fixture = load_json(fixture_path) if fixture_path else None

    proof_builds = {
        "war_proof_sha256": process_sha(war_report),
        "operation_proof_sha256": process_sha(operation_report),
        "expected_sha256": EXPECTED_SHA256,
    }
    build_gate_ok = all(
        value == EXPECTED_SHA256
        for key, value in proof_builds.items()
        if key.endswith("_sha256")
    )

    blockers: list[dict[str, Any]] = []
    war_blocker = summarize_war_blocker(war_path, war_report, fixture)
    if war_blocker:
        blockers.append(war_blocker)
    war_lifecycle_blocker = summarize_war_lifecycle_blocker(fixture)
    if war_lifecycle_blocker:
        blockers.append(war_lifecycle_blocker)
    operation_blocker = summarize_operation_blocker(operation_path, operation_report, fixture)
    if operation_blocker:
        blockers.append(operation_blocker)

    intel_map_blocker = summarize_fixture_only_blocker(
        fixture,
        "intel_map_percentages",
        "intel_map_percentages",
        REQUIRED_INTEL_MAP_FIXTURE_FIELDS,
        (
            "save oracle rows are positional development input only; no current "
            "870ebf live owner/target percentage root or same-tick UI fixture is accepted"
        ),
    )
    if intel_map_blocker:
        blockers.append(intel_map_blocker)

    incoming_action_blocker = summarize_fixture_only_blocker(
        fixture,
        "incoming_action_queue",
        "incoming_diplomatic_action_queue",
        REQUIRED_INCOMING_ACTION_FIXTURE_FIELDS,
        (
            "current validated live surface is relation state/flags only; no "
            "current 870ebf pending-action queue root or visible queue fixture is accepted"
        ),
    )
    if incoming_action_blocker:
        blockers.append(incoming_action_blocker)

    if not build_gate_ok:
        blockers.insert(
            0,
            {
                "id": "build_gate",
                "status": "failed",
                "reason": "proof artifacts are not all from expected current 870ebf SHA",
                "proof_builds": proof_builds,
            },
        )

    report = {
        "schema": "battle-plan.diplomacy_intel_operatives.remaining_root_fixture_gate.v1",
        "generated_utc": datetime.now(timezone.utc).isoformat(timespec="seconds"),
        "lane": LANE,
        "domain_id": DOMAIN_ID,
        "expected_sha256": EXPECTED_SHA256,
        "status": "blocked" if blockers else "accepted",
        "ok": not blockers,
        "proof_inputs": {
            "war_proof": str(war_path),
            "operation_proof": str(operation_path),
            "fixture": str(fixture_path) if fixture_path else None,
            "war_process": proof_process(war_report),
            "operation_process": proof_process(operation_report),
            "proof_builds": proof_builds,
        },
        "blockers": blockers,
        "registration_only_patch_plan": [
            "Do not edit shared collector/recorder entrypoints while another lane owns the slot.",
            "When a slot opens, register domain 53 only after this gate exits 0 against current 870ebf proof.",
            "War root is accepted only when the current war-shape proof remains accepted.",
            "Keep war lifecycle detail badged unavailable until start-date scalar and war grouping/name are validated.",
            "Keep operation rows resisted while operation_detail remains blocked.",
            "Keep intel-map percentages resisted while intel_map_percentages remains blocked.",
            "Keep incoming-action queue badged unavailable while incoming_diplomatic_action_queue remains blocked.",
        ],
    }
    return report, 0 if not blockers else 2


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--war-proof", required=True, help="current-build diplomacy proof JSON")
    parser.add_argument("--operation-proof", required=True, help="current-build operation proof JSON")
    parser.add_argument("--fixture", help="optional same-tick fixture JSON")
    parser.add_argument("--out", required=True, help="output blocker/acceptance JSON")
    args = parser.parse_args(argv)

    try:
        report, code = build_report(args)
    except ValueError as exc:
        print(f"remaining_root_fixture_gate: {exc}", file=sys.stderr)
        return 1

    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    with out_path.open("w", encoding="utf-8") as f:
        json.dump(report, f, indent=2, sort_keys=True)
        f.write("\n")

    print(
        f"remaining_root_fixture_gate: status={report['status']} "
        f"blockers={len(report['blockers'])} out={out_path}"
    )
    return code


if __name__ == "__main__":
    raise SystemExit(main())
