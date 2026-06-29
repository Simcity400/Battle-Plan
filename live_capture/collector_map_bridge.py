"""Preflight map-control roots for the in-process collector.

This module is the state/province-control counterpart to
``collector_country_bridge``. Discovery may scan memory, so it is pre-recording
only. Cache hits validate cached state-object roots for the same ``hoi4.exe``
instance and then re-read owner/controller fields live.

No saves are read. Diagnostics omit raw addresses; cache files are runtime
artifacts under the session output tree.
"""
from __future__ import annotations

from collections import Counter, defaultdict
from dataclasses import dataclass
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import re
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
MIN_USABLE_PROVINCE_ROWS = 10000
MAX_CACHED_CONTROLLER_TAG_ID = 4096
MAX_CACHED_PROVINCE_SLOTS = 20000
MAX_NATIVE_CONTROLLER_TAG_ID = 1024
MAX_NATIVE_PROVINCE_STRIDE = 64
MAP_CONTROL_POINTER_WIDTH = 8


@dataclass(frozen=True)
class StateControlPreflight:
    status: str
    row_count: int
    states: Dict[str, Dict[str, Any]]
    diagnostics: Dict[str, Any]
    state_roots_encoded: Optional[str] = None

    @property
    def usable(self) -> bool:
        return self.status == "resolved" and self.row_count >= MIN_USABLE_STATE_ROWS


@dataclass(frozen=True)
class ProvinceControlPreflight:
    status: str
    row_count: int
    provinces: Dict[str, Dict[str, Any]]
    diagnostics: Dict[str, Any]
    province_bridge_encoded: Optional[str] = None
    controller_bridge_encoded: Optional[str] = None

    @property
    def usable(self) -> bool:
        return self.status == "resolved" and self.row_count >= MIN_USABLE_PROVINCE_ROWS


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
                                state_roots_encoded=encode_map_control_state_roots(roots),
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
                state_roots_encoded=encode_map_control_state_roots(roots),
            )
    except Exception as exc:
        return _state_error("resolve_failed", exc, cache=cache_diag)


