"""weather_temp_model.py - VALIDATED live read of the game per-strategic-region
realized TEMPERATURE (deg C) and the phenomenon enum->name mapping, build 16022851.

BACKGROUND
----------
Earlier weather work (weather_reader.py, weather_offsets.draft.json) concluded the
displayed temperature was NOT a stored scalar and could only be recomputed from
climatology + an unlocated lockstep RNG ("W3 = computed, no oracle"). That negative
came from a VALUE-anchored scan that compared memory against in-game PROVINCE tooltip
temperatures with a tight (+-1.6 C) tolerance. It missed the real field because the
stored value is the REGION-aggregate temperature, which differs from a province
tooltip by up to ~7 C (the tooltip adds per-province terrain/elevation/time-of-day
adjustment on top of the region value).

This module pins the stored field DIRECTLY, found via the decompiled SWeatherPerRegion
save-serializer rather than by value scan:

  The save-writer for SWeatherPerRegion emits a value under save-token 0x2F01 from
  struct offset +0x138:   emit(stream, 0x2F01, *(u64*)(this + 0x138)).
  In the CWeatherChancePeriod (climatology PERIOD definition) serializer, token 0x2F01
  is the TEMPERATURE element-range (period+0x40 = tempMin, period+0x48 = tempMax, with
  the validation string "(%s) Region temperature, min > max" at weather.cpp:0x95).
  => token 0x2F01 == temperature => SWeatherPerRegion +0x138 == realized temperature.

ENCODING
--------
+0x138 is a signed 64-bit fixed-point value (Clausewitz CFixedPoint, scale 100000 / 1e5).
Verified sign-extension: for the 13 sub-zero regions the high dword (+0x13C) is -1
(0xFFFFFFFF) and for the rest it is 0, so the i64 read reproduces the i32 read exactly
across all 305 regions and handles negatives correctly. The 1e5 scale is corroborated
by the CWeatherChancePeriod default temp range constants (-1000000 -> -10.0 C,
3500000 -> +35.0 C) and is Codex-verified.

LIVE VALIDATION (paused frame ~1946.5, May; pid 962744; 305 regions)
  - Range -7.55 .. +37.57 C, mean 13.71, all 304 land regions in a sane Celsius range.
  - 285/304 inside that region's MAY climatology [tmin,tmax] band; of the 19 out-of-band
    11 read exactly 0.0 (all-sea / inactive regions where temp is not computed) and the
    rest sit within ~1 C of a band edge (one-day variation). mean(temp - midpoint) = -2.2 C.
  - Tracks all 5 in-game anchor regions with a consistent region-vs-province offset.

STRUCTURE (offsets are RVAs vs the live module base; relocate every run, ASLR-safe)
  CWeatherManager (vtable RVA 0x2950AF0, singleton)
    +0x40  PdxVector{ begin, u32 size=305, u32 cap=305 } -> inline SWeatherPerRegion[]
  SWeatherPerRegion (stride 352, index == region_id)
    +0x08  u32 region_id
    +0x78  u32 phenomenon enum (0 clear, 1.., realized single phenomenon)
    +0x5D  u8  active/intensity flag (2 when active)
    +0x108..+0x130  per-phenomenon active bool flags (save tokens 0x2F02..0x3B21)
    +0x138 i64/i32 fixed-point(1e5) TEMPERATURE deg C  (save token 0x2F01)  <-- THIS MODULE

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
MGR_REGION_ARRAY_OFF = 0x40        # PdxVector{begin, u32 size, u32 cap}
PERREGION_STRIDE = 352
REGION_ID_OFF = 0x08
PHENOM_OFF = 0x78
INTENSITY_OFF = 0x5D
TEMP_OFF = 0x138                   # i64 fixed-point, scale 1e5, deg C (save token 0x2F01)
FIXED_POINT_SCALE = 100000.0

HEAP_LO, HEAP_HI = 0x10000000000, 0x7FF000000000

# Realized single-phenomenon enum -> name.
# Source: the order the phenomenon keys appear in every map/strategicregions/*.txt
# period block (file-order hypothesis), corroborated by the decompiled save-writer
# token order (0x2F02..0x3B21 for the per-phenomenon active flags) and the
# is_fighting_in_weather predicate token set. 0/1/2 are observed live and match;
# 3..7 are the documented remaining order and are NOT yet value-confirmed in memory.
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

# Save-token <-> phenomenon for the per-region active bool flags at +0x108..+0x130.
# (Documentation; not required for the temperature read.)
PHENOM_FLAG_OFFSETS = {
    0x108: ("rain_heavy", 0x2F02),
    0x110: ("snow", 0x2F03),
    0x118: ("blizzard", 0x2F04),
    0x120: ("arctic_water", 0x2F05),
    0x128: ("sandstorm", 0x2F07),
    0x130: ("mud_or_other", 0x3B21),
}


def phenomenon_name(value):
    """Map a realized phenomenon enum integer to its name (best-known mapping)."""
    return PHENOM_NAMES.get(value, "phenom_%d" % value)


def _isheap(v):
    return HEAP_LO <= v < HEAP_HI


def locate_manager(pid, mb, arena, workers=6):
    """Return the live CWeatherManager singleton address (or None)."""
    hits = fs.parallel_scan(pid, arena[0], arena[1], workers=workers,
                            **fs.make_qword_eq_matcher(mb + WMGR_VTABLE_RVA))
    return hits[0] if hits else None


def read_region_temperatures(pid=None):
    """Read realized per-region temperature (deg C) + phenomenon for all regions.

    Returns dict:
      { 'manager': addr, 'array': addr, 'size': n,
        'regions': { region_id: {'temp_c': float|None, 'temp_raw': int,
                                 'phenom': int, 'name': str, 'active': bool} },
        'temp_min_c': float, 'temp_max_c': float, 'temp_mean_c': float,
        'n_with_temp': int }
    or None on failure (game not running / build mismatch / manager not found).

    temp_c is None for regions whose raw value is exactly 0 (sea-only / inactive
    regions where the engine never computes a temperature this frame); those are
    NOT real 0 deg C readings.
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
        hdr = p.try_read(mgr + MGR_REGION_ARRAY_OFF, 16)
        if len(hdr) < 16:
            return None
        begin, size, cap = struct.unpack("<QII", hdr)
        if not _isheap(begin) or not (1 <= size <= 1000):
            return None
        out = {}
        temps = []
        for k in range(size):
            base = begin + k * PERREGION_STRIDE
            body = p.try_read(base, 0x140)
            if len(body) < 0x140:
                continue
            rid = struct.unpack("<I", body[REGION_ID_OFF:REGION_ID_OFF + 4])[0]
            phenom = struct.unpack("<I", body[PHENOM_OFF:PHENOM_OFF + 4])[0]
            inten = body[INTENSITY_OFF]
            # signed 64-bit fixed-point (sign-extended; +0x13C is -1 for negatives, 0 otherwise)
            raw = struct.unpack("<q", body[TEMP_OFF:TEMP_OFF + 8])[0]
            temp_c = None if raw == 0 else raw / FIXED_POINT_SCALE
            if temp_c is not None and 1 <= rid <= 304:
                temps.append(temp_c)
            out[rid] = {"temp_c": temp_c, "temp_raw": raw,
                        "phenom": phenom, "name": phenomenon_name(phenom),
                        "active": inten != 0}
        res = {"manager": mgr, "array": begin, "size": size, "regions": out,
               "n_with_temp": len(temps)}
        if temps:
            res["temp_min_c"] = min(temps)
            res["temp_max_c"] = max(temps)
            res["temp_mean_c"] = sum(temps) / len(temps)
        return res
    finally:
        p.close()


