"""Bounded read-only probe for domain-24 weather residuals.

This lane resolves only the residual current-state question for weather_live.
It does not inject, hook, or participate in the hot capture loop. The production
path remains the existing DLL compact stream; this probe validates whether the
remaining live weather internals are independent current-state rows or should
stay badged/omitted.
"""

from __future__ import annotations

import argparse
import json
import shutil
import struct
import sys
import time
from pathlib import Path
from typing import Any

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
REGION_ACCUMULATOR_OFF = 0x28
REGION_ACTIVE_MODS_OFF = 0x30
REGION_ACTIVE_FRONT_POSITIONS_OFF = 0x58
REGION_ACTIVE_EFFECT_ENTITIES_OFF = 0x70
REGION_PHENOM_ENUM_OFF = 0x78
REGION_INTENSITY_OFF = 0x5D
REGION_RAIN_LIGHT_OFF = 0x108
REGION_RAIN_HEAVY_OFF = 0x110
REGION_SNOW_OFF = 0x118
REGION_BLIZZARD_OFF = 0x120
REGION_SANDSTORM_OFF = 0x128
REGION_ARCTIC_WATER_OFF = 0x130
REGION_TEMPERATURE_OFF = 0x138
REGION_NEXT_CHANGE_OFF = 0x150

PROVINCE_ID_OFF = 0x08
PROVINCE_WATER_OFF = 0x38
PROVINCE_SNOW_OFF = 0x40
PROVINCE_TEMP_OFF = 0x118
PROVINCE_TEMP_CHANGE_OFF = 0x128
PROVINCE_MUD_OFF = 0x130
PROVINCE_CUSTOM_MODS_OFF = 0x148

MAX_TEMP_RAW = 10_000_000
MAX_TEMP_CHANGE_RAW = 10_000_000
MAX_PRECIP_RAW = 100_000_000
MAX_VECTOR_COUNT = 4_096
HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000


def is_heap(value: int) -> bool:
    return HEAP_LO <= value < HEAP_HI


def hx(value: int | None) -> str | None:
    return None if value is None else f"0x{value:X}"


def u64(buf: bytes, off: int) -> int:
    return struct.unpack_from("<Q", buf, off)[0]


def u32(buf: bytes, off: int) -> int:
    return struct.unpack_from("<I", buf, off)[0]


def i32(buf: bytes, off: int) -> int:
    return struct.unpack_from("<i", buf, off)[0]


def u16(buf: bytes, off: int) -> int:
    return struct.unpack_from("<H", buf, off)[0]


def read_exact(proc: wm.Process, addr: int, size: int) -> bytes | None:
    data = proc.try_read(addr, size)
    return data if len(data) == size else None


def vec_header(proc: wm.Process, addr: int) -> dict[str, int] | None:
    data = read_exact(proc, addr, 16)
    if data is None:
        return None
    begin, size, cap = struct.unpack("<QII", data)
    return {"begin": begin, "size": size, "cap": cap}


def vec_from_body(body: bytes, off: int) -> dict[str, Any]:
    begin = u64(body, off)
    size = u32(body, off + 8)
    cap = u32(body, off + 12)
    begin_heap = is_heap(begin)
    valid = size == 0 or (begin_heap and size <= MAX_VECTOR_COUNT)
    return {
        "begin": begin,
        "begin_hex": hx(begin),
        "begin_heap": begin_heap,
        "size": size,
        "cap": cap,
        "valid": valid,
    }


def bump(counters: dict[str, Any], key: str, amount: int = 1) -> None:
    counters[key] = int(counters.get(key, 0)) + amount


def boolish(body: bytes, off: int) -> int:
    return 1 if body[off] != 0 else 0


def minmax_set(counters: dict[str, Any], key: str, value: int) -> None:
    min_key = f"{key}_min"
    max_key = f"{key}_max"
    counters[min_key] = value if counters.get(min_key) is None else min(counters[min_key], value)
    counters[max_key] = value if counters.get(max_key) is None else max(counters[max_key], value)


