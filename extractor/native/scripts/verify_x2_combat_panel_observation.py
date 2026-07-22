#!/usr/bin/env python3
"""Admit an autonomous exact-a729 X2 combat-panel observation."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import tempfile
from typing import Any

EXE_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
SOURCE = "authenticated_a729_combatview_materializer_1415c9860"
ACTION_SCHEMA = "hoi4.x2.autonomous_combat_panel_actions.v1"
MANIFEST_SCHEMA = "native_collector_session_v1"
CANONICAL_ID = re.compile(r"^[A-Za-z0-9_.-]{8,160}$")
ORDER = [
    "BORDER_CONFLICT_MODIFIER",
    "BM_AIR_SUPPORT",
    "BM_ENEMY_AIR_SUPERIORITY",
    "BM_FORT_MODIFIER",
    "BM_SHORE_BOMBARD",
    "BM_RAILWAY_GUN_BOMBARD",
    "BM_STACKING",
    "BM_MULTIPLE_DIRECTIONS",
    "BM_DECRYPTION",
    "BM_ACTIVE_CODE_CRACKING_BONUSES",
]


def exact_int(value: Any, minimum: int, maximum: int) -> bool:
    return isinstance(value, int) and not isinstance(value, bool) and minimum <= value <= maximum


def lower_sha256(value: Any) -> bool:
    return isinstance(value, str) and re.fullmatch(r"[0-9a-f]{64}", value) is not None


def sha256_bytes(payload: bytes) -> str:
    return hashlib.sha256(payload).hexdigest()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def load_object(path: Path, label: str) -> tuple[dict[str, Any], bytes]:
    try:
        raw = path.read_bytes()
        value = json.loads(raw.decode("utf-8-sig"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError(f"{label} is not readable UTF-8 JSON: {error}") from None
    if not isinstance(value, dict):
        raise ValueError(f"{label} root is not an object")
    return value, raw


def write_json_create_new_atomic(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.exists():
        raise FileExistsError(f"refusing to replace immutable X2 receipt: {path}")
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
        if path.exists():
            raise FileExistsError(f"refusing to replace immutable X2 receipt: {path}")
        os.replace(temporary, path)
    finally:
        try:
            temporary.unlink()
        except FileNotFoundError:
            pass


def validate_manifest(manifest: dict[str, Any]) -> dict[str, Any]:
    if manifest.get("schema") != MANIFEST_SCHEMA:
        raise ValueError("capture manifest schema mismatch")
    session_id = manifest.get("session_id")
    capture_id = manifest.get("capture_id")
    if not isinstance(session_id, str) or CANONICAL_ID.fullmatch(session_id) is None:
        raise ValueError("capture manifest session_id is noncanonical")
    if not isinstance(capture_id, str) or CANONICAL_ID.fullmatch(capture_id) is None:
        raise ValueError("capture manifest capture_id is noncanonical")
    if not exact_int(manifest.get("pid"), 1, (1 << 32) - 1):
        raise ValueError("capture manifest pid is malformed")
    if manifest.get("stream_format") != "compact":
        raise ValueError("X2 observation is not carried by the compact production stream")
    if manifest.get("collector_control_errors") != [] or manifest.get("parse_errors") != []:
        raise ValueError("capture manifest reports collector-control or parse errors")
    if manifest.get("ordering_errors") != []:
        raise ValueError("capture manifest reports ordering errors")
    modset = manifest.get("modset")
    game = modset.get("game") if isinstance(modset, dict) else None
    if (
        not isinstance(modset, dict)
        or modset.get("resolved") is not True
        or modset.get("errors") != []
        or not isinstance(game, dict)
        or game.get("exe_sha256") != EXE_SHA256
    ):
        raise ValueError("capture manifest lacks the exact-a729 resolved product modset")
    compact = manifest.get("compact_stream")
    if not isinstance(compact, dict) or compact.get("decodable") is not True:
        raise ValueError("capture compact stream is not decodable")
    return {"session_id": session_id, "capture_id": capture_id, "pid": manifest["pid"]}


def validate_attestation(attestation: dict[str, Any]) -> dict[str, str]:
    if (
        attestation.get("schema") != "battle-plan.central_rust_prebuilt_artifacts.v1"
        or attestation.get("status") != "PASS_CENTRAL_RELEASE_PREBUILT_ARTIFACTS"
        or attestation.get("successful_exit") is not True
    ):
        raise ValueError("central production-artifact attestation did not pass")
    central = attestation.get("central_validation")
    source = attestation.get("source_input")
    if (
        not isinstance(central, dict)
        or central.get("mode") != "Release"
        or central.get("requested_workspace") not in ("All", "Extractor")
        or "Extractor" not in central.get("validated_workspaces", [])
        or central.get("workspace_package_selection") != ["--workspace"]
        or not isinstance(source, dict)
        or not lower_sha256(source.get("sha256"))
    ):
        raise ValueError("central production-artifact attestation is not a full Extractor Release")
    rows = attestation.get("artifacts")
    matches = [row for row in rows if isinstance(row, dict) and row.get("role") == "collector_dll"] if isinstance(rows, list) else []
    if len(matches) != 1 or not lower_sha256(matches[0].get("sha256")):
        raise ValueError("central production-artifact attestation has no unique collector_dll")
    return {"collector_sha256": matches[0]["sha256"], "source_input_sha256": source["sha256"]}


def validate_action_profile(profile: dict[str, Any]) -> None:
    if (
        profile.get("schema") != "hoi4.x2.autonomous_combat_panel_action_profile.v1"
        or profile.get("status") != "FROZEN"
        or profile.get("program_sha256") != EXE_SHA256
        or profile.get("profile_id") != "d04_active_land_battle_bubbles_1920x1080_v1"
        or profile.get("client_size") != [1920, 1080]
    ):
        raise ValueError("frozen X2 action profile identity is malformed")
    actions = profile.get("actions")
    expected_points = [[1009, 154], [1077, 154], [1073, 800], [1107, 818], [1135, 841]]
    if not isinstance(actions, list) or len(actions) != len(expected_points):
        raise ValueError("frozen X2 action profile candidate roster is malformed")
    for ordinal, (action, point) in enumerate(zip(actions, expected_points)):
        if (
            not isinstance(action, dict)
            or action.get("ordinal") != ordinal
            or action.get("kind") != "select_and_hold_land_battle_panel"
            or action.get("client_point") != point
            or action.get("click_hold_milliseconds") != 80
            or not exact_int(action.get("minimum_materialization_dwell_milliseconds"), 2000, 30000)
            or not exact_int(action.get("minimum_game_hours_held"), 2, 24)
        ):
            raise ValueError(f"frozen X2 action profile candidate {ordinal} is malformed")


def validate_readiness(
    readiness: dict[str, Any], manifest: dict[str, Any], action_profile_sha256: str
) -> dict[str, Any]:
    if (
        readiness.get("schema") != "hoi4.x2.autonomous_combat_panel_ready.v1"
        or readiness.get("status") != "READY_SPEED5_UNPAUSED"
        or readiness.get("process_id") != manifest["pid"]
        or readiness.get("session_id") != manifest["session_id"]
        or readiness.get("capture_id") != manifest["capture_id"]
        or readiness.get("executable_sha256") != EXE_SHA256
        or readiness.get("selection_profile_sha256") != action_profile_sha256
        or readiness.get("game_speed_raw") != 4
        or readiness.get("paused") is not False
        or readiness.get("window_owner_process_id") != manifest["pid"]
        or readiness.get("foreground_owner_process_id") != manifest["pid"]
        or not exact_int(readiness.get("game_hour_raw"), 1, (1 << 31) - 1)
    ):
        raise ValueError("autonomous X2 exact speed-5 readiness receipt is malformed")
    return {"game_hour_raw": readiness["game_hour_raw"], "game_speed_raw": 4}


def action_sequence_sha256(actions: list[dict[str, Any]]) -> str:
    canonical = []
    for action in actions:
        point = action.get("client_point")
        screen = action.get("screen_point")
        canonical.append(
            "|".join(
                [
                    str(action.get("ordinal")),
                    str(point[0]),
                    str(point[1]),
                    str(screen[0]),
                    str(screen[1]),
                    str(action.get("game_hour_raw_before")),
                    str(action.get("game_hour_raw_after")),
                    str(action.get("method")),
                    str(action.get("click_hold_milliseconds")),
                    str(action.get("materialization_dwell_milliseconds")),
                    str(action.get("required_hold_game_hours")),
                ]
            )
        )
    return sha256_bytes(";".join(canonical).encode("utf-8"))


def validate_action(
    action: dict[str, Any], manifest: dict[str, Any], attestation: dict[str, str], attestation_sha256: str
) -> dict[str, Any]:
    if action.get("schema") != ACTION_SCHEMA or action.get("status") != "PASS":
        raise ValueError("autonomous X2 action receipt did not pass")
    for field, expected in (
        ("executable_sha256", EXE_SHA256),
        ("session_id", manifest["session_id"]),
        ("capture_id", manifest["capture_id"]),
        ("process_id", manifest["pid"]),
        ("acquisition_owner", "machine"),
        ("selection_profile", "d04_active_land_battle_bubbles_1920x1080_v1"),
    ):
        if action.get(field) != expected:
            raise ValueError(f"autonomous X2 action receipt {field} mismatch")
    if action.get("client_size") != [1920, 1080] or action.get("window_owner_process_id") != manifest["pid"]:
        raise ValueError("autonomous X2 action receipt window identity mismatch")
    if (
        action.get("collector_loaded") is not True
        or action.get("collector_sha256") != attestation["collector_sha256"]
        or action.get("central_release_attestation_sha256") != attestation_sha256
    ):
        raise ValueError("autonomous X2 action receipt lacks an authenticated loaded collector")
    if not lower_sha256(action.get("selection_profile_sha256")):
        raise ValueError("autonomous X2 action receipt lacks its frozen profile identity")
    if action.get("paused_before") is not True or action.get("paused_after") is not True:
        raise ValueError("autonomous X2 action receipt did not bracket actions while paused")
    actions = action.get("actions")
    expected_points = [[1009, 154], [1077, 154], [1073, 800], [1107, 818], [1135, 841]]
    if not isinstance(actions, list) or len(actions) != len(expected_points):
        raise ValueError("autonomous X2 action roster is absent or unbounded")
    prior_after = None
    for ordinal, row in enumerate(actions):
        if not isinstance(row, dict) or row.get("ordinal") != ordinal:
            raise ValueError("autonomous X2 action ordinal is malformed")
        point = row.get("client_point")
        screen = row.get("screen_point")
        if (
            not isinstance(point, list)
            or len(point) != 2
            or point != expected_points[ordinal]
            or not isinstance(screen, list)
            or len(screen) != 2
            or not exact_int(screen[0], -(1 << 31), (1 << 31) - 1)
            or not exact_int(screen[1], -(1 << 31), (1 << 31) - 1)
            or not exact_int(point[0], 0, 1919)
            or not exact_int(point[1], 0, 1079)
            or row.get("method") != "physical_setcursorpos_mouse_event"
            or row.get("foreground_owner_process_id") != manifest["pid"]
            or row.get("window_owner_process_id") != manifest["pid"]
            or row.get("cursor_confirmed") is not True
            or row.get("click_hold_milliseconds") != 80
            or not exact_int(row.get("materialization_dwell_milliseconds"), 4000, 30000)
        ):
            raise ValueError("autonomous X2 candidate action is not physically authenticated")
        before = row.get("game_hour_raw_before")
        after = row.get("game_hour_raw_after")
        hold_hours = row.get("required_hold_game_hours")
        qpc_before = row.get("qpc_ticks_before")
        qpc_after = row.get("qpc_ticks_after")
        if (
            not exact_int(before, 1, (1 << 31) - 1)
            or not exact_int(hold_hours, 1, 24)
            or not exact_int(after, before, (1 << 31) - 1)
            or not exact_int(qpc_before, 1, (1 << 63) - 1)
            or not exact_int(qpc_after, qpc_before, (1 << 63) - 1)
        ):
            raise ValueError("autonomous X2 candidate clock interval is malformed")
        if prior_after is not None and before < prior_after:
            raise ValueError("autonomous X2 candidate hours are not monotonic")
        prior_after = after
    expected = action_sequence_sha256(actions)
    if action.get("action_sequence_sha256") != expected:
        raise ValueError("autonomous X2 action sequence seal mismatch")
    if action.get("game_hour_raw_after", 0) < actions[-1]["game_hour_raw_after"]:
        raise ValueError("autonomous X2 final clock predates the candidate actions")
    required_hold = max(row["required_hold_game_hours"] for row in actions)
    if action.get("game_hour_raw_after", 0) < actions[0]["game_hour_raw_before"] + required_hold:
        raise ValueError("autonomous X2 candidate sweep was not held across the required game hours")
    return {
        "selection_profile": action["selection_profile"],
        "candidate_count": len(actions),
        "first_game_hour_raw": actions[0]["game_hour_raw_before"],
        "last_game_hour_raw": actions[-1]["game_hour_raw_after"],
        "action_sequence_sha256": expected,
        "collector_sha256": action["collector_sha256"],
        "selection_profile_sha256": action["selection_profile_sha256"],
    }


def admit_side(side: dict[str, Any]) -> dict[str, Any] | None:
    if side.get("panel_observation_source") != SOURCE:
        return None
    sequence = side.get("panel_capture_sequence")
    if not exact_int(sequence, 1, (1 << 64) - 1):
        raise ValueError("panel capture sequence is not a positive u64")
    leader_status = side.get("leader_status")
    leader_id = side.get("leader_id")
    if leader_status == "exact":
        if not exact_int(leader_id, 1, (1 << 32) - 1):
            raise ValueError("exact leader status lacks a positive u32 leader id")
    elif leader_status == "exact_none":
        if leader_id is not None:
            raise ValueError("exact-none leader status carries an id")
    else:
        raise ValueError("authenticated panel row has unknown leader status")
    if side.get("effective_displayed_modifiers_status") != "exact":
        raise ValueError("authenticated panel row lacks exact modifier status")
    modifiers = side.get("effective_displayed_modifiers")
    if not isinstance(modifiers, list) or any(not isinstance(value, str) for value in modifiers):
        raise ValueError("exact displayed modifiers are not a string list")
    try:
        ranks = [ORDER.index(token) for token in modifiers]
    except ValueError as error:
        raise ValueError(f"unknown displayed modifier token: {error}") from None
    if ranks != sorted(set(ranks)):
        raise ValueError("displayed modifiers are duplicate or out of engine order")
    return {
        "capture_sequence": sequence,
        "leader_status": leader_status,
        "leader_id": leader_id,
        "effective_displayed_modifiers": modifiers,
        "source": SOURCE,
    }


def inspect_snapshot(document: dict[str, Any], source_name: str) -> list[dict[str, Any]]:
    combats = document.get("combats")
    if combats is None:
        return []
    if not isinstance(combats, list):
        raise ValueError(f"{source_name}: combats is not a list or null")
    admitted: list[dict[str, Any]] = []
    for index, combat in enumerate(combats):
        if not isinstance(combat, dict) or combat.get("kind") != "land":
            continue
        combat_id = combat.get("combat_id")
        detail = combat.get("land_combat_detail_events_live")
        if not isinstance(detail, dict):
            continue
        generation = detail.get("generation")
        if not exact_int(combat_id, 1, (1 << 32) - 1) or not exact_int(generation, 1, (1 << 64) - 1):
            raise ValueError(f"{source_name}: combat {index} has malformed active identity")
        if detail.get("side_detail_completeness") not in ("both", "both_counts_incomplete"):
            continue
        sides: dict[str, dict[str, Any]] = {}
        for side_name in ("attacker", "defender"):
            side = detail.get(side_name)
            if not isinstance(side, dict):
                raise ValueError(f"{source_name}: active combat {combat_id}:{generation} lacks {side_name}")
            row = admit_side(side)
            if row is not None:
                row["side"] = side_name
                sides[side_name] = row
        # The game materializes both side panels from one selected combat. A
        # one-sided observation cannot prove that exact selected/present join.
        if set(sides) == {"attacker", "defender"}:
            if sides["attacker"]["capture_sequence"] == sides["defender"]["capture_sequence"]:
                raise ValueError("attacker and defender panel rows reuse a capture sequence")
            admitted.append(
                {
                    "snapshot_member": source_name,
                    "combat_id": combat_id,
                    "generation": generation,
                    "side_detail_completeness": detail["side_detail_completeness"],
                    "attacker": sides["attacker"],
                    "defender": sides["defender"],
                }
            )
    return admitted


def verify(
    snapshot_root: Path,
    manifest_path: Path,
    action_path: Path,
    readiness_path: Path,
    action_profile_path: Path,
    attestation_path: Path,
) -> dict[str, Any]:
    if not snapshot_root.is_dir():
        raise ValueError(f"snapshot directory does not exist: {snapshot_root}")
    manifest, manifest_raw = load_object(manifest_path, "capture manifest")
    manifest_identity = validate_manifest(manifest)
    attestation, attestation_raw = load_object(attestation_path, "central Release attestation")
    attestation_identity = validate_attestation(attestation)
    action_profile, action_profile_raw = load_object(action_profile_path, "frozen X2 action profile")
    validate_action_profile(action_profile)
    readiness, readiness_raw = load_object(readiness_path, "autonomous X2 readiness receipt")
    readiness_identity = validate_readiness(
        readiness, manifest_identity, sha256_bytes(action_profile_raw)
    )
    action, action_raw = load_object(action_path, "autonomous X2 action receipt")
    if action.get("selection_profile_sha256") != sha256_bytes(action_profile_raw):
        raise ValueError("autonomous X2 action receipt differs from the supplied frozen profile")
    action_identity = validate_action(
        action, manifest_identity, attestation_identity, sha256_bytes(attestation_raw)
    )
    files = sorted(snapshot_root.rglob("*.json"))
    if not files:
        raise ValueError("acceptance snapshot directory contains no JSON snapshots")
    admitted: list[dict[str, Any]] = []
    evidence_files: list[dict[str, Any]] = []
    for path in files:
        value, _ = load_object(path, f"snapshot {path.name}")
        file_rows = inspect_snapshot(value, path.relative_to(snapshot_root).as_posix())
        if file_rows:
            admitted.extend(file_rows)
            evidence_files.append(
                {
                    "path": path.relative_to(snapshot_root).as_posix(),
                    "sha256": sha256(path),
                    "selected_combat_count": len(file_rows),
                }
            )
    if not admitted:
        raise ValueError(
            "no both-side authenticated X2 selected combat-panel observation found after autonomous actions"
        )
    identities = sorted({(row["combat_id"], row["generation"]) for row in admitted})
    return {
        "schema": "hoi4.x2.combat_panel_observation.verification.v2",
        "status": "PASS",
        "production_admission_ready": True,
        "program_sha256": EXE_SHA256,
        "source": SOURCE,
        "selected_combat_count": len(identities),
        "selected_combat_identities": [
            {"combat_id": combat_id, "generation": generation}
            for combat_id, generation in identities
        ],
        "both_side_observation_count": len(admitted),
        "evidence_files": evidence_files,
        "observations": admitted,
        "autonomous_action": action_identity,
        "autonomous_readiness": readiness_identity,
        "inputs": [
            {"kind": "capture_manifest", "path": manifest_path.name, "sha256": sha256_bytes(manifest_raw)},
            {"kind": "autonomous_action_receipt", "path": action_path.name, "sha256": sha256_bytes(action_raw)},
            {"kind": "autonomous_readiness_receipt", "path": readiness_path.name, "sha256": sha256_bytes(readiness_raw)},
            {"kind": "central_release_attestation", "path": attestation_path.name, "sha256": sha256_bytes(attestation_raw)},
            {"kind": "frozen_action_profile", "path": action_profile_path.name, "sha256": sha256_bytes(action_profile_raw)},
        ],
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--snapshots", type=Path, required=True)
    parser.add_argument("--capture-manifest", type=Path, required=True)
    parser.add_argument("--action-receipt", type=Path, required=True)
    parser.add_argument("--readiness-receipt", type=Path, required=True)
    parser.add_argument("--action-profile", type=Path, required=True)
    parser.add_argument("--prebuilt-attestation", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    out = args.out.resolve()
    try:
        result = verify(
            args.snapshots.resolve(),
            args.capture_manifest.resolve(),
            args.action_receipt.resolve(),
            args.readiness_receipt.resolve(),
            args.action_profile.resolve(),
            args.prebuilt_attestation.resolve(),
        )
    except Exception as error:
        result = {
            "schema": "hoi4.x2.combat_panel_observation.verification.v2",
            "status": "FAIL",
            "production_admission_ready": False,
            "errors": [str(error)],
        }
        try:
            write_json_create_new_atomic(out, result)
        except FileExistsError:
            pass
        return 1
    write_json_create_new_atomic(out, result)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
