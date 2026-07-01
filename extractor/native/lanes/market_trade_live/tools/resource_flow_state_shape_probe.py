"""Live shape rejection for CCountry resource-flow vector candidates.

The broad vector scan can find compact-looking country vectors, but country
resource import/export replay needs per-resource flow rows. This proof samples
the current live state-resource table and rejects candidate country vectors that
cannot structurally carry per-resource produced/exported/to_export rows.
"""

from __future__ import annotations

import argparse
import json
import struct
import sys
import time
from collections import defaultdict
from datetime import datetime, timezone
from pathlib import Path

_HERE = Path(__file__).resolve()
_REPO = _HERE.parents[5]
sys.path.insert(0, str(_REPO / "extractor" / "memory_reader"))
sys.path.insert(0, str(_HERE))

import market_trade_live_compact as mt  # noqa: E402
import world_extract_live as world_live  # noqa: E402
from resource_flow_broad_probe import scan_country  # noqa: E402
from resource_flow_root_probe import country_maps_with_dynamic_fallback  # noqa: E402


HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000

STATE_VTABLE_RVA = 0x29206C0
STATE_OFF_SID = 0x58
STATE_OFF_DEF = 0x30
STATE_OFF_OWNER = 0xC8
STATE_DEF_OFF_RESOURCES_PTR = 0xF8
STATE_DEF_OFF_RESOURCES_SIZE = 0x100
STATE_DEF_OFF_RESOURCES_CAP = 0x104
STATE_RESOURCE_STRIDE = 0x10
STATE_RESOURCE_NO_ID = 0xFFFFFFFF
STATE_RESOURCE_MAX_ROWS = 16
STATE_RESOURCE_MAX_ABS_RAW = 100_000_000

CPROVINCE_VTABLE_RVA = 0x295AF68
CPROVINCE_OFF_ID = 0xA4
CPROVINCE_OFF_STATE = 0xC0

RESOURCE_FLOW_CANDIDATE_OFF = 0x1218
RESOURCE_FLOW_CANDIDATE_STRIDE = 24
RESOURCE_FLOW_CANDIDATE_AMOUNT_OFF = 16
RESOURCE_FLOW_CANDIDATE_AMOUNT_WIDTH = 8
RESOURCE_FLOW_CANDIDATE_RESOURCE_OFF = 8

SAMPLE_TAGS = {
    "GER",
    "ENG",
    "SOV",
    "USA",
    "JAP",
    "ITA",
    "FRA",
    "BRA",
    "ROM",
    "HUN",
    "TUR",
    "SWE",
    "MEX",
    "ARG",
    "CAN",
    "AST",
}


def hx(value: int | None) -> str | None:
    return "0x%X" % value if isinstance(value, int) else None


def read_u32(blob: bytes, off: int) -> int | None:
    if off < 0 or off + 4 > len(blob):
        return None
    return struct.unpack_from("<I", blob, off)[0]


def read_i32(blob: bytes, off: int) -> int | None:
    if off < 0 or off + 4 > len(blob):
        return None
    return struct.unpack_from("<i", blob, off)[0]


def read_u64(blob: bytes, off: int) -> int | None:
    if off < 0 or off + 8 > len(blob):
        return None
    return struct.unpack_from("<Q", blob, off)[0]


def is_heap(value: int | None) -> bool:
    return isinstance(value, int) and HEAP_LO <= value < HEAP_HI and value % 8 == 0


def try_u32(p: mt.winmem.Process, addr: int) -> int | None:
    try:
        return p.u32(addr)
    except OSError:
        return None


def try_u64(p: mt.winmem.Process, addr: int) -> int | None:
    try:
        return p.u64(addr)
    except OSError:
        return None


def state_resource_logical_rows(size: int, cap: int) -> int | None:
    if size == 0:
        return 0
    if size > STATE_RESOURCE_MAX_ROWS:
        return None
    logical = cap if cap != 0 and cap < size else size
    return logical if logical <= STATE_RESOURCE_MAX_ROWS else None


