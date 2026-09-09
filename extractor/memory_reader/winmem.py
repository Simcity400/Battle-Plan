"""winmem.py - stdlib-only, READ-ONLY Windows process memory access.

Core library for the Battle Plan host-side memory reader (Phase 0).
No third-party deps: just ctypes + struct + hashlib.

Design rules (``README.md``, ``Recording``):
  * READ-ONLY. We open with PROCESS_VM_READ | PROCESS_QUERY_INFORMATION and
    NEVER request write rights, never call WriteProcessMemory. The game is
    only ever observed, never touched.
  * 64-bit Python REQUIRED (the game is 64-bit; pointers are 8 bytes).
  * AOB scans use an anchor + bytes.find() so a wildcard pattern over ~50 MB
    resolves in ~30-300 ms instead of multi-second byte-by-byte loops.
  * Build gating: fingerprint() returns the exe SHA-256 so the caller can
    refuse to run on an unknown build rather than risk silent misreads.
"""

import ctypes
import ctypes.wintypes as wt
import hashlib
import struct
import sys

if sys.platform != "win32":
    raise RuntimeError("winmem only runs on Windows")

PYTHON_BITS = struct.calcsize("P") * 8

PROCESS_VM_READ = 0x0010
PROCESS_QUERY_INFORMATION = 0x0400

TH32CS_SNAPPROCESS = 0x00000002
TH32CS_SNAPMODULE = 0x00000008
TH32CS_SNAPMODULE32 = 0x00000010

MAX_PATH = 260
MEM_COMMIT = 0x1000
PAGE_GUARD = 0x100
_SCAN_PAGE = 0x1000

_READABLE = {0x02, 0x04, 0x08, 0x20, 0x40, 0x80}
_EXECUTE = {0x10, 0x20, 0x40, 0x80}

ERROR_BAD_LENGTH = 24
ERROR_PARTIAL_COPY = 299

class PROCESSENTRY32W(ctypes.Structure):
    _fields_ = [
        ("dwSize", wt.DWORD),
        ("cntUsage", wt.DWORD),
        ("th32ProcessID", wt.DWORD),
        ("th32DefaultHeapID", ctypes.c_size_t),
        ("th32ModuleID", wt.DWORD),
        ("cntThreads", wt.DWORD),
        ("th32ParentProcessID", wt.DWORD),
        ("pcPriClassBase", ctypes.c_long),
        ("dwFlags", wt.DWORD),
        ("szExeFile", wt.WCHAR * MAX_PATH),
    ]


class MODULEENTRY32W(ctypes.Structure):
    _fields_ = [
        ("dwSize", wt.DWORD),
        ("th32ModuleID", wt.DWORD),
        ("th32ProcessID", wt.DWORD),
        ("GlblcntUsage", wt.DWORD),
        ("ProccntUsage", wt.DWORD),
        ("modBaseAddr", ctypes.c_void_p),
        ("modBaseSize", wt.DWORD),
        ("hModule", wt.HMODULE),
        ("szModule", wt.WCHAR * 256),
        ("szExePath", wt.WCHAR * MAX_PATH),
    ]


class MEMORY_BASIC_INFORMATION(ctypes.Structure):
    _fields_ = [
        ("BaseAddress", ctypes.c_void_p),
        ("AllocationBase", ctypes.c_void_p),
        ("AllocationProtect", wt.DWORD),
        ("__alignment1", wt.DWORD),
        ("RegionSize", ctypes.c_size_t),
        ("State", wt.DWORD),
        ("Protect", wt.DWORD),
        ("Type", wt.DWORD),
        ("__alignment2", wt.DWORD),
    ]


class MODULEINFO(ctypes.Structure):
    _fields_ = [
        ("lpBaseOfDll", ctypes.c_void_p),
        ("SizeOfImage", wt.DWORD),
        ("EntryPoint", ctypes.c_void_p),
    ]


_k32 = ctypes.WinDLL("kernel32", use_last_error=True)
_psapi = ctypes.WinDLL("psapi", use_last_error=True)

