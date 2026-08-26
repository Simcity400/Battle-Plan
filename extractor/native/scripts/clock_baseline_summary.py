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

# THE EXACT SCHEMAS THIS SUMMARY WILL READ, and nothing else.
#
# This reader used to accept any object with the field names it happened to
# want, which made it a COMPATIBILITY PATH for retired row versions nobody
# meant to keep. A v2 witness row carries no host state at all, and a summary
# built from one exits 0 and publishes a confident receipt -- which is exactly
# the blind spot the v3 bump exists to close (Codex round 1 finding 1 on lane
# TJ). Refusing by name is what makes the no-backwards-compat rule true here
# rather than merely stated in the schema string.
SETTLE_SCHEMA = "hoi4-settle-control-sample-v2"
WITNESS_SCHEMA = "hoi4-passive-client-state-witness-v3"

# NULLABLE, and only beside a named cause. These are the three the observer
# reads off the target, each of which can fail.
HOST_STATE_FIELDS_NULLABLE = (
    "target_processor_affinity_cores",
    "target_page_fault_count",
    "target_working_set_bytes",
)

# Names the cause when a host field is null. See `admission_problem`.
HOST_STATE_UNAVAILABLE_FIELD = "host_state_unavailable"

# WHICH CAUSE EXPLAINS WHICH FIELD.
#
# The observer contains each host query separately and prefixes the reason it
# appends with the query's own name, so the reason is not one opaque string --
# it says which call failed. An earlier revision of this gate accepted ANY
# non-blank reason as cover for ANY null, so a row whose affinity was missing
# and whose only reason was "K32GetProcessMemoryInfo: Win32 error 5" was
# admitted, and TG-3 would then have called missing affinity evidence
# "accounted for" (Codex round 7 finding 3). That defeats the point of the
# field: the confound witness is only useful if a gap in it is named.
HOST_STATE_CAUSE_PREFIX = {
    "target_processor_affinity_cores": "ProcessorAffinity:",
    "target_page_fault_count": "K32GetProcessMemoryInfo:",
    "target_working_set_bytes": "K32GetProcessMemoryInfo:",
}

READY_RECEIPT_FILE = "CAPTURE_READY_V2.json"
PASSIVE_INTERVALS_FILE = "PASSIVE_CLOCK_INTERVALS_V2.json"

# release, so both would quietly dilute the numerator toward the baseline;
WITH_COLLECTOR_PHASES = ("capture",)

ATTACH_ANCHOR_SOURCE = "settle_no_collector"
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



# ============================================================================
# THE COMPLETE SHAPE OF EACH ROW, and why this is a table rather than a list of
# checks.
#
# The admission predicate below started as "is the schema string right" and
# grew a check every review round: the host fields, then their types, then the
# fields the rate is built from, then the pause fields, then nulls, then the
# hour identity, then the domain of its terms, then the process identity, then
# the cause-to-field correspondence. Every one of those was a real defect, and
# every one was found the same way -- by naming a row shape the current list
# did not happen to cover. A validator built from a list of interesting fields
# invites that indefinitely, because the space of invalid rows is not bounded
# by the space of checks anyone thought to write.
#
# So the list is gone and this is the whole row. Each producer emits a FIXED
# key set from one `[ordered]@{ ... }` literal, and a row that is not exactly
# that key set, with exactly those types, is not a row the producer wrote --
# whatever else is true of it. Unknown keys are refused for the same reason
# `ClockSample` is `#[serde(deny_unknown_fields)]`: a key nobody expected is
# either a producer this reader does not know or a hand edit, and neither is
# admissible.
#
# `int|None` marks the four fields that may be null, and only beside a cause
# that names them -- see `HOST_STATE_CAUSE_PREFIX`.
#
# Keeping this in step with the producers is not left to memory: the
# source-scan guards in `lanes/passive_clock_witness/src/script_order_tests.rs`
# pin the emitted key sets in the harness, and
# `test_every_production_field_table_is_exactly_what_the_tests_cover` pins
# these tables against a hand-written copy so neither can drift alone.
_STR = "str"
_INT = "int"
_BOOL = "bool"
_INT_OR_NULL = "int?"
_STR_OR_NULL = "str?"

_COMMON_ROW_SPEC = {
    "schema": _STR,
    "observed_at_utc": _STR,
    "observed_at_unix_ms": _INT,
    "phase": _STR,
    "pid": _INT,
    "process_creation_filetime_utc": _INT,
    "expected_speed": _INT,
    "speed_final": _INT,
    "paused_final": _BOOL,
    "pause_forced_final": _BOOL,
    "tick_inhibited_final": _BOOL,
    "hour_a": _INT,
    "hour_b": _INT,
    "hour_delta": _INT,
    "tick_progress_bits_a": _INT,
    "tick_progress_bits_b": _INT,
    "observed_window_ms": _INT,
    "processor_milliseconds": _INT,
    "available_parallelism": _INT,
    "target_processor_affinity_cores": _INT_OR_NULL,
    "target_page_fault_count": _INT_OR_NULL,
    "target_working_set_bytes": _INT_OR_NULL,
    "host_state_unavailable": _STR_OR_NULL,
    "advancing": _BOOL,
}

