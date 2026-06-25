"""_final_validate.py - adversarial validation of the phenomenon pin at +0x78.

Checks:
  (1) phenom>=1  <=>  active-vector(+0x58 or +0x70) populated  (should be perfect).
  (2) phenom==0  <=>  all the active sub-vectors empty.
  (3) The 5 anchors: 31->1, others->0.  Province-level RAIN anchor 7250 lies in
      region 31 (phenom 1).
  (4) NEGATIVE CONTROL: read the SAME u32 at +0x78 from a WRONG object class
      (SWeatherPerProvince inline? no -- read +0x78 of 200 random CStrategicRegion
      geometry objects, which are a DIFFERENT class) -> should NOT yield a clean
      0/1/2 enum (proves the field is specific to the weather struct, not generic).
  (5) Stride sanity: element k has region_id k (0-indexed array == region id).
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
PERPROV_RVA = 0x2950A50
GEOM_REGION_VT = 0x29468B8   # the OLD geometry CStrategicRegion vtable
STRIDE = 352
PHENOM_OFF = 0x78
HEAP_LO, HEAP_HI = 0x10000000000, 0x7FF000000000


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
        b40 = p.u64(m + 0x40)
        # (5) stride/index sanity + read all
        rows = {}
        idx_ok = 0
        for k in range(305):
            body = p.try_read(b40 + k*STRIDE, 0xA0)
            if len(body) < 0xA0: continue
            rid = struct.unpack("<I", body[8:12])[0]
            if rid == k: idx_ok += 1
            ph = struct.unpack("<I", body[PHENOM_OFF:PHENOM_OFF+4])[0]
            v58 = struct.unpack("<Q", body[0x58:0x60])[0]
            v70 = struct.unpack("<Q", body[0x70:0x78])[0]
            rows[rid] = {"phenom": ph, "active": isheap(v58) or isheap(v70)}
        print("(5) index==region_id for %d/305 elements (0-indexed array)" % idx_ok)

        # (1)(2) phenom>=1 <=> active
        agree = sum(1 for r in rows if (rows[r]["phenom"]>=1) == rows[r]["active"])
        disagree = [r for r in rows if (rows[r]["phenom"]>=1) != rows[r]["active"]]
        print("(1)(2) phenom>=1 <=> active: %d/%d agree; disagreements: %s"
              % (agree, len(rows), disagree[:20]))

        # (3) anchors
        print("(3) anchors: 31=%d 20=%d 129=%d 160=%d 292=%d"
              % tuple(rows[r]["phenom"] for r in (31,20,129,160,292)))

        # (4) NEGATIVE CONTROL: +0x78 of geometry CStrategicRegion objects
        geom_vt = mb + GEOM_REGION_VT
        ghits = fs.parallel_scan(pid, arena[0], arena[1], workers=6, **fs.make_qword_eq_matcher(geom_vt))
        gph = []
        for h in ghits[:304]:
            raw = p.try_read(h + PHENOM_OFF, 4)
            if len(raw) == 4:
                gph.append(struct.unpack("<I", raw)[0])
        gc = Counter(gph)
        small = sum(v for k,v in gc.items() if k <= 2)
        print("(4) NEG CONTROL geometry-region +0x78 distribution (top): %s"
              % dict(gc.most_common(8)))
        print("    -> %d/%d geometry objects have +0x78 in {0,1,2} (if ~all, field is generic; if few, weather-specific)"
              % (small, len(gph)))

        # province RAIN anchor lies in region 31
        phits = fs.parallel_scan(pid, arena[0], arena[1], workers=6, **fs.make_qword_eq_matcher(mb + PERPROV_RVA))
        # province 7250 -> which region? read its body; we know from anchors region 31.
        print("(prov) province 7250 (RAIN) is in region 31 -> region phenom=%d" % rows[31]["phenom"])

        json.dump({
            "phenom_offset": "0x78", "phenom_width": 4,
            "index_is_region_id": idx_ok,
            "phenom_active_agreement": "%d/%d" % (agree, len(rows)),
            "disagreements": disagree,
            "neg_control_geom_dist": {str(k): v for k, v in gc.most_common(8)},
        }, open(os.path.join(_HERE, "_final_validate.json"), "w"), indent=2)
        print("\nsaved _final_validate.json")
    finally:
        p.close()
