#!/usr/bin/env python3
"""Read-only 870ebf probe for division assignment/order/SR candidates.

The probe starts from the validated country-table division enumeration route,
uses CDivision+0xB0 as the positive-control army-parent path, enumerates order
objects from validated CArmy roots, and scans finite candidate fields. It emits
candidate/rejection evidence only; it does not promote any compact row.
"""

from __future__ import annotations

import argparse
from collections import Counter, defaultdict
from datetime import datetime, timezone
import json
from pathlib import Path
import struct
import sys
import time
from typing import Any


REPO_ROOT = Path(__file__).resolve().parents[3]
MEMORY_READER = REPO_ROOT / "extractor" / "memory_reader"
if str(MEMORY_READER) not in sys.path:
    sys.path.insert(0, str(MEMORY_READER))

import winmem  # type: ignore  # noqa: E402


EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
PROC_NAME = "hoi4.exe"
SCHEMA = "battle-plan.division_assignment_probe_870ebf.v1"

CGAMESTATE_RVA = 0x3316E90
COUNTRY_DATABASE_RVA = 0x33189C8
CGAMESTATE_COUNTRY_TABLE = 0x310
CGAMESTATE_COUNTRY_COUNT = 0x31C
COUNTRY_VTABLE_RVA = 0x27D0E90
COUNTRY_TAG_ECHO = 0x08
COUNTRY_DIVISION_WRAPPER_DATA = 0x290
COUNTRY_DIVISION_WRAPPER_CAPACITY = 0x298
COUNTRY_DIVISION_WRAPPER_COUNT = 0x29C
DIVISION_WRAPPER_VTABLE_RVA = 0x2943AD0
DIVISION_WRAPPER_TO_BODY = 0x10
DIVISION_VTABLE_RVA = 0x2943CB0

DIVISION_TYPE = 4713
DIVISION_ALT_UNIT_TYPE = 51
DIVISION_OFF_TYPE = 0x08
DIVISION_OFF_ID = 0x0C
DIVISION_OFF_LOCATION = 0x18
DIVISION_OFF_ARMY_PARENT = 0xB0
DIVISION_OFF_COMBAT_REF = 0x1A4
DIVISION_OFF_OWNER_TAG_DB_ID = 0x1C8
DIVISION_OFF_MOVE_PROGRESS = 0x230
DIVISION_OFF_MOVE_ORDER = 0x1E8
DIVISION_OFF_STRENGTH = 0x410
DIVISION_OFF_ORG = 0x418

ARMY_VTABLE_RVA = 0x293BC60
ARMY_GROUP_VTABLE_RVA = 0x293BCF8
THEATRE_VTABLE_RVA = 0x295EE18
ORDER_VTABLE_RVA = 0x296FAE8
ARMY_TYPE = 53
THEATRE_TYPE = 67
ARMY_OFF_HANDLE = 0x08
ARMY_OFF_ENGINE_MEMBER_COUNT = 0x5C
ARMY_OFF_LEADER_PTR = 0x88
ARMY_OFF_ORDERS_VECTOR = 0x98
ARMY_OFF_ARMY_GROUP_PARENT = 0x1B8
ORDER_OFF_TYPE = 0x30
ORDER_OFF_ARMY_BACKREF = 0x38
ORDER_OFF_TYPE5_STATES_BEGIN = 0xE0
ORDER_OFF_TYPE5_STATES_SIZE = 0xE8

HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000
MAX_COUNTRIES = 4096
MAX_WRAPPERS_PER_COUNTRY = 20000
MAX_DIVISIONS = 50000
MAX_ARMY_ORDERS = 64
MAX_VECTOR_COUNT = 20000
MAX_BODY_SCAN = 0x700
MAX_ARMY_SCAN = 0x260
MAX_LOW_CARDINALITY_VALUE = 256


def hx(value: int | None) -> str | None:
    return f"0x{value:X}" if isinstance(value, int) else None


def is_heap(value: int | None, align: int = 8) -> bool:
    return isinstance(value, int) and HEAP_LO <= value < HEAP_HI and value % align == 0


def read_exact(p: Any, address: int, size: int) -> bytes | None:
    if not isinstance(address, int) or address <= 0:
        return None
    try:
        data = p.read(address, size)
    except Exception:
        return None
    return data if len(data) == size else None


def safe_u64(p: Any, address: int) -> int | None:
    data = read_exact(p, address, 8)
    return struct.unpack("<Q", data)[0] if data is not None else None


