"""Bounded read-only probe for current-build land combat detail roots.

This validates the same active CLandCombat manager chain used by domain 21 on
build 870ebf and emits a narrow field census around the combat body, side
objects, registry, and front object. It is discovery/validation only; it is not
the production hot extraction path.
"""

from __future__ import annotations

import argparse
import ctypes as C
import hashlib
import json
import struct
from ctypes import wintypes as W
from pathlib import Path

EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
PROC_NAME = "hoi4.exe"

C_GAME_STATE_DAILY_UPDATE_OFFSET = 0x1D4390
CG_STATE_DATE_OFFSET = 0x3316E90
GET_LONG_STRING_OFFSET = 0x526A40
DAILY_UPDATE_SENTINEL = bytes.fromhex("48 89 5C 24 18 56 57 41 56 48 83 EC 20 48 8B F1")
GET_LONG_STRING_SENTINEL = bytes.fromhex("48 89 5C 24 08 48 89 74 24 10 57 48 83 EC 20 48")

LAND_COMBAT_MANAGER_OFFSET = 0x260
LAND_COMBAT_MANAGER_DATA = 0x08
LAND_COMBAT_MANAGER_COUNT = 0x14
LAND_COMBAT_MANAGER_BODY_DELTA = 0x10
LAND_COMBAT_VTABLE_RVA = 0x29915B8
LAND_COMBAT_SIDE_VTABLE_RVA = 0x29913C0
LAND_COMBAT_REGISTRY_VTABLE_RVA = 0x29914D8
CPROVINCE_VTABLE_RVA = 0x28426A8
DIVISION_VTABLE_RVA = 0x2944898
LAND_COMBAT_TYPE = 62
DIVISION_TYPES = {4713, 51}

OFF_TYPE = 0x08
OFF_ID = 0x0C
OFF_ATTACKER_SIDE = 0x18
OFF_FRONT = 0x28
OFF_DURATION = 0x30
OFF_DAY = 0x34
OFF_FRONT_PROVINCE = 0xA4
OFF_SIDE_REGISTRY = 0x18
OFF_SIDE_DATA = 0x20
OFF_SIDE_CAPACITY = 0x28
OFF_SIDE_SIZE = 0x2C
OFF_REG_HANDLE = 0x18
OFF_REG_ATTACKER_SIDE = 0x28
OFF_REG_DEFENDER_SIDE = 0x30
OFF_REG_FRONT = 0x38
OFF_DIV_TYPE = 0x08
OFF_DIV_ID = 0x0C

HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000
MAX_LAND_COMBATS = 4000
MAX_SIDE_UNITS = 128
MAX_PROVINCE = 1_000_000

PROCESS_VM_READ = 0x10
PROCESS_QUERY_INFORMATION = 0x400

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


