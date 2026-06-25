"""_supply_u16scan.py - final cheap test (Codex): u16/i16 display-hundredths array.
Scan for raw 16-bit display-hundredths {660,280,290,140,60}, state-id-indexed, same strides.
READ-ONLY. If zero, the stored-array hypothesis is fully closed."""
import os
import sys
import struct
from concurrent.futures import ThreadPoolExecutor
import numpy as np

_HERE = os.path.dirname(os.path.abspath(__file__))
_MR = os.path.dirname(_HERE)
for _p in (_MR, _HERE):
    if _p not in sys.path:
        sys.path.insert(0, _p)
import winmem            # noqa
import fastscan as fs    # noqa

# state_id -> display-hundredths integer
ANCH16 = {42: 280, 53: 290, 157: 140, 219: 660, 237: 60}
PIVOT = 219
STRIDES = [2, 4, 6, 8, 10, 12, 16, 20, 24, 32, 40, 48, 64, 96, 128, 192, 256]


def _m_u16(b, blob, value, signed):
    dt = "<i2" if signed else "<u2"
    phase = (-b) & 1
    n = (len(blob) - phase) & ~1
    if n <= 0:
        return []
    arr = np.frombuffer(blob, dtype=dt, count=n >> 1, offset=phase)
    idx = np.nonzero(arr == np.array(value, dtype=dt))[0]
    if idx.size == 0:
        return []
    return (b + phase + (idx.astype(np.uint64) << np.uint64(1))).tolist()


def thread_scan(pid, arena, value, signed, workers=3):
    lo, hi = arena
    h0 = fs._k32.OpenProcess(fs._PROCESS_VM_READ | fs._PROCESS_QUERY_INFORMATION,
                             False, pid)
    try:
        regions = fs.enumerate_regions(h0, lo, hi)
    finally:
        fs._k32.CloseHandle(h0)
    shards = fs.shard_regions(regions, workers)
    fn = lambda b, blob: _m_u16(b, blob, value, signed)   # noqa

    def run(shard):
        rd = fs._Reader(pid)
        try:
            out = []
            for base, size in shard:
                out.extend(rd.scan_region(base, size, fn, 2))
            return out
        finally:
            rd.close()
    hits = set()
    with ThreadPoolExecutor(max_workers=len(shards)) as ex:
        for r in ex.map(run, shards):
            hits.update(r)
    return sorted(hits)


def read16(p, addr, signed):
    try:
        return p.i16(addr) if signed else p.u16(addr)
    except OSError:
        return None


def main():
    p = winmem.open_process("hoi4.exe")
    arena = fs.discover_arena(p.pid)
    print("pid %d arena %.2f GB" % (p.pid, (arena[1] - arena[0]) / float(1 << 30)),
          flush=True)
    total = 0
    for signed in (False, True):
        A = thread_scan(p.pid, arena, ANCH16[PIVOT], signed, workers=3)
        print("%s pivot(660) hits=%d" % ("i16" if signed else "u16", len(A)), flush=True)
        for a in A:
            for S in STRIDES:
                base = a - PIVOT * S
                ok = True
                for sid, want in ANCH16.items():
                    v = read16(p, base + sid * S, signed)
                    if v != want:
                        ok = False
                        break
                if ok:
                    print("  CANDIDATE %s base=0x%X stride=%d" %
                          ("i16" if signed else "u16", base, S), flush=True)
                    total += 1
    print("\nTOTAL u16/i16 state-indexed candidates: %d" % total, flush=True)
    p.close()


if __name__ == "__main__":
    main()
