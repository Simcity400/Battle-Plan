"""Validate that direct_land_oob is ready for an integration slot.

This gate does not edit shared collector/recorder entrypoints. It checks that
the lane has enough current-build evidence, a stable compact contract, and an
explicit no-hot-loop-heap-sweep plan so the future slot can proceed without more
root discovery.
"""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path
from typing import Any

from direct_oob_proof_gate import validate as validate_direct_proof


EXPECTED_SCHEMA = "direct_land_oob_integration_slot_v1"
EXPECTED_DOMAIN_ID = 44
EXPECTED_HEADER_LEN = 32
EXPECTED_ROW_LEN = 64
EXPECTED_MEMBER_ROW_LEN = 8
EXPECTED_KINDS = {"army": 1, "army_group": 2, "theatre": 5}
EXPECTED_STATUS = {"cold_preflight_only": 5}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("contract_json", type=Path)
    args = ap.parse_args()

    contract = json.loads(args.contract_json.read_text(encoding="utf-8"))
    errors = validate_contract(contract, args.contract_json.parent)
    print(json.dumps({"status": "PASS" if not errors else "FAIL", "errors": errors}, indent=2, sort_keys=True))
    return 0 if not errors else 1


def validate_contract(contract: dict[str, Any], contract_dir: Path) -> list[str]:
    errors: list[str] = []
    if contract.get("schema") != EXPECTED_SCHEMA:
        errors.append(f"schema must be {EXPECTED_SCHEMA}")

    proof_path = resolve_path(contract.get("direct_root_proof_json"), contract_dir)
    if proof_path is None or not proof_path.exists():
        errors.append("direct_root_proof_json must point to an existing proof JSON")
    else:
        proof = json.loads(proof_path.read_text(encoding="utf-8"))
        errors.extend(
            f"direct root proof: {err}"
            for err in validate_direct_proof(
                proof,
                min_empty_direct_armies=1,
                min_theatres=1,
                min_non_type5_orders=1,
            )
        )
        counts = proof.get("summary", {}).get("counts", {})
        minimums = contract.get("proof_minimums", {})
        for key in ("direct_armies", "empty_or_inactive_direct_armies", "theatres"):
            if int(counts.get(key, 0)) < int(minimums.get(key, 0)):
                errors.append(f"proof count {key} below contract minimum")

    compact = contract.get("compact_schema", {})
    if compact.get("domain_id") != EXPECTED_DOMAIN_ID:
        errors.append("compact_schema.domain_id must be 44")
    if compact.get("header_len") != EXPECTED_HEADER_LEN:
        errors.append("compact_schema.header_len must be 32")
    if compact.get("row_len") != EXPECTED_ROW_LEN:
        errors.append("compact_schema.row_len must be 64")
    if compact.get("member_row_len") != EXPECTED_MEMBER_ROW_LEN:
        errors.append("compact_schema.member_row_len must be 8")
    if compact.get("kinds") != EXPECTED_KINDS:
        errors.append("compact_schema.kinds must match army=1, army_group=2, theatre=5")
    if compact.get("status_codes", {}).get("cold_preflight_only") != EXPECTED_STATUS["cold_preflight_only"]:
        errors.append("compact_schema.status_codes.cold_preflight_only must be 5")

    schema_path = resolve_path(contract.get("compact_schema_source"), contract_dir)
    if schema_path is None or not schema_path.exists():
        errors.append("compact_schema_source must point to direct_oob_compact.rs")
    else:
        errors.extend(validate_rust_constants(schema_path))

    collector = contract.get("collector_slot", {})
    if collector.get("shared_entrypoint_slot_required") is not True:
        errors.append("collector_slot.shared_entrypoint_slot_required must be true")
    if collector.get("hot_loop_heap_sweep") is not False:
        errors.append("collector_slot.hot_loop_heap_sweep must be false")
    if collector.get("bounded_root_refresh") is not True:
        errors.append("collector_slot.bounded_root_refresh must be true")
    if collector.get("fail_closed_on_layout_mismatch") is not True:
        errors.append("collector_slot.fail_closed_on_layout_mismatch must be true")
    if collector.get("root_refresh_mode") not in ("preflight_cache_refresh", "bounded_preflight_cache_refresh"):
        errors.append("collector_slot.root_refresh_mode must be a bounded preflight/cache refresh")

    recorder = contract.get("recorder_slot", {})
    if recorder.get("additive_domain_only") is not True:
        errors.append("recorder_slot.additive_domain_only must be true")
    if recorder.get("do_not_change_domains") != [27, 28]:
        errors.append("recorder_slot.do_not_change_domains must be [27, 28]")

    required_gates = set(contract.get("required_post_slot_gates", []))
    for gate in (
        "direct_land_oob_compact_decode",
        "compact_stream_ordered",
        "hoi4replay_verify",
        "zero_skipped_hours",
        "no_source_partial_bridge_worker_failures",
    ):
        if gate not in required_gates:
            errors.append(f"required_post_slot_gates missing {gate}")
    return errors


def validate_rust_constants(path: Path) -> list[str]:
    text = path.read_text(encoding="utf-8")
    checks = {
        "DOMAIN_DIRECT_LAND_OOB": EXPECTED_DOMAIN_ID,
        "HEADER_LEN": EXPECTED_HEADER_LEN,
        "ROW_LEN": EXPECTED_ROW_LEN,
        "MEMBER_ROW_LEN": EXPECTED_MEMBER_ROW_LEN,
        "KIND_ARMY": EXPECTED_KINDS["army"],
        "KIND_ARMY_GROUP": EXPECTED_KINDS["army_group"],
        "KIND_THEATRE": EXPECTED_KINDS["theatre"],
        "STATUS_COLD_PREFLIGHT_ONLY": EXPECTED_STATUS["cold_preflight_only"],
    }
    errors: list[str] = []
    for name, expected in checks.items():
        match = re.search(rf"pub const {name}: [^=]+=\s*([^;]+);", text)
        if match is None:
            errors.append(f"{path.name} missing {name}")
            continue
        try:
            value = int(match.group(1).replace("_", "").strip(), 0)
        except ValueError:
            errors.append(f"{path.name} has unparsable {name}")
            continue
        if value != expected:
            errors.append(f"{path.name} {name}={value}, expected {expected}")
    return errors


def resolve_path(value: Any, base_dir: Path) -> Path | None:
    if not isinstance(value, str) or not value:
        return None
    path = Path(value)
    if path.is_absolute():
        return path
    return (base_dir / path).resolve()


if __name__ == "__main__":
    raise SystemExit(main())
