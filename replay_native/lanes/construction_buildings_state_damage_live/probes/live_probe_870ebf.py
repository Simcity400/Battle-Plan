"""Lane-local read-only live probe for construction/building damage candidates.

This is development evidence, not a hot-loop capture path. It attaches to
hoi4.exe through memory_reader/winmem.py, uses only ReadProcessMemory, and
emits JSON under replay_native/runs.
"""

from __future__ import annotations

import argparse
import json
import os
import struct
import sys
import time
from typing import Any

LANE_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
REPO_ROOT = os.path.abspath(os.path.join(LANE_DIR, "..", "..", ".."))
MEMORY_READER = os.path.join(REPO_ROOT, "memory_reader")
sys.path.insert(0, MEMORY_READER)
sys.path.insert(0, os.path.join(MEMORY_READER, "live_layers"))

import winmem  # type: ignore  # noqa: E402

BUILD_PROFILE = "870ebf"
HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000

COUNTRY_VTABLE_RVA = 0x27D0E90
OBJ_TAG_DB_ID = 0x08
OBJ_NAME = 0x10
OBJ_PROD_MANAGER = 0x0F68
CONSTRUCTION_BEGIN = 0x70
CONSTRUCTION_CAP = 0x78
CONSTRUCTION_SIZE = 0x7C
CONSTRUCTION_STRIDE = 0x08
LINE_TYPE = 0x08
LINE_ID = 0x0C
LINE_ACTIVE_FACTORIES = 0x18
LINE_COST = 0x28
LINE_PRODUCED = 0x38
LINE_CONSTRUCTION_SUB = 0x70
LINE_TYPE_CONSTRUCTION = 59
CONSTRUCTION_TEMPLATE_ID = 0x08
CONSTRUCTION_TEMPLATE_DEF_PTR = 0x10
MAX_CONSTRUCTION_TEMPLATE_ID = 10_000_000
MAX_ACTIVE_FACTORIES = 100_000
MAX_PRODUCTION_VECTOR_ENTRIES = 4096

STATE_VTABLE_RVA = 0x29206C0
STATE_OFF_SID = 0x58
STATE_OFF_DEF = 0x30
STATE_OFF_BUILDINGS_VEC = 0x158
STATE_DEF_OFF_RESOURCES_PTR = 0xF8
STATE_DEF_OFF_RESOURCES_SIZE = 0x100
STATE_DEF_OFF_RESOURCES_CAP = 0x104
STATE_RESOURCE_STRIDE = 0x10
STATE_RESOURCE_NO_ID = 0xFFFFFFFF
STATE_RESOURCE_MAX_ROWS = 16
STATE_RESOURCE_MAX_ABS_RAW = 100_000_000

CPROVINCE_VTABLE_RVA = 0x295AF68
CPROVINCE_OFF_ID = 0xA4
CPROVINCE_OFF_STATE = 0xC0
CPROVINCE_OFF_BUILDINGS_VEC = 0x1C8

CBUILDING_VTABLE_RVA = 0x2973B88
CBUILDING_OFF_TYPE = 0x08
CBUILDING_OFF_LEVEL = 0x40
CBUILDING_OFF_HEALTHY_LEVEL = 0x42
BUILDING_VECTOR_MAX = 64
BUILDING_LEVEL_SANE_MAX = 1000

KNOWN_LINE_U32_OFFSETS = {
    LINE_TYPE,
    LINE_ID,
    LINE_ACTIVE_FACTORIES,
    LINE_COST,
    LINE_PRODUCED,
    LINE_CONSTRUCTION_SUB,
    LINE_CONSTRUCTION_SUB + 4,
}
KNOWN_LINE_U64_OFFSETS = {LINE_TYPE, LINE_CONSTRUCTION_SUB}
KNOWN_SUB_U32_OFFSETS = {
    CONSTRUCTION_TEMPLATE_ID,
    CONSTRUCTION_TEMPLATE_DEF_PTR,
    CONSTRUCTION_TEMPLATE_DEF_PTR + 4,
}
KNOWN_SUB_U64_OFFSETS = {CONSTRUCTION_TEMPLATE_DEF_PTR}


def is_heap(value: int | None) -> bool:
    return value is not None and HEAP_LO <= value < HEAP_HI and value % 8 == 0


