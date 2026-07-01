"""No-op collector attach helper.

This is the first live-process attach path for the in-process collector. It uses
SetWindowsHookEx with the DLL's exported no-op GetMessage hook so Windows loads
the collector into the target UI thread without WriteProcessMemory. The DLL's
DllMain writes the attach heartbeat; real domain readers are added later behind
build/profile gates.
"""
from __future__ import annotations

import argparse
import ctypes
from ctypes import wintypes
from dataclasses import dataclass
import json
from pathlib import Path
import threading
import time
from typing import Any, Dict, List, Optional
import zlib

from .collector_frames import validate_collector_frame


WH_GETMESSAGE = 3
WM_NULL = 0x0000
PIPE_ACCESS_INBOUND = 0x00000001
PIPE_TYPE_BYTE = 0x00000000
PIPE_READMODE_BYTE = 0x00000000
PIPE_WAIT = 0x00000000
ERROR_PIPE_CONNECTED = 535
INVALID_HANDLE_VALUE = ctypes.c_void_p(-1).value


class AttachError(RuntimeError):
    pass


@dataclass(frozen=True)
class AttachResult:
    pid: int
    thread_id: int
    dll_path: Path
    hook_proc: int
    pipe_frame: Optional[Dict[str, Any]] = None
    pipe_frames: Optional[List[Dict[str, Any]]] = None


if ctypes.sizeof(ctypes.c_void_p) != 8:  # pragma: no cover - host invariant on target machines.
    raise AttachError("collector attach helper must run from a 64-bit Python process")


kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
user32 = ctypes.WinDLL("user32", use_last_error=True)

kernel32.LoadLibraryW.argtypes = [wintypes.LPCWSTR]
kernel32.LoadLibraryW.restype = wintypes.HMODULE
kernel32.GetProcAddress.argtypes = [wintypes.HMODULE, wintypes.LPCSTR]
kernel32.GetProcAddress.restype = ctypes.c_void_p
kernel32.FreeLibrary.argtypes = [wintypes.HMODULE]
kernel32.FreeLibrary.restype = wintypes.BOOL
kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
kernel32.CloseHandle.restype = wintypes.BOOL
kernel32.ConnectNamedPipe.argtypes = [wintypes.HANDLE, wintypes.LPVOID]
kernel32.ConnectNamedPipe.restype = wintypes.BOOL
kernel32.CreateNamedPipeW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD,
                                      wintypes.DWORD, wintypes.DWORD, wintypes.DWORD,
                                      wintypes.DWORD, wintypes.LPVOID]
kernel32.CreateNamedPipeW.restype = wintypes.HANDLE
kernel32.ReadFile.argtypes = [wintypes.HANDLE, wintypes.LPVOID, wintypes.DWORD,
                              ctypes.POINTER(wintypes.DWORD), wintypes.LPVOID]
kernel32.ReadFile.restype = wintypes.BOOL

user32.EnumWindows.argtypes = [ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM),
                               wintypes.LPARAM]
user32.EnumWindows.restype = wintypes.BOOL
user32.GetWindowThreadProcessId.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.DWORD)]
user32.GetWindowThreadProcessId.restype = wintypes.DWORD
user32.IsWindowVisible.argtypes = [wintypes.HWND]
user32.IsWindowVisible.restype = wintypes.BOOL
user32.PostThreadMessageW.argtypes = [wintypes.DWORD, wintypes.UINT, wintypes.WPARAM,
                                      wintypes.LPARAM]
user32.PostThreadMessageW.restype = wintypes.BOOL
user32.SetWindowsHookExW.argtypes = [ctypes.c_int, ctypes.c_void_p, wintypes.HMODULE,
                                     wintypes.DWORD]
user32.SetWindowsHookExW.restype = wintypes.HHOOK
user32.UnhookWindowsHookEx.argtypes = [wintypes.HHOOK]
user32.UnhookWindowsHookEx.restype = wintypes.BOOL


def attach_noop(pid: int, dll_path: Path, *, dwell_s: float = 1.0,
                use_pipe: bool = False, pipe_token: Optional[str] = None) -> AttachResult:
    dll = dll_path.resolve()
    if not dll.is_file():
        raise AttachError(f"collector DLL does not exist: {dll}")

    hwnd = _find_main_window(pid)
    if hwnd is None:
        raise AttachError(f"no visible top-level window found for pid {pid}")

    target_pid = wintypes.DWORD()
    thread_id = user32.GetWindowThreadProcessId(hwnd, ctypes.byref(target_pid))
    if not thread_id:
        raise AttachError(f"GetWindowThreadProcessId failed: {_last_error()}")
    if int(target_pid.value) != pid:
        raise AttachError(f"window pid {target_pid.value} did not match target pid {pid}")

    pipe_server = (
        _PipeServer(pid, timeout_s=max(3.0, dwell_s + 3.0), pipe_token=pipe_token)
        if use_pipe else None
    )
    if pipe_server is not None:
        pipe_server.start()

    module = kernel32.LoadLibraryW(str(dll))
    if not module:
        raise AttachError(f"LoadLibraryW failed in controller process: {_last_error()}")
    try:
        proc = kernel32.GetProcAddress(module, b"Hoi4ReplayGetMsgProc")
        if not proc:
            raise AttachError(f"GetProcAddress(Hoi4ReplayGetMsgProc) failed: {_last_error()}")
        hook = user32.SetWindowsHookExW(WH_GETMESSAGE, proc, module, thread_id)
        if not hook:
            raise AttachError(f"SetWindowsHookExW failed: {_last_error()}")
        try:
            user32.PostThreadMessageW(thread_id, WM_NULL, 0, 0)
            if pipe_server is not None:
                pipe_server.wait(dwell_s)
            else:
                time.sleep(dwell_s)
        finally:
            user32.UnhookWindowsHookEx(hook)
    finally:
        kernel32.FreeLibrary(module)

    pipe_frames = pipe_server.finish() if pipe_server is not None else None
    pipe_frame = pipe_frames[-1] if pipe_frames else None
    return AttachResult(pid=pid, thread_id=int(thread_id), dll_path=dll, hook_proc=int(proc),
                        pipe_frame=pipe_frame, pipe_frames=pipe_frames)


