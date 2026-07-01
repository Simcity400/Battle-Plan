#!/usr/bin/env python3
"""Bounded proof probes for remaining occupation lane fields.

This script is proof-only. It reuses the lane-local live roots from
``live_probe.py`` and writes evidence for fields that are not yet safe to emit
as production compact records.
"""

from __future__ import annotations

import argparse
import json
import re
import struct
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO_ROOT = HERE.parents[3]
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(REPO_ROOT / "extractor" / "memory_reader"))
sys.path.insert(0, str(REPO_ROOT / "extractor" / "memory_reader" / "live_layers"))

import live_probe  # noqa: E402
import tag_table  # noqa: E402
import winmem  # noqa: E402

DIVISION_TEMPLATE_VTABLE_RVA = 0x2938F70
DIVISION_TEMPLATE_TYPE = 52
DIVISION_TEMPLATE_OFF_TYPE = 0x08
DIVISION_TEMPLATE_OFF_ID = 0x0C
STATUS_TEMPLATE_CANDIDATE_OFFSETS = (0x78, 0x80)

COUNTRY_OCCUPATION_STATUS_SIZE = 0x130
COUNTRY_COLLABORATION_STATUS_SIZE = 0x100
GRAPH_NODE_SIZE = 0x300
GRAPH_MAX_DEPTH = 3
GRAPH_MAX_NODES = 2500

COUNTRY_OCCUPATION_DATA_VTABLE_RVA = 0x296DD00
STATE_GARRISON_DATA_VTABLE_RVA = 0x296DCB0
GARRISON_STATUS_VTABLE_RVA = 0x29BB568
GARRISON_STATUS_SECONDARY_VTABLE_RVA = 0x29BB5A0
COUNTRY_COLLABORATION_DATA_VTABLE_RVA = 0x29A5D08

COUNTRY_OCCUPATION_STATUS_OCCUPATION_DATA_CONTAINER_OFF = 0x48
COUNTRY_OCCUPATION_DATA_PARENT_STATUS_OFF = 0x08
COUNTRY_OCCUPATION_DATA_OCCUPIED_TAG_OFF = 0x10
COUNTRY_OCCUPATION_DATA_STATE_LIST_CONTAINER_OFF = 0x20
COUNTRY_OCCUPATION_DATA_LAW_LIST_CONTAINER_OFF = 0xB0
COUNTRY_OCCUPATION_DATA_GARRISON_LIST_CONTAINER_OFF = 0xD8

STATE_GARRISON_DATA_COUNTRY_OFF = 0x48
STATE_GARRISON_DATA_STATE_OFF = 0x50
STATE_GARRISON_DATA_OCCUPATION_DATA_OFF = 0x58

GIE_SAVE_ASSIGNMENT_KEYS = (
    "government_in_exile_tag",
    "government_in_exile",
    "exiled_government",
    "legitimacy",
    "host_nation",
)

GIE_ASSET_CONTEXTS = {"leader", "ship", "deployment", "air_wing"}


def hx(value):
    return "0x%X" % value if isinstance(value, int) else None


def line_no_at(text, offset):
    return text.count("\n", 0, offset) + 1


def classify_government_in_exile_tag_window(window):
    contexts = []
    if all(
        token in window
        for token in (
            "attack_skill=",
            "defense_skill=",
            "planning_skill=",
            "logistics_skill=",
        )
    ):
        contexts.append("leader")
    if "ship_name=" in window and "max_manpower=" in window and "manpower=" in window:
        contexts.append("ship")
    if "military_deployment_line=" in window or re.search(r"(?m)^\s*role\s*=", window):
        contexts.append("deployment")
    if (
        "air_wings=" in window
        or "allow_mission_type=" in window
        or "air_untrained_pilots_penalty_factor=" in window
    ):
        contexts.append("air_wing")
    return "+".join(contexts) if contexts else "unknown"


def is_asset_gie_tag_context(context):
    parts = set(context.split("+")) if context else set()
    return bool(parts) and parts.issubset(GIE_ASSET_CONTEXTS)


def parse_country_gie_window(text, match):
    start = max(0, match.start() - 1600)
    end = min(len(text), match.end() + 900)
    window = text[start:end]
    legitimacy = re.search(r"(?m)^\s*legitimacy\s*=\s*([+-]?[0-9]+(?:\.[0-9]+)?)", window)
    exile_leaders = re.search(r"(?m)^\s*exile_army_leaders\s*=\s*([0-9]+)", window)
    return {
        "host_tag": match.group(1),
        "line": line_no_at(text, match.start()),
        "legitimacy": float(legitimacy.group(1)) if legitimacy else None,
        "exile_army_leaders": int(exile_leaders.group(1)) if exile_leaders else None,
    }


def top_level_country_marks(text):
    return [
        (match.start(), match.group(1))
        for match in re.finditer(r"(?m)^\t([A-Z0-9]{3})=\{$", text)
    ]


def country_tag_for_offset(country_marks, offset):
    current = None
    for mark_offset, tag in country_marks:
        if mark_offset > offset:
            break
        current = tag
    return current


def read_u64(p, address):
    try:
        return p.u64(address)
    except OSError:
        return None


def read_u32(p, address):
    try:
        return p.u32(address)
    except OSError:
        return None


def iter_heap_ptrs(blob):
    for offset in range(0, max(0, len(blob) - 8), 8):
        value = struct.unpack_from("<Q", blob, offset)[0]
        if live_probe.is_heap_ptr(value):
            yield offset, value


def read_speed_snapshot(p, roots):
    rows = {}
    for state_id, state_ptr in roots.items():
        try:
            blob = p.read(state_ptr, live_probe.STATE_OFF_COMPLIANCE_SPEED + 4)
        except OSError:
            continue
        if live_probe.u64_from(blob, 0) != p.module_base + live_probe.STATE_VTABLE_RVA:
            continue
        rows[state_id] = {
            "resistance_raw": live_probe.i32_from(blob, live_probe.STATE_OFF_RESISTANCE),
            "resistance_speed_raw": live_probe.i32_from(
                blob, live_probe.STATE_OFF_RESISTANCE_SPEED
            ),
            "compliance_raw": live_probe.i32_from(blob, live_probe.STATE_OFF_COMPLIANCE),
            "compliance_speed_raw": live_probe.i32_from(
                blob, live_probe.STATE_OFF_COMPLIANCE_SPEED
            ),
        }
    return rows


