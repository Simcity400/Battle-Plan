#!/usr/bin/env python3
"""Fail-closed verifier for the H-A29 observed research-ahead fixture."""

from __future__ import annotations

import argparse
from decimal import Decimal, InvalidOperation
import hashlib
import json
import os
from pathlib import Path
import re
import struct
import tempfile
import zipfile


EXPECTED_EXECUTABLE_SHA256 = (
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
)
EXPECTED_AUTHORITY = "a729_observed_research_ahead_selected_present_v3"
EXPECTED_ROOT_JOB_ID = "20260720T124311Z-w5-a29-effective-producer-roots-e50d79d0"
EXPECTED_ROOT_MANIFEST_SHA256 = (
    "82884a9a122f95298d5068bdb68590806346c2c1ac274ca3b56889bb0e5abcc2"
)
EXPECTED_SENTINEL_JOB_ID = (
    "20260720T130641Z-w5-a29-hook-sentinels-and-substitution-i-9016d012"
)
EXPECTED_SENTINEL_DUMP_SHA256 = (
    "3fbca6726f09499e6cf1961969ae4154b9c265f8e5c688f8d939d8397387cee2"
)
EXPECTED_ACTION_GATE_JOB_ID = (
    "20260721T075619Z-wave6-a29-active-ahead-ui-producer-ced43f28"
)
EXPECTED_ACTION_GATE_MANIFEST_SHA256 = (
    "8d802c9eb2fe5e78ab52d7512d8d01dd5793b9c29fac8627c17fb6cafcc621ba"
)
EXPECTED_ACTION_GATE_SENTINEL_JOB_ID = (
    "20260721T081803Z-wave6-a29-year-action-gate-sentinels-442109ac"
)
EXPECTED_ACTION_GATE_SENTINEL_DUMP_SHA256 = (
    "44169d40d8ccc979f2f78eb7be11e2e7995e28dc4f3c6feb9da48ae928257517"
)
EXPECTED_UI_YEAR_PRODUCER_RVA = 0x01BCEA60
EXPECTED_UI_YEAR_HELPER_RVA = 0x00ABEAA0
EXPECTED_UI_YEAR_ACTION_IDENTITY_COMPARE_RVA = 0x01BCF326
EXPECTED_UI_YEAR_HELPER_RETURN_RVA = 0x01BCF384
EXPECTED_EFFECTIVE_VALUE_PRODUCER_RVA = 0x00ABE4C0
EXPECTED_LOCALIZATION_CONSUMER_RVA = 0x02230F20
EXPECTED_FIXED_SCALE = 100_000
DATE_EPOCH_RAW = 43_799_999
EXPECTED_D04_GAME_HOUR_RAW = 60_830_441
EXPECTED_D04_GAME_HOUR_NORMALIZED = EXPECTED_D04_GAME_HOUR_RAW - DATE_EPOCH_RAW
EXPECTED_CLOCK_CONTRACT = {
    "game_state_slot_rva": "0x3316E90",
    "clock_singleton_a_rva": "0x33172E0",
    "clock_singleton_b_rva": "0x33172E8",
    "clock_vtable_rva": "0x2952510",
    "pause_offset": "0x6C1",
    "game_hour_offset": "0x468",
    "game_speed_offset": "0x4BC",
}
MAX_SNAPSHOT_COUNT = 10_000
MAX_SNAPSHOT_BYTES = 128 * 1024 * 1024
CANONICAL_ID = re.compile(r"^[a-z0-9][a-z0-9_.-]{7,159}$")

VALUE_FIELDS = (
    ("ahead_of_time_years_raw", "ahead_of_time_years"),
    ("ahead_of_time_penalty_raw", "ahead_of_time_penalty_pct"),
    ("ahead_of_time_reduction_years_raw", "ahead_of_time_reduction_years"),
    ("ahead_of_time_reduced_penalty_raw", "ahead_of_time_reduced_penalty_pct"),
)
A29_FIELDS = {
    "ahead_of_time_fixed_scale",
    "ahead_of_time_provenance",
    *(field for pair in VALUE_FIELDS for field in pair),
}


def exact_int(value, minimum, maximum):
    return (
        isinstance(value, int)
        and not isinstance(value, bool)
        and minimum <= value <= maximum
    )


def lower_hex_sha256(value):
    return (
        isinstance(value, str)
        and len(value) == 64
        and all(character in "0123456789abcdef" for character in value)
    )


def nonzero_lower_hex_sha256(value):
    return lower_hex_sha256(value) and any(character != "0" for character in value)


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def write_json_atomic(path: Path, value) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    payload = (json.dumps(value, indent=2, sort_keys=True) + "\n").encode("utf-8")
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=path.name + ".", suffix=".tmp", dir=str(path.parent)
    )
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "wb") as handle:
            handle.write(payload)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
        directory_flags = getattr(os, "O_DIRECTORY", 0)
        if directory_flags:
            directory = os.open(path.parent, os.O_RDONLY | directory_flags)
            try:
                os.fsync(directory)
            finally:
                os.close(directory)
    finally:
        try:
            temporary.unlink()
        except FileNotFoundError:
            pass


def load_json_bytes(raw: bytes, label: str):
    try:
        value = json.loads(raw.decode("utf-8-sig"))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError("%s is not valid UTF-8 JSON: %s" % (label, error.msg)) from None
    if not isinstance(value, dict):
        raise ValueError("%s JSON root is not an object" % label)
    return value


def validate_static_bundle_preflight(preflight):
    if preflight.get("passed") is not True:
        raise ValueError("A29 static-bundle preflight did not pass")
    if preflight.get("verification_phase") != "pre_injection_deferred_allowed":
        raise ValueError("A29 static-bundle preflight phase mismatch")
    if preflight.get("token_join_status") != "deferred":
        raise ValueError("A29 static-bundle preflight did not retain the deferred token gate")
    bundle_id = preflight.get("bundle_id")
    product_modset = preflight.get("product_modset_sha256")
    if not nonzero_lower_hex_sha256(bundle_id) or not nonzero_lower_hex_sha256(
        product_modset
    ):
        raise ValueError("A29 static-bundle preflight identity is malformed")
    if not exact_int(preflight.get("artifact_count"), 1, (1 << 32) - 1):
        raise ValueError("A29 static-bundle preflight artifact count is malformed")
    return {
        "bundle_id": bundle_id,
        "product_modset_sha256": product_modset,
        "artifact_count": preflight["artifact_count"],
    }


