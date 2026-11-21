"""Measure the map's playback: one paced harness run with its threads' CPU, or a table of finished runs.

    python tools/playback_measure.py run --viewer <viewer.exe> --recording <file> --out <dir> --label <name>
        [--seed-cache <dir>] [--high-priority] [--contend N] -- --from-hour H --hours N --paced S [viewer arguments]
    python tools/playback_measure.py set --viewer <viewer.exe> --recording <file> --out <dir> --runs <runs.json>
        [--seed-cache <dir>] [--only <name>,...] [--of <what was measured>]
    python tools/playback_measure.py table <run dir>... [--against <dir of an earlier set>]

`run` starts `viewer --playback-cost <recording> ... --refresh-hz 60 --out <dir>/<label>` (the arguments after
`--` go to the viewer; `--refresh-hz` may be given there instead) and, from the frame the harness starts playing
to the end of its measured wall time, samples every thread of that process by name and the machine's busy
time, into `<dir>/<label>/threads.json`: the CPU each thread name used, the viewer's cores, and the cores other
processes used beside it. `--high-priority` runs the viewer above other processes, which takes their load out
of the measurement; `--contend N` runs N busy processes beside it for the playback, which puts a known load in.
Only the process started here is sampled, and only the busy processes started here are stopped. Runs take turns:
one waits for a run another checkout started, so two never time each other.

A run measuring the GPU passes `--gpu-pass-times` among the viewer's arguments, which records each group of
passes' GPU microseconds as every frame's `view.gpu_passes`. A run measuring CPU leaves it off: the timestamp
commands cost the frame thread 0.35 ms a frame inside `render-submit` and the graphics driver 1.07 ms a frame
on `stress-b`, so a run that asks for them is not measuring the CPU the window pays.

`set` makes the runs a JSON file names (`{"<name>": {"arguments": [...], "high_priority": true, "contend": 0}}`), each
as `run` would with the name as its label, and writes their rows to `<dir>/rows.json`: a set measured once is the
baseline later sets are read against, so the build before a change is not run again beside every build after it.
`table --against <dir>` prints that set's row of the same name above each run's.

`table` prints one row a run: the cores used, the frame's work, the share of frames whose work alone was longer
than one frame of the run's refresh rate (a frame the display would drop), its thread's CPU, the interval between
frames and the share of them that took more than one and a half of the display's, how far the picture stood
behind the clock in hours and in milliseconds of playback (negative while a publication prepared ahead of the
head is drawn before the head reaches its hour), the publications a second the simulation made and how many of
them put a new hour on screen, the map side's step, and the hour pacing: the share of frames in which the picture's hour (the
oldest hour the map's layers show) did not advance, then the share of frames that showed a new hour for the clock,
for the layers and for the overlay (at 24 hours a second and 60 frames a second the clock itself has a new hour in
40% of frames: read the layers and the overlay against it), how far the overlay stood behind the hour the layers
drew in hours and in milliseconds of playback, the hours the picture advanced by at most, and the longest time
without a new hour.

The lag and `held` are the layers' alone, because playing the layers are the picture's hour and the overlay is
drawn over them a step behind; what the overlay cost the picture is the `overlay behind` pair.

A frame's interval is the one the run recorded (`interval_us`, a windowed run through a real swapchain), else
the time between the starts of consecutive frames (an offscreen run, paced to the refresh clock).
"""

import argparse
import collections
import ctypes
import ctypes.wintypes as wt
import json
import os
import subprocess
import sys
import threading
import time
from pathlib import Path

HIGH_PRIORITY_CLASS = 0x80
RUN_LIMIT = 900
THREAD_QUERY_LIMITED_INFORMATION = 0x0800
TH32CS_SNAPTHREAD = 4


class ThreadEntry(ctypes.Structure):
    _fields_ = [("size", wt.DWORD), ("usage", wt.DWORD), ("thread", wt.DWORD), ("process", wt.DWORD),
                ("base", wt.LONG), ("delta", wt.LONG), ("flags", wt.DWORD)]


def kernel():
    k = ctypes.WinDLL("kernel32", use_last_error=True)
    k.CreateToolhelp32Snapshot.restype = wt.HANDLE
    k.OpenThread.restype = wt.HANDLE
    k.GetThreadDescription.argtypes = [wt.HANDLE, ctypes.POINTER(ctypes.c_wchar_p)]
    k.GetThreadDescription.restype = ctypes.c_long
    return k


def ticks(value):
    return (value.dwHighDateTime << 32 | value.dwLowDateTime) / 1e7


