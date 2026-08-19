#!/usr/bin/env python3
"""Read a recorder run directory directly and print what actually happened.

No decoding library, no replay app, no debug profile: this opens the raw
artifacts the recorder leaves on disk (`native_session_manifest.json`, the
console logs, and the raw H4CR stream) and reports the exact per-domain
extraction status, so "which data did NOT come out" is answerable without
re-running anything.

Usage:
    python report_raw_capture.py --run-dir <dir> [--json] [--all-domains]
    python report_raw_capture.py --selftest

Exit codes:
    0  clean
    1  blocking domains, or the hourly requirement was not met
    2  a domain published a status bit no compact-stream contract defines
    3  this report reached NO verdict (it crashed, or could not read the run)
"""
from __future__ import annotations

import argparse
import json
import os
import sys

STATUS_BITS = {
    1 << 0: "state_unavailable",
    1 << 1: "profile_mismatch",
    1 << 2: "no_clock",
    1 << 3: "no_scalar_source",
    1 << 4: "war_bridge_applied",
    1 << 5: "extraction_perf_gate",
    1 << 6: "worker_panic",
    1 << 7: "bridge_unavailable",
    1 << 8: "source_unavailable",
    1 << 9: "partial_data",
    1 << 10: "delta_data",
    1 << 11: "transition_baseline_reset",
    1 << 12: "control_invalid",
    1 << 13: "war_segment_absent",
    1 << 14: "source_empty",
    1 << 15: "baseline_clear",
}
KNOWN_MASK = (1 << 16) - 1
FAILURE_MASK = (
    (1 << 0) | (1 << 1) | (1 << 2) | (1 << 3) | (1 << 6)
    | (1 << 7) | (1 << 8) | (1 << 9) | (1 << 12)
)
LANE_LOCAL_BITS = {
    49: {11: "d49:location_candidate_unvalidated"},
    51: {
        11: "d51:flag_names_partial",
        12: "d51:delayed_events_root_unresolved",
        13: "d51:scripted_gui_root_unresolved",
    },
    53: {
        16: "d53:operations_detail_resisted",
        17: "d53:intel_map_resisted",
        18: "d53:war_root_unvalidated",
        19: "d53:incoming_action_queue_unvalidated",
        20: "d53:war_lifecycle_detail_unvalidated",
        21: "d53:faction_group_unvalidated",
        22: "d53:intel_network_graph_unvalidated",
    },
    60: {
        16: "d60:war_casualties_running_accumulator",
        17: "d60:war_residuals_exact",
    },
    99: {
        16: "d99:torn_rows_present",
        17: "d99:stat_block_null_present",
        18: "d99:incomplete_observation",
    },
}
LANE_LOCAL_MASK = {
    domain: sum(1 << bit for bit in bits)
    for domain, bits in LANE_LOCAL_BITS.items()
}


# The set is DERIVED, not listed: `domain_registry.json` is the generated mirror
DOMAIN_REGISTRY_PATH = os.path.normpath(
    os.path.join(
        os.path.dirname(os.path.abspath(__file__)), os.pardir, "domain_registry.json"
    )
)
PSEUDO_DOMAIN_NAMES_FALLBACK = ("clock", "frame")
INFRASTRUCTURE_STATUS_KEY = "infrastructure"


def load_pseudo_domain_names(path: str = DOMAIN_REGISTRY_PATH) -> tuple[set, str]:
    """The wire names of every non-scheduled infrastructure producer.

    Returns the set and a description of WHERE it came from, because "the
    report used its fallback" and "the report read the registry" are different
    claims and a reader is entitled to know which one it is looking at.

    Every shape check below fails to the fallback rather than raising. A
    registry this function cannot read is a reason to fall back to a set that
    something else vouches for; it is never a reason to take a report that is
    otherwise perfectly readable down to EXIT_REPORT_ERROR, which means "this
    report reached NO verdict".
    """
    try:
        with open(path, encoding="utf-8") as handle:
            registry = json.load(handle)
    except (OSError, ValueError) as error:
        return (
            set(PSEUDO_DOMAIN_NAMES_FALLBACK),
            f"pinned fallback ({type(error).__name__}: {error})",
        )
    domains = registry.get("domains") if isinstance(registry, dict) else None
    if not isinstance(domains, list):
        return (
            set(PSEUDO_DOMAIN_NAMES_FALLBACK),
            "pinned fallback (domain_registry.json carries no `domains` list)",
        )
    derived = {
        entry.get("name")
        for entry in domains
        if isinstance(entry, dict)
        and entry.get("status") == INFRASTRUCTURE_STATUS_KEY
        and isinstance(entry.get("name"), str)
    }
    if not derived:
        return (
            set(PSEUDO_DOMAIN_NAMES_FALLBACK),
            "pinned fallback (registry declared no infrastructure domain)",
        )
    pinned = set(PSEUDO_DOMAIN_NAMES_FALLBACK)
    if derived == pinned:
        return derived, path
    detail = []
    missing = sorted(pinned - derived)
    extra = sorted(derived - pinned)
    if missing:
        detail.append(f"registry omits {missing}")
    if extra:
        detail.append(f"registry adds {extra}")
    return derived | pinned, f"{path} UNION pinned fallback ({'; '.join(detail)})"


REASON_SUMMARY_MARKER = "| reason="
REASON_SUMMARIES_SHOWN = 4
REASON_SAMPLE_CHARS = 320


def reason_summary(reason: str) -> str:
    """The producer's own summary clause, or the whole string if it has none.

    The wire reason is `<diagnostic units> | reason=<summary>`. The units are
    per-frame counters, so they make almost every full string unique; the
    summary clause is the stable sentence. A reason with no marker is returned
    whole -- inventing a split that the producer did not write would be a
    fabricated summary.
    """
    index = reason.rfind(REASON_SUMMARY_MARKER)
    if index < 0:
        return reason
    return reason[index + len(REASON_SUMMARY_MARKER):].strip()


