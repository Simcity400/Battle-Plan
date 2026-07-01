"""fastscan.py - PARALLEL, READ-ONLY heap-scan primitive for the game memory reader.

Single-threaded Python RPM scans peg one core (~12 MB/s); an ~8-12 GB arena then
takes 15-22 min while 7 cores idle. fastscan shards the committed arena regions
across N worker PROCESSES (default cpu_count()-2), each of which opens its OWN
read-only handle to the target pid and scans its assigned regions with a
numpy-vectorized matcher. The parent aggregates the per-worker hit lists.

This is the standard scan primitive for future RE and the live snapshot writer.

------------------------------------------------------------------------------
DESIGN / CORRECTNESS INVARIANTS  (verified adversarially; see fastscan_bench_*)
------------------------------------------------------------------------------
  * READ-ONLY. Every handle is OpenProcess(PROCESS_VM_READ|PROCESS_QUERY_INFORMATION).
    We never request write rights and never call WriteProcessMemory. winmem.py is
    imported read-only for its kernel32 prototypes / constants; it is never edited.
  * ENUMERATE ONCE. The parent walks VirtualQueryEx exactly once and freezes the
    region list (base,size). Workers do NOT re-query; they read the exact regions
    handed to them. So the scanned set is identical regardless of worker count.
  * EVERY REGION EXACTLY ONCE. Regions are partitioned (not overlapped) across
    workers by a greedy longest-processing-time bin-pack on byte count. The union
    of all shards == the full region list, with no region in two shards. (A
    boundary-straddling hit cannot occur ACROSS regions because reads are clamped
    to region ends and the arena regions are non-contiguous allocations; within a
    region, chunk overlap covers straddling matches -- see below.)
  * CLAMP EVERY READ TO THE REGION END. A ReadProcessMemory span that crosses a
    VirtualQuery region boundary returns 0 for the WHOLE span on this target, so a
    naive over-read silently drops a region. Each chunk read is min(chunk, end-pos)
    and never reaches past the region's own [base, base+size).
  * CHUNK OVERLAP. Within a region we read in windows with (item_size-1) bytes of
    overlap so a match straddling two windows is still found exactly once (dedup by
    the parent via a set, and by clamping emitted hits to [base, base+size-item)).
  * OWN HANDLE PER WORKER. Each worker process calls OpenProcess itself. Handles
    are per-process kernel objects; there is no shared handle and no cross-process
    RPM race. The parent's handle (used only for enumeration) is closed before fork
    is irrelevant -- spawn re-imports cleanly and workers never inherit it.
  * SPAWN-SAFE MATCHER. Windows multiprocessing uses 'spawn', so the matcher must
    be importable in a fresh interpreter. Pass it as a ("module", "func") pair
    (preferred) OR a top-level picklable callable. A lambda / closure / local
    function will fail to pickle -- documented and enforced with a clear error.
  * SNAPSHOT CAVEAT. parallel_scan and single_scan produce the SAME hit set over a
    STATIC memory image: they run identical matcher logic over the identical frozen
    region list; only the partition differs, and the matchers are phase-aware so
    chunking is invisible. They are NOT guaranteed identical if the LIVE game
    mutates the scanned bytes mid-scan (the game ticks): each path samples memory at
    slightly different wall-clock times. For a coherent snapshot, pause the game or
    accept the small skew. The benchmark asserts equality because, for stable
    vtable slots over the run, the observed image did not change.

------------------------------------------------------------------------------
MATCHER CONTRACT
------------------------------------------------------------------------------
  matcher(base:int, blob:bytes) -> list[int]      # absolute hit addresses

  The blob is one clamped chunk starting at `base`. The matcher returns absolute
  addresses (base + offset) of every hit it finds in `blob`. It MUST be pure /
  stateless and depend only on (base, blob) plus picklable args bound at build
  time. Inner loops are numpy-vectorized (frombuffer + strided/broadcast compare).

  Three builders are provided (each returns a top-level-importable partial):
    * make_qword_eq_matcher(target)            -> aligned u64 == target (vtable scan)
    * make_aos_field_matcher(stride, foff, fsize, pred_kind, value)
                                               -> AoS fixed-stride field test
    * make_bytes_find_matcher(pattern_hex)     -> byte-pattern find (overlapping)

USAGE
-----
    from memory_reader import fastscan as fs
    hits = fs.parallel_scan(
        pid, fs.ARENA_LO, fs.ARENA_HI,
        matcher=("memory_reader.fastscan", "_m_qword_eq"),
        matcher_args=(0x7ff786450ac0,),     # absolute vtable address
        workers=6)
    # hits is a sorted list[int] of absolute addresses.

    # The qword-eq builder wires the (module,func)+args for you:
    spec = fs.make_qword_eq_matcher(vtable_abs)
    hits = fs.parallel_scan(pid, fs.ARENA_LO, fs.ARENA_HI, **spec, workers=6)
"""

