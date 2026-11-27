"""BattlePlan's acceptance suite: the release desktop app on a copy of a real recording.

    python tools/e2e/suite.py shell [--commit <ref>] [--out <shell dir> | --gate-dir <gate dir>] [--dispatch] [--repo <owner/name>]
    python tools/e2e/suite.py build --out <app dir> [--shell]
    python tools/e2e/suite.py build-ui --out <bundle dir>
    python tools/e2e/suite.py run --app <app dir> | --shell <shell dir> --ui <bundle dir>
        --fixture <folder> --game-dir <the game folder> --out <report dir>
        [--tier full|quick] [--history <file>] [--pin all|product] [--record-replies <file>]
        [--state <dir>] [--stages copy,sweep,report] [--phases pages,...] [--cases id,...] [--pages page,...] [--pages-at <fraction>]
    python tools/e2e/suite.py accept --run <report dir> --reason "<the change it accepts>" [--state <dir>]
    python tools/e2e/suite.py playback map|harness|dashboard ...   (tools/e2e/playback.py: the playback phase)

`shell` fetches and verifies the hosted measuring shell built by `.github/workflows/measuring-shell.yml` for `--commit`
(HEAD by default) via `gh`, reusing one already on disk; for a commit on main, `--gate-dir` (or `BATTLEPLAN_GATE_DIR`)
places it in `<gate dir>/shared-shell/<sha[:10]>` so every agent shares one download. If no artifact exists yet,
`--dispatch` starts the hosted build and prints the command to fetch it when it finishes. `build` compiles replay-desktop
with the UI embedded on Boat; `build --shell` compiles the measuring shell on Boat (prefer `suite.py shell` to avoid
Boat). The measuring shell is the same release app serving its UI from a production bundle on disk, so a UI change is
measured by rebuilding only the bundle (`suite.py build-ui --out <bundle dir>` or `npx.cmd vite build --config ui/vite.config.ts`)
and running the shell on it with `run --shell <shell dir> --ui <bundle dir>`; a shell is built once and serves any later
bundle. `run` copies the fixture's recording into a fresh profile, then starts the app on that profile for each session
and runs tools/e2e/sweep.mjs over it, and writes e2e.md: a table per area, failures with screenshots, and the visual
check's differences from the baseline `accept` last made, with diff images. `--pages-at` adds a session that seeks to that
share of the timeline and visits and scrolls every page there, into sweep-pages-at.json. Each app runs in a Job on a
desktop of its own, so it never takes the user's input or focus, at a fixed window size. `--tier quick` is one cold
session for iterating: every page at mid-recording, the heaviest pages played at 1,440 h/s under the trace, the slowest
catalogue cards and one setting of each widget kind. `--history` (by default in the state folder) holds every step that
ever failed, each marked as the product's or the suite's own; the report adds this run's, and no case in it is left out
as a repeat (with `--pin product`, no case the product failed). The header records the machine's CPU and memory load
before the first app starts and before the last one closes, beside the BattlePlan apps running then: this run's, other
suite runs' (each notes the app it started) and every replay-desktop process on the machine, so a number taken beside
other apps is read as such. `--record-replies` has the sweep record every reply and followed message the page receives to
that file (one file a session, named for it when more than one runs). Exits 0 when everything passed, 1 when anything
failed, 2 when the suite itself could not run.
"""

import argparse
import ctypes as c
import datetime
import hashlib
import json
import os
import re
import shutil
import socket
import stat
import subprocess
import sys
import tempfile
import time
import uuid
from ctypes import wintypes as w
from pathlib import Path

APP_TITLES = ("BattlePlan (Alpha)", "BattlePlan (Nightly)")

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
def _default_state(env=None):
    source_env = os.environ if env is None else env
    captures = source_env.get("BATTLEPLAN_CAPTURES") or source_env.get("HOI4_CAPTURES")
    return (Path(captures) if captures else ROOT / "captures") / "ci" / "e2e"


DEFAULT_STATE = _default_state()
WINDOW = (1920, 1080)
SCREEN_VIEWS, TALL_VIEW, SCREEN = ("AllWidgets", "AllWidgetsLegible", "Heaviest"), "AllWidgetsTall", (3840, 2160)
PROFILES = {
    "weak": {"cores": "4", "cpu_throttle": 4.0},
    "weak8": {"cores": "4", "cpu_throttle": 8.0},
}
BOAT_FAILED = 75
CLOSE_SECONDS = 60
STAGES = ("copy", "sweep", "report")
SESSIONS = (
    ("pages", "pages", {}),
    ("seeks", "seeks", {}),
    ("map", "visual,map,playback", {}),
    ("map uncapped", "mapfps", {"BATTLEPLAN_VIEWER_NO_VSYNC": "1"}),
    ("catalogue", "catalogue", {}),
    ("controls", "controls", {}),
    ("pages at", "pages-at", {}),
    ("widget costs", "widget-costs", {}),
)
QUICK_SESSIONS = (
    ("quick", "glance,playback,catalogue,controls", {}),
    ("widget costs", "widget-costs", {}),
)
CACHES = ("local/BattlePlan/viewer", "local/BattlePlan/shader-cache", "local/BattlePlan/artwork", "local/Microsoft",
          "roaming", "webview", "recorder-cache")
KEPT = ("local/BattlePlan/captures",)
APPS = Path(tempfile.gettempdir()) / "battleplan-e2e" / "apps"
APP_EXE = "replay-desktop.exe"
SHELL_PACKAGE = "replay-desktop"
SHELL_PROFILE = "release"
SHELL_BASE_FEATURE = "replay-desktop/custom-protocol"
SHELL_FEATURE = "replay-desktop/served-ui"
SHELL_FEATURES = (SHELL_BASE_FEATURE, SHELL_FEATURE)
SHELL_WORKFLOW = "measuring-shell.yml"
SHELL_ARTIFACT_PREFIX = "measuring-shell-"
SHELL_RECORD = "shell-build.json"
BOAT_RECORD = "boat-artifacts.json"
SHELL_RECORDS = (SHELL_RECORD, BOAT_RECORD)
SHELL_TARGET = "x86_64-pc-windows-msvc"
SHELL_RUN_NAME_PREFIX = "Measuring shell "
GATE_DIR_ENV = "BATTLEPLAN_GATE_DIR"
FULL_SHA = re.compile(r"^[0-9a-f]{40}$")
GITHUB_REMOTE = re.compile(r"^(?:https?://github\.com/|ssh://git@github\.com/|git@github\.com:)([^/]+/[^/]+?)(?:\.git)?/?$")
SHELL_PLACEHOLDER_HTML = "<!doctype html><title>BattlePlan Measuring Shell</title>\n"

k = c.WinDLL("kernel32", use_last_error=True)
u = c.WinDLL("user32", use_last_error=True)
u.SetProcessDpiAwarenessContext.argtypes = [c.c_void_p]
u.SetProcessDpiAwarenessContext(c.c_void_p(-4))


def checked(value):
    if not value:
        raise c.WinError(c.get_last_error())
    return value


class Basic(c.Structure):
    _fields_ = [("process_time", c.c_int64), ("job_time", c.c_int64), ("flags", w.DWORD),
                ("min_ws", c.c_size_t), ("max_ws", c.c_size_t), ("active_limit", w.DWORD),
                ("affinity", c.c_size_t), ("priority", w.DWORD), ("scheduling", w.DWORD)]


class IO(c.Structure):
    _fields_ = [(name, c.c_uint64) for name in ("read_ops", "write_ops", "other_ops", "read_bytes", "write_bytes", "other_bytes")]


class Extended(c.Structure):
    _fields_ = [("basic", Basic), ("io", IO), ("process_memory", c.c_size_t),
                ("job_memory", c.c_size_t), ("peak_process", c.c_size_t), ("peak_job", c.c_size_t)]


class Accounting(c.Structure):
    _fields_ = [(name, c.c_int64) for name in ("user", "kernel", "period_user", "period_kernel")] + [
        (name, w.DWORD) for name in ("faults", "total", "active", "terminated")]


class Startup(c.Structure):
    _fields_ = [("cb", w.DWORD), ("reserved", w.LPWSTR), ("desktop", w.LPWSTR), ("title", w.LPWSTR),
                ("x", w.DWORD), ("y", w.DWORD), ("width", w.DWORD), ("height", w.DWORD), ("xchars", w.DWORD),
                ("ychars", w.DWORD), ("fill", w.DWORD), ("flags", w.DWORD), ("show", w.WORD),
                ("reserved2size", w.WORD), ("reserved2", c.c_void_p), ("stdin", w.HANDLE), ("stdout", w.HANDLE),
                ("stderr", w.HANDLE)]


class DisplayMode(c.Structure):
    _fields_ = [("name", c.c_wchar * 32), ("spec", w.WORD), ("driver", w.WORD), ("size", w.WORD), ("extra", w.WORD),
                ("fields", w.DWORD), ("position", c.c_byte * 16), ("color", c.c_short), ("duplex", c.c_short),
                ("y_resolution", c.c_short), ("tt_option", c.c_short), ("collate", c.c_short), ("form", c.c_wchar * 32),
                ("log_pixels", w.WORD), ("bits", w.DWORD), ("width", w.DWORD), ("height", w.DWORD), ("flags", w.DWORD),
                ("frequency", w.DWORD), ("icm_method", w.DWORD), ("icm_intent", w.DWORD), ("media", w.DWORD),
                ("dither", w.DWORD), ("reserved1", w.DWORD), ("reserved2", w.DWORD), ("panning_width", w.DWORD),
                ("panning_height", w.DWORD)]


class ProcessInfo(c.Structure):
    _fields_ = [("process", w.HANDLE), ("thread", w.HANDLE), ("pid", w.DWORD), ("tid", w.DWORD)]


class MemoryStatus(c.Structure):
    _fields_ = [("length", w.DWORD), ("load", w.DWORD)] + [
        (name, c.c_uint64) for name in ("phys", "phys_free", "page", "page_free", "virtual", "virtual_free", "extended")]


class ProcessEntry(c.Structure):
    _fields_ = [("size", w.DWORD), ("usage", w.DWORD), ("pid", w.DWORD), ("heap", c.c_size_t), ("module", w.DWORD),
                ("threads", w.DWORD), ("parent", w.DWORD), ("priority", c.c_long), ("flags", w.DWORD),
                ("exe", c.c_wchar * 260)]


