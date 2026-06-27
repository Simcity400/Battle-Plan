"""test_scan_advance.py - regression test for the heap-scan advance fix.

Bug: `pos += max(1, len(blob) - OV)` collapses to a 1-byte advance whenever a chunk
read returns < OV bytes (short/partial/final read), re-reading + re-allocating the
same multi-MB window millions of times -> hard crash on the 7.4 GB fragmented
hoi4.exe. Fix: keep the OV overlap only on a FULL mid-region chunk; a final or
short read advances by the bytes read, floored to a page.

This exercises (a) the shared advance formula used by locate.py / winmem.py /
country_index_bridge.py and (b) fastscan._Reader.scan_region directly, asserting
every read profile TERMINATES with a BOUNDED iteration count (the bug would loop
~region_size times). No live process needed.
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import fastscan  # noqa: E402

PAGE = 0x1000


def _advance_locate_style(blen, want, pos, size, OV):
    """Mirror of the FIXED locate/winmem/bridge advance (one source of truth here)."""
    if blen:                                   # non-empty blob
        if pos + blen >= size or blen < want:  # final chunk OR short/partial read
            return pos + max(blen, PAGE)
        return pos + (blen - OV)               # full mid-region chunk
    return pos + max(want, PAGE)               # fully unreadable window


def _walk(size, read_len, OV, CH):
    pos = iters = 0
    while pos < size:
        iters += 1
        assert iters < 5_000_000, "STALL: advance is not making forward progress"
        want = min(CH, size - pos)
        blen = read_len(iters, want)
        newpos = _advance_locate_style(blen, want, pos, size, OV)
        assert newpos > pos, "pos must strictly increase (was %d -> %d)" % (pos, newpos)
        pos = newpos
    return iters


def test_locate_style_advance():
    OV, CH = 4032, 16 << 20                     # locate()'s real OV; large chunk
    # 1) normal full reads -> ~ size/CH iterations
    assert _walk(3 * CH + 500, lambda i, n: n, OV, CH) <= 6
    # 2) PATHOLOGICAL: every read returns < OV bytes (the exact bug trigger).
    #    Bugged code would do ~size/1 iters; fix floors each advance to >= PAGE.
    it = _walk(50 * PAGE, lambda i, n: 10, OV, CH)
    assert it <= 52, "short-read advance must floor to PAGE; got %d" % it
    # 3) tiny final chunk (< OV) right after a full chunk -> no tail crawl
    assert _walk(CH + 100, lambda i, n: n, OV, CH) <= 3
    # 4) fully unreadable windows (RPM returns nothing)
    assert _walk(10 * PAGE, lambda i, n: 0, OV, CH) <= 11
    # 5) partial copy: a mid-region read truncates to 5 bytes once, then full
    assert _walk(3 * CH, lambda i, n: 5 if i == 2 else n, OV, CH) <= 6
    print("locate/winmem/bridge advance formula: bounded + strictly-increasing. OK")


def test_fastscan_scan_region():
    """Drive the REAL fastscan._Reader.scan_region with a mock reader returning
    short reads; small _CHUNK so the bounds are tight and fast to assert."""
    orig_chunk = fastscan._CHUNK
    fastscan._CHUNK = 0x1000
    try:
        class Mock(object):
            def __init__(self, prof):
                self.prof = prof
                self.calls = 0

            def rpm(self, addr, n):
                self.calls += 1
                return b"\x00" * self.prof(self.calls, n)

            scan_region = fastscan._Reader.scan_region

        def noop(base, blob):
            return []

        def run(size, prof, item_size=384):
            m = Mock(prof)
            m.scan_region(0x10000, size, noop, item_size)   # base arbitrary
            return m.calls

        CH = fastscan._CHUNK
        # full reads -> ~ size/CH
        assert run(3 * CH + 50, lambda c, n: n) <= 6
        # PATHOLOGICAL short reads of 50 bytes (< overlap=383): must NOT be ~size/1
        it = run(40 * CH, lambda c, n: 50)
        assert it < 40 * CH // 100, "short reads not floored: %d iterations" % it
        # tiny final chunk (full read of a sub-overlap tail) -> no crawl
        assert run(2 * CH + 7, lambda c, n: n) <= 5
        # fully unreadable windows
        assert run(10 * CH, lambda c, n: 0) <= 12
        print("fastscan._Reader.scan_region advance: bounded + terminates. OK")
    finally:
        fastscan._CHUNK = orig_chunk


if __name__ == "__main__":
    test_locate_style_advance()
    test_fastscan_scan_region()
    print("\nALL SCAN-ADVANCE TESTS PASSED")
