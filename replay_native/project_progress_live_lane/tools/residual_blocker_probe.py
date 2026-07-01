"""Lane-local residual probe for project progress reward/start fields.

Read-only development probe. It does not feed the production hot loop. The
intent is to bound the remaining project-progress backlog before adding any new
domain-41 fields:

* rediscover the current-build CPrototypeReward vtable through MSVC RTTI;
* enumerate reward objects and test bounded anchor windows for reachable roots;
* scan CProjectStateMachine bytes for simple start/completed boolean candidates.
"""

import argparse
import json
import os
import struct
import sys
import time
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[3]
MEMORY_READER = REPO_ROOT / "memory_reader"
sys.path.insert(0, str(MEMORY_READER))

import projects_progress_probe as pp  # noqa: E402
import winmem  # noqa: E402


HEAP_LO = pp.HEAP_LO
HEAP_HI = pp.HEAP_HI

LEGACY_REWARD_RVA = 0x26F5148
LEGACY_PROJECT_RVA = 0x295C190
LEGACY_STATE_MACHINE_RVA = 0x2A087C8
LEGACY_PROTOTYPE_STATE_RVA = 0x2A08770
LEGACY_BREAKTHROUGH_RVA = 0x2A046B0

REWARD_RTTI_NAME = b".?AVCPrototypeReward@NProject@@\x00"


def hx(value):
    return pp.hx(value)


def read_u32_blob(blob, offset):
    if offset + 4 > len(blob):
        return None
    return struct.unpack_from("<I", blob, offset)[0]


def read_u64_blob(blob, offset):
    if offset + 8 > len(blob):
        return None
    return struct.unpack_from("<Q", blob, offset)[0]


def scan_process_bytes(p, min_addr, max_addr, needle, want_exec=None, chunk=8 << 20):
    """Return absolute addresses for a byte pattern in readable regions."""
    if not needle:
        return []
    overlap = max(0, len(needle) - 1)
    hits = []
    for rbase, rsize in p.iter_regions(min_addr, max_addr, want_exec=want_exec):
        pos = 0
        while pos < rsize:
            want = min(chunk, rsize - pos)
            blob = p.try_read(rbase + pos, want)
            if blob:
                idx = blob.find(needle)
                while idx != -1:
                    hits.append(rbase + pos + idx)
                    idx = blob.find(needle, idx + 1)
                if pos + len(blob) >= rsize or len(blob) < want:
                    pos += max(len(blob), 0x1000)
                else:
                    pos += max(1, len(blob) - overlap)
            else:
                pos += max(want, 0x1000)
    return hits


def find_msvc_rtti_vtables(p, rtti_name):
    """Find MSVC x64 vftables for a type name in the loaded module."""
    base = p.module_base
    end = base + p.module_size
    string_hits = scan_process_bytes(p, base, end, rtti_name, want_exec=False)
    out = []
    seen_vtables = set()
    for name_addr in string_hits:
        type_desc = name_addr - 0x10
        if not p.in_module(type_desc):
            continue
        type_desc_rva = type_desc - base
        rva_hits = scan_process_bytes(
            p, base, end, struct.pack("<I", type_desc_rva), want_exec=False
        )
        for rva_hit in rva_hits:
            col_addr = rva_hit - 0x0C
            col_blob = p.try_read(col_addr, 0x18)
            if len(col_blob) != 0x18:
                continue
            sig, offset, cd_offset, p_type, p_class, p_self = struct.unpack(
                "<IIIIII", col_blob
            )
            if p_type != type_desc_rva or p_self != col_addr - base:
                continue
            if sig not in (0, 1) or offset > 0x4000 or cd_offset > 0x4000:
                continue
            col_refs = scan_process_bytes(
                p, base, end, struct.pack("<Q", col_addr), want_exec=False
            )
            for col_ref in col_refs:
                vtable = col_ref + 8
                first_fn = pp.try_u64(p, vtable)
                if first_fn is None or not p.in_module(first_fn):
                    continue
                if vtable in seen_vtables:
                    continue
                seen_vtables.add(vtable)
                out.append(
                    {
                        "rtti_name_addr": name_addr,
                        "type_desc_rva": type_desc_rva,
                        "complete_object_locator_rva": col_addr - base,
                        "vtable_rva": vtable - base,
                        "first_function_rva": first_fn - base,
                    }
                )
    out.sort(key=lambda row: row["vtable_rva"])
    return out


def current_reward_candidates(module_base, rtti_vtables):
    candidates = []
    for row in rtti_vtables:
        candidates.append(
            (
                "rtti_CPrototypeReward_%s" % hx(row["vtable_rva"]),
                module_base + row["vtable_rva"],
            )
        )
    shifted = [
        (
            "legacy_CPrototypeReward_0x26F5148",
            LEGACY_REWARD_RVA,
        ),
        (
            "legacy_plus_CProject_shift",
            LEGACY_REWARD_RVA + (pp.PROJECT_CPROJECT_VTABLE_RVA - LEGACY_PROJECT_RVA),
        ),
        (
            "legacy_plus_CProjectStateMachine_shift",
            LEGACY_REWARD_RVA
            + (pp.PROJECT_STATE_MACHINE_VTABLE_RVA - LEGACY_STATE_MACHINE_RVA),
        ),
        (
            "legacy_plus_CPrototypeProjectState_shift",
            LEGACY_REWARD_RVA
            + (pp.PROJECT_PROTOTYPE_STATE_VTABLE_RVA - LEGACY_PROTOTYPE_STATE_RVA),
        ),
        (
            "legacy_plus_CBreakthroughProgress_shift",
            LEGACY_REWARD_RVA
            + (pp.PROJECT_BREAKTHROUGH_PROGRESS_VTABLE_RVA - LEGACY_BREAKTHROUGH_RVA),
        ),
    ]
    for label, rva in shifted:
        candidates.append((label, module_base + rva))

    unique = []
    seen = set()
    for label, addr in candidates:
        if addr in seen:
            continue
        seen.add(addr)
        unique.append((label, addr))
    return unique


