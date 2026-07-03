"""Raw fixture preservation for the live naval-combat chain on 870ebf.

The active_fixture_proof scan now finds CNavalCombat bodies (vtable 0x29C6F18,
6 live) but the body->side->registry->member offset chain is stale, so nothing
validates. This dumps each live body's raw memory plus every heap-pointer field
it contains (resolved one hop, tagged with any recognized vtable) so the chain
can be reversed offline after the battle ends. Read-only.

Usage: PYTHONPATH=extractor/memory_reader python <this> --pid <pid> --out <json>
"""
import argparse
import json
import struct
import sys

import winmem

PROC_NAME = "hoi4.exe"
BODY_RVA = 0x29C6F18
DUMP_LEN = 0x600
HEAP_LO = 0x0000_0100_0000_0000
HEAP_HI = 0x0000_7000_0000_0000

# Rediscovered 870ebf vtables (rtti_vtable_rediscover.py, 2026-07-03) to tag
# pointer fields with the class they point at.
KNOWN_VTABLES = {
    0x29C6F18: "CNavalCombat",
    0x29C6FF0: "CNavalCombat.alt",
    0x29CD298: "CNavalCombatant",
    0x2A0A228: "CFEXMember",
    0x2941748: "CShip",
    0x29417B0: "CShip.alt",
    0x294BF28: "CFleet",
    0x294BF80: "CFleet.alt",
    0x294C480: "CTaskForce",
    0x294C660: "CTaskForce.b",
    0x294C6D0: "CTaskForce.c",
    0x294C718: "CTaskForce.d",
}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--pid", type=int, required=True)
    ap.add_argument("--out", required=True)
    args = ap.parse_args()

    with winmem.Process(args.pid, PROC_NAME) as p:
        base = p.module_base
        body_vtable = base + BODY_RVA
        pattern = struct.pack("<Q", body_vtable)

        bodies = []
        for rlo, rsize in p.iter_regions(HEAP_LO, HEAP_HI, want_exec=False):
            blob = p.try_read(rlo, rsize)
            if not blob:
                continue
            pos = 0
            while True:
                idx = blob.find(pattern, pos)
                if idx < 0:
                    break
                bodies.append(rlo + idx)
                pos = idx + 8

        def classify_ptr(ptr):
            if not (HEAP_LO <= ptr < HEAP_HI):
                return None
            vt = p.try_read(ptr, 8)
            if not vt or len(vt) < 8:
                return {"ptr": hex(ptr), "vtable": None}
            vtv = struct.unpack("<Q", vt)[0]
            rva = vtv - base if base <= vtv < base + 0x0800_0000 else None
            return {
                "ptr": hex(ptr),
                "vtable_rva": hex(rva) if rva is not None else None,
                "class": KNOWN_VTABLES.get(rva) if rva is not None else None,
            }

        dumped = []
        for addr in bodies:
            raw = p.try_read(addr, DUMP_LEN) or b""
            # Every 8-byte-aligned qword that looks like a heap pointer, resolved one hop.
            ptr_fields = {}
            for off in range(0, len(raw) - 8, 8):
                qv = struct.unpack_from("<Q", raw, off)[0]
                if HEAP_LO <= qv < HEAP_HI:
                    info = classify_ptr(qv)
                    if info:
                        ptr_fields[hex(off)] = info
            dumped.append({
                "body_addr": hex(addr),
                "raw_hex": raw.hex(),
                "heap_ptr_fields": ptr_fields,
            })

        out = {
            "pid": args.pid,
            "module_base": hex(base),
            "body_vtable_rva": hex(BODY_RVA),
            "body_count": len(bodies),
            "dump_len": DUMP_LEN,
            "bodies": dumped,
        }

    with open(args.out, "w", encoding="utf-8") as fh:
        json.dump(out, fh, indent=1)
    # Console summary: which known classes each body points at.
    for b in dumped:
        classes = sorted({v["class"] for v in b["heap_ptr_fields"].values()
                          if v.get("class")})
        print(b["body_addr"], "->", classes)
    print("body_count=", len(bodies), "wrote", args.out)


if __name__ == "__main__":
    sys.exit(main())