def try_u16(p: Any, addr: int) -> int | None:
    try:
        return p.u16(addr)
    except OSError:
        return None


def try_u32(p: Any, addr: int) -> int | None:
    try:
        return p.u32(addr)
    except OSError:
        return None


def try_i32(p: Any, addr: int) -> int | None:
    try:
        return p.i32(addr)
    except OSError:
        return None


def try_u64(p: Any, addr: int) -> int | None:
    try:
        return p.u64(addr)
    except OSError:
        return None


def read_msvc_string(p: Any, addr: int, max_len: int = 96) -> str | None:
    size = try_u64(p, addr + 0x10)
    cap = try_u64(p, addr + 0x18)
    if size is None or cap is None:
        return None
    if size == 0 or size > max_len or cap < size or cap > (1 << 20):
        return None
    chars_addr = addr if cap < 16 else try_u64(p, addr)
    if chars_addr is None:
        return None
    raw = p.try_read(chars_addr, size + 1)
    if len(raw) != size + 1 or raw[-1] != 0:
        return None
    text = raw[:-1]
    if any(byte < 32 or byte >= 127 for byte in text):
        return None
    try:
        return text.decode("utf-8")
    except UnicodeDecodeError:
        return None


def read_country_roots(p: Any, max_regions: int | None = None) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    started = time.time()
    country_vtable = p.module_base + COUNTRY_VTABLE_RVA
    needle = struct.pack("<Q", country_vtable)
    roots_by_tag: dict[int, dict[str, Any]] = {}
    duplicate_tag_db_ids: list[int] = []
    region_count = 0
    candidate_hits = 0
    for base, size in p.iter_regions(HEAP_LO, HEAP_HI):
        region_count += 1
        if max_regions is not None and region_count > max_regions:
            break
        pos = 0
        while pos < size:
            want = min(64 << 20, size - pos)
            blob = p.try_read(base + pos, want)
            if not blob:
                pos += max(want, 0x1000)
                continue
            search = 0
            while True:
                hit = blob.find(needle, search)
                if hit < 0:
                    break
                search = hit + 1
                addr = base + pos + hit
                if addr % 8 != 0:
                    continue
                candidate_hits += 1
                if try_u64(p, addr) != country_vtable:
                    continue
                tag_db_id = try_u32(p, addr + OBJ_TAG_DB_ID)
                if tag_db_id is None or tag_db_id == 0 or tag_db_id > 2048:
                    continue
                name = read_msvc_string(p, addr + OBJ_NAME)
                if not name or read_msvc_string(p, addr + OBJ_NAME) != name:
                    continue
                manager = try_u64(p, addr + OBJ_PROD_MANAGER)
                if tag_db_id in roots_by_tag:
                    duplicate_tag_db_ids.append(tag_db_id)
                    continue
                roots_by_tag[tag_db_id] = {
                    "tag_db_id": tag_db_id,
                    "tag": f"tag_db_{tag_db_id}",
                    "name": name,
                    "country_ptr": hex(addr),
                    "production_manager_ptr": hex(manager) if is_heap(manager) else None,
                }
            if pos + len(blob) >= size or len(blob) < want:
                pos += max(len(blob), 0x1000)
            else:
                pos += len(blob) - 7
    roots = [roots_by_tag[tag_db_id] for tag_db_id in sorted(roots_by_tag)]
    meta: dict[str, Any] = {
        "resolver": "lane_current_country_vtable_scan",
        "duration_ms": round((time.time() - started) * 1000),
        "ok": len(roots) >= 40,
        "country_vtable": hex(country_vtable),
        "country_vtable_rva": hex(COUNTRY_VTABLE_RVA),
        "tag_db_id_offset": hex(OBJ_TAG_DB_ID),
        "name_offset": hex(OBJ_NAME),
        "region_count": region_count,
        "candidate_vtable_hits": candidate_hits,
        "root_count": len(roots),
        "duplicate_tag_db_ids": duplicate_tag_db_ids[:32],
    }
    return roots, meta


