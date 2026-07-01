"""country_misc_live.py - LIVE per-country COUNTRY-MISC scalars (read-only).

Reads, straight from a running hoi4.exe (no save needed at runtime), the
per-country misc scalars pinned on build 16022851 (see
country_misc_offsets.draft.json):

    convoys_destroyed   country_obj +0x1334  u32   (lifetime enemy convoys sunk)
    accidents_score     country_obj +0x14E8  i32/1e5 fixed-point
    ace_count           country_obj +0x12E4  u32   (number of ace records)
    cosmetic_tag        country_obj +0x1488  MSVC std::string (empty if none)

Country object = heap struct at [country_record + 0x08] (same object that holds
manpower/num_ships/stability). Index->TAG is bridged via political_power as a
Rosetta stone against a melted save (the same proven bridge as economy_offsets).

LOCATING THE ARRAY (important): on the 1946 validation frame, locate.locate()'s
owner-pointer liveness check returned NOT FOUND (the documented stale-ARENA /
custom-container signature). So this reader locates the array by the robust
UNIQUE-political_power method: scan the ASLR-relocated arena (fastscan.discover_arena)
for political_power i32 values that are globally unique in the bridge save; each
lands at exactly one record+0x20, all congruent to one base at the 0x120 stride.
That base is then re-validated (clean index run 0..N, names resolve) before use.

BUILD-GATED: refuses to run unless the live exe SHA-256 matches the mapped build,
unless --force is given (same fail-closed gate as locate.py).

READ-ONLY: PROCESS_VM_READ | PROCESS_QUERY_INFORMATION only; ReadProcessMemory +
VirtualQueryEx; no writes, no injection, game untouched. A 2-read sentinel guards
each scalar against mid-read mutation.

Usage:
    python country_misc_live.py                 # locate + dump all countries
    python country_misc_live.py --validate SAVE_TAG_FILE.json_dir
        (validate live == save: pass a dir holding economy.json,
         country_snapshot.json, country_extra.json from the matching save)
"""

import argparse
import json
import os
import struct
import sys

_THIS = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, _THIS)
sys.path.insert(0, os.path.dirname(_THIS))

import winmem
import locate as L
from memory_reader import fastscan as fs

EXPECTED_SHA = "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120"

REC = 0x120
OFF_NAMEPTR = 0x08
OFF_PP = 0x20
OFF_INDEX = 0x5D
PP_SCALE = 100000.0

# --- pinned country-MISC offsets (country_obj-relative), build 16022851 ---
OFF_ACE_COUNT = 0x12E4          # u32
OFF_CONVOYS_DESTROYED = 0x1334  # u32 (= num_ships @ 0x1330 + 4)
OFF_COSMETIC_TAG = 0x1488       # MSVC std::string
OFF_ACCIDENTS_SCORE = 0x14E8    # i32 / 1e5
# proven sibling offsets reused for the liveness re-check:
OFF_MANPOWER = 0x338            # u32
OFF_NUM_SHIPS = 0x1330          # i32


class LocateError(Exception):
    """A genuine country-array LOCATE failure (no live bridge AND the save-PP anchor
    relocate could not prove a base). It subclasses Exception (NOT BaseException/SystemExit)
    so the reader-plug wrapper catches it and HONESTLY OMITS that one domain (Absent, no
    fabrication) instead of letting a BaseException escape and kill the entire capture walk.
    CLI entry points convert it back to a process exit (so standalone fatal-abort is preserved)."""


# ==========================================================================
# build gate
# ==========================================================================
def check_build_gate(p, force=False):
    got = p.fingerprint().get("sha256")
    if got == EXPECTED_SHA:
        print("  build gate: exe SHA-256 matches the mapped build (%s...)" % got[:12])
        return
    msg = ("  build gate: live exe SHA-256 %s != mapped build %s; offsets are "
           "build-specific, reading blind risks SILENT MISREADS." % (got, EXPECTED_SHA))
    if force:
        print(msg + "\n  --force given: continuing (results UNVERIFIED).")
    else:
        raise SystemExit(msg + "\n  Refusing (pass --force to override).")


