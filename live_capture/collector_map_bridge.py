"""Preflight map-control roots for the in-process collector.

This module is the state/province-control counterpart to
``collector_country_bridge``. Discovery may scan memory, so it is pre-recording
only. Cache hits validate cached state-object roots for the same ``hoi4.exe``
instance and then re-read owner/controller fields live.

No saves are read. Diagnostics omit raw addresses; cache files are runtime
artifacts under the session output tree.
"""
from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import sys
from typing import Any, Dict, Iterable, List, Optional

from . import process_identity as _process_identity
from .collector_country_bridge import tag_table_from_country_database_root


_HERE = Path(__file__).resolve().parent
_REPO = _HERE.parent
_MR = _REPO / "memory_reader"
for _p in (str(_REPO), str(_MR)):
    if _p not in sys.path:
        sys.path.insert(0, _p)

MIN_USABLE_STATE_ROWS = 1000


@dataclass(frozen=True)
class StateControlPreflight:
    status: str
    row_count: int
    states: Dict[str, Dict[str, Any]]
    diagnostics: Dict[str, Any]

    @property
    def usable(self) -> bool:
        return self.status == "resolved" and self.row_count >= MIN_USABLE_STATE_ROWS


def resolve_state_control_preflight(
    pid: int,
    *,
    min_rows: int = MIN_USABLE_STATE_ROWS,
    cache_path: Optional[Path] = None,
) -> StateControlPreflight:
    """Resolve live state owner/controller rows for one recorder session."""
    try:
        import winmem  # type: ignore
        import state_detail_live as sdl  # type: ignore
        from live_layers import tag_table as tt  # type: ignore
    except Exception as exc:
        return _state_error("import_failed", exc)

    cache_diag: Dict[str, Any] = {
        "enabled": bool(cache_path),
        "status": "not_requested" if cache_path is None else "miss",
        "tag_table_source": "country_database_root",
    }
    try:
        with winmem.Process(int(pid), "hoi4.exe") as proc:
            identity = _process_identity.identify(proc)
            table = tag_table_from_country_database_root(proc, tt)
            if table is None:
                return _state_status(
                    "tag_table_unavailable",
                    {},
                    cache=cache_diag,
                )
            if cache_path is not None:
                cached = _load_state_roots_cache(cache_path, identity)
                if cached is not None:
                    roots = _coerce_state_roots(cached.get("states"))
                    if _state_roots_valid(proc, roots, sdl):
                        states = state_rows_from_state_roots(proc, table, roots, sdl)
                        if len(states) >= int(min_rows):
                            cache_diag["status"] = "hit"
                            return _state_status(
                                "resolved",
                                states,
                                source="state_roots_cache",
                                cache=cache_diag,
                            )
                    cache_diag["status"] = "stale"

            layer = sdl.StateDetailLive.resolve(proc)
            roots = getattr(layer, "states", None) if layer is not None else None
            roots = _coerce_state_roots(roots)
            if not _state_roots_valid(proc, roots, sdl):
                return _state_status(
                    "state_roots_unavailable",
                    {},
                    cache=cache_diag,
                )
            states = state_rows_from_state_roots(proc, table, roots, sdl)
            if len(states) < int(min_rows):
                return _state_status(
                    "insufficient_rows",
                    states,
                    cache=cache_diag,
                )
            if cache_path is not None:
                cache_diag["status"] = (
                    "stored" if _write_state_roots_cache(cache_path, identity, roots)
                    else "store_failed"
                )
            return _state_status(
                "resolved",
                states,
                source="state_roots",
                cache=cache_diag,
            )
    except Exception as exc:
        return _state_error("resolve_failed", exc, cache=cache_diag)


def state_rows_from_state_roots(
    proc: Any,
    table: Any,
    roots: Dict[int, int],
    state_module: Any,
) -> Dict[str, Dict[str, Any]]:
    """Return ``states`` payload rows from validated state object roots."""
    states: Dict[str, Dict[str, Any]] = {}
    for sid in sorted(roots):
        obj = int(roots[sid])
        try:
            if int(proc.u64(obj)) != int(proc.module_base) + int(state_module.STATE_VTABLE_REL):
                continue
            if int(proc.u32(obj + state_module.OFF_SID)) != int(sid):
                continue
            owner_id = int(proc.u32(obj + state_module.OFF_OWNER))
            controller_id = int(proc.u32(obj + state_module.OFF_CTRL))
        except Exception:
            continue
        states[str(int(sid))] = {
            "owner": table.tag(owner_id) if owner_id else None,
            "controller": table.tag(controller_id) if controller_id else None,
        }
    return states


