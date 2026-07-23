#!/usr/bin/env python3
"""Fail-closed admission for an autonomous A24 six-surface acquisition."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import tempfile
from pathlib import Path

EXE_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
REQUIRED_LINE_KINDS = {
    "evaluated_trigger_result_and_reason_tree",
    "effective_hire_or_removal_cost",
    "aggregated_conditional_modifier",
    "advisor_slot_capacity_vacancy_eligibility_and_disabled_reason",
    "timed_dynamic_modifier_value_source_and_expiry",
    "displayed_party_popularity_change_source",
}
REQUIRED_TAPS = {
    "politics_entry_refresh", "advisor_compact_cost", "advisor_full_tooltip",
    "ideology_popularity_change", "timed_idea_tooltip", "active_idea_tooltip",
    "political_idea_tooltip", "idea_category_tooltip", "boost_party_tooltip",
    "idea_modifier_description", "idea_availability", "effective_idea_cost",
    "resource_affordability", "allowed_to_remove", "party_popularity_computation",
    "timed_idea_remaining_days", "country_can_take_ideas", "boost_party_why_text",
    "boost_party_name_text", "idea_selection_refresh", "idea_compact_cost_refresh",
    "idea_icon_and_modifier_bind", "modifier_value_lookup",
}


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def load(path: Path) -> dict:
    value = json.loads(path.read_text(encoding="utf-8-sig"))
    require(isinstance(value, dict), f"{path}: root is not an object")
    return value


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def atomic_new_json(path: Path, value: object) -> None:
    require(not path.exists(), f"refusing to replace admission: {path}")
    path.parent.mkdir(parents=True, exist_ok=True)
    handle, temporary = tempfile.mkstemp(prefix=f".{path.name}.", suffix=".tmp", dir=path.parent)
    try:
        with os.fdopen(handle, "w", encoding="utf-8", newline="\n") as stream:
            json.dump(value, stream, indent=2, sort_keys=True)
            stream.write("\n"); stream.flush(); os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        Path(temporary).unlink(missing_ok=True)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--actions", type=Path, required=True)
    parser.add_argument("--regrade", type=Path, required=True)
    parser.add_argument("--prebuilt-attestation", type=Path, required=True)
    parser.add_argument("--game-exe", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    actions, grade, prebuilt = load(args.actions), load(args.regrade), load(args.prebuilt_attestation)
    require(sha256(args.game_exe) == EXE_SHA256, "game executable identity mismatch")
    require(actions.get("schema") == "hoi4.a24.autonomous-scene-actions.v1", "action schema mismatch")
    require(actions.get("status") == "PASS_MACHINE_OWNED_SIX_SURFACE_SELECTED_PRESENT", "actions did not pass")
    require(actions.get("human_input_used") is False and actions.get("fabricated_present_or_rows") is False, "action owner authority is invalid")
    require(actions.get("immutable") is True, "action receipt is not immutable")
    cases = actions.get("case_receipts")
    require(isinstance(cases, list) and len(cases) == 6, "exactly six capture cases are required")
    require({case.get("line_kind") for case in cases} == REQUIRED_LINE_KINDS, "six-family scene ownership is incomplete")
    hours = [case.get("frame_hour") for case in cases]
    require(all(isinstance(hour, int) for hour in hours) and all(b == a + 1 for a, b in zip(hours, hours[1:])), "capture cases are not exact consecutive hours")
    observed: set[str] = set()
    receipt_hashes: list[str] = []
    for case in cases:
        name = case.get("receipt_file_name")
        require(isinstance(name, str) and Path(name).name == name, "case receipt name is not portable")
        receipt = args.actions.parent / name
        require(receipt.is_file() and sha256(receipt) == case.get("receipt_sha256"), f"case receipt hash mismatch: {name}")
        body = load(receipt)
        require(body.get("schema") == "hoi4-a24-producer-capture-case-v3", f"case schema mismatch: {name}")
        require(body.get("frame_hour") == case.get("frame_hour") and body.get("country_tag") == actions.get("country_tag"), f"case boundary mismatch: {name}")
        selected = body.get("selected_present")
        require(isinstance(selected, dict) and re.fullmatch(r"[0-9a-f]{64}", str(selected.get("receipt_sha256", ""))), f"case selected-Present is missing: {name}")
        taps = body.get("observed_tap_kinds")
        require(isinstance(taps, list) and set(taps) == set(case.get("observed_tap_kinds", [])), f"case tap summary mismatch: {name}")
        observed.update(taps); receipt_hashes.append(sha256(receipt))
    require(observed == REQUIRED_TAPS, "six-scene union does not cover the exact 23-tap matrix")

    require(grade.get("schema") == "hoi4-a24-live-fixture-regrade-v1", "regrade schema mismatch")
    require(
        grade.get("status")
        == "PASS_EXACT_A729_TAP_MATRIX_DIAGNOSTIC_ONLY_NO_PTI1_SEMANTICS",
        "regrade did not pass the diagnostic-only exact tap matrix",
    )
    require(grade.get("latch_review_authorized") is False, "tap-only grade authorized the semantic latch")
    require(grade.get("game_executable_sha256") == EXE_SHA256, "regrade executable identity mismatch")
    require(grade.get("producer_receipt_sha256s") == receipt_hashes, "regrade case order/hash binding mismatch")
    require(grade.get("frame_hours") == hours and grade.get("tap_kinds") == sorted(REQUIRED_TAPS), "regrade exact matrix summary mismatch")

    require(prebuilt.get("schema") == "battle-plan.central_rust_prebuilt_artifacts.v1" and prebuilt.get("successful_exit") is True, "central Release attestation is invalid")
    collector = next((row for row in prebuilt.get("artifacts", []) if row.get("role") == "collector_dll"), None)
    require(isinstance(collector, dict) and collector.get("sha256") == actions.get("collector_sha256"), "actions did not use the centrally attested collector")
    result = {
        "schema": "hoi4.a24.autonomous-live-admission.v1",
        "status": "PASS_IMMUTABLE_EXACT_A729_SIX_SURFACE_DIAGNOSTIC_ONLY",
        "game_executable_sha256": EXE_SHA256,
        "collector_sha256": actions["collector_sha256"],
        "prebuilt_attestation_sha256": sha256(args.prebuilt_attestation),
        "actions_sha256": sha256(args.actions),
        "regrade_sha256": sha256(args.regrade),
        "producer_receipt_sha256s": receipt_hashes,
        "frame_hours": hours,
        "country_tag": actions["country_tag"],
        "line_kinds": sorted(REQUIRED_LINE_KINDS),
        "tap_kinds": sorted(REQUIRED_TAPS),
        "human_input_required": False,
        "semantic_pti1_source_admitted": False,
        "production_activation_changed": False,
        "immutable": True,
    }
    atomic_new_json(args.output, result)
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
