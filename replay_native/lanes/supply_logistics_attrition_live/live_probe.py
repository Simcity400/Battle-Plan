"""Live proof runner for the supply/logistics/attrition lane.

This is lane-local and read-only. It uses the current 870ebf native collector
roots without touching shared collector/recorder entrypoints:

* CGameState at module+0x3316E90.
* CGameState+0x2B0 CProvince pointer array for province/state/building roots.
* CGameState+0x310 country table -> country+0x290 division wrapper vector for
  division supply proxy rows.

It writes a compact domain-44 payload fixture plus a JSON/Markdown proof report.
"""

from __future__ import annotations

import argparse
import datetime as _dt
import json
import os
import struct
import sys
import time
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional, Tuple

REPO_ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPO_ROOT / "memory_reader"))

import winmem  # noqa: E402

EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
OLD_SUPPLY_READER_SHA256 = "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120"

DOMAIN_ID = 44
DOMAIN_NAME = "supply_logistics_attrition_live"
FIXED_SCALE_RAW = 100_000

STATUS_COMPACT_HEALTH_OK = 0

ROW_KIND_PROVINCE_LOGISTICS = 1
ROW_KIND_DIVISION_SUPPLY = 2
ROW_KIND_COMPUTED_BADGE = 3
ROW_KIND_CSUPPLY_PROVINCE_INPUT = 4
ROW_KIND_FOREIGN_HOMEBASE_PRESSURE_CANDIDATE = 5

ROW_FLAG_RAIL = 1
ROW_FLAG_SUPPLY_NODE = 1 << 1
ROW_FLAG_NAVAL_SUPPLY_HUB = 1 << 2
ROW_FLAG_INFRASTRUCTURE = 1 << 3
ROW_FLAG_LIVE_BUILDING_VECTOR = 1 << 4
ROW_FLAG_DIVISION_CURRENT_SUPPLY = 1 << 5
ROW_FLAG_DIVISION_MAX_SUPPLY = 1 << 6
ROW_FLAG_DIVISION_SUPPLY_GAIN = 1 << 7
ROW_FLAG_DIVISION_OWNER = 1 << 8
ROW_FLAG_EXACT_SUPPLY_WALL = 1 << 9
ROW_FLAG_ATTRITION_WALL = 1 << 10
ROW_FLAG_PROXY_VALUE = 1 << 11
ROW_FLAG_DIVISION_SUPPLY_DEFICIT = 1 << 12
ROW_FLAG_DIVISION_FUEL = 1 << 13
ROW_FLAG_MOTORIZATION_WALL = 1 << 14
ROW_FLAG_CONVOY_PRESSURE_WALL = 1 << 15
ROW_FLAG_CSUPPLY_CANDIDATE_REJECTED = 1 << 16
ROW_FLAG_CSUPPLY_EFFICIENCY = 1 << 17
ROW_FLAG_CSUPPLY_PROVINCE_ARRAY = 1 << 18
ROW_FLAG_FOREIGN_HOMEBASE_PRESSURE = 1 << 19
ROW_FLAG_PRESSURE_CANDIDATE_LAYOUT = 1 << 20

BADGE_EXACT_PROVINCE_SUPPLY_TRANSIENT = 1
BADGE_ATTRITION_RUNTIME_COMPUTED = 2
BADGE_MOTORIZATION_SETTINGS_UNPINNED = 3
BADGE_CONVOY_LOGISTICS_PRESSURE_UNPINNED = 4
BADGE_CSUPPLY_INPUT_ARRAYS_UNSAFE = 5

HEADER_STRUCT = struct.Struct("<Q11I")
ROW_STRUCT = struct.Struct("<5I5i")

HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000

CGAMESTATE_RVA = 0x3316E90
CGAMESTATE_CPROVINCE_ARRAY = 0x2B0
CGAMESTATE_CPROVINCE_COUNT = 0x2BC
CGAMESTATE_COUNTRY_TABLE = 0x310
CGAMESTATE_COUNTRY_COUNT = 0x31C

COUNTRY_VTABLE_RVA = 0x27D0E90
COUNTRY_DIVISION_WRAPPER_DATA = 0x290
COUNTRY_DIVISION_WRAPPER_CAPACITY = 0x298
COUNTRY_DIVISION_WRAPPER_COUNT = 0x29C

STATE_VTABLE_RVA = 0x29206C0
STATE_OFF_SID = 0x58
STATE_OFF_BUILDINGS_VEC = 0x158

CPROVINCE_VTABLE_RVA = 0x295AF68
CPROVINCE_OFF_ID = 0xA4
CPROVINCE_OFF_STATE = 0xC0
CPROVINCE_OFF_BUILDINGS_VEC = 0x1C8

CBUILDING_VTABLE_RVA = 0x2973B88
CBUILDING_OFF_TYPE = 0x08
CBUILDING_OFF_LEVEL = 0x40
BUILDING_VECTOR_MAX = 64
BUILDING_LEVEL_SANE_MAX = 1000

DIVISION_WRAPPER_VTABLE_RVA = 0x2943AD0
DIVISION_WRAPPER_TO_BODY = 0x10
DIVISION_VTABLE_RVA = 0x2943CB0
DIVISION_TYPE = 4713
DIVISION_ALT_UNIT_TYPE = 51
DIVISION_OFF_ID_TYPE = 0x08
DIVISION_OFF_ID_ID = 0x0C
DIVISION_OFF_LOCATION = 0x18
DIVISION_OFF_OWNER_TAG_DB_ID = 0x1C8
DIVISION_OFF_TEMPLATE = 0x3A8
DIVISION_OFF_MAX_SUPPLY = 0x5F0
DIVISION_OFF_SUPPLY_GAIN = 0x5F8
DIVISION_OFF_CURRENT_SUPPLY = 0x600
DIVISION_OFF_FUEL = 0x610

