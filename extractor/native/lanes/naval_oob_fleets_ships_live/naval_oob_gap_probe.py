#!/usr/bin/env python3
"""Lane-local live probe for remaining naval OOB gaps.

This is a read-only validation/proof tool. It intentionally mirrors the current
DLL compact naval OOB traversal and writes proof JSON for fields that are not
yet integrated into the shared collector/recorder entrypoints.
"""

from __future__ import annotations

import argparse
import json
import os
import struct
import sys
import time
from collections import Counter, defaultdict
from typing import Any, Dict, Iterable, List, Optional, Tuple

LANE_DIR = os.path.dirname(os.path.abspath(__file__))
REPO_ROOT = os.path.abspath(os.path.join(LANE_DIR, "..", "..", ".."))
MEMORY_READER = os.path.join(REPO_ROOT, "extractor", "memory_reader")
if MEMORY_READER not in sys.path:
    sys.path.insert(0, MEMORY_READER)

import winmem as wm  # noqa: E402


CG_STATE_DATE_OFFSET = 0x3316E90
CGAMESTATE_COUNTRY_TABLE = 0x310
CGAMESTATE_COUNTRY_COUNT = 0x31C

COUNTRY_VTABLE_RVA = 0x27D0E90
OBJ_TAG_DB_ID = 0x08

FLEET_VTABLE_RVA = 0x294BF28
TASK_FORCE_SUBOBJECT_VTABLE_RVA = 0x294C480
TASK_FORCE_VTABLE_RVA = 0x294C660
THEATRE_VTABLE_RVA = 0x295EE18
OOB_TYPE_FLEET = 61
OOB_TYPE_THEATRE = 67

NAVY_LEADER_VTABLE_RVA = 0x293F650
LEADER_TYPE = 4713
LEADER_OFF_ID = 0x0C
UNIT_LEADER_SKILL_SUBOBJ_PTR = 0xE60
UNIT_LEADER_SKILL_IN_SUBOBJ = 0x1B8
UNIT_LEADER_TRAIT_CAP = 0xDD0
UNIT_LEADER_TRAIT_SIZE = 0xDD4
MAX_UNIT_LEADER_SKILL = 15

CPROVINCE_VTABLE_RVA = 0x295AF68
CPROVINCE_OFF_ID = 0xA4

CSHIP_VTABLE_RVA = 0x2941748
CSHIP_TYPE = 51
CSHIP_OFF_HANDLE = 0x08
CSHIP_OFF_SUNK_CONVOYS = 0x3C
CSHIP_OFF_DEFINITION = 0x98
CSHIP_OFF_CATEGORY = 0xE0
CSHIP_OFF_STRENGTH = 0x6F8
CSHIP_OFF_ORGANISATION = 0x700
CSHIP_OFF_EXPERIENCE = 0x708
CSHIP_OFF_REF100 = 0x710
CSHIP_REF100_RAW = 10_000_000

TASK_FORCE_OFF_LOCATION_PROVINCE = 0x18
TASK_FORCE_OFF_FLEET_PARENT = 0x330
TASK_FORCE_OFF_SHIP_BEGIN = 0x338
TASK_FORCE_OFF_SHIP_COUNT = 0x344

COUNTRY_FLEET_VECTOR_DATA = 0x278
COUNTRY_FLEET_VECTOR_CAPACITY = 0x280
COUNTRY_FLEET_VECTOR_COUNT = 0x284

COMPACT_FLEET_SCAN_SPAN = 0x1800
TASK_FORCE_SCAN_SPAN = 0x800
SHIP_STRING_SCAN_START = 0x40
SHIP_STRING_SCAN_END = 0x800
TASK_FORCE_SUBOBJECT_TO_BODY_DELTAS = (-0x780, 0x10, 0x7A0, 0xF30)

MAX_COUNTRY_SLOTS = 4096
MAX_COUNTRY_FLEET_ROWS = 4096
MAX_FLEET_POINTER_VECTOR_ROWS = 20_000
MAX_FLEET_POINTER_VECTOR_CAPACITY = 40_000
MAX_OOB_TASK_FORCE_SHIPS = 20_000
MAX_OOB_ID = 4_000_000
MAX_SHIP_INSTANCE_ID = 4_000_000
MAX_PROVINCE_ID = 1_000_000
MAX_REGION_ID = 4096

HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000

PROV2REGION_PATH = os.path.join(
    REPO_ROOT, "extractor", "memory_reader", "weather", "prov2region_mod.json"
)


def hx(value: Optional[int]) -> Optional[str]:
    if value is None:
        return None
    return "0x%X" % value


def now_ms() -> int:
    return int(time.time() * 1000)


def safe_read(proc: wm.Process, addr: int, size: int) -> Optional[bytes]:
    if addr is None or size < 0:
        return None
    try:
        data = proc.try_read(addr, size)
    except OSError:
        return None
    if len(data) != size:
        return None
    return data


def safe_u8(proc: wm.Process, addr: int) -> Optional[int]:
    data = safe_read(proc, addr, 1)
    return data[0] if data else None


def safe_u32(proc: wm.Process, addr: int) -> Optional[int]:
    data = safe_read(proc, addr, 4)
    return struct.unpack_from("<I", data, 0)[0] if data else None


def safe_u64(proc: wm.Process, addr: int) -> Optional[int]:
    data = safe_read(proc, addr, 8)
    return struct.unpack_from("<Q", data, 0)[0] if data else None


def u32_from(data: bytes, off: int) -> Optional[int]:
    if off < 0 or off + 4 > len(data):
        return None
    return struct.unpack_from("<I", data, off)[0]


def u64_from(data: bytes, off: int) -> Optional[int]:
    if off < 0 or off + 8 > len(data):
        return None
    return struct.unpack_from("<Q", data, off)[0]


def is_heap_ptr(value: Optional[int]) -> bool:
    return value is not None and HEAP_LO <= value < HEAP_HI


def is_heap_aligned(value: Optional[int], align: int = 8) -> bool:
    return is_heap_ptr(value) and value % align == 0


def read_msvc_string(
    proc: wm.Process, addr: int, max_len: int = 128, utf8_tolerant: bool = True
) -> Optional[str]:
    size = safe_u64(proc, addr + 0x10)
    cap = safe_u64(proc, addr + 0x18)
    if size is None or cap is None:
        return None
    if size == 0 or size > max_len or cap < size or cap > (1 << 20):
        return None
    chars_addr = addr if cap < 16 else safe_u64(proc, addr)
    if not is_heap_ptr(chars_addr) and chars_addr != addr:
        return None
    raw = safe_read(proc, chars_addr, size + 1)
    if raw is None or len(raw) != size + 1 or raw[size] != 0:
        return None
    payload = raw[:size]
    if not utf8_tolerant and not all(32 <= b < 127 for b in payload):
        return None
    try:
        text = payload.decode("utf-8")
    except UnicodeDecodeError:
        try:
            text = payload.decode("latin-1")
        except UnicodeDecodeError:
            return None
    if any(ord(ch) < 32 for ch in text):
        return None
    return text


