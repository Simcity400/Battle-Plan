"""Preflight army-body bridge for the in-process collector.

This resolves live CArmy body addresses once before a collector session and
encodes them for the temporary DLL control file. The DLL then revalidates and
samples each supplied army body for active battle-plan orders without scanning
the heap in the worker loop.
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


MIN_USABLE_ARMY_BODY_ROWS = 1
MAX_ARMY_BODY_BRIDGE_ROWS = 20000
MAX_ARMY_BODY_BRIDGE_ARMY_ID = 2_147_483_647

_HERE = Path(__file__).resolve().parent
_REPO = _HERE.parent
for _p in (str(_REPO), str(_REPO / "memory_reader")):
    if _p not in sys.path:
        sys.path.insert(0, _p)


@dataclass(frozen=True)
class ArmyBodyBridgePreflight:
    status: str
    row_count: int
    encoded: Optional[str]
    diagnostics: Dict[str, Any]

    @property
    def usable(self) -> bool:
        return self.status == "resolved" and bool(self.encoded)


def resolve_army_body_bridge_preflight(
    pid: int,
    *,
    reader_factory: Optional[Callable[[], Any]] = None,
    min_rows: int = MIN_USABLE_ARMY_BODY_ROWS,
    cache_path: Optional[Path] = None,
    allow_cold_scan: bool = False,
) -> ArmyBodyBridgePreflight:
    """Resolve live army-body rows for one recorder session."""
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
            cached = _load_army_body_cache(
                Path(cache_path),
                identity,
                min_rows=int(min_rows),
            )
            if cached is not None:
                cache_diag["status"] = "loaded"
                cached_armies = _coerce_cached_armies(cached.get("armies"))
                valid_armies = _validated_cached_armies(reader, cached_armies)
                cache_diag["cached_row_count"] = len(cached_armies)
                cache_diag["valid_row_count"] = len(valid_armies)
                cache_diag["stale_row_count"] = (
                    len(cached_armies) - len(valid_armies)
                )
                rows = army_body_bridge_rows_from_armies(valid_armies)
                if len(rows) >= int(min_rows):
                    cache_diag["status"] = "hit"
                    encoded = encode_army_body_bridge(rows)
                    return ArmyBodyBridgePreflight(
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
                            source="army_body_cache",
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
                source="army_body_cache",
                cache=cache_diag,
            )

        locate_started = time.monotonic()
        armies = reader.enumerate_armies()
        locate_s = time.monotonic() - locate_started
        rows = army_body_bridge_rows_from_armies(armies)
        if len(rows) < int(min_rows):
            return _preflight_status(
                "insufficient_rows",
                rows,
                elapsed_s=time.monotonic() - started,
                cold_locate_s=locate_s,
                reader_pid=reader_pid,
                requested_pid=pid,
                build_ok=True,
                source="construction_orders_live.enumerate_armies",
                cache=cache_diag,
            )
        if cache_path is not None:
            cache_diag["status"] = (
                "stored" if _write_army_body_cache(
                    Path(cache_path),
                    identity,
                    rows,
                ) else "store_failed"
            )
        encoded = encode_army_body_bridge(rows)
        return ArmyBodyBridgePreflight(
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
                source="construction_orders_live.enumerate_armies",
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


def army_body_bridge_rows_from_armies(armies: Any) -> List[Dict[str, int]]:
    """Normalize ``{army_id: body_ptr}`` rows from the live army reader."""
    if armies is None:
        return []
    raw_items: Iterable[Any]
    if isinstance(armies, str):
        raw_items = _iter_encoded_army_body_bridge(armies)
    elif isinstance(armies, dict):
        raw_items = armies.items()
    else:
        raw_items = armies

    rows_by_id: Dict[int, int] = {}
    seen_ptrs: set[int] = set()
    for item in raw_items:
        army_id, body_ptr = _army_bridge_pair(item)
        if army_id <= 0 or army_id > MAX_ARMY_BODY_BRIDGE_ARMY_ID:
            raise ValueError("army_body_bridge army_id out of range")
        if body_ptr <= 0 or body_ptr % 8:
            raise ValueError("army_body_bridge pointer must be nonzero and 8-byte aligned")
        if army_id in rows_by_id:
            raise ValueError("army_body_bridge duplicate army_id")
        if body_ptr in seen_ptrs:
            raise ValueError("army_body_bridge duplicate pointer")
        rows_by_id[army_id] = body_ptr
        seen_ptrs.add(body_ptr)
        if len(rows_by_id) > MAX_ARMY_BODY_BRIDGE_ROWS:
            raise ValueError("army_body_bridge exceeds max rows")
    return [
        {"army_id": army_id, "body_ptr": rows_by_id[army_id]}
        for army_id in sorted(rows_by_id)
    ]


def encode_army_body_bridge(rows: Any) -> str:
    """Encode ``army_id:0xbody`` rows for ``collector_control.json``."""
    normalized = army_body_bridge_rows_from_armies(rows)
    return ";".join(
        f"{row['army_id']}:0x{row['body_ptr']:X}" for row in normalized
    )


def _default_reader_factory(pid: int) -> Any:
    import construction_orders_live as orders  # type: ignore
    import winmem  # type: ignore

    class _ArmyReader:
        def __init__(self) -> None:
            self._orders = orders
            self._process = winmem.Process(pid, orders.PROC_NAME)
            self.pid = self._process.pid
            self.module_base = self._process.module_base
            self.module_path = self._process.module_path

        def fingerprint(self) -> Dict[str, Any]:
            return self._process.fingerprint()

        def verify_build(self) -> Tuple[bool, Optional[str]]:
            sha = self.fingerprint().get("sha256")
            return sha == self._orders.EXPECTED_SHA256, sha

        def enumerate_armies(self) -> Dict[int, int]:
            return self._orders.enumerate_armies(self._process)

        def validate_armies(self, armies: Dict[int, int]) -> bool:
            return bool(self._orders.validate_armies(self._process, armies))

        def close(self) -> None:
            self._process.close()

    return _ArmyReader()


def _army_bridge_pair(item: Any) -> Tuple[int, int]:
    if isinstance(item, dict):
        army_raw = item.get("army_id", item.get("id"))
        ptr_raw = item.get("body_ptr", item.get("body_addr"))
    else:
        try:
            army_raw, ptr_raw = item
        except Exception as exc:
            raise ValueError("army_body_bridge row must be pair-like") from exc
    return (
        _bridge_int(army_raw, "army_id"),
        _bridge_int(ptr_raw, "body_ptr"),
    )


def _iter_encoded_army_body_bridge(value: str) -> Iterable[Tuple[str, str]]:
    if not value:
        return []
    pairs = []
    for raw in value.split(";"):
        if not raw:
            raise ValueError("army_body_bridge contains empty row")
        if ":" not in raw:
            raise ValueError("army_body_bridge row missing ':'")
        pairs.append(tuple(raw.split(":", 1)))
    return pairs


def _bridge_int(value: Any, name: str) -> int:
    if isinstance(value, bool):
        raise ValueError(f"army_body_bridge {name} must be int-like")
    if isinstance(value, int):
        return value
    if isinstance(value, str):
        value = value.strip()
        if value.lower().startswith("0x"):
            return int(value, 16)
        return int(value, 10)
    raise ValueError(f"army_body_bridge {name} must be int-like")


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
    source: str = "construction_orders_live.enumerate_armies",
    cache: Optional[Dict[str, Any]] = None,
) -> ArmyBodyBridgePreflight:
    rows = list(rows)
    return ArmyBodyBridgePreflight(
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
) -> ArmyBodyBridgePreflight:
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
    return ArmyBodyBridgePreflight(
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
    source: str = "construction_orders_live.enumerate_armies",
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
        for key in ("cached_row_count", "valid_row_count", "stale_row_count"):
            if key in cache:
                try:
                    diagnostics["cache"][key] = int(cache[key])
                except (TypeError, ValueError):
                    pass
        diagnostics["allow_cold_scan"] = bool(cache.get("allow_cold_scan"))
    return diagnostics


def _validated_cached_armies(
    reader: Any,
    armies: Dict[int, int],
) -> Dict[int, int]:
    if not armies:
        return {}
    validate = getattr(reader, "validate_armies", None)
    if not callable(validate):
        return {}
    try:
        if validate(armies):
            return dict(sorted(armies.items()))
    except Exception:
        pass
    valid: Dict[int, int] = {}
    for army_id, body_ptr in sorted(armies.items()):
        try:
            if validate({army_id: body_ptr}):
                valid[army_id] = body_ptr
        except Exception:
            continue
    return valid


def _coerce_cached_armies(value: Any) -> Dict[int, int]:
    if not isinstance(value, dict):
        return {}
    out: Dict[int, int] = {}
    seen_ptrs: set[int] = set()
    for raw_id, raw_ptr in value.items():
        if isinstance(raw_id, bool) or isinstance(raw_ptr, bool):
            continue
        try:
            army_id = int(raw_id)
            body_ptr = int(raw_ptr)
        except (TypeError, ValueError):
            continue
        if army_id <= 0 or army_id > MAX_ARMY_BODY_BRIDGE_ARMY_ID:
            continue
        if body_ptr <= 0 or body_ptr % 8:
            continue
        if body_ptr in seen_ptrs:
            return {}
        out[army_id] = body_ptr
        seen_ptrs.add(body_ptr)
        if len(out) > MAX_ARMY_BODY_BRIDGE_ROWS:
            return {}
    return out


def _load_army_body_cache(
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
    if payload.get("type") != "army_body_bridge_preflight_cache_v1":
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
    armies = _coerce_cached_armies(payload.get("armies"))
    if len(armies) < int(min_rows):
        return None
    return payload


def _write_army_body_cache(
    cache_path: Path,
    identity: _process_identity.ProcessIdentity,
    rows: Iterable[Dict[str, int]],
) -> bool:
    try:
        normalized = army_body_bridge_rows_from_armies(rows)
    except Exception:
        return False
    payload = {
        "type": "army_body_bridge_preflight_cache_v1",
        "created_utc": datetime.now(timezone.utc).isoformat(),
        "pid": int(identity.pid),
        "create_time_100ns": identity.create_time_100ns,
        "exe_sha256": identity.exe_sha256,
        "module_base": identity.module_base,
        "row_count": len(normalized),
        "armies": {
            str(int(row["army_id"])): int(row["body_ptr"])
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
    "ArmyBodyBridgePreflight",
    "army_body_bridge_rows_from_armies",
    "encode_army_body_bridge",
    "resolve_army_body_bridge_preflight",
]
