#!/usr/bin/env python3
"""
Offline builder: strategic-region <-> province mapping for the native replay app.

Pure offline data prep. Reads a the game mod's map/strategicregions/*.txt (each file =
one strategic region: `id=N`, `name="..."`, `provinces={ a b c ... }`) and emits
shared JSON the replay can use to paint weather / naval-region control /
air-superiority on the map instead of only listing reports.

Effective load set (mod override model):
  The mod descriptor replace_path="map/strategicregions" => the mod's
  map/strategicregions/ directory FULLY REPLACES vanilla's. There is no per-file
  merge in a replace_path'd dir: vanilla files are not loaded at all. So when a mod
  dir is present we use ONLY the mod files. When --vanilla is given we additionally
  build the vanilla set purely for a count/coverage cross-check (NOT mixed in).

Clausewitz parse note:
  A region file ALSO contains a big `weather={ period={ between={ 0.0 30.0 } ... } }`
  block full of floats. We must extract ONLY the `provinces={...}` block via brace
  matching, never a global integer regex over the whole file (that would slurp
  weather numbers). We reuse the brace-matching approach from build_map_assets.py.

Inputs (read-only) under <mod>/:
  map/strategicregions/*.txt   per-region: id, name, provinces{}, weather{}
  map/definition.csv           province_id;R;G;B;type;coastal;terrain;continent
                               (used only for land/sea/lake validation buckets)

Outputs under <out>/assets/:
  strategic_region_to_provinces.json  {region_id: [province_id, ...]}
  province_to_strategic_region.json   {province_id: region_id}
  strategic_region_meta.json          {region_id: {name, province_count}}

Dependencies: Python stdlib only.

Usage:
  python build_strategic_regions.py [--mod <path>] [--out <path>] [--vanilla <path>]
Defaults: --mod ../../test (sibling), --out ./ (this script's dir).
"""

import argparse
import json
import os
import re
import sys

# Shared Clausewitz text-parsing primitives (sibling module; this script is run
# from map_pipeline/, so a plain module import resolves).
from clausewitz_parse import (
    ID_RE,
    NAME_RE,
    extract_brace_block,
    find_block,
    strip_comments,
)

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
DEFAULT_MOD = os.path.normpath(os.path.join(SCRIPT_DIR, "..", "..", "test"))

# Anchor provinces another agent needs region ids for (weather array pin).
ANCHORS = {
    7206: "Cyrenaica",
    3946: "Midi-Pyrenees",
    6945: "Afyon",
    7250: "Kolhapur/Deccan",
    10167: "Luzon",
}

# Comment/id/name regexes + the brace-block parser are shared with
# build_map_assets.py; they live in clausewitz_parse.py now.
_ID_RE = ID_RE
_NAME_RE = NAME_RE

_strip_comments = strip_comments
_extract_brace_block = extract_brace_block
# Local thin wrapper: this builder wants just the inner text (not the index) and
# anchors the keyword on a word boundary (historical behaviour).
def _find_block(text, keyword):
    """Return inner text for `keyword = { ... }`, or None."""
    inner, _brace_idx = find_block(text, keyword, word_boundary=True)
    return inner


def parse_region_file(path):
    """Parse one strategicregions/*.txt -> dict, or None if no id."""
    with open(path, "rb") as f:
        raw = f.read().decode("latin1")
    text = _strip_comments(raw)

    m = _ID_RE.search(text)
    if not m:
        return None
    region_id = int(m.group(1))

    name_m = _NAME_RE.search(text)
    name = name_m.group(1) if name_m else ("STRATEGICREGION_%d" % region_id)

    # provinces block is a flat list of integer ids (no '=' inside). Extract ONLY
    # this block via brace matching so we never read weather floats.
    prov_inner = _find_block(text, "provinces")
    provinces = []
    if prov_inner is not None:
        # The provinces block is purely integers; guard against any stray '=' by
        # rejecting tokens that aren't pure non-negative integers.
        provinces = [int(x) for x in re.findall(r"\b\d+\b", prov_inner)]

    return {"id": region_id, "name": name, "provinces": provinces}


