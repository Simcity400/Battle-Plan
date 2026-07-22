#!/usr/bin/env python3
"""Autonomous exact-a729 A24 politics-surface acquisition owner.

The target collector is already injected by the central recorder. This owner
uses the collector's one-argument command ABI and authenticated physical input;
it never supplies a Present receipt, producer observation, PTI1 row, or result.
"""

from __future__ import annotations

import argparse
import ctypes
import hashlib
import json
import sys
import time
from ctypes import wintypes
from pathlib import Path

EXE_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
EXPORT = b"battle-plan_a24_capture_command_json"
GAME_STATE_GLOBAL_RVA = 0x03316E90
GAME_HOUR_OFFSET = 0x468
PRIMARY_COUNTRY_OFFSET = 0x520
FALLBACK_COUNTRY_OFFSET = 0x524
PROCESS_ACCESS = 0x0002 | 0x0400 | 0x0008 | 0x0020 | 0x0010
MEM_COMMIT, MEM_RESERVE, MEM_RELEASE, PAGE_READWRITE = 0x1000, 0x2000, 0x8000, 0x04
TH32CS_SNAPMODULE, TH32CS_SNAPMODULE32 = 0x8, 0x10
WAIT_OBJECT_0, INFINITE, DONT_RESOLVE_DLL_REFERENCES = 0, 0xFFFFFFFF, 1
INVALID_HANDLE_VALUE = ctypes.c_void_p(-1).value
REQUIRED_LINE_KINDS = {
    "evaluated_trigger_result_and_reason_tree",
    "effective_hire_or_removal_cost",
    "aggregated_conditional_modifier",
    "advisor_slot_capacity_vacancy_eligibility_and_disabled_reason",
    "timed_dynamic_modifier_value_source_and_expiry",
    "displayed_party_popularity_change_source",
}


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


def write_new_json(path: Path, value: object) -> None:
    require(not path.exists(), f"refusing to replace immutable receipt: {path}")
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.{time.time_ns()}.tmp")
    encoded = (json.dumps(value, indent=2, sort_keys=True) + "\n").encode("utf-8")
    try:
        with temporary.open("xb") as stream:
            stream.write(encoded)
            stream.flush()
            import os
            os.fsync(stream.fileno())
        temporary.replace(path)
    finally:
        temporary.unlink(missing_ok=True)


class ModuleEntry32W(ctypes.Structure):
    _fields_ = [
        ("dwSize", wintypes.DWORD), ("th32ModuleID", wintypes.DWORD),
        ("th32ProcessID", wintypes.DWORD), ("GlblcntUsage", wintypes.DWORD),
        ("ProccntUsage", wintypes.DWORD), ("modBaseAddr", ctypes.POINTER(ctypes.c_byte)),
        ("modBaseSize", wintypes.DWORD), ("hModule", wintypes.HMODULE),
        ("szModule", wintypes.WCHAR * 256), ("szExePath", wintypes.WCHAR * 260),
    ]


class Rect(ctypes.Structure):
    _fields_ = [("left", ctypes.c_long), ("top", ctypes.c_long),
                ("right", ctypes.c_long), ("bottom", ctypes.c_long)]


class Point(ctypes.Structure):
    _fields_ = [("x", ctypes.c_long), ("y", ctypes.c_long)]


