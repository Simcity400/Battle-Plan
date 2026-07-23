#!/usr/bin/env python3
"""Unattended exact-a729 A31 stable/transition acquisition and admission.

The production wrapper clean-launches the frozen save through the centrally
attested recorder.  This driver then discovers that sole process, proves the
paused D04 clock, resolves the SWE object through the existing exact country
database/CGameState authority, executes the frozen focus UI profile, and
collects the two selected-Present receipts.  It does not enable compact d89.
"""

from __future__ import annotations

import argparse
import ctypes
import hashlib
import json
import os
import subprocess
import sys
import time
from ctypes import wintypes
from pathlib import Path

EXE_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
EXPORT = b"battle-plan_a31_fixture_command_json"
PROCESS_ACCESS = 0x0002 | 0x0400 | 0x0008 | 0x0020 | 0x0010
MEM_COMMIT = 0x1000
MEM_RESERVE = 0x2000
MEM_RELEASE = 0x8000
PAGE_READWRITE = 0x04
TH32CS_SNAPMODULE = 0x00000008
TH32CS_SNAPMODULE32 = 0x00000010
TH32CS_SNAPPROCESS = 0x00000002
WAIT_OBJECT_0 = 0
INFINITE = 0xFFFFFFFF
DONT_RESOLVE_DLL_REFERENCES = 0x00000001
INVALID_HANDLE_VALUE = ctypes.c_void_p(-1).value
GAME_STATE_SLOT_RVA = 0x3316E90
COUNTRY_DATABASE_SLOT_RVA = 0x33189C8
COUNTRY_VTABLE_RVA = 0x27D0E90
COUNTRY_TABLE_OFFSET = 0x310
COUNTRY_COUNT_OFFSET = 0x31C
GAME_HOUR_OFFSET = 0x468
COUNTRY_TAG_DB_ID_OFFSET = 0x08
CLOCK_SINGLETON_A_RVA = 0x33172E0
CLOCK_SINGLETON_B_RVA = 0x33172E8
CLOCK_VTABLE_RVA = 0x2952510
CLOCK_PAUSE_OFFSET = 0x6C1
DATE_EPOCH_RAW = 43_799_999
HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000
EXPECTED_SAVE_SHA256 = "c8488b05a3b87c718eeff979976537b79e867634d0b4dccb1eef6c5a45362735"


class ModuleEntry32W(ctypes.Structure):
    _fields_ = [
        ("dwSize", wintypes.DWORD),
        ("th32ModuleID", wintypes.DWORD),
        ("th32ProcessID", wintypes.DWORD),
        ("GlblcntUsage", wintypes.DWORD),
        ("ProccntUsage", wintypes.DWORD),
        ("modBaseAddr", ctypes.POINTER(ctypes.c_byte)),
        ("modBaseSize", wintypes.DWORD),
        ("hModule", wintypes.HMODULE),
        ("szModule", wintypes.WCHAR * 256),
        ("szExePath", wintypes.WCHAR * 260),
    ]


class ProcessEntry32W(ctypes.Structure):
    _fields_ = [
        ("dwSize", wintypes.DWORD), ("cntUsage", wintypes.DWORD),
        ("th32ProcessID", wintypes.DWORD), ("th32DefaultHeapID", ctypes.c_size_t),
        ("th32ModuleID", wintypes.DWORD), ("cntThreads", wintypes.DWORD),
        ("th32ParentProcessID", wintypes.DWORD), ("pcPriClassBase", wintypes.LONG),
        ("dwFlags", wintypes.DWORD), ("szExeFile", wintypes.WCHAR * 260),
    ]


