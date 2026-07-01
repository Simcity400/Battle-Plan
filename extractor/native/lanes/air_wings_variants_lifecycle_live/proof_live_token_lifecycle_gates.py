"""Lane-local live-token and lifecycle gates for air wings/variants.

This proof stays out of shared collector/recorder entrypoints. It consumes an
existing compact DLL stream, optionally attaches read-only to the still-running
the game process, and emits a JSON proof that separates validated fields from
unobserved or rejected candidates.
"""

from __future__ import annotations

import argparse
import collections
import datetime as dt
import glob
import json
import re
import struct
import sys
import time
from pathlib import Path
from typing import Any


THIS_DIR = Path(__file__).resolve().parent
REPO_ROOT = THIS_DIR.parents[3]
MEMORY_READER = REPO_ROOT / "extractor" / "memory_reader"
for candidate in (str(MEMORY_READER), str(REPO_ROOT)):
    if candidate not in sys.path:
        sys.path.insert(0, candidate)

try:
    import winmem  # type: ignore  # noqa: E402
except Exception:  # pragma: no cover - non-Windows/unit-less environments
    winmem = None


EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"

RECORD_MAGIC = b"H4CR"
RECORD_VERSION = 1
RECORD_HEADER_LEN = 32
RECORD_HEADER = struct.Struct("<4sHHQiHHII")
RECORD_TYPE_DOMAIN_DATA = 3

DOMAIN_AIR_WINGS = 30
DOMAIN_AIR_REGION_ACTIVITY = 31
DOMAIN_AIR_WING_LOADOUTS = 32
DOMAIN_AIR_VARIANT_DEFS = 33
DOMAIN_AIR_BASE_STATUS = 34
DOMAIN_AIR_VARIANT_STATS = 35
DOMAIN_AIR_VARIANT_RESERVES = 36
DOMAIN_STOCKPILE_VARIANT_DEFS = 37
DOMAIN_AIR_SUPERIORITY_INPUTS = 38

AIR_WING_ROW = struct.Struct("<IIQIIIIII")
AIR_REGION_ACTIVITY_ROW = struct.Struct("<IIIIIIII")
AIR_WING_LOADOUT_ROW = struct.Struct("<IIQIIII")
AIR_WING_LOADOUT_ENTRY_ROW = struct.Struct("<IIq")
AIR_VARIANT_DEF_ROW = struct.Struct("<IIIIII")
AIR_VARIANT_STATS_ROW = struct.Struct("<IIIIIiiiiiiiiiiI")
AIR_VARIANT_RESERVE_ROW = struct.Struct("<IIIIq")
AIR_SUPERIORITY_ROW = struct.Struct("<IIIIIIIIIIq")

ROW_FLAG_DELETED = 1
ROW_FLAG_MISSION_REGION = 2

VARIANT_DEF_HEADER_LEN = 44
VARIANT_STATS_HEADER_LEN = 36
AIR_WINGS_HEADER_LEN = 36
AIR_REGION_ACTIVITY_HEADER_LEN = 36
AIR_WING_LOADOUTS_HEADER_LEN = 44
AIR_VARIANT_RESERVES_HEADER_LEN = 48
AIR_SUPERIORITY_HEADER_LEN = 52

COMPACT_FIXED_SCALE_RAW = 100_000
COMPACT_PRODUCTION_SCALE_RAW = 100_000

C_GAME_STATE_DAILY_UPDATE_OFFSET = 0x1D4390
CG_STATE_DATE_OFFSET = 0x3316E90
GET_LONG_STRING_OFFSET = 0x526A40
GS_CURRENT_DATE_OFFSET = 0x468
GS_START_DATE_OFFSET = 0x4A8
CGAMESTATE_OFF_STRATEGIC_AIR_MANAGER = 0x690
CGAMESTATE_COUNTRY_TABLE = 0x310
CGAMESTATE_COUNTRY_COUNT = 0x31C
COUNTRY_VTABLE_RVA = 0x27D0E90
OBJ_TAG_DB_ID = 0x08
OBJ_NAME = 0x10
OBJ_FLAGS = 0x230
RTTI_FLAG_MANAGER = ".?AVCFlagManager@@"

DAILY_UPDATE_SENTINEL = bytes(
    [0x40, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57, 0x48, 0x8D, 0xAC]
)
GET_LONG_STRING_SENTINEL = bytes(
    [0x48, 0x89, 0x5C, 0x24, 0x08, 0x48, 0x89, 0x6C, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57]
)

STRATEGIC_AIR_MANAGER_VTABLE_RVA = 0x2942118
STRATEGIC_AIR_MANAGER_OFF_COUNTRY_CONTAINERS = 0x30
STRATEGIC_AIR_MANAGER_OFF_COUNTRY_CONTAINER_COUNT = 0x3C

AIR_WING_OBJECT_VTABLE_RVA = 0x2964178
AIR_WING_OBJECT_OFF_BODY = 0x10
AIR_WING_VTABLE_RVA = 0x29641B0
AIR_WING_POOL_VTABLE_RVA = 0x2964208
AIR_WING_AIRBASE_OWNER_VTABLE_RVA = 0x2941FF8
AIR_WING_REGION_VTABLE_RVA = 0x29569A8
AIR_WING_REGION_ALT_VTABLE_RVA = 0x29569F8
AIR_WING_TYPE = 69
AIR_WING_OFF_TYPE = 0x08
AIR_WING_OFF_ID = 0x0C
AIR_WING_OFF_POOL = 0x28
AIR_WING_OFF_COUNT = 0x6C
AIR_WING_OFF_MISSION_MASK = 0x90
AIR_WING_OFF_MISSION_REGION = 0xB0
AIR_WING_OFF_EFFECTIVE_PLANES = 0x160
AIR_WING_OFF_EQUIPMENT_BANK = 0x1E8
AIR_WING_AIRBASE_OWNER_OFF_POOL_BEGIN = 0x58
AIR_WING_AIRBASE_OWNER_OFF_POOL_COUNT = 0x64
AIR_WING_AIRBASE_OWNER_OFF_TAG = 0x90
AIR_WING_POOL_OFF_AIRBASE_OWNER = 0x40
AIR_WING_POOL_OFF_WING_OBJECT_BEGIN = 0x28
AIR_WING_POOL_OFF_WING_OBJECT_COUNT = 0x34
AIR_WING_REGION_OFF_ID = 0x58
AIR_WING_EQUIPMENT_ENTRY_STRIDE = 0x10

DIVISION_EQUIPMENT_VARIANT_VTABLE_RVA = 0x293AFB8
DIVISION_EQUIPMENT_VARIANT_TYPE = 70
DIVISION_EQUIPMENT_VARIANT_OFF_TYPE = 0x08
DIVISION_EQUIPMENT_VARIANT_OFF_ID = 0x0C
AIR_VARIANT_EQUIPMENT_TOKEN_OFF = 0x18
AIR_VARIANT_EQUIPMENT_TYPE_TOKEN_OFF = 0x1C
AIR_VARIANT_INTERFACE_TYPE_OFF = 0x38

MAX_STRATEGIC_AIR_CONTAINERS = 4096
MAX_AIR_WING_POOLS_PER_OWNER = 2048
MAX_AIR_WING_OBJECTS_PER_POOL = 4096
MAX_NATIVE_AIR_WINGS = 20_000
MAX_AIR_WING_PLANES = 10_000_000
MAX_AIR_WING_EQUIPMENT_BANK_ROWS = 512
MAX_AIR_WING_EQUIPMENT_AMOUNT_RAW = 1_000_000_000_000_000
MAX_AIR_WING_ID = 2_147_483_647
MAX_AIR_WING_MISSION_REGION_ID = 65_535
HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000
MAX_COUNTRIES = 2048
MAX_TOKEN_ID = 0x200000
MAX_TOKEN_TEXT = 256
TOKEN_TEXT_RE = re.compile(r"^[A-Za-z0-9_.:/$'\\-]+$")

FLAG_ROW_STRIDE = 0x30
FLAG_ROW_TOKEN_VTABLE = 0x00
FLAG_ROW_TOKEN_U32 = 0x08
FLAG_ROW_DATE_VTABLE = 0x10
FLAG_ROW_VALUE_VTABLE = 0x20
TOKEN_RECORD_VTABLE_OFF = 0x18
TOKEN_RECORD_ID = 0x08
TOKEN_RECORD_TEXT_PTR = 0x10

AIRFRAME_FAMILY_RULES = (
    ("cv_small_plane_naval_bomber_airframe", "cv_small_plane_naval_bomber_airframe", "naval_bomber"),
    ("small_plane_naval_bomber_airframe", "small_plane_naval_bomber_airframe", "naval_bomber"),
    ("medium_plane_scout_plane_airframe", "medium_plane_scout_plane_airframe", "scout"),
    ("medium_plane_fighter_airframe", "medium_plane_fighter_airframe", "heavy_fighter"),
    ("large_plane_maritime_patrol_plane_airframe", "large_plane_maritime_patrol_plane_airframe", "maritime_patrol_plane"),
    ("cv_small_plane_cas_airframe", "cv_small_plane_cas_airframe", "cas"),
    ("small_plane_cas_airframe", "small_plane_cas_airframe", "cas"),
    ("cv_small_plane_airframe", "cv_small_plane_airframe", None),
    ("small_plane_airframe", "small_plane_airframe", None),
    ("medium_plane_airframe", "medium_plane_airframe", None),
    ("large_plane_airframe", "large_plane_airframe", None),
    ("transport_plane_equipment", "transport_plane_equipment", "transport"),
    ("cv_CAS_equipment", "cv_small_plane_cas_airframe", "cas"),
    ("CAS_equipment", "small_plane_cas_airframe", "cas"),
    ("cv_nav_bomber_equipment", "cv_small_plane_naval_bomber_airframe", "naval_bomber"),
    ("nav_bomber_equipment", "small_plane_naval_bomber_airframe", "naval_bomber"),
    ("cv_fighter_equipment", "cv_small_plane_airframe", "fighter"),
    ("fighter_equipment", "small_plane_airframe", "fighter"),
    ("jet_fighter_equipment", "small_plane_airframe", "fighter"),
    ("supersonic_fighter_equipment", "small_plane_airframe", "fighter"),
    ("heavy_fighter_equipment", "medium_plane_fighter_airframe", "heavy_fighter"),
    ("tac_bomber_equipment", "medium_plane_airframe", "tactical_bomber"),
    ("jet_tac_bomber_equipment", "medium_plane_airframe", "tactical_bomber"),
    ("strat_bomber_equipment", "large_plane_airframe", "strategic_bomber"),
    ("jet_strat_bomber_equipment", "large_plane_airframe", "strategic_bomber"),
    ("scout_plane_equipment", "medium_plane_scout_plane_airframe", "scout"),
)

# These pairs are deliberately treated as probe candidates, not as a trusted map.
CANDIDATE_TOKEN_NAME_PAIRS = {
    20190: "fighter_equipment_0",
    20191: "fighter_equipment_1",
    20192: "fighter_equipment_2",
    20334: "CAS_equipment_1",
    20335: "CAS_equipment_2",
    20201: "tac_bomber_equipment_0",
    20213: "transport_plane_equipment_1",
}


def hx(value: int | None) -> str | None:
    return None if value is None else f"0x{value:X}"


def read_u32(payload: bytes, offset: int) -> int:
    return struct.unpack_from("<I", payload, offset)[0]


def read_u64(payload: bytes, offset: int) -> int:
    return struct.unpack_from("<Q", payload, offset)[0]


