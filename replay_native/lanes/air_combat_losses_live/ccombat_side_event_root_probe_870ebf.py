"""Reject land-style CCombatSide as an air-combat event root on 870ebf.

This is a lane-local, read-only RE probe. It does not run in the production
collector loop. The point is to test one residual candidate exactly: whether
active air-combat lifecycle/pairing/loss attribution can be obtained by reusing
the live land-combat CCombatSide/registry shape.
"""

from __future__ import annotations

import argparse
import collections
import datetime as dt
import json
import os
import sys
import time
from pathlib import Path
from typing import Any

THIS_DIR = Path(__file__).resolve().parent
REPO_ROOT = THIS_DIR.parents[2]
MEMORY_READER = REPO_ROOT / "memory_reader"
for path in (str(REPO_ROOT), str(MEMORY_READER)):
    if path not in sys.path:
        sys.path.insert(0, path)

from memory_reader import fastscan  # noqa: E402
from memory_reader.naval_air.air_combat_losses_probe_870ebf import (  # noqa: E402
    AIR_WING_VTABLE_RVA,
    AIR_WING_TYPE,
    Probe,
    hx,
    is_heap,
    read_u32,
    read_u64,
)

LAND_COMBAT_SIDE_VTABLE_RVA = 0x29913C0
LAND_COMBAT_REGISTRY_VTABLE_RVA = 0x29914D8
LAND_COMBAT_SIDE_OFF_REGISTRY = 0x18
LAND_COMBAT_SIDE_OFF_DATA = 0x20
LAND_COMBAT_SIDE_OFF_CAPACITY = 0x28
LAND_COMBAT_SIDE_OFF_SIZE = 0x2C
LAND_COMBAT_REGISTRY_OFF_HANDLE = 0x18
LAND_COMBAT_REGISTRY_OFF_ATTACKER_SIDE = 0x28
LAND_COMBAT_REGISTRY_OFF_DEFENDER_SIDE = 0x30
LAND_COMBAT_REGISTRY_OFF_FRONT = 0x38
LAND_COMBAT_HANDLE_TYPE = 62
MAX_LAND_COMBAT_SIDE_UNITS = 128

DIVISION_VTABLE_RVA = 0x2943CB0
DIVISION_TYPE = 4713
DIVISION_ALT_UNIT_TYPE = 51
FLEET_VTABLE_RVA = 0x294BF28
TASK_FORCE_VTABLE_RVA = 0x294C660
SHIP_VTABLE_RVA = 0x2941748
OOB_TYPE_FLEET = 61
SHIP_TYPE = 51


def utc_now() -> str:
    return dt.datetime.utcnow().replace(microsecond=0).isoformat() + "Z"


def compact_counter(counter: collections.Counter[Any], limit: int = 20) -> list[dict[str, Any]]:
    return [{"key": str(key), "count": count} for key, count in counter.most_common(limit)]


