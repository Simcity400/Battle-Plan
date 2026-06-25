"""division_live_reader.py - READ-ONLY live per-DIVISION STATE reader for Battle Plan.

Enumerates every active land division from a running hoi4.exe (CDivision vtable scan, type
4713) and reads the LIVE-STATE fields the map needs to draw movement arrows, in-combat, and
supply state like the game:

  moving                     bool   = movement_progress != 0
  movement_progress          float  = u32[body+0x230] / 100000   (engine metric; can exceed 1.0)
  move_target_province       int    = u32[body+0x18]   (destination while moving)
  movement_source_province   int    = u32[ u64[body+0x1E8] + 0xA4 ]  (source; order-gated)
  in_combat                  bool   = combat_ref_count > 0
  combat_ref_count           int    = u32[body+0x1A4]  (# concurrent land combats)
  army_current_supply_ratio  float  = u32[body+0x600] / 100000  (supply_status; current level)
  max_supply                 float  = u32[body+0x5F0] / 100000  (supply capacity)
  supply_gain                float  = u32[body+0x5F8] / 100000
  fuel                       float  = u32[body+0x610] / 100000  (motorized+; else absent/0)
  experience                 float  = u32[body+0x420] / 100000  (raw division XP)
  experience_tier            str    = green|regular|veteran|elite (derived from raw XP)

Plus the identity fields from the enumerator crack (id, location, owner tag-db-id, org,
strength) so a single pass yields a complete live division record.

Offsets: division_live_offsets.draft.json (build 16022851). Validated against a time-aligned
melted save: movement_progress 1120/1120, experience 1601/1601, max_supply 1604/1604,
army_current_supply_ratio 1596/1596, supply_gain 1396/1396, fuel 144/144,
movement_source_province 1565/1565; in_combat 61/61 vs the live combat reader (0 false pos).
'encircled' is NOT a confirmed body field (no save anchor); an optional explicitly-heuristic
likely_cut_off proxy is provided and clearly labelled.

HARD RULES honored (same as combat_enum.py):
  * READ-ONLY memory (PROCESS_VM_READ | PROCESS_QUERY_INFORMATION); never writes/injects/
    suspends. Passive reads => no MP desync, any cadence.
  * Build-gated: refuses to run unless live hoi4.exe sha256 matches the pinned build.
  * Every fixed-size read is clamped to the VirtualQuery region end (a read straddling a
    region boundary returns 0 for the whole span - known bug).
  * Arena-scoped scan window (0x1F600000000..0x1FA00000000) for division bodies, clamped to
    region end, considerate of concurrent scanners and the live game. Generic heap test
    0x10000000000 <= v < 0x7FF000000000 for pointer validity.
  * ASCII-only output.

Self-contained minimal ctypes helper (adapted from the proven combat/anchor_dynamic pattern);
does NOT edit winmem.py / locate.py / the enumerator. When productionized these offsets merge
into the per-build layout.
"""

import ctypes as C
import struct
import hashlib
import os
import sys
from ctypes import wintypes as W

# --------------------------------------------------------------------------------------
# Build gate + solved constants (module-relative; see division_live_offsets.draft.json)
# --------------------------------------------------------------------------------------
EXPECTED_SHA256 = "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120"
PROC_NAME = "hoi4.exe"

DIV_VT_OFF        = 0x2933F00     # CDivision vtable (module-relative)
MOVE_ORDER_VT_OFF = 0x294AE78     # per-division movement/order object vtable (module-relative)
DIVISION_TYPE     = 4713          # type tag @ body+0x08

# CDivision body field offsets (build 16022851)
OFF_TYPE        = 0x08            # u32 == 4713
OFF_ID          = 0x0C           # u32 division id
OFF_LOCATION    = 0x18           # u32 province (== move DESTINATION while moving)
OFF_COMBAT_REF  = 0x1A4          # u32 count of concurrent land combats (0 => not in combat)
OFF_MOVE_ORDER  = 0x1E8          # u64 -> movement/order object (vtable MOVE_ORDER_VT_OFF)
OFF_MOVE_PROG   = 0x230          # u32 fixed x100000 movement_progress (0 => stationary)
OFF_OWNER       = 0x1C8          # u32 owner tag-db-id (from enumerator crack)
OFF_STRENGTH    = 0x410          # u32 fixed x100000 (from enumerator crack)
OFF_EXPERIENCE  = 0x420          # u32 fixed x100000 division-level experience
OFF_ORG         = 0x418          # u32 fixed x100000 (from enumerator crack)
OFF_MAX_SUPPLY  = 0x5F0          # u32 fixed x100000 supply capacity
OFF_SUPPLY_GAIN = 0x5F8          # u32 fixed x100000
OFF_SUPPLY_RATIO= 0x600          # u32 fixed x100000 current supply level (supply_status)
OFF_FUEL        = 0x610          # u32 fixed x100000 (motorized+; else 0)

