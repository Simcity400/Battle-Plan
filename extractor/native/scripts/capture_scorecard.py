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
        ("expected domains with no per-domain record", len(absent),
         ", ".join(absent) or "none"),
        ("domains emitting gaps but absent from per-domain", len(absent_but_gapping),
         ", ".join(absent_but_gapping) or "none"),
        ("domains that produced no data record", len(zero_records),
         ", ".join(zero_records) or "none"),
        ("domains that produced no rows", len(zero_rows),
         ", ".join(zero_rows) or "none"),
        ("domains reporting dropped or incomplete events", len(dropped),
         "; ".join(f"{n} {json.dumps(p)}" for n, p in sorted(dropped.items())) or "none"),
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

    if args.json:
        json.dump({
            "run_dir": run_dir,
            "manifest": manifest_path,
            "complete": not misses,
            "axes": [{"axis": a, "count": n, "detail": d} for a, n, d in axes],
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
            print(f"  {'':{width}s}   {detail}")
    if misses:
        print(f"\nINCOMPLETE: {len(misses)} of {len(axes)} axes carry a miss\n")
        return 1
    print(f"\nCOMPLETE: all {len(axes)} axes clean\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
