#!/usr/bin/env python3
"""
Offline map-preprocessing pipeline for the native Battle Plan app.

Pure offline data prep: reads a the game mod's map + history + country data and emits
shared static assets that renderers/readers consume to resolve
pixel -> province -> state -> owner. No game, no network.

Inputs (read-only) under <mod>/:
  map/definition.csv          province_id;R;G;B;type;coastal;terrain;continent
  map/provinces.bmp           5632x2048, 24bpp, each pixel's RGB == province colour
  history/states/*.txt        per-state: id, name, provinces{}, victory_points{},
                              state_category, manpower   (latin1 / Windows-1252)
  common/country_tags/*.txt   TAG = "countries/File.txt"
  common/countries/*.txt      per-country file with `color = { r g b }`
  common/countries/colors.txt TAG = { color = ... color_ui = rgb {...} } overrides

Outputs under <out>/assets/:
  province_ids.png            lossless RGB PNG; pixel encodes province id as
                              id = R + G*256 + B*65536   (R = id & 0xFF, etc.)
  province_ids_meta.json      width/height/encoding/downscale info
  province_to_state.json      {province_id: state_id}   (absent => unassigned)
  state_meta.json             {state_id: {name_key, provinces, vp_total,
                              victory_points, state_category, manpower}}
  country_colors.json         {TAG: [r,g,b]}  map colours
  README.md                   asset spec + renderer pipeline

Dependencies: Python stdlib + numpy only (Pillow is NOT used; PNG is written via
stdlib zlib+struct so this runs even where Pillow is absent).

Usage:
  python build_map_assets.py [--mod <path-to-mod>] [--out <path>]
                             [--vanilla <path>] [--downscale N] [--full-res]
Defaults: --mod ../test (sibling of this repo), --out ./ (this script's dir),
          --downscale 2.
"""

import argparse
import colorsys
import json
import os
import re
import struct
import sys
import zlib

import numpy as np

# Shared Clausewitz text-parsing primitives (sibling module; this script is run
# from map_pipeline/, so a plain module import resolves).
from clausewitz_parse import (
    ID_RE,
    NAME_RE,
    extract_brace_block,
    find_block,
    strip_comments,
)

# ---------------------------------------------------------------------------
# Paths / config
# ---------------------------------------------------------------------------

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
# Default mod path is a RELATIVE sibling so no personal absolute path is baked in.
DEFAULT_MOD = os.path.normpath(os.path.join(SCRIPT_DIR, "..", "..", "test"))


def parse_args(argv):
    p = argparse.ArgumentParser(description="Build the game map assets for the native replay app.")
    p.add_argument("--mod", default=DEFAULT_MOD,
                   help="Path to the game mod dir (default: ../../test sibling).")
    p.add_argument("--out", default=SCRIPT_DIR,
                   help="Output dir; assets/ is created under it (default: script dir).")
    p.add_argument("--vanilla", default=None,
                   help="Optional vanilla the game dir for fallback country files.")
    p.add_argument("--downscale", type=int, default=2,
                   help="Integer nearest-neighbour downscale for the id raster "
                        "(1 = full res). Default 2 => 2816x1024.")
    p.add_argument("--full-res", action="store_true",
                   help="Force full-resolution id raster (overrides --downscale).")
    return p.parse_args(argv)


# ---------------------------------------------------------------------------
# definition.csv
# ---------------------------------------------------------------------------

