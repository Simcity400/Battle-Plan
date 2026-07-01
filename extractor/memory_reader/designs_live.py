"""designs_live.py - READ-ONLY live DESIGNS reader for Battle Plan.

Reads the DESIGNS domain from a running hoi4.exe (build 16022851), READ-ONLY:

  DIVISION TEMPLATES (full, validated 649/649 vs a time-aligned melted save):
    id            int    template id           (body+0x0C)
    name          str    custom name / loc key (inline std::string @ body+0x20)
    priority      int    production priority    (body+0x1A4)        0/1/2
    division_cap  int    max simultaneous divs  (body+0x254)
    n_regiments   int    regiment count
    n_support     int    support-company count  (body+0x94)
    regiments     [str]  ordered unit-type keys (array@body+0x68, len@body+0x74)
    support       [str]  support-company keys   (array@body+0x88, len@body+0x94)

  Each regiment/support slot is a sub_unit_def* (own vtable MODULE+0x2763BB0); the
  unit-type key is an inline std::string at def+0x18. Slots whose vtable is not the
  sub_unit_def vtable are placeholders and are skipped (vtable gate).

  EQUIPMENT VARIANTS: see equipment-variant section below once the variant-pool crack
  is merged (offsets in designs_offsets.draft.json -> equipment_variant_pool).

Offsets: designs_offsets.draft.json (build 16022851). Validation against the time-aligned
melt re_scratch/melt_sov_1946.txt (1946.5.19.9): template id set 649/649, name 506/506
(byte-exact incl. non-ASCII; the other 143 carry only a loc key), priority 649/649,
n_support 649/649, regiment composition 649/649 EXACT tally, support composition 649/649
EXACT tally.

HARD RULES (same as division_live_reader.py / combat_enum.py):
  * READ-ONLY memory (PROCESS_VM_READ | PROCESS_QUERY_INFORMATION); never writes/injects/
    suspends. Passive reads => no MP desync, any cadence. Keep the game observed, not touched.
  * Build-gated: refuses to run unless live hoi4.exe sha256 matches the pinned build.
  * Every fixed-size read is clamped to the VirtualQuery region end (a read straddling a
    region boundary returns 0 for the whole span - known bug).
  * Generic heap test 0x10000000000 <= v < 0x7FF000000000 for pointer validity.
  * ASCII-only output.

Self-contained minimal ctypes helper (adapted from the proven division/combat pattern);
does NOT edit winmem.py / locate.py / the enumerator. When productionized these offsets
merge into the per-build layout.
"""

import ctypes as C
import struct
import hashlib
from ctypes import wintypes as W

EXPECTED_SHA256 = "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120"
PROC_NAME = "hoi4.exe"

# module-relative vtables / type tags (see designs_offsets.draft.json)
TMPL_VT_OFF    = 0x29291D0     # CDivisionTemplate vtable
SUBUNIT_VT_OFF = 0x2763BB0     # sub_unit_def vtable (regiment/support entries)
TEMPLATE_TYPE  = 52            # type tag @ template body+0x08

# CDivisionTemplate body field offsets (build 16022851)
OFF_TYPE        = 0x08         # u32 == 52
OFF_ID          = 0x0C         # u32 template id
OFF_NAME        = 0x20         # inline std::string (custom name / loc key)
OFF_REG_ARR     = 0x68         # u64 -> regiment def-pointer array (stride 8)
OFF_REG_LEN     = 0x74         # u32 array length (= +0x70 high dword)
OFF_SUP_ARR     = 0x88         # u64 -> support def-pointer array (stride 8)
OFF_SUP_LEN     = 0x94         # u32 support company count (= n_support)
OFF_PRIORITY    = 0x1A4        # u32 priority (0/1/2)
OFF_DIV_CAP     = 0x254        # u32 division cap

# sub_unit_def
OFF_SUBUNIT_KEY = 0x18         # inline std::string = unit-type key

BODY_SPAN = 0x260             # covers all template scalar fields
HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000
MAX_TEMPLATES = 20000
MAX_ENTRIES   = 64           # sane bound on regiment/support array length

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


