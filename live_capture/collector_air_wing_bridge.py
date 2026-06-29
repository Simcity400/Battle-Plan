"""Preflight air-wing body bridge for the in-process collector.

This resolves live air-wing body addresses before a collector session and
encodes them for the temporary DLL control file. The DLL revalidates each
supplied body and samples count/mission/effective-plane telemetry without
heap-scanning inside the worker loop.
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


MIN_USABLE_AIR_WING_BODY_ROWS = 1
MAX_AIR_WING_BODY_BRIDGE_ROWS = 20000
MAX_AIR_WING_ID = 2_147_483_647
MAX_OWNER_TAG_DB_ID = 1024

_HERE = Path(__file__).resolve().parent
_REPO = _HERE.parent
for _p in (str(_REPO), str(_REPO / "memory_reader")):
    if _p not in sys.path:
        sys.path.insert(0, _p)


@dataclass(frozen=True)
class AirWingBodyBridgePreflight:
    status: str
    row_count: int
    encoded: Optional[str]
    diagnostics: Dict[str, Any]

    @property
    def usable(self) -> bool:
        return self.status == "resolved" and bool(self.encoded)


def resolve_air_wing_body_bridge_preflight(
    pid: int,
    *,
    reader_factory: Optional[Callable[[], Any]] = None,
    min_rows: int = MIN_USABLE_AIR_WING_BODY_ROWS,
    cache_path: Optional[Path] = None,
    allow_cold_scan: bool = False,
) -> AirWingBodyBridgePreflight:
    """Resolve live air-wing rows for one recorder session."""
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
            cached = _load_air_wing_body_cache(
                Path(cache_path),
                identity,
                min_rows=int(min_rows),
            )
            if cached is not None:
                cache_diag["status"] = "loaded"
                cached_rows = _coerce_cached_air_wings(cached.get("air_wings"))
                valid_rows = _validated_cached_air_wings(reader, cached_rows)
                rows = air_wing_body_bridge_rows_from_wings(valid_rows)
                if len(rows) >= int(min_rows):
                    cache_diag["status"] = "hit"
                    encoded = encode_air_wing_body_bridge(rows)
                    return AirWingBodyBridgePreflight(
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
                            source="air_wing_body_cache",
                            cache=cache_diag,
                            reject_stats=_safe_reject_stats(
                                getattr(reader, "reject_stats", None)),
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
                source="air_wing_body_cache",
                cache=cache_diag,
            )

        locate_started = time.monotonic()
        wings = reader.enumerate_air_wings()
        locate_s = time.monotonic() - locate_started
        rows = air_wing_body_bridge_rows_from_wings(wings)
        if len(rows) < int(min_rows):
            return _preflight_status(
                "insufficient_rows",
                rows,
                elapsed_s=time.monotonic() - started,
                cold_locate_s=locate_s,
                reader_pid=reader_pid,
                requested_pid=pid,
                build_ok=True,
                source="NavalAirReader.enumerate_air_wings",
                cache=cache_diag,
                reject_stats=_safe_reject_stats(getattr(reader, "reject_stats", None)),
            )
        if cache_path is not None:
            cache_diag["status"] = (
                "stored" if _write_air_wing_body_cache(
                    Path(cache_path),
                    identity,
                    rows,
                ) else "store_failed"
            )
        encoded = encode_air_wing_body_bridge(rows)
        return AirWingBodyBridgePreflight(
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
                source="NavalAirReader.enumerate_air_wings",
                cache=cache_diag,
                reject_stats=_safe_reject_stats(getattr(reader, "reject_stats", None)),
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


def air_wing_body_bridge_rows_from_wings(wings: Any) -> List[Dict[str, int]]:
    """Normalize ``owner_tag_db_id:wing_id:body_ptr`` rows."""
    if wings is None:
        return []
    raw_items: Iterable[Any]
    if isinstance(wings, str):
        raw_items = _iter_encoded_air_wing_body_bridge(wings)
    elif isinstance(wings, dict):
        raw_items = wings.values()
    else:
        raw_items = wings

    rows_by_key: Dict[Tuple[int, int], int] = {}
    seen_ptrs: set[int] = set()
    for item in raw_items:
        owner_tag_db_id, wing_id, body_ptr = _air_wing_bridge_row(item)
        if owner_tag_db_id <= 0 or owner_tag_db_id > MAX_OWNER_TAG_DB_ID:
            raise ValueError("air_wing_body_bridge owner_tag_db_id out of range")
        if wing_id <= 0 or wing_id > MAX_AIR_WING_ID:
            raise ValueError("air_wing_body_bridge wing_id out of range")
        if body_ptr <= 0 or body_ptr % 8:
            raise ValueError("air_wing_body_bridge pointer must be nonzero and 8-byte aligned")
        key = (owner_tag_db_id, wing_id)
        if key in rows_by_key:
            raise ValueError("air_wing_body_bridge duplicate owner/wing key")
        if body_ptr in seen_ptrs:
            raise ValueError("air_wing_body_bridge duplicate pointer")
        rows_by_key[key] = body_ptr
        seen_ptrs.add(body_ptr)
        if len(rows_by_key) > MAX_AIR_WING_BODY_BRIDGE_ROWS:
            raise ValueError("air_wing_body_bridge exceeds max rows")
    return [
        {
            "owner_tag_db_id": owner,
            "wing_id": wing_id,
            "body_ptr": rows_by_key[(owner, wing_id)],
        }
        for owner, wing_id in sorted(rows_by_key)
    ]


def encode_air_wing_body_bridge(rows: Any) -> str:
    """Encode ``owner_tag_db_id:wing_id:0xbody`` rows for ``collector_control.json``."""
    normalized = air_wing_body_bridge_rows_from_wings(rows)
    return ";".join(
        f"{row['owner_tag_db_id']}:{row['wing_id']}:0x{row['body_ptr']:X}"
        for row in normalized
    )


def _default_reader_factory() -> Any:
    from memory_reader.naval_air.naval_air_reader import NavalAirReader  # type: ignore

    return NavalAirReader()


def _air_wing_bridge_row(item: Any) -> Tuple[int, int, int]:
    if isinstance(item, dict):
        owner_raw = item.get("owner_tag_db_id", item.get("owner_tagid"))
        wing_raw = item.get("wing_id", item.get("id"))
        ptr_raw = item.get("body_ptr", item.get("body_addr"))
    else:
        try:
            owner_raw, wing_raw, ptr_raw = item
        except Exception as exc:
            raise ValueError("air_wing_body_bridge row must be triple-like") from exc
    return (
        _bridge_int(owner_raw, "owner_tag_db_id"),
        _bridge_int(wing_raw, "wing_id"),
        _bridge_int(ptr_raw, "body_ptr"),
    )


def _iter_encoded_air_wing_body_bridge(value: str) -> Iterable[Tuple[str, str, str]]:
    if not value:
        return []
    triples = []
    for raw in value.split(";"):
        if not raw:
            raise ValueError("air_wing_body_bridge contains empty row")
        parts = raw.split(":")
        if len(parts) != 3:
            raise ValueError("air_wing_body_bridge row must have owner:wing:ptr")
        triples.append((parts[0], parts[1], parts[2]))
    return triples


def _bridge_int(value: Any, name: str) -> int:
    if isinstance(value, bool):
        raise ValueError(f"air_wing_body_bridge {name} must be int-like")
    if isinstance(value, int):
        return value
    if isinstance(value, str):
        value = value.strip()
        if value.lower().startswith("0x"):
            return int(value, 16)
        return int(value, 10)
    raise ValueError(f"air_wing_body_bridge {name} must be int-like")


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
    source: str = "NavalAirReader.enumerate_air_wings",
    cache: Optional[Dict[str, Any]] = None,
    reject_stats: Optional[Dict[str, int]] = None,
) -> AirWingBodyBridgePreflight:
    rows = list(rows)
    return AirWingBodyBridgePreflight(
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
            reject_stats=reject_stats,
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
) -> AirWingBodyBridgePreflight:
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
    return AirWingBodyBridgePreflight(
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
    source: str = "NavalAirReader.enumerate_air_wings",
    cache: Optional[Dict[str, Any]] = None,
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
        "cold_locate_s": float(cold_locate_s) if cold_locate_s is not None else None,
        "bridge_supplied": bool(bridge_supplied),
        "source": source,
        "runtime_source": "hoi4.exe",
        "address_manifest_policy": "omitted",
        "discovery_mode": "cache_or_preflight_vtable_enumeration",
        "hot_loop_heap_scan": False,
    }
    if cache is not None:
        diagnostics["cache"] = {
            "enabled": bool(cache.get("enabled")),
            "status": str(cache.get("status") or "unknown"),
        }
        diagnostics["allow_cold_scan"] = bool(cache.get("allow_cold_scan"))
    if reject_stats:
        diagnostics["reject_stats"] = dict(sorted(reject_stats.items()))
    return diagnostics


def _validated_cached_air_wings(
    reader: Any,
    rows: List[Dict[str, int]],
) -> List[Dict[str, int]]:
    out: List[Dict[str, int]] = []
    for row in rows:
        if _reader_air_wing_row_valid(reader, row):
            out.append(row)
    return out


def _reader_air_wing_row_valid(reader: Any, row: Dict[str, int]) -> bool:
    try:
        owner_tag_db_id = int(row["owner_tag_db_id"])
        wing_id = int(row["wing_id"])
        body_ptr = int(row["body_ptr"])
    except (KeyError, TypeError, ValueError):
        return False
    if body_ptr <= 0 or body_ptr % 8:
        return False
    try:
        if reader.u64(body_ptr) != getattr(reader, "wing_vt", None):
            return False
        if reader.u32(body_ptr + 0x08) != 69:
            return False
        if reader.u32(body_ptr + 0x0C) != wing_id:
            return False
        owner = reader.wing_owner_tagid(body_ptr)
        return owner == owner_tag_db_id
    except Exception:
        return False


def _coerce_cached_air_wings(value: Any) -> List[Dict[str, int]]:
    if not isinstance(value, list):
        return []
    out: List[Dict[str, int]] = []
    seen_keys: set[Tuple[int, int]] = set()
    seen_ptrs: set[int] = set()
    for item in value:
        try:
            owner_tag_db_id, wing_id, body_ptr = _air_wing_bridge_row(item)
        except Exception:
            continue
        if not (0 < owner_tag_db_id <= MAX_OWNER_TAG_DB_ID):
            continue
        if not (0 < wing_id <= MAX_AIR_WING_ID):
            continue
        if body_ptr <= 0 or body_ptr % 8:
            continue
        key = (owner_tag_db_id, wing_id)
        if key in seen_keys or body_ptr in seen_ptrs:
            return []
        seen_keys.add(key)
        seen_ptrs.add(body_ptr)
        out.append({
            "owner_tag_db_id": owner_tag_db_id,
            "wing_id": wing_id,
            "body_ptr": body_ptr,
        })
        if len(out) > MAX_AIR_WING_BODY_BRIDGE_ROWS:
            return []
    return out


def _load_air_wing_body_cache(
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
    if payload.get("type") != "air_wing_body_bridge_preflight_cache_v1":
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
    rows = _coerce_cached_air_wings(payload.get("air_wings"))
    if len(rows) < int(min_rows):
        return None
    return payload


def _write_air_wing_body_cache(
    cache_path: Path,
    identity: _process_identity.ProcessIdentity,
    rows: Iterable[Dict[str, int]],
) -> bool:
    try:
        normalized = air_wing_body_bridge_rows_from_wings(rows)
    except Exception:
        return False
    payload = {
        "type": "air_wing_body_bridge_preflight_cache_v1",
        "created_utc": datetime.now(timezone.utc).isoformat(),
        "pid": int(identity.pid),
        "create_time_100ns": identity.create_time_100ns,
        "exe_sha256": identity.exe_sha256,
        "module_base": identity.module_base,
        "row_count": len(normalized),
        "air_wings": [
            {
                "owner_tag_db_id": int(row["owner_tag_db_id"]),
                "wing_id": int(row["wing_id"]),
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
    "AirWingBodyBridgePreflight",
    "air_wing_body_bridge_rows_from_wings",
    "encode_air_wing_body_bridge",
    "resolve_air_wing_body_bridge_preflight",
]
