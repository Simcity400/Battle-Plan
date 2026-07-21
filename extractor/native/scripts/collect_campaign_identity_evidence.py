#!/usr/bin/env python3
"""Read-only CID1 campaign-identity evidence collector. Never writes game memory,
never changes game state, and NEVER emits a verdict.

This is the owner-runnable identity probe that closes the "no owner-runnable
identity probe exists" gap blocking every CID1 (pillar 3) live-acceptance arm.
It captures the two RE-proven CGameState identity fields
(game_unique_seed i32 @ +0x720, game_unique_id MSVC std::string @ +0x728) as
nonce-bound, bracketed, tear-proof observations, appends each to an
append-only transcript, and seals the transcript under a caller-issued evidence
nonce. The independent verifier -- not this collector -- is the only authority
that emits a pass/fail verdict.

The layout/read logic is a faithful Python port of the MERGED collector readers
(extractor/native/collector/src/lib.rs): read_identity_observation /
read_identity_uuid (~L47919-47991) and read_stable_identity_observations
(~L47993-48017), with constants from compact_contract::build_profile
(CGAMESTATE_CURRENT_HOUR 0x468, DOMAIN_CAMPAIGN_ID_GAME_UNIQUE_SEED_OFF 0x720,
DOMAIN_CAMPAIGN_ID_GAME_UNIQUE_ID_OFF 0x728, HEADER_REGION_LEN 0x28,
MAX_STRING_CAPACITY 1<<20) and campaign_identity::CID1_UUID_LEN 36. The
CGameState root and clock sentinel are resolved exactly like the D57 speed probe
(collect_speed_root_evidence_v2.py): gs = u64[base+0x3316E90]; sentinel-validate
u64[clock_a]==base+0x2952510 with clock_a==clock_b at base+0x33172E0/+0x33172E8.

Only the pinned current build is supported:
  exe SHA-256 870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad
Any other executable is refused.

============================ USAGE (owner CLI) ============================
The static-disasm artifact is strict JSON with EXACTLY these keys (see the
verifier's validate_static); create it once for the pinned build:
  {
    "schema": "battle-plan.campaign_identity_static_disasm.v1",
    "exe_sha256": "870ebf1f...c65ad",
    "module_rvas": {
      "cgame_state_singleton_rva": "0x3316e90",
      "clock_singleton_a_rva": "0x33172e0",
      "clock_singleton_b_rva": "0x33172e8",
      "clock_vtable_rva": "0x2952510"
    },
    "field_offsets": {
      "current_hour_off": "0x468",
      "game_unique_seed_off": "0x720",
      "game_unique_id_off": "0x728",
      "header_region_len": "0x28"
    },
    "string_layout": {
      "string_ptr_offset": "0x0",
      "string_size_offset": "0x10",
      "string_capacity_offset": "0x18",
      "sso_heap_threshold": "0x10"
    },
    "identity_grammar": {
      "uuid_len": 36,
      "max_string_capacity": 1048576,
      "hyphen_positions": [8, 13, 18, 23],
      "hex_char_class": "0123456789abcdef"
    },
    "evidence_export_sha256": "<64-hex disassembler evidence-export sha256>"
  }

1) Initialize a fresh transcript under a caller-issued 64-hex nonce:
   python collect_campaign_identity_evidence.py init \
     --pid <the game PID> --session-id <id> \
     --evidence-nonce <fresh 64-hex> \
     --nonce-ledger-root <absolute durable dir> \
     --static-disasm <absolute static artifact json> \
     --out <absolute transcript.json> \
     --action-token AUTHORIZE_READ_ONLY_CAMPAIGN_IDENTITY_EVIDENCE_INIT

2) For EACH labeled observation (repeat N times; e.g. L0/S/D/M matrix arms),
   have the owner/orchestrator put the game in the contract-selected state, then
   run the default automatic same-hour barrier:
   python collect_campaign_identity_evidence.py observe \
     --transcript <transcript.json> \
     --label L0_paused_first_frame \
     --barrier-mode automatic_same_hour \
     --action-token OBSERVE_CAMPAIGN_IDENTITY_L0_paused_first_frame_AUTOMATIC_SAME_HOUR_V1
   The tool takes two independently coherent observations without accepting
   stdin. Both observations must remain in the same game hour and be
   byte-identical on the identity fields. The action nonce, caller-issued
   evidence nonce, exact process binding, and sealed transcript authenticate the
   resulting automatic barrier. The legacy manual_ack mode remains available
   only for reproducing historical receipts.

3) Seal (no verdict):
   python collect_campaign_identity_evidence.py seal \
     --transcript <transcript.json> \
     --action-token SEAL_CAMPAIGN_IDENTITY_TRANSCRIPT_WITHOUT_VERDICT

4) Run verify_campaign_identity_evidence.py for the ONLY verdict. Optionally pin
   --expected-seed/--expected-id there against the melted-save (L0 melt) values.

Different game launches = different transcripts (process binding pins one PID).
=========================================================================
"""

from __future__ import annotations

import argparse
import ctypes
from ctypes import wintypes
import hashlib
import json
import os
import re
import secrets
import struct
import sys
import tempfile
import time
from pathlib import Path

SCHEMA = "battle-plan.campaign_identity_evidence_transcript.v1"
NONCE_ISSUE_SCHEMA = "battle-plan.campaign_identity_nonce_issue.v1"
NONCE_SEAL_SCHEMA = "battle-plan.campaign_identity_nonce_seal.v1"
EXE_SHA = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad".lower()

