"""diplo_relations_live.py - READ-ONLY live BILATERAL-RELATIONS reader for Battle Plan.

Completes the diplomacy domain: reads, save-free, from a running PAUSED hoi4.exe via
ReadProcessMemory only (READ-ONLY; no writes / no injection / no thread suspension, so
it never desyncs MP):

  * OPINION         -- per-ordered-pair net opinion (the save's active_relations.cached_sum).
  * RELATION FLAGS  -- non_aggression_pact / military_access / embargo / docking_rights /
                       air_base_access / naval_blockade / market_access_rights /
                       equipment_purchase / guarantee / improve_relations / attache /
                       licence_production / puppet, detected by the flag sub-object's RTTI
                       vtable. Symmetric flags are de-duplicated as UNDIRECTED pairs.
  * SUBJECTS/AUTONOMY -- overlord -> subject + autonomy_state name + autonomy ratio, read
                       from the CSubjectRelation sub-object (the save's puppet={...} block).

Reach (save-free; cracked by a Ghidra static xref of opinion_modifier.cpp + the relation/
autonomy RTTI vtables, then validated live==save -- see diplo_relations_offsets.draft.json):

    mgr = u64[country_obj + 0xF88]            # per-country diplomacy/relations manager
    tbl = u64[mgr + 0x08]                     # tag-db-id-indexed relation-entry pointer table
    rel = u64[tbl + target_tag_db_id*8]       # per-target relation entry
      opinion (cached_sum)   = i32[rel + 0x300]
      flag present           = a known flag-relation vtable appears at [ptr+0] for some
                               heap ptr among rel's qwords (within a 0x340 window)
    CSubjectRelation sub-object (vtable RVA 0x293A168):
      +0x10 overlord tag_db_id   +0x14 subject tag_db_id
      +0x58 -> autonomy-state DB obj (name std::string at +0x08)
      +0x60 autonomy_ratio i32 x100000

Validated live==save on build 16022851 against the lockstep melt (save_date 1946.5.19.9):
  * opinion 1113/1113 real-country pairs EXACT (the 69 X|WST pairs are the Wasteland
    pseudo-tag, no country object -> outside identity by design).
  * subjects/autonomy 17/17 EXACT (overlord, subject, state name, ratio).
  * relation flags, as undirected pairs: NAP 121/121, puppet 17/17, embargo 10/10,
    military_access 26/26, air_base_access 23/23, docking 53/53, naval_blockade 4/4,
    equipment_purchase 18/18, market_access 35/38 (3 misses all WST).

HARD RULES honored (same as diplomacy_live.py / intel_live.py):
  * READ-ONLY memory; keep the game PAUSED; passive reads => any cadence, no desync.
  * Build-gated: refuses to run unless the live hoi4.exe sha256 matches the build.
  * No personal paths; self-contained; reuses winmem.py + locate.py + diplomacy_live's
    CountryIdentity (vtable scan + tag-db-id bridge).
"""

import os
import sys
import struct

_HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, _HERE)
sys.path.insert(0, os.path.join(_HERE, "live_layers"))
import winmem        # noqa: E402
import locate        # noqa: E402
import diplomacy_live as _dl   # noqa: E402  (reuse the proven CountryIdentity)

EXPECTED_SHA256 = "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120"
PROC_NAME = "hoi4.exe"

# ---- reach offsets (see diplo_relations_offsets.draft.json) ----
OBJ_RELMGR = 0xF88          # country_object + this -> diplomacy/relations manager
MGR_TABLE = 0x08            # manager + this -> tag-db-id-indexed relation-entry ptr table
REL_WINDOW = 0x340         # bytes of the relation entry we read (covers opinion + flags)
REL_OPINION = 0x300        # i32 cached_sum (net opinion) on the relation entry

# ---- CSubjectRelation sub-object (puppet) offsets ----
SUBJ_OVERLORD = 0x10        # i32 overlord tag_db_id
SUBJ_SUBJECT = 0x14        # i32 subject  tag_db_id
SUBJ_STATE_DB = 0x58       # u64 -> shared autonomy-state DB obj (name std::string at +0x08)
SUBJ_STATE_NAME = 0x08     # msvc std::string within the DB obj
SUBJ_RATIO = 0x60          # i32 autonomy ratio x100000
RATIO_SCALE = 100000.0

