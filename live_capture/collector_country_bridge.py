"""Preflight country-object bridge for the in-process collector.

This module converts the existing save-free ``CountryIndexBridge`` discovery into
the compact control-file contract consumed by the DLL. Discovery may scan memory,
so it is a pre-recording step only; normal collector frames receive bounded
``tag_db_id -> country object`` rows and revalidate them inside the DLL.
"""
from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import sys
from typing import Any, Dict, Iterable, List, Optional

from . import process_identity as _process_identity
from .collector_probe_record import (
    encode_country_object_bridge,
    encode_country_record_bridge,
)


_HERE = Path(__file__).resolve().parent
_REPO = _HERE.parent
_MR = _REPO / "memory_reader"
for _p in (str(_REPO), str(_MR)):
    if _p not in sys.path:
        sys.path.insert(0, _p)

MIN_USABLE_COUNTRY_OBJECT_ROWS = 40
COUNTRY_DATABASE_OFFSET = 0x33063F8
COUNTRY_VTABLE_RVA = 0x27C0E80
REC = 0x120
OFF_NAMEPTR = 0x08
OFF_INDEX = 0x5D
OBJ_TAG_DB_ID = 0x08
OBJ_NAME = 0x10
HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000
PAGE = 0x1000


@dataclass(frozen=True)
class CountryObjectBridgePreflight:
    status: str
    row_count: int
    encoded: Optional[str]
    record_encoded: Optional[str]
    diagnostics: Dict[str, Any]

    @property
    def usable(self) -> bool:
        return self.status == "resolved" and bool(self.encoded)


