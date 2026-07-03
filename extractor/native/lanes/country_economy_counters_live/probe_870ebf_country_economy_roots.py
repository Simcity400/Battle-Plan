#!/usr/bin/env python3
"""Bounded current-build root probe for domain 58 country economy counters.

This is lane-local discovery only. It validates the shared country anchors,
scans factory/resource/production residual candidate surfaces, and writes JSON
that can be compared to same-tick save/UI or controlled before/after evidence.

No offset emitted here is safe for compact encode paths unless a later evidence
packet promotes it from CANDIDATE to VALIDATED.
"""

from __future__ import annotations

import argparse
import json
import math
import struct
import sys
import time
from collections import Counter, defaultdict
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parents[4]
MEMORY_READER = REPO_ROOT / "extractor" / "memory_reader"
if str(MEMORY_READER) not in sys.path:
    sys.path.insert(0, str(MEMORY_READER))

import winmem  # type: ignore  # noqa: E402


DOMAIN_ID = 58
DOMAIN_NAME = "country_economy_counters_live"
EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"

CG_STATE_DATE_OFFSET = 0x3316E90
COUNTRY_DATABASE_OFFSET = 0x33189C8
CGAMESTATE_COUNTRY_TABLE = 0x310
CGAMESTATE_COUNTRY_COUNT = 0x31C
COUNTRY_TAG_RECORD_STRIDE = 0x20

COUNTRY_VTABLE_RVA = 0x27D0E90
OBJ_TAG_DB_ID = 0x08
OBJ_PROD_MANAGER = 0x0F68
OBJ_FUEL_STATUS = 0x1580

LINE_BEGIN = 0x58
LINE_CAP = 0x60
LINE_SIZE = 0x64
LINE_STRIDE = 0x08
LINE_TYPE = 0x08
LINE_ID = 0x0C
LINE_ACTIVE_FACTORIES = 0x18
LINE_COST = 0x28
LINE_SPEED = 0x30
LINE_PRODUCED = 0x38
LINE_AMOUNT = 0x48
LINE_SCAN_BYTES = 0xC0
TYPE_MILITARY_LINE = 56
TYPE_NAVAL_LINE = 57
VARIANT_VTABLE_RVA = 0x293AFB8
VARIANT_TYPE = 70

COUNTRY_SCAN_START = 0x400
COUNTRY_SCAN_END = 0x1800
PROD_MANAGER_SCAN_START = 0x00
PROD_MANAGER_SCAN_END = 0x800

HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000
MAX_COUNTRY_TABLE_SLOTS = 4096
MAX_PRODUCTION_LINES = 4096
MAX_VECTOR_ROWS = 256
MAX_COUNTER_ABS = 1_000_000
MAX_RESOURCE_AMOUNT_ABS = 10_000_000_000

RESOURCE_TID_NAMES = {
    0x3E03: "steel",
    0x3E05: "rubber",
    0x3E06: "tungsten",
    0x3E07: "chromium",
    0x4E1F: "aluminium",
    0x4E25: "coal",
    0x4E37: "oil",
}

RESOURCE_ORACLE_CATEGORIES = (
    "produced",
    "exported",
    "to_export",
    "available_derived",
    "imported",
    "to_use",
    "to_use_total",
    "base_export",
    "export",
)
RESOURCE_ORACLE_SCALES = (1, 1000, 100000)
ORACLE_VALIDATION_MIN_TAGS = 3

KNOWN_REJECTED_RESOURCE_CANDIDATES = [
    {
        "path": "CCountry+0x5B8",
        "status": "REJECTED_PRIOR_EVIDENCE",
        "reason": (
            "market_trade_live resource_flow_root_probe found no production-ready "
            "produced/exported/to_export mapping"
        ),
        "evidence": "extractor/native/runs/market_trade_live_resource_flow_20260701_v1/resource_flow_root_probe.json",
    },
    {
        "path": "CCountry+0x1218",
        "status": "REJECTED_PRIOR_EVIDENCE",
        "reason": (
            "market_trade_live state-shape proof found fewer resource ids than "
            "owned live state-resource ids for compared countries"
        ),
        "evidence": "extractor/native/runs/market_trade_live_resource_flow_state_shape_20260701_v1/resource_flow_state_shape_probe.json",
    },
]

DEFAULT_FOCUS_TAGS = ["IRE"]
DEFAULT_SAMPLE_TAGS = [
    "IRE",
    "ENG",
    "GER",
    "USA",
    "SOV",
    "JAP",
    "ITA",
    "FRA",
    "CAN",
    "AST",
    "SWE",
    "TUR",
]


def hx(value: int | None) -> str | None:
    return "0x%X" % value if isinstance(value, int) else None


def is_heap(value: int | None) -> bool:
    return isinstance(value, int) and HEAP_LO <= value < HEAP_HI and value % 8 == 0


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


def blob_u64(raw: bytes, offset: int) -> int | None:
    if offset < 0 or offset + 8 > len(raw):
        return None
    return struct.unpack_from("<Q", raw, offset)[0]


def blob_u32(raw: bytes, offset: int) -> int | None:
    if offset < 0 or offset + 4 > len(raw):
        return None
    return struct.unpack_from("<I", raw, offset)[0]


def blob_i32(raw: bytes, offset: int) -> int | None:
    if offset < 0 or offset + 4 > len(raw):
        return None
    return struct.unpack_from("<i", raw, offset)[0]


def blob_i64(raw: bytes, offset: int) -> int | None:
    if offset < 0 or offset + 8 > len(raw):
        return None
    return struct.unpack_from("<q", raw, offset)[0]


def blob_f32(raw: bytes, offset: int) -> float | None:
    if offset < 0 or offset + 4 > len(raw):
        return None
    value = struct.unpack_from("<f", raw, offset)[0]
    return value if math.isfinite(value) else None


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


def read_roots(p: Any) -> dict[str, Any]:
    module = p.module_base
    game_state = safe_u64(p, module + CG_STATE_DATE_OFFSET)
    country_database = safe_u64(p, module + COUNTRY_DATABASE_OFFSET)
    result = {
        "game_state_slot": hx(module + CG_STATE_DATE_OFFSET),
        "country_database_slot": hx(module + COUNTRY_DATABASE_OFFSET),
        "game_state": hx(game_state),
        "country_database": hx(country_database),
        "status": "unreadable",
    }
    if not is_heap(game_state) or not is_heap(country_database):
        return result

    tag_table = safe_u64(p, country_database)
    tag_count = safe_u32(p, country_database + 0x08)
    active_count = safe_u32(p, country_database + 0x0C)
    country_table = safe_u64(p, game_state + CGAMESTATE_COUNTRY_TABLE)
    country_count = safe_u32(p, game_state + CGAMESTATE_COUNTRY_COUNT)
    result.update(
        {
            "tag_table": hx(tag_table),
            "tag_count": tag_count,
            "active_count": active_count,
            "country_table": hx(country_table),
            "country_count": country_count,
            "country_table_offset": hx(CGAMESTATE_COUNTRY_TABLE),
            "country_count_offset": hx(CGAMESTATE_COUNTRY_COUNT),
            "status": "read",
        }
    )
    return result