class CaptureError(RuntimeError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise CaptureError(message)


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


class RemoteCollector:
    def __init__(self, process_id: int, collector_dll: Path) -> None:
        require(sys.platform == "win32", "A31 live capture requires Windows")
        self.k32 = ctypes.WinDLL("kernel32", use_last_error=True)
        self._declare()
        self.process_id = process_id
        self.collector_dll = collector_dll.resolve()
        require(
            self.collector_dll.is_file(),
            f"collector DLL does not exist: {self.collector_dll}",
        )
        self.process = self.k32.OpenProcess(PROCESS_ACCESS, False, process_id)
        require(bool(self.process), f"OpenProcess failed: {ctypes.get_last_error()}")
        self._authenticate_process_image()
        self.remote_module = self._remote_module_base()
        self.command_rva = self._export_rva()

    def _declare(self) -> None:
        k32 = self.k32
        k32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
        k32.OpenProcess.restype = wintypes.HANDLE
        k32.CloseHandle.argtypes = [wintypes.HANDLE]
        k32.CloseHandle.restype = wintypes.BOOL
        k32.QueryFullProcessImageNameW.argtypes = [
            wintypes.HANDLE, wintypes.DWORD, wintypes.LPWSTR, ctypes.POINTER(wintypes.DWORD)
        ]
        k32.QueryFullProcessImageNameW.restype = wintypes.BOOL
        k32.CreateToolhelp32Snapshot.argtypes = [wintypes.DWORD, wintypes.DWORD]
        k32.CreateToolhelp32Snapshot.restype = wintypes.HANDLE
        k32.Module32FirstW.argtypes = [wintypes.HANDLE, ctypes.POINTER(ModuleEntry32W)]
        k32.Module32FirstW.restype = wintypes.BOOL
        k32.Module32NextW.argtypes = [wintypes.HANDLE, ctypes.POINTER(ModuleEntry32W)]
        k32.Module32NextW.restype = wintypes.BOOL
        k32.LoadLibraryExW.argtypes = [wintypes.LPCWSTR, wintypes.HANDLE, wintypes.DWORD]
        k32.LoadLibraryExW.restype = wintypes.HMODULE
        k32.GetProcAddress.argtypes = [wintypes.HMODULE, ctypes.c_char_p]
        k32.GetProcAddress.restype = ctypes.c_void_p
        k32.FreeLibrary.argtypes = [wintypes.HMODULE]
        k32.FreeLibrary.restype = wintypes.BOOL
        k32.VirtualAllocEx.argtypes = [
            wintypes.HANDLE, ctypes.c_void_p, ctypes.c_size_t, wintypes.DWORD, wintypes.DWORD
        ]
        k32.VirtualAllocEx.restype = ctypes.c_void_p
        k32.VirtualFreeEx.argtypes = [
            wintypes.HANDLE, ctypes.c_void_p, ctypes.c_size_t, wintypes.DWORD
        ]
        k32.VirtualFreeEx.restype = wintypes.BOOL
        k32.WriteProcessMemory.argtypes = [
            wintypes.HANDLE, ctypes.c_void_p, ctypes.c_void_p, ctypes.c_size_t,
            ctypes.POINTER(ctypes.c_size_t),
        ]
        k32.WriteProcessMemory.restype = wintypes.BOOL
        k32.CreateRemoteThread.argtypes = [
            wintypes.HANDLE, ctypes.c_void_p, ctypes.c_size_t, ctypes.c_void_p,
            ctypes.c_void_p, wintypes.DWORD, ctypes.POINTER(wintypes.DWORD),
        ]
        k32.CreateRemoteThread.restype = wintypes.HANDLE
        k32.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
        k32.WaitForSingleObject.restype = wintypes.DWORD
        k32.GetExitCodeThread.argtypes = [wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD)]
        k32.GetExitCodeThread.restype = wintypes.BOOL
        k32.ReadProcessMemory.argtypes = [
            wintypes.HANDLE, ctypes.c_void_p, ctypes.c_void_p, ctypes.c_size_t,
            ctypes.POINTER(ctypes.c_size_t),
        ]
        k32.ReadProcessMemory.restype = wintypes.BOOL
        k32.WriteProcessMemory.argtypes = [
            wintypes.HANDLE, ctypes.c_void_p, ctypes.c_void_p, ctypes.c_size_t,
            ctypes.POINTER(ctypes.c_size_t),
        ]
        k32.WriteProcessMemory.restype = wintypes.BOOL

    def _authenticate_process_image(self) -> None:
        buffer = ctypes.create_unicode_buffer(32768)
        size = wintypes.DWORD(len(buffer))
        require(
            bool(self.k32.QueryFullProcessImageNameW(self.process, 0, buffer, ctypes.byref(size))),
            f"QueryFullProcessImageNameW failed: {ctypes.get_last_error()}",
        )
        image = Path(buffer.value)
        require(image.name.lower() == "hoi4.exe", f"target is not hoi4.exe: {image}")
        require(sha256_file(image) == EXE_SHA256, "target hoi4.exe is not pinned exact a729")

    def _remote_module_base(self) -> int:
        snapshot = self.k32.CreateToolhelp32Snapshot(
            TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, self.process_id
        )
        require(snapshot not in (0, INVALID_HANDLE_VALUE), "module snapshot failed")
        try:
            entry = ModuleEntry32W()
            entry.dwSize = ctypes.sizeof(entry)
            present = bool(self.k32.Module32FirstW(snapshot, ctypes.byref(entry)))
            while present:
                if (entry.szModule.lower() == self.collector_dll.name.lower()
                        and Path(entry.szExePath).resolve() == self.collector_dll):
                    return ctypes.cast(entry.modBaseAddr, ctypes.c_void_p).value
                present = bool(self.k32.Module32NextW(snapshot, ctypes.byref(entry)))
        finally:
            self.k32.CloseHandle(snapshot)
        raise CaptureError(
            "the exact collector DLL is not loaded in the target; start it through the custom harness"
        )

    def module_base(self, name: str) -> int:
        snapshot = self.k32.CreateToolhelp32Snapshot(
            TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, self.process_id
        )
        require(snapshot not in (0, INVALID_HANDLE_VALUE), "module snapshot failed")
        try:
            entry = ModuleEntry32W()
            entry.dwSize = ctypes.sizeof(entry)
            present = bool(self.k32.Module32FirstW(snapshot, ctypes.byref(entry)))
            while present:
                if entry.szModule.lower() == name.lower():
                    value = ctypes.cast(entry.modBaseAddr, ctypes.c_void_p).value
                    require(bool(value), f"{name} module base is null")
                    return int(value)
                present = bool(self.k32.Module32NextW(snapshot, ctypes.byref(entry)))
        finally:
            self.k32.CloseHandle(snapshot)
        raise CaptureError(f"target module is absent: {name}")

    def read(self, address: int, count: int) -> bytes:
        require(count > 0 and count <= 4096, "remote read count is outside the A31 bound")
        buffer = ctypes.create_string_buffer(count)
        read = ctypes.c_size_t()
        require(bool(self.k32.ReadProcessMemory(
            self.process, address, buffer, count, ctypes.byref(read)
        )) and read.value == count, f"ReadProcessMemory failed at 0x{address:X}")
        return buffer.raw

    def stable_read(self, address: int, count: int) -> bytes:
        first = self.read(address, count)
        second = self.read(address, count)
        require(first == second, f"remote value changed at 0x{address:X}")
        return first

    def write(self, address: int, payload: bytes) -> None:
        require(0 < len(payload) <= 8, "remote control write is outside the A31 bound")
        source = ctypes.create_string_buffer(payload)
        written = ctypes.c_size_t()
        require(bool(self.k32.WriteProcessMemory(
            self.process, address, source, len(payload), ctypes.byref(written)
        )) and written.value == len(payload), f"WriteProcessMemory failed at 0x{address:X}")

    def u32(self, address: int) -> int:
        return int.from_bytes(self.stable_read(address, 4), "little")

    def i32(self, address: int) -> int:
        return int.from_bytes(self.stable_read(address, 4), "little", signed=True)

    def u64(self, address: int) -> int:
        return int.from_bytes(self.stable_read(address, 8), "little")

    def _export_rva(self) -> int:
        local = self.k32.LoadLibraryExW(
            str(self.collector_dll), None, DONT_RESOLVE_DLL_REFERENCES
        )
        require(bool(local), f"LoadLibraryExW collector failed: {ctypes.get_last_error()}")
        try:
            address = self.k32.GetProcAddress(local, EXPORT)
            require(bool(address), "collector does not export the A31 JSON command ABI")
            local_base = (
                local if isinstance(local, int) else ctypes.cast(local, ctypes.c_void_p).value
            )
            export_address = (
                address if isinstance(address, int) else ctypes.cast(address, ctypes.c_void_p).value
            )
            require(bool(local_base) and bool(export_address), "collector export identity is null")
            require(export_address >= local_base, "collector export lies below its image base")
            return export_address - local_base
        finally:
            self.k32.FreeLibrary(local)

    def command(self, body: dict[str, object]) -> None:
        encoded = json.dumps(body, separators=(",", ":")).encode("utf-8") + b"\0"
        remote = self.k32.VirtualAllocEx(
            self.process, None, len(encoded), MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE
        )
        require(bool(remote), f"VirtualAllocEx failed: {ctypes.get_last_error()}")
        try:
            written = ctypes.c_size_t()
            source = ctypes.create_string_buffer(encoded)
            require(
                bool(self.k32.WriteProcessMemory(
                    self.process, remote, source, len(encoded), ctypes.byref(written)
                )) and written.value == len(encoded),
                f"WriteProcessMemory failed: {ctypes.get_last_error()}",
            )
            thread_id = wintypes.DWORD()
            thread = self.k32.CreateRemoteThread(
                self.process,
                None,
                0,
                self.remote_module + self.command_rva,
                remote,
                0,
                ctypes.byref(thread_id),
            )
            require(bool(thread), f"CreateRemoteThread failed: {ctypes.get_last_error()}")
            try:
                require(
                    self.k32.WaitForSingleObject(thread, INFINITE) == WAIT_OBJECT_0,
                    "A31 remote command wait failed",
                )
                exit_code = wintypes.DWORD()
                require(
                    bool(self.k32.GetExitCodeThread(thread, ctypes.byref(exit_code))),
                    "GetExitCodeThread failed",
                )
                require(exit_code.value == 0, f"A31 remote command rejected with code {exit_code.value}")
            finally:
                self.k32.CloseHandle(thread)
        finally:
            self.k32.VirtualFreeEx(self.process, remote, 0, MEM_RELEASE)

    def close(self) -> None:
        if self.process:
            self.k32.CloseHandle(self.process)
            self.process = None