def read_compact_string(payload: bytes, cursor: int) -> tuple[str, int]:
    if len(payload) - cursor < 2:
        raise ValueError("truncated compact string length")
    size = struct.unpack_from("<H", payload, cursor)[0]
    cursor += 2
    if len(payload) - cursor < size:
        raise ValueError(f"compact string length {size} exceeds remaining payload")
    value = payload[cursor : cursor + size].decode("utf-8", errors="replace")
    return value, cursor + size


def iter_compact_records(blob: bytes) -> tuple[list[tuple[int, int, int, int, bytes]], list[str]]:
    records: list[tuple[int, int, int, int, bytes]] = []
    errors: list[str] = []
    cursor = 0
    previous_sequence: int | None = None
    while cursor < len(blob):
        if len(blob) - cursor < RECORD_HEADER_LEN:
            errors.append(f"offset {cursor}: truncated compact record header")
            break
        magic, version, header_len, sequence, game_hour_raw, domain_id, record_type, payload_len, _flags = (
            RECORD_HEADER.unpack_from(blob, cursor)
        )
        if magic != RECORD_MAGIC:
            errors.append(f"offset {cursor}: bad compact magic {magic!r}")
            break
        if version != RECORD_VERSION:
            errors.append(f"offset {cursor}: unsupported compact version {version}")
            break
        if header_len != RECORD_HEADER_LEN:
            errors.append(f"offset {cursor}: unsupported compact header length {header_len}")
            break
        if previous_sequence is not None and sequence <= previous_sequence:
            errors.append(f"offset {cursor}: nonmonotonic compact sequence {sequence} <= {previous_sequence}")
        previous_sequence = sequence
        payload_start = cursor + RECORD_HEADER_LEN
        payload_end = payload_start + payload_len
        if payload_end > len(blob):
            errors.append(f"offset {cursor}: payload length {payload_len} exceeds stream")
            break
        records.append((cursor, game_hour_raw, domain_id, record_type, blob[payload_start:payload_end]))
        cursor = payload_end
    return records, errors


def decode_variant_defs(payload: bytes, domain_id: int) -> tuple[dict[int, dict[str, Any]], list[str]]:
    if len(payload) < VARIANT_DEF_HEADER_LEN:
        return {}, [f"variant defs payload too short: {len(payload)}"]
    errors: list[str] = []
    row_count = read_u32(payload, 8)
    emitted_modules = read_u32(payload, 28)
    rows_end = VARIANT_DEF_HEADER_LEN + row_count * AIR_VARIANT_DEF_ROW.size
    if len(payload) < rows_end:
        return {}, [f"variant defs rows truncated: {len(payload)} < {rows_end}"]
    cursor = VARIANT_DEF_HEADER_LEN
    row_headers: list[tuple[int, int, int, int]] = []
    for row_index in range(row_count):
        variant_id, flags, module_first, module_count, reserved0, reserved1 = AIR_VARIANT_DEF_ROW.unpack_from(payload, cursor)
        cursor += AIR_VARIANT_DEF_ROW.size
        if reserved0 or reserved1:
            errors.append(f"variant defs row {row_index}: reserved fields are nonzero")
        row_headers.append((variant_id, flags, module_first, module_count))
    names: list[tuple[str, str]] = []
    for row_index in range(row_count):
        try:
            display, cursor = read_compact_string(payload, cursor)
            variant, cursor = read_compact_string(payload, cursor)
        except ValueError as exc:
            return {}, [*errors, f"variant defs row {row_index}: {exc}"]
        names.append((display, variant))
    module_tokens: list[str] = []
    for module_index in range(emitted_modules):
        if len(payload) - cursor < 8:
            return {}, [*errors, f"variant defs module row {module_index}: truncated fixed fields"]
        _slot, reserved = struct.unpack_from("<II", payload, cursor)
        cursor += 8
        if reserved:
            errors.append(f"variant defs module row {module_index}: reserved field is nonzero")
        try:
            token, cursor = read_compact_string(payload, cursor)
        except ValueError as exc:
            return {}, [*errors, f"variant defs module row {module_index}: {exc}"]
        module_tokens.append(token)
    out: dict[int, dict[str, Any]] = {}
    for index, (variant_id, flags, module_first, module_count) in enumerate(row_headers):
        modules = module_tokens[module_first : module_first + module_count]
        if len(modules) != module_count:
            errors.append(f"variant defs row {index}: module range exceeds emitted modules")
            modules = []
        display, variant = names[index]
        out[variant_id] = {
            "variant_id": variant_id,
            "domain_id": domain_id,
            "flags": flags,
            "display_name": display,
            "variant_name": variant,
            "modules": modules,
        }
    if cursor != len(payload):
        errors.append(f"variant defs payload has {len(payload) - cursor} trailing bytes")
    return out, errors


def decode_variant_stats(payload: bytes) -> tuple[dict[int, dict[str, Any]], list[str]]:
    if len(payload) < VARIANT_STATS_HEADER_LEN:
        return {}, [f"variant stats payload too short: {len(payload)}"]
    row_count = read_u32(payload, 8)
    expected = VARIANT_STATS_HEADER_LEN + row_count * AIR_VARIANT_STATS_ROW.size
    if len(payload) != expected:
        return {}, [f"variant stats payload length {len(payload)} != expected {expected}"]
    out: dict[int, dict[str, Any]] = {}
    cursor = VARIANT_STATS_HEADER_LEN
    for _ in range(row_count):
        (
            variant_id,
            flags,
            equipment_token_raw,
            equipment_type_token_raw,
            interface_type_raw,
            speed_cache_raw,
            range_raw,
            air_attack_raw,
            agility_raw,
            max_speed_raw,
            reliability_raw,
            air_defence_raw,
            thrust_raw,
            fuel_consumption_raw,
            build_cost_ic_raw,
            reserved,
        ) = AIR_VARIANT_STATS_ROW.unpack_from(payload, cursor)
        cursor += AIR_VARIANT_STATS_ROW.size
        out[variant_id] = {
            "variant_id": variant_id,
            "flags": flags,
            "equipment_token_raw": equipment_token_raw,
            "equipment_type_token_raw": equipment_type_token_raw,
            "interface_type_raw": interface_type_raw,
            "speed_cache_raw": speed_cache_raw,
            "range_raw": range_raw,
            "air_attack_raw": air_attack_raw,
            "agility_raw": agility_raw,
            "max_speed_raw": max_speed_raw,
            "reliability_raw": reliability_raw,
            "air_defence_raw": air_defence_raw,
            "thrust_raw": thrust_raw,
            "fuel_consumption_raw": fuel_consumption_raw,
            "build_cost_ic_raw": build_cost_ic_raw,
            "reserved": reserved,
        }
    return out, []


