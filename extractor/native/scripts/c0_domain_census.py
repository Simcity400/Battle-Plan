#!/usr/bin/env python3
"""C0 domain closure census (PRODUCTION_ENDGAME_PLAN.md Wave C, blocks O8 wiring).

Machine-generated census over representative live manifests x the domain roster:
per domain — frames, data records, rows, every individual status bit's presence,
fold acceptance, problems. The output's UNEXPLAINED section is the work list:
every recurring nonzero failure-class bit must be attributed to a concrete source
+ policy before O8 dispositions are designed (an unattributed flag must never
become a typed-but-misunderstood disposition).

Manifest-level granularity. Per-hour spans / first-last occurrence need a stream
decode pass (replay_decoder) — planned as census pass 2; this pass finds WHICH
domain/bit pairs need it.

Usage: python c0_domain_census.py [manifest.json ...]
       (no args: uses the built-in representative set)
"""
import json
import os
import sys

RUNS = (r"C:\Users\Example\OneDrive\Documents\Paradox Interactive\Hearts of Iron IV"
        r"\mod\battle-plan\extractor\native\runs")

# Representative set: long production run (509h, post d47/d69 fixes), the o123
# validation run, and a deep-war serial-wall window (worst load observed).
DEFAULT_MANIFESTS = [
    os.path.join(RUNS, "production_speed5_ShortSpeed5_20260711_113625",
                 "native_session_manifest.json"),
    os.path.join(RUNS, "production_speed5_ShortSpeed5_20260711_142948",
                 "native_session_manifest.json"),
]

# compact_contract/src/lib.rs:132-155
STATUS_BITS = {
    0: "STATE_UNAVAILABLE",
    1: "PROFILE_MISMATCH",
    2: "NO_CLOCK",
    3: "NO_SCALAR_SOURCE",
    4: "WAR_BRIDGE_APPLIED",       # informational
    5: "EXTRACTION_PERF_GATE",     # informational
    6: "WORKER_PANIC",
    7: "BRIDGE_UNAVAILABLE",
    8: "SOURCE_UNAVAILABLE",
    9: "PARTIAL_DATA",
    10: "DELTA_DATA",              # informational (mode marker)
    11: "TRANSITION_BASELINE_RESET",
    12: "CONTROL_INVALID",
    13: "WAR_SEGMENT_ABSENT",      # informational (O7)
}
INFORMATIONAL = {4, 5, 10, 13}

# Attributed flag sources (design docs + live attribution work). A (domain, bit)
# pair listed here is EXPLAINED; everything else nonzero is the C0 work list.
# Values: (attribution, policy-of-record)
ATTRIBUTED = {
    ("market_trade_live", 8): (
        "d43 cold first sweep frame (elapsed-0 warmup; sweep needs ~250ms)",
        "O8 candidate: NOT ExpectedAbsent (dynamic) -> C2 warmup preflight or "
        "typed cold-start accounting; session-OR paints the whole run today"),
    ("characters_leaders_cabinet_live", 8): (
        "d47 cold-start frames before first complete sweep+prewarm",
        "same class as d43 cold; disposition must stay Failed/cold, never "
        "ExpectedAbsent"),
    ("characters_leaders_cabinet_live", 7): (
        "d47 early frames report bridge-unavailable pre-prewarm",
        "attribute precisely in pass 2 (which frames, how many)"),
    ("characters_leaders_cabinet_live", 9): (
        "d47 churn frame: dead root -> rowless PARTIAL (dead_roots ~1/509h "
        "post-dedup fix)",
        "measured, deferred PARTIAL-upsert (496e6e9a9); acceptable rate"),
    ("diplomacy_intel_operatives_live", 13): (
        "O7: war_relation_bridge absent by design in production (d53 carries "
        "wars natively)",
        "informational bit, never in FAILURE_MASK; d17 needs DerivedFrom(d53) "
        "proof or native capture per plan v2"),
    ("tech_sharing_groups", 9): (
        "HISTORIC (pre-ebdc5bc4b/1fbd9f3a4): vacant/tombstone members failed "
        "the group",
        "FIXED 2026-07-11 - post-fix runs must show bit 9 ZERO; nonzero here "
        "on a post-fix manifest = regression"),
}


def bits_of(v):
    return [b for b in STATUS_BITS if v & (1 << b)]


def census(paths):
    rows = {}          # (domain_name) -> aggregated dict
    sessions = []
    for p in paths:
        try:
            m = json.load(open(p))
        except Exception as e:
            print(f"SKIP {p}: {e}")
            continue
        cs = m.get("compact_stream", {})
        sid = str(m.get("session_id", os.path.basename(os.path.dirname(p))))
        sessions.append(sid)
        for name, d in (cs.get("per_domain", {}) or {}).items():
            r = rows.setdefault(name, {
                "domain_id": d.get("domain_id"),
                "sessions": 0, "frames": 0, "data_records": 0,
                "rows": 0, "bits": {}, "folded_all": True, "problems": set(),
            })
            r["sessions"] += 1
            r["frames"] += d.get("record_count", 0)
            r["data_records"] += d.get("data_record_count", 0)
            r["rows"] += d.get("row_count_total", 0)
            r["folded_all"] &= bool(d.get("world_folded", False))
            for b in bits_of(d.get("status_flags_or", 0)):
                r["bits"][b] = r["bits"].get(b, 0) + 1
            for pr in d.get("problems") or []:
                r["problems"].add(str(pr))
    return sessions, rows


def main():
    paths = sys.argv[1:] or DEFAULT_MANIFESTS
    sessions, rows = census(paths)
    print(f"C0 CENSUS over {len(sessions)} session(s): {', '.join(sessions)}")
    print(f"{len(rows)} domains\n")

    unexplained = []
    clean = []
    for name in sorted(rows, key=lambda n: rows[n]["domain_id"] or 0):
        r = rows[name]
        flag_bits = sorted(r["bits"])
        if not flag_bits and r["folded_all"] and not r["problems"]:
            clean.append(name)
            continue
        print(f"d{r['domain_id']:<3} {name}")
        print(f"     frames={r['frames']} data_records={r['data_records']} "
              f"rows={r['rows']} folded={r['folded_all']}")
        for b in flag_bits:
            tag = STATUS_BITS[b]
            info = " (informational)" if b in INFORMATIONAL else ""
            att = ATTRIBUTED.get((name, b))
            if att:
                print(f"     bit{b:>2} {tag}{info}: ATTRIBUTED - {att[0]}")
                print(f"           policy: {att[1]}")
            elif b in INFORMATIONAL:
                print(f"     bit{b:>2} {tag}{info}")
            else:
                print(f"     bit{b:>2} {tag}: *** UNEXPLAINED ***")
                unexplained.append((name, r["domain_id"], tag))
        for pr in sorted(r["problems"]):
            print(f"     problem: {pr}")
        print()

    print(f"CLEAN domains ({len(clean)}): {', '.join(clean)}\n")
    if unexplained:
        print("=== UNEXPLAINED failure-class bits (C0 work list; blocks O8) ===")
        for name, did, tag in unexplained:
            print(f"  d{did} {name}: {tag}")
        sys.exit(1)
    print("=== C0: zero unexplained failure-bit incidence on this set ===")


if __name__ == "__main__":
    main()