def heap_scan_vtable_hits(p, candidates, max_hits_per_label):
    needles = [
        (label, addr, struct.pack("<Q", addr))
        for label, addr in candidates
        if addr is not None
    ]
    hits = {label: [] for label, _, _ in needles}
    truncated = {label: False for label, _, _ in needles}
    bytes_read = 0
    regions = 0
    chunk = 8 << 20
    for rbase, rsize in p.iter_regions(HEAP_LO, HEAP_HI, want_exec=False):
        regions += 1
        pos = 0
        while pos < rsize:
            want = min(chunk, rsize - pos)
            blob = p.try_read(rbase + pos, want)
            if blob:
                bytes_read += len(blob)
                for label, _addr, needle in needles:
                    if truncated[label]:
                        continue
                    idx = blob.find(needle)
                    while idx != -1:
                        obj_addr = rbase + pos + idx
                        if obj_addr % 8 == 0:
                            hits[label].append(obj_addr)
                            if len(hits[label]) >= max_hits_per_label:
                                truncated[label] = True
                                break
                        idx = blob.find(needle, idx + 1)
                if pos + len(blob) >= rsize or len(blob) < want:
                    pos += max(len(blob), 0x1000)
                else:
                    pos += max(1, len(blob) - 7)
            else:
                pos += max(want, 0x1000)
    return hits, truncated, {"regions": regions, "bytes_read": bytes_read}


def collect_anchored_context(p, max_projects):
    module_base = p.module_base
    expected = {
        "CProjectStateMachine": module_base + pp.PROJECT_STATE_MACHINE_VTABLE_RVA,
        "CPrototypeProjectState": module_base + pp.PROJECT_PROTOTYPE_STATE_VTABLE_RVA,
        "CSimpleProjectState": module_base + pp.PROJECT_SIMPLE_STATE_VTABLE_RVA,
        "CBreakthroughProgress": module_base
        + pp.PROJECT_BREAKTHROUGH_PROGRESS_VTABLE_RVA,
    }
    country_vtable = module_base + pp.COUNTRY_VTABLE_RVA
    status_a_vtable = module_base + pp.PROJECT_PROGRAM_STATUS_A_VTABLE_RVA
    status_b_vtable = module_base + pp.PROJECT_PROGRAM_STATUS_B_VTABLE_RVA
    pool_vtable = module_base + pp.PROJECT_POOL_VTABLE_RVA
    project_vtable = module_base + pp.PROJECT_CPROJECT_VTABLE_RVA

    game_state = pp.try_u64(p, module_base + pp.CG_STATE_DATE_OFFSET)
    country_database = pp.try_u64(p, module_base + pp.COUNTRY_DATABASE_OFFSET)
    if not pp.is_heap_ptr(game_state) or not pp.is_heap_ptr(country_database):
        raise RuntimeError("failed to resolve CGameState/country database")
    country_table = pp.try_u64(p, game_state + pp.CGAMESTATE_COUNTRY_TABLE)
    tag_table = pp.try_u64(p, country_database)
    row_count = pp.tag_row_count(p, country_database)
    if not row_count or not pp.is_heap_ptr(country_table) or not pp.is_heap_ptr(tag_table):
        raise RuntimeError("failed to validate country table")

    status_rows = []
    projects = []
    state_machine_invalid = 0
    current_state_invalid = 0
    current_state_hits = Counter()
    worked_hits = 0
    iter_nonzero = 0
    selector_completed_proxy = 0
    pool_status = Counter()

    for tag_db_id in range(1, row_count):
        country_ptr = pp.try_u64(p, country_table + tag_db_id * 8)
        if not country_ptr:
            continue
        if not pp.is_heap_ptr(country_ptr) or pp.try_u64(p, country_ptr) != country_vtable:
            continue
        if pp.try_u32(p, country_ptr + pp.OBJ_TAG_DB_ID) != tag_db_id:
            continue
        tag = pp.read_small_inline_string(p, tag_table + tag_db_id * 32)
        status_ptr = pp.try_u64(p, country_ptr + pp.COUNTRY_OFF_PROGRAM_STATUS)
        if (
            not pp.is_heap_ptr(status_ptr)
            or pp.try_u64(p, status_ptr) != status_a_vtable
            or pp.try_u64(p, status_ptr + 8) != status_b_vtable
        ):
            continue
        status_rows.append({"tag_db_id": tag_db_id, "tag": tag, "status_ptr": status_ptr})
        pool_ptr = pp.try_u64(p, status_ptr + pp.PROJECT_PROGRAM_STATUS_OFF_POOL)
        if (
            not pp.is_heap_ptr(pool_ptr)
            or pp.try_u64(p, pool_ptr) != pool_vtable
        ):
            pool_status["invalid_pool"] += 1
            continue
        rows, status = pp.sample_projects_from_pool(p, pool_ptr, project_vtable)
        pool_status[status] += 1
        if status != "valid":
            continue
        for row in rows:
            if len(projects) >= max_projects:
                break
            state_machine_ptr = row.get("state_machine_ptr")
            if (
                not pp.is_heap_ptr(state_machine_ptr)
                or pp.try_u64(p, state_machine_ptr) != expected["CProjectStateMachine"]
            ):
                state_machine_invalid += 1
                continue
            current = pp.classify_current_state(p, state_machine_ptr, expected)
            if not current["valid"]:
                current_state_invalid += 1
                continue
            row["tag_db_id"] = tag_db_id
            row["tag"] = tag
            row["status_ptr"] = status_ptr
            row["current_state"] = current
            row["worked_proxy"] = row.get("worked_at_facility") is not None
            row["started_progress_proxy"] = bool(row.get("iterations") or 0)
            row["completed_selector_proxy"] = (
                current["offset"] != pp.PROJECT_STATE_MACHINE_OFF_PROTOTYPE_STATE
                or current["enum"] != 0
            )
            projects.append(row)
            current_state_hits[(current["offset"], current["vtable"], current["enum"])] += 1
            if row["worked_proxy"]:
                worked_hits += 1
            if row["started_progress_proxy"]:
                iter_nonzero += 1
            if row["completed_selector_proxy"]:
                selector_completed_proxy += 1
        if len(projects) >= max_projects:
            break

    return {
        "expected_vtables": expected,
        "roots": {
            "game_state": game_state,
            "country_database": country_database,
            "country_table": country_table,
            "tag_row_count": row_count,
        },
        "status_rows": status_rows,
        "projects": projects,
        "pool_status": pool_status,
        "state_machine_invalid": state_machine_invalid,
        "current_state_invalid": current_state_invalid,
        "current_state_hits": current_state_hits,
        "worked_hits": worked_hits,
        "started_progress_proxy_hits": iter_nonzero,
        "completed_selector_proxy_hits": selector_completed_proxy,
    }


