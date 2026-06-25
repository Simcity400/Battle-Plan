"""combat_enum.py - READ-ONLY live ACTIVE-LAND-COMBAT enumerator for Battle Plan.

Lists active land battles from a running hoi4.exe: combat_id, location (province),
combat age (day/duration), attacker division set, and the reachable participant union.
Designed for a high-frequency sampling loop that reconstructs a battle log from the
appear / persist / disappear lifecycle of these combat objects.

Solved + validated 2026-06-24 against a time-aligned melted save (see
combat_offsets.draft.json). Enumeration / combat_id / location / day / duration all 13/13
exact; ATTACKER division set 13/13 exact AND DEFENDER division set 13/13 exact - BOTH read
from live memory (the prior 'defender residual' is SOLVED, and the prior attacker 12/13
longest-run heuristic is superseded). The exact per-side membership comes from the combat-sides
registry and the two side objects, NOT from the save (the save is a validation backstop only).

HARD RULES honored:
  * READ-ONLY memory (PROCESS_VM_READ | PROCESS_QUERY_INFORMATION); never writes / injects /
    suspends. Reads are passive => no MP desync, any cadence.
  * Build-gated: refuses to run unless the live hoi4.exe sha256 matches the pinned build,
    so stale offsets are never used blind.
  * Every fixed-size read is clamped to the VirtualQuery region end (a read straddling a
    region boundary returns 0 for the whole span - known bug).
  * ASCII-only output.

This module does NOT edit winmem.py / locate.py; it carries its own minimal ctypes helper
(adapted from the proven re_scratch/anchor_dynamic.py pattern) so the combat package is
self-contained. When the reader is productionized, the offsets here can be merged into the
per-build layout and this can call the shared winmem instead.
"""

import ctypes as C
import struct
import hashlib
from ctypes import wintypes as W

# --------------------------------------------------------------------------------------
# Build gate + solved constants (module-relative; see combat_offsets.draft.json)
# --------------------------------------------------------------------------------------
EXPECTED_SHA256 = "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120"
PROC_NAME = "hoi4.exe"

# module-relative vtables / type ids
CLAND_COMBAT_VT_OFF = 0x2980CB8   # CLandCombat vtable (module-relative)
DIVISION_VT_OFF     = 0x2933F00   # CDivision vtable   (module-relative)
SIDE_VT_OFF         = 0x2980AC0   # CCombatSide vtable (attacker AND defender side objects)
REG_VT_OFF          = 0x2980BD8   # combat-sides REGISTRY vtable (links the two sides)
LAND_COMBAT_TYPE    = 62          # type tag @ combat_body+0x08
DIVISION_TYPE       = 4713        # type tag @ division_body+0x08

# --- NAVAL combat (CNavalCombat / CNavalCombatant) - solved 2026-06-25, validated 3/3 ---
# Discovered via RTTI off the proven land vtables (the same RTTI walk reproduces the land
# vtables 0x2980CB8/0x2980BD8/0x2980AC0 exactly). Naval is a sibling structure with its OWN
# vtable family; it does NOT reuse the land registry/side vtables (a heap scan for the land
# registry vtable returns only the 8 land combats). See naval_combat_offsets.draft.json.
NAVAL_COMBAT_VT_OFF = 0x29B6660   # CNavalCombat body vtable (module-relative)
NAVAL_REG_VT_OFF    = 0x29B6588   # CNavalCombat nested combat-sides REGISTRY vtable
NAVAL_SIDE_VT_OFF   = 0x29BC908   # CNavalCombatant (per-side) vtable
CCOUNTRY_VT_OFF     = 0x27C0E80   # CCountry vtable (for an optional cross-check of side country)
NAVAL_COMBAT_TYPE   = 4713        # registered type tag for a naval combat handle (low u32 of the
                                  # body+0x08 packed {type, combat_id}). NOTE: this collides with
                                  # the CDivision/CArmy type id; the BODY VTABLE is the real
                                  # discriminator, never the type alone (Codex-confirmed).

# CNavalCombat body field offsets (deltas vs land are documented in the draft json)
OFF_N_TYPEID   = 0x08   # u64 packed: low u32 == 4713 (type), high u32 == combat_id
OFF_N_SIDE_A   = 0x18   # u64 -> SIDE_A object (CNavalCombatant); == registry+0x28 (ATTACKER)
OFF_N_SIDE_B   = 0x20   # u64 -> SIDE_B object (CNavalCombatant); == registry+0x30 (DEFENDER)
OFF_N_FRONT    = 0x28   # u64 -> CProvince (sea-zone front; province id @+0xA4)

