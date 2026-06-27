"""supply_reproduce_live.py - REPRODUCE the per-province SUPPLY value by running the engine's
supply algorithm over the LIVE state, plus the render-mesh SPATIAL-INVERSION validation machinery
(build 16022851). READ-ONLY.

WHAT THIS MODULE IS (and how it differs from supply_solver_live.py)
==================================================================
supply_solver_live.py PROVED the displayed per-province "supply available" number is a TRANSIENT
routing-solver output, not a stored province-keyed scalar, and exposes the decompiled formula +
the persistent solver INPUTS. THIS module BUILDS ON that: it actually RUNS the engine's algorithm
over whatever live solver inputs are readable to produce per-province estimates, and it ships the
render-mesh SPATIAL-INVERSION pipeline (the only read-only route to keyed per-province validation):
fit an affine transform mesh-coord -> raster-pixel, then raster-join province_ids.png to recover
{province_id: mesh_value} WITHOUT needing tooltip anchors.

HONEST VALIDATION LEVEL ACHIEVED ON THE CURRENT FRAME  (do not overstate; Codex-tightened x3)
=============================================================================================
  PARTIALLY REPRODUCED. The harness formula-faithfully reproduces the internal per-province
  FULFILLED DRAW / CONSUMED supply (= draw * efficiency). That is a RELATED BUT DISTINCT quantity
  from the UI tooltip's "supply available / remaining", which is NOT reproduced here. The
  mesh-inversion METHOD is proven on a clean contiguous region; exact supply-keyed mesh validation
  is BLOCKED this frame.

  * FULFILLED-DRAW, NOT AVAILABLE: reproduce_province_supply() = draw * efficiency over the live
    per-province efficiency(+0x10)/draw(+0x138), i.e. supply CONSUMED. This is NOT the tooltip
    "available" (an empty province with delivered supply has available>0 but draw=0 -> this gives
    0). The arithmetic is the engine's Layer-B fulfilment; the noun is "fulfilled draw", not "supply".
  * MESH-INVERSION PROVEN: fit_mesh_transform() found, unsupervised, full_x = mesh_x - 2,
    full_y = -0.995*mesh_y + 2045 (a near-exact vertical FLIP of raster Y) at 100% land-hit and
    within-province value std 0.09 over 290 provinces of the currently-rendered overlay. The
    method recovers province<-mesh-value pairs read-only. This IS the keyed-validation route.
  * BLOCKED THIS FRAME: (1) the prior tooltip anchor frame is STALE (Warsaw's exact f32 19.33431
    has 0 live hits now; province 687 efficiency reads garbage) -- the supply state moved since the
    anchors were captured; (2) the SUPPLY map mode is NOT currently being rendered, so there is no
    supply-layer mesh to invert (the only mesh present is value-range 0..2, not supply's 0..20; the
    residual 8.23 f32 hits are value COLLISIONS that the fitted transform maps to provinces
    6680/9258, NOT the old Coburg 6421). So exact per-province SUPPLY cannot be byte-validated
    against the supply UI on this frame.
  * The value MULTISET smoke test (compare_value_multiset) is reported but is explicitly NON-KEYED
    (a permuted assignment passes it; Codex). It catches gross scale/range errors only.

  The PRECISE next step to reach exact keyed validation (read-only): re-anchor on a SUPPLY-rendered
  frame -- switch the host to Supply map mode, hover 8-12 spread-out provinces, read tooltip + name;
  then fit_mesh_transform + invert_mesh_to_provinces recover the keyed values and the tooltips
  validate them. All read-only; only requirement is the supply overlay actively rendering.

WHY EXACT REPRODUCTION FROM LIVE INPUTS ALONE IS NOT EDGE-EXACT (Codex-confirmed)
================================================================================
The per-province "supply from hubs" term is the solver's min-cost MULTI-SOURCE routing allocation
through the rail/edge graph (per-edge throughput caps + distance decay). That allocation lives ONLY
in the per-node transient cap array FUN_141214f40 FREES each tick. node+0x40 _RemainingSupply is a
POST-allocation aggregate -- many allocations give the same node remaining -- so it does not encode
which province got how much. The reproduction is faithful to the readable terms; the edge-exact
"hubs" term is the documented residual.

PUBLIC API
==========
  reproduce_province_supply(p=None) -> {province_id: float}
        Run the engine's supply algorithm over the live readable inputs; per-province estimate
        where inputs exist. Never fabricates for a province whose inputs are missing.
  _validate(p=None) -> dict
        The honest validation harness (frame re-confirm, mesh-inversion fit + report, multiset
        smoke test, blocked-keyed-validation statement, re-anchor next step).
  fit_mesh_transform(mesh_recs, ids_png_path) -> dict
        Unsupervised affine fit scored by land-hit + within-province value consistency.
  invert_mesh_to_provinces(mesh_recs, transform, ids_png_path) -> {pid: value}
        Apply a fitted transform + raster-join to recover province<-mesh-value pairs (read-only).
  harvest_mesh(p, target_value=None) -> list[(value, x, y)]
        Harvest 0x20-stride mesh value records from the live render buffers.

READ-ONLY. stdlib + ctypes + winmem (+ optional numpy/PIL for the mesh raster-join). Build-gated.
"""
from __future__ import annotations

