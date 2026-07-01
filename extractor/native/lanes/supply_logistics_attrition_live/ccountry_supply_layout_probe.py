"""Current-build CCountrySupplySystem layout probe for domain 44 residuals.

This lane-local probe is read-only. It validates or rejects simple live layout
paths for the unresolved country/node supply residuals:

* CCountry -> CCountrySupplySystem direct pointer join.
* CCountrySupplySystem -> CCountry reverse pointer join.
* Inline/current-object and bounded pointer-region matches for save-oracle
  motorization, disrupted-supply, and foreign-homebase node province anchors.

It does not emit compact rows. Any row emission must wait until a country/node
join and field semantics are proven by a same-frame or controlled differential.
"""

from __future__ import annotations

import argparse
import datetime as _dt
import json
import struct
import sys
import time
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional, Sequence, Set, Tuple

LANE_DIR = Path(__file__).resolve().parent
REPO_ROOT = LANE_DIR.parents[3]
sys.path.insert(0, str(LANE_DIR))
sys.path.insert(0, str(REPO_ROOT / "extractor" / "memory_reader"))

import live_probe  # noqa: E402
import winmem  # noqa: E402

EXPECTED_SHA256 = live_probe.EXPECTED_SHA256
DOMAIN_NAME = live_probe.DOMAIN_NAME
HEAP_LO = live_probe.HEAP_LO
HEAP_HI = live_probe.HEAP_HI

COUNTRY_DATABASE_OFFSET = 0x33189C8
OBJ_TAG_DB_ID = 0x08
MAX_TAG_ROWS = 2048
GS_CURRENT_DATE_OFFSET = 0x468
DATE_EPOCH_CONST = 43_799_999

CCOUNTRY_SUPPLY_VTABLE0_RVA = 0x298BBF8
CCOUNTRY_SUPPLY_VTABLE1_RVA = 0x298BC48
CCSS_SCAN_BYTES = 0x400
COUNTRY_SCAN_BYTES = 0x4000
POINTER_FIELD_SCAN_BYTES = 0x300
POINTER_REGION_SCAN_BYTES = 0x4000
MAX_HEAP_REGION_BYTES = 64 << 20
MAX_POINTER_REGIONS = 2048
MAX_REPORTED_OFFSETS = 24
MAX_REPORTED_MATCHES = 24
MAX_PROVINCE_ID = live_probe.MAX_PROVINCE_ID
PRIMARY_COUNTRY_JOIN_OFFSET = 0x78
BACKREF_POINTER_OFFSETS = {0x70, 0x78, 0x100, 0x310, 0x318, 0x3A0}

DEFAULT_SUPPLY_JSON = (
    REPO_ROOT
    / "extractor/native"
    / "runs"
    / "supply_logistics_attrition_live_save_oracle_870ebf_20260701_v1"
    / "supply.json"
)


def hx(value: Optional[int]) -> Optional[str]:
    return f"0x{value:X}" if isinstance(value, int) and value else None


def is_heap(value: Optional[int], align: int = 8) -> bool:
    return isinstance(value, int) and HEAP_LO <= value < HEAP_HI and value % align == 0


def safe_u64(p: Any, addr: int) -> Optional[int]:
    try:
        return p.u64(addr)
    except OSError:
        return None


def safe_u32(p: Any, addr: int) -> Optional[int]:
    try:
        return p.u32(addr)
    except OSError:
        return None


def read_inline_tag(p: Any, addr: int) -> Optional[str]:
    raw = p.try_read(addr, 4)
    if len(raw) < 3:
        return None
    end = raw.find(b"\0")
    if end < 0:
        end = 3
    try:
        tag = raw[:end].decode("latin-1")
    except UnicodeDecodeError:
        return None
    if not tag or any(ord(ch) < 32 or ord(ch) >= 127 for ch in tag):
        return None
    return tag


def build_gate(p: Any) -> Dict[str, Any]:
    fp = p.fingerprint()
    fp["expected_sha256"] = EXPECTED_SHA256
    fp["build_ok"] = (fp.get("sha256") or "").lower() == EXPECTED_SHA256
    return fp


def decode_datehour(stored: Optional[int]) -> Optional[str]:
    if stored is None:
        return None
    min_value = (1850 * 365 * 24) + DATE_EPOCH_CONST
    max_value = (2200 * 365 * 24) + DATE_EPOCH_CONST
    if stored < min_value or stored > max_value:
        return None
    hours = stored - DATE_EPOCH_CONST
    hour = hours % 24
    rem_days = hours // 24
    year = rem_days // 365
    doy = rem_days - year * 365
    month_lengths = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    month = 1
    for length in month_lengths:
        if doy < length:
            break
        doy -= length
        month += 1
    return f"{year}.{month}.{doy + 1}.{hour}"