# CNavalCombatant (side) field offsets
OFF_NS_REG       = 0x18   # u64 -> nested naval registry (back-ptr, vtable NAVAL_REG_VT)
OFF_NS_TF_DATA   = 0x20   # u64 -> std::vector<CTaskForce*> data (populated on ATTACKER side only)
OFF_NS_TF_SIZE   = 0x2C   # u32 task-force vector size
OFF_NS_CTRY_DATA = 0x68   # u64 -> vector data; each elem = packed country handle (low u32 = tag-db-id)
OFF_NS_CTRY_SIZE = 0x74   # u32 country-vector size (high u32 of the {cap,size} qword @+0x70)
OFF_NS_FEX_DATA  = 0xC8   # u64 -> vector data; each elem = CFEXGroup* (per-country fleet-in-combat)
OFF_NS_FEX_SIZE  = 0xD4   # u32 CFEX-group vector size (high u32 of the {cap,size} qword @+0xD0)
OFF_NS_LEADER    = 0x110  # u64 -> CNavyLeader (admiral); present on the ATTACKER side, null otherwise

# CFEXGroup (per-side, per-country fleet-in-combat) field offsets
# The CFEXGroup embeds a std::vector<CFEXMember*> AT +0x08: data ptr @+0x08, and a packed
# {cap (low u32), size (high u32)} qword @+0x10. So:
#   +0x10 low u32  == vector CAPACITY  (this is the "ships_raw" the prior pass logged; the
#                     12-sub fleets over-allocate to 13, hence the spurious +1 - it is NOT a ship)
#   +0x14          == vector SIZE      (the high u32 of the +0x10 qword) == the REAL ship count
#                     shown in the UI. Walking the +0x08 array yields exactly SIZE CFEXMember*.
OFF_FEX_SHIPS_UI    = 0x14  # u32 = vector SIZE = real ship/sub count (anchor-matching). USE THIS.
OFF_FEX_SHIPS_RAW   = 0x10  # u32 = vector CAPACITY (>= size; the over-alloc slack). NOT a ship count.
OFF_FEX_MEMBERS     = 0x08  # u64 -> std::vector<CFEXMember*> data ptr (the per-ship object array)

# CFEXMember (RTTI .?AVCFEXMember@@) - the per-ship in-combat participant/view-model wrapper.
# This is what the +0x08 ship array points at; it is NOT a raw CShip. Layout is STABLE across
# ships (the three embedded std::strings are fixed 0x20-byte MSVC strings; SSO vs heap does not
# shift later offsets). Solved + count-validated 8/8 sides 2026-06-25. SEMANTICS of the numeric
# combat-state fields (HP/org) are NOT locked - they need a detailed-naval-menu anchor (see the
# draft json "screenshot_requests"); the fields below that are NAMED are validated, the ones
# marked CANDIDATE are structurally located but semantically unconfirmed.
SHIP_VT_OFF         = 0x29F9608  # CFEXMember vtable (module-relative); the ship-array element type
OFF_SHIP_TYPEID     = 0x08  # u64 packed {low u32 = unit type tag (0x33==51 for a SUBMARINE warship),
                            # high u32 = ship INSTANCE id (e.g. 3458)}. For a CONVOY ship BOTH are 0
                            # (convoys are pool entities with no per-ship instance id). The instance
                            # id is the key to resolve the authoritative CShip (HP/org) in a global
                            # ship registry - that registry is a SEPARATE crack, not walked here.
OFF_SHIP_OWNER_TAG  = 0x20  # u32 (low u32 of the qword) = owner country tag-db-id (3/37/35 validated)
OFF_SHIP_MAXSTAT    = 0x30  # u32 = 32000000 for the Early Submarine hull, 0 for a convoy. CONSTANT
                            # across same-type ships => a STATIC per-type value (max-HP or build-cost
                            # in milli-fixed-point), NOT current HP. CANDIDATE (needs the menu anchor).
OFF_SHIP_NAME       = 0x48  # std::string ship NAME ("Shch-121", "ARM Rio Fuerte"); 0x20-byte MSVC str
OFF_SHIP_TYPE       = 0x68  # std::string type display name ("Early Submarine" / "Convoy")
OFF_SHIP_ARCHETYPE  = 0x88  # std::string archetype key ("submarine" / "convoy")
OFF_SHIP_GROUP_BACK = 0xE8  # u64 -> owning CFEXGroup (back-ptr; validated by RTTI)
OFF_SHIP_SLOT       = 0x120 # u32 = monotonic battle slot/line index (2,3,4,... per ship). CANDIDATE
                            # (line position in the battle; not the same as positioning %).
OFF_SHIP_CAPTAIN    = 0x140 # u64 -> CCharacter (ship captain) when present; null/other-union otherwise
# NOTE: +0xF8/+0x100 read 100000 (=100.000% fixed-point) on a MINORITY of ships and 0 on the rest;
# 200000/400000 also seen. This is a per-ship COMBAT-STATE value (engagement/targeting/positioning
# weight), NOT HP/org (a >100% value and the sparse population both disqualify HP). Left UNNAMED.

