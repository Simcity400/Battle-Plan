"""Read-only domain-24 weather-internals probe for the 870ebf collector profile.

This lane-local probe checks whether the currently known active-event vectors on
SWeatherPerRegion add independent replay/gameplay state beyond the compact
weather_live fields already captured by the DLL. It does not inject, hook, or
write to hoi4.exe.
"""

from __future__ import annotations

import argparse
import json
import struct
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
MEMORY_READER = ROOT / "extractor" / "memory_reader"
if str(MEMORY_READER) not in sys.path:
    sys.path.insert(0, str(MEMORY_READER))

import winmem as wm  # noqa: E402

EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"

CG_STATE_DATE_OFFSET = 0x3316E90
WEATHER_CGAMESTATE_MANAGER_OFF = 0x688
WEATHER_MANAGER_VTABLE_RVA = 0x2960BE0
WEATHER_REGION_VTABLE_RVA = 0x2960B90
WEATHER_PROVINCE_VTABLE_RVA = 0x2960B40

WEATHER_MANAGER_PROVINCE_ARRAY_OFF = 0x10
WEATHER_MANAGER_REGION_ARRAY_OFF = 0x40
WEATHER_REGION_STRIDE = 0x160
WEATHER_PROVINCE_STRIDE = 0x180
WEATHER_REGION_COUNT = 305
MIN_PROVINCE_ROWS = 10_000
MAX_PROVINCE_ROWS = 65_536

REGION_ID_OFF = 0x08
REGION_ACTIVE_MODS_OFF = 0x30
REGION_ACCUMULATOR_OFF = 0x28
REGION_ACTIVE_FRONT_POSITIONS_OFF = 0x58
REGION_ACTIVE_EFFECT_ENTITIES_OFF = 0x70
REGION_PHENOM_ENUM_OFF = 0x78
REGION_RAIN_LIGHT_OFF = 0x108
REGION_RAIN_HEAVY_OFF = 0x110
REGION_SNOW_OFF = 0x118
REGION_BLIZZARD_OFF = 0x120
REGION_ARCTIC_WATER_OFF = 0x130

PROVINCE_ID_OFF = 0x08
PROVINCE_LOCAL_FRONT_POSITIONS_OFF = 0x58
PROVINCE_LOCAL_EFFECT_ENTITIES_OFF = 0x70

HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000
MAX_VECTOR_COUNT = 4096


def is_heap(value: int) -> bool:
    return HEAP_LO <= value < HEAP_HI


def hx(value: int | None) -> str | None:
    return None if value is None else f"0x{value:X}"


def u64(buf: bytes, off: int) -> int:
    return struct.unpack_from("<Q", buf, off)[0]


def u32(buf: bytes, off: int) -> int:
    return struct.unpack_from("<I", buf, off)[0]


def read_exact(proc: wm.Process, addr: int, size: int) -> bytes | None:
    data = proc.try_read(addr, size)
    return data if len(data) == size else None


def vec_header(proc: wm.Process, addr: int) -> dict | None:
    data = read_exact(proc, addr, 16)
    if data is None:
        return None
    begin, size, cap = struct.unpack("<QII", data)
    return {"begin": begin, "size": size, "cap": cap}


def vec_from_body(body: bytes, off: int) -> dict:
    begin = u64(body, off)
    size = u32(body, off + 8)
    cap = u32(body, off + 12)
    begin_heap = is_heap(begin)
    if size == 0:
        valid = True
    else:
        # Weather-local vectors on 870ebf do not use the third dword as a
        # reliable capacity. Validate the two production-relevant facts only:
        # readable heap begin and a sane bounded size.
        valid = begin_heap and size <= MAX_VECTOR_COUNT
    return {
        "begin": begin,
        "begin_hex": hx(begin),
        "begin_heap": begin_heap,
        "size": size,
        "cap": cap,
        "valid": valid,
    }


def boolish(body: bytes, off: int) -> int:
    value = body[off]
    return 1 if value != 0 else 0


def inc(counter: dict, key: str, amount: int = 1) -> None:
    counter[key] = counter.get(key, 0) + amount


