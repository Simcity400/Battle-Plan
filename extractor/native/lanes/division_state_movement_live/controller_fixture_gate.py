#!/usr/bin/env python3
"""Gate an owner/controller-divergent division fixture for CDivision+0x1D0.

Inputs:
  1. controller_candidate_probe.py output from the same running fixture.
  2. A small oracle JSON naming the expected divergent divisions.

The gate accepts only if at least one oracle row has owner != controller and the
live candidate value read from CDivision+0x1D0 matches that controller.
"""

from __future__ import annotations

import argparse
import json
import sys
import tempfile
from pathlib import Path
from typing import Any


TEMPLATE = {
    "schema": "division_controller_fixture_oracle_v1",
    "fixture_id": "replace_with_save_or_scenario_name",
    "exe_sha256": "optional_same_as_controller_candidate_probe",
    "source": "time_aligned_save_or_in_game_fixture_note",
    "divisions": [
        {
            "div_id": 0,
            "owner_tag_db_id": 0,
            "controller_tag_db_id": 0,
            "owner_tag": "OWN",
            "controller_tag": "CTL",
            "fixture_note": "must be an expeditionary/borrowed/current-host divergent division",
        }
    ],
}


def _positive_int(value: Any) -> bool:
    return isinstance(value, int) and value > 0


def _load_json(path: Path) -> dict[str, Any]:
    data = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(data, dict):
        raise ValueError(f"{path} top-level JSON must be an object")
    return data


def gate(probe: dict[str, Any], oracle: dict[str, Any]) -> dict[str, Any]:
    reasons: list[str] = []
    units: dict[str, int] = {}
    matched: list[dict[str, Any]] = []

    if probe.get("schema") != "division_controller_candidate_probe_v1":
        reasons.append("probe schema must be division_controller_candidate_probe_v1")
    if oracle.get("schema") != "division_controller_fixture_oracle_v1":
        reasons.append("oracle schema must be division_controller_fixture_oracle_v1")

    candidate = probe.get("candidate") if isinstance(probe.get("candidate"), dict) else {}
    if candidate.get("controller_candidate_offset") != "CDivision+0x1D0":
        reasons.append("probe candidate offset must be CDivision+0x1D0")
    if candidate.get("owner_offset") != "CDivision+0x1C8":
        reasons.append("probe owner offset must be CDivision+0x1C8")

    root = probe.get("root_validation") if isinstance(probe.get("root_validation"), dict) else {}
    if root.get("daily_update_sentinel") is not True:
        reasons.append("probe daily_update_sentinel is not true")
    if root.get("get_long_string_sentinel") is not True:
        reasons.append("probe get_long_string_sentinel is not true")

    if probe.get("errors"):
        reasons.append(f"probe has errors: {probe.get('errors')!r}")

    expected_sha = oracle.get("exe_sha256")
    if expected_sha and expected_sha != probe.get("exe_sha256"):
        reasons.append(
            f"oracle exe_sha256 {expected_sha} != probe exe_sha256 {probe.get('exe_sha256')}"
        )

    probe_rows = probe.get("sample_rows")
    if not isinstance(probe_rows, list):
        reasons.append("probe sample_rows must be a list")
        probe_rows = []
    rows_by_div = {
        row.get("div_id"): row for row in probe_rows if isinstance(row, dict) and _positive_int(row.get("div_id"))
    }

    oracle_rows = oracle.get("divisions")
    if not isinstance(oracle_rows, list) or not oracle_rows:
        reasons.append("oracle divisions must be a non-empty list")
        oracle_rows = []

    divergent_oracle_rows = 0
    joined_rows = 0
    matched_divergent_rows = 0
    for idx, row in enumerate(oracle_rows):
        if not isinstance(row, dict):
            reasons.append(f"oracle row {idx} is not an object")
            continue
        div_id = row.get("div_id")
        owner = row.get("owner_tag_db_id")
        controller = row.get("controller_tag_db_id")
        if not _positive_int(div_id):
            reasons.append(f"oracle row {idx} has invalid div_id {div_id!r}")
            continue
        if not _positive_int(owner):
            reasons.append(f"oracle row {idx} has invalid owner_tag_db_id {owner!r}")
            continue
        if not _positive_int(controller):
            reasons.append(f"oracle row {idx} has invalid controller_tag_db_id {controller!r}")
            continue
        if owner == controller:
            reasons.append(f"oracle row {idx} is not owner/controller-divergent")
            continue
        divergent_oracle_rows += 1
        live = rows_by_div.get(div_id)
        if live is None:
            reasons.append(f"oracle divergent div_id {div_id} is missing from probe sample_rows")
            continue
        joined_rows += 1
        live_owner = live.get("owner_tag_db_id")
        live_candidate = live.get("controller_candidate_tag_db_id")
        if live_owner != owner:
            reasons.append(f"div_id {div_id} live owner {live_owner} != oracle owner {owner}")
            continue
        if live_candidate != controller:
            reasons.append(
                f"div_id {div_id} CDivision+0x1D0 {live_candidate} != oracle controller {controller}"
            )
            continue
        matched_divergent_rows += 1
        matched.append(
            {
                "div_id": div_id,
                "owner_tag_db_id": owner,
                "controller_tag_db_id": controller,
                "body_ptr": live.get("body_ptr"),
            }
        )

    units["oracle_rows"] = len(oracle_rows)
    units["oracle_owner_controller_divergent_rows"] = divergent_oracle_rows
    units["joined_rows"] = joined_rows
    units["matched_divergent_rows"] = matched_divergent_rows
    units["probe_validated_division_rows"] = int(
        (probe.get("counts") if isinstance(probe.get("counts"), dict) else {}).get(
            "validated_division_rows", 0
        )
        or 0
    )

    if divergent_oracle_rows == 0:
        reasons.append("oracle contains no owner/controller-divergent division")
    if matched_divergent_rows == 0:
        reasons.append("no divergent oracle row matched CDivision+0x1D0")

    accepted = not reasons
    return {
        "schema": "division_controller_fixture_gate_v1",
        "accepted": accepted,
        "status": "accepted_controller_candidate" if accepted else "rejected_or_incomplete_fixture",
        "fixture_id": oracle.get("fixture_id"),
        "candidate": {
            "owner_offset": "CDivision+0x1C8",
            "controller_candidate_offset": "CDivision+0x1D0",
        },
        "units": units,
        "matched_rows": matched,
        "reasons": reasons,
    }


