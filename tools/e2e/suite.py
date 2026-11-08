"""Battle Plan's acceptance suite: the release desktop app on a prepared copy of a real recording.

    python tools/e2e/suite.py build --out <app dir>
    python tools/e2e/suite.py run --app <app dir> --fixture <folder> --game-dir <the game folder> --out <report dir>
        [--state <dir>] [--stages prepare,sweep,report] [--phases pages,...] [--cases id,...]
    python tools/e2e/suite.py accept --run <report dir> --reason "<the change it accepts>" [--state <dir>]

`build` compiles replay-desktop and viewer with the UI embedded on Boat. `run` copies the fixture's
recording into a fresh profile, prepares it in the app (wall time, CPU and peak commit of the app's
whole process tree), then starts the app again on that profile and runs tools/e2e/sweep.mjs over
it, and writes e2e.md: a table per area, failures with screenshots, and the visual check's
differences from the baseline `accept` last made, with diff images. Each app runs in a Job on a
desktop of its own, so it never takes the user's input or focus, at a fixed window size. Exits 0
when everything passed, 1 when anything failed, 2 when the suite itself could not run.
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
import time
import uuid
from ctypes import wintypes as w
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
# Outside the repository; the nightly runner passes its own.
DEFAULT_STATE = Path(os.environ.get("BATTLE_PLAN_CAPTURES", os.environ.get("HOI4_CAPTURES", ROOT / "captures"))) / "ci" / "e2e"
WINDOW = (1920, 1080)
BOAT_FAILED = 75
CLOSE_SECONDS = 60
STAGES = ("prepare", "sweep", "report")
# Each session is a fresh app on the prepared profile with every cache emptied, so what it times first is cold.
SESSIONS = (
    ("pages", "pages", {}),
    ("seeks", "seeks", {}),
    ("map", "visual,map,playback", {}),
    ("map uncapped", "mapfps", {"BATTLE_PLAN_VIEWER_NO_VSYNC": "1"}),
    ("catalogue", "catalogue", {}),
    ("controls", "controls", {}),
)
# Under the fresh profile: emptied before every session.
CACHES = ("local/HOI4Replay/viewer", "local/HOI4Replay/shader-cache", "local/HOI4Replay/artwork", "local/Microsoft",
          "roaming", "webview")
# What preparation made, kept: the prepared store, the recording, and the artwork it drew (restored from a copy).
PREPARED = ("local/HOI4Replay/prepared", "local/HOI4Replay/captures", "local/HOI4Replay/artwork")

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
u.CreateDesktopW.argtypes = [w.LPCWSTR, w.LPCWSTR, c.c_void_p, w.DWORD, w.DWORD, c.c_void_p]
u.CreateDesktopW.restype = w.HANDLE
u.CloseDesktop.argtypes = [w.HANDLE]
ENUM = c.WINFUNCTYPE(w.BOOL, w.HWND, w.LPARAM)
u.EnumDesktopWindows.argtypes = [w.HANDLE, ENUM, w.LPARAM]
u.GetWindowThreadProcessId.argtypes = [w.HWND, c.POINTER(w.DWORD)]
u.GetWindowTextW.argtypes = [w.HWND, w.LPWSTR, c.c_int]
u.IsWindowVisible.argtypes = [w.HWND]
u.GetWindowRect.argtypes = [w.HWND, c.POINTER(w.RECT)]
u.SetWindowPos.argtypes = [w.HWND, w.HWND, c.c_int, c.c_int, c.c_int, c.c_int, w.UINT]
u.ShowWindow.argtypes = [w.HWND, c.c_int]
u.PostMessageW.argtypes = [w.HWND, w.UINT, w.WPARAM, w.LPARAM]

u.EnumDisplaySettingsW.argtypes = [w.LPCWSTR, w.DWORD, c.POINTER(DisplayMode)]

WM_CLOSE = 0x10
STILL_ACTIVE = 258


class App:
    """The desktop app in a Job of its own, on a desktop of its own: only this tree is ever stopped."""

    def __init__(self, exe, args, environment, log, port):
        self.port = port
        self.desktop_name = f"HOI4ReplayE2E-{uuid.uuid4().hex[:12]}"
        self.desktop = checked(u.CreateDesktopW(self.desktop_name, None, None, 0, 0x1FF, None))
        self.job = checked(k.CreateJobObjectW(None, None))
        limits = Extended()
        limits.basic.flags = 0x2000  # JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        checked(k.SetInformationJobObject(self.job, 9, c.byref(limits), c.sizeof(limits)))
        self.log = open(log, "wb")
        null = open(os.devnull, "rb")
        for stream in (self.log, null):
            os.set_handle_inheritable(msvcrt_handle(stream), True)
        startup = Startup()
        startup.cb = c.sizeof(startup)
        startup.desktop = "winsta0\\" + self.desktop_name
        startup.flags = 0x100  # STARTF_USESTDHANDLES
        startup.stdout = startup.stderr = msvcrt_handle(self.log)
        startup.stdin = msvcrt_handle(null)
        block = "".join(f"{key}={value}\0" for key, value in environment.items()) + "\0"
        self.info = ProcessInfo()
        command = c.create_unicode_buffer(subprocess.list2cmdline([str(exe), *args]))
        # Suspended until it is in the Job, so nothing it starts escapes it; Unicode environment block.
        checked(k.CreateProcessW(str(exe), command, None, None, True, 0x08000004 | 0x400, (c.c_wchar * len(block))(*block),
                                 str(exe.parent), c.byref(startup), c.byref(self.info)))
        checked(k.AssignProcessToJobObject(self.job, self.info.process))
        if k.ResumeThread(self.info.thread) == 0xFFFFFFFF:
            raise c.WinError(c.get_last_error())
        null.close()
        self.started = time.monotonic()

    def usage(self):
        """CPU seconds and peak commit bytes of every process the app's tree has run."""
        basic, extended = Accounting(), Extended()
        checked(k.QueryInformationJobObject(self.job, 1, c.byref(basic), c.sizeof(basic), None))
        checked(k.QueryInformationJobObject(self.job, 9, c.byref(extended), c.sizeof(extended), None))
        return (basic.user + basic.kernel) / 1e7, extended.peak_job

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
                if title == "Battle Plan":
                    return hwnd
            if not self.running():
                raise RuntimeError(f"the app exited with {self.exit_code()} before showing its window")
            time.sleep(0.25)
        raise RuntimeError(f"the app showed no window in {seconds} s")

    def size(self, width, height):
        hwnd = self.main_window()
        u.ShowWindow(hwnd, 9)  # SW_RESTORE: a maximised window keeps its size
        checked(u.SetWindowPos(hwnd, None, 0, 0, width, height, 0x14))  # SWP_NOZORDER | SWP_NOACTIVATE
        rect = w.RECT()
        checked(u.GetWindowRect(hwnd, c.byref(rect)))
        if (rect.right - rect.left, rect.bottom - rect.top) != (width, height):
            raise RuntimeError(f"the window is {rect.right - rect.left}x{rect.bottom - rect.top}, not {width}x{height}")

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
                    if title == "Battle Plan":
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
        return code


