"""politics_live.py - LIVE per-country POLITICS reader (read-only).

Reads, straight from a running PAUSED hoi4.exe (no save needed at runtime), each
country's political state pinned on build 16022851 (see politics_offsets.draft.json).
The politics state lives in a per-country CPoliticalStatus object hanging off the big
country object at country_object + 0xF90 (the same fixed-offset-submanager pattern as
tech CTechnologyStatus @ +0xF60 and the focus submanager @ +0x1380):

    parties           Pdx-vector {data @ ps+0x20, size @ ps+0x28, cap @ ps+0x2C};
                      element = 8-byte ptr -> CPoliticalParty.  Each party:
                        ideology id  party->[+0x18]=CIdeologyGroup, id std::string @ grp+0x18
                                     ("democratic"/"communism"/"fascism"/"neutrality")
                        popularity   i32 @ party+0x88, fixed-point = percent * 100000
                                     (e.g. 65.00001% -> 6500001)
                      The 4-popularity vector summing to ~100% is the primary anchor:
                      4-vector EXACT 46/46 vs the lockstep save.
    ruling_party      ps+0xD0 -> CPoliticalParty (the current ruling/leader party);
                      ideology id read the same way.            46/46 EXACT vs save.
    ideas             Pdx-vector {data @ ps+0x50, cap @ ps+0x58, size @ ps+0x5C};
                      element = 8-byte ptr -> CIdea, id std::string @ idea+0x18.
                      This is the FULL active-ideas/spirits set AND it contains the
                      economy/trade/mobilisation LAW ideas (laws are ideas in the game).
                      ideas set AND order EXACT 46/46 vs save.
    political_power   i32 @ ps+0xE0, fixed-point x100000 (bonus; == the PP Rosetta).
    election_frequency u32 @ ps+0xE8 (months).                  EXACT vs save.

LOCATION (save-free, self-relocating each session):
  * The country array (rec0/count) is taken from the cached locate when it still
    verifies live, else relocated via the unique-PP method (country_misc_live).
    country object = u64[rec+0x08]; running_index -> TAG bridged by unique
    full-precision political_power vs the save (the same Rosetta as economy/tech/focus),
    with the Codex PP-quorum re-read guard against a stale cached rec0.
  * NO heap/RTTI pool scan is needed for the per-country read: CPoliticalStatus is
    reached by a fixed offset off each country object. Each CPoliticalStatus is
    RTTI-verified (.?AVCPoliticalStatus@@) and each party (.?AVCPoliticalParty@@) /
    idea (.?AVCIdea@@) is RTTI-gated, so a wrong pointer fails closed.

OPEN TICKETS (reported honestly, never faked):
  * LAW sub-classification (which idea is the economy law vs trade law vs conscription
    law). The law ideas ARE in the validated `ideas` list (by id), but tagging each as
    economy/trade/mobilisation needs the CIdea -> CIdeaCategory/token-type resolution,
    which is a deeper deref (CIdea+0x78 -> a slot object that did not resolve to a clean
    category string in the time budget). Exposed: the full ideas list; NOT a typed
    {economy_law, trade_law, mobilization_law} triple. See draft open_tickets.
  * last_election (a the game date int) and elections_allowed (bool): election_frequency is
    pinned (ps+0xE8, EXACT), but the last_election date int and the elections_allowed
    bool were not located in the CPoliticalStatus header this frame (the date is not a
    clean year-scaled int in the header region; likely stored in CCountry or encoded).
    Left UNRESOLVED rather than asserting a wrong offset.
  * political advisors / cabinet: not pinned (would hang off CCountryCharacters); out of
    scope for this pass.

BUILD-GATED: refuses to run unless the live exe SHA-256 matches the mapped build, unless
--force is given (same fail-closed gate as locate.py / tech_live.py / focus_live.py).

READ-ONLY: PROCESS_VM_READ | PROCESS_QUERY_INFORMATION only; ReadProcessMemory +
VirtualQueryEx; no writes, no injection, game untouched, MP-safe.

Public API
----------
    read_country_politics(p, bridge=None) -> { TAG: {
        "popularities": {"democratic": float, "communism": float,
                         "fascism": float, "neutrality": float},  # percent 0..100
        "ruling_party": str | None,        # ruling ideology group id
        "ideas": [idea_id, ...],           # ordered, == save order; includes law ideas
        "idea_count": int,
        "political_power": float,          # bonus
        "election_frequency": int | None,  # months
        "_status_addr": int,
    } }
  where `bridge` is an optional {running_index: TAG} dict (else built from the save PP
  Rosetta in re_scratch/_country_pp.json).

Usage
-----
    python politics_live.py                 # locate + dump every bridged country
    python politics_live.py --validate      # compare live vs the save anchor and report
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
import locate

EXPECTED_SHA = "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120"

HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000

# country array
REC = 0x120
OFF_NAMEPTR = 0x08
OFF_PP = 0x20
OFF_INDEX = 0x5D
PP_SCALE = 100000.0

# country-object -> CPoliticalStatus
OBJ_POLSTATUS = 0xF90        # u64 ptr -> CPoliticalStatus

# CPoliticalStatus offsets
PS_PARTIES_DATA = 0x20       # u64 ptr -> array of CPoliticalParty*
PS_PARTIES_SIZE = 0x28       # u32 size (count of parties, == 4)
PS_PARTIES_CAP = 0x2C        # u32 capacity
PS_IDEAS_DATA = 0x50         # u64 ptr -> array of CIdea*
PS_IDEAS_CAP = 0x58          # u32 capacity
PS_IDEAS_SIZE = 0x5C         # u32 size (== active idea count, incl. law ideas)
PS_RULING_PARTY = 0xD0       # u64 ptr -> CPoliticalParty (ruling/leader party)
PS_PP = 0xE0                 # i32 political power, fixed-point x100000
PS_ELECTION_FREQ = 0xE8      # u32 election frequency (months)

# CPoliticalParty offsets
PARTY_IDEOLOGY_GRP = 0x18    # u64 ptr -> CIdeologyGroup
PARTY_POPULARITY = 0x88      # i32 popularity, fixed-point = percent * 100000

# CIdeologyGroup / CIdea id std::string offset
GRP_ID = 0x18                # std::string ideology-group id within CIdeologyGroup
IDEA_ID = 0x18               # std::string idea id within CIdea

# RTTI type names (fail-closed gating)
RTTI_STATUS = ".?AVCPoliticalStatus@@"
RTTI_PARTY = ".?AVCPoliticalParty@@"
RTTI_IDEA = ".?AVCIdea@@"

# sanity bounds
MAX_PARTIES = 16
MAX_IDEAS = 400
IDEOLOGY_GROUPS = ("democratic", "communism", "fascism", "neutrality")


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


def heap_ok(v):
    return HEAP_LO <= v < HEAP_HI


def _su64(p, a):
    try:
        return p.u64(a)
    except OSError:
        return 0


def _su32(p, a):
    try:
        return p.u32(a)
    except OSError:
        return 0


def _si32(p, a):
    try:
        return p.i32(a)
    except OSError:
        return None


# ==========================================================================
# RTTI type-name resolution (MSVC x64 COL -> TypeDescriptor)
# ==========================================================================
def _rtti_name(p, vtable):
    mb = p.module_base
    if not (mb <= vtable < mb + 0x10000000):
        return None
    col = _su64(p, vtable - 0x08)
    if not (mb <= col < mb + 0x10000000):
        return None
    try:
        td = mb + p.u32(col + 0x0C)
        nm = p.cstr(td + 0x10, 200)
    except OSError:
        return None
    return nm if nm and nm.startswith(".?A") else None


def _obj_rtti(p, obj):
    """RTTI type name of a heap object (reads its vtable then resolves)."""
    if not heap_ok(obj):
        return None
    return _rtti_name(p, _su64(p, obj))


def _ideology_id(p, party):
    grp = _su64(p, party + PARTY_IDEOLOGY_GRP)
    if not heap_ok(grp):
        return None
    return locate.read_msvc_str(p, grp + GRP_ID)


# ==========================================================================
# index -> TAG bridge (unique political_power Rosetta against the save)
# ==========================================================================
def _verify_rec0(p, rec0, depth=24):
    try:
        for k in range(depth):
            if p.u16(rec0 + k * REC + OFF_INDEX) != k:
                return False
    except OSError:
        return False
    return True


def _bridge_is_live(p, rec0, idx_tag, save_pp_by_tag, min_quorum=20):
    """PP-quorum re-read guard (Codex hardening): confirm the (possibly cached) rec0 is
    the LIVE array by re-reading each bridged record's political_power and requiring it
    still equals the save value on two consecutive reads."""
    ok = 0
    for i, tag in idx_tag.items():
        want = round(save_pp_by_tag.get(tag, 0) * PP_SCALE)
        try:
            a = p.i32(rec0 + i * REC + OFF_PP)
            b = p.i32(rec0 + i * REC + OFF_PP)
        except OSError:
            continue
        if a == b == want:
            ok += 1
    return ok >= min_quorum


def _locate_country_array(p, save_pp_by_tag):
    cache = os.path.join(os.path.dirname(_THIS), "re_scratch", "_cmisc_rec0.json")
    if os.path.exists(cache):
        try:
            c = json.load(open(cache))
            rec0, count = c["rec0"], c["count"]
            if _verify_rec0(p, rec0) and 40 <= count <= 1000:
                return rec0, count
        except (OSError, ValueError, KeyError):
            pass
    import country_misc_live as cm
    save_countries = {t: {"political_power": v} for t, v in save_pp_by_tag.items()}
    return cm.locate_via_unique_pp(p, save_countries)


def _build_index_to_tag(p, rec0, count, save_pp_by_tag):
    """running_index -> TAG via unique full-precision political_power (resolves 1946
    puppet/cosmetic-name collisions that a localized-name table would mis-bridge)."""
    by_raw = {}
    for tag, v in save_pp_by_tag.items():
        by_raw.setdefault(round(v * PP_SCALE), []).append(tag)
    idx_tag = {}
    used = set()
    for i in range(count):
        rec = rec0 + i * REC
        try:
            if p.u16(rec + OFF_INDEX) != i:
                continue
            raw = p.i32(rec + OFF_PP)
        except OSError:
            continue
        tags = by_raw.get(raw)
        if tags and len(tags) == 1 and tags[0] not in used:
            idx_tag[i] = tags[0]
            used.add(tags[0])
    return idx_tag


# ==========================================================================
# the reader
# ==========================================================================
def _stable_vec_header(p, ps, data_off, size_off, cap_off):
    """Two-pass stable read of a Pdx-vector {data, size, cap} header so a live tick can
    never splice an old `data` with a new `size` (Codex anti-torn-read). Returns
    (data, size, cap) or None on mutation/OSError."""
    try:
        d1 = p.u64(ps + data_off)
        s1 = p.u32(ps + size_off)
        c1 = p.u32(ps + cap_off)
        d2 = p.u64(ps + data_off)
        s2 = p.u32(ps + size_off)
        c2 = p.u32(ps + cap_off)
    except OSError:
        return None
    if (d1, s1, c1) != (d2, s2, c2):
        return None
    return d1, s1, c1


def _read_parties(p, ps, strict=True):
    """Return (pops_by_ideology, party_ptrs, size, cap) for the parties vector at ps+0x20.

    Codex-hardened: stable header reread, size<=cap invariant, RTTI gate per element,
    distinct ideology ids; caller cross-checks ruling_party membership."""
    hdr = _stable_vec_header(p, ps, PS_PARTIES_DATA, PS_PARTIES_SIZE, PS_PARTIES_CAP)
    if hdr is None:
        return {}, [], 0, 0
    data, size, cap = hdr
    out = {}
    ptrs = []
    if not (heap_ok(data) and 0 < size <= MAX_PARTIES and size <= cap):
        return out, ptrs, size, cap
    for k in range(size):
        pe = _su64(p, data + k * 8)
        if not heap_ok(pe):
            continue
        if strict and _obj_rtti(p, pe) != RTTI_PARTY:
            continue
        ido = _ideology_id(p, pe)
        pr = _si32(p, pe + PARTY_POPULARITY)
        if ido is None or pr is None:
            continue
        if strict and ido in out:          # distinct ideology ids (fail-closed on dup)
            continue
        out[ido] = pr / PP_SCALE
        ptrs.append(pe)
    return out, ptrs, size, cap


def _read_ideas(p, ps, strict=True):
    """Return the ordered list of active idea ids (incl. law ideas) from ps+0x50.

    Codex-hardened: read EXACTLY `size` (never read-until-invalid; unused capacity slots
    can hold stale-but-valid CIdea* that RTTI would pass, resurrecting removed ideas).
    Fail closed if the header is implausible (size>cap or size>MAX_IDEAS)."""
    hdr = _stable_vec_header(p, ps, PS_IDEAS_DATA, PS_IDEAS_CAP, PS_IDEAS_SIZE)
    if hdr is None:
        return []
    data, cap, size = hdr                   # NOTE: this vector's header is {data, cap, size}
    if size == 0:
        return []
    if not (heap_ok(data) and size <= cap and size <= MAX_IDEAS):
        return []                           # implausible -> fail closed (no fallback)
    ideas = []
    for k in range(size):
        e = _su64(p, data + k * 8)
        if not heap_ok(e):
            return []                        # short/torn -> fail closed for the whole list
        if strict and _obj_rtti(p, e) != RTTI_IDEA:
            return []
        s = locate.read_msvc_str(p, e + IDEA_ID)
        if s is None:
            return []
        ideas.append(s)
    return ideas


def read_country_politics(p, bridge=None, strict=True, rec0=None, count=None):
    """LIVE per-country politics state. Returns {TAG: {...}}.

    `bridge` (optional) = {running_index: TAG}. If None, it is built from the save PP
    Rosetta (re_scratch/_country_pp.json) with the PP-quorum live guard. `strict`
    RTTI-gates each CPoliticalStatus / CPoliticalParty / CIdea (fail-closed).

    `rec0`/`count` (optional, supplied TOGETHER): the country-array base + count FROM THE LIVE
    CountryIndexBridge. When given, _locate_country_array is SKIPPED -- removing the redundant
    relocate AND the stale-save-PP relocate that raised (LocateError) and killed the whole walk.
    The bridge base is the live-validated array (authoritative for live capture)."""
    if (rec0 is None) != (count is None):
        raise ValueError("read_country_politics: rec0 and count must be supplied together")
    pp_path = os.path.join(os.path.dirname(_THIS), "re_scratch", "_country_pp.json")
    save_pp = json.load(open(pp_path)) if os.path.exists(pp_path) else {}
    if rec0 is None:
        rec0, count = _locate_country_array(p, save_pp)
    if bridge is None:
        bridge = _build_index_to_tag(p, rec0, count, save_pp)
        if save_pp and not _bridge_is_live(p, rec0, bridge, save_pp):
            import country_misc_live as cm
            save_countries = {t: {"political_power": v} for t, v in save_pp.items()}
            rec0, count = cm.locate_via_unique_pp(p, save_countries)
            bridge = _build_index_to_tag(p, rec0, count, save_pp)

    out = {}
    for i, tag in bridge.items():
        rec = rec0 + i * REC
        try:
            if p.u16(rec + OFF_INDEX) != i:
                continue
            obj = p.u64(rec + OFF_NAMEPTR)
        except OSError:
            continue
        if not heap_ok(obj):
            continue

        ps = _su64(p, obj + OBJ_POLSTATUS)
        if not heap_ok(ps):
            continue
        if strict and _obj_rtti(p, ps) != RTTI_STATUS:
            continue

        pops_by_ideo, party_ptrs, psize, _pcap = _read_parties(p, ps, strict=strict)
        # normalize to the 4 canonical ideology groups (missing => 0.0)
        popularities = {g: pops_by_ideo.get(g, 0.0) for g in IDEOLOGY_GROUPS}

        rp = _su64(p, ps + PS_RULING_PARTY)
        ruling = None
        if heap_ok(rp) and (not strict or _obj_rtti(p, rp) == RTTI_PARTY):
            ruling = _ideology_id(p, rp)

        # Codex discriminator: the ruling-party pointer (+0xD0) must be a member of the
        # +0x20 parties vector. This is the cheap semantic guard that rejects the SECOND
        # 4-party vector at ps+0x140 (default/historical popularities) which RTTI alone
        # cannot distinguish. confidence flag, not a drop (validated 46/46 with it true).
        ruling_in_vec = (rp in party_ptrs) if party_ptrs else False
        low_confidence = (psize != 4) or (not ruling_in_vec)

        ideas = _read_ideas(p, ps, strict=strict)

        pp_raw = _si32(p, ps + PS_PP)
        freq = _su32(p, ps + PS_ELECTION_FREQ)

        out[tag] = {
            "popularities": popularities,
            "ruling_party": ruling,
            "ideas": ideas,
            "idea_count": len(ideas),
            "political_power": (pp_raw / PP_SCALE) if pp_raw is not None else None,
            "election_frequency": freq if 0 < freq <= 600 else None,
            "party_count": psize,
            "low_confidence": low_confidence,
            "_status_addr": ps,
        }
    return out


# ==========================================================================
# validation vs the lockstep save
# ==========================================================================
def _load_save_politics_gt():
    scr = os.path.join(os.path.dirname(_THIS), "re_scratch")
    return json.load(open(os.path.join(scr, "_politics_gt.json")))


def _validate(p):
    """Read live, compare to the save anchor, return a one-screen report dict."""
    gt = _load_save_politics_gt()
    live = read_country_politics(p, strict=True)

    pop_ok = pop_n = 0
    rule_ok = rule_n = 0
    idea_set_ok = idea_ord_ok = idea_n = 0
    freq_ok = freq_n = 0
    pp_ok = pp_n = 0
    pop_bad = []
    idea_bad = []
    for tag, d in live.items():
        g = gt.get(tag)
        if not g:
            continue
        # popularity 4-vector
        pop_n += 1
        if all(abs(d["popularities"].get(k, -9) - g["popularities"][k]) < 0.001
               for k in IDEOLOGY_GROUPS):
            pop_ok += 1
        else:
            pop_bad.append((tag, d["popularities"], g["popularities"]))
        # ruling party
        if g.get("ruling_party"):
            rule_n += 1
            if d["ruling_party"] == g["ruling_party"]:
                rule_ok += 1
        # ideas (set and order)
        if g.get("ideas") is not None:
            idea_n += 1
            if set(d["ideas"]) == set(g["ideas"]):
                idea_set_ok += 1
            if d["ideas"] == g["ideas"]:
                idea_ord_ok += 1
            else:
                idea_bad.append((tag, sorted(set(d["ideas"]) - set(g["ideas"]))[:2],
                                 sorted(set(g["ideas"]) - set(d["ideas"]))[:2]))
        # election frequency
        if g.get("election_frequency") is not None:
            freq_n += 1
            if d["election_frequency"] == g["election_frequency"]:
                freq_ok += 1
        # political power (bonus)
        if g.get("political_power") is not None:
            pp_n += 1
            if d["political_power"] is not None and abs(d["political_power"] - g["political_power"]) < 0.001:
                pp_ok += 1

    return {
        "live_countries": len(live),
        "popularity_4vector_vs_save": "%d/%d exact" % (pop_ok, pop_n),
        "ruling_party_vs_save": "%d/%d exact" % (rule_ok, rule_n),
        "ideas_set_vs_save": "%d/%d exact" % (idea_set_ok, idea_n),
        "ideas_order_vs_save": "%d/%d exact" % (idea_ord_ok, idea_n),
        "election_frequency_vs_save": "%d/%d exact" % (freq_ok, freq_n),
        "political_power_vs_save_bonus": "%d/%d exact" % (pp_ok, pp_n),
        "popularity_mismatches": pop_bad[:6],
        "ideas_mismatches": idea_bad[:6],
        "laws_note": "OPEN TICKET: economy/trade/mobilisation LAW ideas are present in "
                     "the validated `ideas` list by id, but typing each as a specific law "
                     "needs CIdea->category resolution (deeper deref); not exposed as a typed triple.",
        "election_note": "OPEN TICKET: last_election (date int) and elections_allowed (bool) "
                         "not pinned this pass; election_frequency IS pinned (ps+0xE8).",
    }


def main():
    ap = argparse.ArgumentParser(description="LIVE per-country politics reader (read-only).")
    ap.add_argument("--force", action="store_true",
                    help="run even if the exe SHA-256 doesn't match the mapped build")
    ap.add_argument("--validate", action="store_true",
                    help="compare live reads to the save anchor and report match rate")
    args = ap.parse_args()

    p = winmem.open_process("hoi4.exe")
    if p is None:
        raise SystemExit("hoi4.exe not running")
    with p:
        check_build_gate(p, args.force)
        if args.validate:
            print(json.dumps(_validate(p), indent=1))
            return
        live = read_country_politics(p)
        print("\n%-4s %-11s %-7s %-7s %-7s %-7s %5s %5s %5s"
              % ("TAG", "ruling", "dem%", "com%", "fasc%", "neu%", "ideas", "freq", "PP"))
        for tag in sorted(live):
            d = live[tag]
            pp = d["popularities"]
            print("%-4s %-11s %6.2f %6.2f %6.2f %6.2f %5d %5s %6.1f"
                  % (tag, (d["ruling_party"] or "-")[:11],
                     pp["democratic"], pp["communism"], pp["fascism"], pp["neutrality"],
                     d["idea_count"], d["election_frequency"] or "-", d["political_power"]))
        print("\ncountries read: %d" % len(live))


if __name__ == "__main__":
    main()
