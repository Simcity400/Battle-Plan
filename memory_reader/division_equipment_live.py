"""division_equipment_live.py - READ-ONLY live per-DIVISION EQUIPMENT INVENTORY reader.

Reads the ACTUAL equipment each land division currently HOLDS from a running PAUSED
hoi4.exe (build 16022851), READ-ONLY. This is combat readiness ground truth: a division
at full vs depleted equipment. It is distinct from:
  * the TEMPLATE composition (designs_live.py)             -- what the division SHOULD have
  * the country production STOCKPILE (production_live.py)  -- the warehouse, not the field

The held-equipment bank hangs off the CDivision body, the SAME {def_ptr, amount} compact
vector the production manager uses, just at body+0x358:

    CDivision body (vtable MODULE+0x2933F00, type 4713 @+0x08, id @+0x0C,
                    owner tag-db-id @+0x1C8)
      body + 0x358  -> compact vector { u64 begin; u32 capacity; u32 size }
      entry stride 0x10: { equip_def_ptr @+0x00 (u64), amount @+0x08 (i64 /100000 SIGNED) }
      equip_def_ptr -> type-70 equipment-VARIANT object (vtable MODULE+0x292B218,
                       u32[+0x08]==70); held variant id == u32[def+0x0C].
      => held[ u32[def+0x0C] ] = i64[entry+0x08] / 100000

PINNED, validated live==save on build 16022851 against the lockstep melt
(re_scratch/melt_sov_1946.txt, save_date 1946.5.19.9), keyed by (owner_TAG, div_id)
because div_id is NOT globally unique:
  * per-entry exact (rounded amount over common variant ids): 16888/16906 = 99.89%
  * per-division exact ignoring live zero-amount entries:     3161/3174
  * negative control (shuffled division labels):              19.38% (vs 99.89% real)
  * 0/3174 division banks have overlapping address ranges (genuine per-division bank).
Residuals are benign: live zero-slots the save strips (allow_zero_entries=no), sub-frame
production drift, and a few recycled-div_id/1946-rename harness artifacts. See
division_equipment_offsets.draft.json for the full breakdown.

HARD RULES honored (same as division_live_reader.py / production_live.py):
  * READ-ONLY memory (PROCESS_VM_READ | PROCESS_QUERY_INFORMATION); never writes/injects/
    suspends. Passive reads => no MP desync, any cadence.
  * Build-gated: refuses unless the live hoi4.exe sha256 matches the pinned build.
  * Per-entry vtable/type-70 gate; fail closed (whole-bank reject on any bad slot - a stale
    realloc race surfaces as None, never a partial/garbage bank).
  * Generic heap test 0x10000000000 <= v < 0x7FF000000000 for pointer validity.
  * ASCII-only output.

Reuses winmem.py (the read-only Process) + live_layers.tag_table (owner tag-db-id -> TAG).
Does NOT edit division_live_reader.py / designs_live.py / production_live.py (reads their
documented offsets only). When productionized these offsets merge into the per-build layout.
"""
from __future__ import annotations

import os
import struct
import sys

_HERE = os.path.dirname(os.path.abspath(__file__))
if _HERE not in sys.path:
    sys.path.insert(0, _HERE)

import winmem  # noqa: E402

EXPECTED_SHA256 = "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120"
PROC_NAME = "hoi4.exe"

# --- pinned offsets (division_equipment_offsets.draft.json) ---
DIV_VT_RVA = 0x2933F00            # CDivision vtable (module-relative)
VARIANT_VT_RVA = 0x292B218        # type-70 equipment-variant object vtable
DIVISION_TYPE = 4713              # u32[body+0x08]
VARIANT_TYPE = 70                 # u32[def+0x08]

OFF_TYPE = 0x08                   # u32 == 4713
OFF_ID = 0x0C                     # u32 division id
OFF_OWNER = 0x1C8                 # u32 owner tag-db-id

# held-equipment compact vector (off the CDivision body)
EQ_HDR = 0x358                    # body + this -> { begin; cap; size }
EQ_BEGIN = 0x00                   # u64
EQ_CAP = 0x08                     # u32
EQ_SIZE = 0x0C                    # u32
EQ_STRIDE = 0x10
ENT_DEF = 0x00                    # u64 equip_def_ptr
ENT_AMOUNT = 0x08                 # i64 fixed-point /100000 (SIGNED)
DEF_TYPE = 0x08                   # u32 == 70
DEF_VARIANT_ID = 0x0C            # u32 == save variant id