import ctypes as C
import os
import struct
import sys
from ctypes import wintypes as W

import numpy as np

# Import winmem READ-ONLY for the proven kernel32 prototypes + constants. We do
# NOT instantiate winmem.Process in workers (its module-enumeration is parent-only
# work); we reuse its _k32 handle table and MEMORY_BASIC_INFORMATION struct so the
# WinAPI argtypes are exactly the audited ones (no 64-bit truncation).
_THIS_DIR = os.path.dirname(os.path.abspath(__file__))
_PARENT_DIR = os.path.dirname(_THIS_DIR)
# _THIS_DIR makes `import winmem` work; _PARENT_DIR makes the package path
# `memory_reader.fastscan` importable too, so spawn workers and the matcher
# resolver work even when this file is run directly (python fastscan.py).
for _p in (_THIS_DIR, _PARENT_DIR):
    if _p not in sys.path:
        sys.path.insert(0, _p)
import winmem as _wm  # read-only import; never mutated

# ---- arena + heap bounds (canonical, from navcombat_lib / RESEARCH) ----------
ARENA_LO = 0x1F600000000
ARENA_HI = 0x1FA00000000
HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000

_PROCESS_VM_READ = _wm.PROCESS_VM_READ                  # 0x0010
_PROCESS_QUERY_INFORMATION = _wm.PROCESS_QUERY_INFORMATION  # 0x0400
_MEM_COMMIT = _wm.MEM_COMMIT
_PAGE_GUARD = _wm.PAGE_GUARD
_READABLE = _wm._READABLE
_k32 = _wm._k32
_MBI = _wm.MEMORY_BASIC_INFORMATION

_CHUNK = 16 << 20          # 16 MB read window
_MAX_REGION = 1 << 31      # ignore absurd (>2 GB) single regions (matches navcombat_lib)


def isheap(v):
    return v is not None and HEAP_LO <= v < HEAP_HI


# ==========================================================================
# Region enumeration (PARENT ONLY, exactly once)
# ==========================================================================
def enumerate_regions(handle, lo, hi):
    """Walk VirtualQueryEx once over [lo, hi); return [(base, size), ...] for
    committed, readable, non-guard regions, clamped to [lo, hi). Sizes are the
    true region sizes (already clamped); each tuple is a self-contained span a
    worker reads without re-querying."""
    out = []
    addr = lo
    mbi = _MBI()
    while addr < hi:
        if _k32.VirtualQueryEx(handle, C.c_void_p(addr), C.byref(mbi), C.sizeof(mbi)) == 0:
            addr += 0x1000
            continue
        rbase = int(mbi.BaseAddress or addr)
        rsize = int(mbi.RegionSize or 0x1000)
        nxt = rbase + rsize
        prot = mbi.Protect
        low = prot & 0xFF
        if (mbi.State == _MEM_COMMIT and low in _READABLE and not (prot & _PAGE_GUARD)
                and rsize <= _MAX_REGION):
            start = max(rbase, lo)
            stop = min(rbase + rsize, hi)
            if stop > start:
                out.append((start, stop - start))
        addr = nxt if nxt > addr else addr + 0x1000
    return out


def discover_arena(pid, gap=0x100000000):
    """Auto-locate the live heap arena. ASLR relocates it EVERY launch, so the
    ARENA_LO/HI constants above are only a stale hint — after a crash-reload they
    can point at MEM_FREE space, and a scan over them finds nothing (a silent
    false 0==0 pass). This walks the whole heap range once and returns (lo, hi)
    bounding the densest cluster of committed readable regions (a cluster is split
    on a > `gap` address jump), or None. Never hardcode the arena; call this per run."""
    h = _k32.OpenProcess(_PROCESS_VM_READ | _PROCESS_QUERY_INFORMATION, False, pid)
    if not h:
        return None
    try:
        regions = enumerate_regions(h, HEAP_LO, HEAP_HI)
    finally:
        _k32.CloseHandle(h)
    if not regions:
        return None
    regions.sort()
    best = None                       # (total_bytes, lo, hi)
    clo, chi, ctot = regions[0][0], regions[0][0] + regions[0][1], regions[0][1]
    for base, size in regions[1:]:
        if base - chi > gap:
            if best is None or ctot > best[0]:
                best = (ctot, clo, chi)
            clo, chi, ctot = base, base + size, size
        else:
            chi = max(chi, base + size)
            ctot += size
    if best is None or ctot > best[0]:
        best = (ctot, clo, chi)
    return (best[1], best[2])