def resolve_province_control_preflight(
    pid: int,
    *,
    min_rows: int = MIN_USABLE_PROVINCE_ROWS,
    cache_path: Optional[Path] = None,
    state_cache_path: Optional[Path] = None,
    allow_cold_scan: bool = False,
) -> ProvinceControlPreflight:
    """Resolve live province controller rows for one recorder session.

    Cache hits perform only bounded reads from the cached dense province array
    and cached pointer bridge. Cold cache population is explicit because the
    existing locator may scan heap and is not suitable for a hot path.
    """
    try:
        import winmem  # type: ignore
        import state_detail_live as sdl  # type: ignore
        from live_layers import province_control as pc  # type: ignore
        from live_layers import tag_table as tt  # type: ignore
    except Exception as exc:
        return _province_error("import_failed", exc)

    cache_diag: Dict[str, Any] = {
        "enabled": bool(cache_path),
        "status": "not_requested" if cache_path is None else "miss",
        "tag_table_source": "country_database_root",
    }
    state_bridge_diag: Dict[str, Any] = {
        "cache_enabled": bool(state_cache_path),
        "status": "not_requested" if state_cache_path is None else "miss",
    }
    try:
        with winmem.Process(int(pid), "hoi4.exe") as proc:
            identity = _process_identity.identify(proc)
            table = tag_table_from_country_database_root(proc, tt)
            if table is None:
                return _province_status(
                    "tag_table_unavailable",
                    {},
                    cache=cache_diag,
                    state_bridge=state_bridge_diag,
                )

            prov_to_state = pc.load_province_to_state()
            state_ctrl: Optional[Dict[int, int]] = None
            if state_cache_path is not None:
                cached_states = _load_state_roots_cache(state_cache_path, identity)
                roots = _coerce_state_roots(
                    cached_states.get("states") if cached_states is not None else None
                )
                if roots and _state_roots_valid(proc, roots, sdl):
                    state_ctrl = state_controller_ids_from_state_roots(
                        proc,
                        roots,
                        sdl,
                        table=table,
                    )
                    state_bridge_diag["status"] = "hit"
                    state_bridge_diag["row_count"] = len(state_ctrl)
                else:
                    state_bridge_diag["status"] = "stale" if cached_states else "miss"

            if cache_path is not None:
                cached = _load_province_array_cache(cache_path, identity)
                if cached is not None:
                    base = int(cached["base"])
                    slots = int(cached["slots"])
                    ptr2tagid = _coerce_ptr_bridge(cached.get("ptr2tagid"))
                    validation_error = province_array_cache_validation_error(
                        proc,
                        base,
                        slots,
                        ptr2tagid,
                        table,
                        prov_to_state,
                        pc,
                        state_ctrl=state_ctrl,
                    )
                    if validation_error is None:
                        provinces = province_rows_from_cached_base(
                            proc,
                            table,
                            base,
                            slots,
                            ptr2tagid,
                            prov_to_state,
                            pc,
                        )
                        if len(provinces) >= int(min_rows):
                            cache_diag["status"] = "hit"
                            return _province_status(
                                "resolved",
                                provinces,
                                source="province_array_cache",
                                cache=cache_diag,
                                state_bridge=state_bridge_diag,
                                slots=slots,
                                pointer_bridge_count=len(ptr2tagid),
                                province_bridge_encoded=encode_map_control_province_bridge(
                                    base,
                                    slots,
                                    pc,
                                ),
                                controller_bridge_encoded=(
                                    encode_map_control_controller_bridge(ptr2tagid)
                                ),
                            )
                    cache_diag["validation"] = validation_error
                    cache_diag["status"] = "stale"

            if allow_cold_scan and not state_ctrl:
                layer = sdl.StateDetailLive.resolve(proc)
                roots = getattr(layer, "states", None) if layer is not None else None
                roots = _coerce_state_roots(roots)
                if not _state_roots_valid(proc, roots, sdl):
                    return _province_status(
                        "state_roots_unavailable",
                        {},
                        source="state_roots",
                        cache=cache_diag,
                        state_bridge=state_bridge_diag,
                    )
                state_ctrl = state_controller_ids_from_state_roots(
                    proc,
                    roots,
                    sdl,
                    table=table,
                )
                state_bridge_diag["status"] = "resolved"
                state_bridge_diag["row_count"] = len(state_ctrl)
                if state_cache_path is not None:
                    _write_state_roots_cache(state_cache_path, identity, roots)

            if not allow_cold_scan:
                return _province_status(
                    "province_array_cache_unavailable",
                    {},
                    source="province_array_cache",
                    cache=cache_diag,
                    state_bridge=state_bridge_diag,
                )

            if state_ctrl:
                expected = pc._build_expected(state_ctrl, prov_to_state)
                locator_diag: Dict[str, Any] = {
                    "state_controller_count": len(state_ctrl),
                    "province_to_state_count": len(prov_to_state),
                }
                loc = pc.locate_array(proc, expected, diagnostics=locator_diag)
                locator_diag["state_controller_count"] = len(state_ctrl)
                locator_diag["province_to_state_count"] = len(prov_to_state)
                if loc is None:
                    return _province_status(
                        "province_array_locate_failed",
                        {},
                        source="province_array_locator",
                        cache=cache_diag,
                        state_bridge=state_bridge_diag,
                        locator=locator_diag,
                    )
                base, slots = int(loc[0]), int(loc[1])
                ptr2tagid = province_ptr_bridge_from_cached_base(
                    proc,
                    base,
                    slots,
                    state_ctrl,
                    prov_to_state,
                    pc,
                )
                validation_error = province_array_cache_validation_error(
                    proc,
                    base,
                    slots,
                    ptr2tagid,
                    table,
                    prov_to_state,
                    pc,
                    state_ctrl=state_ctrl,
                )
                if validation_error is not None:
                    return _province_status(
                        _province_validation_status(validation_error),
                        {},
                        source="state_roots_cache",
                        cache=cache_diag,
                        state_bridge=state_bridge_diag,
                        slots=slots,
                        pointer_bridge_count=len(ptr2tagid),
                        locator=locator_diag,
                    )
                provinces = province_rows_from_cached_base(
                    proc,
                    table,
                    base,
                    slots,
                    ptr2tagid,
                    prov_to_state,
                    pc,
                )
                if len(provinces) < int(min_rows):
                    return _province_status(
                        "insufficient_rows",
                        provinces,
                        source="state_roots_cache",
                        cache=cache_diag,
                        state_bridge=state_bridge_diag,
                        slots=slots,
                        pointer_bridge_count=len(ptr2tagid),
                        locator=locator_diag,
                    )
                if cache_path is not None:
                    cache_diag["status"] = (
                        "stored" if _write_province_array_cache(
                            cache_path,
                            identity,
                            base,
                            slots,
                            ptr2tagid,
                            len(provinces),
                        )
                        else "store_failed"
                    )
                return _province_status(
                    "resolved",
                    provinces,
                    source="state_roots_cache",
                    cache=cache_diag,
                    state_bridge=state_bridge_diag,
                    slots=slots,
                    pointer_bridge_count=len(ptr2tagid),
                    locator=locator_diag,
                    province_bridge_encoded=encode_map_control_province_bridge(
                        base,
                        slots,
                        pc,
                    ),
                    controller_bridge_encoded=(
                        encode_map_control_controller_bridge(ptr2tagid)
                    ),
                )

            return _province_status(
                "state_roots_unavailable",
                {},
                source="state_roots",
                cache=cache_diag,
                state_bridge=state_bridge_diag,
            )
    except Exception as exc:
        return _province_error(
            "resolve_failed",
            exc,
            cache=cache_diag,
            state_bridge=state_bridge_diag,
        )


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