def validate_speed_signals(p, roots, seconds):
    game_state = p.u64(p.module_base + live_probe.CG_STATE_DATE_OFFSET)
    snapshots = []
    for index in range(3):
        if index:
            time.sleep(seconds)
        snapshots.append(
            {
                "game_hour_raw": p.i32(game_state + live_probe.GS_CURRENT_DATE_OFFSET),
                "rows": read_speed_snapshot(p, roots),
            }
        )

    intervals = []
    resistance_delta_rows = 0
    resistance_speed_matches = 0
    compliance_delta_rows = 0
    compliance_speed_matches = 0
    samples = []
    for left, right in zip(snapshots, snapshots[1:]):
        hour_delta = right["game_hour_raw"] - left["game_hour_raw"]
        interval = {
            "from_hour_raw": left["game_hour_raw"],
            "to_hour_raw": right["game_hour_raw"],
            "hour_delta": hour_delta,
            "resistance_delta_rows": 0,
            "resistance_speed_matches": 0,
            "compliance_delta_rows": 0,
            "compliance_speed_matches": 0,
        }
        for state_id, a in left["rows"].items():
            b = right["rows"].get(state_id)
            if not b:
                continue
            res_delta = b["resistance_raw"] - a["resistance_raw"]
            comp_delta = b["compliance_raw"] - a["compliance_raw"]
            if res_delta:
                resistance_delta_rows += 1
                interval["resistance_delta_rows"] += 1
                if res_delta in (a["resistance_speed_raw"], b["resistance_speed_raw"]):
                    resistance_speed_matches += 1
                    interval["resistance_speed_matches"] += 1
                if len(samples) < 12:
                    samples.append(
                        {
                            "state_id": state_id,
                            "kind": "resistance",
                            "delta": res_delta,
                            "left_speed_raw": a["resistance_speed_raw"],
                            "right_speed_raw": b["resistance_speed_raw"],
                        }
                    )
            if comp_delta:
                compliance_delta_rows += 1
                interval["compliance_delta_rows"] += 1
                if comp_delta in (a["compliance_speed_raw"], b["compliance_speed_raw"]):
                    compliance_speed_matches += 1
                    interval["compliance_speed_matches"] += 1
                if len(samples) < 12:
                    samples.append(
                        {
                            "state_id": state_id,
                            "kind": "compliance",
                            "delta": comp_delta,
                            "left_speed_raw": a["compliance_speed_raw"],
                            "right_speed_raw": b["compliance_speed_raw"],
                        }
                    )
        intervals.append(interval)

    return {
        "seconds_between_samples": seconds,
        "offsets": {
            "STATE_OFF_RESISTANCE_SPEED": hx(live_probe.STATE_OFF_RESISTANCE_SPEED),
            "STATE_OFF_COMPLIANCE_SPEED": hx(live_probe.STATE_OFF_COMPLIANCE_SPEED),
        },
        "intervals": intervals,
        "resistance_delta_rows": resistance_delta_rows,
        "resistance_speed_matches": resistance_speed_matches,
        "compliance_delta_rows": compliance_delta_rows,
        "compliance_speed_matches": compliance_speed_matches,
        "sample_matches": samples,
        "verdict": (
            "validated"
            if resistance_speed_matches and compliance_speed_matches
            else "fixture_insufficient"
        ),
    }


def template_id_at(p, status_ptr, offset):
    try:
        ptr = p.u64(status_ptr + offset)
    except OSError:
        return None
    if not live_probe.is_heap_ptr(ptr):
        return None
    try:
        if p.u64(ptr) != p.module_base + DIVISION_TEMPLATE_VTABLE_RVA:
            return None
        if p.u32(ptr + DIVISION_TEMPLATE_OFF_TYPE) != DIVISION_TEMPLATE_TYPE:
            return None
        return {"template_id": p.u32(ptr + DIVISION_TEMPLATE_OFF_ID), "ptr": hx(ptr)}
    except OSError:
        return None


def probe_template_candidates(p, status_by_tag, oracle_path):
    by_tag = {
        rec["country_tag"]: rec
        for rec in status_by_tag.values()
        if rec.get("country_tag") and rec.get("occupation_status_ptr")
    }
    candidates = {}
    for offset in STATUS_TEMPLATE_CANDIDATE_OFFSETS:
        key = hx(offset)
        valid = {}
        for tag, rec in by_tag.items():
            item = template_id_at(p, rec["occupation_status_ptr"], offset)
            if item:
                valid[tag] = item
        candidates[key] = {
            "valid_template_pointer_rows": len(valid),
            "sample": dict(list(sorted(valid.items()))[:20]),
        }

    compare = None
    path = Path(oracle_path) if oracle_path else None
    if path and path.is_file():
        data = json.loads(path.read_text(encoding="utf-8"))
        expected = {}
        for tag, rec in data.get("countries", {}).items():
            template = rec.get("garrison_template_id")
            if template:
                expected[tag] = template[0]
        compare = {
            "oracle_path": str(path),
            "expected_nonnull_templates": len(expected),
            "offsets": {},
        }
        for offset in STATUS_TEMPLATE_CANDIDATE_OFFSETS:
            exact = 0
            missing = 0
            mismatch = 0
            sample_mismatches = []
            for tag, want in expected.items():
                rec = by_tag.get(tag)
                got = None
                if rec:
                    item = template_id_at(p, rec["occupation_status_ptr"], offset)
                    got = item["template_id"] if item else None
                if got == want:
                    exact += 1
                elif got is None:
                    missing += 1
                else:
                    mismatch += 1
                    if len(sample_mismatches) < 12:
                        sample_mismatches.append(
                            {"tag": tag, "expected": want, "candidate": got}
                        )
            compare["offsets"][hx(offset)] = {
                "exact": exact,
                "missing": missing,
                "mismatch": mismatch,
                "sample_mismatches": sample_mismatches,
            }

    verdict = "candidate_unclassified_fixture_required"
    validated_offset = None
    if compare and compare["expected_nonnull_templates"]:
        validated = []
        rejected = []
        for offset, rec in compare["offsets"].items():
            comparable = rec["exact"] + rec["mismatch"]
            if comparable > 0 and rec["exact"] == comparable:
                validated.append(offset)
            elif rec["mismatch"] > 0:
                rejected.append(offset)
        if len(validated) == 1:
            validated_offset = validated[0]
            verdict = "validated_template_offset"
        elif rejected and not validated:
            verdict = "candidate_rejected_for_current_fixture"

    return {
        "candidate_offsets": [hx(offset) for offset in STATUS_TEMPLATE_CANDIDATE_OFFSETS],
        "candidates": candidates,
        "oracle_compare": compare,
        "verdict": verdict,
        "validated_offset": validated_offset,
        "fixture_contract": (
            "Validated when a fresh save/current live compare has exactly one "
            "candidate offset with every comparable row matching and at least one "
            "other candidate rejected by mismatches. Missing rows are tolerated "
            "only when the live country/status root is absent."
        ),
    }


def collect_reachable_vtable_hits(p, root_ptr, root_size, target_vtables):
    queue = [("root", root_ptr, 0, root_size)]
    seen = set()
    hits = []
    while queue and len(seen) < GRAPH_MAX_NODES:
        path, ptr, depth, size = queue.pop(0)
        if ptr in seen:
            continue
        seen.add(ptr)
        blob = p.try_read(ptr, size if depth == 0 else GRAPH_NODE_SIZE)
        if not blob:
            continue
        if len(blob) >= 8:
            vtable = struct.unpack_from("<Q", blob, 0)[0]
            target_name = target_vtables.get(vtable)
            if target_name:
                hits.append({"ptr": ptr, "path": path, "depth": depth, "kind": target_name})
        if depth >= GRAPH_MAX_DEPTH:
            continue
        for offset, child in iter_heap_ptrs(blob):
            if child in seen:
                continue
            if p.try_read(child, 0x20):
                queue.append((path + " +0x%X" % offset, child, depth + 1, GRAPH_NODE_SIZE))
            if len(queue) + len(seen) >= GRAPH_MAX_NODES:
                break
    return hits, len(seen)


def count_target_ptrs_in_container(p, ptr, target_vtables, state_ptr_to_id):
    if not live_probe.is_heap_ptr(ptr):
        return {"target_counts": {}, "state_ptr_rows": 0, "sample": []}
    blob = p.try_read(ptr, GRAPH_NODE_SIZE)
    if not blob:
        return {"target_counts": {}, "state_ptr_rows": 0, "sample": []}
    target_counts = {}
    state_ptr_rows = 0
    sample = []
    for offset, child in iter_heap_ptrs(blob):
        kind = target_vtables.get(read_u64(p, child))
        state_id = state_ptr_to_id.get(child)
        if kind:
            target_counts[kind] = target_counts.get(kind, 0) + 1
        if state_id is not None:
            state_ptr_rows += 1
        if (kind or state_id is not None) and len(sample) < 8:
            sample.append({"offset": hx(offset), "ptr": hx(child), "kind": kind, "state_id": state_id})
    return {"target_counts": target_counts, "state_ptr_rows": state_ptr_rows, "sample": sample}


