#!/usr/bin/env python3
"""Bounded current-build diplomacy root probe for domain44 work.

This is lane-local by design. It reads the live process, validates current-build
country/relation roots, and emits fail-closed JSON proof. It does not patch or
invoke the shared collector/recorder entrypoints.
"""

from __future__ import annotations

import argparse
import ctypes
import hashlib
import json
import struct
import sys
import time
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parents[4]
MEMORY_READER = REPO_ROOT / "extractor" / "memory_reader"
if str(MEMORY_READER) not in sys.path:
    sys.path.insert(0, str(MEMORY_READER))

import winmem  # type: ignore

try:
    import locate  # type: ignore
except Exception:  # pragma: no cover - optional pretty names only
    locate = None


EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"

CG_STATE_DATE_OFFSET = 0x3316E90
COUNTRY_DATABASE_OFFSET = 0x33189C8
CGAMESTATE_COUNTRY_TABLE = 0x310
CGAMESTATE_COUNTRY_COUNT = 0x31C

OLD_COUNTRY_VTABLE_RVA_16022851 = 0x27C0E80
COUNTRY_VTABLE_RVA_870EBF = 0x27D0E90
COUNTRY_RVA_DELTA = COUNTRY_VTABLE_RVA_870EBF - OLD_COUNTRY_VTABLE_RVA_16022851

OBJ_TAG_DB_ID = 0x08
OBJ_FACTION_GROUP = 0x598
OBJ_RELATION_MANAGER = 0xF88

RELATION_TABLE_OFFSET = 0x08
REL_OPINION = 0x300
REL_SCAN_BYTES = 0x340

SUBJ_OVERLORD = 0x10
SUBJ_SUBJECT = 0x14
SUBJ_STATE_DB = 0x58
SUBJ_STATE_NAME = 0x08
SUBJ_RATIO = 0x60

WR_FIRST_TAGID = 0x00
WR_SECOND_TAGID = 0x04
WR_TYPEPTR = 0x08
WR_FIRST_CAS = 0x40
WR_SECOND_CAS = 0x48
WR_ECHO = 0x68
WR_GAME_DATE_VTABLE_RVA = 0x26F2398
WR_GREGORIAN_DATE_VTABLE_RVA = 0x26F21A0
WR_MAX_CASUALTIES = 50_000_000

HEAP_LO = 0x0000010000000000
HEAP_HI = 0x0000800000000000


OLD_RELATION_FLAG_VTABLE_RVAS = {
    "non_aggression_pact": 0x2939FE8,
    "subject_or_puppet": 0x293A168,
    "embargo": 0x293A2C0,
    "military_access": 0x2939B68,
    "air_base_access": 0x29F9DB8,
    "docking_rights": 0x29F9FA8,
    "naval_blockade": 0x29FA150,
    "market_access_rights": 0x29FA300,
    "equipment_purchase_contract_relation": 0x29FA238,
    "guarantee": 0x2939F28,
    "improve_relations": 0x2939CE8,
    "attache": 0x2939DA8,
    "licence_production": 0x2939AA8,
}

RELATION_FLAG_RTTI_NAMES = {
    "non_aggression_pact": b".?AVCNonAggressionPactRelation@@",
    "subject_or_puppet": b".?AVCSubjectRelation@@",
    "embargo": b".?AVCEmbargoRelation@@",
    "military_access": b".?AVCMilitaryAccessRelation@@",
    "offer_military_access": b".?AVCOfferMilitaryAccessRelation@@",
    "air_base_access": b".?AVCAirBaseAccessRelation@@",
    "offer_air_base_access": b".?AVCOfferAirBaseAccessRelation@@",
    "docking_rights": b".?AVCDockingRightsRelation@@",
    "offer_docking_rights": b".?AVCOfferDockingRightsRelation@@",
    "naval_blockade": b".?AVCNavalBlockadeRelation@@",
    "market_access_rights": b".?AVCMarketAccessRightsRelation@NInternationalMarket@@",
    "equipment_purchase_relation": b".?AVCEquipmentPurchaseRelation@NInternationalMarket@@",
    "guarantee": b".?AVCGuaranteeRelation@@",
    "improve_relations": b".?AVCImproveRelationsRelation@@",
    "attache": b".?AVCAttacheRelation@@",
    "licence_production": b".?AVCLicenceProductionAccessRelation@@",
}


