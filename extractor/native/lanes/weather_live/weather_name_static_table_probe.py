#!/usr/bin/env python3
"""Bounded static probe for weather phenomenon name parity in hoi4.exe.

The live frame only observed phenomenon values 0..2. This probe looks for a
current-build engine string cluster and pointer table that orders all known
weather phenomenon names. It is proof-only static RE, not a runtime source.
"""

from __future__ import annotations

import argparse
import json
import os
import struct
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[3]
EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
PHENOMENA = [
    "no_phenomenon",
    "rain_light",
    "rain_heavy",
    "snow",
    "blizzard",
    "arctic_water",
    "mud",
    "sandstorm",
]


def sha256_file(path: Path) -> str:
    import hashlib

    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def u16(data: bytes, off: int) -> int:
    return struct.unpack_from("<H", data, off)[0]


def u32(data: bytes, off: int) -> int:
    return struct.unpack_from("<I", data, off)[0]


def u64(data: bytes, off: int) -> int:
    return struct.unpack_from("<Q", data, off)[0]


def parse_pe(data: bytes) -> dict[str, Any]:
    if data[:2] != b"MZ":
        raise ValueError("not a PE/MZ file")
    pe_off = u32(data, 0x3C)
    if data[pe_off : pe_off + 4] != b"PE\0\0":
        raise ValueError("missing PE signature")
    coff = pe_off + 4
    number_of_sections = u16(data, coff + 2)
    size_of_optional_header = u16(data, coff + 16)
    opt = coff + 20
    magic = u16(data, opt)
    if magic != 0x20B:
        raise ValueError(f"expected PE32+, got optional header magic 0x{magic:X}")
    image_base = u64(data, opt + 24)
    section_off = opt + size_of_optional_header
    sections = []
    for i in range(number_of_sections):
        off = section_off + i * 40
        name = data[off : off + 8].split(b"\0", 1)[0].decode("ascii", errors="replace")
        virtual_size = u32(data, off + 8)
        virtual_address = u32(data, off + 12)
        raw_size = u32(data, off + 16)
        raw_ptr = u32(data, off + 20)
        sections.append(
            {
                "name": name,
                "virtual_size": virtual_size,
                "virtual_address": virtual_address,
                "raw_size": raw_size,
                "raw_ptr": raw_ptr,
            }
        )
    return {"image_base": image_base, "sections": sections}


def file_off_to_rva(pe: dict[str, Any], file_off: int) -> int | None:
    for sec in pe["sections"]:
        raw_ptr = int(sec["raw_ptr"])
        raw_size = int(sec["raw_size"])
        if raw_ptr <= file_off < raw_ptr + raw_size:
            return int(sec["virtual_address"]) + (file_off - raw_ptr)
    return None


def rva_to_file_off(pe: dict[str, Any], rva: int) -> int | None:
    for sec in pe["sections"]:
        va = int(sec["virtual_address"])
        span = max(int(sec["virtual_size"]), int(sec["raw_size"]))
        if va <= rva < va + span:
            return int(sec["raw_ptr"]) + (rva - va)
    return None


def find_all(data: bytes, needle: bytes) -> list[int]:
    out: list[int] = []
    pos = data.find(needle)
    while pos >= 0:
        out.append(pos)
        pos = data.find(needle, pos + 1)
    return out


def extract_c_strings(data: bytes, start: int, end: int) -> list[dict[str, Any]]:
    start = max(0, start)
    end = min(len(data), end)
    out = []
    i = start
    while i < end:
        while i < end and not (32 <= data[i] <= 126):
            i += 1
        j = i
        while j < end and 32 <= data[j] <= 126:
            j += 1
        if j > i and j < len(data) and data[j] == 0:
            text = data[i:j].decode("ascii", errors="replace")
            if len(text) >= 3:
                out.append({"file_offset": i, "text": text})
        i = max(j + 1, i + 1)
    return out


def weather_string_occurrences(data: bytes, pe: dict[str, Any]) -> dict[str, list[dict[str, Any]]]:
    occ: dict[str, list[dict[str, Any]]] = {}
    image_base = int(pe["image_base"])
    for name in PHENOMENA:
        rows = []
        for off in find_all(data, name.encode("ascii") + b"\0"):
            rva = file_off_to_rva(pe, off)
            rows.append(
                {
                    "file_offset": off,
                    "rva": rva,
                    "va": image_base + rva if rva is not None else None,
                }
            )
        occ[name] = rows
    return occ


