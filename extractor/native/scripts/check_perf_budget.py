#!/usr/bin/env python3
"""Per-domain in-hour cost budget check over ONE named capture (lane LO).

    python scripts/check_perf_budget.py --run-dir <capture dir> [--budget perf_budget/domain_budget_v1.json]
                                        [--json-out <file>] [--tighten-out <file>]

Reads the capture's `native_session_manifest.json` (compact_stream.per_domain
read_time_us p50/p95 per invocation, lane_wall.crit_wall_us per frame, the d81
phase units per record) and compares each against `perf_budget/domain_budget_v1.json`.

Exit codes: 0 PASS (every measured value within budget), 2 FAIL (at least one
breach), 3 INDETERMINATE (the manifest lacks the fields, or fewer than
--min-invocations samples for a domain), 4 usage/IO error.

DETECTION, NOT ACCEPTANCE (Codex refutation 01a00925 s7): this gate keys on
per-invocation p50/p95 and per-frame critical path of an explicitly named run
whose identity (capture id, collector build, save/campaign, reader
concurrency) it prints beside the verdict; it does not pick "the latest run",
and it does not by itself say the game was unaffected - that is the paired
free-run vs recording measurement (runs/lo_zero_impact_plan/measure).
Budgets are a ratchet: --tighten-out writes a candidate table with every budget
lowered to max(measured * headroom, floor) so a landing can commit the new
floor with the run that measured it.
"""
import argparse
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
P50_FLOOR_MS = 2.0
P95_FLOOR_MS = 4.0
DEFAULT_BUDGET = os.path.normpath(os.path.join(HERE, "..", "perf_budget", "domain_budget_v1.json"))