def collect_container_ptrs_by_kind(p, ptr, target_vtables):
    if not live_probe.is_heap_ptr(ptr):
        return {}
    blob = p.try_read(ptr, GRAPH_NODE_SIZE)
    if not blob:
        return {}
    by_kind = {}
    for offset, child in iter_heap_ptrs(blob):
        kind = target_vtables.get(read_u64(p, child))
        if kind:
            by_kind.setdefault(kind, {})[offset] = child
    return by_kind


def target_vtables_for_graph(p):
    return {
        p.module_base + COUNTRY_OCCUPATION_DATA_VTABLE_RVA: "country_occupation_data",
        p.module_base + STATE_GARRISON_DATA_VTABLE_RVA: "state_garrison_data",
        p.module_base + GARRISON_STATUS_VTABLE_RVA: "garrison_status",
        p.module_base + GARRISON_STATUS_SECONDARY_VTABLE_RVA: "garrison_status_secondary",
        p.module_base + COUNTRY_COLLABORATION_DATA_VTABLE_RVA: "country_collaboration_data",
        p.module_base + live_probe.OCCUPATION_LAW_VTABLE_RVA: "occupation_law",
        p.module_base + DIVISION_TEMPLATE_VTABLE_RVA: "division_template",
        p.module_base + live_probe.STATE_VTABLE_RVA: "state",
        p.module_base + live_probe.COUNTRY_VTABLE_RVA: "country",
    }