def thread_times(k, pid):
    """Each thread of `pid`: its name and the CPU seconds it has used."""
    found = {}
    snapshot = k.CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0)
    entry = ThreadEntry()
    entry.size = ctypes.sizeof(entry)
    more = k.Thread32First(wt.HANDLE(snapshot), ctypes.byref(entry))
    while more:
        if entry.process == pid:
            handle = k.OpenThread(THREAD_QUERY_LIMITED_INFORMATION, False, entry.thread)
            if handle:
                made, ended, kernel_time, user_time = wt.FILETIME(), wt.FILETIME(), wt.FILETIME(), wt.FILETIME()
                if k.GetThreadTimes(wt.HANDLE(handle), ctypes.byref(made), ctypes.byref(ended), ctypes.byref(kernel_time),
                                    ctypes.byref(user_time)):
                    name = ctypes.c_wchar_p()
                    named = k.GetThreadDescription(wt.HANDLE(handle), ctypes.byref(name)) >= 0 and name.value
                    found[entry.thread] = (name.value if named else "(unnamed)", ticks(kernel_time) + ticks(user_time))
                k.CloseHandle(wt.HANDLE(handle))
        more = k.Thread32Next(wt.HANDLE(snapshot), ctypes.byref(entry))
    k.CloseHandle(wt.HANDLE(snapshot))
    return found


def machine_busy(k):
    idle, kernel_time, user_time = wt.FILETIME(), wt.FILETIME(), wt.FILETIME()
    k.GetSystemTimes(ctypes.byref(idle), ctypes.byref(kernel_time), ctypes.byref(user_time))
    return ticks(kernel_time) + ticks(user_time) - ticks(idle)


def family(name):
    """A thread's name without its number: `map worker 3` is `map worker`."""
    return name.rstrip("0123456789 ").strip() or name


def measure(args):
    if os.name != "nt":
        raise SystemExit("playback_measure.py run samples Windows threads")
    k = kernel()
    out = Path(args.out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    run_folder = out / args.label
    viewer_arguments = list(args.viewer_arguments)
    if "--refresh-hz" not in viewer_arguments:
        viewer_arguments += ["--refresh-hz", "60"]
    environment = dict(os.environ, BATTLE_PLAN_CAPTURES=str(out))
    if args.seed_cache:
        environment["BATTLE_PLAN_PLAYBACK_SEED_CACHE"] = str(Path(args.seed_cache).resolve())
    command = [str(Path(args.viewer).resolve()), "--playback-cost", str(Path(args.recording).resolve()), *viewer_arguments,
               "--out", str(run_folder)]
    k.CreateMutexW.restype = wt.HANDLE
    turn = k.CreateMutexW(None, False, "Global\\battle-plan-playback-measure")
    waited = time.monotonic()
    k.WaitForSingleObject(wt.HANDLE(turn), wt.DWORD(0xFFFFFFFF))
    waited = time.monotonic() - waited
    if waited > 1:
        print(f"waited {waited:.0f} s for another timing run to end")
    process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, env=environment, text=True,
                               errors="replace", creationflags=HIGH_PRIORITY_CLASS if args.high_priority else 0)
    playing = threading.Event()
    lines = []

    def read():
        for line in process.stdout:
            if "Playback cost: frame 0," in line:
                playing.set()
            if "content coverage policy" not in line:
                lines.append(line.rstrip())

    threading.Thread(target=read, daemon=True).start()
    busy = []
    samples = []
    start = None
    began = time.monotonic()
    while process.poll() is None:
        if time.monotonic() - began > args.limit:
            process.kill()
            process.wait()
            for child in busy:
                child.kill()
            print(f"{args.label}: the viewer had not ended after {args.limit} s and was stopped", file=sys.stderr)
            return 1
        if playing.is_set():
            if start is None:
                start = time.monotonic()
                busy = [subprocess.Popen([sys.executable, "-c", "while True: pass"]) for _ in range(args.contend)]
            samples.append((time.monotonic() - start, thread_times(k, process.pid), machine_busy(k)))
        time.sleep(0.25)
    for child in busy:
        child.kill()
    print("\n".join(lines[-3:]))
    if run_folder.is_dir():
        (run_folder / "viewer.log").write_text("\n".join(lines) + "\n", encoding="utf-8")
    report_path = run_folder / "report.json"
    if not report_path.is_file() or not samples:
        print("The run wrote no report", file=sys.stderr)
        return process.returncode or 1
    wall = json.loads(report_path.read_text(encoding="utf-8"))["wall_seconds"]
    window = [sample for sample in samples if sample[0] <= wall] or samples[:1]
    first, last = window[0], window[-1]
    seconds = max(last[0] - first[0], 1e-9)
    final = {}
    for _, threads, _ in window:
        final.update(threads)
    used, counts = collections.Counter(), collections.Counter()
    for thread, (name, cpu) in final.items():
        used[family(name)] += cpu - first[1].get(thread, (name, 0.0))[1]
        counts[family(name)] += 1
    total = sum(used.values())
    everything = last[2] - first[2]
    record = {"seconds": seconds, "cores": total / seconds, "other_processes_cores": (everything - total) / seconds,
              "threads": len(final), "high_priority": args.high_priority, "contend": args.contend,
              "by_name": [[name, round(cpu, 3), counts[name]] for name, cpu in used.most_common()]}
    (run_folder / "threads.json").write_text(json.dumps(record, indent=1), encoding="utf-8")
    print(f"{record['cores']:.2f} cores over {len(final)} threads; other processes {record['other_processes_cores']:.1f} cores")
    for name, cpu in used.most_common(12):
        print(f"  {name:32s} {cpu / seconds:5.2f} cores  x{counts[name]}")
    return process.returncode


