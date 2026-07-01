#!/usr/bin/env python3
"""Static gate for remaining CFocusStatus paused-byte gaps.

This consumes the live wide-gate proof and the same-build executable. It is
lane-local and read-only. The goal is narrow: reject the all-zero
``CFocusStatus+0x3C..+0x3F`` bytes only if static CFocusStatus vtable-method
evidence shows that ``+0x38`` is used as qword-sized storage, making those bytes
part of a multi-byte field rather than independent booleans.
"""

from __future__ import annotations

import argparse
import json
import struct
from collections import defaultdict
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

import capstone
import pefile


TARGET_GAP_OFFSETS = (0x3C, 0x3D, 0x3E, 0x3F)
QWORD_BASE_OFFSET = 0x38


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def hex_offset(value: int) -> str:
    return f"0x{value:X}"


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def parse_hex(value: Any) -> int | None:
    if not isinstance(value, str) or not value.startswith("0x"):
        return None
    try:
        return int(value, 16)
    except ValueError:
        return None


def executable_ranges(pe: pefile.PE) -> list[tuple[int, int, str]]:
    ranges = []
    for section in pe.sections:
        if not (section.Characteristics & 0x20000000):
            continue
        name = section.Name.rstrip(b"\0").decode("ascii", errors="replace")
        start = int(section.VirtualAddress)
        size = max(int(section.Misc_VirtualSize), int(section.SizeOfRawData))
        ranges.append((start, start + size, name))
    return ranges


def rva_section(ranges: list[tuple[int, int, str]], rva: int) -> str | None:
    for start, end, name in ranges:
        if start <= rva < end:
            return name
    return None


def read_vtable_entries(
    pe: pefile.PE,
    data: bytes,
    vtable_rva: int,
    max_entries: int,
) -> list[dict[str, Any]]:
    image_base = int(pe.OPTIONAL_HEADER.ImageBase)
    image_end = image_base + int(pe.OPTIONAL_HEADER.SizeOfImage)
    ranges = executable_ranges(pe)
    entries = []
    file_offset = pe.get_offset_from_rva(vtable_rva)
    for index in range(max_entries):
        if file_offset + (index + 1) * 8 > len(data):
            break
        va = struct.unpack_from("<Q", data, file_offset + index * 8)[0]
        if not (image_base <= va < image_end):
            break
        rva = va - image_base
        section = rva_section(ranges, rva)
        entries.append({
            "index": index,
            "va": hex(va),
            "rva": hex_offset(rva),
            "section": section or "non_executable_image",
            "executable": section is not None,
        })
    return entries


def code_at(pe: pefile.PE, data: bytes, rva: int, size: int) -> bytes:
    try:
        file_offset = pe.get_offset_from_rva(rva)
    except Exception:
        return b""
    return data[file_offset:file_offset + size]


def canonical_reg(name: str) -> str:
    aliases = {
        "cl": "rcx",
        "cx": "rcx",
        "ecx": "rcx",
        "rcx": "rcx",
        "bl": "rbx",
        "bx": "rbx",
        "ebx": "rbx",
        "rbx": "rbx",
        "dil": "rdi",
        "di": "rdi",
        "edi": "rdi",
        "rdi": "rdi",
        "sil": "rsi",
        "si": "rsi",
        "esi": "rsi",
        "rsi": "rsi",
        "r12d": "r12",
        "r12": "r12",
        "r13d": "r13",
        "r13": "r13",
        "r14d": "r14",
        "r14": "r14",
        "r15d": "r15",
        "r15": "r15",
        "ebp": "rbp",
        "rbp": "rbp",
    }
    return aliases.get(name, name)


