#!/usr/bin/env python3
"""Create-new autonomous A31 acquisition admission."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import time
from pathlib import Path

from verify_a31_fixture_matrix import verify_matrix

EXE = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
SAVE = "c8488b05a3b87c718eeff979976537b79e867634d0b4dccb1eef6c5a45362735"


def require(value: bool, message: str) -> None:
    if not value:
        raise ValueError(message)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def load(path: Path) -> dict[str, object]:
    value = json.loads(path.read_text(encoding="utf-8"))
    require(isinstance(value, dict), f"JSON root is not an object: {path}")
    return value


def within(root: Path, path: Path) -> Path:
    resolved = path.resolve()
    require(resolved.parent == root, f"A31 artifact is outside the immutable capture root: {resolved}")
    return resolved


def write_new(path: Path, value: dict[str, object]) -> None:
    body = dict(value)
    canonical = json.dumps(body, sort_keys=True, separators=(",", ":")).encode("utf-8")
    body["admission_body_sha256"] = hashlib.sha256(canonical).hexdigest()
    encoded = (json.dumps(body, indent=2, sort_keys=True) + "\n").encode("utf-8")
    require(not path.exists(), f"refusing to replace A31 admission: {path}")
    temporary = path.with_name(f".{path.name}.{os.getpid()}.{time.time_ns()}.tmp")
    fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        with os.fdopen(fd, "wb") as stream:
            stream.write(encoded)
            stream.flush()
            os.fsync(stream.fileno())
        fd = -1
        os.rename(temporary, path)
    finally:
        if fd >= 0:
            os.close(fd)
        if temporary.exists():
            temporary.unlink()


def verify(args: argparse.Namespace) -> dict[str, object]:
    root = args.capture_root.resolve()
    require(root.is_dir(), "capture root is absent")
    paths = {name: within(root, getattr(args, name).resolve()) for name in (
        "capture_manifest", "static_bundle_preflight", "readiness_receipt",
        "terminal_repause", "action_receipt", "pair_regrade",
    )}
    profile = args.action_profile.resolve()
    attestation = args.prebuilt_attestation.resolve()
    save = args.continue_save.resolve()
    collector = args.collector_dll.resolve()
    recorder = args.recorder_exe.resolve()
    for path in (*paths.values(), profile, attestation, save, collector, recorder):
        require(path.is_file(), f"required A31 artifact is absent: {path}")
    require(sha256(save) == SAVE, "continued save hash mismatch")

    ready = load(paths["readiness_receipt"])
    require(ready.get("schema") == "hoi4-rendering-law-paused-save-readiness-v1"
            and ready.get("executable_sha256") == EXE
            and ready.get("game_hour") == 60830441
            and ready.get("pause") == 1
            and ready.get("exact_expected_game_hour") is True,
            "paused-save readiness receipt is not the frozen D04 boundary")
    repause = load(paths["terminal_repause"])
    require(repause.get("schema") == "hoi4.shared.authenticated-clock-transition.v2"
            and repause.get("action") == "repause"
            and repause.get("executable_sha256") == EXE
            and repause.get("pause_after") == 1
            and repause.get("game_hour_raw_after") == 60830442,
            "terminal A31 repause receipt mismatch")

    action = load(paths["action_receipt"])
    require(action.get("schema") == "hoi4.a31.autonomous-focus-action-receipt.v1"
            and action.get("executable_sha256") == EXE
            and action.get("country_tag") == "SWE"
            and action.get("stable_game_hour_raw") == 60830441
            and action.get("transition_game_hour_raw") == 60830442
            and action.get("profile_sha256") == sha256(profile),
            "autonomous focus action receipt mismatch")
    steps = action.get("steps")
    require(isinstance(steps, list) and [step.get("action") for step in steps] == [
        "close_all_panels_before_install", "switch_country", "open_stable_focus_view",
        "close_stable_focus_view", "apply_focus_transition",
    ], "autonomous focus action sequence mismatch")

    pair = load(paths["pair_regrade"])
    require(pair.get("schema") == "hoi4.a31.owner_runnable_fixture_pair.v1"
            and pair.get("status") == "PASS_STABLE_AND_DISCRIMINATING_TRANSITION"
            and pair.get("production_scheduling_enabled") is False,
            "paired fixture regrade did not pass")
    require(pair.get("action_receipt") == paths["action_receipt"].name
            and pair.get("action_receipt_sha256") == sha256(paths["action_receipt"])
            and pair.get("action_profile_sha256") == sha256(profile),
            "paired fixture action custody mismatch")
    stable_value = Path(str(pair.get("stable_receipt")))
    transition_value = Path(str(pair.get("transition_receipt")))
    stable = within(root, (stable_value if stable_value.is_absolute() else root / stable_value).resolve())
    transition = within(root, (transition_value if transition_value.is_absolute() else root / transition_value).resolve())
    matrix = verify_matrix(stable, transition)
    require(matrix.get("status") == "PASS_STABLE_AND_DISCRIMINATING_TRANSITION"
            and matrix.get("country_tag") == "SWE"
            and matrix.get("stable_frame_hour") == 17030442
            and matrix.get("transition_frame_hour") == 17030443,
            "independent final matrix regrade mismatch")

    manifest = load(paths["capture_manifest"])
    modset = manifest.get("modset")
    game = modset.get("game") if isinstance(modset, dict) else None
    require(manifest.get("schema") == "native_collector_session_v1"
            and isinstance(game, dict) and game.get("exe_sha256") == EXE
            and manifest.get("scheduled_domains") == ["country_identity"]
            and manifest.get("collector_control_errors") == []
            and manifest.get("parse_errors") == []
            and manifest.get("ordering_errors") == [],
            "native session manifest executable or integrity mismatch")
    require(ready.get("process_id") == repause.get("process_id")
            == action.get("process_id") == manifest.get("pid"),
            "readiness/action/repause/native process identity mismatch")
    compact = manifest.get("compact_stream")
    require(isinstance(compact, dict) and compact.get("decodable") is True
            and compact.get("expected_domains") == ["country_identity"]
            and "country_identity" in compact.get("domains_seen", [])
            and int(manifest.get("game_hours_advanced", 0)) >= 1,
            "native capture did not retain the exact country roster across the transition")
    preflight = load(paths["static_bundle_preflight"])
    require(preflight.get("passed") is True, "static bundle preflight did not pass")
    prebuilt = load(attestation)
    artifact_hashes = prebuilt.get("artifact_sha256")
    require(isinstance(artifact_hashes, dict)
            and artifact_hashes.get("collector_dll") == sha256(collector)
            and artifact_hashes.get("recorder_host") == sha256(recorder),
            "prebuilt attestation does not bind the supplied A31 binaries")

    artifacts = {}
    for name, path in {
        **paths, "action_profile": profile, "prebuilt_attestation": attestation,
        "continue_save": save, "collector_dll": collector, "recorder_exe": recorder,
        "stable_receipt": stable, "transition_receipt": transition,
    }.items():
        artifacts[name] = {
            "file": path.name,
            "custody": "capture_root" if path.parent == root else "authenticated_external_input",
            "sha256": sha256(path),
        }
    return {
        "schema": "hoi4.a31.autonomous-national-focus-admission.v1",
        "verdict": "PASS_AUTONOMOUS_STABLE_AND_DISCRIMINATING_TRANSITION",
        "binary_sha256": EXE,
        "continued_save_sha256": SAVE,
        "country_tag": "SWE",
        "stable_game_hour_raw": 60830441,
        "transition_game_hour_raw": 60830442,
        "stable_frame_hour": 17030442,
        "transition_frame_hour": 17030443,
        "changed_focus_count": matrix["changed_focus_count"],
        "changes": matrix["changes"],
        "artifacts": artifacts,
        "pointer_free_promotion_boundary": True,
        "production_scheduling_enabled": False,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--capture-root", type=Path, required=True)
    parser.add_argument("--capture-manifest", type=Path, required=True)
    parser.add_argument("--prebuilt-attestation", type=Path, required=True)
    parser.add_argument("--static-bundle-preflight", type=Path, required=True)
    parser.add_argument("--readiness-receipt", type=Path, required=True)
    parser.add_argument("--terminal-repause", type=Path, required=True)
    parser.add_argument("--action-profile", type=Path, required=True)
    parser.add_argument("--action-receipt", type=Path, required=True)
    parser.add_argument("--pair-regrade", type=Path, required=True)
    parser.add_argument("--continue-save", type=Path, required=True)
    parser.add_argument("--collector-dll", type=Path, required=True)
    parser.add_argument("--recorder-exe", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    try:
        result = verify(args)
        write_new(args.out.resolve(), result)
    except Exception as error:
        print(json.dumps({"status": "FAIL", "error": str(error)}, sort_keys=True))
        return 1
    print(json.dumps({"status": "PASS", "out": str(args.out.resolve())}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
