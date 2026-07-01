"""Probe residual full-air-combat routes on 870ebf.

This read-only lane probe tests the remaining production-relevant roots for
sortie/combat/loss attribution beyond the accepted strength-delta slice:

* StrategicAirManager-owned roots;
* active air-region-owned objects;
* air-base manager records;
* wing/pool/body mission structures;
* indirect vector backing arrays;
* POD-like record arrays with owner/wing/region identities.

It intentionally stays out of shared collector entrypoints. The output is a
proof artifact for narrowing or blocking residual routes, not production capture
code.
"""

from __future__ import annotations

import argparse
import collections
import datetime as dt
import json
import struct
import sys
from pathlib import Path
from typing import Any

THIS_DIR = Path(__file__).resolve().parent
REPO_ROOT = THIS_DIR.parents[3]
MEMORY_READER = REPO_ROOT / "extractor" / "memory_reader"
for path in (str(REPO_ROOT), str(MEMORY_READER)):
    if path not in sys.path:
        sys.path.insert(0, path)

from memory_reader.naval_air.air_combat_losses_probe_870ebf import (  # noqa: E402
    AIR_BASE_RECORD_VTABLE_RVA,
    AIR_WING_OBJECT_VTABLE_RVA,
    AIR_WING_POOL_VTABLE_RVA,
    AIR_WING_REGION_ALT_VTABLE_RVA,
    AIR_WING_REGION_OFF_ID,
    AIR_WING_REGION_VTABLE_RVA,
    AIR_WING_VTABLE_RVA,
    AIR_WING_OFF_MISSION_REGION,
    AIR_WING_TYPE,
    CGAMESTATE_OFF_STRATEGIC_AIR_MANAGER,
    CG_STATE_DATE_OFFSET,
    Probe,
    hx,
    is_heap,
    qwords,
    read_u32,
    read_u64,
)

STRATEGIC_AIR_MANAGER_OFF_AIR_BASE_STATE_TABLE = 0x90
AIR_WING_OBJECT_HANDLE_TYPE = AIR_WING_TYPE
MAX_VECTOR_RECORDS = 4096
MAX_VECTOR_BYTES = 4 * 1024 * 1024
MAX_ONE_HOP_OBJECTS = 2048
U32_SCAN_STEP = 4
QWORD_SCAN_STEP = 8


def utc_now() -> str:
    return dt.datetime.utcnow().replace(microsecond=0).isoformat() + "Z"


def read_u32_from(blob: bytes, off: int) -> int | None:
    if off < 0 or off + 4 > len(blob):
        return None
    return struct.unpack_from("<I", blob, off)[0]


def read_u64_from(blob: bytes, off: int) -> int | None:
    if off < 0 or off + 8 > len(blob):
        return None
    return struct.unpack_from("<Q", blob, off)[0]


def compact_counter(counter: collections.Counter[Any], limit: int = 20) -> list[dict[str, Any]]:
    return [{"key": str(key), "count": count} for key, count in counter.most_common(limit)]


def stable_vector_headers(proc, addr: int, max_records: int) -> list[dict[str, Any]]:
    """Return both known header interpretations when stable and sane.

    the game containers in this codebase use both documented naming styles around
    the same 16-byte memory shape. Accepting both prevents a false rejection
    when a root uses {begin,size,cap} while another uses {begin,cap,size}.
    """
    first = proc.try_read(addr, 0x10)
    second = proc.try_read(addr, 0x10)
    if len(first) != 0x10 or first != second:
        return []
    begin = read_u64_from(first, 0)
    a = read_u32_from(first, 0x08)
    b = read_u32_from(first, 0x0C)
    if begin is None or a is None or b is None or not is_heap(begin):
        return []
    out = []
    for style, size, cap in (
        ("begin_size_cap", a, b),
        ("begin_cap_size", b, a),
    ):
        if size == 0 or size > cap or size > max_records or cap > max_records:
            continue
        out.append(
            {
                "style": style,
                "begin": begin,
                "size": size,
                "cap": cap,
            }
        )
    return out