def percentile(values, share):
    values = sorted(values)
    return values[min(len(values) - 1, int(len(values) * share))] if values else float("nan")


def intervals(frames):
    """Each measured frame's interval in milliseconds: the run's own, else between frame starts."""
    recorded = [frame["interval_us"] / 1e3 for frame in frames if frame.get("interval_us") is not None]
    if recorded:
        return recorded
    starts = [frame["start_us"] for frame in frames if frame.get("start_us") is not None]
    return [(after - before) / 1e3 for before, after in zip(starts, starts[1:])]


def row(folder):
    report = json.loads((folder / "report.json").read_text(encoding="utf-8"))
    paced = report["paced"]
    frames = [json.loads(line) for line in (folder / "frames.jsonl").read_text(encoding="utf-8").splitlines()][paced["warmup_frames"]:]
    threads = json.loads((folder / "threads.json").read_text(encoding="utf-8")) if (folder / "threads.json").is_file() else {}
    work = [frame["work_us"] / 1e3 for frame in frames]
    gaps = intervals(frames)
    refresh = paced.get("refresh_hz") or 0.0
    late = 1.5e3 / refresh if refresh > 0 else float("inf")
    over = 100 * sum(1 for gap in gaps if gap > late) / len(gaps) if gaps else float("nan")
    budget = 1e3 / refresh if refresh > 0 else float("inf")
    dropped = 100 * sum(1 for cost in work if cost > budget) / len(work) if work else float("nan")
    cpu = [frame["thread_us"] / 1e3 for frame in frames if frame.get("thread_us") is not None]
    oldest = [frame["oldest_hour"] for frame in frames if frame.get("oldest_hour") is not None]
    advance = [after - before for before, after in zip(oldest, oldest[1:])]
    held = 100 * sum(1 for hours in advance if hours == 0) / len(advance) if advance else float("nan")

    def moved(hours):
        pairs = [(before, after) for before, after in zip(hours, hours[1:]) if before is not None and after is not None]
        return f"{100 * sum(1 for before, after in pairs if after != before) / len(pairs):.0f}" if pairs else "-"

    clock = [int(frame["head_hour"]) if frame.get("head_hour") is not None else None for frame in frames]
    parts = " / ".join(moved(hours) for hours in (clock, [frame.get("drawn_hour") for frame in frames],
                                                  [(frame.get("layers") or {}).get("overlay_hour") for frame in frames]))
    lag = paced["lag_hours"]
    late_ms = paced.get("late_ms") or {}
    publications = paced.get("publications_per_second")
    fresh = paced.get("publications_with_a_new_hour_per_second")
    rate = f"{publications:.0f} / {fresh:.0f}" if publications is not None and fresh is not None else "-"
    behind = paced.get("overlay_behind_hours") or {}
    behind_ms = paced.get("overlay_behind_ms") or {}

    def three(values, unit=""):
        if not values or values.get("p50") is None:
            return "-"
        return " / ".join(f"{values[part]:.0f}{unit}" for part in ("p50", "p99", "max"))

    step = (report.get("layers") or {}).get("side_step_ms") or {}
    stepped = f"{step['p50']:.1f} / {step['p90']:.1f}" if "p90" in step else "-"
    return [folder.name, f"{paced['speed_hours_per_second']:g}", f"{threads.get('cores', float('nan')):.2f}",
            f"{threads.get('other_processes_cores', float('nan')):.1f}",
            f"{percentile(work, .5):.1f} / {percentile(work, .99):.1f} / {max(work, default=float('nan')):.0f}", f"{dropped:.1f}%",
            f"{percentile(cpu, .5):.1f} / {percentile(cpu, .99):.1f}",
            f"{percentile(gaps, .5):.1f} / {percentile(gaps, .95):.1f} / {percentile(gaps, .99):.1f} / {max(gaps) if gaps else float('nan'):.0f}",
            f"{over:.1f}%",
            f"{lag['p50'] or 0:.0f} / {lag['max'] or 0:.0f}", three(late_ms), rate, stepped, f"{held:.0f}%", parts,
            three(behind), three(behind_ms),
            f"{percentile(advance, .95):.0f} / {max(advance) if advance else 0:.0f}",
            f"{(paced.get('longest_without_a_new_hour_ms') or 0):.0f}"]


