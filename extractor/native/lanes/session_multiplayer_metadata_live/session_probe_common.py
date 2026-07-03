#!/usr/bin/env python3
"""Shared read-only helpers for domain 57 session-event root probes.

These helpers are lane-local development tooling. They use ReadProcessMemory
through extractor/memory_reader/winmem.py and never write to the game process.
"""

from __future__ import annotations

import hashlib
import struct
import sys
import time
from pathlib import Path
from typing import Any, Iterable


LANE_DIR = Path(__file__).resolve().parent
REPO_ROOT = LANE_DIR.parents[3]
MEMORY_READER = REPO_ROOT / "extractor" / "memory_reader"
if str(MEMORY_READER) not in sys.path:
    sys.path.insert(0, str(MEMORY_READER))

import winmem  # type: ignore  # noqa: E402


PROC_NAME = "hoi4.exe"
EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"

HEAP_LO = 0x0000010000000000
HEAP_HI = 0x0000800000000000
PAGE = 0x1000
SCAN_CHUNK = 16 << 20

DAILY_UPDATE_SENTINEL = bytes(
    [
        0x40,
        0x55,
        0x53,
        0x56,
        0x57,
        0x41,
        0x54,
        0x41,
        0x55,
        0x41,
        0x56,
        0x41,
        0x57,
        0x48,
        0x8D,
        0xAC,
    ]
)
GET_LONG_STRING_SENTINEL = bytes(
    [
        0x48,
        0x89,
        0x5C,
        0x24,
        0x08,
        0x48,
        0x89,
        0x6C,
        0x24,
        0x10,
        0x48,
        0x89,
        0x74,
        0x24,
        0x18,
        0x57,
    ]
)
C_GAME_STATE_DAILY_UPDATE_OFFSET = 0x1D4390
GET_LONG_STRING_OFFSET = 0x526A40
CG_STATE_GAMESTATE_OFFSET = 0x3316E90
GS_CURRENT_DATE = 0x468
GS_START_DATE = 0x4A8


def hx(value: int | None) -> str | None:
    return "0x%X" % value if isinstance(value, int) else None


def is_heap(value: int | None, align: int = 1) -> bool:
    return (
        isinstance(value, int)
        and HEAP_LO <= value < HEAP_HI
        and (align <= 1 or value % align == 0)
    )


def file_sha256(path: str | None) -> str | None:
    if not path:
        return None
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def process_info(p: winmem.Process) -> dict[str, Any]:
    return {
        "pid": p.pid,
        "exe_path": p.module_path,
        "module_base": hx(p.module_base),
        "module_size": p.module_size,
        "sha256": file_sha256(p.module_path),
    }


def read_exact(p: winmem.Process, addr: int, size: int) -> bytes | None:
    if addr <= 0 or size <= 0:
        return None
    try:
        data = p.read(addr, size)
    except Exception:
        return None
    return data if len(data) == size else None


def u64(p: winmem.Process, addr: int) -> int | None:
    data = read_exact(p, addr, 8)
    return struct.unpack("<Q", data)[0] if data is not None else None


def u32(p: winmem.Process, addr: int) -> int | None:
    data = read_exact(p, addr, 4)
    return struct.unpack("<I", data)[0] if data is not None else None


def i32(p: winmem.Process, addr: int) -> int | None:
    data = read_exact(p, addr, 4)
    return struct.unpack("<i", data)[0] if data is not None else None


def read_msvc_string_at(p: winmem.Process, addr: int, maxlen: int = 192) -> str | None:
    """Read an MSVC std::string at addr, accepting SSO or heap storage."""

    size = u64(p, addr + 0x10)
    cap = u64(p, addr + 0x18)
    if size is None or cap is None:
        return None
    if size == 0 or size > maxlen or cap < size or cap > (1 << 22):
        return None
    chars = addr if cap < 16 else u64(p, addr)
    if chars is None or chars <= 0:
        return None
    data = read_exact(p, chars, size + 1)
    if data is None or data[size] != 0:
        return None
    try:
        value = data[:size].decode("utf-8")
    except UnicodeDecodeError:
        try:
            value = data[:size].decode("latin-1")
        except UnicodeDecodeError:
            return None
    if not value or any((ord(ch) < 9 or ord(ch) == 0x7F) for ch in value):
        return None
    return value