def safe_u32(p: Any, address: int) -> int | None:
    data = read_exact(p, address, 4)
    return struct.unpack("<I", data)[0] if data is not None else None


def safe_i32(p: Any, address: int) -> int | None:
    data = read_exact(p, address, 4)
    return struct.unpack("<i", data)[0] if data is not None else None


def process_info(p: Any) -> dict[str, Any]:
    fp = p.fingerprint()
    return {
        "pid": fp.get("pid"),
        "module": fp.get("module"),
        "module_base": hx(fp.get("base")),
        "module_size": fp.get("size"),
        "module_path": fp.get("path"),
        "sha256": fp.get("sha256"),
    }


def object_handle_id(p: Any, addr: int, expected_type: int) -> int | None:
    handle = safe_u64(p, addr + ARMY_OFF_HANDLE)
    if not isinstance(handle, int):
        return None
    if (handle & 0xFFFF_FFFF) != expected_type:
        return None
    object_id = (handle >> 32) & 0xFFFF_FFFF
    return object_id if object_id > 0 else None


def load_tags(p: Any) -> dict[int, str]:
    country_database = safe_u64(p, p.module_base + COUNTRY_DATABASE_RVA)
    if not is_heap(country_database):
        return {}
    tag_table = safe_u64(p, country_database)
    tag_count = safe_u32(p, country_database + 0x08) or 0
    if not is_heap(tag_table):
        return {}
    tags: dict[int, str] = {}
    for tag_id in range(min(tag_count, MAX_COUNTRIES)):
        raw = read_exact(p, tag_table + tag_id * 0x20, 3)
        if not raw:
            continue
        try:
            tag = raw.decode("ascii")
        except UnicodeDecodeError:
            continue
        if tag.isalpha() and tag.isupper():
            tags[tag_id] = tag
    return tags


def enumerate_divisions(p: Any, tags: dict[int, str], sample_limit: int | None = None) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    module = p.module_base
    stats = Counter()
    game_state = safe_u64(p, module + CGAMESTATE_RVA)
    if not is_heap(game_state):
        raise RuntimeError(f"invalid CGameState pointer {hx(game_state)}")
    country_table = safe_u64(p, game_state + CGAMESTATE_COUNTRY_TABLE)
    country_count = safe_u32(p, game_state + CGAMESTATE_COUNTRY_COUNT)
    if not is_heap(country_table) or not isinstance(country_count, int):
        raise RuntimeError("invalid country table/count")
    if country_count <= 0 or country_count > MAX_COUNTRIES:
        raise RuntimeError(f"country_count outside bound: {country_count}")
    country_blob = p.try_read(country_table, country_count * 8)
    if len(country_blob) != country_count * 8:
        raise RuntimeError("short country table read")

    country_vtable = module + COUNTRY_VTABLE_RVA
    wrapper_vtable = module + DIVISION_WRAPPER_VTABLE_RVA
    division_vtable = module + DIVISION_VTABLE_RVA
    rows: list[dict[str, Any]] = []
    seen: set[tuple[int, int]] = set()
    for index in range(country_count):
        country_ptr = struct.unpack_from("<Q", country_blob, index * 8)[0]
        if not is_heap(country_ptr):
            continue
        if safe_u64(p, country_ptr) != country_vtable:
            stats["country_bad_vtable"] += 1
            continue
        tag_echo = safe_u32(p, country_ptr + COUNTRY_TAG_ECHO)
        if tag_echo != index:
            stats["country_bad_tag_echo"] += 1
            continue
        stats["countries_valid"] += 1
        begin = safe_u64(p, country_ptr + COUNTRY_DIVISION_WRAPPER_DATA)
        capacity = safe_u32(p, country_ptr + COUNTRY_DIVISION_WRAPPER_CAPACITY)
        count = safe_u32(p, country_ptr + COUNTRY_DIVISION_WRAPPER_COUNT)
        if not count:
            continue
        if not is_heap(begin) or not isinstance(capacity, int) or count > capacity or count > MAX_WRAPPERS_PER_COUNTRY:
            stats["division_wrapper_vector_rejected"] += 1
            continue
        raw = p.try_read(begin, count * 8)
        if len(raw) != count * 8:
            stats["division_wrapper_vector_short_read"] += 1
            continue
        for slot in range(count):
            wrapper = struct.unpack_from("<Q", raw, slot * 8)[0]
            stats["wrapper_rows_seen"] += 1
            if not is_heap(wrapper):
                stats["wrapper_nonheap"] += 1
                continue
            if safe_u64(p, wrapper) != wrapper_vtable:
                stats["wrapper_bad_vtable"] += 1
                continue
            body = wrapper + DIVISION_WRAPPER_TO_BODY
            if safe_u64(p, body) != division_vtable:
                stats["division_bad_vtable"] += 1
                continue
            div_type = safe_u32(p, body + DIVISION_OFF_TYPE)
            if div_type not in (DIVISION_TYPE, DIVISION_ALT_UNIT_TYPE):
                stats["division_bad_type"] += 1
                continue
            div_id = safe_u32(p, body + DIVISION_OFF_ID)
            owner_id = safe_u32(p, body + DIVISION_OFF_OWNER_TAG_DB_ID)
            if not isinstance(div_id, int) or not isinstance(owner_id, int):
                stats["division_bad_identity"] += 1
                continue
            key = (owner_id, div_id)
            if key in seen:
                stats["division_duplicate_owner_id"] += 1
                continue
            seen.add(key)
            owner_tag = tags.get(owner_id)
            row = {
                "owner_division_key": f"{owner_tag}|{div_id}:{DIVISION_TYPE}" if owner_tag else None,
                "division_key": f"{div_id}:{DIVISION_TYPE}",
                "div_id": div_id,
                "div_type": div_type,
                "owner_tag_db_id": owner_id,
                "owner": owner_tag,
                "country_slot": index,
                "wrapper_addr": wrapper,
                "body_addr": body,
                "location_province": safe_u32(p, body + DIVISION_OFF_LOCATION),
                "army_parent_ptr": safe_u64(p, body + DIVISION_OFF_ARMY_PARENT),
                "combat_ref_count": safe_u32(p, body + DIVISION_OFF_COMBAT_REF),
                "move_progress_raw": safe_u32(p, body + DIVISION_OFF_MOVE_PROGRESS),
                "move_order_ptr": safe_u64(p, body + DIVISION_OFF_MOVE_ORDER),
                "strength_raw": safe_u32(p, body + DIVISION_OFF_STRENGTH),
                "organisation_raw": safe_u32(p, body + DIVISION_OFF_ORG),
            }
            rows.append(row)
            stats["division_rows"] += 1
            if sample_limit is not None and len(rows) >= sample_limit:
                return rows, dict(stats)
            if len(rows) > MAX_DIVISIONS:
                raise RuntimeError("division count exceeded sanity bound")
    return rows, dict(stats)


