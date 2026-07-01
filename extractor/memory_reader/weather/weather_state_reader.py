"""weather_state_reader.py - VALIDATED live read of gameplay weather
STATE (per-region phenomenon), build 16022851.

Unlike weather_reader.py (which chased a per-region TEMPERATURE scalar that does
not exist as stored memory and therefore fails closed), this module reads the
PHENOMENON state that lockstep determinism actually keeps in memory and that
combat/movement modifiers query. Pinned 2026-06-25 via RTTI structure-RE, 5/5
anchor-validated with a clean negative control. See weather_offsets.draft.json
block WEATHER_STATE_PIN_2026_06_25_RTTI.

STRUCTURE (all offsets are RVAs relative to the live module base):
  CWeatherManager        vtable RVA 0x2950AF0   (singleton)
    +0x40  PdxVector{ begin, u32 size=305, u32 cap=305 }
           -> 305 SWeatherPerRegion structs INLINE, stride 352, index==region_id
  SWeatherPerRegion (stride 352):
    +0x00  vtable (RVA 0x2950AA0)
    +0x08  u32 region_id (== array index)
    +0x78  u32 PHENOMENON  (0=clear, 1/2=active; file-order: 1=rain_light,2=rain_heavy,...)
    +0x5D  u8  intensity/active flag (2 when active)
    +0x58  PdxVector of SLocalWeatherPosition (active fronts; populated iff phenom>=1)
    +0x70  PdxVector of weather-effect-entity ptrs (populated iff phenom>=1)

READ-ONLY: PROCESS_VM_READ|PROCESS_QUERY_INFORMATION only.
"""
import argparse
import json
import os
import struct
import sys
from collections import Counter

_HERE = os.path.dirname(os.path.abspath(__file__))
_MR = os.path.dirname(_HERE)
for _p in (_MR, _HERE):
    if _p not in sys.path:
        sys.path.insert(0, _p)
import winmem as wm
import fastscan as fs

EXPECTED_SHA = "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120"

WMGR_VTABLE_RVA = 0x2950AF0
PERREGION_VTABLE_RVA = 0x2950AA0
MGR_REGION_ARRAY_OFF = 0x40        # PdxVector{begin,u32 size,u32 cap}
PERREGION_STRIDE = 352
REGION_ID_OFF = 0x08
PHENOM_OFF = 0x78
INTENSITY_OFF = 0x5D
ACTIVE_VEC58_OFF = 0x58
ACTIVE_VEC70_OFF = 0x70

HEAP_LO, HEAP_HI = 0x10000000000, 0x7FF000000000

# file-order hypothesis (NOT confirmed by a string table; see draft json residual)
PHENOM_NAMES = {0: "clear", 1: "rain", 2: "heavy_rain_or_storm"}


def _isheap(v):
    return HEAP_LO <= v < HEAP_HI


def locate_manager(pid, mb, arena, workers=6):
    """Return the live CWeatherManager singleton address (or None)."""
    hits = fs.parallel_scan(pid, arena[0], arena[1], workers=workers,
                            **fs.make_qword_eq_matcher(mb + WMGR_VTABLE_RVA))
    return hits[0] if hits else None


def read_region_weather(pid=None):
    """Read the per-region phenomenon for all strategic regions.

    Returns dict: { 'manager': addr, 'array': addr, 'size': n,
                    'regions': { region_id: {'phenom': int, 'name': str,
                                             'intensity': int, 'active': bool} },
                    'distribution': {phenom: count} }  or None on failure.
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
        for k in range(size):
            base = begin + k * PERREGION_STRIDE
            body = p.try_read(base, 0x80)
            if len(body) < 0x80:
                continue
            rid = struct.unpack("<I", body[REGION_ID_OFF:REGION_ID_OFF + 4])[0]
            phenom = struct.unpack("<I", body[PHENOM_OFF:PHENOM_OFF + 4])[0]
            inten = body[INTENSITY_OFF]
            v58 = struct.unpack("<Q", body[ACTIVE_VEC58_OFF:ACTIVE_VEC58_OFF + 8])[0]
            v70 = struct.unpack("<Q", body[ACTIVE_VEC70_OFF:ACTIVE_VEC70_OFF + 8])[0]
            active = _isheap(v58) or _isheap(v70)
            out[rid] = {"phenom": phenom,
                        "name": PHENOM_NAMES.get(phenom, "phenom_%d" % phenom),
                        "intensity": inten, "active": active}
        dist = Counter(v["phenom"] for k, v in out.items() if 1 <= k <= 304)
        return {"manager": mgr, "array": begin, "size": size,
                "regions": out, "distribution": dict(dist)}
    finally:
        p.close()


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("--json", action="store_true", help="dump full per-region JSON")
    args = ap.parse_args()
    res = read_region_weather()
    if res is None:
        raise SystemExit("could not read weather (game not running / build mismatch / manager not found)")
    print("CWeatherManager @ 0x%X  region-array @ 0x%X  size=%d"
          % (res["manager"], res["array"], res["size"]))
    print("phenomenon distribution (regions 1..304):")
    for ph in sorted(res["distribution"]):
        print("  %d (%s): %d regions" % (ph, PHENOM_NAMES.get(ph, "?"), res["distribution"][ph]))
    # anchor sanity
    print("anchor regions:")
    for rid in (31, 20, 129, 160, 292):
        r = res["regions"].get(rid)
        if r:
            print("  region %3d -> phenom=%d (%s) intensity=%d active=%s"
                  % (rid, r["phenom"], r["name"], r["intensity"], r["active"]))
    if args.json:
        print(json.dumps({str(k): v for k, v in res["regions"].items()}, indent=2))
