#!/usr/bin/env python3
"""Re-grade the portable and external receipts for one F6 paused attempt.

The underlying pause verdict is recomputed from the retained replay.  This layer
then proves that the run-local receipt and the external resolved-attempt marker
describe those exact bytes and the same capture/session/nonce.  Neither receipt
is accepted as evidence by itself.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

from verify_acceptance_fixture_variance import VerifyError, sha256_file
from verify_f6_pause_proof import RUNG, SCHEMA as PROOF_SCHEMA, verify as verify_pause


SCHEMA = "battle-plan.f6_pause_attempt_receipt.v1"
RECEIPT_SCHEMA = "battle-plan.production_capture_validation_receipt.v1"
MARKER_SCHEMA = "battle-plan.production_capture_attempt.v1"
RECEIPT_NAME = "capture_validation_receipt.json"


def load_object(path: Path, label: str) -> dict:
    if not path.is_file():
        raise VerifyError(f"{label} is missing: {path}")
    value = json.loads(path.read_text(encoding="utf-8-sig"))
    if not isinstance(value, dict):
        raise VerifyError(f"{label} root must be an object")
    return value


def canonical_path(raw: object, expected: Path, label: str) -> Path:
    if not isinstance(raw, str) or not raw:
        raise VerifyError(f"{label} path must be a nonblank string")
    actual = Path(raw).resolve()
    if actual != expected.resolve():
        raise VerifyError(f"{label} path is not canonical for this run")
    return actual


def require_hash(value: object, label: str) -> str:
    if (
        not isinstance(value, str)
        or len(value) != 64
        or any(char not in "0123456789abcdef" for char in value)
    ):
        raise VerifyError(f"{label} must be lowercase SHA-256")
    return value


def verify(run_dir: Path) -> dict:
    run = run_dir.resolve()
    if not run.is_dir():
        raise VerifyError(f"run directory is missing: {run}")

    plan_path = run / "production_capture_plan.json"
    manifest_path = run / "native_session_manifest.json"
    extraction_path = run / "acceptance_snapshot_extraction.json"
    proof_path = run / "acceptance_validation_report.json"
    snapshots = run / "acceptance_snapshots"
    receipt_path = run / RECEIPT_NAME
    transported_marker_path = run / "resolved_attempt_marker.json"

    plan = load_object(plan_path, "capture plan")
    manifest = load_object(manifest_path, "session manifest")
    extraction = load_object(extraction_path, "snapshot extraction report")
    reported_proof = load_object(proof_path, "pause proof report")
    receipt = load_object(receipt_path, "portable validation receipt")

    if plan.get("rung") != RUNG or plan.get("acceptance_mode") != "paused":
        raise VerifyError("capture plan is not the F6 paused-short-proof rung")
    if plan.get("admissible_for_production") is not False:
        raise VerifyError("paused capture plan does not explicitly deny production admission")
    session_id = plan.get("session_id")
    capture_id = plan.get("capture_id")
    nonce = plan.get("acceptance_test_nonce")
    if not isinstance(session_id, str) or not session_id:
        raise VerifyError("capture plan session_id is missing")
    if not isinstance(capture_id, str) or not capture_id:
        raise VerifyError("capture plan capture_id is missing")
    require_hash(nonce, "capture plan acceptance nonce")
    if manifest.get("session_id") != session_id or manifest.get("capture_id") != capture_id:
        raise VerifyError("session manifest is not exactly bound to capture/session identity")

    if receipt.get("schema") != RECEIPT_SCHEMA:
        raise VerifyError("portable validation receipt schema is wrong")
    exact_values = {
        "session_id": session_id,
        "capture_id": capture_id,
        "acceptance_mode": "paused",
        "rung": RUNG,
        "acceptance_test_nonce": nonce,
        "validation_report_schema": PROOF_SCHEMA,
        "resolved_attempt_marker_schema": MARKER_SCHEMA,
        "resolved_attempt_marker_state": "acceptance_validation_passed",
    }
    for key, expected in exact_values.items():
        if receipt.get(key) != expected:
            raise VerifyError(f"portable validation receipt {key} is unbound")
    if receipt.get("admissible_for_production") is not False:
        raise VerifyError("portable validation receipt does not deny production admission")

    artifact_paths = {
        "capture_plan": ("capture_plan_path", "capture_plan_sha256", plan_path),
        "manifest": ("manifest_path", "manifest_sha256", manifest_path),
        "snapshot extraction": (
            "snapshot_extraction_report_path",
            "snapshot_extraction_report_sha256",
            extraction_path,
        ),
        "pause proof": (
            "validation_report_path",
            "validation_report_sha256",
            proof_path,
        ),
        "portable receipt": ("receipt_path", None, receipt_path),
        "transported resolved marker": (
            "transported_resolved_attempt_marker_path",
            None,
            transported_marker_path,
        ),
    }
    for label, (path_key, hash_key, expected_path) in artifact_paths.items():
        actual_path = canonical_path(receipt.get(path_key), expected_path, label)
        if hash_key is not None:
            expected_hash = require_hash(receipt.get(hash_key), f"{label} hash")
            if sha256_file(actual_path) != expected_hash:
                raise VerifyError(f"{label} hash differs from portable receipt")

    replay_file = manifest.get("replay_file")
    if not isinstance(replay_file, str) or not replay_file:
        raise VerifyError("session manifest replay_file is missing")
    replay_path = (run / replay_file).resolve()
    try:
        replay_path.relative_to(run)
    except ValueError as exc:
        raise VerifyError("session manifest replay_file escapes the run directory") from exc
    canonical_path(receipt.get("replay_path"), replay_path, "replay")
    replay_hash = require_hash(receipt.get("replay_sha256"), "replay hash")
    if not replay_path.is_file() or sha256_file(replay_path) != replay_hash:
        raise VerifyError("retained replay bytes differ from portable receipt")
    if extraction.get("replay_sha256") != replay_hash:
        raise VerifyError("snapshot extraction and portable receipt replay hashes differ")

    recomputed = verify_pause(run, snapshots, extraction_path)
    if recomputed.get("ok") is not True or recomputed.get("schema") != PROOF_SCHEMA:
        raise VerifyError("independent paused proof re-grade did not pass canonically")
    if reported_proof != recomputed:
        raise VerifyError("retained pause proof report differs from independent re-grade")
    if receipt.get("acceptance_proof") != recomputed:
        raise VerifyError("portable validation receipt embeds a different pause proof")
    if receipt.get("acceptance_test_observation") != manifest.get("acceptance_test_observation"):
        raise VerifyError("portable validation receipt embeds a different acceptance observation")

    marker_raw = receipt.get("resolved_attempt_marker_path")
    if not isinstance(marker_raw, str) or not marker_raw:
        raise VerifyError("portable validation receipt lacks the resolved marker path")
    marker_path = Path(marker_raw).resolve()
    if marker_path.name != f"{session_id}.resolved.json":
        raise VerifyError("resolved attempt marker filename is not session-bound")
    transported_marker = load_object(transported_marker_path, "transported resolved attempt marker")
    external_marker_available = marker_path.is_file()
    if external_marker_available:
        marker = load_object(marker_path, "resolved attempt marker")
        if sha256_file(transported_marker_path) != sha256_file(marker_path) or transported_marker != marker:
            raise VerifyError("transported and external resolved-attempt markers differ")
    else:
        # Archived runs remain mechanically regradable after transport. The local
        # byte-for-byte marker copy contains the receipt hash and embedded receipt;
        # the replay-derived proof, not marker location, remains the authority.
        marker = transported_marker
    marker_exact = {
        "schema": MARKER_SCHEMA,
        "state": "acceptance_validation_passed",
        "session_id": session_id,
        "capture_id": capture_id,
        "validation_report": str(proof_path),
        "validation_receipt_path": str(receipt_path),
        "admissible_for_production": False,
    }
    for key, expected in marker_exact.items():
        if marker.get(key) != expected:
            raise VerifyError(f"resolved attempt marker {key} is unbound")
    if marker.get("admissible_for_production") is not False:
        raise VerifyError("resolved attempt marker does not strictly deny production admission")
    if marker.get("capture_plan_sha256") != sha256_file(plan_path):
        raise VerifyError("resolved attempt marker capture-plan hash differs")
    if marker.get("validation_receipt_sha256") != sha256_file(receipt_path):
        raise VerifyError("resolved attempt marker portable-receipt hash differs")
    if marker.get("validation_receipt") != receipt:
        raise VerifyError("resolved attempt marker embeds a different validation receipt")

    return {
        "schema": SCHEMA,
        "ok": True,
        "run_dir": str(run),
        "session_id": session_id,
        "capture_id": capture_id,
        "acceptance_test_nonce": nonce,
        "receipt_path": str(receipt_path),
        "receipt_sha256": sha256_file(receipt_path),
        "resolved_attempt_marker_path": str(marker_path),
        "resolved_attempt_marker_available": external_marker_available,
        "resolved_attempt_marker_sha256": sha256_file(marker_path) if external_marker_available else None,
        "transported_resolved_attempt_marker_path": str(transported_marker_path),
        "transported_resolved_attempt_marker_sha256": sha256_file(transported_marker_path),
        "pause_proof_sha256": sha256_file(proof_path),
        "replay_sha256": replay_hash,
        "admissible_for_production": False,
        "problems": [],
    }


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run-dir", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args(argv)
    try:
        report = verify(args.run_dir)
    except (OSError, UnicodeError, json.JSONDecodeError, VerifyError) as exc:
        report = {
            "schema": SCHEMA,
            "ok": False,
            "run_dir": str(args.run_dir),
            "admissible_for_production": False,
            "problems": [str(exc)],
        }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0 if report["ok"] else 2


if __name__ == "__main__":
    sys.exit(main())