_k32.OpenProcess.argtypes = [wt.DWORD, wt.BOOL, wt.DWORD]
_k32.OpenProcess.restype = wt.HANDLE
_k32.CloseHandle.argtypes = [wt.HANDLE]
_k32.CloseHandle.restype = wt.BOOL
_k32.CreateToolhelp32Snapshot.argtypes = [wt.DWORD, wt.DWORD]
_k32.CreateToolhelp32Snapshot.restype = wt.HANDLE
_k32.Process32FirstW.argtypes = [wt.HANDLE, ctypes.POINTER(PROCESSENTRY32W)]
_k32.Process32FirstW.restype = wt.BOOL
_k32.Process32NextW.argtypes = [wt.HANDLE, ctypes.POINTER(PROCESSENTRY32W)]
_k32.Process32NextW.restype = wt.BOOL
_k32.Module32FirstW.argtypes = [wt.HANDLE, ctypes.POINTER(MODULEENTRY32W)]
_k32.Module32FirstW.restype = wt.BOOL
_k32.Module32NextW.argtypes = [wt.HANDLE, ctypes.POINTER(MODULEENTRY32W)]
_k32.Module32NextW.restype = wt.BOOL
_k32.ReadProcessMemory.argtypes = [
    wt.HANDLE, ctypes.c_void_p, ctypes.c_void_p, ctypes.c_size_t,
    ctypes.POINTER(ctypes.c_size_t),
]
_k32.ReadProcessMemory.restype = wt.BOOL
_k32.VirtualQueryEx.argtypes = [
    wt.HANDLE, ctypes.c_void_p, ctypes.POINTER(MEMORY_BASIC_INFORMATION),
    ctypes.c_size_t,
]
_k32.VirtualQueryEx.restype = ctypes.c_size_t
_psapi.EnumProcessModulesEx.argtypes = [
    wt.HANDLE, ctypes.POINTER(wt.HMODULE), wt.DWORD,
    ctypes.POINTER(wt.DWORD), wt.DWORD,
]
_psapi.EnumProcessModulesEx.restype = wt.BOOL
_psapi.GetModuleFileNameExW.argtypes = [wt.HANDLE, wt.HMODULE, wt.LPWSTR, wt.DWORD]
_psapi.GetModuleFileNameExW.restype = wt.DWORD
_psapi.GetModuleInformation.argtypes = [
    wt.HANDLE, wt.HMODULE, ctypes.POINTER(MODULEINFO), wt.DWORD,
]
_psapi.GetModuleInformation.restype = wt.BOOL

_INVALID_HANDLE = wt.HANDLE(-1).value
_LIST_MODULES_ALL = 0x03


def list_processes():
    """Yield (pid, exe_name) for every running process."""
    snap = _k32.CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)
    if not snap or snap == _INVALID_HANDLE:
        raise OSError("CreateToolhelp32Snapshot failed (err %d)" % ctypes.get_last_error())
    try:
        entry = PROCESSENTRY32W()
        entry.dwSize = ctypes.sizeof(PROCESSENTRY32W)
        ok = _k32.Process32FirstW(snap, ctypes.byref(entry))
        while ok:
            yield int(entry.th32ProcessID), entry.szExeFile
            ok = _k32.Process32NextW(snap, ctypes.byref(entry))
    finally:
        _k32.CloseHandle(snap)


def find_pid(process_name):
    """Return the PID of the first process whose exe name matches (case-insensitive)."""
    target = process_name.lower()
    for pid, name in list_processes():
        if name.lower() == target:
            return pid
    return None


def _iter_modules(pid):
    """Yield MODULEENTRY32W copies for a pid (retries the snapshot while the
    module list is mid-update)."""
    snap = _INVALID_HANDLE
    for _ in range(8):
        snap = _k32.CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid)
        if snap and snap != _INVALID_HANDLE:
            break
        if ctypes.get_last_error() not in (ERROR_BAD_LENGTH, ERROR_PARTIAL_COPY):
            break
    if not snap or snap == _INVALID_HANDLE:
        yield from _iter_modules_psapi(pid)
        return
    try:
        entry = MODULEENTRY32W()
        entry.dwSize = ctypes.sizeof(MODULEENTRY32W)
        ok = _k32.Module32FirstW(snap, ctypes.byref(entry))
        if not ok:
            raise OSError("Module32FirstW failed for pid %d (err %d)"
                          % (pid, ctypes.get_last_error()))
        while ok:
            yield MODULEENTRY32W.from_buffer_copy(entry)
            ok = _k32.Module32NextW(snap, ctypes.byref(entry))
    finally:
        _k32.CloseHandle(snap)


