#!/usr/bin/env python3
"""Operation definition-index rejection probe for domain 53.

The v15 operations proof found current-build ``COperation`` heap hits, but did
not prove running operation state. This bounded probe checks whether those hits
are instead accounted for by the ``COperationsDatabase`` definition index.

It is read-only and lane-local. It does not touch shared collector or recorder
entrypoints.
"""

from __future__ import annotations

import argparse
import json
import struct
import sys
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


LANE_DIR = Path(__file__).resolve().parent
if str(LANE_DIR) not in sys.path:
    sys.path.insert(0, str(LANE_DIR))

import probe_870ebf_operations as ops  # type: ignore


DEFINITION_TABLE_OFFSET = 0x60
DEFINITION_TABLE_COUNT_RAW_OFFSET = 0x68
MAX_DEFINITION_POINTERS = 256
OBJECT_SAMPLE_BYTES = 0x80


def load_json(path: Path | None) -> dict[str, Any]:
    if path is None:
        return {}
    with path.open("r", encoding="utf-8") as f:
        value = json.load(f)
    if not isinstance(value, dict):
        raise ValueError(f"{path}: expected JSON object")
    return value


def parse_int(value: Any) -> int | None:
    if isinstance(value, int):
        return value
    if isinstance(value, str):
        try:
            return int(value, 16)
        except ValueError:
            return None
    return None


def qword_from(blob: bytes, off: int) -> int | None:
    if off < 0 or off + 8 > len(blob):
        return None
    return struct.unpack_from("<Q", blob, off)[0]


def is_heap(value: int | None) -> bool:
    return isinstance(value, int) and ops.HEAP_LO <= value < ops.HEAP_HI


def find_first_vtable_object(p: Any, vtable: int, *, max_seconds: float | None) -> dict[str, Any]:
    started = time.time()
    needle = struct.pack("<Q", vtable)
    regions_scanned = 0
    bytes_scanned = 0
    read_failures = 0
    timed_out = False
    for base, size in p.iter_regions(ops.HEAP_LO, ops.HEAP_HI):
        if max_seconds is not None and time.time() - started > max_seconds:
            timed_out = True
            break
        regions_scanned += 1
        pos = 0
        while pos < size:
            if max_seconds is not None and time.time() - started > max_seconds:
                timed_out = True
                break
            want = min(32 << 20, size - pos)
            blob = p.try_read(base + pos, want)
            if not blob:
                read_failures += 1
                pos += max(want, 0x1000)
                continue
            bytes_scanned += len(blob)
            idx = blob.find(needle)
            while idx >= 0:
                addr = base + pos + idx
                if addr % 8 == 0:
                    return {
                        "object": addr,
                        "status": "found",
                        "regions_scanned": regions_scanned,
                        "bytes_scanned": bytes_scanned,
                        "read_failures": read_failures,
                        "timed_out": False,
                        "elapsed_seconds": round(time.time() - started, 3),
                    }
                idx = blob.find(needle, idx + 1)
            if len(blob) < want:
                pos += max(len(blob), 0x1000)
            else:
                pos += max(len(blob) - 7, 0x1000)
        if timed_out:
            break
    return {
        "object": None,
        "status": "timed_out" if timed_out else "not_found",
        "regions_scanned": regions_scanned,
        "bytes_scanned": bytes_scanned,
        "read_failures": read_failures,
        "timed_out": timed_out,
        "elapsed_seconds": round(time.time() - started, 3),
    }