def scan_state_roots(p: mt.winmem.Process, max_regions: int | None) -> dict[int, int]:
    cprovince_vtable = p.module_base + CPROVINCE_VTABLE_RVA
    state_vtable = p.module_base + STATE_VTABLE_RVA
    needle = struct.pack("<Q", cprovince_vtable)
    state_by_id: dict[int, int] = {}
    region_count = 0
    for base, size in p.iter_regions(HEAP_LO, HEAP_HI):
        region_count += 1
        if max_regions is not None and region_count > max_regions:
            break
        pos = 0
        while pos < size:
            want = min(64 << 20, size - pos)
            blob = p.try_read(base + pos, want)
            if not blob:
                pos += max(want, 0x1000)
                continue
            search = 0
            while True:
                hit = blob.find(needle, search)
                if hit < 0:
                    break
                search = hit + 1
                addr = base + pos + hit
                if addr % 8 != 0:
                    continue
                province_id = try_u32(p, addr + CPROVINCE_OFF_ID)
                if province_id is None or province_id == 0 or province_id > 65535:
                    continue
                state_ptr = try_u64(p, addr + CPROVINCE_OFF_STATE)
                if not is_heap(state_ptr) or try_u64(p, state_ptr) != state_vtable:
                    continue
                state_id = try_u32(p, state_ptr + STATE_OFF_SID)
                if state_id is None or state_id == 0 or state_id > 65535:
                    continue
                state_by_id.setdefault(state_id, state_ptr)
            if pos + len(blob) >= size or len(blob) < want:
                pos += max(len(blob), 0x1000)
            else:
                pos += len(blob) - 7
    return state_by_id


def scan_owned_state_resources(
    p: mt.winmem.Process, state_by_id: dict[int, int], id_to_tag: dict[int, str]
) -> tuple[dict[int, dict], dict]:
    by_owner: dict[int, dict] = defaultdict(
        lambda: {"state_count": 0, "states_with_resources": 0, "resource_rows": 0, "resource_ids": set()}
    )
    stats = {
        "state_count": len(state_by_id),
        "invalid_resource_vectors": 0,
        "invalid_resource_rows": 0,
        "resource_rows": 0,
        "states_with_resources": 0,
    }
    for state_id, state_ptr in sorted(state_by_id.items()):
        owner = try_u32(p, state_ptr + STATE_OFF_OWNER) or 0
        if owner == 0 or owner not in id_to_tag:
            continue
        by_owner[owner]["state_count"] += 1
        state_def = try_u64(p, state_ptr + STATE_OFF_DEF)
        if not is_heap(state_def):
            continue
        header = p.try_read(state_def + STATE_DEF_OFF_RESOURCES_PTR, 16)
        if len(header) < 16:
            stats["invalid_resource_vectors"] += 1
            continue
        data = read_u64(header, 0)
        size = read_u32(header, 8)
        cap = read_u32(header, 12)
        if size is None or cap is None:
            stats["invalid_resource_vectors"] += 1
            continue
        logical = state_resource_logical_rows(size, cap)
        if logical is None:
            stats["invalid_resource_vectors"] += 1
            continue
        if logical == 0:
            continue
        if not is_heap(data):
            stats["invalid_resource_vectors"] += 1
            continue
        raw = p.try_read(data, logical * STATE_RESOURCE_STRIDE)
        if len(raw) != logical * STATE_RESOURCE_STRIDE:
            stats["invalid_resource_vectors"] += 1
            continue
        rows_for_state = 0
        for index in range(logical):
            base = index * STATE_RESOURCE_STRIDE
            amount_raw = read_i32(raw, base)
            resource_id = read_u32(raw, base + 8)
            if resource_id == STATE_RESOURCE_NO_ID:
                continue
            if amount_raw is None or resource_id is None or not (0 <= amount_raw <= STATE_RESOURCE_MAX_ABS_RAW):
                stats["invalid_resource_rows"] += 1
                continue
            rows_for_state += 1
            stats["resource_rows"] += 1
            by_owner[owner]["resource_rows"] += 1
            by_owner[owner]["resource_ids"].add(resource_id)
        if rows_for_state:
            stats["states_with_resources"] += 1
            by_owner[owner]["states_with_resources"] += 1
    normalized = {}
    for tag_id, row in by_owner.items():
        normalized[tag_id] = {
            "tag_db_id": tag_id,
            "tag": id_to_tag.get(tag_id),
            "state_count": row["state_count"],
            "states_with_resources": row["states_with_resources"],
            "resource_rows": row["resource_rows"],
            "resource_ids": sorted(row["resource_ids"]),
            "distinct_resource_id_count": len(row["resource_ids"]),
        }
    return normalized, stats


