"""Bounded live/save oracle search for the construction target-state root.

This is a lane-local development probe. It does not collect production data.
It joins the current live construction queue to a save by stable line identity
and searches a bounded memory set for fields that equal the save-visible target
location or its mapped target state.
"""

from __future__ import annotations

import argparse
import json
import os
import struct
import sys
import time
from pathlib import Path
from typing import Any

PROBE_DIR = Path(__file__).resolve().parent
LANE_DIR = PROBE_DIR.parent
REPO_ROOT = LANE_DIR.parent.parent.parent
SAVE_ORACLE_DIR = REPO_ROOT / "save_oracle"
STATIC_PROVINCE_TO_STATE_PATH = REPO_ROOT / "map_pipeline" / "assets" / "province_to_state.json"

sys.path.insert(0, str(PROBE_DIR))
sys.path.insert(0, str(SAVE_ORACLE_DIR))

import live_probe_870ebf as live  # type: ignore  # noqa: E402
import parse_save as ps  # type: ignore  # noqa: E402
import target_state_same_frame_observer as save_parser  # type: ignore  # noqa: E402


SCHEMA = "construction_buildings_state_damage_live_target_state_bounded_root_search_v1"

LINE_SCAN_BYTES = 0x180
SUB_SCAN_BYTES = 0x240
TEMPLATE_DEF_SCAN_BYTES = 0x200
ONE_HOP_SCAN_BYTES = 0x180
MAX_ONE_HOP_BASES_PER_ROW = 96

STATE_LOCATION_TEMPLATES = {
    "air_base",
    "anti_air_building",
    "arms_factory",
    "dockyard",
    "fuel_silo",
    "industrial_complex",
    "infrastructure",
    "nuclear_reactor",
    "radar_station",
    "rocket_site",
    "synthetic_refinery",
}

PROVINCE_LOCATION_TEMPLATES = {
    "bunker",
    "coastal_bunker",
    "naval_base",
    "rail_way",
    "supply_node",
}

REJECTED_CANDIDATE_KEYS = {
    "sub+0xC0:u16:target_state_id",
    "sub+0xC0:u32:target_state_id",
    "sub+0xC0:i32:target_state_id",
    "sub+0xC0:u64_low:target_state_id",
    "sub+0xC0:u16:exact_location_id",
    "sub+0xC0:u32:exact_location_id",
    "sub+0xC0:i32:exact_location_id",
    "sub+0xC0:u64_low:exact_location_id",
    "sub+0xC8:u16:target_state_id",
    "sub+0xC8:u32:target_state_id",
    "sub+0xC8:i32:target_state_id",
    "sub+0xC8:u64_low:target_state_id",
    "sub+0xC8:u16:exact_location_id",
    "sub+0xC8:u32:exact_location_id",
    "sub+0xC8:i32:exact_location_id",
    "sub+0xC8:u64_low:exact_location_id",
}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", required=True)
    parser.add_argument("--save", required=True)
    parser.add_argument("--pid", type=int, default=None)
    parser.add_argument("--rakaly", default=None)
    parser.add_argument("--max-regions", type=int, default=None)
    parser.add_argument("--min-rows", type=int, default=8)
    parser.add_argument("--min-unique-targets", type=int, default=2)
    args = parser.parse_args()

    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    proof = run_probe(
        pid=args.pid,
        save_path=Path(args.save),
        rakaly_arg=args.rakaly,
        out_dir=out_path.parent,
        max_regions=args.max_regions,
        min_rows=args.min_rows,
        min_unique_targets=args.min_unique_targets,
    )
    with out_path.open("w", encoding="utf-8") as handle:
        json.dump(proof, handle, indent=2, sort_keys=True)
        handle.write("\n")

    print(
        json.dumps(
            {
                "out": str(out_path),
                "status": proof["status"],
                "accepted_rows": proof["accepted_rows"],
                "target_state_all_row_candidates": proof["candidate_summary"][
                    "target_state_all_row_candidate_count"
                ],
                "exact_location_all_row_candidates": proof["candidate_summary"][
                    "exact_location_all_row_candidate_count"
                ],
            },
            sort_keys=True,
        )
    )
    return 0 if proof["status"] != "blocked_no_stable_live_save_rows" else 2


