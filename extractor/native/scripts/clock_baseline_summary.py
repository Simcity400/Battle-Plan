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

TWO THINGS THIS FILE GOT WRONG, both fixed here (lane AZ, 2026-08-05):

* THE NUMERATOR WAS NOT BOUNDED TO THE ATTACH. `capture` is a phase LABEL, and
  the harness applied it for as long as the RECORDER PROCESS lived -- which is
  minutes past the collector's 60 s dwell, spent converting a multi-GB stream
  on the same machine. 59 of t26's 65 `capture` samples fall outside the dwell,
  so the published "with collector 7.17 h/s / 92 ms per game-hour" was mostly a
  measurement of the recorder's own post-capture work. See `attach_window`.

* THE RATE WAS DECIDED BY UNDER A SECOND OF OBSERVATION. Bracket-summing is
  unbiased but it only ever sees the ~50-130 ms windows, so a bounded 60 s dwell
  leaves eight of them -- about 0.7 s -- and the number swings by 3 h/s between
  neighbouring runs on the same save. `span_aggregate` differences the game
  clock and the target's own CPU counter BETWEEN samples, covering ~48 s of the
  same dwell from the same file, and puts the collector's CPU cost per
  game-hour on the receipt. Both terms are reported; neither replaces the other.

Exit codes are deliberately narrow. 0 means a summary was produced. 1 means
there was not enough evidence to produce one -- a run that never reached settle,
one whose windows are all paused, or one whose with-collector rate could not be
bounded to the attach. Neither is a verdict on the capture, and the harness
treats this as reporting, not as a gate.
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
READY_RECEIPT_FILE = "CAPTURE_READY_V2.json"
PASSIVE_INTERVALS_FILE = "PASSIVE_CLOCK_INTERVALS_V2.json"

# The graded-window phases that measure the game WHILE the collector is
# attached. `pre_attach` precedes injection and `post_teardown` follows the hook
# release, so both would quietly dilute the numerator toward the baseline;
# `ready_wait` and `ready_validated` straddle injection itself, where hooks are
# being installed and the rate is neither one thing nor the other.
WITH_COLLECTOR_PHASES = ("capture",)

# THE ONLY ANCHOR THAT CAN BE PROVED to precede the collector's dwell clock.
#
# The recorder starts its own `duration_ms` deadline when its capture worker
# begins, inside the injected DLL, BEFORE the READY/ACK exchange and before the
# harness observes anything about the attach. So no witness sample taken after
# the recorder was launched is provably earlier than that clock -- not
# `ready_validated`, not `ready_wait`, and not `attach_started`, because the
# child is already running when the harness takes that bracket and nothing stops
# the DLL from reaching its worker first (Codex gate findings, rounds 1 and 2).
# Anchoring on any of them can push the window's END past the real detach and
# re-admit exactly the post-detach samples this bound exists to remove.
#
# The settle control is on the other side of that line. Its samples are taken
# with no collector attached and before the recorder process is launched at all,
# so the LAST of them is an instant that provably precedes the dwell clock, and
# `last_settle + duration_ms` provably precedes the real detach. The window can
# then only END EARLY: its error costs attached samples and can never admit
# detached ones.
#
# It costs almost nothing. Across the live_unlock runs the gap between the last
# settle sample and the attach observation is 1.3-1.5 s of a 60 s dwell, and the
# settle-anchored window keeps 6-7 capture samples where an attach-anchored one
# keeps 5-6.
ATTACH_ANCHOR_SOURCE = "settle_no_collector"
# Used only when the settle control is absent -- a run that failed before it, or
# an older layout. None of these is sound, so a window anchored on one is
# reported as unsound and raises a problem; it is still better than no bound.
ATTACH_START_PHASE_FALLBACKS = ("attach_started", "ready_wait", "ready_validated")


