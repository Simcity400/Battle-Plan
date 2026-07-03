#!/usr/bin/env python3
"""Read-only ace-assignment fixture snapshot for the game build 870ebf.

Run this before and after assigning/removing a known ace through the UI. The
probe dumps the validated strategic-air path, the selected owner container, the
country ace roster, and selected wing assignment fields. It cannot observe the
0xC52840 callsite register payload by itself; the report states the exact
separate criterion needed to validate rdx=r14.
"""

from __future__ import annotations

import argparse
import collections
import datetime as dt
import hashlib
import json
import struct
from pathlib import Path
from typing import Any

import winmem
from naval_air.air_combat_losses_probe_870ebf import (
    EXPECTED_SHA256,
    Probe,
    hx,
)


PROC_NAME = "hoi4.exe"
HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000

C_GAME_STATE_GLOBAL = 0x3316E90
COUNTRY_DATABASE_GLOBAL = 0x33189C8
CGAMESTATE_COUNTRY_TABLE = 0x310
CGAMESTATE_COUNTRY_COUNT = 0x31C
COUNTRY_VTABLE_RVA = 0x27D0E90
OBJ_TAG_DB_ID = 0x08

STRATEGIC_AIR_OWNER_CONTAINERS_PTR_OFF = 0x30
STRATEGIC_AIR_OWNER_CONTAINERS_COUNT_OFF = 0x3C
OWNER_AIR_WING_POOL_PTR_TABLE_OFF = 0x58
OWNER_AIR_WING_POOL_COUNT_OFF = 0x64
OWNER_TAG_DB_ID_OFF = 0x90

HISTORICAL_ACE_VECTOR_OFF = 0x12D8
HISTORICAL_ACE_COUNT_OFF = 0x12E4
ACE_OBJECT_ID_OFF = 0x0C
ACE_OBJECT_NAME_OFF = 0xF0
ACE_OBJECT_SURNAME_OFF = 0x110
ACE_OBJECT_CALLSIGN_OFF = 0x130
WING_BODY_ASSIGNED_ACE_ID_OFF = 0x2DC
WING_OBJECT_ASSIGNED_ACE_ID_OFF = 0x2EC

MAX_COUNTRIES = 4096
MAX_STRATEGIC_AIR_CONTAINERS = 4096
MAX_ACE_COUNT = 1024
MAX_VECTOR_CAP = 4096
MAX_ACE_STRING_LEN = 96
MAX_VECTOR_BYTES = 128 * 1024


def utc_now() -> str:
    return dt.datetime.now(dt.UTC).replace(microsecond=0).isoformat()


def safe(fn):
    try:
        return fn()
    except Exception:
        return None


def is_heap(value: int | None) -> bool:
    return isinstance(value, int) and HEAP_LO <= value < HEAP_HI and value % 8 == 0


def read_u32_stable(proc: Any, addr: int) -> int | None:
    first = safe(lambda: proc.u32(addr))
    second = safe(lambda: proc.u32(addr))
    return first if first == second else None


def read_u64_stable(proc: Any, addr: int) -> int | None:
    first = safe(lambda: proc.u64(addr))
    second = safe(lambda: proc.u64(addr))
    return first if first == second else None