def date_to_hours(value, hour_shift=0):
    parts = str(value or "").split(".")
    if len(parts) != 4:
        return None
    year, month, day, hour = [int(part) for part in parts]
    base = datetime(1936, 1, 1)
    current = datetime(year, month, day)
    return (current - base).days * 24 + hour + hour_shift


def load_save_truth(path):
    if not path:
        return None
    with open(path, "r", encoding="utf-8") as fh:
        data = json.load(fh)
    projects = {}
    country_history_counts = Counter()
    history_events = []
    countries = data.get("countries") or {}
    for tag, country in countries.items():
        for project in country.get("projects") or []:
            project_id = project.get("id")
            if not isinstance(project_id, int):
                continue
            history = project.get("reward_history") or []
            projects[(tag, project_id)] = {
                "token": project.get("token"),
                "start": bool(project.get("start")),
                "completed": bool(project.get("completed")),
                "current_state": project.get("current_state"),
                "iterations": project.get("iterations"),
                "reward_history_count": len(history),
            }
            country_history_counts[tag] += len(history)
            for event in history:
                ctx = event.get("context") or {}
                history_events.append(
                    {
                        "tag": tag,
                        "project_id": project_id,
                        "token": project.get("token"),
                        "option": event.get("option"),
                        "prototype_reward": event.get("prototype_reward"),
                        "date": event.get("date"),
                        "date_hours": date_to_hours(event.get("date")),
                        "date_hours_minus_12": date_to_hours(event.get("date"), -12),
                        "date_hours_plus_12": date_to_hours(event.get("date"), 12),
                        "province": ctx.get("province"),
                        "progress": ctx.get("progress"),
                        "scientist_char_id": ctx.get("scientist_char_id"),
                    }
                )
    return {
        "path": path,
        "meta": data.get("meta") or {},
        "projects": projects,
        "country_history_counts": dict(country_history_counts),
        "history_events": history_events,
    }


def annotate_with_save_truth(context, save_truth):
    if not save_truth:
        return None
    matched = 0
    start_proxy_mismatches = []
    completed_proxy_mismatches = []
    live_keys = set()
    for row in context["projects"]:
        key = (row.get("tag"), row.get("project_id"))
        live_keys.add(key)
        truth = save_truth["projects"].get(key)
        if not truth:
            continue
        matched += 1
        row["save_start"] = truth["start"]
        row["save_completed"] = truth["completed"]
        row["save_token"] = truth["token"]
        row["save_reward_history_count"] = truth["reward_history_count"]
        if row["started_progress_proxy"] != truth["start"]:
            start_proxy_mismatches.append(
                {
                    "tag": row.get("tag"),
                    "project_id": row.get("project_id"),
                    "token": truth["token"],
                    "live_started_progress_proxy": row["started_progress_proxy"],
                    "save_start": truth["start"],
                    "iterations": row.get("iterations"),
                    "current_state_offset": hx(row["current_state"]["offset"]),
                    "current_state_enum": row["current_state"]["enum"],
                }
            )
        if row["completed_selector_proxy"] != truth["completed"]:
            completed_proxy_mismatches.append(
                {
                    "tag": row.get("tag"),
                    "project_id": row.get("project_id"),
                    "token": truth["token"],
                    "live_completed_selector_proxy": row["completed_selector_proxy"],
                    "save_completed": truth["completed"],
                    "iterations": row.get("iterations"),
                    "current_state_offset": hx(row["current_state"]["offset"]),
                    "current_state_enum": row["current_state"]["enum"],
                }
            )
    save_keys = set(save_truth["projects"].keys())
    return {
        "save_projects_json": save_truth["path"],
        "save_date": (save_truth["meta"].get("save_date") or save_truth["meta"].get("date")),
        "save_projects": len(save_keys),
        "live_projects": len(live_keys),
        "matched_by_tag_and_project_id": matched,
        "live_projects_missing_from_save": len(live_keys - save_keys),
        "save_projects_missing_from_live": len(save_keys - live_keys),
        "start_proxy_mismatches": len(start_proxy_mismatches),
        "start_proxy_mismatch_examples": start_proxy_mismatches[:24],
        "completed_proxy_mismatches": len(completed_proxy_mismatches),
        "completed_proxy_mismatch_examples": completed_proxy_mismatches[:24],
    }


