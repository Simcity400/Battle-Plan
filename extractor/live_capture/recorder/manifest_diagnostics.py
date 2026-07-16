"""Patch recorder diagnostics into a finalized .battleplan manifest.

This stays recorder-local so the GUI can surface cadence diagnostics without
changing the shared capture loop or writer API.
"""
from __future__ import annotations

import json
import os
from typing import Any, Dict, List, Tuple


CADENCE_MODE = "ingame_hour_dedup_v1"
MAX_SKIP_RANGES = 64


def patch_manifest_diagnostics(path: str, diagnostics: Dict[str, Any]) -> bool:
    """Add ``recording_diagnostics`` to a finalized ``.battleplan`` container.

    Returns True when the container was patched. Missing/unfinalized files return
    False so the recorder can keep running and try again after the next finalize.
    """
    if not path or not os.path.isfile(path):
        return False

    from live_capture import replay_writer as rw

    rw._verify_container(path)
    with open(path, "rb") as fh:
        by_name = rw._zip_read_all(fh.read())
    manifest_bytes = by_name.get("manifest.json")
    if manifest_bytes is None:
        return False

    manifest = json.loads(manifest_bytes.decode("utf-8"))
    diagnostics = _json_safe(diagnostics)
    index = _load_payload_index(manifest, by_name)
    audit_cadence_metadata(manifest, index, diagnostics)

    manifest["recording_diagnostics"] = diagnostics
    manifest["manifest_digest"] = None
    manifest["manifest_digest"] = rw.manifest_digest(manifest)

    files = []
    for name in sorted(k for k in by_name if k != "manifest.json"):
        files.append(rw._ZipFile(name, by_name[name], deflate=True))
    man_bytes = json.dumps(manifest, separators=(",", ":"),
                           ensure_ascii=False).encode("utf-8")
    files.append(rw._ZipFile("manifest.json", man_bytes, deflate=False))
    data = rw._write_zip(files)

    # Use the same verify-before-replace publication choke point as the primary
    # writer.  If the patched candidate is invalid, the prior recording remains
    # untouched and the failure propagates to the recorder/GUI.
    rw._atomic_publish_verified_container(path, data)
    return True


def verify_recording_cadence(path: str) -> Dict[str, Any]:
    """Verify patched recorder cadence diagnostics against a ``.battleplan``.

    This is an offline proof helper. It does not claim that a live game was
    unpaused; it only proves the finalized container's manifest/index hours are
    internally consistent with the selected cadence and reported skipped-hour
    diagnostics. Raises ValueError on any mismatch.
    """
    if not path or not os.path.isfile(path):
        raise ValueError("recording does not exist: %r" % (path,))

    from live_capture import replay_writer as rw

    rw._verify_container(path)
    with open(path, "rb") as fh:
        by_name = rw._zip_read_all(fh.read())
    manifest_bytes = by_name.get("manifest.json")
    if manifest_bytes is None:
        raise ValueError("recording has no finalized manifest.json")
    manifest = json.loads(manifest_bytes.decode("utf-8"))
    diagnostics = manifest.get("recording_diagnostics")
    if not isinstance(diagnostics, dict):
        raise ValueError("recording has no recording_diagnostics object")
    index = _load_payload_index(manifest, by_name)
    return audit_cadence_metadata(manifest, index, diagnostics)


