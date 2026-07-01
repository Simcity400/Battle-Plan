#!/usr/bin/env python3
"""Probe CStateGarrisonData scalar semantics against save garrison blocks.

This is lane-local proof code. It deliberately does not feed production rows:
it only classifies or rejects candidate scalar offsets when a live row can be
matched to a save-oracle row by controller tag, occupied tag, state id, and law.
"""

from __future__ import annotations

import argparse
import json
import math
import struct
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO_ROOT = HERE.parents[3]
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(REPO_ROOT / "extractor" / "memory_reader"))
sys.path.insert(0, str(REPO_ROOT / "extractor" / "memory_reader" / "live_layers"))
sys.path.insert(0, str(REPO_ROOT / "extractor" / "save_oracle"))

import deep_common as dc  # noqa: E402
import live_probe  # noqa: E402
import remaining_fields_probe as rem  # noqa: E402
import tag_table  # noqa: E402
import winmem  # noqa: E402
from deep_common import LBRACE, atom_value, iter_block_entries, to_float, to_int  # noqa: E402

STATE_GARRISON_READ_SIZE = 0x180
DIRECT_CHILD_READ_SIZE = 0x300
DIRECT_CHILD_CAP_PER_ROW = 32
FLOAT_TOLERANCE = 0.0007
MIN_VALIDATE_ROWS = 20
MIN_VALIDATE_RATIO = 0.95
MIN_REJECT_ROWS = 12
MIN_NONDEFAULT_REJECT_ROWS = 2
VALUE_COUNT_CAP = 32

FIELD_DEFAULTS = {
    "strength_ratio": 1.0,
    "garrison_required": 0.0,
    "army_manpower_need": 0.0,
    "manpower_pool_value": 0.0,
    "reinforcement_request_count": 0,
}

PINNED_SCALAR_CANDIDATES = {
    "strength_ratio": (
        ("i32", 0x10),
        ("u32", 0x10),
        ("i32", 0x78),
        ("u32", 0x78),
        ("i32", 0x7C),
        ("u32", 0x7C),
        ("fixed100000_i32", 0xF0),
        ("fixed100000_i64", 0xF0),
    ),
    "garrison_required": (
        ("fixed100000_i32", 0xF8),
        ("fixed100000_i64", 0xF8),
    ),
    "army_manpower_need": (
        ("i32", 0x88),
        ("u32", 0x88),
    ),
}


def hx(value):
    return "0x%X" % value if isinstance(value, int) else None


def parse_manpower_pool(tz):
    values = []
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "value":
                tag = None
                value = None
                for subkey, subopener in iter_block_entries(tz):
                    if subopener is LBRACE:
                        dc._skip_block_body(tz)
                        continue
                    raw = atom_value(subopener)
                    if raw is None:
                        continue
                    if subkey == "tag":
                        tag = raw.strip('"')
                    elif subkey == "value":
                        value = to_float(raw)
                values.append({"tag": tag, "value": value})
            else:
                dc._skip_block_body(tz)
    return values


def parse_equipment_need(tz):
    needs = {}
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            dc._skip_block_body(tz)
            continue
        raw = atom_value(opener)
        value = to_float(raw)
        if value is not None:
            needs[key] = value
    return needs


def parse_reinforcement_requests(tz):
    out = []
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key != "request":
                dc._skip_block_body(tz)
                continue
            rec = {"status": None, "progress": None, "date": None}
            for subkey, subopener in iter_block_entries(tz):
                if subopener is LBRACE:
                    dc._skip_block_body(tz)
                    continue
                raw = atom_value(subopener)
                if raw is None:
                    continue
                if subkey == "status":
                    rec["status"] = to_int(raw)
                elif subkey == "progress":
                    rec["progress"] = to_float(raw)
                elif subkey == "date":
                    rec["date"] = raw
            out.append(rec)
    return out


def parse_state_garrison_entry(tz):
    rec = {
        "strength_ratio": None,
        "garrison_required": None,
        "army_manpower_need": None,
        "manpower_pool": [],
        "equipment_need": {},
        "reinforcement_requests": [],
    }
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "manpower_pool":
                rec["manpower_pool"] = parse_manpower_pool(tz)
            elif key == "equipment_need":
                rec["equipment_need"] = parse_equipment_need(tz)
            elif key == "garrison_reinforcement_requests":
                rec["reinforcement_requests"] = parse_reinforcement_requests(tz)
            else:
                dc._skip_block_body(tz)
            continue
        raw = atom_value(opener)
        if raw is None:
            continue
        if key == "strength_ratio":
            rec["strength_ratio"] = to_float(raw)
        elif key == "garrison_required":
            rec["garrison_required"] = to_float(raw)
        elif key == "army_manpower_need":
            rec["army_manpower_need"] = to_int(raw)
    return rec


def parse_occupation_law_list(tz):
    laws = {}
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            dc._skip_block_body(tz)
            continue
        raw = atom_value(opener)
        state_id = to_int(key)
        if state_id is not None and raw is not None:
            laws[state_id] = raw.strip('"')
    return laws


def parse_occupied_country(tz, controller_tag, occupied_tag, out):
    laws_by_state = {}
    pending_garrison = []
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "occupation_law_list":
                laws_by_state = parse_occupation_law_list(tz)
            elif key == "state_garrison_data":
                for state_key, state_opener in iter_block_entries(tz):
                    if state_opener is not LBRACE:
                        continue
                    state_id = to_int(state_key)
                    rec = parse_state_garrison_entry(tz)
                    if state_id is not None:
                        pending_garrison.append((state_id, rec))
            else:
                dc._skip_block_body(tz)
    for state_id, rec in pending_garrison:
        rec = dict(rec)
        rec["controller_tag"] = controller_tag
        rec["occupied_tag"] = occupied_tag
        rec["state_id"] = state_id
        rec["law_name"] = laws_by_state.get(state_id)
        out[(controller_tag, occupied_tag, state_id)] = rec


def parse_occupation_status(tz, controller_tag, out):
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "occupation":
                for occupied_tag, occupied_opener in iter_block_entries(tz):
                    if occupied_opener is LBRACE:
                        parse_occupied_country(tz, controller_tag, occupied_tag, out)
            else:
                dc._skip_block_body(tz)


