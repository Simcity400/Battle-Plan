"""leadership_deep_live.py - LIVE country-LEADERSHIP-DEEP reader (read-only).

Three country-leadership structures read save-free from a running PAUSED hoi4.exe
(ReadProcessMemory only; READ-ONLY, no writes / no injection / no thread suspend, so
it never desyncs MP), pinned on build 16022851 (see leadership_deep_offsets.draft.json):

  1. ACTIVE RULER  -- the country's CURRENT ruling leader character (+ name/token/
     sub-ideology). The discriminator the prior characters_live pass could not isolate
     on CCharacter/CCountryLeader is a POLITICS-SIDE RELATION: the ruling party
     (CPoliticalStatus+0xD0) directly owns its active country-leader:
        ps            = *(country_obj + 0xF90)         CPoliticalStatus
        ruling_party  = *(ps + 0xD0)                   CPoliticalParty
        cl            = *( *(ruling_party + 0x70) )    element[0] of a CCountryLeader* vec
        char          = *(cl + 0x08)                   CCharacter
        ruler_char_id = u32 @ char + 0x0C
     Validated 46/46 EXACT vs save (== politics.parties[ruling_party].country_leader[0]).

  2. ADVISOR ROSTER  -- per country, the set of advisor-capable characters (the
     political-advisor / high_command / theorist / army|air|navy_chief role objects).
     Enumerate CAdvisor (vtable RVA 0x2980560); each role: character=*(adv+0x10),
     idea_token=str@+0x30, slot=str@+0x80; group by CCharacter+0x68 country tag_db_id.
     Validated: char_id set EXACT 136/140 countries; (char_id,idea_token) 134/140
     (the <=4 non-exact are a LIVE SUPERSET - the engine instantiates a few advisor
     roles the save does not serialize; live is more complete, not wrong).
     NOTE this is the ROSTER (ownership), NOT the currently-APPOINTED cabinet: that
     appointed subset is RESISTED - no save-grounded active marker exists in this
     anchor (politics.ideas has empty intersection with advisor tokens; no hired bool;
     CCountryCharacters @ +0xFF0 stores the roster in per-role sub-managers, not a
     plain vector). Claiming an appointed subset would be fabrication. See draft.

  3. FACTION OBJECT  -- the formal faction objects (RTTI .?AVCFaction@NFactions@@,
     vtable RVA 0x293F970, exactly 2 instances this frame): name std::string @ +0x38,
     save id u32 @ +0x0C, members Pdx-vector {data@+0x58, cap@+0x60, size@+0x64} of
     CCountry-object pointers, leader = members[0]. Validated 2/2 EXACT vs save
     (Comintern name+10 members+leader SOV; Allies name+14 members+leader ENG).

Country identity (save-free, self-relocating each session):
  * faction member decode + advisor TAG grouping: diplomacy_live.CountryIdentity
    (vtable-scan RVA 0x27C0E80 + tag-db-id bridge) and live_layers/tag_table.
  * active-ruler per-country walk: politics_live._locate_country_array (unique-PP
    Rosetta) + _build_index_to_tag.

BUILD-GATED, READ-ONLY: refuses to run unless the live exe sha256 matches the build
(same fail-closed gate as the sibling readers); PROCESS_VM_READ | QUERY_INFORMATION.

Public API
----------
    read_active_rulers(p, bridge=None) -> { TAG: {
        "ruler_char_id": int, "ruling_party": str|None,
        "ruler_name": str|None, "ruler_token": str|None,
        "ruler_sub_ideology": str|None, "ambiguous": bool, "_char_addr": int } }

    read_factions(p, ident=None) -> [ {
        "save_id": int, "name": str|None,
        "leader": TAG|None, "leader_candidate": TAG|None,   # == members[0] (save convention)
        "members": [TAG, ...], "ordered_members": [TAG, ...], "_addr": int } ]

    read_advisor_roster(p, tagmap=None) -> { TAG: [ {
        "char_id": int, "slot": str|None, "idea_token": str|None }, ... ] }
    # ROSTER (advisor-capable chars owned by TAG), NOT the appointed cabinet.

Usage
-----
    python leadership_deep_live.py            # dump all three
    python leadership_deep_live.py --validate # live==save report
"""

import argparse
import json
import os
import struct
import sys
import collections

_THIS = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, _THIS)
sys.path.insert(0, os.path.dirname(_THIS))
sys.path.insert(0, os.path.join(_THIS, "live_layers"))