def audit_cadence_metadata(manifest: Dict[str, Any],
                           index: Dict[str, Any],
                           diagnostics: Dict[str, Any]) -> Dict[str, Any]:
    """Audit recorder cadence diagnostics against manifest/index snapshot hours.

    The durable store index is the hour spine. Manifest entries must name the
    same snapshot hours, and diagnostics must report exactly the skipped
    scheduled cadence hours implied by the capture-order index entries.
    """
    if not isinstance(manifest, dict):
        raise ValueError("manifest must be an object")
    if not isinstance(index, dict):
        raise ValueError("index must be an object")
    if not isinstance(diagnostics, dict):
        raise ValueError("recording diagnostics must be an object")

    cadence = diagnostics.get("cadence")
    if not isinstance(cadence, dict):
        raise ValueError("recording diagnostics missing cadence object")
    if cadence.get("mode") != CADENCE_MODE:
        raise ValueError("unsupported cadence diagnostics mode %r"
                         % (cadence.get("mode"),))

    step = _require_int(cadence, "target_step_hours", minimum=1)
    reported_captures = _require_int(cadence, "captures", minimum=0)
    reported_skipped = _require_int(cadence, "skipped_hours", minimum=0)
    reported_skip_events = _require_int(cadence, "skip_events", minimum=0)

    manual_once = cadence.get("manual_once", False)
    if not isinstance(manual_once, bool):
        raise ValueError("cadence.manual_once must be a boolean")

    index_entries = index.get("snapshots")
    if not isinstance(index_entries, list):
        raise ValueError("payload index missing snapshots array")
    index_by_path: Dict[str, int] = {}
    game_id = manifest.get("game_id")
    for entry in index_entries:
        if not isinstance(entry, dict):
            raise ValueError("payload index contains a non-object snapshot")
        file_name = entry.get("file")
        if not isinstance(file_name, str) or not file_name:
            raise ValueError("payload index snapshot has missing file %r"
                             % (file_name,))
        hour = entry.get("hour_index")
        if not isinstance(hour, int) or isinstance(hour, bool):
            raise ValueError("payload index snapshot has non-integer hour_index "
                             "%r" % (hour,))
        path = "payload/%s/%s" % (game_id, file_name)
        if path in index_by_path:
            raise ValueError("payload index contains duplicate snapshot path %r"
                             % (path,))
        index_by_path[path] = hour

    manifest_by_path = _manifest_snapshot_hours_by_path(manifest)
    if manifest_by_path != index_by_path:
        raise ValueError("manifest snapshot entries %r do not match index "
                         "snapshot entries %r"
                         % (manifest_by_path, index_by_path))

    expected_count = len(index_by_path)
    counts = manifest.get("counts") or {}
    if counts.get("snapshots") != expected_count:
        raise ValueError("manifest counts.snapshots %r does not match index "
                         "snapshot count %r"
                         % (counts.get("snapshots"), expected_count))
    if reported_captures != expected_count:
        raise ValueError("cadence.captures %r does not match indexed snapshot "
                         "count %r" % (reported_captures, expected_count))
    if manual_once and expected_count != 1:
        raise ValueError("manual/paused cadence must finalize exactly one "
                         "canonical snapshot, got %r" % (expected_count,))

    ordered_hours = _capture_ordered_hours(index_entries)
    expected_skipped, expected_ranges = _expected_skips(ordered_hours, step)
    if reported_skipped != expected_skipped:
        raise ValueError("cadence.skipped_hours %r does not match expected %r "
                         "from index hours %r at step %r"
                         % (reported_skipped, expected_skipped,
                            ordered_hours, step))
    if reported_skip_events != len(expected_ranges):
        raise ValueError("cadence.skip_events %r does not match expected %r"
                         % (reported_skip_events, len(expected_ranges)))

    reported_ranges = cadence.get("skip_ranges")
    if not isinstance(reported_ranges, list):
        raise ValueError("cadence.skip_ranges must be an array")
    if len(expected_ranges) <= MAX_SKIP_RANGES:
        expected_reported_ranges = expected_ranges
    else:
        expected_reported_ranges = expected_ranges[-MAX_SKIP_RANGES:]
    if reported_ranges != expected_reported_ranges:
        raise ValueError("cadence.skip_ranges %r does not match expected %r"
                         % (reported_ranges, expected_reported_ranges))

    last_captured = cadence.get("last_captured_hour")
    expected_last = ordered_hours[-1] if ordered_hours else None
    if last_captured != expected_last:
        raise ValueError("cadence.last_captured_hour %r does not match last "
                         "capture-order index hour %r"
                         % (last_captured, expected_last))

    return {
        "status": "ok",
        "snapshot_count": expected_count,
        "target_step_hours": step,
        "manual_once": manual_once,
        "capture_order_hours": ordered_hours,
        "expected_skipped_hours": expected_skipped,
        "expected_skip_events": len(expected_ranges),
        "expected_skip_ranges": expected_ranges,
    }