def parse_save_garrison_rows(text):
    out = {}

    def parse_countries(tz):
        for tag, opener in iter_block_entries(tz):
            if opener is not LBRACE:
                continue
            for key, op in iter_block_entries(tz):
                if op is LBRACE:
                    if key == "occupation_status":
                        parse_occupation_status(tz, tag, out)
                    else:
                        dc._skip_block_body(tz)

    meta = dc.top_level_pass(
        text,
        {"countries": parse_countries},
        scalar_keys=("date", "version", "player"),
    )
    return out, meta


def direct_child_roots(p, blob, target_vtables):
    roots = []
    for offset, child in rem.iter_heap_ptrs(blob):
        child_blob = p.try_read(child, DIRECT_CHILD_READ_SIZE)
        if not child_blob or len(child_blob) < 0x20:
            continue
        vtable = struct.unpack_from("<Q", child_blob, 0)[0]
        roots.append(
            {
                "parent_offset": offset,
                "ptr": child,
                "vtable": vtable,
                "kind": target_vtables.get(vtable, "unclassified"),
                "blob": child_blob,
            }
        )
        if len(roots) >= DIRECT_CHILD_CAP_PER_ROW:
            break
    return roots


def live_garrison_rows(p, rows, status_by_tag, tbl):
    target_vtables = {
        p.module_base + rem.COUNTRY_OCCUPATION_DATA_VTABLE_RVA: "country_occupation_data",
        p.module_base + rem.STATE_GARRISON_DATA_VTABLE_RVA: "state_garrison_data",
        p.module_base + live_probe.OCCUPATION_LAW_VTABLE_RVA: "occupation_law",
        p.module_base + live_probe.STATE_VTABLE_RVA: "state",
    }
    child_target_vtables = rem.target_vtables_for_graph(p)
    state_ptr_to_id = {
        row["state_ptr"]: row["state_id"]
        for row in rows
        if isinstance(row.get("state_ptr"), int)
    }
    out = {}
    for controller_id, country in status_by_tag.items():
        controller_tag = country.get("country_tag")
        status_ptr = country.get("occupation_status_ptr")
        if not controller_tag or not status_ptr:
            continue
        status_container = rem.read_u64(
            p, status_ptr + rem.COUNTRY_OCCUPATION_STATUS_OCCUPATION_DATA_CONTAINER_OFF
        )
        occupation_data_by_offset = rem.collect_container_ptrs_by_kind(
            p, status_container, target_vtables
        ).get("country_occupation_data", {})
        for occupation_data_ptr in sorted(set(occupation_data_by_offset.values())):
            if (
                rem.read_u64(
                    p,
                    occupation_data_ptr + rem.COUNTRY_OCCUPATION_DATA_PARENT_STATUS_OFF,
                )
                != status_ptr
            ):
                continue
            occupied_tag_id = rem.read_u32(
                p, occupation_data_ptr + rem.COUNTRY_OCCUPATION_DATA_OCCUPIED_TAG_OFF
            )
            if not isinstance(occupied_tag_id, int) or occupied_tag_id >= tbl.count:
                continue
            occupied_tag = tbl.tag(occupied_tag_id)
            law_container = rem.read_u64(
                p, occupation_data_ptr + rem.COUNTRY_OCCUPATION_DATA_LAW_LIST_CONTAINER_OFF
            )
            garrison_container = rem.read_u64(
                p, occupation_data_ptr + rem.COUNTRY_OCCUPATION_DATA_GARRISON_LIST_CONTAINER_OFF
            )
            law_by_offset = rem.collect_container_ptrs_by_kind(
                p, law_container, target_vtables
            ).get("occupation_law", {})
            garrison_by_offset = rem.collect_container_ptrs_by_kind(
                p, garrison_container, target_vtables
            ).get("state_garrison_data", {})
            for entry_offset, garrison_ptr in garrison_by_offset.items():
                state_ptr = rem.read_u64(p, garrison_ptr + rem.STATE_GARRISON_DATA_STATE_OFF)
                occupation_parent = rem.read_u64(
                    p, garrison_ptr + rem.STATE_GARRISON_DATA_OCCUPATION_DATA_OFF
                )
                state_id = state_ptr_to_id.get(state_ptr)
                if state_id is None or occupation_parent != occupation_data_ptr:
                    continue
                law_ptr = law_by_offset.get(entry_offset)
                law_name = None
                if law_ptr:
                    law_name = live_probe.read_pdx_string(
                        p, law_ptr + live_probe.OCCUPATION_LAW_NAME_OFF
                    )
                blob = p.try_read(garrison_ptr, STATE_GARRISON_READ_SIZE)
                if not blob or len(blob) < STATE_GARRISON_READ_SIZE:
                    continue
                out[(controller_tag, occupied_tag, state_id)] = {
                    "controller_tag": controller_tag,
                    "controller_tag_db_id": controller_id,
                    "occupied_tag": occupied_tag,
                    "occupied_tag_db_id": occupied_tag_id,
                    "state_id": state_id,
                    "law_name": law_name,
                    "entry_offset": entry_offset,
                    "garrison_ptr": garrison_ptr,
                    "blob": blob,
                    "direct_child_roots": direct_child_roots(p, blob, child_target_vtables),
                }
    return out


def iter_candidate_values(blob):
    limit = len(blob)
    for offset in range(0, limit - 4 + 1, 4):
        chunk4 = blob[offset : offset + 4]
        i32 = struct.unpack("<i", chunk4)[0]
        u32 = struct.unpack("<I", chunk4)[0]
        f32 = struct.unpack("<f", chunk4)[0]
        yield "i32", offset, i32
        yield "u32", offset, u32
        yield "fixed1000_i32", offset, i32 / 1000.0
        yield "fixed10000_i32", offset, i32 / 10000.0
        yield "fixed100000_i32", offset, i32 / 100000.0
        if math.isfinite(f32) and abs(f32) < 1e9:
            yield "f32", offset, f32
    for offset in range(0, limit - 8 + 1, 8):
        chunk8 = blob[offset : offset + 8]
        i64 = struct.unpack("<q", chunk8)[0]
        u64 = struct.unpack("<Q", chunk8)[0]
        f64 = struct.unpack("<d", chunk8)[0]
        yield "i64", offset, i64
        yield "u64", offset, u64
        yield "fixed1000_i64", offset, i64 / 1000.0
        yield "fixed10000_i64", offset, i64 / 10000.0
        yield "fixed100000_i64", offset, i64 / 100000.0
        if math.isfinite(f64) and abs(f64) < 1e12:
            yield "f64", offset, f64


