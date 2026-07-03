#!/usr/bin/env python3
"""Read-only 870ebf probe for per-division manpower/loss candidates.

This probe follows the validated current-build country-table division root and
compares finite candidate fields against a same-frame save oracle. It never
promotes offsets by itself; exact candidates are reported as narrowed evidence
for later live validation and lane gates.
"""

from __future__ import annotations

import argparse
from collections import Counter, defaultdict
from datetime import datetime, timezone
import json
from pathlib import Path
import struct
import sys
import time
from typing import Any, Callable


REPO_ROOT = Path(__file__).resolve().parents[3]
MEMORY_READER = REPO_ROOT / "extractor" / "memory_reader"
if str(MEMORY_READER) not in sys.path:
    sys.path.insert(0, str(MEMORY_READER))

import winmem  # type: ignore  # noqa: E402


EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
PROC_NAME = "hoi4.exe"
SCHEMA = "battle-plan.division_manpower_probe_870ebf.v1"

CGAMESTATE_RVA = 0x3316E90
COUNTRY_DATABASE_RVA = 0x33189C8
CGAMESTATE_COUNTRY_TABLE = 0x310
CGAMESTATE_COUNTRY_COUNT = 0x31C
COUNTRY_VTABLE_RVA = 0x27D0E90
COUNTRY_TAG_ECHO = 0x08
COUNTRY_DIVISION_WRAPPER_DATA = 0x290
COUNTRY_DIVISION_WRAPPER_CAPACITY = 0x298
COUNTRY_DIVISION_WRAPPER_COUNT = 0x29C
DIVISION_WRAPPER_VTABLE_RVA = 0x2943AD0
DIVISION_WRAPPER_TO_BODY = 0x10
DIVISION_VTABLE_RVA = 0x2943CB0

DIVISION_TYPE = 4713
DIVISION_ALT_UNIT_TYPE = 51
DIVISION_OFF_TYPE = 0x08
DIVISION_OFF_ID = 0x0C
DIVISION_OFF_LOCATION = 0x18
DIVISION_OFF_ARMY_PARENT = 0xB0
DIVISION_OFF_COMBAT_REF = 0x1A4
DIVISION_OFF_OWNER_TAG_DB_ID = 0x1C8
DIVISION_OFF_CONTROLLER_CANDIDATE = 0x1D0
DIVISION_OFF_MOVE_PROGRESS = 0x230
DIVISION_OFF_EQUIPMENT_BANK = 0x358
DIVISION_OFF_TEMPLATE = 0x3A8
DIVISION_OFF_STRENGTH = 0x410
DIVISION_OFF_ORG = 0x418
DIVISION_OFF_EXPERIENCE = 0x420
DIVISION_OFF_MAX_SUPPLY = 0x5F0
DIVISION_OFF_SUPPLY_GAIN = 0x5F8
DIVISION_OFF_CURRENT_SUPPLY = 0x600
DIVISION_OFF_FUEL = 0x610

HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000
MAX_COUNTRIES = 4096
MAX_DIVISIONS = 50000
MAX_WRAPPERS_PER_COUNTRY = 20000
MAX_BODY_SCAN = 0x780
MAX_SUBOBJECT_SCAN = 0x300
MAX_MANPOWER_VALUE = 1_000_000
MAX_LOSS_VALUE = 50_000_000

KNOWN_DIVISION_OFFSETS = {
    0x00: "vtable",
    DIVISION_OFF_TYPE: "division_type",
    DIVISION_OFF_ID: "division_id",
    DIVISION_OFF_LOCATION: "location_province",
    DIVISION_OFF_ARMY_PARENT: "army_parent_ptr",
    DIVISION_OFF_COMBAT_REF: "combat_ref_count",
    DIVISION_OFF_OWNER_TAG_DB_ID: "owner_tag_db_id",
    DIVISION_OFF_CONTROLLER_CANDIDATE: "controller_candidate",
    DIVISION_OFF_MOVE_PROGRESS: "movement_progress",
    DIVISION_OFF_EQUIPMENT_BANK: "equipment_bank_vector",
    DIVISION_OFF_TEMPLATE: "division_template_ptr",
    DIVISION_OFF_STRENGTH: "strength_fixed100k",
    DIVISION_OFF_ORG: "organisation_fixed100k",
    DIVISION_OFF_EXPERIENCE: "experience_fixed100k",
    DIVISION_OFF_MAX_SUPPLY: "max_supply_fixed100k",
    DIVISION_OFF_SUPPLY_GAIN: "supply_gain_fixed100k",
    DIVISION_OFF_CURRENT_SUPPLY: "current_supply_fixed100k",
    DIVISION_OFF_FUEL: "fuel_fixed100k",
}


