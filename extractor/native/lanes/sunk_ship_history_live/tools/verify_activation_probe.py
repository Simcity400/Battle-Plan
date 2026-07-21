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
import os
from pathlib import Path
from typing import Any

EXPECTED = {
    "binary_id": "hoi4.exe 1.19.2.0.a729",
    "image_base": "0x140000000",
    "executable_sha256": "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad",
    "function_manifest_sha256": "466880cc290e8961dab48c8323346d9d862089d85bbe0f4ba285b924cc959c82",
    "data_artifact_sha256": "5e0ed3e76724032f3e2f4cc5bcc9e5c38347ed862e0489821be8b7bbdd45ca21",
    "writer_sentinel_artifact_sha256": "dccd61e7573317458980fc0796fcba2ba945a434fbacbf3303a588c620307a4a",
    "all_writer_routes_manifest_sha256": "d02f0f1aaf2369db9dc9529062f5e8f59f3f6f751fb9e4c0ab81d47dea1e1de0",
    "csunk_ship_vtable_artifact_sha256": "61140fad7a24c150b60935580374129321719555bb5a4e1db88bd2bcd296937e",
    "csunk_ship_virtuals_manifest_sha256": "755a0011376d06bbd38456a2bb009236d99f4481a9b076789d9d57729c5673fd",
    "attacker_getters_manifest_sha256": "946eff0fb077863ce2dc1f1607ac2c0b9fcfee8f3377662ff14315101c30cbd6",
    "outer_writer_rva": "0x1961dd0",
    "attributed_record_writer_rva": "0x01d2c90",
    "combat_source_record_writer_rva": "0x01d2d10",
    "cause_record_writer_rva": "0x01d2da0",
    "sunk_ship_info_vtable_rva": "0x2941608",
    "outer_entry_sentinel_hex": "4c894c24204c89442418488954241048",
    "record_writer_entry_sentinel_hex": "48895c241048896c2418488974242057",
    "sink_date_record_offset": "0x58",
    "sink_date_owner": "CGameState+0x468",
}


def unsigned_int(value: Any, maximum: int, *, positive: bool = False) -> bool:
    return (
        isinstance(value, int)
        and not isinstance(value, bool)
        and (value > 0 if positive else value >= 0)
        and value <= maximum
    )


def positive_int(value: Any) -> bool:
    return unsigned_int(value, (1 << 64) - 1, positive=True)


def load_single_receipt(path: Path) -> tuple[dict[str, Any], bytes]:
    raw = path.read_bytes()
    lines = [line for line in raw.splitlines() if line.strip()]
    if len(lines) != 1:
        raise ValueError(f"expected exactly one nonempty JSONL receipt, found {len(lines)}")
    value = json.loads(lines[0])
    if not isinstance(value, dict):
        raise ValueError("receipt root is not an object")
    return value, raw


def canonical_windows_path(value: str | Path) -> str:
    normalized = os.path.abspath(os.fspath(value))
    if normalized.startswith("\\\\?\\"):
        normalized = normalized[4:]
    return os.path.normcase(os.path.normpath(normalized))


