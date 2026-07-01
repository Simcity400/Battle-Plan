"""Gate save/live battleplan geometry offset proofs."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


EXPECTED_SCHEMA = "army_oob_orders_battleplans_save_geometry_probe_v1"
EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("proof_json", type=Path)
    ap.add_argument("--min-type5-exact", type=int, default=64)
    ap.add_argument("--min-path70-exact", type=int, default=32)
    ap.add_argument("--require-type3-c0-rejected", action="store_true")
    args = ap.parse_args()
    proof = json.loads(args.proof_json.read_text(encoding="utf-8"))
    errors = validate(
        proof,
        min_type5_exact=args.min_type5_exact,
        min_path70_exact=args.min_path70_exact,
        require_type3_c0_rejected=args.require_type3_c0_rejected,
    )
    result = {
        "status": "PASS" if not errors else "FAIL",
        "candidate_status": proof.get("candidate_status") if not errors else "FAIL",
        "errors": errors,
        "positive_control": summarize(proof.get("positive_control", {})),
        "candidate_results": {
            row.get("name"): summarize(row)
            for row in proof.get("candidate_results", [])
            if isinstance(row, dict)
        },
        "production_position": proof.get("production_position"),
    }
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0 if not errors else 1


def summarize(row: dict[str, Any]) -> dict[str, Any]:
    return {
        "status": row.get("status"),
        "tested_rows": row.get("tested_rows"),
        "valid_rows": row.get("valid_rows"),
        "exact_rows": row.get("exact_rows"),
        "exact_fraction": row.get("exact_fraction"),
    }


def validate(
    proof: dict[str, Any],
    *,
    min_type5_exact: int,
    min_path70_exact: int,
    require_type3_c0_rejected: bool,
) -> list[str]:
    errors: list[str] = []
    if proof.get("schema") != EXPECTED_SCHEMA:
        errors.append("unexpected schema")
    if str(proof.get("exe_sha256", "")).lower() != EXPECTED_SHA256:
        errors.append("unexpected executable sha256")
    if proof.get("errors"):
        errors.append(f"probe reported errors: {proof.get('errors')}")
    positive = proof.get("positive_control", {})
    if positive.get("status") != "positive_control_pass":
        errors.append("type-5 positive control did not pass")
    if int(positive.get("exact_rows", 0)) < min_type5_exact:
        errors.append("not enough exact type-5 positive-control rows")
    candidates = {row.get("name"): row for row in proof.get("candidate_results", []) if isinstance(row, dict)}
    path70 = candidates.get("non_type5_path_vector_plus_0x70")
    if not isinstance(path70, dict):
        errors.append("missing +0x70 path candidate")
    else:
        if path70.get("status") != "validated_save_paired_geometry_slice":
            errors.append("+0x70 path candidate is not validated")
        if int(path70.get("exact_rows", 0)) < min_path70_exact:
            errors.append("not enough exact +0x70 path rows")
    type3_c0 = candidates.get("type3_secondary_vector_plus_0xC0")
    if require_type3_c0_rejected:
        if not isinstance(type3_c0, dict):
            errors.append("missing type3 +0xC0 candidate")
        elif type3_c0.get("status") != "rejected_save_paired_geometry_candidate":
            errors.append("type3 +0xC0 candidate is not rejected")
        elif int(type3_c0.get("exact_rows", 0)) != 0:
            errors.append("type3 +0xC0 rejection has exact rows")
    return errors


if __name__ == "__main__":
    raise SystemExit(main())
