#!/usr/bin/env python3
"""Read-only anchored probe for the CDivision+0x1D0 controller candidate.

This is a lane-local proof helper, not the hot-loop collector. It follows the
collector's current rooted path:

  hoi4.exe module -> CGameState -> country table -> division wrappers -> body

It deliberately does not scan heap/vtables globally.
"""

from __future__ import annotations

import argparse
import ctypes as C
import hashlib
import json
import sys
from ctypes import wintypes as W
from pathlib import Path
from typing import Any


PROC_NAME = "hoi4.exe"

C_GAME_STATE_DAILY_UPDATE_OFFSET = 0x1D4390
CG_STATE_DATE_OFFSET = 0x3316E90
GET_LONG_STRING_OFFSET = 0x526A40
CGAMESTATE_COUNTRY_TABLE = 0x310
CGAMESTATE_COUNTRY_COUNT = 0x31C

COUNTRY_VTABLE_RVA = 0x27D0E90
DIVISION_VTABLE_RVA = 0x2943CB0
DIVISION_WRAPPER_VTABLE_RVA = 0x2943AD0
DIVISION_WRAPPER_TO_BODY = 0x10

COUNTRY_DIVISION_WRAPPER_DATA = 0x290
COUNTRY_DIVISION_WRAPPER_CAPACITY = 0x298
COUNTRY_DIVISION_WRAPPER_COUNT = 0x29C

DIVISION_TYPE = 4713
DIVISION_ALT_UNIT_TYPE = 51
DIVISION_OFF_ID_TYPE = 0x08
DIVISION_OFF_ID_ID = 0x0C
DIVISION_OFF_OWNER_TAG_DB_ID = 0x1C8
DIVISION_OFF_CONTROLLER_CANDIDATE = 0x1D0

MAX_CGAMESTATE_COUNTRIES = 2048
MAX_DIVISION_ROWS = 20000
MAX_DIVISION_OWNER_TAG_DB_ID = MAX_CGAMESTATE_COUNTRIES
HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000

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


_k32 = C.WinDLL("kernel32", use_last_error=True)


class _PE32(C.Structure):
    _fields_ = [
        ("dwSize", W.DWORD),
        ("cntUsage", W.DWORD),
        ("th32ProcessID", W.DWORD),
        ("th32DefaultHeapID", C.POINTER(C.c_ulong)),
        ("th32ModuleID", W.DWORD),
        ("cntThreads", W.DWORD),
        ("th32ParentProcessID", W.DWORD),
        ("pcPriClassBase", C.c_long),
        ("dwFlags", W.DWORD),
        ("szExeFile", W.WCHAR * 260),
    ]


class _ME32(C.Structure):
    _fields_ = [
        ("dwSize", W.DWORD),
        ("th32ModuleID", W.DWORD),
        ("th32ProcessID", W.DWORD),
        ("GlblcntUsage", W.DWORD),
        ("ProccntUsage", W.DWORD),
        ("modBaseAddr", C.POINTER(C.c_byte)),
        ("modBaseSize", W.DWORD),
        ("hModule", W.HMODULE),
        ("szModule", W.WCHAR * 256),
        ("szExePath", W.WCHAR * 260),
    ]


