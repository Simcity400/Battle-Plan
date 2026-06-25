"""naval_air_reader.py - READ-ONLY live NAVAL & AIR theater reader for Battle Plan.

Reads, from a running hoi4.exe (build 16022851), the data behind the Naval and Air map modes
plus fleet/task-force positions and air-wing enumeration:

  * NAVAL SEA-ZONE CONTROL (Naval map mode): per-strategic-region naval supremacy / dominance.
    Chain: strategic-region object (vtable MODULE+0x29468B8) -> region_id @+0x58,
    dominance-container ptr @+0xE8 -> container (vtable MODULE+0x295DF30) -> entries_begin
    @+0x40, count @+0x48 -> per-(region,country) dominance entry (vtable MODULE+0x295DEE0,
    stride 0x50): controller tag-db-id @+0x00, current/target/base_target/individual_ratio/
    previous/decline_from at +0x20/+0x28/+0x30/+0x38/+0x40/+0x48 (u32 fixed-point x100000).
    VALIDATED 33/33 vs a time-aligned save WITH region id.

  * TASK-FORCE POSITIONS (Naval map markers): task_force object (vtable MODULE+0x293C8B0,
    type 61) current_province_id @+0x18. Fleet object enumeration (vtable MODULE+0x293C178).

  * AIR WINGS: enumerated by vtable MODULE+0x29540C0 (type 69 @+0x08, wing_id @+0x0C,
    count @+0x6C). 265/265 vs the save. OWNER, mission strategic_region, and the EAS proxy
    are now SOLVED:
      - OWNER: wing+0x28 -> air_wing_pool (vtable MODULE+0x2954118) -> +0x40 -> CCountry
        (vtable MODULE+0x2932248) -> +0x90 owner tag-db-id. 265/265 vs the save. Pool objects
        are strictly per-owner, so this is the TRUE wing owner, not the base controller.
      - MISSION strategic_region: wing+0xB0 -> a strategic-region object (same class as the
        naval dominance regions) -> region_id @+0x58. 56/56 on region-bearing wings; the other
        209 are idle (null/non-region pointer) and correctly contribute to no region.
      - EAS proxy: effective_planes @ wing+0x160. 56/56 vs the save.
    enumerate_air_superiority() = THE AIR MAP MODE: bucket present region-bearing wings by
    (mission strategic_region, owner) and sum effective_planes. Validated: per-(region,owner)
    sums == the save's air-superiority oracle 12/12 buckets EXACT. Air superiority is DERIVED
    (no stored per-region accumulator; the region.dominance object is naval-only); the exact
    per-wing scalar drifts/recomputes, so effective_planes is the stable faithful color proxy.

Semantics: 'dominance' is NAVAL supremacy / sea-zone dominance (confirmed via mod defines
NNavy.DOMINANCE_DAILY_GAIN_FACTOR, ship naval_dominance_factor, minimum_seazone_dominance,
GFX_naval_dominance_texticon, and by all 31 saved dominance regions being fleet-operated).
It is NOT air superiority.

  * NAVAL / AIR COMBAT (enumerate_naval_combats / enumerate_air_combats): a build/live-state-gated
    DETECTOR. It enumerates every CCombatSide object (the land-combat side class, vtable
    MODULE+0x2980AC0), classifies each side's participants by vtable identity (division / task-force
    / fleet / ship / air-wing), and returns the combats whose participants are naval (task-force/
    ship) or air (wing). RE pass 2026-06-25 (x2: original on a 1946 save's 2 naval_combat entries +
    a RELAUNCH vs the LIVE ticking game) REFUTED the hypothesis that naval/air combat reuses these
    land classes - against LIVE memory, not just the save: 0/87 live CCombatSide objects have task-
    force/ship/wing participants (20 division-only land combats, 64 empty), all 4 registries are land
    (htype=62), the saved combat handles do not exist live, and NO unknown-vtable object holds a >=2
    task-force/ship/wing participant vector by pointer OR by handle. The relaunch ALSO refuted the
    per-region and per-task-force aggregate-combat hypotheses: 0/304 strategic-region objects and
    0/135 task forces point to an unknown-vtable object holding unit participants. The saved task
    forces (477/473) AND their member ships persist live (the TFs moved off the raid sea provinces ->
    the raids ended); the saved naval entries are transient submarine convoy-RAID contacts (day=-1,
    empty defender). So these two methods correctly return [] on this build/state; they are NOT
    fabricated - they run the real machinery and will populate the moment a live fleet-vs-fleet / air
    battle instantiates a side of these classes. Codex (final review) scoped this as a VALIDATED
    NEGATIVE against the direct-object hypotheses, NOT a universal absence proof: index-based /
    vector-indirect / POD-pool / state-distributed-onto-unit-struct storage remain unclosed and need
    a controlled BEFORE/DURING/AFTER temporal diff of a real surface battle. See
    naval_air_offsets.draft.json ('naval_air_combat', incl. pass2_relaunch_2026_06_25) +
    RESEARCH_naval_air.md section 5/5b for the full negative + re-anchor recipe.

HARD RULES honored (same as combat_enum.py): READ-ONLY memory (PROCESS_VM_READ |
PROCESS_QUERY_INFORMATION); never writes/injects/suspends; build-gated by exe sha256; every
fixed-size read clamped to the VirtualQuery region end; arena-scoped scans
(0x1F600000000..0x1FA00000000); fail-closed validators with per-reason reject telemetry;
double-read revalidation of the dominance container header (Codex-recommended). ASCII output.

This module carries its own minimal ctypes helper (adapted from combat_enum.py) so the
naval_air package is self-contained and does NOT edit winmem.py / locate.py.
"""

