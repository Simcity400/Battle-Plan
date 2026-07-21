from __future__ import annotations

import argparse
import hashlib
import json
import re
from pathlib import Path


EXECUTABLE_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
PRODUCER_MANIFEST_SHA256 = "18bac19dc5de7fc0ef90a9341e0bc5ed1ac73c745a1dc96f6177ed08c8f70b35"
HELPER_MANIFEST_SHA256 = "989783832d3fe63a4f87bef2e2ba4e3c34c2048f11e87d55afb15164073e1503"
INTERNAL_MANIFEST_SHA256 = "ea5a7659b80b58cfc3f7e41d518da67b1d9114d9f966a66fe7c069a463f79eb1"
SENTINEL_ARTIFACT_SHA256 = "1bcf5829cd3a6af2ca1720ee26f63285ec374fdbc609adb27152769122203270"
MODIFIER_SENTINEL_ARTIFACT_SHA256 = "7f78ebae425582b1be1ab5ae9b3cd7f0d2c7a174cd4d526d0188b22f5d4e34fa"
INTERNAL_SENTINEL_ARTIFACT_SHA256 = "e354be21660d67e8589f3ff8e8ad98c8eb7b5d40859a67102449ce05754d00ab"
REQUIRED_KINDS = {
    "evaluated_trigger_result_and_reason_tree",
    "effective_hire_or_removal_cost",
    "aggregated_conditional_modifier",
    "advisor_slot_capacity_vacancy_eligibility_and_disabled_reason",
    "timed_dynamic_modifier_value_source_and_expiry",
    "displayed_party_popularity_change_source",
}
POINTER_TEXT = re.compile(r"(?i)(?:^|[^0-9a-f])0x[0-9a-f]{8,16}(?:$|[^0-9a-f])")
REQUIRED_TAPS = {
    "politics_entry_refresh",
    "advisor_compact_cost",
    "advisor_full_tooltip",
    "ideology_popularity_change",
    "timed_idea_tooltip",
    "active_idea_tooltip",
    "political_idea_tooltip",
    "idea_category_tooltip",
    "boost_party_tooltip",
    "idea_modifier_description",
    "idea_availability",
    "effective_idea_cost",
    "resource_affordability",
    "allowed_to_remove",
    "party_popularity_computation",
    "timed_idea_remaining_days",
    "country_can_take_ideas",
    "boost_party_why_text",
    "boost_party_name_text",
    "idea_selection_refresh",
    "idea_compact_cost_refresh",
    "idea_icon_and_modifier_bind",
    "modifier_value_lookup",
}
ROOT_TAPS = {
    "politics_entry_refresh",
    "advisor_compact_cost",
    "advisor_full_tooltip",
    "ideology_popularity_change",
    "timed_idea_tooltip",
    "active_idea_tooltip",
    "political_idea_tooltip",
    "idea_category_tooltip",
    "boost_party_tooltip",
    "idea_modifier_description",
    "idea_selection_refresh",
    "idea_compact_cost_refresh",
    "idea_icon_and_modifier_bind",
}


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def load_object(path: Path) -> dict:
    value = json.loads(path.read_text(encoding="utf-8-sig"))
    require(isinstance(value, dict), f"{path}: root is not an object")
    return value


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def no_pointer_text(value: object, path: str = "$") -> None:
    if isinstance(value, str):
        require(not POINTER_TEXT.search(value), f"{path}: process pointer leaked into durable fixture")
    elif isinstance(value, list):
        for index, child in enumerate(value):
            no_pointer_text(child, f"{path}[{index}]")
    elif isinstance(value, dict):
        for key, child in value.items():
            no_pointer_text(child, f"{path}.{key}")