ROW_SPEC = {
    SETTLE_SCHEMA: dict(_COMMON_ROW_SPEC),
    WITNESS_SCHEMA: dict(
        _COMMON_ROW_SPEC,
        **{
            "exe_sha256": _STR,
            "observer_sha256": _STR,
            "access": _STR,
            "speed_initial": _INT,
            "paused_initial": _BOOL,
            "pause_forced_initial": _BOOL,
            "tick_inhibited_initial": _BOOL,
            "paused_in_window": _BOOL,
            "pause_forced_in_window": _BOOL,
            "tick_inhibited_in_window": _BOOL,
            "window_started_at_utc": _STR,
            "window_started_at_unix_ms": _INT,
            "clock_progress_waited_ms": _INT,
            "clock_progress_budget_ms": _INT,
        }
    ),
}


def _spec_problem(row, spec):
    """The row's shape against `spec`, or None when it matches exactly."""
    missing = sorted(set(spec) - set(row))
    if missing:
        return "is missing {}".format(", ".join(repr(k) for k in missing))
    unknown = sorted(set(row) - set(spec))
    if unknown:
        return "carries {} that no producer of this schema emits".format(
            ", ".join(repr(k) for k in unknown)
        )
    for field in sorted(spec):
        kind = spec[field]
        value = row[field]
        if value is None:
            if kind.endswith("?"):
                continue
            # Only the three counters the observer READS OFF THE TARGET may be
            # null, and only beside a named cause. Everything else either comes
            # from the observer's own environment or is a plain engine read, so
            # a null there is not an unavailable measurement -- it is a field
            # the producer wrote with nothing in it, which nothing in either
            # `[ordered]@{ ... }` literal can do. `available_parallelism` is the
            # one that keeps being mistaken for a host counter: it comes from
            # `[Environment]::ProcessorCount`, which has no failure mode, and
            # `ClockSample` types it as a plain `i32` (Codex round 6 finding 2).
            return (
                "has a null {!r}, which has no failure mode and may never be "
                "null".format(field)
            )
        if kind in (_INT, _INT_OR_NULL):
            if isinstance(value, bool) or not isinstance(value, int):
                return "has {!r} = {!r}, which is not an integer".format(field, value)
        elif kind in (_STR, _STR_OR_NULL):
            if not isinstance(value, str):
                return "has {!r} = {!r}, which is not a string".format(field, value)
        elif kind == _BOOL:
            if not isinstance(value, bool):
                return "has {!r} = {!r}, which is not a boolean".format(field, value)
    return None