class ProcessReader:
    def __init__(self, pid: int | None, proc_name: str) -> None:
        self.pid = pid or self._find_pid(proc_name)
        if not self.pid:
            raise RuntimeError(f"process not found: {proc_name}")
        self.handle = _k32.OpenProcess(0x10 | 0x400, False, self.pid)
        if not self.handle:
            raise RuntimeError(f"OpenProcess failed: {C.get_last_error()}")
        self.module_base, self.module_size, self.exe_path = self._module(self.pid, proc_name)
        if not self.module_base:
            raise RuntimeError(f"module not found: {proc_name}")

    @staticmethod
    def _find_pid(name: str) -> int | None:
        snap = _k32.CreateToolhelp32Snapshot(0x2, 0)
        pe = _PE32()
        pe.dwSize = C.sizeof(pe)
        pid = None
        if _k32.Process32FirstW(snap, C.byref(pe)):
            while True:
                if pe.szExeFile.lower() == name.lower():
                    pid = int(pe.th32ProcessID)
                    break
                if not _k32.Process32NextW(snap, C.byref(pe)):
                    break
        _k32.CloseHandle(snap)
        return pid

    @staticmethod
    def _module(pid: int, name: str) -> tuple[int | None, int | None, str | None]:
        snap = _k32.CreateToolhelp32Snapshot(0x8 | 0x10, pid)
        me = _ME32()
        me.dwSize = C.sizeof(me)
        base = None
        size = None
        path = None
        if _k32.Module32FirstW(snap, C.byref(me)):
            while True:
                if me.szModule.lower() == name.lower():
                    base = C.cast(me.modBaseAddr, C.c_void_p).value
                    size = int(me.modBaseSize)
                    path = str(me.szExePath)
                    break
                if not _k32.Module32NextW(snap, C.byref(me)):
                    break
        _k32.CloseHandle(snap)
        return base, size, path

    def read(self, addr: int, size: int) -> bytes | None:
        buf = (C.c_ubyte * size)()
        got = C.c_size_t(0)
        ok = _k32.ReadProcessMemory(
            self.handle, C.c_void_p(addr), buf, size, C.byref(got)
        )
        if not ok or got.value != size:
            return None
        return bytes(buf)

    def u32(self, addr: int) -> int | None:
        data = self.read(addr, 4)
        return int.from_bytes(data, "little") if data else None

    def u64(self, addr: int) -> int | None:
        data = self.read(addr, 8)
        return int.from_bytes(data, "little") if data else None


def heap_aligned(ptr: int | None, alignment: int = 8) -> bool:
    return bool(ptr and HEAP_LO <= ptr < HEAP_HI and ptr % alignment == 0)


def exe_sha256(path: str | None) -> str | None:
    if not path:
        return None
    try:
        h = hashlib.sha256()
        with open(path, "rb") as fh:
            for chunk in iter(lambda: fh.read(1024 * 1024), b""):
                h.update(chunk)
        return h.hexdigest()
    except OSError:
        return None