CSUPPLYSYSTEM_VTABLE0_RVA = 0x295D0A8
CSUPPLYSYSTEM_VTABLE1_RVA = 0x295D0D0
CSUPPLYSYSTEM_PROVINCE_INPUT_A = 0x10
CSUPPLYSYSTEM_PROVINCE_INPUT_B = 0x138
CSUPPLYSYSTEM_PROVINCE_INPUT_C = 0x150

MAX_CGAMESTATE_COUNTRIES = 2048
MAX_DIVISION_ROWS = 20_000
MAX_PROVINCE_ID = 13_413
MAX_STATE_ID = 1_081
MAX_SUPPLY_RAW = 100_000_000
MAX_SUPPLY_GAIN_RAW = 500_000
MAX_DIVISION_FUEL_RAW = 2_147_483_647

TYPE_ID_NAME = {
    0x27D2: "naval_supply_hub",
    0x2FAB: "infrastructure",
    0x2FB6: "air_base",
    0x31F6: "naval_base",
    0x32B4: "nuclear_reactor",
    0x3C8F: "fuel_silo",
    0x4BF6: "dockyard",
    0x4CBE: "supply_node",
    0x4CC1: "rail_way",
    0x4E22: "arms_factory",
    0x4E2D: "industrial_complex",
    0x4E44: "synthetic_refinery",
    0x5781: "bunker",
    0x5782: "coastal_bunker",
    0x6C2F: "naval_facility",
    0x6D5B: "anti_air_building",
    0x6DEF: "radar_station",
    0x6EB6: "naval_headquarters",
    0x6F7F: "nuclear_facility",
    0x6FB2: "air_facility",
    0x6FE4: "land_facility",
    0x7016: "dam",
    0x7079: "cataract_dam_mountain",
    0x70AB: "canal_kiel",
    0x70DF: "canal_panama",
    0x7144: "industrial_infrastructure",
    0x7176: "landmark_big_ben",
    0x71A9: "landmark_colosseum",
    0x720D: "landmark_eiffel_tower",
    0x723F: "landmark_statue_of_liberty",
    0x7271: "landmark_kremlin",
    0x72A3: "landmark_hofburg_palace",
    0x72D5: "landmark_berlin_reichstag",
    0x7339: "landmark_taj_mahal",
    0x736B: "landmark_sadabad_complex",
    0x739D: "landmark_hagia_sophia",
    0x73CF: "landmark_forbidden_city",
    0x7401: "landmark_nanjing_presidential_palace",
    0x7465: "landmark_nanjing_presidential_palace_gen",
    0x7497: "landmark_great_wall_section",
    0x74C9: "landmark_tokyo_imperial_palace",
    0x755F: "landmark_prague_castle",
    0x7591: "landmark_bojnice_castle",
}


def _is_heap(value: Optional[int]) -> bool:
    return value is not None and HEAP_LO <= value < HEAP_HI and value % 8 == 0


def _clamp_i32(value: Optional[int], max_value: int) -> int:
    if value is None or value < 0 or value > max_value:
        return 0
    return int(value)


def _read_buildings(p: Any, module_base: int, obj_base: int, vec_off: int) -> Optional[Dict[str, int]]:
    if not _is_heap(obj_base):
        return None
    begin = p.u64(obj_base + vec_off)
    packed = p.u64(obj_base + vec_off + 8)
    logical = (packed >> 32) & 0xFFFFFFFF
    if logical > BUILDING_VECTOR_MAX:
        return None
    if logical == 0:
        return {}
    if not _is_heap(begin):
        return None
    blob = p.try_read(begin, logical * 8)
    if not blob or len(blob) < logical * 8:
        return None
    out: Dict[str, int] = {}
    cbuilding_vtable = module_base + CBUILDING_VTABLE_RVA
    for idx in range(logical):
        building_ptr = struct.unpack_from("<Q", blob, idx * 8)[0]
        if not _is_heap(building_ptr):
            continue
        try:
            if p.u64(building_ptr) != cbuilding_vtable:
                continue
            type_id = p.u32(building_ptr + CBUILDING_OFF_TYPE)
            level = p.u16(building_ptr + CBUILDING_OFF_LEVEL)
        except OSError:
            continue
        if level > BUILDING_LEVEL_SANE_MAX:
            continue
        name = TYPE_ID_NAME.get(type_id, f"type_0x{type_id:04X}")
        out[name] = out.get(name, 0) + int(level)
    return out