def is_heap(ptr: int) -> bool:
    return HEAP_LO <= ptr < HEAP_HI


def read_exact(p: winmem.Process, addr: int, size: int) -> bytes | None:
    if addr <= 0:
        return None
    try:
        data = p.read(addr, size)
    except Exception:
        return None
    return data if len(data) == size else None


def u64(p: winmem.Process, addr: int) -> int | None:
    data = read_exact(p, addr, 8)
    return struct.unpack("<Q", data)[0] if data is not None else None


def u32(p: winmem.Process, addr: int) -> int | None:
    data = read_exact(p, addr, 4)
    return struct.unpack("<I", data)[0] if data is not None else None


def i32(p: winmem.Process, addr: int) -> int | None:
    data = read_exact(p, addr, 4)
    return struct.unpack("<i", data)[0] if data is not None else None


def i64(p: winmem.Process, addr: int) -> int | None:
    data = read_exact(p, addr, 8)
    return struct.unpack("<q", data)[0] if data is not None else None


def qword_at(data: bytes, off: int) -> int:
    return struct.unpack_from("<Q", data, off)[0]


def hx(value: int | None) -> str | None:
    return "0x%X" % value if isinstance(value, int) else None


def file_sha256(path: str | None) -> str | None:
    if not path:
        return None
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def read_inline_tag(p: winmem.Process, tag_table: int, tag_id: int) -> str | None:
    data = read_exact(p, tag_table + tag_id * 0x20, 3)
    if data is None:
        return None
    try:
        return data.decode("ascii")
    except UnicodeDecodeError:
        return None


def maybe_obj_name(p: winmem.Process, country: int) -> str | None:
    if locate is None:
        return None
    try:
        value = locate.read_msvc_str(p, country + 0x10)
    except Exception:
        return None
    return value if value else None


def read_module_chunks(p: winmem.Process) -> list[tuple[int, bytes]]:
    chunks: list[tuple[int, bytes]] = []
    end = p.module_base + p.module_size
    for base, size in p.iter_regions(p.module_base, end, want_exec=None):
        blob = p.try_read(base, size)
        if blob:
            chunks.append((base, blob))
    return chunks


def derive_rtti_vtables(
    p: winmem.Process, chunks: list[tuple[int, bytes]], decorated_name: bytes
) -> dict[str, Any]:
    module_start = p.module_base
    module_end = p.module_base + p.module_size
    string_addrs: list[int] = []
    for base, blob in chunks:
        pos = 0
        while True:
            idx = blob.find(decorated_name, pos)
            if idx < 0:
                break
            string_addrs.append(base + idx)
            pos = idx + 1

    col_hits: list[dict[str, int]] = []
    vtables: list[int] = []
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
                        col_hits.append(
                            {
                                "string_addr": string_addr,
                                "type_descriptor": type_descriptor,
                                "complete_object_locator": col,
                                "signature": sig,
                                "offset": off,
                                "cd_offset": cd_off,
                                "class_descriptor_rva": class_ref,
                            }
                        )
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
        "string_matches": string_addrs,
        "complete_object_locators": col_hits,
        "vtables": vtables,
    }


def derive_relation_flag_rtti(p: winmem.Process) -> dict[str, Any]:
    chunks = read_module_chunks(p)
    classes: dict[str, Any] = {}
    for flag_name, decorated in RELATION_FLAG_RTTI_NAMES.items():
        derived = derive_rtti_vtables(p, chunks, decorated)
        classes[flag_name] = {
            "decorated_name": derived["decorated_name"],
            "string_matches": [hx(value) for value in derived["string_matches"]],
            "complete_object_locators": [
                {
                    "string_addr": hx(hit["string_addr"]),
                    "type_descriptor": hx(hit["type_descriptor"]),
                    "complete_object_locator": hx(hit["complete_object_locator"]),
                    "class_descriptor_rva": hx(hit["class_descriptor_rva"]),
                    "offset": hit["offset"],
                    "cd_offset": hit["cd_offset"],
                }
                for hit in derived["complete_object_locators"]
            ],
            "derived_vtables": [
                {"addr": hx(value), "rva": hx(value - p.module_base)} for value in derived["vtables"]
            ],
            "vtable_values": derived["vtables"],
        }
    return {
        "classes": classes,
        "module_chunk_count": len(chunks),
        "module_bytes_read": sum(len(blob) for _, blob in chunks),
    }


