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

The adjacency is a static function of the map; it is cached on disk inside the
derive package (``derive/_cache``), keyed by resolution + assets-dir identity
and fingerprinted against the source raster/mapping bytes so repeated derive
runs do not re-scan the raster and a swapped or regenerated raster can never
serve a stale cache.
"""

from __future__ import annotations

import hashlib
import json
import os
from typing import Dict, Iterable, List, Optional, Set, Tuple

import numpy as np


class ProvinceRasterMismatch(RuntimeError):
    """Typed refusal: the province-id raster on disk cannot be validated against
    the frozen ``province_ids_meta.json`` contract (meta absent, dimensions
    drifted, or land provinces missing from the raster). Fail closed: adjacency
    derived from an unvalidated raster could be silently wrong."""


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


def _resolve_raster_path(assets_dir: str, half_res: bool) -> str:
    """Path of the REQUESTED-resolution province-id PNG, or a typed refusal.

    Never substitutes the other resolution: an explicit request for one raster
    must not silently degrade (or upgrade) to the other, because the result
    would be cached and labeled under the requested resolution (fail-closed
    doctrine: visible refusal, not silent approximation).
    """
    want = "province_ids_half.png" if half_res else "province_ids.png"
    other = "province_ids.png" if half_res else "province_ids_half.png"
    png = os.path.join(assets_dir, want)
    if not os.path.exists(png):
        other_path = os.path.join(assets_dir, other)
        raise FileNotFoundError(
            f"adjacency: required {'half' if half_res else 'full'}-res "
            f"province-id raster missing: {png} (other-resolution candidate "
            f"{other_path} "
            f"{'exists' if os.path.exists(other_path) else 'also missing'}; "
            "refusing to substitute a different resolution). These PNGs are "
            "gitignored with no in-repo generator; restore the asset from a "
            "machine that has it.")
    return png


def _load_meta(assets_dir: str) -> dict:
    """Load ``province_ids_meta.json`` or refuse (typed) -- the raster must
    never be consumed unvalidated."""
    meta_path = os.path.join(assets_dir, "province_ids_meta.json")
    if not os.path.exists(meta_path):
        raise ProvinceRasterMismatch(
            f"adjacency: province_ids_meta.json missing from {assets_dir}; "
            "cannot validate the province-id raster -- refusing to proceed "
            "unvalidated")
    with open(meta_path, "r", encoding="utf-8") as fh:
        return json.load(fh)


def _validate_raster(png: str, ids: np.ndarray, land_ids: Set[int],
                     meta: dict) -> None:
    """Validate the decoded raster against the frozen meta contract.

    Checks (all hard refusals via :class:`ProvinceRasterMismatch`):
      * dimensions match the meta entry for the basename actually opened;
      * every land province id from ``province_to_state.json`` appears in the
        raster (meta asserts ``provinces_lost_at_this_scale: 0``, so this is
        strict -- a lossy re-encode / wrong-map raster fails here).
    """
    base = os.path.basename(png)
    if base == "province_ids.png":
        expected = (int(meta["source_height"]), int(meta["source_width"]))
    elif base == "province_ids_half.png":
        d = meta["downscaled_display"]
        expected = (int(d["height"]), int(d["width"]))
    else:
        raise ProvinceRasterMismatch(
            f"adjacency: no meta contract for raster basename {base!r}")
    if tuple(ids.shape) != expected:
        raise ProvinceRasterMismatch(
            f"adjacency: raster {png} has shape (H, W) = {tuple(ids.shape)} "
            f"but province_ids_meta.json requires {expected}; the raster on "
            "disk is not the one the meta was frozen against -- refusing")
    missing = land_ids - {int(x) for x in np.unique(ids)}
    if missing:
        sample = sorted(missing)[:10]
        raise ProvinceRasterMismatch(
            f"adjacency: raster {png} is missing {len(missing)} land province "
            f"id(s) present in province_to_state.json (sample: {sample}); "
            "meta asserts provinces_lost_at_this_scale: 0 -- raster/mapping "
            "mismatch, refusing")


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
    png = _resolve_raster_path(a, half_res)
    ids = _decode_id_png(png)

    p2s_path = os.path.join(a, "province_to_state.json")
    with open(p2s_path, "r", encoding="utf-8") as fh:
        p2s = json.load(fh)
    land_ids = {int(k) for k in p2s}

    _validate_raster(png, ids, land_ids, _load_meta(a))

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
# disk cache (keyed by res + assets-dir digest, content-fingerprinted against
# the source raster/mapping bytes; kept INSIDE the derive package, never
# written into the read-only map_pipeline assets dir).
# ---------------------------------------------------------------------------
_CACHE_SCHEMA = 1


def _cache_dir() -> str:
    d = os.path.join(os.path.dirname(os.path.abspath(__file__)), "_cache")
    os.makedirs(d, exist_ok=True)
    return d


def _cache_path(assets_dir: str, half_res: bool) -> str:
    suffix = "half" if half_res else "full"
    # Key by resolution AND assets-dir identity so two different --assets dirs
    # never share (or thrash) one cache file. normcase+abspath collapses
    # spelling differences for the same directory (Windows case-insensitivity,
    # relative vs absolute) so they don't cause spurious misses.
    key = hashlib.sha1(
        os.path.normcase(os.path.abspath(assets_dir)).encode("utf-8")
    ).hexdigest()[:10]
    return os.path.join(_cache_dir(),
                        f"province_adjacency_{suffix}_{key}.json")


def _source_fingerprint(assets_dir: str, half_res: bool) -> str:
    """sha256 over the resolved raster + province_to_state.json +
    province_ids_meta.json bytes, plus the raster basename.

    Content-keyed (never size+mtime): a regenerated, swapped, or re-encoded
    source file always invalidates the cache and forces a validated rebuild.
    Missing inputs are typed refusals -- a cache must never be served for
    sources that cannot be fingerprinted.
    """
    png = _resolve_raster_path(assets_dir, half_res)
    h = hashlib.sha256()
    h.update(os.path.basename(png).encode("utf-8"))
    for path in (png,
                 os.path.join(assets_dir, "province_to_state.json"),
                 os.path.join(assets_dir, "province_ids_meta.json")):
        if not os.path.exists(path):
            raise ProvinceRasterMismatch(
                f"adjacency: required source input missing: {path}; cannot "
                "fingerprint the adjacency sources -- refusing")
        with open(path, "rb") as fh:
            h.update(fh.read())
    return h.hexdigest()


def load_or_build_adjacency(assets_dir: Optional[str] = None,
                            half_res: bool = True,
                            use_cache: bool = True) -> Dict[int, List[int]]:
    """Adjacency with an on-disk cache inside the derive package (idempotent).

    The cache is served only when its stored ``source_fingerprint`` matches
    the current source bytes; any legacy/foreign/stale cache file falls
    through to an exact rebuild (rebuild, not warn -- the rebuild is exact,
    so no blocker is needed).
    """
    a = assets_dir or default_assets_dir()
    cache = _cache_path(a, half_res)
    fingerprint = _source_fingerprint(a, half_res) if use_cache else None
    if use_cache and os.path.exists(cache):
        try:
            with open(cache, "r", encoding="utf-8") as fh:
                raw = json.load(fh)
            if (isinstance(raw, dict)
                    and raw.get("schema") == _CACHE_SCHEMA
                    and raw.get("source_fingerprint") == fingerprint
                    and isinstance(raw.get("adjacency"), dict)):
                return {int(k): list(v)
                        for k, v in raw["adjacency"].items()}
            # old flat format / wrong schema / stale fingerprint: rebuild
        except Exception:
            pass  # rebuild on any cache corruption
    adj = build_adjacency(a, half_res=half_res)
    if use_cache:
        try:
            with open(cache, "w", encoding="utf-8") as fh:
                json.dump({"schema": _CACHE_SCHEMA,
                           "source_fingerprint": fingerprint,
                           "adjacency": {str(k): v for k, v in adj.items()}},
                          fh, separators=(",", ":"))
        except Exception:
            pass  # cache write is best-effort; never fatal
    return adj


def iter_edges(adj: Dict[int, List[int]]) -> Iterable[Tuple[int, int]]:
    """Yield each unordered land adjacency edge once (a < b)."""
    for a, nbrs in adj.items():
        for b in nbrs:
            if a < b:
                yield (a, b)