def decode_region_rows(proc: wm.Process, base: int, module_base: int) -> tuple[dict, list[str]]:
    errors: list[str] = []
    counters: dict[str, object] = {
        "rows": WEATHER_REGION_COUNT,
        "phenomenon_counts": {},
        "region_accumulator_min": None,
        "region_accumulator_max": None,
        "region_accumulator_distinct_sample": [],
        "active_signal_rows": 0,
        "phenomenon_active_rows": 0,
        "phenomenon_active_vector_agree": 0,
        "active_front_position_vector_rows": 0,
        "active_effect_entity_vector_rows": 0,
        "active_front_position_entries": 0,
        "active_effect_entity_entries": 0,
        "active_modifier_entries": 0,
        "named_flag_rows": 0,
        "mismatches_first_20": [],
        "sample_rows": [],
    }
    data = read_exact(proc, base, WEATHER_REGION_COUNT * WEATHER_REGION_STRIDE)
    if data is None:
        return counters, [f"could not bulk-read region span {hx(base)}"]
    region_vtable = module_base + WEATHER_REGION_VTABLE_RVA
    accumulator_seen: set[int] = set()

    for idx in range(WEATHER_REGION_COUNT):
        body = data[idx * WEATHER_REGION_STRIDE : (idx + 1) * WEATHER_REGION_STRIDE]
        vtable = u64(body, 0)
        region_id = u32(body, REGION_ID_OFF)
        if vtable != region_vtable:
            errors.append(f"region {idx} vtable {hx(vtable)} != {hx(region_vtable)}")
        if region_id != idx:
            errors.append(f"region {idx} id {region_id} != index")

        phenomenon = u32(body, REGION_PHENOM_ENUM_OFF)
        phen_counts = counters["phenomenon_counts"]
        phen_counts[str(phenomenon)] = phen_counts.get(str(phenomenon), 0) + 1

        front_vec = vec_from_body(body, REGION_ACTIVE_FRONT_POSITIONS_OFF)
        entity_vec = vec_from_body(body, REGION_ACTIVE_EFFECT_ENTITIES_OFF)
        active_mods = vec_from_body(body, REGION_ACTIVE_MODS_OFF)
        for label, vec in (
            ("front_positions", front_vec),
            ("effect_entities", entity_vec),
            ("active_modifiers", active_mods),
        ):
            if not vec["valid"]:
                errors.append(
                    f"region {idx} {label} vector invalid begin={vec['begin_hex']} "
                    f"size={vec['size']} cap={vec['cap']}"
                )

        active_signal = front_vec["begin_heap"] or entity_vec["begin_heap"]
        phenomenon_active = phenomenon >= 1
        if active_signal:
            inc(counters, "active_signal_rows")
        if phenomenon_active:
            inc(counters, "phenomenon_active_rows")
        if front_vec["begin_heap"]:
            inc(counters, "active_front_position_vector_rows")
        if entity_vec["begin_heap"]:
            inc(counters, "active_effect_entity_vector_rows")
        inc(counters, "active_front_position_entries", front_vec["size"])
        inc(counters, "active_effect_entity_entries", entity_vec["size"])
        inc(counters, "active_modifier_entries", active_mods["size"])
        if active_signal == phenomenon_active:
            inc(counters, "phenomenon_active_vector_agree")
        elif len(counters["mismatches_first_20"]) < 20:
            counters["mismatches_first_20"].append(
                {
                    "region_id": idx,
                    "phenomenon": phenomenon,
                    "active_signal": active_signal,
                    "front": {"begin": front_vec["begin_hex"], "size": front_vec["size"]},
                    "entities": {"begin": entity_vec["begin_hex"], "size": entity_vec["size"]},
                }
            )

        named_flag_count = sum(
            boolish(body, off)
            for off in (
                REGION_RAIN_LIGHT_OFF,
                REGION_RAIN_HEAVY_OFF,
                REGION_SNOW_OFF,
                REGION_BLIZZARD_OFF,
                REGION_ARCTIC_WATER_OFF,
            )
        )
        if named_flag_count:
            inc(counters, "named_flag_rows")

        accumulator = u32(body, REGION_ACCUMULATOR_OFF)
        cur_min = counters["region_accumulator_min"]
        cur_max = counters["region_accumulator_max"]
        counters["region_accumulator_min"] = accumulator if cur_min is None else min(cur_min, accumulator)
        counters["region_accumulator_max"] = accumulator if cur_max is None else max(cur_max, accumulator)
        if len(accumulator_seen) < 16:
            accumulator_seen.add(accumulator)

        if len(counters["sample_rows"]) < 10 or idx in (31, 75, 81, 178, 304):
            counters["sample_rows"].append(
                {
                    "region_id": idx,
                    "phenomenon": phenomenon,
                    "active_signal": active_signal,
                    "active_front_positions": {"begin": front_vec["begin_hex"], "size": front_vec["size"]},
                    "active_effect_entities": {"begin": entity_vec["begin_hex"], "size": entity_vec["size"]},
                    "active_modifiers": {"begin": active_mods["begin_hex"], "size": active_mods["size"]},
                    "accumulator_raw": accumulator,
                    "named_flag_count": named_flag_count,
                }
            )

    counters["region_accumulator_distinct_sample"] = sorted(accumulator_seen)
    return counters, errors


