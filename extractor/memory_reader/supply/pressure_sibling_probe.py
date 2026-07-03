#!/usr/bin/env python3
"""Enumerate CCountrySupplySystem pressure sibling candidates for domain 44.

This live-session probe is read-only and fail-closed. It verifies the running
hoi4.exe SHA, walks current-build CCountrySupplySystem objects through the
validated +0x78 country join, and scores sibling pointer fields against
same-save/same-frame supply oracle anchors. It does not promote convoy, port,
or disrupted pressure rows.
"""

from __future__ import annotations

import argparse
import json
import struct
import sys
import time
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional, Sequence, Set, Tuple


SCRIPT_DIR = Path(__file__).resolve().parent
MEMORY_READER = SCRIPT_DIR.parent
REPO_ROOT = MEMORY_READER.parents[1]
LANE_DIR = REPO_ROOT / "extractor" / "native" / "lanes" / "supply_logistics_attrition_live"
for path in (MEMORY_READER, LANE_DIR):
    if str(path) not in sys.path:
        sys.path.insert(0, str(path))

import ccountry_supply_layout_probe as layout_probe  # type: ignore  # noqa: E402
import live_probe  # type: ignore  # noqa: E402
import winmem  # type: ignore  # noqa: E402


EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
DOMAIN_NAME = live_probe.DOMAIN_NAME
HEAP_LO = live_probe.HEAP_LO
HEAP_HI = live_probe.HEAP_HI
MAX_PROVINCE_ID = live_probe.MAX_PROVINCE_ID
CCSS_COUNTRY_BACKPTR = 0x78
KNOWN_FOREIGN_FIELD = 0x1E0
KNOWN_FOREIGN_STRIDE = 0x70
KNOWN_FOREIGN_PROVINCE_OFF = 0x00
KNOWN_FOREIGN_DUP_PROVINCE_OFF = 0x50
SCAN_FIELD_LIMIT = 0x300
DEFAULT_REGION_BYTES = 0x20000
MAX_REPORTED = 96
MAX_VALUE_SLOT_REPORT = 16
STRIDES = (0x10, 0x18, 0x20, 0x28, 0x30, 0x38, 0x40, 0x50, 0x60, 0x70, 0x80)
PRESSURE_VERDICT_CATEGORIES = (
    "disrupted_supply",
    "convoy_pressure",
    "port_pressure",
    "foreign_homebase",
)
PRESSURE_VERDICT_THRESHOLDS = {
    "disrupted_supply": 1,
    "convoy_pressure": 1,
    "port_pressure": 1,
    "foreign_homebase": 1,
}


def hx(value: Optional[int]) -> Optional[str]:
    return "0x%X" % value if isinstance(value, int) else None


def candidate_id(field_offset: str, stride: str, province_offset: str) -> str:
    return "ccss_field_%s_stride_%s_province_%s" % (
        field_offset.replace("+", "").lower(),
        stride.lower(),
        province_offset.replace("+", "").lower(),
    )


def build_pressure_verdicts(row: Dict[str, Any]) -> List[Dict[str, Any]]:
    counts = row.get("category_counts") or {}
    cid = str(row["candidate_id"])
    verdicts: List[Dict[str, Any]] = []
    for category in PRESSURE_VERDICT_CATEGORIES:
        match_count = int(counts.get(category, 0))
        threshold = PRESSURE_VERDICT_THRESHOLDS[category]
        passed = match_count >= threshold
        verdict = "PASS" if passed else "FAIL"
        if passed:
            reason = (
                f"{category} oracle-match count {match_count} meets threshold {threshold}; "
                "candidate is still not promotion-safe without same-frame differential and negative controls"
            )
        else:
            reason = (
                f"{category} oracle-match count {match_count} is below threshold {threshold}; "
                "no live category hit for this candidate"
            )
        verdicts.append({
            "candidate_id": cid,
            "field_offset": row["field_offset"],
            "stride": row["stride"],
            "province_offset": row["province_offset"],
            "category": category,
            "oracle_match_count": match_count,
            "threshold": threshold,
            "verdict": verdict,
            "reason": reason,
            "promotion_safe": False,
        })
    return verdicts


def now_utc() -> str:
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def is_heap(value: Optional[int]) -> bool:
    return isinstance(value, int) and HEAP_LO <= value < HEAP_HI and value % 8 == 0


