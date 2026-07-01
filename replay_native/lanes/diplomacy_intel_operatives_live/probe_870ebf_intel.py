#!/usr/bin/env python3
"""Bounded current-build intel root probe for the diplomacy/intel lane.

Reads the live the game process with ReadProcessMemory only. It uses the already
validated 870ebf country roots, then validates agency, cryptology, and operative
candidate roots without touching shared collector/recorder entrypoints.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import struct
import sys
import time
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parents[3]
MEMORY_READER = REPO_ROOT / "memory_reader"
if str(MEMORY_READER) not in sys.path:
    sys.path.insert(0, str(MEMORY_READER))

import winmem  # type: ignore


EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"

HEAP_LO = 0x0000010000000000
HEAP_HI = 0x0000800000000000

CG_STATE_DATE_OFFSET = 0x3316E90
COUNTRY_DATABASE_OFFSET = 0x33189C8
CGAMESTATE_COUNTRY_TABLE = 0x310
CGAMESTATE_COUNTRY_COUNT = 0x31C
COUNTRY_VTABLE_RVA_870EBF = 0x27D0E90
OLD_COUNTRY_VTABLE_RVA_16022851 = 0x27C0E80
OBJ_TAG_DB_ID = 0x08

OBJ_AGENCY_PTR = 0xFC0
AG_NAME = 0x80
AG_ICON = 0xA0
AG_UPGRADE_PROGRESS = 0xC8
AG_RELATED_COUNT = 0xE0
AG_MAX_OPERATIVES = 0xE4
AG_USABLE_SLOTS = 0xF0
AG_USABLE_SLOTS_MIRROR = 0xF4
AG_CRYPTO_MGR = 0x120
AG_DEFENSE = 0x128
FP_SCALE = 100000.0

MGR_DATA = 0x28
MGR_SIZE = 0x30
MGR_CAP = 0x34
CR_STRIDE = 0x38
CR_VTABLE = 0x00
CR_TARGET = 0x08
CR_DECBYTE = 0x12
CR_AMOUNT = 0x18
CRYPTO_SCALE = 100000.0

OP_TYPE_ID = 4713
IDENT_CLASSDESC = -0x08
IDENT_NAME = 0x18
IDENT_NAME2 = 0x38
IDENT_DETECTED = 0x68
IDENT_LIFEFLAG = 0x110
IDENT_OWNER = 0x118
ACTIVE_OP_LIFEFLAG = 47
POOL_OP_LIFEFLAG = 31
OLD_ACTIVE_OP_CLASSDESC_RVA_16022851 = 0x292F9E0
LEADER_FROM_IDENT = -0x08
OFF_SKILL_SUBOBJ = 0xE60
OFF_XP = 0xE68
OFF_LEVEL_IN_SUBOBJ = 0x1B8
XP_SCALE = 100000.0
SANE_SKILL_MAX = 15

RTTI_NAMES = {
    "CIntelligenceAgency": b".?AVCIntelligenceAgency@@",
    "CCryptology": b".?AVCCryptology@@",
    "CCountryDecryptionState": b".?AVCCountryDecryptionState@@",
    "CUnitLeader": b".?AVCUnitLeader@@",
    "COperativeLeader": b".?AVCOperativeLeader@@",
    "CArmyLeader": b".?AVCArmyLeader@@",
    "CNavyLeader": b".?AVCNavyLeader@@",
}


def hx(value: int | None) -> str | None:
    return "0x%X" % value if isinstance(value, int) else None


def is_heap(value: int | None, align: int = 1) -> bool:
    return isinstance(value, int) and HEAP_LO <= value < HEAP_HI and value % align == 0


def is_module(p: winmem.Process, value: int | None) -> bool:
    return isinstance(value, int) and p.module_base <= value < p.module_base + p.module_size


def read_exact(p: winmem.Process, addr: int, size: int) -> bytes | None:
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


def i64(p: winmem.Process, addr: int) -> int | None:
    data = read_exact(p, addr, 8)
    return struct.unpack("<q", data)[0] if data is not None else None


def file_sha256(path: str | None) -> str | None:
    if not path:
        return None
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def read_inline_tag(p: winmem.Process, tag_table: int, tag_id: int) -> str | None:
    raw = p.try_read(tag_table + tag_id * 0x20, 4)
    if len(raw) < 3:
        return None
    try:
        tag = raw[:3].decode("ascii")
    except UnicodeDecodeError:
        return None
    return tag if all(32 <= ord(ch) < 127 for ch in tag) else None


def read_msvc_string_utf8(p: winmem.Process, addr: int, maxlen: int = 96) -> str | None:
    size = u64(p, addr + 0x10)
    cap = u64(p, addr + 0x18)
    if size is None or cap is None or size == 0 or size > maxlen or cap < size or cap > (1 << 24):
        return None
    data_addr = addr if cap < 16 else u64(p, addr)
    if data_addr is None:
        return None
    raw = p.try_read(data_addr, size + 1)
    if len(raw) != size + 1 or raw[size] != 0:
        return None
    for enc in ("utf-8", "latin-1"):
        try:
            return raw[:size].decode(enc)
        except UnicodeDecodeError:
            pass
    return None


def c_string(p: winmem.Process, addr: int, maxlen: int = 256) -> str | None:
    raw = p.try_read(addr, maxlen)
    if not raw:
        return None
    end = raw.find(b"\0")
    if end < 0:
        return None
    try:
        return raw[:end].decode("ascii")
    except UnicodeDecodeError:
        return raw[:end].decode("latin-1", "replace")


def process_info(p: winmem.Process) -> dict[str, Any]:
    return {
        "pid": p.pid,
        "exe_path": p.module_path,
        "module_base": hx(p.module_base),
        "module_size": p.module_size,
        "sha256": file_sha256(p.module_path),
    }


def read_module_chunks(p: winmem.Process) -> list[tuple[int, bytes]]:
    chunks: list[tuple[int, bytes]] = []
    end = p.module_base + p.module_size
    for base, size in p.iter_regions(p.module_base, end, want_exec=None):
        blob = p.try_read(base, size)
        if blob:
            chunks.append((base, blob))
    return chunks


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
                    sig, off, cd_off, td_ref, class_ref, self_ref = struct.unpack("<IIIIII", raw[:24])
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


def derive_rtti_set(p: winmem.Process) -> dict[str, Any]:
    chunks = read_module_chunks(p)
    classes: dict[str, Any] = {}
    for name, decorated in RTTI_NAMES.items():
        d = derive_rtti_vtables(p, chunks, decorated)
        classes[name] = {
            "decorated_name": d["decorated_name"],
            "string_matches": [hx(v) for v in d["string_matches"]],
            "complete_object_locators": [
                {
                    "string_addr": hx(hit["string_addr"]),
                    "type_descriptor": hx(hit["type_descriptor"]),
                    "complete_object_locator": hx(hit["complete_object_locator"]),
                    "class_descriptor_rva": hx(hit["class_descriptor_rva"]),
                    "offset": hit["offset"],
                    "cd_offset": hit["cd_offset"],
                }
                for hit in d["complete_object_locators"]
            ],
            "derived_vtables": [{"addr": hx(v), "rva": hx(v - p.module_base)} for v in d["vtables"]],
            "vtable_values": d["vtables"],
        }
    return {
        "classes": classes,
        "module_chunk_count": len(chunks),
        "module_bytes_read": sum(len(blob) for _, blob in chunks),
    }


def rtti_type_name_for_vtable(p: winmem.Process, vtable: int) -> str | None:
    col = u64(p, vtable - 8)
    if not is_module(p, col):
        return None
    raw = p.try_read(col, 24)
    if len(raw) < 24:
        return None
    sig, _off, _cd, td_ref, _class_ref, self_ref = struct.unpack("<IIIIII", raw[:24])
    if sig not in (0, 1) or self_ref != col - p.module_base:
        return None
    td = p.module_base + td_ref
    if not is_module(p, td):
        return None
    return c_string(p, td + 0x10, maxlen=256)


def read_country_roots(p: winmem.Process) -> dict[str, Any]:
    game_state = u64(p, p.module_base + CG_STATE_DATE_OFFSET)
    country_database = u64(p, p.module_base + COUNTRY_DATABASE_OFFSET)
    out: dict[str, Any] = {
        "game_state_ptr_source": hx(p.module_base + CG_STATE_DATE_OFFSET),
        "country_database_ptr_source": hx(p.module_base + COUNTRY_DATABASE_OFFSET),
        "game_state": hx(game_state),
        "country_database": hx(country_database),
        "status": "unreadable",
    }
    if not (is_heap(game_state, 8) and is_heap(country_database, 8)):
        return out
    tag_table = u64(p, country_database)
    tag_count = u32(p, country_database + 0x08)
    active_count = u32(p, country_database + 0x0C)
    country_table = u64(p, game_state + CGAMESTATE_COUNTRY_TABLE)
    country_count = u32(p, game_state + CGAMESTATE_COUNTRY_COUNT)
    out.update(
        {
            "tag_table": hx(tag_table),
            "tag_count": tag_count,
            "active_count": active_count,
            "country_table": hx(country_table),
            "country_count": country_count,
            "status": "read",
        }
    )
    return out


def load_tags(p: winmem.Process, roots: dict[str, Any]) -> dict[int, str]:
    tag_table = int(roots.get("tag_table") or "0", 16)
    tag_count = int(roots.get("tag_count") or 0)
    tags: dict[int, str] = {}
    if not is_heap(tag_table, 8):
        return tags
    for tag_id in range(min(tag_count, 2048)):
        tag = read_inline_tag(p, tag_table, tag_id)
        if tag:
            tags[tag_id] = tag
    return tags


def validate_country_objects(
    p: winmem.Process, roots: dict[str, Any], tags: dict[int, str]
) -> tuple[dict[int, int], dict[str, Any]]:
    table = int(roots.get("country_table") or "0", 16)
    count = int(roots.get("country_count") or 0)
    current_vt = p.module_base + COUNTRY_VTABLE_RVA_870EBF
    old_vt = p.module_base + OLD_COUNTRY_VTABLE_RVA_16022851
    countries: dict[int, int] = {}
    old_valid = 0
    bad_echo = 0
    samples: list[dict[str, Any]] = []
    if not is_heap(table, 8):
        return countries, {"status": "country_table_unreadable"}
    for tag_id in sorted(tags):
        if tag_id >= count:
            continue
        ptr = u64(p, table + tag_id * 8)
        if not is_heap(ptr, 8):
            continue
        vt = u64(p, ptr)
        if vt == old_vt:
            old_valid += 1
        if vt != current_vt:
            continue
        if u32(p, ptr + OBJ_TAG_DB_ID) != tag_id:
            bad_echo += 1
            continue
        countries[tag_id] = ptr
        if len(samples) < 12:
            samples.append({"tag_id": tag_id, "tag": tags.get(tag_id), "country_ptr": hx(ptr)})
    return countries, {
        "current_country_vtable_rva": hx(COUNTRY_VTABLE_RVA_870EBF),
        "old_country_vtable_rva": hx(OLD_COUNTRY_VTABLE_RVA_16022851),
        "valid_current_countries": len(countries),
        "old_vtable_valid_country_rows": old_valid,
        "bad_tag_echo_current_rows": bad_echo,
        "samples": samples,
        "status": "validated" if len(countries) >= 40 else "unvalidated",
    }


def vtable_set(rtti: dict[str, Any], class_name: str) -> set[int]:
    return {
        int(v)
        for v in rtti.get("classes", {}).get(class_name, {}).get("vtable_values", [])
        if isinstance(v, int)
    }


def read_agencies(
    p: winmem.Process, countries: dict[int, int], tags: dict[int, str], rtti: dict[str, Any]
) -> tuple[dict[int, dict[str, Any]], dict[str, Any]]:
    agency_vtables = vtable_set(rtti, "CIntelligenceAgency")
    agencies: dict[int, dict[str, Any]] = {}
    agency_ptr_nonheap = 0
    agency_ptr_null = 0
    vtable_reject = 0
    field_reject = 0
    samples: list[dict[str, Any]] = []
    for tag_id, country in sorted(countries.items()):
        ag = u64(p, country + OBJ_AGENCY_PTR)
        if not ag:
            agency_ptr_null += 1
            continue
        if not is_heap(ag, 8):
            agency_ptr_nonheap += 1
            continue
        vt = u64(p, ag)
        if agency_vtables and vt not in agency_vtables:
            vtable_reject += 1
            continue
        max_ops = i32(p, ag + AG_MAX_OPERATIVES)
        slots = i32(p, ag + AG_USABLE_SLOTS)
        slots_mirror = i32(p, ag + AG_USABLE_SLOTS_MIRROR)
        defense_raw = i32(p, ag + AG_DEFENSE)
        upgrade_raw = i32(p, ag + AG_UPGRADE_PROGRESS)
        related = i32(p, ag + AG_RELATED_COUNT)
        name = read_msvc_string_utf8(p, ag + AG_NAME, maxlen=96)
        icon = read_msvc_string_utf8(p, ag + AG_ICON, maxlen=160)
        sane = (
            max_ops is not None
            and 0 <= max_ops <= 64
            and slots is not None
            and 0 <= slots <= 64
            and defense_raw is not None
            and 0 <= defense_raw <= 50_000_000
            and upgrade_raw is not None
            and 0 <= upgrade_raw <= 200_000_000
            and (name is not None or max_ops > 0)
        )
        if not sane:
            field_reject += 1
            continue
        row = {
            "tag_id": tag_id,
            "tag": tags.get(tag_id),
            "country_ptr": hx(country),
            "agency_ptr": hx(ag),
            "agency_vtable": hx(vt),
            "agency_vtable_rva": hx((vt or 0) - p.module_base) if vt else None,
            "name": name,
            "icon": icon,
            "upgrade_progress_raw": upgrade_raw,
            "upgrade_progress": upgrade_raw / FP_SCALE if upgrade_raw is not None else None,
            "max_operative_count": max_ops,
            "related_count_0xE0": related,
            "usable_operative_slots": slots,
            "usable_operative_slots_mirror": slots_mirror,
            "defense_raw": defense_raw,
            "defense": defense_raw / FP_SCALE if defense_raw is not None else None,
        }
        agencies[tag_id] = row
        if len(samples) < 20 and (name or max_ops):
            samples.append(row)
    named = sum(1 for row in agencies.values() if row.get("name"))
    return agencies, {
        "country_agency_ptr_offset": hx(OBJ_AGENCY_PTR),
        "agency_vtable_rtti": [
            {"addr": hx(v), "rva": hx(v - p.module_base)} for v in sorted(agency_vtables)
        ],
        "valid_agencies": len(agencies),
        "named_agencies": named,
        "null_agency_ptrs": agency_ptr_null,
        "nonheap_agency_ptrs": agency_ptr_nonheap,
        "vtable_rejects": vtable_reject,
        "field_rejects": field_reject,
        "samples": samples,
        "status": "validated" if len(agencies) >= 10 and named >= 5 else "unvalidated",
    }


def read_cryptology(
    p: winmem.Process, agencies: dict[int, dict[str, Any]], tags: dict[int, str], rtti: dict[str, Any]
) -> dict[str, Any]:
    crypt_vtables = vtable_set(rtti, "CCryptology")
    state_vtables = vtable_set(rtti, "CCountryDecryptionState")
    valid_targets: list[dict[str, Any]] = []
    manager_heap = 0
    manager_vtable_match = 0
    manager_unreadable = 0
    data_heap = 0
    records_scanned = 0
    state_vtable_rejects = 0
    target_rejects = 0
    amount_rejects = 0
    samples: list[dict[str, Any]] = []
    valid_ids = set(tags)

    for owner_id, agency in sorted(agencies.items()):
        ag = int(str(agency["agency_ptr"]), 16)
        mgr = u64(p, ag + AG_CRYPTO_MGR)
        if not is_heap(mgr, 8):
            continue
        manager_heap += 1
        mvt = u64(p, mgr)
        if crypt_vtables and mvt in crypt_vtables:
            manager_vtable_match += 1
        elif crypt_vtables:
            manager_unreadable += 1
            continue
        datap = u64(p, mgr + MGR_DATA)
        size = u32(p, mgr + MGR_SIZE)
        cap = u32(p, mgr + MGR_CAP)
        if not is_heap(datap, 8):
            continue
        data_heap += 1
        scan_n = cap if cap and 0 < cap <= 256 else (size if size and 0 < size <= 256 else 0)
        for i in range(scan_n):
            rec = datap + i * CR_STRIDE
            rb = p.try_read(rec, CR_STRIDE)
            if len(rb) < CR_STRIDE:
                break
            records_scanned += 1
            vt = struct.unpack_from("<Q", rb, CR_VTABLE)[0]
            target_id = struct.unpack_from("<i", rb, CR_TARGET)[0]
            amount_raw = struct.unpack_from("<q", rb, CR_AMOUNT)[0]
            decrypting = rb[CR_DECBYTE] != 0
            if state_vtables and vt not in state_vtables:
                state_vtable_rejects += 1
                continue
            if target_id not in valid_ids:
                target_rejects += 1
                continue
            if not (0 < amount_raw < 10**12):
                amount_rejects += 1
                continue
            row = {
                "owner_id": owner_id,
                "owner_tag": tags.get(owner_id),
                "agency_ptr": agency["agency_ptr"],
                "cryptology_mgr": hx(mgr),
                "manager_vtable": hx(mvt),
                "manager_vtable_rva": hx((mvt or 0) - p.module_base) if mvt else None,
                "record_ptr": hx(rec),
                "record_index": i,
                "record_vtable": hx(vt),
                "record_vtable_rva": hx(vt - p.module_base),
                "target_id": target_id,
                "target_tag": tags.get(target_id),
                "amount_raw": amount_raw,
                "amount": amount_raw / CRYPTO_SCALE,
                "decrypting": decrypting,
            }
            valid_targets.append(row)
            if len(samples) < 24:
                samples.append(row)

    owners = sorted({row["owner_tag"] for row in valid_targets if row.get("owner_tag")})
    return {
        "agency_crypto_manager_offset": hx(AG_CRYPTO_MGR),
        "manager_data_offset": hx(MGR_DATA),
        "manager_size_offset": hx(MGR_SIZE),
        "manager_cap_offset": hx(MGR_CAP),
        "record_stride": hx(CR_STRIDE),
        "record_target_offset": hx(CR_TARGET),
        "record_decrypting_byte_offset": hx(CR_DECBYTE),
        "record_amount_offset": hx(CR_AMOUNT),
        "cryptology_vtable_rtti": [
            {"addr": hx(v), "rva": hx(v - p.module_base)} for v in sorted(crypt_vtables)
        ],
        "country_decryption_state_vtable_rtti": [
            {"addr": hx(v), "rva": hx(v - p.module_base)} for v in sorted(state_vtables)
        ],
        "manager_heap_candidates": manager_heap,
        "manager_vtable_matches": manager_vtable_match,
        "manager_vtable_rejects": manager_unreadable,
        "data_heap_candidates": data_heap,
        "records_scanned": records_scanned,
        "valid_targets": len(valid_targets),
        "owner_count": len(owners),
        "owners": owners,
        "state_vtable_rejects": state_vtable_rejects,
        "target_rejects": target_rejects,
        "amount_rejects": amount_rejects,
        "samples": samples,
        "status": "validated" if valid_targets else ("manager_validated_no_targets" if manager_vtable_match else "unvalidated"),
    }


def decode_skill_xp(p: winmem.Process, ident: int) -> tuple[int | None, float | None, int | None]:
    leader = ident + LEADER_FROM_IDENT
    xp_raw = i64(p, leader + OFF_XP)
    sub = u64(p, leader + OFF_SKILL_SUBOBJ)
    skill = None
    if is_heap(sub, 8):
        s = u32(p, sub + OFF_LEVEL_IN_SUBOBJ)
        if s is not None and s <= SANE_SKILL_MAX:
            skill = int(s)
    xp = xp_raw / XP_SCALE if xp_raw is not None and 0 <= xp_raw < 10**12 else None
    return skill, xp, xp_raw


def scan_operatives(
    p: winmem.Process, tags: dict[int, str], agencies: dict[int, dict[str, Any]], max_seconds: float | None
) -> dict[str, Any]:
    started = time.time()
    needle = struct.pack("<i", OP_TYPE_ID)
    old_active_desc = p.module_base + OLD_ACTIVE_OP_CLASSDESC_RVA_16022851
    old_shifted_desc = old_active_desc + (COUNTRY_VTABLE_RVA_870EBF - OLD_COUNTRY_VTABLE_RVA_16022851)
    valid_ids = set(tags)
    groups: dict[tuple[int, int, bool], int] = {}
    active: dict[int, dict[str, Any]] = {}
    old_desc_active = 0
    shifted_desc_active = 0
    dual_name_candidates = 0
    owner_invalid = 0
    skill_ok = 0
    xp_ok = 0
    detected_values: dict[int, int] = {}
    region_count = 0
    bytes_scanned = 0
    read_failures = 0
    timed_out = False
    samples: list[dict[str, Any]] = []

    for base, size in p.iter_regions(HEAP_LO, HEAP_HI, want_exec=False):
        if max_seconds is not None and time.time() - started > max_seconds:
            timed_out = True
            break
        region_count += 1
        pos = 0
        chunk = 64 << 20
        while pos < size:
            if max_seconds is not None and time.time() - started > max_seconds:
                timed_out = True
                break
            want = min(chunk, size - pos)
            blob = p.try_read(base + pos, want)
            if not blob:
                read_failures += 1
                pos += max(want, 0x1000)
                continue
            bytes_scanned += len(blob)
            search = 0
            while True:
                idx = blob.find(needle, search)
                if idx < 0:
                    break
                search = idx + 1
                if idx + 8 > len(blob):
                    continue
                op_id = struct.unpack_from("<i", blob, idx + 4)[0]
                if op_id <= 0:
                    continue
                ident = base + pos + idx
                name = read_msvc_string_utf8(p, ident + IDENT_NAME, maxlen=64)
                if not name:
                    continue
                name2 = read_msvc_string_utf8(p, ident + IDENT_NAME2, maxlen=64)
                if name2 != name:
                    continue
                dual_name_candidates += 1
                owner_id = i32(p, ident + IDENT_OWNER)
                owner_valid = owner_id in valid_ids
                if not owner_valid:
                    owner_invalid += 1
                lifeflag = i32(p, ident + IDENT_LIFEFLAG)
                classdesc = u64(p, ident + IDENT_CLASSDESC)
                if lifeflag is None or classdesc is None:
                    continue
                groups[(classdesc, lifeflag, bool(owner_valid))] = groups.get((classdesc, lifeflag, bool(owner_valid)), 0) + 1
                if classdesc == old_active_desc and lifeflag == ACTIVE_OP_LIFEFLAG and owner_valid:
                    old_desc_active += 1
                if classdesc == old_shifted_desc and lifeflag == ACTIVE_OP_LIFEFLAG and owner_valid:
                    shifted_desc_active += 1
                if lifeflag != ACTIVE_OP_LIFEFLAG or not owner_valid:
                    continue
                if op_id in active:
                    continue
                detected_raw = i32(p, ident + IDENT_DETECTED)
                if detected_raw is not None:
                    detected_values[detected_raw] = detected_values.get(detected_raw, 0) + 1
                skill, xp, xp_raw = decode_skill_xp(p, ident)
                if skill is not None:
                    skill_ok += 1
                if xp is not None:
                    xp_ok += 1
                row = {
                    "op_id": op_id,
                    "name": name,
                    "owner_id": owner_id,
                    "owner_tag": tags.get(owner_id) if owner_id is not None else None,
                    "identity_addr": hx(ident),
                    "leader_addr": hx(ident + LEADER_FROM_IDENT),
                    "classdesc": hx(classdesc),
                    "classdesc_rva": hx(classdesc - p.module_base),
                    "classdesc_rtti": rtti_type_name_for_vtable(p, classdesc) if is_module(p, classdesc) else None,
                    "lifeflag": lifeflag,
                    "detected_raw": detected_raw,
                    "detected": detected_raw == 3 if detected_raw is not None else None,
                    "skill": skill,
                    "experience": xp,
                    "experience_raw": xp_raw,
                }
                active[op_id] = row
                if len(samples) < 24:
                    samples.append(row)
            got = len(blob)
            if pos + got >= size or got < want:
                pos += max(got, 0x1000)
            else:
                pos += got - 16
        if timed_out:
            break

    group_rows = []
    for (classdesc, lifeflag, owner_valid), count in sorted(groups.items(), key=lambda item: (-item[1], item[0])):
        group_rows.append(
            {
                "classdesc": hx(classdesc),
                "classdesc_rva": hx(classdesc - p.module_base),
                "classdesc_rtti": rtti_type_name_for_vtable(p, classdesc) if is_module(p, classdesc) else None,
                "lifeflag": lifeflag,
                "owner_valid": owner_valid,
                "count": count,
            }
        )

    by_owner: dict[str, int] = {}
    exceeds_max: list[dict[str, Any]] = []
    for row in active.values():
        owner_tag = str(row.get("owner_tag") or row.get("owner_id"))
        by_owner[owner_tag] = by_owner.get(owner_tag, 0) + 1
    for owner_id, agency in agencies.items():
        tag = agency.get("tag")
        n = by_owner.get(str(tag), 0)
        max_ops = agency.get("max_operative_count")
        if isinstance(max_ops, int) and n > max_ops:
            exceeds_max.append({"tag": tag, "active_operatives": n, "max_operative_count": max_ops})

    active_class_rvas = sorted({row["classdesc_rva"] for row in active.values() if row.get("classdesc_rva")})
    root_validated = bool(active) and not timed_out and skill_ok == len(active) and xp_ok == len(active)
    return {
        "handle_type": OP_TYPE_ID,
        "identity_offsets": {
            "type": hx(0),
            "op_id": hx(4),
            "classdesc": hx(IDENT_CLASSDESC),
            "name": hx(IDENT_NAME),
            "name_duplicate": hx(IDENT_NAME2),
            "detected": hx(IDENT_DETECTED),
            "lifeflag": hx(IDENT_LIFEFLAG),
            "owner": hx(IDENT_OWNER),
        },
        "skill_xp_offsets": {
            "leader_from_identity": hx(LEADER_FROM_IDENT),
            "skill_subobject": hx(OFF_SKILL_SUBOBJ),
            "xp_raw": hx(OFF_XP),
            "skill_level_in_subobject": hx(OFF_LEVEL_IN_SUBOBJ),
        },
        "old_active_classdesc_rva": hx(OLD_ACTIVE_OP_CLASSDESC_RVA_16022851),
        "old_plus_country_delta_classdesc_rva": hx(OLD_ACTIVE_OP_CLASSDESC_RVA_16022851 + (COUNTRY_VTABLE_RVA_870EBF - OLD_COUNTRY_VTABLE_RVA_16022851)),
        "old_active_desc_hits": old_desc_active,
        "old_plus_country_delta_hits": shifted_desc_active,
        "regions_scanned": region_count,
        "bytes_scanned": bytes_scanned,
        "read_failures": read_failures,
        "timed_out": timed_out,
        "dual_name_candidates": dual_name_candidates,
        "owner_invalid_dual_name_candidates": owner_invalid,
        "active_operatives": len(active),
        "active_classdesc_rvas": active_class_rvas,
        "skill_decoded": skill_ok,
        "experience_decoded": xp_ok,
        "detected_raw_distribution": detected_values,
        "owner_counts": dict(sorted(by_owner.items())),
        "owner_count_exceeds_agency_max_diagnostic": exceeds_max,
        "owner_count_note": "diagnostic only; max_operative_count is not a hard cap for all COperativeLeader identities",
        "classdesc_groups": group_rows[:24],
        "samples": samples,
        "status": "validated" if root_validated else ("partial_timeout" if timed_out else "unvalidated"),
    }


def compact_rtti_for_report(rtti: dict[str, Any]) -> dict[str, Any]:
    return {
        "module_chunk_count": rtti.get("module_chunk_count"),
        "module_bytes_read": rtti.get("module_bytes_read"),
        "classes": {
            name: {
                key: value
                for key, value in info.items()
                if key != "vtable_values"
            }
            for name, info in rtti.get("classes", {}).items()
        },
    }


def build_summary(report: dict[str, Any]) -> dict[str, Any]:
    accepted: list[str] = []
    rejected: list[str] = []
    inconclusive: list[str] = []

    if report.get("countries", {}).get("status") == "validated":
        accepted.append("current 870ebf country roots reused for intel probe")
    else:
        inconclusive.append("current country roots")

    agency = report.get("agencies", {})
    if agency.get("status") == "validated":
        accepted.append("country+0xFC0 agency pointer and CIntelligenceAgency RTTI vtable")
    else:
        inconclusive.append("agency root country+0xFC0")

    crypto = report.get("cryptology", {})
    if crypto.get("status") == "validated":
        accepted.append("agency+0x120 CCryptology manager and CCountryDecryptionState stride 0x38 records")
    elif crypto.get("status") == "manager_validated_no_targets":
        accepted.append("agency+0x120 CCryptology manager")
        inconclusive.append("cryptology target records not observed")
    else:
        inconclusive.append("cryptology root")

    ops = report.get("operatives", {})
    if ops.get("status") == "validated":
        accepted.append("active operative dual-name identity roots and skill/xp offsets")
        if ops.get("old_active_desc_hits") == 0:
            rejected.append("old active operative classdesc RVA 0x292F9E0 for current 870ebf")
        if ops.get("old_plus_country_delta_hits") == 0:
            rejected.append("old active operative classdesc plus country delta for current 870ebf")
    elif ops.get("status") == "skipped":
        inconclusive.append("operative root skipped")
    else:
        inconclusive.append("operative root")

    return {
        "accepted_current_build_evidence": accepted,
        "rejected_current_build_candidates": rejected,
        "inconclusive_or_not_observed": inconclusive,
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True)
    ap.add_argument("--skip-operatives", action="store_true")
    ap.add_argument("--operative-timeout", type=float, default=None, help="optional seconds cap for operative heap scan")
    ns = ap.parse_args()

    out_path = Path(ns.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)

    p = winmem.open_process("hoi4.exe")
    if p is None:
        raise SystemExit("hoi4.exe not running")
    with p:
        started = time.time()
        info = process_info(p)
        report: dict[str, Any] = {
            "schema_version": 1,
            "lane": "diplomacy_intel_operatives_live",
            "probe": "probe_870ebf_intel",
            "expected_sha256": EXPECTED_SHA256,
            "started_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(started)),
            "process": info,
            "fail_closed": False,
        }
        if info.get("sha256") != EXPECTED_SHA256:
            report["fail_closed"] = True
            report["failure"] = "build_sha_mismatch"
            report["completed_utc"] = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
            out_path.write_text(json.dumps(report, indent=2, sort_keys=True), encoding="utf-8")
            return 2

        roots = read_country_roots(p)
        tags = load_tags(p, roots)
        countries, country_report = validate_country_objects(p, roots, tags)
        rtti = derive_rtti_set(p)
        agencies, agency_report = read_agencies(p, countries, tags, rtti)
        crypto_report = read_cryptology(p, agencies, tags, rtti)

        report["roots"] = roots
        report["tags"] = {
            "tag_count_read": len(tags),
            "samples": [{"tag_id": tag_id, "tag": tag} for tag_id, tag in list(sorted(tags.items()))[:16]],
        }
        report["countries"] = country_report
        report["rtti"] = compact_rtti_for_report(rtti)
        report["agencies"] = agency_report
        report["cryptology"] = crypto_report
        if ns.skip_operatives:
            report["operatives"] = {"status": "skipped", "reason": "--skip-operatives"}
        else:
            report["operatives"] = scan_operatives(p, tags, agencies, ns.operative_timeout)

        report["summary"] = build_summary(report)
        report["completed_utc"] = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
        report["elapsed_seconds"] = round(time.time() - started, 3)
        out_path.write_text(json.dumps(report, indent=2, sort_keys=True), encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
