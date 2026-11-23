#!/usr/bin/env python3
"""Print self CPU costs from collector worker_sampler files (Python 3.11 + numpy).

Little-endian header <8s4I3Q2I16s2IQ (88 bytes): HOI4WSMP, version,
header length, record size (32), interval_us, qpc_hz, tsc_hz, collector base,
image size/timestamp, RSDS GUID (Windows byte order), PDB age/path length,
game base, then the UTF-8 PDB path. Header length includes the path.
Each 32-byte record is QPC/cycle weight/PC (u64), slot/job role (u16),
thread role/worker index (u8), and two zero bytes. Job 0 means none, otherwise
table index + 1; worker 255 means none. There are no stack samples.
The UTF-8 JSON trailer contains record_count, dropped_counts, threads,
job_roles and modules. Footer <8sQQ is WSMPEND!, trailer offset/length.
Without a footer, complete records remain readable and drops are unknown.

Records are memory-mapped and aggregated in chunks. Unique PCs resolve once;
the bounded address cache spills to disk. Functions merge by symbol start.
Cycle weights measure CPU since each thread's previous check, so attributed
CPU may precede the sampled function or callback. Tables are estimates.
"""

from __future__ import annotations

import argparse
import bisect
import ctypes
import heapq
import json
import math
import ntpath
import os
from pathlib import Path
import re
import sqlite3
import struct
import sys
import tempfile
import time
import uuid
from dataclasses import dataclass
from typing import Callable

import numpy as np


HEADER = struct.Struct("<8s4I3Q2I16s2IQ")
FOOTER = struct.Struct("<8sQQ")
MAGIC = b"HOI4WSMP"
GAME_PREFERRED_BASE = 0x140000000
END_MAGIC = b"WSMPEND!"
VERSION = 1
RECORD_SIZE = 32
CHUNK_RECORDS = 250_000
ADDRESS_CACHE_SIZE = 131_072
ROLE_NAMES = (
    "runtime", "decode", "spool", "status", "spool_helper",
    "second_spool_helper", "spool_sizing", "acquisition", "game", "sampler",
)
RECORD_DTYPE = np.dtype({
    "names": ["qpc", "cycles", "pc", "slot", "job_role", "role", "worker", "padding"],
    "formats": ["<u8", "<u8", "<u8", "<u2", "<u2", "u1", "u1", ("u1", 2)],
    "offsets": [0, 8, 16, 24, 26, 28, 29, 30],
    "itemsize": RECORD_SIZE,
})


class ProfileError(Exception):
    """A named input or symbol-identity refusal."""


@dataclass(frozen=True)
class Header:
    header_length: int
    interval_us: int
    qpc_hz: int
    tsc_hz: int
    collector_base: int
    collector_size: int
    collector_timestamp: int
    pdb_guid: bytes
    pdb_age: int
    pdb_path: str
    game_base: int

    @property
    def guid(self) -> str:
        return str(uuid.UUID(bytes_le=self.pdb_guid)).upper()


def invalid_records(records: np.ndarray) -> np.ndarray:
    """Padding separates a final partial JSON trailer from valid crash records."""
    return (np.any(records["padding"] != 0, axis=1)
            | (records["pc"] == 0) | (records["role"] >= len(ROLE_NAMES)))


class SampleFile:
    def __init__(self, path: str | Path):
        self.path = Path(path)
        size = self.path.stat().st_size
        self.trailer: dict = {}
        self.truncated = True
        self.trailing_bytes = 0
        with self.path.open("rb") as source:
            prefix = source.read(HEADER.size)
            if len(prefix) != HEADER.size:
                raise ProfileError("worker sampler header is incomplete")
            (magic, version, header_len, record_size, interval,
             qpc_hz, tsc_hz, collector_base, collector_size, timestamp, guid,
             age, path_len, game_base) = HEADER.unpack(prefix)
            if magic != MAGIC:
                raise ProfileError("worker sampler magic is not HOI4WSMP")
            if version != VERSION:
                raise ProfileError(f"unsupported worker sampler format version {version}")
            if record_size != RECORD_SIZE:
                raise ProfileError(f"unsupported worker sampler record layout {record_size}")
            if header_len != HEADER.size + path_len or header_len > size:
                raise ProfileError("worker sampler header/PDB path length is invalid")
            try:
                pdb_path = source.read(path_len).decode("utf-8")
            except UnicodeDecodeError as error:
                raise ProfileError("worker sampler PDB path is not UTF-8") from error
            if not interval or not qpc_hz:
                raise ProfileError("worker sampler interval and QPC frequency must be nonzero")
            self.header = Header(header_len, interval, qpc_hz, tsc_hz,
                                 collector_base, collector_size, timestamp,
                                 guid, age, pdb_path, game_base)
            record_end = size
            if size >= header_len + FOOTER.size:
                source.seek(-FOOTER.size, os.SEEK_END)
                footer_magic, trailer_offset, trailer_len = FOOTER.unpack(source.read(FOOTER.size))
                if footer_magic == END_MAGIC:
                    if (trailer_offset < header_len
                            or trailer_offset + trailer_len != size - FOOTER.size
                            or (trailer_offset - header_len) % RECORD_SIZE):
                        raise ProfileError("worker sampler trailer bounds are invalid")
                    source.seek(trailer_offset)
                    try:
                        trailer = json.loads(source.read(trailer_len).decode("utf-8"))
                    except (ValueError, UnicodeDecodeError) as error:
                        raise ProfileError("worker sampler trailer is not valid UTF-8 JSON") from error
                    if not isinstance(trailer, dict):
                        raise ProfileError("worker sampler trailer must be a JSON object")
                    self.trailer = trailer
                    self.truncated = False
                    record_end = trailer_offset
        self.count, self.trailing_bytes = divmod(record_end - header_len, RECORD_SIZE)
        if not self.truncated and self.trailer.get("record_count") != self.count:
            raise ProfileError("worker sampler trailer record_count disagrees with the file")
        self.records = (np.memmap(self.path, dtype=RECORD_DTYPE, mode="r", offset=header_len,
                                  shape=(self.count,)) if self.count
                        else np.empty(0, dtype=RECORD_DTYPE))
        if self.truncated:
            for start in range(0, self.count, CHUNK_RECORDS):
                bad = np.flatnonzero(invalid_records(self.records[start:start + CHUNK_RECORDS]))
                if bad.size:
                    self.count = start + int(bad[0])
                    self.trailing_bytes = size - header_len - self.count * RECORD_SIZE
                    self.records = self.records[:self.count]
                    break

    def chunks(self, count: int = CHUNK_RECORDS):
        for start in range(0, self.count, count):
            chunk = self.records[start:start + count]
            bad = np.flatnonzero(invalid_records(chunk))
            if bad.size:
                raise ProfileError(f"worker sampler record {start + int(bad[0])} has invalid layout")
            yield chunk

    def close(self):
        records = self.records
        self.records = np.empty(0, dtype=RECORD_DTYPE)
        if isinstance(records, np.memmap):
            records._mmap.close()


