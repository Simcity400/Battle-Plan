#!/usr/bin/env python3
"""Independently admit one exact-a729 same-launch A32 pointer witness."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import struct
import tempfile
import zipfile


EXE_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
AUTHORITY_SHA256 = "39b71a6fb75e53e31136e1909452b85106baf83addeb78da92901113cd98f0b0"
D04_HOUR_RAW = 60_830_441
DOMAIN_SEPARATOR = b"hoi4.a729.recorded-input-variant-observation.v3\0"


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def exact_int(value, minimum: int, maximum: int) -> bool:
    return isinstance(value, int) and not isinstance(value, bool) and minimum <= value <= maximum


def uint(value, bits: int, label: str) -> int:
    if isinstance(value, str) and value.isascii() and value.isdecimal():
        value = int(value)
    require(exact_int(value, 0, (1 << bits) - 1), f"{label} is not a u{bits}")
    return value


def lower_sha(value) -> bool:
    return (
        isinstance(value, str)
        and len(value) == 64
        and all(character in "0123456789abcdef" for character in value)
        and any(character != "0" for character in value)
    )


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def load_json(path: Path, label: str):
    try:
        value = json.loads(path.read_text(encoding="utf-8-sig"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError(f"{label} is not readable UTF-8 JSON: {error}") from None
    require(isinstance(value, dict), f"{label} root is not an object")
    return value


def write_create_new(path: Path, value: dict) -> None:
    path = path.resolve()
    require(not path.exists(), f"immutable admission already exists: {path}")
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
        os.link(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def validate_manifest(value: dict) -> dict:
    require(value.get("schema") == "native_collector_session_v1", "capture manifest schema mismatch")
    pid = value.get("pid")
    require(exact_int(pid, 1, (1 << 32) - 1), "capture PID is malformed")
    for key in ("session_id", "capture_id"):
        require(isinstance(value.get(key), str) and value[key].strip(), f"capture {key} is absent")
    require(value.get("stream_format") == "compact", "capture is not compact")
    for key in ("collector_control_errors", "parse_errors", "ordering_errors"):
        require(value.get(key) == [], f"capture reports {key}")
    require(exact_int(value.get("game_hours_advanced"), 1, 24), "capture did not advance a bounded game-hour window")
    modset = value.get("modset")
    game = modset.get("game") if isinstance(modset, dict) else None
    require(
        isinstance(modset, dict)
        and modset.get("resolved") is True
        and modset.get("errors") == []
        and isinstance(game, dict)
        and game.get("exe_sha256") == EXE_SHA256,
        "capture modset/executable authority is unresolved",
    )
    early = value.get("early_render_capture_receipt")
    require(
        isinstance(early, dict)
        and early.get("process_id") == pid
        and early.get("executable_sha256") == EXE_SHA256
        and early.get("created_suspended") is True,
        "capture lacks exact suspended early-launch identity",
    )
    completeness = early.get("completeness")
    require(isinstance(completeness, dict) and completeness and all(completeness.values()), "early hook completeness is not exact")
    static = value.get("static_bundle")
    require(
        isinstance(static, dict)
        and static.get("admission") == "passed"
        and static.get("verification_phase") == "post_capture_finalized"
        and static.get("token_join_status") == "passed"
        and static.get("final_token_gate_satisfied") is True,
        "capture lacks finalized static-bundle admission",
    )
    return {
        "pid": pid,
        "session_id": value["session_id"],
        "capture_id": value["capture_id"],
        "process_start_nonce": uint(early.get("process_start_nonce"), 64, "process_start_nonce"),
        "product_modset_sha256": static.get("product_modset_sha256"),
    }


def validate_attestation(value: dict, manifest: dict) -> dict:
    require(
        value.get("schema") == "battle-plan.central_rust_prebuilt_artifacts.v1"
        and value.get("status") == "PASS_CENTRAL_RELEASE_PREBUILT_ARTIFACTS"
        and value.get("successful_exit") is True,
        "central Release artifact attestation is not authoritative",
    )
    central = value.get("central_validation")
    source = value.get("source_input")
    require(
        isinstance(central, dict)
        and central.get("mode") == "Release"
        and central.get("requested_workspace") in ("All", "Extractor")
        and "Extractor" in central.get("validated_workspaces", [])
        and central.get("workspace_package_selection") == ["--workspace"]
        and isinstance(source, dict)
        and source.get("schema") == "battle-plan.production_rust_input_digest.v1"
        and lower_sha(source.get("sha256")),
        "central attestation is not a full extractor Release over the frozen source input",
    )
    rows = value.get("artifacts")
    require(isinstance(rows, list), "central artifact roster is malformed")
    result = {}
    for role in ("recorder_host", "collector_dll"):
        matches = [row for row in rows if isinstance(row, dict) and row.get("role") == role]
        require(len(matches) == 1 and lower_sha(matches[0].get("sha256")), f"central artifact role is invalid: {role}")
        result[role] = matches[0]["sha256"]
    early = manifest.get("early_render_capture_receipt")
    launch = manifest.get("early_render_launch_manifest")
    require(
        isinstance(early, dict)
        and isinstance(launch, dict)
        and early.get("collector_sha256") == result["collector_dll"]
        and launch.get("collector_sha256") == result["collector_dll"],
        "capture collector is not the centrally attested artifact",
    )
    return result


def validate_preflight(value: dict, manifest: dict) -> None:
    require(
        value.get("passed") is True
        and value.get("verification_phase") == "pre_injection_deferred_allowed"
        and value.get("token_join_status") == "deferred",
        "static-bundle preflight is not authoritative",
    )
    static = manifest.get("static_bundle")
    for key in ("bundle_id", "product_modset_sha256", "artifact_count"):
        require(value.get(key) == static.get(key), f"preflight/final static-bundle mismatch: {key}")


def validate_action(value: dict, identity: dict) -> dict:
    require(
        value.get("schema") == "hoi4.a32.authenticated-pointer-movement-actions.v1"
        and value.get("status") == "PASS",
        "A32 machine-action receipt is not PASS v1",
    )
    for key in ("session_id", "capture_id", "process_id"):
        require(value.get(key) == identity[key if key != "process_id" else "pid"], f"A32 action {key} mismatch")
    require(value.get("executable_sha256") == EXE_SHA256, "A32 action executable mismatch")
    require(value.get("foreground_owner_process_id") == identity["pid"], "A32 action foreground owner mismatch")
    extent = value.get("client_extent")
    require(
        isinstance(extent, list)
        and len(extent) == 2
        and all(exact_int(item, 1, 32768) for item in extent),
        "A32 action client extent is malformed",
    )
    target_rows = value.get("target_positions")
    require(
        isinstance(target_rows, list)
        and len(target_rows) == 2
        and all(
            isinstance(row, dict)
            and set(row) == {"x", "y"}
            and exact_int(row["x"], 0, extent[0] - 1)
            and exact_int(row["y"], 0, extent[1] - 1)
            for row in target_rows
        ),
        "A32 action target set is malformed",
    )
    targets = [[row["x"], row["y"]] for row in target_rows]
    require(targets[0] != targets[1], "A32 action targets are not distinct")
    for point in targets:
        require(
            exact_int(point[0], 0, extent[0] - 1) and exact_int(point[1], 0, extent[1] - 1),
            "A32 action target lies outside the client",
        )
    started = uint(value.get("action_started_qpc_ticks"), 64, "action_started_qpc_ticks")
    completed = uint(value.get("action_completed_qpc_ticks"), 64, "action_completed_qpc_ticks")
    require(started < completed, "A32 action QPC interval is empty")
    reset_pid = value.get("foreign_foreground_reset_process_id")
    reset_started = uint(value.get("foreign_foreground_reset_started_qpc_ticks"), 64, "foreign reset started QPC")
    reset_completed = uint(value.get("foreign_foreground_reset_completed_qpc_ticks"), 64, "foreign reset completed QPC")
    require(
        exact_int(reset_pid, 1, (1 << 32) - 1)
        and reset_pid != identity["pid"]
        and started <= reset_started < reset_completed <= completed,
        "A32 foreign-foreground baseline reset is invalid",
    )
    actions = value.get("actions")
    require(
        isinstance(actions, list)
        and len(actions) == value.get("action_count")
        and len(actions) == 2,
        "A32 action roster is incomplete",
    )
    previous = started
    for ordinal, row in enumerate(actions):
        require(isinstance(row, dict) and row.get("ordinal") == ordinal, "A32 action ordinals are not contiguous")
        require(row.get("target_index") == ordinal, "A32 action target order is invalid")
        require(row.get("client_position") == targets[ordinal], "A32 observed action position differs from target")
        move_start = uint(row.get("move_started_qpc_ticks"), 64, "move_started_qpc_ticks")
        move_end = uint(row.get("move_completed_qpc_ticks"), 64, "move_completed_qpc_ticks")
        require(previous <= move_start <= move_end <= completed, "A32 action QPC chronology is invalid")
        previous = move_end
    return {
        "extent": extent,
        "targets": targets,
        "started_qpc": started,
        "completed_qpc": completed,
        "process_start_utc": value.get("process_start_utc"),
        "action_count": len(actions),
        "reset_started_qpc": reset_started,
        "reset_completed_qpc": reset_completed,
    }


def validate_action_ready(value: dict, identity: dict, action: dict) -> None:
    require(
        value.get("schema") == "hoi4.a32.pointer-current-position-ready.v1"
        and value.get("status") == "PASS"
        and value.get("session_id") == identity["session_id"]
        and value.get("capture_id") == identity["capture_id"]
        and value.get("process_id") == identity["pid"]
        and value.get("process_start_utc") == action["process_start_utc"]
        and value.get("executable_sha256") == EXE_SHA256
        and value.get("client_extent") == action["extent"]
        and value.get("baseline_client_position") == action["targets"][0]
        and value.get("current_client_position") == action["targets"][1],
        "A32 current-position readiness does not bind the action pair",
    )
    reset_start = uint(value.get("reset_started_qpc_ticks"), 64, "reset-start QPC")
    reset_end = uint(value.get("reset_completed_qpc_ticks"), 64, "reset-end QPC")
    current_end = uint(value.get("current_position_completed_qpc_ticks"), 64, "current-position QPC")
    require(
        reset_start == action["reset_started_qpc"]
        and reset_end == action["reset_completed_qpc"]
        and action["started_qpc"] <= reset_start < reset_end < current_end <= action["completed_qpc"],
        "A32 reset/current-position readiness chronology is invalid",
    )


def validate_readiness(value: dict, identity: dict, action: dict) -> None:
    require(value.get("schema") == "hoi4-rendering-law-paused-save-readiness-v1", "readiness schema mismatch")
    require(
        value.get("process_id") == identity["pid"]
        and value.get("executable_sha256") == EXE_SHA256
        and value.get("process_start_utc") == action["process_start_utc"]
        and value.get("paused") is True
        and value.get("game_hour") == D04_HOUR_RAW
        and value.get("exact_expected_game_hour") is True,
        "readiness does not authenticate the exact paused D04 process",
    )


def validate_pulse(value: dict, identity: dict, action: dict) -> None:
    require(
        value.get("schema") == "hoi4-rendering-law-authenticated-clock-pulse-v2"
        and value.get("status") == "PASS"
        and value.get("process_id") == identity["pid"]
        and value.get("executable_sha256") == EXE_SHA256
        and value.get("process_start_utc") == action["process_start_utc"]
        and value.get("process_start_after_utc") == action["process_start_utc"]
        and value.get("game_hour_before") == D04_HOUR_RAW
        and value.get("game_hours_advanced") == 1
        and value.get("exact_target_reached") is True
        and value.get("pause_after") == 1,
        "clock pulse does not bind one exact same-process D04 hour",
    )


def selected_present_sha(value: dict) -> str:
    ordered = {
        "candidate_count": value.get("candidate_count"),
        "candidate_set_sha256": value.get("candidate_set_sha256"),
        "selected_candidate_id": str(uint(value.get("selected_candidate_id"), 64, "selected_candidate_id")),
        "selected_ordinal": value.get("selected_ordinal"),
        "selected_game_hour_raw": value.get("selected_game_hour_raw"),
        "selected_qpc_ticks": str(uint(value.get("selected_qpc_ticks"), 64, "selected_qpc_ticks")),
        "selection_cutoff_qpc_ticks": str(uint(value.get("selection_cutoff_qpc_ticks"), 64, "selection_cutoff_qpc_ticks")),
    }
    require(exact_int(ordered["candidate_count"], 1, (1 << 32) - 1), "selected-Present candidate count is invalid")
    require(lower_sha(ordered["candidate_set_sha256"]), "selected-Present candidate-set hash is invalid")
    require(exact_int(ordered["selected_ordinal"], 0, ordered["candidate_count"] - 1), "selected-Present ordinal is invalid")
    require(exact_int(ordered["selected_game_hour_raw"], 1, (1 << 31) - 1), "selected-Present game hour is invalid")
    return sha256_bytes(json.dumps(ordered, separators=(",", ":"), ensure_ascii=False).encode("utf-8"))


def observation_sha(value: dict) -> str:
    variant = value.get("variant")
    require(variant == "mouse_keyboard", "A32 observation is not mouse_keyboard")
    parts = [
        DOMAIN_SEPARATOR,
        str(value.get("executable_sha256", "")).encode("ascii"),
        struct.pack("<I", uint(value.get("process_id"), 32, "observation process_id")),
    ]
    for key in (
        "process_start_nonce",
        "hook_epoch_nonce",
        "observation_sequence",
        "prior_observation_qpc_ticks",
        "selected_present_qpc_ticks",
        "present_sample_started_qpc_ticks",
        "present_sample_completed_qpc_ticks",
    ):
        parts.append(struct.pack("<Q", uint(value.get(key), 64, key)))
    parts.append(str(value.get("selected_present_receipt_sha256", "")).encode("ascii"))
    extent = value.get("client_extent")
    prior = value.get("prior_client_position")
    current = value.get("current_client_position")
    require(isinstance(extent, list) and len(extent) == 2, "observation client extent is malformed")
    require(isinstance(prior, list) and len(prior) == 2, "observation prior position is malformed")
    require(isinstance(current, list) and len(current) == 2, "observation current position is malformed")
    for item in extent:
        parts.append(struct.pack("<I", uint(item, 32, "client extent")))
    for item in prior + current:
        require(exact_int(item, -(1 << 31), (1 << 31) - 1), "observation position is not i32")
        parts.append(struct.pack("<i", item))
    parts.append(b"\x01")
    parts.append(str(value.get("authority_manifest_sha256", "")).encode("ascii"))
    return sha256_bytes(b"".join(parts))


def inspect_replay(path: Path, manifest: dict, identity: dict, action: dict) -> tuple[list[dict], str]:
    try:
        archive = zipfile.ZipFile(path, "r")
    except (OSError, zipfile.BadZipFile):
        raise ValueError("replay is not a readable ZIP container") from None
    witnesses = []
    with archive:
        embedded = [row for row in archive.infolist() if row.filename == "recorder/native_session_manifest.json"]
        require(len(embedded) == 1, "replay does not have one embedded capture manifest")
        embedded_raw = archive.read(embedded[0])
        embedded_manifest = json.loads(embedded_raw.decode("utf-8-sig"))
        for key in ("schema", "session_id", "capture_id", "pid", "replay_file"):
            require(embedded_manifest.get(key) == manifest.get(key), f"embedded/external manifest mismatch: {key}")
        snapshots = sorted(
            (row for row in archive.infolist() if not row.is_dir() and row.filename.endswith(".snapshot.json")),
            key=lambda row: row.filename,
        )
        require(snapshots and len(snapshots) <= 10_000, "replay snapshot roster is empty or excessive")
        for info in snapshots:
            require(info.file_size <= 256 * 1024 * 1024, "replay snapshot exceeds A32 bound")
            raw = archive.read(info)
            snapshot = json.loads(raw.decode("utf-8-sig"))
            render = snapshot.get("production_render")
            if not isinstance(render, dict):
                continue
            authority = render.get("authority")
            selected = render.get("selected_present")
            if not isinstance(authority, dict) or not isinstance(selected, dict):
                continue
            observation = authority.get("recorded_input_observation")
            if not isinstance(observation, dict):
                continue
            require(render.get("schema") == "hoi4.production-render-submit.v12", "A32 render schema is not v12")
            require(authority.get("recorded_input_variant") == "mouse_keyboard", "render authority input family mismatch")
            require(authority.get("process_id") == identity["pid"], "render authority PID mismatch")
            require(uint(authority.get("process_start_nonce"), 64, "render process nonce") == identity["process_start_nonce"], "render process nonce mismatch")
            require(observation.get("executable_sha256") == EXE_SHA256, "observation executable mismatch")
            require(observation.get("process_id") == identity["pid"], "observation PID mismatch")
            require(uint(observation.get("process_start_nonce"), 64, "observation process nonce") == identity["process_start_nonce"], "observation process nonce mismatch")
            require(observation.get("authority_manifest_sha256") == AUTHORITY_SHA256, "observation authority manifest mismatch")
            computed_selected = selected_present_sha(selected)
            require(observation.get("selected_present_receipt_sha256") == computed_selected, "observation selected-Present seal mismatch")
            computed_observation = observation_sha(observation)
            require(observation.get("observation_receipt_sha256") == computed_observation, "observation receipt digest mismatch")
            require(authority.get("recorded_input_receipt_sha256") == computed_observation, "render authority receipt digest mismatch")
            prior_qpc = uint(observation.get("prior_observation_qpc_ticks"), 64, "prior observation QPC")
            selected_qpc = uint(observation.get("selected_present_qpc_ticks"), 64, "selected-Present QPC")
            sample_start = uint(observation.get("present_sample_started_qpc_ticks"), 64, "sample-start QPC")
            sample_end = uint(observation.get("present_sample_completed_qpc_ticks"), 64, "sample-end QPC")
            require(prior_qpc < selected_qpc <= sample_start <= sample_end, "observation chronology is invalid")
            require(
                action["started_qpc"] <= prior_qpc
                and sample_end <= action["completed_qpc"],
                "selected-Present observation falls outside authenticated pointer actions",
            )
            require(observation.get("client_extent") == action["extent"], "action/producer client extent mismatch")
            prior = observation.get("prior_client_position")
            current = observation.get("current_client_position")
            require(prior in action["targets"] and current in action["targets"] and prior != current, "producer movement is not the authenticated target pair")
            require(selected.get("selected_qpc_ticks") == observation.get("selected_present_qpc_ticks"), "selected-Present/observation QPC mismatch")
            require(snapshot.get("game_hour_raw") in (None, selected.get("selected_game_hour_raw")), "snapshot/selected-Present hour mismatch")
            witnesses.append(
                {
                    "snapshot_member": info.filename,
                    "snapshot_sha256": sha256_bytes(raw),
                    "selected_game_hour_raw": selected.get("selected_game_hour_raw"),
                    "selected_present_receipt_sha256": computed_selected,
                    "observation_receipt_sha256": computed_observation,
                    "observation_sequence": uint(observation.get("observation_sequence"), 64, "observation sequence"),
                    "hook_epoch_nonce": str(uint(observation.get("hook_epoch_nonce"), 64, "hook epoch nonce")),
                    "prior_observation_qpc_ticks": str(prior_qpc),
                    "selected_present_qpc_ticks": str(selected_qpc),
                    "present_sample_started_qpc_ticks": str(sample_start),
                    "present_sample_completed_qpc_ticks": str(sample_end),
                    "client_extent": action["extent"],
                    "prior_client_position": prior,
                    "current_client_position": current,
                }
            )
    require(witnesses, "no authenticated A32 selected-Present pointer movement exists in replay")
    identities = {row["observation_receipt_sha256"] for row in witnesses}
    require(len(identities) == 1, "replay contains more than one A32 pointer authority identity")
    return witnesses, sha256_bytes(embedded_raw)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture-manifest", required=True, type=Path)
    parser.add_argument("--prebuilt-attestation", required=True, type=Path)
    parser.add_argument("--static-bundle-preflight", required=True, type=Path)
    parser.add_argument("--readiness-receipt", required=True, type=Path)
    parser.add_argument("--pulse-receipt", required=True, type=Path)
    parser.add_argument("--action-receipt", required=True, type=Path)
    parser.add_argument("--action-ready-receipt", required=True, type=Path)
    parser.add_argument("--replay", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()

    manifest = load_json(args.capture_manifest, "capture manifest")
    identity = validate_manifest(manifest)
    artifacts = validate_attestation(load_json(args.prebuilt_attestation, "central attestation"), manifest)
    validate_preflight(load_json(args.static_bundle_preflight, "static-bundle preflight"), manifest)
    action = validate_action(load_json(args.action_receipt, "A32 action receipt"), identity)
    validate_action_ready(load_json(args.action_ready_receipt, "A32 action readiness"), identity, action)
    validate_readiness(load_json(args.readiness_receipt, "paused-save readiness"), identity, action)
    validate_pulse(load_json(args.pulse_receipt, "clock pulse"), identity, action)
    witnesses, embedded_manifest_sha = inspect_replay(args.replay, manifest, identity, action)

    input_paths = {
        "capture_manifest": args.capture_manifest,
        "central_release_attestation": args.prebuilt_attestation,
        "static_bundle_preflight": args.static_bundle_preflight,
        "paused_save_readiness": args.readiness_receipt,
        "authenticated_clock_pulse": args.pulse_receipt,
        "authenticated_pointer_actions": args.action_receipt,
        "authenticated_pointer_current_ready": args.action_ready_receipt,
        "replay": args.replay,
    }
    admission = {
        "schema": "hoi4.xui25.a32.immutable-live-pointer-admission.v1",
        "status": "PASS_PRODUCTION_ADMISSION",
        "production_ready": True,
        "authority": {
            "program": "hoi4.exe 1.19.2.0.a729",
            "executable_sha256": EXE_SHA256,
            "recorded_input_authority_manifest_sha256": AUTHORITY_SHA256,
        },
        "capture_identity": {
            "session_id": identity["session_id"],
            "capture_id": identity["capture_id"],
            "process_id": identity["pid"],
            "process_start_nonce": str(identity["process_start_nonce"]),
            "process_start_utc": action["process_start_utc"],
            "product_modset_sha256": identity["product_modset_sha256"],
        },
        "admitted_input_variant": "mouse_keyboard",
        "selected_present_pointer_movement": witnesses[0],
        "matching_snapshot_count": len(witnesses),
        "machine_action_count": action["action_count"],
        "embedded_capture_manifest_sha256": embedded_manifest_sha,
        "central_artifacts": artifacts,
        "inputs": {
            key: {"bytes": path.resolve().stat().st_size, "sha256": sha256_file(path.resolve())}
            for key, path in input_paths.items()
        },
        "failure_policy": "any identity, chronology, movement, digest, process, launch, modset, or selected-Present disagreement rejects; no input-family fallback",
    }
    write_create_new(args.out, admission)
    print(f"A32/XUI25 immutable admission PASS: {args.out.resolve()}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