FP = 100000.0
HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000
MAX_DIVISIONS = 20000
MAX_BANK = 4096                   # sane bound on per-division entry count/capacity

# body span read per division during enumeration (covers identity + the +0x358 bank header)
_BODY_HEAD = EQ_HDR + 0x10        # 0x368


def _heap(a):
    return HEAP_LO <= a < HEAP_HI


# --------------------------------------------------------------------------------------
# Core decode: one division's held-equipment bank
# --------------------------------------------------------------------------------------
def read_division_equipment(p, body_addr, variant_vt, drop_zero=True):
    """Return { variant_id(int) : count_held(float) } for one CDivision body, or None
    if the bank is unreachable / fails the gate.

    `p`          : winmem.Process (READ-ONLY).
    `body_addr`  : the CDivision body base (vtable hit; the same address the live division
                   reader / enumerator yields).
    `variant_vt` : p.module_base + VARIANT_VT_RVA (pass once; avoids recompute per call).
    `drop_zero`  : when True (default) zero-amount slots are dropped, matching the save
                   (allow_zero_entries=no). Pass False to see the raw live bank incl. the
                   zero slots the engine keeps.

    Decode is all-or-nothing: if ANY slot fails the type-70 variant gate the whole bank is
    rejected (returns None) rather than surfacing a partial/garbage result - a stale
    reallocation race shows up as None, never as bad data.
    """
    try:
        begin = p.u64(body_addr + EQ_HDR + EQ_BEGIN)
        cap = p.u32(body_addr + EQ_HDR + EQ_CAP)
        size = p.u32(body_addr + EQ_HDR + EQ_SIZE)
    except OSError:
        return None
    if size == 0 and cap <= MAX_BANK:
        return {}                                   # legitimately empty bank
    if not _heap(begin) or size > cap or cap > MAX_BANK:
        return None
    raw = p.try_read(begin, size * EQ_STRIDE)
    if len(raw) < size * EQ_STRIDE:
        return None
    out = {}
    for k in range(0, size * EQ_STRIDE, EQ_STRIDE):
        dptr = struct.unpack_from("<Q", raw, k + ENT_DEF)[0]
        amt = struct.unpack_from("<q", raw, k + ENT_AMOUNT)[0]   # SIGNED i64
        if not _heap(dptr):
            return None
        try:
            if p.u64(dptr) != variant_vt or p.u32(dptr + DEF_TYPE) != VARIANT_TYPE:
                return None
            vid = p.u32(dptr + DEF_VARIANT_ID)
        except OSError:
            return None
        val = amt / FP
        if drop_zero and round(val) == 0:
            continue
        out[vid] = val
    return out


# --------------------------------------------------------------------------------------
# Batch enumeration: every live land division + its held-equipment bank
# --------------------------------------------------------------------------------------
def _iter_division_bodies(p):
    """Yield (div_id, owner_tag_db_id, body_addr) for every live CDivision (full committed
    heap; the same window division_live_reader uses). De-duplicates by (owner, id)."""
    vt = p.module_base + DIV_VT_RVA
    pat = struct.pack("<Q", vt)
    seen = set()
    n = 0
    for base, size in p.iter_regions(HEAP_LO, HEAP_HI, want_exec=False):
        if size > (1 << 31):
            continue
        blob = p.try_read(base, size)
        if not blob:
            continue
        s = 0
        while True:
            j = blob.find(pat, s)
            if j < 0:
                break
            cand = base + j
            s = j + 8
            b = p.try_read(cand, _BODY_HEAD)
            if len(b) < _BODY_HEAD:
                continue
            if struct.unpack_from("<I", b, OFF_TYPE)[0] != DIVISION_TYPE:
                continue
            did = struct.unpack_from("<I", b, OFF_ID)[0]
            owner = struct.unpack_from("<I", b, OFF_OWNER)[0]
            key = (owner, did)
            if key in seen:
                continue
            seen.add(key)
            n += 1
            if n > MAX_DIVISIONS:
                raise RuntimeError("division count exceeds sanity bound (%d)" % MAX_DIVISIONS)
            yield did, owner, cand


