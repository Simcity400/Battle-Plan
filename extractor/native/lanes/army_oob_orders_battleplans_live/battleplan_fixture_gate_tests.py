from __future__ import annotations

from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))

import battleplan_fixture_gate as gate


def base_fixture() -> dict:
    return {
        "schema": "army_oob_orders_battleplans_fixture_v1",
        "exe_sha256": gate.EXPECTED_SHA256,
        "capture": {
            "dll_only": True,
            "compact_stream": True,
            "rust_decoded": True,
            "hoi4replay_verified": True,
            "zero_skipped_hours": True,
            "no_failures": True,
            "source_unavailable": False,
            "partial_data": False,
            "bridge_unavailable": False,
            "worker_panic": False,
            "game_hours_advanced": 2,
            "max_frame_hour_gap": 1,
        },
    }


def exact_geometry_row() -> dict:
    return {
        "kind": "geometry",
        "status": "exact",
        "army_id": 257,
        "slot_index": 0,
        "order_type": 2,
        "order_generation": 1,
        "validated_offsets": [
            {
                "name": "province_path",
                "offset": "+0x70",
                "type": "vec<u32>",
                "evidence": "fixture geometry mutation changed this vector",
            }
        ],
        "field_values": {"province_path": [12825, 12770, 2007]},
    }


def exact_execution_row() -> dict:
    return {
        "kind": "execution",
        "status": "exact",
        "army_id": 257,
        "slot_index": 0,
        "order_type": 2,
        "order_generation": 1,
        "validated_offsets": [
            {
                "name": "is_executing",
                "offset": "+0x150",
                "type": "u8",
                "evidence": "fixture execution toggle changed this field",
            }
        ],
        "field_values": {"is_executing": True},
    }


def rejected_row(kind: str) -> dict:
    row = {
        "kind": kind,
        "status": "rejected",
        "army_id": 257,
        "slot_index": 0,
        "order_type": 2,
        "order_generation": 1,
        "rejection_evidence": {
            "candidate": "order+0x70" if kind == "geometry" else "order+0x150",
            "reason": "mutation_mismatch" if kind == "geometry" else "not_execution_state",
            "evidence": ["fixture changed the target state, candidate did not match"],
        },
    }
    return row


class BattleplanFixtureGateTests(unittest.TestCase):
    def test_exact_fixture_passes(self) -> None:
        fixture = base_fixture()
        fixture["non_type5_geometry"] = [exact_geometry_row()]
        fixture["battleplan_execution_state"] = [exact_execution_row()]

        self.assertEqual(gate.validate_fixture(fixture, allow_rejection=False), [])

    def test_rejection_requires_allow_rejection(self) -> None:
        fixture = base_fixture()
        fixture["non_type5_geometry"] = [rejected_row("geometry")]
        fixture["battleplan_execution_state"] = [rejected_row("execution")]

        self.assertTrue(gate.validate_fixture(fixture, allow_rejection=False))
        self.assertEqual(gate.validate_fixture(fixture, allow_rejection=True), [])

    def test_passive_candidate_rows_do_not_pass(self) -> None:
        fixture = base_fixture()
        fixture["non_type5_geometry"] = [
            {
                "kind": "geometry",
                "status": "candidate",
                "army_id": 257,
                "slot_index": 0,
                "order_type": 2,
                "order_generation": 1,
                "passive_scan": {
                    "offset": "+0x70/+0x7C",
                    "status": "plausible",
                    "sample": [12825, 12770, 2007],
                },
            }
        ]
        fixture["battleplan_execution_state"] = []

        errors = gate.validate_fixture(fixture, allow_rejection=True)

        self.assertTrue(any("passive candidate rows are not accepted" in err for err in errors))
        self.assertTrue(any("need at least one non-type-5 order" in err for err in errors))

    def test_cached_plan_values_do_not_pass_without_live_offsets(self) -> None:
        fixture = base_fixture()
        fixture["non_type5_geometry"] = [exact_geometry_row()]
        fixture["battleplan_execution_state"] = [
            {
                "kind": "execution",
                "status": "exact",
                "army_id": 257,
                "slot_index": 0,
                "order_type": 2,
                "order_generation": 1,
                "field_values": {"plan_value": 0.42},
            }
        ]

        errors = gate.validate_fixture(fixture, allow_rejection=False)

        self.assertTrue(any("validated_offsets" in err for err in errors))


if __name__ == "__main__":
    unittest.main()
