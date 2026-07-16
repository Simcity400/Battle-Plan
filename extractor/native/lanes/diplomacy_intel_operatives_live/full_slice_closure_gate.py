#!/usr/bin/env python3
"""Full-slice closure gate for diplomacy/intel/operatives domain 53.

This is intentionally stricter than the remaining-root fixture gate. The
remaining-root gate only covers four known root blockers. This gate answers the
lane-owner question for the whole assigned slice:

* war lifecycle detail;
* diplomacy actions/queues;
* intel map percentages;
* operations;
* operatives;
* spy networks;
* agency state;
* compact event/state records needed for replay.

It never accepts a surface from a broad summary. A surface is accepted only when
the supplied proof artifact exposes the current-build production root and the
lane-local compact/replay proof can carry it. Everything else is reported as a
blocker or validated slice.
"""

from __future__ import annotations

import argparse
import json
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


LANE = "diplomacy_intel_operatives_live"
DOMAIN_ID = 53
EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"


def load_json(path: Path | None) -> dict[str, Any]:
    if path is None:
        return {}
    with path.open("r", encoding="utf-8") as f:
        value = json.load(f)
    if not isinstance(value, dict):
        raise ValueError(f"{path}: expected JSON object")
    return value


def process_sha(report: dict[str, Any]) -> str | None:
    process = report.get("process")
    if isinstance(process, dict):
        value = process.get("sha256")
        return value if isinstance(value, str) else None
    value = report.get("build_sha256")
    return value if isinstance(value, str) else None


def status(report: dict[str, Any], *path: str) -> Any:
    value: Any = report
    for key in path:
        if not isinstance(value, dict):
            return None
        value = value.get(key)
    return value


def container_integrity_ok(replay_proof: dict[str, Any]) -> bool:
    return (
        replay_proof.get("container_integrity_verified") is True
        and replay_proof.get("stream_entry_byte_identity_verified") is True
        and replay_proof.get("production_certifying") is False
        and replay_proof.get("source_capture_verified") is False
        and not replay_proof.get("parse_errors")
    )


def production_stream_certified(replay_proof: dict[str, Any]) -> bool:
    hourly = replay_proof.get("hourly_requirement")
    return (
        replay_proof.get("production_certifying") is True
        and replay_proof.get("source_capture_verified") is True
        and replay_proof.get("extraction_perf_gate") is True
        and isinstance(hourly, dict)
        and hourly.get("required") is True
        and hourly.get("met") is True
        and not replay_proof.get("parse_errors")
        and not replay_proof.get("ordering_errors")
    )


def compact_state_ok(replay_proof: dict[str, Any]) -> bool:
    if not container_integrity_ok(replay_proof):
        return False
    return int(replay_proof.get("domain_id") or 0) == DOMAIN_ID and int(
        replay_proof.get("data_rows_total") or 0
    ) > 0


def replay_data_unit_count(replay_proof: dict[str, Any], unit_name: str) -> int:
    data_units = replay_proof.get("data_units_total")
    if not isinstance(data_units, dict):
        return 0
    return int(data_units.get(unit_name) or 0)


def compact_operative_lifecycle_ok(replay_proof: dict[str, Any]) -> bool:
    return compact_state_ok(replay_proof) and replay_data_unit_count(
        replay_proof, "operative_lifecycle_rows"
    ) > 0


def compact_agency_capacity_cluster_ok(replay_proof: dict[str, Any]) -> bool:
    return compact_state_ok(replay_proof) and replay_data_unit_count(
        replay_proof, "agency_capacity_cluster_rows"
    ) > 0


def compact_state_event_rows_ok(replay_proof: dict[str, Any]) -> bool:
    return compact_state_ok(replay_proof) and replay_data_unit_count(
        replay_proof, "compact_state_event_rows"
    ) > 0


def compact_relation_faction_events_ok(replay_proof: dict[str, Any]) -> bool:
    return (
        compact_state_event_rows_ok(replay_proof)
        and replay_data_unit_count(
            replay_proof, "compact_state_event_relation_opinion_rows"
        )
        > 0
        and replay_data_unit_count(
            replay_proof, "compact_state_event_country_faction_group_rows"
        )
        > 0
    )


