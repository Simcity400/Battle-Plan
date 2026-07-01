#!/usr/bin/env python3
"""Agency capacity-field disambiguation probe for domain 53.

The older agency-scalar packet carried the `+0xE4/+0xF0/+0xF4` cluster as
`max_operative_count`, `usable_operative_slots`, and mirror while the fields were
equal in the fixture. The current live 870ebf frame has at least one divergent
row, so this probe re-reads all agency rows, warm-validates the cluster, and
rejects candidate assignments that violate capacity invariants.

It is read-only and lane-local. It does not touch shared collector/recorder
entrypoints and does not claim full agency-state completion.
"""

from __future__ import annotations

import argparse
import json
import sys
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


LANE_DIR = Path(__file__).resolve().parent
if str(LANE_DIR) not in sys.path:
    sys.path.insert(0, str(LANE_DIR))

import probe_870ebf_intel as intel  # type: ignore


def load_owner_counts(path: Path | None) -> dict[str, int]:
    if path is None:
        return {}
    try:
        with path.open("r", encoding="utf-8") as f:
            proof = json.load(f)
    except OSError:
        return {}
    counts = proof.get("operatives", {}).get("owner_counts", {})
    if not isinstance(counts, dict):
        return {}
    out: dict[str, int] = {}
    for tag, value in counts.items():
        if isinstance(tag, str) and isinstance(value, int):
            out[tag] = value
    return out


def read_capacity_rows(p: Any) -> tuple[dict[str, Any], dict[int, dict[str, Any]]]:
    roots = intel.read_country_roots(p)
    tags = intel.load_tags(p, roots)
    countries, country_report = intel.validate_country_objects(p, roots, tags)
    rtti = intel.derive_rtti_set(p)
    agencies, agency_report = intel.read_agencies(p, countries, tags, rtti)
    return {
        "roots": roots,
        "tags": {
            "tag_count_read": len(tags),
            "samples": [
                {"tag_id": tag_id, "tag": tag}
                for tag_id, tag in list(sorted(tags.items()))[:16]
            ],
        },
        "countries": country_report,
        "agencies": agency_report,
    }, agencies


def extract_cluster(row: dict[str, Any]) -> dict[str, Any]:
    return {
        "tag": row.get("tag"),
        "tag_id": row.get("tag_id"),
        "agency_ptr": row.get("agency_ptr"),
        "name": row.get("name"),
        "related_count_0xE0": row.get("related_count_0xE0"),
        "value_0xE4": row.get("max_operative_count"),
        "value_0xF0": row.get("usable_operative_slots"),
        "value_0xF4": row.get("usable_operative_slots_mirror"),
    }


def cluster_key(row: dict[str, Any]) -> tuple[Any, Any, Any]:
    return (
        row.get("max_operative_count"),
        row.get("usable_operative_slots"),
        row.get("usable_operative_slots_mirror"),
    )


def warm_validate(p: Any, agencies: dict[int, dict[str, Any]], delay_seconds: float) -> dict[str, Any]:
    time.sleep(max(0.0, delay_seconds))
    checked = 0
    valid = 0
    invalid: list[dict[str, Any]] = []
    for _tag_id, row in sorted(agencies.items()):
        agency_ptr = row.get("agency_ptr")
        if not isinstance(agency_ptr, str):
            continue
        try:
            ag = int(agency_ptr, 16)
        except ValueError:
            continue
        checked += 1
        reread = {
            "max_operative_count": intel.i32(p, ag + intel.AG_MAX_OPERATIVES),
            "usable_operative_slots": intel.i32(p, ag + intel.AG_USABLE_SLOTS),
            "usable_operative_slots_mirror": intel.i32(
                p, ag + intel.AG_USABLE_SLOTS_MIRROR
            ),
        }
        if cluster_key(row) == cluster_key(reread):
            valid += 1
        else:
            changed = extract_cluster(row)
            changed["reread"] = extract_cluster(
                {
                    **row,
                    "max_operative_count": reread["max_operative_count"],
                    "usable_operative_slots": reread["usable_operative_slots"],
                    "usable_operative_slots_mirror": reread[
                        "usable_operative_slots_mirror"
                    ],
                }
            )
            invalid.append(changed)
    return {
        "delay_seconds": delay_seconds,
        "checked_rows": checked,
        "valid_rows": valid,
        "invalid_rows": len(invalid),
        "invalid_samples": invalid[:12],
        "status": "validated" if checked > 0 and valid == checked else "changed",
    }


