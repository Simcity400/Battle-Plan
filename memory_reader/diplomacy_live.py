"""diplomacy_live.py - READ-ONLY live DIPLOMACY reader for Battle Plan.

Reads, save-free, from a running PAUSED hoi4.exe via ReadProcessMemory only
(READ-ONLY; no writes / no injection / no thread suspension, so it never desyncs
MP), each country's diplomatic state:

  * WARS + AT-WAR set  -- who is at war with whom (per-country enemy set).
  * CASUALTIES         -- per-pair war casualties + per-country cumulative total.
  * FACTIONS           -- formal faction membership (the 2 real factions) by the
                          country diplomatic-group id, exactly matching the save.

Country identity (save-free, self-relocating each session):
  * The country DB index-fingerprint locate() AND its owner-check both FAILED this
    session (heap moved / stale container layout), so we locate country objects by
    the country-object VTABLE (RVA 0x27C0E80, intel_live.py technique) and join them
    to TAGs via the tag-db-id bridge array fingerprinted by the anchor majors.

PINNED structures (see diplomacy_offsets.draft.json), validated live==save on build
16022851 against the lockstep melt (save_date 1946.5.19.9):

  war_relation struct (global pool, header = the tag pair):
      +0x00 first_country  tag_db_id (i32)
      +0x04 second_country tag_db_id (i32)
      +0x40 first_casualties  (i32)
      +0x48 second_casualties (i32)
      +0x68 first/second tag_db_id ECHO  (disambiguating signature)
    -> casualty pairs 144/144 EXACT, at-war set 30/30 EXACT, per-country totals 30/30.

  country object:
      +0x598 faction_group_id (i32)  -- members of one formal faction share a value
                                        (Allies/Comintern matched the save 24/24).

OPEN (reported, not faked -- save oracle is authoritative for these this build):
  * opinion / relation flags (guarantee/NAP/military_access/embargo): the relations
    container is a hash_map keyed by country handle and cached_sum (=opinion) is a
    computed/modifier-derived aggregate, not stored adjacent to the key.
  * subjects/autonomy: no single canonical country-object field across subject types;
    held in a diplomacy/autonomy manager. +0x114C caches the overlord tag_db_id for
    Comintern-bloc puppets only.
  * formal faction OBJECT (name/leader/member-vector): membership is reported via the
    shared +0x598 group; the faction manager object was not pinned.

HARD RULES honored (same as intel_live.py / characters_live.py):
  * READ-ONLY memory; keep the game PAUSED; passive reads => any cadence, no desync.
  * Build-gated: refuses to run unless the live hoi4.exe sha256 matches the build.
  * No personal paths; self-contained; reuses winmem.py + locate.py + tag_table.py.
"""

import os
import sys

_HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, _HERE)
sys.path.insert(0, os.path.join(_HERE, "live_layers"))
import winmem  # noqa: E402
import locate  # noqa: E402
import tag_table  # noqa: E402

EXPECTED_SHA256 = "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120"
PROC_NAME = "hoi4.exe"

# ---- country identity ----
COUNTRY_VTABLE_RVA = 0x27C0E80   # all live country objects share this vtable at +0x00
OBJ_NAME = 0x10                  # localized name std::string within the country object
STRIDE = 8                       # bridge array: one country-object ptr per tag_db_id

# ---- country-object diplomacy fields ----
OBJ_FACTION_GROUP = 0x598        # i32 faction/diplomatic-group id (shared per faction)

# ---- war_relation struct (offsets from the header = the tag pair) ----
WR_FIRST_TAGID = 0x00            # i32
WR_SECOND_TAGID = 0x04           # i32
WR_TYPEPTR = 0x08               # u64 == one of the accepted date vtables below.
                                #        Live 1.19.1 war rows can carry either
                                #        CGameDate or CGregorianDate here; both are
                                #        interleaved in the same war-relation pool.
WR_TYPEPTR_RVA = 0x26F2398      # CGameDate vtable
WR_GREGORIAN_DATE_TYPEPTR_RVA = 0x26F21A0
WR_TYPEPTR_RVAS = (WR_TYPEPTR_RVA, WR_GREGORIAN_DATE_TYPEPTR_RVA)
WR_FIRST_CAS = 0x40             # i32
WR_SECOND_CAS = 0x48            # i32
WR_ECHO = 0x68                  # i32 first tag_db_id again (+0x6C second) -- secondary sig
WR_MAX_CAS = 50_000_000         # sanity bound on a casualty field