def safe_u64(p: Any, addr: int) -> Optional[int]:
    try:
        return p.u64(addr)
    except OSError:
        return None


def open_gated_process(pid: Optional[int]) -> Tuple[Optional[winmem.Process], Dict[str, Any]]:
    process = winmem.Process(pid, "hoi4.exe") if pid is not None else winmem.open_process("hoi4.exe")
    if process is None:
        return None, {
            "ok": False,
            "failure": "hoi4.exe not running or cannot be opened",
            "expected_sha256": EXPECTED_SHA256,
        }
    fp = process.fingerprint()
    fp["expected_sha256"] = EXPECTED_SHA256
    fp["build_ok"] = (fp.get("sha256") or "").lower() == EXPECTED_SHA256
    if not fp["build_ok"]:
        process.close()
        return None, {
            "ok": False,
            "failure": "build_sha_mismatch",
            "process": fp,
            "expected_sha256": EXPECTED_SHA256,
        }
    return process, {"ok": True, "process": fp}


def read_json(path: Path) -> Dict[str, Any]:
    with path.open("r", encoding="utf-8") as handle:
        return json.load(handle)


def add_anchor(
    by_tag: Dict[str, Dict[str, Set[int]]],
    tag: str,
    category: str,
    province: Any,
) -> None:
    if isinstance(province, int) and 0 < province <= MAX_PROVINCE_ID:
        by_tag[tag][category].add(int(province))


def load_pressure_anchors(path: Path) -> Dict[str, Any]:
    data = read_json(path)
    by_tag: Dict[str, Dict[str, Set[int]]] = defaultdict(lambda: defaultdict(set))
    for tag, rec in (data.get("countries") or {}).items():
        if not isinstance(tag, str):
            continue
        for item in rec.get("foreign_homebase_nodes") or []:
            add_anchor(by_tag, tag, "foreign_homebase", item.get("province"))
        for item in rec.get("disrupted_supply") or []:
            add_anchor(by_tag, tag, "disrupted_supply", item.get("node_province"))
            add_anchor(by_tag, tag, "disrupted_supply", item.get("province"))
            add_anchor(by_tag, tag, "disrupted_supply", item.get("port_province"))
        for item in rec.get("node_motorization") or []:
            add_anchor(by_tag, tag, "motorization_control", item.get("node_province"))
        for item in rec.get("convoy_pressure") or []:
            add_anchor(by_tag, tag, "convoy_pressure", item.get("province"))
            add_anchor(by_tag, tag, "convoy_pressure", item.get("node_province"))
        for item in rec.get("port_pressure") or []:
            add_anchor(by_tag, tag, "port_pressure", item.get("province"))
            add_anchor(by_tag, tag, "port_pressure", item.get("port_province"))
    compact = {
        tag: {category: sorted(values) for category, values in categories.items() if values}
        for tag, categories in sorted(by_tag.items())
        if any(categories.values())
    }
    counts = Counter()
    for categories in compact.values():
        for category, values in categories.items():
            counts[category] += len(values)
    return {
        "path": str(path),
        "save_date": data.get("meta", {}).get("save_date"),
        "player": data.get("meta", {}).get("player"),
        "by_tag": compact,
        "category_counts": dict(sorted(counts.items())),
        "tag_count": len(compact),
        "total_anchors": sum(counts.values()),
        "has_disrupted_anchors": counts.get("disrupted_supply", 0) > 0,
    }


def read_module_chunks(p: Any) -> List[Tuple[int, bytes]]:
    chunks: List[Tuple[int, bytes]] = []
    end = p.module_base + p.module_size
    for base, size in p.iter_regions(p.module_base, end, want_exec=None):
        blob = p.try_read(base, size)
        if blob:
            chunks.append((base, blob))
    return chunks


