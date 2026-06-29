"""Session-oriented recorder entrypoint for the in-process collector.

This module is the first bridge from the bounded speed-five proof command to a
normal collector recording path. It still uses the proven bounded worker under
the hood, but it wraps that run with explicit session lifecycle artifacts and a
session manifest that avoids local absolute paths.
"""
from __future__ import annotations

import argparse
from dataclasses import dataclass
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import re
import time
from typing import Any, Dict, Iterable, List, Optional, Tuple

from replay import schema

from . import continuity as _continuity
from . import process_identity as _process_identity
from .collector_advisor_tokens import resolve_advisor_token_filter_preflight
from .collector_army_order_bridge import resolve_army_body_bridge_preflight
from .collector_frames import (
    COLLECTOR_BACKEND,
    COLLECTOR_ABI_VERSION,
    latest_verified_root_probe,
    snapshot_from_collector_frame,
    snapshot_from_root_probe_frame,
    validate_collector_frame,
)
from .collector_country_bridge import resolve_country_object_bridge_preflight
from .collector_division_bridge import resolve_division_body_bridge_preflight
from .collector_naval_combat_bridge import (
    resolve_naval_combat_body_bridge_preflight,
)
from .collector_map_bridge import (
    resolve_province_control_preflight,
    resolve_state_control_preflight,
)
from .collector_probe_record import record_probes
from .collector_idea_groups import resolve_idea_group_filter_preflight
from .durable_store import CaptureLock, DurableCaptureStore
from .recorder.cadence import RecorderCadence
from .replay_writer import ReplayRecorder, _verify_container


DEFAULT_OUT_ROOT = Path("replay/snapshots/_collector_session")
DEFAULT_DWELL_S = 0.25
DEFAULT_SAMPLE_MS = 10


@dataclass
class CollectorFrameResult:
    frame_type: Optional[str]
    sequence: Optional[int]
    date: Optional[str]
    hour_index: Optional[int]
    written_path: Optional[str]
    canonical: bool
    skipped_reason: Optional[str] = None
    error: Optional[str] = None


class CollectorFrameSession:
    """Write decoded collector frames through the normal durable replay path."""

    def __init__(
        self,
        *,
        game_id: str,
        store_root: Path,
        replay_path: Optional[Path] = None,
        target_step_hours: int = 1,
        pid: Optional[int] = None,
        process_instance_id: Optional[str] = None,
        modset: Optional[Dict[str, Any]] = None,
        build_exe_sha256: Optional[str] = None,
        build_pinned: bool = True,
        finalize_every: int = 6,
    ) -> None:
        self.game_id = game_id
        self.store = DurableCaptureStore(str(store_root))
        self.pid = pid
        self.process_instance_id = (
            process_instance_id
            or (f"collector-pid-{pid}" if pid is not None else "collector-unknown")
        )
        self.cadence = RecorderCadence(target_step_hours=target_step_hours)
        state = _continuity.ContinuityState.from_dict(
            self.store.load_continuity_state(game_id))
        self.tracker = _continuity.ContinuityTracker(state)
        self.recorder = None
        if replay_path is not None:
            self.recorder = ReplayRecorder(
                store_root=self.store.root,
                game_id=game_id,
                out_path=str(replay_path),
                modset=modset,
                build_exe_sha256=build_exe_sha256,
                build_pinned=build_pinned,
                schema_version=schema.SCHEMA_VERSION,
            )
        self._captures_since_finalize = 0
        self._finalize_every = max(1, int(finalize_every))
        self._results: List[CollectorFrameResult] = []
        self._last_snapshot: Optional[Dict[str, Any]] = None
        self._sequences: List[int] = []
        self._sequence_bursts: Dict[int, List[int]] = {}
        self.invalid_frames = 0
        self.invalid_frame_errors: List[str] = []
        self.finalize_error: Optional[str] = None

    def record_frames(self, frames: Iterable[Dict[str, Any]]) -> List[CollectorFrameResult]:
        lock = CaptureLock(self.store.lock_path(self.game_id))
        lock.acquire(steal=False)
        try:
            for frame in frames:
                self._results.append(self.record_frame(frame))
            self.finalize()
            return list(self._results)
        finally:
            lock.release()

    def record_frame(self, frame: Dict[str, Any]) -> CollectorFrameResult:
        problems = validate_collector_frame(frame)
        if problems:
            self.invalid_frames += 1
            err = "; ".join(problems)
            self.invalid_frame_errors.append(err)
            return CollectorFrameResult(
                frame_type=frame.get("type") if isinstance(frame, dict) else None,
                sequence=None,
                date=None,
                hour_index=None,
                written_path=None,
                canonical=False,
                error=err,
            )

        sequence = frame.get("sequence")
        if isinstance(sequence, int):
            self._sequences.append(sequence)
            worker_burst_index = frame.get("worker_burst_index")
            if isinstance(worker_burst_index, int):
                self._sequence_bursts.setdefault(
                    worker_burst_index, []).append(sequence)
        frame_type = frame.get("type")
        if frame_type not in ("collector_started", "snapshot"):
            return CollectorFrameResult(
                frame_type=frame_type,
                sequence=sequence if isinstance(sequence, int) else None,
                date=frame.get("date") if isinstance(frame.get("date"), str) else None,
                hour_index=None,
                written_path=None,
                canonical=False,
                skipped_reason="non_snapshot_frame",
            )

        try:
            if latest_verified_root_probe([frame]) is not None:
                snap = snapshot_from_root_probe_frame(frame, self.game_id)
            else:
                snap = snapshot_from_collector_frame(frame, self.game_id)
        except Exception as exc:
            self.invalid_frames += 1
            err = str(exc)
            self.invalid_frame_errors.append(err)
            return CollectorFrameResult(
                frame_type=frame_type,
                sequence=sequence if isinstance(sequence, int) else None,
                date=None,
                hour_index=None,
                written_path=None,
                canonical=False,
                error=err,
            )

        decision = self.cadence.plan(
            _CollectorDateInfo(snap.get("date"), trusted=True))
        if not decision.should_capture:
            return CollectorFrameResult(
                frame_type=frame_type,
                sequence=sequence if isinstance(sequence, int) else None,
                date=snap.get("date"),
                hour_index=snap.get("hour_index"),
                written_path=None,
                canonical=False,
                skipped_reason=decision.reason,
            )

        proposal = self.tracker.compute(
            process_instance_id=self.process_instance_id,
            pid=self.pid,
            hour_index=snap.get("hour_index"),
        )
        tags = proposal.tags
        self.tracker.stamp(snap, tags)
        path = self.store.write_snapshot(snap, allow_unknown_hour=False)
        self.tracker.commit(proposal)
        self.store.save_continuity_state(self.game_id,
                                         self.tracker.state.to_dict())
        self.cadence.commit(decision)
        if self.recorder is not None:
            self.recorder.on_snapshot_written(path)
            self._captures_since_finalize += 1
            if self._captures_since_finalize >= self._finalize_every:
                self._captures_since_finalize = 0
                self._finalize_recorder()
        self._last_snapshot = snap
        return CollectorFrameResult(
            frame_type=frame_type,
            sequence=sequence if isinstance(sequence, int) else None,
            date=snap.get("date"),
            hour_index=snap.get("hour_index"),
            written_path=path,
            canonical=True,
        )

    def finalize(self) -> Optional[str]:
        if self.recorder is None:
            return None
        path = self._finalize_recorder()
        if path is not None:
            if not self.patch_recording_diagnostics(path):
                self.finalize_error = "recording diagnostics patch failed"
            else:
                try:
                    if not _verify_container(path):
                        self.finalize_error = "replay verification failed after diagnostics patch"
                except Exception as exc:
                    self.finalize_error = str(exc)
        return path

    def diagnostics(self) -> Dict[str, Any]:
        diagnostics = self.cadence.to_manifest()
        diagnostics["collector"] = {
            "backend": COLLECTOR_BACKEND,
            "session_mode": "collector_frame_session_v0",
            "invalid_frames": int(self.invalid_frames),
            "invalid_frame_errors": list(self.invalid_frame_errors[-8:]),
            "finalize_error": self.finalize_error,
            "transport_sequence": self._transport_sequence_diagnostics(),
        }
        return diagnostics

    def patch_recording_diagnostics(self, replay_path: str) -> bool:
        from .recorder.manifest_diagnostics import patch_manifest_diagnostics

        return patch_manifest_diagnostics(replay_path, self.diagnostics())

    def _finalize_recorder(self) -> Optional[str]:
        if self.recorder is None:
            return None
        try:
            path = self.recorder.finalize()
            try:
                if _verify_container(path):
                    self.finalize_error = None
                else:
                    self.finalize_error = "replay verification failed"
            except Exception as exc:
                self.finalize_error = str(exc)
            return path
        except Exception as exc:
            self.finalize_error = str(exc)
            return None

    def _transport_sequence_diagnostics(self) -> Dict[str, Any]:
        sequences = list(self._sequences)
        duplicates = len(sequences) - len(set(sequences))
        nonmonotonic = sum(
            1 for a, b in zip(sequences, sequences[1:]) if b <= a
        )
        burst_local = []
        for burst_index in sorted(self._sequence_bursts):
            burst_sequences = self._sequence_bursts[burst_index]
            burst_duplicates = len(burst_sequences) - len(set(burst_sequences))
            burst_nonmonotonic = sum(
                1 for a, b in zip(burst_sequences, burst_sequences[1:])
                if b <= a
            )
            burst_local.append({
                "worker_burst_index": burst_index,
                "count": len(burst_sequences),
                "first": burst_sequences[0] if burst_sequences else None,
                "last": burst_sequences[-1] if burst_sequences else None,
                "monotonic": burst_nonmonotonic == 0,
                "nonmonotonic_pairs": burst_nonmonotonic,
                "duplicate_sequences": burst_duplicates,
            })
        burst_local_duplicates = sum(
            int(b.get("duplicate_sequences") or 0) for b in burst_local)
        burst_local_nonmonotonic = sum(
            int(b.get("nonmonotonic_pairs") or 0) for b in burst_local)
        has_bursts = bool(burst_local)
        return {
            "mode": "burst_local" if has_bursts else "global",
            "count": len(sequences),
            "first": sequences[0] if sequences else None,
            "last": sequences[-1] if sequences else None,
            "monotonic": (
                burst_local_nonmonotonic == 0 if has_bursts
                else nonmonotonic == 0
            ),
            "nonmonotonic_pairs": (
                burst_local_nonmonotonic if has_bursts else nonmonotonic
            ),
            "duplicate_sequences": (
                burst_local_duplicates if has_bursts else duplicates
            ),
            "global_monotonic": nonmonotonic == 0,
            "global_nonmonotonic_pairs": nonmonotonic,
            "global_duplicate_sequences": duplicates,
            "burst_local": burst_local,
        }


