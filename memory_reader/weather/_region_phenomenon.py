"""_region_phenomenon.py - read the active-weather indicator for ALL 304
SWeatherPerRegion objects and map which regions currently have active weather.

The discriminator (from structural decode):
  region has active weather  <=>  +0x58 vector begin pointer != 0 (heap)
We read, per region:
  - region_id (+0x08)
  - +0x28 accumulator (u32)
  - +0x58 begin ptr (qword)   -> active? (nonzero heap)
  - +0x58 size  (+0x60 u32)   -> count of local weather positions
  - +0x70 begin ptr (qword)   -> effect entities active?
Then summarize: how many regions active, list them, confirm 31 active and the
4 clear anchors inactive. Cross-check vs climatology (rain-possible this month).
READ-ONLY.
"""
import json
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
HEAP_LO, HEAP_HI = 0x10000000000, 0x7FF000000000


def isheap(v):
    return HEAP_LO <= v < HEAP_HI


if __name__ == "__main__":
    p = wm.open_process("hoi4.exe")
    mb = p.module_base; pid = p.pid
    p.close()
    arena = fs.discover_arena(pid)
    spec = fs.make_qword_eq_matcher(mb + PERREGION_RVA)
    hits = fs.parallel_scan(pid, arena[0], arena[1], workers=6, **spec)
    print("SWeatherPerRegion instances: %d" % len(hits))

    p = wm.Process(pid, "hoi4.exe")
    rows = {}
    try:
        for h in hits:
            body = p.try_read(h, 0x80)
            if len(body) < 0x78:
                continue
            rid = struct.unpack("<I", body[8:12])[0]
            acc = struct.unpack("<I", body[0x28:0x2C])[0]
            v58 = struct.unpack("<Q", body[0x58:0x60])[0]
            v58_sz = struct.unpack("<I", body[0x60:0x64])[0]
            v70 = struct.unpack("<Q", body[0x70:0x78])[0]
            active = isheap(v58) or isheap(v70)
            rows[rid] = {"obj": h, "acc": acc, "v58": v58, "v58_sz": v58_sz,
                         "v70": v70, "active": active}
    finally:
        p.close()

    active_ids = sorted(r for r in rows if rows[r]["active"] and 1 <= r <= 304)
    print("\nACTIVE-weather regions: %d of %d" % (len(active_ids), len(rows)))
    print("active region ids:", active_ids)

    print("\n=== anchor check ===")
    for rid, exp in ((31, True), (20, False), (129, False), (160, False), (292, False)):
        if rid in rows:
            r = rows[rid]
            ok = "OK" if r["active"] == exp else "*** MISMATCH ***"
            print("  region %3d active=%-5s (expect %s) %s  acc=%d v58_sz=%d"
                  % (rid, r["active"], exp, ok, r["acc"], r["v58_sz"]))

    # climatology cross-check: are active regions rain-possible this month?
    try:
        climo = json.load(open(os.path.join(_HERE, "region_weather_mod.json")))
        # month index 4 (May) per anchors_live note (1946.5)
        rainposs = []
        for rid in active_ids:
            info = climo.get(str(rid))
            if info and info.get("periods"):
                per = info["periods"]
                pp = per[4] if len(per) > 4 else per[-1]
                rainposs.append((rid, pp))
        print("\n%d active regions have climatology data (sample of phenomenon prob)" % len(rainposs))
    except Exception as e:
        print("climatology cross-check skipped:", e)

    json.dump({
        "n_instances": len(hits),
        "n_active": len(active_ids),
        "active_region_ids": active_ids,
        "anchor_check": {str(rid): rows[rid]["active"] for rid in (31,20,129,160,292) if rid in rows},
    }, open(os.path.join(_HERE, "_region_phenomenon.json"), "w"), indent=2)
    print("\nsaved _region_phenomenon.json")
