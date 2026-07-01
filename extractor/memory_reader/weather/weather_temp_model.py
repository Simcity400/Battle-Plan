"""weather_temp_model.py - live read of the game per-PROVINCE realized DISPLAYED
TEMPERATURE (deg C) and per-frame temperature CHANGE, build 16022851.

RETRACTION (2026-06-25 re-validation against fresh in-game anchors)
------------------------------------------------------------------
A prior version of this module claimed the per-REGION field
`SWeatherPerRegion + 0x138` (i64 fixed-point /1e5, save token 0x2F01) was the
"validated realized region temperature". THAT CLAIM IS WITHDRAWN. Re-checked
against 4 fresh debug-tooltip anchors (regions 239/38/7/40, paused frame), the
per-region +0x138 value is OFF from the displayed temperature by a region-specific
0.6 .. 4.3 C and is NOT a uniform region-vs-province term:

    region   displayed   per-region +0x138   diff
    239        20.09          19.29          -0.80
    38         18.16          13.84          -4.32   <-- not a constant offset
    7          11.57          10.95          -0.62
    40         10.46           8.34          -2.12

`SWeatherPerRegion + 0x138` is a region BASE / climatology-derived temperature
(it sits inside the static climatology [tmin,tmax] band for the current month for
all 4 regions), not the realized value the UI shows. The realized DISPLAYED
temperature the tooltip renders is a per-PROVINCE quantity and lives in a SEPARATE
per-province weather array.

THE CORRECT FIELD (this module)
-------------------------------
  CWeatherManager (vtable RVA 0x2950AF0, singleton)
    +0x10  PdxVector{ begin, u32 size=13414, u32 cap=13414 } -> inline
           SWeatherPerProvince[] (stride 0x180 = 384), index == province_id
           (the per-REGION vector is the SEPARATE one at +0x40, size 305.)
  SWeatherPerProvince (stride 384, vtable RVA 0x2950A50, index == province_id)
    +0x08  u16 province_id  (== array index)
    +0x108 i32 /1e5  SECONDARY temperature (prev-tick / other model component;
                     ~50/50 above-or-below the displayed temp; NOT a strict base)
    +0x118 i32 /1e5  DISPLAYED TEMPERATURE deg C        <-- THIS MODULE (primary)
    +0x128 i32 /1e5  TEMPERATURE CHANGE deg C (signed; the tooltip "temp change")
    +0x160 i32 /1e5  mirror of +0x118 (double-buffer copy; == +0x118 when paused)
    +0x170 i32 /1e5  mirror of +0x128

ENCODING
--------
Signed 32-bit fixed-point, scale 100000 (1e5), value is deg C. Negative temps and
negative temp_change are stored as a normal two's-complement i32 (region 38's
temp_change -0.12072 reads 0xFFFFD0D8 = -12072). Read as <i (i32), /1e5.

VALIDATION (paused frame, pid 962744, build 16022851; 4 fresh anchors)
  BIAS-FREE cross-check - province selected by its +0x128 == the user's reported
  temp_change, then +0x118 read INDEPENDENTLY (temp not used as selector):
    region 239 -> province 1896: +0x128 = +1.48054 (exact) ; +0x118 = 20.0977 vs displayed 20.09  OK
    region 38  -> province 3544: +0x128 = -0.12072 (exact) ; +0x118 = 18.1664 vs displayed 18.16  OK
    region 7   -> province 6421: +0x128 = +0.51412 (exact) ; +0x118 = 11.5742 vs displayed 11.57  OK
    region 40  -> province 9244: +0x128 = -0.44206 (exact) ; +0x118 = 10.4611 vs displayed 10.46  OK
  Structural invariants over 308 data-carrying provinces (selected by INDEX, no
  value bias): mirror (+0x160==+0x118 AND +0x170==+0x128) 308/308; +0x118 a sane
  Celsius 308/308; |+0x128| <= 5 308/308. Codex-verified (selection bias retired by
  the temp_change-selected cross-check; manager+0x10 locator endorsed).

PRECIPITATION (2026-06-26 - RESOLVED, supersedes the prior "rain not in this struct"
note). Precipitation and snow ARE in this SAME per-province struct; the earlier scan
missed them only because it tested f32 - they are i32 FIXED-POINT /1e5 (the temperature
encoding family), not float:
    +0x38  i32 /1e5  WATER (precipitation amount; the save's per-province `water` token)
    +0x40  i32 /1e5  SNOW  (snow amount; the save's per-province `snow` token)
VALIDATION (paused frame, build 16022851, vs melt provinces.<pid>): WATER 9469/9469
nonzero provinces exact + all 3945 zero-water provinces read ~0; SNOW 823/823 exact.
High-variance continuous GT (water 1e-5..220.45) -> coincidence-free; single offset
each (no double-buffer mirror, unlike temperature). read_province_precipitation() below.
NOTE: this is the SAVE-PERSISTED per-province precip amount (authoritative, exact-save-
matched), NOT the tooltip's derived "cm rain" display value (which is a per-region/scaled
derivation). Codex-reviewed.

PHENOMENON (clear/rain/snow/...) is a per-REGION field at SWeatherPerRegion+0x78 and
is read by weather_state_reader.py; this module exposes phenomenon_name() for callers
that already hold a phenomenon enum but does NOT re-read it.

READ-ONLY: PROCESS_VM_READ | PROCESS_QUERY_INFORMATION only; ReadProcessMemory; no writes.
"""
import argparse
import json
import os
import struct
import sys

