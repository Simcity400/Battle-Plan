#!/usr/bin/env python3
"""Verify an authenticated one-shot weather worker panic survived framing and fold."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

from verify_acceptance_fixture_variance import VerifyError, evidence_ref, load_snapshots, sha256_file

SCHEMA = "battle-plan.f6_synthetic_fault_proof.v1"
RUNG = "synthetic-fault"
WORKER_PANIC = 1 << 6
SOURCE_UNAVAILABLE = 1 << 8


def verify(run_dir: Path, snapshot_dir: Path, extraction_report_path: Path) -> dict:
    plan_path = run_dir / "production_capture_plan.json"
    manifest_path = run_dir / "native_session_manifest.json"
    plan = json.loads(plan_path.read_text(encoding="utf-8-sig"))
    manifest = json.loads(manifest_path.read_text(encoding="utf-8-sig"))
    if not isinstance(plan, dict) or plan.get("rung") != RUNG:
        raise VerifyError(f"capture plan is not bound to rung {RUNG!r}")
    if plan.get("acceptance_mode") != "synthetic-fault" or plan.get("admissible_for_production") is not False:
        raise VerifyError("capture plan is not explicitly bound to non-production synthetic-fault mode")
    if not isinstance(manifest, dict):
        raise VerifyError("native session manifest root must be an object")
    for key in ("session_id", "capture_id"):
        if not isinstance(plan.get(key), str) or not plan[key]:
            raise VerifyError(f"capture plan {key} is missing")
        if manifest.get(key) is not None and manifest.get(key) != plan[key]:
            raise VerifyError(f"capture plan and manifest {key} differ")
    nonce = plan.get("acceptance_test_nonce")
    if not isinstance(nonce, str) or len(nonce) != 64 or any(c not in "0123456789abcdef" for c in nonce):
        raise VerifyError("capture plan acceptance nonce is noncanonical")
    observation = manifest.get("acceptance_test_observation")
    if not isinstance(observation, dict):
        raise VerifyError("manifest lacks acceptance_test_observation")
    weather_flags = observation.get("weather_effective_failure_status_flags_or")
    if isinstance(weather_flags, bool) or not isinstance(weather_flags, int):
        raise VerifyError("weather effective failure flags are not an integer")
    if (
        observation.get("schema") != "battle-plan.acceptance_test_observation.v1"
        or observation.get("mode") != "synthetic-fault"
        or observation.get("acceptance_test_nonce") != nonce
        or observation.get("admissible_for_production") is not False
        or observation.get("framing_intact") is not True
        or observation.get("ok") is not True
        or weather_flags & (WORKER_PANIC | SOURCE_UNAVAILABLE) != (WORKER_PANIC | SOURCE_UNAVAILABLE)
    ):
        raise VerifyError("manifest synthetic-fault observation is absent, failed, or unbound")

    compact = manifest.get("compact_stream")
    if not isinstance(compact, dict):
        raise VerifyError("manifest compact_stream summary is missing")
    if (
        compact.get("frame_end_record_count", 0) <= 0
        or compact.get("parse_errors") not in ([], None)
        or compact.get("ordering_errors") not in ([], None)
        or compact.get("late_records_after_frame_end", 0) != 0
    ):
        raise VerifyError("compact stream framing did not survive the injected panic")

    extraction = json.loads(extraction_report_path.read_text(encoding="utf-8-sig"))
    if not isinstance(extraction, dict) or extraction.get("schema") != "battle-plan.f6_snapshot_extraction.v1" or extraction.get("ok") is not True:
        raise VerifyError("snapshot extraction report is absent or failed")
    replay_file = manifest.get("replay_file")
    if not isinstance(replay_file, str) or not replay_file:
        raise VerifyError("manifest replay_file is missing")
    replay_path = (run_dir / replay_file).resolve()
    if Path(str(extraction.get("replay_path", ""))).resolve() != replay_path or extraction.get("replay_sha256") != sha256_file(replay_path):
        raise VerifyError("snapshot extraction is not bound to the retained replay")
    extracted_rows = extraction.get("snapshots")
    if not isinstance(extracted_rows, list) or len(extracted_rows) < 2:
        raise VerifyError("snapshot extraction report contains fewer than two snapshots")
    extracted_hashes = {str(Path(row["extracted_path"]).resolve()): row.get("sha256") for row in extracted_rows if isinstance(row, dict) and isinstance(row.get("extracted_path"), str)}

    fault_snapshots = []
    for snap in load_snapshots(snapshot_dir):
        if extracted_hashes.get(snap["path"]) != snap["sha256"]:
            raise VerifyError("snapshot bytes are not bound by the extraction report")
        collector = snap["value"].get("provenance", {}).get("collector", {})
        problems = collector.get("domain_problems") if isinstance(collector, dict) else None
        if not isinstance(problems, list):
            raise VerifyError("snapshot lacks canonical provenance.collector.domain_problems")
        if any(isinstance(row, dict) and row.get("domain_id") == 24 and row.get("problem") == "compact_status_failure" for row in problems):
            fault_snapshots.append(evidence_ref(snap))
    if len(fault_snapshots) != 1:
        raise VerifyError(f"expected one folded weather failure hour, found {len(fault_snapshots)}")
    return {
        "schema": SCHEMA,
        "ok": True,
        "run_dir": str(run_dir.resolve()),
        "session_id": plan["session_id"],
        "capture_id": plan["capture_id"],
        "acceptance_test_nonce": nonce,
        "plan_sha256": sha256_file(plan_path),
        "manifest_sha256": sha256_file(manifest_path),
        "extraction_report_sha256": sha256_file(extraction_report_path),
        "replay_sha256": extraction["replay_sha256"],
        "fault_observation": fault_snapshots[0],
        "proof": {"worker_panic": True, "source_unavailable": True, "framing_survived": True, "folded_once": True},
        "problems": [],
    }


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run-dir", required=True, type=Path)
    parser.add_argument("--snapshot-dir", required=True, type=Path)
    parser.add_argument("--extraction-report", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args(argv)
    try:
        report = verify(args.run_dir, args.snapshot_dir, args.extraction_report)
    except (OSError, UnicodeError, json.JSONDecodeError, VerifyError, KeyError, TypeError) as exc:
        report = {"schema": SCHEMA, "ok": False, "run_dir": str(args.run_dir), "proof": {"worker_panic": False, "source_unavailable": False, "framing_survived": False, "folded_once": False}, "problems": [str(exc)]}
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0 if report["ok"] else 2


if __name__ == "__main__":
    sys.exit(main())