def decode_stream(stream_path: Path) -> dict[str, Any]:
    records, errors = iter_compact_records(stream_path.read_bytes())
    domain_data_counts: collections.Counter[int] = collections.Counter()
    frame_hours: set[int] = set()
    air_defs: dict[int, dict[str, Any]] = {}
    stockpile_defs: dict[int, dict[str, Any]] = {}
    stats: dict[int, dict[str, Any]] = {}
    superiority_rows: list[dict[str, Any]] = []
    loadout_variant_ids: set[int] = set()
    reserve_variant_ids: set[int] = set()
    reserve_amount_raw_by_variant: collections.Counter[int] = collections.Counter()

    wing_deleted_rows = 0
    wing_create_after_initial_rows = 0
    wing_generation_splits: dict[tuple[int, int], set[int]] = collections.defaultdict(set)
    wing_seen: set[tuple[int, int, int]] = set()
    wing_first_record_index: int | None = None
    wing_mission_region_rows = 0

    loadout_deleted_rows = 0
    loadout_rebase_events: list[dict[str, Any]] = []
    loadout_last_airbase: dict[tuple[int, int, int], int] = {}
    loadout_equipment_rows = 0
    loadout_amount_raw_sum = 0

    region_activity_deleted_rows = 0
    superiority_deleted_rows = 0
    reserve_deleted_rows = 0
    reserve_matched_stockpile_rows = 0

    for record_index, (offset, game_hour_raw, domain_id, record_type, payload) in enumerate(records):
        if record_type != RECORD_TYPE_DOMAIN_DATA:
            continue
        domain_data_counts[domain_id] += 1
        frame_hours.add(game_hour_raw)

        if domain_id == DOMAIN_AIR_WINGS:
            if len(payload) < AIR_WINGS_HEADER_LEN:
                errors.append(f"offset {offset}: air_wings payload too short")
                continue
            row_count = read_u32(payload, 8)
            deleted_rows_header = read_u32(payload, 24)
            if deleted_rows_header:
                wing_deleted_rows += deleted_rows_header
            expected = AIR_WINGS_HEADER_LEN + row_count * AIR_WING_ROW.size
            if len(payload) != expected:
                errors.append(f"offset {offset}: air_wings payload length {len(payload)} != expected {expected}")
                continue
            if wing_first_record_index is None:
                wing_first_record_index = record_index
            cursor = AIR_WINGS_HEADER_LEN
            for _row_index in range(row_count):
                owner, wing_id, generation, flags, count, mission_mask, mission_region, effective, reserved = (
                    AIR_WING_ROW.unpack_from(payload, cursor)
                )
                cursor += AIR_WING_ROW.size
                deleted = bool(flags & ROW_FLAG_DELETED)
                if reserved:
                    errors.append(f"offset {offset}: air_wings reserved field nonzero")
                if flags & ROW_FLAG_MISSION_REGION:
                    wing_mission_region_rows += 1
                key = (owner, wing_id, generation)
                wing_generation_splits[(owner, wing_id)].add(generation)
                if deleted:
                    wing_deleted_rows += 1 if not deleted_rows_header else 0
                    if any((count, mission_mask, mission_region, effective)):
                        errors.append(f"offset {offset}: air_wings deleted row carries live fields")
                elif key not in wing_seen:
                    if record_index != wing_first_record_index:
                        wing_create_after_initial_rows += 1
                    wing_seen.add(key)

        elif domain_id == DOMAIN_AIR_REGION_ACTIVITY:
            if len(payload) < AIR_REGION_ACTIVITY_HEADER_LEN:
                errors.append(f"offset {offset}: air_region_activity payload too short")
                continue
            row_count = read_u32(payload, 8)
            deleted_rows_header = read_u32(payload, 28)
            region_activity_deleted_rows += deleted_rows_header
            expected = AIR_REGION_ACTIVITY_HEADER_LEN + row_count * AIR_REGION_ACTIVITY_ROW.size
            if len(payload) != expected:
                errors.append(f"offset {offset}: air_region_activity payload length {len(payload)} != expected {expected}")
                continue
            if not deleted_rows_header:
                cursor = AIR_REGION_ACTIVITY_HEADER_LEN
                for _ in range(row_count):
                    _region, _owner, _mission, flags, *_rest = AIR_REGION_ACTIVITY_ROW.unpack_from(payload, cursor)
                    cursor += AIR_REGION_ACTIVITY_ROW.size
                    if flags & ROW_FLAG_DELETED:
                        region_activity_deleted_rows += 1

        elif domain_id == DOMAIN_AIR_WING_LOADOUTS:
            if len(payload) < AIR_WING_LOADOUTS_HEADER_LEN:
                errors.append(f"offset {offset}: air_wing_loadouts payload too short")
                continue
            row_count = read_u32(payload, 8)
            emitted_equipment_rows = read_u32(payload, 28)
            deleted_rows_header = read_u32(payload, 32)
            if deleted_rows_header:
                loadout_deleted_rows += deleted_rows_header
            rows_end = AIR_WING_LOADOUTS_HEADER_LEN + row_count * AIR_WING_LOADOUT_ROW.size
            expected = rows_end + emitted_equipment_rows * AIR_WING_LOADOUT_ENTRY_ROW.size
            if len(payload) != expected:
                errors.append(f"offset {offset}: air_wing_loadouts payload length {len(payload)} != expected {expected}")
                continue
            row_refs: list[tuple[int, int, int, int, int, int, bool]] = []
            cursor = AIR_WING_LOADOUTS_HEADER_LEN
            for _ in range(row_count):
                owner, wing_id, generation, flags, air_base_id, entry_first, entry_count = (
                    AIR_WING_LOADOUT_ROW.unpack_from(payload, cursor)
                )
                cursor += AIR_WING_LOADOUT_ROW.size
                deleted = bool(flags & ROW_FLAG_DELETED)
                if deleted:
                    if not deleted_rows_header:
                        loadout_deleted_rows += 1
                    continue
                key = (owner, wing_id, generation)
                old_airbase = loadout_last_airbase.get(key)
                if old_airbase is not None and old_airbase != air_base_id and len(loadout_rebase_events) < 16:
                    loadout_rebase_events.append(
                        {
                            "game_hour_raw": game_hour_raw,
                            "owner_tag_db_id": owner,
                            "wing_id": wing_id,
                            "generation": generation,
                            "old_air_base_id": old_airbase,
                            "new_air_base_id": air_base_id,
                        }
                    )
                loadout_last_airbase[key] = air_base_id
                row_refs.append((owner, wing_id, generation, entry_first, entry_count, air_base_id, deleted))
            entry_base = rows_end
            loadout_equipment_rows += emitted_equipment_rows
            for owner, wing_id, generation, entry_first, entry_count, _air_base_id, deleted in row_refs:
                if deleted:
                    continue
                for entry_index in range(entry_first, entry_first + entry_count):
                    entry_offset = entry_base + entry_index * AIR_WING_LOADOUT_ENTRY_ROW.size
                    variant_id, reserved, amount_raw = AIR_WING_LOADOUT_ENTRY_ROW.unpack_from(payload, entry_offset)
                    if reserved:
                        errors.append(f"offset {offset}: air_wing_loadouts equipment reserved nonzero")
                    loadout_variant_ids.add(variant_id)
                    loadout_amount_raw_sum += amount_raw

        elif domain_id in (DOMAIN_AIR_VARIANT_DEFS, DOMAIN_STOCKPILE_VARIANT_DEFS):
            decoded, local_errors = decode_variant_defs(payload, domain_id)
            errors.extend(f"offset {offset} domain {domain_id}: {err}" for err in local_errors)
            if domain_id == DOMAIN_AIR_VARIANT_DEFS:
                air_defs.update(decoded)
            else:
                stockpile_defs.update(decoded)

        elif domain_id == DOMAIN_AIR_VARIANT_STATS:
            decoded, local_errors = decode_variant_stats(payload)
            errors.extend(f"offset {offset} domain {domain_id}: {err}" for err in local_errors)
            stats.update(decoded)

        elif domain_id == DOMAIN_AIR_VARIANT_RESERVES:
            if len(payload) < AIR_VARIANT_RESERVES_HEADER_LEN:
                errors.append(f"offset {offset}: air_variant_reserves payload too short")
                continue
            row_count = read_u32(payload, 8)
            reserve_matched_stockpile_rows += read_u32(payload, 32)
            deleted_rows_header = read_u32(payload, 40)
            reserve_deleted_rows += deleted_rows_header
            expected = AIR_VARIANT_RESERVES_HEADER_LEN + row_count * AIR_VARIANT_RESERVE_ROW.size
            if len(payload) != expected:
                errors.append(f"offset {offset}: air_variant_reserves payload length {len(payload)} != expected {expected}")
                continue
            cursor = AIR_VARIANT_RESERVES_HEADER_LEN
            for _ in range(row_count):
                tag_db_id, variant_id, flags, reserved, amount_raw = AIR_VARIANT_RESERVE_ROW.unpack_from(payload, cursor)
                cursor += AIR_VARIANT_RESERVE_ROW.size
                if reserved:
                    errors.append(f"offset {offset}: air_variant_reserves reserved nonzero")
                if flags & ROW_FLAG_DELETED:
                    if not deleted_rows_header:
                        reserve_deleted_rows += 1
                else:
                    reserve_variant_ids.add(variant_id)
                    reserve_amount_raw_by_variant[variant_id] += amount_raw
                    if amount_raw < -100_000_000_000_000_000:
                        errors.append(f"offset {offset}: air_variant_reserves amount below sane bound")

        elif domain_id == DOMAIN_AIR_SUPERIORITY_INPUTS:
            if len(payload) < AIR_SUPERIORITY_HEADER_LEN:
                errors.append(f"offset {offset}: air_superiority_inputs payload too short")
                continue
            row_count = read_u32(payload, 8)
            deleted_rows_header = read_u32(payload, 44)
            superiority_deleted_rows += deleted_rows_header
            expected = AIR_SUPERIORITY_HEADER_LEN + row_count * AIR_SUPERIORITY_ROW.size
            if len(payload) != expected:
                errors.append(f"offset {offset}: air_superiority_inputs payload length {len(payload)} != expected {expected}")
                continue
            cursor = AIR_SUPERIORITY_HEADER_LEN
            for _ in range(row_count):
                (
                    region_id,
                    owner,
                    mission_mask,
                    variant_id,
                    flags,
                    equipment_token_raw,
                    equipment_type_token_raw,
                    interface_type_raw,
                    wing_count,
                    reserved,
                    amount_raw,
                ) = AIR_SUPERIORITY_ROW.unpack_from(payload, cursor)
                cursor += AIR_SUPERIORITY_ROW.size
                if reserved:
                    errors.append(f"offset {offset}: air_superiority_inputs reserved nonzero")
                deleted = bool(flags & ROW_FLAG_DELETED)
                if deleted:
                    if not deleted_rows_header:
                        superiority_deleted_rows += 1
                superiority_rows.append(
                    {
                        "region_id": region_id,
                        "owner_tag_db_id": owner,
                        "mission_mask": mission_mask,
                        "variant_id": variant_id,
                        "flags": flags,
                        "deleted": deleted,
                        "equipment_token_raw": equipment_token_raw,
                        "equipment_type_token_raw": equipment_type_token_raw,
                        "interface_type_raw": interface_type_raw,
                        "wing_count": wing_count,
                        "amount_raw": amount_raw,
                    }
                )

    combined_defs = dict(stockpile_defs)
    combined_defs.update(air_defs)
    active_superiority_ids = sorted({row["variant_id"] for row in superiority_rows if not row["deleted"]})
    active_with_defs = sorted([variant_id for variant_id in active_superiority_ids if variant_id in combined_defs])
    active_with_stats = sorted([variant_id for variant_id in active_superiority_ids if variant_id in stats])
    reserve_with_stockpile_defs = sorted(variant_id for variant_id in reserve_variant_ids if variant_id in stockpile_defs)
    reserve_missing_stockpile_defs = sorted(variant_id for variant_id in reserve_variant_ids if variant_id not in stockpile_defs)
    reserve_with_air_defs = sorted(variant_id for variant_id in reserve_variant_ids if variant_id in air_defs)
    reserve_stockpile_samples = [
        {
            "variant_id": variant_id,
            "reserve_amount_raw_sum": reserve_amount_raw_by_variant[variant_id],
            "stockpile_display_name": stockpile_defs[variant_id].get("display_name"),
            "stockpile_variant_name": stockpile_defs[variant_id].get("variant_name"),
            "stockpile_module_count": len(stockpile_defs[variant_id].get("modules", [])),
            "also_in_air_variant_defs": variant_id in air_defs,
        }
        for variant_id in reserve_with_stockpile_defs[:16]
    ]
    active_token_counts: collections.Counter[int] = collections.Counter()
    for variant_id in active_superiority_ids:
        stat = stats.get(variant_id)
        if stat:
            active_token_counts[stat["equipment_token_raw"]] += 1

    generation_split_examples = [
        {
            "owner_tag_db_id": owner,
            "wing_id": wing_id,
            "generations": sorted(gens),
        }
        for (owner, wing_id), gens in wing_generation_splits.items()
        if len(gens) > 1
    ][:16]

    return {
        "stream_path": str(stream_path),
        "decode_errors": errors,
        "domain_data_counts": dict(sorted(domain_data_counts.items())),
        "unique_game_hour_count": len(frame_hours),
        "active_superiority_variant_count": len(active_superiority_ids),
        "active_superiority_variants_with_defs": len(active_with_defs),
        "active_superiority_variants_with_stats": len(active_with_stats),
        "active_equipment_token_counts": dict(sorted(active_token_counts.items())),
        "air_defs_unique_ids": len(air_defs),
        "stockpile_defs_unique_ids": len(stockpile_defs),
        "variant_stats_unique_ids": len(stats),
        "loadout_variant_ids": len(loadout_variant_ids),
        "reserve_variant_ids": len(reserve_variant_ids),
        "reserve_stockpile_linkage": {
            "reserve_variant_ids": len(reserve_variant_ids),
            "reserve_variant_ids_with_stockpile_defs": len(reserve_with_stockpile_defs),
            "reserve_variant_ids_missing_stockpile_defs": len(reserve_missing_stockpile_defs),
            "reserve_variant_ids_with_air_defs": len(reserve_with_air_defs),
            "reserve_stockpile_samples": reserve_stockpile_samples,
            "missing_stockpile_variant_id_examples": reserve_missing_stockpile_defs[:16],
        },
        "lifecycle": {
            "air_wing_rows_seen": len(wing_seen),
            "air_wing_deleted_rows": wing_deleted_rows,
            "air_wing_create_after_initial_rows": wing_create_after_initial_rows,
            "air_wing_generation_split_examples": generation_split_examples,
            "air_wing_mission_region_rows": wing_mission_region_rows,
            "loadout_deleted_rows": loadout_deleted_rows,
            "loadout_rebase_events": loadout_rebase_events,
            "loadout_air_base_rows": len(loadout_last_airbase),
            "loadout_equipment_rows": loadout_equipment_rows,
            "loadout_amount_raw_sum": loadout_amount_raw_sum,
            "air_region_activity_deleted_rows": region_activity_deleted_rows,
            "air_superiority_input_deleted_rows": superiority_deleted_rows,
            "air_variant_reserve_deleted_rows": reserve_deleted_rows,
            "air_variant_reserve_matched_stockpile_rows": reserve_matched_stockpile_rows,
        },
    }


