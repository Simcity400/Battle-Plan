"""_active_weather.py - decode the ACTIVE-WEATHER records hung off the per-region
and per-province weather objects, to find the phenomenon enum + intensity.

Per-region (SWeatherPerRegion) for rain region 31 had two populated std::vectors:
  +0x58 : vector of {u32 province_id, u32 0, i32 negA, i32 negB}  (stride 16)
  +0x70 : vector of {u64 ptr, u32 586hi?, ...}                    (stride 16)
Per-province (SWeatherPerProvince) for active provinces (7250 rain, 10167) had:
  +0x58 : vector (begin ptr, count) populated
  +0x70 : vector (begin ptr, count) populated

This script reads each vector's [begin,end), computes element count + stride, and
dumps records so we can spot the phenomenon enum / intensity float.
READ-ONLY.
"""
import os
import sys
import struct

import numpy as np

_HERE = os.path.dirname(os.path.abspath(__file__))
_MR = os.path.dirname(_HERE)
for _p in (_MR, _HERE):
    if _p not in sys.path:
        sys.path.insert(0, _p)
import winmem as wm
import fastscan as fs

PERREGION_RVA = 0x2950AA0
PERPROV_RVA = 0x2950A50
HEAP_LO, HEAP_HI = 0x10000000000, 0x7FF000000000


def isheap(v):
    return HEAP_LO <= v < HEAP_HI


def read_vector(p, obj_base, vec_off):
    """A PdxVector/std::vector at obj+vec_off = {begin@+0, end@+8, cap@+16}.
    Return (begin, end, cap, nbytes)."""
    hdr = p.try_read(obj_base + vec_off, 24)
    if len(hdr) < 24:
        return None
    b, e, c = struct.unpack("<QQQ", hdr)
    if not (isheap(b) and isheap(e) and b <= e):
        return None
    return (b, e, c, e - b)


def dump_records(p, begin, nbytes, stride, label):
    print("    %s: begin=0x%X bytes=%d stride=%d -> %d records" %
          (label, begin, nbytes, stride, nbytes // stride if stride else 0))
    n = min(nbytes // stride, 12) if stride else 0
    data = p.try_read(begin, min(nbytes, stride * 12))
    for i in range(n):
        rec = data[i * stride:(i + 1) * stride]
        us = np.frombuffer(rec[:stride - (stride % 4)], "<u4").tolist() if stride >= 4 else []
        fs_ = [round(float(x), 4) for x in np.frombuffer(rec[:stride - (stride % 4)], "<f4").tolist()]
        print("      [%2d] u32=%s f32=%s" % (i, us, fs_))


if __name__ == "__main__":
    p = wm.open_process("hoi4.exe")
    mb = p.module_base; pid = p.pid
    p.close()
    arena = fs.discover_arena(pid)

    # region 31 object
    spec = fs.make_qword_eq_matcher(mb + PERREGION_RVA)
    rhits = fs.parallel_scan(pid, arena[0], arena[1], workers=6, **spec)
    spec2 = fs.make_qword_eq_matcher(mb + PERPROV_RVA)
    phits = fs.parallel_scan(pid, arena[0], arena[1], workers=6, **spec2)

    p = wm.Process(pid, "hoi4.exe")
    try:
        # find region 31, 160 objects
        reg = {}
        for h in rhits:
            rid = p.u32(h + 8)
            if rid in (31, 160, 20):
                reg[rid] = h
        prov = {}
        for h in phits:
            pv = p.u16(h + 8)
            if pv in (7250, 10167, 3946):
                prov[pv] = h

        print("=== SWeatherPerRegion active-weather vectors ===")
        for rid in (31, 160, 20):
            if rid not in reg:
                continue
            base = reg[rid]
            print("\n region %d @ 0x%X" % (rid, base))
            for voff in (0x10, 0x58, 0x70):
                v = read_vector(p, base, voff)
                if v and v[3] > 0:
                    b, e, c, nb = v
                    # infer stride: try 16, 8, 4, 24, 32
                    for st in (16, 8, 24, 32, 4, 12, 20):
                        if nb % st == 0 and 1 <= nb // st <= 100000:
                            dump_records(p, b, nb, st, "+0x%X" % voff)
                            break
                    else:
                        print("    +0x%X: %d bytes (no clean stride)" % (voff, nb))
                else:
                    print("    +0x%X: empty" % voff)

        print("\n=== SWeatherPerProvince active-weather vectors ===")
        for pv in (7250, 10167, 3946):
            if pv not in prov:
                continue
            base = prov[pv]
            print("\n province %d @ 0x%X" % (pv, base))
            for voff in (0x10, 0x58, 0x70):
                v = read_vector(p, base, voff)
                if v and v[3] > 0:
                    b, e, c, nb = v
                    for st in (16, 8, 24, 32, 4, 12, 20):
                        if nb % st == 0 and 1 <= nb // st <= 100000:
                            dump_records(p, b, nb, st, "+0x%X" % voff)
                            break
                    else:
                        print("    +0x%X: %d bytes (no clean stride)" % (voff, nb))
                else:
                    print("    +0x%X: empty" % voff)
    finally:
        p.close()