def demangle_legacy(name: str) -> str:
    match = re.fullmatch(r"_?_ZN(.*)E(?:\..*)?", name)
    if not match:
        return name
    rest, parts = match.group(1), []
    while rest:
        length = re.match(r"\d+", rest)
        if not length:
            return name
        count = int(length.group())
        rest = rest[length.end():]
        if count > len(rest):
            return name
        parts.append(rest[:count])
        rest = rest[count:]
    if parts and re.fullmatch(r"h[0-9a-f]{16}", parts[-1]):
        parts.pop()
    escapes = {"$LT$": "<", "$GT$": ">", "$RF$": "&", "$BP$": "*", "$C$": ",",
               "$LP$": "(", "$RP$": ")", "$SP$": "@", "..": "::"}
    for index, part in enumerate(parts):
        if part.startswith("_$"):
            part = part[1:]
        for escape, replacement in escapes.items():
            part = part.replace(escape, replacement)
        part = re.sub(r"\$u([0-9a-f]+)\$", lambda m: chr(int(m.group(1), 16)), part)
        parts[index] = part
    return "::".join(parts)


class RustV0:
    """Common Rust v0 paths/types; unfamiliar encodings retain the original name.

    No external demangler is required. This handles nested paths, disambiguators,
    generic arguments, impls, backreferences, punycode identifiers and the ordinary
    primitive/reference/pointer/array/tuple types emitted in collector symbols.
    """

    PRIMITIVES = dict(zip("abcdefhijlmnopstuvxyz", (
        "i8", "bool", "char", "f64", "str", "f32", "u8", "isize", "usize",
        "i32", "u32", "i128", "u128", "_", "i16", "u16", "()", "...", "i64", "u64", "!",
    )))

    def __init__(self, value: str):
        self.value = value
        self.at = 0
        self.depth = 0

    def take(self) -> str:
        value = self.value[self.at]
        self.at += 1
        return value

    def peek(self) -> str:
        return self.value[self.at:self.at + 1]

    def number(self) -> int:
        value = 0
        digits = "0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ"
        if self.peek() == "_":
            self.at += 1
            return 0
        while self.peek() != "_":
            value = value * 62 + digits.index(self.take())
        self.at += 1
        return value + 1

    def disambiguator(self):
        if self.peek() == "s":
            self.at += 1
            self.number()

    def identifier(self) -> str:
        puny = self.peek() == "u"
        if puny:
            self.at += 1
        start = self.at
        while self.peek().isdigit():
            self.at += 1
        if self.at == start:
            raise ValueError("identifier length absent")
        length = int(self.value[start:self.at])
        if self.peek() == "_":
            self.at += 1
        result = self.value[self.at:self.at + length]
        if len(result) != length:
            raise ValueError("short identifier")
        self.at += length
        if puny:
            base, sep, suffix = result.rpartition("_")
            result = ((base + "-" if sep else "") + suffix).encode("ascii").decode("punycode")
        return result

    def backref(self, method: Callable[[], str]) -> str:
        offset = self.number()
        saved = self.at
        if offset >= saved:
            raise ValueError("forward Rust backreference")
        self.at = offset
        result = method()
        self.at = saved
        return result

    def path(self) -> str:
        self.depth += 1
        if self.depth > 80:
            raise ValueError("recursive Rust symbol")
        try:
            kind = self.take()
            if kind == "C":
                self.disambiguator()
                return self.identifier()
            if kind == "N":
                namespace = self.take()
                parent = self.path()
                self.disambiguator()
                child = self.identifier()
                if namespace.isupper():
                    label = {"C": "closure", "S": "shim"}.get(namespace, namespace)
                    child = "{" + label + (":" + child if child else "") + "}"
                return parent + "::" + child
            if kind == "I":
                parent = self.path()
                args = []
                while self.peek() != "E":
                    if self.peek() == "L":
                        self.at += 1
                        self.number()
                        args.append("'_")
                    elif self.peek() == "K":
                        self.at += 1
                        args.append(self.constant())
                    else:
                        args.append(self.type())
                self.at += 1
                return parent + "::<" + ", ".join(args) + ">"
            if kind in "MX":
                self.disambiguator()
                self.path()
                target = self.type()
                trait = " as " + self.path() if kind == "X" else ""
                return "<" + target + trait + ">"
            if kind == "Y":
                return "<" + self.type() + " as " + self.path() + ">"
            if kind == "B":
                return self.backref(self.path)
            raise ValueError("unknown Rust path")
        finally:
            self.depth -= 1

    def constant(self) -> str:
        kind = self.take()
        if kind == "B":
            return self.backref(self.constant)
        if self.peek() == "p":
            self.at += 1
            return "_"
        negative = self.peek() == "n"
        if negative:
            self.at += 1
        start = self.at
        while self.peek() != "_":
            self.take()
        value = int(self.value[start:self.at], 16)
        self.at += 1
        if kind == "b":
            return "true" if value else "false"
        if kind == "c":
            return repr(chr(value))
        return str(-value if negative else value)

    def type(self) -> str:
        kind = self.peek()
        if kind in self.PRIMITIVES:
            self.at += 1
            return self.PRIMITIVES[kind]
        if kind in "RQPOAS TBF".replace(" ", ""):
            self.at += 1
            if kind == "B":
                return self.backref(self.type)
            if kind in "RQ":
                if self.peek() == "L":
                    self.at += 1
                    self.number()
                return ("&mut " if kind == "Q" else "&") + self.type()
            if kind in "PO":
                return ("*mut " if kind == "O" else "*const ") + self.type()
            if kind == "A":
                return "[" + self.type() + "; " + self.constant() + "]"
            if kind == "S":
                return "[" + self.type() + "]"
            if kind in "TF":
                prefix = ""
                if kind == "F":
                    if self.peek() == "G":
                        self.at += 1
                        self.number()
                    if self.peek() == "U":
                        self.at += 1
                        prefix = "unsafe "
                    if self.peek() == "K":
                        self.at += 1
                        abi = self.take() if self.peek() == "C" else self.identifier()
                        prefix += f'extern "{abi}" '
                args = []
                while self.peek() != "E":
                    args.append(self.type())
                self.at += 1
                if kind == "F":
                    result = self.type()
                    return prefix + "fn(" + ", ".join(args) + ")" + (" -> " + result if result != "()" else "")
                return "(" + ", ".join(args) + ("," if len(args) == 1 else "") + ")"
        return self.path()