def state_controller_ids_from_state_roots(
    proc: Any,
    roots: Dict[int, int],
    state_module: Any,
    *,
    table: Any = None,
) -> Dict[int, int]:
    """Return ``state_id -> controller tag-db-id`` from validated state roots."""
    sid_off = _state_sid_offset(state_module)
    ctrl_off = _state_ctrl_offset(state_module)
    controllers: Dict[int, int] = {}
    for sid in sorted(roots):
        obj = int(roots[sid])
        try:
            if int(proc.u32(obj + sid_off)) != int(sid):
                continue
            ctrl = int(proc.u32(obj + ctrl_off))
        except Exception:
            continue
        if ctrl == 0:
            controllers[int(sid)] = ctrl
        elif table is not None:
            if _tag_table_covers(table, {ctrl}):
                controllers[int(sid)] = ctrl
        elif 0 < ctrl <= MAX_CACHED_CONTROLLER_TAG_ID:
            controllers[int(sid)] = ctrl
    return controllers


def province_ptr_bridge_from_cached_base(
    proc: Any,
    base: int,
    slots: int,
    state_ctrl: Dict[int, int],
    prov_to_state: Dict[int, int],
    province_module: Any,
) -> Dict[int, int]:
    """Build ``controller pointer -> tag-db-id`` from a located province array."""
    stride = int(getattr(province_module, "PROV_STRIDE", 8))
    ptr_votes: Dict[int, Counter] = defaultdict(Counter)
    for prov, st in prov_to_state.items():
        cid = int(state_ctrl.get(int(st), 0) or 0)
        if not cid or int(prov) >= int(slots):
            continue
        try:
            ptr = int(proc.u64(int(base) + int(prov) * stride))
        except Exception:
            continue
        if ptr:
            ptr_votes[ptr][cid] += 1

    ptr2tagid: Dict[int, int] = {}
    for ptr, votes in ptr_votes.items():
        top, count = votes.most_common(1)[0]
        total = sum(votes.values())
        if total and count / total >= 0.6:
            ptr2tagid[int(ptr)] = int(top)
    tagids = list(ptr2tagid.values())
    if len(tagids) != len(set(tagids)):
        return {}
    min_controllers = int(getattr(province_module, "MIN_CONTROLLERS", 2))
    max_controllers = int(getattr(province_module, "MAX_CONTROLLERS", 250))
    if not (min_controllers <= len(ptr2tagid) <= max_controllers):
        return {}
    return ptr2tagid


def province_rows_from_cached_base(
    proc: Any,
    table: Any,
    base: int,
    slots: int,
    ptr2tagid: Dict[int, int],
    prov_to_state: Dict[int, int],
    province_module: Any,
) -> Dict[str, Dict[str, Any]]:
    """Return ``provinces`` payload rows from a validated province array base."""
    stride = int(getattr(province_module, "PROV_STRIDE", 8))
    provinces: Dict[str, Dict[str, Any]] = {}
    for prov in sorted(int(k) for k in prov_to_state):
        if prov < 0 or prov >= int(slots):
            continue
        try:
            ptr = int(proc.u64(int(base) + prov * stride))
        except Exception:
            continue
        tag_id = ptr2tagid.get(ptr) if ptr else None
        provinces[str(prov)] = {
            "controller": table.tag(tag_id) if tag_id is not None else None,
            "state_id": int(prov_to_state[prov]),
        }
    return provinces