def attach_window(run_dir, witness_rows, settle_rows):
    """`(start_ms, end_ms, detail)` for the window the collector was attached.

    THE PHASE LABEL IS NOT THE ATTACH WINDOW, and reading it as one is what
    made every `loss_fraction` in `runs/live_unlock` an overstatement. The
    harness's capture-wait loop runs until the RECORDER PROCESS exits, and it
    labelled every sample it took in there `capture`. The recorder outlives its
    dwell by minutes: it detaches the collector at `requested_duration_ms`
    (60 s in every live_unlock run) and then converts a multi-GB stream into a
    session and a manifest, on the same machine, with the game still running.
    In t26, 59 of 65 `capture` samples fall outside the dwell; the published
    7.17 h/s and its 92 ms/game-hour are therefore mostly a measurement of the
    HOST's post-capture conversion, not of the collector. Bounding the
    numerator by the dwell puts t26's span term at 11.17 h/s and 34.0 ms per
    game-hour, in line with every other fast run.

    The bound is derived from evidence the run already carries and nothing
    else: `requested_duration_ms` in the ready receipt is the dwell the recorder
    was asked for, and the LAST settle-control sample is an instant that
    provably precedes the recorder's own dwell clock, because the settle phase
    runs before the recorder process exists. See `ATTACH_ANCHOR_SOURCE` for why
    no witness sample can serve. The window therefore ENDS EARLY relative to the
    true detach, so its error costs attached samples and never admits detached
    ones -- and a bracket is excluded whenever ANY part of it lies outside, not
    merely its end.

    THE ONE CASE THIS BOUND CANNOT SEE is a recorder that detached EARLY: the
    dwell it was asked for is on record, the dwell it actually held is not, so
    an aborted capture's tail would still be counted. `requested_duration_ms` is
    the only dwell any already-captured run wrote down, which is why this reads
    it; it is not the reason the fix is in the harness as well. The harness's
    `recorder_finalize` label is keyed on the recorder's own teardown receipt
    and so measures the dwell that HAPPENED, which closes that case for every
    run captured from now on.

    Newer runs do not need this arithmetic at all: the harness now labels
    post-detach samples `recorder_finalize`, keyed on the recorder's own
    authenticated teardown receipt. This function is what makes the runs
    captured before that change readable, and a second line of defence if the
    labelling ever regresses.

    `detail` is always a dict and always reaches the receipt, so a window that
    could not be derived says so on the wire instead of silently reverting to
    the unbounded number.
    """
    ready_path = os.path.join(run_dir, READY_RECEIPT_FILE)
    duration_ms = None
    if os.path.isfile(ready_path):
        try:
            with open(ready_path, encoding="utf-8-sig") as handle:
                duration_ms = json.load(handle).get("requested_duration_ms")
        except (OSError, ValueError):
            duration_ms = None
    if not isinstance(duration_ms, int) or duration_ms <= 0:
        return None, None, {
            "bounded": False,
            "anchor_is_sound": False,
            "reason": (
                f"{READY_RECEIPT_FILE} carries no positive requested_duration_ms, "
                "so the collector's dwell is unknown and the with-collector rate "
                "is computed over every capture-labelled sample including any "
                "taken after the collector detached"
            ),
        }
    # THE SOUND ANCHOR FIRST: the LAST settle sample, `max`, because it is the
    # latest instant that is still provably before the recorder existed, so it
    # is the largest sound window available.
    settle_stamps = [
        row.get("observed_at_unix_ms")
        for row in settle_rows
        if row.get("phase") == ATTACH_ANCHOR_SOURCE
        and isinstance(row.get("observed_at_unix_ms"), int)
    ]
    start = None
    start_source = None
    anchor_is_sound = False
    if settle_stamps:
        start = max(settle_stamps)
        start_source = ATTACH_ANCHOR_SOURCE
        anchor_is_sound = True
    else:
        for phase in ATTACH_START_PHASE_FALLBACKS:
            # The EARLIEST sample of the earliest available phase: `min`, not
            # `max`. A later sample of the same phase sits further from the
            # recorder's clock start in the direction that admits detached
            # samples, and these anchors are already unsound in that direction.
            stamps = [
                row.get("observed_at_unix_ms")
                for row in witness_rows
                if row.get("phase") == phase
                and isinstance(row.get("observed_at_unix_ms"), int)
            ]
            if stamps:
                start = min(stamps)
                start_source = phase
                break
    if start is None:
        return None, None, {
            "bounded": False,
            "anchor_is_sound": False,
            "reason": (
                f"the run has neither a timestamped {ATTACH_ANCHOR_SOURCE} "
                "sample nor any attach-sequence witness sample "
                f"({', '.join(ATTACH_START_PHASE_FALLBACKS)}), so the start of "
                "the collector's dwell is unknown and the with-collector rate "
                "is computed over every capture-labelled sample including any "
                "taken after the collector detached"
            ),
        }
    end = start + duration_ms
    return start, end, {
        "bounded": True,
        "anchor_is_sound": anchor_is_sound,
        "start_unix_ms": start,
        "end_unix_ms": end,
        "requested_duration_ms": duration_ms,
        "start_source": start_source,
    }