def matches_value(field, expected, actual):
    if expected is None:
        return False
    if field in ("strength_ratio", "garrison_required"):
        return isinstance(actual, (int, float)) and abs(float(actual) - float(expected)) <= FLOAT_TOLERANCE
    if field == "army_manpower_need":
        return isinstance(actual, (int, float)) and abs(float(actual) - float(expected)) <= 0.01
    if field in ("manpower_pool_value",) or field.startswith("equipment_need:"):
        return isinstance(actual, (int, float)) and abs(float(actual) - float(expected)) <= 0.05
    if field == "reinforcement_request_count":
        return isinstance(actual, (int, float)) and abs(float(actual) - float(expected)) <= 0.01
    return False


def rounded_key(value):
    if isinstance(value, float):
        return f"{value:.6f}"
    return str(value)


def add_count_capped(counts, value, cap=VALUE_COUNT_CAP):
    key = rounded_key(value)
    if key in counts:
        counts[key] += 1
    elif len(counts) < cap:
        counts[key] = 1
    else:
        counts["<more>"] = counts.get("<more>", 0) + 1


def is_nondefault(field, value):
    if value is None:
        return False
    default = 0.0 if field.startswith("equipment_need:") else FIELD_DEFAULTS.get(field)
    if default is None:
        return True
    tolerance = FLOAT_TOLERANCE if isinstance(default, float) else 0.01
    return abs(float(value) - float(default)) > tolerance


def candidate_rejection(field, item):
    if item["compared"] < MIN_REJECT_ROWS:
        return None
    if (
        item["nondefault_expected_rows"] >= MIN_NONDEFAULT_REJECT_ROWS
        and item["nondefault_expected_misses"] == item["nondefault_expected_rows"]
    ):
        return "rejected_misses_all_nondefault_fixture_rows"
    if item["match_ratio"] < MIN_VALIDATE_RATIO:
        return "rejected_low_match_ratio_current_fixture"
    return None


def classify_candidates(matches):
    fields = ("strength_ratio", "garrison_required", "army_manpower_need")
    stats = {field: {} for field in fields}
    for live, oracle in matches:
        for encoding, offset, actual in iter_candidate_values(live["blob"]):
            key = (encoding, offset)
            for field in fields:
                expected = oracle.get(field)
                if expected is None:
                    continue
                rec = stats[field].setdefault(
                    key,
                    {
                        "encoding": encoding,
                        "offset": offset,
                        "compared": 0,
                        "matches": 0,
                        "samples": [],
                        "miss_samples": [],
                        "actual_value_counts": {},
                        "expected_value_counts": {},
                        "nondefault_expected_rows": 0,
                        "nondefault_expected_misses": 0,
                    },
                )
                rec["compared"] += 1
                add_count_capped(rec["actual_value_counts"], actual)
                add_count_capped(rec["expected_value_counts"], expected)
                ok = matches_value(field, expected, actual)
                nondefault = is_nondefault(field, expected)
                if nondefault:
                    rec["nondefault_expected_rows"] += 1
                    if not ok:
                        rec["nondefault_expected_misses"] += 1
                        if len(rec["miss_samples"]) < 8:
                            rec["miss_samples"].append(
                                {
                                    "state_id": live["state_id"],
                                    "controller_tag": live["controller_tag"],
                                    "occupied_tag": live["occupied_tag"],
                                    "law_name": live["law_name"],
                                    "expected": expected,
                                    "actual": actual,
                                    "garrison_ptr": hx(live["garrison_ptr"]),
                                }
                            )
                if ok:
                    rec["matches"] += 1
                    if len(rec["samples"]) < 8:
                        rec["samples"].append(
                            {
                                "state_id": live["state_id"],
                                "controller_tag": live["controller_tag"],
                                "occupied_tag": live["occupied_tag"],
                                "law_name": live["law_name"],
                                "expected": expected,
                                "actual": actual,
                                "garrison_ptr": hx(live["garrison_ptr"]),
                            }
                        )
    out = {}
    for field, by_key in stats.items():
        ranked = sorted(
            by_key.values(),
            key=lambda rec: (rec["matches"], rec["matches"] / max(1, rec["compared"])),
            reverse=True,
        )
        compact = []
        selected = list(ranked[:12])
        selected_keys = {(rec["encoding"], rec["offset"]) for rec in selected}
        for pinned in PINNED_SCALAR_CANDIDATES.get(field, ()):
            rec = by_key.get(pinned)
            if rec and pinned not in selected_keys:
                selected.append(rec)
                selected_keys.add(pinned)
        for rec in selected:
            ratio = rec["matches"] / max(1, rec["compared"])
            item = {
                "encoding": rec["encoding"],
                "offset": hx(rec["offset"]),
                "compared": rec["compared"],
                "matches": rec["matches"],
                "match_ratio": ratio,
                "actual_value_counts": rec["actual_value_counts"],
                "expected_value_counts": rec["expected_value_counts"],
                "nondefault_expected_rows": rec["nondefault_expected_rows"],
                "nondefault_expected_misses": rec["nondefault_expected_misses"],
                "samples": rec["samples"],
                "miss_samples": rec["miss_samples"],
            }
            rejection = candidate_rejection(field, item)
            if rejection:
                item["candidate_verdict"] = rejection
            compact.append(item)
        best = compact[0] if compact else None
        threshold_candidates = [
            item
            for item in compact
            if item["compared"] >= MIN_VALIDATE_ROWS
            and item["match_ratio"] >= MIN_VALIDATE_RATIO
        ]
        threshold_offsets = sorted({item["offset"] for item in threshold_candidates})
        rejected_top_candidates = [
            {
                "encoding": item["encoding"],
                "offset": item["offset"],
                "matches": item["matches"],
                "compared": item["compared"],
                "match_ratio": item["match_ratio"],
                "candidate_verdict": item["candidate_verdict"],
                "nondefault_expected_rows": item["nondefault_expected_rows"],
                "nondefault_expected_misses": item["nondefault_expected_misses"],
            }
            for item in compact
            if item.get("candidate_verdict")
        ]
        verdict = "fixture_insufficient"
        if len(threshold_offsets) == 1:
            verdict = "validated_candidate"
        elif len(threshold_offsets) > 1:
            verdict = "ambiguous_candidate_fixture_required"
        if compact and rejected_top_candidates and not any(
            item.get("candidate_verdict") is None
            and item["compared"] >= MIN_VALIDATE_ROWS
            and item["match_ratio"] >= MIN_VALIDATE_RATIO
            for item in compact
        ):
            verdict = "inline_candidates_rejected_current_fixture"
        elif best and best["matches"] == 0:
            verdict = "no_matching_candidate"
        out[field] = {
            "verdict": verdict,
            "top_candidates": compact,
            "threshold_offsets": threshold_offsets,
            "threshold_candidate_count": len(threshold_candidates),
            "rejected_top_candidates": rejected_top_candidates,
        }
    return out


