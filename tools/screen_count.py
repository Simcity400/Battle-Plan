"""What a measured run put on the screen: distinct pictures a second, and how evenly the replay advanced.

    python tools/screen_count.py <run dir>...            one block a run
    python tools/screen_count.py --table <run dir>...    one row a run

A timing run says how often the frame thread finished a frame. These are the two things it does not say.

**Distinct pictures a second** comes from a run taken with `--count-pictures <frames>:<every>`, which compares
each counted frame's picture with the frame before it on the GPU (`report.json`'s `pictures`). A frame whose
output differs is a new picture; one whose output differs while the world behind it does not is the overlay's
alone. Counted apart: frames differing by only a texel or two, and frames differing by at least two steps of
1/255 over more than `--seen` texels, which is a change a watcher could see rather than a flicker.

**How evenly the replay advances** comes from every frame's record (`frames.jsonl`), so any run has it: the game
hours the hour drawn moved on by, frame to frame, against the hours a frame is worth at the run's speed and
refresh (1,440 hours a second at 240 Hz is 6 hours a frame). A run that shows the same hour for several frames
and then jumps reads as a share of frames that advanced nothing, a p99 jump far above the ideal, and a longest
hold in milliseconds. Beside it, what the things a watcher follows did: the counters and figures in view that
changed place (`counters_moved`, which the figures share through the same animator), the largest such move in
pixels at the camera's own scale (`pixels_a_world_unit`), and the frames on which nothing in view moved at all.
"""

import argparse
import json
import sys
from pathlib import Path


def percentile(values, share):
    values = sorted(values)
    return values[min(len(values) - 1, int(len(values) * share))] if values else float("nan")


def three(values, scale=1.0, digits=0):
    if not values:
        return "-"
    return " / ".join(
        f"{value * scale:.{digits}f}"
        for value in (percentile(values, 0.5), percentile(values, 0.99), max(values))
    )


def longest_run(values, holds):
    """The longest stretch of consecutive `values` for which `holds`, in entries."""
    longest = run = 0
    for value in values:
        run = run + 1 if holds(value) else 0
        longest = max(longest, run)
    return longest


class Run:
    def __init__(self, folder):
        self.folder = Path(folder).resolve()
        self.report = json.loads((self.folder / "report.json").read_text(encoding="utf-8"))
        paced = self.report.get("paced") or {}
        self.warmup = paced.get("warmup_frames", 0)
        self.speed = paced.get("speed_hours_per_second") or self.report.get("game_hours_per_wall_second")
        self.refresh = paced.get("refresh_hz") or 0.0
        self.frames = [json.loads(line) for line in (self.folder / "frames.jsonl").read_text(encoding="utf-8").splitlines()]
        self.measured = self.frames[self.warmup:]
        threads = self.folder / "threads.json"
        self.threads = json.loads(threads.read_text(encoding="utf-8")) if threads.is_file() else {}
        scale = self.report.get("pixels_a_world_unit") or {}
        self.pixels_a_unit = scale.get("east")

    # the frames a second the run reached
    def fps(self):
        starts = [frame["start_us"] for frame in self.measured if frame.get("start_us") is not None]
        if len(starts) < 2:
            return float("nan")
        return (len(starts) - 1) / ((starts[-1] - starts[0]) / 1e6)

    def work_ms(self):
        return [frame["work_us"] / 1e3 for frame in self.measured if frame.get("work_us") is not None]

    # how evenly the replay advanced: the hours the drawn picture moved on by, frame to frame
    def advance(self):
        hours = [frame.get("drawn_hour") for frame in self.measured]
        pairs = [(before, after) for before, after in zip(hours, hours[1:]) if before is not None and after is not None]
        return [after - before for before, after in pairs]

    def ideal_hours_a_frame(self):
        fps = self.fps()
        return self.speed / fps if self.speed and fps == fps and fps > 0 else float("nan")

    # what the movers in view did, split by whether the picture's hour advanced that frame
    def movers(self):
        """Per measured frame: counters that changed place, the largest glide, rebuilds, icon
        windows' largest move and the figures drawn. A frame whose record names none of them had
        none: the flight record keeps a name only once something adds to it."""
        rows, hours = [], None
        for frame in self.measured:
            view = frame.get("view") or {}
            overlay = view.get("overlay") or {}
            units = view.get("units") or {}
            drawn = frame.get("drawn_hour")
            advanced = None if hours is None or drawn is None else drawn - hours
            if drawn is not None:
                hours = drawn
            rows.append({
                "advanced": advanced,
                "moved": overlay.get("counters_moved", 0),
                "glide": overlay.get("largest_glide_milli", 0) / 1e3,
                "rebuilds": overlay.get("counter_rebuilds", 0) + overlay.get("counter_snaps", 0),
                "window": overlay.get("largest_window_move_px", 0),
                "figures": units.get("figures"),
            })
        return rows

    def pictures(self):
        """Each counted frame: whether the output differed, whether the world did, and by how much."""
        counted = self.report.get("pictures")
        if not counted:
            return None
        rows = []
        for frame in counted["frames"]:
            parts = frame["parts"]
            rows.append({
                "frame": frame["frame"],
                "screen": parts["screen"]["differing"],
                "screen_seen": parts["screen"]["steps_2"],
                "world": parts["world"]["differing"],
                "world_seen": parts["world"]["steps_2"],
                "largest": parts["screen"]["largest"],
                "rectangle": parts["screen"]["rectangle"],
                "tiles": parts["screen"].get("tiles"),
                "tiles_of": parts["screen"].get("tiles_of"),
            })
        return {"counting": counted, "rows": rows}

    def causes(self):
        """Per frame index: what in the replay could have changed the picture that frame."""
        causes, hours = {}, None
        for frame in self.frames:
            view = frame.get("view") or {}
            overlay = view.get("overlay") or {}
            drawn = frame.get("drawn_hour")
            advanced = drawn is not None and hours is not None and drawn != hours
            if drawn is not None:
                hours = drawn
            posed = sum((view.get(part) or {}).get("actors_posed", 0)
                        for part in ("unit models", "city sprawl", "flying aircraft", "map buildings", "supply trains"))
            causes[frame["frame"]] = {
                "hour": advanced,
                "counters": overlay.get("counters_moved", 0) > 0,
                "posed": posed > 0,
                "weather": ((view.get("weather packets") or {}).get("packets", 0)) > 0,
            }
        return causes


