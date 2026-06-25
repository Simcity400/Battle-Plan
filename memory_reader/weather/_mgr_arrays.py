"""_mgr_arrays.py - decode the CWeatherManager's per-region containers:
  +0x040 : PdxVector {begin, u32 size=305, u32 cap=305}  -- per-region
  +0x008 : big vector (element0 vtable MOD+0x26FB148)
Look for a compact per-region phenomenon/temperature array indexed 0..304 where
region 31 (rain) differs from clear regions. We use the ANCHOR region ids; since
this array is likely 0-indexed by region order, we map via the SWeatherPerRegion
order OR by matching the known per-region accumulator values from +0x28.

Strategy for +0x040 (size 305): read all 305 elements at several strides, then
for each stride find the element index whose pointer == region-31 object (if it's
a pointer array) OR whose embedded id==31.
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
HEAP_LO, HEAP_HI = 0x10000000000, 0x7FF000000000


def isheap(v):
    return HEAP_LO <= v < HEAP_HI


if __name__ == "__main__":
    p = wm.open_process("hoi4.exe")
    mb = p.module_base; pid = p.pid
    p.close()
    arena = fs.discover_arena(pid)
    m = fs.parallel_scan(pid, arena[0], arena[1], workers=6, **fs.make_qword_eq_matcher(mb + WMGR_RVA))[0]
    rhits = fs.parallel_scan(pid, arena[0], arena[1], workers=6, **fs.make_qword_eq_matcher(mb + PERREGION_RVA))

    p = wm.Process(pid, "hoi4.exe")
    try:
        # build region_id -> obj and obj -> region_id
        id2obj, obj2id = {}, {}
        for h in rhits:
            rid = p.u32(h + 8)
            id2obj.setdefault(rid, h)
            obj2id[h] = rid

        body = p.try_read(m, 0x400)

        # ---- +0x040 vector (size 305) ----
        b40 = struct.unpack("<Q", body[0x40:0x48])[0]
        sz40 = struct.unpack("<I", body[0x48:0x4C])[0]
        cap40 = struct.unpack("<I", body[0x4C:0x50])[0]
        print("+0x040 PdxVector begin=0x%X size=%d cap=%d" % (b40, sz40, cap40))
        if isheap(b40) and sz40 == 305:
            # likely a pointer array to the 305 region objects OR an SoA struct.
            raw = p.try_read(b40, 305 * 8)
            ptrs = struct.unpack("<305Q", raw)
            # how many are SWeatherPerRegion object pointers?
            matchobj = sum(1 for q in ptrs if q in obj2id)
            print("  as 8-byte ptr array: %d/305 point to SWeatherPerRegion objects" % matchobj)
            if matchobj > 250:
                # index i -> region id obj2id[ptrs[i]]; find index of region 31
                idx31 = [i for i, q in enumerate(ptrs) if obj2id.get(q) == 31]
                print("  region 31 at array index %s" % idx31)
                # so the manager keeps region objects in this order; the per-region
                # phenomenon could be a PARALLEL SoA array at another offset, indexed
                # the same way. We'll note the index order.
                order = [obj2id.get(q, -1) for q in ptrs]
                print("  first 16 region ids in array order:", order[:16])

        # ---- +0x008 big vector ----
        b08, e08, c08 = struct.unpack("<QQQ", body[0x08:0x20])
        span = e08 - b08
        print("\n+0x008 big vector begin=0x%X span=%d" % (b08, span))
        # element0 vtable was MOD+0x26FB148; treat as object array. find stride by
        # locating the 2nd object (next vtable occurrence).
        head = p.try_read(b08, 0x400)
        vt0 = struct.unpack("<Q", head[:8])[0]
        print("  elem0 vtable 0x%X (MOD+0x%X)" % (vt0, vt0 - mb))
        # find next occurrence of vt0 in head -> stride
        occ = [i for i in range(8, len(head) - 8, 8) if struct.unpack("<Q", head[i:i+8])[0] == vt0]
        print("  vtable re-occurs at byte offsets:", occ[:8])
        if occ:
            stride = occ[0]
            print("  inferred stride = %d -> %d elements" % (stride, span // stride))
        # dump first 0x60 of elem0
        print("  elem0 body: %s" % head[:0x60].hex())

        # ---- scan ALL manager heap-pointer members for a 305-sized array ----
        print("\n=== scan manager body for size==305 PdxVectors ===")
        qn = 0x400 // 8
        qs = struct.unpack("<%dQ" % qn, body[:qn*8])
        for i in range(qn - 1):
            off = i * 8
            q = qs[i]
            if isheap(q):
                nxt = qs[i+1]
                sz = nxt & 0xFFFFFFFF
                cap = nxt >> 32
                if sz in (303, 304, 305, 306) and cap >= sz:
                    print("  +0x%03X begin=0x%X size=%d cap=%d" % (off, q, sz, cap))
    finally:
        p.close()