def _province_rows(p: Any, module_base: int, game_state: int) -> Tuple[List[Tuple[int, ...]], Dict[str, Any]]:
    province_array = p.u64(game_state + CGAMESTATE_CPROVINCE_ARRAY)
    province_count = p.u32(game_state + CGAMESTATE_CPROVINCE_COUNT)
    if province_count == 0 or province_count > MAX_PROVINCE_ID + 1:
        raise RuntimeError(f"invalid province_count {province_count}")
    if not _is_heap(province_array):
        raise RuntimeError(f"invalid province array 0x{province_array:X}")
    blob = p.try_read(province_array, province_count * 8)
    if not blob or len(blob) < province_count * 8:
        raise RuntimeError("could not read CProvince pointer array")

    cprovince_vtable = module_base + CPROVINCE_VTABLE_RVA
    state_vtable = module_base + STATE_VTABLE_RVA
    valid_provinces = 0
    state_ptrs: Dict[int, int] = {}
    state_building_cache: Dict[int, Dict[str, int]] = {}
    rows: List[Tuple[int, ...]] = []
    totals = {
        "province_count": province_count,
        "valid_province_roots": 0,
        "province_rows": 0,
        "states_seen": 0,
        "rail_provinces": 0,
        "supply_node_provinces": 0,
        "naval_supply_hub_provinces": 0,
        "state_infrastructure_province_rows": 0,
        "province_building_rows_with_any_building": 0,
    }
    samples: List[Dict[str, Any]] = []

    for province_id in range(1, province_count):
        province_ptr = struct.unpack_from("<Q", blob, province_id * 8)[0]
        if province_ptr == 0:
            continue
        if not _is_heap(province_ptr):
            continue
        try:
            if p.u64(province_ptr) != cprovince_vtable:
                continue
            if p.u32(province_ptr + CPROVINCE_OFF_ID) != province_id:
                continue
            state_ptr = p.u64(province_ptr + CPROVINCE_OFF_STATE)
        except OSError:
            continue
        valid_provinces += 1
        state_id = 0
        infrastructure = 0
        if _is_heap(state_ptr):
            try:
                if p.u64(state_ptr) == state_vtable:
                    state_id = p.u32(state_ptr + STATE_OFF_SID)
            except OSError:
                state_id = 0
            if 1 <= state_id <= MAX_STATE_ID:
                previous = state_ptrs.get(state_id)
                if previous is None:
                    state_ptrs[state_id] = state_ptr
                elif previous != state_ptr:
                    raise RuntimeError(f"state {state_id} resolved to multiple pointers")
                if state_id not in state_building_cache:
                    state_building_cache[state_id] = (
                        _read_buildings(p, module_base, state_ptr, STATE_OFF_BUILDINGS_VEC) or {}
                    )
                infrastructure = int(state_building_cache[state_id].get("infrastructure", 0))
        province_buildings = _read_buildings(p, module_base, province_ptr, CPROVINCE_OFF_BUILDINGS_VEC)
        if province_buildings is None:
            province_buildings = {}
        rail = int(province_buildings.get("rail_way", 0))
        supply_node = int(province_buildings.get("supply_node", 0))
        naval_supply_hub = int(province_buildings.get("naval_supply_hub", 0))
        flags = ROW_FLAG_LIVE_BUILDING_VECTOR
        if rail:
            flags |= ROW_FLAG_RAIL
            totals["rail_provinces"] += 1
        if supply_node:
            flags |= ROW_FLAG_SUPPLY_NODE
            totals["supply_node_provinces"] += 1
        if naval_supply_hub:
            flags |= ROW_FLAG_NAVAL_SUPPLY_HUB
            totals["naval_supply_hub_provinces"] += 1
        if infrastructure:
            flags |= ROW_FLAG_INFRASTRUCTURE
            totals["state_infrastructure_province_rows"] += 1
        if province_buildings:
            totals["province_building_rows_with_any_building"] += 1
        if state_id:
            rows.append((
                ROW_KIND_PROVINCE_LOGISTICS,
                province_id,
                state_id,
                0,
                flags,
                rail,
                supply_node,
                naval_supply_hub,
                infrastructure,
                len(province_buildings),
            ))
            if len(samples) < 12 and (rail or supply_node or naval_supply_hub):
                samples.append({
                    "province_id": province_id,
                    "state_id": state_id,
                    "rail_way": rail,
                    "supply_node": supply_node,
                    "naval_supply_hub": naval_supply_hub,
                    "infrastructure": infrastructure,
                    "building_keys": sorted(province_buildings.keys()),
                })

    totals["valid_province_roots"] = valid_provinces
    totals["province_rows"] = len(rows)
    totals["states_seen"] = len(state_ptrs)
    return rows, {"summary": totals, "samples": samples}