def validated_facilities(p, facility_hits):
    out = []
    for addr in facility_hits:
        if pp.try_u32(p, addr + pp.PROJECT_PROJECT_OFF_TYPE) != 83:
            continue
        facility_id = pp.try_u32(p, addr + 0x0C)
        province_id = pp.try_u32(p, addr + 0x1C)
        if not facility_id or not province_id:
            continue
        out.append(
            {
                "addr": addr,
                "facility_id": facility_id,
                "province_id": province_id,
                "unread_prototype_rewards": pp.try_u32(p, addr + 0x178) or 0,
            }
        )
    return out


def scan_anchor_windows(p, anchors, reward_objects, one_hop_labels):
    reward_set = set(reward_objects)
    direct_hits = []
    one_hop_hits = []
    scanned = Counter()
    nested_cache = {}
    for anchor in anchors:
        addr = anchor["addr"]
        window = anchor["window"]
        label = anchor["kind"]
        blob = p.try_read(addr, window)
        if not blob:
            continue
        scanned[label] += 1
        for off in range(0, max(0, len(blob) - 7), 8):
            value = read_u64_blob(blob, off)
            if value in reward_set:
                direct_hits.append(
                    {
                        "anchor": anchor["name"],
                        "kind": label,
                        "offset": hx(off),
                        "reward_object": hx(value),
                    }
                )
                continue
            if label not in one_hop_labels or not pp.is_heap_ptr(value):
                continue
            if value not in nested_cache:
                nested_cache[value] = p.try_read(value, 0x200)
            nested = nested_cache[value]
            for nested_off in range(0, max(0, len(nested) - 7), 8):
                nested_value = read_u64_blob(nested, nested_off)
                if nested_value in reward_set:
                    one_hop_hits.append(
                        {
                            "anchor": anchor["name"],
                            "kind": label,
                            "offset": hx(off),
                            "container": hx(value),
                            "container_offset": hx(nested_off),
                            "reward_object": hx(nested_value),
                        }
                    )
    return {
        "scanned_anchor_counts": dict(sorted(scanned.items())),
        "direct_hits": direct_hits[:32],
        "direct_hit_count": len(direct_hits),
        "one_hop_hits": one_hop_hits[:32],
        "one_hop_hit_count": len(one_hop_hits),
    }


def best_reward_vector_candidate(p, begin, end, cap, reward_set):
    if not pp.is_heap_ptr(begin) or not pp.is_heap_ptr(end) or not pp.is_heap_ptr(cap):
        return None
    if not (begin <= end <= cap):
        return None
    size = end - begin
    if size == 0:
        return {
            "stride": None,
            "entries": 0,
            "matched_entries": 0,
            "reward_pointer_offsets": {},
            "objects": [],
        }
    best = None
    for stride in (0x18, 0x20, 0x28, 0x30, 0x38, 0x40):
        if size % stride != 0:
            continue
        entries = size // stride
        if entries > 512:
            continue
        pointer_offsets = Counter()
        objects = []
        matched_entries = 0
        for index in range(entries):
            entry_addr = begin + index * stride
            blob = p.try_read(entry_addr, stride)
            entry_matches = []
            for off in range(0, max(0, len(blob) - 7), 8):
                value = read_u64_blob(blob, off)
                if value in reward_set:
                    pointer_offsets[off] += 1
                    entry_matches.append((off, value))
            if entry_matches:
                matched_entries += 1
                for off, value in entry_matches:
                    objects.append(
                        {
                            "index": index,
                            "entry_addr": entry_addr,
                            "reward_pointer_offset": off,
                            "reward_object": value,
                        }
                    )
        candidate = {
            "stride": stride,
            "entries": entries,
            "matched_entries": matched_entries,
            "reward_pointer_offsets": dict(pointer_offsets),
            "objects": objects,
        }
        if best is None:
            best = candidate
            continue
        best_score = (
            best["matched_entries"],
            max(best["reward_pointer_offsets"].values() or [0]),
            -best["entries"],
        )
        score = (
            candidate["matched_entries"],
            max(candidate["reward_pointer_offsets"].values() or [0]),
            -candidate["entries"],
        )
        if score > best_score:
            best = candidate
    return best


