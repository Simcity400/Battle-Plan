#!/usr/bin/env python3
"""Fail-closed admission for the exact-a729 X10 direct-transition owner fixture."""

from __future__ import annotations

import argparse
from datetime import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import tempfile
import zipfile


EXE_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
LEGACY_MANIFEST = "a9855541571204160bc4ac78dcf6f1dbdfa112496234e84540406a71c37d2fb3"
ON_ACTION_MANIFEST = "8012d37ac03a47187bc2744e509acac0dbec1e6a7a91ffd20d45d58903e0e3da"
TERMINAL_MANIFEST = "213944e845d02896efd26314003a7880384e9d1eddf4a2b799d4449632da8227"
CLUSTER_MANIFEST = "d94eae0f339ee53f6ce69ecc921925519f48138da756ebef353304e31a8d9603"
FULL_MASK = 0xFFF
LEGACY_MASK = 0x01F
DOMAIN = "focus_research_decisions_depth_live"
CANONICAL_ID = re.compile(r"^[a-z0-9][a-z0-9_.-]{7,159}$")
TOKEN = re.compile(r"^[A-Za-z0-9_.:-]{3,191}$")
SHA256 = re.compile(r"^[0-9a-f]{64}$")
MAX_SNAPSHOTS = 20_000
MAX_SNAPSHOT_BYTES = 256 * 1024 * 1024

ROOTS = {
    "focus_completion": (0x002CEF60, "return_observed", True),
    "research_threshold_crossing": (0x00ECE360, "return_observed", True),
    "ordinary_decision_taken": (0x00728BC0, "return_observed", True),
    "targeted_decision_taken": (0x00728CE0, "return_observed", True),
    "timed_decision_activated": (0x00728FB0, "return_observed", True),
    "on_action_invoked": (0x00A6AAE0, "invocation", False),
    "decision_cancel_effect_invoked": (0x0071B350, "invocation", False),
    "decision_remove_effect_invoked": (0x0071B590, "invocation", False),
    "decision_timeout_effect_invoked": (0x0071C490, "invocation", False),
    "targeted_decision_cancel_effect_invoked": (0x0071B7D0, "invocation", False),
    "targeted_decision_remove_effect_invoked": (0x0071BE10, "invocation", False),
    "targeted_decision_timeout_effect_invoked": (0x0071C130, "invocation", False),
}
TARGETED = {
    "targeted_decision_taken",
    "targeted_decision_cancel_effect_invoked",
    "targeted_decision_remove_effect_invoked",
    "targeted_decision_timeout_effect_invoked",
}
DECISIONS = {
    kind for kind in ROOTS if "decision" in kind
}
REQUIRED_CASES = {
    "focus_completion",
    "research_threshold_crossing",
    "ordinary_decision_taken",
    "targeted_decision_taken",
    "timed_decision_activated",
    "outer_and_nested_on_action",
    "country_root_scope",
    "from_scope",
    "from_from_scope",
    "ordinary_cancel_remove_timeout",
    "targeted_cancel_remove_timeout",
}


def exact_int(value, low=0, high=(1 << 64) - 1):
    return isinstance(value, int) and not isinstance(value, bool) and low <= value <= high


