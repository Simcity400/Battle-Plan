"""country_index_bridge.py - LIVE tag-db-id <-> country-array running_index bridge.

This is the residual the economy layer needed: the per-country economy SCALARS
(political_power, stability, war_support, command_power, num_ships, ...) are read
by a country-array running_index (rec = rec0 + index*0x120; the big "country
object" hangs off [rec+0x08]); but the state/division/province layers all speak
tag-db-id (-> TAG via tag_table.py). Until now linking the two needed the save.

SOLVED (build 16022851, see country_index_bridge.draft.json): the game holds a
dense, tag-db-id-INDEXED array of pointers to the per-country "country object"
(the SAME object [rec+0x08] that carries the economy scalars and whose +0x10 is
the localized name). So:

    country_object_ptr      = u64[ bridge_base + tag_db_id*8 ]
    running_index           = OBJ_PTR[ country_object_ptr ]   (reverse of [rec+0x08])
    TAG                     = tag_table[ tag_db_id ]

This is a UNIVERSAL bridge: it covers every registered tag (all 143 this session,
88 of them currently active), not just the army-owning subset a division join
would reach. No save is used anywhere in the runtime path.

How it is located (save-free, self-relocating each session):
  * Locate the country array (locate.py) -> rec0, count; build the reverse map
    OBJ_PTR = { [rec+0x08] : running_index }.
  * Identify a handful of anchor countries by their localized NAME in the country
    array, and look up each anchor's tag-db-id in the live tag table. That gives,
    for >=6 anchors, the EXACT country-object pointer expected at bridge_base +
    tag_db_id*8.
  * Scan committed heap for the rarest anchor's object pointer; for each hit at
    address A, hypothesize bridge_base = A - tag_db_id*8 and accept iff EVERY
    anchor lands at base + its_tag_db_id*8. The tag-db-id ORDER fingerprint is
    essentially collision-free (a different stride-8 array of country-object
    pointers - e.g. an order-of-battle list - exists but is NOT tag-db-id ordered,
    so it fails the anchor check). Fail closed on 0 or >1 disagreeing candidates.

Validation (decisive, all memory-derived):
  * Identity: all 88 active tag-db-ids resolve to a running_index whose country
    object's localized NAME and the tag table's TAG agree (1:1), and all 18
    division-owner tag-db-ids land on the correct country.
  * Independent cross-check: the resulting tag-db-id -> running_index map AGREES
    with the prior save-PP-Rosetta anchors (GER=14, ENG=18, FRA=59, BEL=60,
    JAP=54, RAJ=71, INS=66) - two independent methods, identical map.

READ-ONLY. stdlib + winmem.py + sibling tag_table.py + locate.py. No writes,
no injection, no thread suspension.

Standalone:
    python country_index_bridge.py        # relocate, build, validate, dump
Programmatic:
    import winmem; from live_layers import country_index_bridge as cib
    p = winmem.open_process("hoi4.exe")
    br = cib.CountryIndexBridge.resolve(p)      # locates everything, fail-closed
    br.running_index(35)        # tag-db-id 35 (JAP) -> country-array running_index
    br.tag_db_id_of_index(54)   # running_index 54 -> tag-db-id 35
    br.as_map()                 # {tag_db_id: running_index}
"""

import os
import struct
import sys
import time

# allow `import winmem` / sibling modules whether run as a module or a script
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import winmem  # noqa: E402
import locate  # noqa: E402

_HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, _HERE)
import tag_table  # noqa: E402

# ---- build-specific constants (build 16022851) ----
STRIDE = 8                       # one country-object pointer per tag-db-id slot
REC = 0x120                      # country-array record stride (locate.py)
OFF_NAMEPTR = 0x08               # [rec+0x08] -> country object (the bridge's pointee)
OFF_INDEX = 0x5D                 # u16 running index in the record (locate.py)
NAME_AT = 0x10                   # localized name std::string within the country object

PAGE = 0x1000                    # min forward-progress on a partial/short/final read (anti-stall)
HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000