class Reader:
    def __init__(self, pid: int | None):
        self.pid = pid or self._find_pid()
        if not self.pid:
            raise RuntimeError("hoi4.exe not found")
        self.h = _k32.OpenProcess(PROCESS_VM_READ | PROCESS_QUERY_INFORMATION, False, self.pid)
        if not self.h:
            raise RuntimeError(f"OpenProcess failed err={C.get_last_error()}")
        self.base, self.exe_path = self._module()

    def close(self) -> None:
        if self.h:
            _k32.CloseHandle(self.h)
            self.h = None

    @staticmethod
    def _find_pid() -> int | None:
        snap = _k32.CreateToolhelp32Snapshot(0x2, 0)
        pe = _PE32()
        pe.dwSize = C.sizeof(pe)
        pid = None
        if _k32.Process32FirstW(snap, C.byref(pe)):
            while True:
                if pe.szExeFile.lower() == PROC_NAME:
                    pid = pe.th32ProcessID
                    break
                if not _k32.Process32NextW(snap, C.byref(pe)):
                    break
        _k32.CloseHandle(snap)
        return pid

    def _module(self) -> tuple[int, str]:
        snap = _k32.CreateToolhelp32Snapshot(0x8 | 0x10, self.pid)
        me = _ME32()
        me.dwSize = C.sizeof(me)
        if _k32.Module32FirstW(snap, C.byref(me)):
            while True:
                if me.szModule.lower() == PROC_NAME:
                    base = C.cast(me.modBaseAddr, C.c_void_p).value
                    path = me.szExePath
                    _k32.CloseHandle(snap)
                    return base, path
                if not _k32.Module32NextW(snap, C.byref(me)):
                    break
        _k32.CloseHandle(snap)
        raise RuntimeError("hoi4.exe module not found")

    def rpm(self, addr: int, size: int) -> bytes | None:
        buf = (C.c_char * size)()
        got = C.c_size_t(0)
        if not _k32.ReadProcessMemory(self.h, C.c_void_p(addr), buf, size, C.byref(got)):
            return None
        return bytes(buf[: got.value])

    def u32(self, addr: int) -> int | None:
        data = self.rpm(addr, 4)
        return None if not data or len(data) < 4 else struct.unpack("<I", data)[0]

    def u64(self, addr: int) -> int | None:
        data = self.rpm(addr, 8)
        return None if not data or len(data) < 8 else struct.unpack("<Q", data)[0]

    def hash_ok(self) -> tuple[bool, str]:
        digest = hashlib.sha256(Path(self.exe_path).read_bytes()).hexdigest()
        return digest == EXPECTED_SHA256, digest

    def sentinel_ok(self) -> bool:
        daily = self.rpm(self.base + C_GAME_STATE_DAILY_UPDATE_OFFSET, len(DAILY_UPDATE_SENTINEL))
        long = self.rpm(self.base + GET_LONG_STRING_OFFSET, len(GET_LONG_STRING_SENTINEL))
        return daily == DAILY_UPDATE_SENTINEL and long == GET_LONG_STRING_SENTINEL

    def sentinel_report(self) -> dict:
        daily = self.rpm(self.base + C_GAME_STATE_DAILY_UPDATE_OFFSET, len(DAILY_UPDATE_SENTINEL))
        long = self.rpm(self.base + GET_LONG_STRING_OFFSET, len(GET_LONG_STRING_SENTINEL))
        return {
            "daily_addr": hex(self.base + C_GAME_STATE_DAILY_UPDATE_OFFSET),
            "daily_expected": DAILY_UPDATE_SENTINEL.hex(),
            "daily_observed": None if daily is None else daily.hex(),
            "daily_match": daily == DAILY_UPDATE_SENTINEL,
            "long_string_addr": hex(self.base + GET_LONG_STRING_OFFSET),
            "long_string_expected": GET_LONG_STRING_SENTINEL.hex(),
            "long_string_observed": None if long is None else long.hex(),
            "long_string_match": long == GET_LONG_STRING_SENTINEL,
        }

    def enumerate_land_combats(self) -> list[dict]:
        gamestate = self.u64(self.base + CG_STATE_DATE_OFFSET)
        if not gamestate or not is_heap(gamestate):
            raise RuntimeError("CGameState root invalid")
        manager = gamestate + LAND_COMBAT_MANAGER_OFFSET
        count = self.u32(manager + LAND_COMBAT_MANAGER_COUNT)
        data = self.u64(manager + LAND_COMBAT_MANAGER_DATA)
        if count is None or count > MAX_LAND_COMBATS:
            raise RuntimeError(f"manager count invalid: {count}")
        if count == 0:
            return []
        if not data or not is_heap(data):
            raise RuntimeError("manager data invalid")
        rows = []
        for idx in range(count):
            entry = self.u64(data + idx * 8)
            body = entry + LAND_COMBAT_MANAGER_BODY_DELTA if entry else 0
            row = self.read_land_combat(body)
            if row:
                rows.append(row)
        count2 = self.u32(manager + LAND_COMBAT_MANAGER_COUNT)
        data2 = self.u64(manager + LAND_COMBAT_MANAGER_DATA)
        if count2 != count or data2 != data:
            raise RuntimeError("manager changed during read")
        return rows

    def read_land_combat(self, body: int) -> dict | None:
        if not is_heap(body):
            return None
        if self.u64(body) != self.base + LAND_COMBAT_VTABLE_RVA:
            return None
        if self.u32(body + OFF_TYPE) != LAND_COMBAT_TYPE:
            return None
        combat_id = self.u32(body + OFF_ID)
        if not combat_id:
            return None
        front = self.u64(body + OFF_FRONT)
        if not front or self.u64(front) != self.base + CPROVINCE_VTABLE_RVA:
            return None
        location = self.u32(front + OFF_FRONT_PROVINCE)
        if not location or location > MAX_PROVINCE:
            return None
        attacker = self.u64(body + OFF_ATTACKER_SIDE)
        registry = self.u64(attacker + OFF_SIDE_REGISTRY) if attacker else None
        if not attacker or self.u64(attacker) != self.base + LAND_COMBAT_SIDE_VTABLE_RVA:
            return None
        if not registry or self.u64(registry) != self.base + LAND_COMBAT_REGISTRY_VTABLE_RVA:
            return None
        handle = self.u64(registry + OFF_REG_HANDLE)
        if not handle or (handle & 0xFFFFFFFF) != LAND_COMBAT_TYPE or (handle >> 32) != combat_id:
            return None
        defender = self.u64(registry + OFF_REG_DEFENDER_SIDE)
        if not defender or self.u64(defender) != self.base + LAND_COMBAT_SIDE_VTABLE_RVA:
            return None
        return {
            "combat_id": combat_id,
            "body": hex(body),
            "location": location,
            "day": self.u32(body + OFF_DAY),
            "duration": self.u32(body + OFF_DURATION),
            "attacker_count": self.u32(attacker + OFF_SIDE_SIZE),
            "defender_count": self.u32(defender + OFF_SIDE_SIZE),
            "attacker_units": self.read_side_units(attacker),
            "defender_units": self.read_side_units(defender),
            "body_field_census": self.field_census(body, 0x10, 0x140),
            "attacker_side_field_census": self.field_census(attacker, 0x30, 0x120),
            "defender_side_field_census": self.field_census(defender, 0x30, 0x120),
            "registry_field_census": self.field_census(registry, 0x40, 0x100),
            "front_field_census": self.field_census(front, 0x00, 0x220),
        }

    def read_side_units(self, side: int) -> list[int] | None:
        data = self.u64(side + OFF_SIDE_DATA)
        cap = self.u32(side + OFF_SIDE_CAPACITY)
        size = self.u32(side + OFF_SIDE_SIZE)
        if data is None or cap is None or size is None:
            return None
        if size > cap or size > MAX_SIDE_UNITS or (size and not is_heap(data)):
            return None
        units = []
        for idx in range(size):
            ptr = self.u64(data + idx * 8)
            unit = self.resolve_unit(ptr)
            if not unit:
                return None
            units.append(unit)
        return sorted(set(units))

    def resolve_unit(self, ptr: int | None) -> int | None:
        if not ptr or not is_heap(ptr):
            return None
        for body in (ptr, ptr + 0x10):
            if self.u64(body) == self.base + DIVISION_VTABLE_RVA:
                typ = self.u32(body + OFF_DIV_TYPE)
                div_id = self.u32(body + OFF_DIV_ID)
                if typ in DIVISION_TYPES and div_id and div_id < 0x80000000:
                    return div_id
        return None

    def field_census(self, base: int, start: int, end: int) -> list[dict]:
        rows = []
        for off in range(start, end, 4):
            u = self.u32(base + off)
            if u is None:
                continue
            if u == 0:
                continue
            if u <= MAX_PROVINCE or u <= MAX_LAND_COMBATS or u <= MAX_SIDE_UNITS:
                rows.append({"offset": hex(off), "u32": u})
        return rows[:64]