def read_construction_lines(p: Any, country_roots: list[dict[str, Any]]) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for root in country_roots:
        country_ptr = int(root["country_ptr"], 16)
        manager = try_u64(p, country_ptr + OBJ_PROD_MANAGER)
        if not is_heap(manager):
            continue
        begin = try_u64(p, manager + CONSTRUCTION_BEGIN)
        cap = try_u32(p, manager + CONSTRUCTION_CAP)
        size = try_u32(p, manager + CONSTRUCTION_SIZE)
        if not is_heap(begin) or cap is None or size is None:
            continue
        if size > cap or cap > MAX_PRODUCTION_VECTOR_ENTRIES:
            continue
        raw = p.try_read(begin, size * CONSTRUCTION_STRIDE)
        if len(raw) != size * CONSTRUCTION_STRIDE:
            continue
        priority = 0
        for index in range(size):
            line_ptr = struct.unpack_from("<Q", raw, index * CONSTRUCTION_STRIDE)[0]
            if not is_heap(line_ptr):
                continue
            line_type = try_u32(p, line_ptr + LINE_TYPE)
            if line_type != LINE_TYPE_CONSTRUCTION:
                continue
            active = try_i32(p, line_ptr + LINE_ACTIVE_FACTORIES)
            cost_raw = try_i32(p, line_ptr + LINE_COST)
            produced_raw = try_i32(p, line_ptr + LINE_PRODUCED)
            if active is None or cost_raw is None or produced_raw is None:
                continue
            if not (0 <= active <= MAX_ACTIVE_FACTORIES):
                continue
            if cost_raw < 0 or produced_raw < 0:
                continue
            line_id = try_u32(p, line_ptr + LINE_ID)
            sub_ptr = try_u64(p, line_ptr + LINE_CONSTRUCTION_SUB)
            template_id = None
            template_def_ptr = None
            if is_heap(sub_ptr):
                template_raw = try_u32(p, sub_ptr + CONSTRUCTION_TEMPLATE_ID)
                if template_raw and 0 < template_raw <= MAX_CONSTRUCTION_TEMPLATE_ID:
                    template_id = template_raw
                template_def_raw = try_u64(p, sub_ptr + CONSTRUCTION_TEMPLATE_DEF_PTR)
                if is_heap(template_def_raw):
                    template_def_ptr = template_def_raw
            rows.append(
                {
                    "tag_db_id": root["tag_db_id"],
                    "tag": root["tag"],
                    "line_id": line_id,
                    "priority": priority,
                    "template_id": template_id,
                    "active_factories": active,
                    "cost_raw": cost_raw,
                    "produced_raw": produced_raw,
                    "country_ptr": root["country_ptr"],
                    "manager_ptr": hex(manager),
                    "line_ptr": hex(line_ptr),
                    "sub_ptr": hex(sub_ptr) if is_heap(sub_ptr) else None,
                    "template_def_ptr": hex(template_def_ptr) if template_def_ptr else None,
                }
            )
            priority += 1
    return rows


def scan_cprovince_roots(p: Any, max_regions: int | None = None) -> dict[str, Any]:
    started = time.time()
    cprovince_vtable = p.module_base + CPROVINCE_VTABLE_RVA
    state_vtable = p.module_base + STATE_VTABLE_RVA
    needle = struct.pack("<Q", cprovince_vtable)
    province_by_id: dict[int, dict[str, Any]] = {}
    state_by_id: dict[int, str] = {}
    province_ptr_to_id: dict[int, int] = {}
    state_ptr_to_id: dict[int, int] = {}
    region_count = 0
    candidate_hits = 0
    for base, size in p.iter_regions(HEAP_LO, HEAP_HI):
        region_count += 1
        if max_regions is not None and region_count > max_regions:
            break
        pos = 0
        while pos < size:
            want = min(64 << 20, size - pos)
            blob = p.try_read(base + pos, want)
            if not blob:
                pos += max(want, 0x1000)
                continue
            search = 0
            while True:
                hit = blob.find(needle, search)
                if hit < 0:
                    break
                search = hit + 1
                addr = base + pos + hit
                if addr % 8 != 0:
                    continue
                candidate_hits += 1
                province_id = try_u32(p, addr + CPROVINCE_OFF_ID)
                if province_id is None or province_id == 0 or province_id > 65535:
                    continue
                if try_u64(p, addr) != cprovince_vtable:
                    continue
                state_ptr = try_u64(p, addr + CPROVINCE_OFF_STATE)
                if not is_heap(state_ptr):
                    continue
                if try_u64(p, state_ptr) != state_vtable:
                    continue
                state_id = try_u32(p, state_ptr + STATE_OFF_SID)
                if state_id is None or state_id == 0 or state_id > 65535:
                    continue
                province_by_id.setdefault(
                    province_id,
                    {
                        "province_ptr": hex(addr),
                        "state_id": state_id,
                        "state_ptr": hex(state_ptr),
                    },
                )
                state_by_id.setdefault(state_id, hex(state_ptr))
                province_ptr_to_id.setdefault(addr, province_id)
                state_ptr_to_id.setdefault(state_ptr, state_id)
            if pos + len(blob) >= size or len(blob) < want:
                pos += max(len(blob), 0x1000)
            else:
                pos += len(blob) - 7
    return {
        "duration_ms": round((time.time() - started) * 1000),
        "region_count": region_count,
        "cprovince_vtable": hex(cprovince_vtable),
        "state_vtable": hex(state_vtable),
        "candidate_vtable_hits": candidate_hits,
        "province_count": len(province_by_id),
        "state_count": len(state_by_id),
        "province_by_id": province_by_id,
        "state_by_id": state_by_id,
        "province_ptr_to_id": {hex(k): v for k, v in province_ptr_to_id.items()},
        "state_ptr_to_id": {hex(k): v for k, v in state_ptr_to_id.items()},
    }


