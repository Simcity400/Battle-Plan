#!/usr/bin/env python3
"""Bounded target disassembly proof for air-resolution hook candidates.

The broad static hook probe identifies RTTI/vtable candidates. This companion
tool records exact .pdata containment, instruction snippets, direct calls/jumps,
and air-offset evidence for those candidate RVAs without depending on Ghidra
function boundaries.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import struct
import time
from pathlib import Path
from typing import Any

EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
SCHEMA = "air_resolution_target_disasm_v1"
DEFAULT_EXE = Path(r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV\hoi4.exe")

TARGETS: list[tuple[str, int, str]] = [
    ("DailyUpdate_tbb_vtable_slot1", 0xC57A50, "RTTI start_for DailyUpdate<CStrategicAirManager> vtable slot 1"),
    ("HourlyUpdate_lambda1_vtable_slot1", 0xC57960, "RTTI HourlyUpdateThreadedInternal lambda_1 vtable slot 1"),
    ("HourlyUpdate_lambda2_vtable_slot1", 0xC57AA0, "RTTI HourlyUpdateThreadedInternal lambda_2 vtable slot 1"),
    ("HourlyUpdate_lambda3_vtable_slot1", 0xC57B40, "RTTI HourlyUpdateThreadedInternal lambda_3 vtable slot 1"),
    ("HourlyUpdate_lambda4_vtable_slot1", 0xC57BE0, "RTTI HourlyUpdateThreadedInternal lambda_4 vtable slot 1"),
    ("HourlyUpdate_lambda5_vtable_slot1", 0xC57C30, "RTTI HourlyUpdateThreadedInternal lambda_5 vtable slot 1"),
    ("HourlyPhase1_call_target", 0xC34C80, "Called by HourlyUpdate lambda_1 wrapper"),
    ("DailyPhase_call_target", 0xC350D0, "Called by DailyUpdate TBB wrapper"),
    ("HourlyPhase2_call_target", 0xC35240, "Called by HourlyUpdate lambda_2 wrapper"),
    ("HourlyPhase3_call_target", 0xC35520, "Called by HourlyUpdate lambda_3 wrapper"),
    ("HourlyPhase4_call_target", 0xC35800, "Called by HourlyUpdate lambda_4 wrapper"),
    ("HourlyPhase5_call_target", 0xC35970, "Called by HourlyUpdate lambda_5 wrapper"),
    ("AirWorker_chunk_A", 0xC36380, "Direct jump target from hourly phase worker"),
    ("AirWorker_chunk_B", 0xC36610, "Direct jump target from hourly phase worker"),
    ("AirWorker_chunk_C", 0xC36D00, "Direct jump target from daily/hourly worker"),
    ("AirWorker_chunk_D", 0xC36F90, "Direct jump target from daily/hourly worker"),
    ("AirWorker_chunk_E", 0xC37380, "Direct jump target from daily/hourly worker"),
    ("AirWorker_chunk_F", 0xC37760, "Direct jump target from daily/hourly worker"),
    ("AirWorker_chunk_G", 0xC379F0, "Direct jump target from daily/hourly worker"),
    ("AirWorker_chunk_H", 0xC37CE0, "Direct jump target from daily/hourly worker"),
    ("AirWorker_chunk_I", 0xC37F70, "Direct jump target from daily/hourly worker"),
    ("StrategicAir_ace_management_candidate", 0xC52840, "strategicair.cpp, ace roster/name/start experience candidate"),
    ("StrategicAir_ace_callsite_caller_6F9170", 0x6F9170, "Sole direct caller of 0xC52840; callsite 0x6FA8BA"),
    ("StrategicAir_ace_callsite_arg_factory_6C2D80", 0x6C2D80, "Callsite 0x6FA8BA passes rcx = return(0x6C2D80(r15))"),
    ("Sortie_efficiency_candidate", 0x1963700, "assertion: Could not find an air wing pool to compute sortie efficiency"),
    ("AirRegionCombatData_dtor", 0xC3B670, "CAirRegionCombatData vtable slot 0"),
    ("AirRegionCombatData_serialize", 0x1950F80, "CAirRegionCombatData vtable slot 2"),
    ("AirRegionCombatData_dispatch", 0x1950A30, "CAirRegionCombatData vtable slot 4"),
    ("Deep_air_call_F64560", 0xF64560, "Direct callee below live air worker/helper; no typed object validated yet"),
    ("Deep_air_call_F64880", 0xF64880, "Direct callee below live air worker/helper; CAirMission observed at runtime"),
    ("Deep_air_call_F64A90", 0xF64A90, "Direct callee below live air worker/helper; no live hit in 25h proof"),
    ("Deep_air_call_F652A0", 0xF652A0, "Direct callee below live air worker/helper; CAirMission observed at runtime"),
    ("Deep_air_call_F652E0", 0xF652E0, "Direct callee below live air worker/helper; CStrategicRegion observed at runtime"),
    ("Deep_air_call_F65B00", 0xF65B00, "Direct callee below live air worker/helper; no live hit in 25h proof"),
    ("Deep_air_call_F65B40", 0xF65B40, "Direct callee below live air worker/helper; no live hit in 25h proof"),
    ("Deep_air_call_F69CC0", 0xF69CC0, "Direct callee below live air worker/helper; CAirMission observed at runtime"),
    ("Deep_air_call_F4B6E0", 0xF4B6E0, "Direct callee below live air worker/helper; CAirWing observed at runtime"),
    ("Deep_air_call_F4C430", 0xF4C430, "Direct callee below live air worker/helper; CAirWing observed at runtime"),
    ("Deep_air_call_F49710", 0xF49710, "Direct callee below live air worker/helper; CAirWing observed at runtime"),
    ("Deep_air_call_F51060", 0xF51060, "Direct callee below live air worker/helper; CAirWing observed at runtime"),
    ("Deep_air_call_F62450", 0xF62450, "Direct callee below live air worker/helper; CAirMission observed at runtime"),
    ("Deep_air_call_F625B0", 0xF625B0, "Direct callee below live air worker/helper; CStrategicRegion observed at runtime"),
    ("Deep_air_call_C2B5C0", 0xC2B5C0, "Direct callee below live air worker/helper; CShip observed at runtime"),
    ("Deep_air_call_C3D3A0", 0xC3D3A0, "Direct callee below live air worker/helper; CShip/CAirBase observed at runtime"),
    ("Interior_pair_candidate_F46A10", 0xF46A10, "Second-level callee; runtime observed CAirWing/CAirWing argument shape"),
    ("Interior_pair_candidate_F6B7B0", 0xF6B7B0, "Second-level callee; runtime observed CAirMission/CAirMission argument shape"),
    ("Pair_inner_call_F4E0E0", 0xF4E0E0, "Direct callee from 0xF46A10 paired-CAirWing candidate"),
    ("Pair_inner_call_F49E80", 0xF49E80, "Direct callee from 0xF6B7B0 paired-CAirMission candidate"),
    ("Interior_helper_C448F0", 0xC448F0, "Second-level callee; CAirBase observed at runtime"),
    ("Interior_helper_C44FA0", 0xC44FA0, "Second-level callee; CStrategicAir/CStrategicRegion observed at runtime"),
    ("Interior_helper_E0CD40", 0xE0CD40, "Second-level callee; CStrategicRegion observed at runtime"),
    ("Interior_helper_E0CF00", 0xE0CF00, "Second-level callee; CStrategicRegion observed at runtime"),
    ("Interior_helper_F044D0", 0xF044D0, "Second-level callee; CWeatherManager observed at runtime"),
    ("Interior_helper_F04640", 0xF04640, "Second-level callee below weather/air calculation"),
    ("Interior_helper_F04E40", 0xF04E40, "Second-level callee; CWeatherManager/CArmy observed at runtime"),
    ("Interior_helper_F46A10", 0xF46A10, "Second-level callee; CAirWing state helper observed at runtime"),
    ("Interior_helper_F46B00", 0xF46B00, "Second-level callee; CAirWing observed at runtime"),
    ("Interior_helper_F47850", 0xF47850, "Second-level callee; CAirWing observed at runtime"),
    ("Interior_helper_F48230", 0xF48230, "Second-level callee below CAirWing calculation"),
    ("Interior_helper_F49280", 0xF49280, "Second-level callee below CAirWing calculation"),
    ("Interior_helper_F493A0", 0xF493A0, "Second-level callee; CAirWingPool observed at runtime"),
    ("Interior_helper_F4B5E0", 0xF4B5E0, "Second-level callee; CAirWing observed at runtime"),
    ("Interior_helper_F4B7F0", 0xF4B7F0, "Second-level callee; CAirWing observed at runtime"),
    ("Interior_helper_F4C480", 0xF4C480, "Second-level callee; CAirWing observed at runtime"),
    ("Interior_helper_F51630", 0xF51630, "Second-level callee; CAirWing observed at runtime"),
    ("Interior_helper_F51A60", 0xF51A60, "Second-level callee below CAirWing calculation"),
    ("Interior_helper_F631E0", 0xF631E0, "Second-level callee; CAirMission/CAirBase observed at runtime"),
    ("Interior_helper_F63380", 0xF63380, "Second-level callee; CAirMission observed at runtime"),
    ("Interior_helper_F69BC0", 0xF69BC0, "Second-level callee; CAirMission observed at runtime"),
    ("Interior_helper_F6B7B0", 0xF6B7B0, "Second-level callee; CAirMission scalar helper observed at runtime"),
    ("Interior_helper_F71050", 0xF71050, "Second-level callee; CStrategicRegion/CCountry observed at runtime"),
    ("Interior_helper_1004BF0", 0x1004BF0, "Second-level callee; CSubUnitDefinition observed at runtime"),
    ("Interior_helper_1004C40", 0x1004C40, "Second-level callee; CSubUnitDefinition/CEquipmentVariant observed at runtime"),
    ("Interior_helper_1004C50", 0x1004C50, "Second-level callee; CSubUnitDefinition observed at runtime"),
    ("Interior_helper_1004C80", 0x1004C80, "Second-level callee; CSubUnitDefinition observed at runtime"),
    ("Interior_helper_1004E70", 0x1004E70, "Second-level callee; CSubUnitDefinition/CEquipmentVariantPool observed at runtime"),
    ("Interior_airwing_candidate_F4C480", 0xF4C480, "Second-level callee; runtime observed CAirWing argument shape"),
    ("Interior_airwing_candidate_F4B7F0", 0xF4B7F0, "Second-level callee; runtime observed CAirWing argument shape"),
    ("Interior_airwingpool_candidate_F493A0", 0xF493A0, "Second-level callee; runtime observed CAirWingPool argument shape"),
]

OFFSET_LABELS: dict[int, list[str]] = {
    0x28: ["air_wing_body.pool", "air_wing_pool.wing_object_begin", "generic_vector"],
    0x30: ["strategic_air_manager.country_containers_begin", "worker_state"],
    0x34: ["air_wing_pool.wing_object_count", "worker_state"],
    0x3C: ["strategic_air_manager.country_container_count"],
    0x40: ["air_wing_pool.airbase_owner", "worker_owner"],
    0x58: ["air_region.region_id", "airbase_owner.pool_begin"],
    0x64: ["airbase_owner.pool_count"],
    0x6C: ["air_wing_body.plane_count"],
    0x90: ["strategic_air_manager.air_base_state_table", "air_wing_body.mission_mask"],
    0xB0: ["air_wing_body.mission_region"],
    0x160: ["air_wing_body.effective_planes"],
    0x1E8: ["air_wing_body.equipment_bank"],
    0x290: ["air_wing_body.regular_leader_attachment_a"],
    0x2B0: ["air_wing_body.regular_leader_attachment_b"],
    0x2F0: ["air_wing_body.regular_leader_attachment_c"],
    0x690: ["cgamestate.strategic_air_manager"],
    0x12D8: ["country.ace_roster_vector"],
    0x12E4: ["country.ace_count"],
}

IMMEDIATE_LABELS = {
    69: "air_wing_handle_type",
    70: "equipment_variant_type",
    100000: "equipment_amount_per_plane",
    0x186A0: "equipment_amount_per_plane_hex",
}


def hx(value: int | None) -> str | None:
    return None if value is None else f"0x{value:X}"


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


class Image:
    def __init__(self, path: Path) -> None:
        import pefile

        self.path = path
        self.data = path.read_bytes()
        self.pe = pefile.PE(str(path), fast_load=False)
        self.image_base = int(self.pe.OPTIONAL_HEADER.ImageBase)
        self.sections: list[dict[str, int | str]] = []
        for sec in self.pe.sections:
            name = sec.Name.rstrip(b"\0").decode("ascii", errors="replace")
            self.sections.append(
                {
                    "name": name,
                    "va": int(sec.VirtualAddress),
                    "vsize": int(sec.Misc_VirtualSize),
                    "raw": int(sec.PointerToRawData),
                    "rsize": int(sec.SizeOfRawData),
                }
            )

    def rva_to_file(self, rva: int) -> int | None:
        for sec in self.sections:
            va = int(sec["va"])
            span = max(int(sec["vsize"]), int(sec["rsize"]))
            if va <= rva < va + span:
                return int(sec["raw"]) + (rva - va)
        return None

    def rva_in_text(self, rva: int) -> bool:
        for sec in self.sections:
            if sec["name"] != ".text":
                continue
            va = int(sec["va"])
            span = max(int(sec["vsize"]), int(sec["rsize"]))
            return va <= rva < va + span
        return False


def parse_runtime_functions(img: Image) -> list[dict[str, int]]:
    pdata = next((sec for sec in img.sections if sec["name"] == ".pdata"), None)
    if pdata is None:
        return []
    raw = int(pdata["raw"])
    size = int(pdata["rsize"])
    blob = img.data[raw : raw + size]
    rows = []
    for off in range(0, len(blob) - 12 + 1, 12):
        begin, end, unwind = struct.unpack_from("<III", blob, off)
        if begin and end > begin and img.rva_in_text(begin):
            rows.append({"begin_rva": begin, "end_rva": end, "unwind_rva": unwind})
    rows.sort(key=lambda row: row["begin_rva"])
    return rows


def containing_runtime_function(functions: list[dict[str, int]], rva: int) -> dict[str, Any] | None:
    lo = 0
    hi = len(functions) - 1
    while lo <= hi:
        mid = (lo + hi) // 2
        row = functions[mid]
        if rva < row["begin_rva"]:
            hi = mid - 1
        elif rva >= row["end_rva"]:
            lo = mid + 1
        else:
            return {key: hx(value) for key, value in row.items()}
    return None


def disassemble_target(img: Image, functions: list[dict[str, int]], target: tuple[str, int, str], args: argparse.Namespace) -> dict[str, Any]:
    import capstone

    label, rva, reason = target
    file_off = img.rva_to_file(rva)
    runtime_function = containing_runtime_function(functions, rva)
    row: dict[str, Any] = {
        "label": label,
        "rva": hx(rva),
        "va": hx(img.image_base + rva),
        "reason": reason,
        "runtime_function": runtime_function,
    }
    if file_off is None:
        row["error"] = "target rva is not mappable"
        return row

    md = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_64)
    md.detail = True
    window_bytes = args.window_bytes
    if runtime_function and (
        args.respect_pdata_all
        or (
            args.respect_pdata_wrappers
            and ("vtable_slot" in label or label.startswith("AirRegionCombatData"))
        )
    ):
        runtime_end = int(str(runtime_function["end_rva"]), 16)
        window_bytes = min(window_bytes, max(0, runtime_end - rva))
    row["window_bytes_used"] = hx(window_bytes)
    blob = img.data[file_off : min(len(img.data), file_off + window_bytes)]
    instructions = []
    evidence = []
    calls = []
    jumps = []
    offset_hits: dict[str, int] = {}
    immediate_hits: dict[str, int] = {}
    count = 0
    for insn in md.disasm(blob, img.image_base + rva):
        insn_rva = insn.address - img.image_base
        text = f"{insn.mnemonic} {insn.op_str}".strip()
        record = {"rva": hx(insn_rva), "text": text}
        if len(instructions) < args.max_instructions:
            instructions.append(record)

        labels = []
        if insn.mnemonic == "call" and insn.operands and insn.operands[0].type == capstone.x86.X86_OP_IMM:
            target_rva = int(insn.operands[0].imm) - img.image_base
            calls.append({"insn_rva": hx(insn_rva), "target_rva": hx(target_rva), "target_va": hx(int(insn.operands[0].imm))})
            labels.append("call")
        elif insn.mnemonic == "call":
            calls.append({"insn_rva": hx(insn_rva), "target": text})
            labels.append("call")

        if insn.mnemonic.startswith("j"):
            jump: dict[str, Any] = {"insn_rva": hx(insn_rva), "text": text}
            if insn.operands and insn.operands[0].type == capstone.x86.X86_OP_IMM:
                jump["target_rva"] = hx(int(insn.operands[0].imm) - img.image_base)
            jumps.append(jump)
            labels.append("jump")

        for operand in insn.operands:
            if operand.type == capstone.x86.X86_OP_MEM:
                disp = int(operand.mem.disp)
                for offset_label in OFFSET_LABELS.get(disp, []):
                    offset_hits[offset_label] = offset_hits.get(offset_label, 0) + 1
                    labels.append(offset_label)
            elif operand.type == capstone.x86.X86_OP_IMM:
                imm = int(operand.imm)
                imm_label = IMMEDIATE_LABELS.get(imm)
                if imm_label is not None:
                    immediate_hits[imm_label] = immediate_hits.get(imm_label, 0) + 1
                    labels.append(imm_label)

        if labels and len(evidence) < args.max_evidence:
            evidence.append({**record, "labels": sorted(set(labels))})
        count += 1
        if count >= args.max_disasm_instructions:
            break

    row.update(
        {
            "instruction_count_scanned": count,
            "instructions": instructions,
            "evidence": evidence,
            "calls": calls[: args.max_evidence],
            "jumps": jumps[: args.max_evidence],
            "offset_hits": offset_hits,
            "immediate_hits": immediate_hits,
        }
    )
    return row


def run(args: argparse.Namespace) -> dict[str, Any]:
    started = time.time()
    exe = args.exe.resolve()
    img = Image(exe)
    functions = parse_runtime_functions(img)
    targets = [disassemble_target(img, functions, target, args) for target in TARGETS]
    return {
        "schema": SCHEMA,
        "created_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "exe": str(exe),
        "exe_sha256": sha256_file(exe),
        "expected_sha256": EXPECTED_SHA256,
        "image_base": hx(img.image_base),
        "runtime_function_count": len(functions),
        "scan_limits": {
            "window_bytes": args.window_bytes,
            "max_instructions": args.max_instructions,
            "max_disasm_instructions": args.max_disasm_instructions,
            "max_evidence": args.max_evidence,
        },
        "targets": targets,
        "verdict": {
            "classification": "bounded_candidate_disassembly_recorded",
            "production_hook_validated": False,
            "reason": (
                "Named RTTI wrappers and phase chunks are exact hook candidates, "
                "but this static pass does not prove live arguments or residual event fields."
            ),
        },
        "elapsed_s": round(time.time() - started, 3),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--exe", type=Path, default=DEFAULT_EXE)
    parser.add_argument("--out-dir", type=Path, required=True)
    parser.add_argument("--window-bytes", type=lambda value: int(value, 0), default=0x600)
    parser.add_argument("--respect-pdata-wrappers", action=argparse.BooleanOptionalAction, default=True)
    parser.add_argument("--respect-pdata-all", action=argparse.BooleanOptionalAction, default=False)
    parser.add_argument("--max-instructions", type=int, default=80)
    parser.add_argument("--max-disasm-instructions", type=int, default=260)
    parser.add_argument("--max-evidence", type=int, default=80)
    args = parser.parse_args()

    proof = run(args)
    args.out_dir.mkdir(parents=True, exist_ok=True)
    out = args.out_dir / "air_resolution_target_disasm_870ebf.json"
    out.write_text(json.dumps(proof, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({"out": str(out), "verdict": proof["verdict"], "elapsed_s": proof["elapsed_s"]}, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