def validate_prebuilt_attestation(attestation):
    if (
        attestation.get("schema") != "battle-plan.central_rust_prebuilt_artifacts.v1"
        or attestation.get("status")
        != "PASS_CENTRAL_RELEASE_PREBUILT_ARTIFACTS"
        or attestation.get("successful_exit") is not True
    ):
        raise ValueError("A29 central production-artifact attestation is not authoritative")
    central = attestation.get("central_validation")
    source = attestation.get("source_input")
    validated_workspaces = (
        central.get("validated_workspaces") if isinstance(central, dict) else None
    )
    if (
        not isinstance(central, dict)
        or central.get("mode") != "Release"
        or central.get("requested_workspace") not in ("All", "Extractor")
        or not isinstance(validated_workspaces, list)
        or "Extractor" not in validated_workspaces
        or central.get("workspace_package_selection") != ["--workspace"]
        or not isinstance(source, dict)
        or source.get("schema") != "battle-plan.production_rust_input_digest.v1"
        or not nonzero_lower_hex_sha256(source.get("sha256"))
        or not exact_int(source.get("file_count"), 1, (1 << 31) - 1)
    ):
        raise ValueError("A29 production-artifact attestation is not a full extractor Release")
    rows = attestation.get("artifacts")
    if not isinstance(rows, list):
        raise ValueError("A29 production-artifact attestation artifact roster is malformed")
    resolved = {}
    for role, filename in (
        ("recorder_host", "collector_record.exe"),
        ("collector_dll", "collector.dll"),
    ):
        matches = [row for row in rows if isinstance(row, dict) and row.get("role") == role]
        if len(matches) != 1:
            raise ValueError("A29 production-artifact role is missing or duplicated: %s" % role)
        row = matches[0]
        if (
            Path(str(row.get("path", ""))).name.lower() != filename
            or not exact_int(row.get("bytes"), 1, (1 << 63) - 1)
            or not nonzero_lower_hex_sha256(row.get("sha256"))
        ):
            raise ValueError("A29 production-artifact identity is malformed: %s" % role)
        resolved[role] = row["sha256"]
    resolved["source_input_sha256"] = source["sha256"]
    return resolved


def validate_manifest(manifest, preflight=None, production_artifacts=None):
    if manifest.get("schema") != "native_collector_session_v1":
        raise ValueError("capture manifest schema mismatch")
    if manifest.get("scheduled_domains") != ["tech_live"]:
        raise ValueError("capture manifest is not exact tech_live-only acquisition")
    if manifest.get("stream_format") != "compact":
        raise ValueError("capture manifest is not a compact stream")
    if manifest.get("collector_control_errors") != [] or manifest.get("parse_errors") != []:
        raise ValueError("capture manifest reports collector-control or parse errors")
    if manifest.get("ordering_errors") != []:
        raise ValueError("capture manifest reports ordering errors")
    for field in ("session_id", "capture_id"):
        value = manifest.get(field)
        if not isinstance(value, str) or CANONICAL_ID.fullmatch(value) is None:
            raise ValueError("capture manifest %s is absent or noncanonical" % field)
    if not exact_int(manifest.get("pid"), 1, (1 << 32) - 1):
        raise ValueError("capture manifest pid is malformed")
    modset = manifest.get("modset")
    if (
        not isinstance(modset, dict)
        or modset.get("resolved") is not True
        or modset.get("errors") != []
    ):
        raise ValueError("capture manifest modset/executable preflight is unresolved")
    game = modset.get("game")
    if (
        not isinstance(game, dict)
        or game.get("exe_sha256") != EXPECTED_EXECUTABLE_SHA256
    ):
        raise ValueError("capture manifest executable is not exact a729")
    compact = manifest.get("compact_stream")
    if not isinstance(compact, dict) or compact.get("decodable") is not True:
        raise ValueError("capture manifest compact stream is not decodable")
    if compact.get("expected_domains") != ["tech_live"]:
        raise ValueError("compact-stream expected-domain roster is not exact tech_live")
    domains_seen = compact.get("domains_seen")
    if not isinstance(domains_seen, list) or "tech_live" not in domains_seen:
        raise ValueError("compact stream did not observe tech_live")
    per_domain = compact.get("per_domain")
    tech = per_domain.get("tech_live") if isinstance(per_domain, dict) else None
    if not isinstance(tech, dict) or not exact_int(
        tech.get("data_record_count"), 1, (1 << 64) - 1
    ):
        raise ValueError("compact stream has no tech_live data record")
    hourly = manifest.get("hourly_requirement")
    if (
        not isinstance(hourly, dict)
        or hourly.get("required") is not True
        or hourly.get("met") is not True
        or hourly.get("problems") != []
        or not exact_int(hourly.get("requested_game_hours"), 12, (1 << 31) - 1)
    ):
        raise ValueError("capture manifest did not meet its twelve-hour requirement")
    if not exact_int(manifest.get("game_hours_advanced"), 12, (1 << 31) - 1):
        raise ValueError("compact stream did not prove twelve advancing game hours")
    if manifest.get("skipped_game_hours") != 0:
        raise ValueError("compact stream skipped one or more game hours")
    replay_artifact = manifest.get("replay_artifact")
    if (
        not isinstance(replay_artifact, dict)
        or replay_artifact.get("recovery") is not False
        or replay_artifact.get("reason") != "gate_passed"
        or replay_artifact.get("gate_problems") != []
        or replay_artifact.get("written_file") != manifest.get("replay_file")
    ):
        raise ValueError("capture manifest replay artifact is not the normal gate-passed file")
    launch = manifest.get("early_render_launch_manifest")
    early = manifest.get("early_render_capture_receipt")
    if not isinstance(launch, dict) or not isinstance(early, dict):
        raise ValueError("capture manifest lacks authenticated suspended early-launch evidence")
    if (
        launch.get("executable_sha256") != EXPECTED_EXECUTABLE_SHA256
        or early.get("executable_sha256") != EXPECTED_EXECUTABLE_SHA256
        or early.get("process_id") != manifest.get("pid")
        or early.get("created_suspended") is not True
        or not nonzero_lower_hex_sha256(launch.get("collector_sha256"))
        or early.get("collector_sha256") != launch.get("collector_sha256")
        or not exact_int(early.get("process_start_nonce"), 1, (1 << 64) - 1)
    ):
        raise ValueError("capture manifest early-launch identity is not exact a729")
    if (
        production_artifacts is not None
        and launch.get("collector_sha256") != production_artifacts["collector_dll"]
    ):
        raise ValueError("capture manifest collector is not the attested central Release DLL")
    completeness = early.get("completeness")
    if (
        not isinstance(completeness, dict)
        or not completeness
        or not all(value is True for value in completeness.values())
    ):
        raise ValueError("capture manifest early-launch hook completeness is not exact")
    if not nonzero_lower_hex_sha256(manifest.get("early_render_capture_receipt_sha256")):
        raise ValueError("capture manifest early-render receipt hash is malformed")
    static_bundle = manifest.get("static_bundle")
    if (
        not isinstance(static_bundle, dict)
        or static_bundle.get("admission") != "passed"
        or static_bundle.get("verification_phase") != "post_capture_finalized"
        or static_bundle.get("token_join_status") != "passed"
        or static_bundle.get("final_token_gate_satisfied") is not True
    ):
        raise ValueError("capture manifest lacks the finalized static-bundle admission")
    if preflight is not None:
        for field in ("bundle_id", "product_modset_sha256", "artifact_count"):
            if static_bundle.get(field) != preflight[field]:
                raise ValueError("preflight/final static-bundle %s mismatch" % field)
    capture_provenance = static_bundle.get("capture_provenance")
    stream_storage = manifest.get("stream_storage")
    if (
        not isinstance(capture_provenance, dict)
        or capture_provenance.get("session_id") != manifest.get("session_id")
        or capture_provenance.get("capture_id") != manifest.get("capture_id")
        or not isinstance(stream_storage, dict)
        or capture_provenance.get("stream_sha256")
        != stream_storage.get("decoded_sha256")
        or not nonzero_lower_hex_sha256(capture_provenance.get("stream_sha256"))
    ):
        raise ValueError("final static bundle is not bound to this session/capture stream")
    process_identity = static_bundle.get("process_identity")
    if (
        not isinstance(process_identity, dict)
        or process_identity.get("capture_process_id") != manifest.get("pid")
        or not exact_int(
            process_identity.get("capture_process_creation_filetime_utc"),
            1,
            (1 << 64) - 1,
        )
    ):
        raise ValueError("final static bundle process identity does not match the capture")


