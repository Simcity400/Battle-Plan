"""Movement-only raw delta tracker for decoded live division rows.

This module deliberately does not enumerate or read process memory. It accepts
already-decoded rows from the warmed/pinned division reader path and turns
movement-signature changes into bounded raw collector ``domain_delta`` frames.
"""
from __future__ import annotations

import math
from dataclasses import dataclass
from typing import Any, Dict, Iterable, Mapping, Optional, Tuple

from live_capture.collector_frames import COLLECTOR_ABI_VERSION, COLLECTOR_BACKEND
from replay import schema


DEFAULT_DIVISION_TYPE = 4713
DIVISIONS_DOMAIN = "divisions"
RAW_SOURCE = "C"
RAW_CONFIDENCE = "sampled"

MAX_DIVISION_ID = 10_000_000
MAX_DIVISION_TYPE = 10_000_000
MAX_PROVINCE_ID = 1_000_000
MAX_PROGRESS = 1.0e6

EMITTED_FIELD_ORDER = (
    "div_uid",
    "div_id",
    "div_type",
    "moving",
    "movement_source_province",
    "move_target_province",
    "location",
    "progress_bucket",
    "movement_progress",
    "owner",
)

MOVEMENT_SIGNATURE_FIELDS = (
    "moving",
    "movement_source_province",
    "move_target_province",
    "location",
    "progress_bucket",
)

OPTIONAL_FRAME_KEYS = (
    "session_id",
    "session_sequence",
    "segment_id",
    "process_id",
    "exe_sha256",
    "mod_checksum",
    "profile_id",
    "quality_status",
    "transport",
    "transport_crc32",
    "transport_bytes",
    "duration_ms",
    "elapsed_wall_ms",
)


@dataclass
class _TrackedDivision:
    state: Dict[str, Any]
    movement_signature: Tuple[Any, ...]


class DivisionMovementDeltaTracker:
    """Track movement deltas for successive decoded division rows.

    ``progress_bucket_size`` controls how much movement-progress drift is
    suppressed before a new delta is emitted. Non-moving rows keep movement
    source, target, progress, and bucket as ``None`` so stop frames are compact.
    """

    def __init__(
        self,
        *,
        progress_bucket_size: float = 0.01,
        default_div_type: int = DEFAULT_DIVISION_TYPE,
    ) -> None:
        bucket = _finite_nonnegative_float(progress_bucket_size)
        if bucket is None or bucket <= 0.0:
            raise ValueError("progress_bucket_size must be a positive finite number")
        div_type = _bounded_int(
            default_div_type,
            minimum=1,
            maximum=MAX_DIVISION_TYPE,
            field_name="default_div_type",
        )
        self.progress_bucket_size = bucket
        self.default_div_type = div_type
        self._tracked: Dict[str, _TrackedDivision] = {}

    def reset(self) -> None:
        """Forget all prior observations."""
        self._tracked.clear()

    def observe_rows(
        self,
        rows: Iterable[Mapping[str, Any]],
        *,
        date: str,
        hour_index: int,
        sequence: int,
        tick_ms: int,
        sample_ms: int,
        capture_boundary: str,
        collector_version: str = "division_movement_delta/1",
        **optional_envelope: Any,
    ) -> list[Dict[str, Any]]:
        """Observe a batch of decoded rows and return emitted raw frames.

        ``sequence`` is the first transport sequence to assign. It is advanced
        only for frames that are actually emitted.
        """
        frames: list[Dict[str, Any]] = []
        next_sequence = sequence
        for row in rows:
            frame = self.observe_row(
                row,
                date=date,
                hour_index=hour_index,
                sequence=next_sequence,
                tick_ms=tick_ms,
                sample_ms=sample_ms,
                capture_boundary=capture_boundary,
                collector_version=collector_version,
                **optional_envelope,
            )
            if frame is not None:
                frames.append(frame)
                next_sequence += 1
        return frames

    def observe_row(
        self,
        row: Mapping[str, Any],
        *,
        date: str,
        hour_index: int,
        sequence: int,
        tick_ms: int,
        sample_ms: int,
        capture_boundary: str,
        collector_version: str = "division_movement_delta/1",
        **optional_envelope: Any,
    ) -> Optional[Dict[str, Any]]:
        """Observe one decoded division row and return a frame only on change."""
        state = self._normalise_row(row)
        entity_key = division_entity_key(state["div_id"], state["div_type"])
        signature = _movement_signature(state)
        previous = self._tracked.get(entity_key)

        self._tracked[entity_key] = _TrackedDivision(
            state=dict(state),
            movement_signature=signature,
        )
        if previous is not None and previous.movement_signature == signature:
            return None

        before = previous.state if previous is not None else None
        changed_fields = _changed_fields(before, state)
        payload = {
            "op": _delta_op(before, changed_fields),
            "before": before,
            "after": state,
            "changed_fields": changed_fields,
        }
        return build_division_domain_delta_frame(
            entity_key=entity_key,
            payload=payload,
            date=date,
            hour_index=hour_index,
            sequence=sequence,
            tick_ms=tick_ms,
            sample_ms=sample_ms,
            capture_boundary=capture_boundary,
            collector_version=collector_version,
            **optional_envelope,
        )

    def _normalise_row(self, row: Mapping[str, Any]) -> Dict[str, Any]:
        div_id = _coerce_div_id(row.get("div_id", row.get("id")))
        div_type_value = row.get("div_type")
        if div_type_value is None:
            div_type = self.default_div_type
        else:
            div_type = _bounded_int(
                div_type_value,
                minimum=1,
                maximum=MAX_DIVISION_TYPE,
                field_name="div_type",
            )
        div_uid = division_uid(div_id, div_type)

        progress = _movement_progress(row)
        moving = _moving(row, progress)
        if moving:
            movement_progress = progress
            progress_bucket = _progress_bucket(progress, self.progress_bucket_size)
            source = _province(row.get("movement_source_province"))
            target = _province(row.get("move_target_province"))
        else:
            movement_progress = None
            progress_bucket = None
            source = None
            target = None

        state: Dict[str, Any] = {
            "div_uid": div_uid,
            "div_id": div_id,
            "div_type": div_type,
            "moving": moving,
            "movement_source_province": source,
            "move_target_province": target,
            "location": _province(row.get("location")),
            "progress_bucket": progress_bucket,
            "movement_progress": movement_progress,
        }
        owner = _owner(row.get("owner"))
        if owner is not None:
            state["owner"] = owner
        return _ordered_state(state)


