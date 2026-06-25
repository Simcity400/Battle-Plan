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
  * Validate the candidate is LIVE via a UNIQUE external owner pointer (a real
    std::vector begin/end/cap triple when present, else this build's measured
    custom-container signature), failing closed on ambiguity (find_vector_owner).
  * Cache that begin-field address for the same-session fast path: resolve_cached()
    re-reads [begin_field] and revalidates the index run + names without rescanning.

Offsets are relative to a RECORD START (= the PP field - 0x20), verified live
on build 16022851.
"""

import time
import winmem

REC = 0x120          # 288-byte records
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
VERIFY_DEPTH = 24            # remote index-run depth
MIN_COUNT, MAX_COUNT = 40, 1000


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
    """Clean 0..depth index run + >=2 printable identity names (remote reads)."""
    try:
        for k in range(depth):
            if p.u16(rec0 + k * REC + OFF_INDEX) != k:
                return False
    except OSError:
        return False
    names = 0
    for k in range(depth):
        try:
            s = p.u64(rec0 + k * REC + OFF_NAMEPTR)
        except OSError:
            continue
        if read_msvc_str(p, s + NAME_AT):
            names += 1
            if names >= 2:
                return True
    return names >= 2


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


def find_vector_owner(p, rec0, count, regions=None):
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
    for base, size in (regions or p.iter_regions(0x10000, 0x7FFFFFFFFFFF)):
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
            # advance by bytes actually read, keeping the overlap (H3): a short
            # read at a region edge must not skip the unread tail.
            if blob:
                pos += max(1, len(blob) - OV)
            else:
                pos += max(1, want)

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


def locate(p, progress=None):
    """Scan committed heap for the country array via the index fingerprint.

    Returns (rec0, begin_field, count) once a candidate passes the index/name
    fingerprint AND is confirmed LIVE by a sane record count plus an external
    owner pointer (Codex fix: never return on the fingerprint alone, which could
    be a stale freed copy). Returns None if nothing qualifies."""
    seeds = [(idx, idx.to_bytes(2, "little")) for idx in SEED_IDXS]
    align_mod = OFF_INDEX % 8           # index-field addr % 8 (rec0 is >=8-aligned)
    CH = 16 << 20
    OV = (max(RUN_BACK, RUN_FWD) + 2) * REC
    scanned = 0
    for base, size in p.iter_regions(0x10000, 0x7FFFFFFFFFFF):
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
                        if not _verify_records(p, rec0):
                            continue
                        count, truncated = count_records(p, rec0, MAX_COUNT)
                        if truncated or not (MIN_COUNT <= count <= MAX_COUNT):
                            continue                    # implausible/saturated -> not it (H5)
                        owner = find_vector_owner(p, rec0, count)
                        if owner is None:
                            continue                    # no live owner -> stale copy, keep looking
                        return rec0, owner, count
            scanned += want
            # advance by bytes actually read (H3) so a short read near a region
            # edge doesn't skip the unread tail; keep the OV-record overlap.
            if blob:
                pos += max(1, blen - OV)
            else:
                pos += max(1, want)
        if progress:
            progress(scanned)
    return None


def resolve_cached(p, begin_field):
    """Same-session fast path (H6): re-resolve the array from a cached begin-field
    address without a full scan. Reads rec0 = [begin_field], then revalidates the
    index run + names + a sane count. Returns (rec0, count) if still live, else
    None (caller should fall back to a full `locate()` re-scan = reallocation)."""
    try:
        rec0 = p.u64(begin_field)
    except OSError:
        return None
    if rec0 % 8 != 0 or not _verify_records(p, rec0):
        return None
    count, truncated = count_records(p, rec0, MAX_COUNT)
    if truncated or not (MIN_COUNT <= count <= MAX_COUNT):
        return None
    return rec0, count


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
