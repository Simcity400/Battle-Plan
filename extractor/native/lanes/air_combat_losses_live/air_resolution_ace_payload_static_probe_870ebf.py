#!/usr/bin/env python3
"""Static contract proof for the narrowed 0xC52840 ace payload route."""

from __future__ import annotations

import argparse
import hashlib
import json
import struct
import time
from pathlib import Path
from typing import Any

EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
SCHEMA = "air_resolution_ace_payload_static_probe_v1"
DEFAULT_EXE = Path(r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV\hoi4.exe")

ACE_FUNCTION_RVA = 0xC52840
ACE_FUNCTION_END_RVA = 0xC53460
ACE_CALLER_RVA = 0x6F9170
ACE_BLOCK_RVA = 0x6FA8AC
ACE_CALLSITE_RVA = 0x6FA8BA


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
            self.sections.append(
                {
                    "name": sec.Name.rstrip(b"\0").decode("ascii", errors="replace"),
                    "va": int(sec.VirtualAddress),
                    "vsize": int(sec.Misc_VirtualSize),
                    "raw": int(sec.PointerToRawData),
                    "rsize": int(sec.SizeOfRawData),
                }
            )

    def section_for_rva(self, rva: int) -> dict[str, int | str] | None:
        for sec in self.sections:
            va = int(sec["va"])
            span = max(int(sec["vsize"]), int(sec["rsize"]))
            if va <= rva < va + span:
                return sec
        return None

    def rva_to_file(self, rva: int) -> int:
        sec = self.section_for_rva(rva)
        if sec is None:
            raise ValueError(f"RVA is not mappable: {rva:#x}")
        return int(sec["raw"]) + (rva - int(sec["va"]))

    def read(self, rva: int, size: int) -> bytes:
        off = self.rva_to_file(rva)
        return self.data[off : off + size]


def parse_runtime_functions(img: Image) -> list[dict[str, int]]:
    pdata = next((sec for sec in img.sections if sec["name"] == ".pdata"), None)
    text = next((sec for sec in img.sections if sec["name"] == ".text"), None)
    if pdata is None or text is None:
        return []
    raw = int(pdata["raw"])
    size = int(pdata["rsize"])
    blob = img.data[raw : raw + size]
    text_va = int(text["va"])
    text_span = max(int(text["vsize"]), int(text["rsize"]))
    rows = []
    for off in range(0, len(blob) - 12 + 1, 12):
        begin, end, unwind = struct.unpack_from("<III", blob, off)
        if begin and end > begin and text_va <= begin < text_va + text_span:
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


def read_ascii(img: Image, rva: int, limit: int = 160) -> str | None:
    sec = img.section_for_rva(rva)
    if sec is None:
        return None
    raw = img.rva_to_file(rva)
    out = bytearray()
    for b in img.data[raw : raw + limit]:
        if b == 0:
            break
        if b < 0x20 or b > 0x7E:
            return None
        out.append(b)
    if len(out) < 3:
        return None
    return out.decode("ascii", errors="replace")


def disassemble(img: Image, start_rva: int, end_rva: int) -> list[Any]:
    import capstone

    md = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_64)
    md.detail = True
    blob = img.read(start_rva, end_rva - start_rva)
    return list(md.disasm(blob, img.image_base + start_rva))


def operand_text(insn: Any) -> str:
    return f"{insn.mnemonic} {insn.op_str}".strip()


