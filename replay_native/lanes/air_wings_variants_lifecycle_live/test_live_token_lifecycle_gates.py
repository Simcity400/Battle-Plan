from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

import proof_live_token_lifecycle_gates as proof


def record(sequence: int, hour: int, domain_id: int, payload: bytes) -> bytes:
    return proof.RECORD_HEADER.pack(
        proof.RECORD_MAGIC,
        proof.RECORD_VERSION,
        proof.RECORD_HEADER_LEN,
        sequence,
        hour,
        domain_id,
        proof.RECORD_TYPE_DOMAIN_DATA,
        len(payload),
        0,
    ) + payload


def compact_string(value: str) -> bytes:
    raw = value.encode("utf-8")
    return len(raw).to_bytes(2, "little") + raw


def air_wings_payload() -> bytes:
    rows = [
        proof.AIR_WING_ROW.pack(1, 101, 7, 0, 100, 1, 55, 90, 0),
        proof.AIR_WING_ROW.pack(1, 102, 8, proof.ROW_FLAG_DELETED, 0, 0, 0, 0, 0),
    ]
    return (
        (1).to_bytes(8, "little")
        + (2).to_bytes(4, "little")
        + (2).to_bytes(4, "little")
        + (32).to_bytes(4, "little")
        + (2).to_bytes(4, "little")
        + (1).to_bytes(4, "little")
        + proof.COMPACT_FIXED_SCALE_RAW.to_bytes(4, "little")
        + (0).to_bytes(4, "little")
        + b"".join(rows)
    )


def loadout_payload(air_base_id: int) -> bytes:
    rows = [proof.AIR_WING_LOADOUT_ROW.pack(1, 101, 7, 0, air_base_id, 0, 1)]
    entries = [proof.AIR_WING_LOADOUT_ENTRY_ROW.pack(2001, 0, 100_000)]
    return (
        (1).to_bytes(8, "little")
        + (1).to_bytes(4, "little")
        + (1).to_bytes(4, "little")
        + (32).to_bytes(4, "little")
        + (1).to_bytes(4, "little")
        + (1).to_bytes(4, "little")
        + (1).to_bytes(4, "little")
        + (0).to_bytes(4, "little")
        + proof.COMPACT_FIXED_SCALE_RAW.to_bytes(4, "little")
        + (0).to_bytes(4, "little")
        + b"".join(rows)
        + b"".join(entries)
    )


def reserves_payload() -> bytes:
    row = proof.AIR_VARIANT_RESERVE_ROW.pack(1, 2001, 0, 0, 25_000)
    return (
        (1).to_bytes(8, "little")
        + (1).to_bytes(4, "little")
        + (1).to_bytes(4, "little")
        + (32).to_bytes(4, "little")
        + (2).to_bytes(4, "little")
        + (3).to_bytes(4, "little")
        + (4).to_bytes(4, "little")
        + (3).to_bytes(4, "little")
        + (0).to_bytes(4, "little")
        + (0).to_bytes(4, "little")
        + proof.COMPACT_PRODUCTION_SCALE_RAW.to_bytes(4, "little")
        + row
    )


def stockpile_variant_defs_payload() -> bytes:
    row = proof.AIR_VARIANT_DEF_ROW.pack(2001, 0, 0, 0, 0, 0)
    return (
        (1).to_bytes(8, "little")
        + (1).to_bytes(4, "little")
        + (1).to_bytes(4, "little")
        + (0).to_bytes(4, "little")
        + (1).to_bytes(4, "little")
        + (1).to_bytes(4, "little")
        + (0).to_bytes(4, "little")
        + (0).to_bytes(4, "little")
        + proof.COMPACT_FIXED_SCALE_RAW.to_bytes(4, "little")
        + (0).to_bytes(4, "little")
        + row
        + compact_string("Reserve Test")
        + compact_string("reserve_test_variant")
    )


