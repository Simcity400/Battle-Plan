#!/usr/bin/env python3
"""Fail-closed H-SINK-OK / H-SINK-FAIL receipt regrader.

This tool never launches or attaches to HOI4.  It grades only immutable files in
one completed run directory and emits a hash-bound certification.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import struct
from pathlib import Path
from typing import Any


HEX64 = set("0123456789abcdef")
EXACT_A729_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
EARLY_LAUNCH_SCHEMA = "extractor.early-render-launch-manifest.v1"
EARLY_CAPTURE_SCHEMA = "extractor.early-render-capture-receipt.v1"
EARLY_STAGED_HOOK_COUNT = 71


def reject_duplicate_members(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    value: dict[str, Any] = {}
    for key, child in pairs:
        if key in value:
            raise ValueError(f"duplicate JSON member: {key}")
        value[key] = child
    return value


def load_object(path: Path) -> dict[str, Any]:
    value = json.loads(
        path.read_text(encoding="utf-8"), object_pairs_hook=reject_duplicate_members
    )
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


def is_finite_number(value: Any) -> bool:
    if type(value) is int:
        return True
    return type(value) is float and math.isfinite(value)


def canonical_json_sha256(value: Any) -> str:
    # Rust serde_json::to_vec emits compact UTF-8 in struct/member order.  The
    # decoder above preserves that order, so this reproduces the recorder hash.
    encoded = json.dumps(
        value, ensure_ascii=False, allow_nan=False, separators=(",", ":")
    ).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def runtime_dlc_roster_sha256(names: list[str]) -> str:
    preimage = bytearray(b"hoi4-a729-runtime-dlc-roster-v1\0")
    preimage += struct.pack("<I", len(names))
    for ordinal, name in enumerate(names):
        raw = name.encode("utf-8")
        preimage += struct.pack("<II", ordinal, len(raw))
        preimage += raw
    return hashlib.sha256(preimage).hexdigest()


def expected_early_transcript_sha256(receipt: dict[str, Any]) -> str:
    keys = (
        "launch_manifest_sha256",
        "effective_vfs_index_sha256",
        "effective_vfs_index_row_count",
        "effective_vfs_index_chunk_count",
        "content_manifest_bytes_sha256",
        "content_manifest_byte_count",
        "content_manifest_chunk_count",
        "process_id",
        "process_start_nonce",
        "hook_epoch_nonce",
        "hook_set_sha256",
        "hook_ready_sequence",
        "resume_sequence",
        "hook_ready_qpc",
        "resume_qpc",
        "first_creation_sequence",
        "first_game_d3d_creation_qpc",
        "staged_hook_generation",
        "staged_hook_count",
        "staged_hook_set_sha256",
        "game_device_ptr",
        "immediate_context_ptr",
        "effective_runtime_dlc",
    )
    transcript: dict[str, Any] = {
        "events": [
            "created_suspended",
            "injection_committed",
            "collector_epoch_bound",
            "hooks_ready",
            "primary_thread_resumed",
            "first_game_d3d_creation",
            "staged_hook_set_ready",
            "runtime_dlc_roster_validated",
        ]
    }
    for key in keys:
        transcript[key] = receipt.get(key)
    return canonical_json_sha256(transcript)


def grade_early_capture(
    launch: dict[str, Any], receipt: dict[str, Any], recorded_sha256: Any,
    problems: list[str],
) -> None:
    require(launch.get("schema") == EARLY_LAUNCH_SCHEMA,
            "success_launch_schema_invalid", problems)
    require(receipt.get("schema") == EARLY_CAPTURE_SCHEMA,
            "success_capture_schema_invalid", problems)
    require(receipt.get("launch_manifest_sha256") == canonical_json_sha256(launch),
            "success_launch_manifest_digest_mismatch", problems)
    require(recorded_sha256 == canonical_json_sha256(receipt),
            "success_capture_receipt_digest_mismatch", problems)
    require(receipt.get("effective_vfs_manifest_sha256")
            == launch.get("effective_vfs_manifest_sha256"),
            "success_effective_vfs_mismatch", problems)
    require(receipt.get("collector_sha256") == launch.get("collector_sha256"),
            "success_collector_identity_mismatch", problems)
    require(receipt.get("created_suspended") is True,
            "success_capture_not_created_suspended", problems)
    for key in (
        "process_id", "process_start_nonce", "hook_epoch_nonce",
        "hook_ready_sequence", "resume_sequence", "hook_ready_qpc", "resume_qpc",
        "first_creation_sequence", "first_game_d3d_creation_qpc",
        "game_device_ptr", "immediate_context_ptr",
    ):
        require(isinstance(receipt.get(key), int) and not isinstance(receipt.get(key), bool)
                and receipt[key] > 0, f"success_{key}_invalid", problems)
    require(receipt.get("resume_sequence", 0) > receipt.get("hook_ready_sequence", 0),
            "success_resume_sequence_not_after_ready", problems)
    require(receipt.get("resume_qpc", 0) > receipt.get("hook_ready_qpc", 0),
            "success_resume_qpc_not_after_ready", problems)
    require(receipt.get("first_game_d3d_creation_qpc", 0) >= receipt.get("resume_qpc", 0),
            "success_first_creation_not_after_resume", problems)
    require(receipt.get("staged_hook_generation") == 1,
            "success_staged_hook_generation_invalid", problems)
    require(receipt.get("diagnostic_staged_hook_mask") is None,
            "success_diagnostic_hook_subset_present", problems)
    require(receipt.get("staged_hook_count") == EARLY_STAGED_HOOK_COUNT,
            "success_staged_hook_count_invalid", problems)
    require(receipt.get("game_d3d_creation_count_at_hook_ready") == 0,
            "success_game_d3d_preexisted_at_ready", problems)
    require(receipt.get("preexisting_game_d3d_object_count") == 0,
            "success_preexisting_game_d3d_object", problems)
    completeness = receipt.get("completeness")
    expected_completeness = {
        "all_shader_create", "input_layout_create", "blend_state_create",
        "depth_stencil_state_create", "rasterizer_state_create",
        "sampler_state_create", "buffer_create_update", "texture_create_update",
        "resource_view_create", "draw_state_readback",
    }
    require(isinstance(completeness, dict)
            and set(completeness) == expected_completeness
            and all(value is True for value in completeness.values()),
            "success_hook_completeness_invalid", problems)
    require(receipt.get("transcript_sha256") == expected_early_transcript_sha256(receipt),
            "success_capture_transcript_digest_mismatch", problems)

    runtime = receipt.get("effective_runtime_dlc")
    runtime = runtime if isinstance(runtime, dict) else {}
    names = runtime.get("active_display_names_engine_order")
    names = names if isinstance(names, list) and all(isinstance(v, str) for v in names) else []
    require(runtime.get("schema") == "hoi4.effective_runtime_dlc.v1",
            "success_runtime_dlc_schema_invalid", problems)
    require(runtime.get("source") == "engine_active_dlc_manager_v1",
            "success_runtime_dlc_source_invalid", problems)
    require(runtime.get("sample_count") == 2 and runtime.get("double_stable") is True,
            "success_runtime_dlc_not_double_stable", problems)
    require(runtime.get("manager_getter_rva") == 0x02060F10,
            "success_runtime_dlc_manager_rva_invalid", problems)
    require(runtime.get("active_dlc_enumerator_rva") == 0x02063660,
            "success_runtime_dlc_enumerator_rva_invalid", problems)
    require(runtime.get("string_vector_teardown_rva") == 0x001207D0,
            "success_runtime_dlc_teardown_rva_invalid", problems)
    require(runtime.get("engine_order_sha256") == runtime_dlc_roster_sha256(names),
            "success_runtime_dlc_roster_digest_mismatch", problems)


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
    plan_path = run_dir / "early_launch_authorization_plan.json"
    resolved_path = run_dir / "early_launch_resolved_attempt.json"
    admission_path = run_dir / "prepared_product_modset_admission.json"
    require(plan_path.is_file(), "early_launch_authorization_missing", problems)
    require(resolved_path.is_file(), "early_launch_resolved_attempt_missing", problems)
    require(admission_path.is_file(), "prepared_product_modset_admission_missing", problems)
    plan = load_object(plan_path) if plan_path.is_file() else {}
    resolved = load_object(resolved_path) if resolved_path.is_file() else {}
    admission = load_object(admission_path) if admission_path.is_file() else {}
    require(plan.get("schema") == "battle-plan.early_launch_authorization.v1",
            "early_launch_authorization_schema_invalid", problems)
    require(plan.get("purpose") == "Production",
            "early_launch_authorization_not_full_production", problems)
    require(plan.get("operator_approved") is True and plan.get("dry_run") is False,
            "early_launch_not_operator_authorized", problems)
    require(plan.get("attempt_budget") == 1, "early_launch_attempt_budget_invalid", problems)
    require(resolved.get("schema") == "battle-plan.early_launch_resolved_attempt.v1",
            "early_launch_resolved_attempt_schema_invalid", problems)
    if plan_path.is_file():
        require(resolved.get("authorization_plan_sha256") == sha256(plan_path),
                "early_launch_resolved_plan_hash_mismatch", problems)
    require(plan.get("executable_sha256") == EXACT_A729_SHA256,
            "early_launch_plan_not_exact_a729", problems)
    require(admission.get("schema") == "extractor.product_modset_capture_admission.v1"
            and admission.get("verdict") == "passed",
            "prepared_product_modset_admission_invalid", problems)
    if admission_path.is_file():
        require(plan.get("prepared_admission_sha256") == sha256(admission_path),
                "prepared_product_modset_admission_hash_mismatch", problems)
    require(plan.get("product_modset_sha256") == admission.get("product_modset_sha256"),
            "early_launch_product_modset_mismatch", problems)
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
    require(plan.get("collector_sha256") == launch.get("collector_sha256"),
            "success_plan_collector_identity_mismatch", problems)
    require(plan.get("session_id") == manifest.get("session_id"),
            "success_plan_session_id_mismatch", problems)
    require(plan.get("capture_id") == manifest.get("capture_id"),
            "success_plan_capture_id_mismatch", problems)
    require(resolved.get("native_session_manifest_sha256") == sha256(manifest_path),
            "success_resolved_manifest_hash_mismatch", problems)
    require(resolved.get("session_id") == manifest.get("session_id"),
            "success_resolved_session_id_mismatch", problems)
    require(resolved.get("capture_id") == manifest.get("capture_id"),
            "success_resolved_capture_id_mismatch", problems)
    require(resolved.get("process_id") == capture_receipt.get("process_id"),
            "success_resolved_process_id_mismatch", problems)
    require(resolved.get("process_start_nonce") == capture_receipt.get("process_start_nonce"),
            "success_resolved_process_nonce_mismatch", problems)
    require(resolved.get("early_render_capture_receipt_sha256")
            == manifest.get("early_render_capture_receipt_sha256"),
            "success_resolved_capture_receipt_mismatch", problems)
    require(plan.get("product_modset_sha256")
            == nested(manifest, "static_bundle", "product_modset_sha256"),
            "success_manifest_product_modset_mismatch", problems)
    require(resolved.get("product_modset_sha256") == plan.get("product_modset_sha256"),
            "success_resolved_product_modset_mismatch", problems)
    require(is_hex64(manifest.get("early_render_capture_receipt_sha256")),
            "success_capture_receipt_sha256_invalid", problems)
    if launch and capture_receipt:
        grade_early_capture(
            launch, capture_receipt,
            manifest.get("early_render_capture_receipt_sha256"), problems,
        )

    host_write = acceptance.get("sink_write_us")
    host_write = host_write if isinstance(host_write, dict) else {}
    require(isinstance(host_write.get("count"), int) and host_write["count"] > 0,
            "host_sink_write_samples_empty", problems)
    require(isinstance(host_write.get("avg"), (int, float)),
            "host_sink_write_average_missing", problems)
    require(isinstance(host_write.get("max"), int), "host_sink_write_max_missing", problems)
    require(
        acceptance.get("sink_write_scope")
        == "recorder StreamWriter::write_all calls over the complete physical H4CR capture; capture-set scoped during rollover; buffered/compressed host-path diagnostic, not logical-stream collector sink latency",
        "host_sink_write_scope_invalid",
        problems,
    )
    collector_write = acceptance.get("collector_sink_write_us")
    collector_write = collector_write if isinstance(collector_write, dict) else {}
    require(
        collector_write.get("scope")
        == "collector synchronous writes to the sole authoritative H4CR pipe sink, finalized by the in-band terminal latch record",
        "collector_sink_write_scope_invalid",
        problems,
    )
    collector_count = collector_write.get("count")
    collector_p50 = collector_write.get("p50")
    collector_p99 = collector_write.get("p99")
    collector_max = collector_write.get("max")
    require(type(collector_count) is int and collector_count > 0,
            "collector_sink_time_samples_empty", problems)
    require(type(collector_write.get("total")) is int
            and collector_write["total"] >= 0,
            "collector_sink_time_total_missing", problems)
    require(is_finite_number(collector_write.get("avg"))
            and collector_write["avg"] >= 0,
            "collector_sink_time_average_missing", problems)
    require(
        all(type(value) is int and value >= 0
            for value in (collector_p50, collector_p99, collector_max))
        and collector_p50 <= collector_p99 <= collector_max,
        "collector_sink_time_percentiles_invalid",
        problems,
    )
    require(
        type(collector_write.get("total")) is int
        and type(collector_max) is int
        and collector_max <= collector_write["total"],
        "collector_sink_time_total_invalid",
        problems,
    )
    clock = acceptance.get("clock_skew")
    clock = clock if isinstance(clock, dict) else {}
    require(bool(clock), "clock_skew_unmeasured", problems)
    require(clock.get("wall_monotonic_ok") is True, "wall_clock_non_monotonic", problems)
    require(
        acceptance.get("clock_skew_scope")
        == "complete physical H4CR capture; capture-set scoped during rollover",
        "clock_skew_scope_invalid",
        problems,
    )

    lifecycle = acceptance.get("sink_lifecycle")
    lifecycle = lifecycle if isinstance(lifecycle, dict) else {}
    attempts = lifecycle.get("write_attempts")
    require(lifecycle.get("requested") == 1, "sink_requested_not_one", problems)
    require(lifecycle.get("open_attempts") == 1, "sink_open_attempts_not_one", problems)
    require(lifecycle.get("open_finishes") == 1, "sink_open_finishes_not_one", problems)
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
    latch_collector_write = latch.get("sink_write_us")
    latch_collector_write = latch_collector_write if isinstance(latch_collector_write, dict) else {}
    require(isinstance(latch_collector_write.get("count"), int)
            and latch_collector_write["count"] > 0,
            "collector_sink_time_samples_empty", problems)
    require(latch_collector_write.get("count") == lifecycle.get("write_finishes"),
            "collector_sink_time_sample_count_mismatch", problems)
    require(collector_write.get("count") == latch_collector_write.get("count")
            and collector_write.get("total") == latch_collector_write.get("total")
            and collector_write.get("p50") == latch_collector_write.get("p50")
            and collector_write.get("p99") == latch_collector_write.get("p99")
            and collector_write.get("max") == latch_collector_write.get("max"),
            "acceptance_collector_sink_time_mismatch", problems)
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
    return {
        "manifest": str(manifest_path),
        "manifest_sha256": sha256(manifest_path),
        "early_launch_authorization": str(plan_path),
        "early_launch_authorization_sha256": sha256(plan_path) if plan_path.is_file() else None,
        "early_launch_resolved_attempt_sha256": sha256(resolved_path) if resolved_path.is_file() else None,
        "prepared_product_modset_admission_sha256": (
            sha256(admission_path) if admission_path.is_file() else None
        ),
    }


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
    require(plan.get("admissible_for_production") is False,
            "failure_plan_claimed_production", problems)
    require(plan.get("expected_receipt_schema")
            == "extractor.authenticated-sink-failure.v2",
            "failure_plan_expected_receipt_schema_invalid", problems)
    require(plan.get("expected_topology") == "sole_pipe",
            "failure_plan_topology_invalid", problems)
    require(plan.get("expected_terminality") == "attempt_terminal",
            "failure_plan_terminality_invalid", problems)
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
    require(isinstance(receipt.get("session_id"), str)
            and bool(receipt["session_id"].strip()),
            "failure_session_id_invalid", problems)
    require(isinstance(receipt.get("capture_id"), str)
            and bool(receipt["capture_id"].strip()),
            "failure_capture_id_invalid", problems)
    require(isinstance(receipt.get("process_id"), int)
            and not isinstance(receipt.get("process_id"), bool)
            and receipt["process_id"] > 0,
            "failure_process_id_invalid", problems)
    for key in ("session_id", "capture_id", "process_id"):
        require(plan.get(key) == receipt.get(key), f"plan_{key}_mismatch", problems)
    game_executable_file = plan.get("game_executable_file")
    require(isinstance(game_executable_file, str)
            and Path(game_executable_file).name == game_executable_file,
            "plan_game_executable_file_invalid", problems)
    require(plan.get("game_executable_sha256") == EXACT_A729_SHA256,
            "plan_game_executable_not_exact_a729", problems)
    recorder_executable_file = plan.get("recorder_executable_file")
    require(isinstance(recorder_executable_file, str)
            and Path(recorder_executable_file).name == recorder_executable_file,
            "plan_recorder_executable_file_invalid", problems)
    require(is_hex64(plan.get("recorder_executable_sha256")),
            "plan_recorder_executable_sha256_invalid", problems)
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