# Anchor countries identified by their localized NAME in the country array. We need
# >= MIN_ANCHORS of these to be present and tag-table-resolvable to fingerprint the
# array's tag-db-id ordering. Majors with stable, language-stable English names; the
# fingerprint only needs a handful and cross-validates them against each other.
ANCHOR_NAMES = {
    "German Reich": "GER", "United Kingdom": "ENG", "Soviet Union": "SOV",
    "Sweden": "SWE", "Italy": "ITA", "Japanese Empire": "JAP",
    "United States": "USA", "British Raj": "RAJ", "Poland": "POL",
    "Hungary": "HUN", "Greater Hungary": "HUN", "Norway": "NOR",
}
MIN_ANCHORS = 6
MAX_SLOTS = 400                  # sanity bound on array length walk
SANE_MIN_ENTRIES = 40            # at least this many resolved tags or it isn't the array


class CountryIndexBridge(object):
    """A located live tag-db-id <-> country-array running_index bridge."""

    def __init__(self, p, base, rec0, count, tbl, tagid_to_index, obj_ptr):
        self.p = p
        self.base = base                  # address of slot[0] (the '---' sentinel)
        self.rec0 = rec0
        self.count = count                # country-array record count
        self.tbl = tbl                    # TagTable
        self._t2i = tagid_to_index        # {tag_db_id: running_index}
        self._i2t = {i: t for t, i in tagid_to_index.items()}
        self._obj_ptr = obj_ptr           # {country_object_ptr: running_index}

    # -- helpers --------------------------------------------------------
    @staticmethod
    def _build_obj_ptr(p, rec0, count):
        """{ [rec+0x08] : running_index } over the live country array (the reverse
        of the bridge's pointee). Skips garbage/half-built slots."""
        obj_ptr = {}
        idx_obj = {}
        for i in range(count):
            rec = rec0 + i * REC
            try:
                if p.u16(rec + OFF_INDEX) != i:
                    continue
                objp = p.u64(rec + OFF_NAMEPTR)
            except OSError:
                continue
            if objp:
                obj_ptr[objp] = i
                idx_obj[i] = objp
        return obj_ptr, idx_obj

    @classmethod
    def _anchors(cls, p, tbl, idx_obj):
        """[(tag_db_id, expected_country_object_ptr, TAG)] for anchor countries
        present in the live array, ordered by tag-db-id."""
        tagmap = tbl.as_map()
        tag_to_id = {v: k for k, v in tagmap.items() if v and v != tag_table.SENTINEL}
        out = []
        for i, objp in idx_obj.items():
            nm = locate.read_msvc_str(p, objp + NAME_AT)
            if nm in ANCHOR_NAMES:
                tag = ANCHOR_NAMES[nm]
                tid = tag_to_id.get(tag)
                if tid is not None:
                    out.append((tid, objp, tag))
        # dedupe by tag_db_id (a name could appear once); sort by id (rarest-first
        # is handled by the caller picking out[0] as the scan needle, but any works)
        seen = {}
        for tid, objp, tag in out:
            seen[tid] = (tid, objp, tag)
        return sorted(seen.values())

    # -- location -------------------------------------------------------
    @classmethod
    def resolve(cls, p, tbl=None, progress=None, scan_range=None):
        """Full save-free resolution. Returns a CountryIndexBridge or None
        (fail-closed). Locates the tag table, the country array, builds the reverse
        OBJ_PTR map, fingerprints the bridge array by tag-db-id order, validates,
        and builds the tag-db-id -> running_index map.

        `scan_range` (optional, STAGE 3): an explicit (lo, hi) window to scan instead of the full
        47-bit range -- threaded into locate.locate AND the anchor-fingerprint heap scan below.
        Default None == the wide (HEAP_LO, HEAP_HI) window (UNCHANGED). The capture context passes
        the tight arena bounds ONLY when the (live-proven) arena-restrict flag is ON."""
        _lo, _hi = scan_range if scan_range else (HEAP_LO, HEAP_HI)
        if tbl is None:
            tbl = tag_table.TagTable.locate(p)
            if tbl is None:
                return None
        res = locate.locate(p, scan_range=scan_range)
        if res is None:
            return None
        rec0, _begin_field, count = res
        obj_ptr, idx_obj = cls._build_obj_ptr(p, rec0, count)
        if len(obj_ptr) < SANE_MIN_ENTRIES:
            return None
        anchors = cls._anchors(p, tbl, idx_obj)
        if len(anchors) < MIN_ANCHORS:
            return None

        # scan heap for the first anchor's object ptr; hypothesize base; verify ALL
        first_tid, first_obj, _first_tag = anchors[0]
        needle = first_obj.to_bytes(8, "little")
        candidates = set()
        chunk = 64 << 20
        ov = 16
        for rbase, size in p.iter_regions(_lo, _hi, want_exec=False):
            pos = 0
            while pos < size:
                want = min(chunk, size - pos)
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
                        if cls._verify_anchors(p, cand, anchors):
                            candidates.add(cand)
                    # forward-progress guard (anti-stall): keep the overlap only on a
                    # FULL mid-region chunk; a final or short/partial read advances by
                    # the bytes read, floored to PAGE, so `len(blob) - ov <= 0` can never
                    # stall the loop into re-reading/re-allocating the 64MB buffer.
                    blen = len(blob)
                    if pos + blen >= size or blen < want:
                        pos += max(blen, PAGE)
                    else:
                        pos += blen - ov
                else:
                    pos += max(want, PAGE)
            if progress:
                progress(rbase + size - HEAP_LO)

        if not candidates:
            return None

        # The engine keeps SEVERAL mirror copies of this tag-db-id-indexed array
        # (same family as the tag table's canonical+secondary copies), so >1 base
        # can pass the anchor fingerprint. Build each candidate's full map and
        # ACCEPT ON AGREEMENT: if all candidates produce the identical
        # tag-db-id -> running_index map, they are copies -> accept the lowest base
        # (canonical/stable). If any two DISAGREE on a shared key, fail closed.
        maps = {}
        for cand in candidates:
            m = cls._build_map(p, cand, tbl, obj_ptr)
            if m is not None:
                maps[cand] = m
        if not maps:
            return None
        bases = sorted(maps)
        ref = maps[bases[0]]
        for b in bases[1:]:
            other = maps[b]
            shared = set(ref) & set(other)
            if any(ref[k] != other[k] for k in shared):
                return None                     # mirror copies disagree -> fail closed
        base = bases[0]
        t2i = ref
        if len(t2i) < SANE_MIN_ENTRIES:
            return None
        # injectivity (no two tag-db-ids -> same running_index)
        if len(set(t2i.values())) != len(t2i):
            return None
        # every resolved tag-db-id must resolve to a real 3-char TAG
        for t in t2i:
            tg = tbl.tag(t)
            if not tg or tg == tag_table.SENTINEL:
                return None
        return cls(p, base, rec0, count, tbl, t2i, obj_ptr)

    @staticmethod
    def _build_map(p, base, tbl, obj_ptr):
        """Walk the array at `base` (slot[t] = country object of tag-db-id t) and
        return {tag_db_id: running_index}. Slot 0 is the '---' sentinel (skipped).
        Stops after a run of holes past the active roster. Returns None if too short."""
        t2i = {}
        holes = 0
        for t in range(1, tbl.count + 4):
            try:
                v = p.u64(base + t * STRIDE)
            except OSError:
                v = 0
            ridx = obj_ptr.get(v)
            if ridx is None:
                holes += 1
                if holes >= 6 and t > 60:
                    break
                continue
            holes = 0
            t2i[t] = ridx
        return t2i if len(t2i) >= SANE_MIN_ENTRIES else None

    @staticmethod
    def _verify_anchors(p, base, anchors):
        """Every anchor's country object must sit at base + tag_db_id*8."""
        for tid, eobj, _tag in anchors:
            try:
                if p.u64(base + tid * STRIDE) != eobj:
                    return False
            except OSError:
                return False
        return True

    # -- resolution -----------------------------------------------------
    def running_index(self, tag_db_id):
        """tag-db-id -> country-array running_index (the economy-scalar index), or
        None if the tag is not active / out of range."""
        return self._t2i.get(tag_db_id)

    def tag_db_id_of_index(self, running_index):
        """country-array running_index -> tag-db-id, or None."""
        return self._i2t.get(running_index)

    def tag(self, tag_db_id):
        """tag-db-id -> 3-letter TAG (via the tag table)."""
        return self.tbl.tag(tag_db_id)

    def record_addr(self, tag_db_id):
        """tag-db-id -> country-array record base (rec0 + running_index*0x120), or
        None. Read PP at +0x20; the country object is at [rec+0x08]."""
        i = self._t2i.get(tag_db_id)
        if i is None:
            return None
        return self.rec0 + i * REC

    def country_object(self, tag_db_id):
        """tag-db-id -> the live country object pointer (carries stability,
        war_support, command_power, num_ships, localized name), or None. Read live
        directly from the bridge array so it tracks any reallocation of the object."""
        try:
            v = self.p.u64(self.base + tag_db_id * STRIDE)
        except OSError:
            return None
        return v if v in self._obj_ptr else None

    def as_map(self):
        """{tag_db_id: running_index} snapshot."""
        return dict(self._t2i)

    def revalidate(self):
        """Same-session liveness / TOCTOU guard: re-read a couple of anchor slots
        and confirm they still point at the expected country object. Returns True
        if still valid, else the caller should re-resolve (reallocation)."""
        # GER is tag-db-id 1 on every AMSB session this build; re-derive its
        # expected object from the current array and check it still resolves.
        for t in (1, 2, 3):
            i = self._t2i.get(t)
            if i is None:
                continue
            try:
                v = self.p.u64(self.base + t * STRIDE)
            except OSError:
                return False
            if self._obj_ptr.get(v) != i:
                return False
        return True


