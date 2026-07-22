#!/usr/bin/env python3
"""Aggregate, immutable admission for the four exact-a729 A28 controller families."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path

import verify_a28_controller_fixture as fixture


EXPECTED = {
    "air_selection_list",
    "mio_aggregated_bonus_smooth_list",
    "mio_variant_history_smooth_list",
    "mio_production_history_smooth_list",
}


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def create_new_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    payload = (json.dumps(value, indent=2, sort_keys=True) + "\n").encode()
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o644)
    try:
        os.write(descriptor, payload)
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--observation", type=Path, action="append", required=True)
    parser.add_argument("--action-receipt", type=Path, required=True)
    parser.add_argument("--profile-verification", type=Path, required=True)
    parser.add_argument("--capture-manifest", type=Path, required=True)
    parser.add_argument("--collector-dll", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()

    fixture.require(len(args.observation) == 4, "exactly four observations are required")
    action_raw = args.action_receipt.read_bytes()
    action = json.loads(action_raw)
    fixture.require(
        action.get("schema") == "hoi4.a28.air-mio-action-receipt.v1"
        and action.get("status") == "PASS"
        and action.get("promotion_authorized") is False
        and action.get("machine_navigation_authenticated") is True
        and action.get("navigation_status") == "AUTHENTICATED_EXACT_D04_ACTION_LAW",
        "foreign A28 action receipt",
    )
    profile_verification_raw = args.profile_verification.read_bytes()
    profile_verification = json.loads(profile_verification_raw)
    fixture.require(
        profile_verification.get("schema") == "hoi4.a28.air-mio-actions.static-verification.v1"
        and profile_verification.get("accepted") is True
        and profile_verification.get("promotion_authorized") is False
        and profile_verification.get("navigation_status") == "AUTHENTICATED_EXACT_D04_ACTION_LAW",
        "foreign A28 action-profile verification",
    )
    fixture.require(
        action.get("profile_static_verification_file") == args.profile_verification.name
        and action.get("profile_static_verification_sha256") == hashlib.sha256(profile_verification_raw).hexdigest(),
        "action receipt does not hash-join its profile verification",
    )
    collector_sha = digest(args.collector_dll)
    fixture.require(action.get("collector_sha256") == collector_sha, "collector hash differs")
    marker_families = {
        marker.get("family")
        for marker in action.get("family_markers", [])
        if isinstance(marker, dict)
    }
    fixture.require(marker_families == EXPECTED, "action receipt lacks the four family markers")

    admitted: list[dict[str, object]] = []
    observed_kinds: set[str] = set()
    for path in args.observation:
        raw = path.read_bytes()
        root = json.loads(raw)
        fixture.require(root.get("schema") == fixture.SCHEMA, "foreign observation schema")
        frame_hour = fixture.exact_int(root.get("frame_hour"), "frame_hour")
        collections = root.get("collections")
        fixture.require(
            isinstance(collections, list) and len(collections) == 1,
            "each acquisition observation must contain exactly one family",
        )
        collection = collections[0]
        fixture.require(isinstance(collection, dict), "collection is not an object")
        kind = collection.get("kind")
        fixture.require(kind in EXPECTED, "foreign A28 acquisition family")
        fixture.require(kind not in observed_kinds, "duplicate A28 acquisition family")
        fixture.validate_authority(root, kind)
        fixture.validate_collection(collection, frame_hour, kind)
        observed_kinds.add(kind)
        admitted.append(
            {
                "kind": kind,
                "file": path.name,
                "sha256": hashlib.sha256(raw).hexdigest(),
                "frame_hour": frame_hour,
                "transition_sequence": root["authority"]["transition_sequence"],
                "producer_record_sha256": root["authority"]["producer_record_sha256"],
            }
        )
    fixture.require(observed_kinds == EXPECTED, "four-family observation set is incomplete")

    receipt = {
        "schema": "hoi4.a28.air-mio-acquisition-admission.v1",
        "status": "PASS",
        "executable_sha256": fixture.EXECUTABLE_SHA256,
        "collector_sha256": collector_sha,
        "capture_manifest": {
            "file": args.capture_manifest.name,
            "sha256": digest(args.capture_manifest),
        },
        "action_receipt": {
            "file": args.action_receipt.name,
            "sha256": hashlib.sha256(action_raw).hexdigest(),
        },
        "profile_verification": {
            "file": args.profile_verification.name,
            "sha256": hashlib.sha256(profile_verification_raw).hexdigest(),
        },
        "observations": sorted(admitted, key=lambda item: str(item["kind"])),
        "portable_pointer_free": True,
        "semantic_authority": "raw controller observations mechanically regraded",
        "promotion_authorized": False,
    }
    create_new_json(args.out, receipt)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (fixture.Rejection, OSError, ValueError, KeyError, TypeError) as error:
        raise SystemExit(f"A28_FAIL_CLOSED: {error}")