def validate_navigation_receipt(receipt, manifest):
    if receipt.get("schema") != "hoi4.a29.autonomous-research-navigation.v4":
        raise ValueError("A29 autonomous navigation receipt schema mismatch")
    if receipt.get("acquisition_owner") != "machine":
        raise ValueError("A29 navigation was not machine-owned")
    if (
        receipt.get("result")
        != "EXACT_D04_RESEARCH_TREE_HOVER_COMPLETE_A29_HOOK_VERDICT_REQUIRED"
    ):
        raise ValueError("A29 navigation did not complete the exact D04 route")
    if receipt.get("executable_sha256") != EXPECTED_EXECUTABLE_SHA256:
        raise ValueError("A29 navigation executable is not exact a729")
    if receipt.get("process_id") != manifest.get("pid"):
        raise ValueError("A29 navigation PID does not match the capture manifest")
    if (
        receipt.get("session_id") != manifest.get("session_id")
        or receipt.get("capture_id") != manifest.get("capture_id")
    ):
        raise ValueError("A29 navigation session/capture identity mismatch")
    if not isinstance(receipt.get("process_start_utc"), str) or not receipt[
        "process_start_utc"
    ]:
        raise ValueError("A29 navigation process-start identity is absent")
    if receipt.get("client_size") != [1920, 1080]:
        raise ValueError("A29 navigation did not use the frozen D04 1920x1080 profile")
    if (
        receipt.get("navigation_profile")
        != "New D04 Save Deep War/paused/1944.2.11.18/1920x1080"
    ):
        raise ValueError("A29 navigation profile is not the frozen paused-D04 route")
    if (
        receipt.get("navigation_policy")
        != "exact_d04_two_read_only_ui_clicks_then_future_node_hover_no_technology_click"
    ):
        raise ValueError("A29 navigation does not attest the no-tech-click policy")
    if receipt.get("promotion_authorized") is not False:
        raise ValueError("A29 navigation receipt improperly claims promotion authority")
    if (
        receipt.get("previously_observed_visual_diagnostic")
        != "Technology is 0.88 years ahead of time."
    ):
        raise ValueError("A29 navigation receipt lacks the observed positive diagnostic")
    for field, expected in EXPECTED_CLOCK_CONTRACT.items():
        if receipt.get(field) != expected:
            raise ValueError("A29 navigation exact clock contract mismatch: %s" % field)
    if (
        receipt.get("date_epoch_raw") != DATE_EPOCH_RAW
        or receipt.get("expected_d04_game_hour_raw") != EXPECTED_D04_GAME_HOUR_RAW
        or receipt.get("expected_d04_game_hour_normalized")
        != EXPECTED_D04_GAME_HOUR_NORMALIZED
        or receipt.get("game_hour_raw_before_navigation")
        != EXPECTED_D04_GAME_HOUR_RAW
        or receipt.get("game_hour_raw_after_navigation")
        != EXPECTED_D04_GAME_HOUR_RAW
        or receipt.get("game_hour_normalized_before_navigation")
        != EXPECTED_D04_GAME_HOUR_NORMALIZED
        or receipt.get("game_hour_normalized_after_navigation")
        != EXPECTED_D04_GAME_HOUR_NORMALIZED
        or not exact_int(receipt.get("game_speed_raw_before_navigation"), 0, 4)
        or receipt.get("game_speed_raw_after_navigation")
        != receipt.get("game_speed_raw_before_navigation")
        or receipt.get("pause_before_navigation") != 1
        or receipt.get("pause_after_navigation") != 1
    ):
        raise ValueError(
            "A29 navigation did not preserve the authenticated paused D04 raw/normalized hour"
        )
    steps = receipt.get("steps")
    if not isinstance(steps, list) or len(steps) != 3 or receipt.get("step_count") != 3:
        raise ValueError("A29 navigation does not contain exactly three frozen steps")
    expected_steps = (
        (
            "research_button_click",
            "open_research_topbar",
            [241, 56],
            "physical_setcursorpos_mouse_event",
            "settle_milliseconds",
            500,
        ),
        (
            "active_row_click",
            "open_active_engineer_company_iii",
            [195, 340],
            "physical_setcursorpos_mouse_event",
            "settle_milliseconds",
            500,
        ),
        (
            "future_node_hover",
            "hover_engineer_company_iv_1945",
            [1404, 302],
            "physical_setcursorpos_hover_only",
            "dwell_milliseconds",
            5000,
        ),
    )
    canonical = []
    for index, (step, expected) in enumerate(zip(steps, expected_steps)):
        if not isinstance(step, dict):
            raise ValueError("A29 navigation step %d is malformed" % index)
        action, label, client_point, method, delay_field, minimum_delay = expected
        if (
            step.get("action") != action
            or step.get("label") != label
            or step.get("client_point") != client_point
            or step.get("method") != method
        ):
            raise ValueError("A29 navigation step %d differs from the frozen route" % index)
        if (
            step.get("window_owner_process_id") != receipt.get("process_id")
            or step.get("foreground_owner_process_id") != receipt.get("process_id")
            or step.get("cursor_confirmed") is not True
        ):
            raise ValueError("A29 navigation step %d lacks PID/window/foreground guards" % index)
        screen_point = step.get("screen_point")
        if (
            not isinstance(screen_point, list)
            or len(screen_point) != 2
            or not all(exact_int(value, -32768, 32767) for value in screen_point)
        ):
            raise ValueError("A29 navigation step %d screen point is malformed" % index)
        delay = step.get(delay_field)
        maximum_delay = 30000 if delay_field == "dwell_milliseconds" else 10000
        if not exact_int(delay, minimum_delay, maximum_delay):
            raise ValueError("A29 navigation step %d delay is outside policy" % index)
        click_hold = step.get("click_hold_milliseconds", 0)
        if click_hold != (80 if index < 2 else 0):
            raise ValueError("A29 navigation step %d click-hold policy mismatch" % index)
        canonical.append(
            "%s|%s|%d|%d|%s|%d|%d"
            % (action, label, client_point[0], client_point[1], method, delay, click_hold)
        )
    if receipt.get("final_cursor_screen") != steps[-1].get("screen_point"):
        raise ValueError("A29 navigation final cursor does not retain the future-node hover")
    expected_sha256 = sha256_bytes(";".join(canonical).encode("utf-8"))
    if receipt.get("step_sequence_sha256") != expected_sha256:
        raise ValueError("A29 navigation step-sequence SHA-256 mismatch")
    return {
        "process_id": receipt["process_id"],
        "client_size": receipt["client_size"],
        "step_count": len(steps),
        "step_sequence_sha256": expected_sha256,
        "navigation_policy": receipt["navigation_policy"],
        "process_start_utc": receipt["process_start_utc"],
        "game_hour_raw": EXPECTED_D04_GAME_HOUR_RAW,
        "game_hour_normalized": EXPECTED_D04_GAME_HOUR_NORMALIZED,
    }


