import json
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))

import verify_f6_pause_attempt_receipt as receipt_verifier
import verify_f6_pause_proof as pause_verifier
from extract_f6_replay_snapshots import extract as extract_replay_snapshots
from verify_acceptance_fixture_variance import sha256_file


class PauseAttemptReceiptTests(unittest.TestCase):
    def fixture(self, root: Path):
        run = root / "run"
        snapshots = run / "acceptance_snapshots"
        markers = root / "markers"
        run.mkdir()
        markers.mkdir()
        nonce = "a" * 64
        session_id = "session-1"
        capture_id = "capture-1"

        plan_path = run / "production_capture_plan.json"
        plan_path.write_text(json.dumps({
            "rung": pause_verifier.RUNG,
            "session_id": session_id,
            "capture_id": capture_id,
            "acceptance_mode": "paused",
            "acceptance_test_nonce": nonce,
            "admissible_for_production": False,
        }), encoding="utf-8")
        snapshot = {
            "_r2_session_multiplayer_metadata_live": {
                "pause": {"state": "known", "is_paused": True},
            },
            "provenance": {"collector": {"domain_problems": [
                {"domain_id": 24, "problem": "compact_status_failure"},
            ]}},
        }
        replay = run / "native_session.failed.battleplan"
        index = {
            "game_id": "test-game",
            "snapshots": [
                {"hour_index": hour, "file": f"snapshots/{hour:012}.snapshot.json"}
                for hour in (1, 2)
            ],
        }
        with zipfile.ZipFile(replay, "w", compression=zipfile.ZIP_STORED) as archive:
            archive.writestr("payload/test-game/index.json", json.dumps(index, sort_keys=True))
            for hour in (1, 2):
                archive.writestr(
                    f"payload/test-game/snapshots/{hour:012}.snapshot.json",
                    json.dumps({"hour_index": hour, **snapshot}, sort_keys=True),
                )
        observation = {
            "schema": "battle-plan.acceptance_test_observation.v1",
            "mode": "paused",
            "acceptance_test_nonce": nonce,
            "admissible_for_production": False,
            "ok": True,
            "non_d57_source_unavailable_domains": [24],
        }
        manifest_path = run / "native_session_manifest.json"
        manifest_path.write_text(json.dumps({
            "session_id": session_id,
            "capture_id": capture_id,
            "replay_file": replay.name,
            "acceptance_test_observation": observation,
        }), encoding="utf-8")
        extraction_path = run / "acceptance_snapshot_extraction.json"
        extraction_path.write_text(
            json.dumps(extract_replay_snapshots(replay, snapshots)),
            encoding="utf-8",
        )
        proof_path = run / "acceptance_validation_report.json"
        proof = pause_verifier.verify(run, snapshots, extraction_path)
        proof_path.write_text(json.dumps(proof), encoding="utf-8")

        receipt_path = run / receipt_verifier.RECEIPT_NAME
        marker_path = markers / f"{session_id}.resolved.json"
        transported_marker_path = run / "resolved_attempt_marker.json"
        receipt = {
            "schema": receipt_verifier.RECEIPT_SCHEMA,
            "session_id": session_id,
            "capture_id": capture_id,
            "acceptance_mode": "paused",
            "rung": pause_verifier.RUNG,
            "capture_plan_path": str(plan_path.resolve()),
            "capture_plan_sha256": sha256_file(plan_path),
            "manifest_path": str(manifest_path.resolve()),
            "manifest_sha256": sha256_file(manifest_path),
            "validation_report_path": str(proof_path.resolve()),
            "validation_report_sha256": sha256_file(proof_path),
            "validation_report_schema": pause_verifier.SCHEMA,
            "receipt_path": str(receipt_path.resolve()),
            "resolved_attempt_marker_path": str(marker_path.resolve()),
            "transported_resolved_attempt_marker_path": str(transported_marker_path.resolve()),
            "resolved_attempt_marker_schema": receipt_verifier.MARKER_SCHEMA,
            "resolved_attempt_marker_state": "acceptance_validation_passed",
            "admissible_for_production": False,
            "acceptance_test_nonce": nonce,
            "snapshot_extraction_report_path": str(extraction_path.resolve()),
            "snapshot_extraction_report_sha256": sha256_file(extraction_path),
            "replay_path": str(replay.resolve()),
            "replay_sha256": sha256_file(replay),
            "acceptance_proof": proof,
            "acceptance_test_observation": observation,
        }
        receipt_path.write_text(json.dumps(receipt), encoding="utf-8")
        marker = {
            "schema": receipt_verifier.MARKER_SCHEMA,
            "state": "acceptance_validation_passed",
            "session_id": session_id,
            "capture_id": capture_id,
            "capture_plan_sha256": sha256_file(plan_path),
            "validation_report": str(proof_path.resolve()),
            "validation_receipt_path": str(receipt_path.resolve()),
            "validation_receipt_sha256": sha256_file(receipt_path),
            "validation_receipt": receipt,
            "admissible_for_production": False,
        }
        marker_path.write_text(json.dumps(marker), encoding="utf-8")
        transported_marker_path.write_bytes(marker_path.read_bytes())
        return run, receipt_path, marker_path

    def test_complete_portable_and_resolved_receipt_join_passes(self):
        with tempfile.TemporaryDirectory() as td:
            run, _, _ = self.fixture(Path(td))
            self.assertTrue(receipt_verifier.verify(run)["ok"])

    def test_transported_run_regrades_without_external_marker(self):
        with tempfile.TemporaryDirectory() as td:
            run, _, marker = self.fixture(Path(td))
            marker.unlink()
            result = receipt_verifier.verify(run)
            self.assertTrue(result["ok"])
            self.assertFalse(result["resolved_attempt_marker_available"])

    def test_missing_transported_marker_fails_closed(self):
        with tempfile.TemporaryDirectory() as td:
            run, _, _ = self.fixture(Path(td))
            (run / "resolved_attempt_marker.json").unlink()
            with self.assertRaises(receipt_verifier.VerifyError):
                receipt_verifier.verify(run)

    def test_non_boolean_production_admission_fails_closed(self):
        with tempfile.TemporaryDirectory() as td:
            run, receipt_path, _ = self.fixture(Path(td))
            receipt = json.loads(receipt_path.read_text(encoding="utf-8"))
            receipt["admissible_for_production"] = 0
            receipt_path.write_text(json.dumps(receipt), encoding="utf-8")
            with self.assertRaises(receipt_verifier.VerifyError):
                receipt_verifier.verify(run)

    def test_mutated_artifact_invalidates_receipt(self):
        with tempfile.TemporaryDirectory() as td:
            run, _, _ = self.fixture(Path(td))
            proof_path = run / "acceptance_validation_report.json"
            proof_path.write_text(proof_path.read_text(encoding="utf-8") + "\n", encoding="utf-8")
            with self.assertRaises(receipt_verifier.VerifyError):
                receipt_verifier.verify(run)

    def test_marker_embedded_receipt_divergence_fails_closed(self):
        with tempfile.TemporaryDirectory() as td:
            run, _, marker_path = self.fixture(Path(td))
            marker = json.loads(marker_path.read_text(encoding="utf-8"))
            marker["validation_receipt"]["acceptance_test_nonce"] = "b" * 64
            marker_path.write_text(json.dumps(marker), encoding="utf-8")
            with self.assertRaises(receipt_verifier.VerifyError):
                receipt_verifier.verify(run)

    def test_marker_non_boolean_production_admission_fails_closed(self):
        with tempfile.TemporaryDirectory() as td:
            run, _, marker_path = self.fixture(Path(td))
            marker = json.loads(marker_path.read_text(encoding="utf-8"))
            marker["admissible_for_production"] = 0
            marker_path.write_text(json.dumps(marker), encoding="utf-8")
            (run / "resolved_attempt_marker.json").write_bytes(marker_path.read_bytes())
            with self.assertRaises(receipt_verifier.VerifyError):
                receipt_verifier.verify(run)


if __name__ == "__main__":
    unittest.main()
