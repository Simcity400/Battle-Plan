"""anchor_scan.py - EXACT-VALUE anchor hunt for the live the game weather temperature
array (build 16022851). READ-ONLY, arena-scoped, threaded, chunked, numpy-vectorized.

This REPLACES the fuzzy climatology-correlation approach (which could not
disambiguate a paused frame). We now have 5 EXACT in-game tooltip readings mapped
to their strategic-region indices:

    region 20  France/Midi-Pyrenees   8 C   clear
    region 31  India/Deccan          26 C   RAIN
    region 129 Turkey/Afyon          11 C   clear
    region 160 Philippines/Luzon     24 C   clear
    region 225 Libya/Cyrenaica       16 C   clear

We scan the committed-RW heap arena for a contiguous (or strided) array A such
that A[rid - ib] == anchor_temp (within +-tol C) for ALL 5 anchors at once, over
many encodings (f32 C / f32 K / i32 / i16 / fixed *10/*100/*1000/*1e5 / f16 / f64)
and strides (4,8,16,24,32) and field offsets.

Because the anchor indices are sparse (max 225) but the VALUES are a distinctive
spread (8..26 C with the precise pattern), 5 simultaneous exact matches is highly
selective. We do NOT build a [candidates x regions] matrix (the prior agent OOMed
at 19 GB): we vectorize over candidate array-base offsets within each chunk,
applying the 5 tolerance masks as a streaming AND.

Threaded (not mp 'spawn': this sandbox dies WinError 5 on DuplicateHandle).
numpy + RPM release the GIL so threads scale.
"""
import ctypes as C
import json
import os
import sys
import threading
import time
from ctypes import wintypes as W

import numpy as np

_HERE = os.path.dirname(os.path.abspath(__file__))
_MR = os.path.dirname(_HERE)
for _p in (_MR, _HERE):
    if _p not in sys.path:
        sys.path.insert(0, _p)
import winmem as _wm  # read-only
import fastscan as fs  # discover_arena / enumerate_regions

EXPECTED_SHA = "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120"

# anchor (region_id, temp_C, is_rain)
# NOTE: region ids derived from the AUTHORITATIVE mod strategicregions files
# (mod/test/map/strategicregions/*.txt), NOT the stale weather_climatology.json
# (which mis-mapped Libya prov 7206 to region 225; the mod file places state-663
# Cyrenaica's provinces, incl. 7206, under strategic region 292).
ANCHORS = [
    (20,  8.0, False),   # France  Midi-Pyrenees (prov 3946)
    (31,  26.0, True),   # India   Deccan        (prov 7250)  RAIN
    (129, 11.0, False),  # Turkey  Afyon         (prov 6945)
    (160, 24.0, False),  # Philippines Luzon     (prov 10167)
    (292, 16.0, False),  # Libya   Cyrenaica     (prov 7206)  <- corrected (was 225)
]
RIDS = np.array([a[0] for a in ANCHORS], dtype=np.int64)
TEMPS = np.array([a[1] for a in ANCHORS], dtype=np.float64)
RAIN_IDX = 1  # index into ANCHORS that is the RAIN one (region 31)

_CHUNK = 16 << 20
_K = 273.15


# --------------------------------------------------------------------------
# encodings: each is (name, dtype, item_bytes, to_celsius(arr)->float array)
# We read the raw values at the anchor relative byte offsets, then convert to C.
# --------------------------------------------------------------------------
def _enc_f32_c(a):   return a.astype(np.float64)
def _enc_f32_k(a):   return a.astype(np.float64) - _K
def _enc_f64_c(a):   return a.astype(np.float64)
def _enc_f64_k(a):   return a.astype(np.float64) - _K
def _enc_f16_c(a):   return a.astype(np.float64)
def _enc_i32_raw(a): return a.astype(np.float64)
def _enc_i16_raw(a): return a.astype(np.float64)
def _enc_i32_e1(a):  return a.astype(np.float64) / 10.0
def _enc_i32_e2(a):  return a.astype(np.float64) / 100.0
def _enc_i32_e3(a):  return a.astype(np.float64) / 1000.0
def _enc_i32_e5(a):  return a.astype(np.float64) / 100000.0
def _enc_i16_e1(a):  return a.astype(np.float64) / 10.0
def _enc_i16_e2(a):  return a.astype(np.float64) / 100.0

