import unittest
from pathlib import Path
import sys


LANE_DIR = Path(__file__).resolve().parent
if str(LANE_DIR) not in sys.path:
    sys.path.insert(0, str(LANE_DIR))

import direct_oob_table_probe_gate as gate


def proof_base() -> dict:
    counts = {
        "direct_armies": 10,
        "active_armies_from_division_parents": 9,
        "empty_or_inactive_direct_armies": 1,
        "army_groups": 4,
        "theatres": 2,
    }
    return {
        "schema": gate.EXPECTED_SCHEMA,
        "exe_sha256": gate.EXPECTED_SHA256,
        "preflight": {
            "scan_stats": {
                "accepted_hits": {"army": 10, "army_group": 4, "theatre": 2, "division": 25},
            },
            "summary": {"counts": counts},
        },
        "table_scan": {
            "bytes_scanned": 1024,
            "total_reference_hits": 6,
            "hit_cap_exceeded": False,
            "membership_false_positive_hits": 0,
            "search_strides": [8, 16, 24, 32],
            "target_root_counts": {"army": 10, "army_group": 4, "theatre": 2},
            "thresholds": {"min_kind_coverage": 0.5, "min_total_direct_coverage": 0.5},
            "accepted_table_candidates": [],
            "run_union_coverage_by_kind": {
                "army": {"covered": 2, "total": 10, "coverage": 0.2},
                "army_group": {"covered": 1, "total": 4, "coverage": 0.25},
                "theatre": {"covered": 0, "total": 2, "coverage": 0.0},
            },
            "special_root_coverage": {
                "empty_or_inactive_armies": {
                    "kind": "army",
                    "total": 1,
                    "referenced_anywhere": 1,
                    "referenced_anywhere_ids": [10],
                    "covered_by_any_table_run": 1,
                    "covered_by_any_table_run_ids": [10],
                    "covered_by_accepted_table_run": 0,
                    "covered_by_accepted_table_run_ids": [],
                    "best_single_run": {
                        "start_ref_addr": "0x2000",
                        "stride": 8,
                        "length": 3,
                        "matched": 1,
                        "total": 1,
                        "coverage": 1.0,
                        "ids": [10],
                    },
                }
            },
            "max_coverage": {
                "total_direct_coverage": 0.20,
                "by_kind": {"army": 0.20, "army_group": 0.25, "theatre": 0.0},
            },
        },
        "candidate_status": "REJECTED_PLAIN_DIRECT_ROOT_POINTER_TABLE",
        "rejected_candidate_family": {
            "candidate_family": "plain_contiguous_or_fixed_stride_direct_root_pointer_table",
            "thresholds": {"min_kind_coverage": 0.5, "min_total_direct_coverage": 0.5},
        },
    }


class DirectOobTableProbeGateTests(unittest.TestCase):
    def test_accepts_plain_table_rejection(self) -> None:
        errors = gate.validate(
            proof_base(),
            min_empty_direct_armies=1,
            min_theatres=1,
            min_reference_hits=1,
            min_kind_coverage=0.5,
            min_total_direct_coverage=0.5,
            require_empty_army_table_resolution=False,
        )
        self.assertEqual(errors, [])

    def test_rejection_fails_when_kind_coverage_meets_threshold(self) -> None:
        proof = proof_base()
        proof["table_scan"]["max_coverage"]["by_kind"]["army"] = 0.5
        errors = gate.validate(
            proof,
            min_empty_direct_armies=1,
            min_theatres=1,
            min_reference_hits=1,
            min_kind_coverage=0.5,
            min_total_direct_coverage=0.5,
            require_empty_army_table_resolution=False,
        )
        self.assertTrue(any("army coverage meets table threshold" in err for err in errors))

    def test_rejects_hit_cap_exceeded(self) -> None:
        proof = proof_base()
        proof["table_scan"]["hit_cap_exceeded"] = True
        errors = gate.validate(
            proof,
            min_empty_direct_armies=1,
            min_theatres=1,
            min_reference_hits=1,
            min_kind_coverage=0.5,
            min_total_direct_coverage=0.5,
            require_empty_army_table_resolution=False,
        )
        self.assertTrue(any("hit cap" in err for err in errors))

    def test_accepts_validated_table_candidate(self) -> None:
        proof = proof_base()
        proof["candidate_status"] = "VALIDATED_DIRECT_ROOT_POINTER_TABLE"
        proof["rejected_candidate_family"] = None
        proof["table_scan"]["accepted_table_candidates"] = [
            {
                "stride": 8,
                "total_direct_coverage": 0.75,
                "coverage_by_kind": {"army": 0.7, "army_group": 1.0, "theatre": 0.0},
            }
        ]
        errors = gate.validate(
            proof,
            min_empty_direct_armies=1,
            min_theatres=1,
            min_reference_hits=1,
            min_kind_coverage=0.5,
            min_total_direct_coverage=0.5,
            require_empty_army_table_resolution=False,
        )
        self.assertEqual(errors, [])

    def test_accepts_partial_validated_table_candidate(self) -> None:
        proof = proof_base()
        proof["candidate_status"] = "VALIDATED_PARTIAL_DIRECT_ROOT_POINTER_TABLE"
        proof["rejected_candidate_family"] = None
        proof["table_scan"]["accepted_table_candidates"] = [
            {
                "stride": 8,
                "total_direct_coverage": 0.20,
                "coverage_by_kind": {"army": 0.0, "army_group": 0.0, "theatre": 0.75},
            }
        ]
        errors = gate.validate(
            proof,
            min_empty_direct_armies=1,
            min_theatres=1,
            min_reference_hits=1,
            min_kind_coverage=0.5,
            min_total_direct_coverage=0.5,
            require_empty_army_table_resolution=False,
        )
        self.assertEqual(errors, [])

    def test_strict_empty_army_resolution_requires_accepted_run(self) -> None:
        proof = proof_base()
        errors = gate.validate(
            proof,
            min_empty_direct_armies=1,
            min_theatres=1,
            min_reference_hits=1,
            min_kind_coverage=0.5,
            min_total_direct_coverage=0.5,
            require_empty_army_table_resolution=True,
        )
        self.assertTrue(any("empty/inactive armies" in err for err in errors))

    def test_validated_table_requires_coverage(self) -> None:
        proof = proof_base()
        proof["candidate_status"] = "VALIDATED_DIRECT_ROOT_POINTER_TABLE"
        proof["rejected_candidate_family"] = None
        proof["table_scan"]["accepted_table_candidates"] = [
            {
                "stride": 8,
                "total_direct_coverage": 0.25,
                "coverage_by_kind": {"army": 0.2, "army_group": 0.25, "theatre": 0.0},
            }
        ]
        errors = gate.validate(
            proof,
            min_empty_direct_armies=1,
            min_theatres=1,
            min_reference_hits=1,
            min_kind_coverage=0.5,
            min_total_direct_coverage=0.5,
            require_empty_army_table_resolution=False,
        )
        self.assertTrue(any("coverage thresholds" in err for err in errors))


if __name__ == "__main__":
    unittest.main()
