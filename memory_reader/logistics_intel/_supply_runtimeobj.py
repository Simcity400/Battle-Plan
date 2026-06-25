"""_supply_runtimeobj.py - Pin STATE-SUPPLY as a field on the per-state RUNTIME object.

READ-ONLY research scaffold (logistics_intel). Not committed gameplay code.

The per-state RUNTIME object is already fully resolved (owner_offsets / logistics draft):
  vtable     = MODULE + 0x2910920
  state_id   @ +0x58  (u32)
  state_def  @ +0x30  (ptr to static CState)
  owner      @ +0xC8  (u32 tag-db-id)
  controller @ +0xCC  (u32 tag-db-id)

The state-supply tooltip value is a STATE-level quantity, so it is almost certainly a
field on THIS object (AoS-in-object), NOT a flat state-id-indexed array. The prior supply
work only searched the per-PROVINCE CProvince path; the per-state object was never checked.

METHOD (partition test, the same that pinned owner/controller):
  1. Enumerate every state runtime object (vtable scan, validate state_id@+0x58 in 1..2000).
  2. Build {state_id: object_addr}. Confirm our 5 anchor states are present.
  3. For EVERY byte offset in the object body [0x08 .. SCAN_END), and EVERY encoding
     (f32, f64, fx100000, fx1000, fx100, u32x100), test whether all 5 anchor states decode
     to their anchor state_supply within TOL. Offsets passing all 5 are candidates.
  4. For each candidate offset, TIGHTEN: re-check at ±0.01, and look for the sub-component
     fields (base/infra/pop/vp) at nearby offsets summing to the total.
  5. Validate against WITHHELD states (none available as anchors, so instead: confirm the
     candidate offset gives plausible, in-range, non-degenerate values across ALL ~1080
     states, and FAILS under wrong encoding / index±4).

Tolerances per Codex: UI rounds to 2 decimals so the raw value is ~±0.005; we scan at a
loose TOL to catch fixed-point rounding, then REPORT the actual residual so a loose-tol
alias can be told from a true ±0.005 hit.
"""
import json
import os
import struct
import sys

_HERE = os.path.dirname(os.path.abspath(__file__))
_MR = os.path.dirname(_HERE)
for _p in (_MR, _HERE):
    if _p not in sys.path:
        sys.path.insert(0, _p)
import winmem            # noqa: E402
import fastscan as fs    # noqa: E402

ARENA = None  # discovered live

VT_OFF = 0x2910920        # state runtime object vtable (module-relative)
OFF_STATE_ID = 0x58
OFF_OWNER = 0xC8
OFF_CONTROLLER = 0xCC
OFF_STATE_DEF = 0x30      # ptr to static CState definition

# state_id -> state_supply (PRIMARY anchor)
ANCHORS = {42: 2.80, 53: 2.90, 157: 1.40, 219: 6.60, 237: 0.60}
# sub-components (base/infra/pop/vp) for corroboration
COMPONENTS = {
    42:  {"base": 0.0, "infra": 1.20, "pop": 0.54, "vp": 1.05},
    53:  {"base": 1.40, "infra": 0.90, "pop": 0.26, "vp": 0.35},
    157: {"base": 0.0, "infra": 0.90, "pop": 0.53, "vp": 0.25},
    219: {"base": 1.20, "infra": 1.20, "pop": 1.50, "vp": 2.70},
    237: {"base": 0.0, "infra": 0.30, "pop": 0.03, "vp": 0.25},
}
ANCHOR_IDS = sorted(ANCHORS)

TOL = 0.055               # loose catch; residual reported so true ~0.005 hits stand out
SCAN_END = 0x1800         # object-body scan window (owner at 0xC8 -> object is large)
HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000


def enumerate_state_objects(p):
    """Vtable-scan the arena for state runtime objects; return {state_id: addr}.
    Validates state_id@+0x58 in a sane range (1..2000) and uniqueness."""
    vt = (p.module_base + VT_OFF).to_bytes(8, "little")
    out = {}
    dupes = 0
    for base, size in p.iter_regions(ARENA[0], ARENA[1], want_exec=False):
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
                sid = p.u32(cand + OFF_STATE_ID)
            except OSError:
                continue
            if sid is not None and 1 <= sid <= 2000:
                if sid in out and out[sid] != cand:
                    dupes += 1
                out.setdefault(sid, cand)
    return out, dupes


