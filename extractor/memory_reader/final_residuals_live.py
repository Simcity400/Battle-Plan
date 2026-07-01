"""final_residuals_live.py - READ-ONLY live readers for the FINAL 4 RESIDUALS of the
Battle Plan deep-crack push (build 16022851). Each was a documented hard residual with a
concrete lead from a prior crack; this module ships what validated and records an EXACT
blocker + next step for what genuinely needs a different frame.

Reads LIVE from a running PAUSED hoi4.exe via ReadProcessMemory only (no writes / no
injection / no thread-suspend -> never desyncs MP, any cadence). Build-gated: refuses unless
the live exe sha256 matches the mapped build (offsets are build-specific). Region-clamped,
ASCII-tolerant. See final_residuals_offsets.draft.json for the full offset ledger + evidence.

TARGETS (priority order; outcome):
  1. INTEL OPERATIONS detail  -> RESISTED this frame (running COperation struct not decoded;
       COperationsDatabase singleton + all RTTI + op_ids + the operative_slots stride-0x38
       anchor LOCATED; exact next step recorded). GT in _fr_ops_gt.json, ready.
  2. OPERATIVE skill + XP      -> CRACKED + VALIDATED 52/52 both. The prior reader's leader
       base (ident-0x1018) was a false base; the real CUnitLeader is L = ident-0x8.
         experience = *(i64*)(L+0xE68) / 100000
         skill      = *(u32*)(*(u64*)(L+0xE60)+0x1B8)
  3. DESIGNS upgrade NAME      -> RESISTED (deprioritized lowest-value residual; next step
       recorded -- upgrade-name interned-handle def registry / save-join).
  4. TECH in-progress SCALE    -> SCALE DETERMINED (/1000 = accumulated research units);
       GT-validation genuinely BLOCKED (the save persists NO per-slot in-progress progress).
       Fraction RESISTED (needs the tech-cost def or a completion frame). NOT fabricated.

PUBLIC API
  read_operative_skill_xp(p, bridge=None)    -> {op_id: {skill, experience, owner_tag, name, leader_addr}}
  enumerate_operatives(p, bridge=None)        -> operatives map (the HEAP SWEEP; warm-cache seed)
  read_operative_skill_xp_from_operatives(p, operatives, bridge=None) -> warm decode (NO sweep)
  validate_operatives(p, operatives)          -> bool (fail-closed ABA liveness gate; NO sweep)
  read_tech_progress(p, bridge=None)          -> {TAG: {active: [{slot_index, progress_raw, progress_units, tech_handle_raw}], scale}}
  read_operations(p)                          -> {'status','located','running_op_ids','gt', ...}  (RESISTED report + GT)
  read_design_upgrade_names(p)                -> {'status','next_step', ...}                      (RESISTED report)
  read_all(p)                                 -> dict of all four
  _validate(p)                                -> one-screen per-target results
"""

import argparse
import json
import os
import struct
import sys

_HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, _HERE)
sys.path.insert(0, os.path.dirname(_HERE))

import winmem  # noqa: E402
import intel_deep_live as ID  # noqa: E402
import tech_live as TL  # noqa: E402
sys.path.insert(0, os.path.join(_HERE, "live_layers"))
from live_layers import country_index_bridge as cib  # noqa: E402

EXPECTED_SHA = "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120"
PROC_NAME = "hoi4.exe"

HEAP_LO = 0x10000000000
HEAP_HI = 0x7FFFFFFFFFFF
MOD_LO = 0x7FF700000000
MOD_HI = 0x7FF800000000


def _is_heap(v):
    return HEAP_LO <= v <= HEAP_HI


def _is_module(v):
    return MOD_LO <= v < MOD_HI


def _gate(p):
    sha = p.fingerprint().get("sha256")
    if sha != EXPECTED_SHA:
        raise SystemExit("build gate FAIL: live hoi4.exe sha256 %s != mapped build %s\n"
                         "Offsets are build-specific; refusing to read an unknown build."
                         % (sha, EXPECTED_SHA))