import ctypes as C
import struct
import hashlib
from ctypes import wintypes as W

# --------------------------------------------------------------------------------------
# Build gate + solved constants (module-relative; see naval_air_offsets.draft.json)
# --------------------------------------------------------------------------------------
EXPECTED_SHA256 = "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120"
PROC_NAME = "hoi4.exe"

# module-relative vtables
REGION_VT_OFF   = 0x29468B8   # strategic-region object (primary)
REGION_VT2_OFF  = 0x2946908   # strategic-region object (secondary / interface)
DOM_CONT_VT_OFF = 0x295DF30   # dominance container
DOM_ENT_VT_OFF  = 0x295DEE0   # dominance entry (sits at entry+0x08)
WING_VT_OFF     = 0x29540C0   # air-wing object
FLEET_VT_OFF    = 0x293C178   # fleet object
TF_VT_OFF       = 0x293C8B0   # task-force object
SHIP_VT_OFF     = 0x2931998   # ship object (type 51); pinned 2026-06-25 (combat RE pass)
POOL_VT_OFF     = 0x2954118   # air_wing_pool / air-base object (wing+0x28 target; owner chain)
COUNTRY_VT_OFF  = 0x2932248   # CCountry object (pool+0x40 target; owner tag-db-id @+0x90)

# --- combat objects (the land-combat classes; see memory_reader/combat) ---
# Re-used here ONLY to ENUMERATE combat sides and CLASSIFY their participants (division vs
# task-force vs air-wing). Naval/air combat does NOT currently reuse these classes - see the
# naval_air_combat block in naval_air_offsets.draft.json (RE pass 2026-06-25, land-mirror REFUTED).
SIDE_VT_OFF     = 0x2980AC0   # CCombatSide (land); side participant small-vec data@+0x20 size@+0x2C
REG_VT_OFF      = 0x2980BD8   # combat-sides registry; +0x18 identity, +0x28 atk, +0x30 def, +0x38 front
DIV_VT_OFF      = 0x2933F00   # CDivision body (land participant)
OFF_SIDE_REG    = 0x18        # side -> registry back-ptr
OFF_SIDE_DATA   = 0x20        # side participant array (T**)
OFF_SIDE_CAP    = 0x28        # u32 capacity
OFF_SIDE_SIZE   = 0x2C        # u32 size (participant count)
OFF_REG_HANDLE  = 0x18        # u64 (combat_id<<32)|type
OFF_REG_DEF     = 0x30        # registry -> DEFENDER side
OFF_REG_FRONT   = 0x38        # registry -> front/location object
OFF_FRONT_LOC   = 0xA4        # province id in the front/border object
MAX_SIDE_UNITS  = 4000        # fail-closed bound on a side's participant count

# type tags
WING_TYPE = 69
TF_FLEET_TYPE = 61
SHIP_TYPE = 51
DIV_TYPE = 4713

# strategic-region object fields
OFF_R_VT2   = 0x08
OFF_R_REGID = 0x58
OFF_R_CONT  = 0xE8

# dominance container fields
OFF_C_BEGIN = 0x40
OFF_C_COUNT = 0x48

# dominance entry fields (stride 0x50; vtable @ entry+0x08)
DOM_STRIDE   = 0x50
OFF_E_VT     = 0x08
OFF_E_TAG    = 0x00
OFF_E_TAG2   = 0x10
OFF_E_CUR    = 0x20
OFF_E_TGT    = 0x28
OFF_E_BT     = 0x30
OFF_E_IR     = 0x38
OFF_E_PREV   = 0x40
OFF_E_DEC    = 0x48
FX = 100000.0

# air-wing fields
OFF_W_TYPE  = 0x08
OFF_W_ID    = 0x0C
OFF_W_COUNT = 0x6C
OFF_W_POOL  = 0x28    # -> air_wing_pool/air-base obj (vtable POOL_VT); owner chain step 1
OFF_W_MREG  = 0xB0    # -> mission target strategic-region obj (vtable REGION_VT); region +0x58
OFF_W_EFFP  = 0x160   # u32 effective_planes (operational plane count; the air-sup map proxy)
# owner chain inside the pool / country objects
OFF_POOL_COUNTRY = 0x40   # pool+0x40 -> CCountry obj (vtable COUNTRY_VT)
OFF_COUNTRY_TAG  = 0x90   # country+0x90 = owner tag-db-id

# fleet / task-force fields
OFF_TF_TYPE = 0x08
OFF_TF_ID   = 0x0C
OFF_TF_LOC  = 0x18

# heap / arena ranges and sanity bounds
HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000
ARENA_LO = 0x1F600000000
ARENA_HI = 0x1FA00000000
MAX_REGIONS = 4096
MAX_REGION_ID = 4096
MAX_DOM_ENTRIES = 1024     # generous fail-closed cap (Codex)
MAX_WINGS = 20000
MAX_TASK_FORCES = 20000

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


