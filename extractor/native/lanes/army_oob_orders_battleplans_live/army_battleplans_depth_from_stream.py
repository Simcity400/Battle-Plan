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
DOMAIN_OOB_COMMAND = 28
DOMAIN_ARMY_BATTLEPLANS_DEPTH = 54
DOMAIN_FRAME = 1000

ARMY_ORDER_ROW_LEN = 48
ARMY_ORDER_TARGET_ROW_LEN = 8
DEPTH_HEADER_LEN = 52
DEPTH_ROW_LEN = 64
DEPTH_TARGET_ROW_LEN = 8
DEPTH_PATH_GEOMETRY_ROW_LEN = 8
DEPTH_DIRECT_ROOT_ROW_LEN = 64
DEPTH_DIRECT_ROOT_MEMBER_ROW_LEN = 8

ARMY_ORDER_FLAG_DELETED = 1
ARMY_ORDER_FLAG_TARGET_STATES = 1 << 1
ARMY_ORDER_FLAG_TARGET_STATE_COUNT = 1 << 2

DEPTH_FLAG_DELETED = 1
DEPTH_FLAG_TARGET_STATES = 1 << 1
DEPTH_FLAG_TARGET_STATE_COUNT = 1 << 2
DEPTH_FLAG_NON_TYPE5_GEOMETRY_BLOCKED = 1 << 3
DEPTH_FLAG_EXECUTION_BLOCKED = 1 << 4
DEPTH_FLAG_PATH_GEOMETRY = 1 << 5

STATUS_NONE = 0
STATUS_EXACT = 1
STATUS_UNKNOWN = 2
STATUS_UNSUPPORTED = 3
STATUS_VALIDATION_FAILED = 5
STATUS_FIXTURE_REQUIRED = 6
STATUS_COLD_PREFLIGHT_ONLY = 7

OOB_ROW_LEN = 80
OOB_MEMBER_ROW_LEN = 8
OOB_KIND_ARMY = 1
OOB_FLAG_DELETED = 1

DIRECT_KIND_ARMY = 1
DIRECT_KIND_ARMY_GROUP = 2
DIRECT_KIND_THEATRE = 5

DIRECT_FLAG_DELETED = 1
DIRECT_FLAG_PARENT = 1 << 1
DIRECT_FLAG_OWNER = 1 << 2
DIRECT_FLAG_ENGINE_MEMBER_COUNT = 1 << 3
DIRECT_FLAG_MISSING_MEMBER_COUNT = 1 << 4
DIRECT_FLAG_MEMBER_IDS = 1 << 5
DIRECT_FLAG_ORDER_COUNTS = 1 << 6


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("input_stream", type=Path)
    ap.add_argument("--direct-oob-proof", type=Path)
    ap.add_argument("--out-stream", type=Path, required=True)
    ap.add_argument("--summary", type=Path, required=True)
    args = ap.parse_args()

    source = args.input_stream.read_bytes()
    direct_roots = load_direct_roots(args.direct_oob_proof) if args.direct_oob_proof else []
    output, summary = convert_stream(source, direct_roots=direct_roots)
    args.out_stream.parent.mkdir(parents=True, exist_ok=True)
    args.out_stream.write_bytes(output)
    args.summary.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({"status": "PASS", **summary}, indent=2, sort_keys=True))
    return 0


def convert_stream(source: bytes, *, direct_roots: list[dict[str, Any]]) -> tuple[bytes, dict[str, Any]]:
    records = list(iter_records(source))
    assigned_units_by_hour = collect_assigned_unit_statuses(records)
    direct_member_rows = sum(len(row["member_ids"]) for row in direct_roots)
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
        "non_type5_exact_path_rows": 0,
        "path_geometry_rows": 0,
        "non_type5_fixture_blocked_rows": 0,
        "execution_fixture_blocked_rows": 0,
        "target_state_rows": 0,
        "assigned_units_exact_rows": 0,
        "assigned_units_missing_rows": 0,
        "order_linkage_exact_rows": 0,
        "stance_unknown_rows": 0,
        "aggressiveness_unknown_rows": 0,
        "direct_root_rows": 0,
        "direct_root_member_rows": 0,
        "direct_army_rows": 0,
        "direct_army_group_rows": 0,
        "direct_theatre_rows": 0,
        "direct_cold_preflight_rows": 0,
        "direct_root_unique_rows": len(direct_roots),
        "direct_root_unique_member_rows": direct_member_rows,
        "direct_root_unique_armies": sum(1 for row in direct_roots if row["kind"] == DIRECT_KIND_ARMY),
        "direct_root_unique_army_groups": sum(1 for row in direct_roots if row["kind"] == DIRECT_KIND_ARMY_GROUP),
        "direct_root_unique_theatres": sum(1 for row in direct_roots if row["kind"] == DIRECT_KIND_THEATRE),
        "status_flags_or": 0,
        "hours": [],
    }
    seen_hours: set[int] = set()
    for record in records:
        if record["record_type"] == RECORD_TYPE_DOMAIN_DATA and record["domain_id"] == DOMAIN_ARMY_ORDERS:
            payload, stats = convert_army_orders_payload(
                record["payload"],
                assigned_units_by_hour.get(record["game_hour_raw"], {}),
                direct_roots,
            )
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
                "assigned_units_exact_rows",
                "assigned_units_missing_rows",
                "order_linkage_exact_rows",
                "stance_unknown_rows",
                "aggressiveness_unknown_rows",
                "direct_root_rows",
                "direct_root_member_rows",
                "direct_army_rows",
                "direct_army_group_rows",
                "direct_theatre_rows",
                "direct_cold_preflight_rows",
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