def msvcrt_handle(stream):
    import msvcrt
    return msvcrt.get_osfhandle(stream.fileno())


def refresh_hz():
    """The display's refresh rate, which sets what a dropped frame is."""
    mode = DisplayMode()
    mode.size = c.sizeof(mode)
    checked(u.EnumDisplaySettingsW(None, 0xFFFFFFFF, c.byref(mode)))  # ENUM_CURRENT_SETTINGS
    if mode.frequency <= 1:
        raise RuntimeError(f"the display reports no refresh rate ({mode.frequency})")
    return mode.frequency


def user_folder(given):
    """The Paradox user folder: given, else where the recorder finds it (the Documents shell folder, then
    OneDrive's and the profile's Documents). Its settings.txt sets the language prepared text is made in."""
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
    recordings = sorted((*Path(folder).glob("*.battleplan"), *Path(folder).glob("*.hoi4replay")))
    if len(recordings) != 1:
        raise RuntimeError(f"{folder} holds {len(recordings)} recordings; the fixture is one")
    return recordings[0]


def shader_compiler(state):
    """The pinned shader compiler the viewer needs, installed once into the suite's state."""
    dxc = state / "shader-tools" / "dxc"
    if not (dxc / "dxc.exe").is_file():
        subprocess.run(["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
                        str(ROOT / "tools" / "install-shader-compiler.ps1"), "-Destination", str(dxc)], check=True)
    return dxc / "dxc.exe"


