"""_perprov_probe.py - dissect SWeatherPerProvince (13414 live instances,
vtable RVA 0x2950A50). This is the per-province realized weather cache that
lockstep determinism requires. Find: province_id field, phenomenon enum,
intensity float, temperature.

Anchor provinces (province -> region -> precip, temp):
  7206 -> 292  clear 16C
  3946 ->  20  clear  8C   (state 31; NOTE region 31 != province region)
  6945 -> 129  clear 11C
  7250 ->  31  RAIN  26C   <- the rain province
  10167-> 160  clear 24C

Plan:
  1. Enumerate SWeatherPerProvince instances.
  2. Find province_id field: offset whose values cover the anchor province ids
     and span a province-id-like range (1..~14000).
  3. Build province_id -> obj. Confirm the 5 anchor provinces present.
  4. Phenomenon sweep: province 7250 (rain) differs from 4 clear provinces.
  5. Temperature sweep: 5 anchors ~ 16/8/11/26/24 C.
READ-ONLY.
"""
import json
import os
import sys
import struct
import time

import numpy as np

_HERE = os.path.dirname(os.path.abspath(__file__))
_MR = os.path.dirname(_HERE)
for _p in (_MR, _HERE):
    if _p not in sys.path:
        sys.path.insert(0, _p)
import winmem as wm
import fastscan as fs

PERPROV_RVA = 0x2950A50
BODY = 0x80

# province -> (region, temp_C, is_rain)
ANCHOR_PROV = {7206: (292, 16.0, False), 3946: (20, 8.0, False),
               6945: (129, 11.0, False), 7250: (31, 26.0, True),
               10167: (160, 24.0, False)}
CLEAR_PROV = [7206, 3946, 6945, 10167]
RAIN_PROV = 7250
ALL_PROV = list(ANCHOR_PROV.keys())


def find_id_offset(bodies, targets, lo=1, hi=20000):
    """Find offset/width where the values cover the target ids and lie in [lo,hi]."""
    targets = set(targets)
    cands = []
    for width, dt in ((2, "<u2"), (4, "<u4")):
        maxoff = min(len(b) for _, b in bodies) - width
        for off in range(0, maxoff + 1):
            vals = set()
            inrange = 0
            for _, b in bodies:
                v = int(np.frombuffer(b[off:off + width], dt)[0])
                vals.add(v)
                if lo <= v <= hi:
                    inrange += 1
            covered = len(targets & vals)
            # require covering all anchor provinces + a large distinct in-range set
            if covered == len(targets):
                cands.append((off, width, covered, inrange, len(vals)))
    cands.sort(key=lambda c: (-c[2], -c[3]))
    return cands


