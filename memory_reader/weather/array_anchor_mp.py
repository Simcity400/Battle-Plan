"""array_anchor_mp.py - 5-anchor weather-array scan on TRUE multiprocessing via
fastscan.parallel_scan (6 spawn workers -> real parallelism, no GIL).

The region-object vtable scan already proved fastscan's spawn pool works in this
environment (304 hits, 3.7s). We reuse it: a top-level, picklable matcher that, for
ONE (encoding, stride, ib), returns candidate array-base addresses where the 5
anchors all match. We run f32_celsius FIRST (overwhelming prior), then escalate.

matcher(base, blob, enc_name, stride, ib) -> [base_addr_of_region_ib_element, ...]
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
_RIDS = np.array([20, 31, 129, 160, 292], dtype=np.int64)
_TEMPS = np.array([8.0, 26.0, 11.0, 24.0, 16.0], dtype=np.float64)

# encoding registry: name -> (numpy dtype str, element bytes, kind)
# kind: 'fc'=float celsius, 'fk'=float kelvin, 'div<N>'=int/N, 'raw'=int as-is
_ENC = {
    "f32_celsius": ("<f4", 4, "fc"),
    "f32_kelvin":  ("<f4", 4, "fk"),
    "i32_x10":     ("<i4", 4, "div10"),
    "i32_x100":    ("<i4", 4, "div100"),
    "i32_x1000":   ("<i4", 4, "div1000"),
    "i32_x100000": ("<i4", 4, "div100000"),
    "i32_raw":     ("<i4", 4, "raw"),
    "i16_x10":     ("<i2", 2, "div10"),
    "i16_x100":    ("<i2", 2, "div100"),
    "i16_raw":     ("<i2", 2, "raw"),
    "f16_celsius": ("<f2", 2, "fc"),
}


def _to_celsius(arr, kind):
    a = arr.astype(np.float64)
    if kind == "fc":
        return a
    if kind == "fk":
        return a - _K
    if kind == "raw":
        return a
    if kind.startswith("div"):
        return a / float(kind[3:])
    raise ValueError(kind)


_STRIDES_ALL = (4, 8, 16, 32, 64)


def _m_weather_anchor(base, blob, enc_name, stride, ib):
    """fastscan matcher: candidate region-`ib` element base addrs in this blob for
    the given (enc, stride, ib). Returns absolute addresses (PIVOT element addr)."""
    dtstr, eb, kind = _ENC[enc_name]
    dt = np.dtype(dtstr)
    if stride < eb or (stride % eb):
        return []
    step = stride // eb
    n = len(blob)
    rel = (_RIDS - ib) * step
    omin = int(rel.min())
    g = (rel - omin).astype(np.int64)
    gmax = int(g.max())
    out = []
    for ph in range(eb):
        m = (n - ph) // eb
        if m <= gmax + 1:
            continue
        arr = np.frombuffer(blob, dt, count=m, offset=ph)
        cv = _to_celsius(arr, kind)
        hi = m - gmax
        if hi <= 0:
            continue
        mask = None
        for k in range(len(_RIDS)):
            seg = cv[g[k]: g[k] + hi]
            mk = np.isfinite(seg) & (np.abs(seg - _TEMPS[k]) <= TOL)
            mask = mk if mask is None else (mask & mk)
            if not mask.any():
                break
        if mask is None or not mask.any():
            continue
        for e in np.nonzero(mask)[0].tolist():
            out.append(base + ph + e * eb)
    return out


def _m_weather_all(base, blob, enc_names):
    """ONE-PASS matcher: test ALL (enc in enc_names) x stride x ib per chunk; decode
    each (enc, phase) only ONCE; return PIVOT element addresses for any match. The
    parent re-classifies the (few) hits to recover (enc, stride, ib). Single arena
    pass -> ~1/110 the I/O of the per-combo driver."""
    n = len(blob)
    out = []
    for enc_name in enc_names:
        dtstr, eb, kind = _ENC[enc_name]
        dt = np.dtype(dtstr)
        for ph in range(eb):
            m = (n - ph) // eb
            if m < 16:
                continue
            arr = np.frombuffer(blob, dt, count=m, offset=ph)
            cv = _to_celsius(arr, kind)
            # quick reject: region 31 wants 26C; if no element in band, skip combos
            # (cheap global check across the whole phase array)
            if not np.any(np.isfinite(cv) & (np.abs(cv - 26.0) <= TOL)):
                continue
            for stride in _STRIDES_ALL:
                if stride < eb or (stride % eb):
                    continue
                step = stride // eb
                for ib in (0, 1):
                    rel = (_RIDS - ib) * step
                    omin = int(rel.min())
                    g = (rel - omin).astype(np.int64)
                    gmax = int(g.max())
                    hi = m - gmax
                    if hi <= 0:
                        continue
                    mask = None
                    for k in range(len(_RIDS)):
                        seg = cv[g[k]: g[k] + hi]
                        mk = np.isfinite(seg) & (np.abs(seg - _TEMPS[k]) <= TOL)
                        mask = mk if mask is None else (mask & mk)
                        if not mask.any():
                            break
                    if mask is None or not mask.any():
                        continue
                    for e in np.nonzero(mask)[0].tolist():
                        out.append(base + ph + e * eb)
    return out


def scan_one(pid, arena, regions, enc_name, stride, ib, workers=6):
    """Run ONE (enc,stride,ib) over the arena via fastscan spawn pool."""
    dtstr, eb, kind = _ENC[enc_name]
    # item_size for overlap: widest anchor span in bytes
    item = int(_RIDS.max() * stride + stride + 8)
    hits = fs.parallel_scan(
        pid, arena[0], arena[1],
        matcher=("memory_reader.weather.array_anchor_mp", "_m_weather_anchor"),
        matcher_args=(enc_name, stride, ib),
        item_size=item, workers=workers, regions=regions)
    return hits


def _classify_hit(p, pivot_addr, enc_names):
    """Given a pivot element address that matched SOME combo, find which (enc,
    stride, ib) layout it satisfies, and return the region-ib element base. Reads a
    window around the pivot and re-tests all combos exactly."""
    results = []
    # read a generous window: pivot is the smallest-rel anchor, so the array spans
    # forward up to (max_rid - min_rid)*max_stride bytes; also a little before.
    span = int((292) * max(_STRIDES_ALL) + 64)
    raw = p.try_read(pivot_addr, span)
    for enc_name in enc_names:
        dtstr, eb, kind = _ENC[enc_name]
        for stride in _STRIDES_ALL:
            if stride < eb or (stride % eb):
                continue
            step = stride // eb
            for ib in (0, 1):
                rel = (_RIDS - ib) * step
                omin = int(rel.min())
                g = (rel - omin)        # element offsets from pivot
                ok = True
                vals = {}
                for k in range(len(_RIDS)):
                    off = g[k] * eb
                    if off + eb > len(raw):
                        ok = False
                        break
                    v = _to_celsius(np.frombuffer(raw[off:off+eb], np.dtype(dtstr)), kind)
                    vv = float(v[0])
                    if not np.isfinite(vv) or abs(vv - _TEMPS[k]) > TOL:
                        ok = False
                        break
                    vals[int(_RIDS[k])] = round(vv, 3)
                if ok:
                    ib_base = pivot_addr - omin * eb
                    results.append({"enc": enc_name, "stride": stride, "ib": ib,
                                    "ib_base": "0x%X" % ib_base, "reads": vals})
    return results


def main():
    import argparse
    ap = argparse.ArgumentParser()
    ap.add_argument("--workers", type=int, default=6)
    ap.add_argument("--encs", default="f32_celsius",
                    help="comma list or 'all'")
    ap.add_argument("--strides", default="4,8,16,32,64")
    ap.add_argument("--one-pass", action="store_true",
                    help="single arena pass testing all encs x strides x ib")
    args = ap.parse_args()
    pid = _wm.find_pid("hoi4.exe")
    if pid is None:
        raise SystemExit("hoi4.exe not running")
    arena = fs.discover_arena(pid)
    print("arena 0x%X..0x%X (%.2f GB)" % (arena[0], arena[1], (arena[1]-arena[0])/2**30), flush=True)
    # enumerate regions ONCE, reuse across all (enc,stride,ib) runs
    h = fs._k32.OpenProcess(fs._PROCESS_VM_READ | fs._PROCESS_QUERY_INFORMATION, False, pid)
    try:
        regions = fs.enumerate_regions(h, arena[0], arena[1])
    finally:
        fs._k32.CloseHandle(h)
    print("regions: %d" % len(regions), flush=True)

    encs = list(_ENC) if args.encs == "all" else args.encs.split(",")
    strides = [int(s) for s in args.strides.split(",")]

    if args.one_pass:
        item = int(_RIDS.max() * max(_STRIDES_ALL) + max(_STRIDES_ALL) + 8)
        t0 = time.time()
        pivots = fs.parallel_scan(
            pid, arena[0], arena[1],
            matcher=("memory_reader.weather.array_anchor_mp", "_m_weather_all"),
            matcher_args=(tuple(encs),),
            item_size=item, workers=args.workers, regions=regions)
        dt = time.time() - t0
        print("ONE-PASS scan %.1fs -> %d pivot hits" % (dt, len(pivots)), flush=True)
        p = _wm.Process(pid, "hoi4.exe")
        classified = []
        try:
            for pv in pivots:
                for r in _classify_hit(p, pv, encs):
                    r["pivot_addr"] = "0x%X" % pv
                    classified.append(r)
        finally:
            p.close()
        by = Counter((r["enc"], r["stride"], r["ib"]) for r in classified)
        print("classified %d layouts:" % len(classified))
        for k, c in by.most_common(40):
            print("   enc=%-12s stride=%2d ib=%d  x%d" % (k[0], k[1], k[2], c), flush=True)
        for r in classified[:40]:
            print("   %s S=%d ib=%d enc=%s reads=%s"
                  % (r["ib_base"], r["stride"], r["ib"], r["enc"], r["reads"]), flush=True)
        json.dump(classified, open(os.path.join(_HERE, "_array_anchor_onepass.json"), "w"), indent=1)
        print("wrote _array_anchor_onepass.json", flush=True)
        return

    allhits = []
    for enc in encs:
        for S in strides:
            for ib in (0, 1):
                t0 = time.time()
                hits = scan_one(pid, arena, regions, enc, S, ib, workers=args.workers)
                dt = time.time() - t0
                # convert each pivot addr -> region-ib element base (array start)
                dtstr, eb, kind = _ENC[enc]
                step = S // eb
                omin = int(((_RIDS - ib) * step).min())
                if hits:
                    ibbases = [a - omin * eb for a in hits]
                    print("  enc=%-12s S=%2d ib=%d -> %d hits (%.1fs)  ib_base e.g. %s"
                          % (enc, S, ib, len(hits), dt, ["0x%X" % a for a in ibbases[:5]]), flush=True)
                    for a in ibbases:
                        allhits.append({"enc": enc, "stride": S, "ib": ib, "base_addr": "0x%X" % a})
                else:
                    print("  enc=%-12s S=%2d ib=%d -> 0 (%.1fs)" % (enc, S, ib, dt), flush=True)
    json.dump(allhits, open(os.path.join(_HERE, "_array_anchor_mp_hits.json"), "w"), indent=1)
    print("TOTAL %d hits; wrote _array_anchor_mp_hits.json" % len(allhits), flush=True)


if __name__ == "__main__":
    main()