def verify(
    receipt: dict[str, Any],
    manifest: dict[str, Any],
    collector_dll: Path,
) -> list[str]:
    errors: list[str] = []
    receipt_keys = {
        "schema",
        "authority",
        "evidence_only",
        "process_id",
        "hook_epoch",
        "production_scheduling_enabled",
        "live_fixture_accepted_in_binary",
        "baseline",
        "telemetry",
        "final_member_epoch_publication",
        "events",
        "fixture_contract_satisfied",
        "production_activation_authorized",
        "promotion_note",
    }
    if set(receipt) != receipt_keys:
        errors.append("receipt v2 field set mismatch")
    if receipt.get("schema") != "hoi4.a33_sunk_ship_activation_probe.v2":
        errors.append("schema mismatch")
    authority = receipt.get("authority")
    if not isinstance(authority, dict):
        errors.append("authority is absent or malformed")
    else:
        if set(authority) != {*EXPECTED, "collector_dll_sha256"}:
            errors.append("authority v2 field set mismatch")
        for key, expected in EXPECTED.items():
            if authority.get(key) != expected:
                errors.append(f"authority.{key} mismatch")
        try:
            collector_sha256 = hashlib.sha256(collector_dll.read_bytes()).hexdigest()
        except OSError as exc:
            errors.append(f"collector DLL is unreadable: {exc}")
        else:
            if authority.get("collector_dll_sha256") != collector_sha256:
                errors.append("authority.collector_dll_sha256 mismatch")
    if receipt.get("evidence_only") is not True:
        errors.append("receipt is not explicitly evidence-only")
    if receipt.get("production_scheduling_enabled") is not False:
        errors.append("evidence run used a production-enabled binary")
    if receipt.get("live_fixture_accepted_in_binary") is not False:
        errors.append("evidence run used a pre-promoted live-fixture binary")
    if receipt.get("production_activation_authorized") is not False:
        errors.append("raw receipt illegally claims production activation")
    if not unsigned_int(receipt.get("process_id"), 0xFFFFFFFF, positive=True):
        errors.append("receipt process_id is not positive")
    if not positive_int(receipt.get("hook_epoch")):
        errors.append("receipt hook_epoch is not positive")

    if manifest.get("pid") != receipt.get("process_id"):
        errors.append("capture manifest pid does not match the hook receipt")
    manifest_dll = manifest.get("dll")
    if not isinstance(manifest_dll, str):
        errors.append("capture manifest DLL path is absent")
    elif canonical_windows_path(manifest_dll) != canonical_windows_path(collector_dll):
        errors.append("capture manifest DLL path does not match the independently hashed DLL")
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
    required_scheduled_names = {
        "naval_combat_lifecycle_live",
        "naval_combat_members_live",
    }
    scheduled_names = (
        {value for value in scheduled if isinstance(value, str)}
        if isinstance(scheduled, list)
        else set()
    )
    if (
        not isinstance(scheduled, list)
        or len(scheduled) != len(required_scheduled_names)
        or scheduled_names != required_scheduled_names
    ):
        errors.append("capture manifest does not have the exact D49/D50-only schedule")
    compact = manifest.get("compact_stream")
    if not isinstance(compact, dict) or compact.get("decodable") is not True:
        errors.append("capture manifest compact stream is not decodable")

    baseline = receipt.get("baseline")
    if not isinstance(baseline, dict):
        errors.append("baseline receipt is absent or malformed")
    else:
        if set(baseline) != {
            "source",
            "measured_empty",
            "revalidated_empty_after_hook_enable",
        }:
            errors.append("baseline v2 field set mismatch")
        if baseline.get("measured_empty") is not True:
            errors.append("pre-hook CGameState+0x590 history was not measured empty")
        if baseline.get("revalidated_empty_after_hook_enable") is not True:
            errors.append("CGameState+0x590 history was not empty after hook activation")

    telemetry = receipt.get("telemetry")
    if not isinstance(telemetry, dict):
        errors.append("telemetry is absent or malformed")
    else:
        if set(telemetry) != {
            "dropped_unbound",
            "dropped_reentrant",
            "dropped_unreadable",
            "dropped_lock_busy",
            "dropped_overflow",
            "completeness_poisoned",
            "loss_free",
        }:
            errors.append("telemetry v2 field set mismatch")
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
    latest_sink_date = 0
    for index, event in enumerate(events):
        prefix = f"events[{index}]"
        if not isinstance(event, dict):
            errors.append(f"{prefix} is not an object")
            continue
        event_keys = {
            "event_sequence",
            "thread_id",
            "member_epoch_publication_sequence",
            "member_epoch_game_hour_raw",
            "combat_id",
            "combat_generation",
            "victim_member_battle_slot_index",
            "victim_country_id_raw",
            "attacker_country_id_raw",
            "ui_source_country_id_raw",
            "sink_date_raw",
            "assisted_kill",
            "victim_ship_reference_raw",
            "damage_source_reference_raw",
            "task_force_id_raw",
            "terminal_context_raw",
            "cause_record_flag",
            "ship_name",
            "ship_class_name",
            "attribution_source_name",
            "attribution_source_class_name",
        }
        if set(event) != event_keys:
            errors.append(f"{prefix} v2 field set mismatch")
        sequence = event.get("event_sequence")
        if not positive_int(sequence) or sequence <= 1 or sequence <= prior_sequence:
            errors.append(f"{prefix}.event_sequence is not strictly increasing")
        elif isinstance(sequence, int):
            prior_sequence = sequence
        for key, maximum in (
            ("thread_id", 0xFFFFFFFF),
            ("member_epoch_publication_sequence", (1 << 64) - 1),
            ("member_epoch_game_hour_raw", (1 << 64) - 1),
            ("combat_id", 0xFFFFFFFF),
            ("combat_generation", (1 << 64) - 1),
            ("victim_country_id_raw", 0xFFFFFFFF),
            ("attacker_country_id_raw", 0xFFFFFFFF),
            ("sink_date_raw", 0x7FFFFFFF),
        ):
            if not unsigned_int(event.get(key), maximum, positive=True):
                errors.append(f"{prefix}.{key} is not positive")
        ui_source_country = event.get("ui_source_country_id_raw")
        if not unsigned_int(ui_source_country, 0xFFFFFFFF):
            errors.append(f"{prefix}.ui_source_country_id_raw is invalid")
        publication_sequence = event.get("member_epoch_publication_sequence")
        if isinstance(publication_sequence, int) and not isinstance(publication_sequence, bool):
            latest_publication_sequence = max(latest_publication_sequence, publication_sequence)
        publication_hour = event.get("member_epoch_game_hour_raw")
        if isinstance(publication_hour, int) and not isinstance(publication_hour, bool):
            latest_publication_hour = max(latest_publication_hour, publication_hour)
        sink_date = event.get("sink_date_raw")
        if isinstance(sink_date, int) and not isinstance(sink_date, bool):
            latest_sink_date = max(latest_sink_date, sink_date)
            if (
                isinstance(publication_hour, int)
                and not isinstance(publication_hour, bool)
                and sink_date < publication_hour
            ):
                errors.append(f"{prefix}.sink_date_raw predates its D49/D50 publication")
        for key, required in (
            ("victim_ship_reference_raw", True),
            ("damage_source_reference_raw", False),
        ):
            reference = event.get(key)
            if (
                not isinstance(reference, list)
                or len(reference) != 2
                or not all(unsigned_int(word, 0xFFFFFFFF) for word in reference)
                or (required and reference == [0, 0])
            ):
                errors.append(f"{prefix}.{key} is invalid")
        terminal_context = event.get("terminal_context_raw")
        if (
            not isinstance(terminal_context, list)
            or len(terminal_context) != 2
            or not all(unsigned_int(word, 0xFFFFFFFF) for word in terminal_context)
        ):
            errors.append(f"{prefix}.terminal_context_raw is invalid")
        task_force_id = event.get("task_force_id_raw")
        if not unsigned_int(task_force_id, 0xFFFFFFFF):
            errors.append(f"{prefix}.task_force_id_raw is invalid")
        if not isinstance(event.get("cause_record_flag"), bool):
            errors.append(f"{prefix}.cause_record_flag is not boolean")
        elif event.get("cause_record_flag") and event.get("assisted_kill") is True:
            errors.append(f"{prefix} cannot be both cause-record and assisted-kill")
        for key in ("ship_name", "ship_class_name"):
            value = event.get(key)
            if (
                not isinstance(value, str)
                or not value
                or len(value.encode("utf-8")) >= 256
                or "\x00" in value
            ):
                errors.append(f"{prefix}.{key} is empty or invalid")
        for key in ("attribution_source_name", "attribution_source_class_name"):
            value = event.get(key)
            if (
                not isinstance(value, str)
                or len(value.encode("utf-8")) >= 256
                or "\x00" in value
            ):
                errors.append(f"{prefix}.{key} is invalid")
        slot = event.get("victim_member_battle_slot_index")
        if not isinstance(slot, int) or isinstance(slot, bool) or not 0 <= slot <= 4_096:
            errors.append(f"{prefix}.victim_member_battle_slot_index is invalid")
        if not isinstance(event.get("assisted_kill"), bool):
            errors.append(f"{prefix}.assisted_kill is not boolean")

    final_publication = receipt.get("final_member_epoch_publication")
    if not isinstance(final_publication, dict):
        errors.append("final D49/D50 member-epoch publication receipt is absent")
    else:
        if set(final_publication) != {
            "publication_sequence",
            "game_hour_raw",
            "member_bindings",
        }:
            errors.append("final member publication v2 field set mismatch")
        final_sequence = final_publication.get("publication_sequence")
        final_hour = final_publication.get("game_hour_raw")
        bindings = final_publication.get("member_bindings")
        if (
            not unsigned_int(final_sequence, (1 << 64) - 1, positive=True)
            or final_sequence < latest_publication_sequence
        ):
            errors.append("final member publication sequence predates a captured event")
        if (
            not unsigned_int(final_hour, (1 << 64) - 1, positive=True)
            or final_hour < latest_publication_hour
        ):
            errors.append("final member publication hour predates a captured event")
        if (
            not unsigned_int(final_hour, (1 << 64) - 1, positive=True)
            or final_hour < latest_sink_date
        ):
            errors.append("final member publication hour predates a sink date")
        if not unsigned_int(bindings, (1 << 64) - 1):
            errors.append("final member publication binding count is invalid")

    mechanically_satisfied = not errors
    if receipt.get("fixture_contract_satisfied") is not mechanically_satisfied:
        errors.append("embedded fixture_contract_satisfied disagrees with recomputation")
    return errors


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("receipt", type=Path)
    parser.add_argument("--capture-manifest", type=Path, required=True)
    parser.add_argument("--collector-dll", type=Path, required=True)
    parser.add_argument("--out", type=Path)
    args = parser.parse_args()
    try:
        receipt, raw = load_single_receipt(args.receipt)
        manifest = json.loads(args.capture_manifest.read_text(encoding="utf-8"))
        if not isinstance(manifest, dict):
            raise ValueError("capture manifest root is not an object")
        errors = verify(receipt, manifest, args.collector_dll)
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        raw = b""
        errors = [str(exc)]
    result = {
        "schema": "hoi4.a33_sunk_ship_activation_verification.v1",
        "source": str(args.receipt),
        "source_sha256": hashlib.sha256(raw).hexdigest() if raw else None,
        "capture_manifest": str(args.capture_manifest),
        "collector_dll": str(args.collector_dll),
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