def load_definition(path):
    """Return (ids_array, colors_array) and a color_key->id map.

    ids: int array of province ids in file order.
    colors: Nx3 uint8 (R,G,B).
    color_key = R + G*256 + B*65536.
    Note: id 0 is the sentinel "no province" row (R;G;B == 0;0;0).
    """
    ids = []
    colors = []
    dupes = []
    seen = {}
    with open(path, "r", encoding="latin1") as f:
        for lineno, raw in enumerate(f, 1):
            line = raw.strip()
            if not line:
                continue
            parts = line.split(";")
            if len(parts) < 4:
                continue
            try:
                pid = int(parts[0])
                r, g, b = int(parts[1]), int(parts[2]), int(parts[3])
            except ValueError:
                # header or malformed row; skip
                continue
            key = r + (g << 8) + (b << 16)
            if key in seen and seen[key] != pid:
                dupes.append((key, seen[key], pid))
            else:
                seen[key] = pid
            ids.append(pid)
            colors.append((r, g, b))
    ids = np.array(ids, dtype=np.int64)
    colors = np.array(colors, dtype=np.uint8)
    return ids, colors, seen, dupes


# ---------------------------------------------------------------------------
# provinces.bmp  (24bpp uncompressed, bottom-up BGR)
# ---------------------------------------------------------------------------

def load_bmp_rgb(path):
    """Parse a 24bpp uncompressed BMP into an HxWx3 uint8 RGB array (top-down)."""
    with open(path, "rb") as f:
        data = f.read()
    if data[:2] != b"BM":
        raise ValueError("Not a BMP file (bad magic).")
    pixel_offset = struct.unpack_from("<I", data, 10)[0]
    dib_size = struct.unpack_from("<I", data, 14)[0]
    width = struct.unpack_from("<i", data, 18)[0]
    height_raw = struct.unpack_from("<i", data, 22)[0]
    planes = struct.unpack_from("<H", data, 26)[0]
    bpp = struct.unpack_from("<H", data, 28)[0]
    compression = struct.unpack_from("<I", data, 30)[0]
    if bpp != 24:
        raise ValueError("Only 24bpp BMP supported, got %d." % bpp)
    if compression != 0:
        raise ValueError("Only uncompressed BMP (BI_RGB) supported, got %d." % compression)
    bottom_up = height_raw > 0
    height = abs(height_raw)
    # Row stride is padded to a 4-byte boundary.
    stride = (width * 3 + 3) & ~3
    needed = pixel_offset + stride * height
    if len(data) < needed:
        raise ValueError("BMP truncated: have %d bytes, need %d." % (len(data), needed))
    buf = np.frombuffer(data, dtype=np.uint8, count=stride * height, offset=pixel_offset)
    rows = buf.reshape(height, stride)
    px = rows[:, : width * 3].reshape(height, width, 3)
    if bottom_up:
        px = px[::-1]  # bottom-up -> top-down
    # BMP stores BGR; reverse to RGB. Copy to make it contiguous/writable.
    rgb = px[:, :, ::-1].copy()
    return rgb, width, height, dib_size, planes


# ---------------------------------------------------------------------------
# pixel RGB -> province id, via a 2^24 flat lookup table
# ---------------------------------------------------------------------------

def build_color_to_id_lut(colors, ids):
    """Return an int32 LUT of length 2^24 mapping color_key -> province id.

    Unknown colors map to -1. ~64 MB; fine for a one-time offline build and far
    faster than per-pixel dict lookups over ~11.5M pixels.
    """
    lut = np.full(1 << 24, -1, dtype=np.int32)
    keys = (colors[:, 0].astype(np.int64)
            + (colors[:, 1].astype(np.int64) << 8)
            + (colors[:, 2].astype(np.int64) << 16))
    lut[keys] = ids.astype(np.int32)
    return lut


def rgb_to_province_ids(rgb, lut):
    """Map an HxWx3 RGB array to an HxW int32 province-id array."""
    h, w, _ = rgb.shape
    flat = (rgb[:, :, 0].astype(np.int64)
            + (rgb[:, :, 1].astype(np.int64) << 8)
            + (rgb[:, :, 2].astype(np.int64) << 16))
    return lut[flat].reshape(h, w)


# ---------------------------------------------------------------------------
# PNG writer (stdlib zlib + struct) -- 8-bit RGB, no Pillow
# ---------------------------------------------------------------------------

