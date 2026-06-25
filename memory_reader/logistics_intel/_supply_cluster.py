"""_supply_cluster.py - arena-wide VALUE-CLUSTER scan for the supply anchors.

READ-ONLY. Independent of any object hypothesis. For each anchor VALUE we scan the whole
arena for addresses holding it (f32 + fixed-point), then look for clusters where several
DISTINCT anchor values co-locate within a small window (an array/struct). This:
  * tells us whether the values are even STORED (if 6.60 / 11.83 etc. appear at all);
  * finds a state-supply array by co-location of [2.80,2.90,1.40,6.60,0.60];
  * finds a per-province remaining array by co-location of [11.83,9.64,9.52,7.48,0.65];
  * finds a hub-supply array by co-location of [11.83,6.73,9.52,7.48,0.00].
Distinctive (non-round, less aliasing) values are weighted: 11.83, 9.64, 6.73, 9.52, 7.48.
Uses fastscan (workers=3) for the heavy finds.
"""
import json
import os
import struct
import sys
from collections import defaultdict

_HERE = os.path.dirname(os.path.abspath(__file__))
_MR = os.path.dirname(_HERE)
for _p in (_MR, _HERE):
    if _p not in sys.path:
        sys.path.insert(0, _p)
import winmem            # noqa: E402
import fastscan as fs    # noqa: E402

# anchor value SETS to look for as clusters
SETS = {
    "state_supply":   {42: 2.80, 53: 2.90, 157: 1.40, 219: 6.60, 237: 0.60},
    "prov_remaining": {42: 11.83, 53: 9.64, 157: 9.52, 219: 7.48, 237: 0.65},
    "hub_supply":     {42: 11.83, 53: 6.73, 157: 9.52, 219: 7.48, 237: 0.00},
}
TOL = 0.02   # tight (UI 2-dp -> raw ~+-0.005..0.01)


def _m_f32_near(base, blob, value, tol):
    import numpy as np
    phase = (-base) & 3
    n = (len(blob) - phase) & ~3
    if n <= 0:
        return []
    arr = np.frombuffer(blob, dtype="<f4", count=n >> 2, offset=phase)
    idx = np.nonzero(np.abs(arr - np.float32(value)) <= tol)[0]
    if idx.size == 0:
        return []
    return (base + phase + (idx.astype(np.uint64) << np.uint64(2))).tolist()


def _m_fx_near(base, blob, value, tol, scale, signed):
    """fixed-point i32/u32 / scale within tol of value, on 4-aligned slots."""
    import numpy as np
    phase = (-base) & 3
    n = (len(blob) - phase) & ~3
    if n <= 0:
        return []
    dt = "<i4" if signed else "<u4"
    arr = np.frombuffer(blob, dtype=dt, count=n >> 2, offset=phase).astype("f8") / scale
    idx = np.nonzero(np.abs(arr - value) <= tol)[0]
    if idx.size == 0:
        return []
    return (base + phase + (idx.astype(np.uint64) << np.uint64(2))).tolist()


def _thread_scan(pid, arena, matcher_fn, item_size=4, workers=3):
    """Thread-pool arena scan (fallback when mp spawn fails / WinError 5).

    ReadProcessMemory releases the GIL, so threads parallelize the I/O-bound scan.
    Each thread opens its OWN read-only handle (handles are not thread-shared safely
    for concurrent RPM offset bookkeeping; per-thread handle is cleanest)."""
    from concurrent.futures import ThreadPoolExecutor
    lo, hi = arena
    h0 = fs._k32.OpenProcess(fs._PROCESS_VM_READ | fs._PROCESS_QUERY_INFORMATION,
                             False, pid)
    try:
        regions = fs.enumerate_regions(h0, lo, hi)
    finally:
        fs._k32.CloseHandle(h0)
    shards = fs.shard_regions(regions, workers)

    def run(shard):
        rd = fs._Reader(pid)
        try:
            out = []
            for base, size in shard:
                out.extend(rd.scan_region(base, size, matcher_fn, item_size))
            return out
        finally:
            rd.close()

    hits = set()
    with ThreadPoolExecutor(max_workers=len(shards)) as ex:
        for r in ex.map(run, shards):
            hits.update(r)
    return sorted(hits)


def find_all(pid, arena, value, enc, workers=3):
    if enc == "f32":
        fn = lambda b, blob: _m_f32_near(b, blob, value, TOL)   # noqa: E731
        return _thread_scan(pid, arena, fn, item_size=4, workers=workers)
    scale, signed = {"fx100000": (100000.0, True), "fx1000": (1000.0, True),
                     "fx100": (100.0, True), "u32x100": (100.0, False),
                     "fp16": (65536.0, True), "fp16u": (65536.0, False)}[enc]
    fn = lambda b, blob: _m_fx_near(b, blob, value, TOL, scale, signed)  # noqa: E731
    return _thread_scan(pid, arena, fn, item_size=4, workers=workers)


def cluster(addr_lists, win=0x400):
    """addr_lists: {sid: [addrs]}. Find windows of `win` bytes containing >=3 distinct sids."""
    tagged = []
    for sid, addrs in addr_lists.items():
        for a in addrs:
            tagged.append((a, sid))
    tagged.sort()
    out = []
    n = len(tagged)
    i = 0
    while i < n:
        a0, _ = tagged[i]
        j = i
        sids = {}
        while j < n and tagged[j][0] - a0 <= win:
            sids.setdefault(tagged[j][1], tagged[j][0])
            j += 1
        if len(sids) >= 3:
            out.append({"lo": a0, "span": tagged[j - 1][0] - a0,
                        "sids": {s: hex(x) for s, x in sids.items()}})
        i += 1
    return out


def main():
    p = winmem.open_process("hoi4.exe")
    arena = fs.discover_arena(p.pid)
    pid = p.pid
    print("pid %d arena 0x%X..0x%X (%.2f GB)" %
          (pid, arena[0], arena[1], (arena[1] - arena[0]) / float(1 << 30)))
    p.close()

    encs = ["f32", "fx100000", "fx1000", "fx100", "u32x100"]
    summary = {}
    for setname, anchors in SETS.items():
        print("\n===== SET %s =====" % setname)
        for enc in encs:
            addr_lists = {}
            counts = {}
            for sid, val in anchors.items():
                if val == 0.0 and enc != "f32":
                    # 0 is everywhere in fixed-point; skip it as a cluster member
                    addr_lists[sid] = []
                    counts[sid] = -1
                    continue
                a = find_all(pid, arena, val, enc)
                addr_lists[sid] = a
                counts[sid] = len(a)
            print("  enc=%-9s counts=%s" % (enc, counts))
            cl = cluster(addr_lists)
            if cl:
                print("    >>> %d CLUSTER(s) with >=3 distinct anchors:" % len(cl))
                for c in cl[:10]:
                    print("        lo=0x%X span=0x%X sids=%s" % (c["lo"], c["span"], c["sids"]))
                summary["%s/%s" % (setname, enc)] = cl[:50]
    out = os.path.join(_HERE, "_cluster_results.json")
    with open(out, "w") as f:
        json.dump(summary, f, indent=2)
    print("\nwrote", out)


if __name__ == "__main__":
    main()