# ---- flag-relation vtable RVAs (Ghidra image base 0x140000000; live = base + RVA) ----
FLAG_VT_RVA = {
    0x2939FE8: "non_aggression_pact",
    0x293A168: "puppet",
    0x293A2C0: "embargo",
    0x2939B68: "military_access",
    0x29F9DB8: "air_base_access",
    0x29F9FA8: "docking_rights",
    0x29FA150: "naval_blockade",
    0x29FA300: "market_access_rights",
    0x29FA238: "equipment_purchase_contract_relation",
    0x2939F28: "guarantee",
    0x2939CE8: "improve_relations",
    0x2939DA8: "attache",
    0x2939AA8: "licence_production",
}
SUBJ_VT_RVA = 0x293A168

# flags that are symmetric (both edges identical) -> report as undirected pairs.
# Role-bearing/directional flags keep their own first/second semantics.
_SYMMETRIC_FLAGS = frozenset({
    "non_aggression_pact", "embargo", "military_access", "air_base_access",
    "docking_rights", "naval_blockade", "market_access_rights",
    "equipment_purchase_contract_relation",
})

HEAP_LO = 0x10000000000
HEAP_HI = 0x7FFFFFFFFFFF


def _inheap(v):
    return HEAP_LO <= v <= HEAP_HI


# ==========================================================================
# core reads
# ==========================================================================
def _relation_entry(p, country_obj, target_tag_db_id):
    """rel = u64[ u64[ u64[obj+0xF88] + 8 ] + target*8 ], or None."""
    try:
        mgr = p.u64(country_obj + OBJ_RELMGR)
        if not _inheap(mgr):
            return None
        tbl = p.u64(mgr + MGR_TABLE)
        if not _inheap(tbl):
            return None
        rel = p.u64(tbl + target_tag_db_id * 8)
    except OSError:
        return None
    return rel if _inheap(rel) else None


def _read_relation(p, ident, rel, flag_vt_abs, subj_vt_abs):
    """Parse one relation entry. Returns (opinion_i32_or_None, set(flag_names),
    subject_dict_or_None). subject_dict is read from the CSubjectRelation sub-object."""
    blob = p.try_read(rel, REL_WINDOW)
    if not blob or len(blob) < REL_OPINION + 4:
        return None, set(), None
    opinion = struct.unpack_from("<i", blob, REL_OPINION)[0]
    flags = set()
    subject = None
    # Scan the entry's qwords for heap pointers whose [+0] vtable is a known flag class.
    # (Codex: validate alignment + in-heap to reject arbitrary-pointer false positives.)
    for off in range(0, len(blob) - 8, 8):
        v = struct.unpack_from("<Q", blob, off)[0]
        if not _inheap(v):
            continue
        try:
            vt = p.u64(v)
        except OSError:
            continue
        nm = flag_vt_abs.get(vt)
        if nm is None:
            continue
        flags.add(nm)
        if vt == subj_vt_abs and subject is None:
            subject = _read_subject(p, ident, v)
    return opinion, flags, subject


def _read_subject(p, ident, subj_obj):
    """Read the CSubjectRelation sub-object -> {overlord, subject, autonomy_state, value}."""
    try:
        ov = p.i32(subj_obj + SUBJ_OVERLORD)
        su = p.i32(subj_obj + SUBJ_SUBJECT)
        ratio_raw = p.i32(subj_obj + SUBJ_RATIO)
        db = p.u64(subj_obj + SUBJ_STATE_DB)
    except OSError:
        return None
    state = None
    if _inheap(db):
        state = locate.read_msvc_str(p, db + SUBJ_STATE_NAME, maxlen=64)
    val = (ratio_raw / RATIO_SCALE) if ratio_raw else None
    return {
        "overlord": ident.tag(ov),
        "subject": ident.tag(su),
        "autonomy_state": state,
        "value": val,
    }


