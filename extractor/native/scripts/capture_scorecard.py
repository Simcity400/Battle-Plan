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
import hashlib
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import gap_ledger

SKIPPED_HOUR_REASONS = {"skipped_hour"}

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
            with open(path, "rb") as handle:
                raw = handle.read()
            return json.loads(raw), path, byte_identity(path, raw)
    raise SystemExit(f"no session manifest in {run_dir}")


def byte_identity(path, raw):
    """Identify the bytes actually assessed, never a later reopen of the file."""
    return {"path": os.path.abspath(path), "bytes": len(raw),
            "sha256": hashlib.sha256(raw).hexdigest()}


def evidence_references(run_dir, manifest_identity):
    """Inventory lifecycle evidence without impersonating its authority checks.

    The PowerShell lifecycle verifier authenticates READY/teardown against
    independently established process and configuration pins. Merely finding
    those files beside a manifest cannot establish passive capture or unloading.
    This inventory deliberately makes no current-build or admission claim.
    """
    references = {}
    for name in ("CAPTURE_READY_V2.json", "CAPTURE_TEARDOWN_V1.json",
                 "CAPTURE_REFUSAL_V1.json", "HOOK_INSTALL_OUTCOMES_V1.json",
                 "HOOK_RESTORATION_OUTCOMES_V1.json", "passive_capture_policy.json",
                 "PASSIVE_CLOCK_INTERVALS_V2.json"):
        path = os.path.join(run_dir, name)
        try:
            with open(path, "rb") as handle:
                raw = handle.read()
        except FileNotFoundError:
            references[name] = {"status": "missing"}
        except OSError as error:
            references[name] = {"status": "unreadable", "error": str(error)}
        else:
            references[name] = {"status": "present_not_authenticated",
                                "identity": byte_identity(path, raw)}
    return {
        "scope": "manifest_completeness_only",
        "manifest_identity": manifest_identity,
        "lifecycle_receipts": references,
        "not_evaluated": ["current_release_artifact_binding",
                          "automatic_attachment_scenarios",
                          "passive_capture", "authenticated_ready",
                          "hooks_restored_byte_for_byte", "collector_unloaded",
                          "container_and_content_admission"],
    }


def descheduled(manifest):
    """The domains the capture deliberately did not schedule.

    Returned separately from `score` because this is not an axis: fewer
    descheduled domains is not better. It exists so a reader can tell
    "we chose not to capture this" from "this failed to produce" — the two look
    identical once a domain is simply absent from every ledger.
    """
    stream = manifest.get("compact_stream", {}) or {}
    return stream.get("descheduled_domains", {}) or {}


