"""precip_check.py - find / confirm the PRECIPITATION (weather-type) array as a
second independent signature corroborating a temperature pin (build 16022851).

Anchor precip pattern (region -> precip):
  20  France       clear
  31  India        RAIN   <- the odd one out
  129 Turkey       clear
  160 Philippines  clear
  225 Libya        clear

So the precip array's slots [20,129,160,225] share one "clear" value and slot
[31] differs (a rain enum / nonzero precip intensity). Two uses:

(A) scoped confirm: given a temp candidate's (base, stride, ib), look for a
    parallel precip field at a small offset from the temp field where the 4 clear
    anchors are EQUAL and region 31 differs (and is a plausible small rain enum
    1..2, or simply != clear). This is the strong corroborator.

(B) standalone scan: find any region-indexed u8/u16/u32 array where slots
    [20,129,160,225] are equal and slot[31] differs, with index = rid-ib. (Used
    if temp and precip are separate SoA arrays.)
"""
import os
import sys

import numpy as np

_HERE = os.path.dirname(os.path.abspath(__file__))
_MR = os.path.dirname(_HERE)
for _p in (_MR, _HERE):
    if _p not in sys.path:
        sys.path.insert(0, _p)
import winmem as _wm

# region ids from authoritative mod files (Libya = 292, not the stale climo's 225)
CLEAR_RIDS = [20, 129, 160, 292]
RAIN_RID = 31


def confirm_scoped(p, base_ib_elem, stride, ib, search_bytes=64):
    """Given a temp candidate, sweep field offsets within +-search_bytes for a
    precip field (u8/u16/u32) where the 4 clear anchors are equal and region 31
    differs. base_ib_elem = address of region-`ib` element of the TEMP field.
    Returns list of confirming layouts."""
    # The record base for region rid is (base_ib_elem - temp_field_off) + (rid-ib)*stride
    # but we don't know temp_field_off; treat base_ib_elem as temp field addr and
    # scan precip field at temp_addr + foff for foff in a window.
    out = []
    for width, dt in ((1, "<u1"), (2, "<u2"), (4, "<u4")):
        for foff in range(-search_bytes, search_bytes + 1):
            try:
                clears = []
                for rid in CLEAR_RIDS:
                    a = base_ib_elem + (rid - ib) * stride + foff
                    raw = p.try_read(a, width)
                    if len(raw) != width:
                        raise ValueError
                    clears.append(int(np.frombuffer(raw, dt)[0]))
                ar = base_ib_elem + (RAIN_RID - ib) * stride + foff
                rr = p.try_read(ar, width)
                if len(rr) != width:
                    continue
                rain_v = int(np.frombuffer(rr, dt)[0])
            except ValueError:
                continue
            if len(set(clears)) == 1 and rain_v != clears[0]:
                clear_v = clears[0]
                # plausibility: clear should be 0 (no_phenomenon) and rain a small
                # enum (1/2) under the file-order hypothesis; accept others but flag
                plausible = (clear_v == 0 and 1 <= rain_v <= 7)
                out.append({"field_off": foff, "width": width,
                            "clear_val": clear_v, "rain_val": rain_v,
                            "plausible_enum": bool(plausible)})
    return out


if __name__ == "__main__":
    import argparse
    import json
    ap = argparse.ArgumentParser()
    ap.add_argument("--addr", required=True, help="hex addr of region-ib TEMP element")
    ap.add_argument("--stride", type=int, required=True)
    ap.add_argument("--ib", type=int, default=1)
    ap.add_argument("--window", type=int, default=64)
    args = ap.parse_args()
    p = _wm.open_process("hoi4.exe")
    if p is None:
        raise SystemExit("hoi4.exe not running")
    with p:
        res = confirm_scoped(p, int(args.addr, 16), args.stride, args.ib,
                             search_bytes=args.window)
    print(json.dumps(res, indent=2))