def demangle(name: str) -> str:
    if name.startswith(("_R", "R")):
        value = name[2:] if name.startswith("_R") else name[1:]
        try:
            return RustV0(value).path()
        except (IndexError, ValueError, UnicodeError, RecursionError):
            return name
    try:
        result = demangle_legacy(name)
    except (ValueError, UnicodeError):
        return name
    return re.sub(r"::h[0-9a-f]{16}$", "", result)


@dataclass(frozen=True)
class Module:
    base: int
    size: int
    path: str

    @property
    def name(self) -> str:
        return ntpath.basename(self.path)


@dataclass(frozen=True)
class Function:
    module: str
    address: int
    name: str

    @property
    def label(self) -> str:
        if self.module == "hoi4.exe":
            return self.name
        return self.module + ("" if self.name.startswith("+") else "!") + self.name


UNKNOWN_MODULE = Function("unknown", 0, "<outside any module>")


def game_code() -> Function:
    return Function("hoi4.exe", GAME_PREFERRED_BASE, "<hoi4.exe code>")


def unsymbolized(module: str, base: int) -> Function:
    return Function(module, base, f"<unsymbolized {module}>")


DWORD = ctypes.c_uint32
DWORD64 = ctypes.c_uint64
BOOL = ctypes.c_int32


class GUID(ctypes.Structure):
    _fields_ = [("Data1", DWORD), ("Data2", ctypes.c_uint16),
                ("Data3", ctypes.c_uint16), ("Data4", ctypes.c_ubyte * 8)]


class IMAGEHLP_MODULEW64(ctypes.Structure):
    _fields_ = [("SizeOfStruct", DWORD), ("BaseOfImage", DWORD64),
                ("ImageSize", DWORD), ("TimeDateStamp", DWORD), ("CheckSum", DWORD),
                ("NumSyms", DWORD), ("SymType", ctypes.c_int32),
                ("ModuleName", ctypes.c_wchar * 32), ("ImageName", ctypes.c_wchar * 256),
                ("LoadedImageName", ctypes.c_wchar * 256), ("LoadedPdbName", ctypes.c_wchar * 256),
                ("CVSig", DWORD), ("CVData", ctypes.c_wchar * (260 * 3)),
                ("PdbSig", DWORD), ("PdbSig70", GUID), ("PdbAge", DWORD),
                ("PdbUnmatched", BOOL), ("DbgUnmatched", BOOL), ("LineNumbers", BOOL),
                ("GlobalSymbols", BOOL), ("TypeInfo", BOOL), ("SourceIndexed", BOOL),
                ("Publics", BOOL), ("MachineType", DWORD), ("Reserved", DWORD)]


class SYMBOL_INFOW(ctypes.Structure):
    _fields_ = [("SizeOfStruct", DWORD), ("TypeIndex", DWORD), ("Reserved", DWORD64 * 2),
                ("Index", DWORD), ("Size", DWORD), ("ModBase", DWORD64), ("Flags", DWORD),
                ("Value", DWORD64), ("Address", DWORD64), ("Register", DWORD),
                ("Scope", DWORD), ("Tag", DWORD), ("NameLen", DWORD),
                ("MaxNameLen", DWORD), ("Name", ctypes.c_wchar * 1)]


def check_pdb_identity(header: Header, guid: bytes, age: int, loaded_path: str,
                       unmatched: bool = False):
    expected = f"{header.guid}/{header.pdb_age}"
    actual = f"{str(uuid.UUID(bytes_le=guid)).upper()}/{age}"
    if unmatched or guid != header.pdb_guid or age != header.pdb_age:
        raise ProfileError(f"collector.pdb identity mismatch: recorded {expected}, loaded {actual} ({loaded_path})")


