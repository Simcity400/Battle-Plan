"""The map's golden frames: a fixed set of window-path frames, compared byte for byte.

    python tools/map_golden.py capture --viewer <viewer.exe> --recording <file> --first-hour H --last-hour H
        --out <dir> [--only name,...] [--seed-cache <dir>]
    python tools/map_golden.py compare --baseline <dir> --candidate <dir> [--viewer <viewer.exe>]

`capture` runs the viewer's stepped pixel-proof harness (`--playback-cost --speed max --png-frame`) once per
scene: each loads the recording once and saves a few whole hours as the window draws them, which are the
frames `--dump` writes. A scene's hours are shares of the history between `--first-hour` and `--last-hour`
(the harness names a history's coverage when a run asks past it), so any recording serves. `<dir>/golden.json` lists every frame's SHA-256. `compare` fails on any frame that differs or is
missing, and with `--viewer` writes each differing pair's diff image beside the candidate's frame.
A change to the map's data flow, threading or frame loop leaves every frame identical; a change that is meant
to move pixels says which scenes moved and why.
"""

import argparse
import hashlib
import json
import os
import subprocess
import sys
from pathlib import Path

FRAMES = 3
SCENES = (
    ("world", 0.60, 240, []),
    ("world-early", 0.05, 24, []),
    ("front-close", 0.60, 240, ["--camera", "3200,1500,110"]),
    ("front-mid", 0.75, 48, ["--camera", "3200,1500,400"]),
    ("west-close", 0.40, 1, ["--camera", "2900,1400,90"]),
    ("states", 0.60, 240, ["--map-mode", "states"]),
    ("navy", 0.60, 240, ["--map-mode", "strategic_navy", "--camera", "2900,1300,700"]),
    ("air", 0.60, 240, ["--map-mode", "strategic_air", "--camera", "3100,1450,500"]),
    ("resistance", 0.90, 240, ["--map-mode", "resistance", "--camera", "3200,1500,400"]),
    ("no-fog", 0.60, 240, ["--fog", "off", "--camera", "3200,1500,200"]),
)
SIZE = ("--width", "1280", "--height", "720")


def sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as source:
        for block in iter(lambda: source.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def run(viewer, arguments, captures, seed=None):
    environment = dict(os.environ, BATTLE_PLAN_CAPTURES=str(captures))
    if seed:
        environment["BATTLE_PLAN_PLAYBACK_SEED_CACHE"] = str(seed)
    return subprocess.run([str(viewer), *arguments], env=environment, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                          text=True, errors="replace")


def remove(folder):
    if folder.is_dir():
        for path in sorted(folder.rglob("*"), reverse=True):
            path.rmdir() if path.is_dir() else path.unlink()
        folder.rmdir()


def capture(args):
    out = Path(args.out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    viewer, recording = Path(args.viewer).resolve(), Path(args.recording).resolve()
    first, last = args.first_hour, args.last_hour
    wanted = set(args.only.split(",")) if args.only else None
    seed = Path(args.seed_cache).resolve() if args.seed_cache else None
    identity = {"name": recording.name, "bytes": recording.stat().st_size}
    listing = out / "golden.json"
    held = json.loads(listing.read_text(encoding="utf-8")) if listing.is_file() else {}
    frames = dict(held.get("frames", {})) if held.get("recording") == identity else {}
    failed = []
    for name, share, step, extra in SCENES:
        if wanted and name not in wanted:
            continue
        scene = out / name
        remove(scene)
        start = first + int((last - first - step * FRAMES) * share)
        command = ["--playback-cost", str(recording), "--from-hour", str(start), "--hours", str(FRAMES), "--hour-step",
                   str(step), "--speed", "max", "--complete-models", "--refresh-hz", "60", *SIZE, *extra, "--out", str(scene)]
        for index in range(FRAMES):
            command += ["--png-frame", str(index)]
        done = run(viewer, command, out, seed)
        made = sorted(scene.glob("frame-*.png"))
        frames = {frame: digest for frame, digest in frames.items() if frame.split("/")[0] != name}
        if done.returncode != 0 or len(made) != FRAMES:
            last_line = (done.stdout.strip().splitlines() or ["no output"])[-1][:300]
            print(f"{name}: the viewer failed ({done.returncode}): {last_line}", file=sys.stderr)
            failed.append(name)
            continue
        seed = seed or (scene / "cache")
        for frame in made:
            frames[f"{name}/{frame.name}"] = sha256(frame)
        print(f"{name}: {len(made)} frames from hour {start}, every {step}")
    listing.write_text(json.dumps({"recording": identity, "frames": frames}, indent=1), encoding="utf-8")
    if failed:
        print(f"Not captured: {', '.join(failed)}", file=sys.stderr)
    return 1 if failed else 0


def compare(args):
    baseline, candidate = Path(args.baseline).resolve(), Path(args.candidate).resolve()
    held = json.loads((baseline / "golden.json").read_text(encoding="utf-8"))
    made = json.loads((candidate / "golden.json").read_text(encoding="utf-8"))
    if held["recording"] != made["recording"]:
        print(f"Different recordings: {held['recording']} and {made['recording']}", file=sys.stderr)
        return 2
    failed = 0
    for name, digest in sorted(held["frames"].items()):
        if name.split("/")[0] not in {frame.split("/")[0] for frame in made["frames"]}:
            continue
        other = made["frames"].get(name)
        if other == digest:
            continue
        failed += 1
        if other is None:
            print(f"MISSING {name}")
            continue
        line = f"DIFFERS {name}"
        if args.viewer:
            diff = candidate / (name[:-4] + ".diff.png")
            done = subprocess.run([str(Path(args.viewer).resolve()), "--compare", str(baseline / name), str(candidate / name),
                                   "--diff", str(diff)], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, errors="replace")
            summary = [text for text in done.stdout.splitlines() if "differ" in text.lower() or "max" in text.lower()]
            line += f" ({diff}" + (f"; {summary[0].strip()}" if summary else "") + ")"
        print(line)
    scenes = sorted({frame.split("/")[0] for frame in made["frames"]})
    print(f"{len(made['frames'])} frames of {len(scenes)} scenes compared; {failed} differ")
    return 1 if failed else 0


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    commands = parser.add_subparsers(dest="command", required=True)
    take = commands.add_parser("capture")
    take.add_argument("--viewer", required=True)
    take.add_argument("--recording", required=True)
    take.add_argument("--out", required=True)
    take.add_argument("--first-hour", type=int, required=True, help="the first hour of the history the scenes spread over")
    take.add_argument("--last-hour", type=int, required=True, help="its last hour")
    take.add_argument("--only", help="scene names, comma-separated")
    take.add_argument("--seed-cache", help="a finished run's cache folder (its compiled shaders) the first scene starts from")
    check = commands.add_parser("compare")
    check.add_argument("--baseline", required=True)
    check.add_argument("--candidate", required=True)
    check.add_argument("--viewer", help="writes a diff image for each frame that differs")
    args = parser.parse_args(argv)
    return capture(args) if args.command == "capture" else compare(args)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
