"""_supply_alias_check.py - rule out sub-record aliasing across provinces.

Codex flagged: the +0x108=10.05 candidate is only meaningful if the supply
sub-records are province-LOCAL (distinct pointers per province), not shared. This
locates several provinces' CProvince objects and prints, for the candidate chain
(body+0x190 -> deref+0x348 -> record), the record ADDRESS + the +0x108 value, so we
can confirm distinct addresses + distinct values across provinces. Cheap, read-only.
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

# probe a spread: 3324 (anchor) + neighbours + a few random ids
PROBE_IDS = [3323, 3324, 3325, 3326, 1, 100, 806, 5000, 9000, 13000]
HEAP_LO, HEAP_HI = SL.HEAP_LO, SL.HEAP_HI


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
        # build id -> obj map for our probe ids only
        id2obj = {}
        wanted = set(PROBE_IDS)
        for cand in hits:
            try:
                pidf = p.u32(cand + SL.OFF_PROV_ID)
            except OSError:
                continue
            if pidf in wanted and pidf not in id2obj:
                id2obj[pidf] = cand
        print("located %d/%d probe provinces (of %d objects)\n"
              % (len(id2obj), len(PROBE_IDS), len(hits)))

        def deref(a):
            try:
                v = p.u64(a)
            except OSError:
                return None
            return v if HEAP_LO <= v < HEAP_HI else None

        print("CHAIN: body+0x190 -> deref -> +0x348 -> deref -> record; read +0x108 i32/1e5")
        print("%-7s %-16s %-16s %-16s %-10s" %
              ("provid", "CProvince", "P=+0x190", "record", "+0x108/1e5"))
        records = {}
        for pid in PROBE_IDS:
            obj = id2obj.get(pid)
            if obj is None:
                print("%-7d (not located)" % pid)
                continue
            P = deref(obj + 0x190)
            rec = deref(P + 0x348) if P else None
            val = None
            if rec is not None:
                try:
                    val = p.i32(rec + 0x108) / 100000.0
                except OSError:
                    val = None
            records[pid] = rec
            print("%-7d 0x%-14X 0x%-14X %-16s %s"
                  % (pid, obj, P or 0,
                     ("0x%X" % rec) if rec else "None",
                     ("%.5f" % val) if val is not None else "None"))

        # distinctness check
        recs = [r for r in records.values() if r]
        print("\ndistinct record pointers: %d distinct / %d provinces"
              % (len(set(recs)), len(recs)))
        if len(set(recs)) == len(recs):
            print("  => sub-records are PROVINCE-LOCAL (no aliasing). Good.")
        else:
            print("  => SOME provinces SHARE a record pointer (aliasing risk!).")


if __name__ == "__main__":
    main()
