"""feas2.py - final feasibility (spawn-safe): re-confirm pause + measure whether
the 5 anchor temps CO-OCCUR as f32 in any small window (no order/stride assumption)."""
import ctypes, hashlib, os, sys, time
import numpy as np

_HERE = os.path.dirname(os.path.abspath(__file__)); _MR = os.path.dirname(_HERE)
for _p in (_MR, _HERE):
    if _p not in sys.path:
        sys.path.insert(0, _p)
import winmem as _wm
import fastscan as fs
from weather.coloc_temp_pivot import _m_temp26_f32  # importable matcher


def main():
    pid = _wm.find_pid("hoi4.exe")
    p = _wm.Process(pid, "hoi4.exe")
    mb = p.module_base
    arena = fs.discover_arena(pid)
    # re-enumerate region objects FRESH this run (vtable scan)
    spec = fs.make_qword_eq_matcher(mb + 0x29468B8)
    hits = fs.parallel_scan(pid, arena[0], arena[1], workers=6, **spec)
    id2obj = {}
    for h in hits:
        raw = p.try_read(h + 0x58, 4)
        if len(raw) == 4:
            rid = int(np.frombuffer(raw, "<u4")[0])
            if 1 <= rid <= 304 and rid not in id2obj:
                id2obj[rid] = h
    anchors_present = [r for r in (20, 31, 129, 160, 292) if r in id2obj]
    print("re-enum region objs: %d  anchors present: %s" % (len(id2obj), anchors_present), flush=True)

    # pause check on region 31
    o31 = id2obj.get(31)
    if o31:
        a = hashlib.md5(p.try_read(o31, 0x400)).hexdigest()
        time.sleep(3)
        b = hashlib.md5(p.try_read(o31, 0x400)).hexdigest()
        print("region 31 obj stable over 3s: %s -> %s"
              % (a == b, "PAUSED (weather static)" if a == b else "CHANGED (running/drift!)"), flush=True)

    # co-occurrence: f32 26C sites, count anchors in +-2KB window
    sites = fs.parallel_scan(pid, arena[0], arena[1],
                             matcher=("memory_reader.weather.coloc_temp_pivot", "_m_temp26_f32"),
                             matcher_args=("c",), item_size=4, workers=6)
    print("26C f32 sites: %d" % len(sites), flush=True)
    others = [8.0, 11.0, 24.0, 16.0]
    TOL = 1.6
    WIN = 2048
    hist = {1: 0, 2: 0, 3: 0, 4: 0, 5: 0}
    best = 0; best_addr = None
    for s in sites:
        raw = p.try_read(s - WIN, 2 * WIN + 4)
        if len(raw) < 2 * WIN:
            continue
        arr = np.frombuffer(raw[:(len(raw) // 4) * 4], "<f4").astype(np.float64)
        cnt = 1
        for t in others:
            if np.any(np.isfinite(arr) & (np.abs(arr - t) <= TOL)):
                cnt += 1
        hist[cnt] = hist.get(cnt, 0) + 1
        if cnt > best:
            best = cnt; best_addr = s
    print("co-occurrence histogram (anchors-of-5 within 4KB of each 26C site):", flush=True)
    for k in sorted(hist):
        print("   %d-of-5: %d sites" % (k, hist[k]), flush=True)
    print("BEST: %d/5 at 0x%X" % (best, best_addr or 0), flush=True)
    if best == 5:
        # dump the window for the best site
        raw = p.try_read(best_addr - WIN, 2 * WIN + 4)
        arr = np.frombuffer(raw[:(len(raw) // 4) * 4], "<f4").astype(np.float64)
        for t in [26.0] + others:
            pos = np.nonzero(np.isfinite(arr) & (np.abs(arr - t) <= TOL))[0]
            print("   temp %.0f at window byte offsets: %s"
                  % (t, [(int(x) * 4 - WIN) for x in pos[:6]]), flush=True)
    p.close()


if __name__ == "__main__":
    main()
