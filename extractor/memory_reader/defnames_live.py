"""defnames_live.py - LIVE STATIC-DEF HANDLE->NAME resolvers (read-only).

Many already-cracked live domains pin a static-def HANDLE/INDEX but leave the
human NAME unresolved. They share ONE pattern: a global static-def registry of
def objects, each holding a u32 handle field and a token std::string. This module
cracks that handle->name decode ONCE per registry and exposes resolvers so the
live readers can resolve names with NO save and NO mod-file guessing.

The registries are STATIC content (load-order-deterministic) -> the LIVE registry
is the authoritative handle->name map for THIS build+modset. Every def object is
enumerated by reverse-scanning committed non-exec heap for the def's stable RTTI
vtable (module_base + RVA), the same method the existing domain readers use.

REGISTRIES (build 16022851; see defnames_offsets.draft.json for the full ledger):

  1. TECH  (CTechnologyTemplate, vtable RVA 0x2915F40)         [pinned, 100%]
       handle u32 @ +0x3C   token std::string @ +0x10
       resolves CResearchSlot active_tech_handle / CTechnology tech-def-index.
       4227/4227 researched-vector handles resolved across 43 countries.

  2. IDEA / LAW CATEGORY  (CIdeaGroupType 0x2920270, CIdeaCategory 0x295A928)
       group name std::string @ group+0x38; group->CIdeaCategory ptr @ group+0x60
       (category token @ cat+0x10); group member-idea vector {data @ group+0x18,
       size @ group+0x20, element = CIdea* with token @ idea+0x18}.
       -> {idea_token -> group (the law sub-type) -> category}.   [pinned]
       27/27 mod law tokens match; 46/46 countries get a full law triple.

  3. SCIENTIST TRAIT  (CScientistTrait, vtable RVA 0x26F5470)    [pinned, 179/179]
       handle u32 @ +0x08   token std::string @ +0x20
       (scientist trait elements at CScientist+0x30 are DIRECT def pointers.)

  4. UNIT-LEADER TRAIT  (CUnitLeaderTrait, vtable RVA 0x291F3C0) [registry pinned]
       handle u32 @ +0x08   token std::string @ +0x10  (also an index->token array)
       199 def objects, 196 with a readable token (3 carry a heap-ptr token whose
       SSO header didn't pass the string check this frame; the 196 cover the handle
       set seen in live data). NOTE the leader-side assigned-trait MEMBERSHIP source is
       NOT pinned (the characters_live leader+0x288 flat_map is a modifier-coefficient bag
       in a different id-space) -> the registry resolves handles, but the source list of a
       leader's assigned traits is a follow-up. See draft -> leader_membership_resisted.

  5. MIO POLICY  (def vtable RVA 0x26F4350; no RTTI name, located by handle-scan)
       handle u32 @ +0x08   ICON token std::string @ +0x30 (GFX_mio_policy_<suffix>;
       this is the icon/display key, NOT the unique script name).      [handles decode;
       MIO-side: u32 handle array {data @ MIO+0x168, count @ MIO+0x174}.  membership partial]
       MIO45 -> its 10 allowed policies in save order, 1:1 (validated-partial - the save
       serializes allowed_policies for only 4 MIOs).

RESISTED: MIO unlocked-TRAIT names (org-template-owned, no flat registry - walk the
per-org trait tree or use the save oracle). See draft -> mio_trait.

BUILD-GATED, READ-ONLY: refuses to run unless the live exe sha256 matches the build;
PROCESS_VM_READ | PROCESS_QUERY_INFORMATION only. Game kept PAUSED.

Public API
----------
    build_tech_registry(p)        -> {handle:int -> tech_token:str}
    resolve_tech(p, handle, reg=None)            -> str | None

    build_idea_group_map(p)       -> {idea_token:str -> {"group":str|None,"category":str|None}}
    idea_category(p, idea_token, m=None)         -> {"group":..,"category":..} | None
    classify_laws(p, idea_tokens, m=None)        -> {"economy":tok,"trade_laws":tok,"mobilization_laws":tok}

    build_scientist_trait_registry(p) -> {handle:int -> token:str}
    resolve_scientist_trait(p, handle, reg=None) -> str | None
    resolve_scientist_traits_live(p, char_addr)  -> [token,...]  (direct def-pointer walk)

    build_unit_leader_trait_registry(p) -> {handle:int -> token:str}
    build_unit_leader_trait_index(p)    -> {index:int  -> token:str}
    resolve_trait(p, handle, reg=None)           -> str | None
    resolve_trait_by_index(p, index, idx=None)   -> str | None

    build_mio_policy_registry(p)  -> {handle:int -> gfx_token:str}
    resolve_mio_policy(p, handle, reg=None)      -> str | None
    resolve_mio_policies_live(p, mio_addr, reg=None) -> [gfx_token,...]

Usage
-----
    python defnames_live.py            # dump registry sizes + samples
    python defnames_live.py --validate # full live==save / live==mod report
"""