def resolve_country_object_bridge_preflight(
    pid: int,
    *,
    min_rows: int = MIN_USABLE_COUNTRY_OBJECT_ROWS,
    scan_range: Optional[tuple[int, int]] = None,
    cache_path: Optional[Path] = None,
    object_cache_path: Optional[Path] = None,
    allow_cold_scan: bool = False,
) -> CountryObjectBridgePreflight:
    """Resolve live country-object rows for one recorder session.

    No saves are read. Cache hits and country-database-root rows are bounded;
    heap-wide cold discovery is opt-in because normal speed-five recording
    startup must report a diagnostic quickly instead of scanning for minutes.
    The returned diagnostics intentionally omit raw pointers; the encoded string
    is ephemeral and should only be written to the temporary
    ``collector_control.json`` next to the DLL.
    """
    try:
        import winmem  # type: ignore
        import locate  # type: ignore
        from live_layers import tag_table as tt  # type: ignore
    except Exception as exc:
        return _preflight_error("import_failed", exc)

    try:
        if object_cache_path is None and cache_path is not None:
            object_cache_path = Path(cache_path).with_name("country_objects.json")
        with winmem.Process(int(pid), "hoi4.exe") as proc:
            identity = _process_identity.identify(proc)
            object_cache_diag: Dict[str, Any] = {
                "enabled": bool(object_cache_path),
                "status": "not_requested" if object_cache_path is None else "miss",
            }
            cache_diag: Dict[str, Any] = {
                "enabled": bool(cache_path),
                "status": "not_requested" if cache_path is None else "miss",
                "tag_table_source": "country_database_root",
                "allow_cold_scan": bool(allow_cold_scan),
                "object_cache": object_cache_diag,
            }
            table = tag_table_from_country_database_root(proc, tt)
            tag_table_source = "country_database_root"
            if table is None:
                if not allow_cold_scan:
                    return _preflight_status(
                        "tag_table_unavailable",
                        [],
                        source="country_database_root",
                        cache=cache_diag,
                    )
                table = tt.TagTable.locate(proc)
                tag_table_source = "heap_scan"
                cache_diag["tag_table_source"] = tag_table_source
            if table is None:
                return _preflight_status(
                    "tag_table_unavailable",
                    [],
                    cache=cache_diag,
                )
            cache_diag["tag_table_source"] = tag_table_source
            if cache_path is not None:
                cached = _load_preflight_cache(cache_path, identity)
                if cached is not None:
                    cache_diag["status"] = "loaded"
                    resolved_cached = locate.resolve_cached(
                        proc, int(cached.get("begin_field") or cached.get("rec0"))
                    )
                    if resolved_cached is not None:
                        rec0, count = resolved_cached
                        located = (rec0, cached.get("begin_field") or rec0, count)
                        rows = country_object_bridge_rows_from_country_array(
                            proc, table, located)
                        if len(rows) >= int(min_rows):
                            cache_diag["status"] = "hit"
                            return _encoded_preflight(
                                "resolved",
                                rows,
                                source="live_country_array_cache",
                                cache=cache_diag,
                    )
                    cache_diag["status"] = "stale"
            if object_cache_path is not None:
                object_cached = _load_country_object_preflight_cache(
                    object_cache_path,
                    identity,
                )
                if object_cached is not None:
                    object_cache_diag["status"] = "loaded"
                    rows = country_object_bridge_rows_from_object_cache(
                        proc,
                        table,
                        object_cached,
                        min_rows=int(min_rows),
                    )
                    if len(rows) >= int(min_rows):
                        object_cache_diag["status"] = "hit"
                        return _encoded_preflight(
                            "resolved",
                            rows,
                            source="country_object_cache",
                            cache=cache_diag,
                        )
                    object_cache_diag["status"] = "stale"
            root_rows = country_object_bridge_rows_from_country_database_root(
                proc,
                table,
                min_rows=int(min_rows),
            )
            if len(root_rows) >= int(min_rows):
                if object_cache_path is not None:
                    object_cache_diag["status"] = (
                        "stored" if _write_country_object_preflight_cache(
                            object_cache_path,
                            identity,
                            root_rows,
                        ) else "store_failed"
                    )
                if not allow_cold_scan:
                    return _encoded_preflight(
                        "resolved",
                        root_rows,
                        source="country_database_root",
                        cache=cache_diag,
                    )
                located = locate_country_array_from_object_rows(
                    proc,
                    table,
                    root_rows,
                    min_rows=int(min_rows),
                    scan_range=scan_range,
                )
                if located is not None:
                    record_rows = country_object_bridge_rows_from_country_array(
                        proc,
                        table,
                        located,
                    )
                    if len(record_rows) >= int(min_rows):
                        if cache_path is not None:
                            if _write_preflight_cache(cache_path, identity, located):
                                cache_diag["status"] = "stored"
                            elif cache_diag.get("status") not in {"stale"}:
                                cache_diag["status"] = "store_failed"
                        return _encoded_preflight(
                            "resolved",
                            record_rows,
                            source="country_database_root_record_scan",
                            cache=cache_diag,
                        )
            if not allow_cold_scan:
                return _preflight_status(
                    "cold_scan_disabled",
                    root_rows,
                    source="country_database_root",
                    cache=cache_diag,
                )
            vtable_rows = country_object_bridge_rows_from_vtable_scan(
                proc,
                table,
                min_rows=int(min_rows),
                scan_range=scan_range,
            )
            if len(vtable_rows) >= int(min_rows):
                if object_cache_path is not None:
                    object_cache_diag["status"] = (
                        "stored" if _write_country_object_preflight_cache(
                            object_cache_path,
                            identity,
                            vtable_rows,
                        ) else "store_failed"
                    )
                return _encoded_preflight(
                    "resolved",
                    vtable_rows,
                    source="country_object_vtable_scan",
                    cache=cache_diag,
                )
            located = locate.locate(proc, scan_range=scan_range)
            if located is not None:
                rows = country_object_bridge_rows_from_country_array(
                    proc, table, located)
                if len(rows) >= int(min_rows):
                    if cache_path is not None:
                        if _write_preflight_cache(cache_path, identity, located):
                            cache_diag["status"] = "stored"
                        elif cache_diag.get("status") not in {"stale"}:
                            cache_diag["status"] = "store_failed"
                    return _encoded_preflight(
                        "resolved",
                        rows,
                        source="live_country_array",
                        cache=cache_diag,
                    )
            return _preflight_status(
                "bridge_unavailable",
                rows if located is not None else [],
                source="live_country_array",
                cache=cache_diag,
            )
    except Exception as exc:
        return _preflight_error("resolve_failed", exc)

    if len(rows) < int(min_rows):
        return _preflight_status("insufficient_rows", rows)
    return _encoded_preflight("resolved", rows)


