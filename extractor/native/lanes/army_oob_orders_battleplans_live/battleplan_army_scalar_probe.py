"""Probe army-level battleplan scalar candidates against a same-save oracle.

The save's `orders_group` carries army-level battleplan settings such as
`execution_type`, `cohesion_type`, and `proximity_type`. This lane-local probe
uses those fields only as a candidate oracle, then validates/rejects simple
inline CArmy scalar offsets from live memory through the current-build army
vtable/type/id gates.
"""

from __future__ import annotations

import argparse
from collections import Counter
from datetime import datetime, timezone
import json
from pathlib import Path
import re
import sys
import time
from typing import Any


REPO = Path(__file__).resolve().parents[4]
MEMORY_READER = REPO / "extractor" / "memory_reader"
SAVE_ORACLE = REPO / "extractor" / "save_oracle"
LANE_DIR = Path(__file__).resolve().parent
for path in (str(REPO), str(MEMORY_READER), str(SAVE_ORACLE), str(LANE_DIR)):
    if path not in sys.path:
        sys.path.insert(0, path)

import direct_oob_probe  # type: ignore  # noqa: E402
import winmem  # type: ignore  # noqa: E402
from save_oracle import parse_save as ps  # type: ignore  # noqa: E402


SCHEMA = "army_oob_orders_battleplans_army_scalar_probe_v1"
PROC_NAME = direct_oob_probe.PROC_NAME
EXPECTED_SHA256 = direct_oob_probe.EXPECTED_SHA256

DEFAULT_SCAN_BEGIN = 0x00
DEFAULT_SCAN_END = 0x300
MIN_MATCHED_ARMIES = 32
NEAR_MATCH_FRACTION = 0.95
FIELDS = ("execution_type", "cohesion_type", "proximity_type")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--pid", type=int)
    ap.add_argument("--save", type=Path, default=default_autosave_path())
    ap.add_argument("--rakaly", type=Path)
    ap.add_argument("--out-dir", type=Path, required=True)
    ap.add_argument("--scan-begin", type=parse_int, default=DEFAULT_SCAN_BEGIN)
    ap.add_argument("--scan-end", type=parse_int, default=DEFAULT_SCAN_END)
    args = ap.parse_args()

    payload = run(
        args.pid,
        args.save,
        args.rakaly,
        args.out_dir,
        scan_begin=args.scan_begin,
        scan_end=args.scan_end,
    )
    view = {
        "status": "PASS" if not payload["errors"] else "FAIL",
        "errors": payload["errors"],
        "candidate_status": payload["candidate_status"],
        "matched_armies": payload["matched_armies"],
        "field_statuses": {name: row["status"] for name, row in payload["field_results"].items()},
        "accepted_offsets": {
            name: row["accepted_offsets"] for name, row in payload["field_results"].items()
        },
    }
    print(json.dumps(view, indent=2, sort_keys=True))
    return 0 if not payload["errors"] else 1