def disassemble_method(
    pe: pefile.PE,
    data: bytes,
    method_rva: int,
    max_bytes: int,
    max_instructions: int,
) -> dict[str, Any]:
    image_base = int(pe.OPTIONAL_HEADER.ImageBase)
    code = code_at(pe, data, method_rva, max_bytes)
    md = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_64)
    md.detail = True
    aliases = {"rcx"}
    accesses: list[dict[str, Any]] = []
    target_qword_accesses: list[dict[str, Any]] = []
    target_gap_direct_accesses: list[dict[str, Any]] = []

    for index, insn in enumerate(md.disasm(code, image_base + method_rva)):
        if index >= max_instructions:
            break
        for operand in insn.operands:
            if operand.type != capstone.x86.X86_OP_MEM:
                continue
            base_reg = canonical_reg(insn.reg_name(operand.mem.base))
            disp = int(operand.mem.disp)
            if base_reg not in aliases or disp < 0 or disp >= 0x200:
                continue
            access = {
                "insn_rva": hex_offset(insn.address - image_base),
                "mnemonic": insn.mnemonic,
                "op_str": insn.op_str,
                "offset": hex_offset(disp),
                "size": operand.size,
                "base_reg": base_reg,
            }
            accesses.append(access)
            if disp == QWORD_BASE_OFFSET and operand.size >= 8:
                target_qword_accesses.append(access)
            if disp in TARGET_GAP_OFFSETS:
                target_gap_direct_accesses.append(access)

        if len(insn.operands) >= 2 and insn.operands[0].type == capstone.x86.X86_OP_REG:
            dst = canonical_reg(insn.reg_name(insn.operands[0].reg))
            if insn.mnemonic == "mov" and insn.operands[1].type == capstone.x86.X86_OP_REG:
                src = canonical_reg(insn.reg_name(insn.operands[1].reg))
                if src in aliases:
                    aliases.add(dst)
                elif dst in aliases and dst != "rcx":
                    aliases.discard(dst)
            elif insn.mnemonic == "lea" and insn.operands[1].type == capstone.x86.X86_OP_MEM:
                base_reg = canonical_reg(insn.reg_name(insn.operands[1].mem.base))
                disp = int(insn.operands[1].mem.disp)
                if base_reg in aliases and 0 <= disp < 0x200:
                    access = {
                        "insn_rva": hex_offset(insn.address - image_base),
                        "mnemonic": insn.mnemonic,
                        "op_str": insn.op_str,
                        "offset": hex_offset(disp),
                        "size": 8,
                        "base_reg": base_reg,
                    }
                    accesses.append(access)
                    if disp == QWORD_BASE_OFFSET:
                        target_qword_accesses.append(access)
                elif dst in aliases and dst != "rcx":
                    aliases.discard(dst)
            elif dst in aliases and dst != "rcx" and insn.mnemonic not in ("cmp", "test"):
                aliases.discard(dst)

        if insn.mnemonic.startswith("ret"):
            break

    offset_counts: dict[str, int] = defaultdict(int)
    for access in accesses:
        offset_counts[access["offset"]] += 1
    return {
        "method_rva": hex_offset(method_rva),
        "instruction_count_scanned": min(index + 1 if code else 0, max_instructions),
        "field_access_count": len(accesses),
        "field_offsets_seen": dict(sorted(offset_counts.items(), key=lambda item: int(item[0], 16))),
        "target_qword_accesses": target_qword_accesses,
        "target_gap_direct_accesses": target_gap_direct_accesses,
        "sample_accesses": accesses[:24],
    }


def qword_structural_summary(wide_gate: dict[str, Any]) -> dict[str, Any] | None:
    for item in wide_gate.get("structural_candidates") or []:
        if item.get("offset") == hex_offset(QWORD_BASE_OFFSET):
            return {
                "offset": item.get("offset"),
                "records": item.get("records"),
                "nonzero_rows": item.get("nonzero_rows"),
                "top_values": item.get("top_values"),
                "examples": item.get("examples"),
            }
    return None


def byte_summary(wide_gate: dict[str, Any], offset: int) -> dict[str, Any] | None:
    for item in wide_gate.get("byte_candidates") or []:
        if item.get("offset") == hex_offset(offset):
            return {
                "offset": item.get("offset"),
                "records": item.get("records"),
                "nonzero_rows": item.get("nonzero_rows"),
                "values": item.get("values"),
                "previous_verdict": item.get("verdict"),
            }
    return None


