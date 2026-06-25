"""tag_table.py - LIVE resolver for the game tag-db-id -> 3-letter TAG table.

Removes the save dependency for country identity. The state owner/controller
fields (+0xC8/+0xCC on the per-state runtime object) and the division
owner/controller fields (+0x1C8/+0x1D0 on the CDivision body) all store a
1-based "tag-db-id". This module locates the in-memory array of MSVC
std::string TAGs (indexed directly by tag-db-id; index 0 == "---", the null
tag) and resolves tag_db_id -> TAG live.

Layout (build 16022851, see tag_table.draft.json):
  - contiguous inline array of MSVC std::string, stride 0x20
  - record = base + tag_db_id*0x20 ; chars @+0x00, size(u64) @+0x10, cap(u64) @+0x18
  - SSO: cap == 15, 0 < size <= 15 for real tags; index 0 == "---"
  - relocate per session by a 128-byte anchor of records index0..3
    ("---","GER","ENG","SOV"), disambiguated to the duplicate-free table.

READ-ONLY. stdlib + the project's winmem.py. No writes, no injection.

Usage (standalone):
    python tag_table.py            # locate + dump the live tag-db-id -> TAG map
Programmatic:
    import winmem; from live_layers import tag_table
    p = winmem.open_process("hoi4.exe")
    tbl = tag_table.TagTable.locate(p)
    tbl.tag(35)        # -> "JAP"
    tbl.as_map()       # -> {0:"---", 1:"GER", ...}
"""

import os
import struct
import sys
import time

# allow `import winmem` whether run as a module or a script
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import winmem  # noqa: E402

STRIDE = 0x20
OFF_CHARS = 0x00
OFF_SIZE = 0x10
OFF_CAP = 0x18
SSO_CAP = 15
SENTINEL = "---"

# Heap range (RESEARCH_memory_reader / owner crack). The table lives in the
# game-state arena but we scan the whole committed heap to stay build-robust.
HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000

# Fixed prefix that the array always starts with at game start (the engine's
# registration order). Used to build the relocation anchor and to validate.
FIXED_PREFIX = ["---", "GER", "ENG", "SOV"]
# A few more for validation (must hold across MP-AMSB; from the save DB order).
VALIDATE_PREFIX = ["---", "GER", "ENG", "SOV", "SWE", "FRA", "LUX", "BEL",
                   "HOL", "CZE", "POL"]

MAX_RECORDS = 1024          # sanity bound on table length
SANE_MIN_TAGS = 40          # at least this many real tags or it isn't the table


def _record_bytes(tag):
    """Raw 0x20 bytes of one cap-15 SSO std::string record holding `tag`."""
    s = tag.encode("latin-1")
    buf = s + b"\x00" * (16 - len(s))
    return buf + struct.pack("<Q", len(s)) + struct.pack("<Q", SSO_CAP)


def build_anchor():
    """128-byte anchor = records index0..3 ('---','GER','ENG','SOV')."""
    return b"".join(_record_bytes(t) for t in FIXED_PREFIX)


