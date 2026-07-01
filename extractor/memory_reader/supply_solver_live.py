"""supply_solver_live.py - LIVE per-province SUPPLY via solver-input reproduction (build 16022851).

WHAT THIS MODULE IS
===================
The per-province DISPLAYED "supply available" number (the Supply map-mode tooltip, e.g. a
province showing 19.34) is a COMPUTED, TRANSIENT routing-solver OUTPUT. This module is the
result of the mandate "reproduce the solver from its live inputs". It delivers:

  1. The decompiled solver FORMULA as pure functions (the exact arithmetic, build 16022851).
  2. LIVE READ-ONLY readers for every PERSISTENT solver INPUT that IS stored:
       - per-province supply EFFICIENCY  (CSupplySystem+0x10 [raw pid], i32/1e5)
       - per-province supply DRAW/demand  (CSupplySystem+0x138 [raw pid], i32/1e5)
       - the province -> DENSE-INDEX remap (FUN_140bac110: remap[pid], *(gamestate+0x340))
  3. The honest finding (proven this session, decompile + 4-anchor live test): the displayed
     per-province value is NOT persisted as any province-keyed scalar (not in CProvince, not in
     any CSupplySystem dense- or raw-indexed array), so province_supply_exact() returns {} and
     the usable per-province signal is the division proxy + the efficiency/draw inputs.

WHY THE EXACT DISPLAYED VALUE IS NOT READ-ONLY-PINNABLE (decompile-grounded, country_supply.cpp)
==============================================================================================
* FUN_141214f40 (per supply NODE) allocates a TRANSIENT per-dense-province int64 cap array via
  FUN_140604e20 (init 100000 == 1.0 fixed point), does arr[FUN_140bac110(prov)] = min(arr,
  delivered_cap) down each routing path, calls FUN_141213980 to deliver, then FREES the array at
  function exit. The per-province "available cap" is a solver LOCAL, gone after the tick.
* FUN_141213980 (the delivery) writes delivered supply into the CONSUMER object (consumer+0x20 =
  GetReceivedSupply) and appends a per-CONSUMER record (stride 0x28); it writes NO per-province /
  per-dense-index array. So delivered supply is stored PER-CONSUMER (usage), never per-province.
* FUN_140bac110(prov) = remap[pid] (dense index), remap = *(gamestate+0x340),
  gamestate = *(module+0x33048C0). Verified live (3324->61, 3544->6183, 6421->6995, 9244->11603).
* LIVE 4-anchor test (Gdynia 3324=10.05, Warsaw 3544=19.34, Coburg 6421=8.23, Chuvashia
  9244=0.99): the CProvince sub-record chain body+0x190->+0x348->+0x108 matched ONLY 3324 (a
  coincidence; null for Warsaw, 7.09 for Coburg, 2.84 for Chuvashia). The displayed value DOES
  appear inside CProvince band sub-records but at province-INCONSISTENT offsets (the per-consumer/
  per-route records), never a single stable (band,mid,leaf). The CSupplySystem+0x40[dense]*0x78
  record holds it at NO consistent offset for >=2 anchors; a brute over EVERY CSupplySystem member
  array base[dense*elem] (the CORRECT dense index, which prior raw-pid passes never used) hit NO
  array for >=3 anchors. The value only materializes transiently in the freed per-node cap array
  and in the camera-ordered render mesh (no province key) - both non-persistent.

VALIDATION STATEMENT (honest; Codex-reviewed, adversarial, 3 passes)
====================================================================
On build 16022851 we found NO persistent province-keyed storage for the displayed per-province
"supply available" value in CProvince or CSupplySystem, using both raw province ids AND the
verified dense province remap. The value is produced through the supply solver's transient
per-node dense-province cap array and then consumed by delivery/UI/render paths; it is not exposed
as a stable paused-frame per-province scalar suitable for a read-only lookup. Exact reproduction
requires REPLAYING the routing solver with its per-edge inputs (no province lookup exists). The
supported live read-only outputs are the persistent inputs/signals: per-province efficiency, draw/
demand, the dense remap, and the validated division received-supply proxy. The decompiled formula
(below) is exact GIVEN its per-edge routing inputs.

PUBLIC API
==========
  province_supply_exact(output_dir) -> {province_id: float}
        T0 seam for models/supply_number.py. Returns {} (honest: exact displayed value is
        transient/render-only, not a read-only province scalar). The model then degrades to its
        division-proxy (T1) and structural (T2) tiers - never an invented number.
  read_supply_inputs(p=None) -> dict
        LIVE persistent inputs: {efficiency:{pid:float}, draw:{pid:float}, dense_index:{pid:int},
        meta:{...}}. These ARE stored and validated sensible (efficiency 8769/13414 nonzero,
        0..1 fulfilment; e.g. 3324=0.95, 3544=1.00).
  province_dense_index(p, pid) -> int            (FUN_140bac110 reproduced live)
  supply_path_delivered(...)                     (pure Layer-A routing arithmetic)
  country_alloc_cap(...)                         (pure Layer-B per-consumer cap)

READ-ONLY. stdlib + ctypes + winmem. Build-gated to the sha256 below. No writes to game memory.
"""
from __future__ import annotations

