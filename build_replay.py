"""build_replay.py - compose the whole offline stack into one command.

Pipeline:
  1. PREFLIGHT  - run save_oracle/validate.py; refuse to build from an incoherent
                  output dir (stale save mix) unless --allow-stale-mix.
  2. INGEST     - replay.store: oracle output dir -> canonical snapshot -> store.
  3. TIMELINE   - replay.diff_engine: diff the ordered snapshot series -> events.
  4. RENDER     - replay_render: snapshots -> Frames -> PNG time-lapse + HTML scrubber.

A "replay" is a series of snapshots over time; with a single save you get a
one-frame replay (this still proves the whole stack composes). Drop more saves
(or a live-memory snapshot feed) into the same game_id and re-run to extend it.

Read-only w.r.t. the oracle output; writes only under replay/snapshots/ and
replay_render/output/ (both gitignored). stdlib + numpy (render only).

Usage:
  python build_replay.py [--output-dir DIR] [--game-id ID] [--allow-stale-mix]
                         [--no-render] [--no-scrubber] [--divisions]
"""
from __future__ import annotations

import argparse
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)


def preflight(output_dir: str, allow_stale: bool) -> bool:
    """Run the validator as a subprocess; return True if safe to proceed."""
    argv = [sys.executable, os.path.join(HERE, "save_oracle", "validate.py"),
            "--output-dir", output_dir]
    if allow_stale:
        argv.append("--allow-stale-mix")
    print("=" * 70 + "\n[1/4] PREFLIGHT - save_oracle/validate.py\n" + "=" * 70)
    rc = subprocess.run(argv, cwd=HERE).returncode
    if rc != 0:
        print("\n[ABORT] preflight FAILED (incoherent/stale output dir). "
              "Regenerate save_oracle/output from one save, or pass --allow-stale-mix "
              "to build a best-effort replay anyway.")
        return False
    return True


def main(argv=None):
    ap = argparse.ArgumentParser(description="Compose the offline Battle Plan stack.")
    ap.add_argument("--output-dir", default=os.path.join(HERE, "save_oracle", "output"))
    ap.add_argument("--game-id", default="replay")
    ap.add_argument("--allow-stale-mix", action="store_true")
    ap.add_argument("--no-render", action="store_true")
    ap.add_argument("--no-scrubber", action="store_true")
    ap.add_argument("--divisions", action="store_true", help="draw division markers")
    args = ap.parse_args(argv)

    if not preflight(args.output_dir, args.allow_stale_mix):
        return 1

    # --- 2. ingest -------------------------------------------------------------
    from replay.store import ingest_oracle_dir, SnapshotStore
    from replay.diff_engine import build_timeline
    print("\n" + "=" * 70 + "\n[2/4] INGEST - oracle -> snapshot -> store\n" + "=" * 70)
    snap, path = ingest_oracle_dir(args.output_dir, args.game_id)
    print(f"  snapshot @ hour {snap.get('hour_index')} ({snap.get('date')}) -> {os.path.relpath(path, HERE)}")
    print(f"  countries={len(snap.get('countries') or {})} states={len(snap.get('states') or {})} "
          f"provinces={len(snap.get('provinces') or {})} divisions={len(snap.get('divisions') or [])} "
          f"combats={len(snap.get('combats') or [])}")

    # --- 3. timeline -----------------------------------------------------------
    store = SnapshotStore()
    snaps = store.load_all_ordered(args.game_id)
    print("\n" + "=" * 70 + f"\n[3/4] TIMELINE - diff {len(snaps)} snapshot(s) -> events\n" + "=" * 70)
    events = build_timeline(snaps)
    tl_path = store.write_timeline(args.game_id, events)
    by_type = {}
    for e in events:
        by_type[e.get("type") or e.get("event")] = by_type.get(e.get("type") or e.get("event"), 0) + 1
    print(f"  {len(events)} events -> {os.path.relpath(tl_path, HERE)}")
    if by_type:
        print("  " + ", ".join(f"{k}={v}" for k, v in sorted(by_type.items())))
    elif len(snaps) == 1:
        print("  (single snapshot: no adjacent pair to diff; add more saves to populate events)")

    # --- 4. render -------------------------------------------------------------
    if args.no_render:
        print("\n[4/4] RENDER - skipped (--no-render)")
        print("\nDONE.")
        return 0

    from replay_render.frames import frame_from_replay_snapshot
    from replay_render.timelapse import build_timelapse
    print("\n" + "=" * 70 + "\n[4/4] RENDER - frames -> time-lapse (+ scrubber)\n" + "=" * 70)
    frames = [frame_from_replay_snapshot(s) for s in snaps]
    out_dir = os.path.join(HERE, "replay_render", "output")
    manifest = build_timelapse(frames, out_dir=__import__("pathlib").Path(out_dir),
                               name=args.game_id, show_divisions=args.divisions,
                               progress=True)
    nframes = len(manifest.get("frames") or [])
    print(f"  {nframes} frame(s) -> {os.path.relpath(out_dir, HERE)}/{args.game_id}/")

    if not args.no_scrubber:
        from pathlib import Path
        from replay_render.scrubber import build_scrubber_html
        scr = build_scrubber_html(frames, Path(out_dir) / f"{args.game_id}_scrubber.html",
                                  show_divisions=args.divisions)
        print(f"  scrubber -> {os.path.relpath(str(scr), HERE)}")

    print("\nDONE.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
