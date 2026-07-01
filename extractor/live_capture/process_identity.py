"""process_identity.py -- pin a live hoi4.exe to a stable, verifiable identity.

The continuity tagger (continuity.py) and the build gate both need to know
*which* process a snapshot came from. A bare PID is not enough: Windows recycles
PIDs, and several memory readers each open their own handle, so we must be able
to confirm every reader attached to the SAME process instance.

A ``ProcessIdentity`` carries:

  * ``pid``                  -- the live PID
  * ``create_time_100ns``    -- the process creation FILETIME (PID + creation
                                time is a stable per-instance key that survives
                                PID reuse; from GetProcessTimes)
  * ``exe_sha256``           -- the on-disk exe hash (the build gate input)
  * ``module_base``          -- ASLR base (informational; NOT an identity key)
  * ``instance_id``          -- sha256(exe_sha256 + pid + create_time)[:16], the
                                stable "this process instance" key

Per Codex review: a campaign_id MUST NOT be derived from wallclock (a restart
mid-game would invent a new campaign for the same game). The ``instance_id`` is
the honest memory-only fallback grouping key -- stable for the life of one
hoi4.exe process, and it changes iff the process changes (which is the only thing
we can actually prove without a save's game_unique_seed).

READ-ONLY. stdlib (ctypes) only. No writes, no injection.
"""
from __future__ import annotations

import ctypes
import ctypes.wintypes as wt
import hashlib
import sys
from dataclasses import dataclass
from typing import Optional


# The build this whole memory_reader family was mapped on. All readers gate on
# this same constant; we mirror it here so the capture loop can refuse BEFORE it
# attaches any reader (the readers also each re-check, defense in depth).
PINNED_EXE_SHA256 = (
    "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120"
)


# --------------------------------------------------------------------------
# Win32 (read-only): GetProcessTimes for the per-instance creation timestamp.
# --------------------------------------------------------------------------
if sys.platform == "win32":  # pragma: no cover - exercised only on Windows
    _PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
    _PROCESS_QUERY_INFORMATION = 0x0400

    class _FILETIME(ctypes.Structure):
        _fields_ = [("dwLowDateTime", wt.DWORD), ("dwHighDateTime", wt.DWORD)]

    _k32 = ctypes.WinDLL("kernel32", use_last_error=True)
    _k32.OpenProcess.argtypes = [wt.DWORD, wt.BOOL, wt.DWORD]
    _k32.OpenProcess.restype = wt.HANDLE
    _k32.CloseHandle.argtypes = [wt.HANDLE]
    _k32.CloseHandle.restype = wt.BOOL
    _k32.GetProcessTimes.argtypes = [
        wt.HANDLE, ctypes.POINTER(_FILETIME), ctypes.POINTER(_FILETIME),
        ctypes.POINTER(_FILETIME), ctypes.POINTER(_FILETIME),
    ]
    _k32.GetProcessTimes.restype = wt.BOOL


def _process_create_time_100ns(pid: int) -> Optional[int]:
    """Return the process creation time as a 64-bit FILETIME (100ns ticks since
    1601), or None if it can't be read. PID + this value uniquely identifies a
    process instance across PID reuse."""
    if sys.platform != "win32":  # pragma: no cover
        return None
    # QUERY_LIMITED_INFORMATION needs no VM rights and works for same-user procs.
    handle = _k32.OpenProcess(_PROCESS_QUERY_LIMITED_INFORMATION, False, pid)
    if not handle:
        handle = _k32.OpenProcess(_PROCESS_QUERY_INFORMATION, False, pid)
    if not handle:
        return None
    try:
        creation = _FILETIME()
        exit_t = _FILETIME()
        kernel = _FILETIME()
        user = _FILETIME()
        ok = _k32.GetProcessTimes(handle, ctypes.byref(creation),
                                  ctypes.byref(exit_t), ctypes.byref(kernel),
                                  ctypes.byref(user))
        if not ok:
            return None
        return (creation.dwHighDateTime << 32) | creation.dwLowDateTime
    finally:
        _k32.CloseHandle(handle)


def _sha256_file(path: str, chunk: int = 1 << 20) -> Optional[str]:
    try:
        h = hashlib.sha256()
        with open(path, "rb") as fh:
            for block in iter(lambda: fh.read(chunk), b""):
                h.update(block)
        return h.hexdigest()
    except OSError:
        return None


@dataclass
class ProcessIdentity:
    """Stable, verifiable identity of one live hoi4.exe instance."""

    pid: int
    create_time_100ns: Optional[int]
    exe_sha256: Optional[str]
    exe_path: Optional[str]
    module_base: Optional[int]

    @property
    def instance_id(self) -> str:
        """sha256(exe_sha + pid + create_time)[:16] -- the memory-only stable
        grouping key for one process instance (Codex C). Falls back to whatever
        identity bits are available, so it is always a string."""
        parts = [
            self.exe_sha256 or "no-sha",
            str(self.pid),
            str(self.create_time_100ns if self.create_time_100ns is not None
                else "no-ctime"),
        ]
        return hashlib.sha256("|".join(parts).encode("utf-8")).hexdigest()[:16]

    def build_ok(self) -> bool:
        """True iff the live exe hash matches the pinned build (fail-closed: an
        unknown/unreadable hash is NOT ok)."""
        return self.exe_sha256 == PINNED_EXE_SHA256

    def same_instance(self, other: Optional["ProcessIdentity"]) -> bool:
        """Two identities are the same process instance iff pid AND creation time
        match (creation time defeats PID reuse). If creation time is unavailable
        on either side we fall back to pid-only (best effort, flagged by caller)."""
        if other is None:
            return False
        if self.create_time_100ns is not None and other.create_time_100ns is not None:
            return (self.pid == other.pid
                    and self.create_time_100ns == other.create_time_100ns)
        return self.pid == other.pid

    def to_dict(self) -> dict:
        return {
            "pid": self.pid,
            "create_time_100ns": self.create_time_100ns,
            "exe_sha256": self.exe_sha256,
            "instance_id": self.instance_id,
            "build_ok": self.build_ok(),
            # module_base is ASLR-randomized; informational only, never an id key.
            "module_base": self.module_base,
        }


def identify(proc) -> ProcessIdentity:
    """Build a ProcessIdentity from an open winmem.Process-like handle.

    Accepts anything exposing ``pid``, ``module_path``/``exe_path`` and
    ``module_base`` (winmem.Process and the readers' own handle classes both do).
    The exe hash is read from the loaded module path (the readers verify the same
    way), so this never trusts a hardcoded path.
    """
    pid = int(getattr(proc, "pid"))
    exe_path = (getattr(proc, "module_path", None)
                or getattr(proc, "exe_path", None))
    module_base = getattr(proc, "module_base", None)
    # Prefer the process's own fingerprint() if it has one (winmem.Process).
    exe_sha = None
    fp = getattr(proc, "fingerprint", None)
    if callable(fp):
        try:
            exe_sha = fp().get("sha256")
        except Exception:
            exe_sha = None
    if exe_sha is None and exe_path:
        exe_sha = _sha256_file(exe_path)
    return ProcessIdentity(
        pid=pid,
        create_time_100ns=_process_create_time_100ns(pid),
        exe_sha256=exe_sha,
        exe_path=exe_path,
        module_base=int(module_base) if module_base else None,
    )


__all__ = ["ProcessIdentity", "identify", "PINNED_EXE_SHA256"]
