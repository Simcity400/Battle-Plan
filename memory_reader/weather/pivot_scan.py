"""pivot_scan.py - FAST 5-anchor weather pin via a RARE-PIVOT strategy.

The full-arena cost should be ONE cheap vectorized compare, not a combo loop per
chunk. Region 31 = 26C is the rarest anchor value. So:

  STAGE 1 (cheap, full arena, multiprocess): find every f32 in [26-TOL, 26+TOL]
          (and, for other encodings, the encoded equivalent of 26C). This is a
          single numpy band compare per chunk -- as fast as the qword scan.
  STAGE 2 (tiny, parent): each STAGE-1 hit is a CANDIDATE position for region 31's
          temperature element. For every (enc, stride, ib), region 31 sits at a
          known element offset from region-ib element 0, and the other 4 anchors at
          known offsets. Read a window around each hit and test all combos exactly.
          The 5 distinct temps at 5 fixed relative offsets -> near-zero false rate.

Because 26C f32 is rare in heap memory, STAGE 1 yields few hits and STAGE 2 is fast.
We run STAGE 1 for f32 (covers f32_celsius AND, by scanning the kelvin band, the
f32_kelvin layout) plus i16/i32 fixed-point bands. READ-ONLY.
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
# anchors: rid -> temp; region 31 is the pivot value 26C
_ANCH = [(20, 8.0), (31, 26.0), (129, 11.0), (160, 24.0), (292, 16.0)]
_RIDS = np.array([a[0] for a in _ANCH], dtype=np.int64)
_TEMPS = {a[0]: a[1] for a in _ANCH}
PIVOT_RID = 31
PIVOT_T = 26.0

# STAGE-1 band matchers: each returns positions where the decoded value ~ 26C.
# We register one matcher per (encoding family) keyed by element dtype + decode.
_ENC = {
    "f32_celsius": ("<f4", 4, lambda a: a.astype(np.float64)),
    "f32_kelvin":  ("<f4", 4, lambda a: a.astype(np.float64) - _K),
    "i32_x10":     ("<i4", 4, lambda a: a.astype(np.float64) / 10.0),
    "i32_x100":    ("<i4", 4, lambda a: a.astype(np.float64) / 100.0),
    "i32_x1000":   ("<i4", 4, lambda a: a.astype(np.float64) / 1000.0),
    "i32_x100000": ("<i4", 4, lambda a: a.astype(np.float64) / 100000.0),
    "i16_x10":     ("<i2", 2, lambda a: a.astype(np.float64) / 10.0),
    "i16_x100":    ("<i2", 2, lambda a: a.astype(np.float64) / 100.0),
    "f16_celsius": ("<f2", 2, lambda a: a.astype(np.float64)),
}
_STRIDES = (4, 8, 16, 32, 64)


# ---- STAGE 1 matcher: positions where decoded f32 (or f32-K) is ~26C ----------
def _m_pivot_f32(base, blob, mode):
    """mode 'c' -> f32 Celsius ~26; 'k' -> f32 Kelvin ~ (26+273.15)=299.15."""
    target = PIVOT_T if mode == "c" else (PIVOT_T + _K)
    phase = (-base) & 3
    n = (len(blob) - phase) & ~3
    if n <= 0:
        return []
    arr = np.frombuffer(blob, "<f4", count=n >> 2, offset=phase)
    af = arr.astype(np.float64)
    idx = np.nonzero(np.isfinite(af) & (np.abs(af - target) <= TOL))[0]
    if idx.size == 0:
        return []
    return (base + phase + (idx.astype(np.int64) << 2)).tolist()


def _m_pivot_i(base, blob, dtstr, eb, scale):
    """positions where int field / scale ~ 26C (fixed-point)."""
    phase = (-base) % eb
    m = (len(blob) - phase) // eb
    if m <= 0:
        return []
    arr = np.frombuffer(blob, dtstr, count=m, offset=phase).astype(np.float64) / scale
    idx = np.nonzero(np.abs(arr - PIVOT_T) <= TOL)[0]
    if idx.size == 0:
        return []
    return (base + phase + idx.astype(np.int64) * eb).tolist()


def _m_pivot_f16(base, blob, _u):
    phase = (-base) & 1
    m = (len(blob) - phase) // 2
    if m <= 0:
        return []
    arr = np.frombuffer(blob, "<f2", count=m, offset=phase).astype(np.float64)
    idx = np.nonzero(np.isfinite(arr) & (np.abs(arr - PIVOT_T) <= TOL))[0]
    if idx.size == 0:
        return []
    return (base + phase + idx.astype(np.int64) * 2).tolist()


# STAGE-1 run table: (label, matcher module/func, args, item_size, which encs it
# can confirm in STAGE 2)
_STAGE1 = [
    ("f32c", ("memory_reader.weather.pivot_scan", "_m_pivot_f32"), ("c",), 4, ["f32_celsius"]),
    ("f32k", ("memory_reader.weather.pivot_scan", "_m_pivot_f32"), ("k",), 4, ["f32_kelvin"]),
    ("i32x10",  ("memory_reader.weather.pivot_scan", "_m_pivot_i"), ("<i4", 4, 10.0), 4, ["i32_x10"]),
    ("i32x100", ("memory_reader.weather.pivot_scan", "_m_pivot_i"), ("<i4", 4, 100.0), 4, ["i32_x100"]),
    ("i32x1000",("memory_reader.weather.pivot_scan", "_m_pivot_i"), ("<i4", 4, 1000.0), 4, ["i32_x1000"]),
    ("i32x1e5", ("memory_reader.weather.pivot_scan", "_m_pivot_i"), ("<i4", 4, 100000.0), 4, ["i32_x100000"]),
    ("i16x10",  ("memory_reader.weather.pivot_scan", "_m_pivot_i"), ("<i2", 2, 10.0), 2, ["i16_x10"]),
    ("i16x100", ("memory_reader.weather.pivot_scan", "_m_pivot_i"), ("<i2", 2, 100.0), 2, ["i16_x100"]),
    ("f16c",    ("memory_reader.weather.pivot_scan", "_m_pivot_f16"), (0,), 2, ["f16_celsius"]),
]


# non-pivot region anchors, checked cheapest-first (tiny per-element reads, abort
# on first mismatch) so a wrong layout costs ~1 RPM not a full-span read.
_CHECK_ORDER = [160, 20, 129, 292]   # 24,8,11,16C


def _classify_pivot(p, pivot_addr, encs):
    """pivot_addr is a candidate region-31 TEMP element. For each (enc,stride,ib),
    region-ib elem0 = pivot_addr - (31-ib)*stride. Verify the 4 other anchors with
    tiny per-element reads, aborting on first mismatch."""
    out = []
    for enc in encs:
        dtstr, eb, conv = _ENC[enc]
        for stride in _STRIDES:
            if stride < eb or (stride % eb):
                continue
            for ib in (0, 1):
                ib_base = pivot_addr - (PIVOT_RID - ib) * stride
                ok = True
                reads = {PIVOT_RID: PIVOT_T}
                for rid in _CHECK_ORDER:
                    addr = ib_base + (rid - ib) * stride
                    raw = p.try_read(addr, eb)
                    if len(raw) != eb:
                        ok = False
                        break
                    vv = float(conv(np.frombuffer(raw, dtstr))[0])
                    if not np.isfinite(vv) or abs(vv - _TEMPS[rid]) > TOL:
                        ok = False
                        break
                    reads[rid] = round(vv, 3)
                if ok:
                    out.append({"enc": enc, "stride": stride, "ib": ib,
                                "ib_base": "0x%X" % ib_base, "reads": reads})
    return out


def run(pid, arena, regions, workers=6):
    all_conf = []
    p = _wm.Process(pid, "hoi4.exe")
    try:
        for (label, mf, args, item, encs) in _STAGE1:
            t0 = time.time()
            hits = fs.parallel_scan(pid, arena[0], arena[1], matcher=mf,
                                    matcher_args=args, item_size=item,
                                    workers=workers, regions=regions)
            dt = time.time() - t0
            if len(hits) > 400000:
                print("  STAGE1 %-9s -> %7d pivot hits (%.1fs) -> SKIP classify (band too wide)"
                      % (label, len(hits), dt), flush=True)
                continue
            conf = []
            for hv in hits:
                conf.extend(_classify_pivot(p, hv, encs))
            print("  STAGE1 %-9s -> %7d pivot hits (%.1fs) -> %d confirmed layouts"
                  % (label, len(hits), dt, len(conf)), flush=True)
            all_conf.extend(conf)
    finally:
        p.close()
    return all_conf


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
    by = Counter((c["enc"], c["stride"], c["ib"]) for c in conf)
    print("CONFIRMED LAYOUTS: %d" % len(conf), flush=True)
    for k, c in by.most_common(40):
        print("   enc=%-12s stride=%2d ib=%d  x%d" % (k[0], k[1], k[2], c), flush=True)
    for c in conf[:50]:
        print("   %s enc=%s stride=%d ib=%d reads=%s"
              % (c["ib_base"], c["enc"], c["stride"], c["ib"], c["reads"]), flush=True)
    json.dump(conf, open(os.path.join(_HERE, "_pivot_confirmed.json"), "w"), indent=1)
    print("wrote _pivot_confirmed.json", flush=True)


if __name__ == "__main__":
    main()
