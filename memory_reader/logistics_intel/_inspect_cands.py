"""_inspect_cands.py - inspect the neighborhood candidates: are the 5 anchors at
state-id-consistent offsets (a real per-state supply array), or just round-value
co-location (alias)? READ-ONLY."""
import os
import struct
import sys
import numpy as np

_HERE = os.path.dirname(os.path.abspath(__file__))
_MR = os.path.dirname(_HERE)
for _p in (_MR, _HERE):
    if _p not in sys.path:
        sys.path.insert(0, _p)
import winmem  # noqa

ANCHORS = {42: 2.80, 53: 2.90, 157: 1.40, 219: 6.60, 237: 0.60}
TOL = 0.02
# windows flagged with all-4-others present (sample from the dense 0x24972 cluster + others)
FULL_RECORD = [0x2494cbc8b0c, 0x2497245df0c, 0x249724cbd34, 0x24972513a24,
               0x24972706708, 0x249729764a4]
OBJ_PTR = [0x24a4b22da80]


def dump_window(p, center, half=0x800):
    lo = center - half
    blob = p.try_read(lo, 2 * half)
    if not blob:
        return None
    arrf = np.frombuffer(blob[:len(blob) & ~3], dtype="<f4")
    # find positions (byte offsets from lo) of every anchor value
    positions = {}
    for sid, val in ANCHORS.items():
        with np.errstate(invalid="ignore"):
            idx = np.nonzero(np.abs(arrf - np.float32(val)) <= TOL)[0]
        positions[sid] = [lo + int(i) * 4 for i in idx]
    return lo, positions


def main():
    p = winmem.open_process("hoi4.exe")
    print("=== FULL-RECORD candidates (all 5 anchors co-located) ===")
    for c in FULL_RECORD:
        r = dump_window(p, c)
        if not r:
            print("0x%X unreadable" % c)
            continue
        lo, pos = r
        print("\ncenter 0x%X" % c)
        # report the addresses of each anchor and the deltas between them
        # take the FIRST occurrence of each anchor in the window
        firsts = {sid: (addrs[0] if addrs else None) for sid, addrs in pos.items()}
        for sid in sorted(ANCHORS):
            addrs = pos[sid]
            print("  s%-4d (%.2f): %d hit(s) %s" %
                  (sid, ANCHORS[sid], len(addrs), [hex(a) for a in addrs[:6]]))
        # Are they a tight contiguous block? Sort all anchor addrs, show stride pattern.
        allpts = sorted((a, sid) for sid, addrs in pos.items() for a in addrs)
        if len(allpts) >= 5:
            print("  sorted anchor stream (addr, sid):")
            prev = None
            for a, sid in allpts[:14]:
                d = "" if prev is None else "(+0x%X)" % (a - prev)
                print("     0x%X s%-4d %s" % (a, sid, d))
                prev = a

    print("\n\n=== OBJ-PTR candidate (region of pointers to state-219 obj) ===")
    for c in OBJ_PTR:
        lo = c - 0x40
        blob = p.try_read(lo, 0x120)
        if not blob:
            continue
        print("center 0x%X qword dump:" % c)
        for off in range(0, len(blob) - 8, 8):
            q = struct.unpack_from("<Q", blob, off)[0]
            mark = ""
            f = struct.unpack_from("<f", blob, off)[0]
            print("    0x%X: 0x%016X  (as 2xf32: %.3f %.3f) %s" %
                  (lo + off, q, struct.unpack_from("<f", blob, off)[0],
                   struct.unpack_from("<f", blob, off + 4)[0], mark))
    p.close()


if __name__ == "__main__":
    main()
