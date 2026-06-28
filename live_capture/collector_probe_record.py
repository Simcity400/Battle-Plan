"""Record live collector root-probe frames as durable smoke/proof artifacts."""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import re
import time
from typing import Any, Dict, Iterable, List, Optional, Tuple

from .collector_attach import AttachError, attach_noop
from .collector_division_bridge import encode_division_body_bridge
from .collector_frames import latest_verified_root_probe, snapshot_from_root_probe_frame
from .durable_store import DurableCaptureStore


MAX_COUNTRY_OBJECT_BRIDGE_ROWS = 512


def record_probe(pid: int, dll: Path, out_root: Path, *, dwell_s: float = 1.0,
                 game_id: str = "collector_probe", store_root: Path | None = None) -> Path:
    return record_probes(pid, dll, out_root, dwell_s=dwell_s, game_id=game_id,
                         store_root=store_root, repeat=1, interval_s=0.0)


def record_probes(
    pid: int,
    dll: Path,
    out_root: Path,
    *,
    dwell_s: float = 1.0,
    game_id: str = "collector_probe",
    store_root: Path | None = None,
    repeat: int = 1,
    interval_s: float = 0.0,
    until_game_hours: Optional[int] = None,
    max_wall_s: Optional[float] = None,
    unique_store_hours_only: bool = False,
    worker_duration_s: Optional[float] = None,
    worker_sample_ms: int = 10,
    require_hourly_cadence: Optional[bool] = None,
    session_id: Optional[str] = None,
    write_snapshot_artifacts: bool = True,
    country_object_bridge: Any = None,
    country_record_bridge: Any = None,
    division_body_bridge: Any = None,
    map_control_state_roots: Optional[str] = None,
    map_control_province_bridge: Optional[str] = None,
    map_control_controller_bridge: Optional[str] = None,
    scheduled_snapshot_step_hours: int = 1,
) -> Path:
    if repeat < 1:
        raise ValueError("repeat must be >= 1")
    if interval_s < 0:
        raise ValueError("interval_s must be >= 0")
    if until_game_hours is not None and until_game_hours < 0:
        raise ValueError("until_game_hours must be >= 0")
    if max_wall_s is not None and max_wall_s <= 0:
        raise ValueError("max_wall_s must be > 0")
    if worker_duration_s is not None and worker_duration_s <= 0:
        raise ValueError("worker_duration_s must be > 0")
    if worker_sample_ms < 10:
        raise ValueError("worker_sample_ms must be >= 10")
    if scheduled_snapshot_step_hours < 1:
        raise ValueError("scheduled_snapshot_step_hours must be >= 1")
    encoded_country_object_bridge = (
        encode_country_object_bridge(country_object_bridge)
        if country_object_bridge is not None else None
    )
    country_object_bridge_rows = (
        len(encoded_country_object_bridge.split(";"))
        if encoded_country_object_bridge else 0
    )
    encoded_country_record_bridge = (
        encode_country_record_bridge(country_record_bridge)
        if country_record_bridge is not None else None
    )
    country_record_bridge_rows = (
        len(encoded_country_record_bridge.split(";"))
        if encoded_country_record_bridge else 0
    )
    encoded_division_body_bridge = (
        encode_division_body_bridge(division_body_bridge)
        if division_body_bridge is not None else None
    )
    division_body_bridge_rows = (
        len(encoded_division_body_bridge.split(";"))
        if encoded_division_body_bridge else 0
    )
    map_control_state_root_rows = _encoded_row_count(map_control_state_roots)
    map_control_controller_bridge_rows = _encoded_row_count(
        map_control_controller_bridge
    )
    map_control_province_bridge_supplied = bool(map_control_province_bridge)

    run_id = _run_id(pid)
    effective_session_id = session_id or run_id
    run_dir = out_root / run_id
    run_dir.mkdir(parents=True, exist_ok=False)
    manifest_path = run_dir / "manifest.json"
    store = DurableCaptureStore(str(store_root)) if store_root is not None else None
    samples: List[Dict[str, Any]] = []
    raw_frames: List[Dict[str, Any]] = []
    worker_pipe_tokens: List[str] = []
    attach_errors: List[str] = []
    first_frame: Optional[Dict[str, Any]] = None
    first_snapshot: Optional[Dict[str, Any]] = None
    started = time.monotonic()
    first_game_hour_raw: Optional[int] = None
    stored_hours: set[int] = set()
    stop_reason = "repeat_exhausted"

    for attach_index in range(repeat):
        if attach_index:
            if max_wall_s is not None:
                remaining_s = max_wall_s - (time.monotonic() - started)
                if remaining_s <= 0 or interval_s >= remaining_s:
                    stop_reason = "max_wall_s"
                    break
            time.sleep(interval_s)
        if max_wall_s is not None and time.monotonic() - started >= max_wall_s:
            stop_reason = "max_wall_s"
            break
        if worker_duration_s is not None and max_wall_s is not None:
            remaining_s = max_wall_s - (time.monotonic() - started)
            if remaining_s <= worker_duration_s:
                stop_reason = "max_wall_s"
                break
        sample_started = time.monotonic()
        effective_dwell_s = dwell_s
        if worker_duration_s is not None:
            effective_dwell_s = max(dwell_s, worker_duration_s + 1.0)
        control_path = None
        pipe_token = None
        if worker_duration_s is not None:
            control_path, pipe_token = _write_worker_control(
                dll,
                worker_duration_s,
                worker_sample_ms,
                session_id=effective_session_id,
                country_object_bridge=encoded_country_object_bridge,
                country_record_bridge=encoded_country_record_bridge,
                division_body_bridge=encoded_division_body_bridge,
                map_control_state_roots=map_control_state_roots,
                map_control_province_bridge=map_control_province_bridge,
                map_control_controller_bridge=map_control_controller_bridge,
                scheduled_snapshot_step_hours=scheduled_snapshot_step_hours,
            )
            worker_pipe_tokens.append(pipe_token)
        try:
            try:
                result = _attach_with_retries(
                    pid, dll, dwell_s=effective_dwell_s, pipe_token=pipe_token
                )
            except Exception as exc:
                stop_reason = "attach_error"
                attach_errors.append(
                    "burst_%d:%s" % (attach_index, _safe_error_message(exc)))
                break
        finally:
            if control_path is not None:
                _remove_if_exists(control_path)
        frames = result.pipe_frames or ([result.pipe_frame] if result.pipe_frame else [])
        if not frames:
            stop_reason = "no_pipe_frame"
            attach_errors.append("burst_%d:no_pipe_frame" % attach_index)
            break

        stop_after_attach = False
        for burst_sample_index, frame in enumerate(frames):
            root_probe = latest_verified_root_probe([frame])
            if root_probe is None:
                if _is_raw_stream_frame(frame):
                    raw_index = len(raw_frames)
                    frame_name = f"collector_raw_{raw_index:04d}.json"
                    _write_json(run_dir / frame_name, frame)
                    raw_frames.append(
                        _raw_frame_manifest(
                            raw_index=raw_index,
                            result=result,
                            frame=frame,
                            frame_name=frame_name,
                            worker_burst_index=attach_index,
                            burst_sample_index=burst_sample_index,
                            worker_pipe_token=pipe_token,
                            elapsed_wall_s=time.monotonic() - sample_started,
                        )
                    )
                continue

            current_raw = root_probe.get("current_game_date_raw")
            duplicate_store_hour = (
                store is not None
                and unique_store_hours_only
                and isinstance(current_raw, int)
                and current_raw in stored_hours
            )
            sample_index = len(samples)
            snapshot = None
            frame_name = f"collector_started_{sample_index:04d}.json"
            snapshot_name = (
                None if duplicate_store_hour or not write_snapshot_artifacts
                else f"snapshot_{sample_index:04d}.json"
            )
            frame_path = run_dir / frame_name
            _write_json(frame_path, frame)
            if snapshot_name is not None:
                snapshot = snapshot_from_root_probe_frame(frame, game_id)
                snapshot_path = run_dir / snapshot_name
                _write_json(snapshot_path, snapshot)
            if first_frame is None:
                first_frame = frame
                if snapshot is None:
                    snapshot = snapshot_from_root_probe_frame(frame, game_id)
                first_snapshot = snapshot

            if first_game_hour_raw is None and isinstance(current_raw, int):
                first_game_hour_raw = current_raw
            store_path = None
            store_skipped_reason = None
            if store is not None:
                if duplicate_store_hour:
                    store_skipped_reason = "duplicate_hour"
                else:
                    if snapshot is None:
                        snapshot = snapshot_from_root_probe_frame(frame, game_id)
                    store_path = store.write_snapshot(snapshot)
                    if isinstance(current_raw, int):
                        stored_hours.add(current_raw)
            samples.append(
                _sample_manifest(
                    sample_index=sample_index,
                    result=result,
                    root_probe=root_probe,
                    frame=frame,
                    frame_name=frame_name,
                    snapshot_name=snapshot_name,
                    store_path=store_path,
                    store_skipped_reason=store_skipped_reason,
                    worker_burst_index=attach_index,
                    burst_sample_index=burst_sample_index,
                    worker_pipe_token=pipe_token,
                    elapsed_wall_s=time.monotonic() - sample_started,
                )
            )
            if (until_game_hours is not None and first_game_hour_raw is not None
                    and isinstance(current_raw, int)
                    and current_raw - first_game_hour_raw >= until_game_hours):
                stop_reason = "until_game_hours"
                stop_after_attach = True
                break
        if stop_after_attach:
            break

    if first_frame is not None:
        _write_json(run_dir / "collector_started.json", first_frame)
    if first_snapshot is not None and write_snapshot_artifacts:
        _write_json(run_dir / "snapshot.json", first_snapshot)

    elapsed_wall_s = time.monotonic() - started
    summary = _summarize_samples(samples)
    hourly_requirement = hourly_requirement_diagnostics(
        summary,
        required=(until_game_hours is not None
                  if require_hourly_cadence is None else require_hourly_cadence),
        requested_game_hours=until_game_hours,
        elapsed_wall_s=elapsed_wall_s,
        max_wall_s=max_wall_s,
    )
    _write_json(
        manifest_path,
        {
            "type": "collector_probe_manifest",
            "created_utc": datetime.now(timezone.utc).isoformat(),
            "pid": pid,
            "dll": Path(dll).name,
            "dll_sha256": _file_sha256(Path(dll)),
            "session_id": effective_session_id,
            "game_id": game_id,
            "repeat": repeat,
            "interval_s": interval_s,
            "dwell_s": dwell_s,
            "until_game_hours": until_game_hours,
            "max_wall_s": max_wall_s,
            "unique_store_hours_only": unique_store_hours_only,
            "write_snapshot_artifacts": write_snapshot_artifacts,
            "worker_duration_s": worker_duration_s,
            "worker_sample_ms": worker_sample_ms if worker_duration_s is not None else None,
            "country_object_scan": False,
            "country_object_bridge_supplied": bool(encoded_country_object_bridge),
            "country_object_bridge_rows": country_object_bridge_rows,
            "country_record_bridge_supplied": bool(encoded_country_record_bridge),
            "country_record_bridge_rows": country_record_bridge_rows,
            "division_body_bridge_supplied": bool(encoded_division_body_bridge),
            "division_body_bridge_rows": division_body_bridge_rows,
            "map_control_state_roots_supplied": bool(map_control_state_roots),
            "map_control_state_root_rows": map_control_state_root_rows,
            "map_control_province_bridge_supplied": map_control_province_bridge_supplied,
            "map_control_controller_bridge_supplied": bool(map_control_controller_bridge),
            "map_control_controller_bridge_rows": map_control_controller_bridge_rows,
            "scheduled_snapshot_step_hours": (
                scheduled_snapshot_step_hours if worker_duration_s is not None else None
            ),
            "worker_pipe_tokens": worker_pipe_tokens,
            "attach_errors": attach_errors,
            "partial_session": bool(attach_errors and samples),
            "stop_reason": stop_reason,
            "elapsed_wall_s": elapsed_wall_s,
            "raw_pipe_frame_count": len(raw_frames),
            "raw_frames": raw_frames,
            "first_frame": "collector_started.json",
            "first_snapshot": "snapshot.json" if first_snapshot is not None
            and write_snapshot_artifacts else None,
            **summary,
            "hourly_requirement": hourly_requirement,
            "samples": samples,
        },
    )
    return run_dir


