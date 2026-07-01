"""pivot_scan_prov.py - rare-pivot 5-anchor scan indexed by PROVINCE id.

Tests the per-PROVINCE weather-array hypothesis: the game's in-game tooltip shows the
PROVINCE weather, and weather may be realized per province (~13000-21000 entries).
Same fast rare-pivot method as pivot_scan.py but the 5 anchors are PROVINCE ids:

    prov 7206  -> 16C  (region 292)
    prov 3946  ->  8C  (region 20)
    prov 6945  -> 11C  (region 129)
    prov 7250  -> 26C  (region 31)   <- RAIN; 26C is the rarest -> pivot
    prov 10167 -> 24C  (region 160)

Pivot = the 26C province (7250). STAGE 1: every arena position with decoded value
~26C. STAGE 2: treat each as province-7250's temp element; for every (enc, stride,
ib) require all 5 province anchors to match at (pid - ib)*stride. f32_celsius and
f32_kelvin first (strong priors). READ-ONLY, multiprocess via fastscan.
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
# reuse the band matchers + encoding table from the region pivot scan
from weather import pivot_scan as RPS

_K = 273.15
TOL = 1.6
# province anchors (province_id -> temp_C)
ANCH = [(7206, 16.0), (3946, 8.0), (6945, 11.0), (7250, 26.0), (10167, 24.0)]
TEMPS = {p: t for p, t in ANCH}
PIVOT_PID = 7250
PIVOT_T = 26.0
_STRIDES = (4, 8, 16, 32, 64)
PIDS = [p for p, _ in ANCH]
MAXP = max(PIDS)


# classify by reading only the 5 anchor elements (tiny reads), cheapest-first
# ordering so a wrong layout aborts after ~1 read. The pivot (7250=26C) is already
# known true; check the next-rarest anchor (24C) first.
_CHECK_ORDER = [10167, 3946, 6945, 7206]   # all non-pivot anchors, rare-ish first


def _classify_pivot_prov(p, pivot_addr, encs):
    """pivot_addr = candidate province-7250 temp element. For each (enc,stride,ib),
    province-ib elem0 addr = pivot_addr - (7250-ib)*stride. Verify the 4 other prov
    anchors with tiny per-element reads, aborting on first mismatch."""
    out = []
    for enc in encs:
        dtstr, eb, conv = RPS._ENC[enc]
        for stride in _STRIDES:
            if stride < eb or (stride % eb):
                continue
            for ib in (0, 1):
                ib_base = pivot_addr - (PIVOT_PID - ib) * stride
                ok = True
                reads = {PIVOT_PID: PIVOT_T}
                for pid_ in _CHECK_ORDER:
                    addr = ib_base + (pid_ - ib) * stride
                    raw = p.try_read(addr, eb)
                    if len(raw) != eb:
                        ok = False
                        break
                    vv = float(conv(np.frombuffer(raw, dtstr))[0])
                    if not np.isfinite(vv) or abs(vv - TEMPS[pid_]) > TOL:
                        ok = False
                        break
                    reads[pid_] = round(vv, 3)
                if ok:
                    out.append({"enc": enc, "stride": stride, "ib": ib,
                                "ib_base": "0x%X" % ib_base, "reads": reads})
    return out


def run(pid, arena, regions, encs_stage1, workers=6):
    """encs_stage1: list of (label, matcher, args, item, [encs]) like RPS._STAGE1."""
    conf_all = []
    p = _wm.Process(pid, "hoi4.exe")
    try:
        for (label, mf, args, item, encs) in encs_stage1:
            t0 = time.time()
            hits = fs.parallel_scan(pid, arena[0], arena[1], matcher=mf,
                                    matcher_args=args, item_size=item,
                                    workers=workers, regions=regions)
            dt = time.time() - t0
            conf = []
            # cap classify work: if a band matched absurdly many positions it's a
            # useless wide band -> skip classify (only the f32 bands are selective).
            if len(hits) > 400000:
                print("  STAGE1 %-9s -> %7d pivot hits (%.1fs) -> SKIP classify (band too wide)"
                      % (label, len(hits), dt), flush=True)
                continue
            for hv in hits:
                conf.extend(_classify_pivot_prov(p, hv, encs))
            print("  STAGE1 %-9s -> %7d pivot hits (%.1fs) -> %d confirmed"
                  % (label, len(hits), dt, len(conf)), flush=True)
            conf_all.extend(conf)
    finally:
        p.close()
    return conf_all


def main():
    import argparse
    ap = argparse.ArgumentParser()
    ap.add_argument("--workers", type=int, default=6)
    ap.add_argument("--floats-only", action="store_true",
                    help="only f32 celsius+kelvin (fast, strongest prior)")
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
    if args.floats_only:
        stage1 = [s for s in RPS._STAGE1 if s[0] in ("f32c", "f32k", "f16c")]
    else:
        stage1 = RPS._STAGE1
    conf = run(pid, arena, regions, stage1, workers=args.workers)
    by = Counter((c["enc"], c["stride"], c["ib"]) for c in conf)
    print("CONFIRMED PROVINCE LAYOUTS: %d" % len(conf), flush=True)
    for k, c in by.most_common(40):
        print("   enc=%-12s stride=%2d ib=%d  x%d" % (k[0], k[1], k[2], c), flush=True)
    for c in conf[:50]:
        print("   %s enc=%s stride=%d ib=%d reads=%s"
              % (c["ib_base"], c["enc"], c["stride"], c["ib"], c["reads"]), flush=True)
    json.dump(conf, open(os.path.join(_HERE, "_pivot_prov_confirmed.json"), "w"), indent=1)
    print("wrote _pivot_prov_confirmed.json", flush=True)


if __name__ == "__main__":
    main()