def app_environment(profile, port, extra):
    environment = dict(os.environ)
    for key, folder in (("APPDATA", "roaming"), ("LOCALAPPDATA", "local"), ("WEBVIEW2_USER_DATA_FOLDER", "webview")):
        (profile / folder).mkdir(parents=True, exist_ok=True)
        environment[key] = str(profile / folder)
    environment["WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS"] = (
        f"--remote-debugging-port={port} --disable-backgrounding-occluded-windows "
        "--disable-renderer-backgrounding --disable-background-timer-throttling")
    environment.pop("BATTLE_PLAN_PERF_LOG", None)
    environment.update(extra)
    return environment


def launch(args, work, label, extra=None):
    """The app on the suite's fresh profile, with the user's own game and Paradox user folders, which it only reads."""
    profile = work / "profile"
    port = free_port()
    environment = app_environment(profile, port, {"BATTLE_PLAN_SHADER_COMPILER": str(args.dxc), **(extra or {})})
    app_args = ["--game-dir", args.game_dir, "--user-dir", str(args.user_folder)]
    app = App(Path(args.app) / "replay-desktop.exe", app_args, environment, args.out / f"app-{label}.log", port)
    try:
        app.size(*WINDOW)
    except BaseException:
        app.close()
        raise
    return app


def cold_profile(work):
    """Empties every cache under the fresh profile and puts back the artwork preparation drew; what is left is said."""
    profile = work / "profile"
    for cache in CACHES:
        # The last app's processes release their files a moment after its Job ends.
        deadline = time.monotonic() + 30
        while (profile / cache).exists():
            shutil.rmtree(profile / cache, ignore_errors=True)
            if (profile / cache).exists():
                if time.monotonic() > deadline:
                    raise RuntimeError(f"could not empty {profile / cache} within 30 s")
                time.sleep(1)
    shutil.copytree(work / "prepared-artwork", profile / "local" / "HOI4Replay" / "artwork")
    left = sorted(str(path.relative_to(profile)).replace("\\", "/") for path in (profile / "local" / "HOI4Replay").iterdir())
    expected = sorted(PREPARED)
    if left != expected or any((profile / name).exists() for name in ("roaming", "webview", "local/Microsoft")):
        raise RuntimeError(f"the profile is not cold: it holds {left}")
    return {"emptied": list(CACHES), "kept": list(PREPARED)}


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


def prepare(args, work, recording):
    """Prepares the copied recording in a fresh app and measures that app's tree."""
    app = launch(args, work, "prepare")
    result = {}
    try:
        out = work / "prepare.json"
        log = args.out / "prepare.log"
        with open(log, "wb") as output:
            process = subprocess.Popen([node(), str(HERE / "prepare.mjs"), "--port", str(app.port), "--recording", str(recording),
                                        "--out", str(out)], cwd=ROOT, stdout=subprocess.PIPE, stderr=output,
                                       creationflags=subprocess.CREATE_NO_WINDOW)
            ended = None
            for line in process.stdout:
                output.write(line)
                event = json.loads(line) if line.startswith(b"{") else {}
                if event.get("event") == "ended":
                    ended = (time.monotonic(), *app.usage())
            code = process.wait()
        if code != 0 or not out.is_file() or ended is None:
            raise RuntimeError(f"prepare.mjs failed (exit {code}); see {log}")
        result = json.loads(out.read_text(encoding="utf-8"))
        # From launch: an app with a recording in its library may start preparing it before it is asked.
        result["wallSeconds"] = ended[0] - app.started
        result["cpuSeconds"] = ended[1]
        result["peakCommitGb"] = ended[2] / 2**30
    finally:
        result["exitCode"] = app.close()
    return result


