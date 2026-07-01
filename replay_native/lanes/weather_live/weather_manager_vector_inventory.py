"""Inventory vector-like fields on CWeatherManager for weather residual triage.

This is a bounded read-only probe. It searches the current manager object for
PdxVector-shaped headers and samples their element shapes to locate any remaining
weather particle/entity container not already covered by the known region and
province weather arrays.
"""

from __future__ import annotations

import argparse
import json
import struct
import sys
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[3]
MEMORY_READER = ROOT / "memory_reader"
if str(MEMORY_READER) not in sys.path:
    sys.path.insert(0, str(MEMORY_READER))

import winmem as wm  # noqa: E402

EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
CG_STATE_DATE_OFFSET = 0x3316E90
WEATHER_CGAMESTATE_MANAGER_OFF = 0x688
WEATHER_MANAGER_VTABLE_RVA = 0x2960BE0
HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000


def is_heap(value: int) -> bool:
    return HEAP_LO <= value < HEAP_HI


def hx(value: int | None) -> str | None:
    return None if value is None else f"0x{value:X}"


def u32(buf: bytes, off: int) -> int:
    return struct.unpack_from("<I", buf, off)[0]


def u64(buf: bytes, off: int) -> int:
    return struct.unpack_from("<Q", buf, off)[0]


def read_exact(proc: wm.Process, addr: int, size: int) -> bytes | None:
    data = proc.try_read(addr, size)
    return data if len(data) == size else None


def read_c_string(proc: wm.Process, addr: int, max_len: int = 256) -> str | None:
    data = proc.try_read(addr, max_len)
    if not data:
        return None
    end = data.find(b"\x00")
    if end < 0:
        end = len(data)
    raw = data[:end]
    if not raw:
        return None
    try:
        text = raw.decode("ascii", errors="strict")
    except UnicodeDecodeError:
        return None
    if not all(ch.isprintable() for ch in text):
        return None
    return text


def rtti_name_for_vtable(proc: wm.Process, module_base: int, vtable: int) -> str | None:
    if not (module_base <= vtable < module_base + 0x5000000):
        return None
    col_ptr = proc.u64(vtable - 8)
    if not (module_base <= col_ptr < module_base + 0x5000000):
        return None
    col = proc.try_read(col_ptr, 0x18)
    if len(col) < 0x18 or u32(col, 0) != 1:
        return None
    type_desc = module_base + u32(col, 0x0C)
    self_rva = u32(col, 0x14)
    if module_base + self_rva != col_ptr:
        return None
    return read_c_string(proc, type_desc + 0x10)