def _validate(res, month_idx=None):
    """Cross-check temps vs the static climatology band for the given month index.

    month_idx: 0..11. If None, validation against a fixed month is skipped and only
    the physical-range sanity check is reported. (The live clock decode lives in
    weather_reader.locate_clock; passing the live month here is the caller's job.)
    """
    clim_path = os.path.join(_HERE, "weather_climatology.json")
    if not os.path.exists(clim_path):
        return None
    clim = json.load(open(clim_path))
    inband = below = above = total = 0
    diffs = []
    for rid, r in res["regions"].items():
        if not (1 <= rid <= 304) or r["temp_c"] is None:
            continue
        m = clim.get(str(rid), {}).get("months", {}).get(str(month_idx)) if month_idx is not None else None
        if not m:
            continue
        t = r["temp_c"]; lo = m["tmin"]; hi = m["tmax"]
        total += 1
        if lo <= t <= hi:
            inband += 1
        elif t < lo:
            below += 1
        else:
            above += 1
        diffs.append(t - (lo + hi) / 2.0)
    out = {"month_idx": month_idx, "n": total, "in_band": inband,
           "below": below, "above": above}
    if diffs:
        out["mean_minus_midpoint"] = sum(diffs) / len(diffs)
    return out


if __name__ == "__main__":
    ap = argparse.ArgumentParser(description="Live the game per-region temperature reader.")
    ap.add_argument("--json", action="store_true", help="dump full per-region JSON")
    ap.add_argument("--month", type=int, default=None,
                    help="climatology month index 0..11 for band validation (e.g. 4 = May)")
    args = ap.parse_args()
    res = read_region_temperatures()
    if res is None:
        raise SystemExit("could not read weather temperature "
                         "(game not running / build mismatch / manager not found)")
    print("CWeatherManager @ 0x%X  region-array @ 0x%X  size=%d"
          % (res["manager"], res["array"], res["size"]))
    if "temp_mean_c" in res:
        print("temperature (regions 1..304, %d with data): min %.2f  max %.2f  mean %.2f  (deg C)"
              % (res["n_with_temp"], res["temp_min_c"], res["temp_max_c"], res["temp_mean_c"]))
    print("anchor regions (region-aggregate temp; in-game tooltip is province-resolved):")
    anchors = {20: 8, 31: 26, 129: 11, 160: 24, 292: 16}
    for rid, ttip in anchors.items():
        r = res["regions"].get(rid)
        if r:
            tc = "n/a" if r["temp_c"] is None else "%.2f" % r["temp_c"]
            print("  region %3d -> temp=%s C  phenom=%d (%s)  [tooltip %d]"
                  % (rid, tc, r["phenom"], r["name"], ttip))
    if args.month is not None:
        v = _validate(res, args.month)
        if v:
            print("climatology band check (month idx %d): in-band %d/%d  below %d  above %d  mean(temp-mid) %+.2f"
                  % (v["month_idx"], v["in_band"], v["n"], v["below"], v["above"],
                     v.get("mean_minus_midpoint", 0.0)))
    if args.json:
        print(json.dumps({str(k): v for k, v in res["regions"].items()}, indent=2))
