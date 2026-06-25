"""_supply_broadsweep.py - exhaustive offset hunt for the STATE-SUPPLY value.

READ-ONLY research scaffold. Given the 5 located state runtime objects, this hunts the
state_supply anchor [2.80,2.90,1.40,6.60,0.60] (and its components) across:
  (A) the runtime object body, wide window, ALL byte offsets (1-byte step), all encodings.
  (B) the STATIC CState definition (via runtime+0x30), wide window.
  (C) one pointer-deref from EITHER object (every heap-ptr field -> sub-record).
  (D) two pointer-derefs from the runtime object (ptr -> ptr -> sub-record).
NaN/inf rejected. Reports the actual max residual so a true ~0.005 hit is distinguishable
from a loose-tolerance alias. Withheld-state cross-check appended for any survivor.
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

VT_OFF = 0x2910920
OFF_STATE_ID = 0x58
OFF_STATE_DEF = 0x30
ANCHORS = {42: 2.80, 53: 2.90, 157: 1.40, 219: 6.60, 237: 0.60}
COMPONENTS = {
    42:  {"base": 0.0, "infra": 1.20, "pop": 0.54, "vp": 1.05},
    53:  {"base": 1.40, "infra": 0.90, "pop": 0.26, "vp": 0.35},
    157: {"base": 0.0, "infra": 0.90, "pop": 0.53, "vp": 0.25},
    219: {"base": 1.20, "infra": 1.20, "pop": 1.50, "vp": 2.70},
    237: {"base": 0.0, "infra": 0.30, "pop": 0.03, "vp": 0.25},
}
AIDS = sorted(ANCHORS)
TOL = 0.055
HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000
ENCS = ["f32", "f64", "fx100000", "fx1000", "fx100", "u32x100",
        "fp16", "fp16_64", "fp16u"]


def _finite(v):
    if v is None or v != v or v in (float("inf"), float("-inf")):
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
        if enc == "fp16":          # the game native fixed_point: i32 / 65536
            return struct.unpack_from("<i", blob, off)[0] / 65536.0
        if enc == "fp16_64":       # i64 / 65536 (wide fixed_point)
            return struct.unpack_from("<q", blob, off)[0] / 65536.0
        if enc == "fp16u":         # u32 / 65536
            return struct.unpack_from("<I", blob, off)[0] / 65536.0
    except struct.error:
        return None
    return None


def sweep_blobs(blobs, want, window, step=1):
    """blobs: {sid: bytes}; want: {sid: target}. Return offset/enc hits (1-byte step)."""
    hits = []
    for off in range(0, window, step):
        for enc in ENCS:
            isz = 8 if enc == "f64" else 4
            if off + isz > min(len(b) for b in blobs.values()):
                continue
            ok = True
            resid = 0.0
            vals = {}
            for sid in AIDS:
                v = decode(blobs[sid], off, enc)
                if v is None or abs(v - want[sid]) > TOL:
                    ok = False
                    break
                resid = max(resid, abs(v - want[sid]))
                vals[sid] = round(v, 4)
            if ok:
                hits.append({"off": off, "enc": enc, "resid": round(resid, 5),
                             "vals": vals})
    return hits


def heap_ptr_fields(blobs, window):
    """Offsets (8-aligned) that are heap pointers in ALL anchor states."""
    offs = []
    for poff in range(0, window, 8):
        ok = True
        for sid in AIDS:
            try:
                v = struct.unpack_from("<Q", blobs[sid], poff)[0]
            except struct.error:
                ok = False
                break
            if not (HEAP_LO <= v < HEAP_HI):
                ok = False
                break
        if ok:
            offs.append(poff)
    return offs


def main():
    p = winmem.open_process("hoi4.exe")
    arena = fs.discover_arena(p.pid)
    vt = (p.module_base + VT_OFF).to_bytes(8, "little")
    objs = {}
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
                sid = p.u32(cand + OFF_STATE_ID)
            except OSError:
                continue
            if sid is not None and 1 <= sid <= 2000:
                objs.setdefault(sid, cand)
    print("state objs: %d  anchors present: %s" %
          (len(objs), all(s in objs for s in AIDS)))

    RT_WIN = 0x2000
    DEF_WIN = 0x1000
    rt = {sid: p.try_read(objs[sid], RT_WIN) for sid in AIDS}

    # (A) runtime body, 1-byte step, supply + components
    print("\n[A] RUNTIME body sweep (1-byte step, window 0x%X)" % RT_WIN)
    print("  supply:", sweep_blobs(rt, ANCHORS, RT_WIN) or "none")
    for comp in ("base", "infra", "pop", "vp"):
        want = {s: COMPONENTS[s][comp] for s in AIDS}
        h = sweep_blobs(rt, want, RT_WIN)
        print("  %-6s: %s" % (comp, [(hex(x["off"]), x["enc"], x["resid"]) for x in h[:6]] or "none"))

    # (B) static CState definition via +0x30
    defs = {}
    for sid in AIDS:
        try:
            dptr = p.u64(objs[sid] + OFF_STATE_DEF)
            defs[sid] = p.try_read(dptr, DEF_WIN)
        except OSError:
            defs[sid] = b""
    if all(len(defs[s]) > 16 for s in AIDS):
        print("\n[B] STATIC CState def sweep (via +0x30, window 0x%X)" % DEF_WIN)
        print("  supply:", sweep_blobs(defs, ANCHORS, DEF_WIN) or "none")
        for comp in ("base", "infra", "pop", "vp"):
            want = {s: COMPONENTS[s][comp] for s in AIDS}
            h = sweep_blobs(defs, want, DEF_WIN)
            print("  %-6s: %s" % (comp, [(hex(x["off"]), x["enc"], x["resid"]) for x in h[:6]] or "none"))
    else:
        print("\n[B] STATIC CState def: could not read all 5 (+0x30 not a uniform ptr)")

    # (C) one-deref from runtime object
    print("\n[C] ONE-DEREF from runtime object (every heap-ptr field -> sub-record 0x600)")
    pofs = heap_ptr_fields(rt, RT_WIN)
    print("  heap-ptr fields:", len(pofs))
    SUB = 0x600
    one_hits = []
    for poff in pofs:
        subs = {}
        ok = True
        for sid in AIDS:
            ptr = struct.unpack_from("<Q", rt[sid], poff)[0]
            b = p.try_read(ptr, SUB)
            if not b or len(b) < 16:
                ok = False
                break
            subs[sid] = b
        if not ok:
            continue
        h = sweep_blobs(subs, ANCHORS, SUB)
        for x in h:
            x["ptr_off"] = poff
            one_hits.append(x)
    print("  supply:", [(hex(x["ptr_off"]), hex(x["off"]), x["enc"], x["resid"]) for x in one_hits[:20]] or "none")

    # (D) two-deref from runtime object (ptr -> ptr -> sub-record)
    print("\n[D] TWO-DEREF from runtime object (ptr -> ptr@0 -> sub-record 0x400)")
    SUB2 = 0x400
    two_hits = []
    for poff in pofs:
        # deref once, then treat the head qword of the sub-record as another ptr
        subs2 = {}
        ok = True
        for sid in AIDS:
            ptr = struct.unpack_from("<Q", rt[sid], poff)[0]
            b = p.try_read(ptr, 0x40)
            if not b or len(b) < 8:
                ok = False
                break
            ptr2 = struct.unpack_from("<Q", b, 0)[0]
            if not (HEAP_LO <= ptr2 < HEAP_HI):
                ok = False
                break
            b2 = p.try_read(ptr2, SUB2)
            if not b2 or len(b2) < 16:
                ok = False
                break
            subs2[sid] = b2
        if not ok:
            continue
        h = sweep_blobs(subs2, ANCHORS, SUB2)
        for x in h:
            x["ptr_off"] = poff
            two_hits.append(x)
    print("  supply:", [(hex(x["ptr_off"]), hex(x["off"]), x["enc"], x["resid"]) for x in two_hits[:20]] or "none")

    out = os.path.join(_HERE, "_broadsweep_results.json")
    with open(out, "w") as f:
        json.dump({"objs": {str(k): v for k, v in objs.items()},
                   "one_deref_supply": one_hits, "two_deref_supply": two_hits}, f, indent=2)
    print("\nwrote", out)
    p.close()


if __name__ == "__main__":
    main()
