#!/usr/bin/env python3
"""Owner-runnable exact-a729 A31 stable/transition capture and regrader.

The collector must already be injected by the custom harness.  This driver
invokes its one-argument JSON command export with CreateRemoteThread, collects
the two committed result descriptors, and runs both fail-closed regraders.
It does not enable compact d89.
"""

from __future__ import annotations

import argparse
import ctypes
import hashlib
import json
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
WAIT_OBJECT_0 = 0
INFINITE = 0xFFFFFFFF
DONT_RESOLVE_DLL_REFERENCES = 0x00000001
INVALID_HANDLE_VALUE = ctypes.c_void_p(-1).value


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


def prompt(message: str) -> None:
    input(f"\n{message}\nPress Enter when ready... ")


def capture(args: argparse.Namespace) -> dict[str, object]:
    output_root = args.output_root.resolve()
    output_root.mkdir(parents=True, exist_ok=True)
    require(args.stable_frame_hour < args.transition_frame_hour,
            "transition frame hour must be later than stable frame hour")
    require(len(args.country_tag) == 3 and args.country_tag.isalnum()
            and args.country_tag == args.country_tag.upper(), "country tag is not canonical")
    run_id = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    stable_result = output_root / f"{run_id}-stable.result.json"
    transition_result = output_root / f"{run_id}-transition.result.json"
    pair_result = output_root / f"{run_id}-fixture-pair.regrade.json"
    for path in (stable_result, transition_result, pair_result):
        require(not path.exists(), f"refusing to overwrite {path}")

    remote = RemoteCollector(args.process_id, args.collector_dll)
    start = time.monotonic()
    installed = False
    try:
        prompt(
            "Close the national-focus panel before instrumentation is installed. "
            "This keeps both exact mid-function patch sites quiescent during the byte-gated install."
        )
        remote.command({"operation": "install", "output_root": str(output_root)})
        installed = True
        prompt(
            "Prepare a stable country view containing at least one positive joint-country "
            "focus item, but leave the national-focus panel closed. "
            "The next step arms before the panel is opened so vector publication is witnessed."
        )
        remote.command({
            "operation": "arm",
            "frame_hour": str(args.stable_frame_hour),
            "country_tag": args.country_tag,
            "country_object_address": str(args.country_object_address),
        })
        prompt("Open the national-focus panel, leave it stable for several rendered frames")
        remote.command({
            "operation": "seal",
            "target_game_hour_raw": args.stable_game_hour_raw,
            "elapsed_wall_ms": str(int((time.monotonic() - start) * 1000)),
            "result_path": str(stable_result),
        })

        prompt(
            "Close the national-focus panel and prepare, but do not yet apply, the later "
            "discriminating focus-state/progress/joint-country transition. Keep at least "
            "one positive joint-country focus item in the later view"
        )
        remote.command({
            "operation": "arm",
            "frame_hour": str(args.transition_frame_hour),
            "country_tag": args.country_tag,
            "country_object_address": str(args.country_object_address),
        })
        prompt("Apply or expose the transition and wait until the focus panel visibly refreshes")
        remote.command({
            "operation": "seal",
            "target_game_hour_raw": args.transition_game_hour_raw,
            "elapsed_wall_ms": str(int((time.monotonic() - start) * 1000)),
            "result_path": str(transition_result),
        })
        prompt(
            "Close the national-focus panel before instrumentation teardown so both exact "
            "mid-function sites are quiescent"
        )
        remote.command({"operation": "uninstall"})
        installed = False
    finally:
        if installed:
            try:
                prompt(
                    "The capture did not complete normally. Close the national-focus panel "
                    "before the mandatory instrumentation teardown retry"
                )
                remote.command({"operation": "uninstall"})
            except Exception as error:
                print(f"warning: A31 teardown retry failed: {error}", file=sys.stderr)
        remote.close()

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
        "stable_receipt": str(stable_receipt),
        "transition_receipt": str(transition_receipt),
        "stable_grade": stable_grade,
        "transition_grade": transition_grade,
        "matrix_grade": matrix_grade,
        "production_scheduling_enabled": False,
    }
    with pair_result.open("x", encoding="utf-8", newline="\n") as stream:
        json.dump(result, stream, indent=2, sort_keys=True)
        stream.write("\n")
    return {**result, "pair_result": str(pair_result)}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--process-id", type=int, required=True)
    parser.add_argument("--collector-dll", type=Path, required=True)
    parser.add_argument("--output-root", type=Path, required=True)
    parser.add_argument("--country-tag", required=True)
    parser.add_argument("--country-object-address", type=lambda value: int(value, 0), required=True)
    parser.add_argument("--stable-frame-hour", type=int, required=True)
    parser.add_argument("--stable-game-hour-raw", type=int, required=True)
    parser.add_argument("--transition-frame-hour", type=int, required=True)
    parser.add_argument("--transition-game-hour-raw", type=int, required=True)
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