_HERE = os.path.dirname(os.path.abspath(__file__))
_MR = os.path.dirname(_HERE)
for _p in (_MR, _HERE):
    if _p not in sys.path:
        sys.path.insert(0, _p)
import winmem as wm
import fastscan as fs

EXPECTED_SHA = "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120"

WMGR_VTABLE_RVA = 0x2950AF0
MGR_PROV_ARRAY_OFF = 0x10           # PdxVector{begin, u32 size, u32 cap} -> per-province
MGR_REGION_ARRAY_OFF = 0x40         # the SEPARATE per-region vector (size 305)
PERPROV_STRIDE = 0x180              # 384
PROV_ID_OFF = 0x08                  # u16 province_id == array index
TEMP_OFF = 0x118                    # i32 /1e5 displayed temperature deg C
TEMP_CHANGE_OFF = 0x128            # i32 /1e5 temperature change deg C (signed)
TEMP2_OFF = 0x108                   # i32 /1e5 secondary temp (prev-tick / component)
TEMP_MIRROR_OFF = 0x160            # i32 /1e5 mirror of TEMP_OFF
TEMP_CHANGE_MIRROR_OFF = 0x170     # i32 /1e5 mirror of TEMP_CHANGE_OFF
WATER_OFF = 0x38                    # i32 /1e5 precipitation amount (save `water`)
SNOW_OFF = 0x40                    # i32 /1e5 snow amount (save `snow`)
FIXED_POINT_SCALE = 100000.0

# A province whose weather isn't computed this frame reads exactly 0 at both the
# temp and temp_change slots (sea-only / inactive); expose temp_c as None there
# rather than a literal 0 C.
HEAP_LO, HEAP_HI = 0x10000000000, 0x7FF000000000

# Realized per-region phenomenon enum -> name (file-order; 0/1/2 live-confirmed,
# 3..7 documented order). Kept here for callers that already hold a phenomenon enum
# (the enum itself is read per-region by weather_state_reader.py, NOT by this module).
PHENOM_NAMES = {
    0: "clear",          # no_phenomenon
    1: "rain_light",
    2: "rain_heavy",
    3: "snow",
    4: "blizzard",
    5: "arctic_water",
    6: "mud",
    7: "sandstorm",
}


def phenomenon_name(value):
    """Map a realized phenomenon enum integer to its name (best-known mapping)."""
    return PHENOM_NAMES.get(value, "phenom_%d" % value)


def _isheap(v):
    return HEAP_LO <= v < HEAP_HI


def _i32(buf, off):
    return struct.unpack_from("<i", buf, off)[0]