@dataclass
class _CollectorDateInfo:
    date: Optional[str]
    trusted: bool = True
    source: str = "collector"


def record_collector_session(
    pid: int,
    dll: Path,
    out_root: Path = DEFAULT_OUT_ROOT,
    *,
    game_id: str = "collector_session",
    store_root: Optional[Path] = None,
    duration_s: float = 8.0,
    sample_ms: int = DEFAULT_SAMPLE_MS,
    dwell_s: float = DEFAULT_DWELL_S,
    worker_bursts: int = 1,
    burst_interval_s: float = 0.0,
    until_game_hours: Optional[int] = None,
    max_wall_s: Optional[float] = None,
    require_hourly_cadence: Optional[bool] = None,
    country_object_bridge: Any = None,
    country_record_bridge: Any = None,
    division_body_bridge: Any = None,
    army_body_bridge: Any = None,
    naval_combat_body_bridge: Any = None,
    preflight_country_object_bridge: bool = False,
    allow_country_cold_scan: bool = False,
    preflight_division_movement: bool = False,
    allow_division_cold_scan: bool = False,
    preflight_army_orders: bool = False,
    allow_army_cold_scan: bool = False,
    preflight_naval_combat: bool = False,
    preflight_state_control: bool = False,
    preflight_province_control: bool = False,
    allow_province_cold_scan: bool = False,
) -> Path:
    """Record one collector-backed session and write session lifecycle metadata.

    The current DLL worker is bounded, so ``duration_s`` is the session duration
    for this bridge. Later, the same session manifest shape can wrap a true
    start/stop collector without changing downstream diagnostics.
    """
    if duration_s <= 0:
        raise ValueError("duration_s must be > 0")
    if sample_ms < 10:
        raise ValueError("sample_ms must be >= 10")
    if worker_bursts < 1:
        raise ValueError("worker_bursts must be >= 1")
    if burst_interval_s < 0:
        raise ValueError("burst_interval_s must be >= 0")
    if max_wall_s is not None and max_wall_s <= 0:
        raise ValueError("max_wall_s must be > 0")
    if allow_province_cold_scan and not preflight_province_control:
        raise ValueError("allow_province_cold_scan requires preflight_province_control")
    if allow_country_cold_scan and not preflight_country_object_bridge:
        raise ValueError("allow_country_cold_scan requires preflight_country_object_bridge")
    if allow_division_cold_scan and not preflight_division_movement:
        raise ValueError("allow_division_cold_scan requires preflight_division_movement")
    if allow_army_cold_scan and not preflight_army_orders:
        raise ValueError("allow_army_cold_scan requires preflight_army_orders")

    started_utc = datetime.now(timezone.utc).isoformat()
    started_monotonic = time.monotonic()
    default_wall_s = (
        float(duration_s) * float(worker_bursts)
        + float(burst_interval_s) * float(max(worker_bursts - 1, 0))
        + 7.0
    )
    effective_max_wall_s = (
        max_wall_s if max_wall_s is not None else default_wall_s
    )
    hourly_required = (
        until_game_hours is not None
        if require_hourly_cadence is None else bool(require_hourly_cadence)
    )
    country_object_bridge_preflight = {
        "enabled": bool(preflight_country_object_bridge),
        "status": "not_requested",
        "row_count": 0,
        "allow_cold_scan": bool(allow_country_cold_scan),
    }
    if country_object_bridge is None and preflight_country_object_bridge:
        preflight = resolve_country_object_bridge_preflight(
            pid,
            cache_path=out_root / "_preflight_cache" / "country_array.json",
            allow_cold_scan=allow_country_cold_scan,
        )
        country_object_bridge_preflight = preflight.diagnostics
        country_object_bridge_preflight["enabled"] = True
        country_object_bridge_preflight["allow_cold_scan"] = bool(allow_country_cold_scan)
        if preflight.usable:
            country_object_bridge = preflight.encoded
            country_record_bridge = preflight.record_encoded
    division_body_bridge_preflight = {
        "enabled": bool(preflight_division_movement),
        "status": "not_requested",
        "row_count": 0,
        "allow_cold_scan": bool(allow_division_cold_scan),
    }
    if division_body_bridge is None and preflight_division_movement:
        preflight = resolve_division_body_bridge_preflight(
            pid,
            cache_path=out_root / "_preflight_cache" / "division_bodies.json",
            allow_cold_scan=allow_division_cold_scan,
        )
        division_body_bridge_preflight = preflight.diagnostics
        division_body_bridge_preflight["enabled"] = True
        division_body_bridge_preflight["allow_cold_scan"] = bool(
            allow_division_cold_scan)
        if preflight.usable:
            division_body_bridge = preflight.encoded
    army_body_bridge_preflight = {
        "enabled": bool(preflight_army_orders),
        "status": "not_requested",
        "row_count": 0,
        "allow_cold_scan": bool(allow_army_cold_scan),
    }
    if army_body_bridge is None and preflight_army_orders:
        preflight = resolve_army_body_bridge_preflight(
            pid,
            cache_path=out_root / "_preflight_cache" / "army_bodies.json",
            allow_cold_scan=allow_army_cold_scan,
        )
        army_body_bridge_preflight = preflight.diagnostics
        army_body_bridge_preflight["enabled"] = True
        army_body_bridge_preflight["allow_cold_scan"] = bool(allow_army_cold_scan)
        if preflight.usable:
            army_body_bridge = preflight.encoded
    naval_combat_body_bridge_preflight = {
        "enabled": bool(preflight_naval_combat),
        "status": "not_requested",
        "row_count": 0,
    }
    if naval_combat_body_bridge is None and preflight_naval_combat:
        preflight = resolve_naval_combat_body_bridge_preflight(pid)
        naval_combat_body_bridge_preflight = preflight.diagnostics
        naval_combat_body_bridge_preflight["enabled"] = True
        if preflight.usable:
            naval_combat_body_bridge = preflight.encoded
    map_control_state_roots = None
    map_control_province_bridge = None
    map_control_controller_bridge = None
    state_control_preflight = {
        "enabled": bool(preflight_state_control),
        "status": "not_requested",
        "row_count": 0,
    }
    province_control_preflight = {
        "enabled": bool(preflight_province_control),
        "status": "not_requested",
        "row_count": 0,
    }
    advisor_token_filter_preflight = resolve_advisor_token_filter_preflight(pid)
    advisor_token_filter = (
        advisor_token_filter_preflight.encoded
        if advisor_token_filter_preflight.usable else None
    )
    idea_group_filter_preflight = resolve_idea_group_filter_preflight(pid)
    idea_group_filter = (
        idea_group_filter_preflight.encoded
        if idea_group_filter_preflight.usable else None
    )
    preflight_cache_root = out_root / "_preflight_cache"
    if preflight_state_control:
        preflight = resolve_state_control_preflight(
            pid,
            cache_path=preflight_cache_root / "state_roots.json",
        )
        state_control_preflight = preflight.diagnostics
        state_control_preflight["enabled"] = True
        if preflight.usable:
            map_control_state_roots = getattr(preflight, "state_roots_encoded", None)
    if preflight_province_control:
        preflight = resolve_province_control_preflight(
            pid,
            cache_path=preflight_cache_root / "province_control.json",
            state_cache_path=preflight_cache_root / "state_roots.json",
            allow_cold_scan=allow_province_cold_scan,
        )
        province_control_preflight = preflight.diagnostics
        province_control_preflight["enabled"] = True
        province_control_preflight["allow_cold_scan"] = bool(allow_province_cold_scan)
        if preflight.usable:
            map_control_province_bridge = getattr(
                preflight, "province_bridge_encoded", None)
            map_control_controller_bridge = getattr(
                preflight, "controller_bridge_encoded", None)

    run_dir = record_probes(
        pid,
        dll,
        out_root,
        dwell_s=dwell_s,
        game_id=game_id,
        store_root=None,
        repeat=worker_bursts,
        interval_s=burst_interval_s,
        until_game_hours=until_game_hours,
        max_wall_s=effective_max_wall_s,
        unique_store_hours_only=True,
        worker_duration_s=duration_s,
        worker_sample_ms=sample_ms,
        require_hourly_cadence=hourly_required,
        write_snapshot_artifacts=False,
        country_object_bridge=country_object_bridge,
        country_record_bridge=country_record_bridge,
        division_body_bridge=division_body_bridge,
        army_body_bridge=army_body_bridge,
        naval_combat_body_bridge=naval_combat_body_bridge,
        map_control_state_roots=map_control_state_roots,
        map_control_province_bridge=map_control_province_bridge,
        map_control_controller_bridge=map_control_controller_bridge,
        scheduled_snapshot_step_hours=1,
        advisor_token_filter=advisor_token_filter,
        idea_group_filter=idea_group_filter,
    )

    probe_manifest_path = run_dir / "manifest.json"
    probe_manifest = json.loads(probe_manifest_path.read_text(encoding="utf-8"))
    session_id = _probe_session_id(run_dir, probe_manifest)
    session_store_root = (
        Path(store_root) / session_id if store_root is not None else run_dir / "store"
    )
    replay_path = run_dir / f"{game_id}.battleplan"
    raw_frames = _frames_from_probe_manifest(
        run_dir,
        probe_manifest,
        unique_game_hours_only=False,
    )
    raw_frame_payload_summary = _collector_frame_payload_summary(raw_frames)
    map_control_quality = _map_control_quality_summary(
        state_control_preflight=state_control_preflight,
        province_control_preflight=province_control_preflight,
        probe_manifest=probe_manifest,
        payload_summary=raw_frame_payload_summary,
    )
    raw_stream_transport = _collector_stream_transport_summary(raw_frames)
    frame_stream_path = _write_collector_frame_stream(
        run_dir,
        "collector_frames.jsonl",
        raw_frames,
    )
    frames = _frames_from_probe_manifest(
        run_dir,
        probe_manifest,
        unique_game_hours_only=True,
    )
    build_exe_sha256, modset = _profile_metadata_from_frames(frames)
    frame_session = CollectorFrameSession(
        game_id=game_id,
        store_root=session_store_root,
        replay_path=replay_path if frames else None,
        target_step_hours=1,
        pid=pid,
        process_instance_id=_collector_process_instance_id(
            pid, frames, build_exe_sha256),
        modset=modset,
        build_exe_sha256=build_exe_sha256,
        build_pinned=build_exe_sha256 is not None,
        finalize_every=6,
    )
    if frame_stream_path is not None and frame_session.recorder is not None:
        frame_session.recorder.add_collector_raw_stream(str(frame_stream_path))
    frame_results = frame_session.record_frames(frames)
    stopped_utc = datetime.now(timezone.utc).isoformat()
    elapsed_wall_s = time.monotonic() - started_monotonic
    session_errors = _session_failure_errors(
        probe_manifest=probe_manifest,
        frame_results=frame_results,
        frame_session=frame_session,
        replay_path=replay_path,
        map_control_quality=map_control_quality,
    )

    session_start = _session_frame(
        "session_start",
        session_id=session_id,
        game_id=game_id,
        pid=pid,
        quality_status="session_started",
        started_utc=started_utc,
        duration_ms=_ms(duration_s),
        sample_ms=sample_ms,
        worker_bursts=worker_bursts,
        burst_interval_ms=_ms(burst_interval_s),
    )
    session_stop = _session_frame(
        "session_stop",
        session_id=session_id,
        game_id=game_id,
        pid=pid,
        quality_status=(
            "session_error" if session_errors else _stop_quality(probe_manifest)
        ),
        started_utc=started_utc,
        stopped_utc=stopped_utc,
        elapsed_wall_ms=_ms(elapsed_wall_s),
        stop_reason=probe_manifest.get("stop_reason"),
        sample_count=probe_manifest.get("sample_count", len(frame_results)),
        processed_frame_count=len(frame_results),
        stored_snapshot_count=sum(1 for r in frame_results if r.written_path),
        hourly_requirement=probe_manifest.get("hourly_requirement"),
    )

    _write_json(run_dir / "session_start.json", session_start)
    _write_json(run_dir / "session_stop.json", session_stop)
    _write_json(
        run_dir / "session_manifest.json",
        {
            "type": "collector_session_manifest",
            "session_id": session_id,
            "created_utc": started_utc,
            "stopped_utc": stopped_utc,
            "runtime_source": COLLECTOR_BACKEND,
            "mode": "bounded_worker_session_poll",
            "game_id": game_id,
            "pid": pid,
            "dll": Path(dll).name,
            "duration_ms": _ms(duration_s),
            "sample_ms": sample_ms,
            "dwell_ms": _ms(dwell_s),
            "worker_bursts": worker_bursts,
            "burst_interval_ms": _ms(burst_interval_s),
            "until_game_hours": until_game_hours,
            "max_wall_ms": _ms(effective_max_wall_s),
            "require_hourly_cadence": hourly_required,
            "country_object_bridge_preflight": country_object_bridge_preflight,
            "country_object_bridge_supplied": probe_manifest.get(
                "country_object_bridge_supplied"),
            "country_object_bridge_rows": probe_manifest.get(
                "country_object_bridge_rows"),
            "country_record_bridge_supplied": probe_manifest.get(
                "country_record_bridge_supplied"),
            "country_record_bridge_rows": probe_manifest.get(
                "country_record_bridge_rows"),
            "special_projects_scan": probe_manifest.get("special_projects_scan"),
            "division_body_bridge_preflight": division_body_bridge_preflight,
            "division_body_bridge_supplied": probe_manifest.get(
                "division_body_bridge_supplied"),
            "division_body_bridge_rows": probe_manifest.get(
                "division_body_bridge_rows"),
            "army_body_bridge_preflight": army_body_bridge_preflight,
            "army_body_bridge_supplied": probe_manifest.get(
                "army_body_bridge_supplied"),
            "army_body_bridge_rows": probe_manifest.get(
                "army_body_bridge_rows"),
            "naval_combat_body_bridge_preflight": naval_combat_body_bridge_preflight,
            "naval_combat_body_bridge_supplied": probe_manifest.get(
                "naval_combat_body_bridge_supplied"),
            "naval_combat_body_bridge_rows": probe_manifest.get(
                "naval_combat_body_bridge_rows"),
            "state_control_preflight": state_control_preflight,
            "province_control_preflight": province_control_preflight,
            "map_control_state_roots_supplied": probe_manifest.get(
                "map_control_state_roots_supplied"),
            "map_control_state_root_rows": probe_manifest.get(
                "map_control_state_root_rows"),
            "map_control_province_bridge_supplied": probe_manifest.get(
                "map_control_province_bridge_supplied"),
            "map_control_controller_bridge_supplied": probe_manifest.get(
                "map_control_controller_bridge_supplied"),
            "map_control_controller_bridge_rows": probe_manifest.get(
                "map_control_controller_bridge_rows"),
            "advisor_token_filter_preflight": advisor_token_filter_preflight.diagnostics,
            "advisor_token_filter_supplied": probe_manifest.get(
                "advisor_token_filter_supplied"),
            "advisor_token_filter_tokens": probe_manifest.get(
                "advisor_token_filter_tokens"),
            "advisor_token_filter_sha256": probe_manifest.get(
                "advisor_token_filter_sha256"),
            "idea_group_filter_preflight": idea_group_filter_preflight.diagnostics,
            "idea_group_filter_supplied": probe_manifest.get(
                "idea_group_filter_supplied"),
            "idea_group_filter_rows": probe_manifest.get(
                "idea_group_filter_rows"),
            "idea_group_filter_sha256": probe_manifest.get(
                "idea_group_filter_sha256"),
            "proof_compat_manifest": "manifest.json",
            "session_start_frame": "session_start.json",
            "session_stop_frame": "session_stop.json",
            "collector_frame_stream": frame_stream_path.name if frame_stream_path else None,
            "raw_collector_frame_count": len(raw_frames),
            "duplicate_game_hour_frame_count": max(0, len(raw_frames) - len(frames)),
            "collector_frame_payload_summary": raw_frame_payload_summary,
            "map_control_quality": map_control_quality,
            "canonical_snapshot_step_hours": 1,
            "frame_stream_granularity": {
                "sample_ms": sample_ms,
                "stores_every_valid_collector_frame": True,
                "canonical_snapshots_unique_game_hours_only": True,
            },
            "replay_file": replay_path.name,
            "replay_verified": _safe_verify_container(replay_path),
            "probe_stop_reason": probe_manifest.get("stop_reason"),
            "session_errors": session_errors,
            "elapsed_wall_ms": _ms(elapsed_wall_s),
            "sample_count": probe_manifest.get("sample_count", len(frame_results)),
            "processed_frame_count": len(frame_results),
            "stored_snapshot_count": sum(1 for r in frame_results if r.written_path),
            "unique_game_date_count": probe_manifest.get("unique_game_date_count"),
            "game_hours_advanced": probe_manifest.get("game_hours_advanced"),
            "skipped_game_hours": probe_manifest.get("skipped_game_hours"),
            "hourly_requirement": probe_manifest.get("hourly_requirement"),
            "recording_diagnostics": frame_session.diagnostics(),
            "transport": {
                "worker_pipe_token_count": len(
                    probe_manifest.get("worker_pipe_tokens") or []),
                "sequence_mode": probe_manifest.get("transport_sequence_mode"),
                "monotonic_sequence": probe_manifest.get(
                    "monotonic_transport_sequence"),
                "duplicate_sequences": probe_manifest.get(
                    "duplicate_transport_sequences"),
                "burst_local_monotonic_sequence": probe_manifest.get(
                    "burst_local_monotonic_transport_sequence"),
                "burst_local_duplicate_sequences": probe_manifest.get(
                    "burst_local_duplicate_transport_sequences"),
                "bursts": probe_manifest.get("transport_bursts") or [],
                "first_sequence": probe_manifest.get("first_sequence"),
                "last_sequence": probe_manifest.get("last_sequence"),
                "raw_stream": raw_stream_transport,
            },
        },
    )
    if session_errors:
        raise RuntimeError("collector session failed closed: "
                           + "; ".join(session_errors[:8]))
    return run_dir