# movement/order sub-object
OFF_ORDER_SOURCE = 0xA4          # u32 source province (the save 'previous')

FP = 100000.0                    # fixed-point scale

# body span to read per division (covers all fields above)
BODY_SPAN = 0x620

# heap range / arena window
HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000
ARENA_LO = 0x1F600000000
ARENA_HI = 0x1FA00000000

MAX_DIVISIONS = 20000            # fail-closed sanity bound

# Sane per-field bounds (Codex-recommended; reject garbage rather than surface it).
SANE = {
    "movement_progress": (0.0, 1.0e6),
    "experience":        (0.0, 1.0e5),
    "max_supply":        (0.0, 1.0e4),
    "army_current_supply_ratio": (0.0, 1.0e4),
    "supply_gain":       (0.0, 100.0),
    "fuel":              (0.0, 1.0e6),
    "organisation":      (0.0, 1000.0),
    "strength":          (0.0, 1.0e4),
}

# the game division experience tier cutoffs (raw XP). Approximate engine defines; the raw XP value
# is ground truth - adjust thresholds to the active defines if needed.
XP_TIERS = [(0, "green"), (400, "regular"), (1600, "veteran"), (4000, "elite")]


def xp_tier(raw_xp):
    """Map raw division experience -> green|regular|veteran|elite."""
    tier = "green"
    for cut, name in XP_TIERS:
        if raw_xp >= cut:
            tier = name
    return tier


# --------------------------------------------------------------------------------------
# Minimal read-only process access (self-contained)
# --------------------------------------------------------------------------------------
_k32 = C.WinDLL("kernel32", use_last_error=True)


class _MBI(C.Structure):
    _fields_ = [("BaseAddress", C.c_void_p), ("AllocationBase", C.c_void_p),
                ("AllocationProtect", W.DWORD), ("__a", W.DWORD),
                ("RegionSize", C.c_size_t), ("State", W.DWORD),
                ("Protect", W.DWORD), ("Type", W.DWORD)]


class _PE32(C.Structure):
    _fields_ = [("dwSize", W.DWORD), ("cntUsage", W.DWORD), ("th32ProcessID", W.DWORD),
                ("th32DefaultHeapID", C.POINTER(C.c_ulong)), ("th32ModuleID", W.DWORD),
                ("cntThreads", W.DWORD), ("th32ParentProcessID", W.DWORD),
                ("pcPriClassBase", C.c_long), ("dwFlags", W.DWORD), ("szExeFile", W.WCHAR * 260)]


class _ME32(C.Structure):
    _fields_ = [("dwSize", W.DWORD), ("th32ModuleID", W.DWORD), ("th32ProcessID", W.DWORD),
                ("GlblcntUsage", W.DWORD), ("ProccntUsage", W.DWORD),
                ("modBaseAddr", C.POINTER(C.c_byte)), ("modBaseSize", W.DWORD),
                ("hModule", W.HMODULE), ("szModule", W.WCHAR * 256), ("szExePath", W.WCHAR * 260)]


