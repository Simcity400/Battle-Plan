"""_validate_phenom.py - validate the per-region phenomenon enum at +0x078.

Read +0x078 (u32 phenomenon) and +0x05D/+0x075/+0x028 for ALL 305 inline region
elements. Tabulate the phenomenon distribution; identify the 5 regions with
phenomenon==2 and cross-check their climatology (cold/snow-capable) vs the
phenomenon==1 regions (warmer/rain). Confirms region 31==1(rain) and clears==0.

Also re-reads twice to confirm paused-stability.
READ-ONLY.
"""
import json
import os
import sys
import struct
import time
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
STRIDE = 352
PHENOM_OFF = 0x78
INTEN1_OFF = 0x5D   # u8 ==2 for active
INTEN2_OFF = 0x75   # u8 ==2 for active
ACC_OFF = 0x28
HEAP_LO, HEAP_HI = 0x10000000000, 0x7FF000000000


def isheap(v):
    return HEAP_LO <= v < HEAP_HI


if __name__ == "__main__":
    p = wm.open_process("hoi4.exe")
    mb = p.module_base; pid = p.pid
    p.close()
    arena = fs.discover_arena(pid)
    m = fs.parallel_scan(pid, arena[0], arena[1], workers=6, **fs.make_qword_eq_matcher(mb + WMGR_RVA))[0]

    # climatology
    climo = {}
    try:
        cj = json.load(open(os.path.join(_HERE, "region_weather_mod.json")))
        climo = {int(k): v for k, v in cj.items()}
    except Exception as e:
        print("no climatology:", e)

    p = wm.Process(pid, "hoi4.exe")
    try:
        b40 = p.u64(m + 0x40)
        rows = {}
        for k in range(305):
            base = b40 + k * STRIDE
            body = p.try_read(base, 0xA0)
            if len(body) < 0xA0:
                continue
            rid = struct.unpack("<I", body[8:12])[0]
            ph = struct.unpack("<I", body[PHENOM_OFF:PHENOM_OFF+4])[0]
            i1 = body[INTEN1_OFF]
            i2 = body[INTEN2_OFF]
            acc = struct.unpack("<I", body[ACC_OFF:ACC_OFF+4])[0]
            v58 = struct.unpack("<Q", body[0x58:0x60])[0]
            rows[rid] = {"phenom": ph, "i1": i1, "i2": i2, "acc": acc,
                         "active": isheap(v58)}

        dist = Counter(rows[r]["phenom"] for r in rows if 1 <= r <= 304)
        print("=== phenomenon (+0x78 u32) distribution over regions 1..304 ===")
        for v, c in sorted(dist.items()):
            print("  phenom=%d : %d regions" % (v, c))

        print("\n=== anchor check ===")
        for rid, lab in ((31,"RAIN"),(20,"clear"),(129,"clear"),(160,"clear"),(292,"clear")):
            r = rows.get(rid)
            if r:
                print("  region %3d phenom=%d (expect %s)  i1=%d i2=%d acc=%d active=%s"
                      % (rid, r["phenom"], lab, r["i1"], r["i2"], r["acc"], r["active"]))

        # phenom==2 regions: are they cold (snow-capable)?
        ph2 = sorted(r for r in rows if rows[r]["phenom"] == 2 and 1<=r<=304)
        ph1 = sorted(r for r in rows if rows[r]["phenom"] == 1 and 1<=r<=304)
        print("\n=== regions with phenom==2 (n=%d): %s ===" % (len(ph2), ph2))
        if climo:
            def midtemp(rid):
                info = climo.get(rid)
                if not info or not info.get("periods"): return None
                per = info["periods"]
                pp = per[4] if len(per)>4 else per[-1]  # May
                if pp.get("tmin") is None: return None
                return (pp["tmin"]+pp["tmax"])/2.0
            t2 = [midtemp(r) for r in ph2 if midtemp(r) is not None]
            t1 = [midtemp(r) for r in ph1 if midtemp(r) is not None]
            if t2: print("  phenom==2 May mid-temp: min=%.1f max=%.1f mean=%.1f" % (min(t2),max(t2),sum(t2)/len(t2)))
            if t1: print("  phenom==1 May mid-temp: min=%.1f max=%.1f mean=%.1f" % (min(t1),max(t1),sum(t1)/len(t1)))
            print("  (hypothesis: phenom 1=rain warm, 2=snow/cold -> phenom2 mean << phenom1 mean)")

        # stability re-read
        time.sleep(2.0)
        ph31_a = struct.unpack("<I", p.try_read(b40 + 31*STRIDE, 0xA0)[PHENOM_OFF:PHENOM_OFF+4])[0]
        print("\nstability: region 31 phenom re-read = %d (was %d)" % (ph31_a, rows[31]["phenom"]))

        json.dump({
            "phenom_offset": "0x78", "phenom_width": 4,
            "distribution": {str(k): v for k, v in dist.items()},
            "anchor_phenom": {str(r): rows[r]["phenom"] for r in (31,20,129,160,292) if r in rows},
            "phenom2_regions": ph2,
            "stride": STRIDE,
        }, open(os.path.join(_HERE, "_validate_phenom.json"), "w"), indent=2)
        print("\nsaved _validate_phenom.json")
    finally:
        p.close()
