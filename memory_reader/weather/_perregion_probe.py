"""_perregion_probe.py - dissect SWeatherPerRegion objects to find the phenomenon
enum, intensity, and temperature fields, validated by the rain@31 vs clear anchors.

SWeatherPerRegion vtable RVA = 0x2950AA0 (305 live instances).

Plan:
  1. Re-enumerate SWeatherPerRegion instances (vtable scan).
  2. Find the region-id field: try every u16/u32 offset in the body; the field
     whose 305 values form a permutation/cover of 1..304 (or 0..304) is region_id.
  3. Build region_id -> object map. Confirm all 5 anchors present.
  4. PHENOMENON sweep: for every u8/u16/u32 offset, read across the 5 anchor objects;
     report offsets where the 4 clear (20/129/160/292) share a value and region 31
     differs (the rain discriminator).
  5. TEMPERATURE sweep: f32/fixed-point offset where 5 anchors ~= 8/26/11/24/16 C.
  6. Dump a full hexdump of region-31 and region-20 objects side by side.
READ-ONLY.
"""
import json
import os
import sys
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

PERREGION_RVA = 0x2950AA0
BODY = 0x100  # SWeatherPerRegion is small; scan generously

ANCHORS = [(20, 8.0, False), (31, 26.0, True), (129, 11.0, False),
           (160, 24.0, False), (292, 16.0, False)]
CLEAR_RIDS = [20, 129, 160, 292]
RAIN_RID = 31
ALL_RIDS = [20, 31, 129, 160, 292]


def find_region_id_offset(bodies_by_index):
    """bodies_by_index: list of (obj_addr, body_bytes). Find the offset where the
    set of values across all objects best covers 1..304. Returns list of
    (off, width, score, ids_sample)."""
    n = len(bodies_by_index)
    cands = []
    for width, dt in ((2, "<u2"), (4, "<u4")):
        maxoff = min(len(b) for _, b in bodies_by_index) - width
        for off in range(0, maxoff + 1):
            vals = []
            ok = True
            for _, b in bodies_by_index:
                v = int(np.frombuffer(b[off:off + width], dt)[0])
                vals.append(v)
            inr = [v for v in vals if 1 <= v <= 304]
            distinct_inr = len(set(inr))
            # ideal: 304 or 305 distinct in 1..304, covering the full range
            score = distinct_inr
            if distinct_inr >= 250:  # strong region-id candidate
                cands.append((off, width, distinct_inr, len(set(vals)), min(vals), max(vals)))
    cands.sort(key=lambda c: -c[2])
    return cands


