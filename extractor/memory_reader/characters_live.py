"""characters_live.py - READ-ONLY live CHARACTERS reader for Battle Plan.

Enumerates every military leader (generals = corps_commander, field marshals,
admirals = navy_leader), country (ruling) leaders, and advisor role-objects from a
running hoi4.exe, reading them LIVE via ReadProcessMemory only. The character pool
is located by RTTI-resolved vtables (no static pointer chain): the module image is
swept for the class type-descriptors, each vtable is resolved through its
CompleteObjectLocator, and committed heap is scanned for objects whose +0x00 qword
== the vtable. Instance counts match the save EXACTLY (see character_offsets.draft.json):
CCharacter 6675, CArmyLeader 770 (=600 generals+170 FMs), CNavyLeader 183 admirals.

CONFIRMED (PINNED) fields read here, each validated live==save (see the draft json):
  CArmyLeader / CNavyLeader body:
    +0x00 vtable, +0x08 type(==4713), +0x0C unit_leader_id (assignment + save join key),
    +0x20 std::string script token (e.g. SOV_georgy_zhukov), +0x40 std::string display name,
    +0xE68 i32 experience (fixed-point x100000; army 420/420, navy 87/87),
    +0xF48 i32 ==73 guard, +0xF4C u32 in-leader char_id,
    ARMY skills u8 attack/defense/planning/logistics @+0xF58/+0xF68/+0xF78/+0xF88 (770/770),
    NAVY skills u8 attack/defense/maneuvering/coordination @+0xF68/+0xF78/+0xF88/+0xF98 (183/183).
  CCharacter body:
    +0x00 vtable, +0x08 type(==73), +0x0C char_id, +0x20 std::string name,
    +0x68 u32 country tag_db_id (-> TAG via live_layers/tag_table; DIRECT 878/894, NULL 59/59=0,
    1:1 55/55 countries), +0xA8 -> the character's CArmyLeader/CNavyLeader (unit-leader bridge).
  CCountryLeader body (vtable 0x295A630, 833):
    +0x08 -> the CCharacter this country-leader ROLE belongs to (143/143 active rulers pointed to;
    760 leader-capable chars). The ACTIVE ruler among them is politics-side (RESISTED).

RESISTED (reported with a reason, not faked): overall LEVEL (`skill`) - derived from
accumulated XP via thresholds, no stored co-located field (purity-tested); the ACTIVE
ruling-leader pick (politics-side, blocked on the country-array locator); the trait
MEMBERSHIP SET + token decode (the leader+0x288.. flat_maps are trait_id->XP, readable
here as trait_ids+xp, but size != save ntraits and tokens need the global trait-def DB);
the CAPTURED flag (N=1 anchor). See character_offsets.draft.json.

HARD RULES honored (same as combat_enum.py / division_live_reader.py):
  * READ-ONLY memory (PROCESS_VM_READ | PROCESS_QUERY_INFORMATION); never writes/
    injects/suspends. Passive reads => no MP desync, any cadence. Keep the game PAUSED.
  * Build-gated: refuses to run unless the live hoi4.exe sha256 matches the pinned build.
  * Every fixed-size read is clamped to the VirtualQuery region end (a read straddling
    a region boundary returns 0 for the whole span - known bug).
  * ASCII-only output. Self-contained minimal ctypes helper (does NOT edit winmem.py).
"""

import ctypes as C
import struct
import hashlib
import re
import sys
from ctypes import wintypes as W

# --------------------------------------------------------------------------------------
# Build gate + solved constants (module-relative; see character_offsets.draft.json)
# --------------------------------------------------------------------------------------
EXPECTED_SHA256 = "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120"
PROC_NAME = "hoi4.exe"

# RTTI-resolved class vtables (module-relative). Validated: live heap instance counts
# match the save EXACTLY. Resolved at runtime from RTTI as a self-check (see
# _resolve_vtables); these RVAs are the cached fast path for THIS build.
VT_RVA = {
    "CCharacter":   0x2957D40,   # 6675 instances == save char defs
    "CArmyLeader":  0x292F788,   # 770  == 600 generals + 170 field marshals
    "CNavyLeader":  0x292F8A0,   # 183  == admirals (a 2nd vtable 0x292F9B8 hits the same objs)
    "CCountryLeader": 0x295A630, # 833  (one per character-ideology; ruling disambiguated by politics)
    "CAdvisor":     0x2980560,   # 4952 advisor role-objects
    "CCharacterManager": 0x27BF850,  # singleton
}