def run_probe(
    *,
    pid: int | None,
    save_path: Path,
    rakaly_arg: str | None,
    out_dir: Path,
    max_regions: int | None,
    min_rows: int,
    min_unique_targets: int,
) -> dict[str, Any]:
    started = time.time()
    save_rows, save_meta, save_topology = read_save_rows(save_path, rakaly_arg, out_dir)

    p = open_target_process(pid)
    with p:
        process_meta = {
            "requested_pid": pid,
            "pid": getattr(p, "pid", None),
            "module_name": getattr(p, "module_name", None),
            "module_base": hex(getattr(p, "module_base", 0) or 0),
            "module_size": getattr(p, "module_size", None),
            "module_path": getattr(p, "module_path", None),
        }
        fingerprint = p.fingerprint()
        country_roots, country_meta = live.read_country_roots(p, max_regions=max_regions)
        lines = live.read_construction_lines(p, country_roots)
        topology = live.scan_cprovince_roots(p, max_regions=max_regions)
        accepted_rows, rejected_rows = stable_live_save_rows(
            lines,
            save_rows,
            topology,
            save_topology,
        )
        aggregates, row_summaries, scan_failures = scan_rows_for_roots(p, accepted_rows, topology)

    candidate_summary = summarize_candidates(
        aggregates,
        accepted_rows,
        min_rows=min_rows,
        min_unique_targets=min_unique_targets,
    )
    status = classify_status(candidate_summary, len(accepted_rows), min_rows)
    return {
        "schema": SCHEMA,
        "status": status,
        "duration_ms": round((time.time() - started) * 1000),
        "process": process_meta,
        "fingerprint": fingerprint,
        "build_profile": live.BUILD_PROFILE,
        "save": save_meta,
        "bounded_search": {
            "bases": [
                {
                    "name": "line",
                    "bytes": LINE_SCAN_BYTES,
                    "source": "current live construction line object",
                },
                {
                    "name": "sub",
                    "bytes": SUB_SCAN_BYTES,
                    "source": "current live construction building subobject",
                },
                {
                    "name": "template_def",
                    "bytes": TEMPLATE_DEF_SCAN_BYTES,
                    "source": "current live construction template definition pointer",
                },
                {
                    "name": "one_hop",
                    "bytes": ONE_HOP_SCAN_BYTES,
                    "source": (
                        "heap pointers found in line/sub/template_def bounded ranges, "
                        "capped per row"
                    ),
                    "max_bases_per_row": MAX_ONE_HOP_BASES_PER_ROW,
                },
            ],
            "value_interpretations": [
                "u16 target_state_id",
                "u32 target_state_id",
                "i32 target_state_id",
                "u64 low/high target_state_id",
                "state pointer",
                "u16/u32/i32/u64 exact save building.location",
                "province pointer for province building.location",
            ],
            "previously_rejected_candidates_excluded_from_promotion": sorted(
                REJECTED_CANDIDATE_KEYS
            ),
        },
        "live_queue": {
            "country_root_ok": country_meta.get("ok"),
            "country_root_count": country_meta.get("root_count"),
            "construction_lines": len(lines),
            "tag_db_id_count": len({line["tag_db_id"] for line in lines}),
            "template_id_count": len(
                {
                    line.get("template_id")
                    for line in lines
                    if line.get("template_id") is not None
                }
            ),
        },
        "topology": {
            "state_count": topology.get("state_count"),
            "province_count": topology.get("province_count"),
            "save_state_count": len(save_topology["state_ids"]),
            "save_province_state_count": len(save_topology["province_to_state"]),
            "static_province_state_count": len(save_topology["static_province_to_state"]),
            "static_province_state_path": save_topology["static_province_to_state_path"],
        },
        "accepted_rows": len(accepted_rows),
        "accepted_row_summary": row_summaries,
        "rejected_live_save_rows": rejected_rows[:64],
        "scan_failures": scan_failures[:64],
        "candidate_summary": candidate_summary,
        "result_detail": result_detail(status, candidate_summary),
    }


def open_target_process(pid: int | None) -> Any:
    if pid is not None:
        return live.winmem.Process(pid, "hoi4.exe")
    p = live.winmem.open_process("hoi4.exe")
    if p is None:
        raise SystemExit("hoi4.exe not running")
    return p