def validate_clock_transition_receipt(receipt, manifest, navigation, action):
    if receipt.get("schema") != "hoi4.shared.authenticated-clock-transition.v2":
        raise ValueError("A29 authenticated %s receipt schema mismatch" % action)
    if receipt.get("action") != action:
        raise ValueError("A29 clock receipt action is not %s" % action)
    if receipt.get("executable_sha256") != EXPECTED_EXECUTABLE_SHA256:
        raise ValueError("A29 %s executable is not exact a729" % action)
    if receipt.get("process_id") != manifest.get("pid"):
        raise ValueError("A29 %s PID does not match the capture manifest" % action)
    if (
        receipt.get("session_id") != manifest.get("session_id")
        or receipt.get("capture_id") != manifest.get("capture_id")
        or receipt.get("process_start_utc") != navigation.get("process_start_utc")
    ):
        raise ValueError("A29 %s session/process identity mismatch" % action)
    for field, expected in EXPECTED_CLOCK_CONTRACT.items():
        if receipt.get(field) != expected:
            raise ValueError("A29 %s exact clock contract mismatch: %s" % (action, field))
    before = receipt.get("game_hour_raw_before")
    after = receipt.get("game_hour_raw_after")
    normalized_before = receipt.get("game_hour_normalized_before")
    normalized_after = receipt.get("game_hour_normalized_after")
    if (
        receipt.get("date_epoch_raw") != DATE_EPOCH_RAW
        or receipt.get("expected_ready_game_hour_raw") != EXPECTED_D04_GAME_HOUR_RAW
        or receipt.get("expected_ready_game_hour_normalized")
        != EXPECTED_D04_GAME_HOUR_NORMALIZED
        or not exact_int(before, DATE_EPOCH_RAW + 1, (1 << 31) - 1)
        or not exact_int(after, DATE_EPOCH_RAW + 1, (1 << 31) - 1)
        or normalized_before != before - DATE_EPOCH_RAW
        or normalized_after != after - DATE_EPOCH_RAW
    ):
        raise ValueError("A29 %s raw/normalized clock hours are malformed" % action)
    if not exact_int(receipt.get("speed_raw"), 0, 4):
        raise ValueError("A29 %s raw game speed is malformed" % action)
    if action == "readiness":
        if (
            receipt.get("result") != "AUTHENTICATED_PAUSED_SAVE_READY"
            or receipt.get("pause_before") != 1
            or receipt.get("pause_after") != 1
            or before != EXPECTED_D04_GAME_HOUR_RAW
            or after != before
        ):
            raise ValueError("A29 readiness does not prove the exact paused D04 save")
    elif action == "unpause":
        if (
            receipt.get("result") != "AUTHENTICATED_PAUSED_TO_RUNNING_AND_ADVANCING"
            or receipt.get("pause_before") != 1
            or receipt.get("pause_after") != 0
            or before != EXPECTED_D04_GAME_HOUR_RAW
            or after <= before
        ):
            raise ValueError("A29 unpause does not prove D04 paused-to-running advancement")
    elif (
        receipt.get("result")
        not in ("AUTHENTICATED_RUNNING_TO_PAUSED", "AUTHENTICATED_ALREADY_PAUSED")
        or receipt.get("pause_before") not in (0, 1)
        or receipt.get("pause_after") != 1
        or after < before
        or (receipt.get("pause_before") == 0)
        != (receipt.get("result") == "AUTHENTICATED_RUNNING_TO_PAUSED")
    ):
        raise ValueError("A29 repause does not prove an authenticated paused cleanup state")
    return {
        "process_id": receipt["process_id"],
        "action": action,
        "result": receipt["result"],
        "game_hour_raw_before": before,
        "game_hour_raw_after": after,
        "game_hour_normalized_before": normalized_before,
        "game_hour_normalized_after": normalized_after,
        "speed_raw": receipt["speed_raw"],
    }


