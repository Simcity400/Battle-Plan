"""TEMP snapshot-correlation weather-array pinner (NOT tracked; throwaway).

Paused frame 1946.5.18 (May -> 0-based climatology month 4). Weather is STATIC
this frame, so we pin by SNAPSHOT CORRELATION (rank+magnitude of a contiguous
~304-element temperature array vs the per-region May band signature), not drift.

Reuses fastscan.enumerate_regions + shard_regions + _Reader (region enumeration,
balanced sharding, clamped/overlapped chunked reads), but each worker returns
RICH correlation dicts (not bare addresses), so we keep pearson/spearman/control
scores. READ-ONLY: PROCESS_VM_READ | PROCESS_QUERY_INFORMATION only.
"""
import json, os, sys, time
import numpy as np

_HERE = os.path.dirname(os.path.abspath(__file__))
if _HERE not in sys.path:
    sys.path.insert(0, _HERE)
import winmem
import fastscan as fs

N = 304
MONTH0 = 4                       # May, 0-based
TEMP_LO, TEMP_HI = -60.0, 55.0   # global plausible runtime band


def load_sig(month0):
    clim = json.load(open(os.path.join(_HERE, "weather", "weather_climatology.json"),
                          encoding="ascii"))
    rids, tmin, tmax, mid = [], [], [], []
    for rid in range(1, N + 1):
        mo = clim[str(rid)]["months"][str(month0)]
        rids.append(rid)
        tmin.append(mo["tmin"]); tmax.append(mo["tmax"])
        mid.append((mo["tmin"] + mo["tmax"]) / 2.0)
    return (np.array(rids), np.array(tmin, np.float64),
            np.array(tmax, np.float64), np.array(mid, np.float64))


def _rankdata(a):
    order = np.argsort(a, kind="mergesort")
    ranks = np.empty(len(a), dtype=np.float64)
    ranks[order] = np.arange(len(a), dtype=np.float64)
    a_sorted = a[order]
    n = len(a); i = 0
    while i < n:
        j = i
        while j + 1 < n and a_sorted[j + 1] == a_sorted[i]:
            j += 1
        if j > i:
            ranks[order[i:j + 1]] = (i + j) / 2.0
        i = j + 1
    return ranks


# module-level signature (computed once per process, incl. spawned workers)
RIDS, TMIN, TMAX, MID = load_sig(MONTH0)
_, TMINW, TMAXW, MIDW = load_sig(10)        # November = wrong-month control
MID_R = MID - MID.mean(); MID_SS = float(np.dot(MID_R, MID_R))
MIDW_R = MIDW - MIDW.mean(); MIDW_SS = float(np.dot(MIDW_R, MIDW_R))
MID_RANK = _rankdata(MID); MID_RANK_R = MID_RANK - MID_RANK.mean()
MID_RANK_SS = float(np.dot(MID_RANK_R, MID_RANK_R))
_RNG = np.random.default_rng(12345)
SHUF = RIDS.copy(); _RNG.shuffle(SHUF)        # shuffled-rid control mapping
MID_SHUF = MID[SHUF - 1]; MID_SHUF_R = MID_SHUF - MID_SHUF.mean()
MID_SHUF_SS = float(np.dot(MID_SHUF_R, MID_SHUF_R))


def _corr(v_r, v_ss, ref_r, ref_ss):
    if v_ss <= 0 or ref_ss <= 0:
        return 0.0
    return float(np.dot(v_r, ref_r)) / np.sqrt(v_ss * ref_ss)


def score_vec(v):
    vr = v - v.mean(); vss = float(np.dot(vr, vr))
    pr = _corr(vr, vss, MID_R, MID_SS)
    prW = _corr(vr, vss, MIDW_R, MIDW_SS)
    prShuf = _corr(vr, vss, MID_SHUF_R, MID_SHUF_SS)
    rk = _rankdata(v); rkr = rk - rk.mean(); rkss = float(np.dot(rkr, rkr))
    sp = _corr(rkr, rkss, MID_RANK_R, MID_RANK_SS)
    inband = float(np.mean((v >= TMIN - 6) & (v <= TMAX + 6)))
    return pr, sp, prW, prShuf, inband