def _frames_from_probe_manifest(
    run_dir: Path,
    probe_manifest: Dict[str, Any],
    *,
    unique_game_hours_only: bool = True,
) -> List[Dict[str, Any]]:
    frames: List[Dict[str, Any]] = []
    errors: List[str] = []
    session_id = _probe_session_id(run_dir, probe_manifest)
    samples = probe_manifest.get("samples")
    if not isinstance(samples, list):
        raise RuntimeError("collector probe manifest samples missing or not list")
    entries: List[Tuple[str, int, Dict[str, Any]]] = [
        ("samples", i, sample) for i, sample in enumerate(samples)
    ]
    if not unique_game_hours_only:
        raw_entries = probe_manifest.get("raw_frames") or []
        if not isinstance(raw_entries, list):
            raise RuntimeError("collector probe manifest raw_frames not list")
        entries.extend(
            ("raw_frames", i, sample) for i, sample in enumerate(raw_entries)
        )
        entries.sort(key=lambda item: _manifest_entry_order(item[2]))
    for label, i, sample in entries:
        if not isinstance(sample, dict):
            errors.append(f"{label}[{i}] not object")
            continue
        frame_name = sample.get("frame")
        if not isinstance(frame_name, str):
            errors.append(f"{label}[{i}].frame missing")
            continue
        frame_rel = Path(frame_name)
        if frame_rel.name != frame_name:
            errors.append(f"{label}[{i}].frame unsafe path")
            continue
        frame_path = run_dir / frame_rel
        try:
            frame = json.loads(frame_path.read_text(encoding="utf-8"))
        except OSError as exc:
            errors.append(f"{label}[{i}].frame unreadable: {exc}")
            continue
        except ValueError as exc:
            errors.append(f"{label}[{i}].frame invalid json: {exc}")
            continue
        frame_session_id = frame.get("session_id")
        if frame_session_id is None:
            frame["session_id"] = session_id
        elif frame_session_id != session_id:
            errors.append(
                f"{label}[{i}].frame session_id {frame_session_id!r} != "
                f"{session_id!r}")
            continue
        for key in ("worker_burst_index", "burst_sample_index", "worker_pipe_token"):
            if key in sample and key not in frame:
                frame[key] = sample[key]
        problems = validate_collector_frame(frame)
        if problems:
            errors.append(
                f"{label}[{i}].frame invalid: {'; '.join(problems)}")
            continue
        frames.append(frame)
    if errors:
        raise RuntimeError("collector probe frame load failed: "
                           + "; ".join(errors[:8]))
    if unique_game_hours_only:
        return _prefer_scheduled_frames_by_game_hour(frames)
    return frames