def predicate_label(item: dict) -> str:
    """The stable grouping key for one degraded domain entry.

    The summary clause above is a PROSE key: it is the best split this report
    could make when prose was all the wire carried. `H4FD` version 2 publishes
    the producer's own predicate as a code plus its name, which is the key the
    producer actually decided by -- one code per check, no per-frame values in
    it -- so it groups causes where the summary groups sentences that happen to
    match. The two are printed side by side rather than one replacing the
    other: the predicate says WHICH CHECK failed, the summary says what the
    producer wrote about that particular failure, and a lane chasing an empty
    domain wants both.

    Every entry gets a label, including the ones with no predicate, so the
    printed counts sum to the domain's stored occurrence count. Version 2 always
    writes a predicate, so a large `(none on the wire)` count is a fact about a
    BROKEN capture rather than an older one -- this function has no
    pre-version-2 read path and old captures are disposable. Showing it beats
    dropping those rows out of a total the reader is about to trust.

    The code is rendered beside the name because the name is per-domain, not
    globally unique (the shared contract defines a predicate as the pair), so
    two domains' identically named checks are still two different predicates.
    """
    if "predicate_code" not in item and "predicate_name" not in item:
        return "(none on the wire)"
    code = item.get("predicate_code")
    name = item.get("predicate_name")
    if isinstance(code, bool) or not isinstance(code, int):
        return "(unreadable predicate code)"
    if not isinstance(name, str) or not name.strip():
        return f"(unnamed predicate) [0x{code:08x}]"
    return f"{name} [0x{code:08x}]"


def collect_outcome_reasons(stream: dict) -> tuple[dict, int]:
    """`domain_id -> ` what the wire said about that domain's degraded hours.

    Also returns how many outcome entries carried a domain id or a reason this
    function could not read. Those are COUNTED rather than dropped: silently
    losing a reason is the defect this whole function exists to fix, and raising
    would take a report that is otherwise perfectly readable down to
    EXIT_REPORT_ERROR over one malformed entry.

    The reason is stored EXACTLY as the wire wrote it. Nothing here trims,
    normalises or rewrites it: the stored string is what the "sample full
    reason" line prints and what the distinct-string count counts, so a
    normalising store would report a count and a sample for a string the
    producer never published.
    """
    collected: dict = {}
    unreadable = 0
    for key, kind in (
        ("frame_degraded_outcomes", "degraded"),
        ("frame_refused_outcomes", "refused"),
    ):
        outcomes = stream.get(key)
        for entry in outcomes if isinstance(outcomes, list) else []:
            if not isinstance(entry, dict):
                continue
            hour = entry.get("hour_raw")
            claim = entry.get("coverage_claim")
            claim_key = None if claim is None else (
                claim if isinstance(claim, str) else str(claim)
            )
            items = entry.get("domains")
            for item in items if isinstance(items, list) else []:
                if not isinstance(item, dict):
                    continue
                domain_id = item.get("domain_id")
                reason = item.get("reason")
                if domain_id is None and reason is None:
                    continue
                if not isinstance(reason, str):
                    unreadable += 1
                    continue
                if not reason.strip():
                    continue
                if isinstance(domain_id, bool) or not isinstance(domain_id, int):
                    try:
                        domain_id = int(str(domain_id), 10)
                    except (TypeError, ValueError):
                        unreadable += 1
                        continue
                slot = collected.setdefault(
                    domain_id,
                    {
                        "outcome_counts": {},
                        "coverage_claims": {},
                        "summaries": {},
                        "predicates": {},
                        "distinct_full_reasons": set(),
                        "sample_reason": reason,
                        "first_hour": None,
                        "last_hour": None,
                        "occurrences": 0,
                    },
                )
                slot["outcome_counts"][kind] = slot["outcome_counts"].get(kind, 0) + 1
                if claim_key is not None:
                    slot["coverage_claims"][claim_key] = (
                        slot["coverage_claims"].get(claim_key, 0) + 1
                    )
                summary = reason_summary(reason)
                slot["summaries"][summary] = slot["summaries"].get(summary, 0) + 1
                slot["distinct_full_reasons"].add(reason)
                predicate = predicate_label(item)
                slot["predicates"][predicate] = slot["predicates"].get(predicate, 0) + 1
                slot["occurrences"] += 1
                if isinstance(hour, int) and not isinstance(hour, bool):
                    if slot["first_hour"] is None or hour < slot["first_hour"]:
                        slot["first_hour"] = hour
                    if slot["last_hour"] is None or hour > slot["last_hour"]:
                        slot["last_hour"] = hour
    for slot in collected.values():
        slot["distinct_full_reason_count"] = len(slot.pop("distinct_full_reasons"))
        slot["summaries"] = dict(
            sorted(slot["summaries"].items(), key=lambda kv: (-kv[1], kv[0]))
        )
        slot["predicates"] = dict(
            sorted(slot["predicates"].items(), key=lambda kv: (-kv[1], kv[0]))
        )
    return collected, unreadable