if __name__ == "__main__":
    p = wm.open_process("hoi4.exe")
    if p is None:
        raise SystemExit("hoi4.exe not running")
    mb = p.module_base
    pid = p.pid
    p.close()
    arena = fs.discover_arena(pid)
    vt_abs = mb + PERREGION_RVA
    spec = fs.make_qword_eq_matcher(vt_abs)
    t0 = time.time()
    hits = fs.parallel_scan(pid, arena[0], arena[1], workers=6, **spec)
    print("SWeatherPerRegion vtable 0x%X: %d instances in %.1fs" % (vt_abs, len(hits), time.time() - t0))

    p = wm.Process(pid, "hoi4.exe")
    try:
        bodies = []
        for h in hits:
            b = p.try_read(h, BODY)
            if len(b) >= 0x40:
                bodies.append((h, b))
        print("read %d object bodies" % len(bodies))

        # 1. find region-id field
        idcands = find_region_id_offset(bodies)
        print("\n=== region-id field candidates (>=250 distinct in 1..304) ===")
        for (off, width, distinct, total_distinct, mn, mx) in idcands[:10]:
            print("  +0x%03X width=%d distinct_in_range=%d total_distinct=%d min=%d max=%d"
                  % (off, width, distinct, total_distinct, mn, mx))

        if not idcands:
            print("NO region-id field found in body; dumping first object hex for manual look")
            print(bodies[0][1].hex())
            raise SystemExit(0)

        rid_off, rid_w = idcands[0][0], idcands[0][1]
        dt = "<u2" if rid_w == 2 else "<u4"
        id2obj = {}
        for (h, b) in bodies:
            rid = int(np.frombuffer(b[rid_off:rid_off + rid_w], dt)[0])
            if rid not in id2obj:
                id2obj[rid] = (h, b)
        present = [r for r in ALL_RIDS if r in id2obj]
        print("\nregion-id field at +0x%X (width %d). anchors present: %s of 5 -> %s"
              % (rid_off, rid_w, len(present), present))

        if len(present) < 5:
            print("not all anchors present; cannot validate cleanly. ids covered:",
                  sorted([r for r in id2obj if 1 <= r <= 304])[:20], "...")
            raise SystemExit(0)

        # 2. PHENOMENON discriminator sweep
        print("\n=== PHENOMENON sweep (clear 20/129/160/292 equal, rain 31 differs) ===")
        bclear = [id2obj[r][1] for r in CLEAR_RIDS]
        brain = id2obj[RAIN_RID][1]
        minlen = min(len(b) for b in bclear + [brain])
        precip_hits = []
        for width, wdt in ((1, "<u1"), (2, "<u2"), (4, "<u4")):
            for off in range(0, minlen - width + 1):
                cvals = [int(np.frombuffer(b[off:off + width], wdt)[0]) for b in bclear]
                rv = int(np.frombuffer(brain[off:off + width], wdt)[0])
                if len(set(cvals)) == 1 and rv != cvals[0]:
                    cv = cvals[0]
                    plausible = (cv == 0 and 1 <= rv <= 7) or (cv in (0, 1) and rv in range(0, 8))
                    precip_hits.append((off, width, cv, rv, plausible))
        precip_hits.sort(key=lambda x: (not x[4], x[0]))
        for (off, width, cv, rv, pl) in precip_hits[:40]:
            tag = " <-- plausible enum" if pl else ""
            print("  +0x%03X w=%d clear=%d rain=%d%s" % (off, width, cv, rv, tag))
        if not precip_hits:
            print("  (none)")

        # 3. TEMPERATURE sweep
        print("\n=== TEMPERATURE sweep (5 anchors ~ 8/26/11/24/16 C) ===")
        expect = {r: t for (r, t, _) in ANCHORS}
        encs = [("f32_celsius", "<f4", 4, lambda a: a),
                ("f32_kelvin", "<f4", 4, lambda a: a - 273.15),
                ("i32_x100", "<i4", 4, lambda a: a / 100.0),
                ("i32_x1000", "<i4", 4, lambda a: a / 1000.0),
                ("i16_x100", "<i2", 2, lambda a: a / 100.0),
                ("i16_x10", "<i2", 2, lambda a: a / 10.0)]
        tmatches = []
        bb = {r: id2obj[r][1] for r in ALL_RIDS}
        for (en, edt, eb, conv) in encs:
            for off in range(0, BODY - eb + 1):
                ok = True
                reads = {}
                maxe = 0.0
                for r in ALL_RIDS:
                    b = bb[r]
                    if off + eb > len(b):
                        ok = False; break
                    v = float(conv(np.frombuffer(b[off:off + eb], edt).astype(np.float64))[0])
                    if not np.isfinite(v):
                        ok = False; break
                    reads[r] = round(v, 2)
                    e = abs(v - expect[r])
                    if e > 2.0:
                        ok = False; break
                    maxe = max(maxe, e)
                if ok:
                    tmatches.append((off, en, reads, round(maxe, 2)))
        tmatches.sort(key=lambda m: m[3])
        for (off, en, reads, maxe) in tmatches[:20]:
            print("  +0x%03X enc=%-12s maxerr=%.2f reads=%s" % (off, en, maxe, reads))
        if not tmatches:
            print("  (none within +-2C)")

        # 4. hexdump region 31 (rain) vs region 20 (clear)
        print("\n=== HEXDUMP region 31 (rain) obj 0x%X ===" % id2obj[31][0])
        print(id2obj[31][1].hex())
        print("=== HEXDUMP region 20 (clear) obj 0x%X ===" % id2obj[20][0])
        print(id2obj[20][1].hex())
        print("=== HEXDUMP region 292 (clear,16C) obj 0x%X ===" % id2obj[292][0])
        print(id2obj[292][1].hex())

        json.dump({
            "perregion_vtable": "0x%X" % vt_abs,
            "perregion_vtable_rva": "0x%X" % PERREGION_RVA,
            "n_instances": len(hits),
            "region_id_off": "0x%X" % rid_off, "region_id_width": rid_w,
            "anchors_present": present,
            "anchor_objs": {str(r): "0x%X" % id2obj[r][0] for r in present},
            "precip_candidates": [{"off": "0x%X" % h[0], "width": h[1],
                                   "clear": h[2], "rain": h[3], "plausible": h[4]}
                                  for h in precip_hits[:40]],
            "temp_candidates": [{"off": "0x%X" % m[0], "enc": m[1], "reads": m[2],
                                 "maxerr": m[3]} for m in tmatches[:20]],
        }, open(os.path.join(_HERE, "_perregion_probe.json"), "w"), indent=2)
        print("\nsaved _perregion_probe.json")
    finally:
        p.close()