def classify_child_candidates(matches):
    field_names = ("strength_ratio", "garrison_required", "army_manpower_need")
    child_offset_rows = {}
    known_vtable_counts = {}
    for live, _oracle in matches:
        seen_offsets = set()
        for child in live.get("direct_child_roots") or []:
            parent_offset = child["parent_offset"]
            if parent_offset not in seen_offsets:
                rec = child_offset_rows.setdefault(
                    parent_offset,
                    {
                        "parent_offset": hx(parent_offset),
                        "rows": 0,
                        "kind_counts": {},
                        "vtable_counts": {},
                        "samples": [],
                    },
                )
                rec["rows"] += 1
                seen_offsets.add(parent_offset)
            rec = child_offset_rows[parent_offset]
            kind = child.get("kind") or "unclassified"
            rec["kind_counts"][kind] = rec["kind_counts"].get(kind, 0) + 1
            if kind != "unclassified":
                known_vtable_counts[kind] = known_vtable_counts.get(kind, 0) + 1
            if len(rec["vtable_counts"]) < VALUE_COUNT_CAP:
                vtable_key = hx(child.get("vtable"))
                rec["vtable_counts"][vtable_key] = rec["vtable_counts"].get(vtable_key, 0) + 1
            else:
                rec["vtable_counts"]["<more>"] = rec["vtable_counts"].get("<more>", 0) + 1
            if len(rec["samples"]) < 6:
                rec["samples"].append(
                    {
                        "state_id": live["state_id"],
                        "controller_tag": live["controller_tag"],
                        "occupied_tag": live["occupied_tag"],
                        "ptr": hx(child.get("ptr")),
                        "kind": kind,
                        "vtable": hx(child.get("vtable")),
                    }
                )

    eligible_parent_offsets = {
        offset for offset, rec in child_offset_rows.items() if rec["rows"] >= MIN_REJECT_ROWS
    }
    stats = {field: {} for field in field_names}
    for live, oracle in matches:
        for child in live.get("direct_child_roots") or []:
            parent_offset = child["parent_offset"]
            if parent_offset not in eligible_parent_offsets:
                continue
            kind = child.get("kind") or "unclassified"
            for encoding, child_offset, actual in iter_candidate_values(child["blob"]):
                key = (parent_offset, kind, encoding, child_offset)
                for field in field_names:
                    expected = oracle.get(field)
                    if expected is None:
                        continue
                    rec = stats[field].setdefault(
                        key,
                        {
                            "parent_offset": parent_offset,
                            "kind": kind,
                            "encoding": encoding,
                            "child_offset": child_offset,
                            "compared": 0,
                            "matches": 0,
                            "samples": [],
                            "miss_samples": [],
                            "actual_value_counts": {},
                            "expected_value_counts": {},
                            "nondefault_expected_rows": 0,
                            "nondefault_expected_misses": 0,
                        },
                    )
                    rec["compared"] += 1
                    add_count_capped(rec["actual_value_counts"], actual)
                    add_count_capped(rec["expected_value_counts"], expected)
                    ok = matches_value(field, expected, actual)
                    nondefault = is_nondefault(field, expected)
                    if nondefault:
                        rec["nondefault_expected_rows"] += 1
                        if not ok:
                            rec["nondefault_expected_misses"] += 1
                            if len(rec["miss_samples"]) < 8:
                                rec["miss_samples"].append(
                                    {
                                        "state_id": live["state_id"],
                                        "controller_tag": live["controller_tag"],
                                        "occupied_tag": live["occupied_tag"],
                                        "law_name": live["law_name"],
                                        "expected": expected,
                                        "actual": actual,
                                        "garrison_ptr": hx(live["garrison_ptr"]),
                                        "child_ptr": hx(child.get("ptr")),
                                    }
                                )
                    if ok:
                        rec["matches"] += 1
                        if len(rec["samples"]) < 8:
                            rec["samples"].append(
                                {
                                    "state_id": live["state_id"],
                                    "controller_tag": live["controller_tag"],
                                    "occupied_tag": live["occupied_tag"],
                                    "law_name": live["law_name"],
                                    "expected": expected,
                                    "actual": actual,
                                    "garrison_ptr": hx(live["garrison_ptr"]),
                                    "child_ptr": hx(child.get("ptr")),
                                }
                            )

    field_results = {}
    for field, by_key in stats.items():
        ranked = sorted(
            by_key.values(),
            key=lambda rec: (rec["matches"], rec["matches"] / max(1, rec["compared"])),
            reverse=True,
        )
        compact = []
        for rec in ranked[:12]:
            ratio = rec["matches"] / max(1, rec["compared"])
            item = {
                "parent_offset": hx(rec["parent_offset"]),
                "kind": rec["kind"],
                "encoding": rec["encoding"],
                "child_offset": hx(rec["child_offset"]),
                "compared": rec["compared"],
                "matches": rec["matches"],
                "match_ratio": ratio,
                "actual_value_counts": rec["actual_value_counts"],
                "expected_value_counts": rec["expected_value_counts"],
                "nondefault_expected_rows": rec["nondefault_expected_rows"],
                "nondefault_expected_misses": rec["nondefault_expected_misses"],
                "samples": rec["samples"],
                "miss_samples": rec["miss_samples"],
            }
            rejection = candidate_rejection(field, item)
            if rejection:
                item["candidate_verdict"] = rejection
            compact.append(item)
        threshold_candidates = [
            item
            for item in compact
            if item["compared"] >= MIN_VALIDATE_ROWS
            and item["match_ratio"] >= MIN_VALIDATE_RATIO
        ]
        threshold_scopes = sorted(
            {
                "%s/%s/%s@%s"
                % (
                    item["parent_offset"],
                    item["kind"],
                    item["encoding"],
                    item["child_offset"],
                )
                for item in threshold_candidates
            }
        )
        rejected_top_candidates = [
            {
                "parent_offset": item["parent_offset"],
                "kind": item["kind"],
                "encoding": item["encoding"],
                "child_offset": item["child_offset"],
                "matches": item["matches"],
                "compared": item["compared"],
                "match_ratio": item["match_ratio"],
                "candidate_verdict": item["candidate_verdict"],
                "nondefault_expected_rows": item["nondefault_expected_rows"],
                "nondefault_expected_misses": item["nondefault_expected_misses"],
            }
            for item in compact
            if item.get("candidate_verdict")
        ]
        verdict = "fixture_insufficient"
        if len(threshold_scopes) == 1:
            verdict = "validated_child_candidate"
        elif len(threshold_scopes) > 1:
            verdict = "ambiguous_child_candidate_fixture_required"
        elif compact and rejected_top_candidates and not threshold_scopes:
            verdict = "child_candidates_rejected_current_fixture"
        elif not compact:
            verdict = "no_child_candidate_values"
        field_results[field] = {
            "verdict": verdict,
            "top_candidates": compact,
            "threshold_scopes": threshold_scopes,
            "threshold_candidate_count": len(threshold_candidates),
            "rejected_top_candidates": rejected_top_candidates,
        }

    if not child_offset_rows:
        verdict = "no_direct_child_roots"
    elif any(rec["verdict"] == "validated_child_candidate" for rec in field_results.values()):
        verdict = "validated_child_candidates"
    elif any(
        rec["verdict"] == "ambiguous_child_candidate_fixture_required"
        for rec in field_results.values()
    ):
        verdict = "ambiguous_child_candidates_fixture_required"
    elif all(
        rec["verdict"] in ("child_candidates_rejected_current_fixture", "no_child_candidate_values")
        for rec in field_results.values()
    ):
        verdict = "child_candidates_rejected_current_fixture"
    else:
        verdict = "fixture_insufficient"

    return {
        "verdict": verdict,
        "matched_rows_with_child_roots": sum(
            1 for live, _oracle in matches if live.get("direct_child_roots")
        ),
        "direct_child_root_count": sum(
            len(live.get("direct_child_roots") or []) for live, _oracle in matches
        ),
        "eligible_parent_offsets": [hx(offset) for offset in sorted(eligible_parent_offsets)],
        "known_vtable_counts": known_vtable_counts,
        "parent_offsets": [
            child_offset_rows[offset] for offset in sorted(child_offset_rows)
        ],
        "candidate_fields": field_results,
        "contract": (
            "Direct-child candidates are scanned only from heap pointers stored "
            f"inside CStateGarrisonData+0x0..0x{STATE_GARRISON_READ_SIZE - 1:X}. "
            f"Each readable child root is sampled for 0x{DIRECT_CHILD_READ_SIZE:X} "
            "bytes and compared against the same same-key/same-law save rows. "
            "A child candidate is wireable only with one threshold scope for a "
            "field; rejected scopes are scoped to this direct-child scan and "
            "primitive encodings."
        ),
    }


