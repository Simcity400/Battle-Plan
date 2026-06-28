"""Preflight country-object bridge for the in-process collector.

This module converts the existing save-free ``CountryIndexBridge`` discovery into
the compact control-file contract consumed by the DLL. Discovery may scan memory,
so it is a pre-recording step only; normal collector frames receive bounded
``tag_db_id -> country object`` rows and revalidate them inside the DLL.
"""
from __future__ import annotations

from dataclasses import dataclass
import os
from pathlib import Path
import sys
from typing import Any, Dict, Iterable, List, Optional

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
            table = tt.TagTable.locate(proc)
            if table is None:
                return _preflight_status("tag_table_unavailable", [])
            located = locate.locate(proc, scan_range=scan_range)
            if located is not None:
                rows = country_object_bridge_rows_from_country_array(
                    proc, table, located)
                if len(rows) >= int(min_rows):
                    return _encoded_preflight("resolved", rows, source="live_country_array")
            bridge = cib.CountryIndexBridge.resolve(proc, tbl=table, scan_range=scan_range)
            if bridge is None:
                return _preflight_status(
                    "bridge_unavailable",
                    rows if located is not None else [],
                    source="live_country_array",
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
        diagnostics=_diagnostics(status, rows, source=source),
    )


def _preflight_status(
    status: str,
    rows: Iterable[Dict[str, Any]],
    *,
    source: str = "live_country_index_bridge",
) -> CountryObjectBridgePreflight:
    rows = list(rows)
    return CountryObjectBridgePreflight(
        status=status,
        row_count=len(rows),
        encoded=None,
        record_encoded=None,
        diagnostics=_diagnostics(status, rows, source=source),
    )


def _preflight_error(
    status: str,
    exc: BaseException,
    *,
    rows: Iterable[Dict[str, Any]] = (),
    source: str = "live_country_index_bridge",
) -> CountryObjectBridgePreflight:
    rows = list(rows)
    diagnostics = _diagnostics(status, rows, source=source)
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
) -> Dict[str, Any]:
    rows = list(rows)
    tags = [str(row.get("tag")) for row in rows if row.get("tag")][:8]
    return {
        "status": status,
        "row_count": len(rows),
        "tag_sample": tags,
        "source": source,
        "runtime_source": "hoi4.exe",
        "address_manifest_policy": "omitted",
    }


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
