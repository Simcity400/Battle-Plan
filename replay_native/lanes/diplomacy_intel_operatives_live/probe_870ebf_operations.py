#!/usr/bin/env python3
"""Bounded current-build operations-detail route probe for domain44.

This probe is lane-local and read-only. It derives current 870ebf MSVC RTTI
vtables for operation-related classes and rejects stale 16022851 operation RVAs
when they no longer point at the expected RTTI vtables. A heap vtable scan is
available behind --heap-scan, but is opt-in because module RTTI is enough to
reject stale vtable-RVA routes.

It does not claim running-operation detail extraction unless an actual running
operation root is found. The older residual showed COperationsDatabase is the
operation definition registry, not the running COperation instance list; this
probe current-builds that route so the integration slot does not port a stale
or definition-only root as live operation detail.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import struct
import sys
import time
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parents[3]
MEMORY_READER = REPO_ROOT / "memory_reader"
if str(MEMORY_READER) not in sys.path:
    sys.path.insert(0, str(MEMORY_READER))

import winmem  # type: ignore


EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"

HEAP_LO = 0x0000010000000000
HEAP_HI = 0x0000800000000000
CHUNK = 32 << 20

TARGET_RTTI_NAMES = {
    "COperationsDatabase": b".?AVCOperationsDatabase@@",
    "TReloadableGameItemDatabaseSpecific<COperationsDatabase,COperation>": (
        b".?AV?$TReloadableGameItemDatabaseSpecific@VCOperationsDatabase@@VCOperation@@@@"
    ),
    "COperation": b".?AVCOperation@@",
    "COperationPhase": b".?AVCOperationPhase@@",
    "COperationPhasesDatabase": b".?AVCOperationPhasesDatabase@@",
    "COperationToken": b".?AVCOperationToken@@",
    "COperationTokensDatabase": b".?AVCOperationTokensDatabase@@",
    "COperationResources": b".?AVCOperationResources@@",
}

OLD_OPERATION_RVAS_16022851 = {
    "COperationsDatabase": 0x26F4CB0,
    "TReloadableGameItemDatabaseSpecific<COperationsDatabase,COperation>": 0x26F4C70,
    "COperationPhase": 0x26F4BB0,
    "COperationToken": 0x26F4DE0,
    "COperationResources": 0x26F4B60,
}


def hx(value: int | None) -> str | None:
    return "0x%X" % value if isinstance(value, int) else None


def file_sha256(path: str | None) -> str | None:
    if not path:
        return None
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def read_module_chunks(p: winmem.Process) -> list[tuple[int, bytes]]:
    chunks: list[tuple[int, bytes]] = []
    end = p.module_base + p.module_size
    for base, size in p.iter_regions(p.module_base, end, want_exec=None):
        blob = p.try_read(base, size)
        if blob:
            chunks.append((base, blob))
    return chunks


def derive_rtti_vtables(
    p: winmem.Process, chunks: list[tuple[int, bytes]], decorated_name: bytes
) -> dict[str, Any]:
    module_start = p.module_base
    module_end = p.module_base + p.module_size
    string_addrs: list[int] = []
    for base, blob in chunks:
        pos = 0
        while True:
            idx = blob.find(decorated_name, pos)
            if idx < 0:
                break
            string_addrs.append(base + idx)
            pos = idx + 1

    col_hits: list[dict[str, int]] = []
    vtables: list[int] = []
    for string_addr in string_addrs:
        type_descriptor = string_addr - 0x10
        type_descriptor_rva = type_descriptor - module_start
        if not (module_start <= type_descriptor < module_end):
            continue
        pattern = struct.pack("<I", type_descriptor_rva)
        for base, blob in chunks:
            pos = 0
            while True:
                idx = blob.find(pattern, pos)
                if idx < 0:
                    break
                col = base + idx - 0x0C
                raw = p.try_read(col, 24)
                if len(raw) >= 24:
                    sig, off, cd_off, td_ref, class_ref, self_ref = struct.unpack(
                        "<IIIIII", raw[:24]
                    )
                    if sig in (0, 1) and td_ref == type_descriptor_rva and self_ref == col - module_start:
                        col_hits.append(
                            {
                                "string_addr": string_addr,
                                "type_descriptor": type_descriptor,
                                "complete_object_locator": col,
                                "signature": sig,
                                "offset": off,
                                "cd_offset": cd_off,
                                "class_descriptor_rva": class_ref,
                            }
                        )
                pos = idx + 1

    for hit in col_hits:
        col_ptr = struct.pack("<Q", hit["complete_object_locator"])
        for base, blob in chunks:
            pos = 0
            while True:
                idx = blob.find(col_ptr, pos)
                if idx < 0:
                    break
                vtable = base + idx + 8
                if vtable not in vtables:
                    vtables.append(vtable)
                pos = idx + 1

    return {
        "decorated_name": decorated_name.decode("ascii", "replace"),
        "string_matches": string_addrs,
        "complete_object_locators": col_hits,
        "vtables": vtables,
    }


def derive_targets(p: winmem.Process, chunks: list[tuple[int, bytes]]) -> dict[str, Any]:
    targets: dict[str, Any] = {}
    for label, decorated_name in TARGET_RTTI_NAMES.items():
        d = derive_rtti_vtables(p, chunks, decorated_name)
        vtables = list(d["vtables"])
        entries = [
            {
                "decorated_name": d["decorated_name"],
                "string_matches": [hx(v) for v in d["string_matches"]],
                "complete_object_locators": [
                    {
                        "complete_object_locator": hx(hit["complete_object_locator"]),
                        "type_descriptor": hx(hit["type_descriptor"]),
                        "class_descriptor_rva": hx(hit["class_descriptor_rva"]),
                        "offset": hit["offset"],
                        "cd_offset": hit["cd_offset"],
                    }
                    for hit in d["complete_object_locators"]
                ],
                "derived_vtables": [
                    {"addr": hx(v), "rva": hx(v - p.module_base)} for v in d["vtables"]
                ],
            }
        ]
        targets[label] = {
            "decorated_name": decorated_name.decode("ascii", "replace"),
            "entries": entries,
            "vtable_values": vtables,
            "derived_vtable_rvas": [hx(v - p.module_base) for v in vtables],
        }
    return targets


def u64(p: winmem.Process, addr: int) -> int | None:
    try:
        data = p.read(addr, 8)
    except Exception:
        return None
    return struct.unpack("<Q", data)[0] if len(data) == 8 else None


def c_string(p: winmem.Process, addr: int, maxlen: int = 256) -> str | None:
    raw = p.try_read(addr, maxlen)
    if not raw:
        return None
    end = raw.find(b"\0")
    if end < 0:
        return None
    try:
        return raw[:end].decode("ascii")
    except UnicodeDecodeError:
        return raw[:end].decode("latin-1", "replace")


def is_module(p: winmem.Process, value: int | None) -> bool:
    return isinstance(value, int) and p.module_base <= value < p.module_base + p.module_size


def rtti_type_name_for_vtable(p: winmem.Process, vtable: int) -> str | None:
    col = u64(p, vtable - 8)
    if not is_module(p, col):
        return None
    raw = p.try_read(col, 24)
    if len(raw) < 24:
        return None
    sig, _off, _cd, td_ref, _class_ref, self_ref = struct.unpack("<IIIIII", raw[:24])
    if sig not in (0, 1) or self_ref != col - p.module_base:
        return None
    td = p.module_base + td_ref
    if not is_module(p, td):
        return None
    return c_string(p, td + 0x10, maxlen=256)


def classify_old_operation_rvas(
    p: winmem.Process, current_targets: dict[str, Any], old_rvas: dict[str, int]
) -> dict[str, Any]:
    old: dict[str, Any] = {}
    for name, old_rva in old_rvas.items():
        old_vtable = p.module_base + old_rva
        current_vtables = set(current_targets.get(name, {}).get("vtable_values", []))
        expected_name = TARGET_RTTI_NAMES.get(name, b"").decode("ascii", "replace")
        old_type_name = rtti_type_name_for_vtable(p, old_vtable)
        old[name] = {
            "old_rva_16022851": hx(old_rva),
            "old_vtable_addr_current_module": hx(old_vtable),
            "current_expected_vtable_rvas": [
                hx(vtable - p.module_base) for vtable in sorted(current_vtables)
            ],
            "old_rva_is_current_expected_vtable": old_vtable in current_vtables,
            "old_rva_rtti_type_name": old_type_name,
            "expected_rtti_type_name": expected_name,
            "status": "rejected"
            if old_vtable not in current_vtables or old_type_name != expected_name
            else "still_matches",
        }
    return old


def scan_heap_vtables(
    p: winmem.Process,
    current_targets: dict[str, Any],
    old_rvas: dict[str, int],
    max_samples: int,
    max_seconds: float | None,
) -> dict[str, Any]:
    needles: dict[str, list[tuple[int, bytes, str]]] = {}
    for name, target in current_targets.items():
        for vtable in target.get("vtable_values", []):
            needles.setdefault(name, []).append((vtable, struct.pack("<Q", vtable), "current_rtti"))
    for name, rva in old_rvas.items():
        vtable = p.module_base + rva
        needles.setdefault("old_16022851:" + name, []).append(
            (vtable, struct.pack("<Q", vtable), "old_16022851_rva")
        )

    counts = {name: 0 for name in needles}
    samples: dict[str, list[dict[str, Any]]] = {name: [] for name in needles}
    regions_scanned = 0
    bytes_scanned = 0
    read_failures = 0
    started = time.time()
    timed_out = False

    for base, size in p.iter_regions(HEAP_LO, HEAP_HI):
        if max_seconds is not None and time.time() - started > max_seconds:
            timed_out = True
            break
        regions_scanned += 1
        pos = 0
        overlap = 7
        while pos < size:
            if max_seconds is not None and time.time() - started > max_seconds:
                timed_out = True
                break
            want = min(CHUNK, size - pos)
            data = p.try_read(base + pos, want)
            if not data:
                read_failures += 1
                pos += max(want, 0x1000)
                continue
            bytes_scanned += len(data)
            for name, entries in needles.items():
                for vtable, packed, source in entries:
                    idx = 0
                    while True:
                        hit = data.find(packed, idx)
                        if hit < 0:
                            break
                        addr = base + pos + hit
                        if addr % 8 == 0:
                            counts[name] += 1
                            if len(samples[name]) < max_samples:
                                head = p.try_read(addr, 0x80)
                                first_qwords = []
                                if len(head) >= 0x40:
                                    first_qwords = [
                                        hx(struct.unpack_from("<Q", head, qoff)[0])
                                        for qoff in range(0, 0x40, 8)
                                    ]
                                samples[name].append(
                                    {
                                        "object": hx(addr),
                                        "vtable": hx(vtable),
                                        "vtable_rva": hx(vtable - p.module_base),
                                        "source": source,
                                        "first_qwords_0x00_0x38": first_qwords,
                                    }
                                )
                        idx = hit + 1
            if len(data) < want:
                pos += max(len(data), 0x1000)
            elif want <= overlap:
                pos += want
            else:
                pos += want - overlap
        if timed_out:
            break

    return {
        "regions_scanned": regions_scanned,
        "bytes_scanned": bytes_scanned,
        "read_failures": read_failures,
        "timed_out": timed_out,
        "elapsed_seconds": round(time.time() - started, 3),
        "hit_counts": counts,
        "samples": samples,
    }


def summarize(report: dict[str, Any]) -> dict[str, Any]:
    heap = report.get("heap_vtable_scan", {})
    counts = heap.get("hit_counts", {})
    old_rva_status = report.get("old_16022851_rva_classification", {})
    current_db_hits = counts.get("COperationsDatabase", 0)
    old_hits = {
        name: count for name, count in counts.items() if name.startswith("old_16022851:")
    }
    current_operation_hits = counts.get("COperation", 0)
    timed_out = bool(heap.get("timed_out"))
    heap_status = heap.get("status", "scanned")
    stale_rvas_rejected = bool(old_rva_status) and all(
        row.get("status") == "rejected" for row in old_rva_status.values()
    )
    stale_heap_rejected = bool(old_hits) and all(count == 0 for count in old_hits.values()) and not timed_out

    return {
        "status": (
            "heap_scan_timed_out"
            if timed_out
            else (
                "definition_registry_current_but_operation_detail_unvalidated"
                if current_db_hits or stale_rvas_rejected
                else "operation_route_unvalidated"
            )
        ),
        "current_COperationsDatabase_heap_hits": current_db_hits,
        "current_COperation_heap_hits": current_operation_hits,
        "heap_scan_status": heap_status,
        "heap_scan_timed_out": timed_out,
        "old_16022851_operation_rvas_rejected_by_module_rtti": stale_rvas_rejected,
        "old_16022851_operation_rvas_rejected_by_heap_scan": stale_heap_rejected,
        "old_16022851_hit_counts": old_hits,
        "accepted_for_operation_detail": False,
        "reason": (
            "current module RTTI identifies operation-related definition classes and rejects stale RVAs; "
            "this probe does not find or validate a running COperation detail root"
        ),
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--max-samples", type=int, default=16)
    ap.add_argument("--heap-scan", action="store_true", help="also scan heap vtable hits")
    ap.add_argument(
        "--heap-scan-seconds",
        type=float,
        default=None,
        help="optional heap-scan budget; omitted means full scan",
    )
    ns = ap.parse_args()

    started = time.time()
    p = winmem.open_process("hoi4.exe")
    sha = file_sha256(p.module_path)
    report: dict[str, Any] = {
        "lane": "diplomacy_intel_operatives_live",
        "probe": "probe_870ebf_operations",
        "started_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(started)),
        "process": {
            "pid": p.pid,
            "exe_path": p.module_path,
            "module_base": hx(p.module_base),
            "module_size": p.module_size,
            "sha256": sha,
        },
        "expected_sha256": EXPECTED_SHA256,
        "build_gate_ok": sha == EXPECTED_SHA256,
    }
    if sha != EXPECTED_SHA256:
        report["status"] = "build_gate_failed"
        report["summary"] = {"accepted_for_operation_detail": False}
        ns.out.parent.mkdir(parents=True, exist_ok=True)
        ns.out.write_text(json.dumps(report, indent=2, sort_keys=True), encoding="utf-8")
        return 2

    chunks = read_module_chunks(p)
    report["module_scan"] = {
        "chunks": len(chunks),
        "bytes_read": sum(len(blob) for _, blob in chunks),
    }
    targets = derive_targets(p, chunks)
    report["rtti_targets"] = {
        name: {k: v for k, v in target.items() if k != "vtable_values"}
        for name, target in targets.items()
    }
    report["old_16022851_operation_rvas"] = {
        name: hx(rva) for name, rva in OLD_OPERATION_RVAS_16022851.items()
    }
    report["old_16022851_rva_classification"] = classify_old_operation_rvas(
        p, targets, OLD_OPERATION_RVAS_16022851
    )
    if ns.heap_scan:
        report["heap_vtable_scan"] = scan_heap_vtables(
            p,
            targets,
            OLD_OPERATION_RVAS_16022851,
            max(1, ns.max_samples),
            ns.heap_scan_seconds,
        )
    else:
        report["heap_vtable_scan"] = {
            "status": "skipped",
            "reason": "heap scan is opt-in; module RTTI classification is sufficient to reject stale vtable-RVA routes",
            "hit_counts": {},
            "timed_out": False,
        }
    report["summary"] = summarize(report)
    report["completed_utc"] = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    report["elapsed_seconds"] = round(time.time() - started, 3)

    ns.out.parent.mkdir(parents=True, exist_ok=True)
    ns.out.write_text(json.dumps(report, indent=2, sort_keys=True), encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