def validate_reasons(nodes: object, name: str) -> None:
    require(isinstance(nodes, list), f"{name}: reason tree is not a list")
    prior = -1
    seen: set[str] = set()
    for node in nodes:
        require(isinstance(node, dict), f"{name}: reason node is not an object")
        ordinal = node.get("source_ordinal")
        node_id = node.get("node_id")
        parent = node.get("parent_node_id")
        require(isinstance(ordinal, int) and ordinal > prior, f"{name}: reason order invalid")
        require(isinstance(node_id, str) and node_id and node_id not in seen, f"{name}: reason identity invalid")
        require(parent is None or parent in seen, f"{name}: reason parent does not precede child")
        require(isinstance(node.get("source_identity"), str) and node["source_identity"], f"{name}: reason source missing")
        require(isinstance(node.get("result"), bool), f"{name}: reason result missing")
        require(isinstance(node.get("resolved_reason"), str) and node["resolved_reason"], f"{name}: resolved reason missing")
        prior = ordinal
        seen.add(node_id)


def validate_terms(terms: object, scale: int, total: int, name: str) -> None:
    require(isinstance(terms, list), f"{name}: source terms are not a list")
    prior = -1
    observed = 0
    for term in terms:
        require(isinstance(term, dict), f"{name}: source term is not an object")
        ordinal = term.get("source_ordinal")
        require(isinstance(ordinal, int) and ordinal > prior, f"{name}: source order invalid")
        require(term.get("scale") == scale, f"{name}: source scale mismatch")
        require(isinstance(term.get("raw_value"), int), f"{name}: raw source value missing")
        require(isinstance(term.get("source_identity"), str) and term["source_identity"], f"{name}: source identity missing")
        require(isinstance(term.get("evaluation_context"), str) and term["evaluation_context"], f"{name}: source context missing")
        observed += term["raw_value"]
        prior = ordinal
    require(observed == total, f"{name}: source sum differs from aggregate")


def validate_row(row: dict, frame_hour: int) -> None:
    require(row.get("frame_hour") == frame_hour, "row hour differs from PTI1 frame")
    tag = row.get("country_tag")
    require(isinstance(tag, str) and len(tag) == 3 and tag.isascii() and tag.isupper(), "country tag is not portable")
    for field in ("target_kind", "target_token_or_slot", "source_identity", "evaluation_context"):
        require(isinstance(row.get(field), str) and row[field], f"{field} is empty")
    require(row.get("availability_state") == "measured", "live promotion fixture requires measured rows")
    value = row.get("typed_value")
    require(isinstance(value, dict), "measured row has no typed value")
    kind = row.get("line_kind")
    require(kind in REQUIRED_KINDS, "unknown A24 line kind")
    if kind == "evaluated_trigger_result_and_reason_tree":
        require(value.get("kind") == "trigger_evaluation" and isinstance(value.get("result"), bool), "trigger value malformed")
        validate_reasons(value.get("reason_tree"), kind)
    elif kind == "effective_hire_or_removal_cost":
        require(value.get("kind") == "effective_cost", "cost value malformed")
        require(value.get("action") in {"hire", "remove"}, "cost action invalid")
        require(isinstance(value.get("raw_value"), int) and isinstance(value.get("scale"), int) and value["scale"] > 0, "raw cost invalid")
        require(isinstance(value.get("affordable"), bool), "cost affordability missing")
    elif kind == "aggregated_conditional_modifier":
        require(value.get("kind") == "conditional_modifier_aggregate", "modifier value malformed")
        validate_terms(value.get("terms"), value.get("scale"), value.get("raw_total"), kind)
    elif kind == "advisor_slot_capacity_vacancy_eligibility_and_disabled_reason":
        require(value.get("kind") == "advisor_slot_state", "slot value malformed")
        capacity, occupied = value.get("capacity"), value.get("occupied")
        require(isinstance(capacity, int) and isinstance(occupied, int) and 0 <= occupied <= capacity, "slot counts invalid")
        require(value.get("vacancy") is (occupied < capacity), "slot vacancy disagrees")
        require(isinstance(value.get("eligible"), bool), "slot eligibility missing")
        validate_reasons(value.get("disabled_reason_tree"), kind)
    elif kind == "timed_dynamic_modifier_value_source_and_expiry":
        require(value.get("kind") == "timed_dynamic_modifier", "timed modifier value malformed")
        require(isinstance(value.get("raw_value"), int) and isinstance(value.get("scale"), int) and value["scale"] > 0, "timed raw value invalid")
        start, end = value.get("applied_hour"), value.get("expiry_hour")
        require(start is None or isinstance(start, int), "applied hour invalid")
        require(end is None or isinstance(end, int), "expiry hour invalid")
        require(start is None or end is None or end >= start, "timed expiry precedes application")
    else:
        require(value.get("kind") == "party_popularity_change", "popularity value malformed")
        validate_terms(value.get("sources"), value.get("scale"), value.get("raw_change"), kind)


