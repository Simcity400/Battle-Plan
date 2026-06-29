"""Preflight live OOB command roots for the in-process collector.

The legacy Python reader already proves the command hierarchy by locating
CArmy/CArmyGroup/CFleet/CTaskForce/CTheatre plus the live CDivision/CShip sets.
This module turns that locate into a cache-first DLL bridge so the Rust worker
can revalidate and sample the hierarchy without heap scanning in the hot loop.
"""
from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import re
import sys
import time
from typing import Any, Callable, Dict, Iterable, List, Optional, Tuple

from . import process_identity as _process_identity


MIN_USABLE_OOB_COMMAND_ROWS = 1
MAX_OOB_COMMAND_BRIDGE_ROWS = 50000
MAX_OOB_COMMAND_ID = 4_000_000
OOB_COMMAND_KINDS = ("army", "ag", "fleet", "tf", "theatre", "div", "ship")
_KIND_ORDER = {kind: index for index, kind in enumerate(OOB_COMMAND_KINDS)}

_HERE = Path(__file__).resolve().parent
_REPO = _HERE.parent
for _p in (str(_REPO), str(_REPO / "memory_reader")):
    if _p not in sys.path:
        sys.path.insert(0, _p)


@dataclass(frozen=True)
class OobCommandBridgePreflight:
    status: str
    row_count: int
    encoded: Optional[str]
    diagnostics: Dict[str, Any]

    @property
    def usable(self) -> bool:
        return self.status == "resolved" and bool(self.encoded)


def resolve_oob_command_bridge_preflight(
    pid: int,
    *,
    reader_factory: Optional[Callable[[], Any]] = None,
    min_rows: int = MIN_USABLE_OOB_COMMAND_ROWS,
    cache_path: Optional[Path] = None,
    allow_cold_scan: bool = False,
) -> OobCommandBridgePreflight:
    """Resolve live OOB command object rows for one recorder session."""
    started = time.monotonic()
    locate_s: Optional[float] = None
    reader = None
    reader_pid: Optional[int] = None
    build_ok: Optional[bool] = None
    cache_diag: Dict[str, Any] = {
        "enabled": bool(cache_path),
        "status": "not_requested" if cache_path is None else "miss",
        "allow_cold_scan": bool(allow_cold_scan),
    }
    try:
        factory = reader_factory or (lambda: _default_reader_factory(pid))
        reader = factory()
        reader_pid = _coerce_optional_int(getattr(reader, "pid", None))
        try:
            build_ok_raw, _build_value = reader.verify_build()
            build_ok = bool(build_ok_raw)
        except Exception as exc:
            return _preflight_error(
                "build_gate_failed",
                exc,
                elapsed_s=time.monotonic() - started,
                reader_pid=reader_pid,
                requested_pid=pid,
                build_ok=False,
                cache=cache_diag,
            )
        if reader_pid != int(pid):
            return _preflight_status(
                "pid_mismatch",
                [],
                elapsed_s=time.monotonic() - started,
                reader_pid=reader_pid,
                requested_pid=pid,
                build_ok=build_ok,
                cache=cache_diag,
            )
        if not build_ok:
            return _preflight_status(
                "build_mismatch",
                [],
                elapsed_s=time.monotonic() - started,
                reader_pid=reader_pid,
                requested_pid=pid,
                build_ok=False,
                cache=cache_diag,
            )

        identity = _process_identity.identify(reader)
        if cache_path is not None:
            cached = _load_oob_command_cache(Path(cache_path), identity, min_rows=int(min_rows))
            if cached is not None:
                cache_diag["status"] = "loaded"
                cached_rows = _coerce_cached_rows(cached.get("objects"))
                valid_rows = _validated_cached_rows(reader, cached_rows)
                rows = oob_command_bridge_rows_from_objects(valid_rows)
                if len(rows) >= int(min_rows):
                    cache_diag["status"] = "hit"
                    encoded = encode_oob_command_bridge(rows)
                    return OobCommandBridgePreflight(
                        status="resolved",
                        row_count=len(rows),
                        encoded=encoded,
                        diagnostics=_diagnostics(
                            "resolved",
                            rows,
                            elapsed_s=time.monotonic() - started,
                            cold_locate_s=None,
                            reader_pid=reader_pid,
                            requested_pid=pid,
                            build_ok=True,
                            bridge_supplied=bool(encoded),
                            source="oob_command_cache",
                            cache=cache_diag,
                        ),
                    )
                cache_diag["status"] = "stale"

        if not allow_cold_scan:
            return _preflight_status(
                "cold_scan_disabled",
                [],
                elapsed_s=time.monotonic() - started,
                cold_locate_s=None,
                reader_pid=reader_pid,
                requested_pid=pid,
                build_ok=True,
                source="oob_command_cache",
                cache=cache_diag,
            )

        locate_started = time.monotonic()
        objects = reader.enumerate_command_objs()
        locate_s = time.monotonic() - locate_started
        rows = oob_command_bridge_rows_from_objects(objects)
        if len(rows) < int(min_rows):
            return _preflight_status(
                "insufficient_rows",
                rows,
                elapsed_s=time.monotonic() - started,
                cold_locate_s=locate_s,
                reader_pid=reader_pid,
                requested_pid=pid,
                build_ok=True,
                source="oob_command_live.enumerate_command_objs",
                cache=cache_diag,
            )
        if cache_path is not None:
            cache_diag["status"] = (
                "stored" if _write_oob_command_cache(Path(cache_path), identity, rows)
                else "store_failed"
            )
        encoded = encode_oob_command_bridge(rows)
        return OobCommandBridgePreflight(
            status="resolved",
            row_count=len(rows),
            encoded=encoded,
            diagnostics=_diagnostics(
                "resolved",
                rows,
                elapsed_s=time.monotonic() - started,
                cold_locate_s=locate_s,
                reader_pid=reader_pid,
                requested_pid=pid,
                build_ok=True,
                bridge_supplied=bool(encoded),
                source="oob_command_live.enumerate_command_objs",
                cache=cache_diag,
            ),
        )
    except Exception as exc:
        return _preflight_error(
            "resolve_failed",
            exc,
            elapsed_s=time.monotonic() - started,
            cold_locate_s=locate_s,
            reader_pid=reader_pid,
            requested_pid=pid,
            build_ok=build_ok,
            cache=cache_diag,
        )
    finally:
        if reader is not None:
            try:
                reader.close()
            except Exception:
                pass


