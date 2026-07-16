"""Focused tests for GUI recorder cadence orchestration."""
from __future__ import annotations

import os
import shutil
import sys
import tempfile
import unittest
from unittest import mock

_HERE = os.path.dirname(os.path.abspath(__file__))
_REPO = os.path.normpath(os.path.join(_HERE, os.pardir, os.pardir))
if _REPO not in sys.path:
    sys.path.insert(0, _REPO)

from live_capture.capture import CaptureResult, DateInfo  # noqa: E402
from live_capture.recorder.cadence import RecorderCadence  # noqa: E402
from live_capture.recorder.recorder_main import (  # noqa: E402
    CADENCE_EVERY_2_HOURS_LABEL,
    CADENCE_EVERY_6_HOURS_LABEL,
    CADENCE_MANUAL_LABEL,
    CADENCE_MODE_LABEL,
    CADENCE_MODE_LABELS,
    DEFAULT_POLL_INTERVAL_S,
    DEFAULT_TARGET_STEP_HOURS,
    _RecorderWorker,
    parse_cadence_mode,
)
from replay import schema  # noqa: E402


class _FakeProc:
    def __init__(self, pid=1):
        self.pid = pid
        self.closed = False

    def close(self):
        self.closed = True


class _FakeStore:
    def __init__(self, root):
        self.root = root

    def lock_path(self, game_id):
        return os.path.join(self.root, "_continuity", "%s.lock" % game_id)


class _FakeRecorder:
    def __init__(self):
        self.finalize_calls = 0

    def finalize(self):
        self.finalize_calls += 1


class _FakeClock:
    def __init__(self, values):
        self.values = list(values)
        self.i = 0

    def __call__(self):
        if self.i >= len(self.values):
            return self.values[-1]
        value = self.values[self.i]
        self.i += 1
        return value


class _FakeLoop:
    def __init__(self, worker, root, *, fail_calls=(), pids=None,
                 finalize_every=6):
        self.worker = worker
        self.store = _FakeStore(root)
        self._recorder = _FakeRecorder()
        self._finalize_every = finalize_every
        self.fail_calls = set(fail_calls)
        self.calls = 0
        self.open_calls = 0
        self.pids = list(pids or [1])
        self.attempted_dates = []
        self.captured_dates = []
        self.prev_snapshots = []
        self.failed_skipped_hours = []
        self.last_finalize_error = None

    def open_proc(self):
        idx = min(self.open_calls, len(self.pids) - 1)
        self.open_calls += 1
        return _FakeProc(pid=self.pids[idx])

    def capture_once(self, *, date_info=None, prev_snapshot=None, proc=None,
                     pre_write_check=None):
        del proc
        call = self.calls
        self.calls += 1
        self.attempted_dates.append(date_info.date)
        self.prev_snapshots.append(prev_snapshot)
        if call in self.fail_calls:
            self.failed_skipped_hours.append(
                self.worker._cadence.skipped_hours)
            raise RuntimeError("boom")

        res = CaptureResult(
            written_path="snapshot-%d.json" % call,
            snapshot={"date": date_info.date, "seq": call},
            domains_captured=[],
            tags={},
            canonical=True,
        )
        if pre_write_check is not None:
            err = pre_write_check(res.snapshot)
            if err:
                return CaptureResult(
                    written_path=None,
                    snapshot=res.snapshot,
                    domains_captured=[],
                    tags={},
                    canonical=False,
                    error=err,
                )
        self.captured_dates.append(date_info.date)
        return res


