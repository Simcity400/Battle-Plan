"""idmap_scan.py - PERMUTATION-TOLERANT weather-array discovery (build 16022851).

Codex's strongest defense against a wrong region-index assumption: instead of
assuming slot == region_id-ib, find STRUCTURAL evidence that ties an anchor's
region id to its temperature in memory. Two complementary scans, both READ-ONLY:

(A) ID-MAP scan: find a contiguous array of ~304/305 int16 or int32 values that
    is a permutation/superset of region ids 1..304 (most values unique, in range,
    high coverage). the game vectors are often id-indexed; the slot ORDER of such a
    map tells us the true permutation, so we can then read temps by position.

(B) CO-LOCATION record scan: find byte windows that contain BOTH an anchor's
    region id (as int16/int32) AND that anchor's temperature (~match, several
    encodings) within a small struct span (<= 256 B). If the SAME relative
    (id_off, temp_off, stride) layout holds for multiple anchors at a constant
    stride, that is a strategic-region / weather record array with an embedded id
    field -> the mapping is self-describing (no slot==id assumption needed).

These directly attack the "memory order != region-id order" failure mode.
"""
import ctypes as C
import os
import sys
import threading
import time

import numpy as np

_HERE = os.path.dirname(os.path.abspath(__file__))
_MR = os.path.dirname(_HERE)
for _p in (_MR, _HERE):
    if _p not in sys.path:
        sys.path.insert(0, _p)
import winmem as _wm
import fastscan as fs

# region ids from authoritative mod files (Libya = 292, not the stale climo's 225)
ANCHOR_ID_TEMP = [(20, 8.0), (31, 26.0), (129, 11.0), (160, 24.0), (292, 16.0)]
ALL_RIDS = set(range(1, 305))
_CHUNK = 16 << 20
_K = 273.15


class _Reader(object):
    def __init__(self, pid):
        self.h = _wm._k32.OpenProcess(
            _wm.PROCESS_VM_READ | _wm.PROCESS_QUERY_INFORMATION, False, pid)
        if not self.h:
            raise OSError("OpenProcess failed err %d" % C.get_last_error())

    def rpm(self, addr, n):
        buf = (C.c_char * n)()
        got = C.c_size_t(0)
        ok = _wm._k32.ReadProcessMemory(self.h, C.c_void_p(addr), buf, n, C.byref(got))
        if not ok and got.value == 0:
            return b""
        return bytes(buf[:got.value])

    def close(self):
        if self.h:
            _wm._k32.CloseHandle(self.h)
            self.h = None


# --------------------------------------------------------------------------
# (A) ID-MAP scan: a window of N consecutive int16/int32 that is mostly a
# permutation of 1..304. We slide a length-N window over the typed view and
# require: all values in [1,320], >= COV unique values that fall in [1,304].
# --------------------------------------------------------------------------
def _find_idmaps(blob, base, hits, dt, ebytes, N=304, cov=0.90):
    n = len(blob)
    for phase in range(ebytes):
        m = (n - phase) // ebytes
        if m < N + 2:
            continue
        arr = np.frombuffer(blob[phase:phase + m * ebytes], dt).astype(np.int64)
        inrange = (arr >= 1) & (arr <= 320)
        # need a window of N where >= cov*N are unique ids in 1..304.
        # Cheap prefilter: cumulative count of inrange over window must be high.
        cs = np.concatenate([[0], np.cumsum(inrange.astype(np.int64))])
        win_inrange = cs[N:] - cs[:-N]   # length m-N+1
        cand = np.nonzero(win_inrange >= int(cov * N))[0]
        for s in cand.tolist():
            w = arr[s:s + N]
            ids = w[(w >= 1) & (w <= 304)]
            uniq = np.unique(ids)
            if uniq.size >= int(cov * N):
                hits.append({"kind": "idmap", "addr": base + phase + s * ebytes,
                             "ebytes": ebytes, "n_unique": int(uniq.size),
                             "coverage": float(uniq.size) / N})


# --------------------------------------------------------------------------
# (B) CO-LOCATION scan: find every position where an anchor region id appears as
# an int16 or int32, then check whether that anchor's TEMP appears within +-W
# bytes (several encodings). Record (id_off, temp_off, temp_enc) per anchor. Then
# the parent looks for a CONSISTENT (stride, id_field_off, temp_field_off) shared
# by multiple anchors -> a record array.
# --------------------------------------------------------------------------
_TEMP_ENCODINGS = [
    ("f32_c", "<f4", 4, lambda a: a.astype(np.float64)),
    ("f32_k", "<f4", 4, lambda a: a.astype(np.float64) - _K),
    ("i32",   "<i4", 4, lambda a: a.astype(np.float64)),
    ("i32_x100", "<i4", 4, lambda a: a.astype(np.float64) / 100.0),
    ("i16",   "<i2", 2, lambda a: a.astype(np.float64)),
    ("f16_c", "<f2", 2, lambda a: a.astype(np.float64)),
    ("i8",    "<i1", 1, lambda a: a.astype(np.float64)),
]