# ==========================================================================
# locating the array via unique political_power anchors
# ==========================================================================
def _m_i32_eq(base, blob, target_i32):
    import numpy as np
    a = np.frombuffer(blob, dtype="<i4", count=len(blob) // 4)
    idxs = np.nonzero(a == target_i32)[0]
    return [base + int(i) * 4 for i in idxs]


def _save_pp_index(save_countries):
    """political_power per TAG -> {pp_value_rounded_0.1: [tags]} for the Rosetta
    BRIDGE (live pp/1e5 vs save match tolerance ~0.1)."""
    by_pp = {}
    for tag, c in save_countries.items():
        v = c.get("political_power")
        if v:
            by_pp.setdefault(round(v, 1), []).append(tag)
    return by_pp


def _scan_arena(p, ar, tgt_i32):
    """Scan the arena for an i32 value. Parallel first; fall back to single-process
    if multiprocessing can't spawn (e.g. handle-duplication denied in a sandbox)."""
    matcher = ("memory_reader.country_misc_live", "_m_i32_eq")
    try:
        return set(fs.parallel_scan(p.pid, ar[0], ar[1], matcher=matcher,
                                    matcher_args=(tgt_i32,), item_size=4))
    except Exception as e:
        print("    (parallel scan unavailable: %s -> single-process fallback)"
              % type(e).__name__)
        return set(fs.single_scan(p.pid, ar[0], ar[1], matcher=matcher,
                                  matcher_args=(tgt_i32,), item_size=4))


def locate_via_unique_pp(p, save_countries, n_probe=8):
    """Return (rec0, count) by scanning for globally-unique PP i32 anchors.

    Robust when locate.locate()'s owner check fails (stale-ARENA). Picks the
    n_probe most distinctive (largest, globally-unique-in-save) PP values, scans
    the arena for each, and resolves the common array base.
    """
    # Use FULL-PRECISION political_power as the scan target (raw i32 = round(pp*1e5)),
    # NOT the 0.1-rounded bridge key - the exact in-memory i32 is what we match.
    by_pp_full = {}
    for tag, c in save_countries.items():
        v = c.get("political_power")
        if v:
            by_pp_full.setdefault(round(v * PP_SCALE), []).append(tag)
    uniq = sorted([(tags[0], raw) for raw, tags in by_pp_full.items()
                   if len(tags) == 1],
                  key=lambda tr: -abs(tr[1]))
    probes = uniq[:n_probe]
    if not probes:
        raise LocateError("no globally-unique political_power anchor in the save")
    ar = fs.discover_arena(p.pid)
    if ar is None:
        raise LocateError("discover_arena failed")
    print("  arena: 0x%X-0x%X (%.2f GB)" % (ar[0], ar[1], (ar[1]-ar[0]) / 2**30))

    # Scan probes until we have a few distinctive anchors (>=1 seed with <=3 hits
    # plus a couple of cross-checks). Each scan is one arena pass, so stop early.
    all_hits = {}
    seeds_small = 0
    for tag, raw in probes:
        tgt = struct.unpack("<i", struct.pack("<i", raw))[0]
        hits = _scan_arena(p, ar, tgt)
        all_hits[tag] = hits
        print("    probe %-4s pp_raw=%d -> %d hit(s)" % (tag, raw, len(hits)))
        if len(hits) <= 3:
            seeds_small += 1
        if seeds_small >= 1 and len(all_hits) >= 3:
            break

    # The most distinctive probe (fewest hits) seeds the base candidates.
    seed_tag = min(all_hits, key=lambda t: len(all_hits[t]))
    cand_bases = set()
    for ha in all_hits[seed_tag]:
        rec = ha - OFF_PP
        try:
            gi = p.u16(rec + OFF_INDEX)
        except OSError:
            continue
        if gi > 1000:
            continue
        cand_bases.add(rec - gi * REC)

    best = None
    for rec0 in cand_bases:
        # clean index run 0..24
        ok = True
        for k in range(24):
            try:
                if p.u16(rec0 + k * REC + OFF_INDEX) != k:
                    ok = False
                    break
            except OSError:
                ok = False
                break
        if not ok:
            continue
        count, trunc = L.count_records(p, rec0, L.MAX_COUNT)
        if trunc or not (L.MIN_COUNT <= count <= L.MAX_COUNT):
            continue
        # how many probe PP values resolve to some record+0x20 in [0,count)?
        present = 0
        for tag, hits in all_hits.items():
            if any((rec0 + i * REC + OFF_PP) in hits for i in range(count)):
                present += 1
        score = present - (1 if trunc else 0)
        if best is None or score > best[0]:
            best = (score, rec0, count)
    if best is None:
        raise LocateError("no clean-index array base resolved from the PP anchors")
    return best[1], best[2]


def bridge_index_to_tag(p, rec0, count, save_countries):
    """index -> TAG via unique political_power (the Rosetta bridge)."""
    by_pp = _save_pp_index(save_countries)
    idx_tag = {}
    for i in range(count):
        rec = rec0 + i * REC
        try:
            if p.u16(rec + OFF_INDEX) != i:
                continue
            pp = p.i32(rec + OFF_PP) / PP_SCALE
        except OSError:
            continue
        if abs(pp) < 1e-6:
            continue
        tags = by_pp.get(round(pp, 1))
        if tags and len(tags) == 1:
            idx_tag[i] = tags[0]
    return idx_tag


# ==========================================================================
# field reads (2-read sentinel)
# ==========================================================================
def _sentinel(read_fn):
    """Read twice; return value only if both reads agree, else None."""
    try:
        a = read_fn()
        b = read_fn()
    except OSError:
        return None
    return a if a == b else None


def read_country_misc(p, obj):
    """Return the misc scalar dict for one country object."""
    out = {}
    out["ace_count"] = _sentinel(lambda: p.u32(obj + OFF_ACE_COUNT))
    out["convoys_destroyed"] = _sentinel(lambda: p.u32(obj + OFF_CONVOYS_DESTROYED))
    acc = _sentinel(lambda: p.i32(obj + OFF_ACCIDENTS_SCORE))
    out["accidents_score"] = None if acc is None else acc / PP_SCALE
    ct = L.read_msvc_str(p, obj + OFF_COSMETIC_TAG, maxlen=64)
    out["cosmetic_tag"] = ct or None
    return out


def enumerate_country_misc(p, rec0, count, idx_tag):
    """Yield (tag, index, misc-dict) for every bridged country."""
    for i in range(count):
        tag = idx_tag.get(i)
        if not tag:
            continue
        try:
            obj = p.u64(rec0 + i * REC + OFF_NAMEPTR)
        except OSError:
            continue
        yield tag, i, read_country_misc(p, obj)


# ==========================================================================
# CLI
# ==========================================================================
def _load_save_countries(out_dir):
    """Merge the save oracle JSONs into {tag: {...fields...}} for bridge+validate."""
    def load(name):
        with open(os.path.join(out_dir, name), "r", encoding="utf-8") as fh:
            return json.load(fh)
    econ = load("economy.json").get("countries", {})
    extra = load("country_extra.json").get("countries", {})
    merged = {}
    for tag in set(econ) | set(extra):
        e = econ.get(tag, {}) or {}
        x = extra.get(tag, {}) or {}
        merged[tag] = {
            "political_power": e.get("political_power"),
            "convoys_destroyed": x.get("convoys_destroyed"),
            "accidents_score": x.get("accidents_score"),
            "ace_count": x.get("ace_count"),
            "cosmetic_tag": x.get("cosmetic_tag"),
        }
    return merged


def main():
    ap = argparse.ArgumentParser(description="LIVE per-country misc scalars (read-only).")
    ap.add_argument("--force", action="store_true",
                    help="run even if the exe SHA-256 doesn't match the mapped build")
    ap.add_argument("--save-dir", default=os.path.join(
        os.path.dirname(_THIS), "save_oracle", "output"),
        help="dir with the matching save's economy.json + country_extra.json "
             "(needed for the index->TAG bridge; default save_oracle/output)")
    ap.add_argument("--validate", action="store_true",
                    help="compare live reads to the save oracle and report match rate")
    args = ap.parse_args()

    save_countries = _load_save_countries(args.save_dir)

    p = winmem.open_process("hoi4.exe")
    if p is None:
        raise SystemExit("hoi4.exe not running")
    with p:
        check_build_gate(p, args.force)
        print("locating country array (unique-PP method)...")
        rec0, count = locate_via_unique_pp(p, save_countries)
        idx_tag = bridge_index_to_tag(p, rec0, count, save_countries)
        print("  rec0=0x%X count=%d bridged=%d" % (rec0, count, len(idx_tag)))

        rows = list(enumerate_country_misc(p, rec0, count, idx_tag))
        if not args.validate:
            print("\n%-4s %6s %8s %10s  %s" %
                  ("TAG", "aces", "convDst", "accidents", "cosmetic_tag"))
            for tag, i, m in sorted(rows):
                print("%-4s %6s %8s %10s  %s" % (
                    tag, m["ace_count"], m["convoys_destroyed"],
                    None if m["accidents_score"] is None else round(m["accidents_score"], 5),
                    m["cosmetic_tag"] or ""))
            print("\ncountries read: %d" % len(rows))
            return

        # validation
        fields = ["ace_count", "convoys_destroyed", "accidents_score", "cosmetic_tag"]
        ok = {f: 0 for f in fields}
        tot = {f: 0 for f in fields}
        bad = {f: [] for f in fields}
        for tag, i, m in rows:
            sv = save_countries.get(tag, {})
            for f in fields:
                want = sv.get(f)
                got = m.get(f)
                if f == "accidents_score":
                    if want is None:
                        continue
                    tot[f] += 1
                    if got is not None and abs(got - want) < 1e-3:
                        ok[f] += 1
                    else:
                        bad[f].append((tag, got, want))
                elif f == "cosmetic_tag":
                    # both empty counts as match
                    tot[f] += 1
                    if (got or None) == (want or None):
                        ok[f] += 1
                    else:
                        bad[f].append((tag, got, want))
                else:
                    if want is None:
                        continue
                    tot[f] += 1
                    if got == want:
                        ok[f] += 1
                    else:
                        bad[f].append((tag, got, want))
        print("\n=== VALIDATION: live vs save ===")
        for f in fields:
            print("  %-18s %d/%d exact%s" % (
                f, ok[f], tot[f],
                "" if not bad[f] else "  bad(<=5): %s" % bad[f][:5]))


if __name__ == "__main__":
    # CLI fatal-abort semantics preserved: a library LocateError (downgraded from the old
    # SystemExit so the capture walk survives) is converted back to a non-zero process exit
    # here, so the standalone tool still stops loudly on a genuine locate failure.
    try:
        main()
    except LocateError as _e:
        raise SystemExit("fatal: %s" % _e)
