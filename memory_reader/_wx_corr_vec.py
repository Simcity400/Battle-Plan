"""TEMP vectorized snapshot-correlation weather pinner (NOT tracked; throwaway).

Pins the live ~304-element weather TEMPERATURE array on the PAUSED 1946.5.18 frame
by snapshot correlation vs the per-region May climatology signature.

Speed fix over _wx_corr_scan/_wx_corr_threads: the pearson correlation against the
fixed 304-wide signature is computed FULLY VECTORIZED across all candidate base
offsets O in a chunk, in memory-bounded BATCHES of <=BATCH offsets (the working
(BATCH x 304) f32 buffer is <= ~120 MB, well under the 1 GiB cap). No per-candidate
Python loop. Pre-filter: the first 8 region-columns must be finite floats in the
plausible temp band -> kills the vast majority of offsets before the dot products.

Streaming top-K only (K<=60). Never materializes [all_offsets x 304].
Thread pool (RPM + numpy release the GIL); mp 'spawn' is broken here (WinError 5
DuplicateHandle under the sandbox). READ-ONLY: PROCESS_VM_READ|QUERY only.
"""
import json, os, sys, time, threading, queue, heapq
import numpy as np

_HERE = os.path.dirname(os.path.abspath(__file__))
if _HERE not in sys.path:
    sys.path.insert(0, _HERE)
import winmem
import fastscan as fs

N = 304
MONTH0 = 4
TEMP_LO, TEMP_HI = -60.0, 55.0
BATCH = 100_000


def load_sig(month0):
    clim = json.load(open(os.path.join(_HERE, "weather", "weather_climatology.json"),
                          encoding="ascii"))
    tmin, tmax, mid = [], [], []
    for rid in range(1, N + 1):
        mo = clim[str(rid)]["months"][str(month0)]
        tmin.append(mo["tmin"]); tmax.append(mo["tmax"])
        mid.append((mo["tmin"] + mo["tmax"]) / 2.0)
    return (np.array(tmin, np.float32), np.array(tmax, np.float32),
            np.array(mid, np.float32))


TMIN, TMAX, MID = load_sig(MONTH0)
_, _, MIDN = load_sig(10)                      # November = wrong-month control
_RNG = np.random.default_rng(12345)
_PERM = np.arange(N); _RNG.shuffle(_PERM)
MID_SHUF = MID[_PERM]                           # shuffled-region control

MID_R = (MID - MID.mean()).astype(np.float32); MID_SS = float(MID_R @ MID_R)
MIDN_R = (MIDN - MIDN.mean()).astype(np.float32); MIDN_SS = float(MIDN_R @ MIDN_R)
MIDS_R = (MID_SHUF - MID_SHUF.mean()).astype(np.float32); MIDS_SS = float(MIDS_R @ MIDS_R)


def _batch_pearson(cols, ref_r, ref_ss):
    # cols: (n,304) f32 (already finite). returns (n,) pearson vs ref.
    m = cols.mean(axis=1, keepdims=True)
    cr = cols - m
    num = cr @ ref_r
    ss = np.einsum("ij,ij->i", cr, cr)
    den = np.sqrt(ss * ref_ss)
    out = np.zeros(num.shape, np.float32)
    nz = den > 0
    out[nz] = num[nz] / den[nz]
    return out


# Structural pre-filter band/mean/std for a REAL per-region temperature array.
# May climatology: range [-10,35.5], mean 15.9, std 8.6. Runtime temps stay near
# climatology -> require windowed mean in [-20,35] and std >= STD_MIN. This kills
# the dominant heap noise (all-zero blocks, uniform fills, monotone counters) in
# O(buffer) BEFORE any 304-wide correlation -- the zero-block saturation that
# stalled prior scans. A real temp array spans cold->hot regions => nonzero std.
STD_MIN = 3.0
MEAN_LO, MEAN_HI = -20.0, 35.0


def _window_gate(f32, rel0_max, M, Sf):
    # Build an "in-band" indicator on the REGION-COLUMN grid for stride Sf and
    # temp_off 0: region k sits every Sf slots. For the windowed stats we approximate
    # the 304-region window as the contiguous slice of the relevant slots. For Sf==1
    # this is exact (contiguous array). For Sf>1 we gate on the strided slots.
    # We compute over the strided pick to keep it cheap and correct per layout.
    pass


