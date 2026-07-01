#!/usr/bin/env python3
"""Bounded current-build war-shape probe for diplomacy_intel_operatives_live.

This is a lane-local read-only probe. It does not use the shared collector or
recorder entrypoints. The probe starts from the already validated 870ebf
country/tag roots, then scans the current heap for the old 16022851
``war_relation`` row shape:

  +0x00 u32 first tag-db-id
  +0x04 u32 second tag-db-id
  +0x08 u64 date/type pointer
  +0x40 i32 first casualties
  +0x48 i32 second casualties
  +0x68/+0x6C echo of first/second tag-db-id

Without a same-tick active-war fixture, absence of rows is reported as a
fixture blocker rather than a production root rejection for every possible war
route.
"""

from __future__ import annotations

import argparse
import bisect
import json
import struct
import sys
import time
from pathlib import Path
from typing import Any, Iterable


LANE_DIR = Path(__file__).resolve().parent
if str(LANE_DIR) not in sys.path:
    sys.path.insert(0, str(LANE_DIR))

import probe_870ebf_diplomacy as dip  # type: ignore
import winmem  # type: ignore

try:
    import numpy as np  # type: ignore
except Exception:  # pragma: no cover - numpy exists in the production dev env
    np = None


PAGE = 0x1000
CHUNK = 64 << 20
OVERLAP = 0x80
OLD_WAR_ROUTE = "old_16022851_pair_echo_casualty_shape"
DATE_RTTI_NAMES = {
    "CGameDate": b".?AVCGameDate@@",
    "CGregorianDate": b".?AVCGregorianDate@@",
}


def candidate_offsets(blob: bytes, max_tag_id: int, valid: bytearray) -> Iterable[tuple[int, int, int]]:
    n_words = len(blob) // 4
    if n_words < 2:
        return []
    if np is not None:
        words = np.frombuffer(blob[: n_words * 4], dtype="<u4")
        first = words[:-1]
        second = words[1:]
        mask = (
            (first >= 1)
            & (first <= max_tag_id)
            & (second >= 1)
            & (second <= max_tag_id)
            & (first != second)
        )
        indexes = np.nonzero(mask)[0]

        def _iter() -> Iterable[tuple[int, int, int]]:
            for idx in indexes:
                first_id = int(first[idx])
                second_id = int(second[idx])
                if valid[first_id] and valid[second_id]:
                    yield int(idx) * 4, first_id, second_id

        return _iter()

    def _slow() -> Iterable[tuple[int, int, int]]:
        for idx in range(n_words - 1):
            off = idx * 4
            first_id = struct.unpack_from("<I", blob, off)[0]
            second_id = struct.unpack_from("<I", blob, off + 4)[0]
            if (
                1 <= first_id <= max_tag_id
                and 1 <= second_id <= max_tag_id
                and first_id != second_id
                and valid[first_id]
                and valid[second_id]
            ):
                yield off, first_id, second_id

    return _slow()


def i32_at(blob: bytes, off: int) -> int:
    return struct.unpack_from("<i", blob, off)[0]


def u32_at(blob: bytes, off: int) -> int:
    return struct.unpack_from("<I", blob, off)[0]


def u64_at(blob: bytes, off: int) -> int:
    return struct.unpack_from("<Q", blob, off)[0]


def add_count(counts: dict[str, int], key: str) -> None:
    counts[key] = counts.get(key, 0) + 1


def derive_date_rtti(p: winmem.Process) -> dict[str, Any]:
    chunks = dip.read_module_chunks(p)
    classes: dict[str, Any] = {}
    vtables: dict[int, str] = {}
    for name, decorated in DATE_RTTI_NAMES.items():
        derived = dip.derive_rtti_vtables(p, chunks, decorated)
        values = list(derived.get("vtables") or [])
        for value in values:
            vtables[value] = name
        classes[name] = {
            "decorated_name": decorated.decode("ascii", "replace"),
            "string_matches": [dip.hx(value) for value in derived.get("string_matches", [])],
            "complete_object_locators": [
                {
                    "complete_object_locator": dip.hx(hit["complete_object_locator"]),
                    "type_descriptor": dip.hx(hit["type_descriptor"]),
                    "class_descriptor_rva": dip.hx(hit["class_descriptor_rva"]),
                    "offset": hit["offset"],
                    "cd_offset": hit["cd_offset"],
                }
                for hit in derived.get("complete_object_locators", [])
            ],
            "derived_vtables": [
                {"addr": dip.hx(value), "rva": dip.hx(value - p.module_base)}
                for value in values
            ],
        }
    return {
        "classes": classes,
        "module_chunk_count": len(chunks),
        "module_bytes_read": sum(len(blob) for _, blob in chunks),
        "vtable_names": {str(value): name for value, name in vtables.items()},
    }