class DesignsLiveReader(object):
    """Read-only handle onto a running hoi4.exe with the designs layout pinned."""

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
        self.tmpl_vt = self.module_base + TMPL_VT_OFF
        self.subunit_vt = self.module_base + SUBUNIT_VT_OFF
        self._regions = None
        self._key_cache = {}

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
        if self._regions is not None:
            return self._regions
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
        self._regions = out
        return out

    # ---- build gate ----
    def verify_build(self):
        try:
            with open(self.exe_path, "rb") as f:
                digest = hashlib.sha256(f.read()).hexdigest()
        except Exception as e:
            return False, "exe-unreadable:%s" % e
        return (digest == EXPECTED_SHA256), digest

    # ---- helpers ----
    @staticmethod
    def _u32(b, o):
        return struct.unpack_from("<I", b, o)[0]

    @staticmethod
    def _u64(b, o):
        return struct.unpack_from("<Q", b, o)[0]

    def _is_heap(self, p):
        return HEAP_LO <= p < HEAP_HI

    def _read_inline_str(self, base_addr, maxlen=200):
        """Read an inline MSVC std::string located AT base_addr (ptr,size,cap). SSO decided
        by capacity@+0x18. Returns a latin-1 str (non-ASCII preserved) or None."""
        hdr = self._read_clamped(base_addr, 0x20)
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
            if not self._is_heap(ptr):
                return None
            data = self._read_clamped(ptr, size + 1)
            if len(data) < size + 1 or data[size] != 0:
                return None
            data = data[:size]
        try:
            return data.decode("latin-1")
        except Exception:
            return None

    def _subunit_key(self, defp):
        """sub_unit_def* -> unit-type key (inline std::string @ +0x18), vtable-gated. Cached."""
        if defp in self._key_cache:
            return self._key_cache[defp]
        key = None
        hdr = self._read_clamped(defp, 0x20)
        if len(hdr) >= 8 and struct.unpack_from("<Q", hdr, 0)[0] == self.subunit_vt:
            key = self._read_inline_str(defp + OFF_SUBUNIT_KEY, maxlen=64)
        self._key_cache[defp] = key
        return key

    def _read_def_array(self, body, arr_off, len_off):
        """Read a regiment/support sub_unit_def* array; return ordered list of unit-type keys
        (vtable-gated; placeholder slots skipped)."""
        n = self._u32(body, len_off)
        bufp = self._u64(body, arr_off)
        if n == 0 or n > MAX_ENTRIES or not self._is_heap(bufp):
            return []
        buf = self._read_clamped(bufp, n * 8)
        keys = []
        for i in range(n):
            if i * 8 + 8 > len(buf):
                break
            v = struct.unpack_from("<Q", buf, i * 8)[0]
            if self._is_heap(v):
                k = self._subunit_key(v)
                if k:
                    keys.append(k)
        return keys

    # ---- enumeration ----
    def locate_template_bodies(self):
        """Scan committed heap for the template vtable and return the located bodies as an ORDERED
        list of (body_addr, template_id) in scan order (per-address dedupe + the type==52 gate).
        This is the dominant cost; the pin caches THIS list and re-decodes off it on later walks
        (read_templates_from_bodies). Output identity: enumerate_templates ==
        read_templates_from_bodies(locate_template_bodies())."""
        vt_pat = struct.pack("<Q", self.tmpl_vt)
        seen = set()
        bodies = []                                    # ordered [(cand, template_id)]
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
                if cand in seen:
                    continue
                b = self._read_clamped(cand, BODY_SPAN)
                if len(b) < BODY_SPAN:
                    continue
                if self._u32(b, OFF_TYPE) != TEMPLATE_TYPE:
                    continue
                seen.add(cand)
                bodies.append((cand, self._u32(b, OFF_ID)))
                if len(bodies) > MAX_TEMPLATES:
                    raise RuntimeError("template count exceeds sanity bound (%d)" % MAX_TEMPLATES)
        return bodies

    def read_templates_from_bodies(self, bodies):
        """Decode the per-template records from a pre-located [(body_addr, template_id)] list (the
        WARMED path -- NO heap scan). Re-reads each body LIVE in the SAME order, re-applying the
        span + type==52 + id gate so a stale/freed/realloc'd body is DROPPED (fail-closed, exactly
        as a fresh scan would never have kept it). Output == enumerate_templates."""
        out = []
        for cand, tid in bodies:
            b = self._read_clamped(cand, BODY_SPAN)
            if len(b) < BODY_SPAN:
                continue
            if self._u32(b, OFF_TYPE) != TEMPLATE_TYPE:
                continue
            if self._u32(b, OFF_ID) != tid:            # ABA: same vtable+type, different template
                continue
            out.append(self._decode_template(b))
        return out

    def enumerate_templates(self):
        """Return a list of dicts, one per live division template:
            { id, name, priority, division_cap, n_regiments, n_support, regiments, support }
        """
        return self.read_templates_from_bodies(self.locate_template_bodies())

    def _decode_template(self, b):
        regiments = self._read_def_array(b, OFF_REG_ARR, OFF_REG_LEN)
        support = self._read_def_array(b, OFF_SUP_ARR, OFF_SUP_LEN)
        return {
            "id": self._u32(b, OFF_ID),
            "name": self._read_inline_str_at_body(b, OFF_NAME),
            "priority": self._u32(b, OFF_PRIORITY),
            "division_cap": self._u32(b, OFF_DIV_CAP),
            "n_regiments": len(regiments),
            "n_support": self._u32(b, OFF_SUP_LEN),
            "regiments": regiments,
            "support": support,
        }

    def _read_inline_str_at_body(self, body, off):
        """Read an inline std::string whose header is already in the body snapshot when SSO,
        else fall back to a remote read at the absolute address (we only have the bytes, so
        decode from the body snapshot; for heap-backed strings the body holds the ptr)."""
        # The body snapshot contains the 24-byte header; decode directly.
        size = struct.unpack_from("<Q", body, off + 0x10)[0]
        cap = struct.unpack_from("<Q", body, off + 0x18)[0]
        if size == 0 or size > 200 or cap < size or cap > (1 << 20):
            return None
        if cap < 16:
            return body[off:off + size].decode("latin-1", "replace")
        ptr = struct.unpack_from("<Q", body, off)[0]
        if not self._is_heap(ptr):
            return None
        data = self._read_clamped(ptr, size + 1)
        if len(data) < size + 1 or data[size] != 0:
            return None
        return data[:size].decode("latin-1", "replace")

    def close(self):
        if self.h:
            _k32.CloseHandle(self.h)
            self.h = None