def derive_rtti_vtables(p: Any, decorated_name: bytes) -> Dict[str, Any]:
    chunks = read_module_chunks(p)
    module_start = p.module_base
    module_end = p.module_base + p.module_size
    string_addrs: List[int] = []
    for base, blob in chunks:
        pos = 0
        while True:
            idx = blob.find(decorated_name, pos)
            if idx < 0:
                break
            string_addrs.append(base + idx)
            pos = idx + 1

    col_hits: List[Dict[str, int]] = []
    vtables: List[int] = []
    for string_addr in string_addrs:
        type_descriptor = string_addr - 0x10
        type_descriptor_rva = type_descriptor - module_start
        if not (module_start <= type_descriptor < module_end):
            continue
        pattern = struct.pack("<I", type_descriptor_rva)
        for base, blob in chunks:
            pos = 0
            while True:
                idx = blob.find(pattern, pos)
                if idx < 0:
                    break
                col = base + idx - 0x0C
                raw = p.try_read(col, 24)
                if len(raw) >= 24:
                    sig, off, cd_off, td_ref, class_ref, self_ref = struct.unpack("<IIIIII", raw[:24])
                    if sig in (0, 1) and td_ref == type_descriptor_rva and self_ref == col - module_start:
                        col_hits.append({
                            "string_addr": string_addr,
                            "type_descriptor": type_descriptor,
                            "complete_object_locator": col,
                            "signature": sig,
                            "offset": off,
                            "cd_offset": cd_off,
                            "class_descriptor_rva": class_ref,
                        })
                pos = idx + 1
    for hit in col_hits:
        col_ptr = struct.pack("<Q", hit["complete_object_locator"])
        for base, blob in chunks:
            pos = 0
            while True:
                idx = blob.find(col_ptr, pos)
                if idx < 0:
                    break
                vtable = base + idx + 8
                if vtable not in vtables:
                    vtables.append(vtable)
                pos = idx + 1
    return {
        "decorated_name": decorated_name.decode("ascii", "replace"),
        "module_chunk_count": len(chunks),
        "module_bytes_read": sum(len(blob) for _base, blob in chunks),
        "string_matches": [hx(value) for value in string_addrs],
        "complete_object_locators": [
            {
                "complete_object_locator": hx(hit["complete_object_locator"]),
                "type_descriptor": hx(hit["type_descriptor"]),
                "class_descriptor_rva": hx(hit["class_descriptor_rva"]),
                "offset": hit["offset"],
                "cd_offset": hit["cd_offset"],
            }
            for hit in col_hits
        ],
        "derived_vtables": [{"addr": hx(v), "rva": hx(v - p.module_base)} for v in vtables],
    }


def anchor_sets_for_tag(anchors: Dict[str, Any], tag: str) -> Tuple[Set[int], Dict[int, List[str]]]:
    categories = anchors["by_tag"].get(tag) or {}
    all_values: Set[int] = set()
    category_by_province: Dict[int, List[str]] = defaultdict(list)
    for category, provinces in categories.items():
        for province in provinces:
            all_values.add(int(province))
            category_by_province[int(province)].append(category)
    return all_values, category_by_province


def find_u32_hits(blob: bytes, needles: Set[int]) -> List[Tuple[int, int]]:
    if not needles:
        return []
    hits: List[Tuple[int, int]] = []
    needle_bytes = {struct.pack("<I", value): value for value in needles}
    for packed, value in needle_bytes.items():
        pos = 0
        while True:
            idx = blob.find(packed, pos)
            if idx < 0:
                break
            if idx % 4 == 0:
                hits.append((idx, value))
            pos = idx + 1
    return hits


def plausible_i32(value: int) -> bool:
    return -5_000_000 <= value <= 10_000_000


def summarize_value_slots(blob: bytes, stride: int, province_off: int, record_offsets: Sequence[int]) -> List[Dict[str, Any]]:
    rows: List[Dict[str, Any]] = []
    for value_off in range(0, min(stride, 0x80), 4):
        values: List[int] = []
        for rec_off in record_offsets[:256]:
            if rec_off + value_off + 4 > len(blob):
                continue
            value = struct.unpack_from("<i", blob, rec_off + value_off)[0]
            if plausible_i32(value):
                values.append(value)
        if not values:
            continue
        distinct = len(set(values))
        nonzero = sum(1 for value in values if value != 0)
        if distinct < 2 and nonzero == 0:
            continue
        rows.append({
            "offset": "+0x%X" % value_off,
            "role_hint": "province_slot" if value_off == province_off else "plausible_i32_payload",
            "samples": len(values),
            "nonzero": nonzero,
            "distinct": distinct,
            "min": min(values),
            "max": max(values),
        })
    rows.sort(key=lambda row: (row["role_hint"] != "province_slot", -row["nonzero"], -row["distinct"]))
    return rows[:MAX_VALUE_SLOT_REPORT]


