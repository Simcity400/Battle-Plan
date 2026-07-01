"""Preflight live CShip body addresses for the in-process collector.

This reuses the existing CShip registry crack from ``memory_reader.oob_full_live``
but turns it into the same cache-first bridge contract as divisions, armies, and
air wings. The encoded bridge is temporary worker control only; manifests record
counts and diagnostics, never raw addresses.
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


MIN_USABLE_SHIP_BODY_ROWS = 1
MAX_SHIP_BODY_BRIDGE_ROWS = 20000
MAX_SHIP_INSTANCE_ID = 4_000_000

_HERE = Path(__file__).resolve().parent
_REPO = _HERE.parent
for _p in (str(_REPO), str(_REPO / "memory_reader")):
    if _p not in sys.path:
        sys.path.insert(0, _p)


@dataclass(frozen=True)
class ShipBodyBridgePreflight:
    status: str
    row_count: int
    encoded: Optional[str]
    diagnostics: Dict[str, Any]

    @property
    def usable(self) -> bool:
        return self.status == "resolved" and bool(self.encoded)


def resolve_ship_body_bridge_preflight(
    pid: int,
    *,
    reader_factory: Optional[Callable[[], Any]] = None,
    registry_factory: Optional[Callable[[Any], Any]] = None,
    min_rows: int = MIN_USABLE_SHIP_BODY_ROWS,
    cache_path: Optional[Path] = None,
    allow_cold_scan: bool = False,
) -> ShipBodyBridgePreflight:
    """Resolve live CShip body rows for one recorder session.

    Warm path: load cached ``instance_id -> body_ptr`` roots, validate every root
    by CShip vtable + ``{type=51, instance_id}`` handle, and encode the surviving
    rows. Cold path: only when explicitly allowed, run the known global CShip
    vtable scan through ``OobShipRegistry.build_index`` and store the result.
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
        reg_factory = registry_factory or _default_registry_factory
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
            cached = _load_ship_body_cache(
                Path(cache_path),
                identity,
                min_rows=int(min_rows),
            )
            if cached is not None:
                cache_diag["status"] = "loaded"
                cached_rows = _coerce_cached_ship_bodies(cached.get("ships"))
                valid_rows = _validated_cached_ship_bodies(reader, cached_rows)
                rows = ship_body_bridge_rows_from_ships(valid_rows)
                if len(rows) >= int(min_rows):
                    cache_diag["status"] = "hit"
                    encoded = encode_ship_body_bridge(rows)
                    return ShipBodyBridgePreflight(
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
                            source="ship_body_cache",
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
                source="ship_body_cache",
                cache=cache_diag,
            )

        locate_started = time.monotonic()
        registry = reg_factory(reader)
        index = registry.build_index()
        locate_s = time.monotonic() - locate_started
        rows = ship_body_bridge_rows_from_ships(index)
        if len(rows) < int(min_rows):
            return _preflight_status(
                "insufficient_rows",
                rows,
                elapsed_s=time.monotonic() - started,
                cold_locate_s=locate_s,
                reader_pid=reader_pid,
                requested_pid=pid,
                build_ok=True,
                source="OobShipRegistry.build_index",
                cache=cache_diag,
            )
        if cache_path is not None:
            cache_diag["status"] = (
                "stored" if _write_ship_body_cache(
                    Path(cache_path),
                    identity,
                    rows,
                ) else "store_failed"
            )
        encoded = encode_ship_body_bridge(rows)
        return ShipBodyBridgePreflight(
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
                source="OobShipRegistry.build_index",
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


def ship_body_bridge_rows_from_ships(ships: Any) -> List[Dict[str, int]]:
    """Normalize ``instance_id:body_ptr`` CShip bridge rows."""
    if ships is None:
        return []
    raw_items: Iterable[Any]
    if isinstance(ships, str):
        raw_items = _iter_encoded_ship_body_bridge(ships)
    elif isinstance(ships, dict):
        raw_items = ships.items()
    else:
        raw_items = ships

    rows_by_id: Dict[int, int] = {}
    seen_ptrs: set[int] = set()
    for item in raw_items:
        instance_id, body_ptr = _ship_bridge_pair(item)
        if instance_id <= 0 or instance_id > MAX_SHIP_INSTANCE_ID:
            raise ValueError("ship_body_bridge instance_id out of range")
        if body_ptr <= 0 or body_ptr % 8:
            raise ValueError("ship_body_bridge pointer must be nonzero and 8-byte aligned")
        if instance_id in rows_by_id:
            raise ValueError("ship_body_bridge duplicate instance_id")
        if body_ptr in seen_ptrs:
            raise ValueError("ship_body_bridge duplicate pointer")
        rows_by_id[instance_id] = body_ptr
        seen_ptrs.add(body_ptr)
        if len(rows_by_id) > MAX_SHIP_BODY_BRIDGE_ROWS:
            raise ValueError("ship_body_bridge exceeds max rows")
    return [
        {"instance_id": instance_id, "body_ptr": rows_by_id[instance_id]}
        for instance_id in sorted(rows_by_id)
    ]


def encode_ship_body_bridge(rows: Any) -> str:
    """Encode ``instance_id:0xbody`` rows for ``collector_control.json``."""
    normalized = ship_body_bridge_rows_from_ships(rows)
    return ";".join(
        f"{row['instance_id']}:0x{row['body_ptr']:X}" for row in normalized
    )


def _default_reader_factory() -> Any:
    from memory_reader.combat.combat_enum import CombatReader  # type: ignore

    return CombatReader()


def _default_registry_factory(reader: Any) -> Any:
    from memory_reader.oob_full_live import OobShipRegistry  # type: ignore

    return OobShipRegistry(reader)


def _ship_bridge_pair(item: Any) -> Tuple[int, int]:
    if isinstance(item, dict):
        id_raw = item.get("instance_id", item.get("ship_instance_id", item.get("id")))
        ptr_raw = item.get("body_ptr", item.get("body_addr"))
    else:
        try:
            id_raw, ptr_raw = item
        except Exception as exc:
            raise ValueError("ship_body_bridge row must be pair-like") from exc
    return (
        _bridge_int(id_raw, "instance_id"),
        _bridge_int(ptr_raw, "body_ptr"),
    )


def _iter_encoded_ship_body_bridge(value: str) -> Iterable[Tuple[str, str]]:
    if not value:
        return []
    pairs = []
    for raw in value.split(";"):
        if not raw:
            raise ValueError("ship_body_bridge contains empty row")
        if ":" not in raw:
            raise ValueError("ship_body_bridge row missing ':'")
        pairs.append(tuple(raw.split(":", 1)))
    return pairs


def _bridge_int(value: Any, name: str) -> int:
    if isinstance(value, bool):
        raise ValueError(f"ship_body_bridge {name} must be int-like")
    if isinstance(value, int):
        return value
    if isinstance(value, str):
        value = value.strip()
        if value.lower().startswith("0x"):
            return int(value, 16)
        return int(value, 10)
    raise ValueError(f"ship_body_bridge {name} must be int-like")


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
    source: str = "OobShipRegistry.build_index",
    cache: Optional[Dict[str, Any]] = None,
) -> ShipBodyBridgePreflight:
    rows = list(rows)
    return ShipBodyBridgePreflight(
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
) -> ShipBodyBridgePreflight:
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
    return ShipBodyBridgePreflight(
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
    source: str = "OobShipRegistry.build_index",
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
        "discovery_mode": "cache_or_explicit_cship_vtable_scan",
        "hot_loop_heap_scan": False,
    }
    if cache is not None:
        diagnostics["cache"] = {
            "enabled": bool(cache.get("enabled")),
            "status": str(cache.get("status") or "unknown"),
        }
        diagnostics["allow_cold_scan"] = bool(cache.get("allow_cold_scan"))
    return diagnostics


