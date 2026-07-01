"""Bounded read-only probe for active air combat/loss/reinforcement roots.

This is a development probe for the 870ebf the game executable profile. It does not
write to the game and does not run in the production capture loop.
"""

from __future__ import annotations

import argparse
import collections
import datetime as _dt
import json
import sys
import time
from pathlib import Path

THIS_DIR = Path(__file__).resolve().parent
MEMORY_READER = THIS_DIR.parent
REPO_ROOT = MEMORY_READER.parent
for _path in (str(MEMORY_READER), str(REPO_ROOT)):
    if _path not in sys.path:
        sys.path.insert(0, _path)

import winmem  # noqa: E402

EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"

C_GAME_STATE_DAILY_UPDATE_OFFSET = 0x1D4390
CG_STATE_DATE_OFFSET = 0x3316E90
GET_LONG_STRING_OFFSET = 0x526A40
GS_CURRENT_DATE_OFFSET = 0x468
GS_START_DATE_OFFSET = 0x4A8
CGAMESTATE_OFF_STRATEGIC_AIR_MANAGER = 0x690

DAILY_UPDATE_SENTINEL = bytes(
    [0x40, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41,
     0x55, 0x41, 0x56, 0x41, 0x57, 0x48, 0x8D, 0xAC]
)
GET_LONG_STRING_SENTINEL = bytes(
    [0x48, 0x89, 0x5C, 0x24, 0x08, 0x48, 0x89, 0x6C,
     0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57]
)

STRATEGIC_AIR_MANAGER_VTABLE_RVA = 0x2942118
STRATEGIC_AIR_MANAGER_OFF_COUNTRY_CONTAINERS = 0x30
STRATEGIC_AIR_MANAGER_OFF_COUNTRY_CONTAINER_COUNT = 0x3C
STRATEGIC_AIR_MANAGER_OFF_AIR_BASE_STATE_TABLE = 0x90

AIR_BASE_RECORD_VTABLE_RVA = 0x2941FA0
AIR_WING_OBJECT_VTABLE_RVA = 0x2964178
AIR_WING_OBJECT_OFF_BODY = 0x10
AIR_WING_VTABLE_RVA = 0x29641B0
AIR_WING_POOL_VTABLE_RVA = 0x2964208
AIR_WING_AIRBASE_OWNER_VTABLE_RVA = 0x2941FF8
AIR_WING_REGION_VTABLE_RVA = 0x29569A8
AIR_WING_REGION_ALT_VTABLE_RVA = 0x29569F8
DIVISION_EQUIPMENT_VARIANT_VTABLE_RVA = 0x293AFB8
NAVAL_COMBAT_VTABLE_RVA = 0x29B6660
NAVAL_COMBAT_SIDE_VTABLE_RVA = 0x29BC908
NAVAL_COMBAT_REGISTRY_VTABLE_RVA = 0x29B6588
NAVAL_COMBAT_MEMBER_VTABLE_RVA = 0x29F9608

AIR_WING_TYPE = 69
AIR_WING_OFF_TYPE = 0x08
AIR_WING_OFF_ID = 0x0C
AIR_WING_OFF_POOL = 0x28
AIR_WING_OFF_COUNT = 0x6C
AIR_WING_OFF_MISSION_MASK = 0x90
AIR_WING_OFF_MISSION_REGION = 0xB0
AIR_WING_OFF_EFFECTIVE_PLANES = 0x160
AIR_WING_OFF_EQUIPMENT_BANK = 0x1E8

AIR_WING_AIRBASE_OWNER_OFF_POOL_BEGIN = 0x58
AIR_WING_AIRBASE_OWNER_OFF_POOL_COUNT = 0x64
AIR_WING_AIRBASE_OWNER_OFF_TAG = 0x90
AIR_WING_POOL_OFF_AIRBASE_HANDLE = 0x18
AIR_WING_POOL_OFF_WING_OBJECT_BEGIN = 0x28
AIR_WING_POOL_OFF_WING_OBJECT_COUNT = 0x34
AIR_WING_POOL_OFF_AIRBASE_OWNER = 0x40
AIR_WING_REGION_OFF_ID = 0x58