def probe_occupation_object_graph(p, rows, status_by_tag, tbl, max_countries):
    target_vtables = target_vtables_for_graph(p)
    state_ptr_to_id = {}
    for row in rows:
        ptr = row.get("state_ptr")
        if isinstance(ptr, int):
            state_ptr_to_id[ptr] = row["state_id"]
    country_ptr_to_tag_id = {
        rec["country_ptr"]: tag_id
        for tag_id, rec in status_by_tag.items()
        if isinstance(rec.get("country_ptr"), int)
    }

    active_controllers = {}
    for row in rows:
        if row["flags"] & live_probe.ROW_FLAG_OCCUPIED_STATE and row["controller_tag_db_id"]:
            active_controllers[row["controller_tag_db_id"]] = (
                active_controllers.get(row["controller_tag_db_id"], 0) + 1
            )
    active = [
        (tag_id, count)
        for tag_id, count in sorted(
            active_controllers.items(), key=lambda item: item[1], reverse=True
        )
        if tag_id in status_by_tag
    ][:max_countries]

    totals = {
        "active_occupiers_scanned": len(active),
        "validated_country_occupation_data_rows_from_status_container": 0,
        "validated_country_occupation_data_rows": 0,
        "validated_state_garrison_data_rows": 0,
        "state_garrison_rows_with_state_join": 0,
        "state_garrison_rows_with_controller_country_join": 0,
        "state_garrison_rows_with_occupied_country_join": 0,
        "state_garrison_rows_with_occupation_data_join": 0,
        "state_garrison_rows_with_state_and_occupation_data_join": 0,
        "unique_garrison_state_ids": 0,
        "collaboration_data_hits": 0,
        "garrison_status_hits": 0,
        "law_container_law_ptrs": 0,
        "state_list_state_ptrs": 0,
        "garrison_container_state_garrison_ptrs": 0,
        "per_state_law_same_offset_pairs": 0,
        "per_state_law_pairs_with_state_join": 0,
        "per_state_law_pairs_state_in_occupation_state_list": 0,
        "per_state_law_unique_state_assignments": 0,
        "per_state_law_orphan_law_ptrs": 0,
        "per_state_law_orphan_garrison_ptrs": 0,
    }
    countries = []
    all_garrison_state_ids = set()
    all_state_law_keys = set()
    law_name_counts = {}

    for tag_id, occupied_rows in active:
        rec = status_by_tag[tag_id]
        status_ptr = rec["occupation_status_ptr"]
        hits, seen_nodes = collect_reachable_vtable_hits(
            p, status_ptr, COUNTRY_OCCUPATION_STATUS_SIZE, target_vtables
        )
        occupation_data_hits = []
        state_garrison_hits = []
        garrison_status_hits = []
        seen_occupation_data = set()
        seen_state_garrison = set()
        law_container_law_ptrs = 0
        state_list_state_ptrs = 0
        garrison_container_state_garrison_ptrs = 0
        status_occupation_data_container_rows = 0
        status_occupation_data_container_ptr = read_u64(
            p, status_ptr + COUNTRY_OCCUPATION_STATUS_OCCUPATION_DATA_CONTAINER_OFF
        )
        status_occupation_data_by_offset = collect_container_ptrs_by_kind(
            p, status_occupation_data_container_ptr, target_vtables
        ).get("country_occupation_data", {})
        for occupation_data_ptr in status_occupation_data_by_offset.values():
            if (
                read_u64(
                    p,
                    occupation_data_ptr + COUNTRY_OCCUPATION_DATA_PARENT_STATUS_OFF,
                )
                == status_ptr
            ):
                status_occupation_data_container_rows += 1
        totals["validated_country_occupation_data_rows_from_status_container"] += (
            status_occupation_data_container_rows
        )
        per_state_law_pairs = []
        per_state_law_pair_count = 0
        per_state_law_orphan_laws = 0
        per_state_law_orphan_garrisons = 0

        for hit in hits:
            if hit["kind"] == "garrison_status":
                garrison_status_hits.append(hit)
                continue
            if hit["kind"] == "country_occupation_data":
                ptr = hit["ptr"]
                if ptr in seen_occupation_data:
                    continue
                seen_occupation_data.add(ptr)
                parent_status = read_u64(p, ptr + COUNTRY_OCCUPATION_DATA_PARENT_STATUS_OFF)
                occupied_tag_id = read_u32(p, ptr + COUNTRY_OCCUPATION_DATA_OCCUPIED_TAG_OFF)
                state_container = read_u64(
                    p, ptr + COUNTRY_OCCUPATION_DATA_STATE_LIST_CONTAINER_OFF
                )
                law_container = read_u64(
                    p, ptr + COUNTRY_OCCUPATION_DATA_LAW_LIST_CONTAINER_OFF
                )
                garrison_container = read_u64(
                    p, ptr + COUNTRY_OCCUPATION_DATA_GARRISON_LIST_CONTAINER_OFF
                )
                state_container_summary = count_target_ptrs_in_container(
                    p, state_container, target_vtables, state_ptr_to_id
                )
                law_container_summary = count_target_ptrs_in_container(
                    p, law_container, target_vtables, state_ptr_to_id
                )
                garrison_container_summary = count_target_ptrs_in_container(
                    p, garrison_container, target_vtables, state_ptr_to_id
                )
                state_container_state_ids = {
                    item["state_id"]
                    for item in state_container_summary["sample"]
                    if item.get("state_id") is not None
                }
                # The state list is a compact vector for small lists, but can be
                # longer than the small sample. Re-read it only for the membership
                # proof used below.
                state_container_by_kind = collect_container_ptrs_by_kind(
                    p, state_container, target_vtables
                )
                for state_ptr in state_container_by_kind.get("state", {}).values():
                    state_id = state_ptr_to_id.get(state_ptr)
                    if state_id is not None:
                        state_container_state_ids.add(state_id)
                law_by_offset = collect_container_ptrs_by_kind(
                    p, law_container, target_vtables
                ).get("occupation_law", {})
                garrison_by_offset = collect_container_ptrs_by_kind(
                    p, garrison_container, target_vtables
                ).get("state_garrison_data", {})
                paired_offsets = sorted(set(law_by_offset) & set(garrison_by_offset))
                per_state_law_orphan_laws += len(set(law_by_offset) - set(garrison_by_offset))
                per_state_law_orphan_garrisons += len(
                    set(garrison_by_offset) - set(law_by_offset)
                )
                for entry_offset in paired_offsets:
                    per_state_law_pair_count += 1
                    law_ptr = law_by_offset[entry_offset]
                    garrison_ptr = garrison_by_offset[entry_offset]
                    garrison_state_ptr = read_u64(
                        p, garrison_ptr + STATE_GARRISON_DATA_STATE_OFF
                    )
                    garrison_occupation_data_ptr = read_u64(
                        p, garrison_ptr + STATE_GARRISON_DATA_OCCUPATION_DATA_OFF
                    )
                    state_id = state_ptr_to_id.get(garrison_state_ptr)
                    law_name = live_probe.read_pdx_string(
                        p, law_ptr + live_probe.OCCUPATION_LAW_NAME_OFF
                    )
                    state_join_ok = (
                        state_id is not None and garrison_occupation_data_ptr == ptr
                    )
                    state_in_list = state_id in state_container_state_ids
                    totals["per_state_law_same_offset_pairs"] += 1
                    if state_join_ok:
                        totals["per_state_law_pairs_with_state_join"] += 1
                        all_state_law_keys.add((tag_id, state_id))
                    if state_in_list:
                        totals["per_state_law_pairs_state_in_occupation_state_list"] += 1
                    if law_name:
                        law_name_counts[law_name] = law_name_counts.get(law_name, 0) + 1
                    if len(per_state_law_pairs) < 12:
                        per_state_law_pairs.append(
                            {
                                "offset": hx(entry_offset),
                                "state_id": state_id,
                                "state_join_ok": state_join_ok,
                                "state_in_occupation_state_list": state_in_list,
                                "law_ptr": hx(law_ptr),
                                "law_name": law_name,
                                "garrison_ptr": hx(garrison_ptr),
                            }
                        )
                law_ptrs = law_container_summary["target_counts"].get("occupation_law", 0)
                state_ptrs = state_container_summary["state_ptr_rows"]
                garrison_ptrs = garrison_container_summary["target_counts"].get(
                    "state_garrison_data", 0
                )
                law_container_law_ptrs += law_ptrs
                state_list_state_ptrs += state_ptrs
                garrison_container_state_garrison_ptrs += garrison_ptrs
                valid = (
                    parent_status == status_ptr
                    and isinstance(occupied_tag_id, int)
                    and 0 < occupied_tag_id < tbl.count
                    and bool(tbl.tag(occupied_tag_id))
                )
                if valid:
                    totals["validated_country_occupation_data_rows"] += 1
                occupation_data_hits.append(
                    {
                        "ptr": hx(ptr),
                        "path": hit["path"],
                        "parent_status_matches": parent_status == status_ptr,
                        "occupied_tag_db_id": occupied_tag_id,
                        "occupied_tag": tbl.tag(occupied_tag_id)
                        if isinstance(occupied_tag_id, int) and occupied_tag_id < tbl.count
                        else None,
                        "state_container_state_ptrs": state_ptrs,
                        "law_container_law_ptrs": law_ptrs,
                        "garrison_container_state_garrison_ptrs": garrison_ptrs,
                        "state_container_sample": state_container_summary["sample"],
                        "law_container_sample": law_container_summary["sample"],
                        "garrison_container_sample": garrison_container_summary["sample"],
                    }
                )
                continue
            if hit["kind"] == "state_garrison_data":
                ptr = hit["ptr"]
                if ptr in seen_state_garrison:
                    continue
                seen_state_garrison.add(ptr)
                country_ptr = read_u64(p, ptr + STATE_GARRISON_DATA_COUNTRY_OFF)
                state_ptr = read_u64(p, ptr + STATE_GARRISON_DATA_STATE_OFF)
                occupation_data_ptr = read_u64(p, ptr + STATE_GARRISON_DATA_OCCUPATION_DATA_OFF)
                state_id = state_ptr_to_id.get(state_ptr)
                occupation_data_valid = (
                    live_probe.is_heap_ptr(occupation_data_ptr)
                    and read_u64(p, occupation_data_ptr)
                    == p.module_base + COUNTRY_OCCUPATION_DATA_VTABLE_RVA
                )
                occupation_data_occupied_tag_id = (
                    read_u32(
                        p,
                        occupation_data_ptr + COUNTRY_OCCUPATION_DATA_OCCUPIED_TAG_OFF,
                    )
                    if occupation_data_valid
                    else None
                )
                country_ptr_tag_id = country_ptr_to_tag_id.get(country_ptr)
                controller_country_matches = country_ptr == rec["country_ptr"]
                occupied_country_matches = (
                    isinstance(occupation_data_occupied_tag_id, int)
                    and country_ptr_tag_id == occupation_data_occupied_tag_id
                )
                if state_id is not None:
                    totals["state_garrison_rows_with_state_join"] += 1
                    all_garrison_state_ids.add(state_id)
                if controller_country_matches:
                    totals["state_garrison_rows_with_controller_country_join"] += 1
                if occupied_country_matches:
                    totals["state_garrison_rows_with_occupied_country_join"] += 1
                if occupation_data_valid:
                    totals["state_garrison_rows_with_occupation_data_join"] += 1
                if state_id is not None and occupation_data_valid:
                    totals["state_garrison_rows_with_state_and_occupation_data_join"] += 1
                    totals["validated_state_garrison_data_rows"] += 1
                state_garrison_hits.append(
                    {
                        "ptr": hx(ptr),
                        "path": hit["path"],
                        "country_ptr": hx(country_ptr),
                        "country_ptr_tag_db_id": country_ptr_tag_id,
                        "country_ptr_tag": tbl.tag(country_ptr_tag_id)
                        if isinstance(country_ptr_tag_id, int)
                        and country_ptr_tag_id < tbl.count
                        else None,
                        "country_ptr_matches_controller": controller_country_matches,
                        "country_ptr_matches_occupied": occupied_country_matches,
                        "state_id": state_id,
                        "occupation_data_ptr": hx(occupation_data_ptr),
                        "occupation_data_valid": occupation_data_valid,
                        "occupation_data_occupied_tag_db_id": occupation_data_occupied_tag_id,
                        "occupation_data_occupied_tag": tbl.tag(occupation_data_occupied_tag_id)
                        if isinstance(occupation_data_occupied_tag_id, int)
                        and occupation_data_occupied_tag_id < tbl.count
                        else None,
                    }
                )

        collab_hits = []
        collab_seen_nodes = 0
        if rec.get("collaboration_status_ptr"):
            collab_root_hits, collab_seen_nodes = collect_reachable_vtable_hits(
                p,
                rec["collaboration_status_ptr"],
                COUNTRY_COLLABORATION_STATUS_SIZE,
                target_vtables,
            )
            for hit in collab_root_hits:
                if hit["kind"] == "country_collaboration_data":
                    collab_hits.append(hit)

        totals["law_container_law_ptrs"] += law_container_law_ptrs
        totals["state_list_state_ptrs"] += state_list_state_ptrs
        totals["garrison_container_state_garrison_ptrs"] += garrison_container_state_garrison_ptrs
        totals["per_state_law_orphan_law_ptrs"] += per_state_law_orphan_laws
        totals["per_state_law_orphan_garrison_ptrs"] += per_state_law_orphan_garrisons
        totals["collaboration_data_hits"] += len(collab_hits)
        totals["garrison_status_hits"] += len(garrison_status_hits)
        countries.append(
            {
                "tag_db_id": tag_id,
                "tag": rec["country_tag"],
                "occupied_state_rows": occupied_rows,
                "occupation_graph_nodes_seen": seen_nodes,
                "collaboration_graph_nodes_seen": collab_seen_nodes,
                "status_occupation_data_container_ptr": hx(
                    status_occupation_data_container_ptr
                ),
                "status_occupation_data_container_rows": status_occupation_data_container_rows,
                "country_occupation_data_hits": len(occupation_data_hits),
                "country_occupation_data_sample": occupation_data_hits[:8],
                "state_garrison_data_hits": len(state_garrison_hits),
                "state_garrison_data_sample": state_garrison_hits[:8],
                "per_state_law_same_offset_pairs": per_state_law_pair_count,
                "per_state_law_pair_sample": per_state_law_pairs[:8],
                "per_state_law_orphan_law_ptrs": per_state_law_orphan_laws,
                "per_state_law_orphan_garrison_ptrs": per_state_law_orphan_garrisons,
                "collaboration_data_hits": len(collab_hits),
                "collaboration_data_sample": [
                    {"ptr": hx(hit["ptr"]), "path": hit["path"]} for hit in collab_hits[:8]
                ],
                "garrison_status_hits": len(garrison_status_hits),
            }
        )

    totals["unique_garrison_state_ids"] = len(all_garrison_state_ids)
    totals["per_state_law_unique_state_assignments"] = len(all_state_law_keys)
    return {
        "offsets": {
            "COUNTRY_OCCUPATION_DATA_VTABLE_RVA": hx(COUNTRY_OCCUPATION_DATA_VTABLE_RVA),
            "STATE_GARRISON_DATA_VTABLE_RVA": hx(STATE_GARRISON_DATA_VTABLE_RVA),
            "GARRISON_STATUS_VTABLE_RVA": hx(GARRISON_STATUS_VTABLE_RVA),
            "COUNTRY_COLLABORATION_DATA_VTABLE_RVA": hx(
                COUNTRY_COLLABORATION_DATA_VTABLE_RVA
            ),
            "COUNTRY_OCCUPATION_STATUS_OCCUPATION_DATA_CONTAINER_OFF": hx(
                COUNTRY_OCCUPATION_STATUS_OCCUPATION_DATA_CONTAINER_OFF
            ),
            "COUNTRY_OCCUPATION_DATA_PARENT_STATUS_OFF": hx(
                COUNTRY_OCCUPATION_DATA_PARENT_STATUS_OFF
            ),
            "COUNTRY_OCCUPATION_DATA_OCCUPIED_TAG_OFF": hx(
                COUNTRY_OCCUPATION_DATA_OCCUPIED_TAG_OFF
            ),
            "COUNTRY_OCCUPATION_DATA_STATE_LIST_CONTAINER_OFF": hx(
                COUNTRY_OCCUPATION_DATA_STATE_LIST_CONTAINER_OFF
            ),
            "COUNTRY_OCCUPATION_DATA_LAW_LIST_CONTAINER_OFF": hx(
                COUNTRY_OCCUPATION_DATA_LAW_LIST_CONTAINER_OFF
            ),
            "COUNTRY_OCCUPATION_DATA_GARRISON_LIST_CONTAINER_OFF": hx(
                COUNTRY_OCCUPATION_DATA_GARRISON_LIST_CONTAINER_OFF
            ),
            "STATE_GARRISON_DATA_COUNTRY_OFF": hx(STATE_GARRISON_DATA_COUNTRY_OFF),
            "STATE_GARRISON_DATA_STATE_OFF": hx(STATE_GARRISON_DATA_STATE_OFF),
            "STATE_GARRISON_DATA_OCCUPATION_DATA_OFF": hx(
                STATE_GARRISON_DATA_OCCUPATION_DATA_OFF
            ),
        },
        "totals": totals,
        "law_name_counts": dict(sorted(law_name_counts.items())),
        "countries": countries,
        "verdict": "validated_per_state_law_join_roots_unclassified_garrison_scalars"
        if totals["validated_country_occupation_data_rows"]
        and totals["validated_state_garrison_data_rows"]
        and totals["per_state_law_pairs_with_state_join"]
        else "not_validated",
        "field_contract": (
            "Country occupation data and state garrison data roots are usable for "
            "state/tag joins. CStateGarrisonData+0x50 is the CState join and "
            "+0x58 is the CCountryOccupationData join; +0x48 is not a controller "
            "country join and must not be used for controller identity. Per-state "
            "law override joins are classified by matching occupation-law pointers "
            "and CStateGarrisonData pointers at the same offsets in sibling "
            "CCountryOccupationData containers. Garrison need/equilibrium/progress "
            "still require scalar decoding or a controlled fixture that changes one "
            "garrison value at a time."
        ),
    }