def read_current_date(p: Any, game_state: Optional[int]) -> Dict[str, Any]:
    raw = safe_u32(p, int(game_state) + GS_CURRENT_DATE_OFFSET) if is_heap(game_state) else None
    signed = None if raw is None or raw > 0x7FFFFFFF else int(raw)
    return {
        "raw": signed,
        "decoded": decode_datehour(signed),
    }


def read_country_maps(p: Any) -> Dict[str, Any]:
    game_state = safe_u64(p, p.module_base + live_probe.CGAMESTATE_RVA)
    country_database = safe_u64(p, p.module_base + COUNTRY_DATABASE_OFFSET)
    tag_table = safe_u64(p, country_database) if is_heap(country_database) else None
    tag_count = safe_u32(p, country_database + 8) if is_heap(country_database) else None
    active_count = safe_u32(p, country_database + 12) if is_heap(country_database) else None
    country_table = (
        safe_u64(p, game_state + live_probe.CGAMESTATE_COUNTRY_TABLE)
        if is_heap(game_state)
        else None
    )
    country_count = (
        safe_u32(p, game_state + live_probe.CGAMESTATE_COUNTRY_COUNT)
        if is_heap(game_state)
        else None
    )

    id_to_tag: Dict[int, str] = {}
    id_to_country: Dict[int, int] = {}
    country_vtable = p.module_base + live_probe.COUNTRY_VTABLE_RVA
    rows = min(tag_count or 0, active_count or 0, MAX_TAG_ROWS)
    if is_heap(tag_table):
        for tag_id in range(rows):
            tag = read_inline_tag(p, tag_table + tag_id * 32)
            if tag:
                id_to_tag[tag_id] = tag
    if is_heap(country_table):
        limit = min(country_count or rows, rows or MAX_TAG_ROWS, MAX_TAG_ROWS)
        for tag_id in range(1, limit):
            country = safe_u64(p, country_table + tag_id * 8)
            if (
                is_heap(country)
                and safe_u64(p, country) == country_vtable
                and safe_u32(p, country + OBJ_TAG_DB_ID) == tag_id
            ):
                id_to_country[tag_id] = int(country)

    return {
        "game_state": game_state,
        "country_database": country_database,
        "tag_table": tag_table,
        "tag_count": tag_count,
        "active_count": active_count,
        "country_table": country_table,
        "country_count": country_count,
        "id_to_tag": id_to_tag,
        "id_to_country": id_to_country,
        "tag_to_id": {tag: tag_id for tag_id, tag in id_to_tag.items()},
    }


def scan_ccountry_supply_objects(p: Any) -> List[int]:
    vtable0 = p.module_base + CCOUNTRY_SUPPLY_VTABLE0_RVA
    vtable1 = p.module_base + CCOUNTRY_SUPPLY_VTABLE1_RVA
    needle = struct.pack("<Q", vtable0)
    objects: Set[int] = set()
    chunk = 64 << 20
    overlap = 16
    for base, size in p.iter_regions(HEAP_LO, HEAP_HI, want_exec=False):
        if size > MAX_HEAP_REGION_BYTES:
            pos = 0
            while pos < size:
                want = min(chunk, size - pos)
                blob = p.try_read(base + pos, want)
                if blob:
                    add_ccss_hits_from_blob(objects, blob, base + pos, needle, vtable1)
                    got = len(blob)
                    if pos + got >= size or got < want:
                        pos += max(got, 0x1000)
                    else:
                        pos += got - overlap
                else:
                    pos += max(want, 0x1000)
            continue
        blob = p.try_read(base, size)
        if blob:
            add_ccss_hits_from_blob(objects, blob, base, needle, vtable1)
    return sorted(objects)


def add_ccss_hits_from_blob(
    objects: Set[int],
    blob: bytes,
    base: int,
    needle: bytes,
    vtable1: int,
) -> None:
    search = 0
    while True:
        found = blob.find(needle, search)
        if found < 0:
            break
        search = found + 8
        if found % 8 != 0 or found + 16 > len(blob):
            continue
        second = struct.unpack_from("<Q", blob, found + 8)[0]
        if second == vtable1:
            objects.add(base + found)


