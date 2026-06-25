"""_decode_qword.py - read SWeatherPerRegion as 8-byte qwords and follow the
sub-vectors in the rain region (31) to find the actual per-province phenomenon
records. Also reads the manager's big +0x388 vector to understand province layout.
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
RID_OFF = 0x08
ANCHOR_RIDS = [20, 31, 129, 160, 292]
HEAP_LO, HEAP_HI = 0x10000000000, 0x7FF000000000


def isheap(v):
    return HEAP_LO <= v < HEAP_HI


def ismod(v, mb, msize):
    return mb <= v < mb + msize


if __name__ == "__main__":
    p = wm.open_process("hoi4.exe")
    mb = p.module_base
    msize = p.module_size
    pid = p.pid
    p.close()
    arena = fs.discover_arena(pid)
    spec = fs.make_qword_eq_matcher(mb + PERREGION_RVA)
    hits = fs.parallel_scan(pid, arena[0], arena[1], workers=6, **spec)
    p = wm.Process(pid, "hoi4.exe")
    id2 = {}
    try:
        for h in hits:
            b = p.try_read(h, 0x10)
            if len(b) >= 0x10:
                rid = int(np.frombuffer(b[RID_OFF:RID_OFF + 4], "<u4")[0])
                if rid in ANCHOR_RIDS and rid not in id2:
                    id2[rid] = h

        print("module_base 0x%X size 0x%X" % (mb, msize))
        for r in (31, 20):
            base = id2[r]
            print("\n========== SWeatherPerRegion region %d @ 0x%X ==========" % (r, base))
            body = p.try_read(base, 0xA0)
            for off in range(0, 0xA0, 8):
                q = struct.unpack("<Q", body[off:off + 8])[0]
                tag = ""
                if off == 0:
                    tag = "vtable"
                elif ismod(q, mb, msize):
                    tag = "-> MODULE +0x%X" % (q - mb)
                elif isheap(q):
                    tag = "-> HEAP"
                # also show as 2x u32
                lo, hi = q & 0xFFFFFFFF, q >> 32
                print("  +0x%03X  q=0x%016X  (u32: %d, %d)  %s" % (off, q, lo, hi, tag))

            # Follow the two HEAP pointers in the rain sub-record (+0x58, +0x70)
            if r == 31:
                for poff in (0x58, 0x70):
                    pv = struct.unpack("<Q", body[poff:poff + 8])[0]
                    if isheap(pv):
                        print("\n  --- follow +0x%X ptr 0x%X (vector begin?) ---" % (poff, pv))
                        # the next qword may be the end pointer
                        endv = struct.unpack("<Q", body[poff + 8:poff + 16])[0] if poff + 16 <= 0xA0 else 0
                        sub = p.try_read(pv, 0x80)
                        print("    pointee[0:0x80] hex:", sub.hex())
                        # decode as u32 array
                        arr = np.frombuffer(sub[:0x80], "<u4")
                        print("    as u32:", arr.tolist())
                        arrf = np.frombuffer(sub[:0x80], "<f4")
                        print("    as f32:", [round(float(x),4) for x in arrf.tolist()])
    finally:
        p.close()

    # Manager big vector
    print("\n========== CWeatherManager +0x388 vector ==========")
    p = wm.Process(pid, "hoi4.exe")
    try:
        spec2 = fs.make_qword_eq_matcher(mb + 0x2950AF0)
        mh = fs.parallel_scan(pid, arena[0], arena[1], workers=6, **spec2)
        if mh:
            m = mh[0]
            print("manager @ 0x%X" % m)
            hdr = p.try_read(m + 0x388, 24)
            b, e, c = struct.unpack("<QQQ", hdr)
            print("  begin=0x%X end=0x%X cap=0x%X  span=%d bytes" % (b, e, c, e - b))
            for st in (24, 28, 30, 32, 40, 48, 0x60, 0x80, 0x90, 0xA0):
                if (e - b) % st == 0:
                    print("    stride 0x%X (%d) -> %d elements" % (st, st, (e - b) // st))
            # province count ~13414; (e-b)/13414
            n = 13414
            if (e - b) % n == 0:
                print("    if 13414 provinces -> stride %d bytes" % ((e - b) // n))
            else:
                print("    (e-b)/13414 = %.2f (not integer)" % ((e - b) / n))
                print("    (e-b)/13415 = %.2f" % ((e - b) / 13415))
    finally:
        p.close()
