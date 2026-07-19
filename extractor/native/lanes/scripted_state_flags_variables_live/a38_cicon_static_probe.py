#!/usr/bin/env python3
"""Exact-build static proof for CIcon fields consumed by A38/XUI22.

The probe SHA-gates the installed executable, resolves CIcon through MSVC RTTI,
validates the primary vtable, and disassembles only the virtual slots used by
the scripted-window updater.  It is deliberately bounded and read-only.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import struct
from pathlib import Path

import capstone
import pefile


ROOT = Path(__file__).resolve().parents[4]
EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
EXPECTED_IMAGE_BASE = 0x140000000
TYPE_NAME = ".?AVCIcon@@"
EXPECTED_TYPE_DESCRIPTOR_RVA = 0x330DEF8
EXPECTED_COL_RVA = 0x2DCA9D8
EXPECTED_PRIMARY_VTABLE_RVA = 0x2B341A0

SLOTS = {
    "name_delegate": 0xA8,
    "frame_setter": 0xB0,
    "enabled_getter": 0x278,
    "enable": 0x288,
    "disable": 0x290,
    "image_string_setter": 0x2D8,
    "sprite_getter": 0x2E8,
}

EXPECTED_TARGETS = {
    "name_delegate": 0x14228F080,
    "frame_setter": 0x1422E6A90,
    "enabled_getter": 0x14229ABE0,
    "enable": 0x1422E6490,
    "disable": 0x1422E6460,
    "image_string_setter": 0x1422E6410,
    "sprite_getter": 0x140CF9200,
}


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


class Image:
    def __init__(self, path: Path) -> None:
        self.path = path
        self.data = path.read_bytes()
        self.pe = pefile.PE(data=self.data, fast_load=True)
        self.base = int(self.pe.OPTIONAL_HEADER.ImageBase)

    def rva_to_file(self, rva: int) -> int:
        return int(self.pe.get_offset_from_rva(rva))

    def va_to_file(self, va: int) -> int:
        return self.rva_to_file(va - self.base)

    def file_to_rva(self, file_offset: int) -> int | None:
        for section in self.pe.sections:
            raw = int(section.PointerToRawData)
            size = int(section.SizeOfRawData)
            if raw <= file_offset < raw + size:
                return int(section.VirtualAddress) + file_offset - raw
        return None


def locate_type_descriptor(img: Image) -> int:
    name_offset = img.data.find(TYPE_NAME.encode("ascii") + b"\0")
    if name_offset < 16:
        raise RuntimeError("CIcon RTTI type name not found")
    rva = img.file_to_rva(name_offset - 16)
    if rva is None:
        raise RuntimeError("CIcon RTTI type descriptor is not mapped")
    return rva


def locate_col(img: Image, type_descriptor_rva: int) -> dict[str, int]:
    needle = struct.pack("<I", type_descriptor_rva)
    candidates = []
    pos = img.data.find(needle)
    while pos >= 0:
        start = pos - 12
        if start >= 0 and start + 24 <= len(img.data):
            rva = img.file_to_rva(start)
            if rva is not None:
                sig, obj_off, cd_off, td_rva, class_rva, self_rva = struct.unpack_from(
                    "<IIIIII", img.data, start
                )
                if sig in (0, 1) and td_rva == type_descriptor_rva and self_rva == rva:
                    candidates.append({
                        "rva": rva,
                        "va": img.base + rva,
                        "object_offset": obj_off,
                        "cd_offset": cd_off,
                        "class_descriptor_rva": class_rva,
                    })
        pos = img.data.find(needle, pos + 1)
    primary = [row for row in candidates if row["object_offset"] == 0]
    if len(primary) != 1:
        raise RuntimeError(f"expected one primary CIcon COL, found {len(primary)}")
    return primary[0]


def locate_vtable(img: Image, col_va: int) -> int:
    col_file_ref = img.data.find(struct.pack("<Q", col_va))
    if col_file_ref < 0:
        raise RuntimeError("CIcon COL reference not found")
    rva = img.file_to_rva(col_file_ref + 8)
    if rva is None:
        raise RuntimeError("CIcon vtable is not mapped")
    return rva


def disassemble_to_ret(img: Image, va: int, max_bytes: int = 0x80) -> dict[str, object]:
    start = img.va_to_file(va)
    blob = img.data[start : start + max_bytes]
    md = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_64)
    instructions = []
    consumed = 0
    for insn in md.disasm(blob, va):
        instructions.append({"address": hex(insn.address), "text": f"{insn.mnemonic} {insn.op_str}".strip()})
        consumed = (insn.address - va) + insn.size
        if insn.mnemonic in ("ret", "jmp"):
            break
    return {
        "address": hex(va),
        "rva": hex(va - img.base),
        "bytes": blob[:consumed].hex(),
        "instructions": instructions,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", default=r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV\hoi4.exe")
    parser.add_argument(
        "--out",
        default="replay_interface/re_evidence/rendering_laws/XUI22/a38_cicon_static_probe.json",
    )
    parser.add_argument("--stdout-only", action="store_true")
    args = parser.parse_args()
    exe = Path(args.exe).resolve()
    out = Path(args.out)
    if not out.is_absolute():
        out = ROOT / out

    img = Image(exe)
    exe_sha = sha256_file(exe)
    if exe_sha != EXPECTED_SHA256 or img.base != EXPECTED_IMAGE_BASE:
        raise SystemExit("hoi4.exe identity gate failed")
    type_descriptor_rva = locate_type_descriptor(img)
    col = locate_col(img, type_descriptor_rva)
    vtable_rva = locate_vtable(img, col["va"])
    if (
        type_descriptor_rva != EXPECTED_TYPE_DESCRIPTOR_RVA
        or col["rva"] != EXPECTED_COL_RVA
        or vtable_rva != EXPECTED_PRIMARY_VTABLE_RVA
    ):
        raise SystemExit("CIcon RTTI/vtable gate failed")

    vtable_file = img.rva_to_file(vtable_rva)
    slots = {}
    for name, slot_offset in SLOTS.items():
        target = struct.unpack_from("<Q", img.data, vtable_file + slot_offset)[0]
        if target != EXPECTED_TARGETS[name]:
            raise SystemExit(f"CIcon {name} target mismatch")
        slots[name] = {
            "slot_offset": hex(slot_offset),
            "target": hex(target),
            "target_rva": hex(target - img.base),
            "disassembly": disassemble_to_ret(img, target),
        }

    proof = {
        "schema": "extractor.a38_cicon_static_probe.v1",
        "read_only": True,
        "identity": {
            "program": "hoi4.exe 1.19.2.0.a729",
            "path": str(exe),
            "image_base": hex(img.base),
            "sha256": exe_sha,
        },
        "rtti": {
            "type_name": TYPE_NAME,
            "type_descriptor_rva": hex(type_descriptor_rva),
            "type_descriptor_va": hex(img.base + type_descriptor_rva),
            "complete_object_locator_rva": hex(col["rva"]),
            "complete_object_locator_va": hex(col["va"]),
            "object_offset": hex(col["object_offset"]),
            "primary_vtable_rva": hex(vtable_rva),
            "primary_vtable_va": hex(img.base + vtable_rva),
        },
        "slots": slots,
        "resolved_fields": {
            "name_delegate_object_pointer": "CIcon+0x60; virtual +0x48 on the delegated object",
            "effect_enabled_state": "u32 at CIcon+0xB0",
            "sprite_object_pointer": "pointer at CIcon+0xE8",
            "image_setter_lookup_context": "pointer at CIcon+0xF8",
        },
    }
    rendered = json.dumps(proof, indent=2, sort_keys=True) + "\n"
    if args.stdout_only:
        print(rendered, end="")
    else:
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(rendered, encoding="utf-8")
        print(json.dumps({"out": str(out), "sha256": sha256_file(out), "slots": len(slots)}, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