def load_supply_anchors(path: Path) -> Dict[str, Any]:
    data = json.loads(path.read_text(encoding="utf-8"))
    countries = data.get("countries") or {}
    motorization: List[Dict[str, Any]] = []
    disrupted: List[Dict[str, Any]] = []
    foreign_homebase: List[Dict[str, Any]] = []
    truck_loss_tags: List[str] = []
    for tag, rec in countries.items():
        for item in rec.get("node_motorization") or []:
            motorization.append({"tag": tag, **item})
        for item in rec.get("disrupted_supply") or []:
            disrupted.append({"tag": tag, **item})
        for item in rec.get("foreign_homebase_nodes") or []:
            foreign_homebase.append({"tag": tag, **item})
        if rec.get("lost_trucks_attrition_total") or rec.get("lost_trucks_killed_total"):
            truck_loss_tags.append(tag)

    motor_nodes = {
        int(item["node_province"])
        for item in motorization
        if isinstance(item.get("node_province"), int)
    }
    disrupted_nodes = {
        int(item["node_province"])
        for item in disrupted
        if isinstance(item.get("node_province"), int)
    }
    foreign_nodes = {
        int(item["province"])
        for item in foreign_homebase
        if isinstance(item.get("province"), int)
    }
    all_nodes = {value for value in (motor_nodes | disrupted_nodes | foreign_nodes) if 0 < value <= MAX_PROVINCE_ID}
    motor_levels = Counter()
    by_tag: Dict[str, Dict[str, Set[int]]] = defaultdict(lambda: {
        "motorization": set(),
        "disrupted": set(),
        "foreign_homebase": set(),
    })
    for item in motorization:
        node = item.get("node_province")
        tag = item.get("tag")
        if isinstance(tag, str) and isinstance(node, int) and 0 < node <= MAX_PROVINCE_ID:
            by_tag[tag]["motorization"].add(int(node))
        levels = item.get("levels") or {}
        for level in levels.values():
            if isinstance(level, int):
                motor_levels[level] += 1
    for item in disrupted:
        node = item.get("node_province")
        tag = item.get("tag")
        if isinstance(tag, str) and isinstance(node, int) and 0 < node <= MAX_PROVINCE_ID:
            by_tag[tag]["disrupted"].add(int(node))
    for item in foreign_homebase:
        node = item.get("province")
        tag = item.get("tag")
        if isinstance(tag, str) and isinstance(node, int) and 0 < node <= MAX_PROVINCE_ID:
            by_tag[tag]["foreign_homebase"].add(int(node))

    return {
        "path": str(path),
        "save_date": data.get("meta", {}).get("save_date"),
        "player": data.get("meta", {}).get("player"),
        "motorization": motorization,
        "disrupted": disrupted,
        "foreign_homebase": foreign_homebase,
        "truck_loss_tags": sorted(set(truck_loss_tags)),
        "motor_nodes": sorted(motor_nodes),
        "disrupted_nodes": sorted(disrupted_nodes),
        "foreign_nodes": sorted(foreign_nodes),
        "all_nodes": sorted(all_nodes),
        "motor_levels": dict(sorted(motor_levels.items())),
        "by_tag": {
            tag: {
                category: sorted(values)
                for category, values in categories.items()
                if values
            }
            for tag, categories in sorted(by_tag.items())
        },
    }


def scan_qword_set(blob: bytes, values: Set[int]) -> List[Tuple[int, int]]:
    out: List[Tuple[int, int]] = []
    for off in range(0, max(0, len(blob) - 7), 8):
        value = struct.unpack_from("<Q", blob, off)[0]
        if value in values:
            out.append((off, value))
    return out


def scan_u32_anchors(blob: bytes, anchors: Set[int]) -> List[Tuple[int, int]]:
    out: List[Tuple[int, int]] = []
    if not anchors:
        return out
    for off in range(0, max(0, len(blob) - 3), 4):
        value = struct.unpack_from("<I", blob, off)[0]
        if value in anchors:
            out.append((off, value))
    return out


def country_to_ccss_scan(
    p: Any,
    countries: Dict[int, int],
    id_to_tag: Dict[int, str],
    ccss_objects: Set[int],
) -> Dict[str, Any]:
    offset_counts: Counter[int] = Counter()
    matches: List[Dict[str, Any]] = []
    for tag_id, country in sorted(countries.items()):
        blob = p.try_read(country, COUNTRY_SCAN_BYTES)
        if len(blob) < 8:
            continue
        for off, value in scan_qword_set(blob, ccss_objects):
            offset_counts[off] += 1
            if len(matches) < MAX_REPORTED_MATCHES:
                matches.append({
                    "tag_db_id": tag_id,
                    "tag": id_to_tag.get(tag_id),
                    "country": hx(country),
                    "offset": f"+0x{off:X}",
                    "ccountry_supply_system": hx(value),
                })
    return {
        "scan_bytes_per_country": COUNTRY_SCAN_BYTES,
        "countries_scanned": len(countries),
        "offset_counts": [
            {"offset": f"+0x{off:X}", "matches": count}
            for off, count in offset_counts.most_common(MAX_REPORTED_OFFSETS)
        ],
        "matches": matches,
        "verdict": "direct_country_pointer_join_found" if offset_counts else "rejected_no_direct_country_pointer_join",
    }


