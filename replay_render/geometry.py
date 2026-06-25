"""Map geometry loader for the replay time-lapse renderer.

READ-ONLY consumer of the ``map_pipeline/assets/`` outputs. This module never
recomputes the state->polygon / province->pixel mapping from the raw mod files;
it reuses the pipeline's authoritative ``province_ids*.png`` raster and the
``province_to_state.json`` / ``state_meta.json`` / ``country_colors.json`` it
already produced (see ``map_pipeline/README.md``).

What it provides to the renderer:
  * ``id_raster``      HxW uint32, province id per pixel (id 0 == water/none).
  * ``state_by_prov``  int32[maxId+1], state id per province id (-1 == none).
  * ``country_colors`` {TAG: (r,g,b)}.
  * ``state_meta``     {state_id(int): {...}} static facts (name, vp, ...).
  * ``prov_centroids`` {province_id: (x, y)} pixel centroid, for division markers.
  * ``state_centroids``{state_id: (x, y)} pixel centroid (mean of its provinces).

PNG decode: Pillow fast-path if available, else a small pure-stdlib decoder
(zlib + the PNG None/Sub/Up/Average/Paeth filters) so this runs in the same
"stdlib + numpy only" environments the pipeline targets.
"""

from __future__ import annotations

import json
import struct
import zlib
from dataclasses import dataclass, field
from pathlib import Path
from typing import Dict, Optional, Tuple

import numpy as np


# --------------------------------------------------------------------------
# asset discovery (read-only; relative to this file, never an absolute path)
# --------------------------------------------------------------------------

def _repo_root() -> Path:
    # replay_render/geometry.py -> repo root is the parent of replay_render/
    return Path(__file__).resolve().parent.parent


def _assets_dir() -> Path:
    return _repo_root() / "map_pipeline" / "assets"


# --------------------------------------------------------------------------
# PNG decode -- Pillow fast-path, stdlib fallback
# --------------------------------------------------------------------------

def _decode_png_stdlib(path: Path) -> np.ndarray:
    """Decode an 8-bit RGB (color type 2) PNG to HxWx3 uint8 with zlib+filters.

    Supports exactly what ``map_pipeline.build_map_assets.write_png_rgb`` emits
    (8-bit, color type 2, no interlace, filter 0 per scanline) PLUS the standard
    five scanline filters, so it also reads PNGs re-encoded by other tools.
    """
    data = path.read_bytes()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError(f"not a PNG: {path}")
    pos = 8
    width = height = bit_depth = color_type = interlace = None
    idat = bytearray()
    while pos < len(data):
        (length,) = struct.unpack(">I", data[pos:pos + 4])
        ctype = data[pos + 4:pos + 8]
        payload = data[pos + 8:pos + 8 + length]
        pos += 12 + length  # 4 len + 4 type + payload + 4 crc
        if ctype == b"IHDR":
            (width, height, bit_depth, color_type, _comp, _filt,
             interlace) = struct.unpack(">IIBBBBB", payload)
        elif ctype == b"IDAT":
            idat += payload
        elif ctype == b"IEND":
            break
    if width is None:
        raise ValueError("PNG missing IHDR")
    if bit_depth != 8 or color_type != 2 or interlace != 0:
        raise ValueError(
            f"unsupported PNG (bit_depth={bit_depth}, color_type={color_type}, "
            f"interlace={interlace}); install Pillow for general PNGs")

    raw = zlib.decompress(bytes(idat))
    channels = 3
    stride = width * channels
    out = np.empty((height, stride), dtype=np.uint8)
    prev = np.zeros(stride, dtype=np.uint8)
    src = 0
    for y in range(height):
        ftype = raw[src]
        src += 1
        line = np.frombuffer(raw, dtype=np.uint8, count=stride, offset=src).copy()
        src += stride
        _unfilter_scanline(ftype, line, prev, channels)
        out[y] = line
        prev = line
    return out.reshape(height, width, channels)