def main():
    import collections
    r = DesignsLiveReader()
    ok, sha = r.verify_build()
    print("hoi4.exe pid=%d module_base=0x%X" % (r.pid, r.module_base))
    print("build sha256: %s" % sha)
    print("build gate: %s" % ("OK (matches pinned build)" if ok else "MISMATCH - refusing"))
    if not ok:
        print("Refusing to enumerate on an unverified build (offsets are build-specific).")
        r.close(); return

    tmpls = r.enumerate_templates()
    print("\ndivision templates: %d" % len(tmpls))
    named = sum(1 for t in tmpls if t["name"])
    print("  named: %d" % named)
    by_role = collections.Counter()
    for t in tmpls:
        by_role[t["n_regiments"]] += 1

    # sample: a few combat templates with regiments
    samp = [t for t in tmpls if t["n_regiments"] >= 3][:6]
    print("\nsample templates:")
    for t in samp:
        rc = collections.Counter(t["regiments"])
        sc = collections.Counter(t["support"])
        print("  id=%d name=%r prio=%d cap=%d" % (t["id"], (t["name"] or "")[:28], t["priority"], t["division_cap"]))
        print("     regiments[%d]: %s" % (t["n_regiments"], dict(rc)))
        print("     support[%d]  : %s" % (t["n_support"], dict(sc)))
    r.close()


if __name__ == "__main__":
    main()