def scan_target_candidates(lines: list[dict[str, Any]], topology: dict[str, Any], p: Any) -> dict[str, Any]:
    if not lines:
        return {
            "line_count": 0,
            "aggregate_candidate_count": 0,
            "strict_all_line_candidate_count": 0,
            "strict_all_line_candidates": [],
            "per_line_samples": [],
            "verdict": "blocked_no_live_construction_lines_in_current_frame",
        }
    province_by_id = {int(k): v for k, v in topology.get("province_by_id", {}).items()}
    state_by_id = {int(k): v for k, v in topology.get("state_by_id", {}).items()}
    province_ptr_to_id = {
        int(k, 16): v for k, v in topology.get("province_ptr_to_id", {}).items()
    }
    state_ptr_to_id = {int(k, 16): v for k, v in topology.get("state_ptr_to_id", {}).items()}
    aggregates: dict[str, dict[str, Any]] = {}
    samples: list[dict[str, Any]] = []
    for line in lines:
        bases: list[tuple[str, int, int, set[int], set[int]]] = []
        bases.append(
            (
                "line",
                int(line["line_ptr"], 16),
                0x120,
                KNOWN_LINE_U32_OFFSETS,
                KNOWN_LINE_U64_OFFSETS,
            )
        )
        if line.get("sub_ptr"):
            bases.append(
                (
                    "sub",
                    int(line["sub_ptr"], 16),
                    0x100,
                    KNOWN_SUB_U32_OFFSETS,
                    KNOWN_SUB_U64_OFFSETS,
                )
            )
        if line.get("template_def_ptr"):
            bases.append(("template_def", int(line["template_def_ptr"], 16), 0x160, set(), set()))
        line_candidates: list[dict[str, Any]] = []
        for base_name, addr, length, known_u32, known_u64 in bases:
            raw = p.try_read(addr, length)
            if len(raw) != length:
                continue
            for offset in range(0, length - 3, 4):
                if offset in known_u32:
                    continue
                value = struct.unpack_from("<I", raw, offset)[0]
                if value in state_by_id:
                    line_candidates.append(
                        candidate(line, base_name, offset, "raw_state_id", value, value)
                    )
                if value in province_by_id:
                    mapped = int(province_by_id[value]["state_id"])
                    line_candidates.append(
                        candidate(line, base_name, offset, "raw_province_id", value, mapped)
                    )
            for offset in range(0, length - 7, 8):
                if offset in known_u64:
                    continue
                ptr_value = struct.unpack_from("<Q", raw, offset)[0]
                if ptr_value in state_ptr_to_id:
                    state_id = state_ptr_to_id[ptr_value]
                    line_candidates.append(
                        candidate(line, base_name, offset, "state_ptr", ptr_value, state_id)
                    )
                if ptr_value in province_ptr_to_id:
                    province_id = province_ptr_to_id[ptr_value]
                    mapped = int(province_by_id[province_id]["state_id"])
                    line_candidates.append(
                        candidate(line, base_name, offset, "province_ptr", province_id, mapped)
                    )
                low = ptr_value & 0xFFFFFFFF
                high = (ptr_value >> 32) & 0xFFFFFFFF
                if high in state_by_id:
                    line_candidates.append(
                        candidate(line, base_name, offset, "u64_high_state_id", high, high)
                    )
                if high in province_by_id:
                    mapped = int(province_by_id[high]["state_id"])
                    line_candidates.append(
                        candidate(line, base_name, offset, "u64_high_province_id", high, mapped)
                    )
                if low in state_by_id:
                    line_candidates.append(
                        candidate(line, base_name, offset, "u64_low_state_id", low, low)
                    )
                if low in province_by_id:
                    mapped = int(province_by_id[low]["state_id"])
                    line_candidates.append(
                        candidate(line, base_name, offset, "u64_low_province_id", low, mapped)
                    )
        for found in line_candidates:
            key = f"{found['base']}+{found['offset']}:{found['kind']}"
            agg = aggregates.setdefault(
                key,
                {
                    "base": found["base"],
                    "offset": found["offset"],
                    "kind": found["kind"],
                    "line_count": 0,
                    "tag_db_ids": [],
                    "template_ids": [],
                    "mapped_state_ids": [],
                    "raw_values": [],
                    "sample_lines": [],
                },
            )
            agg["line_count"] += 1
            if found["tag_db_id"] not in agg["tag_db_ids"]:
                agg["tag_db_ids"].append(found["tag_db_id"])
            if found["template_id"] is not None and found["template_id"] not in agg["template_ids"]:
                agg["template_ids"].append(found["template_id"])
            if found["mapped_state_id"] not in agg["mapped_state_ids"]:
                agg["mapped_state_ids"].append(found["mapped_state_id"])
            if found["raw_value"] not in agg["raw_values"]:
                agg["raw_values"].append(found["raw_value"])
            if len(agg["sample_lines"]) < 5:
                agg["sample_lines"].append(found)
        samples.append(
            {
                "tag_db_id": line["tag_db_id"],
                "tag": line["tag"],
                "line_id": line["line_id"],
                "priority": line["priority"],
                "template_id": line["template_id"],
                "candidate_count": len(line_candidates),
                "candidates": line_candidates[:24],
            }
        )
    strict = [
        value
        for value in aggregates.values()
        if value["line_count"] == len(lines) and len(lines) > 0
    ]
    strict.sort(key=lambda row: (row["base"], row["offset"], row["kind"]))
    return {
        "line_count": len(lines),
        "aggregate_candidate_count": len(aggregates),
        "strict_all_line_candidate_count": len(strict),
        "strict_all_line_candidates": strict[:50],
        "per_line_samples": samples[:50],
        "verdict": (
            "candidate_inventory_only_not_semantically_validated"
            if aggregates
            else "rejected_no_state_or_province_candidate_in_scanned_line_sub_template_ranges"
        ),
    }