def encode_map_control_state_roots(roots: Dict[int, int]) -> str:
    """Encode ``state_id:state_ptr`` rows for the DLL control file."""
    normalized = _coerce_state_roots(roots)
    return ";".join(
        f"{int(sid)}:0x{int(ptr):X}"
        for sid, ptr in sorted(normalized.items())
    )


def encode_map_control_province_bridge(
    base: int,
    slots: int,
    province_module: Any,
) -> str:
    """Encode dense province-array metadata for the DLL control file."""
    stride = int(getattr(province_module, "PROV_STRIDE", 8))
    return (
        f"province_array=0x{int(base):X};"
        f"province_count={int(slots)};"
        f"province_stride={stride};"
        "controller_ptr_offset=0"
    )


def encode_map_control_controller_bridge(ptr2tagid: Dict[int, int]) -> str:
    """Encode ``tag_db_id:controller_ptr`` rows for the DLL control file."""
    bridge = _coerce_ptr_bridge(ptr2tagid)
    rows = [
        (int(tag_id), int(ptr))
        for ptr, tag_id in bridge.items()
    ]
    rows.sort()
    return ";".join(f"{tag_id}:0x{ptr:X}" for tag_id, ptr in rows)


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


def _province_array_cache_valid(
    proc: Any,
    base: int,
    slots: int,
    ptr2tagid: Dict[int, int],
    table: Any,
    prov_to_state: Dict[int, int],
    province_module: Any,
    *,
    state_ctrl: Optional[Dict[int, int]] = None,
) -> bool:
    return province_array_cache_validation_error(
        proc,
        base,
        slots,
        ptr2tagid,
        table,
        prov_to_state,
        province_module,
        state_ctrl=state_ctrl,
    ) is None


def province_array_cache_validation_error(
    proc: Any,
    base: int,
    slots: int,
    ptr2tagid: Dict[int, int],
    table: Any,
    prov_to_state: Dict[int, int],
    province_module: Any,
    *,
    state_ctrl: Optional[Dict[int, int]] = None,
) -> Optional[str]:
    """Return a sanitized fail-closed reason for a cached province array."""
    if not _province_base_slots_valid(base, slots, prov_to_state, province_module):
        return "base_slots_invalid"
    if not ptr2tagid:
        return "pointer_bridge_empty"
    if len(ptr2tagid.values()) != len(set(ptr2tagid.values())):
        return "pointer_bridge_not_injective"
    bridge_shape_error = _controller_bridge_shape_error(ptr2tagid, province_module)
    if bridge_shape_error is not None:
        return bridge_shape_error
    if not _tag_table_covers(table, set(ptr2tagid.values())):
        return "tag_table_uncovered"

    distinct_ptrs = _distinct_scheduled_province_controller_ptrs(
        proc,
        base,
        slots,
        province_module,
    )
    if distinct_ptrs is None:
        return "province_array_unreadable"
    bridged_ptrs = set(int(k) for k in ptr2tagid)
    if not distinct_ptrs.issubset(bridged_ptrs):
        return "pointer_bridge_mismatch"
    if state_ctrl:
        expected = province_ptr_bridge_from_cached_base(
            proc,
            base,
            slots,
            state_ctrl,
            prov_to_state,
            province_module,
        )
        if expected and not any(ptr2tagid.get(ptr) == tag_id
                                for ptr, tag_id in expected.items()):
            return "state_bridge_conflict"
    return None


def _province_base_slots_valid(
    base: int,
    slots: int,
    prov_to_state: Dict[int, int],
    province_module: Any,
) -> bool:
    if not prov_to_state:
        return False
    try:
        base_i = int(base)
        slots_i = int(slots)
        stride_i = int(getattr(province_module, "PROV_STRIDE", 8))
    except (TypeError, ValueError):
        return False
    if base_i <= 0 or base_i % MAP_CONTROL_POINTER_WIDTH != 0:
        return False
    if (
        stride_i < MAP_CONTROL_POINTER_WIDTH
        or stride_i > MAX_NATIVE_PROVINCE_STRIDE
        or stride_i % MAP_CONTROL_POINTER_WIDTH != 0
    ):
        return False
    max_prov = max(int(k) for k in prov_to_state)
    if slots_i <= max_prov or slots_i > MAX_CACHED_PROVINCE_SLOTS:
        return False
    heap_lo = int(getattr(province_module, "HEAP_LO", 1))
    heap_hi = int(getattr(province_module, "HEAP_HI", 1 << 63))
    if not (heap_lo <= base_i < heap_hi):
        return False
    try:
        end = base_i + slots_i * stride_i
    except OverflowError:
        return False
    if end > heap_hi:
        return False
    return True