def load_oracle(path: Path) -> dict[str, Any]:
    data = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(data, dict):
        raise ValueError("oracle top-level must be an object")
    return data


def oracle_by_owner_key(oracle: dict[str, Any]) -> dict[str, dict[str, Any]]:
    rows = oracle.get("by_owner_division_key")
    if isinstance(rows, dict):
        return {str(k): v for k, v in rows.items() if isinstance(v, dict)}
    out: dict[str, dict[str, Any]] = {}
    for row in oracle.get("rows", []):
        if isinstance(row, dict) and isinstance(row.get("owner_division_key"), str):
            out[row["owner_division_key"]] = row
    return out


def validate_armies_from_divisions(p: Any, live_rows: list[dict[str, Any]]) -> tuple[dict[int, dict[str, Any]], dict[int, int]]:
    module = p.module_base
    army_vtable = module + ARMY_VTABLE_RVA
    army_group_vtable = module + ARMY_GROUP_VTABLE_RVA
    armies: dict[int, dict[str, Any]] = {}
    ptr_to_army: dict[int, int] = {}
    member_votes: dict[int, list[dict[str, Any]]] = defaultdict(list)
    for row in live_rows:
        parent = row.get("army_parent_ptr")
        if not is_heap(parent):
            continue
        if safe_u64(p, int(parent)) != army_vtable:
            continue
        army_id = object_handle_id(p, int(parent), ARMY_TYPE)
        if army_id is None:
            continue
        ptr_to_army[int(parent)] = army_id
        member_votes[army_id].append(row)
    for army_ptr, army_id in sorted((ptr, aid) for ptr, aid in ptr_to_army.items()):
        ag_ptr = safe_u64(p, army_ptr + ARMY_OFF_ARMY_GROUP_PARENT)
        ag_id = None
        if is_heap(ag_ptr) and safe_u64(p, int(ag_ptr)) == army_group_vtable:
            ag_id = object_handle_id(p, int(ag_ptr), ARMY_TYPE)
        armies[army_id] = {
            "army_id": army_id,
            "army_addr": army_ptr,
            "army_addr_hex": hx(army_ptr),
            "engine_member_count": safe_u32(p, army_ptr + ARMY_OFF_ENGINE_MEMBER_COUNT),
            "leader_ptr": hx(safe_u64(p, army_ptr + ARMY_OFF_LEADER_PTR)),
            "army_group_id": ag_id,
            "army_group_ptr": hx(ag_ptr),
            "live_member_division_ids": sorted({int(row["div_id"]) for row in member_votes.get(army_id, [])}),
            "live_member_count": len(member_votes.get(army_id, [])),
        }
    return armies, ptr_to_army