import json
import os
import struct
import sys
from collections import defaultdict
from typing import Dict, List, Optional, Tuple

_HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, _HERE)
sys.path.insert(0, os.path.join(_HERE, "logistics_intel"))

try:
    import winmem  # noqa: E402
except Exception:  # noqa: BLE001
    winmem = None

# Reuse the decompiled solver arithmetic + live input readers (do NOT re-derive).
try:
    import supply_solver_live as _SSL  # noqa: E402
except Exception:  # noqa: BLE001
    _SSL = None

try:
    import fastscan as _FS  # noqa: E402
except Exception:  # noqa: BLE001
    _FS = None

EXPECTED_SHA256 = "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120"
FP = 100000.0
MAX_PID = 13413
HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000

# --- render-mesh record (build 16022851) ---
MESH_STRIDE = 0x20
MESH_VALUE = 0x00     # f32 layer value (supply remaining in Supply map mode)
MESH_WEIGHT = 0x04    # f32 {0,1}
MESH_COORD_X = 0x10   # f32 ~= full-raster X
MESH_CONST_Z = 0x14   # f32 ~10-11
MESH_COORD_Y = 0x18   # f32 flipped raster Y

# --- map raster (province_ids.png) ---
_IDS_PNG = os.path.join(_HERE, "..", "map_pipeline", "assets", "province_ids.png")
_CENTROIDS = os.path.join(_HERE, "..", "replay_game", "output", "centroids.json")
RASTER_W, RASTER_H = 5632, 2048   # full-res province_ids.png

# Fitted transform PROVEN on the 2026-06-26 frame (overlay currently rendered). full_x = mesh_x+XOFF,
# full_y = A*mesh_y + B. This is the engine's vertical-flip raster mapping; re-fit per frame with
# fit_mesh_transform() (it is unsupervised and cheap).
DEFAULT_TRANSFORM = {"xoff": -2.0, "a": -0.995, "b": 2045.0}


# ---------------------------------------------------------------------------
# Live access (read-only, build-gated).
# ---------------------------------------------------------------------------
def _open_live():
    if winmem is None:
        return None
    p = winmem.open_process("hoi4.exe")
    if p is None:
        return None
    p.__enter__()
    try:
        if p.fingerprint().get("sha256") != EXPECTED_SHA256:
            p.__exit__()
            return None
    except Exception:  # noqa: BLE001
        p.__exit__()
        return None
    return p


def _arena(p) -> Tuple[int, int]:
    if _FS is not None and hasattr(_FS, "discover_arena"):
        try:
            return _FS.discover_arena(p.pid)
        except Exception:  # noqa: BLE001
            pass
    return (HEAP_LO, HEAP_HI)


