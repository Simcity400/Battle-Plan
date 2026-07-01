"""Preflight division-body bridge for the in-process collector.

This resolves live CDivision body addresses once before a collector session and
encodes them for the temporary DLL control file. Diagnostics intentionally omit
raw addresses; the encoded bridge is ephemeral and must not be copied into
session/probe manifests.
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


MIN_USABLE_DIVISION_BODY_ROWS = 1
MAX_DIVISION_BODY_BRIDGE_ROWS = 20000
MAX_DIVISION_BODY_BRIDGE_DIV_ID = 2_147_483_647

_HERE = Path(__file__).resolve().parent
_REPO = _HERE.parent
for _p in (str(_REPO), str(_REPO / "memory_reader")):
    if _p not in sys.path:
        sys.path.insert(0, _p)


@dataclass(frozen=True)
class DivisionBodyBridgePreflight:
    status: str
    row_count: int
    encoded: Optional[str]
    diagnostics: Dict[str, Any]

    @property
    def usable(self) -> bool:
        return self.status == "resolved" and bool(self.encoded)


def resolve_division_body_bridge_preflight(
    pid: int,
    *,
    reader_factory: Optional[Callable[[], Any]] = None,
    min_rows: int = MIN_USABLE_DIVISION_BODY_ROWS,
    cache_path: Optional[Path] = None,
    allow_cold_scan: bool = False,
) -> DivisionBodyBridgePreflight:
    """Resolve live division body rows for one recorder session.

    ``DivisionLiveReader`` finds the live ``hoi4.exe`` process itself, so this
    verifies the reader-selected pid against the requested recorder pid before
    any bridge is accepted. The build gate is verified before accepting cached
    bodies or locating new ones. Cache hits validate each body with the warmed
    reader path and re-emit only still-live ``div_id -> body`` pins. Cold heap
    discovery is explicit so normal speed-five recording can fail closed quickly
    instead of doing a surprise arena scan.
    """
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
        factory = reader_factory or _default_reader_factory
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
            cached = _load_division_body_cache(
                Path(cache_path),
                identity,
                min_rows=int(min_rows),
            )
            if cached is not None:
                cache_diag["status"] = "loaded"
                cached_bodies = _coerce_cached_division_bodies(
                    cached.get("bodies"))
                valid_bodies = _validated_cached_division_bodies(
                    reader,
                    cached_bodies,
                )
                rows = division_body_bridge_rows_from_bodies(valid_bodies)
                if len(rows) >= int(min_rows):
                    cache_diag["status"] = "hit"
                    encoded = encode_division_body_bridge(rows)
                    return DivisionBodyBridgePreflight(
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
                            source="division_body_cache",
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
                source="division_body_cache",
                cache=cache_diag,
            )

        locate_started = time.monotonic()
        bodies = reader.locate_division_bodies()
        locate_s = time.monotonic() - locate_started
        rows = division_body_bridge_rows_from_bodies(bodies)
        if len(rows) < int(min_rows):
            return _preflight_status(
                "insufficient_rows",
                rows,
                elapsed_s=time.monotonic() - started,
                cold_locate_s=locate_s,
                reader_pid=reader_pid,
                requested_pid=pid,
                build_ok=True,
                source="DivisionLiveReader.locate_division_bodies",
                cache=cache_diag,
            )
        if cache_path is not None:
            cache_diag["status"] = (
                "stored" if _write_division_body_cache(
                    Path(cache_path),
                    identity,
                    rows,
                ) else "store_failed"
            )
        encoded = encode_division_body_bridge(rows)
        return DivisionBodyBridgePreflight(
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
                source="DivisionLiveReader.locate_division_bodies",
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


def division_body_bridge_rows_from_bodies(bodies: Any) -> List[Dict[str, int]]:
    """Normalize ``{div_id: body_ptr}`` rows from ``DivisionLiveReader``."""
    if bodies is None:
        return []
    raw_items: Iterable[Any]
    if isinstance(bodies, str):
        raw_items = _iter_encoded_division_body_bridge(bodies)
    elif isinstance(bodies, dict):
        raw_items = bodies.items()
    else:
        raw_items = bodies

    rows_by_id: Dict[int, int] = {}
    seen_ptrs: set[int] = set()
    for item in raw_items:
        div_id, body_ptr = _division_bridge_pair(item)
        if div_id <= 0 or div_id > MAX_DIVISION_BODY_BRIDGE_DIV_ID:
            raise ValueError("division_body_bridge div_id out of range")
        if body_ptr <= 0 or body_ptr % 8:
            raise ValueError("division_body_bridge pointer must be nonzero and 8-byte aligned")
        if div_id in rows_by_id:
            raise ValueError("division_body_bridge duplicate div_id")
        if body_ptr in seen_ptrs:
            raise ValueError("division_body_bridge duplicate pointer")
        rows_by_id[div_id] = body_ptr
        seen_ptrs.add(body_ptr)
        if len(rows_by_id) > MAX_DIVISION_BODY_BRIDGE_ROWS:
            raise ValueError("division_body_bridge exceeds max rows")
    return [
        {"div_id": div_id, "body_ptr": rows_by_id[div_id]}
        for div_id in sorted(rows_by_id)
    ]


def encode_division_body_bridge(rows: Any) -> str:
    """Encode ``div_id:0xbody`` rows for ``collector_control.json``."""
    normalized = division_body_bridge_rows_from_bodies(rows)
    return ";".join(
        f"{row['div_id']}:0x{row['body_ptr']:X}" for row in normalized
    )


def _default_reader_factory() -> Any:
    from memory_reader.division_live.division_live_reader import (  # type: ignore
        DivisionLiveReader,
    )

    return DivisionLiveReader()


def _division_bridge_pair(item: Any) -> Tuple[int, int]:
    if isinstance(item, dict):
        div_raw = item.get("div_id", item.get("id"))
        ptr_raw = item.get("body_ptr", item.get("body_addr"))
    else:
        try:
            div_raw, ptr_raw = item
        except Exception as exc:
            raise ValueError("division_body_bridge row must be pair-like") from exc
    return (
        _bridge_int(div_raw, "div_id"),
        _bridge_int(ptr_raw, "body_ptr"),
    )


def _iter_encoded_division_body_bridge(value: str) -> Iterable[Tuple[str, str]]:
    if not value:
        return []
    pairs = []
    for raw in value.split(";"):
        if not raw:
            raise ValueError("division_body_bridge contains empty row")
        if ":" not in raw:
            raise ValueError("division_body_bridge row missing ':'")
        pairs.append(tuple(raw.split(":", 1)))
    return pairs


def _bridge_int(value: Any, name: str) -> int:
    if isinstance(value, bool):
        raise ValueError(f"division_body_bridge {name} must be int-like")
    if isinstance(value, int):
        return value
    if isinstance(value, str):
        value = value.strip()
        if value.lower().startswith("0x"):
            return int(value, 16)
        return int(value, 10)
    raise ValueError(f"division_body_bridge {name} must be int-like")


def _coerce_optional_int(value: Any) -> Optional[int]:
    if isinstance(value, bool) or value is None:
        return None
    try:
        return int(value)
    except (TypeError, ValueError):
        return None


def _preflight_status(
    status: str,
    rows: Iterable[Dict[str, int]],
    *,
    elapsed_s: float,
    cold_locate_s: Optional[float] = None,
    reader_pid: Optional[int],
    requested_pid: int,
    build_ok: Optional[bool],
    source: str = "DivisionLiveReader.locate_division_bodies",
    cache: Optional[Dict[str, Any]] = None,
) -> DivisionBodyBridgePreflight:
    rows = list(rows)
    return DivisionBodyBridgePreflight(
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
            bridge_supplied=False,
            source=source,
            cache=cache,
        ),
    )


def _preflight_error(
    status: str,
    exc: BaseException,
    *,
    elapsed_s: float,
    cold_locate_s: Optional[float] = None,
    reader_pid: Optional[int],
    requested_pid: int,
    build_ok: Optional[bool],
    cache: Optional[Dict[str, Any]] = None,
) -> DivisionBodyBridgePreflight:
    diagnostics = _diagnostics(
        status,
        [],
        elapsed_s=elapsed_s,
        cold_locate_s=cold_locate_s,
        reader_pid=reader_pid,
        requested_pid=requested_pid,
        build_ok=build_ok,
        bridge_supplied=False,
        cache=cache,
    )
    diagnostics["error"] = _safe_error_message(exc)
    return DivisionBodyBridgePreflight(
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
    cold_locate_s: Optional[float],
    reader_pid: Optional[int],
    requested_pid: int,
    build_ok: Optional[bool],
    bridge_supplied: bool,
    source: str = "DivisionLiveReader.locate_division_bodies",
    cache: Optional[Dict[str, Any]] = None,
) -> Dict[str, Any]:
    row_count = len(list(rows))
    diagnostics: Dict[str, Any] = {
        "status": status,
        "row_count": row_count,
        "build_ok": bool(build_ok) if build_ok is not None else None,
        "pid_match": reader_pid == int(requested_pid),
        "requested_pid": int(requested_pid),
        "reader_pid": reader_pid,
        "elapsed_s": float(elapsed_s),
        "cold_locate_s": float(cold_locate_s) if cold_locate_s is not None else None,
        "bridge_supplied": bool(bridge_supplied),
        "source": source,
        "runtime_source": "hoi4.exe",
        "address_manifest_policy": "omitted",
    }
    if cache is not None:
        diagnostics["cache"] = {
            "enabled": bool(cache.get("enabled")),
            "status": str(cache.get("status") or "unknown"),
        }
        diagnostics["allow_cold_scan"] = bool(cache.get("allow_cold_scan"))
    return diagnostics


def _validated_cached_division_bodies(
    reader: Any,
    bodies: Dict[int, int],
) -> Dict[int, int]:
    if not bodies:
        return {}
    read_from_bodies = getattr(reader, "read_divisions_from_bodies", None)
    if not callable(read_from_bodies):
        return {}
    try:
        decoded = read_from_bodies(bodies)
    except Exception:
        return {}
    valid_ids: set[int] = set()
    for row in decoded or []:
        if not isinstance(row, dict):
            continue
        div_id = _coerce_optional_int(row.get("id", row.get("div_id")))
        if div_id is not None and div_id in bodies:
            valid_ids.add(div_id)
    return {div_id: bodies[div_id] for div_id in sorted(valid_ids)}


def _coerce_cached_division_bodies(value: Any) -> Dict[int, int]:
    if not isinstance(value, dict):
        return {}
    out: Dict[int, int] = {}
    seen_ptrs: set[int] = set()
    for raw_id, raw_ptr in value.items():
        if isinstance(raw_id, bool) or isinstance(raw_ptr, bool):
            continue
        try:
            div_id = int(raw_id)
            body_ptr = int(raw_ptr)
        except (TypeError, ValueError):
            continue
        if div_id <= 0 or div_id > MAX_DIVISION_BODY_BRIDGE_DIV_ID:
            continue
        if body_ptr <= 0 or body_ptr % 8:
            continue
        if body_ptr in seen_ptrs:
            return {}
        out[div_id] = body_ptr
        seen_ptrs.add(body_ptr)
        if len(out) > MAX_DIVISION_BODY_BRIDGE_ROWS:
            return {}
    return out


def _load_division_body_cache(
    cache_path: Path,
    identity: _process_identity.ProcessIdentity,
    *,
    min_rows: int,
) -> Optional[Dict[str, Any]]:
    try:
        payload = json.loads(Path(cache_path).read_text(encoding="utf-8"))
    except (OSError, ValueError, TypeError):
        return None
    if not isinstance(payload, dict):
        return None
    if payload.get("type") != "division_body_bridge_preflight_cache_v1":
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
    bodies = _coerce_cached_division_bodies(payload.get("bodies"))
    if len(bodies) < int(min_rows):
        return None
    return payload


def _write_division_body_cache(
    cache_path: Path,
    identity: _process_identity.ProcessIdentity,
    rows: Iterable[Dict[str, int]],
) -> bool:
    try:
        normalized = division_body_bridge_rows_from_bodies(rows)
    except Exception:
        return False
    payload = {
        "type": "division_body_bridge_preflight_cache_v1",
        "created_utc": datetime.now(timezone.utc).isoformat(),
        "pid": int(identity.pid),
        "create_time_100ns": identity.create_time_100ns,
        "exe_sha256": identity.exe_sha256,
        "module_base": identity.module_base,
        "row_count": len(normalized),
        "bodies": {
            str(int(row["div_id"])): int(row["body_ptr"])
            for row in normalized
        },
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
    msg = re.sub(r"0x[0-9A-Fa-f]+", "0x<redacted>", msg)
    msg = re.sub(r"[A-Za-z]:\\[^;\n\r]+", "<path>", msg)
    msg = re.sub(r"/[^;\n\r ]+", "<path>", msg)
    for value in (os.environ.get("USERNAME"), os.path.expanduser("~")):
        if value:
            msg = msg.replace(value, "<redacted>")
    return msg[:240]


__all__ = [
    "DivisionBodyBridgePreflight",
    "division_body_bridge_rows_from_bodies",
    "encode_division_body_bridge",
    "resolve_division_body_bridge_preflight",
]
