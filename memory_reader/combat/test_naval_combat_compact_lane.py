from __future__ import annotations

import tempfile
import unittest
from pathlib import Path
import json

from memory_reader.combat.naval_combat_compact_lane import (
    MAGIC,
    NavalCombatLifecycleRow,
    NavalCombatMemberRow,
    encode_records,
    gate_stream,
    parse_bridge_rows,
    proof_from_bridge_text,
)
from memory_reader.combat.naval_combat_compact_from_raw import convert_raw_jsonl_to_compact


class NavalCombatCompactLaneTests(unittest.TestCase):
    def test_two_sided_synthetic_stream_passes_gate(self) -> None:
        stream = encode_records(
            sequence_start=7,
            game_hour_raw=128,
            elapsed_wall_ms=2_500,
            combat_rows=[
                NavalCombatLifecycleRow(
                    combat_id=241671,
                    generation=3,
                    body_ptr=0x22AA0001000,
                    location=11842,
                    attacker_tags=("ENG",),
                    defender_tags=("ITA",),
                    attacker_ship_count=2,
                    defender_ship_count=1,
                    attacker_positioning=0.75,
                    defender_positioning=0.5,
                    attacker_status="active",
                    defender_status="active",
                )
            ],
            member_rows=[
                NavalCombatMemberRow(
                    combat_id=241671,
                    generation=3,
                    side="attacker",
                    group_index=0,
                    member_index=0,
                    unit_type_tag=51,
                    ship_instance_id=1001,
                    cship_join_validated=True,
                    owner_tag_db_id=17,
                    name="HMS Synthetic",
                    type_display="Destroyer",
                    archetype="screen",
                    designed_max_strength_raw=100_000,
                    battle_slot_index_raw=1,
                    state_bitmask=1,
                    engagement_weight_raw=9_000,
                ),
                NavalCombatMemberRow(
                    combat_id=241671,
                    generation=3,
                    side="defender",
                    group_index=0,
                    member_index=0,
                    unit_type_tag=0,
                    is_convoy=True,
                    owner_tag_db_id=18,
                    type_display="Convoy",
                    archetype="convoy",
                    designed_max_strength_raw=120_000,
                    battle_slot_index_raw=2,
                    state_bitmask=1,
                    engagement_weight_raw=8_500,
                ),
            ],
        )

        result = gate_stream(stream)

        self.assertTrue(result.ok, result.errors)
        self.assertEqual(result.summary["record_count"], 2)
        self.assertEqual(result.summary["combat_row_count"], 1)
        self.assertEqual(result.summary["member_row_count"], 2)
        self.assertEqual(result.summary["two_sided_combat_count"], 1)

    def test_gate_rejects_missing_defender_member(self) -> None:
        stream = encode_records(
            sequence_start=1,
            game_hour_raw=129,
            elapsed_wall_ms=1_000,
            combat_rows=[
                NavalCombatLifecycleRow(
                    combat_id=99,
                    generation=1,
                    body_ptr=0x22AA0002000,
                    location=1,
                    attacker_ship_count=1,
                    defender_ship_count=1,
                )
            ],
            member_rows=[
                NavalCombatMemberRow(
                    combat_id=99,
                    generation=1,
                    side="attacker",
                    group_index=0,
                    member_index=0,
                    unit_type_tag=51,
                    ship_instance_id=1002,
                    cship_join_validated=True,
                    battle_slot_index_raw=1,
                )
            ],
        )

        result = gate_stream(stream)

        self.assertFalse(result.ok)
        self.assertIn("combat 99:1 lacks two-sided member rows", result.errors)

    def test_encoder_rejects_non_convoy_ship_without_cship_join(self) -> None:
        with self.assertRaises(ValueError):
            encode_records(
                sequence_start=1,
                game_hour_raw=130,
                elapsed_wall_ms=1_000,
                combat_rows=[
                    NavalCombatLifecycleRow(
                        combat_id=100,
                        generation=1,
                        body_ptr=0x22AA0002000,
                        location=1,
                        attacker_ship_count=1,
                        defender_ship_count=1,
                    )
                ],
                member_rows=[
                    NavalCombatMemberRow(
                        combat_id=100,
                        generation=1,
                        side="attacker",
                        group_index=0,
                        member_index=0,
                        unit_type_tag=51,
                        ship_instance_id=1003,
                        cship_join_validated=False,
                        battle_slot_index_raw=1,
                    )
                ],
            )

    def test_gate_rejects_bad_compact_magic(self) -> None:
        stream = bytearray(encode_records(
            sequence_start=1,
            game_hour_raw=1,
            elapsed_wall_ms=1,
            combat_rows=[NavalCombatLifecycleRow(combat_id=1, generation=1, body_ptr=0x22AA0003000)],
            member_rows=[],
        ))
        stream[0:4] = b"BAD!"

        result = gate_stream(bytes(stream))

        self.assertFalse(result.ok)
        self.assertIn("bad compact magic", result.errors[0])
        self.assertEqual(MAGIC, b"H4CR")

    def test_bridge_parser_rejects_empty_and_bad_pointers(self) -> None:
        self.assertEqual(parse_bridge_rows(""), [])
        with self.assertRaises(ValueError):
            parse_bridge_rows("1:0x1000")

    def test_proof_from_bridge_writes_compact_and_report(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            report = proof_from_bridge_text(
                "241671:0x22AA0004000",
                out_dir=Path(td),
                game_hour_raw=512,
            )

            self.assertEqual(report["status"], "passed")
            self.assertTrue(Path(report["compact_path"]).exists())
            self.assertTrue((Path(td) / "naval_combat_lane_local_compact_proof.json").exists())

    def test_historical_raw_converter_builds_two_sided_compact_stream(self) -> None:
        frames = [
            {
                "type": "domain_delta",
                "domain": "naval_air_combats",
                "hour_index": 17021132,
                "confidence": "sampled",
                "payload": {
                    "op": "sample",
                    "after": {
                        "combat_id": 112568,
                        "combat_key": "naval_combat:112568:1",
                        "generation": 1,
                        "location": 5890,
                        "attacker_tags": ["ENG"],
                        "defender_tags": ["JAP"],
                        "attacker_ship_count": 1,
                        "defender_ship_count": 1,
                        "attacker_positioning": 1,
                        "defender_positioning": 1,
                        "attacker_status": "exact",
                        "defender_status": "exact",
                    },
                },
            },
            {
                "type": "domain_delta",
                "domain": "naval_combat_members",
                "hour_index": 17021132,
                "confidence": "sampled",
                "payload": {
                    "op": "sample",
                    "after": {
                        "combat_id": 112568,
                        "combat_key": "naval_combat:112568:1",
                        "generation": 1,
                        "side": "attacker",
                        "group_index": 0,
                        "member_index": 0,
                        "unit_type_tag": 51,
                        "ship_instance_id": 1234,
                        "cship_status": "resolved",
                        "is_convoy": False,
                        "owner_tag_db_id": 17,
                        "owner": "ENG",
                        "name": "HMS Test",
                        "type_display": "Destroyer",
                        "archetype": "screen",
                        "designed_max_strength_raw": 100000,
                        "battle_slot_index_raw": 1,
                        "state_bitmask": 1,
                        "engagement_weight_raw": 1000,
                    },
                },
            },
            {
                "type": "domain_delta",
                "domain": "naval_combat_members",
                "hour_index": 17021132,
                "confidence": "sampled",
                "payload": {
                    "op": "sample",
                    "after": {
                        "combat_id": 112568,
                        "combat_key": "naval_combat:112568:1",
                        "generation": 1,
                        "side": "defender",
                        "group_index": 0,
                        "member_index": 0,
                        "unit_type_tag": 0,
                        "ship_instance_id": None,
                        "is_convoy": True,
                        "owner_tag_db_id": 18,
                        "owner": "JAP",
                        "name": None,
                        "type_display": "Convoy",
                        "archetype": "convoy",
                        "designed_max_strength_raw": 0,
                        "battle_slot_index_raw": 2,
                        "state_bitmask": 8,
                        "engagement_weight_raw": 0,
                    },
                },
            },
        ]
        with tempfile.TemporaryDirectory() as td:
            source = Path(td) / "collector_frames.jsonl"
            source.write_text(
                "".join(json.dumps(frame) + "\n" for frame in frames),
                encoding="utf-8",
            )
            report = convert_raw_jsonl_to_compact(source, Path(td) / "out")

            self.assertEqual(report["status"], "passed")
            self.assertEqual(report["combat_row_count"], 1)
            self.assertEqual(report["member_row_count"], 2)
            self.assertEqual(report["two_sided_combat_ids"], [112568])


if __name__ == "__main__":
    unittest.main()