def token_like(value: str) -> bool:
    if not value:
        return False
    if " " in value or "'" in value:
        return False
    allowed = set("_-.0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ")
    return all(ch in allowed for ch in value)


def validate_oob_body(
    proc: wm.Process, module_base: int, body: int, vtable_rva: int, type_tag: int
) -> Optional[int]:
    if not is_heap_aligned(body):
        return None
    if safe_u64(proc, body) != module_base + vtable_rva:
        return None
    handle = safe_u64(proc, body + 0x08)
    if handle is None or (handle & 0xFFFF_FFFF) != type_tag:
        return None
    object_id = (handle >> 32) & 0xFFFF_FFFF
    if object_id == 0 or object_id > MAX_OOB_ID:
        return None
    return object_id


def validate_country(
    proc: wm.Process, module_base: int, country_ptr: int, slot_id: int
) -> Optional[int]:
    if not is_heap_aligned(country_ptr):
        return None
    if safe_u64(proc, country_ptr) != module_base + COUNTRY_VTABLE_RVA:
        return None
    tag_db_id = safe_u32(proc, country_ptr + OBJ_TAG_DB_ID)
    if tag_db_id is None or tag_db_id == 0 or tag_db_id > MAX_COUNTRY_SLOTS:
        return None
    if tag_db_id != slot_id:
        return None
    return tag_db_id


def read_cship_instance_id(proc: wm.Process, module_base: int, ship_ptr: int) -> Optional[int]:
    if not is_heap_aligned(ship_ptr):
        return None
    if safe_u64(proc, ship_ptr) != module_base + CSHIP_VTABLE_RVA:
        return None
    handle = safe_u64(proc, ship_ptr + CSHIP_OFF_HANDLE)
    if handle is None or (handle & 0xFFFF_FFFF) != CSHIP_TYPE:
        return None
    instance_id = (handle >> 32) & 0xFFFF_FFFF
    if instance_id == 0 or instance_id > MAX_SHIP_INSTANCE_ID:
        return None
    if safe_u32(proc, ship_ptr + CSHIP_OFF_REF100) != CSHIP_REF100_RAW:
        return None
    return instance_id


def validate_task_force_ship_vector(
    proc: wm.Process, module_base: int, task_force_body: int
) -> bool:
    count = safe_u32(proc, task_force_body + TASK_FORCE_OFF_SHIP_COUNT)
    if count is None or count > MAX_OOB_TASK_FORCE_SHIPS:
        return False
    if count == 0:
        return True
    begin = safe_u64(proc, task_force_body + TASK_FORCE_OFF_SHIP_BEGIN)
    if not is_heap_aligned(begin):
        return False
    raw = safe_read(proc, begin, count * 8)
    if raw is None:
        return False
    seen = set()
    for index in range(count):
        ship_ptr = u64_from(raw, index * 8)
        instance_id = read_cship_instance_id(proc, module_base, ship_ptr or 0)
        if instance_id is None or instance_id in seen:
            return False
        seen.add(instance_id)
    return True


def validate_task_force_body(
    proc: wm.Process,
    module_base: int,
    body_ptr: int,
    fleet_roots_by_ptr: Dict[int, Dict[str, Any]],
) -> Optional[Tuple[int, Dict[str, Any]]]:
    task_force_id = validate_oob_body(
        proc, module_base, body_ptr, TASK_FORCE_VTABLE_RVA, OOB_TYPE_FLEET
    )
    if task_force_id is None:
        return None
    parent_ptr = safe_u64(proc, body_ptr + TASK_FORCE_OFF_FLEET_PARENT)
    fleet_root = fleet_roots_by_ptr.get(parent_ptr or 0)
    if fleet_root is None:
        return None
    count = safe_u32(proc, body_ptr + TASK_FORCE_OFF_SHIP_COUNT)
    if count is None or count > MAX_OOB_TASK_FORCE_SHIPS:
        return None
    if count > 0:
        begin = safe_u64(proc, body_ptr + TASK_FORCE_OFF_SHIP_BEGIN)
        if not is_heap_aligned(begin):
            return None
    if not validate_task_force_ship_vector(proc, module_base, body_ptr):
        return None
    return task_force_id, fleet_root


def maybe_insert_task_force(
    proc: wm.Process,
    module_base: int,
    body_ptr: int,
    fleet_roots_by_ptr: Dict[int, Dict[str, Any]],
    task_forces_by_ptr: Dict[int, Dict[str, Any]],
) -> bool:
    validated = validate_task_force_body(proc, module_base, body_ptr, fleet_roots_by_ptr)
    if validated is None:
        return False
    task_force_id, fleet_root = validated
    task_forces_by_ptr.setdefault(
        body_ptr,
        {
            "task_force_id": task_force_id,
            "body_ptr": body_ptr,
            "fleet_id": fleet_root["fleet_id"],
            "fleet_ptr": fleet_root["body_ptr"],
            "owner_tag_db_id": fleet_root.get("owner_tag_db_id"),
        },
    )
    return True


def collect_country_fleet_roots(
    proc: wm.Process,
    module_base: int,
    country_ptr: int,
    owner_tag_db_id: int,
    fleets_by_ptr: Dict[int, Dict[str, Any]],
) -> None:
    vector_data = safe_u64(proc, country_ptr + COUNTRY_FLEET_VECTOR_DATA)
    capacity = safe_u32(proc, country_ptr + COUNTRY_FLEET_VECTOR_CAPACITY)
    count = safe_u32(proc, country_ptr + COUNTRY_FLEET_VECTOR_COUNT)
    if (
        vector_data is None
        or capacity is None
        or count is None
        or count == 0
        or count > capacity
        or count > MAX_COUNTRY_FLEET_ROWS
        or not is_heap_aligned(vector_data)
    ):
        return
    raw = safe_read(proc, vector_data, count * 8)
    if raw is None:
        return
    for index in range(count):
        fleet_ptr = u64_from(raw, index * 8)
        fleet_id = validate_oob_body(
            proc, module_base, fleet_ptr or 0, FLEET_VTABLE_RVA, OOB_TYPE_FLEET
        )
        if fleet_id is None:
            continue
        fleets_by_ptr.setdefault(
            fleet_ptr,
            {
                "fleet_id": fleet_id,
                "body_ptr": fleet_ptr,
                "owner_tag_db_id": owner_tag_db_id,
            },
        )