def decode_reward_history_vectors(p, status_rows, reward_objects, save_truth=None):
    reward_set = set(reward_objects)
    rows = []
    total_entries = 0
    total_matched = 0
    invalid_headers = 0
    live_by_tag = Counter()
    pointer_offsets = Counter()
    all_objects = []
    for status in status_rows:
        status_ptr = status["status_ptr"]
        begin = pp.try_u64(p, status_ptr + 0x58)
        end = pp.try_u64(p, status_ptr + 0x60)
        cap = pp.try_u64(p, status_ptr + 0x68)
        decoded = best_reward_vector_candidate(p, begin, end, cap, reward_set)
        if decoded is None:
            invalid_headers += 1
            continue
        tag = status.get("tag")
        total_entries += decoded["entries"]
        total_matched += decoded["matched_entries"]
        live_by_tag[tag] += decoded["matched_entries"]
        for off, count in decoded["reward_pointer_offsets"].items():
            pointer_offsets[off] += count
        for obj in decoded["objects"]:
            all_objects.append(
                {
                    "tag": tag,
                    "tag_db_id": status["tag_db_id"],
                    **obj,
                }
            )
        if decoded["entries"] or decoded["matched_entries"]:
            rows.append(
                {
                    "tag": tag,
                    "tag_db_id": status["tag_db_id"],
                    "status_ptr": hx(status_ptr),
                    "begin": hx(begin),
                    "end": hx(end),
                    "cap": hx(cap),
                    "stride": hx(decoded["stride"]) if decoded["stride"] else None,
                    "entries": decoded["entries"],
                    "matched_reward_entries": decoded["matched_entries"],
                    "reward_pointer_offsets": {
                        hx(off): count
                        for off, count in sorted(
                            decoded["reward_pointer_offsets"].items()
                        )
                    },
                    "reward_objects": [
                        {
                            "index": obj["index"],
                            "entry_addr": hx(obj["entry_addr"]),
                            "reward_pointer_offset": hx(obj["reward_pointer_offset"]),
                            "reward_object": hx(obj["reward_object"]),
                        }
                        for obj in decoded["objects"][:16]
                    ],
                }
            )

    save_compare = None
    if save_truth:
        save_by_tag = Counter(save_truth["country_history_counts"])
        all_tags = sorted(set(save_by_tag) | set(live_by_tag))
        mismatches = []
        matches = 0
        for tag in all_tags:
            live_count = live_by_tag.get(tag, 0)
            save_count = save_by_tag.get(tag, 0)
            if live_count == save_count:
                matches += 1
            elif live_count or save_count:
                mismatches.append(
                    {"tag": tag, "live_history_entries": live_count, "save_history_entries": save_count}
                )
        save_compare = {
            "save_total_history_entries": sum(save_by_tag.values()),
            "live_total_history_entries": total_matched,
            "tag_count_matches": matches,
            "tag_count_mismatches": len(mismatches),
            "tag_mismatch_examples": mismatches[:32],
            "exact_total_match": sum(save_by_tag.values()) == total_matched,
            "exact_per_tag_match": not mismatches,
        }

    return {
        "header_offsets": {
            "begin": "0x58",
            "end": "0x60",
            "cap": "0x68",
        },
        "tested_strides": ["0x18", "0x20", "0x28", "0x30", "0x38", "0x40"],
        "invalid_headers": invalid_headers,
        "statuses_with_entries": len(rows),
        "total_entries": total_entries,
        "matched_reward_entries": total_matched,
        "reward_pointer_offsets": {
            hx(off): count for off, count in sorted(pointer_offsets.items())
        },
        "live_history_entries_by_tag": dict(sorted(live_by_tag.items())),
        "rows": rows[:64],
        "objects": all_objects,
        "save_compare": save_compare,
    }


def as_u32_value(value):
    if value is None:
        return 0
    ivalue = int(value)
    return ivalue & 0xFFFFFFFF