def expected_legitimacy_raws(country_gie_sample):
    out = {}
    for rec in country_gie_sample or []:
        value = rec.get("legitimacy")
        country_tag = rec.get("country_tag")
        if value is None or country_tag is None:
            continue
        for scale in (1000, 10000, 100000):
            raw = int(round(float(value) * scale))
            out.setdefault(raw, []).append({"country_tag": country_tag, "scale": scale})
    return out


def country_gie_by_country(country_gie_sample):
    return {
        rec["country_tag"]: rec
        for rec in country_gie_sample or []
        if rec.get("country_tag")
    }


def scan_blob_for_gie_scalars(blob, tag_ids_by_tag, legitimacy_raws):
    hits = []
    tag_by_id = {tag_id: tag for tag, tag_id in tag_ids_by_tag.items() if tag_id}
    for offset in range(0, max(0, len(blob) - 4) + 1, 4):
        value = struct.unpack_from("<I", blob, offset)[0]
        tag = tag_by_id.get(value)
        if tag:
            hits.append({"offset": hx(offset), "kind": "tag_db_id", "tag": tag, "value": value})
        legit_matches = legitimacy_raws.get(value)
        if legit_matches:
            hits.append(
                {
                    "offset": hx(offset),
                    "kind": "legitimacy_fixed_candidate",
                    "value": value,
                    "matches": legit_matches[:4],
                }
            )
        if len(hits) >= 16:
            break
    return hits


def evaluate_gie_candidate_scope(scope, hits, target_tag, expected, legitimacy_raws):
    expected_host = expected.get("host_tag") if expected else None
    annotated = []
    host_hits = 0
    country_hits = 0
    unique_legitimacy_hits = 0
    for hit in hits:
        rec = dict(hit)
        role = None
        if hit.get("kind") == "tag_db_id":
            if hit.get("tag") == expected_host:
                role = "expected_host_tag"
                host_hits += 1
            elif hit.get("tag") == target_tag:
                role = "expected_country_tag"
                country_hits += 1
            else:
                role = "other_target_set_tag"
        elif hit.get("kind") == "legitimacy_fixed_candidate":
            matches = legitimacy_raws.get(hit.get("value"), [])
            target_matches = [m for m in matches if m.get("country_tag") == target_tag]
            if target_matches and len(matches) == 1:
                role = "unique_expected_legitimacy"
                unique_legitimacy_hits += 1
            elif target_matches:
                role = "shared_expected_legitimacy"
            else:
                role = "other_target_legitimacy"
        rec["contract_role"] = role
        annotated.append(rec)

    contract_match = bool(expected_host) and host_hits > 0 and (
        country_hits > 0 or unique_legitimacy_hits > 0
    )
    if not expected:
        reason = "no_country_gie_expected_row_for_tag"
    elif contract_match:
        reason = "candidate_scope_has_expected_host_and_target_or_unique_legitimacy"
    else:
        reason = "missing_expected_host_plus_target_or_unique_legitimacy"
    return {
        "scope": scope,
        "hits": annotated,
        "expected_country_tag": target_tag if expected else None,
        "expected_host_tag": expected_host,
        "expected_legitimacy": expected.get("legitimacy") if expected else None,
        "expected_exile_army_leaders": (
            expected.get("exile_army_leaders") if expected else None
        ),
        "expected_host_hits": host_hits,
        "expected_country_hits": country_hits,
        "unique_expected_legitimacy_hits": unique_legitimacy_hits,
        "contract_match": contract_match,
        "reason": reason,
    }