def classify_typeptr(
    p: winmem.Process, typeptr: int, current_date_vtables: dict[int, str]
) -> tuple[str, str | None]:
    if typeptr == 0:
        return "null", None
    current_date = current_date_vtables.get(typeptr)
    if current_date:
        return "current_%s" % current_date, "0x%X" % (typeptr - p.module_base)
    if typeptr == p.module_base + dip.WR_GAME_DATE_VTABLE_RVA:
        return "old_game_date_typeptr", "0x%X" % dip.WR_GAME_DATE_VTABLE_RVA
    if typeptr == p.module_base + dip.WR_GREGORIAN_DATE_VTABLE_RVA:
        return "old_gregorian_date_typeptr", "0x%X" % dip.WR_GREGORIAN_DATE_VTABLE_RVA
    if p.module_base <= typeptr < p.module_base + p.module_size:
        return "foreign_module_ptr", "0x%X" % (typeptr - p.module_base)
    if dip.is_heap(typeptr):
        return "heap_ptr", None
    return "other", None


def scan_old_war_shape(
    p: winmem.Process,
    tags: dict[int, str],
    current_date_vtables: dict[int, str],
    *,
    max_samples: int,
    max_seconds: float | None,
) -> dict[str, Any]:
    valid_ids = sorted(tags)
    max_tag_id = max(valid_ids) if valid_ids else 0
    valid = bytearray(max_tag_id + 1)
    for tag_id in valid_ids:
        valid[tag_id] = 1

    started = time.monotonic()
    timed_out = False
    regions_scanned = 0
    chunks_scanned = 0
    read_failures = 0
    bytes_scanned = 0
    tag_pair_candidates = 0
    echo_matches = 0
    plausible_old_shape_rows: dict[int, dict[str, Any]] = {}
    current_date_rows: dict[int, dict[str, Any]] = {}
    old_date_typeptr_rows = 0
    current_date_typeptr_rows = 0
    typeptr_class_counts: dict[str, int] = {}
    module_typeptr_rva_counts: dict[str, int] = {}
    rejected_bad_casualties = 0

    for region_base, region_size in p.iter_regions(dip.HEAP_LO, dip.HEAP_HI, want_exec=False):
        regions_scanned += 1
        pos = 0
        region_size = int(region_size)
        while pos < region_size:
            if max_seconds is not None and time.monotonic() - started >= max_seconds:
                timed_out = True
                break
            want = min(CHUNK, region_size - pos)
            base = int(region_base) + pos
            blob = p.try_read(base, want)
            if not blob:
                read_failures += 1
                pos += max(want, PAGE)
                continue
            chunks_scanned += 1
            bytes_scanned += len(blob)
            for off, first_id, second_id in candidate_offsets(blob, max_tag_id, valid):
                tag_pair_candidates += 1
                if off + dip.WR_ECHO + 8 > len(blob) or off + dip.WR_SECOND_CAS + 4 > len(blob):
                    continue
                if u32_at(blob, off + dip.WR_ECHO) != first_id:
                    continue
                if u32_at(blob, off + dip.WR_ECHO + 4) != second_id:
                    continue
                echo_matches += 1
                first_cas = i32_at(blob, off + dip.WR_FIRST_CAS)
                second_cas = i32_at(blob, off + dip.WR_SECOND_CAS)
                if not (
                    0 <= first_cas <= dip.WR_MAX_CASUALTIES
                    and 0 <= second_cas <= dip.WR_MAX_CASUALTIES
                ):
                    rejected_bad_casualties += 1
                    continue
                addr = base + off
                if addr in plausible_old_shape_rows:
                    continue
                typeptr = u64_at(blob, off + dip.WR_TYPEPTR)
                kind, rva = classify_typeptr(p, typeptr, current_date_vtables)
                add_count(typeptr_class_counts, kind)
                if rva:
                    add_count(module_typeptr_rva_counts, rva)
                if kind in ("old_game_date_typeptr", "old_gregorian_date_typeptr"):
                    old_date_typeptr_rows += 1
                row = {
                    "row_ptr": "0x%X" % addr,
                    "first_id": first_id,
                    "first_tag": tags.get(first_id),
                    "second_id": second_id,
                    "second_tag": tags.get(second_id),
                    "echo_first": first_id,
                    "echo_second": second_id,
                    "first_casualties_i32": first_cas,
                    "second_casualties_i32": second_cas,
                    "typeptr": "0x%X" % typeptr,
                    "typeptr_class": kind,
                    "typeptr_rva": rva,
                }
                plausible_old_shape_rows[addr] = row
                if kind.startswith("current_C"):
                    current_date_typeptr_rows += 1
                    current_date_rows[addr] = row
            blen = len(blob)
            if pos + blen >= region_size or blen < want:
                pos += max(blen, PAGE)
            else:
                pos += max(blen - OVERLAP, PAGE)
        if timed_out:
            break

    samples = list(plausible_old_shape_rows.values())[:max_samples]
    current_samples = list(current_date_rows.values())[:max_samples]
    current_pairs = {(row["first_id"], row["second_id"]) for row in current_date_rows.values()}
    status = "timed_out"
    if not timed_out:
        status = "old_shape_observed" if plausible_old_shape_rows else "old_shape_not_observed"
    return {
        "candidate": OLD_WAR_ROUTE,
        "old_shape_offsets": {
            "first_tag_id": "0x%X" % dip.WR_FIRST_TAGID,
            "second_tag_id": "0x%X" % dip.WR_SECOND_TAGID,
            "type_ptr": "0x%X" % dip.WR_TYPEPTR,
            "first_casualties_i32": "0x%X" % dip.WR_FIRST_CAS,
            "second_casualties_i32": "0x%X" % dip.WR_SECOND_CAS,
            "echo_first_second": "0x%X/0x%X" % (dip.WR_ECHO, dip.WR_ECHO + 4),
        },
        "old_date_typeptr_vtable_rvas": {
            "game_date": "0x%X" % dip.WR_GAME_DATE_VTABLE_RVA,
            "gregorian_date": "0x%X" % dip.WR_GREGORIAN_DATE_VTABLE_RVA,
        },
        "regions_scanned": regions_scanned,
        "chunks_scanned": chunks_scanned,
        "bytes_scanned": bytes_scanned,
        "read_failures": read_failures,
        "tag_pair_candidates": tag_pair_candidates,
        "echo_matches": echo_matches,
        "rejected_bad_casualties": rejected_bad_casualties,
        "plausible_old_shape_rows": len(plausible_old_shape_rows),
        "current_date_typeptr_rows": current_date_typeptr_rows,
        "current_date_unique_pairs": len(current_pairs),
        "current_date_pair_samples": [
            {
                "first_id": first,
                "first_tag": tags.get(first),
                "second_id": second,
                "second_tag": tags.get(second),
            }
            for first, second in sorted(current_pairs)[:max_samples]
        ],
        "old_date_typeptr_rows": old_date_typeptr_rows,
        "typeptr_class_counts": typeptr_class_counts,
        "module_typeptr_rva_counts": module_typeptr_rva_counts,
        "samples": samples,
        "current_date_samples": current_samples,
        "current_date_rows": list(current_date_rows.values()),
        "timed_out": timed_out,
        "elapsed_seconds": round(time.monotonic() - started, 3),
        "status": status,
    }