def run(
    pid: int | None,
    save_path: Path,
    rakaly_path: Path | None,
    out_dir: Path,
    *,
    scan_begin: int,
    scan_end: int,
) -> dict[str, Any]:
    if pid is None:
        pid = winmem.find_pid(PROC_NAME)
    if pid is None:
        raise RuntimeError("hoi4.exe not running")
    if scan_begin < 0 or scan_end <= scan_begin:
        raise ValueError("invalid scan range")

    out_dir.mkdir(parents=True, exist_ok=True)
    started = time.monotonic()
    text, save_meta = melt_save(save_path, rakaly_path, out_dir)
    save_armies = parse_orders_group_scalars(text)

    proc = winmem.Process(pid, PROC_NAME)
    try:
        fingerprint = proc.fingerprint()
        sha = str(fingerprint.get("sha256") or "").lower()
        if sha != EXPECTED_SHA256:
            raise RuntimeError(f"build gate failed: {sha} != {EXPECTED_SHA256}")
        roots, scan_stats = direct_oob_probe.enum_current_oob_roots(proc)
        army_roots = {int(root.id): int(root.addr) for root in roots.get("army", [])}
        matched_ids = sorted(set(army_roots) & set(save_armies))
        live_rows = read_live_army_bytes(proc, army_roots, matched_ids, scan_begin, scan_end)
        field_results = {
            field: evaluate_field(field, save_armies, live_rows, scan_begin, scan_end)
            for field in FIELDS
        }
        candidate_status = classify_payload(field_results)
        payload = {
            "schema": SCHEMA,
            "created_utc": datetime.now(timezone.utc).isoformat(),
            "pid": pid,
            "exe_sha256": sha,
            "module_base": proc.module_base,
            "module_path": proc.module_path,
            "elapsed_s": time.monotonic() - started,
            "save": save_meta,
            "scan_stats": scan_stats,
            "scan_range": {"begin": f"+0x{scan_begin:X}", "end_exclusive": f"+0x{scan_end:X}"},
            "save_army_count": len(save_armies),
            "live_army_count": len(army_roots),
            "matched_armies": len(matched_ids),
            "matched_army_ids_sample": matched_ids[:64],
            "field_results": field_results,
            "candidate_status": candidate_status,
            "production_position": (
                "same-autosave scalar oracle only; accepted offsets still need DLL integration "
                "and should remain fail-closed if build, army identity, or scalar distribution "
                "validation fails"
            ),
            "errors": [],
        }
        payload["errors"] = validate_payload(payload)
        (out_dir / "battleplan_army_scalar_probe.json").write_text(
            json.dumps(payload, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        return payload
    finally:
        proc.close()


def melt_save(save_path: Path, rakaly_path: Path | None, out_dir: Path) -> tuple[str, dict[str, Any]]:
    rakaly = rakaly_path or ps.find_rakaly(None)
    text, was_melted = ps.melt_if_needed(save_path, rakaly, out_dir)
    melted_path = out_dir / "autosave_melted.txt"
    melted_path.write_text(text, encoding="utf-8", errors="replace")
    return text, {
        "path": str(save_path),
        "rakaly": str(rakaly) if rakaly else None,
        "was_melted": bool(was_melted),
        "melted_path": str(melted_path),
        "text_len": len(text),
    }


def parse_orders_group_scalars(text: str) -> dict[int, dict[str, int]]:
    armies: dict[int, dict[str, int]] = {}
    pos = 0
    needle = "orders_group="
    while True:
        idx = text.find(needle, pos)
        if idx < 0:
            break
        brace = text.find("{", idx + len(needle))
        if brace < 0:
            break
        end = find_matching_brace(text, brace)
        if end is None:
            break
        block = text[brace + 1:end]
        pos = end + 1
        row = parse_orders_group_block(block)
        if row is None:
            continue
        armies[row["army_id"]] = {field: row[field] for field in FIELDS if field in row}
    return armies


def parse_orders_group_block(block: str) -> dict[str, int] | None:
    army_id = parse_id_type_53(block)
    if army_id is None:
        return None
    row: dict[str, int] = {"army_id": army_id}
    for field in FIELDS:
        value = parse_scalar_int(block, field)
        if value is not None:
            row[field] = value
    if not any(field in row for field in FIELDS):
        return None
    return row


def parse_id_type_53(block: str) -> int | None:
    match = re.search(r"\bid\s*=\s*\{\s*id\s*=\s*(\d+)\s+type\s*=\s*53\s*\}", block)
    if not match:
        return None
    return int(match.group(1))


def parse_scalar_int(block: str, field: str) -> int | None:
    match = re.search(rf"\b{re.escape(field)}\s*=\s*(-?\d+)\b", block)
    if not match:
        return None
    return int(match.group(1))


def find_matching_brace(text: str, open_index: int) -> int | None:
    depth = 0
    in_quote = False
    escaped = False
    for index in range(open_index, len(text)):
        ch = text[index]
        if in_quote:
            if escaped:
                escaped = False
            elif ch == "\\":
                escaped = True
            elif ch == '"':
                in_quote = False
            continue
        if ch == '"':
            in_quote = True
            continue
        if ch == "{":
            depth += 1
        elif ch == "}":
            depth -= 1
            if depth == 0:
                return index
    return None


def read_live_army_bytes(
    proc: Any,
    army_roots: dict[int, int],
    matched_ids: list[int],
    scan_begin: int,
    scan_end: int,
) -> dict[int, dict[str, Any]]:
    out: dict[int, dict[str, Any]] = {}
    size = scan_end - scan_begin
    for army_id in matched_ids:
        addr = army_roots[army_id]
        root_id = direct_oob_probe.validate_root_hit(proc, "army", addr)
        if root_id != army_id:
            continue
        data = proc.try_read(addr + scan_begin, size)
        if len(data) != size:
            continue
        out[army_id] = {"addr": addr, "bytes": data}
    return out


def evaluate_field(
    field: str,
    save_armies: dict[int, dict[str, int]],
    live_rows: dict[int, dict[str, Any]],
    scan_begin: int,
    scan_end: int,
) -> dict[str, Any]:
    expected_by_army = {
        army_id: row[field]
        for army_id, row in save_armies.items()
        if field in row and army_id in live_rows
    }
    save_hist = Counter(expected_by_army.values())
    u8_candidates = []
    u32_candidates = []
    for offset in range(scan_begin, scan_end):
        u8_candidates.append(score_offset(expected_by_army, live_rows, offset, scan_begin, "u8"))
    for offset in range(scan_begin, scan_end - 3, 4):
        u32_candidates.append(score_offset(expected_by_army, live_rows, offset, scan_begin, "u32"))
    exact = [
        row
        for row in (*u8_candidates, *u32_candidates)
        if row["matched"] == row["tested"]
        and row["tested"] >= MIN_MATCHED_ARMIES
        and row["candidate_distinct_values"] >= 2
        and len(save_hist) >= 2
    ]
    exact.sort(key=lambda row: (row["width"], row["offset"]))
    near = [
        row
        for row in (*u8_candidates, *u32_candidates)
        if row["tested"] >= MIN_MATCHED_ARMIES and row["match_fraction"] >= NEAR_MATCH_FRACTION
    ]
    near.sort(key=lambda row: (row["match_fraction"], row["matched"]), reverse=True)
    if len(save_hist) < 2:
        status = "INSUFFICIENT_SAVE_VARIATION"
    elif len(exact) == 1:
        status = "VALIDATED_INLINE_ARMY_SCALAR"
    elif len(exact) > 1:
        status = "AMBIGUOUS_INLINE_ARMY_SCALAR"
    elif near:
        status = "NARROWED_INLINE_ARMY_SCALAR_NEAR_MATCH"
    else:
        status = "REJECTED_INLINE_ARMY_SCALAR_RANGE"
    return {
        "field": field,
        "status": status,
        "tested_armies": len(expected_by_army),
        "save_value_histogram": {str(key): save_hist[key] for key in sorted(save_hist)},
        "accepted_offsets": [summarize_score(row) for row in exact[:16]],
        "narrowed_offsets": [summarize_score(row) for row in near[:16]],
        "top_candidates": [summarize_score(row) for row in near[:16]],
        "rejected_range": {
            "begin": f"+0x{scan_begin:X}",
            "end_exclusive": f"+0x{scan_end:X}",
            "widths": ["u8", "u32"],
        },
    }


def score_offset(
    expected_by_army: dict[int, int],
    live_rows: dict[int, dict[str, Any]],
    offset: int,
    scan_begin: int,
    width: str,
) -> dict[str, Any]:
    tested = matched = 0
    values = Counter()
    sample = []
    rel = offset - scan_begin
    for army_id, expected in expected_by_army.items():
        data = live_rows[army_id]["bytes"]
        if width == "u8":
            value = data[rel]
        else:
            if rel + 4 > len(data):
                continue
            value = int.from_bytes(data[rel:rel + 4], "little", signed=False)
        tested += 1
        values[value] += 1
        if value == expected:
            matched += 1
        elif len(sample) < 8:
            sample.append({"army_id": army_id, "expected": expected, "actual": value})
    return {
        "offset": offset,
        "width": width,
        "tested": tested,
        "matched": matched,
        "match_fraction": (matched / tested) if tested else 0.0,
        "candidate_distinct_values": len(values),
        "value_histogram": {str(key): values[key] for key in sorted(values)[:16]},
        "mismatch_sample": sample,
    }


def summarize_score(row: dict[str, Any]) -> dict[str, Any]:
    return {
        "offset": f"+0x{row['offset']:X}",
        "width": row["width"],
        "tested": row["tested"],
        "matched": row["matched"],
        "match_fraction": row["match_fraction"],
        "candidate_distinct_values": row["candidate_distinct_values"],
        "value_histogram": row["value_histogram"],
        "mismatch_sample": row["mismatch_sample"],
    }


def classify_payload(field_results: dict[str, dict[str, Any]]) -> str:
    statuses = {row["status"] for row in field_results.values()}
    if "VALIDATED_INLINE_ARMY_SCALAR" in statuses:
        return "VALIDATED_ARMY_SCALAR_SLICE"
    if "AMBIGUOUS_INLINE_ARMY_SCALAR" in statuses:
        return "AMBIGUOUS_ARMY_SCALAR_SLICE"
    if "NARROWED_INLINE_ARMY_SCALAR_NEAR_MATCH" in statuses:
        return "NARROWED_ARMY_SCALAR_NEAR_MATCH"
    if statuses == {"REJECTED_INLINE_ARMY_SCALAR_RANGE"}:
        return "REJECTED_ARMY_SCALAR_RANGE"
    return "PARTIAL_ARMY_SCALAR_PROBE"


def validate_payload(payload: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    if payload.get("matched_armies", 0) < MIN_MATCHED_ARMIES:
        errors.append("not enough save/live matched armies")
    for field, row in payload.get("field_results", {}).items():
        if row.get("tested_armies", 0) < MIN_MATCHED_ARMIES:
            errors.append(f"{field} tested too few armies")
        if row.get("status") == "VALIDATED_INLINE_ARMY_SCALAR" and len(row.get("accepted_offsets", [])) != 1:
            errors.append(f"{field} validated status requires exactly one accepted offset")
    return errors


def default_autosave_path() -> Path:
    return Path.home() / "OneDrive" / "Documents" / "Paradox Interactive" / "Hearts of Iron IV" / "save games" / "autosave.hoi4"


def parse_int(value: str) -> int:
    return int(value, 0)


if __name__ == "__main__":
    raise SystemExit(main())