def read_session_context(p: winmem.Process) -> dict[str, Any]:
    """Read minimal validated context used by root probes.

    This is not a full reimplementation of live_probe_session.py. It provides
    build sentinels and the current game-hour raw value for timestamping.
    """

    base = p.module_base
    daily_ok = p.try_read(base + C_GAME_STATE_DAILY_UPDATE_OFFSET, 16) == DAILY_UPDATE_SENTINEL
    long_string_ok = p.try_read(base + GET_LONG_STRING_OFFSET, 16) == GET_LONG_STRING_SENTINEL
    gs = u64(p, base + CG_STATE_GAMESTATE_OFFSET)
    context = {
        "sentinels": {
            "daily_update": {
                "rva": hx(C_GAME_STATE_DAILY_UPDATE_OFFSET),
                "ok": bool(daily_ok),
            },
            "get_long_string": {
                "rva": hx(GET_LONG_STRING_OFFSET),
                "ok": bool(long_string_ok),
            },
        },
        "cgame_state": {
            "slot_rva": hx(CG_STATE_GAMESTATE_OFFSET),
            "ptr": hx(gs),
            "heap_ptr": is_heap(gs, 8),
        },
        "game_hour_raw": None,
        "start_hour_raw": None,
    }
    if daily_ok and long_string_ok and is_heap(gs, 8):
        context["game_hour_raw"] = i32(p, gs + GS_CURRENT_DATE)
        context["start_hour_raw"] = i32(p, gs + GS_START_DATE)
    context["ok"] = bool(daily_ok and long_string_ok and is_heap(gs, 8))
    return context


def read_module_chunks(p: winmem.Process) -> list[tuple[int, bytes]]:
    chunks: list[tuple[int, bytes]] = []
    end = p.module_base + p.module_size
    for base, size in p.iter_regions(p.module_base, end, want_exec=None):
        blob = p.try_read(base, size)
        if blob:
            chunks.append((base, blob))
    return chunks


def _looks_like_decorated_rtti(raw: bytes) -> bool:
    if len(raw) < 8 or len(raw) > 220:
        return False
    if not (raw.startswith(b".?AV") or raw.startswith(b".?AU")):
        return False
    if b"@@" not in raw:
        return False
    return all(32 <= b < 127 for b in raw)


def discover_decorated_rtti_names(
    chunks: Iterable[tuple[int, bytes]], keywords: Iterable[str]
) -> list[dict[str, Any]]:
    """Find MSVC decorated RTTI names containing any keyword."""

    lowered = [kw.lower().encode("ascii") for kw in keywords if kw]
    seen: dict[bytes, dict[str, Any]] = {}
    for base, blob in chunks:
        for prefix in (b".?AV", b".?AU"):
            pos = 0
            while True:
                idx = blob.find(prefix, pos)
                if idx < 0:
                    break
                pos = idx + 1
                end = blob.find(b"\x00", idx, min(len(blob), idx + 240))
                if end < 0:
                    continue
                raw = blob[idx:end]
                if not _looks_like_decorated_rtti(raw):
                    continue
                low = raw.lower()
                hits = [kw.decode("ascii") for kw in lowered if kw in low]
                if not hits:
                    continue
                entry = seen.setdefault(
                    raw,
                    {
                        "decorated_name": raw.decode("ascii", "replace"),
                        "keyword_hits": sorted(set(hits)),
                        "string_matches": [],
                    },
                )
                entry["string_matches"].append(base + idx)
    return sorted(seen.values(), key=lambda row: row["decorated_name"])


