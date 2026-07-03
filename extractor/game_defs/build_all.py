#!/usr/bin/env python3
"""Build every game_defs catalog to extractor/game_defs/out/*.json in one pass.

Resolves the layer set + loc ONCE and feeds all parsers (faster than running
each CLI, which re-resolves). Output is the structured static-def data the
replay app consumes for binding-sweep bucket-(c).

Usage:
  BATTLE_PLAN_VANILLA=<install> python extractor/game_defs/build_all.py
  python extractor/game_defs/build_all.py --vanilla <dir> --mods mod/test.mod
"""

from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path

_HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(_HERE))

import resolver as R
import parse_events
import parse_scripted_gui
import parse_tech
import parse_doctrines
import parse_catalogs
import parse_focus_def
import parse_templates
import parse_world_defs
import parse_characters
import parse_localisation
import parse_idea_tech_loc
import parse_decisions
import parse_ship_definitions
import gfx_icon_manifest_gate

GFX_MIN_SPRITES = 1


def _assert_gfx_gate_passed(catalog):
    checks = gfx_icon_manifest_gate.catalog_gate_checks(catalog, min_sprites=GFX_MIN_SPRITES)
    if not all(checks.values()):
        failed = ", ".join(name for name, passed in checks.items() if not passed)
        raise RuntimeError(f"gfx_icon_manifest gate did not pass: {failed}")


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--vanilla", default=None,
                    help="the game install dir (else BATTLE_PLAN_VANILLA env).")
    ap.add_argument("--mods", default=None,
                    help="comma-separated .mod tokens (else active_modset.json).")
    ap.add_argument("--userdata", default=None,
                    help="the game user-data dir used by GFX asset DLC/mod resolution.")
    ap.add_argument("--out", default=str(_HERE / "out"))
    args = ap.parse_args()
    mods = args.mods.split(",") if args.mods else None

    t0 = time.time()
    layers = R.build_layers(vanilla=args.vanilla, mods=mods, userdata=args.userdata)
    loc = R.load_loc(layers)
    print(f"layers={[ly.label for ly in layers]}  loc_keys={len(loc)}")

    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)

    jobs = [
        ("events", lambda: parse_events.parse_events(layers, loc)),
        ("scripted_guis", lambda: parse_scripted_gui.parse_scripted_guis(layers)),
        ("technologies", lambda: parse_tech.parse_technologies(layers)),
        ("doctrines", lambda: parse_doctrines.parse_doctrines(layers)),
        ("catalogs", lambda: parse_catalogs.parse_catalogs(layers)),
        ("focus_defs", lambda: parse_focus_def.parse_focus_defs(layers, loc)),
        ("templates", lambda: parse_templates.parse_templates(layers)),
        ("world_defs", lambda: parse_world_defs.parse_world_defs(layers, loc)),
        ("characters", lambda: parse_characters.parse_characters(layers)),
        ("localisation", lambda: parse_localisation.parse_localisation(layers)),
        ("idea_tech_loc_keys", lambda: parse_idea_tech_loc.parse_idea_tech_loc_keys(layers)),
        ("decisions", lambda: parse_decisions.parse_decisions(layers)),
        ("ship_definitions", lambda: parse_ship_definitions.parse_ship_definitions(layers)),
        (
            "gfx_icon_manifest",
            lambda: gfx_icon_manifest_gate.build_catalog(
                argparse.Namespace(
                    vanilla=args.vanilla,
                    mods=args.mods,
                    userdata=args.userdata,
                    missing_texture_allowlist=gfx_icon_manifest_gate.DEFAULT_MISSING_TEXTURE_ALLOWLIST,
                    strict=True,
                )
            ),
        ),
    ]
    summary = {}
    for name, fn in jobs:
        out = fn()
        if name == "gfx_icon_manifest":
            _assert_gfx_gate_passed(out)
        (out_dir / f"{name}.json").write_text(
            json.dumps(out, ensure_ascii=False, indent=1), encoding="utf-8")
        summary[name] = out.get("meta", {})
        print(f"  wrote {name}.json")
    (out_dir / "_index.json").write_text(
        json.dumps({"meta_by_catalog": summary,
                    "layers": [ly.label for ly in layers]},
                   ensure_ascii=False, indent=1), encoding="utf-8")
    print(f"done in {time.time()-t0:.1f}s -> {out_dir}")


if __name__ == "__main__":
    main()
