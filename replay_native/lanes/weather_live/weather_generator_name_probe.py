"""Bounded generator/RNG/name-parity probe for domain 24 weather residuals.

This probe proves what is already resolved for replay extraction and records the
remaining exact-engine-generator blockers. It is read-only: static file parsing,
current process reads, and executable string/RTTI scans only.
"""

from __future__ import annotations

import argparse
import json
import re
import struct
import sys
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[3]
MEMORY_READER = ROOT / "memory_reader"
if str(MEMORY_READER) not in sys.path:
    sys.path.insert(0, str(MEMORY_READER))

import winmem as wm  # noqa: E402

GAME_ROOT_DEFAULT = Path("X:/SteamLibrary/steamapps/common/Hearts of Iron IV")
EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"

CG_STATE_DATE_OFFSET = 0x3316E90
WEATHER_CGAMESTATE_MANAGER_OFF = 0x688
WEATHER_MANAGER_VTABLE_RVA = 0x2960BE0
WEATHER_MANAGER_REGION_ARRAY_OFF = 0x40
WEATHER_MANAGER_PROVINCE_ARRAY_OFF = 0x10
WEATHER_REGION_STRIDE = 0x160
WEATHER_PROVINCE_STRIDE = 0x180
WEATHER_REGION_COUNT = 305
WEATHER_REGION_VTABLE_RVA = 0x2960B90
WEATHER_PROVINCE_VTABLE_RVA = 0x2960B40

REGION_ID_OFF = 0x08
REGION_PHENOM_ENUM_OFF = 0x78
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

HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000
PHENOMENA = [
    "no_phenomenon",
    "rain_light",
    "rain_heavy",
    "snow",
    "blizzard",
    "arctic_water",
    "mud",
    "sandstorm",
]


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


def read_exact(proc: wm.Process, addr: int, size: int) -> bytes | None:
    data = proc.try_read(addr, size)
    return data if len(data) == size else None


def extract_ascii_hits(blob: bytes, needles: list[str]) -> dict[str, list[int]]:
    out: dict[str, list[int]] = {}
    for needle in needles:
        raw = needle.encode("ascii")
        hits = []
        start = 0
        while True:
            idx = blob.find(raw, start)
            if idx < 0:
                break
            hits.append(idx)
            start = idx + 1
            if len(hits) >= 16:
                break
        out[needle] = hits
    return out


def extract_type_names(blob: bytes, pattern: str) -> list[str]:
    # MSVC TypeDescriptor names are null-terminated decorated ASCII strings.
    names = set()
    for match in re.finditer(rb"\.\?[AVU][A-Za-z0-9_@$?]+@@", blob):
        raw = match.group(0).decode("ascii", errors="ignore")
        if re.search(pattern, raw, flags=re.IGNORECASE):
            names.add(raw)
    return sorted(names)


def parse_weather_static_order(path: Path) -> dict[str, Any]:
    text = path.read_text(encoding="utf-8-sig")
    effects_block = re.search(r"weather_effects\s*=\s*\{", text)
    effect_names = []
    if effects_block:
        for name in PHENOMENA:
            if re.search(rf"\b{re.escape(name)}\s*=\s*\{{", text[effects_block.start() :]):
                effect_names.append(name)
    durations = []
    duration_block = re.search(r"duration\s*=\s*\{(?P<body>.*?)\n\s*\}", text, re.DOTALL)
    if duration_block:
        for name in PHENOMENA:
            if re.search(rf"\b{re.escape(name)}\s*=", duration_block.group("body")):
                durations.append(name)
    return {
        "path": str(path),
        "file_order_hypothesis": {str(idx): name for idx, name in enumerate(PHENOMENA)},
        "weather_effect_names_present": effect_names,
        "duration_names_present": durations,
        "temperature_settings_present": "temperature_variation" in text and "max_temperature_change" in text,
        "rng_formula_settings_present": {
            "temperature_variation": "temperature_variation" in text,
            "max_temperature_change": "max_temperature_change" in text,
            "duration_random_interval_comment": "Random interval" in text,
        },
    }


