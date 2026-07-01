"""Preflight naval-combat bridge for the in-process collector.

This resolves live CNavalCombat body addresses before a collector session and
encodes them for the temporary DLL control file. The underlying reader still
uses a bounded preflight heap/vtable enumeration, so this module is explicit
recorder-side setup, not hot-loop discovery.
"""
from __future__ import annotations

from dataclasses import dataclass
import json
import os
from pathlib import Path
import re
import sys
import time
from typing import Any, Callable, Dict, Iterable, List, Optional, Tuple


MIN_USABLE_NAVAL_COMBAT_ROWS = 1
MAX_NAVAL_COMBAT_BODY_BRIDGE_ROWS = 4096
MAX_NAVAL_COMBAT_ID = 2_147_483_647

_HERE = Path(__file__).resolve().parent
_REPO = _HERE.parent
for _p in (str(_REPO), str(_REPO / "memory_reader")):
    if _p not in sys.path:
        sys.path.insert(0, _p)


@dataclass(frozen=True)
class NavalCombatBodyBridgePreflight:
    status: str
    row_count: int
    encoded: Optional[str]
    diagnostics: Dict[str, Any]

    @property
    def usable(self) -> bool:
        return self.status == "resolved" and bool(self.encoded)


def resolve_naval_combat_body_bridge_preflight(
    pid: int,
    *,
    reader_factory: Optional[Callable[[], Any]] = None,
    min_rows: int = MIN_USABLE_NAVAL_COMBAT_ROWS,
) -> NavalCombatBodyBridgePreflight:
    """Resolve active naval-combat body rows for one recorder session."""
    started = time.monotonic()
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

        combats = reader.enumerate_naval_combats(with_defenders=True)
        rows = naval_combat_body_bridge_rows_from_combats(combats)
        if len(rows) < int(min_rows):
            return _preflight_status(
                "no_active_combats",
                rows,
                elapsed_s=time.monotonic() - started,
                reader_pid=reader_pid,
                requested_pid=pid,
                build_ok=True,
                reject_stats=_safe_reject_stats(getattr(reader, "reject_stats", None)),
            )
        encoded = encode_naval_combat_body_bridge(rows)
        return NavalCombatBodyBridgePreflight(
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
                reject_stats=_safe_reject_stats(getattr(reader, "reject_stats", None)),
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


def naval_combat_body_bridge_rows_from_combats(combats: Any) -> List[Dict[str, int]]:
    """Normalize ``{combat_id, body_addr}`` rows from ``CombatReader``."""
    if combats is None:
        return []
    raw_items: Iterable[Any]
    if isinstance(combats, str):
        raw_items = _iter_encoded_naval_combat_body_bridge(combats)
    elif isinstance(combats, dict):
        raw_items = combats.items()
    else:
        raw_items = combats

    rows_by_id: Dict[int, int] = {}
    seen_ptrs: set[int] = set()
    for item in raw_items:
        combat_id, body_ptr = _naval_bridge_pair(item)
        if combat_id <= 0 or combat_id > MAX_NAVAL_COMBAT_ID:
            raise ValueError("naval_combat_body_bridge combat_id out of range")
        if body_ptr <= 0 or body_ptr % 8:
            raise ValueError(
                "naval_combat_body_bridge pointer must be nonzero and 8-byte aligned"
            )
        if combat_id in rows_by_id:
            raise ValueError("naval_combat_body_bridge duplicate combat_id")
        if body_ptr in seen_ptrs:
            raise ValueError("naval_combat_body_bridge duplicate pointer")
        rows_by_id[combat_id] = body_ptr
        seen_ptrs.add(body_ptr)
        if len(rows_by_id) > MAX_NAVAL_COMBAT_BODY_BRIDGE_ROWS:
            raise ValueError("naval_combat_body_bridge exceeds max rows")
    return [
        {"combat_id": combat_id, "body_ptr": rows_by_id[combat_id]}
        for combat_id in sorted(rows_by_id)
    ]


def encode_naval_combat_body_bridge(rows: Any) -> str:
    """Encode ``combat_id:0xbody`` rows for ``collector_control.json``."""
    normalized = naval_combat_body_bridge_rows_from_combats(rows)
    return ";".join(
        f"{row['combat_id']}:0x{row['body_ptr']:X}" for row in normalized
    )


def _default_reader_factory() -> Any:
    from memory_reader.combat.combat_enum import CombatReader  # type: ignore

    return CombatReader()


def _naval_bridge_pair(item: Any) -> Tuple[int, int]:
    if isinstance(item, dict):
        combat_raw = item.get("combat_id", item.get("id"))
        ptr_raw = item.get("body_ptr", item.get("body_addr"))
    else:
        try:
            combat_raw, ptr_raw = item
        except Exception as exc:
            raise ValueError("naval_combat_body_bridge row must be pair-like") from exc
    return (
        _bridge_int(combat_raw, "combat_id"),
        _bridge_int(ptr_raw, "body_ptr"),
    )


def _iter_encoded_naval_combat_body_bridge(value: str) -> Iterable[Tuple[str, str]]:
    if not value:
        return []
    pairs = []
    for raw in value.split(";"):
        if not raw:
            raise ValueError("naval_combat_body_bridge contains empty row")
        if ":" not in raw:
            raise ValueError("naval_combat_body_bridge row missing ':'")
        pairs.append(tuple(raw.split(":", 1)))
    return pairs


def _bridge_int(value: Any, name: str) -> int:
    if isinstance(value, bool):
        raise ValueError(f"naval_combat_body_bridge {name} must be int-like")
    if isinstance(value, int):
        return value
    if isinstance(value, str):
        value = value.strip()
        if value.lower().startswith("0x"):
            return int(value, 16)
        return int(value, 10)
    raise ValueError(f"naval_combat_body_bridge {name} must be int-like")


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
    reader_pid: Optional[int],
    requested_pid: int,
    build_ok: Optional[bool],
    reject_stats: Optional[Dict[str, int]] = None,
) -> NavalCombatBodyBridgePreflight:
    rows = list(rows)
    return NavalCombatBodyBridgePreflight(
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
            bridge_supplied=False,
            reject_stats=reject_stats,
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
) -> NavalCombatBodyBridgePreflight:
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
    return NavalCombatBodyBridgePreflight(
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
    reject_stats: Optional[Dict[str, int]] = None,
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
        "bridge_supplied": bool(bridge_supplied),
        "source": "CombatReader.enumerate_naval_combats",
        "runtime_source": "hoi4.exe",
        "address_manifest_policy": "omitted",
        "discovery_mode": "preflight_vtable_enumeration",
        "hot_loop_heap_scan": False,
    }
    if reject_stats:
        diagnostics["reject_stats"] = dict(sorted(reject_stats.items()))
    return diagnostics


def _safe_reject_stats(value: Any) -> Dict[str, int]:
    if not isinstance(value, dict):
        return {}
    out: Dict[str, int] = {}
    for key, raw_count in value.items():
        if not isinstance(key, str) or not re.fullmatch(r"[A-Za-z0-9_:-]{1,64}", key):
            continue
        count = _coerce_optional_int(raw_count)
        if count is not None and count >= 0:
            out[key] = min(count, 1_000_000)
    return out


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
    "NavalCombatBodyBridgePreflight",
    "encode_naval_combat_body_bridge",
    "naval_combat_body_bridge_rows_from_combats",
    "resolve_naval_combat_body_bridge_preflight",
]