def _manifest_entry_order(entry: Dict[str, Any]) -> Tuple[int, int, int]:
    burst = entry.get("worker_burst_index")
    burst_sample = entry.get("burst_sample_index")
    sample_index = entry.get("sample_index", entry.get("raw_index"))
    return (
        burst if isinstance(burst, int) else 1 << 30,
        burst_sample if isinstance(burst_sample, int) else 1 << 30,
        sample_index if isinstance(sample_index, int) else 1 << 30,
    )


def _prefer_scheduled_frames_by_game_hour(
    frames: Iterable[Dict[str, Any]],
) -> List[Dict[str, Any]]:
    selected: List[Dict[str, Any]] = []
    by_raw_hour: Dict[int, int] = {}
    for frame in frames:
        root_probe = frame.get("root_probe")
        raw_hour = (
            root_probe.get("current_game_date_raw")
            if isinstance(root_probe, dict) else None
        )
        if not isinstance(raw_hour, int):
            selected.append(frame)
            continue
        existing_index = by_raw_hour.get(raw_hour)
        if existing_index is None:
            by_raw_hour[raw_hour] = len(selected)
            selected.append(frame)
            continue
        existing = selected[existing_index]
        if (
            existing.get("capture_boundary") != "scheduled_hour_snapshot"
            and frame.get("capture_boundary") == "scheduled_hour_snapshot"
        ):
            selected[existing_index] = frame
    return selected


