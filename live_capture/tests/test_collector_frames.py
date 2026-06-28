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
            "session_id": "session-1",
            "session_sequence": 4,
            "duration_ms": 8000,
            "sample_ms": 10,
            "elapsed_wall_ms": 125,
            "date": "1936.1.2.1",
            "domains": ["country_identity", "clock"],
            "countries": {
                "GER": {
                    "identity": {
                        "tag_db_id": 1,
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
        self.assertEqual(snap["provenance"]["collector"]["session_id"], "session-1")
        self.assertEqual(snap["provenance"]["collector"]["session_sequence"], 4)
        self.assertEqual(snap["provenance"]["collector"]["duration_ms"], 8000)
        self.assertEqual(snap["provenance"]["collector"]["sample_ms"], 10)
        self.assertEqual(snap["provenance"]["collector"]["elapsed_wall_ms"], 125)
        self.assertEqual(snap["provenance"]["collector"]["domains"],
                         ["clock", "country_identity"])
        self.assertEqual(snap["countries"]["GER"]["identity"]["original_capital"], 64)
        self.assertEqual(snap["countries"]["GER"]["identity"]["tag_db_id"], 1)

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

    def test_session_and_gap_frames_validate(self):
        frames = [
            {
                "type": "session_start",
                "backend": "in_process_collector",
                "abi_version": 1,
                "session_id": "run-1",
                "process_id": 1234,
                "sample_ms": 10,
                "duration_ms": 8000,
                "quality_status": "session_started",
            },
            {
                "type": "session_stop",
                "backend": "in_process_collector",
                "abi_version": 1,
                "session_id": "run-1",
                "segment_id": 1,
                "elapsed_wall_ms": 8025,
                "quality_status": "session_complete",
            },
            {
                "type": "gap",
                "backend": "in_process_collector",
                "abi_version": 1,
                "date": "1936.1.1.12",
                "segment_id": 1,
                "quality_status": "no_data",
            },
        ]

        for frame in frames:
            self.assertEqual(validate_collector_frame(frame), [])

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
                    "daily_update_sentinel_match": True,
                    "long_string_sentinel_match": True,
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
                "daily_update_sentinel_match": True,
                "long_string_sentinel_match": True,
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
                "GER": {"present": True, "identity": {"tag_db_id": 1}},
                "ENG": {"present": True, "identity": {"tag_db_id": 2}},
            },
            "root_probe": {
                "status": "verified",
                "profile": "vanilla_latest_2026_06_28",
                "daily_update_sentinel_match": True,
                "long_string_sentinel_match": True,
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
        self.assertEqual(snap["countries"]["GER"]["identity"]["tag_db_id"], 1)

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
                        "tag_db_id": 47,
                        "original_capital": 112,
                        "original_tag": None,
                        "reserved_dynamic_country": False,
                        "templates_locked": False,
                    },
                    "scalars": {
                        "stability": 0.61,
                        "war_support": 0.42,
                        "political_power": 7.25,
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
                "daily_update_sentinel_match": True,
                "long_string_sentinel_match": True,
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
        self.assertEqual(por["identity"]["tag_db_id"], 47)
        self.assertEqual(por["identity"]["original_capital"], 112)
        self.assertEqual(por["politics"]["stability"], 0.61)
        self.assertEqual(por["politics"]["war_support"], 0.42)
        self.assertEqual(por["politics"]["political_power"], 7.25)
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

    def test_country_scalars_require_valid_identity(self):
        frame = {
            "type": "snapshot",
            "date": "1936.1.2.1",
            "countries": {
                "GER": {
                    "present": True,
                    "collector_object": "0x1234",
                    "scalars": {"stability": 0.5},
                }
            },
        }

        problems = validate_collector_frame(frame)

        self.assertTrue(any("requires identity.tag_db_id" in p for p in problems),
                        problems)

    def test_country_scalars_validate_ranges(self):
        frame = {
            "type": "snapshot",
            "date": "1936.1.2.1",
            "countries": {
                "GER": {
                    "present": True,
                    "collector_object": "0x1234",
                    "identity": {"tag_db_id": 1},
                    "scalars": {"stability": 1.5, "manpower_pool": -1},
                }
            },
        }

        problems = validate_collector_frame(frame)

        self.assertTrue(any("scalars.stability out of range" in p for p in problems),
                        problems)
        self.assertTrue(any("scalars.manpower_pool out of range" in p for p in problems),
                        problems)

    def test_country_identity_tag_db_id_must_be_int(self):
        frame = {
            "type": "snapshot",
            "date": "1936.1.2.1",
            "countries": {"GER": {"identity": {"tag_db_id": "1"}}},
        }

        problems = validate_collector_frame(frame)

        self.assertTrue(any("identity.tag_db_id" in p for p in problems), problems)

    def test_snapshot_frame_folds_map_control_domains(self):
        frame = {
            "type": "snapshot",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "date": "1936.1.2.1",
            "domains": ["clock", "state_ownership", "state_control",
                        "province_control"],
            "states": {
                "1": {"owner": "GER", "controller": "GER"},
                "2": {"owner": None, "controller": None},
            },
            "provinces": {
                "11": {"controller": "GER", "state_id": 1},
                "12": {"controller": None, "state_id": 2},
            },
        }

        snap = snapshot_from_collector_frame(frame, "collector-test")

        self.assertEqual(schema.validate_snapshot(snap), [])
        self.assertEqual(
            snap["provenance"]["collector"]["domains"],
            ["clock", "province_control", "state_control", "state_ownership"],
        )
        self.assertEqual(snap["states"]["1"]["owner"], "GER")
        self.assertEqual(snap["states"]["1"]["controller"], "GER")
        self.assertIsNone(snap["states"]["2"]["controller"])
        self.assertEqual(snap["provinces"]["11"]["controller"], "GER")
        self.assertEqual(snap["provinces"]["11"]["state_id"], 1)
        self.assertIsNone(snap["provinces"]["12"]["controller"])
        self.assertEqual(snap["map"]["owner_by_state"], {"1": "GER"})
        self.assertEqual(snap["map"]["control_by_province"], {"11": "GER"})

    def test_map_payload_without_domain_claim_is_not_folded(self):
        frame = {
            "type": "snapshot",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "date": "1936.1.2.1",
            "domains": ["clock"],
            "states": {"1": {"controller": "GER"}},
            "provinces": {"11": {"controller": "GER"}},
        }

        snap = snapshot_from_collector_frame(frame, "collector-test")

        self.assertEqual(schema.validate_snapshot(snap), [])
        self.assertEqual(snap["states"], {})
        self.assertEqual(snap["provinces"], {})
        self.assertIsNone(snap["map"]["control_by_province"])

    def test_snapshot_frame_rejects_unsupported_claimed_domains(self):
        frame = {
            "type": "snapshot",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "date": "1936.1.2.1",
            "capture_boundary": "scheduled_hour_snapshot",
            "domains": ["clock", "production_live", "divisions"],
        }

        with self.assertRaisesRegex(ValueError, "unsupported domains: divisions"):
            snapshot_from_collector_frame(frame, "collector-test")

    def test_snapshot_frame_folds_scheduled_fuel_live(self):
        frame = {
            "type": "snapshot",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "date": "1936.1.2.1",
            "capture_boundary": "scheduled_hour_snapshot",
            "domains": ["clock", "country_identity", "fuel_live"],
            "countries": {
                "POR": {
                    "present": True,
                    "collector_object": "0x1000",
                    "identity": {"tag_db_id": 47},
                    "fuel": {"current": 123.5, "max": 456.25},
                },
            },
        }

        snap = snapshot_from_collector_frame(frame, "collector-test")

        self.assertEqual(schema.validate_snapshot(snap), [])
        self.assertEqual(
            snap["provenance"]["collector"]["domains"],
            ["clock", "country_identity", "fuel_live"],
        )
        self.assertEqual(snap["countries"]["POR"]["economy"]["fuel"]["current"], 123.5)
        self.assertEqual(snap["countries"]["POR"]["economy"]["fuel"]["max"], 456.25)

    def test_snapshot_frame_rejects_non_scheduled_fuel_live(self):
        frame = {
            "type": "snapshot",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "date": "1936.1.2.1",
            "capture_boundary": "daily_update",
            "domains": ["clock", "country_identity", "fuel_live"],
            "countries": {
                "POR": {
                    "present": True,
                    "collector_object": "0x1000",
                    "identity": {"tag_db_id": 47},
                    "fuel": {"current": 123.5, "max": 456.25},
                },
            },
        }

        with self.assertRaisesRegex(ValueError, "fuel_live payload requires scheduled"):
            snapshot_from_collector_frame(frame, "collector-test")

    def test_snapshot_frame_folds_scheduled_production_live(self):
        frame = {
            "type": "snapshot",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "date": "1936.1.2.1",
            "capture_boundary": "scheduled_hour_snapshot",
            "domains": ["clock", "country_identity", "production_live"],
            "countries": {
                "GER": {
                    "present": True,
                    "collector_object": "0x1000",
                    "identity": {"tag_db_id": 1},
                    "production": {
                        "stockpile": {"70": 123.5, "71": 0.0, "72": -2.5},
                        "stockpile_total": 121.0,
                        "military_lines": [{
                            "id": 349,
                            "active_factories": 3,
                            "cost": 105.0,
                            "speed": 31.05,
                            "produced": 34.4,
                            "amount": -1,
                        }],
                        "naval_lines": [],
                        "military_factories": 3,
                        "naval_factories": 0,
                    },
                },
                "SOV": {
                    "present": True,
                    "collector_object": "0x2000",
                    "identity": {"tag_db_id": 2},
                    "production": {
                        "stockpile": {},
                        "stockpile_total": 0.0,
                        "military_lines": [],
                        "naval_lines": [],
                        "military_factories": 0,
                        "naval_factories": 0,
                    },
                },
            },
        }

        snap = snapshot_from_collector_frame(frame, "collector-test")

        self.assertEqual(schema.validate_snapshot(snap), [])
        self.assertEqual(
            snap["provenance"]["collector"]["domains"],
            ["clock", "country_identity", "production_live"],
        )
        ger = snap["countries"]["GER"]["production"]
        self.assertEqual(ger["stockpile"], {"70": 123.5, "71": 0.0, "72": -2.5})
        self.assertEqual(ger["stockpile_total"], 121.0)
        self.assertEqual(ger["military_lines"][0]["amount"], -1)
        self.assertEqual(ger["military_factories"], 3)
        self.assertEqual(ger["naval_lines"], [])
        self.assertEqual(ger["naval_factories"], 0)
        sov = snap["countries"]["SOV"]["production"]
        self.assertEqual(sov["stockpile"], {})
        self.assertEqual(sov["stockpile_total"], 0.0)
        self.assertEqual(sov["military_lines"], [])
        self.assertEqual(sov["naval_lines"], [])
        self.assertEqual(sov["military_factories"], 0)
        self.assertEqual(sov["naval_factories"], 0)
        self.assertEqual(snap["countries"]["GER"]["military"]["equipment_stockpile"], [])

    def test_snapshot_frame_preserves_unavailable_production_leaves(self):
        frame = {
            "type": "snapshot",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "date": "1936.1.2.1",
            "capture_boundary": "scheduled_hour_snapshot",
            "domains": ["clock", "country_identity", "production_live"],
            "countries": {
                "GER": {
                    "present": True,
                    "collector_object": "0x1000",
                    "identity": {"tag_db_id": 1},
                    "production": {
                        "military_lines": [{
                            "id": 349,
                            "active_factories": 3,
                            "cost": 105.0,
                            "speed": 31.05,
                            "produced": 34.4,
                            "amount": -1,
                        }],
                        "naval_lines": [],
                        "military_factories": 3,
                        "naval_factories": 0,
                    },
                },
            },
        }

        snap = snapshot_from_collector_frame(frame, "collector-test")

        prod = snap["countries"]["GER"]["production"]
        self.assertIsNone(prod["stockpile"])
        self.assertIsNone(prod["stockpile_total"])
        self.assertEqual(prod["military_lines"][0]["id"], 349)
        self.assertEqual(prod["military_factories"], 3)
        self.assertEqual(prod["naval_lines"], [])
        self.assertEqual(prod["naval_factories"], 0)

    def test_snapshot_frame_rejects_malformed_production_live(self):
        frame = {
            "type": "snapshot",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "date": "1936.1.2.1",
            "capture_boundary": "scheduled_hour_snapshot",
            "domains": ["clock", "country_identity", "production_live"],
            "countries": {
                "GER": {
                    "present": True,
                    "collector_object": "0x1000",
                    "identity": {"tag_db_id": 1},
                    "production": {
                        "stockpile": {"70": float("inf")},
                        "stockpile_total": 0.0,
                        "military_lines": [{
                            "id": 349,
                            "active_factories": -1,
                            "cost": 105.0,
                            "speed": 31.05,
                            "produced": 34.4,
                            "amount": -2,
                        }],
                        "naval_lines": "bad",
                        "military_factories": 3,
                        "naval_factories": 0,
                    },
                },
            },
        }

        problems = validate_collector_frame(frame)

        self.assertTrue(any("production.stockpile['70']" in p
                            for p in problems), problems)
        self.assertTrue(any("military_lines[0].active_factories" in p
                            for p in problems), problems)
        self.assertTrue(any("military_lines[0].amount" in p
                            for p in problems), problems)
        self.assertTrue(any("production.naval_lines not list" in p
                            for p in problems), problems)
        with self.assertRaisesRegex(ValueError, "invalid collector frame"):
            snapshot_from_collector_frame(frame, "collector-test")

    def test_snapshot_frame_rejects_non_scheduled_production_live(self):
        frame = {
            "type": "snapshot",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "date": "1936.1.2.1",
            "capture_boundary": "daily_update",
            "domains": ["clock", "country_identity", "production_live"],
            "countries": {
                "GER": {
                    "present": True,
                    "collector_object": "0x1000",
                    "identity": {"tag_db_id": 1},
                    "production": {
                        "stockpile": {},
                        "stockpile_total": 0.0,
                        "military_lines": [],
                        "naval_lines": [],
                        "military_factories": 0,
                        "naval_factories": 0,
                    },
                },
            },
        }

        with self.assertRaisesRegex(ValueError, "production_live payload requires scheduled"):
            snapshot_from_collector_frame(frame, "collector-test")

    def test_root_probe_frame_requires_scheduled_map_domain_claims(self):
        frame = {
            "type": "collector_started",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "process_id": 32440,
            "tick_ms": 158373765,
            "capture_boundary": "scheduled_hour_snapshot",
            "quality_status": "root_probe_verified",
            "domains": ["clock", "state_control", "province_control"],
            "states": {"1": {"controller": "GER"}},
            "provinces": {"11": {"controller": "GER", "state_id": 1}},
            "root_probe": {
                "status": "verified",
                "profile": "vanilla_latest_2026_06_28",
                "daily_update_sentinel_match": True,
                "long_string_sentinel_match": True,
                "current_game_date": "1936.6.15.7",
                "current_game_date_raw": 60763326,
                "start_date": "1936.1.1.12",
                "start_date_raw": 60759371,
                "country_tag_count": 474,
                "country_active_count": 440,
                "tag_sample": ["---", "GER", "ENG"],
                "domains": ["clock", "state_control", "province_control"],
            },
        }

        snap = snapshot_from_root_probe_frame(frame, "collector-test")

        self.assertEqual(schema.validate_snapshot(snap), [])
        self.assertEqual(
            snap["provenance"]["collector"]["domains"],
            ["clock", "state_control", "province_control"],
        )
        self.assertEqual(snap["states"]["1"]["controller"], "GER")
        self.assertEqual(snap["map"]["control_by_province"], {"11": "GER"})

    def test_root_probe_map_payload_without_scheduled_claim_is_not_folded(self):
        frame = {
            "type": "collector_started",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "process_id": 32440,
            "tick_ms": 158373765,
            "quality_status": "root_probe_verified",
            "states": {"1": {"controller": "GER"}},
            "provinces": {"11": {"controller": "GER", "state_id": 1}},
            "root_probe": {
                "status": "verified",
                "profile": "vanilla_latest_2026_06_28",
                "daily_update_sentinel_match": True,
                "long_string_sentinel_match": True,
                "current_game_date": "1936.6.15.7",
                "current_game_date_raw": 60763326,
                "start_date": "1936.1.1.12",
                "start_date_raw": 60759371,
                "country_tag_count": 474,
                "country_active_count": 440,
                "tag_sample": ["---", "GER", "ENG"],
                "domains": ["clock", "state_control", "province_control"],
            },
        }

        snap = snapshot_from_root_probe_frame(frame, "collector-test")

        self.assertEqual(schema.validate_snapshot(snap), [])
        self.assertEqual(snap["provenance"]["collector"]["domains"], ["clock"])
        self.assertEqual(snap["states"], {})
        self.assertEqual(snap["provinces"], {})

    def test_worker_poll_rejects_heavy_map_payloads(self):
        frame = {
            "type": "collector_started",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "capture_boundary": "worker_poll",
            "countries": {
                "POR": {
                    "present": True,
                    "collector_object": "0x1000",
                    "identity": {"tag_db_id": 47},
                    "fuel": {"current": 1.0, "max": 2.0},
                    "production": {
                        "stockpile": {},
                        "stockpile_total": 0.0,
                        "military_lines": [],
                        "naval_lines": [],
                        "military_factories": 0,
                        "naval_factories": 0,
                    },
                },
            },
            "states": {"1": {"controller": "GER"}},
            "provinces": {"11": {"controller": "GER", "state_id": 1}},
        }

        problems = validate_collector_frame(frame)

        self.assertTrue(any("worker_poll frame must not carry fuel" in p
                            for p in problems), problems)
        self.assertTrue(any("worker_poll frame must not carry production" in p
                            for p in problems), problems)
        self.assertTrue(any("worker_poll frame must not carry states" in p
                            for p in problems), problems)
        self.assertTrue(any("worker_poll frame must not carry provinces" in p
                            for p in problems), problems)

    def test_map_control_rejects_raw_ids_and_bad_tags(self):
        frame = {
            "type": "snapshot",
            "date": "1936.1.2.1",
            "states": {
                "0": {"controller": "GER"},
                "1": {"controller": 7},
            },
            "provinces": {
                "11": {"controller": "Germany"},
                "12": {"controller": "GER", "state_id": "1"},
            },
        }

        problems = validate_collector_frame(frame)

        self.assertTrue(any("state key '0'" in p for p in problems), problems)
        self.assertTrue(any("state '1'.controller" in p for p in problems), problems)
        self.assertTrue(any("province '11'.controller" in p for p in problems),
                        problems)
        self.assertTrue(any("province '12'.state_id" in p for p in problems),
                        problems)

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
