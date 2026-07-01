from __future__ import annotations

from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))

import battleplan_fixture_gate as fixture_gate
import battleplan_integration_guard as guard


def exact_fixture() -> dict:
    return {
        "schema": "army_oob_orders_battleplans_fixture_v1",
        "exe_sha256": fixture_gate.EXPECTED_SHA256,
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
        "non_type5_geometry": [
            {
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
                        "evidence": "fixture geometry mutation matched",
                    }
                ],
                "field_values": {"province_path": [12825, 12770, 2007]},
            }
        ],
        "battleplan_execution_state": [
            {
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
                        "evidence": "fixture execution toggle matched",
                    }
                ],
                "field_values": {"is_executing": True},
            }
        ],
    }


def valid_proposal() -> dict:
    return {
        "schema": "army_oob_orders_battleplans_integration_guard_v1",
        "shared_entrypoint_slot_required": True,
        "touches_shared_entrypoints_now": False,
        "fixture_json": "unused_by_unit_test.json",
        "domain_changes": [
            {
                "domain_name": "army_orders",
                "domain_id": 27,
                "mode": "unchanged",
                "row_width_change": False,
                "version_bump": False,
            },
            {
                "domain_name": "oob_command",
                "domain_id": 28,
                "mode": "unchanged",
                "row_width_change": False,
                "version_bump": False,
            },
            {
                "domain_name": "army_battleplans_depth_live",
                "domain_id": 54,
                "mode": "additive",
                "row_width_change": False,
            },
        ],
        "candidate_sources": [
            {
                "source_kind": "live_fixture",
                "status": "exact",
                "requires_fixture": True,
                "owner_lane": "army_oob_orders_battleplans_live",
            }
        ],
    }


class BattleplanIntegrationGuardTests(unittest.TestCase):
    def test_rejects_protected_domain_row_width_change(self) -> None:
        proposal = valid_proposal()
        proposal["domain_changes"][0]["row_width_change"] = True

        errors = guard.validate_proposal_with_fixture(proposal, exact_fixture(), allow_rejection=False)

        self.assertTrue(any("must not change row width" in err for err in errors))

    def test_rejects_passive_candidate_sources(self) -> None:
        proposal = valid_proposal()
        proposal["candidate_sources"] = [
            {
                "source_kind": "passive_scan",
                "status": "candidate",
                "requires_fixture": False,
                "owner_lane": "army_oob_orders_battleplans_live",
            }
        ]

        errors = guard.validate_proposal_with_fixture(proposal, exact_fixture(), allow_rejection=False)

        self.assertTrue(any("passive_scan" in err for err in errors))
        self.assertTrue(any("status candidate" in err for err in errors))
        self.assertTrue(any("requires_fixture" in err for err in errors))

    def test_rejects_save_cached_execution_sources(self) -> None:
        proposal = valid_proposal()
        proposal["candidate_sources"] = [
            {
                "source_kind": "save_cached_plan_value",
                "status": "exact",
                "requires_fixture": True,
                "owner_lane": "army_oob_orders_battleplans_live",
            }
        ]

        errors = guard.validate_proposal_with_fixture(proposal, exact_fixture(), allow_rejection=False)

        self.assertTrue(any("save_cached_plan_value" in err for err in errors))

    def test_accepts_additive_exact_fixture_plan(self) -> None:
        errors = guard.validate_proposal_with_fixture(valid_proposal(), exact_fixture(), allow_rejection=False)

        self.assertEqual(errors, [])


if __name__ == "__main__":
    unittest.main()