def snapshot_documents_from_replay(
    path: Path, external_manifest, preflight, production_artifacts
):
    try:
        archive = zipfile.ZipFile(path, "r")
    except (OSError, zipfile.BadZipFile):
        raise ValueError("replay is not a readable .battleplan ZIP container") from None
    with archive:
        embedded_infos = [
            info
            for info in archive.infolist()
            if not info.is_dir()
            and info.filename == "recorder/native_session_manifest.json"
        ]
        if len(embedded_infos) != 1:
            raise ValueError("replay does not contain exactly one recorder manifest")
        try:
            embedded_raw = archive.read(embedded_infos[0])
        except (OSError, RuntimeError, zipfile.BadZipFile):
            raise ValueError("embedded recorder manifest is unreadable") from None
        embedded_manifest = load_json_bytes(
            embedded_raw, "recorder/native_session_manifest.json"
        )
        validate_manifest(embedded_manifest, preflight, production_artifacts)
        if not exact_int(external_manifest.get("pid"), 1, (1 << 32) - 1):
            raise ValueError("external capture manifest pid is malformed")
        if (
            not isinstance(external_manifest.get("session_id"), str)
            or not external_manifest["session_id"]
        ):
            raise ValueError("external capture manifest session_id is absent")
        for field in (
            "schema",
            "session_id",
            "capture_id",
            "pid",
            "replay_game_id",
            "dll",
            "duration_ms",
            "sample_ms",
            "stream_format",
            "scheduled_domains",
            "raw_stream",
        ):
            if embedded_manifest.get(field) != external_manifest.get(field):
                raise ValueError(
                    "embedded and external capture manifests differ at %s" % field
                )
        infos = [
            info
            for info in archive.infolist()
            if not info.is_dir() and info.filename.endswith(".snapshot.json")
        ]
        names = [info.filename for info in infos]
        if len(names) != len(set(names)):
            raise ValueError("replay contains duplicate snapshot member names")
        if not infos:
            raise ValueError("replay contains no folded snapshot JSON members")
        if len(infos) > MAX_SNAPSHOT_COUNT:
            raise ValueError("replay exceeds the bounded snapshot-count limit")
        documents = []
        for info in sorted(infos, key=lambda item: item.filename):
            if info.file_size > MAX_SNAPSHOT_BYTES:
                raise ValueError("replay snapshot exceeds the bounded size limit")
            try:
                raw = archive.read(info)
            except (OSError, RuntimeError, zipfile.BadZipFile):
                raise ValueError(
                    "replay snapshot member is unreadable: %s" % info.filename
                ) from None
            documents.append(
                {
                    "source_kind": "replay_snapshot",
                    "source_name": info.filename,
                    "sha256": sha256_bytes(raw),
                    "document": load_json_bytes(raw, info.filename),
                }
            )
        return documents, sha256_bytes(embedded_raw)


def snapshot_document_from_file(path: Path):
    try:
        raw = path.read_bytes()
    except OSError:
        raise ValueError("snapshot input is unreadable") from None
    if len(raw) > MAX_SNAPSHOT_BYTES:
        raise ValueError("snapshot input exceeds the bounded size limit")
    return {
        "source_kind": "snapshot",
        "source_name": path.name,
        "sha256": sha256_bytes(raw),
        "document": load_json_bytes(raw, path.name),
    }


def decimal_matches_raw(value, raw):
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return False
    try:
        decimal = Decimal(str(value))
    except InvalidOperation:
        return False
    return decimal.is_finite() and decimal * EXPECTED_FIXED_SCALE == raw


def expected_source_record_sha256(
    provenance, research_slot_index, tech_def_index_raw, raw_values
):
    canonical = bytearray(b"the game:a729:a29-observed-producer:selected-present:v3\0")
    canonical.extend(EXPECTED_EXECUTABLE_SHA256.encode("ascii"))
    canonical.extend(EXPECTED_ROOT_MANIFEST_SHA256.encode("ascii"))
    canonical.extend(EXPECTED_SENTINEL_DUMP_SHA256.encode("ascii"))
    canonical.extend(EXPECTED_ACTION_GATE_MANIFEST_SHA256.encode("ascii"))
    canonical.extend(EXPECTED_ACTION_GATE_SENTINEL_DUMP_SHA256.encode("ascii"))
    canonical.extend(struct.pack("<Q", EXPECTED_UI_YEAR_HELPER_RETURN_RVA))
    canonical.extend(struct.pack("<Q", provenance["callback_sequence"]))
    canonical.extend(struct.pack("<I", provenance["callback_thread_id"]))
    canonical.extend(struct.pack("<Q", provenance["callback_qpc_ticks"]))
    canonical.extend(struct.pack("<Q", provenance["qpc_frequency_hz"]))
    canonical.extend(struct.pack("<i", provenance["producer_game_hour_raw"]))
    canonical.extend(struct.pack("<I", research_slot_index))
    canonical.extend(struct.pack("<I", tech_def_index_raw))
    canonical.extend(struct.pack("<I", provenance["localization_branch"]))
    for field, _ in VALUE_FIELDS:
        canonical.extend(struct.pack("<q", raw_values[field]))
    canonical.extend(struct.pack("<I", provenance["selected_present_candidate_count"]))
    canonical.extend(struct.pack("<Q", provenance["selected_present_candidate_id"]))
    canonical.extend(struct.pack("<I", provenance["selected_present_ordinal"]))
    canonical.extend(struct.pack("<i", provenance["selected_present_game_hour_raw"]))
    canonical.extend(struct.pack("<Q", provenance["selected_present_qpc_ticks"]))
    canonical.extend(struct.pack("<Q", provenance["selected_present_cutoff_qpc_ticks"]))
    canonical.extend(provenance["selected_present_candidate_set_sha256"].encode("ascii"))
    canonical.extend(provenance["selected_present_receipt_sha256"].encode("ascii"))
    return hashlib.sha256(canonical).hexdigest()