def _finite(v):
    """Reject NaN / inf (NaN comparisons are always False -> false positives)."""
    if v is None:
        return None
    if v != v or v in (float("inf"), float("-inf")):
        return None
    return v


def decode(blob, off, enc):
    try:
        if enc == "f32":
            return _finite(struct.unpack_from("<f", blob, off)[0])
        if enc == "f64":
            return _finite(struct.unpack_from("<d", blob, off)[0])
        if enc == "fx100000":
            return struct.unpack_from("<i", blob, off)[0] / 100000.0
        if enc == "fx1000":
            return struct.unpack_from("<i", blob, off)[0] / 1000.0
        if enc == "fx100":
            return struct.unpack_from("<i", blob, off)[0] / 100.0
        if enc == "u32x100":
            return struct.unpack_from("<I", blob, off)[0] / 100.0
    except struct.error:
        return None
    return None


ENCS = ["f32", "f64", "fx100000", "fx1000", "fx100", "u32x100"]


def find_supply_offset(p, objs):
    """For every (offset, encoding), test all 5 anchors. Return candidate hits."""
    # read the full body window for each anchor state once
    bodies = {}
    for sid in ANCHOR_IDS:
        bodies[sid] = p.try_read(objs[sid], SCAN_END + 8)
    cands = []
    for off in range(0x08, SCAN_END, 4):       # 4-byte aligned offsets
        for enc in ENCS:
            isz = 8 if enc == "f64" else 4
            if off + isz > SCAN_END:
                continue
            ok = True
            resid = 0.0
            vals = {}
            for sid in ANCHOR_IDS:
                v = decode(bodies[sid], off, enc)
                if v is None or abs(v - ANCHORS[sid]) > TOL:
                    ok = False
                    break
                resid = max(resid, abs(v - ANCHORS[sid]))
                vals[sid] = v
            if ok:
                cands.append({"offset": off, "enc": enc,
                              "max_resid": round(resid, 5), "vals": vals})
    return cands, bodies


def find_component_offsets(p, objs):
    """Independently search for offsets holding each sub-component (base/infra/pop/vp)
    across all 5 states - corroboration that this object carries the supply breakdown."""
    bodies = {sid: p.try_read(objs[sid], SCAN_END + 8) for sid in ANCHOR_IDS}
    found = {}
    for comp in ("base", "infra", "pop", "vp"):
        want = {sid: COMPONENTS[sid][comp] for sid in ANCHOR_IDS}
        hits = []
        for off in range(0x08, SCAN_END, 4):
            for enc in ENCS:
                isz = 8 if enc == "f64" else 4
                if off + isz > SCAN_END:
                    continue
                ok = True
                resid = 0.0
                for sid in ANCHOR_IDS:
                    v = decode(bodies[sid], off, enc)
                    if v is None or abs(v - want[sid]) > TOL:
                        ok = False
                        break
                    resid = max(resid, abs(v - want[sid]))
                if ok:
                    hits.append({"offset": off, "enc": enc,
                                 "max_resid": round(resid, 5)})
        found[comp] = hits
    return found


