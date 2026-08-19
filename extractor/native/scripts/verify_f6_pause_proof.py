#!/usr/bin/env python3
"""Build an attempt-bound F6 paused-short-proof verdict from folded snapshots.

This prover requires an exact ladder capture-plan binding, an authenticated D57
pause projection (``state=known`` and ``is_paused=true``), and at least one
non-D57 domain failure in the same hourly snapshot.  It does not accept a
hand-authored ``pause_proof`` boolean in the recorder manifest.
"""

from __future__ import annotations

import argparse
import json
import sys
import tempfile
import zipfile
from pathlib import Path

from extract_f6_replay_snapshots import ExtractError, extract as extract_replay_snapshots
from verify_acceptance_fixture_variance import (
    VerifyError,
    evidence_ref,
    load_snapshots,
    sha256_file,
)


SCHEMA = "battle-plan.f6_pause_proof.v1"
RUNG = "paused-short-proof"
SIDECAR = "_r2_session_multiplayer_metadata_live"


def extraction_inventory(report: dict) -> list[tuple[int, int, str, int, str]]:
    rows = report.get("snapshots")
    if not isinstance(rows, list):
        raise VerifyError("snapshot extraction report has no snapshot inventory")
    inventory = []
    for index, row in enumerate(rows):
        if not isinstance(row, dict):
            raise VerifyError(f"snapshot extraction row {index} is not an object")
        ordinal = row.get("ordinal")
        hour = row.get("hour_index")
        container_path = row.get("container_path")
        byte_count = row.get("bytes")
        digest = row.get("sha256")
        if (
            isinstance(ordinal, bool)
            or not isinstance(ordinal, int)
            or ordinal != index
            or isinstance(hour, bool)
            or not isinstance(hour, int)
            or not isinstance(container_path, str)
            or isinstance(byte_count, bool)
            or not isinstance(byte_count, int)
            or byte_count < 0
            or not isinstance(digest, str)
        ):
            raise VerifyError(f"snapshot extraction row {index} is malformed")
        inventory.append((ordinal, hour, container_path, byte_count, digest))
    return inventory


