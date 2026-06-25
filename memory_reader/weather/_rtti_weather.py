"""_rtti_weather.py - RTTI sweep of hoi4.exe for weather/climate class names.

MSVC RTTI layout (x64):
  vtable[-1] -> CompleteObjectLocator (COL)
  COL.signature (u32 @ +0)          == 1 for x64
  COL.pTypeDescriptor (u32 @ +0x0C) = RVA of TypeDescriptor (relative to image base)
  TypeDescriptor: { void* pVFTable; void* spare; char name[]; }  name @ +0x10
      name is the decorated/mangled type name, e.g. ".?AVCWeatherManager@@"

Strategy (find class names directly, then locate their vtables):
  1. Read the whole module image from memory (committed). Find every occurrence of
     the byte pattern ".?AV" / ".?AU" (class/struct type-descriptor name prefix)
     whose following ASCII contains 'Weather'|'Climate'|'Precip'|'Wind'|'Storm'.
  2. For each such TypeDescriptor (name starts 0x10 into the descriptor), compute
     the descriptor base = name_addr - 0x10, then its RVA.
  3. Find the COL that references it: scan .rdata for a u32 == descriptor RVA at
     COL+0x0C, validate COL signature/structure, then find vtable = COL_addr_ref
     (a pointer to COL sits at vtable-8). We locate the vtable by scanning for an
     aligned pointer (image-relative) to the COL, since vtable[-1] -> COL.
  4. Report (class name, typedescriptor addr, COL addr, vtable addr).

READ-ONLY.
"""
import os
import re
import sys
import time

import numpy as np

_HERE = os.path.dirname(os.path.abspath(__file__))
_MR = os.path.dirname(_HERE)
for _p in (_MR, _HERE):
    if _p not in sys.path:
        sys.path.insert(0, _p)
import winmem as wm

KEYWORDS = (b"Weather", b"Climate", b"Precip", b"Wind", b"Storm", b"Snow",
            b"Temperat", b"Rain", b"Blizzard", b"Sandstorm")


def read_module_image(p):
    """Read all committed readable regions of the main module into a dict
    {base: bytes}. Returns (segments, lo, hi)."""
    base = p.module_base
    end = base + p.module_size
    segs = []
    for rbase, rsize in p.iter_regions(base, end, want_exec=None):
        blob = p.try_read(rbase, rsize)
        if blob:
            segs.append((rbase, blob))
    return segs


def find_type_descriptors(segs):
    """Find every '.?AV'/'.?AU' type-descriptor name in the image whose name
    contains one of KEYWORDS. Returns list of (name_addr, name_bytes)."""
    out = []
    pat = re.compile(rb"\.\?A[VU][\x20-\x7e]{0,200}?@@")
    for base, blob in segs:
        for m in pat.finditer(blob):
            name = m.group(0)
            if any(k in name for k in KEYWORDS):
                out.append((base + m.start(), name))
    return out


def find_all_type_descriptors(segs):
    """Find ALL type-descriptor names (for context / counting)."""
    out = []
    pat = re.compile(rb"\.\?A[VU][\x20-\x7e]{0,200}?@@")
    for base, blob in segs:
        for m in pat.finditer(blob):
            out.append((base + m.start(), m.group(0)))
    return out


def locate_vtables_for_descriptor(p, segs, name_addr, image_base):
    """Given a TypeDescriptor name address, the descriptor base is name_addr-0x10.
    Compute its RVA. Find COL(s) whose pTypeDescriptor (u32 @ +0x0C) == RVA. Then
    find vtable(s): an aligned pointer to the COL (vtable-8 holds &COL)."""
    desc_base = name_addr - 0x10
    desc_rva = desc_base - image_base
    results = []
    # Scan all segments for a u32 == desc_rva that sits at COL+0x0C.
    target = np.uint32(desc_rva & 0xFFFFFFFF)
    col_candidates = []
    for base, blob in segs:
        if len(blob) < 4:
            continue
        arr = np.frombuffer(blob, dtype="<u4")
        idx = np.nonzero(arr == target)[0]
        for i in idx:
            u32_addr = base + int(i) * 4
            col_addr = u32_addr - 0x0C  # COL +0x0C is pTypeDescriptor
            # validate COL: signature u32 @ +0 should be 1 (x64), and
            # COL+0x14 (pSelf RVA in x64) should equal col_addr - image_base
            try:
                sig = p.u32(col_addr)
            except OSError:
                continue
            if sig != 1:
                continue
            col_candidates.append(col_addr)
    # For each COL, find vtables: aligned ptr == col_addr, vtable = ptr_addr + 8
    for col_addr in col_candidates:
        col_ptr_target = np.uint64(col_addr)
        for base, blob in segs:
            n = len(blob) & ~7
            if n < 8:
                continue
            arr = np.frombuffer(blob, dtype="<u8", count=n >> 3)
            idx = np.nonzero(arr == col_ptr_target)[0]
            for i in idx:
                ptr_addr = base + int(i) * 8
                vtable = ptr_addr + 8
                results.append((desc_base, col_addr, vtable))
    return results


if __name__ == "__main__":
    p = wm.open_process("hoi4.exe")
    if p is None:
        raise SystemExit("hoi4.exe not running")
    with p:
        image_base = p.module_base
        print("module_base 0x%X size 0x%X" % (image_base, p.module_size))
        t0 = time.time()
        segs = read_module_image(p)
        print("read %d module segments in %.1fs (%.1f MB)"
              % (len(segs), time.time() - t0, sum(len(b) for _, b in segs) / 1e6))
        descs = find_type_descriptors(segs)
        print("\n=== Weather/Climate-keyworded TypeDescriptors: %d ===" % len(descs))
        seen = set()
        for (addr, name) in descs:
            nm = name.decode("latin-1")
            if nm in seen:
                continue
            seen.add(nm)
            print("  0x%X  %s" % (addr, nm))
        # Now resolve vtables for each unique descriptor
        print("\n=== vtables (RTTI-resolved) ===")
        results = {}
        for (addr, name) in descs:
            nm = name.decode("latin-1")
            vts = locate_vtables_for_descriptor(p, segs, addr, image_base)
            for (db, col, vt) in vts:
                key = (nm, vt)
                if key in results:
                    continue
                results[key] = (db, col, vt)
                print("  %-50s vtable 0x%X (RVA 0x%X)  COL 0x%X" % (nm, vt, vt - image_base, col))
        # save
        import json
        out = [{"name": k[0], "vtable": "0x%X" % k[1], "vtable_rva": "0x%X" % (k[1] - image_base),
                "col": "0x%X" % v[1], "descriptor": "0x%X" % v[0]} for k, v in results.items()]
        json.dump(out, open(os.path.join(_HERE, "_rtti_weather.json"), "w"), indent=2)
        print("\nsaved _rtti_weather.json (%d vtables)" % len(out))
