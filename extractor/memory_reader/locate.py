"""locate.py - self-locating finder for the game country array.

No static pointer / no Cheat Engine anchor: the country DB is buried deep in a
large game-state object (a reverse pointer-scan loops inside the DB cluster and
no static pointer lands near it), so we relocate the array each launch by its
one-of-a-kind structural fingerprint:

  * contiguous fixed-size records (288 bytes / 0x120)
  * a u16 "running index" at record+0x5D that increments 0,1,2,... with no gaps
  * an identity-struct pointer at record+0x08 -> struct whose +0x10 is the
    country's localized name as an MSVC std::string

Robust to relaunch (heap moves) and tolerant of patches (works while the record
layout holds), unlike a brittle deep pointer chain.

Design incorporates Codex review (2026-06-24):
  * MSVC std::string SSO is decided by CAPACITY (+0x18), not size.
  * Seed on a rare mid-range index, verify the run IN-BUFFER before remote reads
    (the old b"\\x00\\x00" seed was ubiquitous -> 6 min and skipped record 0).

LIVENESS REWORK (2026-06-26, Codex-vetted): the original locator required a UNIQUE
external owner pointer (find_vector_owner) to prove the candidate was LIVE. That
owner check is the documented stale-ARENA / custom-container false-NEGATIVE: it
returns None for the REAL live array on many sessions (the engine's container does
not expose the std::vector begin/end/cap triple it looks for), so locate() returned
None even though the array was right there. WORSE, the old name check
(_verify_records) only inspected records 0..23, but the ACTIVE roster can be packed
at HIGH indices (observed: countries at indices 67..226, records 0..96 unnamed) ->
zero names in the head -> rejected. Both gates rejected the live array.

The reworked locator proves liveness WITHOUT a mandatory owner pointer:
  (L1) clean u16 index run 0,1,...,count-1 over the WHOLE array (not just the head),
  (L2) a WHOLE-ARRAY name quorum: >= NAME_QUORUM_MIN records whose [rec+0x08] object
       resolves to a printable localized name, AND >= ANCHOR_QUORUM_MIN well-known
       major-country names present (collision-free identity, no tag table needed),
  (L3) a DOUBLE-READ (re-fetched from process memory) of a few named records is
       stable (TOCTOU / mid-free guard),
  (L4) the named-record objects are INJECTIVE (no duplicate pointer -- a healthy
       registry is 1:1; a torn/half-rebuilt copy is not).
find_vector_owner is kept as a BONUS only (its begin-field is cached when it
happens to succeed). When MORE THAN ONE array passes L1-L4 (a mirror copy), they
are accepted only if they AGREE (same count, same object + PP for every shared
index); any disagreement -> None (fail closed). This is stale-safer than the
owner-mandatory original: empirically exactly one injective clean-index array with
the anchor quorum exists per session, and a stale copy that is the ONLY survivor
(live array mid-rebuild) fails the quorum -> None (safe), never a "best effort" pick.

Offsets are relative to a RECORD START (= the PP field - 0x20), verified live
on build 16022851.
"""

import time
import winmem

REC = 0x120          # 288-byte records
PAGE = 0x1000        # min forward-progress on a partial/short/final chunk read (anti-stall)
OFF_NAMEPTR = 0x08   # -> identity struct
OFF_PP = 0x20        # i32, fixed-point x100000 (PP only; do NOT assume for other fields)
OFF_INDEX = 0x5D     # u16 running index
PP_SCALE = 100000.0
NAME_AT = 0x10       # name std::string within the identity struct
ADJ_AT = 0x30

# seed indices to look for in the index column; any array with > max(these)+1
# records will contain them. First match wins. Rare enough to scan fast.
SEED_IDXS = (16, 24, 40, 64)
RUN_BACK, RUN_FWD = 12, 12   # in-buffer run check window (records)
VERIFY_DEPTH = 24            # remote index-run depth (cheap head pre-filter)
MIN_COUNT, MAX_COUNT = 40, 1000