def save_detail_summary(save_rows):
    equipment_type_counts = {}
    reinforcement_status_counts = {}
    rows_with_progress = 0
    manpower_single_tag_rows = 0
    manpower_multi_tag_rows = 0
    manpower_tag_matches_controller_rows = 0
    for row in save_rows.values():
        manpower_tags = {
            item.get("tag")
            for item in row.get("manpower_pool") or []
            if item.get("tag")
        }
        if len(manpower_tags) == 1:
            manpower_single_tag_rows += 1
            if next(iter(manpower_tags)) == row.get("controller_tag"):
                manpower_tag_matches_controller_rows += 1
        elif len(manpower_tags) > 1:
            manpower_multi_tag_rows += 1
        for equipment_type in (row.get("equipment_need") or {}).keys():
            equipment_type_counts[equipment_type] = equipment_type_counts.get(equipment_type, 0) + 1
        for request in row.get("reinforcement_requests") or []:
            status = request.get("status")
            key = "none" if status is None else str(status)
            reinforcement_status_counts[key] = reinforcement_status_counts.get(key, 0) + 1
            if request.get("progress") is not None:
                rows_with_progress += 1
    return {
        "manpower_pool_rows": sum(1 for row in save_rows.values() if row.get("manpower_pool")),
        "manpower_single_tag_rows": manpower_single_tag_rows,
        "manpower_multi_tag_rows": manpower_multi_tag_rows,
        "manpower_tag_matches_controller_rows": manpower_tag_matches_controller_rows,
        "equipment_need_rows": sum(1 for row in save_rows.values() if row.get("equipment_need")),
        "reinforcement_request_rows": sum(
            1 for row in save_rows.values() if row.get("reinforcement_requests")
        ),
        "equipment_type_counts": equipment_type_counts,
        "reinforcement_status_counts": reinforcement_status_counts,
        "reinforcement_requests_with_progress": rows_with_progress,
    }


def expected_detail_values(oracle, tag_to_id):
    expected = {}
    manpower_pool = oracle.get("manpower_pool") or []
    manpower_values = [
        float(item["value"])
        for item in manpower_pool
        if isinstance(item.get("value"), (int, float))
    ]
    if manpower_values:
        expected["manpower_pool_value"] = sum(manpower_values)
    for equipment_type, value in (oracle.get("equipment_need") or {}).items():
        if isinstance(value, (int, float)):
            expected[f"equipment_need:{equipment_type}"] = float(value)
    if isinstance(oracle.get("reinforcement_requests"), list):
        expected["reinforcement_request_count"] = len(oracle.get("reinforcement_requests") or [])
    return expected


def eligible_direct_child_offsets(matches):
    counts = {}
    for live, _oracle in matches:
        seen = set()
        for child in live.get("direct_child_roots") or []:
            offset = child["parent_offset"]
            if offset in seen:
                continue
            counts[offset] = counts.get(offset, 0) + 1
            seen.add(offset)
    return {offset for offset, count in counts.items() if count >= MIN_REJECT_ROWS}


def iter_detail_scopes(live, eligible_child_offsets):
    yield "inline", None, "state_garrison_data_inline", live["blob"], live["garrison_ptr"]
    for child in live.get("direct_child_roots") or []:
        parent_offset = child["parent_offset"]
        if parent_offset not in eligible_child_offsets:
            continue
        yield "direct_child", parent_offset, child.get("kind") or "unclassified", child["blob"], child.get("ptr")