PAGE = 0x1000
HEAP_LO = 0x10000000000
HEAP_HI = 0x7FFFFFFFFFFF
_HEAP_HI_SCAN = 0x7FF000000000   # scan upper bound used by the sibling locators

# anchor TAG -> localized name (same set the country_index_bridge uses)
_ANCHOR_NAMES = {
    "German Reich": "GER", "United Kingdom": "ENG", "Soviet Union": "SOV",
    "Sweden": "SWE", "Italy": "ITA", "Japanese Empire": "JAP",
    "United States": "USA", "British Raj": "RAJ", "Poland": "POL",
    "Hungary": "HUN", "Greater Hungary": "HUN", "Norway": "NOR",
}
_MIN_ANCHORS = 6


# ==========================================================================
# Country identity: vtable scan + tag-db-id bridge (save-free)
# ==========================================================================
def _vt_at(p, addr):
    """Read the vtable qword at a heap object base, or None (unreadable)."""
    try:
        return p.u64(addr)
    except OSError:
        return None


def _live_country_object(p, obj):
    """Accept a vtable hit only if its name std::string resolves AND is stable across
    a re-read (TOCTOU / freed-object guard, same as intel_live.py)."""
    nm = locate.read_msvc_str(p, obj + OBJ_NAME, maxlen=64)
    if not nm:
        return None
    if locate.read_msvc_str(p, obj + OBJ_NAME, maxlen=64) != nm:
        return None
    return nm


def _scan_vtable_objects(p):
    """{country_object_addr: localized_name} for every live country object (vtable at
    +0x00 == module_base + COUNTRY_VTABLE_RVA, name resolves + stable)."""
    vt = p.module_base + COUNTRY_VTABLE_RVA
    needle = vt.to_bytes(8, "little")
    seen = {}
    CH = 64 << 20
    OV = 16
    for base, size in p.iter_regions(HEAP_LO, _HEAP_HI_SCAN, want_exec=False):
        pos = 0
        while pos < size:
            want = min(CH, size - pos)
            blob = p.try_read(base + pos, want)
            if blob:
                s = 0
                while True:
                    i = blob.find(needle, s)
                    if i < 0:
                        break
                    s = i + 1
                    a = base + pos + i
                    if a % 8 == 0 and a not in seen:
                        nm = _live_country_object(p, a)
                        if nm:
                            seen[a] = nm
                blen = len(blob)
                if pos + blen >= size or blen < want:
                    pos += max(blen, PAGE)            # final / short read
                else:
                    pos += blen - OV                  # full mid-region chunk
            else:
                pos += max(want, PAGE)
    return seen


def _find_bridge_base(p, anchors):
    """Fingerprint the tag-db-id-indexed bridge array (slot[t]=country_object_ptr) by
    requiring EVERY anchor's object at base + tag_db_id*8. Returns the lowest base (the
    engine keeps mirror copies)."""
    first_tid, first_obj, _ = anchors[0]
    needle = first_obj.to_bytes(8, "little")
    cands = set()
    CH = 64 << 20
    OV = 16
    for rbase, size in p.iter_regions(HEAP_LO, _HEAP_HI_SCAN, want_exec=False):
        pos = 0
        while pos < size:
            want = min(CH, size - pos)
            blob = p.try_read(rbase + pos, want)
            if blob:
                s = 0
                while True:
                    i = blob.find(needle, s)
                    if i < 0:
                        break
                    s = i + 1
                    addr = rbase + pos + i
                    if addr % 8 != 0:
                        continue
                    cand = addr - first_tid * STRIDE
                    if cand <= 0:
                        continue
                    ok = True
                    for tid, eobj, _t in anchors:
                        try:
                            if p.u64(cand + tid * STRIDE) != eobj:
                                ok = False
                                break
                        except OSError:
                            ok = False
                            break
                    if ok:
                        cands.add(cand)
                blen = len(blob)
                if pos + blen >= size or blen < want:
                    pos += max(blen, PAGE)
                else:
                    pos += blen - OV
            else:
                pos += max(want, PAGE)
    return min(cands) if cands else None


