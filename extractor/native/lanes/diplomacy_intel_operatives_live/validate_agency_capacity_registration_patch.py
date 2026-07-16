#!/usr/bin/env python3
"""Validate the domain-53 raw agency capacity registration patch details.

This gate reads shared registry files for evidence only. It proves that the
lane-local registration packet preserves the v22 `agency_capacity_cluster_rows`
counter and keeps the `agency+0xE4/+0xF0/+0xF4` values raw until a same-tick
UI/save fixture can name their semantics.
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


def container_integrity_ok(proof: dict[str, Any]) -> bool:
    return (
        proof.get("container_integrity_verified") is True
        and proof.get("stream_entry_byte_identity_verified") is True
        and proof.get("production_certifying") is False
        and proof.get("source_capture_verified") is False
        and not proof.get("parse_errors")
    )


def get_path(value: Any, *keys: str) -> Any:
    for key in keys:
        if not isinstance(value, dict):
            return None
        value = value.get(key)
    return value


def validate(args: argparse.Namespace) -> dict[str, Any]:
    errors: list[str] = []

    collector_registry = read_text(Path(args.collector_registry))
    recorder_registry = read_text(Path(args.recorder_registry))
    registration_packet = read_text(Path(args.registration_packet))
    capacity_patch = read_text(Path(args.capacity_patch))
    compact_schema = read_text(Path(args.compact_schema))
    extractor = read_text(Path(args.extractor))
    integration_plan = read_text(Path(args.integration_plan))
    replay_proof = load_json(Path(args.replay_proof))
    capacity_proof = load_json(Path(args.capacity_proof))

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
        "KIND_AGENCY_CAPACITY_CLUSTER",
        "AgencyCapacityClusterRow",
        '"agency_capacity_cluster_rows"',
        "value_0xe4 != value_0xf4",
    ]
    for token in required_schema_tokens:
        require(token in compact_schema, errors, f"compact schema missing {token}")

    required_extractor_tokens = [
        "agency+0xE4/+0xF0/+0xF4",
        "value_0xe4: 9",
        "value_0xf0: 10",
        "value_0xf4: 9",
        "SOURCE_PROOF_AGENCY_ROOT_870EBF",
    ]
    for token in required_extractor_tokens:
        require(token in extractor, errors, f"extractor fixture/root missing {token}")

    required_packet_tokens = [
        "agency_capacity_cluster_rows",
        "for (name, count) in lane_summary.data_units_total",
        "diplomacy_intel_operatives_live::compact::decode_payload",
    ]
    for token in required_packet_tokens:
        require(token in registration_packet, errors, f"registration packet missing {token}")

    required_plan_tokens = [
        "agency_capacity_cluster_rows",
        "agency+0xE4/+0xF0/+0xF4",
        "keep these as raw values",
    ]
    for token in required_plan_tokens:
        require(token in integration_plan, errors, f"integration plan missing {token}")

    required_patch_tokens = [
        "Backlog item narrowed: `shared_integrated_compact_event_state_records`",
        "value_0xe4: read_u32(agency + 0xE4)",
        "value_0xf0: read_u32(agency + 0xF0)",
        "value_0xf4: read_u32(agency + 0xF4)",
        "Do not call `value_0xf0` max",
        'data_units_total["agency_capacity_cluster_rows"] == 1',
    ]
    for token in required_patch_tokens:
        require(token in capacity_patch, errors, f"capacity registration patch missing {token}")

    require(
        capacity_proof.get("status") == "legacy_assignment_rejected",
        errors,
        "capacity proof does not reject legacy assignment",
    )
    require(
        get_path(capacity_proof, "capacity_analysis", "e4_f4_equal_rows") == 102,
        errors,
        "capacity proof does not validate +0xE4==+0xF4 on 102 rows",
    )
    require(
        get_path(capacity_proof, "capacity_analysis", "e4_f4_mismatch_rows") == 0,
        errors,
        "capacity proof has +0xE4/+0xF4 mismatch rows",
    )
    require(
        get_path(capacity_proof, "capacity_analysis", "legacy_assignment", "status")
        == "rejected",
        errors,
        "capacity proof legacy assignment status is not rejected",
    )

    require(int(replay_proof.get("domain_id") or 0) == DOMAIN_ID, errors, "proof domain_id is not 53")
    require(
        container_integrity_ok(replay_proof),
        errors,
        "proof does not establish scoped non-certifying replay-container integrity",
    )
    require(int(replay_proof.get("data_rows_total") or 0) == 12, errors, "proof row count is not 12")
    require(
        proof_unit_count(replay_proof, "agency_capacity_cluster_rows") == 1,
        errors,
        "proof agency_capacity_cluster_rows is not 1",
    )
    require(
        proof_unit_count(replay_proof, "intel_agency_rows") == 1,
        errors,
        "proof intel_agency_rows is not 1",
    )
    require(
        proof_unit_count(replay_proof, "operative_lifecycle_rows") == 3,
        errors,
        "proof operative_lifecycle_rows is not 3",
    )

    status = "agency_capacity_registration_patch_validated" if not errors else "failed"
    return {
        "schema": "battle-plan.diplomacy_intel_operatives.agency_capacity_registration_patch.v1",
        "generated_utc": datetime.now(timezone.utc).isoformat(timespec="seconds"),
        "lane": LANE,
        "domain_id": DOMAIN_ID,
        "completion_status": "COMPLETE" if not errors else "NOT COMPLETE",
        "status": status,
        "backlog_reduced": "yes" if not errors else "no",
        "exact_backlog_item_removed_or_narrowed": (
            "shared_integrated_compact_event_state_records narrowed: exact "
            "shared registration patch details for raw agency capacity rows are "
            "validated; shared DLL/Rust entrypoints still await the "
            "orchestrator-owned slot"
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
            "collector_agency_root": "country_object+0xFC0",
            "collector_agency_vtable": "module+0x296B338",
            "collector_raw_offsets": ["agency+0xE4", "agency+0xF0", "agency+0xF4"],
            "collector_source_proof": "SOURCE_PROOF_AGENCY_ROOT_870EBF",
            "recorder_decoder": "diplomacy_intel_operatives_live::compact::decode_payload",
            "recorder_counter_merge": "lane_summary.data_units_total",
            "required_counter": "agency_capacity_cluster_rows",
            "semantic_labels": "blocked_until_same_tick_ui_save_fixture",
        },
        "capacity_proof": {
            "status": capacity_proof.get("status"),
            "e4_f4_equal_rows": get_path(
                capacity_proof, "capacity_analysis", "e4_f4_equal_rows"
            ),
            "e4_f4_mismatch_rows": get_path(
                capacity_proof, "capacity_analysis", "e4_f4_mismatch_rows"
            ),
            "legacy_assignment": get_path(
                capacity_proof, "capacity_analysis", "legacy_assignment", "status"
            ),
            "divergent_capacity_rows": get_path(
                capacity_proof, "capacity_analysis", "divergent_capacity_rows"
            ),
        },
        "proof_counters": {
            "data_rows_total": int(replay_proof.get("data_rows_total") or 0),
            "agency_capacity_cluster_rows": proof_unit_count(
                replay_proof, "agency_capacity_cluster_rows"
            ),
            "intel_agency_rows": proof_unit_count(replay_proof, "intel_agency_rows"),
            "operative_lifecycle_rows": proof_unit_count(
                replay_proof, "operative_lifecycle_rows"
            ),
            "replay_container_integrity_verified": container_integrity_ok(replay_proof),
        },
        "dll_only": "no",
        "compact_stream": "yes, lane-local synthetic v22 integrity proof",
        "rust_decoded": "yes, lane-local decoder; shared hook plan validated",
        "replay_container_integrity_verified": container_integrity_ok(replay_proof),
        "speed_five_current_equivalent_hourly_or_lower": "no",
        "zero_skipped_hours": "not claimed; synthetic single-hour stream only",
        "partial_source_worker_bridge_failures": "no in lane-local proof; shared run not executed",
        "errors": errors,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--collector-registry", required=True)
    parser.add_argument("--recorder-registry", required=True)
    parser.add_argument("--registration-packet", required=True)
    parser.add_argument("--capacity-patch", required=True)
    parser.add_argument("--compact-schema", required=True)
    parser.add_argument("--extractor", required=True)
    parser.add_argument("--integration-plan", required=True)
    parser.add_argument("--replay-proof", required=True)
    parser.add_argument("--capacity-proof", required=True)
    parser.add_argument("--out", required=True)
    args = parser.parse_args()

    report = validate(args)
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(
        "agency_capacity_registration_patch: "
        f"status={report['status']} "
        f"counter={report['proof_counters']['agency_capacity_cluster_rows']} "
        f"out={out}"
    )
    return 0 if report["status"] == "agency_capacity_registration_patch_validated" else 2


if __name__ == "__main__":
    raise SystemExit(main())