def ccss_to_country_scan(
    p: Any,
    ccss_objects: Sequence[int],
    country_to_tag: Dict[int, Tuple[int, str]],
) -> Dict[str, Any]:
    country_ptrs = set(country_to_tag)
    offset_counts: Counter[int] = Counter()
    matches: List[Dict[str, Any]] = []
    for obj in ccss_objects:
        blob = p.try_read(obj, CCSS_SCAN_BYTES)
        if len(blob) < 8:
            continue
        for off, value in scan_qword_set(blob, country_ptrs):
            offset_counts[off] += 1
            tag_id, tag = country_to_tag[value]
            if len(matches) < MAX_REPORTED_MATCHES:
                matches.append({
                    "ccountry_supply_system": hx(obj),
                    "offset": f"+0x{off:X}",
                    "country": hx(value),
                    "tag_db_id": tag_id,
                    "tag": tag,
                })
    return {
        "scan_bytes_per_object": CCSS_SCAN_BYTES,
        "objects_scanned": len(ccss_objects),
        "offset_counts": [
            {"offset": f"+0x{off:X}", "matches": count}
            for off, count in offset_counts.most_common(MAX_REPORTED_OFFSETS)
        ],
        "matches": matches,
        "verdict": "ccss_country_backpointer_found" if offset_counts else "rejected_no_ccss_country_backpointer",
    }


def inline_anchor_scan(
    p: Any,
    ccss_objects: Sequence[int],
    anchors: Dict[str, Any],
) -> Dict[str, Any]:
    all_nodes = set(anchors["all_nodes"])
    motor_nodes = set(anchors["motor_nodes"])
    disrupted_nodes = set(anchors["disrupted_nodes"])
    foreign_nodes = set(anchors["foreign_nodes"])
    offset_counts: Counter[int] = Counter()
    category_counts: Dict[int, Counter[str]] = defaultdict(Counter)
    matches: List[Dict[str, Any]] = []
    for obj in ccss_objects:
        blob = p.try_read(obj, CCSS_SCAN_BYTES)
        if len(blob) < 4:
            continue
        for off, value in scan_u32_anchors(blob, all_nodes):
            offset_counts[off] += 1
            cats = []
            if value in motor_nodes:
                cats.append("motorization")
            if value in disrupted_nodes:
                cats.append("disrupted")
            if value in foreign_nodes:
                cats.append("foreign_homebase")
            for cat in cats:
                category_counts[off][cat] += 1
            if len(matches) < MAX_REPORTED_MATCHES:
                matches.append({
                    "ccountry_supply_system": hx(obj),
                    "offset": f"+0x{off:X}",
                    "province": value,
                    "categories": cats,
                })
    return {
        "scan_bytes_per_object": CCSS_SCAN_BYTES,
        "objects_scanned": len(ccss_objects),
        "offset_counts": [
            {
                "offset": f"+0x{off:X}",
                "matches": count,
                "categories": dict(category_counts.get(off, {})),
            }
            for off, count in offset_counts.most_common(MAX_REPORTED_OFFSETS)
        ],
        "matches": matches,
        "verdict": "inline_anchor_candidates_found" if offset_counts else "rejected_no_inline_anchor_values",
    }


def pointer_region_anchor_scan(
    p: Any,
    ccss_objects: Sequence[int],
    anchors: Dict[str, Any],
) -> Dict[str, Any]:
    all_nodes = set(anchors["all_nodes"])
    motor_nodes = set(anchors["motor_nodes"])
    disrupted_nodes = set(anchors["disrupted_nodes"])
    foreign_nodes = set(anchors["foreign_nodes"])
    field_candidates: List[Tuple[int, int, int]] = []
    seen_regions: Set[int] = set()
    for obj in ccss_objects:
        blob = p.try_read(obj, POINTER_FIELD_SCAN_BYTES)
        if len(blob) < 8:
            continue
        for off in range(0x10, min(len(blob) - 7, POINTER_FIELD_SCAN_BYTES), 8):
            ptr = struct.unpack_from("<Q", blob, off)[0]
            if not is_heap(ptr) or ptr in seen_regions:
                continue
            seen_regions.add(ptr)
            field_candidates.append((obj, off, ptr))
            if len(field_candidates) >= MAX_POINTER_REGIONS:
                break
        if len(field_candidates) >= MAX_POINTER_REGIONS:
            break

    field_counts: Counter[int] = Counter()
    category_counts: Dict[int, Counter[str]] = defaultdict(Counter)
    matches: List[Dict[str, Any]] = []
    regions_read = 0
    regions_with_matches = 0
    for obj, field_off, ptr in field_candidates:
        blob = p.try_read(ptr, POINTER_REGION_SCAN_BYTES)
        if len(blob) < 4:
            continue
        regions_read += 1
        local_matches = scan_u32_anchors(blob, all_nodes)
        if not local_matches:
            continue
        regions_with_matches += 1
        field_counts[field_off] += len(local_matches)
        for inner_off, value in local_matches:
            cats = []
            if value in motor_nodes:
                cats.append("motorization")
            if value in disrupted_nodes:
                cats.append("disrupted")
            if value in foreign_nodes:
                cats.append("foreign_homebase")
            for cat in cats:
                category_counts[field_off][cat] += 1
            if len(matches) < MAX_REPORTED_MATCHES:
                matches.append({
                    "ccountry_supply_system": hx(obj),
                    "field_offset": f"+0x{field_off:X}",
                    "region": hx(ptr),
                    "region_offset": f"+0x{inner_off:X}",
                    "province": value,
                    "categories": cats,
                })
    return {
        "object_pointer_scan_bytes": POINTER_FIELD_SCAN_BYTES,
        "region_scan_bytes": POINTER_REGION_SCAN_BYTES,
        "unique_pointer_regions_considered": len(field_candidates),
        "regions_read": regions_read,
        "regions_with_anchor_matches": regions_with_matches,
        "field_offset_counts": [
            {
                "field_offset": f"+0x{off:X}",
                "matches": count,
                "categories": dict(category_counts.get(off, {})),
            }
            for off, count in field_counts.most_common(MAX_REPORTED_OFFSETS)
        ],
        "matches": matches,
        "verdict": (
            "pointer_region_anchor_candidates_found"
            if field_counts
            else "rejected_no_pointer_region_anchor_values"
        ),
    }