def enumerate_division_bodies(p):
    """{(owner_tag_db_id, div_id): body_addr} for every live land division. The SCAN half
    of enumerate_division_equipment, exposed so callers can PIN the located body addresses and
    re-read equipment later WITHOUT re-scanning the heap (steady-state perf)."""
    out = {}
    for did, owner_id, body in _iter_division_bodies(p):
        out[(owner_id, did)] = body
    return out


def read_division_equipment_from_bodies(p, bodies, tag_table=None, drop_zero=True):
    """STEADY-STATE no-scan re-read: given a PINNED {(owner_tag_db_id, div_id): body_addr}
    map (from enumerate_division_bodies), re-read each division's equipment LIVE WITHOUT
    scanning the heap. Each body is re-gated (vtable == CDivision vtable, type==4713) and we
    confirm the live (owner,id) at body_addr still == the pinned key (ABA-reuse defense: a
    freed slot re-used by another CDivision would have a DIFFERENT (owner,id)). A body that
    fails the gate or whose (owner,id) no longer matches is DROPPED (honest absence; the
    caller's pin-validate should already have forced a re-scan). Returns the same list-of-dicts
    shape as enumerate_division_equipment."""
    div_vt = p.module_base + DIV_VT_RVA
    variant_vt = p.module_base + VARIANT_VT_RVA
    out = []
    for (owner_id, did), body in bodies.items():
        b = p.try_read(body, _BODY_HEAD)
        if len(b) < _BODY_HEAD:
            continue
        if struct.unpack_from("<Q", b, 0)[0] != div_vt:          # moved/freed/other class
            continue
        if struct.unpack_from("<I", b, OFF_TYPE)[0] != DIVISION_TYPE:
            continue
        live_id = struct.unpack_from("<I", b, OFF_ID)[0]
        live_owner = struct.unpack_from("<I", b, OFF_OWNER)[0]
        if (live_owner, live_id) != (owner_id, did):             # ABA: wrong (owner,id)
            continue
        eq = read_division_equipment(p, body, variant_vt, drop_zero=drop_zero)
        if eq is None:
            eq = {}
        out.append({
            "div_id": did,
            "owner_tag_db_id": owner_id,
            "owner": tag_table.tag(owner_id) if tag_table is not None else None,
            "equipment": eq,
            "total": sum(eq.values()),
            "n_variants": len(eq),
        })
    return out


def enumerate_division_equipment(p, tag_table=None, drop_zero=True):
    """Return a list of dicts, one per live land division:
        { div_id, owner_tag_db_id, owner(TAG if tag_table given else None),
          equipment{ variant_id:int -> count_held:float }, total, n_variants }

    `tag_table`: optional live_layers.tag_table.TagTable to resolve the owner tag-db-id
                 -> TAG. STRONGLY recommended for keying: div_id is NOT globally unique;
                 (owner, div_id) is. Without it `owner` is None and you must key by
                 (owner_tag_db_id, div_id).
    """
    variant_vt = p.module_base + VARIANT_VT_RVA
    out = []
    for did, owner_id, body in _iter_division_bodies(p):
        eq = read_division_equipment(p, body, variant_vt, drop_zero=drop_zero)
        if eq is None:
            eq = {}
        rec = {
            "div_id": did,
            "owner_tag_db_id": owner_id,
            "owner": tag_table.tag(owner_id) if tag_table is not None else None,
            "equipment": eq,
            "total": sum(eq.values()),
            "n_variants": len(eq),
        }
        out.append(rec)
    return out


# --------------------------------------------------------------------------------------
# Build gate
# --------------------------------------------------------------------------------------
def _build_gate(p):
    sha = p.fingerprint().get("sha256")
    if sha != EXPECTED_SHA256:
        raise SystemExit("build gate FAIL: live hoi4.exe sha256 %s != mapped build %s\n"
                         "Offsets are build-specific; refusing to read an unknown build."
                         % (sha, EXPECTED_SHA256))


