"""BattlePlan's acceptance suite: the release desktop app on a copy of a real recording.

    python tools/e2e/suite.py build --out <app dir> [--shell]
    python tools/e2e/suite.py run --app <app dir> | --shell <shell dir> --ui <bundle dir>
        --fixture <folder> --game-dir <the game folder> --out <report dir>
        [--tier full|quick] [--history <file>] [--pin all|product] [--record-replies <file>]
        [--state <dir>] [--stages copy,sweep,report] [--phases pages,...] [--cases id,...] [--pages page,...] [--pages-at <fraction>]
    python tools/e2e/suite.py accept --run <report dir> --reason "<the change it accepts>" [--state <dir>]
    python tools/e2e/suite.py playback map|harness|dashboard ...   (tools/e2e/playback.py: the playback phase)

`build` compiles replay-desktop with the UI embedded on Boat; `--shell` builds the measuring shell instead,
the same release app serving its UI from a production bundle on disk, so a UI change is measured by rebuilding only
the bundle (`npx.cmd vite build --config ui/vite.config.ts`, into ui/dist) and running the shell on it with
`run --shell <shell dir> --ui ui/dist`; a shell is built once and serves any later bundle. `run` copies the fixture's
recording into a fresh profile, then starts the app on that profile for each session and runs
tools/e2e/sweep.mjs over it, and writes e2e.md: a table per area, failures with screenshots, and the visual check's
differences from the baseline `accept` last made, with diff images. `--pages-at` adds a session that seeks to that
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
import json
import os
import shutil
import socket
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
DEFAULT_STATE = Path(os.environ.get("BATTLEPLAN_CAPTURES", os.environ.get("HOI4_CAPTURES", ROOT / "captures"))) / "ci" / "e2e"
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
          "roaming", "webview")
KEPT = ("local/BattlePlan/captures",)
APPS = Path(tempfile.gettempdir()) / "battleplan-e2e" / "apps"
APP_EXE = "replay-desktop.exe"
SHELL_FEATURE = "replay-desktop/served-ui"

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


def shell_build(folder):
    """What built a measuring shell, or why the folder holds none."""
    try:
        built = json.loads((Path(folder) / "boat-artifacts.json").read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        raise RuntimeError(f"{folder} holds no Boat build record ({error}): build the shell with `suite.py build --shell`")
    arguments = next((item.get("cargo_arguments", []) for item in built.get("artifacts", [])
                      if Path(item.get("path", "")).name == APP_EXE), [])
    if not any(SHELL_FEATURE in argument for argument in arguments):
        raise RuntimeError(f"{folder}'s {APP_EXE} is not a measuring shell (built without {SHELL_FEATURE}): "
                           "build one with `suite.py build --shell`, or pass the folder as --app")
    return {"folder": str(folder), "revision": built.get("revision"), "cargoArguments": arguments}


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


def shader_compiler(state, app):
    """The pinned shader compiler beside the app, whose maps load it from there: fetched once into the suite's state."""
    copy = state / "shader-compiler"
    if not all((copy / name).is_file() for name in SHADER_COMPILER):
        subprocess.run(["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
                        str(ROOT / "tools" / "install-shader-compiler.ps1"), "-Destination", str(copy)], check=True)
    for name in SHADER_COMPILER:
        if not (app / name).is_file():
            shutil.copy2(copy / name, app / name)


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
                    session["appExit"] = app.close()
            session["seconds"] = time.monotonic() - began
            record["sessions"].append(session)
            phases[f"sweep: {name}"] = session["seconds"]
    record["seconds"] = time.monotonic() - started
    (args.out / "run.json").write_text(json.dumps(record, indent=1), encoding="utf-8")
    if "report" not in stages:
        return 0
    arguments = ["--run", args.out / "run.json", "--sweeps", args.out, "--perf", work, "--out", args.out,
                 "--baseline", state / "visual-baseline", "--history", history]
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


def build(args):
    """The release app with the UI embedded, or the measuring shell, compiled on Boat; a Boat failure is retried once,
    as Boat asks."""
    features = "replay-desktop/custom-protocol" + (f",{SHELL_FEATURE}" if args.shell else "")
    command = [sys.executable, str(ROOT / "tools" / "boat.py"), "build", "--out", str(Path(args.out).resolve()), "--release",
               "-p", "replay-desktop", "--features", features]
    for attempt in (1, 2):
        code = subprocess.run(command, cwd=ROOT).returncode
        if code != BOAT_FAILED:
            return code
        print(f"Boat failed (exit {code}), attempt {attempt}", file=sys.stderr)
    return code


def build_ui(args):
    """Builds a production UI bundle into `--out`; `--ungated` sets the measurement-only build constant."""
    out = Path(args.out).resolve()
    if out == ROOT or ROOT in out.parents:
        raise RuntimeError("--out for build-ui must be outside the repository")
    env = dict(os.environ)
    if args.ungated:
        env["BATTLEPLAN_MEASURE_UNGATED"] = "1"
    else:
        env.pop("BATTLEPLAN_MEASURE_UNGATED", None)
    command = ["npx.cmd", "vite", "build", "--config", str(ROOT / "ui" / "vite.config.ts"),
               "--outDir", str(out), "--emptyOutDir"]
    return subprocess.run(command, cwd=ROOT, env=env).returncode


def parse(argv):
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    commands = parser.add_subparsers(dest="command", required=True)
    build_command = commands.add_parser("build")
    build_command.add_argument("--out", required=True)
    build_command.add_argument("--shell", action="store_true",
                               help="the measuring shell: the release app serving its UI from the bundle `run --ui` names")
    build_ui_command = commands.add_parser("build-ui", help="build a production UI bundle for the measuring shell")
    build_ui_command.add_argument("--out", required=True)
    build_ui_command.add_argument("--ungated", action="store_true",
                                  help="measurement-only build that bypasses the playsBackDated placeholder gate")
    accept_command = commands.add_parser("accept", help="make a run's captures the visual baseline")
    accept_command.add_argument("--run", required=True, help="the run's report folder")
    accept_command.add_argument("--reason", required=True, help="the change this baseline accepts")
    accept_command.add_argument("--state", default=str(DEFAULT_STATE))
    run_command = commands.add_parser("run")
    app = run_command.add_mutually_exclusive_group(required=True)
    app.add_argument("--app", help="folder holding replay-desktop.exe; the pinned shader compiler is put beside it")
    app.add_argument("--shell", help="folder holding the measuring shell (`build --shell`); needs --ui")
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
    if args.command == "build":
        return build(args)
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