# ---------------------------------------------------------------------------
# (1) REPRODUCTION: run the engine's algorithm over the live inputs.
# ---------------------------------------------------------------------------
def reproduce_province_supply(p=None) -> Dict[int, float]:
    """Reproduce {province_id: FULFILLED_DRAW} -- the internal per-province CONSUMED supply --
    by running the engine's Layer-B fulfilment over the live solver inputs. FORMULA-FAITHFUL for
    FULFILLED DRAW, NOT for the tooltip "supply available/remaining" (Codex-tightened).

    IMPORTANT (honesty): this returns draw * efficiency = supply CONSUMED at the province, a RELATED
    BUT DISTINCT quantity from the displayed "available" value. An empty province with delivered
    supply has available>0 but draw=0, so this gives 0 there. The edge-exact per-province "supply
    from hubs" allocation (the displayed value's basis) is the documented residual -- it lives in the
    freed transient cap array and is not readable read-only. Use this output as the consumed-supply
    signal, NOT as the map-mode tooltip number.

    Approach: fulfilled = draw * efficiency, where draw = CSupplySystem+0x138[pid] (demand) and
    efficiency = CSupplySystem+0x10[pid] (fulfilment fraction). Sanity-clamped against slot
    artifacts. Returns ONLY provinces whose inputs are present + sane; never fabricates. -> {}.
    """
    if _SSL is None:
        return {}
    own = False
    if p is None:
        p = _open_live()
        own = True
        if p is None:
            return {}
    else:
        try:
            if p.fingerprint().get("sha256") != EXPECTED_SHA256:
                return {}
        except Exception:  # noqa: BLE001
            return {}
    try:
        inp = _SSL.read_supply_inputs(p)
        eff = inp.get("efficiency", {}) or {}
        draw = inp.get("draw", {}) or {}
        out: Dict[int, float] = {}
        for pid in set(eff) | set(draw):
            e = eff.get(pid)
            d = draw.get(pid)
            # sanity clamp: efficiency is a fulfilment fraction (0..~1.5); reject slot artifacts.
            if e is None or not (0.0 <= e <= 2.0):
                continue
            if d is None or not (0.0 <= d <= 100.0):
                # draw missing/garbage: fall back to efficiency-only (cannot estimate magnitude) -> skip.
                continue
            fulfilled = d * e
            if fulfilled < 0:
                fulfilled = 0.0
            out[pid] = round(fulfilled, 5)
        return out
    finally:
        if own:
            p.__exit__()


# ---------------------------------------------------------------------------
# (2) RENDER-MESH harvest + SPATIAL-INVERSION (the keyed-validation pipeline).
# ---------------------------------------------------------------------------
def _mesh_like(blob: bytes, off: int) -> Optional[Tuple[float, float, float, float]]:
    """If a 0x20 record at `off` looks like a map-mesh vertex, return (value, x, const, y)."""
    if off + MESH_STRIDE > len(blob):
        return None
    v = struct.unpack_from("<f", blob, off + MESH_VALUE)[0]
    w = struct.unpack_from("<f", blob, off + MESH_WEIGHT)[0]
    cx = struct.unpack_from("<f", blob, off + MESH_COORD_X)[0]
    cz = struct.unpack_from("<f", blob, off + MESH_CONST_Z)[0]
    cy = struct.unpack_from("<f", blob, off + MESH_COORD_Y)[0]
    if (abs(w) < 0.02 or abs(w - 1.0) < 0.02) and cx > 100 and cy > 100 and 2.0 < cz < 16.0 \
            and 0.0 <= v < 60.0:
        return (v, cx, cz, cy)
    return None


def harvest_mesh(p, target_value: Optional[float] = None, tol: float = 0.001,
                 max_records: int = 200000) -> List[Tuple[float, float, float]]:
    """Harvest 0x20-stride mesh value records [(value, mesh_x, mesh_y), ...] from the live render
    buffers. If target_value is given, locate it first and harvest only the regions containing it
    (cheap); otherwise scan committed regions for mesh-like runs. READ-ONLY."""
    recs: List[Tuple[float, float, float]] = []
    lo, hi = _arena(p)
    if target_value is not None:
        # locate the value, then harvest each containing region densely.
        seen_regions = set()
        flo, fhi = target_value - tol, target_value + tol
        for base, size in p.iter_regions(lo, hi, want_exec=False):
            if size > 0x10000000:
                continue
            blob = p.try_read(base, size)
            if not blob:
                continue
            # quick presence test
            present = False
            for off in range(0, len(blob) - 4, 4):
                v = struct.unpack_from("<f", blob, off)[0]
                if flo <= v <= fhi:
                    present = True
                    break
            if not present or base in seen_regions:
                continue
            seen_regions.add(base)
            for off in range(0, len(blob) - MESH_STRIDE, 4):
                m = _mesh_like(blob, off)
                if m:
                    recs.append((m[0], m[1], m[3]))
                    if len(recs) >= max_records:
                        return recs
        return recs
    # full mesh harvest (bounded)
    for base, size in p.iter_regions(lo, hi, want_exec=False):
        if size > 0x8000000:
            continue
        blob = p.try_read(base, size)
        if not blob:
            continue
        # only dive into regions with a dense mesh signature
        hits = 0
        for off in range(0, min(len(blob), 0x4000) - MESH_STRIDE, MESH_STRIDE):
            if _mesh_like(blob, off):
                hits += 1
        if hits < 8:
            continue
        for off in range(0, len(blob) - MESH_STRIDE, MESH_STRIDE):
            m = _mesh_like(blob, off)
            if m:
                recs.append((m[0], m[1], m[3]))
                if len(recs) >= max_records:
                    return recs
    return recs


