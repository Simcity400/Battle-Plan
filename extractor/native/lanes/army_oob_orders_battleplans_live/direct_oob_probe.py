"""Current-build lane-local direct land-OOB root probe.

This is discovery/proof code only. It reads hoi4.exe through PROCESS_VM_READ and
does not write, inject, or suspend. It deliberately lives outside shared
collector/recorder entrypoints.
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass, asdict
from datetime import datetime, timezone
import json
from pathlib import Path
import struct
import sys
import time
from typing import Any, Iterable


REPO = Path(__file__).resolve().parents[4]
MEMORY_READER = REPO / "extractor" / "memory_reader"
for path in (str(REPO), str(MEMORY_READER)):
    if path not in sys.path:
        sys.path.insert(0, path)

import fastscan  # type: ignore  # noqa: E402
import winmem  # type: ignore  # noqa: E402


EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
PROC_NAME = "hoi4.exe"

HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000
MAX_OOB_ID = 4_000_000
MAX_CLASS_ROWS = 50_000
MAX_ARMY_ORDERS_PER_ARMY = 64
MAX_ORDER_STATE_ROWS = 5_000

ARMY_VTABLE_RVA = 0x293BC60
AG_VTABLE_RVA = 0x293BCF8
THEATRE_VTABLE_RVA = 0x295EE18
DIVISION_VTABLE_RVA = 0x2943CB0
ORDER_VTABLE_RVA = 0x296FAE8

ARMY_TYPE = 53
THEATRE_TYPE = 67
DIVISION_TYPE = 4713
DIVISION_ALT_TYPE = 51

OFF_HANDLE = 0x08
DIVISION_OFF_ID_TYPE = 0x08
DIVISION_OFF_ID_ID = 0x0C
DIVISION_OFF_ARMY_PARENT = 0xB0
DIVISION_OFF_OWNER_TAG_DB_ID = 0x1C8

ARMY_OFF_MEMBER_COUNT = 0x5C
ARMY_OFF_LEADER_PTR = 0x88
ARMY_OFF_ORDERS_VEC = 0x98
ARMY_OFF_AG_PARENT = 0x1B8

ORDER_OFF_TYPE = 0x30
ORDER_OFF_ARMY_BACKREF = 0x38
ORDER_OFF_TYPE5_STATES_BEGIN = 0xE0
ORDER_OFF_TYPE5_STATES_SIZE = 0xE8


@dataclass(frozen=True)
class ObjRoot:
    id: int
    addr: int


def heap_ptr(value: int | None, align: int = 8) -> bool:
    return value is not None and HEAP_LO <= value < HEAP_HI and value % align == 0


def u64_from(blob: bytes, offset: int) -> int:
    return struct.unpack_from("<Q", blob, offset)[0]


def enum_current_oob_roots(p: Any) -> tuple[dict[str, list[ObjRoot]], dict[str, Any]]:
    arena = fastscan.discover_arena(p.pid)
    if arena is None:
        raise RuntimeError("could not discover heap arena")
    regions = fastscan.enumerate_regions(p.handle, arena[0], arena[1])
    specs = {
        "army": (p.module_base + ARMY_VTABLE_RVA, ARMY_TYPE),
        "army_group": (p.module_base + AG_VTABLE_RVA, ARMY_TYPE),
        "theatre": (p.module_base + THEATRE_VTABLE_RVA, THEATRE_TYPE),
        "division": (p.module_base + DIVISION_VTABLE_RVA, None),
    }
    needles = {key: struct.pack("<Q", vt) for key, (vt, _type_tag) in specs.items()}
    roots: dict[str, dict[int, int]] = {key: {} for key in specs}
    stats = {
        "arena": [arena[0], arena[1]],
        "regions": len(regions),
        "bytes_scanned": sum(size for _base, size in regions),
        "raw_hits": {key: 0 for key in specs},
        "accepted_hits": {key: 0 for key in specs},
        "rejected_hits": {key: 0 for key in specs},
    }

    for base, size in regions:
        pos = 0
        chunk = 32 << 20
        while pos < size:
            want = min(chunk, size - pos)
            blob = p.try_read(base + pos, want)
            if not blob:
                pos += max(want, 0x1000)
                continue
            for key, needle in needles.items():
                cursor = 0
                while True:
                    hit = blob.find(needle, cursor)
                    if hit < 0:
                        break
                    cursor = hit + 1
                    addr = base + pos + hit
                    if addr % 8 != 0:
                        continue
                    stats["raw_hits"][key] += 1
                    object_id = validate_root_hit(p, key, addr)
                    if object_id is None:
                        stats["rejected_hits"][key] += 1
                        continue
                    roots[key][object_id] = addr
                    stats["accepted_hits"][key] += 1
                    if len(roots[key]) > MAX_CLASS_ROWS:
                        raise RuntimeError(f"{key} root count exceeded sanity bound")
            got = len(blob)
            if pos + got >= size or got < want:
                pos += max(got, 0x1000)
            else:
                pos += got - 8

    sorted_roots = {
        key: [ObjRoot(object_id, roots[key][object_id]) for object_id in sorted(roots[key])]
        for key in roots
    }
    return sorted_roots, stats


def validate_root_hit(p: Any, key: str, addr: int) -> int | None:
    try:
        if key == "division":
            unit_type = p.u32(addr + DIVISION_OFF_ID_TYPE)
            if unit_type not in (DIVISION_TYPE, DIVISION_ALT_TYPE):
                return None
            object_id = p.u32(addr + DIVISION_OFF_ID_ID)
        else:
            handle = p.u64(addr + OFF_HANDLE)
            expected_type = THEATRE_TYPE if key == "theatre" else ARMY_TYPE
            if (handle & 0xFFFF_FFFF) != expected_type:
                return None
            object_id = (handle >> 32) & 0xFFFF_FFFF
    except OSError:
        return None
    if object_id <= 0 or object_id > MAX_OOB_ID:
        return None
    return int(object_id)


def summarize_direct_land_oob(p: Any, roots: dict[str, list[ObjRoot]]) -> dict[str, Any]:
    army_by_ptr = {root.addr: root.id for root in roots["army"]}
    ag_by_ptr = {root.addr: root.id for root in roots["army_group"]}
    active_armies: dict[int, dict[str, Any]] = {}
    division_owner_votes: dict[int, dict[int, int]] = {}
    orphan_divisions = 0
    for division in roots["division"]:
        try:
            parent = p.u64(division.addr + DIVISION_OFF_ARMY_PARENT)
        except OSError:
            continue
        army_id = army_by_ptr.get(parent)
        if army_id is None:
            orphan_divisions += 1
            continue
        row = active_armies.setdefault(army_id, {"division_ids": [], "division_count": 0})
        row["division_ids"].append(division.id)
        row["division_count"] += 1
        try:
            owner = p.u32(division.addr + DIVISION_OFF_OWNER_TAG_DB_ID)
        except OSError:
            owner = 0
        if owner:
            votes = division_owner_votes.setdefault(army_id, {})
            votes[owner] = votes.get(owner, 0) + 1

    direct_armies: dict[int, dict[str, Any]] = {}
    armies_by_ag: dict[int, list[int]] = {}
    order_type_histogram: dict[str, int] = {}
    non_type5_orders = 0
    type5_exact_orders = 0
    type5_validation_failed = 0
    order_rows = 0
    for army in roots["army"]:
        parent_id = None
        parent_status = "unknown"
        try:
            parent_ptr = p.u64(army.addr + ARMY_OFF_AG_PARENT)
            if parent_ptr in ag_by_ptr:
                parent_id = ag_by_ptr[parent_ptr]
                parent_status = "exact"
                armies_by_ag.setdefault(parent_id, []).append(army.id)
            elif heap_ptr(parent_ptr):
                parent_status = "validation_failed"
            else:
                parent_status = "none"
        except OSError:
            parent_status = "validation_failed"
        try:
            engine_count = p.u32(army.addr + ARMY_OFF_MEMBER_COUNT)
        except OSError:
            engine_count = None
        live_count = active_armies.get(army.id, {}).get("division_count", 0)
        owner = owner_consensus(division_owner_votes.get(army.id))
        orders = read_army_orders(p, army.addr)
        order_rows += len(orders)
        for order in orders:
            order_type_histogram[str(order["order_type"])] = (
                order_type_histogram.get(str(order["order_type"]), 0) + 1
            )
            if order["order_type"] == 5:
                if order["target_status"] == "exact":
                    type5_exact_orders += 1
                elif order["target_status"] == "validation_failed":
                    type5_validation_failed += 1
            else:
                non_type5_orders += 1
        direct_armies[army.id] = {
            "id": army.id,
            "parent_army_group_id": parent_id,
            "parent_status": parent_status,
            "owner_tag_db_id": owner,
            "engine_member_count": engine_count,
            "live_division_member_count": live_count,
            "missing_member_count": (
                engine_count - live_count if engine_count is not None and engine_count >= live_count else None
            ),
            "has_live_division_parent": army.id in active_armies,
            "order_count": len(orders),
            "non_type5_order_count": sum(1 for order in orders if order["order_type"] != 5),
        }

    direct_army_ids = set(direct_armies)
    active_army_ids = set(active_armies)
    empty_or_inactive_ids = sorted(direct_army_ids - active_army_ids)

    army_groups = []
    for root in roots["army_group"]:
        members = sorted(set(armies_by_ag.get(root.id, [])))
        army_groups.append({"id": root.id, "member_army_ids": members, "member_count": len(members)})

    theatres = [{"id": root.id} for root in roots["theatre"]]

    return {
        "counts": {
            "direct_armies": len(direct_armies),
            "active_armies_from_division_parents": len(active_armies),
            "empty_or_inactive_direct_armies": len(empty_or_inactive_ids),
            "army_groups": len(army_groups),
            "theatres": len(theatres),
            "divisions": len(roots["division"]),
            "orphan_divisions_without_direct_army_parent": orphan_divisions,
            "army_order_rows": order_rows,
            "non_type5_order_rows": non_type5_orders,
            "type5_exact_order_rows": type5_exact_orders,
            "type5_validation_failed_rows": type5_validation_failed,
        },
        "order_type_histogram": order_type_histogram,
        "empty_or_inactive_army_ids_sample": empty_or_inactive_ids[:64],
        "direct_armies_sample": sorted(direct_armies.values(), key=lambda row: row["id"])[:64],
        "army_groups_sample": army_groups[:64],
        "theatres": theatres,
        "roots": {
            "armies": [{"id": root.id} for root in roots["army"]],
            "army_groups": [{"id": root.id} for root in roots["army_group"]],
            "theatres": theatres,
        },
    }


def owner_consensus(votes: dict[int, int] | None) -> int | None:
    if not votes:
        return None
    ordered = sorted(votes.items(), key=lambda item: (-item[1], item[0]))
    if len(ordered) == 1 or ordered[0][1] > ordered[1][1]:
        return ordered[0][0]
    return None


def read_army_orders(p: Any, army_addr: int) -> list[dict[str, Any]]:
    try:
        begin = p.u64(army_addr + ARMY_OFF_ORDERS_VEC)
    except OSError:
        return []
    if not heap_ptr(begin):
        return []
    out: list[dict[str, Any]] = []
    order_vtable = p.module_base + ORDER_VTABLE_RVA
    for slot in range(MAX_ARMY_ORDERS_PER_ARMY):
        try:
            order_ptr = p.u64(begin + slot * 8)
        except OSError:
            break
        if not heap_ptr(order_ptr):
            break
        order = read_order(p, army_addr, order_ptr, slot, order_vtable)
        if order is not None:
            out.append(order)
    return out


def read_order(p: Any, army_addr: int, order_ptr: int, slot: int, order_vtable: int) -> dict[str, Any] | None:
    try:
        if p.u64(order_ptr) != order_vtable:
            return None
        if p.u64(order_ptr + ORDER_OFF_ARMY_BACKREF) != army_addr:
            return None
        order_type = p.i32(order_ptr + ORDER_OFF_TYPE)
    except OSError:
        return None
    target_status = "unsupported"
    target_count = None
    if order_type == 5:
        try:
            begin = p.u64(order_ptr + ORDER_OFF_TYPE5_STATES_BEGIN)
            count = p.u32(order_ptr + ORDER_OFF_TYPE5_STATES_SIZE)
        except OSError:
            begin, count = 0, 0
        target_count = int(count)
        if count == 0:
            target_status = "exact"
        elif count > MAX_ORDER_STATE_ROWS or not heap_ptr(begin, 4):
            target_status = "validation_failed"
        else:
            raw = p.try_read(begin, count * 4)
            target_status = "exact" if len(raw) == count * 4 else "validation_failed"
    return {
        "slot": slot,
        "order_type": order_type,
        "target_status": target_status,
        "target_state_count": target_count,
    }


def run(pid: int | None, out_dir: Path) -> dict[str, Any]:
    if pid is None:
        pid = winmem.find_pid(PROC_NAME)
    if pid is None:
        raise RuntimeError("hoi4.exe not running")
    p = winmem.Process(pid, PROC_NAME)
    try:
        fingerprint = p.fingerprint()
        sha = (fingerprint.get("sha256") or "").lower()
        if sha != EXPECTED_SHA256:
            raise RuntimeError(f"build gate failed: {sha} != {EXPECTED_SHA256}")
        started = time.monotonic()
        roots, scan_stats = enum_current_oob_roots(p)
        summary = summarize_direct_land_oob(p, roots)
        elapsed_s = time.monotonic() - started
        payload = {
            "schema": "army_oob_orders_battleplans_direct_oob_probe_v1",
            "created_utc": datetime.now(timezone.utc).isoformat(),
            "pid": pid,
            "exe_sha256": sha,
            "module_base": p.module_base,
            "module_path": p.module_path,
            "elapsed_s": elapsed_s,
            "scan_stats": scan_stats,
            "offsets": {
                "army_vtable_rva": ARMY_VTABLE_RVA,
                "army_group_vtable_rva": AG_VTABLE_RVA,
                "theatre_vtable_rva": THEATRE_VTABLE_RVA,
                "division_vtable_rva": DIVISION_VTABLE_RVA,
                "order_vtable_rva": ORDER_VTABLE_RVA,
                "division_army_parent": DIVISION_OFF_ARMY_PARENT,
                "army_member_count": ARMY_OFF_MEMBER_COUNT,
                "army_orders_vector": ARMY_OFF_ORDERS_VEC,
                "army_group_parent": ARMY_OFF_AG_PARENT,
                "order_type": ORDER_OFF_TYPE,
                "order_army_backref": ORDER_OFF_ARMY_BACKREF,
                "order_type5_states_begin": ORDER_OFF_TYPE5_STATES_BEGIN,
                "order_type5_states_size": ORDER_OFF_TYPE5_STATES_SIZE,
            },
            "summary": summary,
            "production_position": (
                "direct vtable scan is validated discovery evidence and a cold-preflight candidate; "
                "it is not a hot-loop production path until moved into a bounded DLL-native root or "
                "cache-refresh contract"
            ),
        }
        out_dir.mkdir(parents=True, exist_ok=True)
        (out_dir / "direct_oob_probe.json").write_text(
            json.dumps(payload, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        return payload
    finally:
        p.close()


def compact_print(payload: dict[str, Any]) -> None:
    summary = payload["summary"]
    counts = summary["counts"]
    view = {
        "status": "PASS",
        "pid": payload["pid"],
        "elapsed_s": payload["elapsed_s"],
        "counts": counts,
        "order_type_histogram": summary["order_type_histogram"],
        "empty_or_inactive_army_ids_sample": summary["empty_or_inactive_army_ids_sample"][:16],
    }
    print(json.dumps(view, indent=2, sort_keys=True))


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--pid", type=int)
    ap.add_argument("--out-dir", type=Path, required=True)
    args = ap.parse_args()
    payload = run(args.pid, args.out_dir)
    compact_print(payload)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
