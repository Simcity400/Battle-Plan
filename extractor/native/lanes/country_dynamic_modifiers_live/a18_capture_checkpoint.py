#!/usr/bin/env python3
"""Capture one exact-a729 same-hour dynamic-modifier save/live checkpoint.

The probe is read-only. It binds the loaded hoi4.exe, PID+creation time, live
game hour, stable save hash, save date, owner identity, and every exact instance
field. It writes no game memory and refuses a torn container or save whose date
does not equal the bracketed live hour.
"""

from __future__ import annotations

import argparse
import ctypes
import ctypes.wintypes as wt
import hashlib
import json
import os
import sys
import tempfile
from datetime import datetime, timezone
from decimal import Decimal, InvalidOperation
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[3]
MEMORY_READER = REPO / "extractor" / "memory_reader"
SAVE_ORACLE = REPO / "extractor" / "save_oracle"
sys.path.insert(0, str(MEMORY_READER))
sys.path.insert(0, str(SAVE_ORACLE))

import winmem  # noqa: E402
import parse_country_extra  # noqa: E402
import parse_save  # noqa: E402

EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
CGAMESTATE_GLOBAL_RVA = 0x3316E90
COUNTRY_DATABASE_GLOBAL_RVA = 0x33189C8
CGAMESTATE_CURRENT_DATE = 0x468
CGAMESTATE_COUNTRY_TABLE = 0x310
CGAMESTATE_COUNTRY_COUNT = 0x31C
COUNTRY_VTABLE_RVA = 0x27D0E90
COUNTRY_TAG_DB_ID = 0x08
COUNTRY_DYNAMIC_MODIFIER_CONTAINER = 0xE58
CONTAINER_VTABLE_RVA = 0x27BFDE8
DEFINITION_VTABLE_RVA = 0x27BFD48
CONTAINER_BACKREF = 0x08
CONTAINER_ENTRY_ARRAY = 0x28
CONTAINER_CAPACITY = 0x30
CONTAINER_COUNT = 0x34
ENTRY_STRIDE = 0x40
ENTRY_DAYS = 0x10
ENTRY_DEFINITION = 0x18
ENTRY_ENABLED = 0x20
ENTRY_VALUES_BEGIN = 0x28
ENTRY_VALUES_CAPACITY = 0x30
ENTRY_VALUES_COUNT = 0x34
DEFINITION_ID = 0x28
MAX_COUNTRIES = 2048
MAX_ENTRIES = 512
MAX_VALUES = 256
DATE_EPOCH_CONST = 43_799_999
MONTH_LENGTHS = (31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31)


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def process_create_time(pid: int) -> int:
    query_limited = 0x1000

    class FileTime(ctypes.Structure):
        _fields_ = [("low", wt.DWORD), ("high", wt.DWORD)]

    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.OpenProcess.argtypes = [wt.DWORD, wt.BOOL, wt.DWORD]
    kernel.OpenProcess.restype = wt.HANDLE
    kernel.GetProcessTimes.argtypes = [
        wt.HANDLE,
        ctypes.POINTER(FileTime),
        ctypes.POINTER(FileTime),
        ctypes.POINTER(FileTime),
        ctypes.POINTER(FileTime),
    ]
    kernel.GetProcessTimes.restype = wt.BOOL
    kernel.CloseHandle.argtypes = [wt.HANDLE]
    handle = kernel.OpenProcess(query_limited, False, pid)
    if not handle:
        raise RuntimeError("cannot query hoi4.exe creation time")
    try:
        creation, exit_time, kernel_time, user_time = (FileTime() for _ in range(4))
        if not kernel.GetProcessTimes(
            handle,
            ctypes.byref(creation),
            ctypes.byref(exit_time),
            ctypes.byref(kernel_time),
            ctypes.byref(user_time),
        ):
            raise RuntimeError("GetProcessTimes failed")
        return (int(creation.high) << 32) | int(creation.low)
    finally:
        kernel.CloseHandle(handle)


def stable_read(proc, address: int, size: int) -> bytes:
    first = proc.read(address, size)
    second = proc.read(address, size)
    if first != second:
        raise RuntimeError("torn read at 0x%x" % address)
    return first


def u32(data: bytes, offset: int = 0) -> int:
    return int.from_bytes(data[offset : offset + 4], "little")


def i32(data: bytes, offset: int = 0) -> int:
    return int.from_bytes(data[offset : offset + 4], "little", signed=True)


def u64(data: bytes, offset: int = 0) -> int:
    return int.from_bytes(data[offset : offset + 8], "little")


