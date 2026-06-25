"""_decode_perprov.py - precise field decode of SWeatherPerProvince across all 5
anchor provinces, both qword and per-4-byte u32/i32/f32, to map the phenomenon/
intensity/accumulator fields. Includes a RAIN-DIFF marker.
READ-ONLY.
"""
import os
import sys
import struct

import numpy as np

_HERE = os.path.dirname(os.path.abspath(__file__))
_MR = os.path.dirname(_HERE)
for _p in (_MR, _HERE):
    if _p not in sys.path:
        sys.path.insert(0, _p)
import winmem as wm
import fastscan as fs

PERPROV_RVA = 0x2950A50
PID_OFF = 0x08
BODY = 0x80
# province -> (region, temp_C, is_rain)
ANCHOR_PROV = {7206: 16.0, 3946: 8.0, 6945: 11.0, 7250: 26.0, 10167: 24.0}
PROV_RAIN = 7250
PROV_CLEAR = [7206, 3946, 6945, 10167]
ORDER = [3946, 7250, 6945, 10167, 7206]  # 8C, 26C-RAIN, 11C, 24C, 16C
HEAP_LO, HEAP_HI = 0x10000000000, 0x7FF000000000


if __name__ == "__main__":
    p = wm.open_process("hoi4.exe")
    mb = p.module_base; msize = p.module_size; pid = p.pid
    p.close()
    arena = fs.discover_arena(pid)
    spec = fs.make_qword_eq_matcher(mb + PERPROV_RVA)
    hits = fs.parallel_scan(pid, arena[0], arena[1], workers=6, **spec)
    p = wm.Process(pid, "hoi4.exe")
    prov2 = {}
    try:
        for h in hits:
            b = p.try_read(h, BODY)
            if len(b) >= 0x20:
                pv = int(np.frombuffer(b[PID_OFF:PID_OFF + 2], "<u2")[0])
                if pv in ANCHOR_PROV and pv not in prov2:
                    prov2[pv] = (h, b)
        print("module_base 0x%X" % mb)
        hdr = "%-6s |" % "off"
        for pp in ORDER:
            lbl = "p%d(%dC%s)" % (pp, ANCHOR_PROV[pp], "RAIN" if pp == PROV_RAIN else "")
            hdr += " %-20s |" % lbl
        print(hdr)
        print("-" * len(hdr))
        for off in range(0, BODY - 4, 4):
            row = "+0x%03X |" % off
            for pp in ORDER:
                b = prov2[pp][1]
                u = struct.unpack("<I", b[off:off + 4])[0]
                f = struct.unpack("<f", b[off:off + 4])[0]
                if 0 < abs(f) < 1e5 and not (0x30000000 < u < 0xF0000000):
                    cell = "u=%d f=%.3g" % (u, f)
                else:
                    cell = "u=%d" % u
                row += " %-20s |" % cell
            # rain-diff marker
            cu = [struct.unpack("<I", prov2[pp][1][off:off+4])[0] for pp in PROV_CLEAR]
            ru = struct.unpack("<I", prov2[PROV_RAIN][1][off:off+4])[0]
            if len(set(cu)) == 1 and ru != cu[0]:
                row += "  <== RAIN-DIFF clr=%d rain=%d" % (cu[0], ru)
            print(row)

        # follow heap ptr at +0x10 for rain & clear to compare
        print("\n=== follow +0x10 heap ptr ===")
        for pp in (7250, 3946):
            b = prov2[pp][1]
            q = struct.unpack("<Q", b[0x10:0x18])[0]
            if HEAP_LO <= q < HEAP_HI:
                sub = p.try_read(q, 0x40)
                print("  p%d +0x10 -> 0x%X : %s" % (pp, q, sub.hex()))
                print("       u32:", np.frombuffer(sub[:0x40], "<u4").tolist())
    finally:
        p.close()