class RemoteOwner:
    def __init__(self, process_id: int, collector: Path) -> None:
        require(sys.platform == "win32", "A24 acquisition requires Windows")
        self.process_id = process_id
        self.collector = collector.resolve()
        self.k32 = ctypes.WinDLL("kernel32", use_last_error=True)
        self.u32 = ctypes.WinDLL("user32", use_last_error=True)
        self._declare()
        self.process = self.k32.OpenProcess(PROCESS_ACCESS, False, process_id)
        require(bool(self.process), f"OpenProcess failed: {ctypes.get_last_error()}")
        self.exe_path = self._process_image()
        require(sha256_file(self.exe_path) == EXE_SHA256, "target is not exact a729")
        self.module_base = self._module_base(self.exe_path)
        self.collector_base = self._module_base(self.collector)
        self.command_address = self.collector_base + self._export_rva()
        self.window = self._window()

    def _declare(self) -> None:
        k, u = self.k32, self.u32
        k.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
        k.OpenProcess.restype = wintypes.HANDLE
        k.CloseHandle.argtypes = [wintypes.HANDLE]
        k.QueryFullProcessImageNameW.argtypes = [wintypes.HANDLE, wintypes.DWORD, wintypes.LPWSTR, ctypes.POINTER(wintypes.DWORD)]
        k.QueryFullProcessImageNameW.restype = wintypes.BOOL
        k.CreateToolhelp32Snapshot.argtypes = [wintypes.DWORD, wintypes.DWORD]
        k.CreateToolhelp32Snapshot.restype = wintypes.HANDLE
        k.Module32FirstW.argtypes = [wintypes.HANDLE, ctypes.POINTER(ModuleEntry32W)]
        k.Module32FirstW.restype = wintypes.BOOL
        k.Module32NextW.argtypes = [wintypes.HANDLE, ctypes.POINTER(ModuleEntry32W)]
        k.Module32NextW.restype = wintypes.BOOL
        k.LoadLibraryExW.argtypes = [wintypes.LPCWSTR, wintypes.HANDLE, wintypes.DWORD]
        k.LoadLibraryExW.restype = wintypes.HMODULE
        k.GetProcAddress.argtypes = [wintypes.HMODULE, ctypes.c_char_p]
        k.GetProcAddress.restype = ctypes.c_void_p
        k.FreeLibrary.argtypes = [wintypes.HMODULE]
        k.VirtualAllocEx.argtypes = [wintypes.HANDLE, ctypes.c_void_p, ctypes.c_size_t, wintypes.DWORD, wintypes.DWORD]
        k.VirtualAllocEx.restype = ctypes.c_void_p
        k.VirtualFreeEx.argtypes = [wintypes.HANDLE, ctypes.c_void_p, ctypes.c_size_t, wintypes.DWORD]
        k.WriteProcessMemory.argtypes = [wintypes.HANDLE, ctypes.c_void_p, ctypes.c_void_p, ctypes.c_size_t, ctypes.POINTER(ctypes.c_size_t)]
        k.ReadProcessMemory.argtypes = [wintypes.HANDLE, ctypes.c_void_p, ctypes.c_void_p, ctypes.c_size_t, ctypes.POINTER(ctypes.c_size_t)]
        k.CreateRemoteThread.argtypes = [wintypes.HANDLE, ctypes.c_void_p, ctypes.c_size_t, ctypes.c_void_p, ctypes.c_void_p, wintypes.DWORD, ctypes.POINTER(wintypes.DWORD)]
        k.CreateRemoteThread.restype = wintypes.HANDLE
        k.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
        k.GetExitCodeThread.argtypes = [wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD)]
        u.EnumWindows.restype = wintypes.BOOL
        u.GetWindowThreadProcessId.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.DWORD)]
        u.GetClientRect.argtypes = [wintypes.HWND, ctypes.POINTER(Rect)]
        u.ClientToScreen.argtypes = [wintypes.HWND, ctypes.POINTER(Point)]
        u.SetCursorPos.argtypes = [ctypes.c_int, ctypes.c_int]
        u.GetCursorPos.argtypes = [ctypes.POINTER(Point)]
        u.SetForegroundWindow.argtypes = [wintypes.HWND]
        u.ShowWindowAsync.argtypes = [wintypes.HWND, ctypes.c_int]
        u.GetForegroundWindow.restype = wintypes.HWND
        u.mouse_event.argtypes = [wintypes.DWORD, wintypes.DWORD, wintypes.DWORD, wintypes.DWORD, ctypes.c_size_t]
        u.keybd_event.argtypes = [wintypes.BYTE, wintypes.BYTE, wintypes.DWORD, ctypes.c_size_t]

    def _process_image(self) -> Path:
        buffer, size = ctypes.create_unicode_buffer(32768), wintypes.DWORD(32768)
        require(self.k32.QueryFullProcessImageNameW(self.process, 0, buffer, ctypes.byref(size)), "process image query failed")
        path = Path(buffer.value).resolve()
        require(path.name.lower() == "hoi4.exe", f"target is not hoi4.exe: {path}")
        return path

    def _module_base(self, expected: Path) -> int:
        snap = self.k32.CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, self.process_id)
        require(snap not in (0, INVALID_HANDLE_VALUE), "module snapshot failed")
        try:
            entry = ModuleEntry32W(); entry.dwSize = ctypes.sizeof(entry)
            present = bool(self.k32.Module32FirstW(snap, ctypes.byref(entry)))
            while present:
                if Path(entry.szExePath).resolve() == expected.resolve():
                    return ctypes.cast(entry.modBaseAddr, ctypes.c_void_p).value
                present = bool(self.k32.Module32NextW(snap, ctypes.byref(entry)))
        finally:
            self.k32.CloseHandle(snap)
        raise CaptureError(f"authenticated module is not loaded: {expected}")

    def _export_rva(self) -> int:
        local = self.k32.LoadLibraryExW(str(self.collector), None, DONT_RESOLVE_DLL_REFERENCES)
        require(bool(local), "collector export image could not be mapped")
        try:
            address = self.k32.GetProcAddress(local, EXPORT)
            require(bool(address), "collector lacks A24 command export")
            base = local if isinstance(local, int) else ctypes.cast(local, ctypes.c_void_p).value
            address = address if isinstance(address, int) else ctypes.cast(address, ctypes.c_void_p).value
            return address - base
        finally:
            self.k32.FreeLibrary(local)

    def _window(self) -> int:
        found: list[int] = []
        callback_type = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
        def visit(hwnd: int, _: int) -> bool:
            pid = wintypes.DWORD()
            self.u32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
            rect = Rect()
            if pid.value == self.process_id and self.u32.GetClientRect(hwnd, ctypes.byref(rect)) and rect.right > 0:
                found.append(hwnd)
            return True
        self.u32.EnumWindows(callback_type(visit), 0)
        require(len(found) == 1, f"expected one authenticated the game window, found {len(found)}")
        return found[0]

    def read(self, address: int, size: int) -> bytes:
        buffer = ctypes.create_string_buffer(size); count = ctypes.c_size_t()
        require(self.k32.ReadProcessMemory(self.process, address, buffer, size, ctypes.byref(count)) and count.value == size, f"read failed at 0x{address:x}")
        return buffer.raw

    def observation(self) -> tuple[int, int]:
        game_state = int.from_bytes(self.read(self.module_base + GAME_STATE_GLOBAL_RVA, 8), "little")
        require(game_state != 0, "game-state singleton is null")
        hour = int.from_bytes(self.read(game_state + GAME_HOUR_OFFSET, 4), "little", signed=True)
        primary = int.from_bytes(self.read(game_state + PRIMARY_COUNTRY_OFFSET, 4), "little", signed=True)
        fallback = int.from_bytes(self.read(game_state + FALLBACK_COUNTRY_OFFSET, 4), "little", signed=True)
        return hour, primary if primary > 0 else fallback

    def assert_window(self, width: int, height: int) -> None:
        rect = Rect(); require(self.u32.GetClientRect(self.window, ctypes.byref(rect)), "GetClientRect failed")
        require((rect.right, rect.bottom) == (width, height), "client size differs from frozen profile")
        self.u32.ShowWindowAsync(self.window, 9); self.u32.SetForegroundWindow(self.window)
        time.sleep(0.25)
        require(self.u32.GetForegroundWindow() == self.window, "the game window is not foreground")

    def command(self, body: dict[str, object]) -> None:
        encoded = json.dumps(body, separators=(",", ":")).encode() + b"\0"
        remote = self.k32.VirtualAllocEx(self.process, None, len(encoded), MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE)
        require(bool(remote), "VirtualAllocEx failed")
        try:
            source, written = ctypes.create_string_buffer(encoded), ctypes.c_size_t()
            require(self.k32.WriteProcessMemory(self.process, remote, source, len(encoded), ctypes.byref(written)) and written.value == len(encoded), "WriteProcessMemory failed")
            tid = wintypes.DWORD()
            thread = self.k32.CreateRemoteThread(self.process, None, 0, self.command_address, remote, 0, ctypes.byref(tid))
            require(bool(thread), "CreateRemoteThread failed")
            try:
                require(self.k32.WaitForSingleObject(thread, INFINITE) == WAIT_OBJECT_0, "A24 remote command wait failed")
                code = wintypes.DWORD(); require(self.k32.GetExitCodeThread(thread, ctypes.byref(code)), "remote exit-code query failed")
                require(code.value == 0, f"A24 collector command rejected with code {code.value}")
            finally:
                self.k32.CloseHandle(thread)
        finally:
            self.k32.VirtualFreeEx(self.process, remote, 0, MEM_RELEASE)

    def point(self, x: int, y: int, width: int, height: int) -> None:
        self.assert_window(width, height)
        require(0 <= x < width and 0 <= y < height, "profile point outside client")
        point = Point(x, y); require(self.u32.ClientToScreen(self.window, ctypes.byref(point)), "ClientToScreen failed")
        require(self.u32.SetCursorPos(point.x, point.y), "SetCursorPos failed")

    def close(self) -> None:
        if self.process:
            self.k32.CloseHandle(self.process); self.process = None