def probe_country_gie_live_graph(p, status_by_tag, tbl, collab_fixture):
    samples = collab_fixture.get("country_gie_sample") or []
    expected_by_country = country_gie_by_country(samples)
    country_tags = {
        rec.get("country_tag")
        for rec in samples
        if rec.get("country_tag")
    }
    host_tags = set((collab_fixture.get("country_gie_hosts") or {}).keys())
    target_tags = sorted(country_tags | host_tags)
    by_tag = {
        rec["country_tag"]: (tag_id, rec)
        for tag_id, rec in status_by_tag.items()
        if rec.get("country_tag")
    }
    tag_ids_by_tag = {tag: by_tag[tag][0] for tag in target_tags if tag in by_tag}
    legitimacy_raws = expected_legitimacy_raws(samples)
    target_vtables = target_vtables_for_graph(p)
    rows = []
    totals = {
        "target_tags": target_tags,
        "target_tag_count": len(target_tags),
        "valid_collaboration_status_roots": 0,
        "country_collaboration_data_hits": 0,
        "scalar_candidate_hits": 0,
        "candidate_contract_match_scopes": 0,
        "rejected_candidate_scopes": 0,
        "missing_live_country_rows": 0,
    }

    for tag in target_tags:
        item = by_tag.get(tag)
        if not item:
            totals["missing_live_country_rows"] += 1
            rows.append({"tag": tag, "status": "missing_live_country"})
            continue
        tag_id, rec = item
        collab_ptr = rec.get("collaboration_status_ptr")
        if collab_ptr:
            totals["valid_collaboration_status_roots"] += 1
        root_scan = []
        root_contract = None
        expected = expected_by_country.get(tag)
        root_blob = p.try_read(collab_ptr, COUNTRY_COLLABORATION_STATUS_SIZE) if collab_ptr else None
        if root_blob:
            root_scan = scan_blob_for_gie_scalars(root_blob, tag_ids_by_tag, legitimacy_raws)
            totals["scalar_candidate_hits"] += len(root_scan)
            root_contract = evaluate_gie_candidate_scope(
                "collaboration_status_root",
                root_scan,
                tag,
                expected,
                legitimacy_raws,
            )
            if root_scan:
                if root_contract["contract_match"]:
                    totals["candidate_contract_match_scopes"] += 1
                else:
                    totals["rejected_candidate_scopes"] += 1
        collab_hits = []
        collab_seen_nodes = 0
        hit_scans = []
        if collab_ptr:
            graph_hits, collab_seen_nodes = collect_reachable_vtable_hits(
                p,
                collab_ptr,
                COUNTRY_COLLABORATION_STATUS_SIZE,
                target_vtables,
            )
            for hit in graph_hits:
                if hit["kind"] != "country_collaboration_data":
                    continue
                collab_hits.append(hit)
                blob = p.try_read(hit["ptr"], GRAPH_NODE_SIZE)
                scalar_hits = (
                    scan_blob_for_gie_scalars(blob, tag_ids_by_tag, legitimacy_raws)
                    if blob
                    else []
                )
                totals["scalar_candidate_hits"] += len(scalar_hits)
                if len(hit_scans) < 8:
                    scalar_contract = evaluate_gie_candidate_scope(
                        "country_collaboration_data",
                        scalar_hits,
                        tag,
                        expected,
                        legitimacy_raws,
                    )
                    if scalar_hits:
                        if scalar_contract["contract_match"]:
                            totals["candidate_contract_match_scopes"] += 1
                        else:
                            totals["rejected_candidate_scopes"] += 1
                    hit_scans.append(
                        {
                            "ptr": hx(hit["ptr"]),
                            "path": hit["path"],
                            "scalar_hits": scalar_hits,
                            "scalar_contract": scalar_contract,
                        }
                    )
        totals["country_collaboration_data_hits"] += len(collab_hits)
        rows.append(
            {
                "tag": tag,
                "tag_db_id": tag_id,
                "collaboration_status_ptr": hx(collab_ptr),
                "graph_nodes_seen": collab_seen_nodes,
                "country_collaboration_data_hits": len(collab_hits),
                "root_scalar_hits": root_scan,
                "root_scalar_contract": root_contract,
                "collaboration_data_scalar_scans": hit_scans,
            }
        )

    if not target_tags:
        verdict = "fixture_has_no_country_gie_targets"
    elif (
        totals["valid_collaboration_status_roots"] == len(target_tags)
        and totals["country_collaboration_data_hits"] == 0
    ):
        verdict = "blocked_no_country_collaboration_data_hits_for_gie_country_set"
    elif totals["candidate_contract_match_scopes"]:
        verdict = "candidate_contract_hits_require_controlled_fixture"
    elif totals["scalar_candidate_hits"]:
        verdict = "rejected_targeted_scalar_hits_no_gie_contract_current_fixture"
    elif totals["country_collaboration_data_hits"] and not totals["scalar_candidate_hits"]:
        verdict = "unclassified_collaboration_data_hits_without_gie_join_scalars"
    else:
        verdict = "blocked_no_valid_live_gie_join"

    return {
        "verdict": verdict,
        "totals": totals,
        "rows": rows,
        "contract": (
            "This bounded scan is scoped to CCountryCollaborationStatus roots for "
            "country-scope GIE owner tags plus their host tags. It can reject the "
            "collaboration-status graph as a direct source only when all target "
            "countries have valid roots and the graph contains no "
            "CCountryCollaborationData hits. Scalar/tag hits are rejected for the "
            "current fixture unless a scan scope contains the expected host tag "
            "and either the expected country tag or a unique legitimacy raw for "
            "that country. Contract-matching hits are still candidates only and "
            "need a controlled fixture before being wired."
        ),
    }