def sample_inside_window(row, start, end):
    """True when the WHOLE of `row`'s bracket lies inside `[start, end]`.

    A sample is a bracket, not an instant: `window_started_at_unix_ms` opens it
    and `observed_at_unix_ms` closes it. Admitting a bracket that straddles
    detach would put the collector's cost and the host's finalization cost in
    the same number, which is the defect this bound exists to remove -- so a
    straddling bracket is excluded, in either direction.

    A row with no usable timestamps cannot be placed and is excluded. That is
    the fail-closed direction: an unplaceable sample counted IN is an
    unattributed one.
    """
    closed = row.get("observed_at_unix_ms")
    if not isinstance(closed, int):
        return False
    opened = row.get("window_started_at_unix_ms")
    if not isinstance(opened, int):
        window_ms = row.get("observed_window_ms")
        opened = closed - window_ms if isinstance(window_ms, int) else closed
    return opened >= start and closed <= end


def read_jsonl(path):
    """`(rows, problems)` for a JSONL file, or `([], [])` when it is absent.

    Absence is normal: a run that failed before settle has no control file, and
    a run that failed before attach has no witness file.

    EXACTLY ONE malformed line is tolerated: an unterminated FINAL one. That is
    the shape a kill produces -- the harness appends one sample at a time, so an
    interrupted write can only ever truncate the line it was writing, and that
    line has no newline after it. Any OTHER unparseable line is damage the
    append pattern cannot produce, and swallowing it would drop samples out of
    the numerator or the denominator and still print a confident rate. Those are
    named as problems instead, which is what makes `main` exit nonzero: a rate
    computed from a file we could not fully read is not a measurement.
    """
    if not os.path.isfile(path):
        return [], []
    rows = []
    problems = []
    # utf-8-sig: PowerShell's Add-Content -Encoding UTF8 writes a BOM on the
    # first line under Windows PowerShell 5.1.
    with open(path, encoding="utf-8-sig") as handle:
        raw_lines = handle.readlines()
    terminated = (not raw_lines) or raw_lines[-1].endswith("\n")
    last_index = len(raw_lines) - 1
    for index, raw in enumerate(raw_lines):
        line = raw.strip()
        if not line:
            continue
        try:
            rows.append(json.loads(line))
        except ValueError as error:
            if index == last_index and not terminated:
                continue
            problems.append(
                "{} line {} is not JSON ({}); this file was read incompletely, "
                "so any rate derived from it is missing samples".format(
                    os.path.basename(path), index + 1, error
                )
            )
    return rows, problems


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


