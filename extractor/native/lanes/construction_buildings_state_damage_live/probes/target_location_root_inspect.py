"""Inspect the target-location object found by the bounded root search."""

from __future__ import annotations

import argparse
import json
import os
import sys
from collections import Counter
from pathlib import Path
from typing import Any

PROBE_DIR = Path(__file__).resolve().parent
LANE_DIR = PROBE_DIR.parent
REPO_ROOT = LANE_DIR.parent.parent.parent
MEMORY_READER = REPO_ROOT / "extractor" / "memory_reader"
sys.path.insert(0, str(PROBE_DIR))
sys.path.insert(0, str(MEMORY_READER))
sys.path.insert(0, str(MEMORY_READER / "live_layers"))

import live_probe_870ebf as live  # type: ignore  # noqa: E402


SCHEMA = "construction_buildings_state_damage_live_target_location_root_inspect_v1"
SUB_TARGET_LOCATION_PTR = 0x1D8
TARGET_LOCATION_ID = 0x6C
TARGET_LOCATION_PTR = 0x150
PROVINCE_SECONDHOP_SCAN_BYTES = 0x300


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--bounded-proof", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--pid", type=int, default=None)
    args = parser.parse_args()

    proof_path = Path(args.bounded_proof)
    with proof_path.open(encoding="utf-8") as handle:
        bounded = json.load(handle)

    proof = inspect_root(bounded, proof_path, args.pid)
    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    with out_path.open("w", encoding="utf-8") as handle:
        json.dump(proof, handle, indent=2, sort_keys=True)
        handle.write("\n")
    print(
        json.dumps(
            {
                "out": str(out_path),
                "status": proof["status"],
                "rows": proof["summary"]["rows"],
                "location_id_matches": proof["summary"]["location_id_matches"],
                "target_state_derivable_rows": proof["summary"]["target_state_derivable_rows"],
                "pointer_kinds": proof["summary"]["ptr_0x150_kind_counts"],
            },
            sort_keys=True,
        )
    )
    return 0 if proof["status"].startswith("validated_") else 2


def inspect_root(bounded: dict[str, Any], proof_path: Path, pid: int | None) -> dict[str, Any]:
    p = open_target_process(pid)
    with p:
        rows = []
        for row in bounded.get("accepted_row_summary") or []:
            rows.append(inspect_row(p, row))
        summary = summarize(rows)
        province_secondhop = scan_province_secondhop(p, rows)
        status = (
            "validated_exact_location_root_and_target_state_derivation"
            if summary["rows"] > 0
            and summary["location_id_matches"] == summary["rows"]
            and summary["target_state_derivable_rows"] == summary["rows"]
            else "validated_exact_location_root_static_map_required_for_province_target_state"
            if summary["rows"] > 0
            and summary["location_id_matches"] == summary["rows"]
            and summary["location_kind_counts"].get("province", 0) > 0
            and province_secondhop["all_row_candidate_count"] == 0
            else "inconclusive_target_location_root_inspection"
        )
        return {
            "schema": SCHEMA,
            "status": status,
            "source_bounded_proof": str(proof_path).replace("\\", "/"),
            "process": {
                "requested_pid": pid,
                "pid": getattr(p, "pid", None),
                "module_base": hex(getattr(p, "module_base", 0) or 0),
                "module_path": getattr(p, "module_path", None),
            },
            "root": {
                "subobject_target_location_ptr_offset": hex(SUB_TARGET_LOCATION_PTR),
                "target_location_id_offset": hex(TARGET_LOCATION_ID),
                "target_location_ptr_offset": hex(TARGET_LOCATION_PTR),
                "interpretation": (
                    "subobject+0x1D8 points to a target-location object. "
                    "target+0x6C stores save building.location. For state "
                    "locations, target+0x150 points to the matching CState. "
                    "For province locations in this proof, target+0x150 does "
                    "not expose a CProvince/CState target-state path."
                ),
            },
            "summary": summary,
            "province_target_state_secondhop": province_secondhop,
            "rows": rows,
        }


def open_target_process(pid: int | None) -> Any:
    if pid is not None:
        return live.winmem.Process(pid, "hoi4.exe")
    p = live.winmem.open_process("hoi4.exe")
    if p is None:
        raise SystemExit("hoi4.exe not running")
    return p