def admission_problem(row, expected_schema):
    """Why this summary will not read `row`, or None when it will.

    THE ONE PREDICATE. Reporting and filtering both call this, and that is the
    whole point: an earlier revision of this lane reported on the fields and
    filtered on the schema string alone, so a current-schema row missing a host
    field was named as a problem AND still reached the numerator, the
    denominator and the attach-window anchor (Codex round 2 finding 1). Two
    spellings of "acceptable" is one spelling too many.

    Three things are checked, in the order a reader would ask them.

    1. THE SCHEMA STRING. One row version, no compatibility path.
    2. THE WHOLE ROW SHAPE, against `ROW_SPEC`: exactly the producer's key set,
       no more and no less, with each value's type and nullability. The string
       is not evidence on its own -- a hand-edited or half-migrated file can
       carry the new name and none of the evidence the name promises -- and
       checking only the fields a row is refused FOR, never the fields it is
       USED for, was the gap that let numeric strings through `int(...)` and
       published a rate no typed producer could emit (Codex round 3 finding 3).
       See the comment above `ROW_SPEC` for why this is one table rather than a
       list of interesting fields.
    3. THE RELATIONSHIPS BETWEEN FIELDS, which no per-field spec can express:
       a null host counter must be matched by a cause that names ITS query, a
       named cause must be matched by a null, the identity
       `hour_b - hour_a == hour_delta` must hold, and both terms of the rate
       must lie in the domain a sum is meaningful over.

    WHAT THIS PREDICATE IS NOT. It is not a reimplementation of the run-time
    witness authority in `lanes/passive_clock_witness`, and it deliberately
    does not refuse every row that authority would. That authority decides
    whether a capture may CONTINUE, from a live stream; this summary decides
    what rate a FINISHED run achieved, from its files -- and the harness runs
    it "on every route out, including the routes where the capture was
    refused: a refused capture still measured a free-run baseline, and that is
    exactly the number the next attempt is planned from"
    (`Invoke-HeadlessRawCapture.ps1`, above the call, with
    `-AllowNonzeroExit`). A summary that refused what the authority refuses
    could not report on the runs it exists to report on. What the two readers
    DO agree on is: the contract this lane introduced -- a host field may be
    null only beside a named cause -- and the hour identity, which is not
    borrowed from the authority but required independently, because it is the
    arithmetic THIS file publishes.
    """
    seen = row.get("schema")
    if seen != expected_schema:
        return (
            "schema {!r} is not '{}'; this summary reads one row version and "
            "there is no compatibility path, so a receipt built from these "
            "rows would report host state the rows do not carry".format(
                seen, expected_schema
            )
        )

    # THE WHOLE ROW, in one comparison against `ROW_SPEC`. Presence, absence
    # of anything unexpected, and the type of every value.
    shape = _spec_problem(row, ROW_SPEC[expected_schema])
    if shape is not None:
        return "the row claims schema '{}' but {}".format(expected_schema, shape)

    # THEN THE THINGS A SHAPE CANNOT SAY.
    #
    # Everything below is a relationship BETWEEN fields, which no per-field
    # spec can express.
    reason = row[HOST_STATE_UNAVAILABLE_FIELD]
    if isinstance(reason, str) and reason.strip() == "":
        return (
            "the row's '{}' is an empty string; the observer writes null when "
            "nothing failed and a named cause when something did, and neither "
            "of those is blank".format(HOST_STATE_UNAVAILABLE_FIELD)
        )
    reason_is_named = isinstance(reason, str)

    for field in HOST_STATE_FIELDS_NULLABLE:
        if row[field] is not None:
            continue
        if not reason_is_named:
            return (
                "the row's '{}' is null and '{}' names no cause, so the host "
                "state is neither measured nor accounted for".format(
                    field, HOST_STATE_UNAVAILABLE_FIELD
                )
            )
        prefix = HOST_STATE_CAUSE_PREFIX[field]
        if prefix not in reason:
            return (
                "the row's '{}' is null but '{}' is {!r}, which does not "
                "mention '{}' -- the observer names the query that failed, and "
                "a cause for a different query does not account for this "
                "one".format(field, HOST_STATE_UNAVAILABLE_FIELD, reason, prefix)
            )

    if reason_is_named and all(
        row[field] is not None for field in HOST_STATE_FIELDS_NULLABLE
    ):
        return (
            "the row's '{}' is {!r} but every host counter has a value; the "
            "observer writes a cause only for a query that failed, so a row "
            "with both is not one it wrote".format(
                HOST_STATE_UNAVAILABLE_FIELD, reason
            )
        )

    # AND THE ONE ARITHMETIC IDENTITY THE RATE IS BUILT ON.
    #
    # `aggregate` sums `hour_delta` and never looks at the endpoints it is
    # supposed to be the difference of, so a row claiming `hour_a == hour_b`
    # and `hour_delta == 8` publishes eight game-hours that did not happen
    # (Codex round 5 finding 1). Both committed producers emit all three
    # together, from the same observation, so a row where they disagree is not
    # a row either producer wrote.
    #
    # THIS IS NOT THE RUN-TIME AUTHORITY MOVING INTO PYTHON, and the boundary
    # in this function's docstring still holds. It is the one invariant the
    # ARITHMETIC HERE depends on -- the number this summary publishes is
    # `hour_delta`, and this says `hour_delta` means what its name says.
    # Capture-continuation policy, the stall escalation and the rest of
    # `WitnessRun::admit` stay where they are.
    recomputed = row["hour_b"] - row["hour_a"]
    if recomputed != row["hour_delta"]:
        return (
            "the row's hour_delta is {} but hour_b - hour_a is {} ({} -> {}); "
            "this summary sums hour_delta, so a row whose endpoints do not "
            "agree with it publishes game-hours that did not happen".format(
                row["hour_delta"], recomputed, row["hour_a"], row["hour_b"]
            )
        )

    # AND THE DOMAIN THE ARITHMETIC IS DEFINED OVER.
    #
    # `aggregate` sums both of these signed and divides one by the other, so a
    # negative in either is not a small error -- it CANCELS a real sample.
    # Two admitted rows of (hour_delta, observed_window_ms) = (1, 100) and
    # (1, -50) publish 2 hours over 50 ms, or 40 h/s, with no problem named
    # (Codex round 6 finding 1).
    #
    #  * A negative `observed_window_ms` is a measured duration that ran
    #    backwards. Nothing can produce it and nothing should consume it. The
    #    run-time authority refuses it too.
    #  * A negative `hour_delta` is a CLOCK ROLLBACK, and unlike the above it
    #    IS producer-reachable: the observer emits the raw `hour_b - hour_a`.
    #    It is refused here anyway, and the two facts are not in tension. This
    #    summary's numerator is "game hours OBSERVED"; a rollback is not
    #    negative observation, and summing it silently subtracts hours another
    #    sample really did see. Refusing names it instead, which is what a
    #    reader needs -- the row is still in the file.
    if row["observed_window_ms"] < 0:
        return (
            "the row's observed_window_ms is {}; a measured duration cannot "
            "run backwards, and this summary sums it into a "
            "denominator".format(row["observed_window_ms"])
        )
    if row["observed_window_ms"] == 0 and row["hour_delta"] > 0:
        return (
            "the row reports {} game hour(s) over a zero-millisecond window; "
            "positive game time cannot be observed in no wall time, and "
            "`aggregate` sums the two independently so this row's hours land "
            "in the numerator against nothing in the "
            "denominator".format(row["hour_delta"])
        )
    if row["hour_delta"] < 0:
        return (
            "the row's hour_delta is {} ({} -> {}), a clock rollback; this "
            "summary sums hour_delta as game hours OBSERVED, where a negative "
            "silently cancels hours another sample really did see".format(
                row["hour_delta"], row["hour_a"], row["hour_b"]
            )
        )
    return None