def oob_command_bridge_rows_from_objects(objects: Any) -> List[Dict[str, int | str]]:
    """Normalize command-object roots into sorted ``kind:id:body_ptr`` rows."""
    if objects is None:
        return []
    raw_items: Iterable[Any]
    if isinstance(objects, str):
        raw_items = _iter_encoded_oob_command_bridge(objects)
    elif isinstance(objects, dict):
        rows: List[Any] = []
        for kind in OOB_COMMAND_KINDS:
            value = objects.get(kind)
            if isinstance(value, dict):
                for object_id, body_ptr in value.items():
                    rows.append((kind, object_id, body_ptr))
            elif isinstance(value, list):
                for item in value:
                    rows.append(_row_from_kind_item(kind, item))
        raw_items = rows
    else:
        raw_items = objects

    rows_by_key: Dict[Tuple[str, int], int] = {}
    seen_ptrs: set[int] = set()
    for item in raw_items:
        kind, object_id, body_ptr = _oob_bridge_triple(item)
        if kind not in _KIND_ORDER:
            raise ValueError("oob_command_bridge kind unsupported")
        if object_id <= 0 or object_id > MAX_OOB_COMMAND_ID:
            raise ValueError("oob_command_bridge id out of range")
        if body_ptr <= 0 or body_ptr % 8:
            raise ValueError("oob_command_bridge pointer must be nonzero and 8-byte aligned")
        key = (kind, object_id)
        if key in rows_by_key:
            raise ValueError("oob_command_bridge duplicate kind/id")
        if body_ptr in seen_ptrs:
            raise ValueError("oob_command_bridge duplicate pointer")
        rows_by_key[key] = body_ptr
        seen_ptrs.add(body_ptr)
        if len(rows_by_key) > MAX_OOB_COMMAND_BRIDGE_ROWS:
            raise ValueError("oob_command_bridge exceeds max rows")
    return [
        {"kind": kind, "id": object_id, "body_ptr": rows_by_key[(kind, object_id)]}
        for kind, object_id in sorted(rows_by_key, key=lambda k: (_KIND_ORDER[k[0]], k[1]))
    ]


def encode_oob_command_bridge(rows: Any) -> str:
    normalized = oob_command_bridge_rows_from_objects(rows)
    return ";".join(
        f"{row['kind']}:{row['id']}:0x{int(row['body_ptr']):X}"
        for row in normalized
    )


