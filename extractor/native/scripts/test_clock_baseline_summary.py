#!/usr/bin/env python3
"""Tests for clock_baseline_summary.py.

Every fixture is shaped like the two files the harness actually appends:
`settle_control.jsonl` records (`hoi4-settle-control-sample-v1`, `*_final` pause
fields only) and witness records (`hoi4-passive-client-state-witness-v2`, which
also carry `*_in_window`). The numbers in `test_matches_the_hand_derivation_of_
run_t19` are the real t19 totals, so the aggregation is pinned to a measurement
someone already checked by hand rather than to itself.

Each test names the production change that turns it red.
"""

from __future__ import annotations

import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("clock_baseline_summary.py")
SPEC = importlib.util.spec_from_file_location("clock_baseline_summary", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
BASELINE = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = BASELINE
SPEC.loader.exec_module(BASELINE)


def settle_sample(hour_delta, window_ms, paused=False, pause_forced=False):
    return {
        "schema": "hoi4-settle-control-sample-v1",
        "phase": "settle_no_collector",
        "hour_delta": hour_delta,
        "observed_window_ms": window_ms,
        "paused_final": paused,
        "pause_forced_final": pause_forced,
        # Set on essentially every sample of a healthy speed-5 run; present so a
        # test goes red if it is ever made an exclusion.
        "tick_inhibited_final": True,
        "advancing": hour_delta > 0,
    }


def witness_sample(hour_delta, window_ms, phase="capture", paused=False,
                   pause_forced=False, at_ms=None):
    """One witness row. `at_ms` closes its bracket; None means "let RunDir place
    it", which packs the samples tightly inside the dwell the way a real run
    does."""
    row = {
        "schema": "hoi4-passive-client-state-witness-v2",
        "phase": phase,
        "hour_delta": hour_delta,
        "observed_window_ms": window_ms,
        "paused_in_window": paused,
        "pause_forced_in_window": pause_forced,
        "paused_final": paused,
        "pause_forced_final": pause_forced,
        "tick_inhibited_in_window": True,
        "advancing": hour_delta > 0,
    }
    if at_ms is not None:
        row["window_started_at_unix_ms"] = at_ms - window_ms
        row["observed_at_unix_ms"] = at_ms
    return row


def span_row(at_ms, hour_a, cpu_ms, phase="capture", pid=1, paused=False,
             creation=7_000_000, drop=()):
    """A witness row carrying the fields the SPAN term differences.

    `hour_a` (the game clock at sample time), `processor_milliseconds` (the
    target's cumulative `TotalProcessorTime`), `pid` and
    `process_creation_filetime_utc` are on every row a live harness writes; a
    fixture without them cannot exercise the span estimator at all. `drop`
    removes named fields, which is how the missing-counter cases are built.
    """
    row = witness_sample(1, 50, phase=phase, paused=paused, at_ms=at_ms)
    row["hour_a"] = hour_a
    row["processor_milliseconds"] = cpu_ms
    row["pid"] = pid
    row["process_creation_filetime_utc"] = creation
    for field in drop:
        row.pop(field, None)
    return row


# Where a fixture run's dwell starts, and how long the recorder was asked to
# hold it. Both are arbitrary but they must be REAL: every witness row a live
# harness writes carries a timestamp, and the ready receipt carries the dwell,
# so a fixture without them describes a run that cannot exist and would let the
# attach bound go permanently untested.
ATTACH_START_MS = 1_700_000_000_000
DWELL_MS = 60_000


class RunDir:
    """A throwaway run directory holding the two JSONL files, BOM and all."""

    def __init__(self, settle=(), witness=(), dwell_ms=DWELL_MS,
                 ready_receipt=True):
        self._tmp = tempfile.TemporaryDirectory()
        self.path = Path(self._tmp.name)
        self._write("settle_control.jsonl", self._stamped_settle(settle))
        self._write("passive_speed5_witness.jsonl",
                    self._stamped(witness))
        if ready_receipt:
            (self.path / BASELINE.READY_RECEIPT_FILE).write_text(
                json.dumps({"schema": "hoi4-capture-ready-v2",
                            "requested_duration_ms": dwell_ms}),
                encoding="utf-8",
            )

    @staticmethod
    def _stamped_settle(rows):
        """Put the settle control on a clock ending exactly at ATTACH_START_MS.

        The LAST settle sample is the dwell anchor, because it is the last
        instant provably before the recorder was launched. Placing them so the
        last one lands on ATTACH_START_MS is what makes the witness fixtures
        below read naturally as offsets into the dwell.
        """
        rows = list(rows)
        stamped = []
        for index, row in enumerate(rows):
            if "observed_at_unix_ms" in row:
                stamped.append(row)
                continue
            row = dict(row)
            row["observed_at_unix_ms"] = ATTACH_START_MS - (len(rows) - 1 - index) * 100
            stamped.append(row)
        return stamped

    @staticmethod
    def _stamped(rows):
        """Place every unstamped witness row on a clock, inside the dwell.

        A row the caller stamped itself is left alone -- that is how the
        out-of-window tests put a sample past detach.
        """
        rows = list(rows)
        if not rows:
            return rows
        stamped = []
        cursor = ATTACH_START_MS
        for row in rows:
            if "observed_at_unix_ms" in row:
                stamped.append(row)
                continue
            row = dict(row)
            window = int(row.get("observed_window_ms", 0) or 0)
            cursor += window + 10
            row["window_started_at_unix_ms"] = cursor - window
            row["observed_at_unix_ms"] = cursor
            stamped.append(row)
        return stamped

    def _write(self, name, rows):
        if not rows:
            return
        # utf-8-sig: Windows PowerShell 5.1's Add-Content -Encoding UTF8 writes a
        # BOM, and a reader that does not strip it fails on the first line of
        # every real run.
        with open(self.path / name, "w", encoding="utf-8-sig") as handle:
            for row in rows:
                handle.write(json.dumps(row) + "\n")

    def summary(self):
        return BASELINE.summarize(str(self.path))

    def close(self):
        self._tmp.cleanup()


class ClockBaselineSummaryTests(unittest.TestCase):
    def summarize(self, settle=(), witness=()):
        run = RunDir(settle, witness)
        self.addCleanup(run.close)
        return run.summary()

    def test_matches_the_hand_derivation_of_run_t19(self):
        """The real t19 totals: 9 h / 496 ms free-run, 80 h / 6859 ms attached.

        Red if the aggregation divides per sample and averages, or divides by
        anything other than the summed observed window.
        """
        summary = self.summarize(
            settle=[settle_sample(1, 55) for _ in range(9)],
            witness=[witness_sample(1, 85) for _ in range(80)],
        )
        self.assertAlmostEqual(
            summary["free_run"]["hours_per_second"], 9 * 1000.0 / 495, places=6
        )
        self.assertAlmostEqual(
            summary["with_collector"]["hours_per_second"], 80 * 1000.0 / 6800, places=6
        )
        self.assertAlmostEqual(
            summary["retained_fraction"],
            (80 * 1000.0 / 6800) / (9 * 1000.0 / 495),
            places=6,
        )
        self.assertAlmostEqual(
            summary["loss_fraction"], 1.0 - summary["retained_fraction"], places=9
        )
        self.assertEqual(summary["problems"], [])

    def test_long_windows_outweigh_short_ones(self):
        """Summing then dividing, not averaging per-sample rates.

        A mean of rates would score these two samples at (10 + 0.5) / 2 = 5.25
        h/s. The run observed 6 hours over 2.1 s, which is 2.857.

        Red if `aggregate` ever computes per-sample rates.
        """
        summary = self.summarize(
            settle=[settle_sample(1, 100), settle_sample(5, 2000)],
        )
        self.assertAlmostEqual(
            summary["free_run"]["hours_per_second"], 6 * 1000.0 / 2100, places=6
        )

    def test_a_paused_window_is_not_loss(self):
        """Pause is an explanation, so it leaves both sides of the ratio.

        Red if `paused_in_sample` stops reading either the `_in_window` fields
        (witness) or the `_final` fields (settle control) -- and asymmetric
        exclusion is worse than none, because it biases the ratio.
        """
        summary = self.summarize(
            settle=[settle_sample(1, 100), settle_sample(0, 5000, paused=True)],
            witness=[
                witness_sample(1, 100),
                witness_sample(0, 5000, pause_forced=True),
            ],
        )
        self.assertEqual(summary["free_run"]["samples_excluded_paused"], 1)
        self.assertEqual(summary["with_collector"]["samples_excluded_paused"], 1)
        self.assertAlmostEqual(summary["free_run"]["hours_per_second"], 10.0, places=6)
        self.assertAlmostEqual(
            summary["with_collector"]["hours_per_second"], 10.0, places=6
        )
        self.assertAlmostEqual(summary["retained_fraction"], 1.0, places=9)

    def test_tick_inhibited_alone_never_excludes_a_sample(self):
        """It is set on every sample of a healthy run; excluding on it empties
        both sides and makes the summary permanently unknown.

        Red if `tick_inhibited*` is added to `paused_in_sample`.
        """
        summary = self.summarize(
            settle=[settle_sample(1, 100)],
            witness=[witness_sample(1, 100)],
        )
        self.assertEqual(summary["free_run"]["samples_counted"], 1)
        self.assertEqual(summary["with_collector"]["samples_counted"], 1)

    def test_only_capture_phase_samples_are_the_numerator(self):
        """`pre_attach` and `post_teardown` measure a game with no live hooks.

        Counting them dilutes the numerator toward the baseline and understates
        loss. Red if WITH_COLLECTOR_PHASES grows a hookless phase.
        """
        summary = self.summarize(
            witness=[
                witness_sample(1, 50, phase="pre_attach"),
                witness_sample(1, 50, phase="ready_wait"),
                witness_sample(1, 50, phase="ready_validated"),
                witness_sample(1, 200, phase="capture"),
                witness_sample(1, 50, phase="post_teardown"),
            ],
        )
        self.assertEqual(summary["with_collector"]["samples"], 1)
        self.assertAlmostEqual(
            summary["with_collector"]["hours_per_second"], 5.0, places=6
        )

    def test_a_capture_sample_taken_after_the_dwell_is_not_collector_cost(self):
        """THE DEFECT THIS BOUND EXISTS FOR, in miniature.

        The harness's capture-wait loop ran until the RECORDER exited, not until
        the collector detached, and labelled everything in it `capture`. The
        recorder outlives its dwell by minutes converting a multi-GB stream on
        the same box, so those samples measure the HOST slowing the game with no
        collector attached. Here one in-dwell sample says 10 h/s and three
        post-dwell samples say 1 h/s; unbounded, the run reports 1.43 h/s and a
        loss of 86%. Bounded, it reports the 10 h/s that the collector actually
        cost, and says on the receipt that it threw three samples out.

        Red if `summarize` stops intersecting the numerator with
        `attach_window`, or if `sample_inside_window` starts admitting a bracket
        that closes after the dwell.
        """
        summary = self.summarize(
            settle=[settle_sample(1, 100)],
            witness=[
                witness_sample(1, 100, at_ms=ATTACH_START_MS + 1_000),
                witness_sample(1, 1000, at_ms=ATTACH_START_MS + DWELL_MS + 30_000),
                witness_sample(1, 1000, at_ms=ATTACH_START_MS + DWELL_MS + 90_000),
                witness_sample(1, 1000, at_ms=ATTACH_START_MS + DWELL_MS + 150_000),
            ],
        )
        self.assertEqual(summary["with_collector"]["samples"], 1)
        self.assertAlmostEqual(
            summary["with_collector"]["hours_per_second"], 10.0, places=6
        )
        self.assertAlmostEqual(summary["loss_fraction"], 0.0, places=9)
        self.assertTrue(summary["attach_window"]["bounded"])
        self.assertEqual(summary["attach_window"]["samples_labelled"], 4)
        self.assertEqual(summary["attach_window"]["samples_outside"], 3)
        self.assertEqual(summary["problems"], [])

    def test_a_bracket_that_straddles_detach_is_excluded_not_counted(self):
        """A sample is a bracket, not an instant.

        This one opens 500 ms before the dwell ends and closes 500 ms after it,
        so part of what it measured had a collector attached and part did not.
        Counting it would put both costs in one number, which is the defect.

        Red if `sample_inside_window` compares only `observed_at_unix_ms`, or
        only the window start.
        """
        summary = self.summarize(
            settle=[settle_sample(1, 100)],
            witness=[
                witness_sample(1, 100, at_ms=ATTACH_START_MS + 1_000),
                witness_sample(1, 1000, at_ms=ATTACH_START_MS + DWELL_MS + 500),
            ],
        )
        self.assertEqual(summary["with_collector"]["samples"], 1)
        self.assertEqual(summary["attach_window"]["samples_outside"], 1)

    def test_a_run_with_no_dwell_on_record_says_the_rate_is_unbounded(self):
        """Fail loud, not quiet.

        Without the ready receipt the dwell is unknown, so the numerator falls
        back to every capture-labelled sample -- exactly the pre-fix behaviour.
        That number is still printed, because half a measurement beats none, but
        it carries a problem and therefore a nonzero exit code so it can never
        be quoted as if it were bounded.

        Red if the unbounded case is downgraded to a silent field.
        """
        run = RunDir(
            settle=[settle_sample(1, 100)],
            witness=[witness_sample(1, 200)],
            ready_receipt=False,
        )
        self.addCleanup(run.close)
        summary = run.summary()
        self.assertFalse(summary["attach_window"]["bounded"])
        self.assertEqual(summary["with_collector"]["samples"], 1)
        self.assertTrue(
            any("NOT bounded" in problem for problem in summary["problems"]),
            summary["problems"],
        )
        self.assertEqual(BASELINE.main(["--run-dir", str(run.path), "--no-receipt"]), 1)

    def test_a_refusal_with_no_capture_sample_is_not_called_unbounded(self):
        """There is nothing to misattribute when nothing was labelled.

        Red if the unbounded problem stops being conditioned on
        `samples_labelled`: it would then fire on every refused attempt and
        teach a reader to ignore it.
        """
        run = RunDir(settle=[settle_sample(1, 194)], ready_receipt=False)
        self.addCleanup(run.close)
        summary = run.summary()
        self.assertFalse(summary["attach_window"]["bounded"])
        self.assertEqual(summary["attach_window"]["samples_labelled"], 0)
        self.assertEqual(len(summary["problems"]), 1)
        self.assertIn("no with-collector numerator", summary["problems"][0])

    def test_the_transcript_names_how_many_samples_the_bound_removed(self):
        """A truncation nobody can see is a truncation nobody can audit.

        Red if `render` drops the attach-window line.
        """
        summary = self.summarize(
            settle=[settle_sample(1, 100)],
            witness=[
                witness_sample(1, 100, at_ms=ATTACH_START_MS + 1_000),
                witness_sample(1, 1000, at_ms=ATTACH_START_MS + DWELL_MS + 30_000),
            ],
        )
        rendered = BASELINE.render(summary)
        self.assertIn("phase=clock_baseline_attach_window", rendered)
        self.assertIn("bounded=true", rendered)
        self.assertIn("samples_labelled=2", rendered)
        self.assertIn("samples_outside=1", rendered)

    def test_the_span_term_covers_the_gaps_the_brackets_miss(self):
        """Eight 100 ms brackets see 0.8 s of a 60 s dwell; the span sees it all.

        These two samples are 10 s apart and the hour counter moved 50, so the
        run advanced at 5 h/s. Their brackets saw 100 ms each and 1 hour each,
        which reads as 10 h/s -- twice the truth, from 2% of the window. Both
        are reported; only the span is a measurement of the dwell.

        Red if `span_aggregate` goes back to summing `observed_window_ms`, or
        starts averaging per-segment rates.
        """
        rows = [
            span_row(0, hour_a=1000, cpu_ms=10_000),
            span_row(10_000, hour_a=1050, cpu_ms=30_000),
        ]
        span = BASELINE.span_aggregate(rows, "x")
        self.assertEqual(span["segments"], 1)
        self.assertEqual(span["game_hours"], 50)
        self.assertEqual(span["wall_ms"], 10_000)
        self.assertAlmostEqual(span["hours_per_second"], 5.0, places=9)
        # 20 s of CPU over 10 s of wall is two logical cores, and 400 ms of CPU
        # bought each of the 50 game-hours.
        self.assertAlmostEqual(span["target_cpu_cores"], 2.0, places=9)
        self.assertAlmostEqual(span["cpu_ms_per_game_hour"], 400.0, places=9)

    def test_a_segment_that_cannot_be_a_rate_is_dropped_not_folded(self):
        """A pause, a process change, or a counter that went backwards.

        Each of these makes a pair of rows something other than one process
        advancing, and folding any of them corrupts BOTH the rate and the CPU
        term. Only the one clean segment survives.

        Red if any of the four guards in `span_aggregate` is removed.
        """
        rows = [
            span_row(0, hour_a=100, cpu_ms=1_000),
            span_row(1_000, hour_a=110, cpu_ms=2_000),          # clean
            span_row(2_000, hour_a=120, cpu_ms=3_000, paused=True),
            span_row(3_000, hour_a=130, cpu_ms=4_000),
            span_row(4_000, hour_a=125, cpu_ms=5_000),          # hour went back
            span_row(5_000, hour_a=140, cpu_ms=4_000),          # cpu went back
            span_row(6_000, hour_a=150, cpu_ms=6_000, pid=99),  # other process
            span_row(6_000, hour_a=160, cpu_ms=7_000, pid=99),  # no wall time
        ]
        span = BASELINE.span_aggregate(rows, "x")
        self.assertEqual(span["segments"], 1)
        self.assertEqual(span["game_hours"], 10)
        self.assertEqual(span["segments_dropped"], 6)

    def test_the_dwell_is_anchored_on_the_settle_control_not_on_the_witness(self):
        """No witness sample can be proved to precede the recorder's own clock.

        The recorder's deadline starts inside the injected DLL, before the READY
        exchange and before the harness observes anything about the attach --
        and the child is already running when the harness takes its
        `attach_started` bracket, so even that one may be later. Anchoring on
        any of them can push the window's END past the real detach and re-admit
        the post-detach samples the bound exists to remove. The settle control
        is on the other side of the launch, so its LAST sample is sound.

        Here the settle control ends at T and `attach_started` is 6 s later. The
        capture sample at T+58 s is inside the recorder's 60 s dwell measured
        from T; the one at T+63 s is not, but an `attach_started` anchor would
        have called it attached.

        Red if the anchor moves back to a witness phase, if it takes anything
        other than the LAST settle sample, or if the sound anchor stops being
        preferred over the fallbacks.
        """
        summary = self.summarize(
            settle=[settle_sample(1, 100), settle_sample(1, 100)],
            witness=[
                witness_sample(1, 10, phase="attach_started",
                               at_ms=ATTACH_START_MS + 6_000),
                witness_sample(1, 10, phase="ready_validated",
                               at_ms=ATTACH_START_MS + 12_000),
                witness_sample(1, 100, at_ms=ATTACH_START_MS + 58_000),
                witness_sample(1, 100, at_ms=ATTACH_START_MS + 63_000),
            ],
        )
        self.assertEqual(summary["attach_window"]["start_source"],
                         "settle_no_collector")
        self.assertTrue(summary["attach_window"]["anchor_is_sound"])
        self.assertEqual(summary["attach_window"]["start_unix_ms"], ATTACH_START_MS)
        self.assertEqual(summary["with_collector"]["samples"], 1)
        self.assertEqual(summary["attach_window"]["samples_outside"], 1)
        self.assertEqual(summary["problems"], [])

    def test_an_unsound_anchor_is_bounded_but_says_so(self):
        """A run with no settle control can still be bounded, loudly.

        The fallback anchors are all taken after the recorder was launched, so
        the window may end after the real detach. That is the original defect in
        miniature and must not hide behind `bounded=true`.

        Red if the fallback path stops setting `anchor_is_sound=False`, or if
        that no longer raises a problem.
        """
        run = RunDir(
            witness=[
                witness_sample(1, 10, phase="attach_started", at_ms=ATTACH_START_MS),
                witness_sample(1, 100, at_ms=ATTACH_START_MS + 1_000),
            ],
        )
        self.addCleanup(run.close)
        summary = run.summary()
        self.assertTrue(summary["attach_window"]["bounded"])
        self.assertFalse(summary["attach_window"]["anchor_is_sound"])
        self.assertEqual(summary["attach_window"]["start_source"], "attach_started")
        self.assertTrue(
            any("cannot be proved to precede its dwell clock" in p
                for p in summary["problems"]),
            summary["problems"],
        )

    def test_a_repeated_anchor_phase_anchors_on_its_earliest_sample(self):
        """`ready_wait` repeats -- t23 and t26 each carry two.

        Every later sample of an anchor phase sits further from the recorder's
        own clock start, and pushing the anchor later pushes the window's END
        later by the same amount, which is the direction that admits detached
        samples. The EARLIEST sample of the earliest available phase is the only
        conservative choice.

        This run has no settle control and no `attach_started`, so the fallback
        chain is exercised: the two `ready_wait` samples are 8 s apart, and the
        capture sample at first+58 s is inside the dwell only if the earlier one
        anchors it.

        Red if the fallback anchor takes `max` instead of `min`, or if the chain
        is reordered so `ready_validated` wins over `ready_wait`.
        """
        run = RunDir(
            witness=[
                witness_sample(1, 10, phase="ready_wait", at_ms=ATTACH_START_MS),
                witness_sample(1, 10, phase="ready_wait",
                               at_ms=ATTACH_START_MS + 8_000),
                witness_sample(1, 10, phase="ready_validated",
                               at_ms=ATTACH_START_MS + 12_000),
                witness_sample(1, 100, at_ms=ATTACH_START_MS + 58_000),
                witness_sample(1, 100, at_ms=ATTACH_START_MS + 62_000),
            ],
        )
        self.addCleanup(run.close)
        summary = run.summary()
        self.assertEqual(summary["attach_window"]["start_source"], "ready_wait")
        self.assertEqual(summary["attach_window"]["start_unix_ms"], ATTACH_START_MS)
        self.assertEqual(summary["with_collector"]["samples"], 1)
        self.assertEqual(summary["attach_window"]["samples_outside"], 1)

    def test_a_segment_missing_a_counter_is_not_a_measurement(self):
        """Coercing an absent counter to zero manufactures numbers.

        Two rows with no CPU reading become an accepted "this hour cost no
        CPU"; one absent reading followed by a real one becomes a delta the
        size of the whole counter. Neither is a measurement, so neither
        contributes a segment.

        Red if `span_aggregate` goes back to coercing missing fields to 0.
        """
        rows = [
            span_row(0, hour_a=100, cpu_ms=1_000),
            span_row(1_000, hour_a=110, cpu_ms=2_000),                       # clean
            span_row(2_000, hour_a=120, cpu_ms=3_000, drop=("processor_milliseconds",)),
            span_row(3_000, hour_a=130, cpu_ms=4_000),
            span_row(4_000, hour_a=140, cpu_ms=5_000, drop=("hour_a",)),
            span_row(5_000, hour_a=150, cpu_ms=6_000),
        ]
        span = BASELINE.span_aggregate(rows, "x")
        self.assertEqual(span["segments"], 1)
        self.assertEqual(span["game_hours"], 10)
        self.assertEqual(span["target_cpu_ms"], 1_000)

    def test_a_relaunched_process_reusing_the_pid_is_not_one_run(self):
        """A pid is reused; the creation FILETIME is what tells two
        incarnations apart.

        Both rows below carry pid 1, but the second is a different launch whose
        counters restart. Folding them gives a nonsense rate and a nonsense CPU
        term.

        Red if the guard compares only `pid`.
        """
        rows = [
            span_row(0, hour_a=100, cpu_ms=9_000, creation=1),
            span_row(1_000, hour_a=110, cpu_ms=10_000, creation=2),
        ]
        self.assertEqual(BASELINE.span_aggregate(rows, "x")["segments"], 0)

    def test_a_run_that_demonstrably_paused_makes_the_span_term_unreliable(self):
        """A DETECTION, and only a detection.

        The receipt classifies only time the witness actually bracketed; every
        between-bracket gap goes to `unobserved_ms` with no state, so this
        cannot show that a pause fell in a gap, and the problem must not claim
        it did. What it says is narrower and true: this run paused where the
        instrument could see it, the span estimator is blind to gaps, and this
        run cannot settle whether one was contaminated -- so its span term is
        not reliable as collector cost. Both numbers reach the receipt: the
        detection and the exposure it is measured against.

        Red if the detection stops being read, if it is downgraded from a
        problem to a field, or if the message goes back to asserting that the
        span term overstates the collector.
        """
        run = RunDir(
            settle=[settle_sample(1, 100)],
            witness=[
                span_row(ATTACH_START_MS + 1_000, hour_a=100, cpu_ms=1_000),
                span_row(ATTACH_START_MS + 11_000, hour_a=150, cpu_ms=21_000),
            ],
        )
        self.addCleanup(run.close)
        (run.path / BASELINE.PASSIVE_INTERVALS_FILE).write_text(
            json.dumps({"paused_by_engine_ms": 4_000, "paused_unforced_ms": 0,
                        "stalled_ms": 0, "unobserved_ms": 470_000}),
            encoding="utf-8")
        summary = run.summary()
        self.assertEqual(summary["collector_cost"]["observed_pause_ms"], 4_000)
        self.assertEqual(
            summary["collector_cost"]["pause_exposure_unobserved_ms"], 470_000)
        problem = next(
            (p for p in summary["problems"] if "pause/stall" in p), None)
        self.assertIsNotNone(problem, summary["problems"])
        self.assertIn("not reliable as collector cost", problem)
        self.assertIn("470000 ms", problem)
        # The claim it must NOT make: that a hidden pause happened, or by how
        # much the span term is wrong. Asserted positively as well as
        # negatively, because "does not contain the word overstate" is trivial
        # to satisfy while still asserting contamination in other words.
        self.assertIn("cannot show whether one occurred", problem)
        self.assertNotIn("overstate", problem)

    def test_a_clean_passive_clock_receipt_raises_no_pause_problem(self):
        """The counterpart, so the check cannot be a permanent alarm.

        Red if any receipt at all is treated as evidence of a pause.
        """
        run = RunDir(
            settle=[settle_sample(1, 100)],
            witness=[
                span_row(ATTACH_START_MS + 1_000, hour_a=100, cpu_ms=1_000),
                span_row(ATTACH_START_MS + 11_000, hour_a=150, cpu_ms=21_000),
            ],
        )
        self.addCleanup(run.close)
        (run.path / BASELINE.PASSIVE_INTERVALS_FILE).write_text(
            json.dumps({"paused_by_engine_ms": 0, "paused_unforced_ms": 0,
                        "stalled_ms": 0}),
            encoding="utf-8")
        summary = run.summary()
        self.assertEqual(summary["collector_cost"]["observed_pause_ms"], 0)
        self.assertEqual(summary["problems"], [])

    def test_the_collector_cost_is_the_difference_of_the_two_span_terms(self):
        """Free-run 10 h/s at 200 ms CPU/hour, attached 5 h/s at 600 ms.

        So the collector costs 100 ms of wall per game-hour (200 -> 100 h/s
        period... 100 ms period becomes 200 ms) and 400 ms of CPU per game-hour.
        This is the arithmetic the escalation table is quoted in, done once,
        here, instead of by hand from two receipts.

        Red if `collector_cost` subtracts rates instead of periods, or reports
        the attached CPU term as the cost.
        """
        free = BASELINE.span_aggregate(
            [span_row(0, hour_a=0, cpu_ms=0),
             span_row(1_000, hour_a=10, cpu_ms=2_000)], "free")
        attached = BASELINE.span_aggregate(
            [span_row(0, hour_a=0, cpu_ms=0),
             span_row(1_000, hour_a=5, cpu_ms=3_000)], "att")
        cost = BASELINE.collector_cost(free, attached)
        self.assertAlmostEqual(cost["wall_ms_per_game_hour"], 100.0, places=6)
        self.assertAlmostEqual(cost["cpu_ms_per_game_hour"], 400.0, places=6)
        self.assertAlmostEqual(cost["cpu_cores"], 1.0, places=6)

    def test_the_span_terms_reach_the_receipt_and_the_transcript(self):
        """A number nobody can read is not instrumentation.

        Red if `summarize` drops either span block or `collector_cost`, or if
        `render` stops naming the two costs.
        """
        summary = self.summarize(
            settle=[settle_sample(1, 100)],
            witness=[
                witness_sample(1, 100, at_ms=ATTACH_START_MS + 1_000),
                witness_sample(1, 100, at_ms=ATTACH_START_MS + 11_000),
            ],
        )
        self.assertIn("free_run_span", summary)
        self.assertIn("with_collector_span", summary)
        self.assertIn("collector_cost", summary)
        rendered = BASELINE.render(summary)
        self.assertIn("phase=clock_baseline_span", rendered)
        self.assertIn("phase=clock_baseline_collector_cost", rendered)
        self.assertIn("cpu_ms_per_game_hour=", rendered)

    def test_a_refused_capture_still_reports_the_free_run_baseline(self):
        """This is run t20: settle completed, the graded window never produced a
        capture-phase sample. The denominator it measured is exactly what the
        next attempt is planned from, so it must survive on its own.

        Red if the summary refuses to report either term without the other.
        """
        summary = self.summarize(settle=[settle_sample(1, 194) for _ in range(9)])
        self.assertIsNotNone(summary["free_run"]["hours_per_second"])
        self.assertIsNone(summary["with_collector"]["hours_per_second"])
        self.assertIsNone(summary["retained_fraction"])
        self.assertEqual(len(summary["problems"]), 1)
        self.assertIn("no with-collector numerator", summary["problems"][0])

    def test_a_run_with_no_files_at_all_says_so_rather_than_dividing(self):
        summary = self.summarize()
        self.assertIsNone(summary["free_run"]["hours_per_second"])
        self.assertIsNone(summary["loss_fraction"])
        self.assertEqual(len(summary["problems"]), 2)

    def test_a_control_that_observed_no_hours_is_not_a_rate(self):
        """A zero-hour denominator would make every ratio infinite or 0/0.

        Red if the zero-hour case is allowed to fall through as a rate of 0.0.
        """
        summary = self.summarize(
            settle=[settle_sample(0, 3000), settle_sample(0, 3000)],
            witness=[witness_sample(1, 100)],
        )
        self.assertEqual(summary["free_run"]["game_hours"], 0)
        self.assertIsNone(summary["retained_fraction"])
        self.assertTrue(
            any("zero game hours" in problem for problem in summary["problems"]),
            summary["problems"],
        )

    def test_a_truncated_final_line_does_not_lose_the_whole_file(self):
        """The harness appends one sample at a time and can be killed mid-write.

        Red if `read_jsonl` stops tolerating an unparseable line.
        """
        run = RunDir(settle=[settle_sample(1, 100), settle_sample(1, 100)])
        self.addCleanup(run.close)
        path = run.path / "settle_control.jsonl"
        with open(path, "a", encoding="utf-8") as handle:
            handle.write('{"phase": "settle_no_collector", "hour_de')
        summary = run.summary()
        self.assertEqual(summary["free_run"]["samples"], 2)
        self.assertAlmostEqual(summary["free_run"]["hours_per_second"], 10.0, places=6)

    def test_a_malformed_interior_line_is_named_not_swallowed(self):
        """A kill can only truncate the LAST line; anything else is real damage.

        Red if `read_jsonl` goes back to `except ValueError: continue` for every
        line: the interior sample would vanish from the denominator and the tool
        would print a confident 10.0 hours/second and exit 0.
        """
        run = RunDir(settle=[settle_sample(1, 100), settle_sample(1, 100)])
        self.addCleanup(run.close)
        path = run.path / "settle_control.jsonl"
        good = path.read_text("utf-8-sig").splitlines()
        path.write_text(
            good[0] + "\n" + '{"phase": "settle_no_col' + "\n" + good[1] + "\n",
            encoding="utf-8",
        )
        summary = run.summary()
        self.assertEqual(summary["free_run"]["samples"], 2)
        self.assertTrue(
            any("line 2 is not JSON" in problem for problem in summary["problems"]),
            summary["problems"],
        )
        self.assertEqual(BASELINE.main(["--run-dir", str(run.path), "--no-receipt"]), 1)

    def test_an_existing_receipt_is_not_overwritten(self):
        """A run directory is evidence; re-reading it must not rewrite it.

        Red if the receipt is opened `"w"` again: the sentinel below is replaced
        by a fresh summary.
        """
        run = RunDir(settle=[settle_sample(1, 100)], witness=[witness_sample(1, 200)])
        self.addCleanup(run.close)
        receipt = run.path / BASELINE.RECEIPT_NAME
        receipt.write_text('{"schema": "already-published"}', encoding="utf-8")
        self.assertEqual(BASELINE.main(["--run-dir", str(run.path)]), 0)
        self.assertEqual(
            json.loads(receipt.read_text("utf-8"))["schema"], "already-published"
        )

    def test_the_transcript_line_carries_both_terms_and_the_ratio(self):
        """The whole point is that no operator hand-derives these.

        Red if `render` stops naming any of the four numbers.
        """
        summary = self.summarize(
            settle=[settle_sample(1, 100)], witness=[witness_sample(1, 200)]
        )
        rendered = BASELINE.render(summary)
        self.assertIn("free_run_hours_per_second=10.0000", rendered)
        self.assertIn("with_collector_hours_per_second=5.0000", rendered)
        self.assertIn("retained_fraction=0.5000", rendered)
        self.assertIn("loss_fraction=0.5000", rendered)

    def test_main_writes_a_receipt_and_reports_incompleteness_through_its_code(self):
        run = RunDir(settle=[settle_sample(1, 100)], witness=[witness_sample(1, 200)])
        self.addCleanup(run.close)
        self.assertEqual(BASELINE.main(["--run-dir", str(run.path)]), 0)
        receipt = json.loads((run.path / BASELINE.RECEIPT_NAME).read_text("utf-8"))
        self.assertEqual(receipt["schema"], BASELINE.SCHEMA)
        self.assertAlmostEqual(receipt["retained_fraction"], 0.5, places=9)

        bare = RunDir()
        self.addCleanup(bare.close)
        self.assertEqual(BASELINE.main(["--run-dir", str(bare.path)]), 1)


if __name__ == "__main__":
    unittest.main()