def decode_province_vector_presence(proc: wm.Process, base: int, count: int, module_base: int) -> tuple[dict, list[str]]:
    errors: list[str] = []
    counters: dict[str, object] = {
        "rows": count,
        "local_front_position_vector_rows": 0,
        "local_effect_entity_vector_rows": 0,
        "local_front_position_entries": 0,
        "local_effect_entity_entries": 0,
        "sample_rows": [],
    }
    data = read_exact(proc, base, count * WEATHER_PROVINCE_STRIDE)
    if data is None:
        return counters, [f"could not bulk-read province span {hx(base)}"]
    province_vtable = module_base + WEATHER_PROVINCE_VTABLE_RVA

    for idx in range(count):
        body = data[idx * WEATHER_PROVINCE_STRIDE : (idx + 1) * WEATHER_PROVINCE_STRIDE]
        vtable = u64(body, 0)
        province_id = u32(body, PROVINCE_ID_OFF)
        if vtable != province_vtable and len(errors) < 20:
            errors.append(f"province {idx} vtable {hx(vtable)} != {hx(province_vtable)}")
        if province_id != idx and len(errors) < 20:
            errors.append(f"province {idx} id {province_id} != index")
        front_vec = vec_from_body(body, PROVINCE_LOCAL_FRONT_POSITIONS_OFF)
        entity_vec = vec_from_body(body, PROVINCE_LOCAL_EFFECT_ENTITIES_OFF)
        if not front_vec["valid"] and len(errors) < 20:
            errors.append(
                f"province {idx} local front vector invalid begin={front_vec['begin_hex']} "
                f"size={front_vec['size']} cap={front_vec['cap']}"
            )
        if not entity_vec["valid"] and len(errors) < 20:
            errors.append(
                f"province {idx} local entity vector invalid begin={entity_vec['begin_hex']} "
                f"size={entity_vec['size']} cap={entity_vec['cap']}"
            )
        if front_vec["begin_heap"]:
            inc(counters, "local_front_position_vector_rows")
        if entity_vec["begin_heap"]:
            inc(counters, "local_effect_entity_vector_rows")
        inc(counters, "local_front_position_entries", front_vec["size"])
        inc(counters, "local_effect_entity_entries", entity_vec["size"])
        if len(counters["sample_rows"]) < 10 and (front_vec["begin_heap"] or entity_vec["begin_heap"]):
            counters["sample_rows"].append(
                {
                    "province_id": idx,
                    "local_front_positions": {"begin": front_vec["begin_hex"], "size": front_vec["size"]},
                    "local_effect_entities": {"begin": entity_vec["begin_hex"], "size": entity_vec["size"]},
                }
            )

    return counters, errors


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--pid", type=int, default=None)
    ap.add_argument(
        "--out",
        default="extractor/native/lanes/weather_visual_drivers_live/proofs/weather_internals_870ebf_20260701.json",
    )
    args = ap.parse_args()

    pid = args.pid if args.pid is not None else wm.find_pid("hoi4.exe")
    if pid is None:
        raise SystemExit("hoi4.exe is not running")
    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)

    started = time.time()
    proc = wm.Process(pid, "hoi4.exe")
    try:
        fp = proc.fingerprint()
        module_base = proc.module_base
        sha = fp.get("sha256")
        proof: dict = {
            "probe": "weather_internals_870ebf_active_vectors_v1",
            "pid": pid,
            "module_base": hx(module_base),
            "module_path": fp.get("path"),
            "sha256": sha,
            "sha256_expected": EXPECTED_SHA256,
            "build_gate_ok": sha == EXPECTED_SHA256,
            "errors": [],
            "candidate": {
                "manager_vtable_rva": hx(WEATHER_MANAGER_VTABLE_RVA),
                "region_vtable_rva": hx(WEATHER_REGION_VTABLE_RVA),
                "province_vtable_rva": hx(WEATHER_PROVINCE_VTABLE_RVA),
                "region_active_front_positions_off": hx(REGION_ACTIVE_FRONT_POSITIONS_OFF),
                "region_active_effect_entities_off": hx(REGION_ACTIVE_EFFECT_ENTITIES_OFF),
                "province_local_front_positions_off": hx(PROVINCE_LOCAL_FRONT_POSITIONS_OFF),
                "province_local_effect_entities_off": hx(PROVINCE_LOCAL_EFFECT_ENTITIES_OFF),
            },
        }
        if sha != EXPECTED_SHA256:
            proof["errors"].append("build gate mismatch")
            out_path.write_text(json.dumps(proof, indent=2), encoding="utf-8")
            return 2

        gamestate = proc.u64(module_base + CG_STATE_DATE_OFFSET)
        manager = proc.u64(gamestate + WEATHER_CGAMESTATE_MANAGER_OFF) if is_heap(gamestate) else 0
        manager_vtable = proc.u64(manager) if is_heap(manager) else 0
        proof["roots"] = {
            "gamestate_ptr_source": hx(module_base + CG_STATE_DATE_OFFSET),
            "gamestate": hx(gamestate),
            "weather_manager": hx(manager),
            "weather_manager_vtable": hx(manager_vtable),
            "weather_manager_vtable_expected": hx(module_base + WEATHER_MANAGER_VTABLE_RVA),
        }
        if manager_vtable != module_base + WEATHER_MANAGER_VTABLE_RVA:
            proof["errors"].append("weather manager vtable mismatch")

        region_header = vec_header(proc, manager + WEATHER_MANAGER_REGION_ARRAY_OFF)
        province_header = vec_header(proc, manager + WEATHER_MANAGER_PROVINCE_ARRAY_OFF)
        proof["headers"] = {
            "region": {k: hx(v) if k == "begin" else v for k, v in (region_header or {}).items()},
            "province": {k: hx(v) if k == "begin" else v for k, v in (province_header or {}).items()},
        }
        if not region_header or region_header["size"] != WEATHER_REGION_COUNT or region_header["cap"] != WEATHER_REGION_COUNT:
            proof["errors"].append("region vector header invalid")
        if not province_header:
            proof["errors"].append("province vector header unreadable")
        elif not (
            is_heap(province_header["begin"])
            and MIN_PROVINCE_ROWS <= province_header["size"] <= MAX_PROVINCE_ROWS
            and province_header["cap"] >= province_header["size"]
        ):
            proof["errors"].append("province vector header invalid")

        if not proof["errors"]:
            region_counters, region_errors = decode_region_rows(proc, region_header["begin"], module_base)
            province_counters, province_errors = decode_province_vector_presence(
                proc, province_header["begin"], province_header["size"], module_base
            )
            proof["region_active_vector_validation"] = region_counters
            proof["province_local_vector_presence"] = province_counters
            proof["errors"].extend(region_errors[:20])
            proof["errors"].extend(province_errors[:20])

        region = proof.get("region_active_vector_validation", {})
        agree = region.get("phenomenon_active_vector_agree")
        rows = region.get("rows")
        proof["replay_classification"] = {
            "region_active_vectors_independent_gameplay_state": False if agree == rows else None,
            "reason": (
                "On this build the region active vectors are a redundant active-weather discriminator "
                "when they agree with phenomenon>=1 for every SWeatherPerRegion row. The compact stream "
                "already captures phenomenon, named low-byte weather flags, intensity, active modifier "
                "count, next-change raw, and region/province realized scalars. Vector contents remain "
                "visual/local-front particle detail, not a separate non-visual weather cause/effect."
            ),
            "close_zoom_particle_parity": (
                "Not claimed complete here. If exact close-zoom the game weather particles become product "
                "scope, the +0x58/+0x70 vector contents need a separate visual-particle lane."
            ),
            "rng_cursors": (
                "Rejected as domain-24 compact extraction requirement for current-state replay; they are "
                "future-determinism inputs, not required to replay captured hourly realized weather frames."
            ),
        }
        proof["candidate_valid"] = not proof["errors"]
        proof["elapsed_ms"] = int((time.time() - started) * 1000)
        out_path.write_text(json.dumps(proof, indent=2), encoding="utf-8")
        print(json.dumps({"candidate_valid": proof["candidate_valid"], "proof": str(out_path), "errors": proof["errors"][:10]}, indent=2))
        return 0 if proof["candidate_valid"] else 1
    finally:
        proc.close()


if __name__ == "__main__":
    raise SystemExit(main())