def shard_regions(regions, n):
    """Partition `regions` into `n` shards balanced by total bytes, with EVERY
    region in EXACTLY ONE shard (no region split, none dropped, none duplicated).

    Greedy longest-processing-time: sort regions by size desc, assign each to the
    currently-lightest bin. Returns a list of <= n shard-lists (empty bins dropped).
    """
    n = max(1, int(n))
    bins = [[] for _ in range(n)]
    loads = [0] * n
    for base, size in sorted(regions, key=lambda r: r[1], reverse=True):
        j = loads.index(min(loads))     # lightest bin
        bins[j].append((base, size))
        loads[j] += size
    return [b for b in bins if b]


# ==========================================================================
# Per-worker read machinery (each worker owns its handle)
# ==========================================================================
class _Reader(object):
    """A worker's own read-only handle + chunked, region-clamped RPM."""

    def __init__(self, pid):
        self.h = _k32.OpenProcess(
            _PROCESS_VM_READ | _PROCESS_QUERY_INFORMATION, False, pid)
        if not self.h:
            raise OSError("OpenProcess(%d) failed (err %d)"
                          % (pid, C.get_last_error()))

    def close(self):
        if self.h:
            _k32.CloseHandle(self.h)
            self.h = None

    def rpm(self, addr, n):
        """Read up to n bytes; return what was copied (possibly short, possibly b'')."""
        buf = (C.c_char * n)()
        got = C.c_size_t(0)
        ok = _k32.ReadProcessMemory(self.h, C.c_void_p(addr), buf, n, C.byref(got))
        if not ok and got.value == 0:
            return b""
        return bytes(buf[:got.value])

    def scan_region(self, base, size, matcher, item_size):
        """Read [base, base+size) in CLAMPED, OVERLAPPING windows; run matcher on
        each; return absolute hits within [base, base+size). Overlap = item_size-1
        so a hit straddling two windows is found; emitted hits are clamped to the
        region so a hit's full item never extends past base+size.

        Advance rule (Codex review #7): we read in _CHUNK windows and re-present the
        last (item_size-1) bytes to the next window so a straddling match is seen.
        _CHUNK >> overlap, so on a FULL read we always make >> overlap forward
        progress. The only way `len(blob) <= overlap` is a SHORT read at an
        unreadable boundary: then no whole item fits in the remaining unread tail
        that abuts an unreadable page, so we advance PAST the bytes we actually got
        (no useless overlap re-read into a page that just refused us) -- guaranteeing
        progress and never an infinite loop or re-thrash."""
        overlap = max(0, item_size - 1)
        # Guard (Codex review): the full-mid advance `pos += got - overlap` only
        # makes forward progress when chunk > overlap. _CHUNK >> overlap for all real
        # strides, but enforce it so a pathological item_size >= _CHUNK can't go backward.
        chunk = _CHUNK if _CHUNK > overlap else overlap + 0x1000
        region_end = base + size
        hits = []
        pos = 0
        while pos < size:
            want = min(chunk, size - pos)           # CLAMP to region end
            blob = self.rpm(base + pos, want)
            got = len(blob)
            if blob:
                for a in matcher(base + pos, blob):
                    # a hit's full item must fit inside the region
                    if base <= a <= region_end - item_size:
                        hits.append(a)
                if got >= want and pos + got < size:
                    # full MID-region window: overlap the tail into the next window so
                    # a match straddling the boundary is found (deduped by the parent).
                    pos += got - overlap
                else:
                    # short read OR final window: advance past everything we read (the
                    # refused/region boundary has nothing to straddle into). Floor to a
                    # PAGE (0x1000) -- the agreed canonical floor: dominates overlap+1
                    # on both axes (no sub-page crawl for tiny overlaps; no over-skip of
                    # readable data past a partial-read boundary for large item_size).
                    pos += max(got, 0x1000)
            else:
                pos += max(want, 0x1000)
        return hits