class ResidualRouteMatrixProbe(Probe):
    def mission_region_ptr(self, body: int) -> tuple[int | None, int | None]:
        region = read_u64(self.proc, body + AIR_WING_OFF_MISSION_REGION)
        if not is_heap(region):
            return None, None
        if read_u64(self.proc, region) != self.base + AIR_WING_REGION_VTABLE_RVA:
            return None, None
        if read_u64(self.proc, region + 8) != self.base + AIR_WING_REGION_ALT_VTABLE_RVA:
            return None, None
        region_id = read_u32(self.proc, region + AIR_WING_REGION_OFF_ID)
        return region, region_id

    def airbase_record_sources(self, strategic_air: int, max_records: int) -> tuple[list[dict[str, Any]], dict[str, Any]]:
        table = strategic_air + STRATEGIC_AIR_MANAGER_OFF_AIR_BASE_STATE_TABLE
        headers = stable_vector_headers(self.proc, table, max_records)
        summary = {
            "table_addr": hx(table),
            "headers": [
                {
                    "style": header["style"],
                    "begin": hx(header["begin"]),
                    "size": header["size"],
                    "cap": header["cap"],
                }
                for header in headers
            ],
            "record_ptrs_seen": 0,
            "valid_record_ptrs": 0,
        }
        sources: list[dict[str, Any]] = []
        record_vtable = self.base + AIR_BASE_RECORD_VTABLE_RVA
        seen = set()
        for header in headers:
            byte_len = min(header["size"], max_records) * 8
            raw = self.proc.try_read(header["begin"], byte_len)
            if len(raw) != byte_len:
                continue
            for off, ptr in qwords(raw):
                summary["record_ptrs_seen"] += 1
                if ptr in seen or not is_heap(ptr):
                    continue
                seen.add(ptr)
                if read_u64(self.proc, ptr) != record_vtable:
                    continue
                summary["valid_record_ptrs"] += 1
                sources.append(
                    {
                        "kind": "air_base_record",
                        "addr": ptr,
                        "span": 0x120,
                        "via": "StrategicAirManager+0x90",
                        "table_field_off": hex(STRATEGIC_AIR_MANAGER_OFF_AIR_BASE_STATE_TABLE),
                        "record_index": off // 8,
                    }
                )
        return sources, summary

    def build_anchors(
        self,
        strategic_air: int,
        air_wings: list[dict[str, Any]],
        sources: list[dict[str, Any]],
        pools: list[dict[str, Any]],
        max_wing_bodies: int,
        max_airbase_records: int,
        max_container_sources: int,
        max_pool_sources: int,
        max_region_sources: int,
    ) -> dict[str, Any]:
        wing_by_body: dict[int, dict[str, Any]] = {}
        wing_by_object: dict[int, dict[str, Any]] = {}
        wing_by_pool: dict[int, list[dict[str, Any]]] = collections.defaultdict(list)
        wing_by_handle: dict[int, list[dict[str, Any]]] = collections.defaultdict(list)
        wing_by_owner_id: dict[tuple[int, int], dict[str, Any]] = {}
        wing_ids_by_owner: dict[int, dict[int, dict[str, Any]]] = collections.defaultdict(dict)
        region_by_ptr: dict[int, int] = {}
        region_ptr_by_id: dict[int, int] = {}
        active_region_wings = [row for row in air_wings if row["mission_region"] is not None]
        active_pool_ptrs = {row["pool"] for row in active_region_wings}
        for row in air_wings:
            wing_by_body[row["body"]] = row
            wing_by_object[row["object"]] = row
            wing_by_pool[row["pool"]].append(row)
            wing_by_owner_id[(row["owner_tag_db_id"], row["wing_id"])] = row
            wing_ids_by_owner[row["owner_tag_db_id"]][row["wing_id"]] = row
            wing_by_handle[(row["wing_id"] << 32) | AIR_WING_OBJECT_HANDLE_TYPE].append(row)
            region_ptr, region_id = self.mission_region_ptr(row["body"])
            if region_ptr is not None and region_id is not None:
                region_by_ptr[region_ptr] = region_id
                region_ptr_by_id.setdefault(region_id, region_ptr)

        roots: list[dict[str, Any]] = [
            {
                "kind": "strategic_air_manager",
                "addr": strategic_air,
                "span": 0x420,
                "via": "CGameState+0x690",
            }
        ]
        containers = [source for source in sources if source["kind"] == "airbase_owner_container"]
        pools_by_priority = sorted(
            [source for source in sources if source["kind"] == "air_wing_pool"],
            key=lambda source: (source["addr"] not in active_pool_ptrs, source["addr"]),
        )
        roots.extend(
            {
                "kind": source["kind"],
                "addr": source["addr"],
                "span": min(source["span"], 0x120),
                "via": "StrategicAirManager+0x30 container/pool chain",
                "priority": "active_pool" if source["addr"] in active_pool_ptrs else "sampled",
            }
            for source in containers[:max_container_sources]
        )
        roots.extend(
            {
                "kind": source["kind"],
                "addr": source["addr"],
                "span": min(source["span"], 0x160),
                "via": "StrategicAirManager+0x30 container/pool chain",
                "priority": "active_pool" if source["addr"] in active_pool_ptrs else "sampled",
            }
            for source in pools_by_priority[:max_pool_sources]
        )
        roots.extend(
            {
                "kind": "mission_region_object",
                "addr": ptr,
                "span": 0x240,
                "via": "air_wing_body+0xB0",
                "region_id": region_id,
            }
            for ptr, region_id in sorted(region_by_ptr.items(), key=lambda item: item[1])[:max_region_sources]
        )
        selected_wings = active_region_wings[:max_wing_bodies]
        if len(selected_wings) < max_wing_bodies:
            selected_wings.extend(
                row
                for row in air_wings
                if row["mission_region"] is None
            )
        roots.extend(
            {
                "kind": "air_wing_body",
                "addr": row["body"],
                "span": 0x380,
                "via": "StrategicAirManager wing pool enumeration",
                "wing_key": row["key"],
                "owner_tag_db_id": row["owner_tag_db_id"],
                "wing_id": row["wing_id"],
                "region_id": row["mission_region"],
            }
            for row in selected_wings[:max_wing_bodies]
        )
        airbase_sources, airbase_summary = self.airbase_record_sources(strategic_air, max_airbase_records)
        roots.extend(airbase_sources)
        roots_by_key = {}
        for root in roots:
            roots_by_key[(root["kind"], root["addr"])] = root
        roots = list(roots_by_key.values())
        return {
            "roots": roots,
            "wing_by_body": wing_by_body,
            "wing_by_object": wing_by_object,
            "wing_by_pool": wing_by_pool,
            "wing_by_handle": wing_by_handle,
            "wing_by_owner_id": wing_by_owner_id,
            "wing_ids_by_owner": wing_ids_by_owner,
            "region_by_ptr": region_by_ptr,
            "region_ptr_by_id": region_ptr_by_id,
            "airbase_record_summary": airbase_summary,
            "active_region_wings": len(active_region_wings),
            "source_cap_summary": {
                "available_container_sources": len(containers),
                "used_container_sources": min(len(containers), max_container_sources),
                "available_pool_sources": len(pools_by_priority),
                "used_pool_sources": min(len(pools_by_priority), max_pool_sources),
                "active_pool_sources": len(active_pool_ptrs),
                "available_region_sources": len(region_by_ptr),
                "used_region_sources": min(len(region_by_ptr), max_region_sources),
                "used_wing_body_sources": min(len(selected_wings), max_wing_bodies),
            },
        }

    def classify_qword(self, value: int, anchors: dict[str, Any]) -> list[dict[str, Any]]:
        matches = []
        row = anchors["wing_by_body"].get(value)
        if row is not None:
            matches.append({"kind": "wing_body_ptr", "wing_key": row["key"], "region_id": row["mission_region"], "owner_tag_db_id": row["owner_tag_db_id"], "wing_id": row["wing_id"]})
        row = anchors["wing_by_object"].get(value)
        if row is not None:
            matches.append({"kind": "wing_object_ptr", "wing_key": row["key"], "region_id": row["mission_region"], "owner_tag_db_id": row["owner_tag_db_id"], "wing_id": row["wing_id"]})
        pooled = anchors["wing_by_pool"].get(value)
        if pooled:
            matches.append({"kind": "wing_pool_ptr", "wing_keys": [row["key"] for row in pooled[:12]], "owner_tag_db_ids": sorted({row["owner_tag_db_id"] for row in pooled}), "region_ids": sorted({row["mission_region"] for row in pooled if row["mission_region"] is not None})})
        handle_rows = anchors["wing_by_handle"].get(value)
        if handle_rows:
            matches.append({"kind": "wing_handle", "handle": hx(value), "wing_keys": [row["key"] for row in handle_rows[:12]], "owner_tag_db_ids": sorted({row["owner_tag_db_id"] for row in handle_rows}), "wing_id": handle_rows[0]["wing_id"]})
        region_id = anchors["region_by_ptr"].get(value)
        if region_id is not None:
            matches.append({"kind": "mission_region_ptr", "region_id": region_id})
        return matches

    def record_identity_matches(
        self,
        record: bytes,
        record_abs: int,
        anchors: dict[str, Any],
        source_region_id: int | None,
    ) -> dict[str, Any] | None:
        direct_wing_keys: dict[str, dict[str, Any]] = {}
        pod_wing_keys: dict[str, dict[str, Any]] = {}
        region_ids = set()
        qword_matches = []
        for off in range(0, len(record) - 7, QWORD_SCAN_STEP):
            value = read_u64_from(record, off)
            if value is None:
                continue
            for match in self.classify_qword(value, anchors):
                qword_matches.append({"offset": hex(off), **match})
                if match["kind"] in {"wing_body_ptr", "wing_object_ptr"}:
                    direct_wing_keys[match["wing_key"]] = {
                        "owner_tag_db_id": match["owner_tag_db_id"],
                        "wing_id": match["wing_id"],
                        "region_id": match["region_id"],
                        "match_kind": match["kind"],
                    }
                elif match["kind"] == "wing_handle":
                    for row in anchors["wing_by_handle"].get(value, [])[:12]:
                        direct_wing_keys[row["key"]] = {
                            "owner_tag_db_id": row["owner_tag_db_id"],
                            "wing_id": row["wing_id"],
                            "region_id": row["mission_region"],
                            "match_kind": "wing_handle",
                        }
                elif match["kind"] == "mission_region_ptr":
                    region_ids.add(match["region_id"])
                elif match["kind"] == "wing_pool_ptr":
                    for region_id in match.get("region_ids", []):
                        region_ids.add(region_id)

        u32_values = [
            (off, read_u32_from(record, off))
            for off in range(0, len(record) - 3, U32_SCAN_STEP)
        ]
        u32_values = [(off, value) for off, value in u32_values if value is not None]
        active_region_ids = set(anchors["region_ptr_by_id"])
        for off, value in u32_values:
            if value in active_region_ids:
                region_ids.add(value)

        # POD fallback: owner tag + wing id appear in the same small record.
        by_value = collections.defaultdict(list)
        for off, value in u32_values:
            by_value[value].append(off)
        wing_ids_by_owner = anchors["wing_ids_by_owner"]
        for owner in set(by_value).intersection(wing_ids_by_owner):
            for wing_id in set(by_value).intersection(wing_ids_by_owner[owner]):
                row = wing_ids_by_owner[owner][wing_id]
                pod_wing_keys[row["key"]] = {
                    "owner_tag_db_id": owner,
                    "wing_id": wing_id,
                    "region_id": row["mission_region"],
                    "match_kind": "owner_wing_u32_pair",
                    "owner_offsets": [hex(off) for off in by_value[owner][:4]],
                    "wing_id_offsets": [hex(off) for off in by_value[wing_id][:4]],
                }

        if source_region_id is not None:
            region_ids.add(source_region_id)
        wing_keys = {**direct_wing_keys, **pod_wing_keys}
        if not wing_keys and not qword_matches:
            return None
        owners = sorted({row["owner_tag_db_id"] for row in wing_keys.values()})
        direct_owners = sorted({row["owner_tag_db_id"] for row in direct_wing_keys.values()})
        pod_owners = sorted({row["owner_tag_db_id"] for row in pod_wing_keys.values()})
        wing_region_ids = sorted(
            {row["region_id"] for row in wing_keys.values() if row.get("region_id") is not None}
        )
        if wing_region_ids:
            region_ids.update(wing_region_ids)
        return {
            "record_addr": hx(record_abs),
            "wing_keys": sorted(wing_keys),
            "owner_tag_db_ids": owners,
            "direct_wing_ref_owner_tag_db_ids": direct_owners,
            "pod_pair_owner_tag_db_ids": pod_owners,
            "direct_wing_ref_wing_keys": sorted(direct_wing_keys),
            "pod_pair_wing_keys": sorted(pod_wing_keys),
            "region_ids": sorted(region_ids),
            "wing_match_details": wing_keys,
            "qword_matches": qword_matches[:16],
        }

    def inspect_vector_array(
        self,
        source: dict[str, Any],
        header_off: int,
        header: dict[str, Any],
        anchors: dict[str, Any],
        record_strides: list[int],
        max_records: int,
        allow_record_scan: bool,
        max_record_scan_records: int,
    ) -> dict[str, Any] | None:
        source_region_id = source.get("region_id")
        begin = header["begin"]
        size = min(header["size"], max_records)
        qword_matches = []
        qword_byte_len = size * 8
        if qword_byte_len <= MAX_VECTOR_BYTES:
            raw = self.proc.try_read(begin, qword_byte_len)
            if len(raw) == qword_byte_len:
                for off, value in qwords(raw):
                    matches = self.classify_qword(value, anchors)
                    for match in matches:
                        qword_matches.append({"element_index": off // 8, "offset": hex(off), **match})
        record_matches = []
        stride_summaries = []
        if allow_record_scan:
            size_for_records = min(size, max_record_scan_records)
        else:
            size_for_records = 0
        for stride in record_strides if allow_record_scan else []:
            byte_len = size_for_records * stride
            if byte_len <= 0 or byte_len > MAX_VECTOR_BYTES:
                continue
            raw = self.proc.try_read(begin, byte_len)
            if len(raw) != byte_len:
                continue
            matches_for_stride = []
            for index in range(size_for_records):
                start = index * stride
                record = raw[start : start + stride]
                match = self.record_identity_matches(
                    record,
                    begin + start,
                    anchors,
                    source_region_id,
                )
                if match is not None:
                    match["record_index"] = index
                    matches_for_stride.append(match)
            if matches_for_stride:
                owners = sorted(
                    {
                        owner
                        for match in matches_for_stride
                        for owner in match.get("owner_tag_db_ids", [])
                    }
                )
                regions = sorted(
                    {
                        region_id
                        for match in matches_for_stride
                        for region_id in match.get("region_ids", [])
                    }
                )
                wing_keys = sorted(
                    {
                        wing_key
                        for match in matches_for_stride
                        for wing_key in match.get("wing_keys", [])
                    }
                )
                stride_summaries.append(
                    {
                        "stride": hex(stride),
                        "matched_records": len(matches_for_stride),
                        "owner_tag_db_ids": owners,
                        "region_ids": regions,
                        "wing_keys": wing_keys[:24],
                    }
                )
                record_matches.extend(
                    {**match, "stride": hex(stride)}
                    for match in matches_for_stride[:48]
                )

        if not qword_matches and not record_matches:
            return None
        all_owners = set()
        all_regions = set()
        all_wing_keys = set()
        direct_wing_owners = set()
        direct_wing_regions = set()
        direct_wing_keys = set()
        record_direct_wing_owners = set()
        record_direct_wing_regions = set()
        record_direct_wing_keys = set()
        pool_collection_owners = set()
        pool_collection_regions = set()
        pod_pair_owners = set()
        pod_pair_regions = set()
        pod_pair_wing_keys = set()
        for match in qword_matches:
            kind = match.get("kind")
            if kind in {"wing_body_ptr", "wing_object_ptr"}:
                direct_wing_owners.add(match["owner_tag_db_id"])
                if match.get("region_id") is not None:
                    direct_wing_regions.add(match["region_id"])
                direct_wing_keys.add(match["wing_key"])
            elif kind == "wing_handle":
                direct_wing_owners.update(match.get("owner_tag_db_ids", []))
                direct_wing_keys.update(match.get("wing_keys", []))
            elif kind == "wing_pool_ptr":
                pool_collection_owners.update(match.get("owner_tag_db_ids", []))
                pool_collection_regions.update(match.get("region_ids", []))
                all_wing_keys.update(match.get("wing_keys", []))
            elif kind == "mission_region_ptr" and match.get("region_id") is not None:
                all_regions.add(match["region_id"])
        for summary in stride_summaries:
            all_owners.update(summary["owner_tag_db_ids"])
            all_regions.update(summary["region_ids"])
            all_wing_keys.update(summary["wing_keys"])
        for match in record_matches:
            record_direct_wing_owners.update(match.get("direct_wing_ref_owner_tag_db_ids", []))
            record_direct_wing_keys.update(match.get("direct_wing_ref_wing_keys", []))
            pod_pair_owners.update(match.get("pod_pair_owner_tag_db_ids", []))
            pod_pair_wing_keys.update(match.get("pod_pair_wing_keys", []))
            if match.get("direct_wing_ref_owner_tag_db_ids"):
                record_direct_wing_regions.update(match.get("region_ids", []))
            if match.get("pod_pair_owner_tag_db_ids"):
                pod_pair_regions.update(match.get("region_ids", []))
        all_owners.update(direct_wing_owners)
        all_owners.update(record_direct_wing_owners)
        all_owners.update(pool_collection_owners)
        all_owners.update(pod_pair_owners)
        all_regions.update(direct_wing_regions)
        all_regions.update(record_direct_wing_regions)
        all_regions.update(pool_collection_regions)
        all_regions.update(pod_pair_regions)
        all_wing_keys.update(direct_wing_keys)
        all_wing_keys.update(record_direct_wing_keys)
        all_wing_keys.update(pod_pair_wing_keys)
        direct_opposition_capable = len(direct_wing_owners) >= 2 and bool(
            direct_wing_regions or all_regions
        )
        air_base_direct_basing_collection = (
            source["kind"] == "air_base_record"
            and bool(direct_wing_keys)
            and not source_region_id
        )
        unanchored_record_stride_opposed = (
            not direct_opposition_capable
            and len(record_direct_wing_owners) >= 2
            and bool(record_direct_wing_regions or all_regions)
        )
        pod_opposed_only = (
            not direct_opposition_capable
            and not unanchored_record_stride_opposed
            and len(pod_pair_owners) >= 2
            and bool(pod_pair_regions or all_regions)
        )
        local_same_owner = (
            len(all_owners) == 1
            or (pool_collection_owners and len(pool_collection_owners) == 1 and not pod_opposed_only)
        ) and bool(all_wing_keys)
        classification = (
            "rejected_air_base_record_basing_wing_collection"
            if air_base_direct_basing_collection
            else
            "possible_direct_opposed_wing_ref_event_state"
            if direct_opposition_capable
            else "rejected_unanchored_record_stride_opposed_refs"
            if unanchored_record_stride_opposed
            else "rejected_pod_owner_wing_id_collision_without_direct_opposed_refs"
            if pod_opposed_only
            else "rejected_same_owner_or_non_region_collection"
            if local_same_owner
            else "rejected_non_opposed_or_ambiguous_identity_collection"
        )
        return {
            "source_kind": source["kind"],
            "source_addr": hx(source["addr"]),
            "source_via": source.get("via"),
            "source_region_id": source_region_id,
            "header_off": hex(header_off),
            "header_addr": hx(source["addr"] + header_off),
            "header_style": header["style"],
            "begin": hx(begin),
            "size": header["size"],
            "cap": header["cap"],
            "classification": classification,
            "opposition_capable": direct_opposition_capable
            and not air_base_direct_basing_collection,
            "air_base_direct_basing_collection": air_base_direct_basing_collection,
            "unanchored_record_stride_opposed": unanchored_record_stride_opposed,
            "pod_opposed_only": pod_opposed_only,
            "direct_wing_ref_owner_tag_db_ids": sorted(direct_wing_owners),
            "direct_wing_ref_region_ids": sorted(direct_wing_regions),
            "direct_wing_ref_wing_keys": sorted(direct_wing_keys)[:32],
            "record_direct_wing_ref_owner_tag_db_ids": sorted(record_direct_wing_owners),
            "record_direct_wing_ref_region_ids": sorted(record_direct_wing_regions),
            "record_direct_wing_ref_wing_keys": sorted(record_direct_wing_keys)[:32],
            "pod_pair_owner_tag_db_ids": sorted(pod_pair_owners),
            "pod_pair_region_ids": sorted(pod_pair_regions),
            "pod_pair_wing_keys": sorted(pod_pair_wing_keys)[:32],
            "pool_collection_owner_tag_db_ids": sorted(pool_collection_owners),
            "pool_collection_region_ids": sorted(pool_collection_regions),
            "owner_tag_db_ids": sorted(all_owners),
            "region_ids": sorted(all_regions),
            "wing_keys": sorted(all_wing_keys)[:32],
            "qword_matches": qword_matches[:48],
            "record_stride_summaries": stride_summaries[:12],
            "record_matches": record_matches[:64],
            "record_scan_allowed": allow_record_scan,
            "record_scan_size": size_for_records,
        }

    def one_hop_sources(
        self,
        roots: list[dict[str, Any]],
        object_span: int,
        max_one_hop_sources: int,
    ) -> tuple[list[dict[str, Any]], dict[str, Any]]:
        known_skip_vtables = {
            self.base + AIR_WING_OBJECT_VTABLE_RVA,
            self.base + AIR_WING_VTABLE_RVA,
            self.base + AIR_WING_POOL_VTABLE_RVA,
            self.base + AIR_WING_REGION_VTABLE_RVA,
            self.base + AIR_WING_REGION_ALT_VTABLE_RVA,
        }
        seen = set()
        out = []
        stats = collections.Counter()
        vtables = collections.Counter()
        for root in roots:
            blob = self.proc.try_read(root["addr"], root["span"])
            for off, ptr in qwords(blob):
                if not is_heap(ptr) or ptr in seen:
                    continue
                vt = read_u64(self.proc, ptr)
                if vt is None or not (self.base <= vt < self.module_hi):
                    continue
                if vt in known_skip_vtables:
                    stats["known_vtable_skipped"] += 1
                    continue
                seen.add(ptr)
                stats["one_hop_sources"] += 1
                vtables[hex(vt - self.base)] += 1
                out.append(
                    {
                        "kind": f"{root['kind']}:one_hop_object",
                        "addr": ptr,
                        "span": object_span,
                        "via": f"{root['kind']}+{hex(off)}",
                        "parent_kind": root["kind"],
                        "parent_addr": hx(root["addr"]),
                        "field_off": hex(off),
                        "vtable_rva": hex(vt - self.base),
                        "region_id": root.get("region_id"),
                    }
                )
                if len(out) >= min(MAX_ONE_HOP_OBJECTS, max_one_hop_sources):
                    stats["one_hop_limit_reached"] += 1
                    return out, {
                        "counts": dict(stats),
                        "top_vtable_rvas": compact_counter(vtables),
                    }
        return out, {"counts": dict(stats), "top_vtable_rvas": compact_counter(vtables)}

    def scan_sources(
        self,
        roots: list[dict[str, Any]],
        anchors: dict[str, Any],
        record_strides: list[int],
        max_records: int,
        max_record_scan_records: int,
    ) -> dict[str, Any]:
        candidates = []
        source_counts = collections.Counter(root["kind"] for root in roots)
        vector_headers_seen = 0
        accepted_vector_headers = 0
        record_scan_allowed_count = 0
        record_scan_skipped_count = 0
        header_styles = collections.Counter()
        for source in roots:
            for off in range(0, max(0, source["span"] - 0x0F), 8):
                vector_headers_seen += 1
                headers = stable_vector_headers(self.proc, source["addr"] + off, max_records)
                for header in headers:
                    accepted_vector_headers += 1
                    header_styles[header["style"]] += 1
                    source_kind = source["kind"]
                    parent_kind = source.get("parent_kind")
                    is_one_hop = source_kind.endswith(":one_hop_object")
                    allow_record_scan = (
                        header["size"] <= max_record_scan_records
                        and (
                            not is_one_hop
                            or parent_kind
                            in {
                                "strategic_air_manager",
                                "mission_region_object",
                                "air_base_record",
                                "air_wing_body",
                            }
                        )
                    )
                    if allow_record_scan:
                        record_scan_allowed_count += 1
                    else:
                        record_scan_skipped_count += 1
                    result = self.inspect_vector_array(
                        source,
                        off,
                        header,
                        anchors,
                        record_strides,
                        max_records,
                        allow_record_scan,
                        max_record_scan_records,
                    )
                    if result is not None:
                        candidates.append(result)
        event_candidates = [row for row in candidates if row["opposition_capable"]]
        unanchored_record_stride_candidates = [
            row for row in candidates if row.get("unanchored_record_stride_opposed")
        ]
        pod_only_candidates = [row for row in candidates if row.get("pod_opposed_only")]
        by_class = collections.Counter(row["classification"] for row in candidates)
        by_source_class = collections.Counter(
            (row["source_kind"], row["classification"]) for row in candidates
        )
        return {
            "source_counts": dict(source_counts),
            "vector_headers_seen": vector_headers_seen,
            "accepted_vector_headers": accepted_vector_headers,
            "record_scan_allowed_headers": record_scan_allowed_count,
            "record_scan_skipped_headers": record_scan_skipped_count,
            "header_styles": dict(header_styles),
            "candidate_count": len(candidates),
            "event_root_candidate_count": len(event_candidates),
            "unanchored_record_stride_opposed_count": len(unanchored_record_stride_candidates),
            "pod_opposed_only_candidate_count": len(pod_only_candidates),
            "classification_counts": dict(by_class),
            "source_classification_counts": [
                {"source_kind": key[0], "classification": key[1], "count": count}
                for key, count in by_source_class.most_common(40)
            ],
            "status": (
                "possible_direct_opposed_wing_ref_event_state"
                if event_candidates
                else "rejected_no_rooted_opposed_indirect_vector_or_pod_event_state"
            ),
            "event_candidates": event_candidates[:32],
            "unanchored_record_stride_samples": unanchored_record_stride_candidates[:16],
            "pod_opposed_only_samples": pod_only_candidates[:16],
            "candidates": candidates[:160],
        }


def run(args: argparse.Namespace) -> dict[str, Any]:
    probe = ResidualRouteMatrixProbe(args.pid)
    try:
        root = probe.root_probe()
        _game_state, strategic_air = probe.require_root(root)
        air_wings, topology, sources, pools = probe.enumerate_air_wings(strategic_air)
        anchors = probe.build_anchors(
            strategic_air,
            air_wings,
            sources,
            pools,
            max_wing_bodies=args.max_wing_bodies,
            max_airbase_records=args.max_airbase_records,
            max_container_sources=args.max_container_sources,
            max_pool_sources=args.max_pool_sources,
            max_region_sources=args.max_region_sources,
        )
        roots = anchors["roots"]
        one_hop, one_hop_summary = probe.one_hop_sources(
            roots,
            args.object_span,
            args.max_one_hop_sources,
        )
        roots_plus_one_hop = roots + one_hop
        matrix = probe.scan_sources(
            roots_plus_one_hop,
            anchors,
            record_strides=args.record_strides,
            max_records=args.max_vector_records,
            max_record_scan_records=args.max_record_scan_records,
        )
        temporal = probe.temporal_probe(
            strategic_air,
            duration_s=args.duration_s,
            interval_s=args.interval_s,
            max_wings=args.max_body_field_wings,
        )
        route_closure = {
            "StrategicAirManager_roots": (
                "rejected_rooted_indirect_vector_or_pod_event_state"
                if matrix["event_root_candidate_count"] == 0
                else "candidate_needs_followup"
            ),
            "region_owned_manager_containers": (
                "rejected_rooted_region_vector_or_pod_event_state"
                if matrix["event_root_candidate_count"] == 0
                else "candidate_needs_followup"
            ),
            "indirect_index_vector_storage": (
                "rejected_under_current_roots"
                if matrix["event_root_candidate_count"] == 0
                else "candidate_needs_followup"
            ),
            "tick_local_POD_records": (
                "blocked_requires_tick_synchronous_fixture_or_resolution_hook"
                if matrix["event_root_candidate_count"] == 0
                else "candidate_needs_followup"
            ),
            "sortie_lifecycle": "not_captured",
            "combat_pairing": "not_captured",
            "attribution_cause_source": "not_captured",
            "tactical_state": "not_captured",
            "rich_reinforcement_reason": "not_captured",
        }
        fixture_required = {
            "status": "required_for_tick_local_or_function_local_state",
            "reason": (
                "Passive rooted scans can reject stable manager/vector/POD roots, "
                "but cannot prove absence of records allocated and freed entirely "
                "inside an air-resolution tick."
            ),
            "minimum_fixture": (
                "Controlled before/during/after capture around a known active air "
                "combat tick: two opposing countries with exactly one identified "
                "fighter wing each in the same strategic air region, one wing with "
                "a known assigned ace and one without, reinforcements disabled or "
                "manually triggered separately, game paused immediately after a "
                "visible air loss. Capture three read-only heap/root snapshots plus "
                "UI/save anchors for both wing ids, owner tag-db-ids, region id, "
                "ace id/name, loss count, and reinforcement source."
            ),
            "production_path_after_fixture": (
                "Diff StrategicAirManager, active mission-region objects, both wing "
                "bodies/pools, country ace containers, and first-hop manager objects; "
                "if no stable root appears, place an in-DLL read-only hook/probe at "
                "the air-resolution function identified by the diff/disassembly."
            ),
        }
        return {
            "probe": "air_combat_losses_live_residual_route_matrix_probe_870ebf_v1",
            "created_utc": utc_now(),
            "read_only": True,
            "root": root,
            "validated_offsets_reused": {
                "CGameState_global": "module+0x3316E90 -> CGameState*",
                "current_game_date_raw": "CGameState+0x468",
                "StrategicAirManager": "CGameState+0x690",
                "StrategicAirManager_vtable": "module+0x2942118",
                "StrategicAirManager_country_containers": "+0x30 begin, +0x3C count",
                "StrategicAirManager_air_base_state_table": "+0x90 vector header",
                "air_wing_body": "vtable module+0x29641B0, type +0x08=69, id +0x0C, pool +0x28, count +0x6C, mission mask +0x90, mission region +0xB0, effective planes +0x160, equipment bank +0x1E8",
                "mission_region_object": "vtable module+0x29569A8, alt vtable module+0x29569F8, id +0x58",
            },
            "air_wing_root_topology": topology,
            "air_wing_rows_observed": len(air_wings),
            "air_wing_rows_on_mission_region": sum(
                1 for row in air_wings if row["mission_region"] is not None
            ),
            "anchor_summary": {
                "root_source_count": len(roots),
                "one_hop_source_count": len(one_hop),
                "active_region_wings": anchors["active_region_wings"],
                "unique_active_region_objects": len(anchors["region_by_ptr"]),
                "airbase_record_summary": anchors["airbase_record_summary"],
                "source_cap_summary": anchors["source_cap_summary"],
                "one_hop_summary": one_hop_summary,
            },
            "matrix_probe": matrix,
            "temporal_delta_probe": temporal,
            "route_closure": route_closure,
            "fixture_required": fixture_required,
            "verdict": {
                "backlog_item": (
                    "tick-local POD, indirect index/vector, region-owned manager, "
                    "wing/mission, and StrategicAirManager residual roots for full "
                    "air-combat events"
                ),
                "classification": (
                    "candidate_needs_followup"
                    if matrix["event_root_candidate_count"]
                    else "rejected_stable_rooted_residual_routes_blocked_tick_local_state"
                ),
                "compact_record_ready": False,
                "backlog_reduced": matrix["event_root_candidate_count"] == 0,
                "reason": (
                    "No rooted manager/region/wing/vector/POD candidate carried "
                    "opposed air-wing identity under the bounded current-build scan. "
                    "Tick-local function-local state remains blocked on a controlled "
                    "before/during/after fixture or an air-resolution hook."
                    if matrix["event_root_candidate_count"] == 0
                    else "At least one rooted opposed vector/POD candidate requires follow-up."
                ),
            },
        }
    finally:
        probe.close()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pid", type=int, required=True)
    parser.add_argument("--duration-s", type=float, default=20.0)
    parser.add_argument("--interval-s", type=float, default=0.10)
    parser.add_argument("--max-wing-bodies", type=int, default=192)
    parser.add_argument("--max-body-field-wings", type=int, default=128)
    parser.add_argument("--max-airbase-records", type=int, default=4096)
    parser.add_argument("--max-container-sources", type=int, default=96)
    parser.add_argument("--max-pool-sources", type=int, default=128)
    parser.add_argument("--max-region-sources", type=int, default=128)
    parser.add_argument("--max-one-hop-sources", type=int, default=512)
    parser.add_argument("--max-vector-records", type=int, default=4096)
    parser.add_argument("--max-record-scan-records", type=int, default=512)
    parser.add_argument("--object-span", type=lambda raw: int(raw, 0), default=0x500)
    parser.add_argument(
        "--record-strides",
        type=lambda raw: [int(part, 0) for part in raw.split(",") if part.strip()],
        default=[0x10, 0x18, 0x20, 0x28, 0x30, 0x38, 0x40, 0x48, 0x50, 0x60, 0x80],
    )
    parser.add_argument("--out-dir", type=Path, default=None)
    args = parser.parse_args()

    report = run(args)
    out_path = None
    if args.out_dir is not None:
        args.out_dir.mkdir(parents=True, exist_ok=True)
        out_path = args.out_dir / "residual_route_matrix_probe_870ebf.json"
        out_path.write_text(json.dumps(report, indent=2, sort_keys=True), encoding="utf-8")
    summary = {
        "out_path": str(out_path) if out_path else None,
        "root_ok": report["root"]["strategic_air_ok"],
        "air_wing_rows_observed": report["air_wing_rows_observed"],
        "active_region_wings": report["air_wing_rows_on_mission_region"],
        "root_sources": report["anchor_summary"]["root_source_count"],
        "one_hop_sources": report["anchor_summary"]["one_hop_source_count"],
        "accepted_vector_headers": report["matrix_probe"]["accepted_vector_headers"],
        "matrix_candidates": report["matrix_probe"]["candidate_count"],
        "event_root_candidates": report["matrix_probe"]["event_root_candidate_count"],
        "unanchored_record_stride_opposed": report["matrix_probe"][
            "unanchored_record_stride_opposed_count"
        ],
        "pod_opposed_only_candidates": report["matrix_probe"]["pod_opposed_only_candidate_count"],
        "matrix_status": report["matrix_probe"]["status"],
        "advanced_hours": report["temporal_delta_probe"]["advanced_hours"],
        "skipped_hours": report["temporal_delta_probe"]["skipped_hours"],
        "loss_examples": report["temporal_delta_probe"]["loss_like_delta_count"],
        "reinforcement_examples": report["temporal_delta_probe"]["reinforcement_like_delta_count"],
        "classification": report["verdict"]["classification"],
    }
    print(json.dumps(summary, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
