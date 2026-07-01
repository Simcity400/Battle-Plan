"""Decode live weather particle vectors and join them to static weather assets.

This is a bounded read-only probe for the close-zoom particle/vector residual of
domain 24. It does not change the collector hot loop. The goal is to prove
whether the vector payloads are structurally serializable and whether the static
asset side needed to render them is present.
"""

from __future__ import annotations

import argparse
import json
import re
import struct
import sys
import time
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
WEATHER_REGION_VTABLE_RVA = 0x2960B90
WEATHER_PROVINCE_VTABLE_RVA = 0x2960B40

WEATHER_MANAGER_PROVINCE_ARRAY_OFF = 0x10
WEATHER_MANAGER_REGION_ARRAY_OFF = 0x40
WEATHER_REGION_STRIDE = 0x160
WEATHER_PROVINCE_STRIDE = 0x180
WEATHER_REGION_COUNT = 305

REGION_ID_OFF = 0x08
REGION_ACTIVE_FRONT_POSITIONS_OFF = 0x58
REGION_ACTIVE_EFFECT_ENTITIES_OFF = 0x70
REGION_PHENOM_ENUM_OFF = 0x78
PROVINCE_ID_OFF = 0x08
PROVINCE_LOCAL_FRONT_POSITIONS_OFF = 0x58
PROVINCE_LOCAL_EFFECT_ENTITIES_OFF = 0x70

HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000
MAX_VECTOR_COUNT = 4096
MAX_PROVINCE_PAYLOAD_SAMPLES = 256

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


def minmax(counters: dict[str, Any], key: str, value: int) -> None:
    lo = f"{key}_min"
    hi = f"{key}_max"
    counters[lo] = value if counters.get(lo) is None else min(counters[lo], value)
    counters[hi] = value if counters.get(hi) is None else max(counters[hi], value)


def read_c_string(proc: wm.Process, addr: int, max_len: int = 256) -> str | None:
    data = proc.try_read(addr, max_len)
    if not data:
        return None
    end = data.find(b"\x00")
    if end < 0:
        end = len(data)
    raw = data[:end]
    if not raw:
        return None
    try:
        text = raw.decode("ascii", errors="strict")
    except UnicodeDecodeError:
        return None
    if not all((ch.isprintable() or ch in "\t") for ch in text):
        return None
    return text


def rtti_name_for_vtable(proc: wm.Process, module_base: int, vtable: int) -> str | None:
    # MSVC x64 stores the CompleteObjectLocator pointer immediately before the vtable.
    if not (module_base <= vtable < module_base + 0x5000000):
        return None
    col_ptr = proc.u64(vtable - 8)
    if not (module_base <= col_ptr < module_base + 0x5000000):
        return None
    col = proc.try_read(col_ptr, 0x18)
    if len(col) < 0x18:
        return None
    sig = u32(col, 0)
    if sig != 1:
        return None
    type_desc_rva = u32(col, 0x0C)
    self_rva = u32(col, 0x14)
    if module_base + self_rva != col_ptr:
        return None
    type_desc = module_base + type_desc_rva
    name = read_c_string(proc, type_desc + 0x10)
    return name


def parse_weatherpositions(path: Path) -> dict[str, Any]:
    rows: list[dict[str, Any]] = []
    by_region: dict[str, dict[str, int]] = {}
    for raw in path.read_text(encoding="utf-8-sig").splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        parts = [part.strip() for part in line.split(";")]
        if len(parts) != 5:
            continue
        rid, x, y, z, size = parts
        row = {
            "region_id": int(rid),
            "x": float(x),
            "y": float(y),
            "z": float(z),
            "size": size,
        }
        rows.append(row)
        entry = by_region.setdefault(rid, {"total": 0, "small": 0, "big": 0})
        entry["total"] += 1
        if size in ("small", "big"):
            entry[size] += 1
    return {
        "path": str(path),
        "row_count": len(rows),
        "regions": len(by_region),
        "by_region_sample": {key: by_region[key] for key in sorted(by_region, key=int)[:12]},
    }