def _validated_cached_ship_bodies(
    reader: Any,
    rows: List[Dict[str, int]],
) -> List[Dict[str, int]]:
    out: List[Dict[str, int]] = []
    for row in rows:
        if _reader_ship_row_valid(reader, row):
            out.append(row)
    return out


def _reader_ship_row_valid(reader: Any, row: Dict[str, int]) -> bool:
    try:
        instance_id = int(row["instance_id"])
        body_ptr = int(row["body_ptr"])
    except (KeyError, TypeError, ValueError):
        return False
    if body_ptr <= 0 or body_ptr % 8:
        return False
    try:
        import oob_full_live as _oob  # type: ignore

        cship_vt = int(reader.module_base) + int(_oob.CSHIP_VT_OFF)
        if reader.u64(body_ptr) != cship_vt:
            return False
        handle = reader.u64(body_ptr + int(_oob.OFF_HANDLE))
        if handle is None:
            return False
        if (int(handle) & 0xFFFFFFFF) != int(_oob.SHIP_TYPE_TAG):
            return False
        return ((int(handle) >> 32) & 0xFFFFFFFF) == instance_id
    except Exception:
        return False


def _coerce_cached_ship_bodies(value: Any) -> List[Dict[str, int]]:
    if not isinstance(value, list):
        return []
    out: List[Dict[str, int]] = []
    seen_ids: set[int] = set()
    seen_ptrs: set[int] = set()
    for item in value:
        try:
            instance_id, body_ptr = _ship_bridge_pair(item)
        except Exception:
            continue
        if not (0 < instance_id <= MAX_SHIP_INSTANCE_ID):
            continue
        if body_ptr <= 0 or body_ptr % 8:
            continue
        if instance_id in seen_ids or body_ptr in seen_ptrs:
            return []
        seen_ids.add(instance_id)
        seen_ptrs.add(body_ptr)
        out.append({"instance_id": instance_id, "body_ptr": body_ptr})
        if len(out) > MAX_SHIP_BODY_BRIDGE_ROWS:
            return []
    return out


def _load_ship_body_cache(
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
    if payload.get("type") != "ship_body_bridge_preflight_cache_v1":
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
    rows = _coerce_cached_ship_bodies(payload.get("ships"))
    if len(rows) < int(min_rows):
        return None
    return payload


def _write_ship_body_cache(
    cache_path: Path,
    identity: _process_identity.ProcessIdentity,
    rows: Iterable[Dict[str, int]],
) -> bool:
    try:
        normalized = ship_body_bridge_rows_from_ships(rows)
    except Exception:
        return False
    payload = {
        "type": "ship_body_bridge_preflight_cache_v1",
        "created_utc": datetime.now(timezone.utc).isoformat(),
        "pid": int(identity.pid),
        "create_time_100ns": identity.create_time_100ns,
        "exe_sha256": identity.exe_sha256,
        "module_base": identity.module_base,
        "row_count": len(normalized),
        "ships": [
            {
                "instance_id": int(row["instance_id"]),
                "body_ptr": int(row["body_ptr"]),
            }
            for row in normalized
        ],
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
    "ShipBodyBridgePreflight",
    "encode_ship_body_bridge",
    "resolve_ship_body_bridge_preflight",
    "ship_body_bridge_rows_from_ships",
]