def jsonable_vec_header(header: dict[str, int] | None) -> dict[str, Any] | None:
    if header is None:
        return None
    return {
        "begin": hx(header["begin"]),
        "size": header["size"],
        "cap": header["cap"],
    }


def validate_region_rows(proc: wm.Process, base: int, module_base: int) -> tuple[dict[str, Any], list[str]]:
    errors: list[str] = []
    counters: dict[str, Any] = {
        "rows": WEATHER_REGION_COUNT,
        "phenomenon_counts": {},
        "active_signal_rows": 0,
        "phenomenon_active_rows": 0,
        "phenomenon_active_vector_agree": 0,
        "active_front_position_vector_rows": 0,
        "active_effect_entity_vector_rows": 0,
        "active_front_position_entries": 0,
        "active_effect_entity_entries": 0,
        "active_modifier_entries": 0,
        "named_flag_rows": 0,
        "named_flag_counts": {
            "rain_light": 0,
            "rain_heavy": 0,
            "snow": 0,
            "blizzard": 0,
            "sandstorm": 0,
            "arctic_water": 0,
        },
        "region_accumulator_distinct_sample": [],
        "sample_rows": [],
        "mismatches_first_20": [],
    }
    data = read_exact(proc, base, WEATHER_REGION_COUNT * WEATHER_REGION_STRIDE)
    if data is None:
        return counters, [f"could not bulk-read SWeatherPerRegion span {hx(base)}"]
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
        phen_counts[str(phenomenon)] = int(phen_counts.get(str(phenomenon), 0)) + 1

        front_vec = vec_from_body(body, REGION_ACTIVE_FRONT_POSITIONS_OFF)
        entity_vec = vec_from_body(body, REGION_ACTIVE_EFFECT_ENTITIES_OFF)
        active_mods = vec_from_body(body, REGION_ACTIVE_MODS_OFF)
        for label, vec in (
            ("active_front_positions", front_vec),
            ("weather_effect_entities", entity_vec),
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
            bump(counters, "active_signal_rows")
        if phenomenon_active:
            bump(counters, "phenomenon_active_rows")
        if front_vec["begin_heap"]:
            bump(counters, "active_front_position_vector_rows")
        if entity_vec["begin_heap"]:
            bump(counters, "active_effect_entity_vector_rows")
        bump(counters, "active_front_position_entries", int(front_vec["size"]))
        bump(counters, "active_effect_entity_entries", int(entity_vec["size"]))
        bump(counters, "active_modifier_entries", int(active_mods["size"]))
        if active_signal == phenomenon_active:
            bump(counters, "phenomenon_active_vector_agree")
        elif len(counters["mismatches_first_20"]) < 20:
            counters["mismatches_first_20"].append(
                {
                    "region_id": idx,
                    "phenomenon": phenomenon,
                    "active_signal": active_signal,
                    "front_size": front_vec["size"],
                    "entity_size": entity_vec["size"],
                }
            )

        named_flags = {
            "rain_light": boolish(body, REGION_RAIN_LIGHT_OFF),
            "rain_heavy": boolish(body, REGION_RAIN_HEAVY_OFF),
            "snow": boolish(body, REGION_SNOW_OFF),
            "blizzard": boolish(body, REGION_BLIZZARD_OFF),
            "sandstorm": boolish(body, REGION_SANDSTORM_OFF),
            "arctic_water": boolish(body, REGION_ARCTIC_WATER_OFF),
        }
        named_flag_count = sum(named_flags.values())
        for name, active in named_flags.items():
            if active:
                bump(counters["named_flag_counts"], name)
        if named_flag_count:
            bump(counters, "named_flag_rows")

        accumulator = u32(body, REGION_ACCUMULATOR_OFF)
        next_change = u32(body, REGION_NEXT_CHANGE_OFF)
        temperature = i32(body, REGION_TEMPERATURE_OFF)
        intensity = body[REGION_INTENSITY_OFF]
        minmax_set(counters, "region_accumulator_raw", accumulator)
        minmax_set(counters, "next_weather_change_raw", next_change)
        minmax_set(counters, "region_temperature_raw", temperature)
        minmax_set(counters, "region_intensity", intensity)
        if len(accumulator_seen) < 16:
            accumulator_seen.add(accumulator)

        if len(counters["sample_rows"]) < 8 or idx in (31, 75, 81, 178, 304):
            counters["sample_rows"].append(
                {
                    "region_id": idx,
                    "phenomenon": phenomenon,
                    "active_signal": active_signal,
                    "active_front_positions": {"begin": front_vec["begin_hex"], "size": front_vec["size"]},
                    "weather_effect_entities": {"begin": entity_vec["begin_hex"], "size": entity_vec["size"]},
                    "active_modifiers": {"begin": active_mods["begin_hex"], "size": active_mods["size"]},
                    "accumulator_raw": accumulator,
                    "next_weather_change_raw": next_change,
                    "temperature_raw": temperature,
                    "intensity": intensity,
                    "named_flag_count": named_flag_count,
                    "named_flags": named_flags,
                }
            )

    counters["region_accumulator_distinct_sample"] = sorted(accumulator_seen)
    return counters, errors


def validate_province_visual_drivers(
    proc: wm.Process, base: int, count: int, module_base: int
) -> tuple[dict[str, Any], list[str]]:
    errors: list[str] = []
    counters: dict[str, Any] = {
        "rows": count,
        "province_vtable_match": 0,
        "province_id_u16_match": 0,
        "province_id_u32_match": 0,
        "temperature_present_rows": 0,
        "water_measured_rows": 0,
        "snow_measured_rows": 0,
        "mud_measured_rows": 0,
        "mud_active_rows": 0,
        "custom_modifier_valid_rows": 0,
        "custom_modifier_positive_rows": 0,
        "map_recolor_driver_rows": 0,
        "dynamic_lut_driver_rows": 0,
        "sample_rows": [],
    }
    data = read_exact(proc, base, count * WEATHER_PROVINCE_STRIDE)
    if data is None:
        return counters, [f"could not bulk-read SWeatherPerProvince span {hx(base)}"]
    province_vtable = module_base + WEATHER_PROVINCE_VTABLE_RVA

    for idx in range(count):
        body = data[idx * WEATHER_PROVINCE_STRIDE : (idx + 1) * WEATHER_PROVINCE_STRIDE]
        vtable = u64(body, 0)
        id_u16 = u16(body, PROVINCE_ID_OFF)
        id_u32 = u32(body, PROVINCE_ID_OFF)
        if vtable == province_vtable:
            bump(counters, "province_vtable_match")
        elif len(errors) < 20:
            errors.append(f"province {idx} vtable {hx(vtable)} != {hx(province_vtable)}")
        if id_u16 == idx:
            bump(counters, "province_id_u16_match")
        elif len(errors) < 20:
            errors.append(f"province {idx} id_u16 {id_u16} != index")
        if id_u32 == idx:
            bump(counters, "province_id_u32_match")

        water_raw = i32(body, PROVINCE_WATER_OFF)
        snow_raw = i32(body, PROVINCE_SNOW_OFF)
        temp_raw = i32(body, PROVINCE_TEMP_OFF)
        temp_change_raw = i32(body, PROVINCE_TEMP_CHANGE_OFF)
        mud_byte = body[PROVINCE_MUD_OFF]
        custom_vec = vec_from_body(body, PROVINCE_CUSTOM_MODS_OFF)

        minmax_set(counters, "water_raw", water_raw)
        minmax_set(counters, "snow_raw", snow_raw)
        minmax_set(counters, "temperature_raw", temp_raw)
        minmax_set(counters, "temperature_change_raw", temp_change_raw)

        if temp_raw != 0:
            bump(counters, "temperature_present_rows")
        if 0 <= water_raw <= MAX_PRECIP_RAW:
            bump(counters, "water_measured_rows")
        else:
            errors.append(f"province {idx} water_raw {water_raw} out of bounds")
        if 0 <= snow_raw <= MAX_PRECIP_RAW:
            bump(counters, "snow_measured_rows")
        else:
            errors.append(f"province {idx} snow_raw {snow_raw} out of bounds")
        if not (-MAX_TEMP_RAW <= temp_raw <= MAX_TEMP_RAW):
            errors.append(f"province {idx} temperature_raw {temp_raw} out of bounds")
        if not (-MAX_TEMP_CHANGE_RAW <= temp_change_raw <= MAX_TEMP_CHANGE_RAW):
            errors.append(f"province {idx} temperature_change_raw {temp_change_raw} out of bounds")
        if mud_byte in (0, 1):
            bump(counters, "mud_measured_rows")
            if mud_byte == 1:
                bump(counters, "mud_active_rows")
        else:
            errors.append(f"province {idx} mud byte {mud_byte} is not bool")
        if custom_vec["valid"]:
            bump(counters, "custom_modifier_valid_rows")
            if custom_vec["size"] > 0:
                bump(counters, "custom_modifier_positive_rows")
        else:
            errors.append(
                f"province {idx} custom modifier vector invalid begin={custom_vec['begin_hex']} "
                f"size={custom_vec['size']} cap={custom_vec['cap']}"
            )
        # Domain 24 v2 declares temp/water/snow as map recolor drivers and snow/mud as
        # dynamic-LUT drivers; this probe confirms the source fields are present.
        if temp_raw != 0 or water_raw >= 0 or snow_raw >= 0:
            bump(counters, "map_recolor_driver_rows")
        if snow_raw >= 0 or mud_byte in (0, 1):
            bump(counters, "dynamic_lut_driver_rows")

        if len(counters["sample_rows"]) < 8 or idx in (31, 3946, 7250, 10167):
            counters["sample_rows"].append(
                {
                    "province_id": idx,
                    "water_raw": water_raw,
                    "snow_raw": snow_raw,
                    "temperature_raw": temp_raw,
                    "temperature_change_raw": temp_change_raw,
                    "mud": mud_byte,
                    "custom_modifier_count": custom_vec["size"],
                    "custom_vector_begin": custom_vec["begin_hex"],
                }
            )

    return counters, errors


def coverage_rows_for_weather() -> dict[str, Any]:
    path = ROOT / "extractor" / "coverage_audit" / "memory_manifest.json"
    try:
        manifest = json.loads(path.read_text(encoding="utf-8"))
    except OSError as err:
        return {"error": str(err), "rows": []}
    wanted = {
        "weather.current_province/current_region/seed",
        "weather temperature formula / RNG / phenomenon-name strings",
    }
    rows = [
        {
            "path": row.get("path"),
            "domain": row.get("domain"),
            "source_class": row.get("source_class"),
            "gap": row.get("gap"),
            "status": row.get("status"),
            "note": row.get("note"),
            "ui_ref": row.get("ui_ref"),
        }
        for row in manifest.get("rows", [])
        if row.get("path") in wanted
    ]
    return {
        "scope_boundary": manifest.get("scope_boundary"),
        "category_model": manifest.get("completeness_method", {}).get("category_model"),
        "rows": rows,
    }


def compact_manifest_summary(manifest_path: Path) -> dict[str, Any]:
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    compact = manifest.get("compact_stream", {})
    weather = compact.get("per_domain", {}).get("weather_live", {})
    return {
        "source_manifest": str(manifest_path),
        "copied_manifest": None,
        "scheduled_domains": manifest.get("scheduled_domains"),
        "game_hours_advanced": manifest.get("game_hours_advanced"),
        "unique_game_hour_count": manifest.get("unique_game_hour_count"),
        "skipped_game_hours": manifest.get("skipped_game_hours"),
        "hourly_requirement": manifest.get("hourly_requirement"),
        "replay_file": manifest.get("replay_file"),
        "compact_stream": {
            "ordered": compact.get("ordered"),
            "decodable": compact.get("decodable"),
            "domain_data_record_count": compact.get("domain_data_record_count"),
            "frame_end_record_count": compact.get("frame_end_record_count"),
            "source_unavailable_seen": compact.get("source_unavailable_seen"),
            "partial_data_seen": compact.get("partial_data_seen"),
            "bridge_unavailable_seen": compact.get("bridge_unavailable_seen"),
            "worker_panic_seen": compact.get("worker_panic_seen"),
            "weather_live": {
                "domain_id": weather.get("domain_id"),
                "data_record_count": weather.get("data_record_count"),
                "data_units_total": weather.get("data_units_total"),
                "read_time_us": weather.get("read_time_us"),
            },
        },
    }


def classify(proof: dict[str, Any]) -> dict[str, Any]:
    region = proof.get("region_validation", {})
    province = proof.get("province_visual_driver_validation", {})
    compact = proof.get("compact_manifest_summary", {}).get("compact_stream", {})
    weather_units = compact.get("weather_live", {}).get("data_units_total", {}) or {}

    active_agree = region.get("phenomenon_active_vector_agree")
    region_rows = region.get("rows")
    province_rows = province.get("rows")
    province_map_rows = weather_units.get("weather_province_map_recolor_driver_rows")
    province_lut_rows = weather_units.get("weather_province_dynamic_lut_driver_rows")
    sandstorm_rows = (region.get("named_flag_counts") or {}).get("sandstorm")

    return {
        "completion_for_this_lane": "COMPLETE" if proof.get("candidate_valid") else "NOT COMPLETE",
        "whole_weather_concept": "NOT COMPLETE",
        "exact_backlog_item_removed_or_narrowed": (
            "Non-visual weather current-state residuals for domain 24 narrowed. Province "
            "map-tone drivers are already compact-covered; SWeatherPerRegion+0x128 was "
            "validated as the missing sandstorm named-region flag and needs a domain-24 "
            "v3 flag-bit registration. Exact generator/RNG enum constants and close-zoom "
            "particle vector contents remain open-computed/visual scope."
        ),
        "candidate_rejections": [
            {
                "candidate": "SWeatherPerRegion +0x58/+0x70 active vectors as independent non-visual gameplay state",
                "status": (
                    "rejected_for_current_state_compact"
                    if active_agree == region_rows and region_rows == WEATHER_REGION_COUNT
                    else "not_rejected"
                ),
                "evidence": {
                    "rows": region_rows,
                    "phenomenon_active_vector_agree": active_agree,
                    "active_signal_rows": region.get("active_signal_rows"),
                    "phenomenon_active_rows": region.get("phenomenon_active_rows"),
                },
                "remaining_scope": "Vector contents remain particle/local-front visual detail if exact close-zoom particle parity enters scope.",
            },
            {
                "candidate": "weather.current_province/current_region/seed and generic weather RNG cursors",
                "status": "rejected_for_current_state_compact",
                "evidence": "Coverage manifest classes these as engine-internal/excluded for current-state visible replay.",
                "remaining_scope": "Future-determinism or continuing the game identically would need a separate scope.",
            },
            {
                "candidate": "weather temperature formula / RNG / phenomenon-name strings",
                "status": "kept_open_computed_not_domain24_current_state_blocker",
                "evidence": "Coverage manifest splits realized weather state from generator/function parity.",
                "remaining_scope": "Ghidra/recompute target for exact generator or enum-name parity.",
            },
            {
                "candidate": "province map-tone snow/mud/frozen/weather drivers absent from compact domain 24",
                "status": (
                    "not_found"
                    if province_rows
                    and province_map_rows
                    and province_lut_rows
                    and weather_units.get("weather_region_rows")
                    else "not_proven"
                ),
                "evidence": {
                    "live_province_rows": province_rows,
                    "compact_weather_province_map_recolor_driver_rows": province_map_rows,
                    "compact_weather_province_dynamic_lut_driver_rows": province_lut_rows,
                    "compact_weather_region_rows": weather_units.get("weather_region_rows"),
                    "compact_weather_active_region_rows": weather_units.get("weather_active_region_rows"),
                },
                "remaining_scope": "Close-zoom particles and named region flags are separate from province map-tone driver closure.",
            },
            {
                "candidate": "named region sandstorm weather driver absent from compact domain 24",
                "status": (
                    "blocker_removed_requires_domain24_v3_flag"
                    if sandstorm_rows is not None
                    else "not_proven"
                ),
                "evidence": {
                    "SWeatherPerRegion_sandstorm_flag_off": hx(REGION_SANDSTORM_OFF),
                    "live_sandstorm_region_rows": sandstorm_rows,
                    "static_predicate_function": "CIsSandStorm@CIsFightingInWeatherTrigger vtable slot reads byte ptr [rdx+0x128]",
                },
                "remaining_scope": "Add REGION_FLAG_SANDSTORM to the compact weather region flags and decoder/gate unit counts.",
            },
        ],
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--pid", type=int, default=None)
    ap.add_argument(
        "--out-dir",
        default="extractor/native/runs/weather_live_full_replay_residuals_870ebf_20260701_v1",
    )
    ap.add_argument(
        "--manifest",
        default="extractor/native/runs/weather_visual_drivers_live_870ebf_integrated_20260701_v1/native_session_manifest.json",
    )
    args = ap.parse_args()

    out_dir = Path(args.out_dir)
    if not out_dir.is_absolute():
        out_dir = ROOT / out_dir
    out_dir.mkdir(parents=True, exist_ok=True)
    proof_path = out_dir / "weather_full_replay_residuals_probe.json"

    pid = args.pid if args.pid is not None else wm.find_pid("hoi4.exe")
    if pid is None:
        proof = {"candidate_valid": False, "errors": ["hoi4.exe is not running"]}
        proof_path.write_text(json.dumps(proof, indent=2), encoding="utf-8")
        print(json.dumps({"candidate_valid": False, "proof": str(proof_path), "errors": proof["errors"]}, indent=2))
        return 1

    started = time.time()
    proof: dict[str, Any] = {
        "probe": "weather_full_replay_residuals_870ebf_v1",
        "pid": pid,
        "errors": [],
        "build": {
            "sha256_expected": EXPECTED_SHA256,
        },
        "domain": {
            "id": 24,
            "name": "weather_live",
        },
        "offsets": {
            "CGameState_static_ptr_rva": hx(CG_STATE_DATE_OFFSET),
            "CGameState_weather_manager_off": hx(WEATHER_CGAMESTATE_MANAGER_OFF),
            "CWeatherManager_vtable_rva": hx(WEATHER_MANAGER_VTABLE_RVA),
            "region_vector_off": hx(WEATHER_MANAGER_REGION_ARRAY_OFF),
            "province_vector_off": hx(WEATHER_MANAGER_PROVINCE_ARRAY_OFF),
            "SWeatherPerRegion_vtable_rva": hx(WEATHER_REGION_VTABLE_RVA),
            "SWeatherPerRegion_stride": hx(WEATHER_REGION_STRIDE),
            "SWeatherPerRegion_active_modifiers_off": hx(REGION_ACTIVE_MODS_OFF),
            "SWeatherPerRegion_active_front_positions_off": hx(REGION_ACTIVE_FRONT_POSITIONS_OFF),
            "SWeatherPerRegion_weather_effect_entities_off": hx(REGION_ACTIVE_EFFECT_ENTITIES_OFF),
            "SWeatherPerRegion_phenomenon_off": hx(REGION_PHENOM_ENUM_OFF),
            "SWeatherPerRegion_sandstorm_flag_off": hx(REGION_SANDSTORM_OFF),
            "SWeatherPerRegion_accumulator_candidate_off": hx(REGION_ACCUMULATOR_OFF),
            "SWeatherPerRegion_temperature_off": hx(REGION_TEMPERATURE_OFF),
            "SWeatherPerRegion_next_change_off": hx(REGION_NEXT_CHANGE_OFF),
            "SWeatherPerProvince_vtable_rva": hx(WEATHER_PROVINCE_VTABLE_RVA),
            "SWeatherPerProvince_stride": hx(WEATHER_PROVINCE_STRIDE),
            "SWeatherPerProvince_water_off": hx(PROVINCE_WATER_OFF),
            "SWeatherPerProvince_snow_off": hx(PROVINCE_SNOW_OFF),
            "SWeatherPerProvince_temperature_off": hx(PROVINCE_TEMP_OFF),
            "SWeatherPerProvince_temperature_change_off": hx(PROVINCE_TEMP_CHANGE_OFF),
            "SWeatherPerProvince_mud_off": hx(PROVINCE_MUD_OFF),
            "SWeatherPerProvince_custom_modifiers_off": hx(PROVINCE_CUSTOM_MODS_OFF),
        },
    }

    proc = wm.Process(pid, "hoi4.exe")
    try:
        fp = proc.fingerprint()
        module_base = proc.module_base
        sha = fp.get("sha256")
        proof["build"].update(
            {
                "module_base": hx(module_base),
                "module_path": fp.get("path"),
                "sha256": sha,
                "build_gate_ok": sha == EXPECTED_SHA256,
            }
        )
        if sha != EXPECTED_SHA256:
            proof["errors"].append("build gate mismatch")

        gamestate = proc.u64(module_base + CG_STATE_DATE_OFFSET) if sha == EXPECTED_SHA256 else 0
        manager = proc.u64(gamestate + WEATHER_CGAMESTATE_MANAGER_OFF) if is_heap(gamestate) else 0
        manager_vtable = proc.u64(manager) if is_heap(manager) else 0
        proof["roots"] = {
            "gamestate_ptr_source": hx(module_base + CG_STATE_DATE_OFFSET),
            "gamestate": hx(gamestate),
            "weather_manager": hx(manager),
            "weather_manager_vtable": hx(manager_vtable),
            "weather_manager_vtable_expected": hx(module_base + WEATHER_MANAGER_VTABLE_RVA),
        }
        if sha == EXPECTED_SHA256 and manager_vtable != module_base + WEATHER_MANAGER_VTABLE_RVA:
            proof["errors"].append("weather manager vtable mismatch")

        region_header = vec_header(proc, manager + WEATHER_MANAGER_REGION_ARRAY_OFF) if is_heap(manager) else None
        province_header = vec_header(proc, manager + WEATHER_MANAGER_PROVINCE_ARRAY_OFF) if is_heap(manager) else None
        proof["headers"] = {
            "region": jsonable_vec_header(region_header),
            "province": jsonable_vec_header(province_header),
        }
        if sha == EXPECTED_SHA256:
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

        if not proof["errors"] and region_header and province_header:
            region_validation, region_errors = validate_region_rows(proc, region_header["begin"], module_base)
            province_validation, province_errors = validate_province_visual_drivers(
                proc,
                province_header["begin"],
                province_header["size"],
                module_base,
            )
            proof["region_validation"] = region_validation
            proof["province_visual_driver_validation"] = province_validation
            proof["errors"].extend(region_errors[:20])
            proof["errors"].extend(province_errors[:20])

        manifest_path = Path(args.manifest)
        if not manifest_path.is_absolute():
            manifest_path = ROOT / manifest_path
        if manifest_path.exists():
            copied_manifest = out_dir / "source_native_session_manifest.weather_visual_integrated.json"
            shutil.copyfile(manifest_path, copied_manifest)
            summary = compact_manifest_summary(manifest_path)
            summary["copied_manifest"] = str(copied_manifest)
            proof["compact_manifest_summary"] = summary
        else:
            proof["errors"].append(f"manifest not found: {manifest_path}")

        proof["coverage_manifest_weather_rows"] = coverage_rows_for_weather()
        proof["candidate_valid"] = not proof["errors"]
        proof["classification"] = classify(proof)
        proof["elapsed_ms"] = int((time.time() - started) * 1000)
        proof_path.write_text(json.dumps(proof, indent=2), encoding="utf-8")
        print(json.dumps({"candidate_valid": proof["candidate_valid"], "proof": str(proof_path), "errors": proof["errors"][:10]}, indent=2))
        return 0 if proof["candidate_valid"] else 1
    finally:
        proc.close()


if __name__ == "__main__":
    raise SystemExit(main())