def derive_rtti_vtables(
    p: winmem.Process, chunks: list[tuple[int, bytes]], decorated_name: bytes
) -> dict[str, Any]:
    module_start = p.module_base
    module_end = p.module_base + p.module_size
    string_addrs: list[int] = []
    for base, blob in chunks:
        pos = 0
        while True:
            idx = blob.find(decorated_name, pos)
            if idx < 0:
                break
            string_addrs.append(base + idx)
            pos = idx + 1

    col_hits: list[dict[str, int]] = []
    vtables: list[int] = []
    for string_addr in string_addrs:
        type_descriptor = string_addr - 0x10
        type_descriptor_rva = type_descriptor - module_start
        if not (module_start <= type_descriptor < module_end):
            continue
        pattern = struct.pack("<I", type_descriptor_rva)
        for base, blob in chunks:
            pos = 0
            while True:
                idx = blob.find(pattern, pos)
                if idx < 0:
                    break
                col = base + idx - 0x0C
                raw = p.try_read(col, 24)
                if len(raw) >= 24:
                    sig, off, cd_off, td_ref, class_ref, self_ref = struct.unpack(
                        "<IIIIII", raw[:24]
                    )
                    if sig in (0, 1) and td_ref == type_descriptor_rva and self_ref == col - module_start:
                        col_hits.append(
                            {
                                "string_addr": string_addr,
                                "type_descriptor": type_descriptor,
                                "complete_object_locator": col,
                                "signature": sig,
                                "offset": off,
                                "cd_offset": cd_off,
                                "class_descriptor_rva": class_ref,
                            }
                        )
                pos = idx + 1

    for hit in col_hits:
        col_ptr = struct.pack("<Q", hit["complete_object_locator"])
        for base, blob in chunks:
            pos = 0
            while True:
                idx = blob.find(col_ptr, pos)
                if idx < 0:
                    break
                vtable = base + idx + 8
                if vtable not in vtables:
                    vtables.append(vtable)
                pos = idx + 1

    return {
        "decorated_name": decorated_name.decode("ascii", "replace"),
        "string_matches": string_addrs,
        "complete_object_locators": col_hits,
        "vtables": vtables,
    }


def qword_at(data: bytes, off: int) -> int:
    return struct.unpack_from("<Q", data, off)[0]


def scan_heap_vtable_hits(
    p: winmem.Process,
    vtable_to_class: dict[int, str],
    *,
    max_hits: int = 512,
    max_seconds: float | None = None,
) -> dict[str, Any]:
    """Scan heap object starts whose first qword is a target vtable."""

    started = time.time()
    needles = {struct.pack("<Q", vt): vt for vt in vtable_to_class}
    counts: dict[str, int] = {}
    samples: list[dict[str, Any]] = []
    regions_scanned = 0
    bytes_scanned = 0
    read_failures = 0
    timed_out = False
    hit_addrs: list[int] = []

    for base, size in p.iter_regions(HEAP_LO, HEAP_HI, want_exec=False):
        if max_seconds is not None and time.time() - started > max_seconds:
            timed_out = True
            break
        regions_scanned += 1
        try:
            blob = p.read(base, size)
        except Exception:
            read_failures += 1
            continue
        bytes_scanned += len(blob)
        for needle, vt in needles.items():
            pos = 0
            while True:
                idx = blob.find(needle, pos)
                if idx < 0:
                    break
                pos = idx + 1
                addr = base + idx
                if addr % 8 != 0:
                    continue
                class_name = vtable_to_class[vt]
                counts[class_name] = counts.get(class_name, 0) + 1
                hit_addrs.append(addr)
                if len(samples) < 64:
                    samples.append(
                        {
                            "object_ptr": hx(addr),
                            "vtable": hx(vt),
                            "vtable_rva": hx(vt - p.module_base),
                            "class": class_name,
                        }
                    )
                if len(hit_addrs) >= max_hits:
                    timed_out = timed_out or max_seconds is not None and time.time() - started > max_seconds
                    return {
                        "status": "hit_limit_reached",
                        "timed_out": timed_out,
                        "regions_scanned": regions_scanned,
                        "bytes_scanned": bytes_scanned,
                        "read_failures": read_failures,
                        "hit_count": len(hit_addrs),
                        "counts_by_class": counts,
                        "samples": samples,
                        "hit_addresses": hit_addrs,
                    }

    return {
        "status": "scanned",
        "timed_out": timed_out,
        "regions_scanned": regions_scanned,
        "bytes_scanned": bytes_scanned,
        "read_failures": read_failures,
        "hit_count": len(hit_addrs),
        "counts_by_class": counts,
        "samples": samples,
        "hit_addresses": hit_addrs,
    }


def sample_object_strings(
    p: winmem.Process, object_ptr: int, *, max_offset: int = 0x300
) -> list[dict[str, Any]]:
    out: list[dict[str, Any]] = []
    for off in range(0x08, max_offset, 8):
        value = read_msvc_string_at(p, object_ptr + off)
        if value is None:
            continue
        out.append({"offset": hx(off), "text": value[:240], "length": len(value)})
        if len(out) >= 12:
            break
    return out