def read_candidate_rows(
    p: mt.winmem.Process, selected: list[tuple[int, str, int]]
) -> dict[int, dict]:
    out = {}
    for tag_id, tag, ptr in selected:
        hits = [
            row
            for row in scan_country(p, tag_id, tag, ptr)
            if row.get("country_offset") == RESOURCE_FLOW_CANDIDATE_OFF
            and row.get("stride") == RESOURCE_FLOW_CANDIDATE_STRIDE
            and row.get("amount_off") == RESOURCE_FLOW_CANDIDATE_AMOUNT_OFF
            and row.get("amount_width") == RESOURCE_FLOW_CANDIDATE_AMOUNT_WIDTH
            and row.get("resource_off") == RESOURCE_FLOW_CANDIDATE_RESOURCE_OFF
        ]
        if not hits:
            continue
        row = hits[0]
        out[tag_id] = {
            "tag_db_id": tag_id,
            "tag": tag,
            "country_ptr": row.get("country_ptr"),
            "country_offset_hex": row.get("country_offset_hex"),
            "stride": row.get("stride"),
            "resource_ids": row.get("resource_ids") or [],
            "distinct_resource_id_count": len(set(row.get("resource_ids") or [])),
            "rows_preview": row.get("rows_preview") or [],
            "nonzero_amounts": row.get("nonzero_amounts"),
        }
    return out


def classify(candidate_by_tag: dict[int, dict], state_by_owner: dict[int, dict]) -> dict:
    compared = []
    mismatches = []
    for tag_id, candidate in sorted(candidate_by_tag.items()):
        state = state_by_owner.get(tag_id)
        if not state or state.get("distinct_resource_id_count", 0) < 2:
            continue
        row = {
            "tag_db_id": tag_id,
            "tag": candidate["tag"],
            "candidate_distinct_resource_ids": candidate["distinct_resource_id_count"],
            "candidate_resource_ids": candidate["resource_ids"],
            "owned_state_distinct_resource_ids": state["distinct_resource_id_count"],
            "owned_state_resource_ids": state["resource_ids"],
            "owned_state_resource_rows": state["resource_rows"],
        }
        compared.append(row)
        if candidate["distinct_resource_id_count"] < state["distinct_resource_id_count"]:
            mismatches.append(row)
    if not candidate_by_tag:
        return {
            "accepted_for_emission": False,
            "status": "candidate_absent",
            "reason": "No CCountry+0x1218 compact-vector rows were observed in the selected live countries.",
            "compared_country_count": 0,
            "shape_mismatch_count": 0,
            "shape_mismatch_examples": [],
        }
    if mismatches:
        return {
            "accepted_for_emission": False,
            "status": "rejected_candidate_not_full_resource_flow_root",
            "reason": (
                "CCountry+0x1218 cannot be the per-country produced/exported/to_export "
                "resource-flow root: in the same live frame it carries fewer resource ids "
                "than the owning countries' live state-resource resource-id set."
            ),
            "compared_country_count": len(compared),
            "shape_mismatch_count": len(mismatches),
            "shape_mismatch_examples": mismatches[:16],
        }
    return {
        "accepted_for_emission": False,
        "status": "unaccepted_candidate_needs_flow_fixture",
        "reason": (
            "The candidate was not shape-rejected by owned state resources, but still lacks "
            "same-tick produced/exported/to_export/imported values and is not safe to emit."
        ),
        "compared_country_count": len(compared),
        "shape_mismatch_count": 0,
        "shape_mismatch_examples": [],
    }