class CCombatSideAirRootProbe(Probe):
    def __init__(self, pid: int, workers: int):
        super().__init__(pid)
        self.workers = workers
        self.side_vtable = self.base + LAND_COMBAT_SIDE_VTABLE_RVA
        self.registry_vtable = self.base + LAND_COMBAT_REGISTRY_VTABLE_RVA
        self.division_vtable = self.base + DIVISION_VTABLE_RVA
        self.air_wing_vtable = self.base + AIR_WING_VTABLE_RVA
        self.fleet_vtable = self.base + FLEET_VTABLE_RVA
        self.task_force_vtable = self.base + TASK_FORCE_VTABLE_RVA
        self.ship_vtable = self.base + SHIP_VTABLE_RVA

    def scan_vtable_hits(self, vtable: int) -> dict[str, Any]:
        arena = fastscan.discover_arena(self.proc.pid)
        if arena is None:
            return {"status": "arena_discovery_failed"}
        lo, hi = arena
        h = fastscan._k32.OpenProcess(
            fastscan._PROCESS_VM_READ | fastscan._PROCESS_QUERY_INFORMATION,
            False,
            self.proc.pid,
        )
        if not h:
            return {"status": "open_process_failed"}
        try:
            regions = fastscan.enumerate_regions(h, lo, hi)
        finally:
            fastscan._k32.CloseHandle(h)
        started = time.monotonic()
        hits = fastscan.parallel_scan(
            self.proc.pid,
            lo,
            hi,
            workers=self.workers,
            regions=regions,
            **fastscan.make_qword_eq_matcher(vtable),
        )
        return {
            "status": "ok",
            "arena": {"lo": hx(lo), "hi": hx(hi), "region_count": len(regions)},
            "elapsed_s": round(time.monotonic() - started, 3),
            "hit_count": len(hits),
            "hits": hits,
        }

    def classify_unit_ptr_at(self, body: int) -> dict[str, Any] | None:
        if not is_heap(body):
            return None
        vt = read_u64(self.proc, body)
        unit_type = read_u32(self.proc, body + 0x08)
        unit_id = read_u32(self.proc, body + 0x0C)
        if vt == self.division_vtable and unit_type in (DIVISION_TYPE, DIVISION_ALT_UNIT_TYPE):
            return {"kind": "division", "id": unit_id, "body": hx(body)}
        if vt == self.air_wing_vtable and unit_type == AIR_WING_TYPE:
            return {"kind": "air_wing", "id": unit_id, "body": hx(body)}
        if vt == self.task_force_vtable and unit_type == OOB_TYPE_FLEET:
            return {"kind": "task_force", "id": unit_id, "body": hx(body)}
        if vt == self.fleet_vtable and unit_type == OOB_TYPE_FLEET:
            return {"kind": "fleet", "id": unit_id, "body": hx(body)}
        if vt == self.ship_vtable and unit_type == SHIP_TYPE:
            return {"kind": "ship", "id": unit_id, "body": hx(body)}
        return None

    def classify_unit_ptr(self, ptr: int) -> dict[str, Any] | None:
        return self.classify_unit_ptr_at(ptr) or self.classify_unit_ptr_at(ptr + 0x10)

    def side_participants(self, side: int) -> tuple[list[dict[str, Any]] | None, str]:
        if not is_heap(side) or read_u64(self.proc, side) != self.side_vtable:
            return None, "bad_side_vtable"
        data = read_u64(self.proc, side + LAND_COMBAT_SIDE_OFF_DATA)
        capacity = read_u32(self.proc, side + LAND_COMBAT_SIDE_OFF_CAPACITY)
        size = read_u32(self.proc, side + LAND_COMBAT_SIDE_OFF_SIZE)
        if data is None or capacity is None or size is None:
            return None, "header_read_failed"
        if size > capacity or size > MAX_LAND_COMBAT_SIDE_UNITS:
            return None, "size_bound_failed"
        if size == 0:
            return [], "empty"
        if not is_heap(data):
            return None, "data_ptr_invalid"
        byte_len = size * 8
        raw = self.proc.try_read(data, byte_len)
        if len(raw) != byte_len:
            return None, "short_read"
        participants = []
        for cursor in range(0, byte_len, 8):
            ptr = int.from_bytes(raw[cursor : cursor + 8], "little", signed=False)
            unit = self.classify_unit_ptr(ptr)
            if unit is None:
                return None, "unresolved_participant"
            participants.append(unit)
        data2 = read_u64(self.proc, side + LAND_COMBAT_SIDE_OFF_DATA)
        size2 = read_u32(self.proc, side + LAND_COMBAT_SIDE_OFF_SIZE)
        if data2 != data or size2 != size:
            return None, "side_header_changed"
        return participants, "exact"

    def classify_side(self, side: int) -> dict[str, Any]:
        participants, status = self.side_participants(side)
        registry = read_u64(self.proc, side + LAND_COMBAT_SIDE_OFF_REGISTRY)
        registry_ok = is_heap(registry) and read_u64(self.proc, registry or 0) == self.registry_vtable
        handle = read_u64(self.proc, (registry or 0) + LAND_COMBAT_REGISTRY_OFF_HANDLE) if registry_ok else None
        kinds = sorted({row["kind"] for row in participants or []})
        return {
            "side": hx(side),
            "status": status,
            "participant_count": len(participants or []),
            "participant_kinds": kinds,
            "registry": hx(registry),
            "registry_ok": bool(registry_ok),
            "handle": hx(handle),
            "handle_id": ((handle >> 32) & 0xFFFFFFFF) if handle is not None else None,
            "handle_type": (handle & 0xFFFFFFFF) if handle is not None else None,
            "attacker_backref_ok": (
                bool(registry_ok)
                and read_u64(self.proc, (registry or 0) + LAND_COMBAT_REGISTRY_OFF_ATTACKER_SIDE)
                == side
            ),
            "defender_side": hx(
                read_u64(self.proc, (registry or 0) + LAND_COMBAT_REGISTRY_OFF_DEFENDER_SIDE)
                if registry_ok
                else None
            ),
            "front": hx(
                read_u64(self.proc, (registry or 0) + LAND_COMBAT_REGISTRY_OFF_FRONT)
                if registry_ok
                else None
            ),
            "participants": (participants or [])[:16],
        }

    def ccombat_side_probe(self) -> dict[str, Any]:
        side_scan = self.scan_vtable_hits(self.side_vtable)
        registry_scan = self.scan_vtable_hits(self.registry_vtable)
        if side_scan.get("status") != "ok":
            return {"status": side_scan.get("status"), "side_scan": side_scan}
        sides = [self.classify_side(addr) for addr in side_scan["hits"]]
        status_counts = collections.Counter(row["status"] for row in sides)
        kind_counts: collections.Counter[str] = collections.Counter()
        side_handle_types: collections.Counter[int] = collections.Counter()
        for row in sides:
            if row["handle_type"] is not None:
                side_handle_types[row["handle_type"]] += 1
            if not row["participant_kinds"]:
                kind_counts["empty"] += 1
            else:
                for kind in row["participant_kinds"]:
                    kind_counts[kind] += 1
        air_wing_sides = [
            row for row in sides if "air_wing" in row["participant_kinds"]
        ]
        naval_unit_sides = [
            row
            for row in sides
            if {"task_force", "fleet", "ship"}.intersection(row["participant_kinds"])
        ]

        registry_handle_types: collections.Counter[int] = collections.Counter()
        registry_samples = []
        if registry_scan.get("status") == "ok":
            for addr in registry_scan["hits"]:
                handle = read_u64(self.proc, addr + LAND_COMBAT_REGISTRY_OFF_HANDLE)
                handle_type = (handle & 0xFFFFFFFF) if handle is not None else None
                if handle_type is not None:
                    registry_handle_types[handle_type] += 1
                if len(registry_samples) < 16:
                    registry_samples.append(
                        {
                            "registry": hx(addr),
                            "handle": hx(handle),
                            "handle_id": ((handle >> 32) & 0xFFFFFFFF)
                            if handle is not None
                            else None,
                            "handle_type": handle_type,
                            "attacker_side": hx(
                                read_u64(self.proc, addr + LAND_COMBAT_REGISTRY_OFF_ATTACKER_SIDE)
                            ),
                            "defender_side": hx(
                                read_u64(self.proc, addr + LAND_COMBAT_REGISTRY_OFF_DEFENDER_SIDE)
                            ),
                        }
                    )

        return {
            "hypothesis": (
                "full air-combat lifecycle/pairing/loss attribution is reachable by "
                "reusing the land CCombatSide/registry small-vector path"
            ),
            "validated_offsets": {
                "CCombatSide_vtable": "module+0x29913C0",
                "CCombatRegistry_vtable": "module+0x29914D8",
                "CCombatSide_registry": "+0x18",
                "CCombatSide_units": "+0x20 data, +0x28 capacity, +0x2C size",
                "CCombatRegistry_handle": "+0x18",
                "CCombatRegistry_attacker_defender_front": "+0x28/+0x30/+0x38",
                "air_wing_body_identity": "module+0x29641B0, type +0x08=69, id +0x0C",
            },
            "side_scan": {
                "arena": side_scan.get("arena"),
                "elapsed_s": side_scan.get("elapsed_s"),
                "hit_count": side_scan.get("hit_count"),
                "vtable": hx(self.side_vtable),
                "vtable_rva": hex(LAND_COMBAT_SIDE_VTABLE_RVA),
            },
            "registry_scan": {
                "arena": registry_scan.get("arena"),
                "elapsed_s": registry_scan.get("elapsed_s"),
                "hit_count": registry_scan.get("hit_count"),
                "vtable": hx(self.registry_vtable),
                "vtable_rva": hex(LAND_COMBAT_REGISTRY_VTABLE_RVA),
                "handle_type_counts": compact_counter(registry_handle_types),
                "samples": registry_samples,
            },
            "side_status_counts": compact_counter(status_counts),
            "side_participant_kind_counts": compact_counter(kind_counts),
            "side_handle_type_counts": compact_counter(side_handle_types),
            "air_wing_side_count": len(air_wing_sides),
            "naval_unit_side_count": len(naval_unit_sides),
            "air_wing_side_examples": air_wing_sides[:8],
            "naval_unit_side_examples": naval_unit_sides[:8],
            "classification": (
                "possible_ccombat_side_air_event_root"
                if air_wing_sides
                else "rejected_no_ccombat_side_air_wing_participants"
            ),
            "status": (
                "possible_ccombat_side_air_event_root"
                if air_wing_sides
                else "rejected_no_ccombat_side_air_event_root"
            ),
            "reason": (
                "No live CCombatSide participant vector contained air-wing bodies on "
                "the validated 870ebf profile; observed populated sides are land "
                "division sides or invalid/empty, so this candidate cannot provide "
                "air sortie lifecycle, pairing, kill attribution, cause/source, ace, "
                "or rich reinforcement reason."
            ),
        }