def _unfilter_scanline(ftype: int, line: np.ndarray, prev: np.ndarray, bpp: int) -> None:
    """Reverse a PNG scanline filter in place (0 None,1 Sub,2 Up,3 Avg,4 Paeth)."""
    if ftype == 0:
        return
    if ftype == 2:  # Up
        line += prev
        return
    n = line.shape[0]
    if ftype == 1:  # Sub
        for i in range(bpp, n):
            line[i] = (line[i] + line[i - bpp]) & 0xFF
        return
    if ftype == 3:  # Average
        for i in range(n):
            a = int(line[i - bpp]) if i >= bpp else 0
            b = int(prev[i])
            line[i] = (line[i] + ((a + b) >> 1)) & 0xFF
        return
    if ftype == 4:  # Paeth
        for i in range(n):
            a = int(line[i - bpp]) if i >= bpp else 0
            b = int(prev[i])
            c = int(prev[i - bpp]) if i >= bpp else 0
            p = a + b - c
            pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
            pred = a if (pa <= pb and pa <= pc) else (b if pb <= pc else c)
            line[i] = (line[i] + pred) & 0xFF
        return
    raise ValueError(f"unknown PNG filter type {ftype}")


def decode_id_png(path: Path) -> np.ndarray:
    """Decode a province-id PNG -> HxW uint32 (id = R + G*256 + B*65536)."""
    rgb = None
    try:
        from PIL import Image  # optional fast-path
        with Image.open(path) as im:
            rgb = np.asarray(im.convert("RGB"), dtype=np.uint8)
    except Exception:
        rgb = _decode_png_stdlib(path)
    ids = (rgb[:, :, 0].astype(np.uint32)
           | (rgb[:, :, 1].astype(np.uint32) << 8)
           | (rgb[:, :, 2].astype(np.uint32) << 16))
    return ids


# --------------------------------------------------------------------------
# the loaded geometry bundle
# --------------------------------------------------------------------------

@dataclass
class MapGeometry:
    width: int
    height: int
    id_raster: np.ndarray                 # HxW uint32 province id
    state_by_prov: np.ndarray             # int32[maxId+1], -1 = none
    country_colors: Dict[str, Tuple[int, int, int]]
    state_meta: Dict[int, dict]
    prov_centroids: Dict[int, Tuple[float, float]] = field(default_factory=dict)
    state_centroids: Dict[int, Tuple[float, float]] = field(default_factory=dict)
    # representative IN-PROVINCE point (actual province pixel nearest the
    # centroid). A raw centroid of a concave / multi-blob province can land
    # outside it or in the sea; markers use this instead so they sit on land.
    prov_points: Dict[int, Tuple[int, int]] = field(default_factory=dict)
    half_res: bool = True

    @property
    def max_prov_id(self) -> int:
        return int(self.state_by_prov.shape[0] - 1)