def heap_pointer(value: int) -> bool:
    return isinstance(value, int) and 0x10000000000 <= value < 0x7FF000000000 and value % 8 == 0


def read_msvc_string(proc, address: int, limit: int = 128) -> str:
    header = stable_read(proc, address, 0x20)
    length, capacity = u64(header, 0x10), u64(header, 0x18)
    if not 0 < length <= limit or length > capacity or capacity >= 1 << 20:
        raise RuntimeError("invalid MSVC string header at 0x%x" % address)
    chars = address if capacity == 15 else u64(header)
    if capacity != 15 and (capacity < 16 or not heap_pointer(chars)):
        raise RuntimeError("invalid MSVC string storage at 0x%x" % address)
    raw = stable_read(proc, chars, length + 1)
    if raw[-1] != 0:
        raise RuntimeError("unterminated MSVC string at 0x%x" % address)
    value = raw[:-1].decode("utf-8")
    if not all(ch.isprintable() for ch in value):
        raise RuntimeError("non-printable modifier id")
    return value


def tag_table(proc, database: int) -> dict[int, str]:
    header = stable_read(proc, database, 16)
    table, count = u64(header), u32(header, 8)
    if not heap_pointer(table) or not 0 < count <= MAX_COUNTRIES:
        raise RuntimeError("country tag table is invalid")
    out: dict[int, str] = {}
    for index in range(count):
        try:
            tag = read_msvc_string(proc, table + index * 0x20, 16)
        except (OSError, RuntimeError, UnicodeDecodeError):
            continue
        if len(tag) == 3 and tag.isascii():
            out[index] = tag
    return out


def country_roots(proc, game_state: int) -> dict[int, int]:
    table = u64(stable_read(proc, game_state + CGAMESTATE_COUNTRY_TABLE, 8))
    count = u32(stable_read(proc, game_state + CGAMESTATE_COUNTRY_COUNT, 4))
    if not heap_pointer(table) or not 0 < count <= MAX_COUNTRIES:
        raise RuntimeError("country root table is invalid")
    expected_vtable = proc.module_base + COUNTRY_VTABLE_RVA
    out: dict[int, int] = {}
    for index in range(count):
        pointer = u64(stable_read(proc, table + index * 8, 8))
        if not heap_pointer(pointer):
            continue
        identity = stable_read(proc, pointer, 12)
        if u64(identity) == expected_vtable and u32(identity, COUNTRY_TAG_DB_ID) == index:
            out[index] = pointer
    return out


def snapshot_owner(proc, owner: int) -> list[dict]:
    container = owner + COUNTRY_DYNAMIC_MODIFIER_CONTAINER
    header = stable_read(proc, container, 0x38)
    if u64(header) != proc.module_base + CONTAINER_VTABLE_RVA or u64(header, CONTAINER_BACKREF) != owner:
        raise RuntimeError("owner dynamic-modifier container identity failed")
    data, capacity, count = (
        u64(header, CONTAINER_ENTRY_ARRAY),
        u32(header, CONTAINER_CAPACITY),
        u32(header, CONTAINER_COUNT),
    )
    if count > capacity or count > MAX_ENTRIES or (count and not heap_pointer(data)):
        raise RuntimeError("dynamic-modifier vector header invalid")
    rows = []
    for index in range(count):
        address = data + index * ENTRY_STRIDE
        raw = stable_read(proc, address, ENTRY_STRIDE)
        definition = u64(raw, ENTRY_DEFINITION)
        if not heap_pointer(definition):
            raise RuntimeError("dynamic-modifier definition pointer invalid")
        if u64(stable_read(proc, definition, 8)) != proc.module_base + DEFINITION_VTABLE_RVA:
            raise RuntimeError("dynamic-modifier definition vtable mismatch")
        modifier_id = read_msvc_string(proc, definition + DEFINITION_ID)
        enabled_raw = raw[ENTRY_ENABLED]
        if enabled_raw not in (0, 1):
            raise RuntimeError("dynamic-modifier enabled byte invalid")
        days_raw = i32(raw, ENTRY_DAYS)
        if days_raw < -1:
            raise RuntimeError("dynamic-modifier days sentinel invalid")
        values_begin, values_capacity, values_count = (
            u64(raw, ENTRY_VALUES_BEGIN),
            u32(raw, ENTRY_VALUES_CAPACITY),
            u32(raw, ENTRY_VALUES_COUNT),
        )
        if values_count > values_capacity or values_count > MAX_VALUES:
            raise RuntimeError("dynamic-modifier values header invalid")
        values_blob = b""
        if values_count:
            if not heap_pointer(values_begin):
                raise RuntimeError("dynamic-modifier values pointer invalid")
            values_blob = stable_read(proc, values_begin, values_count * 8)
        masked = bytearray(raw)
        for start in (0, ENTRY_DEFINITION, ENTRY_VALUES_BEGIN):
            masked[start : start + 8] = b"\0" * 8
        rows.append(
            {
                "source_ordinal": index,
                "modifier_id": modifier_id,
                "enabled": bool(enabled_raw),
                "values_raw": [
                    int.from_bytes(values_blob[i : i + 8], "little", signed=True)
                    for i in range(0, len(values_blob), 8)
                ],
                "remaining_days": None if days_raw == -1 else days_raw,
                "pointer_masked_entry_hex": masked.hex(),
            }
        )
    if stable_read(proc, container, 0x38) != header:
        raise RuntimeError("dynamic-modifier container changed during snapshot")
    return rows