def print_outcome_reasons(reasons: dict, indent: str) -> None:
    """Print one domain's wire-published reasons. Never a summary of our own."""
    counts = ", ".join(
        f"{count} {kind}" for kind, count in sorted(reasons["outcome_counts"].items())
    )
    if reasons["first_hour"] is None:
        hours = "no readable hour"
    else:
        hours = f"hours {reasons['first_hour']}-{reasons['last_hour']}"
    print(f"{indent}{'reasons:':10s} {counts} frame outcome(s), {hours}")
    claims = ", ".join(
        f"{claim} x{count}" for claim, count in sorted(reasons["coverage_claims"].items())
    )
    if claims:
        print(f"{indent}  coverage claim: {claims}")
    predicates = list(reasons["predicates"].items())
    for predicate, count in predicates[:REASON_SUMMARIES_SHOWN]:
        print(f"{indent}  x{count} predicate {predicate}")
    if len(predicates) > REASON_SUMMARIES_SHOWN:
        print(f"{indent}  (+{len(predicates) - REASON_SUMMARIES_SHOWN} more distinct "
              f"predicate(s))")
    summaries = list(reasons["summaries"].items())
    for summary, count in summaries[:REASON_SUMMARIES_SHOWN]:
        print(f"{indent}  x{count} {summary}")
    if len(summaries) > REASON_SUMMARIES_SHOWN:
        print(f"{indent}  (+{len(summaries) - REASON_SUMMARIES_SHOWN} more distinct "
              f"summary reason(s))")
    sample = reasons["sample_reason"]
    clipped = sample[:REASON_SAMPLE_CHARS]
    suffix = "" if len(sample) == len(clipped) else f" ...[clipped, {len(sample)} chars]"
    print(f"{indent}  sample full reason "
          f"(1 of {reasons['distinct_full_reason_count']} distinct): {clipped}{suffix}")


def globalize(domain_id: int, flags: int) -> int:
    """The word as the GLOBAL vocabulary sees it (contract
    `compact_status_for_global_interpretation`)."""
    return flags & ~LANE_LOCAL_MASK.get(domain_id, 0)


def undefined_bits(domain_id: int, flags: int) -> int:
    """Status bits this capture published that NOTHING defines.

    A registered lane-local badge is defined, so it is stripped first; what
    survives is a producer writing into the status word without a contract
    entry, which no consumer can classify. Deliberately computed from the raw
    word rather than reported away: the strip is the contract's own, and only
    the contract's own.
    """
    return globalize(domain_id, flags) & ~KNOWN_MASK


def decode(flags: int, domain_id: int = -1) -> list[str]:
    lane_bits = LANE_LOCAL_BITS.get(domain_id, {})
    lane_mask = LANE_LOCAL_MASK.get(domain_id, 0)
    names = [
        lane_bits[bit] for bit in sorted(lane_bits) if flags & (1 << bit)
    ]
    globalized = flags & ~lane_mask
    names += [
        name for bit, name in sorted(STATUS_BITS.items()) if globalized & bit
    ]
    undefined = globalized & ~KNOWN_MASK
    if undefined:
        names.append(f"UNDEFINED_BITS(0x{undefined:x})")
    return names


EXIT_UNDEFINED_STATUS_BITS = 2
EXIT_REPORT_ERROR = 3

VERDICT_RECEIPT_PREFIX = "COMPACT_STATUS_VERDICT="


def emit_verdict_receipt(code: int) -> int:
    print(f"{VERDICT_RECEIPT_PREFIX}{code}", file=sys.stderr)
    return code


def verdict_exit_code(undefined: list, blocking: list, hourly: dict) -> int:
    if undefined:
        return EXIT_UNDEFINED_STATUS_BITS
    if blocking or hourly.get("met") is False:
        return 1
    return 0


class ReportError(Exception):
    """This report cannot reach a verdict. Never a statement about the capture."""


def load_manifest(run_dir: str) -> tuple[dict, str]:
    for name in ("native_session_manifest.json", "recorder/native_session_manifest.json"):
        path = os.path.join(run_dir, name)
        if os.path.isfile(path):
            with open(path, encoding="utf-8") as handle:
                return json.load(handle), path
    raise ReportError(f"no native_session_manifest.json under {run_dir}")


