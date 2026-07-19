import json
import tempfile
import unittest
from pathlib import Path

import sys

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))

import verify_acceptance_fixture_variance as verifier


def snapshot(*, ratio, mission, combats, phenomenon):
    return {
        "diplomacy_world": {
            "domain53_extra": {
                "subject_autonomy_rows": [
                    {
                        "overlord_tag_db_id": 1,
                        "subject_tag_db_id": 2,
                        "autonomy_state_hash": 100,
                        "autonomy_ratio_raw": ratio,
                    }
                ]
            }
        },
        "order_of_battle": {
            "extra": {
                "naval_missions_status_live": {
                    "task_forces": [
                        {
                            "task_force_id": 77,
                            "mission_enum": mission,
                            "mission_name": verifier.MISSION_NAMES[mission],
                        }
                    ]
                }
            }
        },
        "naval_combats": combats,
        "map": {
            "_r2_weather_live": {
                "regions": {
                    "12": {
                        "temperature": 20.0,
                        "rain_light": False,
                        "rain_heavy": False,
                        "snow": False,
                        "blizzard": False,
                        "arctic_water": False,
                        "active_signal": False,
                        "phenomenon_active": phenomenon,
                        "sandstorm": False,
                    }
                }
            }
        },
        "provenance": {"collector": {"domain_problems": []}},
    }


class VerifyAcceptanceFixtureVarianceTests(unittest.TestCase):
    def write(self, root: Path, hour: int, value):
        (root / f"{hour:012}.snapshot.json").write_text(
            json.dumps(value), encoding="utf-8"
        )

    def test_all_four_checks_pass_on_exact_transitions(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            self.write(root, 10, snapshot(ratio=20000, mission=1, combats={"900:1": {}}, phenomenon=False))
            self.write(root, 11, snapshot(ratio=30000, mission=1, combats={}, phenomenon=True))
            report = verifier.verify(root)
            self.assertTrue(report["ok"])
            self.assertTrue(all(row["passed"] for row in report["checks"].values()))

    def test_stable_fixture_fails_variance_checks(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            value = snapshot(ratio=20000, mission=1, combats={"900:1": {}}, phenomenon=False)
            self.write(root, 10, value)
            self.write(root, 11, value)
            report = verifier.verify(root)
            self.assertFalse(report["ok"])
            self.assertFalse(report["checks"]["A16_subject_autonomy_change"]["passed"])
            self.assertFalse(report["checks"]["A9_naval_combat_lifecycle_end"]["passed"])
            self.assertFalse(report["checks"]["A20_weather_drift"]["passed"])
            self.assertTrue(report["checks"]["A8_naval_mission_semantics"]["passed"])

    def test_bad_mission_mapping_fails_closed(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            first = snapshot(ratio=20000, mission=1, combats={"900:1": {}}, phenomenon=False)
            first["order_of_battle"]["extra"]["naval_missions_status_live"]["task_forces"][0]["mission_name"] = "PATROL-ish"
            self.write(root, 10, first)
            self.write(root, 11, snapshot(ratio=30000, mission=1, combats={}, phenomenon=True))
            with self.assertRaises(verifier.VerifyError):
                verifier.verify(root)

    def test_failed_domain_hour_cannot_prove_lifecycle_end(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            first = snapshot(ratio=20000, mission=1, combats={"900:1": {}}, phenomenon=False)
            second = snapshot(ratio=30000, mission=1, combats={}, phenomenon=True)
            second["provenance"]["collector"]["domain_problems"] = [
                {"domain_id": 49, "problem": "compact_status_failure"}
            ]
            self.write(root, 10, first)
            self.write(root, 11, second)
            report = verifier.verify(root)
            self.assertFalse(report["checks"]["A9_naval_combat_lifecycle_end"]["passed"])

    def test_missing_domain_provenance_fails_closed(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            first = snapshot(ratio=20000, mission=1, combats={"900:1": {}}, phenomenon=False)
            del first["provenance"]
            self.write(root, 10, first)
            self.write(root, 11, snapshot(ratio=30000, mission=1, combats={}, phenomenon=True))
            with self.assertRaises(verifier.VerifyError):
                verifier.verify(root)

    def test_noncontiguous_snapshot_subset_fails_closed(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            self.write(root, 10, snapshot(ratio=20000, mission=1, combats={"900:1": {}}, phenomenon=False))
            self.write(root, 12, snapshot(ratio=30000, mission=1, combats={}, phenomenon=True))
            with self.assertRaises(verifier.VerifyError):
                verifier.verify(root)


if __name__ == "__main__":
    unittest.main()