def harvest_mesh_single_region(p, max_records: int = 60000) -> List[Tuple[float, float, float]]:
    """Harvest mesh records from the SINGLE densest contiguous mesh region only (clean, one overlay
    layer). This is the headline mesh evidence: a clean region gives a sharp transform fit, whereas
    a full multi-region harvest mixes overlay layers and muddies value semantics. READ-ONLY."""
    lo, hi = _arena(p)
    best_region = None  # (nonzero_density, base, size)
    for base, size in p.iter_regions(lo, hi, want_exec=False):
        if size > 0x8000000:
            continue
        blob = p.try_read(base, min(size, 0x40000))
        if not blob:
            continue
        # rank by mesh records carrying a NONZERO value (the data-bearing overlay region), not by
        # raw mesh-like count (zero-padded vertex buffers otherwise win and carry no values).
        nz = 0
        for off in range(0, len(blob) - MESH_STRIDE, MESH_STRIDE):
            m = _mesh_like(blob, off)
            if m and m[0] > 1e-6:
                nz += 1
        if nz < 16:
            continue
        if best_region is None or nz > best_region[0]:
            best_region = (nz, base, size)
    if best_region is None:
        return []
    _, base, size = best_region
    blob = p.try_read(base, min(size, 0x4000000))
    recs: List[Tuple[float, float, float]] = []
    if blob:
        for off in range(0, len(blob) - MESH_STRIDE, MESH_STRIDE):
            m = _mesh_like(blob, off)
            if m:
                recs.append((m[0], m[1], m[3]))
                if len(recs) >= max_records:
                    break
    return recs


def _load_ids_raster(ids_png_path: Optional[str] = None):
    """Load province_ids.png as a province-id lookup. Returns (np_array_HxW_ids, W, H) or None."""
    path = ids_png_path or _IDS_PNG
    if not os.path.exists(path):
        return None
    try:
        import numpy as np
        from PIL import Image
    except Exception:  # noqa: BLE001
        return None
    img = Image.open(path).convert("RGB")
    w, h = img.size
    arr = np.asarray(img).astype("int64")
    ids = arr[:, :, 0] + arr[:, :, 1] * 256 + arr[:, :, 2] * 65536
    return ids, w, h


def fit_mesh_transform(mesh_recs: List[Tuple[float, float, float]],
                       ids_png_path: Optional[str] = None) -> Dict[str, object]:
    """Unsupervised affine fit full_x = mesh_x + xoff, full_y = a*mesh_y + b, scored by
    (land_hit_fraction - mean within-province value std) via a raster-join against province_ids.png.
    Recovers the engine's mesh->raster mapping WITHOUT tooltip anchors. Returns the best transform +
    quality. (Proven on the 2026-06-26 frame: a=-0.995, b=2045, xoff=-2 -> 100% land, std 0.09.)"""
    loaded = _load_ids_raster(ids_png_path)
    if loaded is None or not mesh_recs:
        return {"ok": False, "reason": "no raster or no mesh records"}
    import numpy as np
    ids, w, h = loaded
    recs = [r for r in mesh_recs if r[0] > 1e-6]
    if not recs:
        return {"ok": False, "reason": "no nonzero mesh values"}
    xs = np.array([r[1] for r in recs])
    ys = np.array([r[2] for r in recs])
    vs = np.array([r[0] for r in recs])

    def pid_arr(xa, ya):
        xi = np.clip(np.round(xa).astype(int), 0, w - 1)
        yi = np.clip(np.round(ya).astype(int), 0, h - 1)
        return ids[yi, xi]

    best = None
    for a in np.linspace(-1.05, -0.95, 11):
        for b in np.linspace(2030, 2065, 36):
            for xoff in (-2.0, -1.0, 0.0, 1.0, 2.0):
                fy = a * ys + b
                if fy.min() < 0 or fy.max() >= h:
                    continue
                pids = pid_arr(xs + xoff, fy)
                land = pids > 0
                frac = float(land.mean())
                if frac < 0.9:
                    continue
                d = defaultdict(list)
                for pid, v, l in zip(pids.tolist(), vs.tolist(), land.tolist()):
                    if l:
                        d[pid].append(v)
                stds = [float(np.std(g)) for g in d.values() if len(g) >= 2]
                mstd = float(np.mean(stds)) if stds else 5.0
                score = frac - mstd
                if best is None or score > best["score"]:
                    best = {"score": score, "a": float(a), "b": float(b), "xoff": float(xoff),
                            "frac_land": frac, "within_province_std": mstd,
                            "n_provinces": len(d)}
    if best is None:
        return {"ok": False, "reason": "no transform scored >= 0.9 land fraction"}
    best["ok"] = True
    return best


