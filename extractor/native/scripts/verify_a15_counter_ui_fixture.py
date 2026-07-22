#!/usr/bin/env python3
"""Fail-closed admission for exact-a729 current-hour D20 counter UI rows."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import tempfile
import zipfile


EXPECTED_EXE_SHA256 = (
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
)
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
CANONICAL_ID = re.compile(r"^[a-z0-9][a-z0-9_.-]{7,159}$")
MAX_SNAPSHOTS = 10_000
MAX_SNAPSHOT_BYTES = 128 * 1024 * 1024


def exact_int(value, minimum, maximum):
    return (
        isinstance(value, int)
        and not isinstance(value, bool)
        and minimum <= value <= maximum
    )


def lower_sha(value):
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


def load_json(raw: bytes, label: str):
    try:
        value = json.loads(raw.decode("utf-8-sig"))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError(f"{label} is not valid UTF-8 JSON: {error}") from None
    if not isinstance(value, dict):
        raise ValueError(f"{label} root is not an object")
    return value


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
    finally:
        try:
            temporary.unlink()
        except FileNotFoundError:
            pass


def validate_attestation(value):
    if (
        value.get("schema") != "battle-plan.central_rust_prebuilt_artifacts.v1"
        or value.get("status") != "PASS_CENTRAL_RELEASE_PREBUILT_ARTIFACTS"
        or value.get("successful_exit") is not True
    ):
        raise ValueError("central Release artifact attestation is not authoritative")
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
        raise ValueError("artifact attestation is not a full extractor Release")
    rows = value.get("artifacts")
    if not isinstance(rows, list):
        raise ValueError("artifact attestation roster is malformed")
    resolved = {}
    for role, filename in (
        ("recorder_host", "collector_record.exe"),
        ("collector_dll", "collector.dll"),
    ):
        matches = [row for row in rows if isinstance(row, dict) and row.get("role") == role]
        if len(matches) != 1:
            raise ValueError(f"artifact role is missing or duplicated: {role}")
        row = matches[0]
        if Path(str(row.get("path", ""))).name.lower() != filename or not lower_sha(
            row.get("sha256")
        ):
            raise ValueError(f"artifact identity is malformed: {role}")
        resolved[role] = row["sha256"]
    return resolved


def validate_preflight(value):
    if (
        value.get("passed") is not True
        or value.get("verification_phase") != "pre_injection_deferred_allowed"
        or value.get("token_join_status") != "deferred"
        or not lower_sha(value.get("bundle_id"))
        or not lower_sha(value.get("product_modset_sha256"))
        or not exact_int(value.get("artifact_count"), 1, (1 << 31) - 1)
    ):
        raise ValueError("static-bundle preflight is not authoritative")
    return {
        "bundle_id": value["bundle_id"],
        "product_modset_sha256": value["product_modset_sha256"],
        "artifact_count": value["artifact_count"],
    }


def validate_manifest(manifest, preflight, artifacts):
    if manifest.get("schema") != "native_collector_session_v1":
        raise ValueError("capture manifest schema mismatch")
    if manifest.get("scheduled_domains") != ["divisions"]:
        raise ValueError("capture is not exact divisions-only acquisition")
    if manifest.get("stream_format") != "compact":
        raise ValueError("capture is not compact")
    if any(manifest.get(field) != [] for field in ("collector_control_errors", "parse_errors", "ordering_errors")):
        raise ValueError("capture manifest reports control, parse, or ordering errors")
    for field in ("session_id", "capture_id"):
        if not isinstance(manifest.get(field), str) or CANONICAL_ID.fullmatch(manifest[field]) is None:
            raise ValueError(f"capture manifest {field} is noncanonical")
    if not exact_int(manifest.get("pid"), 1, (1 << 32) - 1):
        raise ValueError("capture manifest pid is malformed")
    modset = manifest.get("modset")
    game = modset.get("game") if isinstance(modset, dict) else None
    if (
        not isinstance(modset, dict)
        or modset.get("resolved") is not True
        or modset.get("errors") != []
        or not isinstance(game, dict)
        or game.get("exe_sha256") != EXPECTED_EXE_SHA256
    ):
        raise ValueError("capture modset or executable identity is unresolved")
    compact = manifest.get("compact_stream")
    division = (
        compact.get("per_domain", {}).get("divisions")
        if isinstance(compact, dict)
        else None
    )
    if (
        not isinstance(compact, dict)
        or compact.get("decodable") is not True
        or compact.get("expected_domains") != ["divisions"]
        or "divisions" not in compact.get("domains_seen", [])
        or not isinstance(division, dict)
        or not exact_int(division.get("data_record_count"), 1, (1 << 64) - 1)
    ):
        raise ValueError("compact stream did not carry exact D20 divisions data")
    hourly = manifest.get("hourly_requirement")
    if (
        not isinstance(hourly, dict)
        or hourly.get("required") is not True
        or hourly.get("met") is not True
        or hourly.get("problems") != []
        or not exact_int(hourly.get("requested_game_hours"), 1, (1 << 31) - 1)
        or not exact_int(manifest.get("game_hours_advanced"), 1, (1 << 31) - 1)
        or manifest.get("skipped_game_hours") != 0
    ):
        raise ValueError("capture did not meet its continuous advancing-hour requirement")
    replay_artifact = manifest.get("replay_artifact")
    if (
        not isinstance(replay_artifact, dict)
        or replay_artifact.get("recovery") is not False
        or replay_artifact.get("reason") != "gate_passed"
        or replay_artifact.get("gate_problems") != []
        or replay_artifact.get("written_file") != manifest.get("replay_file")
    ):
        raise ValueError("replay artifact is not the normal gate-passed output")
    launch = manifest.get("early_render_launch_manifest")
    early = manifest.get("early_render_capture_receipt")
    if (
        not isinstance(launch, dict)
        or not isinstance(early, dict)
        or launch.get("executable_sha256") != EXPECTED_EXE_SHA256
        or early.get("executable_sha256") != EXPECTED_EXE_SHA256
        or early.get("process_id") != manifest.get("pid")
        or early.get("created_suspended") is not True
        or early.get("collector_sha256") != artifacts["collector_dll"]
        or launch.get("collector_sha256") != artifacts["collector_dll"]
    ):
        raise ValueError("capture lacks attested suspended early-launch identity")
    completeness = early.get("completeness")
    if not isinstance(completeness, dict) or not completeness or not all(
        value is True for value in completeness.values()
    ):
        raise ValueError("early-launch hook completeness is not exact")
    static_bundle = manifest.get("static_bundle")
    if (
        not isinstance(static_bundle, dict)
        or static_bundle.get("admission") != "passed"
        or static_bundle.get("verification_phase") != "post_capture_finalized"
        or static_bundle.get("token_join_status") != "passed"
        or static_bundle.get("final_token_gate_satisfied") is not True
    ):
        raise ValueError("capture lacks finalized static-bundle admission")
    for field, expected in preflight.items():
        if static_bundle.get(field) != expected:
            raise ValueError(f"preflight/final static-bundle mismatch: {field}")
    provenance = static_bundle.get("capture_provenance")
    storage = manifest.get("stream_storage")
    if (
        not isinstance(provenance, dict)
        or not isinstance(storage, dict)
        or provenance.get("session_id") != manifest.get("session_id")
        or provenance.get("capture_id") != manifest.get("capture_id")
        or provenance.get("stream_sha256") != storage.get("decoded_sha256")
        or not lower_sha(provenance.get("stream_sha256"))
    ):
        raise ValueError("final static bundle is not bound to the capture stream")


def validate_clock_receipt(receipt, manifest, action):
    if (
        receipt.get("schema") != "hoi4.shared.authenticated-clock-transition.v2"
        or receipt.get("action") != action
        or receipt.get("process_id") != manifest.get("pid")
        or receipt.get("session_id") != manifest.get("session_id")
        or receipt.get("capture_id") != manifest.get("capture_id")
        or receipt.get("executable_sha256") != EXPECTED_EXE_SHA256
        or not isinstance(receipt.get("process_start_utc"), str)
        or not receipt.get("process_start_utc")
    ):
        raise ValueError(f"authenticated {action} receipt does not bind the capture")
    for field, expected in EXPECTED_CLOCK_CONTRACT.items():
        if receipt.get(field) != expected:
            raise ValueError(f"authenticated {action} exact clock contract mismatch: {field}")
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
        or not exact_int(receipt.get("speed_raw"), 0, 4)
    ):
        raise ValueError(f"authenticated {action} raw/normalized clock state is malformed")
    if action == "readiness":
        if (
            receipt.get("result") != "AUTHENTICATED_PAUSED_SAVE_READY"
            or receipt.get("pause_before") != 1
            or receipt.get("pause_after") != 1
            or before != EXPECTED_D04_GAME_HOUR_RAW
            or after != before
        ):
            raise ValueError("readiness receipt does not prove the exact paused D04 save")
    elif action == "unpause":
        if (
            receipt.get("result") != "AUTHENTICATED_PAUSED_TO_RUNNING_AND_ADVANCING"
            or receipt.get("pause_before") != 1
            or receipt.get("pause_after") != 0
            or before != EXPECTED_D04_GAME_HOUR_RAW
            or after <= before
        ):
            raise ValueError("unpause receipt does not prove paused-to-running advancement")
    elif (
        receipt.get("result")
        not in ("AUTHENTICATED_RUNNING_TO_PAUSED", "AUTHENTICATED_ALREADY_PAUSED")
        or receipt.get("pause_before") not in (0, 1)
        or receipt.get("pause_after") != 1
        or after < before
        or (receipt.get("pause_before") == 0)
        != (receipt.get("result") == "AUTHENTICATED_RUNNING_TO_PAUSED")
    ):
        raise ValueError("repause receipt does not prove paused cleanup")
    return {
        "action": action,
        "result": receipt["result"],
        "process_start_utc": receipt["process_start_utc"],
        "game_hour_raw_before": before,
        "game_hour_raw_after": after,
        "game_hour_normalized_before": normalized_before,
        "game_hour_normalized_after": normalized_after,
        "speed_raw": receipt["speed_raw"],
    }


def replay_snapshots(path: Path, external_manifest, preflight, artifacts):
    try:
        archive = zipfile.ZipFile(path, "r")
    except (OSError, zipfile.BadZipFile):
        raise ValueError("replay is not a readable ZIP container") from None
    with archive:
        embedded = [
            info
            for info in archive.infolist()
            if not info.is_dir()
            and info.filename == "recorder/native_session_manifest.json"
        ]
        if len(embedded) != 1:
            raise ValueError("replay does not contain exactly one recorder manifest")
        embedded_raw = archive.read(embedded[0])
        embedded_manifest = load_json(embedded_raw, embedded[0].filename)
        validate_manifest(embedded_manifest, preflight, artifacts)
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
                raise ValueError(f"embedded/external manifest mismatch: {field}")
        infos = [
            info
            for info in archive.infolist()
            if not info.is_dir() and info.filename.endswith(".snapshot.json")
        ]
        if not infos or len(infos) > MAX_SNAPSHOTS:
            raise ValueError("replay snapshot roster is empty or exceeds bounds")
        if len({info.filename for info in infos}) != len(infos):
            raise ValueError("replay contains duplicate snapshot names")
        snapshots = []
        for info in sorted(infos, key=lambda row: row.filename):
            if info.file_size > MAX_SNAPSHOT_BYTES:
                raise ValueError("replay snapshot exceeds the size bound")
            raw = archive.read(info)
            snapshots.append((info.filename, sha256_bytes(raw), load_json(raw, info.filename)))
        return snapshots, sha256_bytes(embedded_raw)


def inspect_rows(snapshots):
    witnesses = []
    malformed = []
    for member, snapshot_sha, snapshot in snapshots:
        hour = snapshot.get("game_hour_raw")
        rows = snapshot.get("divisions")
        if rows is None:
            continue
        if not isinstance(rows, list):
            malformed.append(f"{member}: divisions is not a list")
            continue
        for index, row in enumerate(rows):
            if not isinstance(row, dict):
                malformed.append(f"{member}:{index}: division row is not an object")
                continue
            fields = (
                row.get("max_organisation"),
                row.get("counter_supply_status"),
                row.get("counter_training_visible"),
                row.get("counter_dig_in_frame"),
            )
            if all(value is None for value in fields):
                continue
            div_id = row.get("div_id")
            maximum, supply, training, dig_frame = fields
            if (
                not exact_int(div_id, 1, (1 << 32) - 1)
                or isinstance(maximum, bool)
                or not isinstance(maximum, (int, float))
                or maximum <= 0
                or isinstance(supply, bool)
                or not isinstance(supply, (int, float))
                or not 0 <= supply <= 1
                or not isinstance(training, bool)
                or not exact_int(dig_frame, 0, 2)
                or not exact_int(hour, 1, (1 << 31) - 1)
            ):
                malformed.append(f"{member}:{index}: malformed A15-bearing D20 row")
                continue
            witnesses.append(
                {
                    "snapshot_member": member,
                    "snapshot_sha256": snapshot_sha,
                    "game_hour_raw": hour,
                    "division_id": div_id,
                    "max_organisation": maximum,
                    "counter_supply_status": supply,
                    "counter_training_visible": training,
                    "counter_dig_in_frame": dig_frame,
                }
            )
    if malformed:
        raise ValueError(malformed[0])
    if not witnesses:
        raise ValueError(
            "no same-snapshot D20 row carries max organisation, counter supply, "
            "training visibility, and dig-in frame; keep land counters visible"
        )
    return witnesses


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture-manifest", required=True, type=Path)
    parser.add_argument("--prebuilt-attestation", required=True, type=Path)
    parser.add_argument("--static-bundle-preflight", required=True, type=Path)
    parser.add_argument("--readiness-receipt", required=True, type=Path)
    parser.add_argument("--unpause-receipt", required=True, type=Path)
    parser.add_argument("--repause-receipt", required=True, type=Path)
    parser.add_argument("--replay", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()
    errors = []
    details = {}
    inputs = []
    try:
        manifest_raw = args.capture_manifest.resolve().read_bytes()
        attestation_raw = args.prebuilt_attestation.resolve().read_bytes()
        preflight_raw = args.static_bundle_preflight.resolve().read_bytes()
        readiness_raw = args.readiness_receipt.resolve().read_bytes()
        unpause_raw = args.unpause_receipt.resolve().read_bytes()
        repause_raw = args.repause_receipt.resolve().read_bytes()
        manifest = load_json(manifest_raw, "capture manifest")
        artifacts = validate_attestation(load_json(attestation_raw, "attestation"))
        preflight = validate_preflight(load_json(preflight_raw, "static preflight"))
        validate_manifest(manifest, preflight, artifacts)
        readiness = validate_clock_receipt(
            load_json(readiness_raw, "readiness receipt"), manifest, "readiness"
        )
        unpause = validate_clock_receipt(
            load_json(unpause_raw, "unpause receipt"), manifest, "unpause"
        )
        repause = validate_clock_receipt(
            load_json(repause_raw, "repause receipt"), manifest, "repause"
        )
        if not (
            readiness["process_start_utc"]
            == unpause["process_start_utc"]
            == repause["process_start_utc"]
        ):
            raise ValueError("clock receipts do not bind one process-start identity")
        if repause["game_hour_raw_after"] < unpause["game_hour_raw_after"]:
            raise ValueError("repause receipt predates the authenticated unpause proof")
        snapshots, embedded_manifest_sha = replay_snapshots(
            args.replay.resolve(), manifest, preflight, artifacts
        )
        witnesses = inspect_rows(snapshots)
        details = {
            "snapshot_count_inspected": len(snapshots),
            "authenticated_witness_count": len(witnesses),
            "witnesses": witnesses,
            "embedded_capture_manifest_sha256": embedded_manifest_sha,
            "authenticated_paused_save_readiness": readiness,
            "authenticated_unpause": unpause,
            "authenticated_repause": repause,
        }
        for kind, path, raw in (
            ("capture_manifest", args.capture_manifest, manifest_raw),
            ("central_release_attestation", args.prebuilt_attestation, attestation_raw),
            ("static_bundle_preflight", args.static_bundle_preflight, preflight_raw),
            ("authenticated_paused_save_readiness", args.readiness_receipt, readiness_raw),
            ("authenticated_unpause", args.unpause_receipt, unpause_raw),
            ("authenticated_repause", args.repause_receipt, repause_raw),
        ):
            inputs.append({"kind": kind, "sha256": sha256_bytes(raw)})
        inputs.append(
            {
                "kind": "replay",
                "sha256": sha256_file(args.replay.resolve()),
            }
        )
    except (OSError, ValueError, RuntimeError, zipfile.BadZipFile) as error:
        errors.append(str(error))
    except Exception as error:
        errors.append(f"verifier rejected malformed input: {type(error).__name__}")
    accepted = not errors
    receipt = {
        "schema": "hoi4.a15.counter_ui_fixture_verification.v1",
        "status": "PASS" if accepted else "FAIL",
        "production_admission_ready": accepted,
        "checks": {
            "exact_a729_early_launch": accepted,
            "central_release_artifacts": accepted,
            "exact_divisions_only_stream": accepted,
            "continuous_advancing_hours": accepted,
            "finalized_static_bundle": accepted,
            "same_snapshot_d20_counter_ui_join": accepted,
            "authenticated_paused_save_readiness": accepted,
            "authenticated_clock_cleanup": accepted,
        },
        "details": details,
        "errors": errors,
        "inputs": inputs,
    }
    write_json_atomic(args.out.resolve(), receipt)
    print(f"A15 counter UI fixture {receipt['status']}: {args.out.resolve()}")
    return 0 if accepted else 1


if __name__ == "__main__":
    raise SystemExit(main())