def load_direct_roots(path: Path) -> list[dict[str, Any]]:
    proof = json.loads(path.read_text(encoding="utf-8"))
    if proof.get("schema") != "army_oob_orders_battleplans_direct_oob_probe_v1":
        raise ValueError("direct OOB proof has unexpected schema")
    summary = proof.get("summary", {})
    roots = summary.get("roots", {})
    army_samples = {row["id"]: row for row in summary.get("direct_armies_sample", [])}
    army_group_samples = {row["id"]: row for row in summary.get("army_groups_sample", [])}

    rows: list[dict[str, Any]] = []
    for root in roots.get("armies", []):
        object_id = int(root["id"])
        sample = army_samples.get(object_id, {})
        engine_count = optional_count_u32(sample.get("engine_member_count"))
        live_count = optional_count_u32(sample.get("live_division_member_count"))
        missing_count = optional_count_u32(sample.get("missing_member_count"))
        if engine_count is None and live_count is None:
            member_status = STATUS_UNKNOWN
        elif missing_count == 0:
            member_status = STATUS_EXACT
        elif missing_count is not None and missing_count > 0:
            member_status = STATUS_VALIDATION_FAILED
        else:
            member_status = STATUS_UNKNOWN
        rows.append(
            {
                "kind": DIRECT_KIND_ARMY,
                "object_id": object_id,
                "deleted": False,
                "parent_id": optional_positive_u32(sample.get("parent_army_group_id")),
                "owner_tag_db_id": optional_positive_u32(sample.get("owner_tag_db_id")),
                "member_ids": [],
                "engine_member_count": engine_count,
                "missing_member_count": missing_count,
                "order_count": optional_count_u32(sample.get("order_count")),
                "non_type5_order_count": optional_count_u32(sample.get("non_type5_order_count")),
                "parent_status": status_from_direct_proof(sample.get("parent_status", "unknown")),
                "member_status": member_status,
                "root_status": STATUS_COLD_PREFLIGHT_ONLY,
            }
        )

    for root in roots.get("army_groups", []):
        object_id = int(root["id"])
        sample = army_group_samples.get(object_id, {})
        member_ids = [int(member_id) for member_id in sample.get("member_army_ids", [])]
        member_count = optional_count_u32(sample.get("member_count"))
        rows.append(
            {
                "kind": DIRECT_KIND_ARMY_GROUP,
                "object_id": object_id,
                "deleted": False,
                "parent_id": None,
                "owner_tag_db_id": None,
                "member_ids": member_ids,
                "engine_member_count": member_count,
                "missing_member_count": None,
                "order_count": None,
                "non_type5_order_count": None,
                "parent_status": STATUS_NONE,
                "member_status": STATUS_EXACT if member_ids else STATUS_UNKNOWN,
                "root_status": STATUS_COLD_PREFLIGHT_ONLY,
            }
        )

    for root in roots.get("theatres", []):
        rows.append(
            {
                "kind": DIRECT_KIND_THEATRE,
                "object_id": int(root["id"]),
                "deleted": False,
                "parent_id": None,
                "owner_tag_db_id": None,
                "member_ids": [],
                "engine_member_count": None,
                "missing_member_count": None,
                "order_count": None,
                "non_type5_order_count": None,
                "parent_status": STATUS_NONE,
                "member_status": STATUS_UNKNOWN,
                "root_status": STATUS_COLD_PREFLIGHT_ONLY,
            }
        )
    rows.sort(key=lambda row: (row["kind"], row["object_id"]))
    return rows


