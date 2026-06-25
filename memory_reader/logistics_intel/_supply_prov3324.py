"""_supply_prov3324.py - TARGETED B2 probe of province 3324's CProvince object.

Anchor (province 3324, state 806, Gdynia, paused ~1946.5.18):
  supply_remaining_province = 10.05   (B2 target; throughput/range-capped solver output)
  supply_from_hubs          = 8.16
  supply_from_state         = 2.40    (infra 1.20 + pop 0.02 + vp 1.15)

GOAL: does province 3324's CProvince object (vtable MOD+0x2950A50, id@+0x08) hold
10.05 (cached remaining) / 8.16 (hub) / 2.40 (state) / the components, at ANY offset
in [0..0x1000] or one pointer-deref away, in ANY common numeric encoding?

READ-ONLY. Reuses supply_live.enumerate_provinces + winmem.Process. No writes.
"""
import os
import struct
import sys

_HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(os.path.dirname(_HERE)))  # repo root for package
sys.path.insert(0, os.path.dirname(_HERE))                    # memory_reader
import winmem  # noqa: E402
import supply_live as SL  # noqa: E402
import fastscan as FS  # noqa: E402

TARGET_PID = 3324
TARGET_STATE = 806

# values to hunt (UI-rounded 2 dp). +-0.02 tolerance.
TARGETS = {
    "remaining_10.05": 10.05,
    "hubs_8.16": 8.16,
    "state_2.40": 2.40,
    "infra_1.20": 1.20,
    "vp_1.15": 1.15,
    "pop_0.02": 0.02,
    # also useful intermediates / sums seen in the tooltip
    "hubs+state_10.56": 10.56,
    "vp_bydgoszcz_0.25": 0.25,
    "vp_torun_0.45": 0.45,
}
TOL = 0.02

BODY_LEN = 0x1000      # sweep [0 .. 0x1000)
DEREF_LEN = 0x600      # one-deref sub-record sweep [0 .. 0x600)
SCAN_STEP = 1          # byte granularity

# encodings: (name, struct_fmt, width, scale)  value = unpack(fmt)/scale
ENCODINGS = [
    ("f32", "<f", 4, 1.0),
    ("f64", "<d", 8, 1.0),
    ("i32/65536", "<i", 4, 65536.0),    # the game native fixed_point
    ("u32/65536", "<I", 4, 65536.0),
    ("i32/1e5", "<i", 4, 100000.0),
    ("u32/1e5", "<I", 4, 100000.0),
    ("i32/1e3", "<i", 4, 1000.0),
    ("u32/1e3", "<I", 4, 1000.0),
    ("i32/1e2", "<i", 4, 100.0),
    ("u32/1e2", "<I", 4, 100.0),
    ("u16/1e2", "<H", 2, 100.0),
    ("i16/1e2", "<h", 2, 100.0),
]


def _finite(v):
    return v == v and v not in (float("inf"), float("-inf")) and abs(v) < 1e12


def sweep_blob(blob, base_off, label, found):
    """Sweep `blob` for any TARGET in any ENCODING. Append matches to `found`."""
    n = len(blob)
    for off in range(0, n, SCAN_STEP):
        for ename, fmt, width, scale in ENCODINGS:
            if off + width > n:
                continue
            try:
                raw = struct.unpack_from(fmt, blob, off)[0]
            except struct.error:
                continue
            v = raw / scale
            if not _finite(v):
                continue
            for tname, tval in TARGETS.items():
                if abs(v - tval) <= TOL:
                    found.append({
                        "region": label,
                        "offset": off,
                        "encoding": ename,
                        "raw": int(raw) if "f" not in fmt else raw,
                        "value": round(v, 5),
                        "matches": tname,
                        "target": tval,
                    })
    return found


