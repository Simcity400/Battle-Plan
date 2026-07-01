"""Current-build live probe for scripted state roots.

This is a lane-local READ-ONLY validator for the 870ebf the game build. It does not
edit or inject into the game. It uses the same current-build country root path as
the native collector, then validates/rejects scripted-state candidate roots.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import time
from collections import Counter
from datetime import datetime, timezone

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..", ".."))
MEMORY_READER = os.path.join(ROOT, "extractor", "memory_reader")
sys.path.insert(0, MEMORY_READER)

import winmem  # noqa: E402

EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"

HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000

CG_STATE_DATE_OFFSET = 0x3316E90
COUNTRY_DATABASE_OFFSET = 0x33189C8
CGAMESTATE_COUNTRY_TABLE = 0x310
CGAMESTATE_COUNTRY_COUNT = 0x31C
COUNTRY_VTABLE_RVA = 0x27D0E90
OBJ_TAG_DB_ID = 0x08
OBJ_NAME = 0x10

OBJ_VARIABLES = 0x218
RTTI_VARIABLES = ".?AVCVariables@@"
CV_BASE = 0x18
CV_SIZE_MASK = 0x20
NODE = 0x30
NODE_VALUE = 0x20
VALUE_SCALE_RAW = 100000

OBJ_FLAGS = 0x230
RTTI_FLAG_MANAGER = ".?AVCFlagManager@@"

MAX_COUNTRIES = 2048
MAX_SIZE = 200000
MAX_BUCKETS = 1 << 21
MAX_KEYLEN = 256


def hx(value):
    return "0x%X" % value if isinstance(value, int) else None


def heap_ok(value, align=8):
    return isinstance(value, int) and HEAP_LO <= value < HEAP_HI and value % align == 0


def try_u8(p, addr):
    try:
        return p.u8(addr)
    except OSError:
        return None


def try_u16(p, addr):
    try:
        return p.u16(addr)
    except OSError:
        return None


def try_u32(p, addr):
    try:
        return p.u32(addr)
    except OSError:
        return None


def try_i64(p, addr):
    try:
        return p.i64(addr)
    except OSError:
        return None


def try_u64(p, addr):
    try:
        return p.u64(addr)
    except OSError:
        return None


def try_read(p, addr, size):
    try:
        return p.read(addr, size)
    except OSError:
        return None


def read_cstr(p, addr, max_len=200):
    data = try_read(p, addr, max_len)
    if not data:
        return None
    end = data.find(b"\0")
    if end < 0:
        return None
    try:
        return data[:end].decode("latin-1")
    except UnicodeDecodeError:
        return None


def rtti_name(p, obj):
    vt = try_u64(p, obj)
    mb = p.module_base
    if not isinstance(vt, int) or not (mb <= vt < mb + 0x10000000):
        return None
    col = try_u64(p, vt - 8)
    if not isinstance(col, int) or not (mb <= col < mb + 0x10000000):
        return None
    td_rva = try_u32(p, col + 0x0C)
    if td_rva is None:
        return None
    name = read_cstr(p, mb + td_rva + 0x10, 200)
    return name if name and name.startswith(".?A") else None


def read_msvc_string(p, addr, max_len=MAX_KEYLEN):
    size = try_u64(p, addr + 0x10)
    cap = try_u64(p, addr + 0x18)
    if not isinstance(size, int) or not isinstance(cap, int):
        return None
    if size == 0 or size > max_len or size > cap or cap >= (1 << 20):
        return None
    inline = cap == 15
    if not inline and cap < 16:
        return None
    chars = addr if inline else try_u64(p, addr)
    if not isinstance(chars, int):
        return None
    data = try_read(p, chars, size + 1)
    if not data or len(data) < size + 1 or data[size] != 0:
        return None
    try:
        text = data[:size].decode("latin-1")
    except UnicodeDecodeError:
        return None
    if all(32 <= ord(ch) < 127 for ch in text):
        return text
    return None


def read_country_name(p, country_ptr):
    return read_msvc_string(p, country_ptr + OBJ_NAME, 96)


def stable_variables_header(p, container):
    b1 = try_u64(p, container + CV_BASE)
    w1 = try_u64(p, container + CV_SIZE_MASK)
    b2 = try_u64(p, container + CV_BASE)
    w2 = try_u64(p, container + CV_SIZE_MASK)
    if None in (b1, w1, b2, w2) or (b1, w1) != (b2, w2):
        return None
    return b1, w1 & 0xFFFFFFFF, w1 >> 32


def enumerate_variables(p, country_ptr):
    container = try_u64(p, country_ptr + OBJ_VARIABLES)
    if not heap_ok(container):
        return {
            "status": "unmeasured",
            "reason": "candidate_pointer_not_heap",
            "container": hx(container),
        }
    rtti = rtti_name(p, container)
    if rtti != RTTI_VARIABLES:
        return {
            "status": "unmeasured",
            "reason": "rtti_mismatch",
            "container": hx(container),
            "rtti": rtti,
        }
    hdr = stable_variables_header(p, container)
    if hdr is None:
        return {
            "status": "unmeasured",
            "reason": "header_unstable",
            "container": hx(container),
            "rtti": rtti,
        }
    base, size, mask = hdr
    if size == 0:
        return {
            "status": "measured_empty",
            "container": hx(container),
            "rtti": rtti,
            "node_base": hx(base),
            "declared_size": size,
            "bucket_mask": mask,
            "entry_count": 0,
            "sample": [],
        }
    nb = mask + 1
    if (
        not heap_ok(base)
        or size > MAX_SIZE
        or nb < 1
        or nb > MAX_BUCKETS
        or (mask & nb) != 0
        or size > nb
    ):
        return {
            "status": "unmeasured",
            "reason": "header_invalid",
            "container": hx(container),
            "rtti": rtti,
            "node_base": hx(base),
            "declared_size": size,
            "bucket_mask": mask,
        }

    end = base + (nb + 4) * NODE
    exact = None
    phase_counts = {}
    for phase in range(0, NODE, 8):
        tmp = {}
        dup = False
        addr = base + phase
        while addr + NODE <= end:
            key = read_msvc_string(p, addr)
            if key is not None:
                if key in tmp:
                    dup = True
                raw = try_i64(p, addr + NODE_VALUE)
                if raw is not None:
                    tmp[key] = raw
            addr += NODE
        phase_counts[str(phase)] = len(tmp)
        if not dup and len(tmp) == size:
            if exact is not None and exact != tmp:
                return {
                    "status": "unmeasured",
                    "reason": "phase_ambiguous",
                    "container": hx(container),
                    "rtti": rtti,
                    "node_base": hx(base),
                    "declared_size": size,
                    "bucket_mask": mask,
                    "phase_counts": phase_counts,
                }
            exact = tmp
    if exact is None:
        return {
            "status": "unmeasured",
            "reason": "size_gate_failed",
            "container": hx(container),
            "rtti": rtti,
            "node_base": hx(base),
            "declared_size": size,
            "bucket_mask": mask,
            "phase_counts": phase_counts,
        }
    sample = [
        {"key": key, "raw": raw, "decoded": raw / VALUE_SCALE_RAW}
        for key, raw in list(sorted(exact.items()))[:8]
    ]
    return {
        "status": "measured",
        "container": hx(container),
        "rtti": rtti,
        "node_base": hx(base),
        "declared_size": size,
        "bucket_mask": mask,
        "entry_count": len(exact),
        "phase_counts": phase_counts,
        "sample": sample,
    }


def probe_flags_candidate(p, country_ptr):
    manager = try_u64(p, country_ptr + OBJ_FLAGS)
    result = {
        "candidate_offset": hx(OBJ_FLAGS),
        "manager": hx(manager),
        "status": "unmeasured",
    }
    if not heap_ok(manager):
        result["reason"] = "candidate_pointer_not_heap"
        return result
    rtti = rtti_name(p, manager)
    result["rtti"] = rtti
    if rtti != RTTI_FLAG_MANAGER:
        result["reason"] = "rtti_mismatch"
        return result
    result["status"] = "root_validated_payload_unresolved"
    qwords = []
    for off in range(0, 0x80, 8):
        value = try_u64(p, manager + off)
        if value is not None:
            qwords.append({"off": hx(off), "u64": hx(value), "heap": heap_ok(value)})
    result["qword_scan_0x00_0x78"] = qwords
    return result


def tag_table_sample(p, country_database):
    table = try_u64(p, country_database)
    tag_count = try_u32(p, country_database + 8)
    active_count = try_u32(p, country_database + 12)
    sample = []
    if heap_ok(table) and isinstance(tag_count, int):
        for idx in range(min(tag_count, 16)):
            tag = read_msvc_string(p, table + idx * 32, 16)
            if tag:
                sample.append({"tag_db_id": idx, "tag": tag})
    return {
        "country_database": hx(country_database),
        "tag_table": hx(table),
        "tag_count": tag_count,
        "active_count": active_count,
        "sample": sample,
    }


def resolve_country_roots(p):
    module_base = p.module_base
    gamestate = try_u64(p, module_base + CG_STATE_DATE_OFFSET)
    if not heap_ok(gamestate):
        raise RuntimeError("CGameState global did not resolve to a heap pointer")
    country_table = try_u64(p, gamestate + CGAMESTATE_COUNTRY_TABLE)
    country_count = try_u32(p, gamestate + CGAMESTATE_COUNTRY_COUNT)
    country_vtable = module_base + COUNTRY_VTABLE_RVA
    if not heap_ok(country_table) or not isinstance(country_count, int):
        raise RuntimeError("country table/count did not resolve")
    if country_count <= 0 or country_count > MAX_COUNTRIES:
        raise RuntimeError(f"country count out of bounds: {country_count}")
    rows = []
    for index in range(country_count):
        country_ptr = try_u64(p, country_table + index * 8)
        if not heap_ok(country_ptr):
            continue
        vtable = try_u64(p, country_ptr)
        tag_db_id = try_u32(p, country_ptr + OBJ_TAG_DB_ID)
        if vtable != country_vtable or tag_db_id != index:
            continue
        rows.append(
            {
                "tag_db_id": index,
                "country_ptr": country_ptr,
                "name": read_country_name(p, country_ptr),
            }
        )
    if not rows:
        raise RuntimeError("no country rows passed vtable/tag-db-id validation")
    return {
        "module_base": module_base,
        "gamestate": gamestate,
        "country_table": country_table,
        "country_count": country_count,
        "country_vtable": country_vtable,
        "rows": rows,
    }


def run_probe(max_countries, include_empty):
    p = winmem.open_process("hoi4.exe")
    if p is None:
        raise SystemExit("hoi4.exe not running")
    with p:
        fp = p.fingerprint()
        sha = str(fp.get("sha256") or "").lower()
        if sha != EXPECTED_SHA256:
            raise SystemExit(
                "live hoi4.exe sha256 %s != expected %s" % (sha, EXPECTED_SHA256)
            )
        started = time.perf_counter()
        roots = resolve_country_roots(p)
        country_database = try_u64(p, p.module_base + COUNTRY_DATABASE_OFFSET)
        measured = []
        variable_status = Counter()
        flag_status = Counter()
        flag_rtti = Counter()
        first_flag_valid = None
        first_variable_unmeasured = None
        scanned = 0
        for row in roots["rows"]:
            if scanned >= max_countries:
                break
            scanned += 1
            var = enumerate_variables(p, row["country_ptr"])
            variable_status[var["status"]] += 1
            if var["status"] == "unmeasured" and first_variable_unmeasured is None:
                first_variable_unmeasured = {
                    "tag_db_id": row["tag_db_id"],
                    "name": row["name"],
                    "country_ptr": hx(row["country_ptr"]),
                    "variable_probe": var,
                }
            flag = probe_flags_candidate(p, row["country_ptr"])
            flag_status[flag["status"]] += 1
            flag_rtti[str(flag.get("rtti"))] += 1
            if flag["status"] == "root_validated_payload_unresolved" and first_flag_valid is None:
                first_flag_valid = {
                    "tag_db_id": row["tag_db_id"],
                    "name": row["name"],
                    "country_ptr": hx(row["country_ptr"]),
                    "flag_probe": flag,
                }
            if include_empty or var["status"] == "measured" or flag["status"].startswith("root_"):
                measured.append(
                    {
                        "tag_db_id": row["tag_db_id"],
                        "name": row["name"],
                        "country_ptr": hx(row["country_ptr"]),
                        "variables": var,
                        "flags": flag,
                    }
                )
        elapsed_ms = int((time.perf_counter() - started) * 1000)
        variable_measured_rows = sum(
            item["variables"].get("entry_count", 0)
            for item in measured
            if item["variables"]["status"] == "measured"
        )
        return {
            "schema": "scripted_state_flags_variables_live_current_build_probe/v1",
            "created_utc": datetime.now(timezone.utc).isoformat(),
            "pid": p.pid,
            "sha256": sha,
            "elapsed_ms": elapsed_ms,
            "source_roots": {
                "module_base": hx(roots["module_base"]),
                "gamestate_global": hx(p.module_base + CG_STATE_DATE_OFFSET),
                "gamestate": hx(roots["gamestate"]),
                "country_table_offset": hx(CGAMESTATE_COUNTRY_TABLE),
                "country_table": hx(roots["country_table"]),
                "country_count_offset": hx(CGAMESTATE_COUNTRY_COUNT),
                "country_count": roots["country_count"],
                "country_vtable_rva": hx(COUNTRY_VTABLE_RVA),
                "country_vtable": hx(roots["country_vtable"]),
                "validated_country_rows": len(roots["rows"]),
                "country_database": tag_table_sample(p, country_database or 0),
            },
            "candidate_offsets": {
                "country_variables": hx(OBJ_VARIABLES),
                "country_flags": hx(OBJ_FLAGS),
            },
            "summary": {
                "countries_scanned": scanned,
                "variables_status": dict(variable_status),
                "variable_entries_measured_in_emitted_rows": variable_measured_rows,
                "flags_status": dict(flag_status),
                "flags_rtti": dict(flag_rtti),
            },
            "first_variable_unmeasured": first_variable_unmeasured,
            "first_flag_valid": first_flag_valid,
            "rows": measured,
        }


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--out", required=True, help="proof JSON path")
    ap.add_argument("--max-countries", type=int, default=256)
    ap.add_argument("--include-empty", action="store_true")
    args = ap.parse_args()
    result = run_probe(args.max_countries, args.include_empty)
    os.makedirs(os.path.dirname(os.path.abspath(args.out)), exist_ok=True)
    with open(args.out, "w", encoding="utf-8") as f:
        json.dump(result, f, indent=2, sort_keys=True)
        f.write("\n")
    print(json.dumps(result["summary"], indent=2, sort_keys=True))
    print(args.out)


if __name__ == "__main__":
    raise SystemExit(main())