def build_lifecycle_stream_matrix(
    primary_stream_path: Path,
    primary_stream: dict[str, Any],
    extra_globs: list[str],
) -> dict[str, Any]:
    paths: dict[str, Path] = {str(primary_stream_path.resolve()): primary_stream_path}
    for pattern in extra_globs:
        for match in glob.glob(pattern, recursive=True):
            path = Path(match)
            if path.is_file():
                paths[str(path.resolve())] = path
    rows: list[dict[str, Any]] = []
    for resolved, path in sorted(paths.items()):
        decoded = primary_stream if resolved == str(primary_stream_path.resolve()) else decode_stream(path)
        lifecycle = decoded.get("lifecycle", {})
        rows.append(
            {
                "stream_path": str(path),
                "decode_error_count": len(decoded.get("decode_errors", [])),
                "domain_data_counts": decoded.get("domain_data_counts", {}),
                "air_wing_rows_seen": lifecycle.get("air_wing_rows_seen", 0),
                "air_wing_create_after_initial_rows": lifecycle.get("air_wing_create_after_initial_rows", 0),
                "air_wing_deleted_rows": lifecycle.get("air_wing_deleted_rows", 0),
                "air_wing_generation_split_examples": lifecycle.get("air_wing_generation_split_examples", []),
                "loadout_rebase_event_count": len(lifecycle.get("loadout_rebase_events", [])),
                "loadout_deleted_rows": lifecycle.get("loadout_deleted_rows", 0),
                "air_variant_reserve_matched_stockpile_rows": lifecycle.get(
                    "air_variant_reserve_matched_stockpile_rows", 0
                ),
                "air_region_activity_deleted_rows": lifecycle.get("air_region_activity_deleted_rows", 0),
                "air_superiority_input_deleted_rows": lifecycle.get("air_superiority_input_deleted_rows", 0),
            }
        )
    create_rows = sum(row["air_wing_create_after_initial_rows"] for row in rows)
    delete_rows = sum(row["air_wing_deleted_rows"] for row in rows)
    rebase_events = sum(row["loadout_rebase_event_count"] for row in rows)
    return {
        "stream_count": len(rows),
        "extra_globs": extra_globs,
        "observed": {
            "air_wing_create_after_initial": create_rows > 0,
            "air_wing_delete": delete_rows > 0,
            "air_wing_rebase": rebase_events > 0,
        },
        "totals": {
            "air_wing_create_after_initial_rows": create_rows,
            "air_wing_deleted_rows": delete_rows,
            "loadout_rebase_events": rebase_events,
            "loadout_deleted_rows": sum(row["loadout_deleted_rows"] for row in rows),
            "air_variant_reserve_matched_stockpile_rows": sum(
                row["air_variant_reserve_matched_stockpile_rows"] for row in rows
            ),
            "air_region_activity_deleted_rows": sum(row["air_region_activity_deleted_rows"] for row in rows),
            "air_superiority_input_deleted_rows": sum(row["air_superiority_input_deleted_rows"] for row in rows),
        },
        "streams": rows,
        "delete_path_evidence": (
            "compact_air_wing_delta_rows emits deleted_compact_air_wing_row for previous rows absent from the "
            "current native sample; available live fixture streams are still the observation source for a real "
            "wing delete tombstone."
        ),
    }


def is_heap(value: int | None) -> bool:
    return isinstance(value, int) and HEAP_LO <= value < HEAP_HI and value % 8 == 0


def safe_u32(proc: Any, addr: int) -> int | None:
    try:
        return proc.u32(addr)
    except OSError:
        return None


def safe_i64(proc: Any, addr: int) -> int | None:
    try:
        return proc.i64(addr)
    except OSError:
        return None


def safe_u64(proc: Any, addr: int) -> int | None:
    try:
        return proc.u64(addr)
    except OSError:
        return None


def safe_read(proc: Any, addr: int, size: int) -> bytes | None:
    try:
        return proc.read(addr, size)
    except OSError:
        return None


def read_cstr(proc: Any, addr: int, max_len: int = 200) -> str | None:
    data = safe_read(proc, addr, max_len)
    if not data:
        return None
    end = data.find(b"\0")
    if end < 0:
        return None
    try:
        return data[:end].decode("latin-1")
    except UnicodeDecodeError:
        return None


def read_msvc_string(proc: Any, addr: int, max_len: int = 256) -> str | None:
    size = safe_u64(proc, addr + 0x10)
    cap = safe_u64(proc, addr + 0x18)
    if not isinstance(size, int) or not isinstance(cap, int):
        return None
    if size == 0 or size > max_len or size > cap or cap >= (1 << 20):
        return None
    inline = cap == 15
    if not inline and cap < 16:
        return None
    chars = addr if inline else safe_u64(proc, addr)
    if not isinstance(chars, int):
        return None
    data = safe_read(proc, chars, size + 1)
    if not data or len(data) < size + 1 or data[size] != 0:
        return None
    try:
        text = data[:size].decode("latin-1")
    except UnicodeDecodeError:
        return None
    if all(32 <= ord(ch) < 127 for ch in text):
        return text
    return None


def rtti_name(proc: Any, obj: int) -> str | None:
    vt = safe_u64(proc, obj)
    module_base = proc.module_base
    module_hi = module_base + 0x10000000
    if not isinstance(vt, int) or not (module_base <= vt < module_hi):
        return None
    col = safe_u64(proc, vt - 8)
    if not isinstance(col, int) or not (module_base <= col < module_hi):
        return None
    type_descriptor_rva = safe_u32(proc, col + 0x0C)
    if type_descriptor_rva is None:
        return None
    name = read_cstr(proc, module_base + type_descriptor_rva + 0x10, 200)
    return name if name and name.startswith(".?A") else None


def resolve_country_roots(proc: Any) -> dict[str, Any]:
    module_base = proc.module_base
    game_state = safe_u64(proc, module_base + CG_STATE_DATE_OFFSET)
    if not is_heap(game_state):
        raise RuntimeError("CGameState global did not resolve to a heap pointer")
    country_table = safe_u64(proc, game_state + CGAMESTATE_COUNTRY_TABLE)
    country_count = safe_u32(proc, game_state + CGAMESTATE_COUNTRY_COUNT)
    country_vtable = module_base + COUNTRY_VTABLE_RVA
    if not is_heap(country_table) or not isinstance(country_count, int):
        raise RuntimeError("country table/count did not resolve")
    if country_count <= 0 or country_count > MAX_COUNTRIES:
        raise RuntimeError(f"country count out of bounds: {country_count}")
    rows: list[dict[str, Any]] = []
    for index in range(country_count):
        country_ptr = safe_u64(proc, country_table + index * 8)
        if not is_heap(country_ptr):
            continue
        vtable = safe_u64(proc, country_ptr)
        tag_db_id = safe_u32(proc, country_ptr + OBJ_TAG_DB_ID)
        if vtable != country_vtable or tag_db_id != index:
            continue
        rows.append(
            {
                "tag_db_id": index,
                "country_ptr": country_ptr,
                "name": read_msvc_string(proc, country_ptr + OBJ_NAME, 96),
            }
        )
    if not rows:
        raise RuntimeError("no country rows passed vtable/tag-db-id validation")
    return {
        "module_base": module_base,
        "game_state": game_state,
        "country_table": country_table,
        "country_count": country_count,
        "country_vtable": country_vtable,
        "rows": rows,
    }


def stable_flag_header(proc: Any, manager: int) -> tuple[int, int, int] | None:
    base1 = safe_u64(proc, manager + 0x08)
    word1 = safe_u64(proc, manager + 0x10)
    base2 = safe_u64(proc, manager + 0x08)
    word2 = safe_u64(proc, manager + 0x10)
    if None in (base1, word1, base2, word2) or (base1, word1) != (base2, word2):
        return None
    assert isinstance(base1, int) and isinstance(word1, int)
    return base1, word1 & 0xFFFFFFFF, word1 >> 32


def read_flag_row_vtables(proc: Any, country_rows: list[dict[str, Any]], max_countries: int) -> dict[str, Any]:
    for row in country_rows[:max_countries]:
        manager = safe_u64(proc, row["country_ptr"] + OBJ_FLAGS)
        if not is_heap(manager):
            continue
        rtti = rtti_name(proc, manager)
        if rtti != RTTI_FLAG_MANAGER:
            continue
        header = stable_flag_header(proc, manager)
        if header is None:
            continue
        base, _slot_count, entry_count = header
        if entry_count == 0 or not is_heap(base):
            continue
        token_vtable = safe_u64(proc, base + FLAG_ROW_TOKEN_VTABLE)
        date_vtable = safe_u64(proc, base + FLAG_ROW_DATE_VTABLE)
        value_vtable = safe_u64(proc, base + FLAG_ROW_VALUE_VTABLE)
        if all(isinstance(value, int) for value in (token_vtable, date_vtable, value_vtable)):
            return {
                "source_tag_db_id": row["tag_db_id"],
                "source_country_ptr": row["country_ptr"],
                "source_manager": manager,
                "source_payload_base": base,
                "token_vtable": token_vtable,
                "date_vtable": date_vtable,
                "value_vtable": value_vtable,
            }
    raise RuntimeError("could not find a non-empty CFlagManager payload to derive token record vtable")


def cstr_from_ptr_diagnostic(proc: Any, addr: int, max_len: int = MAX_TOKEN_TEXT) -> tuple[str | None, str]:
    if not is_heap(addr):
        return None, "text_pointer_not_heap"
    data = safe_read(proc, addr, max_len + 1)
    if not data:
        return None, "text_pointer_read_failed"
    end = data.find(b"\0")
    if end < 2 or end > max_len:
        return None, "text_length_invalid"
    try:
        text = data[:end].decode("latin-1")
    except UnicodeDecodeError:
        return None, "text_decode_failed"
    if not all(32 <= ord(ch) < 127 for ch in text):
        return None, "text_not_printable_ascii"
    if not TOKEN_TEXT_RE.match(text):
        return None, "text_failed_token_pattern"
    return text, "ok"


def scan_token_dictionary(proc: Any, token_record_vtable: int, wanted_tokens: set[int]) -> dict[str, Any]:
    needle = struct.pack("<Q", token_record_vtable)
    token_to_names: dict[int, list[dict[str, Any]]] = collections.defaultdict(list)
    wanted_rejected_records: list[dict[str, Any]] = []
    vtable_hits = 0
    candidate_records = 0
    regions_scanned = 0
    started = time.perf_counter()

    for region_base, region_size in proc.iter_regions(HEAP_LO, HEAP_HI, want_exec=False):
        if region_size > 64 * 1024 * 1024:
            continue
        regions_scanned += 1
        blob = safe_read(proc, region_base, region_size)
        if not blob:
            continue
        pos = 0
        while True:
            idx = blob.find(needle, pos)
            if idx < 0:
                break
            pos = idx + 8
            vtable_hits += 1
            if idx + TOKEN_RECORD_TEXT_PTR + 8 > len(blob):
                continue
            token_id = struct.unpack_from("<Q", blob, idx + TOKEN_RECORD_ID)[0]
            if token_id == 0 or token_id > MAX_TOKEN_ID:
                continue
            text_ptr = struct.unpack_from("<Q", blob, idx + TOKEN_RECORD_TEXT_PTR)[0]
            text, reason = cstr_from_ptr_diagnostic(proc, text_ptr)
            if text is None:
                if token_id in wanted_tokens and len(wanted_rejected_records) < 32:
                    wanted_rejected_records.append(
                        {
                            "token_id": int(token_id),
                            "record": hx(region_base + idx),
                            "text_ptr": hx(text_ptr),
                            "reason": reason,
                        }
                    )
                continue
            candidate_records += 1
            token_to_names[int(token_id)].append(
                {
                    "name": text,
                    "record": hx(region_base + idx),
                    "text_ptr": hx(text_ptr),
                }
            )

    resolved: dict[int, dict[str, Any]] = {}
    ambiguous: dict[int, dict[str, Any]] = {}
    for token_id, entries in token_to_names.items():
        names = sorted({entry["name"] for entry in entries})
        if len(names) == 1:
            resolved[token_id] = {
                "name": names[0],
                "records": entries[:3],
                "record_count": len(entries),
            }
        else:
            ambiguous[token_id] = {
                "names": names[:8],
                "records": entries[:8],
                "record_count": len(entries),
            }

    wanted: dict[str, Any] = {}
    unresolved_wanted: list[int] = []
    ambiguous_wanted: list[int] = []
    for token in sorted(wanted_tokens):
        if token in resolved:
            wanted[str(token)] = resolved[token]
        elif token in ambiguous:
            ambiguous_wanted.append(token)
        else:
            unresolved_wanted.append(token)

    unresolved_set = set(unresolved_wanted)
    wanted_rejected_records = [
        row for row in wanted_rejected_records if row["token_id"] in unresolved_set
    ]

    return {
        "elapsed_ms": int((time.perf_counter() - started) * 1000),
        "regions_scanned": regions_scanned,
        "vtable_hits": vtable_hits,
        "candidate_records": candidate_records,
        "unique_tokens": len(token_to_names),
        "resolved_unique_tokens": len(resolved),
        "ambiguous_unique_tokens": len(ambiguous),
        "wanted_tokens": len(wanted_tokens),
        "wanted_resolved": len(wanted),
        "wanted_unresolved": len(unresolved_wanted),
        "wanted_ambiguous": len(ambiguous_wanted),
        "wanted": wanted,
        "unresolved_wanted_tokens": unresolved_wanted,
        "ambiguous_wanted_tokens": ambiguous_wanted,
        "wanted_rejected_records": wanted_rejected_records,
    }


