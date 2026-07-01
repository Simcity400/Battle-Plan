import unittest
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import market_trade_live_compact as mt


def row(contract_id: int, flags: int) -> mt.MarketTradeRow:
    variant = 1000 + contract_id if flags & mt.FLAG_VARIANT_VALID else 0
    amount = 3_200_000 if flags & mt.FLAG_AMOUNT_VALID else 0
    return mt.MarketTradeRow(
        contract_id=contract_id,
        contract_type=mt.CPURCHASE_CONTRACT_TYPE,
        flags=flags,
        seller_tag_db_id=11,
        buyer_tag_db_id=22,
        equipment_variant_id=variant,
        amount_raw=amount,
        equipment_subbuf_status=(
            mt.STATUS_VALID if flags & mt.FLAG_EQUIPMENT_SUBBUF_VALID else mt.STATUS_INVALID
        ),
        delivery_subbuf_status=(
            mt.STATUS_VALID if flags & mt.FLAG_DELIVERY_SUBBUF_VALID else mt.STATUS_INVALID
        ),
        buyer_ptr_status=mt.STATUS_VALID if flags & mt.FLAG_BUYER_PTR_VALID else mt.STATUS_INVALID,
    )


class MarketTradeLiveCompactTests(unittest.TestCase):
    def test_roundtrip_keeps_valid_and_fail_closed_partial_rows(self):
        valid_flags = (
            mt.FLAG_BUYER_PTR_VALID
            | mt.FLAG_EQUIPMENT_SUBBUF_VALID
            | mt.FLAG_DELIVERY_SUBBUF_VALID
            | mt.FLAG_VARIANT_VALID
            | mt.FLAG_AMOUNT_VALID
        )
        rows = [row(i + 1, valid_flags) for i in range(66)]
        rows.extend(row(100 + i, mt.FLAG_DELIVERY_SUBBUF_VALID) for i in range(4))
        frame = mt.MarketTradeFrame(
            game_hour_raw=60846677,
            elapsed_wall_ms=1234,
            rows=tuple(rows),
            status_flags=0,
            observed_contract_rows=70,
            skipped_contract_rows=0,
        )

        blob = mt.encode_frame(frame)
        decoded, errors = mt.decode_frames(blob)

        self.assertEqual(errors, [])
        self.assertEqual(len(decoded), 1)
        self.assertEqual(len(decoded[0].rows), 70)
        self.assertEqual(decoded[0].buyer_ptr_valid_rows, 66)
        self.assertEqual(decoded[0].variant_valid_rows, 66)
        self.assertEqual(decoded[0].amount_valid_rows, 66)
        self.assertEqual(decoded[0].partial_field_rows, 4)
        for partial in decoded[0].rows[-4:]:
            self.assertEqual(partial.equipment_variant_id, 0)
            self.assertEqual(partial.amount_raw, 0)

    def test_decoder_rejects_fabricated_variant_when_flag_unset(self):
        partial = row(1, mt.FLAG_DELIVERY_SUBBUF_VALID)
        fabricated = mt.MarketTradeRow(
            **{**partial.__dict__, "equipment_variant_id": 99}
        )
        frame = mt.MarketTradeFrame(
            game_hour_raw=1,
            elapsed_wall_ms=1,
            rows=(fabricated,),
            status_flags=0,
            observed_contract_rows=1,
            skipped_contract_rows=0,
        )

        _decoded, errors = mt.decode_frames(mt.encode_frame(frame))

        self.assertTrue(any("variant present without valid flag" in err for err in errors))

    def test_gate_reports_skipped_hours(self):
        frames = [
            mt.MarketTradeFrame(10, 0, (), 0, 0, 0),
            mt.MarketTradeFrame(12, 1, (), 0, 0, 0),
        ]

        gate = mt.gate_frames(frames, [], min_game_hours=2)

        self.assertFalse(gate["ok"])
        self.assertIn("skipped_game_hours", gate["problems"])
        self.assertEqual(gate["skipped_game_hours"], 1)

    def test_gate_accepts_omitted_candidates_without_structural_status(self):
        valid_flags = (
            mt.FLAG_BUYER_PTR_VALID
            | mt.FLAG_EQUIPMENT_SUBBUF_VALID
            | mt.FLAG_DELIVERY_SUBBUF_VALID
            | mt.FLAG_VARIANT_VALID
            | mt.FLAG_AMOUNT_VALID
        )
        frame = mt.MarketTradeFrame(
            game_hour_raw=1,
            elapsed_wall_ms=1,
            rows=(row(1, valid_flags),),
            status_flags=0,
            observed_contract_rows=5,
            skipped_contract_rows=4,
            omitted_candidate_rows=4,
            structural_partial_rows=0,
        )
        decoded, errors = mt.decode_frames(mt.encode_frame(frame))

        gate = mt.gate_frames(decoded, errors, min_game_hours=1)

        self.assertTrue(gate["ok"])
        self.assertEqual(decoded[0].omitted_candidate_rows, 4)
        self.assertEqual(gate["omitted_candidate_rows_max"], 4)
        self.assertFalse(gate["structural_partial_seen"])

    def test_gate_rejects_structural_partial_status(self):
        frame = mt.MarketTradeFrame(
            game_hour_raw=1,
            elapsed_wall_ms=1,
            rows=(),
            status_flags=mt.STATUS_STRUCTURAL_PARTIAL,
            observed_contract_rows=1,
            skipped_contract_rows=1,
            omitted_candidate_rows=0,
            structural_partial_rows=1,
        )

        gate = mt.gate_frames([frame], [], min_game_hours=1)

        self.assertFalse(gate["ok"])
        self.assertIn("structural_partial", gate["problems"])

    def test_compact_dimensions_are_integration_stable(self):
        self.assertEqual(mt.HEADER_LEN, 64)
        self.assertEqual(mt.ROW_LEN, 48)


if __name__ == "__main__":
    unittest.main()