def selected_present_receipt_sha256(provenance):
    receipt = {
        "candidate_count": provenance["selected_present_candidate_count"],
        "candidate_set_sha256": provenance["selected_present_candidate_set_sha256"],
        "selected_candidate_id": str(provenance["selected_present_candidate_id"]),
        "selected_ordinal": provenance["selected_present_ordinal"],
        "selected_game_hour_raw": provenance["selected_present_game_hour_raw"],
        "selected_qpc_ticks": str(provenance["selected_present_qpc_ticks"]),
        "selection_cutoff_qpc_ticks": str(
            provenance["selected_present_cutoff_qpc_ticks"]
        ),
    }
    return sha256_bytes(
        json.dumps(receipt, separators=(",", ":"), ensure_ascii=False).encode("utf-8")
    )


def validate_provenance(
    provenance, snapshot, research_slot_index, tech_def_index_raw, raw_values
):
    if not isinstance(provenance, dict):
        raise ValueError("ahead_of_time_provenance is absent or not an object")
    exact_text = {
        "authority": EXPECTED_AUTHORITY,
        "executable_sha256": EXPECTED_EXECUTABLE_SHA256,
        "exact_root_job_id": EXPECTED_ROOT_JOB_ID,
        "exact_root_manifest_sha256": EXPECTED_ROOT_MANIFEST_SHA256,
        "sentinel_job_id": EXPECTED_SENTINEL_JOB_ID,
        "sentinel_dump_sha256": EXPECTED_SENTINEL_DUMP_SHA256,
        "action_gate_job_id": EXPECTED_ACTION_GATE_JOB_ID,
        "action_gate_manifest_sha256": EXPECTED_ACTION_GATE_MANIFEST_SHA256,
        "action_gate_sentinel_job_id": EXPECTED_ACTION_GATE_SENTINEL_JOB_ID,
        "action_gate_sentinel_dump_sha256": EXPECTED_ACTION_GATE_SENTINEL_DUMP_SHA256,
    }
    for field, expected in exact_text.items():
        if provenance.get(field) != expected:
            raise ValueError("A29 provenance %s mismatch" % field)
    exact_rvas = {
        "ui_year_producer_rva": EXPECTED_UI_YEAR_PRODUCER_RVA,
        "ui_year_helper_rva": EXPECTED_UI_YEAR_HELPER_RVA,
        "ui_year_action_identity_compare_rva": EXPECTED_UI_YEAR_ACTION_IDENTITY_COMPARE_RVA,
        "ui_year_helper_return_rva": EXPECTED_UI_YEAR_HELPER_RETURN_RVA,
        "effective_value_producer_rva": EXPECTED_EFFECTIVE_VALUE_PRODUCER_RVA,
        "localization_consumer_rva": EXPECTED_LOCALIZATION_CONSUMER_RVA,
    }
    for field, expected in exact_rvas.items():
        if provenance.get(field) != expected:
            raise ValueError("A29 provenance %s mismatch" % field)
    for field, maximum in (
        ("callback_sequence", (1 << 64) - 1),
        ("callback_thread_id", (1 << 32) - 1),
        ("callback_qpc_ticks", (1 << 64) - 1),
        ("qpc_frequency_hz", (1 << 64) - 1),
        ("producer_game_hour_raw", (1 << 31) - 1),
    ):
        if not exact_int(provenance.get(field), 1, maximum):
            raise ValueError("A29 provenance %s is not a positive exact integer" % field)
    if not exact_int(provenance.get("localization_branch"), 0, 2):
        raise ValueError("A29 provenance localization_branch is outside 0..2")
    candidate_count = provenance.get("selected_present_candidate_count")
    selected_candidate_id = provenance.get("selected_present_candidate_id")
    selected_ordinal = provenance.get("selected_present_ordinal")
    selected_hour = provenance.get("selected_present_game_hour_raw")
    selected_qpc = provenance.get("selected_present_qpc_ticks")
    cutoff_qpc = provenance.get("selected_present_cutoff_qpc_ticks")
    if not exact_int(candidate_count, 1, (1 << 32) - 1):
        raise ValueError("A29 selected-Present candidate count is malformed")
    if not exact_int(selected_candidate_id, 0, (1 << 64) - 1):
        raise ValueError("A29 selected-Present candidate identity is malformed")
    if not exact_int(selected_ordinal, 0, candidate_count - 1):
        raise ValueError("A29 selected-Present ordinal is outside the candidate set")
    if selected_hour != provenance["producer_game_hour_raw"]:
        raise ValueError("A29 selected-Present hour differs from its producer hour")
    if (
        not exact_int(selected_qpc, 1, (1 << 64) - 1)
        or not exact_int(cutoff_qpc, selected_qpc, (1 << 64) - 1)
        or provenance["callback_qpc_ticks"] > selected_qpc
    ):
        raise ValueError("A29 selected-Present callback/selection QPC ordering is invalid")
    candidate_set_sha = provenance.get("selected_present_candidate_set_sha256")
    selected_receipt_sha = provenance.get("selected_present_receipt_sha256")
    if not nonzero_lower_hex_sha256(candidate_set_sha) or not nonzero_lower_hex_sha256(
        selected_receipt_sha
    ):
        raise ValueError("A29 selected-Present hash identity is malformed")
    if selected_receipt_sha != selected_present_receipt_sha256(provenance):
        raise ValueError("A29 selected-Present receipt SHA-256 is not canonically sealed")
    source_sha = provenance.get("source_record_sha256")
    if not nonzero_lower_hex_sha256(source_sha):
        raise ValueError("A29 source-record SHA-256 is absent, non-lowerhex, or zero")
    if source_sha != expected_source_record_sha256(
        provenance, research_slot_index, tech_def_index_raw, raw_values
    ):
        raise ValueError("A29 source-record SHA-256 does not bind the portable producer row")
    snapshot_hour = snapshot.get("game_hour_raw")
    if snapshot_hour is not None:
        if not exact_int(snapshot_hour, 1, (1 << 31) - 1):
            raise ValueError("snapshot game_hour_raw is not a positive signed i32")
        if provenance["producer_game_hour_raw"] != snapshot_hour:
            raise ValueError("A29 producer hour differs from snapshot game_hour_raw")
    return {
        "authority": provenance["authority"],
        "callback_sequence": provenance["callback_sequence"],
        "callback_thread_id": provenance["callback_thread_id"],
        "producer_game_hour_raw": provenance["producer_game_hour_raw"],
        "localization_branch": provenance.get("localization_branch"),
        "selected_present_candidate_count": candidate_count,
        "selected_present_candidate_id": selected_candidate_id,
        "selected_present_ordinal": selected_ordinal,
        "selected_present_receipt_sha256": selected_receipt_sha,
        "source_record_sha256": source_sha,
    }