HEAP_LO = 0x10000000000      # plausible committed-heap address window for a country
HEAP_HI = 0x7FFFFFFFFFFF     # object pointer (defends the name read from wild ptrs)

# Liveness quorum thresholds (L2/L4). The WHOLE-ARRAY name quorum tolerates an active
# roster packed at high indices; the anchor quorum is a collision-free identity proof
# that needs no tag table. Both are conservative: a real session has ~71 named records
# and ~10 anchors; a non-country array has ~0.
NAME_QUORUM_MIN = 20         # >= this many records resolve to a printable name
ANCHOR_QUORUM_MIN = 6        # >= this many well-known major-country names present
DOUBLE_READ_SAMPLE = 8       # named records double-read for TOCTOU stability

# Well-known major-country localized names. Presence of several is collision-free
# evidence this is THE country array (a random struct array never holds these). Kept
# in sync with country_index_bridge.ANCHOR_NAMES; a few extra stable English names.
ANCHOR_NAMES = frozenset((
    "German Reich", "United Kingdom", "Soviet Union", "Sweden", "Italy",
    "Japanese Empire", "United States", "British Raj", "Poland", "Hungary",
    "Greater Hungary", "Norway", "Portugal", "Finland", "Australia",
    "Switzerland", "Nationalist Spain", "Dominion of Canada", "France",
))


MAX_STR_CAP = 1 << 20   # any std::string capacity above this is garbage, not a name

def read_msvc_str(p, so, maxlen=80):
    """Read an MSVC std::string. SSO is decided by CAPACITY (+0x18): inline when
    cap < 16, else +0 is a heap pointer to the chars (Codex review). M5: bound cap
    and (heap case) require the NUL terminator at +size, to shrink the
    false-positive surface when validating candidate identity structs."""
    try:
        size = p.u64(so + 0x10)
        cap = p.u64(so + 0x18)
    except OSError:
        return None
    if size == 0 or size > maxlen or cap < size or cap > MAX_STR_CAP:
        return None
    try:
        chars = so if cap < 16 else p.u64(so)
        data = p.read(chars, size + 1)          # include the NUL slot
    except OSError:
        return None
    if data[size] != 0:                         # MSVC std::string is NUL-terminated
        return None
    s = data[:size].decode("latin-1")
    return s if all(32 <= ord(c) < 127 for c in s) else None


def _verify_records(p, rec0, depth=VERIFY_DEPTH):
    """Cheap HEAD pre-filter: a clean 0..depth index run only.

    DELIBERATELY no longer requires names in the head -- the active roster can be
    packed at HIGH indices (records 0..96 unnamed on some sessions), so a head name
    check produced FALSE NEGATIVES that rejected the live array. Identity + liveness
    is proven over the WHOLE array by _prove_live() after the count is known. This
    stays as a fast reject for non-array hits (a random struct rarely has a clean
    0..23 u16 run at +0x5D)."""
    try:
        for k in range(depth):
            if p.u16(rec0 + k * REC + OFF_INDEX) != k:
                return False
    except OSError:
        return False
    return True


def _read_name_at(p, rec):
    """[rec+0x08] -> object -> +0x10 localized name, or None. Guards the object
    pointer against wild values before dereferencing (the country array has unnamed
    placeholder slots whose +0x08 may be garbage)."""
    try:
        obj = p.u64(rec + OFF_NAMEPTR)
    except OSError:
        return None
    if not (HEAP_LO <= obj <= HEAP_HI):
        return None
    return read_msvc_str(p, obj + NAME_AT)