def hx(value: int | None) -> str | None:
    return f"0x{value:X}" if isinstance(value, int) else None


def is_heap(value: int | None, align: int = 8) -> bool:
    return isinstance(value, int) and HEAP_LO <= value < HEAP_HI and value % align == 0


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


def scalar_from_blob(blob: bytes, offset: int, encoding: str) -> int | None:
    try:
        if encoding == "u32":
            return struct.unpack_from("<I", blob, offset)[0]
        if encoding == "i32":
            return struct.unpack_from("<i", blob, offset)[0]
        if encoding == "u64":
            value = struct.unpack_from("<Q", blob, offset)[0]
            return value if value <= MAX_LOSS_VALUE else None
        if encoding == "i64":
            value = struct.unpack_from("<q", blob, offset)[0]
            return value if -MAX_LOSS_VALUE <= value <= MAX_LOSS_VALUE else None
    except (struct.error, ValueError):
        return None
    return None


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


def load_tags(p: Any) -> dict[int, str]:
    country_database = safe_u64(p, p.module_base + COUNTRY_DATABASE_RVA)
    if not is_heap(country_database):
        return {}
    tag_table = safe_u64(p, country_database)
    tag_count = safe_u32(p, country_database + 0x08) or 0
    if not is_heap(tag_table):
        return {}
    tags: dict[int, str] = {}
    for tag_id in range(min(tag_count, MAX_COUNTRIES)):
        raw = read_exact(p, tag_table + tag_id * 0x20, 3)
        if not raw:
            continue
        try:
            tag = raw.decode("ascii")
        except UnicodeDecodeError:
            continue
        if tag.isalpha() and tag.isupper():
            tags[tag_id] = tag
    return tags


def enumerate_divisions(p: Any, tags: dict[int, str], sample_limit: int | None = None) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    module = p.module_base
    stats = Counter()
    game_state = safe_u64(p, module + CGAMESTATE_RVA)
    if not is_heap(game_state):
        raise RuntimeError(f"invalid CGameState pointer {hx(game_state)}")
    country_table = safe_u64(p, game_state + CGAMESTATE_COUNTRY_TABLE)
    country_count = safe_u32(p, game_state + CGAMESTATE_COUNTRY_COUNT)
    if not is_heap(country_table) or not isinstance(country_count, int):
        raise RuntimeError("invalid country table/count")
    if country_count <= 0 or country_count > MAX_COUNTRIES:
        raise RuntimeError(f"country_count outside bound: {country_count}")
    country_blob = p.try_read(country_table, country_count * 8)
    if len(country_blob) != country_count * 8:
        raise RuntimeError("short country table read")

    country_vtable = module + COUNTRY_VTABLE_RVA
    wrapper_vtable = module + DIVISION_WRAPPER_VTABLE_RVA
    division_vtable = module + DIVISION_VTABLE_RVA
    rows: list[dict[str, Any]] = []
    seen: set[tuple[int, int]] = set()
    for index in range(country_count):
        country_ptr = struct.unpack_from("<Q", country_blob, index * 8)[0]
        if not is_heap(country_ptr):
            continue
        if safe_u64(p, country_ptr) != country_vtable:
            stats["country_bad_vtable"] += 1
            continue
        tag_echo = safe_u32(p, country_ptr + COUNTRY_TAG_ECHO)
        if tag_echo != index:
            stats["country_bad_tag_echo"] += 1
            continue
        stats["countries_valid"] += 1
        begin = safe_u64(p, country_ptr + COUNTRY_DIVISION_WRAPPER_DATA)
        capacity = safe_u32(p, country_ptr + COUNTRY_DIVISION_WRAPPER_CAPACITY)
        count = safe_u32(p, country_ptr + COUNTRY_DIVISION_WRAPPER_COUNT)
        if not count:
            continue
        if not is_heap(begin) or not isinstance(capacity, int) or count > capacity or count > MAX_WRAPPERS_PER_COUNTRY:
            stats["division_wrapper_vector_rejected"] += 1
            continue
        raw = p.try_read(begin, count * 8)
        if len(raw) != count * 8:
            stats["division_wrapper_vector_short_read"] += 1
            continue
        for slot in range(count):
            wrapper = struct.unpack_from("<Q", raw, slot * 8)[0]
            stats["wrapper_rows_seen"] += 1
            if not is_heap(wrapper):
                stats["wrapper_nonheap"] += 1
                continue
            if safe_u64(p, wrapper) != wrapper_vtable:
                stats["wrapper_bad_vtable"] += 1
                continue
            body = wrapper + DIVISION_WRAPPER_TO_BODY
            if safe_u64(p, body) != division_vtable:
                stats["division_bad_vtable"] += 1
                continue
            div_type = safe_u32(p, body + DIVISION_OFF_TYPE)
            if div_type not in (DIVISION_TYPE, DIVISION_ALT_UNIT_TYPE):
                stats["division_bad_type"] += 1
                continue
            div_id = safe_u32(p, body + DIVISION_OFF_ID)
            owner_id = safe_u32(p, body + DIVISION_OFF_OWNER_TAG_DB_ID)
            if not isinstance(div_id, int) or not isinstance(owner_id, int):
                stats["division_bad_identity"] += 1
                continue
            key = (owner_id, div_id)
            if key in seen:
                stats["division_duplicate_owner_id"] += 1
                continue
            seen.add(key)
            owner_tag = tags.get(owner_id)
            row = {
                "owner_division_key": f"{owner_tag}|{div_id}:{DIVISION_TYPE}" if owner_tag else None,
                "division_key": f"{div_id}:{DIVISION_TYPE}",
                "div_id": div_id,
                "div_type": div_type,
                "owner_tag_db_id": owner_id,
                "owner": owner_tag,
                "country_slot": index,
                "wrapper_addr": wrapper,
                "body_addr": body,
                "location_province": safe_u32(p, body + DIVISION_OFF_LOCATION),
                "army_parent_ptr": safe_u64(p, body + DIVISION_OFF_ARMY_PARENT),
                "combat_ref_count": safe_u32(p, body + DIVISION_OFF_COMBAT_REF),
                "strength_raw": safe_u32(p, body + DIVISION_OFF_STRENGTH),
                "organisation_raw": safe_u32(p, body + DIVISION_OFF_ORG),
            }
            rows.append(row)
            stats["division_rows"] += 1
            if sample_limit is not None and len(rows) >= sample_limit:
                return rows, dict(stats)
            if len(rows) > MAX_DIVISIONS:
                raise RuntimeError("division count exceeded sanity bound")
    return rows, dict(stats)