def _compute_centroids(id_raster: np.ndarray,
                       state_by_prov: np.ndarray
                       ) -> Tuple[Dict[int, Tuple[float, float]],
                                  Dict[int, Tuple[float, float]],
                                  Dict[int, Tuple[int, int]]]:
    """Per-province / per-state mean pixel (x,y) + a representative IN-province
    point per province, in a few vectorised passes.

    * province/state centroid = mean of pixel coords (for labels / fallback).
    * province representative point = the actual province pixel NEAREST the
      centroid -> always inside the province (never sea / outside a concave
      shape). This is what markers use (Codex finding #5).

    id 0 (water) is skipped.
    """
    h, w = id_raster.shape
    flat = id_raster.reshape(-1)
    max_id = int(flat.max())
    xs = np.tile(np.arange(w, dtype=np.float64), h)
    ys = np.repeat(np.arange(h, dtype=np.float64), w)

    sum_x = np.zeros(max_id + 1, dtype=np.float64)
    sum_y = np.zeros(max_id + 1, dtype=np.float64)
    cnt = np.zeros(max_id + 1, dtype=np.int64)
    np.add.at(sum_x, flat, xs)
    np.add.at(sum_y, flat, ys)
    np.add.at(cnt, flat, 1)

    prov_centroids: Dict[int, Tuple[float, float]] = {}
    nz = np.nonzero(cnt)[0]
    cx = np.zeros(max_id + 1, dtype=np.float64)
    cy = np.zeros(max_id + 1, dtype=np.float64)
    for pid in nz:
        if pid == 0:
            continue
        c = cnt[pid]
        cx[pid] = sum_x[pid] / c
        cy[pid] = sum_y[pid] / c
        prov_centroids[int(pid)] = (cx[pid], cy[pid])

    # Representative point: scan every pixel ONCE, track the pixel with the
    # smallest squared distance to its province's centroid (single linear pass,
    # O(pixels) -- no per-province search).
    best_d2 = np.full(max_id + 1, np.inf, dtype=np.float64)
    best_x = np.zeros(max_id + 1, dtype=np.int32)
    best_y = np.zeros(max_id + 1, dtype=np.int32)
    px_cx = cx[flat]                       # centroid x of each pixel's province
    px_cy = cy[flat]
    d2 = (xs - px_cx) ** 2 + (ys - px_cy) ** 2
    # reduce min per province via np.minimum.at on the distance, then a second
    # pass to recover the argmin coordinate (np has no vectorised arg-min-at).
    np.minimum.at(best_d2, flat, d2)
    # recover the coordinate that achieved each province's min distance
    is_best = d2 == best_d2[flat]
    bi = np.nonzero(is_best)[0]
    # later writes win; that's fine, any pixel at the min distance is valid
    best_x[flat[bi]] = (bi % w).astype(np.int32)
    best_y[flat[bi]] = (bi // w).astype(np.int32)
    prov_points: Dict[int, Tuple[int, int]] = {}
    for pid in nz:
        if pid == 0:
            continue
        prov_points[int(pid)] = (int(best_x[pid]), int(best_y[pid]))

    # state centroids
    sb = state_by_prov
    max_state = int(sb.max()) if sb.size else -1
    if max_state < 0:
        return prov_centroids, {}, prov_points
    s_sx = np.zeros(max_state + 1, dtype=np.float64)
    s_sy = np.zeros(max_state + 1, dtype=np.float64)
    s_cn = np.zeros(max_state + 1, dtype=np.int64)
    for pid in nz:
        if pid == 0 or pid >= sb.shape[0]:
            continue
        st = sb[pid]
        if st < 0:
            continue
        s_sx[st] += sum_x[pid]
        s_sy[st] += sum_y[pid]
        s_cn[st] += cnt[pid]
    state_centroids: Dict[int, Tuple[float, float]] = {}
    for st in np.nonzero(s_cn)[0]:
        c = s_cn[st]
        state_centroids[int(st)] = (s_sx[st] / c, s_sy[st] / c)
    return prov_centroids, state_centroids, prov_points


def load_geometry(assets_dir: Optional[Path] = None,
                  half_res: bool = True,
                  with_centroids: bool = True) -> MapGeometry:
    """Load all geometry assets read-only. ``half_res`` picks the 2816x1024 copy.

    Centroids are needed only for the division-marker layer; pass
    ``with_centroids=False`` to skip the (single-pass) centroid computation.
    """
    a = assets_dir or _assets_dir()
    png = a / ("province_ids_half.png" if half_res else "province_ids.png")
    if not png.exists():  # fall back the other way if the chosen res is absent
        png = a / ("province_ids.png" if half_res else "province_ids_half.png")
        half_res = not half_res
    id_raster = decode_id_png(png)
    h, w = id_raster.shape

    p2s = json.loads((a / "province_to_state.json").read_text(encoding="utf-8"))
    raw_colors = json.loads((a / "country_colors.json").read_text(encoding="utf-8"))
    raw_meta = json.loads((a / "state_meta.json").read_text(encoding="utf-8"))

    max_prov = int(id_raster.max())
    for k in p2s:
        v = int(k)
        if v > max_prov:
            max_prov = v
    state_by_prov = np.full(max_prov + 1, -1, dtype=np.int32)
    for k, v in p2s.items():
        pid = int(k)
        if 0 <= pid <= max_prov:
            state_by_prov[pid] = int(v)

    country_colors = {tag: tuple(int(c) for c in rgb) for tag, rgb in raw_colors.items()}
    state_meta = {int(k): v for k, v in raw_meta.items()}

    prov_c: Dict[int, Tuple[float, float]] = {}
    state_c: Dict[int, Tuple[float, float]] = {}
    prov_pts: Dict[int, Tuple[int, int]] = {}
    if with_centroids:
        prov_c, state_c, prov_pts = _compute_centroids(id_raster, state_by_prov)

    return MapGeometry(
        width=w, height=h, id_raster=id_raster, state_by_prov=state_by_prov,
        country_colors=country_colors, state_meta=state_meta,
        prov_centroids=prov_c, state_centroids=state_c, prov_points=prov_pts,
        half_res=half_res,
    )