class DbgHelpResolver:
    def __init__(self, samples: SampleFile, pdb_dir: str, dll: str | None):
        if os.name != "nt" or ctypes.sizeof(ctypes.c_void_p) != 8:
            raise ProfileError("symbolization requires 64-bit Windows Python; --self-test is portable")
        search = Path(pdb_dir).resolve(strict=True)
        if not search.is_dir():
            raise ProfileError(f"PDB search path is not a directory: {search}")
        self.header = samples.header
        if self.header.pdb_guid == bytes(16):
            raise ProfileError("collector.dll header has no RSDS PDB GUID; identity cannot be checked")
        self.modules = modules_from_file(samples)
        self.bases = [module.base for module in self.modules]
        self.dbg = ctypes.WinDLL("dbghelp", use_last_error=True)
        self.kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        self.kernel.GetCurrentProcess.argtypes = []
        self.kernel.GetCurrentProcess.restype = ctypes.c_void_p
        self.process = self.kernel.GetCurrentProcess()
        functions = {
            "SymSetOptions": ([DWORD], DWORD),
            "SymInitializeW": ([ctypes.c_void_p, ctypes.c_wchar_p, BOOL], BOOL),
            "SymCleanup": ([ctypes.c_void_p], BOOL),
            "SymLoadModuleExW": ([ctypes.c_void_p, ctypes.c_void_p, ctypes.c_wchar_p,
                                   ctypes.c_wchar_p, DWORD64, DWORD, ctypes.c_void_p, DWORD], DWORD64),
            "SymGetModuleInfoW64": ([ctypes.c_void_p, DWORD64, ctypes.POINTER(IMAGEHLP_MODULEW64)], BOOL),
            "SymFromAddrW": ([ctypes.c_void_p, DWORD64, ctypes.POINTER(DWORD64),
                               ctypes.POINTER(SYMBOL_INFOW)], BOOL),
        }
        for name, (arguments, result) in functions.items():
            try:
                function = getattr(self.dbg, name)
            except AttributeError as error:
                raise ProfileError(f"Windows dbghelp does not export {name}") from error
            function.argtypes = arguments
            function.restype = result
        self.dbg.SymSetOptions(0x2 | 0x4 | 0x200)
        if not self.dbg.SymInitializeW(self.process, str(search), False):
            raise ProfileError(f"SymInitializeW failed: Windows error {ctypes.get_last_error()}")
        self.initialized = True
        self.loaded: set[int] = set()
        self.symbol_buffer = ctypes.create_string_buffer(ctypes.sizeof(SYMBOL_INFOW) + 2048 * 2)
        self.symbol = ctypes.cast(self.symbol_buffer, ctypes.POINTER(SYMBOL_INFOW))
        self.symbol.contents.SizeOfStruct = ctypes.sizeof(SYMBOL_INFOW)
        self.symbol.contents.MaxNameLen = 2048
        try:
            image = Path(dll).resolve(strict=True) if dll else search / "collector.dll"
            if not dll and not image.is_file():
                image = search / (ntpath.basename(self.header.pdb_path) or "collector.pdb")
            if not image.is_file():
                raise ProfileError(f"collector symbols file is missing: {image}")
            self.load(Module(self.header.collector_base, self.header.collector_size, str(image)), required=True)
            displacement = DWORD64()
            self.dbg.SymFromAddrW(self.process, self.header.collector_base,
                                 ctypes.byref(displacement), self.symbol)
            info = IMAGEHLP_MODULEW64()
            info.SizeOfStruct = ctypes.sizeof(info)
            if not self.dbg.SymGetModuleInfoW64(self.process, self.header.collector_base, ctypes.byref(info)):
                raise ProfileError(f"SymGetModuleInfoW64 collector.dll failed: Windows error {ctypes.get_last_error()}")
            if not info.LoadedPdbName:
                raise ProfileError(f"collector.pdb was not loaded; expected {self.header.guid}/{self.header.pdb_age}")
            guid = ctypes.string_at(ctypes.addressof(info.PdbSig70), 16)
            check_pdb_identity(self.header, guid, int(info.PdbAge), info.LoadedPdbName,
                               bool(info.PdbUnmatched or info.DbgUnmatched))
            self.identity = {"checked": True, "guid": self.header.guid, "age": self.header.pdb_age,
                             "loaded_pdb": info.LoadedPdbName}
        except BaseException:
            self.close()
            raise

    def load(self, module: Module, required: bool = False):
        if module.base in self.loaded:
            return
        self.loaded.add(module.base)
        if not Path(module.path).is_file():
            if required:
                raise ProfileError(f"collector symbols file is missing: {module.path}")
            return
        loaded = self.dbg.SymLoadModuleExW(self.process, None, module.path, module.name,
                                          module.base, module.size, None, 0)
        if not loaded and required:
            raise ProfileError(f"SymLoadModuleExW collector.dll failed: Windows error {ctypes.get_last_error()}")

    def __call__(self, address: int) -> Function:
        index = bisect.bisect_right(self.bases, address) - 1
        module = self.modules[index] if index >= 0 else None
        if module is None or address >= module.base + module.size:
            return UNKNOWN_MODULE
        if module.base == self.header.game_base or module.name.lower() == "hoi4.exe":
            return game_code()
        if module.base != self.header.collector_base:
            self.load(module)
        displacement = DWORD64()
        if self.dbg.SymFromAddrW(self.process, address, ctypes.byref(displacement), self.symbol):
            raw = ctypes.wstring_at(ctypes.addressof(self.symbol.contents) + SYMBOL_INFOW.Name.offset,
                                    self.symbol.contents.NameLen)
            start = int(self.symbol.contents.Address)
            name = demangle(raw)
            return Function(module.name, start, name)
        return unsymbolized(module.name, module.base)

    def close(self):
        if getattr(self, "initialized", False):
            self.dbg.SymCleanup(self.process)
            self.initialized = False