def registry_descheduled(native_root=None):
    """(id, wire name) pairs the domain registry marks out of capture scope.

    An external expectation is what makes the record check more than a
    non-emptiness test: without it a manifest can drop entries, or
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


def readable_domain_id(value):
    """The entry's domain id as an `int`, or `None` if it is not one.

    On the wire a domain id is a `u16`, so anything else is a malformed entry.
    This returns `None` rather than the value because ADMITTING a foreign type
    is what broke this file: a frame carrying both `47` and `"47"` put two
    different types into one `set`, and the `sorted()` that renders the
    always-degrading list then raised `TypeError` and took the whole scorecard
    down -- the exact totality failure the defensive parsing was added to end,
    reached by a route the per-entry checks could not see because it only
    appears when two entries are compared to each other.

    Not coerced, for the reason the raw-capture report gives at its own parse:
    `int("47")` would silently merge a malformed entry into d47's real counts,
    and a report that invents an attribution is worse than one that admits it
    could not read a row. `bool` is an `int` in Python and never a domain id.
    """
    if isinstance(value, bool) or not isinstance(value, int):
        return None
    return value


def score(manifest):
    """Every axis on which the capture can be short of complete.

    Returns a list of (key, count, detail) where a nonzero count is a miss.
    """
    stream = manifest.get("compact_stream", {}) or {}
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

    reader_silent_by_domain = collections.Counter()
    for outcome in gap_outcome_rows:
        if outcome.get("reason") == "reader_silent":
            reader_silent_by_domain[outcome.get("domain_id")] += 1

    gap_count_disagreements = {}
    for name, info in per_domain.items():
        declared = info.get("gap_record_count")
        if declared is None:
            continue
        counted = gaps_by_id.get(info.get("domain_id"), 0)
        if int(declared) != counted:
            gap_count_disagreements[name] = (int(declared), counted)

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

    reason_by_domain = collections.defaultdict(collections.Counter)
    sample_reason = {}
    predicate_label = {}
    degraded_outcomes = stream.get("frame_degraded_outcomes")
    for frame in degraded_outcomes if isinstance(degraded_outcomes, list) else []:
        if not isinstance(frame, dict):
            continue
        entries = frame.get("domains")
        for entry in entries if isinstance(entries, list) else []:
            if not isinstance(entry, dict):
                continue
            reason = entry.get("reason")
            reason = reason.strip() if isinstance(reason, str) else ""
            code = entry.get("predicate_code")
            name = entry.get("predicate_name")
            if isinstance(code, bool) or not isinstance(code, int):
                key = "(no readable predicate)"
                label = key
            else:
                key = code
                shown = name if isinstance(name, str) and name.strip() else "(unnamed)"
                label = f"{shown} [0x{code:08x}]"
            domain_id = readable_domain_id(entry.get("domain_id"))
            if domain_id is None:
                continue
            reason_by_domain[domain_id][key] += 1
            predicate_label.setdefault((domain_id, key), label)
            sample_reason.setdefault((domain_id, key), reason)

    def with_reason(name):
        info = per_domain.get(name) or {}
        domain_id = info.get("domain_id")
        counts = reason_by_domain.get(domain_id)
        if not counts:
            return name
        key, hits = counts.most_common(1)[0]
        label = predicate_label.get((domain_id, key), str(key))
        sample = sample_reason.get((domain_id, key), "")
        if sample:
            return f"{name} [{hits}x {label}: {sample[:90]}]"
        return f"{name} [{hits}x {label[:110]}]"

    dropped = {}
    for name, info in per_domain.items():
        problems = info.get("problems") or {}
        total = sum(v for v in problems.values() if isinstance(v, int))
        if total:
            dropped[name] = problems

    excluded = stream.get("excluded_domain_data", {}) or {}
    artifact = manifest.get("replay_artifact", {}) or {}
    hourly = manifest.get("hourly_requirement", {}) or {}

    raw_degraded = stream.get("frame_degraded_outcomes")
    degraded_offenders = []
    unreadable_records = 0
    if raw_degraded is not None and not isinstance(raw_degraded, list):
        unreadable_records += 1
    for frame in raw_degraded if isinstance(raw_degraded, list) else []:
        if not isinstance(frame, dict):
            unreadable_records += 1
            continue
        offenders = frame.get("domains")
        if offenders is not None and not isinstance(offenders, list):
            unreadable_records += 1
        ids = set()
        for offender in offenders if isinstance(offenders, list) else []:
            if not isinstance(offender, dict):
                unreadable_records += 1
                continue
            domain_id = readable_domain_id(offender.get("domain_id"))
            if domain_id is None:
                unreadable_records += 1
                continue
            ids.add(domain_id)
        degraded_offenders.append(ids)
    degraded_frame_count = len(degraded_offenders)
    degraded_by_domain = collections.Counter()
    for ids in degraded_offenders:
        degraded_by_domain.update(ids)
    name_by_id = {
        info.get("domain_id"): name for name, info in per_domain.items()
    }
    degraded_detail = ", ".join(
        f"d{domain_id} {name_by_id.get(domain_id, '?')}={count}"
        for domain_id, count in degraded_by_domain.most_common()
    ) or "none"
    always_degrading = {
        domain_id for domain_id, count in degraded_by_domain.items()
        if degraded_frame_count and count == degraded_frame_count
    }
    frames_degraded_by_others = sum(
        1 for ids in degraded_offenders if ids - always_degrading
    )
    if always_degrading:
        degraded_detail += (
            "; always-degrading: "
            + ", ".join(f"d{i}" for i in sorted(always_degrading))
            + f"; {frames_degraded_by_others} of {degraded_frame_count} frames "
              "would still be degraded without them")
    if unreadable_records:
        degraded_detail += (
            f"; {unreadable_records} degraded record"
            + ("" if unreadable_records == 1 else "s")
            + " unreadable and excluded from these counts")

    descheduled_block = descheduled(manifest)
    descheduled_names = {
        entry.get("domain")
        for entry in descheduled_block.get("domains", []) or []
    }
    descheduled_but_expected = sorted(descheduled_names & set(expected))
    descheduled_but_reported = sorted(descheduled_names & set(per_domain))

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
        for entry in descheduled_block.get("domains", []) or []:
            name = entry.get("domain") or "<unnamed>"
            for field in ("decided_on", "reason"):
                if not str(entry.get(field) or "").strip():
                    record_faults.append(
                        f"descheduled domain {name} publishes no {field}; a reader cannot "
                        "tell which ruling covered it")
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

    outcome_pairs = set()
    for outcome in gap_outcome_rows:
        domain_id = outcome.get("domain_id")
        start = outcome.get("range_start_hour_raw")
        end = outcome.get("range_end_hour_raw")
        if domain_id is None or start is None or end is None or end < start:
            continue
        for hour in range(start, end + 1):
            outcome_pairs.add((hour, domain_id))

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
            continue
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

    unprojected_outcome_hours = len(outcome_pairs - claimed_pairs)
    flagged_delivered_hours = sum(flagged_by_reason.values())
    host_absent_hours = sum(host_absent_by_reason.values())
    outcome_hours = sum(gap_outcome_hours.values())

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

    explanation_only = []
    empty_but_clean = []
    for name in expected:
        info = per_domain.get(name)
        if info is None:
            continue
        if int(info.get("row_count_total", 0) or 0):
            continue
        domain_id = info.get("domain_id")
        data_records = int(info.get("data_record_count", 0) or 0)
        gap_records = gaps_by_id.get(domain_id, 0)
        if not data_records and not gap_records:
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
        ("capture has no observed frames", int(frames <= 0),
         f"frame_end_record_count={frames}; no frames cannot prove capture"),
        ("capture has no expected domain roster", int(not expected),
         "an empty schedule cannot establish scheduled-domain coverage"),
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
        ("replay container packaging failed", int(bool(artifact.get("pack_failed"))),
         str(artifact.get("pack_error") or artifact.get("pack_failure_note") or "none")),
        ("replay gate problems as sealed under hourly_requirement",
         len(hourly.get("problems", []) or []),
         ", ".join(hourly.get("problems", []) or []) or "none"),
    ]

    if artifact.get("recovery"):
        axes.append(("replay is a recovery artifact, not a promotable one", 1, ""))
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
    manifest, manifest_path, manifest_identity = load_manifest(run_dir)
    axes = score(manifest)
    misses = [axis for axis in axes if axis[1]]
    not_scheduled = descheduled(manifest)

    if args.json:
        json.dump({
            "schema": "hoi4.capture_scorecard.v1",
            "run_dir": run_dir,
            "manifest": manifest_path,
            "complete": not misses,
            "axes": [{"axis": a, "count": n, "detail": d} for a, n, d in axes],
            "descheduled_domains": not_scheduled,
            "evidence": evidence_references(run_dir, manifest_identity),
        }, sys.stdout, indent=2)
        print()
        return 0 if not misses else 1

    print(f"\n=== capture completeness: {manifest.get('session_id')} ===")
    print(f"  run dir  {run_dir}")
    print(f"  duration {manifest.get('duration_ms')} ms\n")
    print("  Scope: manifest completeness only; artifact binding, passive capture,"
          " hook restoration, unload, and container admission are not evaluated.\n")
    width = max(len(a) for a, _, _ in axes)
    for axis, count, detail in axes:
        mark = " " if count == 0 else "!"
        print(f"{mark} {axis:{width}s} {count}")
        if detail and count:
            pad = chr(10) + f"  {'':{width}s}   "
            print(f"  {'':{width}s}   {detail.replace(chr(10), pad)}")

    entries = not_scheduled.get("domains", []) or []
    names = [entry.get("domain") for entry in entries]
    if names:
        print(f"\n  deliberately not scheduled ({len(names)}): {', '.join(names)}")
        by_ruling = {}
        for entry in entries:
            key = (str(entry.get("decided_on") or "?"), str(entry.get("reason") or "?"))
            by_ruling.setdefault(key, []).append(entry.get("domain") or "<unnamed>")
        for (decided_on, reason), domains in sorted(by_ruling.items()):
            print(f"    ruling {decided_on}"
                  f" by {not_scheduled.get('decided_by', '?')}"
                  f" ({not_scheduled.get('authority', '?')}): {', '.join(sorted(domains))}")
            print(f"      reason:  {reason}")
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