def read_save_rows(
    save_path: Path,
    rakaly_arg: str | None,
    out_dir: Path,
) -> tuple[list[dict[str, Any]], dict[str, Any], dict[str, Any]]:
    rakaly = ps.find_rakaly(rakaly_arg)
    text, was_melted = ps.melt_if_needed(save_path, rakaly, out_dir)
    rows, meta = save_parser.parse_save_construction_rows(text)
    save_topology = parse_save_state_topology(text)
    static_province_to_state = load_static_province_to_state()
    save_topology["static_province_to_state"] = static_province_to_state
    save_topology["static_province_to_state_path"] = (
        str(STATIC_PROVINCE_TO_STATE_PATH).replace("\\", "/")
        if STATIC_PROVINCE_TO_STATE_PATH.is_file()
        else None
    )
    line_ids: dict[int, int] = {}
    duplicate_line_ids = []
    for row in rows:
        line_id = row.get("line_id")
        if not isinstance(line_id, int):
            continue
        line_ids[line_id] = line_ids.get(line_id, 0) + 1
    for line_id, count in sorted(line_ids.items()):
        if count > 1:
            duplicate_line_ids.append(line_id)
    return rows, {
        "source_path": str(save_path).replace("\\", "/"),
        "source_filename": save_path.name,
        "was_melted": was_melted,
        "save_date": meta.get("date"),
        "save_version": meta.get("version"),
        "player": meta.get("player"),
        "construction_rows": len(rows),
        "duplicate_line_ids": duplicate_line_ids[:64],
        "state_count": len(save_topology["state_ids"]),
        "province_state_map_count": len(save_topology["province_to_state"]),
        "static_province_state_map_count": len(static_province_to_state),
        "static_province_state_map_path": save_topology["static_province_to_state_path"],
    }, save_topology


def load_static_province_to_state() -> dict[int, int]:
    if not STATIC_PROVINCE_TO_STATE_PATH.is_file():
        return {}
    raw = json.loads(STATIC_PROVINCE_TO_STATE_PATH.read_text(encoding="utf-8"))
    out = {}
    for key, value in raw.items():
        parsed_key = to_int(key)
        parsed_value = to_int(value)
        if parsed_key is not None and parsed_value is not None:
            out[parsed_key] = parsed_value
    return out


def parse_save_state_topology(text: str) -> dict[str, Any]:
    state_ids: set[int] = set()
    province_to_state: dict[int, int] = {}

    def parse_states(tz: ps.Tokenizer) -> None:
        for state_key, opener in ps.iter_block_entries(tz):
            if opener is not ps.LBRACE:
                continue
            keyed_state_id = to_int(state_key)
            parse_state_topology_block(tz, keyed_state_id, state_ids, province_to_state)

    tz = ps.Tokenizer(text)
    while True:
        tok = tz.next()
        if tok is None:
            break
        key = ps.atom_value(tok)
        if key is None:
            if tok is ps.LBRACE:
                ps._skip_block_body(tz)
            continue
        if tz.peek() is not ps.EQUALS:
            continue
        tz.next()
        val = tz.next()
        if val is ps.LBRACE:
            if key == "states":
                parse_states(tz)
            else:
                ps._skip_block_body(tz)
    return {
        "state_ids": state_ids,
        "province_to_state": province_to_state,
    }


def parse_state_topology_block(
    tz: ps.Tokenizer,
    keyed_state_id: int | None,
    state_ids: set[int],
    province_to_state: dict[int, int],
) -> None:
    state_id = keyed_state_id
    pending_provinces: list[int] = []
    for key, opener in ps.iter_block_entries(tz):
        if opener is ps.LBRACE:
            if key == "provinces":
                pending_provinces = read_int_list(tz)
            else:
                ps._skip_block_body(tz)
            continue
        if key == "id":
            parsed = to_int(ps.atom_value(opener))
            if parsed is not None:
                state_id = parsed
    if state_id is None or state_id <= 0:
        return
    state_ids.add(state_id)
    for province_id in pending_provinces:
        province_to_state.setdefault(province_id, state_id)


def read_int_list(tz: ps.Tokenizer) -> list[int]:
    out = []
    for key, opener in ps.iter_block_entries(tz):
        if opener is ps.LBRACE:
            ps._skip_block_body(tz)
            continue
        value = key if opener is None else ps.atom_value(opener)
        parsed = to_int(value)
        if parsed is not None:
            out.append(parsed)
    return out


def to_int(value: Any) -> int | None:
    try:
        return int(float(value))
    except (TypeError, ValueError):
        return None