def _default_reader_factory(pid: int) -> Any:
    import oob_command_live as oob  # type: ignore
    import winmem  # type: ignore

    class _OobCommandReader:
        def __init__(self) -> None:
            self.proc = winmem.Process(pid)
            self.pid = self.proc.pid
            self.module_base = self.proc.module_base
            self.module_path = getattr(self.proc, "module_path", None)

        def fingerprint(self) -> Dict[str, Any]:
            return self.proc.fingerprint()

        def verify_build(self) -> tuple[bool, Optional[str]]:
            got = self.fingerprint().get("sha256")
            return (got == oob.EXPECTED_SHA, got)

        def enumerate_command_objs(self) -> Dict[str, Any]:
            return oob.enumerate_command_objs(self.proc)

        def validate_row(self, kind: str, object_id: int, body_ptr: int) -> bool:
            vt = {
                "army": oob.ARMY_VT_OFF,
                "ag": oob.AG_VT_OFF,
                "fleet": oob.FLEET_VT_OFF,
                "tf": oob.TF_VT_OFF,
                "theatre": oob.THEATRE_VT_OFF,
                "div": oob.DIV_VT_OFF,
                "ship": oob.SHIP_VT_OFF,
            }.get(kind)
            typ = {
                "army": oob.TYPE_ARMY,
                "ag": oob.TYPE_ARMY,
                "fleet": oob.TYPE_FLEET,
                "tf": oob.TYPE_FLEET,
                "theatre": oob.TYPE_THEATRE,
                "div": oob.TYPE_DIVISION,
                "ship": oob.TYPE_SHIP,
            }.get(kind)
            if vt is None or typ is None:
                return False
            try:
                if self.proc.u64(body_ptr) != self.module_base + vt:
                    return False
                handle = self.proc.u64(body_ptr + oob.OFF_HANDLE)
            except OSError:
                return False
            return (
                (handle & 0xFFFFFFFF) == typ
                and ((handle >> 32) & 0xFFFFFFFF) == int(object_id)
            )

        def close(self) -> None:
            self.proc.close()

    return _OobCommandReader()


def _row_from_kind_item(kind: str, item: Any) -> tuple[str, Any, Any]:
    if isinstance(item, dict):
        return (kind, item.get("id"), item.get("body_ptr", item.get("obj")))
    if isinstance(item, (list, tuple)) and len(item) == 2:
        body_ptr, object_id = item
        return (kind, object_id, body_ptr)
    if isinstance(item, (list, tuple)) and len(item) == 3:
        return (str(item[0]), item[1], item[2])
    raise ValueError("oob_command_bridge row malformed")


def _oob_bridge_triple(item: Any) -> tuple[str, int, int]:
    if isinstance(item, dict):
        kind = item.get("kind")
        object_id = item.get("id")
        body_ptr = item.get("body_ptr", item.get("obj"))
    elif isinstance(item, (list, tuple)) and len(item) == 3:
        kind, object_id, body_ptr = item
    else:
        raise ValueError("oob_command_bridge row malformed")
    return str(kind), int(object_id), _parse_int(body_ptr)


def _iter_encoded_oob_command_bridge(value: str) -> Iterable[tuple[str, int, int]]:
    if not value:
        return []
    rows = []
    for raw in value.split(";"):
        parts = raw.split(":")
        if len(parts) != 3:
            raise ValueError("oob_command_bridge encoded row malformed")
        rows.append((parts[0], int(parts[1], 0), int(parts[2], 0)))
    return rows


def _parse_int(value: Any) -> int:
    if isinstance(value, int):
        return value
    if isinstance(value, str):
        return int(value, 0)
    raise ValueError("integer expected")


def _validated_cached_rows(reader: Any, rows: List[Dict[str, int | str]]) -> List[Dict[str, int | str]]:
    out = []
    for row in rows:
        kind = str(row["kind"])
        object_id = int(row["id"])
        body_ptr = int(row["body_ptr"])
        validate = getattr(reader, "validate_row", None)
        if callable(validate):
            if validate(kind, object_id, body_ptr):
                out.append(row)
            continue
        out.append(row)
    return out


def _coerce_cached_rows(value: Any) -> List[Dict[str, int | str]]:
    return oob_command_bridge_rows_from_objects(value)


def _load_oob_command_cache(
    path: Path,
    identity: _process_identity.ProcessIdentity,
    *,
    min_rows: int,
) -> Optional[Dict[str, Any]]:
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None
    if payload.get("cache_type") != "oob_command_bridge_preflight_cache_v1":
        return None
    if int(payload.get("pid", -1)) != identity.pid:
        return None
    if payload.get("create_time_100ns") != identity.create_time_100ns:
        return None
    if payload.get("exe_sha256") != identity.exe_sha256:
        return None
    if payload.get("module_base") != identity.module_base:
        return None
    if int(payload.get("row_count", 0)) < int(min_rows):
        return None
    return payload


