"""Materialize validated save/live path geometry rows as domain-54 compact bytes."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import sys
from typing import Any


LANE_DIR = Path(__file__).resolve().parent
if str(LANE_DIR) not in sys.path:
    sys.path.insert(0, str(LANE_DIR))

import army_battleplans_depth_from_stream as depth_stream  # type: ignore  # noqa: E402
import battleplan_save_geometry_gate as geometry_gate  # type: ignore  # noqa: E402


SCHEMA = "army_battleplans_depth_path_geometry_compact_from_probe_v1"
SOURCE_CANDIDATE = "non_type5_path_vector_plus_0x70"
MAX_PROVINCE_ID = 65_535


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--probe", type=Path, required=True)
    ap.add_argument("--out-stream", type=Path, required=True)
    ap.add_argument("--out-summary", type=Path, required=True)
    ap.add_argument("--game-hour", type=int, default=0)
    ap.add_argument("--min-path-rows", type=int, default=32)
    args = ap.parse_args()

    proof = json.loads(args.probe.read_text(encoding="utf-8"))
    stream, summary = build_stream_from_proof(
        proof,
        source_probe=args.probe,
        game_hour=args.game_hour,
        min_path_rows=args.min_path_rows,
    )
    args.out_stream.parent.mkdir(parents=True, exist_ok=True)
    args.out_summary.parent.mkdir(parents=True, exist_ok=True)
    args.out_stream.write_bytes(stream)
    args.out_summary.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(summary, indent=2, sort_keys=True))
    return 0 if summary["status"] == "PASS" else 1


def build_stream_from_proof(
    proof: dict[str, Any],
    *,
    source_probe: Path | None,
    game_hour: int,
    min_path_rows: int,
) -> tuple[bytes, dict[str, Any]]:
    errors = geometry_gate.validate(
        proof,
        min_type5_exact=64,
        min_path70_exact=min_path_rows,
        require_type3_c0_rejected=True,
    )
    candidates = {row.get("name"): row for row in proof.get("candidate_results", []) if isinstance(row, dict)}
    path_candidate = candidates.get(SOURCE_CANDIDATE)
    rows: list[dict[str, Any]] = []
    row_errors: list[str] = []
    if not isinstance(path_candidate, dict):
        row_errors.append(f"{SOURCE_CANDIDATE} missing")
    else:
        for index, source_row in enumerate(path_candidate.get("compact_rows", [])):
            converted, error = convert_path_row(index, source_row)
            if error:
                row_errors.append(error)
            elif converted is not None:
                rows.append(converted)
    if len(rows) < min_path_rows:
        row_errors.append(f"only {len(rows)} compact path rows, expected at least {min_path_rows}")
    errors.extend(row_errors)

    payload = depth_stream.encode_depth_payload(
        rows,
        [],
        elapsed_ms=0,
        status_flags=0,
        changed_rows=len(rows),
        observed_army_rows=len({row["army_id"] for row in rows}),
        observed_order_rows=len(rows),
    )
    stream = b"".join(
        [
            depth_stream.write_record(
                1,
                game_hour,
                depth_stream.DOMAIN_ARMY_BATTLEPLANS_DEPTH,
                depth_stream.RECORD_TYPE_DOMAIN_DATA,
                payload,
            ),
            depth_stream.write_record(
                2,
                game_hour,
                depth_stream.DOMAIN_FRAME,
                depth_stream.RECORD_TYPE_FRAME_END,
                b"",
            ),
        ]
    )
    path_geometry_rows = sum(len(row["path_provinces"]) for row in rows)
    summary = {
        "schema": SCHEMA,
        "status": "PASS" if not errors else "FAIL",
        "errors": errors,
        "source_probe": str(source_probe) if source_probe else None,
        "source_candidate": SOURCE_CANDIDATE,
        "source_candidate_status": path_candidate.get("status") if isinstance(path_candidate, dict) else None,
        "source_candidate_exact_rows": path_candidate.get("exact_rows") if isinstance(path_candidate, dict) else None,
        "domain_id": depth_stream.DOMAIN_ARMY_BATTLEPLANS_DEPTH,
        "domain_name": "army_battleplans_depth_live",
        "depth_records": 1,
        "frame_end_records": 1,
        "hours": [game_hour, game_hour],
        "zero_skipped_hours": True,
        "one_depth_record_per_hour": True,
        "rows": len(rows),
        "changed_rows": len(rows),
        "target_rows": 0,
        "path_geometry_rows": path_geometry_rows,
        "non_type5_exact_path_rows": len(rows),
        "unsupported_geometry_rows": 0,
        "execution_unknown_rows": len(rows),
        "assigned_units_exact_rows": len(rows),
        "order_linkage_exact_rows": len(rows),
        "stance_unknown_rows": len(rows),
        "aggressiveness_unknown_rows": len(rows),
        "compact_stream_bytes": len(stream),
        "dll_only": False,
        "production_position": (
            "Lane-local compact materialization of save/live validated order+0x70/+0x78 path rows. "
            "Shared DLL hot-loop feed and redraw lifecycle semantics remain open."
        ),
    }
    return stream, summary


def convert_path_row(index: int, source_row: Any) -> tuple[dict[str, Any] | None, str | None]:
    if not isinstance(source_row, dict):
        return None, f"compact row {index} is not an object"
    try:
        army_id = int(source_row["army_id"])
        slot_index = int(source_row["slot_index"])
        order_type = int(source_row["order_type"])
    except (KeyError, TypeError, ValueError) as exc:
        return None, f"compact row {index} has bad identity fields: {exc}"
    path_provinces = source_row.get("path_provinces")
    if not isinstance(path_provinces, list) or not path_provinces:
        return None, f"compact row {index} has no path provinces"
    if order_type == 5:
        return None, f"compact row {index} is type-5, expected non-type-5 path geometry"
    try:
        path_values = [int(value) for value in path_provinces]
    except (TypeError, ValueError) as exc:
        return None, f"compact row {index} has non-integer path province: {exc}"
    if any(value <= 0 or value > MAX_PROVINCE_ID for value in path_values):
        return None, f"compact row {index} has out-of-range path province"
    generation = source_row.get("generation")
    if generation is None:
        generation = source_row.get("instance_id")
    try:
        generation_int = int(generation) if generation is not None else index + 1
    except (TypeError, ValueError):
        generation_int = index + 1
    return (
        {
            "army_id": army_id,
            "generation": generation_int,
            "deleted": False,
            "slot_index": slot_index,
            "order_type": order_type,
            "target_states": [],
            "path_provinces": path_values,
            "target_declared": None,
            "target_status": depth_stream.STATUS_UNSUPPORTED,
            "geometry_status": depth_stream.STATUS_EXACT,
            "execution_status": depth_stream.STATUS_FIXTURE_REQUIRED,
            "assigned_units_status": depth_stream.STATUS_EXACT,
            "order_linkage_status": depth_stream.STATUS_EXACT,
            "stance_status": depth_stream.STATUS_UNKNOWN,
            "aggressiveness_status": depth_stream.STATUS_UNKNOWN,
        },
        None,
    )


if __name__ == "__main__":
    raise SystemExit(main())
