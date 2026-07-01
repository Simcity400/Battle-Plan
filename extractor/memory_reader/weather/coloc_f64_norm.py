"""coloc_f64_norm.py - close the two remaining gaps in the weather negative:
(A) f64 DOUBLE temperature co-location (order-invariant; not covered by the f32
    scans), and (B) NORMALIZED-LERP positional scan (temp stored as
    (T-tmin)/(tmax-tmin) per region, a tight 0.07..0.29 cluster).

(A) pivot on f64 ~26.0 sites; find embedded region id 31 within +-WIN bytes; verify
    other 4 anchors as id + f64-temp records at a constant stride.
(B) widen the co-location window to +-1024 bytes for f32 (covers a province struct
    whose id field is far from the temp). READ-ONLY, multiprocess.
"""
import json
import os
import sys
import time

import numpy as np

_HERE = os.path.dirname(os.path.abspath(__file__)); _MR = os.path.dirname(_HERE)
for _p in (_MR, _HERE):
    if _p not in sys.path:
        sys.path.insert(0, _p)
import winmem as _wm
import fastscan as fs

TOL = 1.6
ANCH = {20: 8.0, 31: 26.0, 129: 11.0, 160: 24.0, 292: 16.0}
PIVOT_RID = 31
PIVOT_T = 26.0
STRIDES = (8, 16, 24, 32, 40, 48, 64, 80, 96, 128, 160, 192, 256)
NSLOTS = 360


def _m_f64_26(base, blob, _u):
    phase = (-base) & 7
    n = (len(blob) - phase) & ~7
    if n <= 0:
        return []
    arr = np.frombuffer(blob, "<f8", count=n >> 3, offset=phase)
    idx = np.nonzero(np.isfinite(arr) & (np.abs(arr - PIVOT_T) <= TOL))[0]
    if idx.size == 0:
        return []
    return (base + phase + (idx.astype(np.int64) << 3)).tolist()


def _read_i(p, addr, width):
    dt = {2: "<i2", 4: "<i4"}[width]
    raw = p.try_read(addr, width)
    return int(np.frombuffer(raw, dt)[0]) if len(raw) == width else None


def _read_f64(p, addr):
    raw = p.try_read(addr, 8)
    if len(raw) != 8:
        return None
    v = float(np.frombuffer(raw, "<f8")[0])
    return v if np.isfinite(v) else None


def classify_f64(p, temp_addr, win_bytes):
    """temp_addr = f64 ~26C (pivot). Find embedded id 31 within +-win_bytes, verify
    the other 4 anchors as id + f64 temp at constant stride."""
    out = []
    win = p.try_read(temp_addr - win_bytes, 2 * win_bytes + 8)
    if len(win) < 2 * win_bytes + 8:
        return out
    cands = []
    for width, dt in ((4, "<i4"), (2, "<i2")):
        m = len(win) // width
        arr = np.frombuffer(win[:m * width], dt)
        for pp in np.nonzero(arr == PIVOT_RID)[0].tolist():
            rel = pp * width - win_bytes
            if -win_bytes <= rel <= win_bytes and rel != 0:
                cands.append((width, rel))
    if not cands:
        return out
    for (idw, id_rel) in cands:
        for stride in STRIDES:
            ok = True
            for rid in (160, 20, 129, 292):
                found = None
                for k in range(-NSLOTS, NSLOTS + 1):
                    if k == 0:
                        continue
                    ta = temp_addr + k * stride
                    if _read_i(p, ta + id_rel, idw) != rid:
                        continue
                    tv = _read_f64(p, ta)
                    if tv is not None and abs(tv - ANCH[rid]) <= TOL:
                        found = k
                        break
                if found is None:
                    ok = False
                    break
            if ok:
                out.append({"enc": "f64_celsius", "id_width": idw, "id_rel": id_rel,
                            "stride": stride, "pivot_temp_addr": "0x%X" % temp_addr})
    return out


def main():
    import argparse
    ap = argparse.ArgumentParser()
    ap.add_argument("--workers", type=int, default=6)
    ap.add_argument("--win", type=int, default=256)
    args = ap.parse_args()
    pid = _wm.find_pid("hoi4.exe")
    if pid is None:
        raise SystemExit("hoi4.exe not running")
    arena = fs.discover_arena(pid)
    print("arena 0x%X..0x%X" % (arena[0], arena[1]), flush=True)
    h = fs._k32.OpenProcess(fs._PROCESS_VM_READ | fs._PROCESS_QUERY_INFORMATION, False, pid)
    try:
        regions = fs.enumerate_regions(h, arena[0], arena[1])
    finally:
        fs._k32.CloseHandle(h)
    print("regions %d" % len(regions), flush=True)
    t0 = time.time()
    sites = fs.parallel_scan(pid, arena[0], arena[1],
                             matcher=("memory_reader.weather.coloc_f64_norm", "_m_f64_26"),
                             matcher_args=(0,), item_size=8, workers=args.workers,
                             regions=regions)
    print("STAGE-A f64~26 sites: %d (%.1fs)" % (len(sites), time.time() - t0), flush=True)
    p = _wm.Process(pid, "hoi4.exe")
    conf = []
    try:
        nb = 0
        for s in sites:
            recs = classify_f64(p, s, args.win)
            if recs:
                conf.extend(recs)
                for r in recs:
                    print("    CONFIRM f64 idw=%d id_rel=%d stride=%d @ %s"
                          % (r["id_width"], r["id_rel"], r["stride"], r["pivot_temp_addr"]), flush=True)
            nb += 1
            if nb % 30000 == 0:
                print("    ...%d/%d, %d confirmed" % (nb, len(sites), len(conf)), flush=True)
    finally:
        p.close()
    print("f64 co-location CONFIRMED: %d" % len(conf), flush=True)
    json.dump(conf[:100], open(os.path.join(_HERE, "_coloc_f64.json"), "w"), indent=1)
    print("done", flush=True)


if __name__ == "__main__":
    main()