def _division_rows(p: Any, module_base: int, game_state: int) -> Tuple[List[Tuple[int, ...]], Dict[str, Any]]:
    country_table = p.u64(game_state + CGAMESTATE_COUNTRY_TABLE)
    country_count = p.u32(game_state + CGAMESTATE_COUNTRY_COUNT)
    if country_count == 0 or country_count > MAX_CGAMESTATE_COUNTRIES:
        raise RuntimeError(f"invalid country_count {country_count}")
    if not _is_heap(country_table):
        raise RuntimeError(f"invalid country table 0x{country_table:X}")
    country_blob = p.try_read(country_table, country_count * 8)
    if not country_blob or len(country_blob) < country_count * 8:
        raise RuntimeError("could not read country table")

    country_vtable = module_base + COUNTRY_VTABLE_RVA
    wrapper_vtable = module_base + DIVISION_WRAPPER_VTABLE_RVA
    division_vtable = module_base + DIVISION_VTABLE_RVA
    rows_by_id: Dict[int, Tuple[int, ...]] = {}
    proxy_by_province: Dict[int, List[int]] = {}
    country_rows_seen = 0
    wrapper_rows_seen = 0
    rejected_wrappers = 0
    samples: List[Dict[str, Any]] = []

    for index in range(country_count):
        country_ptr = struct.unpack_from("<Q", country_blob, index * 8)[0]
        if not _is_heap(country_ptr):
            continue
        try:
            if p.u64(country_ptr) != country_vtable:
                continue
            vector_data = p.u64(country_ptr + COUNTRY_DIVISION_WRAPPER_DATA)
            capacity = p.u32(country_ptr + COUNTRY_DIVISION_WRAPPER_CAPACITY)
            count = p.u32(country_ptr + COUNTRY_DIVISION_WRAPPER_COUNT)
        except OSError:
            continue
        country_rows_seen += 1
        if count == 0:
            continue
        if count > capacity or count > MAX_DIVISION_ROWS or not _is_heap(vector_data):
            rejected_wrappers += int(count)
            continue
        wrapper_blob = p.try_read(vector_data, count * 8)
        if not wrapper_blob or len(wrapper_blob) < count * 8:
            rejected_wrappers += int(count)
            continue
        for item in range(count):
            wrapper_ptr = struct.unpack_from("<Q", wrapper_blob, item * 8)[0]
            wrapper_rows_seen += 1
            if not _is_heap(wrapper_ptr):
                rejected_wrappers += 1
                continue
            try:
                if p.u64(wrapper_ptr) != wrapper_vtable:
                    rejected_wrappers += 1
                    continue
                body = wrapper_ptr + DIVISION_WRAPPER_TO_BODY
                if p.u64(body) != division_vtable:
                    rejected_wrappers += 1
                    continue
                unit_type = p.u32(body + DIVISION_OFF_ID_TYPE)
                if unit_type not in (DIVISION_TYPE, DIVISION_ALT_UNIT_TYPE):
                    rejected_wrappers += 1
                    continue
                div_id = p.u32(body + DIVISION_OFF_ID_ID)
                province_id = p.u32(body + DIVISION_OFF_LOCATION)
                owner = p.u32(body + DIVISION_OFF_OWNER_TAG_DB_ID)
                current = p.u32(body + DIVISION_OFF_CURRENT_SUPPLY)
                max_supply = p.u32(body + DIVISION_OFF_MAX_SUPPLY)
                gain = p.u32(body + DIVISION_OFF_SUPPLY_GAIN)
                fuel = p.u32(body + DIVISION_OFF_FUEL)
                template_ptr = p.u64(body + DIVISION_OFF_TEMPLATE)
            except OSError:
                rejected_wrappers += 1
                continue
            if not div_id or div_id > MAX_DIVISION_ROWS * 1000:
                rejected_wrappers += 1
                continue
            if not province_id or province_id > MAX_PROVINCE_ID:
                rejected_wrappers += 1
                continue
            current_i = _clamp_i32(current, MAX_SUPPLY_RAW)
            max_i = _clamp_i32(max_supply, MAX_SUPPLY_RAW)
            gain_i = _clamp_i32(gain, MAX_SUPPLY_GAIN_RAW)
            fuel_i = _clamp_i32(fuel, MAX_DIVISION_FUEL_RAW)
            deficit_i = max(0, max_i - current_i)
            flags = ROW_FLAG_PROXY_VALUE
            if current_i:
                flags |= ROW_FLAG_DIVISION_CURRENT_SUPPLY
            if max_i:
                flags |= ROW_FLAG_DIVISION_MAX_SUPPLY
            if gain_i:
                flags |= ROW_FLAG_DIVISION_SUPPLY_GAIN
            if deficit_i:
                flags |= ROW_FLAG_DIVISION_SUPPLY_DEFICIT
            if fuel_i:
                flags |= ROW_FLAG_DIVISION_FUEL
            if 0 < owner <= MAX_CGAMESTATE_COUNTRIES:
                flags |= ROW_FLAG_DIVISION_OWNER
            else:
                owner = 0
            row = (
                ROW_KIND_DIVISION_SUPPLY,
                div_id,
                province_id,
                owner,
                flags,
                current_i,
                max_i,
                gain_i,
                deficit_i,
                fuel_i,
            )
            rows_by_id.setdefault(div_id, row)
            proxy_by_province.setdefault(province_id, []).append(current_i)
            if len(samples) < 12:
                samples.append({
                    "division_id": div_id,
                    "province_id": province_id,
                    "owner_tag_db_id": owner or None,
                    "current_supply": round(current_i / FIXED_SCALE_RAW, 5),
                    "max_supply": round(max_i / FIXED_SCALE_RAW, 5),
                    "supply_gain": round(gain_i / FIXED_SCALE_RAW, 5),
                    "supply_deficit": round(deficit_i / FIXED_SCALE_RAW, 5),
                    "fuel": round(fuel_i / FIXED_SCALE_RAW, 5),
                    "template_ptr_valid": _is_heap(template_ptr),
                })

    rows = [rows_by_id[key] for key in sorted(rows_by_id)]
    current_values = [row[5] for row in rows]
    zero_supply = sum(1 for value in current_values if value == 0)
    low_supply = sum(1 for value in current_values if 0 < value < FIXED_SCALE_RAW)
    summary = {
        "country_rows_seen": country_rows_seen,
        "country_count": country_count,
        "wrapper_rows_seen": wrapper_rows_seen,
        "rejected_wrappers": rejected_wrappers,
        "division_rows": len(rows),
        "division_supply_proxy_provinces": len(proxy_by_province),
        "zero_current_supply_divisions": zero_supply,
        "low_current_supply_divisions": low_supply,
        "nonzero_supply_gain_divisions": sum(1 for row in rows if row[7] != 0),
        "nonzero_supply_deficit_divisions": sum(1 for row in rows if row[8] != 0),
        "fuel_divisions": sum(1 for row in rows if row[9] != 0),
        "total_supply_deficit_raw": sum(max(0, row[8]) for row in rows),
        "fuel_proxy_provinces": len({row[2] for row in rows if row[9] != 0}),
    }
    return rows, {"summary": summary, "samples": samples}


def _badge_rows() -> List[Tuple[int, ...]]:
    return [
        (
            ROW_KIND_COMPUTED_BADGE,
            0,
            0,
            0,
            ROW_FLAG_EXACT_SUPPLY_WALL,
            BADGE_EXACT_PROVINCE_SUPPLY_TRANSIENT,
            0,
            0,
            0,
            0,
        ),
        (
            ROW_KIND_COMPUTED_BADGE,
            0,
            0,
            0,
            ROW_FLAG_ATTRITION_WALL,
            BADGE_ATTRITION_RUNTIME_COMPUTED,
            0,
            0,
            0,
            0,
        ),
        (
            ROW_KIND_COMPUTED_BADGE,
            0,
            0,
            0,
            ROW_FLAG_MOTORIZATION_WALL,
            BADGE_MOTORIZATION_SETTINGS_UNPINNED,
            0,
            0,
            0,
            0,
        ),
        (
            ROW_KIND_COMPUTED_BADGE,
            0,
            0,
            0,
            ROW_FLAG_CONVOY_PRESSURE_WALL,
            BADGE_CONVOY_LOGISTICS_PRESSURE_UNPINNED,
            0,
            0,
            0,
            0,
        ),
        (
            ROW_KIND_COMPUTED_BADGE,
            0,
            0,
            0,
            ROW_FLAG_CSUPPLY_CANDIDATE_REJECTED,
            BADGE_CSUPPLY_INPUT_ARRAYS_UNSAFE,
            0,
            0,
            0,
            0,
        ),
    ]


