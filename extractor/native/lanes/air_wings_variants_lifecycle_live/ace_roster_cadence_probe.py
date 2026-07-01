"""Lane-local cadence proof for direct ace roster compact capture.

This wrapper repeatedly runs the validated ace roster probe against live
`hoi4.exe`, writes one compact H4CR record per sample, and records bounded
wall-time evidence for hourly-or-lower scheduling. It deliberately stays outside
shared collector/recorder entrypoints.
"""

from __future__ import annotations

import argparse
import json
import statistics
import struct
import time
from pathlib import Path
from typing import Any

import ace_root_candidate_probe as ace_probe


def read_h4cr_header(path: Path) -> dict[str, Any]:
    data = path.read_bytes()
    if len(data) < 32:
        return {"available": False, "reason": "record_too_short", "bytes": len(data)}
    magic = data[:4]
    if magic != b"H4CR":
        return {"available": False, "reason": "invalid_magic", "magic": magic.hex()}
    version, header_len = struct.unpack_from("<HH", data, 4)
    sequence = struct.unpack_from("<Q", data, 8)[0]
    game_hour_raw = struct.unpack_from("<i", data, 16)[0]
    domain_id, record_type = struct.unpack_from("<HH", data, 20)
    payload_len = struct.unpack_from("<I", data, 24)[0]
    return {
        "available": True,
        "record_bytes": len(data),
        "version": version,
        "header_len": header_len,
        "sequence": sequence,
        "game_hour_raw": game_hour_raw,
        "domain_id": domain_id,
        "record_type": record_type,
        "payload_len": payload_len,
    }


def sample_once(args: argparse.Namespace, sample_dir: Path, sample_index: int) -> dict[str, Any]:
    sample_dir.mkdir(parents=True, exist_ok=True)
    record_path = sample_dir / "air_ace_roster_compact_record.bin"
    probe_args = argparse.Namespace(
        abilities=args.abilities,
        out_dir=str(sample_dir),
        melted_save=args.melted_save,
        compact_record_out=str(record_path),
        pid=args.pid,
    )
    started = time.perf_counter()
    proof = ace_probe.build_probe(probe_args)
    elapsed_ms = int(round((time.perf_counter() - started) * 1000))
    proof_path = sample_dir / "ace_root_candidate_probe.json"
    proof_path.write_text(json.dumps(proof, indent=2, sort_keys=True), encoding="utf-8")
    compact = proof.get("ace_roster_compact_record", {})
    header = read_h4cr_header(record_path) if record_path.exists() else {"available": False, "reason": "missing_record"}
    return {
        "sample_index": sample_index,
        "elapsed_wall_ms": elapsed_ms,
        "proof": str(proof_path),
        "compact_record": str(record_path) if record_path.exists() else None,
        "completion_status": proof.get("completion_status"),
        "conclusion": proof.get("conclusion"),
        "live_clock": proof.get("live_clock"),
        "compact_available": compact.get("available"),
        "row_count": compact.get("row_count"),
        "identity_rows": compact.get("identity_rows"),
        "wing_assignment_rows": compact.get("wing_assignment_rows"),
        "assignment_validated": compact.get("assignment_validated"),
        "record_header": header,
    }


def contiguous_or_single(hours: list[int]) -> bool:
    unique = sorted(set(hours))
    if len(unique) <= 1:
        return True
    return all((b - a) <= 1 for a, b in zip(unique, unique[1:]))