class NavalAirReader(object):
    """Read-only handle onto a running hoi4.exe with the naval/air layout pinned."""

    def __init__(self, proc_name=PROC_NAME):
        self.proc_name = proc_name
        self.pid = self._find_pid(proc_name)
        if self.pid is None:
            raise RuntimeError("process not found: %s" % proc_name)
        self.h = _k32.OpenProcess(0x10 | 0x400, False, self.pid)  # VM_READ | QUERY_INFORMATION
        if not self.h:
            raise RuntimeError("OpenProcess failed (err %d)" % C.get_last_error())
        self.module_base, self.module_size, self.exe_path = self._module(self.pid, proc_name)
        if self.module_base is None:
            raise RuntimeError("module base not found: %s" % proc_name)
        mb = self.module_base
        self.region_vt = mb + REGION_VT_OFF
        self.region_vt2 = mb + REGION_VT2_OFF
        self.dom_cont_vt = mb + DOM_CONT_VT_OFF
        self.dom_ent_vt = mb + DOM_ENT_VT_OFF
        self.wing_vt = mb + WING_VT_OFF
        self.fleet_vt = mb + FLEET_VT_OFF
        self.tf_vt = mb + TF_VT_OFF
        self.ship_vt = mb + SHIP_VT_OFF
        self.pool_vt = mb + POOL_VT_OFF
        self.country_vt = mb + COUNTRY_VT_OFF
        self.side_vt = mb + SIDE_VT_OFF
        self.reg_vt = mb + REG_VT_OFF
        self.div_vt = mb + DIV_VT_OFF
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

    def _note_reject(self, reason):
        self.reject_stats[reason] = self.reject_stats.get(reason, 0) + 1

    @staticmethod
    def _isheap(v):
        return v is not None and HEAP_LO <= v < HEAP_HI

    def _enum_arena(self):
        """Committed, readable, non-executable heap regions in the game arena (cached)."""
        out = []
        addr = ARENA_LO
        mbi = _MBI()
        while addr < ARENA_HI:
            if _k32.VirtualQueryEx(self.h, C.c_void_p(addr), C.byref(mbi), C.sizeof(mbi)) == 0:
                addr += 0x1000
                continue
            base = mbi.BaseAddress or addr
            size = mbi.RegionSize or 0x1000
            nxt = base + size
            committed = (mbi.State == 0x1000)
            not_guard = (mbi.Protect & 0x100) == 0
            readable = (mbi.Protect & 0xFF) in (0x02, 0x04, 0x08, 0x20, 0x40, 0x80)
            if committed and not_guard and readable and size <= (1 << 31) and ARENA_LO <= base < ARENA_HI:
                out.append((base, size))
            addr = nxt if nxt > addr else addr + 0x1000
        return out

    # ---- build gate ----
    def verify_build(self):
        try:
            with open(self.exe_path, "rb") as f:
                digest = hashlib.sha256(f.read()).hexdigest()
        except Exception as e:
            return False, "exe-unreadable:%s" % e
        return (digest == EXPECTED_SHA256), digest

    # ---- vtable scan helper ----
    def _scan_vtable(self, vt, type_off=None, type_val=None):
        """Reverse-scan the arena for objects whose vtable qword == vt. If type_off/type_val
        given, keep only objects with u32[base+type_off]==type_val. Returns list of base addrs."""
        pat = struct.pack("<Q", vt)
        out = []
        for base, size in self._enum_arena():
            blob = self._rpm(base, size)
            if not blob:
                continue
            s = 0
            while True:
                j = blob.find(pat, s)
                if j < 0:
                    break
                cand = base + j
                s = j + 8
                if type_off is None:
                    out.append(cand)
                else:
                    tv = self.u32(cand + type_off)
                    if tv == type_val:
                        out.append(cand)
        return out

    # ---- NAVAL: strategic-region dominance ----
    def _read_dominance(self, region_obj):
        """Return list of dominance-entry dicts for a region, or None (fail-closed).
        [] means the region has a valid-but-empty dominance container (no contest)."""
        cont = self.u64(region_obj + OFF_R_CONT)
        if not self._isheap(cont):
            return []  # no dominance container -> no contest (Codex: NULL is "no data", not error)
        if self.u64(cont) != self.dom_cont_vt:
            self._note_reject("bad_container_vtable"); return None
        beg = self.u64(cont + OFF_C_BEGIN)
        cnt = self.u32(cont + OFF_C_COUNT)
        if cnt is None:
            self._note_reject("count_read_fail"); return None
        if cnt == 0:
            return []
        if cnt > MAX_DOM_ENTRIES:
            self._note_reject("count_sane_bound"); return None
        if not self._isheap(beg):
            self._note_reject("bad_begin_ptr"); return None
        blob = self._read_clamped(beg, cnt * DOM_STRIDE)
        if len(blob) < cnt * DOM_STRIDE:
            self._note_reject("short_entry_read"); return None
        ents = []
        for i in range(cnt):
            e = beg + i * DOM_STRIDE
            if self.u64(e + OFF_E_VT) != self.dom_ent_vt:
                self._note_reject("bad_entry_vtable"); return None
            tag = self.u32(e + OFF_E_TAG)
            if tag != self.u32(e + OFF_E_TAG2):
                self._note_reject("entry_tag_mismatch"); return None
            ents.append({
                "controller_tagid": tag,
                "current": (self.u32(e + OFF_E_CUR) or 0) / FX,
                "target": (self.u32(e + OFF_E_TGT) or 0) / FX,
                "base_target": (self.u32(e + OFF_E_BT) or 0) / FX,
                "individual_ratio": (self.u32(e + OFF_E_IR) or 0) / FX,
                "previous": (self.u32(e + OFF_E_PREV) or 0) / FX,
                "decline_from": (self.u32(e + OFF_E_DEC) or 0) / FX,
            })
        # double-read revalidation of the container header (Codex): reject on mid-read mutation
        if self.u64(cont + OFF_C_BEGIN) != beg or self.u32(cont + OFF_C_COUNT) != cnt:
            self._note_reject("container_race_changed"); return None
        return ents

    def enumerate_sea_zone_control(self, tag_table=None):
        """Return {region_id: [dominance entries]} for every strategic region with a dominance
        container. Each entry: controller_tagid (+ 'controller' TAG if tag_table given),
        current/target/base_target/individual_ratio/previous/decline_from. This IS the Naval map
        mode (naval supremacy / sea-zone dominance). Regions with no contest are omitted.
        A region whose container fails validation is skipped and tallied in reject_stats."""
        regions = {}
        for ro in self._scan_vtable(self.region_vt):
            if self.u64(ro + OFF_R_VT2) != self.region_vt2:
                continue
            rid = self.u32(ro + OFF_R_REGID)
            if rid is None or not (1 <= rid <= MAX_REGION_ID):
                continue
            regions.setdefault(rid, ro)
            if len(regions) > MAX_REGIONS:
                raise RuntimeError("region count exceeds sanity bound - layout likely broken")
        out = {}
        for rid in sorted(regions):
            ents = self._read_dominance(regions[rid])
            if ents:  # only regions with active dominance
                if tag_table is not None:
                    for e in ents:
                        t = tag_table.tag(e["controller_tagid"])
                        e["controller"] = t if t else ("?%d" % e["controller_tagid"])
                out[rid] = ents
        return out

    # ---- task forces (Naval map markers) ----
    def enumerate_task_forces(self):
        """Return [{tf_id, current_province_id, body_addr}] for every live task force.
        current_province_id is the sea/coastal province the task force is currently in."""
        out = []
        seen = set()
        for tf in self._scan_vtable(self.tf_vt, OFF_TF_TYPE, TF_FLEET_TYPE):
            tid = self.u32(tf + OFF_TF_ID)
            if tid is None or tf in seen:
                continue
            seen.add(tf)
            out.append({"tf_id": tid, "current_province_id": self.u32(tf + OFF_TF_LOC),
                        "body_addr": tf})
            if len(out) > MAX_TASK_FORCES:
                raise RuntimeError("task-force count exceeds sanity bound")
        return out

    def enumerate_fleets(self):
        """Return [{fleet_id, body_addr}] for every live fleet (identity only)."""
        out = []
        for fl in self._scan_vtable(self.fleet_vt, OFF_TF_TYPE, TF_FLEET_TYPE):
            fid = self.u32(fl + OFF_TF_ID)
            if fid is None:
                continue
            out.append({"fleet_id": fid, "body_addr": fl})
            if len(out) > MAX_TASK_FORCES:
                raise RuntimeError("fleet count exceeds sanity bound")
        return out

    # ---- air-wing owner + mission region (the air-superiority inputs) ----
    def wing_owner_tagid(self, wb):
        """Resolve a wing's OWNING-COUNTRY tag-db-id via the identity-checked chain
        wing+0x28 (air_wing_pool, vtable POOL_VT) -> +0x40 (CCountry, vtable COUNTRY_VT)
        -> +0x90 (owner tag-db-id). Returns the tag-db-id, or None (fail-closed) if any hop
        fails its vtable guard. Pool objects are strictly per-owner, so this is the TRUE wing
        owner (not the base controller). Validated 265/265 vs the save."""
        pool = self.u64(wb + OFF_W_POOL)
        if not self._isheap(pool) or self.u64(pool) != self.pool_vt:
            self._note_reject("wing_pool_vtable"); return None
        cty = self.u64(pool + OFF_POOL_COUNTRY)
        if not self._isheap(cty) or self.u64(cty) != self.country_vt:
            self._note_reject("wing_country_vtable"); return None
        return self.u32(cty + OFF_COUNTRY_TAG)

    def wing_mission_region(self, wb):
        """Resolve a wing's MISSION strategic_region via wing+0xB0 -> a strategic-region object
        (same class as the naval dominance regions: vtable REGION_VT @+0x00, REGION_VT2 @+0x08)
        -> region_id @+0x58. Returns the region id, or None if the wing has no active
        region-bearing mission (idle / null pointer / non-region object). Validated 56/56."""
        pr = self.u64(wb + OFF_W_MREG)
        if not self._isheap(pr):
            return None
        if self.u64(pr + OFF_R_VT2) != self.region_vt2 or self.u64(pr) != self.region_vt:
            return None
        rid = self.u32(pr + OFF_R_REGID)
        if rid is None or not (1 <= rid <= MAX_REGION_ID):
            return None
        return rid

    # ---- air wings ----
    def enumerate_air_wings(self, tag_table=None):
        """Return [{wing_id, count, owner_tagid, owner, mission_region, effective_planes,
        body_addr}] for every live air wing. All fields SOLVED & save-validated:
          wing_id (+0x0C, 265/265), count (+0x6C, raw strength),
          owner_tagid (owner chain, 265/265; 'owner' TAG if tag_table given),
          mission_region (+0xB0 region object -> +0x58; None when idle; 56/56 on region-bearers),
          effective_planes (+0x160; the air-superiority map-mode proxy, 56/56).
        owner_tagid/mission_region may be None on a fail-closed guard miss (tallied in
        reject_stats); they are never guessed."""
        out = []
        for wb in self._scan_vtable(self.wing_vt, OFF_W_TYPE, WING_TYPE):
            wid = self.u32(wb + OFF_W_ID)
            if wid is None:
                continue
            oid = self.wing_owner_tagid(wb)
            rec = {"wing_id": wid, "count": self.u32(wb + OFF_W_COUNT),
                   "owner_tagid": oid,
                   "mission_region": self.wing_mission_region(wb),
                   "effective_planes": self.u32(wb + OFF_W_EFFP),
                   "body_addr": wb}
            if tag_table is not None and oid is not None:
                t = tag_table.tag(oid)
                rec["owner"] = t if t else ("?%d" % oid)
            out.append(rec)
            if len(out) > MAX_WINGS:
                raise RuntimeError("air-wing count exceeds sanity bound")
        return out

    def enumerate_air_superiority(self, tag_table=None):
        """THE AIR MAP MODE: per-strategic-region air superiority, aggregated from present air
        wings. For every wing on a region-bearing mission (mission_region not None), bucket by
        (mission_region, owner) and SUM effective_planes (+0x160) - the faithful, save-validated
        proxy for the per-wing air-superiority contribution (exact contribution = effective_planes
        * variant air_superiority stat * modifiers; the variant stat is 1 for typical fighters and
        the exact scalar is a drifting recomputed cache, so effective_planes colors the map).

        Returns {region_id: {"by_owner": {owner_key: {"effective_planes": float, "wings": int}},
                             "leader": owner_key, "total": float}}
        where owner_key is the 3-letter TAG if tag_table is given, else the tag-db-id (int).
        The Air map mode is viewer/faction-relative, so per-owner sums are returned and the
        consumer picks the viewer's side / computes the contested share. Validated: per-(region,
        owner) sums == the save's air-superiority oracle 12/12 buckets EXACT."""
        regions = {}
        for wb in self._scan_vtable(self.wing_vt, OFF_W_TYPE, WING_TYPE):
            if self.u32(wb + OFF_W_ID) is None:
                continue
            rid = self.wing_mission_region(wb)
            if rid is None:
                continue  # idle wing: contributes to no region (correct)
            oid = self.wing_owner_tagid(wb)
            if oid is None:
                continue  # fail-closed: do not attribute an unknown owner
            key = oid
            if tag_table is not None:
                t = tag_table.tag(oid)
                key = t if t else ("?%d" % oid)
            effp = self.u32(wb + OFF_W_EFFP) or 0
            r = regions.setdefault(rid, {"by_owner": {}, "leader": None, "total": 0.0})
            b = r["by_owner"].setdefault(key, {"effective_planes": 0.0, "wings": 0})
            b["effective_planes"] += effp
            b["wings"] += 1
            r["total"] += effp
            if len(regions) > MAX_REGIONS:
                raise RuntimeError("air-sup region count exceeds sanity bound")
        for rid, r in regions.items():
            if r["by_owner"]:
                r["leader"] = max(r["by_owner"], key=lambda k: r["by_owner"][k]["effective_planes"])
        return regions

    # ---- NAVAL / AIR COMBAT (build/live-state-gated; see naval_air_offsets.draft.json) ----
    def _classify_unit_ptr(self, ptr):
        """Identify a side-participant element pointer as ('div'|'tf'|'wing'|'ship', id) by
        vtable+type identity (trying ptr and ptr+0x10 for the handle-prefix case). None if it
        resolves to no known unit class."""
        if not self._isheap(ptr):
            return None
        for adj in (0, 0x10):
            p = ptr + adj
            vt = self.u64(p)
            if vt is None:
                continue
            t = self.u32(p + 0x08)
            i = self.u32(p + 0x0C)
            if vt == self.div_vt and t == DIV_TYPE:
                return ("div", i)
            if vt == self.tf_vt and t == TF_FLEET_TYPE:
                return ("tf", i)
            if vt == self.fleet_vt and t == TF_FLEET_TYPE:
                return ("fleet", i)
            if vt == self.wing_vt and t == WING_TYPE:
                return ("wing", i)
            if vt == self.ship_vt and t == SHIP_TYPE:
                return ("ship", i)
        return None

    def _side_participant_kinds(self, side):
        """Read a CCombatSide small-vector (data@+0x20, cap@+0x28, size@+0x2C) and return a list
        of classified participant tuples [('tf'|'wing'|'div'|...,id), ...], or None (fail-closed)
        on any header/element inconsistency. Double-reads the header (ticking game)."""
        if not self._isheap(side) or self.u64(side) != self.side_vt:
            self._note_reject("nc_bad_side_vtable"); return None
        data = self.u64(side + OFF_SIDE_DATA)
        cap = self.u32(side + OFF_SIDE_CAP)
        size = self.u32(side + OFF_SIDE_SIZE)
        if size is None or cap is None:
            self._note_reject("nc_hdr_read_fail"); return None
        if size == 0:
            return []
        if size > cap or size > MAX_SIDE_UNITS or not self._isheap(data):
            self._note_reject("nc_side_sane_bound"); return None
        blob = self._read_clamped(data, size * 8)
        if len(blob) < size * 8:
            self._note_reject("nc_short_read"); return None
        kinds = []
        for i in range(size):
            k = self._classify_unit_ptr(struct.unpack_from("<Q", blob, i * 8)[0])
            if k is None:
                self._note_reject("nc_unresolved_element"); return None
            kinds.append(k)
        if self.u64(side + OFF_SIDE_DATA) != data or self.u32(side + OFF_SIDE_SIZE) != size:
            self._note_reject("nc_race_changed"); return None
        return kinds

    def _enumerate_combats_by_participant(self, want_kinds):
        """Core discovery machinery for naval/air combat. Enumerates every CCombatSide object,
        reads + classifies its participants, and returns the combats whose ATTACKER side
        participants are of a kind in `want_kinds` (e.g. {'tf','fleet','ship'} for naval,
        {'wing'} for air). For each match it follows the side -> registry -> defender side and
        the front/location, mirroring the land reader. Returns a list of dicts.

        IMPORTANT (RE 2026-06-25): with the current build + live state this returns [] for naval
        and air - the land CCombatSide/registry classes are NOT reused for naval/air combat (the
        saved naval entries are transient submarine convoy-RAID contacts, day=-1/empty-defender,
        and are gone live). This method is the validated DETECTOR: it will light up automatically
        if a real fleet-vs-fleet / air battle ever instantiates a side of these classes. See the
        naval_air_combat block of naval_air_offsets.draft.json for the full negative + re-anchor.
        """
        out = []
        for side in self._scan_vtable(self.side_vt):
            kinds = self._side_participant_kinds(side)
            if not kinds:
                continue
            if not any(k[0] in want_kinds for k in kinds):
                continue
            reg = self.u64(side + OFF_SIDE_REG)
            if not self._isheap(reg) or self.u64(reg) != self.reg_vt:
                self._note_reject("nc_bad_registry"); continue
            handle = self.u64(reg + OFF_REG_HANDLE)
            cid = (handle >> 32) & 0xFFFFFFFF if handle else None
            htype = handle & 0xFFFFFFFF if handle else None
            def_side = self.u64(reg + OFF_REG_DEF)
            def_kinds = self._side_participant_kinds(def_side) if self._isheap(def_side) else None
            front = self.u64(reg + OFF_REG_FRONT)
            loc = self.u32(front + OFF_FRONT_LOC) if self._isheap(front) else None
            out.append({
                "combat_id": cid, "handle_type": htype, "location": loc,
                "attacker_units": kinds,
                "defender_units": def_kinds,
                "side_addr": side, "registry_addr": reg,
            })
        return out

    def enumerate_naval_combats(self):
        """Active NAVAL combats whose participants resolve to task forces / fleets / ships.

        Returns a list of combat dicts (see _enumerate_combats_by_participant). With the current
        build + live state this is [] - see the validated NEGATIVE in naval_air_offsets.draft.json
        (naval combat is not represented through the land combat side/registry path; the saved
        naval entries are transient raid contacts). This is the honest, build/live-state-gated
        result, NOT a fabricated offset set: the method runs the real discovery machinery and will
        return populated combats the moment a live fleet battle instantiates a task-force side."""
        return self._enumerate_combats_by_participant({"tf", "fleet", "ship"})

    def enumerate_air_combats(self):
        """Active AIR combats whose participants resolve to air wings.

        Returns a list of combat dicts. With the current build + live state this is [] (the save
        had 0 air combats ever, and no live air-wing combat side exists). Honest, build/live-state
        -gated result via the real discovery machinery - NOT fabricated offsets. Will populate the
        moment a live air battle instantiates an air-wing side. See naval_air_offsets.draft.json."""
        return self._enumerate_combats_by_participant({"wing"})

    # ---- RELAUNCH 2026-06-25: per-region / per-task-force aggregate-combat detector ----
    def detect_aggregate_combat_objects(self, unit_bodies=None):
        """Complementary build/live-state-gated detector for a NAVAL/AIR combat object that is NOT a
        land CCombatSide but a per-strategic-region or per-task-force aggregate (the hypotheses the
        relaunch pass tested and refuted; see naval_air_offsets.draft.json
        'pass2_relaunch_2026_06_25'). It enumerates strategic-region objects and task-force objects,
        and for every pointer field that targets an UNKNOWN-vtable object (not a known unit / side /
        registry / region class) it checks whether that target object holds known unit (tf/ship/wing
        /fleet) body pointers - a region/TF -> combat-object -> participants chain.

        Returns {"region_candidates": [...], "task_force_candidates": [...]} where each candidate is
        {source_addr, field_off, obj_addr, obj_vtable_off, unit_refs:[...], unit_kinds:[...]}. With the
        current build + live state BOTH lists are EMPTY (0/304 regions, 0/135 task forces) - the
        validated negative. It is NOT fabricated: it runs the real region/TF walk and will surface a
        candidate the moment a live battle instantiates a per-region or per-TF combat object holding
        unit references.

        Fail-closed against false positives (Codex-reviewed): the target object must be a REAL
        polymorphic object (first qword is a module-range vtable NOT in the known unit/side/registry/
        region set - POD/data arrays are rejected); participants are counted as DISTINCT body
        addresses (a unit listed twice is one unit); the source object's own address (and its handle-
        prefix forms) is excluded; and the TF path applies an OPPOSITION gate - it requires >=1
        referenced TASK FORCE / FLEET other than the source, so a task force's own member-ship array
        (ships only) can never masquerade as a 2-party encounter.

        Note: this closes the 'per-region / per-TF aggregate object holding DIRECT unit pointers'
        hypothesis only. It does NOT close index-based / vector-indirect / POD-pool / state-on-unit
        storage, nor a combat container that reuses a known manager/region vtable (Codex caveat) -
        those need a controlled before/during/after temporal diff.
        """
        REGION_VT = 0x29468B8
        REGION_VT2 = 0x2946908
        known_vts = {self.region_vt, self.region_vt2, self.dom_cont_vt, self.dom_ent_vt,
                     self.wing_vt, self.fleet_vt, self.tf_vt, self.ship_vt, self.pool_vt,
                     self.country_vt, self.side_vt, self.reg_vt, self.div_vt}

        # unit_kind: body-address (and its handle-prefix form) -> 'tf'|'fleet'|'wing'|'ship'.
        # Used to demand OPPOSITION on the TF path (Codex hardening): a task force legitimately holds
        # its OWN member SHIPS, so ">=2 ships" alone is a membership array, not a combat. A real
        # naval combat aggregates >=2 distinct TASK FORCES / FLEETS (the opposing sides). So the TF
        # path requires at least one referenced task-force/fleet OTHER than the source, never a bare
        # ship list. (The region path is already opposition-implied: a region holding ANY unit-bearing
        # unknown-vtable object is anomalous, since regions don't own units.)
        unit_kind = {}
        if unit_bodies is None:
            unit_bodies = set()
            for vt, typ, kind in ((TF_VT_OFF, TF_FLEET_TYPE, "tf"), (FLEET_VT_OFF, TF_FLEET_TYPE, "fleet"),
                                  (WING_VT_OFF, WING_TYPE, "wing"), (SHIP_VT_OFF, SHIP_TYPE, "ship")):
                for b in self._scan_vtable(self.module_base + vt, 0x08, typ):
                    unit_bodies.add(b); unit_kind[b] = kind
                    unit_bodies.add(b - 0x10); unit_kind[b - 0x10] = kind  # handle-prefix variant

        mod_lo = self.module_base
        mod_hi = self.module_base + 0x4000000

        def obj_holds_distinct_units(obj, span, exclude):
            """Return the set of DISTINCT known-unit body addresses referenced in obj's first
            `span` bytes, excluding `exclude` (the source object's own address + its handle-prefix
            form). Counting DISTINCT units (not repeats) is what makes the count a true multi-party
            signal, not the same unit listed twice."""
            blob = self._read_clamped(obj, span)
            hits = set()
            excl = {exclude, exclude + 0x10, exclude - 0x10}
            for off in range(0, max(0, len(blob) - 8), 8):
                q = struct.unpack_from("<Q", blob, off)[0]
                if q in unit_bodies and q not in excl:
                    hits.add(q)
            return hits

        def scan_fields(src, field_span, target_span, min_units, require_tf_or_fleet):
            cands = []
            raw = self._read_clamped(src, field_span)
            for off in range(0x10, max(0, len(raw) - 8), 8):
                q = struct.unpack_from("<Q", raw, off)[0]
                if not self._isheap(q):
                    continue
                tvt = self.u64(q)
                # require a REAL polymorphic object: first qword is a module-range vtable that is
                # NOT one of the known unit/side/registry/region classes. A non-module-range first
                # qword (POD / data array) is rejected here - those need a temporal diff, not this
                # passive single-snapshot detector (see scope_caveat_codex in the offsets json).
                if tvt is None or not (mod_lo <= tvt < mod_hi) or tvt in known_vts:
                    continue
                refs = obj_holds_distinct_units(q, target_span, exclude=src)
                if len(refs) < min_units:
                    continue
                # OPPOSITION gate (Codex): on the TF path require >=1 task-force/fleet OTHER than the
                # source, so a TF's own member-ship array can never masquerade as a 2-party encounter.
                if require_tf_or_fleet and not any(unit_kind.get(r) in ("tf", "fleet") for r in refs):
                    continue
                cands.append({"source_addr": src, "field_off": off, "obj_addr": q,
                              "obj_vtable_off": tvt - self.module_base,
                              "unit_refs": sorted(refs)[:8],
                              "unit_kinds": sorted(set(unit_kind.get(r) for r in refs))})
            return cands

        region_cands = []
        for ro in self._scan_vtable(self.region_vt):
            if self.u64(ro + 0x08) != self.region_vt2:
                continue
            rid = self.u32(ro + 0x58)
            if rid is None or not (1 <= rid <= MAX_REGION_ID):
                continue
            region_cands.extend(scan_fields(ro, 0x180, 0x100, min_units=1, require_tf_or_fleet=False))

        tf_cands = []
        for tf in self._scan_vtable(self.tf_vt, OFF_TF_TYPE, TF_FLEET_TYPE):
            tf_cands.extend(scan_fields(tf, 0x120, 0x120, min_units=2, require_tf_or_fleet=True))

        return {"region_candidates": region_cands, "task_force_candidates": tf_cands}

    def close(self):
        if self.h:
            _k32.CloseHandle(self.h)
            self.h = None