# (name, numpy dtype to view the raw field, bytes per element, converter)
ENCODINGS = [
    ("f32_celsius", "<f4", 4, _enc_f32_c),
    ("f32_kelvin",  "<f4", 4, _enc_f32_k),
    ("f64_celsius", "<f8", 8, _enc_f64_c),
    ("f64_kelvin",  "<f8", 8, _enc_f64_k),
    ("f16_celsius", "<f2", 2, _enc_f16_c),
    ("i32_raw",     "<i4", 4, _enc_i32_raw),
    ("i16_raw",     "<i2", 2, _enc_i16_raw),
    ("i32_x10",     "<i4", 4, _enc_i32_e1),
    ("i32_x100",    "<i4", 4, _enc_i32_e2),
    ("i32_x1000",   "<i4", 4, _enc_i32_e3),
    ("i32_x100000", "<i4", 4, _enc_i32_e5),
    ("i16_x10",     "<i2", 2, _enc_i16_e1),
    ("i16_x100",    "<i2", 2, _enc_i16_e2),
]


# --------------------------------------------------------------------------
# read-only RPM reader (thread-local handle)
# --------------------------------------------------------------------------
class _Reader(object):
    def __init__(self, pid):
        self.h = _wm._k32.OpenProcess(
            _wm.PROCESS_VM_READ | _wm.PROCESS_QUERY_INFORMATION, False, pid)
        if not self.h:
            raise OSError("OpenProcess(%d) failed err %d" % (pid, C.get_last_error()))

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
# core: scan one blob for an array whose 5 anchor slots match, given an
# encoding + stride (bytes between consecutive region elements) + ib + field off.
#
# For a flat array, stride == element_bytes and field_off == 0. For an AoS,
# stride is the record size and field_off is the temp field's byte offset.
#
# Candidate "array base byte offset" O within the blob means region (ib) sits at
# blob[O]; region rid sits at blob[O + (rid-ib)*stride + field_off]. We require
# every anchor's slot to be inside the blob and to match within tol.
#
# Vectorized: O ranges over all valid byte positions. For each anchor we form the
# value array v_k = view(blob[ O + (rid_k-ib)*stride + field_off ]) over all O,
# convert to C, and AND the tolerance mask. Streaming AND over the 5 anchors.
# --------------------------------------------------------------------------
def _scan_blob_layout(blob, base, dt, ebytes, conv, stride, field_off, ib, tol,
                      hits, max_hits=200):
    n = len(blob)
    u8 = np.frombuffer(blob, np.uint8)
    rel = (RIDS - ib) * stride + field_off  # byte offset of each anchor from O
    rel_min = int(rel.min())
    rel_max = int(rel.max())
    # O must satisfy: O + rel_k in [0, n - ebytes] for all k
    O_lo = max(0, -rel_min)
    O_hi = n - ebytes - rel_max
    if O_hi < O_lo:
        return
    # We want O on element alignment? Not necessarily — arrays can sit at any
    # address. But to bound cost we step O by 1 byte only if ebytes<=2; for 4/8
    # we still must try all byte offsets because the array base is unknown. To
    # keep memory bounded we process O in batches.
    nO = O_hi - O_lo + 1
    BATCH = 1 << 20  # 1M offsets per batch -> 5 float64 arrays = ~40MB, safe
    for start in range(0, nO, BATCH):
        cnt = min(BATCH, nO - start)
        O = np.arange(O_lo + start, O_lo + start + cnt, dtype=np.int64)
        mask = None
        for k in range(len(RIDS)):
            off = O + int(rel[k])  # byte position of anchor k's element for each O
            # gather ebytes-wide field at each off, view as dt
            # build a (cnt, ebytes) byte matrix then view
            cols = u8[off[:, None] + np.arange(ebytes)[None, :]]
            vals = cols.copy().view(dt).reshape(cnt)
            c = conv(vals)
            m = np.abs(c - TEMPS[k]) <= tol
            mask = m if mask is None else (mask & m)
            if not mask.any():
                break
        if mask is None or not mask.any():
            continue
        for o in O[mask].tolist():
            hits.append((base + o, stride, field_off, ib))
            if len(hits) >= max_hits:
                return


# A faster path for the common case (flat contiguous array, stride==ebytes,
# field_off==0, ib in {0,1}) that avoids the byte-matrix gather by viewing the
# whole blob as the dtype on each of the (small) alignment phases.
def _scan_blob_flat(blob, base, dt, ebytes, conv, ib, tol, hits, max_hits=200):
    n = len(blob)
    rel = (RIDS - ib)  # element index of each anchor relative to array start (in elements)
    # For a flat array element stride == ebytes. The array can start at ANY byte
    # offset, so we sweep `phase` 0..ebytes-1 and within each phase view aligned
    # elements, then the candidate array-start element index e means anchor k is
    # at element e + rel[k].
    relmax = int(rel.max())
    relmin = int(rel.min())
    for phase in range(ebytes):
        m = (n - phase) // ebytes
        if m <= relmax:
            continue
        arr = np.frombuffer(blob[phase:phase + m * ebytes], dt)  # m elements
        cv = conv(arr.astype(np.float64) if dt[1] in 'iu' else arr)
        # candidate array-start element index e: need e+relmin>=0 and e+relmax<m
        e_lo = max(0, -relmin)
        e_hi = m - 1 - relmax
        if e_hi < e_lo:
            continue
        ne = e_hi - e_lo + 1
        e = np.arange(e_lo, e_lo + ne, dtype=np.int64)
        mask = None
        for k in range(len(RIDS)):
            vk = cv[e + int(rel[k])]
            mk = np.abs(vk - TEMPS[k]) <= tol
            mask = mk if mask is None else (mask & mk)
            if not mask.any():
                break
        if mask is None or not mask.any():
            continue
        # element index e -> byte addr = base + phase + e*ebytes (this is element ib's pos)
        for ev in e[mask].tolist():
            arr_start_byte = base + phase + ev * ebytes  # where element 'ib' lives
            hits.append((arr_start_byte, ebytes, 0, ib))
            if len(hits) >= max_hits:
                return