def load_regions(regions_dir):
    """Parse every *.txt in a strategicregions dir.

    Returns (region_to_provinces, region_meta, prov_to_region, dupe_regions,
             prov_collisions).
    """
    region_to_provinces = {}
    region_meta = {}
    prov_to_region = {}
    dupe_regions = []        # (region_id, fileA, fileB)
    prov_collisions = []     # (province_id, regionA, regionB)
    seen_region_file = {}

    for fn in sorted(os.listdir(regions_dir)):
        if not fn.lower().endswith(".txt"):
            continue
        rec = parse_region_file(os.path.join(regions_dir, fn))
        if rec is None:
            continue
        rid = rec["id"]
        if rid in seen_region_file:
            dupe_regions.append((rid, seen_region_file[rid], fn))
            # First file wins (deterministic by sorted filename); skip the dupe.
            continue
        seen_region_file[rid] = fn
        region_to_provinces[rid] = rec["provinces"]
        region_meta[rid] = {"name": rec["name"], "province_count": len(rec["provinces"])}
        for p in rec["provinces"]:
            if p in prov_to_region and prov_to_region[p] != rid:
                prov_collisions.append((p, prov_to_region[p], rid))
            prov_to_region[p] = rid

    return region_to_provinces, region_meta, prov_to_region, dupe_regions, prov_collisions


def load_definition_buckets(def_path):
    """Return (land, sea, lake) sets of province ids from definition.csv."""
    land, sea, lake = set(), set(), set()
    if not os.path.isfile(def_path):
        return land, sea, lake
    with open(def_path, "r", encoding="latin1") as f:
        for raw in f:
            parts = raw.strip().split(";")
            if len(parts) < 5:
                continue
            try:
                pid = int(parts[0])
            except ValueError:
                continue
            t = parts[4].strip().lower()
            if t == "land":
                land.add(pid)
            elif t == "sea":
                sea.add(pid)
            elif t == "lake":
                lake.add(pid)
    return land, sea, lake