def classify_detail_candidates(matches, save_rows, tag_to_id):
    eligible_child_offsets = eligible_direct_child_offsets(matches)
    stats = {}
    compared_rows_by_field = {}
    for live, oracle in matches:
        expected_values = expected_detail_values(oracle, tag_to_id)
        for field in expected_values:
            compared_rows_by_field[field] = compared_rows_by_field.get(field, 0) + 1
        if not expected_values:
            continue
        for scope_kind, parent_offset, root_kind, blob, root_ptr in iter_detail_scopes(
            live, eligible_child_offsets
        ):
            for encoding, value_offset, actual in iter_candidate_values(blob):
                key_scope = (
                    scope_kind,
                    parent_offset if parent_offset is not None else -1,
                    root_kind,
                    encoding,
                    value_offset,
                )
                for field, expected in expected_values.items():
                    rec = stats.setdefault(field, {}).setdefault(
                        key_scope,
                        {
                            "scope_kind": scope_kind,
                            "parent_offset": parent_offset,
                            "root_kind": root_kind,
                            "encoding": encoding,
                            "value_offset": value_offset,
                            "compared": 0,
                            "matches": 0,
                            "samples": [],
                            "miss_samples": [],
                            "actual_value_counts": {},
                            "expected_value_counts": {},
                            "nondefault_expected_rows": 0,
                            "nondefault_expected_misses": 0,
                        },
                    )
                    rec["compared"] += 1
                    add_count_capped(rec["actual_value_counts"], actual)
                    add_count_capped(rec["expected_value_counts"], expected)
                    ok = matches_value(field, expected, actual)
                    nondefault = is_nondefault(field, expected)
                    if nondefault:
                        rec["nondefault_expected_rows"] += 1
                        if not ok:
                            rec["nondefault_expected_misses"] += 1
                            if len(rec["miss_samples"]) < 8:
                                rec["miss_samples"].append(
                                    {
                                        "state_id": live["state_id"],
                                        "controller_tag": live["controller_tag"],
                                        "occupied_tag": live["occupied_tag"],
                                        "law_name": live["law_name"],
                                        "expected": expected,
                                        "actual": actual,
                                        "garrison_ptr": hx(live["garrison_ptr"]),
                                        "root_ptr": hx(root_ptr),
                                    }
                                )
                    if ok:
                        rec["matches"] += 1
                        if len(rec["samples"]) < 8:
                            rec["samples"].append(
                                {
                                    "state_id": live["state_id"],
                                    "controller_tag": live["controller_tag"],
                                    "occupied_tag": live["occupied_tag"],
                                    "law_name": live["law_name"],
                                    "expected": expected,
                                    "actual": actual,
                                    "garrison_ptr": hx(live["garrison_ptr"]),
                                    "root_ptr": hx(root_ptr),
                                }
                            )

    field_results = {}
    for field in sorted(compared_rows_by_field):
        ranked = sorted(
            stats.get(field, {}).values(),
            key=lambda rec: (rec["matches"], rec["matches"] / max(1, rec["compared"])),
            reverse=True,
        )
        compact = []
        for rec in ranked[:12]:
            ratio = rec["matches"] / max(1, rec["compared"])
            item = {
                "scope_kind": rec["scope_kind"],
                "parent_offset": hx(rec["parent_offset"]) if rec["parent_offset"] is not None else None,
                "root_kind": rec["root_kind"],
                "encoding": rec["encoding"],
                "value_offset": hx(rec["value_offset"]),
                "compared": rec["compared"],
                "matches": rec["matches"],
                "match_ratio": ratio,
                "actual_value_counts": rec["actual_value_counts"],
                "expected_value_counts": rec["expected_value_counts"],
                "nondefault_expected_rows": rec["nondefault_expected_rows"],
                "nondefault_expected_misses": rec["nondefault_expected_misses"],
                "samples": rec["samples"],
                "miss_samples": rec["miss_samples"],
            }
            rejection = candidate_rejection(field, item)
            if rejection:
                item["candidate_verdict"] = rejection
            compact.append(item)
        threshold_candidates = [
            item
            for item in compact
            if item["compared"] >= MIN_VALIDATE_ROWS
            and item["match_ratio"] >= MIN_VALIDATE_RATIO
        ]
        threshold_scopes = sorted(
            {
                "%s/%s/%s/%s@%s"
                % (
                    item["scope_kind"],
                    item["parent_offset"] or "inline",
                    item["root_kind"],
                    item["encoding"],
                    item["value_offset"],
                )
                for item in threshold_candidates
            }
        )
        rejected_top_candidates = [
            {
                "scope_kind": item["scope_kind"],
                "parent_offset": item["parent_offset"],
                "root_kind": item["root_kind"],
                "encoding": item["encoding"],
                "value_offset": item["value_offset"],
                "matches": item["matches"],
                "compared": item["compared"],
                "match_ratio": item["match_ratio"],
                "candidate_verdict": item["candidate_verdict"],
                "nondefault_expected_rows": item["nondefault_expected_rows"],
                "nondefault_expected_misses": item["nondefault_expected_misses"],
            }
            for item in compact
            if item.get("candidate_verdict")
        ]
        if len(threshold_scopes) == 1:
            verdict = "validated_detail_candidate"
        elif len(threshold_scopes) > 1:
            verdict = "ambiguous_detail_candidate_fixture_required"
        elif compact and rejected_top_candidates and not threshold_scopes:
            verdict = "detail_candidates_rejected_current_fixture"
        elif not compact:
            verdict = "no_detail_candidate_values"
        else:
            verdict = "fixture_insufficient"
        field_results[field] = {
            "verdict": verdict,
            "compared_rows": compared_rows_by_field[field],
            "top_candidates": compact,
            "threshold_scopes": threshold_scopes,
            "threshold_candidate_count": len(threshold_candidates),
            "rejected_top_candidates": rejected_top_candidates,
        }

    if any(rec["verdict"] == "validated_detail_candidate" for rec in field_results.values()):
        verdict = "validated_detail_candidates"
    elif any(
        rec["verdict"] == "ambiguous_detail_candidate_fixture_required"
        for rec in field_results.values()
    ):
        verdict = "ambiguous_detail_candidates_fixture_required"
    elif field_results and all(
        rec["verdict"] in ("detail_candidates_rejected_current_fixture", "no_detail_candidate_values")
        for rec in field_results.values()
    ):
        verdict = "detail_candidates_rejected_current_fixture"
    elif not field_results:
        verdict = "no_detail_fixture_rows"
    else:
        verdict = "fixture_insufficient"
    return {
        "verdict": verdict,
        "save_detail_summary": save_detail_summary(save_rows),
        "eligible_child_offsets": [hx(offset) for offset in sorted(eligible_child_offsets)],
        "candidate_fields": field_results,
        "contract": (
            "Detail candidates cover save garrison manpower_pool values, "
            "equipment_need values by equipment token, and reinforcement request "
            "count. Manpower pool tags are summarized separately; in the current "
            "fixture every manpower pool has one tag and it matches the emitted "
            "controller tag, so it is not an independent garrison vector proof. "
            "Candidate values are scanned across the inline CStateGarrisonData "
            "blob and the same direct/root pointer scopes as scalar candidates. "
            "A detail field is wireable only with one threshold scope; otherwise "
            "the lane must keep manpower/equipment detail unavailable until a "
            "controlled same-hour or one-variable fixture classifies the vector "
            "layout."
        ),
    }


