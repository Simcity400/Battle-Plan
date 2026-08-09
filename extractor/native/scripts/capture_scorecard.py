#!/usr/bin/env python3
"""Score a capture against "all the data, no misses".

`report_raw_capture.py` answers a narrower question: did any domain end the run
carrying a residual failure status. A capture can pass that and still be missing
most of its data, because the verdict never looks at gaps, at domains that
produced no rows, at events a producer dropped, or at hours the sweep never
observed. It also builds its rows from `compact_stream.per_domain`, so a domain
that produced no per-domain record at all is absent from the report entirely
rather than reported as broken.

This scores every axis on which a capture can be incomplete, and is clean only
when all of them are zero. Each axis is counted from the manifest that the
recorder itself sealed, so a passing scorecard is a statement about the stream,
not about the harness that produced it.
"""

import argparse
import collections
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import gap_ledger  # noqa: E402  (path is set immediately above)

# Gap reasons that mean an hour was deliberately skipped rather than lost to a
# reader. Kept separate because they reconcile against `skipped_game_hours`.
SKIPPED_HOUR_REASONS = {"skipped_hour"}

# The v1 failure vocabulary, by bit, from
# `compact_contract/src/status_flags_base.rs`. Named here rather than imported
# because this script reads a sealed manifest and must run against a manifest
# from any checkout; a name this table does not know is printed as its bit so
# an added flag is still visible.
FAILURE_STATUS_NAMES = (
    (1 << 0, "state_unavailable"),
    (1 << 1, "profile_mismatch"),
    (1 << 2, "no_clock"),
    (1 << 3, "no_scalar_source"),
    (1 << 6, "worker_panic"),
    (1 << 7, "bridge_unavailable"),
    (1 << 8, "source_unavailable"),
    (1 << 9, "partial_data"),
    (1 << 12, "control_invalid"),
)


FAILURE_STATUS_MASK = 0
for _bit, _name in FAILURE_STATUS_NAMES:
    FAILURE_STATUS_MASK |= _bit


def residual_failure_status(info):
    """The domain's residual failure word, from whichever field carries it.

    `effective_failure_status_flags_or` is the word the promotion gate reads
    and the one to prefer: it has already had the lane-local status vocabulary
    (d49/d51/d53/d60/d99 badge bits, which are semantics, not failures)
    stripped. It was added late, so a manifest sealed before it exists carries
    only `status_flags_or` -- and on run batch3/deepwar_b that is exactly where
    d81's `768` (source_unavailable|partial_data) lives. Falling back to it
    masked to the failure vocabulary is what lets this axis score the captures
    that motivated it instead of only future ones.

    A PRESENT zero is a clean domain and must not fall back; only an absent
    field does. That is why this tests for None rather than for falsiness.
    """
    effective = info.get("effective_failure_status_flags_or")
    if effective is not None:
        return int(effective or 0)
    return int(info.get("status_flags_or", 0) or 0) & FAILURE_STATUS_MASK


def failure_status_names(flags):
    """Spell a residual failure-status word, keeping unknown bits visible."""
    flags = int(flags or 0)
    if not flags:
        return ""
    names = []
    remaining = flags
    for bit, name in FAILURE_STATUS_NAMES:
        if flags & bit:
            names.append(name)
            remaining &= ~bit
    if remaining:
        names.append(f"unknown_bits=0x{remaining:x}")
    return "|".join(names)


def load_manifest(run_dir):
    for name in ("native_session_manifest.json", "session_manifest.json"):
        path = os.path.join(run_dir, name)
        if os.path.isfile(path):
            with open(path, encoding="utf-8") as handle:
                return json.load(handle), path
    raise SystemExit(f"no session manifest in {run_dir}")


def descheduled(manifest):
    """The domains the capture deliberately did not schedule.

    Returned separately from `score` because this is not an axis: zero
    descheduled domains is not better than five. It exists so a reader can tell
    "we chose not to capture this" from "this failed to produce" — the two look
    identical once a domain is simply absent from every ledger.
    """
    stream = manifest.get("compact_stream", {}) or {}
    return stream.get("descheduled_domains", {}) or {}