def _run_id(pid: int) -> str:
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    return f"{stamp}_pid{pid}"


def _write_json(path: Path, data: Dict[str, Any]) -> None:
    path.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def _file_sha256(path: Path) -> Optional[str]:
    try:
        h = hashlib.sha256()
        with Path(path).open("rb") as f:
            for chunk in iter(lambda: f.read(1024 * 1024), b""):
                h.update(chunk)
        return h.hexdigest()
    except OSError:
        return None


def _attach_with_retries(
    pid: int,
    dll: Path,
    *,
    dwell_s: float,
    attempts: int = 2,
    pipe_token: Optional[str] = None,
) -> Any:
    last_error: Optional[AttachError] = None
    for attempt in range(attempts):
        try:
            return attach_noop(pid, dll, dwell_s=dwell_s, use_pipe=True, pipe_token=pipe_token)
        except AttachError as e:
            last_error = e
            if attempt + 1 >= attempts or "pipe timed out" not in str(e):
                raise
            time.sleep(0.25)
    raise last_error if last_error is not None else RuntimeError("collector attach failed")


def _write_worker_control(
    dll: Path,
    duration_s: float,
    sample_ms: int,
    *,
    session_id: Optional[str] = None,
    country_object_bridge: Optional[str] = None,
    country_record_bridge: Optional[str] = None,
    division_body_bridge: Optional[str] = None,
    map_control_state_roots: Optional[str] = None,
    map_control_province_bridge: Optional[str] = None,
    map_control_controller_bridge: Optional[str] = None,
    scheduled_snapshot_step_hours: int = 1,
) -> tuple[Path, str]:
    control_path = Path(dll).resolve().with_name("collector_control.json")
    pipe_token = f"{time.monotonic_ns():x}"
    payload = {
        "mode": "hourly_poll",
        "duration_ms": int(duration_s * 1000),
        "pipe_token": pipe_token,
        "sample_ms": int(sample_ms),
        "country_object_scan": False,
        "scheduled_snapshot_step_hours": int(scheduled_snapshot_step_hours),
        "sidecar_log": False,
    }
    if session_id:
        payload["session_id"] = session_id
    if country_object_bridge:
        payload["country_object_bridge"] = country_object_bridge
    if country_record_bridge:
        payload["country_record_bridge"] = country_record_bridge
    if division_body_bridge:
        payload["division_body_bridge"] = division_body_bridge
    if map_control_state_roots:
        payload["map_control_state_roots"] = map_control_state_roots
    if map_control_province_bridge:
        payload["map_control_province_bridge"] = map_control_province_bridge
    if map_control_controller_bridge:
        payload["map_control_controller_bridge"] = map_control_controller_bridge
    control_path.write_text(json.dumps(payload, sort_keys=True) + "\n", encoding="utf-8")
    return control_path, pipe_token