def locate_manager(pid, mb, arena, workers=6):
    """Return the live CWeatherManager singleton address (or None)."""
    hits = fs.parallel_scan(pid, arena[0], arena[1], workers=workers,
                            **fs.make_qword_eq_matcher(mb + WMGR_VTABLE_RVA))
    return hits[0] if hits else None


def read_province_temperatures(pid=None):
    """Read the realized per-PROVINCE displayed temperature + temp_change.

    Returns dict:
      { 'manager': addr, 'array': addr, 'size': n,
        'provinces': { province_id: {
              'temp_c': float|None,        # +0x118 displayed temperature deg C
              'temp_raw': int,             # raw i32 at +0x118
              'temp_change_c': float|None, # +0x128 temperature change deg C
              'temp2_c': float|None } },   # +0x108 secondary temp (component)
        'temp_min_c': float, 'temp_max_c': float, 'temp_mean_c': float,
        'n_with_temp': int }
    or None on failure (game not running / build mismatch / manager not found).

    temp_c (and temp_change_c) are None for provinces whose temp slot is exactly 0
    (sea-only / inactive provinces where the engine never realized a temperature this
    frame); those are NOT real 0 deg C readings.
    """
    if pid is None:
        pid = wm.find_pid("hoi4.exe")
        if pid is None:
            return None
    p = wm.Process(pid, "hoi4.exe")
    mb = p.module_base
    fp = p.fingerprint().get("sha256")
    if fp != EXPECTED_SHA:
        print("WARN build sha mismatch: %s (expected %s)" % (fp, EXPECTED_SHA))
    p.close()

    arena = fs.discover_arena(pid)
    if arena is None:
        return None
    mgr = locate_manager(pid, mb, arena)
    if mgr is None:
        return None

    p = wm.Process(pid, "hoi4.exe")
    try:
        hdr = p.try_read(mgr + MGR_PROV_ARRAY_OFF, 16)
        if len(hdr) < 16:
            return None
        begin, size, cap = struct.unpack("<QII", hdr)
        # sanity: readable heap begin, sane element count, plausible vector
        if not _isheap(begin) or not (1 <= size <= 100000) or cap < size:
            return None
        out = {}
        temps = []
        for k in range(size):
            base = begin + k * PERPROV_STRIDE
            body = p.try_read(base, PERPROV_STRIDE)
            if len(body) < TEMP_CHANGE_MIRROR_OFF + 4:
                continue
            pid_read = struct.unpack_from("<H", body, PROV_ID_OFF)[0]
            # the inline array is index==province_id; trust the index for the key but
            # keep the read id available implicitly via k.
            traw = _i32(body, TEMP_OFF)
            tcraw = _i32(body, TEMP_CHANGE_OFF)
            t2raw = _i32(body, TEMP2_OFF)
            temp_c = None if traw == 0 else traw / FIXED_POINT_SCALE
            tc_c = None if traw == 0 else tcraw / FIXED_POINT_SCALE
            t2_c = None if t2raw == 0 else t2raw / FIXED_POINT_SCALE
            if temp_c is not None:
                temps.append(temp_c)
            out[k] = {"temp_c": temp_c, "temp_raw": traw,
                      "temp_change_c": tc_c, "temp2_c": t2_c,
                      "province_id_read": pid_read}
        res = {"manager": mgr, "array": begin, "size": size, "provinces": out,
               "n_with_temp": len(temps)}
        if temps:
            res["temp_min_c"] = min(temps)
            res["temp_max_c"] = max(temps)
            res["temp_mean_c"] = sum(temps) / len(temps)
        return res
    finally:
        p.close()