def span_aggregate(rows, label):
    """Rate and CPU cost over the WHOLE window these samples span.

    WHY A SECOND ESTIMATOR. `aggregate` sums only the sample BRACKETS, and a
    bracket is 50-130 ms long. Bounded to the collector's dwell, a live run
    leaves eight of them: the with-collector rate is decided by well under one
    second of observation out of sixty, and it swings by 3 h/s between runs
    taken minutes apart on the same save. That is not enough to price a change.

    Between two consecutive samples the hour counter and the target's own
    cumulative CPU time both advance, and both are already recorded on every
    row. Differencing them covers the ENTIRE gap -- ~48 s of a 60 s dwell
    instead of ~0.7 s -- from the same file, with no extra observation and
    therefore no extra observer load on the game.

    THE PRICE, stated because it is the reason `aggregate` exists and is kept:
    any pause inside a gap would be INVISIBLE here and would be charged to the
    collector as slowdown. A segment is dropped when either endpoint reports a pause, which
    removes only the pauses the witness happened to bracket. Nothing in the run
    can close that hole -- the passive clock receipt books every between-bracket
    interval to `unobserved_ms` with no state -- so it is SIZED instead: see
    `pause_evidence`, whose two numbers reach the receipt as
    `collector_cost.observed_pause_ms` (a detection, and a problem when nonzero)
    and `collector_cost.pause_exposure_unobserved_ms` (the wall time a pause
    could have hidden in). The bracket term is the pause-safe one; this term is
    the one that covers the dwell. Neither replaces the other.

    `processor_milliseconds` is the target process's own `TotalProcessorTime`,
    and the collector is an injected DLL, so its worker threads are inside that
    number. `cpu_ms_per_game_hour` is therefore game + collector while attached
    and game alone free-run, and the DIFFERENCE between the two is what the
    collector costs in CPU per game-hour. Nothing else in this repository reads
    that field.
    """
    segments = 0
    dropped = 0
    hours = 0
    wall_ms = 0
    cpu_ms = 0
    for first, second in zip(rows, rows[1:]):
        # SAME PROCESS INCARNATION, not merely the same pid. A pid is reused,
        # and both rows carry the creation FILETIME that tells two incarnations
        # apart; comparing only the pid would fold a relaunched game's counters
        # into the same rate (Codex gate finding, round 1).
        if first.get("pid") != second.get("pid"):
            dropped += 1
            continue
        if (first.get("process_creation_filetime_utc")
                != second.get("process_creation_filetime_utc")):
            dropped += 1
            continue
        if paused_in_sample(first) or paused_in_sample(second):
            dropped += 1
            continue
        # EVERY FIELD PRESENT ON BOTH ROWS, or the segment is not a
        # measurement. Coercing a missing counter to zero manufactures numbers:
        # two absent CPU readings become an accepted "this hour cost no CPU",
        # and one absent reading followed by a real one becomes a delta the size
        # of the whole counter (Codex gate finding, round 1).
        fields = ("hour_a", "observed_at_unix_ms", "processor_milliseconds")
        if any(not isinstance(row.get(field), int)
               for row in (first, second) for field in fields):
            dropped += 1
            continue
        delta_hours = second["hour_a"] - first["hour_a"]
        delta_wall = (second["observed_at_unix_ms"]
                      - first["observed_at_unix_ms"])
        delta_cpu = (second["processor_milliseconds"]
                     - first["processor_milliseconds"])
        # A non-advancing or backwards segment is not a rate. The hour counter
        # is monotonic within a session, the wall clock is taken by one
        # observer, and CPU time is cumulative, so any of these going backwards
        # means the two rows are not a pair (a reload, a different process) and
        # counting it would corrupt both terms.
        if delta_hours <= 0 or delta_wall <= 0 or delta_cpu < 0:
            dropped += 1
            continue
        segments += 1
        hours += delta_hours
        wall_ms += delta_wall
        cpu_ms += delta_cpu
    return {
        "label": label,
        "segments": segments,
        "segments_dropped": dropped,
        "game_hours": hours,
        "wall_ms": wall_ms,
        "target_cpu_ms": cpu_ms,
        "hours_per_second": (hours * 1000.0 / wall_ms) if wall_ms > 0 else None,
        "cpu_ms_per_game_hour": (cpu_ms / hours) if hours > 0 else None,
        # Mean number of logical cores the TARGET process kept busy. Read with
        # the machine's core count beside it: the same collector is free on a
        # box the game leaves idle and is not on one it already fills.
        "target_cpu_cores": (cpu_ms / wall_ms) if wall_ms > 0 else None,
    }


def pause_evidence(run_dir):
    """`(observed_pause_ms, unobserved_ms)` from the passive clock receipt.

    THE SPAN ESTIMATOR'S BLIND SPOT, SIZED RATHER THAN CLOSED. `span_aggregate`
    drops a segment when either endpoint's bracket saw a pause, but a pause that
    begins and ends inside the multi-second gap between two brackets is
    invisible to it and would be charged to the collector as slowdown. Whether
    any such pause happened in a given run is not knowable from that run; the
    conditional is the point.

    The passive clock receipt does NOT close that hole, and it is important not
    to read it as if it did (Codex gate finding, round 2): its
    `paused_by_engine_ms` / `paused_unforced_ms` / `stalled_ms` classify only
    time the witness actually bracketed, and every gap between brackets is
    booked to `unobserved_ms` with no state at all. A pause inside a gap
    therefore leaves all three at zero.

    So the two numbers do different jobs and both are published:

    * `observed_pause_ms` is a DETECTION, and only that. Nonzero means the run
      demonstrably paused where the instrument could see it. It does NOT show
      that a gap was contaminated -- the detected pause may lie entirely inside
      brackets `span_aggregate` already dropped -- so what follows is that the
      span term for that run is not RELIABLE as collector cost, not that it is
      wrong, and never by how much. It is a problem.
    * `unobserved_ms` is an EXPOSURE. It is the wall time in which a pause could
      have hidden, and it is large by construction on every healthy run, so it
      is reported as a number and never as an alarm. It is the reason the
      bracket term is kept: that one is pause-safe and this one is not.

    `(None, None)` when the receipt is absent or malformed -- unknown, not zero.
    """
    path = os.path.join(run_dir, PASSIVE_INTERVALS_FILE)
    if not os.path.isfile(path):
        return None, None
    try:
        with open(path, encoding="utf-8-sig") as handle:
            receipt = json.load(handle)
    except (OSError, ValueError):
        return None, None
    observed = 0
    for field in ("paused_by_engine_ms", "paused_unforced_ms", "stalled_ms"):
        value = receipt.get(field)
        if not isinstance(value, int):
            return None, None
        observed += value
    unobserved = receipt.get("unobserved_ms")
    return observed, unobserved if isinstance(unobserved, int) else None