def country_object_bridge_rows_from_country_array(
    proc: Any,
    table: Any,
    located: tuple[int, Any, int],
) -> List[Dict[str, Any]]:
    rec0, _begin_field, count = located
    rows_by_tag_id: Dict[int, Dict[str, Any]] = {}
    for i in range(int(count)):
        rec = int(rec0) + i * REC
        try:
            if proc.u16(rec + OFF_INDEX) != i:
                continue
            object_ptr = int(proc.u64(rec + OFF_NAMEPTR))
            if object_ptr <= 0:
                continue
            tag_db_id = int(proc.u32(object_ptr + OBJ_TAG_DB_ID))
        except Exception:
            continue
        tag = table.tag(tag_db_id)
        if not tag or tag == "---":
            continue
        rows_by_tag_id[tag_db_id] = {
            "tag_db_id": tag_db_id,
            "tag": str(tag),
            "object_ptr": object_ptr,
            "record_ptr": rec,
            "running_index": i,
        }
    return [rows_by_tag_id[k] for k in sorted(rows_by_tag_id)]


def country_object_bridge_rows_from_object_cache(
    proc: Any,
    table: Any,
    cache: Dict[str, Any],
    *,
    min_rows: int = MIN_USABLE_COUNTRY_OBJECT_ROWS,
) -> List[Dict[str, Any]]:
    module_base = int(getattr(proc, "module_base", 0) or 0)
    if module_base <= 0:
        return []
    raw_rows = cache.get("rows")
    if not isinstance(raw_rows, list) or len(raw_rows) < int(min_rows):
        return []
    country_vtable = module_base + COUNTRY_VTABLE_RVA
    rows_by_tag_id: Dict[int, Dict[str, Any]] = {}
    for item in raw_rows:
        if not isinstance(item, dict):
            return []
        try:
            expected_tag_id = int(item["tag_db_id"])
            expected_tag = str(item["tag"])
            object_ptr = int(item["object_ptr"])
        except (KeyError, TypeError, ValueError):
            return []
        if expected_tag_id <= 0 or object_ptr <= 0:
            return []
        row = country_object_bridge_row_from_object(
            proc,
            table,
            object_ptr,
            country_vtable,
        )
        if row is None:
            return []
        if int(row["tag_db_id"]) != expected_tag_id or str(row["tag"]) != expected_tag:
            return []
        if expected_tag_id in rows_by_tag_id:
            return []
        rows_by_tag_id[expected_tag_id] = row
    rows = [rows_by_tag_id[k] for k in sorted(rows_by_tag_id)]
    return rows if len(rows) >= int(min_rows) else []


def tag_table_from_country_database_root(proc: Any, tag_table_module: Any) -> Optional[Any]:
    module_base = int(getattr(proc, "module_base", 0) or 0)
    if module_base <= 0:
        return None
    try:
        country_database = int(proc.u64(module_base + COUNTRY_DATABASE_OFFSET))
        table_base = int(proc.u64(country_database))
        tag_count = int(proc.u32(country_database + 8))
        active_count = int(proc.u32(country_database + 12))
    except Exception:
        return None
    row_count = min(tag_count, active_count)
    if table_base <= 0 or row_count < MIN_USABLE_COUNTRY_OBJECT_ROWS or tag_count > 1024:
        return None
    if active_count <= 0 or active_count > tag_count:
        return None

    tags: List[str] = []
    try:
        for i in range(row_count):
            tag = tag_table_module.TagTable._read_record(
                proc,
                table_base + i * tag_table_module.STRIDE,
            )
            if tag is None:
                return None
            tags.append(tag)
    except Exception:
        return None
    if not tag_table_module.TagTable._is_canonical(tags):
        return None
    return tag_table_module.TagTable(proc, table_base, len(tags), tags)