def write_png_rgb(path, rgb):
    """Write an HxWx3 uint8 array as a lossless RGB PNG (color type 2, 8-bit)."""
    h, w, _ = rgb.shape
    rgb = np.ascontiguousarray(rgb, dtype=np.uint8)

    def chunk(tag, payload):
        out = struct.pack(">I", len(payload)) + tag + payload
        crc = zlib.crc32(tag + payload) & 0xFFFFFFFF
        return out + struct.pack(">I", crc)

    sig = b"\x89PNG\r\n\x1a\n"
    ihdr = struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)  # 8-bit, RGB, no interlace
    # Prepend a filter byte (0 = None) to each scanline.
    raw = np.empty((h, w * 3 + 1), dtype=np.uint8)
    raw[:, 0] = 0
    raw[:, 1:] = rgb.reshape(h, w * 3)
    compressed = zlib.compress(raw.tobytes(), 9)
    with open(path, "wb") as fp:
        fp.write(sig)
        fp.write(chunk(b"IHDR", ihdr))
        fp.write(chunk(b"IDAT", compressed))
        fp.write(chunk(b"IEND", b""))


def encode_ids_to_rgb(id_raster):
    """Encode an HxW province-id array into HxWx3 uint8 (id = R + G*256 + B*65536).

    -1 (unknown/no province) encodes as 0 -> id 0, the definition.csv sentinel,
    which the renderer treats as 'no province'.
    """
    ids = id_raster.astype(np.int64)
    ids = np.where(ids < 0, 0, ids)
    r = (ids & 0xFF).astype(np.uint8)
    g = ((ids >> 8) & 0xFF).astype(np.uint8)
    b = ((ids >> 16) & 0xFF).astype(np.uint8)
    return np.dstack([r, g, b])


# ---------------------------------------------------------------------------
# state files
# ---------------------------------------------------------------------------

# Comment/id/name regexes + the brace-block parser are shared with
# build_strategic_regions.py; they live in clausewitz_parse.py now.
_ID_RE = ID_RE
_NAME_RE = NAME_RE
# These two regexes are specific to history/states files (not shared).
_CATEGORY_RE = re.compile(r"state_category\s*=\s*(\w+)")
_MANPOWER_RE = re.compile(r"\bmanpower\s*=\s*(\d+)")

_strip_comments = strip_comments
_extract_brace_block = extract_brace_block
# Local thin wrapper: this builder's _find_block returns (inner, brace_idx) and
# matches WITHOUT a leading word boundary (historical behaviour).
def _find_block(text, keyword):
    """Return (inner_text, brace_index) for `keyword = { ... }`, or (None, -1)."""
    return find_block(text, keyword)


def parse_state_file(path):
    """Parse one history/states/*.txt into a dict, or None if no id."""
    with open(path, "rb") as f:
        raw = f.read().decode("latin1")
    text = _strip_comments(raw)

    m = _ID_RE.search(text)
    if not m:
        return None
    state_id = int(m.group(1))

    name_m = _NAME_RE.search(text)
    name_key = name_m.group(1) if name_m else ("STATE_%d" % state_id)

    cat_m = _CATEGORY_RE.search(text)
    state_category = cat_m.group(1) if cat_m else None

    mp_m = _MANPOWER_RE.search(text)
    manpower = int(mp_m.group(1)) if mp_m else 0

    # provinces block: a flat list of integer ids (no '=' inside).
    prov_inner, _ = _find_block(text, "provinces")
    provinces = [int(x) for x in re.findall(r"\d+", prov_inner)] if prov_inner else []

    # victory_points: each VP block is a flat list of `prov vp` pairs.
    # There can be MULTIPLE vp blocks (e.g. in nested history dates). We must
    # never read a per-province building block (which has '=' inside) as VP.
    victory_points = {}
    for m in re.finditer(r"victory_points\s*=\s*\{", text):
        inner = _extract_brace_block(text, m.end() - 1)
        # A genuine VP block contains only `prov vp` integer pairs, no '='.
        if "=" in inner:
            continue
        nums = [int(x) for x in re.findall(r"-?\d+", inner)]
        for i in range(0, len(nums) - 1, 2):
            prov, vp = nums[i], nums[i + 1]
            victory_points[prov] = victory_points.get(prov, 0) + vp
    vp_total = sum(victory_points.values())

    return {
        "id": state_id,
        "name_key": name_key,
        "provinces": provinces,
        "victory_points": victory_points,
        "vp_total": vp_total,
        "state_category": state_category,
        "manpower": manpower,
    }