def status_from_direct_proof(value: Any) -> int:
    return {
        "none": STATUS_NONE,
        "exact": STATUS_EXACT,
        "unknown": STATUS_UNKNOWN,
        "validation_failed": STATUS_VALIDATION_FAILED,
    }.get(str(value), STATUS_UNKNOWN)


def optional_positive_u32(value: Any) -> int | None:
    if value is None:
        return None
    out = int(value)
    if out <= 0:
        return None
    return out


def optional_count_u32(value: Any) -> int | None:
    if value is None:
        return None
    out = int(value)
    if out < 0:
        return None
    return out


def collect_assigned_unit_statuses(records: list[dict[str, Any]]) -> dict[int, dict[int, int]]:
    by_hour: dict[int, dict[int, int]] = {}
    for record in records:
        if record["record_type"] != RECORD_TYPE_DOMAIN_DATA or record["domain_id"] != DOMAIN_OOB_COMMAND:
            continue
        by_hour[record["game_hour_raw"]] = parse_oob_assignment_statuses(record["payload"])
    return by_hour


def parse_oob_assignment_statuses(payload: bytes) -> dict[int, int]:
    if len(payload) < 48:
        raise ValueError("oob_command payload shorter than 48 bytes")
    row_count = u32(payload, 8)
    member_count = u32(payload, 32)
    expected_len = 48 + row_count * OOB_ROW_LEN + member_count * OOB_MEMBER_ROW_LEN
    if len(payload) != expected_len:
        raise ValueError(f"oob_command payload length {len(payload)} != {expected_len}")
    by_army: dict[int, int] = {}
    cursor = 48
    for _ in range(row_count):
        kind = u32(payload, cursor)
        object_id = u32(payload, cursor + 4)
        flags = u32(payload, cursor + 8)
        member_status = u32(payload, cursor + 56)
        deleted = (flags & OOB_FLAG_DELETED) != 0
        if kind == OOB_KIND_ARMY and not deleted:
            by_army[object_id] = member_status
        cursor += OOB_ROW_LEN
    return by_army


def convert_army_orders_payload(
    payload: bytes,
    assigned_units_by_army: dict[int, int],
    direct_roots: list[dict[str, Any]],
) -> tuple[bytes, dict[str, int]]:
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
        "assigned_units_exact_rows": 0,
        "assigned_units_missing_rows": 0,
        "order_linkage_exact_rows": 0,
        "stance_unknown_rows": 0,
        "aggressiveness_unknown_rows": 0,
        "direct_root_rows": len(direct_roots),
        "direct_root_member_rows": sum(len(row["member_ids"]) for row in direct_roots),
        "direct_army_rows": sum(1 for row in direct_roots if row["kind"] == DIRECT_KIND_ARMY),
        "direct_army_group_rows": sum(1 for row in direct_roots if row["kind"] == DIRECT_KIND_ARMY_GROUP),
        "direct_theatre_rows": sum(1 for row in direct_roots if row["kind"] == DIRECT_KIND_THEATRE),
        "direct_cold_preflight_rows": sum(
            1 for row in direct_roots if row["root_status"] == STATUS_COLD_PREFLIGHT_ONLY
        ),
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
        assigned_units_status = STATUS_NONE if deleted else assigned_units_by_army.get(army_id, STATUS_UNKNOWN)
        order_linkage_status = STATUS_NONE if deleted else STATUS_EXACT
        stance_status = STATUS_NONE if deleted else STATUS_UNKNOWN
        aggressiveness_status = STATUS_NONE if deleted else STATUS_UNKNOWN
        if not deleted:
            if assigned_units_status == STATUS_EXACT:
                stats["assigned_units_exact_rows"] += 1
            else:
                stats["assigned_units_missing_rows"] += 1
            stats["order_linkage_exact_rows"] += 1
            stats["stance_unknown_rows"] += 1
            stats["aggressiveness_unknown_rows"] += 1
        rows.append(
            {
                "army_id": army_id,
                "generation": generation,
                "deleted": deleted,
                "slot_index": slot_index,
                "order_type": order_type,
                "target_states": target_states,
                "path_provinces": [],
                "target_declared": target_declared if flags & ARMY_ORDER_FLAG_TARGET_STATE_COUNT else None,
                "target_status": target_status if order_type == 5 and not deleted else STATUS_UNSUPPORTED,
                "geometry_status": geometry_status,
                "execution_status": execution_status,
                "assigned_units_status": assigned_units_status,
                "order_linkage_status": order_linkage_status,
                "stance_status": stance_status,
                "aggressiveness_status": aggressiveness_status,
            }
        )
        stats["rows"] += 1
        stats["target_state_rows"] += len(target_states)

    depth_payload = encode_depth_payload(
        rows,
        direct_roots,
        elapsed_ms,
        status_flags,
        min(changed_rows, len(rows)),
        observed_army_rows,
        observed_order_rows,
    )
    return depth_payload, stats


