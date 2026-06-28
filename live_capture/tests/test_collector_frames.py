import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))

from live_capture.collector_frames import (  # noqa: E402
    latest_verified_root_probe,
    make_heartbeat_frame,
    read_collector_jsonl,
    snapshot_from_collector_frame,
    snapshot_from_root_probe_frame,
    validate_collector_frame,
)
from replay import schema  # noqa: E402


class TestCollectorFrames(unittest.TestCase):
    def test_synthetic_snapshot_frame_validates(self):
        frame = {
            "type": "snapshot",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "sequence": 7,
            "exe_sha256": "0" * 64,
            "mod_checksum": "vanilla",
            "profile_id": "hoi4-1.19.1-test",
            "quality_status": "synthetic",
            "capture_boundary": "daily_update",
            "transport": "synthetic",
            "transport_crc32": "1234abcd",
            "transport_bytes": 512,
            "date": "1936.1.2.1",
            "domains": ["country_identity", "clock"],
            "countries": {
                "GER": {
                    "identity": {
                        "original_capital": 64,
                        "original_tag": None,
                        "reserved_dynamic_country": False,
                        "templates_locked": False,
                    },
                },
            },
        }

        snap = snapshot_from_collector_frame(frame, "collector-test")

        self.assertEqual(schema.validate_snapshot(snap), [])
        self.assertEqual(snap["hour_index"], schema.date_to_hour_index("1936.1.2.1"))
        self.assertIsNone(snap["provenance"]["memory"])
        self.assertIsNone(snap["provenance"]["save"])
        self.assertEqual(snap["provenance"]["collector"]["source"]
                         if "source" in snap["provenance"]["collector"] else None, None)
        self.assertEqual(snap["provenance"]["collector"]["backend"],
                         "in_process_collector")
        self.assertEqual(snap["provenance"]["collector"]["sequence"], 7)
        self.assertEqual(snap["provenance"]["collector"]["transport_crc32"], "1234abcd")
        self.assertEqual(snap["provenance"]["collector"]["transport_bytes"], 512)
        self.assertEqual(snap["provenance"]["collector"]["domains"],
                         ["clock", "country_identity"])
        self.assertEqual(snap["countries"]["GER"]["identity"]["original_capital"], 64)

    def test_unknown_domain_rejected(self):
        frame = {
            "type": "snapshot",
            "date": "1936.1.2.1",
            "domains": ["not_a_domain"],
        }

        problems = validate_collector_frame(frame)

        self.assertTrue(any("not canonical" in p for p in problems), problems)
        with self.assertRaisesRegex(ValueError, "not canonical"):
            snapshot_from_collector_frame(frame, "collector-test")

    def test_heartbeat_is_not_a_snapshot(self):
        frame = make_heartbeat_frame(collector_version="0.1.0", sequence=3)

        self.assertEqual(validate_collector_frame(frame), [])
        with self.assertRaisesRegex(ValueError, "not a snapshot"):
            snapshot_from_collector_frame(frame, "collector-test")

    def test_attach_log_root_probe_frames_validate(self):
        frames = [
            {
                "type": "collector_started",
                "backend": "in_process_collector",
                "collector_version": "0.1.0",
                "abi_version": 1,
                "process_id": 6300,
                "tick_ms": 158373750,
                "quality_status": "attach_noop",
                "root_probe": None,
            },
            {
                "type": "collector_started",
                "backend": "in_process_collector",
                "collector_version": "0.1.0",
                "abi_version": 1,
                "process_id": 32440,
                "tick_ms": 158373765,
                "quality_status": "root_probe_verified",
                "root_probe": {
                    "status": "verified",
                    "profile": "vanilla_latest_2026_06_28",
                    "image_base": "0x7FF65D810000",
                    "daily_update": "0x7FF65D9E4540",
                    "get_long_string": "0x7FF65DD369B0",
                    "date_root_ptr": "0x1D499734A00",
                    "country_database_root_ptr": "0x1D3EF67FD60",
                    "current_game_date": "1936.1.1.12",
                    "current_game_date_raw": 60759371,
                    "start_date": "1936.1.1.12",
                    "start_date_raw": 60759371,
                    "country_tag_count": 474,
                    "country_active_count": 440,
                    "tag_sample": ["---", "GER", "ENG"],
                    "domains": ["clock", "country_identity"],
                },
            },
        ]

        for frame in frames:
            self.assertEqual(validate_collector_frame(frame), [])
        probe = latest_verified_root_probe(frames)
        self.assertIsNotNone(probe)
        self.assertEqual(probe["profile"], "vanilla_latest_2026_06_28")
        self.assertEqual(probe["domains"], ["clock", "country_identity"])
        self.assertEqual(probe["tag_sample"][:3], ["---", "GER", "ENG"])

    def test_root_probe_frame_builds_clock_snapshot(self):
        frame = {
            "type": "collector_started",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "process_id": 32440,
            "tick_ms": 158373765,
            "quality_status": "root_probe_verified",
            "root_probe": {
                "status": "verified",
                "profile": "vanilla_latest_2026_06_28",
                "image_base": "0x7FF65D810000",
                "daily_update": "0x7FF65D9E4540",
                "get_long_string": "0x7FF65DD369B0",
                "date_root_ptr": "0x1D499734A00",
                "country_database_root_ptr": "0x1D3EF67FD60",
                "current_game_date": "1936.6.15.7",
                "current_game_date_raw": 60763326,
                "start_date": "1936.1.1.12",
                "start_date_raw": 60759371,
                "country_tag_count": 474,
                "country_active_count": 440,
                "tag_sample": ["---", "GER", "ENG"],
                "domains": ["clock", "country_identity"],
            },
        }

        snap = snapshot_from_root_probe_frame(frame, "collector-test")

        self.assertEqual(schema.validate_snapshot(snap), [])
        self.assertEqual(snap["date"], "1936.6.15.7")
        self.assertEqual(snap["world"]["scenario"]["current_date_live"], "1936.6.15.7")
        self.assertEqual(snap["world"]["scenario"]["start_date_live"], "1936.1.1.12")
        self.assertEqual(snap["provenance"]["collector"]["domains"], ["clock"])
        self.assertEqual(
            snap["provenance"]["collector"]["root_probe"]["tag_sample"][:3],
            ["---", "GER", "ENG"],
        )

    def test_root_probe_frame_folds_present_country_rows(self):
        frame = {
            "type": "collector_started",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "process_id": 32440,
            "tick_ms": 158373765,
            "quality_status": "root_probe_verified",
            "countries": {
                "GER": {"present": True},
                "ENG": {"present": True},
            },
            "root_probe": {
                "status": "verified",
                "profile": "vanilla_latest_2026_06_28",
                "current_game_date": "1936.6.15.7",
                "current_game_date_raw": 60763326,
                "start_date": "1936.1.1.12",
                "start_date_raw": 60759371,
                "country_tag_count": 474,
                "country_active_count": 440,
                "tag_sample": ["---", "GER", "ENG"],
                "domains": ["clock", "country_identity"],
            },
        }

        snap = snapshot_from_root_probe_frame(frame, "collector-test")

        self.assertEqual(schema.validate_snapshot(snap), [])
        self.assertEqual(snap["provenance"]["collector"]["domains"],
                         ["clock", "country_identity"])
        self.assertEqual(sorted(snap["countries"]), ["ENG", "GER"])
        self.assertIsNone(snap["countries"]["GER"]["identity"])

    def test_root_probe_frame_folds_country_scalars(self):
        frame = {
            "type": "collector_started",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "process_id": 32440,
            "tick_ms": 158373765,
            "quality_status": "root_probe_verified",
            "countries": {
                "POR": {
                    "present": True,
                    "collector_object": "0x1234",
                    "identity": {
                        "original_capital": 112,
                        "original_tag": None,
                        "reserved_dynamic_country": False,
                        "templates_locked": False,
                    },
                    "scalars": {
                        "stability": 0.61,
                        "war_support": 0.42,
                        "command_power": 12.5,
                        "num_ships": 7,
                        "manpower_pool": 123456,
                        "convoys": 99.0,
                    },
                },
            },
            "root_probe": {
                "status": "verified",
                "profile": "vanilla_latest_2026_06_28",
                "current_game_date": "1936.6.15.7",
                "current_game_date_raw": 60763326,
                "start_date": "1936.1.1.12",
                "start_date_raw": 60759371,
                "country_tag_count": 474,
                "country_active_count": 440,
                "tag_sample": ["---", "GER", "ENG"],
                "domains": ["clock", "country_identity", "country_scalars"],
            },
        }

        snap = snapshot_from_root_probe_frame(frame, "collector-test")

        self.assertEqual(schema.validate_snapshot(snap), [])
        self.assertEqual(snap["provenance"]["collector"]["domains"],
                         ["clock", "country_identity", "country_scalars"])
        por = snap["countries"]["POR"]
        self.assertEqual(por["identity"]["original_capital"], 112)
        self.assertEqual(por["politics"]["stability"], 0.61)
        self.assertEqual(por["politics"]["war_support"], 0.42)
        self.assertEqual(por["politics"]["command_power"], 12.5)
        self.assertEqual(por["military"]["command_power"], 12.5)
        self.assertEqual(por["military"]["num_ships"], 7)
        self.assertEqual(por["military"]["manpower"]["available"], 123456)
        self.assertEqual(por["military"]["convoys_count"], 99)

    def test_country_present_marker_must_be_bool(self):
        frame = {
            "type": "snapshot",
            "date": "1936.1.2.1",
            "countries": {"GER": {"present": "yes"}},
        }

        problems = validate_collector_frame(frame)

        self.assertTrue(any("present not bool" in p for p in problems), problems)

    def test_country_scalars_validate_types(self):
        frame = {
            "type": "snapshot",
            "date": "1936.1.2.1",
            "countries": {"GER": {"scalars": {"stability": "high", "num_ships": 4.2}}},
        }

        problems = validate_collector_frame(frame)

        self.assertTrue(any("scalars.stability" in p for p in problems), problems)
        self.assertTrue(any("scalars.num_ships" in p for p in problems), problems)

    def test_transport_metadata_types_validate(self):
        frame = {
            "type": "heartbeat",
            "transport": "named_pipe",
            "transport_crc32": 1234,
            "transport_bytes": "512",
        }

        problems = validate_collector_frame(frame)

        self.assertTrue(any("transport_crc32" in p for p in problems), problems)
        self.assertTrue(any("transport_bytes" in p for p in problems), problems)

    def test_jsonl_reader_rejects_bad_root_probe_domain(self):
        import tempfile

        bad = (
            '{"type":"collector_started","backend":"in_process_collector",'
            '"collector_version":"0.1.0","abi_version":1,'
            '"root_probe":{"status":"verified","domains":["bogus"]}}\n'
        )
        with tempfile.TemporaryDirectory() as td:
            path = os.path.join(td, "collector_attach.jsonl")
            with open(path, "w", encoding="utf-8") as f:
                f.write(bad)
            with self.assertRaisesRegex(ValueError, "not canonical"):
                read_collector_jsonl(path)


if __name__ == "__main__":
    unittest.main()