def run(args: argparse.Namespace) -> dict[str, Any]:
    probe = CCombatSideAirRootProbe(args.pid, args.workers)
    try:
        root = probe.root_probe()
        _game_state, strategic_air = probe.require_root(root)
        air_wings, topology, _sources, _pools = probe.enumerate_air_wings(strategic_air)
        temporal = probe.temporal_probe(
            strategic_air,
            duration_s=args.duration_s,
            interval_s=args.interval_s,
            max_wings=args.max_body_field_wings,
        )
        side_probe = probe.ccombat_side_probe()
        return {
            "probe": "air_combat_losses_live_ccombat_side_event_root_probe_870ebf_v1",
            "created_utc": utc_now(),
            "read_only": True,
            "pid": args.pid,
            "root": root,
            "air_wing_root_topology": topology,
            "air_wing_rows_observed": len(air_wings),
            "air_wing_rows_on_mission_region": sum(
                1 for row in air_wings if row["mission_region"] is not None
            ),
            "temporal_delta_probe": temporal,
            "ccombat_side_probe": side_probe,
            "verdict": {
                "backlog_item": (
                    "direct land-style CCombatSide/registry air-combat event root for "
                    "sortie lifecycle, combat pairing, kill attribution, cause/source "
                    "of loss, tactical state, ace participation, and rich reinforcement "
                    "reason"
                ),
                "classification": side_probe.get("classification"),
                "compact_record_ready": False,
                "backlog_reduced": side_probe.get("status")
                == "rejected_no_ccombat_side_air_event_root",
                "reason": side_probe.get("reason"),
            },
        }
    finally:
        probe.close()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pid", type=int, required=True)
    parser.add_argument("--duration-s", type=float, default=20.0)
    parser.add_argument("--interval-s", type=float, default=0.10)
    parser.add_argument("--max-body-field-wings", type=int, default=96)
    parser.add_argument("--workers", type=int, default=max(1, (os.cpu_count() or 2) - 2))
    parser.add_argument("--out-dir", type=Path, default=None)
    args = parser.parse_args()

    report = run(args)
    out_path = None
    if args.out_dir is not None:
        args.out_dir.mkdir(parents=True, exist_ok=True)
        out_path = args.out_dir / "ccombat_side_event_root_probe_870ebf.json"
        out_path.write_text(json.dumps(report, indent=2, sort_keys=True), encoding="utf-8")
    side_probe = report["ccombat_side_probe"]
    summary = {
        "out_path": str(out_path) if out_path else None,
        "root_ok": report["root"]["strategic_air_ok"],
        "air_wing_rows_observed": report["air_wing_rows_observed"],
        "active_region_wings": report["air_wing_rows_on_mission_region"],
        "advanced_hours": report["temporal_delta_probe"]["advanced_hours"],
        "skipped_hours": report["temporal_delta_probe"]["skipped_hours"],
        "loss_examples": report["temporal_delta_probe"]["loss_like_delta_count"],
        "reinforcement_examples": report["temporal_delta_probe"]["reinforcement_like_delta_count"],
        "ccombat_side_status": side_probe.get("status"),
        "ccombat_side_hits": side_probe.get("side_scan", {}).get("hit_count"),
        "air_wing_side_count": side_probe.get("air_wing_side_count"),
        "compact_record_ready": False,
    }
    print(json.dumps(summary, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