def modules_from_file(samples: SampleFile) -> list[Module]:
    modules: dict[int, Module] = {}
    for item in samples.trailer.get("modules", []):
        try:
            module = Module(int(item["base"]), int(item["size"]), str(item["path"]))
        except (KeyError, TypeError, ValueError) as error:
            raise ProfileError("worker sampler module table entry is invalid") from error
        if module.base < 0 or module.size <= 0 or module.base + module.size > 1 << 64:
            raise ProfileError("worker sampler module table bounds are invalid")
        modules[module.base] = module
    header = samples.header
    modules[header.collector_base] = Module(header.collector_base, header.collector_size, "collector.dll")
    return sorted(modules.values(), key=lambda module: module.base)


class AddressCache:
    """Resolve every unique PC once without retaining all PC keys in RAM.

    Ordinary captures use only the dictionary. A high-cardinality capture spills
    it to a temporary sqlite index; the RAM dictionary then caches that index.
    Function aggregates remain keyed by symbol start, so many PCs in one function
    still consume only one function entry. sqlite is part of Python's stdlib.
    """

    def __init__(self, capacity: int = ADDRESS_CACHE_SIZE):
        self.capacity = capacity
        self.values: dict[int, int] = {}
        self.database: sqlite3.Connection | None = None
        self.directory: tempfile.TemporaryDirectory | None = None

    @staticmethod
    def key(address: int) -> bytes:
        return address.to_bytes(8, "little")

    def get(self, address: int) -> int | None:
        identity = self.values.get(address)
        if identity is not None or self.database is None:
            return identity
        row = self.database.execute("SELECT function FROM addresses WHERE pc = ?",
                                    (self.key(address),)).fetchone()
        if row is None:
            return None
        if len(self.values) >= self.capacity:
            self.values.clear()
        self.values[address] = int(row[0])
        return int(row[0])

    def add(self, address: int, identity: int):
        if len(self.values) >= self.capacity:
            if self.database is None:
                self.directory = tempfile.TemporaryDirectory(prefix="worker-profile-pcs-",
                                                              dir=tempfile.gettempdir())
                self.database = sqlite3.connect(Path(self.directory.name) / "addresses.sqlite")
                self.database.execute("PRAGMA cache_size = -2048")
                self.database.execute("PRAGMA journal_mode = OFF")
                self.database.execute("PRAGMA synchronous = OFF")
                self.database.execute("CREATE TABLE addresses (pc BLOB PRIMARY KEY, function INTEGER) WITHOUT ROWID")
                self.database.executemany("INSERT INTO addresses VALUES (?, ?)",
                                          ((self.key(pc), fid) for pc, fid in self.values.items()))
            self.values.clear()
        if self.database is not None:
            self.database.execute("INSERT INTO addresses VALUES (?, ?)", (self.key(address), identity))
        self.values[address] = identity

    def close(self):
        if self.database is not None:
            self.database.close()
            self.database = None
        if self.directory is not None:
            self.directory.cleanup()
            self.directory = None
        self.values.clear()


