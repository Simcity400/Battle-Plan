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


def acquire(pid: int, output: Path, ready: Path, stop: Path, poll_ms: int) -> int:
    timeline = _timeline(pid)
    last_hour = None
    previous_snapshot = None
    with winmem.Process(pid, "hoi4.exe") as process:
        while True:
            if stop.exists():
                break
            try:
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
    _atomic_json(output, timeline)
    return 0


def main(argv=None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pid", required=True, type=int)
    parser.add_argument("--timeline-out", required=True, type=Path)
    parser.add_argument("--ready-file", required=True, type=Path)
    parser.add_argument("--stop-file", required=True, type=Path)
    parser.add_argument("--poll-ms", type=int, default=20)
    args = parser.parse_args(argv)
    if not 5 <= args.poll_ms <= 1000:
        parser.error("--poll-ms must be in 5..1000")
    return acquire(
        args.pid,
        args.timeline_out,
        args.ready_file,
        args.stop_file,
        args.poll_ms,
    )


if __name__ == "__main__":
    raise SystemExit(main())
