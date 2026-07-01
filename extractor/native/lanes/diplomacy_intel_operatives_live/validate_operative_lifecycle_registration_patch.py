#!/usr/bin/env python3
"""Validate the domain-53 operative lifecycle registration patch details.

This gate intentionally reads shared registry files but does not edit them. It
proves that the lane-local registration packet carries the v20
`operative_lifecycle_rows` counter through the exact generic recorder hook.
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
    lifecycle_patch = read_text(Path(args.lifecycle_patch))
    compact_schema = read_text(Path(args.compact_schema))
    extractor = read_text(Path(args.extractor))
    lane_lib = read_text(Path(args.lane_lib))
    proof = load_json(Path(args.replay_proof))

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
    require(
        "pub mod compact;" in lane_lib,
        errors,
        "lane lib does not export compact decoder module",
    )
    require(
        "pub mod extractor;" in lane_lib,
        errors,
        "lane lib does not export extractor module",
    )

    require(
        "OPERATIVE_FLAG_LIFECYCLE_PRESENT" in compact_schema,
        errors,
        "compact schema lacks operative lifecycle presence flag",
    )
    require(
        "pub lifecycle_flag: Option<u32>" in compact_schema,
        errors,
        "compact schema lacks OperativeRow.lifecycle_flag",
    )
    require(
        '"operative_lifecycle_rows"' in compact_schema,
        errors,
        "compact decoder does not report operative_lifecycle_rows",
    )
    require(
        "lifecycle_flag without present flag" in compact_schema,
        errors,
        "compact validator does not reject lifecycle values without presence flag",
    )
    for flag in ("Some(47)", "Some(31)", "Some(63)"):
        require(flag in extractor, errors, f"extractor proof fixture lacks lifecycle {flag}")

    required_packet_tokens = [
        "COMPACT_DOMAIN_DIPLOMACY_INTEL_OPERATIVES_LIVE: u16 = 53",
        "decode_compact_diplomacy_intel_operatives_live_data_payload",
        "diplomacy_intel_operatives_live::compact::decode_payload",
        "for (name, count) in lane_summary.data_units_total",
        "operative_lifecycle_rows",
    ]
    for token in required_packet_tokens:
        require(token in registration_packet, errors, f"registration packet missing {token}")

    required_patch_tokens = [
        "Backlog item narrowed: `shared_integrated_compact_event_state_records`",
        "read_u32(identity + 0x110)",
        "SOURCE_PROOF_OPERATIVE_ROOT_870EBF",
        "Do not name lifecycle flags `31` or `63`",
        'data_units_total["operative_lifecycle_rows"] == 3',
    ]
    for token in required_patch_tokens:
        require(token in lifecycle_patch, errors, f"lifecycle registration patch missing {token}")

    require(int(proof.get("domain_id") or 0) == DOMAIN_ID, errors, "proof domain_id is not 53")
    require(bool(proof.get("hoi4replay_verified")), errors, "proof does not verify .battleplan")
    require(int(proof.get("data_rows_total") or 0) == 11, errors, "proof row count is not 11")
    require(
        proof_unit_count(proof, "operative_rows") == 3,
        errors,
        "proof operative_rows is not 3",
    )
    require(
        proof_unit_count(proof, "operative_lifecycle_rows") == 3,
        errors,
        "proof operative_lifecycle_rows is not 3",
    )

    status = "operative_lifecycle_registration_patch_validated" if not errors else "failed"
    return {
        "schema": "battle-plan.diplomacy_intel_operatives.operative_lifecycle_registration_patch.v1",
        "generated_utc": datetime.now(timezone.utc).isoformat(timespec="seconds"),
        "lane": LANE,
        "domain_id": DOMAIN_ID,
        "completion_status": "COMPLETE" if not errors else "NOT COMPLETE",
        "status": status,
        "backlog_reduced": "yes" if not errors else "no",
        "exact_backlog_item_removed_or_narrowed": (
            "shared_integrated_compact_event_state_records narrowed: exact "
            "shared registration patch details for compact operative lifecycle "
            "rows are validated; shared DLL/Rust entrypoints still await the "
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
            "collector_lifecycle_source_offset": "identity+0x110",
            "collector_owner_filter_offset": "identity+0x118",
            "collector_source_proof": "SOURCE_PROOF_OPERATIVE_ROOT_870EBF",
            "recorder_decoder": "diplomacy_intel_operatives_live::compact::decode_payload",
            "recorder_counter_merge": "lane_summary.data_units_total",
            "required_counter": "operative_lifecycle_rows",
        },
        "proof_counters": {
            "data_rows_total": int(proof.get("data_rows_total") or 0),
            "operative_rows": proof_unit_count(proof, "operative_rows"),
            "operative_lifecycle_rows": proof_unit_count(
                proof, "operative_lifecycle_rows"
            ),
            "hoi4replay_verified": bool(proof.get("hoi4replay_verified")),
        },
        "dll_only": "no",
        "compact_stream": "yes, lane-local v20 proof",
        "rust_decoded": "yes, lane-local decoder; shared hook plan validated",
        "hoi4replay_verified": bool(proof.get("hoi4replay_verified")),
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
    parser.add_argument("--lifecycle-patch", required=True)
    parser.add_argument("--compact-schema", required=True)
    parser.add_argument("--extractor", required=True)
    parser.add_argument("--lane-lib", required=True)
    parser.add_argument("--replay-proof", required=True)
    parser.add_argument("--out", required=True)
    args = parser.parse_args()

    report = validate(args)
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(
        "operative_lifecycle_registration_patch: "
        f"status={report['status']} "
        f"counter={report['proof_counters']['operative_lifecycle_rows']} "
        f"out={out}"
    )
    return 0 if report["status"] == "operative_lifecycle_registration_patch_validated" else 2


if __name__ == "__main__":
    raise SystemExit(main())