DIVISION_EQUIPMENT_VARIANT_TYPE = 70
DIVISION_EQUIPMENT_VARIANT_OFF_TYPE = 0x08
DIVISION_EQUIPMENT_VARIANT_OFF_ID = 0x0C
AIR_WING_EQUIPMENT_ENTRY_STRIDE = 0x10

MAX_STRATEGIC_AIR_CONTAINERS = 4096
MAX_AIR_WING_POOLS_PER_OWNER = 2048
MAX_AIR_WING_OBJECTS_PER_POOL = 4096
MAX_NATIVE_AIR_WING_ROWS = 20000
MAX_AIR_WING_PLANES = 10000000
MAX_AIR_WING_EQUIPMENT_BANK_ROWS = 512
MAX_AIR_WING_EQUIPMENT_AMOUNT_RAW = 1000000000000000
MAX_AIR_WING_MISSION_REGION_ID = 65535
MAX_AIR_WING_ID = 2147483647

HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000
KNOWN_WING_U32_OFFSETS = {
    AIR_WING_OFF_TYPE,
    AIR_WING_OFF_ID,
    AIR_WING_OFF_COUNT,
    AIR_WING_OFF_MISSION_MASK,
    AIR_WING_OFF_EFFECTIVE_PLANES,
}


def is_heap(value: int | None) -> bool:
    return isinstance(value, int) and HEAP_LO <= value < HEAP_HI and value % 8 == 0


def hx(value: int | None) -> str | None:
    return None if value is None else f"0x{value:X}"


def read_u32(proc: winmem.Process, addr: int) -> int | None:
    try:
        return proc.u32(addr)
    except OSError:
        return None


def read_i64(proc: winmem.Process, addr: int) -> int | None:
    try:
        return proc.i64(addr)
    except OSError:
        return None


def read_u64(proc: winmem.Process, addr: int) -> int | None:
    try:
        return proc.u64(addr)
    except OSError:
        return None


def stable_vector_header(proc: winmem.Process, base: int, data_off: int, cap_off: int, size_off: int):
    first = (
        read_u64(proc, base + data_off),
        read_u32(proc, base + cap_off),
        read_u32(proc, base + size_off),
    )
    second = (
        read_u64(proc, base + data_off),
        read_u32(proc, base + cap_off),
        read_u32(proc, base + size_off),
    )
    if first != second or any(value is None for value in first):
        return None
    data, cap, size = first
    return data, cap, size


def qwords(blob: bytes):
    limit = len(blob) - (len(blob) % 8)
    for off in range(0, limit, 8):
        yield off, int.from_bytes(blob[off:off + 8], "little", signed=False)