def _encode_payload(
    rows: List[Tuple[int, ...]],
    province_rows: int,
    division_rows: int,
    badge_rows: int,
    csupply_input_rows: int,
    pressure_candidate_rows: int = 0,
) -> bytes:
    status = STATUS_COMPACT_HEALTH_OK
    header = HEADER_STRUCT.pack(
        0,
        len(rows),
        len(rows),
        status,
        FIXED_SCALE_RAW,
        province_rows,
        division_rows,
        badge_rows,
        province_rows,
        division_rows,
        csupply_input_rows,
        pressure_candidate_rows,
    )
    body = b"".join(ROW_STRUCT.pack(*row) for row in rows)
    return header + body


def _array_i32x1e5_stats(p: Any, base: int, count: int) -> Dict[str, Any]:
    stats: Dict[str, Any] = {
        "count": count,
        "nonzero": 0,
        "zero": 0,
        "negative": 0,
        "gt_100000": 0,
        "gt_1000000": 0,
        "bad_reads": 0,
        "min_raw": None,
        "max_raw": None,
        "samples": {},
    }
    for province_id in range(1, count):
        try:
            value = p.i32(base + province_id * 8)
        except OSError:
            stats["bad_reads"] += 1
            continue
        if value == 0:
            stats["zero"] += 1
        else:
            stats["nonzero"] += 1
        if value < 0:
            stats["negative"] += 1
        if value > FIXED_SCALE_RAW:
            stats["gt_100000"] += 1
        if value > FIXED_SCALE_RAW * 10:
            stats["gt_1000000"] += 1
        stats["min_raw"] = value if stats["min_raw"] is None else min(stats["min_raw"], value)
        stats["max_raw"] = value if stats["max_raw"] is None else max(stats["max_raw"], value)
        if province_id in (1, 2, 10, 100, 500, 1000, 2000, 3324, 3544, 6421, 9244, 12000, 13413):
            stats["samples"][str(province_id)] = value
    return stats


def _find_csupplysystem_candidate(p: Any, module_base: int) -> Dict[str, Any]:
    vtable0 = module_base + CSUPPLYSYSTEM_VTABLE0_RVA
    vtable1 = module_base + CSUPPLYSYSTEM_VTABLE1_RVA
    needle = struct.pack("<QQ", vtable0, vtable1)
    for base, size in p.iter_regions(HEAP_LO, HEAP_HI, want_exec=False):
        if size > 0x10000000:
            continue
        blob = p.try_read(base, size)
        if not blob:
            continue
        search = 0
        while True:
            found = blob.find(needle, search)
            if found < 0:
                break
            search = found + 8
            obj = base + found
            try:
                fields = {}
                for label, off in (
                    ("province_input_a_plus_0x10", CSUPPLYSYSTEM_PROVINCE_INPUT_A),
                    ("province_input_b_plus_0x138", CSUPPLYSYSTEM_PROVINCE_INPUT_B),
                    ("province_input_c_plus_0x150", CSUPPLYSYSTEM_PROVINCE_INPUT_C),
                ):
                    arr = p.u64(obj + off)
                    count = p.u32(obj + off + 8)
                    if count != MAX_PROVINCE_ID + 1 or not _is_heap(arr):
                        fields[label] = {
                            "array": f"0x{arr:X}" if arr else None,
                            "count": count,
                            "verdict": "rejected_shape",
                        }
                        continue
                    stats = _array_i32x1e5_stats(p, arr, count)
                    stats["array"] = f"0x{arr:X}"
                    if stats["bad_reads"]:
                        verdict = "rejected_bad_reads"
                    elif stats["negative"] or stats["gt_100000"]:
                        verdict = "rejected_not_bounded_fixed_point"
                    else:
                        verdict = "bounded_fixed_point_candidate"
                    stats["verdict"] = verdict
                    fields[label] = stats
                a = fields.get("province_input_a_plus_0x10", {})
                b = fields.get("province_input_b_plus_0x138", {})
                c = fields.get("province_input_c_plus_0x150", {})
                return {
                    "status": "candidate_found_plus_0x10_filtered_rows_emitted",
                    "object": f"0x{obj:X}",
                    "vtable0": f"module+0x{CSUPPLYSYSTEM_VTABLE0_RVA:X}",
                    "vtable1": f"module+0x{CSUPPLYSYSTEM_VTABLE1_RVA:X}",
                    "fields": fields,
                    "verdict": (
                        "Current-build CSupplySystem-shaped object was found. +0x10 is "
                        "emitted by the lane as filtered bounded low-i32/1e5 rows, while "
                        f"{a.get('negative', 0) + a.get('gt_100000', 0)} raw +0x10 "
                        "entries remain rejected as out-of-range or packed. +0x138 has "
                        f"{b.get('negative', 0)} negative and {b.get('gt_100000', 0)} "
                        "gt_100000 entries, and +0x150 has "
                        f"{c.get('negative', 0)} negative and {c.get('gt_100000', 0)} "
                        "gt_100000 entries, so those siblings are not emitted."
                    ),
                }
            except OSError:
                continue
    return {
        "status": "candidate_not_found",
        "vtable0": f"module+0x{CSUPPLYSYSTEM_VTABLE0_RVA:X}",
        "vtable1": f"module+0x{CSUPPLYSYSTEM_VTABLE1_RVA:X}",
        "verdict": "No current-build CSupplySystem-shaped object found by the bounded vtable scan.",
    }


def _low_i32_stride8_stats(p: Any, array: int, count: int, max_raw: int) -> Dict[str, Any]:
    stats: Dict[str, Any] = {
        "count": count,
        "valid": 0,
        "nonzero": 0,
        "zero": 0,
        "negative": 0,
        "gt_max_raw": 0,
        "bad_reads": 0,
        "min_raw": None,
        "max_raw": None,
        "samples": {},
    }
    blob = p.try_read(array, count * 8)
    if not blob or len(blob) < count * 8:
        stats["bad_reads"] = count
        stats["verdict"] = "rejected_short_read"
        return stats
    for province_id in range(1, count):
        raw = struct.unpack_from("<i", blob, province_id * 8)[0]
        if raw < 0:
            stats["negative"] += 1
        elif raw > max_raw:
            stats["gt_max_raw"] += 1
        else:
            stats["valid"] += 1
            if raw:
                stats["nonzero"] += 1
            else:
                stats["zero"] += 1
            stats["min_raw"] = raw if stats["min_raw"] is None else min(stats["min_raw"], raw)
            stats["max_raw"] = raw if stats["max_raw"] is None else max(stats["max_raw"], raw)
        if province_id in (1, 2, 10, 100, 500, 1000, 2000, 3324, 3544, 6421, 9244, 12000, 13413):
            stats["samples"][str(province_id)] = raw
    if stats["bad_reads"]:
        verdict = "rejected_bad_reads"
    elif stats["negative"] or stats["gt_max_raw"]:
        verdict = "partially_valid_with_rejected_values"
    elif stats["nonzero"] == 0:
        verdict = "rejected_all_zero"
    else:
        verdict = "valid_bounded_low_i32_stride8"
    stats["verdict"] = verdict
    return stats


