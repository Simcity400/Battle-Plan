"""_follow_active.py - correctly follow region-31's active-weather vectors.

region 31 SWeatherPerRegion @ found live. The two populated members:
  +0x58 : begin ptr (qword), then +0x60 holds two u32 (size,?), +0x64 ...
  +0x70 : begin ptr (qword), then +0x78 holds 1,1
These are PdxVector {ptr, u32 size, u32 cap} OR {ptr begin, ptr end}. We'll read
the begin pointer, then dump a generous block and try to find:
  - the +0x70 vector points to per-province weather records that carry a
    PHENOMENON enum + INTENSITY.
We also resolve each pointer in the +0x70 records: if they are SWeatherPerProvince
object pointers, read those provinces' +0x28/+0x2C/+0x38 to compare with clear ones.
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

PERREGION_RVA = 0x2950AA0
PERPROV_RVA = 0x2950A50
HEAP_LO, HEAP_HI = 0x10000000000, 0x7FF000000000


def isheap(v):
    return HEAP_LO <= v < HEAP_HI


if __name__ == "__main__":
    p = wm.open_process("hoi4.exe")
    mb = p.module_base; pid = p.pid
    p.close()
    arena = fs.discover_arena(pid)
    spec = fs.make_qword_eq_matcher(mb + PERREGION_RVA)
    rhits = fs.parallel_scan(pid, arena[0], arena[1], workers=6, **spec)
    perprov_vt = mb + PERPROV_RVA

    p = wm.Process(pid, "hoi4.exe")
    try:
        reg31 = None
        for h in rhits:
            if p.u32(h + 8) == 31:
                reg31 = h; break
        print("region 31 SWeatherPerRegion @ 0x%X" % reg31)
        body = p.try_read(reg31, 0xA0)

        # +0x58 vector
        v58_begin = struct.unpack("<Q", body[0x58:0x60])[0]
        v58_b = struct.unpack("<I", body[0x60:0x64])[0]
        v58_c = struct.unpack("<I", body[0x64:0x68])[0]
        print("\n+0x58 vector: begin=0x%X  size_u32@+0x60=%d  cap_u32@+0x64=%d" % (v58_begin, v58_b, v58_c))
        if isheap(v58_begin):
            sz = v58_b if 0 < v58_b < 10000 else 16
            data = p.try_read(v58_begin, min(sz * 16, 0x200))
            print("  +0x58 pointee (%d bytes) hex: %s" % (len(data), data[:0x80].hex()))
            # decode as 16-byte records: {u32 prov, u32, i32, i32}
            for i in range(min(len(data) // 16, 14)):
                r = data[i*16:(i+1)*16]
                u = struct.unpack("<IIii", r)
                f = struct.unpack("<ffff", r)
                print("    rec[%2d] u32=%s  i32=%s  f32=%s" % (i, u[:2], u[2:], [round(x,3) for x in f]))

        # +0x70 vector
        v70_begin = struct.unpack("<Q", body[0x70:0x78])[0]
        v70_x = struct.unpack("<II", body[0x78:0x80])
        print("\n+0x70 vector: begin=0x%X  +0x78=%s" % (v70_begin, v70_x))
        if isheap(v70_begin):
            data = p.try_read(v70_begin, 0x200)
            print("  +0x70 pointee hex: %s" % data[:0x80].hex())
            # try 16-byte records {qword ptr, u32, u32}
            for i in range(min(len(data)//16, 14)):
                r = data[i*16:(i+1)*16]
                ptr = struct.unpack("<Q", r[:8])[0]
                a, b2 = struct.unpack("<II", r[8:16])
                tag = ""
                if isheap(ptr):
                    # is it a SWeatherPerProvince?
                    vt = p.try_read(ptr, 8)
                    if len(vt) == 8 and struct.unpack("<Q", vt)[0] == perprov_vt:
                        pvid = p.u16(ptr + 8)
                        tag = "-> SWeatherPerProvince prov %d" % pvid
                    else:
                        tag = "-> HEAP (not perprov; vt=0x%X)" % (struct.unpack("<Q", vt)[0] if len(vt)==8 else 0)
                print("    rec[%2d] ptr=0x%X  u32=(%d,%d)  %s" % (i, ptr, a, b2, tag))

        # Now: compare an ACTIVE province (from +0x58 list) full body vs a clear one.
        # The +0x58 list gave province ids; pick the first, find its SWeatherPerProvince.
        print("\n=== compare active vs clear SWeatherPerProvince bodies ===")
        spec2 = fs.make_qword_eq_matcher(perprov_vt)
        phits = fs.parallel_scan(pid, arena[0], arena[1], workers=6, **spec2)
        prov2 = {}
        for h in phits:
            prov2[p.u16(h+8)] = h
        # province 7250 is the rain anchor; 3946 clear
        for label, pv in (("RAIN 7250", 7250), ("clear 3946", 3946)):
            if pv in prov2:
                b = p.try_read(prov2[pv], 0x80)
                print("  %s @ 0x%X: %s" % (label, prov2[pv], b.hex()))
    finally:
        p.close()