def verify(run_dir: Path, snapshot_dir: Path, extraction_report_path: Path) -> dict:
    plan_path = run_dir / "production_capture_plan.json"
    manifest_path = run_dir / "native_session_manifest.json"
    if not plan_path.is_file() or not manifest_path.is_file():
        raise VerifyError("run lacks production_capture_plan.json or native_session_manifest.json")
    plan = json.loads(plan_path.read_text(encoding="utf-8-sig"))
    manifest = json.loads(manifest_path.read_text(encoding="utf-8-sig"))
    if not isinstance(plan, dict) or plan.get("rung") != RUNG:
        raise VerifyError(f"capture plan is not bound to rung {RUNG!r}")
    if plan.get("acceptance_mode") != "paused" or plan.get("admissible_for_production") is not False:
        raise VerifyError("capture plan is not explicitly bound to non-production paused acceptance mode")
    if not isinstance(manifest, dict):
        raise VerifyError("native session manifest root must be an object")
    session_id = plan.get("session_id")
    capture_id = plan.get("capture_id")
    if not isinstance(session_id, str) or not session_id.strip():
        raise VerifyError("capture plan session_id must be a nonblank string")
    if not isinstance(capture_id, str) or not capture_id.strip():
        raise VerifyError("capture plan capture_id must be a nonblank string")
    if session_id != manifest.get("session_id"):
        raise VerifyError("capture plan and manifest session_id differ")
    manifest_capture_id = manifest.get("capture_id")
    if manifest_capture_id is not None and manifest_capture_id != capture_id:
        raise VerifyError("capture plan and manifest capture_id differ")
    nonce = plan.get("acceptance_test_nonce")
    if not isinstance(nonce, str) or len(nonce) != 64 or any(c not in "0123456789abcdef" for c in nonce):
        raise VerifyError("capture plan acceptance nonce is noncanonical")
    observation = manifest.get("acceptance_test_observation")
    if not isinstance(observation, dict):
        raise VerifyError("manifest lacks acceptance_test_observation")
    if (
        observation.get("schema") != "battle-plan.acceptance_test_observation.v1"
        or observation.get("mode") != "paused"
        or observation.get("acceptance_test_nonce") != nonce
        or observation.get("admissible_for_production") is not False
        or observation.get("ok") is not True
    ):
        raise VerifyError("manifest paused acceptance observation is absent, failed, or unbound")
    source_unavailable_domains = observation.get("non_d57_source_unavailable_domains")
    if not isinstance(source_unavailable_domains, list) or not source_unavailable_domains or any(isinstance(value, bool) or not isinstance(value, int) or value == 57 for value in source_unavailable_domains):
        raise VerifyError("manifest lacks a non-D57 SOURCE_UNAVAILABLE observation")

    extraction = json.loads(extraction_report_path.read_text(encoding="utf-8-sig"))
    if not isinstance(extraction, dict) or extraction.get("schema") != "battle-plan.f6_snapshot_extraction.v1" or extraction.get("ok") is not True:
        raise VerifyError("snapshot extraction report is absent or failed")
    replay_file = manifest.get("replay_file")
    if not isinstance(replay_file, str) or not replay_file:
        raise VerifyError("manifest replay_file is missing")
    replay_path = (run_dir / replay_file).resolve()
    if Path(str(extraction.get("replay_path", ""))).resolve() != replay_path:
        raise VerifyError("snapshot extraction report is not bound to manifest replay_file")
    if extraction.get("replay_sha256") != sha256_file(replay_path):
        raise VerifyError("snapshot extraction replay hash does not match retained replay")
    extracted_rows = extraction.get("snapshots")
    if not isinstance(extracted_rows, list) or len(extracted_rows) < 2:
        raise VerifyError("snapshot extraction report contains fewer than two snapshots")
    try:
        with tempfile.TemporaryDirectory(prefix="hoi4_f6_pause_reextract_") as temp_root:
            fresh = extract_replay_snapshots(replay_path, Path(temp_root) / "snapshots")
    except (OSError, UnicodeError, json.JSONDecodeError, zipfile.BadZipFile, ExtractError) as exc:
        raise VerifyError(f"independent replay snapshot extraction failed: {exc}") from exc
    for key in (
        "replay_sha256",
        "index_path",
        "index_sha256",
        "game_id",
        "snapshot_count",
        "snapshot_inventory_sha256",
    ):
        if extraction.get(key) != fresh.get(key):
            raise VerifyError(f"snapshot extraction report {key} differs from independent replay extraction")
    if extraction_inventory(extraction) != extraction_inventory(fresh):
        raise VerifyError("snapshot extraction inventory differs from independent replay extraction")
    snapshot_root = snapshot_dir.resolve()
    extracted_hashes = {}
    extracted_hours = {}
    for index, row in enumerate(extracted_rows):
        extracted_path = row.get("extracted_path")
        hour = row.get("hour_index")
        if not isinstance(extracted_path, str) or isinstance(hour, bool) or not isinstance(hour, int):
            raise VerifyError(f"snapshot extraction row {index} lacks its retained path/hour binding")
        resolved_path = Path(extracted_path).resolve()
        expected_path = snapshot_root / f"{hour:012}.snapshot.json"
        if resolved_path != expected_path or str(resolved_path) in extracted_hashes:
            raise VerifyError(f"snapshot extraction row {index} has a noncanonical or duplicate retained path")
        extracted_hashes[str(resolved_path)] = row.get("sha256")
        extracted_hours[str(resolved_path)] = hour

    snapshots = load_snapshots(snapshot_dir)
    if len(snapshots) != len(extracted_rows) or set(extracted_hashes) != {snap["path"] for snap in snapshots}:
        raise VerifyError("retained snapshot directory does not exactly match the extraction report")
    for snap in snapshots:
        if extracted_hashes.get(snap["path"]) != snap["sha256"]:
            raise VerifyError("snapshot bytes are not bound by the extraction report")
        if snap["hour_index"] != extracted_hours[snap["path"]] or snap["value"].get("hour_index") != snap["hour_index"]:
            raise VerifyError("retained snapshot filename, extraction row, and body hour differ")
        value = snap["value"]
        sidecar = value.get(SIDECAR)
        if not isinstance(sidecar, dict):
            continue
        pause = sidecar.get("pause")
        if not isinstance(pause, dict) or pause.get("state") != "known" or pause.get("is_paused") is not True:
            continue
        provenance = value.get("provenance")
        collector = provenance.get("collector") if isinstance(provenance, dict) else None
        problems = collector.get("domain_problems") if isinstance(collector, dict) else None
        if not isinstance(problems, list):
            raise VerifyError("paused snapshot lacks a canonical domain_problems array")
        degraded = []
        for problem in problems:
            if not isinstance(problem, dict):
                raise VerifyError("domain problem row must be an object")
            domain_id = problem.get("domain_id")
            if isinstance(domain_id, bool) or not isinstance(domain_id, int):
                raise VerifyError("domain problem domain_id must be an integer")
            if domain_id != 57 and problem.get("problem") == "compact_status_failure":
                degraded.append(domain_id)
        degraded_source_unavailable = sorted(set(degraded).intersection(source_unavailable_domains))
        if degraded_source_unavailable:
            return {
                "schema": SCHEMA,
                "ok": True,
                "run_dir": str(run_dir.resolve()),
                "session_id": session_id,
                "capture_id": capture_id,
                "plan_path": str(plan_path.resolve()),
                "plan_sha256": sha256_file(plan_path),
                "manifest_path": str(manifest_path.resolve()),
                "manifest_sha256": sha256_file(manifest_path),
                "extraction_report_path": str(extraction_report_path.resolve()),
                "extraction_report_sha256": sha256_file(extraction_report_path),
                "replay_path": str(replay_path),
                "replay_sha256": extraction["replay_sha256"],
                "acceptance_test_nonce": nonce,
                "pause_observation": evidence_ref(snap),
                "degraded_domain_ids": degraded_source_unavailable,
                "proof": {"observed_paused": True, "source_unavailable_degrade": True},
                "problems": [],
            }
    return {
        "schema": SCHEMA,
        "ok": False,
        "run_dir": str(run_dir.resolve()),
        "session_id": session_id,
        "capture_id": capture_id,
        "proof": {"observed_paused": False, "source_unavailable_degrade": False},
        "problems": ["no same-snapshot authenticated pause plus non-D57 compact_status_failure observation"],
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
    except (OSError, UnicodeError, json.JSONDecodeError, VerifyError) as exc:
        report = {"schema": SCHEMA, "ok": False, "run_dir": str(args.run_dir), "proof": {"observed_paused": False, "source_unavailable_degrade": False}, "problems": [str(exc)]}
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0 if report["ok"] else 2


if __name__ == "__main__":
    sys.exit(main())