def registry_descheduled(native_root=None):
    """(id, wire name) pairs the domain registry marks out of capture scope.

    An external expectation is what makes the record check more than a
    non-emptiness test: without it a manifest can drop four of five entries, or
    invent one, and still look complete. Returns None when the registry is not
    reachable — the scorecard is runnable against a run directory copied away
    from the tree, and an unavailable cross-check must not masquerade as a
    passing one.

    This is the registry in the CHECKOUT BESIDE THIS SCRIPT, not one pinned to
    the build that sealed the manifest, so this one axis asks "does this capture
    match the capture contract as it stands now" rather than "was it correct
    when it was sealed". That is the right question for the normal use — scoring
    a capture the current tree just produced — but it means an archived manifest
    can change verdict on this axis as the contract evolves. The fault messages
    say which side each disagreement came from so that is legible rather than
    mysterious.
    """
    if native_root is None:
        native_root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    path = os.path.join(native_root, "domain_registry.json")
    try:
        with open(path, encoding="utf-8") as handle:
            registry = json.load(handle)
    except (OSError, ValueError):
        return None
    rows = registry.get("domains")
    if not isinstance(rows, list) or not rows:
        return None
    return {
        (row.get("id"), row.get("name"))
        for row in rows
        if isinstance(row, dict) and row.get("capture_scheduled") is False
    }