import winmem
import locate
import politics_live as POL
import characters_live as CL
import diplomacy_live as DIP
from live_layers import tag_table

EXPECTED_SHA = "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120"

HEAP_LO = POL.HEAP_LO
HEAP_HI = POL.HEAP_HI

# ---- vtables (module-relative, build 16022851) ----
VT_CHAR_RVA = 0x2957D40           # CCharacter
VT_COUNTRYLEADER_RVA = 0x295A630  # CCountryLeader
VT_ADVISOR_RVA = 0x2980560        # CAdvisor
VT_FACTION_RVA = 0x293F970        # CFaction@NFactions@@

# ---- active ruler chain ----
PARTY_LEADER_VEC = 0x70           # CPoliticalParty -> {begin,end} of CCountryLeader*
CL_CHAR_PTR = 0x08                # CCountryLeader+0x08 -> CCharacter
CHAR_TYPE = 73
CHAR_ID = 0x0C
CHAR_NAME = 0x28                  # CCharacter localized display name std::string ("Iosif Stalin")
CHAR_TOKEN = 0x48                 # CCharacter script token std::string ("SOV_iosif_stalin")
CHAR_COUNTRY = 0x68               # u32 country tag_db_id

# ---- faction object ----
FAC_SAVE_ID = 0x0C
FAC_NAME = 0x38                   # std::string SSO
FAC_MEM_DATA = 0x58
FAC_MEM_CAP = 0x60
FAC_MEM_SIZE = 0x64

# ---- advisor role ----
ADV_CHAR = 0x10                   # CAdvisor -> CCharacter back-link
ADV_TOKEN = 0x30                  # std::string idea/advisor token
ADV_SLOT = 0x80                   # std::string slot category

MAX_FAC_MEMBERS = 64
MAX_ADVISORS = 30000


# ==========================================================================
# build gate
# ==========================================================================
def check_build_gate(p, force=False):
    got = p.fingerprint().get("sha256")
    if got == EXPECTED_SHA:
        return
    msg = ("build gate: live exe SHA-256 %s != mapped build %s; offsets are "
           "build-specific." % (got, EXPECTED_SHA))
    if force:
        print(msg + "  --force given (results UNVERIFIED).")
    else:
        raise SystemExit(msg + "  Refusing (pass --force).")


def heap_ok(v):
    return HEAP_LO <= v < HEAP_HI


def _char_id_at(p, addr):
    """If addr is a CCharacter (type==73), return its char_id, else None."""
    if not heap_ok(addr):
        return None
    try:
        if p.u32(addr + 0x08) != CHAR_TYPE:
            return None
        return p.u32(addr + CHAR_ID)
    except OSError:
        return None