def sample_vector(proc: wm.Process, module_base: int, begin: int, size: int) -> dict[str, Any]:
    out: dict[str, Any] = {
        "first_qwords": [],
        "inline_vtable_stride_candidates": [],
        "pointer_element_type_counts": {},
    }
    raw = read_exact(proc, begin, min(size, 16) * 16)
    if raw is None:
        out["read_error"] = True
        return out
    for off in range(0, min(len(raw), 128), 8):
        out["first_qwords"].append(hx(u64(raw, off)))

    for stride in (8, 16, 24, 32, 48, 64, 0x160, 0x180):
        if begin + stride * min(size, 8) < begin:
            continue
        matches: dict[str, int] = {}
        for idx in range(min(size, 8)):
            head = read_exact(proc, begin + idx * stride, 8)
            if head is None:
                continue
            vt = u64(head, 0)
            name = rtti_name_for_vtable(proc, module_base, vt)
            if name:
                matches[name] = matches.get(name, 0) + 1
        if matches:
            out["inline_vtable_stride_candidates"].append({"stride": hx(stride), "type_counts": matches})

    for idx in range(min(size, 16)):
        ptr_head = read_exact(proc, begin + idx * 8, 8)
        if ptr_head is None:
            continue
        ptr = u64(ptr_head, 0)
        if not is_heap(ptr):
            continue
        obj_head = read_exact(proc, ptr, 8)
        if obj_head is None:
            continue
        vt = u64(obj_head, 0)
        name = rtti_name_for_vtable(proc, module_base, vt) or hx(vt) or "unknown"
        counts = out["pointer_element_type_counts"]
        counts[name] = counts.get(name, 0) + 1
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--pid", type=int, default=None)
    ap.add_argument("--scan-bytes", type=lambda raw: int(raw, 0), default=0x800)
    ap.add_argument(
        "--out-dir",
        default="replay_native/runs/weather_live_full_replay_residuals_870ebf_20260701_v2",
    )
    args = ap.parse_args()
    out_dir = Path(args.out_dir)
    if not out_dir.is_absolute():
        out_dir = ROOT / out_dir
    out_dir.mkdir(parents=True, exist_ok=True)
    out_path = out_dir / "weather_manager_vector_inventory.json"

    pid = args.pid if args.pid is not None else wm.find_pid("hoi4.exe")
    proof: dict[str, Any] = {"probe": "weather_manager_vector_inventory_870ebf_v1", "errors": []}
    if pid is None:
        proof["errors"].append("hoi4.exe is not running")
        out_path.write_text(json.dumps(proof, indent=2), encoding="utf-8")
        return 1
    proc = wm.Process(pid, "hoi4.exe")
    try:
        fp = proc.fingerprint()
        module_base = proc.module_base
        sha = fp.get("sha256")
        proof["build"] = {
            "pid": pid,
            "module_base": hx(module_base),
            "module_path": fp.get("path"),
            "sha256": sha,
            "sha256_expected": EXPECTED_SHA256,
            "build_gate_ok": sha == EXPECTED_SHA256,
        }
        if sha != EXPECTED_SHA256:
            proof["errors"].append("build gate mismatch")
        gamestate = proc.u64(module_base + CG_STATE_DATE_OFFSET)
        manager = proc.u64(gamestate + WEATHER_CGAMESTATE_MANAGER_OFF) if is_heap(gamestate) else 0
        manager_vtable = proc.u64(manager) if is_heap(manager) else 0
        proof["roots"] = {
            "gamestate": hx(gamestate),
            "weather_manager": hx(manager),
            "weather_manager_vtable": hx(manager_vtable),
            "weather_manager_vtable_expected": hx(module_base + WEATHER_MANAGER_VTABLE_RVA),
        }
        if manager_vtable != module_base + WEATHER_MANAGER_VTABLE_RVA:
            proof["errors"].append("weather manager vtable mismatch")
        body = read_exact(proc, manager, args.scan_bytes) if not proof["errors"] else None
        if body is None and not proof["errors"]:
            proof["errors"].append("could not read CWeatherManager body")
        candidates = []
        if body is not None:
            for off in range(0, len(body) - 16, 8):
                begin = u64(body, off)
                size = u32(body, off + 8)
                cap = u32(body, off + 12)
                if not is_heap(begin) or size == 0 or size > 1_000_000:
                    continue
                # Some weather-local vectors have cap=0 with nonzero size, so do not reject it.
                candidate = {
                    "offset": hx(off),
                    "begin": hx(begin),
                    "size": size,
                    "cap": cap,
                    "known": {
                        "province_array": off == 0x10,
                        "region_array": off == 0x40,
                    },
                    "sample": sample_vector(proc, module_base, begin, size),
                }
                candidates.append(candidate)
        proof["vector_like_fields"] = candidates
        proof["candidate_count"] = len(candidates)
        out_path.write_text(json.dumps(proof, indent=2), encoding="utf-8")
        print(json.dumps({"proof": str(out_path), "candidate_count": len(candidates), "errors": proof["errors"]}, indent=2))
        return 0 if not proof["errors"] else 1
    finally:
        proc.close()


if __name__ == "__main__":
    raise SystemExit(main())
