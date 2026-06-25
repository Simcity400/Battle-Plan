#!/usr/bin/env python3
"""Precompute per-province + per-state pixel centroids from the province-id raster.

The game UI places combat bubbles and division counters at province/state
centroids on the map. We do NOT have centroids shipped, so this derives them
ONCE (offline, read-only on map_pipeline/) from ``province_ids_half.png``
(2816x1024, ``province_id = R + G*256 + B*65536``; id 0 = sea/unassigned) and
writes ``replay_game/output/centroids.json`` (gitignored, ~hundreds of KB).

Honest by construction:
  * A province with **zero** visible pixels at half-res gets **no** centroid
    entry (skipped, never faked at 0,0). The client treats a missing centroid as
    "can't place an overlay here" — it still lists the combat/division in panels.
  * Coordinates are in **raster pixels** (the same space the client decodes), so
    the client only has to scale by (displayWidth / rasterWidth).

Pillow is optional. If Pillow is missing we fall back to a pure-stdlib PNG
decoder good enough for this 8-bit RGB image. If even that fails (or the asset is
absent) we write nothing and the client computes centroids itself at boot from
the raster it already decoded (see static/map.js ``computeCentroids``) — so the
tool is never blocked on this script. This is a *cache*, not a hard dependency.

Usage:
    python build_centroids.py            # default half-res raster
    python build_centroids.py --full     # use full-res province_ids.png (slower)
"""
from __future__ import annotations

import argparse
import json
import os
import struct
import sys
import zlib
from typing import Dict, List, Optional, Tuple

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
MAP_ASSETS = os.path.join(REPO, "map_pipeline", "assets")
OUT_DIR = os.path.join(HERE, "output")


def _decode_png_rgb(path: str) -> Optional[Tuple[int, int, bytes]]:
    """Return (width, height, rgb_bytes) for an 8-bit RGB/RGBA PNG.

    Tries Pillow first; falls back to a minimal stdlib PNG inflate+unfilter.
    Returns None on failure (caller writes nothing — never fabricates).
    """
    try:  # fast path
        from PIL import Image  # type: ignore
        im = Image.open(path).convert("RGB")
        w, h = im.size
        return w, h, im.tobytes()
    except Exception:
        pass
    try:
        return _stdlib_png_rgb(path)
    except Exception as e:  # pragma: no cover - diagnostic only
        sys.stderr.write(f"[centroids] PNG decode failed for {path}: {e}\n")
        return None


def _paeth(a: int, b: int, c: int) -> int:
    p = a + b - c
    pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
    if pa <= pb and pa <= pc:
        return a
    if pb <= pc:
        return b
    return c


def _stdlib_png_rgb(path: str) -> Tuple[int, int, bytes]:
    """Minimal stdlib PNG -> RGB bytes (8-bit, colour type 2 RGB or 6 RGBA)."""
    with open(path, "rb") as fh:
        data = fh.read()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError("not a PNG")
    pos = 8
    width = height = bit_depth = colour = 0
    idat = bytearray()
    while pos < len(data):
        ln = struct.unpack(">I", data[pos:pos + 4])[0]
        ctype = data[pos + 4:pos + 8]
        body = data[pos + 8:pos + 8 + ln]
        pos += 12 + ln  # length(4)+type(4)+data+crc(4)
        if ctype == b"IHDR":
            width, height, bit_depth, colour = struct.unpack(">IIBB", body[:10])
        elif ctype == b"IDAT":
            idat += body
        elif ctype == b"IEND":
            break
    if bit_depth != 8 or colour not in (2, 6):
        raise ValueError(f"unsupported PNG (depth={bit_depth} colour={colour})")
    channels = 3 if colour == 2 else 4
    raw = zlib.decompress(bytes(idat))
    stride = width * channels
    out = bytearray(width * height * 3)
    prev = bytearray(stride)
    rp = 0
    for y in range(height):
        ftype = raw[rp]; rp += 1
        line = bytearray(raw[rp:rp + stride]); rp += stride
        if ftype == 1:  # Sub
            for i in range(channels, stride):
                line[i] = (line[i] + line[i - channels]) & 0xFF
        elif ftype == 2:  # Up
            for i in range(stride):
                line[i] = (line[i] + prev[i]) & 0xFF
        elif ftype == 3:  # Average
            for i in range(stride):
                a = line[i - channels] if i >= channels else 0
                line[i] = (line[i] + ((a + prev[i]) >> 1)) & 0xFF
        elif ftype == 4:  # Paeth
            for i in range(stride):
                a = line[i - channels] if i >= channels else 0
                c = prev[i - channels] if i >= channels else 0
                line[i] = (line[i] + _paeth(a, prev[i], c)) & 0xFF
        # copy RGB (drop alpha if present)
        op = y * width * 3
        if channels == 3:
            out[op:op + stride] = line
        else:
            for x in range(width):
                s = x * 4; d = op + x * 3
                out[d] = line[s]; out[d + 1] = line[s + 1]; out[d + 2] = line[s + 2]
        prev = line
    return width, height, bytes(out)


