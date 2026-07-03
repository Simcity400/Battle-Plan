from __future__ import annotations

import json
import unittest

from memory_reader.combat.naval_combat_live_fixture_probe import (
    encode_member_bridge_jsonl,
    fixture_contract,
    member_bridge_rows,
    summarize_member_bridge_rows,
)


class NavalCombatFixtureProbeContractTests(unittest.TestCase):
    def test_member_bridge_rows_flatten_two_sided_members(self) -> None:
        validated = [
            {
                "status": "valid",
                "combat_id": 241671,
                "body_ptr": 0x22AA0004000,
                "two_sided_members": True,
                "attacker": {
                    "groups": [
                        {
                            "group_index": 0,
                            "member_rows": [
                                {
                                    "member_index": 0,
                                    "member_ptr": 0x22AA0010000,
                                    "member_ptr_hex": "0x22AA0010000",
                                    "unit_type_tag": 51,
                                    "ship_instance_id": 3458,
                                    "is_convoy": False,
                                    "cship_status": "resolved",
                                }
                            ],
                        }
                    ]
                },
                "defender": {
                    "groups": [
                        {
                            "group_index": 0,
                            "member_rows": [
                                {
                                    "member_index": 0,
                                    "member_ptr": 0x22AA0020000,
                                    "member_ptr_hex": "0x22AA0020000",
                                    "unit_type_tag": 0,
                                    "ship_instance_id": 0,
                                    "is_convoy": True,
                                    "cship_status": "convoy_no_cship",
                                }
                            ],
                        }
                    ]
                },
            }
        ]

        rows = member_bridge_rows(validated, require_two_sided=True)
        summary = summarize_member_bridge_rows(rows)
        encoded = encode_member_bridge_jsonl(rows)

        self.assertEqual(len(rows), 2)
        self.assertEqual(rows[0]["side"], "attacker")
        self.assertEqual(rows[0]["ship_instance_id"], 3458)
        self.assertEqual(rows[1]["side"], "defender")
        self.assertEqual(summary["member_bridge_row_count"], 2)
        self.assertEqual(summary["non_convoy_member_count"], 1)
        self.assertEqual(summary["cship_resolved_member_count"], 1)
        self.assertEqual(summary["cship_unresolved_member_count"], 0)
        self.assertEqual(len([json.loads(line) for line in encoded.splitlines()]), 2)

    def test_member_bridge_rows_respect_two_sided_requirement(self) -> None:
        rows = member_bridge_rows(
            [
                {
                    "status": "valid",
                    "combat_id": 7,
                    "body_ptr": 0x22AA0004000,
                    "two_sided_members": False,
                    "attacker": {"groups": [{"group_index": 0, "member_rows": [{"member_index": 0}]}]},
                    "defender": {"groups": []},
                }
            ],
            require_two_sided=True,
        )

        self.assertEqual(rows, [])

    def test_fixture_contract_requires_cship_join(self) -> None:
        criteria = " ".join(fixture_contract()["success_criteria"])

        self.assertIn("member_bridge_row_count", criteria)
        self.assertIn("cship_unresolved_member_count == 0", criteria)
        self.assertIn("CShip module+0x2941748", criteria)

    def test_fixture_contract_promotes_location_and_pins_backref_walk(self) -> None:
        contract = fixture_contract()
        criteria = " ".join(contract["success_criteria"])

        self.assertIn("location_front_0xA4", criteria)
        self.assertIn("PROMOTED", criteria)
        self.assertIn("D04 save oracle", criteria)
        self.assertNotIn("location_candidate_front_0xA4", criteria)
        self.assertIn("group+0x38 owning side", criteria)
        self.assertIn("vtable_rtti.ok == true", criteria)
        rejections = " ".join(contract["rejection_criteria"])
        self.assertIn("vtable_rtti_mismatch", rejections)
        self.assertIn("back-ref buckets", rejections)


if __name__ == "__main__":
    unittest.main()