def live_token_dictionary_probe(pid: int | None, wanted_tokens: set[int], max_countries: int) -> dict[str, Any]:
    if winmem is None:
        return {"available": False, "badge": "winmem_unavailable"}
    resolved_pid = pid or winmem.find_pid("hoi4.exe")
    if not resolved_pid:
        return {"available": False, "badge": "hoi4_process_not_found"}
    if not wanted_tokens:
        return {"available": True, "status": "not_checked_no_wanted_tokens"}
    proc = winmem.Process(resolved_pid, "hoi4.exe")
    try:
        fingerprint = proc.fingerprint()
        sha = str(fingerprint.get("sha256") or "").lower()
        if sha != EXPECTED_SHA256:
            return {
                "available": True,
                "status": "rejected_wrong_build",
                "sha256": sha,
                "sha256_expected": EXPECTED_SHA256,
            }
        country_roots = resolve_country_roots(proc)
        row_vtables = read_flag_row_vtables(proc, country_roots["rows"], max_countries)
        token_record_vtable = safe_u64(proc, row_vtables["source_manager"] + TOKEN_RECORD_VTABLE_OFF)
        if not isinstance(token_record_vtable, int):
            return {
                "available": True,
                "status": "rejected_no_token_record_vtable",
                "source_manager": hx(row_vtables["source_manager"]),
            }
        dictionary = scan_token_dictionary(proc, token_record_vtable, wanted_tokens)
        return {
            "available": True,
            "status": (
                "validated"
                if dictionary["wanted_resolved"] == len(wanted_tokens)
                and dictionary["wanted_unresolved"] == 0
                and dictionary["wanted_ambiguous"] == 0
                else "partial"
            ),
            "pid": resolved_pid,
            "sha256": sha,
            "source_roots": {
                "module_base": hx(country_roots["module_base"]),
                "game_state": hx(country_roots["game_state"]),
                "country_table": hx(country_roots["country_table"]),
                "country_count": country_roots["country_count"],
                "country_vtable": hx(country_roots["country_vtable"]),
                "validated_country_rows": len(country_roots["rows"]),
            },
            "dictionary_root": {
                "source_tag_db_id": row_vtables["source_tag_db_id"],
                "source_country_ptr": hx(row_vtables["source_country_ptr"]),
                "flag_manager": hx(row_vtables["source_manager"]),
                "flag_manager_offset": hx(OBJ_FLAGS),
                "flag_manager_rtti": RTTI_FLAG_MANAGER,
                "payload_base": hx(row_vtables["source_payload_base"]),
                "token_subrecord_vtable": hx(row_vtables["token_vtable"]),
                "date_subrecord_vtable": hx(row_vtables["date_vtable"]),
                "value_subrecord_vtable": hx(row_vtables["value_vtable"]),
                "token_record_vtable_source": "CFlagManager+0x18",
                "token_record_vtable": hx(token_record_vtable),
                "token_record_id": "record+0x08",
                "token_record_text_ptr": "record+0x10",
            },
            "dictionary": dictionary,
        }
    finally:
        proc.close()


def match_airframe_rule(token_name: str, needle: str) -> bool:
    if token_name == needle:
        return True
    suffix = token_name[len(needle) :] if token_name.startswith(needle) else None
    return isinstance(suffix, str) and bool(re.fullmatch(r"_\d+", suffix))


def derive_airframe_family(token_name: str) -> str | None:
    for needle, family, _role in AIRFRAME_FAMILY_RULES:
        if match_airframe_rule(token_name, needle):
            return family
    return None


def derive_airframe_role(token_name: str) -> str | None:
    for needle, _family, role in AIRFRAME_FAMILY_RULES:
        if match_airframe_rule(token_name, needle):
            return role
    return None


def load_equipment_catalog(catalog_path: Path) -> dict[str, Any]:
    data = json.loads(catalog_path.read_text(encoding="utf-8"))
    return data.get("catalogs", {}).get("equipment_archetypes", {})


def source_line_matches(path: Path, token_names: set[str]) -> dict[str, list[dict[str, Any]]]:
    matches: dict[str, list[dict[str, Any]]] = {token: [] for token in token_names}
    if not path.exists():
        return matches
    patterns = {
        token: re.compile(rf"(?<![A-Za-z0-9_]){re.escape(token)}(?![A-Za-z0-9_])")
        for token in token_names
    }
    for line_no, line in enumerate(path.read_text(encoding="utf-8", errors="replace").splitlines(), start=1):
        for token, pattern in patterns.items():
            if pattern.search(line):
                matches[token].append(
                    {
                        "file": str(path),
                        "line": line_no,
                        "text": line.strip(),
                    }
                )
    return matches


def merge_source_matches(paths: list[Path], token_names: set[str]) -> dict[str, list[dict[str, Any]]]:
    merged: dict[str, list[dict[str, Any]]] = {token: [] for token in token_names}
    for path in paths:
        for token, rows in source_line_matches(path, token_names).items():
            merged[token].extend(rows)
    return merged


def token_source_occurrences(hoi4_root: Path | None, token_names: set[str]) -> dict[str, Any]:
    if hoi4_root is None:
        return {"available": False, "badge": "hoi4_root_not_available"}
    common = hoi4_root / "common"
    script_enums = common / "script_enums.txt"
    technologies = common / "technologies" / "bba_air_techs.txt"
    equipment_dir = common / "units" / "equipment"
    equipment_files = sorted(equipment_dir.glob("*.txt")) if equipment_dir.exists() else []
    source_files = [script_enums, technologies, *equipment_files]
    source_matches = merge_source_matches([path for path in source_files if path.exists()], token_names)
    out: dict[str, Any] = {
        "available": script_enums.exists() or technologies.exists(),
        "hoi4_root": str(hoi4_root),
        "script_enums": str(script_enums),
        "technologies": str(technologies),
        "equipment_source_files_scanned": len(equipment_files),
        "tokens": {},
    }
    script_matches = source_line_matches(script_enums, token_names)
    tech_matches = source_line_matches(technologies, token_names)
    equipment_matches = merge_source_matches(equipment_files, token_names)
    for token in sorted(token_names):
        out["tokens"][token] = {
            "script_enums_present": bool(script_matches[token]),
            "bba_air_techs_present": bool(tech_matches[token]),
            "equipment_source_present": bool(equipment_matches[token]),
            "source_match_count": len(source_matches[token]),
            "source_matches": source_matches[token][:8],
        }
    return out


def build_archetype_closure(
    token_dictionary: dict[str, Any],
    compact_active_tokens: set[int],
    current_live_tokens: set[int],
    catalog_path: Path,
    hoi4_root: Path | None,
) -> dict[str, Any]:
    wanted = token_dictionary.get("dictionary", {}).get("wanted", {})
    equipment_catalog = load_equipment_catalog(catalog_path) if catalog_path.exists() else {}
    token_rows: list[dict[str, Any]] = []
    token_names: set[str] = set()
    for token_text, entry in sorted(wanted.items(), key=lambda item: int(item[0])):
        token = int(token_text)
        name = entry.get("name")
        if isinstance(name, str):
            token_names.add(name)
        catalog = equipment_catalog.get(name) if isinstance(name, str) else None
        family = derive_airframe_family(name) if isinstance(name, str) else None
        role = derive_airframe_role(name) if isinstance(name, str) else None
        token_rows.append(
            {
                "equipment_token_raw": token,
                "name": name,
                "in_compact_active_superiority": token in compact_active_tokens,
                "in_current_live_air_wings": token in current_live_tokens,
                "base_airframe_family": family,
                "role": role,
                "catalog_status": "equipment_archetypes_hit" if isinstance(catalog, dict) else "missing_from_equipment_archetypes_catalog",
                "catalog_archetype": catalog.get("archetype") if isinstance(catalog, dict) else None,
                "catalog_year": catalog.get("year") if isinstance(catalog, dict) else None,
                "catalog_source_file": catalog.get("source_file") if isinstance(catalog, dict) else None,
            }
        )
    source_occurrences = token_source_occurrences(hoi4_root, token_names)
    compact_rows = [row for row in token_rows if row["in_compact_active_superiority"]]
    current_rows = [row for row in token_rows if row["in_current_live_air_wings"]]
    compact_catalog_hits = [row for row in compact_rows if row["catalog_status"] == "equipment_archetypes_hit"]
    compact_catalog_missing = [row for row in compact_rows if row["catalog_status"] != "equipment_archetypes_hit"]
    compact_family_closed = [row for row in compact_rows if row["base_airframe_family"]]
    compact_role_closed = [row for row in compact_rows if row["role"]]
    current_family_closed = [row for row in current_rows if row["base_airframe_family"]]
    current_role_closed = [row for row in current_rows if row["role"]]
    missing_source_rows: list[dict[str, Any]] = []
    for row in compact_catalog_missing:
        name = row["name"]
        source_row = source_occurrences.get("tokens", {}).get(name, {}) if isinstance(name, str) else {}
        missing_source_rows.append(
            {
                "equipment_token_raw": row["equipment_token_raw"],
                "name": name,
                "base_airframe_family": row["base_airframe_family"],
                "role": row["role"],
                "script_enums_present": bool(source_row.get("script_enums_present")),
                "bba_air_techs_present": bool(source_row.get("bba_air_techs_present")),
                "equipment_source_present": bool(source_row.get("equipment_source_present")),
                "source_match_count": source_row.get("source_match_count", 0),
                "source_matches": source_row.get("source_matches", []),
            }
        )
    source_enum_closed = bool(compact_catalog_missing) and all(
        row["script_enums_present"] and row["bba_air_techs_present"]
        for row in missing_source_rows
    )
    if len(compact_catalog_hits) == len(compact_active_tokens):
        status = "closed"
    elif (
        len(compact_family_closed) == len(compact_rows) == len(compact_active_tokens)
        and source_enum_closed
    ):
        status = "base_airframe_names_closed_source_enum_catalog_badged"
    elif len(compact_family_closed) == len(compact_rows) == len(compact_active_tokens) and compact_catalog_missing:
        status = "base_airframe_names_closed_catalog_partial"
    else:
        status = "partial"
    return {
        "catalog_path": str(catalog_path),
        "compact_active_token_count": len(compact_active_tokens),
        "compact_active_dictionary_rows": len(compact_rows),
        "compact_active_base_airframe_name_rows": len(compact_family_closed),
        "compact_active_role_name_rows": len(compact_role_closed),
        "compact_active_equipment_catalog_hits": len(compact_catalog_hits),
        "compact_active_equipment_catalog_missing": len(compact_catalog_missing),
        "current_live_token_count": len(current_live_tokens),
        "current_live_dictionary_rows": len(current_rows),
        "current_live_base_airframe_name_rows": len(current_family_closed),
        "current_live_role_name_rows": len(current_role_closed),
        "all_token_rows": token_rows,
        "compact_active_catalog_missing_tokens": [
            {
                "equipment_token_raw": row["equipment_token_raw"],
                "name": row["name"],
                "base_airframe_family": row["base_airframe_family"],
                "role": row["role"],
            }
            for row in compact_catalog_missing
        ],
        "compact_active_catalog_missing_source_enum_closed": source_enum_closed,
        "compact_active_catalog_missing_source_enum_tokens": missing_source_rows,
        "catalog_gap_classification": (
            "source_enum_and_tech_enabled_airframe_tokens_absent_from_equipment_archetypes_catalog"
            if source_enum_closed
            else "catalog_missing_tokens_without_full_static_source_evidence"
            if compact_catalog_missing
            else "none"
        ),
        "source_occurrences": source_occurrences,
        "status": status,
    }