class CountryIdentity(object):
    """Save-free {tag_db_id <-> TAG <-> country_object} mapping for the live frame.

    The tag_db_id<->TAG maps are STABLE for the session (a TAG keeps its tag-db-id). The
    country_object POINTERS, however, can in principle be reallocated by the engine; so when a
    ``bridge`` (CountryIndexBridge) is attached, ``object_of`` resolves the pointer THROUGH the
    bridge LIVE on every call (``bridge.country_object(tag_db_id)``) rather than serving the
    pointer cached at resolve time. Only the stable maps are cached; pointers are always live
    (Codex-vetted: eliminates the cached-pointer staleness risk for a shared/reused identity)."""

    def __init__(self, tbl, by_tagid, bridge=None):
        self.tbl = tbl
        self._by_tagid = by_tagid                    # {tag_db_id: (TAG, obj)}  (obj = resolve-time)
        self._tag_of_id = {t: v[0] for t, v in by_tagid.items()}
        self._id_of_tag = {v[0]: t for t, v in by_tagid.items()}
        self._obj_of_tag = {v[0]: v[1] for v in by_tagid.values()}
        self.bridge = bridge                         # optional: live pointer resolution

    @classmethod
    def resolve(cls, p, bridge=None):
        tbl = tag_table.TagTable.locate(p)
        if tbl is None:
            return None
        objs = _scan_vtable_objects(p)
        if not objs:
            return None
        name_to_obj = {}
        for a, nm in objs.items():
            name_to_obj.setdefault(nm, a)
        tagmap = tbl.as_map()
        tag_to_id = {v: k for k, v in tagmap.items() if v and v != tag_table.SENTINEL}
        anchors = {}
        for nm, tag in _ANCHOR_NAMES.items():
            if nm in name_to_obj and tag in tag_to_id:
                anchors[tag_to_id[tag]] = (tag_to_id[tag], name_to_obj[nm], tag)
        anchors = sorted(anchors.values())
        if len(anchors) < _MIN_ANCHORS:
            return None
        base = _find_bridge_base(p, anchors)
        if base is None:
            return None
        by_tagid = {}
        for tid in range(1, tbl.count + 4):
            try:
                v = p.u64(base + tid * STRIDE)
            except OSError:
                continue
            if v in objs:
                by_tagid[tid] = (tbl.tag(tid), v)
        if len(by_tagid) < 40:
            return None
        return cls(tbl, by_tagid, bridge=bridge)

    def tag(self, tag_db_id):
        return self._tag_of_id.get(tag_db_id)

    def tag_db_id(self, tag):
        return self._id_of_tag.get(tag)

    def object_of(self, tag):
        """tag -> live country_object pointer. When a bridge is attached, resolve LIVE
        through it (tracks any reallocation); else return the resolve-time pointer. The
        bridge only ever AGREES with or refines the cached pointer for the SAME object;
        a None from the bridge (slot now empty) falls back to the cached pointer so a
        transient bridge miss cannot silently drop a country (the caller's heap-range
        and vtable gates remain the safety net)."""
        if self.bridge is not None:
            tid = self._id_of_tag.get(tag)
            if tid is not None:
                try:
                    obj = self.bridge.country_object(tid)
                except Exception:  # noqa: BLE001 -- bridge read is best-effort; fall back
                    obj = None
                if obj:
                    return obj
        return self._obj_of_tag.get(tag)

    def valid_ids(self):
        return set(self._by_tagid.keys())

    def tags(self):
        return list(self._id_of_tag.keys())

    def population_token(self):
        """A stable per-session fingerprint of the resolved identity SET: the exact frozen
        set of (tag_db_id, TAG) pairs. Stronger than a count (a count cannot detect a TAG
        swap / released country / partial re-resolution). Used by the pin's validate gate."""
        return frozenset((t, v[0]) for t, v in self._by_tagid.items())

    def revalidate(self, p):
        """Cheap fail-closed liveness gate for REUSE (TOCTOU / mid-free guard, mirrors
        CountryIndexBridge.revalidate). Re-reads a spread of anchor country objects and
        confirms each still carries the expected vtable AND a stable localized name. Returns
        True iff every checked anchor still resolves; any failure -> the caller re-resolves
        (never serves a moved/freed identity)."""
        ids = sorted(self._by_tagid.keys())
        if not ids:
            return False
        vt_expected = p.module_base + COUNTRY_VTABLE_RVA
        # spread checks across the id range (first, last, evenly between) -- a single anchor
        # would miss a partial reallocation of the object pool.
        n = min(5, len(ids))
        idxs = sorted({int(round(i * (len(ids) - 1) / max(1, n - 1))) for i in range(n)})
        for j in idxs:
            tid = ids[j]
            tag = self._tag_of_id.get(tid)
            obj = self.object_of(tag)
            if not obj:
                return False
            if _vt_at(p, obj) != vt_expected:        # moved/freed/different class
                return False
            if _live_country_object(p, obj) is None:  # name no longer resolves stably
                return False
        return True