class _FrozenReader(object):
    """A _Reader-compatible reader backed by a pre-captured {base: bytes} snapshot
    (no RPM). Used ONLY by the benchmark to prove chunk/shard/matcher/dedup logic
    over an identical, mutation-free image. NOT used on the production path."""

    def __init__(self, snapshot):
        self._snap = snapshot          # {region_base: bytes}

    def close(self):
        pass

    def rpm(self, addr, n):
        # find the region this read belongs to (reads are always within one region)
        for rbase, data in self._snap.items():
            if rbase <= addr < rbase + len(data):
                off = addr - rbase
                return data[off:off + n]      # naturally clamps to captured bytes
        return b""

    # reuse the real chunking/clamp/advance logic verbatim
    scan_region = _Reader.scan_region


# ==========================================================================
# Worker entrypoint  (top-level so spawn can import + pickle it)
# ==========================================================================
def _resolve_matcher(matcher, matcher_args):
    """Turn (module,func)|callable + args into a concrete (base,blob)->list fn."""
    if isinstance(matcher, (tuple, list)) and len(matcher) == 2:
        import importlib
        mod = importlib.import_module(matcher[0])
        fn = getattr(mod, matcher[1])
    elif callable(matcher):
        fn = matcher
    else:
        raise TypeError("matcher must be a (module_name, func_name) pair or a "
                        "top-level picklable callable; got %r" % (matcher,))
    args = tuple(matcher_args or ())

    def _bound(base, blob):
        return fn(base, blob, *args)
    return _bound


def _scan_shard_with_reader(reader, shard, matcher, matcher_args, item_size):
    """Scan every region in `shard` using an already-open reader (real or frozen).
    Returns a flat list of absolute hits (NOT deduped; caller unions). Shared by
    the production worker and the frozen-image benchmark so they exercise the SAME
    chunk/clamp/advance/matcher logic."""
    fn = _resolve_matcher(matcher, matcher_args)
    hits = []
    for base, size in shard:
        hits.extend(reader.scan_region(base, size, fn, item_size))
    return hits


def _worker(pid, shard, matcher, matcher_args, item_size):
    """Worker process: own handle, scan every region in `shard`, return hits."""
    rd = _Reader(pid)
    try:
        return _scan_shard_with_reader(rd, shard, matcher, matcher_args, item_size)
    finally:
        rd.close()


# ==========================================================================
# Public driver
# ==========================================================================
def parallel_scan(pid, lo, hi, matcher, matcher_args=(), item_size=8,
                  workers=None, regions=None):
    """Parallel arena scan.

    pid          : target process id (read-only handle opened per worker).
    lo, hi       : address range to enumerate/scan (use ARENA_LO/ARENA_HI).
    matcher      : ("module_name","func_name") pair (preferred under spawn) OR a
                   top-level picklable callable with signature
                   matcher(base, blob, *matcher_args) -> list[int].
                   Lambdas/closures/local functions are NOT picklable -> rejected.
    matcher_args : extra picklable args bound to the matcher (e.g. (target_value,)).
    item_size    : size in bytes of the smallest item a single hit refers to (8 for
                   a qword scan; the AoS stride; the pattern length). Controls
                   window overlap and the in-region clamp. MUST be >= 1.
    workers      : process count (default max(1, cpu_count()-2)).
    regions      : optional pre-enumerated [(base,size)] to reuse; if None we
                   enumerate once here.

    Returns a SORTED list[int] of unique absolute hit addresses.
    """
    if item_size < 1:
        raise ValueError("item_size must be >= 1")
    if workers is None:
        workers = max(1, (os.cpu_count() or 2) - 2)

    if regions is None:
        h = _k32.OpenProcess(_PROCESS_VM_READ | _PROCESS_QUERY_INFORMATION, False, pid)
        if not h:
            raise OSError("OpenProcess(%d) failed (err %d)" % (pid, C.get_last_error()))
        try:
            regions = enumerate_regions(h, lo, hi)
        finally:
            _k32.CloseHandle(h)

    shards = shard_regions(regions, workers)
    if not shards:
        return []

    # 1 shard -> run inline (no process overhead; also the single-thread path).
    if len(shards) == 1:
        return sorted(set(_worker(pid, shards[0], matcher, matcher_args, item_size)))

    import multiprocessing as mp
    ctx = mp.get_context("spawn")
    procs_args = [(pid, sh, matcher, matcher_args, item_size) for sh in shards]
    with ctx.Pool(processes=len(shards)) as pool:
        results = pool.starmap(_worker, procs_args)

    merged = set()
    for r in results:
        merged.update(r)
    return sorted(merged)