def extract_balanced_blocks(text: str, keyword: str) -> list[str]:
    blocks: list[str] = []
    for match in re.finditer(rf"\b{re.escape(keyword)}\s*=\s*\{{", text):
        start = match.start()
        brace = text.find("{", match.start())
        depth = 0
        for idx in range(brace, len(text)):
            if text[idx] == "{":
                depth += 1
            elif text[idx] == "}":
                depth -= 1
                if depth == 0:
                    blocks.append(text[start : idx + 1])
                    break
    return blocks


def parse_weather_effects(path: Path) -> dict[str, Any]:
    text = path.read_text(encoding="utf-8-sig")
    effects: dict[str, list[dict[str, Any]]] = {}
    weather_blocks = extract_balanced_blocks(text, "weather_effects")
    if not weather_blocks:
        return {"path": str(path), "effects": effects, "error": "weather_effects block not found"}
    block = weather_blocks[0]
    for phenomenon in PHENOMENA:
        sub_blocks = extract_balanced_blocks(block, phenomenon)
        if not sub_blocks:
            continue
        items = []
        for eff in extract_balanced_blocks(sub_blocks[0], "weather_effect"):
            entity = re.search(r"\bentity\s*=\s*([A-Za-z0-9_]+)", eff)
            fading = re.search(r"\bfading_entity\s*=\s*([A-Za-z0-9_]+)", eff)
            small = re.search(r"\bsmall\s*=\s*yes\b", eff)
            if entity:
                items.append(
                    {
                        "entity": entity.group(1),
                        "fading_entity": fading.group(1) if fading else None,
                        "small": bool(small),
                    }
                )
        effects[phenomenon] = items
    return {"path": str(path), "effects": effects}


def parse_entity_particles(path: Path) -> dict[str, Any]:
    text = path.read_text(encoding="utf-8-sig")
    entities: dict[str, list[str]] = {}
    for block in extract_balanced_blocks(text, "entity"):
        name = re.search(r'\bname\s*=\s*"([^"]+)"', block)
        if not name:
            continue
        particles = re.findall(r'\bparticle\s*=\s*"([^"]+)"', block)
        entities[name.group(1)] = particles
    return {"path": str(path), "entities": entities}


def parse_particles(path: Path) -> dict[str, Any]:
    text = path.read_text(encoding="utf-8-sig")
    particles = set(re.findall(r'\bname\s*=\s*"([^"]+)"', text))
    return {"path": str(path), "particles": sorted(particles)}


def static_asset_summary(game_root: Path) -> dict[str, Any]:
    positions = parse_weatherpositions(game_root / "map" / "weatherpositions.txt")
    effects = parse_weather_effects(game_root / "common" / "weather.txt")
    entities = parse_entity_particles(game_root / "gfx" / "entities" / "weather_entities.asset")
    particles = parse_particles(game_root / "gfx" / "entities" / "particles.gfx")
    entity_names = set(entities["entities"].keys())
    particle_names = set(particles["particles"])
    missing_entities = []
    missing_particles = []
    for phenomenon, rows in effects.get("effects", {}).items():
        for row in rows:
            for key in ("entity", "fading_entity"):
                entity = row.get(key)
                if entity and entity not in entity_names:
                    missing_entities.append({"phenomenon": phenomenon, "entity": entity})
                if entity and entity in entities["entities"]:
                    for particle in entities["entities"][entity]:
                        if particle not in particle_names:
                            missing_particles.append({"entity": entity, "particle": particle})
    return {
        "weatherpositions": positions,
        "weather_effects": effects,
        "weather_entities_count": len(entity_names),
        "particles_count": len(particle_names),
        "missing_weather_effect_entities": missing_entities,
        "missing_weather_entity_particles": missing_particles,
    }


