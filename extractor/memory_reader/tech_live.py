"""tech_live.py - LIVE per-country RESEARCH / TECHNOLOGY reader (read-only).

Reads, straight from a running PAUSED hoi4.exe (no save needed at runtime), the
per-country research/tech state pinned on build 16022851 (see tech_offsets.draft.json):

    completed_tech_count   CTechnologyStatus +0x4C  u32   (size of the researched
                           leveled-tech vector; 43/43 EXACT vs the lockstep save)
    research_slot_count    CTechnologyStatus +0xAC  u32   (current slot capacity;
                           45/46 EXACT vs save research_slot)
    active_research        per active CResearchSlot: {slot_index, tech_handle_raw,
                           tech_def_index_raw, progress_raw} (in-progress research)

LOCATION (save-free, self-relocating each session):
  * The country array (rec0/count) is taken from the cached locate (re_scratch/
    _cmisc_rec0.json) when it still verifies live, else relocated via the unique-PP
    method (country_misc_live.locate_via_unique_pp). index->TAG bridged by unique
    political_power against the save (the same Rosetta as economy/country-misc).
  * CTechnologyStatus objects are enumerated by RTTI vtable (module_base + RVA);
    each is bridged to a TAG by its country-object pointer at +0x130.
  * CResearchSlot objects are enumerated by RTTI vtable and grouped by their owner
    CTechnologyStatus pointer at +0x10.

INVARIANTS (Codex-endorsed, enforced by default / fail-closed):
    capacity >= size; size <= MAX_TECHS; slot_count <= MAX_SLOTS; the +0x130 link
    bridges to a known TAG; idle slot handle == 0x2A4E & tech_ptr == 0; active slot
    handle != 0x2A4E & tech_ptr != 0 & handle == CTechnology+0x08. A 2-read sentinel
    guards the status header against mid-read mutation.

BUILD-GATED: refuses to run unless the live exe SHA-256 matches the mapped build,
unless --force is given (same fail-closed gate as locate.py / projects_live.py).

READ-ONLY: PROCESS_VM_READ | PROCESS_QUERY_INFORMATION only; ReadProcessMemory +
VirtualQueryEx; no writes, no injection, game untouched.

Public API
----------
    read_country_tech(p, bridge=None) -> { TAG: {
        "completed_tech_count": int,
        "research_slot_count": int,
        "active_research": [ {"slot_index": int, "tech_handle_raw": int,
                              "tech_def_index_raw": int, "progress_raw": int}, ... ],
        "_status_addr": int,
    } }
  where `bridge` is an optional {running_index: TAG} dict (else built from the save
  PP Rosetta). Tech names are NOT resolved (open ticket: needs CTechnologyDatabase).

Usage
-----
    python tech_live.py                 # locate + dump every bridged country
    python tech_live.py --validate      # compare live vs the save anchor and report
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

EXPECTED_SHA = "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120"

HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000

# country array
REC = 0x120
OFF_NAMEPTR = 0x08
OFF_PP = 0x20
OFF_INDEX = 0x5D
PP_SCALE = 100000.0

# RTTI-resolved pools (module-relative vtable RVAs, build 16022851)
STATUS_VT_RVA = 0x294E288   # CTechnologyStatus (1 per country)
SLOT_VT_RVA = 0x294E238     # CResearchSlot (per active+idle slot)
TECH_VT_RVA = 0x294E218     # CTechnology (researched/active tech record)
# CTechnology vtable FAMILY (the active-tech object may carry either sibling vtable);
# used as a whitelist when validating an active slot's tech pointer (Codex).
TECH_VT_RVA_FAMILY = (0x294E218, 0x294E1C8)

# CTechnologyStatus offsets
ST_COUNTRY_LINK = 0x130     # u64 ptr -> country object ([rec+0x08])
ST_VEC_DATA = 0x40          # u64 ptr -> CTechnology* vector data
ST_VEC_CAP = 0x48           # u32 capacity
ST_VEC_SIZE = 0x4C          # u32 size == completed leveled tech count
ST_SLOT_COUNT = 0xAC        # u32 current research-slot capacity

# CResearchSlot offsets
SLOT_TECH_HANDLE = 0x08     # u32 active-tech-def handle (0x2A4E == idle/empty)
SLOT_OWNER = 0x10           # u64 ptr -> owning CTechnologyStatus
SLOT_TECH_PTR = 0x18        # u64 ptr -> active CTechnology (0 when idle)
SLOT_PROGRESS = 0x38        # u32 progress accumulator (live-only, unvalidated)
SLOT_INDEX = 0x40           # u32 slot position
SLOT_IDLE_HANDLE = 0x2A4E   # sentinel for an empty slot

# CTechnology offsets
TECH_DEF_INDEX = 0x08       # u32 tech-def index (== slot active-tech handle)

# sanity bounds (Codex strict invariants)
MAX_TECHS = 2000
MAX_SLOTS = 32


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


# ==========================================================================
# vtable enumeration (anti-stall advance, region-clamped)
# ==========================================================================
def _enum_vtable(p, vt_abs):
    """Yield every 8-aligned heap address whose qword == vt_abs (object base)."""
    needle = struct.pack("<Q", vt_abs)
    CH = 32 << 20
    PAGE = 0x1000
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
                    pos += max(blen, PAGE)
                else:
                    pos += blen - OV
            else:
                pos += max(want, PAGE)


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


def _locate_country_array(p, save_pp_by_tag):
    """Return (rec0, count). Prefer the cached locate if it still verifies live,
    else relocate via the unique-PP method (country_misc_live)."""
    cache = os.path.join(os.path.dirname(_THIS), "re_scratch", "_cmisc_rec0.json")
    if os.path.exists(cache):
        try:
            c = json.load(open(cache))
            if _verify_rec0(p, c["rec0"]):
                return c["rec0"], c["count"]
        except (OSError, ValueError, KeyError):
            pass
    import country_misc_live as cm
    save_countries = {t: {"political_power": v} for t, v in save_pp_by_tag.items()}
    return cm.locate_via_unique_pp(p, save_countries)


def _build_index_to_tag(p, rec0, count, save_pp_by_tag):
    """running_index -> TAG via unique full-precision political_power."""
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


def _build_obj_to_index(p, rec0, count):
    """[rec+0x08] (country object) -> running_index."""
    obj_idx = {}
    for i in range(count):
        rec = rec0 + i * REC
        try:
            if p.u16(rec + OFF_INDEX) != i:
                continue
            o = p.u64(rec + OFF_NAMEPTR)
        except OSError:
            continue
        if o:
            obj_idx[o] = i
    return obj_idx


# ==========================================================================
# the reader
# ==========================================================================
def _read_status_header(p, st):
    """2-read sentinel of the CTechnologyStatus header fields used downstream.
    Returns (link, cap, size, slot_count) or None on mutation / OSError."""
    try:
        link = p.u64(st + ST_COUNTRY_LINK)
        cap = p.u32(st + ST_VEC_CAP)
        size = p.u32(st + ST_VEC_SIZE)
        sc = p.u32(st + ST_SLOT_COUNT)
        link2 = p.u64(st + ST_COUNTRY_LINK)
        cap2 = p.u32(st + ST_VEC_CAP)
        size2 = p.u32(st + ST_VEC_SIZE)
        sc2 = p.u32(st + ST_SLOT_COUNT)
    except OSError:
        return None
    if (link, cap, size, sc) != (link2, cap2, size2, sc2):
        return None
    return link, cap, size, sc


# ==========================================================================
# warm-walk pin: locate (cold) -> cache addrs -> re-read fields (warm),
# fail-closed validate forces a full re-scan on any move/free/realloc.
# ==========================================================================
# The two _enum_vtable sweeps (CTechnologyStatus + CResearchSlot) are the ~50s
# cost of read_country_tech. enumerate_tech_addrs runs them ONCE; warm callers
# pass the cached addrs to read_country_tech_from_addrs and validate_tech_addrs.
#
# STABLE-ID (ABA defense) chosen per object class -- both are clean per-object
# stable ids on a PAUSED game (HARD invariant: no alloc/free between reads):
#   * CTechnologyStatus -> its country-link pointer (+0x130). One status object
#     per country; the country object is stable; this same pointer is what the
#     country bridge (obj_idx) keys on, so it is the natural identity.
#   * CResearchSlot     -> (owner_status_ptr +0x10, slot_index +0x40). The owner
#     pointer + the slot's own position make a world-unique pair (a country has
#     <= MAX_SLOTS slots, each at a distinct index), defeating cross-object ABA
#     where a freed slot's address is reused by a DIFFERENT country's slot.
# These ids are exactly the fields the decode bridges on, so a stable id == the
# decode still bridging to the same place; a changed id == the cache is stale.


def _strict_int(x):
    """Coerce to a plain int or None (never raises). bool is rejected (it is an
    int subclass but never a valid addr/id here)."""
    try:
        if isinstance(x, bool):
            return None
        if isinstance(x, int):
            return int(x)
    except Exception:
        return None
    return None


def _split_addr_id(item):
    """Accept a cached entry as a bare int addr or an (addr, stable_id) pair.
    Exception-proof: returns (addr_or_None, id_tuple_or_None) for ANY input,
    never raising. The stable id enumerate stores is always a tuple of plain
    ints; ANY other shape -> None so callers fail CLOSED rather than raise."""
    try:
        if type(item) is int:
            return item, None
        if not isinstance(item, (tuple, list)) or len(item) == 0:
            return None, None
        addr = _strict_int(item[0])
        if len(item) < 2:
            return addr, None
        sid = item[1]
        if not isinstance(sid, (tuple, list)):
            return addr, None
        members = [_strict_int(x) for x in sid]
        if any(m is None for m in members):
            return addr, None
        return addr, tuple(members)
    except Exception:
        return None, None


def _status_stable_id(p, st):
    """(country_link_ptr,) for a CTechnologyStatus -- the strongest stable per-object
    id (one status per country). Returns None on OSError."""
    try:
        return (p.u64(st + ST_COUNTRY_LINK),)
    except OSError:
        return None


def _slot_stable_id(p, sl):
    """(owner_status_ptr, slot_index) for a CResearchSlot -- a world-unique pair.
    Returns None on OSError."""
    try:
        return (p.u64(sl + SLOT_OWNER), p.u32(sl + SLOT_INDEX))
    except OSError:
        return None


def enumerate_tech_addrs(p):
    """COLD locate: the two _enum_vtable sweeps ONLY (no decode, no bridging).
    Returns {"status_addrs": [(addr, stable_id), ...],
             "slot_addrs":   [(addr, stable_id), ...]} in deterministic
    _enum_vtable ORDER -- i.e. the EXACT order the original read_country_tech
    iterated objects (region order, ascending address within each region). This
    is preserved (NOT re-sorted by address) so the cold path stays byte-identical
    to the original even in the pathological duplicate-tag case where the LAST
    status object for a TAG wins out[tag] (one status per country on a paused game
    makes that vacuous, but order preservation makes byte-identity unconditional).
    stable_id is the ABA-defense tuple documented above (status: country-link ptr;
    slot: (owner ptr, slot_index)). An object whose stable id cannot be read at
    enumerate time is dropped here (it could not be validated later anyway -- and
    the original would have OSError-skipped it downstream too)."""
    status_vt = p.module_base + STATUS_VT_RVA
    slot_vt = p.module_base + SLOT_VT_RVA
    status_addrs = []
    for st in _enum_vtable(p, status_vt):
        sid = _status_stable_id(p, st)
        if sid is None:
            continue
        status_addrs.append((st, sid))
    slot_addrs = []
    for sl in _enum_vtable(p, slot_vt):
        sid = _slot_stable_id(p, sl)
        if sid is None:
            continue
        slot_addrs.append((sl, sid))
    return {"status_addrs": status_addrs, "slot_addrs": slot_addrs}


def _build_tech_bridges(p, bridge, rec0, count):
    """Shared bridge construction for read_country_tech / *_from_addrs. Returns
    (bridge, obj_idx). Mirrors the original prologue EXACTLY."""
    if (rec0 is None) != (count is None):
        raise ValueError("read_country_tech: rec0 and count must be supplied together")
    if bridge is None:
        pp_path = os.path.join(os.path.dirname(_THIS), "re_scratch", "_country_pp.json")
        save_pp = json.load(open(pp_path))
        if rec0 is None:
            rec0, count = _locate_country_array(p, save_pp)
        bridge = _build_index_to_tag(p, rec0, count, save_pp)
    elif rec0 is None:
        # caller supplied a bridge but no live base; we still need rec0/count for obj->index
        pp_path = os.path.join(os.path.dirname(_THIS), "re_scratch", "_country_pp.json")
        save_pp = json.load(open(pp_path)) if os.path.exists(pp_path) else {}
        rec0, count = _locate_country_array(p, save_pp)
    obj_idx = _build_obj_to_index(p, rec0, count)
    return bridge, obj_idx


def _decode_tech(p, status_iter, slot_iter, bridge, obj_idx, strict):
    """Shared decode: iterate status addrs then slot addrs, re-reading EVERY field
    LIVE, applying the identical strict gates / drop logic / active_research shape
    and sort. `status_iter`/`slot_iter` yield bare object addresses (int). This is
    the single source of truth for both the cold (enumerate) and warm (cached) paths
    -- the output is byte-identical regardless of where the addrs came from."""
    addr_to_tag = {}     # status_addr -> TAG (for slot grouping)
    out = {}
    for st in status_iter:
        hdr = _read_status_header(p, st)
        if hdr is None:
            continue
        link, cap, size, slot_count = hdr
        idx = obj_idx.get(link)
        if idx is None:
            continue                         # unbridged status (minor/sentinel) -> skip
        tag = bridge.get(idx)
        if tag is None:
            continue
        if strict:
            if not (cap >= size and size <= MAX_TECHS and slot_count <= MAX_SLOTS):
                continue                     # invariant break -> fail closed
            if size > 0 and not heap_ok(p_safe_u64(p, st + ST_VEC_DATA)):
                continue
        addr_to_tag[st] = tag
        out[tag] = {
            "completed_tech_count": size,
            "research_slot_count": slot_count,
            "active_research": [],
            "_status_addr": st,
        }

    # --- CResearchSlot: attach active ones to their country ---
    tech_vt_family = {p.module_base + r for r in TECH_VT_RVA_FAMILY}
    for sl in slot_iter:
        try:
            owner = p.u64(sl + SLOT_OWNER)
        except OSError:
            continue
        tag = addr_to_tag.get(owner)
        if tag is None:
            continue
        try:
            handle = p.u32(sl + SLOT_TECH_HANDLE)
            tech_ptr = p.u64(sl + SLOT_TECH_PTR)
        except OSError:
            continue
        if handle == SLOT_IDLE_HANDLE or tech_ptr == 0:
            continue                         # idle/empty slot
        if strict:
            if not heap_ok(tech_ptr):
                continue
            # active-slot invariant: the active-tech object carries a CTechnology
            # FAMILY vtable AND its +0x08 tech-def index == the slot handle. Both
            # must hold, else this is a valid-looking wrong pointer -> drop the slot.
            try:
                tvt = p.u64(tech_ptr)
                def_idx = p.u32(tech_ptr + TECH_DEF_INDEX)
            except OSError:
                continue
            if tvt not in tech_vt_family or def_idx != handle:
                continue                     # inconsistent -> drop this slot
        else:
            try:
                def_idx = p.u32(tech_ptr + TECH_DEF_INDEX)
            except OSError:
                def_idx = None
        try:
            progress = p.u32(sl + SLOT_PROGRESS)
            slot_index = p.u32(sl + SLOT_INDEX)
        except OSError:
            continue
        out[tag]["active_research"].append({
            "slot_index": slot_index,
            "tech_handle_raw": handle,
            "tech_def_index_raw": def_idx,
            "progress_raw": progress,
        })

    for d in out.values():
        d["active_research"].sort(key=lambda s: s["slot_index"])
    return out


def read_country_tech_from_addrs(p, status_addrs, slot_addrs, bridge=None,
                                 strict=True, rec0=None, count=None):
    """WARM walk: produce the SAME {TAG: {...}} dict read_country_tech returns, but
    iterate the SUPPLIED status_addrs/slot_addrs (from enumerate_tech_addrs, typically
    after validate_tech_addrs passed) instead of running the two _enum_vtable sweeps.
    Every field is RE-READ LIVE. status_addrs/slot_addrs items may be bare int addrs or
    (addr, stable_id) tuples (only the addr is used here; the id is for validate). The
    country bridges (idx_tag / obj_idx) are built EXACTLY as read_country_tech does."""
    bridge, obj_idx = _build_tech_bridges(p, bridge, rec0, count)

    def _addrs(seq):
        for item in seq:
            a, _sid = _split_addr_id(item)
            if a is not None:
                yield a

    return _decode_tech(p, _addrs(status_addrs), _addrs(slot_addrs),
                        bridge, obj_idx, strict)


def read_country_tech(p, bridge=None, strict=True, rec0=None, count=None):
    """LIVE per-country research/tech state. Returns {TAG: {...}}.

    `bridge` (optional) = {running_index: TAG}. If None, it is built from the save
    PP Rosetta (re_scratch/_country_pp.json). `strict` enforces the Codex invariants
    and drops any country whose status object fails them (fail-closed).

    `rec0`/`count` (optional, supplied TOGETHER): the country-array base + count FROM THE LIVE
    CountryIndexBridge. When given, _locate_country_array is SKIPPED in BOTH branches -- removing
    the redundant relocate AND the stale-save-PP relocate that raised (LocateError) and killed
    the whole walk. The bridge base is the live-validated array (authoritative for live capture).

    Body: build the country bridges, run the two cold _enum_vtable sweeps via
    enumerate_tech_addrs, then hand the located addrs to the SHARED decoder. The warm
    path (read_country_tech_from_addrs) feeds cached addrs to that same decoder, so the
    two paths are byte-identical.
    """
    bridge, obj_idx = _build_tech_bridges(p, bridge, rec0, count)
    located = enumerate_tech_addrs(p)
    status_iter = (a for a, _sid in located["status_addrs"])
    slot_iter = (a for a, _sid in located["slot_addrs"])
    return _decode_tech(p, status_iter, slot_iter, bridge, obj_idx, strict)


def validate_tech_addrs(p, status_addrs, slot_addrs):
    """FAIL-CLOSED liveness gate for cached tech addresses (cheap per-addr RPM, NO
    sweep). Returns True only if EVERY supplied addr still points at the SAME live object:

      * each status_addr: vtable(+0x00) == module_base+STATUS_VT_RVA, the 2-read header
        sentinel (_read_status_header) is stable, AND the country-link stable id equals
        the cold id captured by enumerate (ABA defense -- a same-class object reused at
        this addr by ANOTHER country would carry a different country-link ptr).
      * each slot_addr: vtable(+0x00) == module_base+SLOT_VT_RVA AND the (owner_ptr,
        slot_index) stable id equals the cold id (cross-country ABA defense).

    A bare-int entry (no cold stable id -> ABA-indefensible), a short/unreadable read,
    a moved/freed/realloc'd object, or ANY id mismatch -> False, forcing the caller to
    re-run enumerate_tech_addrs (full re-scan).

    SCOPE (Codex-flagged -- be precise): this proves every SUPPLIED addr is the unchanged
    object it was; it CANNOT by itself prove POOL COMPLETENESS (that no NEW status/slot
    appeared, or that none was dropped from the cached list) because it does NO sweep. That
    completeness guarantee is owned by the caller's HARD PAUSED invariant (no object
    alloc/free between reads -> the live pool's membership is fixed). To keep this gate from
    silently passing a degenerate/emptied cache, an EMPTY status pool -> False (a live game
    always has CTechnologyStatus objects; an empty status list means the cache was never
    populated or was cleared -> re-scan). An empty slot pool is allowed (a game with no
    in-progress research legitimately has zero ACTIVE slots, but enumerate keeps idle slots
    too -- see note). Under the paused invariant, a True here means the cached objects are
    byte-for-byte the same ones enumerate located, so the warm walk reproduces the cold walk."""
    if not status_addrs:
        return False        # empty status pool -> cache unpopulated/cleared -> re-scan
    status_vt = p.module_base + STATUS_VT_RVA
    slot_vt = p.module_base + SLOT_VT_RVA

    for item in status_addrs:
        a, cold_id = _split_addr_id(item)
        if a is None or cold_id is None:
            return False                     # no cold id -> cannot defend ABA -> fail closed
        try:
            if p.u64(a) != status_vt:
                return False
        except OSError:
            return False
        if _read_status_header(p, a) is None:
            return False                     # header mutated/unreadable -> stale
        live_id = _status_stable_id(p, a)
        if live_id is None or live_id != cold_id:
            return False

    for item in slot_addrs:
        a, cold_id = _split_addr_id(item)
        if a is None or cold_id is None:
            return False
        try:
            if p.u64(a) != slot_vt:
                return False
        except OSError:
            return False
        live_id = _slot_stable_id(p, a)
        if live_id is None or live_id != cold_id:
            return False

    return True


def p_safe_u64(p, a):
    try:
        return p.u64(a)
    except OSError:
        return 0


# ==========================================================================
# validation vs the lockstep save
# ==========================================================================
def _load_save_tech_gt():
    """Return {TAG: {leveled, total, research_slot}} from the melt-derived GT files."""
    scr = os.path.join(os.path.dirname(_THIS), "re_scratch")
    gt2 = json.load(open(os.path.join(scr, "_tech_gt2.json")))
    slots = json.load(open(os.path.join(scr, "_tech_slots_gt.json")))
    out = {}
    for tag, g in gt2.items():
        out[tag] = {
            "leveled": g.get("leveled"),
            "total": g.get("total"),
            "research_slot": (slots.get(tag) or {}).get("research_slot"),
        }
    return out


def _validate(p):
    """Read live, compare to the save anchor, return a one-screen report dict."""
    gt = _load_save_tech_gt()
    live = read_country_tech(p, strict=True)

    comp_ok = comp_n = 0
    slot_ok = slot_n = 0
    act_ok = act_n = 0
    comp_bad = []
    slot_bad = []
    for tag, d in live.items():
        g = gt.get(tag)
        if not g:
            continue
        # completed leveled tech count
        if g["leveled"] is not None:
            comp_n += 1
            if d["completed_tech_count"] == g["leveled"]:
                comp_ok += 1
            else:
                comp_bad.append((tag, d["completed_tech_count"], g["leveled"]))
        # slot capacity
        if g["research_slot"] is not None:
            slot_n += 1
            if d["research_slot_count"] == g["research_slot"]:
                slot_ok += 1
            else:
                slot_bad.append((tag, d["research_slot_count"], g["research_slot"]))
            # active-slot count (allow idle: active <= research_slot, and == when researching)
            act_n += 1
            n_active = len(d["active_research"])
            if n_active == g["research_slot"] or n_active == 0:
                act_ok += 1

    return {
        "live_countries": len(live),
        "completed_tech_count_vs_save_leveled": "%d/%d exact" % (comp_ok, comp_n),
        "research_slot_count_vs_save": "%d/%d exact" % (slot_ok, slot_n),
        "active_slot_count_consistent": "%d/%d (== research_slot, or 0 = all idle)" % (act_ok, act_n),
        "completed_mismatches": comp_bad[:10],
        "slot_mismatches": slot_bad[:10],
        "total_active_research_slots_read": sum(len(d["active_research"]) for d in live.values()),
    }


def main():
    ap = argparse.ArgumentParser(description="LIVE per-country research/tech reader (read-only).")
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
        live = read_country_tech(p, strict=True)
        print("\n%-4s %6s %6s  %s" % ("TAG", "techs", "slots", "active (slot:handle prog)"))
        for tag in sorted(live):
            d = live[tag]
            act = " ".join("%d:0x%X(%d)" % (a["slot_index"], a["tech_handle_raw"], a["progress_raw"])
                           for a in d["active_research"])
            print("%-4s %6d %6d  %s" % (tag, d["completed_tech_count"],
                                        d["research_slot_count"], act))
        print("\ncountries read: %d" % len(live))


if __name__ == "__main__":
    main()
