#!/usr/bin/env python3
"""Exact-a729 static proof for X4-P3 production-line subtypes.

The legacy domain-11 ``kind`` value merges naval and ship-refit rows.  This
probe independently walks MSVC x64 RTTI in the pinned executable and freezes
the primary-vtable identity law for the four required production-line classes.
It is intentionally bounded to named type descriptors and exact expected RVAs.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import struct
from pathlib import Path
from typing import Any


EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
EXPECTED_IMAGE_BASE = 0x140000000
EXPECTED_CLASSES = {
    "military": {
        "decorated_name": ".?AVCMilitaryProductionLine@@",
        "type_descriptor_rva": 0x32313F8,
        "primary_col_rva": 0x2CD4960,
        "primary_vtable_rva": 0x2965050,
        "secondary_col_rva": 0x2CD4B68,
        "secondary_vtable_rva": 0x2965160,
    },
    "naval": {
        "decorated_name": ".?AVCNavalProductionLine@@",
        "type_descriptor_rva": 0x32314A8,
        "primary_col_rva": 0x2CD4B90,
        "primary_vtable_rva": 0x2965238,
        "secondary_col_rva": 0x2CD4C40,
        "secondary_vtable_rva": 0x2965348,
    },
    "ship_refit": {
        "decorated_name": ".?AVCShipRefitProductionLine@@",
        "type_descriptor_rva": 0x3231568,
        "primary_col_rva": 0x2CD4E68,
        "primary_vtable_rva": 0x2965858,
        "secondary_col_rva": 0x2CD4F20,
        "secondary_vtable_rva": 0x2965968,
    },
    "railway_gun": {
        "decorated_name": ".?AVCRailwayGunProductionLine@@",
        "type_descriptor_rva": 0x32314D8,
        "primary_col_rva": 0x2CD4C68,
        "primary_vtable_rva": 0x2965398,
        "secondary_col_rva": 0x2CD4D20,
        "secondary_vtable_rva": 0x29654A8,
    },
}
BASE_CLASSES = {
    "equipment_production_line": ".?AVCEquipmentProductionLine@@",
    "production_line": ".?AVCProductionLine@@",
}


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def find_all(data: bytes, needle: bytes) -> list[int]:
    hits: list[int] = []
    cursor = data.find(needle)
    while cursor >= 0:
        hits.append(cursor)
        cursor = data.find(needle, cursor + 1)
    return hits


class Image:
    def __init__(self, path: Path) -> None:
        import pefile

        self.path = path
        self.data = path.read_bytes()
        self.pe = pefile.PE(str(path), fast_load=False)
        self.image_base = int(self.pe.OPTIONAL_HEADER.ImageBase)
        self.sections: list[dict[str, Any]] = []
        for section in self.pe.sections:
            self.sections.append(
                {
                    "name": section.Name.rstrip(b"\0").decode("ascii", errors="replace"),
                    "rva": int(section.VirtualAddress),
                    "virtual_size": int(section.Misc_VirtualSize),
                    "file_offset": int(section.PointerToRawData),
                    "file_size": int(section.SizeOfRawData),
                }
            )

    def file_to_rva(self, file_offset: int) -> int | None:
        for section in self.sections:
            start = section["file_offset"]
            if start <= file_offset < start + section["file_size"]:
                return section["rva"] + file_offset - start
        return None

    def rva_to_file(self, rva: int) -> int | None:
        for section in self.sections:
            start = section["rva"]
            span = max(section["virtual_size"], section["file_size"])
            if start <= rva < start + span:
                return section["file_offset"] + rva - start
        return None

    def section_blobs(self, names: set[str]) -> list[tuple[str, int, bytes]]:
        rows = []
        for section in self.sections:
            if section["name"] not in names:
                continue
            start = section["file_offset"]
            end = min(len(self.data), start + section["file_size"])
            rows.append((section["name"], start, self.data[start:end]))
        return rows

    def is_text_va(self, va: int) -> bool:
        rva = va - self.image_base
        for section in self.sections:
            if section["name"] != ".text":
                continue
            start = section["rva"]
            return start <= rva < start + max(section["virtual_size"], section["file_size"])
        return False

    def text_blob(self) -> tuple[int, bytes]:
        for section in self.sections:
            if section["name"] == ".text":
                start = section["file_offset"]
                end = min(len(self.data), start + section["file_size"])
                return self.image_base + section["rva"], self.data[start:end]
        raise ValueError("PE has no .text section")


def locate_type_descriptors(image: Image, decorated_name: str) -> list[int]:
    rows = []
    for name_offset in find_all(image.data, decorated_name.encode("ascii") + b"\0"):
        rva = image.file_to_rva(name_offset - 16)
        if rva is not None:
            rows.append(rva)
    return sorted(set(rows))


def locate_cols(image: Image, type_descriptor_rva: int) -> list[dict[str, int]]:
    rows: list[dict[str, int]] = []
    needle = struct.pack("<I", type_descriptor_rva)
    for _section_name, base, blob in image.section_blobs({".rdata", ".data"}):
        for hit in find_all(blob, needle):
            file_offset = base + hit - 12
            if file_offset < 0 or file_offset + 24 > len(image.data):
                continue
            values = struct.unpack_from("<IIIIII", image.data, file_offset)
            signature, object_offset, cd_offset, td_rva, class_rva, self_rva = values
            col_rva = image.file_to_rva(file_offset)
            if (
                col_rva is None
                or signature not in (0, 1)
                or td_rva != type_descriptor_rva
                or self_rva != col_rva
            ):
                continue
            rows.append(
                {
                    "col_rva": col_rva,
                    "object_offset": object_offset,
                    "constructor_displacement_offset": cd_offset,
                    "class_hierarchy_descriptor_rva": class_rva,
                }
            )
    return sorted(rows, key=lambda row: (row["object_offset"], row["col_rva"]))


def locate_vtables(image: Image, col_rva: int) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    needle = struct.pack("<Q", image.image_base + col_rva)
    for _section_name, base, blob in image.section_blobs({".rdata", ".data"}):
        for hit in find_all(blob, needle):
            vtable_file = base + hit + 8
            vtable_rva = image.file_to_rva(vtable_file)
            if vtable_rva is None:
                continue
            functions = []
            for slot in range(8):
                entry_file = vtable_file + slot * 8
                if entry_file + 8 > len(image.data):
                    break
                function_va = struct.unpack_from("<Q", image.data, entry_file)[0]
                if not image.is_text_va(function_va):
                    break
                functions.append(
                    {
                        "slot": slot,
                        "function_address": hex(function_va),
                        "function_rva": hex(function_va - image.image_base),
                    }
                )
            if functions:
                rows.append({"vtable_rva": vtable_rva, "functions": functions})
    return sorted(rows, key=lambda row: row["vtable_rva"])


def probe_class(image: Image, subtype: str, expected: dict[str, Any]) -> dict[str, Any]:
    descriptors = locate_type_descriptors(image, expected["decorated_name"])
    cols = []
    for descriptor_rva in descriptors:
        for col in locate_cols(image, descriptor_rva):
            vtables = locate_vtables(image, col["col_rva"])
            cols.append({**col, "vtables": vtables})
    primary = [row for row in cols if row["object_offset"] == 0]
    secondary = [row for row in cols if row["object_offset"] == 0x50]
    observed_primary_vtables = [
        table["vtable_rva"] for row in primary for table in row["vtables"]
    ]
    observed_secondary_vtables = [
        table["vtable_rva"] for row in secondary for table in row["vtables"]
    ]
    checks = {
        "unique_type_descriptor": descriptors == [expected["type_descriptor_rva"]],
        "primary_col": [row["col_rva"] for row in primary] == [expected["primary_col_rva"]],
        "primary_vtable": observed_primary_vtables == [expected["primary_vtable_rva"]],
        "secondary_col": [row["col_rva"] for row in secondary]
        == [expected["secondary_col_rva"]],
        "secondary_vtable": observed_secondary_vtables
        == [expected["secondary_vtable_rva"]],
    }
    return {
        "subtype": subtype,
        "decorated_name": expected["decorated_name"],
        "type_descriptor_rvas": [hex(value) for value in descriptors],
        "complete_object_locators": [
            {
                **{key: hex(value) for key, value in row.items() if key != "vtables"},
                "vtables": [
                    {**table, "vtable_rva": hex(table["vtable_rva"])}
                    for table in row["vtables"]
                ],
            }
            for row in cols
        ],
        "primary_vtable_rva": hex(expected["primary_vtable_rva"]),
        "classifier_value": hex(image.image_base + expected["primary_vtable_rva"]),
        "secondary_vtable_rva": hex(expected["secondary_vtable_rva"]),
        "checks": checks,
        "exact": all(checks.values()),
    }


def probe_base_class(image: Image, decorated_name: str) -> dict[str, Any]:
    descriptors = locate_type_descriptors(image, decorated_name)
    cols = []
    for descriptor_rva in descriptors:
        for col in locate_cols(image, descriptor_rva):
            cols.append({**col, "vtables": locate_vtables(image, col["col_rva"])})
    return {
        "decorated_name": decorated_name,
        "type_descriptor_rvas": [hex(value) for value in descriptors],
        "complete_object_locators": [
            {
                "col_rva": hex(row["col_rva"]),
                "object_offset": hex(row["object_offset"]),
                "vtables": [hex(table["vtable_rva"]) for table in row["vtables"]],
            }
            for row in cols
        ],
    }


def locate_primary_vtable_code_xrefs(
    image: Image, targets: dict[int, str]
) -> dict[str, list[dict[str, Any]]]:
    """Find direct x64 RIP-relative references to each primary vtable.

    These are incidental constructor/assignment anchors, not part of the live
    classifier.  Recording them ensures useful exact-build findings survive
    even when the shared Ghidra backend cannot start a requested export.
    """
    import capstone

    text_va, blob = image.text_blob()
    md = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_64)
    md.detail = True
    md.skipdata = True
    hits = {subtype: [] for subtype in targets.values()}
    for instruction in md.disasm(blob, text_va):
        if instruction.id == 0:
            continue
        for operand in instruction.operands:
            if operand.type != capstone.x86.X86_OP_MEM:
                continue
            mem = operand.mem
            if mem.base != capstone.x86.X86_REG_RIP:
                continue
            target = instruction.address + instruction.size + int(mem.disp)
            subtype = targets.get(target)
            if subtype is None:
                continue
            file_offset = image.rva_to_file(instruction.address - image.image_base)
            following = []
            if file_offset is not None:
                for nearby in md.disasm(image.data[file_offset : file_offset + 0x100], instruction.address):
                    if nearby.id == 0:
                        break
                    following.append(
                        {
                            "address": hex(nearby.address),
                            "instruction": f"{nearby.mnemonic} {nearby.op_str}".strip(),
                        }
                    )
                    if len(following) >= 32 or nearby.mnemonic == "ret":
                        break
            hits[subtype].append(
                {
                    "instruction_address": hex(instruction.address),
                    "instruction_rva": hex(instruction.address - image.image_base),
                    "instruction": f"{instruction.mnemonic} {instruction.op_str}".strip(),
                    "target": hex(target),
                    "following_instructions": following,
                }
            )
    return hits


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--exe", default=r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV\hoi4.exe"
    )
    parser.add_argument("--out")
    parser.add_argument("--stdout-only", action="store_true")
    args = parser.parse_args()

    exe = Path(args.exe)
    image = Image(exe)
    sha256 = sha256_file(exe)
    classes = [probe_class(image, subtype, expected) for subtype, expected in EXPECTED_CLASSES.items()]
    all_exact = all(row["exact"] for row in classes)
    primary_targets = {
        image.image_base + expected["primary_vtable_rva"]: subtype
        for subtype, expected in EXPECTED_CLASSES.items()
    }
    proof = {
        "schema": "hoi4-extractor-x4-p3-line-subtype-static-proof-v1",
        "probe": "x4_p3_line_subtype_static_probe.py",
        "program": "hoi4.exe 1.19.2.0.a729",
        "binary": {
            "path": str(exe),
            "sha256": sha256,
            "expected_sha256": EXPECTED_SHA256,
            "image_base": hex(image.image_base),
            "identity_gate_ok": sha256 == EXPECTED_SHA256
            and image.image_base == EXPECTED_IMAGE_BASE,
        },
        "law": {
            "read": "u64(line_ptr+0x00)",
            "mapping": {
                row["classifier_value"]: row["subtype"] for row in classes
            },
            "unknown_policy": "reject the complete country production block",
            "legacy_kind_policy": "preserve kind for base-row compatibility; never infer subtype from kind",
        },
        "classes": classes,
        "base_class_incidental_findings": {
            key: probe_base_class(image, decorated_name)
            for key, decorated_name in BASE_CLASSES.items()
        },
        "primary_vtable_code_xrefs": locate_primary_vtable_code_xrefs(image, primary_targets),
        "verdict": "PROVEN_EXACT_PRIMARY_VTABLE_CLASSIFIER" if all_exact else "REJECTED",
        "errors": [] if all_exact else ["one or more expected RTTI/vtable checks failed"],
    }
    encoded = json.dumps(proof, indent=2, sort_keys=True) + "\n"
    if not args.stdout_only:
        if not args.out:
            parser.error("--out is required unless --stdout-only is set")
        Path(args.out).write_text(encoded, encoding="utf-8")
    print(encoded, end="")
    return 0 if proof["binary"]["identity_gate_ok"] and all_exact else 1


if __name__ == "__main__":
    raise SystemExit(main())