def collect_static_contract(img: Image) -> dict[str, Any]:
    import capstone

    functions = parse_runtime_functions(img)
    insns = disassemble(img, ACE_FUNCTION_RVA, ACE_FUNCTION_END_RVA)
    caller_insns = disassemble(img, ACE_BLOCK_RVA, ACE_BLOCK_RVA + 0x24)

    payload_offsets: dict[int, list[dict[str, str]]] = {}
    country_offsets: dict[int, list[dict[str, str]]] = {}
    ace_entry_offsets: dict[int, list[dict[str, str]]] = {}
    payload_branch_values = []
    rip_refs = []
    helper_calls = []
    roster_mutations = []
    recent: list[dict[str, str]] = []

    for insn in insns:
        rva = int(insn.address - img.image_base)
        text = operand_text(insn)
        rec = {"rva": hx(rva), "text": text}

        if insn.mnemonic == "cmp" and len(insn.operands) == 2:
            op0, op1 = insn.operands
            if op0.type == capstone.x86.X86_OP_REG and insn.reg_name(op0.reg) == "r8d" and op1.type == capstone.x86.X86_OP_IMM:
                payload_branch_values.append({**rec, "command_value": hx(int(op1.imm))})

        for op in insn.operands:
            if op.type != capstone.x86.X86_OP_MEM:
                continue
            base_reg = insn.reg_name(op.mem.base)
            disp = int(op.mem.disp)
            target: dict[int, list[dict[str, str]]] | None = None
            if base_reg == "r15":
                target = payload_offsets
            elif base_reg == "r13":
                target = country_offsets
            elif base_reg == "r14":
                target = ace_entry_offsets
            if target is not None:
                target.setdefault(disp, []).append(rec)
            if base_reg == "rip":
                rip_rva = rva + insn.size + disp
                string = read_ascii(img, rip_rva)
                rip_refs.append({**rec, "target_rva": hx(rip_rva), "ascii": string})

        if insn.mnemonic == "call":
            if any("r15" in row["text"] for row in recent[-3:]) or "r15" in text:
                helper_calls.append({**rec, "recent_context": recent[-4:]})

        if "[r13 + 0x12" in text or "[r13+0x12" in text:
            if insn.mnemonic.startswith("mov") or insn.mnemonic in {"inc", "cmp"}:
                roster_mutations.append(rec)

        recent.append(rec)
        if len(recent) > 8:
            recent.pop(0)

    field_summary = {
        "payload_r15_offsets": [
            {"offset": hx(offset), "count": len(rows), "evidence": rows[:8]}
            for offset, rows in sorted(payload_offsets.items())
        ],
        "country_r13_offsets": [
            {"offset": hx(offset), "count": len(rows), "evidence": rows[:8]}
            for offset, rows in sorted(country_offsets.items())
        ],
        "created_ace_r14_offsets": [
            {"offset": hx(offset), "count": len(rows), "evidence": rows[:8]}
            for offset, rows in sorted(ace_entry_offsets.items())
        ],
    }

    return {
        "ace_function": {
            "rva": hx(ACE_FUNCTION_RVA),
            "end_rva": hx(ACE_FUNCTION_END_RVA),
            "runtime_function": containing_runtime_function(functions, ACE_FUNCTION_RVA),
            "instruction_count": len(insns),
        },
        "caller_block": {
            "caller_function_rva": hx(ACE_CALLER_RVA),
            "block_rva": hx(ACE_BLOCK_RVA),
            "callsite_rva": hx(ACE_CALLSITE_RVA),
            "instructions": [{"rva": hx(int(insn.address - img.image_base)), "text": operand_text(insn)} for insn in caller_insns],
        },
        "field_summary": field_summary,
        "payload_branch_values": payload_branch_values,
        "helper_calls_near_payload": helper_calls[:48],
        "country_roster_mutations": roster_mutations[:48],
        "rip_ascii_refs": [row for row in rip_refs if row["ascii"] is not None],
        "verdict": {
            "classification": "ace_command_payload_contract_narrowed",
            "payload_register": "rdx at 0xC52840 entry, copied to r15",
            "payload_type_field": "r15+0x30",
            "payload_data_fields": ["r15+0x28", "r15+0x34", "r15+0x38", "r15+0x44"],
            "country_ace_roster_fields": ["r13+0x12D8", "r13+0x12E0", "r13+0x12E4", "r13+0x12E8"],
            "reject_for_sortie_pairing_cause_source_reinforcement": True,
            "reason": (
                "0xC52840 consumes a command payload and mutates/queries the country ace roster; "
                "the bounded body has no direct opposed-wing participant vector, loss cause/source, "
                "kill attribution, tactical engagement, or reinforcement-reason payload."
            ),
            "remaining_ace_work": (
                "Trigger command 0x2FB5 through block 0x6FA8AC and validate the live r15 payload "
                "fields for ace assignment/death/wounding semantics."
            ),
        },
    }


def run(args: argparse.Namespace) -> dict[str, Any]:
    started = time.time()
    exe = args.exe.resolve()
    img = Image(exe)
    exe_sha = sha256_file(exe)
    contract = collect_static_contract(img)
    return {
        "schema": SCHEMA,
        "created_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "exe": str(exe),
        "exe_sha256": exe_sha,
        "expected_sha256": EXPECTED_SHA256,
        "expected_sha256_match": exe_sha == EXPECTED_SHA256,
        **contract,
        "elapsed_s": round(time.time() - started, 3),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--exe", type=Path, default=DEFAULT_EXE)
    parser.add_argument("--out-dir", type=Path, required=True)
    args = parser.parse_args()

    proof = run(args)
    args.out_dir.mkdir(parents=True, exist_ok=True)
    out = args.out_dir / "air_resolution_ace_payload_static_probe_870ebf.json"
    out.write_text(json.dumps(proof, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(
        json.dumps(
            {
                "out": str(out),
                "classification": proof["verdict"]["classification"],
                "payload_type_field": proof["verdict"]["payload_type_field"],
                "payload_branch_values": [row["command_value"] for row in proof["payload_branch_values"]],
                "reject_for_sortie_pairing_cause_source_reinforcement": proof["verdict"][
                    "reject_for_sortie_pairing_cause_source_reinforcement"
                ],
                "elapsed_s": proof["elapsed_s"],
            },
            indent=2,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