def reward_field_multiset_probe(p, vector_objects, save_truth):
    if not save_truth or not vector_objects:
        return None
    save_events = save_truth["history_events"]
    if len(save_events) != len(vector_objects):
        return {
            "compared": False,
            "reason": "live vector reward count does not match save history count",
            "live_vector_reward_count": len(vector_objects),
            "save_history_count": len(save_events),
        }

    live_by_offset = {off: Counter() for off in range(0x08, 0x80, 4)}
    live_by_tag_offset = {off: {} for off in range(0x08, 0x80, 4)}
    for obj in vector_objects:
        reward_object = obj["reward_object"]
        tag = obj["tag"]
        blob = p.try_read(reward_object, 0x80)
        if len(blob) < 0x80:
            continue
        for off in range(0x08, 0x80, 4):
            value = read_u32_blob(blob, off)
            live_by_offset[off][value] += 1
            live_by_tag_offset[off].setdefault(tag, Counter())[value] += 1

    save_fields = {
        "date_hours": Counter(as_u32_value(ev["date_hours"]) for ev in save_events),
        "date_hours_minus_12": Counter(
            as_u32_value(ev["date_hours_minus_12"]) for ev in save_events
        ),
        "date_hours_plus_12": Counter(
            as_u32_value(ev["date_hours_plus_12"]) for ev in save_events
        ),
        "province_i32": Counter(as_u32_value(ev["province"]) for ev in save_events),
        "progress_int": Counter(
            as_u32_value(round(float(ev["progress"] or 0.0))) for ev in save_events
        ),
        "scientist_char_id_zero_for_none": Counter(
            as_u32_value(ev["scientist_char_id"]) for ev in save_events
        ),
    }
    exact_matches = []
    for field, save_counter in save_fields.items():
        for off, live_counter in live_by_offset.items():
            if live_counter == save_counter:
                exact_matches.append({"save_field": field, "reward_offset": hx(off)})

    save_by_tag_fields = {}
    for field in save_fields:
        save_by_tag_fields[field] = {}
    for ev in save_events:
        tag = ev["tag"]
        save_by_tag_fields["date_hours"].setdefault(tag, Counter())[
            as_u32_value(ev["date_hours"])
        ] += 1
        save_by_tag_fields["date_hours_minus_12"].setdefault(tag, Counter())[
            as_u32_value(ev["date_hours_minus_12"])
        ] += 1
        save_by_tag_fields["date_hours_plus_12"].setdefault(tag, Counter())[
            as_u32_value(ev["date_hours_plus_12"])
        ] += 1
        save_by_tag_fields["province_i32"].setdefault(tag, Counter())[
            as_u32_value(ev["province"])
        ] += 1
        save_by_tag_fields["progress_int"].setdefault(tag, Counter())[
            as_u32_value(round(float(ev["progress"] or 0.0)))
        ] += 1
        save_by_tag_fields["scientist_char_id_zero_for_none"].setdefault(tag, Counter())[
            as_u32_value(ev["scientist_char_id"])
        ] += 1

    per_tag_matches = []
    for field, by_tag in save_by_tag_fields.items():
        for off, live_by_tag in live_by_tag_offset.items():
            if set(live_by_tag) != set(by_tag):
                continue
            if all(live_by_tag[tag] == by_tag[tag] for tag in by_tag):
                per_tag_matches.append({"save_field": field, "reward_offset": hx(off)})

    return {
        "compared": True,
        "live_vector_reward_count": len(vector_objects),
        "save_history_count": len(save_events),
        "tested_live_offsets": [hx(off) for off in sorted(live_by_offset)],
        "tested_save_fields": sorted(save_fields),
        "exact_multiset_matches": exact_matches,
        "exact_per_tag_multiset_matches": per_tag_matches,
        "accepted_complete_reward_history_fields": False,
        "field_blocker": (
            "Vector membership is count-matched before object-field pinning. "
            "A complete reward-history row still needs option/prototype_reward "
            "token handles and context fields, not just object vtable/date-like bytes."
        ),
    }


def state_machine_boolean_scan(p, projects, window, label_names=None, semantic_truth=False):
    if label_names is None:
        label_names = [
            "worked_proxy",
            "started_progress_proxy",
            "completed_selector_proxy",
        ]
    u8_values = [set() for _ in range(window)]
    u8_by_label = {
        name: [{False: set(), True: set()} for _ in range(window)]
        for name in label_names
    }
    u32_offsets = list(range(0, window - 3, 4))
    u32_values = {off: set() for off in u32_offsets}
    u32_by_label = {
        name: {off: {False: set(), True: set()} for off in u32_offsets}
        for name in label_names
    }
    readable = 0
    short_reads = 0
    label_counts = {name: Counter() for name in label_names}

    for row in projects:
        if any(name not in row for name in label_names):
            continue
        state_machine_ptr = row["state_machine_ptr"]
        blob = p.try_read(state_machine_ptr, window)
        if len(blob) != window:
            short_reads += 1
            continue
        readable += 1
        labels = {name: bool(row[name]) for name in label_names}
        for name, value in labels.items():
            label_counts[name][value] += 1
        for off, value in enumerate(blob):
            if value <= 1:
                u8_values[off].add(value)
                for name, label in labels.items():
                    u8_by_label[name][off][label].add(value)
            else:
                u8_values[off].add(value)
        for off in u32_offsets:
            value = read_u32_blob(blob, off)
            if value is None:
                continue
            if value <= 1:
                u32_values[off].add(value)
                for name, label in labels.items():
                    u32_by_label[name][off][label].add(value)
            else:
                u32_values[off].add(value)

    def candidate_offsets(values_by_offset):
        bool_offsets = []
        nonconstant = []
        for off, values in values_by_offset.items() if isinstance(values_by_offset, dict) else enumerate(values_by_offset):
            if values and values.issubset({0, 1}):
                bool_offsets.append(off)
                if len(values) > 1:
                    nonconstant.append(off)
        return bool_offsets, nonconstant

    def perfect_separators(by_label, offsets):
        out = {}
        for name in label_names:
            label_out = []
            for off in offsets:
                false_values = by_label[name][off][False]
                true_values = by_label[name][off][True]
                if (
                    len(false_values) == 1
                    and len(true_values) == 1
                    and false_values != true_values
                ):
                    label_out.append(
                        {
                            "offset": hx(off),
                            "false_value": next(iter(false_values)),
                            "true_value": next(iter(true_values)),
                        }
                    )
            out[name] = label_out[:32]
        return out

    u8_bool, u8_nonconstant = candidate_offsets(u8_values)
    u32_bool, u32_nonconstant = candidate_offsets(u32_values)
    u8_separators = perfect_separators(u8_by_label, u8_nonconstant)
    u32_separators = perfect_separators(u32_by_label, u32_nonconstant)
    perfect_total = sum(len(v) for v in u8_separators.values()) + sum(
        len(v) for v in u32_separators.values()
    )
    return {
        "window": hx(window),
        "readable_state_machines": readable,
        "short_reads": short_reads,
        "semantic_truth_available": bool(semantic_truth),
        "semantic_truth_note": (
            "save start/completed labels were used"
            if semantic_truth
            else (
                "current live fixture exposes only proxies: owner_facility_ref worked, "
                "CProject+0x74 nonzero progress, and current-state selector; no save "
                "labels for exact start/completed booleans are loaded here"
            )
        ),
        "proxy_label_counts": {
            name: {str(k).lower(): v for k, v in sorted(counter.items())}
            for name, counter in label_counts.items()
        },
        "u8_bool_offsets": len(u8_bool),
        "u8_nonconstant_bool_offsets": len(u8_nonconstant),
        "u8_nonconstant_examples": [hx(off) for off in u8_nonconstant[:32]],
        "u32_bool_offsets": len(u32_bool),
        "u32_nonconstant_bool_offsets": len(u32_nonconstant),
        "u32_nonconstant_examples": [hx(off) for off in u32_nonconstant[:32]],
        "perfect_proxy_separators_u8": u8_separators,
        "perfect_proxy_separators_u32": u32_separators,
        "perfect_proxy_separator_count": perfect_total,
        "accepted_exact_start_completed_fields": False,
        "bounded_rejection": True,
        "rejection_scope": (
            "Reject adding start/completed booleans from simple u8/u32 fields in "
            "CProjectStateMachine[0..window) for this fixture. Exact semantics "
            "remain blocked on a fixture with save/UI labels for start/completed."
            if not semantic_truth
            else "were tested against save labels and no perfect u8/u32 separator was found."
        ),
    }


