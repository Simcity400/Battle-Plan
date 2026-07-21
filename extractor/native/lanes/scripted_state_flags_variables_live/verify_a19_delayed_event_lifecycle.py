#!/usr/bin/env python3
"""Independently regrade folded A19 DEL2 lifecycle observations."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

KEY = "_r2_delayed_event_lifecycle_live"
SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
REASONS = {
    "retained_decrement",
    "executed",
    "failed_trigger_consumed",
    "definition_cancelled",
}


def load_frames(path: Path) -> list[dict[str, Any]]:
    if path.is_dir():
        snapshot_paths = sorted(path.glob("*.snapshot.json"))
        if not snapshot_paths:
            raise ValueError("capture directory has no folded *.snapshot.json files")
        frames = [
            json.loads(snapshot.read_text(encoding="utf-8"))
            for snapshot in snapshot_paths
        ]
    else:
        text = path.read_text(encoding="utf-8")
        try:
            value = json.loads(text)
        except json.JSONDecodeError:
            frames = [json.loads(line) for line in text.splitlines() if line.strip()]
        else:
            frames = value if isinstance(value, list) else [value]
    if not all(isinstance(frame, dict) for frame in frames):
        raise ValueError("capture must contain JSON objects")
    return frames


def grade(frames: list[dict[str, Any]]) -> dict[str, Any]:
    errors: list[str] = []
    observations: list[dict[str, Any]] = []
    epochs: set[int] = set()
    expected_sequence = 1
    for frame_index, frame in enumerate(frames):
        if KEY not in frame:
            continue
        value = frame[KEY]
        if value is None:
            errors.append(f"frame {frame_index}: lifecycle authority is unavailable")
            continue
        if not isinstance(value, dict):
            errors.append(f"frame {frame_index}: lifecycle authority is not an object")
            continue
        expected_keys = {
            "schema_version",
            "authority",
            "executable_sha256",
            "hourly_update_rva",
            "definition_cancel_rva",
            "shared_dispatch_rva",
            "hook_epoch",
            "first_sequence",
            "last_sequence",
            "complete",
            "events",
        }
        if set(value) != expected_keys:
            errors.append(f"frame {frame_index}: exact authority field set rejected")
            continue
        if (
            isinstance(value["schema_version"], bool)
            or value["schema_version"] != 1
            or value["authority"] != "exact_a729_delayed_event_lifecycle"
            or value["executable_sha256"] != SHA256
            or value["hourly_update_rva"] != "0x006efe10"
            or value["definition_cancel_rva"] != "0x006dd7f0"
            or value["shared_dispatch_rva"] != "0x00a028a0"
            or value["complete"] is not True
        ):
            errors.append(f"frame {frame_index}: authority identity/completeness rejected")
            continue
        if (
            not isinstance(value["hook_epoch"], str)
            or not isinstance(value["first_sequence"], str)
            or not isinstance(value["last_sequence"], str)
        ):
            errors.append(f"frame {frame_index}: non-canonical sequence header")
            continue
        try:
            epoch = int(value["hook_epoch"])
            first = int(value["first_sequence"])
            last = int(value["last_sequence"])
        except ValueError:
            errors.append(f"frame {frame_index}: non-canonical sequence header")
            continue
        if (
            value["hook_epoch"] != str(epoch)
            or value["first_sequence"] != str(first)
            or value["last_sequence"] != str(last)
            or not 0 < epoch <= 0xFFFF_FFFF_FFFF_FFFF
            or not 0 <= first <= 0xFFFF_FFFF_FFFF_FFFF
            or not 0 <= last <= 0xFFFF_FFFF_FFFF_FFFF
        ):
            errors.append(f"frame {frame_index}: non-canonical sequence header")
            continue
        events = value["events"]
        if epoch <= 0 or not isinstance(events, list) or len(events) > 1_000_000:
            errors.append(f"frame {frame_index}: epoch/events rejected")
            continue
        epochs.add(epoch)
        if not events:
            if first != 0 or last != 0:
                errors.append(f"frame {frame_index}: empty batch has nonzero sequence bounds")
            continue
        if first != expected_sequence or last - first + 1 != len(events):
            errors.append(f"frame {frame_index}: lifecycle sequence gap")
            expected_sequence = last + 1
        for index, row in enumerate(events):
            if not isinstance(row, dict) or set(row) != {
                "sequence",
                "game_hour_raw",
                "owner_tag_db_id",
                "event_id",
                "originator_tag_db_id",
                "scope_random_seed",
                "due_before",
                "due_after",
                "reason",
            }:
                errors.append(f"frame {frame_index} event {index}: exact field set rejected")
                continue
            if (
                not isinstance(row["sequence"], str)
                or not isinstance(row["scope_random_seed"], str)
                or isinstance(row["game_hour_raw"], bool)
                or not isinstance(row["game_hour_raw"], int)
                or isinstance(row["owner_tag_db_id"], bool)
                or not isinstance(row["owner_tag_db_id"], int)
                or isinstance(row["event_id"], bool)
                or not isinstance(row["event_id"], int)
                or isinstance(row["originator_tag_db_id"], bool)
                or not isinstance(row["originator_tag_db_id"], int)
                or isinstance(row["due_before"], bool)
                or not isinstance(row["due_before"], int)
                or not isinstance(row["reason"], str)
            ):
                errors.append(f"frame {frame_index} event {index}: scalar rejected")
                continue
            try:
                sequence = int(row["sequence"])
                owner = int(row["owner_tag_db_id"])
                event_id = int(row["event_id"])
                originator = int(row["originator_tag_db_id"])
                scope_random = int(row["scope_random_seed"])
                due_before = int(row["due_before"])
            except ValueError:
                errors.append(f"frame {frame_index} event {index}: scalar rejected")
                continue
            if (
                row["sequence"] != str(sequence)
                or row["scope_random_seed"] != str(scope_random)
                or not 0 < sequence <= 0xFFFF_FFFF_FFFF_FFFF
                or not 0 <= scope_random <= 0xFFFF_FFFF_FFFF_FFFF
                or sequence != first + index
                or not -0x7FFF_FFFF <= row["game_hour_raw"] <= 0x7FFF_FFFF
                or owner <= 0
                or originator <= 0
                or not 100_000 <= event_id <= 100_099_999
                or not 0 <= due_before <= 500_000
                or row["reason"] not in REASONS
            ):
                errors.append(f"frame {frame_index} event {index}: identity/reason rejected")
                continue
            due_after = row["due_after"]
            if row["reason"] == "retained_decrement":
                if (
                    isinstance(due_after, bool)
                    or not isinstance(due_after, int)
                    or due_after + 1 != due_before
                ):
                    errors.append(
                        f"frame {frame_index} event {index}: decrement is not exactly one hour"
                    )
                    continue
            elif due_after is not None:
                errors.append(
                    f"frame {frame_index} event {index}: terminal row carries due_after"
                )
                continue
            observations.append(row)
        expected_sequence = last + 1

    covered = {row["reason"] for row in observations}
    if not observations:
        errors.append("no authenticated A19 lifecycle observations")
    if len(epochs) != 1:
        errors.append("capture does not have one uninterrupted hook epoch")
    missing = sorted(REASONS - covered)
    if missing:
        errors.append(f"missing lifecycle classes: {', '.join(missing)}")
    if not any(
        row["originator_tag_db_id"] != row["owner_tag_db_id"] for row in observations
    ):
        errors.append("no discriminating originator != recipient observation")
    return {
        "schema": "hoi4.a19.delayed_event_lifecycle_regrade.v1",
        "status": "pass" if not errors else "fail",
        "executable_sha256": SHA256,
        "frame_count": len(frames),
        "observation_count": len(observations),
        "covered_reasons": sorted(covered),
        "single_hook_epoch": len(epochs) == 1,
        "originator_recipient_separated": any(
            row["originator_tag_db_id"] != row["owner_tag_db_id"]
            for row in observations
        ),
        "errors": errors,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("capture", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    try:
        result = grade(load_frames(args.capture))
    except (OSError, UnicodeError, json.JSONDecodeError, ValueError) as exc:
        result = {
            "schema": "hoi4.a19.delayed_event_lifecycle_regrade.v1",
            "status": "fail",
            "executable_sha256": SHA256,
            "frame_count": 0,
            "observation_count": 0,
            "covered_reasons": [],
            "single_hook_epoch": False,
            "originator_recipient_separated": False,
            "errors": [f"capture input rejected: {exc}"],
        }
    encoded = json.dumps(result, indent=2, sort_keys=True) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(encoded, encoding="utf-8")
    print(encoded, end="")
    return 0 if result["status"] == "pass" else 1


if __name__ == "__main__":
    raise SystemExit(main())