def read_inline_tag(p: Any, tag_table: int, tag_id: int) -> str | None:
    raw = read_exact(p, tag_table + tag_id * COUNTRY_TAG_RECORD_STRIDE, 3)
    if raw is None:
        return None
    try:
        tag = raw.decode("ascii")
    except UnicodeDecodeError:
        return None
    return tag if tag.isalpha() and tag.isupper() else None


def load_tags(p: Any, roots: dict[str, Any]) -> dict[int, str]:
    tag_table = int(roots.get("tag_table") or "0", 16)
    tag_count = int(roots.get("tag_count") or 0)
    if not is_heap(tag_table):
        return {}
    tags: dict[int, str] = {}
    for tag_id in range(min(tag_count, MAX_COUNTRY_TABLE_SLOTS)):
        tag = read_inline_tag(p, tag_table, tag_id)
        if tag:
            tags[tag_id] = tag
    return tags


def discover_countries(
    p: Any,
    roots: dict[str, Any],
    tags: dict[int, str],
    sample_tags: set[str],
    focus_tags: set[str],
    max_countries: int,
    min_countries: int,
) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    country_table = int(roots.get("country_table") or "0", 16)
    country_count = int(roots.get("country_count") or 0)
    expected_vtable = p.module_base + COUNTRY_VTABLE_RVA
    all_valid: list[dict[str, Any]] = []
    rejects: list[dict[str, Any]] = []
    if not is_heap(country_table):
        return [], [{"status": "country_table_unreadable"}]

    for tag_id in range(1, min(country_count, MAX_COUNTRY_TABLE_SLOTS)):
        country_ptr = safe_u64(p, country_table + tag_id * 8)
        tag = tags.get(tag_id, f"id_{tag_id}")
        if not is_heap(country_ptr):
            continue
        vt = safe_u64(p, country_ptr)
        echo = safe_u32(p, country_ptr + OBJ_TAG_DB_ID)
        if vt != expected_vtable or echo != tag_id:
            if len(rejects) < 16:
                rejects.append(
                    {
                        "tag_db_id": tag_id,
                        "tag": tag,
                        "country_ptr": hx(country_ptr),
                        "vtable": hx(vt),
                        "expected_vtable": hx(expected_vtable),
                        "tag_echo": echo,
                    }
                )
            continue
        prod_manager = safe_u64(p, country_ptr + OBJ_PROD_MANAGER)
        fuel_status = safe_u64(p, country_ptr + OBJ_FUEL_STATUS)
        all_valid.append(
            {
                "tag_db_id": tag_id,
                "tag": tag,
                "country_ptr": country_ptr,
                "country_ptr_hex": hx(country_ptr),
                "production_manager": prod_manager if is_heap(prod_manager) else None,
                "production_manager_hex": hx(prod_manager) if is_heap(prod_manager) else None,
                "fuel_status": fuel_status if is_heap(fuel_status) else None,
                "fuel_status_hex": hx(fuel_status) if is_heap(fuel_status) else None,
            }
        )

    selected: list[dict[str, Any]] = []
    for row in all_valid:
        if row["tag"] in focus_tags or row["tag"] in sample_tags:
            selected.append(row)
    for row in all_valid:
        if len(selected) >= min_countries:
            break
        if row not in selected:
            selected.append(row)
    return selected[:max_countries], rejects


def load_json(path: str | None) -> dict[str, Any] | None:
    if not path:
        return None
    with open(path, "r", encoding="utf-8") as handle:
        return json.load(handle)


def economy_targets(economy: dict[str, Any] | None, ui: dict[str, Any] | None) -> dict[str, dict[str, Any]]:
    targets: dict[str, dict[str, Any]] = defaultdict(dict)
    if economy:
        for tag, row in economy.get("countries", {}).items():
            if not isinstance(row, dict):
                continue
            factories = row.get("factories") or {}
            factories_raw = row.get("factories_raw") or {}
            consumer_goods_raw = row.get("consumer_goods_raw") or {}
            resources = row.get("resources") or {}
            targets[tag]["factories_save"] = {
                key: value for key, value in factories.items() if isinstance(value, (int, float))
            }
            targets[tag]["factories_raw"] = {
                key: value
                for key, value in factories_raw.items()
                if isinstance(value, (int, float))
            }
            targets[tag]["consumer_goods_raw"] = {
                key: value
                for key, value in consumer_goods_raw.items()
                if isinstance(value, (int, float))
            }
            targets[tag]["resources_save"] = resources
            for key in ("stability", "war_support"):
                if row.get(key) is not None:
                    targets[tag][f"{key}_owned_by_domain14"] = row.get(key)
    if ui:
        countries = ui.get("countries", ui)
        if isinstance(countries, dict):
            for tag, row in countries.items():
                if isinstance(row, dict):
                    targets[tag].setdefault("ui_values", {}).update(row)
    return dict(targets)


def production_targets(production: dict[str, Any] | None) -> dict[tuple[str, str, int], dict[str, Any]]:
    out: dict[tuple[str, str, int], dict[str, Any]] = {}
    if not production:
        return out
    for tag, country in production.get("countries", {}).items():
        for line in (country or {}).get("equipment_lines", []):
            line_id = line.get("line_id")
            kind = line.get("kind")
            if isinstance(line_id, int) and isinstance(kind, str):
                out[(tag, kind, line_id)] = line
    return out


def line_kind_name(line_type: int) -> str:
    if line_type == TYPE_MILITARY_LINE:
        return "military_lines"
    if line_type == TYPE_NAVAL_LINE:
        return "naval_lines"
    return f"type_{line_type}"


def read_production_lines(p: Any, country: dict[str, Any]) -> list[dict[str, Any]]:
    manager = country.get("production_manager")
    if not is_heap(manager):
        return []
    begin = safe_u64(p, manager + LINE_BEGIN)
    cap = safe_u32(p, manager + LINE_CAP)
    size = safe_u32(p, manager + LINE_SIZE)
    if not is_heap(begin) or not isinstance(size, int) or not isinstance(cap, int):
        return []
    if size > cap or size > MAX_PRODUCTION_LINES:
        return []
    ptrs = read_exact(p, begin, size * LINE_STRIDE)
    if ptrs is None:
        return []
    rows: list[dict[str, Any]] = []
    for index in range(size):
        line_ptr = blob_u64(ptrs, index * LINE_STRIDE)
        if not is_heap(line_ptr):
            continue
        raw = read_exact(p, line_ptr, LINE_SCAN_BYTES)
        if raw is None:
            continue
        line_type = blob_u32(raw, LINE_TYPE)
        if line_type not in (TYPE_MILITARY_LINE, TYPE_NAVAL_LINE):
            continue
        row = {
            "tag_db_id": country["tag_db_id"],
            "tag": country["tag"],
            "kind": line_kind_name(line_type),
            "line_ptr": line_ptr,
            "line_ptr_hex": hx(line_ptr),
            "line_id": blob_i32(raw, LINE_ID),
            "active_factories": blob_i32(raw, LINE_ACTIVE_FACTORIES),
            "cost_raw": blob_i32(raw, LINE_COST),
            "speed_raw": blob_i32(raw, LINE_SPEED),
            "produced_raw": blob_i32(raw, LINE_PRODUCED),
            "amount": blob_i32(raw, LINE_AMOUNT),
            "_raw": raw,
        }
        rows.append(row)
    return rows