def collect_task_force_roots_from_fleet(
    proc: wm.Process,
    module_base: int,
    fleet_ptr: int,
    fleet_roots_by_ptr: Dict[int, Dict[str, Any]],
    task_forces_by_ptr: Dict[int, Dict[str, Any]],
) -> None:
    blob = safe_read(proc, fleet_ptr, COMPACT_FLEET_SCAN_SPAN)
    if blob is None:
        return
    for offset in range(0, max(0, len(blob) - 8), 8):
        candidate = u64_from(blob, offset) or 0
        maybe_insert_task_force_candidate(
            proc, module_base, candidate, fleet_roots_by_ptr, task_forces_by_ptr
        )
    for offset in range(0, max(0, len(blob) - 16), 8):
        vector_data = u64_from(blob, offset)
        capacity = u32_from(blob, offset + 8)
        count = u32_from(blob, offset + 12)
        if (
            vector_data is None
            or capacity is None
            or count is None
            or count == 0
            or count > capacity
            or count > MAX_FLEET_POINTER_VECTOR_ROWS
            or capacity > MAX_FLEET_POINTER_VECTOR_CAPACITY
            or not is_heap_aligned(vector_data)
        ):
            continue
        raw = safe_read(proc, vector_data, count * 8)
        if raw is None:
            continue
        for index in range(count):
            candidate = u64_from(raw, index * 8) or 0
            maybe_insert_task_force_candidate(
                proc, module_base, candidate, fleet_roots_by_ptr, task_forces_by_ptr
            )


def maybe_insert_task_force_candidate(
    proc: wm.Process,
    module_base: int,
    candidate: int,
    fleet_roots_by_ptr: Dict[int, Dict[str, Any]],
    task_forces_by_ptr: Dict[int, Dict[str, Any]],
) -> bool:
    inserted = False
    if maybe_insert_task_force(
        proc, module_base, candidate, fleet_roots_by_ptr, task_forces_by_ptr
    ):
        inserted = True
    if not is_heap_aligned(candidate):
        return inserted
    if safe_u64(proc, candidate) != module_base + TASK_FORCE_SUBOBJECT_VTABLE_RVA:
        return inserted
    for delta in TASK_FORCE_SUBOBJECT_TO_BODY_DELTAS:
        body_ptr = candidate + delta
        if body_ptr <= 0:
            continue
        if maybe_insert_task_force(
            proc, module_base, body_ptr, fleet_roots_by_ptr, task_forces_by_ptr
        ):
            inserted = True
    return inserted


def discover_compact_naval_roots(proc: wm.Process) -> Dict[str, Any]:
    module_base = proc.module_base
    game_state = safe_u64(proc, module_base + CG_STATE_DATE_OFFSET)
    if not is_heap_aligned(game_state):
        raise RuntimeError("CGameState root failed: module+0x%X -> %s" % (CG_STATE_DATE_OFFSET, hx(game_state)))
    country_table = safe_u64(proc, game_state + CGAMESTATE_COUNTRY_TABLE)
    country_count = safe_u32(proc, game_state + CGAMESTATE_COUNTRY_COUNT)
    if (
        country_table is None
        or country_count is None
        or country_count == 0
        or country_count > MAX_COUNTRY_SLOTS
        or not is_heap_aligned(country_table)
    ):
        raise RuntimeError("country table root failed")
    table_raw = safe_read(proc, country_table, country_count * 8)
    if table_raw is None:
        raise RuntimeError("country table read failed")

    countries: List[Dict[str, Any]] = []
    fleets_by_ptr: Dict[int, Dict[str, Any]] = {}
    for tag_db_id in range(1, country_count):
        country_ptr = u64_from(table_raw, tag_db_id * 8) or 0
        owner = validate_country(proc, module_base, country_ptr, tag_db_id)
        if owner is None:
            continue
        countries.append({"tag_db_id": owner, "body_ptr": country_ptr})
        collect_country_fleet_roots(proc, module_base, country_ptr, owner, fleets_by_ptr)

    task_forces_by_ptr: Dict[int, Dict[str, Any]] = {}
    for fleet in list(fleets_by_ptr.values()):
        collect_task_force_roots_from_fleet(
            proc, module_base, fleet["body_ptr"], fleets_by_ptr, task_forces_by_ptr
        )

    fleets = sorted(fleets_by_ptr.values(), key=lambda row: row["fleet_id"])
    task_forces = sorted(task_forces_by_ptr.values(), key=lambda row: row["task_force_id"])
    return {
        "module_base": module_base,
        "game_state": game_state,
        "country_table": country_table,
        "country_count": country_count,
        "countries": countries,
        "fleets": fleets,
        "task_forces": task_forces,
    }


def sample_ship_rows(proc: wm.Process, roots: Dict[str, Any]) -> List[Dict[str, Any]]:
    module_base = roots["module_base"]
    ships_by_id: Dict[int, Dict[str, Any]] = {}
    for task_force in roots["task_forces"]:
        tf_ptr = task_force["body_ptr"]
        count = safe_u32(proc, tf_ptr + TASK_FORCE_OFF_SHIP_COUNT)
        begin = safe_u64(proc, tf_ptr + TASK_FORCE_OFF_SHIP_BEGIN)
        if count is None or count == 0 or count > MAX_OOB_TASK_FORCE_SHIPS:
            continue
        if not is_heap_aligned(begin):
            continue
        raw = safe_read(proc, begin, count * 8)
        if raw is None:
            continue
        for index in range(count):
            ship_ptr = u64_from(raw, index * 8) or 0
            instance_id = read_cship_instance_id(proc, module_base, ship_ptr)
            if instance_id is None:
                continue
            definition = read_msvc_string(proc, ship_ptr + CSHIP_OFF_DEFINITION, 128)
            ships_by_id.setdefault(
                instance_id,
                {
                    "instance_id": instance_id,
                    "body_ptr": ship_ptr,
                    "task_force_id": task_force["task_force_id"],
                    "fleet_id": task_force["fleet_id"],
                    "owner_tag_db_id": task_force.get("owner_tag_db_id"),
                    "definition": definition,
                    "category_enum": safe_u32(proc, ship_ptr + CSHIP_OFF_CATEGORY),
                    "strength_raw": safe_u32(proc, ship_ptr + CSHIP_OFF_STRENGTH),
                    "organisation_raw": safe_u32(proc, ship_ptr + CSHIP_OFF_ORGANISATION),
                    "experience_raw": safe_u32(proc, ship_ptr + CSHIP_OFF_EXPERIENCE),
                    "sunk_convoys": safe_u32(proc, ship_ptr + CSHIP_OFF_SUNK_CONVOYS),
                },
            )
    return sorted(ships_by_id.values(), key=lambda row: row["instance_id"])


def add_string_stat(stats: Dict[Any, Dict[str, Any]], key: Any, ship: Dict[str, Any], value: str) -> None:
    entry = stats.setdefault(
        key,
        {
            "read_count": 0,
            "unique_values": Counter(),
            "equal_definition_count": 0,
            "token_like_count": 0,
            "examples": [],
        },
    )
    entry["read_count"] += 1
    entry["unique_values"][value] += 1
    if value == ship.get("definition"):
        entry["equal_definition_count"] += 1
    if token_like(value):
        entry["token_like_count"] += 1
    if len(entry["examples"]) < 8:
        entry["examples"].append(
            {
                "ship_id": ship["instance_id"],
                "definition": ship.get("definition"),
                "value": value,
            }
        )


