"""Synthetic-data demo + real-data CLI for the replay time-lapse renderer.

Runs with NO real replay data: it fabricates 3 frames showing a conquest
sweeping across a few states, renders the PNG sequence + the HTML scrubber, so
the whole pipeline is exercisable before ``replay/SCHEMA.md`` or live snapshots
exist. It also (if present) builds frames from the real ``save_oracle/output``
single snapshot so you can eyeball the renderer against actual data.

Usage:
    python -m replay_render.demo                 # synthetic 3-frame demo
    python -m replay_render.demo --real          # also use save_oracle output
    python -m replay_render.demo --divisions     # include the division layer
    python -m replay_render.demo --full-res      # full-res raster (heavier)

Outputs to replay_render/output/ (gitignored). Open the HTML via the existing
loopback server -- it roots at the repo so /map_pipeline/... assets resolve:
    python map_view/serve.py
    http://127.0.0.1:8777/replay_render/output/<file>.html
(or pass --embed to inline the raster so the HTML is portable.)
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import List

from .frames import Frame, DivisionMarker, frame_from_ownership_dict
from .geometry import load_geometry
from .timelapse import build_timelapse
from .scrubber import build_scrubber_html


def _repo_root() -> Path:
    return Path(__file__).resolve().parent.parent


def _output_dir() -> Path:
    return Path(__file__).resolve().parent / "output"


# A few real European state ids (from state_meta) so the synthetic frames land
# on recognisable territory: 4/5/9 GER, 6 BEL, 7 ENG-held, 10 POL, 14 FRA, 3 SWI.
_RED = (200, 60, 50)
_BLUE = (60, 110, 210)
_GREEN = (70, 170, 90)
_GREY = (150, 150, 150)


def synthetic_frames() -> List[Frame]:
    """3 frames: BLUE conquers RED's states one by one; one stays contested."""
    states = [3, 4, 5, 6, 7, 9, 10, 14]
    # frame 0: all RED owner, all RED control
    f0 = Frame(
        timestamp="1936.01",
        owner={s: _RED for s in states},
        controller={s: "_red" for s in states},  # tag-form ignored; literal owners used
    )
    # Because owners are literal colours, controller hatch needs literal too; for
    # the demo we instead show ownership flips (cleaner with literal colours).
    f0 = Frame(timestamp="1936.01", owner={s: _RED for s in states})
    # frame 1: BLUE took 4,5,6; 10 contested (owner RED, controller BLUE)
    own1 = {s: _RED for s in states}
    own1[4] = _BLUE; own1[5] = _BLUE; own1[6] = _BLUE
    f1 = Frame(timestamp="1938.06", owner=own1)
    # frame 2: BLUE took most; GREEN took 14; 3 stays RED
    own2 = {s: _BLUE for s in states}
    own2[3] = _RED; own2[14] = _GREEN
    f2 = Frame(timestamp="1940.09", owner=own2)

    # synthetic division markers (province ids in those states; strength varies)
    # provinces 4 GER-area etc.; use a couple of arbitrary real province ids.
    divs2 = [
        DivisionMarker(province_id=4, owner="#3c6ed2", strength=200, label="armor"),
        DivisionMarker(province_id=5, owner="#3c6ed2", strength=80, label="infantry"),
        DivisionMarker(province_id=14, owner="#46aa5a", strength=120, label="infantry"),
    ]
    f2.divisions = divs2
    return [f0, f1, f2]


def real_frames() -> List[Frame]:
    """Build a 1-frame 'time-lapse' from the actual save_oracle snapshot, if present."""
    out = _repo_root() / "save_oracle" / "output"
    own_path = out / "state_ownership.json"
    if not own_path.exists():
        print("[demo] no save_oracle/output/state_ownership.json -- skipping --real")
        return []
    ownership = json.loads(own_path.read_text(encoding="utf-8"))
    meta = {}
    mp = out / "meta.json"
    if mp.exists():
        meta = json.loads(mp.read_text(encoding="utf-8"))
    ts = meta.get("save_date", "save")
    divisions = None
    dp = out / "divisions.json"
    if dp.exists():
        try:
            divisions = json.loads(dp.read_text(encoding="utf-8")).get("divisions")
        except Exception:  # noqa: BLE001
            divisions = None
    # province-level control (province_control.json) -> intra-state fronts
    control_by_prov = None
    pcp = out / "province_control.json"
    if pcp.exists():
        try:
            control_by_prov = json.loads(pcp.read_text(encoding="utf-8"))
        except Exception:  # noqa: BLE001
            control_by_prov = None
    return [frame_from_ownership_dict(ts, ownership, divisions,
                                      control_by_prov=control_by_prov)]


def main():
    ap = argparse.ArgumentParser(description="replay_render synthetic/real demo")
    ap.add_argument("--real", action="store_true", help="also render the real save_oracle snapshot")
    ap.add_argument("--divisions", action="store_true", help="render the division-marker layer")
    ap.add_argument("--full-res", action="store_true", help="use the full-res raster")
    ap.add_argument("--embed", action="store_true", help="inline the raster in the HTML (portable)")
    ap.add_argument("--no-gif", action="store_true", help="skip GIF assembly")
    args = ap.parse_args()

    half = not args.full_res
    show_div = args.divisions

    print("[demo] loading geometry (read-only map_pipeline assets)...")
    geom = load_geometry(half_res=half, with_centroids=show_div)
    print(f"[demo] raster {geom.width}x{geom.height}, {len(geom.state_meta)} states, "
          f"{len(geom.country_colors)} country colours, "
          f"{len(geom.prov_centroids)} province centroids")

    # ---- synthetic ----
    sframes = synthetic_frames()
    print(f"[demo] rendering {len(sframes)} synthetic frames...")
    man = build_timelapse(sframes, geom=geom, name="synthetic",
                          show_front_lines=True, show_divisions=show_div,
                          make_gif=not args.no_gif)
    print(f"[demo] synthetic manifest: {man['frame_count']} frames, "
          f"gif={man['gif']} ({man.get('gif_skipped_reason')})")
    html = build_scrubber_html(sframes, _output_dir() / "synthetic_scrubber.html",
                               title="Battle Plan (synthetic demo)",
                               embed_raster=args.embed, show_divisions=show_div,
                               half_res=half)
    print(f"[demo] synthetic scrubber: {html}")

    # ---- real (optional) ----
    if args.real:
        rframes = real_frames()
        if rframes:
            print(f"[demo] rendering {len(rframes)} real frame(s)...")
            rman = build_timelapse(rframes, geom=geom, name="real_snapshot",
                                   show_front_lines=True, show_divisions=show_div,
                                   make_gif=False)
            print(f"[demo] real manifest: {rman['frame_count']} frame(s)")
            rhtml = build_scrubber_html(rframes, _output_dir() / "real_scrubber.html",
                                        title="Battle Plan (real snapshot)",
                                        embed_raster=args.embed,
                                        show_divisions=show_div, half_res=half)
            print(f"[demo] real scrubber: {rhtml}")

    print("\n[demo] done. Open the HTML via:  python map_view/serve.py")
    print("       http://127.0.0.1:8777/replay_render/output/synthetic_scrubber.html")
    print("       (PNG frames + manifest.json under replay_render/output/<name>/)")


if __name__ == "__main__":
    main()