def encode_country_object_bridge(rows: Any) -> str:
    """Encode bounded tag-db-id -> country-object pointers for DLL preflight.

    The control file carries only ephemeral memory addresses. Session manifests
    record the count, not the address list.
    """
    normalized = _normalize_country_object_bridge_rows(rows)
    return ";".join(f"{tag_db_id}:0x{object_ptr:X}" for tag_db_id, object_ptr in normalized)


def encode_country_record_bridge(rows: Any) -> str:
    """Encode bounded tag-db-id -> country-record pointers for PP reads."""
    normalized = _normalize_country_record_bridge_rows(rows)
    return ";".join(f"{tag_db_id}:0x{record_ptr:X}" for tag_db_id, record_ptr in normalized)


def _normalize_country_object_bridge_rows(rows: Any) -> List[Tuple[int, int]]:
    return _normalize_country_pointer_bridge_rows(
        rows,
        label="country_object_bridge",
        pointer_keys=("object_ptr", "country_object"),
    )


def _normalize_country_record_bridge_rows(rows: Any) -> List[Tuple[int, int]]:
    return _normalize_country_pointer_bridge_rows(
        rows,
        label="country_record_bridge",
        pointer_keys=("record_ptr", "country_record"),
    )


def _normalize_country_pointer_bridge_rows(
    rows: Any,
    *,
    label: str,
    pointer_keys: Tuple[str, str],
) -> List[Tuple[int, int]]:
    if rows is None:
        return []
    if isinstance(rows, str):
        raw_items: Iterable[Any] = _iter_encoded_country_object_bridge(rows)
    elif isinstance(rows, dict):
        raw_items = rows.items()
    else:
        raw_items = rows

    out: List[Tuple[int, int]] = []
    seen_tag_ids: set[int] = set()
    seen_object_ptrs: set[int] = set()
    for item in raw_items:
        tag_db_id, object_ptr = _bridge_item_pair(item, pointer_keys=pointer_keys, label=label)
        if tag_db_id <= 0 or tag_db_id > 1024:
            raise ValueError(f"{label} tag_db_id out of range")
        if object_ptr <= 0 or object_ptr % 8:
            raise ValueError(f"{label} pointer must be nonzero and 8-byte aligned")
        if tag_db_id in seen_tag_ids:
            raise ValueError(f"{label} duplicate tag_db_id")
        if object_ptr in seen_object_ptrs:
            raise ValueError(f"{label} duplicate pointer")
        seen_tag_ids.add(tag_db_id)
        seen_object_ptrs.add(object_ptr)
        out.append((tag_db_id, object_ptr))
        if len(out) > MAX_COUNTRY_OBJECT_BRIDGE_ROWS:
            raise ValueError(f"{label} exceeds max rows")
    return sorted(out)