def tag_specific_anchor_scan(
    p: Any,
    ccss_objects: Sequence[int],
    country_to_tag: Dict[int, Tuple[int, str]],
    country_ptrs: Set[int],
    anchors: Dict[str, Any],
    csupply_object: Optional[int],
) -> Dict[str, Any]:
    by_tag = anchors.get("by_tag", {})
    inline_counts: Counter[int] = Counter()
    inline_categories: Dict[int, Counter[str]] = defaultdict(Counter)
    pointer_counts: Counter[int] = Counter()
    pointer_categories: Dict[int, Counter[str]] = defaultdict(Counter)
    inline_matches: List[Dict[str, Any]] = []
    pointer_matches: List[Dict[str, Any]] = []
    joined_objects = 0
    joined_tags: Counter[str] = Counter()
    joined_anchor_tags: Counter[str] = Counter()
    candidate_regions = 0
    regions_with_matches = 0

    country_vtable = p.module_base + live_probe.COUNTRY_VTABLE_RVA
    for obj in ccss_objects:
        country_ptr = safe_u64(p, obj + PRIMARY_COUNTRY_JOIN_OFFSET)
        if country_ptr not in country_to_tag:
            continue
        tag_id, tag = country_to_tag[country_ptr]
        joined_objects += 1
        joined_tags[tag] += 1
        tag_anchors = by_tag.get(tag) or {}
        if not tag_anchors:
            continue
        joined_anchor_tags[tag] += 1
        anchor_set: Set[int] = set()
        for values in tag_anchors.values():
            anchor_set.update(int(value) for value in values)
        if not anchor_set:
            continue

        body = p.try_read(obj, CCSS_SCAN_BYTES)
        if len(body) >= 4:
            for off, value in scan_u32_anchors(body, anchor_set):
                cats = [
                    category
                    for category, values in tag_anchors.items()
                    if value in set(values)
                ]
                inline_counts[off] += 1
                for cat in cats:
                    inline_categories[off][cat] += 1
                if len(inline_matches) < MAX_REPORTED_MATCHES:
                    inline_matches.append({
                        "ccountry_supply_system": hx(obj),
                        "tag_db_id": tag_id,
                        "tag": tag,
                        "offset": f"+0x{off:X}",
                        "province": value,
                        "categories": cats,
                    })

        for field_off in range(0x10, min(len(body) - 7, POINTER_FIELD_SCAN_BYTES), 8):
            if field_off in BACKREF_POINTER_OFFSETS:
                continue
            ptr = struct.unpack_from("<Q", body, field_off)[0]
            if not is_heap(ptr) or ptr == csupply_object or ptr in country_ptrs:
                continue
            ptr_vtable = safe_u64(p, ptr)
            if ptr_vtable == country_vtable:
                continue
            blob = p.try_read(ptr, POINTER_REGION_SCAN_BYTES)
            if len(blob) < 4:
                continue
            candidate_regions += 1
            local_matches = scan_u32_anchors(blob, anchor_set)
            if not local_matches:
                continue
            regions_with_matches += 1
            for inner_off, value in local_matches:
                cats = [
                    category
                    for category, values in tag_anchors.items()
                    if value in set(values)
                ]
                pointer_counts[field_off] += 1
                for cat in cats:
                    pointer_categories[field_off][cat] += 1
                if len(pointer_matches) < MAX_REPORTED_MATCHES:
                    pointer_matches.append({
                        "ccountry_supply_system": hx(obj),
                        "tag_db_id": tag_id,
                        "tag": tag,
                        "field_offset": f"+0x{field_off:X}",
                        "region": hx(ptr),
                        "region_offset": f"+0x{inner_off:X}",
                        "province": value,
                        "categories": cats,
                    })

    return {
        "primary_join_offset": f"+0x{PRIMARY_COUNTRY_JOIN_OFFSET:X}",
        "joined_objects": joined_objects,
        "joined_tags": len(joined_tags),
        "joined_tags_with_save_anchors": len(joined_anchor_tags),
        "candidate_pointer_regions_read": candidate_regions,
        "candidate_pointer_regions_with_matches": regions_with_matches,
        "inline_offset_counts": [
            {
                "offset": f"+0x{off:X}",
                "matches": count,
                "categories": dict(inline_categories.get(off, {})),
            }
            for off, count in inline_counts.most_common(MAX_REPORTED_OFFSETS)
        ],
        "pointer_field_offset_counts": [
            {
                "field_offset": f"+0x{off:X}",
                "matches": count,
                "categories": dict(pointer_categories.get(off, {})),
            }
            for off, count in pointer_counts.most_common(MAX_REPORTED_OFFSETS)
        ],
        "inline_matches": inline_matches,
        "pointer_matches": pointer_matches,
        "verdict": (
            "tag_specific_anchor_candidates_found"
            if inline_counts or pointer_counts
            else "rejected_no_tag_specific_anchor_candidates"
        ),
    }