def encode_depth_payload(
    rows: list[dict[str, Any]],
    direct_roots: list[dict[str, Any]],
    elapsed_ms: int,
    status_flags: int,
    changed_rows: int,
    observed_army_rows: int,
    observed_order_rows: int,
) -> bytes:
    target_count = sum(len(row["target_states"]) for row in rows)
    path_geometry_count = sum(len(row.get("path_provinces", [])) for row in rows)
    direct_member_count = sum(len(row["member_ids"]) for row in direct_roots)
    unsupported_geometry = sum(
        1 for row in rows if not row["deleted"] and row["geometry_status"] == STATUS_FIXTURE_REQUIRED
    )
    execution_unknown = sum(
        1 for row in rows if not row["deleted"] and row["execution_status"] == STATUS_FIXTURE_REQUIRED
    )
    out = bytearray()
    out += struct.pack(
        "<QIIIIIIIIIII",
        elapsed_ms,
        len(rows),
        changed_rows,
        status_flags,
        observed_army_rows,
        observed_order_rows,
        target_count,
        unsupported_geometry,
        execution_unknown,
        len(direct_roots),
        direct_member_count,
        path_geometry_count,
    )
    target_cursor = 0
    path_cursor = 0
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
        path_provinces = row.get("path_provinces", [])
        if path_provinces:
            flags |= DEPTH_FLAG_PATH_GEOMETRY
        child_first = target_cursor if row["target_states"] else path_cursor if path_provinces else 0
        child_count = len(row["target_states"]) if row["target_states"] else len(path_provinces)
        out += struct.pack(
            "<IQIIiIIIIIIIIII",
            row["army_id"],
            row["generation"],
            flags,
            row["slot_index"],
            row["order_type"],
            child_first,
            child_count,
            row["target_declared"] or 0,
            row["target_status"],
            row["geometry_status"],
            row["execution_status"],
            row["assigned_units_status"],
            row["order_linkage_status"],
            row["stance_status"],
            row["aggressiveness_status"],
        )
        target_cursor += len(row["target_states"])
        path_cursor += len(path_provinces)
    for row in rows:
        for state_id in row["target_states"]:
            out += struct.pack("<II", state_id, 0)
    for row in rows:
        for province_id in row.get("path_provinces", []):
            out += struct.pack("<II", province_id, 0)
    member_cursor = 0
    for row in direct_roots:
        flags = 0
        if row["deleted"]:
            flags |= DIRECT_FLAG_DELETED
        if row["parent_id"] is not None:
            flags |= DIRECT_FLAG_PARENT
        if row["owner_tag_db_id"] is not None:
            flags |= DIRECT_FLAG_OWNER
        if row["engine_member_count"] is not None:
            flags |= DIRECT_FLAG_ENGINE_MEMBER_COUNT
        if row["missing_member_count"] is not None:
            flags |= DIRECT_FLAG_MISSING_MEMBER_COUNT
        if row["member_ids"]:
            flags |= DIRECT_FLAG_MEMBER_IDS
        if row["order_count"] is not None or row["non_type5_order_count"] is not None:
            flags |= DIRECT_FLAG_ORDER_COUNTS
        member_count = len(row["member_ids"])
        member_first = member_cursor if member_count else 0
        out += struct.pack(
            "<IIIIIIIIIIIIIIII",
            row["kind"],
            row["object_id"],
            flags,
            row["parent_id"] or 0,
            row["owner_tag_db_id"] or 0,
            member_first,
            member_count,
            row["engine_member_count"] or 0,
            row["missing_member_count"] or 0,
            row["order_count"] or 0,
            row["non_type5_order_count"] or 0,
            row["parent_status"],
            row["member_status"],
            row["root_status"],
            0,
            0,
        )
        member_cursor += member_count
    for row in direct_roots:
        for member_id in row["member_ids"]:
            out += struct.pack("<II", member_id, 0)
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