def inspect_row(p: Any, row: dict[str, Any]) -> dict[str, Any]:
    out = {
        "line_id": row.get("line_id"),
        "tag_db_id": row.get("tag_db_id"),
        "priority": row.get("priority"),
        "template_id": row.get("template_id"),
        "save_template": row.get("save_template"),
        "location_kind": row.get("location_kind"),
        "expected_location_id": row.get("expected_location_id"),
        "expected_target_state_id": row.get("expected_target_state_id"),
        "sub_ptr": row.get("sub_ptr"),
    }
    sub_ptr_raw = row.get("sub_ptr")
    if not sub_ptr_raw:
        return {**out, "status": "missing_sub_ptr"}
    sub_ptr = int(sub_ptr_raw, 16)
    target_ptr = live.try_u64(p, sub_ptr + SUB_TARGET_LOCATION_PTR)
    if not live.is_heap(target_ptr):
        return {**out, "status": "missing_target_location_ptr"}
    location_id = live.try_u32(p, target_ptr + TARGET_LOCATION_ID)
    ptr_0x150 = live.try_u64(p, target_ptr + TARGET_LOCATION_PTR)
    pointer = classify_pointer(p, ptr_0x150)
    derived_target_state = derive_target_state(pointer)
    location_matches = location_id == row.get("expected_location_id")
    target_matches = derived_target_state == row.get("expected_target_state_id")
    return {
        **out,
        "status": "ok",
        "target_location_ptr": hex(target_ptr),
        "location_id_at_0x6C": location_id,
        "location_id_matches_expected": location_matches,
        "u64_at_0x68": format_u64(live.try_u64(p, target_ptr + 0x68)),
        "ptr_at_0x150": hex(ptr_0x150) if live.is_heap(ptr_0x150) else None,
        "ptr_at_0x150_detail": pointer,
        "derived_target_state_id": derived_target_state,
        "derived_target_state_matches_expected": target_matches,
    }


def classify_pointer(p: Any, ptr: int | None) -> dict[str, Any]:
    if not live.is_heap(ptr):
        return {"kind": "not_heap", "ptr": None}
    vtable = live.try_u64(p, ptr)
    state_vtable = p.module_base + live.STATE_VTABLE_RVA
    province_vtable = p.module_base + live.CPROVINCE_VTABLE_RVA
    if vtable == state_vtable:
        return {
            "kind": "CState",
            "ptr": hex(ptr),
            "state_id": live.try_u32(p, ptr + live.STATE_OFF_SID),
        }
    if vtable == province_vtable:
        state_ptr = live.try_u64(p, ptr + live.CPROVINCE_OFF_STATE)
        state_detail = classify_pointer(p, state_ptr)
        return {
            "kind": "CProvince",
            "ptr": hex(ptr),
            "province_id": live.try_u32(p, ptr + live.CPROVINCE_OFF_ID),
            "state_ptr": hex(state_ptr) if live.is_heap(state_ptr) else None,
            "state_detail": state_detail,
        }
    return {
        "kind": "unknown_heap",
        "ptr": hex(ptr),
        "vtable": hex(vtable) if vtable is not None else None,
    }


def derive_target_state(pointer: dict[str, Any]) -> int | None:
    if pointer.get("kind") == "CState":
        state_id = pointer.get("state_id")
        return state_id if isinstance(state_id, int) else None
    if pointer.get("kind") == "CProvince":
        state_detail = pointer.get("state_detail") or {}
        state_id = state_detail.get("state_id")
        return state_id if isinstance(state_id, int) else None
    return None


def summarize(rows: list[dict[str, Any]]) -> dict[str, Any]:
    pointer_kinds = Counter(
        ((row.get("ptr_at_0x150_detail") or {}).get("kind") or "missing")
        for row in rows
    )
    location_kinds = Counter(row.get("location_kind") for row in rows)
    template_counts = Counter(row.get("save_template") for row in rows)
    location_matches = sum(1 for row in rows if row.get("location_id_matches_expected"))
    target_state_derivable = sum(1 for row in rows if row.get("derived_target_state_matches_expected"))
    return {
        "rows": len(rows),
        "location_id_matches": location_matches,
        "target_state_derivable_rows": target_state_derivable,
        "ptr_0x150_kind_counts": dict(sorted(pointer_kinds.items())),
        "location_kind_counts": dict(sorted(location_kinds.items())),
        "template_counts": dict(sorted(template_counts.items())),
        "sample_failures": [
            row
            for row in rows
            if not row.get("location_id_matches_expected")
            or not row.get("derived_target_state_matches_expected")
        ][:16],
        "province_samples": [
            row for row in rows if row.get("location_kind") == "province"
        ][:16],
    }