import json
import os
import struct
import sys
from typing import Dict, List, Optional

_HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, _HERE)                                  # memory_reader/
sys.path.insert(0, os.path.join(_HERE, "logistics_intel"))  # for supply_live / supply_solver

try:  # live deps optional (pure formula works without them)
    import winmem  # noqa: E402
except Exception:  # noqa: BLE001
    winmem = None

EXPECTED_SHA256 = "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120"
FP = 100000.0                       # CFixedPoint one unit (140aceec0.c returns 100000)
MAX_PID = 13413
HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000

# --- module-relative statics / offsets (build 16022851) ---
GAMESTATE_RVA = 0x33048C0           # DAT_1433048c0: pointer to the gamestate singleton
REMAP_OFF = 0x340                   # gamestate+0x340 -> u32[pid] dense-index remap table (FUN_140bac110)
PROV_VT_OFF = 0x2950A50             # CProvince pool vtable (module-relative); id u32 @ body+0x08
OFF_PROV_ID = 0x08

# CSupplySystem world manager (double vtable +0/+8 @ RVA 0x294CFB8/0x294CFE0).
CSS_VT_RVA = (0x294CFB8, 0x294CFE0)
CSS_PROV_EFF = 0x10                 # CPdxArray<i64> count 13414, [raw pid]: efficiency/fulfilment.
CSS_PROV_DRAW = 0x138              # CPdxArray<i64> count 13414, [raw pid]: supply draw/demand.
# Encoding (verified live): fixed-point value in the LOW i32 of each i64 slot, scale 1e5; read
# i32 at base + pid*8 (stride 8). pid*4 reads garbage (the prior supply_solver.py diagnostic used
# pid*4 - a latent bug in that out-of-lane file; this reader uses the verified pid*8).


# ---------------------------------------------------------------------------
# Pure decompiled solver arithmetic (exact; the OPEN part is the per-edge inputs).
# ---------------------------------------------------------------------------
def supply_path_delivered(
    hop_demands: List[int],
    total_capacity: int,
    reserved: int,
    throughput_calc0: int,
    throughput_calc1: int,
) -> int:
    """Layer-A routing cut-off: one edge's per-province delivered supply (fixed-point x1e5),
    EXACTLY as the decompiled UpdateSupply distribution loop (FUN_140ebd4f0:1382-1502 /
    country_supply.cpp). available = cap-reserved; ratio = available*1e5/cap; cap_ratio =
    min(calc1*1e5/calc0, 1e5); per hop base*cap_ratio/1e5, integer-truncated (compounds the
    sub-1.0 ratios -> tail provinces collapse to ~0). Exact given correct inputs."""
    if total_capacity == 0:
        return 0
    available = total_capacity - reserved
    availability_ratio = (available * 100000) // total_capacity
    if throughput_calc0 == 0:
        cap_ratio = 100000
    else:
        cap_ratio = (throughput_calc1 * 100000) // throughput_calc0
        if cap_ratio > 100000:
            cap_ratio = 100000
    remaining_available = available
    total_delivered = 0
    last = len(hop_demands) - 1
    for i, demand in enumerate(hop_demands):
        base = (availability_ratio * demand) // 100000 if i != last else remaining_available
        delivered = (base * cap_ratio) // 100000
        if delivered > 0:
            remaining_available -= delivered
            total_delivered += delivered
    return total_delivered