def enumerate_candidates_for_object(
    p: Any,
    obj: int,
    tag_id: int,
    tag: str,
    anchors: Dict[str, Any],
    region_bytes: int,
) -> Tuple[List[Dict[str, Any]], List[Dict[str, Any]]]:
    anchor_values, category_by_province = anchor_sets_for_tag(anchors, tag)
    body = p.try_read(obj, SCAN_FIELD_LIMIT)
    if len(body) < 0x20:
        return [], [{
            "tag": tag,
            "tag_db_id": tag_id,
            "ccountry_supply_system": hx(obj),
            "field_offset": None,
            "reason": "ccss_body_short_read",
        }]
    candidates_by_key: Dict[Tuple[int, int, int], Dict[str, Any]] = {}
    rejected_fields: List[Dict[str, Any]] = []
    for field_off in range(0x10, min(len(body) - 7, SCAN_FIELD_LIMIT), 8):
        ptr = struct.unpack_from("<Q", body, field_off)[0]
        if not is_heap(ptr):
            rejected_fields.append({
                "tag": tag,
                "tag_db_id": tag_id,
                "ccountry_supply_system": hx(obj),
                "field_offset": "+0x%X" % field_off,
                "reason": "not_heap_pointer",
                "value": hx(ptr),
            })
            continue
        blob = p.try_read(ptr, region_bytes)
        if len(blob) < 0x20:
            rejected_fields.append({
                "tag": tag,
                "tag_db_id": tag_id,
                "ccountry_supply_system": hx(obj),
                "field_offset": "+0x%X" % field_off,
                "region": hx(ptr),
                "reason": "region_short_read",
                "bytes_read": len(blob),
            })
            continue
        hits = find_u32_hits(blob, anchor_values)
        if not hits:
            rejected_fields.append({
                "tag": tag,
                "tag_db_id": tag_id,
                "ccountry_supply_system": hx(obj),
                "field_offset": "+0x%X" % field_off,
                "region": hx(ptr),
                "reason": "no_tag_specific_anchor_province_hits",
                "bytes_read": len(blob),
            })
            continue
        for hit_off, province in hits:
            categories = category_by_province.get(province, [])
            for stride in STRIDES:
                province_off = hit_off % stride
                if province_off + 4 > stride or province_off > 0x80:
                    continue
                record_off = hit_off - province_off
                key = (field_off, stride, province_off)
                rec = candidates_by_key.setdefault(key, {
                    "tag": tag,
                    "tag_db_id": tag_id,
                    "ccountry_supply_system": hx(obj),
                    "field_offset": "+0x%X" % field_off,
                    "region": hx(ptr),
                    "stride": "0x%X" % stride,
                    "province_offset": "+0x%X" % province_off,
                    "matched_anchor_provinces": set(),
                    "category_counts": Counter(),
                    "record_offsets_raw": [],
                    "samples": [],
                    "_blob": blob,
                    "_record_offsets": [],
                })
                rec["matched_anchor_provinces"].add(province)
                rec["record_offsets_raw"].append(record_off)
                rec["_record_offsets"].append(record_off)
                for category in categories:
                    rec["category_counts"][category] += 1
                if len(rec["samples"]) < 8:
                    dup = None
                    if record_off + KNOWN_FOREIGN_DUP_PROVINCE_OFF + 4 <= len(blob):
                        dup = struct.unpack_from("<I", blob, record_off + KNOWN_FOREIGN_DUP_PROVINCE_OFF)[0]
                    rec["samples"].append({
                        "province": province,
                        "categories": categories,
                        "record_offset": "+0x%X" % record_off,
                        "duplicate_at_0x50": dup,
                    })
    candidates: List[Dict[str, Any]] = []
    for rec in candidates_by_key.values():
        blob = rec.pop("_blob")
        offsets = rec.pop("_record_offsets")
        rec["matched_anchor_provinces"] = sorted(rec["matched_anchor_provinces"])
        rec["category_counts"] = dict(sorted(rec["category_counts"].items()))
        rec["match_count"] = len(rec["record_offsets_raw"])
        rec["distinct_anchor_provinces"] = len(rec["matched_anchor_provinces"])
        rec["value_slot_summary"] = summarize_value_slots(
            blob,
            int(rec["stride"], 16),
            int(rec["province_offset"][3:], 16),
            offsets,
        )
        rec["known_foreign_homebase_control"] = (
            int(rec["field_offset"][3:], 16) == KNOWN_FOREIGN_FIELD
            and int(rec["stride"], 16) == KNOWN_FOREIGN_STRIDE
            and int(rec["province_offset"][3:], 16) == KNOWN_FOREIGN_PROVINCE_OFF
        )
        candidates.append(rec)
    return candidates, rejected_fields