def run() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--wide-gate", type=Path, required=True)
    parser.add_argument("--out-dir", type=Path, required=True)
    parser.add_argument("--max-vtable-entries", type=int, default=64)
    parser.add_argument("--method-bytes", type=lambda value: int(value, 0), default=0x800)
    parser.add_argument("--max-instructions", type=int, default=260)
    args = parser.parse_args()

    wide_gate = load_json(args.wide_gate)
    process = wide_gate.get("process") or {}
    exe_path = Path(process.get("path") or "")
    live_base = parse_hex(process.get("base"))
    vtable_va = None
    for item in wide_gate.get("known_field_validations") or []:
        if item.get("name") == "object_vtable_or_type_pointer":
            values = item.get("top_values") or []
            if values:
                vtable_va = parse_hex(values[0].get("value"))
            break
    if not exe_path.exists() or live_base is None or vtable_va is None:
        raise SystemExit("wide gate lacks executable path, live base, or CFocusStatus vtable value")

    vtable_rva = vtable_va - live_base
    pe = pefile.PE(str(exe_path), fast_load=True)
    data = exe_path.read_bytes()
    entries = read_vtable_entries(pe, data, vtable_rva, args.max_vtable_entries)
    methods = []
    for rva_hex in sorted(
        {entry["rva"] for entry in entries if entry.get("executable")},
        key=lambda value: int(value, 16),
    ):
        methods.append(disassemble_method(
            pe,
            data,
            int(rva_hex, 16),
            args.method_bytes,
            args.max_instructions,
        ))

    target_qword_methods = [
        method for method in methods if method["target_qword_accesses"]
    ]
    target_gap_direct_methods = [
        method for method in methods if method["target_gap_direct_accesses"]
    ]
    gap_live_summaries = [
        byte_summary(wide_gate, offset) for offset in TARGET_GAP_OFFSETS
    ]
    gap_live_all_zero = all(
        item and item.get("nonzero_rows") == 0 for item in gap_live_summaries
    )
    qword_summary = qword_structural_summary(wide_gate)
    qword_nonbool = bool(qword_summary and int(qword_summary.get("nonzero_rows") or 0) > 0)
    rejected = bool(target_qword_methods and not target_gap_direct_methods and gap_live_all_zero and qword_nonbool)

    gate = {
        "schema": "focus_research_decisions_depth_live_focus_pause_static_gap_gate_v1",
        "lane": "focus_research_decisions_depth_live",
        "created_utc": utc_now(),
        "completion_status": "NOT_COMPLETE",
        "backlog_reduced": rejected,
        "wide_gate": str(args.wide_gate),
        "process": process,
        "build_gate": wide_gate.get("build_gate"),
        "exe_path": str(exe_path),
        "vtable": {
            "live_va": hex(vtable_va),
            "live_base": hex(live_base),
            "rva": hex_offset(vtable_rva),
            "entry_count": len(entries),
            "entries": entries,
        },
        "target_offsets": [hex_offset(offset) for offset in TARGET_GAP_OFFSETS],
        "target_qword_base": hex_offset(QWORD_BASE_OFFSET),
        "qword_live_summary": qword_summary,
        "gap_live_summaries": gap_live_summaries,
        "method_scan": {
            "method_count": len(methods),
            "methods_with_target_qword_access": len(target_qword_methods),
            "methods_with_target_gap_direct_access": len(target_gap_direct_methods),
            "target_qword_methods": target_qword_methods,
            "target_gap_direct_methods": target_gap_direct_methods,
            "methods": methods,
        },
        "rejected_byte_candidate_offsets": (
            [hex_offset(offset) for offset in TARGET_GAP_OFFSETS] if rejected else []
        ),
        "remaining_inconclusive_byte_candidate_offsets": (
            [] if rejected else [hex_offset(offset) for offset in TARGET_GAP_OFFSETS]
        ),
        "verdict": (
            "REJECTED_STATIC_QWORD_FIELD_OVERLAP"
            if rejected
            else "INCONCLUSIVE_STATIC_EVIDENCE_INSUFFICIENT"
        ),
        "reason": (
            "CFocusStatus vtable-method disassembly shows qword-sized access at +0x38, "
            "the live qword has non-boolean progress values, and no direct +0x3C..+0x3F "
            "field access was observed in the scanned CFocusStatus vtable methods."
            if rejected
            else "The static method scan did not prove qword overlap for the target gap."
        ),
        "gate": {
            "ok": bool((wide_gate.get("gate") or {}).get("ok")) and rejected,
            "errors": [] if rejected else ["target gap not rejected"],
        },
    }

    args.out_dir.mkdir(parents=True, exist_ok=True)
    (args.out_dir / "focus_pause_static_gap_gate.json").write_text(
        json.dumps(gate, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    lines = [
        "# Focus Pause Static Gap Gate",
        "",
        f"Created UTC: `{gate['created_utc']}`",
        f"Completion status: `{gate['completion_status']}`",
        f"Backlog reduced: `{str(gate['backlog_reduced']).lower()}`",
        "",
        "## Summary",
        "",
        f"- CFocusStatus vtable RVA: `{gate['vtable']['rva']}`.",
        f"- Vtable entries scanned: `{gate['vtable']['entry_count']}`.",
        f"- Methods scanned: `{gate['method_scan']['method_count']}`.",
        f"- Methods with qword access at `+0x38`: `{gate['method_scan']['methods_with_target_qword_access']}`.",
        f"- Methods with direct `+0x3C..+0x3F` access: `{gate['method_scan']['methods_with_target_gap_direct_access']}`.",
        f"- Rejected offsets: `{', '.join(gate['rejected_byte_candidate_offsets'])}`.",
        f"- Remaining inconclusive offsets: `{', '.join(gate['remaining_inconclusive_byte_candidate_offsets'])}`.",
        "",
        "## Evidence",
        "",
    ]
    for method in target_qword_methods:
        accesses = ", ".join(
            f"`{access['insn_rva']} {access['mnemonic']} {access['op_str']}`"
            for access in method["target_qword_accesses"][:4]
        )
        lines.append(f"- Method `{method['method_rva']}`: {accesses}.")
    if not target_qword_methods:
        lines.append("- No target qword access observed.")
    lines.append("")
    lines.append(f"Verdict: `{gate['verdict']}`.")
    (args.out_dir / "FOCUS_PAUSE_STATIC_GAP_GATE.md").write_text(
        "\n".join(lines) + "\n",
        encoding="utf-8",
    )

    print(json.dumps({
        "ok": gate["gate"]["ok"],
        "backlog_reduced": gate["backlog_reduced"],
        "vtable_rva": gate["vtable"]["rva"],
        "vtable_entries": gate["vtable"]["entry_count"],
        "methods_scanned": gate["method_scan"]["method_count"],
        "methods_with_target_qword_access": gate["method_scan"]["methods_with_target_qword_access"],
        "methods_with_target_gap_direct_access": gate["method_scan"]["methods_with_target_gap_direct_access"],
        "rejected_byte_candidate_offsets": gate["rejected_byte_candidate_offsets"],
        "remaining_inconclusive_byte_candidate_offsets": gate["remaining_inconclusive_byte_candidate_offsets"],
        "out_dir": str(args.out_dir),
    }, indent=2))
    return 0 if gate["gate"]["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(run())