# ==========================================================================
# War relations + casualties (PINNED)
# ==========================================================================
def read_war_relations(p, ident, want_addrs=False):
    """Enumerate the global war_relation pool. Returns (relations, other_types) -- or, when
    ``want_addrs`` is True, (relations, other_types, addr_by_pair) where addr_by_pair maps the
    (first_tagid, second_tagid) ordered pair to the record's BASE heap address (the PIN root for
    the warm fast-path -- see read_war_relations_from_addrs).
      relations   : list of {first, second, first_casualties, second_casualties} (TAGs)
      other_types : {foreign_type_ptr: count} -- war_relation-SHAPED records (valid tag
                    pair + echo) whose +0x08 type pointer is an IN-MODULE pointer other
                    than the known one (a possible subtype we'd miss). Normally empty;
                    a nonempty dict means the reader may be under-reporting -> the caller
                    should fail closed (Codex completeness guard).

    Signature: header [u32 +0x00]=first_tagid, [u32 +0x04]=second_tagid (both valid,
    distinct); the +0x68/+0x6C echo equals the same pair; and the PRIMARY signature
    [u64 +0x08] is one of the accepted live date vtables observed in the war-relation
    pool (CGameDate or CGregorianDate). Casualties at +0x40 / +0x48."""
    addr_by_pair = {}
    try:
        import numpy as np
        have_np = True
    except ImportError:
        have_np = False
    valid = ident.valid_ids()
    maxid = max(valid)
    isvalid = bytearray(maxid + 2)
    for i in valid:
        isvalid[i] = 1
    typeptrs = _war_relation_typeptrs(p)
    out = []
    seen = set()
    # Completeness guard: a record that LOOKS like a war_relation (valid tag pair +
    # the +0x68 echo + plausible casualties) but whose +0x08 date-vtable is a
    # DIFFERENT RTTI-backed module pointer could be a missed variant. Non-RTTI
    # in-module integers are common decoys and are ignored here.
    other_types = {}                 # foreign type ptr -> hit count
    rtti_warning_cache = {}
    modlo = p.module_base
    modhi = p.module_base + p.module_size
    CH = 16 << 20
    for rbase, size in p.iter_regions(HEAP_LO, _HEAP_HI_SCAN, want_exec=False):
        pos = 0
        while pos < size:
            want = min(CH, size - pos)
            blob = p.try_read(rbase + pos, want)
            if blob:
                n = len(blob)
                cand = _candidate_headers(blob, n, maxid, have_np)
                for k in cand:
                    bo = k * 4
                    if bo + WR_ECHO + 8 > n or bo + WR_SECOND_CAS + 4 > n:
                        continue
                    ta = _u32(blob, bo)
                    tb = _u32(blob, bo + 4)
                    if ta == tb or ta < 1 or tb < 1 or ta > maxid or tb > maxid:
                        continue
                    if not (isvalid[ta] and isvalid[tb]):
                        continue
                    # secondary echo first (cheap) so the completeness count only sees
                    # genuinely war_relation-SHAPED records, not random tag adjacencies.
                    if _u32(blob, bo + WR_ECHO) != ta or _u32(blob, bo + WR_ECHO + 4) != tb:
                        continue
                    tp = _u64(blob, bo + WR_TYPEPTR) if bo + WR_TYPEPTR + 8 <= n else 0
                    # PRIMARY signature: accepted date vtable at +0x08.
                    if tp not in typeptrs:
                        # Only flag a FOREIGN in-module type as a possible missed subtype
                        # when the record ALSO carries plausible (nonzero) casualties at
                        # the war_relation casualty offsets -- otherwise it's an unrelated
                        # struct (e.g. the faction-group wrapper) that merely shares the
                        # [tagid][tagid]+echo shape, and flagging it is a false alarm.
                        if modlo <= tp < modhi and bo + WR_SECOND_CAS + 4 <= n:
                            ffc = _i32(blob, bo + WR_FIRST_CAS)
                            fsc = _i32(blob, bo + WR_SECOND_CAS)
                            if ((0 < ffc <= WR_MAX_CAS or 0 < fsc <= WR_MAX_CAS)
                                    and _foreign_war_relation_type_maybe_relevant(
                                        p, tp, typeptrs, rtti_warning_cache)):
                                other_types.setdefault(tp, set()).add((ta, tb))
                        continue
                    fc = _i32(blob, bo + WR_FIRST_CAS)
                    sc = _i32(blob, bo + WR_SECOND_CAS)
                    if fc < 0 or sc < 0 or fc > WR_MAX_CAS or sc > WR_MAX_CAS:
                        continue
                    key = (ta, tb)
                    if key in seen:
                        continue
                    seen.add(key)
                    if want_addrs:
                        addr_by_pair[key] = rbase + pos + bo   # record base = PIN root
                    out.append({
                        "first": ident.tag(ta), "second": ident.tag(tb),
                        "first_casualties": fc, "second_casualties": sc,
                    })
                blen = len(blob)
                if pos + blen >= size or blen < want:
                    pos += max(blen, PAGE)
                else:
                    pos += blen - 0x80          # overlap covers the full struct span
            else:
                pos += max(want, PAGE)
    # A foreign type only matters if it carries a war pair we did NOT already capture
    # via the real type pointer (i.e. a genuinely missed war). Pairs already covered are
    # benign look-alikes.
    missed = {}
    for tp, pairs in other_types.items():
        new_pairs = [(ta, tb) for (ta, tb) in pairs if (ta, tb) not in seen]
        if new_pairs:
            missed[tp] = len(new_pairs)
    if want_addrs:
        return out, missed, addr_by_pair
    return out, missed


