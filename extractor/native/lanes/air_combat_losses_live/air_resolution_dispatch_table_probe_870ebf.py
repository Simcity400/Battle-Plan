#!/usr/bin/env python3
"""Decode current-build command dispatch tables for narrowed air RVAs."""

from __future__ import annotations

import argparse
import hashlib
import json
import struct
import time
from pathlib import Path
from typing import Any

EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
SCHEMA = "air_resolution_dispatch_table_probe_v1"
DEFAULT_EXE = Path(r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV\hoi4.exe")

DISPATCH_TABLES: list[dict[str, int | str]] = [
    {"label": "dispatch_jmp_6F9EB0", "jmp_rva": 0x6F9EB0, "cmd_base": 0x2F59, "max_index": 0xA1, "byte_table_rva": 0x6FDAF8, "target_table_rva": 0x6FDAD0},
    {"label": "dispatch_jmp_6FAB6B", "jmp_rva": 0x6FAB6B, "cmd_base": 0x3069, "max_index": 0xD8, "byte_table_rva": 0x6FDBBC, "target_table_rva": 0x6FDB9C},
    {"label": "dispatch_jmp_6FB7DC", "jmp_rva": 0x6FB7DC, "cmd_base": 0x33B8, "max_index": 0xE0, "byte_table_rva": 0x6FDCC0, "target_table_rva": 0x6FDC98},
    {"label": "dispatch_jmp_6FC010", "jmp_rva": 0x6FC010, "cmd_base": 0x35BD, "max_index": 0xEC, "byte_table_rva": 0x6FDDD0, "target_table_rva": 0x6FDDA4},
    {"label": "dispatch_jmp_6FD1AF", "jmp_rva": 0x6FD1AF, "cmd_base": 0x3A9A, "max_index": 0x9D, "byte_table_rva": 0x6FDEDC, "target_table_rva": 0x6FDEC0},
    {"label": "dispatch_jmp_6FD5ED", "jmp_rva": 0x6FD5ED, "cmd_base": 0x4AD0, "max_index": 0xE0, "byte_table_rva": 0x6FDF98, "target_table_rva": 0x6FDF7C},
]


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

    def rva_to_file(self, rva: int) -> int:
        for sec in self.sections:
            va = int(sec["va"])
            span = max(int(sec["vsize"]), int(sec["rsize"]))
            if va <= rva < va + span:
                return int(sec["raw"]) + (rva - va)
        raise ValueError(f"RVA is not mappable: {rva:#x}")

    def u8(self, rva: int) -> int:
        return self.data[self.rva_to_file(rva)]

    def u32(self, rva: int) -> int:
        return struct.unpack_from("<I", self.data, self.rva_to_file(rva))[0]


def decode_table(img: Image, table: dict[str, int | str], target_rvas: set[int]) -> dict[str, Any]:
    cmd_base = int(table["cmd_base"])
    max_index = int(table["max_index"])
    byte_table_rva = int(table["byte_table_rva"])
    target_table_rva = int(table["target_table_rva"])
    entries = []
    target_hits = []
    unique_targets: dict[int, int] = {}
    for index in range(max_index + 1):
        selector = img.u8(byte_table_rva + index)
        target_rva = img.u32(target_table_rva + selector * 4)
        command_id = cmd_base + index
        unique_targets[target_rva] = unique_targets.get(target_rva, 0) + 1
        row = {
            "command_id": hx(command_id),
            "index": hx(index),
            "selector": hx(selector),
            "target_rva": hx(target_rva),
        }
        if target_rva in target_rvas:
            target_hits.append(row)
        if len(entries) < 32 or target_rva in target_rvas:
            entries.append(row)
    return {
        "label": table["label"],
        "jmp_rva": hx(int(table["jmp_rva"])),
        "cmd_base": hx(cmd_base),
        "max_index": hx(max_index),
        "byte_table_rva": hx(byte_table_rva),
        "target_table_rva": hx(target_table_rva),
        "entry_count": max_index + 1,
        "unique_target_count": len(unique_targets),
        "target_hits": target_hits,
        "sample_entries": entries,
    }


def run(args: argparse.Namespace) -> dict[str, Any]:
    started = time.time()
    exe = args.exe.resolve()
    img = Image(exe)
    target_rvas = {int(value, 0) for value in args.target_rva}
    tables = [decode_table(img, table, target_rvas) for table in DISPATCH_TABLES]
    hits = []
    for table in tables:
        for hit in table["target_hits"]:
            hits.append({"table": table["label"], **hit})
    return {
        "schema": SCHEMA,
        "created_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "exe": str(exe),
        "exe_sha256": sha256_file(exe),
        "expected_sha256": EXPECTED_SHA256,
        "expected_sha256_match": sha256_file(exe) == EXPECTED_SHA256,
        "target_rvas": [hx(value) for value in sorted(target_rvas)],
        "tables": tables,
        "target_hit_count": len(hits),
        "target_hits": hits,
        "elapsed_s": round(time.time() - started, 3),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--exe", type=Path, default=DEFAULT_EXE)
    parser.add_argument("--out-dir", type=Path, required=True)
    parser.add_argument("--target-rva", action="append", required=True, help="Target dispatch block RVA, repeatable.")
    args = parser.parse_args()

    proof = run(args)
    args.out_dir.mkdir(parents=True, exist_ok=True)
    out = args.out_dir / "air_resolution_dispatch_table_probe_870ebf.json"
    out.write_text(json.dumps(proof, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(
        json.dumps(
            {
                "out": str(out),
                "target_rvas": proof["target_rvas"],
                "target_hit_count": proof["target_hit_count"],
                "target_hits": proof["target_hits"],
                "elapsed_s": proof["elapsed_s"],
            },
            indent=2,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