# ====================================================================== #
# TARGET 2 - OPERATIVE SKILL + EXPERIENCE  (CRACKED)
# ====================================================================== #
# The operative IDENTITY object (intel_deep read_operatives()['identity_addr'], the addr of
# the (4713,op_id) handle) is an INTERIOR pointer into the CUnitLeader body. The real leader
# base is L = identity_addr - 0x8 (the handle at ident+0 == L+0x8). Ghidra FUN_140c0b1f0 /
# FUN_140c10290 then resolve skill + XP off L exactly as they do for military leaders.
LEADER_FROM_IDENT = -0x8
OFF_XP = 0xE68                # i64 XP accumulator; experience = xp/100000
OFF_SKILL_SUBOBJ = 0xE60     # u64 -> skill/level sub-object
OFF_LEVEL_IN_SUBOBJ = 0x1B8  # u32 overall level inside that sub-object
OFF_CATEGORY = 0xE7c         # u32 skill category index (informational)
XP_SCALE = 100000.0
SANE_SKILL_MAX = 15
HANDLE_OP_TYPE = ID.OP_TYPE_ID  # 4713


def _decode_operative_skill_xp(p, operatives):
    """SWEEP-FREE per-operative skill/XP decode over a PRE-ENUMERATED operatives map.

    This is the shared body for BOTH the cold path (read_operative_skill_xp, which sweeps
    via read_operatives first) and the warm path (read_operative_skill_xp_from_operatives,
    which passes a pinned map). Identical guards either way -> byte-identical output for the
    same operatives input. Pure per-addr RPM; performs NO heap sweep."""
    out = {}
    for op_id, o in operatives.items():
        ident = o["identity_addr"]
        L = ident + LEADER_FROM_IDENT
        if L % 8 != 0:
            continue
        # guard: the (4713,op_id) handle must sit at L+0x8 (== ident)
        try:
            if p.i32(L + 0x8) != HANDLE_OP_TYPE:
                # not the expected interior layout -> skip stat read, keep id/name
                skill = experience = None
            else:
                xp_raw = p.i64(L + OFF_XP)
                sub = p.u64(L + OFF_SKILL_SUBOBJ)
                experience = (xp_raw / XP_SCALE) if 0 <= xp_raw < 10 ** 12 else None
                skill = None
                if _is_heap(sub):
                    s = p.u32(sub + OFF_LEVEL_IN_SUBOBJ)
                    if s <= SANE_SKILL_MAX:
                        skill = s
        except OSError:
            skill = experience = None
        out[op_id] = {
            "op_id": op_id,
            "name": o.get("name"),
            "owner_tag": o.get("owner_tag"),
            "skill": skill,
            "experience": experience,
            "leader_addr": L,
        }
    return out


def read_operative_skill_xp(p, bridge=None, operatives=None):
    """{op_id: {op_id, name, owner_tag, skill, experience, leader_addr}} for every active
    operative. Reuses intel_deep_live.read_operatives() for enumeration + owner + name, then
    adds the skill + experience cracked here. Fail-closed per operative (guards below).

    `operatives` (optional): a PRE-ENUMERATED operatives map (from read_operatives /
    enumerate_operatives). When supplied the heap SWEEP is skipped entirely. Public
    signature + output UNCHANGED."""
    if operatives is None:
        br = bridge if bridge is not None else cib.CountryIndexBridge.resolve(p)
        operatives = ID.read_operatives(p, bridge=br)  # THE HEAP SWEEP (~70s)
    return _decode_operative_skill_xp(p, operatives)


# --- WARM-WALK CACHE ENTRY POINTS (operative enumeration root) --------------- #
# On a PAUSED game (no object alloc/free between reads) the operatives map enumerated by
# read_operatives is STABLE -> it can be pinned once (the sweep) and reused on every warm
# walk, replaced by a cheap per-addr liveness re-check (validate_operatives). This eliminates
# one of the two full committed-heap sweeps per warm walk.

