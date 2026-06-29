import unittest

from live_capture.collector_proof_gate import (
    collector_session_proof_errors,
    collector_session_proof_summary,
)


def _manifest(**overrides):
    manifest = {
        "session_errors": [],
        "replay_verified": True,
        "game_hours_advanced": 12,
        "stored_snapshot_count": 13,
        "hourly_requirement": {
            "met": True,
            "elapsed_wall_s": 11.5,
            "problems": [],
        },
        "collector_frame_payload_summary": {
            "domains_seen": [
                "clock",
                "country_identity",
                "country_scalars",
                "fuel_live",
                "production_live",
                "state_control",
            ],
            "raw_delta_domains_seen": ["clock", "divisions"],
            "heavy_payload_on_lightweight_frames": 0,
        },
    }
    manifest.update(overrides)
    return manifest


class TestCollectorProofGate(unittest.TestCase):
    def test_accepts_speed_five_session_with_required_domains(self):
        proof = collector_session_proof_summary(
            _manifest(),
            required_snapshot_domains=["fuel_live", "state_control"],
            required_raw_delta_domains=["divisions"],
            min_game_hours=12,
        )

        self.assertTrue(proof["ok"])
        self.assertEqual(proof["errors"], [])
        self.assertEqual(proof["hourly_elapsed_wall_s"], 11.5)

    def test_reports_session_hourly_replay_and_domain_failures(self):
        errors = collector_session_proof_errors(
            _manifest(
                session_errors=["attach_errors: pipe closed"],
                replay_verified=False,
                game_hours_advanced=3,
                hourly_requirement={
                    "met": False,
                    "problems": ["skipped_game_hours"],
                },
                collector_frame_payload_summary={
                    "domains_seen": ["clock"],
                    "raw_delta_domains_seen": [],
                    "heavy_payload_on_lightweight_frames": 2,
                },
            ),
            required_snapshot_domains=["production_live"],
            required_raw_delta_domains=["land_combats"],
            min_game_hours=12,
        )

        joined = "\n".join(errors)
        self.assertIn("session_errors present", joined)
        self.assertIn("replay_verified is not true", joined)
        self.assertIn("game_hours_advanced 3 below required 12", joined)
        self.assertIn("hourly_requirement not met", joined)
        self.assertIn("snapshot domain 'production_live' not observed", joined)
        self.assertIn("raw delta domain 'land_combats' not observed", joined)
        self.assertIn("heavy payloads appeared on lightweight frames: 2", joined)

    def test_missing_summary_fails_required_domains(self):
        errors = collector_session_proof_errors(
            _manifest(collector_frame_payload_summary=None),
            required_snapshot_domains=["country_scalars"],
            required_raw_delta_domains=["divisions"],
        )

        self.assertEqual(
            errors,
            [
                "snapshot domain 'country_scalars' not observed",
                "raw delta domain 'divisions' not observed",
            ],
        )


if __name__ == "__main__":
    unittest.main()
