"""_supply_prov3324_struct.py - map the supply sub-record structure for prov 3324.

The focused hunt found 10.05 (remaining) and 8.16 (hubs) as i32 fixed-point in a
2-deref sub-record. Cleanest path:
    CProvince(body) +0x490 -> ptr -> +0x270 -> ptr(0x24A2A6E4D40) : holds 10.05 at +0x108/+0x118/+0x160
    CProvince(body) +0x490 -> ptr -> +0x310 -> ptr(0x24A2A5CE940) : holds 8.158 at +0x119/+0x161
Also a primary path via +0x190.

This pass walks the CProvince body's pointer fields, re-derives the EXACT chain to
the sub-record(s) holding 10.05/8.16, and dumps the full sub-record [0x100..0x300)
as fixed-point so EVERY supply-band sibling (state 2.40, infra/vp/pop components,
remaining, hubs) is visible at its offset. This is what we need to (a) name the
remaining offset and (b) tell the user which offset to validate across more provinces.

READ-ONLY.
"""
import os
import struct
import sys

_HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(os.path.dirname(_HERE)))
sys.path.insert(0, os.path.dirname(_HERE))
import winmem  # noqa: E402
import supply_live as SL  # noqa: E402
import fastscan as FS  # noqa: E402

TARGET_PID = 3324
HEAP_LO, HEAP_HI = SL.HEAP_LO, SL.HEAP_HI


def fp(raw_i32):
    return {
        "/1e5": raw_i32 / 100000.0,
        "/65536": raw_i32 / 65536.0,
        "/1e3": raw_i32 / 1000.0,
    }


def locate(p):
    arena = FS.discover_arena(p.pid)
    alo, ahi = arena
    vt_abs = p.module_base + SL.PROV_VT_OFF
    spec = FS.make_qword_eq_matcher(vt_abs)
    hits = FS.parallel_scan(p.pid, alo, ahi, workers=2, **spec)
    for cand in hits:
        try:
            if p.u32(cand + SL.OFF_PROV_ID) == TARGET_PID:
                return cand
        except OSError:
            pass
    return None


def dump_record(p, addr, name, lo=0x100, hi=0x300):
    print("\n--- %s @ 0x%X  body[0x%X..0x%X) as i32 fixed-point ---" % (name, addr, lo, hi))
    blob = p.try_read(addr + lo, hi - lo)
    if not blob:
        print("   (unreadable)")
        return
    for o in range(0, len(blob) - 4, 4):
        raw = struct.unpack_from("<i", blob, o)[0]
        if raw == 0:
            continue
        v = fp(raw)
        # flag interesting supply-band values
        flag = ""
        for label, tgt in (("REMAIN~10.05", 10.05), ("HUBS~8.16", 8.16),
                           ("STATE~2.40", 2.40), ("INFRA~1.20", 1.20),
                           ("VP~1.15", 1.15)):
            if abs(v["/1e5"] - tgt) <= 0.02 or abs(v["/65536"] - tgt) <= 0.02 \
               or abs(v["/1e3"] - tgt) <= 0.02:
                flag = " <<< " + label
        # only print plausibly-supply-scaled values to cut noise
        if 0.001 <= abs(v["/1e5"]) <= 200 or 0.001 <= abs(v["/65536"]) <= 200:
            print("   +0x%03X  raw=%-11d  /1e5=%-10.5f /65536=%-10.5f /1e3=%-10.3f%s"
                  % (lo + o, raw, v["/1e5"], v["/65536"], v["/1e3"], flag))


def main():
    p = winmem.open_process("hoi4.exe")
    if p is None:
        print("hoi4.exe not running")
        return
    with p:
        if p.fingerprint().get("sha256") != SL.EXPECTED_SHA256:
            print("BUILD MISMATCH")
            return
        obj = locate(p)
        if obj is None:
            print("prov not found")
            return
        print("prov %d CProvince @ 0x%X" % (TARGET_PID, obj))

        # Re-derive the chains. body+0x490 -> P -> +0x270 -> Q (10.05); +0x310 -> R (8.158)
        def deref(a):
            v = p.u64(a)
            return v if HEAP_LO <= v < HEAP_HI else None

        print("\n=== CHAIN A: body+0x490 -> +0x270 (10.05) and +0x310 (8.158) ===")
        P = deref(obj + 0x490)
        print("  body+0x490 -> 0x%X" % (P or 0))
        if P:
            Q = deref(P + 0x270)
            R = deref(P + 0x310)
            print("  +0x270 -> 0x%X    +0x310 -> 0x%X" % (Q or 0, R or 0))
            if Q:
                dump_record(p, Q, "Q=+0x490->+0x270 (REMAINING record)")
            if R:
                dump_record(p, R, "R=+0x490->+0x310 (HUBS record)")

        print("\n=== CHAIN B: body+0x190 -> +0x348/+0x3A0/+0x3D0 ===")
        P2 = deref(obj + 0x190)
        print("  body+0x190 -> 0x%X" % (P2 or 0))
        if P2:
            for sub_off in (0x348, 0x3A0, 0x3D0):
                S = deref(P2 + sub_off)
                if S:
                    dump_record(p, S, "B +0x190->+0x%X" % sub_off)

        # Also: are +0x490 and +0x190 the SAME pointer? (i.e. one manager)
        print("\n=== pointer-field map of CProvince body (heap ptrs) ===")
        body = p.try_read(obj, 0x1000)
        for o in range(0, len(body) - 8, 8):
            ptr = struct.unpack_from("<Q", body, o)[0]
            if HEAP_LO <= ptr < HEAP_HI:
                print("  body+0x%03X -> 0x%X" % (o, ptr))


if __name__ == "__main__":
    main()
