"""build_campaign_replay.py - first MULTI-POINT replay over one campaign.

Diffs two coherent save-points of the SAME playthrough so the map animates and
the diff engine emits real war-progression EVENTS:

    save_oracle/output_1940/   (1940.9.10.13 - fascist Germany at war with Poland)
    save_oracle/output/        (1946.5.18.12 - the validated DDR/communist world)

Pipeline (mirrors build_replay.py, but ingests TWO oracle dirs into ONE game):
  1. PREFLIGHT  - validate BOTH oracle dirs are individually coherent (single
                  save each). Refuse an incoherent source unless --allow-stale-mix.
  2. INGEST     - normalize each dir into a canonical snapshot and write both into
                  one game_id ("campaign"); they key on hour_index so the 1940 and
                  1946 snapshots coexist and sort chronologically.
  3. TIMELINE   - diff the ordered pair -> events; write timeline.json + a by-type
                  histogram (the payoff: state flips, country lifecycle, wars,
                  factions, divisions, focuses, tech, ...).
  4. RENDER     - both snapshots -> Frames -> a 2-frame PNG time-lapse + HTML
                  scrubber under replay_render/output/campaign/.

Read-only w.r.t. the oracle outputs and every replay/render source. Writes only
under replay/snapshots/<game_id>/ and replay_render/output/<game_id>/ (gitignored).
stdlib + numpy (render only).

Usage:
  python build_campaign_replay.py
  python build_campaign_replay.py --game-id campaign --no-render
  python build_campaign_replay.py --dir-1940 ... --dir-1946 ... --allow-stale-mix
"""
from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

DEFAULT_1940 = os.path.join(HERE, "save_oracle", "output_1940")
DEFAULT_1946 = os.path.join(HERE, "save_oracle", "output")


def preflight(output_dir: str, label: str, allow_stale: bool) -> bool:
    """Validate one oracle dir; return True if it is a single coherent source."""
    argv = [sys.executable, os.path.join(HERE, "save_oracle", "validate.py"),
            "--output-dir", output_dir]
    if allow_stale:
        argv.append("--allow-stale-mix")
    print("-" * 70 + f"\n[1/4] PREFLIGHT - {label} ({os.path.relpath(output_dir, HERE)})\n" + "-" * 70)
    rc = subprocess.run(argv, cwd=HERE).returncode
    if rc != 0:
        print(f"\n[ABORT] preflight FAILED for {label} (incoherent/stale dir). "
              "Regenerate it from a single save, or pass --allow-stale-mix.")
        return False
    return True