# CGameState root + clock sentinel (identical to the D57 speed probe).
CGAMESTATE_SINGLETON_RVA = 0x3316E90
CLOCK_SINGLETON_A_RVA = 0x33172E0
CLOCK_SINGLETON_B_RVA = 0x33172E8
CLOCK_VTABLE_RVA = 0x2952510
# CID1 identity field offsets (compact_contract::build_profile).
CURRENT_HOUR_OFF = 0x468
GAME_UNIQUE_SEED_OFF = 0x720
GAME_UNIQUE_ID_OFF = 0x728
HEADER_REGION_LEN = (GAME_UNIQUE_ID_OFF - GAME_UNIQUE_SEED_OFF) + 0x20  # 0x28
STRING_OFF = GAME_UNIQUE_ID_OFF - GAME_UNIQUE_SEED_OFF  # 0x8 within the snapshot
UUID_LEN = 36  # campaign_identity::CID1_UUID_LEN
MAX_STRING_CAPACITY = 1 << 20  # DOMAIN_CAMPAIGN_ID_MAX_STRING_CAPACITY
# Canonical UUID grammar the probe enforces (read_identity_uuid: lowercase-hex
# with hyphens at 8/13/18/23). Pinned by the static artifact so the exact grammar
# the probe enforces is bound by the artifact hash (finding round-2 #3).
UUID_HYPHEN_POSITIONS = (8, 13, 18, 23)
UUID_HEX_CHAR_CLASS = "0123456789abcdef"
# MSVC std::string control-block layout (relative to the string object base).
# These are the offsets the decode path actually uses; the pinned static artifact
# binds them so a different control-block layout cannot hash-pass (finding 3).
STRING_PTR_OFF = 0x0
STRING_SIZE_OFF = 0x10
STRING_CAP_OFF = 0x18
SSO_HEAP_THRESHOLD = 16  # cap < 16 -> inline (SSO); cap >= 16 -> heap

# Transcript-level root constants (pinned, echoed by the verifier).
ROOTS = {
    "cgame_state_singleton_rva": "0x3316e90",
    "clock_singleton_a_rva": "0x33172e0",
    "clock_singleton_b_rva": "0x33172e8",
    "clock_vtable_rva": "0x2952510",
    "current_hour_off": "0x468",
    "game_unique_seed_off": "0x720",
    "game_unique_id_off": "0x728",
    "header_region_len": "0x28",
    "string_ptr_offset": "0x0",
    "string_size_offset": "0x10",
    "string_capacity_offset": "0x18",
    "sso_heap_threshold": "0x10",
    "uuid_hyphen_positions": "8,13,18,23",
    "uuid_hex_char_class": "0123456789abcdef",
}

# Static-disasm artifact structure (also independently validated by the verifier).
STATIC_SCHEMA = "battle-plan.campaign_identity_static_disasm.v1"
STATIC_MODULE_KEYS = ("cgame_state_singleton_rva", "clock_singleton_a_rva", "clock_singleton_b_rva", "clock_vtable_rva")
STATIC_OFFSET_KEYS = ("current_hour_off", "game_unique_seed_off", "game_unique_id_off", "header_region_len")
STATIC_STRING_LAYOUT_KEYS = ("string_ptr_offset", "string_size_offset", "string_capacity_offset", "sso_heap_threshold")
STATIC_GRAMMAR = {
    "uuid_len": UUID_LEN,
    "max_string_capacity": MAX_STRING_CAPACITY,
    "hyphen_positions": list(UUID_HYPHEN_POSITIONS),
    "hex_char_class": UUID_HEX_CHAR_CLASS,
}

HEX64 = re.compile(r"^[0-9a-f]{64}$")
LABEL = re.compile(r"^[A-Za-z0-9_]{2,64}$")
PROCESS_QUERY_INFORMATION = 0x0400
PROCESS_VM_READ = 0x0010
LIST_MODULES_ALL = 0x03
MAX_COHERENT_ATTEMPTS_CEILING = 16


class ProbeError(RuntimeError):
    pass


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        while block := handle.read(1024 * 1024):
            digest.update(block)
    return digest.hexdigest()


def canonical_bytes(value) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False, allow_nan=False).encode("utf-8")


def transcript_digest(value: dict) -> str:
    clone = dict(value)
    clone.pop("transcript_sha256", None)
    return hashlib.sha256(canonical_bytes(clone)).hexdigest()


_LAST_FILETIME = 0


def now_filetime() -> int:
    # Windows FILETIME (100ns ticks since 1601). Guaranteed strictly increasing
    # within one process so two reads taken inside the same coarse clock tick
    # still get distinct, ordered stamps (the verifier requires strict order).
    global _LAST_FILETIME
    value = 116444736000000000 + time.time_ns() // 100
    if value <= _LAST_FILETIME:
        value = _LAST_FILETIME + 1
    _LAST_FILETIME = value
    return value


def file_identity(path: Path) -> dict:
    stat = path.stat()
    return {"volume_serial_number": int(stat.st_dev), "file_index": int(stat.st_ino), "file_size": int(stat.st_size)}


class FILETIME(ctypes.Structure):
    _fields_ = [("low", wintypes.DWORD), ("high", wintypes.DWORD)]


def filetime_int(value: FILETIME) -> int:
    return (int(value.high) << 32) | int(value.low)


# --- MSVC std::string decode (faithful port of read_identity_uuid's extraction) ---

def _u64_from(region: bytes, offset: int) -> int:
    end = offset + 8
    if end > len(region) or offset < 0:
        raise ProbeError(f"std::string field at {offset} is outside the snapshot region")
    return struct.unpack_from("<Q", region, offset)[0]


def _i32_from(region: bytes, offset: int) -> int:
    end = offset + 4
    if end > len(region) or offset < 0:
        raise ProbeError(f"i32 field at {offset} is outside the snapshot region")
    return struct.unpack_from("<i", region, offset)[0]


