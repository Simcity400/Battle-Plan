"""Gate live battleplan candidate probe reports.

The gate accepts a report only if the type-5 positive control passed and, when
requested, named candidates reached an exact `rejected` or `fixture_required`
state with evidence. It prevents using a failed probe as integration evidence.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


EXPECTED_SCHEMA = "army_oob_orders_battleplans_candidate_probe_v1"
EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
POSITIVE_CONTROL = "positive_control_type5_states_plus_0xE0"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("report_json", type=Path)
    ap.add_argument("--require-rejected", action="append", default=[])
    ap.add_argument("--require-fixture-required", action="append", default=[])
    args = ap.parse_args()
    report = json.loads(args.report_json.read_text(encoding="utf-8"))
    errors = validate_report(
        report,
        required_rejected=args.require_rejected,
        required_fixture_required=args.require_fixture_required,
    )
    print(
        json.dumps(
            {
                "status": "PASS" if not errors else "FAIL",
                "errors": errors,
                "candidate_statuses": candidate_statuses(report),
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0 if not errors else 1


def validate_report(
    report: dict[str, Any],
    *,
    required_rejected: list[str],
    required_fixture_required: list[str],
) -> list[str]:
    errors: list[str] = []
    if report.get("schema") != EXPECTED_SCHEMA:
        errors.append(f"schema must be {EXPECTED_SCHEMA}")
    if str(report.get("exe_sha256", "")).lower() != EXPECTED_SHA256:
        errors.append("unexpected executable sha256")
    if report.get("errors"):
        errors.append(f"probe reported errors: {report.get('errors')}")
    candidates = {row.get("name"): row for row in report.get("candidate_results", []) if isinstance(row, dict)}
    positive = candidates.get(POSITIVE_CONTROL)
    if not positive:
        errors.append("missing type-5 positive control")
    elif positive.get("status") != "positive_control_pass":
        errors.append("type-5 positive control did not pass")
    elif int(positive.get("valid_rows", 0)) <= 0 or int(positive.get("tested_rows", 0)) <= 0:
        errors.append("type-5 positive control has no tested/valid rows")
    for name in required_rejected:
        row = candidates.get(name)
        if row is None:
            errors.append(f"missing required rejected candidate {name}")
        elif row.get("status") != "rejected":
            errors.append(f"{name} status {row.get('status')!r} is not rejected")
        elif not row.get("evidence"):
            errors.append(f"{name} rejection lacks evidence")
    for name in required_fixture_required:
        row = candidates.get(name)
        if row is None:
            errors.append(f"missing required fixture-required candidate {name}")
        elif row.get("status") != "fixture_required":
            errors.append(f"{name} status {row.get('status')!r} is not fixture_required")
        elif not row.get("evidence"):
            errors.append(f"{name} fixture blocker lacks evidence")
    return errors


def candidate_statuses(report: dict[str, Any]) -> dict[str, str]:
    out = {}
    for row in report.get("candidate_results", []):
        if isinstance(row, dict) and isinstance(row.get("name"), str):
            out[row["name"]] = str(row.get("status"))
    return out


if __name__ == "__main__":
    raise SystemExit(main())
