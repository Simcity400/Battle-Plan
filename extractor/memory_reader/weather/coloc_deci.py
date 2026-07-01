"""coloc_deci.py - co-location pivoting on i16 DECI-CELSIUS (and centi-C i32),
order-invariant. Covers the encodings the Celsius-float scans + the 'too-wide-band'
positional skips could miss.

Deci-Celsius raw values for the anchors: r20=80, r31=260, r129=110, r160=240,
r292=160. Pivot on region 31's deci-value 260 (i16) -- rarer than the tiny id 31.
For each i16==260 site, look +-WIN bytes for embedded region id 31 (i16/i32), then
verify the other 4 anchors as {id, deci-temp} records at a constant stride. Also a
centi-Celsius i32 variant (2600/800/1100/2400/1600). READ-ONLY.
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
import winmem as _wm
import fastscan as fs

# (id -> raw temp value) for two fixed-point scalings
TOLI = 1                      # +-1 raw unit (covers rounding)
ANCH_DECI = {20: 80, 31: 260, 129: 110, 160: 240, 292: 160}     # i16 x10
ANCH_CENTI = {20: 800, 31: 2600, 129: 1100, 160: 2400, 292: 1600}  # i32 x100
PIVOT_RID = 31
WIN = 160
STRIDES = (8, 12, 16, 20, 24, 28, 32, 40, 48, 64, 96, 128)
NSLOTS = 360


def _m_i_eq(base, blob, width, target):
    dt = {2: "<i2", 4: "<i4"}[width]
    phase = (-base) % width
    m = (len(blob) - phase) // width
    if m <= 0:
        return []
    arr = np.frombuffer(blob, dt, count=m, offset=phase)
    idx = np.nonzero(arr == target)[0]
    if idx.size == 0:
        return []
    return (base + phase + idx.astype(np.int64) * width).tolist()


def _read_i(p, addr, width):
    dt = {2: "<i2", 4: "<i4"}[width]
    raw = p.try_read(addr, width)
    if len(raw) != width:
        return None
    return int(np.frombuffer(raw, dt)[0])


def classify(p, temp_addr, anch, temp_width):
    """temp_addr = pivot region-31 temp element (i16 260 or i32 2600). Find embedded
    region id 31 within +-WIN (i16/i32), verify other anchors at constant stride."""
    out = []
    win = p.try_read(temp_addr - WIN, 2 * WIN + 4)
    if len(win) < 2 * WIN + 4:
        return out
    cands = []   # (id_width, id_rel)
    for idw, dt in ((4, "<i4"), (2, "<i2")):
        m = len(win) // idw
        arr = np.frombuffer(win[:m * idw], dt)
        for pp in np.nonzero(arr == PIVOT_RID)[0].tolist():
            rel = pp * idw - WIN
            if -WIN <= rel <= WIN and rel != 0:
                cands.append((idw, rel))
    if not cands:
        return out
    for (idw, id_rel) in cands:
        for stride in STRIDES:
            resolved = {31: 0}
            ok = True
            for rid in (160, 20, 129, 292):
                found = None
                for k in range(-NSLOTS, NSLOTS + 1):
                    if k == 0:
                        continue
                    ta = temp_addr + k * stride
                    if _read_i(p, ta + id_rel, idw) != rid:
                        continue
                    tv = _read_i(p, ta, temp_width)
                    if tv is not None and abs(tv - anch[rid]) <= TOLI:
                        found = k
                        break
                if found is None:
                    ok = False
                    break
                resolved[rid] = found
            if ok:
                out.append({"id_width": idw, "id_rel": id_rel, "stride": stride,
                            "temp_width": temp_width,
                            "scaling": "x10" if temp_width == 2 else "x100",
                            "pivot_temp_addr": "0x%X" % temp_addr,
                            "records": resolved})
    return out


def run(pid, arena, regions, workers=6):
    confirmed = []
    p = _wm.Process(pid, "hoi4.exe")
    try:
        for (label, twidth, anch, pivval) in (
                ("i16_deci(260)", 2, ANCH_DECI, 260),
                ("i32_centi(2600)", 4, ANCH_CENTI, 2600)):
            t0 = time.time()
            hits = fs.parallel_scan(pid, arena[0], arena[1],
                                    matcher=("memory_reader.weather.coloc_deci", "_m_i_eq"),
                                    matcher_args=(twidth, pivval), item_size=twidth,
                                    workers=workers, regions=regions)
            dt = time.time() - t0
            print("  STAGE-A %-16s -> %d sites (%.1fs)" % (label, len(hits), dt), flush=True)
            nb = 0
            for hv in hits:
                recs = classify(p, hv, anch, twidth)
                if recs:
                    confirmed.extend(recs)
                    for r in recs:
                        print("    CONFIRM %s idw=%d id_rel=%d stride=%d @ %s"
                              % (label, r["id_width"], r["id_rel"], r["stride"],
                                 r["pivot_temp_addr"]), flush=True)
                nb += 1
                if nb % 50000 == 0:
                    print("    ...%d/%d %s, %d confirmed" % (nb, len(hits), label, len(confirmed)), flush=True)
            print("    DONE %s: %d sites -> %d confirmed" % (label, nb, len(confirmed)), flush=True)
    finally:
        p.close()
    return confirmed


def main():
    import argparse
    ap = argparse.ArgumentParser()
    ap.add_argument("--workers", type=int, default=6)
    args = ap.parse_args()
    pid = _wm.find_pid("hoi4.exe")
    if pid is None:
        raise SystemExit("hoi4.exe not running")
    arena = fs.discover_arena(pid)
    print("arena 0x%X..0x%X (%.2f GB)" % (arena[0], arena[1], (arena[1]-arena[0])/2**30), flush=True)
    h = fs._k32.OpenProcess(fs._PROCESS_VM_READ | fs._PROCESS_QUERY_INFORMATION, False, pid)
    try:
        regions = fs.enumerate_regions(h, arena[0], arena[1])
    finally:
        fs._k32.CloseHandle(h)
    print("regions %d" % len(regions), flush=True)
    conf = run(pid, arena, regions, workers=args.workers)
    print("TOTAL CONFIRMED: %d" % len(conf), flush=True)
    json.dump(conf[:200], open(os.path.join(_HERE, "_coloc_deci_confirmed.json"), "w"), indent=1)
    print("wrote _coloc_deci_confirmed.json", flush=True)


if __name__ == "__main__":
    main()
