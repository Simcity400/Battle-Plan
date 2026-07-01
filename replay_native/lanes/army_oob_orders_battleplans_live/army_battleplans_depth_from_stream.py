"""Build a lane-local domain-54 proof stream from validated army_orders_live data.

Input is a compact stream that already contains production DLL domain 27
`army_orders_live` records. Output is a lane-local compact stream containing
domain 54 `army_battleplans_depth_live` records plus frame records. This is a
proof/conversion tool only; it does not edit shared collector/recorder
entrypoints.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import struct
from typing import Any


MAGIC = b"H4CR"
HEADER_LEN = 32
VERSION = 1
RECORD_TYPE_FRAME_END = 2
RECORD_TYPE_DOMAIN_DATA = 3
DOMAIN_ARMY_ORDERS = 27
DOMAIN_ARMY_BATTLEPLANS_DEPTH = 54
DOMAIN_FRAME = 1000

ARMY_ORDER_ROW_LEN = 48
ARMY_ORDER_TARGET_ROW_LEN = 8
DEPTH_HEADER_LEN = 40
DEPTH_ROW_LEN = 64
DEPTH_TARGET_ROW_LEN = 8

ARMY_ORDER_FLAG_DELETED = 1
ARMY_ORDER_FLAG_TARGET_STATES = 1 << 1
ARMY_ORDER_FLAG_TARGET_STATE_COUNT = 1 << 2

DEPTH_FLAG_DELETED = 1
DEPTH_FLAG_TARGET_STATES = 1 << 1
DEPTH_FLAG_TARGET_STATE_COUNT = 1 << 2
DEPTH_FLAG_NON_TYPE5_GEOMETRY_BLOCKED = 1 << 3
DEPTH_FLAG_EXECUTION_BLOCKED = 1 << 4

STATUS_NONE = 0
STATUS_EXACT = 1
STATUS_UNSUPPORTED = 3
STATUS_FIXTURE_REQUIRED = 6


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("input_stream", type=Path)
    ap.add_argument("--out-stream", type=Path, required=True)
    ap.add_argument("--summary", type=Path, required=True)
    args = ap.parse_args()

    source = args.input_stream.read_bytes()
    output, summary = convert_stream(source)
    args.out_stream.parent.mkdir(parents=True, exist_ok=True)
    args.out_stream.write_bytes(output)
    args.summary.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({"status": "PASS", **summary}, indent=2, sort_keys=True))
    return 0


def convert_stream(source: bytes) -> tuple[bytes, dict[str, Any]]:
    records = list(iter_records(source))
    out = bytearray()
    seq = 1
    summary: dict[str, Any] = {
        "schema": "army_battleplans_depth_from_army_orders_v1",
        "source_domain": DOMAIN_ARMY_ORDERS,
        "output_domain": DOMAIN_ARMY_BATTLEPLANS_DEPTH,
        "frame_records": 0,
        "depth_records": 0,
        "rows": 0,
        "type5_exact_rows": 0,
        "type5_non_exact_omitted_rows": 0,
        "non_type5_fixture_blocked_rows": 0,
        "execution_fixture_blocked_rows": 0,
        "target_state_rows": 0,
        "status_flags_or": 0,
        "hours": [],
    }
    seen_hours: set[int] = set()
    for record in records:
        if record["record_type"] == RECORD_TYPE_DOMAIN_DATA and record["domain_id"] == DOMAIN_ARMY_ORDERS:
            payload, stats = convert_army_orders_payload(record["payload"])
            out.extend(write_record(seq, record["game_hour_raw"], DOMAIN_ARMY_BATTLEPLANS_DEPTH, RECORD_TYPE_DOMAIN_DATA, payload))
            seq += 1
            summary["depth_records"] += 1
            for key in (
                "rows",
                "type5_exact_rows",
                "type5_non_exact_omitted_rows",
                "non_type5_fixture_blocked_rows",
                "execution_fixture_blocked_rows",
                "target_state_rows",
            ):
                summary[key] += stats[key]
            summary["status_flags_or"] |= stats["status_flags"]
        elif record["record_type"] == RECORD_TYPE_FRAME_END:
            out.extend(write_record(seq, record["game_hour_raw"], DOMAIN_FRAME, RECORD_TYPE_FRAME_END, record["payload"]))
            seq += 1
            summary["frame_records"] += 1
            seen_hours.add(record["game_hour_raw"])
    summary["hours"] = [min(seen_hours), max(seen_hours)] if seen_hours else []
    summary["unique_frame_hours"] = len(seen_hours)
    summary["game_hours_advanced"] = (max(seen_hours) - min(seen_hours)) if seen_hours else 0
    summary["zero_skipped_hours"] = (
        len(seen_hours) == summary["game_hours_advanced"] + 1 if seen_hours else False
    )
    return bytes(out), summary


def convert_army_orders_payload(payload: bytes) -> tuple[bytes, dict[str, int]]:
    if len(payload) < 40:
        raise ValueError("army_orders payload shorter than 40 bytes")
    elapsed_ms = u64(payload, 0)
    row_count = u32(payload, 8)
    changed_rows = u32(payload, 12)
    status_flags = u32(payload, 16)
    observed_army_rows = u32(payload, 24)
    observed_order_rows = u32(payload, 28)
    target_state_count = u32(payload, 32)
    expected_len = 40 + row_count * ARMY_ORDER_ROW_LEN + target_state_count * ARMY_ORDER_TARGET_ROW_LEN
    if len(payload) != expected_len:
        raise ValueError(f"army_orders payload length {len(payload)} != {expected_len}")
    target_table = []
    target_cursor = 40 + row_count * ARMY_ORDER_ROW_LEN
    for _ in range(target_state_count):
        target_table.append(u32(payload, target_cursor))
        target_cursor += ARMY_ORDER_TARGET_ROW_LEN

    rows: list[dict[str, Any]] = []
    stats = {
        "rows": 0,
        "type5_exact_rows": 0,
        "type5_non_exact_omitted_rows": 0,
        "non_type5_fixture_blocked_rows": 0,
        "execution_fixture_blocked_rows": 0,
        "target_state_rows": 0,
        "status_flags": status_flags,
    }
    cursor = 40
    for _ in range(row_count):
        army_id = u32(payload, cursor)
        generation = u64(payload, cursor + 4)
        flags = u32(payload, cursor + 12)
        slot_index = u32(payload, cursor + 16)
        order_type = i32(payload, cursor + 20)
        target_first = u32(payload, cursor + 24)
        target_count = u32(payload, cursor + 28)
        target_declared = u32(payload, cursor + 32)
        target_status = u32(payload, cursor + 36)
        cursor += ARMY_ORDER_ROW_LEN

        deleted = (flags & ARMY_ORDER_FLAG_DELETED) != 0
        target_states: list[int] = []
        if flags & ARMY_ORDER_FLAG_TARGET_STATES:
            target_states = target_table[target_first : target_first + target_count]
        if order_type == 5 and not deleted and target_status != STATUS_EXACT:
            stats["type5_non_exact_omitted_rows"] += 1
            continue
        geometry_status = STATUS_EXACT if order_type == 5 and not deleted else STATUS_FIXTURE_REQUIRED
        if order_type != 5 and not deleted:
            stats["non_type5_fixture_blocked_rows"] += 1
        execution_status = STATUS_NONE if deleted else STATUS_FIXTURE_REQUIRED
        if not deleted:
            stats["execution_fixture_blocked_rows"] += 1
        if order_type == 5 and target_status == STATUS_EXACT and not deleted:
            stats["type5_exact_rows"] += 1
        rows.append(
            {
                "army_id": army_id,
                "generation": generation,
                "deleted": deleted,
                "slot_index": slot_index,
                "order_type": order_type,
                "target_states": target_states,
                "target_declared": target_declared if flags & ARMY_ORDER_FLAG_TARGET_STATE_COUNT else None,
                "target_status": target_status if order_type == 5 and not deleted else STATUS_UNSUPPORTED,
                "geometry_status": geometry_status,
                "execution_status": execution_status,
            }
        )
        stats["rows"] += 1
        stats["target_state_rows"] += len(target_states)

    depth_payload = encode_depth_payload(
        rows,
        elapsed_ms,
        status_flags,
        min(changed_rows, len(rows)),
        observed_army_rows,
        observed_order_rows,
    )
    return depth_payload, stats


def encode_depth_payload(
    rows: list[dict[str, Any]],
    elapsed_ms: int,
    status_flags: int,
    changed_rows: int,
    observed_army_rows: int,
    observed_order_rows: int,
) -> bytes:
    target_count = sum(len(row["target_states"]) for row in rows)
    unsupported_geometry = sum(
        1 for row in rows if not row["deleted"] and row["geometry_status"] == STATUS_FIXTURE_REQUIRED
    )
    execution_unknown = sum(
        1 for row in rows if not row["deleted"] and row["execution_status"] == STATUS_FIXTURE_REQUIRED
    )
    out = bytearray()
    out += struct.pack(
        "<QIIIIIIII",
        elapsed_ms,
        len(rows),
        changed_rows,
        status_flags,
        observed_army_rows,
        observed_order_rows,
        target_count,
        unsupported_geometry,
        execution_unknown,
    )
    target_cursor = 0
    for row in rows:
        flags = 0
        if row["deleted"]:
            flags |= DEPTH_FLAG_DELETED
        if row["target_states"]:
            flags |= DEPTH_FLAG_TARGET_STATES
        if row["target_declared"] is not None:
            flags |= DEPTH_FLAG_TARGET_STATE_COUNT
        if row["geometry_status"] == STATUS_FIXTURE_REQUIRED:
            flags |= DEPTH_FLAG_NON_TYPE5_GEOMETRY_BLOCKED
        if row["execution_status"] == STATUS_FIXTURE_REQUIRED:
            flags |= DEPTH_FLAG_EXECUTION_BLOCKED
        target_first = target_cursor if row["target_states"] else 0
        out += struct.pack(
            "<IQIIiIIIIIIIIII",
            row["army_id"],
            row["generation"],
            flags,
            row["slot_index"],
            row["order_type"],
            target_first,
            len(row["target_states"]),
            row["target_declared"] or 0,
            row["target_status"],
            row["geometry_status"],
            row["execution_status"],
            0,
            0,
            0,
            0,
        )
        target_cursor += len(row["target_states"])
    for row in rows:
        for state_id in row["target_states"]:
            out += struct.pack("<II", state_id, 0)
    return bytes(out)


def iter_records(data: bytes):
    offset = 0
    while offset < len(data):
        if len(data) - offset < HEADER_LEN:
            raise ValueError(f"truncated record header at {offset}")
        header = data[offset : offset + HEADER_LEN]
        if header[:4] != MAGIC:
            raise ValueError(f"bad compact magic at {offset}")
        version, header_len = struct.unpack_from("<HH", header, 4)
        if version != VERSION or header_len != HEADER_LEN:
            raise ValueError(f"bad compact header at {offset}")
        sequence = u64(header, 8)
        game_hour_raw = i32(header, 16)
        domain_id = struct.unpack_from("<H", header, 20)[0]
        record_type = struct.unpack_from("<H", header, 22)[0]
        payload_len = u32(header, 24)
        start = offset + HEADER_LEN
        end = start + payload_len
        if end > len(data):
            raise ValueError(f"payload exceeds stream at {offset}")
        yield {
            "offset": offset,
            "sequence": sequence,
            "game_hour_raw": game_hour_raw,
            "domain_id": domain_id,
            "record_type": record_type,
            "payload": data[start:end],
        }
        offset = end


def write_record(sequence: int, game_hour_raw: int, domain_id: int, record_type: int, payload: bytes) -> bytes:
    return (
        MAGIC
        + struct.pack("<HHQiHHII", VERSION, HEADER_LEN, sequence, game_hour_raw, domain_id, record_type, len(payload), 0)
        + payload
    )


def u32(data: bytes, offset: int) -> int:
    return struct.unpack_from("<I", data, offset)[0]


def i32(data: bytes, offset: int) -> int:
    return struct.unpack_from("<i", data, offset)[0]


def u64(data: bytes, offset: int) -> int:
    return struct.unpack_from("<Q", data, offset)[0]


if __name__ == "__main__":
    raise SystemExit(main())