class JobProcessIdList(c.Structure):
    _fields_ = [("assigned", w.DWORD), ("in_list", w.DWORD), ("pids", c.c_size_t * 256)]


k.CreateJobObjectW.argtypes = [c.c_void_p, w.LPCWSTR]
k.CreateJobObjectW.restype = w.HANDLE
k.SetInformationJobObject.argtypes = [w.HANDLE, c.c_int, c.c_void_p, w.DWORD]
k.QueryInformationJobObject.argtypes = [w.HANDLE, c.c_int, c.c_void_p, w.DWORD, c.c_void_p]
k.AssignProcessToJobObject.argtypes = [w.HANDLE, w.HANDLE]
k.TerminateJobObject.argtypes = [w.HANDLE, w.UINT]
k.CreateProcessW.argtypes = [w.LPCWSTR, w.LPWSTR, c.c_void_p, c.c_void_p, w.BOOL, w.DWORD, c.c_void_p, w.LPCWSTR,
                             c.POINTER(Startup), c.POINTER(ProcessInfo)]
k.ResumeThread.argtypes = [w.HANDLE]
k.WaitForSingleObject.argtypes = [w.HANDLE, w.DWORD]
k.GetExitCodeProcess.argtypes = [w.HANDLE, c.POINTER(w.DWORD)]
k.CloseHandle.argtypes = [w.HANDLE]
k.GetCurrentProcess.restype = w.HANDLE
k.GetProcessAffinityMask.argtypes = [w.HANDLE, c.POINTER(c.c_size_t), c.POINTER(c.c_size_t)]
k.QueryFullProcessImageNameW.argtypes = [w.HANDLE, w.DWORD, w.LPWSTR, c.POINTER(w.DWORD)]
k.GetSystemTimes.argtypes = [c.POINTER(c.c_uint64)] * 3
k.GlobalMemoryStatusEx.argtypes = [c.POINTER(MemoryStatus)]
k.CreateToolhelp32Snapshot.argtypes = [w.DWORD, w.DWORD]
k.CreateToolhelp32Snapshot.restype = w.HANDLE
k.Process32FirstW.argtypes = [w.HANDLE, c.POINTER(ProcessEntry)]
k.Process32NextW.argtypes = [w.HANDLE, c.POINTER(ProcessEntry)]
k.OpenProcess.argtypes = [w.DWORD, w.BOOL, w.DWORD]
k.OpenProcess.restype = w.HANDLE
k.GetProcessTimes.argtypes = [w.HANDLE] + [c.POINTER(c.c_uint64)] * 4
u.CreateDesktopW.argtypes = [w.LPCWSTR, w.LPCWSTR, c.c_void_p, w.DWORD, w.DWORD, c.c_void_p]
u.CreateDesktopW.restype = w.HANDLE
u.CloseDesktop.argtypes = [w.HANDLE]
ENUM = c.WINFUNCTYPE(w.BOOL, w.HWND, w.LPARAM)
u.EnumDesktopWindows.argtypes = [w.HANDLE, ENUM, w.LPARAM]
u.GetWindowThreadProcessId.argtypes = [w.HWND, c.POINTER(w.DWORD)]
u.GetWindowTextW.argtypes = [w.HWND, w.LPWSTR, c.c_int]
u.IsWindowVisible.argtypes = [w.HWND]
u.GetWindowRect.argtypes = [w.HWND, c.POINTER(w.RECT)]
u.GetClientRect.argtypes = [w.HWND, c.POINTER(w.RECT)]
u.SetWindowPos.argtypes = [w.HWND, w.HWND, c.c_int, c.c_int, c.c_int, c.c_int, w.UINT]
u.ShowWindow.argtypes = [w.HWND, c.c_int]
u.PostMessageW.argtypes = [w.HWND, w.UINT, w.WPARAM, w.LPARAM]

u.EnumDisplaySettingsW.argtypes = [w.LPCWSTR, w.DWORD, c.POINTER(DisplayMode)]

WM_CLOSE = 0x10
STILL_ACTIVE = 258


def system_affinity_mask():
    process_mask, system_mask = c.c_size_t(), c.c_size_t()
    checked(k.GetProcessAffinityMask(k.GetCurrentProcess(), c.byref(process_mask), c.byref(system_mask)))
    return system_mask.value


def parse_cores(spec, system_mask=None):
    if spec is None:
        return None
    text = str(spec).strip()
    if not text:
        raise ValueError("--cores must be a positive core count or hex mask such as 4 or 0x55")
    allowed = system_affinity_mask() if system_mask is None else system_mask
    available = [bit for bit in range(64) if (allowed >> bit) & 1]
    if text.lower().startswith("0x"):
        mask = int(text, 16)
        if mask <= 0 or (mask & ~allowed):
            raise ValueError(f"--cores mask {text} is outside the system affinity mask {allowed:#x}")
        cpus = [bit for bit in available if (mask >> bit) & 1]
        return {"requested": text, "count": len(cpus), "mask": mask, "cpus": cpus}
    count = int(text, 10)
    if count < 1 or count > len(available):
        raise ValueError(f"--cores count {count} must be between 1 and {len(available)}")
    ordered = [bit for bit in available if bit % 2 == 0] + [bit for bit in available if bit % 2 == 1]
    cpus = ordered[:count]
    mask = 0
    for bit in cpus:
        mask |= 1 << bit
    return {"requested": text, "count": count, "mask": mask, "cpus": cpus}


class App:
    """The desktop app in a Job of its own, on a desktop of its own: only this tree is ever stopped."""

    def __init__(self, exe, args, environment, log, port, affinity_mask=None):
        self.port = port
        self.affinity_mask = affinity_mask
        self.desktop_name = f"BattlePlanE2E-{uuid.uuid4().hex[:12]}"
        self.desktop = checked(u.CreateDesktopW(self.desktop_name, None, None, 0, 0x1FF, None))
        self.job = checked(k.CreateJobObjectW(None, None))
        limits = Extended()
        limits.basic.flags = 0x2000 | 0x20 | (0x10 if affinity_mask else 0)
        limits.basic.priority = 0x8000
        if affinity_mask:
            limits.basic.affinity = affinity_mask
        checked(k.SetInformationJobObject(self.job, 9, c.byref(limits), c.sizeof(limits)))
        self.log = open(log, "wb")
        null = open(os.devnull, "rb")
        for stream in (self.log, null):
            os.set_handle_inheritable(msvcrt_handle(stream), True)
        startup = Startup()
        startup.cb = c.sizeof(startup)
        startup.desktop = "winsta0\\" + self.desktop_name
        startup.flags = 0x100
        startup.stdout = startup.stderr = msvcrt_handle(self.log)
        startup.stdin = msvcrt_handle(null)
        block = "".join(f"{key}={value}\0" for key, value in environment.items()) + "\0"
        self.info = ProcessInfo()
        command = c.create_unicode_buffer(subprocess.list2cmdline([str(exe), *args]))
        checked(k.CreateProcessW(str(exe), command, None, None, True, 0x08000004 | 0x400, (c.c_wchar * len(block))(*block),
                                 str(exe.parent), c.byref(startup), c.byref(self.info)))
        checked(k.AssignProcessToJobObject(self.job, self.info.process))
        if k.ResumeThread(self.info.thread) == 0xFFFFFFFF:
            raise c.WinError(c.get_last_error())
        null.close()
        self.started = time.monotonic()
        self.noted = note_app(self.info.process, self.info.pid)

    def usage(self):
        """CPU seconds and peak commit bytes of every process the app's tree has run."""
        basic, extended = Accounting(), Extended()
        checked(k.QueryInformationJobObject(self.job, 1, c.byref(basic), c.sizeof(basic), None))
        checked(k.QueryInformationJobObject(self.job, 9, c.byref(extended), c.sizeof(extended), None))
        return (basic.user + basic.kernel) / 1e7, extended.peak_job

    def confinement(self):
        """Processes currently in this app's Job object and their measured OS affinity masks."""
        plist = JobProcessIdList()
        checked(k.QueryInformationJobObject(self.job, 3, c.byref(plist), c.sizeof(plist), None))
        processes = []
        for index in range(plist.in_list):
            pid = int(plist.pids[index])
            handle = k.OpenProcess(0x1000, False, pid)
            if not handle:
                processes.append({"pid": pid, "exe": None, "affinity": None, "confined": False})
                continue
            try:
                proc_mask, sys_mask = c.c_size_t(), c.c_size_t()
                ok = bool(k.GetProcessAffinityMask(handle, c.byref(proc_mask), c.byref(sys_mask)))
                buf = c.create_unicode_buffer(1024)
                size = w.DWORD(1024)
                exe = Path(buf.value).name if k.QueryFullProcessImageNameW(handle, 0, buf, c.byref(size)) else None
                confined = ok and (self.affinity_mask is None or proc_mask.value == self.affinity_mask)
                processes.append({"pid": pid, "exe": exe, "affinity": f"{proc_mask.value:#x}" if ok else None, "confined": confined})
            finally:
                k.CloseHandle(handle)
        return {
            "mask": f"{self.affinity_mask:#x}" if self.affinity_mask else None,
            "jobProcesses": len(processes),
            "confinedProcesses": sum(1 for item in processes if item["confined"]),
            "allConfined": bool(processes) and all(item["confined"] for item in processes),
            "processes": processes,
        }

    def running(self):
        return k.WaitForSingleObject(self.info.process, 0) == STILL_ACTIVE

    def windows(self):
        found = []

        def collect(hwnd, _):
            pid = w.DWORD()
            u.GetWindowThreadProcessId(hwnd, c.byref(pid))
            if pid.value == self.info.pid and u.IsWindowVisible(hwnd):
                title = c.create_unicode_buffer(512)
                u.GetWindowTextW(hwnd, title, 512)
                found.append((hwnd, title.value))
            return True

        u.EnumDesktopWindows(self.desktop, ENUM(collect), 0)
        return found

    def main_window(self, seconds=120):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            for hwnd, title in self.windows():
                if title in APP_TITLES:
                    return hwnd
            if not self.running():
                raise RuntimeError(f"the app exited with {self.exit_code()} before showing its window")
            time.sleep(0.25)
        raise RuntimeError(f"the app showed no window in {seconds} s")

    def size(self, width, height):
        hwnd = self.main_window()
        u.ShowWindow(hwnd, 9)
        checked(u.SetWindowPos(hwnd, None, 0, 0, width, height, 0x14))
        rect = w.RECT()
        checked(u.GetWindowRect(hwnd, c.byref(rect)))
        if (rect.right - rect.left, rect.bottom - rect.top) != (width, height):
            raise RuntimeError(f"the window is {rect.right - rect.left}x{rect.bottom - rect.top}, not {width}x{height}")

    def client(self, width, height):
        """Sizes the window so its client area is `width` x `height` device pixels."""
        hwnd = self.main_window()
        u.ShowWindow(hwnd, 9)
        outer, inner = w.RECT(), w.RECT()
        checked(u.GetWindowRect(hwnd, c.byref(outer)))
        checked(u.GetClientRect(hwnd, c.byref(inner)))
        frame = (outer.right - outer.left - inner.right, outer.bottom - outer.top - inner.bottom)
        checked(u.SetWindowPos(hwnd, None, 0, 0, width + frame[0], height + frame[1], 0x14))
        checked(u.GetClientRect(hwnd, c.byref(inner)))
        if (inner.right, inner.bottom) != (width, height):
            raise RuntimeError(f"the window's client area is {inner.right}x{inner.bottom}, not {width}x{height}")

    def exit_code(self):
        code = w.DWORD()
        checked(k.GetExitCodeProcess(self.info.process, c.byref(code)))
        return code.value

    def close(self):
        """Closes the app as a user does; its exit code, or None when it did not exit in time and was stopped."""
        code = None
        try:
            if self.running():
                for hwnd, title in self.windows():
                    if title in APP_TITLES:
                        u.PostMessageW(hwnd, WM_CLOSE, 0, 0)
                if k.WaitForSingleObject(self.info.process, CLOSE_SECONDS * 1000) != STILL_ACTIVE:
                    code = self.exit_code()
            else:
                code = self.exit_code()
        finally:
            k.TerminateJobObject(self.job, 1)
            k.CloseHandle(self.info.thread)
            k.CloseHandle(self.info.process)
            k.CloseHandle(self.job)
            u.CloseDesktop(self.desktop)
            self.log.close()
            if self.noted:
                self.noted.unlink(missing_ok=True)
        return code