def sample_ccss_fields(p: Any, ccss_objects: Sequence[int], csupply_object: Optional[int]) -> List[Dict[str, Any]]:
    samples: List[Dict[str, Any]] = []
    for obj in ccss_objects[:12]:
        row: Dict[str, Any] = {"object": hx(obj)}
        try:
            row["vtable0"] = f"module+0x{p.u64(obj) - p.module_base:X}"
            row["vtable1"] = f"module+0x{p.u64(obj + 8) - p.module_base:X}"
        except OSError as exc:
            row["vtable_error"] = str(exc)
        for off in (0x18, 0x20, 0x30, 0x40, 0x58, 0x60, 0x70, 0x78, 0x80, 0x88, 0x90, 0x98, 0xA0, 0xC0, 0x130, 0x140):
            value = safe_u64(p, obj + off)
            row[f"qword_0x{off:X}"] = hx(value) if is_heap(value) else value
        if csupply_object:
            csupply_offsets = []
            blob = p.try_read(obj, CCSS_SCAN_BYTES)
            for off, value in scan_qword_set(blob, {csupply_object}):
                csupply_offsets.append(f"+0x{off:X}")
            row["csupply_backptr_offsets"] = csupply_offsets
        samples.append(row)
    return samples


def find_csupply_object(p: Any) -> Optional[int]:
    v0 = p.module_base + live_probe.CSUPPLYSYSTEM_VTABLE0_RVA
    v1 = p.module_base + live_probe.CSUPPLYSYSTEM_VTABLE1_RVA
    needle = struct.pack("<Q", v0)
    for base, size in p.iter_regions(HEAP_LO, HEAP_HI, want_exec=False):
        if size > MAX_HEAP_REGION_BYTES:
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
            if found + 16 <= len(blob) and struct.unpack_from("<Q", blob, found + 8)[0] == v1:
                return base + found
    return None