def load_states(states_dir):
    states = {}
    prov_to_state = {}
    collisions = []
    for fn in sorted(os.listdir(states_dir)):
        if not fn.lower().endswith(".txt"):
            continue
        rec = parse_state_file(os.path.join(states_dir, fn))
        if rec is None:
            continue
        sid = rec["id"]
        states[sid] = {
            "name_key": rec["name_key"],
            "provinces": rec["provinces"],
            "vp_total": rec["vp_total"],
            "victory_points": rec["victory_points"],
            "state_category": rec["state_category"],
            "manpower": rec["manpower"],
        }
        for p in rec["provinces"]:
            if p in prov_to_state and prov_to_state[p] != sid:
                collisions.append((p, prov_to_state[p], sid))
            prov_to_state[p] = sid
    return states, prov_to_state, collisions


# ---------------------------------------------------------------------------
# country colours
# ---------------------------------------------------------------------------

def hoi4_hsv_to_rgb(h, s, v):
    """Convert HOI4-style HSV to RGB 0..255.

    the game's HSV is non-standard: H in [0,1], but S and V may exceed 1 (used to
    brighten/saturate). We clamp S and V to [0,1] for a faithful display colour
    (values >1 just clip to full). H wraps mod 1.

    Some `colors.txt` entries are MISLABELLED RGB written as `HSV { 221 178 83 }`
    (e.g. BRA). A real hue is in [0,1]; if H > 1 we treat the triple as 0..255 RGB.
    """
    if h > 1.0:  # mislabelled: these are 0..255 RGB, not HSV
        return [min(max(int(round(x)), 0), 255) for x in (h, s, v)]
    h = h % 1.0
    s = min(max(s, 0.0), 1.0)
    v = min(max(v, 0.0), 1.0)
    r, g, b = colorsys.hsv_to_rgb(h, s, v)
    return [int(round(r * 255)), int(round(g * 255)), int(round(b * 255))]


_TAG_FILE_RE = re.compile(r'([A-Z0-9]{2,3})\s*=\s*"([^"]+)"')
# color = { r g b } | color = rgb { r g b } | color = HSV { h s v }
_COLOR_RE = re.compile(
    r"color\s*=\s*(rgb|hsv)?\s*\{\s*([0-9.]+)\s+([0-9.]+)\s+([0-9.]+)\s*\}",
    re.IGNORECASE,
)


def parse_color_value(kind, a, b, c):
    """Given (kind, a, b, c) strings, return [r,g,b] 0..255."""
    kind = (kind or "").lower()
    if kind == "hsv":
        return hoi4_hsv_to_rgb(float(a), float(b), float(c))
    # rgb or bare braces -> integer 0..255 (clamp defensively)
    vals = []
    for x in (a, b, c):
        n = int(round(float(x)))
        vals.append(min(max(n, 0), 255))
    return vals


def load_tag_files(tags_dir):
    """Return {TAG: 'countries/File.txt'} from common/country_tags/*.txt."""
    mapping = {}
    if not os.path.isdir(tags_dir):
        return mapping
    for fn in sorted(os.listdir(tags_dir)):
        if not fn.lower().endswith(".txt"):
            continue
        with open(os.path.join(tags_dir, fn), "rb") as f:
            text = f.read().decode("latin1")
        text = _strip_comments(text)
        for m in _TAG_FILE_RE.finditer(text):
            tag, rel = m.group(1), m.group(2)
            if tag == "dynamic_tags":
                continue
            mapping.setdefault(tag, rel)
    return mapping