def validate_producer_case(path: Path) -> tuple[dict, list[dict]]:
    producer_receipt = load_object(path)
    require(producer_receipt.get("schema") == "hoi4-a24-producer-capture-case-v3", f"{path}: producer receipt schema mismatch")
    require(producer_receipt.get("status") == "complete_pointer_free_exact_a729_capture_case", f"{path}: producer receipt is not a complete capture case")
    require(producer_receipt.get("executable_sha256") == EXECUTABLE_SHA256, f"{path}: producer receipt executable mismatch")
    require(producer_receipt.get("producer_manifest_sha256") == PRODUCER_MANIFEST_SHA256, f"{path}: producer receipt manifest mismatch")
    require(producer_receipt.get("helper_manifest_sha256") == HELPER_MANIFEST_SHA256, f"{path}: producer helper manifest mismatch")
    require(producer_receipt.get("internal_producer_manifest_sha256") == INTERNAL_MANIFEST_SHA256, f"{path}: internal producer manifest mismatch")
    require(producer_receipt.get("sentinel_artifact_sha256") == SENTINEL_ARTIFACT_SHA256, f"{path}: producer sentinel artifact mismatch")
    require(producer_receipt.get("modifier_sentinel_artifact_sha256") == MODIFIER_SENTINEL_ARTIFACT_SHA256, f"{path}: modifier sentinel artifact mismatch")
    require(producer_receipt.get("internal_sentinel_artifact_sha256") == INTERNAL_SENTINEL_ARTIFACT_SHA256, f"{path}: internal sentinel artifact mismatch")
    frame_hour = producer_receipt.get("frame_hour")
    country_tag = producer_receipt.get("country_tag")
    arm_qpc = producer_receipt.get("arm_qpc_ticks")
    seal_qpc = producer_receipt.get("seal_qpc_ticks")
    selected_present = producer_receipt.get("selected_present")
    require(isinstance(frame_hour, int) and frame_hour >= 0, f"{path}: producer frame hour invalid")
    require(isinstance(country_tag, str) and re.fullmatch(r"[A-Z0-9]{3}", country_tag), f"{path}: producer country tag invalid")
    require(isinstance(arm_qpc, int) and arm_qpc > 0, f"{path}: producer arm QPC invalid")
    require(isinstance(seal_qpc, int) and seal_qpc >= arm_qpc, f"{path}: producer seal QPC invalid")
    require(isinstance(selected_present, dict), f"{path}: producer selected-Present binding missing")
    require(isinstance(selected_present.get("receipt_sha256"), str) and re.fullmatch(r"[0-9a-f]{64}", selected_present["receipt_sha256"]), f"{path}: producer selected-Present digest invalid")
    selected_qpc = selected_present.get("qpc_ticks")
    cutoff_qpc = selected_present.get("cutoff_qpc_ticks")
    require(isinstance(selected_qpc, int) and arm_qpc <= selected_qpc <= seal_qpc, f"{path}: selected-Present QPC is outside capture case")
    require(isinstance(cutoff_qpc, int) and cutoff_qpc >= selected_qpc, f"{path}: producer selected-Present cutoff invalid")
    observed_taps = producer_receipt.get("observed_tap_kinds")
    require(isinstance(observed_taps, list) and set(observed_taps) <= REQUIRED_TAPS and set(observed_taps) & ROOT_TAPS, f"{path}: capture case has no valid root tap")
    observations = producer_receipt.get("observations")
    require(isinstance(observations, list) and observations, f"{path}: producer observations missing")
    require({row.get("tap_kind") for row in observations if isinstance(row, dict)} == set(observed_taps), f"{path}: observed tap summary mismatch")
    require(all(isinstance(row, dict) and isinstance(row.get("stack_rvas"), list) and row["stack_rvas"] for row in observations), f"{path}: producer stack authority missing")
    invocation_keys = {(row.get("thread_id"), row.get("invocation_id")) for row in observations}
    sequences: set[int] = set()
    for row in observations:
        require(row.get("frame_hour") == frame_hour, f"{path}: producer observation hour mismatch")
        entry_qpc, return_qpc = row.get("entry_qpc_ticks"), row.get("return_qpc_ticks")
        require(isinstance(entry_qpc, int) and arm_qpc <= entry_qpc, f"{path}: producer entry precedes arm")
        require(isinstance(return_qpc, int) and entry_qpc <= return_qpc <= selected_qpc, f"{path}: producer return is outside selected-Present boundary")
        require(all(isinstance(rva, int) and 0 <= rva < 0x03000000 for rva in row["stack_rvas"]), f"{path}: producer native stack contains a non-image address")
        sequence = row.get("sequence")
        require(isinstance(sequence, int) and sequence not in sequences, f"{path}: producer sequence is missing or duplicated")
        sequences.add(sequence)
        parent = row.get("parent_invocation_id")
        require(parent is None or (row.get("thread_id"), parent) in invocation_keys, f"{path}: producer parent invocation join is missing")
        require(row.get("tap_kind") in ROOT_TAPS or parent is not None, f"{path}: helper tap has no same-case parent")
    no_pointer_text(producer_receipt)
    return producer_receipt, observations