def read_live_weather_outputs(proc: wm.Process, module_base: int) -> tuple[dict[str, Any], list[str]]:
    errors: list[str] = []
    gamestate = proc.u64(module_base + CG_STATE_DATE_OFFSET)
    manager = proc.u64(gamestate + WEATHER_CGAMESTATE_MANAGER_OFF) if is_heap(gamestate) else 0
    manager_vtable = proc.u64(manager) if is_heap(manager) else 0
    if manager_vtable != module_base + WEATHER_MANAGER_VTABLE_RVA:
        return {}, ["weather manager vtable mismatch"]
    region_header = read_exact(proc, manager + WEATHER_MANAGER_REGION_ARRAY_OFF, 16)
    province_header = read_exact(proc, manager + WEATHER_MANAGER_PROVINCE_ARRAY_OFF, 16)
    if region_header is None or province_header is None:
        return {}, ["weather vector header read failed"]
    region_begin, region_size, region_cap = struct.unpack("<QII", region_header)
    province_begin, province_size, province_cap = struct.unpack("<QII", province_header)
    if region_size != WEATHER_REGION_COUNT or region_cap != WEATHER_REGION_COUNT:
        errors.append("region vector header invalid")
    if province_size < 10_000 or province_cap < province_size:
        errors.append("province vector header invalid")
    if errors:
        return {}, errors
    region_data = read_exact(proc, region_begin, region_size * WEATHER_REGION_STRIDE)
    province_data = read_exact(proc, province_begin, province_size * WEATHER_PROVINCE_STRIDE)
    if region_data is None or province_data is None:
        return {}, ["bulk weather array read failed"]

    phen_counts: dict[str, int] = {}
    named_flag_by_phen: dict[str, dict[str, int]] = {}
    for idx in range(region_size):
        body = region_data[idx * WEATHER_REGION_STRIDE : (idx + 1) * WEATHER_REGION_STRIDE]
        if u64(body, 0) != module_base + WEATHER_REGION_VTABLE_RVA:
            errors.append(f"region {idx} vtable mismatch")
            continue
        if u32(body, REGION_ID_OFF) != idx:
            errors.append(f"region {idx} id mismatch")
            continue
        phen = u32(body, REGION_PHENOM_ENUM_OFF)
        phen_counts[str(phen)] = phen_counts.get(str(phen), 0) + 1
        flags = named_flag_by_phen.setdefault(str(phen), {
            "rain_light": 0,
            "rain_heavy": 0,
            "snow": 0,
            "blizzard": 0,
            "sandstorm": 0,
            "arctic_water": 0,
        })
        for name, off in [
            ("rain_light", REGION_RAIN_LIGHT_OFF),
            ("rain_heavy", REGION_RAIN_HEAVY_OFF),
            ("snow", REGION_SNOW_OFF),
            ("blizzard", REGION_BLIZZARD_OFF),
            ("sandstorm", REGION_SANDSTORM_OFF),
            ("arctic_water", REGION_ARCTIC_WATER_OFF),
        ]:
            if body[off] != 0:
                flags[name] += 1

    province = {
        "rows": province_size,
        "temperature_rows": 0,
        "temperature_change_rows": 0,
        "water_rows": 0,
        "snow_rows": 0,
        "mud_measured_rows": 0,
        "mud_active_rows": 0,
        "temp_raw_min": None,
        "temp_raw_max": None,
        "water_raw_min": None,
        "water_raw_max": None,
        "snow_raw_min": None,
        "snow_raw_max": None,
    }
    for idx in range(province_size):
        body = province_data[idx * WEATHER_PROVINCE_STRIDE : (idx + 1) * WEATHER_PROVINCE_STRIDE]
        if u64(body, 0) != module_base + WEATHER_PROVINCE_VTABLE_RVA:
            if len(errors) < 20:
                errors.append(f"province {idx} vtable mismatch")
            continue
        if u32(body, PROVINCE_ID_OFF) != idx:
            if len(errors) < 20:
                errors.append(f"province {idx} id mismatch")
            continue
        temp = i32(body, PROVINCE_TEMP_OFF)
        temp_change = i32(body, PROVINCE_TEMP_CHANGE_OFF)
        water = i32(body, PROVINCE_WATER_OFF)
        snow = i32(body, PROVINCE_SNOW_OFF)
        mud = body[PROVINCE_MUD_OFF]
        if temp != 0:
            province["temperature_rows"] += 1
            province["temperature_change_rows"] += 1 if temp_change != 0 else 0
        if water >= 0:
            province["water_rows"] += 1
        if snow >= 0:
            province["snow_rows"] += 1
        if mud in (0, 1):
            province["mud_measured_rows"] += 1
            if mud == 1:
                province["mud_active_rows"] += 1
        for key, value in [("temp_raw", temp), ("water_raw", water), ("snow_raw", snow)]:
            lo = f"{key}_min"
            hi = f"{key}_max"
            province[lo] = value if province[lo] is None else min(province[lo], value)
            province[hi] = value if province[hi] is None else max(province[hi], value)
    return {
        "roots": {
            "gamestate": hx(gamestate),
            "weather_manager": hx(manager),
            "region_begin": hx(region_begin),
            "province_begin": hx(province_begin),
        },
        "region_phenomenon_counts": phen_counts,
        "region_named_flags_by_phenomenon": named_flag_by_phen,
        "province_generated_outputs": province,
    }, errors


def compact_manifest_weather_units(path: Path) -> dict[str, Any]:
    if not path.exists():
        return {"error": f"manifest not found: {path}"}
    manifest = json.loads(path.read_text(encoding="utf-8"))
    weather = manifest.get("compact_stream", {}).get("per_domain", {}).get("weather_live", {})
    return {
        "manifest": str(path),
        "game_hours_advanced": manifest.get("game_hours_advanced"),
        "skipped_game_hours": manifest.get("skipped_game_hours"),
        "hourly_requirement": manifest.get("hourly_requirement"),
        "failure_flags": {
            "source_unavailable_seen": manifest.get("compact_stream", {}).get("source_unavailable_seen"),
            "partial_data_seen": manifest.get("compact_stream", {}).get("partial_data_seen"),
            "bridge_unavailable_seen": manifest.get("compact_stream", {}).get("bridge_unavailable_seen"),
            "worker_panic_seen": manifest.get("compact_stream", {}).get("worker_panic_seen"),
        },
        "weather_units": weather.get("data_units_total"),
    }