def enumerate_orders(p: Any, armies: dict[int, dict[str, Any]]) -> tuple[dict[int, dict[str, Any]], dict[int, int]]:
    order_vtable = p.module_base + ORDER_VTABLE_RVA
    orders: dict[int, dict[str, Any]] = {}
    ptr_to_order: dict[int, int] = {}
    for army_id, army in armies.items():
        army_addr = int(army["army_addr"])
        begin = safe_u64(p, army_addr + ARMY_OFF_ORDERS_VECTOR)
        if not is_heap(begin):
            continue
        for slot in range(MAX_ARMY_ORDERS):
            order_ptr = safe_u64(p, int(begin) + slot * 8)
            if not is_heap(order_ptr):
                break
            if safe_u64(p, int(order_ptr)) != order_vtable:
                continue
            if safe_u64(p, int(order_ptr) + ORDER_OFF_ARMY_BACKREF) != army_addr:
                continue
            order_type = safe_i32(p, int(order_ptr) + ORDER_OFF_TYPE)
            if not isinstance(order_type, int):
                continue
            order_key = int(order_ptr)
            ptr_to_order[order_key] = order_key
            target_state_count = None
            target_status = "unsupported"
            if order_type == 5:
                vec = safe_u64(p, order_key + ORDER_OFF_TYPE5_STATES_BEGIN)
                count = safe_u32(p, order_key + ORDER_OFF_TYPE5_STATES_SIZE)
                target_state_count = count
                if count == 0:
                    target_status = "exact_empty"
                elif isinstance(count, int) and 0 < count <= 5000 and is_heap(vec, 4):
                    raw = p.try_read(int(vec), count * 4)
                    target_status = "exact" if len(raw) == count * 4 else "validation_failed"
                else:
                    target_status = "validation_failed"
            orders[order_key] = {
                "order_addr": order_key,
                "order_addr_hex": hx(order_key),
                "army_id": army_id,
                "army_addr": army_addr,
                "slot_index": slot,
                "order_type": order_type,
                "target_state_count": target_state_count,
                "target_status": target_status,
            }
    return orders, ptr_to_order


def assignment_positive_control(
    live_rows: list[dict[str, Any]],
    oracle_map: dict[str, dict[str, Any]],
    ptr_to_army: dict[int, int],
) -> dict[str, Any]:
    eligible = matched = mismatch = no_live_parent = no_oracle_assignment = 0
    samples = []
    for row in live_rows:
        key = row.get("owner_division_key")
        if not isinstance(key, str) or key not in oracle_map:
            continue
        oracle_assignment = oracle_map[key].get("assignment")
        if not isinstance(oracle_assignment, dict) or not isinstance(oracle_assignment.get("army_id"), int):
            no_oracle_assignment += 1
            continue
        eligible += 1
        army_id = ptr_to_army.get(int(row.get("army_parent_ptr") or 0))
        if army_id is None:
            no_live_parent += 1
            continue
        if army_id == oracle_assignment["army_id"]:
            matched += 1
            if len(samples) < 12:
                samples.append(
                    {
                        "owner_division_key": key,
                        "army_id": army_id,
                        "army_parent_ptr": hx(row.get("army_parent_ptr")),
                    }
                )
        else:
            mismatch += 1
            if len(samples) < 12:
                samples.append(
                    {
                        "owner_division_key": key,
                        "oracle_army_id": oracle_assignment["army_id"],
                        "live_army_id": army_id,
                        "army_parent_ptr": hx(row.get("army_parent_ptr")),
                    }
                )
    status = "positive_control_exact" if eligible > 0 and mismatch == 0 and matched > 0 else "positive_control_not_closed"
    return {
        "candidate": "CDivision+0xB0 -> CArmy",
        "status": status,
        "eligible_oracle_assignment_rows": eligible,
        "matched_rows": matched,
        "mismatch_rows": mismatch,
        "no_live_parent_rows": no_live_parent,
        "no_oracle_assignment_rows": no_oracle_assignment,
        "samples": samples,
        "position": "positive control and current production source for active-army assignment, not a newly promoted root",
    }