def production_line_summary(lines: list[dict[str, Any]]) -> dict[str, Any]:
    by_tag: dict[str, dict[str, int]] = defaultdict(lambda: defaultdict(int))
    for line in lines:
        tag = line["tag"]
        kind = line["kind"]
        by_tag[tag][f"{kind}_count"] += 1
        by_tag[tag][f"{kind}_active_factories"] += int(line.get("active_factories") or 0)
    return {
        "line_count": len(lines),
        "by_tag": {tag: dict(values) for tag, values in sorted(by_tag.items())},
        "known_validated_offsets": {
            "line_type": hx(LINE_TYPE),
            "line_id": hx(LINE_ID),
            "active_factories": hx(LINE_ACTIVE_FACTORIES),
            "cost_raw": hx(LINE_COST),
            "speed_raw": hx(LINE_SPEED),
            "produced_raw": hx(LINE_PRODUCED),
            "amount": hx(LINE_AMOUNT),
        },
    }


def analyze_line_residual_candidates(
    p: Any,
    lines: list[dict[str, Any]],
    oracle_lines: dict[tuple[str, str, int], dict[str, Any]],
) -> dict[str, Any]:
    scalar_matches: dict[str, dict[int, list[dict[str, Any]]]] = {
        "requested_factories": defaultdict(list),
        "priority": defaultdict(list),
    }
    variant_matches: dict[int, list[dict[str, Any]]] = defaultdict(list)
    matched_lines = 0
    divergent_requested = 0
    variant_vtable = p.module_base + VARIANT_VTABLE_RVA

    for line in lines:
        line_id = line.get("line_id")
        if not isinstance(line_id, int):
            continue
        oracle = oracle_lines.get((line["tag"], line["kind"], line_id))
        if not oracle:
            continue
        matched_lines += 1
        raw = line["_raw"]
        requested = oracle.get("requested_factories")
        active = line.get("active_factories")
        if isinstance(requested, int) and isinstance(active, int) and requested != active:
            divergent_requested += 1
        for field in ("requested_factories", "priority"):
            target = oracle.get(field)
            if not isinstance(target, int):
                continue
            for offset in range(0, len(raw) - 3, 4):
                value = blob_i32(raw, offset)
                if value == target:
                    scalar_matches[field][offset].append(
                        {
                            "tag": line["tag"],
                            "kind": line["kind"],
                            "line_id": line_id,
                            "offset": hx(offset),
                            "value": value,
                            "active_factories": active,
                        }
                    )
        equip_id = oracle.get("equip_id")
        equip_type = oracle.get("equip_type")
        if isinstance(equip_id, int) and equip_type == VARIANT_TYPE:
            for offset in range(0, len(raw) - 7, 8):
                ptr = blob_u64(raw, offset)
                if not is_heap(ptr):
                    continue
                if safe_u64(p, ptr) != variant_vtable:
                    continue
                if safe_u32(p, ptr + 0x08) != VARIANT_TYPE:
                    continue
                if safe_u32(p, ptr + 0x0C) == equip_id:
                    variant_matches[offset].append(
                        {
                            "tag": line["tag"],
                            "kind": line["kind"],
                            "line_id": line_id,
                            "offset": hx(offset),
                            "equipment_variant_ptr": hx(ptr),
                            "equip_id": equip_id,
                        }
                    )

    def summarize_field(field: str, matches_by_offset: dict[int, list[dict[str, Any]]]) -> list[dict[str, Any]]:
        rows = []
        for offset, examples in matches_by_offset.items():
            known_overlap = offset in {
                LINE_TYPE,
                LINE_ID,
                LINE_ACTIVE_FACTORIES,
                LINE_COST,
                LINE_SPEED,
                LINE_PRODUCED,
                LINE_AMOUNT,
            }
            rows.append(
                {
                    "field": field,
                    "offset": hx(offset),
                    "match_count": len(examples),
                    "status": (
                        "CANDIDATE_AMBIGUOUS_KNOWN_FIELD_OVERLAP"
                        if known_overlap
                        else "CANDIDATE_NEEDS_SAME_TICK_OR_DELTA_PROOF"
                    ),
                    "examples": examples[:12],
                }
            )
        rows.sort(key=lambda row: (-row["match_count"], row["offset"]))
        return rows[:24]

    return {
        "oracle_line_matches": matched_lines,
        "requested_factories_lines_where_requested_differs_from_active": divergent_requested,
        "requested_factory_offset_candidates": summarize_field(
            "requested_factories", scalar_matches["requested_factories"]
        ),
        "priority_offset_candidates": summarize_field("priority", scalar_matches["priority"]),
        "equipment_variant_pointer_candidates": [
            {
                "field": "equipment_variant_index",
                "offset": hx(offset),
                "match_count": len(examples),
                "status": "CANDIDATE_NEEDS_SAME_TICK_OR_DELTA_PROOF",
                "examples": examples[:12],
            }
            for offset, examples in sorted(
                variant_matches.items(), key=lambda item: (-len(item[1]), item[0])
            )[:24]
        ],
        "status": (
            "NO_PRODUCTION_ORACLE_SUPPLIED"
            if not oracle_lines
            else "CANDIDATES_ONLY_UNTIL_CONTROLLED_DELTA"
        ),
    }


def plausible_counter(value: int | None) -> bool:
    return isinstance(value, int) and -MAX_COUNTER_ABS <= value <= MAX_COUNTER_ABS


def plausible_float(value: float | None) -> bool:
    return isinstance(value, float) and math.isfinite(value) and -10.0 <= value <= 10.0


def scan_scalar_census(
    p: Any,
    countries: list[dict[str, Any]],
    object_key: str,
    object_label: str,
    start: int,
    end: int,
    min_coverage: int,
    limit: int,
) -> dict[str, Any]:
    groups: dict[tuple[str, int], dict[str, Any]] = {}
    scanned_objects = 0
    for country in countries:
        base = country.get(object_key)
        if not is_heap(base):
            continue
        raw = read_exact(p, base + start, end - start)
        if raw is None:
            continue
        scanned_objects += 1
        for rel in range(0, len(raw) - 3, 4):
            offset = start + rel
            i32_value = blob_i32(raw, rel)
            u32_value = blob_u32(raw, rel)
            f32_value = blob_f32(raw, rel)
            for type_name, value in (("i32", i32_value), ("u32", u32_value)):
                if not plausible_counter(value):
                    continue
                key = (type_name, offset)
                row = groups.setdefault(key, {"values": Counter(), "examples": []})
                row["values"][value] += 1
                if len(row["examples"]) < 8:
                    row["examples"].append({"tag": country["tag"], "value": value})
            if plausible_float(f32_value):
                rounded = round(float(f32_value), 6)
                key = ("f32", offset)
                row = groups.setdefault(key, {"values": Counter(), "examples": []})
                row["values"][rounded] += 1
                if len(row["examples"]) < 8:
                    row["examples"].append({"tag": country["tag"], "value": rounded})

    candidates = []
    for (type_name, offset), row in groups.items():
        coverage = sum(row["values"].values())
        if coverage < min_coverage:
            continue
        unique_values = len(row["values"])
        top = row["values"].most_common(8)
        candidates.append(
            {
                "object": object_label,
                "type": type_name,
                "offset": hx(offset),
                "coverage": coverage,
                "unique_value_count": unique_values,
                "top_values": [{"value": value, "count": count} for value, count in top],
                "examples": row["examples"],
                "status": "CANDIDATE_CENSUS_ONLY",
            }
        )
    candidates.sort(
        key=lambda row: (
            -row["coverage"],
            row["unique_value_count"],
            row["object"],
            row["type"],
            row["offset"],
        )
    )
    return {
        "object": object_label,
        "range": f"{object_label}+{hx(start)}..+{hx(end)}",
        "scanned_objects": scanned_objects,
        "candidate_count": len(candidates),
        "candidates": candidates[:limit],
    }


