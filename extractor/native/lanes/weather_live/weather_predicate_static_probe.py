#!/usr/bin/env python3
"""Static RTTI/vtable probe for weather predicate enum constants.

This bounded probe tries to turn phenomenon name parity from "current-frame only"
into exact static evidence by locating current-build MSVC RTTI for the
CIs* weather predicate classes and scanning their vtable functions for small
enum constants and region-weather-field offsets.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import struct
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[3]
EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"

PREDICATES = {
    "rain_light": ".?AVCIsLightRain@CIsFightingInWeatherTrigger@@",
    "rain_heavy": ".?AVCIsHeavyRain@CIsFightingInWeatherTrigger@@",
    "snow": ".?AVCIsSnow@CIsFightingInWeatherTrigger@@",
    "blizzard": ".?AVCIsBlizzard@CIsFightingInWeatherTrigger@@",
    "arctic_water": ".?AVCIsArticWater@CIsFightingInWeatherTrigger@@",
    "mud": ".?AVCIsMud@CIsFightingInWeatherTrigger@@",
    "sandstorm": ".?AVCIsSandStorm@CIsFightingInWeatherTrigger@@",
}

EXPECTED_ENUM = {
    "rain_light": 1,
    "rain_heavy": 2,
    "snow": 3,
    "blizzard": 4,
    "arctic_water": 5,
    "mud": 6,
    "sandstorm": 7,
}

EXPECTED_REGION_FLAG_OFFSETS = {
    "rain_light": 0x108,
    "rain_heavy": 0x110,
    "snow": 0x118,
    "blizzard": 0x120,
    "sandstorm": 0x128,
    "arctic_water": 0x130,
}


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def u32(data: bytes, off: int) -> int:
    return struct.unpack_from("<I", data, off)[0]


def u64(data: bytes, off: int) -> int:
    return struct.unpack_from("<Q", data, off)[0]


def qword(value: int) -> bytes:
    return struct.pack("<Q", value)


def dword(value: int) -> bytes:
    return struct.pack("<I", value)


def find_all(data: bytes, needle: bytes) -> list[int]:
    out = []
    pos = data.find(needle)
    while pos >= 0:
        out.append(pos)
        pos = data.find(needle, pos + 1)
    return out


class Image:
    def __init__(self, path: Path) -> None:
        import pefile

        self.path = path
        self.data = path.read_bytes()
        self.pe = pefile.PE(str(path), fast_load=False)
        self.image_base = int(self.pe.OPTIONAL_HEADER.ImageBase)
        self.sections = []
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

    def file_to_rva(self, off: int) -> int | None:
        for sec in self.sections:
            raw = sec["raw"]
            rsize = sec["rsize"]
            if raw <= off < raw + rsize:
                return sec["va"] + (off - raw)
        return None

    def rva_to_file(self, rva: int) -> int | None:
        for sec in self.sections:
            va = sec["va"]
            span = max(sec["vsize"], sec["rsize"])
            if va <= rva < va + span:
                return sec["raw"] + (rva - va)
        return None

    def va_to_file(self, va: int) -> int | None:
        return self.rva_to_file(va - self.image_base)

    def section_blob(self, names: set[str]) -> list[tuple[str, int, bytes]]:
        out = []
        for sec in self.sections:
            if sec["name"] in names:
                start = sec["raw"]
                end = min(len(self.data), start + sec["rsize"])
                out.append((sec["name"], start, self.data[start:end]))
        return out

    def va_in_text(self, va: int) -> bool:
        rva = va - self.image_base
        for sec in self.sections:
            if sec["name"] == ".text":
                return sec["va"] <= rva < sec["va"] + max(sec["vsize"], sec["rsize"])
        return False


def locate_type_descriptor(img: Image, decorated_name: str) -> list[dict[str, Any]]:
    rows = []
    needle = decorated_name.encode("ascii") + b"\0"
    for name_off in find_all(img.data, needle):
        td_file = name_off - 16
        rva = img.file_to_rva(td_file)
        if td_file < 0 or rva is None:
            continue
        rows.append(
            {
                "type_descriptor_file": td_file,
                "type_descriptor_rva": rva,
                "type_descriptor_va": img.image_base + rva,
                "name_file_offset": name_off,
            }
        )
    return rows


def locate_cols(img: Image, type_descriptor_rva: int) -> list[dict[str, Any]]:
    rows = []
    needle = dword(type_descriptor_rva)
    for sec_name, base, blob in img.section_blob({".rdata", ".data"}):
        pos = blob.find(needle)
        while pos >= 0:
            file_off = base + pos - 12
            if file_off >= 0 and file_off + 24 <= len(img.data):
                rva = img.file_to_rva(file_off)
                if rva is not None:
                    sig, obj_off, cd_off, td_rva, class_rva, self_rva = struct.unpack_from(
                        "<IIIIII", img.data, file_off
                    )
                    if td_rva == type_descriptor_rva and sig in (0, 1) and self_rva == rva:
                        rows.append(
                            {
                                "section": sec_name,
                                "col_file": file_off,
                                "col_rva": rva,
                                "col_va": img.image_base + rva,
                                "signature": sig,
                                "object_offset": obj_off,
                                "cd_offset": cd_off,
                                "class_descriptor_rva": class_rva,
                            }
                        )
            pos = blob.find(needle, pos + 1)
    return rows


def locate_vtables(img: Image, col_va: int) -> list[dict[str, Any]]:
    rows = []
    needle = qword(col_va)
    for sec_name, base, blob in img.section_blob({".rdata", ".data"}):
        pos = blob.find(needle)
        while pos >= 0:
            ref_file = base + pos
            vtable_file = ref_file + 8
            rva = img.file_to_rva(vtable_file)
            funcs = []
            for i in range(8):
                slot = vtable_file + i * 8
                if slot + 8 <= len(img.data):
                    fn = u64(img.data, slot)
                    if img.va_in_text(fn):
                        funcs.append({"slot": i, "va": fn, "rva": fn - img.image_base})
                    elif funcs:
                        # The weather predicate vtables are adjacent in .rdata with
                        # a COL pointer between them. Stop at the first non-code
                        # slot so a class does not inherit the next class' slots.
                        break
            if rva is not None and funcs:
                rows.append(
                    {
                        "section": sec_name,
                        "vtable_file": vtable_file,
                        "vtable_rva": rva,
                        "vtable_va": img.image_base + rva,
                        "functions": funcs,
                    }
                )
            pos = blob.find(needle, pos + 1)
    return rows


def disassemble_function(img: Image, va: int, max_bytes: int = 0x240) -> dict[str, Any]:
    import capstone

    file_off = img.va_to_file(va)
    if file_off is None:
        return {"va": va, "error": "function va not mappable"}
    blob = img.data[file_off : file_off + max_bytes]
    md = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_64)
    md.detail = True
    instructions = []
    small_immediates = []
    weather_offsets = []
    for insn in md.disasm(blob, va):
        text = f"{insn.mnemonic} {insn.op_str}".strip()
        if len(instructions) < 64:
            instructions.append({"va": insn.address, "text": text})
        for operand in insn.operands:
            if operand.type == capstone.x86.X86_OP_IMM:
                imm = int(operand.imm)
                if 0 <= imm <= 16:
                    small_immediates.append({"va": insn.address, "insn": text, "imm": imm})
            elif operand.type == capstone.x86.X86_OP_MEM:
                disp = int(operand.mem.disp)
                if disp in (0x58, 0x70, 0x78, 0x108, 0x110, 0x118, 0x120, 0x128, 0x130, 0x138):
                    weather_offsets.append({"va": insn.address, "insn": text, "disp": hex(disp)})
        if insn.mnemonic == "ret":
            break
    return {
        "va": va,
        "rva": va - img.image_base,
        "instructions": instructions,
        "small_immediates": small_immediates,
        "weather_offsets": weather_offsets,
    }


def probe_predicate(img: Image, phenomenon: str, decorated: str) -> dict[str, Any]:
    row: dict[str, Any] = {"phenomenon": phenomenon, "decorated_name": decorated}
    type_descriptors = locate_type_descriptor(img, decorated)
    row["type_descriptors"] = type_descriptors
    cols = []
    vtables = []
    function_scans = []
    seen_funcs = set()
    for td in type_descriptors:
        for col in locate_cols(img, int(td["type_descriptor_rva"])):
            cols.append(col)
            for vt in locate_vtables(img, int(col["col_va"])):
                vtables.append(vt)
                for fn in vt["functions"]:
                    va = int(fn["va"])
                    if va in seen_funcs:
                        continue
                    seen_funcs.add(va)
                    function_scans.append(disassemble_function(img, va))
    row["complete_object_locators"] = cols
    row["vtables"] = vtables
    row["function_scans"] = function_scans
    all_imms = sorted(
        {
            int(item["imm"])
            for scan in function_scans
            for item in scan.get("small_immediates", [])
        }
    )
    expected = EXPECTED_ENUM[phenomenon]
    row["small_immediates_seen"] = all_imms
    row["expected_enum_value"] = expected
    row["expected_enum_seen_in_vtable_functions"] = expected in all_imms
    offsets_seen = sorted(
        {
            int(item["disp"], 16)
            for scan in function_scans
            for item in scan.get("weather_offsets", [])
        }
    )
    row["weather_offsets_seen"] = [hex(value) for value in offsets_seen]
    if phenomenon in EXPECTED_REGION_FLAG_OFFSETS:
        expected_off = EXPECTED_REGION_FLAG_OFFSETS[phenomenon]
        row["expected_region_flag_offset"] = hex(expected_off)
        row["expected_region_flag_offset_seen"] = expected_off in offsets_seen
    return row


def classify(rows: list[dict[str, Any]]) -> dict[str, Any]:
    located = [row for row in rows if row.get("vtables")]
    exact_seen = [row for row in rows if row.get("expected_enum_seen_in_vtable_functions")]
    flag_seen = [
        row
        for row in rows
        if row.get("expected_region_flag_offset") and row.get("expected_region_flag_offset_seen")
    ]
    return {
        "predicate_rtti_vtables_located": len(located),
        "expected_enum_immediates_seen": {
            row["phenomenon"]: row["expected_enum_value"]
            for row in exact_seen
        },
        "expected_region_flag_offsets_seen": {
            row["phenomenon"]: row["expected_region_flag_offset"]
            for row in flag_seen
        },
        "phenomenon_name_parity": (
            "predicate_enum_constants_validated"
            if len(exact_seen) == len(PREDICATES)
            else "predicate_region_flag_offsets_validated_enum_constants_not_fully_proven"
            if len(flag_seen) == len(EXPECTED_REGION_FLAG_OFFSETS)
            else "predicate_rtti_located_but_enum_constants_not_fully_proven"
        ),
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--exe", default=r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV\hoi4.exe")
    ap.add_argument("--out-dir", default="extractor/native/runs/weather_live_full_replay_residuals_870ebf_20260701_v2")
    args = ap.parse_args()
    exe = Path(args.exe)
    out_dir = Path(args.out_dir)
    if not out_dir.is_absolute():
        out_dir = ROOT / out_dir
    out_dir.mkdir(parents=True, exist_ok=True)
    out_path = out_dir / "weather_predicate_static_probe.json"

    proof: dict[str, Any] = {"probe": "weather_predicate_static_probe_870ebf_v1", "errors": []}
    try:
        img = Image(exe)
        sha = sha256_file(exe)
        proof["build"] = {
            "path": str(exe),
            "sha256": sha,
            "sha256_expected": EXPECTED_SHA256,
            "build_gate_ok": sha == EXPECTED_SHA256,
            "image_base": hex(img.image_base),
        }
        if sha != EXPECTED_SHA256:
            proof["errors"].append("build gate mismatch")
        rows = [probe_predicate(img, phenomenon, decorated) for phenomenon, decorated in PREDICATES.items()]
        proof["predicates"] = rows
        proof["classification"] = classify(rows)
    except Exception as exc:
        proof["errors"].append(f"{type(exc).__name__}: {exc}")
    out_path.write_text(json.dumps(proof, indent=2), encoding="utf-8")
    print(json.dumps({"proof": str(out_path), "classification": proof.get("classification"), "errors": proof["errors"]}, indent=2))
    return 0 if not proof["errors"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
