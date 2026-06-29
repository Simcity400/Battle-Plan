"""Preflight live special-project roots for the in-process collector.

The existing ``memory_reader.projects_live`` module already locates and decodes
facilities, CProject objects, and scientist characters. This module turns those
locators into the same cache-first control bridge used by divisions, ships, and
OOB command roots: Python resolves and validates ephemeral addresses before the
worker starts, then Rust revalidates the objects in-process before every read.
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


MIN_USABLE_SPECIAL_PROJECT_ROWS = 1
MAX_SPECIAL_PROJECT_BRIDGE_ROWS = 30000
MAX_SPECIAL_PROJECT_ID = 10_000_000
MAX_SPECIAL_PROJECT_PROVINCE_ID = 1_000_000
MAX_SPECIAL_PROJECT_HANDLE = 10_000_000
SPECIAL_PROJECT_KINDS = ("facility", "project", "scientist")
_KIND_ORDER = {kind: index for index, kind in enumerate(SPECIAL_PROJECT_KINDS)}

_HERE = Path(__file__).resolve().parent
_REPO = _HERE.parent
for _p in (str(_REPO), str(_REPO / "memory_reader")):
    if _p not in sys.path:
        sys.path.insert(0, _p)


@dataclass(frozen=True)
class SpecialProjectsBridgePreflight:
    status: str
    row_count: int
    encoded: Optional[str]
    diagnostics: Dict[str, Any]

    @property
    def usable(self) -> bool:
        return self.status == "resolved" and bool(self.encoded)


def resolve_special_projects_bridge_preflight(
    pid: int,
    *,
    reader_factory: Optional[Callable[[], Any]] = None,
    min_rows: int = MIN_USABLE_SPECIAL_PROJECT_ROWS,
    cache_path: Optional[Path] = None,
    allow_cold_scan: bool = False,
) -> SpecialProjectsBridgePreflight:
    """Resolve live special-project object rows for one recorder session."""
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
            cached = _load_special_projects_cache(
                Path(cache_path), identity, min_rows=int(min_rows))
            if cached is not None:
                cache_diag["status"] = "loaded"
                cached_rows = _coerce_cached_rows(cached.get("objects"))
                valid_rows = _validated_cached_rows(reader, cached_rows)
                rows = special_projects_bridge_rows_from_objects(valid_rows)
                if len(rows) >= int(min_rows):
                    cache_diag["status"] = "hit"
                    encoded = encode_special_projects_bridge(rows)
                    return SpecialProjectsBridgePreflight(
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
                            source="special_projects_cache",
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
                source="special_projects_cache",
                cache=cache_diag,
            )

        locate_started = time.monotonic()
        objects = reader.enumerate_special_project_objs()
        locate_s = time.monotonic() - locate_started
        rows = special_projects_bridge_rows_from_objects(objects)
        if len(rows) < int(min_rows):
            return _preflight_status(
                "insufficient_rows",
                rows,
                elapsed_s=time.monotonic() - started,
                cold_locate_s=locate_s,
                reader_pid=reader_pid,
                requested_pid=pid,
                build_ok=True,
                source="projects_live.enumerate_*_addrs",
                cache=cache_diag,
            )
        if cache_path is not None:
            cache_diag["status"] = (
                "stored" if _write_special_projects_cache(
                    Path(cache_path), identity, rows)
                else "store_failed"
            )
        encoded = encode_special_projects_bridge(rows)
        return SpecialProjectsBridgePreflight(
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
                source="projects_live.enumerate_*_addrs",
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


def special_projects_bridge_rows_from_objects(objects: Any) -> List[Dict[str, int | str]]:
    """Normalize special-project roots into sorted bridge rows."""
    if objects is None:
        return []
    raw_items: Iterable[Any]
    if isinstance(objects, str):
        raw_items = _iter_encoded_special_projects_bridge(objects)
    elif isinstance(objects, dict):
        rows: List[Any] = []
        for kind in SPECIAL_PROJECT_KINDS:
            value = objects.get(kind) or objects.get(f"{kind}s")
            if isinstance(value, dict):
                for object_id, body_ptr in value.items():
                    rows.append((kind, object_id, 0, body_ptr))
            elif isinstance(value, list):
                for item in value:
                    rows.append(_row_from_kind_item(kind, item))
        raw_items = rows
    else:
        raw_items = objects

    rows_by_key: Dict[Tuple[str, int, int], int] = {}
    seen_ptrs: set[int] = set()
    for item in raw_items:
        kind, object_id, aux_id, body_ptr = _bridge_quad(item)
        if kind not in _KIND_ORDER:
            raise ValueError("special_projects_bridge kind unsupported")
        _validate_project_ids(kind, object_id, aux_id)
        if body_ptr <= 0 or body_ptr % 8:
            raise ValueError("special_projects_bridge pointer must be nonzero and 8-byte aligned")
        key = (kind, object_id, aux_id)
        if key in rows_by_key:
            raise ValueError("special_projects_bridge duplicate kind/id")
        if body_ptr in seen_ptrs:
            raise ValueError("special_projects_bridge duplicate pointer")
        rows_by_key[key] = body_ptr
        seen_ptrs.add(body_ptr)
        if len(rows_by_key) > MAX_SPECIAL_PROJECT_BRIDGE_ROWS:
            raise ValueError("special_projects_bridge exceeds max rows")
    return [
        {
            "kind": kind,
            "id": object_id,
            "aux_id": aux_id,
            "body_ptr": rows_by_key[(kind, object_id, aux_id)],
        }
        for kind, object_id, aux_id in sorted(
            rows_by_key, key=lambda k: (_KIND_ORDER[k[0]], k[1], k[2]))
    ]


def encode_special_projects_bridge(rows: Any) -> str:
    normalized = special_projects_bridge_rows_from_objects(rows)
    encoded_rows = []
    for row in normalized:
        kind = str(row["kind"])
        object_id = int(row["id"])
        aux_id = int(row["aux_id"])
        body_ptr = int(row["body_ptr"])
        if kind == "scientist":
            encoded_rows.append(f"{kind}:{object_id}:0x{body_ptr:X}")
        else:
            encoded_rows.append(f"{kind}:{object_id}:{aux_id}:0x{body_ptr:X}")
    return ";".join(encoded_rows)


def _default_reader_factory(pid: int) -> Any:
    import projects_live  # type: ignore
    import winmem  # type: ignore

    class _SpecialProjectsReader:
        def __init__(self) -> None:
            self.proc = winmem.Process(pid)
            self.pid = self.proc.pid
            self.module_base = self.proc.module_base
            self.module_path = getattr(self.proc, "module_path", None)

        def fingerprint(self) -> Dict[str, Any]:
            return self.proc.fingerprint()

        def verify_build(self) -> tuple[bool, Optional[str]]:
            got = self.fingerprint().get("sha256")
            return (got == projects_live.EXPECTED_SHA256, got)

        def enumerate_special_project_objs(self) -> Dict[str, Any]:
            return {
                "facility": projects_live.enumerate_facility_addrs(self.proc),
                "project": projects_live.enumerate_project_addrs(self.proc),
                "scientist": projects_live.enumerate_scientist_addrs(self.proc),
            }

        def validate_row(
            self,
            kind: str,
            object_id: int,
            aux_id: int,
            body_ptr: int,
        ) -> bool:
            try:
                if kind == "facility":
                    return (
                        self.proc.u64(body_ptr) == self.module_base + projects_live.FAC_VT_RVA
                        and self.proc.u32(body_ptr + projects_live.FAC_TYPE)
                        == projects_live.FAC_TYPE_VAL
                        and self.proc.u32(body_ptr + projects_live.FAC_ID) == int(object_id)
                        and self.proc.u32(body_ptr + projects_live.FAC_PROV) == int(aux_id)
                    )
                if kind == "project":
                    return (
                        self.proc.u64(body_ptr) == self.module_base + projects_live.PROJ_VT_RVA
                        and self.proc.u32(body_ptr + projects_live.PROJ_TYPE)
                        == projects_live.PROJ_TYPE_VAL
                        and self.proc.u32(body_ptr + projects_live.PROJ_ID) == int(object_id)
                        and self.proc.u32(body_ptr + projects_live.PROJ_DEF_HANDLE)
                        == int(aux_id)
                    )
                if kind == "scientist":
                    return projects_live.validate_scientist_addrs(
                        self.proc, [(int(body_ptr), (int(object_id),))])
            except OSError:
                return False
            return False

        def close(self) -> None:
            self.proc.close()

    return _SpecialProjectsReader()


def _row_from_kind_item(kind: str, item: Any) -> tuple[str, Any, Any, Any]:
    if isinstance(item, dict):
        body_ptr = item.get("body_ptr", item.get("_addr", item.get("addr")))
        if kind == "facility":
            return (kind, item.get("id", item.get("facility_id")), item.get("province"), body_ptr)
        if kind == "project":
            return (kind, item.get("id", item.get("project_id")),
                    item.get("static_def_handle"), body_ptr)
        if kind == "scientist":
            return (kind, item.get("char_id"), 0, body_ptr)
    if isinstance(item, (list, tuple)) and len(item) == 2:
        body_ptr, fp = item
        if kind == "facility" and isinstance(fp, (list, tuple)) and len(fp) >= 2:
            return (kind, fp[0], fp[1], body_ptr)
        if kind == "project" and isinstance(fp, (list, tuple)) and len(fp) >= 2:
            return (kind, fp[0], fp[1], body_ptr)
        if kind == "scientist" and isinstance(fp, (list, tuple)) and len(fp) >= 1:
            return (kind, fp[0], 0, body_ptr)
    if isinstance(item, (list, tuple)) and len(item) == 3 and kind == "scientist":
        return (str(item[0]), item[1], 0, item[2])
    if isinstance(item, (list, tuple)) and len(item) == 4:
        return (str(item[0]), item[1], item[2], item[3])
    raise ValueError("special_projects_bridge row malformed")


def _bridge_quad(item: Any) -> tuple[str, int, int, int]:
    if isinstance(item, dict):
        kind = item.get("kind")
        object_id = item.get("id")
        aux_id = item.get("aux_id", item.get("province", item.get("static_def_handle", 0)))
        body_ptr = item.get("body_ptr", item.get("_addr", item.get("addr")))
    elif isinstance(item, (list, tuple)) and len(item) == 4:
        kind, object_id, aux_id, body_ptr = item
    else:
        raise ValueError("special_projects_bridge row malformed")
    return str(kind), int(object_id), int(aux_id), _parse_int(body_ptr)


def _iter_encoded_special_projects_bridge(value: str) -> Iterable[tuple[str, int, int, int]]:
    if not value:
        return []
    rows = []
    for raw in value.split(";"):
        parts = raw.split(":")
        if len(parts) == 3 and parts[0] == "scientist":
            rows.append((parts[0], int(parts[1], 0), 0, int(parts[2], 0)))
        elif len(parts) == 4 and parts[0] in ("facility", "project"):
            rows.append((parts[0], int(parts[1], 0), int(parts[2], 0), int(parts[3], 0)))
        else:
            raise ValueError("special_projects_bridge encoded row malformed")
    return rows


def _parse_int(value: Any) -> int:
    if isinstance(value, int):
        return value
    if isinstance(value, str):
        return int(value, 0)
    raise ValueError("integer expected")


def _validate_project_ids(kind: str, object_id: int, aux_id: int) -> None:
    if object_id <= 0 or object_id > MAX_SPECIAL_PROJECT_ID:
        raise ValueError("special_projects_bridge id out of range")
    if kind == "facility":
        if aux_id <= 0 or aux_id > MAX_SPECIAL_PROJECT_PROVINCE_ID:
            raise ValueError("special_projects_bridge province out of range")
    elif kind == "project":
        if aux_id <= 0 or aux_id > MAX_SPECIAL_PROJECT_HANDLE:
            raise ValueError("special_projects_bridge handle out of range")
    elif aux_id != 0:
        raise ValueError("special_projects_bridge scientist aux id must be zero")


def _validated_cached_rows(reader: Any, rows: List[Dict[str, int | str]]) -> List[Dict[str, int | str]]:
    out = []
    validate = getattr(reader, "validate_row", None)
    for row in rows:
        kind = str(row["kind"])
        object_id = int(row["id"])
        aux_id = int(row["aux_id"])
        body_ptr = int(row["body_ptr"])
        if callable(validate):
            if validate(kind, object_id, aux_id, body_ptr):
                out.append(row)
            continue
        out.append(row)
    return out


def _coerce_cached_rows(value: Any) -> List[Dict[str, int | str]]:
    return special_projects_bridge_rows_from_objects(value)


def _load_special_projects_cache(
    path: Path,
    identity: _process_identity.ProcessIdentity,
    *,
    min_rows: int,
) -> Optional[Dict[str, Any]]:
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None
    if payload.get("cache_type") != "special_projects_bridge_preflight_cache_v1":
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


def _write_special_projects_cache(
    path: Path,
    identity: _process_identity.ProcessIdentity,
    rows: List[Dict[str, int | str]],
) -> bool:
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        payload = {
            "cache_type": "special_projects_bridge_preflight_cache_v1",
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
) -> SpecialProjectsBridgePreflight:
    return SpecialProjectsBridgePreflight(
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


def _preflight_error(
    status: str,
    exc: Exception,
    *,
    elapsed_s: float,
    cold_locate_s: Optional[float] = None,
    reader_pid: Optional[int] = None,
    requested_pid: Optional[int] = None,
    build_ok: Optional[bool] = None,
    cache: Optional[Dict[str, Any]] = None,
) -> SpecialProjectsBridgePreflight:
    return SpecialProjectsBridgePreflight(
        status=status,
        row_count=0,
        encoded=None,
        diagnostics=_diagnostics(
            status,
            [],
            elapsed_s=elapsed_s,
            cold_locate_s=cold_locate_s,
            reader_pid=reader_pid,
            requested_pid=requested_pid,
            build_ok=build_ok,
            cache=cache,
            error=_safe_error_message(exc),
        ),
    )


def _counts_by_kind(rows: Iterable[Dict[str, int | str]]) -> Dict[str, int]:
    counts = {kind: 0 for kind in SPECIAL_PROJECT_KINDS}
    for row in rows:
        kind = str(row.get("kind"))
        if kind in counts:
            counts[kind] += 1
    return counts


def _coerce_optional_int(value: Any) -> Optional[int]:
    try:
        return int(value) if value is not None else None
    except (TypeError, ValueError):
        return None


_HEX_RE = re.compile(r"0x[0-9a-fA-F]+")


def _safe_error_message(exc: Exception) -> str:
    return _HEX_RE.sub("0x<redacted>", str(exc))


__all__ = [
    "SpecialProjectsBridgePreflight",
    "encode_special_projects_bridge",
    "resolve_special_projects_bridge_preflight",
    "special_projects_bridge_rows_from_objects",
]