def join_current_date_rows_to_relations(
    p: winmem.Process,
    rows: list[dict[str, Any]],
    relation_entries: list[tuple[int, int, int]],
    tags: dict[int, str],
    *,
    max_samples: int,
) -> dict[str, Any]:
    row_by_addr: dict[int, dict[str, Any]] = {}
    for row in rows:
        raw_addr = row.get("row_ptr")
        if isinstance(raw_addr, str):
            row_by_addr[int(raw_addr, 16)] = row
    row_ptrs = set(row_by_addr)
    relation_ptrs = sorted(rel for _src, _target, rel in relation_entries)
    relation_by_ptr = {rel: (src, target, rel) for src, target, rel in relation_entries}

    pointer_matches: dict[int, dict[str, Any]] = {}
    pointer_samples: list[dict[str, Any]] = []
    scanned_relation_entries = 0
    unreadable_relation_entries = 0
    for src, target, rel in relation_entries:
        blob = p.try_read(rel, dip.REL_SCAN_BYTES)
        if len(blob) < dip.REL_SCAN_BYTES:
            unreadable_relation_entries += 1
            continue
        scanned_relation_entries += 1
        for off in range(0, len(blob) - 7, 8):
            ptr = struct.unpack_from("<Q", blob, off)[0]
            row = row_by_addr.get(ptr)
            if row is None:
                continue
            pointer_matches[ptr] = {
                "src_id": src,
                "src_tag": tags.get(src),
                "target_id": target,
                "target_tag": tags.get(target),
                "relation_ptr": "0x%X" % rel,
                "relation_offset": "0x%X" % off,
                "row_ptr": row.get("row_ptr"),
                "row_first_tag": row.get("first_tag"),
                "row_second_tag": row.get("second_tag"),
                "typeptr_rva": row.get("typeptr_rva"),
                "typeptr_class": row.get("typeptr_class"),
            }
            if len(pointer_samples) < max_samples:
                pointer_samples.append(pointer_matches[ptr])

    range_matches: dict[int, dict[str, Any]] = {}
    range_samples: list[dict[str, Any]] = []
    for row_addr, row in row_by_addr.items():
        idx = bisect.bisect_right(relation_ptrs, row_addr) - 1
        if idx < 0:
            continue
        rel = relation_ptrs[idx]
        delta = row_addr - rel
        if not (0 <= delta < dip.REL_SCAN_BYTES):
            continue
        src, target, _rel = relation_by_ptr[rel]
        range_matches[row_addr] = {
            "src_id": src,
            "src_tag": tags.get(src),
            "target_id": target,
            "target_tag": tags.get(target),
            "relation_ptr": "0x%X" % rel,
            "row_ptr": row.get("row_ptr"),
            "row_offset_from_relation": "0x%X" % delta,
            "row_first_tag": row.get("first_tag"),
            "row_second_tag": row.get("second_tag"),
            "typeptr_rva": row.get("typeptr_rva"),
            "typeptr_class": row.get("typeptr_class"),
        }
        if len(range_samples) < max_samples:
            range_samples.append(range_matches[row_addr])

    return {
        "relation_entries_total": len(relation_entries),
        "relation_entries_scanned_for_pointers": scanned_relation_entries,
        "relation_entries_unreadable": unreadable_relation_entries,
        "current_date_rows": len(row_by_addr),
        "pointer_join_rows": len(pointer_matches),
        "range_join_rows": len(range_matches),
        "pointer_samples": pointer_samples,
        "range_samples": range_samples,
        "status": "joined" if pointer_matches or range_matches else "not_joined",
    }