def _csupply_input_rows(p: Any, module_base: int) -> Tuple[List[Tuple[int, ...]], Dict[str, Any]]:
    vtable0 = module_base + CSUPPLYSYSTEM_VTABLE0_RVA
    vtable1 = module_base + CSUPPLYSYSTEM_VTABLE1_RVA
    needle = struct.pack("<QQ", vtable0, vtable1)
    for base, size in p.iter_regions(HEAP_LO, HEAP_HI, want_exec=False):
        if size > 0x10000000:
            continue
        blob = p.try_read(base, size)
        if not blob:
            continue
        search = 0
        while True:
            found = blob.find(needle, search)
            if found < 0:
                break
            search = found + 8
            obj = base + found
            try:
                eff_array = p.u64(obj + CSUPPLYSYSTEM_PROVINCE_INPUT_A)
                eff_count = p.u32(obj + CSUPPLYSYSTEM_PROVINCE_INPUT_A + 8)
                draw_array = p.u64(obj + CSUPPLYSYSTEM_PROVINCE_INPUT_B)
                draw_count = p.u32(obj + CSUPPLYSYSTEM_PROVINCE_INPUT_B + 8)
                x_array = p.u64(obj + CSUPPLYSYSTEM_PROVINCE_INPUT_C)
                x_count = p.u32(obj + CSUPPLYSYSTEM_PROVINCE_INPUT_C + 8)
            except OSError:
                continue
            if eff_count != MAX_PROVINCE_ID + 1 or not _is_heap(eff_array):
                continue
            eff_blob = p.try_read(eff_array, eff_count * 8)
            if not eff_blob or len(eff_blob) < eff_count * 8:
                return [], {
                    "status": "short_read",
                    "object": f"0x{obj:X}",
                    "efficiency": {
                        "array": f"0x{eff_array:X}",
                        "count": eff_count,
                        "verdict": "rejected_short_read",
                    },
                }
            rows: List[Tuple[int, ...]] = []
            rejected_samples: List[Dict[str, Any]] = []
            nonzero = 0
            min_raw: Optional[int] = None
            max_raw: Optional[int] = None
            negative = 0
            gt_100000 = 0
            for province_id in range(1, eff_count):
                raw = struct.unpack_from("<i", eff_blob, province_id * 8)[0]
                if raw < 0:
                    negative += 1
                    if len(rejected_samples) < 12:
                        rejected_samples.append({"province_id": province_id, "raw": raw, "reason": "negative"})
                    continue
                if raw > FIXED_SCALE_RAW:
                    gt_100000 += 1
                    if len(rejected_samples) < 12:
                        rejected_samples.append({"province_id": province_id, "raw": raw, "reason": "gt_100000"})
                    continue
                if raw:
                    nonzero += 1
                min_raw = raw if min_raw is None else min(min_raw, raw)
                max_raw = raw if max_raw is None else max(max_raw, raw)
                rows.append((
                    ROW_KIND_CSUPPLY_PROVINCE_INPUT,
                    province_id,
                    0,
                    0,
                    ROW_FLAG_CSUPPLY_EFFICIENCY | ROW_FLAG_CSUPPLY_PROVINCE_ARRAY,
                    raw,
                    0,
                    0,
                    0,
                    0,
                ))
            draw_report = {
                "array": f"0x{draw_array:X}" if draw_array else None,
                "count": draw_count,
                **(
                    _low_i32_stride8_stats(p, draw_array, draw_count, MAX_SUPPLY_RAW)
                    if draw_count == MAX_PROVINCE_ID + 1 and _is_heap(draw_array)
                    else {"verdict": "rejected_shape"}
                ),
            }
            x_report = {
                "array": f"0x{x_array:X}" if x_array else None,
                "count": x_count,
                **(
                    _low_i32_stride8_stats(p, x_array, x_count, MAX_SUPPLY_RAW)
                    if x_count == MAX_PROVINCE_ID + 1 and _is_heap(x_array)
                    else {"verdict": "rejected_shape"}
                ),
            }
            return rows, {
                "status": "efficiency_rows_emitted",
                "object": f"0x{obj:X}",
                "vtable0": f"module+0x{CSUPPLYSYSTEM_VTABLE0_RVA:X}",
                "vtable1": f"module+0x{CSUPPLYSYSTEM_VTABLE1_RVA:X}",
                "efficiency": {
                    "array": f"0x{eff_array:X}",
                    "count": eff_count,
                    "rows_emitted": len(rows),
                    "nonzero_rows": nonzero,
                    "zero_rows": len(rows) - nonzero,
                    "negative_rejected": negative,
                    "gt_100000_rejected": gt_100000,
                    "min_raw": min_raw,
                    "max_raw": max_raw,
                    "encoding": "low i32 at array + province_id*8, fixed /100000",
                    "verdict": "production_candidate_emitted_with_per_value_bounds",
                    "rejected_samples": rejected_samples,
                },
                "draw_plus_0x138": draw_report,
                "x_plus_0x150": x_report,
                "verdict": (
                    "CSupplySystem +0x10 is emitted as bounded per-province efficiency/fulfilment "
                    "input rows. +0x138 and +0x150 remain un-emitted under current-build guards."
                ),
            }
    return [], {
        "status": "csupplysystem_not_found",
        "vtable0": f"module+0x{CSUPPLYSYSTEM_VTABLE0_RVA:X}",
        "vtable1": f"module+0x{CSUPPLYSYSTEM_VTABLE1_RVA:X}",
        "verdict": "No CSupplySystem-shaped object found.",
    }