def run_regrader(script: Path, *receipts: Path) -> dict[str, object]:
    completed = subprocess.run(
        [sys.executable, str(script), *(str(path) for path in receipts)],
        check=False,
        capture_output=True,
        text=True,
    )
    require(completed.returncode == 0, completed.stdout.strip() or completed.stderr.strip())
    return json.loads(completed.stdout)


def discover_sole_hoi4_pid() -> int:
    require(sys.platform == "win32", "A31 process discovery requires Windows")
    k32 = ctypes.WinDLL("kernel32", use_last_error=True)
    k32.CreateToolhelp32Snapshot.argtypes = [wintypes.DWORD, wintypes.DWORD]
    k32.CreateToolhelp32Snapshot.restype = wintypes.HANDLE
    k32.Process32FirstW.argtypes = [wintypes.HANDLE, ctypes.POINTER(ProcessEntry32W)]
    k32.Process32FirstW.restype = wintypes.BOOL
    k32.Process32NextW.argtypes = [wintypes.HANDLE, ctypes.POINTER(ProcessEntry32W)]
    k32.Process32NextW.restype = wintypes.BOOL
    k32.CloseHandle.argtypes = [wintypes.HANDLE]
    snapshot = k32.CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)
    require(snapshot not in (0, INVALID_HANDLE_VALUE), "process snapshot failed")
    matches: list[int] = []
    try:
        entry = ProcessEntry32W()
        entry.dwSize = ctypes.sizeof(entry)
        present = bool(k32.Process32FirstW(snapshot, ctypes.byref(entry)))
        while present:
            if entry.szExeFile.lower() == "hoi4.exe":
                matches.append(int(entry.th32ProcessID))
            present = bool(k32.Process32NextW(snapshot, ctypes.byref(entry)))
    finally:
        k32.CloseHandle(snapshot)
    require(len(matches) == 1, f"expected one clean-launch hoi4.exe, found {matches}")
    return matches[0]