import argparse
import json
import os
import struct
import sys

_THIS = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, _THIS)
sys.path.insert(0, os.path.dirname(_THIS))

import winmem  # noqa: E402

EXPECTED_SHA = "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120"

HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000

# ---- registry vtable RVAs (module-relative, build 16022851) ----
TECH_TEMPLATE_VT_RVA = 0x2915F40   # CTechnologyTemplate
IDEA_GROUP_VT_RVA = 0x2920270      # CIdeaGroupType
IDEA_CAT_VT_RVA = 0x295A928        # CIdeaCategory
IDEA_VT_RVA = 0x295A8C0            # CIdea (active object)
SCI_TRAIT_VT_RVA = 0x26F5470       # CScientistTrait
UL_TRAIT_VT_RVA = 0x291F3C0        # CUnitLeaderTrait
MIO_POLICY_VT_RVA = 0x26F4350      # MIO policy def

# ---- per-registry field offsets ----
TECH_HANDLE_OFF = 0x3C
TECH_TOKEN_OFF = 0x10

IDEA_GROUP_NAME_OFF = 0x38
IDEA_GROUP_CAT_PTR_OFF = 0x60
IDEA_GROUP_MEMBER_DATA_OFF = 0x18
IDEA_GROUP_MEMBER_SIZE_OFF = 0x20
IDEA_CAT_TOKEN_OFF = 0x10
IDEA_TOKEN_OFF = 0x18              # CIdea token (matches politics_live)

SCI_TRAIT_HANDLE_OFF = 0x08
SCI_TRAIT_TOKEN_OFF = 0x20
# scientist trait-element source (CScientist sub-object)
CHAR_SCI_PTR_OFF = 0xC0
SCI_TRAITS_BEGIN_OFF = 0x30
SCI_TRAITS_COUNT_OFF = 0x38

UL_TRAIT_HANDLE_OFF = 0x08
UL_TRAIT_TOKEN_OFF = 0x10

MIO_POLICY_HANDLE_OFF = 0x08
MIO_POLICY_TOKEN_OFF = 0x30       # GFX/ICON token (GFX_mio_policy_<suffix>), NOT the script name
MIO_POLICY_DATA_OFF = 0x168
MIO_POLICY_COUNT_OFF = 0x174      # the validated allowed_policies_count (== mio_live MIO+0x174).
                                  # +0x170 is sometimes count+1 (a separate capacity/cursor) -> use +0x174.

LAW_GROUPS = ("economy", "trade_laws", "mobilization_laws")
MAX_MEMBERS = 4000
MAX_POLICY_COUNT = 64


# ==========================================================================
# build gate + low-level helpers
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


def _gate_ok(p):
    return p.fingerprint().get("sha256") == EXPECTED_SHA


def heap_ok(v):
    return HEAP_LO <= v < HEAP_HI


def _su32(p, a):
    try:
        return p.u32(a)
    except OSError:
        return None


def _su64(p, a):
    try:
        return p.u64(a)
    except OSError:
        return 0


def _read_msvc_str(p, base, maxlen=256):
    """MSVC std::string at `base`: buffer[16]@+0, size(u64)@+0x10, cap(u64)@+0x18;
    SSO when cap<16 (chars inline at +0) else +0 is a heap ptr. Region-clamp safe."""
    try:
        size = p.u64(base + 0x10)
        cap = p.u64(base + 0x18)
    except OSError:
        return None
    if cap < size or size == 0 or size > maxlen or cap > (1 << 20):
        return None
    try:
        chars = base if cap < 16 else p.u64(base)
        data = p.read(chars, size)
    except OSError:
        return None
    try:
        s = data.decode("latin-1")
    except Exception:
        return None
    return s if all(32 <= ord(c) < 127 for c in s) else None