def _collector_frame_payload_summary(frames: Iterable[Dict[str, Any]]) -> Dict[str, Any]:
    domains_seen: set[str] = set()
    frames_with_states = 0
    frames_with_provinces = 0
    frames_with_countries = 0
    max_country_rows = 0
    max_state_rows = 0
    max_province_rows = 0
    state_control_frames = 0
    province_control_frames = 0
    state_ownership_frames = 0
    country_scalars_frames = 0
    country_misc_frames = 0
    fuel_live_frames = 0
    focus_live_frames = 0
    tech_live_frames = 0
    projects_live_frames = 0
    politics_live_frames = 0
    politics_residuals_frames = 0
    scheduled_production_domain_frames = 0
    scheduled_focus_domain_frames = 0
    scheduled_tech_domain_frames = 0
    scheduled_projects_domain_frames = 0
    scheduled_politics_domain_frames = 0
    scheduled_politics_residuals_domain_frames = 0
    country_rows_with_scalar_payloads = 0
    country_rows_with_non_null_scalar_values = 0
    max_country_scalar_rows_per_frame = 0
    frames_with_production_payloads = 0
    country_rows_with_production_payloads = 0
    max_production_country_rows_per_frame = 0
    frames_with_focus_payloads = 0
    country_rows_with_focus_payloads = 0
    max_focus_country_rows_per_frame = 0
    max_focus_completed_rows_per_country = 0
    frames_with_research_payloads = 0
    country_rows_with_research_payloads = 0
    max_research_country_rows_per_frame = 0
    max_research_active_rows_per_country = 0
    frames_with_special_projects_payloads = 0
    special_project_facility_rows = 0
    special_project_rows = 0
    special_project_scientist_rows = 0
    max_special_project_facilities_per_frame = 0
    max_special_project_rows_per_frame = 0
    max_special_project_scientists_per_frame = 0
    frames_with_politics_payloads = 0
    country_rows_with_politics_payloads = 0
    max_politics_country_rows_per_frame = 0
    max_politics_idea_rows_per_country = 0
    frames_with_politics_residual_payloads = 0
    country_rows_with_politics_residual_payloads = 0
    max_politics_residual_country_rows_per_frame = 0
    max_production_stockpile_entries = 0
    max_production_military_line_rows = 0
    max_production_naval_line_rows = 0
    production_on_lightweight_frames = 0
    focus_on_lightweight_frames = 0
    tech_on_lightweight_frames = 0
    projects_on_lightweight_frames = 0
    politics_on_lightweight_frames = 0
    politics_residuals_on_lightweight_frames = 0
    frames_by_type: Dict[str, int] = {}
    raw_event_types_seen: set[str] = set()
    raw_delta_domains_seen: set[str] = set()
    raw_gap_domains_seen: set[str] = set()
    country_object_identity_statuses: Dict[str, int] = {}
    country_object_identity_max_object_count = 0
    country_object_identity_max_scan_ms = 0
    frames_by_capture_boundary: Dict[str, int] = {}
    heavy_payload_on_lightweight_frames = 0
    for frame in frames:
        frame_type = frame.get("type")
        if isinstance(frame_type, str):
            frames_by_type[frame_type] = frames_by_type.get(frame_type, 0) + 1
        if frame_type == "event":
            event_type = frame.get("event_type")
            if not isinstance(event_type, str):
                payload = frame.get("payload")
                if isinstance(payload, dict):
                    event_type = payload.get("event_type")
            if isinstance(event_type, str):
                raw_event_types_seen.add(event_type)
        elif frame_type == "domain_delta":
            domain = frame.get("domain")
            if isinstance(domain, str):
                raw_delta_domains_seen.add(domain)
        elif frame_type == "gap":
            domain = frame.get("domain")
            if isinstance(domain, str):
                raw_gap_domains_seen.add(domain)
        boundary = frame.get("capture_boundary")
        if isinstance(boundary, str):
            frames_by_capture_boundary[boundary] = (
                frames_by_capture_boundary.get(boundary, 0) + 1
            )
        domains = frame.get("domains")
        if isinstance(domains, list):
            for domain in domains:
                if isinstance(domain, str):
                    domains_seen.add(domain)
            if "state_control" in domains:
                state_control_frames += 1
            if "province_control" in domains:
                province_control_frames += 1
            if "state_ownership" in domains:
                state_ownership_frames += 1
            if "country_scalars" in domains:
                country_scalars_frames += 1
            if "country_misc" in domains:
                country_misc_frames += 1
            if "fuel_live" in domains:
                fuel_live_frames += 1
            if "focus_live" in domains:
                focus_live_frames += 1
            if "tech_live" in domains:
                tech_live_frames += 1
            if "projects_live" in domains:
                projects_live_frames += 1
            if "politics_live" in domains:
                politics_live_frames += 1
            if "politics_residuals" in domains:
                politics_residuals_frames += 1
            production_domain_claimed = "production_live" in domains
            focus_domain_claimed = "focus_live" in domains
            tech_domain_claimed = "tech_live" in domains
            projects_domain_claimed = "projects_live" in domains
            politics_domain_claimed = "politics_live" in domains
            politics_residuals_domain_claimed = "politics_residuals" in domains
        else:
            production_domain_claimed = False
            focus_domain_claimed = False
            tech_domain_claimed = False
            projects_domain_claimed = False
            politics_domain_claimed = False
            politics_residuals_domain_claimed = False
        root_probe = frame.get("root_probe")
        if isinstance(root_probe, dict):
            identity = root_probe.get("country_object_identity")
            if isinstance(identity, dict):
                status = identity.get("status")
                if isinstance(status, str):
                    country_object_identity_statuses[status] = (
                        country_object_identity_statuses.get(status, 0) + 1)
                object_count = _int_value(identity.get("object_count"))
                country_object_identity_max_object_count = max(
                    country_object_identity_max_object_count, object_count)
                scan_ms = _int_value(identity.get("scan_ms"))
                country_object_identity_max_scan_ms = max(
                    country_object_identity_max_scan_ms, scan_ms)
        if (
            production_domain_claimed
            and boundary == "scheduled_hour_snapshot"
        ):
            scheduled_production_domain_frames += 1
        if (
            focus_domain_claimed
            and boundary == "scheduled_hour_snapshot"
        ):
            scheduled_focus_domain_frames += 1
        if (
            tech_domain_claimed
            and boundary == "scheduled_hour_snapshot"
        ):
            scheduled_tech_domain_frames += 1
        if (
            projects_domain_claimed
            and boundary == "scheduled_hour_snapshot"
        ):
            scheduled_projects_domain_frames += 1
        if (
            politics_domain_claimed
            and boundary == "scheduled_hour_snapshot"
        ):
            scheduled_politics_domain_frames += 1
        if (
            politics_residuals_domain_claimed
            and boundary == "scheduled_hour_snapshot"
        ):
            scheduled_politics_residuals_domain_frames += 1
        countries = frame.get("countries")
        scalar_rows_in_frame = 0
        production_rows_in_frame = 0
        focus_rows_in_frame = 0
        research_rows_in_frame = 0
        politics_rows_in_frame = 0
        politics_residual_rows_in_frame = 0
        heavy_payload_in_lightweight_frame = False
        special_projects = frame.get("special_projects")
        special_projects_payload_in_frame = False
        if isinstance(special_projects, dict):
            facilities = special_projects.get("facilities")
            projects = special_projects.get("projects")
            scientists = special_projects.get("scientists")
            if isinstance(facilities, list):
                special_projects_payload_in_frame = True
                special_project_facility_rows += len(facilities)
                max_special_project_facilities_per_frame = max(
                    max_special_project_facilities_per_frame,
                    len(facilities),
                )
            if isinstance(projects, list):
                special_projects_payload_in_frame = True
                special_project_rows += len(projects)
                max_special_project_rows_per_frame = max(
                    max_special_project_rows_per_frame,
                    len(projects),
                )
            if isinstance(scientists, list):
                special_projects_payload_in_frame = True
                special_project_scientist_rows += len(scientists)
                max_special_project_scientists_per_frame = max(
                    max_special_project_scientists_per_frame,
                    len(scientists),
                )
        if special_projects_payload_in_frame:
            frames_with_special_projects_payloads += 1
        if isinstance(countries, dict):
            frames_with_countries += 1
            max_country_rows = max(max_country_rows, len(countries))
            for row in countries.values():
                if not isinstance(row, dict):
                    continue
                scalars = row.get("scalars")
                if isinstance(scalars, dict):
                    scalar_rows_in_frame += 1
                    if any(
                        isinstance(value, (int, float)) and not isinstance(value, bool)
                        for value in scalars.values()
                    ):
                        country_rows_with_non_null_scalar_values += 1
                politics = row.get("politics")
                if politics is not None:
                    politics_rows_in_frame += 1
                    if isinstance(politics, dict):
                        if (
                            "last_election" in politics
                            or "elections_allowed" in politics
                        ):
                            politics_residual_rows_in_frame += 1
                        ideas = politics.get("ideas")
                        if isinstance(ideas, list):
                            max_politics_idea_rows_per_country = max(
                                max_politics_idea_rows_per_country, len(ideas))
                focus = row.get("focus")
                if focus is not None:
                    focus_rows_in_frame += 1
                    if isinstance(focus, dict):
                        completed = focus.get("completed")
                        if isinstance(completed, list):
                            max_focus_completed_rows_per_country = max(
                                max_focus_completed_rows_per_country,
                                len(completed),
                            )
                research = row.get("research")
                if research is not None:
                    research_rows_in_frame += 1
                    if isinstance(research, dict):
                        active_research = research.get("active_research")
                        if isinstance(active_research, list):
                            max_research_active_rows_per_country = max(
                                max_research_active_rows_per_country,
                                len(active_research),
                            )
                production = row.get("production")
                if production is None:
                    continue
                production_rows_in_frame += 1
                if not isinstance(production, dict):
                    continue
                stockpile = production.get("stockpile")
                if isinstance(stockpile, dict):
                    max_production_stockpile_entries = max(
                        max_production_stockpile_entries, len(stockpile))
                military_lines = production.get("military_lines")
                if isinstance(military_lines, list):
                    max_production_military_line_rows = max(
                        max_production_military_line_rows, len(military_lines))
                naval_lines = production.get("naval_lines")
                if isinstance(naval_lines, list):
                    max_production_naval_line_rows = max(
                        max_production_naval_line_rows, len(naval_lines))
        if scalar_rows_in_frame:
            country_rows_with_scalar_payloads += scalar_rows_in_frame
            max_country_scalar_rows_per_frame = max(
                max_country_scalar_rows_per_frame,
                scalar_rows_in_frame,
            )
        if production_rows_in_frame:
            frames_with_production_payloads += 1
            country_rows_with_production_payloads += production_rows_in_frame
            max_production_country_rows_per_frame = max(
                max_production_country_rows_per_frame,
                production_rows_in_frame,
            )
        if focus_rows_in_frame:
            frames_with_focus_payloads += 1
            country_rows_with_focus_payloads += focus_rows_in_frame
            max_focus_country_rows_per_frame = max(
                max_focus_country_rows_per_frame,
                focus_rows_in_frame,
            )
        if research_rows_in_frame:
            frames_with_research_payloads += 1
            country_rows_with_research_payloads += research_rows_in_frame
            max_research_country_rows_per_frame = max(
                max_research_country_rows_per_frame,
                research_rows_in_frame,
            )
        if politics_rows_in_frame:
            frames_with_politics_payloads += 1
            country_rows_with_politics_payloads += politics_rows_in_frame
            max_politics_country_rows_per_frame = max(
                max_politics_country_rows_per_frame,
                politics_rows_in_frame,
            )
        if politics_residual_rows_in_frame:
            frames_with_politics_residual_payloads += 1
            country_rows_with_politics_residual_payloads += (
                politics_residual_rows_in_frame)
            max_politics_residual_country_rows_per_frame = max(
                max_politics_residual_country_rows_per_frame,
                politics_residual_rows_in_frame,
            )
        if (
            boundary == "worker_poll"
            and (production_domain_claimed or production_rows_in_frame)
        ):
            production_on_lightweight_frames += 1
            heavy_payload_in_lightweight_frame = True
        if (
            boundary == "worker_poll"
            and (focus_domain_claimed or focus_rows_in_frame)
        ):
            focus_on_lightweight_frames += 1
            heavy_payload_in_lightweight_frame = True
        if (
            boundary == "worker_poll"
            and (tech_domain_claimed or research_rows_in_frame)
        ):
            tech_on_lightweight_frames += 1
            heavy_payload_in_lightweight_frame = True
        if (
            boundary == "worker_poll"
            and (projects_domain_claimed or special_projects_payload_in_frame)
        ):
            projects_on_lightweight_frames += 1
            heavy_payload_in_lightweight_frame = True
        if (
            boundary == "worker_poll"
            and (politics_domain_claimed or politics_rows_in_frame)
        ):
            politics_on_lightweight_frames += 1
            heavy_payload_in_lightweight_frame = True
        if (
            boundary == "worker_poll"
            and (
                politics_residuals_domain_claimed
                or politics_residual_rows_in_frame
            )
        ):
            politics_residuals_on_lightweight_frames += 1
            heavy_payload_in_lightweight_frame = True
        states = frame.get("states")
        if isinstance(states, dict):
            frames_with_states += 1
            max_state_rows = max(max_state_rows, len(states))
            if states and boundary == "worker_poll":
                heavy_payload_in_lightweight_frame = True
        provinces = frame.get("provinces")
        if isinstance(provinces, dict):
            frames_with_provinces += 1
            max_province_rows = max(max_province_rows, len(provinces))
            if provinces and boundary == "worker_poll":
                heavy_payload_in_lightweight_frame = True
        if boundary == "worker_poll" and heavy_payload_in_lightweight_frame:
            heavy_payload_on_lightweight_frames += 1
    return {
        "frames_by_type": dict(sorted(frames_by_type.items())),
        "raw_event_frame_count": frames_by_type.get("event", 0),
        "raw_domain_delta_frame_count": frames_by_type.get("domain_delta", 0),
        "raw_gap_frame_count": frames_by_type.get("gap", 0),
        "raw_event_types_seen": sorted(raw_event_types_seen),
        "raw_delta_domains_seen": sorted(raw_delta_domains_seen),
        "raw_gap_domains_seen": sorted(raw_gap_domains_seen),
        "domains_seen": sorted(domains_seen),
        "frames_by_capture_boundary": dict(sorted(frames_by_capture_boundary.items())),
        "scheduled_snapshot_frame_count": frames_by_capture_boundary.get(
            "scheduled_hour_snapshot", 0),
        "lightweight_poll_frame_count": frames_by_capture_boundary.get(
            "worker_poll", 0),
        "heavy_payload_on_lightweight_frames": heavy_payload_on_lightweight_frames,
        "frames_with_countries": frames_with_countries,
        "frames_with_states": frames_with_states,
        "frames_with_provinces": frames_with_provinces,
        "max_country_rows": max_country_rows,
        "country_scalars_frames": country_scalars_frames,
        "country_misc_frames": country_misc_frames,
        "fuel_live_frames": fuel_live_frames,
        "focus_live_frames": focus_live_frames,
        "tech_live_frames": tech_live_frames,
        "projects_live_frames": projects_live_frames,
        "politics_live_frames": politics_live_frames,
        "politics_residuals_frames": politics_residuals_frames,
        "country_rows_with_scalar_payloads": country_rows_with_scalar_payloads,
        "country_rows_with_non_null_scalar_values": (
            country_rows_with_non_null_scalar_values),
        "max_country_scalar_rows_per_frame": max_country_scalar_rows_per_frame,
        "country_object_identity_statuses": dict(sorted(
            country_object_identity_statuses.items())),
        "country_object_identity_max_object_count": (
            country_object_identity_max_object_count),
        "country_object_identity_max_scan_ms": country_object_identity_max_scan_ms,
        "max_state_rows": max_state_rows,
        "max_province_rows": max_province_rows,
        "state_ownership_frames": state_ownership_frames,
        "state_control_frames": state_control_frames,
        "province_control_frames": province_control_frames,
        "scheduled_production_domain_frames": scheduled_production_domain_frames,
        "scheduled_focus_domain_frames": scheduled_focus_domain_frames,
        "scheduled_tech_domain_frames": scheduled_tech_domain_frames,
        "scheduled_projects_domain_frames": scheduled_projects_domain_frames,
        "scheduled_politics_domain_frames": scheduled_politics_domain_frames,
        "scheduled_politics_residuals_domain_frames": (
            scheduled_politics_residuals_domain_frames),
        "frames_with_production_payloads": frames_with_production_payloads,
        "country_rows_with_production_payloads": (
            country_rows_with_production_payloads),
        "max_production_country_rows_per_frame": (
            max_production_country_rows_per_frame),
        "max_production_stockpile_entries": max_production_stockpile_entries,
        "max_production_military_line_rows": max_production_military_line_rows,
        "max_production_naval_line_rows": max_production_naval_line_rows,
        "production_on_lightweight_frames": production_on_lightweight_frames,
        "frames_with_focus_payloads": frames_with_focus_payloads,
        "country_rows_with_focus_payloads": country_rows_with_focus_payloads,
        "max_focus_country_rows_per_frame": max_focus_country_rows_per_frame,
        "max_focus_completed_rows_per_country": (
            max_focus_completed_rows_per_country),
        "focus_on_lightweight_frames": focus_on_lightweight_frames,
        "frames_with_research_payloads": frames_with_research_payloads,
        "country_rows_with_research_payloads": (
            country_rows_with_research_payloads),
        "max_research_country_rows_per_frame": (
            max_research_country_rows_per_frame),
        "max_research_active_rows_per_country": (
            max_research_active_rows_per_country),
        "tech_on_lightweight_frames": tech_on_lightweight_frames,
        "frames_with_special_projects_payloads": (
            frames_with_special_projects_payloads),
        "special_project_facility_rows": special_project_facility_rows,
        "special_project_rows": special_project_rows,
        "special_project_scientist_rows": special_project_scientist_rows,
        "max_special_project_facilities_per_frame": (
            max_special_project_facilities_per_frame),
        "max_special_project_rows_per_frame": max_special_project_rows_per_frame,
        "max_special_project_scientists_per_frame": (
            max_special_project_scientists_per_frame),
        "projects_on_lightweight_frames": projects_on_lightweight_frames,
        "frames_with_politics_payloads": frames_with_politics_payloads,
        "country_rows_with_politics_payloads": (
            country_rows_with_politics_payloads),
        "max_politics_country_rows_per_frame": (
            max_politics_country_rows_per_frame),
        "max_politics_idea_rows_per_country": max_politics_idea_rows_per_country,
        "politics_on_lightweight_frames": politics_on_lightweight_frames,
        "frames_with_politics_residual_payloads": (
            frames_with_politics_residual_payloads),
        "country_rows_with_politics_residual_payloads": (
            country_rows_with_politics_residual_payloads),
        "max_politics_residual_country_rows_per_frame": (
            max_politics_residual_country_rows_per_frame),
        "politics_residuals_on_lightweight_frames": (
            politics_residuals_on_lightweight_frames),
    }