def validate_war_relation_addrs(p, ident, addr_by_pair):
    """Fail-closed liveness gate for the PINNED war_relation record addresses. For EVERY pinned
    record re-confirm, at its base addr, the war_relation signature is intact: [u64 +0x08] is an
    accepted date vtable (the primary signature) AND [u32 +0x00]/[u32 +0x04] ==
    the pinned (first_tagid, second_tagid) AND the +0x68/+0x6C echo equals the same pair AND BOTH
    tag-db-ids are STILL in the live identity's valid set (Codex change-1 #4: identity drift must
    also force a re-sweep) AND the casualty fields are still within the cold sweep's 0..WR_MAX_CAS
    bound (Codex change-1 #1: the cold sweep DROPS out-of-bound records, so the warm path must not
    serve a record whose casualties drifted out of bound). The (first_tagid, second_tagid) pair IS
    the stable id (a war between A and B is unique within the frozen capture epoch). Any mismatch /
    unreadable -> False -> the caller re-sweeps the full pool (never serves a moved/freed/ended/
    out-of-bound war as live). Returns True iff every pinned record still validates."""
    if not addr_by_pair:
        return False
    typeptrs = _war_relation_typeptrs(p)
    valid_ids = None
    try:
        valid_ids = ident.valid_ids()
    except Exception:
        valid_ids = None
    for (ta, tb), addr in addr_by_pair.items():
        if valid_ids is not None and (ta not in valid_ids or tb not in valid_ids):
            return False                              # identity drift -> re-sweep (fail closed)
        try:
            if p.u64(addr + WR_TYPEPTR) not in typeptrs:
                return False
            if p.u32(addr + WR_FIRST_TAGID) != ta or p.u32(addr + WR_SECOND_TAGID) != tb:
                return False
            if p.u32(addr + WR_ECHO) != ta or p.u32(addr + WR_ECHO + 4) != tb:
                return False
            fc = p.i32(addr + WR_FIRST_CAS)
            sc = p.i32(addr + WR_SECOND_CAS)
            if fc < 0 or sc < 0 or fc > WR_MAX_CAS or sc > WR_MAX_CAS:
                return False                          # casualties drifted out of the cold bound
        except OSError:
            return False
    return True


def read_war_relations_from_addrs(p, ident, addr_by_pair):
    """Warm-path re-read of the war_relation list from the PINNED record addresses -- NO heap
    sweep. Re-reads each record's casualties (i32 +0x40/+0x48) LIVE and re-resolves the tag names
    through ``ident`` (so a name shift is tracked). The caller MUST have just passed
    validate_war_relation_addrs (the signature/type/pair/identity/casualty-bound gate); this
    function reads only the records' mutable casualty fields + names and re-applies the SAME
    casualty bound the cold sweep enforces (0..WR_MAX_CAS) so the warm list is byte-identical to a
    cold sweep of the SAME frozen pool. Returns the SAME list shape as read_war_relations (the
    caller re-derives at_war/casualties). A per-record read fault / out-of-bound casualty drops
    THAT record (a conservative under-report, never a fabricated value -- and validate would
    already have failed it, forcing a full re-sweep)."""
    out = []
    for (ta, tb), addr in addr_by_pair.items():
        try:
            fc = p.i32(addr + WR_FIRST_CAS)
            sc = p.i32(addr + WR_SECOND_CAS)
        except OSError:
            continue
        if fc < 0 or sc < 0 or fc > WR_MAX_CAS or sc > WR_MAX_CAS:
            continue                                  # mirror the cold sweep's casualty gate
        out.append({
            "first": ident.tag(ta), "second": ident.tag(tb),
            "first_casualties": fc, "second_casualties": sc,
        })
    return out