def probe(args: argparse.Namespace) -> dict:
    pid = args.pid or mt.winmem.find_pid(mt.PROC_NAME)
    if not pid:
        raise SystemExit(f"{mt.PROC_NAME} not running")
    started = time.perf_counter()
    with mt.winmem.Process(pid, mt.PROC_NAME) as p:
        fp = p.fingerprint()
        sha = str(fp.get("sha256") or "").lower()
        if sha != mt.EXPECTED_SHA256 and not args.force:
            raise SystemExit(f"live hoi4.exe sha256 {sha} != expected {mt.EXPECTED_SHA256}")
        game_hour = mt.read_game_hour_raw(p)
        maps = country_maps_with_dynamic_fallback(p)
        selected = []
        for tag_id, tag in sorted(maps["id_to_tag"].items()):
            ptr = maps["id_to_country_dynamic"].get(tag_id)
            if not ptr:
                continue
            if tag in SAMPLE_TAGS or len(selected) < args.min_countries:
                selected.append((tag_id, tag, ptr))
            if len(selected) >= args.max_countries:
                break
        state_roots = scan_state_roots(p, args.max_regions)
        state_by_owner, state_stats = scan_owned_state_resources(
            p, state_roots, maps["id_to_tag"]
        )
        candidate_by_tag = read_candidate_rows(p, selected)
        classification = classify(candidate_by_tag, state_by_owner)
        proof = {
            "schema": "market_trade_live_resource_flow_state_shape_probe_v1",
            "utc": datetime.now(timezone.utc).isoformat(),
            "pid": pid,
            "module": {
                "path": fp.get("path"),
                "base": hx(p.module_base),
                "sha256": sha,
            },
            "game_time": {
                "game_hour_raw": game_hour,
                "game_date": world_live.decode_datehour(game_hour) if isinstance(game_hour, int) else None,
            },
            "candidate": {
                "country_offset": RESOURCE_FLOW_CANDIDATE_OFF,
                "country_offset_hex": hx(RESOURCE_FLOW_CANDIDATE_OFF),
                "stride": RESOURCE_FLOW_CANDIDATE_STRIDE,
                "amount_off": RESOURCE_FLOW_CANDIDATE_AMOUNT_OFF,
                "amount_width": RESOURCE_FLOW_CANDIDATE_AMOUNT_WIDTH,
                "resource_off": RESOURCE_FLOW_CANDIDATE_RESOURCE_OFF,
                "countries_with_candidate": len(candidate_by_tag),
            },
            "live_state_resource_roots": {
                "cprovince_vtable_rva": hx(CPROVINCE_VTABLE_RVA),
                "cprovince_state_ptr_offset": hx(CPROVINCE_OFF_STATE),
                "state_vtable_rva": hx(STATE_VTABLE_RVA),
                "state_id_offset": hx(STATE_OFF_SID),
                "state_owner_offset": hx(STATE_OFF_OWNER),
                "state_def_offset": hx(STATE_OFF_DEF),
                "resources_ptr_offset": hx(STATE_DEF_OFF_RESOURCES_PTR),
                "resources_size_offset": hx(STATE_DEF_OFF_RESOURCES_SIZE),
                "resources_cap_offset": hx(STATE_DEF_OFF_RESOURCES_CAP),
                "resource_stride": STATE_RESOURCE_STRIDE,
                **state_stats,
            },
            "selected_countries": [
                {"tag_db_id": tag_id, "tag": tag, "country_ptr": hx(ptr)}
                for tag_id, tag, ptr in selected[:64]
            ],
            "candidate_examples": list(candidate_by_tag.values())[:32],
            "state_resource_examples": [
                state_by_owner[tag_id]
                for tag_id in sorted(state_by_owner)
                if tag_id in candidate_by_tag
            ][:32],
            "classification": classification,
            "elapsed_ms": round((time.perf_counter() - started) * 1000, 3),
        }
    if args.out_json:
        out = Path(args.out_json)
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(json.dumps(proof, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return proof


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pid", type=int)
    parser.add_argument("--out-json", required=True)
    parser.add_argument("--max-countries", type=int, default=96)
    parser.add_argument("--min-countries", type=int, default=32)
    parser.add_argument("--max-regions", type=int)
    parser.add_argument("--force", action="store_true")
    args = parser.parse_args(argv)
    proof = probe(args)
    print(json.dumps(proof["classification"], indent=2, sort_keys=True))
    print(json.dumps(proof["live_state_resource_roots"], indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
