import unittest
from pathlib import Path
import sys


LANE_DIR = Path(__file__).resolve().parent
if str(LANE_DIR) not in sys.path:
    sys.path.insert(0, str(LANE_DIR))

import direct_oob_warm_cache_gate as gate


def valid_proof() -> dict:
    counts = {
        "direct_armies": 10,
        "active_armies_from_division_parents": 9,
        "empty_or_inactive_direct_armies": 1,
        "army_groups": 3,
        "theatres": 2,
    }
    return {
        "schema": gate.EXPECTED_SCHEMA,
        "exe_sha256": gate.EXPECTED_SHA256,
        "preflight": {
            "scan_stats": {
                "accepted_hits": {"army": 10, "army_group": 3, "theatre": 2, "division": 20},
                "rejected_hits": {"army": 0, "army_group": 0, "theatre": 0, "division": 0},
            },
            "summary": {"counts": counts},
        },
        "warm_iterations": [
            {
                "iteration": 0,
                "warm_heap_sweep": False,
                "validation_mode": "cached_address_vtable_handle_id_liveness",
                "invalid_cached_root_count": 0,
                "summary": {"counts": counts},
            },
            {
                "iteration": 1,
                "warm_heap_sweep": False,
                "validation_mode": "cached_address_vtable_handle_id_liveness",
                "invalid_cached_root_count": 0,
                "summary": {"counts": counts},
            },
            {
                "iteration": 2,
                "warm_heap_sweep": False,
                "validation_mode": "cached_address_vtable_handle_id_liveness",
                "invalid_cached_root_count": 0,
                "summary": {"counts": counts},
            },
        ],
        "totals": {"warm_heap_sweeps": 0, "warm_invalid_cached_roots": 0},
    }


class DirectOobWarmCacheGateTests(unittest.TestCase):
    def test_accepts_valid_warm_cache_proof(self) -> None:
        errors = gate.validate(
            valid_proof(),
            min_iterations=3,
            min_empty_direct_armies=1,
            min_theatres=1,
        )
        self.assertEqual(errors, [])

    def test_rejects_warm_heap_sweep(self) -> None:
        proof = valid_proof()
        proof["totals"]["warm_heap_sweeps"] = 1
        proof["warm_iterations"][1]["warm_heap_sweep"] = True
        errors = gate.validate(
            proof,
            min_iterations=3,
            min_empty_direct_armies=1,
            min_theatres=1,
        )
        self.assertTrue(any("heap sweep" in err or "warm_heap_sweep" in err for err in errors))

    def test_rejects_invalid_cached_root(self) -> None:
        proof = valid_proof()
        proof["totals"]["warm_invalid_cached_roots"] = 1
        proof["warm_iterations"][2]["invalid_cached_root_count"] = 1
        errors = gate.validate(
            proof,
            min_iterations=3,
            min_empty_direct_armies=1,
            min_theatres=1,
        )
        self.assertTrue(any("invalid cached roots" in err for err in errors))

    def test_rejects_count_drift(self) -> None:
        proof = valid_proof()
        proof["warm_iterations"][0]["summary"]["counts"] = {
            **proof["warm_iterations"][0]["summary"]["counts"],
            "theatres": 1,
        }
        errors = gate.validate(
            proof,
            min_iterations=3,
            min_empty_direct_armies=1,
            min_theatres=1,
        )
        self.assertTrue(any("theatres count drifted" in err for err in errors))

    def test_accepts_exact_pure_cache_rejection(self) -> None:
        proof = valid_proof()
        proof["totals"]["warm_invalid_cached_roots"] = 3
        for row in proof["warm_iterations"]:
            row["invalid_cached_root_count"] = 1
            row["invalid_cached_roots"] = [
                {"kind": "army", "id": 10, "addr": "0x12340000", "reason": "vtable_mismatch"}
            ]
        errors = gate.validate_pure_cache_rejection(
            proof,
            min_iterations=3,
            min_empty_direct_armies=1,
            min_theatres=1,
        )
        self.assertEqual(errors, [])

    def test_rejection_requires_stale_root_evidence(self) -> None:
        errors = gate.validate_pure_cache_rejection(
            valid_proof(),
            min_iterations=3,
            min_empty_direct_armies=1,
            min_theatres=1,
        )
        self.assertTrue(any("invalid cached roots" in err or "stale-root evidence" in err for err in errors))


if __name__ == "__main__":
    unittest.main()