def decode_msvc_std_string(region: bytes, string_off: int, read_chars, max_capacity: int, required_size: int) -> tuple[bytes, int, int, str]:
    """Decode one fixed-length MSVC std::string from a coherent control-block
    snapshot. `required_size` is MANDATORY (no default): the exact byte length the
    caller expects (CID1 UUID = 36).

    Faithful to lib.rs read_identity_uuid (order-exact): size @ string_off+0x10,
    capacity @ string_off+0x18. The exact-size gate fires FIRST -- before the
    capacity word is consulted or any pointer is followed -- exactly as the Rust
    authority checks `size != CID1_UUID_LEN` ahead of the heap read
    (collector/src/lib.rs:47961). Because required_size is mandatory and the gate
    is unconditional, NO call path can issue a read wider than required_size+1: a
    garbage control block (e.g. size=capacity=1MiB) is refused without ever
    dereferencing the heap pointer. SSO (inline chars in the control block) when
    cap < SSO_HEAP_THRESHOLD, else the heap pointer @ string_off is followed with
    ONE guarded read of exactly size+1 bytes. Returns
    (raw_bytes_without_nul, size, capacity, storage). Any size/bound/capacity or
    NUL-terminator violation is a torn read -> ProbeError (fail-closed).
    read_chars(addr, length) must return exactly `length` bytes or raise."""
    size = _u64_from(region, string_off + STRING_SIZE_OFF)
    # Mandatory exact-size gate BEFORE the capacity word or any pointer follow.
    if size != required_size:
        raise ProbeError("std::string size is not the required exact length; refusing before any dereference")
    cap = _u64_from(region, string_off + STRING_CAP_OFF)
    if cap < size:
        raise ProbeError("std::string capacity < size (torn read); refusing")
    if cap > max_capacity:
        raise ProbeError("std::string capacity exceeds fail-closed ceiling (torn read); refusing")
    if cap < SSO_HEAP_THRESHOLD:
        end = string_off + size + 1
        if end > len(region):
            raise ProbeError("inline std::string exceeds the snapshot region (torn read); refusing")
        storage = "inline"
        chunk = region[string_off:end]
    else:
        chars_addr = _u64_from(region, string_off + STRING_PTR_OFF)
        storage = "heap"
        chunk = read_chars(chars_addr, size + 1)
    if len(chunk) != size + 1 or chunk[size] != 0:
        raise ProbeError("std::string is not NUL-terminated at its length (torn read); refusing")
    return bytes(chunk[:size]), size, cap, storage


def validate_uuid_grammar(raw: bytes) -> str:
    """Canonical lowercase-hex/hyphen UUID grammar (as in read_identity_uuid),
    driven by the constants the static artifact pins (UUID_HYPHEN_POSITIONS and
    UUID_HEX_CHAR_CLASS). Rejects anything that is not a canonical UUID ->
    ProbeError."""
    if len(raw) != UUID_LEN:
        raise ProbeError("game_unique_id is not the canonical UUID length; refusing")
    hex_bytes = frozenset(UUID_HEX_CHAR_CLASS.encode("ascii"))
    for index, byte in enumerate(raw):
        hyphen = index in UUID_HYPHEN_POSITIONS
        ok = (hyphen and byte == 0x2D) or (not hyphen and byte in hex_bytes)
        if not ok:
            raise ProbeError("game_unique_id violates the canonical UUID grammar; refusing")
    return raw.decode("ascii")


class ReadOnlyProcess:
    """OpenProcess(PROCESS_QUERY_INFORMATION|PROCESS_VM_READ) + ReadProcessMemory
    only. No write access is ever requested; WriteProcessMemory is never called."""

    def __init__(self, pid: int):
        if os.name != "nt":
            raise ProbeError("live collection is Windows-only")
        self.k32 = ctypes.WinDLL("kernel32", use_last_error=True)
        self.psapi = ctypes.WinDLL("psapi", use_last_error=True)
        self.k32.OpenProcess.restype = wintypes.HANDLE
        self.k32.ReadProcessMemory.argtypes = [wintypes.HANDLE, ctypes.c_void_p, ctypes.c_void_p, ctypes.c_size_t, ctypes.POINTER(ctypes.c_size_t)]
        self.k32.ReadProcessMemory.restype = wintypes.BOOL
        self.handle = self.k32.OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, False, pid)
        if not self.handle:
            raise ProbeError(f"OpenProcess read-only failed: {ctypes.get_last_error()}")
        self.pid = pid

    def close(self):
        if self.handle:
            self.k32.CloseHandle(self.handle)
            self.handle = None

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()

    def exe_path(self) -> Path:
        size = wintypes.DWORD(32768)
        buffer = ctypes.create_unicode_buffer(size.value)
        if not self.k32.QueryFullProcessImageNameW(self.handle, 0, buffer, ctypes.byref(size)):
            raise ProbeError("QueryFullProcessImageNameW failed")
        return Path(buffer.value).resolve()

    def creation_filetime(self) -> int:
        created = FILETIME()
        exited = FILETIME()
        kernel = FILETIME()
        user = FILETIME()
        if not self.k32.GetProcessTimes(self.handle, ctypes.byref(created), ctypes.byref(exited), ctypes.byref(kernel), ctypes.byref(user)):
            raise ProbeError("GetProcessTimes failed")
        return filetime_int(created)

    def module_base(self) -> int:
        modules = (wintypes.HMODULE * 1)()
        needed = wintypes.DWORD()
        if not self.psapi.EnumProcessModulesEx(self.handle, modules, ctypes.sizeof(modules), ctypes.byref(needed), LIST_MODULES_ALL):
            raise ProbeError("EnumProcessModulesEx failed")
        return int(ctypes.cast(modules[0], ctypes.c_void_p).value)

    def read(self, address: int, size: int) -> bytes:
        buffer = (ctypes.c_ubyte * size)()
        read = ctypes.c_size_t()
        if not self.k32.ReadProcessMemory(self.handle, ctypes.c_void_p(address), buffer, size, ctypes.byref(read)) or read.value != size:
            raise ProbeError(f"ReadProcessMemory failed at 0x{address:x}")
        return bytes(buffer)

    def u64(self, address: int) -> int:
        return struct.unpack("<Q", self.read(address, 8))[0]