def country_alloc_cap(capacity_units: int, used_units: int, demand_units: int) -> int:
    """Layer-B per-consumer allocation cap = min(capacity-used, demand), whole units
    (cls_CCountrySupplySystem 14121c210:25,32-34)."""
    remaining = capacity_units - used_units
    if remaining <= 0:
        return 0
    return demand_units if demand_units < remaining else remaining


# ---------------------------------------------------------------------------
# Live helpers (read-only).
# ---------------------------------------------------------------------------
def _open_live():
    if winmem is None:
        return None
    p = winmem.open_process("hoi4.exe")
    if p is None:
        return None
    p.__enter__()
    if p.fingerprint().get("sha256") != EXPECTED_SHA256:
        p.__exit__()
        return None
    return p


def _find_csupplysystem(p) -> Optional[int]:
    """The world CSupplySystem instance (double vtable @+0/+8), prefer the 13414-count +0x10 array."""
    vts = {p.module_base + r for r in CSS_VT_RVA}
    vt_bytes = [v.to_bytes(8, "little") for v in vts]
    cands: List[int] = []
    for base, size in p.iter_regions(HEAP_LO, HEAP_HI, want_exec=False):
        if size > 0x10000000:
            continue
        blob = p.try_read(base, size)
        if not blob:
            continue
        for t in vt_bytes:
            s = 0
            while True:
                j = blob.find(t, s)
                if j < 0:
                    break
                s = j + 8
                if j + 16 <= len(blob):
                    nxt = struct.unpack_from("<Q", blob, j + 8)[0]
                    if nxt in vts:
                        cands.append(base + j)
    for h in sorted(set(cands)):
        try:
            eff_base = p.u64(h + CSS_PROV_EFF)
            eff_cnt = p.u32(h + CSS_PROV_EFF + 8)
            draw_cnt = p.u32(h + CSS_PROV_DRAW + 8)
        except OSError:
            continue
        # require BOTH per-province arrays to be province-count sized and the eff base in-heap.
        if (eff_base and HEAP_LO <= eff_base < HEAP_HI
                and 13000 <= eff_cnt <= 14000 and 13000 <= draw_cnt <= 14000):
            return h
    # fail-closed: do NOT guess an unvalidated instance on a build-specific reader.
    return None


def province_dense_index(p, pid: int) -> Optional[int]:
    """Reproduce FUN_140bac110(province): dense_index = remap[pid], remap = *(gamestate+0x340),
    gamestate = *(module+0x33048C0). Returns raw pid if no remap is active (matches the engine)."""
    try:
        gs = p.u64(p.module_base + GAMESTATE_RVA)
    except OSError:
        return None
    if not gs:
        return pid
    try:
        remap = p.u64(gs + REMAP_OFF)
        if not remap or not (HEAP_LO <= remap < HEAP_HI):
            return pid
        return p.u32(remap + pid * 4)
    except OSError:
        return None