def warm_validate_current_date_rows(
    p: winmem.Process,
    rows: list[dict[str, Any]],
    current_date_vtables: dict[int, str],
    tags: dict[int, str],
    *,
    max_samples: int,
    delay_seconds: float,
) -> dict[str, Any]:
    if delay_seconds > 0:
        time.sleep(delay_seconds)

    checked = 0
    valid = 0
    invalid_by_reason: dict[str, int] = {}
    samples: list[dict[str, Any]] = []
    for row in rows:
        raw_addr = row.get("row_ptr")
        if not isinstance(raw_addr, str):
            add_count(invalid_by_reason, "missing_row_ptr")
            continue
        addr = int(raw_addr, 16)
        checked += 1
        blob = p.try_read(addr, dip.WR_ECHO + 8)
        if len(blob) < dip.WR_ECHO + 8:
            add_count(invalid_by_reason, "unreadable")
            continue
        first_id = u32_at(blob, dip.WR_FIRST_TAGID)
        second_id = u32_at(blob, dip.WR_SECOND_TAGID)
        if first_id != row.get("first_id") or second_id != row.get("second_id"):
            add_count(invalid_by_reason, "tag_pair_changed")
            continue
        if u32_at(blob, dip.WR_ECHO) != first_id or u32_at(blob, dip.WR_ECHO + 4) != second_id:
            add_count(invalid_by_reason, "echo_mismatch")
            continue
        typeptr = u64_at(blob, dip.WR_TYPEPTR)
        type_name = current_date_vtables.get(typeptr)
        if not type_name:
            add_count(invalid_by_reason, "date_vtable_changed")
            continue
        first_cas = i32_at(blob, dip.WR_FIRST_CAS)
        second_cas = i32_at(blob, dip.WR_SECOND_CAS)
        if not (
            0 <= first_cas <= dip.WR_MAX_CASUALTIES
            and 0 <= second_cas <= dip.WR_MAX_CASUALTIES
        ):
            add_count(invalid_by_reason, "casualty_bounds")
            continue
        valid += 1
        if len(samples) < max_samples:
            samples.append(
                {
                    "row_ptr": raw_addr,
                    "first_id": first_id,
                    "first_tag": tags.get(first_id),
                    "second_id": second_id,
                    "second_tag": tags.get(second_id),
                    "typeptr_rva": "0x%X" % (typeptr - p.module_base),
                    "typeptr_class": type_name,
                    "first_casualties_i32": first_cas,
                    "second_casualties_i32": second_cas,
                }
            )
    return {
        "checked_rows": checked,
        "valid_rows": valid,
        "invalid_rows": checked - valid,
        "invalid_by_reason": invalid_by_reason,
        "samples": samples,
        "delay_seconds": delay_seconds,
        "status": "validated" if checked > 0 and checked == valid else "failed",
    }