def date_to_raw(value: str) -> int:
    parts = value.split(".")
    if len(parts) != 4:
        raise ValueError("save date is not year.month.day.hour")
    year, month, day, hour = map(int, parts)
    if not (1850 <= year <= 2200 and 1 <= month <= 12 and 1 <= day <= MONTH_LENGTHS[month - 1] and 0 <= hour < 24):
        raise ValueError("save date out of supported range")
    day_index = sum(MONTH_LENGTHS[: month - 1]) + day - 1
    return DATE_EPOCH_CONST + ((year * 365 + day_index) * 24 + hour)


def fixed_five(value) -> int:
    try:
        decimal = Decimal(str(value)) * Decimal(100_000)
    except (InvalidOperation, ValueError) as exc:
        raise ValueError("save modifier value is not numeric") from exc
    integral = decimal.to_integral_value()
    if decimal != integral:
        raise ValueError("save modifier value is not exact fixed-five")
    return int(integral)


def parse_oracle(save: Path, rakaly: str | None) -> dict:
    executable = parse_save.find_rakaly(rakaly)
    with tempfile.TemporaryDirectory(prefix="a18_save_oracle_") as tmp:
        text, melted = parse_save.melt_if_needed(save, executable, Path(tmp))
        outputs, _report = parse_country_extra.build(text, save, melted)
    return outputs["country_extra.json"]


def instance_map(rows: list[dict]) -> dict[str, dict]:
    out = {}
    for row in rows:
        modifier_id = row.get("modifier_id") or row.get("modifier")
        if not isinstance(modifier_id, str) or modifier_id in out:
            raise ValueError("modifier identities are missing or duplicated")
        out[modifier_id] = row
    return out


def joined_row(live: dict, save: dict) -> dict:
    save_values = [fixed_five(value) for value in save.get("value", [])]
    checks = {
        "enabled_equal": live.get("enabled") is save.get("enabled"),
        "values_fixed_five_equal": live.get("values_raw") == save_values,
        "remaining_days_equal": live.get("remaining_days") == save.get("days"),
    }
    return {
        "modifier_id": live["modifier_id"],
        "enabled": live["enabled"],
        "values_raw": live["values_raw"],
        "value_scale": 100_000,
        "remaining_days": live["remaining_days"],
        "checks": checks,
        "accepted": all(checks.values()),
    }


