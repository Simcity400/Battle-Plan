"""Lane-local live probe for the 870ebf SWeatherPerProvince candidate.

This probe is read-only. It validates the current weather-manager root, the
candidate CWeatherManager+0x10 province vector, the SWeatherPerProvince vtable
candidate, stride, ids, and proposed visual-driver offsets. When the candidate
passes, it writes a domain-24 v2 full-snapshot payload with nonzero province rows
for the lane-local Rust gate.
"""

from __future__ import annotations

import argparse
import json
import os
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
MIN_PROVINCE_ROWS = 10000
MAX_PROVINCE_ROWS = 65536

REGION_ID_OFF = 0x08
REGION_ACTIVE_MODS_OFF = 0x30
REGION_INTENSITY_OFF = 0x5D
REGION_RAW_ACTIVITY_OFF = 0x78
REGION_RAIN_LIGHT_OFF = 0x108
REGION_RAIN_HEAVY_OFF = 0x110
REGION_SNOW_OFF = 0x118
REGION_BLIZZARD_OFF = 0x120
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

FIXED_SCALE_RAW = 100000
MAX_TEMP_RAW = 10_000_000
MAX_TEMP_CHANGE_RAW = 10_000_000
MAX_PRECIP_RAW = 100_000_000
MAX_VECTOR_COUNT = 4096

REGION_FLAG_RAIN_LIGHT = 1
REGION_FLAG_RAIN_HEAVY = 1 << 1
REGION_FLAG_SNOW = 1 << 2
REGION_FLAG_BLIZZARD = 1 << 3
REGION_FLAG_ARCTIC_WATER = 1 << 4
REGION_FLAG_ACTIVE_SIGNAL = 1 << 5
REGION_FLAG_PHENOMENON_ACTIVE = 1 << 6

PROVINCE_FLAG_TEMP_PRESENT = 1
PROVINCE_FLAG_TEMP_CHANGE_PRESENT = 1 << 1
PROVINCE_FLAG_WATER_PRESENT = 1 << 2
PROVINCE_FLAG_SNOW_PRESENT = 1 << 3
PROVINCE_FLAG_MUD_PRESENT = 1 << 4
PROVINCE_FLAG_MUD_ACTIVE = 1 << 5
PROVINCE_FLAG_CUSTOM_MODIFIER_COUNT_PRESENT = 1 << 6
PROVINCE_FLAG_STRUCT_VALIDATED = 1 << 7
PROVINCE_FLAG_MAP_RECOLOR_DRIVER = 1 << 8
PROVINCE_FLAG_DYNAMIC_LUT_DRIVER = 1 << 9

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


def vec_header(proc: wm.Process, addr: int) -> dict | None:
    data = read_exact(proc, addr, 16)
    if data is None:
        return None
    begin, size, cap = struct.unpack("<QII", data)
    return {"begin": begin, "size": size, "cap": cap}


def vec_count_from_body(body: bytes, off: int) -> tuple[int, bool]:
    if off + 16 > len(body):
        return 0, False
    begin = u64(body, off)
    size = u32(body, off + 8)
    cap = u32(body, off + 12)
    if size == 0:
        return 0, True
    # Some weather modifier vectors are valid with size=1 and cap=0 on 870ebf.
    # The prior validated reader used begin+size only for this field, so the
    # production guard checks that non-empty vectors point at readable heap and
    # keep a sane bounded size instead of treating cap as authoritative.
    if not is_heap(begin) or size > MAX_VECTOR_COUNT:
        return size, False
    return size, True


def bool_byte(body: bytes, off: int, counters: dict, label: str) -> bool:
    value = body[off]
    if value not in (0, 1):
        counters[f"{label}_non_bool"] = counters.get(f"{label}_non_bool", 0) + 1
    return value != 0


def province_flags(
    temp_present: bool,
    temp_change_present: bool,
    water_present: bool,
    snow_present: bool,
    mud_present: bool,
    mud_active: bool,
    custom_present: bool,
) -> int:
    flags = PROVINCE_FLAG_STRUCT_VALIDATED
    if temp_present:
        flags |= PROVINCE_FLAG_TEMP_PRESENT
    if temp_change_present:
        flags |= PROVINCE_FLAG_TEMP_CHANGE_PRESENT
    if water_present:
        flags |= PROVINCE_FLAG_WATER_PRESENT
    if snow_present:
        flags |= PROVINCE_FLAG_SNOW_PRESENT
    if mud_present:
        flags |= PROVINCE_FLAG_MUD_PRESENT
    if mud_active:
        flags |= PROVINCE_FLAG_MUD_ACTIVE
    if custom_present:
        flags |= PROVINCE_FLAG_CUSTOM_MODIFIER_COUNT_PRESENT
    if temp_present or water_present or snow_present:
        flags |= PROVINCE_FLAG_MAP_RECOLOR_DRIVER
    if snow_present or mud_present:
        flags |= PROVINCE_FLAG_DYNAMIC_LUT_DRIVER
    return flags