def _iter_encoded_country_object_bridge(value: str) -> Iterable[Tuple[str, str]]:
    if not value:
        return []
    pairs = []
    for raw in value.split(";"):
        if not raw:
            raise ValueError("country_object_bridge contains empty row")
        if ":" not in raw:
            raise ValueError("country_object_bridge row missing ':'")
        pairs.append(tuple(raw.split(":", 1)))
    return pairs


def _encoded_row_count(value: Optional[str]) -> int:
    if not value:
        return 0
    return len([row for row in value.split(";") if row])


def _bridge_item_pair(
    item: Any,
    *,
    pointer_keys: Tuple[str, str],
    label: str,
) -> Tuple[int, int]:
    if isinstance(item, dict):
        tag_raw = item.get("tag_db_id")
        ptr_raw = item.get(pointer_keys[0], item.get(pointer_keys[1]))
    else:
        try:
            tag_raw, ptr_raw = item
        except Exception as exc:
            raise ValueError(f"{label} row must be pair-like") from exc
    return _bridge_int(tag_raw, "tag_db_id", label), _bridge_int(ptr_raw, "pointer", label)


def _bridge_int(value: Any, name: str, label: str) -> int:
    if isinstance(value, bool):
        raise ValueError(f"{label} {name} must be int-like")
    if isinstance(value, int):
        return value
    if isinstance(value, str):
        value = value.strip()
        if value.lower().startswith("0x"):
            return int(value, 16)
        return int(value, 10)
    raise ValueError(f"{label} {name} must be int-like")