def read_i32_stable(process, address: int) -> tuple[int, bytes]:
    """Two-consecutive-equal i32 read, byte-faithful to lib.rs read_i32_stable
    (L55442): read twice; if the two reads disagree the value is torn and the
    read is REFUSED (the Rust authority returns None -> identity-unavailable). No
    inner retry -- transient instability is handled by the bounded outer coherence
    loop, matching the authority's two-stable-observation structure. Returns the
    value and the confirmed raw bytes."""
    first = process.read(address, 4)
    second = process.read(address, 4)
    if first != second:
        raise ProbeError("i32 stable read disagreed across two reads (torn); refusing")
    return struct.unpack("<i", first)[0], first


def _read_identity_once(process, base: int) -> dict:
    """One raw identity observation, order-exact to read_identity_observation
    (lib.rs:47924): resolve + sentinel-validate the CGameState root/clock, then the
    game hour FIRST (via read_i32_stable -- two equal reads), THEN the coherent
    +0x720 header region, then the heap UUID chars, then decode with the exact-size
    gate ahead of any dereference. Raises on any sentinel/decode failure
    (fail-closed)."""
    gs_slot = base + CGAMESTATE_SINGLETON_RVA
    clock_a_slot = base + CLOCK_SINGLETON_A_RVA
    clock_b_slot = base + CLOCK_SINGLETON_B_RVA
    expected_vtable = base + CLOCK_VTABLE_RVA
    gs = process.u64(gs_slot)
    clock_a = process.u64(clock_a_slot)
    clock_b = process.u64(clock_b_slot)
    if gs < 0x10000 or clock_a < 0x10000 or clock_a != clock_b or process.u64(clock_a) != expected_vtable:
        raise ProbeError("root/sentinel validation failed")
    # AUTHORITY ORDER: hour (stable) BEFORE the header region.
    hour, hour_bytes = read_i32_stable(process, gs + CURRENT_HOUR_OFF)
    region = process.read(gs + GAME_UNIQUE_SEED_OFF, HEADER_REGION_LEN)
    if len(region) != HEADER_REGION_LEN:
        raise ProbeError("coherent header region under-read; refusing")
    seed = _i32_from(region, 0)
    chars_bytes = b""

    def read_chars(addr: int, length: int) -> bytes:
        nonlocal chars_bytes
        chars_bytes = process.read(addr, length)
        return chars_bytes

    # required_size=UUID_LEN rejects a non-36 size BEFORE the heap dereference.
    raw, size, cap, storage = decode_msvc_std_string(region, STRING_OFF, read_chars, MAX_STRING_CAPACITY, required_size=UUID_LEN)
    uuid = validate_uuid_grammar(raw)
    return {
        "gs": gs,
        "clock_a": clock_a,
        "clock_b": clock_b,
        "vtable": expected_vtable,
        "hour": hour,
        "seed": seed,
        "uuid": uuid,
        "size": size,
        "cap": cap,
        "storage": storage,
        "region_sha": hashlib.sha256(region).hexdigest(),
        "raw": region + chars_bytes + hour_bytes,
    }


def read_coherent_identity(process, base: int, max_attempts: int) -> dict:
    """Coherent read: re-read the identity snapshot until two CONSECUTIVE reads are
    byte-identical (region + heap chars + hour), with bounded retries; then hard
    fail. A torn/unstable read is REFUSED, never retried into plausibility. This is
    the read_stable_identity_observations pattern extended with a retry bound."""
    if max_attempts < 2:
        raise ProbeError("coherent read needs at least two attempts")
    prev = None
    last_error = None
    for count in range(1, max_attempts + 1):
        try:
            cur = _read_identity_once(process, base)
        except ProbeError as exc:
            last_error = exc
            prev = None
            continue
        if prev is not None and prev["raw"] == cur["raw"]:
            cur["coherent_attempts"] = count
            return cur
        prev = cur
    detail = f"; last error: {last_error}" if last_error is not None else ""
    raise ProbeError(f"identity snapshot never stabilized across {max_attempts} reads (torn/unstable); refusing to fabricate{detail}")


def read_record(snapshot: dict, label: str) -> dict:
    return {
        "read_label": label,
        "observed_filetime_utc": now_filetime(),
        "coherent_attempts": snapshot["coherent_attempts"],
        "cgame_state_value": f"0x{snapshot['gs']:x}",
        "clock_slot_a_value": f"0x{snapshot['clock_a']:x}",
        "clock_slot_b_value": f"0x{snapshot['clock_b']:x}",
        "clock_vtable_value": f"0x{snapshot['vtable']:x}",
        "game_hour_raw": snapshot["hour"],
        "game_unique_seed": snapshot["seed"],
        "game_unique_id": snapshot["uuid"],
        "string_storage": snapshot["storage"],
        "string_size": snapshot["size"],
        "string_capacity": snapshot["cap"],
        "header_region_sha256": snapshot["region_sha"],
    }


def identity_fields_equal(a: dict, b: dict) -> bool:
    keys = ("game_unique_seed", "game_unique_id", "string_size", "string_capacity", "string_storage", "header_region_sha256")
    return all(a[key] == b[key] for key in keys)