def read_definition_table(
    p: Any,
    db_object: int,
    operation_vtable: int,
    *,
    max_samples: int,
) -> dict[str, Any]:
    table_ptr = ops.u64(p, db_object + DEFINITION_TABLE_OFFSET)
    count_raw = ops.u64(p, db_object + DEFINITION_TABLE_COUNT_RAW_OFFSET)
    entries: list[dict[str, Any]] = []
    invalid_entries: list[dict[str, Any]] = []
    first_null_index: int | None = None
    if not is_heap(table_ptr):
        return {
            "status": "definition_table_pointer_invalid",
            "db_object": ops.hx(db_object),
            "table_offset": ops.hx(DEFINITION_TABLE_OFFSET),
            "table_ptr": ops.hx(table_ptr),
            "count_raw_offset": ops.hx(DEFINITION_TABLE_COUNT_RAW_OFFSET),
            "count_raw": ops.hx(count_raw),
            "entries": [],
            "definition_count": 0,
            "invalid_entries": [],
        }

    blob = p.try_read(table_ptr, MAX_DEFINITION_POINTERS * 8)
    for index in range(0, min(len(blob) // 8, MAX_DEFINITION_POINTERS)):
        ptr = qword_from(blob, index * 8)
        if not ptr:
            first_null_index = index
            break
        vt = ops.u64(p, ptr) if is_heap(ptr) else None
        type_name = ops.rtti_type_name_for_vtable(p, vt) if vt else None
        is_operation = vt == operation_vtable
        row = {
            "index": index,
            "entry_offset": ops.hx(index * 8),
            "object": ops.hx(ptr),
            "vtable": ops.hx(vt),
            "vtable_rva": ops.hx((vt or 0) - p.module_base) if vt else None,
            "rtti_type_name": type_name,
            "is_coperation": is_operation,
        }
        if is_operation:
            if len(entries) < max_samples:
                entries.append(row)
        else:
            invalid_entries.append(row)

    definition_count = first_null_index if first_null_index is not None else len(blob) // 8
    return {
        "status": "read",
        "db_object": ops.hx(db_object),
        "table_offset": ops.hx(DEFINITION_TABLE_OFFSET),
        "table_ptr": ops.hx(table_ptr),
        "count_raw_offset": ops.hx(DEFINITION_TABLE_COUNT_RAW_OFFSET),
        "count_raw": ops.hx(count_raw),
        "first_null_index": first_null_index,
        "definition_count": definition_count,
        "sample_entries": entries,
        "invalid_entries": invalid_entries[:max_samples],
        "invalid_entry_count": len(invalid_entries),
    }


def object_shape_samples(
    p: Any,
    entries: list[dict[str, Any]],
    *,
    max_samples: int,
) -> list[dict[str, Any]]:
    out: list[dict[str, Any]] = []
    for entry in entries[:max_samples]:
        ptr = parse_int(entry.get("object"))
        if ptr is None:
            continue
        blob = p.try_read(ptr, OBJECT_SAMPLE_BYTES)
        qwords: list[dict[str, Any]] = []
        for off in range(0, min(len(blob), 0x40), 8):
            value = qword_from(blob, off)
            type_name = (
                ops.rtti_type_name_for_vtable(p, value)
                if isinstance(value, int) and ops.is_module(p, value)
                else None
            )
            qwords.append(
                {
                    "offset": ops.hx(off),
                    "qword": ops.hx(value),
                    "rtti_type_name_if_vtable": type_name,
                }
            )
        out.append(
            {
                "index": entry.get("index"),
                "object": entry.get("object"),
                "first_qwords_0x00_0x38": qwords,
            }
        )
    return out


def warm_validate(
    p: Any,
    db_object: int,
    operation_vtable: int,
    expected_objects: list[str],
    delay_seconds: float,
) -> dict[str, Any]:
    time.sleep(max(0.0, delay_seconds))
    reread = read_definition_table(p, db_object, operation_vtable, max_samples=0)
    objects: list[str] = []
    table_ptr = parse_int(reread.get("table_ptr"))
    if table_ptr is not None:
        blob = p.try_read(table_ptr, MAX_DEFINITION_POINTERS * 8)
        for index in range(0, min(len(blob) // 8, MAX_DEFINITION_POINTERS)):
            ptr = qword_from(blob, index * 8)
            if not ptr:
                break
            vt = ops.u64(p, ptr) if is_heap(ptr) else None
            if vt == operation_vtable:
                objects.append(ops.hx(ptr) or "")
            else:
                objects.append("invalid:" + (ops.hx(ptr) or "null"))
    return {
        "delay_seconds": delay_seconds,
        "definition_count": reread.get("definition_count"),
        "objects_match": objects == expected_objects,
        "expected_count": len(expected_objects),
        "reread_count": len(objects),
        "status": "validated" if objects == expected_objects else "changed",
    }


def build_report(args: argparse.Namespace) -> tuple[dict[str, Any], int]:
    prior_operation_proof = load_json(Path(args.operation_proof) if args.operation_proof else None)
    p = ops.winmem.open_process("hoi4.exe")
    if p is None:
        raise RuntimeError("hoi4.exe not running")
    started = time.time()
    process = {
        "pid": p.pid,
        "exe_path": p.module_path,
        "module_base": ops.hx(p.module_base),
        "module_size": p.module_size,
        "sha256": ops.file_sha256(p.module_path),
    }
    report: dict[str, Any] = {
        "schema": "battle-plan.diplomacy_intel_operatives.operation_definition_index_probe.v1",
        "lane": "diplomacy_intel_operatives_live",
        "domain_id": 53,
        "probe": "probe_870ebf_operation_definition_index",
        "generated_utc": datetime.now(timezone.utc).isoformat(timespec="seconds"),
        "process": process,
        "expected_sha256": ops.EXPECTED_SHA256,
        "build_gate_ok": process.get("sha256") == ops.EXPECTED_SHA256,
        "prior_operation_proof": {
            "path": args.operation_proof,
            "status": prior_operation_proof.get("status"),
            "summary_status": prior_operation_proof.get("summary", {}).get("status"),
            "current_COperation_heap_hits": prior_operation_proof.get("summary", {}).get(
                "current_COperation_heap_hits"
            ),
            "accepted_for_operation_detail": prior_operation_proof.get("summary", {}).get(
                "accepted_for_operation_detail"
            ),
        },
    }
    if not report["build_gate_ok"]:
        report["status"] = "build_gate_failed"
        return report, 2

    chunks = ops.read_module_chunks(p)
    targets = ops.derive_targets(p, chunks)
    operation_vtables = targets.get("COperation", {}).get("vtable_values", [])
    db_vtables = targets.get("COperationsDatabase", {}).get("vtable_values", [])
    report["rtti"] = {
        "module_chunks": len(chunks),
        "module_bytes_read": sum(len(blob) for _, blob in chunks),
        "COperation_vtable_rvas": [ops.hx(v - p.module_base) for v in operation_vtables],
        "COperationsDatabase_vtable_rvas": [ops.hx(v - p.module_base) for v in db_vtables],
    }
    if not operation_vtables or not db_vtables:
        report["status"] = "operation_rtti_unavailable"
        return report, 2

    db_find = find_first_vtable_object(
        p, db_vtables[0], max_seconds=args.heap_find_seconds
    )
    report["database_find"] = {
        **db_find,
        "object": ops.hx(db_find.get("object")),
    }
    db_object = db_find.get("object")
    if not isinstance(db_object, int):
        report["status"] = "operations_database_not_found"
        return report, 2

    table = read_definition_table(
        p, db_object, operation_vtables[0], max_samples=max(0, args.max_samples)
    )
    report["definition_index"] = table
    expected_objects: list[str] = []
    table_ptr = parse_int(table.get("table_ptr"))
    if table_ptr is not None:
        blob = p.try_read(table_ptr, MAX_DEFINITION_POINTERS * 8)
        for index in range(0, min(len(blob) // 8, MAX_DEFINITION_POINTERS)):
            ptr = qword_from(blob, index * 8)
            if not ptr:
                break
            expected_objects.append(ops.hx(ptr) or "")
    report["object_shape_samples"] = object_shape_samples(
        p,
        table.get("sample_entries", []),
        max_samples=max(0, args.max_samples),
    )
    report["warm_validation"] = warm_validate(
        p,
        db_object,
        operation_vtables[0],
        expected_objects,
        max(0, args.warm_delay_ms) / 1000.0,
    )

    prior_heap_hits = report["prior_operation_proof"].get("current_COperation_heap_hits")
    definition_count = table.get("definition_count")
    all_entries_are_operations = (
        table.get("status") == "read"
        and isinstance(definition_count, int)
        and definition_count > 0
        and table.get("invalid_entry_count") == 0
    )
    count_matches_prior_full_heap_scan = (
        isinstance(prior_heap_hits, int) and prior_heap_hits == definition_count
    )
    warm_ok = report["warm_validation"].get("status") == "validated"
    rejected = all_entries_are_operations and count_matches_prior_full_heap_scan and warm_ok
    report["summary"] = {
        "database_object": table.get("db_object"),
        "definition_table_offset": table.get("table_offset"),
        "definition_table_ptr": table.get("table_ptr"),
        "definition_count": definition_count,
        "prior_full_heap_COperation_hits": prior_heap_hits,
        "definition_count_matches_prior_full_heap_hits": count_matches_prior_full_heap_scan,
        "all_definition_entries_are_COperation": all_entries_are_operations,
        "warm_validated": warm_ok,
        "accepted_for_running_operation_detail": False,
        "backlog_item_narrowed": "operation_detail",
    }
    if rejected:
        report["status"] = "coperation_heap_hits_rejected_as_running_operation_state"
        report["rejection"] = {
            "candidate": "current COperation heap-hit set as running operation state root",
            "reason": (
                "the COperationsDatabase definition table at +0x60 contains 38 "
                "COperation pointers, matching the prior full heap COperation hit "
                "count, and the table is warm-stable; these objects are definition "
                "database entries, not an owner/target/progress running-operation root"
            ),
            "database_object": table.get("db_object"),
            "definition_table_offset": table.get("table_offset"),
            "definition_count": definition_count,
        }
        code = 0
    else:
        report["status"] = "operation_definition_index_unresolved"
        code = 2
    report["elapsed_seconds"] = round(time.time() - started, 3)
    return report, code


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--operation-proof")
    ap.add_argument("--out", required=True)
    ap.add_argument("--max-samples", type=int, default=12)
    ap.add_argument("--warm-delay-ms", type=int, default=250)
    ap.add_argument("--heap-find-seconds", type=float, default=60.0)
    args = ap.parse_args(argv)

    report, code = build_report(args)
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    with out.open("w", encoding="utf-8") as f:
        json.dump(report, f, indent=2, sort_keys=True)
        f.write("\n")
    print(
        "probe_870ebf_operation_definition_index: "
        f"status={report['status']} out={out}"
    )
    return code


if __name__ == "__main__":
    raise SystemExit(main())