def write_md(out_dir, proof):
    lines = [
        "# Garrison Scalar Probe",
        "",
        "- Live SHA-256: `%s`" % proof["process"]["sha256"],
        "- Save fixture: `%s`, date `%s`" % (
            proof["save_fixture"]["path"],
            proof["save_fixture"]["date"],
        ),
        "- Save garrison rows: `%d`" % proof["save_fixture"]["garrison_rows"],
        "- Live garrison rows: `%d`" % proof["live"]["garrison_rows"],
        "- Matched key rows: `%d`" % proof["compare"]["matched_key_rows"],
        "- Matched same-law rows: `%d`" % proof["compare"]["matched_same_law_rows"],
        "- Verdict: `%s`" % proof["verdict"],
        "",
        "## Candidate Fields",
        "",
    ]
    for field, rec in proof["candidate_fields"].items():
        best = rec["top_candidates"][0] if rec["top_candidates"] else None
        if best:
            rejected = rec.get("rejected_top_candidates") or []
            lines.append(
                "- `%s`: `%s`, best `%s @ %s`, `%d/%d` matches, "
                "threshold offsets `%s`, rejected top candidates `%d`"
                % (
                    field,
                    rec["verdict"],
                    best["encoding"],
                    best["offset"],
                    best["matches"],
                    best["compared"],
                    ", ".join(rec.get("threshold_offsets") or []),
                    len(rejected),
                )
            )
            for item in rec["top_candidates"]:
                if item.get("candidate_verdict") and item["offset"] in {
                    "0x10",
                    "0x78",
                    "0x7C",
                    "0xF0",
                    "0xF8",
                    "0x88",
                }:
                    lines.append(
                        "  - rejected `%s @ %s`: `%s`, `%d/%d` matches, "
                        "non-default misses `%d/%d`, actual values `%s`"
                        % (
                            item["encoding"],
                            item["offset"],
                            item["candidate_verdict"],
                            item["matches"],
                            item["compared"],
                            item["nondefault_expected_misses"],
                            item["nondefault_expected_rows"],
                            item["actual_value_counts"],
                        )
                    )
        else:
            lines.append("- `%s`: `%s`, no candidates" % (field, rec["verdict"]))
    child_probe = proof.get("child_root_probe") or {}
    lines.extend(
        [
            "",
            "## Direct Child Roots",
            "",
            "- Verdict: `%s`" % child_probe.get("verdict"),
            "- Matched rows with child roots: `%s`"
            % child_probe.get("matched_rows_with_child_roots"),
            "- Direct child root count: `%s`" % child_probe.get("direct_child_root_count"),
            "- Eligible parent offsets: `%s`"
            % ", ".join(child_probe.get("eligible_parent_offsets") or []),
            "- Known child vtables: `%s`" % (child_probe.get("known_vtable_counts") or {}),
            "",
        ]
    )
    for field, rec in (child_probe.get("candidate_fields") or {}).items():
        best = rec["top_candidates"][0] if rec["top_candidates"] else None
        if not best:
            lines.append("- `%s`: `%s`, no child candidates" % (field, rec["verdict"]))
            continue
        lines.append(
            "- `%s`: `%s`, best parent `%s` `%s` `%s @ %s`, `%d/%d` matches, "
            "threshold scopes `%s`, rejected top candidates `%d`"
            % (
                field,
                rec["verdict"],
                best["parent_offset"],
                best["kind"],
                best["encoding"],
                best["child_offset"],
                best["matches"],
                best["compared"],
                ", ".join(rec.get("threshold_scopes") or []),
                len(rec.get("rejected_top_candidates") or []),
            )
        )
    detail_probe = proof.get("detail_probe") or {}
    detail_summary = detail_probe.get("save_detail_summary") or {}
    lines.extend(
        [
            "",
            "## Manpower / Equipment Detail",
            "",
            "- Verdict: `%s`" % detail_probe.get("verdict"),
            "- Save rows with manpower pool: `%s`"
            % detail_summary.get("manpower_pool_rows"),
            "- Manpower pool single-tag / multi-tag rows: `%s/%s`"
            % (
                detail_summary.get("manpower_single_tag_rows"),
                detail_summary.get("manpower_multi_tag_rows"),
            ),
            "- Manpower pool tag matches controller rows: `%s`"
            % detail_summary.get("manpower_tag_matches_controller_rows"),
            "- Save rows with equipment need: `%s`"
            % detail_summary.get("equipment_need_rows"),
            "- Save rows with reinforcement requests: `%s`"
            % detail_summary.get("reinforcement_request_rows"),
            "- Equipment types: `%s`" % (detail_summary.get("equipment_type_counts") or {}),
            "- Reinforcement status counts: `%s`"
            % (detail_summary.get("reinforcement_status_counts") or {}),
            "",
        ]
    )
    for field, rec in (detail_probe.get("candidate_fields") or {}).items():
        best = rec["top_candidates"][0] if rec["top_candidates"] else None
        if not best:
            lines.append("- `%s`: `%s`, no detail candidates" % (field, rec["verdict"]))
            continue
        lines.append(
            "- `%s`: `%s`, compared rows `%d`, best `%s` parent `%s` `%s` "
            "`%s @ %s`, `%d/%d` matches, threshold scopes `%s`, rejected top "
            "candidates `%d`"
            % (
                field,
                rec["verdict"],
                rec["compared_rows"],
                best["scope_kind"],
                best["parent_offset"] or "inline",
                best["root_kind"],
                best["encoding"],
                best["value_offset"],
                best["matches"],
                best["compared"],
                ", ".join(rec.get("threshold_scopes") or []),
                len(rec.get("rejected_top_candidates") or []),
            )
        )
    lines.extend(
        [
            "",
            "## Contract",
            "",
            proof["field_contract"],
            "",
            proof.get("rejection_contract", ""),
            "",
            child_probe.get("contract", ""),
            "",
            detail_probe.get("contract", ""),
        ]
    )
    (out_dir / "garrison_scalar_PROOF.md").write_text(
        "\n".join(lines) + "\n", encoding="utf-8"
    )