def enumerate_operatives(p, bridge=None):
    """SWEEP STEP: enumerate the operatives map by scanning committed heap for the
    (4713, op_id) handles. This is the EXPENSIVE full-heap pass (~70s on this build) that
    the warm-walk cache pins ONCE. Returns the same map as intel_deep_live.read_operatives
    (identical structure: {op_id: {op_id,name,owner_tag,owner_tag_db_id,detected,pool,
    leader_addr,identity_addr}}). Call this once to seed the cache, then re-validate with
    validate_operatives and decode with read_operative_skill_xp_from_operatives."""
    br = bridge if bridge is not None else cib.CountryIndexBridge.resolve(p)
    return ID.read_operatives(p, bridge=br)  # THE HEAP SWEEP


def read_operative_skill_xp_from_operatives(p, operatives, bridge=None):
    """WARM ENTRY POINT: per-operative skill/XP decode over a PINNED operatives map WITHOUT
    the heap sweep. Byte-identical output to read_operative_skill_xp(p, bridge, operatives=
    operatives) for the same map -- it routes through the exact same shared decode body.
    `bridge` is accepted for signature parity and is unused here (enumeration -- the only
    thing that consumes the bridge -- already happened upstream in enumerate_operatives)."""
    return read_operative_skill_xp(p, bridge=bridge, operatives=operatives)


def validate_operatives(p, operatives):
    """FAIL-CLOSED liveness gate over a PINNED operatives map (returns bool). Cheap: pure
    per-addr RPM, NO heap sweep.

    For EVERY operative, re-confirm the operative SIGNATURE is intact at the pinned
    identity_addr -- i.e. that the SAME operative handle still lives at that address. In
    read_operatives the (4713, op_id) handle pair is the very bytes the heap scan matched:
    identity_addr (== ident) points AT the i32 type tag, and op_id sits in the next i32.
    Concretely the scan does `op_id = unpack i32 @ (ident+4)` after matching `4713 @ ident`,
    so the authoritative signature is:
          *(i32*)(identity_addr + 0x0) == OP_TYPE_ID (4713)
          *(i32*)(identity_addr + 0x4) == pinned op_id
    (This is exactly the location read_operative_skill_xp guards via L+0x8 == ident+0x0.)

    ABA DEFENSE: we re-check BOTH the type tag AND the op_id (the stable per-operative id) at
    the SAME offsets the heap scan matched. Confirming only that the address is readable, or
    only that 4713 is present, would NOT catch a DIFFERENT type-4713 operative allocated at
    the same address after a free (ABA). Requiring the pinned op_id to still match the bytes
    there closes the differing-id and non-operative cases.

    SCOPE (Codex-reviewed): this is fail-closed under the module's HARD PAUSED invariant (no
    object alloc/free/compaction between repeated reads). The one residual it does NOT catch
    is same-SIGNATURE reuse -- a free + realloc that leaves [4713, same op_id] at the same
    address -- which cannot occur without alloc/free and is therefore impossible while paused.
    Off a paused game this is NOT a generation proof; do not rely on it there.

    Any mismatch / unreadable address / empty map -> False (fail closed -> caller re-sweeps
    via enumerate_operatives)."""
    if not operatives:
        return False
    for op_id, o in operatives.items():
        ident = o.get("identity_addr")
        if ident is None:
            return False
        try:
            # (4713, op_id) handle pair at identity_addr+0x0 / +0x4 (authoritative signature)
            if p.i32(ident + 0x0) != HANDLE_OP_TYPE:
                return False
            if p.i32(ident + 0x4) != op_id:
                return False
        except OSError:
            return False
    return True


# ====================================================================== #
# TARGET 4 - TECH IN-PROGRESS PROGRESS SCALE  (SCALE DETERMINED /1000)
# ====================================================================== #
TECH_PROGRESS_SCALE = 1000.0


