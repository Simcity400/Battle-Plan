#!/usr/bin/env python3
"""Fail-closed verifier for an owner-run A33 raw activation receipt.

This verifier never changes the production latches. A successful verdict means
only that one raw evidence-only run satisfied the mechanical fixture contract.
Root review, central Rust validation, and a separate source edit are still
required before production scheduling can be enabled.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any

EXPECTED = {
    "binary_id": "hoi4.exe 1.19.2.0.a729",
    "image_base": "0x140000000",
    "executable_sha256": "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad",
    "function_manifest_sha256": "37797c3175af53fc483a26555a492a65ac926b815eb4abd650ff01a9c3d5487e",
    "data_artifact_sha256": "c09db5903cf588bc792bbe63a3a34929316d2136ec0e104f4c99931b86b24369",
    "outer_writer_rva": "0x1961dd0",
    "record_constructor_rva": "0x0c1df70",
    "sunk_ship_info_vtable_rva": "0x2941608",
}


def positive_int(value: Any) -> bool:
    return isinstance(value, int) and not isinstance(value, bool) and value > 0


def load_single_receipt(path: Path) -> tuple[dict[str, Any], bytes]:
    raw = path.read_bytes()
    lines = [line for line in raw.splitlines() if line.strip()]
    if len(lines) != 1:
        raise ValueError(f"expected exactly one nonempty JSONL receipt, found {len(lines)}")
    value = json.loads(lines[0])
    if not isinstance(value, dict):
        raise ValueError("receipt root is not an object")
    return value, raw


def verify(receipt: dict[str, Any], manifest: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    if receipt.get("schema") != "hoi4.a33_sunk_ship_activation_probe.v1":
        errors.append("schema mismatch")
    authority = receipt.get("authority")
    if not isinstance(authority, dict):
        errors.append("authority is absent or malformed")
    else:
        for key, expected in EXPECTED.items():
            if authority.get(key) != expected:
                errors.append(f"authority.{key} mismatch")
    if receipt.get("evidence_only") is not True:
        errors.append("receipt is not explicitly evidence-only")
    if receipt.get("production_scheduling_enabled") is not False:
        errors.append("evidence run used a production-enabled binary")
    if receipt.get("live_fixture_accepted_in_binary") is not False:
        errors.append("evidence run used a pre-promoted live-fixture binary")
    if receipt.get("production_activation_authorized") is not False:
        errors.append("raw receipt illegally claims production activation")
    if not positive_int(receipt.get("process_id")):
        errors.append("receipt process_id is not positive")
    if not positive_int(receipt.get("hook_epoch")):
        errors.append("receipt hook_epoch is not positive")

    if manifest.get("pid") != receipt.get("process_id"):
        errors.append("capture manifest pid does not match the hook receipt")
    modset = manifest.get("modset")
    if not isinstance(modset, dict) or modset.get("resolved") is not True:
        errors.append("capture manifest modset/executable preflight is unresolved")
    else:
        game = modset.get("game")
        if not isinstance(game, dict) or game.get("exe_sha256") != EXPECTED["executable_sha256"]:
            errors.append("capture manifest executable SHA-256 mismatch")
        if modset.get("errors") != []:
            errors.append("capture manifest modset preflight has errors")
    scheduled = manifest.get("scheduled_domains")
    scheduled_names = (
        {value for value in scheduled if isinstance(value, str)}
        if isinstance(scheduled, list)
        else set()
    )
    if not {
        "naval_combat_lifecycle_live",
        "naval_combat_members_live",
    }.issubset(scheduled_names):
        errors.append("capture manifest lacks the exact D49/D50 scheduled domains")
    compact = manifest.get("compact_stream")
    if not isinstance(compact, dict) or compact.get("decodable") is not True:
        errors.append("capture manifest compact stream is not decodable")

    baseline = receipt.get("baseline")
    if not isinstance(baseline, dict):
        errors.append("baseline receipt is absent or malformed")
    else:
        if baseline.get("measured_empty") is not True:
            errors.append("pre-hook CGameState+0x590 history was not measured empty")
        if baseline.get("revalidated_empty_after_hook_enable") is not True:
            errors.append("CGameState+0x590 history was not empty after hook activation")

    telemetry = receipt.get("telemetry")
    if not isinstance(telemetry, dict):
        errors.append("telemetry is absent or malformed")
    else:
        for key in (
            "dropped_unbound",
            "dropped_reentrant",
            "dropped_unreadable",
            "dropped_lock_busy",
            "dropped_overflow",
        ):
            if telemetry.get(key) != 0:
                errors.append(f"telemetry.{key} is not zero")
        if telemetry.get("completeness_poisoned") is not False:
            errors.append("hook completeness was poisoned")
        if telemetry.get("loss_free") is not True:
            errors.append("hook did not declare a loss-free drain")

    events = receipt.get("events")
    if not isinstance(events, list) or not events:
        errors.append("no sunk-ship writer event was observed")
        events = []
    prior_sequence = 0
    latest_publication_sequence = 0
    latest_publication_hour = 0
    for index, event in enumerate(events):
        prefix = f"events[{index}]"
        if not isinstance(event, dict):
            errors.append(f"{prefix} is not an object")
            continue
        sequence = event.get("event_sequence")
        if not positive_int(sequence) or sequence <= prior_sequence:
            errors.append(f"{prefix}.event_sequence is not strictly increasing")
        elif isinstance(sequence, int):
            prior_sequence = sequence
        for key in (
            "thread_id",
            "member_epoch_publication_sequence",
            "member_epoch_game_hour_raw",
            "combat_id",
            "combat_generation",
            "killer_country_id_raw",
            "sink_date_raw",
        ):
            if not positive_int(event.get(key)):
                errors.append(f"{prefix}.{key} is not positive")
        publication_sequence = event.get("member_epoch_publication_sequence")
        if isinstance(publication_sequence, int) and not isinstance(publication_sequence, bool):
            latest_publication_sequence = max(latest_publication_sequence, publication_sequence)
        publication_hour = event.get("member_epoch_game_hour_raw")
        if isinstance(publication_hour, int) and not isinstance(publication_hour, bool):
            latest_publication_hour = max(latest_publication_hour, publication_hour)
        identity = event.get("victim_country_identity_raw")
        if (
            not isinstance(identity, list)
            or len(identity) != 2
            or not all(isinstance(word, int) and not isinstance(word, bool) for word in identity)
            or identity == [0, 0]
        ):
            errors.append(f"{prefix}.victim_country_identity_raw is invalid")
        for key in ("ship_name", "ship_class_name"):
            value = event.get(key)
            if (
                not isinstance(value, str)
                or not value
                or len(value.encode("utf-8")) >= 256
                or "\x00" in value
            ):
                errors.append(f"{prefix}.{key} is empty or invalid")
        slot = event.get("victim_member_battle_slot_index")
        if not isinstance(slot, int) or isinstance(slot, bool) or slot < 0:
            errors.append(f"{prefix}.victim_member_battle_slot_index is invalid")
        if not isinstance(event.get("assisted_kill"), bool):
            errors.append(f"{prefix}.assisted_kill is not boolean")

    final_publication = receipt.get("final_member_epoch_publication")
    if not isinstance(final_publication, dict):
        errors.append("final D49/D50 member-epoch publication receipt is absent")
    else:
        final_sequence = final_publication.get("publication_sequence")
        final_hour = final_publication.get("game_hour_raw")
        bindings = final_publication.get("member_bindings")
        if not positive_int(final_sequence) or final_sequence < latest_publication_sequence:
            errors.append("final member publication sequence predates a captured event")
        if not positive_int(final_hour) or final_hour < latest_publication_hour:
            errors.append("final member publication hour predates a captured event")
        if not isinstance(bindings, int) or isinstance(bindings, bool) or bindings < 0:
            errors.append("final member publication binding count is invalid")

    mechanically_satisfied = not errors
    if receipt.get("fixture_contract_satisfied") is not mechanically_satisfied:
        errors.append("embedded fixture_contract_satisfied disagrees with recomputation")
    return errors


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("receipt", type=Path)
    parser.add_argument("--capture-manifest", type=Path, required=True)
    parser.add_argument("--out", type=Path)
    args = parser.parse_args()
    try:
        receipt, raw = load_single_receipt(args.receipt)
        manifest = json.loads(args.capture_manifest.read_text(encoding="utf-8"))
        if not isinstance(manifest, dict):
            raise ValueError("capture manifest root is not an object")
        errors = verify(receipt, manifest)
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        raw = b""
        errors = [str(exc)]
    result = {
        "schema": "hoi4.a33_sunk_ship_activation_verification.v1",
        "source": str(args.receipt),
        "source_sha256": hashlib.sha256(raw).hexdigest() if raw else None,
        "capture_manifest": str(args.capture_manifest),
        "verdict": "candidate_accepted" if not errors else "rejected",
        "errors": errors,
        "production_activation_authorized": False,
    }
    encoded = json.dumps(result, indent=2, sort_keys=True) + "\n"
    if args.out:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(encoded, encoding="utf-8", newline="\n")
    print(encoded, end="")
    return 0 if not errors else 1


if __name__ == "__main__":
    raise SystemExit(main())