def _candidate_headers(blob, n, maxid, have_np):
    """Yield 4-aligned indices k where [u32 k] and [u32 k+1] are both <= maxid and
    nonzero (a fast prefilter for war_relation headers)."""
    if have_np:
        import numpy as np
        cnt = n // 4
        if cnt < 2:
            return []
        u = np.frombuffer(blob, dtype="<u4", count=cnt)
        a = u[:-1]
        b = u[1:]
        return np.nonzero((a >= 1) & (a <= maxid) & (b >= 1) & (b <= maxid) & (a != b))[0]
    # stdlib fallback (slower)
    import struct as _s
    res = []
    for k in range(0, (n // 4) - 1):
        a = _s.unpack_from("<I", blob, k * 4)[0]
        b = _s.unpack_from("<I", blob, k * 4 + 4)[0]
        if 1 <= a <= maxid and 1 <= b <= maxid and a != b:
            res.append(k)
    return res


def at_war_sets(war_relations):
    """{TAG: sorted[enemy TAGs]} from the war_relation list."""
    from collections import defaultdict
    enemies = defaultdict(set)
    for w in war_relations:
        a, b = w["first"], w["second"]
        if a and b:
            enemies[a].add(b)
            enemies[b].add(a)
    return {t: sorted(s) for t, s in enemies.items()}


def country_total_casualties(war_relations):
    """{TAG: cumulative war casualties} = sum of this country's side over all its
    war_relations (matches the melt's per-country casualty sum)."""
    from collections import defaultdict
    tot = defaultdict(int)
    for w in war_relations:
        if w["first"]:
            tot[w["first"]] += w["first_casualties"]
        if w["second"]:
            tot[w["second"]] += w["second_casualties"]
    return dict(tot)


# ==========================================================================
# Factions (membership by shared diplomatic-group id)
# ==========================================================================
def read_faction_groups(p, ident):
    """{TAG: faction_group_id} from country_object+0x598, then group. Returns
    (group_of_tag, members_by_group). A group with >1 member is a faction bloc; the
    2 real formal factions (Allies/Comintern) appear as the multi-member groups that
    match the save (informal war buckets may also share a value -- see draft)."""
    from collections import defaultdict
    group_of = {}
    for tag in ident.tags():
        obj = ident.object_of(tag)
        if not obj:
            continue
        try:
            group_of[tag] = p.i32(obj + OBJ_FACTION_GROUP)
        except OSError:
            continue
    members = defaultdict(list)
    for t, g in group_of.items():
        members[g].append(t)
    return group_of, {g: sorted(ts) for g, ts in members.items()}


# ==========================================================================
# small read helpers
# ==========================================================================
def _u32(blob, off):
    import struct as _s
    return _s.unpack_from("<I", blob, off)[0]


def _i32(blob, off):
    import struct as _s
    return _s.unpack_from("<i", blob, off)[0]


def _u64(blob, off):
    import struct as _s
    return _s.unpack_from("<Q", blob, off)[0]


def _war_relation_typeptrs(p):
    return {p.module_base + rva for rva in WR_TYPEPTR_RVAS}


def _rtti_type_name_for_vftable(p, vftable):
    """Best-effort MSVC RTTI type name for an in-module vftable pointer."""
    try:
        col = p.u64(vftable - 8)
        if col < p.module_base or col >= p.module_base + p.module_size:
            return None
        if p.u32(col) != 1:
            return None
        type_desc_rva = p.u32(col + 12)
        if type_desc_rva >= p.module_size:
            return None
        raw = p.try_read(p.module_base + type_desc_rva + 16, 160)
        if not raw:
            return None
        end = raw.find(b"\0")
        if end >= 0:
            raw = raw[:end]
        return raw.decode("utf-8", "replace")
    except Exception:
        return None


def _foreign_war_relation_type_maybe_relevant(p, tp, accepted_typeptrs, cache):
    if tp in accepted_typeptrs:
        return False
    if tp in cache:
        return cache[tp]
    name = _rtti_type_name_for_vftable(p, tp)
    cache[tp] = bool(name)
    return cache[tp]


# ==========================================================================
# Public API
# ==========================================================================
def read_all(p, ident=None, war_addrs=None, want_war_addrs=False):
    """Resolve identity + read all PINNED diplomacy. Returns a dict:
        {
          "war_relations": [{first, second, first_casualties, second_casualties}, ...],
          "at_war":        {TAG: [enemy TAGs]},
          "casualties":    {TAG: cumulative_casualties},
          "faction_group": {TAG: group_id},
          "faction_members": {group_id: [TAGs]},
          "war_addrs":     {(first_tagid, second_tagid): record_addr},   # only if want_war_addrs
        }
    Raises RuntimeError if country identity cannot be resolved (fail-closed).

    ``ident`` (optional): reuse an already-resolved CountryIdentity (the shared one the capture
    context resolved once at attach) to skip this reader's own vtable+bridge identity scan.
    ``None`` self-resolves (the caller MUST pass None when the shared identity failed/partially
    resolved or disagreed with the bridge, so a bad identity is never threaded -- fail-closed).

    ``war_addrs`` (optional, WARM fast-path): a pinned {(first_tagid,second_tagid): record_addr}
    map (from a prior want_war_addrs cold read). When supplied AND it still validates
    (validate_war_relation_addrs), the war_relations are re-read from those addresses WITHOUT the
    full heap sweep. A validate MISS -> fall through to the full sweep (fail-closed).

    COMPLETENESS (Codex change-1 #2): the war_addrs map is ONLY ever pinned by the cold path when
    its subtype-warning was EMPTY (a clean completeness proof for the frozen pool). So whenever the
    warm path runs, the cold sweep had already proved no foreign-type look-alike war existed, and
    the warm subtype_warning is honestly {} -- NOT a sentinel that could hide a real prior warning.
    If a cold sweep DID find a subtype warning, it returns out_addrs=None so the pin is NOT taken
    and EVERY walk re-sweeps and re-emits the real warning (never suppressed).

    ``want_war_addrs`` (optional): also return the located record addresses under "war_addrs" so a
    caller (the registry pin) can cache them for the warm path (ONLY when the warning is empty)."""
    if ident is None:
        ident = CountryIdentity.resolve(p)
    if ident is None:
        raise RuntimeError("could not resolve country identity (vtable scan + bridge)")
    if war_addrs and validate_war_relation_addrs(p, ident, war_addrs):
        wr = read_war_relations_from_addrs(p, ident, war_addrs)   # WARM: no sweep
        other_types = {}
        out_addrs = dict(war_addrs)                  # a clean pin (only ever pinned warning-free)
    elif want_war_addrs:
        wr, other_types, out_addrs = read_war_relations(p, ident, want_addrs=True)
        # Do NOT pin a pool whose cold sweep flagged a possible missed subtype war: re-sweeping
        # every walk is the only way to keep re-emitting that real warning (Codex change-1 #2).
        if other_types:
            out_addrs = None
    else:
        wr, other_types = read_war_relations(p, ident)
        out_addrs = None
    group_of, members = read_faction_groups(p, ident)
    result = {
        "war_relations": wr,
        "at_war": at_war_sets(wr),
        "casualties": country_total_casualties(wr),
        "faction_group": group_of,
        "faction_members": members,
        # completeness guard: nonempty => possible war_relation SUBTYPE missed. The warm path only
        # runs when the cold sweep was clean, so this is an honest measured-empty {} there.
        "war_relation_subtype_warning": {hex(t): c for t, c in other_types.items()},
    }
    if want_war_addrs or war_addrs:
        result["war_addrs"] = out_addrs
    return result


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
    """Compare the live reader against the melt ground truth. Prints a report and
    returns True iff wars/at-war/casualties/factions all match. Save = ANCHOR ONLY."""
    import json
    if gt_path is None:
        gt_path = os.path.join(_HERE, "..", "re_scratch", "diplo", "ground_truth.json")
    gt_path = os.path.abspath(gt_path)
    with open(gt_path, encoding="utf-8") as fh:
        gt = json.load(fh)

    data = read_all(p)
    ok_all = True

    # --- completeness guard (ADVISORY) ---
    # The at-war + casualty cross-validation below (30/30, 144/144) is the authoritative
    # completeness proof for a frame: if a real war_relation subtype existed, an at-war
    # pair or casualty total would be MISSING. The foreign-type counter only catches
    # unrelated structs that coincidentally share the [tagid][tagid]+echo+nonzero-i32
    # shape (e.g. the faction-group wrapper at vtable 0x7ff786b2b170), so it is advisory.
    w = data.get("war_relation_subtype_warning") or {}
    if w:
        print("  note: %d foreign-type look-alike record group(s) %s -- advisory only; "
              "the at-war/casualty cross-checks below are the completeness proof" % (len(w), w))
    else:
        print("  completeness guard: no foreign war_relation look-alikes")

    # --- casualty pairs ---
    casd = {}
    for k, v in gt["casualties"].items():
        a, b = k.split("|")
        casd[(a, b)] = (v[0] or 0, v[1] or 0)
    live_pairs = {(w["first"], w["second"]): (w["first_casualties"], w["second_casualties"])
                  for w in data["war_relations"] if w["first"] and w["second"]}
    pair_exact = 0
    for (a, b), (fc, sc) in casd.items():
        lv = live_pairs.get((a, b)) or (None if (b, a) not in live_pairs
                                        else live_pairs[(b, a)][::-1])
        if lv == (fc, sc):
            pair_exact += 1
    print("  casualty pairs EXACT: %d/%d" % (pair_exact, len(casd)))
    ok_all &= (pair_exact == len(casd))

    # --- at-war sets ---
    gt_atwar = {t: set(v) for t, v in gt["atwar"].items()}
    live_atwar = {t: set(v) for t, v in data["at_war"].items()}
    aw_ok = sum(1 for t, s in gt_atwar.items() if live_atwar.get(t, set()) == s)
    print("  at-war set EXACT countries: %d/%d" % (aw_ok, len(gt_atwar)))
    ok_all &= (aw_ok == len(gt_atwar))

    # --- per-country total casualties ---
    from collections import defaultdict
    gt_tot = defaultdict(int)
    for (a, b), (fc, sc) in casd.items():
        gt_tot[a] += fc
        gt_tot[b] += sc
    live_tot = data["casualties"]
    tc_ok = sum(1 for t, c in gt_tot.items() if live_tot.get(t, 0) == c)
    print("  per-country total casualties EXACT: %d/%d" % (tc_ok, len(gt_tot)))
    ok_all &= (tc_ok == len(gt_tot))

    # --- faction membership (the 2 real factions) ---
    fac_of = gt["faction_of"]               # TAG -> melt faction id (3=Comintern,2=Allies)
    group_of = data["faction_group"]
    # group melt members by faction id, check each shares one live group value, distinct
    from collections import defaultdict as dd
    melt_groups = dd(list)
    for t, f in fac_of.items():
        melt_groups[f].append(t)
    fac_ok = True
    members_checked = 0
    members_match = 0
    seen_live_vals = {}
    for f, tags in melt_groups.items():
        live_vals = set(group_of.get(t) for t in tags if t in group_of)
        members_checked += sum(1 for t in tags if t in group_of)
        if len(live_vals) == 1 and None not in live_vals:
            v = next(iter(live_vals))
            if v in seen_live_vals:           # two melt factions sharing one live id -> bad
                fac_ok = False
            seen_live_vals[v] = f
            members_match += sum(1 for t in tags if t in group_of)
        else:
            fac_ok = False
    print("  faction membership: %d/%d members in a single shared group; distinct-per-faction: %s"
          % (members_match, members_checked, fac_ok))
    ok_all &= fac_ok and (members_match == members_checked)

    print("  OVERALL live==save: %s" % ("PASS" if ok_all else "FAIL"))
    return ok_all


def main():
    import argparse
    ap = argparse.ArgumentParser(description="Live the game diplomacy reader (read-only).")
    ap.add_argument("--validate", action="store_true", help="compare live vs the anchor melt")
    args = ap.parse_args()
    p = winmem.open_process(PROC_NAME)
    if p is None:
        raise SystemExit("hoi4.exe not running")
    with p:
        _build_gate(p)
        if args.validate:
            print("build gate OK; validating diplomacy live==save (READ-ONLY)...")
            _validate(p)
            return
        print("build gate OK; reading diplomacy (READ-ONLY)...")
        data = read_all(p)
        wr = data["war_relations"]
        print("war_relations: %d" % len(wr))
        print("countries at war: %d" % len(data["at_war"]))
        if data.get("war_relation_subtype_warning"):
            print("WARNING (completeness): foreign war_relation type ptrs: %s"
                  % data["war_relation_subtype_warning"])
        print("\n%-4s %-8s %s" % ("TAG", "casualt.", "enemies"))
        for tag in sorted(data["at_war"]):
            print("%-4s %-8d %s" % (tag, data["casualties"].get(tag, 0),
                                    ",".join(data["at_war"][tag])))
        print("\nfaction groups (group_id -> members):")
        for g, ts in sorted(data["faction_members"].items()):
            if len(ts) > 1:
                print("  group %d (%d): %s" % (g, len(ts), ",".join(ts)))


if __name__ == "__main__":
    main()