def _array_identity(p, rec0, count):
    """Walk ALL `count` records ONCE; return (names_by_index, objs_by_index).

    names_by_index = {running_index: localized_name} for every named record;
    objs_by_index  = {running_index: object_ptr} for the same records. The WHOLE-array
    walk is what makes the locator robust to a high-index-packed active roster."""
    names = {}
    objs = {}
    for i in range(count):
        rec = rec0 + i * REC
        try:
            obj = p.u64(rec + OFF_NAMEPTR)
        except OSError:
            continue
        if not (HEAP_LO <= obj <= HEAP_HI):
            continue
        nm = read_msvc_str(p, obj + NAME_AT)
        if nm:
            names[i] = nm
            objs[i] = obj
    return names, objs


def _double_read_stable(p, rec0, indices):
    """L3: re-fetch (FROM PROCESS MEMORY, not a cached buffer) the index, object
    pointer and name at a sample of records; all must be identical across the two
    reads. Guards against a TOCTOU free / mid-rebuild between the quorum walk and
    acceptance. Returns False on any mismatch or read error."""
    for i in indices:
        rec = rec0 + i * REC
        try:
            i1 = p.u16(rec + OFF_INDEX)
            o1 = p.u64(rec + OFF_NAMEPTR)
            n1 = _read_name_at(p, rec)
            i2 = p.u16(rec + OFF_INDEX)
            o2 = p.u64(rec + OFF_NAMEPTR)
            n2 = _read_name_at(p, rec)
        except OSError:
            return False
        if i1 != i2 or o1 != o2 or n1 != n2 or i1 != i:
            return False
    return True


