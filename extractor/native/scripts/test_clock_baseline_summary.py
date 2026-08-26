#!/usr/bin/env python3
"""Tests for clock_baseline_summary.py.

Every fixture is shaped like the two files the harness actually appends:
`settle_control.jsonl` records (`hoi4-settle-control-sample-v2`, `*_final` pause
fields only) and witness records (`hoi4-passive-client-state-witness-v3`, which
also carry `*_in_window`). The numbers in `test_matches_the_hand_derivation_of_
run_t19` are the real t19 totals, so the aggregation is pinned to a measurement
someone already checked by hand rather than to itself.

Each test names the production change that turns it red.
"""

from __future__ import annotations

import datetime
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


def _utc(unix_ms):
    """The ISO spelling the producer writes beside every millisecond stamp."""
    return (
        datetime.datetime.fromtimestamp(unix_ms / 1000.0, datetime.timezone.utc)
        .isoformat()
        .replace("+00:00", "Z")
    )


def settle_sample(hour_delta, window_ms, paused=False, pause_forced=False,
                  at_ms=None):
    """One settle-control row, in the COMPLETE shape the harness emits.

    Complete on purpose. `admission_problem` compares a row against the whole
    producer key set, so a partial fixture is refused -- and a fixture a live
    reader would refuse is not evidence that the reader survives a live row.
    `RunDir` fills in the timestamps, the only fields whose values depend on
    where the row sits in the run.
    """
    row = {
        "schema": "hoi4-settle-control-sample-v2",
        "phase": "settle_no_collector",
        # The process identity the span estimator joins on. Required at
        # admission, because two rows that both LACK it used to join on
        # `None == None`.
        "pid": 4242,
        "process_creation_filetime_utc": 7_000_000,
        "expected_speed": 5,
        "speed_final": 5,
        # The endpoints the delta is the difference of. Admission requires all
        # three and requires them to agree, because `aggregate` sums the delta
        # and never looks at the endpoints.
        "hour_a": 100,
        "hour_b": 100 + hour_delta,
        "hour_delta": hour_delta,
        "tick_progress_bits_a": 0x3F80_0000,
        "tick_progress_bits_b": 0x3F80_0000,
        "observed_window_ms": window_ms,
        "processor_milliseconds": 1_000,
        "paused_final": paused,
        "pause_forced_final": pause_forced,
        "tick_inhibited_final": True,
        # The v2 host-and-pacing fields. `clock_baseline_summary` reads none of
        # them and REQUIRES all of them, so a fixture without them is refused
        # before aggregation -- which is what these fixtures exist to avoid
        # testing by accident.
        "available_parallelism": 16,
        "target_processor_affinity_cores": 8,
        "target_page_fault_count": 900_000,
        "target_working_set_bytes": 3_800_000_000,
        "host_state_unavailable": None,
        "advancing": hour_delta > 0,
    }
    if at_ms is not None:
        row["observed_at_unix_ms"] = at_ms
        row["observed_at_utc"] = _utc(at_ms)
    return row


def witness_sample(hour_delta, window_ms, phase="capture", paused=False,
                   pause_forced=False, at_ms=None):
    """One witness row, in the COMPLETE shape the harness emits.

    `at_ms` closes its bracket; None means "let RunDir place it", which packs
    the samples tightly inside the dwell the way a real run does.

    Complete for the same reason `settle_sample` is: admission compares against
    the whole producer key set, and a fixture a live reader would refuse proves
    nothing about a live row.
    """
    row = {
        "schema": "hoi4-passive-client-state-witness-v3",
        "phase": phase,
        "pid": 4242,
        "process_creation_filetime_utc": 7_000_000,
        "exe_sha256": "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad",
        "observer_sha256": "0" * 64,
        "access": "PROCESS_QUERY_INFORMATION|PROCESS_VM_READ",
        "expected_speed": 5,
        "speed_initial": 5,
        "speed_final": 5,
        "hour_a": 500,
        "hour_b": 500 + hour_delta,
        "hour_delta": hour_delta,
        "tick_progress_bits_a": 0x3F80_0000,
        "tick_progress_bits_b": 0x3F80_0000,
        "observed_window_ms": window_ms,
        "clock_progress_waited_ms": 60,
        "clock_progress_budget_ms": 3_000,
        "processor_milliseconds": 1_000,
        "paused_initial": paused,
        "paused_final": paused,
        "paused_in_window": paused,
        "pause_forced_initial": pause_forced,
        "pause_forced_final": pause_forced,
        "pause_forced_in_window": pause_forced,
        "tick_inhibited_initial": True,
        "tick_inhibited_final": True,
        "tick_inhibited_in_window": True,
        # The v3 host-and-pacing fields. This summary reads none of them and
        # REQUIRES all of them, so they are here for two reasons: a fixture
        # that does not look like a real row stops being evidence that the
        # summary survives one, and a fixture missing one is refused before
        # aggregation.
        "available_parallelism": 16,
        "target_processor_affinity_cores": 8,
        "target_page_fault_count": 1_200_000,
        "target_working_set_bytes": 3_900_000_000,
        "host_state_unavailable": None,
        "advancing": hour_delta > 0,
    }
    if at_ms is not None:
        row["window_started_at_unix_ms"] = at_ms - window_ms
        row["window_started_at_utc"] = _utc(at_ms - window_ms)
        row["observed_at_unix_ms"] = at_ms
        row["observed_at_utc"] = _utc(at_ms)
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
    # Keep the endpoints consistent with the delta the fixture already set, or
    # admission refuses the row before the span estimator ever sees it.
    row["hour_b"] = hour_a + row["hour_delta"]
    row["processor_milliseconds"] = cpu_ms
    row["pid"] = pid
    row["process_creation_filetime_utc"] = creation
    for field in drop:
        row.pop(field, None)
    return row


WITNESS_FILE_NAME = "passive_speed5_witness.jsonl"



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
        return [RunDir._with_utc(row) for row in stamped]

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
        return [RunDir._with_utc(row) for row in stamped]

    @staticmethod
    def _with_utc(row):
        """Fill each ISO stamp from the millisecond stamp beside it.

        The producer writes both spellings of every instant it records, and
        admission compares against the whole key set -- but the millisecond
        form is the one the fixtures place, because it is the one the tests do
        arithmetic on. Rather than make every fixture carry a redundant string,
        derive it here from whatever placement the row ended up with.
        """
        row = dict(row)
        for stamp, spelled in (
            ("observed_at_unix_ms", "observed_at_utc"),
            ("window_started_at_unix_ms", "window_started_at_utc"),
        ):
            # `isinstance` and not `in`: the null and wrong-type tests put a
            # None or a string in the millisecond field on purpose, and
            # deriving from one of those would make the fixture raise instead
            # of reaching the gate under test.
            if isinstance(row.get(stamp), int) and not isinstance(row[stamp], bool):
                row.setdefault(spelled, _utc(row[stamp]))
        return row

    def _write(self, name, rows):
        if not rows:
            return
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
            span_row(1_000, hour_a=110, cpu_ms=2_000),
            span_row(2_000, hour_a=120, cpu_ms=3_000, paused=True),
            span_row(3_000, hour_a=130, cpu_ms=4_000),
            span_row(4_000, hour_a=125, cpu_ms=5_000),
            span_row(5_000, hour_a=140, cpu_ms=4_000),
            span_row(6_000, hour_a=150, cpu_ms=6_000, pid=99),
            span_row(6_000, hour_a=160, cpu_ms=7_000, pid=99),
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
            span_row(1_000, hour_a=110, cpu_ms=2_000),
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


