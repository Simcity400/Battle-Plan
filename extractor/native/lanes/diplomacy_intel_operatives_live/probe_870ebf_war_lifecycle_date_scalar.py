#!/usr/bin/env python3
"""War lifecycle date-scalar candidate probe for domain 53.

The current 870ebf war root is a validated tag-pair/casualty row pool with an
inline ``CGameDate``/``CGregorianDate`` vtable at row+0x08. This bounded probe
tests whether the inline date object carries a DateHour-compatible start-date
scalar at fixed offsets before the casualty fields.

It is read-only and lane-local. It consumes an existing war-shape proof for row
addresses, live-validates those addresses against hoi4.exe, then scores exact
row offsets. It does not touch shared collector or recorder entrypoints.
"""

from __future__ import annotations

import argparse
import json
import struct
import sys
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


LANE_DIR = Path(__file__).resolve().parent
if str(LANE_DIR) not in sys.path:
    sys.path.insert(0, str(LANE_DIR))

import probe_870ebf_diplomacy as dip  # type: ignore
import probe_870ebf_war_shape as war_shape  # type: ignore
import winmem  # type: ignore


DATE_EPOCH_CONST = 43_799_999
GAMESTATE_CURRENT_DATE = 0x468
GAMESTATE_START_DATE = 0x4A8
ROW_READ_BYTES = 0x80
INLINE_DATE_OBJECT_ROW_OFFSET = 0x08
DEFAULT_ROW_OFFSETS = list(range(0x10, 0x40, 4))


def load_json(path: Path) -> dict[str, Any]:
    with path.open("r", encoding="utf-8") as f:
        value = json.load(f)
    if not isinstance(value, dict):
        raise ValueError(f"{path} did not contain a JSON object")
    return value


def decode_datehour(stored: int | None) -> str | None:
    if stored is None:
        return None
    min_value = (1850 * 365 * 24) + DATE_EPOCH_CONST
    max_value = (2200 * 365 * 24) + DATE_EPOCH_CONST
    if stored < min_value or stored > max_value:
        return None
    hrs = stored - DATE_EPOCH_CONST
    hour = hrs % 24
    rem_days = hrs // 24
    year = rem_days // 365
    doy = rem_days - year * 365
    month_lengths = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    month = 1
    for length in month_lengths:
        if doy < length:
            break
        doy -= length
        month += 1
    return f"{year}.{month}.{doy + 1}.{hour}"


def parse_hex(value: Any) -> int | None:
    if isinstance(value, int):
        return value
    if isinstance(value, str):
        try:
            return int(value, 16)
        except ValueError:
            return None
    return None


def i32_from(blob: bytes, offset: int) -> int | None:
    if offset < 0 or offset + 4 > len(blob):
        return None
    return struct.unpack_from("<i", blob, offset)[0]


def u32_from(blob: bytes, offset: int) -> int | None:
    if offset < 0 or offset + 4 > len(blob):
        return None
    return struct.unpack_from("<I", blob, offset)[0]


def u64_from(blob: bytes, offset: int) -> int | None:
    if offset < 0 or offset + 8 > len(blob):
        return None
    return struct.unpack_from("<Q", blob, offset)[0]


def current_date_summary(p: winmem.Process, roots: dict[str, Any]) -> dict[str, Any]:
    game_state = parse_hex(roots.get("game_state"))
    if not game_state:
        return {
            "game_state": roots.get("game_state"),
            "current_raw": None,
            "current_decoded": None,
            "start_raw": None,
            "start_decoded": None,
        }
    current_raw = dip.i32(p, game_state + GAMESTATE_CURRENT_DATE)
    start_raw = dip.i32(p, game_state + GAMESTATE_START_DATE)
    return {
        "game_state": "0x%X" % game_state,
        "current_offset": "0x%X" % GAMESTATE_CURRENT_DATE,
        "current_raw": current_raw,
        "current_decoded": decode_datehour(current_raw),
        "start_offset": "0x%X" % GAMESTATE_START_DATE,
        "start_raw": start_raw,
        "start_decoded": decode_datehour(start_raw),
    }


def row_source_rows(war_proof: dict[str, Any], max_rows: int) -> list[dict[str, Any]]:
    rows = war_proof.get("scan", {}).get("current_date_rows", [])
    if not isinstance(rows, list):
        return []
    out = [row for row in rows if isinstance(row, dict)]
    if max_rows > 0:
        return out[:max_rows]
    return out