def _remove_if_exists(path: Path) -> None:
    try:
        path.unlink()
    except FileNotFoundError:
        pass


def _sample_manifest(
    *,
    sample_index: int,
    result: Any,
    root_probe: Dict[str, Any],
    frame: Dict[str, Any],
    frame_name: str,
    snapshot_name: Optional[str],
    store_path: Optional[str],
    store_skipped_reason: Optional[str],
    worker_burst_index: int,
    burst_sample_index: int,
    worker_pipe_token: Optional[str],
    elapsed_wall_s: float,
) -> Dict[str, Any]:
    return {
        "sample_index": sample_index,
        "worker_burst_index": worker_burst_index,
        "burst_sample_index": burst_sample_index,
        "worker_pipe_token": worker_pipe_token,
        "elapsed_wall_s": elapsed_wall_s,
        "thread_id": result.thread_id,
        "quality_status": frame.get("quality_status"),
        "sequence": frame.get("sequence"),
        "transport": frame.get("transport"),
        "transport_crc32": frame.get("transport_crc32"),
        "transport_bytes": frame.get("transport_bytes"),
        "profile": root_probe.get("profile"),
        "domains": root_probe.get("domains") or [],
        "current_game_date": root_probe.get("current_game_date"),
        "current_game_date_raw": root_probe.get("current_game_date_raw"),
        "start_date": root_probe.get("start_date"),
        "start_date_raw": root_probe.get("start_date_raw"),
        "country_tag_count": root_probe.get("country_tag_count"),
        "country_active_count": root_probe.get("country_active_count"),
        "tag_sample": root_probe.get("tag_sample") or [],
        "frame": frame_name,
        "snapshot": snapshot_name,
        "store_path": _manifest_store_ref(store_path),
        "store_skipped_reason": store_skipped_reason,
    }