def _write_outputs(out_dir: Path, report: Dict[str, Any], payload: Optional[bytes]) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    if payload is not None:
        payload_path = out_dir / "domain44_payload.bin"
        payload_path.write_bytes(payload)
        report["compact_payload"] = {
            "path": str(payload_path),
            "bytes": len(payload),
            "domain_id": DOMAIN_ID,
            "domain_name": DOMAIN_NAME,
            "record_payload_only": True,
        }
    proof_path = out_dir / "proof.json"
    proof_path.write_text(json.dumps(report, indent=2), encoding="utf-8")
    compact = report.get("compact_payload", {})
    province_summary = report.get("province_logistics", {}).get("summary", {})
    division_summary = report.get("division_supply", {}).get("summary", {})
    csupply_inputs = report.get("csupply_inputs", {})
    csupply_efficiency = csupply_inputs.get("efficiency", {}) if isinstance(csupply_inputs, dict) else {}
    schema = report.get("compact_schema", {})
    candidate = report.get("csupplysystem_candidate", {})
    candidate_fields = candidate.get("fields", {}) if isinstance(candidate, dict) else {}

    def candidate_field_line(label: str, field: Dict[str, Any]) -> str:
        if not isinstance(field, dict) or not field:
            return f"- `{label}`: not observed."
        return (
            f"- Rejected `{label}` (`{field.get('array')}`): count `{field.get('count')}`, "
            f"negative `{field.get('negative')}`, `>100000` `{field.get('gt_100000')}`, "
            f"`>1000000` `{field.get('gt_1000000')}`, bad_reads `{field.get('bad_reads')}`, "
            f"min_raw `{field.get('min_raw')}`, max_raw `{field.get('max_raw')}`, "
            f"verdict `{field.get('verdict')}`."
        )

    md_lines = [
        "# supply_logistics_attrition_live proof",
        "",
        f"- status: `{report['status']}`",
        f"- exe_sha256: `{report.get('process', {}).get('sha256')}`",
        f"- compact_payload: `{compact.get('path', 'none')}`",
        f"- compact_payload_bytes: `{compact.get('bytes', 0)}`",
        f"- compact_rows: `{schema.get('row_count', 0)}`",
        f"- province_rows: `{province_summary.get('province_rows', 0)}`",
        f"- division_rows: `{division_summary.get('division_rows', 0)}`",
        f"- csupply_input_rows: `{schema.get('csupply_input_rows', 0)}`",
        f"- badge_rows: `{schema.get('badge_rows', 0)}`",
        f"- nonzero_supply_deficit_divisions: `{division_summary.get('nonzero_supply_deficit_divisions', 0)}`",
        f"- fuel_divisions: `{division_summary.get('fuel_divisions', 0)}`",
        "",
        "## Validated Slice",
        "",
        "- Validated slice only: province logistics/building-derived rows plus division current/max/gain supply, computed deficit, and fuel proxy.",
        "- Current-build CSupplySystem `+0x10` bounded per-province efficiency/fulfilment inputs are emitted as compact rows.",
        "- Whole supply/logistics/attrition concept is not complete; exact province supply, attrition reason, hub motorization, convoy/logistics pressure, and CSupplySystem draw/remaining arrays remain outside the emitted data.",
        "",
        "## Roots Validated",
        "",
        f"- CGameState: `{report.get('roots_validated', {}).get('cgame_state', 'not validated')}`",
        f"- Province array: `{report.get('roots_validated', {}).get('province_array', 'not validated')}`",
        f"- Province rows: `{report.get('roots_validated', {}).get('cprovince', 'not validated')}`",
        f"- State rows: `{report.get('roots_validated', {}).get('state', 'not validated')}`",
        f"- Building rows: `{report.get('roots_validated', {}).get('building', 'not validated')}`",
        f"- Division rows: `{report.get('roots_validated', {}).get('division', 'not validated')}`",
        "",
        "## CSupply Inputs",
        "",
        f"- CSupplySystem status: `{csupply_inputs.get('status', 'not observed')}`",
        f"- CSupplySystem object: `{csupply_inputs.get('object', 'none')}`",
        f"- Emitted efficiency rows: `{csupply_efficiency.get('rows_emitted', 0)}`",
        f"- Nonzero efficiency rows: `{csupply_efficiency.get('nonzero_rows', 0)}`",
        f"- Rejected efficiency negatives: `{csupply_efficiency.get('negative_rejected', 0)}`",
        f"- Rejected efficiency `>100000`: `{csupply_efficiency.get('gt_100000_rejected', 0)}`",
        f"- Draw `+0x138` verdict: `{csupply_inputs.get('draw_plus_0x138', {}).get('verdict', 'not observed')}`",
        f"- X `+0x150` verdict: `{csupply_inputs.get('x_plus_0x150', {}).get('verdict', 'not observed')}`",
        f"- Verdict: {csupply_inputs.get('verdict', 'no csupply input verdict')}",
        "",
        "## Candidate Rejections",
        "",
        f"- CSupplySystem status: `{candidate.get('status', 'not observed')}`",
        f"- CSupplySystem object: `{candidate.get('object', 'none')}`",
        f"- CSupplySystem vtables: `{candidate.get('vtable0', 'none')}`, `{candidate.get('vtable1', 'none')}`",
        candidate_field_line(
            "province_input_a_plus_0x10",
            candidate_fields.get("province_input_a_plus_0x10", {}),
        ),
        candidate_field_line(
            "province_input_b_plus_0x138",
            candidate_fields.get("province_input_b_plus_0x138", {}),
        ),
        candidate_field_line(
            "province_input_c_plus_0x150",
            candidate_fields.get("province_input_c_plus_0x150", {}),
        ),
        f"- Verdict: {candidate.get('verdict', 'no candidate verdict')}",
        "",
        "## Wall Badges",
        "",
        "- Exact per-province supply tooltip value remains badged as computed/transient.",
        "- Attrition reason/flag remains badged as runtime-computed until a field or function path is pinned.",
        "- Hub motorization settings remain badged until a current-build node-setting root is pinned.",
        "- Convoy/logistics pressure remains badged beyond existing country convoy/fuel domains.",
        "- CSupplySystem `+0x10` efficiency/fulfilment input is emitted with per-value bounds; `+0x138`, `+0x150`, and exact displayed remaining are still not emitted under current guards.",
    ]
    (out_dir / "PROOF.md").write_text("\n".join(md_lines) + "\n", encoding="utf-8")