def live_validate_rows(
    p: winmem.Process,
    rows: list[dict[str, Any]],
    current_date_vtables: dict[int, str],
    *,
    max_samples: int,
) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    valid_rows: list[dict[str, Any]] = []
    invalid_by_reason: dict[str, int] = {}
    invalid_samples: list[dict[str, Any]] = []

    def add_invalid(row: dict[str, Any], reason: str, extra: dict[str, Any] | None = None) -> None:
        invalid_by_reason[reason] = invalid_by_reason.get(reason, 0) + 1
        if len(invalid_samples) < max_samples:
            sample = {
                "reason": reason,
                "row_ptr": row.get("row_ptr"),
                "first_tag": row.get("first_tag"),
                "second_tag": row.get("second_tag"),
            }
            if extra:
                sample.update(extra)
            invalid_samples.append(sample)

    for row in rows:
        row_ptr = parse_hex(row.get("row_ptr"))
        if row_ptr is None:
            add_invalid(row, "missing_row_ptr")
            continue
        blob = p.try_read(row_ptr, ROW_READ_BYTES)
        if len(blob) < ROW_READ_BYTES:
            add_invalid(row, "unreadable")
            continue
        first_id = u32_from(blob, dip.WR_FIRST_TAGID)
        second_id = u32_from(blob, dip.WR_SECOND_TAGID)
        if first_id != row.get("first_id") or second_id != row.get("second_id"):
            add_invalid(
                row,
                "tag_pair_changed",
                {"live_first_id": first_id, "live_second_id": second_id},
            )
            continue
        if (
            u32_from(blob, dip.WR_ECHO) != first_id
            or u32_from(blob, dip.WR_ECHO + 4) != second_id
        ):
            add_invalid(row, "echo_mismatch")
            continue
        typeptr = u64_from(blob, dip.WR_TYPEPTR)
        type_name = current_date_vtables.get(typeptr or 0)
        if not type_name:
            add_invalid(row, "date_vtable_changed", {"typeptr": dip.hx(typeptr)})
            continue
        first_cas = i32_from(blob, dip.WR_FIRST_CAS)
        second_cas = i32_from(blob, dip.WR_SECOND_CAS)
        if not (
            isinstance(first_cas, int)
            and isinstance(second_cas, int)
            and 0 <= first_cas <= dip.WR_MAX_CASUALTIES
            and 0 <= second_cas <= dip.WR_MAX_CASUALTIES
        ):
            add_invalid(
                row,
                "casualty_bounds",
                {"first_casualties_i32": first_cas, "second_casualties_i32": second_cas},
            )
            continue
        valid_rows.append(
            {
                "row_ptr": row_ptr,
                "row_ptr_hex": "0x%X" % row_ptr,
                "first_id": first_id,
                "first_tag": row.get("first_tag"),
                "second_id": second_id,
                "second_tag": row.get("second_tag"),
                "typeptr": typeptr,
                "typeptr_rva": "0x%X" % (typeptr - p.module_base),
                "typeptr_class": type_name,
                "first_casualties_i32": first_cas,
                "second_casualties_i32": second_cas,
                "blob": blob,
            }
        )

    return valid_rows, {
        "source_rows": len(rows),
        "checked_rows": len(rows),
        "valid_rows": len(valid_rows),
        "invalid_rows": len(rows) - len(valid_rows),
        "invalid_by_reason": invalid_by_reason,
        "invalid_samples": invalid_samples,
        "status": "validated" if rows and len(valid_rows) == len(rows) else "partial_or_failed",
    }


