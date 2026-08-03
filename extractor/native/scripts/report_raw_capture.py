#!/usr/bin/env python3
"""Read a recorder run directory directly and print what actually happened.

No decoding library, no replay app, no debug profile: this opens the raw
artifacts the recorder leaves on disk (`native_session_manifest.json`, the
console logs, and the raw H4CR stream) and reports the exact per-domain
extraction status, so "which data did NOT come out" is answerable without
re-running anything.

Usage:
    python report_raw_capture.py --run-dir <dir> [--json] [--all-domains]
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


def verdict_exit_code(undefined: list, blocking: list, hourly: dict) -> int:
    if undefined:
        return EXIT_UNDEFINED_STATUS_BITS
    if blocking or hourly.get("met") is False:
        return 1
    return 0


def load_manifest(run_dir: str) -> tuple[dict, str]:
    for name in ("native_session_manifest.json", "recorder/native_session_manifest.json"):
        path = os.path.join(run_dir, name)
        if os.path.isfile(path):
            with open(path, encoding="utf-8") as handle:
                return json.load(handle), path
    raise SystemExit(f"no native_session_manifest.json under {run_dir}")


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


if __name__ == "__main__":
    raise SystemExit(main())