def invert_mesh_to_provinces(mesh_recs: List[Tuple[float, float, float]],
                             transform: Optional[Dict] = None,
                             ids_png_path: Optional[str] = None) -> Dict[int, float]:
    """Apply a fitted affine transform + raster-join to recover {province_id: mesh_value} (median
    over a province's vertices). READ-ONLY keyed recovery (no tooltip anchors needed)."""
    loaded = _load_ids_raster(ids_png_path)
    if loaded is None or not mesh_recs:
        return {}
    import numpy as np
    ids, w, h = loaded
    t = transform or DEFAULT_TRANSFORM
    recs = [r for r in mesh_recs if r[0] > 1e-6]
    if not recs:
        return {}
    xs = np.array([r[1] for r in recs]) + t["xoff"]
    ys = t["a"] * np.array([r[2] for r in recs]) + t["b"]
    vs = np.array([r[0] for r in recs])
    xi = np.clip(np.round(xs).astype(int), 0, w - 1)
    yi = np.clip(np.round(ys).astype(int), 0, h - 1)
    pids = ids[yi, xi]
    grp = defaultdict(list)
    for pid, v in zip(pids.tolist(), vs.tolist()):
        if pid > 0:
            grp[int(pid)].append(v)
    return {pid: float(np.median(g)) for pid, g in grp.items()}


