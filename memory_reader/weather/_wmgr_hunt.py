"""_wmgr_hunt.py - STRUCTURE-based hunt for the game weather-STATE (phenomenon)
cache, NOT a value scan. Re-targets from temperature (proven unpinnable) to the
gameplay weather state that lockstep determinism requires in memory.

Stages (all READ-ONLY, arena discovered LIVE):
  S0  enumerate the 304 CStrategicRegion objects (proven anchor) -> id2obj.
  S1  RTTI sweep: find every vtable in .rdata whose RTTI TypeDescriptor name
      contains Weather/Climate/Precip; then count live instances of each.
  S2  PER-REGION PHENOMENON: for each candidate weather-state object class found
      in S1, enumerate instances, find a region-id field, and look for a
      phenomenon enum where region 31 (rain) differs from 20/129/160/292 (clear).
  S2b If no separate class: walk the CStrategicRegion object's POINTER fields
      (deref each 8-aligned ptr in the body) to a sibling weather-state object
      and look for the rain-vs-clear discriminator there (1-2 hops).
  Reports offsets, the rain-vs-clear reads, and RTTI names.
"""
import json
import os
import sys
import time
from collections import Counter, defaultdict

import numpy as np

_HERE = os.path.dirname(os.path.abspath(__file__))
_MR = os.path.dirname(_HERE)
for _p in (_MR, _HERE):
    if _p not in sys.path:
        sys.path.insert(0, _p)
import winmem as wm
import fastscan as fs

EXPECTED_SHA = "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120"
REGION_VTABLE_OFF = 0x29468B8
REGION_ID_OFF = 0x58

# anchor (region_id, temp_C, is_rain)
ANCHORS = [(20, 8.0, False), (31, 26.0, True), (129, 11.0, False),
           (160, 24.0, False), (292, 16.0, False)]
CLEAR_RIDS = [20, 129, 160, 292]
RAIN_RID = 31
ALL_ANCHOR_RIDS = [20, 31, 129, 160, 292]


def attach():
    pid = wm.find_pid("hoi4.exe")
    if pid is None:
        raise SystemExit("hoi4.exe not running")
    p = wm.Process(pid, "hoi4.exe")
    mb = p.module_base
    msize = p.module_size
    fp = p.fingerprint().get("sha256")
    p.close()
    if fp != EXPECTED_SHA:
        print("WARN build sha mismatch: %s" % fp)
    arena = fs.discover_arena(pid)
    print("pid %d module_base 0x%X size 0x%X arena 0x%X..0x%X (%.2f GB)"
          % (pid, mb, msize, arena[0], arena[1], (arena[1] - arena[0]) / 2**30))
    return pid, mb, msize, arena


def enum_region_objs(pid, mb, arena, workers=6):
    vt_abs = mb + REGION_VTABLE_OFF
    spec = fs.make_qword_eq_matcher(vt_abs)
    t0 = time.time()
    hits = fs.parallel_scan(pid, arena[0], arena[1], workers=workers, **spec)
    print("[obj] region vtable 0x%X: %d hits in %.1fs" % (vt_abs, len(hits), time.time() - t0))
    p = wm.Process(pid, "hoi4.exe")
    id2obj = {}
    try:
        for h in hits:
            raw = p.try_read(h + REGION_ID_OFF, 4)
            if len(raw) == 4:
                rid = int(np.frombuffer(raw, "<u4")[0])
                if rid not in id2obj:
                    id2obj[rid] = h
    finally:
        p.close()
    inr = [r for r in id2obj if 1 <= r <= 304]
    print("[obj] %d distinct ids, %d in 1..304" % (len(id2obj), len(inr)))
    return id2obj, hits


if __name__ == "__main__":
    pid, mb, msize, arena = attach()
    id2obj, hits = enum_region_objs(pid, mb, arena)
    present = [rid for rid in ALL_ANCHOR_RIDS if rid in id2obj]
    print("[obj] anchors present: %s of 5 -> %s" % (len(present), present))
    json.dump({str(k): "0x%X" % v for k, v in id2obj.items()},
              open(os.path.join(_HERE, "_id2obj.json"), "w"))
    print("saved _id2obj.json (%d entries)" % len(id2obj))