def compact_string_stats(stats: Dict[Any, Dict[str, Any]], total: int, limit: int = 20) -> List[Dict[str, Any]]:
    rows = []
    for key, value in stats.items():
        uniques = value["unique_values"]
        row: Dict[str, Any] = {
            "read_count": value["read_count"],
            "coverage_ratio": round(value["read_count"] / total, 4) if total else 0.0,
            "unique_count": len(uniques),
            "equal_definition_count": value["equal_definition_count"],
            "token_like_count": value["token_like_count"],
            "examples": value["examples"],
        }
        if isinstance(key, tuple):
            row["ship_ptr_field_off"] = "0x%X" % key[0]
            row["target_string_off"] = "0x%X" % key[1]
            row["target_vtable"] = hx(key[2])
        else:
            row["ship_string_off"] = "0x%X" % key
        rows.append(row)
    rows.sort(key=lambda r: (-r["read_count"], -r["unique_count"], r.get("ship_string_off", ""), r.get("ship_ptr_field_off", "")))
    return rows[:limit]


def scan_ship_name_candidates(
    proc: wm.Process, roots: Dict[str, Any], ships: List[Dict[str, Any]], max_ships: int
) -> Dict[str, Any]:
    sample = ships[:max_ships]
    direct_stats: Dict[int, Dict[str, Any]] = {}
    for ship in sample:
        body = ship["body_ptr"]
        for off in range(SHIP_STRING_SCAN_START, SHIP_STRING_SCAN_END + 1, 8):
            value = read_msvc_string(proc, body + off, 128)
            if value:
                add_string_stat(direct_stats, off, ship, value)

    direct_rows = compact_string_stats(direct_stats, len(sample), 30)
    combat_member_offsets = {}
    for off in (0x48, 0x68, 0x88):
        row = next((r for r in direct_rows if r.get("ship_string_off") == "0x%X" % off), None)
        combat_member_offsets["0x%X" % off] = row or {"read_count": 0, "coverage_ratio": 0.0}

    plausible_direct = [
        row
        for row in direct_rows
        if row.get("ship_string_off") != "0x%X" % CSHIP_OFF_DEFINITION
        and row["coverage_ratio"] >= 0.25
        and row["unique_count"] >= 2
        and row["equal_definition_count"] == 0
        and row["token_like_count"] < row["read_count"]
    ]

    indirect_stats: Dict[Tuple[int, int, int], Dict[str, Any]] = {}
    indirect_string_offsets = (
        0x00,
        0x10,
        0x18,
        0x20,
        0x28,
        0x30,
        0x38,
        0x40,
        0x48,
        0x50,
        0x58,
        0x60,
        0x68,
        0x70,
        0x78,
        0x80,
        0x88,
        0x90,
        0x98,
        0xA0,
    )
    for ship in sample:
        body = ship["body_ptr"]
        for ptr_off in range(SHIP_STRING_SCAN_START, SHIP_STRING_SCAN_END + 1, 8):
            target = safe_u64(proc, body + ptr_off)
            if not is_heap_aligned(target):
                continue
            target_vtable = safe_u64(proc, target)
            for str_off in indirect_string_offsets:
                value = read_msvc_string(proc, target + str_off, 128)
                if value:
                    add_string_stat(indirect_stats, (ptr_off, str_off, target_vtable or 0), ship, value)
    indirect_rows = compact_string_stats(indirect_stats, len(sample), 30)
    plausible_indirect = [
        row
        for row in indirect_rows
        if row["coverage_ratio"] >= 0.25
        and row["unique_count"] >= 2
        and row["equal_definition_count"] == 0
        and row["token_like_count"] < row["read_count"]
    ]

    known_definition = next(
        (r for r in direct_rows if r.get("ship_string_off") == "0x%X" % CSHIP_OFF_DEFINITION),
        None,
    )
    if plausible_direct or plausible_indirect:
        status = "blocked_fixture_required"
        verdict = (
            "candidate strings exist, but no current validated root proves persistent ship-name "
            "semantics without a UI/save instance-id fixture"
        )
    else:
        status = "rejected_current_roots"
        verdict = (
            "no plausible persistent ship-name MSVC string was found in CShip[0x40..0x800] "
            "or one bounded CShip indirect pointer layer; known CShip+0x98 resolves definitions only"
        )
    return {
        "status": status,
        "verdict": verdict,
        "sampled_ship_count": len(sample),
        "scan_ranges": {
            "direct_cship_string_offsets": "0x%X..0x%X step 8" % (SHIP_STRING_SCAN_START, SHIP_STRING_SCAN_END),
            "indirect_pointer_fields": "CShip[0x%X..0x%X] qwords, string offsets 0x00..0xA0 sampled set"
            % (SHIP_STRING_SCAN_START, SHIP_STRING_SCAN_END),
        },
        "known_definition_offset": "0x%X" % CSHIP_OFF_DEFINITION,
        "known_definition_stats": known_definition,
        "combat_member_name_offsets_tested_on_cship": combat_member_offsets,
        "plausible_direct_candidates": plausible_direct[:10],
        "plausible_indirect_candidates": plausible_indirect[:10],
        "top_direct_string_offsets": direct_rows,
        "top_indirect_string_offsets": indirect_rows,
    }


def validate_province_ptr(proc: wm.Process, module_base: int, ptr: int) -> Optional[int]:
    if not is_heap_aligned(ptr):
        return None
    if safe_u64(proc, ptr) != module_base + CPROVINCE_VTABLE_RVA:
        return None
    province_id = safe_u32(proc, ptr + CPROVINCE_OFF_ID)
    if province_id is None or province_id == 0 or province_id > MAX_PROVINCE_ID:
        return None
    return province_id


