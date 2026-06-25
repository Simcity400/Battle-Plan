"""_supply_neighborhood.py - FINAL pass (Codex-suggested): value-hit neighborhood filter.

READ-ONLY. The last storage shape not ruled out: a separate supply-manager record keyed
by state (dense index / state_id / CState*), NOT reachable by deref from the per-state
runtime object. We cannot vtable-scan an unknown vtable, so instead:

  1. Find all arena addresses holding the pivot anchor (Moscow state_supply=6.60).
  2. Around each such address (window +-0x800), look for:
       (a) the OTHER FOUR anchor values (2.80,2.90,1.40,0.60) - a co-located supply record;
       (b) the literal state_id 219 as a u32 nearby (record keyed by state_id);
       (c) a pointer to the KNOWN state-219 runtime object or its CState* (record keyed by
           CState*/runtime-object).
  3. Any window hitting >=3 other anchors, OR the state_id, OR a known-object pointer, is a
     real candidate supply record -> dump + report relative offsets.

This is the cheapest decisive test for the "separate keyed record" shape. If it finds
nothing, the computed-not-stored verdict is final.
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
PIVOT = 219
TOL = 0.02
WIN = 0x800
VT_OFF = 0x2910920
OFF_STATE_ID = 0x58
OFF_STATE_DEF = 0x30


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


def thread_scan(pid, arena, value, workers=3):
    lo, hi = arena
    h0 = fs._k32.OpenProcess(fs._PROCESS_VM_READ | fs._PROCESS_QUERY_INFORMATION,
                             False, pid)
    try:
        regions = fs.enumerate_regions(h0, lo, hi)
    finally:
        fs._k32.CloseHandle(h0)
    shards = fs.shard_regions(regions, workers)
    fn = lambda b, blob: _m_f32(b, blob, value, TOL)   # noqa

    def run(shard):
        rd = fs._Reader(pid)
        try:
            out = []
            for base, size in shard:
                out.extend(rd.scan_region(base, size, fn, 4))
            return out
        finally:
            rd.close()
    hits = set()
    with ThreadPoolExecutor(max_workers=len(shards)) as ex:
        for r in ex.map(run, shards):
            hits.update(r)
    return sorted(hits)


def main():
    p = winmem.open_process("hoi4.exe")
    arena = fs.discover_arena(p.pid)
    print("pid %d arena %.2f GB" % (p.pid, (arena[1] - arena[0]) / float(1 << 30)),
          flush=True)

    # known state-219 runtime object + its CState* (for the "keyed by object" test)
    vt = (p.module_base + VT_OFF).to_bytes(8, "little")
    obj219 = csdef219 = None
    for base, size in p.iter_regions(arena[0], arena[1], want_exec=False):
        blob = p.try_read(base, size)
        if not blob:
            continue
        s = 0
        while True:
            j = blob.find(vt, s)
            if j < 0:
                break
            s = j + 8
            cand = base + j
            try:
                if p.u32(cand + OFF_STATE_ID) == 219:
                    obj219 = cand
                    csdef219 = p.u64(cand + OFF_STATE_DEF)
                    break
            except OSError:
                pass
        if obj219:
            break
    print("state219 runtime obj=0x%X CState*=0x%X" %
          (obj219 or 0, csdef219 or 0), flush=True)

    pivots = thread_scan(p.pid, arena, ANCHORS[PIVOT], workers=3)
    print("pivot(6.60) hits=%d -> neighborhood filter (+-0x%X)" % (len(pivots), WIN),
          flush=True)

    others = {sid: v for sid, v in ANCHORS.items() if sid != PIVOT}
    obj219_b = struct.pack("<Q", obj219) if obj219 else None
    csdef_b = struct.pack("<Q", csdef219) if csdef219 else None
    sid_b = struct.pack("<I", 219)

    candidates = []
    for a in pivots:
        lo = a - WIN
        blob = p.try_read(lo, 2 * WIN)
        if not blob or len(blob) < 64:
            continue
        # (a) count other anchors present as f32 in window
        arrf = np.frombuffer(blob[:len(blob) & ~3], dtype="<f4")
        present = []
        for sid, val in others.items():
            with np.errstate(invalid="ignore"):
                if np.any(np.abs(arrf - np.float32(val)) <= TOL):
                    present.append(sid)
        has_sid = sid_b in blob
        has_obj = bool(obj219_b and obj219_b in blob)
        has_csdef = bool(csdef_b and csdef_b in blob)
        if len(present) >= 2 or has_sid or has_obj or has_csdef:
            candidates.append({
                "pivot_addr": hex(a), "others_present": present,
                "has_state_id_219": has_sid, "has_obj219_ptr": has_obj,
                "has_CState_ptr": has_csdef})
    # keep the strongest first
    candidates.sort(key=lambda c: (len(c["others_present"]) + c["has_state_id_219"] * 3
                                   + c["has_obj219_ptr"] * 5 + c["has_CState_ptr"] * 5),
                    reverse=True)
    print("neighborhood candidates: %d" % len(candidates), flush=True)
    for c in candidates[:25]:
        print("  %s others=%s sid=%s obj=%s cstate=%s" %
              (c["pivot_addr"], c["others_present"], c["has_state_id_219"],
               c["has_obj219_ptr"], c["has_CState_ptr"]), flush=True)
    # strongest: any with 4 others present (full record) or an object/CState pointer
    strong = [c for c in candidates if len(c["others_present"]) >= 4
              or c["has_obj219_ptr"] or c["has_CState_ptr"]]
    print("\nSTRONG (full-record or object-keyed): %d" % len(strong), flush=True)
    for c in strong[:25]:
        print("  ", c, flush=True)

    out = os.path.join(_HERE, "_neighborhood_results.json")
    with open(out, "w") as f:
        json.dump({"obj219": obj219, "csdef219": csdef219,
                   "n_pivot": len(pivots), "candidates": candidates[:200],
                   "strong": strong}, f, indent=2)
    print("wrote", out, flush=True)
    p.close()


if __name__ == "__main__":
    main()