def stable_live_save_rows(
    live_lines: list[dict[str, Any]],
    save_rows: list[dict[str, Any]],
    topology: dict[str, Any],
    save_topology: dict[str, Any],
) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    save_by_line: dict[int, list[dict[str, Any]]] = {}
    for row in save_rows:
        line_id = row.get("line_id")
        if isinstance(line_id, int):
            save_by_line.setdefault(line_id, []).append(row)

    live_seen: dict[int, int] = {}
    for line in live_lines:
        line_id = line.get("line_id")
        if isinstance(line_id, int):
            live_seen[line_id] = live_seen.get(line_id, 0) + 1

    accepted = []
    rejected = []
    for line in live_lines:
        line_id = line.get("line_id")
        if not isinstance(line_id, int):
            rejected.append({"reason": "live_line_missing_line_id", "line": public_live_line(line)})
            continue
        if live_seen.get(line_id, 0) != 1:
            rejected.append({"reason": "duplicate_live_line_id", "line": public_live_line(line)})
            continue
        candidates = save_by_line.get(line_id) or []
        if len(candidates) != 1:
            rejected.append(
                {
                    "reason": "missing_or_duplicate_save_line_id",
                    "save_match_count": len(candidates),
                    "line": public_live_line(line),
                }
            )
            continue
        save = candidates[0]
        reason = stable_row_rejection_reason(line, save)
        if reason:
            rejected.append(
                {
                    "reason": reason,
                    "line": public_live_line(line),
                    "save": save_parser.save_public(save),
                }
            )
            continue
        expected = expected_location_and_state(save, topology, save_topology)
        if expected.get("reason"):
            rejected.append(
                {
                    "reason": expected["reason"],
                    "line": public_live_line(line),
                    "save": save_parser.save_public(save),
                }
            )
            continue
        accepted.append(
            {
                "live": line,
                "save": save,
                **expected,
            }
        )
    return accepted, rejected


def stable_row_rejection_reason(line: dict[str, Any], save: dict[str, Any]) -> str | None:
    if save.get("line_type") != live.LINE_TYPE_CONSTRUCTION:
        return "save_line_type_not_construction"
    if save.get("cost_raw") != line.get("cost_raw"):
        return "cost_raw_mismatch"
    if not isinstance(save.get("location_state"), int) or save["location_state"] <= 0:
        return "save_missing_numeric_location"
    if not isinstance(save.get("template"), str):
        return "save_missing_template"
    return None


def expected_location_and_state(
    save: dict[str, Any],
    topology: dict[str, Any],
    save_topology: dict[str, Any],
) -> dict[str, Any]:
    template = save.get("template")
    location = save.get("location_state")
    if not isinstance(location, int):
        return {"reason": "save_location_not_numeric"}
    if template in STATE_LOCATION_TEMPLATES:
        return {
            "location_kind": "state",
            "expected_location_id": location,
            "expected_target_state_id": location,
            "expected_province_id": None,
        }
    if template in PROVINCE_LOCATION_TEMPLATES:
        state_id = save_topology["province_to_state"].get(location)
        source = "save_states"
        if state_id is None:
            province = (topology.get("province_by_id") or {}).get(str(location))
            state_id = int(province["state_id"]) if province else None
            source = "live_cprovince"
        if state_id is None:
            state_id = save_topology["static_province_to_state"].get(location)
            source = "static_map_pipeline_asset"
        if state_id is None:
            return {"reason": "province_location_not_in_save_or_live_topology"}
        return {
            "location_kind": "province",
            "expected_location_id": location,
            "expected_target_state_id": state_id,
            "expected_province_id": location,
            "province_to_state_source": source,
        }
    if (
        (
            location in save_topology["province_to_state"]
            or location in save_topology["static_province_to_state"]
        )
        and location not in save_topology["state_ids"]
    ):
        state_id = save_topology["province_to_state"].get(
            location,
            save_topology["static_province_to_state"].get(location),
        )
        return {
            "location_kind": "province",
            "expected_location_id": location,
            "expected_target_state_id": state_id,
            "expected_province_id": location,
            "template_semantics": "inferred_unambiguous_save_province_location",
        }
    if location in save_topology["state_ids"] and location not in save_topology["province_to_state"]:
        return {
            "location_kind": "state",
            "expected_location_id": location,
            "expected_target_state_id": location,
            "expected_province_id": None,
            "template_semantics": "inferred_unambiguous_save_state_location",
        }
    return {"reason": f"unknown_building_template_location_semantics:{template}"}


