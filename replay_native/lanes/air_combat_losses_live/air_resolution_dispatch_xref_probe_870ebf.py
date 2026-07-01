#!/usr/bin/env python3
"""Bounded xref proof for air-resolution dispatch blocks."""

from __future__ import annotations

import argparse
import hashlib
import json
import struct
import time
from pathlib import Path
from typing import Any

EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
SCHEMA = "air_resolution_dispatch_xref_probe_v1"
DEFAULT_EXE = Path(r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV\hoi4.exe")


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


def parse_runtime_functions(img: Image) -> list[dict[str, int]]:
    pdata = next((sec for sec in img.sections if sec["name"] == ".pdata"), None)
    if pdata is None:
        return []
    raw = int(pdata["raw"])
    size = int(pdata["rsize"])
    blob = img.data[raw : raw + size]
    rows = []
    text = img.section(".text")
    text_va = int(text["va"])
    text_span = max(int(text["vsize"]), int(text["rsize"]))
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


def disasm_window(img: Image, start_rva: int, end_rva: int) -> list[dict[str, str]]:
    import capstone

    start_file = img.rva_to_file(start_rva)
    end_file = img.rva_to_file(end_rva)
    if start_file is None or end_file is None or end_file <= start_file:
        return []
    md = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_64)
    rows = []
    for insn in md.disasm(img.data[start_file:end_file], img.image_base + start_rva):
        rows.append({"rva": hx(int(insn.address - img.image_base)), "text": f"{insn.mnemonic} {insn.op_str}".strip()})
    return rows


def run(args: argparse.Namespace) -> dict[str, Any]:
    import capstone

    started = time.time()
    exe = args.exe.resolve()
    img = Image(exe)
    functions = parse_runtime_functions(img)
    target_rva = args.target_rva
    target_va = img.image_base + target_rva
    text = img.section(".text")
    text_raw = int(text["raw"])
    text_size = int(text["rsize"])
    text_rva = int(text["va"])

    md = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_64)
    md.detail = True
    direct_branches = []
    function = containing_runtime_function(functions, target_rva)
    if function is not None:
        fn_start = int(str(function["begin_rva"]), 16)
        fn_end = int(str(function["end_rva"]), 16)
        fn_file = img.rva_to_file(fn_start)
        fn_end_file = img.rva_to_file(min(fn_end, target_rva + 0x20))
        if fn_file is not None and fn_end_file is not None:
            for insn in md.disasm(img.data[fn_file:fn_end_file], img.image_base + fn_start):
                if not insn.mnemonic.startswith("j"):
                    continue
                if not insn.operands or insn.operands[0].type != capstone.x86.X86_OP_IMM:
                    continue
                branch_target = int(insn.operands[0].imm - img.image_base)
                if branch_target == target_rva:
                    direct_branches.append(
                        {
                            "rva": hx(int(insn.address - img.image_base)),
                            "text": f"{insn.mnemonic} {insn.op_str}".strip(),
                            "target_rva": hx(branch_target),
                        }
                    )

    rel32_xrefs = []
    text_blob = img.data[text_raw : text_raw + text_size]
    for off in range(0, max(0, len(text_blob) - 4)):
        rel = struct.unpack_from("<i", text_blob, off)[0]
        insn_end_rva = text_rva + off + 4
        if insn_end_rva + rel == target_rva:
            candidate_insn_rva = text_rva + off - 1
            context_start = max(text_rva, candidate_insn_rva - args.xref_context_bytes)
            context_end = min(text_rva + text_size, candidate_insn_rva + args.xref_after_bytes)
            rel32_xrefs.append(
                {
                    "rel32_operand_rva": hx(text_rva + off),
                    "candidate_insn_rva": hx(candidate_insn_rva),
                    "candidate_runtime_function": containing_runtime_function(functions, candidate_insn_rva),
                    "context": disasm_window(img, context_start, context_end),
                }
            )

    absolute_xrefs = []
    target_le = struct.pack("<Q", target_va)
    for sec in img.sections:
        raw = int(sec["raw"])
        size = int(sec["rsize"])
        blob = img.data[raw : raw + size]
        pos = blob.find(target_le)
        while pos != -1:
            absolute_xrefs.append(
                {
                    "section": str(sec["name"]),
                    "rva": hx(int(sec["va"]) + pos),
                    "va_value": hx(target_va),
                }
            )
            pos = blob.find(target_le, pos + 1)

    before = disasm_window(img, max(text_rva, target_rva - args.context_bytes), target_rva + args.after_bytes)
    return {
        "schema": SCHEMA,
        "created_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "exe": str(exe),
        "exe_sha256": sha256_file(exe),
        "expected_sha256": EXPECTED_SHA256,
        "expected_sha256_match": sha256_file(exe) == EXPECTED_SHA256,
        "target_rva": hx(target_rva),
        "target_va": hx(target_va),
        "target_runtime_function": function,
        "direct_branches_to_target_in_function": direct_branches,
        "rel32_xrefs_to_target": rel32_xrefs[: args.max_xrefs],
        "rel32_xref_count": len(rel32_xrefs),
        "absolute_xrefs_to_target_va": absolute_xrefs[: args.max_xrefs],
        "absolute_xref_count": len(absolute_xrefs),
        "context": before,
        "elapsed_s": round(time.time() - started, 3),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--exe", type=Path, default=DEFAULT_EXE)
    parser.add_argument("--out-dir", type=Path, required=True)
    parser.add_argument("--target-rva", type=lambda value: int(value, 0), required=True)
    parser.add_argument("--context-bytes", type=lambda value: int(value, 0), default=0x180)
    parser.add_argument("--after-bytes", type=lambda value: int(value, 0), default=0x40)
    parser.add_argument("--xref-context-bytes", type=lambda value: int(value, 0), default=0x80)
    parser.add_argument("--xref-after-bytes", type=lambda value: int(value, 0), default=0x40)
    parser.add_argument("--max-xrefs", type=int, default=128)
    args = parser.parse_args()

    proof = run(args)
    args.out_dir.mkdir(parents=True, exist_ok=True)
    out = args.out_dir / "air_resolution_dispatch_xref_probe_870ebf.json"
    out.write_text(json.dumps(proof, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(
        json.dumps(
            {
                "out": str(out),
                "target_rva": proof["target_rva"],
                "direct_branches": len(proof["direct_branches_to_target_in_function"]),
                "rel32_xref_count": proof["rel32_xref_count"],
                "absolute_xref_count": proof["absolute_xref_count"],
                "elapsed_s": proof["elapsed_s"],
            },
            indent=2,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