def _write_oob_command_cache(
    path: Path,
    identity: _process_identity.ProcessIdentity,
    rows: List[Dict[str, int | str]],
) -> bool:
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        payload = {
            "cache_type": "oob_command_bridge_preflight_cache_v1",
            "created_utc": datetime.now(timezone.utc).isoformat(),
            "pid": identity.pid,
            "create_time_100ns": identity.create_time_100ns,
            "exe_sha256": identity.exe_sha256,
            "module_base": identity.module_base,
            "row_count": len(rows),
            "counts": _counts_by_kind(rows),
            "objects": rows,
        }
        tmp = path.with_suffix(path.suffix + ".tmp")
        tmp.write_text(json.dumps(payload, sort_keys=True) + "\n", encoding="utf-8")
        os.replace(tmp, path)
        return True
    except OSError:
        return False


def _diagnostics(
    status: str,
    rows: List[Dict[str, int | str]],
    *,
    elapsed_s: float,
    cold_locate_s: Optional[float] = None,
    reader_pid: Optional[int] = None,
    requested_pid: Optional[int] = None,
    build_ok: Optional[bool] = None,
    bridge_supplied: bool = False,
    source: Optional[str] = None,
    cache: Optional[Dict[str, Any]] = None,
    error: Optional[str] = None,
) -> Dict[str, Any]:
    diag: Dict[str, Any] = {
        "status": status,
        "row_count": len(rows),
        "counts": _counts_by_kind(rows),
        "elapsed_s": elapsed_s,
        "hot_loop_heap_scan": False,
        "bridge_supplied": bridge_supplied,
        "allow_cold_scan": bool((cache or {}).get("allow_cold_scan", False)),
        "address_manifest_policy": "omitted",
    }
    if cold_locate_s is not None:
        diag["cold_locate_s"] = cold_locate_s
    if reader_pid is not None:
        diag["reader_pid"] = reader_pid
    if requested_pid is not None:
        diag["requested_pid"] = requested_pid
        diag["pid_match"] = reader_pid == requested_pid
    if build_ok is not None:
        diag["build_ok"] = build_ok
    if source:
        diag["source"] = source
    if cache is not None:
        diag["cache"] = dict(cache)
    if error:
        diag["error"] = error[:240]
    return diag


def _preflight_status(
    status: str,
    rows: List[Dict[str, int | str]],
    *,
    elapsed_s: float,
    cold_locate_s: Optional[float] = None,
    reader_pid: Optional[int] = None,
    requested_pid: Optional[int] = None,
    build_ok: Optional[bool] = None,
    source: Optional[str] = None,
    cache: Optional[Dict[str, Any]] = None,
) -> OobCommandBridgePreflight:
    return OobCommandBridgePreflight(
        status=status,
        row_count=len(rows),
        encoded=None,
        diagnostics=_diagnostics(
            status,
            rows,
            elapsed_s=elapsed_s,
            cold_locate_s=cold_locate_s,
            reader_pid=reader_pid,
            requested_pid=requested_pid,
            build_ok=build_ok,
            source=source,
            cache=cache,
        ),
    )


def _preflight_error(status: str, exc: Exception, **kwargs: Any) -> OobCommandBridgePreflight:
    return OobCommandBridgePreflight(
        status=status,
        row_count=0,
        encoded=None,
        diagnostics=_diagnostics(status, [], error=_safe_error(exc), **kwargs),
    )


def _counts_by_kind(rows: List[Dict[str, int | str]]) -> Dict[str, int]:
    counts = {kind: 0 for kind in OOB_COMMAND_KINDS}
    for row in rows:
        kind = str(row.get("kind"))
        if kind in counts:
            counts[kind] += 1
    return counts


def _coerce_optional_int(value: Any) -> Optional[int]:
    try:
        return None if value is None else int(value)
    except (TypeError, ValueError):
        return None


def _safe_error(exc: Exception) -> str:
    return re.sub(r"0x[0-9A-Fa-f]+", "0x<redacted>", f"{type(exc).__name__}: {exc}")


__all__ = [
    "OOB_COMMAND_KINDS",
    "OobCommandBridgePreflight",
    "encode_oob_command_bridge",
    "oob_command_bridge_rows_from_objects",
    "resolve_oob_command_bridge_preflight",
]