def scan_rows_for_roots(
    p: Any,
    accepted_rows: list[dict[str, Any]],
    topology: dict[str, Any],
) -> tuple[dict[str, dict[str, Any]], list[dict[str, Any]], list[dict[str, Any]]]:
    aggregates: dict[str, dict[str, Any]] = {}
    row_summaries = []
    failures = []
    state_ptr_by_id = {int(k): int(v, 16) for k, v in (topology.get("state_by_id") or {}).items()}
    province_ptr_by_id = {
        int(k): int(v["province_ptr"], 16)
        for k, v in (topology.get("province_by_id") or {}).items()
    }
    for row in accepted_rows:
        live_line = row["live"]
        row_key = row_identity(row)
        row_match_count = 0
        bases = bounded_bases(p, live_line, failures, row_key)
        expected_state_ptr = state_ptr_by_id.get(row["expected_target_state_id"])
        expected_province_ptr = (
            province_ptr_by_id.get(row["expected_province_id"])
            if row.get("expected_province_id")
            else None
        )
        for base in bases:
            raw = p.try_read(base["addr"], base["length"])
            if len(raw) != base["length"]:
                failures.append(
                    {
                        "row": row_key,
                        "reason": "base_read_short",
                        "base": base_public(base),
                        "bytes_read": len(raw),
                    }
                )
                continue
            matches = scan_base(
                raw,
                base,
                expected_state_id=row["expected_target_state_id"],
                expected_location_id=row["expected_location_id"],
                expected_state_ptr=expected_state_ptr,
                expected_province_ptr=expected_province_ptr,
            )
            row_match_count += len(matches)
            for match in matches:
                add_aggregate(aggregates, row, match)
        row_summaries.append(
            {
                **row_key,
                "save_template": row["save"].get("template"),
                "location_kind": row["location_kind"],
                "expected_location_id": row["expected_location_id"],
                "expected_target_state_id": row["expected_target_state_id"],
                "candidate_match_count": row_match_count,
                "line_ptr": live_line.get("line_ptr"),
                "sub_ptr": live_line.get("sub_ptr"),
                "template_def_ptr": live_line.get("template_def_ptr"),
            }
        )
    return aggregates, row_summaries, failures


def bounded_bases(
    p: Any,
    line: dict[str, Any],
    failures: list[dict[str, Any]],
    row_key: dict[str, Any],
) -> list[dict[str, Any]]:
    bases = []
    for name, ptr_key, length in (
        ("line", "line_ptr", LINE_SCAN_BYTES),
        ("sub", "sub_ptr", SUB_SCAN_BYTES),
        ("template_def", "template_def_ptr", TEMPLATE_DEF_SCAN_BYTES),
    ):
        ptr_raw = line.get(ptr_key)
        if not ptr_raw:
            continue
        addr = int(ptr_raw, 16)
        bases.append({"name": name, "addr": addr, "length": length, "direct": True})

    seen = {base["addr"] for base in bases}
    one_hop_count = 0
    for base in list(bases):
        raw = p.try_read(base["addr"], base["length"])
        if len(raw) != base["length"]:
            failures.append(
                {
                    "row": row_key,
                    "reason": "pointer_source_read_short",
                    "base": base_public(base),
                    "bytes_read": len(raw),
                }
            )
            continue
        for offset in range(0, len(raw) - 7, 8):
            ptr = struct.unpack_from("<Q", raw, offset)[0]
            if not live.is_heap(ptr) or ptr in seen:
                continue
            seen.add(ptr)
            bases.append(
                {
                    "name": f"{base['name']}+0x{offset:X}->deref",
                    "addr": ptr,
                    "length": ONE_HOP_SCAN_BYTES,
                    "direct": False,
                    "source_base": base["name"],
                    "source_offset": f"0x{offset:X}",
                }
            )
            one_hop_count += 1
            if one_hop_count >= MAX_ONE_HOP_BASES_PER_ROW:
                return bases
    return bases