def is_heap(value: int) -> bool:
    return HEAP_LO <= value < HEAP_HI


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--pid", type=int, default=None)
    ap.add_argument("--out-dir", required=True)
    args = ap.parse_args()

    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    r = Reader(args.pid)
    try:
        ok, sha = r.hash_ok()
        report = {
            "probe": "land_combat_detail_events_live_870ebf_roots",
            "pid": r.pid,
            "module_base": hex(r.base),
            "exe_sha256": sha,
            "build_gate_ok": ok,
            "sentinel_gate_ok": r.sentinel_ok(),
            "sentinel_report": r.sentinel_report(),
            "roots_validated": [],
            "active_land_combats": [],
            "candidate_status": {},
        }
        if not ok:
            report["candidate_status"]["all"] = "blocked_build_hash_mismatch"
        elif not report["sentinel_gate_ok"]:
            report["candidate_status"]["all"] = "blocked_collector_sentinel_mismatch"
        else:
            rows = r.enumerate_land_combats()
            report["active_land_combats"] = rows
            report["roots_validated"] = [
                "CGameState = u64[module+0x3316E90]",
                "active land combat manager = CGameState+0x260",
                "manager data/count = +0x08/+0x14",
                "CLandCombat body = manager_entry+0x10, vtable module+0x29915B8, type 62",
                "attacker side = body+0x18, CCombatSide vtable module+0x29913C0",
                "registry = attacker_side+0x18, vtable module+0x29914D8, identity handle +0x18",
                "defender side = registry+0x30",
                "front/location = body+0x28 -> CProvince vtable module+0x28426A8, province +0xA4",
            ]
            report["candidate_status"]["phase_lifecycle_and_participant_delta"] = "validated_from_domain21_roots"
            report["candidate_status"]["casualty_equipment_tactics_reserves_width_terrain_modifiers"] = (
                "not_validated_by_this_bounded_census_no_semantic_oracle"
            )
        (out_dir / "land_combat_detail_roots_probe.json").write_text(
            json.dumps(report, indent=2), encoding="utf-8"
        )
        print(json.dumps(report, indent=2))
    finally:
        r.close()


if __name__ == "__main__":
    main()