def scan_division_pointer_fields(
    p: Any,
    live_rows: list[dict[str, Any]],
    oracle_map: dict[str, dict[str, Any]],
    ptr_to_army: dict[int, int],
    ptr_to_order: dict[int, int],
    orders: dict[int, dict[str, Any]],
    body_scan: int,
) -> list[dict[str, Any]]:
    body_blobs: dict[str, bytes] = {}
    for row in live_rows:
        key = row.get("owner_division_key")
        if not isinstance(key, str):
            continue
        blob = p.try_read(int(row["body_addr"]), body_scan)
        if len(blob) >= body_scan:
            body_blobs[key] = blob
    rows = []
    for offset in range(0, body_scan - 7, 8):
        army_hits = order_hits = heap_hits = matched_army = matched_order_army = 0
        target_vtables = Counter()
        examples = []
        for row in live_rows:
            key = row.get("owner_division_key")
            blob = body_blobs.get(str(key))
            if not blob:
                continue
            pointer = struct.unpack_from("<Q", blob, offset)[0]
            if not is_heap(pointer):
                continue
            heap_hits += 1
            vt = safe_u64(p, pointer)
            if isinstance(vt, int):
                target_vtables[hx(vt - p.module_base) if p.in_module(vt) else hx(vt)] += 1
            if pointer in ptr_to_army:
                army_hits += 1
                oracle_assignment = oracle_map.get(str(key), {}).get("assignment")
                if isinstance(oracle_assignment, dict) and oracle_assignment.get("army_id") == ptr_to_army[pointer]:
                    matched_army += 1
            if pointer in ptr_to_order:
                order_hits += 1
                order = orders.get(pointer, {})
                oracle_assignment = oracle_map.get(str(key), {}).get("assignment")
                if isinstance(oracle_assignment, dict) and oracle_assignment.get("army_id") == order.get("army_id"):
                    matched_order_army += 1
            if len(examples) < 6 and (pointer in ptr_to_army or pointer in ptr_to_order):
                examples.append(
                    {
                        "owner_division_key": key,
                        "pointer": hx(pointer),
                        "target": "army" if pointer in ptr_to_army else "order",
                        "target_id": ptr_to_army.get(pointer) or orders.get(pointer, {}).get("slot_index"),
                    }
                )
        if offset == DIVISION_OFF_ARMY_PARENT and army_hits:
            status = "positive_control_army_parent"
            reason = "known CDivision+0xB0 active-army parent path"
        elif army_hits or order_hits:
            status = "candidate_requires_fixture_or_join_gate"
            reason = "pointer resolves to validated CArmy/order objects; needs exact assignment/order semantics"
        elif heap_hits:
            status = "rejected_no_valid_army_or_order_target"
            reason = "heap pointers present but none resolve to validated CArmy/order targets"
        else:
            status = "rejected_no_heap_pointer_coverage"
            reason = "no heap pointers at this body offset"
        rows.append(
            {
                "offset": f"+0x{offset:X}",
                "heap_hits": heap_hits,
                "army_pointer_hits": army_hits,
                "order_pointer_hits": order_hits,
                "matched_oracle_army_rows": matched_army,
                "matched_order_backref_army_rows": matched_order_army,
                "top_target_vtables_or_first_qword": dict(target_vtables.most_common(8)),
                "examples": examples,
                "status": status,
                "reason": reason,
            }
        )
    rows.sort(
        key=lambda row: (
            row["status"] not in ("positive_control_army_parent", "candidate_requires_fixture_or_join_gate"),
            -row["matched_oracle_army_rows"],
            -row["army_pointer_hits"],
            -row["order_pointer_hits"],
            row["offset"],
        )
    )
    return rows


def decode_division_ptr(p: Any, pointer: int) -> int | None:
    if not is_heap(pointer):
        return None
    if safe_u64(p, pointer) == p.module_base + DIVISION_VTABLE_RVA:
        div_type = safe_u32(p, pointer + DIVISION_OFF_TYPE)
        div_id = safe_u32(p, pointer + DIVISION_OFF_ID)
        if div_type in (DIVISION_TYPE, DIVISION_ALT_UNIT_TYPE) and isinstance(div_id, int):
            return div_id
    body = pointer + DIVISION_WRAPPER_TO_BODY
    if safe_u64(p, body) == p.module_base + DIVISION_VTABLE_RVA:
        div_type = safe_u32(p, body + DIVISION_OFF_TYPE)
        div_id = safe_u32(p, body + DIVISION_OFF_ID)
        if div_type in (DIVISION_TYPE, DIVISION_ALT_UNIT_TYPE) and isinstance(div_id, int):
            return div_id
    return None


