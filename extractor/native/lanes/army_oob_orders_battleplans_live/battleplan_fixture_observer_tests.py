from __future__ import annotations

from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))

import battleplan_fixture_observer as observer


def base_observation() -> dict:
    obs = observer.observation_template()
    obs["observations"][0].update(
        {
            "army_id": 257,
            "before": {"value": [12825, 12770]},
            "after": {"value": [12825, 12770, 2007]},
        }
    )
    obs["observations"][1].update({"army_id": 257, "before": {"value": 0}, "after": {"value": 1}})
    return obs


class BattleplanFixtureObserverTests(unittest.TestCase):
    def test_changed_observations_build_exact_fixture(self) -> None:
        fixture, errors = observer.build_fixture(base_observation(), allow_rejection=False)

        self.assertEqual(errors, [])
        self.assertIsNotNone(fixture)
        self.assertEqual(fixture["non_type5_geometry"][0]["status"], "exact")
        self.assertEqual(fixture["battleplan_execution_state"][0]["status"], "exact")

    def test_changed_observations_preserve_validated_offsets(self) -> None:
        obs = base_observation()
        obs["observations"][0]["validated_offsets"] = [
            {
                "name": "province_path",
                "offset": "+0x70",
                "type": "vec<u32>",
                "evidence": "sampled vector begin pointer",
            },
            {
                "name": "province_path_count",
                "offset": "+0x78",
                "type": "u32",
                "evidence": "sampled vector count",
            },
        ]

        fixture, errors = observer.build_fixture(obs, allow_rejection=False)

        self.assertEqual(errors, [])
        self.assertEqual(len(fixture["non_type5_geometry"][0]["validated_offsets"]), 2)
        self.assertEqual(fixture["non_type5_geometry"][0]["validated_offsets"][1]["offset"], "+0x78")

    def test_unchanged_observations_build_rejections_when_allowed(self) -> None:
        obs = base_observation()
        obs["observations"][0]["after"] = obs["observations"][0]["before"]
        obs["observations"][1]["after"] = obs["observations"][1]["before"]

        fixture, errors = observer.build_fixture(obs, allow_rejection=True)

        self.assertEqual(errors, [])
        self.assertEqual(fixture["non_type5_geometry"][0]["status"], "rejected")
        self.assertEqual(fixture["battleplan_execution_state"][0]["status"], "rejected")

    def test_unchanged_observations_fail_without_rejection_mode(self) -> None:
        obs = base_observation()
        obs["observations"][0]["after"] = obs["observations"][0]["before"]
        obs["observations"][1]["after"] = obs["observations"][1]["before"]

        _fixture, errors = observer.build_fixture(obs, allow_rejection=False)

        self.assertTrue(any("need at least one non-type-5 order" in err for err in errors))
        self.assertTrue(any("need at least one battleplan execution-state row" in err for err in errors))

    def test_passive_scan_source_fails(self) -> None:
        obs = base_observation()
        obs["observations"][0]["source_kind"] = "passive_scan"

        _fixture, errors = observer.build_fixture(obs, allow_rejection=False)

        self.assertTrue(any("passive_scan" in err for err in errors))


if __name__ == "__main__":
    unittest.main()
