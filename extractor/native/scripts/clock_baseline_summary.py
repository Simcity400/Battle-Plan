#!/usr/bin/env python3
"""Print the free-run clock baseline a capture's loss must be divided by.

Loss is `with_collector / free_run`, and both terms are already recorded by
`Invoke-HeadlessRawCapture.ps1` -- and were, until now, left for an operator to
aggregate by hand:

* `settle_control.jsonl` holds the `settle_no_collector` samples. Same process,
  same save, same speed, no collector: the only hooks-free measurement of this
  exact run the harness ever takes, and therefore the denominator.
* `passive_speed5_witness.jsonl` holds the graded-window samples. The
  `capture`-phase ones are the numerator.

Both are taken by the SAME instrument (`Read-Hoi4GameState.ps1`), which brackets
a read of the game hour, waits for it to change, and reports `hour_delta` over
`observed_window_ms`. Aggregating deltas and windows separately and dividing
once is what makes this a measurement of the run rather than a mean of
per-sample rates, where a single 50 ms window would weigh as much as a 3 s one.

Rates are quoted per WALL second, so they are only comparable within one run on
one machine: a loaded machine slows the free-run term too, which is the reason
the paired control exists at all. The ratio is the portable number.

Exit codes are deliberately narrow. 0 means a summary was produced. 1 means
there was not enough evidence to produce one -- a run that never reached settle,
or one whose windows are all paused. Neither is a verdict on the capture, and
the harness treats this as reporting, not as a gate.
"""

from __future__ import annotations

import argparse
import json
import os
import sys

SCHEMA = "hoi4-clock-baseline-v1"
RECEIPT_NAME = "CLOCK_BASELINE_V1.json"

SETTLE_FILE = "settle_control.jsonl"
WITNESS_FILE = "passive_speed5_witness.jsonl"

# The graded-window phases that measure the game WHILE the collector is
# attached. `pre_attach` precedes injection and `post_teardown` follows the hook
# release, so both would quietly dilute the numerator toward the baseline;
# `ready_wait` and `ready_validated` straddle injection itself, where hooks are
# being installed and the rate is neither one thing nor the other.
WITH_COLLECTOR_PHASES = ("capture",)


def read_jsonl(path):
    """Every JSON object in a JSONL file, or [] when the file is absent.

    Absence is normal: a run that failed before settle has no control file, and
    a run that failed before attach has no witness file. Tolerating a trailing
    partial line matters too -- the harness appends these one sample at a time
    and can be killed mid-write.
    """
    if not os.path.isfile(path):
        return []
    rows = []
    # utf-8-sig: PowerShell's Add-Content -Encoding UTF8 writes a BOM on the
    # first line under Windows PowerShell 5.1.
    with open(path, encoding="utf-8-sig") as handle:
        for line in handle:
            line = line.strip()
            if not line:
                continue
            try:
                rows.append(json.loads(line))
            except ValueError:
                continue
    return rows


def paused_in_sample(row):
    """True when the engine reported a pause anywhere in the sample's window.

    A paused world is stopped on purpose, not slowed by the collector -- the game
    auto-pauses for events, decisions, focus completion, war declarations and
    peace conferences -- so counting that time as loss would price ordinary play
    as collector overhead.

    The witness schema carries `*_in_window` (did it pause at ANY point inside
    the bracket) and the settle-control schema carries only `*_final`. Take
    whichever fields a row actually has: demanding `_in_window` would silently
    admit every settle sample and exclude witness samples on a stricter rule,
    which biases the very ratio this file exists to compute.

    `tick_inhibited` is deliberately NOT an exclusion. It is set on essentially
    every sample of a healthy speed-5 run, so excluding on it empties both
    sides.
    """
    for field in (
        "paused_in_window",
        "pause_forced_in_window",
        "paused_final",
        "pause_forced_final",
    ):
        if bool(row.get(field, False)):
            return True
    return False