def load_oracle(path: Path) -> dict[str, Any]:
    data = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(data, dict):
        raise ValueError("oracle top-level must be an object")
    return data


def oracle_by_owner_key(oracle: dict[str, Any]) -> dict[str, dict[str, Any]]:
    rows = oracle.get("by_owner_division_key")
    if isinstance(rows, dict):
        return {str(k): v for k, v in rows.items() if isinstance(v, dict)}
    out: dict[str, dict[str, Any]] = {}
    for row in oracle.get("rows", []):
        if isinstance(row, dict) and isinstance(row.get("owner_division_key"), str):
            out[row["owner_division_key"]] = row
    return out


def target_value(oracle_row: dict[str, Any], target: str) -> int | None:
    if target == "current_manpower":
        value = oracle_row.get("oracle_current_manpower")
    elif target == "cumulative_losses":
        value = oracle_row.get("oracle_cumulative_killed")
    else:
        value = None
    return value if isinstance(value, int) and value >= 0 else None


def target_bound(target: str) -> int:
    return MAX_MANPOWER_VALUE if target == "current_manpower" else MAX_LOSS_VALUE


def route_id(route: dict[str, Any]) -> str:
    ptr = route.get("pointer_offset")
    field = int(route["field_offset"])
    enc = str(route["encoding"])
    if ptr is None:
        return f"CDivision+0x{field:X}:{enc}"
    return f"CDivision+0x{int(ptr):X}->+0x{field:X}:{enc}"


def read_route_value(p: Any, body: int, route: dict[str, Any], body_blob: bytes | None = None) -> int | None:
    ptr = route.get("pointer_offset")
    field = int(route["field_offset"])
    encoding = str(route["encoding"])
    size = 8 if encoding in ("u64", "i64") else 4
    if ptr is None:
        if body_blob is not None and field + size <= len(body_blob):
            return scalar_from_blob(body_blob, field, encoding)
        data = read_exact(p, body + field, size)
        return scalar_from_blob(data or b"", 0, encoding) if data is not None else None
    pointer = safe_u64(p, body + int(ptr))
    if not is_heap(pointer):
        return None
    data = read_exact(p, pointer + field, size)
    return scalar_from_blob(data or b"", 0, encoding) if data is not None else None