def country_object_bridge_rows_from_country_database_root(
    proc: Any,
    table: Any,
    *,
    min_rows: int = MIN_USABLE_COUNTRY_OBJECT_ROWS,
    search_span: int = 0x1000,
) -> List[Dict[str, Any]]:
    module_base = int(getattr(proc, "module_base", 0) or 0)
    if module_base <= 0:
        return []
    try:
        country_database = int(proc.u64(module_base + COUNTRY_DATABASE_OFFSET))
    except Exception:
        return []
    if country_database <= 0:
        return []
    country_vtable = module_base + COUNTRY_VTABLE_RVA
    candidates: Dict[int, List[Dict[str, Any]]] = {}
    for off in range(0, max(0, int(search_span)), 8):
        field_addr = country_database + off
        for base in _candidate_array_bases(proc, field_addr):
            if base in candidates:
                continue
            rows = country_object_bridge_rows_from_tag_indexed_array(
                proc,
                table,
                base,
                country_vtable,
                min_rows=int(min_rows),
            )
            if len(rows) >= int(min_rows):
                candidates[base] = rows
    if not candidates:
        return []
    bases = sorted(candidates)
    ref = _tag_object_map(candidates[bases[0]])
    for base in bases[1:]:
        other = _tag_object_map(candidates[base])
        shared = set(ref) & set(other)
        if len(shared) < int(min_rows):
            return []
        if any(ref[tag_db_id] != other[tag_db_id] for tag_db_id in shared):
            return []
    return candidates[bases[0]]


def _candidate_array_bases(proc: Any, field_addr: int) -> Iterable[int]:
    yield int(field_addr)
    try:
        ptr = int(proc.u64(field_addr))
    except Exception:
        return
    if ptr > 0 and ptr % 8 == 0:
        yield ptr


def country_object_bridge_rows_from_tag_indexed_array(
    proc: Any,
    table: Any,
    base: int,
    country_vtable: int,
    *,
    min_rows: int = MIN_USABLE_COUNTRY_OBJECT_ROWS,
) -> List[Dict[str, Any]]:
    if int(base) <= 0 or int(base) % 8:
        return []
    table_count = int(getattr(table, "count", 0) or 0)
    if table_count <= int(min_rows) or table_count > 1024:
        return []
    rows: List[Dict[str, Any]] = []
    seen_objects = set()
    holes = 0
    for tag_db_id in range(1, table_count):
        try:
            object_ptr = int(proc.u64(int(base) + tag_db_id * 8))
        except Exception:
            object_ptr = 0
        if object_ptr <= 0:
            holes += 1
            if holes >= 16 and tag_db_id > int(min_rows):
                break
            continue
        holes = 0
        if object_ptr in seen_objects:
            return []
        row = country_object_bridge_row_from_object(
            proc,
            table,
            object_ptr,
            country_vtable,
        )
        if row is None or int(row["tag_db_id"]) != tag_db_id:
            return []
        seen_objects.add(object_ptr)
        rows.append(row)
    return rows if len(rows) >= int(min_rows) else []


def _tag_object_map(rows: Iterable[Dict[str, Any]]) -> Dict[int, int]:
    return {
        int(row["tag_db_id"]): int(row["object_ptr"])
        for row in rows
        if row.get("tag_db_id") is not None and row.get("object_ptr")
    }