def process_info(p: winmem.Process) -> dict[str, Any]:
    return {
        "pid": p.pid,
        "exe_path": p.module_path,
        "module_base": hex(p.module_base),
        "module_size": p.module_size,
        "sha256": file_sha256(p.module_path),
    }


def read_roots(p: winmem.Process) -> dict[str, Any]:
    module = p.module_base
    game_state = u64(p, module + CG_STATE_DATE_OFFSET)
    country_database = u64(p, module + COUNTRY_DATABASE_OFFSET)
    result: dict[str, Any] = {
        "game_state_ptr_source": hex(module + CG_STATE_DATE_OFFSET),
        "country_database_ptr_source": hex(module + COUNTRY_DATABASE_OFFSET),
        "game_state": hex(game_state or 0),
        "country_database": hex(country_database or 0),
        "status": "unreadable",
    }
    if not (game_state and is_heap(game_state) and country_database and is_heap(country_database)):
        return result

    tag_table = u64(p, country_database)
    tag_count = u32(p, country_database + 0x08)
    active_count = u32(p, country_database + 0x0C)
    country_table = u64(p, game_state + CGAMESTATE_COUNTRY_TABLE)
    country_count = u32(p, game_state + CGAMESTATE_COUNTRY_COUNT)

    result.update(
        {
            "tag_table": hex(tag_table or 0),
            "tag_count": tag_count,
            "active_count": active_count,
            "country_table": hex(country_table or 0),
            "country_count": country_count,
            "status": "read",
        }
    )
    return result


def load_tags(p: winmem.Process, roots: dict[str, Any]) -> dict[int, str]:
    tag_table = int(roots.get("tag_table", "0"), 16)
    tag_count = int(roots.get("tag_count") or 0)
    tags: dict[int, str] = {}
    if not is_heap(tag_table):
        return tags
    for tag_id in range(min(tag_count, 2048)):
        tag = read_inline_tag(p, tag_table, tag_id)
        if tag and all(32 <= ord(ch) <= 126 for ch in tag):
            tags[tag_id] = tag
    return tags


def validate_country_vtable(
    p: winmem.Process,
    roots: dict[str, Any],
    tags: dict[int, str],
    rva: int,
) -> dict[str, Any]:
    country_table = int(roots.get("country_table", "0"), 16)
    country_count = int(roots.get("country_count") or 0)
    expected_vtable = p.module_base + rva
    valid: dict[int, int] = {}
    bad_vtable = 0
    bad_tag_echo = 0
    unreadable = 0
    samples: list[dict[str, Any]] = []

    if not is_heap(country_table):
        return {
            "rva": hex(rva),
            "expected_vtable": hex(expected_vtable),
            "valid_country_count": 0,
            "status": "country_table_unreadable",
        }

    for tag_id in sorted(tags):
        if tag_id >= country_count:
            continue
        ptr = u64(p, country_table + tag_id * 8)
        if not ptr or not is_heap(ptr):
            unreadable += 1
            continue
        vt = u64(p, ptr)
        if vt != expected_vtable:
            bad_vtable += 1
            continue
        echo = u32(p, ptr + OBJ_TAG_DB_ID)
        if echo != tag_id:
            bad_tag_echo += 1
            continue
        valid[tag_id] = ptr
        if len(samples) < 12:
            samples.append(
                {
                    "tag_id": tag_id,
                    "tag": tags.get(tag_id),
                    "country_ptr": hex(ptr),
                    "name": maybe_obj_name(p, ptr),
                }
            )

    status = "validated" if len(valid) >= 40 else "rejected_or_unvalidated"
    return {
        "rva": hex(rva),
        "expected_vtable": hex(expected_vtable),
        "valid_country_count": len(valid),
        "bad_vtable_count": bad_vtable,
        "bad_tag_echo_count": bad_tag_echo,
        "unreadable_or_null_count": unreadable,
        "samples": samples,
        "status": status,
        "countries": {str(k): v for k, v in valid.items()},
    }


