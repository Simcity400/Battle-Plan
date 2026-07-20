#!/usr/bin/env python3
"""Normalize exact D54 fixture observations into a fail-closed carrier.

The input is written by the live harness/operator.  This tool never reads game
memory and never activates a replay feature.  It preserves boundary order,
ordered engine vectors, raw row bytes, raw pointers, and save-oracle ordering.
Even a mechanically complete carrier leaves every production activation false;
promotion remains a separate reviewed source change.
"""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
from pathlib import Path
from typing import Any


OBSERVATION_SCHEMA = "hoi4.d54.exact_fixture_observation.v1"
CARRIER_SCHEMA = "hoi4.d54.exact_fixture_carrier.v1"
EXPECTED_EXE_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
CURRENT_HEADER_LEN = 60
CURRENT_ROW_LEN = 88
STATUS_EXACT = 1

FIXTURE_PHASES = {
    "type2_redraw_delete": ("baseline", "redraw", "delete"),
    "type4_discriminator": ("created",),
    "membership_lifecycle": ("baseline", "assign", "remove"),
    "empty_inactive_manager": ("baseline", "add_member", "remove_member"),
}

ACTIVATION_KEYS = (
    "same_instance_type2_redraw_delete",
    "type4_geometry",
    "complete_membership_equality_lifecycle",
    "empty_inactive_manager_root",
)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    template = sub.add_parser("write-template", help="write an inactive observation template")
    template.add_argument("--out", type=Path, required=True)
    build = sub.add_parser("build", help="build a normalized, activation-false carrier")
    build.add_argument("observation_json", type=Path)
    build.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()

    if args.command == "write-template":
        write_json(args.out, observation_template())
        return 0

    raw_bytes = args.observation_json.read_bytes()
    observation = json.loads(raw_bytes.decode("utf-8"))
    carrier, errors = build_carrier(observation, hashlib.sha256(raw_bytes).hexdigest())
    if errors:
        print(json.dumps({"status": "OBSERVATIONS_REQUIRED", "errors": errors}, indent=2, sort_keys=True))
        return 1
    write_json(args.out, carrier)
    print(
        json.dumps(
            {
                "status": "CARRIER_WRITTEN_ACTIVATION_FALSE",
                "out": str(args.out),
                "receipts": carrier["observation_receipts"],
                "activation": carrier["activation"],
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0


def write_json(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def build_carrier(observation: dict[str, Any], source_sha256: str) -> tuple[dict[str, Any], list[str]]:
    errors = validate_top_level(observation)
    fixtures = observation.get("fixtures") if isinstance(observation.get("fixtures"), dict) else {}
    receipts = {
        "same_instance_type2_redraw_delete": validate_type2(
            fixtures.get("type2_redraw_delete"), errors
        ),
        "type4_discriminator": validate_type4(fixtures.get("type4_discriminator"), errors),
        "complete_membership_equality_lifecycle": validate_membership(
            fixtures.get("membership_lifecycle"), errors
        ),
        "empty_inactive_manager": validate_manager(
            fixtures.get("empty_inactive_manager"), errors
        ),
    }
    activation = {key: False for key in ACTIVATION_KEYS}
    activation["reason"] = (
        "An observation carrier cannot activate production semantics; reviewed evidence and a separate "
        "promotion edit are required."
    )
    carrier = {
        "schema": CARRIER_SCHEMA,
        "source_observation_sha256": source_sha256,
        "binary": copy.deepcopy(observation.get("binary")),
        "capture": copy.deepcopy(observation.get("capture")),
        "observation_receipts": receipts,
        "activation": activation,
        # Keep the complete source object.  Object keys may be sorted at JSON
        # serialization, but all arrays remain in original engine/harness order.
        "raw_observation": copy.deepcopy(observation),
        "ordering_contract": {
            "fixture_boundary_arrays": "source order preserved",
            "path_provinces": "engine order preserved; never sorted or deduplicated",
            "assigned_division_ids": "engine order preserved; equality uses a duplicate-free set view",
            "save_scheduled_member_ids": "save order preserved; equality uses a duplicate-free set view",
            "save_scheduled_member_refs": "typed save references preserved in source order",
            "manager_entries": "bounded-walk enumeration order preserved",
        },
    }
    return carrier, errors


def validate_top_level(observation: Any) -> list[str]:
    errors: list[str] = []
    if not isinstance(observation, dict):
        return ["root must be an object"]
    if observation.get("schema") != OBSERVATION_SCHEMA:
        errors.append(f"schema must be {OBSERVATION_SCHEMA}")
    binary = observation.get("binary")
    if not isinstance(binary, dict):
        errors.append("binary must be an object")
    else:
        if str(binary.get("sha256", "")).lower() != EXPECTED_EXE_SHA256:
            errors.append("binary.sha256 does not match a729 authority")
        if binary.get("image_base") != "0x140000000":
            errors.append("binary.image_base must be 0x140000000")
    capture = observation.get("capture")
    if not isinstance(capture, dict):
        errors.append("capture must be an object")
    else:
        for key in ("single_process", "paused_boundaries", "harness_authenticated"):
            if capture.get(key) is not True:
                errors.append(f"capture.{key} must be true")
        for key in ("session_id", "harness_name", "harness_version"):
            if not isinstance(capture.get(key), str) or not capture[key]:
                errors.append(f"capture.{key} is required")
        if not is_sha256(capture.get("harness_sha256")):
            errors.append("capture.harness_sha256 is required")
        if not positive_int(capture.get("process_id")):
            errors.append("capture.process_id must be positive")
        wire = capture.get("d54_wire")
        if not isinstance(wire, dict):
            errors.append("capture.d54_wire must be an object")
        else:
            if wire.get("domain_id") != 54:
                errors.append("capture.d54_wire.domain_id must be 54")
            if wire.get("header_len") != CURRENT_HEADER_LEN or wire.get("row_len") != CURRENT_ROW_LEN:
                errors.append("capture.d54_wire must declare the current 60/88 layout")
    if observation.get("activation_requested") is not False:
        errors.append("activation_requested must be false")
    fixtures = observation.get("fixtures")
    if not isinstance(fixtures, dict):
        errors.append("fixtures must be an object")
    else:
        for key in FIXTURE_PHASES:
            if key not in fixtures:
                errors.append(f"fixtures.{key} is required")
    return errors


def fixture_boundaries(value: Any, fixture_name: str, errors: list[str]) -> list[dict[str, Any]]:
    prefix = f"fixtures.{fixture_name}"
    if not isinstance(value, dict):
        errors.append(f"{prefix} must be an object")
        return []
    boundaries = value.get("boundaries")
    if not isinstance(boundaries, list):
        errors.append(f"{prefix}.boundaries must be a list")
        return []
    expected = FIXTURE_PHASES[fixture_name]
    phases = tuple(item.get("phase") if isinstance(item, dict) else None for item in boundaries)
    if phases != expected:
        errors.append(f"{prefix}.boundaries phases must be {list(expected)} in that order")
    prior_sequence = -1
    seen_ids: set[str] = set()
    for index, boundary in enumerate(boundaries):
        name = f"{prefix}.boundaries[{index}]"
        if not isinstance(boundary, dict):
            errors.append(f"{name} must be an object")
            continue
        boundary_id = boundary.get("boundary_id")
        if not isinstance(boundary_id, str) or not boundary_id or boundary_id in seen_ids:
            errors.append(f"{name}.boundary_id must be a unique non-empty string")
        else:
            seen_ids.add(boundary_id)
        sequence = boundary.get("sequence")
        if not is_nonnegative_int(sequence) or sequence <= prior_sequence:
            errors.append(f"{name}.sequence must increase strictly in source order")
        elif isinstance(sequence, int):
            prior_sequence = sequence
        if boundary.get("paused") is not True:
            errors.append(f"{name}.paused must be true")
        if not isinstance(boundary.get("boundary_token"), str) or not boundary["boundary_token"]:
            errors.append(f"{name}.boundary_token is required")
    return [item for item in boundaries if isinstance(item, dict)]


def validate_type2(value: Any, errors: list[str]) -> bool:
    start = len(errors)
    boundaries = fixture_boundaries(value, "type2_redraw_delete", errors)
    if len(boundaries) != 3:
        return False
    rows = [order_snapshot(boundary, f"fixtures.type2_redraw_delete.boundaries[{index}]", errors) for index, boundary in enumerate(boundaries)]
    if any(row is None for row in rows):
        return False
    baseline, redraw, deleted = rows
    for index, row in enumerate(rows):
        if row["order_type"] != 2:
            errors.append(f"type2 boundary {index} order_type must be 2")
    if baseline.get("geometry_status") != STATUS_EXACT or redraw.get("geometry_status") != STATUS_EXACT:
        errors.append("type2 baseline and redraw geometry_status must be exact")
    if baseline["deleted"] or redraw["deleted"] or not deleted["deleted"]:
        errors.append("type2 lifecycle must be live, live, tombstone")
    identities = {(row["army_id"], row["engine_order_instance_id"], row["generation"]) for row in rows}
    if len(identities) != 1:
        errors.append("type2 lifecycle must preserve army_id, +0x244 instance id, and generation")
    if boundaries[0]["raw_order"].get("order_pointer") != boundaries[1]["raw_order"].get("order_pointer"):
        errors.append("type2 baseline/redraw must preserve the raw order pointer")
    if baseline["path_provinces"] == redraw["path_provinces"]:
        errors.append("type2 redraw must change the ordered path")
    if deleted["path_provinces"]:
        errors.append("type2 tombstone path must be empty")
    return len(errors) == start


def validate_type4(value: Any, errors: list[str]) -> bool:
    start = len(errors)
    boundaries = fixture_boundaries(value, "type4_discriminator", errors)
    if len(boundaries) != 1:
        return False
    boundary = boundaries[0]
    row = order_snapshot(boundary, "fixtures.type4_discriminator.boundaries[0]", errors)
    if row is None:
        return False
    if row["order_type"] != 4 or row["deleted"]:
        errors.append("type4 discriminator requires one live order_type 4 row")
    raw_candidates = boundary.get("raw_geometry_candidates")
    if not isinstance(raw_candidates, list) or not raw_candidates:
        errors.append("type4 raw_geometry_candidates must preserve at least one raw candidate")
    else:
        offsets = {(item.get("begin_offset"), item.get("size_offset")) for item in raw_candidates if isinstance(item, dict)}
        if ("+0x70", "+0x78") not in offsets:
            errors.append("type4 raw_geometry_candidates must include +0x70/+0x78")
        for index, candidate in enumerate(raw_candidates):
            if not isinstance(candidate, dict) or not is_hex(candidate.get("raw_descriptor_hex")):
                errors.append(f"type4 raw_geometry_candidates[{index}] needs raw_descriptor_hex")
                continue
            ordered_values = candidate.get("ordered_values")
            if not isinstance(ordered_values, list) or not all(positive_int(item) for item in ordered_values):
                errors.append(f"type4 raw_geometry_candidates[{index}].ordered_values must preserve engine order")
    save = boundary.get("save_order")
    validate_save_order(save, boundary.get("boundary_token"), "type4 save_order", errors)
    if isinstance(save, dict):
        if save.get("army_id") != row["army_id"] or save.get("instance_id") != row["engine_order_instance_id"]:
            errors.append("type4 save_order must match the live army and +0x244 order instance identity")
        for key in ("path_provinces", "states"):
            values = save.get(key)
            if not isinstance(values, list) or not all(positive_int(item) for item in values):
                errors.append(f"type4 save_order.{key} must preserve save order")
        if not isinstance(save.get("raw_order_object"), dict) or not save["raw_order_object"]:
            errors.append("type4 save_order.raw_order_object is required")
    return len(errors) == start


def validate_membership(value: Any, errors: list[str]) -> bool:
    start = len(errors)
    boundaries = fixture_boundaries(value, "membership_lifecycle", errors)
    if len(boundaries) != 3:
        return False
    rows = []
    sets = []
    for index, boundary in enumerate(boundaries):
        name = f"fixtures.membership_lifecycle.boundaries[{index}]"
        row = order_snapshot(boundary, name, errors)
        rows.append(row)
        if row is None:
            sets.append(set())
            continue
        live_ids = row["assigned_division_ids"]
        if row["assigned_units_status"] != STATUS_EXACT:
            errors.append(f"{name}.order.assigned_units_status must be exact")
        if len(live_ids) != len(set(live_ids)):
            errors.append(f"{name}.order.assigned_division_ids contains duplicates")
        save = boundary.get("save_order")
        save_ids = validate_save_order(save, boundary.get("boundary_token"), f"{name}.save_order", errors)
        if isinstance(save, dict):
            if save.get("army_id") != row["army_id"] or save.get("instance_id") != row["engine_order_instance_id"]:
                errors.append(f"{name}.save_order must match live army and +0x244 identity")
        if set(live_ids) != set(save_ids):
            errors.append(f"{name} live assigned IDs must equal save scheduled-member IDs")
        sets.append(set(live_ids))
    if any(row is None for row in rows):
        return False
    identities = {(row["army_id"], row["engine_order_instance_id"], row["generation"]) for row in rows}
    if len(identities) != 1:
        errors.append("membership lifecycle must preserve army_id, +0x244 instance id, and generation")
    pointers = {boundary["raw_order"].get("order_pointer") for boundary in boundaries}
    if len(pointers) != 1:
        errors.append("membership lifecycle must preserve the raw order pointer")
    if len(sets[0]) < 2:
        errors.append("membership baseline must contain at least two assigned divisions")
    added = sets[1] - sets[0]
    removed_on_assign = sets[0] - sets[1]
    removed = sets[1] - sets[2]
    added_on_remove = sets[2] - sets[1]
    if len(added) != 1 or removed_on_assign:
        errors.append("membership assign boundary must add exactly one division and remove none")
    if len(removed) != 1 or added_on_remove:
        errors.append("membership remove boundary must remove exactly one division and add none")
    return len(errors) == start


def validate_manager(value: Any, errors: list[str]) -> bool:
    start = len(errors)
    boundaries = fixture_boundaries(value, "empty_inactive_manager", errors)
    if len(boundaries) != 3:
        return False
    fixture_ids = value.get("fixture_army_ids") if isinstance(value, dict) else None
    if not isinstance(fixture_ids, dict):
        errors.append("empty_inactive_manager.fixture_army_ids must be an object")
        return False
    expected = {fixture_ids.get("active"), fixture_ids.get("inactive"), fixture_ids.get("zero_member")}
    if len(expected) != 3 or not all(positive_int(item) for item in expected):
        errors.append("manager fixture requires three distinct positive active/inactive/zero_member army IDs")
    member_sets = []
    manager_pointer = None
    for index, boundary in enumerate(boundaries):
        name = f"fixtures.empty_inactive_manager.boundaries[{index}]"
        manager = boundary.get("manager")
        if not isinstance(manager, dict):
            errors.append(f"{name}.manager must be an object")
            member_sets.append(set())
            continue
        pointer = manager.get("container_pointer")
        if not is_pointer(pointer):
            errors.append(f"{name}.manager.container_pointer must be a nonzero hex pointer")
        if manager_pointer is None:
            manager_pointer = pointer
        elif pointer != manager_pointer:
            errors.append("manager container pointer must remain stable across fixture boundaries")
        if not is_pointer(manager.get("producer_or_consumer_address")):
            errors.append(f"{name}.manager.producer_or_consumer_address is required")
        if not is_hex(manager.get("raw_descriptor_hex")):
            errors.append(f"{name}.manager.raw_descriptor_hex is required")
        rooting = manager.get("rooting")
        if not isinstance(rooting, dict):
            errors.append(f"{name}.manager.rooting must be an object")
        else:
            if rooting.get("source_kind") not in ("producer_rooted", "consumer_rooted"):
                errors.append(f"{name}.manager.rooting.source_kind must be producer_rooted or consumer_rooted")
            if rooting.get("arena_scan") is not False or rooting.get("cached_address_repair") is not False:
                errors.append(f"{name}.manager.rooting must reject arena scans and cached-address repair")
            root_chain = rooting.get("root_chain")
            if not isinstance(root_chain, list) or not root_chain or not all(is_pointer(item) for item in root_chain):
                errors.append(f"{name}.manager.rooting.root_chain must preserve nonzero pointers")
        walk = manager.get("walk")
        if not isinstance(walk, dict) or walk.get("bounded") is not True:
            errors.append(f"{name}.manager.walk must be bounded")
            member_sets.append(set())
            continue
        entries = walk.get("entries")
        if not isinstance(entries, list):
            errors.append(f"{name}.manager.walk.entries must be a list")
            member_sets.append(set())
            continue
        if walk.get("declared_count") != len(entries):
            errors.append(f"{name}.manager.walk.declared_count must equal entry count")
        if not is_pointer(walk.get("begin_pointer")) or not is_pointer(walk.get("end_pointer")):
            errors.append(f"{name}.manager.walk begin/end pointers are required")
        if not positive_int(walk.get("stride")):
            errors.append(f"{name}.manager.walk.stride must be positive")
        if is_pointer(walk.get("begin_pointer")) and is_pointer(walk.get("end_pointer")) and positive_int(walk.get("stride")):
            begin = int(walk["begin_pointer"], 16)
            end = int(walk["end_pointer"], 16)
            if end < begin or end - begin != len(entries) * walk["stride"]:
                errors.append(f"{name}.manager.walk bounds must equal declared contiguous entry span")
        army_ids = []
        zero_members = set()
        for entry_index, entry in enumerate(entries):
            if not isinstance(entry, dict) or entry.get("enumeration_index") != entry_index:
                errors.append(f"{name}.manager.walk.entries[{entry_index}] must preserve enumeration_index")
                continue
            army_id = entry.get("army_id")
            if not positive_int(army_id) or not is_pointer(entry.get("army_pointer")):
                errors.append(f"{name}.manager.walk.entries[{entry_index}] needs army identity and pointer")
                continue
            expected_role = next((role for role, expected_id in fixture_ids.items() if expected_id == army_id), None)
            if expected_role is not None:
                if entry.get("fixture_role") != expected_role:
                    errors.append(f"{name}.manager.walk.entries[{entry_index}] fixture_role mismatch")
                receipt = entry.get("classification_receipt")
                if not isinstance(receipt, dict) or not isinstance(receipt.get("source"), str) or not receipt["source"]:
                    errors.append(f"{name}.manager.walk.entries[{entry_index}] needs classification_receipt")
            if not is_hex(entry.get("raw_army_window_hex")):
                errors.append(f"{name}.manager.walk.entries[{entry_index}].raw_army_window_hex is required")
            members = entry.get("member_division_ids")
            if not isinstance(members, list) or not all(positive_int(item) for item in members):
                errors.append(f"{name}.manager.walk.entries[{entry_index}].member_division_ids is invalid")
                members = []
            army_ids.append(army_id)
            if not members:
                zero_members.add(army_id)
        if len(army_ids) != len(set(army_ids)):
            errors.append(f"{name}.manager.walk.entries contains duplicate army IDs")
        if not expected.issubset(set(army_ids)):
            errors.append(f"{name}.manager walk must enumerate every fixture army")
        save_armies = boundary.get("save_armies")
        save_by_id: dict[int, set[int]] = {}
        if not isinstance(save_armies, dict):
            errors.append(f"{name}.save_armies must be an object")
        else:
            if save_armies.get("boundary_token") != boundary.get("boundary_token"):
                errors.append(f"{name}.save_armies boundary token must match live observation")
            if not is_sha256(save_armies.get("save_sha256")):
                errors.append(f"{name}.save_armies.save_sha256 is required")
            if not isinstance(save_armies.get("save_path"), str) or not save_armies["save_path"]:
                errors.append(f"{name}.save_armies.save_path is required")
            save_rows = save_armies.get("armies")
            if not isinstance(save_rows, list):
                errors.append(f"{name}.save_armies.armies must be a list")
            else:
                seen_save_army_ids: set[int] = set()
                for save_index, save_row in enumerate(save_rows):
                    if not isinstance(save_row, dict) or not positive_int(save_row.get("army_id")):
                        errors.append(f"{name}.save_armies.armies[{save_index}] is invalid")
                        continue
                    army_ref = save_row.get("army_ref")
                    if not isinstance(army_ref, dict) or army_ref.get("id") != save_row["army_id"] or not positive_int(army_ref.get("type")):
                        errors.append(f"{name}.save_armies.armies[{save_index}].army_ref must preserve typed identity")
                    if save_row["army_id"] in seen_save_army_ids:
                        errors.append(f"{name}.save_armies contains duplicate army ID {save_row['army_id']}")
                    seen_save_army_ids.add(save_row["army_id"])
                    save_members = save_row.get("member_division_ids")
                    if not isinstance(save_members, list) or not all(positive_int(item) for item in save_members):
                        errors.append(f"{name}.save_armies.armies[{save_index}].member_division_ids is invalid")
                        continue
                    if len(save_members) != len(set(save_members)):
                        errors.append(f"{name}.save_armies.armies[{save_index}].member_division_ids contains duplicates")
                    save_refs = save_row.get("member_refs")
                    if typed_ref_ids(save_refs) != save_members:
                        errors.append(f"{name}.save_armies.armies[{save_index}].member_refs must preserve typed refs in ID order")
                    save_by_id[save_row["army_id"]] = set(save_members)
                if not expected.issubset(set(save_by_id)):
                    errors.append(f"{name}.save_armies must contain every fixture army")
        live_by_id = {
            entry.get("army_id"): set(entry.get("member_division_ids") or [])
            for entry in entries
            if isinstance(entry, dict) and positive_int(entry.get("army_id"))
        }
        for fixture_id in expected:
            if fixture_id in live_by_id and fixture_id in save_by_id and live_by_id[fixture_id] != save_by_id[fixture_id]:
                errors.append(f"{name} live/save member mismatch for fixture army {fixture_id}")
        zero_id = fixture_ids.get("zero_member")
        zero_entry = next((entry for entry in entries if isinstance(entry, dict) and entry.get("army_id") == zero_id), None)
        member_sets.append(set((zero_entry or {}).get("member_division_ids") or []))
        if index in (0, 2) and zero_id not in zero_members:
            errors.append(f"{name} zero_member fixture army must have no divisions")
    if len(member_sets) == 3:
        if len(member_sets[1] - member_sets[0]) != 1 or member_sets[0] - member_sets[1]:
            errors.append("manager add_member boundary must add exactly one division to the zero-member army")
        if member_sets[2] != member_sets[0]:
            errors.append("manager remove_member boundary must restore the zero-member baseline set")
    return len(errors) == start


def order_snapshot(boundary: dict[str, Any], name: str, errors: list[str]) -> dict[str, Any] | None:
    start = len(errors)
    row = boundary.get("order")
    if not isinstance(row, dict):
        errors.append(f"{name}.order must be an object")
        return None
    for key in ("army_id", "engine_order_instance_id", "generation"):
        if not positive_int(row.get(key)):
            errors.append(f"{name}.order.{key} must be a positive integer")
    if not is_nonnegative_int(row.get("slot_index")):
        errors.append(f"{name}.order.slot_index must be nonnegative")
    if not isinstance(row.get("order_type"), int):
        errors.append(f"{name}.order.order_type must be an integer")
    if not isinstance(row.get("deleted"), bool):
        errors.append(f"{name}.order.deleted must be boolean")
    for key in ("path_provinces", "assigned_division_ids"):
        values = row.get(key)
        if not isinstance(values, list) or not all(positive_int(item) for item in values):
            errors.append(f"{name}.order.{key} must be an ordered positive-integer list")
    for key in ("geometry_status", "assigned_units_status", "active_status", "wire_flags", "wire_row_index"):
        if not is_nonnegative_int(row.get(key)):
            errors.append(f"{name}.order.{key} must be a nonnegative integer")
    if not isinstance(row.get("active"), bool):
        errors.append(f"{name}.order.active must be boolean")
    if not row.get("deleted") and row.get("active_status") != STATUS_EXACT:
        errors.append(f"{name}.order.active_status must be exact for a live current-wire row")
    if not is_hex(row.get("raw_row_hex"), exact_bytes=CURRENT_ROW_LEN):
        errors.append(f"{name}.order.raw_row_hex must contain exactly 88 bytes")
    record = boundary.get("d54_record")
    if not isinstance(record, dict):
        errors.append(f"{name}.d54_record must be an object")
    else:
        if not is_nonnegative_int(record.get("stream_sequence")):
            errors.append(f"{name}.d54_record.stream_sequence must be nonnegative")
        if not isinstance(record.get("game_hour"), int) or isinstance(record.get("game_hour"), bool):
            errors.append(f"{name}.d54_record.game_hour must be an integer")
        if not is_nonnegative_int(record.get("record_offset")):
            errors.append(f"{name}.d54_record.record_offset must be nonnegative")
        if not is_sha256(record.get("payload_sha256")):
            errors.append(f"{name}.d54_record.payload_sha256 is required")
        if not is_hex(record.get("raw_header_hex"), exact_bytes=32):
            errors.append(f"{name}.d54_record.raw_header_hex must contain exactly 32 bytes")
    if not row.get("deleted"):
        raw_order = boundary.get("raw_order")
        validate_raw_order(raw_order, row, name, errors)
    return row if len(errors) == start else None


def validate_raw_order(value: Any, row: dict[str, Any], name: str, errors: list[str]) -> None:
    if not isinstance(value, dict):
        errors.append(f"{name}.raw_order must be an object for live rows")
        return
    if not is_pointer(value.get("order_pointer")):
        errors.append(f"{name}.raw_order.order_pointer must be a nonzero hex pointer")
    fields = value.get("fields")
    required = {
        "type_at_0x30": 4,
        "army_backref_at_0x38": 8,
        "path_begin_at_0x70": 8,
        "path_size_at_0x78": 4,
        "assigned_begin_at_0x210": 8,
        "assigned_size_at_0x21c": 4,
        "instance_id_at_0x244": 4,
        "active_at_0x248": 1,
    }
    if not isinstance(fields, dict):
        errors.append(f"{name}.raw_order.fields must be an object")
        return
    for key, byte_len in required.items():
        if not is_hex(fields.get(key), exact_bytes=byte_len):
            errors.append(f"{name}.raw_order.fields.{key} must contain exactly {byte_len} raw bytes")
    if value.get("decoded_instance_id") != row.get("engine_order_instance_id"):
        errors.append(f"{name}.raw_order.decoded_instance_id must equal the D54 +0x244 identity")
    if value.get("decoded_order_type") != row.get("order_type"):
        errors.append(f"{name}.raw_order.decoded_order_type must equal the D54 order type")
    if value.get("decoded_active") != row.get("active"):
        errors.append(f"{name}.raw_order.decoded_active must equal the D54 +0x248 value")
    if value.get("ordered_path_provinces") != row.get("path_provinces"):
        errors.append(f"{name}.raw_order ordered path must equal the D54 ordered path")
    if value.get("ordered_assigned_division_ids") != row.get("assigned_division_ids"):
        errors.append(f"{name}.raw_order assigned IDs must equal the D54 ordered assigned IDs")


def validate_save_order(value: Any, boundary_token: Any, name: str, errors: list[str]) -> list[int]:
    if not isinstance(value, dict):
        errors.append(f"{name} must be an object")
        return []
    if value.get("boundary_token") != boundary_token:
        errors.append(f"{name}.boundary_token must match the live paused boundary")
    if not is_sha256(value.get("save_sha256")):
        errors.append(f"{name}.save_sha256 is required")
    if not isinstance(value.get("save_path"), str) or not value["save_path"]:
        errors.append(f"{name}.save_path is required")
    if not positive_int(value.get("instance_id")):
        errors.append(f"{name}.instance_id must be positive")
    if not positive_int(value.get("army_id")):
        errors.append(f"{name}.army_id must be positive")
    army_ref = value.get("army_ref")
    if not isinstance(army_ref, dict) or army_ref.get("id") != value.get("army_id") or not positive_int(army_ref.get("type")):
        errors.append(f"{name}.army_ref must preserve the typed save army identity")
    scheduled = value.get("scheduled_member_ids")
    if not isinstance(scheduled, list) or not all(positive_int(item) for item in scheduled):
        errors.append(f"{name}.scheduled_member_ids must be an ordered positive-integer list")
        return []
    if len(scheduled) != len(set(scheduled)):
        errors.append(f"{name}.scheduled_member_ids contains duplicates")
    if typed_ref_ids(value.get("scheduled_member_refs")) != scheduled:
        errors.append(f"{name}.scheduled_member_refs must preserve typed refs in ID order")
    return scheduled


def typed_ref_ids(value: Any) -> list[int] | None:
    if not isinstance(value, list):
        return None
    ids = []
    for item in value:
        if not isinstance(item, dict) or not positive_int(item.get("id")) or not positive_int(item.get("type")):
            return None
        ids.append(item["id"])
    return ids


def positive_int(value: Any) -> bool:
    return isinstance(value, int) and not isinstance(value, bool) and value > 0


def is_nonnegative_int(value: Any) -> bool:
    return isinstance(value, int) and not isinstance(value, bool) and value >= 0


def is_pointer(value: Any) -> bool:
    if not isinstance(value, str) or not value.startswith("0x"):
        return False
    try:
        return int(value, 16) > 0
    except ValueError:
        return False


def is_sha256(value: Any) -> bool:
    return isinstance(value, str) and len(value) == 64 and all(char in "0123456789abcdefABCDEF" for char in value)


def is_hex(value: Any, exact_bytes: int | None = None) -> bool:
    if not isinstance(value, str) or len(value) % 2:
        return False
    if exact_bytes is not None and len(value) != exact_bytes * 2:
        return False
    return bool(value) and all(char in "0123456789abcdefABCDEF" for char in value)


def blank_order(order_type: int, *, deleted: bool = False) -> dict[str, Any]:
    return {
        "army_id": 0,
        "engine_order_instance_id": 0,
        "generation": 0,
        "slot_index": 0,
        "order_type": order_type,
        "deleted": deleted,
        "path_provinces": [],
        "assigned_division_ids": [],
        "assigned_units_status": STATUS_EXACT if not deleted else 0,
        "geometry_status": 0,
        "active": False,
        "active_status": STATUS_EXACT if not deleted else 0,
        "wire_flags": 0,
        "wire_row_index": 0,
        "raw_row_hex": "",
    }


def blank_raw_order() -> dict[str, Any]:
    return {
        "order_pointer": "0x0",
        "decoded_instance_id": 0,
        "decoded_order_type": 0,
        "decoded_active": False,
        "ordered_path_provinces": [],
        "ordered_assigned_division_ids": [],
        "fields": {
            "type_at_0x30": "",
            "army_backref_at_0x38": "",
            "path_begin_at_0x70": "",
            "path_size_at_0x78": "",
            "assigned_begin_at_0x210": "",
            "assigned_size_at_0x21c": "",
            "instance_id_at_0x244": "",
            "active_at_0x248": "",
        },
    }


def blank_boundary(phase: str, sequence: int, order_type: int, *, deleted: bool = False) -> dict[str, Any]:
    result = {
        "boundary_id": f"replace-{phase}-boundary-id",
        "phase": phase,
        "sequence": sequence,
        "paused": True,
        "boundary_token": f"replace-{phase}-same-boundary-token",
        "d54_record": {
            "stream_sequence": 0,
            "game_hour": 0,
            "record_offset": 0,
            "payload_sha256": "",
            "raw_header_hex": "",
        },
        "order": blank_order(order_type, deleted=deleted),
    }
    if not deleted:
        result["raw_order"] = blank_raw_order()
    return result


def observation_template() -> dict[str, Any]:
    type2 = [
        blank_boundary("baseline", 0, 2),
        blank_boundary("redraw", 1, 2),
        blank_boundary("delete", 2, 2, deleted=True),
    ]
    type4 = blank_boundary("created", 3, 4)
    type4["raw_geometry_candidates"] = [
        {
            "begin_offset": "+0x70",
            "size_offset": "+0x78",
            "raw_descriptor_hex": "",
            "ordered_values": [],
        }
    ]
    type4["save_order"] = {
        "boundary_token": type4["boundary_token"],
        "save_sha256": "",
        "save_path": "",
        "army_id": 0,
        "army_ref": {"id": 0, "type": 0},
        "instance_id": 0,
        "path_provinces": [],
        "states": [],
        "scheduled_member_ids": [],
        "scheduled_member_refs": [],
        "raw_order_object": {},
    }
    membership = []
    for sequence, phase in enumerate(FIXTURE_PHASES["membership_lifecycle"], start=4):
        boundary = blank_boundary(phase, sequence, 2)
        boundary["save_order"] = {
            "boundary_token": boundary["boundary_token"],
            "save_sha256": "",
            "save_path": "",
            "army_id": 0,
            "army_ref": {"id": 0, "type": 0},
            "instance_id": 0,
            "scheduled_member_ids": [],
            "scheduled_member_refs": [],
        }
        membership.append(boundary)
    manager = []
    for sequence, phase in enumerate(FIXTURE_PHASES["empty_inactive_manager"], start=7):
        manager.append(
            {
                "boundary_id": f"replace-manager-{phase}-boundary-id",
                "phase": phase,
                "sequence": sequence,
                "paused": True,
                "boundary_token": f"replace-manager-{phase}-same-boundary-token",
                "manager": {
                    "container_pointer": "0x0",
                    "producer_or_consumer_address": "0x0",
                    "raw_descriptor_hex": "",
                    "rooting": {
                        "source_kind": "replace-with-producer_rooted-or-consumer_rooted",
                        "arena_scan": False,
                        "cached_address_repair": False,
                        "root_chain": [],
                    },
                    "walk": {
                        "bounded": True,
                        "begin_pointer": "0x0",
                        "end_pointer": "0x0",
                        "stride": 0,
                        "declared_count": 0,
                        "entries": [],
                    },
                },
                "save_armies": {
                    "boundary_token": f"replace-manager-{phase}-same-boundary-token",
                    "save_sha256": "",
                    "save_path": "",
                    "armies": [],
                },
            }
        )
    return {
        "schema": OBSERVATION_SCHEMA,
        "binary": {
            "version": "1.19.2.0.a729",
            "sha256": EXPECTED_EXE_SHA256,
            "image_base": "0x140000000",
        },
        "capture": {
            "session_id": "replace-with-harness-session-id",
            "harness_name": "replace-with-harness-name",
            "harness_version": "replace-with-harness-version",
            "harness_sha256": "",
            "process_id": 0,
            "single_process": True,
            "paused_boundaries": True,
            "harness_authenticated": True,
            "d54_wire": {"domain_id": 54, "header_len": 60, "row_len": 88},
        },
        "activation_requested": False,
        "fixtures": {
            "type2_redraw_delete": {"boundaries": type2},
            "type4_discriminator": {"boundaries": [type4]},
            "membership_lifecycle": {"boundaries": membership},
            "empty_inactive_manager": {
                "fixture_army_ids": {"active": 0, "inactive": 0, "zero_member": 0},
                "boundaries": manager,
            },
        },
    }


if __name__ == "__main__":
    raise SystemExit(main())