def flatten_numeric_targets(targets: dict[str, Any]) -> list[dict[str, Any]]:
    out: list[dict[str, Any]] = []

    def walk(prefix: str, value: Any) -> None:
        if isinstance(value, bool):
            return
        if isinstance(value, (int, float)):
            out.append({"path": prefix, "value": value})
        elif isinstance(value, dict):
            for key, child in value.items():
                walk(f"{prefix}.{key}" if prefix else str(key), child)

    walk("", targets)
    return out


def nonzero_number(value: Any) -> bool:
    return isinstance(value, (int, float)) and not isinstance(value, bool) and value not in (0, 0.0)


def int_target(value: Any) -> int | None:
    if isinstance(value, bool):
        return None
    if isinstance(value, int):
        return value
    if isinstance(value, float) and math.isfinite(value) and abs(value - round(value)) < 0.000001:
        return int(round(value))
    return None


def float_close(observed: float | int, target: float | int, scale: int = 1) -> bool:
    want = float(target) * scale
    got = float(observed)
    return abs(got - want) <= max(0.001 * max(1.0, abs(want)), 0.001)


def oracle_eligible_tags_by_path(
    targets_by_tag: dict[str, dict[str, Any]],
    scanned_tags: set[str],
) -> dict[str, set[str]]:
    eligible: dict[str, set[str]] = defaultdict(set)
    for tag, target_tree in targets_by_tag.items():
        if tag not in scanned_tags:
            continue
        for target in flatten_numeric_targets(target_tree):
            if nonzero_number(target["value"]):
                eligible[target["path"]].add(tag)
    return eligible


def scalar_match_status(path: str, matched: set[str], eligible: set[str]) -> str:
    if "stability_owned_by_domain14" in path or "war_support_owned_by_domain14" in path:
        if len(matched) >= ORACLE_VALIDATION_MIN_TAGS and matched == eligible:
            return "OVERLAP_DOMAIN14_ORACLE_ALL_SAMPLED_DO_NOT_EMIT"
        return "OVERLAP_DOMAIN14_CANDIDATE_DO_NOT_EMIT"
    if len(matched) >= ORACLE_VALIDATION_MIN_TAGS and matched == eligible:
        return "VALIDATED_ORACLE_ALL_SAMPLED"
    if len(matched) >= ORACLE_VALIDATION_MIN_TAGS:
        return "CANDIDATE_ORACLE_PARTIAL"
    return "CANDIDATE_ORACLE_INSUFFICIENT_COUNTRIES"


def path_get(tree: dict[str, Any], path: str) -> Any:
    value: Any = tree
    for part in path.split("."):
        if not isinstance(value, dict) or part not in value:
            return None
        value = value[part]
    return value


def is_factory_field_validation_candidate(path: str) -> bool:
    return (
        path.startswith("factories_save.")
        or path.startswith("factories_raw.")
        or path.startswith("consumer_goods_raw.")
        or path.startswith("ui_values.factories.")
        or path.startswith("ui_values.consumer_goods.")
        or path.startswith("consumer_goods.")
    )


def parse_hex_offset(value: str) -> int | None:
    try:
        return int(value, 16)
    except (TypeError, ValueError):
        return None


def read_scalar_at_offset(
    p: Any,
    country: dict[str, Any],
    object_label: str,
    offset: int,
    type_name: str,
) -> Any:
    base = (
        country.get("country_ptr")
        if object_label == "CCountry"
        else country.get("production_manager")
        if object_label == "CProductionManager"
        else None
    )
    if not is_heap(base):
        return None
    raw = read_exact(p, base + offset, 4)
    if raw is None:
        return None
    if type_name == "i32":
        return blob_i32(raw, 0)
    if type_name == "u32":
        return blob_u32(raw, 0)
    if type_name == "f32":
        value = blob_f32(raw, 0)
        return round(float(value), 6) if isinstance(value, float) and math.isfinite(value) else None
    return None


def scalar_oracle_equal(type_name: str, live_value: Any, oracle_value: Any) -> bool:
    if not nonzero_number(oracle_value) and oracle_value not in (0, 0.0):
        return False
    if type_name in ("i32", "u32"):
        target = int_target(oracle_value)
        return target is not None and live_value == target
    if type_name == "f32" and isinstance(live_value, (int, float)):
        return isinstance(oracle_value, (int, float)) and float_close(live_value, oracle_value)
    return False