def probe_factions(p: winmem.Process, countries: dict[int, int], tags: dict[int, str]) -> dict[str, Any]:
    groups: dict[int, list[int]] = {}
    unreadable = 0
    for tag_id, country in sorted(countries.items()):
        group = i32(p, country + OBJ_FACTION_GROUP)
        if group is None:
            unreadable += 1
            continue
        groups.setdefault(group, []).append(tag_id)

    multi = {g: ids for g, ids in groups.items() if g > 0 and len(ids) > 1}
    samples = []
    for group, ids in sorted(multi.items(), key=lambda item: (-len(item[1]), item[0]))[:10]:
        samples.append(
            {
                "group": group,
                "size": len(ids),
                "tags": [tags.get(i, str(i)) for i in ids[:16]],
            }
        )
    return {
        "offset": hex(OBJ_FACTION_GROUP),
        "countries_read": len(countries) - unreadable,
        "unreadable": unreadable,
        "distinct_groups": len(groups),
        "nonzero_groups": sum(1 for g in groups if g > 0),
        "multi_member_nonzero_groups": len(multi),
        "samples": samples,
        "status": "validated_structural" if len(countries) - unreadable >= 40 else "unvalidated",
    }


def collect_relation_entries(
    p: winmem.Process,
    countries: dict[int, int],
    target_ids: list[int],
) -> tuple[list[tuple[int, int, int]], dict[str, Any]]:
    entries: list[tuple[int, int, int]] = []
    manager_ok = 0
    manager_bad = 0
    table_ok = 0
    table_bad = 0
    relation_null = 0
    relation_unreadable = 0
    opinions_plausible = 0
    opinions_nonzero = 0
    opinion_min: int | None = None
    opinion_max: int | None = None
    samples: list[dict[str, Any]] = []

    for src_id, country in sorted(countries.items()):
        mgr = u64(p, country + OBJ_RELATION_MANAGER)
        if not mgr or not is_heap(mgr):
            manager_bad += 1
            continue
        manager_ok += 1
        table = u64(p, mgr + RELATION_TABLE_OFFSET)
        if not table or not is_heap(table):
            table_bad += 1
            continue
        table_ok += 1
        for target_id in target_ids:
            if target_id == src_id:
                continue
            rel = u64(p, table + target_id * 8)
            if not rel:
                relation_null += 1
                continue
            if not is_heap(rel):
                relation_unreadable += 1
                continue
            op = i32(p, rel + REL_OPINION)
            if op is not None and -200 <= op <= 200:
                opinions_plausible += 1
                if op != 0:
                    opinions_nonzero += 1
                opinion_min = op if opinion_min is None else min(opinion_min, op)
                opinion_max = op if opinion_max is None else max(opinion_max, op)
                entries.append((src_id, target_id, rel))
                if len(samples) < 12 and (op != 0 or len(samples) < 4):
                    samples.append(
                        {
                            "src_id": src_id,
                            "target_id": target_id,
                            "relation_ptr": hex(rel),
                            "opinion": op,
                        }
                    )

    stats = {
        "country_relation_manager_offset": hex(OBJ_RELATION_MANAGER),
        "manager_ok": manager_ok,
        "manager_bad": manager_bad,
        "relation_table_offset": hex(RELATION_TABLE_OFFSET),
        "table_ok": table_ok,
        "table_bad": table_bad,
        "relation_null": relation_null,
        "relation_unreadable_or_nonheap": relation_unreadable,
        "relation_entries_with_plausible_opinion": opinions_plausible,
        "nonzero_opinion_entries": opinions_nonzero,
        "opinion_offset": hex(REL_OPINION),
        "opinion_min": opinion_min,
        "opinion_max": opinion_max,
        "samples": samples,
        "status": "validated_structural" if manager_ok >= 40 and opinions_plausible >= 1000 else "unvalidated",
    }
    return entries, stats


