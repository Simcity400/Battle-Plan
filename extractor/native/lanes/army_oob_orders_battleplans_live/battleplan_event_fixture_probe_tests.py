from __future__ import annotations

from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))

import battleplan_event_fixture_probe as probe
import battleplan_fixture_observer as observer
import battleplan_fixture_gate as gate


def capture_ok() -> dict:
    return {
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
    }


def watch() -> dict:
    return {
        "army_id": 257,
        "army_addr": "0x10000001000",
        "order_addr": "0x10000002000",
        "slot_index": 0,
        "order_type": 2,
        "order_generation": 1,
    }


def geometry_entry(values: list[int]) -> dict:
    return {
        "watch_order": watch(),
        "valid": True,
        "candidates": [
            {
                "candidate_name": "type1_type2_type3_path_vector_plus_0x70",
                "field_name": "province_path",
                "field_type": "vec<u32>",
                "begin_offset": "+0x70",
                "size_offset": "+0x7C",
                "valid": True,
                "value": values,
            }
        ],
    }


def execution_entry(values: dict[str, int]) -> dict:
    return {
        "watch_order": watch(),
        "valid": True,
        "u8_window": values,
    }


class BattleplanEventFixtureProbeTests(unittest.TestCase):
    def test_builds_gateable_rows_from_redraw_and_toggle(self) -> None:
        initial = {
            "geometry": [geometry_entry([10, 11])],
            "execution": [execution_entry({"+0x80": 0, "+0x81": 7, "+0x150": 0})],
        }
        after_geometry = {
            "geometry": [geometry_entry([10, 11, 12])],
            "execution": [execution_entry({"+0x80": 0, "+0x81": 8, "+0x150": 1})],
        }
        after_toggle = {
            "geometry": [geometry_entry([10, 11, 12])],
            "execution": [execution_entry({"+0x80": 1, "+0x81": 8, "+0x150": 0})],
        }

        obs = probe.build_observation(
            gate.EXPECTED_SHA256,
            capture_ok(),
            initial,
            after_geometry,
            after_toggle,
            max_execution_candidates=8,
        )
        fixture, errors = observer.build_fixture(obs, allow_rejection=False)

        self.assertEqual(errors, [])
        self.assertEqual(len(obs["observations"]), 2)
        self.assertEqual(obs["observations"][1]["candidate"], "+0x80")
        self.assertEqual(fixture["non_type5_geometry"][0]["validated_offsets"][1]["offset"], "+0x7C")
        self.assertEqual(fixture["battleplan_execution_state"][0]["field_values"], {"execution_u8_0x80": 1})

    def test_execution_scan_requires_no_redraw_mutation(self) -> None:
        initial = {
            "geometry": [geometry_entry([10, 11])],
            "execution": [execution_entry({"+0x80": 0, "+0x82": 0})],
        }
        after_geometry = {
            "geometry": [geometry_entry([10, 11])],
            "execution": [execution_entry({"+0x80": 1, "+0x82": 0})],
        }
        after_toggle = {
            "geometry": [geometry_entry([10, 11])],
            "execution": [execution_entry({"+0x80": 0, "+0x82": 1})],
        }

        rows = probe.execution_observation_rows(initial, after_geometry, after_toggle, max_rows=8)

        self.assertEqual([row["candidate"] for row in rows], ["+0x82"])


if __name__ == "__main__":
    unittest.main()