def scan_province_candidates(
    proc: wm.Process,
    module_base: int,
    objects: Iterable[Dict[str, Any]],
    object_kind: str,
    span: int,
    locations_by_ptr: Optional[Dict[int, int]] = None,
) -> Dict[str, Any]:
    ptr_offsets: Dict[int, Dict[str, Any]] = {}
    u32_offsets: Dict[int, Counter] = defaultdict(Counter)
    object_count = 0
    for obj in objects:
        object_count += 1
        body = obj["body_ptr"]
        blob = safe_read(proc, body, span)
        if blob is None:
            continue
        for off in range(0, max(0, len(blob) - 8), 8):
            qword = u64_from(blob, off) or 0
            province_id = validate_province_ptr(proc, module_base, qword)
            if province_id is not None:
                entry = ptr_offsets.setdefault(
                    off, {"count": 0, "province_ids": Counter(), "examples": []}
                )
                entry["count"] += 1
                entry["province_ids"][province_id] += 1
                if len(entry["examples"]) < 8:
                    entry["examples"].append(
                        {
                            "object_id": obj.get("task_force_id") or obj.get("fleet_id"),
                            "object_ptr": hx(body),
                            "province_id": province_id,
                        }
                    )
        for off in range(0, max(0, len(blob) - 4), 4):
            value = u32_from(blob, off)
            if value is not None and 0 < value <= 20_000:
                u32_offsets[off][value] += 1

    ptr_rows = []
    for off, value in ptr_offsets.items():
        ptr_rows.append(
            {
                "offset": "0x%X" % off,
                "coverage_ratio": round(value["count"] / object_count, 4) if object_count else 0.0,
                "count": value["count"],
                "unique_province_count": len(value["province_ids"]),
                "top_province_ids": value["province_ids"].most_common(8),
                "examples": value["examples"],
            }
        )
    ptr_rows.sort(key=lambda row: (-row["count"], row["offset"]))

    u32_rows = []
    for off, counts in u32_offsets.items():
        total = sum(counts.values())
        if total == 0:
            continue
        if object_kind == "task_force" and off == TASK_FORCE_OFF_LOCATION_PROVINCE:
            label = "validated_current_location"
        else:
            label = "unclassified_province_id_candidate"
        if total >= max(2, object_count // 4):
            u32_rows.append(
                {
                    "offset": "0x%X" % off,
                    "classification": label,
                    "coverage_ratio": round(total / object_count, 4) if object_count else 0.0,
                    "count": total,
                    "unique_value_count": len(counts),
                    "top_values": counts.most_common(8),
                }
            )
    u32_rows.sort(key=lambda row: (0 if row["classification"] == "validated_current_location" else 1, -row["count"], row["offset"]))
    return {
        "object_kind": object_kind,
        "object_count": object_count,
        "scan_span": "0x%X" % span,
        "province_pointer_offsets": ptr_rows[:30],
        "province_id_u32_offsets": u32_rows[:30],
    }


def validate_navy_leader(proc: wm.Process, module_base: int, leader_ptr: int) -> Optional[Dict[str, Any]]:
    if not is_heap_aligned(leader_ptr):
        return None
    if safe_u64(proc, leader_ptr) != module_base + NAVY_LEADER_VTABLE_RVA:
        return None
    leader_type = safe_u32(proc, leader_ptr + 0x08)
    leader_id = safe_u32(proc, leader_ptr + LEADER_OFF_ID)
    if leader_type != LEADER_TYPE or leader_id is None or leader_id == 0 or leader_id > MAX_OOB_ID:
        return None
    skill = None
    skill_subobj = safe_u64(proc, leader_ptr + UNIT_LEADER_SKILL_SUBOBJ_PTR)
    if is_heap_aligned(skill_subobj):
        raw_skill = safe_u32(proc, skill_subobj + UNIT_LEADER_SKILL_IN_SUBOBJ)
        if raw_skill is not None and raw_skill <= MAX_UNIT_LEADER_SKILL:
            skill = raw_skill
    trait_cap = safe_u32(proc, leader_ptr + UNIT_LEADER_TRAIT_CAP)
    trait_size = safe_u32(proc, leader_ptr + UNIT_LEADER_TRAIT_SIZE)
    if trait_cap is not None and trait_size is not None:
        if trait_size > trait_cap or trait_cap > 512:
            return None
    return {
        "leader_ptr": leader_ptr,
        "leader_id": leader_id,
        "skill": skill,
        "trait_size": trait_size,
        "trait_cap": trait_cap,
    }


def scan_vtable_pointer_candidates(
    proc: wm.Process,
    module_base: int,
    objects: Iterable[Dict[str, Any]],
    object_kind: str,
    span: int,
    target_vtable_rva: int,
    target_type: Optional[int] = None,
) -> Dict[str, Any]:
    offsets: Dict[int, Dict[str, Any]] = {}
    object_count = 0
    target_vtable = module_base + target_vtable_rva
    for obj in objects:
        object_count += 1
        body = obj["body_ptr"]
        blob = safe_read(proc, body, span)
        if blob is None:
            continue
        for off in range(0, max(0, len(blob) - 8), 8):
            qword = u64_from(blob, off) or 0
            for adjust in (0, 0x10):
                ptr = qword + adjust
                if not is_heap_aligned(ptr):
                    continue
                if safe_u64(proc, ptr) != target_vtable:
                    continue
                if target_type is not None and safe_u32(proc, ptr + 0x08) != target_type:
                    continue
                entry = offsets.setdefault(
                    off,
                    {
                        "count": 0,
                        "adjustments": Counter(),
                        "target_ids": Counter(),
                        "examples": [],
                    },
                )
                target_id = safe_u32(proc, ptr + 0x0C)
                entry["count"] += 1
                entry["adjustments"][adjust] += 1
                if target_id is not None:
                    entry["target_ids"][target_id] += 1
                if len(entry["examples"]) < 8:
                    entry["examples"].append(
                        {
                            "source_id": obj.get("fleet_id") or obj.get("task_force_id"),
                            "source_ptr": hx(body),
                            "field_offset": "0x%X" % off,
                            "target_ptr": hx(ptr),
                            "target_id": target_id,
                            "adjust": "0x%X" % adjust,
                        }
                    )
    rows = []
    for off, entry in offsets.items():
        rows.append(
            {
                "offset": "0x%X" % off,
                "coverage_ratio": round(entry["count"] / object_count, 4) if object_count else 0.0,
                "count": entry["count"],
                "adjustments": [["0x%X" % k, v] for k, v in entry["adjustments"].most_common()],
                "target_ids": entry["target_ids"].most_common(8),
                "examples": entry["examples"],
            }
        )
    rows.sort(key=lambda row: (-row["count"], row["offset"]))
    return {
        "object_kind": object_kind,
        "object_count": object_count,
        "scan_span": "0x%X" % span,
        "target_vtable_rva": "0x%X" % target_vtable_rva,
        "target_type": target_type,
        "offsets": rows[:30],
    }


def scan_naval_leaders(proc: wm.Process, roots: Dict[str, Any]) -> Dict[str, Any]:
    module_base = roots["module_base"]

    def scan(objects: Iterable[Dict[str, Any]], kind: str, span: int) -> Dict[str, Any]:
        offsets: Dict[int, Dict[str, Any]] = {}
        object_count = 0
        for obj in objects:
            object_count += 1
            body = obj["body_ptr"]
            blob = safe_read(proc, body, span)
            if blob is None:
                continue
            for off in range(0, max(0, len(blob) - 8), 8):
                qword = u64_from(blob, off) or 0
                for adjust in (0, 0x10):
                    candidate = validate_navy_leader(proc, module_base, qword + adjust)
                    if candidate is None:
                        continue
                    entry = offsets.setdefault(
                        off,
                        {
                            "count": 0,
                            "adjustments": Counter(),
                            "leader_ids": Counter(),
                            "skills": Counter(),
                            "examples": [],
                        },
                    )
                    entry["count"] += 1
                    entry["adjustments"][adjust] += 1
                    entry["leader_ids"][candidate["leader_id"]] += 1
                    if candidate["skill"] is not None:
                        entry["skills"][candidate["skill"]] += 1
                    if len(entry["examples"]) < 8:
                        entry["examples"].append(
                            {
                                "source_id": obj.get("fleet_id") or obj.get("task_force_id"),
                                "source_ptr": hx(body),
                                "field_offset": "0x%X" % off,
                                "leader_ptr": hx(candidate["leader_ptr"]),
                                "leader_id": candidate["leader_id"],
                                "skill": candidate["skill"],
                                "trait_size": candidate["trait_size"],
                                "adjust": "0x%X" % adjust,
                            }
                        )
        rows = []
        for off, entry in offsets.items():
            rows.append(
                {
                    "offset": "0x%X" % off,
                    "coverage_ratio": round(entry["count"] / object_count, 4) if object_count else 0.0,
                    "count": entry["count"],
                    "adjustments": [["0x%X" % k, v] for k, v in entry["adjustments"].most_common()],
                    "leader_ids": entry["leader_ids"].most_common(12),
                    "skills": entry["skills"].most_common(12),
                    "examples": entry["examples"],
                }
            )
        rows.sort(key=lambda row: (-row["count"], row["offset"]))
        return {"object_kind": kind, "object_count": object_count, "scan_span": "0x%X" % span, "offsets": rows}

    fleet_scan = scan(roots["fleets"], "fleet", COMPACT_FLEET_SCAN_SPAN)
    task_force_scan = scan(roots["task_forces"], "task_force", TASK_FORCE_SCAN_SPAN)
    found = bool(fleet_scan["offsets"] or task_force_scan["offsets"])
    if found:
        status = "blocked_fixture_required"
        verdict = (
            "CNavyLeader direct pointer candidates validate by vtable, type, leader id, "
            "skill chain, and trait vector bounds, but assigned-admiral semantics require "
            "an admiral assignment/removal fixture before promoting a production offset"
        )
    else:
        status = "rejected_direct_pointer_scan"
        verdict = "no CNavyLeader direct pointer found in CFleet[0x0..0x1800] or CTaskForce[0x0..0x800]"
    return {
        "status": status,
        "verdict": verdict,
        "navy_leader_vtable_rva": "0x%X" % NAVY_LEADER_VTABLE_RVA,
        "leader_validation": {
            "vtable": "module+0x%X" % NAVY_LEADER_VTABLE_RVA,
            "type_off_0x08": LEADER_TYPE,
            "id_off": "0x%X" % LEADER_OFF_ID,
            "skill_chain": "leader+0x%X -> +0x%X" % (UNIT_LEADER_SKILL_SUBOBJ_PTR, UNIT_LEADER_SKILL_IN_SUBOBJ),
            "trait_vector_size_cap": ["0x%X" % UNIT_LEADER_TRAIT_SIZE, "0x%X" % UNIT_LEADER_TRAIT_CAP],
        },
        "fleet_scan": fleet_scan,
        "task_force_scan": task_force_scan,
    }


def scan_mission_candidates(proc: wm.Process, roots: Dict[str, Any]) -> Dict[str, Any]:
    offsets: Dict[int, Counter] = defaultdict(Counter)
    object_count = 0
    for task_force in roots["task_forces"]:
        object_count += 1
        blob = safe_read(proc, task_force["body_ptr"], TASK_FORCE_SCAN_SPAN)
        if blob is None:
            continue
        for off in range(0, max(0, len(blob) - 4), 4):
            if off in (0x08, 0x0C, TASK_FORCE_OFF_LOCATION_PROVINCE, TASK_FORCE_OFF_SHIP_COUNT):
                continue
            value = u32_from(blob, off)
            if value is not None and value <= 64:
                offsets[off][value] += 1
    rows = []
    for off, values in offsets.items():
        total = sum(values.values())
        if total < max(2, object_count // 4):
            continue
        rows.append(
            {
                "offset": "0x%X" % off,
                "coverage_ratio": round(total / object_count, 4) if object_count else 0.0,
                "unique_value_count": len(values),
                "top_values": values.most_common(12),
            }
        )
    rows.sort(key=lambda row: (-row["coverage_ratio"], row["unique_value_count"], row["offset"]))
    return {
        "status": "blocked_fixture_required",
        "verdict": "single live snapshot cannot assign mission enum semantics; bounded CTaskForce small-u32 candidates are emitted for a before/after mission diff",
        "task_force_count": object_count,
        "scan_range": "CTaskForce[0x0..0x%X] u32 step 4" % TASK_FORCE_SCAN_SPAN,
        "candidate_offsets": rows[:40],
    }


def load_prov2region() -> Tuple[Dict[int, int], Optional[str]]:
    try:
        with open(PROV2REGION_PATH, "r", encoding="utf-8") as f:
            raw = json.load(f)
        return {int(k): int(v) for k, v in raw.items()}, None
    except Exception as exc:  # pragma: no cover - proof environment issue
        return {}, str(exc)


def derive_region_rollups(
    proc: wm.Process, roots: Dict[str, Any], ships: List[Dict[str, Any]]
) -> Dict[str, Any]:
    prov2region, error = load_prov2region()
    ship_counts_by_tf = Counter(ship["task_force_id"] for ship in ships)
    buckets: Dict[Tuple[int, int, int], Dict[str, int]] = {}
    missing = []
    tf_rows = []
    for task_force in roots["task_forces"]:
        location = safe_u32(proc, task_force["body_ptr"] + TASK_FORCE_OFF_LOCATION_PROVINCE)
        region_id = prov2region.get(location or -1)
        if region_id is None:
            missing.append(
                {
                    "task_force_id": task_force["task_force_id"],
                    "fleet_id": task_force["fleet_id"],
                    "location_province_id": location,
                }
            )
            continue
        owner = task_force.get("owner_tag_db_id") or 0
        key = (region_id, owner, task_force["fleet_id"])
        bucket = buckets.setdefault(
            key,
            {
                "strategic_region_id": region_id,
                "owner_tag_db_id": owner,
                "fleet_id": task_force["fleet_id"],
                "task_force_count": 0,
                "ship_count": 0,
            },
        )
        bucket["task_force_count"] += 1
        bucket["ship_count"] += ship_counts_by_tf[task_force["task_force_id"]]
        tf_rows.append(
            {
                "task_force_id": task_force["task_force_id"],
                "fleet_id": task_force["fleet_id"],
                "owner_tag_db_id": owner,
                "location_province_id": location,
                "strategic_region_id": region_id,
                "ship_count": ship_counts_by_tf[task_force["task_force_id"]],
            }
        )
    status = "validated_derived" if prov2region and not error else "blocked_mapping_unavailable"
    if missing and status == "validated_derived":
        status = "partial_mapping_gap"
    return {
        "status": status,
        "verdict": "task-force strategic-region rollups derive from validated CTaskForce+0x18 current province and dev province-to-region map",
        "prov2region_path": PROV2REGION_PATH,
        "prov2region_error": error,
        "prov2region_entries": len(prov2region),
        "task_force_count": len(roots["task_forces"]),
        "task_forces_with_region": len(tf_rows),
        "missing_region_task_forces": missing[:30],
        "task_force_region_rows": tf_rows[:80],
        "region_buckets": sorted(buckets.values(), key=lambda row: (row["strategic_region_id"], row["owner_tag_db_id"], row["fleet_id"]))[:120],
    }


def make_fixture_contracts(output_path: str) -> List[Dict[str, Any]]:
    probe = "python extractor/native/lanes/naval_oob_fleets_ships_live/naval_oob_gap_probe.py"
    return [
        {
            "id": "naval_oob_mission_home_base_diff_v1",
            "purpose": "Resolve fleet/task-force mission and home-base/port offsets from the already validated CFleet/CTaskForce roots.",
            "commands": [
                "%s --out extractor/native/runs/naval_oob_fleets_ships_live_870ebf_20260701_gap_probe_fixture_before/naval_oob_gap_probe_summary.json --snapshot-label before_mission_homebase_change" % probe,
                "%s --out extractor/native/runs/naval_oob_fleets_ships_live_870ebf_20260701_gap_probe_fixture_after/naval_oob_gap_probe_summary.json --snapshot-label after_mission_homebase_change" % probe,
            ],
            "operator_steps_between_commands": [
                "Pick one live task force that remains in the same fleet and record its task_force_id from the before JSON.",
                "Change only that task force mission in the UI to a different mission such as patrol or strike force.",
                "Change only that same task force or fleet home base to a different naval base/port if the UI allows it.",
                "Do not split/merge the task force between the before and after commands.",
            ],
            "acceptance": [
                "The same task_force_id and fleet_id appear in both JSON files.",
                "A mission candidate offset changes for the edited task force and remains stable for unedited task forces.",
                "A home-base candidate offset changes to the expected port/naval-base province or CProvince pointer while CTaskForce+0x18 remains the current location field.",
                "No shared collector/recorder entrypoint is edited until these offsets are validated.",
            ],
        },
        {
            "id": "naval_oob_ship_name_fixture_v1",
            "purpose": "Resolve persistent ship instance names if they are stored outside the direct CShip body scan.",
            "commands": [
                "%s --out extractor/native/runs/naval_oob_fleets_ships_live_870ebf_20260701_gap_probe_ship_name_fixture/naval_oob_gap_probe_summary.json --snapshot-label ship_name_fixture" % probe
            ],
            "operator_steps_before_command": [
                "Provide one visible ship instance id from the compact ships domain and its UI-displayed persistent ship name, or a time-aligned save oracle mapping instance id to name.",
                "If the game supports rename, rename one known ship to a unique ASCII marker and rerun the command.",
            ],
            "acceptance": [
                "The probe candidate strings contain the exact expected name for the same ship instance id.",
                "The candidate is not equal to CShip+0x98 definition/class and is stable across a second read.",
                "If no candidate contains the expected name, persistent names are rejected for current CShip-root extraction and must be sourced from a separate validated identity registry.",
            ],
        },
        {
            "id": "naval_oob_leader_assignment_diff_v1",
            "purpose": "Select the production fleet-admiral offset from validated CNavyLeader pointer candidates.",
            "commands": [
                "%s --out extractor/native/runs/naval_oob_fleets_ships_live_870ebf_20260701_gap_probe_leader_fixture_before/naval_oob_gap_probe_summary.json --snapshot-label before_admiral_change" % probe,
                "%s --out extractor/native/runs/naval_oob_fleets_ships_live_870ebf_20260701_gap_probe_leader_fixture_after/naval_oob_gap_probe_summary.json --snapshot-label after_admiral_change" % probe,
            ],
            "operator_steps_between_commands": [
                "Pick one live fleet and record its fleet_id from the before JSON.",
                "Assign, remove, or swap that fleet's admiral in the UI without deleting or merging the fleet.",
                "Do not change task-force membership for that fleet between the before and after commands.",
            ],
            "acceptance": [
                "The same fleet_id appears in both JSON files.",
                "Exactly one CFleet CNavyLeader candidate offset changes to the expected leader_id or to zero/null after removal.",
                "The candidate continues to validate with CNavyLeader vtable module+0x%X, type 4713, id +0x0C, skill leader+0xE60 -> +0x1B8, and sane trait size/cap." % NAVY_LEADER_VTABLE_RVA,
            ],
        },
        {
            "id": "naval_oob_direct_theatre_pointer_recheck_v1",
            "purpose": "Recheck direct CTheatre linkage if a future build/game state assigns naval theatre pointers.",
            "commands": [
                "%s --out %s --snapshot-label theatre_recheck" % (probe, output_path.replace("\\", "/"))
            ],
            "acceptance": [
                "The theatre pointer scan reports a CFleet or CTaskForce offset with target vtable module+0x%X and type %d." % (THEATRE_VTABLE_RVA, OOB_TYPE_THEATRE),
                "The same offset validates on at least two roots or against a UI/save theatre assignment fixture.",
            ],
        },
    ]


def build_summary(proc: wm.Process, args: argparse.Namespace) -> Dict[str, Any]:
    started = now_ms()
    roots = discover_compact_naval_roots(proc)
    ships = sample_ship_rows(proc, roots)
    module_base = roots["module_base"]

    mission = scan_mission_candidates(proc, roots)
    ship_names = scan_ship_name_candidates(proc, roots, ships, args.max_ship_name_scan)
    home_base_tf = scan_province_candidates(
        proc,
        module_base,
        roots["task_forces"],
        "task_force",
        TASK_FORCE_SCAN_SPAN,
    )
    home_base_fleet = scan_province_candidates(
        proc,
        module_base,
        roots["fleets"],
        "fleet",
        COMPACT_FLEET_SCAN_SPAN,
    )
    leader = scan_naval_leaders(proc, roots)
    theatre_fleet = scan_vtable_pointer_candidates(
        proc,
        module_base,
        roots["fleets"],
        "fleet",
        COMPACT_FLEET_SCAN_SPAN,
        THEATRE_VTABLE_RVA,
        OOB_TYPE_THEATRE,
    )
    theatre_task_force = scan_vtable_pointer_candidates(
        proc,
        module_base,
        roots["task_forces"],
        "task_force",
        TASK_FORCE_SCAN_SPAN,
        THEATRE_VTABLE_RVA,
        OOB_TYPE_THEATRE,
    )
    region_rollups = derive_region_rollups(proc, roots, ships)

    direct_home_candidates = [
        row
        for row in home_base_tf["province_pointer_offsets"] + home_base_fleet["province_pointer_offsets"]
        if row["coverage_ratio"] > 0
    ]
    home_base_status = "blocked_fixture_required"
    home_base_verdict = (
        "CTaskForce+0x18 current location is validated; home base/port semantics require "
        "a controlled port change fixture before any province candidate can be promoted"
    )
    if not direct_home_candidates:
        home_base_status = "rejected_direct_province_pointer_scan"
        home_base_verdict = (
            "no direct CProvince* home-base pointer found in CFleet[0x0..0x1800] or "
            "CTaskForce[0x0..0x800]; raw province-id candidates still need a port-change fixture"
        )

    theatre_found = bool(theatre_fleet["offsets"] or theatre_task_force["offsets"])
    theatre_status = "blocked_fixture_required" if theatre_found else "rejected_direct_pointer_scan"
    theatre_verdict = (
        "direct CTheatre pointer candidate exists but needs UI/save theatre assignment semantics"
        if theatre_found
        else "no direct CTheatre pointer found in CFleet[0x0..0x1800] or CTaskForce[0x0..0x800]"
    )

    compact_extension = {
        "lane_local_schema_file": os.path.join(LANE_DIR, "gap_extension_compact.rs"),
        "provisional_domain_id": 29001,
        "records": [
            "FleetExtensionRow: fleet id, optional leader id/skill/traits, optional home base, optional mission/theatre, region bucket span",
            "TaskForceExtensionRow: task-force id, fleet id, location province, derived strategic region, optional mission/home base/leader",
            "ShipNameRow: ship instance id plus optional persistent name or explicit rejected/badged state",
            "RegionBucketRow: strategic-region/owner/fleet bucket with task-force and ship counts",
        ],
        "status_bits": {
            "mission_fixture_required": True,
            "home_base_fixture_required": True,
            "ship_name_rejected_or_fixture_required": True,
            "region_rollup_derived_from_location": region_rollups["status"] in ("validated_derived", "partial_mapping_gap"),
            "leader_direct_candidate": bool(
                leader.get("fleet_scan", {}).get("offsets")
                or leader.get("task_force_scan", {}).get("offsets")
            ),
        },
    }

    output_path = args.out or ""
    return {
        "schema_version": 1,
        "snapshot_label": args.snapshot_label,
        "generated_unix_ms": started,
        "duration_ms": now_ms() - started,
        "process": {
            "pid": proc.pid,
            "module": proc.module_name,
            "module_base": hx(proc.module_base),
            "module_size": proc.module_size,
            "module_path": proc.module_path,
            "sha256": proc.fingerprint().get("sha256"),
        },
        "validated_roots": {
            "CGameState": "module+0x%X -> %s" % (CG_STATE_DATE_OFFSET, hx(roots["game_state"])),
            "country_table": "CGameState+0x%X -> %s count=%s" % (CGAMESTATE_COUNTRY_TABLE, hx(roots["country_table"]), roots["country_count"]),
            "CCountry_vtable": "module+0x%X" % COUNTRY_VTABLE_RVA,
            "CFleet_vtable": "module+0x%X" % FLEET_VTABLE_RVA,
            "CTaskForce_vtable": "module+0x%X" % TASK_FORCE_VTABLE_RVA,
            "CShip_vtable": "module+0x%X" % CSHIP_VTABLE_RVA,
            "CTaskForce_location": "CTaskForce+0x%X u32 current province" % TASK_FORCE_OFF_LOCATION_PROVINCE,
            "CTaskForce_ship_vector": "CTaskForce+0x%X/+0x%X" % (TASK_FORCE_OFF_SHIP_BEGIN, TASK_FORCE_OFF_SHIP_COUNT),
        },
        "counts": {
            "countries": len(roots["countries"]),
            "fleets": len(roots["fleets"]),
            "task_forces": len(roots["task_forces"]),
            "ships": len(ships),
        },
        "root_samples": {
            "fleets": [
                {
                    "fleet_id": row["fleet_id"],
                    "owner_tag_db_id": row.get("owner_tag_db_id"),
                    "body_ptr": hx(row["body_ptr"]),
                }
                for row in roots["fleets"][:20]
            ],
            "task_forces": [
                {
                    "task_force_id": row["task_force_id"],
                    "fleet_id": row["fleet_id"],
                    "owner_tag_db_id": row.get("owner_tag_db_id"),
                    "body_ptr": hx(row["body_ptr"]),
                    "location_province_id": safe_u32(proc, row["body_ptr"] + TASK_FORCE_OFF_LOCATION_PROVINCE),
                }
                for row in roots["task_forces"][:40]
            ],
            "ships": [
                {
                    "instance_id": row["instance_id"],
                    "task_force_id": row["task_force_id"],
                    "fleet_id": row["fleet_id"],
                    "owner_tag_db_id": row.get("owner_tag_db_id"),
                    "body_ptr": hx(row["body_ptr"]),
                    "definition": row.get("definition"),
                }
                for row in ships[:40]
            ],
        },
        "remaining_gap_results": {
            "persistent_ship_names": ship_names,
            "fleet_task_force_missions": mission,
            "home_base_port_linkage": {
                "status": home_base_status,
                "verdict": home_base_verdict,
                "task_force_scan": home_base_tf,
                "fleet_scan": home_base_fleet,
            },
            "naval_leaders": leader,
            "theatre_linkage": {
                "status": theatre_status,
                "verdict": theatre_verdict,
                "fleet_scan": theatre_fleet,
                "task_force_scan": theatre_task_force,
            },
            "strategic_region_rollups": region_rollups,
        },
        "compact_extension_plan": compact_extension,
        "fixture_contracts": make_fixture_contracts(output_path),
        "integration_patch_plan": [
            "Promote validated naval leader pointer offsets, if accepted, into sample_compact_naval_oob_command_rows by filling existing CompactOobCommandRow leader_id/leader_skill/leader_trait_count for fleet rows only.",
            "Add a new compact naval OOB extension domain from gap_extension_compact.rs for fields that do not fit domains 28/29: task-force mission, home base/port, persistent ship name state, theatre/region buckets.",
            "Keep CTaskForce+0x18 as current location; derive strategic_region_id from the province-to-region table during collection or recorder ingest and fail closed if a province has no map entry.",
            "Do not source persistent names from CFEXMember+0x48; that offset remains combat-member-only and belongs to naval_combat_live.",
            "Run the fixture contracts before promoting mission/home-base/name candidates from blocked to production fields.",
        ],
    }


def write_json(path: str, value: Dict[str, Any]) -> None:
    os.makedirs(os.path.dirname(os.path.abspath(path)), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        json.dump(value, f, indent=2, sort_keys=True)
        f.write("\n")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", default=None, help="proof JSON output path")
    parser.add_argument("--snapshot-label", default="single_live_snapshot")
    parser.add_argument("--max-ship-name-scan", type=int, default=128)
    args = parser.parse_args()

    proc = wm.open_process("hoi4.exe")
    if proc is None:
        raise SystemExit("hoi4.exe is not running")
    try:
        summary = build_summary(proc, args)
    finally:
        proc.close()

    if args.out:
        write_json(args.out, summary)
        print("wrote %s" % args.out)
    else:
        print(json.dumps(summary, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