def scan_base(
    raw: bytes,
    base: dict[str, Any],
    *,
    expected_state_id: int,
    expected_location_id: int,
    expected_state_ptr: int | None,
    expected_province_ptr: int | None,
) -> list[dict[str, Any]]:
    matches = []
    for offset in range(0, len(raw) - 1, 2):
        value = struct.unpack_from("<H", raw, offset)[0]
        matches.extend(value_matches(base, offset, "u16", value, expected_state_id, expected_location_id))
    for offset in range(0, len(raw) - 3, 4):
        uvalue = struct.unpack_from("<I", raw, offset)[0]
        ivalue = struct.unpack_from("<i", raw, offset)[0]
        matches.extend(
            value_matches(base, offset, "u32", uvalue, expected_state_id, expected_location_id)
        )
        matches.extend(
            value_matches(base, offset, "i32", ivalue, expected_state_id, expected_location_id)
        )
    for offset in range(0, len(raw) - 7, 8):
        value = struct.unpack_from("<Q", raw, offset)[0]
        low = value & 0xFFFFFFFF
        high = (value >> 32) & 0xFFFFFFFF
        matches.extend(
            value_matches(base, offset, "u64_low", low, expected_state_id, expected_location_id)
        )
        matches.extend(
            value_matches(base, offset, "u64_high", high, expected_state_id, expected_location_id)
        )
        if expected_state_ptr is not None and value == expected_state_ptr:
            matches.append(match(base, offset, "state_ptr", "target_state_ptr", value))
        if expected_province_ptr is not None and value == expected_province_ptr:
            matches.append(match(base, offset, "province_ptr", "exact_location_province_ptr", value))
    return matches


def value_matches(
    base: dict[str, Any],
    offset: int,
    access: str,
    value: int,
    expected_state_id: int,
    expected_location_id: int,
) -> list[dict[str, Any]]:
    out = []
    if value == expected_state_id:
        out.append(match(base, offset, access, "target_state_id", value))
    if value == expected_location_id:
        out.append(match(base, offset, access, "exact_location_id", value))
    return out


def match(
    base: dict[str, Any],
    offset: int,
    access: str,
    mode: str,
    value: int,
) -> dict[str, Any]:
    return {
        "key": f"{base['name']}+0x{offset:X}:{access}:{mode}",
        "base": base["name"],
        "offset": f"0x{offset:X}",
        "access": access,
        "mode": mode,
        "value": value if value < (1 << 32) else hex(value),
        "base_addr": hex(base["addr"]),
        "direct": base.get("direct", False),
    }


def add_aggregate(
    aggregates: dict[str, dict[str, Any]],
    row: dict[str, Any],
    found: dict[str, Any],
) -> None:
    item = aggregates.setdefault(
        found["key"],
        {
            "key": found["key"],
            "base": found["base"],
            "offset": found["offset"],
            "access": found["access"],
            "mode": found["mode"],
            "direct": found["direct"],
            "row_count": 0,
            "line_ids": [],
            "save_templates": [],
            "location_kinds": [],
            "expected_target_state_ids": [],
            "expected_location_ids": [],
            "samples": [],
            "previously_rejected": found["key"] in REJECTED_CANDIDATE_KEYS,
        },
    )
    item["row_count"] += 1
    append_unique(item["line_ids"], row["live"].get("line_id"), limit=32)
    append_unique(item["save_templates"], row["save"].get("template"), limit=16)
    append_unique(item["location_kinds"], row["location_kind"], limit=8)
    append_unique(item["expected_target_state_ids"], row["expected_target_state_id"], limit=32)
    append_unique(item["expected_location_ids"], row["expected_location_id"], limit=32)
    if len(item["samples"]) < 8:
        item["samples"].append(
            {
                **row_identity(row),
                "save_template": row["save"].get("template"),
                "location_kind": row["location_kind"],
                "expected_target_state_id": row["expected_target_state_id"],
                "expected_location_id": row["expected_location_id"],
                "base_addr": found["base_addr"],
                "value": found["value"],
            }
        )