def decode_front_payload(proc: wm.Process, vec: dict[str, Any]) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    counters: dict[str, Any] = {
        "records": 0,
        "second_dword_nonzero": 0,
        "sign_counts": {},
        "sample_records": [],
    }
    samples: list[dict[str, Any]] = []
    if not vec["begin_heap"] or vec["size"] == 0:
        return counters, samples
    data = read_exact(proc, vec["begin"], int(vec["size"]) * 16)
    if data is None:
        counters["read_error"] = True
        return counters, samples
    for idx in range(int(vec["size"])):
        off = idx * 16
        local_idx, zeroish, signed_offset, sign = struct.unpack_from("<IIii", data, off)
        bump(counters, "records")
        if zeroish != 0:
            bump(counters, "second_dword_nonzero")
        minmax(counters, "local_idx", int(local_idx))
        minmax(counters, "signed_offset", int(signed_offset))
        sign_counts = counters["sign_counts"]
        sign_counts[str(sign)] = int(sign_counts.get(str(sign), 0)) + 1
        rec = {
            "local_idx": local_idx,
            "zeroish": zeroish,
            "signed_offset": signed_offset,
            "sign": sign,
        }
        if len(counters["sample_records"]) < 12:
            counters["sample_records"].append(rec)
        if len(samples) < 12:
            samples.append(rec)
    return counters, samples


def decode_entity_payload(
    proc: wm.Process, module_base: int, vec: dict[str, Any]
) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    counters: dict[str, Any] = {
        "records": 0,
        "entity_type_counts": {},
        "arg_a_counts": {},
        "arg_b_counts": {},
        "sample_records": [],
    }
    samples: list[dict[str, Any]] = []
    if not vec["begin_heap"] or vec["size"] == 0:
        return counters, samples
    data = read_exact(proc, vec["begin"], int(vec["size"]) * 16)
    if data is None:
        counters["read_error"] = True
        return counters, samples
    for idx in range(int(vec["size"])):
        off = idx * 16
        ptr, arg_a, arg_b = struct.unpack_from("<QII", data, off)
        bump(counters, "records")
        arg_a_counts = counters["arg_a_counts"]
        arg_b_counts = counters["arg_b_counts"]
        arg_a_counts[str(arg_a)] = int(arg_a_counts.get(str(arg_a), 0)) + 1
        arg_b_counts[str(arg_b)] = int(arg_b_counts.get(str(arg_b), 0)) + 1
        object_head = read_exact(proc, ptr, 8) if is_heap(ptr) else None
        vtable = u64(object_head, 0) if object_head is not None else 0
        if object_head is None:
            bump(counters, "unreadable_entity_ptrs")
        type_name = rtti_name_for_vtable(proc, module_base, vtable) if vtable else None
        type_key = type_name or hx(vtable) or "unknown"
        type_counts = counters["entity_type_counts"]
        type_counts[type_key] = int(type_counts.get(type_key, 0)) + 1
        rec = {
            "entity_ptr": hx(ptr),
            "entity_vtable": hx(vtable),
            "entity_type": type_name,
            "arg_a": arg_a,
            "arg_b": arg_b,
        }
        if len(counters["sample_records"]) < 12:
            counters["sample_records"].append(rec)
        if len(samples) < 12:
            samples.append(rec)
    return counters, samples


def merge_counters(dst: dict[str, Any], src: dict[str, Any], prefix: str = "") -> None:
    for key, value in src.items():
        if key in ("sample_records", "sign_counts", "entity_type_counts", "arg_a_counts", "arg_b_counts"):
            continue
        if key.endswith("_min") or key.endswith("_max"):
            cur = dst.get(prefix + key)
            if cur is None:
                dst[prefix + key] = value
            elif key.endswith("_min"):
                dst[prefix + key] = min(cur, value)
            else:
                dst[prefix + key] = max(cur, value)
        elif isinstance(value, int):
            bump(dst, prefix + key, value)


