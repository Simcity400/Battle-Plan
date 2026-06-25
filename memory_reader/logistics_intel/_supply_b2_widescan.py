"""_supply_b2_widescan.py - WIDE structural search for a STORED per-province
'remaining' slot that holds each of the 4 anchors' OWN remaining with ONE
consistent (band_off, leaf_off, val_off, encoding).

This is the decisive falsifier for B2: the narrow candidate (0x190/0x348/0x108)
failed 3/4. Here we cast a much wider net -- every band pointer in the body, a
dense leaf-pointer sweep, a dense value sweep, multiple encodings -- and require
the SAME structural address+encoding to read all 4 provinces' remaining. If even
this finds nothing, the per-province remaining is not a read-only-pinnable stored
slot (it is solver/manager computed), and B2 stays a documented wall.

Strategy to keep it cheap + avoid zero/aliasing false-positives:
  * Use the 3 DISCRIMINATING provinces (3324=10.05, 9524=5.87, 687=0.28) as the
    primary constraint; 10523=0.00 is included only as a non-contradiction check
    (0.00 is trivially common -> not a positive signal).
  * A structural slot is a CANDIDATE iff it reads remaining for 3324 AND 9524 AND
    687 with the SAME (band,leaf,val,enc). Those three are mutually distinctive.

READ-ONLY. 1 worker. No writes.
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

ANCHORS = {3324: 10.05, 9524: 5.87, 687: 0.28, 10523: 0.00}
DISCRIMINATING = [3324, 9524, 687]   # mutually distinctive; the real test
PROBE_IDS = list(ANCHORS.keys())
HEAP_LO, HEAP_HI = SL.HEAP_LO, SL.HEAP_HI
TOL = 0.02

# wide band-pointer set: scan EVERY aligned qword in body[0..0x1000] that looks
# like a heap pointer (not just the prior agent's 0x180-strided list).
BODY_LEN = 0x1000
# within each band sub-object, scan EVERY aligned qword in [0..0x600) that looks
# like a heap pointer -> a candidate leaf record.
BAND_LEN = 0x600
# within each leaf record, scan EVERY 4-aligned value slot in [0..0x300).
LEAF_LEN = 0x300

ENCODINGS = [
    ("i32/1e5", "<i", 4, 100000.0),
    ("u32/1e5", "<I", 4, 100000.0),
    ("f32", "<f", 4, 1.0),
    ("i32/65536", "<i", 4, 65536.0),
    ("u32/65536", "<I", 4, 65536.0),
    ("i32/1e3", "<i", 4, 1000.0),
    ("i32/1e2", "<i", 4, 100.0),
]


def _finite(v):
    return v == v and v not in (float("inf"), float("-inf")) and abs(v) < 1e9


def heap_ptr(p, a):
    try:
        v = p.u64(a)
    except OSError:
        return None
    return v if (v is not None and HEAP_LO <= v < HEAP_HI) else None


def find_slots_for(p, obj, target):
    """Return the set of (band_off, leaf_off, val_off, enc) structural addresses
    on this province object whose decoded value == `target` (+-TOL)."""
    found = set()
    body = p.try_read(obj, BODY_LEN)
    if not body or len(body) < 16:
        return found
    for bo in range(0, len(body) - 8, 8):
        band = struct.unpack_from("<Q", body, bo)[0]
        if not (HEAP_LO <= band < HEAP_HI):
            continue
        sub = p.try_read(band, BAND_LEN)
        if not sub or len(sub) < 16:
            continue
        for lo in range(0, len(sub) - 8, 8):
            leaf = struct.unpack_from("<Q", sub, lo)[0]
            if not (HEAP_LO <= leaf < HEAP_HI):
                continue
            rec = p.try_read(leaf, LEAF_LEN)
            if not rec or len(rec) < 8:
                continue
            for vo in range(0, len(rec) - 4, 4):
                for ename, fmt, width, scale in ENCODINGS:
                    raw = struct.unpack_from(fmt, rec, vo)[0]
                    v = raw / scale
                    if not _finite(v):
                        continue
                    if abs(v - target) <= TOL:
                        found.add((bo, lo, vo, ename))
    return found


def main():
    p = winmem.open_process("hoi4.exe")
    if p is None:
        print("hoi4.exe not running")
        return
    with p:
        if p.fingerprint().get("sha256") != SL.EXPECTED_SHA256:
            print("BUILD MISMATCH -- refusing")
            return
        arena = FS.discover_arena(p.pid)
        if arena is None:
            print("could not discover arena -- ABORT")
            return
        alo, ahi = arena
        print("arena 0x%X..0x%X" % (alo, ahi))
        vt_abs = p.module_base + SL.PROV_VT_OFF
        spec = FS.make_qword_eq_matcher(vt_abs)
        hits = FS.parallel_scan(p.pid, alo, ahi, workers=1, **spec)
        id2obj = {}
        wanted = set(PROBE_IDS)
        for cand in hits:
            try:
                pidf = p.u32(cand + SL.OFF_PROV_ID)
            except OSError:
                continue
            if pidf in wanted and pidf not in id2obj:
                id2obj[pidf] = cand
        print("located %d/%d (%d objects)\n" % (len(id2obj), len(PROBE_IDS), len(hits)))

        slots = {}
        for pid in PROBE_IDS:
            obj = id2obj.get(pid)
            if obj is None:
                slots[pid] = set()
                print("prov %-6d NOT located" % pid)
                continue
            s = find_slots_for(p, obj, ANCHORS[pid])
            slots[pid] = s
            print("prov %-6d remaining=%.2f : %d structural slots read it (wide net)"
                  % (pid, ANCHORS[pid], len(s)))

        # intersection over the 3 DISCRIMINATING provinces (the real test)
        disc = [pid for pid in DISCRIMINATING if id2obj.get(pid)]
        common = None
        for pid in disc:
            common = slots[pid] if common is None else (common & slots[pid])
        common = common or set()

        print("\n%s" % ("=" * 70))
        print("COMMON (band,leaf,val,enc) reading remaining across the 3 DISCRIMINATING")
        print("provinces (3324=10.05, 9524=5.87, 687=0.28):")
        print("=" * 70)
        if not common:
            print("  NONE.  No single structural slot+encoding holds all three")
            print("         provinces' OWN remaining. => per-province remaining is")
            print("         NOT a read-only-pinnable stored field (computed/solver-owned).")
        else:
            for (bo, lo, vo, en) in sorted(common):
                # does it ALSO survive 10523==0.00 (non-contradiction)?
                also0 = (bo, lo, vo, en) in slots.get(10523, set())
                print("  body+0x%X -> +0x%X -> +0x%X  enc=%-10s  (10523==0.00 too: %s)"
                      % (bo, lo, vo, en, "yes" if also0 else "no"))
            print("\n  >>> %d candidate structural pin(s) survive the 3-province test."
                  % len(common))

        print("\nB2: %s" % ("STORED SLOT FOUND -- re-examine" if common
                            else "NO STORED SLOT -- candidate pin REFUTED, wall stands"))


if __name__ == "__main__":
    main()