def picture_counts(run, seen):
    """Distinct pictures a second, by part, and the longest stretch of identical frames."""
    counted = run.pictures()
    if not counted:
        return None
    rows = counted["rows"]
    fps = run.fps()
    share = lambda n: n / len(rows) if rows else float("nan")
    changed = [row for row in rows if row["screen"] > 0]
    visible = [row for row in rows if row["screen_seen"] > seen]
    overlay_only = [row for row in changed if row["world"] == 0]
    world = [row for row in changed if row["world"] > 0]
    # Identical frames run on inside a window only: a window's first frame has no frame before it.
    indexes = {row["frame"] for row in rows}
    identical = longest_run(
        [row for row in rows],
        lambda row: row["screen"] == 0 and row["frame"] - 1 in indexes,
    )
    causes = run.causes()
    by_cause = {}
    for cause in ("hour", "counters", "posed", "weather"):
        by_cause[cause] = share(sum(1 for row in rows if causes.get(row["frame"], {}).get(cause))) * fps
    local = [row for row in changed if row["tiles"] is not None and row["tiles"] <= 8]
    everywhere = [row for row in changed if row["tiles"] is not None and row["tiles"] >= 64]
    return {
        "counted": len(rows),
        "by_cause": by_cause,
        "local_a_second": share(len(local)) * fps,
        "everywhere_a_second": share(len(everywhere)) * fps,
        "tiles": [row["tiles"] for row in changed if row["tiles"] is not None],
        "tiles_of": next((row["tiles_of"] for row in rows if row.get("tiles_of")), None),
        "fps": fps,
        "distinct_a_second": share(len(changed)) * fps,
        "seen_a_second": share(len(visible)) * fps,
        "world_a_second": share(len(world)) * fps,
        "overlay_only_a_second": share(len(overlay_only)) * fps,
        "identical_share": share(len(rows) - len(changed)),
        "longest_identical": identical + 1 if identical else 0,
        "texels": run.report["viewport"][0] * run.report["viewport"][1],
        "differing": [row["screen"] for row in changed],
        "seen": [row["screen_seen"] for row in changed],
        "largest": [row["largest"] for row in changed],
    }