def division_uid(div_id: int, div_type: int = DEFAULT_DIVISION_TYPE) -> str:
    """Return the canonical payload identity for a division."""
    return f"{div_id}:{div_type}"


def division_entity_key(div_id: int, div_type: int = DEFAULT_DIVISION_TYPE) -> str:
    """Return the stable raw-frame entity key for a division."""
    return f"division:{division_uid(div_id, div_type)}"


def build_division_domain_delta_frame(
    *,
    entity_key: str,
    payload: Mapping[str, Any],
    date: str,
    hour_index: int,
    sequence: int,
    tick_ms: int,
    sample_ms: int,
    capture_boundary: str,
    collector_version: str = "division_movement_delta/1",
    **optional_envelope: Any,
) -> Dict[str, Any]:
    """Build a raw collector frame for a division movement delta."""
    frame: Dict[str, Any] = {
        "type": "domain_delta",
        "backend": COLLECTOR_BACKEND,
        "collector_version": collector_version,
        "abi_version": COLLECTOR_ABI_VERSION,
        "sequence": sequence,
        "tick_ms": tick_ms,
        "date": date,
        "hour_index": hour_index,
        "sample_ms": sample_ms,
        "capture_boundary": capture_boundary,
        "domain": DIVISIONS_DOMAIN,
        "entity_key": entity_key,
        "source": RAW_SOURCE,
        "confidence": RAW_CONFIDENCE,
        "payload": dict(payload),
    }
    for key in OPTIONAL_FRAME_KEYS:
        value = optional_envelope.get(key)
        if value is not None:
            frame[key] = value
    return frame


def _ordered_state(state: Mapping[str, Any]) -> Dict[str, Any]:
    ordered: Dict[str, Any] = {}
    for field in EMITTED_FIELD_ORDER:
        if field in state:
            ordered[field] = state[field]
    return ordered


def _movement_signature(state: Mapping[str, Any]) -> Tuple[Any, ...]:
    return tuple(state.get(field) for field in MOVEMENT_SIGNATURE_FIELDS)


def _changed_fields(
    before: Optional[Mapping[str, Any]],
    after: Mapping[str, Any],
) -> list[str]:
    if before is None:
        return [field for field in EMITTED_FIELD_ORDER if field in after]
    changed = {
        field
        for field in set(before) | set(after)
        if before.get(field) != after.get(field)
    }
    ordered = [field for field in EMITTED_FIELD_ORDER if field in changed]
    ordered.extend(sorted(changed - set(EMITTED_FIELD_ORDER)))
    return ordered


def _delta_op(
    before: Optional[Mapping[str, Any]],
    changed_fields: Iterable[str],
) -> str:
    if before is None:
        return "upsert"
    changed = set(changed_fields)
    if changed and changed <= {"progress_bucket", "movement_progress"}:
        return "sample"
    return "upsert"


def _coerce_div_id(value: Any) -> int:
    return _bounded_int(
        value,
        minimum=0,
        maximum=MAX_DIVISION_ID,
        field_name="div_id",
    )


def _bounded_int(value: Any, *, minimum: int, maximum: int, field_name: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise ValueError(f"{field_name} must be an int")
    if value < minimum or value > maximum:
        raise ValueError(f"{field_name} {value!r} outside [{minimum}, {maximum}]")
    return value


def _province(value: Any) -> Optional[int]:
    if isinstance(value, bool) or not isinstance(value, int):
        return None
    if value <= 0 or value > MAX_PROVINCE_ID:
        return None
    return value


def _owner(value: Any) -> Optional[str]:
    if not isinstance(value, str) or not schema.valid_tag(value):
        return None
    return value


def _moving(row: Mapping[str, Any], progress: Optional[float]) -> bool:
    value = row.get("moving")
    if isinstance(value, bool):
        return value
    if isinstance(value, int) and value in (0, 1):
        return bool(value)
    return progress is not None and progress > 0.0


def _movement_progress(row: Mapping[str, Any]) -> Optional[float]:
    for key in ("movement_progress", "move_progress", "progress"):
        if key in row:
            return _finite_nonnegative_float(row.get(key))
    return None


def _finite_nonnegative_float(value: Any) -> Optional[float]:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return None
    parsed = float(value)
    if not math.isfinite(parsed) or parsed < 0.0 or parsed > MAX_PROGRESS:
        return None
    return parsed


def _progress_bucket(
    progress: Optional[float],
    progress_bucket_size: float,
) -> Optional[int]:
    if progress is None:
        return None
    return int(math.floor(progress / progress_bucket_size))


__all__ = [
    "DEFAULT_DIVISION_TYPE",
    "DIVISIONS_DOMAIN",
    "DivisionMovementDeltaTracker",
    "build_division_domain_delta_frame",
    "division_entity_key",
    "division_uid",
]