def run_self_test() -> None:
    probe = {
        "schema": "division_controller_candidate_probe_v1",
        "exe_sha256": "abc",
        "candidate": {
            "owner_offset": "CDivision+0x1C8",
            "controller_candidate_offset": "CDivision+0x1D0",
        },
        "root_validation": {
            "daily_update_sentinel": True,
            "get_long_string_sentinel": True,
        },
        "counts": {"validated_division_rows": 2},
        "sample_rows": [
            {
                "div_id": 10,
                "owner_tag_db_id": 1,
                "controller_candidate_tag_db_id": 2,
                "body_ptr": "0x10",
            }
        ],
        "errors": [],
    }
    oracle = {
        "schema": "division_controller_fixture_oracle_v1",
        "fixture_id": "synthetic",
        "exe_sha256": "abc",
        "divisions": [{"div_id": 10, "owner_tag_db_id": 1, "controller_tag_db_id": 2}],
    }
    rejected_oracle = {
        "schema": "division_controller_fixture_oracle_v1",
        "fixture_id": "synthetic",
        "exe_sha256": "abc",
        "divisions": [{"div_id": 10, "owner_tag_db_id": 1, "controller_tag_db_id": 1}],
    }
    if not gate(probe, oracle)["accepted"]:
        raise AssertionError("expected divergent synthetic fixture to pass")
    if gate(probe, rejected_oracle)["accepted"]:
        raise AssertionError("expected equality-only synthetic fixture to fail")

    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp) / "template.json"
        path.write_text(json.dumps(TEMPLATE), encoding="utf-8")
        loaded = _load_json(path)
        if loaded.get("schema") != "division_controller_fixture_oracle_v1":
            raise AssertionError("template failed to round-trip")


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--probe", type=Path)
    parser.add_argument("--oracle", type=Path)
    parser.add_argument("--out", type=Path)
    parser.add_argument("--write-template", type=Path)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)

    if args.self_test:
        run_self_test()
        print("controller_fixture_gate self-test passed")
        return 0
    if args.write_template:
        args.write_template.parent.mkdir(parents=True, exist_ok=True)
        args.write_template.write_text(json.dumps(TEMPLATE, indent=2) + "\n", encoding="utf-8")
        print(str(args.write_template))
        return 0
    if not args.probe or not args.oracle:
        parser.error("--probe and --oracle are required unless --self-test or --write-template is used")

    result = gate(_load_json(args.probe), _load_json(args.oracle))
    rendered = json.dumps(result, indent=2, sort_keys=True)
    if args.out:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(rendered + "\n", encoding="utf-8")
    print(rendered)
    return 0 if result["accepted"] else 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
