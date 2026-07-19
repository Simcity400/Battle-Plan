import json
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))

import verify_f6_pause_proof as verifier


class PauseProofTests(unittest.TestCase):
    def fixture(self, root: Path, *, paused=True, degraded=True, rung=verifier.RUNG):
        run = root / "run"
        snaps = root / "snaps"
        run.mkdir()
        snaps.mkdir()
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
        for hour in (1, 2):
            (snaps / f"{hour:012}.snapshot.json").write_text(json.dumps(snapshot), encoding="utf-8")
        replay = run / "native_session.failed.battleplan"
        replay.write_bytes(b"fixture replay")
        (run / "native_session_manifest.json").write_text(json.dumps({
            "session_id": "session-1", "capture_id": "capture-1", "replay_file": replay.name,
            "acceptance_test_observation": {
                "schema": "battle-plan.acceptance_test_observation.v1", "mode": "paused",
                "acceptance_test_nonce": nonce, "admissible_for_production": False, "ok": True,
                "non_d57_source_unavailable_domains": [24] if degraded else [],
            },
        }), encoding="utf-8")
        extraction = root / "extraction.json"
        extraction.write_text(json.dumps({
            "schema": "battle-plan.f6_snapshot_extraction.v1", "ok": True,
            "replay_path": str(replay.resolve()), "replay_sha256": verifier.sha256_file(replay),
            "snapshots": [
                {"extracted_path": str(path.resolve()), "sha256": verifier.sha256_file(path)}
                for path in sorted(snaps.glob("*.snapshot.json"))
            ],
        }), encoding="utf-8")
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


if __name__ == "__main__":
    unittest.main()
