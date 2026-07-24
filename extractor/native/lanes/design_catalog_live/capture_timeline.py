#!/usr/bin/env python3
"""Acquire authenticated repeated A27 full censuses for one recorder session.

Each admitted observation is produced by ``live_a729_acceptance.capture`` and
therefore carries its own whole-sweep coherence seal. The timeline never
invents a mutation: gaps, torn sweeps, and rejected observations are retained
for the pack-time coverage gate.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
import time
from pathlib import Path

LANE = Path(__file__).resolve().parent
REPO = LANE.parents[3]
sys.path.insert(0, str(LANE))
sys.path.insert(0, str(REPO / "extractor" / "memory_reader"))

import winmem  # noqa: E402
import live_a729_acceptance as a27  # noqa: E402

SCHEMA = "extractor.a27.live_heap_catalog_timeline.a729.v1"
ACTION_SCHEMA = "extractor.a27.authenticated_transaction_request.v1"
RELEASE_SCHEMA = "extractor.a27.authenticated_transaction_release.v1"


def _canonical_bytes(value: object) -> bytes:
    return json.dumps(
        value, ensure_ascii=False, sort_keys=True, separators=(",", ":")
    ).encode("utf-8")


def _atomic_json(path: Path, value: object) -> None:
    encoded = json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2).encode("utf-8")
    temporary = path.with_suffix(path.suffix + ".tmp")
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary.write_bytes(encoded)
    os.replace(temporary, path)


def _read_json(path: Path) -> dict:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise RuntimeError(f"{path.name} is not a JSON object")
    return value


def _timeline(pid: int) -> dict:
    return {
        "schema": SCHEMA,
        "authority": {
            "program": "hoi4.exe 1.19.2.0.a729",
            "program_sha256": a27.PROGRAM_SHA256,
            "pid": pid,
            "producer": "repeated_external_whole_sweep",
        },
        "captures": [],
        "rejected_observations": [],
        "same_hour_recheck_count": 0,
        "transport": "first_full_then_native_row_deltas",
        "complete_sequence": False,
        "action_windows": [],
    }


def _stable_id(row: dict) -> str:
    return f"{row['id']}:{row['type']}"


def _semantic_row(row: dict) -> dict:
    return {
        key: value
        for key, value in pointer_free(row).items()
        if key != "catalog_scan_ordinal"
    }


def pointer_free(value):
    if isinstance(value, list):
        return [pointer_free(item) for item in value]
    if isinstance(value, dict):
        return {
            key: pointer_free(item)
            for key, item in value.items()
            if key != "pointer_identity"
            and not key.endswith("_pointer")
            and key not in {"pid", "module_base", "game_state"}
        }
    return value


def _delta(previous: dict, current: dict) -> dict:
    out = {
        "frame_metadata": {
            key: value
            for key, value in current.items()
            if key not in {"variants", "division_templates"}
        },
        "variant_upserts": [],
        "variant_tombstones": [],
        "template_upserts": [],
        "template_tombstones": [],
        "ordered_design_ids": [
            _stable_id(row)
            for row in current["division_templates"] + current["variants"]
        ],
    }
    for key, upserts, tombstones in (
        ("variants", "variant_upserts", "variant_tombstones"),
        ("division_templates", "template_upserts", "template_tombstones"),
    ):
        before = {_stable_id(row): row for row in previous[key]}
        after = {_stable_id(row): row for row in current[key]}
        out[tombstones] = sorted(set(before) - set(after))
        out[upserts] = [
            row
            for stable_id, row in after.items()
            if stable_id not in before
            or _semantic_row(before[stable_id]) != _semantic_row(row)
        ]
    return out


def _service_action_window(
    process,
    timeline: dict,
    output: Path,
    stop: Path,
    action_directory: Path,
    action_sequence: int,
    poll_ms: int,
) -> bool:
    """Suspend censuses while one authenticated UI transaction crosses an hour.

    A mutation after an admitted same-hour recheck would otherwise correctly
    trip the producer's same-hour-mutation fatal.  The create-new request/ready/
    release/resumed quartet proves that no census ran between the last sealed
    baseline and the first census at the next hour.
    """
    request_path = action_directory / f"request_{action_sequence:02}.json"
    if not request_path.exists():
        return False
    request = _read_json(request_path)
    if (
        request.get("schema") != ACTION_SCHEMA
        or request.get("sequence") != action_sequence
        or request.get("from_hour") != timeline["captures"][-1]["capture_hour_index"]
        or request.get("baseline_snapshot_sha256")
        != timeline["captures"][-1]["snapshot_sha256"]
        or request.get("transaction")
        not in {"rename", "edit", "obsolete", "reactivate"}
        or not isinstance(request.get("profile_sha256"), str)
        or len(request["profile_sha256"]) != 64
    ):
        raise RuntimeError(f"action request {action_sequence} identity is invalid")
    if timeline["captures"][-1]["same_hour_rechecks"] < 1:
        raise RuntimeError("action window requested before a sealed same-hour recheck")

    ready_path = action_directory / f"ready_{action_sequence:02}.json"
    ready_receipt = {
        "schema": "extractor.a27.authenticated_transaction_ready.v1",
        "sequence": action_sequence,
        "transaction": request["transaction"],
        "from_hour": request["from_hour"],
        "baseline_snapshot_sha256": request["baseline_snapshot_sha256"],
        "request_sha256": hashlib.sha256(request_path.read_bytes()).hexdigest(),
        "profile_sha256": request["profile_sha256"],
        "census_suspended": True,
    }
    _atomic_json(ready_path, ready_receipt)

    release_path = action_directory / f"release_{action_sequence:02}.json"
    while not release_path.exists():
        if stop.exists():
            raise RuntimeError("capture stopped inside an authenticated action window")
        time.sleep(poll_ms / 1000.0)
    release = _read_json(release_path)
    expected_to_hour = request["from_hour"] + 1
    game_state = process.u64(process.module_base + a27.CGAMESTATE_RVA)
    observed_hour = process.u32(game_state + a27.GS_CURRENT_DATE) - a27.DATE_EPOCH_CONST
    if (
        release.get("schema") != RELEASE_SCHEMA
        or release.get("sequence") != action_sequence
        or release.get("transaction") != request["transaction"]
        or release.get("from_hour") != request["from_hour"]
        or release.get("to_hour") != expected_to_hour
        or release.get("profile_sha256") != request["profile_sha256"]
        or release.get("ready_sha256")
        != hashlib.sha256(ready_path.read_bytes()).hexdigest()
        or observed_hour != expected_to_hour
    ):
        raise RuntimeError(f"action release {action_sequence} identity/hour is invalid")

    window = {
        "sequence": action_sequence,
        "transaction": request["transaction"],
        "from_hour": request["from_hour"],
        "to_hour": expected_to_hour,
        "baseline_snapshot_sha256": request["baseline_snapshot_sha256"],
        "profile_sha256": request["profile_sha256"],
        "request_sha256": ready_receipt["request_sha256"],
        "ready_sha256": release["ready_sha256"],
        "release_sha256": hashlib.sha256(release_path.read_bytes()).hexdigest(),
        "censuses_suspended_between_hours": True,
    }
    timeline["action_windows"].append(window)
    _atomic_json(output, timeline)
    _atomic_json(
        action_directory / f"resumed_{action_sequence:02}.json",
        {
            "schema": "extractor.a27.authenticated_transaction_resumed.v1",
            **window,
        },
    )
    return True


def acquire(
    pid: int,
    output: Path,
    ready: Path,
    stop: Path,
    poll_ms: int,
    action_directory: Path | None,
) -> int:
    timeline = _timeline(pid)
    last_hour = None
    previous_snapshot = None
    next_action_sequence = 0
    process = None
    while process is None:
        if stop.exists():
            _atomic_json(output, timeline)
            return 2
        try:
            process = winmem.Process(pid, "hoi4.exe")
        except (OSError, RuntimeError, ValueError) as error:
            # Early-launch production capture starts this helper while the game
            # is still suspended. Windows can return ERROR_PARTIAL_COPY until
            # the primary thread has entered the user-mode loader, so module
            # discovery is a retryable observation just like an uninitialized
            # CGameState root. Do not let that expected pre-resume phase kill
            # the authenticated producer.
            rejection = {
                "sequence_before_retry": len(timeline["captures"]),
                "error": str(error),
            }
            if (
                not timeline["rejected_observations"]
                or timeline["rejected_observations"][-1] != rejection
            ):
                timeline["rejected_observations"].append(rejection)
                _atomic_json(output, timeline)
            time.sleep(poll_ms / 1000.0)

    with process:
        while True:
            if stop.exists():
                break
            try:
                if (
                    action_directory is not None
                    and timeline["captures"]
                    and _service_action_window(
                        process,
                        timeline,
                        output,
                        stop,
                        action_directory,
                        next_action_sequence,
                        poll_ms,
                    )
                ):
                    next_action_sequence += 1
                game_state = process.u64(process.module_base + a27.CGAMESTATE_RVA)
                if not a27._heap(game_state):
                    raise RuntimeError("CGameState root is not initialized")
                observed_date = process.u32(game_state + a27.GS_CURRENT_DATE)
                observed_hour = observed_date - a27.DATE_EPOCH_CONST
                if observed_hour < 0:
                    raise RuntimeError("CGameState date is outside the portable replay epoch")
                snapshot = a27.capture(process)
                capture_hour = snapshot["authority"]["capture_hour_index"]
                if capture_hour == last_hour:
                    timeline["same_hour_recheck_count"] += 1
                    timeline["captures"][-1]["same_hour_rechecks"] += 1
                    recheck_sha256 = hashlib.sha256(_canonical_bytes(snapshot)).hexdigest()
                    timeline["captures"][-1]["same_hour_recheck_sha256"].append(
                        recheck_sha256
                    )
                    if recheck_sha256 != timeline["captures"][-1]["snapshot_sha256"]:
                        timeline["fatal"] = {
                            "kind": "same_hour_catalog_mutation",
                            "capture_hour_index": capture_hour,
                            "sequence_after": len(timeline["captures"]) - 1,
                        }
                        _atomic_json(output, timeline)
                        return 3
                    continue
                if last_hour is not None and capture_hour <= last_hour:
                    raise RuntimeError("accepted A27 capture hour is not strictly increasing")
                snapshot_sha256 = hashlib.sha256(_canonical_bytes(snapshot)).hexdigest()
                entry = {
                    "sequence": len(timeline["captures"]),
                    "capture_hour_index": capture_hour,
                    "snapshot_sha256": snapshot_sha256,
                    "same_hour_rechecks": 0,
                    "same_hour_recheck_sha256": [],
                }
                if previous_snapshot is None:
                    entry["snapshot"] = snapshot
                else:
                    entry["delta"] = _delta(previous_snapshot, snapshot)
                timeline["captures"].append(entry)
                previous_snapshot = snapshot
                last_hour = capture_hour
                _atomic_json(output, timeline)
                # This is the recorder's baseline barrier, not merely a
                # CGameState-root probe.  Exposing readiness before the first
                # coherent whole sweep allowed the compact stream to begin
                # while A27 still had no census for its first hour.
                if len(timeline["captures"]) == 1:
                    ready.parent.mkdir(parents=True, exist_ok=True)
                    ready.touch(exist_ok=True)
            except (OSError, RuntimeError, ValueError) as error:
                rejection = {
                    "sequence_before_retry": len(timeline["captures"]),
                    "error": str(error),
                }
                if (
                    not timeline["rejected_observations"]
                    or timeline["rejected_observations"][-1] != rejection
                ):
                    timeline["rejected_observations"].append(rejection)
                    _atomic_json(output, timeline)
                time.sleep(poll_ms / 1000.0)

    if not timeline["captures"]:
        _atomic_json(output, timeline)
        return 2
    timeline["complete_sequence"] = True
    timeline["capture_count"] = len(timeline["captures"])
    timeline["first_capture_hour_index"] = timeline["captures"][0]["capture_hour_index"]
    timeline["last_capture_hour_index"] = timeline["captures"][-1]["capture_hour_index"]
    timeline["action_window_count"] = len(timeline["action_windows"])
    _atomic_json(output, timeline)
    return 0


def main(argv=None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pid", required=True, type=int)
    parser.add_argument("--timeline-out", required=True, type=Path)
    parser.add_argument("--ready-file", required=True, type=Path)
    parser.add_argument("--stop-file", required=True, type=Path)
    parser.add_argument("--poll-ms", type=int, default=20)
    parser.add_argument("--action-directory", type=Path)
    args = parser.parse_args(argv)
    if not 5 <= args.poll_ms <= 1000:
        parser.error("--poll-ms must be in 5..1000")
    return acquire(
        args.pid,
        args.timeline_out,
        args.ready_file,
        args.stop_file,
        args.poll_ms,
        args.action_directory,
    )


if __name__ == "__main__":
    raise SystemExit(main())
