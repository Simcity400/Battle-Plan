"""Province adjacency from the authoritative province-id raster.

Pure offline data prep. Reuses the map pipeline's ``province_ids*.png`` (each
pixel's RGB IS the province id; see ``map_pipeline/README.md``) to compute the
set of LAND province-pairs that share a pixel border.

Model (deliberately simple and documented):

  * 4-neighbour contact only. A province pixel is "adjacent" to the province on
    its immediate RIGHT or immediate DOWN neighbour. Diagonal-only contact does
    NOT count (Codex finding D: diagonal touch is not a the game border).
  * NO wraparound. We compare ``ids[:, :-1]`` vs ``ids[:, 1:]`` (and the down
    shift) by SLICING, never ``np.roll`` -- so the right/bottom map edges never
    fabricate an adjacency across the seam (Codex finding D).
  * LAND only. Sea/lake provinces have non-zero ids in the raster but are absent
    from ``province_to_state.json``; we drop any pair touching a non-land id
    (and id 0, the "no province" sentinel). Membership is by the land set, not
    by ``id == 0`` alone (Codex finding D, last bullet).

KNOWN LIMITS (documented, not bugs):
  * Scripted straits / sea crossings in the mod's ``adjacencies.csv`` are NOT
    represented -- raster contact only. So an island reachable only by a strait
    looks non-adjacent here. This makes the encirclement model slightly OVER-
    eager for strait-connected land (it can't see the strait), never the reverse.
  * Impassable-border terrain is not filtered (we have no terrain adjacency
    asset). A border that exists on the map but is impassable still counts.

The adjacency is a static function of the map; it is cached on disk next to the
assets so repeated derive runs do not re-scan the raster.
"""

from __future__ import annotations

import json
import os
from typing import Dict, Iterable, List, Optional, Set, Tuple

import numpy as np

ADJ_CACHE_BASENAME = "province_adjacency.json"


# ---------------------------------------------------------------------------
# Raster decode is self-contained so this package does not depend on old render tools.
# ---------------------------------------------------------------------------
def _decode_id_png(path: str) -> np.ndarray:
    """Decode a province-id PNG -> HxW uint32 (id = R + G*256 + B*65536)."""
    from PIL import Image  # numpy + Pillow are both available in this repo

    with Image.open(path) as im:
        rgb = np.asarray(im.convert("RGB"), dtype=np.uint8)
    return (rgb[:, :, 0].astype(np.uint32)
            | (rgb[:, :, 1].astype(np.uint32) << 8)
            | (rgb[:, :, 2].astype(np.uint32) << 16))


def default_assets_dir() -> str:
    here = os.path.dirname(os.path.abspath(__file__))
    return os.path.normpath(os.path.join(here, os.pardir, "map_pipeline", "assets"))


# ---------------------------------------------------------------------------
# adjacency build
# ---------------------------------------------------------------------------
def _pairs_from_raster(ids: np.ndarray, land: np.ndarray) -> Set[Tuple[int, int]]:
    """Set of unordered (a<b) LAND province-id pairs sharing a 4-neighbour edge.

    ``land`` is a boolean lookup indexed by province id (land[id] == True for a
    real land province). Pairs are filtered so BOTH endpoints are land and the
    two ids differ.
    """
    pairs: Set[Tuple[int, int]] = set()

    def _collect(a: np.ndarray, b: np.ndarray) -> None:
        # a,b are equal-shape id arrays of touching pixels (sliced, no wrap).
        diff = a != b
        if not diff.any():
            return
        av = a[diff].astype(np.int64)
        bv = b[diff].astype(np.int64)
        # both endpoints must be land (drops id 0 and all sea/lake ids)
        keep = land[av] & land[bv]
        av = av[keep]
        bv = bv[keep]
        if av.size == 0:
            return
        lo = np.minimum(av, bv)
        hi = np.maximum(av, bv)
        # unique unordered pairs in this batch, then union into the set
        stacked = np.stack([lo, hi], axis=1)
        uniq = np.unique(stacked, axis=0)
        for row in uniq:
            pairs.add((int(row[0]), int(row[1])))

    # right neighbour: column j vs j+1  (slice, never roll -> no x-wrap)
    _collect(ids[:, :-1], ids[:, 1:])
    # down neighbour: row i vs i+1      (slice, never roll -> no y-wrap)
    _collect(ids[:-1, :], ids[1:, :])
    return pairs


def build_adjacency(assets_dir: Optional[str] = None,
                    half_res: bool = True) -> Dict[int, List[int]]:
    """Compute land province adjacency from the id raster.

    Returns ``{province_id: sorted([neighbour_ids])}``. Half-res is the default
    (verified by the map pipeline to lose 0 provinces at x2) and is ~4x faster;
    pass ``half_res=False`` for the authoritative 5632x2048 raster.
    """
    a = assets_dir or default_assets_dir()
    png = os.path.join(a, "province_ids_half.png" if half_res
                       else "province_ids.png")
    if not os.path.exists(png):
        png = os.path.join(a, "province_ids.png" if half_res
                           else "province_ids_half.png")
    ids = _decode_id_png(png)

    p2s_path = os.path.join(a, "province_to_state.json")
    with open(p2s_path, "r", encoding="utf-8") as fh:
        p2s = json.load(fh)
    land_ids = {int(k) for k in p2s}

    max_id = int(ids.max())
    for pid in land_ids:
        if pid > max_id:
            max_id = pid
    land = np.zeros(max_id + 1, dtype=bool)
    for pid in land_ids:
        if 0 <= pid <= max_id:
            land[pid] = True

    pairs = _pairs_from_raster(ids, land)

    adj: Dict[int, Set[int]] = {}
    for x, y in pairs:
        adj.setdefault(x, set()).add(y)
        adj.setdefault(y, set()).add(x)
    return {pid: sorted(nbrs) for pid, nbrs in adj.items()}


# ---------------------------------------------------------------------------
# disk cache (keyed by res; kept INSIDE the derive package, never written into
# the read-only map_pipeline assets dir).
# ---------------------------------------------------------------------------
def _cache_dir() -> str:
    d = os.path.join(os.path.dirname(os.path.abspath(__file__)), "_cache")
    os.makedirs(d, exist_ok=True)
    return d


def _cache_path(assets_dir: str, half_res: bool) -> str:
    suffix = "half" if half_res else "full"
    return os.path.join(_cache_dir(), f"province_adjacency_{suffix}.json")


def load_or_build_adjacency(assets_dir: Optional[str] = None,
                            half_res: bool = True,
                            use_cache: bool = True) -> Dict[int, List[int]]:
    """Adjacency with an on-disk cache next to the assets (idempotent)."""
    a = assets_dir or default_assets_dir()
    cache = _cache_path(a, half_res)
    if use_cache and os.path.exists(cache):
        try:
            with open(cache, "r", encoding="utf-8") as fh:
                raw = json.load(fh)
            return {int(k): list(v) for k, v in raw.items()}
        except Exception:
            pass  # rebuild on any cache corruption
    adj = build_adjacency(a, half_res=half_res)
    if use_cache:
        try:
            with open(cache, "w", encoding="utf-8") as fh:
                json.dump({str(k): v for k, v in adj.items()}, fh,
                          separators=(",", ":"))
        except Exception:
            pass  # cache is best-effort; never fatal
    return adj


def iter_edges(adj: Dict[int, List[int]]) -> Iterable[Tuple[int, int]]:
    """Yield each unordered land adjacency edge once (a < b)."""
    for a, nbrs in adj.items():
        for b in nbrs:
            if a < b:
                yield (a, b)
