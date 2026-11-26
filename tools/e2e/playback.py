"""The acceptance suite's playback phase: the map headless on the viewer's paced harness and proof tool, judged
against the bars of "completely smooth", and the dashboard judged from a desktop app's perf log.

    python tools/e2e/playback.py map --viewer <viewer.exe> --recording <file> --from-hour H --hours N --out <dir>
        [--speed 1440] [--refresh-hz 60] [--seeks 10] [--cores 0x55] [--repeat 1]
        [--only floor,layers,layers-seeks,proof] [--repeat-roles layers]
        [--oversized-recording <file> [--oversized-factor K]]
    python tools/e2e/playback.py harness --viewer <viewer.exe> --out <run dir> [--cores 0x55]
        [--minutes 30] [--stop-after S] [--label NAME] -- <viewer arguments, without --out>
    python tools/e2e/playback.py dashboard --perf-log <file> --out <dir> [--refresh-hz 60] [--page NAME]
        [--playing-seconds S] [--seeks N]

`harness` runs one `viewer --playback-cost` or `--playhead-proof` with its process handle taken at spawn, pinned to
`--cores`, its peak memory sampled, the machine's load recorded before and after, its exit code kept, and stops only
that process (on `--minutes`, or deliberately after `--stop-after` seconds); then it says WHOLE or PARTIAL.
`map` runs the phase's set of harness runs into <dir> and writes <dir>/verdicts.md. `dashboard` judges a perf log
(BATTLEPLAN_PERF_LOG) into <dir>/dashboard.md. The viewer loads the pinned shader compiler, dxcompiler.dll and dxil.dll, from
beside itself; while the folder BATTLEPLAN_TIMING_LOCK names exists, a run waits for it (up to ten minutes, then goes on and says so); a run starts only
with 8 GB of physical memory free, and is partial when the machine ran under 2 GB free during it (paging). Exits 0 when
every judged bar passed, 1 when any failed or a run was partial, 2 when the phase could not run.
"""

import argparse
import ctypes as c
import json
import os
import subprocess
import sys
import threading
import time
from ctypes import wintypes as w
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
LOCK_WAIT_SECONDS = 600
MEMORY_TO_START = 8 << 30
FLOOR_FRAMES = 600
FLOOR_PAN = 8
SEEK_FRAMES = 15
ROLES = ("floor", "layers", "layers-seeks", "proof")
DEFAULT_ROLES = "floor,layers,layers-seeks"

k = c.WinDLL("kernel32", use_last_error=True)
psapi = c.WinDLL("psapi", use_last_error=True)


class FileTime(c.Structure):
    _fields_ = [("low", w.DWORD), ("high", w.DWORD)]

    def ticks(self):
        return self.high << 32 | self.low


class MemoryStatus(c.Structure):
    _fields_ = [("length", w.DWORD), ("load", w.DWORD)] + [(name, c.c_uint64) for name in (
        "total_physical", "available_physical", "total_file", "available_file", "total_virtual", "available_virtual", "extended")]


def available_memory():
    status = MemoryStatus()
    status.length = c.sizeof(status)
    if not k.GlobalMemoryStatusEx(c.byref(status)):
        raise c.WinError(c.get_last_error())
    return status.available_physical


def wait_for_memory():
    """Seconds held until physical memory free reached MEMORY_TO_START."""
    began = time.monotonic()
    while available_memory() < MEMORY_TO_START:
        time.sleep(5)
    return round(time.monotonic() - began, 1)


class Memory(c.Structure):
    _fields_ = [("cb", w.DWORD), ("faults", w.DWORD)] + [(name, c.c_size_t) for name in (
        "peak_working_set", "working_set", "peak_paged_pool", "paged_pool", "peak_nonpaged_pool", "nonpaged_pool",
        "pagefile", "peak_pagefile", "private")]