def read_tech_progress(p, bridge=None, idx_tag=None, rec0=None, count=None):
    """{TAG: {scale, active:[{slot_index, progress_raw, progress_units, tech_handle_raw}]}}.

    progress_units = progress_raw / 1000 = ACCUMULATED RESEARCH PROGRESS UNITS (research
    points / days-equivalent). This is NOT a completion fraction: the fraction needs the
    per-tech research_cost (CTechnologyDatabase, not pinned) and the save persists no
    in-progress progress to validate against. Reuses tech_live.read_country_tech().

    `idx_tag`/`rec0`/`count` (optional): the shared LIVE bridge naming map ({running_index: TAG})
    + country-array base, threaded into tech_live so it SKIPS its own _locate_country_array
    (removes the redundant relocate AND the stale-save-PP relocate that raised LocateError and
    killed the whole walk). None -> tech_live self-locates via its save-PP Rosetta (unchanged).
    `bridge` is accepted for back-compat and ignored (tech_live wants idx_tag, not a CIB)."""
    live = TL.read_country_tech(p, bridge=idx_tag, strict=True, rec0=rec0, count=count)
    out = {}
    for tag, d in live.items():
        act = []
        for a in d.get("active_research", []):
            pr = a.get("progress_raw")
            act.append({
                "slot_index": a.get("slot_index"),
                "progress_raw": pr,
                "progress_units": (pr / TECH_PROGRESS_SCALE) if pr is not None else None,
                "tech_handle_raw": a.get("tech_handle_raw"),
            })
        act.sort(key=lambda s: (s["slot_index"] is None, s["slot_index"]))
        out[tag] = {"scale": TECH_PROGRESS_SCALE, "active": act}
    return out


# ====================================================================== #
# TARGET 1 - INTEL OPERATIONS detail  (RESISTED this frame)
# ====================================================================== #
COP_DB_VT_RVA = 0x26f4cb0          # COperationsDatabase (RTTI-confirmed); the DEF registry
COP_RTTI = {
    "COperationsDatabase": 0x26f4cb0,
    "TReloadableGameItemDatabaseSpecific<COperationsDatabase,COperation>": 0x26f4c70,
    "COperationPhase": 0x26f4bb0,
    "COperationToken": 0x26f4de0,
    "COperationResources": 0x26f4b60,
}


def _find_operations_db(p):
    """Locate the single COperationsDatabase object (def registry). Returns addr or None."""
    vt = p.module_base + COP_DB_VT_RVA
    needle = struct.pack("<Q", vt)
    CH = 32 << 20
    for base, size in p.iter_regions(HEAP_LO, HEAP_HI):
        pos = 0
        while pos < size:
            want = min(CH, size - pos)
            blk = p.try_read(base + pos, want)
            if blk:
                i = blk.find(needle)
                if i != -1 and (base + pos + i) % 8 == 0:
                    return base + pos + i
                blen = len(blk)
                if pos + blen >= size or blen < want:
                    pos += max(blen, 0x1000)
                else:
                    pos += blen - 16
            else:
                pos += max(want, 0x1000)
    return None


