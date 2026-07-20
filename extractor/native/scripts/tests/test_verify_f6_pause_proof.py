import json
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))

import verify_f6_pause_proof as verifier
from extract_f6_replay_snapshots import extract as extract_replay_snapshots


class PauseProofTests(unittest.TestCase):
    def fixture(self, root: Path, *, paused=True, degraded=True, rung=verifier.RUNG):
        run = root / "run"
        snaps = root / "snaps"
        run.mkdir()
        nonce = "a" * 64
        (run / "production_capture_plan.json").write_text(json.dumps({
            "rung": rung, "session_id": "session-1", "capture_id": "capture-1",
            "acceptance_mode": "paused", "acceptance_test_nonce": nonce,
            "admissible_for_production": False,
        }), encoding="utf-8")
        problem_rows = [{"domain_id": 24, "problem": "compact_status_failure"}] if degraded else []
        snapshot = {
            verifier.SIDECAR: {"pause": {"state": "known", "is_paused": paused}},
            "provenance": {"collector": {"domain_problems": problem_rows}},
        }
        replay = run / "native_session.failed.battleplan"
        snapshot_bytes = {
            hour: json.dumps({"hour_index": hour, **snapshot}, sort_keys=True).encode("utf-8")
            for hour in (1, 2)
        }
        index = {
            "game_id": "test-game",
            "snapshots": [
                {"hour_index": hour, "file": f"snapshots/{hour:012}.snapshot.json"}
                for hour in (1, 2)
            ],
        }
        with zipfile.ZipFile(replay, "w", compression=zipfile.ZIP_STORED) as archive:
            archive.writestr("payload/test-game/index.json", json.dumps(index, sort_keys=True))
            for hour, raw in snapshot_bytes.items():
                archive.writestr(f"payload/test-game/snapshots/{hour:012}.snapshot.json", raw)
        (run / "native_session_manifest.json").write_text(json.dumps({
            "session_id": "session-1", "capture_id": "capture-1", "replay_file": replay.name,
            "acceptance_test_observation": {
                "schema": "battle-plan.acceptance_test_observation.v1", "mode": "paused",
                "acceptance_test_nonce": nonce, "admissible_for_production": False, "ok": True,
                "non_d57_source_unavailable_domains": [24] if degraded else [],
            },
        }), encoding="utf-8")
        extraction = root / "extraction.json"
        extraction.write_text(json.dumps(extract_replay_snapshots(replay, snaps)), encoding="utf-8")
        return run, snaps, extraction

    def test_authenticated_pause_and_degrade_pass(self):
        with tempfile.TemporaryDirectory() as td:
            run, snaps, extraction = self.fixture(Path(td))
            self.assertTrue(verifier.verify(run, snaps, extraction)["ok"])

    def test_pause_without_degrade_fails(self):
        with tempfile.TemporaryDirectory() as td:
            run, snaps, extraction = self.fixture(Path(td), degraded=False)
            with self.assertRaises(verifier.VerifyError):
                verifier.verify(run, snaps, extraction)

    def test_wrong_rung_fails_closed(self):
        with tempfile.TemporaryDirectory() as td:
            run, snaps, extraction = self.fixture(Path(td), rung="speed5-short-proof")
            with self.assertRaises(verifier.VerifyError):
                verifier.verify(run, snaps, extraction)

    def test_missing_capture_identity_fails_closed(self):
        with tempfile.TemporaryDirectory() as td:
            run, snaps, extraction = self.fixture(Path(td))
            plan_path = run / "production_capture_plan.json"
            plan = json.loads(plan_path.read_text(encoding="utf-8"))
            del plan["capture_id"]
            plan_path.write_text(json.dumps(plan), encoding="utf-8")
            with self.assertRaises(verifier.VerifyError):
                verifier.verify(run, snaps, extraction)

    def test_mutated_extracted_snapshot_and_report_fail_replay_binding(self):
        with tempfile.TemporaryDirectory() as td:
            run, snaps, extraction = self.fixture(Path(td))
            snapshot_path = sorted(snaps.glob("*.snapshot.json"))[0]
            body = json.loads(snapshot_path.read_text(encoding="utf-8"))
            body["forged_after_extraction"] = True
            snapshot_path.write_text(json.dumps(body), encoding="utf-8")
            report = json.loads(extraction.read_text(encoding="utf-8"))
            report["snapshots"][0]["bytes"] = snapshot_path.stat().st_size
            report["snapshots"][0]["sha256"] = verifier.sha256_file(snapshot_path)
            extraction.write_text(json.dumps(report), encoding="utf-8")
            with self.assertRaises(verifier.VerifyError):
                verifier.verify(run, snaps, extraction)

    def test_swapped_retained_paths_fail_hour_binding(self):
        with tempfile.TemporaryDirectory() as td:
            run, snaps, extraction = self.fixture(Path(td))
            report = json.loads(extraction.read_text(encoding="utf-8"))
            first = report["snapshots"][0]["extracted_path"]
            report["snapshots"][0]["extracted_path"] = report["snapshots"][1]["extracted_path"]
            report["snapshots"][1]["extracted_path"] = first
            extraction.write_text(json.dumps(report), encoding="utf-8")
            with self.assertRaises(verifier.VerifyError):
                verifier.verify(run, snaps, extraction)


if __name__ == "__main__":
    unittest.main()