def candidate(
    line: dict[str, Any],
    base_name: str,
    offset: int,
    kind: str,
    raw_value: int,
    mapped_state_id: int,
) -> dict[str, Any]:
    return {
        "tag_db_id": line["tag_db_id"],
        "tag": line["tag"],
        "line_id": line["line_id"],
        "priority": line["priority"],
        "template_id": line["template_id"],
        "base": base_name,
        "offset": hex(offset),
        "kind": kind,
        "raw_value": raw_value if raw_value < (1 << 32) else hex(raw_value),
        "mapped_state_id": mapped_state_id,
    }


def scan_building_health(p: Any, topology: dict[str, Any], max_states: int, max_provinces: int) -> dict[str, Any]:
    state_items = sorted((int(k), int(v, 16)) for k, v in topology.get("state_by_id", {}).items())
    province_items = sorted(
        (
            int(k),
            int(v["province_ptr"], 16),
            int(v["state_id"]),
        )
        for k, v in topology.get("province_by_id", {}).items()
    )
    cbuilding_vtable = p.module_base + CBUILDING_VTABLE_RVA
    stats: dict[str, Any] = {
        "cbuilding_vtable": hex(cbuilding_vtable),
        "state_vectors_scanned": 0,
        "province_vectors_scanned": 0,
        "building_entries": 0,
        "healthy_readable": 0,
        "healthy_equals_level": 0,
        "healthy_less_than_level": 0,
        "healthy_greater_than_level": 0,
        "healthy_missing": 0,
        "damaged_examples": [],
        "undamaged_examples": [],
        "invalid_examples": [],
    }
    for state_id, state_ptr in state_items[:max_states]:
        stats["state_vectors_scanned"] += 1
        consume_building_vector(
            p,
            cbuilding_vtable,
            "state",
            state_id,
            state_id,
            state_ptr,
            STATE_OFF_BUILDINGS_VEC,
            stats,
        )
    for province_id, province_ptr, state_id in province_items[:max_provinces]:
        stats["province_vectors_scanned"] += 1
        consume_building_vector(
            p,
            cbuilding_vtable,
            "province",
            province_id,
            state_id,
            province_ptr,
            CPROVINCE_OFF_BUILDINGS_VEC,
            stats,
        )
    if stats["healthy_less_than_level"]:
        stats["verdict"] = "current_live_frame_contains_healthy_level_below_level"
    elif stats["healthy_readable"] and stats["healthy_greater_than_level"] == 0:
        stats["verdict"] = "coherent_undamaged_frame_only_no_damaged_semantics_proof"
    else:
        stats["verdict"] = "healthy_level_candidate_not_validated"
    return stats