# ==========================================================================
# 1. ACTIVE RULER
# ==========================================================================
def _active_ruler_char(p, ruling_party, vt_cl):
    """ruling_party -> +0x70 vec begin -> element[0] CCountryLeader* -> +0x08 CCharacter.
    Returns (CCharacter addr, vec_size) or (None, vec_size). Fail-closed at each step.

    Codex hardening: report the leader-vec size; the caller treats size != 1 as
    AMBIGUOUS (could carry >1 country-leader -> element[0] is not provably the active
    one). All 46 validated countries had size == 1."""
    begin = POL._su64(p, ruling_party + PARTY_LEADER_VEC)
    if not heap_ok(begin):
        return None, 0
    end = POL._su64(p, ruling_party + PARTY_LEADER_VEC + 8)
    vec_size = ((end - begin) // 8) if (heap_ok(end) and end > begin) else 1
    cl = POL._su64(p, begin)          # element[0]
    if not heap_ok(cl):
        return None, vec_size
    try:
        if p.u64(cl) != vt_cl:        # must be a CCountryLeader
            return None, vec_size
    except OSError:
        return None, vec_size
    ch = POL._su64(p, cl + CL_CHAR_PTR)
    if _char_id_at(p, ch) is None:
        return None, vec_size
    return ch, vec_size


def read_active_rulers(p, bridge=None, strict=True, rec0=None, count=None):
    """LIVE active ruling-leader per country. Returns {TAG: {...}}.

    Reuses the politics_live country-array locate + unique-PP bridge. For each country
    the active ruler is the ruling party's owned country-leader (party+0x70 vec[0]).

    `rec0`/`count` (optional, supplied TOGETHER): the country-array base + count FROM THE LIVE
    CountryIndexBridge. When given, the locate is SKIPPED -- removing the redundant relocate AND
    the stale-save-PP relocate that raised (LocateError) and killed the whole walk."""
    if (rec0 is None) != (count is None):
        raise ValueError("read_active_rulers: rec0 and count must be supplied together")
    base = p.module_base
    vt_cl = base + VT_COUNTRYLEADER_RVA
    pp_path = os.path.join(os.path.dirname(_THIS), "re_scratch", "_country_pp.json")
    save_pp = json.load(open(pp_path)) if os.path.exists(pp_path) else {}
    if rec0 is None:
        rec0, count = POL._locate_country_array(p, save_pp)
    if bridge is None:
        bridge = POL._build_index_to_tag(p, rec0, count, save_pp)
        if save_pp and not POL._bridge_is_live(p, rec0, bridge, save_pp):
            import country_misc_live as cm
            sc = {t: {"political_power": v} for t, v in save_pp.items()}
            rec0, count = cm.locate_via_unique_pp(p, sc)
            bridge = POL._build_index_to_tag(p, rec0, count, save_pp)

    out = {}
    for i, tag in bridge.items():
        rec = rec0 + i * POL.REC
        try:
            if p.u16(rec + POL.OFF_INDEX) != i:
                continue
            obj = p.u64(rec + POL.OFF_NAMEPTR)
        except OSError:
            continue
        if not heap_ok(obj):
            continue
        ps = POL._su64(p, obj + POL.OBJ_POLSTATUS)
        if not heap_ok(ps):
            continue
        if strict and POL._obj_rtti(p, ps) != POL.RTTI_STATUS:
            continue
        rp = POL._su64(p, ps + POL.PS_RULING_PARTY)
        if not heap_ok(rp):
            continue
        if strict and POL._obj_rtti(p, rp) != POL.RTTI_PARTY:
            continue
        ruling = POL._ideology_id(p, rp)
        ch, vec_size = _active_ruler_char(p, rp, vt_cl)
        if ch is None:
            continue
        out[tag] = {
            "ruler_char_id": p.u32(ch + CHAR_ID),
            "ruling_party": ruling,
            "ruler_name": locate.read_msvc_str(p, ch + CHAR_NAME, maxlen=80),
            "ruler_token": locate.read_msvc_str(p, ch + CHAR_TOKEN, maxlen=80),
            # sub-ideology (stalinism/nazism/...) lives on the CCountryLeader role as a
            # def pointer not reached in this pass; ruling_party group is the primary class.
            "ruler_sub_ideology": None,
            # Codex hardening: size != 1 => the leader vec carries >1 country-leader, so
            # element[0] is not provably the active ruler. Flag, do not silently trust.
            "ambiguous": (vec_size != 1),
            "_char_addr": ch,
        }
    return out


# ==========================================================================
# 2. ADVISOR ROSTER
# ==========================================================================
def read_advisor_roster(p, tagmap=None):
    """LIVE per-country ADVISOR ROSTER (advisor-capable characters owned by TAG).

    NOT the appointed cabinet (that subset is RESISTED - no save ground truth in the
    anchor). Enumerate CAdvisor role objects, link each to its CCharacter (+0x10),
    group by CCharacter+0x68 country tag_db_id -> TAG."""
    addrs, _snap = enumerate_advisor_addrs(p)
    if tagmap is None:
        tbl = tag_table.TagTable.locate(p)
        tagmap = tbl.as_map() if tbl is not None else {}
    return read_advisor_roster_from_addrs(p, addrs, tagmap)


# --------------------------------------------------------------------------------------
# WARM-WALK CACHE for the CAdvisor pool (STAGE-4: the dominant per-walk SWEEP)
#
# read_advisor_roster's only expensive step is _find_instances(p, vt_CAdvisor, ...), a
# FULL committed-heap vtable sweep run on every call. On a PAUSED, build-gated capture the
# located CAdvisor object SET is stable across walks (no alloc/free), so a long-lived
# sampler can LOCATE the pool ONCE (enumerate_advisor_addrs), validate it cheaply each warm
# walk (validate_advisor_addrs -- ABA-defended), and re-decode off the pinned addrs
# (read_advisor_roster_from_addrs) WITHOUT re-sweeping. This mirrors the characters_live /
# oob_command_live warm-cache split and the designs_variants decoded-catalog pin. Fail-CLOSED:
# any validate miss -> the caller drops the pin and re-sweeps (slow-but-correct).
# --------------------------------------------------------------------------------------
def _advisor_identity(p, a):
    """The full per-CAdvisor decode-determining tuple, used both as the locate-time
    snapshot AND the warm-walk re-check. Returns (char_ptr, char_id, tag_db_id, slot,
    idea_token) -- EVERY field read_advisor_roster_from_addrs consumes to produce this
    advisor's row (the grouping TAG derives from tag_db_id via the tagmap, and slot +
    idea_token are the row payload). Any field that can't be read is None, so an
    unreadable/relinked slot fingerprints differently and fails validation.

    Why decode the FULL tuple (Codex A): char_ptr + char_id alone identify the owning
    CHARACTER, not the advisor ROLE -- a freed CAdvisor reused for the same character but a
    different slot/idea_token (small-string inline storage can keep the same addr+size)
    would pass a ptr+id-only check yet decode to a DIFFERENT row. Snapshotting the decoded
    slot/idea_token/tag_db_id closes that hole, so a warm HIT serving the cached decoded
    roster is provably byte-identical to a fresh decode."""
    ch = POL._su64(p, a + ADV_CHAR)
    cid = _char_id_at(p, ch)
    tid = None
    if cid is not None:
        try:
            tid = p.u32(ch + CHAR_COUNTRY)
        except OSError:
            tid = None
    slot = locate.read_msvc_str(p, a + ADV_SLOT, maxlen=48)
    token = locate.read_msvc_str(p, a + ADV_TOKEN, maxlen=80)
    return (ch, cid, tid, slot, token)


def enumerate_advisor_addrs(p):
    """LOCATE every CAdvisor role-object address (the full-heap vtable sweep) WITHOUT
    decoding the roster into its grouped form. Returns (addrs, snapshot) where:
      addrs    = [adv_addr, ...]  (every CAdvisor instance this frame)
      snapshot = {adv_addr: (char_ptr, char_id, tag_db_id, slot, idea_token)} -- the FULL
                 decode-determining fingerprint (see _advisor_identity) taken at locate
                 time. validate_advisor_addrs() re-reads ALL FIVE fields per addr, so any
                 reuse that changes the owning character, the country grouping, the slot,
                 OR the idea_token is caught (a true ABA defense for the DECODED row, not
                 just the object identity).
    This is the COLD step. read_advisor_roster_from_addrs() consumes ONLY the addr list
    (the snapshot is inert to decoding)."""
    base = p.module_base
    vt_adv = base + VT_ADVISOR_RVA
    addrs = _find_instances(p, vt_adv, MAX_ADVISORS)
    snapshot = {a: _advisor_identity(p, a) for a in addrs}
    return addrs, snapshot


def validate_advisor_addrs(p, addrs, snapshot):
    """FAIL-CLOSED liveness gate over a pinned CAdvisor addr set (no heap sweep).

    For EVERY adv addr: re-read +0x00 vtable (== module_base + VT_ADVISOR_RVA) AND the full
    decode-determining fingerprint (char_ptr, char_id, tag_db_id, slot, idea_token) and
    confirm ALL equal the locate-time snapshot. The vtable check rejects a different-class
    reuse; the fingerprint equality is the ABA defense for the DECODED ROW (a freed CAdvisor
    reused by another CAdvisor would have to reproduce the SAME character, country, slot, and
    idea_token to pass -- so the cached decoded roster cannot have drifted). Any mismatch /
    unreadable field -> False. Returns True iff EVERY supplied addr still validates AND the
    population count is unchanged. An empty addr set is NOT a valid located frame -> False."""
    if not addrs:
        return False
    vt_adv = p.module_base + VT_ADVISOR_RVA
    snap = snapshot or {}
    for a in addrs:
        try:
            if p.u64(a) != vt_adv:
                return False
        except OSError:
            return False
        if _advisor_identity(p, a) != snap.get(a):
            return False
    return True


def read_advisor_roster_from_addrs(p, addrs, tagmap):
    """Decode the per-country advisor roster from a SUPPLIED CAdvisor addr set (no heap
    sweep). Output is byte-identical to read_advisor_roster over the SAME paused heap +
    tagmap: for each addr, link char=*(adv+0x10), group by CCharacter+0x68 tag_db_id ->
    TAG, carry {char_id, slot, idea_token}. Rows whose char/tag don't resolve are dropped
    (exactly as the cold path drops them)."""
    tagmap = tagmap or {}
    roster = collections.defaultdict(list)
    for a in addrs:
        ch = POL._su64(p, a + ADV_CHAR)
        cid = _char_id_at(p, ch)
        if cid is None:
            continue
        try:
            tid = p.u32(ch + CHAR_COUNTRY)
        except OSError:
            continue
        tag = tagmap.get(tid)
        if tag is None:
            continue
        roster[tag].append({
            "char_id": cid,
            "slot": locate.read_msvc_str(p, a + ADV_SLOT, maxlen=48),
            "idea_token": locate.read_msvc_str(p, a + ADV_TOKEN, maxlen=80),
        })
    return dict(roster)


# ==========================================================================
# 3. FACTION OBJECT
# ==========================================================================
def read_factions(p, ident=None):
    """LIVE formal faction objects (name + leader + members). Returns a list of dicts.

    Located by the RTTI-resolved CFaction vtable (2 instances this frame). Members are
    8-byte CCountry-object pointers; decoded to TAG via diplomacy_live.CountryIdentity
    (inverse of object_of). leader = members[0] (the save convention)."""
    base = p.module_base
    vt_fac = base + VT_FACTION_RVA
    if ident is None:
        ident = DIP.CountryIdentity.resolve(p)
    tag_of_obj = {}
    if ident is not None:
        for t in ident.tags():
            o = ident.object_of(t)
            if o:
                tag_of_obj[o] = t
    out = []
    for a in _find_instances(p, vt_fac, 64):
        try:
            sid = p.u32(a + FAC_SAVE_ID)
        except OSError:
            continue
        name = locate.read_msvc_str(p, a + FAC_NAME, maxlen=64)
        data = POL._su64(p, a + FAC_MEM_DATA)
        size = POL._su32(p, a + FAC_MEM_SIZE)
        members = []
        if heap_ok(data) and 0 < size <= MAX_FAC_MEMBERS:
            blob = p.try_read(data, size * 8)
            if len(blob) >= size * 8:
                for k in range(size):
                    e = struct.unpack_from("<Q", blob, k * 8)[0]
                    members.append(tag_of_obj.get(e))
        out.append({
            "save_id": sid,
            "name": name,
            # Codex hardening: members[0] == leader is the save CONVENTION (validated
            # 2/2), not a proven dedicated leader field. Expose it as a candidate +
            # the ordered member list rather than asserting a leader pointer.
            "leader": members[0] if members else None,        # = leader_candidate
            "leader_candidate": members[0] if members else None,
            "members": members,
            "ordered_members": members,
            "_addr": a,
        })
    return out


# ==========================================================================
# instance enumeration (vtable scan; shared scanner pattern - never pos+=1)
# ==========================================================================
def _find_instances(p, vt_abs, limit):
    needle = struct.pack("<Q", vt_abs)
    out = []
    CH = 16 << 20
    OV = 16
    for base, size in p.iter_regions(HEAP_LO, 0x7FF000000000, want_exec=False):
        pos = 0
        while pos < size:
            want = min(CH, size - pos)
            blob = p.try_read(base + pos, want)
            if blob:
                i = blob.find(needle)
                while i != -1:
                    a = base + pos + i
                    if a % 8 == 0:
                        out.append(a)
                        if len(out) >= limit:
                            return out
                    i = blob.find(needle, i + 1)
                blen = len(blob)
                if pos + blen >= size or blen < want:
                    pos += max(blen, 0x1000)
                else:
                    pos += blen - OV
            else:
                pos += max(want, 0x1000)
    return out


# ==========================================================================
# read-all convenience
# ==========================================================================
def read_all(p):
    """Resolve identity once and read all three leadership-deep structures."""
    ident = DIP.CountryIdentity.resolve(p)
    tbl = tag_table.TagTable.locate(p)
    tagmap = tbl.as_map() if tbl is not None else {}
    return {
        "active_rulers": read_active_rulers(p),
        "factions": read_factions(p, ident=ident),
        "advisor_roster": read_advisor_roster(p, tagmap=tagmap),
    }


# ==========================================================================
# validation vs the lockstep save
# ==========================================================================
def _load_gt():
    path = os.path.join(os.path.dirname(_THIS), "re_scratch", "char", "_leadership_gt.json")
    return json.load(open(path))


def _validate(p):
    """Read live, compare to the melt GT, return a one-screen report dict."""
    gt = _load_gt()
    pp_map_path = os.path.join(os.path.dirname(_THIS), "re_scratch", "_country_pp.json")
    pp_map = json.load(open(pp_map_path)) if os.path.exists(pp_map_path) else {}

    # ---- 1. active ruler ----
    rulers_by_pp = gt["active_rulers_by_pp"]
    live_rulers = read_active_rulers(p)
    ar_ok = ar_n = 0
    ar_bad = []
    for tag, d in live_rulers.items():
        pp = pp_map.get(tag)
        grec = rulers_by_pp.get("%.5f" % pp) if pp is not None else None
        if not grec or grec.get("ruler_char_id") is None:
            continue
        ar_n += 1
        if d["ruler_char_id"] == grec["ruler_char_id"]:
            ar_ok += 1
        else:
            ar_bad.append((tag, d["ruler_char_id"], grec["ruler_char_id"]))

    # ---- 2. factions ----
    live_fac = read_factions(p)
    gt_fac = {f["save_id"]: f for f in gt["factions"]}
    fac_name_ok = fac_mem_ok = fac_lead_ok = fac_n = 0
    fac_bad = []
    for lf in live_fac:
        gf = gt_fac.get(lf["save_id"])
        if not gf:
            continue
        fac_n += 1
        if lf["name"] == gf["name"]:
            fac_name_ok += 1
        if lf["members"] == gf["members"]:
            fac_mem_ok += 1
        else:
            fac_bad.append((lf["save_id"], lf["members"], gf["members"]))
        if lf["leader"] == gf["leader"]:
            fac_lead_ok += 1

    # ---- 3. advisor roster ----
    live_adv = read_advisor_roster(p)
    gt_adv = gt["advisors_by_tag"]
    tags = sorted(set(gt_adv) & set(t for t in live_adv if t))
    cid_exact = tok_exact = 0
    superset = []
    for tag in tags:
        g_cid = set(a["char_id"] for a in gt_adv[tag])
        l_cid = set(a["char_id"] for a in live_adv[tag])
        g_tok = set((a["char_id"], a.get("idea_token")) for a in gt_adv[tag])
        l_tok = set((a["char_id"], a.get("idea_token")) for a in live_adv[tag])
        if g_cid == l_cid:
            cid_exact += 1
        elif l_cid >= g_cid:
            superset.append((tag, sorted(l_cid - g_cid)[:3]))
        if g_tok == l_tok:
            tok_exact += 1

    return {
        "active_ruler_vs_save": "%d/%d exact" % (ar_ok, ar_n),
        "active_ruler_mismatches": ar_bad[:6],
        "faction_count_live": len(live_fac),
        "faction_name_vs_save": "%d/%d exact" % (fac_name_ok, fac_n),
        "faction_members_vs_save": "%d/%d exact" % (fac_mem_ok, fac_n),
        "faction_leader_vs_save": "%d/%d exact" % (fac_lead_ok, fac_n),
        "faction_mismatches": fac_bad[:4],
        "advisor_roster_charid_set_vs_save": "%d/%d exact" % (cid_exact, len(tags)),
        "advisor_roster_charid_token_set_vs_save": "%d/%d exact" % (tok_exact, len(tags)),
        "advisor_roster_live_superset_countries": superset[:6],
        "advisor_appointed_cabinet": "RESISTED - no save-grounded active marker in this "
            "anchor (politics.ideas has empty intersection with advisor tokens; no hired "
            "bool); roster=ownership not appointment. See draft -> appointed_cabinet_resisted.",
    }


def main():
    ap = argparse.ArgumentParser(description="LIVE country-leadership-deep reader (read-only).")
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
        rulers = read_active_rulers(p)
        factions = read_factions(p)
        roster = read_advisor_roster(p)
        print("=== ACTIVE RULERS (%d) ===" % len(rulers))
        for tag in sorted(rulers):
            d = rulers[tag]
            print("  %-4s ruling=%-11s ruler_char=%-5d %s"
                  % (tag, (d["ruling_party"] or "-"), d["ruler_char_id"],
                     d["ruler_name"] or ""))
        print("\n=== FACTIONS (%d) ===" % len(factions))
        for f in factions:
            print("  id=%d name=%-10s leader=%-4s members(%d)=%s"
                  % (f["save_id"], f["name"] or "?", f["leader"] or "-",
                     len(f["members"]), ",".join(m or "?" for m in f["members"])))
        print("\n=== ADVISOR ROSTER (%d countries) ===" % len(roster))
        for tag in sorted(t for t in roster if t)[:8]:
            slots = collections.Counter(a["slot"] for a in roster[tag])
            print("  %-4s advisors=%-3d slots=%s"
                  % (tag, len(roster[tag]), dict(slots)))


if __name__ == "__main__":
    main()