def scan_army_member_vectors(p: Any, armies: dict[int, dict[str, Any]], army_scan: int) -> list[dict[str, Any]]:
    results = []
    offsets = range(0, max(0, army_scan - 16), 4)
    for offset in offsets:
        for stride in (8, 16, 24, 32):
            armies_with_vector = exact_live_sets = subset_live_sets = rejected_vectors = 0
            examples = []
            for army_id, army in armies.items():
                army_addr = int(army["army_addr"])
                begin = safe_u64(p, army_addr + offset)
                capacity = safe_u32(p, army_addr + offset + 8)
                count = safe_u32(p, army_addr + offset + 12)
                if not is_heap(begin) or not isinstance(count, int) or not isinstance(capacity, int):
                    continue
                if count <= 0 or count > capacity or count > MAX_VECTOR_COUNT:
                    rejected_vectors += 1
                    continue
                raw = p.try_read(int(begin), count * stride)
                if len(raw) != count * stride:
                    rejected_vectors += 1
                    continue
                ids = []
                for index in range(count):
                    pointer = struct.unpack_from("<Q", raw, index * stride)[0]
                    div_id = decode_division_ptr(p, pointer)
                    if div_id is not None:
                        ids.append(div_id)
                if not ids:
                    continue
                armies_with_vector += 1
                got = set(ids)
                want = set(int(v) for v in army.get("live_member_division_ids", []))
                if got == want and want:
                    exact_live_sets += 1
                elif got and got <= want:
                    subset_live_sets += 1
                if len(examples) < 8:
                    examples.append(
                        {
                            "army_id": army_id,
                            "offset": f"+0x{offset:X}",
                            "stride": stride,
                            "count": count,
                            "decoded_division_ids_sample": sorted(got)[:16],
                            "live_backref_ids_sample": sorted(want)[:16],
                            "exact_live_set": got == want and bool(want),
                        }
                    )
            if armies_with_vector == 0 and rejected_vectors == 0:
                continue
            if exact_live_sets:
                status = "candidate_army_member_vector_exact_live_backref_sets"
                reason = "decoded vector matches CDivision+0xB0 member sets for at least one army"
            elif armies_with_vector:
                status = "rejected_partial_or_unmatched_member_vector"
                reason = "decoded division-like entries did not exactly match live backref sets"
            else:
                status = "rejected_invalid_vector_header"
                reason = "vector headers were invalid or unreadable"
            results.append(
                {
                    "candidate": f"CArmy+0x{offset:X} vector stride {stride}",
                    "offset": f"+0x{offset:X}",
                    "stride": stride,
                    "armies_with_vector": armies_with_vector,
                    "exact_live_backref_sets": exact_live_sets,
                    "subset_live_backref_sets": subset_live_sets,
                    "rejected_vectors": rejected_vectors,
                    "examples": examples,
                    "status": status,
                    "reason": reason,
                }
            )
    results.sort(
        key=lambda row: (
            not str(row["status"]).startswith("candidate_"),
            -row["exact_live_backref_sets"],
            -row["armies_with_vector"],
            row["candidate"],
        )
    )
    return results


