#!/usr/bin/env python3
"""Shared helpers for event discovery probes.

The helpers are deliberately read-only. They use winmem.Process only for
ReadProcessMemory/process metadata and never request write rights.
"""

from __future__ import annotations

import json
import struct
import time
from pathlib import Path
from typing import Any, Iterable

import winmem  # type: ignore


EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"

HEAP_LO = 0x0000010000000000
HEAP_HI = 0x0000800000000000

CGAMESTATE_GLOBAL_RVA = 0x3316E90
COUNTRY_DATABASE_RVA = 0x33189C8
CGAMESTATE_COUNTRY_TABLE = 0x310
CGAMESTATE_COUNTRY_COUNT = 0x31C
COUNTRY_VTABLE_RVA = 0x27D0E90
COUNTRY_TAG_DB_ID = 0x08

MAX_COUNTRIES = 2048
MAX_VECTOR_COUNT = 200000


def hx(value: int | None) -> str | None:
    return "0x%X" % value if isinstance(value, int) else None


def parse_int(value: str | int) -> int:
    if isinstance(value, int):
        return value
    return int(str(value), 0)


def write_json(path: Path, payload: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def read_json(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8-sig"))


def heap_ok(value: int | None, align: int = 8) -> bool:
    return isinstance(value, int) and HEAP_LO <= value < HEAP_HI and value % align == 0


def module_ok(p: winmem.Process, value: int | None) -> bool:
    return isinstance(value, int) and p.module_base <= value < p.module_base + p.module_size


def read_exact(p: winmem.Process, addr: int, size: int) -> bytes | None:
    try:
        data = p.read(addr, size)
    except Exception:
        return None
    return data if len(data) == size else None


def try_u8(p: winmem.Process, addr: int) -> int | None:
    data = read_exact(p, addr, 1)
    return struct.unpack("<B", data)[0] if data is not None else None


def try_u32(p: winmem.Process, addr: int) -> int | None:
    data = read_exact(p, addr, 4)
    return struct.unpack("<I", data)[0] if data is not None else None


def try_i32(p: winmem.Process, addr: int) -> int | None:
    data = read_exact(p, addr, 4)
    return struct.unpack("<i", data)[0] if data is not None else None


def try_u64(p: winmem.Process, addr: int) -> int | None:
    data = read_exact(p, addr, 8)
    return struct.unpack("<Q", data)[0] if data is not None else None


def try_i64(p: winmem.Process, addr: int) -> int | None:
    data = read_exact(p, addr, 8)
    return struct.unpack("<q", data)[0] if data is not None else None


def read_cstr(p: winmem.Process, addr: int, maxlen: int = 256) -> str | None:
    raw = p.try_read(addr, maxlen)
    if not raw:
        return None
    end = raw.find(b"\0")
    if end < 0:
        return None
    for enc in ("ascii", "latin-1"):
        try:
            return raw[:end].decode(enc)
        except UnicodeDecodeError:
            pass
    return None


def read_msvc_string(p: winmem.Process, addr: int, maxlen: int = 160) -> str | None:
    size = try_u64(p, addr + 0x10)
    cap = try_u64(p, addr + 0x18)
    if not isinstance(size, int) or not isinstance(cap, int):
        return None
    if size == 0 or size > maxlen or size > cap or cap >= (1 << 24):
        return None
    data_addr = addr if cap < 16 else try_u64(p, addr)
    if not isinstance(data_addr, int):
        return None
    raw = p.try_read(data_addr, size + 1)
    if len(raw) != size + 1 or raw[size] != 0:
        return None
    for enc in ("utf-8", "latin-1"):
        try:
            text = raw[:size].decode(enc)
            if all(ch == "\t" or ch == "\n" or 32 <= ord(ch) < 127 for ch in text):
                return text
            return text
        except UnicodeDecodeError:
            pass
    return None


def rtti_name_for_vtable(p: winmem.Process, vtable: int | None) -> str | None:
    if not module_ok(p, vtable):
        return None
    col = try_u64(p, int(vtable) - 8)
    if not module_ok(p, col):
        return None
    raw = read_exact(p, int(col), 24)
    if raw is None:
        return None
    sig, _off, _cd, td_ref, _class_ref, self_ref = struct.unpack("<IIIIII", raw)
    if sig not in (0, 1) or self_ref != int(col) - p.module_base:
        return None
    td = p.module_base + td_ref
    if not module_ok(p, td):
        return None
    name = read_cstr(p, td + 0x10, 256)
    return name if name and name.startswith(".?A") else None


def rtti_name_for_object(p: winmem.Process, obj: int | None) -> str | None:
    if not heap_ok(obj):
        return None
    return rtti_name_for_vtable(p, try_u64(p, int(obj)))


def process_info(p: winmem.Process) -> dict[str, Any]:
    fp = p.fingerprint()
    return {
        "pid": p.pid,
        "module": p.module_name,
        "module_base": hx(p.module_base),
        "module_size": p.module_size,
        "module_path": p.module_path,
        "sha256": str(fp.get("sha256") or "").lower(),
    }


def fail_closed_report(probe: str, info: dict[str, Any], reason: str) -> dict[str, Any]:
    return {
        "schema": "event_probe_fail_closed/v1",
        "probe": probe,
        "expected_sha256": EXPECTED_SHA256,
        "process": info,
        "fail_closed": True,
        "failure": reason,
        "completed_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
    }


def open_process_with_sha_or_report(probe: str) -> tuple[winmem.Process | None, dict[str, Any] | None, int]:
    p = winmem.open_process("hoi4.exe")
    if p is None:
        return None, {
            "schema": "event_probe_fail_closed/v1",
            "probe": probe,
            "expected_sha256": EXPECTED_SHA256,
            "process": {"name": "hoi4.exe"},
            "fail_closed": True,
            "failure": "process_not_running",
            "completed_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        }, 3
    info = process_info(p)
    if info.get("sha256") != EXPECTED_SHA256:
        report = fail_closed_report(probe, info, "build_sha_mismatch")
        p.close()
        return None, report, 2
    return p, None, 0


def read_inline_tag(p: winmem.Process, tag_table: int, tag_id: int) -> str | None:
    raw = p.try_read(tag_table + tag_id * 0x20, 4)
    if len(raw) < 3:
        return None
    try:
        tag = raw[:3].decode("ascii")
    except UnicodeDecodeError:
        return None
    return tag if all(32 <= ord(ch) < 127 for ch in tag) else None


def resolve_country_roots(p: winmem.Process, max_countries: int | None = None) -> dict[str, Any]:
    game_state = try_u64(p, p.module_base + CGAMESTATE_GLOBAL_RVA)
    country_database = try_u64(p, p.module_base + COUNTRY_DATABASE_RVA)
    if not heap_ok(game_state) or not heap_ok(country_database):
        raise RuntimeError("CGameState or country database global did not resolve")
    tag_table = try_u64(p, int(country_database))
    tag_count = try_u32(p, int(country_database) + 0x08)
    active_count = try_u32(p, int(country_database) + 0x0C)
    country_table = try_u64(p, int(game_state) + CGAMESTATE_COUNTRY_TABLE)
    country_count = try_u32(p, int(game_state) + CGAMESTATE_COUNTRY_COUNT)
    if not heap_ok(tag_table) or not heap_ok(country_table) or not isinstance(country_count, int):
        raise RuntimeError("country table or tag table did not resolve")
    if country_count <= 0 or country_count > MAX_COUNTRIES:
        raise RuntimeError("country count out of bounds: %r" % (country_count,))

    limit = min(country_count, max_countries or country_count)
    country_vtable = p.module_base + COUNTRY_VTABLE_RVA
    rows: list[dict[str, Any]] = []
    invalid = {"nonheap": 0, "vtable_mismatch": 0, "tag_echo_mismatch": 0}
    for tag_id in range(limit):
        ptr = try_u64(p, int(country_table) + tag_id * 8)
        if not heap_ok(ptr):
            invalid["nonheap"] += 1
            continue
        vt = try_u64(p, int(ptr))
        if vt != country_vtable:
            invalid["vtable_mismatch"] += 1
            continue
        echo = try_u32(p, int(ptr) + COUNTRY_TAG_DB_ID)
        if echo != tag_id:
            invalid["tag_echo_mismatch"] += 1
            continue
        rows.append(
            {
                "tag_db_id": tag_id,
                "tag": read_inline_tag(p, int(tag_table), tag_id),
                "country_ptr": int(ptr),
            }
        )
    if not rows:
        raise RuntimeError("no validated country rows")
    return {
        "game_state": int(game_state),
        "country_database": int(country_database),
        "tag_table": int(tag_table),
        "tag_count": tag_count,
        "active_count": active_count,
        "country_table": int(country_table),
        "country_count": country_count,
        "country_vtable": country_vtable,
        "validated_countries": rows,
        "invalid_country_slots": invalid,
    }


def read_module_chunks(p: winmem.Process) -> list[tuple[int, bytes]]:
    chunks: list[tuple[int, bytes]] = []
    end = p.module_base + p.module_size
    for base, size in p.iter_regions(p.module_base, end, want_exec=None):
        blob = p.try_read(base, size)
        if blob:
            chunks.append((base, blob))
    return chunks


def iter_rtti_type_names(p: winmem.Process, chunks: list[tuple[int, bytes]], terms: Iterable[str]) -> list[str]:
    needles = [term.lower() for term in terms]
    names: set[str] = set()
    for _base, blob in chunks:
        pos = 0
        while True:
            idx = blob.find(b".?A", pos)
            if idx < 0:
                break
            pos = idx + 1
            end = blob.find(b"\0", idx)
            if end < 0 or end - idx > 220:
                continue
            raw = blob[idx:end]
            try:
                name = raw.decode("ascii")
            except UnicodeDecodeError:
                continue
            lower = name.lower()
            if any(term in lower for term in needles):
                names.add(name)
    return sorted(names)


def derive_vtables_for_type_name(
    p: winmem.Process, chunks: list[tuple[int, bytes]], decorated_name: str
) -> list[int]:
    name_bytes = decorated_name.encode("ascii", "replace")
    type_descriptors: set[int] = set()
    for base, blob in chunks:
        pos = 0
        while True:
            idx = blob.find(name_bytes, pos)
            if idx < 0:
                break
            type_descriptors.add(base + idx - 0x10)
            pos = idx + 1

    cols: set[int] = set()
    for td in type_descriptors:
        if not module_ok(p, td):
            continue
        td_rva = td - p.module_base
        pattern = struct.pack("<I", td_rva)
        for base, blob in chunks:
            pos = 0
            while True:
                idx = blob.find(pattern, pos)
                if idx < 0:
                    break
                col = base + idx - 0x0C
                raw = read_exact(p, col, 24)
                if raw is not None:
                    sig, _off, _cd, td_ref, _class_ref, self_ref = struct.unpack("<IIIIII", raw)
                    if sig in (0, 1) and td_ref == td_rva and self_ref == col - p.module_base:
                        cols.add(col)
                pos = idx + 1

    vtables: set[int] = set()
    for col in cols:
        needle = struct.pack("<Q", col)
        for base, blob in chunks:
            pos = 0
            while True:
                idx = blob.find(needle, pos)
                if idx < 0:
                    break
                vtable = base + idx + 8
                if module_ok(p, vtable):
                    vtables.add(vtable)
                pos = idx + 1
    return sorted(vtables)


def rediscover_rtti_by_terms(p: winmem.Process, terms: Iterable[str], max_names: int = 256) -> dict[str, Any]:
    chunks = read_module_chunks(p)
    names = iter_rtti_type_names(p, chunks, terms)[:max_names]
    classes = []
    for name in names:
        vtables = derive_vtables_for_type_name(p, chunks, name)
        classes.append(
            {
                "decorated_name": name,
                "vtables": [{"addr": hx(v), "rva": hx(v - p.module_base)} for v in vtables],
                "vtable_values": vtables,
            }
        )
    return {
        "terms": list(terms),
        "module_chunk_count": len(chunks),
        "module_bytes_read": sum(len(blob) for _base, blob in chunks),
        "classes": classes,
    }


def compact_rtti_report(rtti: dict[str, Any]) -> dict[str, Any]:
    return {
        "terms": rtti.get("terms", []),
        "module_chunk_count": rtti.get("module_chunk_count"),
        "module_bytes_read": rtti.get("module_bytes_read"),
        "classes": [
            {key: value for key, value in row.items() if key != "vtable_values"}
            for row in rtti.get("classes", [])
        ],
    }


def rtti_vtable_values(rtti: dict[str, Any], substrings: Iterable[str]) -> set[int]:
    needles = [s.lower() for s in substrings]
    out: set[int] = set()
    for row in rtti.get("classes", []):
        name = str(row.get("decorated_name") or "").lower()
        if any(needle in name for needle in needles):
            for value in row.get("vtable_values", []):
                if isinstance(value, int):
                    out.add(value)
    return out


def vector_shapes_at(p: winmem.Process, owner_addr: int, offset: int) -> list[dict[str, Any]]:
    addr = owner_addr + offset
    data = try_u64(p, addr)
    word8 = try_u64(p, addr + 8)
    word16 = try_u64(p, addr + 16)
    out: list[dict[str, Any]] = []
    if not heap_ok(data):
        return out
    if isinstance(word8, int):
        low = word8 & 0xFFFFFFFF
        high = word8 >> 32
        for label, count, cap in (
            ("packed_high_count_low_capacity", high, low),
            ("packed_low_count_high_capacity", low, high),
        ):
            if 0 <= count <= cap <= MAX_VECTOR_COUNT and count > 0:
                out.append({"kind": label, "data": data, "count": count, "capacity": cap})
    if isinstance(word8, int) and isinstance(word16, int):
        size32 = word8 & 0xFFFFFFFF
        cap32 = word16 & 0xFFFFFFFF
        if 0 < size32 <= cap32 <= MAX_VECTOR_COUNT:
            out.append({"kind": "u32_size_u32_capacity_at_8_16", "data": data, "count": size32, "capacity": cap32})
    return out


def sample_pointer_vector(
    p: winmem.Process,
    data: int,
    count: int,
    interesting_vtables: set[int],
    max_entries: int = 32,
) -> dict[str, Any]:
    samples: list[dict[str, Any]] = []
    heap_ptrs = 0
    interesting = 0
    rtti_counts: dict[str, int] = {}
    for index in range(min(count, max_entries)):
        ptr = try_u64(p, data + index * 8)
        if not heap_ok(ptr):
            continue
        heap_ptrs += 1
        vt = try_u64(p, int(ptr))
        name = rtti_name_for_vtable(p, vt)
        if name:
            rtti_counts[name] = rtti_counts.get(name, 0) + 1
        is_interesting = isinstance(vt, int) and vt in interesting_vtables
        if is_interesting:
            interesting += 1
        if len(samples) < 12:
            samples.append(
                {
                    "index": index,
                    "ptr": hx(ptr),
                    "vtable": hx(vt),
                    "vtable_rva": hx(vt - p.module_base) if isinstance(vt, int) else None,
                    "rtti": name,
                    "interesting_rtti": is_interesting,
                }
            )
    return {
        "sampled_entries": min(count, max_entries),
        "heap_pointer_entries": heap_ptrs,
        "interesting_rtti_entries": interesting,
        "rtti_counts": dict(sorted(rtti_counts.items(), key=lambda item: (-item[1], item[0]))[:16]),
        "samples": samples,
    }


def candidate_status_from_sample(sample: dict[str, Any], positive_oracle_rows: int = 0) -> str:
    if sample.get("interesting_rtti_entries", 0) > 0:
        return "candidate_found_needs_fixture_match" if positive_oracle_rows else "candidate_found_but_no_positive_oracle"
    if positive_oracle_rows:
        return "candidate_rejected_no_event_like_rtti"
    return "unpromoted_no_positive_fixture"