if __name__ == "__main__":
    p = wm.open_process("hoi4.exe")
    mb = p.module_base
    pid = p.pid
    p.close()
    arena = fs.discover_arena(pid)
    spec = fs.make_qword_eq_matcher(mb + PERPROV_RVA)
    t0 = time.time()
    hits = fs.parallel_scan(pid, arena[0], arena[1], workers=6, **spec)
    print("SWeatherPerProvince vtable 0x%X: %d instances in %.1fs"
          % (mb + PERPROV_RVA, len(hits), time.time() - t0))

    p = wm.Process(pid, "hoi4.exe")
    try:
        bodies = []
        for h in hits:
            b = p.try_read(h, BODY)
            if len(b) >= 0x20:
                bodies.append((h, b))
        print("read %d bodies" % len(bodies))

        idcands = find_id_offset(bodies, ALL_PROV)
        print("\n=== province-id field candidates (cover all 5 anchors) ===")
        for (off, width, cov, inr, dist) in idcands[:10]:
            print("  +0x%03X w=%d covered=%d inrange=%d distinct=%d" % (off, width, cov, inr, dist))
        if not idcands:
            print("NO province-id field covering all 5 anchors. Dump first body:")
            print(bodies[0][1].hex())
            # Try covering 4 of 5
            for width, dt in ((2,"<u2"),(4,"<u4")):
                maxoff = min(len(b) for _,b in bodies)-width
                best=None
                for off in range(maxoff+1):
                    vals=set(int(np.frombuffer(b[off:off+width],dt)[0]) for _,b in bodies)
                    cov=len(set(ALL_PROV)&vals)
                    if best is None or cov>best[2]:
                        best=(off,width,cov)
                print("  best %d-cover at +0x%X w=%d"%(best[2],best[0],width))
            raise SystemExit(0)

        pid_off, pid_w = idcands[0][0], idcands[0][1]
        dt = "<u2" if pid_w == 2 else "<u4"
        prov2obj = {}
        for (h, b) in bodies:
            pv = int(np.frombuffer(b[pid_off:pid_off + pid_w], dt)[0])
            if pv not in prov2obj:
                prov2obj[pv] = (h, b)
        present = [pp for pp in ALL_PROV if pp in prov2obj]
        print("\nprovince-id field at +0x%X w=%d. anchors present: %s of 5 -> %s"
              % (pid_off, pid_w, len(present), present))
        if len(present) < 5:
            raise SystemExit("not all anchor provinces present")

        # phenomenon sweep
        print("\n=== PHENOMENON sweep (4 clear provinces equal, rain 7250 differs) ===")
        bclear = [prov2obj[pp][1] for pp in CLEAR_PROV]
        brain = prov2obj[RAIN_PROV][1]
        minlen = min(len(b) for b in bclear + [brain])
        ph = []
        for width, wdt in ((1, "<u1"), (2, "<u2"), (4, "<u4")):
            for off in range(0, minlen - width + 1):
                cv = [int(np.frombuffer(b[off:off + width], wdt)[0]) for b in bclear]
                rv = int(np.frombuffer(brain[off:off + width], wdt)[0])
                if len(set(cv)) == 1 and rv != cv[0]:
                    pl = (cv[0] == 0 and 1 <= rv <= 7)
                    ph.append((off, width, cv[0], rv, pl))
        ph.sort(key=lambda x: (not x[4], x[0]))
        for (off, w, c, r, pl) in ph[:40]:
            print("  +0x%03X w=%d clear=%d rain=%d%s" % (off, w, c, r, "  <-- plausible enum" if pl else ""))
        if not ph:
            print("  (none)")

        # temperature sweep
        print("\n=== TEMPERATURE sweep (5 anchors ~ 16/8/11/26/24 C) ===")
        expect = {pp: ANCHOR_PROV[pp][1] for pp in ALL_PROV}
        encs = [("f32_celsius", "<f4", 4, lambda a: a),
                ("f32_kelvin", "<f4", 4, lambda a: a - 273.15),
                ("i32_x100", "<i4", 4, lambda a: a / 100.0),
                ("i32_x1000", "<i4", 4, lambda a: a / 1000.0),
                ("i16_x100", "<i2", 2, lambda a: a / 100.0),
                ("i16_x10", "<i2", 2, lambda a: a / 10.0)]
        tm = []
        bb = {pp: prov2obj[pp][1] for pp in ALL_PROV}
        for (en, edt, eb, conv) in encs:
            for off in range(0, BODY - eb + 1):
                ok = True; reads = {}; mxe = 0.0
                for pp in ALL_PROV:
                    b = bb[pp]
                    if off + eb > len(b): ok = False; break
                    v = float(conv(np.frombuffer(b[off:off + eb], edt).astype(np.float64))[0])
                    if not np.isfinite(v): ok = False; break
                    reads[pp] = round(v, 2)
                    e = abs(v - expect[pp])
                    if e > 2.0: ok = False; break
                    mxe = max(mxe, e)
                if ok:
                    tm.append((off, en, reads, round(mxe, 2)))
        tm.sort(key=lambda m: m[3])
        for (off, en, reads, mxe) in tm[:20]:
            print("  +0x%03X enc=%-12s maxerr=%.2f reads=%s" % (off, en, mxe, reads))
        if not tm:
            print("  (none within +-2C)")

        # hexdumps
        print("\n=== HEXDUMP province 7250 (RAIN 26C) @ 0x%X ===" % prov2obj[7250][0])
        print(prov2obj[7250][1].hex())
        print("=== HEXDUMP province 3946 (clear 8C) @ 0x%X ===" % prov2obj[3946][0])
        print(prov2obj[3946][1].hex())
        print("=== HEXDUMP province 7206 (clear 16C) @ 0x%X ===" % prov2obj[7206][0])
        print(prov2obj[7206][1].hex())

        json.dump({
            "perprov_vtable": "0x%X" % (mb + PERPROV_RVA),
            "perprov_vtable_rva": "0x%X" % PERPROV_RVA,
            "n_instances": len(hits),
            "province_id_off": "0x%X" % pid_off, "province_id_width": pid_w,
            "anchors_present": present,
            "anchor_objs": {str(pp): "0x%X" % prov2obj[pp][0] for pp in present},
            "phenomenon_candidates": [{"off": "0x%X" % x[0], "width": x[1],
                                       "clear": x[2], "rain": x[3], "plausible": x[4]} for x in ph[:40]],
            "temp_candidates": [{"off": "0x%X" % m[0], "enc": m[1], "reads": m[2], "maxerr": m[3]} for m in tm[:20]],
        }, open(os.path.join(_HERE, "_perprov_probe.json"), "w"), indent=2)
        print("\nsaved _perprov_probe.json")
    finally:
        p.close()