def country_object_bridge_rows_from_vtable_scan(
    proc: Any,
    table: Any,
    *,
    min_rows: int = MIN_USABLE_COUNTRY_OBJECT_ROWS,
    scan_range: Optional[tuple[int, int]] = None,
) -> List[Dict[str, Any]]:
    module_base = int(getattr(proc, "module_base", 0) or 0)
    if module_base <= 0:
        return []
    country_vtable = module_base + COUNTRY_VTABLE_RVA
    rows_by_tag_id: Dict[int, Dict[str, Any]] = {}
    ambiguous_tag_ids = set()
    for object_ptr in _scan_aligned_qword_hits(
        proc,
        country_vtable,
        scan_range=scan_range,
    ):
        row = country_object_bridge_row_from_object(
            proc,
            table,
            object_ptr,
            country_vtable,
        )
        if row is None:
            continue
        tag_db_id = int(row["tag_db_id"])
        old = rows_by_tag_id.get(tag_db_id)
        if old is not None and old.get("object_ptr") != row.get("object_ptr"):
            ambiguous_tag_ids.add(tag_db_id)
            rows_by_tag_id.pop(tag_db_id, None)
            continue
        if tag_db_id not in ambiguous_tag_ids:
            rows_by_tag_id[tag_db_id] = row
    if ambiguous_tag_ids and len(rows_by_tag_id) < int(min_rows):
        return []
    return [rows_by_tag_id[k] for k in sorted(rows_by_tag_id)]


def country_object_bridge_row_from_object(
    proc: Any,
    table: Any,
    object_ptr: int,
    country_vtable: int,
) -> Optional[Dict[str, Any]]:
    if int(object_ptr) <= 0 or int(object_ptr) % 8:
        return None
    try:
        if int(proc.u64(object_ptr)) != int(country_vtable):
            return None
        tag_db_id = int(proc.u32(object_ptr + OBJ_TAG_DB_ID))
    except Exception:
        return None
    tag = table.tag(tag_db_id)
    if not tag or tag == "---":
        return None
    name = _read_stable_msvc_string(proc, object_ptr + OBJ_NAME, max_len=96)
    if not name:
        return None
    return {
        "tag_db_id": tag_db_id,
        "tag": str(tag),
        "object_ptr": int(object_ptr),
    }


def locate_country_array_from_object_rows(
    proc: Any,
    table: Any,
    rows: Iterable[Dict[str, Any]],
    *,
    min_rows: int = MIN_USABLE_COUNTRY_OBJECT_ROWS,
    scan_range: Optional[tuple[int, int]] = None,
    max_anchors: int = 8,
) -> Optional[tuple[int, int, int]]:
    rows = list(rows)
    if len(rows) < int(min_rows):
        return None
    expected_objects = {
        int(row["tag_db_id"]): int(row["object_ptr"])
        for row in rows
        if row.get("tag_db_id") is not None and row.get("object_ptr")
    }
    anchors = _record_anchor_rows(rows)[:max(1, int(max_anchors))]
    seen_rec0 = set()
    for row in anchors:
        object_ptr = int(row["object_ptr"])
        for hit in _scan_aligned_qword_hits(
            proc,
            object_ptr,
            scan_range=scan_range,
        ):
            record_ptr = int(hit) - OFF_NAMEPTR
            try:
                running_index = int(proc.u16(record_ptr + OFF_INDEX))
            except Exception:
                continue
            if not (0 <= running_index <= 1000):
                continue
            rec0 = record_ptr - running_index * REC
            if rec0 in seen_rec0:
                continue
            seen_rec0.add(rec0)
            if not _record_base_has_clean_index_run(proc, rec0):
                continue
            count = _count_records(proc, rec0)
            if not (int(min_rows) <= count <= 1000):
                continue
            record_rows = country_object_bridge_rows_from_country_array(
                proc,
                table,
                (rec0, rec0, count),
            )
            if _record_rows_match_expected_objects(
                record_rows,
                expected_objects,
                min_rows=int(min_rows),
            ):
                return rec0, rec0, count
    return None