def run(args: argparse.Namespace) -> dict[str, object]:
    profile = json.loads(args.profile.read_text(encoding="utf-8-sig"))
    require(profile.get("schema") == "hoi4.a24.autonomous-politics-scenes.v1", "profile schema mismatch")
    authority, policy, scenes = profile["authority"], profile["policy"], profile["scenes"]
    require(authority["executable_sha256"] == EXE_SHA256, "profile executable authority mismatch")
    require(len(scenes) == 6 and {scene["line_kind"] for scene in scenes} == REQUIRED_LINE_KINDS, "profile must own exactly all six semantic surfaces")
    require(len({scene["id"] for scene in scenes}) == 6, "scene identities are duplicated")
    width, height = authority["client_size"]
    output = args.output.resolve(); require(output.is_dir(), "output directory must already exist")
    receipt_path = output / "a24_autonomous_scene_actions.json"
    require(not receipt_path.exists(), "action receipt already exists")
    owner = RemoteOwner(args.process_id, args.collector_dll)
    original_cursor = Point(); owner.u32.GetCursorPos(ctypes.byref(original_cursor))
    installed = False; action_rows: list[dict[str, object]] = []; case_rows: list[dict[str, object]] = []
    expected_hour = int(authority["initial_game_hour_raw"])
    try:
        owner.assert_window(width, height)
        require(owner.observation() == (expected_hour, int(authority["country_handle"])), "initial paused save identity mismatch")
        time.sleep(1.25); require(owner.observation()[0] == expected_hour, "game is not paused at initial boundary")
        owner.command({"operation": "install"}); installed = True
        for index, scene in enumerate(scenes):
            require(owner.observation() == (expected_hour, int(authority["country_handle"])), f"scene {scene['id']} boundary changed")
            owner.command({"operation": "arm", "frame_hour": str(expected_hour)})
            for action in scene["actions"]:
                kind, label = action["kind"], action["label"]
                started = time.time_ns()
                if kind in {"click", "hover"}:
                    owner.point(int(action["x"]), int(action["y"]), width, height)
                    if kind == "click":
                        owner.u32.mouse_event(0x0002, 0, 0, 0, 0); owner.u32.mouse_event(0x0004, 0, 0, 0, 0)
                    time.sleep(policy["tooltip_dwell_milliseconds"] / 1000 if kind == "hover" else policy["ui_settle_milliseconds"] / 1000)
                elif kind == "hover_sweep":
                    require(action["points"], "hover sweep is empty")
                    for x, y in action["points"]:
                        owner.point(int(x), int(y), width, height)
                        time.sleep(policy["tooltip_dwell_milliseconds"] / 1000)
                elif kind == "wheel":
                    owner.point(int(action["x"]), int(action["y"]), width, height)
                    owner.u32.mouse_event(0x0800, 0, 0, int(action["delta"]) & 0xFFFFFFFF, 0)
                    time.sleep(policy["ui_settle_milliseconds"] / 1000)
                elif kind == "key":
                    owner.assert_window(width, height)
                    virtual_key, scan_code = int(action["virtual_key"]), int(action["scan_code"])
                    require(0 <= virtual_key <= 255 and 0 <= scan_code <= 255, "profile key is outside byte range")
                    owner.u32.keybd_event(virtual_key, scan_code, 0, 0)
                    owner.u32.keybd_event(virtual_key, scan_code, 2, 0)
                    time.sleep(policy["ui_settle_milliseconds"] / 1000)
                else:
                    raise CaptureError(f"unsupported profile action {kind}")
                action_rows.append({"scene_id": scene["id"], "kind": kind, "label": label, "started_unix_ns": started, "completed_unix_ns": time.time_ns()})
            time.sleep(policy["present_dwell_milliseconds"] / 1000)
            case_path = output / f"a24_politics_producer_capture_{index + 1:02d}_{scene['id'].lower()}.json"
            owner.command({"operation": "seal", "frame_hour": str(expected_hour), "target_game_hour_raw": expected_hour, "country_tag": authority["country_tag"], "output_path": str(case_path)})
            case = json.loads(case_path.read_text(encoding="utf-8-sig"))
            require(case.get("schema") == "hoi4-a24-producer-capture-case-v3" and case.get("frame_hour") == expected_hour, "collector case receipt identity mismatch")
            case_rows.append({"scene_id": scene["id"], "line_kind": scene["line_kind"], "frame_hour": expected_hour, "receipt_file_name": case_path.name, "receipt_sha256": sha256_file(case_path), "observed_tap_kinds": case.get("observed_tap_kinds", [])})
            if index + 1 < len(scenes):
                owner.u32.keybd_event(0x31, 0x02, 0, 0); owner.u32.keybd_event(0x31, 0x02, 2, 0)
                owner.u32.keybd_event(0x20, 0x39, 0, 0); owner.u32.keybd_event(0x20, 0x39, 2, 0)
                deadline = time.monotonic() + args.hour_timeout_seconds
                while time.monotonic() < deadline and owner.observation()[0] == expected_hour:
                    time.sleep(0.05)
                owner.u32.keybd_event(0x20, 0x39, 0, 0); owner.u32.keybd_event(0x20, 0x39, 2, 0)
                next_hour = owner.observation()[0]
                require(next_hour == expected_hour + 1, f"hour pulse advanced {next_hour - expected_hour} hours")
                time.sleep(1.0); require(owner.observation()[0] == next_hour, "repause did not freeze next scene hour")
                expected_hour = next_hour
        owner.command({"operation": "uninstall"}); installed = False
    finally:
        if installed:
            try: owner.command({"operation": "uninstall"})
            except Exception: pass
        owner.u32.SetCursorPos(original_cursor.x, original_cursor.y)
        owner.close()
    result = {"schema": "hoi4.a24.autonomous-scene-actions.v1", "status": "PASS_MACHINE_OWNED_SIX_SURFACE_SELECTED_PRESENT", "profile_path": str(args.profile.resolve()), "profile_sha256": sha256_file(args.profile), "process_id": args.process_id, "collector_sha256": sha256_file(args.collector_dll), "initial_game_hour_raw": authority["initial_game_hour_raw"], "final_game_hour_raw": expected_hour, "country_tag": authority["country_tag"], "case_receipts": case_rows, "actions": action_rows, "immutable": True, "human_input_used": False, "fabricated_present_or_rows": False}
    write_new_json(receipt_path, result)
    return {**result, "receipt_path": str(receipt_path)}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--process-id", type=int, required=True)
    parser.add_argument("--collector-dll", type=Path, required=True)
    parser.add_argument("--profile", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--hour-timeout-seconds", type=int, default=60)
    args = parser.parse_args()
    try:
        print(json.dumps(run(args), sort_keys=True))
        return 0
    except Exception as error:
        print(json.dumps({"status": "FAIL", "error": str(error)}, sort_keys=True))
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
