"""Gate the direct-OOB bounded warm-cache proof."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


EXPECTED_SCHEMA = "army_oob_orders_battleplans_direct_oob_warm_cache_v1"
EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("proof_json", type=Path)
    ap.add_argument("--min-iterations", type=int, default=3)
    ap.add_argument("--min-empty-direct-armies", type=int, default=1)
    ap.add_argument("--min-theatres", type=int, default=1)
    ap.add_argument(
        "--allow-pure-cache-rejection",
        action="store_true",
        help="accept exact stale-root evidence as rejection of a pure cached-address warm loop",
    )
    args = ap.parse_args()

    proof = json.loads(args.proof_json.read_text(encoding="utf-8"))
    if args.allow_pure_cache_rejection:
        errors = validate_pure_cache_rejection(
            proof,
            min_iterations=args.min_iterations,
            min_empty_direct_armies=args.min_empty_direct_armies,
            min_theatres=args.min_theatres,
        )
        candidate_status = "REJECTED_PURE_PREFLIGHT_CACHE" if not errors else "FAIL"
    else:
        errors = validate(
            proof,
            min_iterations=args.min_iterations,
            min_empty_direct_armies=args.min_empty_direct_armies,
            min_theatres=args.min_theatres,
        )
        candidate_status = "VALIDATED_WARM_CACHE" if not errors else "FAIL"
    result = {
        "status": "PASS" if not errors else "FAIL",
        "candidate_status": candidate_status,
        "errors": errors,
        "exe_sha256": proof.get("exe_sha256"),
        "preflight_counts": proof.get("preflight", {}).get("summary", {}).get("counts", {}),
        "warm_iterations": len(proof.get("warm_iterations", [])),
        "totals": proof.get("totals", {}),
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
) -> list[str]:
    errors: list[str] = []
    if proof.get("schema") != EXPECTED_SCHEMA:
        errors.append("unexpected schema")
    if str(proof.get("exe_sha256", "")).lower() != EXPECTED_SHA256:
        errors.append("unexpected executable sha256")
    preflight = proof.get("preflight")
    if not isinstance(preflight, dict):
        return errors + ["missing preflight object"]
    scan_stats = preflight.get("scan_stats", {})
    accepted = scan_stats.get("accepted_hits", {})
    rejected = scan_stats.get("rejected_hits", {})
    for key in ("army", "army_group", "theatre", "division"):
        if accepted.get(key, 0) <= 0:
            errors.append(f"preflight accepted no {key} roots")
        if rejected.get(key, 0) != 0:
            errors.append(f"preflight had rejected {key} roots")
    preflight_counts = preflight.get("summary", {}).get("counts", {})
    if preflight_counts.get("direct_armies", 0) <= preflight_counts.get("active_armies_from_division_parents", 0):
        errors.append("direct army roots did not exceed active-army roots")
    if preflight_counts.get("empty_or_inactive_direct_armies", 0) < min_empty_direct_armies:
        errors.append("not enough empty/inactive direct armies")
    if preflight_counts.get("theatres", 0) < min_theatres:
        errors.append("not enough theatres")
    warm = proof.get("warm_iterations")
    if not isinstance(warm, list) or len(warm) < min_iterations:
        errors.append("not enough warm iterations")
        warm = []
    totals = proof.get("totals", {})
    if totals.get("warm_heap_sweeps") != 0:
        errors.append("warm phase performed heap sweeps")
    if totals.get("warm_invalid_cached_roots") != 0:
        errors.append("warm phase had invalid cached roots")
    for item in warm:
        if item.get("warm_heap_sweep") is not False:
            errors.append(f"iteration {item.get('iteration')} warm_heap_sweep must be false")
        if item.get("validation_mode") != "cached_address_vtable_handle_id_liveness":
            errors.append(f"iteration {item.get('iteration')} has unexpected validation mode")
        if item.get("invalid_cached_root_count") != 0:
            errors.append(f"iteration {item.get('iteration')} has invalid cached roots")
        counts = item.get("summary", {}).get("counts", {})
        for key in ("direct_armies", "army_groups", "theatres"):
            if counts.get(key) != preflight_counts.get(key):
                errors.append(f"iteration {item.get('iteration')} {key} count drifted")
        if counts.get("empty_or_inactive_direct_armies", 0) < min_empty_direct_armies:
            errors.append(f"iteration {item.get('iteration')} lost empty/inactive direct armies")
        if counts.get("theatres", 0) < min_theatres:
            errors.append(f"iteration {item.get('iteration')} lost theatres")
    return errors


def validate_pure_cache_rejection(
    proof: dict[str, Any],
    *,
    min_iterations: int,
    min_empty_direct_armies: int,
    min_theatres: int,
) -> list[str]:
    errors: list[str] = []
    if proof.get("schema") != EXPECTED_SCHEMA:
        errors.append("unexpected schema")
    if str(proof.get("exe_sha256", "")).lower() != EXPECTED_SHA256:
        errors.append("unexpected executable sha256")
    preflight = proof.get("preflight")
    if not isinstance(preflight, dict):
        return errors + ["missing preflight object"]
    preflight_counts = preflight.get("summary", {}).get("counts", {})
    if preflight_counts.get("empty_or_inactive_direct_armies", 0) < min_empty_direct_armies:
        errors.append("preflight did not prove empty/inactive direct armies")
    if preflight_counts.get("theatres", 0) < min_theatres:
        errors.append("preflight did not prove theatres")
    warm = proof.get("warm_iterations")
    if not isinstance(warm, list) or len(warm) < min_iterations:
        errors.append("not enough warm iterations")
        warm = []
    totals = proof.get("totals", {})
    if totals.get("warm_heap_sweeps") != 0:
        errors.append("rejection proof must not use warm heap sweeps")
    if int(totals.get("warm_invalid_cached_roots", 0)) <= 0:
        errors.append("rejection proof has no invalid cached roots")
    allowed_reasons = {"vtable_mismatch", "object_id_mismatch", "handle_or_type_validation_failed", "unreadable_vtable"}
    saw_invalid = False
    for item in warm:
        if item.get("warm_heap_sweep") is not False:
            errors.append(f"iteration {item.get('iteration')} warm_heap_sweep must be false")
        invalid_roots = item.get("invalid_cached_roots")
        if not isinstance(invalid_roots, list) or not invalid_roots:
            continue
        saw_invalid = True
        for root in invalid_roots:
            if root.get("reason") not in allowed_reasons:
                errors.append(f"iteration {item.get('iteration')} has unsupported stale-root reason")
            if root.get("kind") not in {"army", "army_group", "theatre", "division"}:
                errors.append(f"iteration {item.get('iteration')} has bad stale-root kind")
            if not isinstance(root.get("id"), int) or root["id"] <= 0:
                errors.append(f"iteration {item.get('iteration')} stale root missing id")
            if not isinstance(root.get("addr"), str) or not root["addr"].startswith("0x"):
                errors.append(f"iteration {item.get('iteration')} stale root missing address")
    if not saw_invalid:
        errors.append("no iteration carried stale-root evidence")
    return errors


if __name__ == "__main__":
    raise SystemExit(main())