def candidate_flag_vtables(module_base: int) -> dict[int, dict[str, Any]]:
    candidates: dict[int, dict[str, Any]] = {}
    for name, old_rva in OLD_RELATION_FLAG_VTABLE_RVAS.items():
        for set_name, rva in (("old_16022851_rva", old_rva), ("old_plus_country_delta", old_rva + COUNTRY_RVA_DELTA)):
            candidates[module_base + rva] = {"name": name, "set": set_name, "rva": rva}
    return candidates


def current_rtti_flag_vtables(rtti: dict[str, Any]) -> dict[int, dict[str, Any]]:
    candidates: dict[int, dict[str, Any]] = {}
    for name, info in rtti.get("classes", {}).items():
        for value in info.get("vtable_values", []):
            if isinstance(value, int):
                candidates[value] = {"name": name, "set": "current_870ebf_rtti", "rva": value - 0}
    return candidates


def read_subject_candidate(p: winmem.Process, subj_ptr: int, tags: dict[int, str]) -> dict[str, Any] | None:
    overlord = i32(p, subj_ptr + SUBJ_OVERLORD)
    subject = i32(p, subj_ptr + SUBJ_SUBJECT)
    ratio = i64(p, subj_ptr + SUBJ_RATIO)
    state_db = u64(p, subj_ptr + SUBJ_STATE_DB)
    if overlord not in tags or subject not in tags or ratio is None or not 0 <= ratio <= 100_000:
        return None
    state_name = None
    if state_db and is_heap(state_db) and locate is not None:
        try:
            state_name = locate.read_msvc_str(p, state_db + SUBJ_STATE_NAME, maxlen=96)
        except Exception:
            state_name = None
    return {
        "subject_relation_ptr": hex(subj_ptr),
        "overlord_id": overlord,
        "overlord_tag": tags.get(overlord),
        "subject_id": subject,
        "subject_tag": tags.get(subject),
        "autonomy_state_db": hex(state_db or 0),
        "autonomy_state": state_name,
        "autonomy_ratio_raw": ratio,
    }