def _controller_bridge_shape_error(
    ptr2tagid: Dict[int, int],
    province_module: Any,
) -> Optional[str]:
    heap_lo = int(getattr(province_module, "HEAP_LO", 1))
    heap_hi = int(getattr(province_module, "HEAP_HI", 1 << 63))
    for ptr, tag_id in ptr2tagid.items():
        if int(tag_id) <= 0 or int(tag_id) > MAX_NATIVE_CONTROLLER_TAG_ID:
            return "pointer_bridge_out_of_range"
        if int(ptr) % MAP_CONTROL_POINTER_WIDTH != 0:
            return "pointer_bridge_out_of_range"
        if not (heap_lo <= int(ptr) < heap_hi):
            return "pointer_bridge_out_of_range"
    return None


def _distinct_scheduled_province_controller_ptrs(
    proc: Any,
    base: int,
    slots: int,
    province_module: Any,
) -> Optional[set[int]]:
    stride = int(getattr(province_module, "PROV_STRIDE", 8))
    heap_lo = int(getattr(province_module, "HEAP_LO", 1))
    heap_hi = int(getattr(province_module, "HEAP_HI", 1 << 63))
    distinct: set[int] = set()
    try:
        for pid in range(1, int(slots)):
            ptr = int(proc.u64(int(base) + pid * stride))
            if ptr == 0:
                continue
            if not (heap_lo <= ptr < heap_hi):
                return None
            distinct.add(ptr)
    except Exception:
        return None
    return distinct


def _tag_table_covers(table: Any, tag_ids: Iterable[int]) -> bool:
    tag_ids = set(int(t) for t in tag_ids)
    covers = getattr(table, "covers", None)
    if callable(covers):
        try:
            return bool(covers(tag_ids))
        except Exception:
            return False
    try:
        for tag_id in tag_ids:
            if tag_id <= 0:
                return False
            tag = table.tag(tag_id)
            if not tag or tag == "---":
                return False
    except Exception:
        return False
    return True


def _coerce_ptr_bridge(value: Any) -> Dict[int, int]:
    if not isinstance(value, dict):
        return {}
    out: Dict[int, int] = {}
    for raw_ptr, raw_tid in value.items():
        if isinstance(raw_ptr, bool) or isinstance(raw_tid, bool):
            continue
        try:
            ptr = int(raw_ptr)
            tid = int(raw_tid)
        except (TypeError, ValueError):
            continue
        if ptr > 0 and ptr % 8 == 0 and tid > 0:
            out[ptr] = tid
    return out


def _state_sid_offset(state_module: Any) -> int:
    if hasattr(state_module, "OFF_SID"):
        return int(getattr(state_module, "OFF_SID"))
    return int(getattr(state_module, "STATE_OFF_SID"))


def _state_ctrl_offset(state_module: Any) -> int:
    if hasattr(state_module, "OFF_CTRL"):
        return int(getattr(state_module, "OFF_CTRL"))
    return int(getattr(state_module, "STATE_OFF_CTRL"))


def _province_validation_status(validation_error: str) -> str:
    if validation_error == "tag_table_uncovered":
        return "tag_table_uncovered"
    if validation_error.startswith("base_") or validation_error.endswith("_unreadable"):
        return "province_array_invalid"
    return "pointer_bridge_invalid"


def _state_status(
    status: str,
    states: Dict[str, Dict[str, Any]],
    *,
    source: str = "state_roots",
    cache: Optional[Dict[str, Any]] = None,
    state_roots_encoded: Optional[str] = None,
) -> StateControlPreflight:
    return StateControlPreflight(
        status=status,
        row_count=len(states),
        states=states,
        diagnostics=_state_diagnostics(status, states, source=source, cache=cache),
        state_roots_encoded=state_roots_encoded,
    )