def _map_control_quality_summary(
    *,
    state_control_preflight: Dict[str, Any],
    province_control_preflight: Dict[str, Any],
    probe_manifest: Dict[str, Any],
    payload_summary: Dict[str, Any],
) -> Dict[str, Any]:
    scheduled_frames = _int_value(
        payload_summary.get("scheduled_snapshot_frame_count"))
    requested_domains: List[str] = []
    captured_domains: List[str] = []
    omitted_domains: Dict[str, str] = {}

    if state_control_preflight.get("enabled"):
        requested_domains.extend(["state_ownership", "state_control"])
        reason = _map_control_request_blocker(
            state_control_preflight,
            supplied=bool(probe_manifest.get("map_control_state_roots_supplied")),
            supplied_label="state_roots_not_supplied",
            scheduled_frames=scheduled_frames,
        )
        if reason is not None:
            omitted_domains["state_ownership"] = reason
            omitted_domains["state_control"] = reason
        else:
            if _int_value(payload_summary.get("state_ownership_frames")) > 0:
                captured_domains.append("state_ownership")
            else:
                omitted_domains["state_ownership"] = "no_scheduled_domain_frames"
            if _int_value(payload_summary.get("state_control_frames")) > 0:
                captured_domains.append("state_control")
            else:
                omitted_domains["state_control"] = "no_scheduled_domain_frames"

    if province_control_preflight.get("enabled"):
        requested_domains.append("province_control")
        reason = _map_control_request_blocker(
            province_control_preflight,
            supplied=(
                bool(probe_manifest.get("map_control_province_bridge_supplied"))
                and bool(probe_manifest.get("map_control_controller_bridge_supplied"))
            ),
            supplied_label="province_or_controller_bridge_not_supplied",
            scheduled_frames=scheduled_frames,
        )
        if reason is not None:
            omitted_domains["province_control"] = reason
        elif _int_value(payload_summary.get("province_control_frames")) > 0:
            captured_domains.append("province_control")
        else:
            omitted_domains["province_control"] = "no_scheduled_domain_frames"

    observed_map_domains = (
        ("state_ownership", "state_ownership_frames"),
        ("state_control", "state_control_frames"),
        ("province_control", "province_control_frames"),
    )
    for domain, count_key in observed_map_domains:
        if _int_value(payload_summary.get(count_key)) > 0:
            captured_domains.append(domain)
            omitted_domains.pop(domain, None)

    heavy_payload_on_lightweight_frames = _int_value(
        payload_summary.get("heavy_payload_on_lightweight_frames"))
    fail_closed = bool(omitted_domains) or heavy_payload_on_lightweight_frames > 0
    return {
        "requested_domains": sorted(set(requested_domains)),
        "captured_domains": sorted(set(captured_domains)),
        "omitted_domains": dict(sorted(omitted_domains.items())),
        "scheduled_snapshot_frame_count": scheduled_frames,
        "state_control_frames": _int_value(
            payload_summary.get("state_control_frames")),
        "state_ownership_frames": _int_value(
            payload_summary.get("state_ownership_frames")),
        "province_control_frames": _int_value(
            payload_summary.get("province_control_frames")),
        "map_control_state_roots_supplied": bool(
            probe_manifest.get("map_control_state_roots_supplied")),
        "map_control_state_root_rows": _int_value(
            probe_manifest.get("map_control_state_root_rows")),
        "map_control_province_bridge_supplied": bool(
            probe_manifest.get("map_control_province_bridge_supplied")),
        "map_control_controller_bridge_supplied": bool(
            probe_manifest.get("map_control_controller_bridge_supplied")),
        "map_control_controller_bridge_rows": _int_value(
            probe_manifest.get("map_control_controller_bridge_rows")),
        "heavy_payload_on_lightweight_frames": heavy_payload_on_lightweight_frames,
        "fail_closed": fail_closed,
    }


