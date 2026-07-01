"""Preflight live diplomacy war-relation records for the DLL collector.

The existing ``memory_reader.diplomacy_live`` reader knows how to locate the
global ``war_relation`` pool. This module converts that cold discovery into a
compact bridge for the in-process collector: scheduled DLL frames revalidate and
read pinned records, while manifests keep only counts and diagnostics.
"""
from __future__ import annotations

from dataclasses import dataclass
import os
from pathlib import Path
import re
import sys
import time
from typing import Any, Callable, Dict, Iterable, List, Optional, Tuple

from . import process_identity as _process_identity


MAX_TAG_DB_ID = 1024
MAX_WAR_RELATION_BRIDGE_ROWS = 10000
MIN_USABLE_WAR_RELATION_ROWS = 1
_COUNTRY_TAG_RE = re.compile(r"^[A-Z][A-Z0-9]{2}$")

_HERE = Path(__file__).resolve().parent
_REPO = _HERE.parent
_MR = _REPO / "memory_reader"
for _p in (str(_REPO), str(_MR)):
    if _p not in sys.path:
        sys.path.insert(0, _p)


@dataclass(frozen=True)
class WarRelationBridgePreflight:
    status: str
    row_count: int
    encoded: Optional[str]
    diagnostics: Dict[str, Any]

    @property
    def usable(self) -> bool:
        return self.status == "resolved" and bool(self.encoded)


def resolve_war_relation_bridge_preflight(
    pid: int,
    *,
    reader_factory: Optional[Callable[[], Any]] = None,
    min_rows: int = MIN_USABLE_WAR_RELATION_ROWS,
) -> WarRelationBridgePreflight:
    """Resolve pinned ``war_relation`` rows for one recorder session.

    This may run the known heap sweep during preflight, but the returned bridge
    lets the DLL hot path avoid heap scanning. The encoded value is address
    bearing and is only for ``collector_control.json``.
    """
    started = time.monotonic()
    reader = None
    reader_pid: Optional[int] = None
    build_ok: Optional[bool] = None
    try:
        import diplomacy_live as dl  # type: ignore

        factory = reader_factory or _default_reader_factory
        reader = factory()
        reader_pid = _coerce_optional_int(getattr(reader, "pid", None))
        identity = _process_identity.identify(reader)
        build_ok = identity.build_ok()
        if reader_pid != int(pid):
            return _preflight_status(
                "pid_mismatch",
                [],
                elapsed_s=time.monotonic() - started,
                reader_pid=reader_pid,
                requested_pid=pid,
                build_ok=build_ok,
            )
        if not build_ok:
            return _preflight_status(
                "build_mismatch",
                [],
                elapsed_s=time.monotonic() - started,
                reader_pid=reader_pid,
                requested_pid=pid,
                build_ok=False,
            )

        ident, identity_source = _resolve_war_relation_identity(dl, reader)
        if ident is None:
            return _preflight_status(
                "identity_unresolved",
                [],
                elapsed_s=time.monotonic() - started,
                reader_pid=reader_pid,
                requested_pid=pid,
                build_ok=True,
                identity_source=identity_source,
            )

        relations, subtype_warning, addr_by_pair = dl.read_war_relations(
            reader,
            ident,
            want_addrs=True,
        )
        subtype_warning_count = sum(int(v) for v in (subtype_warning or {}).values())
        rows = war_relation_bridge_rows_from_addrs(addr_by_pair)
        if len(rows) < int(min_rows):
            status = "subtype_warning" if subtype_warning else "insufficient_rows"
            return _preflight_status(
                status,
                rows,
                elapsed_s=time.monotonic() - started,
                reader_pid=reader_pid,
                requested_pid=pid,
                build_ok=True,
                subtype_warning_count=subtype_warning_count,
                identity_source=identity_source,
            )

        encoded = encode_war_relation_bridge(rows)
        return WarRelationBridgePreflight(
            status="resolved",
            row_count=len(rows),
            encoded=encoded,
            diagnostics=_diagnostics(
                "resolved",
                rows,
                elapsed_s=time.monotonic() - started,
                reader_pid=reader_pid,
                requested_pid=pid,
                build_ok=True,
                bridge_supplied=bool(encoded),
                relation_count=len(relations or []),
                at_war_country_count=len(dl.at_war_sets(relations or [])),
                casualty_country_count=len(dl.country_total_casualties(relations or [])),
                subtype_warning_count=subtype_warning_count,
                identity_source=identity_source,
            ),
        )
    except Exception as exc:
        return _preflight_error(
            "resolve_failed",
            exc,
            elapsed_s=time.monotonic() - started,
            reader_pid=reader_pid,
            requested_pid=pid,
            build_ok=build_ok,
        )
    finally:
        if reader is not None:
            try:
                reader.close()
            except Exception:
                pass