def build_surface_report(
    diplomacy: dict[str, Any],
    intel: dict[str, Any],
    war: dict[str, Any],
    operation: dict[str, Any],
    remaining: dict[str, Any],
    replay_proof: dict[str, Any],
    agency_capacity: dict[str, Any] | None = None,
    war_date_scalar: dict[str, Any] | None = None,
    operation_definition: dict[str, Any] | None = None,
    operative_lifecycle: dict[str, Any] | None = None,
    operative_lifecycle_registration: dict[str, Any] | None = None,
    agency_capacity_registration: dict[str, Any] | None = None,
    state_events_registration: dict[str, Any] | None = None,
) -> list[dict[str, Any]]:
    blockers_by_id = {
        blocker.get("id"): blocker
        for blocker in remaining.get("blockers", [])
        if isinstance(blocker, dict)
    }

    surfaces: list[dict[str, Any]] = []

    war_root_ok = bool(war.get("accepted_for_production_war_root")) or (
        status(diplomacy, "wars", "status") == "validated"
    )
    war_date_status = status(war_date_scalar or {}, "status")
    war_date_rejected = war_date_status == "inline_date_scalar_offsets_rejected"
    war_lifecycle_blocker = blockers_by_id.get("war_lifecycle_detail")
    if isinstance(war_lifecycle_blocker, dict) and war_date_rejected:
        war_lifecycle_blocker = dict(war_lifecycle_blocker)
        war_lifecycle_blocker["narrowed_by"] = {
            "probe": status(war_date_scalar or {}, "probe"),
            "status": war_date_status,
            "candidate_rejected": status(war_date_scalar or {}, "rejection", "candidate"),
            "row_offsets_rejected": status(
                war_date_scalar or {}, "rejection", "row_offsets_rejected"
            ),
            "live_valid_rows": status(
                war_date_scalar or {}, "summary", "live_valid_rows"
            ),
        }
    surfaces.append(
        {
            "surface": "war_lifecycle_detail",
            "status": "blocked" if "war_lifecycle_detail" in blockers_by_id else "accepted",
            "validated_slice": (
                "current 870ebf tag-pair/casualty war row pool"
                + (
                    "; inline DateHour scalar row+0x10..row+0x3C rejected"
                    if war_date_rejected
                    else ""
                )
                if war_root_ok
                else None
            ),
            "missing_extractable_fields": [
                "war_name_or_id",
                "attacker/defender side grouping",
                (
                    "start_date_scalar outside rejected inline row+0x10..row+0x3C "
                    "DateHour range"
                    if war_date_rejected
                    else "start_date_scalar"
                ),
                "war_grouping_or_handle",
                "war start/update/end event records",
            ],
            "blocker": war_lifecycle_blocker,
        }
    )

    relation_ok = status(diplomacy, "relations", "status") == "validated_structural"
    flags_ok = status(diplomacy, "relation_flags", "status") in (
        "validated_current_rtti",
        "validated_shifted_candidates",
        "validated_old_candidates",
    )
    surfaces.append(
        {
            "surface": "diplomacy_actions_queues",
            "status": "blocked",
            "validated_slice": (
                "settled bilateral relation table, opinions, flags, subjects/autonomy"
                if relation_ok or flags_ok
                else None
            ),
            "missing_extractable_fields": [
                "incoming diplomatic action queue root",
                "pending actor/target/action_kind rows",
                "action accepted/rejected/expired event records",
            ],
            "blocker": blockers_by_id.get("incoming_diplomatic_action_queue"),
        }
    )

    surfaces.append(
        {
            "surface": "intel_map_percentages",
            "status": "blocked",
            "validated_slice": None,
            "missing_extractable_fields": [
                "viewer_tag",
                "target_tag",
                "civilian_percent",
                "army_percent",
                "navy_percent",
                "air_percent",
            ],
            "blocker": blockers_by_id.get("intel_map_percentages"),
        }
    )

    operation_summary = operation.get("summary") if isinstance(operation, dict) else {}
    operation_accepted = bool(
        isinstance(operation_summary, dict)
        and operation_summary.get("accepted_for_operation_detail")
    )
    operation_definition_status = status(operation_definition or {}, "status")
    operation_heap_rejected = (
        operation_definition_status
        == "coperation_heap_hits_rejected_as_running_operation_state"
    )
    operation_blocker = blockers_by_id.get("operation_detail")
    if isinstance(operation_blocker, dict) and operation_heap_rejected:
        operation_blocker = dict(operation_blocker)
        operation_blocker["narrowed_by"] = {
            "probe": status(operation_definition or {}, "probe"),
            "status": operation_definition_status,
            "candidate_rejected": status(
                operation_definition or {}, "rejection", "candidate"
            ),
            "database_object": status(
                operation_definition or {}, "summary", "database_object"
            ),
            "definition_table_offset": status(
                operation_definition or {}, "summary", "definition_table_offset"
            ),
            "definition_count": status(
                operation_definition or {}, "summary", "definition_count"
            ),
            "prior_full_heap_COperation_hits": status(
                operation_definition or {}, "summary", "prior_full_heap_COperation_hits"
            ),
        }
    surfaces.append(
        {
            "surface": "operations",
            "status": "accepted" if operation_accepted else "blocked",
            "validated_slice": (
                "current operation definition RTTI"
                + (
                    "; COperation heap-hit set rejected as COperationsDatabase definition table"
                    if operation_heap_rejected
                    else ""
                )
                if operation
                else None
            ),
            "missing_extractable_fields": [
                (
                    "running operation owner outside rejected COperation definition table"
                    if operation_heap_rejected
                    else "running operation owner"
                ),
                "target tag/state",
                "operation token/name",
                "phase/progress/date/duration",
                "assigned operative slots",
                "running/finished operation event records",
            ],
            "blocker": operation_blocker,
        }
    )

    operative_status = status(intel, "operatives", "status")
    operative_ok = operative_status == "validated"
    operative_lifecycle_status = status(operative_lifecycle or {}, "status")
    operative_lifecycle_groups_ok = (
        operative_lifecycle_status == "operative_lifecycle_groups_validated"
    )
    operative_lifecycle_compact_ok = compact_operative_lifecycle_ok(replay_proof)
    operative_lifecycle_registration_ok = (
        status(operative_lifecycle_registration or {}, "status")
        == "operative_lifecycle_registration_patch_validated"
    )
    agency_capacity_registration_ok = (
        status(agency_capacity_registration or {}, "status")
        == "agency_capacity_registration_patch_validated"
    )
    state_events_registration_ok = (
        status(state_events_registration or {}, "status")
        == "state_events_registration_patch_validated"
    )
    compact_state_events_ok = compact_state_event_rows_ok(replay_proof)
    compact_relation_faction_events = compact_relation_faction_events_ok(replay_proof)
    surfaces.append(
        {
            "surface": "operatives",
            "status": "validated_slice" if operative_ok else "blocked",
            "validated_slice": (
                "active operative identity, owner, detected state, skill, and XP"
                + (
                    "; COperativeLeader lifecycle flag cohorts validated at identity+0x110"
                    if operative_lifecycle_groups_ok
                    else ""
                )
                + (
                    "; compact operative lifecycle rows verified"
                    if operative_lifecycle_compact_ok
                    else ""
                )
                if operative_ok
                else None
            ),
            "missing_extractable_fields": [
                "operative mission assignment",
                "operation assignment link",
                "traits",
                "nationalities",
                "portrait/gfx token",
                (
                    "semantic names for lifeflag 31/63 cohorts and recruitable operative pool"
                    if operative_lifecycle_groups_ok
                    else "recruitable operative pool"
                ),
                (
                    "captured_by and positive captured fixture; lifeflag 63 is only a validated cohort"
                    if operative_lifecycle_groups_ok
                    else "captured_by and positive captured fixture"
                ),
                "operative lifecycle event records",
            ],
            "blocker": {
                "id": "operative_full_lifecycle",
                "status": "schema_and_fixture_gap",
                "reason": (
                    (
                        "active roster fields and compact lifecycle flag rows are "
                        "validated, but current live proof does not carry full "
                        "save-oracle operative lifecycle"
                    )
                    if operative_lifecycle_compact_ok
                    else (
                        "active roster fields are validated, but the compact schema "
                        "and current live proof do not carry full save-oracle "
                        "operative lifecycle"
                    )
                    + (
                        "; lifecycle flag cohorts are rooted but not semantically named"
                        if operative_lifecycle_groups_ok
                        else ""
                    )
                ),
                "narrowed_by": (
                    {
                        "probe": status(operative_lifecycle or {}, "probe"),
                        "status": operative_lifecycle_status,
                        "identity_classdesc_rva": "0x293F790",
                        "lifeflag_offset": "0x110",
                        "owner_valid_COperativeLeader_lifeflag_counts": status(
                            operative_lifecycle or {},
                            "summary",
                            "owner_valid_COperativeLeader_lifeflag_counts",
                        ),
                        "warm_validated": status(
                            operative_lifecycle or {}, "summary", "warm_validated"
                        ),
                        "compact_operative_lifecycle_rows": replay_data_unit_count(
                            replay_proof, "operative_lifecycle_rows"
                        ),
                        "compact_container_integrity_verified": container_integrity_ok(
                            replay_proof
                        ),
                    }
                    if operative_lifecycle_groups_ok or operative_lifecycle_compact_ok
                    else None
                ),
            },
        }
    )

    surfaces.append(
        {
            "surface": "spy_networks",
            "status": "blocked",
            "validated_slice": None,
            "missing_extractable_fields": [
                "strategic operative network graph root",
                "per-owner/per-target network strength",
                "mission/network state by operative and target",
                "network grow/decay event records",
            ],
            "blocker": {
                "id": "spy_network_root",
                "status": "no_current_root",
                "reason": (
                    "save oracle identifies strategic_operatives as a separate graph; "
                    "no current 870ebf live root or compact row schema is accepted"
                ),
            },
        }
    )

    agency_ok = status(intel, "agencies", "status") == "validated"
    cryptology_status = status(intel, "cryptology", "status")
    cryptology_ok = cryptology_status in ("validated", "manager_validated_no_targets")
    capacity_status = status(agency_capacity or {}, "status")
    capacity_validated_slice = (
        "; capacity cluster narrowed: legacy +0xE4=max/+0xF0=usable assignment rejected"
        if capacity_status == "legacy_assignment_rejected"
        else ""
    )
    capacity_compact_slice = (
        "; raw agency capacity cluster rows verified"
        if compact_agency_capacity_cluster_ok(replay_proof)
        else ""
    )
    capacity_missing = (
        "same-tick UI/save semantic confirmation for +0xF0 as max and +0xE4/+0xF4 as usable/current slots"
        if capacity_status == "legacy_assignment_rejected"
        else "max-vs-usable slot disambiguation fixture"
    )
    surfaces.append(
        {
            "surface": "agency_state",
            "status": "validated_slice" if agency_ok else "blocked",
            "validated_slice": (
                "agency name/icon/upgrade progress/max operatives/usable slots/defense"
                + (" plus cryptology targets" if cryptology_ok else "")
                + capacity_validated_slice
                + capacity_compact_slice
                if agency_ok
                else None
            ),
            "missing_extractable_fields": [
                "upgrade branch levels",
                "current upgrade token",
                "is_created/building state",
                "elapsed days for next slot",
                "recruitment pool",
                capacity_missing,
            ],
            "blocker": {
                "id": "agency_full_state",
                "status": "validated_slice_only",
                "reason": (
                    "agency scalar root is accepted, but full save-oracle agency "
                    "state is not represented in domain 53 compact rows"
                    + (
                        "; capacity semantic labels are narrowed but not fully accepted"
                        if capacity_status == "legacy_assignment_rejected"
                        else ""
                    )
                    + (
                        "; raw capacity cluster rows are compact-verified without semantic labels"
                        if compact_agency_capacity_cluster_ok(replay_proof)
                        else ""
                    )
                ),
                "narrowed_by": (
                    {
                        "probe": status(agency_capacity or {}, "probe"),
                        "status": capacity_status,
                        "raw_offsets": ["agency+0xE4", "agency+0xF0", "agency+0xF4"],
                        "e4_f4_equal_rows": status(
                            agency_capacity or {},
                            "capacity_analysis",
                            "e4_f4_equal_rows",
                        ),
                        "legacy_assignment": status(
                            agency_capacity or {},
                            "capacity_analysis",
                            "legacy_assignment",
                            "status",
                        ),
                        "compact_agency_capacity_cluster_rows": replay_data_unit_count(
                            replay_proof, "agency_capacity_cluster_rows"
                        ),
                    }
                    if compact_agency_capacity_cluster_ok(replay_proof)
                    else None
                ),
            },
        }
    )

    surfaces.append(
        {
            "surface": "compact_event_state_records",
            "status": "blocked",
            "validated_slice": (
                "lane-local H4CR domain-data state record and .battleplan verification"
                + (
                    " with compact operative lifecycle rows"
                    if compact_operative_lifecycle_ok(replay_proof)
                    else ""
                )
                + (
                    "; exact shared registration details validated for operative lifecycle rows"
                    if operative_lifecycle_registration_ok
                    else ""
                )
                + (
                    "; exact shared registration details validated for raw agency capacity rows"
                    if agency_capacity_registration_ok
                    else ""
                )
                + (
                    "; compact validated-state event rows verified"
                    if compact_state_events_ok
                    else ""
                )
                + (
                    " including relation opinion and country faction-group observations"
                    if compact_relation_faction_events
                    else ""
                )
                + (
                    "; exact shared registration details validated for compact state-event rows"
                    if state_events_registration_ok
                    else ""
                )
                if compact_state_ok(replay_proof)
                else None
            ),
            "missing_extractable_fields": [
                "shared DLL collector registration",
                "shared Rust recorder decode registration",
                "speed-five/current-equivalent hourly proof",
                "event records for wars, diplomacy queue actions, operations, "
                "operative lifecycle, spy networks, and agency changes",
            ],
            "blocker": {
                "id": "production_compact_event_stream",
                "status": "shared_slot_and_event_schema_missing",
                "reason": (
                    "lane-local compact state proof exists, but the full assigned "
                    "slice has no integrated DLL/Rust speed-five proof and lacks "
                    "event records for the blocked surfaces"
                ),
                "narrowed_by": [
                    item
                    for item in [
                        (
                            {
                                "proof": replay_proof.get("hoi4replay"),
                                "status": "compact_state_event_rows_verified",
                                "required_counter": "compact_state_event_rows",
                                "proof_counter": replay_data_unit_count(
                                    replay_proof, "compact_state_event_rows"
                                ),
                                "sources": [
                                    "relation_flag",
                                    "relation_opinion",
                                    "subject_autonomy",
                                    "country_faction_group",
                                    "agency_scalar",
                                    "agency_capacity",
                                    "operative_lifecycle",
                                    "cryptology",
                                ],
                                "relation_opinion_counter": replay_data_unit_count(
                                    replay_proof,
                                    "compact_state_event_relation_opinion_rows",
                                ),
                                "country_faction_group_counter": replay_data_unit_count(
                                    replay_proof,
                                    "compact_state_event_country_faction_group_rows",
                                ),
                                "container_integrity_verified": container_integrity_ok(
                                    replay_proof
                                ),
                            }
                            if compact_state_events_ok
                            else None
                        ),
                        (
                            {
                                "proof": status(
                                    state_events_registration or {}, "schema"
                                ),
                                "status": status(
                                    state_events_registration or {}, "status"
                                ),
                                "required_counter": status(
                                    state_events_registration or {},
                                    "validated_registration_details",
                                    "required_counter",
                                ),
                                "recorder_decoder": status(
                                    state_events_registration or {},
                                    "validated_registration_details",
                                    "recorder_decoder",
                                ),
                                "recorder_counter_merge": status(
                                    state_events_registration or {},
                                    "validated_registration_details",
                                    "recorder_counter_merge",
                                ),
                                "master_contract_status": status(
                                    state_events_registration or {},
                                    "validated_registration_details",
                                    "master_contract_status",
                                ),
                                "proof_counter": status(
                                    state_events_registration or {},
                                    "proof_counters",
                                    "compact_state_event_rows",
                                ),
                            }
                            if state_events_registration_ok
                            else None
                        ),
                        (
                            {
                                "proof": status(
                                    operative_lifecycle_registration or {}, "schema"
                                ),
                                "status": status(
                                    operative_lifecycle_registration or {}, "status"
                                ),
                                "required_counter": status(
                                    operative_lifecycle_registration or {},
                                    "validated_registration_details",
                                    "required_counter",
                                ),
                                "recorder_decoder": status(
                                    operative_lifecycle_registration or {},
                                    "validated_registration_details",
                                    "recorder_decoder",
                                ),
                                "collector_lifecycle_source_offset": status(
                                    operative_lifecycle_registration or {},
                                    "validated_registration_details",
                                    "collector_lifecycle_source_offset",
                                ),
                                "proof_counter": status(
                                    operative_lifecycle_registration or {},
                                    "proof_counters",
                                    "operative_lifecycle_rows",
                                ),
                            }
                            if operative_lifecycle_registration_ok
                            else None
                        ),
                        (
                            {
                                "proof": status(
                                    agency_capacity_registration or {}, "schema"
                                ),
                                "status": status(
                                    agency_capacity_registration or {}, "status"
                                ),
                                "required_counter": status(
                                    agency_capacity_registration or {},
                                    "validated_registration_details",
                                    "required_counter",
                                ),
                                "recorder_decoder": status(
                                    agency_capacity_registration or {},
                                    "validated_registration_details",
                                    "recorder_decoder",
                                ),
                                "collector_raw_offsets": status(
                                    agency_capacity_registration or {},
                                    "validated_registration_details",
                                    "collector_raw_offsets",
                                ),
                                "semantic_labels": status(
                                    agency_capacity_registration or {},
                                    "validated_registration_details",
                                    "semantic_labels",
                                ),
                                "proof_counter": status(
                                    agency_capacity_registration or {},
                                    "proof_counters",
                                    "agency_capacity_cluster_rows",
                                ),
                            }
                            if agency_capacity_registration_ok
                            else None
                        ),
                    ]
                    if item
                ],
            },
        }
    )

    return surfaces