class _PipeServer:
    def __init__(self, pid: int, *, timeout_s: float = 3.0,
                 pipe_token: Optional[str] = None) -> None:
        self.pid = pid
        self.timeout_s = timeout_s
        suffix = f"_{pipe_token}" if pipe_token else ""
        self.pipe_name = rf"\\.\pipe\battle-plan_collector_{pid}{suffix}"
        self._thread: Optional[threading.Thread] = None
        self._frames: List[Dict[str, Any]] = []
        self._error: Optional[BaseException] = None
        self._done = threading.Event()
        self._handle_lock = threading.Lock()
        self._handle: Optional[int] = None

    def start(self) -> None:
        self._thread = threading.Thread(target=self._run, name="collector-pipe-server",
                                        daemon=True)
        self._thread.start()

    def finish(self) -> List[Dict[str, Any]]:
        if self._thread is None:
            return []
        self._thread.join(self.timeout_s)
        if self._thread.is_alive():
            self._close_pipe_handle()
            self._thread.join(1.0)
            raise AttachError(f"collector pipe timed out: {self.pipe_name}")
        if self._error is not None:
            raise AttachError(f"collector pipe failed: {self._error}") from self._error
        return self._frames

    def wait(self, timeout_s: float) -> None:
        self._done.wait(max(timeout_s, 0.0))

    def _run(self) -> None:
        handle = kernel32.CreateNamedPipeW(
            self.pipe_name,
            PIPE_ACCESS_INBOUND,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
            1,
            65536,
            65536,
            int(self.timeout_s * 1000),
            None,
        )
        if handle == INVALID_HANDLE_VALUE:
            self._error = RuntimeError(f"CreateNamedPipeW failed: {_last_error()}")
            return
        self._set_pipe_handle(handle)
        try:
            connected = kernel32.ConnectNamedPipe(handle, None)
            if not connected and _last_error() != ERROR_PIPE_CONNECTED:
                self._error = RuntimeError(f"ConnectNamedPipe failed: {_last_error()}")
                return
            chunks: List[bytes] = []
            while True:
                buf = ctypes.create_string_buffer(65536)
                got = wintypes.DWORD()
                ok = kernel32.ReadFile(handle, buf, len(buf), ctypes.byref(got), None)
                if not ok or got.value == 0:
                    break
                chunks.append(bytes(buf[:got.value]))
            raw_bytes = b"".join(chunks)
            if not raw_bytes:
                self._error = RuntimeError("collector pipe returned no bytes")
                return
            for raw_line in raw_bytes.splitlines():
                if not raw_line.strip():
                    continue
                frame = json.loads(raw_line.decode("utf-8"))
                frame.setdefault("transport", "named_pipe")
                frame["transport_bytes"] = len(raw_line)
                frame["transport_crc32"] = f"{zlib.crc32(raw_line) & 0xFFFFFFFF:08x}"
                problems = validate_collector_frame(frame)
                if problems:
                    self._error = RuntimeError("invalid collector pipe frame: "
                                               + "; ".join(problems))
                    return
                self._frames.append(frame)
        except BaseException as e:  # keep background exceptions visible to the caller.
            self._error = e
        finally:
            self._close_pipe_handle()
            self._done.set()

    def _set_pipe_handle(self, handle: int) -> None:
        with self._handle_lock:
            self._handle = int(handle)

    def _close_pipe_handle(self) -> None:
        with self._handle_lock:
            handle = self._handle
            self._handle = None
        if handle is not None:
            kernel32.CloseHandle(handle)


def _find_main_window(pid: int) -> Optional[int]:
    found: list[int] = []
    callback_type = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)

    @callback_type
    def enum_proc(hwnd: wintypes.HWND, _lparam: wintypes.LPARAM) -> wintypes.BOOL:
        if not user32.IsWindowVisible(hwnd):
            return True
        owner = wintypes.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(owner))
        if int(owner.value) == pid:
            found.append(int(hwnd))
            return False
        return True

    user32.EnumWindows(enum_proc, 0)
    return found[0] if found else None


def _last_error() -> int:
    return ctypes.get_last_error()


def _main() -> int:
    parser = argparse.ArgumentParser(description="No-op attach collector.dll to a running the game PID")
    parser.add_argument("--pid", type=int, required=True)
    parser.add_argument("--dll", type=Path, required=True)
    parser.add_argument("--dwell-s", type=float, default=1.0)
    parser.add_argument("--pipe", action="store_true")
    args = parser.parse_args()

    result = attach_noop(args.pid, args.dll, dwell_s=args.dwell_s, use_pipe=args.pipe)
    print(f"pid={result.pid}")
    print(f"thread_id={result.thread_id}")
    print(f"dll={result.dll_path}")
    print(f"hook_proc=0x{result.hook_proc:x}")
    if result.pipe_frame is not None:
        print("pipe_frame=" + json.dumps(result.pipe_frame, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(_main())
