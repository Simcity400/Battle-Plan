#!/usr/bin/env python3
"""Assemble an A26 same-Present matrix from owner-harness case receipts.

The in-process harness owns `collector::A26TooltipCaptureSession`; this script
never attaches to the game or manufactures a positive row. Each input must be a raw
JSON receipt emitted after one selected-Present seal. The independent verifier
is invoked only by the human/central evidence workflow, not by this assembler.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from datetime import datetime, timezone
from pathlib import Path

SCHEMA = "a26-same-present-runtime-receipt-v1"
REQUIRED_CASES = {
    "hidden_baseline_1920x1080",
    "capture_start_visible_1920x1080",
    "show_1920x1080",
    "hide_1920x1080",
    "destroyed_1920x1080",
    "owner_change_1920x1080",
    "multiline_dynamic_1920x1080",
    "edge_geometry_1920x1080",
    "edge_geometry_2560x1440",
}


def canonical_sha256(value: object) -> str:
    payload = json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
    return hashlib.sha256(payload.encode("utf-8")).hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--case-receipt", action="append", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()

    cases = []
    paths = []
    for path in args.case_receipt:
        raw = path.read_bytes()
        case = json.loads(raw)
        if not isinstance(case, dict):
            raise SystemExit(f"{path}: case receipt must be an object")
        case["case_receipt_file_sha256"] = hashlib.sha256(raw).hexdigest()
        cases.append(case)
        paths.append(str(path.resolve()))

    seen = {case.get("case_id") for case in cases}
    missing = sorted(REQUIRED_CASES - seen)
    extra = sorted(value for value in seen - REQUIRED_CASES if isinstance(value, str))
    if missing or extra or len(cases) != len(REQUIRED_CASES):
        raise SystemExit(
            f"matrix case set rejected; missing={missing}, extra={extra}, count={len(cases)}"
        )

    receipt = {
        "schema": SCHEMA,
        "assembled_utc": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
        "source_case_receipts": paths,
        "cases": sorted(cases, key=lambda row: row["case_id"]),
    }
    receipt["matrix_sha256"] = canonical_sha256(receipt["cases"])
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(receipt, indent=2, sort_keys=True, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    print(args.output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