def run(pid: Optional[int], out_dir: Path) -> Dict[str, Any]:
    started = time.time()
    p = winmem.Process(pid, "hoi4.exe") if pid is not None else winmem.open_process("hoi4.exe")
    report: Dict[str, Any] = {
        "lane": DOMAIN_NAME,
        "status": "fail_closed",
        "old_python_readers_expected_sha256": OLD_SUPPLY_READER_SHA256,
        "expected_sha256": EXPECTED_SHA256,
        "read_only": True,
        "writes_to_game_memory": False,
    }
    if p is None:
        report["reason"] = "hoi4.exe not running or cannot be opened"
        _write_outputs(out_dir, report, None)
        return report

    payload: Optional[bytes] = None
    p.__enter__()
    try:
        fp = p.fingerprint()
        got_sha = (fp.get("sha256") or "").lower()
        report["process"] = fp
        if got_sha != EXPECTED_SHA256:
            report["reason"] = "build mismatch; offsets are build-specific"
            _write_outputs(out_dir, report, None)
            return report

        module_base = p.module_base
        game_state = p.u64(module_base + CGAMESTATE_RVA)
        if not _is_heap(game_state):
            raise RuntimeError(f"invalid CGameState pointer 0x{game_state:X}")
        province_rows, province_report = _province_rows(p, module_base, game_state)
        division_rows, division_report = _division_rows(p, module_base, game_state)
        csupply_input_rows, csupply_input_report = _csupply_input_rows(p, module_base)
        csupplysystem_report = _find_csupplysystem_candidate(p, module_base)
        badge_rows = _badge_rows()
        rows = province_rows + division_rows + csupply_input_rows + badge_rows
        payload = _encode_payload(
            rows,
            len(province_rows),
            len(division_rows),
            len(badge_rows),
            len(csupply_input_rows),
            0,
        )
        report.update({
            "status": "live_roots_validated",
            "roots_validated": {
                "cgame_state": "module+0x3316E90 -> CCurrentGameState",
                "province_array": "CGameState+0x2B0 ptr, CGameState+0x2BC count",
                "cprovince": "vtable module+0x295AF68, province_id u32@+0xA4, state_ptr@+0xC0, buildings vector@+0x1C8",
                "state": "vtable module+0x29206C0, state_id u32@+0x58, buildings vector@+0x158",
                "building": "vtable module+0x2973B88, type_id u32@+0x08, level u16@+0x40",
                "division": "CGameState+0x310 country table -> country+0x290 wrapper vector -> wrapper+0x10 body; vtable module+0x2943CB0; id/type/location/supply/fuel @ +0x0C/+0x08/+0x18/+0x5F0,+0x5F8,+0x600,+0x610",
            },
            "province_logistics": province_report,
            "division_supply": division_report,
            "csupply_inputs": csupply_input_report,
            "csupplysystem_candidate": csupplysystem_report,
            "computed_badges": [
                "exact_per_province_supply_tooltip_value_transient_not_persisted",
                "attrition_flag_reason_runtime_computed_unpinned",
                "hub_motorization_settings_unpinned_current_build",
                "convoy_logistics_pressure_unpinned_current_build",
                "csupplysystem_draw_x_remaining_arrays_unemitted_by_bounds",
            ],
            "compact_schema": {
                "domain_id": DOMAIN_ID,
                "domain_name": DOMAIN_NAME,
                "header_len": HEADER_STRUCT.size,
                "row_len": ROW_STRUCT.size,
                "row_count": len(rows),
                "province_rows": len(province_rows),
                "division_rows": len(division_rows),
                "csupply_input_rows": len(csupply_input_rows),
                "pressure_candidate_rows": 0,
                "badge_rows": len(badge_rows),
            },
        })
    except Exception as exc:  # noqa: BLE001
        report["status"] = "probe_failed"
        report["reason"] = str(exc)
    finally:
        p.__exit__()

    report["elapsed_s"] = round(time.time() - started, 3)
    _write_outputs(out_dir, report, payload if report["status"] == "live_roots_validated" else None)
    return report


def default_out_dir() -> Path:
    stamp = _dt.datetime.now().strftime("%Y%m%d_%H%M%S")
    return REPO_ROOT / "replay_native" / "runs" / f"supply_logistics_attrition_live_{stamp}"


def main(argv: Optional[Iterable[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pid", type=int, default=None, help="hoi4.exe pid; defaults to process name lookup")
    parser.add_argument("--out-dir", type=Path, default=None, help="proof output directory")
    args = parser.parse_args(list(argv) if argv is not None else None)
    out_dir = args.out_dir or default_out_dir()
    report = run(args.pid, out_dir)
    print(json.dumps({
        "status": report["status"],
        "out_dir": str(out_dir),
        "province_rows": report.get("compact_schema", {}).get("province_rows", 0),
        "division_rows": report.get("compact_schema", {}).get("division_rows", 0),
        "csupply_input_rows": report.get("compact_schema", {}).get("csupply_input_rows", 0),
        "payload_bytes": report.get("compact_payload", {}).get("bytes", 0),
        "reason": report.get("reason"),
    }, indent=2))
    return 0 if report["status"] == "live_roots_validated" else 1


if __name__ == "__main__":
    raise SystemExit(main())