class Aggregates:
    def __init__(self, samples: SampleFile, resolver: Callable[[int], Function]):
        self.samples = samples
        self.resolver = resolver
        self.functions: list[Function] = []
        self.function_ids: dict[tuple[str, int, str], int] = {}
        self.address_cache = AddressCache()
        self.stats = np.zeros((64, 2), dtype=np.float64)
        self.groups: dict[str, dict[int, dict[int, tuple[int, float]]]] = {
            "role": {}, "job": {},
        }
        self.total_cycles = 0
        self.group_totals: dict[str, dict[int, tuple[int, float]]] = {kind: {} for kind in self.groups}

    def function_id(self, address: int) -> int:
        cached = self.address_cache.get(address)
        if cached is not None:
            return cached
        function = self.resolver(address)
        key = (function.module, function.address, function.name)
        identity = self.function_ids.get(key)
        if identity is None:
            identity = len(self.functions)
            if identity >= 1 << 32:
                raise ProfileError("function ids exceed the 32-bit group-key encoding")
            self.function_ids[key] = identity
            self.functions.append(function)
            if identity >= len(self.stats):
                self.stats = np.vstack((self.stats, np.zeros_like(self.stats)))
        self.address_cache.add(address, identity)
        return identity

    def map_pcs(self, chunk: np.ndarray) -> np.ndarray:
        unique, inverse = np.unique(chunk['pc'], return_inverse=True)
        identities = np.fromiter((self.function_id(int(pc)) for pc in unique), dtype=np.int64,
                                 count=len(unique))
        return identities[inverse]

    @staticmethod
    def sums(ids: np.ndarray, cycles: np.ndarray):
        unique, inverse, counts = np.unique(ids, return_inverse=True, return_counts=True)
        weighted = np.bincount(inverse, weights=cycles)
        return unique, counts, weighted

    @staticmethod
    def add_sparse(target: dict, keys, counts, cycles):
        for key, count, weight in zip(keys, counts, cycles):
            identity = int(key)
            before_count, before_cycles = target.get(identity, (0, 0.0))
            target[identity] = (before_count + int(count), before_cycles + float(weight))

    def add_groups(self, kind: str, group_ids: np.ndarray, self_ids: np.ndarray, cycles: np.ndarray):
        ids, counts, weights = self.sums(group_ids, cycles)
        self.add_sparse(self.group_totals[kind], ids, counts, weights)
        keys = ((group_ids.astype(np.uint64) << np.uint64(32))
                | self_ids.astype(np.uint64))
        ids, counts, weights = self.sums(keys, cycles)
        groups = self.groups[kind]
        for key, count, weight in zip(ids, counts, weights):
            identity = int(key)
            group, function = identity >> 32, identity & 0xFFFFFFFF
            entries = groups.setdefault(group, {})
            before_count, before_cycles = entries.get(function, (0, 0.0))
            entries[function] = (before_count + int(count), before_cycles + float(weight))

    def run(self, chunk_size: int = CHUNK_RECORDS, after_chunk: Callable[[], None] | None = None):
        try:
            return self._run(chunk_size, after_chunk)
        finally:
            self.address_cache.close()

    def _run(self, chunk_size: int, after_chunk: Callable[[], None] | None):
        for chunk in self.samples.chunks(chunk_size):
            cycles = chunk["cycles"].astype(np.float64)
            self.total_cycles += int(np.sum(chunk["cycles"], dtype=object))
            first = self.map_pcs(chunk)
            count = len(self.functions)
            self.stats[:count, 0] += np.bincount(first, minlength=count)
            self.stats[:count, 1] += np.bincount(first, weights=cycles, minlength=count)
            self.add_groups("role", chunk["role"], first, cycles)
            self.add_groups("job", chunk["job_role"], first, cycles)
            if after_chunk:
                after_chunk()
        return self

    def group_name(self, kind: str, identity: int) -> str:
        if kind == "role":
            return ROLE_NAMES[identity] if identity < len(ROLE_NAMES) else f"role:{identity}"
        if kind == "job":
            names = self.samples.trailer.get("job_roles", [])
            return "none" if not identity else (str(names[identity - 1]) if identity <= len(names)
                                               else f"job:{identity - 1} (name unavailable)")

    def rows(self, top: int, hours: float, entries=None):
        if entries is None:
            entries = {i: (int(values[0]), values[1])
                       for i, values in enumerate(self.stats[:len(self.functions)])
                       if values[0]}
        order = heapq.nlargest(top, entries, key=lambda i: (entries[i][1], -i))
        hz = self.samples.header.tsc_hz
        return [(self.functions[i].label, entries[i][0], entries[i][1],
                 entries[i][1] * 1000 / hz / hours if hz else None) for i in order]

    def print_report(self, top: int, hours: float, identity: dict):
        header = self.samples.header
        print(f"worker sampler: {self.samples.count:,} records; interval {header.interval_us} us; "
              f"{hours:g} game hours")
        print(f"collector.pdb identity checked: {identity['guid']}/{identity['age']}")
        if self.samples.truncated:
            print(f"TRUNCATED: no footer; {self.samples.trailing_bytes} trailing bytes excluded. "
                  "Drop counts, job names and non-collector module mapping may be unavailable.")
        drops = self.samples.trailer.get('dropped_counts') if not self.samples.truncated else 'unknown'
        print(f"drops: {drops}")
        print("CPU estimates use cycle weights since each thread's previous check; "
              "that CPU may precede the sampled function or game callback. "
              "Cycle sums use float64, exact through 2**53 cycles.")

        def table(title, rows):
            print(f"\n{title}\n{'samples':>10} {'cycle ms/h':>12}  function")
            for function, count, _, cost in rows:
                estimate = f"{cost:.4f}" if cost is not None else 'unavailable'
                print(f"{count:10,} {estimate:>12}  {function}")

        table("Overall self", self.rows(top, hours))
        for kind in self.groups:
            print(f"\n{kind} totals\n{'samples':>10} {'cycle ms/h':>12}  {kind}")
            for group, (count, cycles) in sorted(self.group_totals[kind].items(), key=lambda item: -item[1][1]):
                cost = cycles * 1000 / header.tsc_hz / hours if header.tsc_hz else None
                estimate = f"{cost:.4f}" if cost is not None else 'unavailable'
                print(f"{count:10,} {estimate:>12}  {self.group_name(kind, group)}")
            for group in sorted(self.groups[kind]):
                table(f"{kind}: {self.group_name(kind, group)} self",
                      self.rows(top, hours, self.groups[kind][group]))


def write_synthetic(path: Path, records: np.ndarray, trailer: dict, footer: bool = True,
                    repetitions: int = 1):
    pdb_path = b"collector.pdb"
    guid = uuid.UUID("12345678-1234-5678-90ab-cdef01234567").bytes_le
    with path.open("wb") as target:
        target.write(HEADER.pack(MAGIC, VERSION, HEADER.size + len(pdb_path), RECORD_SIZE,
                                 250, 10_000_000, 1_000_000, 0x180000000,
                                 0x10000, 123, guid, 2, len(pdb_path), GAME_PREFERRED_BASE))
        target.write(pdb_path)
        blob = records.tobytes()
        for _ in range(repetitions):
            target.write(blob)
        if footer:
            offset = target.tell()
            blob = json.dumps(trailer, separators=(",", ":")).encode("utf-8")
            target.write(blob)
            target.write(FOOTER.pack(END_MAGIC, offset, len(blob)))


def peak_working_set() -> int | None:
    if os.name != "nt":
        return None

    class PROCESS_MEMORY_COUNTERS(ctypes.Structure):
        _fields_ = [("cb", DWORD), ("PageFaultCount", DWORD)] + [
            (name, ctypes.c_size_t) for name in (
                "PeakWorkingSetSize", "WorkingSetSize", "QuotaPeakPagedPoolUsage",
                "QuotaPagedPoolUsage", "QuotaPeakNonPagedPoolUsage", "QuotaNonPagedPoolUsage",
                "PagefileUsage", "PeakPagefileUsage",
            )]

    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.GetCurrentProcess.argtypes = []
    kernel.GetCurrentProcess.restype = ctypes.c_void_p
    kernel.K32GetProcessMemoryInfo.argtypes = [ctypes.c_void_p,
                                              ctypes.POINTER(PROCESS_MEMORY_COUNTERS), DWORD]
    kernel.K32GetProcessMemoryInfo.restype = BOOL
    info = PROCESS_MEMORY_COUNTERS()
    info.cb = ctypes.sizeof(info)
    if not kernel.K32GetProcessMemoryInfo(kernel.GetCurrentProcess(), ctypes.byref(info), info.cb):
        raise ProfileError(f"self-test GetProcessMemoryInfo failed: Windows error {ctypes.get_last_error()}")
    return int(info.PeakWorkingSetSize)