def validate_a29_row(row, snapshot):
    slot_index = row.get("slot_index")
    tech_handle_raw = row.get("tech_handle_raw")
    tech_def_index_raw = row.get("tech_def_index_raw")
    tech_name = row.get("tech_name")
    if not exact_int(slot_index, 0, (1 << 32) - 1):
        raise ValueError("slot_index is not a u32")
    if (
        not exact_int(tech_handle_raw, 0, (1 << 32) - 1)
        or not exact_int(tech_def_index_raw, 0, (1 << 32) - 1)
        or tech_handle_raw != tech_def_index_raw
    ):
        raise ValueError("active technology handle/definition identity is malformed")
    if tech_name is not None and (
        not isinstance(tech_name, str) or not tech_name.strip()
    ):
        raise ValueError("active technology name is present but malformed")
    if row.get("ahead_of_time_fixed_scale") != EXPECTED_FIXED_SCALE:
        raise ValueError("ahead-of-time fixed scale is not 100000")
    raw_values = {}
    values = {}
    for raw_field, value_field in VALUE_FIELDS:
        raw = row.get(raw_field)
        value = row.get(value_field)
        if not exact_int(raw, 0, 10_000_000_000):
            raise ValueError("%s is outside the admitted fixed-five range" % raw_field)
        if not decimal_matches_raw(value, raw):
            raise ValueError("%s is not exactly %s / 100000" % (value_field, raw_field))
        raw_values[raw_field] = raw
        values[value_field] = value
    provenance = validate_provenance(
        row.get("ahead_of_time_provenance"),
        snapshot,
        slot_index,
        tech_def_index_raw,
        raw_values,
    )
    branch = provenance["localization_branch"]
    penalty = raw_values["ahead_of_time_penalty_raw"]
    reduction = raw_values["ahead_of_time_reduction_years_raw"]
    reduced = raw_values["ahead_of_time_reduced_penalty_raw"]
    if branch == 1:
        if reduction != 0 or reduced != penalty:
            raise ValueError(
                "unreduced localization branch does not carry zero reduction and VALUE equality"
            )
    elif branch == 2:
        if reduction <= 0:
            raise ValueError(
                "reduced localization branch does not carry a positive observed reduction"
            )
    else:
        raise ValueError("A29 localization branch is not exact branch 1 or 2")
    return {
        "slot_index": slot_index,
        "tech_handle_raw": tech_handle_raw,
        "tech_def_index_raw": tech_def_index_raw,
        "tech_name": tech_name,
        "raw_values": raw_values,
        "values": values,
        "provenance": provenance,
    }