def _raw_frame_manifest(
    *,
    raw_index: int,
    result: Any,
    frame: Dict[str, Any],
    frame_name: str,
    worker_burst_index: int,
    burst_sample_index: int,
    worker_pipe_token: Optional[str],
    elapsed_wall_s: float,
) -> Dict[str, Any]:
    return {
        "raw_index": raw_index,
        "worker_burst_index": worker_burst_index,
        "burst_sample_index": burst_sample_index,
        "worker_pipe_token": worker_pipe_token,
        "elapsed_wall_s": elapsed_wall_s,
        "thread_id": result.thread_id,
        "type": frame.get("type"),
        "sequence": frame.get("sequence"),
        "domain": frame.get("domain"),
        "event_type": frame.get("event_type"),
        "gap_kind": frame.get("gap_kind"),
        "capture_boundary": frame.get("capture_boundary"),
        "frame": frame_name,
    }


def _is_raw_stream_frame(frame: Dict[str, Any]) -> bool:
    return isinstance(frame, dict) and frame.get("type") in {
        "domain_delta",
        "event",
        "gap",
    }


def _manifest_store_ref(store_path: Optional[str]) -> Optional[str]:
    if not store_path:
        return None
    return Path(store_path).name


def _safe_error_message(exc: Exception) -> str:
    raw = str(exc)
    if raw:
        raw = re.sub(r"[A-Za-z]:\\[^;\n\r]+", "<path>", raw)
        raw = re.sub(r"/[^;\n\r ]+", "<path>", raw)
        raw = raw[:160]
        return "%s:%s" % (type(exc).__name__, raw)
    return type(exc).__name__