def scan_strategic_redeploy_candidates(
    p: Any,
    live_rows: list[dict[str, Any]],
    oracle_map: dict[str, dict[str, Any]],
    body_scan: int,
) -> dict[str, Any]:
    sr_keys = {
        key
        for key, row in oracle_map.items()
        if isinstance(row.get("strategic_redeploy_save_hints"), dict)
        and row.get("strategic_redeploy_save_hints")
    }
    eligible = [row for row in live_rows if isinstance(row.get("owner_division_key"), str) and row["owner_division_key"] in oracle_map]
    if not sr_keys:
        return {
            "status": "fixture_blocked_no_save_strategic_redeploy_rows",
            "reason": "save oracle exposed no strategic/redeploy/transport/embark scalar hints",
            "already_mapped_movement_fields": {
                "moving": "CDivision+0x230 nonzero",
                "movement_progress": "CDivision+0x230 u32/100000",
                "move_target_province": "CDivision+0x18 while moving",
                "movement_source_province": "CDivision+0x1E8 -> +0xA4 behind movement-order vtable",
                "known_gap": "retreat/strategic-redeploy/naval-transport mode semantics are not proven by these fields",
            },
            "candidate_offsets": [],
        }
    sr_rows = [row for row in eligible if row.get("owner_division_key") in sr_keys]
    non_sr_rows = [row for row in eligible if row.get("owner_division_key") not in sr_keys]
    blobs: dict[str, bytes] = {}
    for row in eligible:
        key = row.get("owner_division_key")
        if not isinstance(key, str):
            continue
        blob = p.try_read(int(row["body_addr"]), body_scan)
        if len(blob) >= body_scan:
            blobs[key] = blob
    candidates = []
    for offset in range(0, body_scan - 3, 4):
        sr_values = Counter()
        non_values = Counter()
        for row in sr_rows:
            blob = blobs.get(str(row.get("owner_division_key")))
            if blob:
                value = struct.unpack_from("<I", blob, offset)[0]
                if value <= MAX_LOW_CARDINALITY_VALUE:
                    sr_values[value] += 1
        for row in non_sr_rows:
            blob = blobs.get(str(row.get("owner_division_key")))
            if blob:
                value = struct.unpack_from("<I", blob, offset)[0]
                if value <= MAX_LOW_CARDINALITY_VALUE:
                    non_values[value] += 1
        if not sr_values:
            continue
        sr_top, sr_count = sr_values.most_common(1)[0]
        non_same = non_values.get(sr_top, 0)
        if sr_count == len(sr_rows) and non_same == 0:
            status = "candidate_exact_partition_requires_live_sr_fixture"
        elif sr_count > non_same:
            status = "candidate_enriched_requires_live_sr_fixture"
        else:
            status = "rejected_not_sr_partition"
        candidates.append(
            {
                "offset": f"+0x{offset:X}",
                "encoding": "u32_low_cardinality",
                "sr_rows": len(sr_rows),
                "non_sr_rows": len(non_sr_rows),
                "sr_top_value": sr_top,
                "sr_top_count": sr_count,
                "non_sr_same_value_count": non_same,
                "sr_values": dict(sr_values.most_common(8)),
                "non_sr_values": dict(non_values.most_common(8)),
                "status": status,
            }
        )
    candidates.sort(
        key=lambda row: (
            not str(row["status"]).startswith("candidate_"),
            -row["sr_top_count"],
            row["non_sr_same_value_count"],
            row["offset"],
        )
    )
    return {
        "status": "candidate_scan_completed_requires_live_sr_fixture",
        "sr_oracle_rows": len(sr_rows),
        "non_sr_oracle_rows": len(non_sr_rows),
        "already_mapped_movement_fields": {
            "moving": "CDivision+0x230 nonzero",
            "movement_progress": "CDivision+0x230 u32/100000",
            "move_target_province": "CDivision+0x18 while moving",
            "movement_source_province": "CDivision+0x1E8 -> +0xA4 behind movement-order vtable",
            "known_gap": "retreat/strategic-redeploy/naval-transport mode semantics are not proven by these fields",
        },
        "candidate_offsets": candidates,
    }


def compact_live_rows(rows: list[dict[str, Any]], limit: int) -> list[dict[str, Any]]:
    out = []
    for row in rows[:limit]:
        out.append(
            {
                "owner_division_key": row.get("owner_division_key"),
                "division_key": row.get("division_key"),
                "div_id": row.get("div_id"),
                "owner": row.get("owner"),
                "owner_tag_db_id": row.get("owner_tag_db_id"),
                "body_addr": hx(row.get("body_addr")),
                "army_parent_ptr": hx(row.get("army_parent_ptr")),
                "location_province": row.get("location_province"),
                "move_progress_raw": row.get("move_progress_raw"),
                "combat_ref_count": row.get("combat_ref_count"),
            }
        )
    return out


