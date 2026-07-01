import unittest
from pathlib import Path
import sys


LANE_DIR = Path(__file__).resolve().parent
if str(LANE_DIR) not in sys.path:
    sys.path.insert(0, str(LANE_DIR))

import army_battleplans_depth_path_geometry_from_probe as builder


def proof_base() -> dict:
    rows = [
        {
            "army_id": 10 + index,
            "slot_index": 0,
            "order_type": 2 if index % 2 == 0 else 3,
            "instance_id": 1000 + index,
            "generation": 1000 + index,
            "path_provinces": [101 + index, 201 + index],
        }
        for index in range(32)
    ]
    return {
        "schema": "army_oob_orders_battleplans_save_geometry_probe_v1",
        "exe_sha256": "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad",
        "errors": [],
        "positive_control": {
            "name": "positive_control_type5_states_plus_0xE0",
            "status": "positive_control_pass",
            "tested_rows": 100,
            "valid_rows": 90,
            "exact_rows": 80,
        },
        "candidate_results": [
            {
                "name": "non_type5_path_vector_plus_0x70",
                "status": "validated_save_paired_geometry_slice",
                "tested_rows": 40,
                "valid_rows": 38,
                "exact_rows": 32,
                "compact_rows": rows,
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


class PathGeometryFromProbeTests(unittest.TestCase):
    def test_builds_compact_stream_from_valid_rows(self) -> None:
        stream, summary = builder.build_stream_from_proof(
            proof_base(),
            source_probe=None,
            game_hour=77,
            min_path_rows=32,
        )
        self.assertEqual(summary["status"], "PASS")
        self.assertGreater(len(stream), 0)
        self.assertEqual(summary["rows"], 32)
        self.assertEqual(summary["path_geometry_rows"], 64)
        self.assertEqual(summary["non_type5_exact_path_rows"], 32)
        self.assertEqual(summary["unsupported_geometry_rows"], 0)

    def test_rejects_missing_compact_rows(self) -> None:
        proof = proof_base()
        proof["candidate_results"][0]["compact_rows"] = []
        _stream, summary = builder.build_stream_from_proof(
            proof,
            source_probe=None,
            game_hour=77,
            min_path_rows=32,
        )
        self.assertEqual(summary["status"], "FAIL")
        self.assertTrue(any("compact path rows" in error for error in summary["errors"]))


if __name__ == "__main__":
    unittest.main()
