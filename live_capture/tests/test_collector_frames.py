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

    def _special_projects_payload(self):
        return {
            "facilities": [{
                "id": 1,
                "province": 118,
                "building": "special_project_facility",
                "active_project_handle": 9001,
                "active": True,
                "scientist_char_id": 44,
                "unread_prototype_rewards": 0,
            }],
            "projects": [{
                "id": 77,
                "static_def_handle": 9001,
                "iterations": 2,
                "worked_at_facility": 1,
            }],
            "scientists": [{
                "char_id": 44,
                "name_token": "POR_scientist_a",
                "display_name": "Test Scientist",
                "skills": {
                    "specialization_nuclear": 1,
                    "specialization_naval": 0,
                    "specialization_air": 2,
                    "specialization_land": 3,
                },
                "trait_count": 1,
            }],
            "count": {"facilities": 1, "projects": 1, "scientists": 1},
            "source": "C",
        }

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
                "key": "combat:2",
                "from": None,
                "to": {"province_id": 42},
                "subject": "GER",
                "detail": {"order_generation": 3, "path": [1, 2, 3]},
            },
            domain="combats",
            entity_key="combat:2",
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

    def test_raw_division_event_frame_validates(self):
        frame = self._raw_frame(
            "event",
            {
                "event_type": "division_org_strength_delta",
                "key": "division:100:4713",
                "subject": "division",
                "detail": {
                    "div_uid": "100:4713",
                    "div_id": 100,
                    "div_type": 4713,
                    "moving": True,
                    "movement_source_province": 3324,
                    "move_target_province": 3325,
                    "location": 3325,
                    "movement_progress": 6.013,
                    "progress_bucket": 601,
                    "organisation": 84.5,
                    "strength": 91.25,
                    "combat_ref_count": 2,
                    "in_combat": True,
                    "changed_fields": ["organisation", "strength"],
                },
            },
            entity_key="division:100:4713",
            confidence="sampled",
        )

        self.assertEqual(validate_collector_frame(frame), [])

    def test_raw_division_event_rejects_bad_detail(self):
        frame = self._raw_frame(
            "event",
            {
                "event_type": "division_order_changed",
                "key": "division:100:4713",
                "subject": "unit",
                "detail": {
                    "div_uid": "101:4713",
                    "div_id": 101,
                    "div_type": 4713,
                    "body_ptr": "0x1234",
                    "changed_fields": ["organisation", "body_addr"],
                },
            },
            entity_key="division:100:4713",
            confidence="sampled",
        )

        problems = validate_collector_frame(frame)

        for needle in (
            "event_type 'division_order_changed' invalid",
            "subject != 'division'",
            "payload contains raw pointer/tag-db diagnostic key",
            "detail.body_ptr unsupported",
            "detail.div_uid != entity_key",
            "detail.div_id != entity_key",
            "detail.changed_fields[1] invalid",
        ):
            self.assertTrue(any(needle in p for p in problems), (needle, problems))

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

    def test_raw_land_combat_lifecycle_event_frame_validates(self):
        frame = self._raw_frame(
            "event",
            {
                "event_type": "land_combat_started",
                "key": "land_combat:7:0",
                "subject": "land_combat",
                "detail": {
                    "combat_key": "land_combat:7:0",
                    "combat_id": 7,
                    "generation": 0,
                    "location": 1234,
                    "day": 3,
                    "duration": 3,
                    "attacker_divisions": [100, 101],
                    "defender_divisions": [200],
                    "attacker_status": "exact",
                    "defender_status": "exact",
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
            },
            domain="land_combats",
            entity_key="land_combat:7:0",
            confidence="sampled",
        )

        self.assertEqual(validate_collector_frame(frame), [])

    def test_raw_land_combat_ended_event_frame_validates(self):
        frame = self._raw_frame(
            "event",
            {
                "event_type": "land_combat_ended",
                "key": "land_combat:7:0",
                "subject": "land_combat",
                "detail": {
                    "combat_key": "land_combat:7:0",
                    "combat_id": 7,
                    "generation": 0,
                    "location": 1234,
                    "day": 3,
                    "duration": 3,
                    "attacker_divisions": [100],
                    "defender_divisions": [200],
                    "attacker_status": "exact",
                    "defender_status": "exact",
                    "changed_fields": ["combat_key"],
                    "end_reason": "resolved",
                },
            },
            domain="land_combats",
            entity_key="land_combat:7:0",
            confidence="sampled",
        )

        self.assertEqual(validate_collector_frame(frame), [])

    def test_raw_land_combat_lifecycle_event_rejects_bad_detail(self):
        frame = self._raw_frame(
            "event",
            {
                "event_type": "land_combat_finished",
                "key": "land_combat:8:0",
                "subject": "combat",
                "detail": {
                    "combat_key": "land_combat:8:0",
                    "combat_id": 8,
                    "generation": 0,
                    "location": 0,
                    "attacker_divisions": [100, 100],
                    "defender_divisions": "unknown",
                    "attacker_status": "measured",
                    "changed_fields": ["location", "body_addr"],
                    "end_reason": "because",
                    "body_ptr": "0x1234",
                },
            },
            domain="land_combats",
            entity_key="land_combat:7:0",
            confidence="sampled",
        )

        problems = validate_collector_frame(frame)

        for needle in (
            "payload key != entity_key",
            "event_type 'land_combat_finished' invalid",
            "subject != 'land_combat'",
            "payload contains raw pointer/tag-db diagnostic key",
            "detail.body_ptr unsupported",
            "detail.combat_key != entity_key",
            "detail.combat_id != entity_key",
            "detail.location not positive int|null in range",
            "detail.attacker_divisions[1] duplicate division id",
            "detail.defender_divisions not list|null",
            "detail.attacker_status invalid",
            "detail.changed_fields[1] invalid",
            "detail.end_reason set for non-ended event",
        ):
            self.assertTrue(any(needle in p for p in problems), (needle, problems))

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

    def test_raw_army_order_delta_frame_validates(self):
        frame = self._raw_frame(
            "domain_delta",
            {
                "op": "sample",
                "before": None,
                "after": {
                    "order_key": "army_order:17:4",
                    "army_id": 17,
                    "generation": 4,
                    "slot_index": 2,
                    "order_type": 5,
                    "target_states": [107, 108],
                    "target_state_count": 2,
                    "target_status": "exact",
                },
                "changed_fields": [
                    "order_key",
                    "army_id",
                    "generation",
                    "slot_index",
                    "order_type",
                    "target_states",
                    "target_state_count",
                    "target_status",
                ],
            },
            domain="army_orders",
            entity_key="army_order:17:4",
            confidence="sampled",
        )

        self.assertEqual(validate_collector_frame(frame), [])

    def test_raw_army_order_lifecycle_event_frame_validates(self):
        frame = self._raw_frame(
            "event",
            {
                "event_type": "army_order_added",
                "key": "army_order:17:4",
                "subject": "army_order",
                "detail": {
                    "order_key": "army_order:17:4",
                    "army_id": 17,
                    "generation": 4,
                    "slot_index": 2,
                    "order_type": 5,
                    "target_states": [107, 108],
                    "target_state_count": 2,
                    "target_status": "exact",
                    "changed_fields": [
                        "order_key",
                        "army_id",
                        "generation",
                        "slot_index",
                        "order_type",
                        "target_states",
                        "target_state_count",
                        "target_status",
                    ],
                },
            },
            domain="army_orders",
            entity_key="army_order:17:4",
            confidence="sampled",
        )

        self.assertEqual(validate_collector_frame(frame), [])

    def test_raw_army_order_gap_delta_frame_validates(self):
        frame = self._raw_frame(
            "domain_delta",
            {
                "op": "sample",
                "before": None,
                "after": {
                    "gap_key": "army_orders:gap:bridge_entry_invalid",
                    "gap_kind": "bridge_entry_invalid",
                    "status": "validation_failed",
                },
                "changed_fields": ["gap_key", "gap_kind", "status"],
            },
            domain="army_orders",
            entity_key="army_orders:gap:bridge_entry_invalid",
            confidence="gap",
        )

        self.assertEqual(validate_collector_frame(frame), [])

    def test_raw_army_order_delta_rejects_raw_leakage_and_bad_rows(self):
        frame = self._raw_frame(
            "domain_delta",
            {
                "op": "sample",
                "before": None,
                "after": {
                    "order_key": "army_order:18:4",
                    "army_id": 18,
                    "generation": True,
                    "slot_index": 999,
                    "order_type": 10001,
                    "target_states": [107, 107, 0, 70000],
                    "target_state_count": 2,
                    "target_status": "measured",
                    "order_addr": "0x1234",
                },
                "changed_fields": ["target_states", "body_addr"],
            },
            domain="army_orders",
            entity_key="army_order:17:4",
            confidence="sampled",
        )

        problems = validate_collector_frame(frame)

        for needle in (
            "payload contains raw pointer/tag-db diagnostic key",
            "changed_fields[1] raw pointer/tag-db diagnostic key",
            "after.order_addr unsupported",
            "after.order_key != entity_key",
            "after.army_id != entity_key",
            "after.generation not nonnegative int|null in range",
            "after.slot_index not nonnegative int|null in range",
            "after.order_type not bounded int|null",
            "after.target_states[1] duplicate state id",
            "after.target_states[2] not positive state id",
            "after.target_states[3] not positive state id",
            "after.target_state_count != target_states length",
            "after.target_status invalid",
        ):
            self.assertTrue(any(needle in p for p in problems), (needle, problems))

    def test_raw_naval_combat_delta_frame_validates(self):
        frame = self._raw_frame(
            "domain_delta",
            {
                "op": "sample",
                "before": None,
                "after": {
                    "combat_key": "naval_combat:241671:0",
                    "combat_id": 241671,
                    "generation": 0,
                    "location": 1234,
                    "attacker_tags": ["ENG", "POR"],
                    "defender_tags": ["ITA"],
                    "attacker_ship_count": 12,
                    "defender_ship_count": 4,
                    "attacker_commander": "eng_admiral",
                    "defender_commander": None,
                    "attacker_positioning": 0.87,
                    "defender_positioning": None,
                    "attacker_status": "exact",
                    "defender_status": "validation_failed",
                },
                "changed_fields": [
                    "combat_key",
                    "combat_id",
                    "generation",
                    "location",
                    "attacker_tags",
                    "defender_tags",
                    "attacker_ship_count",
                    "defender_ship_count",
                    "attacker_commander",
                    "defender_commander",
                    "attacker_positioning",
                    "defender_positioning",
                    "attacker_status",
                    "defender_status",
                ],
            },
            domain="naval_air_combats",
            entity_key="naval_combat:241671:0",
            confidence="sampled",
        )

        self.assertEqual(validate_collector_frame(frame), [])

    def test_raw_naval_combat_lifecycle_event_frame_validates(self):
        frame = self._raw_frame(
            "event",
            {
                "event_type": "naval_combat_started",
                "key": "naval_combat:241671:0",
                "subject": "naval_combat",
                "detail": {
                    "combat_key": "naval_combat:241671:0",
                    "combat_id": 241671,
                    "generation": 0,
                    "location": 1234,
                    "attacker_tags": ["ENG", "POR"],
                    "defender_tags": ["ITA"],
                    "attacker_ship_count": 12,
                    "defender_ship_count": 4,
                    "attacker_commander": "eng_admiral",
                    "defender_commander": "ita_admiral",
                    "attacker_positioning": 0.87,
                    "defender_positioning": 0.45,
                    "attacker_status": "exact",
                    "defender_status": "exact",
                    "changed_fields": [
                        "combat_key",
                        "combat_id",
                        "generation",
                        "location",
                        "attacker_tags",
                        "defender_tags",
                        "attacker_ship_count",
                        "defender_ship_count",
                        "attacker_commander",
                        "defender_commander",
                        "attacker_positioning",
                        "defender_positioning",
                        "attacker_status",
                        "defender_status",
                    ],
                },
            },
            domain="naval_air_combats",
            entity_key="naval_combat:241671:0",
            confidence="sampled",
        )

        self.assertEqual(validate_collector_frame(frame), [])

    def test_raw_naval_combat_ended_event_frame_validates(self):
        frame = self._raw_frame(
            "event",
            {
                "event_type": "naval_combat_ended",
                "key": "naval_combat:241671:0",
                "subject": "naval_combat",
                "detail": {
                    "combat_key": "naval_combat:241671:0",
                    "combat_id": 241671,
                    "generation": 0,
                    "location": 1234,
                    "attacker_tags": ["ENG"],
                    "defender_tags": ["ITA"],
                    "attacker_ship_count": 10,
                    "defender_ship_count": 0,
                    "attacker_commander": "eng_admiral",
                    "defender_commander": "ita_admiral",
                    "attacker_positioning": 0.92,
                    "defender_positioning": 0.0,
                    "attacker_status": "exact",
                    "defender_status": "exact",
                    "changed_fields": ["attacker_ship_count", "defender_ship_count"],
                    "end_reason": "resolved",
                },
            },
            domain="naval_air_combats",
            entity_key="naval_combat:241671:0",
            confidence="sampled",
        )

        self.assertEqual(validate_collector_frame(frame), [])

    def test_raw_naval_combat_gap_delta_frame_validates(self):
        frame = self._raw_frame(
            "domain_delta",
            {
                "op": "sample",
                "before": None,
                "after": {
                    "gap_key": "naval_air_combats:gap:bridge_entry_invalid",
                    "gap_kind": "bridge_entry_invalid",
                    "status": "validation_failed",
                },
                "changed_fields": ["gap_key", "gap_kind", "status"],
            },
            domain="naval_air_combats",
            entity_key="naval_air_combats:gap:bridge_entry_invalid",
            confidence="gap",
        )

        self.assertEqual(validate_collector_frame(frame), [])

    def test_raw_naval_combat_delta_rejects_raw_leakage_and_bad_rows(self):
        frame = self._raw_frame(
            "domain_delta",
            {
                "op": "sample",
                "before": None,
                "after": {
                    "combat_key": "naval_combat:241672:0",
                    "combat_id": 241672,
                    "generation": True,
                    "location": 0,
                    "attacker_tags": ["ENG", "ENG", "GERM"],
                    "defender_tags": "unknown",
                    "attacker_ship_count": -1,
                    "defender_ship_count": 20001,
                    "attacker_positioning": float("inf"),
                    "defender_positioning": 100.01,
                    "attacker_status": "measured",
                    "body_ptr": "0x1234",
                },
                "changed_fields": ["location", "body_addr"],
            },
            domain="naval_air_combats",
            entity_key="naval_combat:241671:0",
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
            "after.attacker_tags[1] duplicate TAG",
            "after.attacker_tags[2] not valid TAG",
            "after.defender_tags not list|null",
            "after.attacker_ship_count not nonnegative int|null in range",
            "after.defender_ship_count not nonnegative int|null in range",
            "after.attacker_positioning not finite number|null in range",
            "after.defender_positioning not finite number|null in range",
            "after.attacker_status invalid",
        ):
            self.assertTrue(any(needle in p for p in problems), (needle, problems))

    def test_raw_naval_combat_member_delta_frame_validates(self):
        member_key = (
            "naval_combat_members:naval_combat:241671:3:attacker:ship:3458"
        )
        frame = self._raw_frame(
            "domain_delta",
            {
                "op": "sample",
                "before": None,
                "after": {
                    "member_key": member_key,
                    "combat_key": "naval_combat:241671:3",
                    "combat_id": 241671,
                    "generation": 3,
                    "side": "attacker",
                    "group_index": 0,
                    "member_index": 2,
                    "unit_type_tag": 51,
                    "ship_instance_id": 3458,
                    "is_convoy": False,
                    "owner": "FRA",
                    "name": "Sidi-Ferruch",
                    "type_display": "Cruiser Submarine",
                    "archetype": "submarine",
                    "designed_max_strength_raw": 49_000_000,
                    "battle_slot_index_raw": 4,
                    "state_bitmask": 4,
                    "engagement_weight_raw": 100_000,
                },
                "changed_fields": [
                    "member_key",
                    "combat_key",
                    "combat_id",
                    "generation",
                    "side",
                    "group_index",
                    "member_index",
                    "unit_type_tag",
                    "ship_instance_id",
                    "is_convoy",
                    "owner",
                    "name",
                    "type_display",
                    "archetype",
                    "designed_max_strength_raw",
                    "battle_slot_index_raw",
                    "state_bitmask",
                    "engagement_weight_raw",
                ],
            },
            domain="naval_combat_members",
            entity_key=member_key,
            confidence="sampled",
        )

        self.assertEqual(validate_collector_frame(frame), [])

    def test_raw_naval_combat_member_convoy_delta_frame_validates(self):
        member_key = (
            "naval_combat_members:naval_combat:241671:3:defender:convoy:0:12:19"
        )
        frame = self._raw_frame(
            "domain_delta",
            {
                "op": "sample",
                "before": None,
                "after": {
                    "member_key": member_key,
                    "combat_key": "naval_combat:241671:3",
                    "combat_id": 241671,
                    "generation": 3,
                    "side": "defender",
                    "group_index": 0,
                    "member_index": 12,
                    "unit_type_tag": 0,
                    "ship_instance_id": None,
                    "is_convoy": True,
                    "owner": "JAP",
                    "name": None,
                    "type_display": "Convoy",
                    "archetype": "convoy",
                    "designed_max_strength_raw": 0,
                    "battle_slot_index_raw": 19,
                    "state_bitmask": 8,
                    "engagement_weight_raw": 0,
                },
                "changed_fields": ["state_bitmask", "engagement_weight_raw"],
            },
            domain="naval_combat_members",
            entity_key=member_key,
            confidence="sampled",
        )

        self.assertEqual(validate_collector_frame(frame), [])

    def test_raw_naval_combat_member_gap_delta_frame_validates(self):
        frame = self._raw_frame(
            "domain_delta",
            {
                "op": "sample",
                "before": None,
                "after": {
                    "gap_key": "naval_combat_members:gap:member_entry_invalid",
                    "gap_kind": "member_entry_invalid",
                    "status": "validation_failed",
                },
                "changed_fields": ["gap_key", "gap_kind", "status"],
            },
            domain="naval_combat_members",
            entity_key="naval_combat_members:gap:member_entry_invalid",
            confidence="gap",
        )

        self.assertEqual(validate_collector_frame(frame), [])

    def test_raw_naval_combat_member_delta_rejects_raw_leakage_and_bad_rows(self):
        frame = self._raw_frame(
            "domain_delta",
            {
                "op": "sample",
                "before": None,
                "after": {
                    "member_key": (
                        "naval_combat_members:naval_combat:241671:3:"
                        "attacker:ship:3459"
                    ),
                    "combat_key": "naval_combat:241672:3",
                    "combat_id": 241672,
                    "generation": True,
                    "side": "left",
                    "group_index": 257,
                    "member_index": 1001,
                    "unit_type_tag": 0,
                    "ship_instance_id": 9999,
                    "is_convoy": True,
                    "owner": "FRAA",
                    "name": "",
                    "designed_max_strength_raw": 1_000_000_001,
                    "battle_slot_index_raw": 20_001,
                    "state_bitmask": 256,
                    "engagement_weight_raw": 10_000_001,
                    "member_addr": "0x1234",
                    "owner_tag_db_id": 5,
                },
                "changed_fields": ["state_bitmask", "member_addr"],
            },
            domain="naval_combat_members",
            entity_key="naval_combat_members:naval_combat:241671:3:attacker:ship:3458",
            confidence="sampled",
        )

        problems = validate_collector_frame(frame)

        for needle in (
            "payload contains raw pointer/tag-db diagnostic key",
            "changed_fields[1] raw pointer/tag-db diagnostic key",
            "after.member_addr unsupported",
            "after.owner_tag_db_id unsupported",
            "after.member_key != entity_key",
            "after.combat_key != member_key",
            "after.combat_id != member_key",
            "after.generation not nonnegative int|null in range",
            "after.side invalid",
            "after.group_index not nonnegative int|null in range",
            "after.member_index not nonnegative int|null in range",
            "after.unit_type_tag invalid for ship",
            "after.ship_instance_id != member_key",
            "after.is_convoy true for ship key",
            "after.owner not TAG|null",
            "after.name not bounded str|null",
            "after.designed_max_strength_raw not nonnegative int|null in range",
            "after.battle_slot_index_raw not nonnegative int|null in range",
            "after.state_bitmask not nonnegative int|null in range",
            "after.engagement_weight_raw not nonnegative int|null in range",
        ):
            self.assertTrue(any(needle in p for p in problems), (needle, problems))

    def test_raw_air_wing_delta_frame_validates(self):
        frame = self._raw_frame(
            "domain_delta",
            {
                "op": "sample",
                "before": None,
                "after": {
                    "air_wing_key": "air_wing:POR:3:0",
                    "owner": "POR",
                    "wing_id": 3,
                    "generation": 0,
                    "count": 96,
                    "mission_mask": 1,
                    "mission_region": 254,
                    "effective_planes": 83,
                },
                "changed_fields": [
                    "air_wing_key",
                    "owner",
                    "wing_id",
                    "generation",
                    "count",
                    "mission_mask",
                    "mission_region",
                    "effective_planes",
                ],
            },
            domain="air_wings",
            entity_key="air_wing:POR:3:0",
            confidence="sampled",
        )

        self.assertEqual(validate_collector_frame(frame), [])

    def test_raw_air_wing_lifecycle_event_frame_validates(self):
        frame = self._raw_frame(
            "event",
            {
                "event_type": "air_wing_updated",
                "key": "air_wing:POR:3:0",
                "subject": "air_wing",
                "detail": {
                    "air_wing_key": "air_wing:POR:3:0",
                    "owner": "POR",
                    "wing_id": 3,
                    "generation": 0,
                    "count": 96,
                    "mission_mask": 5,
                    "mission_region": None,
                    "effective_planes": 0,
                    "changed_fields": ["mission_mask", "mission_region", "effective_planes"],
                },
            },
            domain="air_wings",
            entity_key="air_wing:POR:3:0",
            confidence="sampled",
        )

        self.assertEqual(validate_collector_frame(frame), [])

    def test_raw_air_wing_gap_delta_frame_validates(self):
        frame = self._raw_frame(
            "domain_delta",
            {
                "op": "sample",
                "before": None,
                "after": {
                    "gap_key": "air_wings:gap:bridge_entry_invalid",
                    "gap_kind": "bridge_entry_invalid",
                    "status": "validation_failed",
                },
                "changed_fields": ["gap_key", "gap_kind", "status"],
            },
            domain="air_wings",
            entity_key="air_wings:gap:bridge_entry_invalid",
            confidence="gap",
        )

        self.assertEqual(validate_collector_frame(frame), [])

    def test_raw_air_wing_delta_rejects_raw_leakage_and_bad_rows(self):
        frame = self._raw_frame(
            "domain_delta",
            {
                "op": "sample",
                "before": None,
                "after": {
                    "air_wing_key": "air_wing:ENG:3:0",
                    "owner": "ENG",
                    "wing_id": 3,
                    "generation": True,
                    "count": -1,
                    "mission_mask": 4_294_967_296,
                    "mission_region": 0,
                    "effective_planes": 10_000_001,
                    "body_ptr": "0x1234",
                },
                "changed_fields": ["count", "tag_db_id"],
            },
            domain="air_wings",
            entity_key="air_wing:POR:3:0",
            confidence="sampled",
        )

        problems = validate_collector_frame(frame)

        for needle in (
            "payload contains raw pointer/tag-db diagnostic key",
            "changed_fields[1] raw pointer/tag-db diagnostic key",
            "after.body_ptr unsupported",
            "after.air_wing_key != entity_key",
            "after.owner != entity_key",
            "after.generation not nonnegative int|null in range",
            "after.count not nonnegative int|null in range",
            "after.mission_mask not nonnegative u32|null",
            "after.mission_region not positive int|null in range",
            "after.effective_planes not nonnegative int|null in range",
        ):
            self.assertTrue(any(needle in p for p in problems), (needle, problems))

    def test_raw_ship_delta_frame_validates(self):
        frame = self._raw_frame(
            "domain_delta",
            {
                "op": "sample",
                "before": None,
                "after": {
                    "ship_key": "ship:3458",
                    "instance_id": 3458,
                    "definition": "submarine_2",
                    "category_enum": 2,
                    "category": "submarine",
                    "strength_raw": 3_997_607,
                    "strength_hp": 39.97607,
                    "organisation_raw": 5_420_000,
                    "organisation": 54.2,
                    "experience_raw": 9_936_256,
                    "experience": 99.36256,
                    "sunk_convoys": 0,
                },
                "changed_fields": [
                    "ship_key",
                    "instance_id",
                    "definition",
                    "category_enum",
                    "category",
                    "strength_raw",
                    "strength_hp",
                    "organisation_raw",
                    "organisation",
                    "experience_raw",
                    "experience",
                    "sunk_convoys",
                ],
            },
            domain="ships",
            entity_key="ship:3458",
            confidence="sampled",
        )

        self.assertEqual(validate_collector_frame(frame), [])

    def test_raw_ship_gap_delta_frame_validates(self):
        frame = self._raw_frame(
            "domain_delta",
            {
                "op": "sample",
                "before": None,
                "after": {
                    "gap_key": "ships:gap:bridge_entry_invalid",
                    "gap_kind": "bridge_entry_invalid",
                    "status": "validation_failed",
                },
                "changed_fields": ["gap_key", "gap_kind", "status"],
            },
            domain="ships",
            entity_key="ships:gap:bridge_entry_invalid",
            confidence="gap",
        )

        self.assertEqual(validate_collector_frame(frame), [])

    def test_raw_ship_delta_rejects_raw_leakage_and_bad_rows(self):
        frame = self._raw_frame(
            "domain_delta",
            {
                "op": "sample",
                "before": None,
                "after": {
                    "ship_key": "ship:3459",
                    "instance_id": 3459,
                    "definition": "0x1234",
                    "category_enum": 4,
                    "category": "bad_category",
                    "strength_raw": 1_000_000_001,
                    "strength_hp": 10_000.1,
                    "organisation_raw": 100_000_001,
                    "organisation": 1000.1,
                    "experience_raw": 100_000_001,
                    "experience": 1000.1,
                    "sunk_convoys": 100_000_001,
                    "body_ptr": "0x1234",
                },
                "changed_fields": ["strength_raw", "body_ptr"],
            },
            domain="ships",
            entity_key="ship:3458",
            confidence="sampled",
        )

        problems = validate_collector_frame(frame)

        for needle in (
            "payload contains raw pointer/tag-db diagnostic key",
            "changed_fields[1] raw pointer/tag-db diagnostic key",
            "after.body_ptr unsupported",
            "after.ship_key != entity_key",
            "after.instance_id != entity_key",
            "after.definition looks like pointer",
            "after.category_enum invalid",
            "after.category invalid",
            "after.strength_raw not nonnegative int|null in range",
            "after.strength_hp not finite number|null in range",
            "after.organisation_raw not nonnegative int|null in range",
            "after.organisation not finite number|null in range",
            "after.experience_raw not nonnegative int|null in range",
            "after.experience not finite number|null in range",
            "after.sunk_convoys not nonnegative int|null in range",
        ):
            self.assertTrue(any(needle in p for p in problems), (needle, problems))

    def test_raw_oob_command_fleet_delta_frame_validates(self):
        frame = self._raw_frame(
            "domain_delta",
            {
                "op": "sample",
                "before": None,
                "after": {
                    "oob_key": "oob_command:fleet:61500",
                    "entity_kind": "fleet",
                    "fleet_id": 61500,
                    "task_force_id": None,
                    "task_force_ids": [61700],
                    "task_force_count": 1,
                    "task_force_status": "exact",
                    "parent_status": None,
                    "ship_ids": None,
                    "ship_count": None,
                    "ship_status": None,
                },
                "changed_fields": [
                    "oob_key",
                    "entity_kind",
                    "fleet_id",
                    "task_force_ids",
                    "task_force_count",
                    "task_force_status",
                ],
            },
            domain="oob_command",
            entity_key="oob_command:fleet:61500",
            confidence="sampled",
        )

        self.assertEqual(validate_collector_frame(frame), [])

    def test_raw_oob_command_task_force_delta_frame_validates(self):
        frame = self._raw_frame(
            "domain_delta",
            {
                "op": "sample",
                "before": None,
                "after": {
                    "oob_key": "oob_command:task_force:61700",
                    "entity_kind": "task_force",
                    "fleet_id": 61500,
                    "task_force_id": 61700,
                    "task_force_ids": None,
                    "task_force_count": None,
                    "task_force_status": None,
                    "parent_status": "exact",
                    "ship_ids": [5001, 5002],
                    "ship_count": 2,
                    "ship_status": "exact",
                },
                "changed_fields": [
                    "oob_key",
                    "entity_kind",
                    "fleet_id",
                    "task_force_id",
                    "parent_status",
                    "ship_ids",
                    "ship_count",
                    "ship_status",
                ],
            },
            domain="oob_command",
            entity_key="oob_command:task_force:61700",
            confidence="sampled",
        )

        self.assertEqual(validate_collector_frame(frame), [])

    def test_raw_oob_command_army_group_delta_frame_validates(self):
        frame = self._raw_frame(
            "domain_delta",
            {
                "op": "sample",
                "before": None,
                "after": {
                    "oob_key": "oob_command:army_group:81000",
                    "entity_kind": "army_group",
                    "army_id": None,
                    "army_group_id": 81000,
                    "fleet_id": None,
                    "task_force_id": None,
                    "member_division_ids": None,
                    "member_division_count": None,
                    "member_division_status": None,
                    "member_army_ids": [82000, 82001],
                    "member_army_count": 2,
                    "member_army_status": "exact",
                    "leader_id": None,
                    "leader_status": None,
                    "member_count_engine": None,
                    "missing_member_count": None,
                    "task_force_ids": None,
                    "task_force_count": None,
                    "task_force_status": None,
                    "parent_status": None,
                    "ship_ids": None,
                    "ship_count": None,
                    "ship_status": None,
                },
                "changed_fields": [
                    "oob_key",
                    "entity_kind",
                    "army_group_id",
                    "member_army_ids",
                    "member_army_count",
                    "member_army_status",
                ],
            },
            domain="oob_command",
            entity_key="oob_command:army_group:81000",
            confidence="sampled",
        )

        self.assertEqual(validate_collector_frame(frame), [])

    def test_raw_oob_command_army_delta_frame_validates(self):
        frame = self._raw_frame(
            "domain_delta",
            {
                "op": "sample",
                "before": None,
                "after": {
                    "oob_key": "oob_command:army:82000",
                    "entity_kind": "army",
                    "army_id": 82000,
                    "army_group_id": 81000,
                    "fleet_id": None,
                    "task_force_id": None,
                    "member_division_ids": [1101, 1102, 1103],
                    "member_division_count": 3,
                    "member_division_status": "exact",
                    "member_army_ids": None,
                    "member_army_count": None,
                    "member_army_status": None,
                    "leader_id": 305,
                    "leader_status": "exact",
                    "member_count_engine": 3,
                    "missing_member_count": 0,
                    "task_force_ids": None,
                    "task_force_count": None,
                    "task_force_status": None,
                    "parent_status": "exact",
                    "ship_ids": None,
                    "ship_count": None,
                    "ship_status": None,
                },
                "changed_fields": [
                    "oob_key",
                    "entity_kind",
                    "army_id",
                    "army_group_id",
                    "member_division_ids",
                    "member_division_count",
                    "member_division_status",
                    "leader_id",
                    "leader_status",
                    "member_count_engine",
                    "missing_member_count",
                    "parent_status",
                ],
            },
            domain="oob_command",
            entity_key="oob_command:army:82000",
            confidence="sampled",
        )

        self.assertEqual(validate_collector_frame(frame), [])

    def test_raw_oob_command_gap_delta_frame_validates(self):
        frame = self._raw_frame(
            "domain_delta",
            {
                "op": "sample",
                "before": None,
                "after": {
                    "gap_key": "oob_command:gap:bridge_entry_invalid",
                    "gap_kind": "bridge_entry_invalid",
                    "status": "validation_failed",
                },
                "changed_fields": ["gap_key", "gap_kind", "status"],
            },
            domain="oob_command",
            entity_key="oob_command:gap:bridge_entry_invalid",
            confidence="gap",
        )

        self.assertEqual(validate_collector_frame(frame), [])

    def test_raw_oob_command_delta_rejects_raw_leakage_and_bad_rows(self):
        frame = self._raw_frame(
            "domain_delta",
            {
                "op": "sample",
                "before": None,
                "after": {
                    "oob_key": "oob_command:task_force:61701",
                    "entity_kind": "task_force",
                    "fleet_id": 0,
                    "task_force_id": 61701,
                    "task_force_ids": [61700, 61700],
                    "task_force_count": -1,
                    "task_force_status": "bad",
                    "parent_status": "exact",
                    "ship_ids": [0],
                    "ship_count": 20001,
                    "ship_status": "bad",
                    "body_ptr": "0x1234",
                },
                "changed_fields": ["ship_ids", "body_ptr"],
            },
            domain="oob_command",
            entity_key="oob_command:task_force:61700",
            confidence="sampled",
        )

        problems = validate_collector_frame(frame)

        for needle in (
            "payload contains raw pointer/tag-db diagnostic key",
            "changed_fields[1] raw pointer/tag-db diagnostic key",
            "after.body_ptr unsupported",
            "after.oob_key != entity_key",
            "after.fleet_id not positive int|null in range",
            "after.task_force_id != entity_key",
            "after.task_force_ids[1] duplicate id",
            "after.task_force_count not nonnegative int|null in range",
            "after.task_force_status invalid",
            "after.ship_ids[0] not positive int in range",
            "after.ship_count not nonnegative int|null in range",
            "after.ship_status invalid",
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

    def test_snapshot_frame_folds_scheduled_buildings_live(self):
        frame = {
            "type": "snapshot",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "date": "1936.1.2.1",
            "capture_boundary": "scheduled_hour_snapshot",
            "domains": ["clock", "buildings_live"],
            "states": {
                "1": {"buildings": {"arms_factory": 2, "air_base": 1}},
                "2": {"buildings": {}},
            },
            "provinces": {
                "11": {"buildings": {"naval_base": 10, "rail_way": 2}},
            },
        }

        snap = snapshot_from_collector_frame(frame, "collector-test")

        self.assertEqual(schema.validate_snapshot(snap), [])
        self.assertEqual(
            snap["provenance"]["collector"]["domains"],
            ["buildings_live", "clock"],
        )
        self.assertEqual(snap["states"]["1"]["buildings"]["arms_factory"], 2)
        self.assertEqual(snap["states"]["2"]["buildings"], {})
        self.assertEqual(snap["provinces"]["11"]["buildings"]["naval_base"], 10)
        self.assertIsNone(snap["map"]["control_by_province"])

    def test_snapshot_frame_rejects_malformed_buildings_live(self):
        frame = {
            "type": "snapshot",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "date": "1936.1.2.1",
            "capture_boundary": "scheduled_hour_snapshot",
            "domains": ["clock", "buildings_live"],
            "states": {
                "1": {"buildings": {"bad key": 2, "arms_factory": 1001}},
            },
        }

        problems = validate_collector_frame(frame)

        self.assertTrue(any("not script token" in p for p in problems), problems)
        self.assertTrue(any("not int in range" in p for p in problems), problems)

    def test_snapshot_frame_rejects_non_scheduled_buildings_live_payload(self):
        frame = {
            "type": "snapshot",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "date": "1936.1.2.1",
            "capture_boundary": "daily_update",
            "domains": ["clock", "buildings_live"],
            "states": {"1": {"buildings": {"arms_factory": 2}}},
        }

        problems = validate_collector_frame(frame)

        self.assertTrue(any("buildings_live payload requires scheduled" in p
                            for p in problems), problems)
        with self.assertRaisesRegex(ValueError,
                                    "buildings_live payload requires scheduled"):
            snapshot_from_collector_frame(frame, "collector-test")

    def test_snapshot_frame_folds_scheduled_projects_live(self):
        frame = {
            "type": "snapshot",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "date": "1936.1.2.1",
            "capture_boundary": "scheduled_hour_snapshot",
            "domains": ["clock", "projects_live"],
            "special_projects": self._special_projects_payload(),
        }

        snap = snapshot_from_collector_frame(frame, "collector-test")

        self.assertEqual(schema.validate_snapshot(snap), [])
        self.assertEqual(
            snap["provenance"]["collector"]["domains"],
            ["clock", "projects_live"],
        )
        self.assertEqual(snap["special_projects"]["source"], "C")
        self.assertEqual(snap["special_projects"]["count"]["facilities"], 1)
        self.assertEqual(snap["special_projects"]["facilities"][0]["province"], 118)
        self.assertEqual(
            snap["special_projects"]["projects"][0]["worked_at_facility"], 1)
        self.assertEqual(
            snap["special_projects"]["scientists"][0]["skills"]
            ["specialization_land"],
            3,
        )

    def test_root_probe_frame_folds_scheduled_projects_live(self):
        frame = {
            "type": "snapshot",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "date": "1936.1.2.1",
            "capture_boundary": "scheduled_hour_snapshot",
            "domains": ["clock", "projects_live"],
            "special_projects": {
                "facilities": [],
                "projects": [],
                "scientists": [],
                "count": {"facilities": 0, "projects": 0, "scientists": 0},
                "source": "C",
            },
            "root_probe": {
                "status": "verified",
                "daily_update_sentinel_match": True,
                "long_string_sentinel_match": True,
                "current_game_date": "1936.1.2.1",
                "start_date": "1936.1.1.12",
                "domains": ["clock", "projects_live"],
            },
        }

        snap = snapshot_from_root_probe_frame(frame, "collector-test")

        self.assertEqual(schema.validate_snapshot(snap), [])
        self.assertEqual(snap["provenance"]["collector"]["domains"],
                         ["clock", "projects_live"])
        self.assertEqual(snap["special_projects"]["source"], "C")
        self.assertEqual(snap["special_projects"]["facilities"], [])

    def test_projects_payload_without_domain_claim_is_not_folded(self):
        frame = {
            "type": "snapshot",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "date": "1936.1.2.1",
            "capture_boundary": "scheduled_hour_snapshot",
            "domains": ["clock"],
            "special_projects": self._special_projects_payload(),
        }

        snap = snapshot_from_collector_frame(frame, "collector-test")

        self.assertEqual(schema.validate_snapshot(snap), [])
        self.assertIsNone(snap["special_projects"]["facilities"])
        self.assertIsNone(snap["special_projects"]["source"])

    def test_snapshot_frame_rejects_non_scheduled_projects_live_payload(self):
        frame = {
            "type": "snapshot",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "date": "1936.1.2.1",
            "capture_boundary": "daily_update",
            "domains": ["clock", "projects_live"],
            "special_projects": self._special_projects_payload(),
        }

        problems = validate_collector_frame(frame)

        self.assertTrue(any("projects_live payload requires scheduled" in p
                            for p in problems), problems)
        with self.assertRaisesRegex(ValueError,
                                    "projects_live payload requires scheduled"):
            snapshot_from_collector_frame(frame, "collector-test")

    def test_worker_poll_rejects_special_projects_payload(self):
        frame = {
            "type": "snapshot",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "date": "1936.1.2.1",
            "capture_boundary": "worker_poll",
            "domains": ["clock"],
            "special_projects": self._special_projects_payload(),
        }

        problems = validate_collector_frame(frame)

        self.assertTrue(any("worker_poll frame must not carry special_projects" in p
                            for p in problems), problems)

    def test_special_projects_rejects_raw_pointer_diagnostics(self):
        payload = self._special_projects_payload()
        payload["facilities"][0]["body_addr"] = "0x1234"
        frame = {
            "type": "snapshot",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "date": "1936.1.2.1",
            "capture_boundary": "scheduled_hour_snapshot",
            "domains": ["clock", "projects_live"],
            "special_projects": payload,
        }

        problems = validate_collector_frame(frame)

        self.assertTrue(any("raw pointer/tag-db" in p for p in problems), problems)

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

    def test_snapshot_frame_folds_scheduled_focus_live(self):
        frame = {
            "type": "snapshot",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "date": "1936.1.2.1",
            "capture_boundary": "scheduled_hour_snapshot",
            "domains": ["clock", "country_identity", "focus_live"],
            "countries": {
                "GER": {
                    "present": True,
                    "collector_object": "0x1000",
                    "identity": {"tag_db_id": 1},
                    "focus": {
                        "focus_tree": "german_focus",
                        "completed": ["GER_autarky", "GER_army_innovation"],
                        "completed_count": 2,
                        "current": "GER_west_wall",
                        "progress": 12.5,
                    },
                },
                "SOV": {
                    "present": True,
                    "collector_object": "0x2000",
                    "identity": {"tag_db_id": 2},
                    "focus": {
                        "focus_tree": "soviet_focus",
                        "completed": [],
                        "completed_count": 0,
                        "current_continuous": "continuous_repair",
                        "progress": 0.0,
                    },
                },
            },
        }

        snap = snapshot_from_collector_frame(frame, "collector-test")

        self.assertEqual(schema.validate_snapshot(snap), [])
        self.assertEqual(
            snap["provenance"]["collector"]["domains"],
            ["clock", "country_identity", "focus_live"],
        )
        ger = snap["countries"]["GER"]["focus"]
        self.assertEqual(ger["focus_tree"], "german_focus")
        self.assertEqual(ger["completed"], ["GER_autarky", "GER_army_innovation"])
        self.assertEqual(ger["completed_count"], 2)
        self.assertEqual(ger["current"], "GER_west_wall")
        self.assertEqual(ger["progress"], 12.5)
        sov = snap["countries"]["SOV"]["focus"]
        self.assertEqual(sov["completed"], [])
        self.assertEqual(sov["completed_count"], 0)
        self.assertEqual(sov["current_continuous"], "continuous_repair")
        self.assertEqual(
            set(snap["_live_measured_leaf_tags"]["focus.completed"]),
            {"GER", "SOV"},
        )

    def test_snapshot_frame_rejects_malformed_focus_live(self):
        frame = {
            "type": "snapshot",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "date": "1936.1.2.1",
            "capture_boundary": "scheduled_hour_snapshot",
            "domains": ["clock", "country_identity", "focus_live"],
            "countries": {
                "GER": {
                    "present": True,
                    "collector_object": "0x1000",
                    "identity": {"tag_db_id": 1},
                    "focus": {
                        "focus_tree": "bad token!",
                        "completed": ["GER_autarky", "bad token!"],
                        "completed_count": 3,
                        "current": "GER_west_wall",
                        "current_continuous": "continuous_repair",
                        "progress": -1.0,
                    },
                },
            },
        }

        problems = validate_collector_frame(frame)

        for needle in (
            "focus.focus_tree",
            "focus.completed[1]",
            "focus.completed_count does not match",
            "current and current_continuous both set",
            "focus.progress out of range",
        ):
            self.assertTrue(any(needle in p for p in problems), (needle, problems))
        with self.assertRaisesRegex(ValueError, "invalid collector frame"):
            snapshot_from_collector_frame(frame, "collector-test")

    def test_snapshot_frame_rejects_non_scheduled_focus_live(self):
        frame = {
            "type": "snapshot",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "date": "1936.1.2.1",
            "capture_boundary": "daily_update",
            "domains": ["clock", "country_identity", "focus_live"],
            "countries": {
                "GER": {
                    "present": True,
                    "collector_object": "0x1000",
                    "identity": {"tag_db_id": 1},
                    "focus": {
                        "focus_tree": "german_focus",
                        "completed": [],
                        "completed_count": 0,
                    },
                },
            },
        }

        with self.assertRaisesRegex(ValueError, "focus_live payload requires scheduled"):
            snapshot_from_collector_frame(frame, "collector-test")

    def test_snapshot_frame_folds_scheduled_tech_live(self):
        frame = {
            "type": "snapshot",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "date": "1936.1.2.1",
            "capture_boundary": "scheduled_hour_snapshot",
            "domains": ["clock", "country_identity", "tech_live"],
            "countries": {
                "GER": {
                    "present": True,
                    "collector_object": "0x1000",
                    "identity": {"tag_db_id": 1},
                    "research": {
                        "completed_tech_count": 120,
                        "research_slot_count": 4,
                        "active_research": [{
                            "slot_index": 0,
                            "tech_handle_raw": 0x1234,
                            "tech_def_index_raw": 5,
                            "progress_raw": 9000,
                            "tech_name": "basic_machine_tools",
                            "progress_units": 9.0,
                        }],
                    },
                },
                "SOV": {
                    "present": True,
                    "collector_object": "0x2000",
                    "identity": {"tag_db_id": 2},
                    "research": {
                        "completed_tech_count": 0,
                        "research_slot_count": 3,
                        "active_research": [],
                    },
                },
            },
        }

        snap = snapshot_from_collector_frame(frame, "collector-test")

        self.assertEqual(schema.validate_snapshot(snap), [])
        self.assertEqual(
            snap["provenance"]["collector"]["domains"],
            ["clock", "country_identity", "tech_live"],
        )
        ger = snap["countries"]["GER"]["research"]
        self.assertEqual(ger["tech_count"], 120)
        self.assertEqual(ger["research_slots"], 4)
        self.assertEqual(ger["in_progress_live"][0]["tech_handle_raw"], 0x1234)
        self.assertEqual(ger["in_progress_live"][0]["tech_name"], "basic_machine_tools")
        self.assertEqual(ger["in_progress_live"][0]["progress_units"], 9.0)
        self.assertEqual(ger["in_progress"], [])
        sov = snap["countries"]["SOV"]["research"]
        self.assertEqual(sov["tech_count"], 0)
        self.assertEqual(sov["research_slots"], 3)
        self.assertEqual(sov["in_progress_live"], [])
        self.assertEqual(
            set(snap["_live_measured_leaf_tags"]["research.in_progress_live"]),
            {"GER", "SOV"},
        )

    def test_snapshot_frame_rejects_malformed_tech_live(self):
        frame = {
            "type": "snapshot",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "date": "1936.1.2.1",
            "capture_boundary": "scheduled_hour_snapshot",
            "domains": ["clock", "country_identity", "tech_live"],
            "countries": {
                "GER": {
                    "present": True,
                    "collector_object": "0x1000",
                    "identity": {"tag_db_id": 1},
                    "research": {
                        "completed_tech_count": True,
                        "research_slot_count": 33,
                        "active_research": [
                            {
                                "slot_index": 32,
                                "tech_handle_raw": -1,
                                "tech_name": "bad token!",
                                "progress_units": -1.0,
                                "unexpected": 1,
                            },
                            "not a dict",
                        ],
                    },
                },
            },
        }

        problems = validate_collector_frame(frame)

        for needle in (
            "research.completed_tech_count",
            "research.research_slot_count",
            "active_research[0].slot_index",
            "active_research[0].tech_handle_raw",
            "active_research[0].tech_name",
            "active_research[0].progress_units",
            "active_research[0].unexpected unsupported",
            "active_research[1] not dict",
        ):
            self.assertTrue(any(needle in p for p in problems), (needle, problems))
        with self.assertRaisesRegex(ValueError, "invalid collector frame"):
            snapshot_from_collector_frame(frame, "collector-test")

    def test_snapshot_frame_rejects_non_scheduled_tech_live(self):
        frame = {
            "type": "snapshot",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "date": "1936.1.2.1",
            "capture_boundary": "daily_update",
            "domains": ["clock", "country_identity", "tech_live"],
            "countries": {
                "GER": {
                    "present": True,
                    "collector_object": "0x1000",
                    "identity": {"tag_db_id": 1},
                    "research": {
                        "completed_tech_count": 12,
                        "research_slot_count": 4,
                    },
                },
            },
        }

        with self.assertRaisesRegex(ValueError, "tech_live payload requires scheduled"):
            snapshot_from_collector_frame(frame, "collector-test")

    def test_snapshot_frame_folds_scheduled_politics_live(self):
        frame = {
            "type": "snapshot",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "date": "1936.1.2.1",
            "capture_boundary": "scheduled_hour_snapshot",
            "domains": ["clock", "country_identity", "politics_live"],
            "countries": {
                "POR": {
                    "present": True,
                    "collector_object": "0x1000",
                    "identity": {"tag_db_id": 47},
                    "politics": {
                        "popularities": {
                            "democratic": 72.5,
                            "communism": 0.0,
                            "fascism": 12.5,
                            "neutrality": 15.0,
                        },
                        "ruling_party": "neutrality",
                        "ideas": [
                            "war_economy",
                            "free_trade",
                            "volunteer_only",
                            "sour_loser",
                            "companhia_uniao_fabril",
                        ],
                        "ideas_other": ["sour_loser"],
                        "national_spirits": ["sour_loser"],
                        "laws": {
                            "economy": "war_economy",
                            "trade_laws": "free_trade",
                            "mobilization_laws": "volunteer_only",
                        },
                        "idea_groups": {
                            "country": ["sour_loser"],
                            "economy": ["war_economy"],
                            "industrial_concern": ["companhia_uniao_fabril"],
                            "trade_laws": ["free_trade"],
                            "mobilization_laws": ["volunteer_only"],
                        },
                        "advisor_ideas": {
                            "industrial_concern": ["companhia_uniao_fabril"],
                        },
                        "idea_count": 5,
                        "election_frequency": 48,
                        "party_count": 4,
                        "low_confidence": False,
                    },
                },
            },
        }

        snap = snapshot_from_collector_frame(frame, "collector-test")

        self.assertEqual(schema.validate_snapshot(snap), [])
        self.assertEqual(
            snap["provenance"]["collector"]["domains"],
            ["clock", "country_identity", "politics_live"],
        )
        politics = snap["countries"]["POR"]["politics"]
        self.assertEqual(politics["ruling_party"], "neutrality")
        self.assertEqual(politics["party_popularities"]["democratic"], 72.5)
        self.assertEqual(
            politics["ideas"],
            [
                "war_economy",
                "free_trade",
                "volunteer_only",
                "sour_loser",
                "companhia_uniao_fabril",
            ],
        )
        self.assertEqual(politics["ideas_other"], ["sour_loser"])
        self.assertEqual(politics["national_spirits"], ["sour_loser"])
        self.assertEqual(
            politics["laws"],
            {
                "economy": "war_economy",
                "trade_laws": "free_trade",
                "mobilization_laws": "volunteer_only",
            },
        )
        self.assertEqual(
            politics["idea_groups"]["industrial_concern"],
            ["companhia_uniao_fabril"],
        )
        self.assertEqual(
            politics["advisor_ideas"],
            {"industrial_concern": ["companhia_uniao_fabril"]},
        )
        self.assertEqual(politics["idea_count"], 5)
        self.assertEqual(politics["election_frequency"], 48)
        self.assertEqual(politics["party_count"], 4)
        self.assertFalse(politics["low_confidence"])
        self.assertIsNone(politics["political_power"])
        measured = snap["_live_measured_leaf_tags"]
        self.assertEqual(measured["politics.ideas_other"], ["POR"])
        self.assertEqual(measured["politics.national_spirits"], ["POR"])

    def test_snapshot_frame_marks_measured_empty_politics_ideas_other(self):
        frame = {
            "type": "snapshot",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "date": "1936.1.2.1",
            "capture_boundary": "scheduled_hour_snapshot",
            "domains": ["clock", "country_identity", "politics_live"],
            "countries": {
                "POR": {
                    "present": True,
                    "collector_object": "0x1000",
                    "identity": {"tag_db_id": 47},
                    "politics": {
                        "ideas": [],
                        "ideas_other": [],
                        "national_spirits": [],
                    },
                },
            },
        }

        snap = snapshot_from_collector_frame(frame, "collector-test")

        self.assertEqual(schema.validate_snapshot(snap), [])
        politics = snap["countries"]["POR"]["politics"]
        self.assertEqual(politics["ideas"], [])
        self.assertEqual(politics["ideas_other"], [])
        self.assertEqual(politics["national_spirits"], [])
        measured = snap["_live_measured_leaf_tags"]
        self.assertEqual(measured["politics.ideas_other"], ["POR"])
        self.assertEqual(measured["politics.national_spirits"], ["POR"])

    def test_snapshot_frame_rejects_malformed_politics_live(self):
        frame = {
            "type": "snapshot",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "date": "1936.1.2.1",
            "capture_boundary": "scheduled_hour_snapshot",
            "domains": ["clock", "country_identity", "politics_live"],
            "countries": {
                "POR": {
                    "present": True,
                    "collector_object": "0x1000",
                    "identity": {"tag_db_id": 47},
                    "politics": {
                        "popularities": {"bad": 1.0, "democratic": float("inf")},
                        "ruling_party": "bad",
                        "ideas": ["ok", "bad token!"],
                        "ideas_other": ["ok", "bad token!"],
                        "national_spirits": ["ok", "bad token!"],
                        "laws": {
                            "bad key!": "ok",
                            "economy": "bad token!",
                        },
                        "idea_groups": {
                            "industrial_concern": ["ok", "bad token!"],
                        },
                        "advisor_ideas": {
                            "bad group!": ["ok"],
                        },
                        "idea_count": 401,
                        "election_frequency": 0,
                        "party_count": 17,
                        "low_confidence": 1,
                    },
                },
            },
        }

        problems = validate_collector_frame(frame)

        for needle in (
            "popularities key",
            "popularities.democratic",
            "ruling_party",
            "ideas[1]",
            "ideas_other[1]",
            "national_spirits[1]",
            "laws key",
            "laws.economy",
            "idea_groups.industrial_concern[1]",
            "advisor_ideas key",
            "idea_count",
            "election_frequency",
            "party_count",
            "low_confidence",
        ):
            self.assertTrue(any(needle in p for p in problems), (needle, problems))
        with self.assertRaisesRegex(ValueError, "invalid collector frame"):
            snapshot_from_collector_frame(frame, "collector-test")

    def test_snapshot_frame_rejects_non_scheduled_politics_live(self):
        frame = {
            "type": "snapshot",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "date": "1936.1.2.1",
            "capture_boundary": "daily_update",
            "domains": ["clock", "country_identity", "politics_live"],
            "countries": {
                "POR": {
                    "present": True,
                    "collector_object": "0x1000",
                    "identity": {"tag_db_id": 47},
                    "politics": {
                        "popularities": {"neutrality": 100.0},
                        "ruling_party": "neutrality",
                    },
                },
            },
        }

        with self.assertRaisesRegex(ValueError, "politics_live payload requires scheduled"):
            snapshot_from_collector_frame(frame, "collector-test")

    def test_snapshot_frame_folds_scheduled_decisions_live(self):
        frame = {
            "type": "snapshot",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "date": "1936.1.2.1",
            "capture_boundary": "scheduled_hour_snapshot",
            "domains": ["clock", "country_identity", "decisions_live"],
            "countries": {
                "POR": {
                    "present": True,
                    "collector_object": "0x1000",
                    "identity": {"tag_db_id": 47},
                    "decisions": {
                        "status_present": {
                            "active_timed_decision": 1,
                            "active_targeted_decision": 2,
                            "decision_to_re_enable": 1,
                        },
                        "timed_live": [
                            {
                                "decision": "POR_expand_aluminium_mines",
                                "days": 43,
                                "state": 0,
                                "state_name": "active",
                            },
                        ],
                    },
                },
                "ENG": {
                    "present": True,
                    "collector_object": "0x2000",
                    "identity": {"tag_db_id": 2},
                    "decisions": {
                        "status_present": {},
                        "timed_live": [],
                    },
                },
            },
        }

        snap = snapshot_from_collector_frame(frame, "collector-test")

        self.assertEqual(schema.validate_snapshot(snap), [])
        self.assertEqual(
            snap["provenance"]["collector"]["domains"],
            ["clock", "country_identity", "decisions_live"],
        )
        decisions = snap["countries"]["POR"]["decisions"]
        self.assertEqual(
            decisions["status_present"],
            {
                "active_timed_decision": 1,
                "active_targeted_decision": 2,
                "decision_to_re_enable": 1,
            },
        )
        self.assertEqual(
            decisions["timed_live"],
            [{
                "decision": "POR_expand_aluminium_mines",
                "days": 43,
                "state": 0,
                "state_name": "active",
            }],
        )
        self.assertEqual(snap["countries"]["ENG"]["decisions"]["timed_live"], [])
        self.assertEqual(
            set(snap["_live_measured_leaf_tags"]["decisions.timed_live"]),
            {"POR", "ENG"},
        )

    def test_snapshot_frame_rejects_malformed_decisions_live(self):
        frame = {
            "type": "snapshot",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "date": "1936.1.2.1",
            "capture_boundary": "scheduled_hour_snapshot",
            "domains": ["clock", "country_identity", "decisions_live"],
            "countries": {
                "POR": {
                    "present": True,
                    "collector_object": "0x1000",
                    "identity": {"tag_db_id": 47},
                    "decisions": {
                        "status_present": {
                            "bad key!": 1,
                            "active_timed_decision": -1,
                        },
                        "timed_live": [
                            {
                                "decision": "bad token!",
                                "days": True,
                                "state": -1,
                                "state_name": "not active",
                            },
                        ],
                    },
                },
            },
        }

        problems = validate_collector_frame(frame)

        for needle in (
            "status_present key",
            "status_present.active_timed_decision",
            "timed_live[0].decision",
            "timed_live[0].days",
            "timed_live[0].state",
            "timed_live[0].state_name",
        ):
            self.assertTrue(any(needle in p for p in problems), (needle, problems))
        with self.assertRaisesRegex(ValueError, "invalid collector frame"):
            snapshot_from_collector_frame(frame, "collector-test")

    def test_snapshot_frame_folds_scheduled_diplomacy_live(self):
        frame = {
            "type": "snapshot",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "date": "1936.1.2.1",
            "capture_boundary": "scheduled_hour_snapshot",
            "domains": ["clock", "country_identity", "diplomacy_live"],
            "countries": {
                "GER": {
                    "present": True,
                    "collector_object": "0x1000",
                    "identity": {"tag_db_id": 1},
                    "diplomacy": {"faction_group": 7},
                },
                "ITA": {
                    "present": True,
                    "collector_object": "0x2000",
                    "identity": {"tag_db_id": 11},
                    "diplomacy": {"faction_group": 7},
                },
                "SOV": {
                    "present": True,
                    "collector_object": "0x3000",
                    "identity": {"tag_db_id": 3},
                    "diplomacy": {"faction_group": 9},
                },
            },
        }

        snap = snapshot_from_collector_frame(frame, "collector-test")

        self.assertEqual(schema.validate_snapshot(snap), [])
        self.assertEqual(
            snap["provenance"]["collector"]["domains"],
            ["clock", "country_identity", "diplomacy_live"],
        )
        self.assertEqual(
            snap["countries"]["GER"]["diplomacy"]["faction_group"],
            7,
        )
        self.assertIsNone(snap["countries"]["GER"]["diplomacy"]["at_war"])
        self.assertIsNone(snap["countries"]["GER"]["diplomacy"]["casualties"])
        self.assertEqual(
            snap["diplomacy_live"]["faction_members"],
            {"7": ["GER", "ITA"], "9": ["SOV"]},
        )
        self.assertIsNone(snap["diplomacy_live"]["war_relations"])
        self.assertIsNone(snap["diplomacy_live"]["subtype_warning"])
        self.assertEqual(snap["diplomacy_live"]["source"], "C")
        self.assertEqual(
            set(snap["_live_measured_leaf_tags"]["diplomacy.faction_group"]),
            {"GER", "ITA", "SOV"},
        )

    def test_snapshot_frame_folds_scheduled_diplomacy_war_relations(self):
        frame = {
            "type": "snapshot",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "date": "1939.9.2.1",
            "capture_boundary": "scheduled_hour_snapshot",
            "domains": ["clock", "country_identity", "diplomacy_live"],
            "countries": {
                "GER": {
                    "present": True,
                    "collector_object": "0x1000",
                    "identity": {"tag_db_id": 1},
                    "diplomacy": {
                        "faction_group": 7,
                        "at_war": ["SOV"],
                        "casualties": 100,
                    },
                },
                "SOV": {
                    "present": True,
                    "collector_object": "0x2000",
                    "identity": {"tag_db_id": 3},
                    "diplomacy": {
                        "faction_group": 9,
                        "at_war": ["GER"],
                        "casualties": 200,
                    },
                },
            },
            "diplomacy_live": {
                "war_relations": [
                    {
                        "first": "GER",
                        "second": "SOV",
                        "first_casualties": 100,
                        "second_casualties": 200,
                    }
                ],
                "faction_members": {"7": ["GER"], "9": ["SOV"]},
                "subtype_warning": {},
                "source": "C",
            },
        }

        snap = snapshot_from_collector_frame(frame, "collector-test")

        self.assertEqual(schema.validate_snapshot(snap), [])
        self.assertEqual(snap["countries"]["GER"]["diplomacy"]["at_war"], ["SOV"])
        self.assertEqual(snap["countries"]["SOV"]["diplomacy"]["casualties"], 200)
        self.assertEqual(
            snap["diplomacy_live"]["war_relations"],
            [{
                "first": "GER",
                "second": "SOV",
                "first_casualties": 100,
                "second_casualties": 200,
            }],
        )
        self.assertEqual(
            set(snap["_live_measured_leaf_tags"]["diplomacy.at_war"]),
            {"GER", "SOV"},
        )
        self.assertEqual(
            set(snap["_live_measured_leaf_tags"]["diplomacy.casualties"]),
            {"GER", "SOV"},
        )

    def test_snapshot_frame_rejects_malformed_diplomacy_live(self):
        frame = {
            "type": "snapshot",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "date": "1936.1.2.1",
            "capture_boundary": "scheduled_hour_snapshot",
            "domains": ["clock", "country_identity", "diplomacy_live"],
            "countries": {
                "POR": {
                    "present": True,
                    "collector_object": "0x1000",
                    "identity": {"tag_db_id": 47},
                    "diplomacy": {"faction_group": True},
                },
            },
        }

        problems = validate_collector_frame(frame)

        self.assertTrue(
            any("diplomacy.faction_group" in p for p in problems),
            problems,
        )
        with self.assertRaisesRegex(ValueError, "invalid collector frame"):
            snapshot_from_collector_frame(frame, "collector-test")

    def test_snapshot_frame_rejects_non_scheduled_diplomacy_live(self):
        frame = {
            "type": "snapshot",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "date": "1936.1.2.1",
            "capture_boundary": "daily_update",
            "domains": ["clock", "country_identity", "diplomacy_live"],
            "countries": {
                "POR": {
                    "present": True,
                    "collector_object": "0x1000",
                    "identity": {"tag_db_id": 47},
                    "diplomacy": {"faction_group": 5},
                },
            },
        }

        with self.assertRaisesRegex(ValueError, "diplomacy_live payload requires scheduled"):
            snapshot_from_collector_frame(frame, "collector-test")

    def test_snapshot_frame_folds_scheduled_politics_residuals(self):
        frame = {
            "type": "snapshot",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "date": "1936.1.2.1",
            "capture_boundary": "scheduled_hour_snapshot",
            "domains": ["clock", "country_identity", "politics_residuals"],
            "countries": {
                "POR": {
                    "present": True,
                    "collector_object": "0x1000",
                    "identity": {"tag_db_id": 47},
                    "politics": {
                        "last_election": "1933.7.1.1",
                        "elections_allowed": False,
                        "appointed_advisors": [],
                        "appointed_advisor_details": [
                            {
                                "idea_token": "POR_salazar",
                                "slot": "political_advisor",
                                "ledger": "political",
                                "character": "POR_antonio_de_oliveira_salazar",
                            }
                        ],
                        "appointed_advisor_slots": {
                            "political_advisor": ["POR_salazar"],
                        },
                        "appointed_advisor_ledgers": {
                            "political": ["POR_salazar"],
                        },
                    },
                },
            },
        }

        snap = snapshot_from_collector_frame(frame, "collector-test")

        self.assertEqual(schema.validate_snapshot(snap), [])
        self.assertEqual(
            snap["provenance"]["collector"]["domains"],
            ["clock", "country_identity", "politics_residuals"],
        )
        politics = snap["countries"]["POR"]["politics"]
        self.assertEqual(politics["last_election"], "1933.7.1.1")
        self.assertFalse(politics["elections_allowed"])
        self.assertEqual(politics["appointed_advisors"], [])
        self.assertEqual(
            politics["appointed_advisor_details"],
            [
                {
                    "idea_token": "POR_salazar",
                    "slot": "political_advisor",
                    "ledger": "political",
                    "character": "POR_antonio_de_oliveira_salazar",
                }
            ],
        )
        self.assertEqual(
            politics["appointed_advisor_slots"],
            {"political_advisor": ["POR_salazar"]},
        )
        self.assertEqual(
            politics["appointed_advisor_ledgers"],
            {"political": ["POR_salazar"]},
        )
        self.assertIsNone(politics["ruling_party"])

    def test_snapshot_frame_rejects_malformed_politics_residuals(self):
        frame = {
            "type": "snapshot",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "date": "1936.1.2.1",
            "capture_boundary": "scheduled_hour_snapshot",
            "domains": ["clock", "country_identity", "politics_residuals"],
            "countries": {
                "POR": {
                    "present": True,
                    "collector_object": "0x1000",
                    "identity": {"tag_db_id": 47},
                    "politics": {
                        "last_election": "1933-7-1",
                        "elections_allowed": 0,
                        "appointed_advisors": ["POR_salazar", "bad token!"],
                        "appointed_advisor_details": [
                            {"idea_token": "POR_salazar", "slot": "bad slot!"}
                        ],
                        "appointed_advisor_slots": {
                            "political advisor": ["POR_salazar"],
                        },
                        "appointed_advisor_ledgers": {
                            "political": ["bad token!"],
                        },
                    },
                },
            },
        }

        problems = validate_collector_frame(frame)

        self.assertTrue(any("last_election" in p for p in problems), problems)
        self.assertTrue(any("elections_allowed" in p for p in problems), problems)
        self.assertTrue(any("appointed_advisors[1]" in p for p in problems), problems)
        self.assertTrue(any("appointed_advisor_details[0].slot" in p
                            for p in problems), problems)
        self.assertTrue(any("appointed_advisor_slots key" in p for p in problems),
                        problems)
        self.assertTrue(any("appointed_advisor_ledgers.political[0]" in p
                            for p in problems), problems)
        with self.assertRaisesRegex(ValueError, "invalid collector frame"):
            snapshot_from_collector_frame(frame, "collector-test")

    def test_snapshot_frame_rejects_non_scheduled_politics_residuals(self):
        frame = {
            "type": "snapshot",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "date": "1936.1.2.1",
            "capture_boundary": "daily_update",
            "domains": ["clock", "country_identity", "politics_residuals"],
            "countries": {
                "POR": {
                    "present": True,
                    "collector_object": "0x1000",
                    "identity": {"tag_db_id": 47},
                    "politics": {
                        "last_election": "1933.7.1.1",
                        "elections_allowed": True,
                    },
                },
            },
        }

        with self.assertRaisesRegex(ValueError, "politics_residuals payload requires scheduled"):
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

    def test_root_probe_frame_folds_scheduled_buildings_live(self):
        frame = {
            "type": "collector_started",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "process_id": 32440,
            "tick_ms": 158373765,
            "capture_boundary": "scheduled_hour_snapshot",
            "quality_status": "root_probe_verified",
            "domains": ["clock", "buildings_live"],
            "states": {"1": {"buildings": {"arms_factory": 2}}},
            "provinces": {"11": {"buildings": {"naval_base": 10}}},
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
                "domains": ["clock", "buildings_live"],
            },
        }

        snap = snapshot_from_root_probe_frame(frame, "collector-test")

        self.assertEqual(schema.validate_snapshot(snap), [])
        self.assertEqual(snap["provenance"]["collector"]["domains"],
                         ["clock", "buildings_live"])
        self.assertEqual(snap["states"]["1"]["buildings"], {"arms_factory": 2})
        self.assertEqual(snap["provinces"]["11"]["buildings"], {"naval_base": 10})

    def test_root_probe_frame_folds_scheduled_diplomacy_live(self):
        frame = {
            "type": "collector_started",
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "abi_version": 1,
            "process_id": 32440,
            "tick_ms": 158373765,
            "capture_boundary": "scheduled_hour_snapshot",
            "quality_status": "root_probe_verified",
            "domains": ["clock", "diplomacy_live"],
            "countries": {
                "GER": {
                    "present": True,
                    "collector_object": "0x1000",
                    "identity": {"tag_db_id": 1},
                    "diplomacy": {"faction_group": 7},
                },
                "ITA": {
                    "present": True,
                    "collector_object": "0x2000",
                    "identity": {"tag_db_id": 11},
                    "diplomacy": {"faction_group": 7},
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
                "tag_sample": ["---", "GER", "ITA"],
                "domains": ["clock", "diplomacy_live"],
            },
        }

        snap = snapshot_from_root_probe_frame(frame, "collector-test")

        self.assertEqual(schema.validate_snapshot(snap), [])
        self.assertEqual(
            snap["provenance"]["collector"]["domains"],
            ["clock", "country_identity", "diplomacy_live"],
        )
        self.assertEqual(
            snap["countries"]["GER"]["diplomacy"]["faction_group"],
            7,
        )
        self.assertEqual(
            snap["diplomacy_live"]["faction_members"],
            {"7": ["GER", "ITA"]},
        )
        self.assertIsNone(snap["diplomacy_live"]["war_relations"])

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
                    "research": {
                        "completed_tech_count": 12,
                        "research_slot_count": 4,
                    },
                    "politics": {
                        "popularities": {"neutrality": 100.0},
                        "ruling_party": "neutrality",
                        "last_election": "1933.7.1.1",
                        "elections_allowed": False,
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
        self.assertTrue(any("worker_poll frame must not carry research" in p
                            for p in problems), problems)
        self.assertTrue(any("worker_poll frame must not carry politics" in p
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
            "domains": [
                "clock",
                "fuel_live",
                "tech_live",
                "politics_live",
                "politics_residuals",
                "production_live",
                "state_control",
            ],
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
                "13": {"controller": None, "controller_tag_db_id": "454"},
            },
        }

        problems = validate_collector_frame(frame)

        self.assertTrue(any("state key '0'" in p for p in problems), problems)
        self.assertTrue(any("state '1'.controller" in p for p in problems), problems)
        self.assertTrue(any("province '11'.controller" in p for p in problems),
                        problems)
        self.assertTrue(any("province '12'.state_id" in p for p in problems),
                        problems)
        self.assertTrue(any("province '13'.controller_tag_db_id" in p
                            for p in problems), problems)

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