def scan_blob_vec(base, blob, Sf_list, ibases, hard_frac, pmin, topk_heap, lock):
    f32 = np.frombuffer(blob[: (len(blob) // 4) * 4], np.float32)
    nf = f32.size
    need = int(np.ceil(hard_frac * N))
    for Sf in Sf_list:
        for Tf in range(Sf):
            for ibase in ibases:
                rel = (np.arange(N) + ibase) * Sf + Tf
                span = int(rel.max()) + 1
                M = nf - span                      # number of candidate base idx O
                if M <= 0:
                    continue
                # ---- O(buffer) structural gate: windowed in-band count + mean + std
                # over the 304 strided region slots starting at each candidate O.
                # Strided gather of the slot grid: take f32 at positions Tf+ibase*Sf,
                # +Sf, +2Sf, ... then a sliding window of length N over THAT grid.
                g0 = Tf + ibase * Sf
                grid = f32[g0::Sf] if Sf > 1 else f32[g0:]
                ng = grid.size
                if ng < N:
                    continue
                ok = (np.isfinite(grid) & (grid >= TEMP_LO) & (grid <= TEMP_HI))
                okf = ok.astype(np.float64)
                gz = np.where(ok, grid, 0.0).astype(np.float64)
                cok = np.concatenate(([0.0], np.cumsum(okf)))
                c1 = np.concatenate(([0.0], np.cumsum(gz)))
                c2 = np.concatenate(([0.0], np.cumsum(gz * gz)))
                cntw = (cok[N:] - cok[:-N])
                sw = (c1[N:] - c1[:-N])
                ssw = (c2[N:] - c2[:-N])
                with np.errstate(invalid="ignore", divide="ignore"):
                    meanw = sw / N
                    varw = ssw / N - meanw * meanw
                    stdw = np.sqrt(np.clip(varw, 0, None))
                gridkeep = ((cntw >= need) & (meanw >= MEAN_LO) & (meanw <= MEAN_HI)
                            & (stdw >= STD_MIN))
                if not gridkeep.any():
                    continue
                # gridkeep index i -> candidate base float-index O = g0 + i*Sf
                gi = np.nonzero(gridkeep)[0]
                Oidx = g0 + gi * Sf
                Oidx = Oidx[Oidx < M]
                if Oidx.size == 0:
                    continue
                # Oidx are ABSOLUTE float-indices of each candidate's region-0 slot
                # (= g0 + gi*Sf). Region k sits at Oidx + k*Sf. Keep only candidates
                # whose full 304-region span fits in the buffer.
                Oidx = Oidx[Oidx + (N - 1) * Sf < nf]
                if Oidx.size == 0:
                    continue
                for bs in range(0, Oidx.size, BATCH):
                    Ob = Oidx[bs:bs + BATCH]
                    cols = np.empty((Ob.size, N), np.float32)
                    for k in range(N):
                        cols[:, k] = f32[Ob + k * Sf]
                    fin = np.isfinite(cols) & (cols >= TEMP_LO) & (cols <= TEMP_HI)
                    good = fin.sum(axis=1) >= need
                    if not good.any():
                        continue
                    cols = cols[good]; Og = Ob[good]
                    if not np.isfinite(cols).all():
                        bad = ~np.isfinite(cols).all(axis=1)
                        if bad.any():
                            keep2 = ~bad
                            cols = cols[keep2]; Og = Og[keep2]
                        if cols.shape[0] == 0:
                            continue
                    pr = _batch_pearson(cols, MID_R, MID_SS)
                    sel = pr >= pmin
                    if not sel.any():
                        continue
                    cols_s = cols[sel]; Og_s = Og[sel]; pr_s = pr[sel]
                    prN = _batch_pearson(cols_s, MIDN_R, MIDN_SS)
                    prS = _batch_pearson(cols_s, MIDS_R, MIDS_SS)
                    inb = ((cols_s >= TMIN - 6) & (cols_s <= TMAX + 6)).mean(axis=1)
                    for i in range(cols_s.shape[0]):
                        rec = (float(pr_s[i]),
                               {"addr": int(base + int(Og_s[i]) * 4),
                                "stride": Sf * 4, "temp_off": Tf * 4, "ibase": ibase,
                                "pearson": float(pr_s[i]),
                                "ctrl_wrongmonth": float(prN[i]),
                                "ctrl_shuffled": float(prS[i]),
                                "inband_frac": float(inb[i]),
                                "head": [round(float(x), 1) for x in cols_s[i, :8]]})
                        with lock:
                            if len(topk_heap) < 60:
                                heapq.heappush(topk_heap, rec)
                            elif rec[0] > topk_heap[0][0]:
                                heapq.heapreplace(topk_heap, rec)


def _loop(rd, q, Sf_list, ibases, hard_frac, pmin, chunk, heap, lock, maxSf):
    overlap = maxSf * 4 * (N + 4)
    while True:
        try:
            base, size = q.get_nowait()
        except queue.Empty:
            return
        pos = 0
        while pos < size:
            want = min(chunk, size - pos)
            blob = rd.rpm(base + pos, want)
            got = len(blob)
            if got >= 4 * (N + 4):
                scan_blob_vec(base + pos, blob, Sf_list, ibases, hard_frac,
                              pmin, heap, lock)
            if got >= want:
                pos += max(1, got - overlap)
            elif got:
                pos += max(1, got)
            else:
                pos += max(1, want)


def main():
    import argparse
    ap = argparse.ArgumentParser()
    ap.add_argument("--strides", default="4")       # byte strides
    ap.add_argument("--threads", type=int, default=10)
    ap.add_argument("--hard-frac", type=float, default=0.85)
    ap.add_argument("--pearson-min", type=float, default=0.55)
    ap.add_argument("--chunk", type=int, default=32 << 20)
    ap.add_argument("--arena-lo", default="0x24900000000")
    ap.add_argument("--arena-hi", default="0x24B00000000")
    ap.add_argument("--out", default=os.path.join(_HERE, "_wx_corr_hits.json"))
    args = ap.parse_args()
    sbytes = [int(s) for s in args.strides.split(",")]
    Sf_list = [s // 4 for s in sbytes if s % 4 == 0]
    arena_lo = int(args.arena_lo, 16); arena_hi = int(args.arena_hi, 16)

    pid = winmem.find_pid("hoi4.exe")
    if pid is None:
        raise SystemExit("hoi4.exe not running")
    p = winmem.Process(pid, "hoi4.exe")
    base = p.module_base
    sha = (p.fingerprint().get("sha256") or "")
    p.close()
    print("pid=%d base=0x%X sha=%s" % (pid, base, sha[:12]), flush=True)

    h = fs._k32.OpenProcess(fs._PROCESS_VM_READ | fs._PROCESS_QUERY_INFORMATION,
                            False, pid)
    regions = fs.enumerate_regions(h, arena_lo, arena_hi)
    fs._k32.CloseHandle(h)
    total = sum(s for _, s in regions)
    nthr = max(1, args.threads)
    print("regions=%d total=%.2fGB threads=%d Sf=%s monthsig=May(0based4)"
          % (len(regions), total / 1e9, nthr, Sf_list), flush=True)

    q = queue.Queue()
    for r in sorted(regions, key=lambda x: -x[1]):
        q.put(r)
    heap = []
    lock = threading.Lock()
    readers = [fs._Reader(pid) for _ in range(nthr)]
    maxSf = max(Sf_list)
    t0 = time.time()
    ths = [threading.Thread(target=_loop,
                            args=(readers[i], q, Sf_list, (0, 1), args.hard_frac,
                                  args.pearson_min, args.chunk, heap, lock, maxSf))
           for i in range(nthr)]
    for t in ths:
        t.start()
    for t in ths:
        t.join()
    for rd in readers:
        rd.close()
    dt = time.time() - t0

    hits = [rec for _, rec in sorted(heap, key=lambda x: -x[0])]
    json.dump({"base": base, "sha": sha, "month0": MONTH0, "n_hits": len(hits),
               "scan_s": dt, "strides": sbytes,
               "arena": [args.arena_lo, args.arena_hi], "hits": hits},
              open(args.out, "w"), indent=1)
    print("scan %.1fs hits(top%d)=%d -> %s" % (dt, len(hits), len(hits), args.out),
          flush=True)
    for d in hits[:20]:
        print("  0x%X S=%d Toff=%d ib=%d  P=%.3f | ctrlWM=%.3f ctrlSH=%.3f inband=%.2f head=%s"
              % (d["addr"], d["stride"], d["temp_off"], d["ibase"], d["pearson"],
                 d["ctrl_wrongmonth"], d["ctrl_shuffled"], d["inband_frac"],
                 d["head"]), flush=True)


if __name__ == "__main__":
    main()