def inspect_snapshots(snapshots):
    accepted = []
    rejected = []
    snapshot_count = 0
    for source in snapshots:
        snapshot_count += 1
        snapshot = source["document"]
        countries = snapshot.get("countries")
        if countries is None:
            continue
        if not isinstance(countries, dict):
            rejected.append("%s: countries is not an object" % source["source_name"])
            continue
        for country_key in sorted(countries):
            country = countries[country_key]
            research = country.get("research") if isinstance(country, dict) else None
            rows = research.get("in_progress_live") if isinstance(research, dict) else None
            if rows is None:
                continue
            if not isinstance(rows, list):
                rejected.append(
                    "%s:%s: research.in_progress_live is not a list"
                    % (source["source_name"], country_key)
                )
                continue
            for index, row in enumerate(rows):
                if not isinstance(row, dict):
                    rejected.append(
                        "%s:%s:%d: active research row is not an object"
                        % (source["source_name"], country_key, index)
                    )
                    continue
                if not any(field in row for field in A29_FIELDS):
                    continue
                label = "%s:%s:%d" % (source["source_name"], country_key, index)
                try:
                    witness = validate_a29_row(row, snapshot)
                    witness.update(
                        {
                            "country_key": country_key,
                            "snapshot_member": source["source_name"],
                            "snapshot_sha256": source["sha256"],
                        }
                    )
                    accepted.append(witness)
                except ValueError as error:
                    rejected.append("%s: %s" % (label, error))
    if rejected:
        raise ValueError("malformed A29-bearing row: %s" % rejected[0])
    if not accepted:
        raise ValueError(
            "no authenticated A29 row found; open an active research technology "
            "year/ahead tooltip and keep it visible through a same-hour tech_live poll"
        )
    accepted.sort(
        key=lambda row: (
            row["snapshot_member"],
            row["country_key"],
            row["slot_index"],
            row["provenance"]["callback_sequence"],
        )
    )
    return snapshot_count, accepted


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture-manifest", required=True)
    parser.add_argument("--prebuilt-attestation", required=True)
    parser.add_argument("--static-bundle-preflight", required=True)
    parser.add_argument("--navigation-receipt", required=True)
    parser.add_argument("--readiness-receipt", required=True)
    parser.add_argument("--unpause-receipt", required=True)
    parser.add_argument("--repause-receipt", required=True)
    parser.add_argument("--replay")
    parser.add_argument("--snapshot", action="append", default=[])
    parser.add_argument("--out", required=True)
    args = parser.parse_args()

    out_path = Path(args.out).resolve()
    manifest_path = Path(args.capture_manifest).resolve()
    attestation_path = Path(args.prebuilt_attestation).resolve()
    preflight_path = Path(args.static_bundle_preflight).resolve()
    navigation_path = Path(args.navigation_receipt).resolve()
    readiness_path = Path(args.readiness_receipt).resolve()
    unpause_path = Path(args.unpause_receipt).resolve()
    repause_path = Path(args.repause_receipt).resolve()
    replay_path = Path(args.replay).resolve() if args.replay else None
    snapshot_paths = [Path(value).resolve() for value in args.snapshot]
    errors = []
    details = {}
    inputs = []
    try:
        if replay_path is None and not snapshot_paths:
            raise ValueError("at least one --replay or --snapshot input is required")
        try:
            attestation_raw = attestation_path.read_bytes()
        except OSError:
            raise ValueError("central production-artifact attestation is unreadable") from None
        attestation = load_json_bytes(
            attestation_raw, "central production-artifact attestation"
        )
        artifact_details = validate_prebuilt_attestation(attestation)
        inputs.append(
            {
                "kind": "central_release_artifact_attestation",
                "sha256": sha256_bytes(attestation_raw),
            }
        )
        try:
            preflight_raw = preflight_path.read_bytes()
        except OSError:
            raise ValueError("static-bundle preflight receipt is unreadable") from None
        preflight = load_json_bytes(preflight_raw, "static-bundle preflight receipt")
        preflight_details = validate_static_bundle_preflight(preflight)
        inputs.append(
            {"kind": "static_bundle_preflight", "sha256": sha256_bytes(preflight_raw)}
        )
        try:
            manifest_raw = manifest_path.read_bytes()
        except OSError:
            raise ValueError("capture manifest is unreadable") from None
        inputs.append(
            {
                "kind": "capture_manifest",
                "sha256": sha256_bytes(manifest_raw),
            }
        )
        manifest = load_json_bytes(manifest_raw, "capture manifest")
        validate_manifest(manifest, preflight_details, artifact_details)
        try:
            navigation_raw = navigation_path.read_bytes()
        except OSError:
            raise ValueError("A29 autonomous navigation receipt is unreadable") from None
        navigation = load_json_bytes(navigation_raw, "A29 autonomous navigation receipt")
        navigation_details = validate_navigation_receipt(navigation, manifest)
        inputs.append(
            {
                "kind": "autonomous_navigation_receipt",
                "sha256": sha256_bytes(navigation_raw),
            }
        )
        try:
            readiness_raw = readiness_path.read_bytes()
        except OSError:
            raise ValueError("A29 authenticated readiness receipt is unreadable") from None
        readiness = load_json_bytes(readiness_raw, "A29 authenticated readiness receipt")
        readiness_details = validate_clock_transition_receipt(
            readiness, manifest, navigation_details, "readiness"
        )
        inputs.append(
            {
                "kind": "authenticated_paused_save_readiness_receipt",
                "sha256": sha256_bytes(readiness_raw),
            }
        )
        try:
            unpause_raw = unpause_path.read_bytes()
        except OSError:
            raise ValueError("A29 authenticated unpause receipt is unreadable") from None
        unpause = load_json_bytes(unpause_raw, "A29 authenticated unpause receipt")
        unpause_details = validate_clock_transition_receipt(
            unpause, manifest, navigation_details, "unpause"
        )
        inputs.append(
            {
                "kind": "authenticated_unpause_receipt",
                "sha256": sha256_bytes(unpause_raw),
            }
        )
        try:
            repause_raw = repause_path.read_bytes()
        except OSError:
            raise ValueError("A29 authenticated repause receipt is unreadable") from None
        repause = load_json_bytes(repause_raw, "A29 authenticated repause receipt")
        repause_details = validate_clock_transition_receipt(
            repause, manifest, navigation_details, "repause"
        )
        if (
            repause_details["game_hour_raw_after"]
            < unpause_details["game_hour_raw_after"]
        ):
            raise ValueError("A29 repause receipt predates the authenticated unpause proof")
        inputs.append(
            {
                "kind": "authenticated_repause_receipt",
                "sha256": sha256_bytes(repause_raw),
            }
        )
        snapshots = []
        if replay_path is not None:
            if not replay_path.is_file():
                raise ValueError("replay input is absent or not a file")
            inputs.append({"kind": "replay", "sha256": sha256_file(replay_path)})
            replay_snapshots, embedded_manifest_sha256 = (
                snapshot_documents_from_replay(
                    replay_path, manifest, preflight_details, artifact_details
                )
            )
            snapshots.extend(replay_snapshots)
            inputs.append(
                {
                    "kind": "embedded_capture_manifest",
                    "sha256": embedded_manifest_sha256,
                }
            )
        for path in snapshot_paths:
            snapshots.append(snapshot_document_from_file(path))
            inputs.append({"kind": "snapshot", "sha256": sha256_file(path)})
        snapshot_count, witnesses = inspect_snapshots(snapshots)
        details = {
            "snapshot_count_inspected": snapshot_count,
            "authenticated_witness_count": len(witnesses),
            "witnesses": witnesses,
            "autonomous_navigation": navigation_details,
            "authenticated_paused_save_readiness": readiness_details,
            "authenticated_unpause": unpause_details,
            "authenticated_repause": repause_details,
            "static_bundle": preflight_details,
            "production_artifacts": artifact_details,
        }
    except (OSError, ValueError, zipfile.BadZipFile) as error:
        errors.append(str(error))
    except Exception as error:
        # A malformed or hostile evidence bundle must still produce a stable
        # FAIL receipt instead of accidentally becoming an ungraded artifact.
        errors.append("verifier rejected input: %s" % type(error).__name__)

    accepted = not errors
    receipt = {
        "schema": "hoi4.a29.research_ahead_fixture_verification.v2",
        "status": "PASS" if accepted else "FAIL",
        "production_admission_ready": accepted,
        "checks": {
            "exact_a729_tech_live_capture": accepted,
            "observed_localization_provenance": accepted,
            "same_hour_producer_join": accepted,
            "fixed_five_raw_value_equality": accepted,
            "localization_branch_semantics": accepted,
            "pointer_free_source_hash": accepted,
            "autonomous_navigation_receipt": accepted,
            "authenticated_paused_save_readiness": accepted,
            "authenticated_clock_unpause": accepted,
            "authenticated_clock_repause": accepted,
            "static_bundle_preflight_finalized_join": accepted,
            "selected_present_v3_action_gate": accepted,
            "session_capture_process_binding": accepted,
            "central_release_artifact_attestation": accepted,
        },
        "details": details,
        "errors": errors,
        "inputs": inputs,
    }
    write_json_atomic(out_path, receipt)
    print("A29 fixture %s: %s" % (receipt["status"], out_path))
    return 0 if accepted else 1


if __name__ == "__main__":
    raise SystemExit(main())