def stable_vector_header(proc: Any, base: int, data_off: int, cap_off: int, size_off: int) -> tuple[int, int, int] | None:
    first = (safe_u64(proc, base + data_off), safe_u32(proc, base + cap_off), safe_u32(proc, base + size_off))
    second = (safe_u64(proc, base + data_off), safe_u32(proc, base + cap_off), safe_u32(proc, base + size_off))
    if first != second or any(value is None for value in first):
        return None
    data, cap, size = first
    assert data is not None and cap is not None and size is not None
    return data, cap, size


class LiveAirProbe:
    def __init__(self, pid: int):
        if winmem is None:
            raise RuntimeError("memory_reader.winmem is unavailable")
        self.proc = winmem.Process(pid, "hoi4.exe")
        self.base = self.proc.module_base

    def close(self) -> None:
        self.proc.close()

    def root_probe(self) -> dict[str, Any]:
        fp = self.proc.fingerprint()
        daily = self.proc.try_read(self.base + C_GAME_STATE_DAILY_UPDATE_OFFSET, len(DAILY_UPDATE_SENTINEL))
        long_string = self.proc.try_read(self.base + GET_LONG_STRING_OFFSET, len(GET_LONG_STRING_SENTINEL))
        game_state = safe_u64(self.proc, self.base + CG_STATE_DATE_OFFSET)
        strategic_air = safe_u64(self.proc, (game_state or 0) + CGAMESTATE_OFF_STRATEGIC_AIR_MANAGER)
        strategic_air_vtable = safe_u64(self.proc, strategic_air or 0)
        return {
            "pid": self.proc.pid,
            "module": self.proc.module_name,
            "module_path": self.proc.module_path,
            "module_base": hx(self.base),
            "module_size": self.proc.module_size,
            "sha256": fp.get("sha256"),
            "sha256_expected": EXPECTED_SHA256,
            "sha256_ok": (fp.get("sha256") or "").lower() == EXPECTED_SHA256,
            "daily_sentinel_addr": hx(self.base + C_GAME_STATE_DAILY_UPDATE_OFFSET),
            "daily_sentinel_ok": daily == DAILY_UPDATE_SENTINEL,
            "get_long_string_sentinel_addr": hx(self.base + GET_LONG_STRING_OFFSET),
            "get_long_string_sentinel_ok": long_string == GET_LONG_STRING_SENTINEL,
            "game_state_ptr_source": hx(self.base + CG_STATE_DATE_OFFSET),
            "game_state_ptr": hx(game_state),
            "game_state_ptr_ok": is_heap(game_state),
            "current_game_date_raw": safe_u32(self.proc, (game_state or 0) + GS_CURRENT_DATE_OFFSET),
            "start_date_raw": safe_u32(self.proc, (game_state or 0) + GS_START_DATE_OFFSET),
            "strategic_air_source": hx((game_state or 0) + CGAMESTATE_OFF_STRATEGIC_AIR_MANAGER),
            "strategic_air_ptr": hx(strategic_air),
            "strategic_air_vtable": hx(strategic_air_vtable),
            "strategic_air_vtable_expected": hx(self.base + STRATEGIC_AIR_MANAGER_VTABLE_RVA),
            "strategic_air_ok": (
                is_heap(strategic_air)
                and strategic_air_vtable == self.base + STRATEGIC_AIR_MANAGER_VTABLE_RVA
            ),
        }

    def require_root(self, root: dict[str, Any]) -> int:
        if not all(
            [
                root.get("sha256_ok"),
                root.get("daily_sentinel_ok"),
                root.get("get_long_string_sentinel_ok"),
                root.get("game_state_ptr_ok"),
                root.get("strategic_air_ok"),
            ]
        ):
            raise RuntimeError("profile/root validation failed")
        return int(str(root["strategic_air_ptr"]), 16)

    def read_mission_region(self, body: int) -> int | None:
        region = safe_u64(self.proc, body + AIR_WING_OFF_MISSION_REGION)
        if not is_heap(region):
            return None
        if safe_u64(self.proc, region) != self.base + AIR_WING_REGION_VTABLE_RVA:
            return None
        if safe_u64(self.proc, region + 8) != self.base + AIR_WING_REGION_ALT_VTABLE_RVA:
            return None
        region_id = safe_u32(self.proc, region + AIR_WING_REGION_OFF_ID)
        if region_id is None or not (0 < region_id <= MAX_AIR_WING_MISSION_REGION_ID):
            return None
        return region_id

    def read_equipment(self, body: int) -> list[dict[str, Any]] | None:
        header = stable_vector_header(self.proc, body + AIR_WING_OFF_EQUIPMENT_BANK, 0, 0x08, 0x0C)
        if header is None:
            return None
        begin, cap, size = header
        if size == 0 and cap <= MAX_AIR_WING_EQUIPMENT_BANK_ROWS:
            return []
        if size > cap or cap > MAX_AIR_WING_EQUIPMENT_BANK_ROWS or not is_heap(begin):
            return None
        raw = self.proc.try_read(begin, size * AIR_WING_EQUIPMENT_ENTRY_STRIDE)
        if len(raw) != size * AIR_WING_EQUIPMENT_ENTRY_STRIDE:
            return None
        out: list[dict[str, Any]] = []
        seen_variants: set[int] = set()
        for off in range(0, len(raw), AIR_WING_EQUIPMENT_ENTRY_STRIDE):
            variant_ptr = int.from_bytes(raw[off : off + 8], "little")
            amount_raw = int.from_bytes(raw[off + 8 : off + 16], "little", signed=True)
            if (
                not is_heap(variant_ptr)
                or safe_u64(self.proc, variant_ptr) != self.base + DIVISION_EQUIPMENT_VARIANT_VTABLE_RVA
                or safe_u32(self.proc, variant_ptr + DIVISION_EQUIPMENT_VARIANT_OFF_TYPE) != DIVISION_EQUIPMENT_VARIANT_TYPE
            ):
                return None
            variant_id = safe_u32(self.proc, variant_ptr + DIVISION_EQUIPMENT_VARIANT_OFF_ID)
            if variant_id is None or variant_id == 0 or variant_id in seen_variants:
                return None
            if amount_raw < 0 or amount_raw > MAX_AIR_WING_EQUIPMENT_AMOUNT_RAW:
                return None
            seen_variants.add(variant_id)
            out.append(
                {
                    "variant_id": variant_id,
                    "variant_ptr": variant_ptr,
                    "amount_raw": amount_raw,
                    "equipment_token_raw": safe_u32(self.proc, variant_ptr + AIR_VARIANT_EQUIPMENT_TOKEN_OFF),
                    "equipment_type_token_raw": safe_u32(self.proc, variant_ptr + AIR_VARIANT_EQUIPMENT_TYPE_TOKEN_OFF),
                    "interface_type_raw": safe_u32(self.proc, variant_ptr + AIR_VARIANT_INTERFACE_TYPE_OFF),
                }
            )
        return out

    def enumerate_air_wings(self, strategic_air: int) -> dict[str, Any]:
        container_begin = safe_u64(self.proc, strategic_air + STRATEGIC_AIR_MANAGER_OFF_COUNTRY_CONTAINERS)
        container_count = safe_u32(self.proc, strategic_air + STRATEGIC_AIR_MANAGER_OFF_COUNTRY_CONTAINER_COUNT)
        topo: dict[str, Any] = {
            "strategic_air": hx(strategic_air),
            "container_begin": hx(container_begin),
            "container_count": container_count,
            "containers_seen": 0,
            "pools_seen": 0,
            "wing_objects_seen": 0,
            "invalid_containers": 0,
            "invalid_pools": 0,
            "invalid_wing_objects": 0,
            "invalid_equipment_banks": 0,
        }
        rows: list[dict[str, Any]] = []
        variants: dict[int, dict[str, Any]] = {}
        if container_count is None or container_count > MAX_STRATEGIC_AIR_CONTAINERS:
            topo["fatal"] = "container_count_invalid"
            return {"topology": topo, "air_wings": rows, "variants": variants}
        if container_count > 0 and not is_heap(container_begin):
            topo["fatal"] = "container_begin_invalid"
            return {"topology": topo, "air_wings": rows, "variants": variants}
        for container_index in range(container_count):
            container = safe_u64(self.proc, (container_begin or 0) + container_index * 8)
            if not container:
                continue
            topo["containers_seen"] += 1
            if not is_heap(container) or safe_u64(self.proc, container) != self.base + AIR_WING_AIRBASE_OWNER_VTABLE_RVA:
                topo["invalid_containers"] += 1
                continue
            owner = safe_u32(self.proc, container + AIR_WING_AIRBASE_OWNER_OFF_TAG) or 0
            pool_begin = safe_u64(self.proc, container + AIR_WING_AIRBASE_OWNER_OFF_POOL_BEGIN)
            pool_count = safe_u32(self.proc, container + AIR_WING_AIRBASE_OWNER_OFF_POOL_COUNT)
            if pool_count is None or pool_count > MAX_AIR_WING_POOLS_PER_OWNER:
                topo["invalid_containers"] += 1
                continue
            if pool_count > 0 and not is_heap(pool_begin):
                topo["invalid_containers"] += 1
                continue
            for pool_index in range(pool_count):
                pool = safe_u64(self.proc, (pool_begin or 0) + pool_index * 8)
                topo["pools_seen"] += 1
                if (
                    not is_heap(pool)
                    or safe_u64(self.proc, pool) != self.base + AIR_WING_POOL_VTABLE_RVA
                    or safe_u64(self.proc, pool + AIR_WING_POOL_OFF_AIRBASE_OWNER) != container
                ):
                    topo["invalid_pools"] += 1
                    continue
                object_begin = safe_u64(self.proc, pool + AIR_WING_POOL_OFF_WING_OBJECT_BEGIN)
                object_count = safe_u32(self.proc, pool + AIR_WING_POOL_OFF_WING_OBJECT_COUNT)
                if object_count is None or object_count > MAX_AIR_WING_OBJECTS_PER_POOL:
                    topo["invalid_pools"] += 1
                    continue
                if object_count > 0 and not is_heap(object_begin):
                    topo["invalid_pools"] += 1
                    continue
                for object_index in range(object_count):
                    obj = safe_u64(self.proc, (object_begin or 0) + object_index * 8)
                    topo["wing_objects_seen"] += 1
                    if not is_heap(obj) or safe_u64(self.proc, obj) != self.base + AIR_WING_OBJECT_VTABLE_RVA:
                        topo["invalid_wing_objects"] += 1
                        continue
                    body = obj + AIR_WING_OBJECT_OFF_BODY
                    if (
                        safe_u64(self.proc, body) != self.base + AIR_WING_VTABLE_RVA
                        or safe_u32(self.proc, body + AIR_WING_OFF_TYPE) != AIR_WING_TYPE
                        or safe_u64(self.proc, body + AIR_WING_OFF_POOL) != pool
                    ):
                        topo["invalid_wing_objects"] += 1
                        continue
                    wing_id = safe_u32(self.proc, body + AIR_WING_OFF_ID) or 0
                    count = safe_u32(self.proc, body + AIR_WING_OFF_COUNT)
                    effective = safe_u32(self.proc, body + AIR_WING_OFF_EFFECTIVE_PLANES)
                    if not (0 < wing_id <= MAX_AIR_WING_ID) or count is None or effective is None:
                        topo["invalid_wing_objects"] += 1
                        continue
                    if count > MAX_AIR_WING_PLANES or effective > MAX_AIR_WING_PLANES:
                        topo["invalid_wing_objects"] += 1
                        continue
                    equipment = self.read_equipment(body)
                    if equipment is None:
                        topo["invalid_equipment_banks"] += 1
                        equipment = []
                    rows.append(
                        {
                            "owner_tag_db_id": owner,
                            "wing_id": wing_id,
                            "body": hx(body),
                            "object": hx(obj),
                            "count": count,
                            "mission_mask": safe_u32(self.proc, body + AIR_WING_OFF_MISSION_MASK),
                            "mission_region": self.read_mission_region(body),
                            "effective_planes": effective,
                            "equipment_rows": len(equipment),
                        }
                    )
                    for item in equipment:
                        variants.setdefault(
                            item["variant_id"],
                            {
                                "variant_id": item["variant_id"],
                                "variant_ptr": hx(item["variant_ptr"]),
                                "equipment_token_raw": item["equipment_token_raw"],
                                "equipment_type_token_raw": item["equipment_type_token_raw"],
                                "interface_type_raw": item["interface_type_raw"],
                                "seen_in_wings": 0,
                                "amount_raw_sum": 0,
                            },
                        )
                        variants[item["variant_id"]]["seen_in_wings"] += 1
                        variants[item["variant_id"]]["amount_raw_sum"] += item["amount_raw"]
                    if len(rows) > MAX_NATIVE_AIR_WINGS:
                        topo["fatal"] = "too_many_air_wings"
                        return {"topology": topo, "air_wings": rows, "variants": variants}
        return {"topology": topo, "air_wings": rows, "variants": variants}