def admitted(rows, expected_schema):
    """The rows `admission_problem` accepts, in order."""
    return [row for row in rows if admission_problem(row, expected_schema) is None]


def schema_problems(rows, filename, expected_schema):
    """Name every row this summary refuses, and why.

    PROBLEMS, NOT EXCEPTIONS. `main` already exits nonzero on a non-empty
    problems list, and a refusal that reached the operator as a traceback would
    lose the rest of the accounting -- the same reason `read_jsonl` names an
    unparseable line instead of raising.

    Reported as COUNTS PER DISTINCT CAUSE rather than one line per row: a file
    written entirely by a retired producer would otherwise print thousands of
    identical problems and bury every other finding. A file with two different
    faults still prints two lines, because the cause is the key.
    """
    causes = {}
    for row in rows:
        problem = admission_problem(row, expected_schema)
        if problem is not None:
            causes[problem] = causes.get(problem, 0) + 1
    return [
        "{} holds {} row(s) this summary will not read: {}".format(
            filename, count, cause
        )
        for cause, count in sorted(causes.items())
    ]


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
        # IDENTITY MUST BE PRESENT, not merely equal. `None == None` is true,
        # so two rows that both LACK a pid used to join and publish a span rate
        # for a process nothing identified (Codex round 7 finding 2).
        # `admission_problem` now requires both fields of both current schemas,
        # so this is the second line of defence and the one that holds when
        # this function is called directly, as its own tests do.
        if any(
            row.get(key) is None
            for row in (first, second)
            for key in ("pid", "process_creation_filetime_utc")
        ):
            dropped += 1
            continue
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
    # BEFORE AGGREGATION. A refused row must not reach a numerator or a
    # denominator even though the run will exit nonzero anyway: the receipt is
    # written whatever the problems say, and a rate computed from rows this
    # reader does not accept is a number somebody will quote.
    read_problems = (
        read_problems
        + schema_problems(settle_rows, SETTLE_FILE, SETTLE_SCHEMA)
        + schema_problems(witness_rows, WITNESS_FILE, WITNESS_SCHEMA)
    )
    # THE SAME PREDICATE THAT NAMED THEM DROPS THEM, and it runs before
    # `attach_window`, which reads both lists to place the collector's dwell.
    # A refused row must anchor nothing either.
    settle_rows = admitted(settle_rows, SETTLE_SCHEMA)
    witness_rows = admitted(witness_rows, WITNESS_SCHEMA)
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
        problems.append(
            "the with-collector rate is NOT bounded to the collector's attach "
            f"window: {window['reason']}"
        )
    if (window["bounded"] and not window["anchor_is_sound"]
            and window["samples_labelled"] > 0):
        problems.append(
            "the attach window is anchored on "
            f"{window['start_source']}, which is taken after the recorder was "
            "launched and so cannot be proved to precede its dwell clock; the "
            "window may end after the real detach and the with-collector rate "
            "may still include post-detach samples"
        )
    if pause_ms and with_collector_span["segments"] > 0:
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
        "phase=clock_baseline_attach_window "
        f"bounded={str(summary['attach_window']['bounded']).lower()} "
        f"samples_labelled={summary['attach_window']['samples_labelled']} "
        f"samples_outside={summary['attach_window']['samples_outside']} "
        f"requested_duration_ms={summary['attach_window'].get('requested_duration_ms', 'unknown')} "
        f"anchor={summary['attach_window'].get('start_source', 'none')} "
        f"anchor_is_sound={str(summary['attach_window']['anchor_is_sound']).lower()}",
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
        receipt = os.path.join(args.run_dir, RECEIPT_NAME)
        try:
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