def _iter_modules_psapi(pid):
    """Yield module snapshots via PSAPI when Toolhelp module snapshots are
    blocked. This uses the same read-only/query process rights as Process."""
    handle = _k32.OpenProcess(PROCESS_VM_READ | PROCESS_QUERY_INFORMATION, False, pid)
    if not handle:
        raise OSError("OpenProcess(%d) failed for PSAPI module fallback (err %d)"
                      % (pid, ctypes.get_last_error()))
    try:
        modules = (wt.HMODULE * 4096)()
        needed = wt.DWORD(0)
        ok = _psapi.EnumProcessModulesEx(
            handle, modules, ctypes.sizeof(modules),
            ctypes.byref(needed), _LIST_MODULES_ALL,
        )
        if not ok or needed.value == 0:
            raise OSError("PSAPI module enumeration failed for pid %d (err %d)"
                          % (pid, ctypes.get_last_error()))
        count = min(needed.value // ctypes.sizeof(wt.HMODULE), len(modules))
        for i in range(count):
            hmod = modules[i]
            info = MODULEINFO()
            if not _psapi.GetModuleInformation(
                handle, hmod, ctypes.byref(info), ctypes.sizeof(info)
            ):
                continue
            path_buf = ctypes.create_unicode_buffer(MAX_PATH)
            chars = _psapi.GetModuleFileNameExW(handle, hmod, path_buf, MAX_PATH)
            path = path_buf.value[:chars] if chars else ""
            name = path.rsplit("\\", 1)[-1] if path else ""
            entry = MODULEENTRY32W()
            entry.dwSize = ctypes.sizeof(MODULEENTRY32W)
            entry.th32ProcessID = pid
            entry.modBaseAddr = info.lpBaseOfDll
            entry.modBaseSize = info.SizeOfImage
            entry.hModule = hmod
            entry.szModule = name[:255]
            entry.szExePath = path[:MAX_PATH - 1]
            yield entry
    finally:
        _k32.CloseHandle(handle)


def parse_pattern(pattern):
    """'48 8B 05 ?? ?? ?? ?? 48' -> (bytes, mask[]) ; mask True = fixed byte."""
    pat = bytearray()
    mask = []
    for tok in pattern.replace("0x", "").split():
        if tok in ("??", "?", "**", "*"):
            pat.append(0)
            mask.append(False)
        else:
            pat.append(int(tok, 16))
            mask.append(True)
    if not pat:
        raise ValueError("empty pattern")
    return bytes(pat), mask


def _best_anchor(mask):
    """Longest run of fixed bytes -> (start, length). Used to seed bytes.find()."""
    best_start = best_len = 0
    i, n = 0, len(mask)
    while i < n:
        if mask[i]:
            j = i
            while j < n and mask[j]:
                j += 1
            if j - i > best_len:
                best_start, best_len = i, j - i
            i = j
        else:
            i += 1
    return best_start, best_len


MIN_ANCHOR = 3


def scan_blob(blob, base_addr, pattern, limit=None, allow_short=False):
    """Return absolute addresses in [base_addr ..] where pattern matches blob.

    Stops once `limit` matches are found. Rejects patterns whose longest fixed
    run is < MIN_ANCHOR bytes (would scan far too broadly) unless allow_short.
    """
    pat, mask = parse_pattern(pattern)
    astart, alen = _best_anchor(mask)
    if alen == 0:
        raise ValueError("pattern needs at least one fixed byte to anchor on")
    if alen < MIN_ANCHOR and not allow_short:
        raise ValueError(
            "pattern's longest fixed run is %d byte(s); add more fixed bytes or "
            "pass allow_short=True (risks a very broad/slow scan)" % alen)
    anchor = pat[astart:astart + alen]
    plen = len(pat)
    out = []
    idx = blob.find(anchor)
    while idx != -1:
        start = idx - astart
        if 0 <= start and start + plen <= len(blob):
            for k in range(plen):
                if mask[k] and blob[start + k] != pat[k]:
                    break
            else:
                out.append(base_addr + start)
                if limit is not None and len(out) >= limit:
                    break
        idx = blob.find(anchor, idx + 1)
    return out


class Process(object):
    """Read-only handle to a target process + its main module."""

    def __init__(self, pid, module_name=None):
        if PYTHON_BITS != 64:
            raise RuntimeError(
                "64-bit Python required to read a 64-bit process; this "
                "interpreter is %d-bit." % PYTHON_BITS)
        self.pid = pid
        self.handle = _k32.OpenProcess(
            PROCESS_VM_READ | PROCESS_QUERY_INFORMATION, False, pid
        )
        if not self.handle:
            raise OSError("OpenProcess(%d) failed (err %d). Run as the same user as the game."
                          % (pid, ctypes.get_last_error()))
        self.module_name = None
        self.module_base = 0
        self.module_size = 0
        self.module_path = None
        self._last_read_ok = False
        self._last_read_got = 0
        self._last_read_err = 0
        try:
            self._load_module(module_name)
        except BaseException:
            self.close()
            raise

    def _load_module(self, module_name):
        first = None
        for m in _iter_modules(self.pid):
            if first is None:
                first = m
            if module_name is None or m.szModule.lower() == module_name.lower():
                self.module_name = m.szModule
                self.module_base = int(m.modBaseAddr or 0)
                self.module_size = int(m.modBaseSize)
                self.module_path = m.szExePath
                return
        if module_name is not None and first is not None:
            raise OSError("module %r not found in pid %d" % (module_name, self.pid))
        if first is not None:
            self.module_name = first.szModule
            self.module_base = int(first.modBaseAddr or 0)
            self.module_size = int(first.modBaseSize)
            self.module_path = first.szExePath
        if not self.module_base or not self.module_size:
            raise OSError("no usable module found for pid %d (enumeration empty)" % self.pid)

    def close(self):
        if self.handle:
            _k32.CloseHandle(self.handle)
            self.handle = None

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()

    def try_read(self, address, size):
        """Read up to `size` bytes; return whatever was read (may be short).

        Captures the ReadProcessMemory status (ok / bytes copied / last error)
        IMMEDIATELY (H3) so callers can react to a partial copy and read() can
        report the correct errno instead of a stale one from a later WinAPI call.
        `len(return value)` is the actual byte count, so scanners can advance by
        real bytes read (with their required overlap) and never skip an unread
        tail near a region edge."""
        buf = (ctypes.c_char * size)()
        got = ctypes.c_size_t(0)
        ok = _k32.ReadProcessMemory(self.handle, ctypes.c_void_p(address), buf,
                                    size, ctypes.byref(got))
        self._last_read_ok = bool(ok)
        self._last_read_got = got.value
        self._last_read_err = ctypes.get_last_error()
        if not ok and got.value == 0:
            return b""
        return bytes(buf[:got.value])

    def read(self, address, size):
        """Read exactly `size` bytes or raise (errno from the actual RPM call)."""
        data = self.try_read(address, size)
        if len(data) != size:
            raise OSError("read 0x%X (%d bytes) failed: got %d (err %d)"
                          % (address, size, len(data), self._last_read_err))
        return data

    def _num(self, address, fmt, size):
        return struct.unpack(fmt, self.read(address, size))[0]

    def u8(self, a):  return self._num(a, "<B", 1)
    def i8(self, a):  return self._num(a, "<b", 1)
    def u16(self, a): return self._num(a, "<H", 2)
    def i16(self, a): return self._num(a, "<h", 2)
    def u32(self, a): return self._num(a, "<I", 4)
    def i32(self, a): return self._num(a, "<i", 4)
    def u64(self, a): return self._num(a, "<Q", 8)
    def i64(self, a): return self._num(a, "<q", 8)
    def f32(self, a): return self._num(a, "<f", 4)
    def f64(self, a): return self._num(a, "<d", 8)

    def ptr(self, a):
        """Read an 8-byte pointer."""
        return self.u64(a)

    def cstr(self, address, maxlen=64, encoding="latin-1"):
        raw = self.try_read(address, maxlen)
        nul = raw.find(b"\x00")
        if nul != -1:
            raw = raw[:nul]
        return raw.decode(encoding, "replace")

    def tag(self, address):
        """Read a HOI4-style country tag (3 ascii chars)."""
        return self.cstr(address, 4).strip()

    def chain(self, base, offsets, deref_base=True):
        """Resolve a CE-style pointer chain.

        Starting at `base`, for each offset: addr = deref(addr) + offset.
        With deref_base=False the first deref is skipped (base is already an
        object address). Returns the final ADDRESS (read the value there
        yourself, or treat it as the object base).

        e.g. country = chain(pPlayer, [0x408, 0x90])
             == read64(read64(pPlayer)+0x408) + 0x90
        """
        addr = base
        for i, off in enumerate(offsets):
            if i == 0 and not deref_base:
                addr = addr + off
            else:
                addr = self.u64(addr) + off
        return addr

    def iter_regions(self, min_addr, max_addr, want_exec=None):
        """Yield (base, size) for committed, readable regions in range.
        want_exec True/False filters to executable / non-executable pages."""
        mbi = MEMORY_BASIC_INFORMATION()
        addr = min_addr
        while addr < max_addr:
            if _k32.VirtualQueryEx(self.handle, ctypes.c_void_p(addr),
                                   ctypes.byref(mbi), ctypes.sizeof(mbi)) == 0:
                break
            rbase = int(mbi.BaseAddress or 0)
            rsize = int(mbi.RegionSize)
            nxt = rbase + rsize
            prot = mbi.Protect
            low = prot & 0xFF
            if (mbi.State == MEM_COMMIT and low in _READABLE
                    and not (prot & PAGE_GUARD)):
                is_exec = low in _EXECUTE
                if want_exec is None or is_exec == want_exec:
                    start = max(rbase, min_addr)
                    stop = min(rbase + rsize, max_addr)
                    if stop > start:
                        yield start, stop - start
            if nxt <= addr:
                break
            addr = nxt

    def scan_module(self, pattern, want_exec=True, limit=None, allow_short=False):
        """AOB-scan the main module (code pages by default).

        Reads each region in overlapping windows so a match straddling a window
        boundary isn't missed, bounds memory use, and stops early once `limit`
        matches are found."""
        pat, _ = parse_pattern(pattern)
        plen = len(pat)
        chunk = 8 << 20
        overlap = plen - 1
        if overlap >= chunk:
            chunk = overlap + _SCAN_PAGE
        out = []
        end = self.module_base + self.module_size
        for rbase, rsize in self.iter_regions(self.module_base, end, want_exec=want_exec):
            pos = 0
            while pos < rsize:
                want = min(chunk, rsize - pos)
                blob = self.try_read(rbase + pos, want)
                if blob:
                    remaining = None if limit is None else max(0, limit - len(out))
                    out.extend(scan_blob(blob, rbase + pos, pattern,
                                         limit=remaining, allow_short=allow_short))
                    if limit and len(out) >= limit:
                        return out[:limit]
                if blob:
                    blen = len(blob)
                    if pos + blen >= rsize or blen < want:
                        pos += max(blen, _SCAN_PAGE)
                    else:
                        pos += blen - overlap
                else:
                    pos += max(want, _SCAN_PAGE)
        return out

    def rip_target(self, instr_addr, disp_offset, instr_len):
        """Decode a RIP-relative load: target = instr_addr + instr_len + disp32,
        where disp32 lives at instr_addr + disp_offset. Recovers the runtime
        address of a global without hardcoding it."""
        disp = self.i32(instr_addr + disp_offset)
        return instr_addr + instr_len + disp

    def in_module(self, address):
        return self.module_base <= address < self.module_base + self.module_size

    def fingerprint(self):
        """Return a dict identifying the exact build, for hash-gating."""
        info = {
            "pid": self.pid,
            "module": self.module_name,
            "base": self.module_base,
            "size": self.module_size,
            "path": self.module_path,
            "python_bits": PYTHON_BITS,
            "sha256": None,
        }
        if self.module_path:
            try:
                info["sha256"] = sha256_file(self.module_path)
            except OSError as e:
                info["sha256_error"] = str(e)
        return info


def sha256_file(path, chunk=1 << 20):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(chunk), b""):
            h.update(block)
    return h.hexdigest()


def open_process(process_name="hoi4.exe", module_name=None):
    """Convenience: find `process_name` and return an open Process, or None.
    module_name defaults to process_name (the main module)."""
    pid = find_pid(process_name)
    if pid is None:
        return None
    return Process(pid, module_name or process_name)