def read_weather_vectors(proc: wm.Process, module_base: int) -> tuple[dict[str, Any], list[str]]:
    errors: list[str] = []
    gamestate = proc.u64(module_base + CG_STATE_DATE_OFFSET)
    manager = proc.u64(gamestate + WEATHER_CGAMESTATE_MANAGER_OFF) if is_heap(gamestate) else 0
    manager_vtable = proc.u64(manager) if is_heap(manager) else 0
    region_header = read_exact(proc, manager + WEATHER_MANAGER_REGION_ARRAY_OFF, 16) if is_heap(manager) else None
    province_header = read_exact(proc, manager + WEATHER_MANAGER_PROVINCE_ARRAY_OFF, 16) if is_heap(manager) else None
    if manager_vtable != module_base + WEATHER_MANAGER_VTABLE_RVA:
        errors.append("weather manager vtable mismatch")
    if not region_header or not province_header:
        errors.append("weather vector header read failed")
        return {}, errors
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
        return {}, ["bulk weather vector read failed"]

    proof: dict[str, Any] = {
        "roots": {
            "gamestate": hx(gamestate),
            "weather_manager": hx(manager),
            "weather_manager_vtable": hx(manager_vtable),
            "region_begin": hx(region_begin),
            "region_size": region_size,
            "province_begin": hx(province_begin),
            "province_size": province_size,
        },
        "region_vectors": {
            "active_rows": 0,
            "front_vector_rows": 0,
            "effect_entity_vector_rows": 0,
            "front_payload": {"records": 0, "second_dword_nonzero": 0, "sample_records": []},
            "effect_entity_payload": {"records": 0, "sample_records": []},
            "sample_rows": [],
        },
        "province_vectors": {
            "front_vector_rows": 0,
            "effect_entity_vector_rows": 0,
            "front_entries_from_headers": 0,
            "effect_entity_entries_from_headers": 0,
            "front_payload_sampled": {"records": 0, "second_dword_nonzero": 0, "sample_records": []},
            "effect_entity_payload_sampled": {"records": 0, "sample_records": []},
            "sample_rows": [],
        },
    }

    region_front_all = proof["region_vectors"]["front_payload"]
    region_entity_all = proof["region_vectors"]["effect_entity_payload"]
    for idx in range(region_size):
        body = region_data[idx * WEATHER_REGION_STRIDE : (idx + 1) * WEATHER_REGION_STRIDE]
        vtable = u64(body, 0)
        rid = u32(body, REGION_ID_OFF)
        phen = u32(body, REGION_PHENOM_ENUM_OFF)
        if vtable != module_base + WEATHER_REGION_VTABLE_RVA:
            errors.append(f"region {idx} vtable mismatch")
            continue
        front = vec_from_body(body, REGION_ACTIVE_FRONT_POSITIONS_OFF)
        entity = vec_from_body(body, REGION_ACTIVE_EFFECT_ENTITIES_OFF)
        if phen >= 1:
            bump(proof["region_vectors"], "active_rows")
        if front["begin_heap"]:
            bump(proof["region_vectors"], "front_vector_rows")
        if entity["begin_heap"]:
            bump(proof["region_vectors"], "effect_entity_vector_rows")
        front_payload, _ = decode_front_payload(proc, front)
        entity_payload, _ = decode_entity_payload(proc, module_base, entity)
        merge_counters(region_front_all, front_payload)
        merge_counters(region_entity_all, entity_payload)
        for map_key in ("sign_counts",):
            merged = region_front_all.setdefault(map_key, {})
            for key, value in front_payload.get(map_key, {}).items():
                merged[key] = int(merged.get(key, 0)) + int(value)
        for map_key in ("entity_type_counts", "arg_a_counts", "arg_b_counts"):
            merged = region_entity_all.setdefault(map_key, {})
            for key, value in entity_payload.get(map_key, {}).items():
                merged[key] = int(merged.get(key, 0)) + int(value)
        if len(region_front_all["sample_records"]) < 12:
            region_front_all["sample_records"].extend(front_payload.get("sample_records", [])[: 12 - len(region_front_all["sample_records"])])
        if len(region_entity_all["sample_records"]) < 12:
            region_entity_all["sample_records"].extend(entity_payload.get("sample_records", [])[: 12 - len(region_entity_all["sample_records"])])
        if (front["begin_heap"] or entity["begin_heap"]) and len(proof["region_vectors"]["sample_rows"]) < 12:
            proof["region_vectors"]["sample_rows"].append(
                {
                    "region_id": rid,
                    "phenomenon": phen,
                    "phenomenon_name_file_order": PHENOMENA[phen] if phen < len(PHENOMENA) else f"phenom_{phen}",
                    "front": {"begin": front["begin_hex"], "size": front["size"], "cap": front["cap"]},
                    "effect_entities": {"begin": entity["begin_hex"], "size": entity["size"], "cap": entity["cap"]},
                }
            )

    sampled_front_payloads = 0
    sampled_entity_payloads = 0
    province_front_all = proof["province_vectors"]["front_payload_sampled"]
    province_entity_all = proof["province_vectors"]["effect_entity_payload_sampled"]
    for idx in range(province_size):
        body = province_data[idx * WEATHER_PROVINCE_STRIDE : (idx + 1) * WEATHER_PROVINCE_STRIDE]
        vtable = u64(body, 0)
        pid = u32(body, PROVINCE_ID_OFF)
        if vtable != module_base + WEATHER_PROVINCE_VTABLE_RVA:
            if len(errors) < 20:
                errors.append(f"province {idx} vtable mismatch")
            continue
        front = vec_from_body(body, PROVINCE_LOCAL_FRONT_POSITIONS_OFF)
        entity = vec_from_body(body, PROVINCE_LOCAL_EFFECT_ENTITIES_OFF)
        if front["begin_heap"]:
            bump(proof["province_vectors"], "front_vector_rows")
            bump(proof["province_vectors"], "front_entries_from_headers", int(front["size"]))
        if entity["begin_heap"]:
            bump(proof["province_vectors"], "effect_entity_vector_rows")
            bump(proof["province_vectors"], "effect_entity_entries_from_headers", int(entity["size"]))
        if front["begin_heap"] and sampled_front_payloads < MAX_PROVINCE_PAYLOAD_SAMPLES:
            payload, _ = decode_front_payload(proc, front)
            merge_counters(province_front_all, payload)
            merged = province_front_all.setdefault("sign_counts", {})
            for key, value in payload.get("sign_counts", {}).items():
                merged[key] = int(merged.get(key, 0)) + int(value)
            if len(province_front_all["sample_records"]) < 12:
                province_front_all["sample_records"].extend(payload.get("sample_records", [])[: 12 - len(province_front_all["sample_records"])])
            sampled_front_payloads += 1
        if entity["begin_heap"] and sampled_entity_payloads < MAX_PROVINCE_PAYLOAD_SAMPLES:
            payload, _ = decode_entity_payload(proc, module_base, entity)
            merge_counters(province_entity_all, payload)
            for map_key in ("entity_type_counts", "arg_a_counts", "arg_b_counts"):
                merged = province_entity_all.setdefault(map_key, {})
                for key, value in payload.get(map_key, {}).items():
                    merged[key] = int(merged.get(key, 0)) + int(value)
            if len(province_entity_all["sample_records"]) < 12:
                province_entity_all["sample_records"].extend(payload.get("sample_records", [])[: 12 - len(province_entity_all["sample_records"])])
            sampled_entity_payloads += 1
        if (front["begin_heap"] or entity["begin_heap"]) and len(proof["province_vectors"]["sample_rows"]) < 12:
            proof["province_vectors"]["sample_rows"].append(
                {
                    "province_id": pid,
                    "front": {"begin": front["begin_hex"], "size": front["size"], "cap": front["cap"]},
                    "effect_entities": {"begin": entity["begin_hex"], "size": entity["size"], "cap": entity["cap"]},
                }
            )

    proof["province_vectors"]["payload_sample_limit"] = MAX_PROVINCE_PAYLOAD_SAMPLES
    return proof, errors


