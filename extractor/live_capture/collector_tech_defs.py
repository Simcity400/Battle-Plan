"""Cache-first tech-definition filter for live research-slot enrichment."""
from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import sys
import time
from typing import Any, Callable, Dict, Iterable, Optional, Tuple

from . import process_identity as _process_identity


MAX_TECH_DEF_FILTER_ROWS = 8192
MAX_TECH_DEF_FILTER_PAYLOAD_BYTES = 512 * 1024
MAX_TECH_HANDLE = 10_000_000
MAX_TECH_TOKEN_LENGTH = 160

_HERE = Path(__file__).resolve().parent
_REPO = _HERE.parent
for _p in (str(_REPO), str(_REPO / "memory_reader")):
    if _p not in sys.path:
        sys.path.insert(0, _p)


@dataclass(frozen=True)
class TechDefFilterPreflight:
    status: str
    row_count: int
    encoded: Optional[str]
    diagnostics: Dict[str, Any]

    @property
    def usable(self) -> bool:
        return self.status == "resolved" and bool(self.encoded)


def resolve_tech_def_filter_preflight(
    pid: int,
    *,
    reader_factory: Optional[Callable[[], Any]] = None,
    cache_path: Optional[Path] = None,
    allow_cold_scan: bool = False,
) -> TechDefFilterPreflight:
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
                {},
                elapsed_s=time.monotonic() - started,
                reader_pid=reader_pid,
                requested_pid=pid,
                build_ok=build_ok,
                cache=cache_diag,
            )
        if not build_ok:
            return _preflight_status(
                "build_mismatch",
                {},
                elapsed_s=time.monotonic() - started,
                reader_pid=reader_pid,
                requested_pid=pid,
                build_ok=False,
                cache=cache_diag,
            )

        identity = _process_identity.identify(reader)
        if cache_path is not None:
            cached = _load_tech_def_cache(Path(cache_path), identity)
            if cached is not None:
                cache_diag["status"] = "loaded"
                rows = _coerce_tech_def_rows(cached.get("tech_defs"))
                if rows:
                    cache_diag["status"] = "hit"
                    encoded = encode_tech_def_filter(rows)
                    return TechDefFilterPreflight(
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
                            source="tech_def_cache",
                            cache=cache_diag,
                        ),
                    )
                cache_diag["status"] = "stale"

        if not allow_cold_scan:
            return _preflight_status(
                "cold_scan_disabled",
                {},
                elapsed_s=time.monotonic() - started,
                reader_pid=reader_pid,
                requested_pid=pid,
                build_ok=True,
                source="tech_def_cache",
                cache=cache_diag,
            )

        locate_started = time.monotonic()
        rows = _coerce_tech_def_rows(reader.build_tech_defs())
        locate_s = time.monotonic() - locate_started
        if not rows:
            return _preflight_status(
                "empty",
                rows,
                elapsed_s=time.monotonic() - started,
                cold_locate_s=locate_s,
                reader_pid=reader_pid,
                requested_pid=pid,
                build_ok=True,
                source="defnames_live.build_tech_registry",
                cache=cache_diag,
            )
        if cache_path is not None:
            cache_diag["status"] = (
                "stored" if _write_tech_def_cache(Path(cache_path), identity, rows)
                else "store_failed"
            )
        encoded = encode_tech_def_filter(rows)
        return TechDefFilterPreflight(
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
                source="defnames_live.build_tech_registry",
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


def encode_tech_def_filter(rows: Any) -> str:
    normalized = _coerce_tech_def_rows(rows)
    encoded = ";".join(f"{handle}={token}" for handle, token in sorted(normalized.items()))
    if len(encoded.encode("utf-8")) > MAX_TECH_DEF_FILTER_PAYLOAD_BYTES:
        raise ValueError("tech_def_filter payload too large")
    return encoded


def tech_def_filter_metadata(encoded: Optional[str]) -> Dict[str, Any]:
    if not encoded:
        return {
            "tech_def_filter_supplied": False,
            "tech_def_filter_rows": 0,
            "tech_def_filter_sha256": None,
        }
    return {
        "tech_def_filter_supplied": True,
        "tech_def_filter_rows": _encoded_row_count(encoded),
        "tech_def_filter_sha256": hashlib.sha256(encoded.encode("utf-8")).hexdigest(),
    }


def _default_reader_factory(pid: int) -> Any:
    import defnames_live  # type: ignore
    import winmem  # type: ignore

    class _TechDefReader:
        def __init__(self) -> None:
            self.proc = winmem.Process(pid)
            self.pid = self.proc.pid
            self.module_base = self.proc.module_base
            self.module_path = getattr(self.proc, "module_path", None)

        def fingerprint(self) -> Dict[str, Any]:
            return self.proc.fingerprint()

        def verify_build(self) -> tuple[bool, Optional[str]]:
            got = self.fingerprint().get("sha256")
            return (got == defnames_live.EXPECTED_SHA, got)

        def build_tech_defs(self) -> Dict[int, str]:
            return defnames_live.build_tech_registry(self.proc)

        def close(self) -> None:
            self.proc.close()

    return _TechDefReader()


def _coerce_tech_def_rows(rows: Any) -> Dict[int, str]:
    if rows is None:
        return {}
    if isinstance(rows, str):
        raw_items: Iterable[Any] = _iter_encoded_rows(rows)
    elif isinstance(rows, dict):
        raw_items = rows.items()
    else:
        raw_items = rows
    normalized: Dict[int, str] = {}
    for item in raw_items:
        try:
            handle_raw, token_raw = item
        except Exception as exc:
            raise ValueError("tech_def_filter row must be pair-like") from exc
        handle = _parse_int(handle_raw)
        if handle <= 0 or handle > MAX_TECH_HANDLE:
            raise ValueError("tech_def_filter handle out of range")
        if not isinstance(token_raw, str):
            raise ValueError("tech_def_filter token must be str")
        token = token_raw.strip()
        if not _valid_script_token(token):
            raise ValueError("tech_def_filter token not script token")
        previous = normalized.get(handle)
        if previous is not None and previous != token:
            raise ValueError("tech_def_filter duplicate handle with different token")
        normalized[handle] = token
        if len(normalized) > MAX_TECH_DEF_FILTER_ROWS:
            raise ValueError("tech_def_filter too many rows")
    return normalized


def _iter_encoded_rows(value: str) -> Iterable[Tuple[int, str]]:
    if not value:
        return []
    rows = []
    for raw in value.split(";"):
        if not raw:
            raise ValueError("tech_def_filter contains empty row")
        if "=" not in raw:
            raise ValueError("tech_def_filter row missing '='")
        handle, token = raw.split("=", 1)
        rows.append((int(handle, 0), token))
    return rows


def _parse_int(value: Any) -> int:
    if isinstance(value, int) and not isinstance(value, bool):
        return value
    if isinstance(value, str):
        return int(value, 0)
    raise ValueError("integer expected")


def _load_tech_def_cache(
    path: Path,
    identity: _process_identity.ProcessIdentity,
) -> Optional[Dict[str, Any]]:
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None
    if payload.get("cache_type") != "tech_def_filter_preflight_cache_v1":
        return None
    if int(payload.get("pid", -1)) != identity.pid:
        return None
    if payload.get("create_time_100ns") != identity.create_time_100ns:
        return None
    if payload.get("exe_sha256") != identity.exe_sha256:
        return None
    if payload.get("module_base") != identity.module_base:
        return None
    return payload


def _write_tech_def_cache(
    path: Path,
    identity: _process_identity.ProcessIdentity,
    rows: Dict[int, str],
) -> bool:
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        payload = {
            "cache_type": "tech_def_filter_preflight_cache_v1",
            "created_utc": datetime.now(timezone.utc).isoformat(),
            "pid": identity.pid,
            "create_time_100ns": identity.create_time_100ns,
            "exe_sha256": identity.exe_sha256,
            "module_base": identity.module_base,
            "row_count": len(rows),
            "tech_defs": {str(k): rows[k] for k in sorted(rows)},
        }
        tmp = path.with_suffix(path.suffix + ".tmp")
        tmp.write_text(json.dumps(payload, sort_keys=True) + "\n", encoding="utf-8")
        os.replace(tmp, path)
        return True
    except OSError:
        return False


def _diagnostics(
    status: str,
    rows: Dict[int, str],
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
    encoded = encode_tech_def_filter(rows) if rows else ""
    diag: Dict[str, Any] = {
        "status": status,
        "row_count": len(rows),
        "elapsed_s": elapsed_s,
        "hot_loop_heap_scan": False,
        "bridge_supplied": bridge_supplied,
        "allow_cold_scan": bool((cache or {}).get("allow_cold_scan", False)),
        "address_manifest_policy": "omitted",
        "sha256": hashlib.sha256(encoded.encode("utf-8")).hexdigest()
        if encoded else None,
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
    rows: Dict[int, str],
    *,
    elapsed_s: float,
    cold_locate_s: Optional[float] = None,
    reader_pid: Optional[int] = None,
    requested_pid: Optional[int] = None,
    build_ok: Optional[bool] = None,
    source: Optional[str] = None,
    cache: Optional[Dict[str, Any]] = None,
) -> TechDefFilterPreflight:
    return TechDefFilterPreflight(
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
) -> TechDefFilterPreflight:
    return TechDefFilterPreflight(
        status=status,
        row_count=0,
        encoded=None,
        diagnostics=_diagnostics(
            status,
            {},
            elapsed_s=elapsed_s,
            cold_locate_s=cold_locate_s,
            reader_pid=reader_pid,
            requested_pid=requested_pid,
            build_ok=build_ok,
            cache=cache,
            error=_safe_error_message(exc),
        ),
    )


def _valid_script_token(value: str) -> bool:
    if not (0 < len(value) <= MAX_TECH_TOKEN_LENGTH):
        return False
    return all(
        ("A" <= ch <= "Z")
        or ("a" <= ch <= "z")
        or ("0" <= ch <= "9")
        or ch in "_-.:"
        for ch in value
    )


def _encoded_row_count(encoded: str) -> int:
    if not encoded:
        return 0
    return len([row for row in encoded.split(";") if row])


def _coerce_optional_int(value: Any) -> Optional[int]:
    try:
        return int(value) if value is not None else None
    except (TypeError, ValueError):
        return None


_HEX_RE = re.compile(r"0x[0-9a-fA-F]+")


def _safe_error_message(exc: Exception) -> str:
    return _HEX_RE.sub("0x<redacted>", str(exc)).replace("\r", " ").replace("\n", " ")


__all__ = [
    "TechDefFilterPreflight",
    "encode_tech_def_filter",
    "resolve_tech_def_filter_preflight",
    "tech_def_filter_metadata",
]