def created(process):
    """A process's creation time, which tells it from a later process given the same id."""
    times = [c.c_uint64() for _ in range(4)]
    checked(k.GetProcessTimes(process, *map(c.byref, times)))
    return times[0].value


def note_app(process, pid):
    """Notes an app this run started where other suite runs count it; the note, or None when it could not be written."""
    try:
        APPS.mkdir(parents=True, exist_ok=True)
        note = APPS / f"{pid}-{created(process)}.json"
        note.write_text(json.dumps({"pid": pid, "suite": os.getpid()}), encoding="utf-8")
        return note
    except OSError:
        return None


def alive(pid, creation):
    process = k.OpenProcess(0x1000, False, pid)
    if not process:
        return False
    try:
        code = w.DWORD()
        return bool(k.GetExitCodeProcess(process, c.byref(code)) and code.value == STILL_ACTIVE
                    and created(process) == creation)
    except OSError:
        return False
    finally:
        k.CloseHandle(process)


def running_apps(own):
    """BattlePlan apps running now: this run's, other suite runs' (by their notes) and every replay-desktop process
    on the machine (counted by name, only read)."""
    noted = 0
    for note in APPS.glob("*.json") if APPS.is_dir() else ():
        pid, _, creation = note.stem.partition("-")
        if pid.isdigit() and creation.isdigit() and alive(int(pid), int(creation)):
            noted += 1
    machine = None
    snapshot = k.CreateToolhelp32Snapshot(0x2, 0)
    if snapshot and snapshot != c.c_void_p(-1).value:
        try:
            machine = 0
            entry = ProcessEntry()
            entry.size = c.sizeof(entry)
            more = k.Process32FirstW(snapshot, c.byref(entry))
            while more:
                machine += entry.exe.lower() == APP_EXE
                more = k.Process32NextW(snapshot, c.byref(entry))
        finally:
            k.CloseHandle(snapshot)
    return {"thisRun": own, "otherRuns": max(0, noted - own), "machine": machine}


def machine_load(own, seconds=1.0):
    """The share of all logical CPUs busy over `seconds`, the share of memory in use, and the apps running."""
    def times():
        idle, kernel, user = c.c_uint64(), c.c_uint64(), c.c_uint64()
        checked(k.GetSystemTimes(c.byref(idle), c.byref(kernel), c.byref(user)))
        return idle.value, kernel.value + user.value
    idle, total = times()
    time.sleep(seconds)
    idle_after, total_after = times()
    memory = MemoryStatus()
    memory.length = c.sizeof(memory)
    checked(k.GlobalMemoryStatusEx(c.byref(memory)))
    busy = 1 - (idle_after - idle) / (total_after - total) if total_after > total else None
    return {"at": datetime.datetime.now().isoformat(timespec="seconds"),
            "cpuPercent": None if busy is None else round(100 * busy, 1), "memoryPercent": memory.load,
            "apps": running_apps(own)}


def shell_cargo_arguments(shell=True):
    """Single shared Cargo build definition for the desktop app and measuring shell."""
    features = ",".join(SHELL_FEATURES if shell else (SHELL_BASE_FEATURE,))
    return ["build", f"--{SHELL_PROFILE}", "-p", SHELL_PACKAGE, "--features", features]


def shell_artifact_name(revision):
    """GitHub Actions artifact name carrying the full commit SHA."""
    return f"{SHELL_ARTIFACT_PREFIX}{revision}"