class TagTable(object):
    """A located live tag-db-id -> TAG table."""

    def __init__(self, p, base, count, tags):
        self.p = p
        self.base = base            # address of index 0 ("---")
        self.count = count          # number of valid records
        self._tags = tags           # list[str], index == tag_db_id

    # -- record reads ---------------------------------------------------
    @staticmethod
    def _read_record(p, addr):
        """Return the TAG string of an SSO std::string record, or None if the
        record is not a valid cap-15 SSO string of printable chars."""
        try:
            blob = p.read(addr, STRIDE)
        except OSError:
            return None
        size = struct.unpack_from("<Q", blob, OFF_SIZE)[0]
        cap = struct.unpack_from("<Q", blob, OFF_CAP)[0]
        if cap != SSO_CAP or not (0 < size <= 15):
            return None
        chars = blob[:size]
        if not all(32 <= c < 127 for c in chars):
            return None
        return chars.decode("latin-1")

    @classmethod
    def _read_all(cls, p, base):
        """Walk records from base until the first invalid one. Returns list."""
        tags = []
        i = 0
        while i < MAX_RECORDS:
            t = cls._read_record(p, base + i * STRIDE)
            if t is None:
                break
            tags.append(t)
            i += 1
        return tags

    @staticmethod
    def _is_canonical(tags):
        """A candidate is the canonical table iff:
          - starts with '---'
          - matches the fixed validate-prefix
          - has >= SANE_MIN_TAGS real tags
          - NO duplicate non-sentinel tags (the secondary copy fails this)."""
        if len(tags) < len(VALIDATE_PREFIX):
            return False
        if tags[:len(VALIDATE_PREFIX)] != VALIDATE_PREFIX:
            return False
        body = [t for t in tags if t and t != SENTINEL]
        if len(body) < SANE_MIN_TAGS:
            return False
        if len(body) != len(set(body)):
            return False               # duplicate tag -> reject (fail closed)
        return True

    # -- location -------------------------------------------------------
    @classmethod
    def locate(cls, p, progress=None):
        """Scan committed heap for the anchor; disambiguate to the canonical
        (duplicate-free) table. Returns a TagTable or None.

        Fail-closed: if more than one canonical candidate is found and they
        disagree on any index, returns None rather than guess."""
        anchor = build_anchor()
        alen = len(anchor)
        ov = alen + 8
        chunk = 32 << 20
        candidates = []           # (base, tags)
        scanned = 0
        for base, size in p.iter_regions(HEAP_LO, HEAP_HI, want_exec=False):
            pos = 0
            while pos < size:
                want = min(chunk, size - pos)
                blob = p.try_read(base + pos, want)
                if blob:
                    s = 0
                    while True:
                        i = blob.find(anchor, s)
                        if i < 0:
                            break
                        s = i + 1
                        cand_base = base + pos + i
                        tags = cls._read_all(p, cand_base)
                        if cls._is_canonical(tags):
                            candidates.append((cand_base, tags))
                    scanned += len(blob)
                pos += max(1, (len(blob) if blob else want) - ov)
            if progress:
                progress(scanned)

        if not candidates:
            return None
        # dedupe by base; if multiple canonical candidates, require agreement
        uniq = {}
        for b, t in candidates:
            uniq[b] = t
        if len(uniq) == 1:
            b, t = next(iter(uniq.items()))
            return cls(p, b, len(t), t)
        # multiple canonical tables: accept only if they agree on every shared
        # index (then pick the shortest = tightest active roster).
        bases = sorted(uniq, key=lambda b: len(uniq[b]))
        ref = uniq[bases[0]]
        for b in bases[1:]:
            other = uniq[b]
            n = min(len(ref), len(other))
            if ref[:n] != other[:n]:
                return None            # disagree -> fail closed
        b = bases[0]
        return cls(p, b, len(uniq[b]), uniq[b])

    # -- resolution -----------------------------------------------------
    def tag(self, tag_db_id):
        """tag_db_id -> 3-letter TAG (live re-read). 0 -> '---'. Out of range
        or invalid -> None."""
        if tag_db_id < 0 or tag_db_id >= self.count:
            return None
        return self._read_record(self.p, self.base + tag_db_id * STRIDE)

    def as_map(self):
        """Snapshot {tag_db_id: TAG} for all valid records (cached at locate)."""
        return {i: t for i, t in enumerate(self._tags)}

    def covers(self, tag_db_ids):
        """Fail-closed coverage check: every id in `tag_db_ids` is in range and
        resolves to a 3-char tag. Caller passes the set of live tag-db-ids seen in
        state owner/controller + division owner fields; if any is out of bounds or
        unresolved -> the table is too short / wrong -> fail closed (Codex c)."""
        for tid in tag_db_ids:
            if tid == 0:
                continue
            if tid < 0 or tid >= self.count:
                return False
            t = self.tag(tid)
            if not t or t == SENTINEL:
                return False
        return True

    def revalidate(self):
        """Re-read records 0..len(VALIDATE_PREFIX) and confirm the prefix still
        holds (same-session liveness / TOCTOU guard). Returns True if still
        valid, else the caller should re-locate (reallocation)."""
        for i, want in enumerate(VALIDATE_PREFIX):
            if self._read_record(self.p, self.base + i * STRIDE) != want:
                return False
        return True


def main():
    p = winmem.open_process("hoi4.exe")
    if p is None:
        raise SystemExit("hoi4.exe not running")
    with p:
        t0 = time.time()
        tbl = TagTable.locate(p)
        if tbl is None:
            print("tag table: NOT FOUND (%.1fs)" % (time.time() - t0))
            raise SystemExit(1)
        print("tag table located in %.1fs: base=0x%X count=%d"
              % (time.time() - t0, tbl.base, tbl.count))
        m = tbl.as_map()
        print("tag-db-id -> TAG (%d entries):" % len(m))
        line = []
        for i in sorted(m):
            line.append("%d=%s" % (i, m[i]))
            if len(line) == 10:
                print("  " + "  ".join(line))
                line = []
        if line:
            print("  " + "  ".join(line))


if __name__ == "__main__":
    main()