def build_field_validation_tables(
    p: Any,
    countries: list[dict[str, Any]],
    targets_by_tag: dict[str, dict[str, Any]],
    matches: list[dict[str, Any]],
    validation_tags: list[str],
) -> list[dict[str, Any]]:
    countries_by_tag = {country["tag"]: country for country in countries}
    candidate_keys: set[tuple[str, str, str, str]] = set()
    for match in matches:
        path = match.get("target_path", "")
        if not is_factory_field_validation_candidate(path):
            continue
        candidate_keys.add(
            (
                match["object"],
                match["offset"],
                match["type"],
                path,
            )
        )

    tables = []
    for object_label, offset_hex, type_name, target_path in sorted(candidate_keys):
        offset = parse_hex_offset(offset_hex)
        if offset is None:
            continue
        rows = []
        match_count = 0
        missing = []
        eligible_count = 0
        oracle_values = []
        live_values = []
        for tag in validation_tags:
            country = countries_by_tag.get(tag)
            oracle_value = path_get(targets_by_tag.get(tag, {}), target_path)
            live_value = (
                read_scalar_at_offset(p, country, object_label, offset, type_name)
                if country is not None
                else None
            )
            if country is None or oracle_value is None or live_value is None:
                missing.append(tag)
                match_bool: bool | None = None
            else:
                eligible_count += 1
                match_bool = scalar_oracle_equal(type_name, live_value, oracle_value)
                if match_bool:
                    match_count += 1
                if isinstance(oracle_value, (int, float)) and not isinstance(oracle_value, bool):
                    oracle_values.append(oracle_value)
                if isinstance(live_value, (int, float)) and not isinstance(live_value, bool):
                    live_values.append(live_value)
            rows.append(
                {
                    "tag": tag,
                    "live_value_at_offset": live_value,
                    "oracle_value_for_tag": oracle_value,
                    "match_bool": match_bool,
                }
            )

        distinct_oracle_values = sorted({str(value) for value in oracle_values})
        if eligible_count < 6:
            status = "CANDIDATE_INSUFFICIENT_RAW_ORACLE_COVERAGE"
        elif len(distinct_oracle_values) <= 1:
            status = "REJECTED_CONSTANT_ORACLE_VECTOR"
        elif match_count == eligible_count:
            status = "VALIDATED_FIELD_ALL_SAMPLED_NONCONSTANT"
        elif match_count >= max(1, eligible_count - 1):
            status = "CANDIDATE_FIELD_NEAR_ALL_SAMPLED"
        else:
            status = "REJECTED_FIELD_MISMATCH"

        tables.append(
            {
                "object": object_label,
                "offset": offset_hex,
                "type": type_name,
                "target_path": target_path,
                "matched_tag_count": match_count,
                "sampled_tag_count": len(validation_tags),
                "eligible_non_null_tag_count": eligible_count,
                "distinct_oracle_value_count": len(distinct_oracle_values),
                "live_value_vector": live_values,
                "oracle_value_vector": oracle_values,
                "missing_tags": missing,
                "per_country": rows,
                "status": status,
            }
        )

    alias_groups: dict[tuple[str, tuple[Any, ...]], list[int]] = defaultdict(list)
    for index, table in enumerate(tables):
        if table["status"] == "VALIDATED_FIELD_ALL_SAMPLED_NONCONSTANT":
            alias_groups[
                (
                    table["target_path"],
                    tuple(row["live_value_at_offset"] for row in table["per_country"]),
                )
            ].append(index)
    for indexes in alias_groups.values():
        if len(indexes) <= 1:
            continue
        for index in indexes:
            tables[index]["status"] = "CANDIDATE_FIELD_ALIAS_SAME_VECTOR"
            tables[index]["alias_group_size"] = len(indexes)

    status_rank = {
        "VALIDATED_FIELD_ALL_SAMPLED_NONCONSTANT": 0,
        "CANDIDATE_FIELD_ALIAS_SAME_VECTOR": 1,
        "CANDIDATE_FIELD_NEAR_ALL_SAMPLED": 2,
        "CANDIDATE_INSUFFICIENT_RAW_ORACLE_COVERAGE": 3,
        "REJECTED_FIELD_MISMATCH": 4,
        "REJECTED_CONSTANT_ORACLE_VECTOR": 5,
    }
    tables.sort(
        key=lambda row: (
            status_rank.get(row["status"], 9),
            row["target_path"],
            row["object"],
            row["offset"],
            row["type"],
        )
    )
    return tables


def summarize_cross_country_scalar_matches(
    matches: list[dict[str, Any]],
    targets_by_tag: dict[str, dict[str, Any]],
    scanned_tags: set[str],
) -> list[dict[str, Any]]:
    eligible_by_path = oracle_eligible_tags_by_path(targets_by_tag, scanned_tags)
    groups: dict[tuple[str, str, str, str], dict[str, Any]] = {}
    for match in matches:
        key = (
            match["object"],
            match["offset"],
            match["type"],
            match["target_path"],
        )
        row = groups.setdefault(
            key,
            {
                "tags": set(),
                "examples": [],
                "target_values": {},
            },
        )
        tag = match["tag"]
        row["tags"].add(tag)
        row["target_values"][tag] = match["target_value"]
        if len(row["examples"]) < 16:
            row["examples"].append(match)

    summaries = []
    for (obj, offset, type_name, path), row in groups.items():
        matched = set(row["tags"])
        eligible = eligible_by_path.get(path, set())
        missing = sorted(eligible - matched)
        status = scalar_match_status(path, matched, eligible)
        summaries.append(
            {
                "object": obj,
                "offset": offset,
                "type": type_name,
                "target_path": path,
                "matched_tag_count": len(matched),
                "eligible_nonzero_tag_count": len(eligible),
                "matched_tags": sorted(matched),
                "missing_tags": missing,
                "target_values_by_tag": dict(sorted(row["target_values"].items())),
                "examples": row["examples"],
                "status": status,
            }
        )

    status_rank = {
        "VALIDATED_ORACLE_ALL_SAMPLED": 0,
        "OVERLAP_DOMAIN14_ORACLE_ALL_SAMPLED_DO_NOT_EMIT": 1,
        "CANDIDATE_ORACLE_PARTIAL": 2,
        "OVERLAP_DOMAIN14_CANDIDATE_DO_NOT_EMIT": 3,
        "CANDIDATE_ORACLE_INSUFFICIENT_COUNTRIES": 4,
    }
    summaries.sort(
        key=lambda row: (
            status_rank.get(row["status"], 9),
            -row["matched_tag_count"],
            row["target_path"],
            row["object"],
            row["offset"],
            row["type"],
        )
    )
    return summaries


def find_oracle_scalar_matches(
    p: Any,
    countries: list[dict[str, Any]],
    targets_by_tag: dict[str, dict[str, Any]],
    validation_tags: list[str],
) -> dict[str, Any]:
    matches: list[dict[str, Any]] = []
    scanned_tags = {country["tag"] for country in countries}
    for country in countries:
        tag = country["tag"]
        targets = flatten_numeric_targets(targets_by_tag.get(tag, {}))
        targets = [row for row in targets if nonzero_number(row["value"])]
        if not targets:
            continue
        objects = [
            ("CCountry", country.get("country_ptr"), COUNTRY_SCAN_START, COUNTRY_SCAN_END),
            (
                "CProductionManager",
                country.get("production_manager"),
                PROD_MANAGER_SCAN_START,
                PROD_MANAGER_SCAN_END,
            ),
        ]
        for label, base, start, end in objects:
            if not is_heap(base):
                continue
            raw = read_exact(p, base + start, end - start)
            if raw is None:
                continue
            for target in targets:
                tval = target["value"]
                t_int = int_target(tval)
                for rel in range(0, len(raw) - 3, 4):
                    offset = start + rel
                    i32_value = blob_i32(raw, rel)
                    u32_value = blob_u32(raw, rel)
                    f32_value = blob_f32(raw, rel)
                    for type_name, value in (("i32", i32_value), ("u32", u32_value)):
                        if t_int is not None and value == t_int:
                            matches.append(
                                {
                                    "tag": tag,
                                    "object": label,
                                    "offset": hx(offset),
                                    "type": type_name,
                                    "target_path": target["path"],
                                    "target_value": tval,
                                    "observed_value": value,
                                    "status": "CANDIDATE_SINGLE_SNAPSHOT_MATCH",
                                }
                            )
                    if isinstance(f32_value, float):
                        if float_close(float(f32_value), float(tval)):
                            matches.append(
                                {
                                    "tag": tag,
                                    "object": label,
                                    "offset": hx(offset),
                                    "type": "f32",
                                    "target_path": target["path"],
                                    "target_value": tval,
                                    "observed_value": round(float(f32_value), 6),
                                    "status": "CANDIDATE_SINGLE_SNAPSHOT_MATCH",
                                }
                            )
    summaries = summarize_cross_country_scalar_matches(matches, targets_by_tag, scanned_tags)
    field_tables = build_field_validation_tables(
        p, countries, targets_by_tag, matches, validation_tags
    )
    validated = [
        row
        for row in summaries
        if row["status"] == "VALIDATED_ORACLE_ALL_SAMPLED"
    ]
    validated_fields = [
        row
        for row in field_tables
        if row["status"] == "VALIDATED_FIELD_ALL_SAMPLED_NONCONSTANT"
    ]
    return {
        "match_count": len(matches),
        "matches": matches[:160],
        "cross_country_match_count": len(summaries),
        "validated_cross_country_count": len(validated),
        "cross_country_matches": summaries[:80],
        "field_validation_table_count": len(field_tables),
        "field_validation_validated_count": len(validated_fields),
        "field_validation_tables": field_tables,
        "status": (
            "NO_FACTORY_OR_UI_ORACLE_SUPPLIED"
            if not targets_by_tag
            else (
                "ORACLE_VALIDATED_CANDIDATES_PRESENT"
                if validated
                else "CANDIDATES_ONLY_NO_ALL_SAMPLED_MATCH"
            )
        ),
    }


