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
REC = 0x120
OFF_NAMEPTR = 0x08
OFF_INDEX = 0x5D
OBJ_TAG_DB_ID = 0x08


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
) -> CountryObjectBridgePreflight:
    """Resolve live country-object rows for one recorder session.

    No saves are read. The returned diagnostics intentionally omit raw pointers;
    the encoded string is ephemeral and should only be written to the temporary
    ``collector_control.json`` next to the DLL.
    """
    try:
        import winmem  # type: ignore
        import locate  # type: ignore
        from live_layers import country_index_bridge as cib  # type: ignore
        from live_layers import tag_table as tt  # type: ignore
    except Exception as exc:
        return _preflight_error("import_failed", exc)

    try:
        with winmem.Process(int(pid), "hoi4.exe") as proc:
            identity = _process_identity.identify(proc)
            table = tag_table_from_country_database_root(proc, tt)
            tag_table_source = "country_database_root"
            if table is None:
                table = tt.TagTable.locate(proc)
                tag_table_source = "heap_scan"
            if table is None:
                return _preflight_status("tag_table_unavailable", [])
            cache_diag: Dict[str, Any] = {
                "enabled": bool(cache_path),
                "status": "not_requested" if cache_path is None else "miss",
                "tag_table_source": tag_table_source,
            }
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
            bridge = cib.CountryIndexBridge.resolve(proc, tbl=table, scan_range=scan_range)
            if bridge is None:
                return _preflight_status(
                    "bridge_unavailable",
                    rows if located is not None else [],
                    source="live_country_array",
                    cache=cache_diag,
                )
            rows = country_object_bridge_rows_from_bridge(bridge)
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
    args = parser.parse_args(argv)

    result = resolve_country_object_bridge_preflight(args.pid)
    if args.emit_control_value and result.encoded:
        print(result.encoded)
    else:
        print(json.dumps(result.diagnostics, indent=2, sort_keys=True))
    return 0 if result.usable else 1


if __name__ == "__main__":
    raise SystemExit(main())
