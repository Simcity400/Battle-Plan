import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))

from live_capture.collector_frames import validate_collector_frame  # noqa: E402
from memory_reader.division_live.division_movement_delta import (  # noqa: E402
    DEFAULT_DIVISION_TYPE,
    DivisionMovementDeltaTracker,
    division_entity_key,
    division_uid,
)
from replay import schema  # noqa: E402


class TestDivisionMovementDeltaTracker(unittest.TestCase):
    def _tracker(self):
        return DivisionMovementDeltaTracker(progress_bucket_size=0.25)

    def _row(self, **overrides):
        row = {
            "id": 17,
            "moving": True,
            "movement_progress": 0.10,
            "movement_source_province": 100,
            "move_target_province": 200,
            "location": 200,
            "owner": "GER",
            "owner_tag_db_id": 1,
        }
        row.update(overrides)
        return row

    def _observe(self, tracker, row, **overrides):
        date = overrides.pop("date", "1936.1.1.12")
        kwargs = {
            "date": date,
            "hour_index": schema.date_to_hour_index(date),
            "sequence": overrides.pop("sequence", 11),
            "tick_ms": overrides.pop("tick_ms", 123456),
            "sample_ms": overrides.pop("sample_ms", 10),
            "capture_boundary": overrides.pop("capture_boundary", "worker_poll"),
            "collector_version": overrides.pop("collector_version", "0.1.0"),
        }
        kwargs.update(overrides)
        frame = tracker.observe_row(row, **kwargs)
        if frame is not None:
            self.assertEqual(validate_collector_frame(frame), [])
        return frame

    def test_first_observation_emits_initial_movement_state(self):
        frame = self._observe(self._tracker(), self._row())

        self.assertIsNotNone(frame)
        self.assertEqual(frame["type"], "domain_delta")
        self.assertEqual(frame["domain"], "divisions")
        self.assertEqual(frame["source"], "C")
        self.assertEqual(frame["confidence"], "sampled")
        self.assertEqual(frame["entity_key"], "division:17:4713")
        payload = frame["payload"]
        self.assertEqual(payload["op"], "upsert")
        self.assertIsNone(payload["before"])
        after = payload["after"]
        self.assertEqual(after["div_uid"], "17:4713")
        self.assertEqual(after["div_id"], 17)
        self.assertEqual(after["div_type"], DEFAULT_DIVISION_TYPE)
        self.assertEqual(after["moving"], True)
        self.assertEqual(after["movement_source_province"], 100)
        self.assertEqual(after["move_target_province"], 200)
        self.assertEqual(after["location"], 200)
        self.assertEqual(after["progress_bucket"], 0)
        self.assertEqual(after["movement_progress"], 0.10)
        self.assertEqual(after["owner"], "GER")
        self.assertNotIn("owner_tag_db_id", after)

    def test_unchanged_record_is_noop(self):
        tracker = self._tracker()
        self._observe(tracker, self._row())

        frame = self._observe(tracker, self._row(), sequence=12)

        self.assertIsNone(frame)

    def test_progress_delta_emits_only_when_bucket_crosses(self):
        tracker = self._tracker()
        self._observe(tracker, self._row(movement_progress=0.10))

        same_bucket = self._observe(
            tracker,
            self._row(movement_progress=0.24),
            sequence=12,
        )
        crossed_bucket = self._observe(
            tracker,
            self._row(movement_progress=0.25),
            sequence=13,
        )

        self.assertIsNone(same_bucket)
        self.assertIsNotNone(crossed_bucket)
        payload = crossed_bucket["payload"]
        self.assertEqual(payload["op"], "sample")
        self.assertEqual(payload["changed_fields"],
                         ["progress_bucket", "movement_progress"])
        self.assertEqual(payload["before"]["progress_bucket"], 0)
        self.assertEqual(payload["after"]["progress_bucket"], 1)
        self.assertEqual(payload["after"]["movement_progress"], 0.25)

    def test_movement_start_and_stop_emit_discrete_deltas(self):
        tracker = self._tracker()
        self._observe(
            tracker,
            self._row(
                moving=False,
                movement_progress=0.0,
                movement_source_province=None,
                move_target_province=None,
                location=100,
            ),
        )

        start = self._observe(tracker, self._row(movement_progress=0.10), sequence=12)
        stop = self._observe(
            tracker,
            self._row(
                moving=False,
                movement_progress=0.0,
                movement_source_province=None,
                move_target_province=None,
                location=200,
            ),
            sequence=13,
        )

        self.assertIsNotNone(start)
        self.assertEqual(start["payload"]["op"], "upsert")
        self.assertIn("moving", start["payload"]["changed_fields"])
        self.assertEqual(start["payload"]["before"]["moving"], False)
        self.assertEqual(start["payload"]["after"]["moving"], True)
        self.assertIsNotNone(stop)
        self.assertEqual(stop["payload"]["op"], "upsert")
        self.assertIn("moving", stop["payload"]["changed_fields"])
        self.assertEqual(stop["payload"]["after"]["moving"], False)
        self.assertIsNone(stop["payload"]["after"]["movement_source_province"])
        self.assertIsNone(stop["payload"]["after"]["move_target_province"])
        self.assertIsNone(stop["payload"]["after"]["progress_bucket"])
        self.assertIsNone(stop["payload"]["after"]["movement_progress"])

    def test_aba_id_type_safety_uses_distinct_stable_keys(self):
        tracker = self._tracker()
        first = self._observe(tracker, self._row(id=17, div_type=4713))
        reused_id = self._observe(
            tracker,
            self._row(id=17, div_type=5000, movement_progress=0.10),
            sequence=12,
        )

        self.assertEqual(first["entity_key"], division_entity_key(17, 4713))
        self.assertEqual(reused_id["entity_key"], division_entity_key(17, 5000))
        self.assertIsNone(reused_id["payload"]["before"])
        self.assertEqual(reused_id["payload"]["after"]["div_uid"],
                         division_uid(17, 5000))

    def test_observe_rows_advances_sequence_only_for_emitted_frames(self):
        tracker = self._tracker()
        frames = tracker.observe_rows(
            [
                self._row(id=1, movement_progress=0.10),
                self._row(id=1, movement_progress=0.20),
                self._row(id=2, movement_progress=0.10),
            ],
            date="1936.1.1.12",
            hour_index=schema.date_to_hour_index("1936.1.1.12"),
            sequence=40,
            tick_ms=123456,
            sample_ms=10,
            capture_boundary="worker_poll",
            collector_version="0.1.0",
        )

        self.assertEqual([frame["sequence"] for frame in frames], [40, 41])
        for frame in frames:
            self.assertEqual(validate_collector_frame(frame), [])

    def test_raw_pointer_and_tag_db_id_do_not_enter_payload(self):
        tracker = self._tracker()

        frame = self._observe(
            tracker,
            self._row(
                owner=None,
                body_addr="0x1234",
                collector_object="0x5678",
                owner_tag_db_id=99,
            ),
        )

        after = frame["payload"]["after"]
        self.assertNotIn("body_addr", after)
        self.assertNotIn("collector_object", after)
        self.assertNotIn("owner_tag_db_id", after)
        self.assertNotIn("owner", after)


if __name__ == "__main__":
    unittest.main()