# Body field offsets (build 16022851)
OFF_VT          = 0x00
OFF_TYPE        = 0x08   # u32: 4713 for unit leaders, 73 for CCharacter
OFF_ID          = 0x0C   # u32: unit_leader_id (army/navy) OR char_id (CCharacter)
OFF_TOKEN       = 0x20   # std::string script token (army/navy) ; name (CCharacter)
OFF_NAME_ARMY   = 0x40   # std::string display name (army/navy leaders)
OFF_CHAR_NAME   = 0x20   # std::string display name (CCharacter)
OFF_CHAR_COUNTRY = 0x68  # u32 country tag_db_id (-> TAG via tag_table; 0 == null '---')
OFF_CHAR_UL_PTR = 0xA8   # CCharacter -> its CArmyLeader/CNavyLeader

# CCountryLeader (vtable 0x295A630): +0x08 -> the role-character's CCharacter.
OFF_CL_CHAR_PTR = 0x08

# Traits: a sequence of MSVC flat_map blocks in the leader body. The FIRST block
# header sits at +0x288 ({vtable@+0, begin_ptr@+0x10, u32 size@+0x18, u32 cap@+0x1C});
# the array at begin_ptr is records {u64 trait_id, i64 trait_xp} (stride 0x10).
# PARTIAL: trait_ids/xp are read here, but this is the trait-XP map (size != save
# ntraits) - NOT the membership set, and trait_id->token is not decoded. See draft.
OFF_TRAITS_BLOCK0 = 0x288
TRAIT_BLOCK_BEGIN = 0x10
TRAIT_BLOCK_SIZE  = 0x18
TRAIT_REC_STRIDE  = 0x10

# Deep stat fields in the CArmyLeader/CNavyLeader body (~0x1200 bytes). All PINNED
# 100% per-leader vs the lockstep save (see character_offsets.draft.json).
OFF_EXPERIENCE  = 0xE68   # i32 fixed-point x100000 (army 420/420, navy 87/87)
OFF_CHARID_IN_L = 0xF4C   # u32 owning char_id (770/770); type==73 @ +0xF48
# skill u8 records (stride 0x10): ARMY first record @ +0xF58, NAVY shifted +0x10.
OFF_ARMY_ATTACK    = 0xF58
OFF_ARMY_DEFENSE   = 0xF68
OFF_ARMY_PLANNING  = 0xF78
OFF_ARMY_LOGISTICS = 0xF88
OFF_NAVY_ATTACK       = 0xF68
OFF_NAVY_DEFENSE      = 0xF78
OFF_NAVY_MANEUVERING  = 0xF88
OFF_NAVY_COORDINATION = 0xF98
FP100K = 100000.0

UNIT_LEADER_TYPE = 4713
CHARACTER_TYPE   = 73

HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000
BODY_SPAN = 0x1000   # read deep enough to cover the stat region (~+0xFA0)
MAX_INSTANCES = 200000   # fail-closed sanity bound

# --------------------------------------------------------------------------------------
# Minimal read-only process access (self-contained; adapted from combat/division_live)
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


