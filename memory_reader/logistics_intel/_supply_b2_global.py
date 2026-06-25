"""_supply_b2_global.py - Codex residual hardeners for the B2 refutation.

Two cheap extra tests Codex asked for before declaring the wall:

(A) ENCODING EXTENSION on the CProvince-rooted structural scan: re-run the wide
    structural intersection adding f64 and fixed /1e4 and /1e6 (the headline scan
    already covered i32/1e5, u32/1e5, f32, i32/65536, u32/65536, i32/1e3, i32/1e2).
    Decisive iff intersection across the 3 distinctive provinces is still EMPTY.

(B) GLOBAL PROVINCE-INDEXED ARRAY test: forget the CProvince object -- is the
    per-province remaining stored in a flat arena array indexed by province id?
    Find every arena address whose decoded value == a DISTINCTIVE province's
    remaining, for 3 distinctive provinces (3324=10.05, 9524=5.87, 687=0.28), in
    several encodings, then test base = hit - pid*stride for a range of strides,
    requiring the SAME base+stride to read all 3 provinces' remaining at
    base + pid*stride. (Same decisive logic the prior state-supply stridescan used.)
    PASS would mean a stored flat array exists; EMPTY across all strides => none.

READ-ONLY. 1 worker. No writes.
"""
import os
import struct
import sys
from collections import defaultdict

_HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(os.path.dirname(_HERE)))
sys.path.insert(0, os.path.dirname(_HERE))
import winmem  # noqa: E402
import supply_live as SL  # noqa: E402
import fastscan as FS  # noqa: E402

ANCHORS = {3324: 10.05, 9524: 5.87, 687: 0.28, 10523: 0.00}
DISCRIMINATING = [3324, 9524, 687]
HEAP_LO, HEAP_HI = SL.HEAP_LO, SL.HEAP_HI
TOL = 0.02

# (A) extra encodings only (the 7 already-tested ones are not repeated)
EXTRA_ENC = [
    ("f64", "<d", 8, 1.0),
    ("i32/1e4", "<i", 4, 10000.0),
    ("i32/1e6", "<i", 4, 1000000.0),
    ("u32/1e6", "<I", 4, 1000000.0),
]
BODY_LEN = 0x1000
BAND_LEN = 0x600
LEAF_LEN = 0x300


def _finite(v):
    return v == v and v not in (float("inf"), float("-inf")) and abs(v) < 1e9


def find_slots_extra(p, obj, target):
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
            for vo in range(0, len(rec) - 8, 4):
                for ename, fmt, width, scale in EXTRA_ENC:
                    if vo + width > len(rec):
                        continue
                    raw = struct.unpack_from(fmt, rec, vo)[0]
                    v = raw / scale
                    if not _finite(v):
                        continue
                    if abs(v - target) <= TOL:
                        found.add((bo, lo, vo, ename))
    return found


