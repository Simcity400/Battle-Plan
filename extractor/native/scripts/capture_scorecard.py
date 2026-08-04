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

# Gap reasons that mean an hour was deliberately skipped rather than lost to a
# reader. Kept separate because they reconcile against `skipped_game_hours`.
SKIPPED_HOUR_REASONS = {"skipped_hour"}


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
    per_domain = stream.get("per_domain", {}) or {}
    expected = list(stream.get("expected_domains", []) or [])
    frames = int(stream.get("frame_end_record_count", 0) or 0)

    advanced = int(manifest.get("game_hours_advanced", 0) or 0)
    observed = int(manifest.get("unique_game_hour_count", 0) or 0)
    skipped_declared = int(manifest.get("skipped_game_hours", 0) or 0)

    gap_ledger = collections.Counter(
        gap.get("reason", "?") for gap in stream.get("gaps", []) or []
    )
    gap_outcomes = collections.Counter(
        outcome.get("reason", "?")
        for outcome in stream.get("domain_gap_outcomes", []) or []
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
        outcome.get("domain_id") for outcome in stream.get("domain_gap_outcomes", []) or []
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
    for gap in stream.get("gaps", []) or []:
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
    for outcome in stream.get("domain_gap_outcomes", []) or []:
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

    # The two gap ledgers describe the same stream and must agree wherever they
    # share a reason; a divergence means one of them is not counting something.
    shared = set(gap_ledger) & set(gap_outcomes)
    ledger_disagreements = {
        reason: (gap_ledger[reason], gap_outcomes[reason])
        for reason in sorted(shared)
        if gap_ledger[reason] != gap_outcomes[reason]
    }

    axes = [
        ("hours the clock advanced but no frame observed", hours_unobserved,
         f"{observed} observed of {span_hours} in span "
         f"(game_hours_advanced={advanced} is a delta, so the span is +1)"),
        ("...of those, hours no skipped-hour gap explains", hours_unaccounted,
         f"manifest declares skipped_game_hours={skipped_declared}, "
         f"ledger carries {gap_ledger.get('skipped_hour', 0)} skipped_hour "
         f"records covering {skipped_hours_covered} hours"),
        ("domain-hours with a gap instead of data", sum(gap_outcomes.values()),
         ", ".join(f"{r}={n}" for r, n in sorted(gap_outcomes.items())) or "none"),
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
        ("domains reporting dropped or incomplete events", len(dropped),
         "; ".join(f"{n} {json.dumps(p)}" for n, p in sorted(dropped.items())) or "none"),
        ("faults in the deliberately-not-scheduled record", len(record_faults),
         "; ".join(record_faults) or "none"),
        ("descheduled domains still in the expected roster", len(descheduled_but_expected),
         ", ".join(descheduled_but_expected) or "none"),
        ("descheduled domains that reported data", len(descheduled_but_reported),
         ", ".join(descheduled_but_reported) or "none"),
        ("refused frames", int(stream.get("frame_refused_record_count", 0) or 0), ""),
        ("degraded frames", int(stream.get("frame_degraded_record_count", 0) or 0), ""),
        ("records excluded from the seal", int(excluded.get("total_records", 0) or 0), ""),
        ("gap ledgers disagreeing on a shared reason", len(ledger_disagreements),
         json.dumps(ledger_disagreements) if ledger_disagreements else "none"),
        ("parse errors", len(manifest.get("parse_errors", []) or []), ""),
        ("ordering errors", len(manifest.get("ordering_errors", []) or []), ""),
        ("collector control errors",
         len(manifest.get("collector_control_errors", []) or []), ""),
        ("domain coverage errors",
         len((stream.get("domain_coverage", {}) or {}).get("errors", []) or []), ""),
        ("replay gate problems", len(artifact.get("gate_problems", []) or []), ""),
        ("hourly requirement problems", len(hourly.get("problems", []) or []), ""),
    ]

    # A recovery replay is a failed seal wearing a replay's name.
    if artifact.get("recovery"):
        axes.append(("replay is a recovery artifact, not a promotable one", 1, ""))
    # An hourly requirement of zero hours passes without asserting anything.
    if hourly.get("required") and not int(hourly.get("requested_game_hours", 0) or 0):
        axes.append(("hourly requirement passed with zero hours requested", 1,
                     "the check is vacuous as configured"))

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
