"""Preflight division-body bridge for the in-process collector.

This resolves live CDivision body addresses once before a collector session and
encodes them for the temporary DLL control file. Diagnostics intentionally omit
raw addresses; the encoded bridge is ephemeral and must not be copied into
session/probe manifests.
"""
from __future__ import annotations

from dataclasses import dataclass
import os
from pathlib import Path
import re
import sys
import time
from typing import Any, Callable, Dict, Iterable, List, Optional, Tuple


MAX_DIVISION_BODY_BRIDGE_ROWS = 20000

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
    min_rows: int = 1,
) -> DivisionBodyBridgePreflight:
    """Resolve live division body rows for one recorder session.

    ``DivisionLiveReader`` finds the live ``hoi4.exe`` process itself, so this
    verifies the reader-selected pid against the requested recorder pid before
    any bridge is accepted. The build gate is verified before locating bodies.
    """
    started = time.monotonic()
    locate_s: Optional[float] = None
    reader = None
    reader_pid: Optional[int] = None
    build_ok: Optional[bool] = None
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
            )

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
        if div_id <= 0:
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
        "source": "DivisionLiveReader.locate_division_bodies",
        "runtime_source": "hoi4.exe",
        "address_manifest_policy": "omitted",
    }
    return diagnostics


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