def _map_control_request_blocker(
    preflight: Dict[str, Any],
    *,
    supplied: bool,
    supplied_label: str,
    scheduled_frames: int,
) -> Optional[str]:
    status = preflight.get("status")
    if status != "resolved":
        return "preflight_unresolved:%s" % (status or "unknown")
    if not supplied:
        return supplied_label
    if scheduled_frames <= 0:
        return "no_scheduled_snapshot_frames"
    return None


def _int_value(value: Any) -> int:
    return value if isinstance(value, int) and not isinstance(value, bool) else 0


def _collector_stream_transport_summary(
    frames: Iterable[Dict[str, Any]],
) -> Dict[str, Any]:
    frame_list = list(frames)
    sequences = [
        frame.get("sequence") for frame in frame_list
        if isinstance(frame.get("sequence"), int)
    ]
    by_burst: Dict[int, List[int]] = {}
    for frame in frame_list:
        burst = frame.get("worker_burst_index")
        sequence = frame.get("sequence")
        if isinstance(burst, int) and isinstance(sequence, int):
            by_burst.setdefault(burst, []).append(sequence)
    burst_summaries = [
        {
            "worker_burst_index": burst,
            **_sequence_summary(seq),
        }
        for burst, seq in sorted(by_burst.items())
    ]
    collector_started_count = sum(
        1 for frame in frame_list if frame.get("type") == "collector_started"
    )
    return {
        "frame_count": len(frame_list),
        "sequence_count": len(sequences),
        "mode": "burst_local" if burst_summaries else "global",
        **_sequence_summary(sequences),
        "collector_started_frame_count": collector_started_count,
        "average_frames_per_collector_started": (
            len(frame_list) / collector_started_count
            if collector_started_count else None
        ),
        "burst_local": burst_summaries,
        "burst_local_monotonic_sequence": all(
            item["monotonic_sequence"] for item in burst_summaries
        ) if burst_summaries else _sequence_summary(sequences)["monotonic_sequence"],
        "burst_local_duplicate_sequences": sum(
            int(item["duplicate_sequences"]) for item in burst_summaries
        ),
        "burst_local_missing_sequences": sum(
            int(item["missing_sequences"]) for item in burst_summaries
        ),
        "burst_local_max_sequence_gap": max(
            (int(item["max_sequence_gap"]) for item in burst_summaries),
            default=_sequence_summary(sequences)["max_sequence_gap"],
        ),
    }


def _sequence_summary(sequences: List[int]) -> Dict[str, Any]:
    duplicates = len(sequences) - len(set(sequences))
    nonmonotonic = sum(
        1 for a, b in zip(sequences, sequences[1:]) if b <= a
    )
    positive_gaps = [
        max(b - a - 1, 0) for a, b in zip(sequences, sequences[1:]) if b > a
    ]
    return {
        "count": len(sequences),
        "first_sequence": sequences[0] if sequences else None,
        "last_sequence": sequences[-1] if sequences else None,
        "monotonic_sequence": nonmonotonic == 0,
        "nonmonotonic_pairs": nonmonotonic,
        "duplicate_sequences": duplicates,
        "missing_sequences": sum(positive_gaps),
        "max_sequence_gap": max(positive_gaps, default=0),
    }


def _write_collector_frame_stream(
    run_dir: Path,
    filename: str,
    frames: Iterable[Dict[str, Any]],
) -> Optional[Path]:
    frame_list = list(frames)
    if not frame_list:
        return None
    path = run_dir / filename
    with path.open("w", encoding="utf-8", newline="\n") as fh:
        for frame in frame_list:
            fh.write(json.dumps(frame, sort_keys=True, separators=(",", ":")))
            fh.write("\n")
    return path