def aggregate_candidates(candidates: Sequence[Dict[str, Any]]) -> List[Dict[str, Any]]:
    grouped: Dict[Tuple[str, str, str], Dict[str, Any]] = {}
    for row in candidates:
        key = (row["field_offset"], row["stride"], row["province_offset"])
        agg = grouped.setdefault(key, {
            "field_offset": row["field_offset"],
            "stride": row["stride"],
            "province_offset": row["province_offset"],
            "tags": set(),
            "tag_ids": set(),
            "match_count": 0,
            "distinct_anchor_provinces": set(),
            "category_counts": Counter(),
            "known_foreign_homebase_control": row.get("known_foreign_homebase_control", False),
            "examples": [],
        })
        agg["tags"].add(row["tag"])
        agg["tag_ids"].add(row["tag_db_id"])
        agg["match_count"] += int(row["match_count"])
        agg["distinct_anchor_provinces"].update(row["matched_anchor_provinces"])
        agg["category_counts"].update(row["category_counts"])
        if len(agg["examples"]) < 12:
            agg["examples"].append({
                "tag": row["tag"],
                "tag_db_id": row["tag_db_id"],
                "ccountry_supply_system": row["ccountry_supply_system"],
                "region": row["region"],
                "samples": row["samples"][:3],
                "value_slot_summary": row["value_slot_summary"][:6],
            })
    rows: List[Dict[str, Any]] = []
    for agg in grouped.values():
        row = {
            "candidate_id": candidate_id(agg["field_offset"], agg["stride"], agg["province_offset"]),
            "field_offset": agg["field_offset"],
            "stride": agg["stride"],
            "province_offset": agg["province_offset"],
            "tags": sorted(agg["tags"]),
            "tag_ids": sorted(agg["tag_ids"]),
            "tag_count": len(agg["tags"]),
            "match_count": agg["match_count"],
            "distinct_anchor_provinces": len(agg["distinct_anchor_provinces"]),
            "category_counts": dict(sorted(agg["category_counts"].items())),
            "known_foreign_homebase_control": agg["known_foreign_homebase_control"],
            "examples": agg["examples"],
            "promotion_safe": False,
        }
        row["candidate_verdicts"] = build_pressure_verdicts(row)
        rows.append(row)
    rows.sort(
        key=lambda row: (
            row["known_foreign_homebase_control"],
            row["category_counts"].get("disrupted_supply", 0),
            row["tag_count"],
            row["distinct_anchor_provinces"],
            row["match_count"],
        ),
        reverse=True,
    )
    return rows