# --------------------------------------------------------------------------
# region worker
# --------------------------------------------------------------------------
def _scan_region(rd, base, size, encodings, strides, tol, flat_only, hits, lock,
                 max_hits):
    relmax_global = int((RIDS.max()) * max(strides) + max(s for s in strides) + 64)
    overlap = relmax_global + 64
    pos = 0
    local = []
    while pos < size:
        want = min(_CHUNK, size - pos)
        blob = rd.rpm(base + pos, want)
        got = len(blob)
        if got >= 4096:
            b = base + pos
            for (name, dt, ebytes, conv) in encodings:
                if flat_only:
                    for ib in (0, 1):
                        tmp = []
                        _scan_blob_flat(blob, b, dt, ebytes, conv, ib, tol, tmp, max_hits)
                        for h in tmp:
                            local.append((name,) + h)
                else:
                    for S in strides:
                        if S < ebytes:
                            continue
                        # field offsets within the stride (step 4 keeps it bounded)
                        foffs = range(0, S - ebytes + 1, 4) if S > ebytes else (0,)
                        for fo in foffs:
                            for ib in (0, 1):
                                tmp = []
                                _scan_blob_layout(blob, b, dt, ebytes, conv, S, fo,
                                                  ib, tol, tmp, max_hits)
                                for h in tmp:
                                    local.append((name,) + h)
        if got >= want:
            pos += max(1, got - overlap)
        else:
            pos += max(1, got)
        if len(local) >= max_hits:
            break
    if local:
        with lock:
            hits.extend(local)


def scan(pid, arena, tol=1.5, flat_only=True, strides=(8, 16, 24, 32),
         workers=6, max_hits=500):
    """Threaded exact-anchor scan. Returns list of hit dicts."""
    lo, hi = arena
    h = _wm._k32.OpenProcess(
        _wm.PROCESS_VM_READ | _wm.PROCESS_QUERY_INFORMATION, False, pid)
    try:
        regions = fs.enumerate_regions(h, lo, hi)
    finally:
        _wm._k32.CloseHandle(h)
    # balance by bytes
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
                _scan_region(rd, rb, rs, ENCODINGS, strides, tol, flat_only,
                             hits, lock, max_hits)
                if len(hits) >= max_hits:
                    return
        finally:
            rd.close()

    threads = [threading.Thread(target=run, args=(b,)) for b in bins if b]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    out = []
    for h in hits:
        out.append({"encoding": h[0], "addr": h[1], "stride": h[2],
                    "field_off": h[3], "ibase": h[4]})
    return out


if __name__ == "__main__":
    import argparse
    ap = argparse.ArgumentParser()
    ap.add_argument("--tol", type=float, default=1.5)
    ap.add_argument("--strided", action="store_true", help="also sweep AoS strides")
    ap.add_argument("--workers", type=int, default=6)
    ap.add_argument("--max-hits", type=int, default=500)
    args = ap.parse_args()
    pid = _wm.find_pid("hoi4.exe")
    if pid is None:
        raise SystemExit("hoi4.exe not running")
    p = _wm.Process(pid, "hoi4.exe")
    if p.fingerprint().get("sha256") != EXPECTED_SHA:
        print("WARN build sha mismatch")
    p.close()
    arena = fs.discover_arena(pid)
    print("arena 0x%X..0x%X (%.2f GB)" % (arena[0], arena[1],
                                          (arena[1] - arena[0]) / 2**30))
    t0 = time.time()
    hits = scan(pid, arena, tol=args.tol, flat_only=not args.strided,
                workers=args.workers, max_hits=args.max_hits)
    print("scan %.1fs -> %d hits" % (time.time() - t0, len(hits)))
    from collections import Counter
    enc = Counter(h["encoding"] for h in hits)
    print("by encoding:", dict(enc))
    for h in hits[:40]:
        print("  0x%X enc=%s stride=%d foff=%d ib=%d"
              % (h["addr"], h["encoding"], h["stride"], h["field_off"], h["ibase"]))
