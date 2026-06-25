"""_decode_struct.py - field-by-field decode of SWeatherPerRegion across all 5
anchor regions, side by side, to map phenomenon/intensity/temp/climatology-input
fields. Prints each 4-byte slot decoded as u32/i32/f32 for all 5 anchors.
READ-ONLY.
"""
import os
import sys
import time
import struct

import numpy as np

_HERE = os.path.dirname(os.path.abspath(__file__))
_MR = os.path.dirname(_HERE)
for _p in (_MR, _HERE):
    if _p not in sys.path:
        sys.path.insert(0, _p)
import winmem as wm
import fastscan as fs

PERREGION_RVA = 0x2950AA0
BODY = 0xA0
RID_OFF = 0x08
ANCHOR_RIDS = [20, 31, 129, 160, 292]
EXPECT_T = {20: 8, 31: 26, 129: 11, 160: 24, 292: 16}
EXPECT_RAIN = {20: False, 31: True, 129: False, 160: False, 292: False}


def f32(b):
    return struct.unpack("<f", b)[0]


if __name__ == "__main__":
    p = wm.open_process("hoi4.exe")
    mb = p.module_base
    pid = p.pid
    p.close()
    arena = fs.discover_arena(pid)
    spec = fs.make_qword_eq_matcher(mb + PERREGION_RVA)
    hits = fs.parallel_scan(pid, arena[0], arena[1], workers=6, **spec)
    p = wm.Process(pid, "hoi4.exe")
    id2 = {}
    try:
        for h in hits:
            b = p.try_read(h, BODY)
            if len(b) >= 0x10:
                rid = int(np.frombuffer(b[RID_OFF:RID_OFF + 4], "<u4")[0])
                if rid in ANCHOR_RIDS and rid not in id2:
                    id2[rid] = (h, b)
    finally:
        p.close()

    print("module_base 0x%X" % mb)
    print("anchor objects:")
    for r in ANCHOR_RIDS:
        print("  region %3d -> 0x%X  (expect %dC rain=%s)" % (r, id2[r][0], EXPECT_T[r], EXPECT_RAIN[r]))

    print("\n%-6s | %-22s | %-22s | %-22s | %-22s | %-22s" %
          ("off", "r20(8C clr)", "r31(26C RAIN)", "r129(11C clr)", "r160(24C clr)", "r292(16C clr)"))
    print("-" * 130)
    for off in range(0, BODY - 4, 4):
        cells = []
        diff_rain = False
        clearvals = []
        for r in ANCHOR_RIDS:
            b = id2[r][1]
            raw = b[off:off + 4]
            u = struct.unpack("<I", raw)[0]
            f = struct.unpack("<f", raw)[0]
            # show u32 and (if plausible float) f32
            if abs(f) < 1e6 and (f != 0) and not (u > 0x40000000 and u < 0x80000000 and abs(f) > 1e3):
                cell = "u=%d f=%.3g" % (u, f)
            else:
                cell = "u=%d" % u
            cells.append(cell)
        # mark offset where r31 differs from the 4 clears
        c_u = [struct.unpack("<I", id2[r][1][off:off+4])[0] for r in (20,129,160,292)]
        r_u = struct.unpack("<I", id2[31][1][off:off+4])[0]
        mark = ""
        if len(set(c_u)) == 1 and r_u != c_u[0]:
            mark = "  <== RAIN-DIFF (clr=%d rain=%d)" % (c_u[0], r_u)
        print("+0x%03X | %-22s | %-22s | %-22s | %-22s | %-22s%s" %
              (off, cells[0], cells[1], cells[2], cells[3], cells[4], mark))