def _province_status(
    status: str,
    provinces: Dict[str, Dict[str, Any]],
    *,
    source: str = "province_control_resolve",
    cache: Optional[Dict[str, Any]] = None,
    state_bridge: Optional[Dict[str, Any]] = None,
    slots: Optional[int] = None,
    pointer_bridge_count: Optional[int] = None,
    locator: Optional[Dict[str, Any]] = None,
    province_bridge_encoded: Optional[str] = None,
    controller_bridge_encoded: Optional[str] = None,
) -> ProvinceControlPreflight:
    return ProvinceControlPreflight(
        status=status,
        row_count=len(provinces),
        provinces=provinces,
        diagnostics=_province_diagnostics(
            status,
            provinces,
            source=source,
            cache=cache,
            state_bridge=state_bridge,
            slots=slots,
            pointer_bridge_count=pointer_bridge_count,
            locator=locator,
        ),
        province_bridge_encoded=province_bridge_encoded,
        controller_bridge_encoded=controller_bridge_encoded,
    )


def _province_error(
    status: str,
    exc: BaseException,
    *,
    cache: Optional[Dict[str, Any]] = None,
    state_bridge: Optional[Dict[str, Any]] = None,
) -> ProvinceControlPreflight:
    diagnostics = _province_diagnostics(
        status,
        {},
        source="province_control_resolve",
        cache=cache,
        state_bridge=state_bridge,
    )
    diagnostics["error"] = _safe_error_message(exc)
    return ProvinceControlPreflight(
        status=status,
        row_count=0,
        provinces={},
        diagnostics=diagnostics,
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


def _province_diagnostics(
    status: str,
    provinces: Dict[str, Dict[str, Any]],
    *,
    source: str,
    cache: Optional[Dict[str, Any]] = None,
    state_bridge: Optional[Dict[str, Any]] = None,
    slots: Optional[int] = None,
    pointer_bridge_count: Optional[int] = None,
    locator: Optional[Dict[str, Any]] = None,
) -> Dict[str, Any]:
    controller_rows = sum(1 for row in provinces.values() if row.get("controller"))
    distinct_controllers = len({
        row.get("controller") for row in provinces.values() if row.get("controller")
    })
    diagnostics: Dict[str, Any] = {
        "status": status,
        "row_count": len(provinces),
        "controller_rows": controller_rows,
        "distinct_controllers": distinct_controllers,
        "province_sample": list(sorted(provinces))[:8],
        "source": source,
        "runtime_source": "hoi4.exe",
        "address_manifest_policy": "omitted",
    }
    if slots is not None:
        diagnostics["slots"] = int(slots)
    if pointer_bridge_count is not None:
        diagnostics["pointer_bridge_count"] = int(pointer_bridge_count)
    if cache is not None:
        diagnostics["cache"] = {
            "enabled": bool(cache.get("enabled")),
            "status": str(cache.get("status") or "unknown"),
        }
        if cache.get("tag_table_source"):
            diagnostics["cache"]["tag_table_source"] = str(cache["tag_table_source"])
        if cache.get("validation"):
            diagnostics["cache"]["validation"] = str(cache["validation"])
    if state_bridge is not None:
        diagnostics["state_bridge"] = {
            "cache_enabled": bool(state_bridge.get("cache_enabled")),
            "status": str(state_bridge.get("status") or "unknown"),
        }
        if state_bridge.get("row_count") is not None:
            diagnostics["state_bridge"]["row_count"] = int(state_bridge["row_count"])
    locator_diag = _sanitize_locator_diagnostics(locator)
    if locator_diag:
        diagnostics["locator"] = locator_diag
    return diagnostics


def _sanitize_locator_diagnostics(value: Optional[Dict[str, Any]]) -> Dict[str, Any]:
    if not isinstance(value, dict):
        return {}
    scalar_keys = {
        "version",
        "result",
        "required_expected_count",
        "expected_count",
        "expected_controller_count",
        "top_controller_group_count",
        "equality_pair_count",
        "inequality_pair_count",
        "max_province_id",
        "stride",
        "required_span_bytes",
        "numpy_enabled",
        "elapsed_ms",
        "slots",
        "hit_scan_range",
        "hit_partition_bad_count",
        "hit_controller_count",
        "state_controller_count",
        "province_to_state_count",
    }
    out: Dict[str, Any] = {}
    for key in scalar_keys:
        if key in value:
            raw = value[key]
            if isinstance(raw, bool):
                out[key] = bool(raw)
            elif isinstance(raw, int):
                out[key] = int(raw)
            elif isinstance(raw, str) and not raw.lower().startswith("0x"):
                out[key] = raw[:80]
    range_keys = {
        "name",
        "regions_seen",
        "regions_skipped_small",
        "regions_read",
        "regions_read_failed",
        "bytes_read",
        "largest_region_bytes",
        "candidate_offsets",
        "survivor_offsets",
        "unaligned_survivors",
        "partition_candidates",
        "partition_rejects",
        "eq_rejected_regions",
        "neq_rejected_regions",
    }
    raw_ranges = value.get("scan_ranges")
    if isinstance(raw_ranges, list):
        ranges: List[Dict[str, Any]] = []
        for raw_range in raw_ranges:
            if not isinstance(raw_range, dict):
                continue
            safe_range: Dict[str, Any] = {}
            for key in range_keys:
                if key not in raw_range:
                    continue
                raw = raw_range[key]
                if isinstance(raw, bool):
                    safe_range[key] = bool(raw)
                elif isinstance(raw, int):
                    safe_range[key] = int(raw)
                elif isinstance(raw, str) and not raw.lower().startswith("0x"):
                    safe_range[key] = raw[:80]
            if safe_range:
                ranges.append(safe_range)
        if ranges:
            out["scan_ranges"] = ranges
    return out


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


def _load_province_array_cache(
    cache_path: Path,
    identity: _process_identity.ProcessIdentity,
) -> Optional[Dict[str, Any]]:
    try:
        payload = json.loads(Path(cache_path).read_text(encoding="utf-8"))
    except (OSError, ValueError, TypeError):
        return None
    if not isinstance(payload, dict):
        return None
    if payload.get("type") != "province_control_preflight_cache_v1":
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
    base = payload.get("base")
    slots = payload.get("slots")
    if not isinstance(base, int) or not isinstance(slots, int):
        return None
    if base <= 0 or base % 8 != 0:
        return None
    if slots <= 0 or slots > MAX_CACHED_PROVINCE_SLOTS:
        return None
    if len(_coerce_ptr_bridge(payload.get("ptr2tagid"))) < 2:
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


def _write_province_array_cache(
    cache_path: Path,
    identity: _process_identity.ProcessIdentity,
    base: int,
    slots: int,
    ptr2tagid: Dict[int, int],
    province_count: int,
) -> bool:
    payload = {
        "type": "province_control_preflight_cache_v1",
        "created_utc": datetime.now(timezone.utc).isoformat(),
        "pid": int(identity.pid),
        "create_time_100ns": identity.create_time_100ns,
        "exe_sha256": identity.exe_sha256,
        "module_base": identity.module_base,
        "base": int(base),
        "slots": int(slots),
        "province_count": int(province_count),
        "ptr2tagid": {
            str(int(ptr)): int(tag_id)
            for ptr, tag_id in sorted(ptr2tagid.items())
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
    for value in (os.environ.get("USERNAME"), os.path.expanduser("~")):
        if value:
            msg = msg.replace(value, "<redacted>")
    return msg[:240]


def main(argv: Optional[List[str]] = None) -> int:
    import argparse

    parser = argparse.ArgumentParser(
        description="Resolve live map-control rows for collector preflight.")
    parser.add_argument("--pid", type=int, required=True)
    parser.add_argument("--domain", choices=("states", "provinces"), default="states")
    parser.add_argument("--cache", type=Path)
    parser.add_argument("--state-cache", type=Path,
                        help="State-root cache used to bridge province pointers.")
    parser.add_argument("--allow-cold-scan", action="store_true",
                        help="Allow the old heap-scanning province array locator.")
    args = parser.parse_args(argv)
    if args.domain == "provinces":
        result = resolve_province_control_preflight(
            args.pid,
            cache_path=args.cache,
            state_cache_path=args.state_cache,
            allow_cold_scan=args.allow_cold_scan,
        )
    else:
        result = resolve_state_control_preflight(args.pid, cache_path=args.cache)
    print(json.dumps(result.diagnostics, indent=2, sort_keys=True))
    return 0 if result.usable else 1


if __name__ == "__main__":
    raise SystemExit(main())