class DivisionLiveReader(object):
    """Read-only handle onto a running hoi4.exe with the division-live layout pinned."""

    def __init__(self, proc_name=PROC_NAME):
        self.proc_name = proc_name
        self.pid = self._find_pid(proc_name)
        if self.pid is None:
            raise RuntimeError("process not found: %s" % proc_name)
        self.h = _k32.OpenProcess(0x10 | 0x400, False, self.pid)   # VM_READ | QUERY_INFORMATION
        if not self.h:
            raise RuntimeError("OpenProcess failed (err %d)" % C.get_last_error())
        self.module_base, self.module_size, self.exe_path = self._module(self.pid, proc_name)
        if self.module_base is None:
            raise RuntimeError("module base not found: %s" % proc_name)
        self.div_vt = self.module_base + DIV_VT_OFF
        self.move_order_vt = self.module_base + MOVE_ORDER_VT_OFF
        self._regions_cache = None

    # ---- process / module discovery ----
    @staticmethod
    def _find_pid(name):
        s = _k32.CreateToolhelp32Snapshot(0x2, 0)
        pe = _PE32(); pe.dwSize = C.sizeof(pe); pid = None
        if _k32.Process32FirstW(s, C.byref(pe)):
            while True:
                if pe.szExeFile.lower() == name.lower():
                    pid = pe.th32ProcessID; break
                if not _k32.Process32NextW(s, C.byref(pe)):
                    break
        _k32.CloseHandle(s)
        return pid

    @staticmethod
    def _module(pid, name):
        s = _k32.CreateToolhelp32Snapshot(0x8 | 0x10, pid)
        me = _ME32(); me.dwSize = C.sizeof(me); base = size = path = None
        if _k32.Module32FirstW(s, C.byref(me)):
            while True:
                if me.szModule.lower() == name.lower():
                    base = C.cast(me.modBaseAddr, C.c_void_p).value
                    size = me.modBaseSize
                    path = me.szExePath
                    break
                if not _k32.Module32NextW(s, C.byref(me)):
                    break
        _k32.CloseHandle(s)
        return base, size, path

    # ---- raw reads (clamped) ----
    def _rpm(self, addr, n):
        buf = (C.c_char * n)(); got = C.c_size_t(0)
        if not _k32.ReadProcessMemory(self.h, C.c_void_p(addr), buf, n, C.byref(got)):
            return None
        return bytes(buf[:got.value])

    def _region_end(self, addr):
        mbi = _MBI()
        if _k32.VirtualQueryEx(self.h, C.c_void_p(addr), C.byref(mbi), C.sizeof(mbi)) == 0:
            return None
        base = mbi.BaseAddress or addr
        return base + (mbi.RegionSize or 0x1000)

    def _read_clamped(self, addr, n):
        end = self._region_end(addr)
        if end is not None and addr + n > end:
            n = max(0, end - addr)
        if n == 0:
            return b""
        return self._rpm(addr, n) or b""

    def _enum_regions(self):
        """Committed, readable, non-executable heap regions (cached)."""
        if self._regions_cache is not None:
            return self._regions_cache
        out = []
        addr = 0x10000
        mbi = _MBI()
        while addr < HEAP_HI:
            if _k32.VirtualQueryEx(self.h, C.c_void_p(addr), C.byref(mbi), C.sizeof(mbi)) == 0:
                addr += 0x1000
                continue
            base = mbi.BaseAddress or addr
            size = mbi.RegionSize or 0x1000
            nxt = base + size
            committed = (mbi.State == 0x1000)
            not_guard = (mbi.Protect & 0x100) == 0
            readable = (mbi.Protect & 0xFF) in (0x02, 0x04, 0x08, 0x20, 0x40, 0x80)
            if committed and not_guard and readable and size <= (1 << 31) and HEAP_LO <= base < HEAP_HI:
                out.append((base, size))
            addr = nxt if nxt > addr else addr + 0x1000
        self._regions_cache = out
        return out

    # ---- build gate ----
    def verify_build(self):
        """Return (ok, sha256). Hashes the on-disk exe at the loaded module path."""
        try:
            with open(self.exe_path, "rb") as f:
                digest = hashlib.sha256(f.read()).hexdigest()
        except Exception as e:
            return False, "exe-unreadable:%s" % e
        return (digest == EXPECTED_SHA256), digest

    # ---- field decoders ----
    @staticmethod
    def _u32(b, o):
        return struct.unpack_from("<I", b, o)[0]

    @staticmethod
    def _u64(b, o):
        return struct.unpack_from("<Q", b, o)[0]

    def _fixed(self, b, o, field):
        """u32/100000 with a sane-bounds guard; returns None if out of range (garbage)."""
        v = self._u32(b, o) / FP
        lo, hi = SANE.get(field, (-1e30, 1e30))
        return v if lo <= v <= hi else None

    def _read_source_province(self, body_addr):
        """movement_source_province = u32[ u64[body+0x1E8] + 0xA4 ], gated on the move-order
        object's vtable. Returns province id or None when no/invalid order."""
        ptr = None
        hdr = self._rpm(body_addr + OFF_MOVE_ORDER, 8)
        if hdr and len(hdr) == 8:
            ptr = struct.unpack("<Q", hdr)[0]
        if ptr is None or not (HEAP_LO <= ptr < HEAP_HI):
            return None
        sub = self._read_clamped(ptr, OFF_ORDER_SOURCE + 4)
        if len(sub) < OFF_ORDER_SOURCE + 4:
            return None
        if struct.unpack_from("<Q", sub, 0)[0] != self.move_order_vt:   # vtable gate
            return None
        prov = struct.unpack_from("<I", sub, OFF_ORDER_SOURCE)[0]
        return prov if 0 < prov < 0x10000 else None

    # ---- enumeration ----
    def enumerate_divisions(self, resolve_owner_tag=None):
        """Return a list of dicts, one per live division:
            { id, owner_tag_db_id, owner (optional via resolve_owner_tag),
              location, move_target_province, movement_source_province, moving,
              movement_progress, in_combat, combat_ref_count,
              army_current_supply_ratio, max_supply, supply_gain, fuel,
              organisation, strength, experience, experience_tier }

        Enumeration: scan committed heap for the CDivision vtable qword
        (MODULE+0x2933F00), keep bodies with u32[+0x08] == 4713. Each field is read from a
        single body snapshot, with sane-bounds guards (out-of-range -> field is None).

        resolve_owner_tag: optional callable tag_db_id -> 'TAG' (e.g. tag_table.TagTable.tag).
        """
        vt_pat = struct.pack("<Q", self.div_vt)
        seen = {}                       # id -> body_addr (dedupe)
        out = []
        for base, size in self._enum_regions():
            blob = self._rpm(base, size)
            if not blob:
                continue
            s = 0
            while True:
                j = blob.find(vt_pat, s)
                if j < 0:
                    break
                cand = base + j
                s = j + 8
                b = self._read_clamped(cand, BODY_SPAN)
                if len(b) < BODY_SPAN:
                    continue
                if self._u32(b, OFF_TYPE) != DIVISION_TYPE:
                    continue
                did = self._u32(b, OFF_ID)
                if did in seen:
                    continue
                seen[did] = cand
                rec = self._decode_body(cand, b, resolve_owner_tag)
                out.append(rec)
                if len(out) > MAX_DIVISIONS:
                    raise RuntimeError("division count exceeds sanity bound (%d) - layout likely broken" % MAX_DIVISIONS)
        return out

    def _decode_body(self, body_addr, b, resolve_owner_tag):
        mp = self._fixed(b, OFF_MOVE_PROG, "movement_progress")
        moving = (self._u32(b, OFF_MOVE_PROG) != 0)
        combat_ref = self._u32(b, OFF_COMBAT_REF)
        loc = self._u32(b, OFF_LOCATION)
        owner_id = self._u32(b, OFF_OWNER)
        raw_xp = self._fixed(b, OFF_EXPERIENCE, "experience")
        rec = {
            "id": self._u32(b, OFF_ID),
            "owner_tag_db_id": owner_id,
            "location": loc,
            "move_target_province": loc if moving else None,
            "movement_source_province": self._read_source_province(body_addr) if moving else None,
            "moving": moving,
            "movement_progress": mp,
            "in_combat": combat_ref > 0,
            "combat_ref_count": combat_ref,
            "army_current_supply_ratio": self._fixed(b, OFF_SUPPLY_RATIO, "army_current_supply_ratio"),
            "max_supply": self._fixed(b, OFF_MAX_SUPPLY, "max_supply"),
            "supply_gain": self._fixed(b, OFF_SUPPLY_GAIN, "supply_gain"),
            "fuel": self._fixed(b, OFF_FUEL, "fuel"),
            "organisation": self._fixed(b, OFF_ORG, "organisation"),
            "strength": self._fixed(b, OFF_STRENGTH, "strength"),
            "experience": raw_xp,
            "experience_tier": xp_tier(raw_xp) if raw_xp is not None else None,
        }
        if resolve_owner_tag is not None:
            try:
                rec["owner"] = resolve_owner_tag(owner_id)
            except Exception:
                rec["owner"] = None
        return rec

    def likely_cut_off(self, rec, supply_ratio_threshold=3.0):
        """OPTIONAL, EXPLICITLY-HEURISTIC proxy for 'encircled / cut off from supply'. This is
        NOT a confirmed memory field (the save has no encircled bool and no division-body bool
        was found). It flags a division as a CANDIDATE cut-off when its current supply level is
        very low and it is not gaining supply. Treat as a hint, not ground truth; combining with
        a supply-network / province-control layer (not in this crack) would make it reliable."""
        sr = rec.get("army_current_supply_ratio")
        sg = rec.get("supply_gain")
        if sr is None:
            return False
        return sr < supply_ratio_threshold and (sg is None or sg <= 0.0)

    def close(self):
        if self.h:
            _k32.CloseHandle(self.h)
            self.h = None


