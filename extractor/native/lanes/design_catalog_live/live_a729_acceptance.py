#!/usr/bin/env python3
"""Read-only live acceptance for the exact a729 A27 catalog producer.

The complete catalog is discovered from one read-only shared-heap vtable sweep.
Country template vectors supply template ownership/source order.  Production-
manager A/B vectors are joined only as optional equipment-designer membership
and order occurrences: they are *not* complete catalog roots (the retained
same-tick process contains 5,281 variants, while A/B reaches 5,267).

The save oracle is loaded only after the live snapshot is frozen and is never
used to populate a live field, textual country tag, lifecycle bit, definition,
or source occurrence.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import struct
import sys
from pathlib import Path

LANE = Path(__file__).resolve().parent
REPO = LANE.parents[3]
sys.path.insert(0, str(LANE))
sys.path.insert(0, str(REPO / "extractor" / "memory_reader"))

import winmem  # noqa: E402

from a729_layout import (  # noqa: E402
    COUNTRY_PROD_MANAGER,
    COUNTRY_TAG,
    COUNTRY_TEMPLATES_COUNT,
    COUNTRY_TEMPLATES_DATA,
    MANAGER_LIST_A_COUNT,
    MANAGER_LIST_A_DATA,
    MANAGER_LIST_B_COUNT,
    MANAGER_LIST_B_DATA,
    OBJECT_ID,
    OBJECT_TYPE,
    PROGRAM_SHA256,
    TEMPLATE_VTABLE,
    TYPE_TEMPLATE,
    TYPE_VARIANT,
    VARIANT_CREATOR,
    VARIANT_OBSOLETE,
    VARIANT_ORIGIN,
    VARIANT_VTABLE,
)

CGAMESTATE_RVA = 0x3316E90
COUNTRY_DATABASE_RVA = 0x33189C8
COUNTRY_VTABLE_RVA = 0x27D0E90
GS_COUNTRY_TABLE = 0x310
GS_COUNTRY_COUNT = 0x31C
GS_CURRENT_DATE = 0x468
DATE_EPOCH_CONST = 43_799_999
# A COUNT budget, not an identity ceiling. `2048` was a guess at the
# vanilla-plus-DLC country count (vanilla defines 439 tags), and this file is a
# PRODUCTION producer that `recorder_host` launches and waits 30 s on for
# readiness -- so on an overhaul above the guess it raised, and recording never
# started at all.
#
# 65_536 is the tree's country-table budget: a slot is one 8-byte pointer, so the
# registry bulk read below is 512 KB at the ceiling, and it is 149x the vanilla
# count. Both use sites bound a count before a read or a loop, and neither is an
# id, so neither takes the structural id ceiling.
MAX_COUNTRIES = 65536
MAX_VECTOR = 100_000
HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000
VARIANT_VTABLE_RVA = VARIANT_VTABLE - 0x140000000
TEMPLATE_VTABLE_RVA = TEMPLATE_VTABLE - 0x140000000
SUBUNIT_DEF_VTABLE_RVA = 0x2773BA8
EQUIPMENT_DEF_VTABLE_RVA = 0x29218A0
MODULE_DEF_VTABLE_RVA = 0x2921850
MODULE_CONTAINER_VTABLE_RVA = 0x306D170
UPGRADE_CONTAINER_VTABLE_RVA = 0x27AB628

VARIANT_DISPLAY_NAME = 0x28
VARIANT_VARIANT_NAME = 0x48
VARIANT_UPGRADE_CONTAINER = 0x68
VARIANT_UPGRADE_DATA = 0x70
VARIANT_UPGRADE_CAP = 0x78
VARIANT_UPGRADE_COUNT = 0x7C
VARIANT_MODULE_CONTAINER = 0xB0
VARIANT_MODULE_DATA = 0xB8
VARIANT_MODULE_CAP = 0xC0
VARIANT_MODULE_COUNT = 0xC4
VARIANT_TYPE_PTR = 0x3F0

TEMPLATE_NAME = 0x20
TEMPLATE_GRID_DIMENSIONS = 0x60
TEMPLATE_REGIMENT_DATA = 0x68
TEMPLATE_REGIMENT_COUNT = 0x74
TEMPLATE_SUPPORT_DATA = 0x88
TEMPLATE_SUPPORT_COUNT = 0x94
TEMPLATE_REGIMENTAL_SUPPORT_DATA = 0xA8
TEMPLATE_REGIMENTAL_SUPPORT_COUNT = 0xB4
TEMPLATE_PRIORITY = 0x1A4
TEMPLATE_LOCK_REGIME_SNAPSHOT = 0x1CC
TEMPLATE_IS_LOCKED = 0x235
TEMPLATE_OBSOLETE = 0x236

CACHED_STAT_FIELDS = (
    ("speed_cache", 0x270, 100_000),
    ("range", 0x278, 1_000),
    ("air_attack", 0x280, 100_000),
    ("agility", 0x288, 100_000),
    ("max_speed", 0x2F0, 100_000),
    ("reliability", 0x308, 100_000),
    ("air_defence", 0x318, 100_000),
    ("thrust", 0x320, 100_000),
    ("fuel_consumption", 0x328, 100_000),
    ("build_cost_ic", 0x368, 100_000),
)


def _hx(value: int) -> str:
    return f"0x{value:X}"


def _heap(value: int) -> bool:
    return 0x10000000000 <= value < 0x7FF000000000 and value % 8 == 0


def _stable_vector(p, owner: int, data_off: int, count_off: int, label: str) -> list[int]:
    d1 = p.u64(owner + data_off)
    n1 = p.u32(owner + count_off)
    if n1 > MAX_VECTOR:
        raise RuntimeError(f"{label}: count {n1} exceeds bound")
    if n1 == 0:
        if (d1, n1) != (p.u64(owner + data_off), p.u32(owner + count_off)):
            raise RuntimeError(f"{label}: torn empty header")
        return []
    if not _heap(d1):
        raise RuntimeError(f"{label}: invalid data pointer {_hx(d1)}")
    raw = p.read(d1, n1 * 8)
    if (d1, n1) != (p.u64(owner + data_off), p.u32(owner + count_off)):
        raise RuntimeError(f"{label}: torn vector")
    return list(struct.unpack(f"<{n1}Q", raw))


def _i32(p, address: int) -> int:
    return struct.unpack("<i", p.read(address, 4))[0]


def _msvc_bytes(p, address: int, label: str, max_len: int = 512) -> bytes:
    """Read one exact MSVC string, including valid empty strings.

    The returned bytes are the authority.  UTF-8 decoding is a separate,
    fail-closed step so non-ASCII design names are not silently discarded.
    """
    header = p.read(address, 0x20)
    size = struct.unpack_from("<Q", header, 0x10)[0]
    capacity = struct.unpack_from("<Q", header, 0x18)[0]
    if size > max_len or capacity < size or capacity > 1 << 20:
        raise RuntimeError(f"{label}: invalid MSVC string header")
    if size == 0:
        return b""
    if capacity < 16:
        return header[:size]
    pointer = struct.unpack_from("<Q", header)[0]
    if not _heap(pointer):
        raise RuntimeError(f"{label}: invalid MSVC string pointer")
    return p.read(pointer, size)


def _msvc_utf8(p, address: int, label: str, max_len: int = 512) -> tuple[str | None, str]:
    raw = _msvc_bytes(p, address, label, max_len=max_len)
    try:
        decoded = raw.decode("utf-8") if raw else None
    except UnicodeDecodeError as error:
        raise RuntimeError(f"{label}: non-UTF-8 engine string") from error
    return decoded, raw.hex()


def _enum_vtable(p, vtable: int):
    """Yield every aligned shared-heap object starting with ``vtable``."""
    needle = struct.pack("<Q", vtable)
    chunk_size = 32 << 20
    overlap = 16
    for base, size in p.iter_regions(HEAP_LO, HEAP_HI, want_exec=False):
        offset = 0
        while offset < size:
            wanted = min(chunk_size, size - offset)
            blob = p.try_read(base + offset, wanted)
            if not blob:
                offset += max(wanted, 0x1000)
                continue
            found = blob.find(needle)
            while found >= 0:
                address = base + offset + found
                if address % 8 == 0:
                    yield address
                found = blob.find(needle, found + 1)
            if len(blob) < wanted:
                offset += max(len(blob), 0x1000)
            elif offset + len(blob) >= size:
                offset += len(blob)
            else:
                offset += len(blob) - overlap


def _catalog_objects(p, vtable_rva: int, object_type: int, label: str) -> list[tuple[int, int]]:
    """Freeze one exact-ID catalog from a complete vtable sweep."""
    vtable = p.module_base + vtable_rva
    by_id: dict[int, int] = {}
    for pointer in _enum_vtable(p, vtable):
        if p.u32(pointer + OBJECT_TYPE) != object_type:
            continue
        object_id = p.u32(pointer + OBJECT_ID)
        if object_id <= 0:
            raise RuntimeError(f"{label}: zero stable ID")
        prior = by_id.setdefault(object_id, pointer)
        if prior != pointer:
            raise RuntimeError(f"{label}: stable ID {object_id} has multiple live pointers")
    if not by_id:
        raise RuntimeError(f"{label}: empty heap catalog")
    # Heap address is deliberately not replay/UI order.  It is retained only
    # as a deterministic scan epoch ordinal and same-snapshot identity.
    return [(object_id, by_id[object_id]) for object_id in sorted(by_id, key=lambda value: by_id[value])]


def _inline_tag(p, address: int) -> str | None:
    raw = p.try_read(address, 4)
    if len(raw) != 4:
        return None
    raw = raw.split(b"\0", 1)[0]
    try:
        value = raw.decode("ascii")
    except UnicodeDecodeError:
        return None
    return value if len(value) == 3 and value.isprintable() else None


def _tag_map(p) -> dict[int, str]:
    database = p.u64(p.module_base + COUNTRY_DATABASE_RVA)
    if not _heap(database):
        raise RuntimeError("country database root is invalid")
    table = p.u64(database)
    # Packed header: low u32 is capacity, high u32 is logical size.  Entry zero
    # is the engine's --- sentinel.  Dynamic tags are allocated immediately
    # after this immutable table as D01, D02, ... in raw-ID order.
    capacity = p.u32(database + 8)
    count = p.u32(database + 0x0C)
    if not _heap(table) or not (1 <= count <= MAX_COUNTRIES):
        raise RuntimeError("country tag table is invalid")
    if capacity < count:
        raise RuntimeError("country tag table capacity is below count")
    out = {}
    for tag_id in range(count):
        tag = _inline_tag(p, table + tag_id * 0x20)
        if tag:
            out[tag_id] = tag
    if len(out) != count:
        raise RuntimeError("country tag table has an undecodable row")
    out["__dynamic_base__"] = count
    return out


def _key(row: dict) -> str:
    return f"{int(row['id'])}:{int(row['type'])}"


def _bind_tag(mapping: dict[int, str], raw_id: int, tag: str, source: str) -> None:
    if raw_id == 0 and tag == "---":
        return
    prior = mapping.setdefault(raw_id, tag)
    if prior != tag:
        raise RuntimeError(f"tag-db id {raw_id} maps to both {prior} and {tag} ({source})")


def _resolve_tag(tags: dict, raw_id: int) -> str:
    value = tags.get(raw_id)
    if value:
        return value
    dynamic_base = int(tags["__dynamic_base__"])
    dynamic_ordinal = raw_id - dynamic_base + 1
    if 1 <= dynamic_ordinal <= 99:
        return f"D{dynamic_ordinal:02d}"
    raise RuntimeError(f"unresolved country tag id {raw_id}")


def _variant_definition(p, pointer: int, variant_id: int) -> dict:
    equipment_handle = p.u32(pointer + 0x18)
    equipment_def = p.u64(pointer + VARIANT_TYPE_PTR)
    if not _heap(equipment_def):
        raise RuntimeError(f"variant {variant_id}: invalid equipment-definition pointer")
    if p.u64(equipment_def) != p.module_base + EQUIPMENT_DEF_VTABLE_RVA:
        raise RuntimeError(f"variant {variant_id}: equipment-definition vtable mismatch")
    if p.u32(equipment_def + 0x08) != equipment_handle:
        raise RuntimeError(f"variant {variant_id}: equipment handle/pointer disagreement")
    archetype, archetype_hex = _msvc_utf8(
        p, equipment_def + 0x18, f"variant {variant_id} archetype", max_len=160
    )
    if not archetype:
        raise RuntimeError(f"variant {variant_id}: empty archetype token")
    display_name, display_name_hex = _msvc_utf8(
        p, pointer + VARIANT_DISPLAY_NAME, f"variant {variant_id} display name", max_len=256
    )
    variant_name, variant_name_hex = _msvc_utf8(
        p, pointer + VARIANT_VARIANT_NAME, f"variant {variant_id} variant name", max_len=256
    )

    if p.u64(pointer + VARIANT_MODULE_CONTAINER) != p.module_base + MODULE_CONTAINER_VTABLE_RVA:
        raise RuntimeError(f"variant {variant_id}: module-container vtable mismatch")
    module_data = p.u64(pointer + VARIANT_MODULE_DATA)
    module_capacity = p.u32(pointer + VARIANT_MODULE_CAP)
    module_count = p.u32(pointer + VARIANT_MODULE_COUNT)
    if module_capacity < module_count or module_count > 256:
        raise RuntimeError(f"variant {variant_id}: invalid module header")
    modules = []
    if module_count:
        if not _heap(module_data):
            raise RuntimeError(f"variant {variant_id}: invalid module data pointer")
        raw = p.read(module_data, module_count * 0x10)
        for ordinal in range(module_count):
            offset = ordinal * 0x10
            slot_handle = struct.unpack_from("<I", raw, offset)[0]
            module_def = struct.unpack_from("<Q", raw, offset + 0x08)[0]
            if not _heap(module_def) or p.u64(module_def) != p.module_base + MODULE_DEF_VTABLE_RVA:
                raise RuntimeError(f"variant {variant_id}: module {ordinal} identity mismatch")
            module_token, module_token_hex = _msvc_utf8(
                p, module_def + 0x18, f"variant {variant_id} module {ordinal}", max_len=160
            )
            if not module_token:
                raise RuntimeError(f"variant {variant_id}: module {ordinal} has empty token")
            modules.append(
                {
                    "source_ordinal": ordinal,
                    "slot_handle": slot_handle,
                    "module_definition_handle": p.u32(module_def + 0x08),
                    "module_token": module_token,
                    "module_token_utf8_hex": module_token_hex,
                }
            )

    if p.u64(pointer + VARIANT_UPGRADE_CONTAINER) != p.module_base + UPGRADE_CONTAINER_VTABLE_RVA:
        raise RuntimeError(f"variant {variant_id}: upgrade-container vtable mismatch")
    upgrade_data = p.u64(pointer + VARIANT_UPGRADE_DATA)
    upgrade_capacity = p.u32(pointer + VARIANT_UPGRADE_CAP)
    upgrade_count = p.u32(pointer + VARIANT_UPGRADE_COUNT)
    if upgrade_capacity < upgrade_count or upgrade_count > 256:
        raise RuntimeError(f"variant {variant_id}: invalid upgrade header")
    upgrades = []
    if upgrade_count:
        if not _heap(upgrade_data):
            raise RuntimeError(f"variant {variant_id}: invalid upgrade data pointer")
        raw = p.read(upgrade_data, upgrade_count * 0x10)
        for ordinal in range(upgrade_count):
            offset = ordinal * 0x10
            level_raw = struct.unpack_from("<Q", raw, offset + 0x08)[0]
            upgrades.append(
                {
                    "source_ordinal": ordinal,
                    "upgrade_handle": struct.unpack_from("<I", raw, offset)[0],
                    "level": level_raw & 0xFF,
                    "level_raw_u64": level_raw,
                }
            )

    cached_stats = [
        {
            "source_ordinal": ordinal,
            "stat_id": stat_id,
            "raw_i32": _i32(p, pointer + offset),
            "scale": scale,
        }
        for ordinal, (stat_id, offset, scale) in enumerate(CACHED_STAT_FIELDS)
    ]
    return {
        "equipment_definition_pointer": _hx(equipment_def),
        "equipment_token_raw": equipment_handle,
        "archetype_token": archetype,
        "archetype_token_utf8_hex": archetype_hex,
        "display_name": display_name,
        "display_name_utf8_hex": display_name_hex,
        "variant_name": variant_name,
        "variant_name_utf8_hex": variant_name_hex,
        "modules_complete": True,
        "modules": modules,
        "upgrades_complete": True,
        "upgrades": upgrades,
        "cached_stats_complete": True,
        "cached_stats": cached_stats,
    }


def _subunit_rows(p, data: int, count: int, rows_per_column: int, label: str) -> tuple[list[dict], list[int]]:
    if count > 64:
        raise RuntimeError(f"{label}: count exceeds bound")
    if count == 0:
        return [], []
    if not _heap(data):
        raise RuntimeError(f"{label}: invalid data pointer")
    raw = p.read(data, count * 8)
    rows = []
    placeholders = []
    expected_vtable = p.module_base + SUBUNIT_DEF_VTABLE_RVA
    for ordinal in range(count):
        definition = struct.unpack_from("<Q", raw, ordinal * 8)[0]
        if not _heap(definition) or p.u64(definition) != expected_vtable:
            placeholders.append(ordinal)
            continue
        token, token_hex = _msvc_utf8(p, definition + 0x18, f"{label}/{ordinal}", max_len=160)
        if not token:
            raise RuntimeError(f"{label}/{ordinal}: empty subunit token")
        rows.append(
            {
                "source_ordinal": ordinal,
                "x": ordinal // rows_per_column,
                "y": ordinal % rows_per_column,
                "subunit_definition_pointer": _hx(definition),
                "subunit_definition_handle": p.u32(definition + 0x08),
                "type": token,
                "type_utf8_hex": token_hex,
            }
        )
    return rows, placeholders


def _template_definition(p, pointer: int, template_id: int) -> dict:
    name, name_hex = _msvc_utf8(p, pointer + TEMPLATE_NAME, f"template {template_id} name", max_len=256)
    dimensions = p.read(pointer + TEMPLATE_GRID_DIMENSIONS, 4)
    columns, rows_per_column = struct.unpack("<HH", dimensions)
    if not (1 <= columns <= 16 and 1 <= rows_per_column <= 16):
        raise RuntimeError(f"template {template_id}: invalid regiment-grid dimensions")
    regiments, regiment_placeholders = _subunit_rows(
        p,
        p.u64(pointer + TEMPLATE_REGIMENT_DATA),
        p.u32(pointer + TEMPLATE_REGIMENT_COUNT),
        rows_per_column,
        f"template {template_id} regiments",
    )
    support, support_placeholders = _subunit_rows(
        p,
        p.u64(pointer + TEMPLATE_SUPPORT_DATA),
        p.u32(pointer + TEMPLATE_SUPPORT_COUNT),
        max(1, p.u32(pointer + TEMPLATE_SUPPORT_COUNT)),
        f"template {template_id} support",
    )
    # One pointer slot per regiment column.  Empty columns contain the engine's
    # placeholder object; populated slots are exact subunit definitions at y=0.
    regimental_support, regimental_support_placeholders = _subunit_rows(
        p,
        p.u64(pointer + TEMPLATE_REGIMENTAL_SUPPORT_DATA),
        p.u32(pointer + TEMPLATE_REGIMENTAL_SUPPORT_COUNT),
        1,
        f"template {template_id} regimental support",
    )
    return {
        "name": name,
        "name_utf8_hex": name_hex,
        "regiment_grid_columns": columns,
        "regiment_grid_rows": rows_per_column,
        "regiment_pointer_count": p.u32(pointer + TEMPLATE_REGIMENT_COUNT),
        "regiment_placeholder_ordinals": regiment_placeholders,
        "regiments": regiments,
        "support_pointer_count": p.u32(pointer + TEMPLATE_SUPPORT_COUNT),
        "support_placeholder_ordinals": support_placeholders,
        "support": support,
        "regimental_support_pointer_count": p.u32(pointer + TEMPLATE_REGIMENTAL_SUPPORT_COUNT),
        "regimental_support_placeholder_ordinals": regimental_support_placeholders,
        "regimental_support": regimental_support,
        "priority": p.u32(pointer + TEMPLATE_PRIORITY),
        "lock_regime_snapshot_raw": p.u8(pointer + TEMPLATE_LOCK_REGIME_SNAPSHOT),
        "is_locked": bool(p.u8(pointer + TEMPLATE_IS_LOCKED)),
        "obsolete": bool(p.u8(pointer + TEMPLATE_OBSOLETE) & 1),
    }


def capture(p) -> dict:
    fingerprint = p.fingerprint()
    if fingerprint.get("sha256") != PROGRAM_SHA256:
        raise RuntimeError("live executable does not match pinned a729 SHA-256")
    game_state = p.u64(p.module_base + CGAMESTATE_RVA)
    if not _heap(game_state):
        raise RuntimeError("CGameState root is invalid")
    current_date_raw = p.u32(game_state + GS_CURRENT_DATE)
    capture_hour_index = current_date_raw - DATE_EPOCH_CONST
    if capture_hour_index < 0:
        raise RuntimeError("CGameState date is outside the portable replay epoch")
    table = p.u64(game_state + GS_COUNTRY_TABLE)
    count = p.u32(game_state + GS_COUNTRY_COUNT)
    if not _heap(table) or not (1 <= count <= MAX_COUNTRIES):
        raise RuntimeError("country registry is invalid")
    raw = p.read(table, count * 8)
    if (table, count) != (p.u64(game_state + GS_COUNTRY_TABLE), p.u32(game_state + GS_COUNTRY_COUNT)):
        raise RuntimeError("country registry is torn")
    pointers = list(struct.unpack(f"<{count}Q", raw))
    if len(set(pointers)) != len(pointers):
        raise RuntimeError("country registry contains duplicate pointers")
    tags = _tag_map(p)
    country_vtable = p.module_base + COUNTRY_VTABLE_RVA
    countries: list[tuple[int, str, int]] = []
    for index, country in enumerate(pointers):
        if not _heap(country) or p.u64(country) != country_vtable:
            raise RuntimeError(f"country {index}: identity gate failed")
        raw_tag = p.u32(country + COUNTRY_TAG)
        tag = _resolve_tag(tags, raw_tag)
        countries.append((index, tag, country))

    variant_catalog = _catalog_objects(p, VARIANT_VTABLE_RVA, TYPE_VARIANT, "equipment variants")
    template_catalog = _catalog_objects(p, TEMPLATE_VTABLE_RVA, TYPE_TEMPLATE, "division templates")
    variant_by_pointer = {pointer: variant_id for variant_id, pointer in variant_catalog}
    template_by_pointer = {pointer: template_id for template_id, pointer in template_catalog}
    occurrences: dict[int, list[dict]] = {pointer: [] for _, pointer in variant_catalog}
    first_ui_discovery: dict[int, int] = {}
    template_sources: dict[int, dict] = {}
    source_vectors: dict[tuple[int, str], list[int]] = {}
    template_vectors: dict[int, list[int]] = {}
    ui_discovery_ordinal = 0
    for registry_index, tag, country in countries:
        manager = p.u64(country + COUNTRY_PROD_MANAGER)
        if not _heap(manager):
            raise RuntimeError(f"{tag}: production manager invalid")
        for source_list, data_off, count_off in (
            ("A", MANAGER_LIST_A_DATA, MANAGER_LIST_A_COUNT),
            ("B", MANAGER_LIST_B_DATA, MANAGER_LIST_B_COUNT),
        ):
            source_vector = _stable_vector(p, manager, data_off, count_off, f"{tag}/{source_list}")
            source_vectors[(registry_index, source_list)] = source_vector
            for ordinal, ptr in enumerate(source_vector):
                if (
                    not _heap(ptr)
                    or p.u64(ptr) != p.module_base + VARIANT_VTABLE_RVA
                    or p.u32(ptr + OBJECT_TYPE) != TYPE_VARIANT
                ):
                    raise RuntimeError(f"{tag}/{source_list}/{ordinal}: variant identity failed")
                design_id = p.u32(ptr + OBJECT_ID)
                if variant_by_pointer.get(ptr) != design_id:
                    raise RuntimeError(f"{tag}/{source_list}/{ordinal}: A/B pointer absent from full catalog")
                occurrence = {
                    "source_country_tag": tag,
                    "source_country_registry_index": registry_index,
                    "source_list": source_list,
                    "source_list_ordinal": ordinal,
                    "pointer_identity": _hx(ptr),
                }
                occurrences[ptr].append(occurrence)
                if ptr not in first_ui_discovery:
                    first_ui_discovery[ptr] = ui_discovery_ordinal
                    ui_discovery_ordinal += 1

        template_vector = _stable_vector(
            p, country, COUNTRY_TEMPLATES_DATA, COUNTRY_TEMPLATES_COUNT, f"{tag}/templates"
        )
        template_vectors[registry_index] = template_vector
        for ordinal, ptr in enumerate(template_vector):
            if (
                not _heap(ptr)
                or p.u64(ptr) != p.module_base + TEMPLATE_VTABLE_RVA
                or p.u32(ptr + OBJECT_TYPE) != TYPE_TEMPLATE
            ):
                raise RuntimeError(f"{tag}/templates/{ordinal}: template identity failed")
            design_id = p.u32(ptr + OBJECT_ID)
            if template_by_pointer.get(ptr) != design_id:
                raise RuntimeError(f"{tag}/templates/{ordinal}: pointer absent from full catalog")
            if ptr in template_sources:
                raise RuntimeError(f"template {design_id}: duplicate country-vector membership")
            template_sources[ptr] = {
                "owner_tag": tag,
                "source_country_tag": tag,
                "source_country_registry_index": registry_index,
                "country_vector_ordinal": ordinal,
            }

    if set(template_sources) != set(template_by_pointer):
        raise RuntimeError("country template vectors do not cover the complete heap template catalog")

    variants = []
    for catalog_ordinal, (design_id, ptr) in enumerate(variant_catalog):
        creator_raw = p.u32(ptr + VARIANT_CREATOR)
        origin_raw = p.u32(ptr + VARIANT_ORIGIN)
        creator = _resolve_tag(tags, creator_raw)
        origin = None if origin_raw == 0 else _resolve_tag(tags, origin_raw)
        definition = _variant_definition(p, ptr, design_id)
        variants.append(
            {
                "id": design_id,
                "type": TYPE_VARIANT,
                "owner_tag": creator,
                "creator_tag": creator,
                "origin_override_tag": origin,
                "selected_country_tag": origin or creator,
                "obsolete": bool(p.u8(ptr + VARIANT_OBSOLETE)),
                "definition_complete": True,
                "catalog_scan_ordinal": catalog_ordinal,
                "first_ui_discovery_ordinal": first_ui_discovery.get(ptr),
                "source_occurrences": occurrences[ptr],
                "pointer_identity": _hx(ptr),
                "definition": definition,
            }
        )

    templates = []
    for catalog_ordinal, (design_id, ptr) in enumerate(template_catalog):
        source = template_sources[ptr]
        definition = _template_definition(p, ptr, design_id)
        templates.append(
            {
                "id": design_id,
                "type": TYPE_TEMPLATE,
                **source,
                "obsolete": definition["obsolete"],
                "definition_complete": True,
                "catalog_scan_ordinal": catalog_ordinal,
                "pointer_identity": _hx(ptr),
                "definition": definition,
            }
        )

    # A stable vector header alone does not make a multi-second heap sweep one
    # coherent observation. Re-read every membership root and every exact
    # semantic field after the first projection, then repeat both vtable
    # censuses. Any create/edit/rename/obsolete/delete or owner/order mutation
    # during the sweep rejects the entire capture.
    if p.u32(game_state + GS_CURRENT_DATE) != current_date_raw:
        raise RuntimeError("A27 catalog crossed a game-hour boundary")
    if tags != _tag_map(p):
        raise RuntimeError("country tag table changed during A27 capture")
    if (table, count) != (
        p.u64(game_state + GS_COUNTRY_TABLE),
        p.u32(game_state + GS_COUNTRY_COUNT),
    ):
        raise RuntimeError("country registry header changed during A27 capture")
    if p.read(table, count * 8) != raw:
        raise RuntimeError("country registry rows changed during A27 capture")
    for registry_index, tag, country in countries:
        manager = p.u64(country + COUNTRY_PROD_MANAGER)
        if not _heap(manager):
            raise RuntimeError(f"{tag}: production manager changed during A27 capture")
        for source_list, data_off, count_off in (
            ("A", MANAGER_LIST_A_DATA, MANAGER_LIST_A_COUNT),
            ("B", MANAGER_LIST_B_DATA, MANAGER_LIST_B_COUNT),
        ):
            if _stable_vector(
                p, manager, data_off, count_off, f"{tag}/{source_list}/seal"
            ) != source_vectors[(registry_index, source_list)]:
                raise RuntimeError(f"{tag}/{source_list}: membership/order changed during A27 capture")
        if _stable_vector(
            p,
            country,
            COUNTRY_TEMPLATES_DATA,
            COUNTRY_TEMPLATES_COUNT,
            f"{tag}/templates/seal",
        ) != template_vectors[registry_index]:
            raise RuntimeError(f"{tag}/templates: membership/order changed during A27 capture")
    for row, (design_id, ptr) in zip(variants, variant_catalog, strict=True):
        if (
            _resolve_tag(tags, p.u32(ptr + VARIANT_CREATOR)) != row["creator_tag"]
            or (
                None
                if p.u32(ptr + VARIANT_ORIGIN) == 0
                else _resolve_tag(tags, p.u32(ptr + VARIANT_ORIGIN))
            )
            != row["origin_override_tag"]
            or bool(p.u8(ptr + VARIANT_OBSOLETE)) != row["obsolete"]
            or _variant_definition(p, ptr, design_id) != row["definition"]
        ):
            raise RuntimeError(f"variant {design_id}: definition/lifecycle changed during A27 capture")
    for row, (design_id, ptr) in zip(templates, template_catalog, strict=True):
        if _template_definition(p, ptr, design_id) != row["definition"]:
            raise RuntimeError(f"template {design_id}: definition/lifecycle changed during A27 capture")
    if _catalog_objects(p, VARIANT_VTABLE_RVA, TYPE_VARIANT, "equipment variants/seal") != variant_catalog:
        raise RuntimeError("equipment-variant heap membership changed during A27 capture")
    if _catalog_objects(p, TEMPLATE_VTABLE_RVA, TYPE_TEMPLATE, "division templates/seal") != template_catalog:
        raise RuntimeError("division-template heap membership changed during A27 capture")
    if p.u32(game_state + GS_CURRENT_DATE) != current_date_raw:
        raise RuntimeError("A27 catalog crossed a game-hour boundary while sealing")
    return {
        "schema": "extractor.a27.live_heap_catalog.a729.v2",
        "authority": {
            "program": "hoi4.exe 1.19.2.0.a729",
            "program_sha256": PROGRAM_SHA256,
            "pid": p.pid,
            "module_base": _hx(p.module_base),
            "game_state": _hx(game_state),
            "current_date_raw": current_date_raw,
            "capture_hour_index": capture_hour_index,
            "coherence": "double_read_all_roots_fields_and_membership",
        },
        "full_replacement": True,
        "completeness": {
            "membership": True,
            "owner": True,
            "definitions": True,
            "lifecycle": True,
            "ui_occurrences": True,
        },
        "catalog_roots": {
            "variants": "shared_heap/vtable+type+stable_id",
            "division_templates": "shared_heap/vtable+type+stable_id",
            "template_owner_order": "CCountry+0x1B8/+0x1C4",
            "equipment_ui_occurrences": "CCountry+0xF68 manager A(+0xA0/+0xAC) then B(+0xE8/+0xF4)",
        },
        "country_count": count,
        "countries": [{"registry_index": i, "tag": tag} for i, tag, _ in countries],
        "variants": variants,
        "division_templates": templates,
        "ui_occurrence_counts": {
            "unique_variants": len(first_ui_discovery),
            "catalog_variants_without_occurrence": len(variant_catalog) - len(first_ui_discovery),
        },
    }


def compare(snapshot: dict, oracle: dict) -> dict:
    def oracle_utf8(value: object) -> object:
        """Repair Rakaly JSON's UTF-8-as-Latin-1 text without touching native data."""
        if not isinstance(value, str):
            return value
        try:
            repaired = value.encode("latin-1").decode("utf-8")
        except (UnicodeEncodeError, UnicodeDecodeError):
            return value
        return repaired

    live_variants = {_key(row): row for row in snapshot["variants"]}
    save_variants = oracle["equipment_variants"]
    live_templates = {_key(row): row for row in snapshot["division_templates"]}
    save_templates = {_key(row): row for row in oracle["division_templates"]}
    variant_ids_equal = set(live_variants) == set(save_variants)
    template_ids_equal = set(live_templates) == set(save_templates)
    variant_owner_equal = variant_ids_equal and all(
        live_variants[key]["owner_tag"] == save_variants[key]["creator"] for key in save_variants
    )
    variant_origin_equal = variant_ids_equal and all(
        (live_variants[key]["origin_override_tag"] or "---") == (save_variants[key].get("origin") or "---")
        for key in save_variants
    )
    variant_obsolete_equal = variant_ids_equal and all(
        live_variants[key]["obsolete"] == bool(save_variants[key]["obsolete"]) for key in save_variants
    )
    template_owner_equal = template_ids_equal and all(
        live_templates[key]["owner_tag"] == save_templates[key]["country"] for key in save_templates
    )
    variant_archetype_equal = variant_ids_equal and all(
        live_variants[key]["definition"]["archetype_token"] == save_variants[key]["archetype"]
        for key in save_variants
    )
    variant_display_name_equal = variant_ids_equal and all(
        live_variants[key]["definition"]["display_name"] == oracle_utf8(save_variants[key].get("name"))
        for key in save_variants
    )
    variant_module_token_multisets_equal = variant_ids_equal and all(
        sorted(row["module_token"] for row in live_variants[key]["definition"]["modules"])
        == sorted((save_variants[key].get("modules") or {}).values())
        for key in save_variants
    )
    variant_upgrade_level_multisets_equal = variant_ids_equal and all(
        sorted(row["level"] for row in live_variants[key]["definition"]["upgrades"])
        == sorted((save_variants[key].get("upgrades") or {}).values())
        for key in save_variants
    )
    template_definition_equal = template_ids_equal and all(
        live_templates[key]["definition"]["name"] == oracle_utf8(save_templates[key]["raw_name"])
        and live_templates[key]["definition"]["priority"] == save_templates[key]["priority"]
        and live_templates[key]["definition"]["is_locked"] == bool(save_templates[key]["is_locked"])
        and [
            {field: row[field] for field in ("type", "x", "y")}
            for row in live_templates[key]["definition"]["regiments"]
        ]
        == save_templates[key]["regiments"]
        and [
            {field: row[field] for field in ("type", "x", "y")}
            for row in live_templates[key]["definition"]["support"]
        ]
        == save_templates[key]["support"]
        and [
            {field: row[field] for field in ("type", "x", "y")}
            for row in live_templates[key]["definition"]["regimental_support"]
        ]
        == save_templates[key]["regimental_support"]
        for key in save_templates
    )

    def mismatch_summary(name: str, rows: list[tuple[str, object, object]]) -> dict:
        return {
            "field": name,
            "count": len(rows),
            "examples": [
                {"id": key, "live": live, "oracle": expected}
                for key, live, expected in rows[:10]
            ],
        }

    display_name_mismatches = [
        (
            key,
            live_variants[key]["definition"]["display_name"],
            oracle_utf8(save_variants[key].get("name")),
        )
        for key in save_variants
        if live_variants[key]["definition"]["display_name"] != oracle_utf8(save_variants[key].get("name"))
    ] if variant_ids_equal else []
    template_field_mismatches = []
    if template_ids_equal:
        template_fields = {
            "name": lambda key: (
                live_templates[key]["definition"]["name"], oracle_utf8(save_templates[key]["raw_name"])
            ),
            "priority": lambda key: (
                live_templates[key]["definition"]["priority"], save_templates[key]["priority"]
            ),
            "is_locked": lambda key: (
                live_templates[key]["definition"]["is_locked"], bool(save_templates[key]["is_locked"])
            ),
        }
        for field, values in template_fields.items():
            rows = []
            for key in save_templates:
                live, expected = values(key)
                if live != expected:
                    rows.append((key, live, expected))
            template_field_mismatches.append(mismatch_summary(field, rows))
        for field in ("regiments", "support", "regimental_support"):
            rows = []
            for key in save_templates:
                live = [
                    {item: row[item] for item in ("type", "x", "y")}
                    for row in live_templates[key]["definition"][field]
                ]
                expected = save_templates[key][field]
                if live != expected:
                    rows.append((key, live, expected))
            template_field_mismatches.append(mismatch_summary(field, rows))
    checks = {
        "variant_ids_equal": variant_ids_equal,
        "variant_owner_equal": variant_owner_equal,
        "variant_origin_equal": variant_origin_equal,
        "variant_obsolete_equal": variant_obsolete_equal,
        "template_ids_equal": template_ids_equal,
        "template_owner_equal": template_owner_equal,
        "variant_archetype_equal": variant_archetype_equal,
        "variant_display_name_equal": variant_display_name_equal,
        "variant_module_token_multisets_equal": variant_module_token_multisets_equal,
        "variant_upgrade_level_multisets_equal": variant_upgrade_level_multisets_equal,
        "template_definition_equal": template_definition_equal,
    }
    variants_without_ui_occurrence = sorted(
        key for key, row in live_variants.items() if not row.get("source_occurrences")
    )
    return {
        "schema": "extractor.a27.live_heap_catalog_acceptance.a729.v2",
        "status": "PASS" if all(checks.values()) else "FAIL",
        "checks": checks,
        "counts": {
            "countries": snapshot["country_count"],
            "variants": len(live_variants),
            "division_templates": len(live_templates),
            "variants_with_ui_occurrence": len(live_variants) - len(variants_without_ui_occurrence),
            "variants_without_ui_occurrence": len(variants_without_ui_occurrence),
        },
        "variants_without_ui_occurrence": variants_without_ui_occurrence,
        "mismatches": {
            "variant_display_name": mismatch_summary("display_name", display_name_mismatches),
            "template_definition": template_field_mismatches,
        },
        "oracle_text_repairs": {
            "variant_display_names": sum(
                1
                for row in save_variants.values()
                if row.get("name") != oracle_utf8(row.get("name"))
            ),
            "template_names": sum(
                1
                for row in save_templates.values()
                if row.get("raw_name") != oracle_utf8(row.get("raw_name"))
            ),
            "rule": "latin-1 codepoints reinterpreted as UTF-8 when the byte sequence is valid",
        },
        "input_hashes": {
            "snapshot_sha256": hashlib.sha256(
                json.dumps(snapshot, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode("utf-8")
            ).hexdigest()
        },
    }


def main(argv=None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--oracle", type=Path)
    parser.add_argument("--snapshot-out", required=True, type=Path)
    parser.add_argument("--report-out", type=Path)
    parser.add_argument(
        "--capture-only",
        action="store_true",
        help="freeze the authenticated native heap snapshot without consulting a save oracle",
    )
    parser.add_argument(
        "--snapshot-in",
        type=Path,
        help="reverify a previously frozen native snapshot without attaching to the game",
    )
    parser.add_argument("--pid", type=int)
    args = parser.parse_args(argv)
    if not args.capture_only and (args.oracle is None or args.report_out is None):
        parser.error("--oracle and --report-out are required unless --capture-only is used")
    if args.capture_only and args.snapshot_in:
        parser.error("--capture-only cannot be combined with --snapshot-in")
    if args.snapshot_in:
        snapshot = json.loads(args.snapshot_in.read_text(encoding="utf-8"))
    else:
        pid = args.pid or winmem.find_pid("hoi4.exe")
        if not pid:
            raise SystemExit("hoi4.exe is not running")
        with winmem.Process(pid, "hoi4.exe") as process:
            snapshot = capture(process)
    snapshot_bytes = json.dumps(snapshot, ensure_ascii=False, sort_keys=True, indent=2).encode("utf-8")
    args.snapshot_out.parent.mkdir(parents=True, exist_ok=True)
    args.snapshot_out.write_bytes(snapshot_bytes)
    if args.capture_only:
        print(
            f"CAPTURED: authenticated live heap full replacement with "
            f"{len(snapshot['variants'])} variants and "
            f"{len(snapshot['division_templates'])} templates"
        )
        return 0

    # Read the same-tick oracle only after the process capture is frozen.  This
    # makes it mechanically impossible for save-derived values to populate the
    # native snapshot.
    oracle = json.loads(args.oracle.read_text(encoding="utf-8"))
    report = compare(snapshot, oracle)
    args.report_out.parent.mkdir(parents=True, exist_ok=True)
    if args.snapshot_in and args.snapshot_in.resolve() == args.snapshot_out.resolve():
        snapshot_bytes = args.snapshot_out.read_bytes()
    report["input_hashes"]["snapshot_file_sha256"] = hashlib.sha256(snapshot_bytes).hexdigest()
    report["input_hashes"]["oracle_sha256"] = hashlib.sha256(args.oracle.read_bytes()).hexdigest()
    args.report_out.write_text(json.dumps(report, sort_keys=True, indent=2), encoding="utf-8")
    print(
        f"{report['status']}: live heap full replacement has {report['counts']['variants']} variants and "
        f"{report['counts']['division_templates']} templates across {report['counts']['countries']} countries; "
        f"A/B covers {report['counts']['variants_with_ui_occurrence']} variants"
    )
    return 0 if report["status"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
