#!/usr/bin/env python3
"""Regrade an X12 early-launch replay without trusting extracted sidecars.

This closes the machine-checkable half of the live gate.  It deliberately does
not elevate the production latch: visible action evidence still needs a human
semantic join to prove that the captured callbacks correspond to the requested
UI actions.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import struct
import sys
import tempfile
from pathlib import Path

import extract_f6_replay_snapshots as snapshot_extractor

SCHEMA = "battle-plan.x12_speed_pause_activation.v1"
EXACT_EXE_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
BASELINE_SOURCE = "hourly_hook_ready_baseline"
TRANSITION_SOURCES = {"speed_setter", "pause_toggle", "pause_forced"}


class ProofError(RuntimeError):
    pass


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def require_int(value, context: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise ProofError(f"{context} is not an integer")
    return value


def decode_tick(bits: int, context: str) -> float:
    if bits < 0 or bits > 0xFFFF_FFFF or bits == 0x8000_0000:
        raise ProofError(f"{context} has invalid f32 bits")
    value = struct.unpack("<f", struct.pack("<I", bits))[0]
    if not math.isfinite(value) or value < 0.0 or value > 1.0:
        raise ProofError(f"{context} is outside the authenticated 0..=1 range")
    return value


def validate_history(value, context: str) -> list[dict]:
    if not isinstance(value, list) or not value:
        raise ProofError(f"{context} is not a nonempty array")
    first = value[0]
    if not isinstance(first, dict):
        raise ProofError(f"{context}[0] is not an object")
    if set(first) != {"hour_index", "speed", "paused", "transition_sequence", "source"}:
        raise ProofError(f"{context}[0] is not the exact X12 baseline shape")
    baseline_hour = require_int(first["hour_index"], f"{context}[0].hour_index")
    baseline_speed = require_int(first["speed"], f"{context}[0].speed")
    if baseline_hour < 0 or baseline_speed not in range(5):
        raise ProofError(f"{context}[0] baseline is outside exact ranges")
    if not isinstance(first["paused"], bool):
        raise ProofError(f"{context}[0].paused is not boolean")
    if first["transition_sequence"] != 0 or first["source"] != BASELINE_SOURCE:
        raise ProofError(f"{context}[0] is not the authenticated pre-action baseline")

    previous_hour = baseline_hour
    previous_speed = baseline_speed
    previous_paused = first["paused"]
    previous_tick = None
    normalized = [first]
    for ordinal, row in enumerate(value[1:], 1):
        row_context = f"{context}[{ordinal}]"
        if not isinstance(row, dict):
            raise ProofError(f"{row_context} is not an object")
        if set(row) != {
            "hour_index",
            "speed",
            "paused",
            "transition_sequence",
            "tick_progress_f32_bits",
            "source",
        }:
            raise ProofError(f"{row_context} is not the exact transition shape")
        hour = require_int(row["hour_index"], f"{row_context}.hour_index")
        speed = require_int(row["speed"], f"{row_context}.speed")
        sequence = require_int(row["transition_sequence"], f"{row_context}.transition_sequence")
        bits = require_int(row["tick_progress_f32_bits"], f"{row_context}.tick_progress_f32_bits")
        paused = row["paused"]
        source = row["source"]
        if hour < 0 or hour < previous_hour or speed not in range(5):
            raise ProofError(f"{row_context} regresses hour or has invalid speed")
        if not isinstance(paused, bool) or source not in TRANSITION_SOURCES:
            raise ProofError(f"{row_context} has invalid pause/source typing")
        if sequence != ordinal:
            raise ProofError(f"{row_context} sequence is not consecutive from one")
        tick = decode_tick(bits, f"{row_context}.tick_progress_f32_bits")
        if hour == previous_hour and previous_tick is not None and tick < previous_tick:
            raise ProofError(f"{row_context} regresses same-hour tick progress")
        if source == "speed_setter":
            delta_ok = speed != previous_speed and paused == previous_paused
        else:
            delta_ok = speed == previous_speed and paused != previous_paused
        if not delta_ok:
            raise ProofError(f"{row_context} source does not match its exact state delta")
        previous_hour = hour
        previous_speed = speed
        previous_paused = paused
        previous_tick = tick
        normalized.append(row)
    return normalized


def required_action_pattern(history: list[dict]) -> dict:
    transitions = history[1:]
    speed_rows = [row for row in transitions if row["source"] == "speed_setter"]
    speed_pair = None
    for left_index, left in enumerate(speed_rows):
        for right in speed_rows[left_index + 1 :]:
            if left["hour_index"] == right["hour_index"] and left["speed"] != right["speed"]:
                speed_pair = [left["transition_sequence"], right["transition_sequence"]]
                break
        if speed_pair is not None:
            break
    if speed_pair is None:
        raise ProofError("history has no two distinct speed mutations in one engine hour")

    bracket = None
    for paused in transitions:
        if paused["source"] not in {"pause_toggle", "pause_forced"} or not paused["paused"]:
            continue
        for speed in transitions:
            if (
                speed["source"] == "speed_setter"
                and speed["paused"]
                and speed["hour_index"] == paused["hour_index"]
                and speed["transition_sequence"] > paused["transition_sequence"]
            ):
                for resumed in transitions:
                    if (
                        resumed["source"] in {"pause_toggle", "pause_forced"}
                        and not resumed["paused"]
                        and resumed["hour_index"] == paused["hour_index"]
                        and resumed["transition_sequence"] > speed["transition_sequence"]
                    ):
                        bracket = [
                            paused["transition_sequence"],
                            speed["transition_sequence"],
                            resumed["transition_sequence"],
                        ]
                        break
                if bracket is not None:
                    break
        if bracket is not None:
            break
    if bracket is None:
        raise ProofError("history has no same-hour pause -> speed mutation -> resume bracket")
    return {"same_hour_speed_pair_sequences": speed_pair, "pause_speed_resume_sequences": bracket}


def verify(
    run_dir: Path,
    hoi4_exe: Path,
    collector_dll: Path,
    visible_action_evidence: Path | None,
) -> dict:
    run_dir = run_dir.resolve()
    manifest_path = run_dir / "native_session_manifest.json"
    if not manifest_path.is_file():
        raise ProofError("native_session_manifest.json is absent")
    manifest = json.loads(manifest_path.read_text(encoding="utf-8-sig"))
    replay_name = manifest.get("replay_file")
    if not isinstance(replay_name, str) or Path(replay_name).name != replay_name:
        raise ProofError("manifest replay_file is not one canonical local filename")
    replay = (run_dir / replay_name).resolve()
    if replay.parent != run_dir or not replay.is_file():
        raise ProofError("manifest replay_file escapes or is absent")
    if sha256_file(hoi4_exe.resolve()) != EXACT_EXE_SHA256:
        raise ProofError("hoi4.exe does not match exact a729 authority")
    collector_sha256 = sha256_file(collector_dll.resolve())
    launch = manifest.get("early_render_launch_manifest")
    receipt = manifest.get("early_render_capture_receipt")
    if not isinstance(launch, dict) or not isinstance(receipt, dict):
        raise ProofError("manifest has no authenticated suspended early-launch identity")
    if (
        launch.get("executable_sha256") != EXACT_EXE_SHA256
        or receipt.get("executable_sha256") != EXACT_EXE_SHA256
        or launch.get("collector_sha256") != collector_sha256
        or receipt.get("collector_sha256") != collector_sha256
    ):
        raise ProofError("early-launch executable/collector identity is not exact")
    process_id = require_int(receipt.get("process_id"), "early receipt process_id")
    process_nonce = require_int(
        receipt.get("process_start_nonce"), "early receipt process_start_nonce"
    )
    if process_id <= 0 or process_id != manifest.get("pid") or process_nonce <= 0:
        raise ProofError("early receipt process identity disagrees with the capture manifest")
    if receipt.get("created_suspended") is not True:
        raise ProofError("X12 evidence activation was not inherited through suspended early launch")
    completeness = receipt.get("completeness")
    if not isinstance(completeness, dict) or len(completeness) != 10 or not all(
        value is True for value in completeness.values()
    ):
        raise ProofError("early hook completeness receipt is not exact")
    if manifest.get("stream_format") != "compact":
        raise ProofError("manifest is not a compact capture")
    if manifest.get("parse_errors") != [] or manifest.get("ordering_errors") != []:
        raise ProofError("manifest reports parse or ordering errors")
    scheduled = manifest.get("scheduled_domains")
    if scheduled != ["session_multiplayer_metadata_live"]:
        raise ProofError("manifest does not bind the exact D57-only schedule")

    with tempfile.TemporaryDirectory(prefix="x12-regrade-") as temporary:
        extraction = snapshot_extractor.extract(replay, Path(temporary))
        prior_history = None
        final_history = None
        history_snapshot_count = 0
        for row in extraction["snapshots"]:
            snapshot = json.loads(Path(row["extracted_path"]).read_text(encoding="utf-8-sig"))
            session = snapshot.get("session")
            speed_events = session.get("speed_events") if isinstance(session, dict) else None
            if speed_events is None:
                if prior_history is not None:
                    raise ProofError("X12 history disappeared after its authenticated baseline")
                continue
            history = validate_history(speed_events, f"snapshot[{row['hour_index']}].session.speed_events")
            if prior_history is not None and history[: len(prior_history)] != prior_history:
                raise ProofError("X12 cumulative history is not prefix-monotonic across snapshots")
            prior_history = history
            final_history = history
            history_snapshot_count += 1
        if final_history is None:
            raise ProofError("no replay snapshot contains an authenticated X12 history")
        pattern = required_action_pattern(final_history)
        extraction_receipt = {
            key: value for key, value in extraction.items() if key != "snapshots"
        }
        extraction_receipt["snapshots"] = [
            {key: value for key, value in row.items() if key != "extracted_path"}
            for row in extraction["snapshots"]
        ]

    history_bytes = json.dumps(final_history, sort_keys=True, separators=(",", ":")).encode()
    visible = None
    if visible_action_evidence is not None:
        visible_action_evidence = visible_action_evidence.resolve()
        if not visible_action_evidence.is_file():
            raise ProofError("visible action evidence path is absent")
        visible = {
            "path": str(visible_action_evidence),
            "sha256": sha256_file(visible_action_evidence),
            "bytes": visible_action_evidence.stat().st_size,
        }
    return {
        "schema": SCHEMA,
        "ok": True,
        "machine_gate_passed": True,
        "production_activation_recommended": False,
        "human_semantic_review_required": True,
        "run_dir": str(run_dir),
        "manifest_path": str(manifest_path),
        "manifest_sha256": sha256_file(manifest_path),
        "replay_path": str(replay),
        "replay_sha256": sha256_file(replay),
        "hoi4_exe_path": str(hoi4_exe.resolve()),
        "hoi4_exe_sha256": EXACT_EXE_SHA256,
        "collector_dll_path": str(collector_dll.resolve()),
        "collector_dll_sha256": collector_sha256,
        "process_id": process_id,
        "process_start_nonce": process_nonce,
        "snapshot_extraction": extraction_receipt,
        "history_snapshot_count": history_snapshot_count,
        "final_history_rows": len(final_history),
        "final_history_sha256": hashlib.sha256(history_bytes).hexdigest(),
        "required_pattern": pattern,
        "visible_action_evidence": visible,
        "problems": [],
    }


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run-dir", required=True, type=Path)
    parser.add_argument("--hoi4-exe", required=True, type=Path)
    parser.add_argument("--collector-dll", required=True, type=Path)
    parser.add_argument("--visible-action-evidence", type=Path)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args(argv)
    try:
        report = verify(
            args.run_dir,
            args.hoi4_exe,
            args.collector_dll,
            args.visible_action_evidence,
        )
    except (OSError, UnicodeError, json.JSONDecodeError, ProofError, snapshot_extractor.ExtractError) as exc:
        report = {
            "schema": SCHEMA,
            "ok": False,
            "machine_gate_passed": False,
            "production_activation_recommended": False,
            "human_semantic_review_required": True,
            "problems": [str(exc)],
        }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0 if report["ok"] else 2


if __name__ == "__main__":
    sys.exit(main())