def decode_region_rows(proc: wm.Process, base: int, module_base: int) -> tuple[list[dict], dict, list[str]]:
    errors: list[str] = []
    counters: dict[str, int] = {}
    region_vtable = module_base + WEATHER_REGION_VTABLE_RVA
    rows: list[dict] = []
    span = WEATHER_REGION_COUNT * WEATHER_REGION_STRIDE
    data = read_exact(proc, base, span)
    if data is None:
        errors.append(f"could not bulk-read region span {hx(base)} len {span}")
        return rows, counters, errors

    for idx in range(WEATHER_REGION_COUNT):
        body = data[idx * WEATHER_REGION_STRIDE : (idx + 1) * WEATHER_REGION_STRIDE]
        vtable = u64(body, 0)
        region_id = u32(body, REGION_ID_OFF)
        if vtable == region_vtable:
            counters["region_vtable_match"] = counters.get("region_vtable_match", 0) + 1
        else:
            errors.append(f"region {idx} vtable {hx(vtable)} != {hx(region_vtable)}")
        if region_id == idx:
            counters["region_id_match"] = counters.get("region_id_match", 0) + 1
        else:
            errors.append(f"region {idx} id {region_id} != index")
        phenomenon = u32(body, REGION_RAW_ACTIVITY_OFF)
        active_signal = is_heap(u64(body, 0x58)) or is_heap(u64(body, 0x70))
        active_mods, active_mods_valid = vec_count_from_body(body, REGION_ACTIVE_MODS_OFF)
        if active_mods_valid:
            counters["region_active_mod_vec_valid"] = counters.get("region_active_mod_vec_valid", 0) + 1
        else:
            errors.append(f"region {idx} active-modifier vector invalid")
        flags = 0
        flag_fields = [
            (REGION_RAIN_LIGHT_OFF, REGION_FLAG_RAIN_LIGHT, "rain_light"),
            (REGION_RAIN_HEAVY_OFF, REGION_FLAG_RAIN_HEAVY, "rain_heavy"),
            (REGION_SNOW_OFF, REGION_FLAG_SNOW, "snow"),
            (REGION_BLIZZARD_OFF, REGION_FLAG_BLIZZARD, "blizzard"),
            (REGION_ARCTIC_WATER_OFF, REGION_FLAG_ARCTIC_WATER, "arctic_water"),
        ]
        for off, bit, label in flag_fields:
            if bool_byte(body, off, counters, f"region_{label}"):
                flags |= bit
        if active_signal:
            flags |= REGION_FLAG_ACTIVE_SIGNAL
        if phenomenon >= 1:
            flags |= REGION_FLAG_PHENOMENON_ACTIVE
        if (phenomenon >= 1) != active_signal:
            errors.append(f"region {idx} phenomenon/active-signal mismatch")
        row = {
            "region_id": region_id,
            "flags": flags,
            "temperature_raw": i32(body, REGION_TEMPERATURE_OFF),
            "phenomenon": phenomenon,
            "next_weather_change_raw": u32(body, REGION_NEXT_CHANGE_OFF),
            "active_modifier_count": active_mods,
            "intensity": body[REGION_INTENSITY_OFF],
        }
        rows.append(row)
    return rows, counters, errors