def build_anchor_list(status_rows, projects, facilities):
    anchors = []
    for row in status_rows:
        anchors.append(
            {
                "kind": "CProgramStatus",
                "name": "tag_%d_status" % row["tag_db_id"],
                "addr": row["status_ptr"],
                "window": 0x100,
            }
        )
    for row in facilities:
        anchors.append(
            {
                "kind": "CProgramFacility",
                "name": "facility_%d" % row["facility_id"],
                "addr": row["addr"],
                "window": 0x240,
            }
        )
    for row in projects:
        anchors.append(
            {
                "kind": "CProject",
                "name": "project_%d" % row["project_id"],
                "addr": row["addr"],
                "window": pp.PROJECT_COUNTRY_STRIDE,
            }
        )
        anchors.append(
            {
                "kind": "CProjectStateMachine",
                "name": "project_%d_state_machine" % row["project_id"],
                "addr": row["state_machine_ptr"],
                "window": 0x280,
            }
        )
    return anchors


def probe(args):
    with winmem.Process(args.pid, "hoi4.exe") as p:
        fp = p.fingerprint()
        sha = str(fp.get("sha256") or "").lower()
        if sha != pp.EXPECTED_SHA256 and not args.force:
            raise SystemExit(
                "live hoi4.exe sha256 %s != expected %s; pass --force to probe anyway"
                % (sha, pp.EXPECTED_SHA256)
            )
        module_base = p.module_base

        save_truth = load_save_truth(args.save_projects_json)
        context = collect_anchored_context(p, args.max_projects)
        save_comparison = annotate_with_save_truth(context, save_truth)
        rtti_vtables = find_msvc_rtti_vtables(p, REWARD_RTTI_NAME)
        candidates = current_reward_candidates(module_base, rtti_vtables)
        candidates.append(("CProgram_facility", module_base + pp.PROJECT_FACILITY_VTABLE_RVA))
        heap_hits, truncated, heap_scan = heap_scan_vtable_hits(
            p, candidates, args.max_vtable_hits
        )

        facility_hits = heap_hits.get("CProgram_facility", [])
        facilities = validated_facilities(p, facility_hits)
        reward_labels = [
            label for label, _addr in candidates if label != "CProgram_facility"
        ]
        rtti_labels = [label for label in reward_labels if label.startswith("rtti_")]
        selected_labels = rtti_labels or reward_labels
        reward_objects = []
        for label in selected_labels:
            reward_objects.extend(heap_hits.get(label, []))
        reward_objects = sorted(set(reward_objects))

        anchors = build_anchor_list(
            context["status_rows"], context["projects"], facilities
        )
        anchor_scan = scan_anchor_windows(
            p,
            anchors,
            reward_objects,
            one_hop_labels={"CProgramStatus", "CProgramFacility"},
        )
        vector_decode = decode_reward_history_vectors(
            p, context["status_rows"], reward_objects, save_truth
        )
        reward_field_probe = reward_field_multiset_probe(
            p, vector_decode["objects"], save_truth
        )
        bool_scan = state_machine_boolean_scan(
            p, context["projects"], args.state_machine_window
        )
        save_bool_scan = None
        if save_truth:
            save_bool_scan = state_machine_boolean_scan(
                p,
                context["projects"],
                args.state_machine_window,
                label_names=["save_start", "save_completed"],
                semantic_truth=True,
            )

        reward_hit_counts = {
            label: {
                "vtable": hx(addr),
                "vtable_rva": hx(addr - module_base),
                "heap_objects": len(heap_hits.get(label, [])),
                "truncated": bool(truncated.get(label)),
            }
            for label, addr in candidates
        }
        reward_samples = []
        for addr in reward_objects[: args.example_objects]:
            blob = p.try_read(addr, 0x80)
            low_u32 = []
            if blob:
                for off in range(0x08, min(len(blob), 0x80) - 3, 4):
                    value = read_u32_blob(blob, off)
                    if value is not None and value <= 10_000_000:
                        low_u32.append({"offset": hx(off), "value": value})
            reward_samples.append(
                {
                    "addr": hx(addr),
                    "first_low_bounded_u32_fields": low_u32[:16],
                }
            )

        return {
            "schema": "project_progress_live_residual_probe_v1",
            "utc": datetime.now(timezone.utc).isoformat(),
            "pid": args.pid,
            "module_path": fp.get("path"),
            "module_base": hx(module_base),
            "sha256": sha,
            "roots": {key: hx(value) for key, value in context["roots"].items()},
            "expected_vtables": {
                key: hx(value) for key, value in context["expected_vtables"].items()
            },
            "counts": {
                "program_status_rows": len(context["status_rows"]),
                "sampled_projects": len(context["projects"]),
                "state_machine_invalid": context["state_machine_invalid"],
                "current_state_invalid": context["current_state_invalid"],
                "worked_proxy_projects": context["worked_hits"],
                "started_progress_proxy_projects": context[
                    "started_progress_proxy_hits"
                ],
                "completed_selector_proxy_projects": context[
                    "completed_selector_proxy_hits"
                ],
                "validated_facilities": len(facilities),
                "facilities_with_unread_rewards": sum(
                    1 for row in facilities if row["unread_prototype_rewards"] > 0
                ),
            },
            "pool_status": dict(sorted(context["pool_status"].items())),
            "current_state_selector_counts": [
                {
                    "offset": hx(off),
                    "vtable": vtable,
                    "enum": enum_value,
                    "count": count,
                }
                for (off, vtable, enum_value), count in sorted(
                    context["current_state_hits"].items()
                )
            ],
            "save_oracle_comparison": save_comparison,
            "reward_history": {
                "rtti_name": REWARD_RTTI_NAME.rstrip(b"\x00").decode("ascii"),
                "rtti_vtables": [
                    {
                        "vtable_rva": hx(row["vtable_rva"]),
                        "complete_object_locator_rva": hx(
                            row["complete_object_locator_rva"]
                        ),
                        "type_desc_rva": hx(row["type_desc_rva"]),
                        "first_function_rva": hx(row["first_function_rva"]),
                    }
                    for row in rtti_vtables
                ],
                "heap_scan": heap_scan,
                "candidate_vtable_hits": reward_hit_counts,
                "selected_reward_object_count": len(reward_objects),
                "selected_reward_object_samples": reward_samples,
                "anchor_windows": {
                    "CProgramStatus": "0x100 direct plus one-hop pointer payload 0x200",
                    "CProgramFacility": "0x240 direct plus one-hop pointer payload 0x200",
                    "CProject": "0x1C0 direct",
                    "CProjectStateMachine": "0x280 direct",
                },
                "anchor_scan": anchor_scan,
                "status_plus_0x58_vector": {
                    key: value
                    for key, value in vector_decode.items()
                    if key != "objects"
                },
                "object_field_multiset_probe": reward_field_probe,
                "accepted_reward_history_fields": False,
                "bounded_blocker": True,
                "blocker_scope": (
                    "CPrototypeReward objects and the CProgramStatus+0x58 vector are "
                    "located. Full reward-history rows are still blocked on byte-pinning "
                    "option/prototype_reward token handles and context fields; do not "
                    "add fired reward-history fields from object bytes until those pass."
                ),
            },
            "start_completed_boolean_scan": bool_scan,
            "start_completed_save_truth_scan": save_bool_scan,
            "examples": {
                "facilities_with_unread_rewards": [
                    {
                        "addr": hx(row["addr"]),
                        "facility_id": row["facility_id"],
                        "province_id": row["province_id"],
                        "unread_prototype_rewards": row["unread_prototype_rewards"],
                    }
                    for row in facilities
                    if row["unread_prototype_rewards"] > 0
                ][: args.example_objects],
                "worked_projects": [
                    {
                        "project_id": row["project_id"],
                        "static_def_handle": row["static_def_handle"],
                        "iterations": row["iterations"],
                        "state_machine_ptr": hx(row["state_machine_ptr"]),
                        "current_state_offset": hx(row["current_state"]["offset"]),
                        "current_state_enum": row["current_state"]["enum"],
                        "worked_at_facility": row["worked_at_facility"],
                    }
                    for row in context["projects"]
                    if row["worked_proxy"]
                ][: args.example_objects],
            },
        }