MAX_SHIPS_PER_GROUP = 1000  # fail-closed sanity bound on a single group's ship vector size

# CNavyLeader: inline/heap std::string name @+0x20 (SSO if <=15 chars else heap ptr)
OFF_NL_NAME = 0x20

# combat-sides registry (CNavalCombat nested registry, vtable NAVAL_REG_VT)
OFF_NREG_HANDLE   = 0x18  # u64 packed {type=4713, combat_id} identity back-ref (== body+0x08)
OFF_NREG_ATK_SIDE = 0x28  # u64 -> ATTACKER side (authoritative; == body+0x18)
OFF_NREG_DEF_SIDE = 0x30  # u64 -> DEFENDER side (authoritative; == body+0x20)
OFF_NREG_FRONT    = 0x38  # u64 -> CProvince front (== body+0x28)

# CLandCombat body field offsets
OFF_TYPE      = 0x08   # u32 == 62
OFF_COMBAT_ID = 0x0C   # u32 combat id
OFF_ATK_SIDE  = 0x18   # u64 -> ATTACKER side object (CCombatSide)
OFF_FRONT     = 0x28   # u64 -> front/border object (holds location)
OFF_DURATION  = 0x30   # u32 (== day for all samples; semantics provisional)
OFF_DAY       = 0x34   # u32 combat age (days)

# front object
OFF_FRONT_LOC = 0xA4   # u32 province id

# side object (CCombatSide, vtable MODULE+0x2980AC0) - attacker AND defender share this
OFF_SIDE_REG   = 0x18  # u64 -> combat-sides registry (back-ptr)
OFF_SIDE_DATA  = 0x20  # u64 -> CDivision** participant array (side-owned storage)
OFF_SIDE_CAP   = 0x28  # u32 capacity (>= size)
OFF_SIDE_SIZE  = 0x2C  # u32 size == participant (division) count -> read EXACTLY this many

# combat-sides registry (vtable MODULE+0x2980BD8)
OFF_REG_HANDLE   = 0x18  # u64 packed (combat_id<<32)|62  -> identity back-ref
OFF_REG_ATK_SIDE = 0x28  # u64 -> attacker side (== combat_body+0x18)
OFF_REG_DEF_SIDE = 0x30  # u64 -> DEFENDER side object  <-- the solved defender source
OFF_REG_FRONT    = 0x38  # u64 -> front/border object

# division body
OFF_DIV_TYPE = 0x08
OFF_DIV_ID   = 0x0C

# safety bounds
MAX_SIDE_UNITS = 2000  # fail-closed sanity bound on a side's participant count

# heap range / sanity
HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000
MAX_COMBATS = 4000          # sanity bound (RESEARCH section 3 fail-closed)

# --------------------------------------------------------------------------------------
# Minimal read-only process access (self-contained; adapted from anchor_dynamic.py)
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