def lower_sha(value):
    return isinstance(value, str) and SHA256.fullmatch(value) is not None and value != "0" * 64


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def fnv1a64(value: bytes) -> int:
    digest = 0xCBF29CE484222325
    for byte in value:
        digest = ((digest ^ byte) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return digest


def load_object(raw: bytes, label: str):
    try:
        value = json.loads(raw.decode("utf-8-sig"))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError(f"{label} is not valid UTF-8 JSON: {error}") from None
    if not isinstance(value, dict):
        raise ValueError(f"{label} root is not an object")
    return value


def write_create_new(path: Path, value) -> None:
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
        try:
            os.link(temporary, path)
        except FileExistsError:
            raise ValueError(f"refusing to replace immutable receipt: {path}") from None
        temporary.unlink()
    finally:
        try:
            temporary.unlink()
        except FileNotFoundError:
            pass


def validate_contract(value):
    if value.get("schema") != "hoi4.x10.direct_transition_owner_fixture.v1":
        raise ValueError("X10 fixture contract schema mismatch")
    binary = value.get("binary")
    carrier = value.get("carrier")
    authority = value.get("authority")
    if (
        not isinstance(binary, dict)
        or binary.get("sha256") != EXE_SHA256
        or binary.get("image_base") != "0x140000000"
        or not isinstance(carrier, dict)
        or carrier.get("domain_id") != 45
        or carrier.get("magic") != "D45E"
        or carrier.get("trailer_version") != 3
        or carrier.get("evidence_activation_env") != "BATTLE_PLAN_D45_X10_OWNER_FIXTURE=1"
        or carrier.get("production_activation_before_fixture") is not False
        or not isinstance(authority, dict)
        or authority.get("legacy_manifest_sha256") != LEGACY_MANIFEST
        or authority.get("on_action_manifest_sha256") != ON_ACTION_MANIFEST
        or authority.get("terminal_manifest_sha256") != TERMINAL_MANIFEST
        or authority.get("cluster_exhaustion_manifest_sha256") != CLUSTER_MANIFEST
    ):
        raise ValueError("X10 fixture contract authority drift")
    kinds = value.get("required_kinds")
    if not isinstance(kinds, list) or len(kinds) != len(ROOTS):
        raise ValueError("X10 fixture required-kind table is malformed")
    observed = {}
    for row in kinds:
        if not isinstance(row, dict) or row.get("kind") not in ROOTS:
            raise ValueError("X10 fixture required-kind row is malformed")
        observed[row["kind"]] = (int(row["rva"], 16), row.get("phase"))
    if observed != {kind: values[:2] for kind, values in ROOTS.items()}:
        raise ValueError("X10 fixture required-kind table drift")
    if set(value.get("required_cases", [])) != REQUIRED_CASES:
        raise ValueError("X10 fixture required-case table drift")


def validate_profile(value):
    if value.get("schema") != "hoi4.x10.autonomous-owner-fixture-actions.v1":
        raise ValueError("X10 action profile schema mismatch")
    if (
        value.get("executable_sha256") != EXE_SHA256
        or not lower_sha(value.get("fixture_nonce_sha256"))
        or not lower_sha(value.get("product_modset_sha256"))
        or not lower_sha(value.get("save_sha256"))
        or not exact_int(value.get("expected_game_hour_raw"), 1, (1 << 31) - 1)
        or not exact_int(value.get("expected_client_width"), 640, 16384)
        or not exact_int(value.get("expected_client_height"), 480, 16384)
        or set(value.get("covered_cases", [])) != REQUIRED_CASES
    ):
        raise ValueError("X10 action profile authority or fixture identity is malformed")
    actions = value.get("actions")
    expectations = value.get("expected_fixture_rows")
    sources = value.get("fixture_sources")
    if not isinstance(actions, list) or not actions or len(actions) > 512:
        raise ValueError("X10 action profile actions are absent or out of bounds")
    labels = [row.get("label") for row in actions if isinstance(row, dict)]
    if len(labels) != len(actions) or any(not isinstance(label, str) or not label for label in labels) or len(set(labels)) != len(labels):
        raise ValueError("X10 action labels are malformed or duplicated")
    allowed_actions = {"wait_for_loaded_paused_save", "console_command", "click", "keypress", "advance_hours", "wait"}
    if any(row.get("kind") not in allowed_actions for row in actions):
        raise ValueError("X10 action profile contains an unsupported action kind")
    valid_keys = {"escape", "enter", "space", "f1", "f2", "f3", "f4", "f5"}
    for action in actions:
        kind = action["kind"]
        if kind == "wait_for_loaded_paused_save" and (
            not exact_int(action.get("timeout_seconds"), 1, 1800)
            or not exact_int(action.get("stable_milliseconds"), 250, 30_000)
        ):
            raise ValueError("X10 paused-save action timing is out of bounds")
        if kind == "console_command" and (
            not isinstance(action.get("command"), str)
            or not action["command"].strip()
            or len(action["command"]) > 1024
            or "\n" in action["command"]
            or "\r" in action["command"]
            or not exact_int(action.get("settle_milliseconds"), 0, 600_000)
        ):
            raise ValueError("X10 console action is malformed or out of bounds")
        if kind == "click" and (
            not exact_int(action.get("x"), 0, value["expected_client_width"] - 1)
            or not exact_int(action.get("y"), 0, value["expected_client_height"] - 1)
            or not exact_int(action.get("settle_milliseconds"), 0, 600_000)
        ):
            raise ValueError("X10 click action is outside the authenticated client")
        if kind == "keypress" and (
            str(action.get("key", "")).lower() not in valid_keys
            or not exact_int(action.get("settle_milliseconds"), 0, 600_000)
        ):
            raise ValueError("X10 keypress action is malformed")
        if kind == "advance_hours" and (
            not exact_int(action.get("hours"), 1, 168)
            or not exact_int(action.get("timeout_seconds"), 1, 1800)
        ):
            raise ValueError("X10 advancing-hour action is out of bounds")
        if kind == "wait" and not exact_int(action.get("milliseconds"), 0, 600_000):
            raise ValueError("X10 wait action is out of bounds")
    readiness = [row for row in actions if row.get("kind") == "wait_for_loaded_paused_save"]
    if (
        len(readiness) != 1
        or actions[0] is not readiness[0]
        or readiness[0].get("game_hour_raw") != value["expected_game_hour_raw"]
    ):
        raise ValueError("X10 action profile must begin with one exact paused-save readiness action")
    final_settle = actions[-1]
    if (
        final_settle.get("kind") != "advance_hours"
        or final_settle.get("role") != "final_drain_settle"
        or not exact_int(final_settle.get("hours"), 2, 168)
    ):
        raise ValueError("X10 action profile must end with a two-or-more-hour final drain settle")
    if not isinstance(sources, list) or not sources or len(sources) > 256:
        raise ValueError("X10 fixture source roster is absent or out of bounds")
    source_paths = set()
    for source in sources:
        if (
            not isinstance(source, dict)
            or not isinstance(source.get("relative_path"), str)
            or Path(source["relative_path"]).is_absolute()
            or ".." in Path(source["relative_path"]).parts
            or not lower_sha(source.get("sha256"))
        ):
            raise ValueError("X10 fixture source identity is not portable")
        if source["relative_path"] in source_paths:
            raise ValueError("X10 fixture source identity is duplicated")
        source_paths.add(source["relative_path"])
    if not isinstance(expectations, list) or len(expectations) < len(ROOTS) or len(expectations) > 512:
        raise ValueError("X10 expected fixture row table is absent or out of bounds")
    seen_kinds = set()
    seen_tokens = set()
    normalized = []
    action_ordinals = {label: ordinal for ordinal, label in enumerate(labels)}
    last_action_ordinal = -1
    for ordinal, row in enumerate(expectations):
        if not isinstance(row, dict):
            raise ValueError("X10 expected fixture row is not an object")
        kind = row.get("kind")
        token = row.get("authored_token")
        if kind not in ROOTS:
            raise ValueError("X10 expected fixture kind is malformed")
        if kind == "research_threshold_crossing":
            if token is not None or row.get("subject_token_hash_fnv1a64") not in (0, "0") or not exact_int(row.get("subject_key"), 1, (1 << 32) - 1):
                raise ValueError("X10 research expectation must use the exact definition index and zero token hash")
            token_hash = 0
        else:
            if not isinstance(token, str) or TOKEN.fullmatch(token) is None:
                raise ValueError("X10 expected authored token is malformed")
            token_hash = fnv1a64(token.encode("utf-8"))
            if row.get("subject_token_hash_fnv1a64") not in (token_hash, str(token_hash)):
                raise ValueError(f"X10 expected token hash drift at ordinal {ordinal}")
        identity = (kind, token_hash)
        if identity in seen_tokens:
            raise ValueError("X10 expected fixture kind/token is duplicated")
        seen_tokens.add(identity)
        seen_kinds.add(kind)
        owner_key = row.get("owner_key")
        if kind in DECISIONS or kind == "focus_completion":
            if not exact_int(owner_key, 1, (1 << 32) - 1):
                raise ValueError("X10 focus/decision expectation lacks an exact owner key")
        elif owner_key != 0:
            raise ValueError("X10 non-owner expectation must explicitly carry owner_key zero")
        target_required = row.get("target_required")
        if target_required is not (kind in TARGETED):
            raise ValueError("X10 target-presence expectation disagrees with the root kind")
        scope_identity = row.get("scope_identity")
        action_label = row.get("action_label")
        if action_label not in labels or action_label == final_settle.get("label"):
            raise ValueError("X10 expected fixture row is not bound to a transaction-driving action")
        if action_ordinals[action_label] < last_action_ordinal:
            raise ValueError("X10 expected fixture row order regresses across action steps")
        last_action_ordinal = action_ordinals[action_label]
        if kind == "on_action_invoked":
            if not isinstance(scope_identity, str) or TOKEN.fullmatch(scope_identity) is None:
                raise ValueError("X10 on-action expectation lacks a stable fixture scope identity")
        elif scope_identity is not None:
            raise ValueError("X10 non-on-action expectation carries a scope identity")
        normalized.append({
            "ordinal": ordinal,
            "kind": kind,
            "token": token,
            "token_hash": token_hash,
            "subject_key": row.get("subject_key", 0),
            "owner_key": owner_key,
            "target_required": target_required,
            "scope_identity": scope_identity,
            "action_label": action_label,
        })
    if seen_kinds != set(ROOTS):
        raise ValueError("X10 action profile does not witness every required root kind")
    return normalized


def validate_attestation(value):
    if (
        value.get("schema") != "battle-plan.central_rust_prebuilt_artifacts.v1"
        or value.get("status") != "PASS_CENTRAL_RELEASE_PREBUILT_ARTIFACTS"
        or value.get("successful_exit") is not True
    ):
        raise ValueError("central Release artifact attestation is not authoritative")
    rows = value.get("artifacts")
    central = value.get("central_validation")
    source = value.get("source_input")
    if (
        not isinstance(central, dict)
        or central.get("mode") != "Release"
        or central.get("requested_workspace") not in ("All", "Extractor")
        or "Extractor" not in central.get("validated_workspaces", [])
        or central.get("workspace_package_selection") != ["--workspace"]
        or not isinstance(source, dict)
        or source.get("schema") != "battle-plan.production_rust_input_digest.v1"
        or not lower_sha(source.get("sha256"))
    ):
        raise ValueError("central Release attestation is not a complete extractor build")
    if not isinstance(rows, list):
        raise ValueError("central Release artifact roster is malformed")
    resolved = {}
    for role, filename in (("recorder_host", "collector_record.exe"), ("collector_dll", "collector.dll")):
        matches = [row for row in rows if isinstance(row, dict) and row.get("role") == role]
        if len(matches) != 1 or Path(str(matches[0].get("path", ""))).name.lower() != filename or not lower_sha(matches[0].get("sha256")):
            raise ValueError(f"central Release artifact role is invalid: {role}")
        resolved[role] = matches[0]["sha256"]
    return resolved


def validate_preflight(value, profile):
    if (
        value.get("passed") is not True
        or value.get("verification_phase") != "pre_injection_deferred_allowed"
        or value.get("token_join_status") != "deferred"
        or not lower_sha(value.get("bundle_id"))
        or value.get("product_modset_sha256") != profile["product_modset_sha256"]
    ):
        raise ValueError("X10 static-bundle preflight is not authoritative for the fixture modset")
    return value["bundle_id"]


def validate_action_receipt(value, profile, profile_sha, artifacts):
    if (
        value.get("schema") != "hoi4.x10.authenticated-owner-fixture-action-receipt.v1"
        or value.get("status") != "COMPLETED"
        or value.get("executable_sha256") != EXE_SHA256
        or value.get("collector_sha256") != artifacts["collector_dll"]
        or value.get("profile_sha256") != profile_sha
        or value.get("fixture_nonce_sha256") != profile["fixture_nonce_sha256"]
        or value.get("product_modset_sha256") != profile["product_modset_sha256"]
        or value.get("client_size") != [profile["expected_client_width"], profile["expected_client_height"]]
        or value.get("production_activation_authorized") is not False
        or value.get("semantic_authority") != "Input dispatch only; exact D45E verifier confirms every semantic transaction."
        or value.get("final_pause_byte") != 1
        or not exact_int(value.get("final_game_hour_raw"), 1, (1 << 31) - 1)
        or not exact_int(value.get("process_id"), 1, (1 << 32) - 1)
        or CANONICAL_ID.fullmatch(str(value.get("session_id", ""))) is None
        or CANONICAL_ID.fullmatch(str(value.get("capture_id", ""))) is None
    ):
        raise ValueError("X10 action receipt is not bound to the authenticated fixture")
    try:
        process_start = datetime.fromisoformat(str(value.get("process_start_utc", "")).replace("Z", "+00:00"))
    except ValueError:
        raise ValueError("X10 action receipt process start is not an ISO-8601 timestamp") from None
    if process_start.tzinfo is None or process_start.utcoffset().total_seconds() != 0:
        raise ValueError("X10 action receipt process start is not UTC")
    steps = value.get("steps")
    if not isinstance(steps, list) or len(steps) != len(profile["actions"]):
        raise ValueError("X10 action receipt step count differs from the action profile")
    for expected, observed in zip(profile["actions"], steps):
        if (
            not isinstance(observed, dict)
            or observed.get("label") != expected.get("label")
            or observed.get("kind") != expected.get("kind")
            or observed.get("completed") is not True
            or observed.get("semantic_effect_confirmed") is not False
        ):
            raise ValueError("X10 action receipt/profile sequence mismatch")
        kind = expected["kind"]
        if kind == "wait_for_loaded_paused_save":
            loaded = observed.get("loaded_save")
            if (
                not isinstance(loaded, dict)
                or loaded.get("game_hour_raw") != profile["expected_game_hour_raw"]
                or loaded.get("pause_byte") != 1
                or loaded.get("singletons_stable") is not True
            ):
                raise ValueError("X10 action receipt lacks exact paused-save readiness")
        elif kind == "console_command":
            command = expected.get("command")
            if (
                not isinstance(command, str)
                or observed.get("command_sha256") != sha256_bytes(command.encode("utf-8"))
            ):
                raise ValueError("X10 console action receipt does not bind the frozen command")
        elif kind == "click":
            if observed.get("client_point") != [expected.get("x"), expected.get("y")]:
                raise ValueError("X10 click action receipt does not bind the frozen client point")
        elif kind == "keypress":
            if observed.get("key") != str(expected.get("key", "")).lower():
                raise ValueError("X10 keypress action receipt does not bind the frozen key")
        elif kind == "advance_hours":
            clock = observed.get("clock")
            if (
                not isinstance(clock, dict)
                or clock.get("requested_hours") != expected.get("hours")
                or clock.get("repaused") is not True
                or clock.get("speed_index") != 0
                or clock.get("exact_target") is not True
                or not exact_int(clock.get("game_hour_before"), 1, (1 << 31) - 1)
                or not exact_int(clock.get("game_hour_after"), 1, (1 << 31) - 1)
                or clock["game_hour_after"] != clock["game_hour_before"] + clock["requested_hours"]
            ):
                raise ValueError("X10 advancing-hour receipt is incomplete")


def validate_manifest(value, profile, action, artifacts, bundle_id):
    if (
        value.get("schema") != "native_collector_session_v1"
        or value.get("stream_format") != "compact"
        or value.get("scheduled_domains") != [DOMAIN]
        or any(value.get(field) != [] for field in ("collector_control_errors", "parse_errors", "ordering_errors"))
        or value.get("session_id") != action["session_id"]
        or value.get("capture_id") != action["capture_id"]
        or value.get("pid") != action["process_id"]
    ):
        raise ValueError("X10 capture manifest is not the exact error-free D45 session")
    modset = value.get("modset")
    game = modset.get("game") if isinstance(modset, dict) else None
    if not isinstance(game, dict) or game.get("exe_sha256") != EXE_SHA256 or modset.get("resolved") is not True or modset.get("errors") != []:
        raise ValueError("X10 capture executable/modset identity is unresolved")
    early = value.get("early_render_capture_receipt")
    launch = value.get("early_render_launch_manifest")
    if (
        not isinstance(early, dict)
        or not isinstance(launch, dict)
        or early.get("executable_sha256") != EXE_SHA256
        or launch.get("executable_sha256") != EXE_SHA256
        or early.get("collector_sha256") != artifacts["collector_dll"]
        or launch.get("collector_sha256") != artifacts["collector_dll"]
        or early.get("process_id") != value.get("pid")
        or early.get("created_suspended") is not True
    ):
        raise ValueError("X10 capture lacks authenticated suspended early launch")
    compact = value.get("compact_stream")
    domain = compact.get("per_domain", {}).get(DOMAIN) if isinstance(compact, dict) else None
    if (
        not isinstance(compact, dict)
        or compact.get("decodable") is not True
        or compact.get("expected_domains") != [DOMAIN]
        or DOMAIN not in compact.get("domains_seen", [])
        or not isinstance(domain, dict)
        or not exact_int(domain.get("data_record_count"), 2, (1 << 64) - 1)
    ):
        raise ValueError("X10 compact stream lacks enough exact D45 records")
    static = value.get("static_bundle")
    if (
        not isinstance(static, dict)
        or static.get("admission") != "passed"
        or static.get("verification_phase") != "post_capture_finalized"
        or static.get("token_join_status") != "passed"
        or static.get("final_token_gate_satisfied") is not True
        or static.get("bundle_id") != bundle_id
        or static.get("product_modset_sha256") != profile["product_modset_sha256"]
    ):
        raise ValueError("X10 final static-bundle/token admission failed")
    provenance = static.get("capture_provenance")
    storage = value.get("stream_storage")
    if (
        not isinstance(provenance, dict)
        or not isinstance(storage, dict)
        or provenance.get("session_id") != value.get("session_id")
        or provenance.get("capture_id") != value.get("capture_id")
        or provenance.get("stream_sha256") != storage.get("decoded_sha256")
        or not lower_sha(provenance.get("stream_sha256"))
    ):
        raise ValueError("X10 finalized static bundle is not bound to the capture stream")
    replay = value.get("replay_artifact")
    if not isinstance(replay, dict) or replay.get("reason") != "gate_passed" or replay.get("gate_problems") != []:
        raise ValueError("X10 replay artifact did not pass the normal recorder gate")


def read_snapshots(replay: Path, external_manifest):
    try:
        archive = zipfile.ZipFile(replay, "r")
    except (OSError, zipfile.BadZipFile):
        raise ValueError("X10 replay is not a readable ZIP container") from None
    with archive:
        embedded = [info for info in archive.infolist() if not info.is_dir() and info.filename == "recorder/native_session_manifest.json"]
        if len(embedded) != 1:
            raise ValueError("X10 replay lacks exactly one embedded recorder manifest")
        embedded_manifest = load_object(archive.read(embedded[0]), "embedded recorder manifest")
        for field in ("schema", "session_id", "capture_id", "pid", "stream_format", "scheduled_domains", "raw_stream"):
            if embedded_manifest.get(field) != external_manifest.get(field):
                raise ValueError(f"X10 embedded/external manifest mismatch: {field}")
        infos = sorted(
            [info for info in archive.infolist() if not info.is_dir() and info.filename.endswith(".snapshot.json")],
            key=lambda row: row.filename,
        )
        if len(infos) < 2 or len(infos) > MAX_SNAPSHOTS or len({row.filename for row in infos}) != len(infos):
            raise ValueError("X10 replay snapshot roster is invalid")
        result = []
        for info in infos:
            if info.file_size > MAX_SNAPSHOT_BYTES:
                raise ValueError("X10 replay snapshot exceeds the size bound")
            raw = archive.read(info)
            result.append((info.filename, sha256_bytes(raw), load_object(raw, info.filename)))
        return result


def validate_row(row, expected_sequence):
    if not isinstance(row, dict) or row.get("sequence") != expected_sequence:
        raise ValueError("X10 D45E row sequence is not gap-free from one")
    kind = row.get("kind")
    if kind not in ROOTS:
        raise ValueError("X10 D45E row has an unknown kind")
    rva, phase, returned = ROOTS[kind]
    if (
        row.get("hook_rva") != rva
        or row.get("phase") != phase
        or row.get("original_returned") is not returned
        or not exact_int(row.get("thread_id"), 1, (1 << 32) - 1)
        or not exact_int(row.get("event_hour_raw"), 1, (1 << 31) - 1)
        or not exact_int(row.get("owner_instance_id"), 1)
        or not exact_int(row.get("subject_instance_id"), 1)
        or not exact_int(row.get("argument_instance_id"))
        or not exact_int(row.get("owner_key"))
        or not exact_int(row.get("subject_token_hash_fnv1a64"))
        or not exact_int(row.get("return_bits"))
    ):
        raise ValueError("X10 D45E row violates its exact root/phase/scalar contract")
    if kind in TARGETED and row["argument_instance_id"] == 0:
        raise ValueError("X10 targeted decision row lacks the target opaque id")
    if kind not in TARGETED and row["argument_instance_id"] != 0:
        raise ValueError("X10 nontargeted row unexpectedly carries a target opaque id")
    before = row.get("before")
    after = row.get("after")
    for summary in (before, after):
        if (
            not isinstance(summary, dict)
            or not exact_int(summary.get("state_bits"))
            or not exact_int(summary.get("progress_raw"), -(1 << 31), (1 << 31) - 1)
            or not exact_int(summary.get("remaining_raw"), -(1 << 31), (1 << 31) - 1)
        ):
            raise ValueError("X10 D45E row state summary is malformed")
    if returned:
        if row["return_bits"] != 1 or after["state_bits"] & (1 << 63) == 0:
            raise ValueError("X10 return-observed row lacks its exact return markers")
    elif (
        row["return_bits"] != 0
        or before != {"state_bits": 0, "progress_raw": 0, "remaining_raw": 0}
        or after != {"state_bits": 2, "progress_raw": 0, "remaining_raw": 0}
    ):
        raise ValueError("X10 invocation row fabricates post-return state")


def inspect_histories(snapshots, expected_rows):
    envelopes = []
    prefix = []
    epoch = None
    mask = 0
    drain = None
    for member, snapshot_sha, snapshot in snapshots:
        envelope = snapshot.get("_r2_direct_frd_transitions")
        if envelope is None:
            continue
        if not isinstance(envelope, dict):
            raise ValueError("X10 replay D45E envelope is malformed")
        if (
            envelope.get("schema_version") != 3
            or envelope.get("authority") != "exact_a729_d45e_hook_epoch"
            or envelope.get("executable_sha256") != EXE_SHA256
            or envelope.get("legacy_root_manifest_sha256") != LEGACY_MANIFEST
            or envelope.get("on_action_root_manifest_sha256") != ON_ACTION_MANIFEST
            or envelope.get("terminal_root_manifest_sha256") != TERMINAL_MANIFEST
            or envelope.get("cluster_exhaustion_manifest_sha256") != CLUSTER_MANIFEST
            or envelope.get("installed_root_mask") != FULL_MASK
            or envelope.get("producer_complete") is not True
            or envelope.get("evidence_fixture") is not True
        ):
            raise ValueError("X10 replay D45E envelope authority drift")
        current_epoch = envelope.get("hook_epoch")
        current_drain = envelope.get("last_drain_sequence")
        current_mask = envelope.get("observed_kind_mask")
        rows = envelope.get("transitions")
        if (
            not exact_int(current_epoch, 1)
            or not exact_int(current_drain, 1)
            or not exact_int(current_mask, 0, FULL_MASK)
            or not isinstance(rows, list)
        ):
            raise ValueError("X10 replay D45E epoch/drain/mask/history is malformed")
        if epoch is None:
            epoch = current_epoch
        elif current_epoch != epoch:
            raise ValueError("X10 replay D45E hook epoch changed")
        if drain is not None and current_drain <= drain:
            raise ValueError("X10 replay D45E drain sequence is not strictly increasing")
        if current_mask | mask != current_mask:
            raise ValueError("X10 replay D45E observed-kind mask regressed")
        for sequence, row in enumerate(rows, 1):
            validate_row(row, sequence)
        if rows[: len(prefix)] != prefix:
            raise ValueError("X10 replay D45E accumulated history mutated")
        prefix = rows
        mask = current_mask
        drain = current_drain
        envelopes.append({
            "snapshot_member": member,
            "snapshot_sha256": snapshot_sha,
            "hook_epoch": current_epoch,
            "drain_sequence": current_drain,
            "observed_kind_mask": current_mask,
            "row_count": len(rows),
            "rows": rows,
        })
    if len(envelopes) < 2 or envelopes[-1]["observed_kind_mask"] != FULL_MASK:
        raise ValueError("X10 replay never reached the full 0xFFF witness mask")
    if not any((row["observed_kind_mask"] & LEGACY_MASK) == LEGACY_MASK for row in envelopes):
        raise ValueError("X10 replay never witnessed the legacy low-five root mask")
    final = envelopes[-1]
    actual = final["rows"]
    matches = []
    scope_map = {}
    last_index = -1
    for expected in expected_rows:
        candidates = [
            (index, row)
            for index, row in enumerate(actual)
            if row["kind"] == expected["kind"]
            and row["subject_token_hash_fnv1a64"] == expected["token_hash"]
            and row.get("subject_key") == expected["subject_key"]
            and row["owner_key"] == expected["owner_key"]
            and (row["argument_instance_id"] != 0) is expected["target_required"]
        ]
        if len(candidates) != 1:
            raise ValueError(f"X10 fixture row is missing or duplicated: {expected['kind']}:{expected['token']}")
        index, row = candidates[0]
        if index <= last_index:
            raise ValueError("X10 fixture transaction order differs from the frozen action profile")
        last_index = index
        match = {
            "expected_ordinal": expected["ordinal"],
            "action_label": expected["action_label"],
            "history_index": index,
            "sequence": row["sequence"],
            "kind": expected["kind"],
            "authored_token": expected["token"],
            "subject_token_hash_fnv1a64": expected["token_hash"],
            "subject_key": row.get("subject_key"),
            "owner_key": row["owner_key"],
            "argument_instance_id": row["argument_instance_id"],
        }
        if expected["scope_identity"] is not None:
            match["scope_identity"] = expected["scope_identity"]
            match["scope_instance_id"] = row["owner_instance_id"]
            scope_map.setdefault(expected["scope_identity"], []).append(row["owner_instance_id"])
        matches.append(match)
    return {
        "hook_epoch": final["hook_epoch"],
        "first_sequence": actual[0]["sequence"],
        "last_sequence": actual[-1]["sequence"],
        "final_drain_sequence": final["drain_sequence"],
        "observed_kind_mask": final["observed_kind_mask"],
        "last_full_history_snapshot": final["snapshot_member"],
        "last_full_history_snapshot_sha256": final["snapshot_sha256"],
        "clean_final_drain_authority": "recorder successful teardown with empty collector_control_errors",
        "history_row_count": len(actual),
        "fixture_rows": matches,
        "authenticated_scope_identity_map": [
            {"scope_identity": identity, "capture_local_scope_instance_ids": sorted(set(ids))}
            for identity, ids in sorted(scope_map.items())
        ],
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--contract", required=True, type=Path)
    parser.add_argument("--action-profile", required=True, type=Path)
    parser.add_argument("--action-receipt", required=True, type=Path)
    parser.add_argument("--capture-manifest", required=True, type=Path)
    parser.add_argument("--prebuilt-attestation", required=True, type=Path)
    parser.add_argument("--static-bundle-preflight", required=True, type=Path)
    parser.add_argument("--replay", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()
    errors = []
    details = {}
    inputs = []
    try:
        raw = {name: path.resolve().read_bytes() for name, path in (
            ("contract", args.contract),
            ("action_profile", args.action_profile),
            ("action_receipt", args.action_receipt),
            ("capture_manifest", args.capture_manifest),
            ("prebuilt_attestation", args.prebuilt_attestation),
            ("static_bundle_preflight", args.static_bundle_preflight),
        )}
        contract = load_object(raw["contract"], "X10 fixture contract")
        profile = load_object(raw["action_profile"], "X10 action profile")
        action = load_object(raw["action_receipt"], "X10 action receipt")
        manifest = load_object(raw["capture_manifest"], "capture manifest")
        attestation = load_object(raw["prebuilt_attestation"], "prebuilt attestation")
        preflight = load_object(raw["static_bundle_preflight"], "static-bundle preflight")
        validate_contract(contract)
        expected_rows = validate_profile(profile)
        artifacts = validate_attestation(attestation)
        bundle_id = validate_preflight(preflight, profile)
        validate_action_receipt(action, profile, sha256_bytes(raw["action_profile"]), artifacts)
        validate_manifest(manifest, profile, action, artifacts, bundle_id)
        snapshots = read_snapshots(args.replay.resolve(), manifest)
        details = inspect_histories(snapshots, expected_rows)
        process_nonce = "\0".join((
            action["session_id"], action["capture_id"], str(action["process_id"]), action["process_start_utc"]
        )).encode("utf-8")
        details.update({
            "session_id": action["session_id"],
            "capture_id": action["capture_id"],
            "process_launch_nonce_sha256": sha256_bytes(process_nonce),
            "fixture_nonce_sha256": profile["fixture_nonce_sha256"],
            "product_modset_sha256": profile["product_modset_sha256"],
            "action_profile_sha256": sha256_bytes(raw["action_profile"]),
            "action_receipt_sha256": sha256_bytes(raw["action_receipt"]),
            "replay_sha256": sha256_file(args.replay.resolve()),
        })
        for name, value in raw.items():
            inputs.append({"kind": name, "sha256": sha256_bytes(value)})
        inputs.append({"kind": "replay", "sha256": details["replay_sha256"]})
    except (OSError, ValueError, RuntimeError, zipfile.BadZipFile) as error:
        errors.append(str(error))
    except Exception as error:
        errors.append(f"verifier rejected malformed input: {type(error).__name__}")
    accepted = not errors
    receipt = {
        "schema": "hoi4.x10.direct-transition-owner-fixture-verification.v1",
        "status": "PASS" if accepted else "FAIL",
        "production_activation_ready": accepted,
        "portable_pointer_free": accepted,
        "checks": {
            "exact_a729_suspended_early_launch": accepted,
            "central_release_artifacts": accepted,
            "authenticated_fixture_modset_and_actions": accepted,
            "d45e_v3_full_witness_mask": accepted,
            "gap_free_epoch_and_history": accepted,
            "exact_root_rva_phase_and_token_join": accepted,
            "decision_owner_and_target_join": accepted,
            "authenticated_scope_identity_join": accepted,
            "clean_final_empty_drain": accepted,
            "collector_fold_and_replay_parity": accepted,
            "zero_capture_errors": accepted,
        },
        "details": details,
        "errors": errors,
        "inputs": inputs,
    }
    write_create_new(args.out.resolve(), receipt)
    print(f"X10 owner fixture {receipt['status']}: {args.out.resolve()}")
    return 0 if accepted else 1


if __name__ == "__main__":
    raise SystemExit(main())
