#!/usr/bin/env python3
"""Emit a lane-local domain-24 v3 weather payload with the sandstorm flag.

This is a bounded proof emitter. It reads the same live roots as the production
collector and writes a compact domain payload for the lane-local Rust gate. It
is not a hot-loop replay source and does not replace the DLL collector.
"""

from __future__ import annotations

import argparse
import json
import struct
import sys
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[3]
LANE = Path(__file__).resolve().parent
if str(LANE) not in sys.path:
    sys.path.insert(0, str(LANE))

import weather_full_replay_residuals_probe as w  # noqa: E402


DOMAIN_ID = 24
DOMAIN_NAME = "weather_live"
HEADER_LEN = 48
REGION_ROW_LEN = 32
PROVINCE_ROW_LEN = 32
FIXED_SCALE_RAW = 100_000

REGION_FLAG_RAIN_LIGHT = 1
REGION_FLAG_RAIN_HEAVY = 1 << 1
REGION_FLAG_SNOW = 1 << 2
REGION_FLAG_BLIZZARD = 1 << 3
REGION_FLAG_ARCTIC_WATER = 1 << 4
REGION_FLAG_ACTIVE_SIGNAL = 1 << 5
REGION_FLAG_PHENOMENON_ACTIVE = 1 << 6
REGION_FLAG_SANDSTORM = 1 << 7

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


def u32le(value: int) -> bytes:
    return struct.pack("<I", value & 0xFFFFFFFF)


def i32le(value: int) -> bytes:
    return struct.pack("<i", int(value))


def u64le(value: int) -> bytes:
    return struct.pack("<Q", value & 0xFFFFFFFFFFFFFFFF)


def province_visual_flags(
    temperature_present: bool,
    temp_change_present: bool,
    water_present: bool,
    snow_present: bool,
    mud_present: bool,
    mud_active: bool,
    custom_modifier_count_present: bool,
) -> int:
    flags = PROVINCE_FLAG_STRUCT_VALIDATED
    if temperature_present:
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
    if custom_modifier_count_present:
        flags |= PROVINCE_FLAG_CUSTOM_MODIFIER_COUNT_PRESENT
    if temperature_present or water_present or snow_present:
        flags |= PROVINCE_FLAG_MAP_RECOLOR_DRIVER
    if snow_present or mud_present:
        flags |= PROVINCE_FLAG_DYNAMIC_LUT_DRIVER
    return flags


def encode_region_row(body: bytes) -> tuple[bytes, dict[str, Any]]:
    region_id = w.u32(body, w.REGION_ID_OFF)
    phenomenon = w.u32(body, w.REGION_PHENOM_ENUM_OFF)
    front_vec = w.vec_from_body(body, w.REGION_ACTIVE_FRONT_POSITIONS_OFF)
    entity_vec = w.vec_from_body(body, w.REGION_ACTIVE_EFFECT_ENTITIES_OFF)
    active_mods = w.vec_from_body(body, w.REGION_ACTIVE_MODS_OFF)
    flags = 0
    named_flags = {
        "rain_light": w.boolish(body, w.REGION_RAIN_LIGHT_OFF),
        "rain_heavy": w.boolish(body, w.REGION_RAIN_HEAVY_OFF),
        "snow": w.boolish(body, w.REGION_SNOW_OFF),
        "blizzard": w.boolish(body, w.REGION_BLIZZARD_OFF),
        "sandstorm": w.boolish(body, w.REGION_SANDSTORM_OFF),
        "arctic_water": w.boolish(body, w.REGION_ARCTIC_WATER_OFF),
    }
    if named_flags["rain_light"]:
        flags |= REGION_FLAG_RAIN_LIGHT
    if named_flags["rain_heavy"]:
        flags |= REGION_FLAG_RAIN_HEAVY
    if named_flags["snow"]:
        flags |= REGION_FLAG_SNOW
    if named_flags["blizzard"]:
        flags |= REGION_FLAG_BLIZZARD
    if named_flags["arctic_water"]:
        flags |= REGION_FLAG_ARCTIC_WATER
    if named_flags["sandstorm"]:
        flags |= REGION_FLAG_SANDSTORM
    active_signal = bool(front_vec["begin_heap"] or entity_vec["begin_heap"])
    if active_signal:
        flags |= REGION_FLAG_ACTIVE_SIGNAL
    if phenomenon >= 1:
        flags |= REGION_FLAG_PHENOMENON_ACTIVE
    row = b"".join(
        [
            u32le(region_id),
            u32le(flags),
            i32le(w.i32(body, w.REGION_TEMPERATURE_OFF)),
            u32le(phenomenon),
            u32le(w.u32(body, w.REGION_NEXT_CHANGE_OFF)),
            u32le(int(active_mods["size"])),
            u32le(body[w.REGION_INTENSITY_OFF]),
            u32le(0),
        ]
    )
    return row, {
        "region_id": region_id,
        "phenomenon": phenomenon,
        "flags": flags,
        "named_flags": named_flags,
        "active_signal": active_signal,
    }


