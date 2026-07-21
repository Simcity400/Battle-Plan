#!/usr/bin/env python3
"""Independent fail-closed verifier for the A26 same-Present live matrix."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
from pathlib import Path

SCHEMA = "a26-same-present-runtime-receipt-v1"
A729_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
MANIFEST_SHA256 = "a23b63ccefc021eeced9a1a918588d3e2fbba07e5d1afb4d53f83cc7c1640177"
REQUIRED_CASES = {
    "hidden_baseline_1920x1080": (1920, 1080, False),
    "capture_start_visible_1920x1080": (1920, 1080, True),
    "show_1920x1080": (1920, 1080, True),
    "hide_1920x1080": (1920, 1080, False),
    "destroyed_1920x1080": (1920, 1080, False),
    "owner_change_1920x1080": (1920, 1080, True),
    "multiline_dynamic_1920x1080": (1920, 1080, True),
    "edge_geometry_1920x1080": (1920, 1080, True),
    "edge_geometry_2560x1440": (2560, 1440, True),
}
HEX64 = re.compile(r"^[0-9a-f]{64}$")
PRODUCER_ID = re.compile(
    r"^a26:a729:controller=0x002A4080:owner-generation=[1-9][0-9]*:"
    r"observation=[1-9][0-9]*:qpc=[1-9][0-9]*$"
)


def canonical_sha256(value: object) -> str:
    payload = json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
    return hashlib.sha256(payload.encode("utf-8")).hexdigest()

def selected_present_sha256(selected: dict) -> str:
    preimage = {
        "candidate_count": selected["candidate_count"],
        "candidate_set_sha256": selected["candidate_set_sha256"],
        "selected_candidate_id": selected["selected_candidate_id"],
        "selected_ordinal": selected["selected_ordinal"],
        "selected_game_hour_raw": selected["selected_game_hour_raw"],
        "selected_qpc_ticks": selected["selected_qpc_ticks"],
        "selection_cutoff_qpc_ticks": selected["selection_cutoff_qpc_ticks"],
    }
    payload = json.dumps(preimage, separators=(",", ":"), ensure_ascii=False)
    return hashlib.sha256(payload.encode("utf-8")).hexdigest()


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def validate_instance(case_id: str, expected: dict, actual: dict) -> None:
    keys = (
        "frame_hour",
        "pov_country_or_global",
        "hovered_element_authored_name",
        "tooltip_owner_authored_name",
        "repeat_index",
        "screen_rect",
        "viewport_width_px",
        "viewport_height_px",
        "producer_record_identity",
        "final_resolved_text",
    )
    for key in keys:
        require(actual.get(key) == expected.get(key), f"{case_id}: {key} parity mismatch")


def validate_case(case: dict) -> None:
    case_id = case.get("case_id")
    require(case_id in REQUIRED_CASES, f"unexpected case {case_id!r}")
    width, height, visible = REQUIRED_CASES[case_id]
    process_nonce = case.get("process_start_nonce")
    require(
        isinstance(process_nonce, str)
        and process_nonce.isdigit()
        and not process_nonce.startswith("0"),
        f"{case_id}: invalid process-start nonce",
    )
    require(
        isinstance(case.get("hook_install_epoch"), int)
        and case["hook_install_epoch"] > 0,
        f"{case_id}: invalid hook install epoch",
    )
    require(case.get("executable_sha256") == A729_SHA256, f"{case_id}: binary mismatch")
    require(case.get("producer_manifest_sha256") == MANIFEST_SHA256, f"{case_id}: manifest mismatch")
    require(
        HEX64.fullmatch(case.get("case_receipt_file_sha256", "")) is not None,
        f"{case_id}: raw case receipt hash absent",
    )
    require(case.get("viewport") == [width, height], f"{case_id}: viewport mismatch")
    selected = case.get("selected_present")
    hook = case.get("hook_observation")
    d72 = case.get("d72")
    fold = case.get("fold")
    replay = case.get("replay")
    require(all(isinstance(row, dict) for row in (selected, hook, d72, fold, replay)), f"{case_id}: missing stage")
    require(
        isinstance(selected.get("candidate_count"), int)
        and selected["candidate_count"] > 0
        and isinstance(selected.get("selected_ordinal"), int)
        and 0 <= selected["selected_ordinal"] < selected["candidate_count"],
        f"{case_id}: invalid selected candidate cardinality",
    )
    require(
        HEX64.fullmatch(selected.get("candidate_set_sha256", "")) is not None
        and selected["candidate_set_sha256"] != "0" * 64,
        f"{case_id}: invalid candidate-set hash",
    )
    for field in ("selected_candidate_id", "selected_qpc_ticks", "selection_cutoff_qpc_ticks"):
        value = selected.get(field)
        require(isinstance(value, str) and value.isdigit() and (value == "0" or not value.startswith("0")), f"{case_id}: noncanonical {field}")
    candidate_id = int(selected["selected_candidate_id"])
    selected_qpc = int(selected["selected_qpc_ticks"])
    cutoff_qpc = int(selected["selection_cutoff_qpc_ticks"])
    require(candidate_id > 0 and 0 < selected_qpc <= cutoff_qpc, f"{case_id}: invalid selected identity/QPC")
    raw_hour = selected.get("selected_game_hour_raw")
    frame_hour = selected.get("frame_hour")
    require(isinstance(raw_hour, int) and isinstance(frame_hour, int), f"{case_id}: invalid selected hour")
    require(frame_hour == raw_hour - 43_799_999 and frame_hour >= 0, f"{case_id}: canonical hour mismatch")
    receipt_sha256 = selected_present_sha256(selected)
    require(selected.get("receipt_sha256") == receipt_sha256, f"{case_id}: selected receipt hash mismatch")
    joins = (
        ("selected_present_candidate_id", candidate_id),
        ("selected_present_ordinal", selected["selected_ordinal"]),
        ("selected_present_qpc_ticks", selected_qpc),
        ("selected_present_receipt_sha256", receipt_sha256),
        ("frame_hour", frame_hour),
    )
    for field, expected in joins:
        require(d72.get(field) == expected, f"{case_id}: d72 {field} mismatch")
        require(fold.get(field) == expected, f"{case_id}: fold {field} mismatch")
    require(hook.get("qpc_ticks", 0) <= selected_qpc, f"{case_id}: hook is newer than Present")
    require(hook.get("observation_sequence", 0) > 0, f"{case_id}: no controller observation")
    require(hook.get("owner_generation", 0) > 0, f"{case_id}: owner generation absent")
    require(hook.get("install_epoch") == case["hook_install_epoch"], f"{case_id}: hook epoch mismatch")
    if case_id == "owner_change_1920x1080":
        require(
            hook.get("owner_generation", 0) > hook.get("prior_owner_generation", 0) > 0,
            f"{case_id}: owner generation did not advance",
        )
    require(d72.get("schema_version") == 2 and fold.get("schema_version") == 2, f"{case_id}: not TTI2")
    require(d72.get("status_flags") == 0 and fold.get("status_flags") == 0, f"{case_id}: failure status")
    require(
        d72.get("authority") == "exact_a729_tooltip_controller"
        and fold.get("authority") == "exact_a729_tooltip_controller",
        f"{case_id}: authority mismatch",
    )
    require(
        d72.get("producer_manifest_sha256") == MANIFEST_SHA256
        and fold.get("producer_manifest_sha256") == MANIFEST_SHA256,
        f"{case_id}: stage manifest mismatch",
    )
    require(
        d72.get("completeness_record_identity")
        == fold.get("completeness_record_identity"),
        f"{case_id}: completeness identity mismatch",
    )
    require(
        d72.get("completeness_record_identity")
        == (
            f"a26:a729:selected-present={candidate_id}:"
            f"observation={hook.get('observation_sequence')}:"
            f"observation-qpc={hook.get('qpc_ticks')}"
        ),
        f"{case_id}: completeness identity is not the hooked observation",
    )
    require(d72.get("expected_instance_count") == len(d72.get("instances", [])), f"{case_id}: d72 count mismatch")
    require(fold.get("expected_instance_count") == len(fold.get("instances", [])), f"{case_id}: fold count mismatch")
    require(d72.get("instances") == fold.get("instances"), f"{case_id}: d72/fold mismatch")
    require(bool(hook.get("visible")) == visible, f"{case_id}: visibility mismatch")
    if not visible:
        require(d72.get("instances") == [], f"{case_id}: hidden is not measured-empty")
        require(replay.get("tooltip_binding") is None, f"{case_id}: hidden emitted shell")
        return
    instances = d72.get("instances")
    require(isinstance(instances, list) and len(instances) == 1, f"{case_id}: expected one tooltip")
    instance = instances[0]
    validate_instance(case_id, hook.get("instance", {}), instance)
    require(instance.get("tooltip_owner_authored_name") == "ToolTip", f"{case_id}: wrong owner")
    require(PRODUCER_ID.fullmatch(instance.get("producer_record_identity", "")) is not None, f"{case_id}: nonportable producer identity")
    require(instance.get("viewport_width_px") == width and instance.get("viewport_height_px") == height, f"{case_id}: instance viewport mismatch")
    require(replay.get("tooltip_binding") == {
        "x": instance["screen_rect"]["x"],
        "y": instance["screen_rect"]["y"],
        "width": instance["screen_rect"]["width"],
        "height": instance["screen_rect"]["height"],
        "text": instance["final_resolved_text"],
    }, f"{case_id}: replay binding mismatch")
    if case_id == "multiline_dynamic_1920x1080":
        require("\n" in instance.get("final_resolved_text", ""), f"{case_id}: multiline blob absent")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("receipt", type=Path)
    args = parser.parse_args()
    receipt = json.loads(args.receipt.read_text(encoding="utf-8"))
    require(receipt.get("schema") == SCHEMA, "schema mismatch")
    cases = receipt.get("cases")
    require(isinstance(cases, list), "cases must be a list")
    require({case.get("case_id") for case in cases} == set(REQUIRED_CASES), "case matrix incomplete")
    require(len(cases) == len(REQUIRED_CASES), "duplicate case IDs")
    require(receipt.get("matrix_sha256") == canonical_sha256(cases), "matrix hash mismatch")
    sources = receipt.get("source_case_receipts")
    require(isinstance(sources, list) and len(sources) == len(REQUIRED_CASES), "raw case paths missing")
    raw_hashes = {}
    for source in sources:
        raw = Path(source).read_bytes()
        raw_case = json.loads(raw)
        case_id = raw_case.get("case_id")
        require(case_id in REQUIRED_CASES and case_id not in raw_hashes, "raw case set invalid")
        raw_hashes[case_id] = hashlib.sha256(raw).hexdigest()
    require(
        all(raw_hashes[case["case_id"]] == case["case_receipt_file_sha256"] for case in cases),
        "raw case receipt hash mismatch",
    )
    for case in cases:
        validate_case(case)
    by_id = {case["case_id"]: case for case in cases}
    lifecycle_ids = (
        "show_1920x1080",
        "hide_1920x1080",
        "destroyed_1920x1080",
        "owner_change_1920x1080",
    )
    lifecycle = [by_id[case_id] for case_id in lifecycle_ids]
    require(
        len({case["process_start_nonce"] for case in lifecycle}) == 1
        and len({case["hook_install_epoch"] for case in lifecycle}) == 1,
        "lifecycle cases do not share one hook ownership epoch",
    )
    sequences = [case["hook_observation"]["observation_sequence"] for case in lifecycle]
    require(
        all(left < right for left, right in zip(sequences, sequences[1:])),
        "lifecycle observation sequence is not strictly ordered",
    )
    destroyed_generation = by_id["destroyed_1920x1080"]["hook_observation"][
        "owner_generation"
    ]
    owner_change = by_id["owner_change_1920x1080"]["hook_observation"]
    require(
        owner_change.get("prior_owner_generation") == destroyed_generation
        and owner_change.get("owner_generation", 0) > destroyed_generation,
        "owner-change generation does not continue the destruction lifecycle",
    )
    print(f"PASS A26 same-Present matrix: {len(cases)} authenticated cases")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        raise SystemExit(f"FAIL A26 same-Present matrix: {error}")