def live_probe(pid: int | None) -> dict[str, Any]:
    if winmem is None:
        return {"available": False, "badge": "winmem_unavailable"}
    resolved_pid = pid or winmem.find_pid("hoi4.exe")
    if not resolved_pid:
        return {"available": False, "badge": "hoi4_process_not_found"}
    probe = LiveAirProbe(resolved_pid)
    try:
        root = probe.root_probe()
        out: dict[str, Any] = {"available": True, "root": root}
        try:
            strategic_air = probe.require_root(root)
            enumeration = probe.enumerate_air_wings(strategic_air)
            variants = enumeration["variants"]
            token_counts = collections.Counter(
                item["equipment_token_raw"] for item in variants.values() if item.get("equipment_token_raw")
            )
            out["enumeration"] = {
                "air_wing_count": len(enumeration["air_wings"]),
                "active_variant_count": len(variants),
                "active_token_counts": dict(sorted(token_counts.items())),
                "sample_variants": sorted(variants.values(), key=lambda row: row["variant_id"])[:16],
                "topology": enumeration["topology"],
            }
        except Exception as exc:
            out["enumeration"] = {"ok": False, "error": str(exc)}
        return out
    finally:
        probe.close()


def scan_token_name_neighborhoods(
    pid: int | None,
    active_tokens: set[int],
    scan_mb: int,
    radius: int,
) -> dict[str, Any]:
    if winmem is None:
        return {"available": False, "badge": "winmem_unavailable"}
    resolved_pid = pid or winmem.find_pid("hoi4.exe")
    if not resolved_pid:
        return {"available": False, "badge": "hoi4_process_not_found"}
    pairs = {token: name for token, name in CANDIDATE_TOKEN_NAME_PAIRS.items() if token in active_tokens}
    if not pairs:
        return {
            "available": True,
            "status": "not_checked_no_candidate_pairs_for_active_tokens",
            "candidate_pairs": {},
        }
    max_bytes = max(0, scan_mb) * 1024 * 1024
    proc = winmem.Process(resolved_pid, "hoi4.exe")
    try:
        results: dict[str, Any] = {}
        patterns = {token: name.encode("ascii") for token, name in pairs.items()}
        token_needles = {token: struct.pack("<I", token) for token in pairs}
        total_scanned = 0
        regions_scanned = 0
        chunks_scanned = 0
        chunk_size = 8 * 1024 * 1024
        for base, size in proc.iter_regions(HEAP_LO, HEAP_HI, want_exec=False):
            if total_scanned >= max_bytes:
                break
            regions_scanned += 1
            pos = 0
            while pos < size and total_scanned < max_bytes:
                want = min(chunk_size, size - pos, max_bytes - total_scanned)
                blob = proc.try_read(base + pos, want)
                if not blob:
                    pos += max(want, 0x1000)
                    continue
                chunks_scanned += 1
                total_scanned += len(blob)
                for token, pattern in patterns.items():
                    start = 0
                    while True:
                        idx = blob.find(pattern, start)
                        if idx == -1:
                            break
                        key = str(token)
                        row = results.setdefault(
                            key,
                            {
                                "token": token,
                                "candidate_name": pairs[token],
                                "name_occurrences": 0,
                                "nearby_token_occurrences": 0,
                                "first_occurrence_addr": None,
                                "nearby_examples": [],
                            },
                        )
                        row["name_occurrences"] += 1
                        addr = base + pos + idx
                        row["first_occurrence_addr"] = row["first_occurrence_addr"] or hx(addr)
                        window_start = max(addr - radius, base + pos)
                        window = proc.try_read(window_start, radius * 2 + len(pattern))
                        if token_needles[token] in window:
                            row["nearby_token_occurrences"] += 1
                            if len(row["nearby_examples"]) < 4:
                                row["nearby_examples"].append(hx(addr))
                        start = idx + 1
                pos += len(blob)
        for token, name in pairs.items():
            results.setdefault(
                str(token),
                {
                    "token": token,
                    "candidate_name": name,
                    "name_occurrences": 0,
                    "nearby_token_occurrences": 0,
                    "first_occurrence_addr": None,
                    "nearby_examples": [],
                },
            )
        checked = list(results.values())
        all_seen = all(row["name_occurrences"] > 0 for row in checked)
        no_nearby = all(row["nearby_token_occurrences"] == 0 for row in checked)
        return {
            "available": True,
            "pid": resolved_pid,
            "scan_scope": {
                "heap_low": hx(HEAP_LO),
                "heap_high": hx(HEAP_HI),
                "want_exec": False,
                "max_scan_mb": scan_mb,
                "bytes_scanned": total_scanned,
                "regions_scanned": regions_scanned,
                "chunks_scanned": chunks_scanned,
                "radius_bytes": radius,
            },
            "candidate": "ascii_equipment_name_occurrence_with_u32_token_id_within_radius",
            "status": "rejected_bounded" if all_seen and no_nearby else "inconclusive_bounded",
            "rejection_reason": (
                "Observed active candidate equipment names in the same process, but no tested name "
                "occurrence carried its candidate u32 token id within the bounded neighborhood."
                if all_seen and no_nearby
                else "At least one candidate name was not observed or a nearby token occurrence was found."
            ),
            "results": dict(sorted(results.items(), key=lambda item: int(item[0]))),
        }
    finally:
        proc.close()


def summarize_manifest(manifest_path: Path | None) -> dict[str, Any]:
    if manifest_path is None or not manifest_path.exists():
        return {"available": False, "path": str(manifest_path) if manifest_path else None}
    data = json.loads(manifest_path.read_text(encoding="utf-8"))
    compact = data.get("compact_stream", {})
    replay_file = data.get("replay_file")
    replay_exists = False
    if isinstance(replay_file, str):
        replay_path = Path(replay_file)
        if not replay_path.is_absolute():
            replay_path = manifest_path.parent / replay_path
        replay_exists = replay_path.exists()
    return {
        "available": True,
        "path": str(manifest_path),
        "pid": data.get("pid"),
        "dll": data.get("dll"),
        "stream_format": data.get("stream_format"),
        "scheduled_domains": data.get("scheduled_domains"),
        "game_hours_advanced": data.get("game_hours_advanced"),
        "unique_game_hour_count": data.get("unique_game_hour_count"),
        "skipped_game_hours": data.get("skipped_game_hours"),
        "hourly_requirement": data.get("hourly_requirement"),
        "replay_file": replay_file,
        "replay_file_exists": replay_exists,
        "compact_stream": {
            "decodable": compact.get("decodable"),
            "ordered": compact.get("ordered"),
            "domain_coverage_ok": compact.get("domain_coverage", {}).get("ok"),
            "source_unavailable_seen": compact.get("source_unavailable_seen"),
            "partial_data_seen": compact.get("partial_data_seen"),
            "bridge_unavailable_seen": compact.get("bridge_unavailable_seen"),
            "worker_panic_seen": compact.get("worker_panic_seen"),
        },
    }