def _write_outputs(out_dir: Path, report: Dict[str, Any]) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "ccountry_supply_layout_probe.json").write_text(
        json.dumps(report, indent=2),
        encoding="utf-8",
    )
    summary = report.get("summary", {})
    lines = [
        "# supply_logistics_attrition_live CCountrySupplySystem layout probe",
        "",
        f"- status: `{report.get('status')}`",
        f"- exe_sha256: `{report.get('process', {}).get('sha256')}`",
        f"- ccss_objects: `{summary.get('ccountry_supply_system_objects')}`",
        f"- live_countries: `{summary.get('live_countries')}`",
        f"- motorization_anchor_nodes: `{summary.get('motorization_anchor_nodes')}`",
        f"- disrupted_anchor_nodes: `{summary.get('disrupted_anchor_nodes')}`",
        f"- foreign_homebase_anchor_nodes: `{summary.get('foreign_homebase_anchor_nodes')}`",
        "",
        "## Join Scans",
        "",
        (
            f"- CCountry -> CCountrySupplySystem: "
            f"`{report.get('country_to_ccss_scan', {}).get('verdict')}`"
        ),
        (
            f"- CCountrySupplySystem -> CCountry: "
            f"`{report.get('ccss_to_country_scan', {}).get('verdict')}`"
        ),
        "",
        "## Anchor Scans",
        "",
        f"- Inline object anchors: `{report.get('inline_anchor_scan', {}).get('verdict')}`",
        (
            f"- Pointer-region anchors: "
            f"`{report.get('pointer_region_anchor_scan', {}).get('verdict')}`"
        ),
        (
            f"- Tag-specific anchors via `+0x78`: "
            f"`{report.get('tag_specific_anchor_scan', {}).get('verdict')}`"
        ),
        "",
        "## Offset Counts",
        "",
    ]
    for title, key, field in (
        ("Country direct join", "country_to_ccss_scan", "offset_counts"),
        ("CCSS country backpointer", "ccss_to_country_scan", "offset_counts"),
        ("Inline anchors", "inline_anchor_scan", "offset_counts"),
        ("Pointer-region anchors", "pointer_region_anchor_scan", "field_offset_counts"),
        ("Tag-specific inline anchors", "tag_specific_anchor_scan", "inline_offset_counts"),
        ("Tag-specific pointer anchors", "tag_specific_anchor_scan", "pointer_field_offset_counts"),
    ):
        lines.extend([f"### {title}", ""])
        rows = report.get(key, {}).get(field, [])
        if rows:
            for row in rows[:12]:
                label = row.get("offset") or row.get("field_offset")
                cats = row.get("categories")
                cat_text = f", categories `{cats}`" if cats else ""
                lines.append(f"- `{label}`: `{row.get('matches')}` matches{cat_text}.")
        else:
            lines.append("- no matches")
        lines.append("")
    lines.extend([
        "## Verdict",
        "",
        "- This proof does not emit motorization or pressure rows. A direct live country join or field-level anchor is required before compact production capture.",
        "- Rejected joins are production-useful: future agents should not retry direct `CCountry` body pointer paths for this build unless the fixture changes.",
    ])
    (out_dir / "CCOUNTRY_SUPPLY_LAYOUT_PROBE.md").write_text(
        "\n".join(lines) + "\n",
        encoding="utf-8",
    )