def main(argv):
    p = argparse.ArgumentParser(description="Build the game strategic-region province maps.")
    p.add_argument("--mod", default=DEFAULT_MOD,
                   help="Path to the game mod dir (default: ../../test sibling).")
    p.add_argument("--out", default=SCRIPT_DIR,
                   help="Output dir; assets/ created under it (default: script dir).")
    p.add_argument("--vanilla", default=None,
                   help="Optional vanilla the game dir for a count/coverage cross-check "
                        "(NOT mixed into the effective load set when mod present).")
    args = p.parse_args(argv)

    mod = os.path.abspath(args.mod)
    out_root = os.path.abspath(args.out)
    assets = os.path.join(out_root, "assets")
    os.makedirs(assets, exist_ok=True)

    mod_regions_dir = os.path.join(mod, "map", "strategicregions")
    vanilla_regions_dir = None
    if args.vanilla:
        vrd = os.path.join(os.path.abspath(args.vanilla), "map", "strategicregions")
        if os.path.isdir(vrd):
            vanilla_regions_dir = vrd

    print("=" * 70)
    print("the game strategic-region province mapping")
    print("  mod : %s" % mod_regions_dir)
    if vanilla_regions_dir:
        print("  van : %s (cross-check only)" % vanilla_regions_dir)
    print("=" * 70)

    if not os.path.isdir(mod_regions_dir):
        print("ERROR: missing %s" % mod_regions_dir, file=sys.stderr)
        return 2

    # EFFECTIVE LOAD SET ----------------------------------------------------
    # The mod replace_path's map/strategicregions => mod files fully replace
    # vanilla. We therefore use ONLY the mod files for the emitted assets.
    n_mod_files = len([f for f in os.listdir(mod_regions_dir) if f.lower().endswith(".txt")])
    print("[load set] %d mod region files (replace_path => vanilla NOT loaded)"
          % n_mod_files)

    (region_to_provinces, region_meta, prov_to_region,
     dupe_regions, prov_collisions) = load_regions(mod_regions_dir)

    n_regions = len(region_to_provinces)
    n_mapped_prov = len(prov_to_region)
    print("[regions] %d strategic regions parsed" % n_regions)
    print("[provinces] %d distinct provinces mapped to a region" % n_mapped_prov)

    if dupe_regions:
        print("  WARNING: %d duplicate region ids across files (first-by-name kept):"
              % len(dupe_regions))
        for rid, a, b in dupe_regions[:10]:
            print("    id=%d in '%s' and '%s'" % (rid, a, b))
    if prov_collisions:
        print("  WARNING: %d provinces claimed by >1 region:" % len(prov_collisions))
        for pp, a, b in prov_collisions[:10]:
            print("    province %d in regions %d and %d" % (pp, a, b))

    # WRITE ASSETS ----------------------------------------------------------
    s2p = {str(rid): sorted(provs) for rid, provs in sorted(region_to_provinces.items())}
    p2s = {str(pid): rid for pid, rid in sorted(prov_to_region.items())}
    meta = {str(rid): region_meta[rid] for rid in sorted(region_meta)}

    with open(os.path.join(assets, "strategic_region_to_provinces.json"), "w",
              encoding="utf-8") as f:
        json.dump(s2p, f, separators=(",", ":"))
    with open(os.path.join(assets, "province_to_strategic_region.json"), "w",
              encoding="utf-8") as f:
        json.dump(p2s, f, separators=(",", ":"))
    with open(os.path.join(assets, "strategic_region_meta.json"), "w",
              encoding="utf-8") as f:
        json.dump(meta, f, indent=1)

    # VALIDATION ------------------------------------------------------------
    print("-" * 70)
    print("VALIDATION")
    def_path = os.path.join(mod, "map", "definition.csv")
    land, sea, lake = load_definition_buckets(def_path)
    print("  definition.csv: %d land, %d sea, %d lake provinces"
          % (len(land), len(sea), len(lake)))

    mapped = set(prov_to_region.keys())
    land_mapped = land & mapped
    sea_mapped = sea & mapped
    lake_mapped = lake & mapped
    land_unmapped = sorted(land - mapped)
    sea_unmapped = sorted(sea - mapped)

    if land:
        pct = 100.0 * len(land_mapped) / len(land)
        print("  LAND coverage: %d / %d land provinces have a strategic region (%.2f%%)"
              % (len(land_mapped), len(land), pct))
        print("  SEA  coverage: %d / %d sea provinces have a (naval) strategic region"
              % (len(sea_mapped), len(sea)))
        print("  LAKE coverage: %d / %d lake provinces mapped"
              % (len(lake_mapped), len(lake)))
        if land_unmapped:
            print("  UNMAPPED LAND provinces (%d): %s%s"
                  % (len(land_unmapped), land_unmapped[:40],
                     " ..." if len(land_unmapped) > 40 else ""))
        else:
            print("  UNMAPPED LAND provinces: 0 (every land province has a region)")
        # provinces mapped to a region but with NO type in definition (or absent)
        ghost = sorted(mapped - land - sea - lake)
        if ghost:
            print("  NOTE: %d region-mapped provinces not land/sea/lake in definition.csv "
                  "(likely id 0 / impassable): %s" % (len(ghost), ghost[:20]))
    else:
        print("  (definition.csv not found; skipped land/sea coverage)")

    # Region count reconciliation vs the 304 weather-array assumption.
    print("-" * 70)
    print("WEATHER-REGION RECONCILIATION")
    print("  strategic-region count (effective) : %d" % n_regions)
    print("  mod region files on disk           : %d" % n_mod_files)
    if vanilla_regions_dir:
        v_r2p, _, _, _, _ = load_regions(vanilla_regions_dir)
        n_van_files = len([f for f in os.listdir(vanilla_regions_dir)
                           if f.lower().endswith(".txt")])
        print("  vanilla region files / parsed      : %d / %d"
              % (n_van_files, len(v_r2p)))
    print("  weather array assumed length       : 304")
    if n_regions == 304:
        print("  => MATCH: the weather array (assumed 304) == strategic-region count. "
              "The weather partition IS the strategic-region partition (one weather "
              "block per strategic region).")
    else:
        print("  => MISMATCH: weather-array length (304) != strategic-region count "
              "(%d). The weather partition differs from the region partition." % n_regions)

    # Anchor provinces ------------------------------------------------------
    print("-" * 70)
    print("ANCHOR PROVINCES (for the weather pin)")
    anchor_out = {}
    for pid, label in ANCHORS.items():
        rid = prov_to_region.get(pid)
        rname = region_meta.get(rid, {}).get("name") if rid is not None else None
        anchor_out[pid] = rid
        print("  province %-6d (%-16s) -> strategic_region %s  %s"
              % (pid, label, rid if rid is not None else "UNMAPPED",
                 ("[%s]" % rname) if rname else ""))

    print("-" * 70)
    print("Assets written under: %s" % assets)
    print("  strategic_region_to_provinces.json (%d regions)" % n_regions)
    print("  province_to_strategic_region.json  (%d provinces)" % n_mapped_prov)
    print("  strategic_region_meta.json         (%d regions)" % len(meta))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