def _enum_vtable(p, vt_abs):
    """Yield every 8-aligned heap address whose qword == vt_abs (object base).
    Uses the proven anti-stall, region-clamped advance (shared scanner pattern)."""
    needle = struct.pack("<Q", vt_abs)
    CH = 32 << 20
    OV = 16
    for base, size in p.iter_regions(HEAP_LO, HEAP_HI, want_exec=False):
        pos = 0
        while pos < size:
            want = min(CH, size - pos)
            blob = p.try_read(base + pos, want)
            if blob:
                i = blob.find(needle)
                while i != -1:
                    addr = base + pos + i
                    if addr % 8 == 0:
                        yield addr
                    i = blob.find(needle, i + 1)
                blen = len(blob)
                if pos + blen >= size or blen < want:
                    pos += max(blen, 0x1000)
                else:
                    pos += blen - OV
            else:
                pos += max(want, 0x1000)


# ==========================================================================
# 1. TECH registry
# ==========================================================================
def build_tech_registry(p):
    """{tech-def handle:int -> tech token:str} from CTechnologyTemplate objects."""
    if not _gate_ok(p):
        return {}
    reg = {}
    vt = p.module_base + TECH_TEMPLATE_VT_RVA
    for a in _enum_vtable(p, vt):
        tok = _read_msvc_str(p, a + TECH_TOKEN_OFF)
        h = _su32(p, a + TECH_HANDLE_OFF)
        if tok and h is not None:
            reg[h] = tok
    return reg


def resolve_tech(p, handle, reg=None):
    if reg is None:
        reg = build_tech_registry(p)
    return reg.get(handle)


# ==========================================================================
# 2. IDEA / LAW CATEGORY registry
# ==========================================================================
def _build_category_tokens(p):
    """{CIdeaCategory addr -> category token}."""
    out = {}
    for a in _enum_vtable(p, p.module_base + IDEA_CAT_VT_RVA):
        out[a] = _read_msvc_str(p, a + IDEA_CAT_TOKEN_OFF)
    return out