def encode_province_row(body: bytes) -> tuple[bytes, dict[str, Any]]:
    province_id = w.u32(body, w.PROVINCE_ID_OFF)
    water_raw = w.i32(body, w.PROVINCE_WATER_OFF)
    snow_raw = w.i32(body, w.PROVINCE_SNOW_OFF)
    temperature_raw = w.i32(body, w.PROVINCE_TEMP_OFF)
    temperature_change_raw = w.i32(body, w.PROVINCE_TEMP_CHANGE_OFF)
    mud = w.boolish(body, w.PROVINCE_MUD_OFF)
    custom_vec = w.vec_from_body(body, w.PROVINCE_CUSTOM_MODS_OFF)
    temperature_present = temperature_raw != 0
    temp_change_present = temperature_present
    flags = province_visual_flags(
        temperature_present=temperature_present,
        temp_change_present=temp_change_present,
        water_present=True,
        snow_present=True,
        mud_present=True,
        mud_active=bool(mud),
        custom_modifier_count_present=True,
    )
    row = b"".join(
        [
            u32le(province_id),
            u32le(flags),
            i32le(temperature_raw if temperature_present else 0),
            i32le(temperature_change_raw if temp_change_present else 0),
            i32le(water_raw),
            i32le(snow_raw),
            u32le(int(custom_vec["size"])),
            u32le(0),
        ]
    )
    return row, {
        "province_id": province_id,
        "flags": flags,
        "temperature_present": temperature_present,
        "mud_active": bool(mud),
        "custom_modifier_count": int(custom_vec["size"]),
    }