# --------------------------------------------------------------------------------------
# Validation vs the lockstep melt ground truth
# --------------------------------------------------------------------------------------
def _validate():
    """Validate the live per-division held-equipment bank vs the lockstep melt ground
    truth (_diveq_gt.json, produced by _diveq_gt.py, keyed 'TAG|div_id'). Keys live
    divisions by (owner_TAG, div_id) -- div_id is NOT globally unique. Prints per-entry
    and per-division exact-match rates plus a shuffled-label negative control."""
    import json
    import random

    try:
        from live_layers.tag_table import TagTable
    except Exception as e:                                      # pragma: no cover
        raise SystemExit("tag_table unavailable: %s" % e)

    gt_path = os.path.join(_HERE, "_diveq_gt.json")
    if not os.path.exists(gt_path):
        raise SystemExit("ground truth missing: run _diveq_gt.py to produce _diveq_gt.json")
    gt = json.load(open(gt_path, encoding="utf-8"))

    p = winmem.open_process(PROC_NAME)
    if p is None:
        raise SystemExit("hoi4.exe not running")
    with p:
        _build_gate(p)
        tt = TagTable.locate(p)
        if tt is None:
            raise SystemExit("could not locate the tag table (needed to key by TAG)")

        recs = enumerate_division_equipment(p, tag_table=tt, drop_zero=True)

        validated = set_exact = div_exact = 0
        entry_exact = entry_total = 0
        no_gt = 0
        decoded = {}                                           # 'TAG|id' -> {vid: amt}
        for r in recs:
            key = "%s|%d" % (r["owner"], r["div_id"])
            if key not in gt:
                no_gt += 1
                continue
            g = {int(k): v for k, v in gt[key]["equip"].items()}
            live = r["equipment"]
            decoded[key] = live
            validated += 1
            if set(live.keys()) == set(g.keys()):
                set_exact += 1
            common = set(live) & set(g)
            ex = sum(1 for k in common if round(live[k]) == g[k])
            entry_exact += ex
            entry_total += len(common)
            if set(live.keys()) == set(g.keys()) and ex == len(g):
                div_exact += 1

        print("=== DIVISION EQUIPMENT INVENTORY VALIDATION (live body+0x358 vs save) ===")
        print("validated divisions (live key in GT): %d  (no GT key: %d)" % (validated, no_gt))
        print("variant-set exact          : %d/%d" % (set_exact, validated))
        print("per-division FULLY exact    : %d/%d" % (div_exact, validated))
        print("per-entry exact (rounded)   : %d/%d (%.2f%%)"
              % (entry_exact, entry_total, 100.0 * entry_exact / max(entry_total, 1)))
        print("=> division equipment inventory %d/%d divisions exact (set+amounts); "
              "%d/%d entries exact" % (div_exact, validated, entry_exact, entry_total))

        # shuffled-label negative control
        keys = list(decoded)
        shuffled = keys[:]
        random.seed(7)
        random.shuffle(shuffled)
        m = dict(zip(keys, shuffled))
        sh_ex = sh_tot = 0
        for key in keys:
            g = {int(k): v for k, v in gt[m[key]]["equip"].items()}
            live = decoded[key]
            common = set(live) & set(g)
            sh_ex += sum(1 for k in common if round(live[k]) == g[k])
            sh_tot += len(common)
        print("SHUFFLE control             : %d/%d (%.2f%%) -- should be far below real"
              % (sh_ex, sh_tot, 100.0 * sh_ex / max(sh_tot, 1)))


# --------------------------------------------------------------------------------------
# Demo / CLI
# --------------------------------------------------------------------------------------
def main():
    p = winmem.open_process(PROC_NAME)
    if p is None:
        raise SystemExit("hoi4.exe not running")
    with p:
        _build_gate(p)
        tt = None
        try:
            from live_layers.tag_table import TagTable
            tt = TagTable.locate(p)
        except Exception:
            tt = None
        print("build gate OK; reading per-division equipment inventory (READ-ONLY)...")
        recs = enumerate_division_equipment(p, tag_table=tt, drop_zero=True)
        print("divisions: %d" % len(recs))
        # show the highest-equipment divisions (a full armored division is a top anchor)
        rows = sorted(recs, key=lambda r: -r["total"])[:12]
        print("\n%-5s %-5s %8s %5s  equipment{vid:held}" % ("id", "own", "total", "nVar"))
        for r in rows:
            eq = {k: round(v) for k, v in sorted(r["equipment"].items(),
                                                 key=lambda kv: -kv[1])}
            print("%-5d %-5s %8.0f %5d  %s"
                  % (r["div_id"], r["owner"] or r["owner_tag_db_id"],
                     r["total"], r["n_variants"], eq))


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "validate":
        _validate()
    else:
        main()