class Probe:
    def __init__(self, pid: int):
        self.proc = winmem.Process(pid, "hoi4.exe")
        self.base = self.proc.module_base
        self.module_hi = self.proc.module_base + self.proc.module_size
        self.known_vtables = {
            self.base + STRATEGIC_AIR_MANAGER_VTABLE_RVA,
            self.base + AIR_BASE_RECORD_VTABLE_RVA,
            self.base + AIR_WING_OBJECT_VTABLE_RVA,
            self.base + AIR_WING_VTABLE_RVA,
            self.base + AIR_WING_POOL_VTABLE_RVA,
            self.base + AIR_WING_AIRBASE_OWNER_VTABLE_RVA,
            self.base + AIR_WING_REGION_VTABLE_RVA,
            self.base + AIR_WING_REGION_ALT_VTABLE_RVA,
            self.base + DIVISION_EQUIPMENT_VARIANT_VTABLE_RVA,
            self.base + NAVAL_COMBAT_VTABLE_RVA,
            self.base + NAVAL_COMBAT_SIDE_VTABLE_RVA,
            self.base + NAVAL_COMBAT_REGISTRY_VTABLE_RVA,
            self.base + NAVAL_COMBAT_MEMBER_VTABLE_RVA,
        }

    def close(self):
        self.proc.close()

    def root_probe(self) -> dict:
        fp = self.proc.fingerprint()
        fp_sha = (fp.get("sha256") or "").lower()
        daily = self.proc.try_read(self.base + C_GAME_STATE_DAILY_UPDATE_OFFSET, len(DAILY_UPDATE_SENTINEL))
        long_string = self.proc.try_read(self.base + GET_LONG_STRING_OFFSET, len(GET_LONG_STRING_SENTINEL))
        game_state = read_u64(self.proc, self.base + CG_STATE_DATE_OFFSET)
        strategic_air = read_u64(self.proc, (game_state or 0) + CGAMESTATE_OFF_STRATEGIC_AIR_MANAGER)
        strategic_air_vt = read_u64(self.proc, strategic_air or 0)
        current_raw = read_u32(self.proc, (game_state or 0) + GS_CURRENT_DATE_OFFSET)
        start_raw = read_u32(self.proc, (game_state or 0) + GS_START_DATE_OFFSET)

        return {
            "pid": self.proc.pid,
            "module": self.proc.module_name,
            "module_path": self.proc.module_path,
            "module_base": hx(self.base),
            "module_size": self.proc.module_size,
            "sha256": fp.get("sha256"),
            "sha256_expected": EXPECTED_SHA256,
            "sha256_ok": fp_sha == EXPECTED_SHA256,
            "daily_sentinel_addr": hx(self.base + C_GAME_STATE_DAILY_UPDATE_OFFSET),
            "daily_sentinel_ok": daily == DAILY_UPDATE_SENTINEL,
            "get_long_string_sentinel_addr": hx(self.base + GET_LONG_STRING_OFFSET),
            "get_long_string_sentinel_ok": long_string == GET_LONG_STRING_SENTINEL,
            "game_state_ptr_source": hx(self.base + CG_STATE_DATE_OFFSET),
            "game_state_ptr": hx(game_state),
            "game_state_ptr_ok": is_heap(game_state),
            "current_game_date_raw": current_raw,
            "start_date_raw": start_raw,
            "strategic_air_source": hx((game_state or 0) + CGAMESTATE_OFF_STRATEGIC_AIR_MANAGER),
            "strategic_air_ptr": hx(strategic_air),
            "strategic_air_vtable": hx(strategic_air_vt),
            "strategic_air_vtable_expected": hx(self.base + STRATEGIC_AIR_MANAGER_VTABLE_RVA),
            "strategic_air_ok": (
                is_heap(strategic_air)
                and strategic_air_vt == self.base + STRATEGIC_AIR_MANAGER_VTABLE_RVA
            ),
        }

    def require_root(self, root: dict) -> tuple[int, int]:
        required = [
            root["sha256_ok"],
            root["daily_sentinel_ok"],
            root["get_long_string_sentinel_ok"],
            root["game_state_ptr_ok"],
            root["strategic_air_ok"],
        ]
        if not all(required):
            raise RuntimeError("profile/root validation failed")
        game_state = int(root["game_state_ptr"], 16)
        strategic_air = int(root["strategic_air_ptr"], 16)
        return game_state, strategic_air

    def read_mission_region(self, body: int) -> int | None:
        region = read_u64(self.proc, body + AIR_WING_OFF_MISSION_REGION)
        if not is_heap(region):
            return None
        if read_u64(self.proc, region) != self.base + AIR_WING_REGION_VTABLE_RVA:
            return None
        if read_u64(self.proc, region + 8) != self.base + AIR_WING_REGION_ALT_VTABLE_RVA:
            return None
        region_id = read_u32(self.proc, region + AIR_WING_REGION_OFF_ID)
        if region_id is None or not (0 < region_id <= MAX_AIR_WING_MISSION_REGION_ID):
            return None
        return region_id

    def read_equipment(self, body: int) -> list[dict] | None:
        header = stable_vector_header(
            self.proc,
            body + AIR_WING_OFF_EQUIPMENT_BANK,
            0,
            0x08,
            0x0C,
        )
        if header is None:
            return None
        begin, cap, size = header
        if size == 0 and cap <= MAX_AIR_WING_EQUIPMENT_BANK_ROWS:
            return []
        if size > cap or cap > MAX_AIR_WING_EQUIPMENT_BANK_ROWS or not is_heap(begin):
            return None
        byte_len = size * AIR_WING_EQUIPMENT_ENTRY_STRIDE
        raw = self.proc.try_read(begin, byte_len)
        if len(raw) != byte_len:
            return None
        rows = []
        seen = set()
        for off in range(0, byte_len, AIR_WING_EQUIPMENT_ENTRY_STRIDE):
            variant_ptr = int.from_bytes(raw[off:off + 8], "little")
            amount_raw = int.from_bytes(raw[off + 8:off + 16], "little", signed=True)
            if (
                not is_heap(variant_ptr)
                or read_u64(self.proc, variant_ptr) != self.base + DIVISION_EQUIPMENT_VARIANT_VTABLE_RVA
                or read_u32(self.proc, variant_ptr + DIVISION_EQUIPMENT_VARIANT_OFF_TYPE)
                != DIVISION_EQUIPMENT_VARIANT_TYPE
            ):
                return None
            variant_id = read_u32(self.proc, variant_ptr + DIVISION_EQUIPMENT_VARIANT_OFF_ID)
            if variant_id is None or variant_id == 0 or variant_id in seen:
                return None
            if amount_raw < 0 or amount_raw > MAX_AIR_WING_EQUIPMENT_AMOUNT_RAW:
                return None
            seen.add(variant_id)
            rows.append({
                "variant_id": variant_id,
                "variant_ptr": variant_ptr,
                "amount_raw": amount_raw,
            })
        return rows

    def enumerate_air_wings(self, strategic_air: int) -> tuple[list[dict], dict, list[dict], list[dict]]:
        container_begin = read_u64(self.proc, strategic_air + STRATEGIC_AIR_MANAGER_OFF_COUNTRY_CONTAINERS)
        container_count = read_u32(self.proc, strategic_air + STRATEGIC_AIR_MANAGER_OFF_COUNTRY_CONTAINER_COUNT)
        topo = {
            "strategic_air": hx(strategic_air),
            "container_begin": hx(container_begin),
            "container_count": container_count,
            "containers_seen": 0,
            "pools_seen": 0,
            "wing_objects_seen": 0,
            "invalid_containers": 0,
            "invalid_pools": 0,
            "invalid_wing_objects": 0,
            "invalid_equipment_banks": 0,
        }
        sources = [{"kind": "strategic_air_manager", "addr": strategic_air, "span": 0x200}]
        pools = []
        rows_by_body = {}
        if container_count is None or container_count > MAX_STRATEGIC_AIR_CONTAINERS:
            topo["fatal"] = "container_count_invalid"
            return [], topo, sources, pools
        if container_count > 0 and not is_heap(container_begin):
            topo["fatal"] = "container_begin_invalid"
            return [], topo, sources, pools

        for container_index in range(container_count):
            container = read_u64(self.proc, container_begin + container_index * 8)
            if not container:
                continue
            topo["containers_seen"] += 1
            if (
                not is_heap(container)
                or read_u64(self.proc, container) != self.base + AIR_WING_AIRBASE_OWNER_VTABLE_RVA
            ):
                topo["invalid_containers"] += 1
                continue
            owner_tag = read_u32(self.proc, container + AIR_WING_AIRBASE_OWNER_OFF_TAG) or 0
            sources.append({"kind": "airbase_owner_container", "addr": container, "span": 0x120})
            pool_begin = read_u64(self.proc, container + AIR_WING_AIRBASE_OWNER_OFF_POOL_BEGIN)
            pool_count = read_u32(self.proc, container + AIR_WING_AIRBASE_OWNER_OFF_POOL_COUNT)
            if pool_count is None or pool_count > MAX_AIR_WING_POOLS_PER_OWNER:
                topo["invalid_containers"] += 1
                continue
            if pool_count > 0 and not is_heap(pool_begin):
                topo["invalid_containers"] += 1
                continue
            for pool_index in range(pool_count):
                pool = read_u64(self.proc, pool_begin + pool_index * 8)
                topo["pools_seen"] += 1
                if (
                    not is_heap(pool)
                    or read_u64(self.proc, pool) != self.base + AIR_WING_POOL_VTABLE_RVA
                    or read_u64(self.proc, pool + AIR_WING_POOL_OFF_AIRBASE_OWNER) != container
                ):
                    topo["invalid_pools"] += 1
                    continue
                sources.append({"kind": "air_wing_pool", "addr": pool, "span": 0x160})
                pools.append({"pool": pool, "container": container, "owner_tag_db_id": owner_tag})
                object_begin = read_u64(self.proc, pool + AIR_WING_POOL_OFF_WING_OBJECT_BEGIN)
                object_count = read_u32(self.proc, pool + AIR_WING_POOL_OFF_WING_OBJECT_COUNT)
                if object_count is None or object_count > MAX_AIR_WING_OBJECTS_PER_POOL:
                    topo["invalid_pools"] += 1
                    continue
                if object_count > 0 and not is_heap(object_begin):
                    topo["invalid_pools"] += 1
                    continue
                for object_index in range(object_count):
                    obj = read_u64(self.proc, object_begin + object_index * 8)
                    topo["wing_objects_seen"] += 1
                    if (
                        not is_heap(obj)
                        or read_u64(self.proc, obj) != self.base + AIR_WING_OBJECT_VTABLE_RVA
                    ):
                        topo["invalid_wing_objects"] += 1
                        continue
                    body = obj + AIR_WING_OBJECT_OFF_BODY
                    if (
                        read_u64(self.proc, body) != self.base + AIR_WING_VTABLE_RVA
                        or read_u32(self.proc, body + AIR_WING_OFF_TYPE) != AIR_WING_TYPE
                        or read_u64(self.proc, body + AIR_WING_OFF_POOL) != pool
                    ):
                        topo["invalid_wing_objects"] += 1
                        continue
                    wing_id = read_u32(self.proc, body + AIR_WING_OFF_ID) or 0
                    count = read_u32(self.proc, body + AIR_WING_OFF_COUNT)
                    effective = read_u32(self.proc, body + AIR_WING_OFF_EFFECTIVE_PLANES)
                    if (
                        not (0 < wing_id <= MAX_AIR_WING_ID)
                        or count is None
                        or effective is None
                        or count > MAX_AIR_WING_PLANES
                        or effective > MAX_AIR_WING_PLANES
                    ):
                        topo["invalid_wing_objects"] += 1
                        continue
                    equipment = self.read_equipment(body)
                    if equipment is None:
                        topo["invalid_equipment_banks"] += 1
                        equipment = []
                    rows_by_body.setdefault(body, {
                        "key": f"{owner_tag}:{wing_id}",
                        "owner_tag_db_id": owner_tag,
                        "wing_id": wing_id,
                        "object": obj,
                        "body": body,
                        "pool": pool,
                        "count": count,
                        "effective_planes": effective,
                        "mission_mask": read_u32(self.proc, body + AIR_WING_OFF_MISSION_MASK),
                        "mission_region": self.read_mission_region(body),
                        "equipment_amount_sum": sum(item["amount_raw"] for item in equipment),
                        "equipment_variant_count": len(equipment),
                    })
                    if len(rows_by_body) > MAX_NATIVE_AIR_WING_ROWS:
                        topo["fatal"] = "too_many_air_wings"
                        return [], topo, sources, pools
        return list(rows_by_body.values()), topo, sources, pools

    def direct_ref_probe(self, sources: list[dict], air_wings: list[dict]) -> dict:
        wing_refs = {row["body"] for row in air_wings}
        wing_refs.update(row["object"] for row in air_wings)
        scanned = collections.Counter()
        unknown_ptrs = collections.Counter()
        candidates = []
        for source in sources:
            scanned[source["kind"]] += 1
            blob = self.proc.try_read(source["addr"], source["span"])
            for field_off, ptr in qwords(blob):
                if not is_heap(ptr):
                    continue
                vt = read_u64(self.proc, ptr)
                if vt is None or not (self.base <= vt < self.module_hi):
                    continue
                if vt in self.known_vtables:
                    continue
                unknown_ptrs[source["kind"]] += 1
                target = self.proc.try_read(ptr, 0x400)
                refs = []
                for off, q in qwords(target):
                    if q in wing_refs:
                        refs.append({"offset": off, "ref": hx(q)})
                if len({r["ref"] for r in refs}) >= 2:
                    candidates.append({
                        "source_kind": source["kind"],
                        "source_addr": hx(source["addr"]),
                        "field_off": hex(field_off),
                        "target_addr": hx(ptr),
                        "target_vtable": hx(vt),
                        "target_vtable_rva": hex(vt - self.base),
                        "wing_refs": refs[:16],
                    })
        return {
            "candidate": "direct manager/container/pool object holding air-wing refs",
            "classification": "ambiguous aggregate/list-like object, not a validated combat/loss/reinforcement event root",
            "criterion": "unknown-vtable target object in bounded root fields contains >=2 direct air-wing body/object refs in first 0x400 bytes",
            "source_spans": {
                "strategic_air_manager": "0x0..0x200",
                "airbase_owner_container": "0x0..0x120",
                "air_wing_pool": "0x0..0x160",
            },
            "sources_scanned": dict(scanned),
            "unknown_vtable_pointers_seen": dict(unknown_ptrs),
            "candidate_count": len(candidates),
            "status": "ambiguous_direct_aggregate_candidate_not_event_root" if candidates else "rejected_no_direct_aggregate_object",
            "candidates": candidates[:50],
        }

    def body_u32_fields(self, body: int) -> dict[int, int]:
        fields = {}
        blob = self.proc.try_read(body, 0x240)
        limit = len(blob) - (len(blob) % 4)
        for off in range(0, limit, 4):
            fields[off] = int.from_bytes(blob[off:off + 4], "little", signed=False)
        return fields

    def temporal_probe(self, strategic_air: int, duration_s: float, interval_s: float, max_wings: int) -> dict:
        samples = []
        previous_by_key = None
        baseline_fields = {}
        unknown_u32_changes = collections.Counter()
        field_change_examples = collections.defaultdict(list)
        loss_like = []
        reinforcement_like = []
        other_changes = collections.Counter()
        deadline = time.monotonic() + duration_s
        while True:
            now = time.monotonic()
            if now > deadline and samples:
                break
            wall = _dt.datetime.utcnow().replace(microsecond=0).isoformat() + "Z"
            game_state = read_u64(self.proc, self.base + CG_STATE_DATE_OFFSET)
            raw_hour = read_u32(self.proc, (game_state or 0) + GS_CURRENT_DATE_OFFSET)
            rows, topo, _, _ = self.enumerate_air_wings(strategic_air)
            by_key = {row["key"]: row for row in rows}
            if previous_by_key is not None:
                for key, row in by_key.items():
                    old = previous_by_key.get(key)
                    if not old:
                        continue
                    count_delta = row["count"] - old["count"]
                    equip_delta = row["equipment_amount_sum"] - old["equipment_amount_sum"]
                    eff_delta = row["effective_planes"] - old["effective_planes"]
                    if count_delta < 0 or equip_delta < 0:
                        loss_like.append({
                            "sample_utc": wall,
                            "key": key,
                            "count_delta": count_delta,
                            "equipment_amount_delta": equip_delta,
                            "old_count": old["count"],
                            "new_count": row["count"],
                            "old_equipment_amount_sum": old["equipment_amount_sum"],
                            "new_equipment_amount_sum": row["equipment_amount_sum"],
                            "mission_region": row["mission_region"],
                            "mission_mask": row["mission_mask"],
                        })
                    elif count_delta > 0 or equip_delta > 0:
                        reinforcement_like.append({
                            "sample_utc": wall,
                            "key": key,
                            "count_delta": count_delta,
                            "equipment_amount_delta": equip_delta,
                            "old_count": old["count"],
                            "new_count": row["count"],
                            "old_equipment_amount_sum": old["equipment_amount_sum"],
                            "new_equipment_amount_sum": row["equipment_amount_sum"],
                            "mission_region": row["mission_region"],
                            "mission_mask": row["mission_mask"],
                        })
                    if eff_delta:
                        other_changes["effective_planes"] += 1
                    if row["mission_mask"] != old["mission_mask"]:
                        other_changes["mission_mask"] += 1
                    if row["mission_region"] != old["mission_region"]:
                        other_changes["mission_region"] += 1
            tracked_rows = [row for row in rows if row["mission_region"] is not None][:max_wings]
            if not tracked_rows:
                tracked_rows = rows[:max_wings]
            for row in tracked_rows:
                fields = self.body_u32_fields(row["body"])
                old_fields = baseline_fields.setdefault(row["key"], fields)
                for off, value in fields.items():
                    if off in KNOWN_WING_U32_OFFSETS:
                        continue
                    old_value = old_fields.get(off)
                    if old_value is not None and old_value != value:
                        unknown_u32_changes[off] += 1
                        if len(field_change_examples[off]) < 5:
                            field_change_examples[off].append({
                                "key": row["key"],
                                "old": old_value,
                                "new": value,
                                "body": hx(row["body"]),
                            })
            samples.append({
                "sample_utc": wall,
                "raw_hour": raw_hour,
                "air_wing_rows": len(rows),
                "active_region_wings": sum(1 for row in rows if row["mission_region"] is not None),
                "plane_count_sum": sum(row["count"] for row in rows),
                "effective_planes_sum": sum(row["effective_planes"] for row in rows),
                "equipment_amount_sum": sum(row["equipment_amount_sum"] for row in rows),
                "invalid_wing_objects": topo.get("invalid_wing_objects", 0),
                "invalid_equipment_banks": topo.get("invalid_equipment_banks", 0),
            })
            previous_by_key = by_key
            if time.monotonic() + interval_s > deadline:
                if time.monotonic() > deadline:
                    break
            time.sleep(interval_s)

        distinct_hours = []
        for sample in samples:
            hour = sample["raw_hour"]
            if hour is not None and (not distinct_hours or distinct_hours[-1] != hour):
                distinct_hours.append(hour)
        skipped = 0
        for old, new in zip(distinct_hours, distinct_hours[1:]):
            if new > old + 1:
                skipped += new - old - 1
        top_unknown = []
        for off, count in unknown_u32_changes.most_common(20):
            top_unknown.append({
                "offset": hex(off),
                "change_count": count,
                "examples": field_change_examples[off],
            })
        return {
            "duration_s": duration_s,
            "interval_s": interval_s,
            "samples": len(samples),
            "first_hour": distinct_hours[0] if distinct_hours else None,
            "last_hour": distinct_hours[-1] if distinct_hours else None,
            "distinct_hours_seen": len(distinct_hours),
            "advanced_hours": (
                distinct_hours[-1] - distinct_hours[0] if len(distinct_hours) >= 2 else 0
            ),
            "skipped_hours": skipped,
            "hourly_or_lower_sampling_passed": skipped == 0 and len(distinct_hours) > 0,
            "loss_like_known_wing_or_loadout_deltas": loss_like[:50],
            "loss_like_delta_count": len(loss_like),
            "reinforcement_like_known_wing_or_loadout_deltas": reinforcement_like[:50],
            "reinforcement_like_delta_count": len(reinforcement_like),
            "other_known_field_change_counts": dict(other_changes),
            "top_unknown_air_wing_body_u32_changes": top_unknown,
            "first_sample": samples[0] if samples else None,
            "last_sample": samples[-1] if samples else None,
        }