def collaboration_gie_fixture_summary(oracle_occupation, save_melt):
    summary = {
        "oracle_occupation_path": str(oracle_occupation) if oracle_occupation else None,
        "save_melt_path": str(save_melt) if save_melt else None,
        "save_date": None,
        "countries_with_collaboration_payload": 0,
        "collaboration_note": None,
        "gie_assignment_key_counts": {},
        "government_in_exile_tag_rows": 0,
        "government_in_exile_tag_leader_context_rows": 0,
        "government_in_exile_tag_asset_scope_rows": 0,
        "government_in_exile_tag_unknown_context_rows": 0,
        "government_in_exile_tag_context_counts": {},
        "government_in_exile_tags": {},
        "government_in_exile_tag_sample": [],
        "country_gie_rows": 0,
        "country_gie_countries": {},
        "country_gie_hosts": {},
        "country_gie_sample": [],
        "verdict": "fixture_not_checked",
        "fixture_contract": (
            "Collaboration/GIE detail can be wired only when a save fixture has "
            "non-empty collaboration or country-scope government-in-exile target "
            "data that can be joined to live "
            "CCountryCollaborationStatus/CCountryCollaborationData objects by tag. "
            "government_in_exile_tag rows inside leaders, ships, deployments, or "
            "air wings are asset attribution and are not occupation/collaboration "
            "country detail."
        ),
    }
    path = Path(oracle_occupation) if oracle_occupation else None
    if path and path.is_file():
        data = json.loads(path.read_text(encoding="utf-8"))
        meta = data.get("meta", {})
        summary["save_date"] = meta.get("save_date")
        summary["collaboration_note"] = (
            meta.get("notes", {}).get("collaboration")
            if isinstance(meta.get("notes"), dict)
            else None
        )
        count = 0
        for rec in data.get("countries", {}).values():
            collab = rec.get("collaboration") if isinstance(rec, dict) else None
            if isinstance(collab, dict) and collab:
                count += 1
        summary["countries_with_collaboration_payload"] = count

    melt = Path(save_melt) if save_melt else None
    if melt and melt.is_file():
        text = melt.read_text(encoding="utf-8-sig", errors="replace")
        key_counts = {}
        for key in GIE_SAVE_ASSIGNMENT_KEYS:
            key_counts[key] = len(
                re.findall(rf"(?m)(?:^|\s){re.escape(key)}\s*=", text)
            )
        summary["gie_assignment_key_counts"] = key_counts
        country_marks = top_level_country_marks(text)
        for match in re.finditer(r'government_in_exile_tag\s*=\s*"([^"]+)"', text):
            tag = match.group(1)
            start = max(0, match.start() - 1000)
            end = min(len(text), match.end() + 700)
            window = text[start:end]
            context = classify_government_in_exile_tag_window(window)
            context_parts = set(context.split("+")) if context else set()
            asset_context = is_asset_gie_tag_context(context)
            summary["government_in_exile_tag_rows"] += 1
            summary["government_in_exile_tag_context_counts"][context] = (
                summary["government_in_exile_tag_context_counts"].get(context, 0)
                + 1
            )
            if "leader" in context_parts:
                summary["government_in_exile_tag_leader_context_rows"] += 1
            if asset_context:
                summary["government_in_exile_tag_asset_scope_rows"] += 1
            if context == "unknown":
                summary["government_in_exile_tag_unknown_context_rows"] += 1
            summary["government_in_exile_tags"][tag] = (
                summary["government_in_exile_tags"].get(tag, 0) + 1
            )
            if len(summary["government_in_exile_tag_sample"]) < 8:
                summary["government_in_exile_tag_sample"].append(
                    {
                        "tag": tag,
                        "line": line_no_at(text, match.start()),
                        "context": context,
                        "asset_scope": asset_context,
                        "context_has_attack_skill": "attack_skill=" in window,
                        "context_has_defense_skill": "defense_skill=" in window,
                        "context_has_ship_name": "ship_name=" in window,
                        "context_has_military_deployment_line": (
                            "military_deployment_line=" in window
                        ),
                        "context_has_air_wings": "air_wings=" in window,
                    }
                )
        for match in re.finditer(
            r'hosting_our_government_in_exile\s*=\s*"([^"]+)"', text
        ):
            rec = parse_country_gie_window(text, match)
            rec["country_tag"] = country_tag_for_offset(country_marks, match.start())
            summary["country_gie_rows"] += 1
            country_tag = rec.get("country_tag")
            if country_tag:
                summary["country_gie_countries"][country_tag] = (
                    summary["country_gie_countries"].get(country_tag, 0) + 1
                )
            host = rec["host_tag"]
            summary["country_gie_hosts"][host] = (
                summary["country_gie_hosts"].get(host, 0) + 1
            )
            if len(summary["country_gie_sample"]) < 8:
                summary["country_gie_sample"].append(rec)

    only_asset_tags = (
        summary["government_in_exile_tag_rows"] > 0
        and summary["government_in_exile_tag_rows"]
        == summary["government_in_exile_tag_asset_scope_rows"]
    )
    if (
        summary["countries_with_collaboration_payload"] == 0
        and not summary["government_in_exile_tag_rows"]
        and not summary["country_gie_rows"]
        and not any(summary["gie_assignment_key_counts"].values())
    ):
        summary["verdict"] = "fixture_blocked_no_collaboration_or_gie_save_payload"
    elif (
        summary["countries_with_collaboration_payload"] == 0
        and only_asset_tags
        and not summary["country_gie_rows"]
        and not any(
            count
            for key, count in summary["gie_assignment_key_counts"].items()
            if key != "government_in_exile_tag"
        )
    ):
        summary["verdict"] = (
            "fixture_blocked_collaboration_empty_gie_tags_are_asset_scope"
        )
    elif (
        summary["countries_with_collaboration_payload"] == 0
        and only_asset_tags
        and summary["country_gie_rows"] > 0
    ):
        summary["verdict"] = (
            "fixture_has_country_gie_payload_unjoined_to_live_collaboration_status"
        )
    elif (
        summary["countries_with_collaboration_payload"] > 0
        or summary["country_gie_rows"] > 0
        or any(summary["gie_assignment_key_counts"].values())
    ):
        summary["verdict"] = "fixture_has_candidate_payload"
    return summary