def build_report(args: argparse.Namespace, p: Any, gate: Dict[str, Any]) -> Dict[str, Any]:
    anchors = load_pressure_anchors(args.supply_json)
    countries = layout_probe.read_country_maps(p)
    live_date = layout_probe.read_current_date(p, countries.get("game_state"))
    ccss_objects = layout_probe.scan_ccountry_supply_objects(p)
    id_to_tag = countries["id_to_tag"]
    country_to_tag = {
        country: (tag_id, id_to_tag.get(tag_id, ""))
        for tag_id, country in countries["id_to_country"].items()
    }
    joined: List[Tuple[int, int, str]] = []
    unjoined = 0
    for obj in ccss_objects:
        country = safe_u64(p, obj + CCSS_COUNTRY_BACKPTR)
        if country in country_to_tag:
            tag_id, tag = country_to_tag[country]
            joined.append((obj, tag_id, tag))
        else:
            unjoined += 1

    all_candidates: List[Dict[str, Any]] = []
    all_rejections: List[Dict[str, Any]] = []
    for obj, tag_id, tag in joined:
        if tag not in anchors["by_tag"]:
            continue
        candidates, rejections = enumerate_candidates_for_object(
            p,
            obj,
            tag_id,
            tag,
            anchors,
            args.region_bytes,
        )
        all_candidates.extend(candidates)
        all_rejections.extend(rejections)

    aggregated = aggregate_candidates(all_candidates)
    candidate_verdicts = [
        verdict
        for row in aggregated
        for verdict in row.get("candidate_verdicts", [])
    ]
    rtti = None if args.skip_rtti else derive_rtti_vtables(p, b".?AVCCountrySupplySystem@@")
    has_disrupted_candidate = any(
        verdict.get("category") == "disrupted_supply" and verdict.get("verdict") == "PASS"
        for verdict in candidate_verdicts
    )
    control = next((row for row in aggregated if row.get("known_foreign_homebase_control")), None)
    verdict_pass_counts = Counter(
        verdict["category"]
        for verdict in candidate_verdicts
        if verdict.get("verdict") == "PASS"
    )
    status = "pressure_sibling_candidates_enumerated"
    if not anchors["has_disrupted_anchors"]:
        status = "fixture_missing_disrupted_supply_anchors"
    elif not has_disrupted_candidate:
        status = "no_disrupted_pressure_candidate_matched_anchors"
    return {
        "schema_version": 1,
        "lane": DOMAIN_NAME,
        "probe": "pressure_sibling_probe",
        "expected_sha256": EXPECTED_SHA256,
        "started_utc": args.started_utc,
        "completed_utc": now_utc(),
        "read_only": True,
        "writes_to_game_memory": False,
        "status": status,
        "gate": gate,
        "live_date": live_date,
        "anchor_oracle": anchors,
        "ccountry_supply_system": {
            "objects": len(ccss_objects),
            "joined_objects": len(joined),
            "unjoined_objects": unjoined,
            "country_backptr_offset": "+0x%X" % CCSS_COUNTRY_BACKPTR,
            "rtti": rtti,
        },
        "known_control_candidate": {
            "field_offset": "+0x%X" % KNOWN_FOREIGN_FIELD,
            "stride": "0x%X" % KNOWN_FOREIGN_STRIDE,
            "province_offset": "+0x%X" % KNOWN_FOREIGN_PROVINCE_OFF,
            "duplicate_province_offset": "+0x%X" % KNOWN_FOREIGN_DUP_PROVINCE_OFF,
            "matched_in_this_run": control is not None,
            "score": control,
        },
        "aggregated_candidates": aggregated[:MAX_REPORTED],
        "candidate_verdicts": candidate_verdicts,
        "candidate_verdict_count_full": len(candidate_verdicts),
        "candidate_verdict_pass_counts": dict(sorted(verdict_pass_counts.items())),
        "candidate_count_full": len(aggregated),
        "rejected_pointer_fields": all_rejections[:MAX_REPORTED],
        "rejected_pointer_field_count_full": len(all_rejections),
        "rejection_reason_counts": dict(Counter(row["reason"] for row in all_rejections)),
        "promotion_safe_pressure_rows": 0,
        "promotion_rule": (
            "A pressure sibling may only be promoted after same-frame disrupted/convoy/port "
            "anchors match with negative controls and a before/after fixture rejects province-id coincidences."
        ),
    }


