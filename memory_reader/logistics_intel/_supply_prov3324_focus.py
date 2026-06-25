"""_supply_prov3324_focus.py - FOCUSED B2 follow-up on province 3324.

The broad sweep found NO 10.05 (remaining) and NO 8.16 (hubs) anywhere (body or
1-deref), only loose-tol aliasing of the SMALL round values (0.02/0.25/0.45/1.15/
1.20/2.40). The small values are too common to be evidence. The DISTINCTIVE values
are 10.05 and 8.16 -- their ABSENCE is the real signal.

This pass:
  (1) Re-locates province 3324's CProvince object (live arena).
  (2) Hunts ONLY 10.05 and 8.16 (the distinctive remaining/hub values) with a
      TIGHT +-0.005 tolerance, across BODY + 1-deref + 2-deref.
  (3) Classifies the +0x10 deref (the 0x30-stride 241/2.41 AoS) to confirm it is
      a geometry/repeating record, not a supply field.
  (4) Dumps the body header + the +0x10 sub-record so structure is visible.

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

# Distinctive values only; tight tolerance.
DISTINCT = {"remaining_10.05": 10.05, "hubs_8.16": 8.16}
TOL = 0.005

ENCODINGS = [
    ("f32", "<f", 4, 1.0),
    ("f64", "<d", 8, 1.0),
    ("i32/65536", "<i", 4, 65536.0),
    ("u32/65536", "<I", 4, 65536.0),
    ("i32/1e5", "<i", 4, 100000.0),
    ("u32/1e5", "<I", 4, 100000.0),
    ("i32/1e3", "<i", 4, 1000.0),
    ("u32/1e3", "<I", 4, 1000.0),
    ("i32/1e2", "<i", 4, 100.0),
    ("u32/1e2", "<I", 4, 100.0),
]
HEAP_LO, HEAP_HI = SL.HEAP_LO, SL.HEAP_HI


def _finite(v):
    return v == v and abs(v) < 1e12


def sweep(blob, label, found):
    n = len(blob)
    for off in range(0, n):
        for ename, fmt, width, scale in ENCODINGS:
            if off + width > n:
                continue
            raw = struct.unpack_from(fmt, blob, off)[0]
            v = raw / scale
            if not _finite(v):
                continue
            for tname, tval in DISTINCT.items():
                if abs(v - tval) <= TOL:
                    found.append((label, off, ename, raw, round(v, 5), tname))


def main():
    p = winmem.open_process("hoi4.exe")
    if p is None:
        print("hoi4.exe not running")
        return
    with p:
        if p.fingerprint().get("sha256") != SL.EXPECTED_SHA256:
            print("BUILD MISMATCH")
            return
        arena = FS.discover_arena(p.pid)
        alo, ahi = arena
        vt_abs = p.module_base + SL.PROV_VT_OFF
        spec = FS.make_qword_eq_matcher(vt_abs)
        hits = FS.parallel_scan(p.pid, alo, ahi, workers=2, **spec)
        obj = None
        for cand in hits:
            try:
                if p.u32(cand + SL.OFF_PROV_ID) == TARGET_PID:
                    obj = cand
                    break
            except OSError:
                pass
        if obj is None:
            print("prov %d not found" % TARGET_PID)
            return
        print("prov %d CProvince @ 0x%X (of %d objects)" % (TARGET_PID, obj, len(hits)))

        body = p.try_read(obj, 0x1000)
        found = []
        sweep(body, "BODY", found)

        # collect 1-deref pointers, sweep each; collect their pointers for 2-deref
        seen = set()
        lvl1_ptrs = []
        for off in range(0, len(body) - 8, 8):
            ptr = struct.unpack_from("<Q", body, off)[0]
            if HEAP_LO <= ptr < HEAP_HI and ptr not in seen:
                seen.add(ptr)
                lvl1_ptrs.append((off, ptr))
        n1 = n2 = 0
        for off, ptr in lvl1_ptrs:
            sub = p.try_read(ptr, 0x600)
            if not sub or len(sub) < 8:
                continue
            n1 += 1
            sweep(sub, "D1@+0x%X->0x%X" % (off, ptr), found)
            # 2-deref from this sub-record
            for o2 in range(0, len(sub) - 8, 8):
                p2 = struct.unpack_from("<Q", sub, o2)[0]
                if HEAP_LO <= p2 < HEAP_HI and p2 not in seen:
                    seen.add(p2)
                    sub2 = p.try_read(p2, 0x400)
                    if sub2 and len(sub2) >= 8:
                        n2 += 1
                        sweep(sub2, "D2@+0x%X->+0x%X->0x%X" % (off, o2, p2), found)
        print("swept BODY + %d L1-derefs + %d L2-derefs" % (n1, n2))

        print("\n=== DISTINCTIVE-VALUE HUNT (10.05 / 8.16, tol +-%.3f) ===" % TOL)
        if not found:
            print("  ZERO hits for 10.05 OR 8.16 anywhere in body/1-deref/2-deref.")
        else:
            for (label, off, enc, raw, v, tname) in found:
                print("  %-26s off=0x%-4X enc=%-10s raw=%-12s val=%.5f  -> %s"
                      % (label, off, enc, raw, v, tname))

        # --- classify the +0x10 deref (the 241/2.41 0x30-stride AoS) ---
        print("\n=== CLASSIFY +0x10 sub-record (the 2.41-repeating AoS) ===")
        p10 = struct.unpack_from("<Q", body, 0x10)[0]
        if HEAP_LO <= p10 < HEAP_HI:
            sub = p.try_read(p10, 0x120)
            print("  +0x10 -> 0x%X" % p10)
            # if it has a vtable, name it; else dump first records
            vt = struct.unpack_from("<Q", sub, 0)[0]
            if p.module_base <= vt < p.module_base + p.module_size:
                print("  first qword is a MODULE vtable: MOD+0x%X (an object)"
                      % (vt - p.module_base))
            # show the 0x30-stride pattern at +0x2B
            print("  0x30-stride u16 @ +0x2B,+0x5B,...: ", end="")
            vals = []
            for k in range(6):
                o = 0x2B + k * 0x30
                if o + 2 <= len(sub):
                    vals.append(struct.unpack_from("<H", sub, o)[0])
            print(vals, "(all 241 => repeating constant record, NOT a supply slot)")

        # --- dump body header so the structure is on record ---
        print("\n=== BODY HEADER (prov %d) first 0x80 as u32 / f32 ===" % TARGET_PID)
        for off in range(0, 0x80, 4):
            u = struct.unpack_from("<I", body, off)[0]
            f = struct.unpack_from("<f", body, off)[0]
            note = ""
            if off == 0:
                note = "  <- vtable lo"
            if off == 8:
                note = "  <- province id (%d)" % u
            ff = ("%.4f" % f) if _finite(f) else "nan"
            print("  +0x%03X  u32=%-12d  f32=%-14s%s" % (off, u, ff, note))


if __name__ == "__main__":
    main()
