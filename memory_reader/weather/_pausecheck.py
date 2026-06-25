"""_pausecheck.py - is the game paused? Re-read region 31 SWeatherPerRegion body
twice ~3s apart and compare. Also re-locate region 31 fresh (objects may move?).
Confirms whether the +0x58/+0x70 populated state is stable or transient, and
whether region 31 still differs from clear regions.
READ-ONLY.
"""
import hashlib
import os
import sys
import time
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


def snap(p, base, n=0xA0):
    return p.try_read(base, n)


if __name__ == "__main__":
    p = wm.open_process("hoi4.exe")
    mb = p.module_base; pid = p.pid
    p.close()
    arena = fs.discover_arena(pid)
    spec = fs.make_qword_eq_matcher(mb + PERREGION_RVA)
    hits = fs.parallel_scan(pid, arena[0], arena[1], workers=6, **spec)
    p = wm.Process(pid, "hoi4.exe")
    try:
        id2 = {}
        for h in hits:
            rid = p.u32(h + 8)
            id2[rid] = h
        for rid in (31, 20, 160, 292, 129):
            if rid not in id2:
                print("region %d not found" % rid)
                continue
            base = id2[rid]
            s1 = snap(p, base)
            time.sleep(2.5)
            s2 = snap(p, base)
            same = s1 == s2
            # show the +0x58/+0x70 region
            v58_1 = struct.unpack("<Q", s1[0x58:0x60])[0]
            v58_2 = struct.unpack("<Q", s2[0x58:0x60])[0]
            v70_1 = struct.unpack("<Q", s1[0x70:0x78])[0]
            v70_2 = struct.unpack("<Q", s2[0x70:0x78])[0]
            print("region %3d @ 0x%X  stable=%s  +0x58: 0x%X->0x%X  +0x70: 0x%X->0x%X"
                  % (rid, base, same, v58_1, v58_2, v70_1, v70_2))
            if not same:
                # which offsets changed
                diffs = [i for i in range(min(len(s1), len(s2))) if s1[i] != s2[i]]
                print("    changed byte offsets: %s" % ["+0x%X" % d for d in diffs[:40]])
    finally:
        p.close()
