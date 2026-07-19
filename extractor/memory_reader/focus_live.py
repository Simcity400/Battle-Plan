"""focus_live.py - LIVE per-country NATIONAL FOCUS reader (read-only).

Reads, straight from a running PAUSED hoi4.exe (no save at runtime), each country's
national-focus state pinned on build 16022851 (see focus_offsets.draft.json). The focus
state lives in a per-country focus submanager (a CFocusStatus-like object) hanging off
the big country object at country_object + 0x1380:

    focus_tree        [country_object+0x1370] -> CFocusTree def, id std::string @ def+0x08
                      ("german_focus", "soviet_focus", ...)            20/20 EXACT vs save
    completed         Pdx-vector {data @ submgr+0x40, cap @ +0x48, size @ +0x4C};
                      element = 8-byte ptr -> CFocus def, id std::string @ def+0x18.
                      completed_count == size                          22/22 EXACT (count)
                      completed list (set AND order)                   22/22 EXACT
    current           [submgr+0x10] -> active NATIONAL-focus object (0 when none);
                      focus id std::string @ obj+0x18                  9/9 EXACT vs save
    current_continuous[submgr+0x18] -> active CONTINUOUS-focus object (0 when none);
                      focus id std::string @ obj+0x18                  13/13 EXACT vs save
    progress          submgr+0x38  i32 /100000 = DAYS into the active focus
                      (national or continuous)                         19/19 EXACT vs save

National-current and continuous-current are mutually exclusive in the live heap exactly
as in the save (a country has current= XOR current_continuous=); progress @+0x38 applies
to whichever set is active.

LOCATION (save-free, self-relocating each session):
  * The country array (rec0/count) is taken from the cached locate (re_scratch/
    _cmisc_rec0.json) when it still verifies live, else relocated via the unique-PP
    method (country_misc_live.locate_via_unique_pp). country object = u64[rec+0x08];
    running_index -> TAG bridged by unique full-precision political_power vs the save
    (the same Rosetta as economy / tech / country-misc).
  * NO RTTI pool scan is needed: the submanager is reached by a fixed offset off each
    country object, so the reader only walks the country array (fast, ~no heap scan).

OPEN TICKETS (reported honestly, never faked):
  * paused (save bool paused=yes/no): every active country reads paused=no this frame, so
    there is NO paused=yes ground truth to pin the live byte against. Left UNRESOLVED
    (the live offset is not asserted); the reader exposes paused=None with a documented
    reason rather than synthesizing False. See draft open_tickets.
  * available_focuses (a second Pdx-vector @ submgr+0x78 of NOT-completed focus defs,
    size ~19-42 per major): structurally decoded and id-resolvable, but the save carries
    no field for it, so its precise semantic (selectable/available vs candidate) is
    UNVALIDATED. Exposed as available_focuses_candidate behind include_candidate=True,
    flagged unvalidated. Not part of the validated deliverable.
  * current focus PROGRESS as a fraction/percent: +0x38 is days-elapsed (matches save
    progress=, EXACT). The 0..1 completion fraction is serialize/UI-computed from the
    focus cost & modifiers; not a single stored field. days-elapsed is the honest stored
    input and is exposed.

BUILD-GATED: refuses to run unless the live exe SHA-256 matches the mapped build, unless
--force is given (same fail-closed gate as locate.py / tech_live.py / intel_live.py).

READ-ONLY: PROCESS_VM_READ | PROCESS_QUERY_INFORMATION only; ReadProcessMemory +
VirtualQueryEx; no writes, no injection, game untouched, MP-safe.

Public API
----------
    read_country_focus(p, bridge=None, include_candidate=False) -> { TAG: {
        "focus_tree": str | None,
        "completed_count": int,
        "completed": [focus_id, ...],          # ordered, == save order
        "current": focus_id | None,            # active NATIONAL focus
        "current_set": [focus_id] | [],        # exact measured set; None = read failure
        "current_continuous": focus_id | None, # active CONTINUOUS focus
        "current_continuous_set": [focus_id] | [], # exact measured set; None = read failure
        "progress_days": float | None,         # days into the active focus
        "paused": None,                        # open ticket (no paused=yes this frame)
        "_submgr_addr": int,
        # only when include_candidate=True (UNVALIDATED):
        "available_focuses_candidate": [focus_id, ...],
    } }
  where `bridge` is an optional {running_index: TAG} dict (else built from the save PP
  Rosetta in re_scratch/_country_pp.json).

Usage
-----
    python focus_live.py                 # locate + dump every bridged country
    python focus_live.py --validate      # compare live vs the save anchor and report
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

# country-object -> focus fields
OBJ_FOCUS_TREE = 0x1370   # u64 ptr -> CFocusTree def (id std::string @ def + TREE_ID)
OBJ_SUBMGR = 0x1380       # u64 ptr -> focus submanager (CFocusStatus-like)
TREE_ID = 0x08            # std::string focus_tree id within the CFocusTree def

# focus submanager (off the submgr base)
SM_CURRENT = 0x10         # u64 ptr -> active NATIONAL-focus object (0 = none)
SM_CONTINUOUS = 0x18      # u64 ptr -> active CONTINUOUS-focus object (0 = none)
SM_PROGRESS = 0x38        # i32 /100000 = days into the active focus
SM_COMPLETED_DATA = 0x40  # u64 ptr -> completed-focus vector data
SM_COMPLETED_CAP = 0x48   # u32 capacity
SM_COMPLETED_SIZE = 0x4C  # u32 size == completed_count
SM_CANDIDATE_DATA = 0x78  # u64 ptr -> not-completed ("available") vector data (UNVALIDATED)
SM_CANDIDATE_SIZE = 0x80  # u32 size (UNVALIDATED). +0x84 is NOT a verified capacity.

# focus objects/defs
FOCUS_OBJ_ID = 0x18       # std::string focus id within an active-focus object
FOCUS_DEF_ID = 0x18       # std::string focus id within a CFocus def (completed/candidate element)
ELEM_STRIDE = 8           # completed/candidate element = one 8-byte ptr -> CFocus def

# sanity bounds
MAX_FOCUSES = 4000
MAX_ID_LEN = 128


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


def _safe_u64(p, a):
    try:
        return p.u64(a)
    except OSError:
        return 0


# ==========================================================================
# region-clamped string read (a vector header / std::string can straddle a
# VirtualQueryEx region boundary; clamp to the region or the whole span reads 0)
# ==========================================================================
def _region_end(p, addr):
    import ctypes as C
    mbi = winmem.MEMORY_BASIC_INFORMATION()
    if winmem._k32.VirtualQueryEx(p.handle, C.c_void_p(addr), C.byref(mbi),
                                  C.sizeof(mbi)) == 0:
        return None
    return int(mbi.BaseAddress or 0) + int(mbi.RegionSize)


def _cread(p, addr, want):
    e = _region_end(p, addr)
    if e is None:
        return b""
    nn = min(want, e - addr)
    return p.try_read(addr, nn) if nn > 0 else b""


def _read_id_str(p, base):
    """Read an MSVC std::string focus id (SSO decided by capacity @ +0x18), clamped."""
    sb = _cread(p, base + 0x10, 8)
    cb = _cread(p, base + 0x18, 8)
    if len(sb) < 8 or len(cb) < 8:
        return None
    sz = struct.unpack("<Q", sb)[0]
    cap = struct.unpack("<Q", cb)[0]
    if sz == 0 or sz > MAX_ID_LEN or cap < sz:
        return None
    if cap < 16:
        data = _cread(p, base, sz)
    else:
        pb = _cread(p, base, 8)
        if len(pb) < 8:
            return None
        ptr = struct.unpack("<Q", pb)[0]
        if not heap_ok(ptr):
            return None
        data = _cread(p, ptr, sz)
    if len(data) < sz:
        return None
    try:
        s = data[:sz].decode("latin-1")
    except ValueError:
        return None
    return s if all(32 <= ord(c) < 127 for c in s) else None


# ==========================================================================
# index -> TAG bridge (unique political_power Rosetta against the save)
# ==========================================================================
def _verify_rec0(p, rec0, depth=24):
    """The 0..depth running-index column is clean. (The first records of this build's
    country array are placeholder/sentinel slots whose [rec+0x08] is NOT a country
    object, so a localized-name check on the head of the array would wrongly reject a
    live base; the real liveness proof here is the unique-PP bridge below, which
    requires 46 distinct full-precision political_power values to each land on the
    correct record - far stronger than any head-of-array fingerprint.)"""
    try:
        for k in range(depth):
            if p.u16(rec0 + k * REC + OFF_INDEX) != k:
                return False
    except OSError:
        return False
    return True


def _bridge_is_live(p, rec0, idx_tag, save_pp_by_tag, min_quorum=20):
    """Codex hardening for cached-rec0 reuse: re-read the political_power of each
    bridged record and confirm it STILL equals the save value (and stays equal on a
    second read). A stale same-build heap copy would not carry the live PP. Returns
    True iff >= min_quorum bridged records re-verify (the count itself - 46 here - is
    the liveness signal a bare index column can't fake)."""
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
    """running_index -> TAG via unique full-precision political_power (resolves the
    1946 puppet/cosmetic-name collisions that a localized-name table would mis-bridge,
    e.g. Nationalist Spain vs the exile Spains)."""
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
def _read_vec_ids(p, data, size, def_id_off, _cache):
    """Resolve a Pdx-vector of 8-byte CFocus-def pointers to focus id strings.
    `_cache` maps def_ptr -> id (defs are long-lived/static; cache per call)."""
    ids = []
    blob = _cread(p, data, size * ELEM_STRIDE) if size else b""
    for k in range(size):
        if (k + 1) * ELEM_STRIDE <= len(blob):
            ev = struct.unpack_from("<Q", blob, k * ELEM_STRIDE)[0]
        else:
            ep = _cread(p, data + k * ELEM_STRIDE, ELEM_STRIDE)
            ev = struct.unpack("<Q", ep)[0] if len(ep) >= ELEM_STRIDE else 0
        if not heap_ok(ev):
            ids.append(None)
            continue
        s = _cache.get(ev)
        if s is None:
            s = _read_id_str(p, ev + def_id_off)
            if s is not None:
                _cache[ev] = s
        ids.append(s)
    return ids


def read_country_focus(p, bridge=None, include_candidate=False, strict=True,
                       rec0=None, count=None):
    """LIVE per-country national-focus state. Returns {TAG: {...}}.

    `bridge` (optional) = {running_index: TAG}. If None, it is built from the save
    PP Rosetta (re_scratch/_country_pp.json). `include_candidate=True` adds the
    UNVALIDATED available_focuses_candidate list. `strict` enforces the structural
    invariants (capacity >= size, heap-range pointers) and skips a country whose
    submanager fails them (fail-closed).

    `rec0`/`count` (optional, MUST be supplied together): the country-array base + record
    count taken FROM THE LIVE CountryIndexBridge (CountryIndexBridge.rec0/.count). When given,
    the reader SKIPS _locate_country_array entirely -- this both removes a redundant full-heap
    relocate AND, critically, avoids the save-PP anchor relocate that raises (stale PP vs live
    memory) and previously killed the whole walk. The bridge base is the live-validated array
    (locate.locate's structural+name quorum); it is the authoritative source for live capture.
    When ONLY ONE of rec0/count is supplied that is a caller bug -> fail closed."""
    if (rec0 is None) != (count is None):
        raise ValueError("read_country_focus: rec0 and count must be supplied together")
    pp_path = os.path.join(os.path.dirname(_THIS), "re_scratch", "_country_pp.json")
    save_pp = json.load(open(pp_path)) if os.path.exists(pp_path) else {}
    if rec0 is None:
        # no live bridge base -> self-locate (cached rec0 if it re-verifies, else PP relocate).
        rec0, count = _locate_country_array(p, save_pp)
    if bridge is None:
        bridge = _build_index_to_tag(p, rec0, count, save_pp)
        # Confirm the (possibly cached) rec0 is the LIVE array via a PP quorum re-read
        # (Codex hardening): a stale same-build copy would fail the live PP match.
        if save_pp and not _bridge_is_live(p, rec0, bridge, save_pp):
            import country_misc_live as cm
            save_countries = {t: {"political_power": v} for t, v in save_pp.items()}
            rec0, count = cm.locate_via_unique_pp(p, save_countries)
            bridge = _build_index_to_tag(p, rec0, count, save_pp)

    def_cache = {}
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

        sm = _safe_u64(p, obj + OBJ_SUBMGR)
        if not heap_ok(sm):
            continue

        # --- completed vector (the high-variance anchor) ---
        try:
            comp_data = p.u64(sm + SM_COMPLETED_DATA)
            comp_cap = p.u32(sm + SM_COMPLETED_CAP)
            comp_size = p.u32(sm + SM_COMPLETED_SIZE)
        except OSError:
            continue
        if strict and not (comp_cap >= comp_size and comp_size <= MAX_FOCUSES):
            continue
        if strict and comp_size > 0 and not heap_ok(comp_data):
            continue
        completed = _read_vec_ids(p, comp_data, comp_size, FOCUS_DEF_ID, def_cache)

        # --- focus tree id ---
        tree_def = _safe_u64(p, obj + OBJ_FOCUS_TREE)
        focus_tree = _read_id_str(p, tree_def + TREE_ID) if heap_ok(tree_def) else None

        # --- current national / continuous focus (mutually exclusive) ---
        nat = _safe_u64(p, sm + SM_CURRENT)
        cont = _safe_u64(p, sm + SM_CONTINUOUS)
        nat_set = heap_ok(nat)
        cont_set = heap_ok(cont)
        current = _read_id_str(p, nat + FOCUS_OBJ_ID) if nat_set else None
        current_continuous = _read_id_str(p, cont + FOCUS_OBJ_ID) if cont_set else None
        current_set = ([current] if current is not None else None) if nat_set else (
            [] if nat == 0 else None)
        current_continuous_set = (
            [current_continuous] if current_continuous is not None else None
        ) if cont_set else ([] if cont == 0 else None)

        # --- progress (days into the active focus); only meaningful when exactly one
        # of the two focus pointers is set (the live XOR invariant), so we never expose
        # the stale/idle value an inactive submanager carries (Codex: gate on the raw
        # pointer, not the resolved id) ---
        progress_days = None
        if nat_set != cont_set:
            try:
                progress_days = p.i32(sm + SM_PROGRESS) / PP_SCALE
            except OSError:
                progress_days = None

        rec_out = {
            "focus_tree": focus_tree,
            "completed_count": comp_size,
            "completed": completed,
            "current": current,
            "current_set": current_set,
            "current_continuous": current_continuous,
            "current_continuous_set": current_continuous_set,
            "progress_days": progress_days,
            "paused": None,   # OPEN TICKET: no paused=yes ground truth this frame
            "_submgr_addr": sm,
        }

        if include_candidate:
            # UNVALIDATED. Header is {data @ +0x78, size @ +0x80, <unknown> @ +0x84};
            # +0x84 is NOT a verified capacity (observed < size, e.g. ENG size=19 +0x84=5),
            # so we guard on heap data + a sane size only, never cap>=size.
            try:
                cd = p.u64(sm + SM_CANDIDATE_DATA)
                cn = p.u32(sm + SM_CANDIDATE_SIZE)
            except OSError:
                cd = cn = 0
            if heap_ok(cd) and 0 < cn <= MAX_FOCUSES:
                ids = _read_vec_ids(p, cd, cn, FOCUS_DEF_ID, def_cache)
                # only keep if it actually resolved to focus ids (fail-closed on garbage)
                rec_out["available_focuses_candidate"] = (
                    ids if sum(1 for x in ids if x) >= max(1, cn // 2) else [])
            else:
                rec_out["available_focuses_candidate"] = []

        out[tag] = rec_out
    return out


# ==========================================================================
# validation vs the lockstep save
# ==========================================================================
def _load_save_focus_gt():
    """{TAG: {focus_tree, completed[], completed_count, current, progress, current_continuous}}
    from the save oracle + the raw-save current_continuous derivation."""
    root = os.path.dirname(_THIS)
    gt = json.load(open(os.path.join(root, "save_oracle", "output", "focus.json")))["countries"]
    cc_path = os.path.join(root, "re_scratch", "_focus_curcont_gt.json")
    cc = json.load(open(cc_path)) if os.path.exists(cc_path) else {}
    for tag, d in gt.items():
        d["current_continuous"] = cc.get(tag)
    return gt


def _validate(p):
    """Read live, compare to the save anchor, return a one-screen report dict."""
    gt = _load_save_focus_gt()
    live = read_country_focus(p, strict=True)

    cnt_ok = cnt_n = 0
    list_ok = list_n = 0
    tree_ok = tree_n = 0
    cur_ok = cur_n = 0
    cc_ok = cc_n = 0
    prog_ok = prog_n = 0
    cnt_bad = []
    list_bad = []
    for tag, d in live.items():
        g = gt.get(tag)
        if not g:
            continue
        # completed count
        cnt_n += 1
        if d["completed_count"] == g["completed_count"]:
            cnt_ok += 1
        else:
            cnt_bad.append((tag, d["completed_count"], g["completed_count"]))
        # completed list (set + order) for countries that have any
        if g["completed_count"] > 0:
            list_n += 1
            if d["completed"] == g["completed"]:
                list_ok += 1
            else:
                live_set = set(x for x in d["completed"] if x)
                list_bad.append((tag, sorted(live_set - set(g["completed"]))[:2],
                                 sorted(set(g["completed"]) - live_set)[:2]))
        # focus tree
        if g.get("focus_tree"):
            tree_n += 1
            if d["focus_tree"] == g["focus_tree"]:
                tree_ok += 1
        # current national focus
        if g.get("current"):
            cur_n += 1
            if d["current"] == g["current"]:
                cur_ok += 1
        # current continuous focus
        if g.get("current_continuous"):
            cc_n += 1
            if d["current_continuous"] == g["current_continuous"]:
                cc_ok += 1
        # progress days (only when a focus is active in the save)
        if g.get("progress") is not None and (g.get("current") or g.get("current_continuous")):
            prog_n += 1
            if d["progress_days"] is not None and abs(d["progress_days"] - g["progress"]) < 0.5:
                prog_ok += 1

    return {
        "live_countries": len(live),
        "completed_count_vs_save": "%d/%d exact" % (cnt_ok, cnt_n),
        "completed_list_set_and_order_vs_save": "%d/%d exact" % (list_ok, list_n),
        "focus_tree_vs_save": "%d/%d exact" % (tree_ok, tree_n),
        "current_focus_vs_save": "%d/%d exact" % (cur_ok, cur_n),
        "current_continuous_vs_save": "%d/%d exact" % (cc_ok, cc_n),
        "progress_days_vs_save": "%d/%d exact" % (prog_ok, prog_n),
        "completed_count_mismatches": cnt_bad[:10],
        "completed_list_mismatches": list_bad[:10],
        "paused": "OPEN TICKET (all active countries paused=no this frame; no paused=yes "
                  "ground truth to pin the live byte; reader exposes paused=None)",
    }


def main():
    ap = argparse.ArgumentParser(description="LIVE per-country national-focus reader (read-only).")
    ap.add_argument("--force", action="store_true",
                    help="run even if the exe SHA-256 doesn't match the mapped build")
    ap.add_argument("--validate", action="store_true",
                    help="compare live reads to the save anchor and report match rate")
    ap.add_argument("--candidate", action="store_true",
                    help="also dump the UNVALIDATED available_focuses_candidate vector")
    args = ap.parse_args()

    p = winmem.open_process("hoi4.exe")
    if p is None:
        raise SystemExit("hoi4.exe not running")
    with p:
        check_build_gate(p, args.force)
        if args.validate:
            print(json.dumps(_validate(p), indent=1))
            return
        live = read_country_focus(p, include_candidate=args.candidate)
        print("\n%-4s %-22s %5s  %-30s %-34s %s"
              % ("TAG", "tree", "done", "current", "current_continuous", "prog"))
        for tag in sorted(live):
            d = live[tag]
            prog = "" if d["progress_days"] is None else "%.0fd" % d["progress_days"]
            print("%-4s %-22s %5d  %-30s %-34s %s"
                  % (tag, (d["focus_tree"] or "-")[:22], d["completed_count"],
                     (d["current"] or "-")[:30], (d["current_continuous"] or "-")[:34], prog))
        print("\ncountries read: %d" % len(live))


if __name__ == "__main__":
    main()