def summarize_probe_samples(samples: List[Dict[str, Any]]) -> Dict[str, Any]:
    dates = [s.get("current_game_date") for s in samples
             if isinstance(s.get("current_game_date"), str)]
    raw_dates = [s.get("current_game_date_raw") for s in samples
                 if isinstance(s.get("current_game_date_raw"), int)]
    sequences = [s.get("sequence") for s in samples if isinstance(s.get("sequence"), int)]
    transport_bursts = _transport_burst_summaries(samples)
    unique_dates = []
    for date in dates:
        if date not in unique_dates:
            unique_dates.append(date)
    unique_raw_dates = []
    for raw in raw_dates:
        if raw not in unique_raw_dates:
            unique_raw_dates.append(raw)
    positive_hour_deltas = [
        b - a for a, b in zip(unique_raw_dates, unique_raw_dates[1:]) if b > a
    ]
    skipped_game_hours = sum(max(delta - 1, 0) for delta in positive_hour_deltas)
    duplicate_sequences = len(sequences) - len(set(sequences))
    duplicate_game_hour_samples = len(raw_dates) - len(unique_raw_dates)
    stored_snapshot_count = sum(1 for s in samples if s.get("store_path"))
    duplicate_hour_store_skips = sum(
        1 for s in samples if s.get("store_skipped_reason") == "duplicate_hour"
    )
    return {
        "sample_count": len(samples),
        "first_game_date": dates[0] if dates else None,
        "last_game_date": dates[-1] if dates else None,
        "unique_game_dates": unique_dates,
        "unique_game_date_count": len(unique_dates),
        "monotonic_game_time": all(a <= b for a, b in zip(raw_dates, raw_dates[1:])),
        "game_hours_advanced": (raw_dates[-1] - raw_dates[0])
        if len(raw_dates) >= 2 else 0,
        "positive_game_hour_deltas": positive_hour_deltas,
        "skipped_game_hours": skipped_game_hours,
        "hourly_cadence_ok": skipped_game_hours == 0,
        "duplicate_game_hour_samples": duplicate_game_hour_samples,
        "stored_snapshot_count": stored_snapshot_count,
        "duplicate_hour_store_skips": duplicate_hour_store_skips,
        "first_sequence": sequences[0] if sequences else None,
        "last_sequence": sequences[-1] if sequences else None,
        "monotonic_transport_sequence": all(a < b for a, b in zip(sequences, sequences[1:])),
        "duplicate_transport_sequences": duplicate_sequences,
        "transport_sequence_mode": "burst_local" if transport_bursts else "global",
        "transport_bursts": transport_bursts,
        "burst_local_monotonic_transport_sequence": all(
            b.get("monotonic_sequence") is True for b in transport_bursts
        ) if transport_bursts else all(a < b for a, b in zip(sequences, sequences[1:])),
        "burst_local_duplicate_transport_sequences": sum(
            int(b.get("duplicate_sequences") or 0) for b in transport_bursts
        ),
    }