def collector_cost(free_span, with_span):
    """What the collector added, per game-hour, from the two span terms.

    `wall_ms_per_game_hour` is the number the escalation table quotes: the
    extra wall-clock the game spent on each hour while the collector was
    attached. `cpu_ms_per_game_hour` is the CPU it spent doing it, which is the
    part that does not depend on how loaded the box was, and the two together
    say whether a wall cost is work or waiting.
    """
    free_rate = free_span["hours_per_second"]
    with_rate = with_span["hours_per_second"]
    cost = {
        "wall_ms_per_game_hour": None,
        "cpu_ms_per_game_hour": None,
        "cpu_cores": None,
    }
    if free_rate and with_rate:
        cost["wall_ms_per_game_hour"] = 1000.0 / with_rate - 1000.0 / free_rate
    if (free_span["cpu_ms_per_game_hour"] is not None
            and with_span["cpu_ms_per_game_hour"] is not None):
        cost["cpu_ms_per_game_hour"] = (with_span["cpu_ms_per_game_hour"]
                                        - free_span["cpu_ms_per_game_hour"])
    if (free_span["target_cpu_cores"] is not None
            and with_span["target_cpu_cores"] is not None):
        cost["cpu_cores"] = (with_span["target_cpu_cores"]
                             - free_span["target_cpu_cores"])
    return cost


