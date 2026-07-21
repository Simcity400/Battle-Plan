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
STREAM_SCHEMA = "battle-plan.campaign_identity_stream_boundary.v1"
ACTION_SCHEMA = "battle-plan.campaign_identity_arm_action_receipt.v1"
CURRENT_EXE_SHA = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
ARMS = ("S1", "S2", "S3", "S4", "D1", "D2", "M1", "M2", "M3")
HEX64 = re.compile(r"^[0-9a-f]{64}$")
UUID = re.compile(r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")

ROOT_KEYS = {"schema", "contract_path", "contract_sha256", "created_filetime_utc", "arms"}
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
}
STREAM_KEYS = {
    "schema", "status", "transcript_sha256", "campaign_id_v1", "boundary_kind",
    "previous_campaign_id_v1", "cra1_acknowledged", "logical_stream_id",
    "cid1_sequence", "cid1_domain", "cid1_type", "cid1_payload_sha256",
    "cfa1_payload_sha256", "first_world_sequence", "no_uncertain_frames",
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


def campaign_id(uuid: str, seed: int) -> str:
    return hashlib.sha256(
        b"hoi4-campaign-id-v1\0" + uuid.encode("ascii") + b"\0" + str(seed).encode("ascii")
    ).hexdigest()


def endpoint(value: dict, arm: str, expected_role: str) -> dict:
    value = exact(value, ENDPOINT_KEYS, f"{arm}/{expected_role} endpoint")
    if value["role"] != expected_role or not isinstance(value["machine_id"], str) or not value["machine_id"]:
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
        "transcript_sha256": value["transcript_sha256"],
        "logical_stream_id": stream["logical_stream_id"],
        "boundary_kind": stream["boundary_kind"],
        "previous_campaign_id_v1": stream["previous_campaign_id_v1"],
        "cra1_acknowledged": stream["cra1_acknowledged"],
    }


def verify_arm(arm: str, value: dict) -> dict:
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
    del driver
    del precondition
    if not isinstance(action["started_filetime_utc"], int) or not isinstance(action["completed_filetime_utc"], int) or action["completed_filetime_utc"] <= action["started_filetime_utc"]:
        raise GateError(f"arm {arm} action interval is invalid")
    if not isinstance(value["endpoints"], list) or len(value["endpoints"]) != 2:
        raise GateError(f"arm {arm} requires exactly two real endpoints")
    endpoints = [endpoint(raw, arm, role) for raw, role in zip(value["endpoints"], roles)]
    endpoint_hashes = [entry["transcript_sha256"] for entry in endpoints]
    if action["endpoint_transcript_sha256s"] != endpoint_hashes:
        raise GateError(f"arm {arm} action receipt does not bind its endpoint transcripts")
    left, right = endpoints
    same_pair = left["pair"] == right["pair"]
    if arm in {"S1", "S2", "S3", "S4", "M1", "M2", "M3"} and not same_pair:
        raise GateError(f"arm {arm} requires equal complete identity pairs")
    if arm in {"D1", "D2"} and same_pair:
        raise GateError(f"arm {arm} requires distinct complete identity pairs")
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
    if not isinstance(receipt["created_filetime_utc"], int) or receipt["created_filetime_utc"] <= 0:
        raise GateError("matrix receipt timestamp is invalid")
    arms = receipt["arms"]
    if not isinstance(arms, dict) or set(arms) != set(ARMS):
        raise GateError("matrix receipt must contain S1-S4,D1-D2,M1-M3 arms exactly")
    results = [verify_arm(arm, arms[arm]) for arm in ARMS]
    return {
        "schema": REPORT_SCHEMA,
        "verdict": "passed",
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
