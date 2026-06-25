"""_supply_b2_validate.py - FINAL VALIDATION of the B2 supply pin (wall B2).

Confirms the candidate CProvince chain holds each province's OWN
supply_remaining across 4 VARIED provinces, with the SAME offset + encoding.

Candidate pin (from prov 3324):
  CProvince(id).body+0x190 -> deref -> +0x348 -> deref -> i32 @ +0x108 / 100000

ANCHORS (supply_province_anchors_live.json):
  3324  remaining 10.05  hubs  8.16  state 2.40  (re-confirm)
  9524  remaining  5.87  hubs  5.87  state 7.00  (remaining==hubs; must NOT read 7.00)
  687   remaining  0.28  hubs 13.10  state 1.00  (KILLER: must read 0.28 NOT 13.10)
  10523 remaining  0.00  hubs  0.00  state 2.00  (zero; must NOT read 2.00)

PASS iff round(mem,2) == tooltip_remaining for ALL 4 with the SAME (chain,enc),
AND the killer (687) reads 0.28 not 13.10, AND it does NOT alias hubs/state.

Also reads the hub candidate (body+0x190 -> +0x3D0 -> +0x288) for corroboration.

If +0x348 leaf is NULL: scan the band-pointers (body+0x190 region) for the leaf
whose +0x108 rounds to that province's remaining, and report whether a SINGLE
consistent (band-offset, leaf-offset) wins across all 4 (stable) or varies (weak).

READ-ONLY. 1 worker (concurrency: weather + deep-naval scans live). No writes.
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

# province -> (remaining, hubs, state)
ANCHORS = {
    3324:  (10.05, 8.16,  2.40),
    9524:  (5.87,  5.87,  7.00),
    687:   (0.28,  13.10, 1.00),
    10523: (0.00,  0.00,  2.00),
}
PROBE_IDS = list(ANCHORS.keys())

HEAP_LO, HEAP_HI = SL.HEAP_LO, SL.HEAP_HI
FP = 100000.0
TOL = 0.02  # +-0.02 on a 2dp tooltip read (rounding + measurement)

# candidate chains
REM_CHAIN = ("body+0x190 -> +0x348 -> +0x108", 0x190, 0x348, 0x108)
HUB_CHAIN = ("body+0x190 -> +0x3D0 -> +0x288", 0x190, 0x3D0, 0x288)

# band-pointer offsets in the CProvince body (the 0x180-strided heap ptrs the
# prior agent mapped) -- used for the fallback scan when +0x348 is null.
BAND_OFFS = [0x10, 0x190, 0x310, 0x490, 0x610, 0x790, 0x910,
             0xA90, 0xC10, 0xD90, 0xF10, 0x958, 0x970, 0x988]
# leaf offsets within a band sub-object that the prior agent saw carry values.
LEAF_PTR_OFFS = [0x270, 0x310, 0x348, 0x3A0, 0x3D0]
# value slots within a leaf record.
VALUE_OFFS = [0x108, 0x118, 0x128, 0x288, 0x298, 0x2A8]


def deref(p, a):
    """Read a qword at `a`; return it iff it looks like a live heap pointer."""
    try:
        v = p.u64(a)
    except OSError:
        return None
    return v if (v is not None and HEAP_LO <= v < HEAP_HI) else None


def read_i32_fp(p, a):
    """i32 fixed-point /1e5 at address a, or None."""
    try:
        raw = p.i32(a)
    except OSError:
        return None, None
    if raw is None:
        return None, None
    return raw, raw / FP


def follow(p, obj, band_off, leaf_off, val_off):
    """body+band_off -> deref -> +leaf_off -> deref -> i32@+val_off /1e5.
    Returns (record_addr, raw, value) or (None, None, None)."""
    band = deref(p, obj + band_off)
    if band is None:
        return None, None, None
    rec = deref(p, band + leaf_off)
    if rec is None:
        return None, None, None
    raw, val = read_i32_fp(p, rec + val_off)
    return rec, raw, val


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

        arena = FS.discover_arena(p.pid)
        if arena is None:
            print("could not discover heap arena -- ABORT")
            return
        alo, ahi = arena
        print("arena (live): 0x%X..0x%X  (%.2f GB)"
              % (alo, ahi, (ahi - alo) / float(1 << 30)))

        # locate all 4 CProvince objects in ONE scan (1 worker per concurrency note)
        vt_abs = p.module_base + SL.PROV_VT_OFF
        print("Scanning arena for CProvince vtable 0x%X (1 worker)..." % vt_abs)
        spec = FS.make_qword_eq_matcher(vt_abs)
        hits = FS.parallel_scan(p.pid, alo, ahi, workers=1, **spec)
        print("  %d CProvince-like objects" % len(hits))

        id2obj = {}
        wanted = set(PROBE_IDS)
        for cand in hits:
            try:
                pidf = p.u32(cand + SL.OFF_PROV_ID)
            except OSError:
                continue
            if pidf in wanted and pidf not in id2obj:
                id2obj[pidf] = cand
        missing = wanted - set(id2obj)
        print("  located %d/%d target provinces%s\n"
              % (len(id2obj), len(PROBE_IDS),
                 ("" if not missing else "  MISSING: %s" % sorted(missing))))

        # =============================================================
        # PASS 1: the candidate REMAINING chain, same offset for all 4
        # =============================================================
        print("=" * 78)
        print("PASS 1  REMAINING candidate: %s   (i32/1e5)" % REM_CHAIN[0])
        print("=" * 78)
        _, b_off, l_off, v_off = REM_CHAIN
        print("%-7s %-16s %-10s %-10s %-9s %-9s %-6s"
              % ("prov", "record", "read", "remain", "hubs", "state", "verd"))
        rem_results = {}
        rem_records = {}
        for pid in PROBE_IDS:
            obj = id2obj.get(pid)
            rem, hubs, state = ANCHORS[pid]
            if obj is None:
                print("%-7d (not located)" % pid)
                rem_results[pid] = None
                continue
            rec, raw, val = follow(p, obj, b_off, l_off, v_off)
            rem_records[pid] = rec
            if val is None:
                print("%-7d %-16s %-10s %-10.2f %-9.2f %-9.2f %s"
                      % (pid, "NULL-leaf", "None", rem, hubs, state, "NULL"))
                rem_results[pid] = None
                continue
            rval = round(val, 2)
            ok = abs(val - rem) <= TOL
            # does it instead alias hubs/state? (diagnostic)
            alias = []
            if abs(val - hubs) <= TOL and abs(rem - hubs) > TOL:
                alias.append("HUBS")
            if abs(val - state) <= TOL and abs(rem - state) > TOL:
                alias.append("STATE")
            verd = "PASS" if ok else ("FAIL/" + "+".join(alias) if alias else "FAIL")
            print("%-7d 0x%-14X %-10.5f %-10.2f %-9.2f %-9.2f %s"
                  % (pid, rec or 0, val, rem, hubs, state, verd))
            rem_results[pid] = (rval, ok, val)

        passes = sum(1 for r in rem_results.values() if r and r[1])
        killer = rem_results.get(687)
        killer_ok = bool(killer and killer[1] and abs(killer[2] - 0.28) <= TOL)
        print("\n  REMAINING chain: %d/4 PASS  | killer(687)=0.28? %s"
              % (passes, "YES" if killer_ok else "NO"))

        # =============================================================
        # PASS 2: the HUB corroborating field
        # =============================================================
        print("\n" + "=" * 78)
        print("PASS 2  HUB candidate: %s   (i32/1e5)" % HUB_CHAIN[0])
        print("=" * 78)
        _, hb_off, hl_off, hv_off = HUB_CHAIN
        print("%-7s %-16s %-10s %-9s %-6s"
              % ("prov", "record", "read", "hubs", "verd"))
        hub_pass = 0
        hub_n = 0
        for pid in PROBE_IDS:
            obj = id2obj.get(pid)
            rem, hubs, state = ANCHORS[pid]
            if obj is None:
                print("%-7d (not located)" % pid)
                continue
            rec, raw, val = follow(p, obj, hb_off, hl_off, hv_off)
            if val is None:
                print("%-7d %-16s %-10s %-9.2f %s" % (pid, "NULL-leaf", "None", hubs, "NULL"))
                continue
            hub_n += 1
            ok = abs(val - hubs) <= TOL
            if ok:
                hub_pass += 1
            print("%-7d 0x%-14X %-10.5f %-9.2f %s"
                  % (pid, rec or 0, val, hubs, "PASS" if ok else "FAIL"))
        print("\n  HUB chain: %d/%d match the hub value" % (hub_pass, hub_n))

        # =============================================================
        # PASS 3: fallback band-scan for any province whose REM leaf was null
        #         OR whose candidate read did not match -> find which fixed
        #         (band_off, leaf_off, val_off) reads each province's remaining,
        #         and whether ONE structural address wins across all 4.
        # =============================================================
        print("\n" + "=" * 78)
        print("PASS 3  Structural scan: which fixed (band,leaf,val) reads remaining?")
        print("=" * 78)
        # For each province, collect every (band_off,leaf_off,val_off) whose
        # i32/1e5 rounds to that province's remaining (within TOL).
        per_prov_winners = {}
        for pid in PROBE_IDS:
            obj = id2obj.get(pid)
            rem, hubs, state = ANCHORS[pid]
            if obj is None:
                per_prov_winners[pid] = set()
                continue
            winners = set()
            for bo in BAND_OFFS:
                band = deref(p, obj + bo)
                if band is None:
                    continue
                for lo in LEAF_PTR_OFFS:
                    rec = deref(p, band + lo)
                    if rec is None:
                        continue
                    for vo in VALUE_OFFS:
                        raw, val = read_i32_fp(p, rec + vo)
                        if val is None:
                            continue
                        if abs(val - rem) <= TOL:
                            winners.add((bo, lo, vo))
            per_prov_winners[pid] = winners
            print("  prov %-6d remaining=%.2f : %d (band,leaf,val) combos read it%s"
                  % (pid, rem, len(winners),
                     "" if rem > 0.01 else "  (zero is easy to hit -> low specificity)"))

        # intersection: a structural address that reads remaining for ALL 4
        located = [pid for pid in PROBE_IDS if id2obj.get(pid)]
        common = None
        for pid in located:
            common = per_prov_winners[pid] if common is None else (common & per_prov_winners[pid])
        common = common or set()
        print("\n  COMMON (band,leaf,val) that reads remaining across all %d located provinces:"
              % len(located))
        if not common:
            print("    NONE -- no single structural address reads every province's remaining.")
        else:
            for (bo, lo, vo) in sorted(common):
                print("    body+0x%X -> +0x%X -> +0x%X   <<< stable structural pin"
                      % (bo, lo, vo))
            # is the headline candidate among the winners?
            headline = (0x190, 0x348, 0x108)
            print("    headline candidate (0x190,0x348,0x108) in common set: %s"
                  % ("YES" if headline in common else "NO"))

        # =============================================================
        # VERDICT
        # =============================================================
        print("\n" + "=" * 78)
        print("B2 VERDICT")
        print("=" * 78)
        all4 = (passes == 4)
        headline_stable = (0x190, 0x348, 0x108) in common
        locked = (all4 and killer_ok) or (headline_stable and killer_ok and len(located) == 4)
        print("  remaining chain 4/4 PASS (headline +0x348/+0x108): %s" % all4)
        print("  killer 687 reads 0.28 (not 13.10):                 %s" % killer_ok)
        print("  headline chain in the all-4 common structural set: %s" % headline_stable)
        print("  -> B2 %s" % ("LOCKED" if locked else "NOT locked (see above)"))


if __name__ == "__main__":
    main()