def run(args):
    started = time.monotonic()
    args.out = Path(args.out).resolve()
    args.out.mkdir(parents=True, exist_ok=True)
    state = Path(args.state).resolve()
    work = state / "work"
    stages = args.stages.split(",")
    phases = {}
    record = {"commit": git_head(), "started": datetime.datetime.now().isoformat(timespec="seconds"), "phases": phases,
              "refreshHz": refresh_hz()}
    if not (Path(args.game_dir) / "hoi4.exe").is_file():
        raise RuntimeError(f"{args.game_dir} holds no hoi4.exe")
    for name in ("replay-desktop.exe", "viewer.exe"):
        if not (Path(args.app) / name).is_file():
            raise RuntimeError(f"{args.app} holds no {name}")
    source = fixture_recording(args.fixture)
    args.user_folder = user_folder(args.user_dir)
    record["userFolder"] = str(args.user_folder)
    # Baselines are of one fixture: its name and size tell a swapped fixture from tonight's.
    record["fixture"] = {"name": source.name, "bytes": source.stat().st_size}
    args.dxc = shader_compiler(state)
    # In the fresh profile's own library, where the app lists it, as a recording the user saved.
    recording = work / "profile" / "local" / "HOI4Replay" / "captures" / source.name
    if "prepare" in stages:
        began = time.monotonic()
        # Every night from nothing: the copied recording, a fresh profile, no prepared data.
        if work.exists():
            shutil.rmtree(work)
        recording.parent.mkdir(parents=True)
        shutil.copy2(source, recording)
        phases["copy"] = time.monotonic() - began
        began = time.monotonic()
        try:
            prep = prepare(args, work, recording)
        except Exception as error:
            prep = {"state": "not run", "error": str(error)}
        (args.out / "prep.json").write_text(json.dumps(prep, indent=1), encoding="utf-8")
        phases["prepare"] = time.monotonic() - began
        artwork = work / "profile" / "local" / "HOI4Replay" / "artwork"
        if artwork.is_dir():
            shutil.copytree(artwork, work / "prepared-artwork")
    elif not recording.is_file():
        raise RuntimeError(f"no prepared work in {work}: run the prepare stage first")
    prepared = json.loads((args.out / "prep.json").read_text(encoding="utf-8")) if (args.out / "prep.json").is_file() else None
    if "sweep" in stages and prepared is not None and prepared.get("state") != "ready":
        # Nothing can be measured on a recording that is not prepared; the report says why, once.
        record["sweepNotRun"] = f"the fixture was not prepared: {prepared.get('error') or prepared.get('state')}"
    elif "sweep" in stages:
        if not (work / "prepared-artwork").is_dir():
            raise RuntimeError(f"no copy of the prepared artwork in {work}: run the prepare stage first")
        record["sessions"] = []
        wanted = args.phases.split(",") if args.phases else None
        for name, session_phases, environment in SESSIONS:
            chosen = [phase for phase in session_phases.split(",") if wanted is None or phase in wanted]
            if not chosen:
                continue
            began = time.monotonic()
            session = {"name": name, "phases": chosen, "emptied": list(CACHES), "kept": list(PREPARED)}
            slug = name.replace(" ", "-")
            # A session that cannot start is a failure of its own; the next still runs, and the report is written.
            # The viewer's suite input channel, for pointer and key input to the native map: suite sessions only.
            input_address = f"127.0.0.1:{free_port()}"
            try:
                cold_profile(work)
                app = launch(args, work, f"sweep-{slug}", {"BATTLE_PLAN_PERF_LOG": str(work / f"perf-{slug}.jsonl"),
                                                           "BATTLE_PLAN_E2E_INPUT": input_address, "BATTLE_PLAN_E2E_CAPTURE_CLOCK": "0",
                                                           **environment})
            except Exception as error:
                session.update(error=str(error), sweepExit=None, appExit=None, seconds=time.monotonic() - began)
                record["sessions"].append(session)
                continue
            try:
                session["sweepExit"] = driver("sweep.mjs", args.out / f"sweep-{slug}.log", "--port", app.port, "--out", args.out,
                                              "--work", work, "--session", slug, "--desktop", app.desktop_name, "--app-pid", app.info.pid,
                                              "--input", input_address,
                                              "--phases", ",".join(chosen), *(["--cases", args.cases] if args.cases else []),
                                              seconds=3 * 3600)
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
                 "--history", state / "prep-history.jsonl", "--baseline", state / "visual-baseline"]
    if (args.out / "prep.json").is_file():
        arguments += ["--prep", args.out / "prep.json"]
    if "prepare" in stages:
        arguments.append("--record")
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
    """The release app with the UI embedded, compiled on Boat; a Boat failure is retried once, as Boat asks."""
    command = [sys.executable, str(ROOT / "tools" / "boat.py"), "build", "--out", str(Path(args.out).resolve()), "--release",
               "-p", "replay-desktop", "-p", "viewer", "--features", "replay-desktop/custom-protocol"]
    for attempt in (1, 2):
        code = subprocess.run(command, cwd=ROOT).returncode
        if code != BOAT_FAILED:
            return code
        print(f"Boat failed (exit {code}), attempt {attempt}", file=sys.stderr)
    return code