def war_relation_bridge_rows_from_addrs(addr_by_pair: Any) -> List[Dict[str, int]]:
    """Normalize ``(first_tag_db_id, second_tag_db_id) -> record_ptr`` rows."""
    if not addr_by_pair:
        return []
    if isinstance(addr_by_pair, str):
        raw_items: Iterable[Any] = _iter_encoded_war_relation_bridge(addr_by_pair)
    elif isinstance(addr_by_pair, dict):
        raw_items = addr_by_pair.items()
    else:
        raw_items = addr_by_pair

    rows_by_pair: Dict[Tuple[int, int], int] = {}
    seen_ptrs: set[int] = set()
    for item in raw_items:
        first, second, record_ptr = _war_relation_bridge_item(item)
        if not (0 < first <= MAX_TAG_DB_ID and 0 < second <= MAX_TAG_DB_ID):
            raise ValueError("war_relation_bridge tag_db_id out of range")
        if first == second:
            raise ValueError("war_relation_bridge identical tag ids")
        if record_ptr <= 0 or record_ptr % 4:
            raise ValueError("war_relation_bridge pointer must be nonzero and 4-byte aligned")
        key = (first, second)
        if key in rows_by_pair:
            raise ValueError("war_relation_bridge duplicate pair")
        if record_ptr in seen_ptrs:
            raise ValueError("war_relation_bridge duplicate pointer")
        rows_by_pair[key] = record_ptr
        seen_ptrs.add(record_ptr)
        if len(rows_by_pair) > MAX_WAR_RELATION_BRIDGE_ROWS:
            raise ValueError("war_relation_bridge exceeds max rows")

    return [
        {
            "first_tag_db_id": first,
            "second_tag_db_id": second,
            "record_ptr": rows_by_pair[(first, second)],
        }
        for first, second in sorted(rows_by_pair)
    ]


def encode_war_relation_bridge(rows: Any) -> str:
    """Encode ``first:second:0xrecord`` rows for ``collector_control.json``."""
    normalized = war_relation_bridge_rows_from_addrs(rows)
    return ";".join(
        f"{row['first_tag_db_id']}:{row['second_tag_db_id']}:0x{row['record_ptr']:X}"
        for row in normalized
    )


def _default_reader_factory() -> Any:
    import winmem  # type: ignore

    return winmem.Process(winmem.find_pid("hoi4.exe"), "hoi4.exe")


class _TagTableWarIdentity:
    def __init__(self, tbl: Any):
        self.tbl = tbl
        self._map = {
            int(k): str(v)
            for k, v in (tbl.as_map() or {}).items()
            if _valid_country_tag(v)
        }

    @classmethod
    def resolve(cls, dl: Any, reader: Any) -> Optional["_TagTableWarIdentity"]:
        try:
            tbl = dl.tag_table.TagTable.locate(reader)
        except Exception:
            return None
        if tbl is None:
            return None
        ident = cls(tbl)
        if len(ident.valid_ids()) < 40:
            return None
        return ident

    def tag(self, tag_db_id: Any) -> Optional[str]:
        try:
            tid = int(tag_db_id)
        except (TypeError, ValueError):
            return None
        try:
            tag = self.tbl.tag(tid)
        except Exception:
            tag = None
        if _valid_country_tag(tag):
            return str(tag)
        return self._map.get(tid)

    def valid_ids(self) -> set[int]:
        return set(self._map)


def _resolve_war_relation_identity(dl: Any, reader: Any) -> Tuple[Any, str]:
    try:
        ident = dl.CountryIdentity.resolve(reader)
    except Exception:
        ident = None
    if ident is not None:
        return ident, "country_identity"
    ident = _TagTableWarIdentity.resolve(dl, reader)
    if ident is not None:
        return ident, "tag_table"
    return None, "unresolved"


def _valid_country_tag(value: Any) -> bool:
    return isinstance(value, str) and bool(_COUNTRY_TAG_RE.match(value)) and value != "---"


def _war_relation_bridge_item(item: Any) -> Tuple[int, int, int]:
    if isinstance(item, dict):
        first_raw = item.get("first_tag_db_id", item.get("first"))
        second_raw = item.get("second_tag_db_id", item.get("second"))
        ptr_raw = item.get("record_ptr", item.get("addr", item.get("body_ptr")))
    else:
        try:
            if isinstance(item, (list, tuple)) and len(item) == 3:
                first_raw, second_raw, ptr_raw = item
            else:
                pair_raw, ptr_raw = item
                if isinstance(pair_raw, (list, tuple)) and len(pair_raw) == 2:
                    first_raw, second_raw = pair_raw
                else:
                    first_raw, second_raw, ptr_raw = item
        except Exception as exc:
            raise ValueError("war_relation_bridge row must be pair-like") from exc
    return (
        _bridge_int(first_raw, "first_tag_db_id"),
        _bridge_int(second_raw, "second_tag_db_id"),
        _bridge_int(ptr_raw, "record_ptr"),
    )


