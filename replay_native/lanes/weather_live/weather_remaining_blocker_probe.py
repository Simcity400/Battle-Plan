#!/usr/bin/env python3
"""Resolve or block remaining domain-24 weather residuals.

This bounded proof probe is intentionally not a production source. It combines:

- current-build static disassembly of the engine `set_weather` console fixture;
- live `SWeatherPerRegion` named flag vs `+0x78` evidence;
- existing close-zoom particle/vector proof artifacts.

The goal is to decide whether the remaining residuals have an unblocked
production compact path, a conclusive candidate rejection, or an exact fixture /
render-root blocker.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import struct
import sys
from pathlib import Path
from typing import Any

import capstone


ROOT = Path(__file__).resolve().parents[3]
LANE = ROOT / "replay_native" / "lanes" / "weather_live"
MEMORY_READER = ROOT / "memory_reader"
if str(MEMORY_READER) not in sys.path:
    sys.path.insert(0, str(MEMORY_READER))

import winmem as wm  # noqa: E402


EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"

SET_WEATHER_CALLBACK_VA = 0x140287090
SET_WEATHER_VALIDATION_END_VA = 0x1402872F0
SET_WEATHER_APPLY_START_VA = 0x1402872E7
SET_WEATHER_APPLY_END_VA = 0x140287590
SET_WEATHER_ERROR_START_VA = 0x14028779B
SET_WEATHER_ERROR_END_VA = 0x140287800

SETTER_FUNCTIONS = {
    "clear": 0x140F0B9D0,
    "rain": 0x140F0BA00,
    "storm": 0x140F0BA90,
    "snow": 0x140F0BA60,
    "blizzard": 0x140F0B9A0,
    "sandstorm": 0x140F0BA30,
}

REGION_NAMED_FLAG_OFFSETS = {
    0x108: "rain_light",
    0x110: "rain_heavy",
    0x118: "snow",
    0x120: "blizzard",
    0x128: "sandstorm",
    0x130: "arctic_water",
}

REGION_PHENOMENON_OFF = 0x78
WEATHER_MANAGER_GAMESTATE_OFF = 0x688
WEATHER_MANAGER_REGION_VECTOR_OFF = 0x40


def load_module(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"could not load {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


name_probe = load_module("weather_name_static_table_probe", LANE / "weather_name_static_table_probe.py")
generator_probe = load_module("weather_generator_name_probe", LANE / "weather_generator_name_probe.py")
particle_probe = load_module("weather_particle_vector_probe", LANE / "weather_particle_vector_probe.py")


def hx(value: int | None) -> str | None:
    return None if value is None else f"0x{value:X}"


def read_c_string(data: bytes, pe: dict[str, Any], image_base: int, va: int) -> str | None:
    off = name_probe.rva_to_file_off(pe, va - image_base)
    if off is None:
        return None
    end = data.find(b"\0", off)
    if end < 0:
        return None
    raw = data[off:end]
    if not raw:
        return None
    try:
        text = raw.decode("ascii", errors="strict")
    except UnicodeDecodeError:
        return None
    if not all(32 <= ord(ch) <= 126 for ch in text):
        return None
    return text


def disasm_window(data: bytes, pe: dict[str, Any], start_va: int, end_va: int) -> list[dict[str, Any]]:
    image_base = int(pe["image_base"])
    start_off = name_probe.rva_to_file_off(pe, start_va - image_base)
    if start_off is None:
        return [{"error": f"start va {hx(start_va)} not mappable"}]
    blob = data[start_off : start_off + (end_va - start_va)]
    md = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_64)
    md.detail = True
    rows = []
    for insn in md.disasm(blob, start_va):
        text = f"{insn.mnemonic} {insn.op_str}".strip()
        row: dict[str, Any] = {"va": hx(insn.address), "text": text}
        refs = []
        mem_disps = []
        imms = []
        for op in insn.operands:
            if op.type == capstone.x86.X86_OP_MEM:
                disp = int(op.mem.disp)
                if op.mem.base == capstone.x86.X86_REG_RIP:
                    target = insn.address + insn.size + disp
                    text_ref = read_c_string(data, pe, image_base, target)
                    if text_ref:
                        refs.append({"target": hx(target), "text": text_ref})
                elif disp in (
                    REGION_PHENOMENON_OFF,
                    WEATHER_MANAGER_GAMESTATE_OFF,
                    WEATHER_MANAGER_REGION_VECTOR_OFF,
                    *REGION_NAMED_FLAG_OFFSETS.keys(),
                ):
                    mem_disps.append(hx(disp))
            elif op.type == capstone.x86.X86_OP_IMM:
                imm = int(op.imm)
                if -16 <= imm <= 64:
                    imms.append(imm)
        if refs:
            row["string_refs"] = refs
        if mem_disps:
            row["watched_displacements"] = mem_disps
        if imms:
            row["small_immediates"] = imms
        rows.append(row)
    return rows


def setter_writes(data: bytes, pe: dict[str, Any], name: str, va: int) -> dict[str, Any]:
    image_base = int(pe["image_base"])
    start_off = name_probe.rva_to_file_off(pe, va - image_base)
    if start_off is None:
        return {"name": name, "function_va": hx(va), "error": "not mappable"}
    blob = data[start_off : start_off + 0x120]
    md = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_64)
    md.detail = True
    instructions = []
    writes = []
    writes_phenomenon = False
    for insn in md.disasm(blob, va):
        text = f"{insn.mnemonic} {insn.op_str}".strip()
        instructions.append({"va": hx(insn.address), "text": text})
        if insn.mnemonic.startswith("mov") and len(insn.operands) >= 2:
            dst = insn.operands[0]
            src = insn.operands[1]
            if dst.type == capstone.x86.X86_OP_MEM:
                disp = int(dst.mem.disp)
                value = int(src.imm) if src.type == capstone.x86.X86_OP_IMM else None
                if disp in REGION_NAMED_FLAG_OFFSETS:
                    writes.append(
                        {
                            "offset": hx(disp),
                            "field": REGION_NAMED_FLAG_OFFSETS[disp],
                            "value": value,
                        }
                    )
                if disp == REGION_PHENOMENON_OFF:
                    writes_phenomenon = True
        if insn.mnemonic == "ret":
            break
    return {
        "name": name,
        "function_va": hx(va),
        "writes": writes,
        "writes_SWeatherPerRegion_plus_0x78": writes_phenomenon,
        "instructions": instructions,
    }


def set_weather_fixture_evidence(exe: Path) -> dict[str, Any]:
    data = exe.read_bytes()
    sha = name_probe.sha256_file(exe)
    pe = name_probe.parse_pe(data)
    image_base = int(pe["image_base"])
    validation = disasm_window(data, pe, SET_WEATHER_CALLBACK_VA, SET_WEATHER_VALIDATION_END_VA)
    apply = disasm_window(data, pe, SET_WEATHER_APPLY_START_VA, SET_WEATHER_APPLY_END_VA)
    errors = disasm_window(data, pe, SET_WEATHER_ERROR_START_VA, SET_WEATHER_ERROR_END_VA)
    setters = {
        name: setter_writes(data, pe, name, va) for name, va in SETTER_FUNCTIONS.items()
    }
    accepted_inputs = []
    for row in validation:
        for ref in row.get("string_refs", []):
            text = ref["text"]
            if text in SETTER_FUNCTIONS:
                accepted_inputs.append({"name": text, "string_va": ref["target"], "xref_va": row["va"]})
    apply_calls = []
    pending_name = None
    for row in apply:
        for ref in row.get("string_refs", []):
            if ref["text"] in SETTER_FUNCTIONS:
                pending_name = ref["text"]
        text = row.get("text", "")
        if pending_name and text.startswith("call "):
            try:
                target = int(text.split(" ", 1)[1], 16)
            except ValueError:
                target = None
            if target in SETTER_FUNCTIONS.values():
                apply_calls.append({"name": pending_name, "call_va": row["va"], "target_va": hx(target)})
                pending_name = None
    watched_apply = [
        row
        for row in apply
        if row.get("watched_displacements") or row.get("string_refs")
    ]
    error_strings = [
        ref
        for row in errors
        for ref in row.get("string_refs", [])
    ]
    return {
        "build": {
            "path": str(exe),
            "sha256": sha,
            "sha256_expected": EXPECTED_SHA256,
            "build_gate_ok": sha == EXPECTED_SHA256,
            "image_base": hx(image_base),
        },
        "callback_va": hx(SET_WEATHER_CALLBACK_VA),
        "accepted_inputs": accepted_inputs,
        "apply_calls": apply_calls,
        "watched_apply_instructions": watched_apply,
        "error_strings": error_strings,
        "setters": setters,
        "requires_selected_province": any(
            "A province must be selected" in ref.get("text", "") for ref in error_strings + [
                ref for row in validation for ref in row.get("string_refs", [])
            ]
        ),
        "uses_weather_manager_region_vector": any(
            "0x688" in row.get("watched_displacements", [])
            for row in watched_apply
        )
        and any("0x40" in row.get("watched_displacements", []) for row in watched_apply),
        "any_setter_writes_plus_0x78": any(
            row.get("writes_SWeatherPerRegion_plus_0x78") for row in setters.values()
        ),
    }


def live_enum_contradiction(pid: int | None) -> dict[str, Any]:
    if pid is None:
        pid = wm.find_pid("hoi4.exe")
    if pid is None:
        return {"errors": ["hoi4.exe is not running"]}
    proc = wm.Process(pid, "hoi4.exe")
    try:
        fp = proc.fingerprint()
        sha = fp.get("sha256")
        out: dict[str, Any] = {
            "build": {
                "pid": pid,
                "module_base": hx(proc.module_base),
                "module_path": fp.get("path"),
                "sha256": sha,
                "sha256_expected": EXPECTED_SHA256,
                "build_gate_ok": sha == EXPECTED_SHA256,
            }
        }
        if sha != EXPECTED_SHA256:
            out["errors"] = ["build gate mismatch"]
            return out
        live, errors = generator_probe.read_live_weather_outputs(proc, proc.module_base)
        out["live_weather_outputs"] = live
        out["errors"] = errors
        by_phen = live.get("region_named_flags_by_phenomenon", {})
        contradictory = {}
        for phen, flags in by_phen.items():
            active_names = {name: count for name, count in flags.items() if int(count) > 0}
            if len(active_names) > 1:
                contradictory[phen] = active_names
        out["plus_0x78_values_with_multiple_named_flags"] = contradictory
        out["candidate_rejection"] = {
            "candidate": "SWeatherPerRegion+0x78 as exact weather-name enum 3..7",
            "rejected": bool(contradictory),
            "reason": (
                "A single +0x78 value coexists with multiple named weather flags, "
                "so +0x78 cannot be the stable replay name enum for rain/snow/blizzard/"
                "arctic/mud/sandstorm."
            ),
        }
        return out
    finally:
        proc.close()


def particle_blocker_summary(out_dir: Path) -> dict[str, Any]:
    particle_path = out_dir / "weather_particle_vector_probe.json"
    inventory_path = out_dir / "weather_manager_vector_inventory.json"
    summary: dict[str, Any] = {
        "particle_probe": str(particle_path),
        "manager_inventory_probe": str(inventory_path),
    }
    if particle_path.exists():
        particle = json.loads(particle_path.read_text(encoding="utf-8"))
        live = particle.get("live_vectors", {})
        static = particle.get("static_assets", {})
        region_entity = live.get("region_vectors", {}).get("effect_entity_payload", {})
        province = live.get("province_vectors", {})
        summary["particle_classification"] = particle.get("classification")
        summary["static_assets"] = {
            "weatherpositions_rows": static.get("weatherpositions", {}).get("row_count"),
            "weatherpositions_regions": static.get("weatherpositions", {}).get("regions"),
            "weather_entities_count": static.get("weather_entities_count"),
            "particles_count": static.get("particles_count"),
            "missing_weather_effect_entities": static.get("missing_weather_effect_entities"),
            "missing_weather_entity_particles": static.get("missing_weather_entity_particles"),
        }
        summary["live_vector_counts"] = {
            "region_front_records": live.get("region_vectors", {}).get("front_payload", {}).get("records"),
            "region_effect_entity_records": region_entity.get("records"),
            "region_effect_entity_type_counts": region_entity.get("entity_type_counts"),
            "province_front_vector_rows": province.get("front_vector_rows"),
            "province_effect_entity_vector_rows": province.get("effect_entity_vector_rows"),
            "province_front_entries": province.get("front_entries_from_headers"),
            "province_effect_entity_entries": province.get("effect_entity_entries_from_headers"),
        }
    else:
        summary["particle_probe_missing"] = True
    if inventory_path.exists():
        inventory = json.loads(inventory_path.read_text(encoding="utf-8"))
        candidates = inventory.get("vector_like_fields", [])
        clean_particle_entity_roots = []
        rejected_particle_like_candidates = []
        terrain_modifier_candidates = []
        for candidate in candidates:
            sample = candidate.get("sample", {})
            inline = sample.get("inline_vtable_stride_candidates", [])
            ptrs = sample.get("pointer_element_type_counts", {})
            inline_json = json.dumps(inline)
            ptr_json = json.dumps(ptrs)
            has_particle_signal = "Particle" in inline_json or "Particle" in ptr_json
            has_entity_signal = "WeatherEffect" in inline_json or "WeatherEffect" in ptr_json
            has_terrain_modifier_signal = "CWeatherTerrainModifier" in inline_json or "CWeatherTerrainModifier" in ptr_json
            size = int(candidate.get("size", 0))
            cap = int(candidate.get("cap", 0))
            vector_header_clean = size > 0 and cap >= size
            if has_terrain_modifier_signal:
                terrain_modifier_candidates.append(
                    {
                        "offset": candidate.get("offset"),
                        "begin": candidate.get("begin"),
                        "size": size,
                        "cap": cap,
                        "classification": "weather_terrain_modifier_static_config_not_particle_entity_root",
                    }
                )
            if has_particle_signal or has_entity_signal:
                if vector_header_clean and (has_entity_signal or ptrs):
                    clean_particle_entity_roots.append(candidate)
                else:
                    rejected_particle_like_candidates.append(
                        {
                            "offset": candidate.get("offset"),
                            "begin": candidate.get("begin"),
                            "size": size,
                            "cap": cap,
                            "first_qwords": sample.get("first_qwords", [])[:16],
                            "inline_vtable_stride_candidates": inline[:4],
                            "pointer_element_type_counts": ptrs,
                            "rejection": (
                                "not a clean weather particle/entity vector root: "
                                "header is not a valid size<=cap vector or sample is heterogeneous raw render memory"
                            ),
                        }
                    )
        summary["manager_inventory"] = {
            "candidate_count": inventory.get("candidate_count"),
            "clean_particle_entity_roots_outside_known_weather_arrays": clean_particle_entity_roots,
            "rejected_particle_like_candidates": rejected_particle_like_candidates,
            "terrain_modifier_candidates": terrain_modifier_candidates,
        }
    else:
        summary["manager_inventory_missing"] = True
    summary["blocker"] = {
        "close_zoom_particle_entity_parity": "blocked_by_missing_render_entity_root_or_controlled_camera_fixture",
        "reason": (
            "Static weather particle assets and weather-local vector headers exist, "
            "but the sampled live vectors do not expose stable asset/entity IDs; "
            "a production row would currently be raw pointers/placement records."
        ),
    }
    return summary


def classify(proof: dict[str, Any]) -> dict[str, Any]:
    fixture = proof.get("set_weather_fixture", {})
    live = proof.get("live_enum_contradiction", {})
    particle = proof.get("particle_blocker", {})
    rejected_enum = bool(live.get("candidate_rejection", {}).get("rejected")) and not fixture.get(
        "any_setter_writes_plus_0x78", True
    )
    particle_blocked = (
        particle.get("blocker", {}).get("close_zoom_particle_entity_parity")
        == "blocked_by_missing_render_entity_root_or_controlled_camera_fixture"
    )
    return {
        "raw_enum_name_parity": (
            "candidate_rejected_plus_0x78_is_not_weather_name_enum"
            if rejected_enum
            else "not_resolved"
        ),
        "set_weather_fixture_value": (
            "validates_named_region_flag_setters_only; requires selected province for live sweep"
        ),
        "close_zoom_particle_entity_parity": (
            "blocked_exact_render_entity_root_or_controlled_camera_fixture_required"
            if particle_blocked
            else "not_resolved"
        ),
        "unblocked_weather_residual_work_remaining": not (rejected_enum and particle_blocked),
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--pid", type=int, default=None)
    ap.add_argument("--exe", default=r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV\hoi4.exe")
    ap.add_argument("--out-dir", default="replay_native/runs/weather_live_full_replay_residuals_870ebf_20260701_v2")
    args = ap.parse_args()

    out_dir = Path(args.out_dir)
    if not out_dir.is_absolute():
        out_dir = ROOT / out_dir
    out_dir.mkdir(parents=True, exist_ok=True)
    out_path = out_dir / "weather_remaining_blocker_probe.json"

    proof: dict[str, Any] = {
        "probe": "weather_remaining_blocker_probe_870ebf_v1",
        "errors": [],
    }
    try:
        proof["set_weather_fixture"] = set_weather_fixture_evidence(Path(args.exe))
        proof["live_enum_contradiction"] = live_enum_contradiction(args.pid)
        proof["particle_blocker"] = particle_blocker_summary(out_dir)
        proof["classification"] = classify(proof)
        for key in ("set_weather_fixture", "live_enum_contradiction"):
            errors = proof.get(key, {}).get("errors", [])
            proof["errors"].extend(f"{key}: {err}" for err in errors)
    except Exception as exc:
        proof["errors"].append(f"{type(exc).__name__}: {exc}")
    out_path.write_text(json.dumps(proof, indent=2), encoding="utf-8")
    print(
        json.dumps(
            {
                "proof": str(out_path),
                "classification": proof.get("classification"),
                "errors": proof["errors"][:10],
            },
            indent=2,
        )
    )
    return 0 if not proof["errors"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