def is_heap_pointer(value: int) -> bool:
    return HEAP_LO <= value < HEAP_HI and value % 8 == 0


def discover_country(remote: RemoteCollector, tag: str) -> dict[str, int]:
    base = remote.module_base("hoi4.exe")
    database = remote.u64(base + COUNTRY_DATABASE_SLOT_RVA)
    game_state = remote.u64(base + GAME_STATE_SLOT_RVA)
    require(is_heap_pointer(database) and is_heap_pointer(game_state),
            "country database or CGameState singleton is not an exact heap pointer")
    tag_table = remote.u64(database)
    tag_count = remote.u32(database + 8)
    active_count = remote.u32(database + 12)
    require(is_heap_pointer(tag_table) and 40 <= tag_count <= 1024
            and 40 <= active_count <= tag_count, "country tag database header is invalid")
    prefixes = ("---", "GER", "ENG", "SOV")
    found_id: int | None = None
    for ordinal in range(active_count):
        row = remote.stable_read(tag_table + ordinal * 32, 32)
        length = int.from_bytes(row[16:20], "little")
        body = row[:length] if 0 < length <= 7 else b""
        decoded = (body.decode("ascii") if body
                   and all(byte in b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-" for byte in body)
                   else "")
        if ordinal < len(prefixes):
            require(decoded == prefixes[ordinal], "country tag database prefix mismatch")
        if decoded == tag:
            require(found_id is None, f"duplicate country tag in database: {tag}")
            found_id = ordinal
    require(found_id is not None and found_id > 0, f"country tag is absent: {tag}")
    table = remote.u64(game_state + COUNTRY_TABLE_OFFSET)
    country_count = remote.u32(game_state + COUNTRY_COUNT_OFFSET)
    require(is_heap_pointer(table) and active_count <= country_count <= 4096
            and found_id < country_count, "CGameState country table is invalid")
    object_address = remote.u64(table + found_id * 8)
    require(is_heap_pointer(object_address), f"{tag} country object is absent")
    require(remote.u64(object_address) == base + COUNTRY_VTABLE_RVA,
            f"{tag} CCountry vtable mismatch")
    require(remote.u32(object_address + COUNTRY_TAG_DB_ID_OFFSET) == found_id,
            f"{tag} CCountry tag echo mismatch")
    require(remote.u64(base + GAME_STATE_SLOT_RVA) == game_state
            and remote.u64(base + COUNTRY_DATABASE_SLOT_RVA) == database
            and remote.u64(game_state + COUNTRY_TABLE_OFFSET) == table
            and remote.u32(game_state + COUNTRY_COUNT_OFFSET) == country_count,
            "country roots changed during discovery")
    return {
        "module_base": base, "game_state": game_state, "country_database": database,
        "country_table": table, "country_count": country_count,
        "tag_db_id": found_id, "object_address": object_address,
    }


class AuthenticatedInput:
    def __init__(self, remote: RemoteCollector, expected_width: int, expected_height: int) -> None:
        self.remote = remote
        self.user32 = ctypes.WinDLL("user32", use_last_error=True)
        self.expected_width = expected_width
        self.expected_height = expected_height
        self._declare()
        self.window = self._window_for_pid()
        rect = wintypes.RECT()
        require(bool(self.user32.GetClientRect(self.window, ctypes.byref(rect))), "GetClientRect failed")
        require(rect.right - rect.left == expected_width and rect.bottom - rect.top == expected_height,
                f"A31 profile requires {expected_width}x{expected_height} client")
        self.activate()

    def _declare(self) -> None:
        callback = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
        self._callback_type = callback
        self.user32.EnumWindows.argtypes = [callback, wintypes.LPARAM]
        self.user32.GetWindowThreadProcessId.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.DWORD)]
        self.user32.IsWindowVisible.argtypes = [wintypes.HWND]
        self.user32.GetClientRect.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.RECT)]
        self.user32.ClientToScreen.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.POINT)]
        self.user32.SetCursorPos.argtypes = [ctypes.c_int, ctypes.c_int]
        self.user32.SetForegroundWindow.argtypes = [wintypes.HWND]
        self.user32.GetForegroundWindow.restype = wintypes.HWND
        self.user32.VkKeyScanW.argtypes = [wintypes.WCHAR]
        self.user32.VkKeyScanW.restype = ctypes.c_short
        self.user32.keybd_event.argtypes = [wintypes.BYTE, wintypes.BYTE, wintypes.DWORD, ctypes.c_size_t]
        self.user32.mouse_event.argtypes = [wintypes.DWORD, wintypes.DWORD, wintypes.DWORD, wintypes.DWORD, ctypes.c_size_t]

    def _window_for_pid(self) -> int:
        matches: list[int] = []
        @self._callback_type
        def visit(hwnd: int, _param: int) -> bool:
            pid = wintypes.DWORD()
            self.user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
            if pid.value == self.remote.process_id and self.user32.IsWindowVisible(hwnd):
                matches.append(int(hwnd))
            return True
        self.user32.EnumWindows(visit, 0)
        require(len(matches) == 1, f"expected one visible exact-a729 window, found {len(matches)}")
        return matches[0]

    def activate(self) -> None:
        if int(self.user32.GetForegroundWindow()) != self.window:
            require(bool(self.user32.SetForegroundWindow(self.window)), "SetForegroundWindow failed")
            time.sleep(0.75)
        require(int(self.user32.GetForegroundWindow()) == self.window, "exact-a729 window is not foreground")

    def key(self, vk: int, scan: int = 0) -> None:
        self.activate()
        flags = 0x0008 if scan else 0
        self.user32.keybd_event(vk, scan, flags, 0)
        time.sleep(0.05)
        self.user32.keybd_event(vk, scan, flags | 0x0002, 0)

    def text(self, value: str) -> None:
        # Console commands in this frozen profile are ASCII. VkKeyScanW gives
        # their physical US-layout key and modifier tuple.
        for char in value:
            code = self.user32.VkKeyScanW(char)
            require(code != -1, f"cannot synthesize console character {char!r}")
            vk, modifiers = code & 0xFF, (code >> 8) & 0xFF
            if modifiers & 1:
                self.user32.keybd_event(0x10, 0, 0, 0)
            self.key(vk)
            if modifiers & 1:
                self.user32.keybd_event(0x10, 0, 0x0002, 0)

    def click(self, x: int, y: int) -> None:
        require(0 <= x < self.expected_width and 0 <= y < self.expected_height,
                f"A31 client point is outside profile: {x},{y}")
        self.activate()
        point = wintypes.POINT(x, y)
        require(bool(self.user32.ClientToScreen(self.window, ctypes.byref(point))), "ClientToScreen failed")
        require(bool(self.user32.SetCursorPos(point.x, point.y)), "SetCursorPos failed")
        self.user32.mouse_event(0x0002, 0, 0, 0, 0)
        time.sleep(0.08)
        self.user32.mouse_event(0x0004, 0, 0, 0, 0)

    def console(self, command: str, settle_ms: int) -> None:
        self.key(0, 0x29)
        time.sleep(0.25)
        self.text(command)
        self.key(0x0D)
        time.sleep(settle_ms / 1000)
        self.key(0, 0x29)