def emit_payload(proc: w.wm.Process) -> tuple[bytes, dict[str, Any]]:
    module_base = proc.module_base
    gamestate = proc.u64(module_base + w.CG_STATE_DATE_OFFSET)
    manager = proc.u64(gamestate + w.WEATHER_CGAMESTATE_MANAGER_OFF) if w.is_heap(gamestate) else 0
    manager_vtable = proc.u64(manager) if w.is_heap(manager) else 0
    if manager_vtable != module_base + w.WEATHER_MANAGER_VTABLE_RVA:
        raise RuntimeError("weather manager vtable mismatch")
    region_header = w.vec_header(proc, manager + w.WEATHER_MANAGER_REGION_ARRAY_OFF)
    province_header = w.vec_header(proc, manager + w.WEATHER_MANAGER_PROVINCE_ARRAY_OFF)
    if not region_header or int(region_header["size"]) != w.WEATHER_REGION_COUNT:
        raise RuntimeError("weather region header mismatch")
    if not province_header or not (w.MIN_PROVINCE_ROWS <= int(province_header["size"]) <= w.MAX_PROVINCE_ROWS):
        raise RuntimeError("weather province header mismatch")

    region_bytes = w.read_exact(
        proc,
        int(region_header["begin"]),
        int(region_header["size"]) * w.WEATHER_REGION_STRIDE,
    )
    province_bytes = w.read_exact(
        proc,
        int(province_header["begin"]),
        int(province_header["size"]) * w.WEATHER_PROVINCE_STRIDE,
    )
    if region_bytes is None or province_bytes is None:
        raise RuntimeError("weather array read failed")

    region_rows = bytearray()
    province_rows = bytearray()
    sandstorm_rows = 0
    active_region_rows = 0
    sample_regions = []
    for idx in range(int(region_header["size"])):
        body = region_bytes[idx * w.WEATHER_REGION_STRIDE : (idx + 1) * w.WEATHER_REGION_STRIDE]
        row, summary = encode_region_row(body)
        region_rows.extend(row)
        if summary["flags"] & REGION_FLAG_SANDSTORM:
            sandstorm_rows += 1
        if summary["phenomenon"] >= 1:
            active_region_rows += 1
        if len(sample_regions) < 8 or summary["flags"] & REGION_FLAG_SANDSTORM:
            sample_regions.append(summary)

    province_sample_count = 0
    mud_active_rows = 0
    for idx in range(int(province_header["size"])):
        body = province_bytes[idx * w.WEATHER_PROVINCE_STRIDE : (idx + 1) * w.WEATHER_PROVINCE_STRIDE]
        row, summary = encode_province_row(body)
        province_rows.extend(row)
        if summary["mud_active"]:
            mud_active_rows += 1
        if province_sample_count < 4:
            province_sample_count += 1

    region_count = int(region_header["size"])
    province_count = int(province_header["size"])
    changed_rows = region_count + province_count
    header = b"".join(
        [
            u64le(0),
            u32le(region_count),
            u32le(changed_rows),
            u32le(0),
            u32le(province_count),
            u32le(region_count),
            u32le(province_count),
            u32le(FIXED_SCALE_RAW),
            u32le(active_region_rows),
            u32le(0),
            u32le(0),
        ]
    )
    payload = bytes(header + region_rows + province_rows)
    manifest = {
        "domain_id": DOMAIN_ID,
        "domain_name": DOMAIN_NAME,
        "schema": "weather_live.domain24.sandstorm_v3.payload.v1",
        "payload_kind": "domain_data_payload_only",
        "payload_bytes": len(payload),
        "header_len": HEADER_LEN,
        "region_row_len": REGION_ROW_LEN,
        "province_row_len": PROVINCE_ROW_LEN,
        "region_rows": region_count,
        "province_rows": province_count,
        "changed_rows": changed_rows,
        "active_region_rows": active_region_rows,
        "sandstorm_region_rows": sandstorm_rows,
        "mud_active_rows": mud_active_rows,
        "roots": {
            "gamestate": w.hx(gamestate),
            "weather_manager": w.hx(manager),
            "region_begin": w.hx(int(region_header["begin"])),
            "province_begin": w.hx(int(province_header["begin"])),
        },
        "offsets": {
            "SWeatherPerRegion_sandstorm_flag_off": w.hx(w.REGION_SANDSTORM_OFF),
        },
        "sample_regions": sample_regions,
    }
    return payload, manifest


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--pid", type=int, default=None)
    ap.add_argument("--out-dir", default="replay_native/runs/weather_live_full_replay_residuals_870ebf_20260701_v2")
    args = ap.parse_args()
    out_dir = Path(args.out_dir)
    if not out_dir.is_absolute():
        out_dir = ROOT / out_dir
    out_dir.mkdir(parents=True, exist_ok=True)
    payload_path = out_dir / "domain24_weather_sandstorm_v3_payload.bin"
    manifest_path = out_dir / "weather_sandstorm_v3_payload_manifest.json"

    pid = args.pid if args.pid is not None else w.wm.find_pid("hoi4.exe")
    if pid is None:
        manifest_path.write_text(json.dumps({"errors": ["hoi4.exe is not running"]}, indent=2), encoding="utf-8")
        return 1

    proc = w.wm.Process(pid, "hoi4.exe")
    try:
        fp = proc.fingerprint()
        if fp.get("sha256") != w.EXPECTED_SHA256:
            raise RuntimeError("build gate mismatch")
        payload, manifest = emit_payload(proc)
        manifest["build"] = {
            "pid": pid,
            "module_base": w.hx(proc.module_base),
            "module_path": fp.get("path"),
            "sha256": fp.get("sha256"),
            "sha256_expected": w.EXPECTED_SHA256,
            "build_gate_ok": True,
        }
        payload_path.write_bytes(payload)
        manifest["payload"] = str(payload_path)
        manifest_path.write_text(json.dumps(manifest, indent=2), encoding="utf-8")
        print(json.dumps({
            "payload": str(payload_path),
            "manifest": str(manifest_path),
            "payload_bytes": len(payload),
            "sandstorm_region_rows": manifest["sandstorm_region_rows"],
        }, indent=2))
        return 0 if manifest["sandstorm_region_rows"] > 0 else 1
    finally:
        proc.close()


if __name__ == "__main__":
    raise SystemExit(main())