def candidate_routes_for_body(body_scan: int) -> list[dict[str, Any]]:
    routes: list[dict[str, Any]] = []
    for encoding, width, step in (("u32", 4, 4), ("i32", 4, 4), ("u64", 8, 8), ("i64", 8, 8)):
        for offset in range(0, max(0, body_scan - width + 1), step):
            routes.append({"pointer_offset": None, "field_offset": offset, "encoding": encoding})
    return routes


def discover_pointer_offsets(
    p: Any,
    eligible: list[dict[str, Any]],
    body_blobs: dict[str, bytes],
    body_scan: int,
    min_rows: int,
) -> list[dict[str, Any]]:
    rows = []
    for offset in range(0, body_scan - 7, 8):
        heap_rows = 0
        vtables = Counter()
        examples = []
        for row in eligible:
            blob = body_blobs.get(row["owner_division_key"] or "")
            if not blob or offset + 8 > len(blob):
                continue
            pointer = struct.unpack_from("<Q", blob, offset)[0]
            if not is_heap(pointer):
                continue
            heap_rows += 1
            vt = safe_u64(p, pointer)
            if isinstance(vt, int):
                vtables[hx(vt - p.module_base) if p.in_module(vt) else hx(vt)] += 1
            if len(examples) < 4:
                examples.append({"division": row["owner_division_key"], "pointer": hx(pointer)})
        status = "accepted_for_subobject_scan" if heap_rows >= min_rows else "rejected_low_heap_pointer_coverage"
        rows.append(
            {
                "pointer_offset": f"+0x{offset:X}",
                "heap_rows": heap_rows,
                "eligible_rows": len(eligible),
                "coverage": round(heap_rows / max(1, len(eligible)), 6),
                "top_vtables_or_first_qword": dict(vtables.most_common(8)),
                "examples": examples,
                "status": status,
                "reason": None if status == "accepted_for_subobject_scan" else "not enough rows carry heap pointers at this body offset",
            }
        )
    return rows


def subobject_routes(pointer_offsets: list[dict[str, Any]], subobject_scan: int) -> list[dict[str, Any]]:
    routes: list[dict[str, Any]] = []
    accepted = [
        int(str(row["pointer_offset"])[3:], 16)
        for row in pointer_offsets
        if row.get("status") == "accepted_for_subobject_scan"
    ]
    for pointer_offset in accepted:
        for encoding, width, step in (("u32", 4, 4), ("i32", 4, 4), ("u64", 8, 8), ("i64", 8, 8)):
            for field_offset in range(0, max(0, subobject_scan - width + 1), step):
                routes.append(
                    {
                        "pointer_offset": pointer_offset,
                        "field_offset": field_offset,
                        "encoding": encoding,
                    }
                )
    return routes


def classify_candidate(
    *,
    route: dict[str, Any],
    target: str,
    eligible: int,
    exact: int,
    distinct_oracle: int,
    exact_divergent_current_need: int,
    exact_nonzero_loss: int,
    current_need_divergent_rows: int,
    nonzero_loss_rows: int,
    min_exact: int,
    min_fraction: float,
    min_distinct: int,
) -> tuple[str, str]:
    fraction = exact / max(1, eligible)
    rid = route_id(route)
    ptr = route.get("pointer_offset")
    field = int(route["field_offset"])
    if target == "current_manpower" and current_need_divergent_rows <= 0:
        return "fixture_blocked_no_current_need_divergence", (
            "save oracle has no rows where current manpower differs from need; "
            "template/manpower-need aliases cannot be rejected in this fixture"
        )
    if target == "cumulative_losses" and nonzero_loss_rows <= 0:
        return "fixture_blocked_no_nonzero_loss_oracle", (
            "save oracle has no nonzero killed rows; all-zero loss aliases cannot be rejected"
        )
    if ptr is None and field in KNOWN_DIVISION_OFFSETS and exact > 0:
        return "rejected_known_division_field_alias", (
            f"{rid} overlaps known {KNOWN_DIVISION_OFFSETS[field]} field; coincidence is not manpower/loss proof"
        )
    if ptr == DIVISION_OFF_TEMPLATE and exact > 0:
        return "rejected_template_route_alias", (
            "candidate is behind CDivision+0x3A8 template pointer; template requirement is not current manpower/loss proof"
        )
    if distinct_oracle < min_distinct:
        return "rejected_low_oracle_diversity", (
            f"only {distinct_oracle} distinct oracle values; too weak to prove this candidate"
        )
    if exact < min_exact or fraction < min_fraction:
        if exact == 0:
            return "rejected_no_exact_match", "0 exact matches against same-frame oracle"
        return "rejected_partial_match", (
            f"{exact}/{eligible} exact matches below threshold {min_exact} and fraction {min_fraction}"
        )
    if target == "current_manpower" and exact_divergent_current_need <= 0:
        return "rejected_current_need_alias_not_disproved", (
            "candidate did not exactly match any row where current manpower differs from manpower need"
        )
    if target == "cumulative_losses" and exact_nonzero_loss <= 0:
        return "rejected_zero_loss_alias_not_disproved", (
            "candidate did not exactly match any nonzero killed row"
        )
    if target == "current_manpower":
        return "narrowed_current_manpower_candidate", "exact same-frame oracle match; needs live repeat/gate before promotion"
    return "narrowed_cumulative_losses_candidate", "exact same-frame killed oracle match; needs two-poll combat delta before promotion"


