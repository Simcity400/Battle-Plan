"""_phenom_enum.py - find the per-region PHENOMENON TYPE enum (not just active/
inactive) by reading the full SWeatherPerRegion body for ALL active regions and
looking for a small-range field (0..7) that varies among active regions in a way
consistent with the climatology phenomenon-possibility (rain vs snow vs storm).

Also pins the inline-array stride (sizeof SWeatherPerRegion) by reading the
manager +0x040 array element by element until region ids stop matching.

Approach for the enum:
  For every byte/u16/u32 offset in the body, collect the value across all active
  regions. A phenomenon enum candidate is a field that:
    - is 0 for ALL inactive regions (clear), and
    - is in a small set {1..7} for active regions.
  Report the candidate offsets and the value distribution.
READ-ONLY.
"""
import json
import os
import sys
import struct
from collections import Counter

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
HEAP_LO, HEAP_HI = 0x10000000000, 0x7FF000000000
BODY = 0xA0


def isheap(v):
    return HEAP_LO <= v < HEAP_HI


if __name__ == "__main__":
    p = wm.open_process("hoi4.exe")
    mb = p.module_base; pid = p.pid
    p.close()
    arena = fs.discover_arena(pid)
    m = fs.parallel_scan(pid, arena[0], arena[1], workers=6, **fs.make_qword_eq_matcher(mb + WMGR_RVA))[0]

    p = wm.Process(pid, "hoi4.exe")
    try:
        # ---- pin inline-array stride ----
        b40 = p.u64(m + 0x40)
        # element 0 vtable should be perregion vtable; find stride by scanning for
        # next perregion vtable occurrence
        blk = p.try_read(b40, 0x400)
        perreg_vt = mb + PERREGION_RVA
        occ = [i for i in range(8, len(blk)-8) if struct.unpack("<Q", blk[i:i+8])[0] == perreg_vt]
        stride = occ[0] if occ else None
        print("inline-array stride (sizeof SWeatherPerRegion) = %s" % stride)
        # verify: read region ids at element starts
        if stride:
            ids = [p.u32(b40 + k*stride + 8) for k in range(10)]
            print("first 10 element region_ids:", ids)

        # ---- read all 305 region bodies via the inline array ----
        rows = {}
        n = 305
        for k in range(n):
            base = b40 + k*stride
            body = p.try_read(base, BODY)
            if len(body) < BODY: continue
            rid = struct.unpack("<I", body[8:12])[0]
            v58 = struct.unpack("<Q", body[0x58:0x60])[0]
            v70 = struct.unpack("<Q", body[0x70:0x78])[0]
            active = isheap(v58) or isheap(v70)
            rows[rid] = {"body": body, "active": active, "base": base}
        active_ids = sorted(r for r in rows if rows[r]["active"] and 1<=r<=304)
        inactive_ids = sorted(r for r in rows if not rows[r]["active"] and 1<=r<=304)
        print("active=%d inactive=%d" % (len(active_ids), len(inactive_ids)))

        # ---- enum-field search: 0 for all inactive, small {1..15} for active ----
        print("\n=== phenomenon-enum field candidates ===")
        cands = []
        for width, dt in ((1,"<B"),(2,"<H"),(4,"<I")):
            for off in range(0, BODY - width + 1):
                inact_vals = set(struct.unpack(dt, rows[r]["body"][off:off+width])[0] for r in inactive_ids)
                act_vals = [struct.unpack(dt, rows[r]["body"][off:off+width])[0] for r in active_ids]
                # candidate: inactive all 0, active in a small enum range
                if inact_vals == {0}:
                    avset = set(act_vals)
                    if avset and max(avset) <= 15 and 0 not in avset:
                        cands.append((off, width, sorted(avset), Counter(act_vals)))
        cands.sort(key=lambda c: (len(c[2]), c[0]))
        for (off, width, vals, cnt) in cands[:30]:
            print("  +0x%03X w=%d active_values=%s dist=%s" % (off, width, vals, dict(cnt.most_common())))
        if not cands:
            print("  (no field that is 0-for-inactive AND small-enum-for-active)")
            # relax: just 0 for inactive, anything for active, but bounded distinct
            print("\n  relaxed (0 for inactive, <=8 distinct active values):")
            for width, dt in ((1,"<B"),(2,"<H"),(4,"<I")):
                for off in range(0, BODY-width+1):
                    inact = set(struct.unpack(dt, rows[r]["body"][off:off+width])[0] for r in inactive_ids)
                    act = [struct.unpack(dt, rows[r]["body"][off:off+width])[0] for r in active_ids]
                    if inact=={0} and 1<=len(set(act))<=8 and 0 not in set(act):
                        print("    +0x%03X w=%d active=%s" % (off, width, dict(Counter(act).most_common())))

        # report region 31 vs a few other active regions full body
        print("\n=== sample active region bodies ===")
        for r in [31] + [x for x in active_ids if x != 31][:4]:
            print("  region %3d: %s" % (r, rows[r]["body"][:0x40].hex()))
    finally:
        p.close()