def _profile_metadata_from_frames(
    frames: Iterable[Dict[str, Any]],
) -> tuple[Optional[str], Optional[Dict[str, Any]]]:
    profile_label: Optional[str] = None
    for frame in frames:
        root_probe = latest_verified_root_probe([frame])
        if isinstance(root_probe, dict) and isinstance(root_probe.get("profile"), str):
            profile_label = root_probe["profile"]
            break
    if not profile_label:
        return None, None
    profile_path = Path(__file__).with_name("collector_profiles") / f"{profile_label}.json"
    try:
        profile = json.loads(profile_path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None, None
    exe_sha = profile.get("exe_sha256")
    if not (
        isinstance(exe_sha, str)
        and len(exe_sha) == 64
        and all(c in "0123456789abcdefABCDEF" for c in exe_sha)
    ):
        exe_sha = None
    modset = profile.get("mod_profile")
    return exe_sha, modset if isinstance(modset, dict) else None


def _probe_session_id(run_dir: Path, probe_manifest: Dict[str, Any]) -> str:
    value = probe_manifest.get("session_id")
    if isinstance(value, str) and value:
        return value
    return run_dir.name


def _collector_process_instance_id(
    pid: int,
    frames: Iterable[Dict[str, Any]],
    build_exe_sha256: Optional[str],
) -> str:
    exe_sha256 = build_exe_sha256
    if exe_sha256 is None:
        for frame in frames:
            value = frame.get("exe_sha256")
            if isinstance(value, str) and value:
                exe_sha256 = value
                break
    create_time = _process_identity._process_create_time_100ns(pid)
    if create_time is not None and exe_sha256:
        payload = "%s|%d|%d" % (exe_sha256, int(pid), int(create_time))
        return hashlib.sha256(payload.encode("utf-8")).hexdigest()[:16]
    return f"collector-pid-{pid}"


def _session_failure_errors(
    *,
    probe_manifest: Dict[str, Any],
    frame_results: List[CollectorFrameResult],
    frame_session: CollectorFrameSession,
    replay_path: Path,
    map_control_quality: Optional[Dict[str, Any]] = None,
) -> List[str]:
    errors: List[str] = []
    attach_errors = probe_manifest.get("attach_errors")
    if isinstance(attach_errors, list) and attach_errors:
        errors.append(
            "attach_errors:" + "|".join(
                str(e)[:160] for e in attach_errors[:4]))
    if frame_session.invalid_frames:
        errors.append(f"invalid_frames:{frame_session.invalid_frames}")
    if frame_session.finalize_error:
        errors.append(f"finalize_error:{_safe_error_text(frame_session.finalize_error)}")
    if not frame_results and probe_manifest.get("sample_count", 0):
        errors.append("no_manifest_frames_loaded")
    expected_unique = probe_manifest.get("unique_game_date_count")
    written_count = sum(1 for result in frame_results if result.written_path)
    if isinstance(expected_unique, int) and written_count < expected_unique:
        errors.append(
            f"durable_snapshot_count:{written_count}<unique_game_date_count:{expected_unique}")
    result_errors = [r.error for r in frame_results if r.error]
    if result_errors:
        errors.append("frame_result_errors:" + "|".join(result_errors[:4]))
    if isinstance(map_control_quality, dict):
        omitted = map_control_quality.get("omitted_domains")
        if isinstance(omitted, dict) and omitted:
            parts = [
                f"{domain}:{reason}"
                for domain, reason in sorted(omitted.items())
            ]
            errors.append("map_control_omitted:" + "|".join(parts[:4]))
        heavy_light = map_control_quality.get("heavy_payload_on_lightweight_frames")
        if isinstance(heavy_light, int) and heavy_light > 0:
            errors.append(f"map_control_lightweight_payload:{heavy_light}")
    if not replay_path.exists():
        errors.append("replay_file_missing")
    elif not _safe_verify_container(replay_path):
        errors.append("replay_verification_failed")
    return errors


def _safe_verify_container(replay_path: Path) -> bool:
    try:
        return _verify_container(str(replay_path))
    except Exception:
        return False


def _safe_error_text(value: str) -> str:
    value = re.sub(r"[A-Za-z]:\\[^;\n\r]+", "<path>", str(value))
    value = re.sub(r"/[^;\n\r ]+", "<path>", value)
    return value[:200]


def _session_frame(frame_type: str, **fields: Any) -> Dict[str, Any]:
    frame: Dict[str, Any] = {
        "type": frame_type,
        "backend": COLLECTOR_BACKEND,
        "abi_version": COLLECTOR_ABI_VERSION,
        "domains": [],
    }
    frame.update(fields)
    return frame


def _ms(seconds: float) -> int:
    return int(round(max(0.0, float(seconds)) * 1000.0))


def _stop_quality(probe_manifest: Dict[str, Any]) -> str:
    requirement = probe_manifest.get("hourly_requirement")
    if isinstance(requirement, dict) and requirement.get("required"):
        return "session_complete" if requirement.get("met") else "session_gap"
    if probe_manifest.get("sample_count", 0):
        return "session_complete"
    return "session_no_samples"


def _write_json(path: Path, data: Dict[str, Any]) -> None:
    path.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n",
                    encoding="utf-8")


def _main() -> int:
    parser = argparse.ArgumentParser(
        description="Record a collector-backed session using the in-process DLL"
    )
    parser.add_argument("--pid", type=int, required=True)
    parser.add_argument("--dll", type=Path, required=True)
    parser.add_argument("--out-root", type=Path, default=DEFAULT_OUT_ROOT)
    parser.add_argument("--store-root", type=Path, default=None)
    parser.add_argument("--game-id", default="collector_session")
    parser.add_argument("--duration-s", type=float, default=8.0)
    parser.add_argument("--sample-ms", type=int, default=DEFAULT_SAMPLE_MS)
    parser.add_argument("--dwell-s", type=float, default=DEFAULT_DWELL_S)
    parser.add_argument("--worker-bursts", type=int, default=1)
    parser.add_argument("--burst-interval-s", type=float, default=0.0)
    parser.add_argument("--until-game-hours", type=int, default=None)
    parser.add_argument("--max-wall-s", type=float, default=None)
    parser.add_argument(
        "--preflight-country-object-bridge",
        action="store_true",
        help="resolve live country object pointers before recording; no saves required",
    )
    parser.add_argument(
        "--allow-country-cold-scan",
        action="store_true",
        help="allow slow country-array heap locators during preflight",
    )
    parser.add_argument(
        "--preflight-division-movement",
        action="store_true",
        help="resolve live division bodies before recording movement deltas",
    )
    parser.add_argument(
        "--allow-division-cold-scan",
        action="store_true",
        help="allow the slow division body locator during preflight",
    )
    parser.add_argument(
        "--preflight-army-orders",
        action="store_true",
        help="resolve live army bodies before recording battle-plan order deltas",
    )
    parser.add_argument(
        "--allow-army-cold-scan",
        action="store_true",
        help="allow the slow army body locator during preflight",
    )
    parser.add_argument(
        "--preflight-naval-combat",
        action="store_true",
        help="resolve active naval combat bodies before recording raw naval combat deltas",
    )
    parser.add_argument(
        "--preflight-state-control",
        action="store_true",
        help="resolve/cache live state owner/controller roots before recording",
    )
    parser.add_argument(
        "--preflight-province-control",
        action="store_true",
        help="use cached live province-control bridge if available; no cold scan by default",
    )
    parser.add_argument(
        "--allow-province-cold-scan",
        action="store_true",
        help="allow the slow province array locator during preflight",
    )
    parser.add_argument(
        "--allow-hourly-gaps",
        action="store_true",
        help="write diagnostics but exit 0 even if requested hourly coverage has gaps",
    )
    args = parser.parse_args()

    run_dir = record_collector_session(
        args.pid,
        args.dll,
        args.out_root,
        game_id=args.game_id,
        store_root=args.store_root,
        duration_s=args.duration_s,
        sample_ms=args.sample_ms,
        dwell_s=args.dwell_s,
        worker_bursts=args.worker_bursts,
        burst_interval_s=args.burst_interval_s,
        until_game_hours=args.until_game_hours,
        max_wall_s=args.max_wall_s,
        require_hourly_cadence=(
            not args.allow_hourly_gaps if args.until_game_hours is not None else False
        ),
        preflight_country_object_bridge=args.preflight_country_object_bridge,
        allow_country_cold_scan=args.allow_country_cold_scan,
        preflight_division_movement=args.preflight_division_movement,
        allow_division_cold_scan=args.allow_division_cold_scan,
        preflight_army_orders=args.preflight_army_orders,
        allow_army_cold_scan=args.allow_army_cold_scan,
        preflight_naval_combat=args.preflight_naval_combat,
        preflight_state_control=args.preflight_state_control,
        preflight_province_control=args.preflight_province_control,
        allow_province_cold_scan=args.allow_province_cold_scan,
    )
    print(run_dir)
    manifest = json.loads((run_dir / "session_manifest.json").read_text(
        encoding="utf-8"))
    requirement = manifest.get("hourly_requirement") or {}
    return 0 if requirement.get("met", True) else 2


__all__ = [
    "CollectorFrameResult",
    "CollectorFrameSession",
    "DEFAULT_DWELL_S",
    "DEFAULT_OUT_ROOT",
    "DEFAULT_SAMPLE_MS",
    "record_collector_session",
]


if __name__ == "__main__":
    raise SystemExit(_main())
