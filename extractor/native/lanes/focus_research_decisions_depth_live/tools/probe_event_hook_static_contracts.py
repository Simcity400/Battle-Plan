#!/usr/bin/env python3
"""Reject static event-hook anchors that are only registration/catalog strings.

This consumes the v1 string/xref event-hook gate and inspects each remaining
static anchor with PE unwind function ranges plus local disassembly. A string
xref is rejected only when the exact instruction pattern shows the string being
passed as a static registration/catalog argument rather than entering a runtime
callback with focus/research/decision objects.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import struct
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

import capstone
import pefile


EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
MIN_REGISTRATION_FUNCTION_STRING_REFS = 1000


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def hex_rva(value: int | None) -> str | None:
    return None if value is None else f"0x{value:X}"


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def parse_hex(value: Any) -> int | None:
    if not isinstance(value, str) or not value.startswith("0x"):
        return None
    try:
        return int(value, 16)
    except ValueError:
        return None


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def ascii_string_map(pe: pefile.PE, data: bytes) -> dict[int, str]:
    strings: dict[int, str] = {}
    for match in re.finditer(rb"[ -~]{4,220}", data):
        try:
            rva = pe.get_rva_from_offset(match.start())
        except Exception:
            continue
        strings[rva] = match.group(0).decode("latin1", errors="replace")
    return strings


def runtime_functions(pe: pefile.PE) -> list[tuple[int, int]]:
    funcs = []
    for item in getattr(pe, "DIRECTORY_ENTRY_EXCEPTION", []) or []:
        begin = int(item.struct.BeginAddress)
        end = int(item.struct.EndAddress)
        if end > begin:
            funcs.append((begin, end))
    return funcs


def containing_function(functions: list[tuple[int, int]], rva: int) -> tuple[int, int] | None:
    matches = [(begin, end) for begin, end in functions if begin <= rva < end]
    if not matches:
        return None
    return min(matches, key=lambda item: item[1] - item[0])


def code_at(pe: pefile.PE, data: bytes, rva: int, size: int) -> bytes:
    try:
        file_offset = pe.get_offset_from_rva(rva)
    except Exception:
        return b""
    return data[file_offset:file_offset + size]


def disassemble_function(pe: pefile.PE, data: bytes, begin: int, end: int) -> list[Any]:
    image_base = int(pe.OPTIONAL_HEADER.ImageBase)
    md = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_64)
    md.detail = True
    return list(md.disasm(code_at(pe, data, begin, end - begin), image_base + begin))


def immediate_call_target_rva(pe: pefile.PE, insn: Any) -> int | None:
    if insn.mnemonic != "call" or not insn.operands:
        return None
    operand = insn.operands[0]
    if operand.type != capstone.x86.X86_OP_IMM:
        return None
    return int(operand.imm) - int(pe.OPTIONAL_HEADER.ImageBase)


def instruction_string_refs(pe: pefile.PE, insn: Any, string_map: dict[int, str]) -> list[dict[str, Any]]:
    image_base = int(pe.OPTIONAL_HEADER.ImageBase)
    refs = []
    for operand in insn.operands:
        target_rva = None
        if operand.type == capstone.x86.X86_OP_MEM and insn.reg_name(operand.mem.base) == "rip":
            target_rva = int(insn.address + insn.size + operand.mem.disp - image_base)
        elif operand.type == capstone.x86.X86_OP_IMM:
            target_rva = int(operand.imm - image_base)
        if target_rva is None or target_rva not in string_map:
            continue
        refs.append({
            "target_rva": hex_rva(target_rva),
            "value": string_map[target_rva],
        })
    return refs


def function_string_ref_summary(
    pe: pefile.PE,
    insns: list[Any],
    string_map: dict[int, str],
) -> dict[str, Any]:
    refs = []
    for insn in insns:
        for ref in instruction_string_refs(pe, insn, string_map):
            refs.append((int(str(ref["target_rva"]), 16), ref["value"]))
    counts = Counter(value for _rva, value in refs)
    return {
        "string_ref_count": len(refs),
        "unique_string_ref_count": len(counts),
        "sample_string_refs": [
            {"target_rva": hex_rva(rva), "value": value}
            for rva, value in refs[:24]
        ],
    }


def local_contract(
    pe: pefile.PE,
    insns: list[Any],
    xref_rva: int,
    string_map: dict[int, str],
) -> dict[str, Any]:
    image_base = int(pe.OPTIONAL_HEADER.ImageBase)
    index = next((i for i, insn in enumerate(insns) if int(insn.address - image_base) == xref_rva), None)
    if index is None:
        return {"found": False}
    xref = insns[index]
    local = insns[max(0, index - 12):min(len(insns), index + 16)]
    local_rows = []
    local_strings = []
    for insn in local:
        refs = instruction_string_refs(pe, insn, string_map)
        local_rows.append({
            "insn_rva": hex_rva(int(insn.address - image_base)),
            "mnemonic": insn.mnemonic,
            "op_str": insn.op_str,
            "string_refs": refs,
            "call_target_rva": hex_rva(immediate_call_target_rva(pe, insn)),
        })
        for ref in refs:
            local_strings.append({
                "insn_rva": hex_rva(int(insn.address - image_base)),
                **ref,
            })

    prior_call = None
    for insn in reversed(insns[max(0, index - 8):index]):
        target = immediate_call_target_rva(pe, insn)
        if target is not None:
            prior_call = {
                "insn_rva": hex_rva(int(insn.address - image_base)),
                "target_rva": hex_rva(target),
            }
            break

    next_call = None
    for insn in insns[index + 1:index + 8]:
        target = immediate_call_target_rva(pe, insn)
        if target is not None:
            next_call = {
                "insn_rva": hex_rva(int(insn.address - image_base)),
                "target_rva": hex_rva(target),
            }
            break

    xref_register = None
    if xref.mnemonic == "lea" and xref.operands and xref.operands[0].type == capstone.x86.X86_OP_REG:
        xref_register = xref.reg_name(xref.operands[0].reg)

    rcx_global_setup = None
    edx_from_registration_key = None
    for insn in insns[index + 1:index + 5]:
        if insn.mnemonic == "lea" and insn.operands and insn.operands[0].type == capstone.x86.X86_OP_REG:
            if insn.reg_name(insn.operands[0].reg) == "rcx":
                rcx_global_setup = {
                    "insn_rva": hex_rva(int(insn.address - image_base)),
                    "op_str": insn.op_str,
                }
        if insn.mnemonic == "mov" and insn.operands and insn.operands[0].type == capstone.x86.X86_OP_REG:
            if insn.reg_name(insn.operands[0].reg) == "edx":
                edx_from_registration_key = {
                    "insn_rva": hex_rva(int(insn.address - image_base)),
                    "op_str": insn.op_str,
                }

    return {
        "found": True,
        "xref_instruction": {
            "insn_rva": hex_rva(xref_rva),
            "mnemonic": xref.mnemonic,
            "op_str": xref.op_str,
            "string_argument_register": xref_register,
        },
        "prior_call": prior_call,
        "next_call": next_call,
        "rcx_global_setup_after_xref": rcx_global_setup,
        "edx_registration_key_after_xref": edx_from_registration_key,
        "local_string_refs": local_strings,
        "local_instructions": local_rows,
    }


def anchor_contracts(event_gate: dict[str, Any], pe: pefile.PE, data: bytes) -> list[dict[str, Any]]:
    string_map = ascii_string_map(pe, data)
    functions = runtime_functions(pe)
    by_function: dict[tuple[int, int], list[Any]] = {}
    function_summaries: dict[tuple[int, int], dict[str, Any]] = {}
    out = []
    for anchor in event_gate.get("possible_static_anchors") or []:
        text_xrefs = anchor.get("text_xrefs") or []
        for xref in text_xrefs:
            xref_rva = parse_hex(xref.get("insn_rva"))
            string_rva = parse_hex(anchor.get("rva"))
            if xref_rva is None or string_rva is None:
                continue
            function_range = containing_function(functions, xref_rva)
            if function_range is None:
                out.append({
                    "anchor": anchor,
                    "xref": xref,
                    "verdict": "INCONCLUSIVE_NO_UNWIND_FUNCTION",
                    "reason": "No PE unwind/runtime-function range contains this xref.",
                })
                continue
            if function_range not in by_function:
                insns = disassemble_function(pe, data, function_range[0], function_range[1])
                by_function[function_range] = insns
                function_summaries[function_range] = function_string_ref_summary(pe, insns, string_map)
            insns = by_function[function_range]
            summary = function_summaries[function_range]
            contract = local_contract(pe, insns, xref_rva, string_map)
            registration_pattern = (
                contract.get("found")
                and (contract.get("xref_instruction") or {}).get("string_argument_register") == "r8"
                and bool(contract.get("prior_call"))
                and bool(contract.get("next_call"))
                and bool(contract.get("rcx_global_setup_after_xref"))
                and bool(contract.get("edx_registration_key_after_xref"))
                and int(summary.get("string_ref_count") or 0) >= MIN_REGISTRATION_FUNCTION_STRING_REFS
            )
            out.append({
                "anchor": {
                    "value": anchor.get("value"),
                    "needle": anchor.get("needle"),
                    "rva": anchor.get("rva"),
                    "section": anchor.get("section"),
                },
                "xref": xref,
                "function": {
                    "begin_rva": hex_rva(function_range[0]),
                    "end_rva": hex_rva(function_range[1]),
                    "size": hex_rva(function_range[1] - function_range[0]),
                    **summary,
                },
                "contract": contract,
                "verdict": (
                    "REJECTED_STATIC_REGISTRATION_ARGUMENT_NOT_RUNTIME_CALLBACK"
                    if registration_pattern
                    else "INCONCLUSIVE_NO_STATIC_REGISTRATION_PATTERN"
                ),
                "reason": (
                    "The anchor string is loaded into Windows x64 argument register r8, "
                    "then passed with a registration key in edx and a global registry "
                    "address in rcx to a shared registration/catalog call inside a "
                    "large string-registration function. This does not provide a "
                    "runtime callback body or focus/research/decision object argument contract."
                    if registration_pattern
                    else "The local xref pattern was not strong enough to reject this anchor."
                ),
            })
    return out


def build_gate(event_gate_path: Path) -> dict[str, Any]:
    event_gate = load_json(event_gate_path)
    exe_path = Path(event_gate.get("exe_path") or "")
    if not exe_path.exists():
        raise FileNotFoundError(exe_path)
    data = exe_path.read_bytes()
    actual_sha = hashlib.sha256(data).hexdigest()
    pe = pefile.PE(str(exe_path), fast_load=False)
    contracts = anchor_contracts(event_gate, pe, data)
    rejected = [
        item for item in contracts
        if str(item.get("verdict") or "").startswith("REJECTED")
    ]
    inconclusive = [
        item for item in contracts
        if not str(item.get("verdict") or "").startswith("REJECTED")
    ]
    return {
        "schema": "focus_research_decisions_depth_live_event_hook_static_contract_gate_v1",
        "lane": "focus_research_decisions_depth_live",
        "created_utc": utc_now(),
        "completion_status": "NOT_COMPLETE",
        "event_hook_gate": str(event_gate_path),
        "exe_path": str(exe_path),
        "build_gate": {
            "expected_sha256": EXPECTED_SHA256,
            "actual_sha256": actual_sha,
            "ok": actual_sha.lower() == EXPECTED_SHA256,
        },
        "candidate_anchor_count": len(contracts),
        "rejected_static_anchor_count": len(rejected),
        "remaining_static_anchor_count": len(inconclusive),
        "contracts": contracts,
        "rejected_static_anchors": rejected,
        "remaining_static_anchors": inconclusive,
        "backlog_reduced": bool(rejected),
        "remaining_blocker": {
            "field": "direct_engine_event_hooks",
            "status": "BLOCKED",
            "reason": (
                "All current static string/xref anchors are rejected as registration "
                "catalog entries, not runtime callbacks. A direct hook still requires "
                "a concrete runtime function with focus/research/decision object "
                "arguments and live invocation proof."
            ),
            "next_evidence_needed": (
                "dynamic breakpoint/hook proof at a non-registration function, or a "
                "new static root whose local contract passes live object arguments"
            ),
        },
        "gate": {
            "ok": actual_sha.lower() == EXPECTED_SHA256 and bool(contracts) and not inconclusive,
            "errors": [] if actual_sha.lower() == EXPECTED_SHA256 and bool(contracts) and not inconclusive else [
                "build mismatch, no anchors, or inconclusive static anchors remain"
            ],
        },
    }


def write_markdown(path: Path, gate: dict[str, Any]) -> None:
    lines = [
        "# Event Hook Static Contract Gate",
        "",
        f"Created UTC: `{gate['created_utc']}`",
        f"Completion status: `{gate['completion_status']}`",
        f"Backlog reduced: `{str(gate['backlog_reduced']).lower()}`",
        "",
        "## Summary",
        "",
        f"- Candidate static anchors: `{gate['candidate_anchor_count']}`.",
        f"- Rejected static anchors: `{gate['rejected_static_anchor_count']}`.",
        f"- Remaining static anchors: `{gate['remaining_static_anchor_count']}`.",
        "",
        "## Rejections",
        "",
    ]
    for item in gate["rejected_static_anchors"]:
        anchor = item["anchor"]
        function = item["function"]
        contract = item["contract"]
        lines.append(
            f"- `{anchor['value']}` at `{anchor['rva']}`: {item['verdict']}; "
            f"function `{function['begin_rva']}..{function['end_rva']}`, "
            f"string refs `{function['string_ref_count']}`, "
            f"xref `{contract['xref_instruction']['insn_rva']} "
            f"{contract['xref_instruction']['mnemonic']} {contract['xref_instruction']['op_str']}`."
        )
        if contract.get("next_call"):
            lines.append(
                f"  Registration call: `{contract['next_call']['insn_rva']}` -> `{contract['next_call']['target_rva']}`."
            )
    if not gate["rejected_static_anchors"]:
        lines.append("- None.")
    lines.extend([
        "",
        "## Remaining Blocker",
        "",
        f"- `{gate['remaining_blocker']['field']}`: {gate['remaining_blocker']['reason']}",
        f"  Next evidence: {gate['remaining_blocker']['next_evidence_needed']}.",
    ])
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def run() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--event-gate", type=Path, required=True)
    parser.add_argument("--out-dir", type=Path, required=True)
    args = parser.parse_args()

    gate = build_gate(args.event_gate)
    args.out_dir.mkdir(parents=True, exist_ok=True)
    (args.out_dir / "event_hook_static_contract_gate.json").write_text(
        json.dumps(gate, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    write_markdown(args.out_dir / "EVENT_HOOK_STATIC_CONTRACT_GATE.md", gate)
    print(json.dumps({
        "ok": gate["gate"]["ok"],
        "backlog_reduced": gate["backlog_reduced"],
        "candidate_anchor_count": gate["candidate_anchor_count"],
        "rejected_static_anchor_count": gate["rejected_static_anchor_count"],
        "remaining_static_anchor_count": gate["remaining_static_anchor_count"],
        "out_dir": str(args.out_dir),
    }, indent=2))
    return 0 if gate["gate"]["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(run())