def vtable_scan(pid: int, base: int, workers: int) -> dict:
    from memory_reader import fastscan

    arena = fastscan.discover_arena(pid)
    if arena is None:
        return {"status": "arena_discovery_failed"}
    lo, hi = arena
    h = winmem._k32.OpenProcess(
        winmem.PROCESS_VM_READ | winmem.PROCESS_QUERY_INFORMATION, False, pid
    )
    try:
        regions = fastscan.enumerate_regions(h, lo, hi)
    finally:
        winmem._k32.CloseHandle(h)
    out = {
        "arena": {"lo": hx(lo), "hi": hx(hi), "region_count": len(regions),
                  "bytes": sum(size for _, size in regions)},
        "workers": workers,
        "scans": {},
    }
    scan_targets = {
        "CNavalCombat_vtable": base + NAVAL_COMBAT_VTABLE_RVA,
        "CNavalCombatant_side_vtable": base + NAVAL_COMBAT_SIDE_VTABLE_RVA,
        "CFEXMember_vtable": base + NAVAL_COMBAT_MEMBER_VTABLE_RVA,
    }
    for name, vt in scan_targets.items():
        spec = fastscan.make_qword_eq_matcher(vt)
        started = time.monotonic()
        hits = fastscan.parallel_scan(
            pid, lo, hi, workers=workers, regions=regions, **spec
        )
        out["scans"][name] = {
            "vtable": hx(vt),
            "rva": hex(vt - base),
            "hit_count": len(hits),
            "elapsed_s": round(time.monotonic() - started, 3),
            "sample_hits": [hx(hit) for hit in hits[:20]],
        }
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--pid", type=int, required=True)
    ap.add_argument("--duration-s", type=float, default=20.0)
    ap.add_argument("--interval-s", type=float, default=0.10)
    ap.add_argument("--max-body-field-wings", type=int, default=96)
    ap.add_argument("--scan-vtables", action="store_true")
    ap.add_argument("--workers", type=int, default=4)
    ap.add_argument("--out-dir", default=None)
    args = ap.parse_args()

    probe = Probe(args.pid)
    try:
        root = probe.root_probe()
        game_state, strategic_air = probe.require_root(root)
        air_wings, topology, sources, _pools = probe.enumerate_air_wings(strategic_air)
        direct_refs = probe.direct_ref_probe(sources, air_wings)
        temporal = probe.temporal_probe(
            strategic_air,
            duration_s=args.duration_s,
            interval_s=args.interval_s,
            max_wings=args.max_body_field_wings,
        )
        report = {
            "probe": "air_combat_losses_live_870ebf_bounded_probe_v2",
            "created_utc": _dt.datetime.utcnow().replace(microsecond=0).isoformat() + "Z",
            "read_only": True,
            "root": root,
            "validated_offsets": {
                "CGameState_global": "module+0x3316E90 -> CGameState*",
                "current_game_date_raw": "CGameState+0x468",
                "StrategicAirManager": "CGameState+0x690",
                "StrategicAirManager_vtable": "module+0x2942118",
                "StrategicAirManager_country_containers": "+0x30 begin, +0x3C count",
                "airbase_owner_container": "vtable module+0x2941FF8, +0x90 owner tag, +0x58 pool begin, +0x64 pool count",
                "air_wing_pool": "vtable module+0x2964208, +0x18 airbase handle, +0x28 wing object begin, +0x34 wing object count, +0x40 owner backref",
                "air_wing_object": "vtable module+0x2964178, body at object+0x10",
                "air_wing_body": "vtable module+0x29641B0, type +0x08=69, id +0x0C, pool +0x28, count +0x6C, mission mask +0x90, mission region +0xB0, effective planes +0x160, equipment bank +0x1E8",
            },
            "air_wing_root_topology": topology,
            "air_wing_rows_observed": len(air_wings),
            "air_wing_rows_on_mission_region": sum(1 for row in air_wings if row["mission_region"] is not None),
            "direct_ref_candidate_probe": direct_refs,
            "temporal_probe": temporal,
            "vtable_scan": None,
            "verdict": {
                "compact_record_ready": False,
                "reason": (
                    "no distinct active air combat/loss/reinforcement root was validated; "
                    "probe only validated the existing strategic-air/air-wing/loadout topology "
                    "and rejected the bounded direct-aggregate candidate"
                ),
            },
        }
        if args.scan_vtables:
            report["vtable_scan"] = vtable_scan(args.pid, probe.base, args.workers)
        out_path = None
        if args.out_dir:
            out_dir = Path(args.out_dir)
            out_dir.mkdir(parents=True, exist_ok=True)
            out_path = out_dir / "air_combat_losses_probe_870ebf.json"
            out_path.write_text(json.dumps(report, indent=2, sort_keys=True), encoding="utf-8")
        summary = {
            "out_path": str(out_path) if out_path else None,
            "root_ok": root["strategic_air_ok"],
            "air_wing_rows_observed": report["air_wing_rows_observed"],
            "direct_ref_status": direct_refs["status"],
            "loss_like_delta_count": temporal["loss_like_delta_count"],
            "reinforcement_like_delta_count": temporal["reinforcement_like_delta_count"],
            "advanced_hours": temporal["advanced_hours"],
            "skipped_hours": temporal["skipped_hours"],
            "hourly_or_lower_sampling_passed": temporal["hourly_or_lower_sampling_passed"],
            "compact_record_ready": False,
        }
        print(json.dumps(summary, indent=2, sort_keys=True))
        return 0
    finally:
        probe.close()


if __name__ == "__main__":
    raise SystemExit(main())
