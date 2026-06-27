"""weather_fields_live.py - LIVE per-region + per-province WEATHER fields (read-only).

Extends weather_state_reader.py's CWeatherManager locator with the 6 weather fields
cracked + validated read-only vs the content-matched USA_1936_02_07_06 melt
(build 16022851; see re_scratch/_w2wx_weather_offsets.draft.json). The melt is the
SAME frame as the live game, so validation is exact-match.

CWeatherManager singleton (vtable RVA 0x2950AF0):
  +0x40  PdxVector{begin,u32 size==305,u32 cap} -> SWeatherPerRegion, stride 352, idx==region_id
  +0x10  PdxVector{begin,u32 size==13414,u32 cap} -> SWeatherPerProvince, stride 0x180, idx==province_id

SWeatherPerRegion (struct-relative):
  +0x138 i32 temperature  (/1e5 deg C)                                   [305/305 EXACT]
  phenomenon flags (u8 low-byte boolean):
    rain_light 0x108, rain_heavy 0x110, snow 0x118, blizzard 0x120, arctic_water 0x130
                                                                          [each 305/305 EXACT]
  +0x150 u32 next_weather_change (abs_hours = raw - 43799999; raw==43817520 = no-change)
                                                                          [304/304 EXACT]
  +0x30  PdxVector active_modifiers {begin,u32 size,u32 cap}             [count 305/305]

SWeatherPerProvince (struct-relative):
  +0x130 u8 mud (boolean)                                       [13414-wide: TP146 FP0 FN0]
  +0x148 PdxVector custom_modifiers {begin,u32 size,u32 cap}    [13414-wide: TP52 FP0 FN0]

phenom flags MUST be read as the low BYTE (u8): rain_heavy(0x110) + arctic_water(0x130)
share their 4-byte word with an adjacent packed constant, so a u32 read false-positives
(Codex-confirmed).

READ-ONLY. BUILD-GATED. Every field is read from the located struct; an unreadable
region/province is omitted (never fabricated). A measured-empty active_modifiers /
custom_modifiers list is a REAL read of the allocated empty vector.
"""
import os
import struct
import sys

_HERE = os.path.dirname(os.path.abspath(__file__))
_MR = os.path.dirname(_HERE)
for _p in (_MR, _HERE):
    if _p not in sys.path:
        sys.path.insert(0, _p)

import winmem as wm  # noqa: E402
import fastscan as fs  # noqa: E402
from weather_state_reader import (  # noqa: E402
    EXPECTED_SHA, WMGR_VTABLE_RVA, locate_manager, _isheap,
    MGR_REGION_ARRAY_OFF, PERREGION_STRIDE, REGION_ID_OFF,
)

TEMP_SCALE = 1e5
NWC_EPOCH = 43799999          # absolute_hours = raw - NWC_EPOCH
NWC_NO_CHANGE = 43817520      # sentinel: no scheduled change

# per-region struct offsets
R_TEMP = 0x138
R_PHENOM = {"rain_light": 0x108, "rain_heavy": 0x110, "snow": 0x118,
            "blizzard": 0x120, "arctic_water": 0x130}
R_NWC = 0x150
R_ACTIVE_MODS = 0x30          # PdxVector{begin,u32 size,u32 cap}

# per-province array + struct offsets
MGR_PROVINCE_ARRAY_OFF = 0x10
PERPROVINCE_STRIDE = 0x180
P_MUD = 0x130
P_CUSTOM_MODS = 0x148         # PdxVector{begin,u32 size,u32 cap}


def _vec_size(body, off):
    """Read a PdxVector {begin u64, size u32} size from a struct body slice. Returns
    0 when begin is not a heap pointer (a genuinely empty vector) or size is absurd."""
    if off + 12 > len(body):
        return None
    begin = struct.unpack_from("<Q", body, off)[0]
    size = struct.unpack_from("<I", body, off + 8)[0]
    if size == 0:
        return 0
    if not _isheap(begin) or size > 4096:
        return 0
    return size