def write_outputs(out_path: Path, report: Dict[str, Any]) -> None:
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    lines = [
        "# Domain 44 Pressure Sibling Probe",
        "",
        f"- status: `{report.get('status')}`",
        f"- exe_sha256: `{report.get('gate', {}).get('process', {}).get('sha256')}`",
        f"- live_date: `{report.get('live_date', {}).get('decoded')}`",
        f"- anchor categories: `{report.get('anchor_oracle', {}).get('category_counts')}`",
        f"- ccss objects/joined: `{report.get('ccountry_supply_system', {}).get('objects')}` / `{report.get('ccountry_supply_system', {}).get('joined_objects')}`",
        f"- candidate_count_full: `{report.get('candidate_count_full')}`",
        f"- candidate_verdict_count_full: `{report.get('candidate_verdict_count_full')}`",
        f"- candidate_verdict_pass_counts: `{report.get('candidate_verdict_pass_counts')}`",
        f"- rejected_pointer_field_count_full: `{report.get('rejected_pointer_field_count_full')}`",
        f"- promotion_safe_pressure_rows: `{report.get('promotion_safe_pressure_rows')}`",
        "",
        "## Top Candidates",
        "",
    ]
    for row in report.get("aggregated_candidates", [])[:16]:
        verdict_summary = ", ".join(
            f"{verdict.get('category')}={verdict.get('verdict')}("
            f"{verdict.get('oracle_match_count')}/{verdict.get('threshold')})"
            for verdict in row.get("candidate_verdicts", [])
        )
        lines.append(
            f"- `{row.get('candidate_id')}` field `{row.get('field_offset')}` stride `{row.get('stride')}` "
            f"province `{row.get('province_offset')}`: tags `{row.get('tag_count')}`, "
            f"provinces `{row.get('distinct_anchor_provinces')}`, categories "
            f"`{row.get('category_counts')}`, verdicts `{verdict_summary}`, "
            f"control `{row.get('known_foreign_homebase_control')}`."
        )
    verdicts = report.get("candidate_verdicts", [])
    pass_verdicts = [verdict for verdict in verdicts if verdict.get("verdict") == "PASS"]
    fail_verdicts = [verdict for verdict in verdicts if verdict.get("verdict") != "PASS"]
    lines.extend([
        "",
        "## Per-Candidate Verdicts",
        "",
    ])
    if not verdicts:
        lines.append("- No candidate verdicts were emitted because no aggregate candidates matched oracle provinces.")
    else:
        lines.append("- Showing all PASS verdicts first, then the first FAIL verdicts needed for audit context.")
        for verdict in (pass_verdicts + fail_verdicts)[:64]:
            lines.append(
                f"- `{verdict.get('candidate_id')}` field `{verdict.get('field_offset')}` "
                f"stride `{verdict.get('stride')}` province `{verdict.get('province_offset')}` "
                f"category `{verdict.get('category')}`: `{verdict.get('verdict')}` "
                f"({verdict.get('oracle_match_count')}/{verdict.get('threshold')}) - "
                f"{verdict.get('reason')}"
            )
    lines.extend([
        "",
        "## Rejection Summary",
        "",
    ])
    for reason, count in sorted(report.get("rejection_reason_counts", {}).items()):
        lines.append(f"- `{reason}`: `{count}` pointer fields.")
    lines.extend([
        "",
        "## Verdict",
        "",
        "- No convoy, port, disrupted, or foreign-homebase pressure row is promoted by this probe.",
        "- The `+0x1E0` foreign-homebase family is a control candidate; sibling fields still need same-frame fixture proof and negative controls.",
    ])
    out_path.with_suffix(".md").write_text("\n".join(lines) + "\n", encoding="utf-8")


def write_fail(out_path: Path, status: str, gate: Dict[str, Any], reason: str) -> None:
    report = {
        "schema_version": 1,
        "lane": DOMAIN_NAME,
        "probe": "pressure_sibling_probe",
        "expected_sha256": EXPECTED_SHA256,
        "completed_utc": now_utc(),
        "read_only": True,
        "writes_to_game_memory": False,
        "status": status,
        "gate": gate,
        "reason": reason,
        "promotion_safe_pressure_rows": 0,
    }
    write_outputs(out_path, report)


def parse_args(argv: Optional[Iterable[str]] = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pid", type=int, default=None, help="hoi4.exe pid; defaults to process lookup")
    parser.add_argument("--supply-json", required=True, type=Path, help="same-save supply oracle JSON")
    parser.add_argument("--out", required=True, type=Path, help="JSON report path")
    parser.add_argument("--region-bytes", type=lambda raw: int(raw, 0), default=DEFAULT_REGION_BYTES)
    parser.add_argument("--skip-rtti", action="store_true", help="skip module RTTI rediscovery")
    args = parser.parse_args(list(argv) if argv is not None else None)
    args.started_utc = now_utc()
    return args


def main(argv: Optional[Iterable[str]] = None) -> int:
    args = parse_args(argv)
    process, gate = open_gated_process(args.pid)
    if process is None:
        write_fail(args.out, "fail_closed", gate, gate.get("failure", "build gate failed"))
        return 2
    try:
        report = build_report(args, process, gate)
        write_outputs(args.out, report)
        print(json.dumps({
            "status": report["status"],
            "out": str(args.out),
            "candidate_count_full": report["candidate_count_full"],
            "candidate_verdict_count_full": report["candidate_verdict_count_full"],
            "candidate_verdict_pass_counts": report["candidate_verdict_pass_counts"],
            "rejected_pointer_field_count_full": report["rejected_pointer_field_count_full"],
            "promotion_safe_pressure_rows": report["promotion_safe_pressure_rows"],
        }, indent=2))
        return 0 if report["status"] == "pressure_sibling_candidates_enumerated" else 2
    except Exception as exc:  # noqa: BLE001
        write_fail(args.out, "probe_failed", gate, str(exc))
        print(json.dumps({"status": "probe_failed", "out": str(args.out), "reason": str(exc)}, indent=2))
        return 1
    finally:
        process.close()


if __name__ == "__main__":
    raise SystemExit(main())