def scale_self_test(directory: str):
    path = Path(directory) / "scale.bin"
    records = np.zeros(20_000, dtype=RECORD_DTYPE)
    records["qpc"] = np.arange(len(records)) * 2500
    records["cycles"] = 100
    records["role"] = 7
    records["worker"] = 0
    records["job_role"] = 1
    records["pc"] = np.uint64(0x180001000) + (np.arange(len(records)) % 512).astype(np.uint64) * 32
    repetitions = 500
    count = len(records) * repetitions
    trailer = {"record_count": count, "dropped_counts": {}, "threads": [],
               "job_roles": ["synthetic"], "modules": []}
    started = time.perf_counter()
    write_synthetic(path, records, trailer, repetitions=repetitions)
    samples = SampleFile(path)

    def limits():
        peak = peak_working_set()
        assert peak is None or peak < 2 * 1024**3, f"scale check exceeded 2 GiB: {peak}"

    def resolve(address: int) -> Function:
        index = (address - 0x180001000) // 32
        return Function("collector.dll", address, f"synthetic_{index}")

    try:
        aggregate = Aggregates(samples, resolve).run(after_chunk=limits)
        assert samples.count == 10_000_000
        assert aggregate.total_cycles == 1_000_000_000
        assert int(aggregate.stats[:, 0].sum()) == count
        assert len(aggregate.functions) == 512
        assert aggregate.group_totals["job"][1] == (count, 1_000_000_000)
        limits()
        elapsed = time.perf_counter() - started
        peak = peak_working_set()
    finally:
        samples.close()
    peak_text = f"{peak / 1024**2:.1f} MiB peak working set" if peak is not None else "peak working set unavailable"
    print(f"worker_profile scale self-test: PASS ({count:,} instruction-pointer records, 512 synthetic "
          f"functions, {elapsed:.2f} s including file creation, {peak_text})")