class CombatReader(object):
    """Read-only handle onto a running hoi4.exe with the combat layout pinned."""

    def __init__(self, proc_name=PROC_NAME):
        self.proc_name = proc_name
        self.pid = self._find_pid(proc_name)
        if self.pid is None:
            raise RuntimeError("process not found: %s" % proc_name)
        # PROCESS_VM_READ (0x10) | PROCESS_QUERY_INFORMATION (0x400) - read-only
        self.h = _k32.OpenProcess(0x10 | 0x400, False, self.pid)
        if not self.h:
            raise RuntimeError("OpenProcess failed (err %d)" % C.get_last_error())
        self.module_base, self.module_size, self.exe_path = self._module(self.pid, proc_name)
        if self.module_base is None:
            raise RuntimeError("module base not found: %s" % proc_name)
        self.cland_vt = self.module_base + CLAND_COMBAT_VT_OFF
        self.div_vt = self.module_base + DIVISION_VT_OFF
        self.side_vt = self.module_base + SIDE_VT_OFF
        self.reg_vt = self.module_base + REG_VT_OFF
        # naval combat sibling vtables (solved 2026-06-25)
        self.cnaval_vt = self.module_base + NAVAL_COMBAT_VT_OFF
        self.nreg_vt = self.module_base + NAVAL_REG_VT_OFF
        self.nside_vt = self.module_base + NAVAL_SIDE_VT_OFF
        self.ccountry_vt = self.module_base + CCOUNTRY_VT_OFF
        self.ship_vt = self.module_base + SHIP_VT_OFF   # CFEXMember (per-ship object)
        self._div_cache = {}
        self._regions_cache = None
        # telemetry: count of rejected side reads by reason (Codex-recommended)
        self.reject_stats = {}

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

    def u32(self, addr):
        b = self._rpm(addr, 4)
        return None if not b or len(b) < 4 else struct.unpack("<I", b)[0]

    def u64(self, addr):
        b = self._rpm(addr, 8)
        return None if not b or len(b) < 8 else struct.unpack("<Q", b)[0]

    def _region_end(self, addr):
        """End (exclusive) of the committed region containing addr, or None."""
        mbi = _MBI()
        if _k32.VirtualQueryEx(self.h, C.c_void_p(addr), C.byref(mbi), C.sizeof(mbi)) == 0:
            return None
        base = mbi.BaseAddress or addr
        return base + (mbi.RegionSize or 0x1000)

    def _read_clamped(self, addr, n):
        """RPM clamped to the VirtualQuery region end (a fixed-size read straddling a region
        boundary returns 0/None for the whole span - known bug). Returns up to region end."""
        end = self._region_end(addr)
        if end is not None and addr + n > end:
            n = max(0, end - addr)
        if n == 0:
            return b""
        return self._rpm(addr, n) or b""

    def _note_reject(self, reason):
        self.reject_stats[reason] = self.reject_stats.get(reason, 0) + 1

    def _enum_regions(self):
        """Committed, readable, non-executable heap regions (cached per reader)."""
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
        """Return (ok, sha256). Hashes the on-disk exe at the loaded module path.
        Codex note: ideally also validate the loaded module bytes, not just the file
        (Steam can update the file under a running process). Hashing the file is the
        cheap first gate; the offsets only apply when this matches."""
        try:
            with open(self.exe_path, "rb") as f:
                digest = hashlib.sha256(f.read()).hexdigest()
        except Exception as e:
            return False, "exe-unreadable:%s" % e
        return (digest == EXPECTED_SHA256), digest

    # ---- division resolution ----
    def _div_id(self, body_addr):
        if body_addr in self._div_cache:
            return self._div_cache[body_addr]
        b = self._rpm(body_addr, 0x10)
        rid = None
        if b and len(b) >= 0x10:
            if struct.unpack_from("<Q", b, 0)[0] == self.div_vt and \
               struct.unpack_from("<I", b, OFF_DIV_TYPE)[0] == DIVISION_TYPE:
                rid = struct.unpack_from("<I", b, OFF_DIV_ID)[0]
        self._div_cache[body_addr] = rid
        return rid

    def _resolve_unit_ptr(self, ptr):
        """A side's participant array element points at (CDivision_body) or
        (CDivision_body - 0x10). Return div_id or None."""
        if not (HEAP_LO <= ptr < HEAP_HI):
            return None
        d = self._div_id(ptr)
        if d is not None:
            return d
        return self._div_id(ptr + 0x10)

    def _read_side_size_data(self, side):
        """Validate a side object (CCombatSide, vtable side_vt) and return (size, data_ptr)
        or (None, None) on any failure (with a recorded reject reason)."""
        if not side or not (HEAP_LO <= side < HEAP_HI):
            self._note_reject("no_side"); return None, None
        if self.u64(side) != self.side_vt:
            self._note_reject("bad_side_vtable"); return None, None
        data = self.u64(side + OFF_SIDE_DATA)
        cap = self.u32(side + OFF_SIDE_CAP)
        size = self.u32(side + OFF_SIDE_SIZE)
        if size is None or cap is None:
            self._note_reject("hdr_read_fail"); return None, None
        if size > cap:
            self._note_reject("size_gt_cap"); return None, None
        if size > MAX_SIDE_UNITS:
            self._note_reject("size_sane_bound"); return None, None
        return size, data

    def _side_units(self, side):
        """Exact participant division ids of a combat side object (CCombatSide).

        Layout (solved 2026-06-24, build-stable, allocation-invariant):
            side+0x20 = CDivision** data ; side+0x28 = u32 capacity ; side+0x2C = u32 size.
        Reads EXACTLY size pointers (region-clamped), resolves each to a CDivision, and
        DOUBLE-READS the {data,cap,size} header to defend against a vector realloc/mutation
        racing the read (Codex-recommended). Returns sorted unique ids, or None (fail-closed)
        on any inconsistency. Validated 13/13 vs the save for BOTH attacker and defender sides.
        """
        size, data = self._read_side_size_data(side)
        if size is None:
            return None
        if size == 0:
            return []
        if not data or not (HEAP_LO <= data < HEAP_HI):
            self._note_reject("bad_data_ptr"); return None
        blob = self._read_clamped(data, size * 8)
        if len(blob) < size * 8:
            self._note_reject("short_read"); return None
        ids = []
        for i in range(size):
            d = self._resolve_unit_ptr(struct.unpack_from("<Q", blob, i * 8)[0])
            if d is None:
                self._note_reject("non_division_element"); return None
            ids.append(d)
        # double-read revalidation: header (and data ptr) must be unchanged
        size2, data2 = self._read_side_size_data(side)
        if size2 != size or data2 != data:
            self._note_reject("race_changed"); return None
        return sorted(set(ids))

    def _combat_sides(self, body):
        """Return (attacker_side_ptr, defender_side_ptr, registry_ptr) for a combat body,
        with the registry identity back-ref ({combat_id,62}) verified. Any field that fails
        identity returns None for that slot."""
        cid = self.u32(body + OFF_COMBAT_ID)
        atk_side = self.u64(body + OFF_ATK_SIDE)
        if not atk_side or not (HEAP_LO <= atk_side < HEAP_HI) or self.u64(atk_side) != self.side_vt:
            return None, None, None
        reg = self.u64(atk_side + OFF_SIDE_REG)
        if not reg or not (HEAP_LO <= reg < HEAP_HI) or self.u64(reg) != self.reg_vt:
            return atk_side, None, None
        # registry identity: high u32 == combat_id, low u32 == 62
        handle = self.u64(reg + OFF_REG_HANDLE)
        if handle is None or (handle & 0xFFFFFFFF) != LAND_COMBAT_TYPE or ((handle >> 32) & 0xFFFFFFFF) != cid:
            return atk_side, None, reg
        def_side = self.u64(reg + OFF_REG_DEF_SIDE)
        if not def_side or not (HEAP_LO <= def_side < HEAP_HI) or self.u64(def_side) != self.side_vt:
            def_side = None
        return atk_side, def_side, reg

    # ---- enumeration ----
    def enumerate_land_combats(self, with_defenders=True, **_legacy):
        """Return a list of dicts, one per active land combat:
            { combat_id, body_addr, location, day, duration,
              attacker_units:[div_id,...],
              defender_units:[div_id,...]   # only if with_defenders (default True)
            }
        Both attacker_units and defender_units are EXACT sets read from live memory (13/13
        validated vs the save). A side that fails the fail-closed validators yields None for
        that side (distinguishable from an empty []), and the reason is tallied in
        self.reject_stats. Enumeration: vtable scan (MODULE+0x2980CB8) filtered by type==62.
        """
        vt_pat = struct.pack("<Q", self.cland_vt)
        bodies = {}
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
                hdr = self._rpm(cand, 0x38)
                if hdr and len(hdr) >= 0x38 and struct.unpack_from("<I", hdr, OFF_TYPE)[0] == LAND_COMBAT_TYPE:
                    cid = struct.unpack_from("<I", hdr, OFF_COMBAT_ID)[0]
                    bodies.setdefault(cid, cand)
                if len(bodies) > MAX_COMBATS:
                    raise RuntimeError("combat count exceeds sanity bound (%d) - layout likely broken" % MAX_COMBATS)

        out = []
        for cid in sorted(bodies):
            body = bodies[cid]
            duration = self.u32(body + OFF_DURATION)
            day = self.u32(body + OFF_DAY)
            front = self.u64(body + OFF_FRONT)
            location = self.u32(front + OFF_FRONT_LOC) if front and HEAP_LO <= front < HEAP_HI else None
            atk_side, def_side, _reg = self._combat_sides(body)
            attacker = self._side_units(atk_side) if atk_side else None
            rec = {
                "combat_id": cid,
                "body_addr": body,
                "location": location,
                "day": day,
                "duration": duration,
                "attacker_units": attacker,
            }
            if with_defenders:
                rec["defender_units"] = self._side_units(def_side) if def_side else None
            out.append(rec)
        return out

    # ======================================================================
    # NAVAL combat (CNavalCombat / CNavalCombatant) - solved + validated 3/3
    # 2026-06-25 against the live paused frame's 3 user-anchored naval battles
    # (FRA/JAP, MEX/JAP, SOV/FIN) on attacker tag, defender tag, sub count,
    # ship count AND commander name. See naval_combat_offsets.draft.json.
    # ======================================================================
    def _read_vec(self, side, data_off, size_off):
        """Return (data_ptr, size) of a side's inline std::vector-like field, or (None, 0)."""
        data = self.u64(side + data_off)
        size = self.u32(side + size_off)
        if size is None:
            return None, 0
        if size > MAX_SIDE_UNITS:           # fail-closed sanity bound
            return None, 0
        return data, size

    def _naval_side_countries(self, side):
        """All tag-db-ids on a naval side (side+0x68 vector of packed country handles).
        Reads ALL entries (coalition-safe, per Codex); low u32 of each handle = tag-db-id."""
        data, size = self._read_vec(side, OFF_NS_CTRY_DATA, OFF_NS_CTRY_SIZE)
        if not data or size == 0 or not (HEAP_LO <= data < HEAP_HI):
            return []
        out = []
        for i in range(size):
            h = self.u64(data + i * 8)
            if h is None:
                continue
            out.append(h & 0xFFFFFFFF)
        return out

    def _naval_side_ships(self, side):
        """(ui_count, raw_count) summed over ALL CFEXGroup entries of a naval side
        (side+0xC8 vector). ui = sum CFEXGroup+0x14 (anchor-matching: subs/ships as shown);
        raw = sum CFEXGroup+0x10 (broader total, logged not trusted). Coalition-safe.
        Returns (None, None) if the group vector / a group object is invalid (fail-closed)."""
        data, size = self._read_vec(side, OFF_NS_FEX_DATA, OFF_NS_FEX_SIZE)
        if not data or size == 0 or not (HEAP_LO <= data < HEAP_HI):
            return None, None
        ui = raw = 0
        for i in range(size):
            grp = self.u64(data + i * 8)
            if grp is None or not (HEAP_LO <= grp < HEAP_HI):
                return None, None
            u = self.u32(grp + OFF_FEX_SHIPS_UI)
            r = self.u32(grp + OFF_FEX_SHIPS_RAW)
            if u is None or r is None:
                return None, None
            ui += u
            raw += r
        return ui, raw

    def _stdstr(self, addr, maxlen=128):
        """Read an MSVC std::string at `addr` (fixed 0x20-byte layout: [16 bytes data-or-ptr]
        [u64 size][u64 cap]). cap<=15 => inline SSO bytes; cap>15 => heap data ptr at +0x00.
        Returns the decoded text (utf-8, latin-1 fallback) or ''. READ-ONLY."""
        hdr = self._rpm(addr, 0x18) or b""
        if len(hdr) < 0x18:
            return ""
        size = struct.unpack_from("<Q", hdr, 0x10)[0]
        cap = struct.unpack_from("<Q", hdr, 0x18 - 8)[0] if len(hdr) >= 0x18 else 0
        # cap actually lives at +0x18; re-read fully (struct is 0x20 bytes)
        full = self._rpm(addr, 0x20) or b""
        if len(full) < 0x20:
            return ""
        size = struct.unpack_from("<Q", full, 0x10)[0]
        cap = struct.unpack_from("<Q", full, 0x18)[0]
        if size > maxlen:
            size = maxlen
        if cap <= 15:
            raw = full[:16][:size]
        else:
            p = struct.unpack_from("<Q", full, 0)[0]
            if not (HEAP_LO <= p < HEAP_HI):
                return ""
            raw = self._rpm(p, min(size, maxlen)) or b""
        raw = raw.split(b"\x00", 1)[0]
        try:
            return raw.decode("utf-8")
        except Exception:
            return raw.decode("latin-1", "replace")

    def _group_ships(self, grp):
        """Walk a CFEXGroup's embedded std::vector<CFEXMember*> (data@+0x08, {cap,size}@+0x10).
        Returns the list of CFEXMember* whose vtable == the ship vtable (SHIP_VT_OFF). Fail-closed:
        returns [] if the group/vector is invalid. Validated 8/8 sides == the UI count."""
        if not grp or not (HEAP_LO <= grp < HEAP_HI):
            return []
        data = self.u64(grp + OFF_FEX_MEMBERS)
        packed = self.u64(grp + OFF_FEX_SHIPS_RAW)   # {cap (low u32), size (high u32)}
        if data is None or packed is None:
            return []
        size = (packed >> 32) & 0xFFFFFFFF
        if size == 0:
            return []
        if size > MAX_SHIPS_PER_GROUP or not (HEAP_LO <= data < HEAP_HI):
            return []
        ships = []
        for i in range(size):
            s = self.u64(data + i * 8)
            if s and (HEAP_LO <= s < HEAP_HI) and self.u64(s) == self.ship_vt:
                ships.append(s)
        return ships

    def _read_ship(self, member):
        """Decode one CFEXMember (per-ship in-combat object). Returns a dict with the
        VALIDATED fields (name/type/archetype/owner_tag/type_tag/instance_id/slot) plus the
        structurally-located but SEMANTICALLY-UNCONFIRMED candidates (max_stat). HP/org are
        NOT returned because they are not yet anchor-locked on this object (warship HP/org
        resolves via the +0x08 instance id through a separate global ship registry)."""
        pack = self.u64(member + OFF_SHIP_TYPEID) or 0
        type_tag = pack & 0xFFFFFFFF
        instance_id = (pack >> 32) & 0xFFFFFFFF
        owner_q = self.u64(member + OFF_SHIP_OWNER_TAG) or 0
        return {
            "addr": member,
            "name": self._stdstr(member + OFF_SHIP_NAME),
            "type": self._stdstr(member + OFF_SHIP_TYPE),
            "archetype": self._stdstr(member + OFF_SHIP_ARCHETYPE),
            "owner_tag_db_id": owner_q & 0xFFFFFFFF,
            "unit_type_tag": type_tag,          # 0x33==submarine; 0 for a convoy
            "instance_id": instance_id,         # in-game ship id; 0 for a convoy
            "is_convoy": (type_tag == 0 and instance_id == 0),
            "slot_index": self.u32(member + OFF_SHIP_SLOT),
            "max_stat_raw": self.u32(member + OFF_SHIP_MAXSTAT),  # CANDIDATE: static per-type
        }

    def enumerate_naval_ships(self, body_addr):
        """Given a CNavalCombat body address (from enumerate_naval_combats), return per-side
        per-ship detail:
            { combat_id, attacker_ships:[ship_dict,...], defender_ships:[ship_dict,...] }
        ship_dict carries the VALIDATED identity fields (name/type/archetype/owner/instance_id/
        slot) - see _read_ship. The ship COUNT per side is validated 8/8 against the UI anchors
        (the walk reaches exactly SIZE CFEXMember* per group). Fail-closed per the same identity
        chain as enumerate_naval_combats."""
        atk, deff, reg = self._naval_sides(body_addr)
        out = {"combat_id": ((self.u64(body_addr + OFF_N_TYPEID) or 0) >> 32),
               "attacker_ships": [], "defender_ships": []}
        if atk is None:
            return out
        for side, key in ((atk, "attacker_ships"), (deff, "defender_ships")):
            if side is None:
                continue
            data, n = self._read_vec(side, OFF_NS_FEX_DATA, OFF_NS_FEX_SIZE)
            if not data or n == 0 or not (HEAP_LO <= data < HEAP_HI):
                continue
            for gi in range(n):
                grp = self.u64(data + gi * 8)
                for m in self._group_ships(grp):
                    out[key].append(self._read_ship(m))
        return out

    def _naval_side_commander(self, side):
        """Admiral name (str) from side+0x110 -> CNavyLeader, or None. Defenders / leaderless
        sides return None (anchors: defenders show 'No Commander')."""
        nl = self.u64(side + OFF_NS_LEADER)
        if not nl or not (HEAP_LO <= nl < HEAP_HI):
            return None
        # name std::string at +0x20: heap ptr (long names) or inline SSO (<=15 chars)
        pv = self.u64(nl + OFF_NL_NAME)
        if pv and HEAP_LO <= pv < HEAP_HI:
            s = self._cstr(pv, 64)
            if s:
                return s
        raw = self._rpm(nl + OFF_NL_NAME, 16) or b""
        raw = raw.split(b"\x00", 1)[0]
        try:
            return raw.decode("utf-8")
        except Exception:
            return raw.decode("latin-1", "replace")

    def _cstr(self, addr, maxlen=64):
        raw = self._rpm(addr, maxlen) or b""
        raw = raw.split(b"\x00", 1)[0]
        try:
            return raw.decode("utf-8")
        except Exception:
            return raw.decode("latin-1", "replace")

    def _naval_sides(self, body):
        """Resolve (attacker_side, defender_side, registry) for a naval combat body using the
        REGISTRY ORDER as authoritative (Codex: leader/TF presence is heuristic only). Verifies
        the full identity chain; returns (None, None, None) for a body that fails any check so a
        stale/ghost heap object is never reported."""
        cid = (self.u64(body + OFF_N_TYPEID) or 0) >> 32
        sa = self.u64(body + OFF_N_SIDE_A)
        sb = self.u64(body + OFF_N_SIDE_B)
        if not sa or not (HEAP_LO <= sa < HEAP_HI) or self.u64(sa) != self.nside_vt:
            self._note_reject("naval_bad_side_a"); return None, None, None
        if not sb or not (HEAP_LO <= sb < HEAP_HI) or self.u64(sb) != self.nside_vt:
            self._note_reject("naval_bad_side_b"); return None, None, None
        reg = self.u64(sa + OFF_NS_REG)
        if not reg or not (HEAP_LO <= reg < HEAP_HI) or self.u64(reg) != self.nreg_vt:
            self._note_reject("naval_bad_registry"); return None, None, None
        handle = self.u64(reg + OFF_NREG_HANDLE)
        if handle is None or (handle & 0xFFFFFFFF) != NAVAL_COMBAT_TYPE or (handle >> 32) != cid:
            self._note_reject("naval_bad_identity"); return None, None, None
        if self.u64(reg + OFF_NREG_ATK_SIDE) != sa or self.u64(reg + OFF_NREG_DEF_SIDE) != sb:
            self._note_reject("naval_side_order_mismatch"); return None, None, None
        return sa, sb, reg

    def enumerate_naval_combats(self, with_defenders=True):
        """Return a list of dicts, one per active NAVAL combat:
            { combat_id, body_addr, location,            # location = sea-zone province id
              attacker_countries:[tag_db_id,...], attacker_ships, attacker_ships_raw,
              attacker_commander,
              defender_countries:[tag_db_id,...], defender_ships, defender_ships_raw,
              defender_commander }
        Attacker = registry+0x28 side (authoritative order), defender = registry+0x30.
        *_ships is the UI-matching count (CFEXGroup+0x14, summed over coalition groups);
        *_ships_raw is the broader CFEXGroup+0x10 total (logged, not anchor-validated).
        A body failing the identity chain is dropped (fail-closed) and tallied in
        self.reject_stats. Enumeration: vtable scan for CNavalCombat body (MODULE+0x29B6660)."""
        vt_pat = struct.pack("<Q", self.cnaval_vt)
        bodies = {}
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
                tid = self._rpm(cand + OFF_N_TYPEID, 8)
                if tid and len(tid) >= 8:
                    packed = struct.unpack("<Q", tid)[0]
                    if (packed & 0xFFFFFFFF) == NAVAL_COMBAT_TYPE:
                        bodies.setdefault(packed >> 32, cand)
                if len(bodies) > MAX_COMBATS:
                    raise RuntimeError("naval combat count exceeds sanity bound (%d)" % MAX_COMBATS)

        out = []
        for cid in sorted(bodies):
            body = bodies[cid]
            atk, deff, reg = self._naval_sides(body)
            if atk is None:
                continue                      # fail-closed: ghost / stale body
            front = self.u64(reg + OFF_NREG_FRONT)
            location = self.u32(front + OFF_FRONT_LOC) if front and HEAP_LO <= front < HEAP_HI else None
            a_ui, a_raw = self._naval_side_ships(atk)
            rec = {
                "combat_id": cid,
                "body_addr": body,
                "location": location,
                "attacker_countries": self._naval_side_countries(atk),
                "attacker_ships": a_ui,
                "attacker_ships_raw": a_raw,
                "attacker_commander": self._naval_side_commander(atk),
            }
            if with_defenders:
                d_ui, d_raw = self._naval_side_ships(deff)
                rec["defender_countries"] = self._naval_side_countries(deff)
                rec["defender_ships"] = d_ui
                rec["defender_ships_raw"] = d_raw
                rec["defender_commander"] = self._naval_side_commander(deff)
            out.append(rec)
        return out

    def close(self):
        if self.h:
            _k32.CloseHandle(self.h)
            self.h = None