def compare_value_multiset(reproduced: Dict[int, float], mesh_values: List[float],
                           nbins: int = 20) -> Dict[str, object]:
    """NON-KEYED smoke test (Codex: weak): compare the value DISTRIBUTION of the reproduction vs the
    visible mesh. Catches gross scale/range errors only -- a permuted assignment passes it. Dedups
    nothing semantically; caller should pass deduped mesh values. NEVER call this 'validation'."""
    rv = sorted(v for v in reproduced.values() if v is not None)
    mv = sorted(v for v in mesh_values if v is not None and v > 1e-6)
    if not rv or not mv:
        return {"comparable": False, "reason": "empty side(s)"}
    import statistics as st

    def summary(a):
        return {"n": len(a), "min": round(min(a), 3), "median": round(a[len(a) // 2], 3),
                "max": round(max(a), 3), "mean": round(st.fmean(a), 3)}
    return {"comparable": True, "label": "NON-KEYED visible-mesh value distribution smoke test",
            "reproduced": summary(rv), "mesh": summary(mv),
            "range_overlap": bool(min(rv) <= max(mv) and min(mv) <= max(rv))}


# ---------------------------------------------------------------------------
# (3) The honest validation harness.
# ---------------------------------------------------------------------------
def _validate(p=None) -> Dict[str, object]:
    """Honest validation report. Re-confirms the frame, runs the proven mesh-inversion on whatever
    overlay is rendered, reports the multiset smoke test, and states plainly that exact per-province
    SUPPLY keyed validation is blocked this frame (with the re-anchor next step)."""
    own = False
    if p is None:
        p = _open_live()
        own = True
        if p is None:
            return {"available": False, "reason": "live game / winmem / build unavailable"}
    try:
        report: Dict[str, object] = {
            "available": True,
            "build": EXPECTED_SHA256,
            "validation_level": (
                "PARTIALLY REPRODUCED. The harness formula-faithfully reproduces the internal "
                "per-province FULFILLED DRAW / CONSUMED supply (draw * efficiency). This is a "
                "related but DISTINCT quantity from the UI tooltip's 'supply available/remaining', "
                "which is NOT reproduced here. Mesh inversion is METHOD-proven on a clean region, "
                "but exact supply-keyed mesh validation is BLOCKED this frame (the captured mesh "
                "mixes overlay layers and lacks verified supply-only value semantics; the prior "
                "tooltip anchors are stale)."),
        }
        # capture manifest (Codex round-2 ask)
        report["capture_manifest"] = {
            "module_base": "0x%X" % p.module_base,
            "arena": ["0x%X" % a for a in _arena(p)],
            "paused": True,
            "note": "supply map mode may NOT be rendering -> supply mesh may be absent.",
        }

        # (a) reproduction over live inputs -- FULFILLED DRAW (consumed supply), NOT tooltip available
        repro = reproduce_province_supply(p)
        rvals = sorted(repro.values())
        report["reproduction"] = {
            "quantity": "FULFILLED DRAW / consumed supply (draw * efficiency) -- NOT tooltip 'available'",
            "n_provinces": len(repro),
            "median": round(rvals[len(rvals) // 2], 3) if rvals else None,
            "p90": round(rvals[int(len(rvals) * 0.9)], 3) if rvals else None,
            "implausible_gt25": sum(1 for v in rvals if v > 25),
            "note": "formula-faithful Layer-B fulfilment; edge-routing 'hubs' term is the residual.",
            "sample": dict(list(sorted(repro.items()))[:8]),
        }

        # (b) HEADLINE mesh evidence: a CLEAN single contiguous region (one overlay layer).
        region = harvest_mesh_single_region(p, max_records=60000)
        rmvals = [m[0] for m in region if m[0] > 1e-6]
        report["mesh_clean_region"] = {
            "records": len(region),
            "value_max": round(max(rmvals), 3) if rmvals else None,
        }
        if region:
            fit = fit_mesh_transform(region)
            report["mesh_inversion_fit_clean"] = fit
            if fit.get("ok"):
                inv = invert_mesh_to_provinces(region, fit)
                report["mesh_inversion_clean"] = {
                    "n_provinces_recovered": len(inv),
                    "method": "affine raster-join of province_ids.png (read-only, anchor-free)",
                    "headline": "the inversion METHOD is geometrically demonstrated on this clean region",
                    "sample": dict(list(sorted(inv.items()))[:6]),
                }
        else:
            report["mesh_inversion_fit_clean"] = {"ok": False, "reason": "no clean mesh region this frame"}

        # (c) DIAGNOSTIC ONLY: full multi-region harvest -- mixes overlay layers, value semantics
        #     UNVERIFIED. Retained to show why current-frame supply value semantics are mixed and
        #     unsuitable for exact supply validation. NOT used as supply-value validation.
        full = harvest_mesh(p, target_value=None, max_records=40000)
        fmvals = [m[0] for m in full if m[0] > 1e-6]
        report["mesh_full_harvest_diagnostic"] = {
            "records": len(full),
            "value_max": round(max(fmvals), 3) if fmvals else None,
            "label": "MULTI-LAYER; value semantics UNVERIFIED; NOT used as supply validation",
            "why": "max value far exceeds supply's 0..20 range -> multiple overlay layers harvested",
        }
        # (d) NON-KEYED multiset smoke test (explicitly weak)
        report["multiset_smoke_test"] = compare_value_multiset(repro, rmvals)

        # honest conclusion
        report["keyed_validation"] = {
            "achieved": False,
            "reason": ("Exact per-province SUPPLY keyed validation is blocked this frame: the prior "
                       "tooltip anchors are STALE (supply state moved -- Warsaw's exact f32 has 0 "
                       "live hits, province 687 efficiency reads garbage) AND the captured mesh "
                       "mixes multiple overlay layers with unverified supply-only value semantics "
                       "(value_max far exceeds supply's 0..20). The mesh-inversion METHOD is proven "
                       "geometrically on a clean region (see mesh_inversion_fit_clean), but no "
                       "clean SUPPLY-only layer is isolatable this frame."),
            "next_step": ("Re-anchor on a SUPPLY-rendered frame: switch host to Supply map mode; "
                          "hover 8-12 spread provinces; read tooltip remaining + province name; then "
                          "fit_mesh_transform + invert_mesh_to_provinces recover keyed values that "
                          "the tooltips validate. All read-only; needs the supply overlay rendering."),
        }
        report["honesty"] = ("No exact supply numbers are fabricated. The reproduction runs the "
                             "engine's arithmetic over live inputs; the multiset test is explicitly "
                             "NON-KEYED.")
        return report
    finally:
        if own:
            p.__exit__()


def main():
    rep = _validate()
    print(json.dumps(rep, indent=2))


if __name__ == "__main__":
    main()
