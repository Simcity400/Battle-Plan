#!/usr/bin/env python3
"""Badge current per-division manpower as unavailable from domain-20 compact rows.

This is intentionally scoped. It does not prove the game lacks a manpower field
somewhere in memory; it proves the current division compact path does not emit
or reserve a production manpower slot, and it records the owner for alternative
sources.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Any


FORBIDDEN_SOURCE_PATTERNS = {
    "collector_division_manpower_offset": re.compile(r"DIVISION_OFF_[A-Z0-9_]*MANPOWER"),
    "collector_division_manpower_flag": re.compile(r"COMPACT_DIVISION_ROW_FLAG_MANPOWER"),
    "recorder_division_manpower_flag": re.compile(r"COMPACT_DIVISION_ROW_FLAG_MANPOWER"),
    "lane_domain20_manpower_field": re.compile(r"(?i)(row_flag_manpower|division_manpower_rows|raw_manpower)"),
}


def _load_json(path: Path) -> dict[str, Any]:
    data = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(data, dict):
        raise ValueError(f"{path} top-level JSON must be an object")
    return data


def _read(path: Path) -> str:
    return path.read_text(encoding="utf-8", errors="replace")


def gate(
    *,
    summary_path: Path,
    collector_path: Path,
    recorder_path: Path,
    lane_domain_path: Path,
) -> dict[str, Any]:
    reasons: list[str] = []
    evidence: dict[str, Any] = {}
    summary = _load_json(summary_path)
    units = summary.get("data_units_total") if isinstance(summary.get("data_units_total"), dict) else {}

    division_rows = int(units.get("division_rows", 0) or 0)
    reserved_tail_zero_rows = int(units.get("division_reserved_tail_zero_rows", 0) or 0)
    manpower_rows = int(units.get("division_manpower_rows", 0) or 0)

    if summary.get("domain_id") != 20 or summary.get("domain_name") != "divisions":
        reasons.append("summary is not domain-20 divisions")
    if summary.get("has_failures") is not False:
        reasons.append("domain-20 summary has failures")
    if summary.get("errors"):
        reasons.append(f"domain-20 summary has errors: {summary.get('errors')!r}")
    if division_rows <= 0:
        reasons.append("domain-20 summary has no division rows")
    if reserved_tail_zero_rows != division_rows:
        reasons.append(
            f"reserved tail zero rows {reserved_tail_zero_rows} != division rows {division_rows}"
        )
    if manpower_rows != 0:
        reasons.append(f"domain-20 summary already reports division_manpower_rows={manpower_rows}")

    collector = _read(collector_path)
    recorder = _read(recorder_path)
    lane_domain = _read(lane_domain_path)
    source_checks = {
        "collector_division_manpower_offset": FORBIDDEN_SOURCE_PATTERNS[
            "collector_division_manpower_offset"
        ].search(collector)
        is None,
        "collector_division_manpower_flag": FORBIDDEN_SOURCE_PATTERNS[
            "collector_division_manpower_flag"
        ].search(collector)
        is None,
        "recorder_division_manpower_flag": FORBIDDEN_SOURCE_PATTERNS[
            "recorder_division_manpower_flag"
        ].search(recorder)
        is None,
        "lane_domain20_manpower_field": FORBIDDEN_SOURCE_PATTERNS[
            "lane_domain20_manpower_field"
        ].search(lane_domain)
        is None,
    }
    for key, ok in source_checks.items():
        if not ok:
            reasons.append(f"source check failed: {key}")

    evidence["summary_path"] = str(summary_path)
    evidence["division_rows"] = division_rows
    evidence["division_reserved_tail_zero_rows"] = reserved_tail_zero_rows
    evidence["division_manpower_rows"] = manpower_rows
    evidence["source_checks"] = source_checks

    ownership = {
        "division_state_movement_live": "badge unavailable for current per-division manpower in domain-20 compact rows",
        "designs_or_template_lane": "owns template manpower requirements after exact live roots are validated",
        "country_economy_or_manpower_lane": "owns country manpower pools and deployable manpower",
        "combat_losses_or_stats_lanes": "own killed/lost manpower deltas by combat/stat context",
        "save_oracle": "development validation only, not hot-loop production capture",
    }

    accepted = not reasons
    return {
        "schema": "division_manpower_compact_path_gate_v1",
        "accepted": accepted,
        "status": (
            "unavailable_from_current_domain20_compact_path"
            if accepted
            else "not_badged_due_to_failed_evidence"
        ),
        "evidence": evidence,
        "ownership": ownership,
        "reasons": reasons,
    }


def run_self_test() -> None:
    import tempfile

    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        summary = root / "summary.json"
        collector = root / "collector.rs"
        recorder = root / "recorder.rs"
        lane = root / "domain20.rs"
        summary.write_text(
            json.dumps(
                {
                    "domain_id": 20,
                    "domain_name": "divisions",
                    "has_failures": False,
                    "errors": [],
                    "data_units_total": {
                        "division_rows": 2,
                        "division_reserved_tail_zero_rows": 2,
                    },
                }
            ),
            encoding="utf-8",
        )
        collector.write_text("const DIVISION_OFF_STRENGTH: usize = 0x410;", encoding="utf-8")
        recorder.write_text("const COMPACT_DIVISION_ROW_FLAG_TEMPLATE: u32 = 1 << 13;", encoding="utf-8")
        lane.write_text("pub const ROW_FLAG_TEMPLATE: u32 = 1 << 13;", encoding="utf-8")
        if not gate(
            summary_path=summary,
            collector_path=collector,
            recorder_path=recorder,
            lane_domain_path=lane,
        )["accepted"]:
            raise AssertionError("expected compact-path unavailable badge to pass")
        collector.write_text("const DIVISION_OFF_MANPOWER: usize = 0x123;", encoding="utf-8")
        if gate(
            summary_path=summary,
            collector_path=collector,
            recorder_path=recorder,
            lane_domain_path=lane,
        )["accepted"]:
            raise AssertionError("expected manpower source pattern to fail")


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--summary", type=Path)
    parser.add_argument("--collector", type=Path, default=Path("extractor/native/collector/src/lib.rs"))
    parser.add_argument("--recorder", type=Path, default=Path("extractor/native/recorder_host/src/main.rs"))
    parser.add_argument(
        "--lane-domain",
        type=Path,
        default=Path("extractor/native/lanes/division_state_movement_live/domain20_compact.rs"),
    )
    parser.add_argument("--out", type=Path)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)

    if args.self_test:
        run_self_test()
        print("manpower_compact_path_gate self-test passed")
        return 0
    if not args.summary:
        parser.error("--summary is required unless --self-test is used")

    result = gate(
        summary_path=args.summary,
        collector_path=args.collector,
        recorder_path=args.recorder,
        lane_domain_path=args.lane_domain,
    )
    rendered = json.dumps(result, indent=2, sort_keys=True)
    if args.out:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(rendered + "\n", encoding="utf-8")
    print(rendered)
    return 0 if result["accepted"] else 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
