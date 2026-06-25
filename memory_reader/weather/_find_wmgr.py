"""_find_wmgr.py - locate the live CWeatherManager instance(s) and probe their
arrays for the per-region / per-province weather-state caches.

From RTTI:
  CWeatherManager       vtable RVA 0x2950AF0
  SWeatherPerRegion     vtable RVA 0x2950AA0   (struct; may be a Reader glue, not a true obj vtable)
  SWeatherPerProvince   vtable RVA 0x2950A50
  CWeatherTerrainModifier vtable RVA 0x2950A00

Plan:
  1. Scan arena for aligned qword == (module_base + 0x2950AF0) -> CWeatherManager instances.
  2. For each manager, dump body [0..0x400] and look for std::vector triples
     (begin,end,cap pointers into heap) — these are the per-region / per-province
     state arrays. Print element counts (end-begin)/stride for plausible strides.
  3. Also scan arena directly for the SWeatherPerRegion/SWeatherPerProvince vtable
     to see if those structs are individually allocated (vtable-bearing) objects.
READ-ONLY.
"""
import os
import sys
import time

import numpy as np

_HERE = os.path.dirname(os.path.abspath(__file__))
_MR = os.path.dirname(_HERE)
for _p in (_MR, _HERE):
    if _p not in sys.path:
        sys.path.insert(0, _p)
import winmem as wm
import fastscan as fs

WMGR_RVA = 0x2950AF0
PERREGION_RVA = 0x2950AA0
PERPROV_RVA = 0x2950A50
TERRAINMOD_RVA = 0x2950A00


def scan_vtable(pid, arena, vt_abs, label, workers=6):
    spec = fs.make_qword_eq_matcher(vt_abs)
    t0 = time.time()
    hits = fs.parallel_scan(pid, arena[0], arena[1], workers=workers, **spec)
    print("[%s] vtable 0x%X: %d hits in %.1fs" % (label, vt_abs, len(hits), time.time() - t0))
    return hits


def looks_heap(v):
    return 0x10000000000 <= v < 0x7FF000000000


def dump_vectors(p, base, span=0x400):
    """Scan an object body for (begin,end,cap) std::vector triples pointing into
    heap, with begin<=end<=cap. Returns list of (off, begin, end, cap, nbytes)."""
    raw = p.try_read(base, span)
    if len(raw) < 24:
        return []
    out = []
    qn = len(raw) // 8
    qs = np.frombuffer(raw[:qn * 8], dtype="<u8")
    for i in range(qn - 2):
        b, e, c = int(qs[i]), int(qs[i + 1]), int(qs[i + 2])
        if looks_heap(b) and looks_heap(e) and looks_heap(c):
            if b <= e <= c and (e - b) <= (1 << 30) and (c - b) <= (1 << 30):
                out.append((i * 8, b, e, c, e - b))
    return out


if __name__ == "__main__":
    p = wm.open_process("hoi4.exe")
    if p is None:
        raise SystemExit("hoi4.exe not running")
    mb = p.module_base
    pid = p.pid
    p.close()
    arena = fs.discover_arena(pid)
    print("module_base 0x%X arena 0x%X..0x%X" % (mb, arena[0], arena[1]))

    wmgr_hits = scan_vtable(pid, arena, mb + WMGR_RVA, "CWeatherManager")
    perregion_hits = scan_vtable(pid, arena, mb + PERREGION_RVA, "SWeatherPerRegion")
    perprov_hits = scan_vtable(pid, arena, mb + PERPROV_RVA, "SWeatherPerProvince")
    terrain_hits = scan_vtable(pid, arena, mb + TERRAINMOD_RVA, "CWeatherTerrainModifier")

    p = wm.Process(pid, "hoi4.exe")
    try:
        for h in wmgr_hits[:8]:
            print("\n=== CWeatherManager @ 0x%X ===" % h)
            vecs = dump_vectors(p, h, 0x600)
            for (off, b, e, c, nb) in vecs:
                print("  +0x%03X vector begin=0x%X end=0x%X cap=0x%X  bytes=%d" % (off, b, e, c, nb))
                for st in (4, 8, 16, 24, 32, 40, 48, 64):
                    if nb % st == 0 and 0 < nb // st <= 100000:
                        n = nb // st
                        if 200 <= n <= 6000:  # plausible region(304)/province(~13000) counts
                            print("        stride %3d -> %d elements  <-- plausible" % (st, n))
                        elif n in (304, 305, 303):
                            print("        stride %3d -> %d elements  <-- REGION COUNT" % (st, n))
    finally:
        p.close()

    import json
    json.dump({
        "module_base": "0x%X" % mb,
        "wmgr_hits": ["0x%X" % h for h in wmgr_hits],
        "perregion_hits": ["0x%X" % h for h in perregion_hits[:50]],
        "perprov_hits": ["0x%X" % h for h in perprov_hits[:50]],
        "terrain_hits": ["0x%X" % h for h in terrain_hits[:50]],
        "n_perregion": len(perregion_hits),
        "n_perprov": len(perprov_hits),
        "n_terrain": len(terrain_hits),
    }, open(os.path.join(_HERE, "_wmgr_instances.json"), "w"), indent=2)
    print("\nsaved _wmgr_instances.json")