def main() -> int:
    parser = argparse.ArgumentParser(description="Regrade an owner-captured A24 PTI1 v2 live fixture.")
    parser.add_argument("--producer-receipt", type=Path, required=True, action="append")
    parser.add_argument("--snapshot", type=Path)
    parser.add_argument("--game-exe", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    require(sha256(args.game_exe) == EXECUTABLE_SHA256, "game executable hash mismatch")
    cases = [validate_producer_case(path) for path in args.producer_receipt]
    producer_receipts = [case[0] for case in cases]
    all_observations = [row for case in cases for row in case[1]]
    frame_hours = {receipt["frame_hour"] for receipt in producer_receipts}
    country_tags = {receipt["country_tag"] for receipt in producer_receipts}
    case_ids = {receipt.get("capture_case_identity") for receipt in producer_receipts}
    observed_union = {row["tap_kind"] for row in all_observations}
    require(len(frame_hours) == 1 and len(country_tags) == 1, "producer cases do not share one game-hour/country boundary")
    require(None not in case_ids and len(case_ids) == len(producer_receipts), "producer capture-case identities are missing or duplicated")
    require(observed_union == REQUIRED_TAPS, "producer capture-case union does not cover the exact 23-tap matrix")
    frame_hour = next(iter(frame_hours))
    producer_receipt = producer_receipts[-1]
    observations = all_observations
    if args.snapshot is None:
        receipt = {
            "schema": "hoi4-a24-live-fixture-regrade-v1",
            "status": "PASS_EXACT_A729_TAP_MATRIX_READY_FOR_LATCH_REVIEW",
            "producer_receipt_paths": [str(path.resolve()) for path in args.producer_receipt],
            "producer_receipt_sha256s": [sha256(path) for path in args.producer_receipt],
            "game_executable_sha256": EXECUTABLE_SHA256,
            "producer_manifest_sha256": PRODUCER_MANIFEST_SHA256,
            "helper_manifest_sha256": HELPER_MANIFEST_SHA256,
            "frame_hour": producer_receipt.get("frame_hour"),
            "selected_present_receipt_sha256": producer_receipt.get("selected_present", {}).get("receipt_sha256"),
            "capture_case_count": len(producer_receipts),
            "observation_count": len(observations),
            "tap_kinds": sorted(REQUIRED_TAPS),
            "pointer_free": True,
            "d73_activation_changed": False,
        }
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        print(json.dumps(receipt, indent=2, sort_keys=True))
        return 0

    snapshot = load_object(args.snapshot)
    pti1 = snapshot.get("_r2_politics_tooltip_inputs_live")
    production = snapshot.get("production_render")
    require(isinstance(pti1, dict), "snapshot has no measured A24 fold")
    require(isinstance(production, dict), "snapshot has no production-render selected Present")
    require(pti1.get("schema_version") == 2, "A24 fixture is not PTI1 v2")
    require(pti1.get("authority") == "exact_a729_politics_producer", "A24 authority mismatch")
    require(pti1.get("producer_manifest_sha256") == PRODUCER_MANIFEST_SHA256, "producer manifest mismatch")
    frame_hour = pti1.get("frame_hour")
    require(isinstance(frame_hour, int) and frame_hour >= 0, "A24 frame hour invalid")
    selected = production.get("selected_present")
    require(isinstance(selected, dict), "production render selected Present missing")
    matching_case = next((receipt for receipt in producer_receipts if receipt["frame_hour"] == frame_hour and receipt["selected_present"].get("receipt_sha256") == pti1.get("selected_present_receipt_sha256")), None)
    require(matching_case is not None, "PTI1 selected-Present is not bound to any producer capture case")
    require(matching_case["selected_present"].get("candidate_id") == pti1.get("selected_present_candidate_id"), "producer-case candidate mismatch")
    require(matching_case["selected_present"].get("ordinal") == pti1.get("selected_present_ordinal"), "producer-case ordinal mismatch")
    require(matching_case["selected_present"].get("qpc_ticks") == pti1.get("selected_present_qpc_ticks"), "producer-case QPC mismatch")
    require(matching_case["selected_present"].get("cutoff_qpc_ticks") == pti1.get("selected_present_cutoff_qpc_ticks"), "producer-case cutoff mismatch")
    require(selected.get("selected_candidate_id") == pti1.get("selected_present_candidate_id"), "selected candidate mismatch")
    require(selected.get("selected_ordinal") == pti1.get("selected_present_ordinal"), "selected ordinal mismatch")
    require(selected.get("selected_qpc_ticks") == pti1.get("selected_present_qpc_ticks"), "selected QPC mismatch")
    require(selected.get("selection_cutoff_qpc_ticks") == pti1.get("selected_present_cutoff_qpc_ticks"), "selected cutoff mismatch")
    receipt_sha256 = pti1.get("selected_present_receipt_sha256")
    require(isinstance(receipt_sha256, str) and re.fullmatch(r"[0-9a-f]{64}", receipt_sha256), "selected-Present receipt hash invalid")
    rows = pti1.get("rows")
    require(isinstance(rows, list) and len(rows) == pti1.get("expected_row_count"), "A24 row completeness mismatch")
    require({row.get("line_kind") for row in rows if isinstance(row, dict)} == REQUIRED_KINDS, "A24 fixture does not cover exactly all six families")
    keys: set[tuple[object, ...]] = set()
    for row in rows:
        require(isinstance(row, dict), "A24 row is not an object")
        validate_row(row, frame_hour)
        key = (row.get("country_tag"), row.get("target_kind"), row.get("target_token_or_slot"), row.get("line_kind"))
        require(key not in keys, "duplicate A24 row key")
        keys.add(key)
    no_pointer_text(pti1)

    receipt = {
        "schema": "hoi4-a24-live-fixture-regrade-v1",
        "status": "PASS_EXACT_A729_PTI1_V2_SELECTED_PRESENT",
        "snapshot_path": str(args.snapshot.resolve()),
        "snapshot_sha256": sha256(args.snapshot),
        "producer_receipt_paths": [str(path.resolve()) for path in args.producer_receipt],
        "producer_receipt_sha256s": [sha256(path) for path in args.producer_receipt],
        "capture_case_count": len(producer_receipts),
        "game_executable_sha256": EXECUTABLE_SHA256,
        "producer_manifest_sha256": PRODUCER_MANIFEST_SHA256,
        "frame_hour": frame_hour,
        "selected_present_receipt_sha256": receipt_sha256,
        "completeness_record_identity": pti1.get("completeness_record_identity"),
        "row_count": len(rows),
        "line_kinds": sorted(REQUIRED_KINDS),
        "pointer_free": True,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(receipt, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
