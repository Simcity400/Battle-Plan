#!/usr/bin/env python3
"""Bounded direct-callsite proof for narrowed air-resolution RVAs."""

from __future__ import annotations

import argparse
import hashlib
import json
import struct
import time
from collections import deque
from pathlib import Path
from typing import Any

EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
SCHEMA = "air_resolution_callsite_probe_v1"
DEFAULT_EXE = Path(r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV\hoi4.exe")

DEFAULT_TARGETS = {
    0xC52840: "StrategicAir_ace_management_candidate",
    0xF46A10: "Rejected_pair_shape_airwing_state_helper",
    0xF6B7B0: "Rejected_pair_shape_airmission_scalar_helper",
    0xF4E0E0: "Inner_helper_from_F46A10",
    0xF49E80: "Inner_helper_from_F6B7B0",
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

    def section(self, name: str) -> dict[str, int | str]:
        row = next((sec for sec in self.sections if sec["name"] == name), None)
        if row is None:
            raise SystemExit(f"section {name!r} not found")
        return row

    def rva_to_file(self, rva: int) -> int | None:
        for sec in self.sections:
            va = int(sec["va"])
            span = max(int(sec["vsize"]), int(sec["rsize"]))
            if va <= rva < va + span:
                return int(sec["raw"]) + (rva - va)
        return None

    def rva_in_text(self, rva: int) -> bool:
        text = self.section(".text")
        va = int(text["va"])
        span = max(int(text["vsize"]), int(text["rsize"]))
        return va <= rva < va + span


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


def callsites(args: argparse.Namespace) -> dict[str, Any]:
    import capstone

    started = time.time()
    exe = args.exe.resolve()
    img = Image(exe)
    functions = parse_runtime_functions(img)
    text = img.section(".text")
    text_raw = int(text["raw"])
    text_size = int(text["rsize"])
    text_rva = int(text["va"])
    targets = {
        int(value, 0): DEFAULT_TARGETS.get(int(value, 0), f"target_{int(value, 0):X}")
        for value in (args.target_rva or [hex(rva) for rva in DEFAULT_TARGETS])
    }
    target_vas = {img.image_base + rva: rva for rva in targets}

    md = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_64)
    md.detail = True
    rows = []
    scanned = text_size
    text_blob = img.data[text_raw : text_raw + text_size]
    target_rvas = set(targets)
    for off in range(0, max(0, len(text_blob) - 5)):
        opcode = text_blob[off]
        if opcode not in (0xE8, 0xE9):
            continue
        rel = struct.unpack_from("<i", text_blob, off + 1)[0]
        insn_rva = text_rva + off
        target_rva = insn_rva + 5 + rel
        if target_rva not in target_rvas:
            continue
        caller_function = containing_runtime_function(functions, insn_rva)
        if args.context_from_function and caller_function is not None:
            context_start_rva = int(str(caller_function["begin_rva"]), 16)
        else:
            context_start_rva = max(text_rva, insn_rva - args.context_bytes)
        context_file = img.rva_to_file(context_start_rva)
        incoming_branches = []
        if context_file is None:
            context = []
            call_text = "call/jmp rel32"
        else:
            context_blob = img.data[context_file : img.rva_to_file(insn_rva + 5) or context_file]
            context = []
            call_text = "call/jmp rel32"
            for insn in md.disasm(context_blob, img.image_base + context_start_rva):
                insn_text = f"{insn.mnemonic} {insn.op_str}".strip()
                row = {"rva": hx(int(insn.address - img.image_base)), "text": insn_text}
                context.append(row)
                if int(insn.address - img.image_base) == insn_rva:
                    call_text = insn_text
                if (
                    insn.mnemonic.startswith("j")
                    and insn.operands
                    and insn.operands[0].type == capstone.x86.X86_OP_IMM
                ):
                    branch_target_rva = int(insn.operands[0].imm - img.image_base)
                    if insn_rva - args.branch_window <= branch_target_rva <= insn_rva:
                        incoming_branches.append(
                            {
                                "rva": hx(int(insn.address - img.image_base)),
                                "text": insn_text,
                                "target_rva": hx(branch_target_rva),
                            }
                        )
        preceding = [row for row in context if row["rva"] != hx(insn_rva)][-args.preceding :]
        arg_setup = [
            row
            for row in preceding
            if any(reg in row["text"].split(",", 1)[0] for reg in ("rcx", "rdx", "r8", "r9"))
        ]
        rows.append(
            {
                "target_label": targets[target_rva],
                "target_rva": hx(target_rva),
                "call_insn_rva": hx(insn_rva),
                "call_text": call_text,
                "caller_runtime_function": caller_function,
                "preceding_instructions": preceding,
                "arg_setup_preceding": arg_setup,
                "incoming_branches_to_call_block": incoming_branches[-args.max_incoming_branches :],
            }
        )

    return {
        "schema": SCHEMA,
        "created_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "exe": str(exe),
        "exe_sha256": sha256_file(exe),
        "expected_sha256": EXPECTED_SHA256,
        "expected_sha256_match": sha256_file(exe) == EXPECTED_SHA256,
        "image_base": hx(img.image_base),
        "runtime_function_count": len(functions),
        "scan_limits": {"preceding": args.preceding},
        "target_rvas": [{"label": label, "rva": hx(rva)} for rva, label in targets.items()],
        "text_bytes_scanned": scanned,
        "direct_callsite_count": len(rows),
        "callsites": rows,
        "elapsed_s": round(time.time() - started, 3),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--exe", type=Path, default=DEFAULT_EXE)
    parser.add_argument("--out-dir", type=Path, required=True)
    parser.add_argument("--target-rva", action="append", help="Target RVA, repeatable. Defaults to narrowed pair/ace RVAs.")
    parser.add_argument("--preceding", type=int, default=14)
    parser.add_argument("--context-bytes", type=int, default=96)
    parser.add_argument("--context-from-function", action="store_true")
    parser.add_argument("--branch-window", type=lambda value: int(value, 0), default=0x20)
    parser.add_argument("--max-incoming-branches", type=int, default=32)
    args = parser.parse_args()

    proof = callsites(args)
    args.out_dir.mkdir(parents=True, exist_ok=True)
    out = args.out_dir / "air_resolution_callsite_probe_870ebf.json"
    out.write_text(json.dumps(proof, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(
        json.dumps(
            {
                "out": str(out),
                "direct_callsite_count": proof["direct_callsite_count"],
                "elapsed_s": proof["elapsed_s"],
            },
            indent=2,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