def block(run, seen):
    print(f"== {run.folder.name}: {run.speed:g} game hours a second, asked for {run.refresh:g} Hz, "
          f"{run.report['viewport'][0]}x{run.report['viewport'][1]}")
    fps = run.fps()
    work = run.work_ms()
    print(f"   frames a second reached {fps:.0f}; the frame's work p50/p99/max {three(work, digits=1)} ms;"
          f" {run.threads.get('cores', float('nan')):.2f} cores ({run.threads.get('other_processes_cores', float('nan')):.1f} others)")
    counts = picture_counts(run, seen)
    if counts:
        print(f"   DISTINCT PICTURES A SECOND {counts['distinct_a_second']:.0f} of {fps:.0f} frames"
              f" (a watcher could see the change in {counts['seen_a_second']:.0f});"
              f" the world {counts['world_a_second']:.0f}, the overlay alone {counts['overlay_only_a_second']:.0f}")
        print(f"   identical to the frame before: {100 * counts['identical_share']:.0f}% of {counts['counted']} counted frames;"
              f" longest stretch of one picture {counts['longest_identical']} frames"
              f" ({1e3 * counts['longest_identical'] / fps:.0f} ms)")
        print(f"   texels differing in a changed frame p50/p99/max {three(counts['differing'])} of {counts['texels']};"
              f" of them by two steps or more {three(counts['seen'])}; largest step {three(counts['largest'])} of 255")
        if counts["tiles"]:
            print(f"   where the seen change was: tiles of {counts['tiles_of']} holding one p50/p99/max {three(counts['tiles'])};"
                  f" a few places (8 or fewer) {counts['local_a_second']:.0f} a second, over most of the view (64 or more) {counts['everywhere_a_second']:.0f}")
        print("   what could have changed it, per second of counted frames: "
              + ", ".join(f"{name} {rate:.0f}" for name, rate in counts["by_cause"].items()))
    else:
        print("   DISTINCT PICTURES: not counted (the run was taken without --count-pictures)")
    advance = run.advance()
    ideal = run.ideal_hours_a_frame()
    held = sum(1 for hours in advance if hours == 0)
    longest = longest_run(advance, lambda hours: hours == 0)
    print(f"   HOURS THE PICTURE ADVANCED A FRAME: ideal {ideal:.1f};"
          f" p50/p99/max {three(advance)}; nothing in {100 * held / len(advance):.0f}% of frames;"
          f" longest hold {longest} frames ({1e3 * longest / fps:.0f} ms)")
    rows = run.movers()
    for name, kept in (("the hour held", lambda row: row["advanced"] == 0),
                       ("the hour advanced", lambda row: (row["advanced"] or 0) > 0)):
        taken = [row for row in rows if row["advanced"] is not None and kept(row)]
        if not taken:
            continue
        moved = [row["moved"] for row in taken]
        still = sum(1 for count in moved if count == 0)
        glides = [row["glide"] for row in taken if row["rebuilds"] == 0]
        scale = run.pixels_a_unit
        pixels = f" = {three(glides, scale or 0, 2)} px" if scale else ""
        print(f"   MOVERS IN VIEW, {name} ({len(taken)} frames): counters that changed place p50/p99/max {three(moved)};"
              f" nothing in view moved in {100 * still / len(taken):.0f}% of them")
        print(f"      largest glide a frame, rebuilds apart, p50/p99/max {three(glides, digits=3)} world units{pixels}"
              f"{f' ({scale:.1f} px a world unit)' if scale else ''}")
    windows = [row["window"] for row in rows]
    if any(windows):
        still = sum(1 for move in windows if move == 0)
        print(f"   icon windows: largest move a frame p50/p99/max {three(windows)} px; none moved in {100 * still / len(windows):.0f}% of frames")
    figures = [row["figures"] for row in rows if row["figures"] is not None]
    if figures:
        print(f"   figures drawn p50/max {percentile(figures, 0.5):.0f} / {max(figures)}")


HEADS = ["run", "h/s", "Hz", "fps", "distinct/s", "seen/s", "world/s", "overlay/s", "identical", "longest same",
         "ideal h", "advance h p50/p99/max", "no advance", "longest hold ms", "counters moved p50/p99/max (hour held)",
         "nothing moved", "glide px p99", "work ms p50/p99", "cores"]


def row(run, seen):
    counts = picture_counts(run, seen)
    fps = run.fps()
    advance = run.advance()
    nothing = 100 * sum(1 for hours in advance if hours == 0) / len(advance) if advance else float("nan")
    longest = longest_run(advance, lambda hours: hours == 0)
    rows = run.movers()
    held = [row for row in rows if row["advanced"] == 0]
    moved = [row["moved"] for row in held]
    glides = [row["glide"] for row in held if row["rebuilds"] == 0]
    still = 100 * sum(1 for count in moved if count == 0) / len(moved) if moved else float("nan")
    glide = percentile(glides, 0.99) * run.pixels_a_unit if glides and run.pixels_a_unit else float("nan")
    work = run.work_ms()
    return [run.folder.name, f"{run.speed:g}", f"{run.refresh:g}", f"{fps:.0f}",
            f"{counts['distinct_a_second']:.0f}" if counts else "-",
            f"{counts['seen_a_second']:.0f}" if counts else "-",
            f"{counts['world_a_second']:.0f}" if counts else "-",
            f"{counts['overlay_only_a_second']:.0f}" if counts else "-",
            f"{100 * counts['identical_share']:.0f}%" if counts else "-",
            f"{counts['longest_identical']}" if counts else "-",
            f"{run.ideal_hours_a_frame():.1f}", three(advance), f"{nothing:.0f}%", f"{1e3 * longest / fps:.0f}",
            three(moved), f"{still:.0f}%", f"{glide:.2f}",
            f"{percentile(work, .5):.1f} / {percentile(work, .99):.1f}",
            f"{run.threads.get('cores', float('nan')):.2f}"]


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("runs", nargs="+")
    parser.add_argument("--table", action="store_true", help="one row a run instead of a block")
    parser.add_argument("--seen", type=int, default=64,
                        help="texels differing by two steps or more above which a frame's change is one a watcher could see")
    args = parser.parse_args(argv)
    runs = [Run(folder) for folder in args.runs]
    if not args.table:
        for run in runs:
            block(run, args.seen)
        return 0
    rows = [row(run, args.seen) for run in runs]
    widths = [max(len(str(cell)) for cell in column) for column in zip(HEADS, *rows)]
    for cells in [HEADS, ["-" * width for width in widths], *rows]:
        print("| " + " | ".join(str(cell).ljust(width) for cell, width in zip(cells, widths)) + " |")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
