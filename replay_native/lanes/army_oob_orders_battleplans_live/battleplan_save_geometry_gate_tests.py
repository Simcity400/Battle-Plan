import unittest
from pathlib import Path
import sys


LANE_DIR = Path(__file__).resolve().parent
if str(LANE_DIR) not in sys.path:
    sys.path.insert(0, str(LANE_DIR))

import battleplan_save_geometry_gate as gate


def proof_base() -> dict:
    return {
        "schema": gate.EXPECTED_SCHEMA,
        "exe_sha256": gate.EXPECTED_SHA256,
        "errors": [],
        "positive_control": {
            "name": "positive_control_type5_states_plus_0xE0",
            "status": "positive_control_pass",
            "tested_rows": 100,
            "valid_rows": 95,
            "exact_rows": 80,
        },
        "candidate_results": [
            {
                "name": "non_type5_path_vector_plus_0x70",
                "status": "validated_save_paired_geometry_slice",
                "tested_rows": 100,
                "valid_rows": 90,
                "exact_rows": 50,
            },
            {
                "name": "type3_secondary_vector_plus_0xC0",
                "status": "rejected_save_paired_geometry_candidate",
                "tested_rows": 12,
                "valid_rows": 12,
                "exact_rows": 0,
            },
        ],
    }


class BattleplanSaveGeometryGateTests(unittest.TestCase):
    def test_accepts_validated_path70_and_rejected_c0(self) -> None:
        errors = gate.validate(
            proof_base(),
            min_type5_exact=64,
            min_path70_exact=32,
            require_type3_c0_rejected=True,
        )
        self.assertEqual(errors, [])

    def test_requires_type5_control(self) -> None:
        proof = proof_base()
        proof["positive_control"]["exact_rows"] = 10
        errors = gate.validate(
            proof,
            min_type5_exact=64,
            min_path70_exact=32,
            require_type3_c0_rejected=True,
        )
        self.assertTrue(any("type-5" in err for err in errors))

    def test_requires_path70_exact_rows(self) -> None:
        proof = proof_base()
        proof["candidate_results"][0]["exact_rows"] = 2
        errors = gate.validate(
            proof,
            min_type5_exact=64,
            min_path70_exact=32,
            require_type3_c0_rejected=True,
        )
        self.assertTrue(any("+0x70" in err for err in errors))

    def test_requires_c0_rejection_when_requested(self) -> None:
        proof = proof_base()
        proof["candidate_results"][1]["status"] = "validated_save_paired_geometry_slice"
        proof["candidate_results"][1]["exact_rows"] = 3
        errors = gate.validate(
            proof,
            min_type5_exact=64,
            min_path70_exact=32,
            require_type3_c0_rejected=True,
        )
        self.assertTrue(any("+0xC0" in err for err in errors))


if __name__ == "__main__":
    unittest.main()
