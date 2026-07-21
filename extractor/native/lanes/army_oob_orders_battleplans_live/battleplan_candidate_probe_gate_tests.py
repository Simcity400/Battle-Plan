from __future__ import annotations

from pathlib import Path
import sys
import unittest

HERE = Path(__file__).resolve().parent
if str(HERE) not in sys.path:
    sys.path.insert(0, str(HERE))

from battleplan_candidate_probe_gate import EXPECTED_SHA256, validate_report


def valid_report() -> dict:
    return {
        "schema": "army_oob_orders_battleplans_candidate_probe_v1",
        "exe_sha256": EXPECTED_SHA256,
        "errors": [],
        "candidate_results": [
            {
                "name": "positive_control_type5_states_plus_0xE0",
                "status": "positive_control_pass",
                "tested_rows": 10,
                "valid_rows": 9,
                "evidence": ["type-5 positive control decoded"],
            },
            {
                "name": "template_is_executing_u8_plus_0x150",
                "status": "rejected",
                "tested_rows": 10,
                "valid_rows": 10,
                "evidence": ["non-boolean values"],
            },
            {
                "name": "type1_type2_type3_path_vector_plus_0x70",
                "status": "fixture_required",
                "tested_rows": 10,
                "valid_rows": 9,
                "evidence": ["vector-shaped only"],
            },
        ],
    }


class CandidateProbeGateTests(unittest.TestCase):
    def test_accepts_required_rejection_and_fixture_blocker(self) -> None:
        errors = validate_report(
            valid_report(),
            required_rejected=["template_is_executing_u8_plus_0x150"],
            required_fixture_required=["type1_type2_type3_path_vector_plus_0x70"],
        )
        self.assertEqual(errors, [])

    def test_rejects_failed_positive_control(self) -> None:
        report = valid_report()
        report["candidate_results"][0]["status"] = "positive_control_failed"
        errors = validate_report(report, required_rejected=[], required_fixture_required=[])
        self.assertTrue(any("positive control" in error for error in errors))

    def test_rejects_missing_required_candidate(self) -> None:
        errors = validate_report(valid_report(), required_rejected=["missing"], required_fixture_required=[])
        self.assertTrue(any("missing required rejected candidate" in error for error in errors))


if __name__ == "__main__":
    unittest.main()
