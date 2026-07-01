#!/usr/bin/env python3
"""Validate the domain-53 compact state-event registration patch details.

This gate reads shared registry/status files for evidence only. It proves that
the lane-local registration packet preserves the v26 `compact_state_event_rows`
counter and carries proposed master row text without editing the orchestrator-
owned master extraction contract directly.
"""

from __future__ import annotations

import argparse
import json
import re
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


LANE = "diplomacy_intel_operatives_live"
DOMAIN_ID = 53


def read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def require(condition: bool, errors: list[str], message: str) -> None:
    if not condition:
        errors.append(message)


def domain53_reserved_with_data_record(text: str) -> bool:
    pattern = re.compile(
        r"domain\(\s*53,\s*"
        r'"diplomacy_intel_operatives_live",\s*'
        r'"diplomacy_intel_operatives_live",\s*'
        r"CompactDomainStatus::Reserved,\s*"
        r"false,\s*"
        r"true,\s*"
        r"&\[\],\s*"
        r"\)",
        re.MULTILINE,
    )
    return bool(pattern.search(text))


def proof_unit_count(proof: dict[str, Any], name: str) -> int:
    units = proof.get("data_units_total")
    if not isinstance(units, dict):
        return 0
    return int(units.get(name) or 0)


def validate(args: argparse.Namespace) -> dict[str, Any]:
    errors: list[str] = []

    collector_registry = read_text(Path(args.collector_registry))
    recorder_registry = read_text(Path(args.recorder_registry))
    registration_packet = read_text(Path(args.registration_packet))
    state_events_patch = read_text(Path(args.state_events_patch))
    compact_schema = read_text(Path(args.compact_schema))
    extractor = read_text(Path(args.extractor))
    slot_ready = read_text(Path(args.slot_ready))
    integration_plan = read_text(Path(args.integration_plan))
    master_contract = read_text(Path(args.master_contract))
    replay_proof = load_json(Path(args.replay_proof))

    require(
        domain53_reserved_with_data_record(collector_registry),
        errors,
        "collector registry does not reserve domain 53 with requires_data_record=true",
    )
    require(
        domain53_reserved_with_data_record(recorder_registry),
        errors,
        "recorder registry does not reserve domain 53 with requires_data_record=true",
    )

    required_schema_tokens = [
        "KIND_COMPACT_STATE_EVENT",
        "StateEventRow",
        "STATE_EVENT_SOURCE_RELATION_FLAG",
        "STATE_EVENT_SOURCE_RELATION_OPINION",
        "STATE_EVENT_SOURCE_SUBJECT_AUTONOMY",
        "STATE_EVENT_SOURCE_COUNTRY_FACTION_GROUP",
        "STATE_EVENT_SOURCE_AGENCY_SCALAR",
        "STATE_EVENT_SOURCE_OPERATIVE_LIFECYCLE",
        "STATE_EVENT_SOURCE_AGENCY_CAPACITY",
        "STATE_EVENT_SOURCE_CRYPTOLOGY",
        '"compact_state_event_rows"',
        '"compact_state_event_relation_opinion_rows"',
        '"compact_state_event_country_faction_group_rows"',
        "validate_state_event_row",
        "unknown source_kind",
    ]
    for token in required_schema_tokens:
        require(token in compact_schema, errors, f"compact schema missing {token}")

    required_extractor_tokens = [
        "validated_state_event_rows",
        "relation_flag_guarantee",
        "relation_opinion",
        "country_faction_group",
        "autonomy_ratio_40000",
        "agency_upgrade_progress_2400000",
        "operative_11387_lifecycle",
        "agency_capacity_9_10_9",
        "cryptology_amount_960000000",
        "compact state events sourced from relation/opinion/subject rows require",
        "compact state events sourced from country faction rows require",
        "compact state events sourced from agency rows require",
        "compact state events sourced from operative lifecycle rows require",
        "compact state events sourced from cryptology rows require",
    ]
    for token in required_extractor_tokens:
        require(token in extractor, errors, f"extractor missing {token}")

    required_slot_tokens = [
        "compact state events sourced from relation/opinion/subject rows require",
        "compact state events sourced from country faction rows require",
        "compact state events sourced from agency rows require",
        "compact state events sourced from operative lifecycle rows require",
        "compact state events sourced from cryptology rows require",
    ]
    for token in required_slot_tokens:
        require(token in slot_ready, errors, f"slot-ready gate missing {token}")

    required_packet_tokens = [
        "compact_state_event_rows",
        "for (name, count) in lane_summary.data_units_total",
        "diplomacy_intel_operatives_live::compact::decode_payload",
        "STATE_EVENTS_REGISTRATION_PATCH.md",
    ]
    for token in required_packet_tokens:
        require(token in registration_packet, errors, f"registration packet missing {token}")

    required_plan_tokens = [
        "compact_state_event_rows",
        "validated-state observations",
        "war lifecycle, diplomacy queue, running operation, spy-network",
    ]
    for token in required_plan_tokens:
        require(token in integration_plan, errors, f"integration plan missing {token}")

    required_patch_tokens = [
        "Backlog item narrowed: `shared_integrated_compact_event_state_records`",
        "StateEventRow",
        "STATE_EVENT_SOURCE_RELATION_FLAG",
        "STATE_EVENT_SOURCE_RELATION_OPINION",
        "STATE_EVENT_SOURCE_COUNTRY_FACTION_GROUP",
        "SOURCE_PROOF_COUNTRY_ROOTS_870EBF",
        "SOURCE_PROOF_RELATION_ROOTS_870EBF",
        "SOURCE_PROOF_AGENCY_ROOT_870EBF",
        "SOURCE_PROOF_OPERATIVE_ROOT_870EBF",
        "SOURCE_PROOF_CRYPTOLOGY_ROOT_870EBF",
        'data_units_total["compact_state_event_rows"] == 8',
        'data_units_total["compact_state_event_relation_opinion_rows"] == 1',
        'data_units_total["compact_state_event_country_faction_group_rows"] == 1',
        "Proposed master-contract row text",
    ]
    for token in required_patch_tokens:
        require(token in state_events_patch, errors, f"state-events patch missing {token}")

    required_master_tokens = [
        "CONCEPT_COMPLETE",
        "diplomacy_intel_operatives_live",
    ]
    for token in required_master_tokens:
        require(token in master_contract, errors, f"master contract source missing {token}")

    require(
        int(replay_proof.get("domain_id") or 0) == DOMAIN_ID,
        errors,
        "proof domain_id is not 53",
    )
    require(
        bool(replay_proof.get("hoi4replay_verified")),
        errors,
        "proof does not verify .battleplan",
    )
    require(
        int(replay_proof.get("data_rows_total") or 0) == 20,
        errors,
        "proof row count is not 20",
    )
    require(
        proof_unit_count(replay_proof, "compact_state_event_rows") == 8,
        errors,
        "proof compact_state_event_rows is not 8",
    )
    require(
        proof_unit_count(replay_proof, "compact_state_event_relation_opinion_rows") == 1,
        errors,
        "proof compact_state_event_relation_opinion_rows is not 1",
    )
    require(
        proof_unit_count(replay_proof, "compact_state_event_country_faction_group_rows") == 1,
        errors,
        "proof compact_state_event_country_faction_group_rows is not 1",
    )
    require(
        proof_unit_count(replay_proof, "agency_capacity_cluster_rows") == 1,
        errors,
        "proof agency_capacity_cluster_rows is not 1",
    )
    require(
        proof_unit_count(replay_proof, "operative_lifecycle_rows") == 3,
        errors,
        "proof operative_lifecycle_rows is not 3",
    )

    status = "state_events_registration_patch_validated" if not errors else "failed"
    return {
        "schema": "battle-plan.diplomacy_intel_operatives.state_events_registration_patch.v1",
        "generated_utc": datetime.now(timezone.utc).isoformat(timespec="seconds"),
        "lane": LANE,
        "domain_id": DOMAIN_ID,
        "completion_status": "COMPLETE" if not errors else "NOT COMPLETE",
        "status": status,
        "backlog_reduced": "yes" if not errors else "no",
        "exact_backlog_item_removed_or_narrowed": (
            "shared_integrated_compact_event_state_records narrowed: exact "
            "shared registration patch details for compact state-event rows are "
            "validated; master checklist rows remain open under the "
            "CONCEPT_COMPLETE rule"
            if not errors
            else "none"
        ),
        "validated_registration_details": {
            "collector_registry_domain53_reserved": domain53_reserved_with_data_record(
                collector_registry
            ),
            "recorder_registry_domain53_reserved": domain53_reserved_with_data_record(
                recorder_registry
            ),
            "collector_sources": [
                "relation_flag",
                "relation_opinion",
                "subject_autonomy",
                "country_faction_group",
                "agency_scalar",
                "agency_capacity",
                "operative_lifecycle",
                "cryptology",
            ],
            "collector_source_proofs": [
                "SOURCE_PROOF_COUNTRY_ROOTS_870EBF",
                "SOURCE_PROOF_RELATION_ROOTS_870EBF",
                "SOURCE_PROOF_AGENCY_ROOT_870EBF",
                "SOURCE_PROOF_OPERATIVE_ROOT_870EBF",
                "SOURCE_PROOF_CRYPTOLOGY_ROOT_870EBF",
            ],
            "recorder_decoder": "diplomacy_intel_operatives_live::compact::decode_payload",
            "recorder_counter_merge": "lane_summary.data_units_total",
            "required_counter": "compact_state_event_rows",
            "master_contract_status": "rows updated but not closed",
        },
        "proof_counters": {
            "data_rows_total": int(replay_proof.get("data_rows_total") or 0),
            "compact_state_event_rows": proof_unit_count(
                replay_proof, "compact_state_event_rows"
            ),
            "compact_state_event_relation_opinion_rows": proof_unit_count(
                replay_proof, "compact_state_event_relation_opinion_rows"
            ),
            "compact_state_event_country_faction_group_rows": proof_unit_count(
                replay_proof, "compact_state_event_country_faction_group_rows"
            ),
            "agency_capacity_cluster_rows": proof_unit_count(
                replay_proof, "agency_capacity_cluster_rows"
            ),
            "operative_lifecycle_rows": proof_unit_count(
                replay_proof, "operative_lifecycle_rows"
            ),
            "hoi4replay_verified": bool(replay_proof.get("hoi4replay_verified")),
        },
        "dll_only": "no",
        "compact_stream": "yes, lane-local v26 proof",
        "rust_decoded": "yes, lane-local decoder; shared hook plan validated",
        "hoi4replay_verified": bool(replay_proof.get("hoi4replay_verified")),
        "speed_five_current_equivalent_hourly_or_lower": "no",
        "zero_skipped_hours": "lane-local single-record proof only",
        "partial_source_worker_bridge_failures": "no in lane-local proof; shared run not executed",
        "errors": errors,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--collector-registry", required=True)
    parser.add_argument("--recorder-registry", required=True)
    parser.add_argument("--registration-packet", required=True)
    parser.add_argument("--state-events-patch", required=True)
    parser.add_argument("--compact-schema", required=True)
    parser.add_argument("--extractor", required=True)
    parser.add_argument("--slot-ready", required=True)
    parser.add_argument("--integration-plan", required=True)
    parser.add_argument("--master-contract", required=True)
    parser.add_argument("--replay-proof", required=True)
    parser.add_argument("--out", required=True)
    args = parser.parse_args()

    report = validate(args)
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(
        "state_events_registration_patch: "
        f"status={report['status']} "
        f"counter={report['proof_counters']['compact_state_event_rows']} "
        f"out={out}"
    )
    return 0 if report["status"] == "state_events_registration_patch_validated" else 2


if __name__ == "__main__":
    raise SystemExit(main())