def classify(proof: dict[str, Any]) -> dict[str, Any]:
    live = proof.get("live_vectors", {})
    static = proof.get("static_assets", {})
    errors = proof.get("errors", [])
    assets_ok = not static.get("missing_weather_effect_entities") and not static.get("missing_weather_entity_particles")
    region_vectors = live.get("region_vectors", {})
    region_front_payload = region_vectors.get("front_payload", {})
    region_entity_payload = region_vectors.get("effect_entity_payload", {})
    region_front_records = region_front_payload.get("records", 0)
    region_entity_records = region_entity_payload.get("records", 0)
    province_front_rows = live.get("province_vectors", {}).get("front_vector_rows", 0)
    province_entity_rows = live.get("province_vectors", {}).get("effect_entity_vector_rows", 0)
    entity_type_counts = region_entity_payload.get("entity_type_counts", {})
    clean_weather_entity_records = sum(
        int(count)
        for type_name, count in entity_type_counts.items()
        if isinstance(type_name, str) and ("weather" in type_name.lower() or "particle" in type_name.lower())
    )
    blocker_state = (
        "static_assets_and_vector_headers_validated_but_stable_particle_entity_semantics_blocked"
        if not errors and assets_ok and (region_front_records > 0 or province_front_rows > 0)
        else "not_resolved"
    )
    return {
        "close_zoom_particle_vector_contents": blocker_state,
        "static_weather_particle_assets": "validated" if assets_ok else "missing_static_assets",
        "region_payload_shape": {
            "front_records": region_front_records,
            "effect_entity_records": region_entity_records,
            "front_record_layout": "16-byte raw records; stable vector size/stride validated, semantic fields not production-named",
            "effect_entity_record_layout": "16-byte raw records; pointer-like first qword is usually CModifier or unreadable, not a stable weather entity id",
            "effect_entity_type_counts": entity_type_counts,
            "clean_weather_or_particle_entity_records": clean_weather_entity_records,
        },
        "province_payload_scope": {
            "front_vector_rows": province_front_rows,
            "effect_entity_vector_rows": province_entity_rows,
            "payload_samples_per_kind": MAX_PROVINCE_PAYLOAD_SAMPLES,
        },
        "production_gap": (
            "Close-zoom static weather assets and weather-local vector headers are present, but the live "
            "+0x58/+0x70 payloads do not expose stable weather particle/entity ids. A production compact tail "
            "would currently serialize pointers/raw records, so it is blocked until a render/entity root or "
            "controlled fixture maps these records to stable asset names and placements."
        ),
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--pid", type=int, default=None)
    ap.add_argument("--game-root", default=str(GAME_ROOT_DEFAULT))
    ap.add_argument(
        "--out-dir",
        default="replay_native/runs/weather_live_full_replay_residuals_870ebf_20260701_v2",
    )
    args = ap.parse_args()

    out_dir = Path(args.out_dir)
    if not out_dir.is_absolute():
        out_dir = ROOT / out_dir
    out_dir.mkdir(parents=True, exist_ok=True)
    proof_path = out_dir / "weather_particle_vector_probe.json"
    started = time.time()

    proof: dict[str, Any] = {
        "probe": "weather_particle_vector_probe_870ebf_v1",
        "domain": {"id": 24, "name": "weather_live"},
        "errors": [],
        "game_root": str(Path(args.game_root)),
    }
    pid = args.pid if args.pid is not None else wm.find_pid("hoi4.exe")
    if pid is None:
        proof["errors"].append("hoi4.exe is not running")
        proof["classification"] = classify(proof)
        proof_path.write_text(json.dumps(proof, indent=2), encoding="utf-8")
        return 1

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
        proof["static_assets"] = static_asset_summary(Path(args.game_root))
        if not proof["errors"]:
            live_vectors, errors = read_weather_vectors(proc, module_base)
            proof["live_vectors"] = live_vectors
            proof["errors"].extend(errors)
        proof["classification"] = classify(proof)
        proof["elapsed_ms"] = int((time.time() - started) * 1000)
        proof_path.write_text(json.dumps(proof, indent=2), encoding="utf-8")
        print(json.dumps({"proof": str(proof_path), "classification": proof["classification"], "errors": proof["errors"][:10]}, indent=2))
        return 0 if not proof["errors"] else 1
    finally:
        proc.close()


if __name__ == "__main__":
    raise SystemExit(main())