def classify(proof: dict[str, Any]) -> dict[str, Any]:
    compact_units = (proof.get("compact_manifest", {}) or {}).get("weather_units", {}) or {}
    province = (proof.get("live_weather_outputs", {}) or {}).get("province_generated_outputs", {}) or {}
    direct_outputs_captured = bool(
        compact_units.get("weather_province_temp_rows")
        and compact_units.get("weather_province_water_rows")
        and compact_units.get("weather_province_snow_rows")
        and compact_units.get("weather_province_mud_measured_rows")
        and province.get("temperature_rows")
    )
    observed_values = set((proof.get("live_weather_outputs", {}) or {}).get("region_phenomenon_counts", {}).keys())
    unobserved = [str(idx) for idx in range(len(PHENOMENA)) if str(idx) not in observed_values]
    return {
        "generated_weather_outputs_for_replay": "resolved_by_direct_capture" if direct_outputs_captured else "not_resolved",
        "exact_temperature_generator_rng": (
            "not_required_for_replay_frames_because_outputs_are_captured; "
            "blocked_for_exact_future_determinism_without controlled RNG/update-function fixture"
        ),
        "phenomenon_name_parity": (
            "partial_current_frame_only"
            if unobserved
            else "all_enum_values_observed"
        ),
        "unobserved_phenomenon_values_this_frame": unobserved,
        "required_fixture_to_finish_names": (
            "controlled single-player set_weather sweep or static Ghidra trace of CIs* weather predicate compares"
            if unobserved
            else None
        ),
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--pid", type=int, default=None)
    ap.add_argument("--game-root", default=str(GAME_ROOT_DEFAULT))
    ap.add_argument(
        "--manifest",
        default="replay_native/runs/weather_visual_drivers_live_870ebf_integrated_20260701_v1/native_session_manifest.json",
    )
    ap.add_argument(
        "--out-dir",
        default="replay_native/runs/weather_live_full_replay_residuals_870ebf_20260701_v2",
    )
    args = ap.parse_args()

    out_dir = Path(args.out_dir)
    if not out_dir.is_absolute():
        out_dir = ROOT / out_dir
    out_dir.mkdir(parents=True, exist_ok=True)
    out_path = out_dir / "weather_generator_name_probe.json"
    proof: dict[str, Any] = {"probe": "weather_generator_name_probe_870ebf_v1", "errors": []}

    game_root = Path(args.game_root)
    exe_path = game_root / "hoi4.exe"
    exe_bytes = exe_path.read_bytes()
    proof["static_weather"] = parse_weather_static_order(game_root / "common" / "weather.txt")
    proof["exe_string_hits"] = extract_ascii_hits(exe_bytes, PHENOMENA + ["temperature_variation", "Random", "Rng", "Seed"])
    proof["weather_trigger_type_names"] = extract_type_names(
        exe_bytes,
        r"(Weather|Rain|Snow|Blizzard|Sand|Mud|TemperatureTrigger|ArticWater)",
    )
    proof["rng_type_name_candidates"] = extract_type_names(exe_bytes, r"(Random|Rng|Rand|Seed|Mersenne|PRNG)")
    manifest_path = Path(args.manifest)
    if not manifest_path.is_absolute():
        manifest_path = ROOT / manifest_path
    proof["compact_manifest"] = compact_manifest_weather_units(manifest_path)

    pid = args.pid if args.pid is not None else wm.find_pid("hoi4.exe")
    if pid is None:
        proof["errors"].append("hoi4.exe is not running")
    else:
        proc = wm.Process(pid, "hoi4.exe")
        try:
            fp = proc.fingerprint()
            module_base = proc.module_base
            sha = fp.get("sha256")
            proof["build"] = {
                "pid": pid,
                "module_base": hx(module_base),
                "module_path": fp.get("path"),
                "sha256": sha,
                "sha256_expected": EXPECTED_SHA256,
                "build_gate_ok": sha == EXPECTED_SHA256,
            }
            if sha != EXPECTED_SHA256:
                proof["errors"].append("build gate mismatch")
            if sha == EXPECTED_SHA256:
                live_outputs, errors = read_live_weather_outputs(proc, module_base)
                proof["live_weather_outputs"] = live_outputs
                proof["errors"].extend(errors)
        finally:
            proc.close()

    proof["classification"] = classify(proof)
    out_path.write_text(json.dumps(proof, indent=2), encoding="utf-8")
    print(json.dumps({"proof": str(out_path), "classification": proof["classification"], "errors": proof["errors"][:10]}, indent=2))
    return 0 if not proof["errors"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
