"""_decode_mgr040.py - decode CWeatherManager +0x040 (305-element array) and the
+0x008 vector to find a per-region phenomenon/temperature SoA where region 31
(rain) differs from clears.

+0x040: begin=B, size=305, cap=305. stride = (some total)/305. We don't have the
cap-bytes directly (cap is a count, 305). The element stride: read a big block and
look for the region_id sequence (1,2,3,... or the strategicregion order) to lock
stride. Then map index->region and read phenomenon/temp columns.
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

WMGR_RVA = 0x2950AF0
PERREGION_RVA = 0x2950AA0
HEAP_LO, HEAP_HI = 0x10000000000, 0x7FF000000000

ANCHOR_T = {20: 8, 31: 26, 129: 11, 160: 24, 292: 16}
CLEAR = [20, 129, 160, 292]
RAIN = 31


def isheap(v):
    return HEAP_LO <= v < HEAP_HI


if __name__ == "__main__":
    p = wm.open_process("hoi4.exe")
    mb = p.module_base; pid = p.pid
    p.close()
    arena = fs.discover_arena(pid)
    m = fs.parallel_scan(pid, arena[0], arena[1], workers=6, **fs.make_qword_eq_matcher(mb + WMGR_RVA))[0]

    p = wm.Process(pid, "hoi4.exe")
    try:
        body = p.try_read(m, 0x60)
        b40 = struct.unpack("<Q", body[0x40:0x48])[0]
        sz40 = 305
        # read a generous block; guess stride by where region ids land.
        # Try strides; for each, read element[k] first u32/u16 across k=0..304 and
        # see if they form the region-id permutation 1..304 (or 0..304).
        print("+0x040 begin=0x%X size=305" % b40)
        blocks = p.try_read(b40, 305 * 64)  # up to stride 64
        best = None
        for stride in (8, 12, 16, 20, 24, 28, 32, 40, 48, 56, 64):
            if 305 * stride > len(blocks):
                continue
            # read field at element start as u32
            for foff in (0, 4, 8):
                ids = []
                for k in range(305):
                    off = k * stride + foff
                    v = struct.unpack("<I", blocks[off:off+4])[0]
                    ids.append(v)
                inr = [v for v in ids if 1 <= v <= 304]
                distinct = len(set(inr))
                if distinct >= 250:
                    print("  stride=%d foff=%d -> %d distinct region ids in 1..304 (sample %s)"
                          % (stride, foff, distinct, ids[:8]))
                    if best is None or distinct > best[2]:
                        best = (stride, foff, distinct)
        if best:
            stride, foff, _ = best
            print("\nLOCKED: stride=%d, region_id at elem+%d" % (stride, foff))
            # map region id -> element base
            id2elem = {}
            for k in range(305):
                eb = b40 + k * stride
                rid = struct.unpack("<I", blocks[k*stride+foff:k*stride+foff+4])[0]
                id2elem.setdefault(rid, eb)
            present = [r for r in [20,31,129,160,292] if r in id2elem]
            print("anchors present in +0x040 array:", present)
            if len(present) == 5:
                # phenomenon sweep within element
                ebytes = {r: p.try_read(id2elem[r], stride) for r in [20,31,129,160,292]}
                print("\n=== element hexdump ===")
                for r in [31,20,292]:
                    print("  region %3d (%s): %s" % (r, "RAIN" if r==31 else "clear", ebytes[r].hex()))
                print("\n=== phenomenon sweep (clear equal, rain 31 differs) ===")
                for width, dt in ((1,"<u1"),(2,"<u2"),(4,"<u4")):
                    for o in range(0, stride - width + 1):
                        cv = [struct.unpack(dt, ebytes[r][o:o+width])[0] for r in CLEAR]
                        rv = struct.unpack(dt, ebytes[RAIN][o:o+width])[0]
                        if len(set(cv))==1 and rv != cv[0]:
                            pl = (cv[0]==0 and 1<=rv<=7)
                            print("  +%d w=%d clear=%d rain=%d%s" % (o, width, cv[0], rv, "  <--plausible" if pl else ""))
                print("\n=== temperature sweep (5 anchors) ===")
                encs=[("f32C","<f4",4,lambda a:a),("f32K","<f4",4,lambda a:a-273.15),
                      ("i16x100","<i2",2,lambda a:a/100.),("i16x10","<i2",2,lambda a:a/10.),
                      ("i32x100","<i4",4,lambda a:a/100.),("i32x1000","<i4",4,lambda a:a/1000.)]
                for en,edt,eb_,conv in encs:
                    for o in range(0,stride-eb_+1):
                        ok=True;rd={}
                        for r in [20,31,129,160,292]:
                            v=float(conv(np.frombuffer(ebytes[r][o:o+eb_],edt).astype(np.float64))[0])
                            if not np.isfinite(v) or abs(v-ANCHOR_T[r])>2.0: ok=False;break
                            rd[r]=round(v,2)
                        if ok: print("  +%d %s reads=%s"%(o,en,rd))
        else:
            print("could not lock +0x040 stride via region-id field. Dump first 0x100:")
            print(blocks[:0x100].hex())
    finally:
        p.close()