def _prove_live(p, rec0, count):
    """L1-L4 liveness proof for a candidate country array (Codex-vetted 2026-06-26).

    Returns a dict {quorum, anchors, names, objs} if the candidate is a LIVE country
    array, else None. NO external owner pointer is required (find_vector_owner is the
    documented stale-ARENA false-negative); instead:
      L1 clean full index run (already proven by the caller's count walk, re-affirmed
         cheaply here via the names/objs walk staying on-index),
      L2 WHOLE-ARRAY name quorum >= NAME_QUORUM_MIN AND >= ANCHOR_QUORUM_MIN of the
         well-known major-country ANCHOR_NAMES present (collision-free identity),
      L3 a DOUBLE-READ of a sample of named records is stable (TOCTOU guard),
      L4 the named-record objects are INJECTIVE (no duplicate object pointer).
    A non-country array fails L2; a torn/half-rebuilt copy fails L3 or L4; a stale
    copy whose live original is mid-rebuild simply fails the quorum -> None (safe)."""
    names, objs = _array_identity(p, rec0, count)
    quorum = len(names)
    if quorum < NAME_QUORUM_MIN:
        return None
    anchors = sum(1 for nm in names.values() if nm in ANCHOR_NAMES)
    if anchors < ANCHOR_QUORUM_MIN:
        return None
    # L4 injectivity: a healthy registry maps each slot to a DISTINCT object.
    objvals = list(objs.values())
    if len(set(objvals)) != len(objvals):
        return None
    # L3 double-read a representative sample (spread across the named records).
    named_idx = sorted(names)
    if named_idx:
        step = max(1, len(named_idx) // DOUBLE_READ_SAMPLE)
        sample = named_idx[::step][:DOUBLE_READ_SAMPLE]
        if not _double_read_stable(p, rec0, sample):
            return None
    return {"quorum": quorum, "anchors": anchors, "names": names, "objs": objs}


def _candidates_agree(p, a, b):
    """Two passing candidates (dicts from _collect with rec0/count/proof) are mirror
    copies iff they have the SAME count and, for every shared running_index that BOTH
    name, the SAME object pointer AND the same PP i32 at rec+0x20. Returns True if they
    agree (-> they are copies, keep the better one), False if they DISAGREE (ambiguous
    -> the caller fails closed)."""
    if a["count"] != b["count"]:
        return False
    shared = set(a["proof"]["objs"]) & set(b["proof"]["objs"])
    # Codex fail-closed: two DISTINCT live arrays with disjoint named rosters would
    # have an EMPTY intersection -> a vacuous "agree". Real mirror copies of the SAME
    # array share their entire named-index set, so require a substantial overlap; an
    # empty (or trivially small) shared set is ambiguous, not agreement.
    if len(shared) < NAME_QUORUM_MIN:
        return False
    for i in shared:
        if a["proof"]["objs"][i] != b["proof"]["objs"][i]:
            return False
        try:
            if p.i32(a["rec0"] + i * REC + OFF_PP) != p.i32(b["rec0"] + i * REC + OFF_PP):
                return False
        except OSError:
            return False
    return True


def count_records(p, rec0, cap=MAX_COUNT):
    """Walk the index run forward; array length = first index that breaks.

    Returns (count, truncated). `truncated` is True iff the walk hit `cap` while
    the run was still clean -> the real array may be longer than we counted and
    `count`/`expected_end` are unreliable (H5: never silently saturate)."""
    n = 0
    try:
        while n < cap and p.u16(rec0 + n * REC + OFF_INDEX) == n:
            n += 1
    except OSError:
        return n, False
    return n, n >= cap


def _is_live_holder(p, addr, rec0):
    """Re-read [addr] right before accepting (B1): confirm it still equals rec0
    (defends against a TOCTOU free between scan and accept)."""
    try:
        return p.u64(addr) == rec0
    except OSError:
        return False


def find_vector_owner(p, rec0, count, regions=None, scan_range=None):
    """Prove the array is LIVE by finding its owner pointer (B1 hardened).

    Codex's generic advice was to validate the owner as a std::vector
    begin/end/cap triple. On THIS build the container is NOT a std::vector: the
    owner's +8/+0x10 (would-be end/cap) are both zero (measured live). So we
    enforce a stronger, fail-closed contract instead of "first echo wins":

      1. Collect EVERY external, 8-aligned qword in committed memory that holds
         rec0 (outside the array's own block).
      2. A candidate is a real std::vector triple iff [+8]==expected_end,
         [+0x10]>=expected_end, and the capacity span is a sane multiple of REC.
      3. Decision (fail closed on ambiguity, per layout `fail_closed`):
           - exactly ONE vector triple            -> accept it
           - >1 vector triple                      -> None (ambiguous)
           - else exactly ONE external holder total-> require the measured custom
                                                      signature [+8]==0 && [+0x10]==0
           - else (>1 holder)                      -> accept ONLY if exactly one
                                                      passes the custom signature
           - otherwise                             -> None
      4. Re-read [owner]==rec0 immediately before returning (TOCTOU guard).

    A stale freed array would have no live external object referencing it (the
    live manager points at the current array). Returns the begin-field address to
    cache for fast same-session re-resolve (read [begin_field] + revalidate), or
    None when no owner can be uniquely proven."""
    needle = rec0.to_bytes(8, "little")
    arr_lo, arr_hi = rec0 - 0x1000, rec0 + count * REC + 0x1000
    expected_end = rec0 + count * REC
    CH = 16 << 20
    OV = 16  # 8-byte needle never straddles a 16-byte overlap
    holders = set()       # DISTINCT external aligned qwords == rec0 (overlapping
    triples = set()       # chunk reads can hit the same addr twice -> dedup so a
    custom = set()        # boundary holder can't fake a uniqueness failure, B1).
    _lo, _hi = scan_range if scan_range else (0x10000, 0x7FFFFFFFFFFF)
    for base, size in (regions or p.iter_regions(_lo, _hi)):
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
                    addr = base + pos + i
                    if addr % 8 != 0 or arr_lo <= addr < arr_hi:
                        continue                       # unaligned, or a self-ref
                    if addr in holders:
                        continue                       # already classified
                    try:
                        f8 = p.u64(addr + 8)
                        f10 = p.u64(addr + 0x10)
                    except OSError:
                        continue                       # can't classify -> ignore
                    holders.add(addr)
                    if (f8 == expected_end and f10 >= f8
                            and (f10 - rec0) % REC == 0
                            and (f10 - rec0) // REC <= MAX_COUNT):
                        triples.add(addr)              # real std::vector triple
                    elif f8 == 0 and f10 == 0:
                        custom.add(addr)               # this build's signature
            # Advance with a forward-progress guard. Keep the OV overlap ONLY for a
            # FULL mid-region chunk (where a needle could straddle the chunk
            # boundary); a final or short/partial read advances by the bytes read,
            # floored to PAGE, so `len(blob) - OV <= 0` can never stall the loop into
            # re-reading (and re-allocating) the same window millions of times (H3).
            if blob:
                blen = len(blob)
                if pos + blen >= size or blen < want:
                    pos += max(blen, PAGE)        # final chunk or short/partial read
                else:
                    pos += blen - OV              # full mid-region chunk (blen == CH)
            else:
                pos += max(want, PAGE)

    # B1 fail-closed: return an owner ONLY when exactly ONE acceptable holder
    # exists (a real std::vector triple XOR this build's {0,0} signature) AND no
    # other UNEXPLAINED external holder competes for rec0. Any unexplained holder,
    # a triple+custom mix, or >1 of either kind, is ambiguous -> fail closed. (The
    # earlier `len(custom)==1` accept fired even when other holders existed.)
    unexplained = holders - triples - custom
    if unexplained:
        return None                                    # a competing holder -> ambiguous
    if len(triples) + len(custom) != 1:
        return None                                    # 0, >1, or triple+custom mix
    addr = next(iter(triples)) if triples else next(iter(custom))
    return addr if _is_live_holder(p, addr, rec0) else None


def locate(p, progress=None, tbl=None, scan_range=None):
    """Scan committed heap for the LIVE country array via the index fingerprint.

    Returns (rec0, begin_field, count). A candidate must pass the cheap index
    fingerprint (clean head run + a sane count) AND the whole-array liveness proof
    _prove_live() (name + anchor quorum, double-read stable, injective objects --
    NO mandatory owner pointer; see the module docstring). ALL passing candidates
    are collected; if more than one survives they must AGREE (mirror copies) or the
    result is ambiguous -> None (fail closed). The accepted candidate's begin_field
    is find_vector_owner()'s pointer when it happens to resolve (cacheable), else
    rec0 itself (resolve_cached treats a rec0-equal token as the base). `tbl` is
    accepted for API compatibility (the anchor-name quorum needs no tag table).
    Returns None if nothing qualifies.

    `scan_range` (optional, STAGE 3): an explicit (lo, hi) address window to scan instead of the
    full 47-bit range. Default None == the wide (0x10000, 0x7FFFFFFFFFFF) window (UNCHANGED
    behaviour). The capture context passes the tight arena bounds here ONLY when the (live-proven)
    arena-restrict flag is ON; OFF leaves this None -> identical scan to before."""
    lo, hi = scan_range if scan_range else (0x10000, 0x7FFFFFFFFFFF)
    seeds = [(idx, idx.to_bytes(2, "little")) for idx in SEED_IDXS]
    align_mod = OFF_INDEX % 8           # index-field addr % 8 (rec0 is >=8-aligned)
    CH = 16 << 20
    OV = (max(RUN_BACK, RUN_FWD) + 2) * REC
    scanned = 0
    candidates = {}                     # rec0 -> {rec0, count, proof}
    for base, size in p.iter_regions(lo, hi):
        if size < REC * (max(SEED_IDXS) + 2):
            continue
        pos = 0
        while pos < size:
            want = min(CH, size - pos)
            blob = p.try_read(base + pos, want)
            if blob:
                blen = len(blob)
                for sidx, sneedle in seeds:
                    s = 0
                    while True:
                        i = blob.find(sneedle, s)
                        if i < 0:
                            break
                        s = i + 1
                        if (base + pos + i) % 8 != align_mod:
                            continue
                        # in-buffer run check around this hypothesized index field
                        good = checked = 0
                        bad = False
                        for d in range(-RUN_BACK, RUN_FWD + 1):
                            q = i + d * REC
                            if q < 0 or q + 1 >= blen:
                                continue
                            checked += 1
                            if (blob[q] | (blob[q + 1] << 8)) == sidx + d:
                                good += 1
                            else:
                                bad = True
                                break
                        if bad or checked < 6 or good < checked:
                            continue
                        rec0 = base + pos + i - sidx * REC - OFF_INDEX
                        if rec0 in candidates:
                            continue                    # already proven (seed dup)
                        if not _verify_records(p, rec0):
                            continue                    # cheap head reject
                        count, truncated = count_records(p, rec0, MAX_COUNT)
                        if truncated or not (MIN_COUNT <= count <= MAX_COUNT):
                            continue                    # implausible/saturated (H5)
                        proof = _prove_live(p, rec0, count)
                        if proof is None:
                            continue                    # not the live country array
                        candidates[rec0] = {"rec0": rec0, "count": count,
                                            "proof": proof}
            scanned += want
            # Advance with a forward-progress guard (see find_vector_owner): keep the
            # OV-record overlap only for a FULL mid-region chunk; a final or
            # short/partial read advances by the bytes read, floored to PAGE, so
            # `blen - OV <= 0` can never stall the loop into a re-read/re-alloc storm.
            if blob:
                if pos + blen >= size or blen < want:
                    pos += max(blen, PAGE)        # final chunk or short/partial read
                else:
                    pos += blen - OV              # full mid-region chunk (blen == CH)
            else:
                pos += max(want, PAGE)
        if progress:
            progress(scanned)

    if not candidates:
        return None
    cands = list(candidates.values())
    if len(cands) > 1:
        # Multiple survivors: accept ONLY if they all agree (mirror copies). Any
        # disagreement on a shared index's object/PP -> ambiguous -> fail closed.
        ref = cands[0]
        for other in cands[1:]:
            if not _candidates_agree(p, ref, other):
                return None
    # Pick the largest name quorum (the live array has the full active roster);
    # ties -> lowest address (canonical/stable mirror). Equal quorum among agreeing
    # mirrors is harmless (their fields agree by the check above).
    best = max(cands, key=lambda c: (c["proof"]["quorum"], -c["rec0"]))
    rec0, count = best["rec0"], best["count"]
    owner = find_vector_owner(p, rec0, count)   # BONUS signal (often None this build)
    begin_field = owner if owner is not None else rec0
    return rec0, begin_field, count


def resolve_cached(p, begin_field):
    """Same-session fast path (H6): re-resolve the array from a cached token without
    a full scan, then re-prove liveness. The token is EITHER an owner begin-field
    (find_vector_owner succeeded -> rec0 = [begin_field]) OR rec0 ITSELF (the common
    case this build, where the owner check returns None and locate() caches rec0).
    We try both interpretations and revalidate with _verify_records + a sane count +
    the whole-array liveness proof. Returns (rec0, count) if still live, else None
    (caller should fall back to a full `locate()` re-scan = reallocation)."""
    # Codex fail-closed ordering: try interpretation B (token IS rec0; the common case
    # this build, owner=None) FIRST so a coincidental record-0 first-qword can't win;
    # then interpretation A (token is an owner pointer holding rec0). Collect every
    # interpretation that re-proves live; if they resolve to DIFFERENT bases, the token
    # is ambiguous -> None (a full re-scan disambiguates). De-dup so the same base from
    # both interpretations is not a false "ambiguous".
    candidates = []
    if begin_field % 8 == 0:
        candidates.append(begin_field)         # B: token-as-rec0 (preferred)
    try:
        deref = p.u64(begin_field)
        if deref % 8 == 0:
            candidates.append(deref)           # A: token-as-owner-pointer
    except OSError:
        pass
    proven = {}
    seen = set()
    for rec0 in candidates:
        if rec0 in seen:
            continue
        seen.add(rec0)
        if not _verify_records(p, rec0):
            continue
        count, truncated = count_records(p, rec0, MAX_COUNT)
        if truncated or not (MIN_COUNT <= count <= MAX_COUNT):
            continue
        if _prove_live(p, rec0, count) is None:
            continue
        proven[rec0] = count
    if not proven:
        return None
    if len(proven) != 1:
        return None                            # two distinct live bases -> ambiguous
    rec0 = next(iter(proven))
    return rec0, proven[rec0]


def read_country(p, rec0, i):
    rec = rec0 + i * REC
    pp = p.i32(rec + OFF_PP) / PP_SCALE      # signed cast before use (Codex)
    idx = p.u16(rec + OFF_INDEX)
    name = adj = None
    s = p.u64(rec + OFF_NAMEPTR)
    name = read_msvc_str(p, s + NAME_AT)
    adj = read_msvc_str(p, s + ADJ_AT)
    return idx, pp, name, adj


def _layout_sha256():
    """The exe SHA-256 this layout was mapped on, read from the sibling
    layout manifest. Returns None if the manifest is missing/unreadable."""
    import json
    import os
    path = os.path.join(os.path.dirname(os.path.abspath(__file__)),
                        "layout.16022851.json")
    try:
        with open(path, "r", encoding="utf-8") as fh:
            return json.load(fh).get("build", {}).get("exe_sha256")
    except (OSError, ValueError):
        return None


def _check_build_gate(p, force):
    """M7: refuse to run against an exe whose SHA-256 isn't the mapped build
    (the README's non-negotiable "fail closed" gate), unless --force is given.
    A missing/unreadable manifest is ALSO fail-closed: we can't verify, so we
    refuse rather than read an unknown build blindly."""
    want = _layout_sha256()
    got = p.fingerprint().get("sha256")
    if want is not None and got == want:
        print("  build gate: exe SHA-256 matches the mapped build (%s...)" % want[:12])
        return
    if want is None:
        msg = ("  build gate: no/unreadable layout manifest SHA -> cannot verify the "
               "build. Offsets are build-specific; reading blind risks SILENT MISREADS.")
    else:
        msg = ("  build gate: live exe SHA-256 %s does not match the mapped build %s\n"
               "  Offsets are build-specific; reading an unknown build risks SILENT "
               "MISREADS." % (got, want))
    if force:
        print(msg + "\n  --force given: continuing anyway (results are UNVERIFIED).")
    else:
        raise SystemExit(msg + "\n  Refusing (pass --force to override).")


def main():
    import argparse
    ap = argparse.ArgumentParser(description="Locate the game country array (read-only).")
    ap.add_argument("--force", action="store_true",
                    help="run even if the exe SHA-256 doesn't match the mapped build")
    args = ap.parse_args()

    p = winmem.open_process("hoi4.exe")
    if p is None:
        raise SystemExit("hoi4.exe not running")
    with p:
        _check_build_gate(p, args.force)
        print("locating country array via fingerprint (+ liveness)...")
        t0 = time.time()
        res = locate(p)
        if res is None:
            print("  locate: %.1fs -> NOT FOUND" % (time.time() - t0))
            raise SystemExit(1)
        rec0, begin_field, count = res
        print("  locate: %.1fs -> rec0=0x%X, count=%d, LIVE owner begin-field=0x%X (cache this)"
              % (time.time() - t0, rec0, count, begin_field))
        print("\n%4s %-10s %-26s %s" % ("idx", "PP", "name", "adjective"))
        shown = 0
        for i in range(0, count):           # use the located count, not a guess (L5)
            try:
                idx, pp, name, adj = read_country(p, rec0, i)
            except OSError:
                continue
            if idx != i:                    # stale/garbage slot -> skip (L5)
                continue
            if name and len(name) >= 2:
                shown += 1
                print("%4d %-10.3f %-26s %s" % (i, pp, name, adj or ""))
        print("\nactive countries: %d" % shown)


if __name__ == "__main__":
    main()