def score(manifest):
    """Every axis on which the capture can be short of complete.

    Returns a list of (key, count, detail) where a nonzero count is a miss.
    """
    stream = manifest.get("compact_stream", {}) or {}
    # The gap ledger is RUN ENCODED in the sealed manifest -- the flat arrays
    # this scorecard used to walk cost 33.5 KB of JSON per game hour. Expanded
    # once, here, into exactly the rows the flat arrays held, so every axis below
    # is unchanged.
    #
    # A ledger this scorecard cannot read is ITS OWN AXIS, not zero gaps. Every
    # gap axis below counts rows, so an unreadable or absent ledger would
    # otherwise score a capture perfectly clean on the strength of a document
    # nobody could open -- the strongest false claim this tool can make.
    ledger_fault = ""
    try:
        gap_rows = gap_ledger.expand_gaps(stream)
        gap_outcome_rows = gap_ledger.expand_domain_gap_outcomes(stream)
    except gap_ledger.GapLedgerError as error:
        gap_rows, gap_outcome_rows = [], []
        ledger_fault = str(error)
    per_domain = stream.get("per_domain", {}) or {}
    expected = list(stream.get("expected_domains", []) or [])
    frames = int(stream.get("frame_end_record_count", 0) or 0)

    advanced = int(manifest.get("game_hours_advanced", 0) or 0)
    observed = int(manifest.get("unique_game_hour_count", 0) or 0)
    skipped_declared = int(manifest.get("skipped_game_hours", 0) or 0)

    def _span(record, start_key, end_key):
        start, end = record.get(start_key), record.get(end_key)
        if start is None or end is None or end < start:
            return 0
        return end - start + 1

    # Domain-hours, not records. An H4DG range is INCLUSIVE and can be
    # coalesced across several hours, so a record count understates the loss --
    # t17 carries 7238 records over 8054 domain-hours.
    gap_outcome_hours = collections.Counter()
    for outcome in gap_outcome_rows:
        gap_outcome_hours[outcome.get("reason", "?")] += _span(
            outcome, "range_start_hour_raw", "range_end_hour_raw")

    gap_reason_counts = collections.Counter(
        gap.get("reason", "?") for gap in gap_rows
    )
    gap_outcomes = collections.Counter(
        outcome.get("reason", "?")
        for outcome in gap_outcome_rows
    )

    data_records = int(stream.get("domain_data_record_count", 0) or 0)
    gap_records = int(stream.get("domain_gap_record_count", 0) or 0)
    domain_hours = frames * len(expected)

    # A domain-hour must resolve to exactly one of: a data record, or a gap
    # record naming why there is none. Anything left over is an hour that no
    # ledger accounts for, which is the failure mode that hides real loss.
    #
    # Reconcile per domain, not in aggregate: one domain silent for hundreds of
    # hours and another over-reporting would cancel out in a single subtraction.
    gaps_by_id = collections.Counter(
        outcome.get("domain_id") for outcome in gap_outcome_rows
    )
    unresolved_by_domain = {}
    for name, info in per_domain.items():
        resolved = int(info.get("data_record_count", 0) or 0) + gaps_by_id.get(
            info.get("domain_id"), 0
        )
        if resolved != frames:
            unresolved_by_domain[name] = frames - resolved
    unresolved_domain_hours = sum(abs(v) for v in unresolved_by_domain.values())

    # The hours the clock advanced through but no frame observed, and the part
    # of that which no skipped-hour gap explains.
    #
    # Two traps here, both of which produced wrong numbers on t15 and both of
    # which are the reason this quantity must not be eyeballed:
    #
    # 1. `game_hours_advanced` is a DELTA (last_hour - first_hour), while
    #    `unique_game_hour_count` is a COUNT. Subtracting one from the other
    #    drops the inclusive endpoint and reports one hour too few -- 145 where
    #    the capture really has 146. The span is `advanced + 1` hours.
    # 2. `skipped_hour` gap entries are COALESCED RANGES, so counting the
    #    records counts runs, not hours. t15 carries 125 such records covering
    #    exactly 146 hours (105 runs of 1, 19 of 2, 1 of 3). Comparing 146
    #    against 125 invents 20 unexplained hours that do not exist.
    span_hours = advanced + 1 if advanced or observed else 0
    hours_unobserved = span_hours - observed
    skipped_hours_covered = 0
    for gap in gap_rows:
        if gap.get("reason") not in SKIPPED_HOUR_REASONS:
            continue
        start = gap.get("hour_start")
        end = gap.get("hour_end")
        if start is None or end is None or end < start:
            continue
        skipped_hours_covered += end - start + 1
    hours_unaccounted = hours_unobserved - skipped_hours_covered

    # `reader_silent` is the seal saying a scheduled domain produced neither a
    # data record nor any statement about why. It is an honest floor, not a
    # clean outcome: the hour IS now accounted, which is the whole point, but
    # the thing being accounted is a producer that returned nothing. Counted on
    # its own axis so it can never pass as an ordinary explained gap.
    reader_silent_by_domain = collections.Counter()
    for outcome in gap_outcome_rows:
        if outcome.get("reason") == "reader_silent":
            reader_silent_by_domain[outcome.get("domain_id")] += 1

    # The host counts gap records per domain while building `per_domain`, and
    # the outcome ledger is scanned separately. They describe the same records,
    # so a divergence means one of them is not counting something.
    gap_count_disagreements = {}
    for name, info in per_domain.items():
        declared = info.get("gap_record_count")
        if declared is None:
            continue
        # NOT `observed` -- that name holds unique_game_hour_count, and
        # rebinding it here left the hours axis explaining itself with the
        # last domain's gap count ("89 observed of 594") while its headline
        # number, computed earlier, was right. A shadowed name in a report is
        # a wrong report.
        counted = gaps_by_id.get(info.get("domain_id"), 0)
        if int(declared) != counted:
            gap_count_disagreements[name] = (int(declared), counted)

    # A domain can be absent from `per_domain` and still be emitting gaps, so
    # name both shapes: it is the shape the per-domain report cannot see.
    known_ids = {info.get("domain_id") for info in per_domain.values()}
    absent = sorted(set(expected) - set(per_domain))
    absent_but_gapping = sorted(
        f"d{domain_id} ({gaps_by_id[domain_id]} gaps)"
        for domain_id in gaps_by_id
        if domain_id not in known_ids
    )
    zero_records = sorted(
        name
        for name, info in per_domain.items()
        if int(info.get("data_record_count", 0) or 0) == 0
    )
    zero_rows = sorted(
        name
        for name, info in per_domain.items()
        if int(info.get("row_count_total", 0) or 0) == 0
    )

    # A domain that produced nothing has almost always SAID why, once per
    # frame, in its degraded record -- and naming the domain without its reason
    # is what sent a whole lane after an answer the manifest already carried.
    # Report the dominant reason beside each name so this file answers "why is
    # it empty" without a second tool.
    reason_by_domain = collections.defaultdict(collections.Counter)
    for frame in stream.get("frame_degraded_outcomes", []) or []:
        for entry in frame.get("domains", []) or []:
            reason = (entry.get("reason") or "").strip()
            if reason:
                reason_by_domain[entry.get("domain_id")][reason] += 1

    def with_reason(name):
        info = per_domain.get(name) or {}
        counts = reason_by_domain.get(info.get("domain_id"))
        if not counts:
            return name
        reason, hits = counts.most_common(1)[0]
        # The reasons carry per-frame values, so the leading clause is the
        # stable part; the tail is a sample and would differ every frame.
        return f"{name} [{hits}x {reason[:110]}]"

    # Producers that counted what they threw away. Any nonzero counter here is
    # data the reader saw and did not record.
    dropped = {}
    for name, info in per_domain.items():
        problems = info.get("problems") or {}
        total = sum(v for v in problems.values() if isinstance(v, int))
        if total:
            dropped[name] = problems

    excluded = stream.get("excluded_domain_data", {}) or {}
    artifact = manifest.get("replay_artifact", {}) or {}
    hourly = manifest.get("hourly_requirement", {}) or {}

    # A degraded-frame count on its own says nothing actionable: 505 of 592 is
    # either one broken domain repeated 505 times or 505 unrelated events, and
    # those call for opposite responses. Name the offenders and how often each
    # appears, so the count carries its own explanation.
    degraded_outcomes = stream.get("frame_degraded_outcomes", []) or []
    degraded_by_domain = collections.Counter()
    for frame in degraded_outcomes:
        for offender in frame.get("domains", []) or []:
            degraded_by_domain[offender.get("domain_id")] += 1
    name_by_id = {
        info.get("domain_id"): name for name, info in per_domain.items()
    }
    degraded_detail = ", ".join(
        f"d{domain_id} {name_by_id.get(domain_id, '?')}={count}"
        for domain_id, count in degraded_by_domain.most_common()
    ) or "none"
    # A domain that offends in EVERY degraded frame is a standing condition, not
    # an event; separating those says how many frames would still be degraded if
    # it were fixed.
    always_degrading = {
        domain_id for domain_id, count in degraded_by_domain.items()
        if degraded_outcomes and count == len(degraded_outcomes)
    }
    frames_degraded_by_others = sum(
        1 for frame in degraded_outcomes
        if {offender.get("domain_id") for offender in frame.get("domains", []) or []}
        - always_degrading
    )
    if always_degrading:
        degraded_detail += (
            "; always-degrading: "
            + ", ".join(f"d{i}" for i in sorted(always_degrading))
            + f"; {frames_degraded_by_others} of {len(degraded_outcomes)} frames "
              "would still be degraded without them")

    # A production capture must not expect a domain the owner ruled out of
    # capture scope: if it does, some roster path did not get the deschedule and
    # the capture is about to count hundreds of empty frames against a domain
    # nobody asked for. An evidence capture that deliberately passes
    # `--scheduled-domain` for one of them will also show these two axes; that
    # is a deliberate configuration showing up as expected, not a silent one.
    descheduled_block = descheduled(manifest)
    descheduled_names = {
        entry.get("domain")
        for entry in descheduled_block.get("domains", []) or []
    }
    descheduled_but_expected = sorted(descheduled_names & set(expected))
    descheduled_but_reported = sorted(descheduled_names & set(per_domain))

    # The record itself is an axis. A capture whose roster shrank but whose
    # manifest carries no reasoned account of why is exactly the D81 shape this
    # whole mechanism exists to prevent, and it must not score COMPLETE.
    record_faults = []
    if not descheduled_block:
        record_faults.append(
            "no descheduled_domains block in the manifest — a current recorder always "
            "seals one, so this is either a dropped record or a manifest sealed before "
            "the block existed")
    else:
        if not descheduled_names:
            record_faults.append("descheduled_domains block names no domains")
        if any(not name for name in descheduled_names):
            record_faults.append("a descheduled domain has no wire name")
        for field in ("reason", "decided_on", "decided_by", "authority"):
            if not str(descheduled_block.get(field) or "").strip():
                record_faults.append(f"descheduled_domains.{field} is missing or empty")
        # Non-emptiness alone lets a record drop entries or invent one. Compare
        # it against the registry, which owns the decision.
        expected_pairs = registry_descheduled()
        if expected_pairs is None:
            record_faults.append(
                "cannot reach domain_registry.json to verify the record names the "
                "domains the registry rules out of capture scope")
        else:
            recorded_pairs = {
                (entry.get("domain_id"), entry.get("domain"))
                for entry in descheduled_block.get("domains", []) or []
            }
            for missing in sorted(expected_pairs - recorded_pairs, key=lambda p: (p[0] or 0)):
                record_faults.append(
                    f"the checkout's registry rules d{missing[0]} {missing[1]} out of "
                    "capture scope but the manifest record omits it")
            for extra in sorted(recorded_pairs - expected_pairs, key=lambda p: (p[0] or 0)):
                record_faults.append(
                    f"the manifest records d{extra[0]} {extra[1]} as descheduled but the "
                    "checkout's registry does not")

    # The two ledgers are NOT two counts of the same thing, and comparing them
    # per-reason for equality -- which this axis used to do -- overstates loss.
    #
    # `domain_gap_outcomes` is one entry per H4DG record: a scheduled domain's
    # own terminal statement that it emitted NO data for those hours. That is
    # the gap count.
    #
    # `gaps` is the host-derived ledger, and `derive_gap_records` builds it as a
    # SUPERSET: every H4DG projected into it, PLUS whole-clock skipped hours,
    # PLUS one record per domain-hour that DID emit data and carried a failure
    # status bit. Those last are annotations on delivered data, not absences.
    #
    # On t17 the old axis read `source_unavailable: 4703 vs 501` and invited the
    # reading that 4202 domain-hours were lost. They were not: all 4202 have a
    # DOMAIN_DATA record in the stream. The 501 are the real gaps.
    #
    # The reasons are NOT the join key: the recorder renames some on the way in
    # (an H4DG `torn` projects to a ledger `torn_sweep`, `overrun` to
    # `backpressure`), so subtracting per reason invents a difference where
    # there is none. Restating that mapping here would be a second copy of the
    # recorder's table, free to drift from it silently.
    #
    # The (hour, domain) PAIR is a sound join key and needs no table. A derived
    # record is backed by a producer outcome exactly when the pairs it claims
    # are claimed by an H4DG for the same domain; anything else it claims is
    # host-derived. The recorder guarantees at most one outcome per domain-hour,
    # so the split is unambiguous.
    outcome_pairs = set()
    for outcome in gap_outcome_rows:
        domain_id = outcome.get("domain_id")
        start = outcome.get("range_start_hour_raw")
        end = outcome.get("range_end_hour_raw")
        if domain_id is None or start is None or end is None or end < start:
            continue
        for hour in range(start, end + 1):
            outcome_pairs.add((hour, domain_id))

    # The host-derived arm is not one thing. `derive_gap_records` raises a
    # COVERAGE witness when a domain-hour produced no PERF or too few DATA, and
    # a STATUS witness when it produced its records and flagged them. The first
    # is an absence the producer never named; the second is delivered data. They
    # must not share an axis -- counting a coverage shortfall as a delivery is
    # the same overstatement, pointing the other way, as the equality axis this
    # replaced.
    #
    # The witness names the difference itself, so no reason table is needed: a
    # coverage witness carries the observed/expected record counts it is
    # asserting, and arm (c) only ever raises one when something was short.
    COVERAGE_WITNESSES = {"coverage", "coverage_and_status"}

    claimed_pairs = set()
    flagged_pairs = set()
    flagged_by_reason = collections.Counter()
    host_absent_pairs = set()
    host_absent_by_reason = collections.Counter()
    for record in gap_rows:
        start, end = record.get("hour_start"), record.get("hour_end")
        domains = record.get("domains") or []
        if start is None or end is None or end < start:
            continue
        if not domains:
            continue  # a global whole-clock hour, not a per-domain outcome
        witness = record.get("witness") or {}
        is_absence = witness.get("kind") in COVERAGE_WITNESSES
        for domain_id in domains:
            for hour in range(start, end + 1):
                if (hour, domain_id) in outcome_pairs:
                    claimed_pairs.add((hour, domain_id))
                elif is_absence:
                    host_absent_pairs.add((hour, domain_id))
                    host_absent_by_reason[record.get("reason", "?")] += 1
                else:
                    flagged_pairs.add((hour, domain_id))
                    flagged_by_reason[record.get("reason", "?")] += 1

    # Every producer outcome must reach the derived ledger. This is the real
    # reconciliation the old equality axis was reaching for, and unlike it a
    # nonzero count here is genuine loss: a domain-hour the producer said it
    # could not fill, that the ledger the replay is graded against never heard
    # about.
    unprojected_outcome_hours = len(outcome_pairs - claimed_pairs)
    flagged_delivered_hours = sum(flagged_by_reason.values())
    host_absent_hours = sum(host_absent_by_reason.values())
    outcome_hours = sum(gap_outcome_hours.values())

    # A flagged delivery whose payload was then rejected from the fold has an
    # envelope and no content. It is counted on the exclusion axis too; said
    # here so the flagged number is not read as "delivered but usable".
    excluded_records = (excluded.get("records") or [])
    excluded_pairs = {
        (record.get("game_hour_raw"), record.get("domain_id"))
        for record in excluded_records
    }
    def _as_id(key):
        try:
            return int(key)
        except (TypeError, ValueError):
            return None

    excluded_detail = ", ".join(
        f"d{key} {name_by_id.get(_as_id(key), '?')}={count}"
        for key, count in sorted(
            (excluded.get("per_domain") or {}).items(),
            key=lambda item: (_as_id(item[0]) is None, _as_id(item[0]) or 0, item[0]))
    ) or "none"
    if excluded_pairs:
        also_flagged = len(excluded_pairs & flagged_pairs)
        also_gapped = len(excluded_pairs & outcome_pairs)
        excluded_detail += (
            f"; {len(excluded_pairs)} distinct domain-hours"
            f" ({also_flagged} also carry a producer failure status,"
            f" {also_gapped} sit inside a producer gap,"
            f" {len(excluded_pairs) - also_flagged - also_gapped} are otherwise"
            " indistinguishable from a clean delivery)")

    # COVERED-BY-DATA is not COVERED-BY-EXPLANATION, and the coverage gate
    # cannot tell them apart.
    #
    # `compact_domain_coverage_errors_with_gaps` asks one question per
    # scheduled domain-hour: did a PERF record arrive, and where the domain
    # requires one, a DATA record. d81 answers yes for every one of its 318
    # hours and carries no rail state in any of them -- every record is a
    # SOURCE_UNAVAILABLE envelope whose payload is empty. `domain_coverage.ok`
    # is therefore true, and a reader who stops at that word reads the capture
    # as covering d81. The hours ARE accounted; what they are accounted BY is
    # an explanation, not data, and only one of those is the thing the capture
    # was run to collect.
    #
    # Both facts have to be printed, because a domain carrying no rows is two
    # different situations that call for opposite responses. A domain can be
    # legitimately empty for a whole capture -- no nuclear strike was ever
    # made, so d24 has nothing to say and says it cleanly. That is not the d81
    # shape. The residual failure-status word is exactly what separates them:
    # d81's hours carry `source_unavailable`, the empty-but-clean domain's
    # carry nothing. So the axis COUNTS only the domains whose emptiness is a
    # named failure, and prints the clean-empty ones beside it as context that
    # must never silently become a miss.
    #
    # Derived here from `per_domain` rather than read from a manifest field so
    # that it also scores captures sealed before the recorder published its own
    # split -- t23 included.
    explanation_only = []
    empty_but_clean = []
    for name in expected:
        info = per_domain.get(name)
        if info is None:
            # "expected domains with no per-domain record" already owns this;
            # counting it twice would double-report one absence.
            continue
        if int(info.get("row_count_total", 0) or 0):
            continue
        domain_id = info.get("domain_id")
        data_records = int(info.get("data_record_count", 0) or 0)
        gap_records = gaps_by_id.get(domain_id, 0)
        if not data_records and not gap_records:
            # Nothing accounts for these hours at all. That is a strictly worse
            # condition and the reconciliation axis above is what names it.
            continue
        failure = residual_failure_status(info)
        entry = (name, domain_id, data_records, gap_records, failure)
        (explanation_only if failure else empty_but_clean).append(entry)

    def _coverage_line(entry):
        name, domain_id, data_records, gap_records, failure = entry
        return (f"{name} [d{domain_id}: {data_records} data records, "
                f"{gap_records} gap records, 0 rows in the whole capture, "
                f"status={failure_status_names(failure) or 'clean'}]")

    coverage_detail = (
        chr(10).join(_coverage_line(e) for e in sorted(explanation_only))
        or "none")
    if empty_but_clean:
        coverage_detail += chr(10) + (
            "carried no rows either, but with no failure status, so this is "
            "read as nothing to report rather than nothing read: "
            + ", ".join(f"{e[0]} (d{e[1]})" for e in sorted(empty_but_clean)))

    axes = [
        # FIRST, because every gap axis below is read through the ledger: if this
        # is nonzero, their zeros mean "not read", not "nothing there".
        ("the gap ledger could not be read", 1 if ledger_fault else 0,
         ledger_fault or "read"),
        ("hours the clock advanced but no frame observed", hours_unobserved,
         f"{observed} observed of {span_hours} in span "
         f"(game_hours_advanced={advanced} is a delta, so the span is +1)"),
        ("...of those, hours no skipped-hour gap explains", hours_unaccounted,
         f"manifest declares skipped_game_hours={skipped_declared}, "
         f"ledger carries {gap_reason_counts.get('skipped_hour', 0)} skipped_hour "
         f"records covering {skipped_hours_covered} hours"),
        ("domain-hours with a gap instead of data", outcome_hours,
         (", ".join(f"{r}={n}" for r, n in sorted(gap_outcome_hours.items())) or "none")
         + f"  [{sum(gap_outcomes.values())} H4DG records over inclusive ranges;"
           f" a coalesced range covers more than one hour]"),
        ("domain-hours no ledger accounts for", unresolved_domain_hours,
         "; ".join(f"{n} short by {v}" if v > 0 else f"{n} over by {-v}"
                   for n, v in sorted(unresolved_by_domain.items()))
         + f"  [of {domain_hours} domain-hours = {frames} frames x {len(expected)} domains, "
           f"{data_records} data + {gap_records} gap]"),
        ("domain-hours a producer left with no outcome at all",
         sum(reader_silent_by_domain.values()),
         "; ".join(f"d{domain_id}={n}"
                   for domain_id, n in sorted(reader_silent_by_domain.items()))
         or "none"),
        ("per-domain gap counts disagreeing with the outcome ledger",
         len(gap_count_disagreements),
         json.dumps(gap_count_disagreements) if gap_count_disagreements else "none"),
        ("expected domains with no per-domain record", len(absent),
         ", ".join(absent) or "none"),
        ("domains emitting gaps but absent from per-domain", len(absent_but_gapping),
         ", ".join(absent_but_gapping) or "none"),
        ("domains that produced no data record", len(zero_records),
         ", ".join(zero_records) or "none"),
        ("domains that produced no rows", len(zero_rows),
         chr(10).join(with_reason(n) for n in zero_rows) or "none"),
        ("scheduled domains covered only by an explanation, never by data",
         len(explanation_only), coverage_detail),
        ("domains reporting dropped or incomplete events", len(dropped),
         "; ".join(f"{n} {json.dumps(p)}" for n, p in sorted(dropped.items())) or "none"),
        ("faults in the deliberately-not-scheduled record", len(record_faults),
         "; ".join(record_faults) or "none"),
        ("descheduled domains still in the expected roster", len(descheduled_but_expected),
         ", ".join(descheduled_but_expected) or "none"),
        ("descheduled domains that reported data", len(descheduled_but_reported),
         ", ".join(descheduled_but_reported) or "none"),
        ("refused frames", int(stream.get("frame_refused_record_count", 0) or 0), ""),
        ("degraded frames", int(stream.get("frame_degraded_record_count", 0) or 0),
         degraded_detail),
        ("records excluded from the seal", int(excluded.get("total_records", 0) or 0),
         excluded_detail),
        ("producer gap domain-hours the derived ledger never claimed",
         unprojected_outcome_hours,
         "an H4DG outcome that no derived record covers: the replay is graded "
         "against a ledger that has not heard of it"),
        ("domain-hours absent with no producer statement", host_absent_hours,
         (", ".join(f"{r}={n}" for r, n in sorted(host_absent_by_reason.items())) or "none")
         + "  [the host derived the absence from missing PERF/DATA; no producer "
           "said why. Worse than a gap, not better: a named gap is evidence]"),
        ("domain-hours delivered with a producer failure status",
         flagged_delivered_hours,
         (", ".join(f"{r}={n}" for r, n in sorted(flagged_by_reason.items())) or "none")
         + "  [these DID emit a data record; they are not gaps, and they are "
           "not clean either. The split is by CANONICAL reason, which is a "
           "priority order over the status word, not a partition of it: a "
           "domain-hour carrying both source_unavailable and partial_data is "
           "counted once, under source_unavailable. Read partial_data as a "
           "floor, never as the partial-data population]"),
        ("parse errors", len(manifest.get("parse_errors", []) or []), ""),
        ("ordering errors", len(manifest.get("ordering_errors", []) or []), ""),
        ("collector control errors",
         len(manifest.get("collector_control_errors", []) or []), ""),
        ("domain coverage errors",
         len((stream.get("domain_coverage", {}) or {}).get("errors", []) or []), ""),
        ("replay gate problems", len(artifact.get("gate_problems", []) or []),
         ", ".join(artifact.get("gate_problems", []) or []) or "none"),
        # `hourly_requirement.problems` is the WHOLE replay gate despite its
        # name -- `requested_game_hours` governs exactly one of its checks. The
        # old label said "hourly requirement problems", which sent a reader
        # after the hour floor for a failure that had nothing to do with it.
        ("replay gate problems as sealed under hourly_requirement",
         len(hourly.get("problems", []) or []),
         ", ".join(hourly.get("problems", []) or []) or "none"),
    ]

    # A recovery replay is a failed seal wearing a replay's name.
    if artifact.get("recovery"):
        axes.append(("replay is a recovery artifact, not a promotable one", 1, ""))
    # The hour floor asserts nothing at zero. Stated as vacuity, not as a pass:
    # the old wording claimed the requirement "passed" without ever reading
    # `met`, and printed that on t17 while `met` was false.
    if hourly.get("required") and not int(hourly.get("requested_game_hours", 0) or 0):
        axes.append(("game-hour floor requested was zero, so it asserts nothing", 1,
                     "--min-game-hours 0 cannot distinguish a capture that "
                     "advanced hundreds of game hours from one whose clock "
                     "never moved; it is not what failed this run, and "
                     f"hourly_requirement.met is {hourly.get('met')}"))

    return axes


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run-dir", required=True)
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()

    run_dir = os.path.abspath(args.run_dir)
    manifest, manifest_path = load_manifest(run_dir)
    axes = score(manifest)
    misses = [axis for axis in axes if axis[1]]
    not_scheduled = descheduled(manifest)

    if args.json:
        json.dump({
            "run_dir": run_dir,
            "manifest": manifest_path,
            "complete": not misses,
            "axes": [{"axis": a, "count": n, "detail": d} for a, n, d in axes],
            "descheduled_domains": not_scheduled,
        }, sys.stdout, indent=2)
        print()
        return 0 if not misses else 1

    print(f"\n=== capture completeness: {manifest.get('session_id')} ===")
    print(f"  run dir  {run_dir}")
    print(f"  duration {manifest.get('duration_ms')} ms\n")
    width = max(len(a) for a, _, _ in axes)
    for axis, count, detail in axes:
        mark = " " if count == 0 else "!"
        print(f"{mark} {axis:{width}s} {count}")
        if detail and count:
            # A multi-line detail (one entry per line) must keep the detail
            # column, or the continuation lines read as separate axes.
            pad = chr(10) + f"  {'':{width}s}   "
            print(f"  {'':{width}s}   {detail.replace(chr(10), pad)}")

    # Printed unconditionally, and outside the axis table, because a clean
    # scorecard must still say what it was not asked to capture. A capture that
    # scores COMPLETE over a silently shrunken roster is the failure this
    # section exists to prevent.
    names = [entry.get("domain") for entry in not_scheduled.get("domains", []) or []]
    if names:
        print(f"\n  deliberately not scheduled ({len(names)}): {', '.join(names)}")
        print(f"    reason:    {not_scheduled.get('reason', '?')}")
        print(f"    decided:   {not_scheduled.get('decided_on', '?')}"
              f" by {not_scheduled.get('decided_by', '?')}"
              f" ({not_scheduled.get('authority', '?')})")
    else:
        print("\n  deliberately not scheduled: no record in this manifest"
              " (see the record-fault axis above)")

    if misses:
        print(f"\nINCOMPLETE: {len(misses)} of {len(axes)} axes carry a miss\n")
        return 1
    print(f"\nCOMPLETE: all {len(axes)} axes clean\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