def build_report(args: argparse.Namespace) -> dict[str, Any]:
    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    samples = []
    for index in range(args.samples):
        samples.append(sample_once(args, out_dir / f"sample_{index:02d}", index))
        if index + 1 < args.samples and args.sleep_seconds > 0:
            time.sleep(args.sleep_seconds)

    elapsed_values = [sample["elapsed_wall_ms"] for sample in samples]
    game_hours = [
        sample.get("record_header", {}).get("game_hour_raw")
        for sample in samples
        if isinstance(sample.get("record_header", {}).get("game_hour_raw"), int)
    ]
    compact_ok = all(sample.get("compact_available") and sample.get("row_count", 0) > 0 for sample in samples)
    assignments_ok = all(sample.get("assignment_validated") for sample in samples)
    headers_ok = all(
        sample.get("record_header", {}).get("domain_id") == ace_probe.DOMAIN_ID
        and sample.get("record_header", {}).get("record_type") == ace_probe.COMPACT_RECORD_TYPE_ACE_ROSTER_DATA
        for sample in samples
    )
    max_elapsed = max(elapsed_values) if elapsed_values else None
    bounded_probe_runtime = bool(
        samples
        and compact_ok
        and assignments_ok
        and headers_ok
        and max_elapsed is not None
        and max_elapsed <= args.max_sample_wall_ms
    )
    unique_hours = sorted(set(game_hours))
    zero_skipped = contiguous_or_single(game_hours)
    speed_five_hourly_proven = bool(bounded_probe_runtime and len(unique_hours) > 1 and zero_skipped)
    return {
        "schema": "air_variant_archetypes_live.ace_roster_cadence_probe.v1",
        "completion_status": "NOT_COMPLETE",
        "backlog_reduced": bool(samples and compact_ok and headers_ok),
        "sample_count": len(samples),
        "sleep_seconds": args.sleep_seconds,
        "max_sample_wall_ms_gate": args.max_sample_wall_ms,
        "elapsed_wall_ms": {
            "min": min(elapsed_values) if elapsed_values else None,
            "max": max_elapsed,
            "mean": statistics.mean(elapsed_values) if elapsed_values else None,
        },
        "compact_records_all_available": compact_ok,
        "assignment_all_validated": assignments_ok,
        "record_headers_all_domain46_type_0xACE1": headers_ok,
        "game_hour_raw_samples": game_hours,
        "unique_game_hour_raw_count": len(unique_hours),
        "unique_game_hour_raw_values": unique_hours,
        "zero_skipped_hours": zero_skipped,
        "bounded_probe_runtime_below_gate": bounded_probe_runtime,
        "hourly_or_lower_live_cadence_bounded": speed_five_hourly_proven,
        "speed_five_current_equivalent_hourly_or_lower": speed_five_hourly_proven,
        "cadence_candidate_rejection": (
            "lane-local Python ReadProcessMemory probe emits valid ace compact records but does not prove "
            "production speed-five hourly capture; shared DLL scheduler registration is still required"
            if not speed_five_hourly_proven
            else None
        ),
        "cadence_scope": (
            "lane-local live ReadProcessMemory probe cost evidence only; wing assignment is mutable, "
            "so startup-only/static capture is rejected"
        ),
        "samples": samples,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--abilities", required=True)
    parser.add_argument("--out-dir", required=True)
    parser.add_argument("--melted-save")
    parser.add_argument("--pid", type=int)
    parser.add_argument("--samples", type=int, default=3)
    parser.add_argument("--sleep-seconds", type=float, default=0.0)
    parser.add_argument("--max-sample-wall-ms", type=int, default=3_600_000)
    args = parser.parse_args()

    report = build_report(args)
    out_path = Path(args.out_dir) / "ace_roster_cadence_probe.json"
    out_path.write_text(json.dumps(report, indent=2, sort_keys=True), encoding="utf-8")
    print(json.dumps({"out": str(out_path), **{k: report[k] for k in (
        "completion_status",
        "backlog_reduced",
        "sample_count",
        "elapsed_wall_ms",
        "unique_game_hour_raw_count",
        "zero_skipped_hours",
        "hourly_or_lower_live_cadence_bounded",
        "bounded_probe_runtime_below_gate",
    )}}, indent=2, sort_keys=True))
    return 0 if report.get("backlog_reduced") else 1


if __name__ == "__main__":
    raise SystemExit(main())