def evaluate_routes(
    p: Any,
    live_rows: list[dict[str, Any]],
    oracle_map: dict[str, dict[str, Any]],
    routes: list[dict[str, Any]],
    body_blobs: dict[str, bytes],
    target: str,
    args: argparse.Namespace,
) -> tuple[list[dict[str, Any]], dict[str, dict[str, int]]]:
    eligible = []
    for row in live_rows:
        key = row.get("owner_division_key")
        if not isinstance(key, str) or key not in oracle_map:
            continue
        oracle_value = target_value(oracle_map[key], target)
        if oracle_value is None or oracle_value > target_bound(target):
            continue
        eligible.append((row, oracle_map[key], oracle_value))

    current_need_divergent_rows = sum(
        1
        for _row, oracle_row, value in eligible
        if isinstance(oracle_row.get("oracle_manpower_need"), int)
        and oracle_row["oracle_manpower_need"] != value
    )
    nonzero_loss_rows = sum(1 for _row, _oracle_row, value in eligible if value > 0)
    distinct_oracle_values = len({value for _row, _oracle_row, value in eligible})
    summaries: list[dict[str, Any]] = []
    values_for_delta: dict[str, dict[str, int]] = {}

    for route in routes:
        exact = 0
        tested = 0
        exact_divergent = 0
        exact_nonzero_loss = 0
        value_hist = Counter()
        samples = []
        row_values: dict[str, int] = {}
        for row, oracle_row, oracle_value in eligible:
            key = str(row["owner_division_key"])
            value = read_route_value(p, int(row["body_addr"]), route, body_blobs.get(key))
            if value is None or value < 0 or value > target_bound(target):
                continue
            tested += 1
            value_hist[value] += 1
            if value == oracle_value:
                exact += 1
                row_values[key] = value
                if (
                    target == "current_manpower"
                    and isinstance(oracle_row.get("oracle_manpower_need"), int)
                    and oracle_row["oracle_manpower_need"] != oracle_value
                ):
                    exact_divergent += 1
                if target == "cumulative_losses" and oracle_value > 0:
                    exact_nonzero_loss += 1
                if len(samples) < 8:
                    samples.append(
                        {
                            "owner_division_key": key,
                            "oracle_value": oracle_value,
                            "live_value": value,
                            "oracle_need": oracle_row.get("oracle_manpower_need"),
                            "body_addr": hx(row.get("body_addr")),
                        }
                    )
        status, reason = classify_candidate(
            route=route,
            target=target,
            eligible=len(eligible),
            exact=exact,
            distinct_oracle=distinct_oracle_values,
            exact_divergent_current_need=exact_divergent,
            exact_nonzero_loss=exact_nonzero_loss,
            current_need_divergent_rows=current_need_divergent_rows,
            nonzero_loss_rows=nonzero_loss_rows,
            min_exact=args.min_exact,
            min_fraction=args.min_fraction,
            min_distinct=args.min_distinct,
        )
        rid = route_id(route)
        summary = {
            "target": target,
            "route": route,
            "route_id": rid,
            "status": status,
            "reason": reason,
            "eligible_rows": len(eligible),
            "tested_rows": tested,
            "exact_rows": exact,
            "exact_fraction": round(exact / max(1, len(eligible)), 6),
            "distinct_oracle_values": distinct_oracle_values,
            "current_need_divergent_rows": current_need_divergent_rows,
            "exact_divergent_current_need_rows": exact_divergent,
            "nonzero_loss_rows": nonzero_loss_rows,
            "exact_nonzero_loss_rows": exact_nonzero_loss,
            "top_values": value_hist.most_common(8),
            "samples": samples,
        }
        summaries.append(summary)
        if status.startswith("narrowed_"):
            values_for_delta[rid] = row_values

    summaries.sort(
        key=lambda row: (
            not str(row["status"]).startswith("narrowed_"),
            -int(row["exact_rows"]),
            -float(row["exact_fraction"]),
            row["route_id"],
        )
    )
    return summaries, values_for_delta