def summarize(run_dir):
    settle_rows, read_problems = read_jsonl(os.path.join(run_dir, SETTLE_FILE))
    witness_rows, witness_read_problems = read_jsonl(
        os.path.join(run_dir, WITNESS_FILE)
    )
    read_problems = read_problems + witness_read_problems
    settle = [row for row in settle_rows if row.get("phase") == "settle_no_collector"]
    labelled = [
        row for row in witness_rows if row.get("phase") in WITH_COLLECTOR_PHASES
    ]
    start, end, window = attach_window(run_dir, witness_rows, settle_rows)
    if start is None:
        witness = labelled
    else:
        witness = [row for row in labelled if sample_inside_window(row, start, end)]
    window["samples_labelled"] = len(labelled)
    window["samples_outside"] = len(labelled) - len(witness)
    free_run = aggregate(settle, "free_run")
    with_collector = aggregate(witness, "with_collector")
    free_run_span = span_aggregate(settle, "free_run_span")
    with_collector_span = span_aggregate(witness, "with_collector_span")
    cost = collector_cost(free_run_span, with_collector_span)
    pause_ms, unobserved_ms = pause_evidence(run_dir)
    cost["observed_pause_ms"] = pause_ms
    cost["pause_exposure_unobserved_ms"] = unobserved_ms

    retained = None
    loss = None
    if free_run["hours_per_second"] and with_collector["hours_per_second"] is not None:
        retained = with_collector["hours_per_second"] / free_run["hours_per_second"]
        loss = 1.0 - retained

    problems = list(read_problems)
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
            "capture-phase sample inside the collector's attach window"
        )
    if not window["bounded"] and window["samples_labelled"] > 0:
        # NOT a soft note. An unbounded numerator is the exact defect that
        # published t26 at 7.17 h/s, and a run that cannot be bounded has to
        # say so where the exit code carries it, not in a field a reader may
        # never open.
        #
        # Conditioned on there being something to bound. A run that produced no
        # capture-labelled sample at all -- an attempt refused before the
        # graded window, which is a normal outcome -- has no sample that could
        # be misattributed, and reporting an unbounded window there would fire
        # this on every refusal and teach a reader to ignore it.
        problems.append(
            "the with-collector rate is NOT bounded to the collector's attach "
            f"window: {window['reason']}"
        )
    if (window["bounded"] and not window["anchor_is_sound"]
            and window["samples_labelled"] > 0):
        # Bounded, but on an anchor that cannot be proved to precede the
        # recorder's own dwell clock -- so the window may end LATE and re-admit
        # post-detach samples. That is the original defect in miniature, and it
        # must not hide behind `bounded=true`.
        problems.append(
            "the attach window is anchored on "
            f"{window['start_source']}, which is taken after the recorder was "
            "launched and so cannot be proved to precede its dwell clock; the "
            "window may end after the real detach and the with-collector rate "
            "may still include post-detach samples"
        )
    if pause_ms and with_collector_span["segments"] > 0:
        # WHAT THIS DOES AND DOES NOT SAY. It says the run demonstrably paused,
        # which makes it a run in which pausing happens rather than one where
        # the question is hypothetical. It does NOT say a pause fell in a gap:
        # the detected pause may lie entirely inside brackets that
        # `span_aggregate` already dropped, and nothing in the run can decide
        # that either way (Codex gate finding, round 3). So the message reports
        # the demonstrated pause and the exposure it was measured against, and
        # says the span term is not RELIABLE here -- not that it is wrong, and
        # not by how much. The exposure figure alone is never a problem: it is
        # nonzero on every healthy run.
        exposure = (
            f"{unobserved_ms} ms" if isinstance(unobserved_ms, int) else "an unknown amount"
        )
        problems.append(
            f"the engine reported {pause_ms} ms of pause/stall over the graded "
            f"window, against {exposure} of wall time the witness never "
            "bracketed. A pause inside an unbracketed gap is invisible to the "
            "span estimator and would be charged to the collector, and this run "
            "cannot show whether one occurred, so its span rate and "
            "collector_cost are not reliable as collector cost; the bracket "
            "terms are the pause-safe ones for this run"
        )
    return {
        "schema": SCHEMA,
        "run_dir": os.path.abspath(run_dir),
        "free_run": free_run,
        "with_collector": with_collector,
        "free_run_span": free_run_span,
        "with_collector_span": with_collector_span,
        "collector_cost": cost,
        "attach_window": window,
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
        # The truncation is never silent: how many capture-labelled samples
        # this run took, and how many of them the attach bound threw out, on
        # the same transcript line as the rate they would otherwise have moved.
        "phase=clock_baseline_attach_window "
        f"bounded={str(summary['attach_window']['bounded']).lower()} "
        f"samples_labelled={summary['attach_window']['samples_labelled']} "
        f"samples_outside={summary['attach_window']['samples_outside']} "
        f"requested_duration_ms={summary['attach_window'].get('requested_duration_ms', 'unknown')} "
        f"anchor={summary['attach_window'].get('start_source', 'none')} "
        f"anchor_is_sound={str(summary['attach_window']['anchor_is_sound']).lower()}",
        # The span terms, which cover the whole window rather than the sample
        # brackets, and the CPU the target burned while doing it. These are the
        # two numbers a perf change is judged on.
        "phase=clock_baseline_span "
        f"free_run_hours_per_second={quote(summary['free_run_span']['hours_per_second'])} "
        f"with_collector_hours_per_second={quote(summary['with_collector_span']['hours_per_second'])} "
        f"free_run_segments={summary['free_run_span']['segments']} "
        f"with_collector_segments={summary['with_collector_span']['segments']}",
        "phase=clock_baseline_collector_cost "
        f"wall_ms_per_game_hour={quote(summary['collector_cost']['wall_ms_per_game_hour'], 1)} "
        f"cpu_ms_per_game_hour={quote(summary['collector_cost']['cpu_ms_per_game_hour'], 1)} "
        f"cpu_cores={quote(summary['collector_cost']['cpu_cores'], 2)} "
        f"free_run_cpu_ms_per_game_hour={quote(summary['free_run_span']['cpu_ms_per_game_hour'], 1)} "
        f"with_collector_cpu_ms_per_game_hour={quote(summary['with_collector_span']['cpu_ms_per_game_hour'], 1)}",
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
        receipt = os.path.join(args.run_dir, RECEIPT_NAME)
        try:
            # "x", NOT "w". A run directory is evidence: once a capture has
            # published its receipt, re-running this tool over the same run must
            # not silently replace the number that run was judged on. Truncating
            # mode made `clock_baseline_summary.py --run-dir runs/live_unlock/t22`
            # -- an ordinary read of an old run -- an overwrite of that run's
            # receipt.
            with open(receipt, "x", encoding="utf-8") as handle:
                json.dump(summary, handle, indent=2, sort_keys=True)
        except FileExistsError:
            sys.stderr.write(
                "phase=clock_baseline_problem detail=receipt already exists and "
                f"was NOT overwritten: {receipt}\n"
            )
        except OSError as error:
            sys.stderr.write(
                f"phase=clock_baseline_problem detail=cannot write receipt: {error}\n"
            )

    if summary["problems"]:
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