def read_supply_inputs(p=None, css=None) -> Dict[str, object]:
    """LIVE persistent solver INPUTS (read-only). Returns:
       {"efficiency": {pid: float}, "draw": {pid: float}, "meta": {...}}
    where efficiency = per-province supply fulfilment [0..1+], CSupplySystem+0x10 [raw pid] (i32 in
    an i64 CPdxArray, stride 8 - verified live), and draw = per-province supply draw/demand,
    CSupplySystem+0x138 [raw pid]. Both ARE stored and validate sensible (e.g. 3324=0.95, 3544=1.0).
    The province -> dense-index remap (FUN_140bac110) is NOT bulk-returned here (it is cheap to
    compute on demand) - use province_dense_index(p, pid). meta carries the css instance + counts.
    Returns {} on a build mismatch / no live game. Only nonzero entries are returned (sparse).

    `css` (optional): the already-located CSupplySystem instance address (the pin's cached root).
    When given, the per-walk _find_csupplysystem heap scan is SKIPPED. None self-locates."""
    own = False
    if p is None:
        p = _open_live()       # _open_live() build-gates internally
        own = True
        if p is None:
            return {}
    else:
        # fail-closed even when a caller supplies a handle: a public API must never read a
        # non-matching build (offsets are build-specific).
        try:
            if p.fingerprint().get("sha256") != EXPECTED_SHA256:
                return {}
        except Exception:  # noqa: BLE001
            return {}
    try:
        if css is None:
            css = _find_csupplysystem(p)
        if not css:
            return {}
        out: Dict[str, object] = {"meta": {"csupplysystem": "0x%X" % css}}
        for name, off in (("efficiency", CSS_PROV_EFF), ("draw", CSS_PROV_DRAW)):
            try:
                base = p.u64(css + off)
                cnt = p.u32(css + off + 8)
            except OSError:
                continue
            if not base or not (HEAP_LO <= base < HEAP_HI) or not cnt:
                continue
            # CPdxArray<i64>: fixed-point value in the LOW i32 at stride 8 (verified live: pid*8
            # gives sensible fulfilment 0..1; pid*4 does not). Bound by the ACTUAL bytes read so
            # a short try_read can't raise struct.error - fail closed to the entries we have.
            want = min(cnt, MAX_PID + 1) * 8
            blob = p.try_read(base, want)
            d: Dict[int, float] = {}
            if blob:
                usable = len(blob) // 8
                for pid in range(1, min(min(cnt, MAX_PID + 1), usable)):
                    raw = struct.unpack_from("<i", blob, pid * 8)[0]
                    if raw != 0:
                        d[pid] = raw / FP
            out[name] = d
            out["meta"][name + "_count"] = len(d)
        # dense index map (only meaningful where present; cheap to compute on demand instead,
        # so we expose the helper rather than a full 13k map here).
        out["meta"]["dense_index_via"] = "province_dense_index(p, pid)"
        out["meta"]["build"] = EXPECTED_SHA256
        return out
    finally:
        if own:
            p.__exit__()


# ---------------------------------------------------------------------------
# T0 seam consumed by models/supply_number.py.
# ---------------------------------------------------------------------------
def province_supply_exact(output_dir: str) -> Dict[int, float]:  # noqa: ARG001
    """Return {province_id: exact displayed supply remaining} for the T0 tier.

    HONESTY CONTRACT: returns ONLY values proven exact AND looked up by province id. The displayed
    per-province "supply available" is NOT persisted as any province-keyed scalar on build
    16022851 (decompile + 4-anchor live test with the verified dense remap; see module docstring).
    It is a transient routing-solver output (freed per-node cap array + render mesh), reproducible
    only by replaying the routing. We DO NOT fabricate -> return {} so models/supply_number.py
    degrades to its division-proxy (T1) and structural (T2) tiers, never to an invented number.
    The proven live read-only signals are read_supply_inputs() (efficiency/draw) and the division
    proxy (logistics_intel.supply_live.enumerate_supply)."""
    return {}


# Either-shape accessor for the model.
class _Solver:
    province_supply = staticmethod(province_supply_exact)
    supply_path_delivered = staticmethod(supply_path_delivered)
    country_alloc_cap = staticmethod(country_alloc_cap)


SOLVER = _Solver()


def main():
    p = _open_live()
    if p is None:
        print("supply_solver_live: hoi4.exe not running or build mismatch (gated to %s)" % EXPECTED_SHA256[:12])
        return
    try:
        print("province_supply_exact(): %d entries (honest {} - displayed value is transient)"
              % len(province_supply_exact("")))
        for pid in (3324, 3544, 6421, 9244):
            di = province_dense_index(p, pid)
            print("  dense_index(%d) = %s" % (pid, di))
        inp = read_supply_inputs(p)
        meta = inp.get("meta", {})
        print("read_supply_inputs(): efficiency=%s draw=%s entries  css=%s"
              % (meta.get("efficiency_count"), meta.get("draw_count"), meta.get("csupplysystem")))
        eff = inp.get("efficiency", {})
        for pid in (3324, 3544, 6421, 9244):
            print("  prov %-5d efficiency=%s draw=%s"
                  % (pid, eff.get(pid), inp.get("draw", {}).get(pid)))
    finally:
        p.__exit__()


if __name__ == "__main__":
    main()