def probe(pid: int | None, proc_name: str, sample_limit: int) -> dict[str, Any]:
    reader = ProcessReader(pid, proc_name)
    base = int(reader.module_base)
    daily_match = (
        reader.read(base + C_GAME_STATE_DAILY_UPDATE_OFFSET, len(DAILY_UPDATE_SENTINEL))
        == DAILY_UPDATE_SENTINEL
    )
    long_string_match = (
        reader.read(base + GET_LONG_STRING_OFFSET, len(GET_LONG_STRING_SENTINEL))
        == GET_LONG_STRING_SENTINEL
    )
    result: dict[str, Any] = {
        "schema": "division_controller_candidate_probe_v1",
        "pid": reader.pid,
        "module_base": f"0x{base:X}",
        "exe_path": reader.exe_path,
        "exe_sha256": exe_sha256(reader.exe_path),
        "root_path": (
            "hoi4.exe+CG_STATE_DATE_OFFSET -> CGameState+0x310 country table -> "
            "CCountry+0x290 wrapper vector -> wrapper+0x10 CDivision"
        ),
        "candidate": {
            "owner_offset": "CDivision+0x1C8",
            "controller_candidate_offset": "CDivision+0x1D0",
        },
        "root_validation": {
            "daily_update_sentinel": daily_match,
            "get_long_string_sentinel": long_string_match,
        },
        "semantic_status": "unread",
        "counts": {},
        "sample_rows": [],
        "errors": [],
    }
    if not daily_match or not long_string_match:
        result["semantic_status"] = "blocked_sentinel_mismatch"
        result["errors"].append("collector sentinels did not match")
        return result

    game_state = reader.u64(base + CG_STATE_DATE_OFFSET)
    country_table = reader.u64((game_state or 0) + CGAMESTATE_COUNTRY_TABLE)
    country_count = reader.u32((game_state or 0) + CGAMESTATE_COUNTRY_COUNT)
    result["root_validation"].update(
        {
            "game_state": f"0x{game_state:X}" if game_state else None,
            "country_table": f"0x{country_table:X}" if country_table else None,
            "country_count": country_count,
        }
    )
    if (
        not heap_aligned(game_state)
        or not heap_aligned(country_table)
        or not country_count
        or country_count > MAX_CGAMESTATE_COUNTRIES
    ):
        result["semantic_status"] = "blocked_invalid_country_table_root"
        result["errors"].append("invalid CGameState country table root")
        return result

    country_vtable = base + COUNTRY_VTABLE_RVA
    wrapper_vtable = base + DIVISION_WRAPPER_VTABLE_RVA
    division_vtable = base + DIVISION_VTABLE_RVA
    country_ptrs = reader.read(country_table, country_count * 8)
    if not country_ptrs:
        result["semantic_status"] = "blocked_country_table_unreadable"
        result["errors"].append("country table unreadable")
        return result

    rows_by_id: dict[int, dict[str, Any]] = {}
    counts = {
        "country_rows": 0,
        "country_rows_with_division_vector": 0,
        "wrapper_rows": 0,
        "validated_division_rows": 0,
        "duplicate_division_ids": 0,
        "invalid_candidate_rows": 0,
        "candidate_equals_owner_rows": 0,
        "owner_controller_divergent_rows": 0,
    }
    for country_index in range(country_count):
        country_ptr = int.from_bytes(country_ptrs[country_index * 8 : country_index * 8 + 8], "little")
        if not heap_aligned(country_ptr):
            continue
        if reader.u64(country_ptr) != country_vtable:
            continue
        counts["country_rows"] += 1
        vec_data = reader.u64(country_ptr + COUNTRY_DIVISION_WRAPPER_DATA)
        capacity = reader.u32(country_ptr + COUNTRY_DIVISION_WRAPPER_CAPACITY)
        count = reader.u32(country_ptr + COUNTRY_DIVISION_WRAPPER_COUNT)
        if not count:
            continue
        if not heap_aligned(vec_data) or capacity is None or count > capacity or count > MAX_DIVISION_ROWS:
            continue
        wrapper_bytes = reader.read(vec_data, count * 8)
        if not wrapper_bytes:
            continue
        counts["country_rows_with_division_vector"] += 1
        for idx in range(count):
            wrapper = int.from_bytes(wrapper_bytes[idx * 8 : idx * 8 + 8], "little")
            if not heap_aligned(wrapper):
                continue
            if reader.u64(wrapper) != wrapper_vtable:
                continue
            counts["wrapper_rows"] += 1
            body = wrapper + DIVISION_WRAPPER_TO_BODY
            if reader.u64(body) != division_vtable:
                continue
            type_tag = reader.u32(body + DIVISION_OFF_ID_TYPE)
            if type_tag not in {DIVISION_TYPE, DIVISION_ALT_UNIT_TYPE}:
                continue
            div_id = reader.u32(body + DIVISION_OFF_ID_ID)
            if not div_id or div_id > 0x7FFF_FFFF:
                continue
            owner = reader.u32(body + DIVISION_OFF_OWNER_TAG_DB_ID)
            controller = reader.u32(body + DIVISION_OFF_CONTROLLER_CANDIDATE)
            if div_id in rows_by_id:
                counts["duplicate_division_ids"] += 1
                continue
            row = {
                "div_id": div_id,
                "body_ptr": f"0x{body:X}",
                "country_table_index": country_index,
                "owner_tag_db_id": owner,
                "controller_candidate_tag_db_id": controller,
                "type_tag": type_tag,
            }
            rows_by_id[div_id] = row
            counts["validated_division_rows"] += 1
            if not controller or controller > MAX_DIVISION_OWNER_TAG_DB_ID:
                counts["invalid_candidate_rows"] += 1
            elif owner == controller:
                counts["candidate_equals_owner_rows"] += 1
            else:
                counts["owner_controller_divergent_rows"] += 1
            if len(result["sample_rows"]) < sample_limit:
                result["sample_rows"].append(row)

    result["counts"] = counts
    if counts["validated_division_rows"] == 0:
        result["semantic_status"] = "blocked_no_valid_division_rows"
        result["errors"].append("no validated CDivision rows from anchored roots")
    elif counts["owner_controller_divergent_rows"] == 0:
        result["semantic_status"] = "blocked_no_owner_controller_divergent_fixture"
    elif counts["invalid_candidate_rows"] == 0:
        result["semantic_status"] = "needs_oracle_gate_for_divergent_rows"
    else:
        result["semantic_status"] = "blocked_invalid_controller_candidate_values"
    return result


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pid", type=int)
    parser.add_argument("--process-name", default=PROC_NAME)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--sample-limit", type=int, default=25)
    args = parser.parse_args(argv)

    result = probe(args.pid, args.process_name, max(0, args.sample_limit))
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(result, sort_keys=True))
    return 0 if not result.get("errors") else 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