def sha256_file(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _is_regular_file(path):
    return os.path.lexists(path) and not _is_link_or_reparse(path) and stat.S_ISREG(os.lstat(path).st_mode)


def _find_hosted_record(folder):
    candidate = Path(folder) / SHELL_RECORD
    return candidate if _is_regular_file(candidate) else None


def _is_hosted_shell_only(folder):
    folder = Path(folder)
    if _find_hosted_record(folder) is None:
        return False
    allowed = {SHELL_RECORD, APP_EXE, *SHADER_COMPILER}
    for entry in folder.iterdir():
        if entry.name not in allowed or not _is_regular_file(entry):
            return False
    return True


def _select_shell_record(folder, require_hosted=False):
    folder = Path(folder)
    names = (SHELL_RECORD,) if require_hosted else SHELL_RECORDS
    candidates = [folder / name for name in names if _is_regular_file(folder / name)]
    if not candidates:
        expected = SHELL_RECORD if require_hosted else f"{SHELL_RECORD} or {BOAT_RECORD}"
        raise RuntimeError(f"{folder} holds no shell build record ({expected}): fetch one with `suite.py shell`")
    if len(candidates) == 1:
        record = candidates[0]
        try:
            built = json.loads(record.read_text(encoding="utf-8"))
        except (OSError, ValueError) as error:
            raise RuntimeError(f"{record} is unreadable ({error}): fetch the shell again with `suite.py shell`")
        return record, built
    exe = folder / APP_EXE
    if not _is_regular_file(exe):
        raise RuntimeError(f"{folder} holds both {SHELL_RECORD} and {BOAT_RECORD} without {APP_EXE}")
    actual_sha = sha256_file(exe).lower()
    for record in candidates:
        try:
            built = json.loads(record.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            continue
        if not isinstance(built, dict):
            continue
        item = next((entry for entry in built.get("artifacts", [])
                     if isinstance(entry, dict) and Path(entry.get("path", "")).name == APP_EXE), None)
        if item and isinstance(item.get("sha256"), str) and item["sha256"].lower() == actual_sha:
            return record, built
    raise RuntimeError(f"{folder} holds both {SHELL_RECORD} and {BOAT_RECORD} and neither matches {APP_EXE} SHA-256 {actual_sha}")


def shell_build(folder, expected_revision=None, verify_hash=False, require_hosted=False, require_compiler=False):
    """What built a measuring shell, or why the folder holds none."""
    folder = Path(folder)
    record, built = _select_shell_record(folder, require_hosted=require_hosted)
    if not isinstance(built, dict):
        raise RuntimeError(f"{record} is not a JSON object")
    if require_hosted and built.get("source") != "github-actions":
        raise RuntimeError(f"{record} is not a hosted GitHub Actions measuring shell record")
    artifacts = built.get("artifacts", [])
    item = next((entry for entry in artifacts if isinstance(entry, dict) and Path(entry.get("path", "")).name == APP_EXE), None)
    arguments = list(item.get("cargo_arguments", [])) if item else []
    if not any(SHELL_FEATURE in argument for argument in arguments):
        raise RuntimeError(f"{folder}'s {APP_EXE} is not a measuring shell (built without {SHELL_FEATURE}): "
                           "fetch one with `suite.py shell`, or pass the folder as --app")
    revision = built.get("revision")
    if expected_revision is not None and revision != expected_revision:
        raise RuntimeError(f"{folder} is a measuring shell for {revision or '?'}, not {expected_revision}")
    if verify_hash:
        if f"--{SHELL_PROFILE}" not in arguments or SHELL_PACKAGE not in arguments:
            raise RuntimeError(f"{record} does not match the release {SHELL_PACKAGE} measuring shell build")
        exe = folder / APP_EXE
        if not _is_regular_file(exe):
            raise RuntimeError(f"{folder} holds no {APP_EXE}")
        expected_bytes = item.get("bytes")
        expected_sha = item.get("sha256")
        if not isinstance(expected_bytes, int) or isinstance(expected_bytes, bool) or not isinstance(expected_sha, str) or not expected_sha:
            raise RuntimeError(f"{record} has no size or SHA-256 for {APP_EXE}")
        actual_bytes = exe.stat().st_size
        if actual_bytes != expected_bytes:
            raise RuntimeError(f"{exe} is {actual_bytes} bytes, expected {expected_bytes}")
        actual_sha = sha256_file(exe)
        if actual_sha.lower() != expected_sha.lower():
            raise RuntimeError(f"{exe} SHA-256 is {actual_sha}, expected {expected_sha}")
    if require_compiler:
        missing = missing_shader_compiler(folder)
        if missing:
            raise RuntimeError(f"{folder} is missing the pinned shader compiler ({', '.join(missing)})")
    return {"folder": str(folder), "revision": revision, "cargoArguments": arguments}


def ui_bundle(folder):
    """A production bundle's entry page and size, or why the folder is not one (the shell refuses it too)."""
    entry = Path(folder) / "index.html"
    if not entry.is_file():
        raise RuntimeError(f"{folder} holds no index.html: give the folder `vite build` wrote (ui/dist)")
    if "/src/main.tsx" in entry.read_text(encoding="utf-8", errors="replace"):
        raise RuntimeError(f"{folder} is the UI's source, not a production bundle: give the folder `vite build` wrote")
    files = [path for path in Path(folder).rglob("*") if path.is_file()]
    return {"folder": str(folder), "built": datetime.datetime.fromtimestamp(entry.stat().st_mtime).isoformat(timespec="seconds"),
            "files": len(files), "bytes": sum(path.stat().st_size for path in files)}


def recorded_replies(path, slug, sessions):
    """The reply file of one session: the given file, or one named for the session when several run."""
    if sessions == 1:
        return path
    path = Path(path)
    return str(path.with_name(f"{path.stem}-{slug}{path.suffix}"))


def msvcrt_handle(stream):
    import msvcrt
    return msvcrt.get_osfhandle(stream.fileno())


def refresh_hz():
    """The display's refresh rate, which sets what a dropped frame is."""
    mode = DisplayMode()
    mode.size = c.sizeof(mode)
    checked(u.EnumDisplaySettingsW(None, 0xFFFFFFFF, c.byref(mode)))
    if mode.frequency <= 1:
        raise RuntimeError(f"the display reports no refresh rate ({mode.frequency})")
    return mode.frequency


def user_folder(given):
    """The Paradox user folder: given, else where the recorder finds it (the Documents shell folder, then
    OneDrive's and the profile's Documents)."""
    if given:
        return Path(given)
    import winreg
    candidates = []
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, r"Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders") as key:
            candidates.append(Path(os.path.expandvars(winreg.QueryValueEx(key, "Personal")[0])))
    except OSError:
        pass
    candidates += [Path(os.environ[name]) / "Documents" for name in ("OneDrive", "USERPROFILE") if os.environ.get(name)]
    for documents in candidates:
        folder = documents / "Paradox Interactive" / "Hearts of Iron IV"
        if folder.is_dir():
            return folder
    raise RuntimeError("no Paradox user folder in the Documents shell folder, OneDrive or the profile; pass --user-dir")


def free_port():
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


def node():
    found = shutil.which("node")
    if not found:
        raise RuntimeError("node is not on PATH")
    return found


def fixture_recording(folder):
    recordings = sorted(Path(folder).glob("*.battleplan"))
    if len(recordings) != 1:
        raise RuntimeError(f"{folder} holds {len(recordings)} recordings; the fixture is one")
    return recordings[0]


SHADER_COMPILER = ("dxcompiler.dll", "dxil.dll")


def missing_shader_compiler(folder):
    folder = Path(folder)
    return [name for name in SHADER_COMPILER if not _is_regular_file(folder / name)]


def _copy_atomic(src, dst, temp_dir=None):
    tmp = (dst.parent if temp_dir is None else Path(temp_dir)) / f".{dst.parent.name}.{dst.name}.{os.getpid()}-{time.time_ns()}.tmp"
    try:
        shutil.copy2(src, tmp)
        os.replace(tmp, dst)
    except BaseException:
        tmp.unlink(missing_ok=True)
        raise


def _stage_shader_pair(src_dir, dst_dir, temp_dir=None):
    for name in sorted(SHADER_COMPILER, key=lambda name: not _is_regular_file(dst_dir / name)):
        _copy_atomic(src_dir / name, dst_dir / name, temp_dir=temp_dir)


def shader_compiler(state, app, runner=subprocess.run):
    """The pinned shader compiler beside the app, whose maps load it from there: fetched once into the suite's state."""
    app = Path(app)
    if not missing_shader_compiler(app):
        return
    copy = Path(state) / "shader-compiler"
    if missing_shader_compiler(copy):
        copy.parent.mkdir(parents=True, exist_ok=True)
        staged = Path(tempfile.mkdtemp(prefix=".shader-compiler.staging-", dir=str(copy.parent)))
        try:
            done = runner(
                ["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
                 str(ROOT / "tools" / "install-shader-compiler.ps1"), "-Destination", str(staged)],
                capture_output=True, text=True, encoding="utf-8", errors="replace",
            )
            if done.returncode != 0:
                detail = (getattr(done, "stderr", None) or getattr(done, "stdout", None) or "").strip()
                raise RuntimeError(f"install-shader-compiler.ps1 failed (exit {done.returncode}): {detail}")
            missing = missing_shader_compiler(staged)
            if missing:
                raise RuntimeError(f"install-shader-compiler.ps1 did not produce {', '.join(missing)}")
            copy.mkdir(parents=True, exist_ok=True)
            _stage_shader_pair(staged, copy)
        finally:
            shutil.rmtree(staged, ignore_errors=True)
    _stage_shader_pair(copy, app, temp_dir=app.parent)
    missing = missing_shader_compiler(app)
    if missing:
        raise RuntimeError(f"{app} is missing the pinned shader compiler ({', '.join(missing)})")


def app_environment(profile, port, extra):
    environment = dict(os.environ)
    for key, folder in (("APPDATA", "roaming"), ("LOCALAPPDATA", "local"), ("WEBVIEW2_USER_DATA_FOLDER", "webview")):
        (profile / folder).mkdir(parents=True, exist_ok=True)
        environment[key] = str(profile / folder)
    environment["WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS"] = (
        f"--remote-debugging-port={port} --disable-backgrounding-occluded-windows "
        "--disable-renderer-backgrounding --disable-background-timer-throttling")
    environment.pop("BATTLEPLAN_PERF_LOG", None)
    environment.update(extra)
    environment["BATTLEPLAN_CACHE"] = str(profile / "recorder-cache")
    return environment


def launch(args, work, label, extra=None):
    """The app on the suite's fresh profile, with the user's own game and Paradox user folders, which it only reads."""
    profile = work / "profile"
    port = free_port()
    environment = app_environment(profile, port, dict(extra or {}))
    if args.ui:
        environment["BATTLEPLAN_UI_DIST"] = args.ui
    app_args = ["--game-dir", args.game_dir, "--user-dir", str(args.user_folder)]
    app = App(Path(args.app) / "replay-desktop.exe", app_args, environment, args.out / f"app-{label}.log", port,
              affinity_mask=args.cores["mask"] if args.cores else None)
    try:
        if args.window_client:
            app.client(*args.window_client)
        else:
            app.size(*WINDOW)
    except BaseException:
        app.close()
        raise
    return app


def cold_profile(work):
    """Empties every cache under the fresh profile; what is left is said."""
    profile = work / "profile"
    for cache in CACHES:
        deadline = time.monotonic() + 30
        while (profile / cache).exists():
            shutil.rmtree(profile / cache, ignore_errors=True)
            if (profile / cache).exists():
                if time.monotonic() > deadline:
                    raise RuntimeError(f"could not empty {profile / cache} within 30 s")
                time.sleep(1)
    left = sorted(str(path.relative_to(profile)).replace("\\", "/") for path in (profile / "local" / "BattlePlan").iterdir())
    expected = sorted(KEPT)
    if left != expected or any((profile / name).exists() for name in ("roaming", "webview", "local/Microsoft")):
        raise RuntimeError(f"the profile is not cold: it holds {left}")
    return {"emptied": list(CACHES), "kept": list(KEPT)}


def driver(script, log, *arguments, seconds):
    """Runs a Node driver into its log; its exit code, or None when it ran out of time."""
    with open(log, "wb") as output:
        process = subprocess.Popen([node(), str(HERE / script), *map(str, arguments)], cwd=ROOT, stdout=output,
                                   stderr=subprocess.STDOUT, creationflags=subprocess.CREATE_NO_WINDOW)
        try:
            return process.wait(timeout=seconds)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
            return None


def state_lock(state):
    """Holds `state` for this run: the copy stage empties its work folder, so two runs never share one."""
    import msvcrt
    state.mkdir(parents=True, exist_ok=True)
    lock = open(state / "run.lock", "a+b")
    try:
        msvcrt.locking(lock.fileno(), msvcrt.LK_NBLCK, 1)
    except OSError:
        lock.close()
        raise RuntimeError(f"another suite run is using --state {state}: give each run a state folder of its own")
    return lock


def run(args):
    started = time.monotonic()
    args.out = Path(args.out).resolve()
    args.out.mkdir(parents=True, exist_ok=True)
    state = Path(args.state).resolve()
    with state_lock(state):
        return run_locked(args, started, state)


def run_locked(args, started, state):
    work = state / "work"
    stages = args.stages.split(",")
    phases = {}
    record = {"commit": git_head(), "started": datetime.datetime.now().isoformat(timespec="seconds"), "phases": phases,
              "refreshHz": refresh_hz(), "tier": args.tier, "requested": [],
              "profile": args.profile, "cpuThrottle": args.cpu_throttle,
              "cores": {**args.cores, "mask": f"{args.cores['mask']:#x}"} if args.cores else None,
              "confinement": None,
              "windowClient": list(args.window_client) if args.window_client else None,
              "machine": {"logicalCpus": os.cpu_count(), "start": None, "end": None}}
    if args.ui:
        record["shell"] = {**shell_build(args.app), "ui": ui_bundle(args.ui)}
    history = Path(args.history).resolve() if args.history else state / "history.json"
    if not (Path(args.game_dir) / "hoi4.exe").is_file():
        raise RuntimeError(f"{args.game_dir} holds no hoi4.exe")
    if not (Path(args.app) / "replay-desktop.exe").is_file():
        raise RuntimeError(f"{args.app} holds no replay-desktop.exe")
    source = fixture_recording(args.fixture)
    args.user_folder = user_folder(args.user_dir)
    record["userFolder"] = str(args.user_folder)
    record["fixture"] = {"name": source.name, "bytes": source.stat().st_size}
    shader_compiler(state, Path(args.app))
    recording = work / "profile" / "local" / "BattlePlan" / "captures" / source.name
    if "copy" in stages:
        began = time.monotonic()
        if work.exists():
            shutil.rmtree(work)
        recording.parent.mkdir(parents=True)
        shutil.copy2(source, recording)
        phases["copy"] = time.monotonic() - began
    elif not recording.is_file():
        raise RuntimeError(f"no copied recording in {work}: run the copy stage first")
    record["fixture"]["sha256"] = "sha256:" + sha256_file(recording)
    record["selection"] = {"cases": args.cases, "pages": args.pages, "kinds": args.kinds}
    if "sweep" in stages:
        record["sessions"] = []
        wanted = args.phases.split(",") if args.phases else (["widget-costs"] if args.kinds else None)
        planned = []
        for name, session_phases, environment in (QUICK_SESSIONS if args.tier == "quick" else SESSIONS):
            chosen = [phase for phase in session_phases.split(",") if (wanted is None or phase in wanted)
                      and (phase != "pages-at" or args.pages_at is not None)
                      and (phase != "widget-costs" or (wanted is not None and "widget-costs" in wanted))]
            if chosen:
                planned.append((name, chosen, environment))
        for name, chosen, environment in planned:
            record["requested"] += chosen
            began = time.monotonic()
            session = {"name": name, "phases": chosen, "emptied": list(CACHES), "kept": list(KEPT)}
            slug = name.replace(" ", "-")
            input_address = f"127.0.0.1:{free_port()}"
            if args.record_replies:
                session["recordedReplies"] = recorded_replies(args.record_replies, slug, len(planned))
            try:
                cold_profile(work)
                session["machine"] = {"before": machine_load(0)}
                record["machine"]["start"] = record["machine"]["start"] or session["machine"]["before"]
                app = launch(args, work, f"sweep-{slug}", {"BATTLEPLAN_PERF_LOG": str(work / f"perf-{slug}.jsonl"),
                                                           "BATTLEPLAN_E2E_INPUT": input_address, "BATTLEPLAN_E2E_CAPTURE_CLOCK": "0",
                                                           **environment})
            except Exception as error:
                session.update(error=str(error), sweepExit=None, appExit=None, seconds=time.monotonic() - began)
                record["sessions"].append(session)
                continue
            try:
                session["sweepExit"] = driver("sweep.mjs", args.out / f"sweep-{slug}.log", "--port", app.port, "--out", args.out,
                                              "--work", work, "--session", slug, "--desktop", app.desktop_name, "--app-pid", app.info.pid,
                                              "--input", input_address,
                                              "--phases", ",".join(chosen), "--tier", args.tier, "--history", history, "--pin", args.pin,
                                              *(["--cpu-throttle", args.cpu_throttle] if args.cpu_throttle != 1.0 else []),
                                              *(["--cores-count", args.cores["count"]] if args.cores else []),
                                              *(["--cases", args.cases] if args.cases else []),
                                              *(["--pages", args.pages] if args.pages else []),
                                              *(["--pages-at", args.pages_at] if args.pages_at is not None else []),
                                              *(["--kinds", args.kinds] if args.kinds else []),
                                              *(["--all-views"] if args.all_views else []),
                                              *(["--costs", args.costs] if args.costs else []),
                                              *(["--heaviest", args.heaviest] if args.heaviest is not None else []),
                                              *(["--record-replies", session["recordedReplies"]] if args.record_replies else []),
                                              seconds=3 * 3600)
            finally:
                try:
                    if args.cores and app.running():
                        session["confinement"] = record["confinement"] = app.confinement()
                    session["machine"]["after"] = record["machine"]["end"] = machine_load(int(app.running()))
                finally:
                    session["sessionPeakCommitBytes"] = app.usage()[1]
                    session["appExit"] = app.close()
            session["seconds"] = time.monotonic() - began
            record["sessions"].append(session)
            phases[f"sweep: {name}"] = session["seconds"]
    record["seconds"] = time.monotonic() - started
    (args.out / "run.json").write_text(json.dumps(record, indent=1), encoding="utf-8")
    if "report" not in stages:
        return 0
    arguments = ["--run", args.out / "run.json", "--sweeps", args.out, "--perf", work, "--out", args.out,
                 "--baseline", state / "visual-baseline", "--history", history, "--perf-baseline", state / "perf-baseline.json"]
    code = driver("report.mjs", args.out / "report.log", *arguments, seconds=1800)
    if code not in (0, 1):
        raise RuntimeError(f"report.mjs failed (exit {code}); see {args.out / 'report.log'}")
    return code


def git_head():
    try:
        return subprocess.run(["git", "-C", str(ROOT), "rev-parse", "HEAD"], capture_output=True, text=True, check=True).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        return None


def accept(args):
    """Makes a run's captures the visual baseline, recording which run and change it accepted and why."""
    run_folder, state = Path(args.run).resolve(), Path(args.state).resolve()
    if not args.reason.strip():
        raise RuntimeError("acceptance needs a nonblank reason")
    if getattr(args, "counts", None):
        with state_lock(state):
            return accept_counts(args, run_folder, state)
    captures = run_folder / "visual"
    manifest = json.loads((captures / "manifest.json").read_text(encoding="utf-8"))
    if not manifest["complete"]:
        failed = [item["caseId"] for item in manifest["items"] if item["status"] != "captured"]
        raise RuntimeError(f"{len(failed)} captures failed, so this run cannot be a baseline: {', '.join(failed[:10])}")
    run = json.loads((run_folder / "run.json").read_text(encoding="utf-8"))
    baseline = state / "visual-baseline"
    staged = state / "visual-baseline.new"
    if staged.exists():
        shutil.rmtree(staged)
    shutil.copytree(captures, staged, ignore=shutil.ignore_patterns("diff"))
    (staged / "accepted.json").write_text(json.dumps({
        "acceptedAt": datetime.datetime.now().isoformat(timespec="seconds"), "commit": run.get("commit"),
        "run": str(run_folder), "reason": args.reason, "items": len(manifest["items"]), "fixture": run.get("fixture"),
    }, indent=1), encoding="utf-8")
    if baseline.exists():
        shutil.rmtree(baseline)
    staged.rename(baseline)
    print(f"Accepted {len(manifest['items'])} captures of {run_folder} ({(run.get('commit') or '?')[:10]}) as the visual baseline: {args.reason}")
    return 0


def accept_counts(args, folder, state):
    run = json.loads((folder / "run.json").read_text(encoding="utf-8"))
    measured = json.loads((folder / "counts.json").read_text(encoding="utf-8"))
    report = json.loads((folder / "e2e.json").read_text(encoding="utf-8"))
    if measured["run"] != run or report.get("run") != run or not run.get("fixture", {}).get("sha256"):
        raise RuntimeError("count scenarios do not match this run")
    if any(not row["pass"] for section in report["sections"] if section["title"] != "Accepted request counts" for row in section["rows"]):
        raise RuntimeError("a failed action or incomplete measurement cannot be accepted")
    entries = measured["entries"]
    if len({entry["key"] for entry in entries}) != len(entries):
        raise RuntimeError("duplicate count keys")
    chosen = [entry for entry in entries if entry["key"] in args.counts]
    for entry in chosen:
        identity = json.dumps([entry["recording"], entry["scenario"], entry["interaction"]], ensure_ascii=False, separators=(",", ":"))
        if entry["key"] != hashlib.sha256(identity.encode("utf-8")).hexdigest():
            raise RuntimeError("selected count scenario does not match its entry key")
    if len(chosen) != len(set(args.counts)) or any(not entry["complete"] or entry["recording"] != run["fixture"]["sha256"] or not entry["scenario"] for entry in chosen):
        raise RuntimeError("selected counts are unmeasured or have mismatched scenarios")
    baseline = state / "perf-baseline.json"
    previous = json.loads(baseline.read_text(encoding="utf-8")) if baseline.exists() else {"version": 1, "entries": []}
    if previous["version"] != 1 or len({(e["recording"], e["scenario"], e["interaction"]) for e in previous["entries"]}) != len(previous["entries"]):
        raise RuntimeError("invalid baseline version or duplicate count keys")
    selected = {(e["recording"], e["scenario"], e["interaction"]) for e in chosen}
    kept = [e for e in previous["entries"] if (e["recording"], e["scenario"], e["interaction"]) not in selected]
    for entry in chosen:
        if any(not isinstance(value, int) or value < 0 for value in entry["counts"].values()):
            raise RuntimeError("invalid request count")
        kept.append({**entry, "counts": {name: {"accepted": value, "tolerance": {"absolute": 2 if value else 0, "relative": .2 if value else 0}} for name, value in entry["counts"].items()},
                     "acceptedAt": datetime.datetime.now().isoformat(timespec="seconds"), "commit": run.get("commit"), "run": str(folder), "reason": args.reason})
    if baseline.exists():
        archive = state / "perf-archive"
        archive.mkdir(exist_ok=True)
        shutil.copy2(baseline, archive / f"{uuid.uuid4()}.json")
    staged = state / f"perf-baseline.{uuid.uuid4()}.new"
    try:
        staged.write_text(json.dumps({"version": 1, "entries": kept}, indent=1), encoding="utf-8")
        os.replace(staged, baseline)
    finally:
        staged.unlink(missing_ok=True)
    print(f"Accepted {len(chosen)} count entries: {args.reason}")
    return 0


def run_build_boat(out, shell=False, runner=subprocess.run, env=None):
    """The release app with the UI embedded, or the measuring shell, compiled on Boat; a Boat failure is retried once,
    as Boat asks."""
    if shell:
        print("note: `python tools/e2e/suite.py shell` fetches or dispatches the hosted measuring shell without Boat", file=sys.stderr)
    cargo_args = shell_cargo_arguments(shell=shell)
    out_dir = Path(out).resolve()
    command = [sys.executable, str(ROOT / "tools" / "boat.py"), "build", "--out", str(out_dir), *cargo_args[1:]]
    for attempt in (1, 2):
        code = runner(command, cwd=ROOT).returncode
        if code != BOAT_FAILED:
            if code == 0 and shell:
                shader_compiler(_default_state(env), out_dir, runner=runner)
            return code
        print(f"Boat failed (exit {code}), attempt {attempt}", file=sys.stderr)
    return code


def build(args):
    return run_build_boat(args.out, shell=args.shell)


def parse_github_repo(url):
    match = GITHUB_REMOTE.match(url.strip())
    return match.group(1) if match else None


def github_repo(given=None, env=None, runner=subprocess.run):
    if given:
        return given.strip()
    source_env = os.environ if env is None else env
    if source_env.get("GITHUB_REPOSITORY"):
        return source_env["GITHUB_REPOSITORY"].strip()
    done = runner(["git", "-C", str(ROOT), "remote", "get-url", "origin"], capture_output=True, text=True, encoding="utf-8", errors="replace")
    if done.returncode == 0:
        parsed = parse_github_repo(done.stdout)
        if parsed:
            return parsed
    raise RuntimeError("could not determine the GitHub repository from origin; pass --repo owner/name")


def resolve_commit(ref, root=ROOT, runner=subprocess.run):
    text = (ref or "HEAD").strip()
    if not text:
        text = "HEAD"
    done = runner(["git", "-C", str(root), "rev-parse", "--verify", f"{text}^{{commit}}"], capture_output=True, text=True, encoding="utf-8", errors="replace")
    if done.returncode == 0:
        sha = done.stdout.strip().lower()
        if FULL_SHA.match(sha):
            return sha
    if FULL_SHA.match(text.lower()):
        return text.lower()
    detail = (done.stderr or done.stdout or "").strip()
    raise RuntimeError(f"could not resolve commit {ref!r}: {detail or 'unknown revision'}")


def commit_on_main(revision, runner=subprocess.run):
    for ref in ("origin/main", "main"):
        check = runner(["git", "-C", str(ROOT), "rev-parse", "--verify", "--quiet", f"{ref}^{{commit}}"], capture_output=True, text=True, encoding="utf-8", errors="replace")
        if check.returncode == 0:
            anc = runner(["git", "-C", str(ROOT), "merge-base", "--is-ancestor", revision, ref], capture_output=True, text=True, encoding="utf-8", errors="replace")
            return anc.returncode == 0
    return False


def shell_destination(revision, out=None, gate_dir=None, env=None, runner=subprocess.run):
    if out:
        return Path(os.path.abspath(str(out)))
    source_env = os.environ if env is None else env
    gate = gate_dir or source_env.get(GATE_DIR_ENV)
    if not gate:
        raise RuntimeError(f"pass --out <dir>, --gate-dir <dir>, or set {GATE_DIR_ENV}")
    if not commit_on_main(revision, runner=runner):
        raise RuntimeError(
            f"{revision[:10]} is not on origin/main: pass --out <dir> for a branch shell "
            "(shared-shell holds only main commits; run `git fetch origin` if it was just merged)"
        )
    return Path(os.path.abspath(str(gate))) / "shared-shell" / revision[:10]


def validate_shell_target(out):
    """Refuses repo paths, links/junctions, and non-empty directories without a hosted shell build record."""
    raw = Path(os.path.abspath(str(out)))
    for candidate in (raw, *raw.parents):
        if os.path.lexists(candidate) and _is_link_or_reparse(candidate):
            raise RuntimeError(f"refusing {raw}: {candidate} is a symlink or junction")
    resolved_root = Path(os.path.realpath(str(ROOT)))
    resolved_raw = Path(os.path.realpath(str(raw)))
    for target, root in ((raw, ROOT), (resolved_raw, resolved_root)):
        if target == root or root in target.parents:
            raise RuntimeError("shell destination must be outside the repository")
    if os.path.lexists(raw):
        st = os.lstat(raw)
        if not stat.S_ISDIR(st.st_mode):
            raise RuntimeError(f"refusing to overwrite {raw}: path exists and is not a directory")
        entries = list(raw.iterdir())
        if entries and not _is_hosted_shell_only(raw):
            raise RuntimeError(
                f"refusing to overwrite {raw}: directory holds entries outside a dedicated hosted measuring shell "
                f"({SHELL_RECORD}, {APP_EXE}, {', '.join(SHADER_COMPILER)}); pass a new, empty, or dedicated shell folder"
            )
    return raw


def _gh_api(endpoint, runner=subprocess.run):
    done = runner(["gh", "api", endpoint], capture_output=True, text=True, encoding="utf-8", errors="replace")
    if done.returncode != 0:
        detail = (done.stderr or done.stdout or "").strip()
        raise RuntimeError(f"gh api {endpoint} failed (exit {done.returncode}): {detail}")
    try:
        return json.loads(done.stdout)
    except ValueError as error:
        raise RuntimeError(f"gh api {endpoint} returned invalid JSON: {error}")


def find_hosted_shell_artifact(repo, revision, runner=subprocess.run):
    """The newest successful hosted run id carrying the measuring shell artifact for `revision`, or (None, reason)."""
    name = shell_artifact_name(revision)
    artifacts = _gh_api(f"repos/{repo}/actions/artifacts?name={name}&per_page=100", runner=runner).get("artifacts", [])
    if not artifacts:
        return None, f"no hosted measuring shell artifact {name} exists yet"
    reasons = []
    for artifact in sorted(artifacts, key=lambda entry: entry.get("created_at") or "", reverse=True):
        if artifact.get("expired"):
            reasons.append("expired")
            continue
        origin = artifact.get("workflow_run") or {}
        run_id = origin.get("id")
        if not run_id or origin.get("head_repository_id") != origin.get("repository_id"):
            reasons.append("not built from this repository")
            continue
        state = _gh_api(f"repos/{repo}/actions/runs/{run_id}", runner=runner)
        if state.get("path") != f".github/workflows/{SHELL_WORKFLOW}":
            reasons.append(f"built by {state.get('path')}, not {SHELL_WORKFLOW}")
            continue
        if state.get("status") != "completed":
            reasons.append(f"run {run_id} is still {state.get('status')}")
            continue
        if state.get("conclusion") != "success":
            reasons.append(f"run {run_id} ended {state.get('conclusion')}")
            continue
        return run_id, None
    if set(reasons) == {"expired"}:
        return None, f"hosted measuring shell artifact {name} has expired"
    return None, f"no usable measuring shell artifact {name}: {reasons[0]}"


def find_active_shell_run(repo, revision, runner=subprocess.run):
    done = runner(
        ["gh", "api", f"repos/{repo}/actions/workflows/{SHELL_WORKFLOW}/runs?per_page=50"],
        capture_output=True, text=True, encoding="utf-8", errors="replace",
    )
    if done.returncode != 0:
        return None
    try:
        runs = json.loads(done.stdout).get("workflow_runs", [])
    except ValueError:
        return None
    expected_title = f"{SHELL_RUN_NAME_PREFIX}{revision}"
    for entry in runs:
        if entry.get("status") == "completed":
            continue
        if entry.get("display_title") == expected_title:
            return entry
    return None


def format_shell_command(revision, out=None, gate_dir=None, repo=None, dispatch=False):
    parts = ["python", "tools/e2e/suite.py", "shell", "--commit", revision]
    if out:
        parts += ["--out", str(out)]
    elif gate_dir:
        parts += ["--gate-dir", str(gate_dir)]
    if repo:
        parts += ["--repo", repo]
    if dispatch:
        parts.append("--dispatch")
    return subprocess.list2cmdline(parts)


def _install_staged_shell(staging, target, revision):
    validate_shell_target(target)
    backup = None
    if os.path.lexists(target):
        backup = target.parent / f".{target.name}.old-{os.getpid()}-{time.time_ns()}"
        target.rename(backup)
    try:
        staging.rename(target)
    except OSError:
        if backup is not None and os.path.lexists(backup) and not os.path.lexists(target):
            backup.rename(target)
        elif backup is None and target.is_dir():
            try:
                shell_build(target, expected_revision=revision, verify_hash=True, require_hosted=True, require_compiler=True)
                shutil.rmtree(staging, ignore_errors=True)
                return
            except RuntimeError:
                pass
        raise
    if backup is not None:
        if _is_hosted_shell_only(backup):
            shutil.rmtree(backup, ignore_errors=True)
        else:
            try:
                backup.rmdir()
            except OSError as error:
                kept = sorted(entry.name for entry in backup.iterdir()) if backup.is_dir() else []
                raise RuntimeError(
                    f"refusing to delete backup {backup}: unexpected entries appeared before swap "
                    f"({', '.join(kept) or error}); fetched shell is installed at {target}"
                ) from error


def _current_head(runner=subprocess.run):
    done = runner(["git", "-C", str(ROOT), "rev-parse", "--verify", "--quiet", "HEAD^{commit}"], capture_output=True, text=True, encoding="utf-8", errors="replace")
    if done.returncode == 0:
        sha = done.stdout.strip().lower()
        if FULL_SHA.match(sha):
            return sha
    return None


def run_fetch_shell(commit="HEAD", out=None, gate_dir=None, dispatch=False, repo=None, env=None, runner=subprocess.run):
    """Fetches and verifies the hosted measuring shell for `commit`, or dispatches the workflow when absent."""
    revision = resolve_commit(commit, runner=runner)
    target = validate_shell_target(shell_destination(revision, out=out, gate_dir=gate_dir, env=env, runner=runner))
    cache_state = _default_state(env)
    if target.is_dir() and _find_hosted_record(target) is not None:
        try:
            shell_build(target, expected_revision=revision, verify_hash=True, require_hosted=True)
        except RuntimeError:
            pass
        else:
            shader_compiler(cache_state, target, runner=runner)
            print(f"Measuring shell ready at {target} ({revision[:10]})")
            return 0
    resolved_repo = github_repo(repo, env=env, runner=runner)
    artifact_name = shell_artifact_name(revision)
    run_id, why = find_hosted_shell_artifact(resolved_repo, revision, runner=runner)
    if run_id is not None:
        target.parent.mkdir(parents=True, exist_ok=True)
        staging = Path(tempfile.mkdtemp(prefix=f".{target.name}.staging-", dir=str(target.parent)))
        try:
            done = runner(
                ["gh", "run", "download", str(run_id), "--repo", resolved_repo, "--name", artifact_name, "--dir", str(staging)],
                capture_output=True, text=True, encoding="utf-8", errors="replace",
            )
            if done.returncode != 0:
                detail = (done.stderr or done.stdout or "").strip()
                raise RuntimeError(f"gh run download failed (exit {done.returncode}): {detail}")
            shell_build(staging, expected_revision=revision, verify_hash=True, require_hosted=True)
            shader_compiler(cache_state, staging, runner=runner)
            _install_staged_shell(staging, target, revision)
            print(f"Fetched measuring shell for {revision[:10]} into {target} (run {run_id})")
            return 0
        except BaseException:
            if staging.exists():
                shutil.rmtree(staging, ignore_errors=True)
            raise
    fetch_cmd = format_shell_command(revision, out=out, gate_dir=gate_dir, repo=repo, dispatch=False)
    active = find_active_shell_run(resolved_repo, revision, runner=runner)
    if active is not None:
        where = active.get("html_url") or f"run {active.get('id')}"
        print(
            f"Measuring shell build for {revision[:10]} is already {active.get('status')}: {where}\n"
            f"When it finishes, fetch it with:\n  {fetch_cmd}",
            file=sys.stderr,
        )
        return 1
    if dispatch:
        pushed = runner(
            ["gh", "api", f"repos/{resolved_repo}/commits/{revision}"],
            capture_output=True, text=True, encoding="utf-8", errors="replace",
        )
        if pushed.returncode != 0:
            detail = (pushed.stderr or pushed.stdout or "").strip()
            raise RuntimeError(f"commit {revision} is not on GitHub ({resolved_repo}): push your branch before dispatching ({detail})")
        dispatched = runner(
            ["gh", "workflow", "run", SHELL_WORKFLOW, "--repo", resolved_repo, "-f", f"ref={revision}"],
            capture_output=True, text=True, encoding="utf-8", errors="replace",
        )
        if dispatched.returncode != 0:
            detail = (dispatched.stderr or dispatched.stdout or "").strip()
            raise RuntimeError(f"gh workflow run {SHELL_WORKFLOW} failed (exit {dispatched.returncode}): {detail}")
        print(
            f"Dispatched {SHELL_WORKFLOW} for {revision}.\n"
            f"Track progress: gh run list --repo {resolved_repo} --workflow {SHELL_WORKFLOW}\n"
            f"When it finishes, fetch it with:\n  {fetch_cmd}",
            file=sys.stderr,
        )
        return 1
    dispatch_cmd = format_shell_command(revision, out=out, gate_dir=gate_dir, repo=repo, dispatch=True)
    message = f"{why}.\nStart a hosted build with:\n  {dispatch_cmd}"
    if _current_head(runner=runner) == revision:
        message += "\nOr compile this checkout on Boat into a separate folder with:\n  python tools/e2e/suite.py build --shell --out <separate-dir>"
    print(message, file=sys.stderr)
    return 1


def _load_ci_module():
    tools_dir = str(ROOT / "tools")
    if tools_dir not in sys.path:
        sys.path.insert(0, tools_dir)
    import ci
    return ci


def run_build_hosted_shell(out, target=None, source_root=None, env=None, runner=subprocess.run):
    """Builds and packages the measuring shell on GitHub Actions using the shared Cargo definition."""
    source_env = os.environ if env is None else env
    if source_env.get("GITHUB_ACTIONS") != "true":
        raise RuntimeError("build-hosted-shell runs only on GitHub Actions (GITHUB_ACTIONS=true)")
    ci = _load_ci_module()
    work_root = Path(source_root).resolve() if source_root else ROOT
    revision = resolve_commit("HEAD", root=work_root, runner=runner)
    target_dir = Path(target).resolve() if target else (work_root / ".cargo-target")
    out_dir = Path(out).resolve()
    out_dir.mkdir(parents=True, exist_ok=True)
    if target_dir.is_dir():
        for entry in list(target_dir.iterdir()):
            if entry.is_dir() and entry.name != SHELL_PROFILE and ((entry / ".fingerprint").is_dir() or entry.name == "shader-compiler"):
                shutil.rmtree(entry, ignore_errors=True)
    ui_dist = work_root / "ui" / "dist"
    created_ui_dist = False
    if not ui_dist.exists():
        ui_dist.mkdir(parents=True, exist_ok=True)
        placeholder = ui_dist / "index.html"
        placeholder.write_text(SHELL_PLACEHOLDER_HTML, encoding="utf-8")
        os.utime(placeholder, (ci.SETTLED, ci.SETTLED))
        os.utime(ui_dist, (ci.SETTLED, ci.SETTLED))
        created_ui_dist = True
    started = time.monotonic()
    cargo_args = shell_cargo_arguments(shell=True)
    command = ["cargo", *cargo_args, "--message-format=json-render-diagnostics"]
    cargo_env = dict(source_env)
    cargo_env["CARGO_TARGET_DIR"] = str(target_dir)
    try:
        done = runner(command, cwd=work_root, env=cargo_env, stdout=subprocess.PIPE, text=True, encoding="utf-8", errors="replace")
    finally:
        if created_ui_dist:
            shutil.rmtree(ui_dist, ignore_errors=True)
    built_exe = None
    compiled_units, fresh_units = 0, 0
    for line in (done.stdout or "").splitlines():
        try:
            event = json.loads(line)
        except ValueError:
            continue
        if event.get("reason") == "compiler-message":
            rendered = event.get("message", {}).get("rendered")
            if rendered:
                print(rendered, end="", file=sys.stderr, flush=True)
        elif event.get("reason") == "compiler-artifact":
            if event.get("fresh"):
                fresh_units += 1
            else:
                compiled_units += 1
            executable = event.get("executable")
            if executable and Path(executable).name == APP_EXE:
                built_exe = Path(executable)
    if done.returncode != 0:
        return done.returncode
    if built_exe is None or not built_exe.is_file():
        built_exe = target_dir / SHELL_PROFILE / APP_EXE
    if not built_exe.is_file():
        raise RuntimeError(f"Cargo finished without producing {built_exe}")
    staged_exe = out_dir / APP_EXE
    shutil.copy2(built_exe, staged_exe)
    digest = sha256_file(staged_exe)
    size = staged_exe.stat().st_size
    run_id_text = source_env.get("GITHUB_RUN_ID")
    run_id = int(run_id_text) if run_id_text and run_id_text.isdigit() else None
    record = {
        "source": "github-actions",
        "revision": revision,
        "package": SHELL_PACKAGE,
        "profile": SHELL_PROFILE,
        "features": list(SHELL_FEATURES),
        "cargo_arguments": cargo_args,
        "target": SHELL_TARGET,
        "workflow": SHELL_WORKFLOW,
        "run_id": run_id,
        "built_at": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "artifacts": [{
            "path": APP_EXE,
            "sha256": digest,
            "bytes": size,
            "cargo_arguments": cargo_args,
        }],
    }
    (out_dir / SHELL_RECORD).write_text(json.dumps(record, indent=2) + "\n", encoding="utf-8")
    shell_build(out_dir, expected_revision=revision, verify_hash=True, require_hosted=True)
    live = ci.live_units(done.stdout or "")
    profile_dir = target_dir / SHELL_PROFILE
    pruned = ci.prune(profile_dir, live) if (profile_dir / ".fingerprint").is_dir() else 0
    if target_dir.is_dir():
        (target_dir / ci.BUILT_FROM).write_text(revision + "\n", encoding="utf-8")
    elapsed = time.monotonic() - started
    print(
        f"Built measuring shell for {revision[:10]} in {elapsed:.1f}s "
        f"({compiled_units} compiled, {fresh_units} fresh units; pruned {pruned} unused entries)",
        flush=True,
    )
    return 0


UI_BUNDLE_MARKER = ".battleplan-ui-bundle"
STOCK_VITE_CONFIG = ROOT / "ui" / "vite.config.ts"
_FILE_ATTRIBUTE_REPARSE_POINT = 0x0400


def _is_link_or_reparse(path):
    try:
        st = os.lstat(path)
    except OSError:
        return False
    if stat.S_ISLNK(st.st_mode):
        return True
    if getattr(st, "st_file_attributes", 0) & _FILE_ATTRIBUTE_REPARSE_POINT:
        return True
    isjunction = getattr(os.path, "isjunction", None)
    return bool(isjunction and isjunction(path))


def validate_ui_output(out):
    """Refuses repo paths, links/junctions, and non-empty directories without the build-ui marker."""
    raw = Path(os.path.abspath(str(out)))
    for candidate in (raw, *raw.parents):
        if os.path.lexists(candidate) and _is_link_or_reparse(candidate):
            raise RuntimeError(f"refusing {raw}: {candidate} is a symlink or junction")
    resolved_root = Path(os.path.realpath(str(ROOT)))
    resolved_raw = Path(os.path.realpath(str(raw)))
    for target, root in ((raw, ROOT), (resolved_raw, resolved_root)):
        if target == root or root in target.parents:
            raise RuntimeError("--out for build-ui must be outside the repository")
    if os.path.lexists(raw):
        st = os.lstat(raw)
        if not stat.S_ISDIR(st.st_mode):
            raise RuntimeError(f"refusing to overwrite {raw}: path exists and is not a directory")
        marker = raw / UI_BUNDLE_MARKER
        if os.path.lexists(marker):
            if _is_link_or_reparse(marker) or not stat.S_ISREG(os.lstat(marker).st_mode):
                raise RuntimeError(f"refusing to overwrite {raw}: {UI_BUNDLE_MARKER} is not a regular file")
        elif any(raw.iterdir()):
            raise RuntimeError(
                f"refusing to overwrite {raw}: directory is not empty and does not hold {UI_BUNDLE_MARKER} "
                "(pass a new or empty folder, or one previously created by `suite.py build-ui`)"
            )
    return raw


def build_ui_command(out):
    return ["npx.cmd", "vite", "build", "--config", str(STOCK_VITE_CONFIG), "--outDir", str(out), "--emptyOutDir", "false"]


def _is_marked_bundle_dir(path):
    marker = path / UI_BUNDLE_MARKER
    return os.path.lexists(marker) and not _is_link_or_reparse(marker) and stat.S_ISREG(os.lstat(marker).st_mode)


def find_ui_leftovers(target):
    parent = target.parent
    if not parent.is_dir():
        return []
    old = sorted(parent.glob(f".{target.name}.old-*"))
    staging = sorted(parent.glob(f".{target.name}.staging-*"))
    return [*old, *staging]


def run_build_ui(out, runner=subprocess.run):
    """Builds into a fresh staging folder beside `--out`, then swaps `--out` and writes the marker only on success."""
    target = validate_ui_output(out)
    leftovers = find_ui_leftovers(target)
    if leftovers and not os.path.lexists(target):
        names = ", ".join(str(p) for p in leftovers)
        print(f"note: found recoverable leftover bundle(s) for missing {target}: {names}", file=sys.stderr)
    target.parent.mkdir(parents=True, exist_ok=True)
    staging = Path(tempfile.mkdtemp(prefix=f".{target.name}.staging-", dir=str(target.parent)))
    env = dict(os.environ)
    try:
        code = runner(build_ui_command(staging), cwd=ROOT, env=env).returncode
        if code != 0:
            shutil.rmtree(staging, ignore_errors=True)
            return code
        (staging / UI_BUNDLE_MARKER).write_text("built by tools/e2e/suite.py build-ui\n", encoding="utf-8")
        validate_ui_output(target)
        backup = None
        if os.path.lexists(target):
            backup = target.parent / f".{target.name}.old-{os.getpid()}-{time.time_ns()}"
            target.rename(backup)
        try:
            staging.rename(target)
        except OSError:
            if backup is not None and os.path.lexists(backup) and not os.path.lexists(target):
                backup.rename(target)
            raise
        if backup is not None:
            if _is_marked_bundle_dir(backup):
                shutil.rmtree(backup, ignore_errors=True)
            else:
                try:
                    backup.rmdir()
                except OSError as error:
                    kept = sorted(entry.name for entry in backup.iterdir()) if backup.is_dir() else []
                    raise RuntimeError(
                        f"refusing to delete unmarked backup {backup}: unexpected entries appeared before swap "
                        f"({', '.join(kept) or error}); new bundle is installed at {target}"
                    ) from error
        return 0
    except BaseException:
        if staging.exists():
            shutil.rmtree(staging, ignore_errors=True)
        raise


def build_ui(args):
    """Builds a production UI bundle into `--out`."""
    return run_build_ui(args.out)


def parse(argv):
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    commands = parser.add_subparsers(dest="command", required=True)
    shell_command = commands.add_parser("shell", help="fetch a hosted measuring shell for a commit, or dispatch its workflow")
    shell_command.add_argument("--commit", default="HEAD", help="commit SHA or ref to obtain the measuring shell for (default: HEAD)")
    shell_dest = shell_command.add_mutually_exclusive_group()
    shell_dest.add_argument("--out", help="destination folder for the measuring shell")
    shell_dest.add_argument("--gate-dir", help=f"gate folder holding shared-shell/<main sha> (defaults to {GATE_DIR_ENV})")
    shell_command.add_argument("--dispatch", action="store_true", help="dispatch the hosted workflow if no artifact exists yet")
    shell_command.add_argument("--repo", help="GitHub repository as owner/name (defaults to GITHUB_REPOSITORY or origin)")
    hosted_command = commands.add_parser("build-hosted-shell", help="build and package the measuring shell on GitHub Actions")
    hosted_command.add_argument("--out", required=True, help="folder to stage replay-desktop.exe and shell-build.json")
    hosted_command.add_argument("--target", help="Cargo target directory (default: .cargo-target)")
    hosted_command.add_argument("--source-root", help="source checkout to build (default: this script's repository root)")
    build_command = commands.add_parser("build", help="compile replay-desktop on Boat")
    build_command.add_argument("--out", required=True)
    build_command.add_argument("--shell", action="store_true",
                               help="compile the measuring shell on Boat (prefer `suite.py shell` for the hosted build)")
    build_ui_command = commands.add_parser("build-ui", help="build a production UI bundle for the measuring shell")
    build_ui_command.add_argument("--out", required=True)
    accept_command = commands.add_parser("accept", help="make a run's captures the visual baseline")
    accept_command.add_argument("--run", required=True, help="the run's report folder")
    accept_command.add_argument("--reason", required=True, help="the change this baseline accepts")
    accept_command.add_argument("--state", default=str(DEFAULT_STATE))
    accept_command.add_argument("--counts", action="append", help="explicit count entry key from counts.json; repeat to select more")
    run_command = commands.add_parser("run")
    app = run_command.add_mutually_exclusive_group(required=True)
    app.add_argument("--app", help="folder holding replay-desktop.exe; the pinned shader compiler is put beside it")
    app.add_argument("--shell", help="folder holding the measuring shell (`suite.py shell` or `build --shell`); needs --ui")
    run_command.add_argument("--ui", help="the production bundle the shell serves (`vite build` output, ui/dist); builds nothing")
    run_command.add_argument("--record-replies", help="file the sweep records every reply and followed message the page receives to (tools/e2e/replies.mjs)")
    run_command.add_argument("--fixture", default=os.environ.get("BATTLEPLAN_E2E_FIXTURE", os.environ.get("HOI4_E2E_FIXTURE")), required=not os.environ.get("BATTLEPLAN_E2E_FIXTURE", os.environ.get("HOI4_E2E_FIXTURE")))
    run_command.add_argument("--game-dir", default=os.environ.get("BATTLEPLAN_GAME_DIR", os.environ.get("HOI4_GAME_DIR")), required=not os.environ.get("BATTLEPLAN_GAME_DIR", os.environ.get("HOI4_GAME_DIR")))
    run_command.add_argument("--user-dir", default=os.environ.get("BATTLEPLAN_USER_DIR", os.environ.get("HOI4_USER_DIR")), help="the Paradox user folder, else found as the recorder finds it")
    run_command.add_argument("--out", required=True)
    run_command.add_argument("--state", default=str(DEFAULT_STATE), help="baseline history, shader compiler and the last run's work")
    run_command.add_argument("--tier", choices=("full", "quick"), default="full", help="quick: one session for iterating, every page once")
    run_command.add_argument("--history", help="every step that ever failed (JSON), read and extended by each run; <state>/history.json by default")
    run_command.add_argument("--pin", choices=("all", "product"), default="all",
                             help="which earlier failures keep a case from being left out as a repeat: all, or only the product's")
    run_command.add_argument("--stages", default=",".join(STAGES))
    run_command.add_argument("--phases", help="sweep phases to run, all by default")
    run_command.add_argument("--cases", help="control case ids to run, all by default; in the map phases, the map interactions to run")
    run_command.add_argument("--pages", help="pages the playback and control phases visit, all by default")
    run_command.add_argument("--pages-at", type=float, help="share of the timeline (0 to 1) the pages-at phase visits every page at")
    run_command.add_argument("--profile", choices=tuple(PROFILES), help="named machine profile: weak (4 cores, 4x) or weak8 (4 cores, 8x)")
    run_command.add_argument("--cpu-throttle", type=float, help="CPU throttle rate (>= 1); forwarded to Chromium and drawing-lane stretch()")
    run_command.add_argument("--cores", help="logical processor count or hex affinity mask applied to the app Job object and page")
    run_command.add_argument("--kinds", help="comma-separated widget kinds for the widget-costs phase (all registered kinds by default)")
    run_command.add_argument("--all-views", action="store_true", help="enumerate every registered widget view instead of one default placement per kind")
    run_command.add_argument("--costs", help="path to widget-costs.json used to build the Heaviest fixture page")
    run_command.add_argument("--heaviest", type=int, help="number of most expensive widget kinds placed on the Heaviest page (default 30)")
    run_command.add_argument("--window-client", help="window client area in device pixels, WxH")
    args = parser.parse_args(argv)
    if args.command == "run":
        if bool(args.shell) != bool(args.ui):
            parser.error("--ui goes with --shell, and --shell needs --ui")
        args.app = args.app or args.shell
        for name in ("app", "fixture", "game_dir", "out", "state"):
            path = Path(getattr(args, name)).resolve()
            if name in ("out", "state") and (path == ROOT or ROOT in path.parents):
                parser.error(f"--{name.replace('_', '-')} must be outside the repository")
            setattr(args, name, str(path))
        if args.history:
            history = Path(args.history).resolve()
            if history == ROOT or ROOT in history.parents:
                parser.error("--history must be outside the repository")
            args.history = str(history)
        if args.ui:
            args.ui = str(Path(args.ui).resolve())
        if args.costs:
            args.costs = str(Path(args.costs).resolve())
        if args.record_replies:
            replies = Path(args.record_replies).resolve()
            if ROOT in replies.parents:
                parser.error("--record-replies must be outside the repository")
            args.record_replies = str(replies)
        if any(stage not in STAGES for stage in args.stages.split(",")):
            parser.error(f"--stages takes {', '.join(STAGES)}")
        if args.pages_at is not None and not 0 <= args.pages_at <= 1:
            parser.error("--pages-at takes a share of the timeline from 0 to 1")
        if args.phases and "pages-at" in args.phases.split(",") and args.pages_at is None:
            parser.error("--phases pages-at needs --pages-at <fraction>")
        if args.profile:
            preset = PROFILES[args.profile]
            if args.cores is None:
                args.cores = preset["cores"]
            if args.cpu_throttle is None:
                args.cpu_throttle = preset["cpu_throttle"]
        if args.cpu_throttle is None:
            args.cpu_throttle = 1.0
        if args.cpu_throttle < 1.0:
            parser.error("--cpu-throttle must be >= 1")
        try:
            args.cores = parse_cores(args.cores)
        except ValueError as error:
            parser.error(str(error))
        if args.heaviest is not None and args.heaviest < 1:
            parser.error("--heaviest must be >= 1")
        pages = args.pages.split(",") if args.pages else []
        if "Heaviest" in pages and not args.costs:
            parser.error("--pages Heaviest needs --costs <widget-costs.json>")
        if any(view in pages for view in SCREEN_VIEWS) and TALL_VIEW in pages:
            parser.error(f"--pages {next(view for view in SCREEN_VIEWS if view in pages)} and {TALL_VIEW} need different windows")
        if any(view in pages for view in SCREEN_VIEWS) and not args.window_client:
            args.window_client = "x".join(map(str, SCREEN))
        if args.window_client:
            try:
                args.window_client = tuple(int(part) for part in args.window_client.lower().split("x"))
                assert len(args.window_client) == 2 and min(args.window_client) > 0
            except (ValueError, AssertionError):
                parser.error("--window-client takes WxH in device pixels, such as 3840x2160")
    return args


def main(argv):
    if argv[:1] == ["playback"]:
        import playback
        return playback.main(argv[1:])
    args = parse(argv)
    if args.command == "shell":
        try:
            return run_fetch_shell(
                commit=args.commit,
                out=args.out,
                gate_dir=args.gate_dir,
                dispatch=args.dispatch,
                repo=args.repo,
            )
        except Exception as error:
            print(f"Measuring shell failed: {error}", file=sys.stderr)
            return 2
    if args.command == "build-hosted-shell":
        try:
            return run_build_hosted_shell(
                out=args.out,
                target=args.target,
                source_root=args.source_root,
            )
        except Exception as error:
            print(f"Hosted shell build failed: {error}", file=sys.stderr)
            return 2
    if args.command == "build":
        try:
            return build(args)
        except Exception as error:
            print(f"Build failed: {error}", file=sys.stderr)
            return 2
    if args.command == "build-ui":
        try:
            return build_ui(args)
        except Exception as error:
            print(f"UI build failed: {error}", file=sys.stderr)
            return 2
    if args.command == "accept":
        try:
            return accept(args)
        except Exception as error:
            print(f"Not accepted: {error}", file=sys.stderr)
            return 2
    try:
        return run(args)
    except Exception as error:
        print(f"The suite could not run: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