def main():
    r = CombatReader()
    ok, sha = r.verify_build()
    print("hoi4.exe pid=%d module_base=0x%X" % (r.pid, r.module_base))
    print("build sha256: %s" % sha)
    print("build gate: %s" % ("OK (matches pinned build)" if ok else "MISMATCH - refusing to trust offsets"))
    if not ok:
        print("Refusing to enumerate on an unverified build (offsets are build-specific).")
        r.close()
        return
    combats = r.enumerate_land_combats(with_defenders=True)
    print("\nactive land combats: %d" % len(combats))
    for c in combats:
        print("  combat_id=%d province=%s age_days=%s attacker=%s defender=%s" % (
            c["combat_id"], c["location"], c["day"], c["attacker_units"], c.get("defender_units")))

    navals = r.enumerate_naval_combats(with_defenders=True)
    print("\nactive naval combats: %d" % len(navals))
    for c in navals:
        print("  combat_id=%d sea_prov=%s  ATK tags=%s subs=%s (raw %s) cmdr=%r |"
              " DEF tags=%s ships=%s (raw %s) cmdr=%r" % (
              c["combat_id"], c["location"],
              c["attacker_countries"], c["attacker_ships"], c["attacker_ships_raw"],
              c["attacker_commander"],
              c.get("defender_countries"), c.get("defender_ships"), c.get("defender_ships_raw"),
              c.get("defender_commander")))
        # per-ship detail (count-validated; identity fields validated, HP/org pending anchor)
        det = r.enumerate_naval_ships(c["body_addr"])
        for side_key in ("attacker_ships", "defender_ships"):
            for s in det[side_key]:
                kind = "CONVOY" if s["is_convoy"] else ("warship t=0x%X id=%d" % (s["unit_type_tag"], s["instance_id"]))
                print("      %-3s slot=%-3s owner_tag=%-3s %-16s name=%r type=%r [%s]" % (
                    "ATK" if side_key == "attacker_ships" else "DEF",
                    s["slot_index"], s["owner_tag_db_id"], "max_stat=%s" % s["max_stat_raw"],
                    s["name"], s["type"], kind))

    if r.reject_stats:
        print("\nrejected side reads (fail-closed, by reason): %s" % r.reject_stats)
    r.close()


if __name__ == "__main__":
    main()