def scan_blob(base, blob, strides, ibases, hard_frac, pearson_min):
    out = []
    f32 = np.frombuffer(blob[: (len(blob) // 4) * 4], np.float32)
    nf = f32.size
    need = int(np.ceil(hard_frac * N))
    for Sb in strides:
        if Sb % 4:
            continue
        Sf = Sb // 4
        for Tf in range(Sf):
            for ibase in ibases:
                # ibase = record index at which region 1 sits (0: region1 at slot0;
                # 1: slot0 is a dummy/region-0 header, region1 at slot1).
                rel = (RIDS - 1 + ibase) * Sf + Tf
                if rel.min() < 0:
                    continue
                span = int(rel.max()) + 1
                rec0_max = nf - span
                if rec0_max <= 0:
                    continue
                m = np.ones(rec0_max, dtype=bool)
                for j in range(min(8, N)):
                    cj = f32[rel[j]: rel[j] + rec0_max]
                    m &= np.isfinite(cj) & (cj >= TEMP_LO) & (cj <= TEMP_HI)
                    if not m.any():
                        break
                if not m.any():
                    continue
                surv_all = np.nonzero(m)[0]
                # CRASH-PROOFING: never materialize a (surv x 304) matrix whole.
                # Process survivors in capped sub-batches (<=100k rows -> the
                # (batch,304) f64 buffer is <= ~240 MB, far under the 1 GiB cap).
                BATCH = 100_000
                for bstart in range(0, surv_all.size, BATCH):
                    surv = surv_all[bstart:bstart + BATCH]
                    cnt = np.zeros(surv.size, dtype=np.int32)
                    allvals = np.empty((surv.size, N), dtype=np.float64)
                    for k in range(N):
                        cj = f32[rel[k]: rel[k] + rec0_max][surv]
                        allvals[:, k] = cj
                        cnt += np.isfinite(cj) & (cj >= TEMP_LO) & (cj <= TEMP_HI)
                    keep = cnt >= need
                    pa = surv[keep]; va = allvals[keep]
                    for r, O in enumerate(pa.tolist()):
                        v = va[r]
                        if not np.all(np.isfinite(v)):
                            continue
                        pr, sp, prW, prShuf, inband = score_vec(v)
                        if pr < pearson_min:
                            continue
                        out.append({"addr": int(base + O * 4), "stride": Sb,
                                    "temp_off": Tf * 4, "ibase": ibase,
                                    "pearson": pr, "spearman": sp,
                                    "ctrl_wrongmonth": prW, "ctrl_shuffled": prShuf,
                                    "inband_frac": inband,
                                    "head": [round(float(x), 1) for x in v[:8]]})
    return out


def _worker(pid, shard, strides, ibases, hard_frac, pearson_min, chunk):
    rd = fs._Reader(pid)
    out = []
    overlap = max(strides) * (N + 4)
    try:
        for base, size in shard:
            pos = 0
            while pos < size:
                want = min(chunk, size - pos)
                blob = rd.rpm(base + pos, want)
                got = len(blob)
                if got >= 4 * (N + 4):
                    out.extend(scan_blob(base + pos, blob, strides, ibases,
                                         hard_frac, pearson_min))
                if got >= want:
                    pos += max(1, got - overlap)
                elif got:
                    pos += max(1, got)
                else:
                    pos += max(1, want)
    finally:
        rd.close()
    return out


def main():
    import argparse
    ap = argparse.ArgumentParser()
    ap.add_argument("--strides", default="4,8,12,16,20,24,28,32")
    ap.add_argument("--workers", type=int, default=None)
    ap.add_argument("--hard-frac", type=float, default=0.85)
    ap.add_argument("--pearson-min", type=float, default=0.50)
    ap.add_argument("--chunk", type=int, default=32 << 20)
    ap.add_argument("--arena-lo", default=None, help="hex, e.g. 0x24900000000")
    ap.add_argument("--arena-hi", default=None, help="hex, e.g. 0x24B00000000")
    ap.add_argument("--out", default=os.path.join(_HERE, "_wx_corr_hits.json"))
    args = ap.parse_args()
    strides = tuple(int(s) for s in args.strides.split(","))
    arena_lo = int(args.arena_lo, 16) if args.arena_lo else fs.ARENA_LO
    arena_hi = int(args.arena_hi, 16) if args.arena_hi else fs.ARENA_HI

    pid = winmem.find_pid("hoi4.exe")
    if pid is None:
        raise SystemExit("hoi4.exe not running")
    p = winmem.Process(pid, "hoi4.exe")
    base = p.module_base
    sha = (p.fingerprint().get("sha256") or "")
    p.close()
    print("pid=%d base=0x%X sha=%s" % (pid, base, sha[:12]))

    h = fs._k32.OpenProcess(fs._PROCESS_VM_READ | fs._PROCESS_QUERY_INFORMATION,
                            False, pid)
    regions = fs.enumerate_regions(h, arena_lo, arena_hi)
    fs._k32.CloseHandle(h)
    total = sum(s for _, s in regions)
    workers = args.workers or max(1, (os.cpu_count() or 2) - 2)
    print("regions=%d total=%.2fGB workers=%d strides=%s monthsig=May(0based4)"
          % (len(regions), total / 1e9, workers, strides))

    shards = fs.shard_regions(regions, workers)
    t0 = time.time()
    if len(shards) == 1:
        results = [_worker(pid, shards[0], strides, (0, 1), args.hard_frac,
                           args.pearson_min, args.chunk)]
    else:
        import multiprocessing as mp
        ctx = mp.get_context("spawn")
        a = [(pid, sh, strides, (0, 1), args.hard_frac, args.pearson_min, args.chunk)
             for sh in shards]
        with ctx.Pool(processes=len(shards)) as pool:
            results = pool.starmap(_worker, a)
    hits = [d for r in results for d in r]
    dt = time.time() - t0

    # dedup by addr+stride+temp_off, keep best pearson
    bykey = {}
    for d in hits:
        k = (d["addr"], d["stride"], d["temp_off"], d["ibase"])
        if k not in bykey or d["pearson"] > bykey[k]["pearson"]:
            bykey[k] = d
    hits = sorted(bykey.values(), key=lambda d: -d["pearson"])
    json.dump({"base": base, "sha": sha, "month0": MONTH0, "n_hits": len(hits),
               "scan_s": dt, "strides": strides, "hits": hits[:200]},
              open(args.out, "w"), indent=1)
    print("scan %.1fs hits=%d (saved top200 -> %s)" % (dt, len(hits), args.out))
    for d in hits[:15]:
        print("  0x%X S=%d Toff=%d ib=%d  P=%.3f Sp=%.3f | ctrlWM=%.3f ctrlSH=%.3f inband=%.2f  head=%s"
              % (d["addr"], d["stride"], d["temp_off"], d["ibase"], d["pearson"],
                 d["spearman"], d["ctrl_wrongmonth"], d["ctrl_shuffled"],
                 d["inband_frac"], d["head"]))


if __name__ == "__main__":
    main()