def score_offsets(
    rows: list[dict[str, Any]],
    row_offsets: list[int],
    current_raw: int | None,
    start_raw: int | None,
    *,
    max_samples: int,
) -> list[dict[str, Any]]:
    scores: list[dict[str, Any]] = []
    for row_offset in row_offsets:
        decoded_count = 0
        lte_current_count = 0
        gte_start_count = 0
        equals_current_count = 0
        equals_game_start_count = 0
        raw_values: set[int] = set()
        decoded_values: set[str] = set()
        samples: list[dict[str, Any]] = []
        for row in rows:
            raw_value = i32_from(row["blob"], row_offset)
            decoded = decode_datehour(raw_value)
            if decoded is not None:
                decoded_count += 1
                decoded_values.add(decoded)
                raw_values.add(int(raw_value))
            if (
                isinstance(raw_value, int)
                and isinstance(current_raw, int)
                and raw_value <= current_raw
            ):
                lte_current_count += 1
            if (
                isinstance(raw_value, int)
                and isinstance(start_raw, int)
                and raw_value >= start_raw
            ):
                gte_start_count += 1
            if isinstance(raw_value, int) and raw_value == current_raw:
                equals_current_count += 1
            if isinstance(raw_value, int) and raw_value == start_raw:
                equals_game_start_count += 1
            if len(samples) < max_samples:
                samples.append(
                    {
                        "row_ptr": row["row_ptr_hex"],
                        "first_tag": row.get("first_tag"),
                        "second_tag": row.get("second_tag"),
                        "raw_i32": raw_value,
                        "decoded_datehour": decoded,
                    }
                )
        row_count = len(rows)
        accepted = bool(
            row_count > 0
            and decoded_count == row_count
            and lte_current_count == row_count
            and gte_start_count == row_count
            and equals_current_count != row_count
            and equals_game_start_count != row_count
        )
        scores.append(
            {
                "row_offset": "0x%X" % row_offset,
                "date_object_relative_offset": "0x%X"
                % (row_offset - INLINE_DATE_OBJECT_ROW_OFFSET),
                "checked_rows": row_count,
                "datehour_decodable_rows": decoded_count,
                "lte_current_date_rows": lte_current_count,
                "gte_game_start_date_rows": gte_start_count,
                "equals_current_date_rows": equals_current_count,
                "equals_game_start_date_rows": equals_game_start_count,
                "distinct_raw_values": len(raw_values),
                "distinct_decoded_values": len(decoded_values),
                "sample_values": samples,
                "status": "candidate_supported" if accepted else "rejected_by_invariants",
                "rejection_reason": None
                if accepted
                else (
                    "not every live-validated row has a DateHour scalar between "
                    "game start and current date, or the value collapses to the "
                    "global current/start date rather than row lifecycle detail"
                ),
            }
        )
    return scores


def warm_validate_scores(
    p: winmem.Process,
    rows: list[dict[str, Any]],
    candidates: list[dict[str, Any]],
    delay_seconds: float,
    *,
    max_samples: int,
) -> dict[str, Any]:
    time.sleep(max(0.0, delay_seconds))
    checked = 0
    stable = 0
    invalid: list[dict[str, Any]] = []
    for candidate in candidates:
        if candidate.get("status") != "candidate_supported":
            continue
        row_offset = parse_hex(candidate.get("row_offset"))
        if row_offset is None:
            continue
        checked += 1
        changed_rows = 0
        for row in rows:
            blob = p.try_read(row["row_ptr"], ROW_READ_BYTES)
            old_value = i32_from(row["blob"], row_offset)
            new_value = i32_from(blob, row_offset) if len(blob) >= ROW_READ_BYTES else None
            if old_value != new_value:
                changed_rows += 1
                if len(invalid) < max_samples:
                    invalid.append(
                        {
                            "row_offset": candidate.get("row_offset"),
                            "row_ptr": row["row_ptr_hex"],
                            "first_tag": row.get("first_tag"),
                            "second_tag": row.get("second_tag"),
                            "old_raw_i32": old_value,
                            "new_raw_i32": new_value,
                        }
                    )
        candidate["warm_changed_rows"] = changed_rows
        if changed_rows == 0:
            candidate["warm_status"] = "stable"
            stable += 1
        else:
            candidate["warm_status"] = "changed"
            candidate["status"] = "rejected_by_warm_validation"
    return {
        "delay_seconds": delay_seconds,
        "candidate_offsets_checked": checked,
        "stable_candidate_offsets": stable,
        "changed_candidate_offsets": checked - stable,
        "changed_samples": invalid,
        "status": "validated" if checked > 0 and checked == stable else "no_stable_candidate",
    }