def load_fixture(path: str | None) -> dict[str, Any] | None:
    if not path:
        return None
    with open(path, "r", encoding="utf-8") as f:
        return json.load(f)


def summarize_fixture(fixture: dict[str, Any] | None, process_sha: str | None) -> dict[str, Any]:
    required = [
        "build_sha256",
        "same_tick_utc",
        "war_name_or_id",
        "attacker_tags",
        "defender_tags",
        "ui_or_save_at_war_evidence",
    ]
    if not fixture:
        return {
            "supplied": False,
            "section": "war_root",
            "required_fields": required,
            "missing_fields": required,
            "usable": False,
        }
    section = fixture.get("war_root") if isinstance(fixture, dict) else None
    if not isinstance(section, dict):
        return {
            "supplied": True,
            "section": "war_root",
            "required_fields": required,
            "missing_fields": required,
            "usable": False,
            "reason": "war_root section missing or not an object",
        }
    missing = [field for field in required if not section.get(field)]
    if section.get("build_sha256") != process_sha:
        missing.append("matching_build_sha256")
    return {
        "supplied": True,
        "section": "war_root",
        "required_fields": required,
        "missing_fields": missing,
        "usable": not missing,
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True)
    ap.add_argument("--fixture", default=None)
    ap.add_argument("--max-samples", type=int, default=16)
    ap.add_argument("--max-seconds", type=float, default=None)
    ap.add_argument("--warm-delay-ms", type=int, default=250)
    ns = ap.parse_args()

    out_path = Path(ns.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)

    p = winmem.open_process("hoi4.exe")
    if p is None:
        raise SystemExit("hoi4.exe not running")

    with p:
        started = time.time()
        info = dip.process_info(p)
        report: dict[str, Any] = {
            "schema_version": 1,
            "lane": "diplomacy_intel_operatives_live",
            "probe": "probe_870ebf_war_shape",
            "expected_sha256": dip.EXPECTED_SHA256,
            "started_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(started)),
            "process": info,
            "build_gate_ok": info.get("sha256") == dip.EXPECTED_SHA256,
        }
        if not report["build_gate_ok"]:
            report["status"] = "build_mismatch"
            report["completed_utc"] = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
            out_path.write_text(json.dumps(report, indent=2, sort_keys=True), encoding="utf-8")
            return 2

        roots = dip.read_roots(p)
        tags = dip.load_tags(p, roots)
        current_country = dip.validate_country_vtable(
            p, roots, tags, dip.COUNTRY_VTABLE_RVA_870EBF
        )
        country_count = int(current_country.get("valid_country_count") or 0)
        current_countries = {
            int(tag_id): int(ptr) for tag_id, ptr in current_country.get("countries", {}).items()
        }
        current_country.pop("countries", None)
        report["roots"] = roots
        report["tag_table"] = {
            "tag_count_read": len(tags),
            "max_tag_id": max(tags) if tags else None,
            "samples": [{"tag_id": k, "tag": v} for k, v in list(sorted(tags.items()))[:16]],
        }
        report["country_vtable"] = current_country
        if country_count < 40:
            report["status"] = "country_roots_unvalidated"
            report["completed_utc"] = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
            out_path.write_text(json.dumps(report, indent=2, sort_keys=True), encoding="utf-8")
            return 2

        relation_entries, relation_stats = dip.collect_relation_entries(
            p, current_countries, sorted(current_countries)
        )
        report["relations"] = relation_stats
        date_rtti = derive_date_rtti(p)
        report["date_rtti"] = {
            "classes": date_rtti["classes"],
            "module_chunk_count": date_rtti["module_chunk_count"],
            "module_bytes_read": date_rtti["module_bytes_read"],
        }
        current_date_vtables = {
            int(value): name for value, name in date_rtti.get("vtable_names", {}).items()
        }
        fixture = load_fixture(ns.fixture)
        report["fixture"] = summarize_fixture(fixture, info.get("sha256"))
        report["scan"] = scan_old_war_shape(
            p,
            tags,
            current_date_vtables,
            max_samples=max(0, ns.max_samples),
            max_seconds=ns.max_seconds,
        )
        report["relation_join"] = join_current_date_rows_to_relations(
            p,
            report["scan"].get("current_date_rows") or [],
            relation_entries,
            tags,
            max_samples=max(0, ns.max_samples),
        )
        report["warm_validation"] = warm_validate_current_date_rows(
            p,
            report["scan"].get("current_date_rows") or [],
            current_date_vtables,
            tags,
            max_samples=max(0, ns.max_samples),
            delay_seconds=max(0, ns.warm_delay_ms) / 1000.0,
        )

        scan = report["scan"]
        relation_join = report["relation_join"]
        warm_validation = report["warm_validation"]
        fixture_usable = bool(report["fixture"].get("usable"))
        accepted = bool(
            scan.get("current_date_typeptr_rows", 0) > 0
            and scan.get("old_date_typeptr_rows", 0) == 0
            and warm_validation.get("status") == "validated"
        )
        report["accepted_for_production_war_root"] = accepted
        if accepted:
            report["status"] = "validated_current_date_war_shape"
        elif scan.get("timed_out"):
            report["status"] = "scan_timed_out"
        elif scan.get("plausible_old_shape_rows", 0) == 0:
            report["status"] = "old_shape_not_observed_fixture_missing"
        else:
            report["status"] = "old_shape_observed_fixture_missing"
        report["summary"] = {
            "old_16022851_pair_echo_casualty_shape_observed": scan.get(
                "plausible_old_shape_rows", 0
            )
            > 0,
            "old_16022851_date_typeptr_rows": scan.get("old_date_typeptr_rows", 0),
            "current_date_typeptr_rows": scan.get("current_date_typeptr_rows", 0),
            "current_date_unique_pairs": scan.get("current_date_unique_pairs", 0),
            "relation_pointer_join_rows": relation_join.get("pointer_join_rows", 0),
            "relation_range_join_rows": relation_join.get("range_join_rows", 0),
            "warm_checked_rows": warm_validation.get("checked_rows", 0),
            "warm_valid_rows": warm_validation.get("valid_rows", 0),
            "warm_invalid_rows": warm_validation.get("invalid_rows", 0),
            "active_war_fixture_usable": fixture_usable,
            "accepted_for_production_war_root": accepted,
            "reason": (
                "current date RTTI plus warm row-address validation is accepted as "
                "structural war-root evidence; fixture remains useful for lifecycle/detail field naming"
            ),
        }
        report["completed_utc"] = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
        report["elapsed_seconds"] = round(time.time() - started, 3)
        out_path.write_text(json.dumps(report, indent=2, sort_keys=True), encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