def self_test() -> int:
    with tempfile.TemporaryDirectory(prefix="worker-profile-self-test-", dir=tempfile.gettempdir()) as directory:
        path = Path(directory) / "synthetic.bin"
        records = np.zeros(5, dtype=RECORD_DTYPE)
        records["qpc"] = np.arange(5) * 2500
        records["cycles"] = [100, 200, 300, 400, 500]
        records["slot"] = [0, 0, 1, 2, 2]
        records["role"] = [7, 7, 8, 2, 8]
        records["worker"] = [0, 0, 255, 255, 255]
        records["job_role"] = [1, 1, 2, 0, 2]
        a, a2, b, c, game = 0x180001000, 0x180001004, 0x180002000, 0x180003000, GAME_PREFERRED_BASE + 0x10000
        records["pc"] = [a, a2, b, c, game]
        trailer = {"record_count": 5, "dropped_counts": {"write_backlog": 9},
                   "threads": [{"slot": 0, "thread_id": 101, "role": "acquisition", "worker_index": 0},
                               {"slot": 1, "thread_id": 102, "role": "game", "worker_index": None},
                               {"slot": 2, "thread_id": 103, "role": "spool", "worker_index": None}],
                   "job_roles": ["countries", "divisions"],
                   "modules": [{"base": 0x180000000, "size": 0x10000, "path": "collector.dll"},
                               {"base": GAME_PREFERRED_BASE, "size": 0x2000000, "path": "hoi4.exe"}]}
        write_synthetic(path, records, trailer)
        calls = []

        def resolve(address: int) -> Function:
            calls.append(address)
            if address in (a, a2):
                return Function("collector.dll", a, "A")
            if address == game:
                return game_code()
            return Function("collector.dll", address, {b: "B", c: "C"}[address])

        samples = SampleFile(path)
        try:
            assert not samples.truncated and samples.count == 5
            assert samples.header.guid == "12345678-1234-5678-90AB-CDEF01234567"
            assert samples.header.pdb_path == "collector.pdb"
            aggregates = Aggregates(samples, resolve)
            aggregates.address_cache.capacity = 2
            aggregates.run(chunk_size=2)
            ids = {function.name: index for index, function in enumerate(aggregates.functions)}
            assert HEADER.size == 88 and RECORD_DTYPE.itemsize == 32
            assert np.array_equal(aggregates.stats[ids["A"]], [2, 300])
            assert np.array_equal(aggregates.stats[ids["B"]], [1, 300])
            assert np.array_equal(aggregates.stats[ids["C"]], [1, 400])
            assert aggregates.total_cycles == 1500
            assert aggregates.group_totals["role"][8] == (2, 800)
            assert aggregates.groups["job"][1][ids["A"]] == (2, 300)
            assert len(calls) == 5 and len(set(calls)) == 5
            assert aggregates.rows(20, 2)[0][1:] == (1, 500, 0.25)
            assert aggregates.group_name('job', 1) == 'countries'
            assert aggregates.group_name('role', 7) == 'acquisition'
            assert aggregates.rows(20, 2, aggregates.groups['job'][1])[0][3] == 0.15
            assert set(aggregates.groups) == {'role', 'job'}
            check_pdb_identity(samples.header, samples.header.pdb_guid, 2, "synthetic.pdb")
            for guid, age in ((bytes(16), 2), (samples.header.pdb_guid, 3)):
                try:
                    check_pdb_identity(samples.header, guid, age, "wrong.pdb")
                except ProfileError as error:
                    assert "collector.pdb identity mismatch" in str(error)
                else:
                    raise AssertionError("mismatched PDB was accepted")
            assert len(modules_from_file(samples)) == 2
        finally:
            samples.close()
        many = Path(directory) / "unresolved.bin"
        count = 200_000
        unresolved = np.zeros(count, dtype=RECORD_DTYPE)
        unresolved["cycles"] = 1
        unresolved["pc"] = np.arange(count, dtype=np.uint64) * 16 + 0x10_0000_0000
        write_synthetic(many, unresolved, dict(trailer, record_count=count))
        samples = SampleFile(many)
        try:
            aggregates = Aggregates(samples, lambda address: UNKNOWN_MODULE if address & 16 == 0
                                    else unsymbolized("collector.dll", 0x180000000))
            aggregates.run(chunk_size=50_000)
            assert len(aggregates.functions) == 2
            assert int(aggregates.stats[:2, 0].sum()) == count and int(aggregates.stats[0, 0]) == count // 2
        finally:
            samples.close()
        truncated = Path(directory) / "truncated.bin"
        write_synthetic(truncated, records, trailer, footer=False)
        with truncated.open("ab") as stream:
            stream.write(b"incomplete")
        samples = SampleFile(truncated)
        try:
            assert samples.truncated and samples.count == 5 and samples.trailing_bytes == 10
            assert Aggregates(samples, resolve).run(1).total_cycles == 1500
        finally:
            samples.close()
        with truncated.open("ab") as stream:
            stream.write(json.dumps(trailer).encode("utf-8"))
        samples = SampleFile(truncated)
        try:
            assert samples.truncated and samples.count == 5
        finally:
            samples.close()
        empty = Path(directory) / "empty.bin"
        write_synthetic(empty, records[:0], {**trailer, "record_count": 0})
        samples = SampleFile(empty)
        try:
            assert Aggregates(samples, resolve).run().rows(10, 2) == []
        finally:
            samples.close()
        original = path.read_bytes()
        for label, offset, replacement in (
            ("version", 8, struct.pack("<I", VERSION + 1)),
            ("record layout", 16, struct.pack("<I", RECORD_SIZE + 1)),
            ("header/PDB path", 12, struct.pack("<I", HEADER.size)),
            ("trailer bounds", len(original) - 16, struct.pack("<Q", 1)),
        ):
            malformed = Path(directory) / "malformed.bin"
            changed = bytearray(original)
            changed[offset:offset + len(replacement)] = replacement
            malformed.write_bytes(changed)
            try:
                rejected = SampleFile(malformed)
            except ProfileError as error:
                assert label in str(error)
            else:
                rejected.close()
                raise AssertionError(f"invalid {label} was accepted")
        write_synthetic(Path(directory) / "malformed.bin", records, {**trailer, "record_count": 6})
        try:
            rejected = SampleFile(Path(directory) / "malformed.bin")
        except ProfileError as error:
            assert "record_count" in str(error)
        else:
            rejected.close()
            raise AssertionError("incorrect trailer record_count was accepted")
        assert demangle("_ZN4test3foo17h0123456789abcdefE") == "test::foo"
        assert demangle("_RNvC4test3foo") == "test::foo"
        assert demangle("_RINvC4test3foomE") == "test::foo::<u32>"
        assert demangle("_RNvCs1234_4test3foo") == "test::foo"
        assert demangle("ordinary_symbol") == "ordinary_symbol"
        assert demangle("_Rinvalid") == "_Rinvalid"
        scale_self_test(directory)
    print("worker_profile self-test: PASS (format, chunked weights, attribution, ms/h, "
          "truncation, empty input, PDB mismatch refusal, Rust demangling)")
    return 0


def positive_int(value: str) -> int:
    number = int(value)
    if number <= 0:
        raise argparse.ArgumentTypeError("must be a positive integer")
    return number


def positive_float(value: str) -> float:
    number = float(value)
    if not math.isfinite(number) or number <= 0:
        raise argparse.ArgumentTypeError("must be a finite positive number")
    return number


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("samples", nargs="?", help="worker-samples.bin")
    parser.add_argument("--pdb-dir", help="local directory containing the matching collector.pdb")
    parser.add_argument("--dll", help="matching collector.dll (a standalone collector.pdb can be loaded without it)")
    parser.add_argument("--hours", type=positive_float, help="required game hours covered, for cycle ms/h estimates")
    parser.add_argument("--top", type=positive_int, default=30, help="rows per ranking (default 30)")
    parser.add_argument("--self-test", action="store_true", help="check synthetic files and a 10M-record scale fixture without real PDBs")
    arguments = parser.parse_args()
    if arguments.self_test:
        return self_test()
    if not arguments.samples or not arguments.pdb_dir or arguments.hours is None:
        parser.error("samples, --pdb-dir and --hours are required unless --self-test is used")
    samples = None
    resolver = None
    try:
        samples = SampleFile(arguments.samples)
        resolver = DbgHelpResolver(samples, arguments.pdb_dir, arguments.dll)
        aggregate = Aggregates(samples, resolver).run()
        aggregate.print_report(arguments.top, arguments.hours, resolver.identity)
        return 0
    except (ProfileError, OSError, ValueError, KeyError, TypeError, sqlite3.Error) as error:
        print(f"worker_profile: {error}", file=sys.stderr)
        return 1
    finally:
        if resolver:
            resolver.close()
        if samples:
            samples.close()


if __name__ == "__main__":
    raise SystemExit(main())
