"""Gate direct-OOB root pointer table probe evidence."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


EXPECTED_SCHEMA = "army_oob_orders_battleplans_direct_oob_table_probe_v1"
EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
DIRECT_KINDS = ("army", "army_group", "theatre")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("proof_json", type=Path)
    ap.add_argument("--min-empty-direct-armies", type=int, default=1)
    ap.add_argument("--min-theatres", type=int, default=1)
    ap.add_argument("--min-reference-hits", type=int, default=1)
    ap.add_argument("--min-kind-coverage", type=float, default=0.50)
    ap.add_argument("--min-total-direct-coverage", type=float, default=0.50)
    ap.add_argument(
        "--require-empty-army-table-resolution",
        action="store_true",
        help="fail unless every empty/inactive army is covered by an accepted table run",
    )
    args = ap.parse_args()

    proof = json.loads(args.proof_json.read_text(encoding="utf-8"))
    errors = validate(
        proof,
        min_empty_direct_armies=args.min_empty_direct_armies,
        min_theatres=args.min_theatres,
        min_reference_hits=args.min_reference_hits,
        min_kind_coverage=args.min_kind_coverage,
        min_total_direct_coverage=args.min_total_direct_coverage,
        require_empty_army_table_resolution=args.require_empty_army_table_resolution,
    )
    result = {
        "status": "PASS" if not errors else "FAIL",
        "candidate_status": proof.get("candidate_status") if not errors else "FAIL",
        "errors": errors,
        "exe_sha256": proof.get("exe_sha256"),
        "preflight_counts": proof.get("preflight", {}).get("summary", {}).get("counts", {}),
        "reference_hits": proof.get("table_scan", {}).get("total_reference_hits"),
        "hit_cap_exceeded": proof.get("table_scan", {}).get("hit_cap_exceeded"),
        "max_coverage": proof.get("table_scan", {}).get("max_coverage"),
        "run_union_coverage_by_kind": proof.get("table_scan", {}).get("run_union_coverage_by_kind"),
        "special_root_coverage": proof.get("table_scan", {}).get("special_root_coverage"),
        "accepted_table_candidates": proof.get("table_scan", {}).get("accepted_table_candidates", [])[:3],
        "production_position": proof.get("production_position"),
    }
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0 if not errors else 1


def validate(
    proof: dict[str, Any],
    *,
    min_empty_direct_armies: int,
    min_theatres: int,
    min_reference_hits: int,
    min_kind_coverage: float,
    min_total_direct_coverage: float,
    require_empty_army_table_resolution: bool,
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
    if counts.get("direct_armies", 0) <= counts.get("active_armies_from_division_parents", 0):
        errors.append("direct army roots did not exceed active-army roots")
    if counts.get("empty_or_inactive_direct_armies", 0) < min_empty_direct_armies:
        errors.append("not enough empty/inactive direct armies")
    if counts.get("army_groups", 0) <= 0:
        errors.append("no army groups proved")
    if counts.get("theatres", 0) < min_theatres:
        errors.append("not enough theatres")

    scan_stats = preflight.get("scan_stats", {})
    accepted = scan_stats.get("accepted_hits", {})
    for key in (*DIRECT_KINDS, "division"):
        if accepted.get(key, 0) <= 0:
            errors.append(f"preflight accepted no {key} roots")

    table_scan = proof.get("table_scan")
    if not isinstance(table_scan, dict):
        return errors + ["missing table_scan object"]
    if table_scan.get("hit_cap_exceeded"):
        errors.append("reference hit cap exceeded")
    if table_scan.get("membership_false_positive_hits", 0) != 0:
        errors.append("reference scan had membership false positives")
    if table_scan.get("bytes_scanned", 0) <= 0:
        errors.append("reference scan read no bytes")
    if table_scan.get("total_reference_hits", 0) < min_reference_hits:
        errors.append("not enough direct-root reference hits")
    if table_scan.get("search_strides") != [8, 16, 24, 32]:
        errors.append("unexpected search strides")
    root_counts = table_scan.get("target_root_counts", {})
    for key in DIRECT_KINDS:
        if root_counts.get(key, 0) <= 0:
            errors.append(f"missing {key} target roots")
    errors.extend(validate_special_coverage(table_scan, require_empty_army_table_resolution))

    candidate_status = proof.get("candidate_status")
    if candidate_status in {
        "VALIDATED_DIRECT_ROOT_POINTER_TABLE",
        "VALIDATED_PARTIAL_DIRECT_ROOT_POINTER_TABLE",
    }:
        errors.extend(
            validate_accepted_candidate(
                table_scan,
                partial=candidate_status == "VALIDATED_PARTIAL_DIRECT_ROOT_POINTER_TABLE",
                min_kind_coverage=min_kind_coverage,
                min_total_direct_coverage=min_total_direct_coverage,
            )
        )
    elif candidate_status == "REJECTED_PLAIN_DIRECT_ROOT_POINTER_TABLE":
        errors.extend(
            validate_rejection(
                proof,
                table_scan,
                min_kind_coverage=min_kind_coverage,
                min_total_direct_coverage=min_total_direct_coverage,
            )
        )
    else:
        errors.append("unexpected candidate status")
    return errors


def validate_special_coverage(
    table_scan: dict[str, Any],
    require_empty_army_table_resolution: bool,
) -> list[str]:
    errors: list[str] = []
    special = table_scan.get("special_root_coverage")
    if not isinstance(special, dict):
        return ["missing special root coverage"]
    empty = special.get("empty_or_inactive_armies")
    if not isinstance(empty, dict):
        errors.append("missing empty/inactive army coverage")
    else:
        total = int(empty.get("total", 0))
        if total <= 0:
            errors.append("no empty/inactive armies proved")
        if int(empty.get("referenced_anywhere", 0)) > total:
            errors.append("empty/inactive army reference count exceeds total")
        if int(empty.get("covered_by_any_table_run", 0)) > total:
            errors.append("empty/inactive army table-run count exceeds total")
        if int(empty.get("covered_by_accepted_table_run", 0)) > total:
            errors.append("empty/inactive army accepted-run count exceeds total")
        if (
            require_empty_army_table_resolution
            and int(empty.get("covered_by_accepted_table_run", 0)) != total
        ):
            errors.append("empty/inactive armies are not fully covered by accepted table runs")
    return errors


def validate_accepted_candidate(
    table_scan: dict[str, Any],
    *,
    partial: bool,
    min_kind_coverage: float,
    min_total_direct_coverage: float,
) -> list[str]:
    candidates = table_scan.get("accepted_table_candidates")
    if not isinstance(candidates, list) or not candidates:
        return ["validated table status without accepted candidates"]
    for candidate in candidates:
        if candidate.get("stride") not in {8, 16, 24, 32}:
            return ["accepted candidate has unsupported stride"]
        if candidate.get("total_direct_coverage", 0.0) >= min_total_direct_coverage:
            if partial:
                return ["partial table status used for total direct coverage candidate"]
            return []
        coverage = candidate.get("coverage_by_kind", {})
        if any(float(coverage.get(kind, 0.0)) >= min_kind_coverage for kind in DIRECT_KINDS):
            return []
    return ["accepted candidates do not meet coverage thresholds"]


def validate_rejection(
    proof: dict[str, Any],
    table_scan: dict[str, Any],
    *,
    min_kind_coverage: float,
    min_total_direct_coverage: float,
) -> list[str]:
    errors: list[str] = []
    rejection = proof.get("rejected_candidate_family")
    if not isinstance(rejection, dict):
        return ["missing rejected candidate family"]
    if rejection.get("candidate_family") != "plain_contiguous_or_fixed_stride_direct_root_pointer_table":
        errors.append("unexpected rejected candidate family")
    if table_scan.get("accepted_table_candidates"):
        errors.append("rejection proof contains accepted table candidates")
    max_cov = table_scan.get("max_coverage", {})
    if float(max_cov.get("total_direct_coverage", 0.0)) >= min_total_direct_coverage:
        errors.append("total direct coverage meets table threshold")
    by_kind = max_cov.get("by_kind", {})
    for kind in DIRECT_KINDS:
        if float(by_kind.get(kind, 0.0)) >= min_kind_coverage:
            errors.append(f"{kind} coverage meets table threshold")
    thresholds = table_scan.get("thresholds", {})
    if float(thresholds.get("min_kind_coverage", 0.0)) != min_kind_coverage:
        errors.append("proof used unexpected kind coverage threshold")
    if float(thresholds.get("min_total_direct_coverage", 0.0)) != min_total_direct_coverage:
        errors.append("proof used unexpected total coverage threshold")
    return errors


if __name__ == "__main__":
    raise SystemExit(main())