def decode_province_rows(proc: wm.Process, base: int, count: int, module_base: int) -> tuple[list[dict], dict, list[str]]:
    errors: list[str] = []
    counters: dict[str, int] = {}
    province_vtable = module_base + WEATHER_PROVINCE_VTABLE_RVA
    rows: list[dict] = []
    span = count * WEATHER_PROVINCE_STRIDE
    data = read_exact(proc, base, span)
    if data is None:
        errors.append(f"could not bulk-read province span {hx(base)} len {span}")
        return rows, counters, errors

    water_min = snow_min = temp_min = temp_change_min = None
    water_max = snow_max = temp_max = temp_change_max = None
    sample_rows: list[dict] = []
    interesting: dict[str, dict] = {}

    for idx in range(count):
        body = data[idx * WEATHER_PROVINCE_STRIDE : (idx + 1) * WEATHER_PROVINCE_STRIDE]
        vtable = u64(body, 0)
        id_u16 = u16(body, PROVINCE_ID_OFF)
        id_u32 = u32(body, PROVINCE_ID_OFF)
        if vtable == province_vtable:
            counters["province_vtable_match"] = counters.get("province_vtable_match", 0) + 1
        else:
            if counters.get("province_vtable_mismatch", 0) < 10:
                errors.append(f"province {idx} vtable {hx(vtable)} != {hx(province_vtable)}")
            counters["province_vtable_mismatch"] = counters.get("province_vtable_mismatch", 0) + 1
        if id_u16 == idx:
            counters["province_id_u16_match"] = counters.get("province_id_u16_match", 0) + 1
        else:
            if counters.get("province_id_u16_mismatch", 0) < 10:
                errors.append(f"province {idx} id_u16 {id_u16} != index")
            counters["province_id_u16_mismatch"] = counters.get("province_id_u16_mismatch", 0) + 1
        if id_u32 == idx:
            counters["province_id_u32_match"] = counters.get("province_id_u32_match", 0) + 1
        water_raw = i32(body, PROVINCE_WATER_OFF)
        snow_raw = i32(body, PROVINCE_SNOW_OFF)
        temp_raw = i32(body, PROVINCE_TEMP_OFF)
        temp_change_raw = i32(body, PROVINCE_TEMP_CHANGE_OFF)
        mud_byte = body[PROVINCE_MUD_OFF]
        custom_count, custom_valid = vec_count_from_body(body, PROVINCE_CUSTOM_MODS_OFF)

        water_min = water_raw if water_min is None else min(water_min, water_raw)
        water_max = water_raw if water_max is None else max(water_max, water_raw)
        snow_min = snow_raw if snow_min is None else min(snow_min, snow_raw)
        snow_max = snow_raw if snow_max is None else max(snow_max, snow_raw)
        temp_min = temp_raw if temp_min is None else min(temp_min, temp_raw)
        temp_max = temp_raw if temp_max is None else max(temp_max, temp_raw)
        temp_change_min = temp_change_raw if temp_change_min is None else min(temp_change_min, temp_change_raw)
        temp_change_max = temp_change_raw if temp_change_max is None else max(temp_change_max, temp_change_raw)

        if temp_raw != 0:
            counters["temperature_nonzero"] = counters.get("temperature_nonzero", 0) + 1
            interesting.setdefault("temperature_nonzero", {"province_id": idx, "temperature_raw": temp_raw})
        if water_raw > 0:
            counters["water_positive"] = counters.get("water_positive", 0) + 1
            interesting.setdefault("water_positive", {"province_id": idx, "water_raw": water_raw})
        if water_raw == 0:
            counters["water_zero_measured"] = counters.get("water_zero_measured", 0) + 1
        if snow_raw > 0:
            counters["snow_positive"] = counters.get("snow_positive", 0) + 1
            interesting.setdefault("snow_positive", {"province_id": idx, "snow_raw": snow_raw})
        if snow_raw == 0:
            counters["snow_zero_measured"] = counters.get("snow_zero_measured", 0) + 1
        if mud_byte == 1:
            counters["mud_active"] = counters.get("mud_active", 0) + 1
            interesting.setdefault("mud_active", {"province_id": idx})
        elif mud_byte == 0:
            counters["mud_false_measured"] = counters.get("mud_false_measured", 0) + 1
        else:
            if counters.get("mud_non_bool", 0) < 10:
                errors.append(f"province {idx} mud byte {mud_byte} is not bool")
            counters["mud_non_bool"] = counters.get("mud_non_bool", 0) + 1
        if custom_valid:
            counters["custom_vector_valid"] = counters.get("custom_vector_valid", 0) + 1
        else:
            if counters.get("custom_vector_invalid", 0) < 10:
                errors.append(f"province {idx} custom vector invalid count={custom_count}")
            counters["custom_vector_invalid"] = counters.get("custom_vector_invalid", 0) + 1
        if custom_count > 0:
            counters["custom_modifier_positive"] = counters.get("custom_modifier_positive", 0) + 1
            interesting.setdefault("custom_modifier_positive", {"province_id": idx, "count": custom_count})

        if not (-MAX_TEMP_RAW <= temp_raw <= MAX_TEMP_RAW):
            errors.append(f"province {idx} temperature_raw {temp_raw} out of bounds")
        if not (-MAX_TEMP_CHANGE_RAW <= temp_change_raw <= MAX_TEMP_CHANGE_RAW):
            errors.append(f"province {idx} temperature_change_raw {temp_change_raw} out of bounds")
        if not (0 <= water_raw <= MAX_PRECIP_RAW):
            errors.append(f"province {idx} water_raw {water_raw} out of bounds")
        if not (0 <= snow_raw <= MAX_PRECIP_RAW):
            errors.append(f"province {idx} snow_raw {snow_raw} out of bounds")

        temp_present = temp_raw != 0
        flags = province_flags(
            temp_present=temp_present,
            temp_change_present=temp_present,
            water_present=water_raw >= 0,
            snow_present=snow_raw >= 0,
            mud_present=mud_byte in (0, 1),
            mud_active=mud_byte == 1,
            custom_present=custom_valid,
        )
        row = {
            "province_id": idx,
            "flags": flags,
            "temperature_raw": temp_raw if temp_present else 0,
            "temperature_change_raw": temp_change_raw if temp_present else 0,
            "water_raw": water_raw,
            "snow_raw": snow_raw,
            "custom_modifier_count": custom_count if custom_valid else 0,
        }
        rows.append(row)
        if len(sample_rows) < 8 or idx in (31, 3946, 7250, 10167):
            sample_rows.append(
                {
                    "province_id": idx,
                    "vtable": hx(vtable),
                    "id_u16": id_u16,
                    "id_u32": id_u32,
                    "water_raw": water_raw,
                    "snow_raw": snow_raw,
                    "temperature_raw": temp_raw,
                    "temperature_change_raw": temp_change_raw,
                    "mud": mud_byte,
                    "custom_modifier_count": custom_count,
                    "custom_vector_valid": custom_valid,
                    "flags": flags,
                }
            )

    counters["water_raw_min"] = water_min
    counters["water_raw_max"] = water_max
    counters["snow_raw_min"] = snow_min
    counters["snow_raw_max"] = snow_max
    counters["temperature_raw_min"] = temp_min
    counters["temperature_raw_max"] = temp_max
    counters["temperature_change_raw_min"] = temp_change_min
    counters["temperature_change_raw_max"] = temp_change_max
    counters["sample_rows"] = sample_rows
    counters["interesting_rows"] = interesting
    return rows, counters, errors