def find_aligned_qword_refs(
    p: winmem.Process,
    targets: list[int],
    *,
    max_refs: int = 2048,
    max_seconds: float | None = None,
) -> dict[int, list[int]]:
    """Find heap qword references to target pointers, bounded for probe use."""

    started = time.time()
    needles = {struct.pack("<Q", target): target for target in targets}
    refs: dict[int, list[int]] = {target: [] for target in targets}
    total_refs = 0
    for base, size in p.iter_regions(HEAP_LO, HEAP_HI, want_exec=False):
        if max_seconds is not None and time.time() - started > max_seconds:
            break
        try:
            blob = p.read(base, size)
        except Exception:
            continue
        for needle, target in needles.items():
            pos = 0
            while True:
                idx = blob.find(needle, pos)
                if idx < 0:
                    break
                pos = idx + 1
                addr = base + idx
                if addr % 8 != 0:
                    continue
                refs[target].append(addr)
                total_refs += 1
                if total_refs >= max_refs:
                    return refs
    return refs


def candidate_vector_owners_for_array_ref(
    p: winmem.Process,
    array_ref_addr: int,
    *,
    max_back_slots: int = 16,
    max_forward_slots: int = 64,
    max_owner_scan_seconds: float | None = 15.0,
) -> list[dict[str, Any]]:
    """Infer std::vector-like owners for an array slot that references an object."""

    owners: list[dict[str, Any]] = []
    for back in range(max_back_slots + 1):
        begin = array_ref_addr - back * 8
        if begin <= 0:
            continue
        qwords: list[int] = []
        for idx in range(max_forward_slots):
            value = u64(p, begin + idx * 8)
            if value is None:
                break
            qwords.append(value)
        if not qwords:
            continue
        logical_end = begin + len(qwords) * 8
        refs = find_aligned_qword_refs(
            p, [begin], max_refs=128, max_seconds=max_owner_scan_seconds
        ).get(begin, [])
        for owner in refs:
            end = u64(p, owner + 8)
            cap = u64(p, owner + 0x10)
            if end is None or cap is None:
                continue
            if not (begin <= end <= cap and end <= logical_end and (end - begin) % 8 == 0):
                continue
            count = (end - begin) // 8
            if count <= 0 or count > max_forward_slots:
                continue
            owners.append(
                {
                    "begin_field": hx(owner),
                    "begin": hx(begin),
                    "end": hx(end),
                    "cap": hx(cap),
                    "count": count,
                    "contains_slot": hx(array_ref_addr),
                }
            )
    dedup: dict[tuple[str | None, str | None, str | None], dict[str, Any]] = {}
    for row in owners:
        dedup[(row.get("begin_field"), row.get("begin"), row.get("end"))] = row
    return list(dedup.values())


def scan_heap_text_needles(
    p: winmem.Process,
    needles: list[str],
    *,
    max_hits: int = 128,
    max_seconds: float | None = None,
) -> dict[str, Any]:
    """Find ASCII/UTF-8 text anchors in committed heap memory."""

    encoded = {
        needle: needle.encode("utf-8")
        for needle in needles
        if needle and len(needle.encode("utf-8")) >= 4
    }
    started = time.time()
    hits: dict[str, list[dict[str, Any]]] = {needle: [] for needle in encoded}
    total = 0
    regions_scanned = 0
    bytes_scanned = 0
    timed_out = False
    for base, size in p.iter_regions(HEAP_LO, HEAP_HI, want_exec=False):
        if max_seconds is not None and time.time() - started > max_seconds:
            timed_out = True
            break
        regions_scanned += 1
        try:
            blob = p.read(base, size)
        except Exception:
            continue
        bytes_scanned += len(blob)
        for needle, raw in encoded.items():
            pos = 0
            while True:
                idx = blob.find(raw, pos)
                if idx < 0:
                    break
                pos = idx + 1
                hits[needle].append({"addr": hx(base + idx), "encoding": "utf-8"})
                total += 1
                if total >= max_hits:
                    return {
                        "status": "hit_limit_reached",
                        "timed_out": timed_out,
                        "regions_scanned": regions_scanned,
                        "bytes_scanned": bytes_scanned,
                        "hits": hits,
                        "hit_count": total,
                    }
    return {
        "status": "scanned",
        "timed_out": timed_out,
        "regions_scanned": regions_scanned,
        "bytes_scanned": bytes_scanned,
        "hits": hits,
        "hit_count": total,
    }
