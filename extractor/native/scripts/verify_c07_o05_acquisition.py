#!/usr/bin/env python3
"""Fail-closed regrader for a C07/O05 selected-Present building observation."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import struct
import sys
from pathlib import Path

EXE_SHA = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
PRODUCER_SHA = "41d9d7ba2ed076958b57fc6b243731039dc066ad48ec569bb834bb73b21aee1f"
CONSUMER_SHA = "8e8f0e12082f11b543645ef7ef5ff4940dfb573fbfbfb34f564646a121e74946"
PRODUCER_JOB = "20260720T125102Z-w5-c07-runtime-record-layout-selector-fi-4498228b"
CONSUMER_JOB = "20260720T125147Z-w5-c07-entity-instance-transform-render--1040cb63"
SCHEMA = "hoi4.c07.o05.selected_present_observation.v1"
O05_SCHEMA = "hoi4.production-o05-building-semantics.v1"


class Reject(RuntimeError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise Reject(message)


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def compact(value: object) -> bytes:
    return json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode("utf-8")


def load_object(path: Path, label: str) -> dict:
    value = json.loads(path.read_text(encoding="utf-8"))
    require(isinstance(value, dict), f"{label} is not an object")
    return value


def decimal_u64(value: object, label: str) -> int:
    require(isinstance(value, str) and value.isdigit(), f"{label} is not canonical decimal u64")
    parsed = int(value)
    require(0 <= parsed <= 2**64 - 1 and str(parsed) == value, f"{label} is outside canonical u64")
    return parsed


def exact_uint(value: object, label: str, maximum: int = 0xFFFFFFFF) -> int:
    require(isinstance(value, int) and not isinstance(value, bool)
            and 0 <= value <= maximum, f"{label} is not an unsigned integer")
    return value


def finite_bits(bits: object, label: str) -> float:
    require(isinstance(bits, int) and not isinstance(bits, bool) and 0 <= bits <= 0xFFFFFFFF,
            f"{label} is not u32 bits")
    value = struct.unpack("<f", struct.pack("<I", bits))[0]
    require(math.isfinite(value), f"{label} is not finite")
    return value


def output_key(output: dict) -> tuple[int, int, int]:
    require(isinstance(output, dict), "render output is not an object")
    require(set(output) == {"kind", "pass_ordinal", "entity_instance_id", "draw_order"},
            "O05 output is not an exact entity reference")
    require(output["kind"] == "entity", "O05 output is not entity")
    return (
        exact_uint(output["pass_ordinal"], "pass_ordinal"),
        decimal_u64(output["entity_instance_id"], "entity_instance_id"),
        exact_uint(output["draw_order"], "draw_order"),
    )


def grade(observation_path: Path, capture_manifest_path: Path) -> dict:
    receipt = load_object(observation_path, "observation")
    manifest = load_object(capture_manifest_path, "capture manifest")
    expected_top = {
        "schema", "status", "executable_sha256", "producer_manifest_sha256",
        "consumer_manifest_sha256", "producer_job_id", "consumer_job_id",
        "selected_present_sha256", "production_frame_sha256", "raw_scope_sha256",
        "scope", "portable_counts", "production_render_frame", "receipt_body_sha256",
    }
    require(set(receipt) == expected_top, "observation has missing or unknown top-level fields")
    require(receipt["schema"] == SCHEMA and receipt["status"] == "candidate",
            "observation schema/status mismatch")
    require(receipt["executable_sha256"] == EXE_SHA, "observation executable mismatch")
    require(receipt["producer_manifest_sha256"] == PRODUCER_SHA, "producer manifest mismatch")
    require(receipt["consumer_manifest_sha256"] == CONSUMER_SHA, "consumer manifest mismatch")
    require(receipt["producer_job_id"] == PRODUCER_JOB, "producer job mismatch")
    require(receipt["consumer_job_id"] == CONSUMER_JOB, "consumer job mismatch")
    for field in ("selected_present_sha256", "production_frame_sha256",
                  "raw_scope_sha256", "receipt_body_sha256"):
        require(isinstance(receipt[field], str) and len(receipt[field]) == 64
                and all(c in "0123456789abcdef" for c in receipt[field]),
                f"{field} is not lowercase SHA-256")
    body = dict(receipt)
    expected_receipt_hash = body.pop("receipt_body_sha256")
    require(sha256_bytes(compact(body)) == expected_receipt_hash, "receipt body hash mismatch")

    frame = receipt["production_render_frame"]
    require(isinstance(frame, dict), "production_render_frame is not an object")
    require(sha256_bytes(compact(frame)) == receipt["production_frame_sha256"],
            "production frame hash mismatch")
    authority = frame.get("authority")
    require(isinstance(authority, dict), "production frame authority is absent")
    vfs_sha = authority.get("effective_vfs_manifest_sha256")
    require(isinstance(vfs_sha, str) and len(vfs_sha) == 64, "effective-VFS hash is absent")
    selected = frame.get("selected_present")
    require(isinstance(selected, dict), "selected-Present receipt is absent")
    selected_keys = {
        "candidate_count", "candidate_set_sha256", "selected_candidate_id",
        "selected_ordinal", "selected_game_hour_raw", "selected_qpc_ticks",
        "selection_cutoff_qpc_ticks",
    }
    require(set(selected) == selected_keys, "selected-Present receipt fields differ")
    canonical_selected = {
        "candidate_count": exact_uint(selected["candidate_count"], "candidate_count"),
        "candidate_set_sha256": selected["candidate_set_sha256"],
        "selected_candidate_id": selected["selected_candidate_id"],
        "selected_ordinal": exact_uint(selected["selected_ordinal"], "selected_ordinal"),
        "selected_game_hour_raw": selected["selected_game_hour_raw"],
        "selected_qpc_ticks": selected["selected_qpc_ticks"],
        "selection_cutoff_qpc_ticks": selected["selection_cutoff_qpc_ticks"],
    }
    require(isinstance(canonical_selected["candidate_set_sha256"], str)
            and len(canonical_selected["candidate_set_sha256"]) == 64
            and all(c in "0123456789abcdef"
                    for c in canonical_selected["candidate_set_sha256"]),
            "selected-Present candidate-set hash is malformed")
    require(isinstance(canonical_selected["selected_game_hour_raw"], int)
            and not isinstance(canonical_selected["selected_game_hour_raw"], bool)
            and -(2**31) <= canonical_selected["selected_game_hour_raw"] <= 2**31 - 1,
            "selected-Present game hour is not i32")
    decimal_u64(canonical_selected["selected_candidate_id"], "selected_candidate_id")
    selected_qpc = decimal_u64(
        canonical_selected["selected_qpc_ticks"], "selected-Present QPC")
    cutoff_qpc = decimal_u64(
        canonical_selected["selection_cutoff_qpc_ticks"], "selected-Present cutoff QPC")
    require(selected_qpc > 0 and selected_qpc <= cutoff_qpc,
            "selected-Present QPC/cutoff ordering is invalid")
    require(canonical_selected["candidate_count"] > 0
            and canonical_selected["selected_ordinal"] + 1
            == canonical_selected["candidate_count"],
            "selected-Present candidate count/ordinal is invalid")
    require(sha256_bytes(compact(canonical_selected)) == receipt["selected_present_sha256"],
            "selected-Present hash mismatch")
    require(frame.get("display_submit_index") == selected.get("selected_candidate_id"),
            "display submit and selected-Present indices differ")
    frame_update = decimal_u64(frame.get("update_index"), "frame update index")
    frame_first = decimal_u64(frame.get("first_sequence"), "frame first sequence")
    frame_last = decimal_u64(frame.get("last_sequence"), "frame last sequence")
    require(frame_first <= frame_last, "production frame sequence frontier is reversed")

    o05 = frame.get("o05_buildings")
    require(isinstance(o05, dict), "O05 carrier is absent")
    require(o05.get("schema") == O05_SCHEMA, "O05 schema mismatch")
    require(o05.get("executable_sha256") == EXE_SHA, "O05 executable mismatch")
    require(o05.get("runtime_producer_manifest_sha256") == PRODUCER_SHA,
            "O05 producer manifest mismatch")
    require(o05.get("runtime_consumer_manifest_sha256") == CONSUMER_SHA,
            "O05 consumer manifest mismatch")
    require(o05.get("complete_selected_present_scope") is True, "O05 scope is incomplete")
    require(o05.get("update_index") == frame.get("update_index")
            and o05.get("first_sequence") == frame.get("first_sequence")
            and o05.get("last_sequence") == frame.get("last_sequence"),
            "O05 frame frontier mismatch")
    require(decimal_u64(o05.get("update_index"), "O05 update index") == frame_update,
            "O05 update index is not canonical")
    source = o05.get("map_buildings_source")
    require(isinstance(source, dict)
            and source.get("normalized_game_relative_path") == "map/buildings.txt"
            and source.get("effective_vfs_manifest_sha256") == vfs_sha,
            "O05 map/buildings source authority mismatch")

    submits = frame.get("submits")
    require(isinstance(submits, list) and len(submits) == 1,
            "production frame does not retain exactly one submit")
    display = [
        row for row in submits
        if isinstance(row, dict)
        and str(row.get("submit_index")) == frame["display_submit_index"]
    ]
    require(len(display) == 1, "selected display submit is absent or duplicated")
    display = display[0]
    require(display.get("presentation_qpc_ticks") == selected["selected_qpc_ticks"],
            "selected submit QPC differs from selected-Present receipt")
    ordered_draws = display.get("ordered_draws")
    entities = display.get("entities")
    require(isinstance(ordered_draws, list) and isinstance(entities, list),
            "selected submit draw/entity arrays are absent")
    entity_by_key = {}
    for entity in entities:
        require(isinstance(entity, dict), "entity submission is not an object")
        key = (
            exact_uint(entity["pass_ordinal"], "submission pass_ordinal"),
            decimal_u64(entity["entity_instance_id"], "submission entity_instance_id"),
            exact_uint(entity["draw_order"], "submission draw_order"),
        )
        require(key not in entity_by_key, "entity submission key is duplicated")
        entity_by_key[key] = entity

    records = o05.get("records")
    ordered_outputs = o05.get("ordered_outputs")
    require(isinstance(records, list) and records, "O05 record list is empty")
    require(isinstance(ordered_outputs, list) and ordered_outputs,
            "O05 ordered output list is absent or empty")
    seen_runtime = set()
    seen_entities = set()
    per_record_outputs = []
    counts = {"no_entity_selected": 0, "selected_not_submitted": 0, "submitted": 0}
    for ordinal, record in enumerate(records):
        require(isinstance(record, dict), "O05 record is not an object")
        require(exact_uint(record.get("update_ordinal"), "O05 update ordinal") == ordinal,
                "O05 update ordinal is not contiguous")
        runtime_id = decimal_u64(record.get("runtime_object_id"), "runtime_object_id")
        event_sequence = decimal_u64(record.get("event_sequence"), "event_sequence")
        require(frame_first <= event_sequence <= frame_last,
                "O05 event sequence crosses the frame frontier")
        require(runtime_id != 0 and runtime_id not in seen_runtime,
                "O05 runtime object ID is zero or duplicated")
        seen_runtime.add(runtime_id)
        require(record.get("object_tag_u32") == 0x165, "O05 record tag mismatch")
        exact_uint(record.get("building_type_id"), "building_type_id")
        kind_index = record.get("building_kind_index_i32")
        require(isinstance(kind_index, int) and not isinstance(kind_index, bool)
                and -(2**31) <= kind_index <= 2**31 - 1,
                "O05 building kind index is not i32")
        require((kind_index == 0 and record.get("object_kind") == "province_building")
                or (kind_index != 0 and record.get("object_kind") == "special_map_building"),
                "O05 kind/index branch mismatch")
        exact_uint(record.get("state_handle_u16"), "state_handle_u16", 0xFFFF)
        for field in ("province_handle_primary_i32", "province_handle_secondary_i32"):
            value = record.get(field)
            require(isinstance(value, int) and not isinstance(value, bool)
                    and -(2**31) <= value <= 2**31 - 1, f"{field} is not i32")
        for field in (
            "building_order_u8", "selected_entity_ordinal_u8",
            "entity_visual_state_u8", "type_visibility_flag_u8",
        ):
            exact_uint(record.get(field), field, 0xFF)
        require(isinstance(record.get("destroyed_variant"), bool),
                "destroyed_variant is not boolean")
        position = record.get("position_xyz_f32_bits")
        require(isinstance(position, list) and len(position) == 3,
                "O05 position is not XYZ")
        for index, bits in enumerate(position):
            finite_bits(bits, f"position[{index}]")
        finite_bits(record.get("yaw_f32_bits"), "yaw")
        require(finite_bits(record.get("presentation_scale_f32_bits"), "presentation scale") >= 0,
                "O05 presentation scale is negative")
        outputs = record.get("referenced_outputs")
        require(isinstance(outputs, list), "O05 referenced_outputs is absent")
        entity_id_value = record.get("selected_entity_instance_id")
        selected_entity_id = None if entity_id_value is None else decimal_u64(
            entity_id_value, "selected_entity_instance_id")
        submitted = bool(outputs)
        expected = ("submitted" if submitted else
                    "selected_not_submitted" if selected_entity_id is not None else
                    "no_entity_selected")
        require(record.get("render_disposition") == expected, "O05 disposition mismatch")
        counts[expected] += 1
        require((record.get("world_transform_4x4_f32_bits") is not None)
                == (selected_entity_id is not None), "O05 transform/selection mismatch")
        require((record.get("entity_asset") is not None) == submitted
                and (record.get("mesh") is not None) == submitted,
                "O05 content presence differs from submit disposition")
        if selected_entity_id is not None:
            require(selected_entity_id != 0 and selected_entity_id not in seen_entities,
                    "O05 selected entity is zero or shared")
            seen_entities.add(selected_entity_id)
            matrix = record["world_transform_4x4_f32_bits"]
            require(isinstance(matrix, list) and len(matrix) == 16, "O05 matrix is not 4x4")
            for index, bits in enumerate(matrix):
                finite_bits(bits, f"matrix[{index}]")
            require(matrix[3] == 0 and matrix[7] == 0 and matrix[11] == 0
                    and matrix[15] == 0x3F800000, "O05 matrix is not affine")
        keys = [output_key(output) for output in outputs]
        require(all(key[1] == selected_entity_id for key in keys),
                "O05 output crosses selected entity identity")
        complete = [output_key(output) for output in ordered_draws
                    if isinstance(output, dict)
                    and output.get("kind") == "entity"
                    and output.get("entity_instance_id") == entity_id_value]
        require(keys == complete, "O05 record does not cover the complete entity output set")
        for key in keys:
            require(key in entity_by_key, "O05 output references absent entity submission")
            entity = entity_by_key[key]
            require(entity["content_identity"]["resolved_entity_asset"] == record["entity_asset"]
                    and entity["content_identity"]["mesh"] == record["mesh"]
                    and entity["world_transform_4x4_f32_bits"]
                    == record["world_transform_4x4_f32_bits"],
                    "O05 semantic content/transform differs from entity submission")
        per_record_outputs.extend(outputs)

    ordered_keys = [output_key(output) for output in ordered_outputs]
    require(len(ordered_keys) == len(set(ordered_keys)), "O05 ordered outputs are duplicated")
    ordered_key_set = set(ordered_keys)
    expected_order = [output_key(output) for output in ordered_draws
                      if isinstance(output, dict)
                      and output.get("kind") == "entity"
                      and output_key(output) in ordered_key_set]
    require(ordered_keys == expected_order, "O05 ordered outputs differ from display order")
    require(set(map(output_key, per_record_outputs)) == set(ordered_keys),
            "O05 per-record output cover differs from ordered outputs")

    scope = receipt["scope"]
    portable_counts = receipt["portable_counts"]
    require(isinstance(scope, dict) and isinstance(portable_counts, dict),
            "receipt counts are absent")
    require(set(scope) == {
        "game_hour_raw", "first_qpc_ticks", "completed_qpc_ticks",
        "qpc_frequency_hz", "record_count",
    }, "scope fields differ")
    require(set(portable_counts) == {
        "record_count", "no_entity_selected", "selected_not_submitted",
        "submitted", "ordered_output_count",
    }, "portable-count fields differ")
    require(exact_uint(scope.get("record_count"), "scope record count", 2**64 - 1)
            == len(records)
            and exact_uint(portable_counts.get("record_count"),
                           "portable record count", 2**64 - 1) == len(records)
            and exact_uint(portable_counts.get("ordered_output_count"),
                           "portable output count", 2**64 - 1) == len(ordered_outputs),
            "receipt record/output counts mismatch")
    require(all(exact_uint(portable_counts.get(key), f"{key} count", 2**64 - 1) == value
                for key, value in counts.items()),
            "receipt disposition counts mismatch")
    require(counts["submitted"] > 0, "observation contains no submitted generic building")
    require(scope.get("game_hour_raw") == selected.get("selected_game_hour_raw"),
            "scope game hour differs from selected Present")
    require(decimal_u64(scope.get("first_qpc_ticks"), "scope first QPC")
            <= decimal_u64(scope.get("completed_qpc_ticks"), "scope completed QPC")
            <= selected_qpc,
            "scope QPC order crosses selected Present")
    require(decimal_u64(scope.get("qpc_frequency_hz"), "scope QPC frequency") > 0,
            "scope QPC frequency is zero")
    require(decimal_u64(scope.get("qpc_frequency_hz"), "scope QPC frequency")
            == decimal_u64(frame.get("presentation_qpc_frequency_hz"), "frame QPC frequency"),
            "scope QPC frequency differs from production frame")

    require(manifest.get("schema") == "native_collector_session_v1",
            "capture manifest schema mismatch")
    early_launch = manifest.get("early_render_launch_manifest")
    require(isinstance(early_launch, dict), "capture manifest has no early-launch authority")
    require(early_launch.get("schema") == "extractor.early-render-launch-manifest.v1",
            "early-launch authority schema mismatch")
    require(early_launch.get("executable_sha256") == EXE_SHA,
            "capture manifest does not bind exact-a729 executable")
    require(early_launch.get("effective_vfs_manifest_sha256") == vfs_sha,
            "capture manifest effective VFS differs from production frame")
    return {
        "schema": "hoi4.c07.o05.selected_present_regrade.v1",
        "ok": True,
        "observation_path": str(observation_path.resolve()),
        "observation_sha256": sha256_bytes(observation_path.read_bytes()),
        "capture_manifest_path": str(capture_manifest_path.resolve()),
        "capture_manifest_sha256": sha256_bytes(capture_manifest_path.read_bytes()),
        "record_count": len(records),
        "ordered_output_count": len(ordered_outputs),
        "disposition_counts": counts,
        "selected_present_sha256": receipt["selected_present_sha256"],
        "production_frame_sha256": receipt["production_frame_sha256"],
        "raw_scope_sha256": receipt["raw_scope_sha256"],
        "problems": [],
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--observation", required=True, type=Path)
    parser.add_argument("--capture-manifest", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args(argv)
    try:
        report = grade(args.observation, args.capture_manifest)
    except (OSError, UnicodeError, json.JSONDecodeError, KeyError, TypeError, ValueError, Reject) as exc:
        report = {
            "schema": "hoi4.c07.o05.selected_present_regrade.v1",
            "ok": False,
            "observation_path": str(args.observation),
            "capture_manifest_path": str(args.capture_manifest),
            "problems": [str(exc)],
        }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0 if report["ok"] else 2


if __name__ == "__main__":
    sys.exit(main())