def read_province_precipitation(pid=None):
    """Read the realized per-PROVINCE PRECIPITATION (`water`) and SNOW amounts from the
    SAME CWeatherManager per-province array as the temperature.

    Returns dict:
      { 'manager': addr, 'array': addr, 'size': n,
        'provinces': { province_id: {
              'water': float|None,   # +0x38 i32/1e5 precipitation amount (save `water`)
              'snow':  float|None,   # +0x40 i32/1e5 snow amount (save `snow`)
              'water_raw': int, 'snow_raw': int } },
        'n_with_water': int, 'n_with_snow': int,
        'water_max': float, 'water_mean': float }
    or None on failure.

    These are the SAVE-PERSISTED per-province amounts (validated exact vs the save's
    provinces.<pid>.water / .snow), NOT the tooltip's derived "cm rain" display value.
    water/snow are None for provinces whose slot reads exactly 0 (no precip this frame).
    """
    if pid is None:
        pid = wm.find_pid("hoi4.exe")
        if pid is None:
            return None
    p = wm.Process(pid, "hoi4.exe")
    mb = p.module_base
    fp = p.fingerprint().get("sha256")
    if fp != EXPECTED_SHA:
        print("WARN build sha mismatch: %s (expected %s)" % (fp, EXPECTED_SHA))
    p.close()

    arena = fs.discover_arena(pid)
    if arena is None:
        return None
    mgr = locate_manager(pid, mb, arena)
    if mgr is None:
        return None

    p = wm.Process(pid, "hoi4.exe")
    try:
        hdr = p.try_read(mgr + MGR_PROV_ARRAY_OFF, 16)
        if len(hdr) < 16:
            return None
        begin, size, cap = struct.unpack("<QII", hdr)
        if not _isheap(begin) or not (1 <= size <= 100000) or cap < size:
            return None
        out = {}
        waters = []
        nsnow = 0
        for k in range(size):
            base = begin + k * PERPROV_STRIDE
            body = p.try_read(base, PERPROV_STRIDE)
            if len(body) < SNOW_OFF + 4:
                continue
            wraw = _i32(body, WATER_OFF)
            sraw = _i32(body, SNOW_OFF)
            # raw == 0  -> None: a measured-0 is indistinguishable from "absent this frame"
            #   and mirrors the save's omitted-when-default token (snow/water are validated
            #   live==save exact for ALL nonzero provinces; see validate_snow_usa1936.py).
            # raw <  0  -> None: an ANOMALY (precip/snow are non-negative); never observed
            #   on the validated frame (snow raw range 3..8000000). The raw i32 is preserved
            #   in snow_raw/water_raw regardless, so no caller loses the measured slot.
            water = None if wraw <= 0 else wraw / FIXED_POINT_SCALE
            snow = None if sraw <= 0 else sraw / FIXED_POINT_SCALE
            if water is not None:
                waters.append(water)
            if snow is not None:
                nsnow += 1
            out[k] = {"water": water, "snow": snow,
                      "water_raw": wraw, "snow_raw": sraw}
        res = {"manager": mgr, "array": begin, "size": size, "provinces": out,
               "n_with_water": len(waters), "n_with_snow": nsnow}
        if waters:
            res["water_max"] = max(waters)
            res["water_mean"] = sum(waters) / len(waters)
        return res
    finally:
        p.close()


def precipitation_for_province(res, province_id):
    """Convenience: precipitation `water` (save units) for one province, or None."""
    if not res:
        return None
    pr = res.get("provinces", {}).get(province_id)
    return pr["water"] if pr else None


# Backwards-compat alias: the previous public entry point name. It now returns the
# per-PROVINCE result (the realized displayed temperature), NOT the retired per-region
# read. Kept so existing callers don't break; the 'regions' key is gone -> 'provinces'.
def read_region_temperatures(pid=None):
    """DEPRECATED name. Returns per-PROVINCE temperatures (see
    read_province_temperatures). The old per-region +0x138 read was retired because it
    is a climatology base, not the displayed temperature."""
    return read_province_temperatures(pid)


def temperature_for_province(res, province_id):
    """Convenience: displayed temp (deg C) for one province from a result dict, or None."""
    if not res:
        return None
    pr = res.get("provinces", {}).get(province_id)
    return pr["temp_c"] if pr else None