def consume_building_vector(
    p: Any,
    cbuilding_vtable: int,
    scope: str,
    location_id: int,
    state_id: int,
    obj_ptr: int,
    vec_off: int,
    stats: dict[str, Any],
) -> None:
    begin = try_u64(p, obj_ptr + vec_off)
    packed = try_u64(p, obj_ptr + vec_off + 8)
    if not is_heap(begin) or packed is None:
        return
    logical = (packed >> 32) & 0xFFFFFFFF
    if logical > BUILDING_VECTOR_MAX:
        return
    raw = p.try_read(begin, logical * 8)
    if len(raw) != logical * 8:
        return
    for index in range(logical):
        building_ptr = struct.unpack_from("<Q", raw, index * 8)[0]
        if not is_heap(building_ptr):
            continue
        if try_u64(p, building_ptr) != cbuilding_vtable:
            continue
        type_id = try_u32(p, building_ptr + CBUILDING_OFF_TYPE)
        level = try_u16(p, building_ptr + CBUILDING_OFF_LEVEL)
        healthy = try_u16(p, building_ptr + CBUILDING_OFF_HEALTHY_LEVEL)
        if type_id is None or level is None or level > BUILDING_LEVEL_SANE_MAX:
            continue
        stats["building_entries"] += 1
        if healthy is None:
            stats["healthy_missing"] += 1
            continue
        stats["healthy_readable"] += 1
        if healthy == level:
            stats["healthy_equals_level"] += 1
            if len(stats["undamaged_examples"]) < 16:
                stats["undamaged_examples"].append(
                    {
                        "scope": scope,
                        "location_id": location_id,
                        "state_id": state_id,
                        "building_ptr": hex(building_ptr),
                        "type_id": type_id,
                        "level_at_0x40": level,
                        "healthy_level_at_0x42": healthy,
                    }
                )
        elif healthy < level:
            stats["healthy_less_than_level"] += 1
            if len(stats["damaged_examples"]) < 16:
                stats["damaged_examples"].append(
                    {
                        "scope": scope,
                        "location_id": location_id,
                        "state_id": state_id,
                        "building_ptr": hex(building_ptr),
                        "type_id": type_id,
                        "level_at_0x40": level,
                        "healthy_level_at_0x42": healthy,
                    }
                )
        else:
            stats["healthy_greater_than_level"] += 1
            if len(stats["invalid_examples"]) < 16:
                stats["invalid_examples"].append(
                    {
                        "scope": scope,
                        "location_id": location_id,
                        "state_id": state_id,
                        "building_ptr": hex(building_ptr),
                        "type_id": type_id,
                        "level_at_0x40": level,
                        "healthy_level_at_0x42": healthy,
                    }
                )