def build_idea_group_map(p, report_collisions=False):
    """{idea_token -> {"group": group_name|None, "category": category_token|None}}.

    Walks every CIdeaGroupType: its name (the law sub-type) @ +0x38, its category via
    +0x60 -> CIdeaCategory(+0x10), and its ordered member-idea vector {data @ +0x18,
    size @ +0x20, element = CIdea* with token @ +0x18}.

    Token-keyed: the game script addresses ideas by bare token, so a token belongs to exactly
    one group. A multi-group token would be ambiguous -> we COLLISION-GUARD: if the same
    token shows up under two different group names we keep the first and record the conflict
    (returned via the second value when report_collisions=True). An idea in NO group (e.g. a
    dynamic country spirit) simply never appears here -> idea_category() returns None for it,
    which is a valid non-law/non-grouped idea, NOT an unresolved failure."""
    if not _gate_ok(p):
        return ({}, {}) if report_collisions else {}
    cats = _build_category_tokens(p)
    idea_vt = p.module_base + IDEA_VT_RVA
    out = {}
    collisions = {}
    for a in _enum_vtable(p, p.module_base + IDEA_GROUP_VT_RVA):
        name = _read_msvc_str(p, a + IDEA_GROUP_NAME_OFF)
        cat = cats.get(_su64(p, a + IDEA_GROUP_CAT_PTR_OFF))
        data = _su64(p, a + IDEA_GROUP_MEMBER_DATA_OFF)
        size = _su32(p, a + IDEA_GROUP_MEMBER_SIZE_OFF)
        if not name or not heap_ok(data) or size is None or not (0 <= size <= MAX_MEMBERS):
            continue
        arr = p.try_read(data, size * 8)
        for k in range(min(size, len(arr) // 8)):
            e = struct.unpack_from("<Q", arr, k * 8)[0]
            if not heap_ok(e):
                continue
            try:
                if p.u64(e) != idea_vt:        # RTTI gate: only real CIdea elements
                    continue
            except OSError:
                continue
            tok = _read_msvc_str(p, e + IDEA_TOKEN_OFF)
            if not tok:
                continue
            prev = out.get(tok)
            if prev is not None and prev["group"] != name:
                collisions.setdefault(tok, [prev["group"]]).append(name)
                continue                       # keep the first (fail-closed on ambiguity)
            out[tok] = {"group": name, "category": cat}
    return (out, collisions) if report_collisions else out


def idea_category(p, idea_token, m=None):
    if m is None:
        m = build_idea_group_map(p)
    return m.get(idea_token)


def classify_laws(p, idea_tokens, m=None):
    """Given a country's idea token list, return {law_group -> idea_token} for the
    economy / trade_laws / mobilization_laws laws present."""
    if m is None:
        m = build_idea_group_map(p)
    out = {}
    for tok in idea_tokens:
        info = m.get(tok)
        if info and info.get("group") in LAW_GROUPS:
            out[info["group"]] = tok
    return out


# ==========================================================================
# 3. SCIENTIST TRAIT registry
# ==========================================================================
def build_scientist_trait_registry(p):
    """{scientist-trait handle:int -> token:str} from CScientistTrait objects."""
    if not _gate_ok(p):
        return {}
    reg = {}
    for a in _enum_vtable(p, p.module_base + SCI_TRAIT_VT_RVA):
        tok = _read_msvc_str(p, a + SCI_TRAIT_TOKEN_OFF)
        h = _su32(p, a + SCI_TRAIT_HANDLE_OFF)
        if tok and h is not None:
            reg[h] = tok
    return reg


def resolve_scientist_trait(p, handle, reg=None):
    if reg is None:
        reg = build_scientist_trait_registry(p)
    return reg.get(handle)


def _scientist_trait_def_addrs(p):
    return set(_enum_vtable(p, p.module_base + SCI_TRAIT_VT_RVA))


def resolve_scientist_traits_live(p, char_addr, def_addrs=None):
    """A scientist's trait tokens via the DIRECT def-pointer elements:
    CCharacter+0xC0 -> CScientist; +0x30 = element vector data (stride 8), +0x38 = count.
    Each element is a CScientistTrait* -> token @ +0x20. Returns [token,...] or None."""
    if def_addrs is None:
        def_addrs = _scientist_trait_def_addrs(p)
    s1 = _su64(p, char_addr + CHAR_SCI_PTR_OFF)
    if not heap_ok(s1):
        return None
    begin = _su64(p, s1 + SCI_TRAITS_BEGIN_OFF)
    cnt = _su32(p, s1 + SCI_TRAITS_COUNT_OFF)
    if not heap_ok(begin) or cnt is None or cnt > 64:
        return None
    arr = p.try_read(begin, cnt * 8)
    if len(arr) < cnt * 8:
        return None
    out = []
    for k in range(cnt):
        e = struct.unpack_from("<Q", arr, k * 8)[0]
        if e in def_addrs:
            out.append(_read_msvc_str(p, e + SCI_TRAIT_TOKEN_OFF))
        else:
            return None                       # not a clean def pointer -> fail closed
    return out


# ==========================================================================
# 4. UNIT-LEADER TRAIT registry (commander/general/admiral traits)
# ==========================================================================
def build_unit_leader_trait_registry(p):
    """{trait-def handle:int -> token:str} from CUnitLeaderTrait objects."""
    if not _gate_ok(p):
        return {}
    reg = {}
    for a in _enum_vtable(p, p.module_base + UL_TRAIT_VT_RVA):
        tok = _read_msvc_str(p, a + UL_TRAIT_TOKEN_OFF)
        h = _su32(p, a + UL_TRAIT_HANDLE_OFF)
        if tok and h is not None:
            reg[h] = tok
    return reg


def build_unit_leader_trait_index(p):
    """{registration index:int -> token:str} via the contiguous CUnitLeaderTrait*
    registration array. Finds the array by locating a long run of def pointers."""
    if not _gate_ok(p):
        return {}
    defmap = {a: _read_msvc_str(p, a + UL_TRAIT_TOKEN_OFF)
              for a in _enum_vtable(p, p.module_base + UL_TRAIT_VT_RVA)}
    if not defmap:
        return {}
    defset = set(defmap)
    # locate the registration array: scan heap for a pointer to any def, then walk a
    # contiguous run of def pointers and keep the longest run.
    anchor = next(iter(defset))
    needle = struct.pack("<Q", anchor)
    best = None
    for base, size in p.iter_regions(HEAP_LO, HEAP_HI, want_exec=False):
        pos = 0
        CH = 32 << 20
        while pos < size:
            want = min(CH, size - pos)
            blob = p.try_read(base + pos, want)
            if blob:
                i = blob.find(needle)
                while i != -1:
                    cand = base + pos + i
                    start = cand
                    while heap_ok(_su64(p, start - 8)) and _su64(p, start - 8) in defset:
                        start -= 8
                    end = cand
                    while heap_ok(_su64(p, end)) and _su64(p, end) in defset:
                        end += 8
                    n = (end - start) // 8
                    if best is None or n > best[1]:
                        best = (start, n)
                    i = blob.find(needle, i + 1)
                blen = len(blob)
                if pos + blen >= size or blen < want:
                    pos += max(blen, 0x1000)
                else:
                    pos += blen - 8
            else:
                pos += max(want, 0x1000)
        if best and best[1] >= len(defmap):
            break
    if not best:
        return {}
    start, n = best
    arr = p.try_read(start, n * 8)
    out = {}
    for k in range(min(n, len(arr) // 8)):
        e = struct.unpack_from("<Q", arr, k * 8)[0]
        out[k] = defmap.get(e)
    return out


def resolve_trait(p, handle, reg=None):
    if reg is None:
        reg = build_unit_leader_trait_registry(p)
    return reg.get(handle)


def resolve_trait_by_index(p, index, idx=None):
    if idx is None:
        idx = build_unit_leader_trait_index(p)
    return idx.get(index)


# ==========================================================================
# 5. MIO POLICY registry
# ==========================================================================
def build_mio_policy_registry(p):
    """{policy-def handle:int -> ICON token:str} from the MIO policy def pool.

    NOTE: the token at +0x30 is the policy's GFX/ICON key (GFX_mio_policy_<suffix>), NOT
    the script name (mio_policy_<category>_<suffix>). The icon token is NOT guaranteed unique
    across policies (e.g. land+navy welding_specialists share an icon), so treat it as a
    display/icon identifier, not a canonical key. The script name needs the category prefix
    (general/land/air/navy) which the def object does not store."""
    if not _gate_ok(p):
        return {}
    reg = {}
    for a in _enum_vtable(p, p.module_base + MIO_POLICY_VT_RVA):
        tok = _read_msvc_str(p, a + MIO_POLICY_TOKEN_OFF)
        h = _su32(p, a + MIO_POLICY_HANDLE_OFF)
        if tok and h is not None:
            reg[h] = tok
    return reg


def resolve_mio_policy(p, handle, reg=None):
    if reg is None:
        reg = build_mio_policy_registry(p)
    return reg.get(handle)


def mio_policy_suffix(gfx_token):
    """GFX_mio_policy_<suffix> -> <suffix> (the stable policy identifier)."""
    if gfx_token and gfx_token.startswith("GFX_mio_policy_"):
        return gfx_token[len("GFX_mio_policy_"):]
    return gfx_token


def resolve_mio_policies_live(p, mio_addr, reg=None):
    """An MIO's allowed-policy ICON tokens: the first `count` (MIO+0x174, the validated
    allowed_policies_count) u32 handles at the MIO+0x168 data ptr (4-byte stride). Returns
    [icon_token,...] (empty if none). Validated-partial: MIO45 -> its 10 allowed policies
    in save order, 1:1. Use +0x174 for the count (+0x170 is occasionally count+1)."""
    if reg is None:
        reg = build_mio_policy_registry(p)
    data = _su64(p, mio_addr + MIO_POLICY_DATA_OFF)
    cnt = _su32(p, mio_addr + MIO_POLICY_COUNT_OFF)
    if cnt is not None:
        cnt &= 0xFFFF
    if not heap_ok(data) or cnt is None or cnt == 0 or cnt > MAX_POLICY_COUNT:
        return []
    raw = p.try_read(data, cnt * 4)
    if len(raw) < cnt * 4:
        return []
    out = []
    for k in range(cnt):
        h = struct.unpack_from("<I", raw, k * 4)[0]
        tok = reg.get(h)
        if tok:
            out.append(tok)
    return out


# ==========================================================================
# validation
# ==========================================================================
def _validate(p):
    """Read each registry live, validate vs the lockstep save / mod files, return a
    one-screen report dict. Imports the sibling domain readers for handle sources."""
    rep = {}
    scr = os.path.join(os.path.dirname(_THIS), "re_scratch")

    # --- 1. TECH ---
    treg = build_tech_registry(p)
    rep["tech_registry"] = {"def_objects": len(treg),
                            "handle_range": ("0x%X..0x%X" % (min(treg), max(treg))) if treg else None}
    try:
        import tech_live
        live = tech_live.read_country_tech(p, strict=True)
        gt_path = os.path.join(_THIS, "_defnames_tech_gt.json")
        gt = json.load(open(gt_path)) if os.path.exists(gt_path) else {}
        res_ok = res_n = 0
        subset_ok = subset_n = 0
        for tag, d in live.items():
            st = d["_status_addr"]
            data = _su64(p, st + 0x40)
            size = _su32(p, st + 0x4C) or 0
            toks = set()
            unresolved = 0
            for i in range(size):
                tp = _su64(p, data + i * 8)
                di = _su32(p, tp + 0x08)
                t = treg.get(di) if di is not None else None
                res_n += 1
                if t:
                    res_ok += 1
                    toks.add(t)
                else:
                    unresolved += 1
            g = set(gt.get(tag, []))
            if g:
                subset_n += 1
                if not (toks - g) and unresolved == 0:
                    subset_ok += 1
        rep["tech_validation"] = {
            "researched_handles_resolved": "%d/%d" % (res_ok, res_n),
            "per_country_token_subset_clean": "%d/%d (mismatches = hidden mod techs / GT-extraction)"
                                              % (subset_ok, subset_n) if subset_n else "no GT file",
        }
    except Exception as e:
        rep["tech_validation"] = "skipped (%s)" % e

    # --- 2. IDEA / LAW CATEGORY ---
    imap = build_idea_group_map(p)
    rep["idea_group_map"] = {"ideas_mapped": len(imap),
                             "law_groups_present": sorted({v["group"] for v in imap.values()
                                                           if v["group"] in LAW_GROUPS})}
    try:
        import politics_live
        pol = politics_live.read_country_politics(p, strict=True)
        triple_ok = 0
        for tag, d in pol.items():
            laws = classify_laws(p, d["ideas"], imap)
            if all(g in laws for g in LAW_GROUPS):
                triple_ok += 1
        rep["idea_validation"] = {"countries_with_full_law_triple": "%d/%d" % (triple_ok, len(pol))}
        # mod cross-check (if the parsed mod GT is present)
        modgt = os.path.join(_THIS, "_defnames_modlaw_gt.json")
        if os.path.exists(modgt):
            mg = json.load(open(modgt))
            ok = sum(1 for t, g in mg.items() if (imap.get(t) or {}).get("group") == g)
            rep["idea_validation"]["mod_law_token_group_match"] = "%d/%d" % (ok, len(mg))
    except Exception as e:
        rep["idea_validation"] = "skipped (%s)" % e

    # --- 3. SCIENTIST TRAIT ---
    sreg = build_scientist_trait_registry(p)
    rep["scientist_trait_registry"] = {"def_objects": len(sreg),
                                       "handle_range": ("0x%X..0x%X" % (min(sreg), max(sreg))) if sreg else None}
    try:
        import projects_live
        def_addrs = _scientist_trait_def_addrs(p)
        scis = projects_live.read_scientists(p)
        live_tr = {}
        for s in scis:
            tr = resolve_scientist_traits_live(p, s["_addr"], def_addrs)
            if tr is not None:
                live_tr[s["char_id"]] = set(tr)
        anc = os.path.join(_THIS, "_proj_anchors.json")
        ok = n = 0
        if os.path.exists(anc):
            save = json.load(open(anc)).get("scientists", {})
            sv = {}
            for tag, lst in save.items():
                for x in lst:
                    if x.get("char_id") is not None:
                        sv[x["char_id"]] = set(x.get("traits") or [])
            for cid, trs in sv.items():
                lv = live_tr.get(cid)
                if lv is None:
                    continue
                n += 1
                if lv == trs:
                    ok += 1
        rep["scientist_validation"] = {"trait_set_live_eq_save": "%d/%d" % (ok, n) if n else "no anchor"}
    except Exception as e:
        rep["scientist_validation"] = "skipped (%s)" % e

    # --- 4. UNIT-LEADER TRAIT (registry only; leader membership source resisted) ---
    ureg = build_unit_leader_trait_registry(p)
    uidx = build_unit_leader_trait_index(p)
    rep["unit_leader_trait_registry"] = {
        "def_objects": len(ureg),
        "handle_range": ("0x%X..0x%X" % (min(ureg), max(ureg))) if ureg else None,
        "registration_index_entries": len(uidx),
        "index0_token": uidx.get(0),
        "note": "registry pinned (handle->token + index->token). Leader assigned-trait "
                "MEMBERSHIP source NOT pinned (leader+0x288 flat_map is a modifier bag, "
                "different id-space) - see draft leader_membership_resisted.",
    }

    # --- 5. MIO POLICY ---
    preg = build_mio_policy_registry(p)
    rep["mio_policy_registry"] = {"def_objects": len(preg),
                                  "handle_range": ("0x%X..0x%X" % (min(preg), max(preg))) if preg else None}
    try:
        import mio_live
        mios = mio_live.enumerate_mios(p)
        cnt_ok = cnt_n = 0
        for r in mios.values():
            pols = resolve_mio_policies_live(p, r["addr"], preg)
            if r["allowed_policies_count"] == 0:
                continue
            cnt_n += 1
            if len(pols) == r["allowed_policies_count"]:
                cnt_ok += 1
        rep["mio_policy_validation"] = {
            "per_mio_resolved_count_eq_allowed": "%d/%d (populated MIOs)" % (cnt_ok, cnt_n) if cnt_n else "0",
            "note": "MIO45 -> its 10 allowed policies resolve 1:1 IN ORDER vs the save "
                    "allowed_policies={}. Save serializes allowed_policies for only 4 MIOs "
                    "(rest inherit), so a broad N/N save SET cross-check is thin; the "
                    "handle->token decode is solid.",
        }
    except Exception as e:
        rep["mio_policy_validation"] = "skipped (%s)" % e

    rep["mio_trait_names"] = "RESISTED: org-template-owned (no flat registry). Walk the per-org " \
                             "trait tree or use the save oracle. See draft -> mio_trait."
    return rep


def main():
    ap = argparse.ArgumentParser(description="LIVE static-def handle->name resolvers (read-only).")
    ap.add_argument("--force", action="store_true",
                    help="run even if the exe SHA-256 doesn't match the mapped build")
    ap.add_argument("--validate", action="store_true",
                    help="validate every registry vs the lockstep save / mod files")
    args = ap.parse_args()

    p = winmem.open_process("hoi4.exe")
    if p is None:
        raise SystemExit("hoi4.exe not running")
    with p:
        check_build_gate(p, args.force)
        if args.validate:
            print(json.dumps(_validate(p), indent=1))
            return
        treg = build_tech_registry(p)
        imap = build_idea_group_map(p)
        sreg = build_scientist_trait_registry(p)
        ureg = build_unit_leader_trait_registry(p)
        preg = build_mio_policy_registry(p)
        print("\nregistry sizes:")
        print("  tech                 %4d  e.g. 0x83C2 -> %s" % (len(treg), treg.get(0x83C2)))
        print("  idea_group_map       %4d  e.g. war_economy -> %s" % (len(imap), imap.get("war_economy")))
        print("  scientist_trait      %4d  e.g. 0x57D8 -> %s" % (len(sreg), sreg.get(0x57D8)))
        print("  unit_leader_trait    %4d  e.g. 0x87DA -> %s" % (len(ureg), ureg.get(0x87DA)))
        print("  mio_policy           %4d  e.g. 0x825D -> %s" % (len(preg), preg.get(0x825D)))


if __name__ == "__main__":
    main()
