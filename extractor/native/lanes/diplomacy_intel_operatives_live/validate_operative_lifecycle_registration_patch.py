#!/usr/bin/env python3
"""Verify retirement of the disproven domain-53 operative-lifecycle patch.

Exact-a729 producer RE proved that ``identity+0x110`` is the capacity word of
the MSVC portrait/icon string at ``identity+0xf8``.  Historical v20 receipts
remain readable compatibility evidence, but they must never promote that
allocator capacity as a gameplay lifecycle value.
"""

from __future__ import annotations

import argparse
import json
import os
import tempfile
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


def write_json_atomic(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(
        prefix=f".{path.name}.", suffix=".tmp", dir=path.parent
    )
    temporary_path = Path(temporary)
    try:
        with os.fdopen(fd, "w", encoding="utf-8", newline="\n") as stream:
            json.dump(value, stream, indent=2, sort_keys=True)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary_path, path)
    finally:
        temporary_path.unlink(missing_ok=True)


def proof_unit_count(proof: dict[str, Any], name: str) -> int:
    units = proof.get("data_units_total")
    if not isinstance(units, dict):
        return 0
    return int(units.get(name) or 0)


def validate(args: argparse.Namespace) -> dict[str, Any]:
    errors: list[str] = []
    # Retain every historical CLI input so existing aggregate invocations do
    # not break, but only the current production/schema sources and old receipt
    # are semantically relevant to this retirement gate.
    for name in (
        "collector_registry",
        "recorder_registry",
        "registration_packet",
        "lifecycle_patch",
        "lane_lib",
    ):
        read_text(Path(getattr(args, name)))

    compact_schema = read_text(Path(args.compact_schema))
    extractor = read_text(Path(args.extractor))
    proof = load_json(Path(args.replay_proof))

    require(
        "pub lifecycle_flag: Option<u32>" in compact_schema,
        errors,
        "backward decoder no longer carries the optional legacy field",
    )
    require(
        "OPERATIVE_FLAG_LIFECYCLE_PRESENT" in compact_schema,
        errors,
        "backward decoder no longer recognizes legacy v20 payloads",
    )
    require(
        "DOMAIN53_OPERATIVE_LIFECYCLE_FLAG" not in extractor,
        errors,
        "production extractor still references the disproven lifecycle offset",
    )
    require(
        "read_u32(identity + 0x110)" not in extractor,
        errors,
        "production extractor still reads portrait-string capacity as lifecycle",
    )
    require(
        "lifecycle_flag: None" in extractor,
        errors,
        "current operative rows are not explicitly lifecycle-free",
    )
    require(
        'stable_hash32(&format!("assignment_{assignment}"))' in extractor,
        errors,
        "current operative rows do not use the exact assignment discriminator",
    )
    require(
        "operative_11387_lifecycle" not in extractor
        and "lifeflag_47" not in extractor
        and "Some(47)" not in extractor
        and "Some(31)" not in extractor
        and "Some(63)" not in extractor,
        errors,
        "current Rust fixtures still fabricate allocator capacities as lifecycle",
    )
    require(
        int(proof.get("domain_id") or 0) == DOMAIN_ID,
        errors,
        "legacy receipt domain_id is not 53",
    )

    legacy_count = proof_unit_count(proof, "operative_lifecycle_rows")
    status = (
        "operative_lifecycle_false_semantic_retirement_verified"
        if not errors
        else "failed"
    )
    return {
        "schema": "battle-plan.diplomacy_intel_operatives.operative_lifecycle_retirement.v2",
        "generated_utc": datetime.now(timezone.utc).isoformat(timespec="seconds"),
        "lane": LANE,
        "domain_id": DOMAIN_ID,
        "completion_status": "RETIRED" if not errors else "NOT COMPLETE",
        "status": status,
        "backlog_reduced": "yes" if not errors else "no",
        "exact_backlog_item_removed_or_narrowed": (
            "operative lifecycle at identity+0x110 retired as false semantic; "
            "legacy payload decode remains compatibility-only"
            if not errors
            else "none"
        ),
        "retirement": {
            "disproven_offset": "identity+0x110",
            "actual_role": "capacity word for MSVC string at identity+0xf8",
            "replacement_current_semantic": "numeric assignment discriminator at leader+0x1080",
            "legacy_receipt_operative_lifecycle_rows": legacy_count,
            "legacy_receipt_promotable": False,
        },
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
    write_json_atomic(out, report)
    print(
        "operative_lifecycle_retirement: "
        f"status={report['status']} "
        f"legacy_counter={report['retirement']['legacy_receipt_operative_lifecycle_rows']} "
        f"out={out}"
    )
    return (
        0
        if report["status"]
        == "operative_lifecycle_false_semantic_retirement_verified"
        else 2
    )


if __name__ == "__main__":
    raise SystemExit(main())