def summarize_candidates(
    aggregates: dict[str, dict[str, Any]],
    rows: list[dict[str, Any]],
    *,
    min_rows: int,
    min_unique_targets: int,
) -> dict[str, Any]:
    row_count = len(rows)
    unique_targets = sorted({row["expected_target_state_id"] for row in rows})
    unique_locations = sorted({row["expected_location_id"] for row in rows})
    target_all = [
        item
        for item in aggregates.values()
        if item["mode"] in ("target_state_id", "target_state_ptr")
        and item["row_count"] == row_count
        and not item["previously_rejected"]
    ]
    exact_all = [
        item
        for item in aggregates.values()
        if item["mode"] in ("exact_location_id", "exact_location_province_ptr")
        and item["row_count"] == row_count
        and not item["previously_rejected"]
    ]
    target_partial = [
        item
        for item in aggregates.values()
        if item["mode"] in ("target_state_id", "target_state_ptr")
        and item["row_count"] >= min_rows
        and item["row_count"] < row_count
        and not item["previously_rejected"]
    ]
    target_all.sort(key=candidate_sort_key)
    exact_all.sort(key=candidate_sort_key)
    target_partial.sort(key=candidate_sort_key)
    return {
        "accepted_row_count": row_count,
        "minimum_rows": min_rows,
        "minimum_unique_targets": min_unique_targets,
        "unique_expected_target_state_ids": unique_targets,
        "unique_expected_location_ids": unique_locations,
        "aggregate_candidate_count": len(aggregates),
        "target_state_all_row_candidate_count": len(target_all),
        "exact_location_all_row_candidate_count": len(exact_all),
        "target_state_partial_candidate_count": len(target_partial),
        "target_state_all_row_candidates": target_all[:64],
        "exact_location_all_row_candidates": exact_all[:64],
        "target_state_partial_candidates": target_partial[:64],
        "rejected_candidate_hits": [
            item for item in aggregates.values() if item.get("previously_rejected")
        ][:32],
    }


def candidate_sort_key(item: dict[str, Any]) -> tuple[int, str, str, str]:
    direct_rank = 0 if item.get("direct") else 1
    access_rank = {
        "u32": 0,
        "i32": 1,
        "u16": 2,
        "state_ptr": 3,
        "province_ptr": 4,
        "u64_low": 5,
        "u64_high": 6,
    }.get(item.get("access"), 99)
    return (direct_rank, str(access_rank), str(item.get("base")), str(item.get("offset")))


def classify_status(summary: dict[str, Any], accepted_rows: int, min_rows: int) -> str:
    if accepted_rows < min_rows:
        return "blocked_no_stable_live_save_rows"
    if summary["target_state_all_row_candidate_count"] > 0:
        return "bounded_target_state_candidates_found"
    if summary["exact_location_all_row_candidate_count"] > 0:
        return "bounded_exact_location_candidates_found_target_state_root_absent"
    return "rejected_bounded_direct_and_onehop_candidate_set"


def result_detail(status: str, summary: dict[str, Any]) -> str:
    if status == "bounded_target_state_candidates_found":
        return (
            "At least one non-rejected bounded candidate matched the expected target state "
            "for every accepted stable live/save row. Candidate rows are not production "
            "registration until reviewed and promoted."
        )
    if status == "bounded_exact_location_candidates_found_target_state_root_absent":
        return (
            "No target-state candidate matched every row, but at least one exact save "
            "building.location candidate did. Province rows would still require "
            "province-to-state mapping."
        )
    if status == "rejected_bounded_direct_and_onehop_candidate_set":
        return (
            "No non-rejected direct line/sub/template_def or one-hop pointer candidate "
            "matched every accepted stable live/save row."
        )
    return "Not enough stable live/save rows survived to evaluate the bounded root set."


def append_unique(items: list[Any], value: Any, *, limit: int) -> None:
    if value is not None and value not in items and len(items) < limit:
        items.append(value)


def public_live_line(line: dict[str, Any]) -> dict[str, Any]:
    keys = [
        "tag",
        "tag_db_id",
        "line_id",
        "priority",
        "template_id",
        "active_factories",
        "cost_raw",
        "produced_raw",
        "line_ptr",
        "sub_ptr",
        "template_def_ptr",
    ]
    return {key: line.get(key) for key in keys}


def row_identity(row: dict[str, Any]) -> dict[str, Any]:
    line = row["live"]
    return {
        "tag_db_id": line.get("tag_db_id"),
        "line_id": line.get("line_id"),
        "priority": line.get("priority"),
        "template_id": line.get("template_id"),
        "cost_raw": line.get("cost_raw"),
    }


def base_public(base: dict[str, Any]) -> dict[str, Any]:
    return {
        "name": base.get("name"),
        "addr": hex(base.get("addr", 0)),
        "length": base.get("length"),
        "direct": base.get("direct"),
    }


if __name__ == "__main__":
    raise SystemExit(main())
