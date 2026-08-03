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

# --- compact status vocabulary (extractor/native/compact_contract/src) -------
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
    # The read reached its source, the source answered, and the answer was
    # empty. Informational, and deliberately absent from FAILURE_MASK below: a
    # measured zero is an ANSWER, not a missing hour. Before this bit existed a
    # domain that legitimately observed nothing published `source_unavailable`,
    # which is what a BROKEN reader publishes, and this report could not tell
    # them apart.
    1 << 14: "source_empty",
}
KNOWN_MASK = (1 << 15) - 1
FAILURE_MASK = (
    (1 << 0) | (1 << 1) | (1 << 2) | (1 << 3) | (1 << 6)
    | (1 << 7) | (1 << 8) | (1 << 9) | (1 << 12)
)
# Lane-local status badges, per domain: a domain-scoped vocabulary that must be
# stripped before the word is read against the global v1 bits. This table IS a
# copy of `compact_contract::lane_local_status_mask`, and the copy is exactly
# how this report went wrong: it knew only d49/d51 while the contract had
# registered d53, d60 and d99, so every d53/d60 capture was reported as
# "UNKNOWN_BITS" -- a false contract violation on bits the contract defines.
# `compact_contract/tests/script_status_vocabulary_sweep.rs` now parses this
# table and fails if it ever disagrees with the contract again.
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


# A capture that published a status bit nothing defines is not "a capture with
# a problem to weigh" -- it is a capture whose status word cannot be read at
# all, by this report or by any other consumer. It gets its own exit code so
# callers can block on THAT specifically without having to block on every
# ordinary residual failure (a PARTIAL_DATA hour is a known, classifiable
# outcome; an undefined bit is not).
EXIT_UNDEFINED_STATUS_BITS = 2
# This report REACHED NO VERDICT: it crashed, or could not read the capture at
# all. Distinct from 1 on purpose. Exit 1 means "I read the status words and
# some domain is failing" -- an answer. An unhandled exception used to land on
# 1 as well, so a malformed manifest or a bug in this file was indistinguishable
# from a clean, informative "blocking domains" verdict, and a caller that walks
# past exit 1 (as the capture harness does, deliberately) walked past a
# definedness check that never ran.
EXIT_REPORT_ERROR = 3

# Printed to STDERR by every path that reaches a verdict, and by no other path.
# An exit code alone cannot be trusted: a SyntaxError, a failed import, or a
# missing interpreter kills python BEFORE any code in this file runs, and the
# process exits 1 -- which is this report's code for "I read the status words
# and some domain is failing", an ANSWER that a caller is entitled to walk past.
# The receipt is the difference between "the check said 1" and "something
# exited 1"; a caller that does not see it must treat the definedness check as
# never having run.
#
# STDERR, not stdout, because `--json` writes one complete JSON document to
# stdout and a caller pipes it straight into a parser. A receipt appended there
# makes the document unparseable -- which is exactly what it did before this
# was moved.
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

    def build(domains):
        return {
            "session_id": "selftest",
            "compact_stream": {
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
            },
            "hourly_requirement": {"met": True, "problems": []},
            "replay_artifact": {
                "written_file": "x", "recovery": False, "gate_problems": [],
            },
        }

    def run(domains, raw_manifest=None, write_manifest=True):
        with tempfile.TemporaryDirectory() as run_dir:
            if write_manifest:
                path = os.path.join(run_dir, "native_session_manifest.json")
                with open(path, "w", encoding="utf-8") as handle:
                    if raw_manifest is None:
                        json.dump(build(domains), handle)
                    else:
                        handle.write(raw_manifest)
            argv = sys.argv
            sys.argv = [argv[0], "--run-dir", run_dir]
            buffer = io.StringIO()
            try:
                # guarded_main, not main: the exit code a caller BLOCKS on is
                # the guarded one, and the guard is itself under test.
                with contextlib.redirect_stdout(buffer):
                    with contextlib.redirect_stderr(buffer):
                        code = guarded_main()
            finally:
                sys.argv = argv
            return code, buffer.getvalue()

    cases = (
        ("a clean word is clean",
         {"fuel_live": (10, 0)}, 0, "VERDICT: clean", "UNDEFINED"),
        # The regression this file exists for: these are the exact words
        # runs/live_unlock/t15 published, and they were reported as
        # UNKNOWN_BITS(0x530000)/UNKNOWN_BITS(0x30000).
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
         {"fuel_live": (10, 1 << 15)}, EXIT_UNDEFINED_STATUS_BITS,
         "CONTRACT VIOLATION: UNDEFINED STATUS BITS", None),
        # The strip is domain-scoped, never a blanket amnesty for high bits.
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

    # "This report reached no verdict" must never be reported as an ordinary
    # blocking verdict: exit 1 is an ANSWER, and the capture harness walks past
    # exit 1 on purpose. Before EXIT_REPORT_ERROR existed, a malformed manifest
    # and a crash in this file both landed on 1.
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
    total = len(cases) + len(error_cases)
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
    for entry in manifest.get("expected_partial_tolerances_applied", []) or []:
        tolerated[entry.get("domain")] = int(entry.get("tolerated_flags", 0))

    rows = []
    for name, info in sorted(per_domain.items()):
        domain_id = int(info.get("domain_id", -1))
        raw = int(info.get("status_flags_or", 0))
        globalized = globalize(domain_id, raw)
        residual_failure = globalized & FAILURE_MASK & ~tolerated.get(name, 0)
        rows.append({
            "domain": name,
            "domain_id": domain_id,
            "raw_flags": raw,
            "flag_names": decode(raw, domain_id),
            "lane_local_mask": LANE_LOCAL_MASK.get(domain_id, 0),
            "lane_local_bits": decode(
                raw & LANE_LOCAL_MASK.get(domain_id, 0), domain_id
            ),
            "tolerated_flags": tolerated.get(name, 0),
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
    missing = sorted(expected - seen)
    unexpected = sorted(seen - expected)

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
        # This report builds its rows from `per_domain`, so a domain that never
        # produced anything is invisible to every list above. For a domain that
        # was never scheduled that absence is correct but unexplained, so carry
        # the decision through verbatim rather than leaving the roster short by
        # five with no reason attached.
        "descheduled_domains": stream.get("descheduled_domains", {}) or {},
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
    not_scheduled = [
        entry.get("domain")
        for entry in report["descheduled_domains"].get("domains", []) or []
    ]
    if not_scheduled:
        block = report["descheduled_domains"]
        line("deliberately not scheduled", f"{len(not_scheduled)}: {', '.join(not_scheduled)}")
        line("  decision", f"{block.get('decided_on', '?')}"
                           f" by {block.get('decided_by', '?')}"
                           f" ({block.get('authority', '?')})")
        # Without the reason the line reads as an unexplained shortfall, which
        # is the thing this record exists to stop it being.
        line("  reason", block.get("reason", "?"))

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
                  f"{', '.join(decode(row['tolerated_flags'], row['domain_id']))}")
        print(f"    {'RESIDUAL:':10s} {', '.join(row['residual_failure_names']) or 'none'}")
        print(f"    {'volume:':10s} records={row['data_record_count']} rows={row['row_count_total']}")
        if row["problems"]:
            print(f"    {'problems:':10s} {json.dumps(row['problems'])[:400]}")

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
    except Exception as error:  # noqa: BLE001 -- see docstring
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
