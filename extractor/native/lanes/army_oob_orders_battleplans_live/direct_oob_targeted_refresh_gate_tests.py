import unittest
from pathlib import Path
import sys


LANE_DIR = Path(__file__).resolve().parent
if str(LANE_DIR) not in sys.path:
    sys.path.insert(0, str(LANE_DIR))

import direct_oob_targeted_refresh_gate as gate


def proof_base(candidate_status: str = "REJECTED_TARGETED_HANDLE_REFRESH_ON_MISMATCH") -> dict:
    counts = {
        "direct_armies": 10,
        "active_armies_from_division_parents": 9,
        "empty_or_inactive_direct_armies": 1,
        "army_groups": 3,
        "theatres": 2,
    }
    rejection = None
    if candidate_status == "REJECTED_TARGETED_HANDLE_REFRESH_ON_MISMATCH":
        rejection = {
            "candidate_family": "cached_direct_roots_plus_targeted_handle_scan_refresh_on_mismatch",
            "reasons": ["one or more stale cached roots were not found by targeted handle refresh"],
            "max_bounded_scan_bytes": 512 << 20,
        }
    return {
        "schema": gate.EXPECTED_SCHEMA,
        "exe_sha256": gate.EXPECTED_SHA256,
        "preflight": {
            "summary": {"counts": counts},
        },
        "warm_iterations": [
            {
                "iteration": 0,
                "validation_mode": "cached_address_then_targeted_handle_refresh",
                "targeted_handle_refresh": True,
                "invalid_cached_root_count": 2,
                "refresh": {
                    "target_count": 2,
                    "bytes_scanned": 4096,
                    "unresolved_count": 1,
                    "membership_false_positive_hits": 0,
                },
                "summary": {"counts": {**counts, "direct_armies": 9}},
            },
            {
                "iteration": 1,
                "validation_mode": "cached_address_then_targeted_handle_refresh",
                "targeted_handle_refresh": True,
                "invalid_cached_root_count": 2,
                "refresh": {
                    "target_count": 2,
                    "bytes_scanned": 4096,
                    "unresolved_count": 1,
                    "membership_false_positive_hits": 0,
                },
                "summary": {"counts": {**counts, "direct_armies": 9}},
            },
        ],
        "totals": {
            "warm_iterations": 2,
            "warm_invalid_cached_roots": 4,
            "targeted_refresh_scans": 2,
            "targeted_refresh_bytes_scanned": 8192,
            "targeted_refresh_unresolved_roots": 2,
            "warm_heap_sweeps": 0,
        },
        "candidate_status": candidate_status,
        "rejected_candidate_family": rejection,
    }


class DirectOobTargetedRefreshGateTests(unittest.TestCase):
    def test_accepts_exact_refresh_rejection(self) -> None:
        errors = gate.validate(
            proof_base(),
            min_iterations=2,
            min_empty_direct_armies=1,
            min_theatres=1,
            max_bounded_scan_bytes=512 << 20,
            allow_refresh_rejection=True,
        )
        self.assertEqual(errors, [])

    def test_accepts_refresh_rejection_without_current_empty_armies_when_not_required(self) -> None:
        proof = proof_base()
        counts = proof["preflight"]["summary"]["counts"]
        counts["active_armies_from_division_parents"] = counts["direct_armies"]
        counts["empty_or_inactive_direct_armies"] = 0
        errors = gate.validate(
            proof,
            min_iterations=2,
            min_empty_direct_armies=0,
            min_theatres=1,
            max_bounded_scan_bytes=512 << 20,
            allow_refresh_rejection=True,
        )
        self.assertEqual(errors, [])

    def test_rejection_requires_allow_flag(self) -> None:
        errors = gate.validate(
            proof_base(),
            min_iterations=2,
            min_empty_direct_armies=1,
            min_theatres=1,
            max_bounded_scan_bytes=512 << 20,
            allow_refresh_rejection=False,
        )
        self.assertTrue(any("allow-refresh-rejection" in err for err in errors))

    def test_rejection_requires_unresolved_or_unbounded(self) -> None:
        proof = proof_base()
        for row in proof["warm_iterations"]:
            row["refresh"]["unresolved_count"] = 0
            row["refresh"]["bytes_scanned"] = 4096
        errors = gate.validate(
            proof,
            min_iterations=2,
            min_empty_direct_armies=1,
            min_theatres=1,
            max_bounded_scan_bytes=512 << 20,
            allow_refresh_rejection=True,
        )
        self.assertTrue(any("unresolved roots or unbounded scan cost" in err for err in errors))

    def test_accepts_strict_validated_refresh(self) -> None:
        proof = proof_base("VALIDATED_BOUNDED_TARGETED_HANDLE_REFRESH")
        counts = proof["preflight"]["summary"]["counts"]
        for row in proof["warm_iterations"]:
            row["refresh"]["unresolved_count"] = 0
            row["summary"]["counts"] = counts
        proof["totals"]["targeted_refresh_unresolved_roots"] = 0
        errors = gate.validate(
            proof,
            min_iterations=2,
            min_empty_direct_armies=1,
            min_theatres=1,
            max_bounded_scan_bytes=512 << 20,
            allow_refresh_rejection=False,
        )
        self.assertEqual(errors, [])

    def test_strict_validated_refresh_rejects_count_drift(self) -> None:
        proof = proof_base("VALIDATED_BOUNDED_TARGETED_HANDLE_REFRESH")
        for row in proof["warm_iterations"]:
            row["refresh"]["unresolved_count"] = 0
        errors = gate.validate(
            proof,
            min_iterations=2,
            min_empty_direct_armies=1,
            min_theatres=1,
            max_bounded_scan_bytes=512 << 20,
            allow_refresh_rejection=False,
        )
        self.assertTrue(any("direct_armies count drifted" in err for err in errors))


if __name__ == "__main__":
    unittest.main()