def _state_roots_valid(proc: Any, roots: Dict[int, int], state_module: Any) -> bool:
    if len(roots) < MIN_USABLE_STATE_ROWS:
        return False
    vt_expected = int(proc.module_base) + int(state_module.STATE_VTABLE_REL)
    try:
        for sid, obj in roots.items():
            if int(proc.u64(int(obj))) != vt_expected:
                return False
            if int(proc.u32(int(obj) + state_module.OFF_SID)) != int(sid):
                return False
    except Exception:
        return False
    return True


def _coerce_state_roots(value: Any) -> Dict[int, int]:
    if not isinstance(value, dict):
        return {}
    out: Dict[int, int] = {}
    for raw_sid, raw_obj in value.items():
        if isinstance(raw_sid, bool) or isinstance(raw_obj, bool):
            continue
        try:
            sid = int(raw_sid)
            obj = int(raw_obj)
        except (TypeError, ValueError):
            continue
        if sid > 0 and obj > 0 and obj % 8 == 0:
            out[sid] = obj
    return out


def _state_status(
    status: str,
    states: Dict[str, Dict[str, Any]],
    *,
    source: str = "state_roots",
    cache: Optional[Dict[str, Any]] = None,
) -> StateControlPreflight:
    return StateControlPreflight(
        status=status,
        row_count=len(states),
        states=states,
        diagnostics=_state_diagnostics(status, states, source=source, cache=cache),
    )


def _state_error(
    status: str,
    exc: BaseException,
    *,
    cache: Optional[Dict[str, Any]] = None,
) -> StateControlPreflight:
    diagnostics = _state_diagnostics(status, {}, source="state_roots", cache=cache)
    diagnostics["error"] = _safe_error_message(exc)
    return StateControlPreflight(
        status=status,
        row_count=0,
        states={},
        diagnostics=diagnostics,
    )


def _state_diagnostics(
    status: str,
    states: Dict[str, Dict[str, Any]],
    *,
    source: str,
    cache: Optional[Dict[str, Any]] = None,
) -> Dict[str, Any]:
    owner_rows = sum(1 for row in states.values() if row.get("owner"))
    controller_rows = sum(1 for row in states.values() if row.get("controller"))
    diagnostics: Dict[str, Any] = {
        "status": status,
        "row_count": len(states),
        "owner_rows": owner_rows,
        "controller_rows": controller_rows,
        "state_sample": list(sorted(states))[:8],
        "source": source,
        "runtime_source": "hoi4.exe",
        "address_manifest_policy": "omitted",
    }
    if cache is not None:
        diagnostics["cache"] = {
            "enabled": bool(cache.get("enabled")),
            "status": str(cache.get("status") or "unknown"),
        }
        if cache.get("tag_table_source"):
            diagnostics["cache"]["tag_table_source"] = str(cache["tag_table_source"])
    return diagnostics


def _load_state_roots_cache(
    cache_path: Path,
    identity: _process_identity.ProcessIdentity,
) -> Optional[Dict[str, Any]]:
    try:
        payload = json.loads(Path(cache_path).read_text(encoding="utf-8"))
    except (OSError, ValueError, TypeError):
        return None
    if not isinstance(payload, dict):
        return None
    if payload.get("type") != "state_roots_preflight_cache_v1":
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
    states = payload.get("states")
    if len(_coerce_state_roots(states)) < MIN_USABLE_STATE_ROWS:
        return None
    return payload


def _write_state_roots_cache(
    cache_path: Path,
    identity: _process_identity.ProcessIdentity,
    roots: Dict[int, int],
) -> bool:
    payload = {
        "type": "state_roots_preflight_cache_v1",
        "created_utc": datetime.now(timezone.utc).isoformat(),
        "pid": int(identity.pid),
        "create_time_100ns": identity.create_time_100ns,
        "exe_sha256": identity.exe_sha256,
        "module_base": identity.module_base,
        "state_count": len(roots),
        "states": {str(int(sid)): int(obj) for sid, obj in sorted(roots.items())},
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
    for value in (os.environ.get("USERNAME"), os.path.expanduser("~")):
        if value:
            msg = msg.replace(value, "<redacted>")
    return msg[:240]


def main(argv: Optional[List[str]] = None) -> int:
    import argparse

    parser = argparse.ArgumentParser(
        description="Resolve live state map-control rows for collector preflight.")
    parser.add_argument("--pid", type=int, required=True)
    parser.add_argument("--cache", type=Path)
    args = parser.parse_args(argv)
    result = resolve_state_control_preflight(args.pid, cache_path=args.cache)
    print(json.dumps(result.diagnostics, indent=2, sort_keys=True))
    return 0 if result.usable else 1


if __name__ == "__main__":
    raise SystemExit(main())