def color_from_country_file(common_dir, rel, vanilla_common_dir=None):
    """Read the first `color = ...` from a per-country file path (rel like
    'countries/France.txt'). Returns [r,g,b] or None. Falls back to vanilla."""
    candidates = [os.path.join(common_dir, rel)]
    if vanilla_common_dir:
        candidates.append(os.path.join(vanilla_common_dir, rel))
    for path in candidates:
        if not os.path.isfile(path):
            continue
        with open(path, "rb") as f:
            text = f.read().decode("latin1")
        text = _strip_comments(text)
        m = _COLOR_RE.search(text)
        if m:
            return parse_color_value(m.group(1), m.group(2), m.group(3), m.group(4))
    return None


def load_colors_txt_overrides(colors_path):
    """Parse common/countries/colors.txt -> {TAG: [r,g,b]} from the `color = ...`
    (NOT color_ui). Returns {} if missing."""
    overrides = {}
    if not os.path.isfile(colors_path):
        return overrides
    with open(colors_path, "rb") as f:
        text = f.read().decode("latin1")
    text = _strip_comments(text)
    # Find each `TAG = {` block, then the first `color = ...` inside it.
    for m in re.finditer(r"\b([A-Z0-9]{2,3})\s*=\s*\{", text):
        tag = m.group(1)
        inner = _extract_brace_block(text, m.end() - 1)
        cm = _COLOR_RE.search(inner)
        if cm:
            overrides[tag] = parse_color_value(cm.group(1), cm.group(2),
                                               cm.group(3), cm.group(4))
    return overrides


def build_country_colors(common_dir, vanilla_common_dir=None):
    """Map colour per TAG: per-country file color first, colors.txt overrides it."""
    tags_dir = os.path.join(common_dir, "country_tags")
    tag_files = load_tag_files(tags_dir)
    if vanilla_common_dir and not tag_files:
        tag_files = load_tag_files(os.path.join(vanilla_common_dir, "country_tags"))

    colors = {}
    missing = []
    for tag, rel in sorted(tag_files.items()):
        c = color_from_country_file(common_dir, rel, vanilla_common_dir)
        if c is None:
            missing.append(tag)
            continue
        colors[tag] = c

    overrides = load_colors_txt_overrides(
        os.path.join(common_dir, "countries", "colors.txt"))
    n_over = 0
    for tag, c in overrides.items():
        # Only override tags we actually know about (have a country file), plus
        # keep override-only tags too (some colour-only tags may lack files).
        colors[tag] = c
        n_over += 1
    return colors, tag_files, missing, n_over


# ---------------------------------------------------------------------------
# main
# ---------------------------------------------------------------------------

