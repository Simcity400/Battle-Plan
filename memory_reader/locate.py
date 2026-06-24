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
  * Validate the candidate is LIVE via its std::vector owner (begin/end/cap), and
    cache that begin-field address as the same-session fast path.

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


def read_msvc_str(p, so, maxlen=80):
    """Read an MSVC std::string. SSO is decided by CAPACITY (+0x18): inline when
    cap < 16, else +0 is a heap pointer to the chars (Codex review)."""
    try:
        size = p.u64(so + 0x10)
        cap = p.u64(so + 0x18)
    except OSError:
        return None
    if size == 0 or size > maxlen or cap < size:
        return None
    try:
        data = p.read(so if cap < 16 else p.u64(so), size)
    except OSError:
        return None
    s = data.decode("latin-1")
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


def count_records(p, rec0, cap=512):
    """Walk the index run forward; the array length = first index that breaks."""
    n = 0
    try:
        while n < cap and p.u16(rec0 + n * REC + OFF_INDEX) == n:
            n += 1
    except OSError:
        pass
    return n


def find_vector_owner(p, rec0, count, regions=None):
    """Prove the array is LIVE by finding its owner pointer.

    Codex's generic advice was to validate the owner as a std::vector
    begin/end/cap triple. But on THIS build the container is NOT a std::vector:
    the owner's +8/+0x10 (would-be end/cap) are both zero (measured live). So we
    keep the sound intent without the triple: require a heap-ALIGNED, EXTERNAL
    (outside the array's own block) pointer holding rec0. The level-1 reverse
    scan found exactly ONE such pointer (the begin field M1) - a stale freed
    array would have no live external object referencing it (the live manager
    points at the current array). If a future build DOES present a real vector
    triple we accept it preferentially. Returns the begin-field address to cache
    for fast same-session re-resolve (read [begin_field] + revalidate)."""
    needle = rec0.to_bytes(8, "little")
    arr_lo, arr_hi = rec0 - 0x1000, rec0 + count * REC + 0x1000
    expected_end = rec0 + count * REC
    CH = 16 << 20
    fallback = None
    for base, size in (regions or p.iter_regions(0x10000, 0x7FFFFFFFFFFF)):
        pos = 0
        while pos < size:
            blob = p.try_read(base + pos, min(CH, size - pos))
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
                    try:
                        end = p.u64(addr + 8)
                        cap = p.u64(addr + 0x10)
                    except OSError:
                        end = cap = 0
                    if end == expected_end and end <= cap:
                        return addr                    # real std::vector triple (preferred)
                    if fallback is None:
                        fallback = addr                # this build's signal: aligned external owner
            pos += max(1, CH - 16)
    return fallback


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
                        count = count_records(p, rec0, MAX_COUNT)
                        if not (MIN_COUNT <= count <= MAX_COUNT):
                            continue                    # implausible size -> not it
                        owner = find_vector_owner(p, rec0, count)
                        if owner is None:
                            continue                    # no live owner -> stale copy, keep looking
                        return rec0, owner, count
            scanned += want
            pos += max(1, CH - OV)
        if progress:
            progress(scanned)
    return None


def read_country(p, rec0, i):
    rec = rec0 + i * REC
    pp = p.i32(rec + OFF_PP) / PP_SCALE      # signed cast before use (Codex)
    idx = p.u16(rec + OFF_INDEX)
    name = adj = None
    s = p.u64(rec + OFF_NAMEPTR)
    name = read_msvc_str(p, s + NAME_AT)
    adj = read_msvc_str(p, s + ADJ_AT)
    return idx, pp, name, adj


def main():
    p = winmem.open_process("hoi4.exe")
    if p is None:
        raise SystemExit("hoi4.exe not running")
    with p:
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
        for i in range(0, 260):
            try:
                idx, pp, name, adj = read_country(p, rec0, i)
            except OSError:
                continue
            if name and len(name) >= 2:
                shown += 1
                print("%4d %-10.3f %-26s %s" % (i, pp, name, adj or ""))
        print("\nactive countries: %d" % shown)


if __name__ == "__main__":
    main()