def load(path):
    with open(path, "r", encoding="utf-8") as handle:
        return json.load(handle)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--run-dir", required=True)
    ap.add_argument("--budget", default=DEFAULT_BUDGET)
    ap.add_argument("--json-out", default=None)
    ap.add_argument("--tighten-out", default=None)
    ap.add_argument("--min-invocations", type=int, default=20)
    args = ap.parse_args()

    manifest_path = os.path.join(args.run_dir, "native_session_manifest.json")
    if not os.path.isfile(manifest_path):
        print(f"INDETERMINATE: no manifest at {manifest_path}")
        return 3
    manifest = load(manifest_path)
    budget = load(args.budget)
    stream = manifest.get("compact_stream", {}) or {}
    per_domain = stream.get("per_domain", {}) or {}
    lane_wall = stream.get("lane_wall", {}) or {}
    identity = {
        "run_dir": os.path.abspath(args.run_dir),
        "capture_id": manifest.get("capture_id"),
        "session_id": manifest.get("session_id"),
        "collector_build": manifest.get("collector_build"),
        "campaign_identity": manifest.get("campaign_identity"),
        "reader_concurrency": manifest.get("reader_concurrency"),
        "duration_ms": manifest.get("duration_ms"),
        "game_hours_advanced": manifest.get("game_hours_advanced"),
        "frames": lane_wall.get("frames"),
        "budget_schema": budget.get("schema"),
        "budget_baseline_run": budget.get("baseline_run"),
    }
    headroom = float(budget.get("headroom_over_baseline", 1.3))
    rows = []
    breaches = []
    indeterminate = []

    def check(name, measured, limit, note=""):
        if measured is None or limit is None:
            indeterminate.append(name)
            rows.append((name, measured, limit, "n/a"))
            return
        verdict = "ok" if measured <= limit else "BREACH"
        if verdict == "BREACH":
            breaches.append((name, measured, limit))
        rows.append((name, measured, limit, verdict + (f" {note}" if note else "")))

    # per-domain per-invocation p50 / p95
    tighten = json.loads(json.dumps(budget))
    sum_p50 = 0.0
    for name, spec in sorted(budget.get("domains", {}).items(), key=lambda kv: kv[1]["domain_id"]):
        info = per_domain.get(name)
        rt = (info or {}).get("read_time_us") or {}
        count = int(rt.get("count", 0) or 0)
        if count < args.min_invocations:
            indeterminate.append(f"{name}: {count} invocations")
            rows.append((f"d{spec['domain_id']:<3} {name} p50", None, spec["read_time_p50_ms_max"], f"n/a ({count} invocations)"))
            continue
        # A missing percentile is INDETERMINATE, never substituted (Codex
        # review 01a0096c, MAJOR 5: an absent p95 must not pass every budget).
        if rt.get("p50") is None or rt.get("p95") is None:
            indeterminate.append(f"{name}: read_time_us lacks p50/p95")
            rows.append((f"d{spec['domain_id']:<3} {name} p50", None, spec["read_time_p50_ms_max"], "n/a (no percentiles)"))
            continue
        p50 = rt["p50"] / 1000.0
        p95 = rt["p95"] / 1000.0
        sum_p50 += p50
        check(f"d{spec['domain_id']:<3} {name} p50", round(p50, 3), max(spec["read_time_p50_ms_max"], P50_FLOOR_MS))
        check(f"d{spec['domain_id']:<3} {name} p95", round(p95, 3), max(spec["read_time_p95_ms_max"], P95_FLOOR_MS))
        t = tighten["domains"][name]
        # Floors: P50_FLOOR_MS / P95_FLOOR_MS. Below that a domain's percentile is
        # scheduler noise on a shared box (t71: 1.61 vs 1.38 ms p95 breaches),
        # not a cost signal.
        t["read_time_p50_ms_max"] = round(min(t["read_time_p50_ms_max"], max(p50 * headroom, P50_FLOOR_MS)), 2)
        t["read_time_p95_ms_max"] = round(min(t["read_time_p95_ms_max"], max(p95 * headroom, P95_FLOOR_MS)), 2)
    g = budget.get("global", {})
    check("global sum of domain p50 (ms)", round(sum_p50, 1), g.get("sum_domain_read_time_p50_ms_max"))
    crit = lane_wall.get("crit_wall_us") or {}
    crit_p50 = crit.get("p50")
    crit_p95 = crit.get("p95")
    check("global crit_wall p50 (ms)", (crit_p50 / 1000.0) if crit_p50 else None, g.get("crit_wall_p50_ms_max"))
    check("global crit_wall p95 (ms)", (crit_p95 / 1000.0) if crit_p95 else None, g.get("crit_wall_p95_ms_max"))
    tg = tighten["global"]
    tg["sum_domain_read_time_p50_ms_max"] = round(min(tg["sum_domain_read_time_p50_ms_max"], sum_p50 * headroom), 1)
    if crit_p50:
        tg["crit_wall_p50_ms_max"] = round(min(tg["crit_wall_p50_ms_max"], crit_p50 / 1000.0 * headroom), 1)
    if crit_p95:
        tg["crit_wall_p95_ms_max"] = round(min(tg["crit_wall_p95_ms_max"], crit_p95 / 1000.0 * headroom), 1)

    # per-record phase units
    for name, units in (budget.get("phase_units_per_record") or {}).items():
        info = per_domain.get(name) or {}
        recs = int(info.get("data_record_count", 0) or 0)
        totals = info.get("data_units_total") or {}
        for unit, spec in units.items():
            if recs < args.min_invocations or unit not in totals:
                indeterminate.append(f"{name}.{unit}")
                rows.append((f"{name}.{unit} per record", None, spec["ms_max"], f"n/a ({recs} records)"))
                continue
            per_record_ms = totals[unit] / recs / 1000.0
            check(f"{name}.{unit} per record (ms)", round(per_record_ms, 3), spec["ms_max"])
            tighten["phase_units_per_record"][name][unit]["ms_max"] = round(
                min(spec["ms_max"], per_record_ms * headroom), 2)

    # torn / backpressure accounting from the gap ledger, reported (not budgeted here)
    gap = stream.get("gap_ledger") or {}
    accounting = {"gap_ledger_keys": sorted(gap.keys())[:20] if isinstance(gap, dict) else None,
                  "frame_refused_record_count": stream.get("frame_refused_record_count"),
                  "frame_degraded_record_count": stream.get("frame_degraded_record_count")}

    print("PERF BUDGET CHECK  (detection gate; budgets are a ratchet)")
    for key, value in identity.items():
        print(f"  {key:22} {value}")
    print()
    print(f"  {'metric':60} {'measured':>10} {'budget':>10}  verdict")
    for name, measured, limit, verdict in rows:
        m = "-" if measured is None else f"{measured:.3f}"
        l = "-" if limit is None else f"{limit:.3f}"
        print(f"  {name:60} {m:>10} {l:>10}  {verdict}")
    print()
    print(f"  accounting: {json.dumps(accounting)}")
    if breaches:
        verdict = "FAIL"
        code = 2
        print(f"VERDICT: FAIL - {len(breaches)} breach(es):")
        for name, measured, limit in breaches:
            print(f"    {name}: {measured} > {limit}")
    elif indeterminate:
        verdict = "INDETERMINATE"
        code = 3
        print(f"VERDICT: INDETERMINATE - {len(indeterminate)} metric(s) unmeasurable: {indeterminate[:8]}")
    else:
        verdict = "PASS"
        code = 0
        print("VERDICT: PASS - every measured metric within budget")
    if args.json_out:
        with open(args.json_out, "w", encoding="utf-8") as handle:
            json.dump({"verdict": verdict, "identity": identity, "rows": rows, "breaches": breaches,
                       "indeterminate": indeterminate, "accounting": accounting}, handle, indent=1)
    if args.tighten_out and verdict != "PASS":
        print("tightened table NOT written: the ratchet only advances on a PASS")
    elif args.tighten_out:
        tighten["baseline_run"] = identity["run_dir"]
        with open(args.tighten_out, "w", encoding="utf-8") as handle:
            json.dump(tighten, handle, indent=1)
        print(f"tightened candidate table written to {args.tighten_out}")
    return code


