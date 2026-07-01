"""Gate a lane-local direct land-OOB root proof.

This validates the read-only direct_oob_probe.py output. It intentionally does
not promote the slow arena scan as a hot-loop production source; it only proves
that the current-build roots exist and are worth moving into a bounded DLL
preflight/cache-refresh path.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("proof_json", type=Path)
    ap.add_argument("--min-empty-direct-armies", type=int, default=1)
    ap.add_argument("--min-theatres", type=int, default=1)
    ap.add_argument("--min-non-type5-orders", type=int, default=1)
    args = ap.parse_args()

    proof = json.loads(args.proof_json.read_text(encoding="utf-8"))
    errors = validate(
        proof,
        min_empty_direct_armies=args.min_empty_direct_armies,
        min_theatres=args.min_theatres,
        min_non_type5_orders=args.min_non_type5_orders,
    )
    summary = proof.get("summary", {})
    counts = summary.get("counts", {})
    scan_stats = proof.get("scan_stats", {})
    result: dict[str, Any] = {
        "status": "PASS" if not errors else "FAIL",
        "errors": errors,
        "exe_sha256": proof.get("exe_sha256"),
        "counts": counts,
        "accepted_hits": scan_stats.get("accepted_hits", {}),
        "rejected_hits": scan_stats.get("rejected_hits", {}),
        "order_type_histogram": summary.get("order_type_histogram", {}),
        "production_position": proof.get("production_position"),
    }
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0 if not errors else 1


def validate(
    proof: dict[str, Any],
    *,
    min_empty_direct_armies: int,
    min_theatres: int,
    min_non_type5_orders: int,
) -> list[str]:
    errors: list[str] = []
    if proof.get("schema") != "army_oob_orders_battleplans_direct_oob_probe_v1":
        errors.append("unexpected schema")
    if str(proof.get("exe_sha256", "")).lower() != EXPECTED_SHA256:
        errors.append("unexpected executable sha256")
    summary = proof.get("summary")
    if not isinstance(summary, dict):
        return errors + ["missing summary"]
    counts = summary.get("counts", {})
    scan_stats = proof.get("scan_stats", {})
    accepted = scan_stats.get("accepted_hits", {})
    rejected = scan_stats.get("rejected_hits", {})
    for key in ("army", "army_group", "theatre", "division"):
        if accepted.get(key, 0) <= 0:
            errors.append(f"no accepted {key} roots")
        if rejected.get(key, 0) != 0:
            errors.append(f"{key} root scan had rejected hits")
    if counts.get("direct_armies", 0) <= counts.get("active_armies_from_division_parents", 0):
        errors.append("direct army roots did not exceed division-parent active-army roots")
    if counts.get("empty_or_inactive_direct_armies", 0) < min_empty_direct_armies:
        errors.append("not enough empty/inactive direct armies proved")
    if counts.get("theatres", 0) < min_theatres:
        errors.append("not enough theatre roots proved")
    if counts.get("non_type5_order_rows", 0) < min_non_type5_orders:
        errors.append("no non-type-5 order rows observed for fixture targeting")
    if counts.get("type5_validation_failed_rows", 0) != 0:
        errors.append("type-5 target-state validation failures present")
    return errors


if __name__ == "__main__":
    raise SystemExit(main())