def _validate_precip(res):
    """Validate live precip vs the crack-time precip GT (_gt_precip.json) if present.
    Returns a tally dict (water N/N, snow N/N)."""
    gt_path = os.path.join(_HERE, "_gt_precip.json")
    if not res or not os.path.isfile(gt_path):
        return {}
    gt = json.load(open(gt_path))
    water = {int(k): v for k, v in gt.get("water", {}).items()}
    snow = {int(k): v for k, v in gt.get("snow", {}).items()}
    prov = res["provinces"]
    wm_ = ws = 0
    for pid, wv in water.items():
        pr = prov.get(pid)
        if pr and pr["water"] is not None and abs(pr["water"] - wv) <= max(0.01, abs(wv) * 0.003):
            wm_ += 1
    for pid, sv in snow.items():
        pr = prov.get(pid)
        if pr and pr["snow"] is not None and abs(pr["snow"] - sv) <= max(0.01, abs(sv) * 0.003):
            ws += 1
    return {"water": "%d/%d" % (wm_, len(water)), "snow": "%d/%d" % (ws, len(snow))}


if __name__ == "__main__":
    ap = argparse.ArgumentParser(description="Live the game per-province displayed-temperature + precipitation reader.")
    ap.add_argument("--json", action="store_true", help="dump full per-province JSON")
    ap.add_argument("--prov", type=int, action="append", default=None,
                    help="print the temp for specific province id(s)")
    ap.add_argument("--precip", action="store_true",
                    help="read+validate per-province precipitation (water) and snow instead of temperature")
    args = ap.parse_args()
    if args.precip:
        pres = read_province_precipitation()
        if pres is None:
            raise SystemExit("could not read precipitation "
                             "(game not running / build mismatch / manager not found)")
        print("CWeatherManager @ 0x%X  per-province array @ 0x%X  size=%d"
              % (pres["manager"], pres["array"], pres["size"]))
        if "water_mean" in pres:
            print("precipitation (water): %d provinces with data; max %.4f mean %.4f; snow provinces %d"
                  % (pres["n_with_water"], pres["water_max"], pres["water_mean"], pres["n_with_snow"]))
        v = _validate_precip(pres)
        if v:
            print("validation vs crack-time melt anchor: water %s  snow %s"
                  % (v.get("water"), v.get("snow")))
        for prov in (args.prov or []):
            pr = pres["provinces"].get(prov)
            if pr is None:
                print("  province %d: not in array" % prov)
            else:
                print("  province %d: water=%s snow=%s"
                      % (prov, "n/a" if pr["water"] is None else "%.5f" % pr["water"],
                         "n/a" if pr["snow"] is None else "%.5f" % pr["snow"]))
        raise SystemExit(0)
    res = read_province_temperatures()
    if res is None:
        raise SystemExit("could not read weather temperature "
                         "(game not running / build mismatch / manager not found)")
    print("CWeatherManager @ 0x%X  per-province array @ 0x%X  size=%d"
          % (res["manager"], res["array"], res["size"]))
    if "temp_mean_c" in res:
        print("displayed temperature (%d provinces with data): min %.2f  max %.2f  mean %.2f  (deg C)"
              % (res["n_with_temp"], res["temp_min_c"], res["temp_max_c"], res["temp_mean_c"]))
    # the 4 re-validation anchor provinces (selected in RE by exact temp_change)
    print("re-validation anchor provinces (displayed temp / temp_change):")
    for prov, disp in ((1896, 20.09), (3544, 18.16), (6421, 11.57), (9244, 10.46)):
        pr = res["provinces"].get(prov)
        if pr and pr["temp_c"] is not None:
            print("  province %5d -> temp=%6.2f C  temp_change=%+.5f  [in-game displayed %.2f]"
                  % (prov, pr["temp_c"], pr["temp_change_c"], disp))
    for prov in (args.prov or []):
        pr = res["provinces"].get(prov)
        if pr is None:
            print("  province %d: not in array" % prov)
        elif pr["temp_c"] is None:
            print("  province %d: no temperature this frame (inactive/sea)" % prov)
        else:
            print("  province %d: temp=%.2f C  temp_change=%+.5f  temp2=%s"
                  % (prov, pr["temp_c"], pr["temp_change_c"],
                     "n/a" if pr["temp2_c"] is None else "%.2f" % pr["temp2_c"]))
    if args.json:
        print(json.dumps({str(k): v for k, v in res["provinces"].items()}, indent=2))