def read_region_fields(pid=None, _located=None):
    """{region_id: {temperature, phenomenon[list of active names], next_weather_change,
    active_modifier_count}} for all strategic regions, or None on failure.

    ``_located`` (optional): a pre-resolved (proc, mgr) pair from locate_weather() so
    region+province reads in one capture share a SINGLE CWeatherManager locate. The proc
    is NOT closed here when supplied (the caller owns it)."""
    own = _located is None
    p, mgr = _located if _located is not None else _open_and_locate(pid)
    if p is None:
        return None
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
            body = p.try_read(base, PERREGION_STRIDE)
            if len(body) < R_TEMP + 4:
                continue
            rid = struct.unpack_from("<I", body, REGION_ID_OFF)[0]
            temp_raw = struct.unpack_from("<i", body, R_TEMP)[0]
            phenom = [name for name, off in R_PHENOM.items() if body[off] != 0]
            nwc_raw = struct.unpack_from("<I", body, R_NWC)[0]
            nwc = None if nwc_raw == NWC_NO_CHANGE else (nwc_raw - NWC_EPOCH)
            am = _vec_size(body, R_ACTIVE_MODS)
            out[rid] = {
                "temperature": round(temp_raw / TEMP_SCALE, 3),
                "phenomenon": sorted(phenom),
                "next_weather_change_hours": nwc,
                "active_modifier_count": am,
            }
        return {"manager": mgr, "size": size, "regions": out}
    finally:
        if own:
            p.close()


def read_province_fields(pid=None, _located=None):
    """{province_id: {mud: bool, custom_modifier_count: int}} for all provinces, or
    None. Only provinces with a real allocated struct are returned (fail-closed).

    ``_located`` (optional): a pre-resolved (proc, mgr) pair from locate_weather() so the
    region+province reads share a SINGLE locate. The proc is NOT closed when supplied."""
    own = _located is None
    p, mgr = _located if _located is not None else _open_and_locate(pid)
    if p is None:
        return None
    try:
        hdr = p.try_read(mgr + MGR_PROVINCE_ARRAY_OFF, 16)
        if len(hdr) < 16:
            return None
        begin, size, cap = struct.unpack("<QII", hdr)
        if not _isheap(begin) or not (1 <= size <= 100000):
            return None
        out = {}
        for k in range(size):
            base = begin + k * PERPROVINCE_STRIDE
            body = p.try_read(base, PERPROVINCE_STRIDE)
            if len(body) < P_CUSTOM_MODS + 12:
                continue
            mud = bool(body[P_MUD])
            cm = _vec_size(body, P_CUSTOM_MODS)
            out[k] = {"mud": mud, "custom_modifier_count": cm}
        return {"manager": mgr, "size": size, "provinces": out}
    finally:
        if own:
            p.close()


def locate_weather(pid=None):
    """Open hoi4.exe (build-gated) + locate the CWeatherManager ONCE, returning an open
    (proc, mgr) pair for the caller to thread into read_region_fields(_located=...) +
    read_province_fields(_located=...). The CALLER owns the proc and must close it.
    Returns (None, None) on any failure."""
    return _open_and_locate(pid)


def _open_and_locate(pid):
    """Open hoi4.exe (build-gated) + locate the CWeatherManager. Returns (proc, mgr)
    or (None, None)."""
    if pid is None:
        pid = wm.find_pid("hoi4.exe")
        if pid is None:
            return None, None
    p = wm.Process(pid, "hoi4.exe")
    mb = p.module_base
    if p.fingerprint().get("sha256") != EXPECTED_SHA:
        p.close()
        return None, None
    p.close()
    arena = fs.discover_arena(pid)
    if arena is None:
        return None, None
    mgr = locate_manager(pid, mb, arena)
    if mgr is None:
        return None, None
    return wm.Process(pid, "hoi4.exe"), mgr
