"""_mgr_body.py - full dump of the CWeatherManager singleton body, enumerating
every std::vector triple and pointer, to find a compact per-region phenomenon
array (the cache combat queries) distinct from the 305 SWeatherPerRegion objects.
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

WMGR_RVA = 0x2950AF0
PERREGION_RVA = 0x2950AA0
PERPROV_RVA = 0x2950A50
HEAP_LO, HEAP_HI = 0x10000000000, 0x7FF000000000


def isheap(v):
    return HEAP_LO <= v < HEAP_HI


if __name__ == "__main__":
    p = wm.open_process("hoi4.exe")
    mb = p.module_base; msize = p.module_size; pid = p.pid
    p.close()
    arena = fs.discover_arena(pid)
    mh = fs.parallel_scan(pid, arena[0], arena[1], workers=6, **fs.make_qword_eq_matcher(mb + WMGR_RVA))
    perreg_vt = mb + PERREGION_RVA
    perprov_vt = mb + PERPROV_RVA
    print("CWeatherManager instances: %d" % len(mh))
    m = mh[0]
    print("manager @ 0x%X" % m)

    p = wm.Process(pid, "hoi4.exe")
    try:
        body = p.try_read(m, 0x600)
        print("\n=== qword dump (0..0x600), flagging vectors & typed pointers ===")
        qn = len(body) // 8
        qs = struct.unpack("<%dQ" % qn, body[:qn*8])
        for i in range(qn):
            off = i * 8
            q = qs[i]
            tag = ""
            if isheap(q):
                # vector triple?
                if i + 2 < qn:
                    e, c = qs[i+1], qs[i+2]
                    if isheap(e) and isheap(c) and q <= e <= c and (e-q) <= (1<<31):
                        tag = "VEC begin=0x%X end=0x%X cap=0x%X span=%d" % (q, e, c, e-q)
                        # what does begin point to? sample vtable
                        vt = p.try_read(q, 8)
                        if len(vt) == 8:
                            tv = struct.unpack("<Q", vt)[0]
                            if tv == perreg_vt: tag += " [->SWeatherPerRegion]"
                            elif tv == perprov_vt: tag += " [->SWeatherPerProvince]"
                            elif mb <= tv < mb+msize: tag += " [elem0 vt MOD+0x%X]" % (tv-mb)
                if not tag:
                    tag = "-> HEAP"
            elif mb <= q < mb + msize:
                tag = "-> MOD+0x%X" % (q - mb)
            if tag:
                print("  +0x%03X  0x%016X  %s" % (off, q, tag))

        # The +0x388 big vector: figure out stride that yields 304/305/13414
        print("\n=== +0x388 big vector stride analysis ===")
        b, e, c = struct.unpack("<QQQ", body[0x388:0x3A0])
        span = e - b
        print("  span=%d bytes" % span)
        for n in (304, 305, 13413, 13414, 13415):
            if span % n == 0:
                print("    /%d -> stride %d" % (n, span // n))
            else:
                print("    /%d -> %.3f (non-integer)" % (n, span / n))
        for st in (4, 8, 12, 16, 24, 28, 30):
            if span % st == 0:
                print("    stride %d -> %d elements" % (st, span // st))
        # sample first elements of +0x388
        sample = p.try_read(b, 0x80)
        print("  first 0x80 bytes: %s" % sample.hex())
        print("  as u32:", np.frombuffer(sample, "<u4").tolist())
    finally:
        p.close()