def main(argv):
    args = parse_args(argv)
    mod = os.path.abspath(args.mod)
    out_root = os.path.abspath(args.out)
    assets = os.path.join(out_root, "assets")
    os.makedirs(assets, exist_ok=True)

    vanilla_common = None
    if args.vanilla:
        vc = os.path.join(os.path.abspath(args.vanilla), "common")
        if os.path.isdir(vc):
            vanilla_common = vc

    def need(p):
        if not os.path.exists(p):
            print("ERROR: missing input: %s" % p, file=sys.stderr)
            sys.exit(2)
        return p

    def_path = need(os.path.join(mod, "map", "definition.csv"))
    bmp_path = need(os.path.join(mod, "map", "provinces.bmp"))
    states_dir = need(os.path.join(mod, "history", "states"))
    common_dir = need(os.path.join(mod, "common"))

    print("=" * 70)
    print("the game map asset pipeline")
    print("  mod : %s" % mod)
    print("  out : %s" % assets)
    print("=" * 70)

    # 1. definition.csv ----------------------------------------------------
    ids, colors, color_to_id, dupes = load_definition(def_path)
    n_def = len(ids)
    print("[definition.csv] %d province rows (incl. id 0 sentinel)" % n_def)
    if dupes:
        print("  WARNING: %d duplicate colour keys in definition.csv" % len(dupes))
        for key, a, b in dupes[:5]:
            print("    color_key=%d -> ids %d and %d" % (key, a, b))

    # 2. BMP ---------------------------------------------------------------
    rgb, w, h, dib, planes = load_bmp_rgb(bmp_path)
    print("[provinces.bmp] %dx%d, %d planes, DIB header %d bytes" % (w, h, planes, dib))

    lut = build_color_to_id_lut(colors, ids)
    id_raster = rgb_to_province_ids(rgb, lut)
    n_unknown_px = int(np.count_nonzero(id_raster < 0))
    distinct_ids_in_bmp = np.unique(id_raster[id_raster >= 0])
    print("  distinct province ids present in bmp: %d" % distinct_ids_in_bmp.size)
    if n_unknown_px:
        print("  WARNING: %d pixels have a colour not in definition.csv" % n_unknown_px)

    # 3. encode + write PNG(s) ---------------------------------------------
    # The FULL-RES raster is the authoritative hit-test asset (no province can
    # be lost). A downscaled display copy is optional (--downscale, default 2)
    # and we report any province that disappears at that scale.
    ids_full_set = set(int(x) for x in distinct_ids_in_bmp.tolist())

    full_encoded = encode_ids_to_rgb(id_raster)
    full_png = os.path.join(assets, "province_ids.png")
    write_png_rgb(full_png, full_encoded)
    full_size = os.path.getsize(full_png)
    print("[province_ids.png] %dx%d (full res, authoritative), %d bytes"
          % (w, h, full_size))

    factor = 1 if args.full_res else max(1, args.downscale)
    half_info = None
    lost = []
    if factor > 1:
        raster_half = id_raster[::factor, ::factor]
        hh, hw = raster_half.shape
        ids_half_set = set(int(x) for x in np.unique(raster_half[raster_half >= 0]).tolist())
        lost = sorted(ids_full_set - ids_half_set)
        half_encoded = encode_ids_to_rgb(raster_half)
        half_png = os.path.join(assets, "province_ids_half.png")
        write_png_rgb(half_png, half_encoded)
        half_size = os.path.getsize(half_png)
        print("[province_ids_half.png] %dx%d (downscale x%d, display), %d bytes"
              % (hw, hh, factor, half_size))
        if lost:
            print("  NOTE: %d provinces have no pixel at x%d downscale "
                  "(use the full-res PNG for hit-testing those)"
                  % (len(lost), factor))
        half_info = {
            "file": "province_ids_half.png",
            "width": hw,
            "height": hh,
            "downscale_factor": factor,
            "downscale_method": "nearest-neighbour (stride slice); ids never blend",
            "provinces_lost_at_this_scale": len(lost),
            "lost_province_ids_sample": lost[:50],
        }

    meta = {
        "authoritative": {
            "file": "province_ids.png",
            "format": "PNG, 8-bit RGB, lossless (no alpha)",
            "width": w,
            "height": h,
            "note": "Full resolution; every province present. Use for pixel->province hit-testing.",
        },
        "downscaled_display": half_info,
        "source_width": w,
        "source_height": h,
        "encoding": "province_id = R + G*256 + B*65536",
        "decode_js": "id = data[i] | (data[i+1] << 8) | (data[i+2] << 16)  // getImageData RGBA, R first",
        "sentinel": "id 0 == no province (sea/lake/unassigned colours and unknown pixels map here)",
        "render_smoothing": "Draw with imageSmoothingEnabled=false; never scale the id PNG with interpolation.",
        "note": ("Pixel colour is the ENCODED province id, NOT a display colour. "
                 "Renderer decodes id, then province_to_state -> state owner -> country_colors."),
    }
    with open(os.path.join(assets, "province_ids_meta.json"), "w", encoding="utf-8") as f:
        json.dump(meta, f, indent=2)

    # 4. states ------------------------------------------------------------
    states, prov_to_state, collisions = load_states(states_dir)
    print("[states] %d states parsed" % len(states))
    if collisions:
        print("  WARNING: %d province->state collisions (province in >1 state)"
              % len(collisions))
        for p, a, b in collisions[:5]:
            print("    province %d in states %d and %d" % (p, a, b))

    with open(os.path.join(assets, "province_to_state.json"), "w", encoding="utf-8") as f:
        json.dump({str(k): v for k, v in sorted(prov_to_state.items())}, f,
                  separators=(",", ":"))
    with open(os.path.join(assets, "state_meta.json"), "w", encoding="utf-8") as f:
        json.dump({str(k): v for k, v in sorted(states.items())}, f, indent=1)

    # 5. country colours ---------------------------------------------------
    country_colors, tag_files, missing_color, n_over = build_country_colors(
        common_dir, vanilla_common)
    print("[country_colors] %d tags total, %d map colours resolved (%d from colors.txt override)"
          % (len(tag_files), len(country_colors), n_over))
    if missing_color:
        print("  NOTE: %d tags had no resolvable colour: %s"
              % (len(missing_color), ", ".join(missing_color[:15])))
    with open(os.path.join(assets, "country_colors.json"), "w", encoding="utf-8") as f:
        json.dump({k: v for k, v in sorted(country_colors.items())}, f, indent=1)

    # 6. VALIDATION --------------------------------------------------------
    print("-" * 70)
    print("VALIDATION")
    all_prov_ids = set(int(x) for x in ids)
    bmp_id_set = set(int(x) for x in distinct_ids_in_bmp.tolist())

    expectations = {
        351: {"count": 70},
        400: {"count": 65},
        495: {"count": 44},
        1: {"provinces": [3838, 9851, 11804]},
    }
    ok = True
    for sid, exp in expectations.items():
        st = states.get(sid)
        if st is None:
            print("  FAIL: state %d not found" % sid)
            ok = False
            continue
        provs = st["provinces"]
        label = st["name_key"]
        if "count" in exp:
            match = len(provs) == exp["count"]
            ok = ok and match
            print("  state %d (%s): %d provinces (expected %d) %s"
                  % (sid, label, len(provs), exp["count"], "OK" if match else "FAIL"))
        if "provinces" in exp:
            match = provs == exp["provinces"]
            ok = ok and match
            print("  state %d (%s): provinces %s (expected %s) %s"
                  % (sid, label, provs, exp["provinces"], "OK" if match else "FAIL"))
        # each province exists in definition.csv and has a colour present in bmp
        for p in provs:
            in_def = p in all_prov_ids
            in_bmp = p in bmp_id_set
            if not in_def:
                print("    FAIL: province %d (state %d) not in definition.csv" % (p, sid))
                ok = False
            elif not in_bmp:
                print("    NOTE: province %d (state %d) in definition but no pixels in bmp"
                      % (p, sid))

    # counts
    n_states = len(states)
    n_prov_total = n_def  # includes id 0
    assigned = set(prov_to_state.keys())
    # "real" provinces are those in definition with id != 0
    real_prov = all_prov_ids - {0}
    unassigned = sorted(real_prov - assigned)
    # land provinces specifically
    print("  province rows in definition.csv (incl id 0): %d" % n_prov_total)
    print("  states parsed: %d" % n_states)
    print("  provinces assigned to a state: %d" % len(assigned))
    print("  provinces in definition but unassigned to any state: %d" % len(unassigned))

    # cross-check: assigned provinces that don't exist in definition
    ghost = sorted(assigned - all_prov_ids)
    if ghost:
        print("  WARNING: %d provinces referenced by states but absent from definition.csv: %s"
              % (len(ghost), ghost[:15]))

    print("-" * 70)
    spot_ok = ok
    print("SPOT-CHECK RESULT: %s" % ("PASS" if spot_ok else "FAIL"))
    assert spot_ok, "Spot-check validation failed; see report above."

    print("Done. Assets written under: %s" % assets)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