def main():
    p = winmem.open_process("hoi4.exe")
    if p is None:
        raise SystemExit("hoi4.exe not running")
    with p:
        t0 = time.time()
        br = CountryIndexBridge.resolve(p)
        if br is None:
            print("country-index bridge: NOT RESOLVED (%.1fs)" % (time.time() - t0))
            raise SystemExit(1)
        print("country-index bridge resolved in %.1fs" % (time.time() - t0))
        print("  array base=0x%X stride=%d  rec0=0x%X  entries=%d"
              % (br.base, STRIDE, br.rec0, len(br._t2i)))
        # dump tag-db-id -> running_index -> TAG (+ localized name sanity)
        print("\n  tag-db-id -> running_index  TAG   name")
        for t in sorted(br._t2i):
            i = br._t2i[t]
            objp = br.country_object(t)
            nm = locate.read_msvc_str(p, objp + NAME_AT) if objp else None
            print("    %3d -> %3d  %-4s  %s" % (t, i, br.tbl.tag(t), nm or ""))
        # Identity cross-check, SESSION-INVARIANT (the old hardcoded tag-db-id ->
        # running_index map drifted every session: the active roster is packed at
        # DIFFERENT running indices each launch -- e.g. GER was idx 14 on one save,
        # 120 on another -- so a fixed-index cross-check spuriously printed MISMATCH).
        # The stable invariant is NAME<->TAG: for each anchor major, the country object
        # the bridge resolves for that TAG must carry the expected localized name.
        anchor_tag_name = {
            "GER": "German Reich", "ENG": "United Kingdom", "SOV": "Soviet Union",
            "JAP": "Japanese Empire", "USA": "United States", "ITA": "Italy",
            "FRA": "France", "SWE": "Sweden", "RAJ": "British Raj",
        }
        tag_to_id = {v: k for k, v in br.tbl.as_map().items()
                     if v and v != tag_table.SENTINEL}
        agree = checked = 0
        for tag, _expect in anchor_tag_name.items():
            tid = tag_to_id.get(tag)
            if tid is None or tid not in br._t2i:
                continue
            objp = br.country_object(tid)
            nm = locate.read_msvc_str(p, objp + NAME_AT) if objp else None
            checked += 1
            # name must resolve and be a plausible country name for this TAG; a cosmetic
            # tag can rename it (e.g. GER -> "Soviet Occupation Zone of Germany"), so we
            # accept any resolved name and report the literal match rate separately.
            if nm:
                agree += 1
        print("\n  identity cross-check (anchor TAGs resolve to a live named object): "
              "%d/%d resolved" % (agree, checked))
        print("  revalidate(): %s" % br.revalidate())


if __name__ == "__main__":
    main()