def _record_anchor_rows(rows: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
    preferred = {
        "GER": 0, "ENG": 1, "SOV": 2, "USA": 3, "JAP": 4,
        "FRA": 5, "ITA": 6, "SWE": 7, "RAJ": 8,
    }
    return sorted(
        rows,
        key=lambda row: (
            preferred.get(str(row.get("tag")), 100),
            int(row.get("tag_db_id") or 0),
        ),
    )


def _record_rows_match_expected_objects(
    record_rows: Iterable[Dict[str, Any]],
    expected_objects: Dict[int, int],
    *,
    min_rows: int,
) -> bool:
    matched = 0
    for row in record_rows:
        tag_db_id = int(row.get("tag_db_id") or 0)
        object_ptr = int(row.get("object_ptr") or 0)
        if expected_objects.get(tag_db_id) == object_ptr:
            matched += 1
        elif tag_db_id in expected_objects:
            return False
    return matched >= int(min_rows)


def _record_base_has_clean_index_run(proc: Any, rec0: int, depth: int = 24) -> bool:
    try:
        for i in range(depth):
            if int(proc.u16(rec0 + i * REC + OFF_INDEX)) != i:
                return False
    except Exception:
        return False
    return True


def _count_records(proc: Any, rec0: int, cap: int = 1000) -> int:
    n = 0
    try:
        while n < cap and int(proc.u16(rec0 + n * REC + OFF_INDEX)) == n:
            n += 1
    except Exception:
        return n
    return n


def _scan_aligned_qword_hits(
    proc: Any,
    needle: int,
    *,
    scan_range: Optional[tuple[int, int]] = None,
) -> Iterable[int]:
    lo, hi = scan_range if scan_range else (HEAP_LO, HEAP_HI)
    needle_bytes = int(needle).to_bytes(8, "little", signed=False)
    chunk = 64 << 20
    overlap = 16
    for base, size in proc.iter_regions(lo, hi, want_exec=False):
        pos = 0
        while pos < size:
            want = min(chunk, size - pos)
            blob = proc.try_read(base + pos, want)
            if blob:
                start = 0
                while True:
                    i = blob.find(needle_bytes, start)
                    if i < 0:
                        break
                    start = i + 1
                    addr = base + pos + i
                    if addr % 8 == 0:
                        yield addr
                blen = len(blob)
                if pos + blen >= size or blen < want:
                    pos += max(blen, PAGE)
                else:
                    pos += blen - overlap
            else:
                pos += max(want, PAGE)


def _read_stable_msvc_string(proc: Any, addr: int, max_len: int) -> Optional[str]:
    first = _read_msvc_string(proc, addr, max_len)
    if not first:
        return None
    second = _read_msvc_string(proc, addr, max_len)
    if second != first:
        return None
    return first


def _read_msvc_string(proc: Any, addr: int, max_len: int) -> Optional[str]:
    try:
        size = int(proc.u64(addr + 0x10))
        cap = int(proc.u64(addr + 0x18))
    except Exception:
        return None
    if size <= 0 or size > int(max_len) or cap < size or cap > (1 << 20):
        return None
    try:
        chars_addr = int(addr) if cap < 16 else int(proc.u64(addr))
        data = proc.read(chars_addr, size + 1)
    except Exception:
        return None
    if len(data) != size + 1 or data[size] != 0:
        return None
    raw = data[:size]
    if not all(32 <= b < 127 for b in raw):
        return None
    return raw.decode("latin-1")


def country_object_bridge_rows_from_bridge(bridge: Any) -> List[Dict[str, Any]]:
    rows: List[Dict[str, Any]] = []
    for tag_db_id in sorted(int(k) for k in bridge.as_map()):
        tag = bridge.tag(tag_db_id)
        object_ptr = bridge.country_object(tag_db_id)
        if not tag or not object_ptr:
            continue
        rows.append({
            "tag_db_id": int(tag_db_id),
            "tag": str(tag),
            "object_ptr": int(object_ptr),
        })
    return rows


def _encoded_preflight(
    status: str,
    rows: Iterable[Dict[str, Any]],
    *,
    source: str = "live_country_index_bridge",
    cache: Optional[Dict[str, Any]] = None,
) -> CountryObjectBridgePreflight:
    rows = list(rows)
    try:
        encoded = encode_country_object_bridge(rows)
        record_encoded = encode_country_record_bridge(
            [row for row in rows if row.get("record_ptr")]
        )
    except Exception as exc:
        return _preflight_error("encode_failed", exc, rows=rows, source=source)
    return CountryObjectBridgePreflight(
        status=status,
        row_count=len(rows),
        encoded=encoded,
        record_encoded=record_encoded or None,
        diagnostics=_diagnostics(status, rows, source=source, cache=cache),
    )


def _preflight_status(
    status: str,
    rows: Iterable[Dict[str, Any]],
    *,
    source: str = "live_country_index_bridge",
    cache: Optional[Dict[str, Any]] = None,
) -> CountryObjectBridgePreflight:
    rows = list(rows)
    return CountryObjectBridgePreflight(
        status=status,
        row_count=len(rows),
        encoded=None,
        record_encoded=None,
        diagnostics=_diagnostics(status, rows, source=source, cache=cache),
    )


def _preflight_error(
    status: str,
    exc: BaseException,
    *,
    rows: Iterable[Dict[str, Any]] = (),
    source: str = "live_country_index_bridge",
    cache: Optional[Dict[str, Any]] = None,
) -> CountryObjectBridgePreflight:
    rows = list(rows)
    diagnostics = _diagnostics(status, rows, source=source, cache=cache)
    diagnostics["error"] = _safe_error_message(exc)
    return CountryObjectBridgePreflight(
        status=status,
        row_count=len(rows),
        encoded=None,
        record_encoded=None,
        diagnostics=diagnostics,
    )


def _diagnostics(
    status: str,
    rows: Iterable[Dict[str, Any]],
    *,
    source: str,
    cache: Optional[Dict[str, Any]] = None,
) -> Dict[str, Any]:
    rows = list(rows)
    tags = [str(row.get("tag")) for row in rows if row.get("tag")][:8]
    diagnostics = {
        "status": status,
        "row_count": len(rows),
        "tag_sample": tags,
        "source": source,
        "runtime_source": "hoi4.exe",
        "address_manifest_policy": "omitted",
    }
    if cache is not None:
        diagnostics["cache"] = {
            "enabled": bool(cache.get("enabled")),
            "status": str(cache.get("status") or "unknown"),
        }
        if cache.get("tag_table_source"):
            diagnostics["cache"]["tag_table_source"] = str(cache["tag_table_source"])
        if "allow_cold_scan" in cache:
            diagnostics["allow_cold_scan"] = bool(cache.get("allow_cold_scan"))
        object_cache = cache.get("object_cache")
        if isinstance(object_cache, dict):
            diagnostics["object_cache"] = {
                "enabled": bool(object_cache.get("enabled")),
                "status": str(object_cache.get("status") or "unknown"),
            }
    return diagnostics


def _load_preflight_cache(
    cache_path: Path,
    identity: _process_identity.ProcessIdentity,
) -> Optional[Dict[str, Any]]:
    try:
        payload = json.loads(Path(cache_path).read_text(encoding="utf-8"))
    except (OSError, ValueError, TypeError):
        return None
    if not isinstance(payload, dict):
        return None
    if payload.get("type") != "country_array_preflight_cache_v1":
        return None
    if int(payload.get("pid", -1)) != int(identity.pid):
        return None
    cached_ctime = payload.get("create_time_100ns")
    if identity.create_time_100ns is not None and cached_ctime != identity.create_time_100ns:
        return None
    if identity.exe_sha256 and payload.get("exe_sha256") != identity.exe_sha256:
        return None
    if identity.module_base and payload.get("module_base") != identity.module_base:
        return None
    rec0 = payload.get("rec0")
    count = payload.get("count")
    if not isinstance(rec0, int) or not isinstance(count, int):
        return None
    if rec0 <= 0 or count < MIN_USABLE_COUNTRY_OBJECT_ROWS:
        return None
    return payload


def _load_country_object_preflight_cache(
    cache_path: Path,
    identity: _process_identity.ProcessIdentity,
) -> Optional[Dict[str, Any]]:
    try:
        payload = json.loads(Path(cache_path).read_text(encoding="utf-8"))
    except (OSError, ValueError, TypeError):
        return None
    if not isinstance(payload, dict):
        return None
    if payload.get("type") != "country_object_bridge_preflight_cache_v1":
        return None
    if int(payload.get("pid", -1)) != int(identity.pid):
        return None
    cached_ctime = payload.get("create_time_100ns")
    if identity.create_time_100ns is not None and cached_ctime != identity.create_time_100ns:
        return None
    if identity.exe_sha256 and payload.get("exe_sha256") != identity.exe_sha256:
        return None
    if identity.module_base and payload.get("module_base") != identity.module_base:
        return None
    rows = payload.get("rows")
    if not isinstance(rows, list) or len(rows) < MIN_USABLE_COUNTRY_OBJECT_ROWS:
        return None
    return payload


def _write_preflight_cache(
    cache_path: Path,
    identity: _process_identity.ProcessIdentity,
    located: tuple[int, Any, int],
) -> bool:
    rec0, begin_field, count = located
    payload = {
        "type": "country_array_preflight_cache_v1",
        "created_utc": datetime.now(timezone.utc).isoformat(),
        "pid": int(identity.pid),
        "create_time_100ns": identity.create_time_100ns,
        "exe_sha256": identity.exe_sha256,
        "module_base": identity.module_base,
        "rec0": int(rec0),
        "begin_field": int(begin_field or rec0),
        "count": int(count),
    }
    try:
        path = Path(cache_path)
        path.parent.mkdir(parents=True, exist_ok=True)
        tmp = path.with_name(f".{path.name}.tmp")
        tmp.write_text(json.dumps(payload, sort_keys=True) + "\n", encoding="utf-8")
        tmp.replace(path)
        return True
    except OSError:
        return False


def _write_country_object_preflight_cache(
    cache_path: Path,
    identity: _process_identity.ProcessIdentity,
    rows: Iterable[Dict[str, Any]],
) -> bool:
    payload_rows = []
    for row in rows:
        try:
            tag_db_id = int(row["tag_db_id"])
            tag = str(row["tag"])
            object_ptr = int(row["object_ptr"])
        except (KeyError, TypeError, ValueError):
            return False
        if tag_db_id <= 0 or object_ptr <= 0 or not tag:
            return False
        payload_rows.append({
            "tag_db_id": tag_db_id,
            "tag": tag,
            "object_ptr": object_ptr,
        })
    if len(payload_rows) < MIN_USABLE_COUNTRY_OBJECT_ROWS:
        return False
    payload = {
        "type": "country_object_bridge_preflight_cache_v1",
        "created_utc": datetime.now(timezone.utc).isoformat(),
        "pid": int(identity.pid),
        "create_time_100ns": identity.create_time_100ns,
        "exe_sha256": identity.exe_sha256,
        "module_base": identity.module_base,
        "row_count": len(payload_rows),
        "rows": payload_rows,
    }
    try:
        path = Path(cache_path)
        path.parent.mkdir(parents=True, exist_ok=True)
        tmp = path.with_name(f".{path.name}.tmp")
        tmp.write_text(json.dumps(payload, sort_keys=True) + "\n", encoding="utf-8")
        tmp.replace(path)
        return True
    except OSError:
        return False


def _safe_error_message(exc: BaseException) -> str:
    msg = str(exc)
    for value in (os.environ.get("USERNAME"), os.path.expanduser("~")):
        if value:
            msg = msg.replace(value, "<redacted>")
    return msg[:240]


def main(argv: Optional[List[str]] = None) -> int:
    import argparse
    import json

    parser = argparse.ArgumentParser(
        description="Resolve live country-object bridge rows for collector preflight.")
    parser.add_argument("--pid", type=int, required=True)
    parser.add_argument("--emit-control-value", action="store_true",
                        help="Print the encoded tag_db_id:pointer string.")
    parser.add_argument("--allow-cold-scan", action="store_true",
                        help="Allow slow heap-wide cold discovery.")
    args = parser.parse_args(argv)

    result = resolve_country_object_bridge_preflight(
        args.pid,
        allow_cold_scan=args.allow_cold_scan,
    )
    if args.emit_control_value and result.encoded:
        print(result.encoded)
    else:
        print(json.dumps(result.diagnostics, indent=2, sort_keys=True))
    return 0 if result.usable else 1


if __name__ == "__main__":
    raise SystemExit(main())
