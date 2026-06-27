"""Tests for GUI recorder in-game-hour cadence accounting."""
from __future__ import annotations

import os
import sys
import unittest

_HERE = os.path.dirname(os.path.abspath(__file__))
_REPO = os.path.normpath(os.path.join(_HERE, os.pardir, os.pardir))
if _REPO not in sys.path:
    sys.path.insert(0, _REPO)

from live_capture.capture import DateInfo  # noqa: E402
from live_capture.recorder.cadence import RecorderCadence  # noqa: E402
from replay import schema  # noqa: E402


class TestRecorderCadence(unittest.TestCase):
    def test_duplicate_hour_is_not_captured(self):
        c = RecorderCadence()
        first = c.observe(DateInfo("1936.1.1.0", trusted=True, source="clock"))
        dup = c.observe(DateInfo("1936.1.1.0", trusted=True, source="clock"))

        self.assertTrue(first.should_capture)
        self.assertEqual(first.reason, "first_hour")
        self.assertFalse(dup.should_capture)
        self.assertEqual(dup.reason, "duplicate_hour")
        self.assertEqual(c.duplicate_observations, 1)
        self.assertEqual(c.skipped_hours, 0)

    def test_sequential_hour_is_captured_without_skip(self):
        c = RecorderCadence()
        c.observe(DateInfo("1936.1.1.0", trusted=True, source="clock"))
        nxt = c.observe(DateInfo("1936.1.1.1", trusted=True, source="clock"))

        self.assertTrue(nxt.should_capture)
        self.assertEqual(nxt.reason, "sequential_hour")
        self.assertEqual(nxt.skipped_delta, 0)
        self.assertEqual(c.skipped_hours, 0)
        self.assertEqual(c.last_captured_hour,
                         schema.date_to_hour_index("1936.1.1.1"))

    def test_hourly_mode_captures_every_observed_hour_across_day_boundary(self):
        c = RecorderCadence()
        dates = ["1936.1.1.%d" % h for h in range(24)] + ["1936.1.2.0"]
        decisions = [
            c.observe(DateInfo(d, trusted=True, source="clock"))
            for d in dates
        ]

        self.assertTrue(all(d.should_capture for d in decisions))
        self.assertEqual([d.hour_index for d in decisions],
                         [schema.date_to_hour_index(d) for d in dates])
        self.assertEqual(c.captures, len(dates))
        self.assertEqual(c.skipped_hours, 0)
        self.assertEqual(c.cadence_wait_observations, 0)
        self.assertEqual(c.duplicate_observations, 0)
        self.assertEqual(c.to_manifest()["cadence"]["target_step_hours"], 1)

    def test_every_two_hours_waits_until_step_is_due(self):
        c = RecorderCadence(target_step_hours=2)
        c.observe(DateInfo("1936.1.1.0", trusted=True, source="clock"))
        not_due = c.observe(DateInfo("1936.1.1.1", trusted=True,
                                     source="clock"))
        due = c.observe(DateInfo("1936.1.1.2", trusted=True,
                                 source="clock"))

        self.assertFalse(not_due.should_capture)
        self.assertEqual(not_due.reason, "cadence_not_due")
        self.assertEqual(c.cadence_wait_observations, 1)
        self.assertTrue(due.should_capture)
        self.assertEqual(due.reason, "sequential_hour")
        self.assertEqual(c.skipped_hours, 0)
        self.assertEqual(c.last_captured_hour,
                         schema.date_to_hour_index("1936.1.1.2"))

    def test_skipped_hours_are_counted_after_commit(self):
        c = RecorderCadence()
        h0 = schema.date_to_hour_index("1936.1.1.0")
        c.observe(DateInfo("1936.1.1.0", trusted=True, source="clock"))
        jumped = c.plan(DateInfo("1936.1.1.3", trusted=True, source="clock"))

        self.assertTrue(jumped.should_capture)
        self.assertEqual(jumped.reason, "skipped_hours")
        self.assertEqual(jumped.skipped_delta, 2)
        self.assertEqual(c.skipped_hours, 0)
        self.assertEqual(c.skip_events, 0)
        self.assertEqual(c.skip_ranges, [])

        c.commit(jumped)
        self.assertEqual(c.skipped_hours, 2)
        self.assertEqual(c.skip_events, 1)
        self.assertEqual(c.skip_ranges, [{
            "start_hour": h0 + 1,
            "end_hour": h0 + 2,
            "count": 2,
        }])
        man = c.to_manifest()["cadence"]
        self.assertEqual(man["status"], "lagging")
        self.assertEqual(man["skipped_hours"], 2)

    def test_every_two_hours_skips_only_missed_cadence_targets(self):
        c = RecorderCadence(target_step_hours=2)
        h0 = schema.date_to_hour_index("1936.1.1.0")
        c.observe(DateInfo("1936.1.1.0", trusted=True, source="clock"))
        jumped = c.observe(DateInfo("1936.1.1.6", trusted=True,
                                    source="clock"))

        self.assertTrue(jumped.should_capture)
        self.assertEqual(jumped.reason, "skipped_hours")
        self.assertEqual(jumped.skipped_delta, 2)
        self.assertEqual(c.skipped_hours, 2)
        self.assertEqual(c.skip_ranges, [{
            "start_hour": h0 + 2,
            "end_hour": h0 + 4,
            "count": 2,
            "step_hours": 2,
        }])

    def test_failed_capture_hour_retries_instead_of_duplicate(self):
        c = RecorderCadence()
        first = c.plan(DateInfo("1936.1.1.0", trusted=True, source="clock"))
        retry = c.plan(DateInfo("1936.1.1.0", trusted=True, source="clock"))

        self.assertTrue(first.should_capture)
        self.assertTrue(retry.should_capture)
        self.assertEqual(retry.reason, "first_hour")
        self.assertEqual(c.captures, 0)
        self.assertEqual(c.last_captured_hour, None)
        self.assertEqual(c.duplicate_observations, 0)

    def test_forced_capture_preserves_same_hour_source_change(self):
        c = RecorderCadence()
        c.observe(DateInfo("1936.1.1.0", trusted=True, source="clock"))
        forced = c.plan(
            DateInfo("1936.1.1.0", trusted=True, source="clock"),
            force_capture_reason="process_changed")

        self.assertTrue(forced.should_capture)
        self.assertEqual(forced.reason, "process_changed")
        self.assertEqual(forced.base_hour,
                         schema.date_to_hour_index("1936.1.1.0"))
        c.commit(forced)
        self.assertEqual(c.captures, 2)
        self.assertEqual(c.duplicate_observations, 0)
        self.assertEqual(c.last_capture_reason, "process_changed")

    def test_forced_capture_does_not_hide_missed_scheduled_hours(self):
        c = RecorderCadence()
        h0 = schema.date_to_hour_index("1936.1.1.0")
        c.observe(DateInfo("1936.1.1.0", trusted=True, source="clock"))
        forced = c.plan(
            DateInfo("1936.1.1.3", trusted=True, source="clock"),
            force_capture_reason="process_changed")

        self.assertTrue(forced.should_capture)
        self.assertEqual(forced.reason, "skipped_hours")
        self.assertEqual(forced.skipped_delta, 2)
        c.commit(forced)
        self.assertEqual(c.skipped_hours, 2)
        self.assertEqual(c.skip_ranges, [{
            "start_hour": h0 + 1,
            "end_hour": h0 + 2,
            "count": 2,
        }])

    def test_untrusted_clock_waits_without_capture(self):
        c = RecorderCadence()
        decision = c.observe(DateInfo(None, trusted=False, source="none"))

        self.assertFalse(decision.should_capture)
        self.assertEqual(decision.reason, "untrusted_time")
        self.assertEqual(c.captures, 0)
        self.assertEqual(c.untrusted_observations, 1)
        self.assertEqual(c.status_label(), "clock_untrusted")

    def test_untrusted_startup_then_trusted_capture_keeps_warning(self):
        c = RecorderCadence()
        c.observe(DateInfo(None, trusted=False, source="none"))
        c.observe(DateInfo("1936.1.1.0", trusted=True, source="clock"))

        man = c.to_manifest()["cadence"]
        self.assertEqual(man["status"], "coverage_warning")
        self.assertEqual(man["startup_untrusted_observations"], 1)
        self.assertEqual(man["coverage_warnings"], [{
            "code": "startup_clock_untrusted",
            "untrusted_observations": 1,
        }])

    def test_untrusted_after_failed_first_capture_counts_as_startup_warning(self):
        c = RecorderCadence()
        c.plan(DateInfo("1936.1.1.0", trusted=True, source="clock"))
        c.observe(DateInfo(None, trusted=False, source="none"))
        c.observe(DateInfo("1936.1.1.0", trusted=True, source="clock"))

        man = c.to_manifest()["cadence"]
        self.assertEqual(man["status"], "coverage_warning")
        self.assertEqual(man["startup_untrusted_observations"], 1)


if __name__ == "__main__":
    unittest.main()