def build_report(args: argparse.Namespace) -> tuple[dict[str, Any], int]:
    diplomacy = load_json(Path(args.diplomacy_proof))
    intel = load_json(Path(args.intel_proof))
    war = load_json(Path(args.war_proof))
    operation = load_json(Path(args.operation_proof))
    remaining = load_json(Path(args.remaining_gate))
    replay_proof = load_json(Path(args.replay_proof))
    agency_capacity = load_json(Path(args.agency_capacity_proof)) if args.agency_capacity_proof else {}
    war_date_scalar = (
        load_json(Path(args.war_date_scalar_proof)) if args.war_date_scalar_proof else {}
    )
    operation_definition = (
        load_json(Path(args.operation_definition_proof))
        if args.operation_definition_proof
        else {}
    )
    operative_lifecycle = (
        load_json(Path(args.operative_lifecycle_proof))
        if args.operative_lifecycle_proof
        else {}
    )
    operative_lifecycle_registration = (
        load_json(Path(args.operative_lifecycle_registration_proof))
        if args.operative_lifecycle_registration_proof
        else {}
    )
    agency_capacity_registration = (
        load_json(Path(args.agency_capacity_registration_proof))
        if args.agency_capacity_registration_proof
        else {}
    )
    state_events_registration = (
        load_json(Path(args.state_events_registration_proof))
        if args.state_events_registration_proof
        else {}
    )

    proof_builds = {
        "diplomacy": process_sha(diplomacy),
        "intel": process_sha(intel),
        "war": process_sha(war),
        "operation": process_sha(operation),
        "replay": replay_proof.get("build_sha256"),
        "agency_capacity": process_sha(agency_capacity) if agency_capacity else None,
        "war_date_scalar": process_sha(war_date_scalar) if war_date_scalar else None,
        "operation_definition": (
            process_sha(operation_definition) if operation_definition else None
        ),
        "operative_lifecycle": (
            process_sha(operative_lifecycle) if operative_lifecycle else None
        ),
        "operative_lifecycle_registration": None,
        "state_events_registration": None,
    }
    build_gate_ok = all(
        value == EXPECTED_SHA256 for value in proof_builds.values() if value is not None
    )

    surfaces = build_surface_report(
        diplomacy,
        intel,
        war,
        operation,
        remaining,
        replay_proof,
        agency_capacity,
        war_date_scalar,
        operation_definition,
        operative_lifecycle,
        operative_lifecycle_registration,
        agency_capacity_registration,
        state_events_registration,
    )
    incomplete = [
        row
        for row in surfaces
        if row["status"] in ("blocked", "validated_slice")
    ]
    production_complete = (
        build_gate_ok and not incomplete and production_stream_certified(replay_proof)
    )

    report = {
        "schema": "battle-plan.diplomacy_intel_operatives.full_slice_closure.v1",
        "generated_utc": datetime.now(timezone.utc).isoformat(timespec="seconds"),
        "lane": LANE,
        "domain_id": DOMAIN_ID,
        "completion_status": "COMPLETE" if production_complete else "NOT COMPLETE",
        "backlog_reduced": "yes",
        "exact_backlog_item_removed_or_narrowed": (
            (
                "relations_opinions_and_factions narrowed: compact observed-state "
                "rows for relation opinion and country faction-group values are "
                "implemented and Rust/.battleplan verified; tension, faction "
                "leader/name/lifecycle, diplomacy action queues, and shared "
                "speed-five production proof remain open"
            )
            if compact_relation_faction_events_ok(replay_proof)
            else (
                "shared_integrated_compact_event_state_records narrowed: exact "
                "registration patch details for compact state-event rows are "
                "validated, including domain-53 registry reservation, source-proof "
                "gating, generic Rust decoder merge, compact_state_event_rows "
                "counter preservation, and master checklist rows that remain open"
            )
            if status(state_events_registration, "status")
            == "state_events_registration_patch_validated"
            else (
                "compact_event_state_records narrowed: compact validated-state "
                "event rows are implemented and Rust/.battleplan verified for "
                "relation flag, subject autonomy, agency scalar, agency capacity, "
                "operative lifecycle, and cryptology observations; war lifecycle, "
                "diplomacy queue, running operation, spy-network, and shared "
                "speed-five event streams remain blocked"
            )
            if compact_state_event_rows_ok(replay_proof)
            else (
                "shared_integrated_compact_event_state_records narrowed: exact "
                "registration patch details for raw agency capacity rows are "
                "validated, including collector agency+0xE4/+0xF0/+0xF4 raw "
                "source, domain-53 reserved registry state, generic Rust "
                "decoder merge, and agency_capacity_cluster_rows counter "
                "preservation without semantic relabeling"
            )
            if status(agency_capacity_registration, "status")
            == "agency_capacity_registration_patch_validated"
            else (
                "agency_full_state narrowed: raw agency capacity cluster rows "
                "for agency+0xE4/+0xF0/+0xF4 are implemented and "
                "Rust/.battleplan verified; v16 still rejects the old "
                "+0xE4=max/+0xF0=usable labels and requires same-tick UI/save "
                "evidence before naming +0xF0 or +0xE4/+0xF4 semantics"
            )
            if compact_agency_capacity_cluster_ok(replay_proof)
            and status(agency_capacity, "status") == "legacy_assignment_rejected"
            else (
                "shared_integrated_compact_event_state_records narrowed: exact "
                "registration patch details for compact operative lifecycle rows "
                "are validated, including collector identity+0x110 source, "
                "domain-53 reserved registry state, generic Rust decoder merge, "
                "and operative_lifecycle_rows counter preservation"
            )
            if status(operative_lifecycle_registration, "status")
            == "operative_lifecycle_registration_patch_validated"
            else (
                "operative_full_lifecycle narrowed: compact operative "
                "lifecycle/identity rows are implemented and Rust/.battleplan "
                "verified for lifeflag 47/31/63 cohorts; semantic names, "
                "mission links, traits, nationalities, portraits, and captured "
                "fixtures remain blocked"
            )
            if compact_operative_lifecycle_ok(replay_proof)
            and status(operative_lifecycle, "status")
            == "operative_lifecycle_groups_validated"
            else (
                "operative_full_lifecycle narrowed: current COperativeLeader "
                "identity root and lifeflag cohorts at identity+0x110 are "
                "validated and warm-stable, with semantics still fixture-gated"
            )
            if status(operative_lifecycle, "status")
            == "operative_lifecycle_groups_validated"
            else (
                "operation_detail narrowed: current COperation heap-hit set is "
                "rejected as running operation state because COperationsDatabase+0x60 "
                "contains the full 38-entry definition table matching the prior "
                "full heap COperation count"
            )
            if status(operation_definition, "status")
            == "coperation_heap_hits_rejected_as_running_operation_state"
            else (
                "war_lifecycle_detail narrowed: inline DateHour scalar at "
                "accepted war row offsets row+0x10..row+0x3C is rejected; "
                "full domain 53 residual scope remains executable-gated"
            )
            if status(war_date_scalar, "status") == "inline_date_scalar_offsets_rejected"
            else (
                "full domain 53 residual scope is now executable-gated: accepted "
                "slices are separated from blocked war lifecycle, diplomacy queues, "
                "intel map, operations, operatives lifecycle, spy networks, agency "
                "full state, and compact event/integration records"
            )
        ),
        "expected_sha256": EXPECTED_SHA256,
        "build_gate_ok": build_gate_ok,
        "proof_builds": proof_builds,
        "proof_inputs": {
            "diplomacy_proof": args.diplomacy_proof,
            "intel_proof": args.intel_proof,
            "war_proof": args.war_proof,
            "operation_proof": args.operation_proof,
            "remaining_gate": args.remaining_gate,
            "replay_proof": args.replay_proof,
            "agency_capacity_proof": args.agency_capacity_proof,
            "war_date_scalar_proof": args.war_date_scalar_proof,
            "operation_definition_proof": args.operation_definition_proof,
            "operative_lifecycle_proof": args.operative_lifecycle_proof,
            "operative_lifecycle_registration_proof": args.operative_lifecycle_registration_proof,
            "agency_capacity_registration_proof": args.agency_capacity_registration_proof,
            "state_events_registration_proof": args.state_events_registration_proof,
        },
        "surfaces": surfaces,
        "accepted_slices": [
            {
                "surface": row["surface"],
                "validated_slice": row["validated_slice"],
            }
            for row in surfaces
            if row.get("validated_slice")
        ],
        "blockers": [
            {
                "surface": row["surface"],
                "status": row["status"],
                "missing_extractable_fields": row["missing_extractable_fields"],
                "blocker": row["blocker"],
            }
            for row in surfaces
            if row["status"] in ("blocked", "validated_slice")
        ],
        "dll_only": "no",
        "compact_stream": "lane-local synthetic state fixture yes; production event/state stream no",
        "rust_decoded": "lane-local yes; shared recorder no",
        "replay_container_integrity_verified": container_integrity_ok(replay_proof),
        "production_stream_certified": production_stream_certified(replay_proof),
        "speed_five_current_equivalent_hourly_or_lower": "no",
        "zero_skipped_hours": (
            bool(status(replay_proof, "skipped_game_hours") == 0)
            if "skipped_game_hours" in replay_proof
            else "lane-local synthetic single-hour stream; no source cadence claim"
        ),
        "partial_source_worker_bridge_failures": "no in lane-local proof; no shared DLL proof",
    }
    return report, 0 if production_complete else 2


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--diplomacy-proof", required=True)
    parser.add_argument("--intel-proof", required=True)
    parser.add_argument("--war-proof", required=True)
    parser.add_argument("--operation-proof", required=True)
    parser.add_argument("--remaining-gate", required=True)
    parser.add_argument("--replay-proof", required=True)
    parser.add_argument("--agency-capacity-proof")
    parser.add_argument("--war-date-scalar-proof")
    parser.add_argument("--operation-definition-proof")
    parser.add_argument("--operative-lifecycle-proof")
    parser.add_argument("--operative-lifecycle-registration-proof")
    parser.add_argument("--agency-capacity-registration-proof")
    parser.add_argument("--state-events-registration-proof")
    parser.add_argument("--out", required=True)
    args = parser.parse_args(argv)

    report, code = build_report(args)
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    with out.open("w", encoding="utf-8") as f:
        json.dump(report, f, indent=2, sort_keys=True)
        f.write("\n")
    print(
        "full_slice_closure_gate: "
        f"status={report['completion_status']} blockers={len(report['blockers'])} out={out}"
    )
    return code


if __name__ == "__main__":
    raise SystemExit(main())
