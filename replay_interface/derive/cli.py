"""CLI: recompute the derived layers for a stored game (or a loose snapshot file).

Pure offline. Reads stored snapshots from ``replay_interface/replay/snapshots/<game_id>/`` via the
replay ``SnapshotStore`` (read-only import), overlays the derived layers, and
writes the snapshots back (re-validated by the store). ``gross_ic_by_equipment``
is filled where a >=2-snapshot series exists (adjacent monotone delta), else null.

Usage:
    python -m derive.cli <game_id> [--root <snapshots_root>] [--assets <dir>]
                         [--half-res|--full-res] [--dry-run]
    python -m derive.cli --file <snapshot.json> [--prev <prev.json>] [--out <path>]

``--dry-run`` prints the per-layer fill summary without writing.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from typing import Any, Dict, List, Optional

# read-only import of the replay store
_HERE = os.path.dirname(os.path.abspath(__file__))
_REPO = os.path.normpath(os.path.join(_HERE, os.pardir))
if _REPO not in sys.path:
    sys.path.insert(0, _REPO)

from derive import adjacency as _adj            # noqa: E402
from derive.overlay import derive_layers        # noqa: E402


def _load_store():
    from replay.store import SnapshotStore  # read-only use
    return SnapshotStore


def _summ(snap: Dict[str, Any]) -> str:
    d = snap.get("_derived", {}).get("layers", {})
    return json.dumps(d, separators=(",", ":"))


def run_game(game_id: str, *, root: Optional[str], assets_dir: Optional[str],
             half_res: bool, dry_run: bool) -> int:
    SnapshotStore = _load_store()
    store = SnapshotStore(root=root)
    entries = store.list_snapshots(game_id)
    if not entries:
        print(f"[derive] no snapshots for game {game_id!r} under "
              f"{store.game_dir(game_id)}", file=sys.stderr)
        return 2

    # build the static adjacency once (cached on disk by the module)
    adj = _adj.load_or_build_adjacency(assets_dir=assets_dir, half_res=half_res)

    # load all ordered so we can pair adjacent snapshots for gross_ic deltas
    snaps: List[Dict[str, Any]] = store.load_all_ordered(game_id)
    print(f"[derive] game {game_id}: {len(snaps)} snapshot(s)")
    written = 0
    for i, snap in enumerate(snaps):
        prev = snaps[i - 1] if i > 0 else None
        out = derive_layers(snap, prev_snapshot=prev, adj=adj,
                            assets_dir=assets_dir)
        date = out.get("date")
        print(f"  [{i}] {date}: {_summ(out)}")
        if not dry_run:
            path = store.write_snapshot(out)
            written += 1
            print(f"      written -> {os.path.basename(path)}")
    if dry_run:
        print("[derive] dry-run: nothing written")
    else:
        print(f"[derive] wrote {written} snapshot(s)")
    return 0


def run_file(path: str, *, prev: Optional[str], out: Optional[str],
             assets_dir: Optional[str], half_res: bool, dry_run: bool) -> int:
    with open(path, "r", encoding="utf-8") as fh:
        snap = json.load(fh)
    prev_snap = None
    if prev:
        with open(prev, "r", encoding="utf-8") as fh:
            prev_snap = json.load(fh)
    adj = _adj.load_or_build_adjacency(assets_dir=assets_dir, half_res=half_res)
    result = derive_layers(snap, prev_snapshot=prev_snap, adj=adj,
                           assets_dir=assets_dir)
    print(f"[derive] {path}: {_summ(result)}")
    if dry_run:
        print("[derive] dry-run: nothing written")
        return 0
    dest = out or path
    with open(dest, "w", encoding="utf-8") as fh:
        json.dump(result, fh, separators=(",", ":"))
    print(f"[derive] written -> {dest}")
    return 0


def main(argv: Optional[List[str]] = None) -> int:
    ap = argparse.ArgumentParser(description="Recompute derived replay layers.")
    ap.add_argument("game_id", nargs="?", help="stored game id under the snapshots root")
    ap.add_argument("--root", default=None, help="snapshots root (default replay_interface/replay/snapshots)")
    ap.add_argument("--assets", default=None, help="map assets dir (default replay_interface/map_pipeline/assets)")
    res = ap.add_mutually_exclusive_group()
    res.add_argument("--half-res", dest="half_res", action="store_true", default=True,
                     help="use the half-res raster for adjacency (default, lossless)")
    res.add_argument("--full-res", dest="half_res", action="store_false",
                     help="use the full-res raster for adjacency")
    ap.add_argument("--dry-run", action="store_true", help="compute + print, do not write")
    ap.add_argument("--file", default=None, help="recompute a single loose snapshot file")
    ap.add_argument("--prev", default=None, help="prior snapshot for gross_ic delta (with --file)")
    ap.add_argument("--out", default=None, help="output path for --file (default: in place)")
    args = ap.parse_args(argv)

    if args.file:
        return run_file(args.file, prev=args.prev, out=args.out,
                        assets_dir=args.assets, half_res=args.half_res,
                        dry_run=args.dry_run)
    if not args.game_id:
        ap.error("provide a game_id or --file <snapshot.json>")
    return run_game(args.game_id, root=args.root, assets_dir=args.assets,
                    half_res=args.half_res, dry_run=args.dry_run)


if __name__ == "__main__":
    raise SystemExit(main())