def single_scan(pid, lo, hi, matcher, matcher_args=(), item_size=8, regions=None):
    """Single-process reference scan (one handle, all regions). Same hit set as
    parallel_scan; used by the benchmark to prove correctness."""
    if regions is None:
        h = _k32.OpenProcess(_PROCESS_VM_READ | _PROCESS_QUERY_INFORMATION, False, pid)
        if not h:
            raise OSError("OpenProcess(%d) failed (err %d)" % (pid, C.get_last_error()))
        try:
            regions = enumerate_regions(h, lo, hi)
        finally:
            _k32.CloseHandle(h)
    return sorted(set(_worker(pid, list(regions), matcher, matcher_args, item_size)))


# ==========================================================================
# Vectorized matchers (TOP-LEVEL so spawn can import them) + their builders
# ==========================================================================
# (a) aligned-qword == target value  --- the vtable / pointer-referrer scan.
#
# ALIGNMENT NOTE (Codex review #6/#8): this matches qwords on ABSOLUTE 8-byte
# boundaries, not chunk-relative ones. A chunk does NOT start at an 8-aligned
# address (the previous window advanced by CHUNK - (item_size-1), e.g. CHUNK-7),
# so we must compute the phase = (-base) % 8 to find the first absolute-aligned
# qword inside this blob. This makes the chunked result identical to scanning the
# whole region in one buffer. It deliberately does NOT find UNALIGNED 8-byte
# matches the way blob.find(struct.pack('<Q',target)) would -- vtable/pointer
# slots are always 8-aligned, so for pointer scans the sets coincide, but if you
# need unaligned 8-byte literal matches use make_bytes_find_matcher instead.
def _m_qword_eq(base, blob, target):
    """Every ABSOLUTE-8-aligned qword in blob equal to `target` (vectorized)."""
    phase = (-base) & 7                       # bytes to first 8-aligned addr
    n = (len(blob) - phase) & ~7              # whole aligned qwords after phase
    if n <= 0:
        return []
    arr = np.frombuffer(blob, dtype="<u8", count=n >> 3, offset=phase)
    idx = np.nonzero(arr == np.uint64(target))[0]
    if idx.size == 0:
        return []
    return (base + phase + (idx.astype(np.uint64) << np.uint64(3))).tolist()


def make_qword_eq_matcher(target):
    """Builder for the aligned-qword==target scan (vtables, pointer referrers).
    Returns kwargs for parallel_scan: matcher/matcher_args/item_size."""
    return {"matcher": ("memory_reader.fastscan", "_m_qword_eq"),
            "matcher_args": (int(target),), "item_size": 8}


# (b) AoS fixed-stride field partition: at each `stride`-aligned slot, test a
#     field at byte offset `foff` of size `fsize` (1/2/4/8) against `value`.
_AOS_DT = {1: "<u1", 2: "<u2", 4: "<u4", 8: "<u8"}


def _m_aos_field_eq(base, blob, stride, foff, fsize, value, array_base):
    """Hits = slot base addresses where the field == value, for slots phased to
    `array_base` (slot 0 sits at array_base, array_base+stride, ...).

    PHASE-AWARE (Codex review #8): a chunk does NOT start on a stride multiple
    (windows advance by CHUNK-(stride-1)), so reshaping from blob[0] would desync
    slots across chunk boundaries. We compute the first slot boundary at-or-after
    this blob's `base`: phase = (array_base - base) mod stride, then reshape from
    there. With item_size=stride window overlap, every slot is fully contained in
    at least one chunk and surfaces exactly once (parent dedups via the set).

    `array_base` should be the AoS array's start address (any address congruent to
    a real slot boundary mod `stride` works). If you don't know it, pass the region
    base and ensure the array begins there."""
    if fsize not in _AOS_DT:
        raise ValueError("fsize must be 1,2,4 or 8")
    if foff + fsize > stride:
        raise ValueError("field [%d,%d) exceeds stride %d" % (foff, foff + fsize, stride))
    phase = (int(array_base) - int(base)) % stride      # offset of first slot in blob
    avail = len(blob) - phase
    if avail < stride:
        return []
    nslots = avail // stride
    buf = np.frombuffer(blob, dtype=np.uint8, count=nslots * stride, offset=phase)
    rows = buf.reshape(nslots, stride)
    field = np.ascontiguousarray(rows[:, foff:foff + fsize])
    vals = field.view(_AOS_DT[fsize]).reshape(nslots)
    idx = np.nonzero(vals == np.array(value, dtype=_AOS_DT[fsize]))[0]
    if idx.size == 0:
        return []
    return (base + phase + idx.astype(np.uint64) * np.uint64(stride)).tolist()