def _transport_burst_summaries(samples: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
    grouped: Dict[int, List[Dict[str, Any]]] = {}
    for sample in samples:
        burst_index = sample.get("worker_burst_index")
        if not isinstance(burst_index, int):
            continue
        grouped.setdefault(burst_index, []).append(sample)

    bursts: List[Dict[str, Any]] = []
    for burst_index in sorted(grouped):
        burst_samples = grouped[burst_index]
        sequences = [
            s.get("sequence") for s in burst_samples
            if isinstance(s.get("sequence"), int)
        ]
        duplicates = len(sequences) - len(set(sequences))
        nonmonotonic = sum(
            1 for a, b in zip(sequences, sequences[1:]) if b <= a
        )
        pipe_tokens = [
            s.get("worker_pipe_token") for s in burst_samples
            if isinstance(s.get("worker_pipe_token"), str)
        ]
        bursts.append({
            "worker_burst_index": burst_index,
            "sample_count": len(burst_samples),
            "first_sequence": sequences[0] if sequences else None,
            "last_sequence": sequences[-1] if sequences else None,
            "monotonic_sequence": nonmonotonic == 0,
            "nonmonotonic_pairs": nonmonotonic,
            "duplicate_sequences": duplicates,
            "worker_pipe_token": pipe_tokens[0] if pipe_tokens else None,
        })
    return bursts


def hourly_requirement_diagnostics(
    summary: Dict[str, Any],
    *,
    required: bool,
    requested_game_hours: Optional[int] = None,
    elapsed_wall_s: Optional[float] = None,
    max_wall_s: Optional[float] = None,
) -> Dict[str, Any]:
    problems: List[str] = []
    if required:
        if summary.get("sample_count", 0) <= 0:
            problems.append("no_verified_samples")
        if summary.get("monotonic_game_time") is not True:
            problems.append("game_time_not_monotonic")
        if summary.get("hourly_cadence_ok") is not True:
            problems.append("skipped_game_hours")
        if (requested_game_hours is not None
                and summary.get("game_hours_advanced", 0) < requested_game_hours):
            problems.append("requested_game_hours_not_reached")
        if (max_wall_s is not None and elapsed_wall_s is not None
                and elapsed_wall_s > max_wall_s):
            problems.append("max_wall_s_exceeded")
    return {
        "required": required,
        "requested_game_hours": requested_game_hours,
        "max_wall_s": max_wall_s,
        "elapsed_wall_s": elapsed_wall_s,
        "met": not problems,
        "problems": problems,
    }


def _summarize_samples(samples: List[Dict[str, Any]]) -> Dict[str, Any]:
    return summarize_probe_samples(samples)


def _main() -> int:
    parser = argparse.ArgumentParser(
        description="Record collector root-probe frames into smoke artifacts"
    )
    parser.add_argument("--pid", type=int, required=True)
    parser.add_argument("--dll", type=Path, required=True)
    parser.add_argument("--out-root", type=Path, default=Path("replay/snapshots/_collector_probe"))
    parser.add_argument("--store-root", type=Path, default=None)
    parser.add_argument("--game-id", default="collector_probe")
    parser.add_argument("--dwell-s", type=float, default=1.0)
    parser.add_argument("--repeat", type=int, default=1)
    parser.add_argument("--interval-s", type=float, default=0.0)
    parser.add_argument("--until-game-hours", type=int, default=None)
    parser.add_argument("--max-wall-s", type=float, default=None)
    parser.add_argument("--unique-store-hours-only", action="store_true")
    parser.add_argument("--worker-duration-s", type=float, default=None)
    parser.add_argument("--worker-sample-ms", type=int, default=10)
    parser.add_argument(
        "--allow-hourly-gaps",
        action="store_true",
        help="write diagnostics but exit 0 even if requested hourly proof has gaps",
    )
    args = parser.parse_args()

    repeat = args.repeat
    if args.until_game_hours is not None and repeat == 1:
        repeat = 1000
    run_dir = record_probes(args.pid, args.dll, args.out_root, dwell_s=args.dwell_s,
                            game_id=args.game_id, store_root=args.store_root,
                            repeat=repeat, interval_s=args.interval_s,
                            until_game_hours=args.until_game_hours,
                            max_wall_s=args.max_wall_s,
                            unique_store_hours_only=args.unique_store_hours_only,
                            worker_duration_s=args.worker_duration_s,
                            worker_sample_ms=args.worker_sample_ms,
                            require_hourly_cadence=not args.allow_hourly_gaps
                            if args.until_game_hours is not None else False)
    print(run_dir)
    manifest = json.loads((run_dir / "manifest.json").read_text(encoding="utf-8"))
    requirement = manifest.get("hourly_requirement") or {}
    return 0 if requirement.get("met", True) else 2


if __name__ == "__main__":
    raise SystemExit(_main())