class CharactersLiveReader(object):
    """Read-only handle onto a running hoi4.exe with the character layout pinned."""

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
        self.vt = {k: self.module_base + rva for k, rva in VT_RVA.items()}
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

    def _read(self, addr, n):
        end = self._region_end(addr)
        if end is not None and addr + n > end:
            n = max(0, end - addr)
        if n == 0:
            return b""
        return self._rpm(addr, n) or b""

    def u32(self, a):
        b = self._read(a, 4)
        return None if len(b) < 4 else struct.unpack("<I", b)[0]

    def u64(self, a):
        b = self._read(a, 8)
        return None if len(b) < 8 else struct.unpack("<Q", b)[0]

    def read_msvc_str(self, so, maxlen=128):
        """MSVC std::string at `so`: SSO decided by capacity(+0x18)<16 else heap ptr@+0."""
        hdr = self._read(so, 0x20)
        if len(hdr) < 0x20:
            return None
        size = struct.unpack_from("<Q", hdr, 0x10)[0]
        cap = struct.unpack_from("<Q", hdr, 0x18)[0]
        if size == 0 or size > maxlen or cap < size or cap > (1 << 20):
            return None
        if cap < 16:
            data = hdr[:size]
        else:
            ptr = struct.unpack_from("<Q", hdr, 0)[0]
            data = self._read(ptr, size + 1)
            if len(data) < size + 1 or data[size] != 0:
                return None
            data = data[:size]
        try:
            s = data.decode("latin-1")
        except Exception:
            return None
        return s if all(32 <= ord(c) < 127 for c in s) else None

    # ---- build gate ----
    def verify_build(self):
        try:
            with open(self.exe_path, "rb") as f:
                digest = hashlib.sha256(f.read()).hexdigest()
        except Exception as e:
            return False, "exe-unreadable:%s" % e
        return (digest == EXPECTED_SHA256), digest

    # ---- region enumeration ----
    def _enum_regions(self):
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

    # ---- RTTI self-check: re-derive a vtable from RTTI and compare to the cached RVA ----
    def _resolve_vtable_rtti(self, class_name):
        """Return the absolute vtable addr for '.?AVClassName@@' via RTTI, or None.
        Used as a build self-check so a silent layout shift is caught."""
        ib = self.module_base
        # read the module image (committed regions within the module span)
        segs = []
        addr = ib
        end = ib + self.module_size
        mbi = _MBI()
        while addr < end:
            if _k32.VirtualQueryEx(self.h, C.c_void_p(addr), C.byref(mbi), C.sizeof(mbi)) == 0:
                break
            base = mbi.BaseAddress or addr
            size = mbi.RegionSize or 0x1000
            if mbi.State == 0x1000 and (mbi.Protect & 0xFF) in (0x02, 0x04, 0x08, 0x20, 0x40, 0x80):
                blob = self._rpm(base, min(size, end - base))
                if blob:
                    segs.append((base, blob))
            nxt = base + size
            addr = nxt if nxt > addr else addr + 0x1000
        needle = (".?AV" + class_name + "@@").encode()
        name_addr = None
        for b, blob in segs:
            i = blob.find(needle)
            if i != -1:
                name_addr = b + i
                break
        if name_addr is None:
            return None
        desc_rva = (name_addr - 0x10 - ib) & 0xFFFFFFFF
        # find COL: u32 == desc_rva at COL+0x0C, sig==1, pSelf RVA self-check
        col = None
        tgt = struct.pack("<I", desc_rva)
        for b, blob in segs:
            j = blob.find(tgt)
            while j != -1:
                col_addr = b + j - 0x0C
                sig = self.u32(col_addr)
                pself = self.u32(col_addr + 0x14)
                if sig == 1 and pself == ((col_addr - ib) & 0xFFFFFFFF):
                    col = col_addr
                    break
                j = blob.find(tgt, j + 1)
            if col:
                break
        if col is None:
            return None
        # vtable = &COL + 8
        cneedle = struct.pack("<Q", col)
        for b, blob in segs:
            k = blob.find(cneedle)
            if k != -1:
                return b + k + 8
        return None

    def rtti_self_check(self):
        """Confirm the cached vtable RVAs still match RTTI on this build."""
        report = {}
        for name in ("CCharacter", "CArmyLeader", "CNavyLeader"):
            got = self._resolve_vtable_rtti(name)
            want = self.vt[name]
            report[name] = {"ok": (got == want),
                            "rtti": ("0x%X" % got) if got else None,
                            "cached": "0x%X" % want}
        return report

    # ---- instance enumeration by vtable ----
    def find_instances(self, vt_abs, limit=MAX_INSTANCES):
        needle = struct.pack("<Q", vt_abs)
        out = []
        CH = 16 << 20
        for base, size in self._enum_regions():
            pos = 0
            while pos < size:
                want = min(CH, size - pos)
                blob = self._rpm(base + pos, want)
                if blob:
                    i = blob.find(needle)
                    while i != -1:
                        addr = base + pos + i
                        if addr % 8 == 0:
                            out.append(addr)
                            if len(out) >= limit:
                                return out
                        i = blob.find(needle, i + 1)
                    pos += max(1, len(blob) - 16)
                else:
                    pos += max(1, want)
        return out

    # ---- per-object readers ----
    def read_unit_leader(self, addr, kind):
        """One CArmyLeader/CNavyLeader -> dict. kind in {'army','navy'}.

        Reads the PINNED identity + deep stat fields. The 4 category skills and
        experience match the save 100%; the overall `level`/`skill` is RESISTED
        (derived from the categories, not a stored co-located field)."""
        b = self._read(addr, BODY_SPAN)
        if len(b) < 0xFA0:
            return None
        if struct.unpack_from("<I", b, OFF_TYPE)[0] != UNIT_LEADER_TYPE:
            return None
        # guard the deep stat header: type==73 @ +0xF48 (the in-leader char header)
        deep_type = struct.unpack_from("<I", b, 0xF48)[0]
        exp_raw = struct.unpack_from("<i", b, OFF_EXPERIENCE)[0]
        rec = {
            "addr": "0x%X" % addr,
            "kind": kind,
            "unit_leader_id": struct.unpack_from("<I", b, OFF_ID)[0],
            "token": self.read_msvc_str(addr + OFF_TOKEN),
            "name": self.read_msvc_str(addr + OFF_NAME_ARMY),
            "char_id_in_leader": (struct.unpack_from("<I", b, OFF_CHARID_IN_L)[0]
                                  if deep_type == CHARACTER_TYPE else None),
            "experience": (exp_raw / FP100K) if deep_type == CHARACTER_TYPE else None,
            # RESISTED - overall level/skill (see module docstring / draft json):
            "level": None, "_unpinned": ["level"],
        }
        if kind == "army":
            rec.update({
                "attack":    b[OFF_ARMY_ATTACK],
                "defense":   b[OFF_ARMY_DEFENSE],
                "planning":  b[OFF_ARMY_PLANNING],
                "logistics": b[OFF_ARMY_LOGISTICS],
                "maneuvering": None, "coordination": None,
            })
        else:
            rec.update({
                "attack":       b[OFF_NAVY_ATTACK],
                "defense":      b[OFF_NAVY_DEFENSE],
                "maneuvering":  b[OFF_NAVY_MANEUVERING],
                "coordination": b[OFF_NAVY_COORDINATION],
                "planning": None, "logistics": None,
            })
        return rec

    def read_character(self, addr):
        b = self._read(addr, BODY_SPAN)
        if len(b) < 0xB0:
            return None
        if struct.unpack_from("<I", b, OFF_TYPE)[0] != CHARACTER_TYPE:
            return None
        return {
            "addr": "0x%X" % addr,
            "char_id": struct.unpack_from("<I", b, OFF_ID)[0],
            "name": self.read_msvc_str(addr + OFF_CHAR_NAME),
            # PINNED country tag_db_id (resolve to TAG with a tag table; 0 == null '---').
            "country_tag_db_id": struct.unpack_from("<I", b, OFF_CHAR_COUNTRY)[0],
            "unit_leader_ptr": struct.unpack_from("<Q", b, OFF_CHAR_UL_PTR)[0],
        }

    def read_trait_xp(self, leader_addr, max_traits=64):
        """PARTIAL: read the FIRST trait-XP flat_map block on a leader.

        Returns a list of {trait_id, trait_xp} (trait_xp is the raw fixed-point i64).
        NOTE this is the trait-XP/progress map, NOT the membership SET (its size can
        differ from the save ntraits by +/-2), and trait_id is NOT decoded to a token
        (needs the global trait-def DB). See character_offsets.draft.json -> traits_xp_map."""
        hdr = self._read(leader_addr + OFF_TRAITS_BLOCK0, 0x20)
        if len(hdr) < 0x20:
            return []
        begin = struct.unpack_from("<Q", hdr, TRAIT_BLOCK_BEGIN)[0]
        size = struct.unpack_from("<I", hdr, TRAIT_BLOCK_SIZE)[0]
        if not (HEAP_LO <= begin < HEAP_HI) or not (0 < size <= max_traits):
            return []
        raw = self._read(begin, size * TRAIT_REC_STRIDE)
        out = []
        if len(raw) < size * TRAIT_REC_STRIDE:
            # short read near a region boundary -> the array is incomplete; signal it
            # rather than silently returning a truncated set.
            return [{"_partial": True, "_read_records": len(raw) // TRAIT_REC_STRIDE,
                     "_expected": size}]
        for i in range(min(size, len(raw) // TRAIT_REC_STRIDE)):
            tid = struct.unpack_from("<Q", raw, i * TRAIT_REC_STRIDE)[0]
            txp = struct.unpack_from("<q", raw, i * TRAIT_REC_STRIDE + 8)[0]
            if 0 < tid < (1 << 20):   # sane trait id; reject the trailing end sentinel
                out.append({"trait_id": tid, "trait_xp": txp})
        return out

    def enumerate_country_leaders(self):
        """PINNED role link: every CCountryLeader role-object -> its CCharacter.

        Returns a list of {addr, char_id, name} for the 760-ish leadership-capable
        characters. The ACTIVE ruling leader among a country's entries is politics-side
        and NOT determined here (RESISTED; see draft json -> active_ruler_pick)."""
        out = []
        for a in self.find_instances(self.vt["CCountryLeader"]):
            b = self._read(a, 0x10)
            if len(b) < 0x10 or struct.unpack_from("<Q", b, OFF_VT)[0] != self.vt["CCountryLeader"]:
                continue
            cptr = struct.unpack_from("<Q", b, OFF_CL_CHAR_PTR)[0]
            cid = name = None
            if HEAP_LO <= cptr < HEAP_HI:
                cb = self._read(cptr, 0x30)
                if len(cb) >= 0x10 and struct.unpack_from("<I", cb, OFF_TYPE)[0] == CHARACTER_TYPE:
                    cid = struct.unpack_from("<I", cb, OFF_ID)[0]
                    name = self.read_msvc_str(cptr + OFF_CHAR_NAME)
            out.append({"addr": "0x%X" % a, "char_id": cid, "name": name})
        return out

    def _tag_resolver(self):
        """Best-effort live tag_db_id -> TAG resolver via the sibling tag_table.
        Returns a dict {tag_db_id: TAG} or None if the table can't be located."""
        try:
            import winmem  # noqa: F401  (only to confirm the sibling stack is importable)
            from live_layers import tag_table  # type: ignore
            import winmem as _wm
            p = _wm.open_process(self.proc_name)
            return tag_table.TagTable.locate(p).as_map()
        except Exception:
            return None

    # ---- warm-walk address cache (located once cold, re-read live on warm walks) ----
    def _trait_block_fp(self, leader_addr, max_traits=64):
        """Per-leader trait flat_map CONTENT fingerprint at leader+OFF_TRAITS_BLOCK0 -- the
        external sub-array read_trait_xp() decodes. Snapshotted at locate time and re-checked by
        validate_addrs(), so the warm cache may serve the cached DECODED trait_xp ONLY while the
        trait array is byte-identical.

        Returns (begin, size, content_hash): begin+size catch a realloc/resize, and content_hash
        (an md5 over the exact trait records) catches an IN-PLACE, SAME-SIZE trait edit -- e.g. a
        commander TRAIT ASSIGNED while the game is paused (Codex: the trait-assignment UI works
        while paused, so a begin+size-only fingerprint would let a stale cached roster through).
        This is STRICTLY stronger than the designs_variants ptr+size payload fp -- the trait array
        is tiny (<=64 * 0x10 bytes), so hashing it on each warm validate is cheap relative to the
        per-leader re-decode it lets us skip. (None, None, None) on an unreadable/insane header."""
        hdr = self._read(leader_addr + OFF_TRAITS_BLOCK0, 0x20)
        if len(hdr) < 0x20:
            return (None, None, None)
        begin = struct.unpack_from("<Q", hdr, TRAIT_BLOCK_BEGIN)[0]
        size = struct.unpack_from("<I", hdr, TRAIT_BLOCK_SIZE)[0]
        if not (HEAP_LO <= begin < HEAP_HI) or not (0 <= size <= max_traits):
            # an empty (size 0) or insane block: hash is None, but begin+size still pin it.
            return (begin, size, None)
        raw = self._read(begin, size * TRAIT_REC_STRIDE)
        if len(raw) < size * TRAIT_REC_STRIDE:
            return (begin, size, None)            # short read -> can't hash; begin+size still pin
        return (begin, size, hashlib.md5(raw).hexdigest())

    def enumerate_addrs(self):
        """LOCATE every leader/character body address WITHOUT decoding any fields.

        Returns the exact addr sets enumerate()/enumerate_country_leaders() iterate via
        find_instances, for the warm-walk cache:
          {"army": [addr,...], "navy": [addr,...], "char": [addr,...],
           "country_leader": [addr,...], "_ids": {addr: expected_stable_id, ...},
           "_traits": {leader_addr: (begin, size, content_hash), ...}}.
        The "_ids" map is the ABA-defense snapshot taken at locate time (army/navy/char ->
        +0x0C id; country_leader -> +0x08 CCharacter ptr, its only stable identity anchor),
        compared by validate_addrs() so a freed-then-same-class-reused slot is caught even
        if the paused-game invariant is violated. The "_traits" map fingerprints each army/
        navy leader's external trait flat_map (begin ptr, size, CONTENT hash) so validate_addrs()
        proves the DECODED trait sub-array is byte-identical -- the content hash catches an
        in-place SAME-SIZE trait edit (a trait assigned while PAUSED), which a ptr+size-only fp
        would miss (needed because the warm cache now serves the cached DECODED enumeration).
        enumerate_from_addrs() consumes ONLY the address lists (the "_ids"/"_traits" keys are
        inert to decoding).
        This is the COLD step (four full committed-heap vtable sweeps). On a PAUSED game
        the located addresses are stable across walks (no alloc/free) - validate_addrs()
        gates re-use and forces a full re-scan fail-closed if any body moved/freed/reused."""
        army = list(self.find_instances(self.vt["CArmyLeader"]))
        navy = list(self.find_instances(self.vt["CNavyLeader"]))
        char = list(self.find_instances(self.vt["CCharacter"]))
        cl = list(self.find_instances(self.vt["CCountryLeader"]))
        ids = {}
        for a in army:
            ids[a] = self.u32(a + OFF_ID)        # unit_leader_id
        for a in navy:
            ids[a] = self.u32(a + OFF_ID)        # unit_leader_id
        for a in char:
            ids[a] = self.u32(a + OFF_ID)        # char_id
        for a in cl:
            ids[a] = self.u64(a + OFF_CL_CHAR_PTR)  # role-character ptr (stable identity)
        # trait flat_map fingerprint for the leaders read_trait_xp() decodes (army + navy).
        traits = {}
        for a in army:
            traits[a] = self._trait_block_fp(a)
        for a in navy:
            traits[a] = self._trait_block_fp(a)
        return {"army": army, "navy": navy, "char": char,
                "country_leader": cl, "_ids": ids, "_traits": traits}

    def validate_addrs(self, addr_sets):
        """FAIL-CLOSED liveness gate over a cached addr_sets (no heap sweep).

        For EVERY address in army/navy/char: re-read +0x00 vtable (== the expected class
        vtable for that set), +0x08 type (==4713 for army/navy, ==73 for char), and
        +0x0C id (== the id snapshotted at locate time). For country_leader: re-check
        +0x00 vtable AND +0x08 CCharacter ptr (== the snapshotted role-character ptr).
        The cached-id/ptr EQUALITY check is the real ABA defense: a freed slot reused by a
        SAME-CLASS object passes vtable+type but its id/ptr differs -> fail. vtable+type
        reject a different-class reuse; an unreadable field reads None != the cached value
        -> fail. Returns True iff EVERY supplied address still validates; any mismatch or
        unreadable field -> False. Validates ALL addresses (not a sentinel sample).
        Empty sets validate vacuously (the addr_sets ARE the count).
        If "_ids" is absent (a hand-built addr_sets), the id-equality step is skipped and
        only vtable+type+readability are enforced (still fail-closed on class/readability)."""
        ids = addr_sets.get("_ids") or {}
        checks = (
            ("army", self.vt["CArmyLeader"], UNIT_LEADER_TYPE),
            ("navy", self.vt["CNavyLeader"], UNIT_LEADER_TYPE),
            ("char", self.vt["CCharacter"], CHARACTER_TYPE),
        )
        for key, want_vt, want_type in checks:
            for a in addr_sets.get(key, ()):
                vt = self.u64(a + OFF_VT)
                if vt != want_vt:
                    return False
                ty = self.u32(a + OFF_TYPE)
                if ty != want_type:
                    return False
                cur_id = self.u32(a + OFF_ID)
                # ABA defense: id must be readable AND equal the locate-time snapshot.
                # (vtable+type reject a different-class reuse; the id-equality rejects a
                # same-class reuse and an unreadable slot, which reads None.)
                if cur_id is None:
                    return False
                if a in ids and cur_id != ids[a]:
                    return False
        want_cl_vt = self.vt["CCountryLeader"]
        for a in addr_sets.get("country_leader", ()):
            if self.u64(a + OFF_VT) != want_cl_vt:
                return False
            cur_ptr = self.u64(a + OFF_CL_CHAR_PTR)
            if cur_ptr is None:
                return False
            if a in ids and cur_ptr != ids[a]:
                return False
        # PAYLOAD fingerprint (designs_variants parity): re-check each army/navy leader's
        # external trait flat_map (begin ptr, size). The vtable+type+id check above proves the
        # leader BODY is the same object; this proves its decoded trait sub-array is structurally
        # unchanged, so the warm cache may serve the cached DECODED trait_xp. Absent on a hand-
        # built addr_sets (no "_traits") -> skipped, like the "_ids" step.
        traits = addr_sets.get("_traits")
        if isinstance(traits, dict):
            for key in ("army", "navy"):
                for a in addr_sets.get(key, ()):
                    if a in traits and self._trait_block_fp(a) != traits[a]:
                        return False
        return True

    # ---- top-level enumeration ----
    def enumerate(self, resolve_country=True, read_traits=True):
        """Return a dict with army leaders, navy leaders, characters, and counts.

        resolve_country: attach 'country' (3-letter TAG) to each leader/character via
          the live tag table (best-effort; falls back to the raw tag_db_id if the
          table can't be located - e.g. dynamic D-pool ids >= the table extent).
        read_traits: attach 'trait_xp' (list of {trait_id, trait_xp}) to each leader
          from the first trait-XP flat_map (PARTIAL - ids only, not the membership set).

        Single decode path: LOCATE the body addresses (cold sweep) then decode them.
        Warm walks can skip the sweep by caching enumerate_addrs() and (after
        validate_addrs() passes) calling enumerate_from_addrs() with the cached set."""
        addr_sets = self.enumerate_addrs()
        return self.enumerate_from_addrs(addr_sets, resolve_country=resolve_country,
                                         read_traits=read_traits)

    def enumerate_from_addrs(self, addr_sets, resolve_country=True, read_traits=True):
        """Same dict as enumerate(), but iterate the SUPPLIED addr_sets (no heap sweep).

        Re-reads EVERY field LIVE from the cached addresses - only the located addresses
        are cached, never the values. Output is byte-identical to enumerate() when
        addr_sets == enumerate_addrs() over the same PAUSED heap."""
        army = [r for a in addr_sets["army"]
                if (r := self.read_unit_leader(a, "army"))]
        navy = [r for a in addr_sets["navy"]
                if (r := self.read_unit_leader(a, "navy"))]
        chars = {}
        ul_to_char = {}
        for a in addr_sets["char"]:
            r = self.read_character(a)
            if r:
                chars[r["char_id"]] = r
                if HEAP_LO <= r["unit_leader_ptr"] < HEAP_HI:
                    ul_to_char[r["unit_leader_ptr"]] = r["char_id"]

        tagmap = self._tag_resolver() if resolve_country else None
        if tagmap is not None:
            for c in chars.values():
                tid = c.get("country_tag_db_id")
                # tag_db_id 0 -> "---" (the engine's null tag); ids >= the located
                # table extent (dynamic D-pool) resolve to None until the table covers them.
                c["country"] = tagmap.get(tid)

        # bridge each unit leader back to its character via the addr the char points at
        for lst in (army, navy):
            for r in lst:
                cid = ul_to_char.get(int(r["addr"], 16))
                r["char_id"] = cid
                if cid is not None and cid in chars:
                    r["char_name"] = chars[cid]["name"]
                    r["country_tag_db_id"] = chars[cid].get("country_tag_db_id")
                    if tagmap is not None:
                        r["country"] = chars[cid].get("country")
                if read_traits:
                    r["trait_xp"] = self.read_trait_xp(int(r["addr"], 16))  # PARTIAL
        return {
            "army_leaders": army,    # generals + field marshals (role split needs skills - RESISTED)
            "navy_leaders": navy,    # admirals
            "characters": chars,
            "counts": {
                "army_leaders": len(army),
                "navy_leaders": len(navy),
                "characters": len(chars),
            },
        }


# --------------------------------------------------------------------------------------
# CLI / validation
# --------------------------------------------------------------------------------------
def _load_save_anchors(path):
    import json
    try:
        return json.load(open(path))
    except Exception:
        return None


def main():
    import argparse, os
    ap = argparse.ArgumentParser(description="Live the game character reader (read-only).")
    ap.add_argument("--force", action="store_true",
                    help="run even if the exe sha256 doesn't match the pinned build")
    ap.add_argument("--anchors", default=None,
                    help="path to _anchors_save.json for a live==save validation report")
    args = ap.parse_args()

    r = CharactersLiveReader()
    ok, digest = r.verify_build()
    if not ok and not args.force:
        raise SystemExit("build gate: live exe sha256 %s != pinned %s (pass --force to override)"
                         % (digest, EXPECTED_SHA256))
    print("pid %d module_base 0x%X sha %s%s"
          % (r.pid, r.module_base, digest[:16], "" if ok else "  (MISMATCH - --force)"))

    # RTTI self-check
    chk = r.rtti_self_check()
    for name, c in chk.items():
        print("  rtti %-14s cached=%s rtti=%s %s"
              % (name, c["cached"], c["rtti"], "OK" if c["ok"] else "** MISMATCH **"))

    out = r.enumerate()
    c = out["counts"]
    print("\nlive counts: army_leaders=%d navy_leaders=%d characters=%d"
          % (c["army_leaders"], c["navy_leaders"], c["characters"]))

    # spot-check a few named leaders
    print("\nsample army leaders (a/d/p/l skills + xp):")
    for rec in out["army_leaders"][:6]:
        print("   ul_id=%-6d %-22s a/d/p/l=%s/%s/%s/%s xp=%s token=%s"
              % (rec["unit_leader_id"], (rec.get("char_name") or rec.get("name") or "?")[:22],
                 rec["attack"], rec["defense"], rec["planning"], rec["logistics"],
                 ("%.2f" % rec["experience"]) if rec.get("experience") is not None else "?",
                 rec.get("token")))
    print("sample navy leaders (a/d/m/c skills + xp):")
    for rec in out["navy_leaders"][:4]:
        print("   ul_id=%-6d %-22s a/d/m/c=%s/%s/%s/%s xp=%s token=%s"
              % (rec["unit_leader_id"], (rec.get("char_name") or rec.get("name") or "?")[:22],
                 rec["attack"], rec["defense"], rec["maneuvering"], rec["coordination"],
                 ("%.2f" % rec["experience"]) if rec.get("experience") is not None else "?",
                 rec.get("token")))

    if args.anchors:
        anc = _load_save_anchors(args.anchors)
        if anc:
            save = anc.get("counts", {})
            n_army_save = save.get("generals", 0) + save.get("field_marshals", 0)
            print("\n=== live vs save ===")
            print("army leaders: live %d  save %d (%d gen + %d FM)  %s"
                  % (c["army_leaders"], n_army_save, save.get("generals", 0),
                     save.get("field_marshals", 0),
                     "MATCH" if c["army_leaders"] == n_army_save else "DIFF"))
            print("navy leaders: live %d  save %d  %s"
                  % (c["navy_leaders"], save.get("admirals", 0),
                     "MATCH" if c["navy_leaders"] == save.get("admirals", 0) else "DIFF"))
            # identity validation: every save unit_leader_id present live, with matching token
            by_ulid = {a["unit_leader_id"]: a for a in anc.get("military", [])
                       if a.get("unit_leader_id")}
            live_by_ul = {x["unit_leader_id"]: x for x in out["army_leaders"] + out["navy_leaders"]}
            tok_ok = tok_tot = present = 0
            skill_ok = {f: [0, 0] for f in
                        ("attack", "defense", "planning", "logistics",
                         "maneuvering", "coordination", "experience")}
            for ul, sa in by_ulid.items():
                lv = live_by_ul.get(ul)
                if lv is None:
                    continue
                present += 1
                if lv.get("token"):
                    tok_tot += 1
                    if lv["token"] == sa.get("name"):
                        tok_ok += 1
                for f in skill_ok:
                    sval = sa.get(f)
                    lval = lv.get(f)
                    if sval is None or lval is None:
                        continue
                    skill_ok[f][1] += 1
                    if f == "experience":
                        if abs(lval - sval) < 0.001:
                            skill_ok[f][0] += 1
                    elif lval == sval:
                        skill_ok[f][0] += 1
            print("identity: %d/%d save unit_leader_ids present live; token==save-name %d/%d"
                  % (present, len(by_ulid), tok_ok, tok_tot))
            print("PINNED field validation (live == save):")
            for f in ("attack", "defense", "planning", "logistics",
                      "maneuvering", "coordination", "experience"):
                ok, tot = skill_ok[f]
                if tot:
                    print("   %-12s %d/%d (%.0f%%)" % (f, ok, tot, 100.0 * ok / tot))
            print("level/skill (overall): RESISTED (derived from XP via thresholds) - see draft json.")

            # --- COUNTRY validation (CCharacter+0x68 -> TAG via tag table) ---
            chars = out["characters"]
            mil_by_cid = {}
            for m in anc.get("military", []):
                cid = m.get("char_id")
                if cid is not None and m.get("country") and cid not in mil_by_cid:
                    mil_by_cid[cid] = m["country"]
            tagmap = r._tag_resolver()
            if tagmap is not None:
                inv = {v: k for k, v in tagmap.items()}
                direct_ok = direct_tot = null_ok = null_tot = 0
                for m in anc.get("military", []):
                    cid = m.get("char_id"); sc = m.get("country")
                    lv = chars.get(cid)
                    if lv is None:
                        continue
                    tid = lv.get("country_tag_db_id")
                    if sc is None:
                        null_tot += 1
                        if tid == 0:
                            null_ok += 1
                    else:
                        direct_tot += 1
                        if inv.get(sc) == tid:
                            direct_ok += 1
                print("\n=== COUNTRY (CCharacter+0x68 tag_db_id -> TAG) [PINNED] ===")
                if direct_tot:
                    print("   direct live->save TAG: %d/%d (%.1f%%)  null-country->0: %d/%d  "
                          "(misses are dynamic D-pool ids >= tag-table extent)"
                          % (direct_ok, direct_tot, 100.0 * direct_ok / direct_tot, null_ok, null_tot))
            else:
                print("\n=== COUNTRY: tag table unavailable this run; raw tag_db_id still read at +0x68 ===")

            # --- RULING-LEADER ROLE link (CCountryLeader+0x08 -> CCharacter) ---
            cls = r.enumerate_country_leaders()
            cl_cids = set(x["char_id"] for x in cls if x["char_id"] is not None)
            ruler_cids = set(l["char_id"] for l in anc.get("leaders", []))
            covered = len(ruler_cids & cl_cids)
            print("=== COUNTRY-LEADER ROLE link (CCountryLeader+0x08) [PINNED role; active-pick RESISTED] ===")
            print("   role objects: %d  leadership-capable chars: %d  active rulers covered: %d/%d"
                  % (len(cls), len(cl_cids), covered, len(ruler_cids)))

            # --- TRAITS (PARTIAL: trait-XP flat_map ids; not the membership set) ---
            zh = live_by_ul.get(676)
            if zh and zh.get("trait_xp"):
                ids = [t["trait_id"] for t in zh["trait_xp"]]
                print("=== TRAITS (leader+0x288 flat_map trait_id->xp) [PARTIAL; ids only] ===")
                print("   Zhukov ul=676 trait_xp entries=%d sample trait_ids=%s (token decode RESISTED)"
                      % (len(ids), ids[:8]))


if __name__ == "__main__":
    main()