def clock(remote: RemoteCollector) -> tuple[int, int, int]:
    base = remote.module_base("hoi4.exe")
    state = remote.u64(base + GAME_STATE_SLOT_RVA)
    clocks = {value for value in (
        remote.u64(base + CLOCK_SINGLETON_A_RVA), remote.u64(base + CLOCK_SINGLETON_B_RVA)
    ) if value}
    require(len(clocks) == 1, "clock singleton is ambiguous")
    clock_address = next(iter(clocks))
    require(remote.u64(clock_address) == base + CLOCK_VTABLE_RVA, "clock vtable mismatch")
    require(is_heap_pointer(state), "CGameState singleton is invalid")
    return state, clock_address, remote.i32(state + GAME_HOUR_OFFSET)


def require_paused_hour(remote: RemoteCollector, expected: int) -> None:
    state, clock_address, observed = clock(remote)
    require(observed == expected, f"expected raw game hour {expected}, observed {observed}")
    require(remote.stable_read(clock_address + CLOCK_PAUSE_OFFSET, 1) == b"\x01",
            "game is not paused")
    require(remote.u64(remote.module_base("hoi4.exe") + GAME_STATE_SLOT_RVA) == state,
            "CGameState changed during paused-hour proof")


def load_profile(path: Path) -> dict[str, object]:
    profile = json.loads(path.read_text(encoding="utf-8"))
    require(profile.get("schema") == "hoi4.a31.autonomous-focus-actions.v1", "action profile schema mismatch")
    authority = profile.get("authority")
    require(isinstance(authority, dict)
            and authority.get("executable_sha256") == EXE_SHA256
            and authority.get("save_sha256") == EXPECTED_SAVE_SHA256,
            "action profile authority mismatch")
    oracle = path.parent / "A31_D04_SAVE_FOCUS_ORACLE_20260722.json"
    require(oracle.is_file()
            and authority.get("save_oracle_receipt_sha256") == sha256_file(oracle),
            "A31 save-focus oracle receipt mismatch")
    require(profile.get("country_tag") == "SWE", "A31 autonomous country must be SWE")
    require(profile.get("stable_game_hour_raw") == 60830441
            and profile.get("transition_game_hour_raw") == 60830442,
            "A31 autonomous raw-hour pair mismatch")
    require(profile.get("stable_frame_hour") == 17030442
            and profile.get("transition_frame_hour") == 17030443,
            "A31 autonomous normalized-hour pair mismatch")
    require(profile.get("expected_client_size") == [1920, 1080], "A31 client profile mismatch")
    require(profile.get("switch_country_command") == "tag SWE"
            and profile.get("politics_active_focus_point") == [220, 174]
            and profile.get("focus_search_point") == [1617, 100]
            and profile.get("target_focus_id") == "SWE_infrastructure"
            and profile.get("target_focus_search_text") == "Infrastructure Investment",
            "A31 focus action sequence changed")
    for field, minimum, maximum in (
        ("settle_milliseconds", 500, 5000),
        ("panel_settle_milliseconds", 2000, 10000),
        ("search_settle_milliseconds", 500, 5000),
        ("detail_settle_milliseconds", 500, 5000),
        ("transition_settle_milliseconds", 1000, 10000),
        ("transition_timeout_seconds", 30, 300),
    ):
        value = profile.get(field)
        require(isinstance(value, int) and minimum <= value <= maximum,
                f"A31 action timing is invalid: {field}")
    return profile