def main():
    p = winmem.open_process("hoi4.exe")
    if p is None:
        print("hoi4.exe not running")
        return
    with p:
        fp = p.fingerprint()
        print("module_base=0x%X  sha256=%s" % (p.module_base, fp.get("sha256")))
        if fp.get("sha256") != SL.EXPECTED_SHA256:
            print("BUILD MISMATCH -- refusing (offsets build-specific)")
            return

        # --- LIVE arena discovery (the hardcoded ARENA_LO/HI in supply_live are
        #     stale after an ASLR reload; discover_arena walks the heap live). ---
        arena = FS.discover_arena(p.pid)
        if arena is None:
            print("could not discover heap arena -- ABORT")
            return
        alo, ahi = arena
        print("arena (live): 0x%X..0x%X  (%.2f GB)"
              % (alo, ahi, (ahi - alo) / float(1 << 30)))

        # --- Find province 3324's CProvince object: scan arena for the vtable
        #     (qword-eq, parallel, capped at 2 workers per concurrency note), then
        #     keep the candidate whose id field @+0x08 == 3324. ---
        vt_abs = p.module_base + SL.PROV_VT_OFF
        print("Scanning arena for CProvince vtable 0x%X (id@+0x%X == %d)..."
              % (vt_abs, SL.OFF_PROV_ID, TARGET_PID))
        spec = FS.make_qword_eq_matcher(vt_abs)
        hits = FS.parallel_scan(p.pid, alo, ahi, workers=2, **spec)
        print("  vtable hits: %d total CProvince-like objects" % len(hits))

        obj = None
        matches_3324 = []
        for cand in hits:
            try:
                pidf = p.u32(cand + SL.OFF_PROV_ID)
            except OSError:
                continue
            if pidf == TARGET_PID:
                matches_3324.append(cand)
        if not matches_3324:
            print("NO CProvince object with id==%d found -- ABORT" % TARGET_PID)
            return
        if len(matches_3324) > 1:
            print("  WARNING: %d objects claim id==%d: %s"
                  % (len(matches_3324), TARGET_PID,
                     ["0x%X" % a for a in matches_3324]))
        obj = matches_3324[0]
        print("  province %d CProvince body @ 0x%X" % (TARGET_PID, obj))

        # confirm id field
        pid_field = p.u32(obj + SL.OFF_PROV_ID)
        print("  id field @+0x%X = %d (expect %d) -> %s"
              % (SL.OFF_PROV_ID, pid_field, TARGET_PID,
                 "OK" if pid_field == TARGET_PID else "MISMATCH"))

        # read the whole body
        body = p.try_read(obj, BODY_LEN)
        print("  body read: %d bytes" % len(body))

        found = []
        sweep_blob(body, 0, "BODY", found)

        # 1-level derefs: every aligned qword in the body that looks like a heap
        # pointer -> read its sub-record and sweep that too.
        HEAP_LO, HEAP_HI = SL.HEAP_LO, SL.HEAP_HI
        seen_ptrs = set()
        nptr = 0
        for off in range(0, len(body) - 8, 8):
            ptr = struct.unpack_from("<Q", body, off)[0]
            if HEAP_LO <= ptr < HEAP_HI and ptr not in seen_ptrs:
                seen_ptrs.add(ptr)
                sub = p.try_read(ptr, DEREF_LEN)
                if sub and len(sub) >= 8:
                    nptr += 1
                    sweep_blob(sub, 0, "DEREF@body+0x%X->0x%X" % (off, ptr), found)
        print("  swept %d distinct heap-pointer derefs" % nptr)

        # report
        print("\n=== CANDIDATE MATCHES (prov %d) ===" % TARGET_PID)
        if not found:
            print("  NONE. No offset in body[0..0x1000] or 1-deref holds any target "
                  "value in any of %d encodings (+-%.2f)." % (len(ENCODINGS), TOL))
        else:
            # group by which target matched
            by_target = {}
            for f in found:
                by_target.setdefault(f["matches"], []).append(f)
            for tname in sorted(by_target):
                rows = by_target[tname]
                print("\n  >> %s (target=%.2f) : %d hit(s)"
                      % (tname, TARGETS[tname], len(rows)))
                for r in rows[:40]:
                    print("     %-30s off=0x%-4X enc=%-10s raw=%-12s val=%.5f"
                          % (r["region"], r["offset"], r["encoding"],
                             r["raw"], r["value"]))
                if len(rows) > 40:
                    print("     ... +%d more" % (len(rows) - 40))

        # Highlight: did the B2 target (10.05) appear in the BODY (not deref) at all?
        body_remaining = [f for f in found
                          if f["matches"] == "remaining_10.05" and f["region"] == "BODY"]
        print("\n=== B2 VERDICT (10.05 in CProvince BODY directly) ===")
        if body_remaining:
            for r in body_remaining:
                print("  CACHED-REMAINING CANDIDATE: body+0x%X enc=%s val=%.5f"
                      % (r["offset"], r["encoding"], r["value"]))
        else:
            print("  10.05 NOT found inline in the CProvince body.")


if __name__ == "__main__":
    main()
