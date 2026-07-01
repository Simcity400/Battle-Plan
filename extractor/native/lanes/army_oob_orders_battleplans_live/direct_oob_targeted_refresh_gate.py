"""Gate the direct-OOB targeted refresh-on-mismatch proof."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


EXPECTED_SCHEMA = "army_oob_orders_battleplans_direct_oob_targeted_refresh_v1"
EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("proof_json", type=Path)
    ap.add_argument("--min-iterations", type=int, default=2)
    ap.add_argument("--min-empty-direct-armies", type=int, default=1)
    ap.add_argument("--min-theatres", type=int, default=1)
    ap.add_argument("--max-bounded-scan-mb", type=int, default=512)
    ap.add_argument(
        "--allow-refresh-rejection",
        action="store_true",
        help="accept exact evidence rejecting targeted handle refresh as a bounded hot-loop path",
    )
    args = ap.parse_args()

    proof = json.loads(args.proof_json.read_text(encoding="utf-8"))
    errors = validate(
        proof,
        min_iterations=args.min_iterations,
        min_empty_direct_armies=args.min_empty_direct_armies,
        min_theatres=args.min_theatres,
        max_bounded_scan_bytes=max(1, args.max_bounded_scan_mb) << 20,
        allow_refresh_rejection=args.allow_refresh_rejection,
    )
    result = {
        "status": "PASS" if not errors else "FAIL",
        "candidate_status": proof.get("candidate_status") if not errors else "FAIL",
        "errors": errors,
        "exe_sha256": proof.get("exe_sha256"),
        "preflight_counts": proof.get("preflight", {}).get("summary", {}).get("counts", {}),
        "totals": proof.get("totals", {}),
        "rejected_candidate_family": proof.get("rejected_candidate_family"),
        "production_position": proof.get("production_position"),
    }
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0 if not errors else 1


def validate(
    proof: dict[str, Any],
    *,
    min_iterations: int,
    min_empty_direct_armies: int,
    min_theatres: int,
    max_bounded_scan_bytes: int,
    allow_refresh_rejection: bool,
) -> list[str]:
    errors: list[str] = []
    if proof.get("schema") != EXPECTED_SCHEMA:
        errors.append("unexpected schema")
    if str(proof.get("exe_sha256", "")).lower() != EXPECTED_SHA256:
        errors.append("unexpected executable sha256")
    preflight = proof.get("preflight")
    if not isinstance(preflight, dict):
        return errors + ["missing preflight object"]
    counts = preflight.get("summary", {}).get("counts", {})
    if counts.get("direct_armies", 0) <= 0:
        errors.append("preflight proved no direct armies")
    if counts.get("empty_or_inactive_direct_armies", 0) < min_empty_direct_armies:
        errors.append("not enough empty/inactive direct armies")
    if counts.get("theatres", 0) < min_theatres:
        errors.append("not enough theatres")
    warm = proof.get("warm_iterations")
    if not isinstance(warm, list) or len(warm) < min_iterations:
        errors.append("not enough warm iterations")
        warm = []
    totals = proof.get("totals", {})
    if totals.get("warm_heap_sweeps") != 0:
        errors.append("warm phase performed full vtable heap sweeps")
    if int(totals.get("warm_invalid_cached_roots", 0)) <= 0:
        errors.append("refresh-on-mismatch was not exercised")
    if int(totals.get("targeted_refresh_scans", 0)) <= 0:
        errors.append("no targeted refresh scans were recorded")

    candidate_status = proof.get("candidate_status")
    if candidate_status == "VALIDATED_BOUNDED_TARGETED_HANDLE_REFRESH":
        if counts.get("direct_armies", 0) <= counts.get("active_armies_from_division_parents", 0):
            errors.append("direct army roots did not exceed active-army roots")
        errors.extend(validate_strict_refresh(warm, counts, max_bounded_scan_bytes))
        if proof.get("rejected_candidate_family") is not None:
            errors.append("validated refresh proof must not carry a rejection object")
    elif candidate_status == "REJECTED_TARGETED_HANDLE_REFRESH_ON_MISMATCH":
        if not allow_refresh_rejection:
            errors.append("targeted refresh rejection requires --allow-refresh-rejection")
        errors.extend(validate_refresh_rejection(proof, warm, max_bounded_scan_bytes))
    else:
        errors.append("unexpected candidate status")
    return errors


def validate_strict_refresh(
    warm: list[dict[str, Any]],
    preflight_counts: dict[str, Any],
    max_bounded_scan_bytes: int,
) -> list[str]:
    errors: list[str] = []
    for row in warm:
        refresh = row.get("refresh", {})
        if row.get("validation_mode") != "cached_address_then_targeted_handle_refresh":
            errors.append(f"iteration {row.get('iteration')} has unexpected validation mode")
        if row.get("targeted_handle_refresh") is not True:
            errors.append(f"iteration {row.get('iteration')} did not mark targeted refresh")
        if refresh.get("target_count", 0) > 0:
            if refresh.get("bytes_scanned", 0) > max_bounded_scan_bytes:
                errors.append(f"iteration {row.get('iteration')} refresh scan exceeded bounded threshold")
            if refresh.get("unresolved_count", 0) != 0:
                errors.append(f"iteration {row.get('iteration')} left unresolved stale roots")
        counts = row.get("summary", {}).get("counts", {})
        for key in ("direct_armies", "army_groups", "theatres"):
            if counts.get(key) != preflight_counts.get(key):
                errors.append(f"iteration {row.get('iteration')} {key} count drifted")
        if counts.get("empty_or_inactive_direct_armies", 0) <= 0:
            errors.append(f"iteration {row.get('iteration')} lost empty/inactive direct armies")
    return errors


def validate_refresh_rejection(
    proof: dict[str, Any],
    warm: list[dict[str, Any]],
    max_bounded_scan_bytes: int,
) -> list[str]:
    errors: list[str] = []
    rejection = proof.get("rejected_candidate_family")
    if not isinstance(rejection, dict):
        errors.append("missing rejected candidate family")
        rejection = {}
    if rejection.get("candidate_family") != "cached_direct_roots_plus_targeted_handle_scan_refresh_on_mismatch":
        errors.append("unexpected rejected candidate family")
    reasons = rejection.get("reasons")
    if not isinstance(reasons, list) or not reasons:
        errors.append("refresh rejection has no reasons")
    saw_refresh_evidence = False
    saw_unresolved = False
    saw_unbounded = False
    for row in warm:
        refresh = row.get("refresh", {})
        if row.get("invalid_cached_root_count", 0) > 0:
            saw_refresh_evidence = True
        if refresh.get("unresolved_count", 0) > 0:
            saw_unresolved = True
        if refresh.get("target_count", 0) > 0 and refresh.get("bytes_scanned", 0) > max_bounded_scan_bytes:
            saw_unbounded = True
        if refresh.get("membership_false_positive_hits", 0) != 0:
            errors.append(f"iteration {row.get('iteration')} had membership false positives")
        if refresh.get("target_count", 0) > 0 and refresh.get("bytes_scanned", 0) <= 0:
            errors.append(f"iteration {row.get('iteration')} refresh scanned no bytes")
    if not saw_refresh_evidence:
        errors.append("no stale-root evidence")
    if not saw_unresolved and not saw_unbounded:
        errors.append("rejection did not prove unresolved roots or unbounded scan cost")
    return errors


if __name__ == "__main__":
    raise SystemExit(main())