def write_md(out_dir, proof):
    speed = proof["speed_signals"]
    template = proof["garrison_template_candidates"]
    graph = proof.get("occupation_object_graph", {})
    graph_totals = graph.get("totals", {})
    collab_fixture = proof.get("collaboration_gie_fixture", {})
    gie_live = proof.get("country_gie_live_graph", {})
    gie_live_totals = gie_live.get("totals", {})
    lines = [
        "# Remaining Occupation Field Probe",
        "",
        "## Speed Signals",
        "",
        "- Resistance speed offset: `%s`"
        % speed["offsets"]["STATE_OFF_RESISTANCE_SPEED"],
        "- Compliance speed offset: `%s`"
        % speed["offsets"]["STATE_OFF_COMPLIANCE_SPEED"],
        "- Resistance delta rows/matches: `%d/%d`"
        % (speed["resistance_delta_rows"], speed["resistance_speed_matches"]),
        "- Compliance delta rows/matches: `%d/%d`"
        % (speed["compliance_delta_rows"], speed["compliance_speed_matches"]),
        "- Verdict: `%s`" % speed["verdict"],
        "",
        "## Garrison Template Candidates",
        "",
        "- Candidate offsets: `%s`" % ", ".join(template["candidate_offsets"]),
        "- Verdict: `%s`" % template["verdict"],
        "- Fixture contract: %s" % template["fixture_contract"],
    ]
    if graph:
        lines.extend(
            [
                "",
                "## Occupation Object Graph",
                "",
                "- Verdict: `%s`" % graph.get("verdict"),
                "- Active occupiers scanned: `%s`"
                % graph_totals.get("active_occupiers_scanned"),
                "- Valid country occupation data rows: `%s`"
                % graph_totals.get("validated_country_occupation_data_rows"),
                "- Valid occupation data rows from `CCountryOccupationStatus+0x48`: `%s`"
                % graph_totals.get(
                    "validated_country_occupation_data_rows_from_status_container"
                ),
                "- Valid state garrison data rows: `%s`"
                % graph_totals.get("validated_state_garrison_data_rows"),
                "- State+occupation-data joined garrison rows: `%s`"
                % graph_totals.get("state_garrison_rows_with_state_and_occupation_data_join"),
                "- Controller-country matches at `CStateGarrisonData+0x48`: `%s`"
                % graph_totals.get("state_garrison_rows_with_controller_country_join"),
                "- Occupied-country matches at `CStateGarrisonData+0x48`: `%s`"
                % graph_totals.get("state_garrison_rows_with_occupied_country_join"),
                "- State garrison unique state ids: `%s`"
                % graph_totals.get("unique_garrison_state_ids"),
                "- Law-container law pointers: `%s`"
                % graph_totals.get("law_container_law_ptrs"),
                "- State-list state pointers: `%s`"
                % graph_totals.get("state_list_state_ptrs"),
                "- Garrison-container state-garrison pointers: `%s`"
                % graph_totals.get("garrison_container_state_garrison_ptrs"),
                "- Per-state law same-offset law/garrison pairs: `%s`"
                % graph_totals.get("per_state_law_same_offset_pairs"),
                "- Per-state law pairs with state join: `%s`"
                % graph_totals.get("per_state_law_pairs_with_state_join"),
                "- Per-state law pairs with state in occupation state-list: `%s`"
                % graph_totals.get("per_state_law_pairs_state_in_occupation_state_list"),
                "- Per-state law unique state assignments: `%s`"
                % graph_totals.get("per_state_law_unique_state_assignments"),
                "- Per-state law orphan law/garrison ptrs: `%s/%s`"
                % (
                    graph_totals.get("per_state_law_orphan_law_ptrs"),
                    graph_totals.get("per_state_law_orphan_garrison_ptrs"),
                ),
                "- Collaboration data hits: `%s`"
                % graph_totals.get("collaboration_data_hits"),
                "- Contract: %s" % graph.get("field_contract"),
            ]
        )
    if collab_fixture:
        lines.extend(
            [
                "",
                "## Collaboration / GIE Fixture",
                "",
                "- Verdict: `%s`" % collab_fixture.get("verdict"),
                "- Oracle occupation path: `%s`"
                % collab_fixture.get("oracle_occupation_path"),
                "- Save melt path: `%s`" % collab_fixture.get("save_melt_path"),
                "- Save date: `%s`" % collab_fixture.get("save_date"),
                "- Countries with collaboration payload: `%s`"
                % collab_fixture.get("countries_with_collaboration_payload"),
                "- GIE assignment key counts: `%s`"
                % collab_fixture.get("gie_assignment_key_counts"),
                "- `government_in_exile_tag` rows / leader-context rows: `%s/%s`"
                % (
                    collab_fixture.get("government_in_exile_tag_rows"),
                    collab_fixture.get(
                        "government_in_exile_tag_leader_context_rows"
                    ),
                ),
                "- `government_in_exile_tag` asset-scope / unknown rows: `%s/%s`"
                % (
                    collab_fixture.get("government_in_exile_tag_asset_scope_rows"),
                    collab_fixture.get("government_in_exile_tag_unknown_context_rows"),
                ),
                "- `government_in_exile_tag` context counts: `%s`"
                % collab_fixture.get("government_in_exile_tag_context_counts"),
                "- `government_in_exile_tag` tag counts: `%s`"
                % collab_fixture.get("government_in_exile_tags"),
                "- Country-scope GIE rows: `%s`"
                % collab_fixture.get("country_gie_rows"),
                "- Country-scope GIE country counts: `%s`"
                % collab_fixture.get("country_gie_countries"),
                "- Country-scope GIE host counts: `%s`"
                % collab_fixture.get("country_gie_hosts"),
                "- Country-scope GIE sample: `%s`"
                % collab_fixture.get("country_gie_sample"),
                "- Collaboration note: %s"
                % collab_fixture.get("collaboration_note"),
                "- Fixture contract: %s"
                % collab_fixture.get("fixture_contract"),
            ]
        )
    if gie_live:
        lines.extend(
            [
                "",
                "## Country GIE Live Graph",
                "",
                "- Verdict: `%s`" % gie_live.get("verdict"),
                "- Target tags: `%s`" % gie_live_totals.get("target_tags"),
                "- Valid collaboration-status roots: `%s/%s`"
                % (
                    gie_live_totals.get("valid_collaboration_status_roots"),
                    gie_live_totals.get("target_tag_count"),
                ),
                "- Country collaboration-data hits: `%s`"
                % gie_live_totals.get("country_collaboration_data_hits"),
                "- Scalar candidate hits: `%s`"
                % gie_live_totals.get("scalar_candidate_hits"),
                "- Contract-match candidate scopes: `%s`"
                % gie_live_totals.get("candidate_contract_match_scopes"),
                "- Rejected candidate scopes: `%s`"
                % gie_live_totals.get("rejected_candidate_scopes"),
                "- Missing live country rows: `%s`"
                % gie_live_totals.get("missing_live_country_rows"),
                "- Contract: %s" % gie_live.get("contract"),
            ]
        )
    compare = template.get("oracle_compare")
    if compare:
        lines.extend(["", "## Oracle Candidate Compare", ""])
        for offset, rec in compare["offsets"].items():
            lines.append(
                "- `%s`: exact `%d`, missing `%d`, mismatch `%d`"
                % (offset, rec["exact"], rec["missing"], rec["mismatch"])
            )
    (out_dir / "remaining_fields_PROOF.md").write_text(
        "\n".join(lines) + "\n", encoding="utf-8"
    )


def run(out_dir, seconds, oracle_occupation, graph_countries, save_melt):
    p = winmem.open_process("hoi4.exe")
    if p is None:
        raise RuntimeError("hoi4.exe not running")
    with p:
        fp = live_probe.checked_fingerprint(p)
        tbl = tag_table.TagTable.locate(p)
        if tbl is None:
            raise RuntimeError("tag table not found")
        roots = live_probe.resolve_state_roots(p)
        status_by_tag, status_summary = live_probe.read_country_occupation_status(
            p, roots["game_state"], tbl
        )
        rows, invalid_states = live_probe.read_state_rows(
            p, roots["state_roots"], tbl, status_by_tag
        )
        collab_fixture = collaboration_gie_fixture_summary(oracle_occupation, save_melt)
        proof = {
            "process": {
                "pid": fp.get("pid"),
                "base": hx(fp.get("base")),
                "sha256": fp.get("sha256"),
            },
            "state_roots": len(roots["state_roots"]),
            "invalid_state_rows": invalid_states,
            "country_occupation_status": status_summary,
            "speed_signals": validate_speed_signals(p, roots["state_roots"], seconds),
            "garrison_template_candidates": probe_template_candidates(
                p, status_by_tag, oracle_occupation
            ),
            "occupation_object_graph": probe_occupation_object_graph(
                p, rows, status_by_tag, tbl, graph_countries
            ),
            "collaboration_gie_fixture": collab_fixture,
            "country_gie_live_graph": probe_country_gie_live_graph(
                p, status_by_tag, tbl, collab_fixture
            ),
        }

    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "remaining_fields_probe.json").write_text(
        json.dumps(proof, indent=2), encoding="utf-8"
    )
    write_md(out_dir, proof)
    return proof


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out-dir", required=True)
    parser.add_argument("--seconds", type=int, default=5)
    parser.add_argument("--graph-countries", type=int, default=8)
    parser.add_argument("--oracle-occupation", default="save_oracle/output/occupation.json")
    parser.add_argument("--save-melt", default=None)
    args = parser.parse_args(argv)
    proof = run(
        Path(args.out_dir),
        args.seconds,
        args.oracle_occupation,
        args.graph_countries,
        args.save_melt,
    )
    print(json.dumps({"out_dir": args.out_dir, "proof": proof}, indent=2))


if __name__ == "__main__":
    raise SystemExit(main())