def read_small_inline_string(proc: Any, addr: int) -> str | None:
    raw = safe(lambda: proc.try_read(addr, 32))
    if raw is None or len(raw) != 32:
        return None
    size = struct.unpack_from("<I", raw, 16)[0]
    if size == 0 or size > 7:
        return None
    body = raw[:size]
    if not all(byte in b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-" for byte in body):
        return None
    value = body.decode("ascii", errors="replace")
    if len(value) != 3 or (value != "---" and not any(ch.isalpha() for ch in value)):
        return None
    return value


def read_msvc_string_stable(proc: Any, addr: int, max_len: int = MAX_ACE_STRING_LEN) -> str | None:
    def read_once() -> str | None:
        raw = proc.try_read(addr, 0x20)
        if len(raw) < 0x20:
            return None
        size = struct.unpack_from("<Q", raw, 0x10)[0]
        cap = struct.unpack_from("<Q", raw, 0x18)[0]
        if size == 0:
            return None
        if size > max_len or cap < size or cap > (1 << 20):
            return None
        if cap < 16:
            body = raw[:size]
        else:
            ptr = struct.unpack_from("<Q", raw, 0)[0]
            if not is_heap(ptr):
                return None
            heap = proc.try_read(ptr, size + 1)
            if len(heap) < size + 1 or heap[size] != 0:
                return None
            body = heap[:size]
        if not all(32 <= byte < 127 for byte in body):
            return None
        try:
            return body.decode("utf-8")
        except UnicodeDecodeError:
            return body.decode("latin-1", errors="replace")

    first = safe(read_once)
    second = safe(read_once)
    return first if first == second else None


def custom_vector_header(proc: Any, obj: int, off: int) -> tuple[int, int, int] | None:
    data = read_u64_stable(proc, obj + off)
    cap = read_u32_stable(proc, obj + off + 8)
    size = read_u32_stable(proc, obj + off + 0x0C)
    if data is None or cap is None or size is None:
        return None
    if size > cap or cap > MAX_VECTOR_CAP:
        return None
    if size and not is_heap(data):
        return None
    return data, cap, size


def country_tag_table(proc: Any, country_database: int | None) -> tuple[dict[int, str], dict[str, Any]]:
    if not is_heap(country_database):
        return {}, {"valid": False, "reason": "country_database_invalid", "country_database": hx(country_database)}
    tag_table = read_u64_stable(proc, country_database)
    tag_count = read_u32_stable(proc, country_database + 8)
    active_count = read_u32_stable(proc, country_database + 12)
    meta: dict[str, Any] = {
        "country_database": hx(country_database),
        "tag_table": hx(tag_table),
        "tag_count": tag_count,
        "active_count": active_count,
    }
    if not is_heap(tag_table) or not tag_count or not active_count:
        meta["valid"] = False
        return {}, meta
    tags = {}
    for tag_id in range(min(tag_count, active_count, 1024)):
        tag = read_small_inline_string(proc, tag_table + tag_id * 32)
        if tag:
            tags[tag_id] = tag
    meta["valid"] = len(tags) >= 40
    meta["row_count"] = len(tags)
    return tags, meta


def live_country_rows(proc: Any, module_base: int, game_state: int, tags: dict[int, str]) -> list[dict[str, Any]]:
    table = read_u64_stable(proc, game_state + CGAMESTATE_COUNTRY_TABLE)
    count = read_u32_stable(proc, game_state + CGAMESTATE_COUNTRY_COUNT)
    if not is_heap(table) or not count or count > MAX_COUNTRIES:
        return []
    country_vtable = module_base + COUNTRY_VTABLE_RVA
    rows = []
    for index in range(1, min(count, len(tags), MAX_COUNTRIES)):
        ptr = read_u64_stable(proc, table + index * 8)
        if not is_heap(ptr) or read_u64_stable(proc, ptr) != country_vtable:
            continue
        tag_db_id = read_u32_stable(proc, ptr + OBJ_TAG_DB_ID)
        if tag_db_id != index:
            continue
        rows.append({"tag_db_id": index, "tag": tags.get(index), "country_ptr": ptr})
    return rows


def read_ace_roster(proc: Any, country: int | None, *, sample_limit: int) -> dict[str, Any]:
    if not is_heap(country):
        return {"available": False, "reason": "country_ptr_invalid", "country": hx(country)}
    live_count = read_u32_stable(proc, country + HISTORICAL_ACE_COUNT_OFF)
    header = custom_vector_header(proc, country, HISTORICAL_ACE_VECTOR_OFF)
    out: dict[str, Any] = {
        "available": False,
        "country": hx(country),
        "ace_count_offset": "0x12E4",
        "ace_count": live_count,
        "vector_offset": "0x12D8",
        "vector_header": None,
        "entries": [],
    }
    if header is None:
        out["reason"] = "ace_vector_header_invalid"
        return out
    data, cap, size = header
    out["vector_header"] = {"data": hx(data), "cap": cap, "size": size}
    if live_count is None or size != live_count or live_count > MAX_ACE_COUNT:
        out["reason"] = "ace_count_vector_size_mismatch"
        return out
    raw = proc.try_read(data, min(MAX_VECTOR_BYTES, size * 8)) if size else b""
    if len(raw) < size * 8:
        out["reason"] = "ace_vector_read_truncated"
        out["bytes_read"] = len(raw)
        return out
    entries = []
    for index in range(size):
        ptr = struct.unpack_from("<Q", raw, index * 8)[0]
        if not is_heap(ptr):
            continue
        ace_id = read_u32_stable(proc, ptr + ACE_OBJECT_ID_OFF)
        if not ace_id:
            continue
        entries.append(
            {
                "country_ace_index": index,
                "ace_ptr": hx(ptr),
                "ace_id": ace_id,
                "name": read_msvc_string_stable(proc, ptr + ACE_OBJECT_NAME_OFF),
                "surname": read_msvc_string_stable(proc, ptr + ACE_OBJECT_SURNAME_OFF),
                "callsign": read_msvc_string_stable(proc, ptr + ACE_OBJECT_CALLSIGN_OFF),
            }
        )
    out["available"] = True
    out["entry_count"] = len(entries)
    out["entries"] = entries[:sample_limit]
    out["duplicate_ace_ids"] = [
        ace_id
        for ace_id, count in collections.Counter(row["ace_id"] for row in entries).items()
        if count > 1
    ][:16]
    return out


def dump_qwords(proc: Any, addr: int | None, span: int) -> dict[str, Any]:
    if not is_heap(addr):
        return {"available": False, "reason": "address_invalid", "addr": hx(addr)}
    blob = proc.try_read(addr, span)
    qwords = []
    for off in range(0, len(blob) - 7, 8):
        qwords.append({"off": f"0x{off:X}", "value": hx(struct.unpack_from("<Q", blob, off)[0])})
    return {
        "available": len(blob) == span,
        "addr": hx(addr),
        "span": f"0x{span:X}",
        "bytes_read": len(blob),
        "sha256": hashlib.sha256(blob).hexdigest(),
        "qwords": qwords,
    }


def find_owner_container(proc: Any, strategic_air: int, owner_tag_db_id: int | None) -> dict[str, Any]:
    begin = read_u64_stable(proc, strategic_air + STRATEGIC_AIR_OWNER_CONTAINERS_PTR_OFF)
    count = read_u32_stable(proc, strategic_air + STRATEGIC_AIR_OWNER_CONTAINERS_COUNT_OFF)
    meta: dict[str, Any] = {
        "strategic_air": hx(strategic_air),
        "container_table": hx(begin),
        "container_count": count,
        "owner_tag_db_id": owner_tag_db_id,
    }
    if owner_tag_db_id is None:
        meta["available"] = False
        meta["reason"] = "owner_tag_db_id_required"
        return meta
    if not is_heap(begin) or count is None or count > MAX_STRATEGIC_AIR_CONTAINERS:
        meta["available"] = False
        meta["reason"] = "owner_container_table_invalid"
        return meta
    for index in range(count):
        container = read_u64_stable(proc, begin + index * 8)
        if not is_heap(container):
            continue
        owner = read_u32_stable(proc, container + OWNER_TAG_DB_ID_OFF)
        if owner == owner_tag_db_id:
            meta.update(
                {
                    "available": True,
                    "container_index": index,
                    "container": hx(container),
                    "pool_table": hx(read_u64_stable(proc, container + OWNER_AIR_WING_POOL_PTR_TABLE_OFF)),
                    "pool_count": read_u32_stable(proc, container + OWNER_AIR_WING_POOL_COUNT_OFF),
                }
            )
            return meta
    meta["available"] = False
    meta["reason"] = "owner_container_not_found"
    return meta


def resolve_owner_tag_db_id(
    args: argparse.Namespace,
    tags: dict[int, str],
) -> int | None:
    if args.owner_tag_db_id is not None:
        return args.owner_tag_db_id
    if args.owner_tag:
        wanted = args.owner_tag.upper()
        for tag_db_id, tag in tags.items():
            if tag == wanted:
                return tag_db_id
    return None


def selected_wing_report(
    proc: Any,
    air_wings: list[dict[str, Any]],
    *,
    owner_tag_db_id: int | None,
    wing_id: int | None,
    wing_span: int,
) -> dict[str, Any]:
    selected = [
        row
        for row in air_wings
        if (owner_tag_db_id is None or row.get("owner_tag_db_id") == owner_tag_db_id)
        and (wing_id is None or row.get("wing_id") == wing_id)
    ]
    rows = []
    for row in selected[:32]:
        obj = row["object"]
        body = row["body"]
        raw = proc.try_read(body, wing_span)
        rows.append(
            {
                "key": f"{row['owner_tag_db_id']}:{row['wing_id']}",
                "owner_tag_db_id": row["owner_tag_db_id"],
                "wing_id": row["wing_id"],
                "object": hx(obj),
                "body": hx(body),
                "mission_region": row.get("mission_region"),
                "mission_mask": row.get("mission_mask"),
                "count": row.get("count"),
                "effective_planes": row.get("effective_planes"),
                "body_assigned_ace_id_0x2DC": read_u32_stable(proc, body + WING_BODY_ASSIGNED_ACE_ID_OFF),
                "object_assigned_ace_id_0x2EC": read_u32_stable(proc, obj + WING_OBJECT_ASSIGNED_ACE_ID_OFF),
                "body_span": f"0x{wing_span:X}",
                "body_bytes_read": len(raw),
                "body_sha256": hashlib.sha256(raw).hexdigest(),
            }
        )
    return {
        "requested_owner_tag_db_id": owner_tag_db_id,
        "requested_wing_id": wing_id,
        "matched_rows": len(selected),
        "rows": rows,
    }


OWNER_CONTAINER_IDENTITY_FIELDS = (
    "strategic_air",
    "container_table",
    "container_count",
    "owner_tag_db_id",
    "container_index",
    "container",
    "pool_table",
    "pool_count",
)


def valid_hex_ptr(value: Any) -> bool:
    return isinstance(value, str) and value.startswith("0x")


def owner_container_identity(report: dict[str, Any]) -> dict[str, Any]:
    container = report.get("owner_container", {})
    return {field: container.get(field) for field in OWNER_CONTAINER_IDENTITY_FIELDS}


def dump_identity(report: dict[str, Any], key: str) -> dict[str, Any]:
    dump = report.get(key, {})
    return {
        "available": dump.get("available"),
        "addr": dump.get("addr"),
        "span": dump.get("span"),
        "bytes_read": dump.get("bytes_read"),
    }


def roster_summary(report: dict[str, Any]) -> dict[str, Any]:
    roster = report.get("owner_ace_roster", {})
    return {
        "available": roster.get("available"),
        "reason": roster.get("reason"),
        "country": roster.get("country"),
        "ace_count": roster.get("ace_count"),
        "entry_count": roster.get("entry_count"),
        "duplicate_ace_ids": roster.get("duplicate_ace_ids", []),
    }


def compare_failure(
    status: str,
    before: dict[str, Any],
    target_ace_id: int | None,
    preconditions: dict[str, Any],
) -> dict[str, Any]:
    return {
        "available": False,
        "before_source": before.get("source_path"),
        "target_ace_id": target_ace_id,
        "assignment_field_diffs": [],
        "preconditions": preconditions,
        "status": status,
        "candidate_side_effect_status": "not_evaluated_precondition_failed",
        "rdx_payload_status": "not_validated_by_polling",
    }


def compare_preconditions(before: dict[str, Any], after: dict[str, Any]) -> tuple[str | None, dict[str, Any]]:
    before_strategic_air = before.get("roots", {}).get("strategic_air")
    after_strategic_air = after.get("roots", {}).get("strategic_air")
    before_owner_country = before.get("owner_country", {}).get("country_ptr")
    after_owner_country = after.get("owner_country", {}).get("country_ptr")
    before_container = before.get("owner_container", {})
    after_container = after.get("owner_container", {})
    before_container_id = owner_container_identity(before)
    after_container_id = owner_container_identity(after)
    before_dump = dump_identity(before, "owner_container_dump")
    after_dump = dump_identity(after, "owner_container_dump")
    before_roster = roster_summary(before)
    after_roster = roster_summary(after)

    preconditions = {
        "strategic_air": {
            "before": before_strategic_air,
            "after": after_strategic_air,
            "stable": before_strategic_air == after_strategic_air,
        },
        "owner_country": {
            "before": before_owner_country,
            "after": after_owner_country,
            "stable": before_owner_country == after_owner_country,
        },
        "owner_container": {
            "before": before_container_id,
            "after": after_container_id,
            "stable": before_container_id == after_container_id,
        },
        "owner_container_dump": {
            "before": before_dump,
            "after": after_dump,
            "stable": before_dump == after_dump,
            "matches_owner_container": (
                before_dump.get("addr") == before_container.get("container")
                and after_dump.get("addr") == after_container.get("container")
            ),
        },
        "owner_ace_roster": {
            "before": before_roster,
            "after": after_roster,
            "readable": bool(before_roster.get("available")) and bool(after_roster.get("available")),
        },
    }

    if not (valid_hex_ptr(before_strategic_air) and valid_hex_ptr(after_strategic_air)):
        return "failed_closed_strategic_air_unreadable", preconditions
    if before_strategic_air != after_strategic_air:
        return "failed_closed_strategic_air_unstable", preconditions
    if not (valid_hex_ptr(before_owner_country) and valid_hex_ptr(after_owner_country)):
        return "failed_closed_owner_country_unreadable", preconditions
    if before_owner_country != after_owner_country:
        return "failed_closed_owner_country_unstable", preconditions
    if not (before_container.get("available") and after_container.get("available")):
        return "failed_closed_owner_container_unreadable", preconditions
    if before_container_id != after_container_id:
        return "failed_closed_owner_container_unstable", preconditions
    if not (before_dump.get("available") and after_dump.get("available")):
        return "failed_closed_owner_container_dump_unreadable", preconditions
    if not preconditions["owner_container_dump"]["matches_owner_container"]:
        return "failed_closed_owner_container_dump_mismatch", preconditions
    if before_dump != after_dump:
        return "failed_closed_owner_container_dump_unstable", preconditions
    if not preconditions["owner_ace_roster"]["readable"]:
        return "failed_closed_ace_roster_unreadable", preconditions
    return None, preconditions


def compare_reports(before: dict[str, Any], after: dict[str, Any], target_ace_id: int | None) -> dict[str, Any]:
    precondition_status, preconditions = compare_preconditions(before, after)
    if precondition_status is not None:
        return compare_failure(precondition_status, before, target_ace_id, preconditions)

    before_rows = {
        row["key"]: row
        for row in before.get("selected_wing", {}).get("rows", [])
        if isinstance(row.get("key"), str)
    }
    after_rows = {
        row["key"]: row
        for row in after.get("selected_wing", {}).get("rows", [])
        if isinstance(row.get("key"), str)
    }
    diffs = []
    for key, new in after_rows.items():
        old = before_rows.get(key)
        if old is None:
            continue
        before_ace = old.get("body_assigned_ace_id_0x2DC")
        after_ace = new.get("body_assigned_ace_id_0x2DC")
        if before_ace != after_ace:
            diffs.append(
                {
                    "key": key,
                    "before_body_assigned_ace_id_0x2DC": before_ace,
                    "after_body_assigned_ace_id_0x2DC": after_ace,
                    "target_ace_id_match": after_ace == target_ace_id if target_ace_id is not None else None,
                }
            )
    return {
        "available": True,
        "before_source": before.get("source_path"),
        "target_ace_id": target_ace_id,
        "assignment_field_diffs": diffs,
        "preconditions": preconditions,
        "status": (
            "candidate_assignment_side_effect_observed"
            if diffs and (target_ace_id is None or any(row.get("target_ace_id_match") for row in diffs))
            else "candidate_assignment_side_effect_not_observed"
        ),
        "rdx_payload_status": "not_validated_by_polling",
    }


def build_report(args: argparse.Namespace) -> tuple[int, dict[str, Any]]:
    pid = args.pid or winmem.find_pid(args.process)
    if pid is None:
        return 2, {
            "schema": "air_ace_assignment_fixture_probe_870ebf_v1",
            "created_utc": utc_now(),
            "status": "failed_closed_process_not_found",
            "process": args.process,
        }
    probe = Probe(pid)
    try:
        root = probe.root_probe()
        if not root.get("sha256_ok"):
            return 3, {
                "schema": "air_ace_assignment_fixture_probe_870ebf_v1",
                "created_utc": utc_now(),
                "status": "failed_closed_sha_mismatch",
                "root": root,
            }
        game_state, strategic_air = probe.require_root(root)
        country_database = read_u64_stable(probe.proc, probe.base + COUNTRY_DATABASE_GLOBAL)
        tags, tag_meta = country_tag_table(probe.proc, country_database)
        owner_tag_db_id = resolve_owner_tag_db_id(args, tags)
        countries = live_country_rows(probe.proc, probe.base, game_state, tags)
        country_by_tag_id = {row["tag_db_id"]: row["country_ptr"] for row in countries}
        owner_country = country_by_tag_id.get(owner_tag_db_id) if owner_tag_db_id is not None else None
        air_wings, topology, _sources, _pools = probe.enumerate_air_wings(strategic_air)
        owner_container = find_owner_container(probe.proc, strategic_air, owner_tag_db_id)
        owner_container_addr = (
            int(owner_container["container"], 16)
            if owner_container.get("available") and isinstance(owner_container.get("container"), str)
            else None
        )
        report = {
            "schema": "air_ace_assignment_fixture_probe_870ebf_v1",
            "created_utc": utc_now(),
            "snapshot_label": args.snapshot_label,
            "read_only": True,
            "process": args.process,
            "pid": pid,
            "root": root,
            "module_sha256_expected": EXPECTED_SHA256,
            "roots": {
                "game_state_global": hx(probe.base + C_GAME_STATE_GLOBAL),
                "game_state": hx(game_state),
                "strategic_air": hx(strategic_air),
                "country_database_global": hx(probe.base + COUNTRY_DATABASE_GLOBAL),
                "country_database": hx(country_database),
            },
            "tag_table": tag_meta,
            "requested_owner": {
                "owner_tag": args.owner_tag,
                "owner_tag_db_id": owner_tag_db_id,
                "resolved_tag": tags.get(owner_tag_db_id) if owner_tag_db_id is not None else None,
            },
            "owner_country": {
                "country_ptr": hx(owner_country),
                "country_vtable_expected": hx(probe.base + COUNTRY_VTABLE_RVA),
            },
            "strategic_air_dump": dump_qwords(probe.proc, strategic_air, args.manager_span),
            "owner_container": owner_container,
            "owner_container_dump": dump_qwords(probe.proc, owner_container_addr, args.container_span),
            "owner_ace_roster": read_ace_roster(
                probe.proc,
                owner_country,
                sample_limit=args.ace_sample_limit,
            ),
            "air_wing_topology": topology,
            "selected_wing": selected_wing_report(
                probe.proc,
                air_wings,
                owner_tag_db_id=owner_tag_db_id,
                wing_id=args.wing_id,
                wing_span=args.wing_span,
            ),
            "rdx_r14_payload_validation_contract": {
                "polling_limit": (
                    "This probe does not observe registers and cannot validate rdx=r14. "
                    "It only dumps before/after side effects."
                ),
                "required_callsite_if_authorized": "0x6FA8BA -> 0xC52840 under command id 0x2FB5",
                "required_rcx_match": (
                    "rcx must equal the selected owner strategic-air container dumped in "
                    "owner_container.container."
                ),
                "required_rdx_payload_match": (
                    "The same command instance's rdx/r14 object must have discriminator "
                    "dword +0x30 and fields +0x28/+0x34/+0x38/+0x44 matching the UI action "
                    "owner, wing id, ace id, and assignment/remove semantics proven by these "
                    "before/after polling dumps."
                ),
                "promotion_rule": (
                    "Without the register capture, ace assignment side effects may be accepted "
                    "as polling-observable state, but rdx payload interpretation remains blocked."
                ),
            },
            "status": "completed_snapshot",
        }
        if args.compare_before:
            before = json.loads(args.compare_before.read_text(encoding="utf-8"))
            before["source_path"] = str(args.compare_before)
            report["compare_before"] = compare_reports(before, report, args.ace_id)
        return 0, report
    except Exception as exc:  # noqa: BLE001
        return 4, {
            "schema": "air_ace_assignment_fixture_probe_870ebf_v1",
            "created_utc": utc_now(),
            "status": "failed_closed_exception",
            "error": repr(exc),
        }
    finally:
        probe.close()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pid", type=int)
    parser.add_argument("--process", default=PROC_NAME)
    parser.add_argument("--snapshot-label", required=True)
    parser.add_argument("--owner-tag")
    parser.add_argument("--owner-tag-db-id", type=int)
    parser.add_argument("--wing-id", type=int)
    parser.add_argument("--ace-id", type=int)
    parser.add_argument("--manager-span", type=lambda value: int(value, 0), default=0x220)
    parser.add_argument("--container-span", type=lambda value: int(value, 0), default=0x220)
    parser.add_argument("--wing-span", type=lambda value: int(value, 0), default=0x380)
    parser.add_argument("--ace-sample-limit", type=int, default=128)
    parser.add_argument("--compare-before", type=Path)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()

    if not args.owner_tag and args.owner_tag_db_id is None:
        parser.error("provide --owner-tag or --owner-tag-db-id")

    code, report = build_report(args)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    summary = {
        "out": str(args.out),
        "status": report.get("status"),
        "snapshot_label": report.get("snapshot_label"),
        "sha_ok": report.get("root", {}).get("sha256_ok"),
        "owner_container_available": report.get("owner_container", {}).get("available"),
        "selected_wing_rows": report.get("selected_wing", {}).get("matched_rows"),
        "compare_status": report.get("compare_before", {}).get("status"),
    }
    print(json.dumps(summary, indent=2, sort_keys=True))
    return code


if __name__ == "__main__":
    raise SystemExit(main())