def main(argv=None):
    ap = argparse.ArgumentParser(description="Two-point the game campaign replay (1940 -> 1946).")
    ap.add_argument("--dir-1940", default=DEFAULT_1940, help="earlier oracle output dir")
    ap.add_argument("--dir-1946", default=DEFAULT_1946, help="later oracle output dir")
    ap.add_argument("--game-id", default="campaign")
    ap.add_argument("--allow-stale-mix", action="store_true")
    ap.add_argument("--no-render", action="store_true")
    ap.add_argument("--no-scrubber", action="store_true")
    ap.add_argument("--divisions", action="store_true", help="draw division markers")
    ap.add_argument("--keep-existing", action="store_true",
                    help="do NOT clear the game's snapshot dir before ingesting")
    args = ap.parse_args(argv)

    try:
        sys.stdout.reconfigure(encoding="utf-8")
    except Exception:  # noqa: BLE001
        pass

    # --- 1. preflight: BOTH dirs must each be a single coherent save ----------
    print("=" * 70 + "\nCAMPAIGN REPLAY - two save-points of one playthrough\n" + "=" * 70)
    if not preflight(args.dir_1940, "1940 source", args.allow_stale_mix):
        return 1
    if not preflight(args.dir_1946, "1946 source", args.allow_stale_mix):
        return 1

    from replay.store import ingest_oracle_dir, SnapshotStore

    # Start from a clean game dir so a re-run can't leave a stale 3rd snapshot
    # (each snapshot is keyed by hour_index; clearing keeps the pair canonical).
    store = SnapshotStore()
    gdir = store.game_dir(args.game_id)
    if os.path.isdir(gdir) and not args.keep_existing:
        shutil.rmtree(gdir)
        print(f"\n[reset] cleared existing {os.path.relpath(gdir, HERE)}")

    # --- 2. ingest both oracle dirs into ONE game_id --------------------------
    print("\n" + "=" * 70 + "\n[2/4] INGEST - two oracle dirs -> snapshots (one game)\n" + "=" * 70)
    for output_dir, era in ((args.dir_1940, "1940"), (args.dir_1946, "1946")):
        snap, path = ingest_oracle_dir(output_dir, args.game_id)
        print(f"  [{era}] snapshot @ hour {snap.get('hour_index')} ({snap.get('date')}) "
              f"-> {os.path.relpath(path, HERE)}")
        print(f"        countries={len(snap.get('countries') or {})} "
              f"states={len(snap.get('states') or {})} "
              f"provinces={len(snap.get('provinces') or {})} "
              f"divisions={len(snap.get('divisions') or [])} "
              f"combats={len(snap.get('combats') or [])}")

    # --- 3. timeline: diff the ordered pair -> events -------------------------
    from replay.diff_engine import build_timeline
    snaps = store.load_all_ordered(args.game_id)
    print("\n" + "=" * 70 + f"\n[3/4] TIMELINE - diff {len(snaps)} snapshot(s) -> events\n" + "=" * 70)
    if len(snaps) < 2:
        print("  [WARN] fewer than 2 snapshots in the game; nothing to diff.")
    order = " -> ".join(f"{s.get('date')}(h{s.get('hour_index')})" for s in snaps)
    print(f"  order: {order}")

    events = build_timeline(snaps)
    tl_path = store.write_timeline(args.game_id, events)

    by_type: dict = {}
    for e in events:
        t = e.get("type") or e.get("event")
        by_type[t] = by_type.get(t, 0) + 1
    print(f"  {len(events)} events -> {os.path.relpath(tl_path, HERE)}")
    if by_type:
        print("\n  EVENT HISTOGRAM (1940 -> 1946), by type:")
        width = max((len(k or "") for k in by_type), default=0)
        for k, v in sorted(by_type.items(), key=lambda kv: (-kv[1], kv[0])):
            print(f"    {str(k).ljust(width)}  {v}")
    elif len(snaps) == 1:
        print("  (single snapshot: no adjacent pair to diff)")

    # --- 4. render: 2-frame time-lapse + scrubber -----------------------------
    if args.no_render:
        print("\n[4/4] RENDER - skipped (--no-render)")
        print("\nDONE.")
        return 0

    from pathlib import Path
    from replay_render.frames import frame_from_replay_snapshot
    from replay_render.timelapse import build_timelapse
    print("\n" + "=" * 70 + "\n[4/4] RENDER - frames -> time-lapse (+ scrubber)\n" + "=" * 70)
    frames = [frame_from_replay_snapshot(s) for s in snaps]
    out_dir = os.path.join(HERE, "replay_render", "output")
    manifest = build_timelapse(frames, out_dir=Path(out_dir),
                               name=args.game_id, show_divisions=args.divisions,
                               progress=True)
    nframes = len(manifest.get("frames") or [])
    print(f"  {nframes} frame(s) -> {os.path.relpath(out_dir, HERE)}/{args.game_id}/")

    if not args.no_scrubber:
        from replay_render.scrubber import build_scrubber_html
        scr = build_scrubber_html(frames, Path(out_dir) / f"{args.game_id}_scrubber.html",
                                  show_divisions=args.divisions)
        print(f"  scrubber -> {os.path.relpath(str(scr), HERE)}")

    print("\nDONE.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