def scan_state_resources(
    p: Any,
    topology: dict[str, Any],
    max_states: int,
) -> dict[str, Any]:
    state_items = sorted((int(k), int(v, 16)) for k, v in topology.get("state_by_id", {}).items())
    stats: dict[str, Any] = {
        "state_vectors_scanned": 0,
        "states_with_resources": 0,
        "resource_rows": 0,
        "invalid_resource_vectors": 0,
        "invalid_resource_rows": 0,
        "samples": [],
        "invalid_examples": [],
    }
    for state_id, state_ptr in state_items[:max_states]:
        stats["state_vectors_scanned"] += 1
        state_def = try_u64(p, state_ptr + STATE_OFF_DEF)
        if not is_heap(state_def):
            continue
        data = try_u64(p, state_def + STATE_DEF_OFF_RESOURCES_PTR)
        size = try_u32(p, state_def + STATE_DEF_OFF_RESOURCES_SIZE)
        cap = try_u32(p, state_def + STATE_DEF_OFF_RESOURCES_CAP)
        if size is None or cap is None:
            stats["invalid_resource_vectors"] += 1
            continue
        logical = state_resource_logical_rows(size, cap)
        if logical is None:
            stats["invalid_resource_vectors"] += 1
            continue
        if logical == 0:
            continue
        if not is_heap(data):
            stats["invalid_resource_vectors"] += 1
            continue
        raw = p.try_read(data, logical * STATE_RESOURCE_STRIDE)
        if len(raw) != logical * STATE_RESOURCE_STRIDE:
            stats["invalid_resource_vectors"] += 1
            continue
        rows_for_state = 0
        for index in range(logical):
            offset = index * STATE_RESOURCE_STRIDE
            amount_raw = struct.unpack_from("<i", raw, offset)[0]
            resource_id = struct.unpack_from("<I", raw, offset + 8)[0]
            if resource_id == STATE_RESOURCE_NO_ID:
                continue
            if not (0 <= amount_raw <= STATE_RESOURCE_MAX_ABS_RAW):
                stats["invalid_resource_rows"] += 1
                if len(stats["invalid_examples"]) < 16:
                    stats["invalid_examples"].append(
                        {
                            "state_id": state_id,
                            "state_ptr": hex(state_ptr),
                            "resource_id": resource_id,
                            "amount_raw": amount_raw,
                        }
                    )
                continue
            rows_for_state += 1
            stats["resource_rows"] += 1
            if len(stats["samples"]) < 32:
                stats["samples"].append(
                    {
                        "state_id": state_id,
                        "state_ptr": hex(state_ptr),
                        "resource_id": resource_id,
                        "amount_raw": amount_raw,
                    }
                )
        if rows_for_state:
            stats["states_with_resources"] += 1
    if stats["resource_rows"] and not stats["invalid_resource_rows"]:
        stats["verdict"] = "state_resource_rows_live_readable"
    elif stats["resource_rows"]:
        stats["verdict"] = "state_resource_rows_partially_readable"
    else:
        stats["verdict"] = "state_resource_rows_not_observed"
    return stats