def parse(argv):
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    commands = parser.add_subparsers(dest="command", required=True)
    build_command = commands.add_parser("build")
    build_command.add_argument("--out", required=True)
    accept_command = commands.add_parser("accept", help="make a run's captures the visual baseline")
    accept_command.add_argument("--run", required=True, help="the run's report folder")
    accept_command.add_argument("--reason", required=True, help="the change this baseline accepts")
    accept_command.add_argument("--state", default=str(DEFAULT_STATE))
    run_command = commands.add_parser("run")
    run_command.add_argument("--app", required=True, help="folder holding replay-desktop.exe and viewer.exe")
    run_command.add_argument("--fixture", default=os.environ.get("BATTLE_PLAN_E2E_FIXTURE", os.environ.get("HOI4_E2E_FIXTURE")), required=not os.environ.get("BATTLE_PLAN_E2E_FIXTURE", os.environ.get("HOI4_E2E_FIXTURE")))
    run_command.add_argument("--game-dir", default=os.environ.get("BATTLE_PLAN_GAME_DIR", os.environ.get("HOI4_GAME_DIR")), required=not os.environ.get("BATTLE_PLAN_GAME_DIR", os.environ.get("HOI4_GAME_DIR")))
    run_command.add_argument("--user-dir", default=os.environ.get("BATTLE_PLAN_USER_DIR", os.environ.get("HOI4_USER_DIR")), help="the Paradox user folder, else found as the recorder finds it")
    run_command.add_argument("--out", required=True)
    run_command.add_argument("--state", default=str(DEFAULT_STATE), help="baseline history, shader compiler and the last run's work")
    run_command.add_argument("--stages", default=",".join(STAGES))
    run_command.add_argument("--phases", help="sweep phases to run, all by default")
    run_command.add_argument("--cases", help="control case ids to run, all by default")
    args = parser.parse_args(argv)
    if args.command == "run":
        for name in ("app", "fixture", "game_dir", "out", "state"):
            path = Path(getattr(args, name)).resolve()
            if name in ("out", "state") and (path == ROOT or ROOT in path.parents):
                parser.error(f"--{name.replace('_', '-')} must be outside the repository")
            setattr(args, name, str(path))
        if any(stage not in STAGES for stage in args.stages.split(",")):
            parser.error(f"--stages takes {', '.join(STAGES)}")
    return args


def main(argv):
    args = parse(argv)
    if args.command == "build":
        return build(args)
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