def write_payload(path: Path, region_rows: list[dict], province_rows: list[dict], status_flags: int) -> None:
    active_region_count = sum(1 for row in region_rows if row["phenomenon"] >= 1)
    changed_rows = len(region_rows) + len(province_rows)
    with path.open("wb") as f:
        f.write(struct.pack("<Q", 0))
        f.write(struct.pack("<I", len(region_rows)))
        f.write(struct.pack("<I", changed_rows))
        f.write(struct.pack("<I", status_flags))
        f.write(struct.pack("<I", len(province_rows)))
        f.write(struct.pack("<I", WEATHER_REGION_COUNT))
        f.write(struct.pack("<I", len(province_rows)))
        f.write(struct.pack("<I", FIXED_SCALE_RAW))
        f.write(struct.pack("<I", active_region_count))
        f.write(struct.pack("<I", 0))
        f.write(struct.pack("<I", 0))
        for row in region_rows:
            f.write(struct.pack("<IIiIIII", row["region_id"], row["flags"], row["temperature_raw"],
                                row["phenomenon"], row["next_weather_change_raw"],
                                row["active_modifier_count"], row["intensity"]))
            f.write(struct.pack("<I", 0))
        for row in province_rows:
            f.write(struct.pack("<IIiiiiiI", row["province_id"], row["flags"], row["temperature_raw"],
                                row["temperature_change_raw"], row["water_raw"], row["snow_raw"],
                                row["custom_modifier_count"], 0))


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--pid", type=int, default=None)
    ap.add_argument("--out-dir", default="extractor/native/runs/weather_visual_drivers_live_870ebf_20260701_v2")
    args = ap.parse_args()

    pid = args.pid if args.pid is not None else wm.find_pid("hoi4.exe")
    if pid is None:
        raise SystemExit("hoi4.exe is not running")
    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    payload_path = out_dir / "domain24_weather_visual_v2_payload.bin"
    proof_path = out_dir / "weather_province_probe_870ebf.json"

    started = time.time()
    proc = wm.Process(pid, "hoi4.exe")
    try:
        fp = proc.fingerprint()
        module_base = proc.module_base
        sha = fp.get("sha256")
        proof: dict = {
            "probe": "weather_visual_drivers_live_province_candidate_870ebf_v1",
            "pid": pid,
            "module_base": hx(module_base),
            "module_path": fp.get("path"),
            "sha256": sha,
            "sha256_expected": EXPECTED_SHA256,
            "build_gate_ok": sha == EXPECTED_SHA256,
            "candidate": {
                "manager_vtable_rva": hx(WEATHER_MANAGER_VTABLE_RVA),
                "region_vtable_rva": hx(WEATHER_REGION_VTABLE_RVA),
                "province_vtable_rva": hx(WEATHER_PROVINCE_VTABLE_RVA),
                "province_vector_off": hx(WEATHER_MANAGER_PROVINCE_ARRAY_OFF),
                "province_stride": hx(WEATHER_PROVINCE_STRIDE),
                "water_off": hx(PROVINCE_WATER_OFF),
                "snow_off": hx(PROVINCE_SNOW_OFF),
                "temperature_off": hx(PROVINCE_TEMP_OFF),
                "temperature_change_off": hx(PROVINCE_TEMP_CHANGE_OFF),
                "mud_off": hx(PROVINCE_MUD_OFF),
                "custom_modifiers_off": hx(PROVINCE_CUSTOM_MODS_OFF),
            },
            "errors": [],
        }
        if sha != EXPECTED_SHA256:
            proof["errors"].append("build gate mismatch")
            proof_path.write_text(json.dumps(proof, indent=2), encoding="utf-8")
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

        region_rows: list[dict] = []
        province_rows: list[dict] = []
        region_errors: list[str] = []
        province_errors: list[str] = []
        if region_header and not proof["errors"]:
            region_rows, region_counters, region_errors = decode_region_rows(proc, region_header["begin"], module_base)
            proof["region_validation"] = {
                "rows": len(region_rows),
                "counters": region_counters,
                "errors_first_20": region_errors[:20],
            }
            proof["errors"].extend(region_errors[:20])
        if province_header and not proof["errors"]:
            province_rows, province_counters, province_errors = decode_province_rows(
                proc, province_header["begin"], province_header["size"], module_base
            )
            proof["province_validation"] = {
                "rows": len(province_rows),
                "counters": province_counters,
                "errors_first_20": province_errors[:20],
            }
            proof["errors"].extend(province_errors[:20])

        candidate_valid = (
            not proof["errors"]
            and len(region_rows) == WEATHER_REGION_COUNT
            and province_header is not None
            and len(province_rows) == province_header["size"]
            and proof.get("province_validation", {}).get("counters", {}).get("province_vtable_match") == province_header["size"]
            and proof.get("province_validation", {}).get("counters", {}).get("province_id_u16_match") == province_header["size"]
            and proof.get("province_validation", {}).get("counters", {}).get("custom_vector_valid") == province_header["size"]
            and proof.get("province_validation", {}).get("counters", {}).get("mud_non_bool", 0) == 0
        )
        proof["candidate_valid"] = candidate_valid
        if candidate_valid:
            write_payload(payload_path, region_rows, province_rows, status_flags=32)
            proof["payload"] = {
                "path": str(payload_path),
                "region_rows": len(region_rows),
                "province_rows": len(province_rows),
                "bytes": payload_path.stat().st_size,
                "status_flags": 32,
                "changed_rows": len(region_rows) + len(province_rows),
            }
        else:
            proof["payload"] = None
        proof["elapsed_ms"] = int((time.time() - started) * 1000)
        proof_path.write_text(json.dumps(proof, indent=2), encoding="utf-8")
        print(json.dumps({
            "candidate_valid": candidate_valid,
            "proof": str(proof_path),
            "payload": str(payload_path) if candidate_valid else None,
            "province_rows": len(province_rows),
            "errors": proof["errors"][:10],
        }, indent=2))
        return 0 if candidate_valid else 1
    finally:
        proc.close()


if __name__ == "__main__":
    raise SystemExit(main())
