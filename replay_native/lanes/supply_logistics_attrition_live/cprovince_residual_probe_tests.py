import unittest
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import cprovince_residual_probe as probe


class CProvinceResidualProbeTests(unittest.TestCase):
    def test_classifies_bounded_varied_i32_as_anchor_required(self):
        stats = probe._classify_i32_series([100_000, 250_000, 500_000, 750_000, 900_000])
        self.assertEqual(stats["verdict"], "i32_fixed_1e5_candidate_requires_anchor")
        self.assertEqual(stats["distinct"], 5)
        self.assertEqual(stats["negative"], 0)

    def test_rejects_negative_and_overrange_series(self):
        negative = probe._classify_i32_series([100_000, -1, 200_000, 300_000, 400_000])
        self.assertEqual(negative["verdict"], "rejected_negative_values")
        overrange = probe._classify_i32_series([100_000, 200_000, probe.MAX_SUPPLY_RAW + 1])
        self.assertEqual(overrange["verdict"], "rejected_gt_max_supply_raw")

    def test_anchor_match_requires_multiple_clean_remaining_matches(self):
        anchors = [
            {"province_id": 10, "remaining": 1.0, "hubs": 0.5, "state": 0.25, "sum": 0.75},
            {"province_id": 20, "remaining": 2.0, "hubs": 1.5, "state": 0.25, "sum": 1.75},
            {"province_id": 30, "remaining": 3.0, "hubs": 2.5, "state": 0.25, "sum": 2.75},
        ]
        values = {10: 100_000, 20: 200_000, 30: 300_000}
        result = probe._anchor_match_summary(anchors, values, tolerance=0.055)
        self.assertEqual(result["remaining_matches"], 3)
        self.assertEqual(result["other_component_matches"], 0)
        self.assertEqual(result["verdict"], "anchor_value_match_requires_temporal_differential")

    def test_anchor_match_rejects_sibling_component_aliases(self):
        anchors = [
            {"province_id": 10, "remaining": 1.0, "hubs": 0.5},
            {"province_id": 20, "remaining": 2.0, "hubs": 1.5},
            {"province_id": 30, "remaining": 3.0, "hubs": 2.5},
        ]
        values = {10: 50_000, 20: 150_000, 30: 250_000}
        result = probe._anchor_match_summary(anchors, values, tolerance=0.055)
        self.assertEqual(result["remaining_matches"], 0)
        self.assertEqual(result["other_component_matches"], 3)
        self.assertEqual(result["verdict"], "anchor_rejected")


if __name__ == "__main__":
    unittest.main()