def _operator_ack(prompt: str) -> None:
    """Operator-paced acknowledgement between the bracketing reads. No stopwatch:
    it blocks until the operator presses Enter. A closed/redirected stdin (EOF)
    is NOT a silent auto-advance -- it fails closed so read B never fires without
    a real acknowledgement, and the surrounding command burns the nonce
    (finding 4). Tests monkeypatch this."""
    try:
        input(prompt)
    except EOFError as exc:
        raise ProbeError("bracket acknowledgement stdin is closed (EOF); refusing to auto-advance to read B") from exc


# --- Durable nonce ledger (ported from the D57 speed probe) --------------------

def atomic_json(path: Path, value) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temp = path.with_name(path.name + f".{secrets.token_hex(8)}.tmp")
    data = (json.dumps(value, indent=2, sort_keys=True) + "\n").encode()
    with temp.open("xb") as handle:
        handle.write(data)
        handle.flush()
        os.fsync(handle.fileno())
    os.replace(temp, path)


def exclusive_json(path: Path, value) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temp = path.with_name(path.name + f".{secrets.token_hex(8)}.tmp")
    data = (json.dumps(value, indent=2, sort_keys=True) + "\n").encode()
    try:
        with temp.open("xb") as handle:
            handle.write(data)
            handle.flush()
            os.fsync(handle.fileno())
        os.link(temp, path)
    finally:
        try:
            temp.unlink()
        except FileNotFoundError:
            pass


def issued_path(root: Path, nonce: str) -> Path:
    return root / f"{nonce}.issued.json"


def consume_nonce(root: Path, nonce: str, state: str, session_id: str, transcript_path: Path, transcript_sha256: str, created: int, consumed: int | None = None) -> None:
    source = issued_path(root, nonce)
    marker = source.with_suffix(source.suffix + ".consumed")
    destination = root / f"{nonce}.{state}.json"
    value = {
        "schema": NONCE_SEAL_SCHEMA,
        "state": state,
        "session_id": session_id,
        "evidence_nonce_hex": nonce,
        "transcript_path": str(transcript_path.resolve()),
        "transcript_sha256": transcript_sha256,
        "created_filetime_utc": created,
        "consumed_filetime_utc": now_filetime() if consumed is None else consumed,
    }
    conflicting = [root / f"{nonce}.{candidate}.json" for candidate in ("consumed_success", "consumed_failure") if candidate != state]
    if any(path.exists() for path in conflicting):
        raise ProbeError("nonce already has a conflicting terminal outcome")
    if destination.exists():
        if json.loads(destination.read_text(encoding="utf-8-sig")) != value:
            raise ProbeError("nonce outcome already exists with different binding")
    else:
        exclusive_json(destination, value)
    if source.exists():
        if marker.exists():
            raise ProbeError("nonce has both issued and consumed markers")
        os.rename(source, marker)
    elif not marker.is_file():
        raise ProbeError("nonce is absent or has no durable consumed marker")


def load_transcript(path: Path) -> dict:
    value = json.loads(path.read_text(encoding="utf-8-sig"))
    if not isinstance(value, dict) or value.get("schema") != SCHEMA:
        raise ProbeError("unsupported transcript")
    if value.get("transcript_sha256"):
        raise ProbeError("sealed transcript is immutable")
    return value


def process_binding(process) -> dict:
    exe = process.exe_path()
    return {
        "pid": process.pid,
        "creation_filetime_utc": process.creation_filetime(),
        "exe_path": str(exe),
        "exe_sha256": sha256_file(exe),
        "exe_file_id": file_identity(exe),
        "module_base": f"0x{process.module_base():x}",
    }


def assert_same_process(process, transcript: dict) -> int:
    actual = process_binding(process)
    if actual != transcript["process"]:
        raise ProbeError("PID/process creation/executable/module binding changed")
    if sha256_file(Path(__file__).resolve()) != transcript["tool"]["probe_sha256"]:
        raise ProbeError("probe tool bytes changed after initialization")
    return int(actual["module_base"], 16)


def bind_static_artifact(path: Path) -> None:
    """Read the pinned static-disasm artifact and REFUSE unless it describes the
    exact layout this probe uses -- every module RVA, field offset, std::string
    control-block offset (ptr/size/capacity), the SSO/heap threshold, and the
    identity grammar (finding 3). Because the collector both binds this and hashes
    it into the transcript, the pinned hash genuinely binds the layout actually
    dereferenced. Any mismatch is fail-closed -> ProbeError (nonce burns)."""
    value = json.loads(path.read_text(encoding="utf-8-sig"))
    if not isinstance(value, dict):
        raise ProbeError("static artifact is not a JSON object")
    if value.get("schema") != STATIC_SCHEMA:
        raise ProbeError("static artifact schema mismatch")
    if value.get("exe_sha256") != EXE_SHA:
        raise ProbeError("static artifact executable binding mismatch")
    if value.get("module_rvas") != {key: ROOTS[key] for key in STATIC_MODULE_KEYS}:
        raise ProbeError("static artifact module_rvas do not bind the probe's roots")
    if value.get("field_offsets") != {key: ROOTS[key] for key in STATIC_OFFSET_KEYS}:
        raise ProbeError("static artifact field_offsets do not bind the probe's offsets")
    layout = value.get("string_layout")
    if layout != {key: ROOTS[key] for key in STATIC_STRING_LAYOUT_KEYS}:
        raise ProbeError("static artifact string_layout does not bind the probe's std::string offsets")
    grammar = value.get("identity_grammar")
    if grammar != STATIC_GRAMMAR:
        raise ProbeError("static artifact identity_grammar does not bind the probe's UUID grammar")
    if not HEX64.fullmatch(str(value.get("evidence_export_sha256", ""))):
        raise ProbeError("static artifact evidence-export hash is not lowercase SHA-256")
    # Cross-check the pinned layout equals the constants the decode path compiles.
    if (int(layout["string_ptr_offset"], 16) != STRING_PTR_OFF or int(layout["string_size_offset"], 16) != STRING_SIZE_OFF or int(layout["string_capacity_offset"], 16) != STRING_CAP_OFF or int(layout["sso_heap_threshold"], 16) != SSO_HEAP_THRESHOLD):
        raise ProbeError("static artifact string layout disagrees with the probe's compiled offsets")
    # Cross-check the pinned grammar equals what validate_uuid_grammar enforces.
    if tuple(grammar["hyphen_positions"]) != UUID_HYPHEN_POSITIONS or grammar["hex_char_class"] != UUID_HEX_CHAR_CLASS or grammar["uuid_len"] != UUID_LEN:
        raise ProbeError("static artifact grammar disagrees with the probe's enforced UUID grammar")