# ==========================================================================
# enumeration
# ==========================================================================
def read_all(p, ident=None):
    """Resolve identity + enumerate every ordered country relation. Returns a dict:
        {
          "opinion":   {"SRC|TGT": cached_sum, ...}          # nonzero net opinion
          "flags":     {"SRC|TGT": [flag, ...], ...}         # directed flag presence
          "undirected_flags": {"A|B"(A<B): [flag, ...], ...} # symmetric flags de-duped
          "subjects":  [{overlord, subject, autonomy_state, value}, ...]  # 1 per subject
        }
    Raises RuntimeError if country identity cannot be resolved (fail-closed).

    ``ident`` (optional): reuse the shared CountryIdentity (resolved once at attach) to skip
    this reader's own identity scan; None self-resolves (caller passes None on shared failure)."""
    if ident is None:
        ident = _dl.CountryIdentity.resolve(p)
    if ident is None:
        raise RuntimeError("could not resolve country identity (vtable scan + bridge)")
    base = p.module_base
    flag_vt_abs = {base + rva: nm for rva, nm in FLAG_VT_RVA.items()}
    subj_vt_abs = base + SUBJ_VT_RVA

    opinion = {}
    flags = {}
    subjects = {}                      # (overlord, subject) -> dict (de-dup; both edges)
    tags = ident.tags()
    for src in tags:
        so = ident.object_of(src)
        if not so:
            continue
        for tgt in tags:
            if src == tgt:
                continue
            tt = ident.tag_db_id(tgt)
            if tt is None:
                continue
            rel = _relation_entry(p, so, tt)
            if rel is None:
                continue
            op, fl, subj = _read_relation(p, ident, rel, flag_vt_abs, subj_vt_abs)
            key = "%s|%s" % (src, tgt)
            if op:
                opinion[key] = op
            if fl:
                flags[key] = sorted(fl)
            if subj and subj["overlord"] and subj["subject"]:
                subjects[(subj["overlord"], subj["subject"])] = subj

    return {
        "opinion": opinion,
        "flags": flags,
        "undirected_flags": _undirected_symmetric(flags),
        "subjects": sorted(subjects.values(),
                           key=lambda s: (s["overlord"] or "", s["subject"] or "")),
    }


def _undirected_symmetric(flags):
    """Collapse symmetric flags (stored on both edges) into one undirected entry keyed
    A|B with A<B. Directional flags are kept only on the directed `flags` map."""
    out = {}
    for key, fl in flags.items():
        a, b = key.split("|")
        sym = [f for f in fl if f in _SYMMETRIC_FLAGS]
        if not sym:
            continue
        uk = "%s|%s" % (a, b) if a < b else "%s|%s" % (b, a)
        out.setdefault(uk, set()).update(sym)
    return {k: sorted(v) for k, v in out.items()}


def opinion_of(p, ident, src_tag, tgt_tag):
    """One-shot net opinion (cached_sum) for an ordered pair, or None."""
    so = ident.object_of(src_tag)
    tt = ident.tag_db_id(tgt_tag)
    if not so or tt is None:
        return None
    rel = _relation_entry(p, so, tt)
    if rel is None:
        return None
    try:
        return p.i32(rel + REL_OPINION)
    except OSError:
        return None


# ==========================================================================
# build gate
# ==========================================================================
def _build_gate(p):
    sha = p.fingerprint().get("sha256")
    if sha != EXPECTED_SHA256:
        raise SystemExit("build gate FAIL: live hoi4.exe sha256 %s != mapped build %s\n"
                         "Offsets are build-specific; refusing to read an unknown build."
                         % (sha, EXPECTED_SHA256))