def focus_scalar_samples(
    p: Any,
    countries: list[dict[str, Any]],
    focus_tags: set[str],
) -> dict[str, Any]:
    samples: dict[str, Any] = {}
    for country in countries:
        if country["tag"] not in focus_tags:
            continue
        tag_samples: dict[str, Any] = {}
        for label, base, start, end in (
            ("CCountry", country.get("country_ptr"), COUNTRY_SCAN_START, COUNTRY_SCAN_END),
            (
                "CProductionManager",
                country.get("production_manager"),
                PROD_MANAGER_SCAN_START,
                PROD_MANAGER_SCAN_END,
            ),
        ):
            if not is_heap(base):
                continue
            raw = read_exact(p, base + start, end - start)
            if raw is None:
                continue
            ints: dict[str, int] = {}
            floats: dict[str, float] = {}
            for rel in range(0, len(raw) - 3, 4):
                offset = start + rel
                i32_value = blob_i32(raw, rel)
                f32_value = blob_f32(raw, rel)
                if plausible_counter(i32_value):
                    ints[hx(offset) or "0x0"] = int(i32_value)
                if plausible_float(f32_value):
                    floats[hx(offset) or "0x0"] = round(float(f32_value), 6)
            tag_samples[label] = {
                "i32": ints,
                "f32": floats,
                "range": f"{label}+{hx(start)}..+{hx(end)}",
            }
        samples[country["tag"]] = tag_samples
    return samples


def plausible_resource_id(value: int | None) -> bool:
    if not isinstance(value, int):
        return False
    return 0 <= value <= 128 or value in RESOURCE_TID_NAMES


def score_resource_vector(
    raw: bytes,
    size: int,
    stride: int,
    resource_off: int,
    amount_off: int,
    amount_width: int,
) -> dict[str, Any] | None:
    if size <= 0 or size > MAX_VECTOR_ROWS or len(raw) < size * stride:
        return None
    rows = []
    resource_ids = []
    nonzero = 0
    for index in range(size):
        base = index * stride
        rid = blob_u32(raw, base + resource_off)
        amount = blob_i64(raw, base + amount_off) if amount_width == 8 else blob_i32(raw, base + amount_off)
        if not plausible_resource_id(rid) or not isinstance(amount, int):
            return None
        if abs(amount) > MAX_RESOURCE_AMOUNT_ABS:
            return None
        resource_ids.append(rid)
        if amount:
            nonzero += 1
        if len(rows) < 12:
            rows.append(
                {
                    "resource_id": rid,
                    "resource_name_if_known": RESOURCE_TID_NAMES.get(rid),
                    "amount_raw": amount,
                }
            )
    return {
        "resource_ids": resource_ids,
        "distinct_resource_id_count": len(set(resource_ids)),
        "duplicate_resource_ids": len(resource_ids) - len(set(resource_ids)),
        "nonzero_amount_rows": nonzero,
        "rows_preview": rows,
    }


def scan_resource_vectors_for_object(
    p: Any,
    country: dict[str, Any],
    object_label: str,
    base: int | None,
    start: int,
    end: int,
) -> list[dict[str, Any]]:
    if not is_heap(base):
        return []
    blob = read_exact(p, base + start, end - start)
    if blob is None:
        return []
    hits = []
    layouts = (
        (0, 4, 4),
        (4, 0, 4),
        (0, 8, 8),
        (8, 0, 8),
        (8, 16, 8),
        (16, 8, 8),
        (0, 16, 8),
        (16, 0, 8),
    )
    for rel in range(0, len(blob) - 15, 8):
        begin = blob_u64(blob, rel)
        cap = blob_u32(blob, rel + 8)
        size = blob_u32(blob, rel + 12)
        if not is_heap(begin) or not isinstance(size, int) or not isinstance(cap, int):
            continue
        if size == 0 or size > cap or cap > MAX_VECTOR_ROWS:
            continue
        for stride in (8, 12, 16, 24, 32, 40):
            raw = read_exact(p, begin, size * stride)
            if raw is None:
                continue
            for amount_off, resource_off, amount_width in layouts:
                if max(amount_off + amount_width, resource_off + 4) > stride:
                    continue
                scored = score_resource_vector(
                    raw, size, stride, resource_off, amount_off, amount_width
                )
                if scored is None:
                    continue
                object_offset = start + rel
                hits.append(
                    {
                        "tag_db_id": country["tag_db_id"],
                        "tag": country["tag"],
                        "object": object_label,
                        "object_offset": object_offset,
                        "object_offset_hex": hx(object_offset),
                        "begin": hx(begin),
                        "size": size,
                        "cap": cap,
                        "stride": stride,
                        "amount_off": amount_off,
                        "amount_width": amount_width,
                        "resource_off": resource_off,
                        **scored,
                    }
                )
    return hits


def resource_target_map(
    targets_by_tag: dict[str, dict[str, Any]],
    tag: str,
    category: str,
) -> dict[str, Any]:
    resources = targets_by_tag.get(tag, {}).get("resources_save", {})
    if not isinstance(resources, dict):
        return {}
    values = resources.get(category, {})
    return values if isinstance(values, dict) else {}