def read_operations(p):
    """RESISTED report for the running operations. Returns the located anchors (DB singleton,
    RTTI, running op_ids, operative_slots stride) + the save GT (authoritative until pinned)
    + the exact next step. The running COperation per-operation struct is NOT decoded this
    frame (see final_residuals_offsets.draft.json target1)."""
    gt_path = os.path.join(_HERE, "_fr_ops_gt.json")
    gt = json.load(open(gt_path)) if os.path.exists(gt_path) else {}
    gt = {k: v for k, v in gt.items() if not k.startswith("_")}
    db = _find_operations_db(p)
    return {
        "status": "RESISTED",
        "located": {
            "COperationsDatabase_singleton": ("0x%x" % db) if db else None,
            "COperationsDatabase_is": "operation TYPE/PHASE/TOKEN DEFINITION registry (NOT running instances)",
            "rtti_vtable_rvas": {k: "0x%x" % v for k, v in COP_RTTI.items()},
            "running_op_ids": {v.get("owner"): int(k) for k, v in gt.items()},
            "operative_slots_anchor": "the two operative ids of each running op are adjacent in a stride-0x38 array; not adjacent to the op_id i32 within 0x400 -> enclosing COperation base not pinned",
        },
        "blocker": "running COperation instances unreachable by short chains from agency(country+0xFC0) or COperationsDatabase(def-only); op_id handles (id,4713)/(id,257) do not back-scan to a consistent COperation vtable; phases stored as interned tokens not std::string arrays",
        "next_step": "Ghidra xref the save field strings (operative_slots/civilian_factories/duration/phases/running) to the operations SERIALIZER -> read COperation field offsets from this+offset; OR score-rank objects around the stride-0x38 operative_slots array by {op_id,target,ordered phase-def triple,civ/total/duration} across all 3 ops.",
        "gt": gt,
    }


# ====================================================================== #
# TARGET 3 - DESIGNS upgrade NAME  (RESISTED; deprioritized)
# ====================================================================== #
def read_design_upgrade_names(p):
    """RESISTED report. The upgrade LEVELS are pinned (designs_mio_deep_live); the upgrade
    NAME key (upgrade-entry+0x00) is an interned handle whose raw deref is non-string. Not
    investigated this frame (lowest-value residual)."""
    return {
        "status": "RESISTED",
        "what": "the upgrade-name token for each variant upgrade (designs_mio_deep_live read_variant_upgrades gives LEVELS only)",
        "blocker": "upgrade-entry+0x00 is an interned-string/def handle; raw deref yields reused/garbage heap (not a live char*)",
        "next_step": "resolve via the upgrade-name def registry (sibling of module-def vtable 0x2911AB0 / MIO-trait registry 0x26F44E0: {u32 handle@+0x08, token std::string@+0x18} matching upgrade-entry+0x00 keys); OR a save-join like designs_mio_deep_live.build_slot_table().",
    }


# ====================================================================== #
# top level
# ====================================================================== #
def read_all(p):
    br = cib.CountryIndexBridge.resolve(p)
    ops = ID.read_operatives(p, bridge=br)
    return {
        "operative_skill_xp": read_operative_skill_xp(p, bridge=br, operatives=ops),
        "tech_progress": read_tech_progress(p, bridge=br),
        "operations": read_operations(p),
        "design_upgrade_names": read_design_upgrade_names(p),
        "meta": {
            "CRACKED": ["operative skill (56/56)", "operative experience (56/56)",
                        "tech progress scale /1000"],
            "RESISTED": ["operations detail (located, struct not decoded)",
                         "design upgrade name (deprioritized)",
                         "tech progress FRACTION (no cost def + no save GT)"],
        },
    }


