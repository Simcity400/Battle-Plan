#!/usr/bin/env python3
"""Deny-extra regrader for the complete CID1 S/D/M lifecycle matrix.

This verifier never drives HOI4.  It joins already sealed single-process CID1
transcripts, their independent gate reports, same-tick save oracles, stream
boundary receipts, and authenticated arm-action receipts.  Missing endpoints
remain missing; an ``unavailable`` or synthetic endpoint can never pass.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import tempfile
from pathlib import Path


SCHEMA = "battle-plan.campaign_identity_matrix_receipt.v1"
REPORT_SCHEMA = "battle-plan.campaign_identity_matrix_gate.v1"
CONTRACT_SCHEMA = "battle-plan.campaign_identity_matrix_contract.v1"
TRANSCRIPT_SCHEMA = "battle-plan.campaign_identity_evidence_transcript.v1"
GATE_SCHEMA = "battle-plan.campaign_identity_gate.v1"
ORACLE_SCHEMA = "hoi4.save_campaign_identity_oracle.v1"
STREAM_SCHEMA = "battle-plan.campaign_identity_stream_boundary.v2"
ACTION_SCHEMA = "battle-plan.campaign_identity_arm_action_receipt.v1"
CURRENT_EXE_SHA = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
ARMS = ("S1", "S2", "S3", "S4", "D1", "D2", "M1", "M2", "M3")
HEX64 = re.compile(r"^[0-9a-f]{64}$")
UUID = re.compile(r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")

ROOT_KEYS = {
    "schema", "contract_path", "contract_sha256", "matrix_nonce_hex",
    "created_filetime_utc", "arms",
}
ARM_KEYS = {"status", "action_receipt_path", "action_receipt_sha256", "endpoints"}
ENDPOINT_KEYS = {
    "role", "machine_id", "transcript_path", "transcript_file_sha256", "transcript_sha256", "gate_path",
    "gate_sha256", "oracle_path", "oracle_sha256", "stream_boundary_path",
    "stream_boundary_sha256",
}
ACTION_KEYS = {
    "schema", "arm", "status", "operation", "evidence_nonce_hex",
    "driver_path", "driver_sha256", "precondition_path", "precondition_sha256", "started_filetime_utc",
    "completed_filetime_utc", "endpoint_roles", "endpoint_transcript_sha256s",
    "request_path", "request_sha256", "owner_profile_path", "owner_profile_sha256",
    "execution_receipt_path", "execution_receipt_sha256",
}
PRECONDITION_KEYS = {
    "schema", "arm", "executable_sha256", "modset_identity",
    "modset_admission_sha256", "bookmark", "countries",
    "launch_settings_sha256", "selected_save_sha256s",
}
PASSIVE_REQUEST_KEYS = {
    "schema", "arm", "matrix_nonce_hex", "arm_nonce_hex",
    "pinned_executable_sha256", "contract_path", "contract_sha256",
    "registration_driver_path",
    "registration_driver_sha256", "precondition_path", "precondition_sha256",
    "endpoint_descriptor_paths", "endpoint_descriptor_sha256s", "output_root",
}
PASSIVE_PROFILE_KEYS = {
    "schema", "arm", "precondition_path", "precondition_sha256",
    "observation_policy", "endpoint_roles",
}
PASSIVE_EXECUTION_KEYS = {
    "schema", "arm", "request_path", "request_sha256", "owner_profile_path",
    "owner_profile_sha256", "started_filetime_utc", "completed_filetime_utc", "steps",
}
PASSIVE_REQUEST_SCHEMA = "battle-plan.campaign_identity_passive_arm_request.v1"
PASSIVE_PROFILE_SCHEMA = "battle-plan.campaign_identity_passive_observer_profile.v1"
PASSIVE_EXECUTION_SCHEMA = "battle-plan.campaign_identity_passive_arm_execution.v1"
STREAM_KEYS = {
    "schema", "status", "transcript_sha256", "campaign_id_v1", "boundary_kind",
    "previous_campaign_id_v1", "cra1_acknowledged", "logical_stream_id",
    "cid1_sequence", "cid1_domain", "cid1_type", "cid1_payload_sha256",
    "cfa1_payload_sha256", "first_world_sequence", "no_uncertain_frames",
    "rollover_set_descriptor_path", "rollover_set_descriptor_sha256",
    "raw_member_path", "raw_member_sha256", "raw_member_bytes",
    "last_complete_sequence",
}

EXPECTED_ROLES = {
    "S1": ("before_save", "after_same_process_reload"),
    "S2": ("newer_lineage", "older_lineage_reload"),
    "S3": ("before_relaunch", "after_relaunch"),
    "S4": ("original_name", "renamed_copy"),
    "D1": ("new_campaign_a", "new_campaign_b"),
    "D2": ("campaign_a", "campaign_b_after_switch"),
    "M1": ("host", "hotjoin_client"),
    "M2": ("host", "observer"),
    "M3": ("original_host", "rehost"),
}
EXPECTED_OPERATIONS = {
    "S1": "save_reload_same_process",
    "S2": "load_older_same_lineage",
    "S3": "close_relaunch_same_save",
    "S4": "rename_copy_load",
    "D1": "create_identical_start_campaign_pair",
    "D2": "switch_campaign_a_to_b_same_process",
    "M1": "host_hotjoin_same_hour",
    "M2": "host_observer_same_hour",
    "M3": "cross_machine_rehost_same_save",
}


class GateError(ValueError):
    pass


def _pairs(pairs):
    out = {}
    for key, value in pairs:
        if key in out:
            raise GateError(f"duplicate JSON key: {key}")
        out[key] = value
    return out


def load_json(path: Path, label: str):
    try:
        return json.loads(path.read_text(encoding="utf-8-sig"), object_pairs_hook=_pairs)
    except (OSError, json.JSONDecodeError) as exc:
        raise GateError(f"cannot read {label}: {exc}") from exc


def exact(value, keys: set[str], label: str):
    if not isinstance(value, dict) or set(value) != keys:
        actual = set(value) if isinstance(value, dict) else set()
        raise GateError(f"{label} keys differ; missing={sorted(keys-actual)} extra={sorted(actual-keys)}")
    return value


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def require_hash(value, label: str) -> str:
    if not isinstance(value, str) or HEX64.fullmatch(value) is None:
        raise GateError(f"{label} is not lowercase SHA-256")
    return value


def checked_path(raw, expected_hash, label: str) -> Path:
    path = Path(str(raw)).resolve()
    require_hash(expected_hash, f"{label} hash")
    if not path.is_file() or sha256_file(path) != expected_hash:
        raise GateError(f"{label} file/hash mismatch")
    return path


def checked_cid1_evidence_permit(
    path: Path, arm: str, request_path: Path, request: dict, owner_profile_sha256: str
) -> None:
    try:
        raw = path.read_bytes()
        text = raw.decode("utf-8")
    except (OSError, UnicodeDecodeError) as exc:
        raise GateError(f"arm {arm} evidence permit is unreadable: {exc}") from exc
    if not raw or len(raw) > 1024 or b"\r" in raw or not raw.endswith(b"\n"):
        raise GateError(f"arm {arm} evidence permit framing is invalid")
    lines = text[:-1].split("\n")
    if len(lines) != 8 or lines[0] != "CID1-LIFECYCLE-EVIDENCE-PERMIT-V1":
        raise GateError(f"arm {arm} evidence permit schema is invalid")
    expected = (
        ("arm", arm),
        ("matrix_nonce_hex", request.get("matrix_nonce_hex")),
        ("arm_nonce_hex", request.get("arm_nonce_hex")),
        ("request_sha256", sha256_file(request_path)),
        ("owner_profile_sha256", owner_profile_sha256),
        ("contract_sha256", request.get("contract_sha256")),
        ("pinned_executable_sha256", CURRENT_EXE_SHA),
    )
    for line, (key, expected_value) in zip(lines[1:], expected):
        if line != f"{key}={expected_value}":
            raise GateError(f"arm {arm} evidence permit {key} binding mismatch")
    matrix_nonce = request.get("matrix_nonce_hex")
    arm_nonce = request.get("arm_nonce_hex")
    if (not isinstance(matrix_nonce, str) or not isinstance(arm_nonce, str)
            or HEX64.fullmatch(matrix_nonce) is None or HEX64.fullmatch(arm_nonce) is None
            or matrix_nonce == "0" * 64 or arm_nonce == "0" * 64 or matrix_nonce == arm_nonce):
        raise GateError(f"arm {arm} evidence permit nonces are invalid")


def campaign_id(uuid: str, seed: int) -> str:
    return hashlib.sha256(
        b"hoi4-campaign-id-v1\0" + uuid.encode("ascii") + b"\0" + str(seed).encode("ascii")
    ).hexdigest()


def endpoint(value: dict, arm: str, expected_role: str) -> dict:
    value = exact(value, ENDPOINT_KEYS, f"{arm}/{expected_role} endpoint")
    if (value["role"] != expected_role or not isinstance(value["machine_id"], str)
            or HEX64.fullmatch(value["machine_id"]) is None):
        raise GateError(f"{arm}/{expected_role} endpoint role or machine id is invalid")

    transcript_path = checked_path(value["transcript_path"], value["transcript_file_sha256"], f"{arm}/{expected_role} transcript")
    gate_path = checked_path(value["gate_path"], value["gate_sha256"], f"{arm}/{expected_role} gate")
    oracle_path = checked_path(value["oracle_path"], value["oracle_sha256"], f"{arm}/{expected_role} oracle")
    stream_path = checked_path(value["stream_boundary_path"], value["stream_boundary_sha256"], f"{arm}/{expected_role} stream boundary")
    transcript = load_json(transcript_path, "transcript")
    gate = load_json(gate_path, "single-transcript gate")
    oracle = load_json(oracle_path, "save oracle")
    stream = exact(load_json(stream_path, "stream boundary"), STREAM_KEYS, "stream boundary")

    if transcript.get("schema") != TRANSCRIPT_SCHEMA or transcript.get("transcript_sha256") != value["transcript_sha256"]:
        raise GateError(f"{arm}/{expected_role} transcript authority mismatch")
    if gate.get("schema") != GATE_SCHEMA or gate.get("verdict") != "passed" or gate.get("transcript_sha256") != value["transcript_sha256"]:
        raise GateError(f"{arm}/{expected_role} single-transcript gate mismatch")
    if gate.get("exe_sha256") != CURRENT_EXE_SHA or gate.get("melt_comparison_checked") is not True:
        raise GateError(f"{arm}/{expected_role} lacks exact-build melt-checked authority")
    observations = transcript.get("observations")
    if not isinstance(observations, list) or not observations:
        raise GateError(f"{arm}/{expected_role} transcript has no observations")
    observation = observations[-1]
    if observation.get("action_token") != f"OBSERVE_CAMPAIGN_IDENTITY_{observation.get('label')}_AUTOMATIC_SAME_HOUR_V1":
        raise GateError(f"{arm}/{expected_role} did not use the authenticated automatic same-hour barrier")
    read_a, read_b = observation.get("read_a"), observation.get("read_b")
    if not isinstance(read_a, dict) or not isinstance(read_b, dict):
        raise GateError(f"{arm}/{expected_role} final observation is incomplete")
    if read_a.get("game_hour_raw") != read_b.get("game_hour_raw"):
        raise GateError(f"{arm}/{expected_role} final observation is not same-hour")
    uuid, seed = read_b.get("game_unique_id"), read_b.get("game_unique_seed")
    if not isinstance(uuid, str) or UUID.fullmatch(uuid) is None or not isinstance(seed, int) or isinstance(seed, bool):
        raise GateError(f"{arm}/{expected_role} live identity is malformed")
    digest = campaign_id(uuid, seed)

    if oracle.get("schema") != ORACLE_SCHEMA or not isinstance(oracle.get("identity"), dict):
        raise GateError(f"{arm}/{expected_role} oracle schema mismatch")
    identity = oracle["identity"]
    if identity.get("pair_valid") is not True or identity.get("game_unique_id") != uuid:
        raise GateError(f"{arm}/{expected_role} live/save UUID mismatch")
    if str(identity.get("game_unique_seed")) != str(seed) or identity.get("campaign_id_v1") != digest:
        raise GateError(f"{arm}/{expected_role} live/save seed or digest mismatch")

    if stream["schema"] != STREAM_SCHEMA or stream["status"] != "passed":
        raise GateError(f"{arm}/{expected_role} stream boundary is not passed")
    if not isinstance(stream["logical_stream_id"], int) or isinstance(stream["logical_stream_id"], bool) or stream["logical_stream_id"] < 0:
        raise GateError(f"{arm}/{expected_role} logical stream id is invalid")
    if stream["transcript_sha256"] != value["transcript_sha256"] or stream["campaign_id_v1"] != digest:
        raise GateError(f"{arm}/{expected_role} stream boundary identity mismatch")
    if (stream["cid1_sequence"], stream["cid1_domain"], stream["cid1_type"], stream["first_world_sequence"]) != (0, 0, 4, 1):
        raise GateError(f"{arm}/{expected_role} CID1/world ordering mismatch")
    if stream["cid1_payload_sha256"] != stream["cfa1_payload_sha256"] or stream["no_uncertain_frames"] is not True:
        raise GateError(f"{arm}/{expected_role} CFA1 or uncertainty boundary mismatch")
    descriptor_path = checked_path(
        stream["rollover_set_descriptor_path"],
        stream["rollover_set_descriptor_sha256"],
        f"{arm}/{expected_role} rollover descriptor",
    )
    raw_member_path = checked_path(
        stream["raw_member_path"], stream["raw_member_sha256"],
        f"{arm}/{expected_role} raw logical stream",
    )
    if raw_member_path.stat().st_size != stream["raw_member_bytes"]:
        raise GateError(f"{arm}/{expected_role} raw member byte count mismatch")
    try:
        import materialize_cid1_stream_boundary as boundary_authority
        descriptor = boundary_authority.load_json(descriptor_path, "rollover set descriptor")
        parsed_member = boundary_authority.parse_stream(raw_member_path)
    except Exception as exc:
        raise GateError(f"{arm}/{expected_role} raw stream authority rejected: {exc}") from exc
    streams = descriptor.get("ordered_streams") if isinstance(descriptor, dict) else None
    if descriptor.get("schema") != "cid1-rollover-set-v1" or not isinstance(streams, list):
        raise GateError(f"{arm}/{expected_role} rollover descriptor schema mismatch")
    logical_id = stream["logical_stream_id"]
    if logical_id >= len(streams) or streams[logical_id].get("logical_stream_id") != logical_id:
        raise GateError(f"{arm}/{expected_role} descriptor lacks logical member")
    member = streams[logical_id]
    descriptor_member_path = (descriptor_path.parent / member.get("relative_path", "")).resolve()
    if descriptor_member_path != raw_member_path:
        raise GateError(f"{arm}/{expected_role} descriptor/raw member path mismatch")
    if (member.get("file_sha256_hex") != stream["raw_member_sha256"]
            or member.get("file_bytes") != stream["raw_member_bytes"]
            or member.get("last_complete_sequence") != stream["last_complete_sequence"]
            or parsed_member["last_complete_sequence"] != stream["last_complete_sequence"]
            or parsed_member["campaign_id_v1"] != digest
            or parsed_member["payload_sha256"] != stream["cid1_payload_sha256"]):
        raise GateError(f"{arm}/{expected_role} descriptor/raw CID1 join mismatch")
    if stream["boundary_kind"] not in {"initial", "rollover"} or not isinstance(stream["cra1_acknowledged"], bool):
        raise GateError(f"{arm}/{expected_role} stream boundary kind is invalid")
    if stream["boundary_kind"] == "initial":
        if stream["previous_campaign_id_v1"] != "" or stream["cra1_acknowledged"] is not False:
            raise GateError(f"{arm}/{expected_role} initial boundary carries rollover authority")
    elif require_hash(stream["previous_campaign_id_v1"], "previous campaign digest") == digest:
        # Same-pair rollovers are permitted for reload/rollback, so equality is
        # not an error here.  Arm-specific checks below bind the predecessor.
        pass
    require_hash(stream["cid1_payload_sha256"], "CID1 payload hash")

    process = transcript.get("process", {})
    return {
        "role": expected_role,
        "machine_id": value["machine_id"],
        "pair": (uuid, seed, digest),
        "hour": read_b["game_hour_raw"],
        "pid": process.get("pid"),
        "creation": process.get("creation_filetime_utc"),
        "oracle_filename": oracle.get("source", {}).get("filename"),
        "oracle_save_sha256": require_hash(oracle.get("source", {}).get("sha256"), f"{arm}/{expected_role} oracle save hash"),
        "transcript_sha256": value["transcript_sha256"],
        "logical_stream_id": stream["logical_stream_id"],
        "boundary_kind": stream["boundary_kind"],
        "previous_campaign_id_v1": stream["previous_campaign_id_v1"],
        "cra1_acknowledged": stream["cra1_acknowledged"],
    }


def verify_arm(
    arm: str, value: dict, *, contract_path: Path,
    contract_sha256: str, matrix_nonce_hex: str,
) -> dict:
    value = exact(value, ARM_KEYS, f"arm {arm}")
    if value["status"] != "complete":
        raise GateError(f"arm {arm} is not complete")
    action_path = checked_path(value["action_receipt_path"], value["action_receipt_sha256"], f"arm {arm} action receipt")
    action = exact(load_json(action_path, f"arm {arm} action receipt"), ACTION_KEYS, f"arm {arm} action receipt")
    roles = EXPECTED_ROLES[arm]
    if action["schema"] != ACTION_SCHEMA or action["arm"] != arm or action["status"] != "passed":
        raise GateError(f"arm {arm} action receipt authority mismatch")
    if action["operation"] != EXPECTED_OPERATIONS[arm] or tuple(action["endpoint_roles"]) != roles:
        raise GateError(f"arm {arm} action operation/roles mismatch")
    require_hash(action["evidence_nonce_hex"], f"arm {arm} evidence nonce")
    require_hash(action["precondition_sha256"], f"arm {arm} precondition hash")
    driver = checked_path(action["driver_path"], action["driver_sha256"], f"arm {arm} driver")
    precondition = checked_path(action["precondition_path"], action["precondition_sha256"], f"arm {arm} precondition")
    precondition_value = exact(load_json(precondition, f"arm {arm} precondition"), PRECONDITION_KEYS, f"arm {arm} precondition")
    if (precondition_value["schema"] != "battle-plan.campaign_identity_arm_precondition.v1"
            or precondition_value["arm"] != arm
            or precondition_value["executable_sha256"] != CURRENT_EXE_SHA
            or not isinstance(precondition_value["modset_identity"], str) or not precondition_value["modset_identity"]
            or not isinstance(precondition_value["bookmark"], str) or not precondition_value["bookmark"]
            or not isinstance(precondition_value["countries"], list) or not precondition_value["countries"]
            or not all(isinstance(tag, str) and re.fullmatch(r"[A-Z0-9]{3}", tag) for tag in precondition_value["countries"])
            or not isinstance(precondition_value["selected_save_sha256s"], list)
            or not all(HEX64.fullmatch(str(value)) for value in precondition_value["selected_save_sha256s"])):
        raise GateError(f"arm {arm} precondition semantics are invalid")
    require_hash(precondition_value["modset_admission_sha256"], f"arm {arm} modset admission")
    require_hash(precondition_value["launch_settings_sha256"], f"arm {arm} launch settings")
    request_path = checked_path(action["request_path"], action["request_sha256"], f"arm {arm} request")
    owner_profile_path = checked_path(action["owner_profile_path"], action["owner_profile_sha256"], f"arm {arm} owner profile")
    execution_path = checked_path(action["execution_receipt_path"], action["execution_receipt_sha256"], f"arm {arm} execution receipt")
    request = load_json(request_path, f"arm {arm} request")
    owner_profile = load_json(owner_profile_path, f"arm {arm} owner profile")
    execution = load_json(execution_path, f"arm {arm} execution receipt")
    passive = request.get("schema") == PASSIVE_REQUEST_SCHEMA
    passive_descriptor_values = None
    sd_arm = arm in {"S1", "S2", "S3", "S4", "D1", "D2"}
    if passive:
        request = exact(request, PASSIVE_REQUEST_KEYS, f"arm {arm} passive request")
        owner_profile = exact(owner_profile, PASSIVE_PROFILE_KEYS, f"arm {arm} passive profile")
        execution = exact(execution, PASSIVE_EXECUTION_KEYS, f"arm {arm} passive execution")
        if (request["arm"] != arm
                or request["pinned_executable_sha256"] != CURRENT_EXE_SHA
                or Path(str(request["contract_path"])).resolve() != contract_path
                or request["contract_sha256"] != contract_sha256
                or Path(str(request["registration_driver_path"])).resolve() != driver
                or request["registration_driver_sha256"] != action["driver_sha256"]
                or Path(str(request["precondition_path"])).resolve() != precondition
                or request["precondition_sha256"] != action["precondition_sha256"]):
            raise GateError(f"arm {arm} passive request binding mismatch")
        require_hash(request["matrix_nonce_hex"], f"arm {arm} matrix nonce")
        require_hash(request["arm_nonce_hex"], f"arm {arm} plan nonce")
        if (request["matrix_nonce_hex"] != matrix_nonce_hex
                or request["matrix_nonce_hex"] == request["arm_nonce_hex"]
                or action["evidence_nonce_hex"] != request["arm_nonce_hex"]):
            raise GateError(f"arm {arm} passive request nonce binding mismatch")
        if (owner_profile["schema"] != PASSIVE_PROFILE_SCHEMA
                or owner_profile["arm"] != arm
                or Path(str(owner_profile["precondition_path"])).resolve() != precondition
                or owner_profile["precondition_sha256"] != action["precondition_sha256"]
                or owner_profile["observation_policy"] != "user_drives_client_observer_records_only"
                or tuple(owner_profile["endpoint_roles"]) != roles):
            raise GateError(f"arm {arm} passive profile/precondition binding mismatch")
        if (execution["schema"] != PASSIVE_EXECUTION_SCHEMA
                or execution["arm"] != arm
                or Path(str(execution["request_path"])).resolve() != request_path
                or execution["request_sha256"] != action["request_sha256"]
                or Path(str(execution["owner_profile_path"])).resolve() != owner_profile_path
                or execution["owner_profile_sha256"] != action["owner_profile_sha256"]
                or execution["started_filetime_utc"] != action["started_filetime_utc"]
                or execution["completed_filetime_utc"] != action["completed_filetime_utc"]
                or not isinstance(execution["steps"], list) or not execution["steps"]
                or not isinstance(action["endpoint_transcript_sha256s"], list)
                or len(action["endpoint_transcript_sha256s"]) != 2):
            raise GateError(f"arm {arm} passive execution receipt binding mismatch")
    else:
        request_schema = ("battle-plan.campaign_identity_sd_arm_request.v2" if sd_arm
                          else "battle-plan.campaign_identity_multiplayer_action_request.v1")
        profile_schema = ("battle-plan.campaign_identity_sd_owner_profile.v1" if sd_arm
                          else "battle-plan.campaign_identity_multiplayer_owner_profile.v1")
        execution_schema = ("battle-plan.campaign_identity_sd_owner_execution.v1" if sd_arm
                            else "battle-plan.campaign_identity_multiplayer_owner_execution.v1")
        driver_path_key = "action_driver_path" if sd_arm else "transport_driver_path"
        driver_hash_key = "action_driver_sha256" if sd_arm else "transport_driver_sha256"
        if (request.get("schema") != request_schema
                or request.get("arm") != arm
                or Path(str(request.get(driver_path_key, ""))).resolve() != driver
                or request.get(driver_hash_key) != action["driver_sha256"]
                or Path(str(request.get("owner_profile_path", ""))).resolve() != owner_profile_path
                or request.get("owner_profile_sha256") != action["owner_profile_sha256"]):
            raise GateError(f"arm {arm} request/driver/profile binding mismatch")
        if (owner_profile.get("schema") != profile_schema
                or owner_profile.get("arm") != arm
                or Path(str(owner_profile.get("precondition_path", ""))).resolve() != precondition
                or owner_profile.get("precondition_sha256") != action["precondition_sha256"]):
            raise GateError(f"arm {arm} owner profile/precondition binding mismatch")
        if (execution.get("schema") != execution_schema
                or execution.get("arm") != arm
                or Path(str(execution.get("request_path", ""))).resolve() != request_path
                or execution.get("request_sha256") != action["request_sha256"]
                or Path(str(execution.get("owner_profile_path", ""))).resolve() != owner_profile_path
                or execution.get("owner_profile_sha256") != action["owner_profile_sha256"]
                or execution.get("started_filetime_utc") != action["started_filetime_utc"]
                or execution.get("completed_filetime_utc") != action["completed_filetime_utc"]
                or not isinstance(execution.get("steps"), list) or not execution["steps"]):
            raise GateError(f"arm {arm} execution receipt binding mismatch")
    if sd_arm and not passive:
        permit_path = checked_path(
            execution.get("evidence_permit_path"),
            execution.get("evidence_permit_sha256"),
            f"arm {arm} evidence permit",
        )
        checked_cid1_evidence_permit(
            permit_path, arm, request_path, request, action["owner_profile_sha256"]
        )
    step_kinds = [step.get("kind") for step in execution["steps"] if isinstance(step, dict)]
    if len(step_kinds) != len(execution["steps"]):
        raise GateError(f"arm {arm} execution receipt contains a malformed step")
    required_step_kinds = {
        "S1": {"observe_endpoint", "window_input", "finalize_endpoint"},
        "S2": {"observe_endpoint", "window_input", "finalize_endpoint"},
        "S3": {"observe_endpoint", "discover_exact_process", "wait_process_exit", "finalize_endpoint"},
        "S4": {"copy_save_create_new", "observe_endpoint", "discover_exact_process", "finalize_endpoint"},
        "D1": {"discover_created_save", "observe_endpoint", "window_input", "finalize_endpoint"},
        "D2": {"observe_endpoint", "window_input", "finalize_endpoint"},
    }
    if sd_arm and not passive and not required_step_kinds[arm].issubset(set(step_kinds)):
        raise GateError(f"arm {arm} execution receipt omits an operation-defining owner step")
    if sd_arm and not passive:
        recording_steps = [step for step in execution["steps"] if step.get("kind") == "start_recording_owner"]
        if not recording_steps or any(
            Path(str(step.get("evidence_permit_path", ""))).resolve() != permit_path
            or step.get("evidence_permit_sha256") != execution.get("evidence_permit_sha256")
            for step in recording_steps
        ):
            raise GateError(f"arm {arm} recording owner did not inherit the authenticated evidence permit")
        observed_roles = [step.get("role") for step in execution["steps"] if isinstance(step, dict) and step.get("kind") == "observe_endpoint"]
        finalized_roles = [step.get("role") for step in execution["steps"] if isinstance(step, dict) and step.get("kind") == "finalize_endpoint"]
        if observed_roles != list(roles) or finalized_roles != list(roles):
            raise GateError(f"arm {arm} owner steps do not acquire/finalize endpoint roles in contract order")
        if arm == "D1" and step_kinds.count("discover_created_save") != 2:
            raise GateError("arm D1 must discover exactly two freshly created saves")
        if arm == "S4" and step_kinds.count("copy_save_create_new") != 1:
            raise GateError("arm S4 must create exactly one byte-identical renamed save")
    if passive:
        expected_steps = ["register_endpoint", "register_endpoint", "classify_relationship"]
        if step_kinds != expected_steps:
            raise GateError(f"arm {arm} passive execution has unexpected steps")
        register_steps = execution["steps"][:2]
        if [step.get("role") for step in register_steps] != list(roles):
            raise GateError(f"arm {arm} passive endpoint roles are out of order")
        descriptor_paths = request["endpoint_descriptor_paths"]
        descriptor_hashes = request["endpoint_descriptor_sha256s"]
        if (not isinstance(descriptor_paths, list) or len(descriptor_paths) != 2
                or not isinstance(descriptor_hashes, list) or len(descriptor_hashes) != 2):
            raise GateError(f"arm {arm} passive request must bind two endpoint descriptors")
        passive_descriptor_values = []
        for index, step in enumerate(register_steps):
            descriptor = checked_path(
                descriptor_paths[index], descriptor_hashes[index],
                f"arm {arm} passive endpoint descriptor {index}",
            )
            if (Path(str(step.get("descriptor_path", ""))).resolve() != descriptor
                    or step.get("descriptor_sha256") != descriptor_hashes[index]
                    or step.get("transcript_sha256") != action["endpoint_transcript_sha256s"][index]):
                raise GateError(f"arm {arm} passive endpoint registration binding mismatch")
            passive_descriptor_values.append(
                load_json(descriptor, f"arm {arm} passive endpoint descriptor {index}")
            )
        classifier = execution["steps"][2]
        if (classifier.get("classifier") != "verify_campaign_identity_matrix.py"
                or classifier.get("policy") != "independent_fail_closed_regrade"):
            raise GateError(f"arm {arm} passive classifier policy mismatch")
    del driver
    del precondition
    if not isinstance(action["started_filetime_utc"], int) or not isinstance(action["completed_filetime_utc"], int) or action["completed_filetime_utc"] <= action["started_filetime_utc"]:
        raise GateError(f"arm {arm} action interval is invalid")
    if not isinstance(value["endpoints"], list) or len(value["endpoints"]) != 2:
        raise GateError(f"arm {arm} requires exactly two real endpoints")
    if passive and value["endpoints"] != passive_descriptor_values:
        raise GateError(f"arm {arm} matrix endpoints differ from registered descriptor bytes")
    endpoints = [endpoint(raw, arm, role) for raw, role in zip(value["endpoints"], roles)]
    endpoint_hashes = [entry["transcript_sha256"] for entry in endpoints]
    if action["endpoint_transcript_sha256s"] != endpoint_hashes:
        raise GateError(f"arm {arm} action receipt does not bind its endpoint transcripts")
    left, right = endpoints
    selected_save_hashes = precondition_value["selected_save_sha256s"]
    if arm == "D1":
        if selected_save_hashes:
            raise GateError("arm D1 precondition must not preselect retained saves")
    elif selected_save_hashes != [left["oracle_save_sha256"], right["oracle_save_sha256"]]:
        raise GateError(f"arm {arm} precondition does not bind the ordered endpoint saves")
    same_pair = left["pair"] == right["pair"]
    if arm in {"S1", "S2", "S3", "S4", "M1", "M2", "M3"} and not same_pair:
        raise GateError(f"arm {arm} requires equal complete identity pairs")
    if arm in {"D1", "D2"} and same_pair:
        raise GateError(f"arm {arm} requires distinct complete identity pairs")
    if arm == "D1" and left["hour"] != right["hour"]:
        raise GateError("arm D1 identical-start campaigns were not observed at the same raw hour")
    if arm in {"S1", "S2", "D2"} and (left["pid"], left["creation"]) != (right["pid"], right["creation"]):
        raise GateError(f"arm {arm} requires one process lifetime")
    if arm in {"S1", "S2", "D2"}:
        if right["boundary_kind"] != "rollover" or right["cra1_acknowledged"] is not True:
            raise GateError(f"arm {arm} requires an acknowledged rollover before its second endpoint")
        if right["previous_campaign_id_v1"] != left["pair"][2] or right["logical_stream_id"] <= left["logical_stream_id"]:
            raise GateError(f"arm {arm} rollover does not bind the predecessor logical stream")
    if arm == "S2" and not right["hour"] < left["hour"]:
        raise GateError("arm S2 does not prove an older-hour rollback")
    if arm == "S3" and (left["pid"], left["creation"]) == (right["pid"], right["creation"]):
        raise GateError("arm S3 does not prove a process relaunch")
    if arm == "S4" and left["oracle_filename"] == right["oracle_filename"]:
        raise GateError("arm S4 does not prove a renamed/copy filename")
    if arm in {"M1", "M2"} and (left["pid"], left["creation"]) == (right["pid"], right["creation"]):
        raise GateError(f"arm {arm} requires two instrumented process endpoints")
    if arm in {"M1", "M2"} and left["hour"] != right["hour"]:
        raise GateError(f"arm {arm} endpoints are not at the same game hour")
    if arm == "M3" and left["machine_id"] == right["machine_id"]:
        raise GateError("arm M3 requires distinct machine identities")
    return {"arm": arm, "pair_relation": "equal" if same_pair else "different", "endpoint_transcripts": [e["transcript_sha256"] for e in endpoints]}


def verify(args) -> dict:
    contract_path = checked_path(args.contract, args.expected_contract_sha256, "matrix contract")
    contract = load_json(contract_path, "matrix contract")
    if contract.get("schema") != CONTRACT_SCHEMA or contract.get("pinned_executable_sha256") != CURRENT_EXE_SHA:
        raise GateError("matrix contract schema/build mismatch")
    receipt = exact(load_json(args.receipt.resolve(), "matrix receipt"), ROOT_KEYS, "matrix receipt")
    if receipt["schema"] != SCHEMA or Path(receipt["contract_path"]).resolve() != contract_path:
        raise GateError("matrix receipt contract path mismatch")
    if receipt["contract_sha256"] != args.expected_contract_sha256:
        raise GateError("matrix receipt contract hash mismatch")
    matrix_nonce_hex = require_hash(receipt["matrix_nonce_hex"], "matrix receipt nonce")
    if matrix_nonce_hex == "0" * 64:
        raise GateError("matrix receipt nonce is zero")
    if not isinstance(receipt["created_filetime_utc"], int) or receipt["created_filetime_utc"] <= 0:
        raise GateError("matrix receipt timestamp is invalid")
    arms = receipt["arms"]
    if not isinstance(arms, dict) or set(arms) != set(ARMS):
        raise GateError("matrix receipt must contain S1-S4,D1-D2,M1-M3 arms exactly")
    results = []
    external_m3_pending = False
    for arm in ARMS:
        arm_value = arms[arm]
        if (arm == "M3" and getattr(args, "allow_external_m3_pending", False)
                and isinstance(arm_value, dict)
                and arm_value.get("status") == "blocked_external_second_machine"):
            exact(arm_value, ARM_KEYS, "arm M3")
            if (arm_value["action_receipt_path"] != ""
                    or arm_value["action_receipt_sha256"] != ""
                    or arm_value["endpoints"] != []):
                raise GateError("pending M3 must not carry fabricated action or endpoint evidence")
            external_m3_pending = True
            results.append({
                "arm": "M3",
                "status": "blocked_external_second_machine",
                "pair_relation": "unobserved",
                "endpoint_transcripts": [],
            })
            continue
        results.append(verify_arm(
            arm,
            arm_value,
            contract_path=contract_path,
            contract_sha256=args.expected_contract_sha256,
            matrix_nonce_hex=matrix_nonce_hex,
        ))
    return {
        "schema": REPORT_SCHEMA,
        "verdict": "blocked_external_m3" if external_m3_pending else "passed",
        "local_production_ready": True,
        "cross_machine_rehost_ready": not external_m3_pending,
        "matrix_receipt_path": str(args.receipt.resolve()),
        "matrix_receipt_sha256": sha256_file(args.receipt.resolve()),
        "contract_path": str(contract_path),
        "contract_sha256": args.expected_contract_sha256,
        "pinned_executable_sha256": CURRENT_EXE_SHA,
        "arms": results,
    }


def atomic_json(path: Path, value) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix=f".{path.name}.", suffix=".tmp", dir=path.parent)
    try:
        with os.fdopen(fd, "w", encoding="utf-8", newline="\n") as handle:
            json.dump(value, handle, indent=2, sort_keys=True)
            handle.write("\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--contract", type=Path, required=True)
    parser.add_argument("--expected-contract-sha256", required=True)
    parser.add_argument("--receipt", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument(
        "--allow-external-m3-pending",
        action="store_true",
        help="validate S1-S4,D1-D2,M1-M2 while retaining M3 as an evidence-free fail-closed external dependency",
    )
    args = parser.parse_args(argv)
    try:
        require_hash(args.expected_contract_sha256, "expected contract hash")
        report = verify(args)
        atomic_json(args.out.resolve(), report)
    except (GateError, OSError) as exc:
        print(f"campaign-identity matrix v1: FAIL: {exc}")
        return 1
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