def summarize_resource_oracle_matches(
    groups: dict[tuple[Any, ...], list[dict[str, Any]]],
    targets_by_tag: dict[str, dict[str, Any]],
) -> dict[str, Any]:
    if not targets_by_tag:
        return {
            "match_count": 0,
            "validated_cross_country_count": 0,
            "matches": [],
            "status": "NO_RESOURCE_ORACLE_SUPPLIED",
        }

    summaries = []
    for (obj, offset, stride, amount_off, amount_width, resource_off), rows in groups.items():
        for category in RESOURCE_ORACLE_CATEGORIES:
            for scale in RESOURCE_ORACLE_SCALES:
                matched_tags: set[str] = set()
                eligible_tags: set[str] = set()
                examples = []
                mismatches = []
                for row in rows:
                    tag = row["tag"]
                    target_map = resource_target_map(targets_by_tag, tag, category)
                    if not target_map:
                        continue
                    known_targets = {
                        key: value
                        for key, value in target_map.items()
                        if key in RESOURCE_TID_NAMES.values() and nonzero_number(value)
                    }
                    if not known_targets:
                        continue
                    comparable = []
                    failed = []
                    for item in row["rows_preview"]:
                        name = item.get("resource_name_if_known")
                        if not name or name not in known_targets:
                            continue
                        target_value = known_targets[name]
                        observed_value = item.get("amount_raw")
                        comparable.append(
                            {
                                "resource": name,
                                "observed_raw": observed_value,
                                "target_value": target_value,
                            }
                        )
                        if not float_close(observed_value, target_value, scale):
                            failed.append(
                                {
                                    "resource": name,
                                    "observed_raw": observed_value,
                                    "target_value": target_value,
                                    "scale": scale,
                                }
                            )
                    if not comparable:
                        continue
                    eligible_tags.add(tag)
                    if not failed:
                        matched_tags.add(tag)
                        if len(examples) < 12:
                            examples.append(
                                {
                                    "tag": tag,
                                    "category": category,
                                    "scale": scale,
                                    "comparisons": comparable[:8],
                                }
                            )
                    elif len(mismatches) < 8:
                        mismatches.append({"tag": tag, "failed": failed[:8]})

                if not eligible_tags:
                    continue
                missing = sorted(eligible_tags - matched_tags)
                if len(matched_tags) >= ORACLE_VALIDATION_MIN_TAGS and not missing:
                    status = "VALIDATED_RESOURCE_VECTOR_ORACLE_ALL_SAMPLED"
                elif len(matched_tags) >= ORACLE_VALIDATION_MIN_TAGS:
                    status = "CANDIDATE_RESOURCE_VECTOR_ORACLE_PARTIAL"
                else:
                    status = "CANDIDATE_RESOURCE_VECTOR_INSUFFICIENT_COUNTRIES"
                summaries.append(
                    {
                        "object": obj,
                        "object_offset": hx(offset),
                        "path": f"{obj}+{hx(offset)}",
                        "stride": stride,
                        "amount_off": amount_off,
                        "amount_width": amount_width,
                        "resource_off": resource_off,
                        "resource_category": category,
                        "scale": scale,
                        "matched_tag_count": len(matched_tags),
                        "eligible_nonzero_tag_count": len(eligible_tags),
                        "matched_tags": sorted(matched_tags),
                        "missing_tags": missing,
                        "examples": examples,
                        "mismatch_examples": mismatches,
                        "status": status,
                    }
                )

    status_rank = {
        "VALIDATED_RESOURCE_VECTOR_ORACLE_ALL_SAMPLED": 0,
        "CANDIDATE_RESOURCE_VECTOR_ORACLE_PARTIAL": 1,
        "CANDIDATE_RESOURCE_VECTOR_INSUFFICIENT_COUNTRIES": 2,
    }
    summaries.sort(
        key=lambda row: (
            status_rank.get(row["status"], 9),
            -row["matched_tag_count"],
            row["resource_category"],
            row["path"],
            row["stride"],
            row["scale"],
        )
    )
    validated = [
        row
        for row in summaries
        if row["status"] == "VALIDATED_RESOURCE_VECTOR_ORACLE_ALL_SAMPLED"
    ]
    return {
        "match_count": len(summaries),
        "validated_cross_country_count": len(validated),
        "matches": summaries[:64],
        "status": (
            "RESOURCE_ORACLE_VALIDATED_CANDIDATES_PRESENT"
            if validated
            else "CANDIDATES_ONLY_NO_ALL_SAMPLED_RESOURCE_MATCH"
        ),
    }


def scan_resource_vectors(
    p: Any,
    countries: list[dict[str, Any]],
    targets_by_tag: dict[str, dict[str, Any]] | None = None,
) -> dict[str, Any]:
    candidates: list[dict[str, Any]] = []
    for country in countries:
        candidates.extend(
            scan_resource_vectors_for_object(
                p,
                country,
                "CCountry",
                country.get("country_ptr"),
                COUNTRY_SCAN_START,
                COUNTRY_SCAN_END,
            )
        )
        candidates.extend(
            scan_resource_vectors_for_object(
                p,
                country,
                "CProductionManager",
                country.get("production_manager"),
                PROD_MANAGER_SCAN_START,
                PROD_MANAGER_SCAN_END,
            )
        )

    groups: dict[tuple[Any, ...], list[dict[str, Any]]] = defaultdict(list)
    for row in candidates:
        key = (
            row["object"],
            row["object_offset"],
            row["stride"],
            row["amount_off"],
            row["amount_width"],
            row["resource_off"],
        )
        groups[key].append(row)
    summaries = []
    for (obj, offset, stride, amount_off, amount_width, resource_off), rows in groups.items():
        sequences = Counter(tuple(row["resource_ids"]) for row in rows)
        tag_count = len({row["tag_db_id"] for row in rows})
        nonzero_tags = sum(1 for row in rows if row["nonzero_amount_rows"] > 0)
        duplicate_tags = sum(1 for row in rows if row["duplicate_resource_ids"] > 0)
        prior_reject = obj == "CCountry" and offset in (0x5B8, 0x1218)
        summaries.append(
            {
                "object": obj,
                "object_offset": hx(offset),
                "path": f"{obj}+{hx(offset)}",
                "stride": stride,
                "amount_off": amount_off,
                "amount_width": amount_width,
                "resource_off": resource_off,
                "tag_count": tag_count,
                "nonzero_tag_count": nonzero_tags,
                "duplicate_resource_id_tag_count": duplicate_tags,
                "distinct_resource_id_sequences": len(sequences),
                "top_resource_id_sequences": [
                    {"resource_ids": list(ids), "count": count}
                    for ids, count in sequences.most_common(5)
                ],
                "examples": rows[:8],
                "status": (
                    "REJECTED_PRIOR_EVIDENCE_DO_NOT_EMIT"
                    if prior_reject
                    else "CANDIDATE_VECTOR_SHAPE_ONLY"
                ),
            }
        )
    summaries.sort(
        key=lambda row: (
            row["status"].startswith("REJECTED"),
            -row["tag_count"],
            -row["nonzero_tag_count"],
            row["duplicate_resource_id_tag_count"],
            row["distinct_resource_id_sequences"],
            row["path"],
        )
    )
    return {
        "candidate_layout_count": len(candidates),
        "candidate_group_count": len(summaries),
        "best_groups": summaries[:32],
        "oracle_matches": summarize_resource_oracle_matches(groups, targets_by_tag or {}),
        "known_rejected_candidates": KNOWN_REJECTED_RESOURCE_CANDIDATES,
        "status": (
            "CANDIDATES_ONLY_REQUIRE_SAME_TICK_RESOURCE_ORACLE"
            if not targets_by_tag
            else "CANDIDATES_WITH_RESOURCE_ORACLE_MATCH_SUMMARY"
        ),
    }