def make_aos_field_matcher(stride, foff, fsize, value, array_base):
    """Builder for the AoS fixed-stride field-equality scan.

    stride/foff/fsize/value : record stride, field byte offset, field size (1/2/4/8),
                              and the value to match.
    array_base              : an address on a real slot boundary (the array start).
                              Slots are phased to this, so the scan is correct even
                              though chunk boundaries don't land on stride multiples.
    Use item_size=stride (set for you) so window overlap covers a slot straddling
    two chunks."""
    return {"matcher": ("memory_reader.fastscan", "_m_aos_field_eq"),
            "matcher_args": (int(stride), int(foff), int(fsize), int(value), int(array_base)),
            "item_size": int(stride)}


# (c) byte-pattern find (overlapping, all occurrences). Vectorized first-byte
#     prefilter + broadcast compare over candidate windows.
def _m_bytes_find(base, blob, pattern):
    """All (possibly overlapping) occurrences of literal `pattern` (bytes) in blob.
    Vectorized: find first-byte candidate offsets, then broadcast-compare the full
    pattern across all candidates at once (no per-candidate Python loop)."""
    if isinstance(pattern, str):
        pattern = bytes.fromhex(pattern.replace(" ", ""))
    plen = len(pattern)
    if plen == 0 or len(blob) < plen:
        return []
    buf = np.frombuffer(blob, dtype=np.uint8)
    pat = np.frombuffer(pattern, dtype=np.uint8)
    last = len(buf) - plen + 1
    cand = np.nonzero(buf[:last] == pat[0])[0]
    if cand.size == 0:
        return []
    if plen > 1:
        # windows[i, j] = buf[cand[i] + j]; compare against pat broadcast
        windows = buf[cand[:, None] + np.arange(plen)[None, :]]
        keep = np.all(windows == pat[None, :], axis=1)
        cand = cand[keep]
        if cand.size == 0:
            return []
    return (base + cand.astype(np.uint64)).tolist()


def make_bytes_find_matcher(pattern):
    """Builder for a literal byte-pattern find. `pattern` is bytes or a hex string
    ('48 8B 05' or '488b05'). item_size = pattern length so window overlap covers a
    match straddling two chunks."""
    if isinstance(pattern, str):
        pat = bytes.fromhex(pattern.replace(" ", ""))
    else:
        pat = bytes(pattern)
    if not pat:
        raise ValueError("empty pattern")
    return {"matcher": ("memory_reader.fastscan", "_m_bytes_find"),
            "matcher_args": (pat,), "item_size": len(pat)}


# ==========================================================================
# CLI smoke test (ASCII-only): parallel vs single qword-eq over the arena.
# ==========================================================================
def _selftest():
    import time
    pid = _wm.find_pid("hoi4.exe")
    if pid is None:
        print("hoi4.exe not running; nothing to scan")
        return
    # Use the live combat-side vtable as the qword target (a known dense scan).
    p = _wm.Process(pid, "hoi4.exe")
    SIDE_VT = 0x2980AC0
    vt_abs = p.module_base + SIDE_VT
    p.close()
    spec = make_qword_eq_matcher(vt_abs)

    # Discover the arena LIVE — the ARENA_LO/HI hint goes stale after an ASLR
    # crash-reload and would make this selftest scan MEM_FREE space (a false 0==0).
    arena = discover_arena(pid)
    if arena is None:
        print("could not discover heap arena; nothing to scan")
        return
    alo, ahi = arena
    print("arena (live-discovered): 0x%X..0x%X  (%.1f GB)" % (alo, ahi, (ahi - alo) / float(1 << 30)))

    t0 = time.time()
    single = single_scan(pid, alo, ahi, **spec)
    ts = time.time() - t0

    t0 = time.time()
    par = parallel_scan(pid, alo, ahi, workers=max(1, (os.cpu_count() or 2) - 2), **spec)
    tp = time.time() - t0

    print("single hits=%d  %.1fs" % (len(single), ts))
    print("parallel hits=%d  %.1fs  speedup=%.2fx" % (len(par), tp, ts / tp if tp else 0.0))
    print("hit sets identical: %s" % (single == par))
    if not single:
        print("WARNING: 0 hits — vtable absent or arena mis-discovered; NOT a meaningful pass")


if __name__ == "__main__":
    _selftest()