def scan_relation_flag_candidates(
    p: winmem.Process,
    entries: list[tuple[int, int, int]],
    tags: dict[int, str],
    rtti: dict[str, Any],
) -> dict[str, Any]:
    vt_candidates = candidate_flag_vtables(p.module_base)
    rtti_candidates = current_rtti_flag_vtables(rtti)
    for vt, meta in rtti_candidates.items():
        vt_candidates[vt] = {"name": meta["name"], "set": meta["set"], "rva": vt - p.module_base}
    counts: dict[str, dict[str, int]] = {}
    samples: list[dict[str, Any]] = []
    subjects: dict[tuple[int, int], dict[str, Any]] = {}
    scanned_qwords = 0
    heap_ptrs = 0
    unique_flag_ptrs: set[int] = set()
    unique_by_key: set[tuple[int, int, int, str, str]] = set()

    for src_id, target_id, rel in entries:
        data = read_exact(p, rel, REL_SCAN_BYTES)
        if data is None:
            continue
        for off in range(0, REL_SCAN_BYTES - 7, 8):
            scanned_qwords += 1
            ptr = qword_at(data, off)
            if not is_heap(ptr):
                continue
            heap_ptrs += 1
            vt = u64(p, ptr)
            meta = vt_candidates.get(vt or 0)
            if not meta:
                continue
            key = (src_id, target_id, off, meta["name"], meta["set"])
            if key in unique_by_key:
                continue
            unique_by_key.add(key)
            unique_flag_ptrs.add(ptr)
            by_set = counts.setdefault(meta["set"], {})
            by_set[meta["name"]] = by_set.get(meta["name"], 0) + 1
            if meta["name"] == "subject_or_puppet":
                subject = read_subject_candidate(p, ptr, tags)
                if subject:
                    subjects[(subject["overlord_id"], subject["subject_id"])] = subject
            if len(samples) < 20:
                samples.append(
                    {
                        "src_id": src_id,
                        "src_tag": tags.get(src_id),
                        "target_id": target_id,
                        "target_tag": tags.get(target_id),
                        "relation_ptr": hex(rel),
                        "relation_window_offset": hex(off),
                        "flag_ptr": hex(ptr),
                        "flag_vtable": hex(vt or 0),
                        "flag_rva": hex(meta["rva"]),
                        "flag_name": meta["name"],
                        "candidate_set": meta["set"],
                    }
                )

    shifted_hits = sum(counts.get("old_plus_country_delta", {}).values())
    old_hits = sum(counts.get("old_16022851_rva", {}).values())
    current_hits = sum(counts.get("current_870ebf_rtti", {}).values())
    if current_hits > 0:
        status = "validated_current_rtti"
    elif shifted_hits > 0:
        status = "validated_shifted_candidates"
    elif old_hits > 0:
        status = "validated_old_candidates"
    else:
        status = "not_observed"
    return {
        "rtti": {
            "module_chunk_count": rtti.get("module_chunk_count"),
            "module_bytes_read": rtti.get("module_bytes_read"),
            "classes": {
                name: {
                    key: value
                    for key, value in info.items()
                    if key != "vtable_values"
                }
                for name, info in rtti.get("classes", {}).items()
            },
        },
        "scan_relation_entries": len(entries),
        "scan_bytes_per_relation": REL_SCAN_BYTES,
        "scanned_qwords": scanned_qwords,
        "heap_qwords": heap_ptrs,
        "candidate_delta_from_country_vtable": hex(COUNTRY_RVA_DELTA),
        "candidate_sets": {
            "old_16022851_rva": {k: hex(v) for k, v in OLD_RELATION_FLAG_VTABLE_RVAS.items()},
            "old_plus_country_delta": {k: hex(v + COUNTRY_RVA_DELTA) for k, v in OLD_RELATION_FLAG_VTABLE_RVAS.items()},
            "current_870ebf_rtti": {
                name: [row["rva"] for row in info.get("derived_vtables", [])]
                for name, info in rtti.get("classes", {}).items()
            },
        },
        "hits_by_candidate_set": counts,
        "unique_flag_ptrs": len(unique_flag_ptrs),
        "subjects": sorted(subjects.values(), key=lambda item: (item["overlord_tag"] or "", item["subject_tag"] or "")),
        "samples": samples,
        "status": status,
    }


def scan_war_relations(p: winmem.Process, tags: dict[int, str], max_hits: int = 20000) -> dict[str, Any]:
    valid_rows: dict[int, dict[str, Any]] = {}
    typeptrs = {
        p.module_base + WR_GAME_DATE_VTABLE_RVA: "game_date",
        p.module_base + WR_GREGORIAN_DATE_VTABLE_RVA: "gregorian_date",
    }
    needles = {struct.pack("<Q", ptr): (ptr, name) for ptr, name in typeptrs.items()}
    candidate_hits = 0
    shape_rejects = 0
    read_failures = 0
    regions_scanned = 0
    bytes_scanned = 0

    for base, size in p.iter_regions(HEAP_LO, HEAP_HI, want_exec=False):
        size = int(size)
        base = int(base)
        if size <= 0:
            continue
        regions_scanned += 1
        try:
            data = p.read(base, size)
        except Exception:
            read_failures += 1
            continue
        bytes_scanned += len(data)
        for needle, (typeptr, type_name) in needles.items():
            start = 0
            while True:
                off = data.find(needle, start)
                if off < 0:
                    break
                candidate_hits += 1
                addr = base + off - WR_TYPEPTR
                start = off + 1
                if addr in valid_rows:
                    continue
                row = validate_war_row(data, base, addr, typeptr, type_name, tags)
                if row is None:
                    shape_rejects += 1
                    continue
                valid_rows[addr] = row
                if len(valid_rows) >= max_hits:
                    break
            if len(valid_rows) >= max_hits:
                break
        if len(valid_rows) >= max_hits:
            break

    samples = list(valid_rows.values())[:20]
    return {
        "war_relation_offsets": {
            "first_tag_id": hex(WR_FIRST_TAGID),
            "second_tag_id": hex(WR_SECOND_TAGID),
            "type_ptr": hex(WR_TYPEPTR),
            "first_casualties": hex(WR_FIRST_CAS),
            "second_casualties": hex(WR_SECOND_CAS),
            "echo": hex(WR_ECHO),
        },
        "typeptr_vtable_rvas": {
            "game_date": hex(WR_GAME_DATE_VTABLE_RVA),
            "gregorian_date": hex(WR_GREGORIAN_DATE_VTABLE_RVA),
        },
        "typeptr_addresses": {name: hex(ptr) for ptr, name in typeptrs.items()},
        "regions_scanned": regions_scanned,
        "bytes_scanned": bytes_scanned,
        "typeptr_hits": candidate_hits,
        "shape_rejects": shape_rejects,
        "read_failures": read_failures,
        "valid_rows": len(valid_rows),
        "samples": samples,
        "status": "validated" if valid_rows else "not_observed",
    }