def build_report(args: argparse.Namespace) -> tuple[dict[str, Any], int]:
    focus_tags = {tag.upper() for tag in args.focus_tags}
    sample_tag_order = [tag.upper() for tag in args.sample_tags]
    sample_tags = set(sample_tag_order)
    economy = load_json(args.economy_oracle)
    production = load_json(args.production_oracle)
    ui_values = load_json(args.ui_values)
    econ_targets = economy_targets(economy, ui_values)
    prod_targets = production_targets(production)

    p = winmem.open_process("hoi4.exe")
    if p is None:
        raise RuntimeError("hoi4.exe not running")

    with p:
        started = time.perf_counter()
        process = process_info(p)
        report: dict[str, Any] = {
            "schema": "battle-plan.country_economy_counters_live.root_probe.v1",
            "lane": DOMAIN_NAME,
            "domain_id": DOMAIN_ID,
            "probe": "probe_870ebf_country_economy_roots",
            "snapshot_label": args.snapshot_label,
            "generated_utc": datetime.now(timezone.utc).isoformat(timespec="seconds"),
            "expected_sha256": EXPECTED_SHA256,
            "process": process,
            "build_gate_ok": process.get("sha256") == EXPECTED_SHA256,
            "fail_closed": process.get("sha256") != EXPECTED_SHA256,
        }
        if not report["build_gate_ok"]:
            report["status"] = "build_gate_failed"
            return report, 2

        roots = read_roots(p)
        tags = load_tags(p, roots)
        countries, country_rejects = discover_countries(
            p,
            roots,
            tags,
            sample_tags,
            focus_tags,
            max(1, args.max_countries),
            max(1, args.min_countries),
        )
        lines: list[dict[str, Any]] = []
        for country in countries:
            lines.extend(read_production_lines(p, country))

        min_coverage = max(3, min(len(countries), args.min_countries) // 4)
        report.update(
            {
                "roots": roots,
                "tag_table": {
                    "tag_count_read": len(tags),
                    "samples": [
                        {"tag_db_id": tag_id, "tag": tag}
                        for tag_id, tag in list(sorted(tags.items()))[:16]
                    ],
                },
                "validated_anchors": {
                    "CGameState": f"module+{hx(CG_STATE_DATE_OFFSET)}",
                    "country_table": f"CGameState+{hx(CGAMESTATE_COUNTRY_TABLE)} count CGameState+{hx(CGAMESTATE_COUNTRY_COUNT)}",
                    "CCountry_vtable": f"module+{hx(COUNTRY_VTABLE_RVA)}",
                    "CCountry_tag_echo": f"CCountry+{hx(OBJ_TAG_DB_ID)}",
                    "production_manager": f"CCountry+{hx(OBJ_PROD_MANAGER)}",
                    "fuel_overlap_owned_elsewhere": f"CCountry+{hx(OBJ_FUEL_STATUS)} is domain 10 fuel_live",
                    "production_line_vector_overlap": f"CProductionManager+{hx(LINE_BEGIN)}/{hx(LINE_CAP)}/{hx(LINE_SIZE)} is domain 11 production_live",
                },
                "selected_countries": [
                    {
                        key: value
                        for key, value in country.items()
                        if key not in ("country_ptr", "production_manager", "fuel_status")
                    }
                    for country in countries
                ],
                "country_reject_samples": country_rejects,
                "overlap_notes": {
                    "fuel": "Domain 10 already owns fuel capacity/gain/use; this lane must not duplicate fuel rows.",
                    "stability_war_support": "Domain 14 politics_live owns stability and war support; economy probe records them only as oracle overlap if present.",
                    "production_lines": "Domain 11 owns base line rows; domain 58 only probes residual requested/efficiency/shortage/conversion/history fields.",
                    "market_trade": "Domain 43 owns market contracts/licensing/trade effects; domain 58 only probes non-market country aggregate resource counters.",
                    "state_resources": "Domains 23/48 own state resource deposits; domain 58 only probes country aggregate produced/imported/exported/used/deficit counters.",
                },
                "production_lines": {
                    "summary": production_line_summary(lines),
                    "residual_candidates": analyze_line_residual_candidates(
                        p, lines, prod_targets
                    ),
                    "sample_rows": [
                        {
                            key: value
                            for key, value in line.items()
                            if key not in ("_raw", "line_ptr")
                        }
                        for line in lines[:80]
                    ],
                },
                "factory_consumer_scalar_census": {
                    "country_object": scan_scalar_census(
                        p,
                        countries,
                        "country_ptr",
                        "CCountry",
                        COUNTRY_SCAN_START,
                        COUNTRY_SCAN_END,
                        min_coverage,
                        args.scalar_limit,
                    ),
                    "production_manager": scan_scalar_census(
                        p,
                        countries,
                        "production_manager",
                        "CProductionManager",
                        PROD_MANAGER_SCAN_START,
                        PROD_MANAGER_SCAN_END,
                        min_coverage,
                        args.scalar_limit,
                    ),
                    "oracle_matches": find_oracle_scalar_matches(
                        p, countries, econ_targets, sample_tag_order
                    ),
                },
                "resource_flow_vector_candidates": scan_resource_vectors(
                    p, countries, econ_targets
                ),
                "focus_scalar_samples": focus_scalar_samples(p, countries, focus_tags),
                "evidence_inputs": {
                    "economy_oracle": args.economy_oracle,
                    "production_oracle": args.production_oracle,
                    "ui_values": args.ui_values,
                    "focus_tags": sorted(focus_tags),
                    "sample_tags": sorted(sample_tags),
                },
                "promotion_rule": (
                    "All offsets in this report are CANDIDATE unless later JSON proves "
                    "same-tick save/UI equality or controlled before/after delta semantics. "
                    "Known rejected candidates remain rejected without new evidence that "
                    "directly answers the prior rejection."
                ),
                "status": "probe_completed_candidates_only",
                "elapsed_ms": round((time.perf_counter() - started) * 1000, 3),
            }
        )
        return report, 0


def write_report(path: str, report: dict[str, Any]) -> None:
    out = Path(path)
    out.parent.mkdir(parents=True, exist_ok=True)
    with out.open("w", encoding="utf-8") as handle:
        json.dump(report, handle, indent=2, sort_keys=True)
        handle.write("\n")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", required=True)
    parser.add_argument("--snapshot-label", default="single_live_snapshot")
    parser.add_argument("--focus-tags", nargs="*", default=DEFAULT_FOCUS_TAGS)
    parser.add_argument("--sample-tags", nargs="*", default=DEFAULT_SAMPLE_TAGS)
    parser.add_argument("--max-countries", type=int, default=96)
    parser.add_argument("--min-countries", type=int, default=32)
    parser.add_argument("--scalar-limit", type=int, default=80)
    parser.add_argument("--economy-oracle")
    parser.add_argument("--production-oracle")
    parser.add_argument("--ui-values")
    args = parser.parse_args(argv)

    report, code = build_report(args)
    write_report(args.out, report)
    print(
        "probe_870ebf_country_economy_roots: "
        f"status={report.get('status')} out={args.out}"
    )
    return code


if __name__ == "__main__":
    raise SystemExit(main())