def compute(raster_file: str) -> Optional[Dict]:
    png = _decode_png_rgb(os.path.join(MAP_ASSETS, raster_file))
    if png is None:
        return None
    w, h, rgb = png
    # accumulate sum_x, sum_y, count, AND a known on-shape pixel per province id.
    sx: Dict[int, int] = {}
    sy: Dict[int, int] = {}
    cnt: Dict[int, int] = {}
    any_px: Dict[int, Tuple[int, int]] = {}   # first-seen pixel (guaranteed on the id)
    for y in range(h):
        row = y * w * 3
        for x in range(w):
            o = row + x * 3
            pid = rgb[o] | (rgb[o + 1] << 8) | (rgb[o + 2] << 16)
            if pid == 0:
                continue
            sx[pid] = sx.get(pid, 0) + x
            sy[pid] = sy.get(pid, 0) + y
            cnt[pid] = cnt.get(pid, 0) + 1
            if pid not in any_px:
                any_px[pid] = (x, y)

    def _pid_at(px: int, py: int) -> int:
        if px < 0 or py < 0 or px >= w or py >= h:
            return -1
        o = (py * w + px) * 3
        return rgb[o] | (rgb[o + 1] << 8) | (rgb[o + 2] << 16)

    # Snap each arithmetic centroid to a real pixel of the SAME id (concave /
    # archipelago shapes can put the mean in sea or in a neighbour). If the mean
    # already sits on the id, keep it; else fall back to a known on-shape pixel.
    # This matches the client's computeCentroids() so cache == client (Codex A).
    prov_centroids: Dict[str, List[int]] = {}
    for pid, n in cnt.items():
        cx, cy = round(sx[pid] / n), round(sy[pid] / n)
        if _pid_at(cx, cy) != pid:
            cx, cy = any_px[pid]
        prov_centroids[str(pid)] = [cx, cy]

    # state centroids = pixel-weighted mean of their provinces (so big provinces
    # pull correctly), derived from province->state.
    p2s_path = os.path.join(MAP_ASSETS, "province_to_state.json")
    state_centroids: Dict[str, List[int]] = {}
    try:
        with open(p2s_path, encoding="utf-8") as fh:
            p2s = json.load(fh)
        ssx: Dict[str, int] = {}; ssy: Dict[str, int] = {}; scnt: Dict[str, int] = {}
        s_any: Dict[str, Tuple[int, int]] = {}
        prov_state = {int(k): v for k, v in p2s.items() if str(k).isdigit()}
        for pid, n in cnt.items():
            sid = p2s.get(str(pid))
            if sid is None:
                continue
            k = str(sid)
            ssx[k] = ssx.get(k, 0) + sx[pid]
            ssy[k] = ssy.get(k, 0) + sy[pid]
            scnt[k] = scnt.get(k, 0) + n
            if k not in s_any:
                s_any[k] = any_px[pid]

        def _state_at(px: int, py: int) -> Optional[int]:
            pid = _pid_at(px, py)
            return prov_state.get(pid) if pid > 0 else None

        for k, n in scnt.items():
            cx, cy = round(ssx[k] / n), round(ssy[k] / n)
            if str(_state_at(cx, cy)) != k:     # snap to a pixel actually in this state
                cx, cy = s_any[k]
            state_centroids[k] = [cx, cy]
    except Exception as e:  # pragma: no cover
        sys.stderr.write(f"[centroids] state centroid join skipped: {e}\n")

    return {
        "raster_file": raster_file,
        "raster_width": w,
        "raster_height": h,
        "encoding": "province_id = R + G*256 + B*65536; coords in raster pixels",
        "n_provinces": len(prov_centroids),
        "n_states": len(state_centroids),
        "provinces": prov_centroids,
        "states": state_centroids,
    }


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description="Precompute province/state centroids.")
    ap.add_argument("--full", action="store_true",
                    help="use full-res province_ids.png (slower, sharper)")
    args = ap.parse_args(argv)
    raster = "province_ids.png" if args.full else "province_ids_half.png"
    result = compute(raster)
    if result is None:
        sys.stderr.write("[centroids] could not build (raster/decoder absent). "
                         "The client will compute centroids itself at boot.\n")
        return 1
    os.makedirs(OUT_DIR, exist_ok=True)
    out_path = os.path.join(OUT_DIR, "centroids.json")
    with open(out_path, "w", encoding="utf-8") as fh:
        json.dump(result, fh, separators=(",", ":"))
    print(f"[centroids] wrote {result['n_provinces']} provinces / "
          f"{result['n_states']} states -> {out_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