k.GetSystemTimes.argtypes = [c.POINTER(FileTime)] * 3
k.GlobalMemoryStatusEx.argtypes = [c.POINTER(MemoryStatus)]
k.GetProcessTimes.argtypes = [w.HANDLE] + [c.POINTER(FileTime)] * 4
k.OpenProcess.argtypes = [w.DWORD, w.BOOL, w.DWORD]
k.OpenProcess.restype = w.HANDLE
k.CloseHandle.argtypes = [w.HANDLE]
k.QueryFullProcessImageNameW.argtypes = [w.HANDLE, w.DWORD, w.LPWSTR, c.POINTER(w.DWORD)]
k.SetProcessAffinityMask.argtypes = [w.HANDLE, c.c_size_t]
psapi.EnumProcesses.argtypes = [c.POINTER(w.DWORD), w.DWORD, c.POINTER(w.DWORD)]
psapi.GetProcessMemoryInfo.argtypes = [w.HANDLE, c.POINTER(Memory), w.DWORD]
QUERY_LIMITED = 0x1000


def process_times():
    """CPU 100 ns ticks and image name of every process this user can query, by PID."""
    pids = (w.DWORD * 8192)()
    used = w.DWORD()
    psapi.EnumProcesses(pids, c.sizeof(pids), c.byref(used))
    found = {}
    for pid in pids[:used.value // 4]:
        handle = k.OpenProcess(QUERY_LIMITED, False, pid)
        if not handle:
            continue
        try:
            times = [FileTime() for _ in range(4)]
            name = c.create_unicode_buffer(1024)
            size = w.DWORD(1024)
            if k.GetProcessTimes(handle, *[c.byref(t) for t in times]):
                k.QueryFullProcessImageNameW(handle, 0, name, c.byref(size))
                found[pid] = (times[2].ticks() + times[3].ticks(), Path(name.value).name or str(pid))
        finally:
            k.CloseHandle(handle)
    return found


def machine_load(seconds=2.0):
    """The machine's CPU over `seconds`, in percent of every logical processor, and the busiest processes in it."""
    idle0, kernel0, user0 = FileTime(), FileTime(), FileTime()
    k.GetSystemTimes(c.byref(idle0), c.byref(kernel0), c.byref(user0))
    before = process_times()
    time.sleep(seconds)
    idle1, kernel1, user1 = FileTime(), FileTime(), FileTime()
    k.GetSystemTimes(c.byref(idle1), c.byref(kernel1), c.byref(user1))
    after = process_times()
    total = (kernel1.ticks() - kernel0.ticks()) + (user1.ticks() - user0.ticks())
    idle = idle1.ticks() - idle0.ticks()
    busy = []
    for pid, (ticks, name) in after.items():
        if pid in before and before[pid][1] == name:
            busy.append({"name": name, "pid": pid, "cpu": 100.0 * (ticks - before[pid][0]) / max(1, total)})
    busy.sort(key=lambda entry: -entry["cpu"])
    lock = os.environ.get("BATTLEPLAN_TIMING_LOCK", os.environ.get("HOI4_TIMING_LOCK"))
    return {"at": time.strftime("%Y-%m-%dT%H:%M:%S%z"), "cpu_percent": round(100.0 * (total - idle) / max(1, total), 1),
            "busy": [{**entry, "cpu": round(entry["cpu"], 1)} for entry in busy[:8] if entry["cpu"] >= 0.5],
            "lock": bool(lock) and Path(lock).is_dir(), "free_memory_mb": round(available_memory() / 2 ** 20)}


def wait_for_lock():
    """Seconds waited for the timing lock to go, and whether it was still there when the wait ended."""
    lock = os.environ.get("BATTLEPLAN_TIMING_LOCK", os.environ.get("HOI4_TIMING_LOCK"))
    began = time.monotonic()
    while lock and Path(lock).is_dir() and time.monotonic() - began < LOCK_WAIT_SECONDS:
        time.sleep(5)
    return round(time.monotonic() - began, 1), bool(lock) and Path(lock).is_dir()


def cores_of(mask):
    return [index for index in range(64) if mask >> index & 1]


def spawn(command, environment, folder, cores, minutes, stop_after):
    """Runs `command` with its handle taken at spawn and stops only that process. Its exit code, why it was stopped
    (None when it exited by itself), its peak working set and private bytes, its page faults, the least physical memory
    the machine had free while it ran, CPU seconds and wall seconds."""
    with open(folder / "stdout.txt", "wb") as out, open(folder / "stderr.txt", "wb") as err:
        began = time.monotonic()
        process = subprocess.Popen(command, env=environment, stdout=out, stderr=err, stdin=subprocess.DEVNULL,
                                   creationflags=subprocess.CREATE_NO_WINDOW)
        handle = w.HANDLE(int(process._handle))
        if cores and not k.SetProcessAffinityMask(handle, cores):
            process.kill()
            process.wait()
            raise c.WinError(c.get_last_error())
        (folder / "pid.txt").write_text(f"{process.pid}\n", encoding="utf-8")
        peak = {"working_set": 0, "private": 0, "cpu": 0.0, "faults": 0, "least_free": available_memory()}
        done = threading.Event()

        def sample():
            memory = Memory()
            memory.cb = c.sizeof(memory)
            times = [FileTime() for _ in range(4)]
            while True:
                if psapi.GetProcessMemoryInfo(handle, c.byref(memory), memory.cb):
                    peak["working_set"] = max(peak["working_set"], memory.peak_working_set)
                    peak["private"] = max(peak["private"], memory.private)
                    peak["faults"] = memory.faults
                peak["least_free"] = min(peak["least_free"], available_memory())
                if k.GetProcessTimes(handle, *[c.byref(t) for t in times]):
                    peak["cpu"] = (times[2].ticks() + times[3].ticks()) / 1e7
                if done.wait(0.05):
                    return

        sampler = threading.Thread(target=sample, daemon=True)
        sampler.start()
        limit = minutes * 60 if stop_after is None else min(minutes * 60, stop_after)
        stopped = None
        try:
            process.wait(timeout=limit)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
            stopped = f"stopped after {stop_after} s, as asked" if stop_after is not None and stop_after <= minutes * 60 else f"stopped at the {minutes} min limit"
        done.set()
        sampler.join()
        return {"exit": process.returncode, "stopped": stopped, "pid": process.pid, "seconds": round(time.monotonic() - began, 2),
                "peak": {"working_set_mb": round(peak["working_set"] / 2 ** 20, 1), "private_mb": round(peak["private"] / 2 ** 20, 1)},
                "memory": {"least_free_mb": round(peak["least_free"] / 2 ** 20), "page_faults": peak["faults"]},
                "cpu_s": round(peak["cpu"], 2)}


def check_shader_compiler(viewer):
    """The pinned DXC the viewer loads from beside itself is there."""
    folder = Path(viewer).parent
    missing = [name for name in ("dxcompiler.dll", "dxil.dll") if not (folder / name).is_file()]
    if missing:
        raise RuntimeError(f"the viewer needs the pinned {' and '.join(missing)} beside it: "
                           f"tools/install-shader-compiler.ps1 -Destination {folder}")


def harness_run(viewer, arguments, folder, *, cores=0, minutes=30, stop_after=None, label=None, page="map",
                role=None, repeat=None):
    """One harness run into `folder` (new): the viewer writes `folder/harness`, the runner `folder/run.json`."""
    folder.mkdir(parents=True)
    environment = dict(os.environ)
    environment["BATTLEPLAN_CAPTURES"] = str(folder)
    environment["BATTLEPLAN_CACHE"] = str(folder / "recorder-cache")
    check_shader_compiler(viewer)
    environment.pop("BATTLEPLAN_PERF_LOG", None)
    kind = "proof" if "--playhead-proof" in arguments else "paced" if "--paced" in arguments else "step"
    command = [str(viewer), *arguments, "--out", str(folder / "harness")]
    waited = held = 0.0
    while True:
        seconds, still = wait_for_lock()
        waited += seconds
        held += wait_for_memory()
        before = machine_load()
        if still or not before["lock"]:
            break
    result = spawn(command, environment, folder, cores, minutes, stop_after)
    after = machine_load()
    record = {"label": label or folder.name, "page": page, "role": role, "repeat": repeat, "kind": kind, "viewer": str(viewer),
              "args": arguments, "command": command,
              "cores": f"{cores:#x} ({', '.join(map(str, cores_of(cores)))})" if cores else None,
              "lock_wait": {"seconds": waited, "still_held": still}, "memory_wait_seconds": held, "load": {"before": before, "after": after}, **result}
    (folder / "run.json").write_text(json.dumps(record, indent=1), encoding="utf-8")
    return record


def node(*arguments):
    return subprocess.run(["node", str(HERE / "playback.mjs"), *map(str, arguments)], cwd=ROOT).returncode


def phase_runs(args, recording, page):
    """The phase's runs on one recording: (role, viewer arguments). Every map draws from the recording's playhead."""
    span = ["--from-hour", str(args.from_hour), "--hours", str(args.hours), "--refresh-hz", str(args.refresh_hz)]
    paced = ["--playback-cost", str(recording), *span, "--paced", str(args.speed)]
    floor = ["--playback-cost", str(recording), "--from-hour", str(args.from_hour), "--hours", str(FLOOR_FRAMES),
             "--refresh-hz", str(args.refresh_hz), "--paced", str(args.speed), "--paused", "--pan", str(FLOOR_PAN)]
    runs = [("floor", floor), ("layers", paced)]
    missing = []
    frames = args.hours / args.speed * args.refresh_hz
    seeks = min(args.seeks, int(frames / SEEK_FRAMES) - 1)
    if seeks > 0:
        runs.append(("layers-seeks", [*paced, "--seeks", str(seeks)]))
    else:
        missing.append({"page": page, "run": "layers-seeks", "bar": "seek: sought hour on screen",
                        "why": f"the span plays {frames:.0f} frames; the harness seeks at least {SEEK_FRAMES} apart"})
    runs.append(("proof", ["--playhead-proof", str(recording), "--timing", "--seeks", str(args.seeks),
                           "--speed", str(args.speed), "--hours", str(args.hours)]))
    wanted = set((args.only or DEFAULT_ROLES).split(","))
    return [run for run in runs if run[0] in wanted], missing


def map_phase(args):
    out = Path(args.out).resolve()
    if out.exists() and any(out.iterdir()):
        raise RuntimeError(f"{out} is not empty: a phase writes into a folder of its own")
    out.mkdir(parents=True, exist_ok=True)
    cores = int(args.cores, 0) if args.cores else 0
    sets = [("map", Path(args.recording).resolve())]
    missing = []
    if args.oversized_recording:
        sets.append((f"map x{args.oversized_factor or '?'} (oversized)", Path(args.oversized_recording).resolve()))
    else:
        missing.append({"page": "map (oversized)", "run": "–", "bar": "every map bar on a world several times the size",
                        "why": "no multiplied recording given (--oversized-recording)"})
    for page, recording in sets:
        runs, absent = phase_runs(args, recording, page)
        missing += absent
        repeated = set(args.repeat_roles.split(",")) if args.repeat_roles else None
        for repeat in range(1, args.repeat + 1):
            for role, arguments in runs:
                if repeat > 1 and repeated is not None and role not in repeated:
                    continue
                slug = f"{'o-' if page != 'map' else ''}{repeat}-{role}"
                record = harness_run(args.viewer, arguments, out / slug, cores=cores, minutes=args.minutes,
                                     label=role if args.repeat == 1 else f"{role} #{repeat}", page=page, role=role, repeat=repeat)
                print(f"{page} {record['label']}: exit {record['exit']}{' (' + record['stopped'] + ')' if record['stopped'] else ''}, "
                      f"{record['seconds']} s, CPU {record['load']['before']['cpu_percent']}% -> {record['load']['after']['cpu_percent']}%", flush=True)
    (out / "phase.json").write_text(json.dumps({"recording": args.recording, "from": args.from_hour, "hours": args.hours,
                                                "speed": args.speed, "refresh_hz": args.refresh_hz, "seeks": args.seeks, "cores": args.cores,
                                                "unmeasurable": missing}, indent=1), encoding="utf-8")
    return node("map", out)


def parse(argv):
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    commands = parser.add_subparsers(dest="command", required=True)
    phase = commands.add_parser("map", help="the map's playback phase, headless")
    phase.add_argument("--viewer", required=True)
    phase.add_argument("--recording", required=True)
    phase.add_argument("--from-hour", type=int, required=True)
    phase.add_argument("--hours", type=int, required=True, help="game hours of head travel")
    phase.add_argument("--speed", type=float, default=1440)
    phase.add_argument("--refresh-hz", type=float, default=60)
    phase.add_argument("--seeks", type=int, default=10)
    phase.add_argument("--cores", default="0x55", help="affinity mask; empty for none")
    phase.add_argument("--repeat", type=int, default=1)
    phase.add_argument("--only", help=f"runs to make, of {','.join(ROLES)} (default {DEFAULT_ROLES})")
    phase.add_argument("--repeat-roles", help="the runs --repeat repeats; all by default")
    phase.add_argument("--oversized-recording")
    phase.add_argument("--oversized-factor")
    phase.add_argument("--minutes", type=float, default=30)
    phase.add_argument("--out", required=True)
    one = commands.add_parser("harness", help="one viewer harness or proof run, pinned and checked whole")
    one.add_argument("--viewer", required=True)
    one.add_argument("--out", required=True, help="a new folder for the run")
    one.add_argument("--cores", default="0x55")
    one.add_argument("--minutes", type=float, default=30)
    one.add_argument("--stop-after", type=float, help="stop the run after this many seconds (its verdict is then partial)")
    one.add_argument("--label")
    one.add_argument("arguments", nargs=argparse.REMAINDER)
    board = commands.add_parser("dashboard", help="judge a desktop app's perf log")
    board.add_argument("--perf-log", required=True)
    board.add_argument("--out", required=True)
    board.add_argument("--refresh-hz")
    board.add_argument("--page")
    board.add_argument("--playing-seconds")
    board.add_argument("--seeks")
    args = parser.parse_args(argv)
    if args.command == "map":
        for name in ("from_hour", "hours", "repeat", "seeks"):
            if getattr(args, name) < (0 if name in ("from_hour", "seeks") else 1):
                parser.error(f"--{name.replace('_', '-')} is out of range")
    if args.command == "harness":
        args.arguments = [value for value in args.arguments if value != "--"] if args.arguments[:1] == ["--"] else args.arguments
        if "--out" in args.arguments:
            parser.error("the runner gives the viewer its --out")
    return args


def main(argv):
    args = parse(argv)
    try:
        if args.command == "map":
            return map_phase(args)
        if args.command == "harness":
            folder = Path(args.out).resolve()
            record = harness_run(args.viewer, args.arguments, folder, cores=int(args.cores, 0) if args.cores else 0,
                                 minutes=args.minutes, stop_after=args.stop_after, label=args.label)
            print(f"{record['label']}: exit {record['exit']}{' (' + record['stopped'] + ')' if record['stopped'] else ''}, pid {record['pid']}, "
                  f"{record['seconds']} s, peak {record['peak']['working_set_mb']} MB, CPU {record['load']['before']['cpu_percent']}% -> {record['load']['after']['cpu_percent']}%")
            return node("verify", folder)
        extra = [value for name in ("refresh_hz", "page", "playing_seconds", "seeks") if getattr(args, name) is not None
                 for value in (f"--{name.replace('_', '-')}", getattr(args, name))]
        return node("dashboard", "--perf-log", Path(args.perf_log).resolve(), "--out", Path(args.out).resolve(), *extra)
    except Exception as error:
        print(f"The playback phase could not run: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
