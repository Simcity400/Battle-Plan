import json
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))

import verify_f6_synthetic_fault_proof as verifier


class SyntheticFaultProofTests(unittest.TestCase):
    def fixture(self, root: Path, *, fault_hours=(1,)):
        run = root / "run"
        snaps = root / "snaps"
        run.mkdir(); snaps.mkdir()
        nonce = "b" * 64
        (run / "production_capture_plan.json").write_text(json.dumps({
            "rung": verifier.RUNG, "session_id": "session-1", "capture_id": "capture-1",
            "acceptance_mode": "synthetic-fault", "acceptance_test_nonce": nonce,
            "admissible_for_production": False,
        }), encoding="utf-8")
        for hour in (1, 2):
            problems = [{"domain_id": 24, "problem": "compact_status_failure"}] if hour in fault_hours else []
            (snaps / f"{hour:012}.snapshot.json").write_text(json.dumps({
                "hour_index": hour, "provenance": {"collector": {"domain_problems": problems}},
            }), encoding="utf-8")
        replay = run / "native_session.failed.battleplan"
        replay.write_bytes(b"fixture replay")
        (run / "native_session_manifest.json").write_text(json.dumps({
            "session_id": "session-1", "capture_id": "capture-1", "replay_file": replay.name,
            "acceptance_test_observation": {
                "schema": "battle-plan.acceptance_test_observation.v1", "mode": "synthetic-fault",
                "acceptance_test_nonce": nonce, "admissible_for_production": False,
                "framing_intact": True, "ok": True,
                "weather_effective_failure_status_flags_or": verifier.WORKER_PANIC | verifier.SOURCE_UNAVAILABLE,
            },
            "compact_stream": {"frame_end_record_count": 2, "parse_errors": [], "ordering_errors": [], "late_records_after_frame_end": 0},
        }), encoding="utf-8")
        extraction = root / "extraction.json"
        extraction.write_text(json.dumps({
            "schema": "battle-plan.f6_snapshot_extraction.v1", "ok": True,
            "replay_path": str(replay.resolve()), "replay_sha256": verifier.sha256_file(replay),
            "snapshots": [{"extracted_path": str(path.resolve()), "sha256": verifier.sha256_file(path)} for path in sorted(snaps.glob("*.snapshot.json"))],
        }), encoding="utf-8")
        return run, snaps, extraction

    def test_one_folded_fault_hour_passes(self):
        with tempfile.TemporaryDirectory() as td:
            run, snaps, extraction = self.fixture(Path(td))
            self.assertTrue(verifier.verify(run, snaps, extraction)["ok"])

    def test_repeated_fault_hours_fail_one_shot_proof(self):
        with tempfile.TemporaryDirectory() as td:
            run, snaps, extraction = self.fixture(Path(td), fault_hours=(1, 2))
            with self.assertRaises(verifier.VerifyError):
                verifier.verify(run, snaps, extraction)


if __name__ == "__main__":
    unittest.main()