def run(out_dir, save_melt):
    text = Path(save_melt).read_text(encoding="utf-8-sig", errors="replace")
    save_rows, save_meta = parse_save_garrison_rows(text)
    p = winmem.open_process("hoi4.exe")
    if p is None:
        raise RuntimeError("hoi4.exe not running")
    with p:
        fp = live_probe.checked_fingerprint(p)
        tbl = tag_table.TagTable.locate(p)
        tag_to_id = {tag: tag_id for tag_id, tag in tbl.as_map().items()}
        roots = live_probe.resolve_state_roots(p)
        status_by_tag, status_summary = live_probe.read_country_occupation_status(
            p, roots["game_state"], tbl
        )
        rows, invalid_states = live_probe.read_state_rows(
            p, roots["state_roots"], tbl, status_by_tag
        )
        live_rows = live_garrison_rows(p, rows, status_by_tag, tbl)

    matched = []
    key_only = 0
    law_mismatches = []
    for key, live in live_rows.items():
        oracle = save_rows.get(key)
        if not oracle:
            continue
        key_only += 1
        if live.get("law_name") and oracle.get("law_name") and live["law_name"] != oracle["law_name"]:
            if len(law_mismatches) < 12:
                law_mismatches.append(
                    {
                        "controller_tag": key[0],
                        "occupied_tag": key[1],
                        "state_id": key[2],
                        "live_law": live.get("law_name"),
                        "save_law": oracle.get("law_name"),
                    }
                )
            continue
        matched.append((live, oracle))

    candidate_fields = classify_candidates(matched)
    child_root_probe = classify_child_candidates(matched)
    detail_probe = classify_detail_candidates(matched, save_rows, tag_to_id)
    validated = [
        field
        for field, rec in candidate_fields.items()
        if rec["verdict"] == "validated_candidate"
    ]
    ambiguous = [
        field
        for field, rec in candidate_fields.items()
        if rec["verdict"] == "ambiguous_candidate_fixture_required"
    ]
    if validated:
        verdict = "validated_scalar_candidates"
    elif ambiguous:
        verdict = "ambiguous_scalar_candidates_fixture_required"
    elif child_root_probe["verdict"] == "validated_child_candidates":
        verdict = "validated_child_scalar_candidates"
    elif child_root_probe["verdict"] == "ambiguous_child_candidates_fixture_required":
        verdict = "ambiguous_child_scalar_candidates_fixture_required"
    elif detail_probe["verdict"] == "validated_detail_candidates":
        verdict = "validated_detail_candidates"
    elif detail_probe["verdict"] == "ambiguous_detail_candidates_fixture_required":
        verdict = "ambiguous_detail_candidates_fixture_required"
    elif (
        candidate_fields
        and all(
            rec["verdict"] == "inline_candidates_rejected_current_fixture"
            for rec in candidate_fields.values()
        )
        and child_root_probe["verdict"]
        in ("no_direct_child_roots", "child_candidates_rejected_current_fixture")
        and detail_probe["verdict"]
        in ("no_detail_fixture_rows", "detail_candidates_rejected_current_fixture")
    ):
        verdict = "scanned_garrison_candidates_rejected_current_fixture"
    else:
        verdict = "fixture_insufficient"
    proof = {
        "process": {
            "pid": fp.get("pid"),
            "base": hx(fp.get("base")),
            "sha256": fp.get("sha256"),
        },
        "save_fixture": {
            "path": str(save_melt),
            "date": save_meta.get("date"),
            "version": save_meta.get("version"),
            "player": save_meta.get("player"),
            "garrison_rows": len(save_rows),
        },
        "live": {
            "state_roots": len(roots["state_roots"]),
            "invalid_state_rows": invalid_states,
            "occupation_status": status_summary,
            "garrison_rows": len(live_rows),
        },
        "compare": {
            "matched_key_rows": key_only,
            "matched_same_law_rows": len(matched),
            "law_mismatches_sample": law_mismatches,
        },
        "candidate_fields": candidate_fields,
        "child_root_probe": child_root_probe,
        "detail_probe": detail_probe,
        "verdict": verdict,
        "field_contract": (
            "Only fields with verdict validated_candidate are safe to wire. "
            "ambiguous_candidate_fixture_required means more than one live offset "
            "passes the threshold, so a controlled fixture with a non-default "
            "value is required before wiring. fixture_insufficient means the "
            "current live process and save fixture do not provide enough "
            "same-key/same-law rows to classify the scalar; keep garrison "
            "need/equilibrium/progress row-badged unavailable until a same-hour "
            "save or controlled UI flip is captured."
        ),
        "rejection_contract": (
            "Candidate offsets are rejected for this fixture when they have at "
            f"least {MIN_REJECT_ROWS} compared same-key/same-law rows and either "
            "miss every non-default save row or remain below the validation "
            f"ratio {MIN_VALIDATE_RATIO:.2f}. Rejection is scoped to the scanned "
            f"CStateGarrisonData inline range 0x0..0x{STATE_GARRISON_READ_SIZE - 1:X} "
            "and the listed primitive encodings."
        ),
    }

    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "garrison_scalar_probe.json").write_text(
        json.dumps(proof, indent=2), encoding="utf-8"
    )
    write_md(out_dir, proof)
    return proof


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out-dir", required=True)
    parser.add_argument(
        "--save-melt",
        default=str(Path.home() / "AppData" / "Local" / "Temp" / "ger1946_melt.txt"),
    )
    args = parser.parse_args(argv)
    proof = run(Path(args.out_dir), Path(args.save_melt))
    print(json.dumps({"out_dir": args.out_dir, "proof": proof}, indent=2))


if __name__ == "__main__":
    raise SystemExit(main())
