import json
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
    def _raw_frame(self, frame_type, payload, **overrides):
        date = "1936.1.1.12"
        frame = {
            "type": frame_type,
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "sequence": 11,
            "session_sequence": 5,
            "tick_ms": 123456,
            "date": date,
            "hour_index": schema.date_to_hour_index(date),
            "sample_ms": 10,
            "capture_boundary": "worker_poll",
            "domain": "divisions",
            "entity_key": "division:GER:1",
            "source": "C",
            "confidence": "exact",
            "payload": payload,
        }
        frame.update(overrides)
        return frame

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
                "collector_version": "0.1.0",
                "abi_version": 1,
                "sequence": 11,
                "session_sequence": 3,
                "tick_ms": 123456,
                "date": "1936.1.1.12",
                "hour_index": schema.date_to_hour_index("1936.1.1.12"),
                "sample_ms": 10,
                "capture_boundary": "worker_poll",
                "domain": "combats",
                "entity_key": "gap:combats",
                "source": "C",
                "confidence": "gap",
                "segment_id": 1,
                "quality_status": "no_data",
                "payload": {
                    "gap_kind": "dropped_frames",
                    "domain": "combats",
                    "start_sequence": 20,
                    "end_sequence": 23,
                    "reason": "ring_buffer_overflow",
                    "detail": {"dropped": 3},
                },
            },
        ]

        for frame in frames:
            self.assertEqual(validate_collector_frame(frame), [])

    def test_raw_event_frame_validates(self):
        frame = self._raw_frame(
            "event",
            {
                "event_type": "movement_started",
                "key": "division:GER:1",
                "from": None,
                "to": {"province_id": 42},
                "subject": "GER",
                "detail": {"order_generation": 3, "path": [1, 2, 3]},
            },
        )

        self.assertEqual(validate_collector_frame(frame), [])

    def test_raw_event_frame_rejects_malformed_envelope_and_payload(self):
        frame = self._raw_frame(
            "event",
            {
                "event_type": "",
                "key": "0x1234",
                "detail": ["not", "an", "object"],
            },
            source="memory_reader",
            confidence="certain",
            hour_index=True,
            domain="not_a_domain",
            entity_key=42,
        )

        problems = validate_collector_frame(frame)

        for needle in (
            "hour_index not int in range",
            "domain 'not_a_domain' not canonical",
            "entity_key missing or not stable str",
            "source 'memory_reader' != 'C'",
            "confidence 'certain' invalid",
            "event_type missing",
            "key missing or not stable str",
            "detail not object",
        ):
            self.assertTrue(any(needle in p for p in problems), (needle, problems))

    def test_raw_domain_delta_frame_validates(self):
        frame = self._raw_frame(
            "domain_delta",
            {
                "op": "upsert",
                "before": {"province": 41, "progress_bucket": 3},
                "after": {"province": 42, "progress_bucket": 4},
                "changed_fields": ["province", "progress_bucket"],
            },
            domain="combats",
            entity_key="combat:2",
            confidence="sampled",
        )

        self.assertEqual(validate_collector_frame(frame), [])

    def test_raw_division_movement_delta_frame_validates(self):
        frame = self._raw_frame(
            "domain_delta",
            {
                "op": "sample",
                "before": {
                    "div_uid": "100:4713",
                    "div_id": 100,
                    "div_type": 4713,
                    "moving": False,
                    "movement_source_province": None,
                    "move_target_province": None,
                    "location": 3324,
                    "movement_progress": 0.0,
                    "progress_bucket": 0,
                    "owner": None,
                },
                "after": {
                    "div_uid": "100:4713",
                    "div_id": 100,
                    "div_type": 4713,
                    "moving": True,
                    "movement_source_province": 3324,
                    "move_target_province": 3325,
                    "location": 3325,
                    "movement_progress": 6.013,
                    "progress_bucket": 601,
                    "owner": "GER",
                },
                "changed_fields": [
                    "moving",
                    "movement_source_province",
                    "move_target_province",
                    "location",
                    "movement_progress",
                    "progress_bucket",
                ],
            },
            entity_key="division:100:4713",
            confidence="sampled",
        )

        self.assertEqual(validate_collector_frame(frame), [])

    def test_raw_division_movement_delta_expanded_fields_validate(self):
        frame = self._raw_frame(
            "domain_delta",
            {
                "op": "sample",
                "before": {
                    "organisation": None,
                    "strength": None,
                    "combat_ref_count": None,
                    "in_combat": None,
                },
                "after": {
                    "organisation": 84.5,
                    "strength": 9123.25,
                    "combat_ref_count": 2,
                    "in_combat": True,
                },
                "changed_fields": [
                    "organisation",
                    "strength",
                    "combat_ref_count",
                    "in_combat",
                ],
            },
            entity_key="division:100:4713",
            confidence="sampled",
        )

        self.assertEqual(validate_collector_frame(frame), [])

    def test_raw_division_movement_delta_rejects_raw_leakage(self):
        frame = self._raw_frame(
            "domain_delta",
            {
                "op": "sample",
                "before": None,
                "after": {
                    "div_uid": "100:4713",
                    "div_id": 100,
                    "div_type": 4713,
                    "moving": True,
                    "location": 3325,
                    "body_ptr": "0x1234",
                    "diagnostic": {"owner_tag_db_id": 1},
                },
                "changed_fields": ["location", "body_addr"],
            },
            entity_key="division:0x1234",
            confidence="sampled",
        )

        problems = validate_collector_frame(frame)

        for needle in (
            "entity_key not stable division key",
            "payload contains raw pointer/tag-db diagnostic key",
            "changed_fields[1] raw pointer/tag-db diagnostic key",
        ):
            self.assertTrue(any(needle in p for p in problems), (needle, problems))

    def test_raw_division_movement_delta_rejects_bad_numeric_and_tag_fields(self):
        frame = self._raw_frame(
            "domain_delta",
            {
                "op": "sample",
                "before": None,
                "after": {
                    "div_uid": "100:4713",
                    "div_id": True,
                    "div_type": 4713,
                    "moving": 1,
                    "movement_source_province": False,
                    "move_target_province": 0,
                    "location": 1_000_001,
                    "movement_progress": float("inf"),
                    "progress_bucket": True,
                    "owner": "GERM",
                },
                "changed_fields": [
                    "div_id",
                    "moving",
                    "movement_source_province",
                    "move_target_province",
                    "location",
                    "movement_progress",
                    "progress_bucket",
                    "owner",
                ],
            },
            entity_key="division:100:4713",
            confidence="sampled",
        )

        problems = validate_collector_frame(frame)

        for needle in (
            "after.div_id not nonnegative int|null",
            "after.moving not bool|null",
            "after.movement_source_province not positive int|null",
            "after.move_target_province not positive int|null",
            "after.location not positive int|null",
            "after.movement_progress not finite nonnegative number|null in range",
            "after.progress_bucket not nonnegative int|null in range",
            "after.owner not TAG|null",
        ):
            self.assertTrue(any(needle in p for p in problems), (needle, problems))

    def test_raw_division_movement_delta_rejects_bad_expanded_fields(self):
        cases = [
            (
                {"organisation": float("nan")},
                "after.organisation not finite number|null in range",
            ),
            (
                {"strength": float("inf")},
                "after.strength not finite number|null in range",
            ),
            (
                {"combat_ref_count": True},
                "after.combat_ref_count not nonnegative int|null in range",
            ),
            (
                {"in_combat": 1},
                "after.in_combat not bool|null",
            ),
            (
                {"organisation": 1000.01},
                "after.organisation not finite number|null in range",
            ),
            (
                {"strength": 10000.01},
                "after.strength not finite number|null in range",
            ),
            (
                {"combat_ref_count": 1001},
                "after.combat_ref_count not nonnegative int|null in range",
            ),
        ]
        for after, needle in cases:
            with self.subTest(after=after):
                frame = self._raw_frame(
                    "domain_delta",
                    {
                        "op": "sample",
                        "before": None,
                        "after": after,
                        "changed_fields": list(after),
                    },
                    entity_key="division:100:4713",
                    confidence="sampled",
                )

                problems = validate_collector_frame(frame)

                self.assertTrue(
                    any(needle in p for p in problems),
                    (needle, problems),
                )

    def test_raw_land_combat_delta_frame_validates(self):
        frame = self._raw_frame(
            "domain_delta",
            {
                "op": "sample",
                "before": None,
                "after": {
                    "combat_key": "land_combat:7:0",
                    "combat_id": 7,
                    "generation": 0,
                    "location": 1234,
                    "day": 3,
                    "duration": 3,
                    "attacker_divisions": [100, 101],
                    "defender_divisions": None,
                    "attacker_status": "exact",
                    "defender_status": "validation_failed",
                },
                "changed_fields": [
                    "combat_key",
                    "combat_id",
                    "generation",
                    "location",
                    "day",
                    "duration",
                    "attacker_divisions",
                    "defender_divisions",
                    "attacker_status",
                    "defender_status",
                ],
            },
            domain="land_combats",
            entity_key="land_combat:7:0",
            confidence="sampled",
        )

        self.assertEqual(validate_collector_frame(frame), [])

    def test_raw_land_combat_gap_delta_frame_validates(self):
        frame = self._raw_frame(
            "domain_delta",
            {
                "op": "sample",
                "before": None,
                "after": {
                    "gap_key": "land_combats:gap:manager_read_failed",
                    "gap_kind": "manager_read_failed",
                    "status": "validation_failed",
                },
                "changed_fields": ["gap_key", "gap_kind", "status"],
            },
            domain="land_combats",
            entity_key="land_combats:gap:manager_read_failed",
            confidence="gap",
        )

        self.assertEqual(validate_collector_frame(frame), [])

    def test_raw_land_combat_delta_rejects_raw_leakage_and_bad_rows(self):
        frame = self._raw_frame(
            "domain_delta",
            {
                "op": "sample",
                "before": None,
                "after": {
                    "combat_key": "land_combat:8:0",
                    "combat_id": 8,
                    "generation": True,
                    "location": 0,
                    "attacker_divisions": [100, 100, "x"],
                    "defender_divisions": "unknown",
                    "attacker_status": "measured",
                    "body_ptr": "0x1234",
                },
                "changed_fields": ["location", "body_addr"],
            },
            domain="land_combats",
            entity_key="land_combat:7:0",
            confidence="sampled",
        )

        problems = validate_collector_frame(frame)

        for needle in (
            "payload contains raw pointer/tag-db diagnostic key",
            "changed_fields[1] raw pointer/tag-db diagnostic key",
            "after.body_ptr unsupported",
            "after.combat_key != entity_key",
            "after.combat_id != entity_key",
            "after.generation not nonnegative int|null in range",
            "after.location not positive int|null in range",
            "after.attacker_divisions[1] duplicate division id",
            "after.attacker_divisions[2] not positive division id",
            "after.defender_divisions not list|null",
            "after.attacker_status invalid",
        ):
            self.assertTrue(any(needle in p for p in problems), (needle, problems))

    def test_raw_domain_delta_frame_rejects_malformed_payload(self):
        frame = self._raw_frame(
            "domain_delta",
            {
                "op": "replace",
                "changed_fields": ["province", 7],
                "unexpected": True,
            },
        )

        problems = validate_collector_frame(frame)

        for needle in (
            "op 'replace' invalid",
            "after missing",
            "changed_fields[1] not bounded str",
            "key 'unexpected' unsupported",
        ):
            self.assertTrue(any(needle in p for p in problems), (needle, problems))

    def test_raw_gap_frame_validates(self):
        frame = self._raw_frame(
            "gap",
            {
                "gap_kind": "layout_invalid",
                "domain": "combats",
                "start_sequence": 20,
                "end_sequence": 23,
                "reason": "combat_side_validation_failed",
                "detail": [{"side": "attacker", "participants": None}],
            },
            domain="combats",
            entity_key="gap:combats",
            confidence="gap",
        )

        self.assertEqual(validate_collector_frame(frame), [])

    def test_raw_gap_frame_rejects_malformed_payload(self):
        frame = self._raw_frame(
            "gap",
            {
                "gap_kind": "lost",
                "domain": "divisions",
                "start_sequence": 30,
                "end_sequence": 29,
                "reason": "not machine readable",
                "detail": "too loose",
            },
            domain="combats",
            entity_key="gap:combats",
            confidence="gap",
        )

        problems = validate_collector_frame(frame)

        for needle in (
            "gap_kind 'lost' invalid",
            "domain != raw frame domain",
            "start_sequence > end_sequence",
            "reason missing or not machine-readable str",
            "detail not object|list",
        ):
            self.assertTrue(any(needle in p for p in problems), (needle, problems))

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

    def test_snapshot_frame_folds_country_misc(self):
        frame = {
            "type": "snapshot",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "date": "1936.1.2.1",
            "domains": ["clock", "country_identity", "country_misc"],
            "countries": {
                "GER": {
                    "present": True,
                    "collector_object": "0x1234",
                    "identity": {"tag_db_id": 1},
                    "misc": {
                        "convoys_destroyed": 12,
                        "accidents_score": 0.5,
                        "ace_count": 3,
                        "cosmetic_tag": None,
                    },
                },
                "SOV": {
                    "present": True,
                    "collector_object": "0x2000",
                    "identity": {"tag_db_id": 2},
                    "misc": {
                        "convoys_destroyed": 0,
                        "accidents_score": 0.0,
                        "ace_count": 0,
                        "cosmetic_tag": "SOV_communist",
                    },
                },
            },
        }

        snap = snapshot_from_collector_frame(frame, "collector-test")

        self.assertEqual(schema.validate_snapshot(snap), [])
        self.assertEqual(
            snap["provenance"]["collector"]["domains"],
            ["clock", "country_identity", "country_misc"],
        )
        ger = snap["countries"]["GER"]["misc"]
        self.assertEqual(ger["convoys_destroyed"], 12)
        self.assertEqual(ger["accidents_score"], 0.5)
        self.assertEqual(ger["ace_count"], 3)
        self.assertIn("cosmetic_tag", ger)
        self.assertIsNone(ger["cosmetic_tag"])
        sov = snap["countries"]["SOV"]["misc"]
        self.assertEqual(sov["convoys_destroyed"], 0)
        self.assertEqual(sov["accidents_score"], 0.0)
        self.assertEqual(sov["ace_count"], 0)
        self.assertEqual(sov["cosmetic_tag"], "SOV_communist")

    def test_root_probe_frame_derives_country_misc_domain(self):
        frame = {
            "type": "collector_started",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "process_id": 32440,
            "tick_ms": 158373765,
            "quality_status": "root_probe_verified",
            "countries": {
                "GER": {
                    "present": True,
                    "collector_object": "0x1234",
                    "identity": {"tag_db_id": 1},
                    "misc": {"cosmetic_tag": None},
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
                "domains": ["clock", "country_identity"],
            },
        }

        snap = snapshot_from_root_probe_frame(frame, "collector-test")

        self.assertEqual(schema.validate_snapshot(snap), [])
        self.assertEqual(
            snap["provenance"]["collector"]["domains"],
            ["clock", "country_identity", "country_misc"],
        )
        self.assertIn("cosmetic_tag", snap["countries"]["GER"]["misc"])
        self.assertIsNone(snap["countries"]["GER"]["misc"]["cosmetic_tag"])

    def test_country_misc_rejects_malformed_values(self):
        frame = {
            "type": "snapshot",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "date": "1936.1.2.1",
            "domains": ["clock", "country_identity", "country_misc"],
            "countries": {
                "GER": {
                    "present": True,
                    "collector_object": "0x1234",
                    "identity": {"tag_db_id": 1},
                    "misc": {
                        "convoys_destroyed": -1,
                        "accidents_score": float("inf"),
                        "ace_count": True,
                        "cosmetic_tag": "bad tag!",
                    },
                },
            },
        }

        problems = validate_collector_frame(frame)

        for needle in (
            "misc.convoys_destroyed",
            "misc.accidents_score",
            "misc.ace_count",
            "misc.cosmetic_tag",
        ):
            self.assertTrue(any(needle in p for p in problems), (needle, problems))
        with self.assertRaisesRegex(ValueError, "misc.convoys_destroyed"):
            snapshot_from_collector_frame(frame, "collector-test")

    def test_country_scalars_reject_bool_and_non_finite_numbers(self):
        frame = {
            "type": "snapshot",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "date": "1936.1.2.1",
            "domains": ["clock", "country_identity", "country_scalars"],
            "countries": {
                "POR": {
                    "present": True,
                    "collector_object": "0x1234",
                    "identity": {"tag_db_id": 47},
                    "scalars": {
                        "stability": True,
                        "political_power": float("nan"),
                        "command_power": float("inf"),
                        "num_ships": True,
                    },
                },
            },
        }

        problems = validate_collector_frame(frame)

        for needle in (
            "scalars.stability",
            "scalars.political_power",
            "scalars.command_power",
            "scalars.num_ships",
        ):
            self.assertTrue(any(needle in p for p in problems), (needle, problems))
        with self.assertRaisesRegex(ValueError, "scalars.stability"):
            snapshot_from_collector_frame(frame, "collector-test")

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
            "capture_boundary": "scheduled_hour_snapshot",
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
            "capture_boundary": "scheduled_hour_snapshot",
            "domains": ["clock"],
            "states": {"1": {"controller": "GER"}},
            "provinces": {"11": {"controller": "GER"}},
        }

        snap = snapshot_from_collector_frame(frame, "collector-test")

        self.assertEqual(schema.validate_snapshot(snap), [])
        self.assertEqual(snap["states"], {})
        self.assertEqual(snap["provinces"], {})
        self.assertIsNone(snap["map"]["control_by_province"])

    def test_snapshot_frame_rejects_non_scheduled_map_control_payload(self):
        frame = {
            "type": "snapshot",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "date": "1936.1.2.1",
            "capture_boundary": "daily_update",
            "domains": ["clock", "state_control", "province_control"],
            "states": {"1": {"controller": "GER"}},
            "provinces": {"11": {"controller": "GER"}},
        }

        problems = validate_collector_frame(frame)

        self.assertTrue(any("map-control payload requires scheduled" in p
                            for p in problems), problems)
        with self.assertRaisesRegex(ValueError,
                                    "map-control payload requires scheduled"):
            snapshot_from_collector_frame(frame, "collector-test")

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
                    "fuel": {
                        "current": 123.5,
                        "max": 456.25,
                        "gain": 7.5,
                        "cost": -2.25,
                    },
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
        self.assertEqual(snap["countries"]["POR"]["economy"]["fuel"]["gain"], 7.5)
        self.assertEqual(snap["countries"]["POR"]["economy"]["fuel"]["cost"], -2.25)

    def test_snapshot_frame_rejects_non_finite_or_bool_fuel_live(self):
        bad_values = (True, float("nan"), float("inf"))
        for value in bad_values:
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
                        "fuel": {"current": value, "max": 456.25},
                    },
                },
            }

            problems = validate_collector_frame(frame)

            self.assertTrue(
                any("fuel.current" in p and "not number|null" in p for p in problems),
                (value, problems),
            )
            with self.assertRaisesRegex(ValueError, "fuel.current"):
                snapshot_from_collector_frame(frame, "collector-test")

    def test_snapshot_frame_rejects_non_finite_fuel_gain_and_cost(self):
        bad_cases = (
            ("gain", float("nan")),
            ("cost", float("inf")),
            ("cost", True),
        )
        for key, value in bad_cases:
            with self.subTest(key=key, value=value):
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
                            "fuel": {"current": 123.5, "max": 456.25, key: value},
                        },
                    },
                }

                problems = validate_collector_frame(frame)

                self.assertTrue(
                    any(f"fuel.{key}" in p and "not number|null" in p
                        for p in problems),
                    (key, value, problems),
                )
                with self.assertRaisesRegex(ValueError, f"fuel.{key}"):
                    snapshot_from_collector_frame(frame, "collector-test")

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
            "capture_boundary": "scheduled_hour_snapshot",
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
                "domains": ["clock"],
            },
        }

        snap = snapshot_from_root_probe_frame(frame, "collector-test")

        self.assertEqual(schema.validate_snapshot(snap), [])
        self.assertEqual(snap["provenance"]["collector"]["domains"], ["clock"])
        self.assertEqual(snap["states"], {})
        self.assertEqual(snap["provinces"], {})

    def test_root_probe_rejects_non_scheduled_map_control_claims(self):
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
                "daily_update_sentinel_match": True,
                "long_string_sentinel_match": True,
                "current_game_date": "1936.6.15.7",
                "current_game_date_raw": 60763326,
                "start_date": "1936.1.1.12",
                "start_date_raw": 60759371,
                "country_tag_count": 474,
                "country_active_count": 440,
                "tag_sample": ["---", "GER", "ENG"],
                "domains": ["clock", "state_control"],
            },
        }

        problems = validate_collector_frame(frame)

        self.assertTrue(any("map-control payload requires scheduled" in p
                            for p in problems), problems)
        with self.assertRaisesRegex(ValueError,
                                    "map-control payload requires scheduled"):
            snapshot_from_root_probe_frame(frame, "collector-test")

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

    def test_worker_poll_rejects_heavy_domain_claims_without_payload(self):
        frame = {
            "type": "snapshot",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "date": "1936.1.2.1",
            "capture_boundary": "worker_poll",
            "domains": ["clock", "fuel_live", "production_live", "state_control"],
        }

        problems = validate_collector_frame(frame)

        self.assertTrue(any("worker_poll frame must not claim scheduled domains"
                            in p for p in problems), problems)
        with self.assertRaisesRegex(ValueError, "must not claim scheduled domains"):
            snapshot_from_collector_frame(frame, "collector-test")

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

    def test_jsonl_reader_rejects_malformed_raw_frame(self):
        import tempfile

        frame = self._raw_frame(
            "domain_delta",
            {
                "op": "replace",
                "changed_fields": ["province"],
            },
        )
        with tempfile.TemporaryDirectory() as td:
            path = os.path.join(td, "collector_frames.jsonl")
            with open(path, "w", encoding="utf-8") as f:
                f.write(json.dumps(frame) + "\n")
            with self.assertRaisesRegex(ValueError, "op 'replace' invalid"):
                read_collector_jsonl(path)


if __name__ == "__main__":
    unittest.main()
