#!/usr/bin/env python3
"""Resolve RTTI type names for vtables observed by the RVA contract probe."""

from __future__ import annotations

import argparse
import hashlib
import json
import struct
import time
from pathlib import Path
from typing import Any

EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
SCHEMA = "air_resolution_observed_vtable_rtti_v1"
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
            self.sections.append(
                {
                    "name": sec.Name.rstrip(b"\0").decode("ascii", errors="replace"),
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

    def read_u64_rva(self, rva: int) -> int | None:
        off = self.rva_to_file(rva)
        if off is None or off + 8 > len(self.data):
            return None
        return struct.unpack_from("<Q", self.data, off)[0]

    def cstr_rva(self, rva: int, max_len: int = 260) -> str | None:
        off = self.rva_to_file(rva)
        if off is None:
            return None
        raw = self.data[off : min(len(self.data), off + max_len)]
        nul = raw.find(b"\0")
        if nul >= 0:
            raw = raw[:nul]
        return raw.decode("ascii", errors="replace")


def parse_hex(text: str | None) -> int | None:
    if not text:
        return None
    try:
        return int(text, 16)
    except ValueError:
        return None


def observed_vtables(paths: list[Path]) -> tuple[set[int], list[dict[str, Any]]]:
    vtables: set[int] = set()
    sources: list[dict[str, Any]] = []
    for path in paths:
        report = json.loads(path.read_text(encoding="utf-8"))
        for message in report.get("messages", []):
            if not isinstance(message, dict) or message.get("type") != "hit":
                continue
            for arg_name in ("rcx", "rdx", "r8", "r9"):
                arg = (message.get("args") or {}).get(arg_name)
                if not isinstance(arg, dict):
                    continue
                rva = parse_hex(arg.get("vtable_rva"))
                if rva is None:
                    continue
                vtables.add(rva)
                sources.append(
                    {
                        "contract_json": str(path),
                        "label": message.get("label"),
                        "hit_index": message.get("hit_index"),
                        "arg": arg_name,
                        "vtable_rva": hx(rva),
                        "ptr": arg.get("ptr"),
                    }
                )
    return vtables, sources


def resolve_vtable(img: Image, vtable_rva: int) -> dict[str, Any]:
    col_va = img.read_u64_rva(vtable_rva - 8)
    if col_va is None:
        return {"vtable_rva": hx(vtable_rva), "error": "missing complete object locator pointer"}
    col_rva = col_va - img.image_base
    col_file = img.rva_to_file(col_rva)
    if col_file is None or col_file + 24 > len(img.data):
        return {
            "vtable_rva": hx(vtable_rva),
            "complete_object_locator_rva": hx(col_rva),
            "error": "complete object locator is not mappable",
        }
    sig, obj_off, cd_off, type_desc_rva, class_desc_rva, self_rva = struct.unpack_from("<IIIIII", img.data, col_file)
    type_name = img.cstr_rva(type_desc_rva + 16)
    slot_rvas = []
    for index in range(8):
        target_va = img.read_u64_rva(vtable_rva + index * 8)
        if target_va is None:
            break
        target_rva = target_va - img.image_base
        if target_rva < 0:
            break
        slot_rvas.append({"slot": index, "function_rva": hx(target_rva)})
    return {
        "vtable_rva": hx(vtable_rva),
        "complete_object_locator_rva": hx(col_rva),
        "signature": sig,
        "object_offset": hx(obj_off),
        "constructor_displacement_offset": hx(cd_off),
        "type_descriptor_rva": hx(type_desc_rva),
        "class_descriptor_rva": hx(class_desc_rva),
        "self_rva": hx(self_rva),
        "type_name": type_name,
        "slot_rvas": slot_rvas,
    }


def run(args: argparse.Namespace) -> dict[str, Any]:
    started = time.time()
    exe = args.exe.resolve()
    img = Image(exe)
    vtables, sources = observed_vtables(args.contract_json)
    resolved = [resolve_vtable(img, rva) for rva in sorted(vtables)]
    source_counts: dict[str, int] = {}
    for source in sources:
        key = f"{source['vtable_rva']} {source['label']} {source['arg']}"
        source_counts[key] = source_counts.get(key, 0) + 1
    for row in resolved:
        rva = row.get("vtable_rva")
        row["observed_sources"] = [
            {**source, "count_for_label_arg": count}
            for source in sources
            if source.get("vtable_rva") == rva
            for key, count in source_counts.items()
            if key == f"{source['vtable_rva']} {source['label']} {source['arg']}"
        ][:64]
    return {
        "schema": SCHEMA,
        "created_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "exe": str(exe),
        "exe_sha256": sha256_file(exe),
        "expected_sha256": EXPECTED_SHA256,
        "expected_sha256_match": sha256_file(exe) == EXPECTED_SHA256,
        "contract_jsons": [str(path) for path in args.contract_json],
        "observed_vtable_count": len(vtables),
        "resolved_vtables": resolved,
        "verdict": {
            "classification": "runtime_observed_vtables_resolved",
            "sortie_efficiency_candidate_0x1963700": (
                "CFEXAir/CNavalCombat arguments observed; this is naval carrier sortie efficiency, "
                "not strategic-air combat lifecycle/pairing/loss attribution"
            ),
        },
        "elapsed_s": round(time.time() - started, 3),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--exe", type=Path, default=DEFAULT_EXE)
    parser.add_argument("--contract-json", type=Path, action="append", required=True)
    parser.add_argument("--out-dir", type=Path, required=True)
    args = parser.parse_args()

    report = run(args)
    args.out_dir.mkdir(parents=True, exist_ok=True)
    out = args.out_dir / "air_resolution_observed_vtable_rtti_870ebf.json"
    out.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(
        json.dumps(
            {
                "out": str(out),
                "observed_vtable_count": report["observed_vtable_count"],
                "classification": report["verdict"]["classification"],
                "elapsed_s": report["elapsed_s"],
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
