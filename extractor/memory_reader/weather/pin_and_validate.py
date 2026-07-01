"""pin_and_validate.py - take a candidate weather-array (base, stride, enc, ib),
read back all 5 anchors + 304 regions, climatology cross-check, AND search for a
parallel PRECIP field (4 clear anchors equal, region 31 differs). Prints a verdict.

Usage: python pin_and_validate.py --addr 0x... --stride 4 --enc f32_celsius --ib 1
"""
import argparse
import json
import os
import sys

import numpy as np

_HERE = os.path.dirname(os.path.abspath(__file__))
_MR = os.path.dirname(_HERE)
for _p in (_MR, _HERE):
    if _p not in sys.path:
        sys.path.insert(0, _p)
import winmem as _wm
import validate_candidate as VC
import precip_check as PC

ap = argparse.ArgumentParser()
ap.add_argument("--addr", required=True)
ap.add_argument("--stride", type=int, required=True)
ap.add_argument("--enc", required=True)
ap.add_argument("--ib", type=int, default=1)
ap.add_argument("--tol", type=float, default=1.6)
ap.add_argument("--month", type=int, default=4)
ap.add_argument("--precip-window", type=int, default=128)
args = ap.parse_args()

p = _wm.open_process("hoi4.exe")
if p is None:
    raise SystemExit("hoi4.exe not running")
base = int(args.addr, 16)
with p:
    rep = VC.validate(p, base, args.stride, args.enc, args.ib, tol=args.tol, month=args.month)
    precip = PC.confirm_scoped(p, base, args.stride, args.ib, search_bytes=args.precip_window)
print(json.dumps({"temp": rep, "precip_candidates": precip[:20]}, indent=2))
print("\nVERDICT: anchors %d/5 pass; n_finite=%s; climatology_pearson_May=%s; precip_layouts=%d"
      % (rep["anchor_pass"], rep.get("n_finite"),
         rep.get("climatology_pearson_May"), len(precip)))