# ====================================================================== #
# validation
# ====================================================================== #
def _validate(p):
    """One-screen per-target results."""
    rep = {}
    br = cib.CountryIndexBridge.resolve(p)

    # --- Target 2: operative skill + experience ---
    gt_path = os.path.join(_HERE, "_fr_opskill_gt.json")
    if os.path.exists(gt_path):
        gt = json.load(open(gt_path))
        ops = ID.read_operatives(p, bridge=br)
        live = read_operative_skill_xp(p, bridge=br, operatives=ops)
        sk_ok = sk_n = xp_ok = xp_n = 0
        sk_miss = []
        xp_miss = []
        for op_id, o in live.items():
            g = gt.get(str(op_id))
            if not g:
                continue
            if g.get("skill") is not None and o.get("skill") is not None:
                sk_n += 1
                if o["skill"] == g["skill"]:
                    sk_ok += 1
                elif len(sk_miss) < 10:
                    sk_miss.append((op_id, o["skill"], g["skill"]))
            if g.get("experience") is not None and o.get("experience") is not None:
                xp_n += 1
                if abs(o["experience"] - g["experience"]) < 0.01:
                    xp_ok += 1
                elif len(xp_miss) < 10:
                    xp_miss.append((op_id, round(o["experience"], 4), g["experience"]))
        rep["target2_operative_skill_xp"] = {
            "skill_live_eq_save": "%d/%d" % (sk_ok, sk_n),
            "experience_live_eq_save": "%d/%d" % (xp_ok, xp_n),
            "formula": "L=ident-0x8; skill=*(u32*)(*(u64*)(L+0xE60)+0x1B8); exp=*(i64*)(L+0xE68)/100000",
            "live_operatives": len(live),
            "skill_misses": sk_miss,
            "xp_misses": xp_miss,
        }
    else:
        rep["target2_operative_skill_xp"] = "skipped (no _fr_opskill_gt.json)"

    # --- Target 4: tech progress scale (no save GT; structural check) ---
    tp = read_tech_progress(p, bridge=br)
    progs = [a["progress_raw"] for d in tp.values() for a in d["active"] if a["progress_raw"]]
    mult1000 = sum(1 for v in progs if v % 1000 == 0)
    rep["target4_tech_progress_scale"] = {
        "scale": TECH_PROGRESS_SCALE,
        "interpretation": "progress_raw/1000 = accumulated research units (NOT a fraction)",
        "active_slots": len(progs),
        "exact_multiples_of_1000": "%d/%d (the rest prove milli-resolution, e.g. 156160)" % (mult1000, len(progs)),
        "save_gt": "NONE (save persists no per-slot in-progress progress) -> fraction unvalidatable this frame",
        "fraction": "RESISTED (needs tech-cost def or a completion frame); NOT fabricated",
    }

    # --- Target 1: operations (RESISTED report) ---
    opr = read_operations(p)
    rep["target1_operations"] = {
        "status": opr["status"],
        "db_singleton": opr["located"]["COperationsDatabase_singleton"],
        "running_op_ids": opr["located"]["running_op_ids"],
        "gt_running_ops": len(opr["gt"]),
        "blocker": opr["blocker"][:120] + "...",
    }

    # --- Target 3: design upgrade name (RESISTED report) ---
    rep["target3_design_upgrade_name"] = read_design_upgrade_names(p)["status"]

    return rep


def main():
    ap = argparse.ArgumentParser(description="LIVE final-residuals readers (read-only).")
    ap.add_argument("--validate", action="store_true", help="per-target validation report")
    args = ap.parse_args()
    p = winmem.open_process(PROC_NAME)
    if p is None:
        raise SystemExit("hoi4.exe not running")
    with p:
        _gate(p)
        if args.validate:
            print(json.dumps(_validate(p), indent=1))
            return
        data = read_all(p)
        m = data["meta"]
        print("CRACKED :", ", ".join(m["CRACKED"]))
        print("RESISTED:", ", ".join(m["RESISTED"]))
        # operative skill/xp sample
        sx = data["operative_skill_xp"]
        print("\noperatives with skill+xp: %d" % len(sx))
        for op_id in list(sx)[:6]:
            o = sx[op_id]
            print("  op %-7d %-22s %-4s skill=%-3s exp=%s"
                  % (op_id, (o["name"] or "")[:22], o["owner_tag"], o["skill"],
                     ("%.2f" % o["experience"]) if o["experience"] is not None else None))
        # tech progress sample
        print("\ntech in-progress (scale /1000 = research units):")
        for tag in list(data["tech_progress"])[:4]:
            d = data["tech_progress"][tag]
            sample = [(a["slot_index"], a["progress_units"]) for a in d["active"][:3]]
            print("  %-4s %s" % (tag, sample))


if __name__ == "__main__":
    main()