def _colocate(blob, base, hits, window=256, tol=1.5):
    n = len(blob)
    u8 = np.frombuffer(blob, np.uint8)
    for (rid, temp) in ANCHOR_ID_TEMP:
        for id_dt, id_bytes in (("<i4", 4), ("<i2", 2)):
            m = n // id_bytes
            idarr = np.frombuffer(blob[:m * id_bytes], id_dt)
            positions = np.nonzero(idarr == rid)[0]
            if positions.size == 0 or positions.size > 50000:
                continue  # skip absurdly common (e.g. rid as a tiny int everywhere)
            for p in positions.tolist():
                id_byte = p * id_bytes
                lo = max(0, id_byte - window)
                hi = min(n, id_byte + window)
                seg = u8[lo:hi]
                for (ename, tdt, tb, conv) in _TEMP_ENCODINGS:
                    mm = len(seg) // tb
                    for ph in range(tb):
                        k = (len(seg) - ph) // tb
                        if k <= 0:
                            continue
                        tv = np.frombuffer(seg.tobytes()[ph:ph + k * tb], tdt)
                        cv = conv(tv)
                        good = np.nonzero(np.abs(cv - temp) <= tol)[0]
                        for g in good.tolist():
                            temp_byte = lo + ph + g * tb
                            hits.append({"kind": "coloc", "rid": rid,
                                         "id_addr": base + id_byte,
                                         "id_bytes": id_bytes,
                                         "temp_addr": base + temp_byte,
                                         "temp_enc": ename,
                                         "delta": temp_byte - id_byte})


def _scan_region(rd, base, size, do_idmap, do_coloc, hits, lock, cap):
    overlap = 4096
    pos = 0
    local = []
    while pos < size:
        want = min(_CHUNK, size - pos)
        blob = rd.rpm(base + pos, want)
        got = len(blob)
        if got >= 4096:
            b = base + pos
            if do_idmap:
                _find_idmaps(blob, b, local, "<i2", 2)
                _find_idmaps(blob, b, local, "<i4", 4)
            if do_coloc:
                _colocate(blob, b, local)
        if got >= want:
            pos += max(1, got - overlap)
        else:
            pos += max(1, got)
        if len(local) >= cap:
            break
    if local:
        with lock:
            hits.extend(local)


def scan(pid, arena, do_idmap=True, do_coloc=True, workers=6, cap=200000):
    lo, hi = arena
    h = _wm._k32.OpenProcess(
        _wm.PROCESS_VM_READ | _wm.PROCESS_QUERY_INFORMATION, False, pid)
    try:
        regions = fs.enumerate_regions(h, lo, hi)
    finally:
        _wm._k32.CloseHandle(h)
    regions.sort(key=lambda r: r[1], reverse=True)
    bins = [[] for _ in range(workers)]
    load = [0] * workers
    for rb, rs in regions:
        j = load.index(min(load))
        bins[j].append((rb, rs))
        load[j] += rs
    hits = []
    lock = threading.Lock()

    def run(shard):
        rd = _Reader(pid)
        try:
            for rb, rs in shard:
                _scan_region(rd, rb, rs, do_idmap, do_coloc, hits, lock, cap)
        finally:
            rd.close()

    threads = [threading.Thread(target=run, args=(b,)) for b in bins if b]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    return hits


if __name__ == "__main__":
    import argparse
    from collections import Counter
    ap = argparse.ArgumentParser()
    ap.add_argument("--no-coloc", action="store_true")
    ap.add_argument("--no-idmap", action="store_true")
    ap.add_argument("--workers", type=int, default=6)
    args = ap.parse_args()
    pid = _wm.find_pid("hoi4.exe")
    if pid is None:
        raise SystemExit("hoi4.exe not running")
    arena = fs.discover_arena(pid)
    print("arena 0x%X..0x%X" % arena)
    t0 = time.time()
    hits = scan(pid, arena, do_idmap=not args.no_idmap,
                do_coloc=not args.no_coloc, workers=args.workers)
    print("scan %.1fs -> %d hits" % (time.time() - t0, len(hits)))
    idmaps = [h for h in hits if h["kind"] == "idmap"]
    colocs = [h for h in hits if h["kind"] == "coloc"]
    print("ID-MAPS: %d" % len(idmaps))
    idmaps.sort(key=lambda h: -h["coverage"])
    for h in idmaps[:20]:
        print("  0x%X ebytes=%d uniq=%d cov=%.3f"
              % (h["addr"], h["ebytes"], h["n_unique"], h["coverage"]))
    print("CO-LOCATIONS: %d" % len(colocs))
    # group by (delta, temp_enc, id_bytes) to find a consistent record layout
    by = Counter((h["delta"], h["temp_enc"], h["id_bytes"]) for h in colocs)
    print("top (delta, enc, idbytes) by count:")
    for k, c in by.most_common(25):
        print("   delta=%d enc=%s idb=%d  x%d" % (k[0], k[1], k[2], c))
