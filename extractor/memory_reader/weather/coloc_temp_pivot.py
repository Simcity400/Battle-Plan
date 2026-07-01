"""coloc_temp_pivot.py - ORDER-INVARIANT co-location, pivoting on the RARE TEMP.

Pivoting on id==31 is bad: 31 is a tiny int that appears ~800k times. Instead pivot
on region 31's TEMPERATURE 26C (f32: ~113k sites, far more structured), then look
for the embedded region id 31 nearby. A real weather record has {..., id=31, ...,
temp=26C, ...} so id and temp co-locate within one record (<= ~128 bytes).

STAGE A (cheap, multiprocess): all f32 ~26C positions (also f32-Kelvin pass).
STAGE B (parent, per 26C site): search +-WIN bytes for int32/int16 == 31. Each
(id_width, id_off relative to temp) is a candidate record layout. Then VERIFY: the
record array has a stride S; for each other anchor we must find, within +-NSLOTS
slots of the pivot record, a slot whose embedded id == anchor_id AND whose temp (at
the same temp-relative offset) == anchor_temp. A layout that resolves all 5 anchors
= self-describing weather record array (no order assumption). READ-ONLY.
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

_K = 273.15
TOL = 1.6
ANCH = {20: 8.0, 31: 26.0, 129: 11.0, 160: 24.0, 292: 16.0}
PIVOT_RID = 31
PIVOT_T = 26.0
WIN = 160               # +- bytes around the temp to find the id
STRIDES = (8, 12, 16, 20, 24, 28, 32, 40, 44, 48, 56, 64, 80, 96, 128)
NSLOTS = 360


def _m_temp26_f32(base, blob, mode):
    target = PIVOT_T if mode == "c" else (PIVOT_T + _K)
    phase = (-base) & 3
    n = (len(blob) - phase) & ~3
    if n <= 0:
        return []
    arr = np.frombuffer(blob, "<f4", count=n >> 2, offset=phase).astype(np.float64)
    idx = np.nonzero(np.isfinite(arr) & (np.abs(arr - target) <= TOL))[0]
    if idx.size == 0:
        return []
    return (base + phase + (idx.astype(np.int64) << 2)).tolist()


def _read_i(p, addr, width):
    dt = {2: "<i2", 4: "<i4"}[width]
    raw = p.try_read(addr, width)
    if len(raw) != width:
        return None
    return int(np.frombuffer(raw, dt)[0])


def _read_f32(p, addr, mode):
    raw = p.try_read(addr, 4)
    if len(raw) != 4:
        return None
    v = float(np.frombuffer(raw, "<f4")[0])
    if not np.isfinite(v):
        return None
    return v - _K if mode == "k" else v


def classify(p, temp_addr, mode):
    """temp_addr = a f32 ~26C (pivot region 31). Find embedded id 31 nearby, then
    verify all 5 anchors as id+temp records at a constant stride."""
    out = []
    # 1. find id field offset: read the +-WIN window ONCE, locate int==31 with numpy
    win = p.try_read(temp_addr - WIN, 2 * WIN + 4)
    if len(win) < 2 * WIN + 4:
        return out
    u8 = np.frombuffer(win, np.uint8)
    cands = []   # (id_width, id_rel)  where id_addr = temp_addr + id_rel
    for width, dt in ((4, "<i4"), (2, "<i2")):
        m = len(win) // width
        arr = np.frombuffer(win[:m * width], dt)
        # positions (byte) where value == 31; rel = byte_pos - WIN
        pos = np.nonzero(arr == PIVOT_RID)[0]
        for pp in pos.tolist():
            rel = pp * width - WIN
            if -WIN <= rel <= WIN:
                cands.append((width, rel))
    if not cands:
        return out
    # 2. for each (id_width, id_rel, stride) verify other anchors
    for (width, id_rel) in cands:
        for stride in STRIDES:
            resolved = {31: {"k": 0, "temp_addr": "0x%X" % temp_addr}}
            ok = True
            for rid in (160, 20, 129, 292):   # 24,8,11,16 cheapest-first
                found = None
                for k in range(-NSLOTS, NSLOTS + 1):
                    if k == 0:
                        continue
                    ta = temp_addr + k * stride         # candidate temp elem
                    ida = ta + id_rel                   # its id field
                    if _read_i(p, ida, width) != rid:
                        continue
                    tv = _read_f32(p, ta, mode)
                    if tv is not None and abs(tv - ANCH[rid]) <= TOL:
                        found = k
                        break
                if found is None:
                    ok = False
                    break
                resolved[rid] = {"k": found}
            if ok:
                # record array base = temp_addr of the lowest-k resolved record
                out.append({"id_width": width, "id_rel": id_rel, "stride": stride,
                            "temp_enc": ("f32_kelvin" if mode == "k" else "f32_celsius"),
                            "pivot_temp_addr": "0x%X" % temp_addr,
                            "records": resolved})
    return out


def run(pid, arena, regions, workers=6):
    confirmed = []
    p = _wm.Process(pid, "hoi4.exe")
    try:
        for mode, label in (("c", "f32c-26"), ("k", "f32k-299.15")):
            t0 = time.time()
            hits = fs.parallel_scan(pid, arena[0], arena[1],
                                    matcher=("memory_reader.weather.coloc_temp_pivot", "_m_temp26_f32"),
                                    matcher_args=(mode,), item_size=4,
                                    workers=workers, regions=regions)
            dt = time.time() - t0
            print("  STAGE-A %-12s -> %d temp sites (%.1fs)" % (label, len(hits), dt), flush=True)
            nb = 0
            for hv in hits:
                recs = classify(p, hv, mode)
                if recs:
                    confirmed.extend(recs)
                    for r in recs:
                        print("    CONFIRM enc=%s idw=%d id_rel=%d stride=%d @ temp %s"
                              % (r["temp_enc"], r["id_width"], r["id_rel"], r["stride"],
                                 r["pivot_temp_addr"]), flush=True)
                nb += 1
                if nb % 20000 == 0:
                    print("    ...classified %d/%d %s sites, %d confirmed"
                          % (nb, len(hits), label, len(confirmed)), flush=True)
            print("    DONE %s: classified %d sites -> %d confirmed"
                  % (label, nb, len(confirmed)), flush=True)
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
    by = Counter((c["id_width"], c["id_rel"], c["stride"], c["temp_enc"]) for c in conf)
    for k, c in by.most_common(30):
        print("   idw=%d id_rel=%d stride=%d enc=%s  x%d" % (k[0], k[1], k[2], k[3], c), flush=True)
    json.dump(conf[:200], open(os.path.join(_HERE, "_coloc_temp_confirmed.json"), "w"), indent=1)
    print("wrote _coloc_temp_confirmed.json", flush=True)


if __name__ == "__main__":
    main()
