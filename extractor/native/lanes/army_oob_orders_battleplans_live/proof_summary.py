"""Gate a compact army/OOB proof manifest.

This is lane-local and intentionally avoids shared recorder imports. It checks
the proof contract for compact `army_orders` and `oob_command` runs.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


EXPECTED_DOMAINS = ("army_orders", "oob_command")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("manifest", type=Path)
    ap.add_argument("--min-game-hours", type=int, default=5)
    args = ap.parse_args()

    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    errors = gate_manifest(manifest, args.manifest.parent, args.min_game_hours)
    compact = manifest.get("compact_stream", {})
    per_domain = compact.get("per_domain", {})
    summary = {
        "status": "PASS" if not errors else "FAIL",
        "errors": errors,
        "run_dir": str(args.manifest.parent),
        "game_hours_advanced": manifest.get("game_hours_advanced"),
        "unique_game_hour_count": manifest.get("unique_game_hour_count"),
        "skipped_game_hours": manifest.get("skipped_game_hours"),
        "frame_end_record_count": compact.get("frame_end_record_count"),
        "total_bytes": compact.get("total_bytes"),
        "division_root_source": manifest.get("division_root_source"),
        "army_orders": summarize_domain(per_domain.get("army_orders", {})),
        "oob_command": summarize_domain(per_domain.get("oob_command", {})),
    }
    print(json.dumps(summary, indent=2, sort_keys=True))
    return 0 if not errors else 1


def gate_manifest(manifest: dict[str, Any], run_dir: Path, min_game_hours: int) -> list[str]:
    errors: list[str] = []
    compact = manifest.get("compact_stream")
    if not isinstance(compact, dict):
        return ["missing compact_stream object"]

    scheduled = tuple(manifest.get("scheduled_domains") or ())
    if scheduled != EXPECTED_DOMAINS:
        errors.append(f"scheduled domains {scheduled!r} != {EXPECTED_DOMAINS!r}")
    if manifest.get("stream_format") != "compact":
        errors.append("stream_format is not compact")
    if not manifest.get("extraction_perf_gate"):
        errors.append("extraction_perf_gate is false")
    if manifest.get("division_root_source") != "dll_native":
        errors.append("division_root_source is not dll_native")
    if manifest.get("division_body_bridge_rows") not in (0, None):
        errors.append("division_body_bridge_rows is nonzero")

    if manifest.get("game_hours_advanced", 0) < min_game_hours:
        errors.append("insufficient game_hours_advanced")
    if manifest.get("skipped_game_hours") != 0:
        errors.append("skipped_game_hours is nonzero")
    hourly = manifest.get("hourly_requirement") or {}
    if hourly.get("met") is not True:
        errors.append(f"hourly_requirement not met: {hourly.get('problems')}")
    if manifest.get("parse_errors"):
        errors.append("manifest parse_errors is non-empty")
    if manifest.get("ordering_errors"):
        errors.append("manifest ordering_errors is non-empty")

    for key in ("decodable", "ordered"):
        if compact.get(key) is not True:
            errors.append(f"compact_stream.{key} is not true")
    coverage = compact.get("domain_coverage") or {}
    if coverage.get("ok") is not True:
        errors.append(f"domain coverage failed: {coverage.get('errors')}")
    for key in ("worker_panic_seen", "bridge_unavailable_seen", "source_unavailable_seen", "partial_data_seen"):
        if compact.get(key) is not False:
            errors.append(f"compact_stream.{key} is not false")

    frame_count = compact.get("frame_end_record_count")
    per_domain = compact.get("per_domain") or {}
    for domain_name in EXPECTED_DOMAINS:
        domain = per_domain.get(domain_name)
        if not isinstance(domain, dict):
            errors.append(f"missing per-domain summary for {domain_name}")
            continue
        if domain.get("data_record_count") != frame_count:
            errors.append(f"{domain_name} data_record_count does not match frame count")
        if domain.get("data_rows_total", 0) <= 0:
            errors.append(f"{domain_name} emitted no rows")
        if domain.get("status_flags_or", 0) & ((1 << 6) | (1 << 7) | (1 << 8) | (1 << 9)):
            errors.append(f"{domain_name} has failure status flags")

    raw_stream = run_dir / str(manifest.get("raw_stream", "collector_stream.bin"))
    replay_file = run_dir / str(manifest.get("replay_file", "native_session.battleplan"))
    if not raw_stream.exists():
        errors.append(f"missing raw stream {raw_stream}")
    if not replay_file.exists():
        errors.append(f"missing replay file {replay_file}")
    return errors


def summarize_domain(domain: dict[str, Any]) -> dict[str, Any]:
    units = domain.get("data_units_total") or {}
    return {
        "data_record_count": domain.get("data_record_count"),
        "data_rows_total": domain.get("data_rows_total"),
        "changed_rows_total": domain.get("changed_rows_total"),
        "read_time_p95_us": (domain.get("read_time_us") or {}).get("p95"),
        "status_flags_or": domain.get("status_flags_or"),
        "key_units": {
            key: units[key]
            for key in sorted(units)
            if key.endswith("_rows") or key.endswith("_armies") or key.endswith("_orders")
        },
    }


if __name__ == "__main__":
    raise SystemExit(main())
