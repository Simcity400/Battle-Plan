"""Ad-hoc RTTI vtable rediscovery for the naval-combat lane on build 870ebf.

The active_fixture_proof scanner hunts CNavalCombat/side/registry/member/CShip
bodies by hard-coded vtable RVAs (candidate_route in poll_*.json). Those RVAs
predate 870ebf and find zero bodies during a live battle. This reuses the shared
RTTI complete-object-locator walk (market_trade_live_probe.derive_rtti_vtables)
to recover the current vtables by class NAME, so the scanner constants can be
refreshed with live provenance. Read-only; safe to run against the live game.

Usage:
  PYTHONPATH=extractor/memory_reader python <this> --pid <pid> --out <json>
"""
import argparse
import json
import struct
import sys

import winmem
from market_trade_live_probe import read_module_chunks, derive_rtti_vtables

PROC_NAME = "hoi4.exe"
EXPECTED_SHA = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"

# Decorated MSVC RTTI type-descriptor names (".?AV<class>@@").
CLASS_NAMES = [
    b".?AVCNavalCombat@@",
    b".?AVCNavalCombatant@@",
    b".?AVCFEXMember@@",
    b".?AVCNavalCombatSide@@",
    b".?AVCFleet@@",
    b".?AVCTaskForce@@",
    b".?AVCShip@@",
]

HEAP_LO = 0x0000_0100_0000_0000
HEAP_HI = 0x0000_7000_0000_0000


def count_live_bodies(p, vtable, limit=64):
    """Count heap objects whose first qword == vtable (live instances)."""
    pattern = struct.pack("<Q", vtable)
    hits = []
    for base, size in p.iter_regions(HEAP_LO, HEAP_HI, want_exec=False):
        blob = p.try_read(base, size)
        if not blob:
            continue
        pos = 0
        while True:
            idx = blob.find(pattern, pos)
            if idx < 0:
                break
            hits.append(base + idx)
            if len(hits) >= limit:
                return hits
            pos = idx + 8
    return hits


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--pid", type=int, required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--count-bodies", action="store_true",
                    help="also heap-scan for live instances of each rediscovered vtable")
    args = ap.parse_args()

    with winmem.Process(args.pid, PROC_NAME) as p:
        module_base = p.module_base
        fp = p.fingerprint() if hasattr(p, "fingerprint") else {}
        sha = fp.get("sha256") if isinstance(fp, dict) else None
        chunks = read_module_chunks(p)

        out = {
            "pid": args.pid,
            "module_base": hex(module_base),
            "module_sha256": sha,
            "sha_matches_870ebf": (sha == EXPECTED_SHA) if sha else None,
            "classes": {},
        }
        for name in CLASS_NAMES:
            key = name.decode()
            try:
                string_addrs, col_hits, vtables = derive_rtti_vtables(p, chunks, name)
                entry = {
                    "string_hits": len(string_addrs),
                    "col_hits": len(col_hits),
                    "vtables": [hex(v) for v in vtables],
                    "vtable_rvas": [hex(v - module_base) for v in vtables],
                }
                if args.count_bodies and vtables:
                    entry["live_body_counts"] = {
                        hex(v - module_base): len(count_live_bodies(p, v))
                        for v in vtables
                    }
                out["classes"][key] = entry
            except Exception as exc:  # noqa: BLE001
                out["classes"][key] = {"error": repr(exc)}

    with open(args.out, "w", encoding="utf-8") as fh:
        json.dump(out, fh, indent=1)
    print(json.dumps({k: (v.get("vtable_rvas"), v.get("live_body_counts"))
                      if isinstance(v, dict) else v
                      for k, v in out["classes"].items()}, indent=1))
    print("wrote", args.out)


if __name__ == "__main__":
    sys.exit(main())