def analyze(
    agencies: dict[int, dict[str, Any]], owner_counts: dict[str, int]
) -> dict[str, Any]:
    rows = [extract_cluster(row) for _tag_id, row in sorted(agencies.items())]
    divergent = [
        row
        for row in rows
        if len({row["value_0xE4"], row["value_0xF0"], row["value_0xF4"]}) > 1
    ]
    e4_f4_equal = [
        row for row in rows if row["value_0xE4"] == row["value_0xF4"]
    ]
    legacy_violations = [
        row
        for row in rows
        if isinstance(row["value_0xE4"], int)
        and isinstance(row["value_0xF0"], int)
        and row["value_0xF0"] > row["value_0xE4"]
    ]
    candidate_f0_max_violations = [
        row
        for row in rows
        if isinstance(row["value_0xE4"], int)
        and isinstance(row["value_0xF0"], int)
        and row["value_0xE4"] > row["value_0xF0"]
    ]

    for row in divergent:
        tag = row.get("tag")
        if isinstance(tag, str) and tag in owner_counts:
            row["active_operatives_from_intel_proof"] = owner_counts[tag]

    return {
        "row_count": len(rows),
        "divergent_capacity_rows": len(divergent),
        "divergent_samples": divergent[:24],
        "e4_f4_equal_rows": len(e4_f4_equal),
        "e4_f4_mismatch_rows": len(rows) - len(e4_f4_equal),
        "legacy_assignment": {
            "candidate": "max_operative_count=agency+0xE4, usable_operative_slots=agency+0xF0",
            "status": "rejected" if legacy_violations else "not_rejected_by_this_frame",
            "reason": (
                "candidate emits usable_operative_slots greater than max_operative_count "
                "on current stable rows"
                if legacy_violations
                else "no monotonic-capacity violation observed"
            ),
            "violation_count": len(legacy_violations),
            "violation_samples": legacy_violations[:24],
        },
        "candidate_f0_max_e4_f4_usable": {
            "candidate": "max_operative_count=agency+0xF0, usable/current slots=agency+0xE4 mirrored at +0xF4",
            "status": (
                "candidate_supported_not_full_validation"
                if candidate_f0_max_violations == [] and len(e4_f4_equal) == len(rows)
                else "candidate_has_violations"
            ),
            "support": {
                "e4_f4_equal_rows": len(e4_f4_equal),
                "e4_f4_mismatch_rows": len(rows) - len(e4_f4_equal),
                "e4_greater_than_f0_rows": len(candidate_f0_max_violations),
            },
            "remaining_blocker": (
                "needs same-tick UI/save agency fixture to name +0xF0 as max "
                "and +0xE4/+0xF4 as usable/current slots with semantic authority"
            ),
        },
    }


def build_report(args: argparse.Namespace) -> tuple[dict[str, Any], int]:
    owner_counts = load_owner_counts(Path(args.intel_proof) if args.intel_proof else None)
    p = intel.winmem.open_process("hoi4.exe")
    if p is None:
        raise RuntimeError("hoi4.exe not running")
    with p:
        started = time.time()
        process = intel.process_info(p)
        report: dict[str, Any] = {
            "schema": "battle-plan.diplomacy_intel_operatives.agency_capacity_probe.v1",
            "lane": "diplomacy_intel_operatives_live",
            "domain_id": 53,
            "probe": "probe_870ebf_agency_capacity",
            "generated_utc": datetime.now(timezone.utc).isoformat(timespec="seconds"),
            "process": process,
            "expected_sha256": intel.EXPECTED_SHA256,
            "build_gate_ok": process.get("sha256") == intel.EXPECTED_SHA256,
        }
        if not report["build_gate_ok"]:
            report["status"] = "build_gate_failed"
            return report, 2

        root_report, agencies = read_capacity_rows(p)
        report.update(root_report)
        report["capacity_analysis"] = analyze(agencies, owner_counts)
        report["warm_validation"] = warm_validate(
            p, agencies, max(0, args.warm_delay_ms) / 1000.0
        )
        legacy = report["capacity_analysis"]["legacy_assignment"]
        warm = report["warm_validation"]
        report["status"] = (
            "legacy_assignment_rejected"
            if legacy["status"] == "rejected" and warm["status"] == "validated"
            else "capacity_assignment_unresolved"
        )
        report["elapsed_seconds"] = round(time.time() - started, 3)
        return report, 0 if report["status"] == "legacy_assignment_rejected" else 2


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--out", required=True)
    ap.add_argument("--intel-proof", help="optional current intel proof for active-owner counts")
    ap.add_argument("--warm-delay-ms", type=int, default=250)
    args = ap.parse_args(argv)

    report, code = build_report(args)
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    with out.open("w", encoding="utf-8") as f:
        json.dump(report, f, indent=2, sort_keys=True)
        f.write("\n")
    print(
        "probe_870ebf_agency_capacity: "
        f"status={report['status']} out={out}"
    )
    return code


if __name__ == "__main__":
    raise SystemExit(main())