def find_supply_deref(p, objs, max_ptr_off=0x900, sub_window=0x400):
    """For each POINTER field on the object (every 8-aligned slot whose value is a
    heap address), deref it and scan the sub-record [0..sub_window) for an offset
    holding the anchor state_supply across all 5 states. Catches manager-owned
    sub-records reached via a single pointer (the documented supply-record shape)."""
    # collect pointer fields that are consistently heap pointers across all 5 states
    ptr_offs = []
    bodies = {sid: p.try_read(objs[sid], max_ptr_off + 8) for sid in ANCHOR_IDS}
    for poff in range(0x08, max_ptr_off, 8):
        good = True
        for sid in ANCHOR_IDS:
            try:
                v = struct.unpack_from("<Q", bodies[sid], poff)[0]
            except struct.error:
                good = False
                break
            if not (HEAP_LO <= v < HEAP_HI):
                good = False
                break
        if good:
            ptr_offs.append(poff)
    cands = []
    for poff in ptr_offs:
        # read each state's sub-record
        subs = {}
        ok_all = True
        for sid in ANCHOR_IDS:
            ptr = struct.unpack_from("<Q", bodies[sid], poff)[0]
            blob = p.try_read(ptr, sub_window + 8)
            if not blob or len(blob) < 16:
                ok_all = False
                break
            subs[sid] = blob
        if not ok_all:
            continue
        for off in range(0x00, sub_window, 4):
            for enc in ENCS:
                isz = 8 if enc == "f64" else 4
                if off + isz > sub_window:
                    continue
                ok = True
                resid = 0.0
                vals = {}
                for sid in ANCHOR_IDS:
                    v = decode(subs[sid], off, enc)
                    if v is None or abs(v - ANCHORS[sid]) > TOL:
                        ok = False
                        break
                    resid = max(resid, abs(v - ANCHORS[sid]))
                    vals[sid] = v
                if ok:
                    cands.append({"ptr_off": poff, "sub_off": off, "enc": enc,
                                  "max_resid": round(resid, 5),
                                  "vals": {k: round(v, 4) for k, v in vals.items()}})
    return cands, ptr_offs


def main():
    global ARENA
    p = winmem.open_process("hoi4.exe")
    if p is None:
        raise SystemExit("hoi4.exe not running")
    ARENA = fs.discover_arena(p.pid)
    print("pid %d arena 0x%X..0x%X (%.2f GB) build=%s" %
          (p.pid, ARENA[0], ARENA[1], (ARENA[1] - ARENA[0]) / float(1 << 30),
           p.fingerprint().get("sha256", "?")[:12]))

    objs, dupes = enumerate_state_objects(p)
    print("state runtime objects: %d  (dupes ignored: %d)" % (len(objs), dupes))
    missing = [s for s in ANCHOR_IDS if s not in objs]
    if missing:
        print("MISSING anchor states:", missing)
        # show what we got near them
        raise SystemExit("cannot proceed without all 5 anchor objects")
    for sid in ANCHOR_IDS:
        addr = objs[sid]
        owner = p.u32(addr + OFF_OWNER)
        print("  state %4d obj=0x%X owner_tagid=%d (sanity check state_id@+0x58=%d)" %
              (sid, addr, owner, p.u32(addr + OFF_STATE_ID)))

    cands, bodies = find_supply_offset(p, objs)
    print("\n=== STATE-SUPPLY candidate offsets (all 5 anchors within %.3f) ===" % TOL)
    for c in cands:
        print("  off=+0x%-4X enc=%-9s max_resid=%.5f vals=%s" %
              (c["offset"], c["enc"], c["max_resid"],
               {k: round(v, 4) for k, v in c["vals"].items()}))
    if not cands:
        print("  (none)")

    comps = find_component_offsets(p, objs)
    print("\n=== SUB-COMPONENT corroboration (independent offset search) ===")
    for comp, hits in comps.items():
        print("  %-6s: %d offset(s) %s" %
              (comp, len(hits),
               [(hex(h["offset"]), h["enc"], h["max_resid"]) for h in hits[:8]]))

    print("\n=== ONE-DEREF sub-record scan (pointer field -> sub-record) ===")
    deref_cands, ptr_offs = find_supply_deref(p, objs)
    print("  consistent pointer fields on object: %d" % len(ptr_offs))
    for c in deref_cands:
        print("  ptr_off=+0x%-4X sub_off=+0x%-4X enc=%-9s max_resid=%.5f vals=%s" %
              (c["ptr_off"], c["sub_off"], c["enc"], c["max_resid"], c["vals"]))
    if not deref_cands:
        print("  (none)")

    out = os.path.join(_HERE, "_runtimeobj_results.json")
    with open(out, "w") as f:
        json.dump({"pid": p.pid, "arena": list(ARENA),
                   "objs": {str(k): v for k, v in objs.items()},
                   "anchors": ANCHORS, "supply_candidates": cands,
                   "component_hits": comps, "deref_candidates": deref_cands,
                   "ptr_offs": ptr_offs}, f, indent=2)
    print("\nwrote", out)
    p.close()


if __name__ == "__main__":
    main()
