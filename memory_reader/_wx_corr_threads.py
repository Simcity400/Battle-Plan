"""TEMP thread-parallel snapshot-correlation weather pinner (NOT tracked; throwaway).

Same correlation logic as _wx_corr_scan.scan_blob, but uses a THREAD pool instead
of multiprocessing. This environment's mp 'spawn' path dies with WinError 5 on
DuplicateHandle (sandboxed handle-dup policy). RPM (ReadProcessMemory) and the
numpy correlation both release the GIL, so threads give real parallelism here and
need no handle duplication. READ-ONLY: PROCESS_VM_READ|PROCESS_QUERY_INFORMATION.

Paused frame 1946.5.18 -> May -> 0-based climatology month 4. Static frame =>
pin by snapshot correlation (rank+magnitude of a contiguous ~304-elem temp array
vs the per-region May band signature), not drift.
"""
import json, os, sys, time, threading, queue
import numpy as np

_HERE = os.path.dirname(os.path.abspath(__file__))
if _HERE not in sys.path:
    sys.path.insert(0, _HERE)
import winmem
import fastscan as fs
import _wx_corr_scan as cs   # reuse scan_blob + signature (same MONTH0=4)


def _reader_loop(rd, region_q, strides, ibases, hard_frac, pearson_min, chunk,
                 out_list, lock):
    overlap = max(strides) * (cs.N + 4)
    local = []
    while True:
        try:
            base, size = region_q.get_nowait()
        except queue.Empty:
            break
        pos = 0
        while pos < size:
            want = min(chunk, size - pos)
            blob = rd.rpm(base + pos, want)
            got = len(blob)
            if got >= 4 * (cs.N + 4):
                local.extend(cs.scan_blob(base + pos, blob, strides, ibases,
                                          hard_frac, pearson_min))
            if got >= want:
                pos += max(1, got - overlap)
            elif got:
                pos += max(1, got)
            else:
                pos += max(1, want)
        region_q.task_done()
    with lock:
        out_list.extend(local)


def main():
    import argparse
    ap = argparse.ArgumentParser()
    ap.add_argument("--strides", default="4")
    ap.add_argument("--threads", type=int, default=8)
    ap.add_argument("--hard-frac", type=float, default=0.85)
    ap.add_argument("--pearson-min", type=float, default=0.50)
    ap.add_argument("--chunk", type=int, default=32 << 20)
    ap.add_argument("--arena-lo", default="0x24900000000")
    ap.add_argument("--arena-hi", default="0x24B00000000")
    ap.add_argument("--out", default=os.path.join(_HERE, "_wx_corr_hits.json"))
    args = ap.parse_args()
    strides = tuple(int(s) for s in args.strides.split(","))
    arena_lo = int(args.arena_lo, 16)
    arena_hi = int(args.arena_hi, 16)

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
    print("regions=%d total=%.2fGB threads=%d strides=%s monthsig=May(0based4)"
          % (len(regions), total / 1e9, nthr, strides), flush=True)

    # one Reader (own handle) per thread
    region_q = queue.Queue()
    for r in sorted(regions, key=lambda x: -x[1]):   # big regions first
        region_q.put(r)
    out_list = []
    lock = threading.Lock()
    readers = [fs._Reader(pid) for _ in range(nthr)]
    t0 = time.time()
    threads = [threading.Thread(target=_reader_loop,
                                args=(readers[i], region_q, strides, (0, 1),
                                      args.hard_frac, args.pearson_min,
                                      args.chunk, out_list, lock))
               for i in range(nthr)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    for rd in readers:
        rd.close()
    dt = time.time() - t0

    bykey = {}
    for d in out_list:
        k = (d["addr"], d["stride"], d["temp_off"], d["ibase"])
        if k not in bykey or d["pearson"] > bykey[k]["pearson"]:
            bykey[k] = d
    hits = sorted(bykey.values(), key=lambda d: -d["pearson"])
    json.dump({"base": base, "sha": sha, "month0": cs.MONTH0, "n_hits": len(hits),
               "scan_s": dt, "strides": strides, "arena": [args.arena_lo, args.arena_hi],
               "hits": hits[:200]},
              open(args.out, "w"), indent=1)
    print("scan %.1fs hits=%d (saved top200 -> %s)" % (dt, len(hits), args.out),
          flush=True)
    for d in hits[:20]:
        print("  0x%X S=%d Toff=%d ib=%d  P=%.3f Sp=%.3f | ctrlWM=%.3f ctrlSH=%.3f inband=%.2f  head=%s"
              % (d["addr"], d["stride"], d["temp_off"], d["ibase"], d["pearson"],
                 d["spearman"], d["ctrl_wrongmonth"], d["ctrl_shuffled"],
                 d["inband_frac"], d["head"]), flush=True)


if __name__ == "__main__":
    main()