def selftest() -> int:
    """Prove the undefined-status-bit verdict through the REAL entry path.

    Every case runs `main()` over a synthetic run directory, so what is pinned
    is the exit code and the printed verdict a caller actually blocks on -- not
    a private helper that could keep agreeing with itself while the gate stops
    working.
    """
    import contextlib
    import io
    import tempfile

    def build(domains, stream_extra=None):
        stream = {
            "expected_domains": list(domains),
            "domains_seen": list(domains),
            "frame_end_record_count": 1,
            "per_domain": {
                name: {
                    "domain_id": domain_id,
                    "status_flags_or": flags,
                    "data_record_count": 1,
                    "row_count_total": 1,
                    "world_folded": True,
                    "problems": {},
                }
                for name, (domain_id, flags) in domains.items()
            },
        }
        stream.update(stream_extra or {})
        return {
            "session_id": "selftest",
            "compact_stream": stream,
            "hourly_requirement": {"met": True, "problems": []},
            "replay_artifact": {
                "written_file": "x", "recovery": False, "gate_problems": [],
            },
        }

    def run(domains, raw_manifest=None, write_manifest=True, stream_extra=None):
        with tempfile.TemporaryDirectory() as run_dir:
            if write_manifest:
                path = os.path.join(run_dir, "native_session_manifest.json")
                with open(path, "w", encoding="utf-8") as handle:
                    if raw_manifest is None:
                        json.dump(build(domains, stream_extra), handle)
                    else:
                        handle.write(raw_manifest)
            argv = sys.argv
            sys.argv = [argv[0], "--run-dir", run_dir]
            buffer = io.StringIO()
            try:
                with contextlib.redirect_stdout(buffer):
                    with contextlib.redirect_stderr(buffer):
                        code = guarded_main()
            finally:
                sys.argv = argv
            return code, buffer.getvalue()

    cases = (
        ("a clean word is clean",
         {"fuel_live": (10, 0)}, 0, "VERDICT: clean", "UNDEFINED"),
        ("d53's registered badges are not undefined",
         {"diplomacy_intel_operatives_live": (53, 0x530000)}, 0,
         "VERDICT: clean", "UNDEFINED"),
        ("d60's registered badges are not undefined",
         {"capitulation_war_state_live": (60, 0x30000)}, 0,
         "VERDICT: clean", "UNDEFINED"),
        ("an ordinary failure status stays exit 1, not the undefined class",
         {"fuel_live": (10, 1 << 9)}, 1, "VERDICT: BLOCKED",
         "UNDEFINED STATUS BITS"),
        ("an unassigned bit on a domain with no lane vocabulary is fatal",
         {"fuel_live": (10, 1 << KNOWN_MASK.bit_length())},
         EXIT_UNDEFINED_STATUS_BITS,
         "CONTRACT VIOLATION: UNDEFINED STATUS BITS", None),
        ("an unregistered high bit on d53 is fatal despite d53's badges",
         {"diplomacy_intel_operatives_live": (53, 0x530000 | (1 << 23))},
         EXIT_UNDEFINED_STATUS_BITS, "0x800000", None),
        ("d99's bit 18 is fatal when d60 publishes it",
         {"capitulation_war_state_live": (60, 0x30000 | (1 << 18))},
         EXIT_UNDEFINED_STATUS_BITS, "0x40000", None),
        ("the verdict names the offending domain",
         {"fuel_live": (10, 1 << 31)}, EXIT_UNDEFINED_STATUS_BITS,
         "fuel_live (domain 10) published status bits", None),
    )

    error_cases = (
        ("a malformed manifest reaches no verdict, it is not a blocking one",
         "{not json", True),
        ("an empty manifest file reaches no verdict", "", True),
        ("a manifest of the wrong shape reaches no verdict", "[]", True),
        ("a missing manifest reaches no verdict", None, False),
    )

    def has_receipt(out, code):
        """The receipt must be present and must agree with the exit code.

        Checked on EVERY case, not in a case of its own: the receipt exists so
        a caller can tell "this report said 1" from "something exited 1", and a
        verdict path that forgot to emit it is indistinguishable from a python
        that died before this file ran.
        """
        return f"{VERDICT_RECEIPT_PREFIX}{code}" in out

    failures = 0
    for label, domains, want_code, want_text, forbidden in cases:
        code, out = run(domains)
        ok = code == want_code and want_text in out and has_receipt(out, code)
        if forbidden is not None and forbidden in out:
            ok = False
        print(f"selftest {'ok  ' if ok else 'FAIL'} exit={code} "
              f"(want {want_code}): {label}")
        if not ok:
            failures += 1
            print(out)
    for label, raw, write in error_cases:
        code, out = run({}, raw_manifest=raw, write_manifest=write)
        ok = (code == EXIT_REPORT_ERROR and "REPORT ERROR" in out
              and has_receipt(out, code))
        print(f"selftest {'ok  ' if ok else 'FAIL'} exit={code} "
              f"(want {EXIT_REPORT_ERROR}): {label}")
        if not ok:
            failures += 1
            print(out)

    roster_cases = (
        ("the frame pseudo-domain is not an unexpected domain",
         {"frame": (1000, 0)}, {"fuel_live": (10, 0)},
         "infrastructure pseudo-domains      frame", "unexpected domains in stream"),
        ("the clock pseudo-domain is not an unexpected domain either",
         {"clock": (1, 0)}, {"fuel_live": (10, 0)},
         "infrastructure pseudo-domains      clock", "unexpected domains in stream"),
        ("a schedulable domain seen but not expected is still unexpected",
         {"production_live": (11, 0)}, {"fuel_live": (10, 0)},
         "unexpected domains in stream (1)", None),
        ("a pseudo-domain does not hide a real unexpected domain beside it",
         {"frame": (1000, 0), "production_live": (11, 0)}, {"fuel_live": (10, 0)},
         "unexpected domains in stream (1)", None),
    )
    for label, seen_only, scheduled, want_text, forbidden in roster_cases:
        domains = dict(scheduled)
        domains.update(seen_only)
        code, out = run(
            domains,
            stream_extra={"expected_domains": sorted(scheduled),
                          "domains_seen": sorted(domains)},
        )
        ok = want_text in out and has_receipt(out, code)
        if forbidden is not None and forbidden in out:
            ok = False
        print(f"selftest {'ok  ' if ok else 'FAIL'} exit={code}: {label}")
        if not ok:
            failures += 1
            print(out)

    names, source = load_pseudo_domain_names()
    derived_ok = (
        source == DOMAIN_REGISTRY_PATH
        and names == set(PSEUDO_DOMAIN_NAMES_FALLBACK)
    )
    print(f"selftest {'ok  ' if derived_ok else 'FAIL'}: the pseudo-domain set is "
          f"read from domain_registry.json ({sorted(names)} via {source})")
    if not derived_ok:
        failures += 1
    registry_cases = (
        ("a registry entry the contract does not have is still derived",
         '{"domains": [{"name": "sentinel_infra", "status": "infrastructure"},'
         ' {"name": "clock", "status": "infrastructure"},'
         ' {"name": "frame", "status": "infrastructure"}]}',
         lambda names, source: "sentinel_infra" in names and "UNION" in source),
        ("a truncated registry keeps the pinned names and says so",
         '{"domains": [{"name": "clock", "status": "infrastructure"}]}',
         lambda names, source: names == {"clock", "frame"}
         and "registry omits ['frame']" in source),
        ("a schedulable-only registry falls back",
         '{"domains": [{"name": "fuel_live", "status": "existing"}]}',
         lambda names, source: names == set(PSEUDO_DOMAIN_NAMES_FALLBACK)
         and source.startswith("pinned fallback")),
        ("a registry whose domains is a mapping falls back, it does not raise",
         '{"domains": {"clock": {"status": "infrastructure"}}}',
         lambda names, source: names == set(PSEUDO_DOMAIN_NAMES_FALLBACK)
         and source.startswith("pinned fallback")),
        ("a registry of the wrong top-level shape falls back",
         '[]',
         lambda names, source: source.startswith("pinned fallback")),
        ("a malformed registry falls back",
         '{not json',
         lambda names, source: source.startswith("pinned fallback")),
        ("an absent registry falls back and says so",
         None,
         lambda names, source: names == set(PSEUDO_DOMAIN_NAMES_FALLBACK)
         and source.startswith("pinned fallback")),
    )
    for label, text, predicate in registry_cases:
        with tempfile.TemporaryDirectory() as registry_dir:
            registry_path = os.path.join(registry_dir, "domain_registry.json")
            if text is not None:
                with open(registry_path, "w", encoding="utf-8") as handle:
                    handle.write(text)
            names, source = load_pseudo_domain_names(registry_path)
            ok = predicate(names, source)
        print(f"selftest {'ok  ' if ok else 'FAIL'}: {label} "
              f"({sorted(names)} via {source})")
        if not ok:
            failures += 1

    reason_stream = {
        "frame_degraded_outcomes": [
            {
                "hour_raw": 1234,
                "coverage_claim": "published_frame_annotated_unknown",
                "domains": [{
                    "domain_id": 10,
                    "status_flags": 1 << 8,
                    "non_tolerated_flags": 1 << 8,
                    "reason": "domain 10: probe=1 | reason=SELFTEST_REASON_SENTINEL",
                }],
            },
        ],
    }
    reason_cases = (
        ("a blocking domain's wire reason is printed",
         {"fuel_live": (10, 1 << 8)}, reason_stream,
         "SELFTEST_REASON_SENTINEL", None),
        ("a non-blocking domain's wire reason is printed, not dropped",
         {"fuel_live": (10, 0)}, reason_stream,
         "degraded/refused outcome reasons on non-blocking domains",
         None),
        ("a reason with no summary clause is shown whole",
         {"fuel_live": (10, 1 << 8)},
         {"frame_degraded_outcomes": [{
             "hour_raw": 7, "coverage_claim": "c",
             "domains": [{"domain_id": 10, "reason": "BARE_SENTINEL no marker"}],
         }]},
         "x1 BARE_SENTINEL no marker", None),
        ("a reason for a domain with no per_domain row still surfaces",
         {"fuel_live": (10, 0)},
         {"frame_degraded_outcomes": [{
             "hour_raw": 7, "coverage_claim": "c",
             "domains": [{"domain_id": 999, "reason": "ORPHAN_SENTINEL"}],
         }]},
         "ORPHAN_SENTINEL", None),
        ("no reason section when the wire carried no reasons",
         {"fuel_live": (10, 1 << 8)}, {},
         "VERDICT: BLOCKED", "reasons:"),
        ("an unreadable domain id costs one reason, not the verdict",
         {"fuel_live": (10, 0)},
         {"frame_degraded_outcomes": [{
             "hour_raw": 7, "coverage_claim": "c",
             "domains": [{"domain_id": "not-a-number", "reason": "x"},
                         {"domain_id": 10, "reason": "KEYED_SENTINEL"}],
         }]},
         "1 degraded/refused frame outcome(s) carried a domain id or a reason",
         None),
        ("a non-integral domain id is counted, never truncated onto a real one",
         {"fuel_live": (10, 0)},
         {"frame_degraded_outcomes": [{
             "hour_raw": 7,
             "domains": [{"domain_id": 10.9, "reason": "TRUNCATION_SENTINEL"}],
         }]},
         "1 degraded/refused frame outcome(s) carried a domain id or a reason",
         "TRUNCATION_SENTINEL"),
        ("a non-string reason costs one reason, not the verdict",
         {"fuel_live": (10, 0)},
         {"frame_degraded_outcomes": [{
             "hour_raw": 7, "domains": [{"domain_id": 10, "reason": 123}],
         }]},
         "1 degraded/refused frame outcome(s) carried a domain id or a reason",
         None),
        ("type-mixed and unhashable coverage claims do not cost the verdict",
         {"fuel_live": (10, 0)},
         {"frame_degraded_outcomes": [
             {"hour_raw": 1, "coverage_claim": 5,
              "domains": [{"domain_id": 10, "reason": "a"}]},
             {"hour_raw": 2, "coverage_claim": "c",
              "domains": [{"domain_id": 10, "reason": "b"}]},
             {"hour_raw": 3, "coverage_claim": {"k": 1},
              "domains": [{"domain_id": 10, "reason": "c"}]},
         ]},
         "VERDICT: clean", None),
        ("an unreadable hour is named, not printed as a range",
         {"fuel_live": (10, 0)},
         {"frame_degraded_outcomes": [{
             "hour_raw": "not-an-hour",
             "domains": [{"domain_id": 10, "reason": "HOURLESS_SENTINEL"}],
         }]},
         "no readable hour", None),
        ("a malformed outcomes block does not cost the verdict",
         {"fuel_live": (10, 0)},
         {"frame_degraded_outcomes": ["not a dict", {"domains": "not a list"},
                                      {"domains": [None, 7]}, None]},
         "VERDICT: clean", None),
        ("an outcomes block that is not a list does not cost the verdict",
         {"fuel_live": (10, 0)},
         {"frame_degraded_outcomes": {"nope": 1},
          "frame_refused_outcomes": "nope"},
         "VERDICT: clean", None),
        ("whitespace variants are distinct strings and the sample is verbatim",
         {"fuel_live": (10, 0)},
         {"frame_degraded_outcomes": [
             {"hour_raw": 1, "domains": [{"domain_id": 10,
                                          "reason": " VERBATIM_SENTINEL"}]},
             {"hour_raw": 2, "domains": [{"domain_id": 10,
                                          "reason": "VERBATIM_SENTINEL"}]},
         ]},
         "sample full reason (1 of 2 distinct):  VERBATIM_SENTINEL", None),
        ("one reason written by two predicates is two causes",
         {"fuel_live": (10, 0)},
         {"frame_degraded_outcomes": [
             {"hour_raw": 1, "domains": [{
                 "domain_id": 10, "reason": "rows=3 | reason=the read did not complete",
                 "predicate_code": 0x00020011, "predicate_name": "d10_rows_rejected"}]},
             {"hour_raw": 2, "domains": [{
                 "domain_id": 10, "reason": "rows=9 | reason=the read did not complete",
                 "predicate_code": 0x00020012,
                 "predicate_name": "outcome_substantiated_from_record"}]},
         ]},
         "x1 predicate d10_rows_rejected [0x00020011]", "x2 predicate"),
        ("an entry with no predicate is counted as having none",
         {"fuel_live": (10, 0)},
         {"frame_degraded_outcomes": [
             {"hour_raw": 1, "domains": [{"domain_id": 10, "reason": "a"}]},
             {"hour_raw": 2, "domains": [{"domain_id": 10, "reason": "b"}]},
         ]},
         "x2 predicate (none on the wire)", None),
        ("an unreadable predicate code is named, not coerced",
         {"fuel_live": (10, 0)},
         {"frame_degraded_outcomes": [
             {"hour_raw": 1, "domains": [{
                 "domain_id": 10, "reason": "a",
                 "predicate_code": 10.9, "predicate_name": "d10_rows_rejected"}]},
         ]},
         "x1 predicate (unreadable predicate code)", None),
    )
    for label, domains, stream_extra, want_text, forbidden in reason_cases:
        code, out = run(domains, stream_extra=stream_extra)
        ok = want_text in out and has_receipt(out, code)
        if forbidden is not None and forbidden in out:
            ok = False
        print(f"selftest {'ok  ' if ok else 'FAIL'} exit={code}: {label}")
        if not ok:
            failures += 1
            print(out)

    total = (len(cases) + len(error_cases) + len(roster_cases)
             + len(reason_cases) + len(registry_cases) + 1)
    print(f"selftest: {total - failures}/{total} passed")
    return 1 if failures else 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--run-dir", required=True)
    parser.add_argument("--json", action="store_true", help="emit the report as JSON")
    parser.add_argument("--all-domains", action="store_true",
                        help="list every domain, not only the ones with status bits")
    args = parser.parse_args()

    run_dir = os.path.abspath(args.run_dir)
    manifest, manifest_path = load_manifest(run_dir)
    stream = manifest.get("compact_stream", {})
    per_domain = stream.get("per_domain", {})

    tolerated = {}
    tolerated_reason = {}
    tolerated_frames = {}
    for entry in manifest.get("expected_partial_tolerances_applied", []) or []:
        tolerated[entry.get("domain")] = int(entry.get("tolerated_flags", 0))
        tolerated_reason[entry.get("domain")] = entry.get("reason") or ""
        tolerated_frames[entry.get("domain")] = int(entry.get("frames_affected", 0))

    outcome_reasons, unreadable_reason_count = collect_outcome_reasons(stream)

    rows = []
    for name, info in sorted(per_domain.items()):
        domain_id = int(info.get("domain_id", -1))
        raw = int(info.get("status_flags_or", 0))
        globalized = globalize(domain_id, raw)
        residual_failure = globalized & FAILURE_MASK & ~tolerated.get(name, 0)
        rows.append({
            "outcome_reasons": outcome_reasons.get(domain_id),
            "domain": name,
            "domain_id": domain_id,
            "raw_flags": raw,
            "flag_names": decode(raw, domain_id),
            "lane_local_mask": LANE_LOCAL_MASK.get(domain_id, 0),
            "lane_local_bits": decode(
                raw & LANE_LOCAL_MASK.get(domain_id, 0), domain_id
            ),
            "tolerated_flags": tolerated.get(name, 0),
            "tolerated_reason": tolerated_reason.get(name, ""),
            "tolerated_frames_affected": tolerated_frames.get(name, 0),
            "residual_failure_flags": residual_failure,
            "residual_failure_names": decode(residual_failure, domain_id),
            "unknown_bits": undefined_bits(domain_id, raw),
            "data_record_count": info.get("data_record_count", 0),
            "row_count_total": info.get("row_count_total", 0),
            "problems": info.get("problems", {}) or {},
            "world_folded": info.get("world_folded"),
        })

    undefined = [r for r in rows if r["unknown_bits"]]
    blocking = [r for r in rows if r["residual_failure_flags"] or r["unknown_bits"]]
    silent = [r for r in rows if r["data_record_count"] == 0]
    with_problems = [r for r in rows if r["problems"]]

    expected = set(stream.get("expected_domains", []) or [])
    seen = set(stream.get("domains_seen", []) or [])
    pseudo_names, pseudo_source = load_pseudo_domain_names()
    pseudo_seen = sorted(seen & pseudo_names)
    missing = sorted(expected - seen)
    unexpected = sorted(seen - expected - pseudo_names)

    storage = manifest.get("stream_storage", {}) or {}
    hourly = manifest.get("hourly_requirement", {}) or {}
    artifact = manifest.get("replay_artifact", {}) or {}

    raw_stream_name = manifest.get("raw_stream") or "collector_stream.bin"
    raw_stream_path = os.path.join(run_dir, raw_stream_name)
    raw_stream_bytes = os.path.getsize(raw_stream_path) if os.path.isfile(raw_stream_path) else None

    report = {
        "run_dir": run_dir,
        "manifest": manifest_path,
        "session_id": manifest.get("session_id"),
        "pid": manifest.get("pid"),
        "duration_ms": manifest.get("duration_ms"),
        "stream_format": manifest.get("stream_format"),
        "stream_compression": storage.get("compression"),
        "raw_stream_file": raw_stream_name,
        "raw_stream_bytes_on_disk": raw_stream_bytes,
        "stream_decoded_bytes": storage.get("decoded_bytes"),
        "stream_decodable": stream.get("decodable"),
        "stream_ordered": stream.get("ordered"),
        "game_hours_advanced": manifest.get("game_hours_advanced"),
        "unique_game_hour_count": manifest.get("unique_game_hour_count"),
        "skipped_game_hours": manifest.get("skipped_game_hours"),
        "frame_end_record_count": stream.get("frame_end_record_count"),
        "domain_count": len(per_domain),
        "expected_domain_count": stream.get("domain_coverage", {}).get("expected_domain_count"),
        "domains_missing_from_stream": missing,
        "domains_unexpected_in_stream": unexpected,
        "infrastructure_pseudo_domains_in_stream": pseudo_seen,
        "pseudo_domain_authority": pseudo_source,
        "pseudo_domain_names": sorted(pseudo_names),
        "domain_coverage_errors": stream.get("domain_coverage", {}).get("errors", []),
        "parse_errors": manifest.get("parse_errors", []),
        "ordering_errors": manifest.get("ordering_errors", []),
        "collector_control_errors": manifest.get("collector_control_errors", []),
        "hourly_requirement_met": hourly.get("met"),
        "hourly_requirement_problems": hourly.get("problems", []),
        "replay_written_file": artifact.get("written_file"),
        "replay_is_recovery": artifact.get("recovery"),
        "replay_gate_problems": artifact.get("gate_problems", []),
        "blocking_domains": blocking,
        "undefined_status_bit_domains": [
            {
                "domain": r["domain"],
                "domain_id": r["domain_id"],
                "raw_flags": r["raw_flags"],
                "lane_local_mask": r["lane_local_mask"],
                "undefined_bits": r["unknown_bits"],
            }
            for r in undefined
        ],
        "silent_domains": [r["domain"] for r in silent],
        "domains_with_problems": [r["domain"] for r in with_problems],
        "descheduled_domains": stream.get("descheduled_domains", {}) or {},
        "outcome_reasons_by_domain_id": {
            str(domain_id): reasons for domain_id, reasons in sorted(outcome_reasons.items())
        },
        "outcome_reasons_unreadable": unreadable_reason_count,
        "all_domains": rows if args.all_domains else None,
    }

    if args.json:
        json.dump(report, sys.stdout, indent=2)
        print()
        return verdict_exit_code(undefined, blocking, hourly)

    def line(label: str, value: object) -> None:
        print(f"  {label:34s} {value}")

    print(f"\n=== capture: {report['session_id']} ===")
    line("run dir", run_dir)
    line("pid / duration", f"{report['pid']} / {report['duration_ms']} ms")
    line("stream format / compression", f"{report['stream_format']} / {report['stream_compression']}")
    line("raw stream on disk", f"{raw_stream_name} = {raw_stream_bytes} bytes")
    line("decoded stream bytes", report["stream_decoded_bytes"])
    line("decodable / ordered", f"{report['stream_decodable']} / {report['stream_ordered']}")
    line("game hours advanced", report["game_hours_advanced"])
    line("unique hours / skipped", f"{report['unique_game_hour_count']} / {report['skipped_game_hours']}")
    line("frames (frame_end records)", report["frame_end_record_count"])
    line("domains in stream / expected", f"{report['domain_count']} / {report['expected_domain_count']}")
    line("infrastructure pseudo-domains",
         f"{', '.join(pseudo_seen) or 'none in stream'}"
         f" (not scheduled by construction; authority: {pseudo_source})")
    not_scheduled = [
        entry.get("domain")
        for entry in report["descheduled_domains"].get("domains", []) or []
    ]
    if not_scheduled:
        block = report["descheduled_domains"]
        line("deliberately not scheduled", f"{len(not_scheduled)}: {', '.join(not_scheduled)}")
        by_ruling = {}
        for entry in block.get("domains", []) or []:
            key = (str(entry.get("decided_on") or "?"), str(entry.get("reason") or "?"))
            by_ruling.setdefault(key, []).append(entry.get("domain") or "<unnamed>")
        for (decided_on, reason), domains in sorted(by_ruling.items()):
            line(f"  ruling {decided_on}",
                 f"by {block.get('decided_by', '?')}"
                 f" ({block.get('authority', '?')}): {', '.join(sorted(domains))}")
            line("    reason", reason)

    print("\n--- gate verdict ---")
    line("hourly requirement met", report["hourly_requirement_met"])
    if report["hourly_requirement_problems"]:
        line("hourly problems", ", ".join(report["hourly_requirement_problems"]))
    line("replay file written", report["replay_written_file"])
    line("recovery (failed) replay", report["replay_is_recovery"])
    if report["replay_gate_problems"]:
        line("replay gate problems", ", ".join(report["replay_gate_problems"]))

    for label, key in (
        ("domains missing from stream", "domains_missing_from_stream"),
        ("unexpected domains in stream", "domains_unexpected_in_stream"),
        ("domain coverage errors", "domain_coverage_errors"),
        ("parse errors", "parse_errors"),
        ("ordering errors", "ordering_errors"),
        ("collector control errors", "collector_control_errors"),
        ("domains with zero data records", "silent_domains"),
    ):
        if report[key]:
            print(f"\n--- {label} ({len(report[key])}) ---")
            for item in report[key]:
                print(f"  {item}")

    if undefined:
        print(f"\n--- CONTRACT VIOLATION: UNDEFINED STATUS BITS ({len(undefined)}) ---")
        for row in undefined:
            print(f"  {row['domain']} (domain {row['domain_id']}) published status bits "
                  f"0x{row['unknown_bits']:x} that the compact status contract does not "
                  f"define")
            print(f"    raw status word          0x{row['raw_flags']:x}")
            print(f"    assigned v1 vocabulary   0x{KNOWN_MASK:x}")
            print(f"    registered lane-local    0x{row['lane_local_mask']:x} "
                  f"(compact_contract::lane_local_status_mask)")
            print(f"    UNDEFINED                0x{row['unknown_bits']:x} "
                  f"(bits {', '.join(str(b) for b in range(32) if row['unknown_bits'] & (1 << b))})")
        print("  No consumer can classify an undefined bit: it is neither a failure nor an\n"
              "  informational status. Either declare it in compact_contract (and register it\n"
              "  in lane_local_status_mask) or stop the producer writing it.")

    print(f"\n--- domains with residual failure/undefined status ({len(blocking)}) ---")
    if not blocking:
        print("  none")
    for row in blocking:
        print(f"  {row['domain']:48s} d{row['domain_id']:<3d} raw=0x{row['raw_flags']:x}")
        print(f"    {'flags:':10s} {', '.join(row['flag_names']) or 'none'}")
        if row["lane_local_bits"]:
            print(f"    {'lane-local:':10s} {', '.join(row['lane_local_bits'])}")
        if row["unknown_bits"]:
            print(f"    {'UNDEFINED:':10s} 0x{row['unknown_bits']:x}")
        if row["tolerated_flags"]:
            print(f"    {'tolerated:':10s} "
                  f"{', '.join(decode(row['tolerated_flags'], row['domain_id']))}"
                  f" over {row['tolerated_frames_affected']} frame(s)")
            if row["tolerated_reason"]:
                print(f"    {'because:':10s} {row['tolerated_reason']}")
        print(f"    {'RESIDUAL:':10s} {', '.join(row['residual_failure_names']) or 'none'}")
        print(f"    {'volume:':10s} records={row['data_record_count']} rows={row['row_count_total']}")
        if row["outcome_reasons"]:
            print_outcome_reasons(row["outcome_reasons"], "    ")
        if row["problems"]:
            print(f"    {'problems:':10s} {json.dumps(row['problems'])[:400]}")

    blocking_ids = {row["domain_id"] for row in blocking}
    other_reason_rows = [
        row for row in rows
        if row["outcome_reasons"] and row["domain_id"] not in blocking_ids
    ]
    unmatched_reason_ids = sorted(
        set(outcome_reasons) - {row["domain_id"] for row in rows}
    )
    if unreadable_reason_count:
        print(f"\n  NOTE: {unreadable_reason_count} degraded/refused frame outcome(s) "
              f"carried a domain id or a reason this report could not read, and are "
              f"therefore absent from every reason list above. The status bits for "
              f"those hours are unaffected; only the wire's stated cause is missing.")
    if other_reason_rows or unmatched_reason_ids:
        print(f"\n--- degraded/refused outcome reasons on non-blocking domains "
              f"({len(other_reason_rows) + len(unmatched_reason_ids)}) ---")
        for row in other_reason_rows:
            print(f"  {row['domain']:48s} d{row['domain_id']:<3d}")
            print_outcome_reasons(row["outcome_reasons"], "    ")
        for domain_id in unmatched_reason_ids:
            print(f"  (no per-domain entry){'':26s} d{domain_id:<3d}")
            print_outcome_reasons(outcome_reasons[domain_id], "    ")

    if with_problems:
        print(f"\n--- domains reporting skip/omission counters ({len(with_problems)}) ---")
        for row in with_problems:
            print(f"  {row['domain']:48s} {json.dumps(row['problems'])[:300]}")

    if args.all_domains:
        print(f"\n--- every domain ({len(rows)}) ---")
        for row in rows:
            names = ", ".join(row["flag_names"]) or "clean"
            print(f"  {row['domain']:48s} d{row['domain_id']:<3d} recs={row['data_record_count']:<6d} "
                  f"rows={row['row_count_total']:<10d} {names}")

    exit_code = verdict_exit_code(undefined, blocking, hourly)
    verdict = {
        0: "clean",
        1: "BLOCKED",
        EXIT_UNDEFINED_STATUS_BITS: "BLOCKED (UNDEFINED STATUS BITS)",
    }[exit_code]
    print(f"\nVERDICT: {verdict} "
          f"({len(blocking)} domain(s) with residual failure status, "
          f"{len(undefined)} with undefined status bits)\n")
    return exit_code


def guarded_main() -> int:
    """`main` with every unexpected failure mapped to EXIT_REPORT_ERROR.

    Catches Exception rather than a list of expected types: a failure this file
    did not anticipate must not be reported as an ordinary verdict. `SystemExit`
    (argparse usage errors) and `KeyboardInterrupt` (the operator cancelled)
    both propagate untouched -- neither is a report crash, and relabelling a
    cancellation as one would be a lie about what happened.
    """
    try:
        code = main()
    except ReportError as error:
        print(f"\nREPORT ERROR: {error}", file=sys.stderr)
        return emit_verdict_receipt(EXIT_REPORT_ERROR)
    except Exception as error:
        import traceback

        traceback.print_exc()
        print(
            f"\nREPORT ERROR: {type(error).__name__}: {error}\n"
            "This report reached NO VERDICT. It is not evidence that the capture "
            "is clean, and it is not evidence that it is dirty.",
            file=sys.stderr,
        )
        return emit_verdict_receipt(EXIT_REPORT_ERROR)
    return emit_verdict_receipt(code)


if __name__ == "__main__":
    if "--selftest" in sys.argv[1:]:
        raise SystemExit(selftest())
    raise SystemExit(guarded_main())