def build_report(args: argparse.Namespace) -> tuple[dict[str, Any], int]:
    war_proof_path = Path(args.war_proof)
    war_proof = load_json(war_proof_path)
    p = winmem.open_process("hoi4.exe")
    if p is None:
        raise RuntimeError("hoi4.exe not running")
    with p:
        started = time.time()
        process = dip.process_info(p)
        report: dict[str, Any] = {
            "schema": "battle-plan.diplomacy_intel_operatives.war_lifecycle_date_scalar_probe.v1",
            "lane": "diplomacy_intel_operatives_live",
            "domain_id": 53,
            "probe": "probe_870ebf_war_lifecycle_date_scalar",
            "generated_utc": datetime.now(timezone.utc).isoformat(timespec="seconds"),
            "process": process,
            "expected_sha256": dip.EXPECTED_SHA256,
            "build_gate_ok": process.get("sha256") == dip.EXPECTED_SHA256,
            "war_proof": {
                "path": str(war_proof_path),
                "status": war_proof.get("status"),
                "accepted_for_production_war_root": war_proof.get(
                    "accepted_for_production_war_root"
                ),
            },
        }
        if not report["build_gate_ok"]:
            report["status"] = "build_gate_failed"
            return report, 2
        if not war_proof.get("accepted_for_production_war_root"):
            report["status"] = "war_root_proof_not_accepted"
            return report, 2

        roots = dip.read_roots(p)
        report["roots"] = roots
        report["game_dates"] = current_date_summary(p, roots)
        date_rtti = war_shape.derive_date_rtti(p)
        current_date_vtables = {
            int(value): name for value, name in date_rtti.get("vtable_names", {}).items()
        }
        report["date_rtti"] = {
            "classes": date_rtti.get("classes"),
            "module_chunk_count": date_rtti.get("module_chunk_count"),
            "module_bytes_read": date_rtti.get("module_bytes_read"),
        }
        source_rows = row_source_rows(war_proof, max(0, args.max_rows))
        live_rows, live_report = live_validate_rows(
            p, source_rows, current_date_vtables, max_samples=max(0, args.max_samples)
        )
        report["live_row_validation"] = live_report
        row_offsets = DEFAULT_ROW_OFFSETS
        report["tested_candidate_offsets"] = [
            {
                "row_offset": "0x%X" % value,
                "date_object_relative_offset": "0x%X"
                % (value - INLINE_DATE_OBJECT_ROW_OFFSET),
            }
            for value in row_offsets
        ]
        report["offset_scores"] = score_offsets(
            live_rows,
            row_offsets,
            report["game_dates"].get("current_raw"),
            report["game_dates"].get("start_raw"),
            max_samples=max(0, args.max_samples),
        )
        report["warm_validation"] = warm_validate_scores(
            p,
            live_rows,
            report["offset_scores"],
            max(0, args.warm_delay_ms) / 1000.0,
            max_samples=max(0, args.max_samples),
        )
        supported = [
            row
            for row in report["offset_scores"]
            if row.get("status") == "candidate_supported"
            and row.get("warm_status") in (None, "stable")
        ]
        report["summary"] = {
            "source_rows": len(source_rows),
            "live_valid_rows": live_report.get("valid_rows"),
            "candidate_offsets_tested": len(row_offsets),
            "supported_offsets_after_warm": [
                row.get("row_offset") for row in supported
            ],
            "rejected_offsets": [
                row.get("row_offset")
                for row in report["offset_scores"]
                if row.get("status") != "candidate_supported"
            ],
            "backlog_item_narrowed": "war_lifecycle_detail",
        }
        if supported:
            report["status"] = "inline_date_scalar_candidate_supported"
            report["remaining_blocker"] = (
                "candidate DateHour scalar offset still needs same-tick UI/save "
                "war lifecycle evidence before naming it as war start date"
            )
            code = 0
        elif live_report.get("valid_rows", 0) > 0:
            report["status"] = "inline_date_scalar_offsets_rejected"
            report["rejection"] = {
                "candidate": "inline DateHour scalar inside accepted war row date object",
                "row_offsets_rejected": report["summary"]["rejected_offsets"],
                "date_object_relative_offsets_rejected": [
                    entry["date_object_relative_offset"]
                    for entry in report["tested_candidate_offsets"]
                    if entry["row_offset"] in report["summary"]["rejected_offsets"]
                ],
                "reason": (
                    "No tested offset across row+0x10..row+0x3C produced a stable "
                    "DateHour scalar between game start and current date on every "
                    "live-validated row."
                ),
            }
            code = 0
        else:
            report["status"] = "no_live_valid_war_rows"
            code = 2
        report["elapsed_seconds"] = round(time.time() - started, 3)
        return report, code


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--war-proof", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--max-rows", type=int, default=512)
    ap.add_argument("--max-samples", type=int, default=12)
    ap.add_argument("--warm-delay-ms", type=int, default=250)
    args = ap.parse_args(argv)

    report, code = build_report(args)
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    with out.open("w", encoding="utf-8") as f:
        json.dump(report, f, indent=2, sort_keys=True)
        f.write("\n")
    print(
        "probe_870ebf_war_lifecycle_date_scalar: "
        f"status={report['status']} out={out}"
    )
    return code


if __name__ == "__main__":
    raise SystemExit(main())