class SchemaGateTests(unittest.TestCase):
    """The summary reads ONE row version, and says so when handed another.

    Lane TJ moved the witness row to `...witness-v3` and the settle-control row
    to `...sample-v2` to carry host state. Codex round 1 finding 1: this reader
    filtered by phase and never looked at the schema, so a retired row version
    still produced a confident receipt and exit 0 -- a compatibility path
    nobody meant to keep, in the one place the no-backwards-compat rule was
    supposed to bite.

    Codex round 2 finding 1: the first fix REPORTED on the host fields but
    FILTERED on the schema string alone, so a current-schema row missing a host
    field was named as a problem and still reached the numerator, the
    denominator and the attach-window anchor. Every test below therefore checks
    both halves -- that the refusal is named, AND that the row contributed
    nothing.
    """

    def _run(self, settle, witness):
        run = RunDir(settle, witness)
        self.addCleanup(run.close)
        return run.summary()

    # Both arms, so a mutation that skips validation for ONE schema cannot
    # survive. Every host / rate case below is parameterised over this
    # (Codex round 5 finding 3): before it, every host mutation touched the
    # witness row only, and a production change that dropped the settle arm
    # would have gone unnoticed.
    ARMS = (
        ("free_run", "settle"),
        ("with_collector", "witness"),
    )

    def _healthy_pair(self):
        return (
            [settle_sample(1, 100)],
            [witness_sample(1, 100, at_ms=ATTACH_START_MS + 1_000)],
        )

    def _row(self, settle, witness, which):
        return settle[0] if which == "settle" else witness[0]

    def assert_refused(self, summary, needle, arm, also=None):
        """Named as a problem AND absent from the arm it would have fed.

        `also` is a second substring that must appear in the SAME problem line.
        Without it a loose one-word needle can be satisfied by an unrelated
        problem the run would have printed anyway -- the missing-`phase` case
        was green against a generic "capture-phase" message rather than against
        the gate's own refusal (Codex round 4 finding 3).
        """
        matches = [
            problem
            for problem in summary["problems"]
            if needle in problem and (also is None or also in problem)
        ]
        self.assertTrue(
            matches,
            "no problem mentioned {!r}{}: {}".format(
                needle,
                "" if also is None else " together with {!r}".format(also),
                summary["problems"],
            ),
        )
        self.assertIsNone(
            summary[arm]["hours_per_second"],
            "a refused row still produced a {} rate".format(arm),
        )
        self.assertEqual(summary[arm]["samples"], 0, summary[arm])

    def test_the_healthy_pair_is_admitted_and_produces_a_rate(self):
        """The POSITIVE CONTROL, and the reason every other test here means
        something.

        Kills: a gate so strict that nothing a live harness writes gets
        through. Each negative test below would still pass against a gate that
        refused everything; this is the one that says the fixtures -- shaped
        like what the harness emits -- are admitted and DO produce rates.
        """
        settle, witness = self._healthy_pair()
        summary = self._run(settle, witness)
        self.assertEqual(summary["problems"], [])
        self.assertIsNotNone(summary["free_run"]["hours_per_second"])
        self.assertIsNotNone(summary["with_collector"]["hours_per_second"])
        self.assertEqual(summary["with_collector"]["samples"], 1)

    def test_a_retired_witness_schema_is_refused_and_feeds_nothing(self):
        """Kills: deleting the witness arm of the gate, or filtering on
        anything other than `admission_problem`.

        A v2 row carries no host state at all.
        """
        settle, witness = self._healthy_pair()
        witness[0]["schema"] = "hoi4-passive-client-state-witness-v2"
        summary = self._run(settle, witness)
        self.assert_refused(summary, "witness-v2", "with_collector")

    def test_a_retired_settle_schema_is_refused_and_feeds_nothing(self):
        """Kills: deleting the settle arm of the gate."""
        settle, witness = self._healthy_pair()
        settle[0]["schema"] = "hoi4-settle-control-sample-v1"
        summary = self._run(settle, witness)
        self.assert_refused(summary, "sample-v1", "free_run")

    def test_a_missing_host_field_is_refused_and_feeds_nothing(self):
        """Kills: reporting on the host fields while filtering on the schema
        string -- Codex round 2 finding 1 exactly -- and dropping either arm.

        The first fix named this row as a problem and still counted it.
        """
        for arm, which in self.ARMS:
            for field in (
                "available_parallelism",
                "target_processor_affinity_cores",
                "target_page_fault_count",
                "target_working_set_bytes",
                "host_state_unavailable",
            ):
                with self.subTest(arm=arm, field=field):
                    settle, witness = self._healthy_pair()
                    self._row(settle, witness, which).pop(field)
                    summary = self._run(settle, witness)
                    self.assert_refused(summary, field, arm, also="is missing")

    def test_a_null_host_field_with_no_named_cause_is_refused(self):
        """Kills: a presence-only check.

        `None` is a legal VALUE -- the observer writes it when a host query
        fails -- but only beside a named cause. Nulls with an empty reason are
        a host-state-free row wearing the new schema string, which is the shape
        a presence-only check waves through.
        """
        for arm, which in self.ARMS:
            for reason, needle in (
                (None, "names no cause"),
                # A blank string is refused a step earlier, and for its own
                # stated reason: the observer writes null or a named cause and
                # never whitespace.
                ("", "empty string"),
                ("   ", "empty string"),
            ):
                with self.subTest(arm=arm, reason=reason):
                    settle, witness = self._healthy_pair()
                    row = self._row(settle, witness, which)
                    row["target_working_set_bytes"] = None
                    row["host_state_unavailable"] = reason
                    summary = self._run(settle, witness)
                    self.assert_refused(summary, needle, arm)

    def test_a_null_host_field_WITH_a_named_cause_is_admitted(self):
        """Kills: refusing an honestly-unavailable row -- on EITHER arm.

        An unavailable diagnostic must not destroy the engine sample beside it
        (Codex round 2 finding 3). This is the counterpart to the test above:
        the same nulls, with the cause named, are admitted and still produce a
        rate. Parameterised over both arms because a schema-specific change
        refusing named nulls on settle rows alone would otherwise survive
        (Codex round 6 finding 3).
        """
        for arm, which in self.ARMS:
            with self.subTest(arm=arm):
                settle, witness = self._healthy_pair()
                row = self._row(settle, witness, which)
                for field in BASELINE.HOST_STATE_FIELDS_NULLABLE:
                    row[field] = None
                # BOTH queries named, joined the way the observer joins them,
                # because all three counters are null and one cause cannot
                # account for two different failed calls (Codex round 7
                # finding 3). An earlier version of this test asserted that
                # the memory cause alone was enough, which is the invalid
                # combination the gate now refuses.
                row["host_state_unavailable"] = (
                    "ProcessorAffinity: access denied; "
                    "K32GetProcessMemoryInfo: Win32 error 5"
                )
                summary = self._run(settle, witness)
                self.assertEqual(summary["problems"], [])
                self.assertIsNotNone(summary[arm]["hours_per_second"])

    def test_a_non_nullable_host_field_is_refused_even_with_a_named_cause(self):
        """Kills: putting `available_parallelism` back in the nullable loop.

        Codex round 6 finding 2: it has no failure mode --
        `[Environment]::ProcessorCount` cannot fail and `ClockSample` types it
        as a plain `i32` -- so a null there is not an unavailable measurement,
        and a cause naming a DIFFERENT query does not explain it.
        """
        for arm, which in self.ARMS:
            with self.subTest(arm=arm):
                settle, witness = self._healthy_pair()
                row = self._row(settle, witness, which)
                row["available_parallelism"] = None
                row["host_state_unavailable"] = "K32GetProcessMemoryInfo: Win32 error 5"
                summary = self._run(settle, witness)
                self.assert_refused(
                    summary, "available_parallelism", arm, also="no failure mode"
                )

    def test_a_non_integer_host_field_is_refused(self):
        """Kills: accepting whatever type happens to be there.

        `True` is included on purpose: `isinstance(True, int)` is true in
        Python, so a bare integer check admits a boolean, and `True` is not a
        core count.
        """
        for arm, which in self.ARMS:
            for value in ("16", 16.5, True, [], {}):
                with self.subTest(arm=arm, value=value):
                    settle, witness = self._healthy_pair()
                    self._row(settle, witness, which)["available_parallelism"] = value
                    summary = self._run(settle, witness)
                    self.assert_refused(summary, "not an integer", arm)

    def test_a_refused_row_cannot_anchor_the_attach_window(self):
        """Kills: filtering after `attach_window` instead of before it.

        `attach_window` reads BOTH row lists to place the collector's dwell, so
        a row refused for the numerator could still have moved the window that
        decides which rows the numerator sees.

        Built so the fallback anchor is the one under test: the settle arm is
        refused, which empties the primary anchor, and the fallback takes
        `min()` over the `attach_started` witness stamps. The refused row is
        the EARLIER of the two, so if it leaked in it would win.
        """
        refused_ms = ATTACH_START_MS - 9_000
        admitted_ms = ATTACH_START_MS - 4_000

        settle, witness = self._healthy_pair()
        settle[0]["schema"] = "hoi4-settle-control-sample-v1"
        refused = witness_sample(1, 100, phase="attach_started", at_ms=refused_ms)
        refused["target_working_set_bytes"] = None
        refused["host_state_unavailable"] = None
        witness.append(refused)
        witness.append(
            witness_sample(1, 100, phase="attach_started", at_ms=admitted_ms)
        )

        summary = self._run(settle, witness)
        window = summary["attach_window"]
        self.assertTrue(
            any("names no cause" in problem for problem in summary["problems"]),
            summary["problems"],
        )
        self.assertTrue(window["bounded"], window)
        self.assertEqual(window["start_source"], "attach_started", window)
        self.assertNotEqual(
            window["start_unix_ms"],
            refused_ms,
            "the refused row anchored the attach window",
        )
        self.assertEqual(window["start_unix_ms"], admitted_ms, window)

    def test_one_retired_file_is_one_problem_not_one_per_row(self):
        """Kills: a per-row problem list.

        A file written entirely by a retired producer would print thousands of
        identical lines and bury every other finding.
        """
        settle, _ = self._healthy_pair()
        witness = [
            witness_sample(1, 100, at_ms=ATTACH_START_MS + 1_000 + n * 100)
            for n in range(25)
        ]
        for row in witness:
            row["schema"] = "hoi4-passive-client-state-witness-v2"
        summary = self._run(settle, witness)
        schema_lines = [p for p in summary["problems"] if "witness-v2" in p]
        self.assertEqual(len(schema_lines), 1, summary["problems"])
        self.assertIn("25 row(s)", schema_lines[0])

    def test_a_numeric_string_rate_field_is_refused_and_feeds_nothing(self):
        """Kills: validating only the fields a row is refused FOR, and never
        the fields it is USED for -- Codex round 3 finding 3 exactly.

        `aggregate` coerces `hour_delta` and `observed_window_ms` with
        `int(...)`, which accepts the string "1". So a row with numeric strings
        passed the host-field gate and published a rate no typed producer could
        have emitted, from a run that exited 0.
        """
        for field in ("hour_delta", "observed_window_ms"):
            with self.subTest(field=field):
                settle, witness = self._healthy_pair()
                witness[0][field] = str(witness[0][field])
                summary = self._run(settle, witness)
                self.assert_refused(summary, field, "with_collector")

    def test_a_rate_field_of_the_wrong_type_is_refused_on_either_arm(self):
        """Kills: gating only the witness arm, or accepting a bool as an int.

        `True` is in the list on purpose: `isinstance(True, int)` is true in
        Python, and one game-hour is not `True`.
        """
        for field, value in (
            ("hour_delta", True),
            ("observed_window_ms", 100.5),
            ("phase", 7),
            ("paused_final", "no"),
        ):
            with self.subTest(field=field, value=value):
                settle, witness = self._healthy_pair()
                settle[0][field] = value
                summary = self._run(settle, witness)
                self.assert_refused(summary, field, "free_run")

    def test_a_missing_rate_field_is_refused(self):
        """Kills: type-checking a field only when it happens to be there.

        Through the whole summary, not just the predicate: this is the half
        that says a refused row also FEEDS NOTHING. The exhaustive per-key
        coverage is `test_every_key_of_the_producer_row_is_required` below;
        this is the pipeline half, on the three fields the rate is built from.
        """
        for field in ("phase", "hour_delta", "observed_window_ms"):
            with self.subTest(field=field):
                settle, witness = self._healthy_pair()
                witness[0].pop(field)
                summary = self._run(settle, witness)
                self.assert_refused(
                    summary, field, "with_collector", also="is missing"
                )

    def test_a_key_no_producer_emits_is_refused(self):
        """Kills: checking only that the expected keys are there.

        A row that carries everything AND something else is not a row either
        `[ordered]@{ ... }` literal can produce -- it is a hand edit or a
        producer this reader does not know. `ClockSample` refuses the same
        shape with `#[serde(deny_unknown_fields)]`, and a summary that read it
        anyway would publish a receipt from a file the run-time reader would
        not parse.
        """
        for arm, which in self.ARMS:
            with self.subTest(arm=arm):
                settle, witness = self._healthy_pair()
                self._row(settle, witness, which)["gpu_busy_percent"] = 40
                summary = self._run(settle, witness)
                self.assert_refused(
                    summary, "gpu_busy_percent", arm, also="no producer"
                )

    def test_one_key_swapped_for_another_is_refused(self):
        """Kills: deciding whether to difference the key sets by their SIZE.

        Codex round 8 finding 2. Every other shape case changes the key count
        -- the missing tests remove one, the unknown test adds one -- so a gate
        that compared `len(row)` with `len(spec)` first and only differenced
        the sets when they disagreed would pass all of them. A row with
        `exe_sha256` replaced by `gpu_busy_percent` has exactly the right
        number of keys and is still not a row any producer wrote.

        The committed implementation computes both set differences
        unconditionally, so this passes today; it exists so that it cannot stop
        being true.
        """
        row = dict(self._complete(BASELINE.WITNESS_SCHEMA))
        row.pop("exe_sha256")
        row["gpu_busy_percent"] = 40
        self.assertEqual(len(row), len(BASELINE.ROW_SPEC[BASELINE.WITNESS_SCHEMA]))
        problem = BASELINE.admission_problem(row, BASELINE.WITNESS_SCHEMA)
        self.assertIsNotNone(problem)
        self.assertIn("exe_sha256", problem)

    # ------------------------------------------------------------------
    # THE EXHAUSTIVE SHAPE COVERAGE.
    #
    # These three drive `admission_problem` DIRECTLY rather than through
    # `RunDir`, and deliberately: the property under test is "every key of the
    # producer's row is required, typed and non-null", which is a property of
    # the predicate, and routing 62 mutations each through a temporary
    # directory would only re-test the filtering the pipeline tests above
    # already pin. That the predicate is what `summarize` filters on is itself
    # pinned, by every `assert_refused` case in this class.

    @staticmethod
    def _complete(schema):
        """One complete row of `schema`, exactly as a producer writes it."""
        if schema == BASELINE.SETTLE_SCHEMA:
            return settle_sample(1, 100, at_ms=ATTACH_START_MS)
        return witness_sample(1, 100, at_ms=ATTACH_START_MS + 1_000)

    def test_a_complete_row_of_each_schema_is_admitted(self):
        """The POSITIVE CONTROL for the three exhaustive tests below.

        Each of them asserts that a MUTATED row is refused, and all three would
        pass against a predicate that refused everything. This is the one that
        says the unmutated fixtures -- which carry the producers' whole key
        sets -- are admitted.
        """
        for schema in (BASELINE.SETTLE_SCHEMA, BASELINE.WITNESS_SCHEMA):
            with self.subTest(schema=schema):
                self.assertIsNone(
                    BASELINE.admission_problem(self._complete(schema), schema)
                )

    def test_every_key_of_the_producer_row_is_required(self):
        """Kills: demoting ANY key of either row to optional.

        Over the WHOLE key set of both schemas, because the previous version of
        this gate carried a hand-picked list of "the fields the rate is built
        from" and each review round found another field outside it that a row
        could lose without being noticed. There is nothing left to pick: a row
        the producer wrote has all of these.
        """
        for schema in (BASELINE.SETTLE_SCHEMA, BASELINE.WITNESS_SCHEMA):
            for field in sorted(BASELINE.ROW_SPEC[schema]):
                with self.subTest(schema=schema, field=field):
                    row = dict(self._complete(schema))
                    row.pop(field)
                    problem = BASELINE.admission_problem(row, schema)
                    self.assertIsNotNone(problem, field)
                    if field == "schema":
                        # Losing the schema key is refused one step earlier, by
                        # the version check, and says so in its own words.
                        self.assertIn("no compatibility path", problem)
                    else:
                        self.assertIn(repr(field), problem)
                        self.assertIn("is missing", problem)

    def test_every_key_of_the_producer_row_is_typed(self):
        """Kills: accepting whatever type happens to be in a field.

        `aggregate` coerces with `int(...)`, which accepts the string "1", so
        an untyped field is a rate no typed producer could have emitted (Codex
        round 3 finding 3). The wrong value per kind is chosen to be the one a
        naive check would miss: `True` for an integer field, because
        `isinstance(True, int)` is true in Python, and an integer for a boolean
        field for the same reason in reverse.
        """
        wrong = {
            "str": 7,
            "str?": 7,
            "int": True,
            "int?": True,
            "bool": 1,
        }
        expected = {
            "str": "not a string",
            "str?": "not a string",
            "int": "not an integer",
            "int?": "not an integer",
            "bool": "not a boolean",
        }
        for schema in (BASELINE.SETTLE_SCHEMA, BASELINE.WITNESS_SCHEMA):
            spec = BASELINE.ROW_SPEC[schema]
            for field in sorted(spec):
                if field == "schema":
                    # Its VALUE is the version check's business, above.
                    continue
                with self.subTest(schema=schema, field=field):
                    row = dict(self._complete(schema))
                    row[field] = wrong[spec[field]]
                    problem = BASELINE.admission_problem(row, schema)
                    self.assertIsNotNone(problem, field)
                    self.assertIn(repr(field), problem)
                    self.assertIn(expected[spec[field]], problem)

    def test_only_the_target_counters_may_be_null(self):
        """Kills: skipping a present-but-null field anywhere in the row.

        Codex round 4 finding 1: `"paused_final": null` on an otherwise valid
        settle row passed admission, `paused_in_sample` read it as "not
        paused", and the row entered the free-run denominator. "Absent" and
        "present with nothing in it" are different, and only the three counters
        the observer reads OFF THE TARGET can legitimately be the second --
        those are the only queries that can fail.

        Over every key of both rows, so no field can quietly join the nullable
        set. The nullable three are checked in the other direction here too: a
        null in one of them, beside the cause that names its own query, is
        ADMITTED, which is what stops this from being satisfied by a predicate
        that simply refuses every null.
        """
        for schema in (BASELINE.SETTLE_SCHEMA, BASELINE.WITNESS_SCHEMA):
            spec = BASELINE.ROW_SPEC[schema]
            for field in sorted(spec):
                with self.subTest(schema=schema, field=field):
                    row = dict(self._complete(schema))
                    row[field] = None
                    if field in BASELINE.HOST_STATE_FIELDS_NULLABLE:
                        row["host_state_unavailable"] = (
                            BASELINE.HOST_STATE_CAUSE_PREFIX[field] + " denied"
                        )
                        self.assertIsNone(
                            BASELINE.admission_problem(row, schema), field
                        )
                        continue
                    problem = BASELINE.admission_problem(row, schema)
                    if field == "host_state_unavailable":
                        # Null is its RESTING value -- it means nothing failed
                        # -- so with every counter present this is a complete
                        # row and admitting it is correct.
                        self.assertIsNone(problem, problem)
                        continue
                    self.assertIsNotNone(problem, field)
                    if field == "schema":
                        self.assertIn("no compatibility path", problem)
                    else:
                        self.assertIn(repr(field), problem)
                        self.assertIn("may never be null", problem)

    # HAND-WRITTEN MIRRORS OF THE PRODUCTION TABLES.
    #
    # The exhaustive tests above iterate `ROW_SPEC`, which means none of them
    # can notice that table SHRINKING -- they simply stop asking. These are
    # what notices (Codex round 5 finding 3 for the first instance of this
    # failure mode, round 6 finding 3 for the second). They are also the copy a
    # reader can diff against the two `[ordered]@{ ... }` literals in
    # `Invoke-HeadlessRawCapture.ps1` by eye.
    SETTLE_SPEC = {
        "schema": "str",
        "observed_at_utc": "str",
        "observed_at_unix_ms": "int",
        "phase": "str",
        "pid": "int",
        "process_creation_filetime_utc": "int",
        "expected_speed": "int",
        "speed_final": "int",
        "paused_final": "bool",
        "pause_forced_final": "bool",
        "tick_inhibited_final": "bool",
        "hour_a": "int",
        "hour_b": "int",
        "hour_delta": "int",
        "tick_progress_bits_a": "int",
        "tick_progress_bits_b": "int",
        "observed_window_ms": "int",
        "processor_milliseconds": "int",
        "available_parallelism": "int",
        "target_processor_affinity_cores": "int?",
        "target_page_fault_count": "int?",
        "target_working_set_bytes": "int?",
        "host_state_unavailable": "str?",
        "advancing": "bool",
    }
    WITNESS_ONLY_SPEC = {
        "exe_sha256": "str",
        "observer_sha256": "str",
        "access": "str",
        "speed_initial": "int",
        "paused_initial": "bool",
        "pause_forced_initial": "bool",
        "tick_inhibited_initial": "bool",
        "paused_in_window": "bool",
        "pause_forced_in_window": "bool",
        "tick_inhibited_in_window": "bool",
        "window_started_at_utc": "str",
        "window_started_at_unix_ms": "int",
        "clock_progress_waited_ms": "int",
        "clock_progress_budget_ms": "int",
    }

    def test_row_spec_is_exactly_what_the_harness_emits(self):
        """Kills: `ROW_SPEC` drifting from the producer in EITHER direction.

        This is the failure the whole shape gate is exposed to, and the only
        one it can suffer FAIL-CLOSED on a live capture. A key the harness
        emits and `ROW_SPEC` lacks refuses every real row for carrying
        something unknown; a key `ROW_SPEC` has and the harness lost refuses
        every real row for missing it. Either way the summary publishes nothing
        from a real run and every fixture test in this file stays green,
        because the fixtures are built from `ROW_SPEC`'s own idea of a row.

        So this reads the PRODUCER: it brace-matches each `[ordered]@{ ... }`
        literal in `Invoke-HeadlessRawCapture.ps1` -- located by the `schema =`
        line inside it, so renaming the variable cannot hide it -- and compares
        the key set against `ROW_SPEC`. The hand-written mirror below is the
        other half: it catches the table shrinking, which a scan of a harness
        that shrank with it would not.
        """
        harness = self._harness()
        for schema in (BASELINE.SETTLE_SCHEMA, BASELINE.WITNESS_SCHEMA):
            with self.subTest(schema=schema):
                emitted = self._emitted_keys(harness, schema)
                self.assertEqual(
                    sorted(emitted),
                    sorted(BASELINE.ROW_SPEC[schema]),
                    "ROW_SPEC and the harness disagree about {}".format(schema),
                )

    @staticmethod
    def _harness():
        """The committed harness source, normalised to LF."""
        return (
            Path(BASELINE.__file__)
            .with_name("Invoke-HeadlessRawCapture.ps1")
            .read_text(encoding="utf-8-sig")
            .replace("\r\n", "\n")
        )

    @classmethod
    def _emitted_keys(cls, harness, schema):
        """Just the keys of the literal that emits `schema`."""
        return list(cls._emitted_pairs(harness, schema))

    @staticmethod
    def _emitted_pairs(harness, schema):
        """`{key: value expression}` for the literal that emits `schema`.

        Located from the inside out: find the `schema = '<name>'` assignment,
        walk back to the `[ordered]@{` that opens the literal it sits in, then
        brace-match forward. Nothing here depends on a variable name, a line
        number, or the order of the keys.
        """
        marker = "= '{}'".format(schema)
        at = harness.index(marker)
        open_at = harness.rindex("[ordered]@{", 0, at)
        cursor = harness.index("{", open_at)
        depth = 0
        end = None
        for index in range(cursor, len(harness)):
            if harness[index] == "{":
                depth += 1
            elif harness[index] == "}":
                depth -= 1
                if depth == 0:
                    end = index
                    break
        assert end is not None, schema
        pairs = {}
        for line in harness[cursor + 1:end].split("\n"):
            stripped = line.strip()
            if not stripped or stripped.startswith("#"):
                continue
            name, sep, value = stripped.partition("=")
            assert sep, "unparsed line in the {} literal: {!r}".format(schema, line)
            name = name.strip()
            assert name not in pairs, "duplicate key {!r} in {}".format(name, schema)
            pairs[name] = value.strip()
        return pairs

    # WHAT EACH KEY'S VALUE EXPRESSION IS ALLOWED TO LOOK LIKE.
    #
    # Codex round 8 finding 1: pinning the key set says nothing about the
    # value. `available_parallelism = [string]$state.AvailableParallelism`
    # keeps every key in place and makes every live row fail both `ROW_SPEC`
    # and `ClockSample`; casting one of the four uncast fields silently turns a
    # failed query's `$null` into `0` or `""`, which is the exact
    # measured-vs-unavailable confusion those fields exist to prevent.
    #
    # So the rule is per KIND, and it is a rule about the cast:
    #
    #   int   -> [int] or [int64]      str  -> [string] or a quoted literal
    #   bool  -> [bool]                *?   -> pinned exactly, see below
    #
    # `[int]$null` is `0` and `[string]$null` is `""`, so a cast on a nullable
    # field does not merely change its type -- it destroys the distinction
    # between "measured zero" and "could not measure".
    CAST_FOR_KIND = {
        "int": ("[int]", "[int64]"),
        "str": ("[string]",),
        "bool": ("[bool]",),
    }

    # THE NULLABLE FOUR, PINNED EXACTLY.
    #
    # "No leading cast" is not the property that matters (Codex round 9 finding
    # 2). `target_page_fault_count = 0` carries no cast and destroys the same
    # distinction a cast would; so does any computed expression that cannot
    # produce `$null`. What these fields must be is a bare read of the property
    # the observer set, which is a small closed set, so it is written out.
    NULLABLE_EXPRESSION = {
        "hoi4-settle-control-sample-v2": {
            "target_processor_affinity_cores": "$sample.TargetProcessorAffinityCores",
            "target_page_fault_count": "$sample.TargetPageFaultCount",
            "target_working_set_bytes": "$sample.TargetWorkingSetBytes",
            "host_state_unavailable": "$sample.HostStateUnavailable",
        },
        "hoi4-passive-client-state-witness-v3": {
            "target_processor_affinity_cores": "$state.TargetProcessorAffinityCores",
            "target_page_fault_count": "$state.TargetPageFaultCount",
            "target_working_set_bytes": "$state.TargetWorkingSetBytes",
            "host_state_unavailable": "$state.HostStateUnavailable",
        },
    }

    # THE KEYS WRITTEN AS A BARE VARIABLE, and how that variable gets its type.
    #
    # Two modes, because two kinds of variable:
    #
    #  * "param" -- a typed parameter, assigned once by the call. The pinned
    #    string is the declaration, and it must still be in the file.
    #  * "assignments" -- an ordinary variable. EVERY assignment to it must
    #    start with one of the pinned prefixes, not merely one of them
    #    (Codex round 9 finding 1: `$gamePid` is assigned on the attach path
    #    and on the launch path, and pinning one `[int]` left the other free to
    #    become `[string]` with the settle row serialising differently for
    #    exactly the captures the passive path produces).
    #  * "sole" -- an ordinary variable whose type is not a cast but the shape
    #    of the expression, so the expression itself is pinned AND it must be
    #    the only assignment. `$readOnlyObserverSha256` is a string because it
    #    ends in `.ToLowerInvariant()`, which no prefix can express.
    UNCAST_BY_DECLARATION = {
        ("hoi4-settle-control-sample-v2", "pid"): (
            "$gamePid",
            "assignments",
            ("[int]", "0"),
        ),
        ("hoi4-settle-control-sample-v2", "process_creation_filetime_utc"): (
            "$gameCreationFiletimeUtc",
            "assignments",
            ("[uint64](",),
        ),
        ("hoi4-settle-control-sample-v2", "expected_speed"): (
            "$Speed",
            "param",
            "[int]$Speed = 5,",
        ),
        ("hoi4-passive-client-state-witness-v3", "phase"): (
            "$Phase",
            "param",
            "[Parameter(Mandatory = $true)][string]$Phase,",
        ),
        ("hoi4-passive-client-state-witness-v3", "pid"): (
            "$TargetProcessId",
            "param",
            "[Parameter(Mandatory = $true)][int]$TargetProcessId,",
        ),
        ("hoi4-passive-client-state-witness-v3", "process_creation_filetime_utc"): (
            "$ExpectedCreationFiletimeUtc",
            "param",
            "[Parameter(Mandatory = $true)][uint64]$ExpectedCreationFiletimeUtc,",
        ),
        ("hoi4-passive-client-state-witness-v3", "expected_speed"): (
            "$ExpectedSpeed",
            "param",
            "[Parameter(Mandatory = $true)][int]$ExpectedSpeed",
        ),
        ("hoi4-passive-client-state-witness-v3", "observer_sha256"): (
            "$readOnlyObserverSha256",
            "sole",
            "$readOnlyObserverSha256 = (\n"
            "    Get-FileHash -LiteralPath $readOnlyObserver -Algorithm SHA256\n"
            ").Hash.ToLowerInvariant()",
        ),
    }

    @classmethod
    def _value_problems(cls, harness):
        """Every way the harness's value expressions disagree with `ROW_SPEC`.

        A LIST, not a set of assertions, so the same rule can be applied to a
        MUTATED copy of the harness and asserted to fire. A positive control
        that only re-runs the parser proves nothing about the rule (Codex round
        9 finding 3).
        """
        problems = []
        for schema in (BASELINE.SETTLE_SCHEMA, BASELINE.WITNESS_SCHEMA):
            spec = BASELINE.ROW_SPEC[schema]
            for field, value in cls._emitted_pairs(harness, schema).items():
                kind = spec.get(field)
                if kind is None:
                    problems.append(
                        "{}: {!r} is emitted and ROW_SPEC does not know it".format(
                            schema, field
                        )
                    )
                    continue
                if kind.endswith("?"):
                    expected = cls.NULLABLE_EXPRESSION[schema][field]
                    if value != expected:
                        problems.append(
                            "{}: {!r} may be null and must be the bare read "
                            "{!r}; a cast makes $null into 0 or \"\", and a "
                            "computed value cannot be null at all. Found "
                            "{!r}".format(schema, field, expected, value)
                        )
                    continue
                declared = cls.UNCAST_BY_DECLARATION.get((schema, field))
                if declared is not None:
                    expression, mode, detail = declared
                    if value != expression:
                        problems.append(
                            "{}: {!r} is allowlisted as the bare variable {} "
                            "and is now {!r}".format(schema, field, expression, value)
                        )
                        continue
                    if mode in ("param", "sole"):
                        if detail not in harness:
                            problems.append(
                                "{}: {!r} is emitted as {} and its typed "
                                "declaration {!r} is gone".format(
                                    schema, field, expression, detail
                                )
                            )
                        if mode == "sole":
                            assignments = cls._assignments_to(harness, expression)
                            if len(assignments) != 1:
                                problems.append(
                                    "{}: {!r} is emitted as {}, whose type comes "
                                    "from the shape of one pinned expression, and "
                                    "it now has {} assignments: {}".format(
                                        schema,
                                        field,
                                        expression,
                                        len(assignments),
                                        assignments,
                                    )
                                )
                        continue
                    for assignment in cls._assignments_to(harness, expression):
                        if not assignment.startswith(detail):
                            problems.append(
                                "{}: {!r} is emitted as {}, and one of that "
                                "variable's assignments is {!r}, which is not "
                                "one of {}".format(
                                    schema, field, expression, assignment, detail
                                )
                            )
                    continue
                if value.startswith("'"):
                    if kind != "str" or not value.endswith("'"):
                        problems.append(
                            "{}: {!r} is read as {} and written as the literal "
                            "{!r}".format(schema, field, kind, value)
                        )
                    continue
                if not value.startswith(cls.CAST_FOR_KIND[kind]):
                    problems.append(
                        "{}: {!r} is read as {} and the harness writes it as "
                        "{!r}; expected one of {}".format(
                            schema, field, kind, value, cls.CAST_FOR_KIND[kind]
                        )
                    )
        return problems

    @staticmethod
    def _assignments_to(harness, variable):
        """The right-hand side of every `<variable> = ...` in the harness.

        Deliberately every one. Finding one well-typed assignment says nothing
        about the others, and it is the OTHER branch that reaches the row on
        the path this measurement actually runs.
        """
        found = []
        for line in harness.split("\n"):
            stripped = line.strip()
            if not stripped.startswith(variable + " "):
                continue
            name, sep, value = stripped.partition("=")
            if not sep or name.strip() != variable:
                continue
            found.append(value.strip())
        assert found, "no assignment to {} at all".format(variable)
        return found

    def test_the_harness_emits_each_key_as_the_type_row_spec_reads(self):
        """Kills: retyping a producer value without touching its key.

        The key-set scan above is blind to everything right of the `=`, so a
        cast change leaves it green and breaks every live row (Codex round 8
        finding 1). This applies `_value_problems` -- the rule for each key's
        `ROW_SPEC` kind -- to the committed harness.

        The four nullable fields are the sharp end: they are uncast ON PURPOSE,
        and a cast on one of them would publish `0` cores or `0` resident bytes
        as if they had been measured.
        """
        self.assertEqual(self._value_problems(self._harness()), [])

    def test_the_value_rule_fires_on_every_edit_it_exists_for(self):
        """Kills: a rule so loose no realistic edit violates it -- and a
        positive control that only re-runs the parser.

        Codex round 9 finding 3: the previous version of this test asserted
        that `_emitted_pairs` SAW the edited expression, never that the rule
        REJECTED it, so it would have stayed green with the rule deleted. Each
        case below runs the real rule over a mutated copy of the harness and
        requires it to name the field.

        None of these edits changes a key, so the key-set scan is blind to all
        four.
        """
        harness = self._harness()
        cases = (
            (
                "a non-nullable field is retyped",
                "available_parallelism          = [int]$state.AvailableParallelism",
                "available_parallelism          = [string]$state.AvailableParallelism",
                "available_parallelism",
            ),
            (
                "a nullable counter is cast, so $null becomes 0",
                "target_page_fault_count        = $state.TargetPageFaultCount",
                "target_page_fault_count        = [int64]$state.TargetPageFaultCount",
                "target_page_fault_count",
            ),
            (
                "a nullable counter is given a value that cannot be null",
                "target_working_set_bytes       = $state.TargetWorkingSetBytes",
                "target_working_set_bytes       = 0",
                "target_working_set_bytes",
            ),
            (
                "one of three assignments to an allowlisted variable is retyped",
                "    $gamePid = [int]$created.ProcessId",
                "    $gamePid = [string]$created.ProcessId",
                "pid",
            ),
            (
                "a sole-assignment variable gains a second assignment",
                "$readOnlyObserverSha256 = (",
                "$readOnlyObserverSha256 = 'deadbeef'\n$readOnlyObserverSha256 = (",
                "observer_sha256",
            ),
        )
        for name, old, new, field in cases:
            with self.subTest(case=name):
                self.assertEqual(harness.count(old), 1, "stale anchor: {!r}".format(old))
                problems = self._value_problems(harness.replace(old, new, 1))
                self.assertTrue(
                    any(repr(field) in problem for problem in problems),
                    "the rule did not name {!r}: {}".format(field, problems),
                )

    def test_the_producer_scan_can_actually_fail(self):
        """Kills: a scan whose parse is so forgiving it matches anything.

        `test_row_spec_is_exactly_what_the_harness_emits` compares two sets it
        derives, and a parse that silently returned everything -- or nothing --
        would make it vacuous in the one direction that matters. This feeds the
        same parser a literal with a key ADDED and one with a key REMOVED, and
        requires it to see the difference.
        """
        harness = self._harness()
        baseline = self._emitted_keys(harness, BASELINE.WITNESS_SCHEMA)
        added = harness.replace(
            "        advancing                      = [bool]$state.Advancing\n",
            "        advancing                      = [bool]$state.Advancing\n"
            "        gpu_busy_percent               = [int]$state.GpuBusyPercent\n",
            1,
        )
        self.assertNotEqual(added, harness, "the anchor for the ADD case is stale")
        self.assertEqual(
            sorted(set(self._emitted_keys(added, BASELINE.WITNESS_SCHEMA)) - set(baseline)),
            ["gpu_busy_percent"],
        )
        removed = harness.replace(
            "        clock_progress_budget_ms       = [int64]$state.ClockProgressBudgetMs\n",
            "",
            1,
        )
        self.assertNotEqual(removed, harness, "the anchor for the REMOVE case is stale")
        self.assertEqual(
            sorted(set(baseline) - set(self._emitted_keys(removed, BASELINE.WITNESS_SCHEMA))),
            ["clock_progress_budget_ms"],
        )

    def test_every_production_field_table_is_exactly_what_the_tests_cover(self):
        """Kills: adding OR removing an entry in any production field table
        without covering it here.

        The exhaustive tests above iterate `ROW_SPEC`, so a shrinking table
        makes them all quietly stop asking. This is the only test that fails
        when one loses a member.
        """
        self.assertEqual(BASELINE.ROW_SPEC[BASELINE.SETTLE_SCHEMA], self.SETTLE_SPEC)
        self.assertEqual(
            BASELINE.ROW_SPEC[BASELINE.WITNESS_SCHEMA],
            dict(self.SETTLE_SPEC, **self.WITNESS_ONLY_SPEC),
        )
        # The settle row is a strict subset, which is why the witness spec is
        # built from it rather than written out twice.
        self.assertEqual(set(self.SETTLE_SPEC) & set(self.WITNESS_ONLY_SPEC), set())
        # Exactly two schemas, so a third cannot appear unpinned.
        self.assertEqual(
            sorted(BASELINE.ROW_SPEC),
            sorted((BASELINE.SETTLE_SCHEMA, BASELINE.WITNESS_SCHEMA)),
        )
        self.assertEqual(
            BASELINE.HOST_STATE_FIELDS_NULLABLE,
            (
                "target_processor_affinity_cores",
                "target_page_fault_count",
                "target_working_set_bytes",
            ),
        )
        # Every nullable host field has a cause prefix, and no others.
        self.assertEqual(
            tuple(sorted(BASELINE.HOST_STATE_CAUSE_PREFIX)),
            tuple(sorted(BASELINE.HOST_STATE_FIELDS_NULLABLE)),
        )
        # And the two tables agree on WHICH fields those are: `ROW_SPEC` says a
        # field is nullable with its kind, `HOST_STATE_FIELDS_NULLABLE` says so
        # by membership, and the relationship checks read the second while the
        # shape check reads the first.
        for schema, spec in BASELINE.ROW_SPEC.items():
            nullable = tuple(
                field for field in spec if spec[field] == "int?"
            )
            self.assertEqual(
                sorted(nullable),
                sorted(BASELINE.HOST_STATE_FIELDS_NULLABLE),
                schema,
            )

    def test_a_witness_only_pause_field_is_not_required_of_the_settle_row(self):
        """Kills: one required list for both schemas.

        The settle-control row has no `*_in_window` pause fields at all, so a
        single required list would refuse every settle row the harness writes.
        """
        settle, witness = self._healthy_pair()
        for field in ("paused_in_window", "pause_forced_in_window"):
            self.assertNotIn(field, settle[0])
            self.assertIn(field, witness[0])
        summary = self._run(settle, witness)
        self.assertEqual(summary["problems"], [])
        self.assertIsNotNone(summary["free_run"]["hours_per_second"])

    def test_a_negative_term_of_the_rate_is_refused(self):
        """Kills: dropping either lower bound on the rate's own terms.

        Codex round 6 finding 1: `aggregate` sums both signed and divides one
        by the other, so a negative CANCELS a real sample rather than nudging
        it. Two rows of (hour_delta, observed_window_ms) = (1, 100) and
        (1, -50) published 2 hours over 50 ms -- 40 h/s -- with nothing named.
        """
        for arm, which in self.ARMS:
            with self.subTest(arm=arm, field="observed_window_ms"):
                settle, witness = self._healthy_pair()
                self._row(settle, witness, which)["observed_window_ms"] = -50
                summary = self._run(settle, witness)
                self.assert_refused(
                    summary, "observed_window_ms", arm, also="cannot run backwards"
                )
            with self.subTest(arm=arm, field="hour_delta"):
                settle, witness = self._healthy_pair()
                row = self._row(settle, witness, which)
                # Kept self-consistent, so this is refused for being a
                # rollback and not for failing the identity check.
                row["hour_delta"] = -1
                row["hour_b"] = row["hour_a"] - 1
                summary = self._run(settle, witness)
                self.assert_refused(summary, "hour_delta", arm, also="rollback")

    def test_positive_hours_over_a_zero_window_are_refused(self):
        """Kills: bounding the terms at zero and stopping there.

        Codex round 7 finding 1: `aggregate` sums hours and milliseconds
        INDEPENDENTLY, so rows of (1, 0) and (1, 100) publish two hours over
        one hundred milliseconds -- 20 h/s -- from a sample that observed game
        time in no wall time at all.
        """
        for arm, which in self.ARMS:
            with self.subTest(arm=arm):
                settle, witness = self._healthy_pair()
                self._row(settle, witness, which)["observed_window_ms"] = 0
                summary = self._run(settle, witness)
                self.assert_refused(
                    summary, "zero-millisecond window", arm, also="game hour"
                )

    def test_a_cause_for_a_different_query_does_not_excuse_a_null(self):
        """Kills: accepting any non-blank reason as cover for any null.

        Codex round 7 finding 3. The observer names the query that failed --
        `ProcessorAffinity:` or `K32GetProcessMemoryInfo:` -- so a row whose
        affinity is missing and whose only cause is the memory one is a row
        that does not account for its own gap. TG-3 would then have called
        missing affinity evidence "accounted for", which defeats the confound
        witness this lane exists to build.
        """
        for arm, which in self.ARMS:
            for field, wrong in (
                ("target_processor_affinity_cores", "K32GetProcessMemoryInfo: Win32 error 5"),
                ("target_page_fault_count", "ProcessorAffinity: access denied"),
                ("target_working_set_bytes", "ProcessorAffinity: access denied"),
            ):
                with self.subTest(arm=arm, field=field):
                    settle, witness = self._healthy_pair()
                    row = self._row(settle, witness, which)
                    row[field] = None
                    row["host_state_unavailable"] = wrong
                    summary = self._run(settle, witness)
                    self.assert_refused(summary, field, arm, also="does not mention")

    def test_the_matching_cause_does_excuse_its_own_null(self):
        """Kills: a correspondence so strict nothing the observer writes fits.

        The counterpart to the test above, and the reason it is not simply
        "refuse nulls": each field's own prefix admits it.
        """
        for arm, which in self.ARMS:
            for field, right in (
                ("target_processor_affinity_cores", "ProcessorAffinity: access denied"),
                ("target_page_fault_count", "K32GetProcessMemoryInfo: Win32 error 5"),
                ("target_working_set_bytes", "K32GetProcessMemoryInfo: Win32 error 5"),
            ):
                with self.subTest(arm=arm, field=field):
                    settle, witness = self._healthy_pair()
                    row = self._row(settle, witness, which)
                    row[field] = None
                    row["host_state_unavailable"] = right
                    summary = self._run(settle, witness)
                    self.assertEqual(summary["problems"], [])
                    self.assertIsNotNone(summary[arm]["hours_per_second"])

    def test_a_cause_with_nothing_null_is_refused(self):
        """Kills: validating the cause only when something is null.

        The observer writes a cause only for a query that failed, so a row
        carrying both a full set of counters and a cause is not one it wrote.
        """
        for arm, which in self.ARMS:
            with self.subTest(arm=arm):
                settle, witness = self._healthy_pair()
                self._row(settle, witness, which)["host_state_unavailable"] = (
                    "K32GetProcessMemoryInfo: Win32 error 5"
                )
                summary = self._run(settle, witness)
                self.assert_refused(
                    summary, "every host counter has a value", arm
                )

    def test_a_non_string_cause_is_refused(self):
        """Kills: asking only whether the cause is a non-blank string.

        On a row whose counters are all present the old check never looked at
        the value at all, so a number or a list rode through.
        """
        for arm, which in self.ARMS:
            for value in (7, [], {}, True):
                with self.subTest(arm=arm, value=value):
                    settle, witness = self._healthy_pair()
                    self._row(settle, witness, which)["host_state_unavailable"] = value
                    summary = self._run(settle, witness)
                    self.assert_refused(
                        summary, "host_state_unavailable", arm, also="not a string"
                    )

    def test_a_zero_hour_delta_is_still_admitted(self):
        """Kills: refusing a stalled sample along with the negative ones.

        A window in which the clock did not move is an ordinary observation and
        the denominator still counts its wall time. Only NEGATIVE is refused.
        """
        settle, _ = self._healthy_pair()
        witness = [
            witness_sample(0, 100, at_ms=ATTACH_START_MS + 1_000),
            witness_sample(1, 100, at_ms=ATTACH_START_MS + 2_000),
        ]
        summary = self._run(settle, witness)
        self.assertEqual(summary["problems"], [])
        # Both admitted: the stalled one contributes 0 hours and its 100 ms of
        # wall time, which is what a stall costs the rate.
        self.assertEqual(summary["with_collector"]["samples_counted"], 2)
        self.assertEqual(summary["with_collector"]["game_hours"], 1)
        self.assertEqual(summary["with_collector"]["observed_window_ms"], 200)

    def test_the_hour_endpoints_must_agree_with_the_delta(self):
        """Kills: dropping the `hour_b - hour_a == hour_delta` identity.

        Codex round 5 finding 1: `aggregate` sums `hour_delta` and never looks
        at the endpoints it is supposed to be the difference of, so a row
        claiming `hour_a == hour_b` with `hour_delta = 8` published eight
        game-hours that did not happen. Both producers emit all three from one
        observation, so a row where they disagree is not a row either wrote.
        """
        for arm, which in self.ARMS:
            with self.subTest(arm=arm):
                settle, witness = self._healthy_pair()
                row = self._row(settle, witness, which)
                row["hour_b"] = row["hour_a"]
                summary = self._run(settle, witness)
                self.assert_refused(summary, "hour_delta", arm, also="did not happen")

    def test_an_absent_optional_rate_field_is_still_admitted(self):
        """Kills: promoting the if-present list to required.

        The settle-control row has no `*_in_window` pause fields at all, and
        the span estimator is documented to tolerate a row without its inputs.
        A gate that demanded them would refuse rows the harness really writes.
        """
        settle, witness = self._healthy_pair()
        for field in ("paused_in_window", "pause_forced_in_window"):
            self.assertNotIn(field, settle[0])
        summary = self._run(settle, witness)
        self.assertEqual(summary["problems"], [])
        self.assertIsNotNone(summary["free_run"]["hours_per_second"])

    def test_two_different_faults_in_one_file_are_two_problems(self):
        """Kills: collapsing per FILE instead of per CAUSE.

        Counting by file would report one of these and hide the other.
        """
        settle, _ = self._healthy_pair()
        witness = [
            witness_sample(1, 100, at_ms=ATTACH_START_MS + 1_000),
            witness_sample(1, 100, at_ms=ATTACH_START_MS + 2_000),
        ]
        witness[0]["schema"] = "hoi4-passive-client-state-witness-v2"
        witness[1].pop("target_page_fault_count")
        summary = self._run(settle, witness)
        self.assertEqual(
            len([p for p in summary["problems"] if WITNESS_FILE_NAME in p]),
            2,
            summary["problems"],
        )


if __name__ == "__main__":
    unittest.main()