HEADS = ["run", "h/s", "cores", "others", "work ms p50/p99/max", "work over a frame", "frame CPU ms p50/p99",
         "interval ms p50/p95/p99/max", "over 1.5x", "lag h p50/max", "late ms p50/p99/max", "publications/s all/new", "step ms p50/p90",
         "held", "new hour % clock/layers/overlay", "overlay behind h p50/p99/max",
         "overlay behind ms p50/p99/max", "advance h p95/max", "longest ms"]


def measure_set(args):
    runs = json.loads(Path(args.runs).read_text(encoding="utf-8"))
    only = set(args.only.split(",")) if args.only else set(runs)
    unknown = only - set(runs)
    if unknown:
        raise SystemExit(f"{args.runs} names no run {sorted(unknown)}")
    out = Path(args.out).resolve()
    taken = [name for name in runs if name in only and (out / name).exists()]
    if taken:
        raise SystemExit(f"{out} already holds {taken}: a set is measured into a folder of its own")
    rows = {}
    for name, run in runs.items():
        if name not in only:
            continue
        code = measure(argparse.Namespace(viewer=args.viewer, recording=args.recording, out=args.out, label=name,
                                          seed_cache=args.seed_cache, high_priority=run.get("high_priority", False),
                                          contend=run.get("contend", 0), limit=run.get("limit", RUN_LIMIT),
                                          viewer_arguments=list(run["arguments"])))
        if not (out / name / "threads.json").is_file():
            print(f"{name}: the viewer exited {code} and left no report", file=sys.stderr)
            return code or 1
        if code:
            print(f"{name}: the viewer exited {code} after its report (a check of its own failed: see its last lines above)")
        rows[name] = row(out / name)
    (out / "rows.json").write_text(json.dumps({"of": args.of or out.name, "heads": HEADS, "rows": rows}, indent=1), encoding="utf-8")
    return table(argparse.Namespace(runs=[str(out / name) for name in rows], against=None))


def table(args):
    heads = HEADS
    rows = []
    earlier = json.loads((Path(args.against) / "rows.json").read_text(encoding="utf-8")) if args.against else None
    for folder in args.runs:
        made = row(Path(folder).resolve())
        if earlier and made[0] in earlier["rows"]:
            before = dict(zip(earlier["heads"], earlier["rows"][made[0]]))
            rows.append([f"{made[0]} of {earlier['of']}", *(before.get(head, "-") for head in heads[1:])])
        rows.append(made)
    widths = [max(len(str(cell)) for cell in column) for column in zip(heads, *rows)]
    for cells in [heads, ["-" * width for width in widths], *rows]:
        print("| " + " | ".join(str(cell).ljust(width) for cell, width in zip(cells, widths)) + " |")
    return 0


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    commands = parser.add_subparsers(dest="command", required=True)
    run = commands.add_parser("run")
    run.add_argument("--viewer", required=True)
    run.add_argument("--recording", required=True)
    run.add_argument("--out", required=True, help="the folder the run's own folder is made in")
    run.add_argument("--label", required=True)
    run.add_argument("--seed-cache")
    run.add_argument("--high-priority", action="store_true")
    run.add_argument("--contend", type=int, default=0)
    run.add_argument("--limit", type=int, default=RUN_LIMIT, help="seconds after which a viewer still running is stopped")
    run.add_argument("viewer_arguments", nargs=argparse.REMAINDER)
    several = commands.add_parser("set")
    several.add_argument("--viewer", required=True)
    several.add_argument("--recording", required=True)
    several.add_argument("--out", required=True, help="a folder of this set's own")
    several.add_argument("--runs", required=True, help="the JSON file naming the runs")
    several.add_argument("--seed-cache")
    several.add_argument("--only")
    several.add_argument("--of", help="what the set is of, as later tables name it (default: the folder's name)")
    listing = commands.add_parser("table")
    listing.add_argument("runs", nargs="+")
    listing.add_argument("--against")
    args = parser.parse_args(argv)
    if args.command == "table":
        return table(args)
    if args.command == "set":
        return measure_set(args)
    if args.viewer_arguments[:1] == ["--"]:
        args.viewer_arguments = args.viewer_arguments[1:]
    return measure(args)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