def build_report(args: argparse.Namespace) -> dict[str, Any]:
    oracle = load_oracle(args.oracle)
    oracle_map = oracle_by_owner_key(oracle)
    p = winmem.Process(args.pid, PROC_NAME) if args.pid else winmem.open_process(PROC_NAME)
    if p is None:
        raise RuntimeError("hoi4.exe not running")
    with p:
        started = time.perf_counter()
        info = process_info(p)
        report: dict[str, Any] = {
            "schema": SCHEMA,
            "probe": "probe_870ebf_division_assignment",
            "snapshot_label": args.snapshot_label,
            "generated_utc": datetime.now(timezone.utc).isoformat(timespec="seconds"),
            "expected_sha256": EXPECTED_SHA256,
            "process": info,
            "build_gate_ok": info.get("sha256") == EXPECTED_SHA256,
            "fail_closed": info.get("sha256") != EXPECTED_SHA256,
            "oracle": {
                "path": str(args.oracle),
                "save_date": oracle.get("source", {}).get("save_date"),
                "stats": oracle.get("stats", {}),
            },
        }
        if not report["build_gate_ok"]:
            report["status"] = "build_gate_failed"
            return report
        tags = load_tags(p)
        live_rows, enum_stats = enumerate_divisions(p, tags, sample_limit=args.sample_limit)
        armies, ptr_to_army = validate_armies_from_divisions(p, live_rows)
        orders, ptr_to_order = enumerate_orders(p, armies)
        body_scan = min(max(0x100, args.body_scan_bytes), MAX_BODY_SCAN)
        army_scan = min(max(0x80, args.army_scan_bytes), MAX_ARMY_SCAN)
        report.update(
            {
                "status": "probe_completed_candidates_only",
                "elapsed_ms": round((time.perf_counter() - started) * 1000, 3),
                "tag_table": {
                    "tag_count_read": len(tags),
                    "samples": [{"tag_db_id": key, "tag": value} for key, value in list(sorted(tags.items()))[:16]],
                },
                "validated_roots": {
                    "CGameState": f"module+{hx(CGAMESTATE_RVA)}",
                    "country_table": f"CGameState+{hx(CGAMESTATE_COUNTRY_TABLE)} count CGameState+{hx(CGAMESTATE_COUNTRY_COUNT)}",
                    "CCountry_vtable": f"module+{hx(COUNTRY_VTABLE_RVA)}",
                    "CDivision": f"CCountry+0x290 wrapper vector -> wrapper+0x10 body vtable module+{hx(DIVISION_VTABLE_RVA)}",
                    "CArmy_positive_control": f"CDivision+{hx(DIVISION_OFF_ARMY_PARENT)} -> vtable module+{hx(ARMY_VTABLE_RVA)} handle type {ARMY_TYPE}",
                    "CArmy_orders": f"CArmy+{hx(ARMY_OFF_ORDERS_VECTOR)} -> COrder vtable module+{hx(ORDER_VTABLE_RVA)} backref COrder+{hx(ORDER_OFF_ARMY_BACKREF)}",
                },
                "enumeration_stats": enum_stats,
                "live_rows": compact_live_rows(live_rows, args.output_live_row_limit),
                "army_summary": {
                    "armies_from_division_parents": len(armies),
                    "orders_from_active_armies": len(orders),
                    "order_type_histogram": dict(Counter(row["order_type"] for row in orders.values())),
                    "sample_armies": list(armies.values())[:40],
                    "sample_orders": list(orders.values())[:80],
                },
                "assignment_positive_control": assignment_positive_control(live_rows, oracle_map, ptr_to_army),
                "division_pointer_field_candidates": scan_division_pointer_fields(
                    p, live_rows, oracle_map, ptr_to_army, ptr_to_order, orders, body_scan
                ),
                "army_member_vector_candidates": scan_army_member_vectors(p, armies, army_scan),
                "strategic_redeploy_candidates": scan_strategic_redeploy_candidates(
                    p, live_rows, oracle_map, body_scan
                ),
                "promotion_policy": (
                    "No field in this report is promoted. CDivision+0xB0 is a positive-control active-army route; "
                    "new pointer/vector/SR candidates require same-frame oracle or controlled UI/live fixture evidence."
                ),
            }
        )
        return report


def write_report(path: Path, report: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pid", type=int, default=None)
    parser.add_argument("--oracle", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--snapshot-label", default="single_live_snapshot")
    parser.add_argument("--sample-limit", type=int, default=None)
    parser.add_argument("--body-scan-bytes", type=int, default=0x680)
    parser.add_argument("--army-scan-bytes", type=int, default=0x240)
    parser.add_argument("--output-live-row-limit", type=int, default=80)
    args = parser.parse_args(argv)
    report = build_report(args)
    write_report(args.out, report)
    view = {
        "status": report.get("status"),
        "out": str(args.out),
        "build_gate_ok": report.get("build_gate_ok"),
        "division_rows": report.get("enumeration_stats", {}).get("division_rows"),
        "armies_from_division_parents": report.get("army_summary", {}).get("armies_from_division_parents"),
        "orders_from_active_armies": report.get("army_summary", {}).get("orders_from_active_armies"),
        "assignment_positive_control": report.get("assignment_positive_control", {}).get("status"),
        "sr_status": report.get("strategic_redeploy_candidates", {}).get("status"),
    }
    print(json.dumps(view, indent=2, sort_keys=True))
    return 0 if report.get("build_gate_ok") else 2


if __name__ == "__main__":
    raise SystemExit(main())