def aggregate(rows, label):
    """Total game hours over total observed wall time, and the rate."""
    considered = [row for row in rows if not paused_in_sample(row)]
    hours = sum(int(row.get("hour_delta", 0) or 0) for row in considered)
    window_ms = sum(int(row.get("observed_window_ms", 0) or 0) for row in considered)
    rate = None
    if window_ms > 0:
        rate = hours * 1000.0 / window_ms
    return {
        "label": label,
        "samples": len(rows),
        "samples_counted": len(considered),
        "samples_excluded_paused": len(rows) - len(considered),
        "game_hours": hours,
        "observed_window_ms": window_ms,
        "hours_per_second": rate,
    }


def summarize(run_dir):
    settle = [
        row
        for row in read_jsonl(os.path.join(run_dir, SETTLE_FILE))
        if row.get("phase") == "settle_no_collector"
    ]
    witness = [
        row
        for row in read_jsonl(os.path.join(run_dir, WITNESS_FILE))
        if row.get("phase") in WITH_COLLECTOR_PHASES
    ]
    free_run = aggregate(settle, "free_run")
    with_collector = aggregate(witness, "with_collector")

    retained = None
    loss = None
    if free_run["hours_per_second"] and with_collector["hours_per_second"] is not None:
        retained = with_collector["hours_per_second"] / free_run["hours_per_second"]
        loss = 1.0 - retained

    problems = []
    if free_run["hours_per_second"] is None:
        problems.append(
            "no free-run denominator: settle_control.jsonl has no unpaused "
            "settle_no_collector sample with an observed window"
        )
    elif free_run["game_hours"] == 0:
        problems.append(
            "the free-run control observed zero game hours, so it cannot be a rate"
        )
    if with_collector["hours_per_second"] is None:
        problems.append(
            "no with-collector numerator: the witness has no unpaused "
            "capture-phase sample with an observed window"
        )
    return {
        "schema": SCHEMA,
        "run_dir": os.path.abspath(run_dir),
        "free_run": free_run,
        "with_collector": with_collector,
        "retained_fraction": retained,
        "loss_fraction": loss,
        "problems": problems,
    }


def quote(value, digits=4):
    if value is None:
        return "unknown"
    return f"{value:.{digits}f}"


def render(summary):
    free_run = summary["free_run"]
    with_collector = summary["with_collector"]
    lines = [
        "phase=clock_baseline "
        f"free_run_hours_per_second={quote(free_run['hours_per_second'])} "
        f"with_collector_hours_per_second={quote(with_collector['hours_per_second'])} "
        f"retained_fraction={quote(summary['retained_fraction'])} "
        f"loss_fraction={quote(summary['loss_fraction'])}",
        "phase=clock_baseline_free_run "
        f"samples={free_run['samples']} counted={free_run['samples_counted']} "
        f"excluded_paused={free_run['samples_excluded_paused']} "
        f"game_hours={free_run['game_hours']} "
        f"observed_window_ms={free_run['observed_window_ms']}",
        "phase=clock_baseline_with_collector "
        f"samples={with_collector['samples']} counted={with_collector['samples_counted']} "
        f"excluded_paused={with_collector['samples_excluded_paused']} "
        f"game_hours={with_collector['game_hours']} "
        f"observed_window_ms={with_collector['observed_window_ms']}",
    ]
    for problem in summary["problems"]:
        lines.append(f"phase=clock_baseline_problem detail={problem}")
    return "\n".join(lines)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run-dir", required=True)
    parser.add_argument(
        "--no-receipt",
        action="store_true",
        help="print the summary without writing CLOCK_BASELINE_V1.json",
    )
    args = parser.parse_args(argv)

    if not os.path.isdir(args.run_dir):
        sys.stderr.write(f"phase=clock_baseline_problem detail=no run dir {args.run_dir}\n")
        return 1

    summary = summarize(args.run_dir)
    sys.stdout.write(render(summary) + "\n")

    if not args.no_receipt:
        # Best effort. The summary already went to the transcript, and a run
        # directory that cannot be written to must not turn reporting into a
        # failure.
        try:
            receipt = os.path.join(args.run_dir, RECEIPT_NAME)
            with open(receipt, "w", encoding="utf-8") as handle:
                json.dump(summary, handle, indent=2, sort_keys=True)
        except OSError as error:
            sys.stderr.write(
                f"phase=clock_baseline_problem detail=cannot write receipt: {error}\n"
            )

    if summary["problems"]:
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