class TestRecorderWorkerCadence(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="rec_worker_")
        self.statuses = []

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def _worker(self, *, target_step_hours=DEFAULT_TARGET_STEP_HOURS,
                cadence_label=CADENCE_MODE_LABEL, manual_once=False):
        worker = _RecorderWorker("worker_cadence", self.tmp, 0.0,
                                 self.statuses.append,
                                 target_step_hours=target_step_hours,
                                 cadence_label=cadence_label,
                                 manual_once=manual_once)
        worker._cadence = RecorderCadence(
            target_step_hours=target_step_hours)
        return worker

    def _run_sequence(self, dates, *, fail_calls=(),
                      target_step_hours=DEFAULT_TARGET_STEP_HOURS,
                      cadence_label=CADENCE_MODE_LABEL,
                      manual_once=False, monotonic_values=None, pids=None,
                      finalize_every=6):
        worker = self._worker(target_step_hours=target_step_hours,
                              cadence_label=cadence_label,
                              manual_once=manual_once)
        if monotonic_values is not None:
            worker._monotonic = _FakeClock(monotonic_values)
        loop = _FakeLoop(worker, self.tmp, fail_calls=fail_calls, pids=pids,
                         finalize_every=finalize_every)
        worker._loop = loop
        # Cadence unit fixtures do not build a real replay container.  Keep the
        # diagnostics publisher outside these cadence-only tests; its strict
        # failure behavior is covered explicitly below and in the manifest tests.
        worker._patch_recording_diagnostics = lambda **_kwargs: True
        seq = list(dates)
        pos = {"i": 0}

        def date_provider(proc):
            del proc
            di = seq[pos["i"]]
            pos["i"] += 1
            if pos["i"] >= len(seq):
                worker.stop()
            return di

        def on_capture(res):
            if res.error:
                return
            if res.written_path is not None and res.canonical:
                worker.n_captured += 1

        worker._run_until_stop(date_provider, on_capture)
        return worker, loop

    def test_failed_capture_retries_same_hour_instead_of_duplicate(self):
        h0 = DateInfo("1936.1.1.0", trusted=True, source="clock")
        worker, loop = self._run_sequence([h0, h0, h0], fail_calls={0})

        self.assertEqual(loop.attempted_dates, ["1936.1.1.0", "1936.1.1.0"])
        self.assertEqual(loop.captured_dates, ["1936.1.1.0"])
        self.assertEqual(worker.n_captured, 1)
        self.assertEqual(worker._cadence.captures, 1)
        self.assertEqual(worker._cadence.duplicate_observations, 0)

    def test_skipped_hour_counters_update_only_after_successful_write(self):
        h0 = DateInfo("1936.1.1.0", trusted=True, source="clock")
        h3 = DateInfo("1936.1.1.3", trusted=True, source="clock")
        worker, loop = self._run_sequence([h0, h0, h3, h3, h3],
                                          fail_calls={1})

        self.assertEqual(loop.failed_skipped_hours, [0])
        self.assertEqual(loop.captured_dates, ["1936.1.1.0", "1936.1.1.3"])
        self.assertEqual(worker._cadence.skipped_hours, 2)
        self.assertEqual(worker._cadence.skip_events, 1)

    def test_sequence_dedups_and_records_one_skipped_hour(self):
        dates = [
            DateInfo("1936.1.1.0", trusted=True, source="clock"),
            DateInfo("1936.1.1.0", trusted=True, source="clock"),
            DateInfo("1936.1.1.0", trusted=True, source="clock"),
            DateInfo("1936.1.1.1", trusted=True, source="clock"),
            DateInfo("1936.1.1.1", trusted=True, source="clock"),
            DateInfo("1936.1.1.3", trusted=True, source="clock"),
            DateInfo("1936.1.1.3", trusted=True, source="clock"),
        ]
        worker, loop = self._run_sequence(dates)

        h2 = schema.date_to_hour_index("1936.1.1.2")
        self.assertEqual(loop.captured_dates, [
            "1936.1.1.0",
            "1936.1.1.1",
            "1936.1.1.3",
        ])
        self.assertEqual(worker.n_captured, 3)
        self.assertEqual(worker._cadence.duplicate_observations, 1)
        self.assertEqual(worker._cadence.skipped_hours, 1)
        self.assertEqual(worker._cadence.skip_ranges, [{
            "start_hour": h2,
            "end_hour": h2,
            "count": 1,
        }])

    def test_hourly_worker_captures_all_observed_hours_across_day_boundary(self):
        hourly_dates = (["1936.1.1.%d" % h for h in range(24)]
                        + ["1936.1.2.0"])
        # Each capture performs a pre-write clock sentinel, so provide the same
        # trusted hour twice: once for planning and once for the stable sentinel.
        dates = []
        for d in hourly_dates:
            dates.extend([
                DateInfo(d, trusted=True, source="clock"),
                DateInfo(d, trusted=True, source="clock"),
            ])
        worker, loop = self._run_sequence(dates)

        self.assertEqual(loop.captured_dates, hourly_dates)
        self.assertEqual(worker.n_captured, len(hourly_dates))
        self.assertEqual(worker._cadence.captures, len(hourly_dates))
        self.assertEqual(worker._cadence.skipped_hours, 0)
        self.assertEqual(worker._cadence.skip_events, 0)
        self.assertEqual(worker._cadence.capture_clock_crossings, 0)
        self.assertEqual(
            worker._diagnostics["cadence"]["target_step_hours"],
            DEFAULT_TARGET_STEP_HOURS)
        self.assertEqual(worker._diagnostics["cadence"]["status"], "ok")

    def test_process_change_captures_same_hour_instead_of_deduping(self):
        h0 = DateInfo("1936.1.1.0", trusted=True, source="clock")
        worker, loop = self._run_sequence([h0, h0, h0, h0],
                                          pids=[101, 202])

        self.assertEqual(loop.captured_dates, [
            "1936.1.1.0",
            "1936.1.1.0",
        ])
        self.assertEqual(worker.n_captured, 2)
        self.assertEqual(worker._cadence.captures, 2)
        self.assertEqual(worker._cadence.duplicate_observations, 0)
        self.assertEqual(worker._cadence.last_capture_reason,
                         "process_changed")

    def test_process_change_captures_even_when_selected_cadence_not_due(self):
        h0 = DateInfo("1936.1.1.0", trusted=True, source="clock")
        h1 = DateInfo("1936.1.1.1", trusted=True, source="clock")
        worker, loop = self._run_sequence(
            [h0, h0, h1, h1],
            target_step_hours=6,
            cadence_label=CADENCE_EVERY_6_HOURS_LABEL,
            pids=[101, 202])

        self.assertEqual(loop.captured_dates, [
            "1936.1.1.0",
            "1936.1.1.1",
        ])
        self.assertEqual(worker.n_captured, 2)
        self.assertEqual(worker._cadence.cadence_wait_observations, 0)
        self.assertEqual(worker._cadence.skipped_hours, 0)
        self.assertEqual(worker._cadence.last_capture_reason,
                         "process_changed")

    def test_worker_batches_container_finalize_between_captures(self):
        hourly_dates = ["1936.1.1.%d" % h for h in range(4)]
        dates = []
        for d in hourly_dates:
            dates.extend([
                DateInfo(d, trusted=True, source="clock"),
                DateInfo(d, trusted=True, source="clock"),
            ])

        worker, loop = self._run_sequence(dates, finalize_every=3)

        self.assertEqual(worker.n_captured, 4)
        self.assertEqual(loop._recorder.finalize_calls, 2)

    def test_worker_final_publication_failure_propagates(self):
        h0 = DateInfo("1936.1.1.0", trusted=True, source="clock")
        worker = self._worker()
        loop = _FakeLoop(worker, self.tmp, finalize_every=6)
        worker._loop = loop

        def fail_finalize():
            loop._recorder.finalize_calls += 1
            raise ValueError("candidate verification failed")

        loop._recorder.finalize = fail_finalize
        observations = [h0, h0]
        pos = {"i": 0}

        def date_provider(_proc):
            value = observations[pos["i"]]
            pos["i"] += 1
            if pos["i"] >= len(observations):
                worker.stop()
            return value

        def on_capture(res):
            if not res.error and res.written_path and res.canonical:
                worker.n_captured += 1

        with self.assertRaisesRegex(RuntimeError,
                                    "replay finalization failed"):
            worker._run_until_stop(date_provider, on_capture)
        self.assertEqual(loop.last_finalize_error,
                         "candidate verification failed")

    def test_gui_reports_publication_failure_and_never_saved(self):
        worker = self._worker()

        def fail_after_capture(_date_provider, _on_capture):
            worker.n_captured = 1
            raise RuntimeError("replay finalization failed: rejected candidate")

        worker._run_until_stop = fail_after_capture
        with mock.patch("live_capture.capture.CaptureLoop",
                        return_value=object()), \
                mock.patch("traceback.print_exc"):
            worker._run()

        joined = " ".join(self.statuses)
        self.assertIn("Could not record: replay finalization failed", joined)
        self.assertNotIn("Saved:", joined)

    def test_strict_diagnostics_failure_propagates_to_gui_caller(self):
        worker = self._worker()
        worker._loop = _FakeLoop(worker, self.tmp)
        worker._diagnostics = {"cadence": {
            "skipped_hours": 1,
            "coverage_warnings": [],
            "timing": {"warning_count": 0},
        }}
        with mock.patch(
                "live_capture.recorder.manifest_diagnostics."
                "patch_manifest_diagnostics",
                side_effect=ValueError("rejected diagnostics candidate")):
            with self.assertRaisesRegex(
                    RuntimeError, "diagnostics publication failed"):
                worker._patch_recording_diagnostics(
                    force=True, fail_closed=True)
        self.assertIn("rejected diagnostics candidate",
                      worker._loop.last_finalize_error)

    def test_every_two_worker_waits_and_skips_one_scheduled_target(self):
        dates = [
            DateInfo("1936.1.1.0", trusted=True, source="clock"),
            DateInfo("1936.1.1.0", trusted=True, source="clock"),
            DateInfo("1936.1.1.1", trusted=True, source="clock"),
            DateInfo("1936.1.1.4", trusted=True, source="clock"),
            DateInfo("1936.1.1.4", trusted=True, source="clock"),
        ]
        worker, loop = self._run_sequence(
            dates,
            target_step_hours=2,
            cadence_label=CADENCE_EVERY_2_HOURS_LABEL)

        h0 = schema.date_to_hour_index("1936.1.1.0")
        self.assertEqual(loop.captured_dates, [
            "1936.1.1.0",
            "1936.1.1.4",
        ])
        self.assertEqual(worker.n_captured, 2)
        self.assertEqual(worker._cadence.cadence_wait_observations, 1)
        self.assertEqual(worker._cadence.skipped_hours, 1)
        self.assertEqual(worker._cadence.skip_ranges, [{
            "start_hour": h0 + 2,
            "end_hour": h0 + 2,
            "count": 1,
            "step_hours": 2,
        }])
        self.assertIn(CADENCE_EVERY_2_HOURS_LABEL,
                      " ".join(self.statuses))

    def test_manual_mode_captures_one_canonical_snapshot_then_stops(self):
        h0 = DateInfo("1936.1.1.0", trusted=True, source="clock")
        h1 = DateInfo("1936.1.1.1", trusted=True, source="clock")
        worker, loop = self._run_sequence(
            [h0, h0, h1, h1],
            cadence_label=CADENCE_MANUAL_LABEL,
            manual_once=True)

        self.assertTrue(worker._stop.is_set())
        self.assertEqual(loop.captured_dates, ["1936.1.1.0"])
        self.assertEqual(worker.n_captured, 1)
        self.assertTrue(worker._diagnostics["cadence"]["manual_once"])
        self.assertEqual(
            worker._diagnostics["cadence"]["recorder_mode_label"],
            CADENCE_MANUAL_LABEL)

    def test_hour_crossing_rejects_before_durable_write(self):
        h0 = DateInfo("1936.1.1.0", trusted=True, source="clock")
        h1 = DateInfo("1936.1.1.1", trusted=True, source="clock")
        worker, loop = self._run_sequence([h0, h1, h1, h1])

        self.assertEqual(loop.attempted_dates, [
            "1936.1.1.0",
            "1936.1.1.1",
        ])
        self.assertEqual(loop.captured_dates, ["1936.1.1.1"])
        self.assertEqual(worker.n_captured, 1)
        self.assertEqual(worker._cadence.capture_clock_checks, 2)
        self.assertEqual(worker._cadence.capture_clock_crossings, 1)
        self.assertEqual(worker._cadence.last_captured_hour,
                         schema.date_to_hour_index("1936.1.1.1"))
        cadence = worker._diagnostics["cadence"]
        self.assertEqual(cadence["status"], "coverage_warning")
        self.assertEqual(
            cadence["capture_clock_sentinel"]["hour_crossings"], 1)
        self.assertEqual(
            cadence["capture_clock_sentinel"]["mode"],
            "pre_write_check_v1")
        self.assertEqual(
            cadence["capture_clock_sentinel"]["action"],
            "reject_before_durable_write")
        self.assertIn("rejected 1 capture", " ".join(self.statuses))
        self.assertIn("before write", " ".join(self.statuses))

    def test_previous_snapshot_is_kept_between_worker_captures(self):
        h0 = DateInfo("1936.1.1.0", trusted=True, source="clock")
        h1 = DateInfo("1936.1.1.1", trusted=True, source="clock")
        worker, loop = self._run_sequence([h0, h0, h1, h1])

        self.assertEqual(worker.n_captured, 2)
        self.assertIsNone(loop.prev_snapshots[0])
        self.assertEqual(loop.prev_snapshots[1], {"date": "1936.1.1.0",
                                                  "seq": 0})

    def test_worker_reports_capture_timing_when_keeping_up(self):
        h0 = DateInfo("1936.1.1.0", trusted=True, source="clock")
        h1 = DateInfo("1936.1.1.1", trusted=True, source="clock")
        worker, loop = self._run_sequence(
            [h0, h0, h1, h1],
            monotonic_values=[0.0, 0.0, 5.0, 30.0, 30.0, 40.0])

        self.assertEqual(worker.n_captured, 2)
        self.assertEqual(loop.captured_dates, ["1936.1.1.0", "1936.1.1.1"])
        timing = worker._diagnostics["cadence"]["timing"]
        self.assertEqual(timing["last_capture_duration_ms"], 10000)
        self.assertEqual(timing["observed_game_hour_interval_ms"], 30000)
        self.assertTrue(timing["keeping_up"])
        self.assertEqual(timing["status"], "keeping_up")
        self.assertIn("keeping up", " ".join(self.statuses))

    def test_duplicate_polls_do_not_shorten_observed_hour_interval(self):
        h0 = DateInfo("1936.1.1.0", trusted=True, source="clock")
        h1 = DateInfo("1936.1.1.1", trusted=True, source="clock")
        worker, loop = self._run_sequence(
            [h0, h0, h0, h1, h1],
            monotonic_values=[0.0, 0.0, 5.0, 29.0, 30.0, 30.0, 40.0])

        self.assertEqual(worker.n_captured, 2)
        self.assertEqual(loop.captured_dates, ["1936.1.1.0", "1936.1.1.1"])
        self.assertEqual(worker._cadence.duplicate_observations, 1)
        timing = worker._diagnostics["cadence"]["timing"]
        self.assertEqual(timing["last_capture_duration_ms"], 10000)
        self.assertEqual(timing["observed_game_hour_interval_ms"], 30000)
        self.assertTrue(timing["keeping_up"])
        self.assertEqual(timing["warning_count"], 0)

    def test_worker_warns_when_capture_is_slower_than_observed_hour(self):
        h0 = DateInfo("1936.1.1.0", trusted=True, source="clock")
        h1 = DateInfo("1936.1.1.1", trusted=True, source="clock")
        worker, loop = self._run_sequence(
            [h0, h0, h1, h1],
            monotonic_values=[0.0, 0.0, 1.0, 3.0, 3.0, 7.0])

        self.assertEqual(worker.n_captured, 2)
        self.assertEqual(loop.captured_dates, ["1936.1.1.0", "1936.1.1.1"])
        cadence = worker._diagnostics["cadence"]
        timing = cadence["timing"]
        self.assertEqual(timing["last_capture_duration_ms"], 4000)
        self.assertEqual(timing["observed_game_hour_interval_ms"], 3000)
        self.assertFalse(timing["keeping_up"])
        self.assertEqual(timing["status"], "not_keeping_up")
        self.assertEqual(timing["warning_count"], 1)
        self.assertEqual(
            timing["last_warning"]["code"],
            "capture_slower_than_game_hour")
        self.assertEqual(cadence["coverage_warnings"][-1]["code"],
                         "capture_slower_than_game_hour")
        self.assertIn("not keeping up", " ".join(self.statuses))
        self.assertIn("Lower game speed", " ".join(self.statuses))

    def test_capture_hour_crossing_rejects_snapshot(self):
        h0 = DateInfo("1936.1.1.0", trusted=True, source="clock")
        h1 = DateInfo("1936.1.1.1", trusted=True, source="clock")
        worker, loop = self._run_sequence([h0, h1])

        cadence = worker._diagnostics["cadence"]
        self.assertEqual(worker.n_captured, 0)
        self.assertEqual(loop.captured_dates, [])
        self.assertEqual(loop.attempted_dates, ["1936.1.1.0"])
        self.assertEqual(cadence["capture_clock_sentinel"]["hour_crossings"],
                         1)
        self.assertEqual(cadence["coverage_warnings"][0]["code"],
                         "capture_clock_hour_crossed")
        self.assertIn("rejected 1 capture", " ".join(self.statuses))

    def test_worker_default_cadence_settings_are_explicit(self):
        self.assertEqual(CADENCE_MODE_LABEL, "Every in-game hour")
        self.assertEqual(CADENCE_MODE_LABELS, (
            CADENCE_MODE_LABEL,
            CADENCE_EVERY_2_HOURS_LABEL,
            CADENCE_EVERY_6_HOURS_LABEL,
            CADENCE_MANUAL_LABEL,
        ))
        self.assertEqual(parse_cadence_mode("").label, CADENCE_MODE_LABEL)
        self.assertEqual(parse_cadence_mode(
            CADENCE_EVERY_6_HOURS_LABEL).target_step_hours, 6)
        self.assertFalse(parse_cadence_mode(
            CADENCE_EVERY_6_HOURS_LABEL).manual_once)
        self.assertTrue(parse_cadence_mode(CADENCE_MANUAL_LABEL).manual_once)
        worker = _RecorderWorker(
            "worker_cadence", self.tmp, DEFAULT_POLL_INTERVAL_S,
            self.statuses.append,
            target_step_hours=DEFAULT_TARGET_STEP_HOURS)

        self.assertEqual(worker.interval_s, 1.0)
        self.assertEqual(worker.target_step_hours, 1)
        self.assertEqual(worker.cadence_label, CADENCE_MODE_LABEL)
        self.assertFalse(worker.manual_once)


if __name__ == "__main__":
    unittest.main()