def state_resource_logical_rows(size: int, cap: int) -> int | None:
    if size == 0:
        return 0
    if size > STATE_RESOURCE_MAX_ROWS:
        return None
    logical = cap if cap != 0 and cap < size else size
    if logical > STATE_RESOURCE_MAX_ROWS:
        return None
    return logical


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", required=True, help="path to write live_probe_870ebf.json")
    parser.add_argument("--skip-topology", action="store_true")
    parser.add_argument("--max-regions", type=int, default=None)
    parser.add_argument("--max-health-states", type=int, default=1081)
    parser.add_argument("--max-health-provinces", type=int, default=13275)
    args = parser.parse_args()

    started = time.time()
    p = winmem.open_process("hoi4.exe")
    if p is None:
        raise SystemExit("hoi4.exe not running")
    with p:
        fingerprint = p.fingerprint()
        country_roots, country_meta = read_country_roots(p, max_regions=args.max_regions)
        construction_lines = read_construction_lines(p, country_roots)
        topology: dict[str, Any] = {}
        target_candidates: dict[str, Any] = {
            "verdict": "not_attempted_topology_skipped",
            "line_count": len(construction_lines),
        }
        building_health: dict[str, Any] = {"verdict": "not_attempted_topology_skipped"}
        state_resources: dict[str, Any] = {"verdict": "not_attempted_topology_skipped"}
        if not args.skip_topology:
            topology = scan_cprovince_roots(p, max_regions=args.max_regions)
            target_candidates = scan_target_candidates(construction_lines, topology, p)
            if not country_meta.get("ok") and not construction_lines:
                target_candidates["verdict"] = "blocked_country_roots_unresolved_current_build"
            building_health = scan_building_health(
                p,
                topology,
                max_states=args.max_health_states,
                max_provinces=args.max_health_provinces,
            )
            state_resources = scan_state_resources(
                p,
                topology,
                max_states=args.max_health_states,
            )
        result = {
            "schema": "construction_buildings_state_damage_live_probe_870ebf_v1",
            "build_profile": BUILD_PROFILE,
            "probe_kind": "read_only_live_memory",
            "duration_ms": round((time.time() - started) * 1000),
            "fingerprint": fingerprint,
            "validated_offsets_attempted": {
                "construction_manager": {
                    "country_to_manager": hex(OBJ_PROD_MANAGER),
                    "vector_begin": hex(CONSTRUCTION_BEGIN),
                    "vector_cap": hex(CONSTRUCTION_CAP),
                    "vector_size": hex(CONSTRUCTION_SIZE),
                    "line_type": hex(LINE_TYPE),
                    "line_type_value": LINE_TYPE_CONSTRUCTION,
                    "line_id": hex(LINE_ID),
                    "active_factories": hex(LINE_ACTIVE_FACTORIES),
                    "cost_raw": hex(LINE_COST),
                    "produced_raw": hex(LINE_PRODUCED),
                    "subobject_ptr": hex(LINE_CONSTRUCTION_SUB),
                    "template_id": hex(CONSTRUCTION_TEMPLATE_ID),
                    "template_def_ptr_candidate": hex(CONSTRUCTION_TEMPLATE_DEF_PTR),
                },
                "building_health": {
                    "cbuilding_vtable_rva": hex(CBUILDING_VTABLE_RVA),
                    "type_id": hex(CBUILDING_OFF_TYPE),
                    "level": hex(CBUILDING_OFF_LEVEL),
                    "healthy_level_candidate": hex(CBUILDING_OFF_HEALTHY_LEVEL),
                },
                "topology": {
                    "cprovince_vtable_rva": hex(CPROVINCE_VTABLE_RVA),
                    "cprovince_id": hex(CPROVINCE_OFF_ID),
                    "cprovince_state_ptr": hex(CPROVINCE_OFF_STATE),
                    "state_vtable_rva": hex(STATE_VTABLE_RVA),
                    "state_id": hex(STATE_OFF_SID),
                },
                "state_resources": {
                    "state_def_offset": hex(STATE_OFF_DEF),
                    "resources_ptr_offset": hex(STATE_DEF_OFF_RESOURCES_PTR),
                    "resources_size_offset": hex(STATE_DEF_OFF_RESOURCES_SIZE),
                    "resources_cap_offset": hex(STATE_DEF_OFF_RESOURCES_CAP),
                    "resource_stride": STATE_RESOURCE_STRIDE,
                },
            },
            "country_roots": country_meta,
            "construction": {
                "line_count": len(construction_lines),
                "tag_db_id_count": len(
                    {line["tag_db_id"] for line in construction_lines}
                ),
                "template_id_count": len(
                    {
                        line["template_id"]
                        for line in construction_lines
                        if line["template_id"] is not None
                    }
                ),
                "tag_db_ids": sorted(
                    {line["tag_db_id"] for line in construction_lines}
                )[:128],
                "template_ids": sorted(
                    {
                        line["template_id"]
                        for line in construction_lines
                        if line["template_id"] is not None
                    }
                )[:128],
                "sample_lines": construction_lines[:100],
            },
            "topology": {
                key: value
                for key, value in topology.items()
                if key not in ("province_by_id", "state_by_id", "province_ptr_to_id", "state_ptr_to_id")
            },
            "target_state_candidates": target_candidates,
            "building_health": building_health,
            "state_resources": state_resources,
            "verdict": {
                "construction_target_state": target_candidates.get("verdict"),
                "healthy_level_0x42": building_health.get("verdict"),
                "state_resources": state_resources.get("verdict"),
            },
        }
    os.makedirs(os.path.dirname(os.path.abspath(args.out)), exist_ok=True)
    with open(args.out, "w", encoding="utf-8") as handle:
        json.dump(result, handle, indent=2, sort_keys=True)
        handle.write("\n")
    print(json.dumps({
        "out": args.out,
        "construction_lines": len(construction_lines),
        "target_state_verdict": result["verdict"]["construction_target_state"],
        "healthy_level_verdict": result["verdict"]["healthy_level_0x42"],
        "duration_ms": result["duration_ms"],
    }, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
