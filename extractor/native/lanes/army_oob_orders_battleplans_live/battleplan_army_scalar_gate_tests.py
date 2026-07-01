import unittest
from pathlib import Path
import sys


LANE_DIR = Path(__file__).resolve().parent
if str(LANE_DIR) not in sys.path:
    sys.path.insert(0, str(LANE_DIR))

import battleplan_army_scalar_gate as gate


def offset(offset: str = "+0x118") -> dict:
    return {
        "offset": offset,
        "width": "u32",
        "tested": 64,
        "matched": 64,
        "match_fraction": 1.0,
        "candidate_distinct_values": 3,
        "value_histogram": {"0": 10, "1": 40, "2": 14},
        "mismatch_sample": [],
    }


def proof() -> dict:
    rejected = {
        "status": "REJECTED_INLINE_ARMY_SCALAR_RANGE",
        "tested_armies": 64,
        "save_value_histogram": {"0": 20, "1": 44},
        "accepted_offsets": [],
        "top_candidates": [],
    }
    return {
        "schema": gate.EXPECTED_SCHEMA,
        "exe_sha256": gate.EXPECTED_SHA256,
        "errors": [],
        "matched_armies": 64,
        "candidate_status": "VALIDATED_ARMY_SCALAR_SLICE",
        "field_results": {
            "execution_type": {
                "status": "VALIDATED_INLINE_ARMY_SCALAR",
                "tested_armies": 64,
                "save_value_histogram": {"0": 10, "1": 40, "2": 14},
                "accepted_offsets": [offset()],
                "narrowed_offsets": [offset()],
                "top_candidates": [offset()],
            },
            "cohesion_type": dict(rejected),
            "proximity_type": dict(rejected),
        },
    }


class BattleplanArmyScalarGateTests(unittest.TestCase):
    def test_accepts_validated_scalar(self) -> None:
        errors = gate.validate(
            proof(),
            required_validated=["execution_type"],
            allow_ambiguous=False,
            allow_near_match=False,
            min_matched_armies=32,
        )
        self.assertEqual(errors, [])

    def test_rejects_missing_required_validation(self) -> None:
        payload = proof()
        payload["field_results"]["execution_type"]["status"] = "REJECTED_INLINE_ARMY_SCALAR_RANGE"
        payload["field_results"]["execution_type"]["accepted_offsets"] = []
        errors = gate.validate(
            payload,
            required_validated=["execution_type"],
            allow_ambiguous=False,
            allow_near_match=False,
            min_matched_armies=32,
        )
        self.assertTrue(any("execution_type is not validated" in err for err in errors))

    def test_ambiguous_fails_without_allow_flag(self) -> None:
        payload = proof()
        payload["field_results"]["execution_type"]["status"] = "AMBIGUOUS_INLINE_ARMY_SCALAR"
        payload["field_results"]["execution_type"]["accepted_offsets"] = [offset("+0x118"), offset("+0x11C")]
        errors = gate.validate(
            payload,
            required_validated=[],
            allow_ambiguous=False,
            allow_near_match=False,
            min_matched_armies=32,
        )
        self.assertTrue(any("execution_type is ambiguous" in err for err in errors))

    def test_ambiguous_can_be_allowed(self) -> None:
        payload = proof()
        payload["candidate_status"] = "AMBIGUOUS_ARMY_SCALAR_SLICE"
        payload["field_results"]["execution_type"]["status"] = "AMBIGUOUS_INLINE_ARMY_SCALAR"
        payload["field_results"]["execution_type"]["accepted_offsets"] = [offset("+0x118"), offset("+0x11C")]
        errors = gate.validate(
            payload,
            required_validated=[],
            allow_ambiguous=True,
            allow_near_match=False,
            min_matched_armies=32,
        )
        self.assertEqual(errors, [])

    def test_near_match_requires_allow_flag(self) -> None:
        payload = proof()
        payload["candidate_status"] = "NARROWED_ARMY_SCALAR_NEAR_MATCH"
        payload["field_results"]["execution_type"]["status"] = "NARROWED_INLINE_ARMY_SCALAR_NEAR_MATCH"
        payload["field_results"]["execution_type"]["accepted_offsets"] = []
        payload["field_results"]["execution_type"]["narrowed_offsets"] = [
            {**offset("+0x1A4"), "matched": 63, "match_fraction": 63 / 64}
        ]
        errors = gate.validate(
            payload,
            required_validated=[],
            allow_ambiguous=False,
            allow_near_match=False,
            min_matched_armies=32,
        )
        self.assertTrue(any("near-match only" in err for err in errors))

        errors = gate.validate(
            payload,
            required_validated=[],
            allow_ambiguous=False,
            allow_near_match=True,
            min_matched_armies=32,
        )
        self.assertEqual(errors, [])


if __name__ == "__main__":
    unittest.main()