def run(pid: Optional[int], supply_json: Path, out_dir: Path) -> Dict[str, Any]:
    started = time.time()
    p = winmem.Process(pid, "hoi4.exe") if pid is not None else winmem.open_process("hoi4.exe")
    report: Dict[str, Any] = {
        "lane": DOMAIN_NAME,
        "status": "fail_closed",
        "read_only": True,
        "writes_to_game_memory": False,
        "supply_json": str(supply_json),
    }
    if p is None:
        report["reason"] = "hoi4.exe not running or cannot be opened"
        _write_outputs(out_dir, report)
        return report
    p.__enter__()
    try:
        fp = build_gate(p)
        report["process"] = fp
        if not fp.get("build_ok"):
            report["reason"] = "build mismatch; offsets are build-specific"
            _write_outputs(out_dir, report)
            return report
        anchors = load_supply_anchors(supply_json)
        countries = read_country_maps(p)
        current_date = read_current_date(p, countries.get("game_state"))
        id_to_country = countries["id_to_country"]
        id_to_tag = countries["id_to_tag"]
        country_to_tag = {
            country: (tag_id, id_to_tag.get(tag_id, ""))
            for tag_id, country in id_to_country.items()
        }
        ccss_objects = scan_ccountry_supply_objects(p)
        ccss_set = set(ccss_objects)
        csupply_object = find_csupply_object(p)

        country_join = country_to_ccss_scan(p, id_to_country, id_to_tag, ccss_set)
        reverse_join = ccss_to_country_scan(p, ccss_objects, country_to_tag)
        inline_scan = inline_anchor_scan(p, ccss_objects, anchors)
        pointer_scan = pointer_region_anchor_scan(p, ccss_objects, anchors)
        tag_specific_scan = tag_specific_anchor_scan(
            p,
            ccss_objects,
            country_to_tag,
            set(country_to_tag),
            anchors,
            csupply_object,
        )

        narrowed = any(
            item.get("verdict") in (
                "direct_country_pointer_join_found",
                "ccss_country_backpointer_found",
                "inline_anchor_candidates_found",
                "pointer_region_anchor_candidates_found",
                "tag_specific_anchor_candidates_found",
            )
            for item in (country_join, reverse_join, inline_scan, pointer_scan, tag_specific_scan)
        )
        rejected_paths = [
            item.get("verdict")
            for item in (country_join, reverse_join, inline_scan, pointer_scan, tag_specific_scan)
            if str(item.get("verdict", "")).startswith("rejected_")
        ]
        report.update({
            "status": "ccountry_supply_layout_narrowed" if narrowed else "ccountry_supply_layout_rejected_simple_paths",
            "summary": {
                "ccountry_supply_system_objects": len(ccss_objects),
                "ccountry_supply_vtables": [
                    f"module+0x{CCOUNTRY_SUPPLY_VTABLE0_RVA:X}",
                    f"module+0x{CCOUNTRY_SUPPLY_VTABLE1_RVA:X}",
                ],
                "csupply_object": hx(csupply_object),
                "live_countries": len(id_to_country),
                "tag_rows": len(id_to_tag),
                "save_date": anchors.get("save_date"),
                "current_live_date": current_date.get("decoded"),
                "current_live_date_raw": current_date.get("raw"),
                "save_player": anchors.get("player"),
                "motorization_anchor_nodes": len(anchors["motor_nodes"]),
                "disrupted_anchor_nodes": len(anchors["disrupted_nodes"]),
                "foreign_homebase_anchor_nodes": len(anchors["foreign_nodes"]),
                "all_anchor_nodes": len(anchors["all_nodes"]),
                "motorization_entries": len(anchors["motorization"]),
                "disrupted_entries": len(anchors["disrupted"]),
                "foreign_homebase_entries": len(anchors["foreign_homebase"]),
                "rejected_simple_paths": rejected_paths,
                "production_safe_fields": 0,
            },
            "anchors": {
                "path": anchors["path"],
                "save_date": anchors["save_date"],
                "player": anchors["player"],
                "motorization_entry_count": len(anchors["motorization"]),
                "disrupted_entry_count": len(anchors["disrupted"]),
                "foreign_homebase_entry_count": len(anchors["foreign_homebase"]),
                "motorization_nodes_sample": anchors["motor_nodes"][:24],
                "disrupted_nodes_sample": anchors["disrupted_nodes"][:24],
                "foreign_homebase_nodes_sample": anchors["foreign_nodes"][:24],
                "motor_levels": anchors["motor_levels"],
            },
            "country_map": {
                "game_state": hx(countries["game_state"]),
                "country_database": hx(countries["country_database"]),
                "tag_table": hx(countries["tag_table"]),
                "tag_count": countries["tag_count"],
                "active_count": countries["active_count"],
                "country_table": hx(countries["country_table"]),
                "country_count": countries["country_count"],
                "sample_tags": [
                    {
                        "tag_db_id": tag_id,
                        "tag": id_to_tag.get(tag_id),
                        "country": hx(id_to_country.get(tag_id)),
                    }
                    for tag_id in sorted(id_to_country)[:24]
                ],
            },
            "sample_ccountry_supply_fields": sample_ccss_fields(p, ccss_objects, csupply_object),
            "country_to_ccss_scan": country_join,
            "ccss_to_country_scan": reverse_join,
            "inline_anchor_scan": inline_scan,
            "pointer_region_anchor_scan": pointer_scan,
            "tag_specific_anchor_scan": tag_specific_scan,
        })
    except Exception as exc:  # noqa: BLE001
        report["status"] = "probe_failed"
        report["reason"] = str(exc)
    finally:
        p.__exit__()
    report["elapsed_s"] = round(time.time() - started, 3)
    _write_outputs(out_dir, report)
    return report


def default_out_dir() -> Path:
    stamp = _dt.datetime.now().strftime("%Y%m%d_%H%M%S")
    return REPO_ROOT / "extractor/native" / "runs" / f"supply_logistics_attrition_live_ccountry_layout_{stamp}"


def main(argv: Optional[Iterable[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pid", type=int, default=None, help="hoi4.exe pid; defaults to process name lookup")
    parser.add_argument("--supply-json", type=Path, default=DEFAULT_SUPPLY_JSON, help="save-oracle supply.json anchors")
    parser.add_argument("--out-dir", type=Path, default=None, help="proof output directory")
    args = parser.parse_args(list(argv) if argv is not None else None)
    out_dir = args.out_dir or default_out_dir()
    report = run(args.pid, args.supply_json, out_dir)
    print(json.dumps({
        "status": report.get("status"),
        "out_dir": str(out_dir),
        "ccountry_supply_system_objects": report.get("summary", {}).get("ccountry_supply_system_objects"),
        "live_countries": report.get("summary", {}).get("live_countries"),
        "country_join": report.get("country_to_ccss_scan", {}).get("verdict"),
        "reverse_join": report.get("ccss_to_country_scan", {}).get("verdict"),
        "inline_anchor_scan": report.get("inline_anchor_scan", {}).get("verdict"),
        "pointer_region_anchor_scan": report.get("pointer_region_anchor_scan", {}).get("verdict"),
        "tag_specific_anchor_scan": report.get("tag_specific_anchor_scan", {}).get("verdict"),
        "elapsed_s": report.get("elapsed_s"),
    }, indent=2))
    return 0 if report.get("status") in (
        "ccountry_supply_layout_narrowed",
        "ccountry_supply_layout_rejected_simple_paths",
    ) else 1


if __name__ == "__main__":
    raise SystemExit(main())