def _iter_encoded_war_relation_bridge(value: str) -> Iterable[Tuple[int, int, int]]:
    if not value:
        return []
    out = []
    for raw in value.split(";"):
        if not raw:
            raise ValueError("war_relation_bridge contains empty row")
        parts = raw.split(":")
        if len(parts) != 3:
            raise ValueError("war_relation_bridge row must be first:second:ptr")
        out.append((
            _bridge_int(parts[0], "first_tag_db_id"),
            _bridge_int(parts[1], "second_tag_db_id"),
            _bridge_int(parts[2], "record_ptr"),
        ))
    return out


def _bridge_int(value: Any, name: str) -> int:
    if isinstance(value, bool):
        raise ValueError(f"war_relation_bridge {name} must be int-like")
    if isinstance(value, int):
        return value
    index = getattr(value, "__index__", None)
    if index is not None:
        try:
            return int(index())
        except Exception as exc:
            raise ValueError(f"war_relation_bridge {name} must be int-like") from exc
    if isinstance(value, str):
        value = value.strip()
        if value.lower().startswith("0x"):
            return int(value, 16)
        return int(value, 10)
    raise ValueError(f"war_relation_bridge {name} must be int-like")


def _preflight_status(
    status: str,
    rows: Iterable[Dict[str, int]],
    *,
    elapsed_s: float,
    reader_pid: Optional[int],
    requested_pid: int,
    build_ok: Optional[bool],
    bridge_supplied: bool = False,
    subtype_warning_count: int = 0,
    identity_source: Optional[str] = None,
) -> WarRelationBridgePreflight:
    rows = list(rows)
    return WarRelationBridgePreflight(
        status=status,
        row_count=len(rows),
        encoded=None,
        diagnostics=_diagnostics(
            status,
            rows,
            elapsed_s=elapsed_s,
            reader_pid=reader_pid,
            requested_pid=requested_pid,
            build_ok=build_ok,
            bridge_supplied=bridge_supplied,
            subtype_warning_count=subtype_warning_count,
            identity_source=identity_source,
        ),
    )


def _preflight_error(
    status: str,
    exc: BaseException,
    *,
    elapsed_s: float,
    reader_pid: Optional[int],
    requested_pid: int,
    build_ok: Optional[bool],
) -> WarRelationBridgePreflight:
    diagnostics = _diagnostics(
        status,
        [],
        elapsed_s=elapsed_s,
        reader_pid=reader_pid,
        requested_pid=requested_pid,
        build_ok=build_ok,
        bridge_supplied=False,
    )
    diagnostics["error"] = _safe_error_message(exc)
    return WarRelationBridgePreflight(
        status=status,
        row_count=0,
        encoded=None,
        diagnostics=diagnostics,
    )


def _diagnostics(
    status: str,
    rows: Iterable[Dict[str, int]],
    *,
    elapsed_s: float,
    reader_pid: Optional[int],
    requested_pid: int,
    build_ok: Optional[bool],
    bridge_supplied: bool,
    relation_count: Optional[int] = None,
    at_war_country_count: Optional[int] = None,
    casualty_country_count: Optional[int] = None,
    subtype_warning_count: int = 0,
    identity_source: Optional[str] = None,
) -> Dict[str, Any]:
    rows = list(rows)
    return {
        "status": status,
        "row_count": len(rows),
        "relation_count": relation_count if relation_count is not None else len(rows),
        "at_war_country_count": at_war_country_count,
        "casualty_country_count": casualty_country_count,
        "subtype_warning_count": int(subtype_warning_count),
        "advisory_subtype_warning": int(subtype_warning_count) > 0 and bool(bridge_supplied),
        "identity_source": identity_source,
        "build_ok": bool(build_ok) if build_ok is not None else None,
        "pid_match": reader_pid == int(requested_pid),
        "requested_pid": int(requested_pid),
        "reader_pid": reader_pid,
        "elapsed_s": float(elapsed_s),
        "bridge_supplied": bool(bridge_supplied),
        "source": "diplomacy_live.read_war_relations",
        "runtime_source": "hoi4.exe",
        "address_manifest_policy": "omitted",
        "discovery_mode": "preflight_war_relation_heap_scan",
        "preflight_heap_scan": True,
        "hot_loop_heap_scan": False,
    }


def _coerce_optional_int(value: Any) -> Optional[int]:
    if isinstance(value, bool) or value is None:
        return None
    try:
        return int(value)
    except (TypeError, ValueError):
        return None


def _safe_error_message(exc: BaseException) -> str:
    msg = str(exc)
    msg = re.sub(r"0x[0-9A-Fa-f]+", "0x<redacted>", msg)
    msg = re.sub(r"[A-Za-z]:\\[^;\n\r]+", "<path>", msg)
    msg = re.sub(r"/[^;\n\r ]+", "<path>", msg)
    for value in (os.environ.get("USERNAME"), os.path.expanduser("~")):
        if value:
            msg = msg.replace(value, "<redacted>")
    return msg[:240]


__all__ = [
    "WarRelationBridgePreflight",
    "encode_war_relation_bridge",
    "resolve_war_relation_bridge_preflight",
    "war_relation_bridge_rows_from_addrs",
]