def validate_war_row(
    region_data: bytes,
    region_base: int,
    addr: int,
    typeptr: int,
    type_name: str,
    tags: dict[int, str],
) -> dict[str, Any] | None:
    off = addr - region_base
    if off < 0 or off + 0x70 > len(region_data):
        return None
    first = struct.unpack_from("<I", region_data, off + WR_FIRST_TAGID)[0]
    second = struct.unpack_from("<I", region_data, off + WR_SECOND_TAGID)[0]
    seen_typeptr = struct.unpack_from("<Q", region_data, off + WR_TYPEPTR)[0]
    first_cas = struct.unpack_from("<q", region_data, off + WR_FIRST_CAS)[0]
    second_cas = struct.unpack_from("<q", region_data, off + WR_SECOND_CAS)[0]
    echo = struct.unpack_from("<I", region_data, off + WR_ECHO)[0]
    if seen_typeptr != typeptr:
        return None
    if first not in tags or second not in tags or first == second:
        return None
    if echo != first:
        return None
    if not (0 <= first_cas <= WR_MAX_CASUALTIES and 0 <= second_cas <= WR_MAX_CASUALTIES):
        return None
    return {
        "row_ptr": hex(addr),
        "type": type_name,
        "first_id": first,
        "first_tag": tags.get(first),
        "second_id": second,
        "second_tag": tags.get(second),
        "first_casualties": first_cas,
        "second_casualties": second_cas,
        "echo": echo,
    }


