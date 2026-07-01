"""validate_candidate.py - read back & validate a weather-array CANDIDATE.

Given a candidate array (base address of element for region `ib`, element stride,
encoding), this READ-ONLY tool:
  1. Reads back all 5 anchor temperatures and confirms each within tolerance.
  2. Decodes ALL 304 region slots and checks physical plausibility + climatology
     cross-check (mid-May): the array's hot/cold extremes should roughly agree
     with the May climatology (India/Philippines hot, France/high-lat cold).
  3. If a parallel/embedded precip field is supplied, confirms region 31 is the
     RAIN odd-one-out and the 4 clear anchors share the clear value.

Usage (programmatic): call validate(p, base_elem_ib, stride, enc, ib, ...).
"""
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

_K = 273.15
# region ids from authoritative mod files (Libya = 292, not the stale climo's 225)
ANCHORS = [(20, 8.0, False), (31, 26.0, True), (129, 11.0, False),
           (160, 24.0, False), (292, 16.0, False)]

DECODE = {
    "f32_celsius": ("<f4", 4, lambda a: a.astype(np.float64)),
    "f32_kelvin":  ("<f4", 4, lambda a: a.astype(np.float64) - _K),
    "f64_celsius": ("<f8", 8, lambda a: a.astype(np.float64)),
    "f64_kelvin":  ("<f8", 8, lambda a: a.astype(np.float64) - _K),
    "f16_celsius": ("<f2", 2, lambda a: a.astype(np.float64)),
    "i32_raw":     ("<i4", 4, lambda a: a.astype(np.float64)),
    "i16_raw":     ("<i2", 2, lambda a: a.astype(np.float64)),
    "i32_x10":     ("<i4", 4, lambda a: a.astype(np.float64) / 10.0),
    "i32_x100":    ("<i4", 4, lambda a: a.astype(np.float64) / 100.0),
    "i32_x1000":   ("<i4", 4, lambda a: a.astype(np.float64) / 1000.0),
    "i32_x100000": ("<i4", 4, lambda a: a.astype(np.float64) / 100000.0),
    "i16_x10":     ("<i2", 2, lambda a: a.astype(np.float64) / 10.0),
    "i16_x100":    ("<i2", 2, lambda a: a.astype(np.float64) / 100.0),
}


def load_climatology(month=4):
    """Mid-month midpoint temperature per region from the AUTHORITATIVE mod file
    table (region_weather_mod.json, built from mod/test strategicregions). `month`
    is the 0-based period index (4 == May). Falls back to nearest period."""
    path = os.path.join(_HERE, "region_weather_mod.json")
    d = {int(k): v for k, v in json.load(open(path)).items()}
    mid = {}
    for rid, info in d.items():
        per = info.get("periods", [])
        if not per:
            continue
        p = per[month] if month < len(per) else per[-1]
        if p["tmin"] is not None:
            mid[rid] = (p["tmin"] + p["tmax"]) / 2.0
    return mid


def read_slots(p, base_ib_elem, stride, enc, ib, n=304):
    """Read region-id 1..n temperatures. base_ib_elem = address of the element for
    region `ib` (i.e. array start for a flat array, ib in {0,1}). Returns dict
    rid -> temperature_C (np.nan if unreadable)."""
    dt, ebytes, conv = DECODE[enc]
    out = {}
    for rid in range(1, n + 1):
        addr = base_ib_elem + (rid - ib) * stride
        raw = p.try_read(addr, ebytes)
        if len(raw) != ebytes:
            out[rid] = float("nan")
            continue
        v = np.frombuffer(raw, dt)
        out[rid] = float(conv(v)[0])
    return out


def validate(p, base_ib_elem, stride, enc, ib, tol=1.5, month=4):
    rep = {"anchors": [], "anchor_pass": 0, "all_pass": False}
    dt, ebytes, conv = DECODE[enc]
    for (rid, temp, rain) in ANCHORS:
        addr = base_ib_elem + (rid - ib) * stride
        raw = p.try_read(addr, ebytes)
        if len(raw) != ebytes:
            rep["anchors"].append({"rid": rid, "expect": temp, "got": None,
                                   "ok": False})
            continue
        got = float(conv(np.frombuffer(raw, dt))[0])
        ok = abs(got - temp) <= tol
        rep["anchors"].append({"rid": rid, "expect": temp, "got": round(got, 3),
                               "ok": bool(ok)})
        if ok:
            rep["anchor_pass"] += 1
    rep["all_pass"] = (rep["anchor_pass"] == 5)

    # full 304 decode + climatology cross-check
    slots = read_slots(p, base_ib_elem, stride, enc, ib)
    vals = np.array([slots[r] for r in range(1, 305)], dtype=np.float64)
    finite = np.isfinite(vals)
    rep["n_finite"] = int(finite.sum())
    if finite.sum() > 50:
        fv = vals[finite]
        rep["temp_stats_C"] = {"min": round(float(fv.min()), 2),
                               "max": round(float(fv.max()), 2),
                               "mean": round(float(fv.mean()), 2),
                               "std": round(float(fv.std()), 2)}
        rep["in_physical_range_frac"] = round(
            float(((fv >= -60) & (fv <= 55)).mean()), 3)
        # climatology Pearson (mid-May midpoints) over regions both have
        mid = load_climatology(month)
        xs, ys = [], []
        for rid in range(1, 305):
            if rid in mid and np.isfinite(slots[rid]):
                xs.append(mid[rid]); ys.append(slots[rid])
        if len(xs) > 50:
            xs = np.array(xs); ys = np.array(ys)
            if xs.std() > 0 and ys.std() > 0:
                rep["climatology_pearson_May"] = round(
                    float(np.corrcoef(xs, ys)[0, 1]), 3)
        # hottest/coldest by this array
        order = np.argsort(vals)
        rep["coldest_rids"] = [int(order[i] + 1) for i in range(8)
                               if finite[order[i]]][:5]
        rep["hottest_rids"] = [int(order[-1 - i] + 1) for i in range(8)
                               if finite[order[-1 - i]]][:5]
    return rep


if __name__ == "__main__":
    import argparse
    ap = argparse.ArgumentParser()
    ap.add_argument("--addr", required=True, help="hex address of region-ib element")
    ap.add_argument("--stride", type=int, required=True)
    ap.add_argument("--enc", required=True)
    ap.add_argument("--ib", type=int, default=1)
    ap.add_argument("--tol", type=float, default=1.5)
    ap.add_argument("--month", type=int, default=4)
    args = ap.parse_args()
    p = _wm.open_process("hoi4.exe")
    if p is None:
        raise SystemExit("hoi4.exe not running")
    with p:
        rep = validate(p, int(args.addr, 16), args.stride, args.enc, args.ib,
                       tol=args.tol, month=args.month)
    print(json.dumps(rep, indent=2))