# (B) global flat-array test --------------------------------------------------
# distinctive raw values to hunt arena-wide, per encoding
GLOBAL_ENC = [
    ("f32", "<f", 4, 1.0),
    ("i32/1e5", "<i", 4, 100000.0),
    ("i32/1e3", "<i", 4, 1000.0),
    ("i32/65536", "<i", 4, 65536.0),
]
STRIDES = [4, 6, 8, 12, 16, 24, 32, 40, 48, 64, 96, 128, 256]


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
        if arena is None:
            print("no arena")
            return
        alo, ahi = arena
        print("arena 0x%X..0x%X" % (alo, ahi))

        # locate the 4 provinces (reuse vtable scan, 1 worker)
        vt_abs = p.module_base + SL.PROV_VT_OFF
        spec = FS.make_qword_eq_matcher(vt_abs)
        hits = FS.parallel_scan(p.pid, alo, ahi, workers=1, **spec)
        id2obj = {}
        for cand in hits:
            try:
                pidf = p.u32(cand + SL.OFF_PROV_ID)
            except OSError:
                continue
            if pidf in ANCHORS and pidf not in id2obj:
                id2obj[pidf] = cand
        print("located %d/%d\n" % (len(id2obj), len(ANCHORS)))

        # ---- (A) extra-encoding structural intersection ----
        print("=" * 70)
        print("(A) CProvince-rooted structural scan, EXTRA encodings (f64,/1e4,/1e6)")
        print("=" * 70)
        slots = {}
        for pid in DISCRIMINATING:
            obj = id2obj.get(pid)
            if obj is None:
                slots[pid] = set()
                continue
            slots[pid] = find_slots_extra(p, obj, ANCHORS[pid])
            print("  prov %-6d remaining=%.2f : %d extra-encoding slots"
                  % (pid, ANCHORS[pid], len(slots[pid])))
        common = None
        for pid in DISCRIMINATING:
            if id2obj.get(pid):
                common = slots[pid] if common is None else (common & slots[pid])
        common = common or set()
        if common:
            print("  COMMON extra-encoding slot(s):")
            for c in sorted(common):
                print("    band+0x%X leaf+0x%X val+0x%X %s" % c)
        else:
            print("  COMMON: NONE (extra encodings add no stored slot).")

        # ---- (B) global province-id-indexed flat array ----
        print("\n" + "=" * 70)
        print("(B) GLOBAL province-id-indexed flat-array test (3 distinctive provs)")
        print("=" * 70)
        # collect arena-wide hit addresses for each (province, encoding)
        regions = list(p.iter_regions(alo, ahi, want_exec=False))
        # for each encoding, build address sets for each distinctive province value
        found_any = False
        for ename, fmt, width, scale in GLOBAL_ENC:
            # gather hit address lists per province for this encoding
            hitsets = {pid: [] for pid in DISCRIMINATING}
            for base, size in regions:
                blob = p.try_read(base, size)
                if not blob or len(blob) < width:
                    continue
                # step by 4 (typical scalar alignment)
                for off in range(0, len(blob) - width, 4):
                    raw = struct.unpack_from(fmt, blob, off)[0]
                    v = raw / scale
                    if not _finite(v):
                        continue
                    for pid in DISCRIMINATING:
                        if abs(v - ANCHORS[pid]) <= TOL:
                            hitsets[pid].append(base + off)
            counts = {pid: len(hitsets[pid]) for pid in DISCRIMINATING}
            # cap: if any province has 0 hits, no array possible in this enc
            if any(counts[pid] == 0 for pid in DISCRIMINATING):
                print("  enc=%-10s hits=%s -> a province has 0 hits, no array" % (ename, counts))
                continue
            # stride test: anchor on the RAREST province to bound work
            anchor_pid = min(DISCRIMINATING, key=lambda x: counts[x])
            others = [x for x in DISCRIMINATING if x != anchor_pid]
            set_by_pid = {pid: set(hitsets[pid]) for pid in DISCRIMINATING}
            hit_array = False
            for a in hitsets[anchor_pid]:
                base = a - anchor_pid * 1  # placeholder; recomputed per stride
                for stride in STRIDES:
                    b0 = a - anchor_pid * stride
                    ok = True
                    for pid in others:
                        if (b0 + pid * stride) not in set_by_pid[pid]:
                            ok = False
                            break
                    if ok:
                        # also require 10523==0.00 non-contradiction if locatable
                        print("    POSSIBLE ARRAY enc=%s base=0x%X stride=%d" % (ename, b0, stride))
                        hit_array = True
                        found_any = True
            print("  enc=%-10s hits=%s -> %s"
                  % (ename, counts, "ARRAY CANDIDATE(S) above" if hit_array else "no consistent base+stride"))
        if not found_any:
            print("\n  GLOBAL ARRAY: NONE. No base+stride reads all 3 distinctive")
            print("  provinces' remaining at base+pid*stride in any tested encoding.")

        # ---- verdict ----
        print("\n" + "=" * 70)
        print("RESIDUAL VERDICT")
        print("=" * 70)
        print("  (A) extra-encoding CProvince intersection: %s"
              % ("FOUND -- re-examine" if common else "EMPTY"))
        print("  (B) global province-indexed array:         %s"
              % ("FOUND -- re-examine" if found_any else "EMPTY"))
        if not common and not found_any:
            print("  => Both residual hardeners EMPTY. B2 stored-slot hypothesis fully")
            print("     refuted across CProvince-rooted AND global-array models. Wall stands.")


if __name__ == "__main__":
    main()