def cluster_candidates(data: bytes, pe: dict[str, Any], occurrences: dict[str, list[dict[str, Any]]]) -> list[dict[str, Any]]:
    all_offsets = sorted({int(row["file_offset"]) for rows in occurrences.values() for row in rows})
    clusters: list[dict[str, Any]] = []
    seen: set[tuple[int, int]] = set()
    for anchor in all_offsets:
        start = max(0, anchor - 96)
        end = min(len(data), anchor + 384)
        strings = extract_c_strings(data, start, end)
        names = [row["text"] for row in strings if row["text"] in PHENOMENA]
        if set(names) >= set(PHENOMENA):
            first = min(row["file_offset"] for row in strings if row["text"] in PHENOMENA)
            last = max(row["file_offset"] for row in strings if row["text"] in PHENOMENA)
            key = (first, last)
            if key in seen:
                continue
            seen.add(key)
            ordered = [row["text"] for row in strings if row["text"] in PHENOMENA and first <= row["file_offset"] <= last]
            clusters.append(
                {
                    "first_file_offset": first,
                    "last_file_offset": last,
                    "first_rva": file_off_to_rva(pe, first),
                    "ordered_weather_names": ordered,
                    "matches_expected_order": ordered == PHENOMENA,
                }
            )
    return clusters


def section_bytes(data: bytes, sec: dict[str, Any]) -> tuple[int, bytes]:
    start = int(sec["raw_ptr"])
    end = min(len(data), start + int(sec["raw_size"]))
    return start, data[start:end]


def pointer_table_candidates(data: bytes, pe: dict[str, Any], occurrences: dict[str, list[dict[str, Any]]]) -> list[dict[str, Any]]:
    image_base = int(pe["image_base"])
    va_sets = {name: {int(row["va"]) for row in rows if row.get("va") is not None} for name, rows in occurrences.items()}
    rva_sets = {name: {int(row["rva"]) for row in rows if row.get("rva") is not None} for name, rows in occurrences.items()}
    candidates: list[dict[str, Any]] = []
    for sec in pe["sections"]:
        if sec["name"] not in {".rdata", ".data", ".pdata"}:
            continue
        base, blob = section_bytes(data, sec)
        limit = len(blob)
        for stride, pointer_size, mode in ((8, 8, "va64"), (16, 8, "va64_stride16"), (4, 4, "rva32"), (16, 4, "rva32_stride16")):
            needed = stride * (len(PHENOMENA) - 1) + pointer_size
            for rel in range(0, max(0, limit - needed), 4):
                ok = True
                values = []
                for idx, name in enumerate(PHENOMENA):
                    pos = rel + idx * stride
                    value = u64(blob, pos) if pointer_size == 8 else u32(blob, pos)
                    values.append(value)
                    expected = va_sets[name] if mode.startswith("va64") else rva_sets[name]
                    if value not in expected:
                        ok = False
                        break
                if ok:
                    file_off = base + rel
                    candidates.append(
                        {
                            "section": sec["name"],
                            "file_offset": file_off,
                            "rva": file_off_to_rva(pe, file_off),
                            "mode": mode,
                            "stride": stride,
                            "values": [hex(v) for v in values],
                            "resolved_names": PHENOMENA,
                            "matches_expected_order": True,
                            "image_base": hex(image_base),
                        }
                    )
    candidates.sort(key=lambda row: (row["section"], row["file_offset"], row["mode"]))
    return candidates[:64]


def classify(proof: dict[str, Any]) -> dict[str, Any]:
    clusters = proof.get("string_clusters", [])
    pointers = proof.get("pointer_table_candidates", [])
    exact_cluster = any(row.get("matches_expected_order") for row in clusters)
    exact_pointer = any(row.get("matches_expected_order") for row in pointers)
    if exact_cluster and exact_pointer:
        status = "static_engine_name_order_validated"
    elif exact_cluster:
        status = "static_string_cluster_order_validated_pointer_table_not_found"
    else:
        status = "not_resolved"
    return {
        "phenomenon_name_parity": status,
        "enum_name_order": {str(idx): name for idx, name in enumerate(PHENOMENA)} if exact_cluster else {},
        "pointer_table_evidence": "present" if exact_pointer else "not_found",
        "live_unobserved_values_retired_by_static_order": bool(exact_cluster and exact_pointer),
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
    out_path = out_dir / "weather_name_static_table_probe.json"

    proof: dict[str, Any] = {"probe": "weather_name_static_table_probe_870ebf_v1", "errors": []}
    try:
        data = exe.read_bytes()
        sha = sha256_file(exe)
        pe = parse_pe(data)
        proof["build"] = {
            "path": str(exe),
            "sha256": sha,
            "sha256_expected": EXPECTED_SHA256,
            "build_gate_ok": sha == EXPECTED_SHA256,
            "image_base": hex(int(pe["image_base"])),
        }
        if sha != EXPECTED_SHA256:
            proof["errors"].append("build gate mismatch")
        proof["weather_string_occurrences"] = weather_string_occurrences(data, pe)
        proof["string_clusters"] = cluster_candidates(data, pe, proof["weather_string_occurrences"])
        proof["pointer_table_candidates"] = pointer_table_candidates(data, pe, proof["weather_string_occurrences"])
        proof["classification"] = classify(proof)
    except Exception as exc:  # proof artifact should keep the failure reason.
        proof["errors"].append(f"{type(exc).__name__}: {exc}")
    out_path.write_text(json.dumps(proof, indent=2), encoding="utf-8")
    print(json.dumps({"proof": str(out_path), "classification": proof.get("classification"), "errors": proof["errors"]}, indent=2))
    return 0 if not proof["errors"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