def build_summary(report: dict[str, Any]) -> dict[str, Any]:
    country_candidates = report["country_vtable_candidates"]
    current_country = country_candidates["current_870ebf"]
    old_country = country_candidates["old_16022851"]
    relation = report.get("relations", {})
    flags = report.get("relation_flags", {})
    wars = report.get("wars", {})
    factions = report.get("factions", {})

    accepted: list[str] = []
    rejected: list[str] = []
    inconclusive: list[str] = []

    if current_country.get("status") == "validated":
        accepted.append("COUNTRY_VTABLE_RVA_870EBF=0x27D0E90 via country table + tag echo")
    else:
        inconclusive.append("COUNTRY_VTABLE_RVA_870EBF")
    if old_country.get("valid_country_count") == 0 and current_country.get("valid_country_count", 0) >= 40:
        rejected.append("OLD_COUNTRY_VTABLE_RVA_16022851=0x27C0E80 for current 870ebf")
    else:
        inconclusive.append("OLD_COUNTRY_VTABLE_RVA_16022851")

    if factions.get("status") == "validated_structural":
        accepted.append("OBJ_FACTION_GROUP=0x598 structural country-field root")
    else:
        inconclusive.append("OBJ_FACTION_GROUP=0x598")

    if relation.get("status") == "validated_structural":
        accepted.append("COUNTRY+0xF88 relation manager, manager+0x08 table, relation+0x300 opinion")
    else:
        inconclusive.append("relation manager/opinion root")

    if flags.get("status") == "validated_current_rtti":
        accepted.append("relation flag vtables derived from current 870ebf RTTI and observed in live relation entries")
        if flags.get("subjects"):
            accepted.append("CSubjectRelation current RTTI vtable and +0x10/+0x14/+0x58/+0x60 subject fields")
        if not flags.get("hits_by_candidate_set", {}).get("old_16022851_rva"):
            rejected.append("unshifted 16022851 relation flag vtables for current 870ebf")
        if not flags.get("hits_by_candidate_set", {}).get("old_plus_country_delta"):
            rejected.append("old relation flag vtables shifted only by country delta for current 870ebf")
    elif flags.get("status") == "validated_shifted_candidates":
        accepted.append("relation flag vtables = 16022851 RVAs + 0x10010 country-vtable delta")
        if not flags.get("hits_by_candidate_set", {}).get("old_16022851_rva"):
            rejected.append("unshifted 16022851 relation flag vtables for current 870ebf")
    elif flags.get("status") == "validated_old_candidates":
        accepted.append("unshifted 16022851 relation flag vtables observed on current 870ebf")
    else:
        inconclusive.append("relation flag vtable candidates")

    if wars.get("status") == "validated":
        accepted.append("war relation rows via WR_* offsets and date typeptr RVAs")
    else:
        inconclusive.append("war relation rows not observed by bounded typeptr scan")

    return {
        "accepted_current_build_evidence": accepted,
        "rejected_current_build_candidates": rejected,
        "inconclusive_or_not_observed": inconclusive,
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True, help="JSON proof output path")
    ap.add_argument("--skip-war-scan", action="store_true")
    ns = ap.parse_args()

    out_path = Path(ns.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)

    p = winmem.open_process("hoi4.exe")
    if p is None:
        raise SystemExit("hoi4.exe not running")
    with p:
        started = time.time()
        info = process_info(p)
        report: dict[str, Any] = {
            "schema_version": 1,
            "lane": "diplomacy_intel_operatives_live",
            "build_target": "870ebf",
            "expected_sha256": EXPECTED_SHA256,
            "started_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(started)),
            "process": info,
            "fail_closed": False,
        }

        if info.get("sha256") != EXPECTED_SHA256:
            report["fail_closed"] = True
            report["failure"] = "build_sha_mismatch"
            report["completed_utc"] = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
            out_path.write_text(json.dumps(report, indent=2, sort_keys=True), encoding="utf-8")
            return 2

        roots = read_roots(p)
        tags = load_tags(p, roots)
        report["roots"] = roots
        report["tag_table"] = {
            "tag_count_read": len(tags),
            "samples": [{"tag_id": k, "tag": v} for k, v in list(sorted(tags.items()))[:16]],
        }

        current_country = validate_country_vtable(p, roots, tags, COUNTRY_VTABLE_RVA_870EBF)
        old_country = validate_country_vtable(p, roots, tags, OLD_COUNTRY_VTABLE_RVA_16022851)
        current_countries = {int(k): int(v) for k, v in current_country.get("countries", {}).items()}
        target_ids = sorted(current_countries)
        current_country.pop("countries", None)
        old_country.pop("countries", None)
        report["country_vtable_candidates"] = {
            "current_870ebf": current_country,
            "old_16022851": old_country,
            "country_vtable_rva_delta": hex(COUNTRY_RVA_DELTA),
        }

        report["factions"] = probe_factions(p, current_countries, tags)

        relation_entries, relation_stats = collect_relation_entries(p, current_countries, target_ids)
        for sample in relation_stats.get("samples", []):
            sample["src_tag"] = tags.get(sample["src_id"])
            sample["target_tag"] = tags.get(sample["target_id"])
        report["relations"] = relation_stats
        relation_flag_rtti = derive_relation_flag_rtti(p)
        report["relation_flags"] = scan_relation_flag_candidates(p, relation_entries, tags, relation_flag_rtti)

        if ns.skip_war_scan:
            report["wars"] = {"status": "skipped", "reason": "--skip-war-scan"}
        else:
            report["wars"] = scan_war_relations(p, tags)

        report["summary"] = build_summary(report)
        report["completed_utc"] = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
        report["elapsed_seconds"] = round(time.time() - started, 3)
        out_path.write_text(json.dumps(report, indent=2, sort_keys=True), encoding="utf-8")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