def _load_payload_index(manifest: Dict[str, Any],
                        by_name: Dict[str, bytes]) -> Dict[str, Any]:
    game_id = manifest.get("game_id")
    if not isinstance(game_id, str) or not game_id:
        raise ValueError("manifest missing game_id")
    ipath = "payload/%s/index.json" % game_id
    index_bytes = by_name.get(ipath)
    if index_bytes is None:
        raise ValueError("payload index missing at %s" % ipath)
    index = json.loads(index_bytes.decode("utf-8"))
    if index.get("game_id") != game_id:
        raise ValueError("payload index game_id %r does not match manifest %r"
                         % (index.get("game_id"), game_id))
    return index


def _manifest_snapshot_hours_by_path(manifest: Dict[str, Any]) -> Dict[str, int]:
    entries = manifest.get("entries")
    if not isinstance(entries, list):
        raise ValueError("manifest entries must be an array")
    by_path: Dict[str, int] = {}
    for entry in entries:
        if not isinstance(entry, dict) or entry.get("role") != "snapshot":
            continue
        path = entry.get("path")
        if not isinstance(path, str) or not path:
            raise ValueError("manifest snapshot entry has missing path %r"
                             % (path,))
        hour = entry.get("hour_index")
        if not isinstance(hour, int) or isinstance(hour, bool):
            raise ValueError("manifest snapshot entry has non-integer "
                             "hour_index %r" % (hour,))
        if path in by_path:
            raise ValueError("manifest contains duplicate snapshot path %r"
                             % (path,))
        by_path[path] = hour
    return by_path


def _capture_ordered_hours(index_entries: List[Dict[str, Any]]) -> List[int]:
    def key(entry: Dict[str, Any]) -> Tuple[int, int, int, str]:
        seq = entry.get("capture_seq")
        seg = entry.get("segment_id")
        hour = entry.get("hour_index")
        if not isinstance(seq, int) or isinstance(seq, bool):
            raise ValueError("payload index snapshot missing integer "
                             "capture_seq; cannot audit capture-order cadence")
        if not isinstance(seg, int) or isinstance(seg, bool):
            seg = 0
        return (seq, seg, hour, str(entry.get("file") or ""))

    ordered = sorted(index_entries, key=key)
    return [entry["hour_index"] for entry in ordered]


def _expected_skips(hours: List[int], step: int) -> Tuple[int, List[Dict[str, int]]]:
    total = 0
    ranges: List[Dict[str, int]] = []
    for prev, cur in zip(hours, hours[1:]):
        gap = cur - prev
        if gap <= 0:
            continue
        skipped = max(0, (gap - 1) // step)
        if not skipped:
            continue
        start = prev + step
        end = prev + (step * skipped)
        entry = {
            "start_hour": int(start),
            "end_hour": int(end),
            "count": int(skipped),
        }
        if step != 1:
            entry["step_hours"] = int(step)
        ranges.append(entry)
        total += skipped
    return total, ranges


def _require_int(obj: Dict[str, Any], key: str, *, minimum: int) -> int:
    value = obj.get(key)
    if not isinstance(value, int) or isinstance(value, bool) or value < minimum:
        raise ValueError("cadence.%s must be an integer >= %d, got %r"
                         % (key, minimum, value))
    return value


def _json_safe(obj: Any) -> Any:
    return json.loads(json.dumps(obj, separators=(",", ":"),
                                 ensure_ascii=False, allow_nan=False))


__all__ = [
    "audit_cadence_metadata",
    "patch_manifest_diagnostics",
    "verify_recording_cadence",
]