def write_create_new_json(path: Path, value: dict[str, object]) -> None:
    encoded = (json.dumps(value, indent=2, sort_keys=True) + "\n").encode("utf-8")
    require(not path.exists(), f"refusing to replace create-new A31 artifact: {path}")
    temporary = path.with_name(f".{path.name}.{os.getpid()}.{time.time_ns()}.tmp")
    descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        with os.fdopen(descriptor, "wb") as stream:
            stream.write(encoded)
            stream.flush()
            os.fsync(stream.fileno())
        descriptor = -1
        os.rename(temporary, path)
    finally:
        if descriptor >= 0:
            os.close(descriptor)
        if temporary.exists():
            temporary.unlink()


def capture(args: argparse.Namespace) -> dict[str, object]:
    output_root = args.output_root.resolve()
    output_root.mkdir(parents=True, exist_ok=True)
    profile = load_profile(args.action_profile.resolve())
    country_tag = str(profile["country_tag"])
    stable_frame_hour = int(profile["stable_frame_hour"])
    transition_frame_hour = int(profile["transition_frame_hour"])
    stable_game_hour_raw = int(profile["stable_game_hour_raw"])
    transition_game_hour_raw = int(profile["transition_game_hour_raw"])
    run_id = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    stable_result = output_root / f"{run_id}-stable.result.json"
    transition_result = output_root / f"{run_id}-transition.result.json"
    pair_result = output_root / f"{run_id}-fixture-pair.regrade.json"
    for path in (stable_result, transition_result, pair_result):
        require(not path.exists(), f"refusing to overwrite {path}")

    process_id = discover_sole_hoi4_pid()
    remote = RemoteCollector(process_id, args.collector_dll)
    require_paused_hour(remote, stable_game_hour_raw)
    country = discover_country(remote, country_tag)
    ui = AuthenticatedInput(remote, 1920, 1080)
    start = time.monotonic()
    installed = False
    steps: list[dict[str, object]] = []
    try:
        ui.key(0x1B); time.sleep(0.5); ui.key(0x1B); time.sleep(0.5)
        steps.append({"action": "close_all_panels_before_install", "key": "ESCAPE", "count": 2})
        remote.command({"operation": "install", "output_root": str(output_root)})
        installed = True
        ui.console(str(profile["switch_country_command"]), int(profile["settle_milliseconds"]))
        steps.append({"action": "switch_country", "command": profile["switch_country_command"]})
        remote.command({
            "operation": "arm",
            "frame_hour": str(stable_frame_hour),
            "country_tag": country_tag,
            "country_object_address": str(country["object_address"]),
        })
        ui.key(0x51)
        time.sleep(int(profile["settle_milliseconds"]) / 1000)
        focus_x, focus_y = profile["politics_active_focus_point"]
        ui.click(int(focus_x), int(focus_y))
        time.sleep(int(profile["panel_settle_milliseconds"]) / 1000)
        steps.append({"action": "open_stable_focus_view", "politics_key": "Q",
                      "client_point": [focus_x, focus_y]})
        remote.command({
            "operation": "seal",
            "target_game_hour_raw": stable_game_hour_raw,
            "elapsed_wall_ms": str(int((time.monotonic() - start) * 1000)),
            "result_path": str(stable_result),
        })

        ui.key(0x1B); time.sleep(0.75)
        steps.append({"action": "close_stable_focus_view", "key": "ESCAPE"})
        remote.command({
            "operation": "arm",
            "frame_hour": str(transition_frame_hour),
            "country_tag": country_tag,
            "country_object_address": str(country["object_address"]),
        })
        ui.key(0x51); time.sleep(int(profile["settle_milliseconds"]) / 1000)
        ui.click(int(focus_x), int(focus_y)); time.sleep(int(profile["panel_settle_milliseconds"]) / 1000)
        search_x, search_y = profile["focus_search_point"]
        ui.click(int(search_x), int(search_y)); time.sleep(0.25)
        ui.text(str(profile["target_focus_search_text"]))
        time.sleep(int(profile["search_settle_milliseconds"]) / 1000)
        ui.key(0x0D); time.sleep(int(profile["detail_settle_milliseconds"]) / 1000)
        ui.key(0x0D); time.sleep(int(profile["transition_settle_milliseconds"]) / 1000)
        # One hour gives the state producer a distinct real frame and prevents
        # same-hour metadata from masquerading as the requested transition.
        state, clock_address, before = clock(remote)
        require(before == stable_game_hour_raw, "A31 clock moved before transition pulse")
        ui.key(0x70); time.sleep(0.2)  # F1 prevents the one-hour gate from overshooting.
        remote.write(clock_address + CLOCK_PAUSE_OFFSET, b"\x00")
        deadline = time.monotonic() + int(profile["transition_timeout_seconds"])
        try:
            while remote.i32(state + GAME_HOUR_OFFSET) < transition_game_hour_raw:
                require(time.monotonic() < deadline, "A31 transition hour timeout")
                time.sleep(0.002)
        finally:
            remote.write(clock_address + CLOCK_PAUSE_OFFSET, b"\x01")
        time.sleep(0.25)
        require_paused_hour(remote, transition_game_hour_raw)
        # Re-open if focus selection returned to politics/map and wait for a
        # new exact frame refresh. A redundant Q/click is fail-closed because
        # the hook receipt, not this action log, proves the actual view.
        ui.key(0x51); time.sleep(0.75); ui.click(int(focus_x), int(focus_y))
        time.sleep(int(profile["panel_settle_milliseconds"]) / 1000)
        steps.append({"action": "apply_focus_transition", "search_text": profile["target_focus_search_text"],
                      "search_point": [search_x, search_y], "start_key": "RETURN",
                      "game_hour_before": before, "game_hour_after": transition_game_hour_raw})
        remote.command({
            "operation": "seal",
            "target_game_hour_raw": transition_game_hour_raw,
            "elapsed_wall_ms": str(int((time.monotonic() - start) * 1000)),
            "result_path": str(transition_result),
        })
        ui.key(0x1B); time.sleep(0.75)
        remote.command({"operation": "uninstall"})
        installed = False
    finally:
        if installed:
            try:
                ui.key(0x1B); time.sleep(0.75)
                remote.command({"operation": "uninstall"})
            except Exception as error:
                print(f"warning: A31 teardown retry failed: {error}", file=sys.stderr)
        remote.close()

    action_receipt = output_root / f"{run_id}-autonomous-actions.json"
    write_create_new_json(action_receipt, {
        "schema": "hoi4.a31.autonomous-focus-action-receipt.v1",
        "process_id": process_id,
        "executable_sha256": EXE_SHA256,
        "profile_sha256": sha256_file(args.action_profile.resolve()),
        "country_tag": country_tag,
        "country_tag_db_id": country["tag_db_id"],
        "country_address_authority": "exact CGameState+0x310 tag-indexed table, CCountry vtable and +0x08 tag echo",
        "stable_game_hour_raw": stable_game_hour_raw,
        "transition_game_hour_raw": transition_game_hour_raw,
        "steps": steps,
        "semantic_authority": "paired NFP1 receipts and fail-closed mechanical regraders only",
        "promotion_authorized": False,
    })

    stable_descriptor = json.loads(stable_result.read_text(encoding="utf-8"))
    transition_descriptor = json.loads(transition_result.read_text(encoding="utf-8"))
    stable_receipt = Path(stable_descriptor["receipt_path"]).resolve()
    transition_receipt = Path(transition_descriptor["receipt_path"]).resolve()
    tools = Path(__file__).resolve().parent
    stable_grade = run_regrader(tools / "verify_a31_acquisition_receipt.py", stable_receipt)
    transition_grade = run_regrader(tools / "verify_a31_acquisition_receipt.py", transition_receipt)
    matrix_grade = run_regrader(
        tools / "verify_a31_fixture_matrix.py", stable_receipt, transition_receipt
    )
    result = {
        "schema": "hoi4.a31.owner_runnable_fixture_pair.v1",
        "status": "PASS_STABLE_AND_DISCRIMINATING_TRANSITION",
        "stable_receipt": stable_receipt.name,
        "transition_receipt": transition_receipt.name,
        "stable_grade": stable_grade,
        "transition_grade": transition_grade,
        "matrix_grade": matrix_grade,
        "action_receipt": action_receipt.name,
        "action_receipt_sha256": sha256_file(action_receipt),
        "action_profile_sha256": sha256_file(args.action_profile.resolve()),
        "production_scheduling_enabled": False,
    }
    write_create_new_json(pair_result, result)
    return {
        **result,
        "action_receipt_path": str(action_receipt),
        "pair_result": str(pair_result),
    }


def main() -> int:
    require(
        os.environ.get("BATTLE_PLAN_CLIENT_DRIVING_OPT_IN", os.environ.get("HOI4_CLIENT_DRIVING_OPT_IN"))
        == "I_ACKNOWLEDGE_THIS_TOOL_WILL_CONTROL_HOI4",
        "legacy the game client driving is disabled; explicit process-local opt-in is required",
    )
    parser = argparse.ArgumentParser()
    parser.add_argument("--collector-dll", type=Path, required=True)
    parser.add_argument("--output-root", type=Path, required=True)
    parser.add_argument("--action-profile", type=Path, required=True)
    args = parser.parse_args()
    try:
        result = capture(args)
    except Exception as error:
        print(json.dumps({"status": "FAIL", "error": str(error)}, sort_keys=True))
        return 1
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
