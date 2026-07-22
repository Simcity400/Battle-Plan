#!/usr/bin/env python3
"""Independent fail-closed join for an autonomous XUI21 acquisition."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
from pathlib import Path
from typing import Any

EXE_SHA = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
SOURCE_SAVE_SHA = "c8488b05a3b87c718eeff979976537b79e867634d0b4dccb1eef6c5a45362735"


class Reject(ValueError):
    pass


def load(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        raise Reject(f"cannot read {path.name}: {exc}") from exc
    if not isinstance(value, dict):
        raise Reject(f"{path.name} is not an object")
    return value


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def require(value: bool, message: str) -> None:
    if not value:
        raise Reject(message)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--seed-action", type=Path, required=True)
    parser.add_argument("--matrix-action", type=Path, required=True)
    parser.add_argument("--seed-manifest", type=Path, required=True)
    parser.add_argument("--matrix-manifest", type=Path, required=True)
    parser.add_argument("--runtime-receipt", type=Path, required=True)
    parser.add_argument("--runtime-regrade", type=Path, required=True)
    parser.add_argument("--profile", type=Path, required=True)
    parser.add_argument("--fixture", type=Path, required=True)
    parser.add_argument("--source-save", type=Path, required=True)
    parser.add_argument("--seeded-save", type=Path, required=True)
    parser.add_argument("--prebuilt-attestation", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        seed = load(args.seed_action)
        matrix = load(args.matrix_action)
        seed_manifest = load(args.seed_manifest)
        matrix_manifest = load(args.matrix_manifest)
        runtime = load(args.runtime_receipt)
        regrade = load(args.runtime_regrade)
        profile = load(args.profile)
        prebuilt = load(args.prebuilt_attestation)
        require(digest(args.source_save) == SOURCE_SAVE_SHA, "source save is not frozen New D04 Save Deep War")
        require(profile.get("schema") == "hoi4-xui21-autonomous-action-profile-v1", "profile schema mismatch")
        authority = profile.get("authority")
        require(isinstance(authority, dict), "profile authority absent")
        require(authority.get("executable_sha256") == EXE_SHA, "profile executable authority mismatch")
        require(authority.get("source_save_sha256") == SOURCE_SAVE_SHA, "profile source-save authority mismatch")
        require(authority.get("client_size") == [1920, 1080], "profile client-size authority mismatch")
        cells = profile.get("cells")
        require(isinstance(cells, list) and len(cells) == 20, "profile does not own 20 cells")
        classes = ("country", "news", "state", "unit_leader", "operative_leader")
        paths = ("player_selection", "ai_selection", "timeout_default_option", "timeout_no_choice")
        for index, row in enumerate(cells):
            require(isinstance(row, dict), f"profile cell {index} is not an object")
            require(row.get("cell_index") == index, f"profile cell {index} ordinal mismatch")
            require(row.get("popup_class") == classes[index // 4], f"profile cell {index} class mismatch")
            require(row.get("resolution_path") == paths[index % 4], f"profile cell {index} path mismatch")
        require(seed.get("schema") == "hoi4-xui21-autonomous-action-receipt-v1", "seed action schema mismatch")
        require(seed.get("status") == "pass_seeded_pending_save" and seed.get("mode") == "SeedPendingSave", "seed action did not pass")
        require(matrix.get("status") == "pass_machine_drove_complete_runtime_receipt" and matrix.get("mode") == "DriveMatrix", "matrix action did not pass")
        require(seed.get("executable_sha256") == EXE_SHA == matrix.get("executable_sha256"), "action binary mismatch")
        require(seed.get("profile_sha256") == digest(args.profile) == matrix.get("profile_sha256"), "action/profile join mismatch")
        require(seed.get("seeded_save_sha256") == digest(args.seeded_save), "seed action/save join mismatch")
        require(seed.get("process_id") == seed_manifest.get("pid"), "seed action/manifest PID mismatch")
        require(matrix.get("process_id") == matrix_manifest.get("pid"), "matrix action/manifest PID mismatch")
        require(seed.get("process_id") != matrix.get("process_id"), "capture-start baseline reused the seeding process")
        require(seed.get("process_start_utc") != matrix.get("process_start_utc"), "capture-start baseline reused one process epoch")
        require(matrix.get("raw_coverage_receipt_sha256") == digest(args.runtime_receipt), "matrix action/runtime receipt hash mismatch")
        require(runtime.get("schema") == "hoi4-xui21-runtime-coverage-receipt-v1", "runtime receipt schema mismatch")
        require(runtime.get("executable_sha256") == EXE_SHA, "runtime receipt binary mismatch")
        require(runtime.get("matrix_bits") == 1048575, "runtime matrix is incomplete")
        observations = runtime.get("observations")
        require(isinstance(observations, list) and len(observations) == 20, "runtime receipt lacks 20 cells")
        lifecycle = runtime.get("lifecycle")
        require(isinstance(lifecycle, dict) and set(lifecycle) == {"repeated_event_id", "same_hour_multiple_firings", "capture_start_pending", "non_materialised_resolution"}, "runtime lifecycle schema mismatch")
        require(all(value is not None for value in lifecycle.values()), "runtime receipt lacks a lifecycle witness")
        require(regrade.get("schema") == "hoi4-xui21-runtime-coverage-regrade-v1" and regrade.get("ok") is True, "independent runtime regrade rejected")
        require(regrade.get("receipt_sha256") == digest(args.runtime_receipt), "runtime regrade/receipt hash mismatch")
        require(regrade.get("session_id") == runtime.get("session_id"), "runtime regrade session mismatch")
        require(re.fullmatch(r"p[0-9]+-n[0-9]+-s[0-9]+", str(runtime.get("session_id", ""))) is not None, "runtime session identity malformed")
        require(prebuilt.get("schema") is not None, "prebuilt attestation schema absent")
        result = {
            "schema": "hoi4-xui21-autonomous-acquisition-verification-v1",
            "status": "pass_exact_save_seed_fresh_session_20_cells_4_edges",
            "executable_sha256": EXE_SHA,
            "source_save_sha256": SOURCE_SAVE_SHA,
            "seeded_save_sha256": digest(args.seeded_save),
            "fixture_sha256": digest(args.fixture),
            "profile_sha256": digest(args.profile),
            "runtime_receipt_sha256": digest(args.runtime_receipt),
            "runtime_session_id": runtime["session_id"],
            "matrix_bits": runtime["matrix_bits"],
            "cell_count": len(observations),
            "lifecycle_witness_count": 4,
            "fresh_process_boundary": True,
            "independent_runtime_regrade": True,
            "promotion_authorized": False,
        }
    except (OSError, Reject) as exc:
        result = {
            "schema": "hoi4-xui21-autonomous-acquisition-verification-v1",
            "status": "fail_closed",
            "error": str(exc),
            "promotion_authorized": False,
        }
    encoded = json.dumps(result, indent=2, sort_keys=True) + "\n"
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("x", encoding="utf-8", newline="\n") as stream:
        stream.write(encoded)
        stream.flush()
        os.fsync(stream.fileno())
    print(encoded, end="")
    return 0 if result["status"].startswith("pass_") else 1


if __name__ == "__main__":
    raise SystemExit(main())