def self_test():
    """kills: a missing percentile being substituted (must be INDETERMINATE),
    a breach not failing, and the ratchet advancing on anything but a PASS."""
    import tempfile
    budget = {"schema": "H4_PERF_BUDGET_V1", "headroom_over_baseline": 1.3,
              "global": {"sum_domain_read_time_p50_ms_max": 100.0, "crit_wall_p50_ms_max": 50.0, "crit_wall_p95_ms_max": 80.0},
              "phase_units_per_record": {},
              "domains": {"alpha": {"domain_id": 1, "read_time_p50_ms_max": 10.0, "read_time_p95_ms_max": 20.0}}}
    def run(rt, crit):
        with tempfile.TemporaryDirectory() as tmp:
            os.makedirs(os.path.join(tmp, "run"))
            manifest = {"compact_stream": {"per_domain": {"alpha": {"domain_id": 1, "read_time_us": rt}},
                                            "lane_wall": {"crit_wall_us": crit, "frames": 10}}}
            json.dump(manifest, open(os.path.join(tmp, "run", "native_session_manifest.json"), "w"))
            json.dump(budget, open(os.path.join(tmp, "budget.json"), "w"))
            tighten = os.path.join(tmp, "tight.json")
            argv = sys.argv
            sys.argv = ["x", "--run-dir", os.path.join(tmp, "run"), "--budget", os.path.join(tmp, "budget.json"),
                        "--min-invocations", "1", "--tighten-out", tighten]
            try:
                code = main()
            finally:
                sys.argv = argv
            return code, os.path.exists(tighten)
    assert run({"count": 5, "p50": 5000, "p95": 9000, "avg": 5000}, {"p50": 40000, "p95": 70000}) == (0, True)
    assert run({"count": 5, "p50": 5000, "avg": 5000}, {"p50": 40000, "p95": 70000}) == (3, False), "missing p95 must be INDETERMINATE"
    assert run({"count": 5, "p50": 15000, "p95": 9000, "avg": 15000}, {"p50": 40000, "p95": 70000}) == (2, False), "breach must FAIL and not ratchet"
    assert run({"count": 5, "p95": 9000, "avg": 5000}, {"p50": 40000, "p95": 70000}) == (3, False), "missing p50 must be INDETERMINATE"
    # The floors: a domain budgeted below them is judged at the floor.
    budget["domains"]["alpha"]["read_time_p50_ms_max"] = 0.5
    budget["domains"]["alpha"]["read_time_p95_ms_max"] = 1.0
    assert run({"count": 5, "p50": 1500, "p95": 3500, "avg": 1500}, {"p50": 40000, "p95": 70000})[0] == 0, "values under the floors pass"
    assert run({"count": 5, "p50": 2500, "p95": 3500, "avg": 2500}, {"p50": 40000, "p95": 70000})[0] == 2, "values over the floors breach"
    budget["domains"]["alpha"]["read_time_p50_ms_max"] = 10.0
    budget["domains"]["alpha"]["read_time_p95_ms_max"] = 20.0
    print("check_perf_budget self-test: ok")


if __name__ == "__main__":
    if len(sys.argv) == 2 and sys.argv[1] == "--self-test":
        self_test()
        sys.exit(0)
    try:
        sys.exit(main())
    except (OSError, ValueError, KeyError) as error:
        print(f"ERROR: {error}")
        sys.exit(4)