def main():
    r = NavalAirReader()
    ok, sha = r.verify_build()
    print("hoi4.exe pid=%d module_base=0x%X" % (r.pid, r.module_base))
    print("build sha256: %s" % sha)
    print("build gate: %s" % ("OK (matches pinned build)" if ok else "MISMATCH - refusing"))
    if not ok:
        print("Refusing to read on an unverified build (offsets are build-specific).")
        r.close()
        return

    # optional country-tag resolution
    tbl = None
    try:
        import os, sys
        sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
        import winmem  # noqa
        from live_layers import tag_table  # noqa
        p = winmem.open_process("hoi4.exe")
        if p is not None:
            tbl = tag_table.TagTable.locate(p)
    except Exception as e:
        print("(tag table unavailable: %s - showing raw tag-db-ids)" % e)

    print("\n=== NAVAL sea-zone control (region dominance / naval supremacy) ===")
    szc = r.enumerate_sea_zone_control(tag_table=tbl)
    print("regions with naval dominance: %d" % len(szc))
    for rid in sorted(szc):
        for e in szc[rid]:
            who = e.get("controller", "tagid=%d" % e["controller_tagid"])
            print("  region %3d : %-5s current=%9.3f target=%9.3f ratio=%.4f"
                  % (rid, who, e["current"], e["target"], e["individual_ratio"]))

    print("\n=== TASK-FORCE positions (Naval map markers) ===")
    tfs = r.enumerate_task_forces()
    print("live task forces: %d" % len(tfs))
    for t in tfs[:20]:
        print("  tf_id=%d province=%s" % (t["tf_id"], t["current_province_id"]))
    print("  ... (%d total)" % len(tfs))

    print("\n=== FLEETS ===")
    print("live fleets: %d" % len(r.enumerate_fleets()))

    print("\n=== AIR WINGS (id, count, owner, mission region, effective_planes) ===")
    wings = r.enumerate_air_wings(tag_table=tbl)
    print("live air wings: %d" % len(wings))
    n_owned = sum(1 for w in wings if w["owner_tagid"] is not None)
    n_onmission = sum(1 for w in wings if w["mission_region"] is not None)
    print("owner resolved: %d/%d ; on a region-bearing mission: %d"
          % (n_owned, len(wings), n_onmission))
    for w in wings[:20]:
        print("  wing_id=%-4d owner=%-5s region=%-5s count=%-4s eff_planes=%s"
              % (w["wing_id"], w.get("owner", w["owner_tagid"]),
                 w["mission_region"], w["count"], w["effective_planes"]))
    print("  ... (%d total)" % len(wings))

    print("\n=== AIR SUPERIORITY (the Air map mode: per-region per-owner sum eff_planes) ===")
    asup = r.enumerate_air_superiority(tag_table=tbl)
    print("regions with air activity: %d" % len(asup))
    for rid in sorted(asup):
        r_ = asup[rid]
        parts = ", ".join("%s=%.0f(%dw)" % (k, v["effective_planes"], v["wings"])
                          for k, v in sorted(r_["by_owner"].items(),
                                             key=lambda kv: -kv[1]["effective_planes"]))
        print("  region %3d : leader=%-5s | %s" % (rid, r_["leader"], parts))

    print("\n=== NAVAL / AIR COMBAT (build/live-state-gated detector) ===")
    nc = r.enumerate_naval_combats()
    ac = r.enumerate_air_combats()
    print("active naval combats (task-force/ship sides): %d" % len(nc))
    for c in nc[:20]:
        print("  naval combat_id=%s loc=%s atk=%s def=%s"
              % (c["combat_id"], c["location"], c["attacker_units"], c["defender_units"]))
    print("active air combats (air-wing sides): %d" % len(ac))
    for c in ac[:20]:
        print("  air combat_id=%s loc=%s atk=%s def=%s"
              % (c["combat_id"], c["location"], c["attacker_units"], c["defender_units"]))
    if not nc and not ac:
        print("  (none live via the CCombatSide path - see naval_air_offsets.draft.json")
        print("   'naval_air_combat': the saved naval entries are transient submarine convoy-raid")
        print("   contacts; the land combat side/registry classes are NOT reused for naval/air.)")

    print("\n=== AGGREGATE COMBAT DETECTOR (per-region / per-task-force; relaunch 2026-06-25) ===")
    agg = r.detect_aggregate_combat_objects()
    print("per-region combat-object candidates: %d" % len(agg["region_candidates"]))
    for c in agg["region_candidates"][:20]:
        print("  region@0x%X +0x%X -> obj@0x%X vt=%s units=%s"
              % (c["source_addr"], c["field_off"], c["obj_addr"],
                 ("MODULE+0x%X" % c["obj_vtable_off"]) if c["obj_vtable_off"] is not None else "?",
                 c["unit_refs"]))
    print("per-task-force combat-object candidates: %d" % len(agg["task_force_candidates"]))
    for c in agg["task_force_candidates"][:20]:
        print("  tf@0x%X +0x%X -> obj@0x%X vt=%s units=%s"
              % (c["source_addr"], c["field_off"], c["obj_addr"],
                 ("MODULE+0x%X" % c["obj_vtable_off"]) if c["obj_vtable_off"] is not None else "?",
                 c["unit_refs"]))
    if not agg["region_candidates"] and not agg["task_force_candidates"]:
        print("  (none - no per-region/per-TF aggregate combat object holds unit participants.")
        print("   Validated negative against the direct-object hypotheses; NOT a universal absence")
        print("   proof - index/vector-indirect/POD/distributed storage needs a before/during/after")
        print("   temporal diff of a real surface battle. See re_anchor_required_condition.5.)")

    if r.reject_stats:
        print("\nrejected reads (fail-closed, by reason): %s" % r.reject_stats)
    r.close()


if __name__ == "__main__":
    main()