# ==========================================================================
# Validation (live == save) against the anchor melt
# ==========================================================================
def _validate(p, gt_path=None):
    """Compare the live reader against the relation ground truth (extracted from the
    melt by _relgt.py). Prints a report and returns True iff opinion/subjects/flags all
    match for real countries. Save = ANCHOR ONLY. WST=Wasteland pseudo-tag pairs are
    excluded (no country object; not a diplomatic actor)."""
    import json
    if gt_path is None:
        gt_path = os.path.join(_HERE, "..", "re_scratch", "diplo", "rel_ground_truth.json")
    gt_path = os.path.abspath(gt_path)
    with open(gt_path, encoding="utf-8") as fh:
        gt = json.load(fh)

    data = read_all(p)
    ok_all = True

    def real(key):
        a, b = key.split("|")
        return a != "WST" and b != "WST"

    # --- opinion (real-country pairs) ---
    g_op = {k: v for k, v in gt["opinion_cached_sum"].items()
            if v != 0 and real(k) and k.split("|")[0] != k.split("|")[1]}
    live_op = data["opinion"]
    op_match = sum(1 for k, v in g_op.items() if live_op.get(k) == v)
    print("  opinion (real pairs) EXACT: %d/%d" % (op_match, len(g_op)))
    ok_all &= (op_match == len(g_op))
    wst_op = sum(1 for k, v in gt["opinion_cached_sum"].items() if v != 0 and not real(k))
    print("    (excluded %d X|WST opinion pairs -- Wasteland pseudo-tag, no country object)"
          % wst_op)

    # --- subjects / autonomy ---
    g_sub = {(x["overlord"], x["subject"]): (x["autonomy_state"], x["value"])
             for x in gt["puppets"]}
    l_sub = {(x["overlord"], x["subject"]): (x["autonomy_state"], x["value"])
             for x in data["subjects"]}
    sub_match = sum(1 for k, v in g_sub.items() if l_sub.get(k) == v)
    print("  subjects/autonomy EXACT (overlord,subject,state,ratio): %d/%d"
          % (sub_match, len(g_sub)))
    for k, v in g_sub.items():
        if l_sub.get(k) != v:
            print("    MISMATCH %s save=%s live=%s" % (k, v, l_sub.get(k)))
    ok_all &= (sub_match == len(g_sub))

    # --- relation flags (undirected, real pairs) ---
    def undirected(d):
        u = {}
        for k, fl in d.items():
            a, b = k.split("|")
            uk = frozenset((a, b))
            u.setdefault(uk, set()).update(fl)
        return u
    gu = undirected(gt["relation_flags"])
    lu = undirected(data["flags"])
    flag_types = sorted({f for v in gu.values() for f in v})
    total_real = 0
    covered_real = 0
    for ft in flag_types:
        gpairs = {k for k, v in gu.items() if ft in v and "WST" not in k}
        lpairs = {k for k, v in lu.items() if ft in v}
        cov = len(gpairs & lpairs)
        total_real += len(gpairs)
        covered_real += cov
        tag = "OK" if cov == len(gpairs) else ("%d missing" % (len(gpairs) - cov))
        print("    %-38s %d/%d  %s" % (ft, cov, len(gpairs), tag))
    print("  relation flags (undirected, real pairs) covered: %d/%d"
          % (covered_real, total_real))
    ok_all &= (covered_real == total_real)

    print("  OVERALL live==save: %s" % ("PASS" if ok_all else "FAIL"))
    return ok_all


def main():
    import argparse
    ap = argparse.ArgumentParser(description="Live the game bilateral-relations reader (read-only).")
    ap.add_argument("--validate", action="store_true", help="compare live vs the anchor melt")
    args = ap.parse_args()
    p = winmem.open_process(PROC_NAME)
    if p is None:
        raise SystemExit("hoi4.exe not running")
    with p:
        _build_gate(p)
        if args.validate:
            print("build gate OK; validating relations live==save (READ-ONLY)...")
            _validate(p)
            return
        print("build gate OK; reading bilateral relations (READ-ONLY)...")
        data = read_all(p)
        print("opinion pairs (nonzero): %d" % len(data["opinion"]))
        print("flagged ordered pairs:   %d" % len(data["flags"]))
        print("subjects:                %d" % len(data["subjects"]))
        print("\nsubjects (overlord -> subject : autonomy_state x ratio):")
        for s in data["subjects"]:
            print("  %s -> %-4s : %-34s %s"
                  % (s["overlord"], s["subject"], s["autonomy_state"],
                     ("%.2f" % s["value"]) if s["value"] is not None else "-"))


if __name__ == "__main__":
    main()