def main(argv=None):
    parser = argparse.ArgumentParser(
        description="Bound project-progress reward/start residual candidates"
    )
    parser.add_argument("--pid", type=int, default=None, help="live hoi4.exe PID")
    parser.add_argument("--force", action="store_true")
    parser.add_argument("--max-projects", type=int, default=30000)
    parser.add_argument("--max-vtable-hits", type=int, default=10000)
    parser.add_argument("--state-machine-window", type=lambda x: int(x, 0), default=0x280)
    parser.add_argument("--example-objects", type=int, default=12)
    parser.add_argument("--save-projects-json", default=None)
    parser.add_argument("--out", default=None)
    args = parser.parse_args(argv)
    if args.pid is None:
        args.pid = winmem.find_pid("hoi4.exe")
    if not args.pid:
        raise SystemExit("hoi4.exe PID not found")
    if args.max_projects <= 0:
        raise SystemExit("--max-projects must be positive")
    started = time.perf_counter()
    summary = probe(args)
    summary["elapsed_ms"] = round((time.perf_counter() - started) * 1000, 3)
    if args.out:
        os.makedirs(os.path.dirname(os.path.abspath(args.out)), exist_ok=True)
        with open(args.out, "w", encoding="utf-8") as fh:
            json.dump(summary, fh, indent=2, sort_keys=True)
            fh.write("\n")
    print(json.dumps(summary, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