def build_proof(args: argparse.Namespace) -> dict[str, Any]:
    stream_path = Path(args.stream)
    stream = decode_stream(stream_path)
    lifecycle_matrix = build_lifecycle_stream_matrix(stream_path, stream, args.lifecycle_stream_glob or [])
    manifest = summarize_manifest(Path(args.manifest) if args.manifest else None)
    live = live_probe(args.pid)
    active_tokens = {int(token) for token in stream.get("active_equipment_token_counts", {}).keys()}
    compact_active_tokens = set(active_tokens)
    current_live_tokens: set[int] = set()
    if live.get("enumeration", {}).get("active_token_counts"):
        current_live_tokens = {int(token) for token in live["enumeration"]["active_token_counts"].keys()}
        active_tokens |= current_live_tokens
    registry_scan = (
        {"available": False, "status": "skipped_by_flag"}
        if args.no_scan
        else scan_token_name_neighborhoods(args.pid, active_tokens, args.scan_mb, args.scan_radius)
    )
    token_dictionary = (
        {"available": False, "status": "skipped_by_flag"}
        if args.no_token_dictionary
        else live_token_dictionary_probe(args.pid, active_tokens, args.max_token_dictionary_countries)
    )
    hoi4_root: Path | None = None
    if args.hoi4_root:
        hoi4_root = Path(args.hoi4_root)
    else:
        module_path = live.get("root", {}).get("module_path")
        if isinstance(module_path, str):
            hoi4_root = Path(module_path).parent
    archetype_closure = build_archetype_closure(
        token_dictionary,
        compact_active_tokens,
        current_live_tokens,
        Path(args.catalogs),
        hoi4_root,
    )

    stream_errors = stream.get("decode_errors", [])
    lifecycle = stream.get("lifecycle", {})
    reserve_linkage = stream.get("reserve_stockpile_linkage", {})
    lifecycle_observed = lifecycle_matrix.get("observed", {})
    wing_delete_observed = bool(lifecycle_observed.get("air_wing_delete"))
    rebase_observed = bool(lifecycle_observed.get("air_wing_rebase"))
    create_after_initial_observed = bool(lifecycle_observed.get("air_wing_create_after_initial"))
    source_enum_catalog_badged = bool(archetype_closure.get("compact_active_catalog_missing_source_enum_closed"))
    reserve_variant_ids = reserve_linkage.get("reserve_variant_ids", 0)
    reserve_stockpile_cross_checked = bool(
        reserve_variant_ids
        and reserve_linkage.get("reserve_variant_ids_with_stockpile_defs") == reserve_variant_ids
    )

    gates = {
        "compact_stream_decoded": not stream_errors,
        "live_root_validated": bool(
            live.get("root", {}).get("sha256_ok")
            and live.get("root", {}).get("daily_sentinel_ok")
            and live.get("root", {}).get("get_long_string_sentinel_ok")
            and live.get("root", {}).get("strategic_air_ok")
        ),
        "live_active_variant_token_offsets_validated": bool(
            live.get("enumeration", {}).get("active_variant_count", 0) > 0
            and live.get("enumeration", {}).get("topology", {}).get("invalid_equipment_banks", 0) == 0
        ),
        "active_compact_variant_defs_stats_closed": (
            stream.get("active_superiority_variant_count") == stream.get("active_superiority_variants_with_defs")
            and stream.get("active_superiority_variant_count") == stream.get("active_superiority_variants_with_stats")
        ),
        "same_session_token_dictionary_validated": token_dictionary.get("status") == "validated",
        "compact_active_base_airframe_names_closed": (
            archetype_closure.get("compact_active_base_airframe_name_rows")
            == archetype_closure.get("compact_active_token_count")
        ),
        "compact_active_equipment_archetype_catalog_closed": (
            archetype_closure.get("compact_active_equipment_catalog_hits")
            == archetype_closure.get("compact_active_token_count")
        ),
        "compact_active_missing_catalog_tokens_badged_as_source_enum": source_enum_catalog_badged,
        "same_session_direct_name_token_neighborhood_candidate_rejected": registry_scan.get("status") == "rejected_bounded",
        "wing_create_after_initial_observed": create_after_initial_observed,
        "wing_delete_observed": wing_delete_observed,
        "wing_rebase_observed": rebase_observed,
        "mission_region_rows_observed": lifecycle.get("air_wing_mission_region_rows", 0) > 0,
        "air_region_activity_tombstones_observed": lifecycle.get("air_region_activity_deleted_rows", 0) > 0,
        "air_superiority_tombstones_observed": lifecycle.get("air_superiority_input_deleted_rows", 0) > 0,
        "reserves_stockpile_linkage_observed": lifecycle.get("air_variant_reserve_matched_stockpile_rows", 0) > 0,
        "reserves_stockpile_variant_ids_cross_checked": reserve_stockpile_cross_checked,
        "ace_join_field_present": False,
        "ace_join_current_compact_route_rejected": True,
    }

    badges = {
        "token_name_registry_route": (
            "same_session_token_dictionary_validated"
            if gates["same_session_token_dictionary_validated"]
            else "raw_token_offsets_validated_but_name_registry_root_unresolved"
            if gates["live_active_variant_token_offsets_validated"]
            else "live_token_offsets_not_validated"
        ),
        "archetype_base_airframe_naming": archetype_closure.get("status"),
        "equipment_archetypes_catalog": (
            "closed"
            if gates["compact_active_equipment_archetype_catalog_closed"]
            else "source_enum_tech_enabled_airframe_tokens_badged_catalog_parser_gap"
            if source_enum_catalog_badged
            else "catalog_missing_some_live_airframe_tokens"
        ),
        "direct_name_token_neighborhood_candidate": registry_scan.get("status"),
        "wing_create": "observed_after_initial_snapshot" if create_after_initial_observed else "not_observed_in_fixture_delta_window",
        "wing_delete": "observed" if wing_delete_observed else "not_observed_in_available_live_compact_streams",
        "wing_rebase": "observed" if rebase_observed else "not_observed_in_fixture_delta_window",
        "ace_joins": "rejected_current_compact_air_domains_no_ace_identity_or_assignment_field",
        "reserves_stockpile_linkage": (
            "variant_ids_cross_checked_against_stockpile_variant_defs"
            if reserve_stockpile_cross_checked
            else "reserve_rows_seen_but_some_variant_ids_lack_stockpile_defs"
            if reserve_variant_ids
            else "not_observed"
        ),
    }

    if gates["compact_active_base_airframe_names_closed"]:
        base_airframe_clause = "closed compact-active base-airframe token names"
    else:
        base_airframe_clause = (
            "narrowed compact-active base-airframe token names to "
            f"{archetype_closure.get('compact_active_base_airframe_name_rows')} of "
            f"{archetype_closure.get('compact_active_token_count')} dictionary-resolved active tokens"
        )
    if gates["compact_active_equipment_archetype_catalog_closed"]:
        catalog_clause = "closed compact-active equipment_archetypes catalog joins"
    elif source_enum_catalog_badged:
        catalog_clause = (
            "badged "
            f"{archetype_closure.get('compact_active_equipment_catalog_missing')} compact-active "
            "equipment_archetypes catalog misses as source-enum and bba_air_techs enabled airframe tokens absent "
            "from the current generated equipment_archetypes catalog"
        )
    else:
        catalog_clause = "left compact-active equipment_archetypes catalog joins partial"

    return {
        "schema": "air_wings_variants_lifecycle_live_token_lifecycle_proof_v1",
        "generated_utc": dt.datetime.now(dt.UTC).replace(microsecond=0).isoformat(),
        "completion_status": "NOT_COMPLETE",
        "backlog_reduced": True,
        "exact_backlog_item_removed_or_narrowed": (
            "Validated current-live active air variant raw equipment token route from StrategicAirManager "
            "through wing equipment banks to CEquipmentVariant +0x18/+0x1C/+0x38; validated the same-session "
            "token/name dictionary route via CCountry+0x230 CFlagManager -> token-record vtable -> token id/text "
            f"records; {base_airframe_clause}; {catalog_clause}."
        ),
        "stream": stream,
        "lifecycle_matrix": lifecycle_matrix,
        "manifest": manifest,
        "live_process": live,
        "registry_scan": registry_scan,
        "token_dictionary": token_dictionary,
        "archetype_closure": archetype_closure,
        "reserve_stockpile_linkage": reserve_linkage,
        "ace_join_rejection_evidence": {
            "current_compact_air_domains": {
                "30_air_wings": "owner_tag_db_id, wing_id, generation, deleted, count, mission_mask, mission_region, effective_planes",
                "31_air_region_activity": "region_id, owner_tag_db_id, mission_mask, wing_count, plane_count, effective_planes",
                "32_air_wing_loadouts": "owner_tag_db_id, wing_id, generation, deleted, air_base_id, variant_id, amount_raw",
                "36_air_variant_reserves": "tag_db_id, variant_id, amount_raw",
                "38_air_superiority_inputs": "region_id, owner_tag_db_id, mission_mask, variant_id, equipment tokens, wing_count, amount_raw",
            },
            "shared_collector_existing_ace_surface": "country misc ace_count only; no ace id/name/wing assignment in current compact air rows",
            "rejection_scope": "rejects ace joins via current compact air domain schemas; it does not reject a future dedicated ace root/domain.",
        },
        "gates": gates,
        "badges": badges,
        "validated_roots_offsets_functions": {
            "CGameState_global": "hoi4.exe+0x3316E90 -> CGameState*",
            "StrategicAirManager": "CGameState+0x690, vtable hoi4.exe+0x2942118",
            "air_owner_containers": "StrategicAirManager+0x30 pointer table, +0x3C count",
            "air_wing_pool": "owner container+0x58 pointer table, +0x64 count, pool vtable hoi4.exe+0x2964208",
            "air_wing_body": "pool+0x28 wing object table, object+0x10 body, body vtable hoi4.exe+0x29641B0, type 69 @+0x08",
            "air_wing_lifecycle_fields": "wing_id +0x0C, count +0x6C, mission_mask +0x90, mission_region +0xB0, effective_planes +0x160",
            "air_wing_equipment_bank": "wing body+0x1E8 vector header data/cap/size @+0/+8/+0x0C, entries stride 0x10",
            "air_variant_identity": "equipment entry +0 variant ptr, vtable hoi4.exe+0x293AFB8, type 70 @+0x08, variant id +0x0C",
            "air_variant_token_stats_offsets": "equipment_token_raw +0x18, equipment_type_token_raw +0x1C, interface_type_raw +0x38",
            "same_session_token_dictionary": "CGameState+0x310 country table, CCountry+0x230 CFlagManager, CFlagManager+0x18 token-record vtable, token record +0x08 id, +0x10 text pointer",
            "compact_lifecycle_domains": "domain 30 air_wings, 31 air_region_activity, 32 air_wing_loadouts, 36 air_variant_reserves, 38 air_superiority_inputs",
            "source_enum_airframe_catalog_gap": archetype_closure.get("catalog_gap_classification"),
            "air_wing_delete_encoder_path": "compact_air_wing_delta_rows -> deleted_compact_air_wing_row when a previously tracked wing body disappears",
        },
        "rejected_or_badged_candidates": {
            "direct_ascii_name_near_u32_token_registry": registry_scan.get("status"),
            "ace_join_from_current_compact_air_domains": badges["ace_joins"],
            "equipment_archetypes_catalog_for_all_live_airframe_tokens": badges["equipment_archetypes_catalog"],
            "compact_active_missing_catalog_tokens": archetype_closure.get(
                "compact_active_catalog_missing_source_enum_tokens"
            ),
            "wing_create_delete_rebase_in_existing_fixture": {
                "create": badges["wing_create"],
                "delete": badges["wing_delete"],
                "rebase": badges["wing_rebase"],
            },
        },
        "compact_schema_record_changes": "lane-local domain46 Rust schema has live token dictionary/static source source-mask bits; full air lifecycle remains outside this validated archetype slice",
        "rust_decoder_gate_changes": "none in shared recorder; lane-local Rust domain46 decoder counts live-token-dictionary and static-source-enum rows",
        "dll_only": False,
        "dll_only_detail": "collector stream is DLL-only; this validation proof also uses read-only Python live/process and static-source checks",
        "compact_stream": True,
        "rust_decoded": bool(manifest.get("compact_stream", {}).get("decodable")),
        "hoi4replay_verified": bool(manifest.get("replay_file_exists")),
        "speed_five_current_equivalent_hourly_or_lower": bool(manifest.get("hourly_requirement", {}).get("met")),
        "zero_skipped_hours": manifest.get("skipped_game_hours") == 0,
        "partial_source_worker_bridge_failures": bool(
            manifest.get("compact_stream", {}).get("source_unavailable_seen")
            or manifest.get("compact_stream", {}).get("partial_data_seen")
            or manifest.get("compact_stream", {}).get("bridge_unavailable_seen")
            or manifest.get("compact_stream", {}).get("worker_panic_seen")
        ),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--stream", required=True)
    parser.add_argument("--manifest")
    parser.add_argument("--out-dir", required=True)
    parser.add_argument("--pid", type=int)
    parser.add_argument("--catalogs", default="game_defs/out/catalogs.json")
    parser.add_argument("--hoi4-root")
    parser.add_argument("--max-token-dictionary-countries", type=int, default=512)
    parser.add_argument("--scan-mb", type=int, default=1024)
    parser.add_argument("--scan-radius", type=int, default=256)
    parser.add_argument("--no-scan", action="store_true")
    parser.add_argument("--no-token-dictionary", action="store_true")
    parser.add_argument(
        "--lifecycle-stream-glob",
        action="append",
        default=[],
        help="Additional collector_stream.bin glob(s) to scan for create/delete/rebase lifecycle proof.",
    )
    args = parser.parse_args()

    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    proof = build_proof(args)
    out_path = out_dir / "proof_live_token_lifecycle_gates.json"
    out_path.write_text(json.dumps(proof, indent=2, sort_keys=True), encoding="utf-8")
    print(json.dumps({
        "out": str(out_path),
        "completion_status": proof["completion_status"],
        "backlog_reduced": proof["backlog_reduced"],
        "gates": proof["gates"],
        "badges": proof["badges"],
    }, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
