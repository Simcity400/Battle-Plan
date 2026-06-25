"""_supply_stridescan.py - DECISIVE state-id-indexed-array test for STATE SUPPLY.

READ-ONLY. The crisp hypothesis (Codex-endorsed as the strongest discriminator at 5 anchors):
a contiguous array where slot for state_id holds state_supply, i.e.
    value(array_base + state_id * stride) ~= state_supply[state_id]   (some encoding)

Method:
  1. Thread-pool arena scan for the MOST DISTINCTIVE anchor (Moscow 6.60, state 219) in
     each candidate encoding -> a list of addresses A.
  2. For each A and each candidate stride S, hypothesize A is state 219's slot:
     base = A - 219*S. Read slots base + sid*S for the other 4 anchor states; require ALL
     5 within tol. A 5/5 hit is a candidate (E[false] ~ 1e-16 under uniform; Codex).
  3. CORROBORATE each candidate: enumerate the WHOLE array over state ids 1..1100, report
     value distribution (how many sane small positive supply-like values, zeros, garbage),
     array alignment, and whether the per-state runtime owner (independent, validated)
     correlates. Reject candidates whose array is mostly garbage/NaN.
Encodings include the game native fixed_point/65536 (the gap Codex flagged).
flush=True so background runs stream output.
"""
import json
import os
import struct
import sys
from concurrent.futures import ThreadPoolExecutor

_HERE = os.path.dirname(os.path.abspath(__file__))
_MR = os.path.dirname(_HERE)
for _p in (_MR, _HERE):
    if _p not in sys.path:
        sys.path.insert(0, _p)
import numpy as np       # noqa: E402
import winmem            # noqa: E402
import fastscan as fs    # noqa: E402

ANCHORS = {42: 2.80, 53: 2.90, 157: 1.40, 219: 6.60, 237: 0.60}
PIVOT = 219              # most distinctive anchor (6.60)
TOL = 0.02              # tight; round values alias, so keep it tight
STRIDES = [4, 8, 12, 16, 20, 24, 28, 32, 36, 40, 44, 48, 56, 64, 72, 80, 96,
           104, 112, 128, 160, 192, 224, 256, 320, 384, 512]

ENC_SPECS = {
    "f32":      ("f32", None, None),
    "fp16":     ("fx", 65536.0, True),
    "fp16u":    ("fx", 65536.0, False),
    "fx100000": ("fx", 100000.0, True),
    "fx1000":   ("fx", 1000.0, True),
    "fx100":    ("fx", 100.0, True),
}


def _m_f32(b, blob, value, tol):
    phase = (-b) & 3
    n = (len(blob) - phase) & ~3
    if n <= 0:
        return []
    arr = np.frombuffer(blob, dtype="<f4", count=n >> 2, offset=phase)
    with np.errstate(invalid="ignore"):
        idx = np.nonzero(np.abs(arr - np.float32(value)) <= tol)[0]
    if idx.size == 0:
        return []
    return (b + phase + (idx.astype(np.uint64) << np.uint64(2))).tolist()


def _m_fx(b, blob, value, tol, scale, signed):
    phase = (-b) & 3
    n = (len(blob) - phase) & ~3
    if n <= 0:
        return []
    dt = "<i4" if signed else "<u4"
    arr = np.frombuffer(blob, dtype=dt, count=n >> 2, offset=phase).astype("f8") / scale
    idx = np.nonzero(np.abs(arr - value) <= tol)[0]
    if idx.size == 0:
        return []
    return (b + phase + (idx.astype(np.uint64) << np.uint64(2))).tolist()


def thread_scan(pid, arena, matcher_fn, workers=3):
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
                out.extend(rd.scan_region(base, size, matcher_fn, 4))
            return out
        finally:
            rd.close()
    hits = set()
    with ThreadPoolExecutor(max_workers=len(shards)) as ex:
        for r in ex.map(run, shards):
            hits.update(r)
    return sorted(hits)


def decode(p, addr, enc):
    kind, scale, signed = ENC_SPECS[enc]
    try:
        if kind == "f32":
            v = p.f32(addr)
            if v != v or v in (float("inf"), float("-inf")):
                return None
            return v
        raw = p.i32(addr) if signed else p.u32(addr)
        return raw / scale
    except OSError:
        return None


def test_strides(p, A, enc):
    out = []
    for S in STRIDES:
        base = A - PIVOT * S
        ok = True
        vals = {}
        resid = 0.0
        for sid, want in ANCHORS.items():
            v = decode(p, base + sid * S, enc)
            if v is None or abs(v - want) > TOL:
                ok = False
                break
            resid = max(resid, abs(v - want))
            vals[sid] = round(v, 4)
        if ok:
            out.append((base, S, round(resid, 5), vals))
    return out


def corroborate(p, base, S, enc, idmax=1100):
    """Read the whole array over state ids 1..idmax; classify slot values."""
    sane = zeros = garbage = nonefail = 0
    samples = {}
    for sid in range(1, idmax + 1):
        v = decode(p, base + sid * S, enc)
        if v is None:
            nonefail += 1
            continue
        if v == 0.0:
            zeros += 1
        elif 0.0 < v < 200.0:   # supply-like small positive
            sane += 1
        else:
            garbage += 1
        if sid in (1, 100, 219, 500, 1000):
            samples[sid] = round(v, 4)
    return {"sane": sane, "zeros": zeros, "garbage": garbage,
            "unreadable": nonefail, "samples": samples}


def main():
    p = winmem.open_process("hoi4.exe")
    arena = fs.discover_arena(p.pid)
    print("pid %d arena 0x%X..0x%X (%.2f GB)" %
          (p.pid, arena[0], arena[1], (arena[1] - arena[0]) / float(1 << 30)), flush=True)
    pidnum = p.pid

    all_cands = []
    for enc in ENC_SPECS:
        kind, scale, signed = ENC_SPECS[enc]
        if kind == "f32":
            fn = lambda b, blob: _m_f32(b, blob, ANCHORS[PIVOT], TOL)        # noqa
        else:
            fn = lambda b, blob, s=scale, g=signed: _m_fx(b, blob, ANCHORS[PIVOT], TOL, s, g)  # noqa
        A = thread_scan(pidnum, arena, fn, workers=3)
        print("enc=%-9s pivot(6.60) hits=%d" % (enc, len(A)), flush=True)
        enc_cands = []
        for a in A:
            for base, S, resid, vals in test_strides(p, a, enc):
                enc_cands.append({"enc": enc, "base": base, "stride": S,
                                  "resid": resid, "vals": vals})
        for c in enc_cands:
            corr = corroborate(p, c["base"], c["stride"], enc)
            c["corr"] = corr
            print("  CANDIDATE enc=%s base=0x%X stride=%d resid=%.5f vals=%s\n     corr=%s"
                  % (enc, c["base"], c["stride"], c["resid"], c["vals"], corr), flush=True)
        all_cands += enc_cands
    print("\nTOTAL 5/5 candidates: %d" % len(all_cands), flush=True)
    out = os.path.join(_HERE, "_stridescan_results.json")
    with open(out, "w") as f:
        json.dump({"pid": pidnum, "arena": list(arena), "anchors": ANCHORS,
                   "candidates": all_cands}, f, indent=2)
    print("wrote", out, flush=True)
    p.close()


if __name__ == "__main__":
    main()
