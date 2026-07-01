"""Gate army-level battleplan scalar probe reports."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


EXPECTED_SCHEMA = "army_oob_orders_battleplans_army_scalar_probe_v1"
EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
FIELDS = ("execution_type", "cohesion_type", "proximity_type")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("proof_json", type=Path)
    ap.add_argument("--require-validated", action="append", default=[])
    ap.add_argument("--allow-ambiguous", action="store_true")
    ap.add_argument("--allow-near-match", action="store_true")
    ap.add_argument("--min-matched-armies", type=int, default=32)
    args = ap.parse_args()

    proof = json.loads(args.proof_json.read_text(encoding="utf-8"))
    errors = validate(
        proof,
        required_validated=args.require_validated,
        allow_ambiguous=args.allow_ambiguous,
        allow_near_match=args.allow_near_match,
        min_matched_armies=args.min_matched_armies,
    )
    result = {
        "status": "PASS" if not errors else "FAIL",
        "candidate_status": proof.get("candidate_status") if not errors else "FAIL",
        "errors": errors,
        "matched_armies": proof.get("matched_armies"),
        "field_statuses": {
            field: proof.get("field_results", {}).get(field, {}).get("status")
            for field in FIELDS
        },
        "accepted_offsets": {
            field: proof.get("field_results", {}).get(field, {}).get("accepted_offsets", [])
            for field in FIELDS
        },
        "narrowed_offsets": {
            field: proof.get("field_results", {}).get(field, {}).get("narrowed_offsets", [])
            for field in FIELDS
        },
        "production_position": proof.get("production_position"),
    }
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0 if not errors else 1


def validate(
    proof: dict[str, Any],
    *,
    required_validated: list[str],
    allow_ambiguous: bool,
    allow_near_match: bool,
    min_matched_armies: int,
) -> list[str]:
    errors: list[str] = []
    if proof.get("schema") != EXPECTED_SCHEMA:
        errors.append("unexpected schema")
    if str(proof.get("exe_sha256", "")).lower() != EXPECTED_SHA256:
        errors.append("unexpected executable sha256")
    if proof.get("errors"):
        errors.append(f"probe reported errors: {proof.get('errors')}")
    if int(proof.get("matched_armies", 0)) < min_matched_armies:
        errors.append("not enough matched armies")
    fields = proof.get("field_results")
    if not isinstance(fields, dict):
        return errors + ["missing field_results"]
    for field in FIELDS:
        row = fields.get(field)
        if not isinstance(row, dict):
            errors.append(f"missing {field} result")
            continue
        errors.extend(
            validate_field(
                field,
                row,
                allow_ambiguous=allow_ambiguous,
                allow_near_match=allow_near_match,
            )
        )
    for field in required_validated:
        row = fields.get(field)
        if not isinstance(row, dict):
            errors.append(f"missing required field {field}")
        elif row.get("status") != "VALIDATED_INLINE_ARMY_SCALAR":
            errors.append(f"{field} is not validated")
    return errors


def validate_field(
    field: str,
    row: dict[str, Any],
    *,
    allow_ambiguous: bool,
    allow_near_match: bool,
) -> list[str]:
    errors: list[str] = []
    status = row.get("status")
    tested = int(row.get("tested_armies", 0))
    if tested <= 0:
        errors.append(f"{field} tested no armies")
    save_hist = row.get("save_value_histogram", {})
    if status != "INSUFFICIENT_SAVE_VARIATION" and len(save_hist) < 2:
        errors.append(f"{field} status {status} requires save variation")
    accepted = row.get("accepted_offsets", [])
    if status == "VALIDATED_INLINE_ARMY_SCALAR":
        if not isinstance(accepted, list) or len(accepted) != 1:
            errors.append(f"{field} validated status requires one accepted offset")
        else:
            errors.extend(validate_offset_row(field, accepted[0]))
    elif status == "AMBIGUOUS_INLINE_ARMY_SCALAR":
        if not allow_ambiguous:
            errors.append(f"{field} is ambiguous")
        if not isinstance(accepted, list) or len(accepted) < 2:
            errors.append(f"{field} ambiguous status requires multiple offsets")
    elif status == "NARROWED_INLINE_ARMY_SCALAR_NEAR_MATCH":
        if not allow_near_match:
            errors.append(f"{field} is near-match only")
        narrowed = row.get("narrowed_offsets", [])
        if not isinstance(narrowed, list) or not narrowed:
            errors.append(f"{field} near-match status requires narrowed offsets")
        else:
            first = narrowed[0]
            if float(first.get("match_fraction", 0.0)) < 0.95:
                errors.append(f"{field} near-match offset is below threshold")
            if int(first.get("candidate_distinct_values", 0)) < 2:
                errors.append(f"{field} near-match offset lacks value variation")
    elif status in {"REJECTED_INLINE_ARMY_SCALAR_RANGE", "INSUFFICIENT_SAVE_VARIATION"}:
        if accepted:
            errors.append(f"{field} rejected/insufficient status carries accepted offsets")
    else:
        errors.append(f"{field} has unexpected status {status!r}")
    return errors


def validate_offset_row(field: str, row: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    if not isinstance(row.get("offset"), str) or not row["offset"].startswith("+0x"):
        errors.append(f"{field} accepted offset missing offset")
    if row.get("width") not in {"u8", "u32"}:
        errors.append(f"{field} accepted offset has bad width")
    if int(row.get("tested", 0)) <= 0 or int(row.get("matched", 0)) != int(row.get("tested", -1)):
        errors.append(f"{field} accepted offset is not exact")
    if int(row.get("candidate_distinct_values", 0)) < 2:
        errors.append(f"{field} accepted offset lacks value variation")
    return errors


if __name__ == "__main__":
    raise SystemExit(main())