def command_init(args) -> None:
    args._nonce_issued_by_this_invocation = False
    if args.action_token != "AUTHORIZE_READ_ONLY_CAMPAIGN_IDENTITY_EVIDENCE_INIT":
        raise ProbeError("explicit initialization action token missing")
    if not HEX64.fullmatch(args.evidence_nonce):
        raise ProbeError("evidence nonce must be a caller-issued lowercase 64-hex value")
    if not re.fullmatch(r"[A-Za-z0-9_.-]{8,128}", args.session_id):
        raise ProbeError("session id is invalid")
    if args.out.exists():
        raise ProbeError("refusing to reuse transcript path")
    ledger = args.nonce_ledger_root.resolve()
    ledger.mkdir(parents=True, exist_ok=True)
    created = now_filetime()
    issue = issued_path(ledger, args.evidence_nonce)
    history = [issue, issue.with_suffix(issue.suffix + ".consumed"), ledger / f"{args.evidence_nonce}.consumed_success.json", ledger / f"{args.evidence_nonce}.consumed_failure.json"]
    if any(path.exists() for path in history):
        raise ProbeError("evidence nonce has prior ledger history and cannot be reused")
    issued = {
        "schema": NONCE_ISSUE_SCHEMA,
        "state": "issued",
        "session_id": args.session_id,
        "evidence_nonce_hex": args.evidence_nonce,
        "transcript_path": str(args.out.resolve()),
        "created_filetime_utc": created,
    }
    exclusive_json(issue, issued)
    args._nonce_issued_by_this_invocation = True
    static = args.static_disasm.resolve()
    bind_static_artifact(static)
    probe = Path(__file__).resolve()
    with ReadOnlyProcess(args.pid) as process:
        binding = process_binding(process)
    if binding["exe_sha256"] != EXE_SHA or Path(binding["exe_path"]).name.lower() != "hoi4.exe":
        raise ProbeError("target is not the supported hoi4.exe build")
    transcript = {
        "schema": SCHEMA,
        "capture": {
            "session_id": args.session_id,
            "evidence_nonce_hex": args.evidence_nonce,
            "nonce_ledger_root": str(ledger),
            "created_filetime_utc": created,
            "completed_filetime_utc": 0,
        },
        "process": binding,
        "tool": {"probe_path": str(probe), "probe_sha256": sha256_file(probe)},
        "static_disasm": {"artifact_path": str(static), "artifact_sha256": sha256_file(static)},
        "roots": ROOTS,
        "observations": [],
        "transcript_sha256": "",
    }
    atomic_json(args.out, transcript)
    print(f"initialized campaign-identity transcript: {args.out}")


def command_observe(args) -> None:
    if not LABEL.fullmatch(args.label):
        raise ProbeError("observation label must match [A-Za-z0-9_]{2,64}")
    manual_token = f"OBSERVE_CAMPAIGN_IDENTITY_{args.label}_AND_ACK"
    barrier_mode = args.barrier_mode
    if barrier_mode is None:
        # Preserve historical invocations that carried the explicit AND_ACK
        # token while making the new automatic token the stdin-free path.
        barrier_mode = "manual_ack" if args.action_token == manual_token else "automatic_same_hour"
    if barrier_mode == "automatic_same_hour":
        expected_token = (
            f"OBSERVE_CAMPAIGN_IDENTITY_{args.label}_AUTOMATIC_SAME_HOUR_V1"
        )
    elif barrier_mode == "manual_ack":
        expected_token = manual_token
    else:
        raise ProbeError("unsupported campaign-identity barrier mode")
    if args.action_token != expected_token:
        raise ProbeError(f"explicit observation action token missing; expected {expected_token}")
    transcript = load_transcript(args.transcript)
    sequence = len(transcript["observations"]) + 1
    with ReadOnlyProcess(transcript["process"]["pid"]) as process:
        base = assert_same_process(process, transcript)
        gs = process.u64(base + CGAMESTATE_SINGLETON_RVA)
        clock_a = process.u64(base + CLOCK_SINGLETON_A_RVA)
        clock_b = process.u64(base + CLOCK_SINGLETON_B_RVA)
        expected_vtable = base + CLOCK_VTABLE_RVA
        if gs < 0x10000 or clock_a < 0x10000 or clock_a != clock_b or process.u64(clock_a) != expected_vtable:
            raise ProbeError("root/sentinel validation failed")
        addresses = {
            "cgame_state_slot": f"0x{base + CGAMESTATE_SINGLETON_RVA:x}",
            "clock_slot_a": f"0x{base + CLOCK_SINGLETON_A_RVA:x}",
            "clock_slot_b": f"0x{base + CLOCK_SINGLETON_B_RVA:x}",
            "clock_vtable": f"0x{expected_vtable:x}",
            "cgame_state": f"0x{gs:x}",
            "clock": f"0x{clock_a:x}",
            "current_hour": f"0x{gs + CURRENT_HOUR_OFF:x}",
            "game_unique_seed": f"0x{gs + GAME_UNIQUE_SEED_OFF:x}",
            "game_unique_id_string": f"0x{gs + GAME_UNIQUE_ID_OFF:x}",
        }
        print(f"observation '{args.label}': taking stable read A...", flush=True)
        snap_a = read_coherent_identity(process, base, args.max_attempts)
        read_a = read_record(snap_a, "A")
        if barrier_mode == "manual_ack":
            _operator_ack(f"read A captured (seed={snap_a['seed']}). Keep the game at this exact state, then press Enter to take bracketing read B... ")
        print(f"observation '{args.label}': taking bracketing read B...", flush=True)
        snap_b = read_coherent_identity(process, base, args.max_attempts)
        read_b = read_record(snap_b, "B")
    if not identity_fields_equal(read_a, read_b):
        raise ProbeError("bracketing reads A and B disagree on identity; the campaign identity is not stable across the observation window; refusing")
    if barrier_mode == "automatic_same_hour" and read_a["game_hour_raw"] != read_b["game_hour_raw"]:
        raise ProbeError("automatic campaign-identity barrier crossed a game-hour boundary; refusing")
    observation = {
        "sequence": sequence,
        "label": args.label,
        "action_nonce_hex": secrets.token_hex(32),
        "action_token": expected_token,
        "addresses": addresses,
        "read_a": read_a,
        "read_b": read_b,
    }
    transcript["observations"].append(observation)
    atomic_json(args.transcript, transcript)
    print(f"recorded bracketed observation sequence {sequence} (label '{args.label}'); no verdict emitted")


