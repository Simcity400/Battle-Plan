#!/usr/bin/env python3
"""Fail-closed H-SINK-OK / H-SINK-FAIL receipt regrader.

This tool never launches or attaches to HOI4.  It grades only immutable files in
one completed run directory and emits a hash-bound certification.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any


HEX64 = set("0123456789abcdef")
EXACT_A729_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"


def load_object(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"{path} is not a JSON object")
    return value


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def is_hex64(value: Any) -> bool:
    return isinstance(value, str) and len(value) == 64 and set(value) <= HEX64


def require(condition: bool, problem: str, problems: list[str]) -> None:
    if not condition:
        problems.append(problem)


def nested(root: dict[str, Any], *keys: str) -> Any:
    value: Any = root
    for key in keys:
        if not isinstance(value, dict):
            return None
        value = value.get(key)
    return value


def grade_success(run_dir: Path, problems: list[str]) -> dict[str, Any]:
    manifest_path = run_dir / "native_session_manifest.json"
    require(manifest_path.is_file(), "native_session_manifest_missing", problems)
    if not manifest_path.is_file():
        return {"manifest": None}
    manifest = load_object(manifest_path)
    acceptance = manifest.get("acceptance_telemetry")
    require(isinstance(acceptance, dict), "acceptance_telemetry_missing", problems)
    acceptance = acceptance if isinstance(acceptance, dict) else {}
    require(
        acceptance.get("schema") == "battle-plan.acceptance_telemetry.v1",
        "acceptance_schema_invalid",
        problems,
    )
    require(acceptance.get("complete") is True, "acceptance_not_complete", problems)
    require(acceptance.get("problems") == [], "acceptance_problems_present", problems)

    launch = manifest.get("early_render_launch_manifest")
    launch = launch if isinstance(launch, dict) else {}
    capture_receipt = manifest.get("early_render_capture_receipt")
    capture_receipt = capture_receipt if isinstance(capture_receipt, dict) else {}
    require(launch.get("executable_sha256") == EXACT_A729_SHA256,
            "success_launch_not_exact_a729", problems)
    require(capture_receipt.get("executable_sha256") == EXACT_A729_SHA256,
            "success_capture_not_exact_a729", problems)
    require(capture_receipt.get("created_suspended") is True,
            "success_capture_not_created_suspended", problems)
    require(is_hex64(launch.get("collector_sha256")),
            "success_launch_collector_sha256_invalid", problems)
    require(capture_receipt.get("collector_sha256") == launch.get("collector_sha256"),
            "success_collector_identity_mismatch", problems)
    require(is_hex64(manifest.get("early_render_capture_receipt_sha256")),
            "success_capture_receipt_sha256_invalid", problems)

    host_write = acceptance.get("sink_write_us")
    host_write = host_write if isinstance(host_write, dict) else {}
    require(isinstance(host_write.get("count"), int) and host_write["count"] > 0,
            "host_sink_write_samples_empty", problems)
    require(isinstance(host_write.get("avg"), (int, float)),
            "host_sink_write_average_missing", problems)
    require(isinstance(host_write.get("max"), int), "host_sink_write_max_missing", problems)
    require(
        acceptance.get("sink_write_scope")
        == "host StreamWriter::write_all calls over live named-pipe chunks",
        "host_sink_write_scope_invalid",
        problems,
    )
    clock = acceptance.get("clock_skew")
    clock = clock if isinstance(clock, dict) else {}
    require(bool(clock), "clock_skew_unmeasured", problems)
    require(clock.get("wall_monotonic_ok") is True, "wall_clock_non_monotonic", problems)

    lifecycle = acceptance.get("sink_lifecycle")
    lifecycle = lifecycle if isinstance(lifecycle, dict) else {}
    attempts = lifecycle.get("write_attempts")
    require(lifecycle.get("requested") == 1, "sink_requested_not_one", problems)
    require(lifecycle.get("opened") == 1, "sink_opened_not_one", problems)
    require(lifecycle.get("open_failures") == 0, "sink_open_failure_seen", problems)
    require(isinstance(attempts, int) and attempts > 0, "sink_write_attempts_empty", problems)
    require(lifecycle.get("write_finishes") == attempts, "sink_write_finish_mismatch", problems)
    require(lifecycle.get("write_commits") == attempts, "sink_write_commit_mismatch", problems)
    require(lifecycle.get("write_failures") == 0, "sink_write_failure_seen", problems)
    require(lifecycle.get("write_in_flight") == 0, "sink_write_left_in_flight", problems)
    require(lifecycle.get("file_bytes_committed") == 0, "file_sink_was_not_disabled", problems)
    require(
        isinstance(lifecycle.get("pipe_bytes_committed"), int)
        and lifecycle["pipe_bytes_committed"] > 0,
        "pipe_sink_committed_no_bytes",
        problems,
    )

    latch = nested(manifest, "compact_stream", "hour_latch")
    latch = latch if isinstance(latch, dict) else {}
    require(latch.get("terminal_proof_seen") is True, "terminal_latch_proof_missing", problems)
    require(latch.get("configured_sink_count") == 1, "configured_sink_not_one", problems)
    require(latch.get("pipe_sink_enabled") == 1, "pipe_sink_not_enabled", problems)
    require(latch.get("file_sink_enabled") == 0, "file_sink_enabled", problems)
    require(latch.get("per_hour_delivery_proof_complete") is True,
            "per_hour_delivery_unproven", problems)
    require(latch.get("sink_write_success_verified") is True,
            "sink_write_success_unverified", problems)
    require(latch.get("sink_write_gap") is None, "sink_write_gap_present", problems)
    require(nested(manifest, "hourly_requirement", "met") is True,
            "hourly_requirement_not_met", problems)

    replay_name = manifest.get("replay_file")
    require(isinstance(replay_name, str) and Path(replay_name).name == replay_name,
            "replay_file_name_invalid", problems)
    if isinstance(replay_name, str) and Path(replay_name).name == replay_name:
        require((run_dir / replay_name).is_file(), "replay_file_missing", problems)
    finalize = manifest.get("host_finalize")
    require(isinstance(finalize, dict), "host_finalize_missing", problems)
    if isinstance(finalize, dict):
        require(nested(finalize, "fold", "skipped") is not True,
                "host_fold_skipped", problems)
        require(nested(finalize, "fold", "truncated_tail_bytes") == 0,
                "host_fold_truncated_tail", problems)
        require(nested(finalize, "fold", "framing_stopped") is None,
                "host_fold_framing_stopped", problems)
        require(nested(finalize, "verify", "stream_sha256_matches_capture") is True,
                "host_verify_stream_hash_mismatch", problems)
    require(not (run_dir / "collector_sink_failure.json").exists(),
            "failure_receipt_present_in_success_run", problems)
    return {"manifest": str(manifest_path), "manifest_sha256": sha256(manifest_path)}


def grade_failure(run_dir: Path, expected: str, problems: list[str]) -> dict[str, Any]:
    receipt_path = run_dir / "collector_sink_failure.json"
    plan_path = run_dir / "sink_failure_acceptance_plan.json"
    require(receipt_path.is_file(), "sink_failure_receipt_missing", problems)
    require(plan_path.is_file(), "sink_failure_acceptance_plan_missing", problems)
    if not receipt_path.is_file() or not plan_path.is_file():
        return {"receipt": None, "plan": None}
    receipt = load_object(receipt_path)
    plan = load_object(plan_path)
    require(receipt.get("schema") == "extractor.authenticated-sink-failure.v2",
            "sink_failure_schema_invalid", problems)
    require(plan.get("schema") == "extractor.sink-failure-acceptance-plan.v1",
            "sink_failure_plan_schema_invalid", problems)
    require(receipt.get("admissible_for_production") is False,
            "failure_receipt_claimed_production", problems)
    require(receipt.get("attempt_terminal") is True, "failure_not_terminal", problems)
    require(receipt.get("authenticated_h4ct_session") is True,
            "failure_h4ct_not_authenticated", problems)
    require(receipt.get("replay_created") is False, "failure_claimed_replay", problems)
    acceptance = receipt.get("acceptance_test")
    acceptance = acceptance if isinstance(acceptance, dict) else {}
    require(acceptance.get("mode") == expected, "failure_mode_mismatch", problems)
    require(acceptance.get("report_matches_armed_fault") is True,
            "armed_fault_report_mismatch", problems)
    require(is_hex64(acceptance.get("nonce")), "failure_nonce_invalid", problems)
    require(is_hex64(acceptance.get("control_authorization_v1")),
            "failure_control_authorization_invalid", problems)
    require(acceptance.get("acceptance_plan_sha256") == sha256(plan_path),
            "failure_plan_hash_mismatch", problems)
    require(plan.get("mode") == expected, "plan_mode_mismatch", problems)
    require(plan.get("nonce") == acceptance.get("nonce"), "plan_nonce_mismatch", problems)
    require(plan.get("control_authorization_v1") == acceptance.get("control_authorization_v1"),
            "plan_control_authorization_mismatch", problems)
    for key in ("session_id", "capture_id", "process_id"):
        require(plan.get(key) == receipt.get(key), f"plan_{key}_mismatch", problems)
    game_executable_file = plan.get("game_executable_file")
    require(isinstance(game_executable_file, str)
            and Path(game_executable_file).name == game_executable_file,
            "plan_game_executable_file_invalid", problems)
    require(plan.get("game_executable_sha256") == EXACT_A729_SHA256,
            "plan_game_executable_not_exact_a729", problems)
    collector_dll_file = plan.get("collector_dll_file")
    require(isinstance(collector_dll_file, str)
            and Path(collector_dll_file).name == collector_dll_file,
            "plan_collector_dll_file_invalid", problems)
    require(is_hex64(plan.get("collector_dll_sha256")),
            "plan_collector_dll_sha256_invalid", problems)

    pipe_bit = 2
    require(receipt.get("requested_bits") == pipe_bit, "failure_not_sole_pipe", problems)
    require(receipt.get("failed_bits") == pipe_bit, "failed_sink_not_pipe", problems)
    require(receipt.get("file_bytes_committed") == 0, "failure_file_bytes_nonzero", problems)
    require(receipt.get("error_class") == "OtherIo", "fault_error_class_invalid", problems)
    require(receipt.get("win32_error") is None, "fault_win32_error_present", problems)
    if expected == "sink-open-fault":
        require(receipt.get("kind") == "Setup", "open_fault_kind_invalid", problems)
        require(receipt.get("opened_bits") == 0, "open_fault_claimed_open", problems)
        for key in ("last_complete_sequence", "failed_sequence"):
            require(receipt.get(key) is None, f"open_fault_{key}_present", problems)
        require(receipt.get("pipe_bytes_committed") == 0,
                "open_fault_pipe_bytes_nonzero", problems)
    else:
        require(receipt.get("kind") == "Terminal", "write_fault_kind_invalid", problems)
        require(receipt.get("opened_bits") == pipe_bit, "write_fault_pipe_not_open", problems)
        last = receipt.get("last_complete_sequence")
        failed = receipt.get("failed_sequence")
        require(isinstance(last, int) and isinstance(failed, int) and last + 1 == failed,
                "write_fault_sequence_progress_invalid", problems)
        require(isinstance(receipt.get("pipe_bytes_committed"), int)
                and receipt["pipe_bytes_committed"] > 0,
                "write_fault_has_no_committed_prefix", problems)
    require(not (run_dir / "native_session_manifest.json").exists(),
            "failure_run_published_session_manifest", problems)
    require(not any(run_dir.glob("*.battleplan")), "failure_run_published_replay", problems)
    return {
        "receipt": str(receipt_path),
        "receipt_sha256": sha256(receipt_path),
        "plan": str(plan_path),
        "plan_sha256": sha256(plan_path),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--run-dir", required=True, type=Path)
    parser.add_argument(
        "--expect",
        required=True,
        choices=("success", "sink-open-fault", "sink-write-fault"),
    )
    parser.add_argument("--out", type=Path)
    args = parser.parse_args()
    run_dir = args.run_dir.resolve()
    problems: list[str] = []
    evidence = (
        grade_success(run_dir, problems)
        if args.expect == "success"
        else grade_failure(run_dir, args.expect, problems)
    )
    result = {
        "schema": "extractor.sink-reconciliation-verification.v1",
        "run_dir": str(run_dir),
        "expected_outcome": args.expect,
        "verdict": "passed" if not problems else "failed",
        "problems": problems,
        "evidence": evidence,
    }
    encoded = json.dumps(result, indent=2, sort_keys=True) + "\n"
    if args.out:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(encoded, encoding="utf-8")
    print(encoded, end="")
    return 0 if not problems else 1


if __name__ == "__main__":
    raise SystemExit(main())