def scan_province_secondhop(p: Any, rows: list[dict[str, Any]]) -> dict[str, Any]:
    province_rows = [row for row in rows if row.get("location_kind") == "province"]
    aggregates: dict[str, dict[str, Any]] = {}
    failures = []
    for row in province_rows:
        bases = []
        target_ptr_raw = row.get("target_location_ptr")
        ptr_150_raw = row.get("ptr_at_0x150")
        if target_ptr_raw:
            bases.append(("target_location", int(target_ptr_raw, 16)))
        if ptr_150_raw:
            bases.append(("target_location+0x150->deref", int(ptr_150_raw, 16)))
        for base_name, addr in bases:
            raw = p.try_read(addr, PROVINCE_SECONDHOP_SCAN_BYTES)
            if len(raw) != PROVINCE_SECONDHOP_SCAN_BYTES:
                failures.append(
                    {
                        "line_id": row.get("line_id"),
                        "base": base_name,
                        "addr": hex(addr),
                        "bytes_read": len(raw),
                    }
                )
                continue
            for found in scan_expected_target_state(
                p,
                raw,
                base_name,
                row.get("expected_target_state_id"),
            ):
                add_secondhop_aggregate(aggregates, row, found)
    all_row = [
        item
        for item in aggregates.values()
        if item["row_count"] == len(province_rows)
    ]
    partial = [
        item
        for item in aggregates.values()
        if item["row_count"] >= 4 and item["row_count"] < len(province_rows)
    ]
    all_row.sort(key=lambda item: (item["base"], item["offset"], item["access"], item["mode"]))
    partial.sort(key=lambda item: (-item["row_count"], item["base"], item["offset"]))
    return {
        "status": (
            "validated_province_target_state_secondhop_candidate"
            if all_row
            else "rejected_province_target_state_secondhop_candidate_set"
            if province_rows
            else "not_applicable_no_province_rows"
        ),
        "province_rows": len(province_rows),
        "scan_bytes_per_base": PROVINCE_SECONDHOP_SCAN_BYTES,
        "all_row_candidate_count": len(all_row),
        "partial_candidate_count": len(partial),
        "all_row_candidates": all_row[:32],
        "partial_candidates": partial[:32],
        "failures": failures[:32],
    }


def scan_expected_target_state(
    p: Any,
    raw: bytes,
    base_name: str,
    expected_state_id: Any,
) -> list[dict[str, Any]]:
    if not isinstance(expected_state_id, int):
        return []
    out = []
    for offset in range(0, len(raw) - 1, 2):
        value = int.from_bytes(raw[offset : offset + 2], "little", signed=False)
        if value == expected_state_id:
            out.append(secondhop_match(base_name, offset, "u16", "target_state_id", value))
    for offset in range(0, len(raw) - 3, 4):
        uvalue = int.from_bytes(raw[offset : offset + 4], "little", signed=False)
        ivalue = int.from_bytes(raw[offset : offset + 4], "little", signed=True)
        if uvalue == expected_state_id:
            out.append(secondhop_match(base_name, offset, "u32", "target_state_id", uvalue))
        if ivalue == expected_state_id:
            out.append(secondhop_match(base_name, offset, "i32", "target_state_id", ivalue))
    for offset in range(0, len(raw) - 7, 8):
        value = int.from_bytes(raw[offset : offset + 8], "little", signed=False)
        low = value & 0xFFFFFFFF
        high = (value >> 32) & 0xFFFFFFFF
        if low == expected_state_id:
            out.append(secondhop_match(base_name, offset, "u64_low", "target_state_id", low))
        if high == expected_state_id:
            out.append(secondhop_match(base_name, offset, "u64_high", "target_state_id", high))
        pointer = classify_pointer(p, value)
        derived = derive_target_state(pointer)
        if derived == expected_state_id:
            out.append(secondhop_match(base_name, offset, "ptr", "target_state_ptr", value))
    return out


def secondhop_match(
    base_name: str,
    offset: int,
    access: str,
    mode: str,
    value: int,
) -> dict[str, Any]:
    return {
        "key": f"{base_name}+0x{offset:X}:{access}:{mode}",
        "base": base_name,
        "offset": f"0x{offset:X}",
        "access": access,
        "mode": mode,
        "value": value if value < (1 << 32) else hex(value),
    }


def add_secondhop_aggregate(
    aggregates: dict[str, dict[str, Any]],
    row: dict[str, Any],
    found: dict[str, Any],
) -> None:
    item = aggregates.setdefault(
        found["key"],
        {
            **found,
            "row_count": 0,
            "line_ids": [],
            "templates": [],
            "expected_target_state_ids": [],
            "samples": [],
        },
    )
    item["row_count"] += 1
    append_unique(item["line_ids"], row.get("line_id"), limit=32)
    append_unique(item["templates"], row.get("save_template"), limit=16)
    append_unique(item["expected_target_state_ids"], row.get("expected_target_state_id"), limit=32)
    if len(item["samples"]) < 8:
        item["samples"].append(
            {
                "line_id": row.get("line_id"),
                "save_template": row.get("save_template"),
                "expected_location_id": row.get("expected_location_id"),
                "expected_target_state_id": row.get("expected_target_state_id"),
                "target_location_ptr": row.get("target_location_ptr"),
                "ptr_at_0x150": row.get("ptr_at_0x150"),
            }
        )


def append_unique(items: list[Any], value: Any, *, limit: int) -> None:
    if value is not None and value not in items and len(items) < limit:
        items.append(value)


def format_u64(value: int | None) -> str | None:
    return hex(value) if value is not None else None


if __name__ == "__main__":
    raise SystemExit(main())