def run(args) -> dict:
    save = Path(args.save).resolve()
    before_stat = save.stat()
    before_hash = sha256_file(save)
    oracle = parse_oracle(save, args.rakaly)
    after_hash = sha256_file(save)
    after_stat = save.stat()
    if (before_hash, before_stat.st_size, before_stat.st_mtime_ns) != (
        after_hash,
        after_stat.st_size,
        after_stat.st_mtime_ns,
    ):
        raise RuntimeError("save changed while it was parsed")

    matches = [(pid, name) for pid, name in winmem.list_processes() if name.lower() == "hoi4.exe"]
    if len(matches) != 1:
        raise RuntimeError("checkpoint requires exactly one hoi4.exe process")
    proc = winmem.Process(matches[0][0], "hoi4.exe")
    with proc:
        fingerprint = proc.fingerprint()
        executable_sha = str(fingerprint.get("sha256") or "").lower()
        if executable_sha != EXPECTED_SHA256:
            raise RuntimeError("live executable SHA-256 does not match exact a729")
        creation = process_create_time(proc.pid)
        game_state = u64(stable_read(proc, proc.module_base + CGAMESTATE_GLOBAL_RVA, 8))
        if not heap_pointer(game_state):
            raise RuntimeError("CGameState singleton did not resolve")
        hour_before = i32(stable_read(proc, game_state + CGAMESTATE_CURRENT_DATE, 4))
        database = u64(stable_read(proc, proc.module_base + COUNTRY_DATABASE_GLOBAL_RVA, 8))
        tags = tag_table(proc, database)
        tag_ids = [tag_id for tag_id, tag in tags.items() if tag == args.owner_tag]
        if len(tag_ids) != 1:
            raise RuntimeError("owner tag did not resolve uniquely")
        owner_tag_db_id = tag_ids[0]
        roots = country_roots(proc, game_state)
        if owner_tag_db_id not in roots:
            raise RuntimeError("owner country root is absent")
        live_rows = snapshot_owner(proc, roots[owner_tag_db_id])
        hour_after = i32(stable_read(proc, game_state + CGAMESTATE_CURRENT_DATE, 4))
        if hour_before != hour_after:
            raise RuntimeError("game hour changed during checkpoint")

    save_date = oracle.get("meta", {}).get("save_date")
    save_hour_raw = date_to_raw(save_date)
    if save_hour_raw != hour_before:
        raise RuntimeError(
            "save/live hour mismatch: save=%d live=%d" % (save_hour_raw, hour_before)
        )
    save_rows = oracle.get("countries", {}).get(args.owner_tag, {}).get("dynamic_modifiers")
    if not isinstance(save_rows, list):
        raise RuntimeError("save oracle owner dynamic_modifiers missing")
    live_by_id, save_by_id = instance_map(live_rows), instance_map(save_rows)

    joins = []
    for modifier_id in (args.permanent_modifier, args.timed_modifier):
        live, saved = live_by_id.get(modifier_id), save_by_id.get(modifier_id)
        if args.phase == "post_removal" and modifier_id == args.timed_modifier:
            joins.append(
                {
                    "modifier_id": modifier_id,
                    "absent_live": live is None,
                    "absent_save": saved is None,
                    "accepted": live is None and saved is None,
                }
            )
            continue
        if live is None or saved is None:
            raise RuntimeError("required modifier %s is absent" % modifier_id)
        joins.append(joined_row(live, saved))
    if not all(join["accepted"] for join in joins):
        raise RuntimeError("one or more save/live modifier joins failed")
    permanent = next(join for join in joins if join["modifier_id"] == args.permanent_modifier)
    if permanent.get("remaining_days", object()) is not None:
        raise RuntimeError("declared permanent modifier did not carry the -1/null sentinel")
    if args.phase != "post_removal":
        timed = next(join for join in joins if join["modifier_id"] == args.timed_modifier)
        if not isinstance(timed.get("remaining_days"), int) or timed["remaining_days"] < 1:
            raise RuntimeError("positive checkpoint timed modifier must have at least one day")

    instance_material = "%s|%d|%d" % (executable_sha, proc.pid, creation)
    return {
        "schema": "hoi4.a18.dynamic_modifier_save_live_checkpoint.v1",
        "created_utc": datetime.now(timezone.utc).isoformat(),
        "phase": args.phase,
        "process_identity": {
            "pid": proc.pid,
            "create_time_100ns": creation,
            "instance_id": hashlib.sha256(instance_material.encode()).hexdigest()[:16],
            "executable_sha256": executable_sha,
            "module_base": hex(fingerprint["base"]),
        },
        "save": {
            "sha256": before_hash,
            "size_bytes": before_stat.st_size,
            "mtime_ns": before_stat.st_mtime_ns,
            "date": save_date,
            "game_hour_raw": save_hour_raw,
            "version": oracle.get("meta", {}).get("save_version"),
        },
        "live": {
            "game_hour_raw_before": hour_before,
            "game_hour_raw_after": hour_after,
            "owner_tag": args.owner_tag,
            "owner_tag_db_id": owner_tag_db_id,
            "validated_country_count": len(roots),
            "instance_count": len(live_rows),
        },
        "target": {
            "permanent_modifier": args.permanent_modifier,
            "timed_modifier": args.timed_modifier,
        },
        "joins": joins,
        "same_hour": True,
        "accepted": True,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--save", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--owner-tag", required=True)
    parser.add_argument("--timed-modifier", required=True)
    parser.add_argument("--permanent-modifier", required=True)
    parser.add_argument("--phase", required=True, choices=("positive_before", "positive_after", "post_removal"))
    parser.add_argument("--rakaly")
    args = parser.parse_args()
    result = run(args)
    out = Path(args.out).resolve()
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(out)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