def _try_combat_crosscheck():
    """Optional: import the repo combat reader and return the live in-combat division-id set.
    Returns (set, n_combats) or (None, 0) if the combat package is unavailable."""
    repo_mr = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))   # .../memory_reader
    if repo_mr not in sys.path:
        sys.path.insert(0, repo_mr)
    try:
        from combat.combat_enum import CombatReader
    except Exception:
        return None, 0
    try:
        cr = CombatReader()
        ok, _ = cr.verify_build()
        if not ok:
            cr.close(); return None, 0
        combats = cr.enumerate_land_combats(with_defenders=True)
        s = set()
        for c in combats:
            for k in ("attacker_units", "defender_units"):
                if c.get(k):
                    s.update(c[k])
        cr.close()
        return s, len(combats)
    except Exception:
        return None, 0


def main():
    r = DivisionLiveReader()
    ok, sha = r.verify_build()
    print("hoi4.exe pid=%d module_base=0x%X" % (r.pid, r.module_base))
    print("build sha256: %s" % sha)
    print("build gate: %s" % ("OK (matches pinned build)" if ok else "MISMATCH - refusing to trust offsets"))
    if not ok:
        print("Refusing to enumerate on an unverified build (offsets are build-specific).")
        r.close(); return

    divs = r.enumerate_divisions()
    moving = [d for d in divs if d["moving"]]
    in_combat = [d for d in divs if d["in_combat"]]
    print("\ndivisions: %d  (moving=%d, in_combat=%d)" % (len(divs), len(moving), len(in_combat)))

    # in_combat cross-check vs the live combat reader (ground truth)
    cset, ncomb = _try_combat_crosscheck()
    if cset is not None:
        reader_set = set(d["id"] for d in in_combat)
        fp = reader_set - cset
        fn = cset - reader_set
        print("combat cross-check: %d land combats, %d in-combat ids (reader_flag set=%d) "
              "| false_pos=%d false_neg=%d"
              % (ncomb, len(cset), len(reader_set), len(fp), len(fn)))
    else:
        print("combat cross-check: combat package unavailable (skipped)")

    print("\nsample moving divisions:")
    for d in moving[:6]:
        print("  id=%d %s->%s prog=%.3f org=%.1f str=%.1f sup=%.1f/%.1f xp=%.0f(%s) fuel=%s combat=%d"
              % (d["id"], d["movement_source_province"], d["move_target_province"],
                 d["movement_progress"] or 0, d["organisation"] or 0, d["strength"] or 0,
                 d["army_current_supply_ratio"] or 0, d["max_supply"] or 0,
                 d["experience"] or 0, d["experience_tier"], d["fuel"], d["combat_ref_count"]))
    print("\nsample in-combat divisions:")
    for d in in_combat[:6]:
        print("  id=%d prov=%d combat_ref_count=%d str=%.1f org=%.1f sup=%.1f"
              % (d["id"], d["location"], d["combat_ref_count"], d["strength"] or 0,
                 d["organisation"] or 0, d["army_current_supply_ratio"] or 0))
    cutoff = [d for d in divs if r.likely_cut_off(d)]
    print("\nlikely_cut_off (HEURISTIC proxy, NOT confirmed encircled): %d divisions" % len(cutoff))
    r.close()


if __name__ == "__main__":
    main()