def command_seal(args) -> None:
    transcript = load_transcript(args.transcript)
    if args.action_token != "SEAL_CAMPAIGN_IDENTITY_TRANSCRIPT_WITHOUT_VERDICT":
        raise ProbeError("explicit seal token missing")
    if len(transcript["observations"]) < 1:
        raise ProbeError("transcript has no observations to seal")
    transcript["capture"]["completed_filetime_utc"] = now_filetime()
    transcript["transcript_sha256"] = transcript_digest(transcript)
    atomic_json(args.transcript, transcript)
    consume_nonce(
        Path(transcript["capture"]["nonce_ledger_root"]),
        transcript["capture"]["evidence_nonce_hex"],
        "consumed_success",
        transcript["capture"]["session_id"],
        args.transcript,
        transcript["transcript_sha256"],
        transcript["capture"]["created_filetime_utc"],
        transcript["capture"]["completed_filetime_utc"],
    )
    print(f"sealed transcript bytes only: {args.transcript}; run the independent verifier for a verdict")


def command_self_test(_args) -> None:
    passed = 0
    # Nonce-ledger lifecycle: issue -> consumed_failure; conflicting outcome and
    # exclusive-overwrite are both rejected.
    with tempfile.TemporaryDirectory(prefix="cid1_nonce_v1_") as temp:
        root = Path(temp)
        nonce = "a" * 64
        transcript = root / "transcript.json"
        transcript.write_text("{}", encoding="utf-8")
        issue = issued_path(root, nonce)
        issued = {"schema": NONCE_ISSUE_SCHEMA, "state": "issued", "session_id": "self_test", "evidence_nonce_hex": nonce, "transcript_path": str(transcript.resolve()), "created_filetime_utc": 100}
        exclusive_json(issue, issued)
        consume_nonce(root, nonce, "consumed_failure", "self_test", transcript, "", 100, 200)
        marker = issue.with_suffix(issue.suffix + ".consumed")
        outcome = root / f"{nonce}.consumed_failure.json"
        if issue.exists() or not marker.is_file() or not outcome.is_file():
            raise AssertionError("nonce was not durably consumed")
        if json.loads(marker.read_text(encoding="utf-8-sig")) != issued:
            raise AssertionError("consumed issue marker changed")
        passed += 1
        try:
            consume_nonce(root, nonce, "consumed_success", "self_test", transcript, "f" * 64, 100, 201)
        except ProbeError:
            passed += 1
        else:
            raise AssertionError("conflicting nonce outcome was accepted")
        try:
            exclusive_json(outcome, {})
        except FileExistsError:
            passed += 1
        else:
            raise AssertionError("exclusive outcome write overwrote history")

    # MSVC std::string decode: both heap and SSO paths, plus tear rejections.
    uuid = b"a4b17fed-ed12-4361-9064-fb6576638110"
    heap_region = bytearray(HEADER_REGION_LEN)
    struct.pack_into("<i", heap_region, 0, 1026046527)
    struct.pack_into("<Q", heap_region, STRING_OFF, 0x200000)  # chars_addr
    struct.pack_into("<Q", heap_region, STRING_OFF + 0x10, UUID_LEN)  # size
    struct.pack_into("<Q", heap_region, STRING_OFF + 0x18, 63)  # capacity (heap)
    raw, size, cap, storage = decode_msvc_std_string(bytes(heap_region), STRING_OFF, lambda addr, length: uuid + b"\x00" if (addr == 0x200000 and length == UUID_LEN + 1) else (_ for _ in ()).throw(ProbeError("bad chars addr")), MAX_STRING_CAPACITY, UUID_LEN)
    if raw != uuid or size != UUID_LEN or storage != "heap" or validate_uuid_grammar(raw) != uuid.decode():
        raise AssertionError("heap std::string decode failed")
    passed += 1
    sso = b"hello"
    sso_region = bytearray(64)
    sso_region[STRING_OFF:STRING_OFF + len(sso)] = sso  # inline chars in the control block
    struct.pack_into("<Q", sso_region, STRING_OFF + 0x10, len(sso))  # size
    struct.pack_into("<Q", sso_region, STRING_OFF + 0x18, 15)  # capacity (SSO)
    raw2, size2, cap2, storage2 = decode_msvc_std_string(bytes(sso_region), STRING_OFF, lambda *_: (_ for _ in ()).throw(AssertionError("SSO must not read heap")), MAX_STRING_CAPACITY, len(sso))
    if raw2 != sso or size2 != len(sso) or storage2 != "inline":
        raise AssertionError("SSO std::string decode failed")
    passed += 1
    # capacity < size -> torn (size matches the required 5, then cap 2 < size).
    torn = bytearray(sso_region)
    struct.pack_into("<Q", torn, STRING_OFF + 0x18, 2)  # cap < size
    try:
        decode_msvc_std_string(bytes(torn), STRING_OFF, lambda *_: b"", MAX_STRING_CAPACITY, len(sso))
    except ProbeError:
        passed += 1
    else:
        raise AssertionError("capacity < size was accepted")
    # capacity over ceiling -> torn.
    huge = bytearray(heap_region)
    struct.pack_into("<Q", huge, STRING_OFF + 0x18, MAX_STRING_CAPACITY + 1)
    try:
        decode_msvc_std_string(bytes(huge), STRING_OFF, lambda a, n: b"\x00" * n, MAX_STRING_CAPACITY, UUID_LEN)
    except ProbeError:
        passed += 1
    else:
        raise AssertionError("capacity over ceiling was accepted")
    # missing NUL terminator -> torn.
    try:
        decode_msvc_std_string(bytes(heap_region), STRING_OFF, lambda a, n: uuid + b"X", MAX_STRING_CAPACITY, UUID_LEN)
    except ProbeError:
        passed += 1
    else:
        raise AssertionError("missing NUL terminator was accepted")
    # FINDING 1: an over-long size is rejected BEFORE any heap dereference. A
    # garbage control block claiming size=capacity=1MiB must NOT issue a wide read.
    oversized = bytearray(HEADER_REGION_LEN)
    struct.pack_into("<Q", oversized, STRING_OFF, 0x200000)  # chars_addr
    struct.pack_into("<Q", oversized, STRING_OFF + STRING_SIZE_OFF, MAX_STRING_CAPACITY)  # size = 1MiB
    struct.pack_into("<Q", oversized, STRING_OFF + STRING_CAP_OFF, MAX_STRING_CAPACITY)  # cap = 1MiB (<= ceiling)

    def _must_not_read(_addr, _length):
        raise AssertionError("read_chars was called despite an over-long size (no exact-size gate)")

    try:
        decode_msvc_std_string(bytes(oversized), STRING_OFF, _must_not_read, MAX_STRING_CAPACITY, required_size=UUID_LEN)
    except ProbeError:
        passed += 1
    else:
        raise AssertionError("over-long size passed the exact-size gate")
    # UUID grammar rejection.
    try:
        validate_uuid_grammar(b"A4B17FED-ED12-4361-9064-FB6576638110")  # uppercase
    except ProbeError:
        passed += 1
    else:
        raise AssertionError("non-canonical UUID grammar was accepted")
    print(f"read-only campaign-identity collector self-test: passed={passed} failed=0")


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description="Read-only CID1 campaign-identity evidence collector (no verdict).")
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("self-test")
    init = sub.add_parser("init")
    init.add_argument("--pid", type=int, required=True)
    init.add_argument("--session-id", required=True)
    init.add_argument("--evidence-nonce", required=True)
    init.add_argument("--nonce-ledger-root", type=Path, required=True)
    init.add_argument("--static-disasm", type=Path, required=True)
    init.add_argument("--out", type=Path, required=True)
    init.add_argument("--action-token", required=True)
    observe = sub.add_parser("observe")
    observe.add_argument("--transcript", type=Path, required=True)
    observe.add_argument("--label", required=True)
    observe.add_argument("--action-token", required=True)
    observe.add_argument(
        "--barrier-mode",
        choices=("automatic_same_hour", "manual_ack"),
        default=None,
        help="automatic_same_hour is the production path; legacy AND_ACK action tokens infer manual_ack when this option is omitted",
    )
    observe.add_argument("--max-attempts", type=int, choices=range(2, MAX_COHERENT_ATTEMPTS_CEILING + 1), default=8)
    seal = sub.add_parser("seal")
    seal.add_argument("--transcript", type=Path, required=True)
    seal.add_argument("--action-token", required=True)
    args = parser.parse_args(argv)
    try:
        {"self-test": command_self_test, "init": command_init, "observe": command_observe, "seal": command_seal}[args.command](args)
    except (Exception, KeyboardInterrupt) as exc:
        try:
            if args.command == "init" and getattr(args, "_nonce_issued_by_this_invocation", False) and HEX64.fullmatch(str(getattr(args, "evidence_nonce", ""))):
                issue = issued_path(args.nonce_ledger_root.resolve(), args.evidence_nonce)
                if issue.is_file():
                    issued = json.loads(issue.read_text(encoding="utf-8-sig"))
                    consume_nonce(args.nonce_ledger_root.resolve(), args.evidence_nonce, "consumed_failure", args.session_id, args.out, "", issued["created_filetime_utc"])
            elif args.command in ("observe", "seal") and args.transcript.is_file():
                failed = json.loads(args.transcript.read_text(encoding="utf-8-sig"))
                capture = failed["capture"]
                consume_nonce(Path(capture["nonce_ledger_root"]), capture["evidence_nonce_hex"], "consumed_failure", capture["session_id"], args.transcript, failed.get("transcript_sha256", ""), capture["created_filetime_utc"])
        except Exception:
            pass
        print(f"read-only campaign-identity collector: FAIL: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