def superiority_payload() -> bytes:
    rows = [
        proof.AIR_SUPERIORITY_ROW.pack(55, 1, 1, 2001, 0, 20190, 1, 5, 1, 0, 100_000),
        proof.AIR_SUPERIORITY_ROW.pack(55, 1, 1, 2002, proof.ROW_FLAG_DELETED, 20190, 1, 5, 0, 0, 0),
    ]
    return (
        (1).to_bytes(8, "little")
        + (2).to_bytes(4, "little")
        + (2).to_bytes(4, "little")
        + (32).to_bytes(4, "little")
        + (1).to_bytes(4, "little")
        + (1).to_bytes(4, "little")
        + (1).to_bytes(4, "little")
        + (1).to_bytes(4, "little")
        + (0).to_bytes(4, "little")
        + (0).to_bytes(4, "little")
        + (1).to_bytes(4, "little")
        + proof.COMPACT_FIXED_SCALE_RAW.to_bytes(4, "little")
        + b"".join(rows)
    )


class DecodeStreamLifecycleTests(unittest.TestCase):
    def test_lifecycle_rows_decode_delete_rebase_and_reserve_linkage(self) -> None:
        blob = b"".join(
            [
                record(1, 100, proof.DOMAIN_AIR_WINGS, air_wings_payload()),
                record(2, 100, proof.DOMAIN_AIR_WING_LOADOUTS, loadout_payload(5)),
                record(3, 101, proof.DOMAIN_AIR_WING_LOADOUTS, loadout_payload(6)),
                record(4, 101, proof.DOMAIN_STOCKPILE_VARIANT_DEFS, stockpile_variant_defs_payload()),
                record(5, 101, proof.DOMAIN_AIR_VARIANT_RESERVES, reserves_payload()),
                record(6, 101, proof.DOMAIN_AIR_SUPERIORITY_INPUTS, superiority_payload()),
            ]
        )
        with tempfile.TemporaryDirectory() as td:
            path = Path(td) / "collector_stream.bin"
            path.write_bytes(blob)
            decoded = proof.decode_stream(path)

        self.assertEqual(decoded["decode_errors"], [])
        lifecycle = decoded["lifecycle"]
        self.assertEqual(lifecycle["air_wing_deleted_rows"], 1)
        self.assertEqual(len(lifecycle["loadout_rebase_events"]), 1)
        self.assertEqual(lifecycle["loadout_rebase_events"][0]["old_air_base_id"], 5)
        self.assertEqual(lifecycle["loadout_rebase_events"][0]["new_air_base_id"], 6)
        self.assertEqual(lifecycle["air_variant_reserve_matched_stockpile_rows"], 3)
        self.assertEqual(lifecycle["air_superiority_input_deleted_rows"], 1)
        reserve_linkage = decoded["reserve_stockpile_linkage"]
        self.assertEqual(reserve_linkage["reserve_variant_ids"], 1)
        self.assertEqual(reserve_linkage["reserve_variant_ids_with_stockpile_defs"], 1)
        self.assertEqual(reserve_linkage["reserve_variant_ids_missing_stockpile_defs"], 0)

    def test_derives_modular_airframe_family_from_live_token_name(self) -> None:
        self.assertEqual(
            proof.derive_airframe_family("small_plane_cas_airframe_2"),
            "small_plane_cas_airframe",
        )
        self.assertEqual(
            proof.derive_airframe_family("cv_small_plane_naval_bomber_airframe_1"),
            "cv_small_plane_naval_bomber_airframe",
        )
        self.assertEqual(
            proof.derive_airframe_family("transport_plane_equipment_1"),
            "transport_plane_equipment",
        )
        self.assertIsNone(proof.derive_airframe_family("not_an_airframe_token"))

    def test_source_line_matches_exact_token_boundaries(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            path = Path(td) / "source.txt"
            path.write_text(
                "small_plane_cas_airframe_1\n"
                "prefix_small_plane_cas_airframe_1_suffix\n"
                "cv_small_plane_cas_airframe_1 = cv_CAS_equipment_1\n",
                encoding="utf-8",
            )
            matches = proof.source_line_matches(
                path,
                {"small_plane_cas_airframe_1", "cv_small_plane_cas_airframe_1"},
            )
        self.assertEqual([row["line"] for row in matches["small_plane_cas_airframe_1"]], [1])
        self.assertEqual([row["line"] for row in matches["cv_small_plane_cas_airframe_1"]], [3])


if __name__ == "__main__":
    unittest.main()
