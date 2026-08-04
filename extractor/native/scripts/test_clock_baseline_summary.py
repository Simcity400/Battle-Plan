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


def witness_sample(hour_delta, window_ms, phase="capture", paused=False, pause_forced=False):
    return {
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


class RunDir:
    """A throwaway run directory holding the two JSONL files, BOM and all."""

    def __init__(self, settle=(), witness=()):
        self._tmp = tempfile.TemporaryDirectory()
        self.path = Path(self._tmp.name)
        self._write("settle_control.jsonl", settle)
        self._write("passive_speed5_witness.jsonl", witness)

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