def prepare_body_blobs(p: Any, rows: list[dict[str, Any]], scan_bytes: int) -> dict[str, bytes]:
    blobs: dict[str, bytes] = {}
    for row in rows:
        key = row.get("owner_division_key")
        if not isinstance(key, str):
            continue
        blob = p.try_read(int(row["body_addr"]), scan_bytes)
        if len(blob) >= scan_bytes:
            blobs[key] = blob
    return blobs


def compact_live_rows(rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    out = []
    for row in rows:
        out.append(
            {
                "owner_division_key": row.get("owner_division_key"),
                "division_key": row.get("division_key"),
                "div_id": row.get("div_id"),
                "div_type": row.get("div_type"),
                "owner": row.get("owner"),
                "owner_tag_db_id": row.get("owner_tag_db_id"),
                "body_addr": hx(row.get("body_addr")),
                "location_province": row.get("location_province"),
                "combat_ref_count": row.get("combat_ref_count"),
                "strength_raw": row.get("strength_raw"),
                "organisation_raw": row.get("organisation_raw"),
            }
        )
    return out


def build_probe_report(args: argparse.Namespace) -> dict[str, Any]:
    oracle = load_oracle(args.oracle)
    oracle_map = oracle_by_owner_key(oracle)
    p = winmem.Process(args.pid, PROC_NAME) if args.pid else winmem.open_process(PROC_NAME)
    if p is None:
        raise RuntimeError("hoi4.exe not running")
    with p:
        started = time.perf_counter()
        info = process_info(p)
        report: dict[str, Any] = {
            "schema": SCHEMA,
            "probe": "probe_870ebf_division_manpower",
            "snapshot_label": args.snapshot_label,
            "generated_utc": datetime.now(timezone.utc).isoformat(timespec="seconds"),
            "expected_sha256": EXPECTED_SHA256,
            "process": info,
            "build_gate_ok": info.get("sha256") == EXPECTED_SHA256,
            "fail_closed": info.get("sha256") != EXPECTED_SHA256,
            "oracle": {
                "path": str(args.oracle),
                "save_date": oracle.get("source", {}).get("save_date"),
                "stats": oracle.get("stats", {}),
            },
        }
        if not report["build_gate_ok"]:
            report["status"] = "build_gate_failed"
            return report
        tags = load_tags(p)
        rows, enum_stats = enumerate_divisions(p, tags, sample_limit=args.sample_limit)
        body_scan = min(max(0x100, args.body_scan_bytes), MAX_BODY_SCAN)
        subobject_scan = min(max(0x40, args.subobject_scan_bytes), MAX_SUBOBJECT_SCAN)
        body_blobs = prepare_body_blobs(p, rows, body_scan)
        eligible_rows = [
            row
            for row in rows
            if isinstance(row.get("owner_division_key"), str)
            and row["owner_division_key"] in oracle_map
            and row["owner_division_key"] in body_blobs
        ]
        min_pointer_rows = max(args.min_pointer_rows, int(len(eligible_rows) * args.min_pointer_fraction))
        pointer_offsets = discover_pointer_offsets(p, eligible_rows, body_blobs, body_scan, min_pointer_rows)
        routes = candidate_routes_for_body(body_scan) + subobject_routes(pointer_offsets, subobject_scan)
        current_candidates, current_delta_values = evaluate_routes(
            p, rows, oracle_map, routes, body_blobs, "current_manpower", args
        )
        loss_candidates, loss_delta_values = evaluate_routes(
            p, rows, oracle_map, routes, body_blobs, "cumulative_losses", args
        )
        route_values_for_delta: dict[str, dict[str, int]] = {}
        route_values_for_delta.update(current_delta_values)
        route_values_for_delta.update(loss_delta_values)
        candidate_routes_for_delta = [
            row
            for row in current_candidates + loss_candidates
            if str(row.get("status", "")).startswith("narrowed_")
        ][: args.delta_route_limit]
        if not candidate_routes_for_delta:
            candidate_routes_for_delta = [
                row
                for row in current_candidates + loss_candidates
                if int(row.get("exact_rows") or 0) > 0
            ][: args.delta_route_limit]
            for row in candidate_routes_for_delta:
                rid = row["route_id"]
                if rid not in route_values_for_delta:
                    route_values_for_delta[rid] = {}

        report.update(
            {
                "status": "probe_completed_candidates_only",
                "elapsed_ms": round((time.perf_counter() - started) * 1000, 3),
                "tag_table": {
                    "tag_count_read": len(tags),
                    "samples": [{"tag_db_id": key, "tag": value} for key, value in list(sorted(tags.items()))[:16]],
                },
                "validated_roots": {
                    "CGameState": f"module+{hx(CGAMESTATE_RVA)}",
                    "country_table": f"CGameState+{hx(CGAMESTATE_COUNTRY_TABLE)} count CGameState+{hx(CGAMESTATE_COUNTRY_COUNT)}",
                    "CCountry_vtable": f"module+{hx(COUNTRY_VTABLE_RVA)}",
                    "division_wrapper_vector": f"CCountry+{hx(COUNTRY_DIVISION_WRAPPER_DATA)}/{hx(COUNTRY_DIVISION_WRAPPER_CAPACITY)}/{hx(COUNTRY_DIVISION_WRAPPER_COUNT)}",
                    "division_wrapper_vtable": f"module+{hx(DIVISION_WRAPPER_VTABLE_RVA)}",
                    "CDivision_body": f"wrapper+{hx(DIVISION_WRAPPER_TO_BODY)} vtable module+{hx(DIVISION_VTABLE_RVA)}",
                },
                "enumeration_stats": enum_stats,
                "live_rows": compact_live_rows(rows[: args.output_live_row_limit]),
                "eligible_live_oracle_rows": len(eligible_rows),
                "scan_contract": {
                    "body_scan_bytes": body_scan,
                    "subobject_scan_bytes": subobject_scan,
                    "candidate_route_count": len(routes),
                    "min_exact": args.min_exact,
                    "min_fraction": args.min_fraction,
                    "min_distinct": args.min_distinct,
                    "min_pointer_rows": min_pointer_rows,
                    "promotion_policy": "No route is promoted by this probe. Narrowed routes require live repeat/delta evidence and gap_contract_gate before shared integration.",
                },
                "pointer_offset_census": pointer_offsets,
                "current_manpower_candidates": current_candidates,
                "cumulative_loss_candidates": loss_candidates,
                "candidate_routes_for_delta": [
                    {
                        "target": row["target"],
                        "route_id": row["route_id"],
                        "route": row["route"],
                        "status": row["status"],
                        "exact_rows": row["exact_rows"],
                        "exact_fraction": row["exact_fraction"],
                    }
                    for row in candidate_routes_for_delta
                ],
                "route_row_values_for_delta": {
                    rid: values for rid, values in route_values_for_delta.items() if values
                },
            }
        )
        return report


def route_lookup(report: dict[str, Any]) -> dict[str, dict[str, Any]]:
    routes = {}
    for row in report.get("candidate_routes_for_delta", []):
        if isinstance(row, dict) and isinstance(row.get("route_id"), str) and isinstance(row.get("route"), dict):
            routes[row["route_id"]] = row
    return routes


def compare_against_before(args: argparse.Namespace) -> dict[str, Any]:
    before = json.loads(args.compare_before.read_text(encoding="utf-8"))
    current = build_probe_report(args)
    if not current.get("build_gate_ok"):
        current["compare_status"] = "build_gate_failed"
        return current
    route_rows = before.get("route_row_values_for_delta", {})
    before_routes = route_lookup(before)
    current_rows = {
        row.get("owner_division_key"): row
        for row in current.get("live_rows", [])
        if isinstance(row, dict) and isinstance(row.get("owner_division_key"), str)
    }
    p = winmem.Process(args.pid, PROC_NAME) if args.pid else winmem.open_process(PROC_NAME)
    if p is None:
        raise RuntimeError("hoi4.exe not running")
    with p:
        tags = load_tags(p)
        full_rows, _enum_stats = enumerate_divisions(p, tags, sample_limit=args.sample_limit)
        body_blobs = prepare_body_blobs(p, full_rows, min(max(0x100, args.body_scan_bytes), MAX_BODY_SCAN))
        full_by_key = {
            row["owner_division_key"]: row
            for row in full_rows
            if isinstance(row.get("owner_division_key"), str)
        }
        comparisons = []
        for rid, values in route_rows.items():
            route_meta = before_routes.get(rid, {})
            route = route_meta.get("route")
            if not isinstance(route, dict) or not isinstance(values, dict):
                continue
            deltas = []
            positive = negative = changed = combat_after = combat_changed = 0
            for key, before_value in values.items():
                row = full_by_key.get(key)
                if row is None:
                    continue
                after_value = read_route_value(p, int(row["body_addr"]), route, body_blobs.get(key))
                if not isinstance(after_value, int) or not isinstance(before_value, int):
                    continue
                delta = after_value - before_value
                if delta != 0:
                    changed += 1
                if delta > 0:
                    positive += 1
                elif delta < 0:
                    negative += 1
                if int(row.get("combat_ref_count") or 0) > 0:
                    combat_after += 1
                    if delta != 0:
                        combat_changed += 1
                if len(deltas) < 24 and delta != 0:
                    deltas.append(
                        {
                            "owner_division_key": key,
                            "before": before_value,
                            "after": after_value,
                            "delta": delta,
                            "combat_ref_count_after": row.get("combat_ref_count"),
                        }
                    )
            target = route_meta.get("target")
            if target == "current_manpower":
                verdict = "supports_current_manpower_delta" if negative > 0 and positive == 0 else "delta_not_disambiguating"
            elif target == "cumulative_losses":
                verdict = "supports_cumulative_losses_delta" if positive > 0 and negative == 0 else "delta_not_disambiguating"
            else:
                verdict = "unknown_target"
            comparisons.append(
                {
                    "route_id": rid,
                    "target": target,
                    "before_status": route_meta.get("status"),
                    "compared_rows": len(values),
                    "changed_rows": changed,
                    "positive_delta_rows": positive,
                    "negative_delta_rows": negative,
                    "combat_rows_after": combat_after,
                    "combat_changed_rows_after": combat_changed,
                    "delta_samples": deltas,
                    "verdict": verdict,
                    "success_note": (
                        "Use only as disambiguation evidence. A candidate still needs exact same-frame oracle evidence and lane gate acceptance."
                    ),
                }
            )
    current["compare_before"] = {
        "path": str(args.compare_before),
        "snapshot_label": before.get("snapshot_label"),
        "generated_utc": before.get("generated_utc"),
    }
    current["delta_comparisons"] = comparisons
    current["compare_status"] = "delta_compare_completed"
    current_rows.clear()
    return current


def write_report(path: Path, report: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pid", type=int, default=None)
    parser.add_argument("--oracle", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--snapshot-label", default="single_live_snapshot")
    parser.add_argument("--compare-before", type=Path, default=None)
    parser.add_argument("--sample-limit", type=int, default=None)
    parser.add_argument("--body-scan-bytes", type=int, default=0x700)
    parser.add_argument("--subobject-scan-bytes", type=int, default=0x240)
    parser.add_argument("--min-pointer-rows", type=int, default=16)
    parser.add_argument("--min-pointer-fraction", type=float, default=0.05)
    parser.add_argument("--min-exact", type=int, default=24)
    parser.add_argument("--min-fraction", type=float, default=0.95)
    parser.add_argument("--min-distinct", type=int, default=4)
    parser.add_argument("--delta-route-limit", type=int, default=16)
    parser.add_argument("--output-live-row-limit", type=int, default=80)
    args = parser.parse_args(argv)

    if args.compare_before:
        report = compare_against_before(args)
    else:
        report = build_probe_report(args)
    write_report(args.out, report)
    view = {
        "status": report.get("status"),
        "compare_status": report.get("compare_status"),
        "out": str(args.out),
        "build_gate_ok": report.get("build_gate_ok"),
        "division_rows": report.get("enumeration_stats", {}).get("division_rows"),
        "eligible_live_oracle_rows": report.get("eligible_live_oracle_rows"),
        "current_top": [
            {k: row.get(k) for k in ("route_id", "status", "exact_rows", "exact_fraction")}
            for row in report.get("current_manpower_candidates", [])[:5]
        ],
        "loss_top": [
            {k: row.get(k) for k in ("route_id", "status", "exact_rows", "exact_fraction")}
            for row in report.get("cumulative_loss_candidates", [])[:5]
        ],
    }
    print(json.dumps(view, indent=2, sort_keys=True))
    return 0 if report.get("build_gate_ok") else 2


if __name__ == "__main__":
    raise SystemExit(main())
