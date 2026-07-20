#!/usr/bin/env python3
"""Deterministically maintain the Wave-2 exact-ten promotion plan.

This is a static plan builder.  It never contacts Ghidra, runs a verifier, or
reads/writes a broker receipt.  Existing replacement_result records are kept
for every unchanged semantic request hash.
"""

from __future__ import annotations

import copy
import hashlib
import json
from pathlib import Path


HERE = Path(__file__).resolve().parent
MANIFEST_PATH = HERE / "WAVE2_INVALID_20_SHARD_REEXPORT_MANIFEST_20260720.json"
PLAN_PATH = HERE / "WAVE2_10_SHARD_REEXPORT_RECONCILIATION_20260720.json"
EVIDENCE_ROOT = Path(r"C:\hoi4_evidence\ghidra_agent_exports")


def semantic_preimage(operation: str, request: dict) -> str:
    if operation == "search_strings":
        normalized = {"terms": request["terms"]}
    elif operation == "dump_data":
        normalized = {"addresses": request["addresses"]}
    elif operation == "export_functions":
        normalized = {
            "addresses": request["addresses"],
            "depth": request["depth"],
            "pcode": request["pcode"],
        }
    else:
        raise ValueError(operation)
    return json.dumps(
        {"operation": operation, "request": normalized},
        ensure_ascii=False,
        separators=(",", ":"),
    )


def semantic_hash(operation: str, request: dict) -> str:
    return hashlib.sha256(semantic_preimage(operation, request).encode("utf-8")).hexdigest()


def file_sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def pending_result(expected_hash: str) -> dict:
    return {
        "status": "pending_exact_10_reexport",
        "receipt_authoritative": False,
        "actual_job_id": None,
        "actual_shard_id": None,
        "job_json_path": None,
        "job_json_sha256": None,
        "actual_request_sha256": None,
        "expected_request_sha256": expected_hash,
        "request_hash_match": None,
        "identity_contract": None,
        "identity_manifest_sha256": None,
        "artifact": None,
        "artifact_sha256": None,
        "pool_status_receipt": {
            "status_snapshot_path": None,
            "status_snapshot_sha256": None,
            "observed_shard_count": None,
            "observed_ready_shards": None,
        },
        "planned_target_shard_must_match_actual": False,
    }


SUPPLEMENTAL_ACTIONS = [
    # D81 exact owner/live-object/frame route.  These entries were retained as
    # depth children, but the dossier explicitly requires direct seed receipts.
    ("D81-001", "0x140b626f0", 1, False, "D81_EXACT_RAIL_REPLAY_STATE", ["20260719T140113Z-wave2-hub-complete-owner-route-ctor-dtor-e43946b7"], "depth-1 child in the cited owner-route export; promoted as the dossier's exact function entry"),
    ("D81-002", "0x140b629c0", 1, False, "D81_EXACT_RAIL_REPLAY_STATE", ["20260719T134507Z-cap6-outer-gui-mapicons-loader-and-cgui--9e7245c4"], "depth-1 child in the cited manager construction export; promoted as the dossier's exact function entry"),
    ("D81-003", "0x140b63880", 1, True, "D81_EXACT_RAIL_REPLAY_STATE", ["20260716T033006Z-ui12-cmapicon-common-update-transform-cu-c7f848fb"], "depth-1 child in the cited active-vector export; promoted as the dossier's exact function entry"),
    ("D81-004", "0x140b64520", 1, False, "D81_EXACT_RAIL_REPLAY_STATE", ["20260719T140113Z-wave2-hub-complete-owner-route-ctor-dtor-e43946b7"], "depth-1 child in the cited owner-route export; promoted as the dossier's exact function entry"),
    ("D81-005", "0x14127afb0", 1, True, "D81_EXACT_RAIL_REPLAY_STATE", ["20260716T033006Z-ui12-cmapicon-common-update-transform-cu-c7f848fb"], "depth-1 child in the cited active-vector removal export; promoted as the dossier's exact function entry"),
    ("D81-006", "0x14162d360", 1, True, "D81_EXACT_RAIL_REPLAY_STATE", ["20260716T033006Z-ui12-cmapicon-common-update-transform-cu-c7f848fb"], "direct seed in the cited active-vector export"),
    ("D81-007", "0x1418fd210", 1, True, "D81_EXACT_RAIL_REPLAY_STATE", ["20260719T133218Z-wave2-supply-hub-overlay-frame-writer-6888abc1"], "depth-1 child in the cited frame-writer export; promoted as the dossier's exact function entry"),
    # X2 exact terminal-owner and synchronous width roots.
    ("X2-001", "0x141299610", 0, True, "X2_LAND_COMBAT_RESIDUALS", ["20260719T085156Z-x2-land-width-loss-outcome-leaves-2f6d1e11"], "direct depth-0 side-width producer seed from the retained lane authority"),
    ("X2-002", "0x1416dcdc0", 0, True, "X2_LAND_COMBAT_RESIDUALS", ["20260719T085156Z-x2-land-width-loss-outcome-leaves-2f6d1e11"], "direct depth-0 owning-vector join seed from the retained lane authority"),
    ("X2-003", "0x140cfd210", 0, True, "X2_LAND_COMBAT_RESIDUALS", ["20260719T085156Z-x2-land-width-loss-outcome-leaves-2f6d1e11"], "direct depth-0 owning-vector join seed from the retained lane authority"),
    ("X2-004", "0x140c6e270", 2, True, "X2_LAND_COMBAT_RESIDUALS", ["20260719T085900Z-x2-exact-division-width-leaf-4564eaf4"], "direct depth-2 mutable division-width leaf seed from the retained lane authority"),
    # Convoy terminal route roots named by the closeout but previously present
    # only as children of broader exports.
    ("CONVOY-001", "0x141961dd0", 1, True, "A6_A7_SUPPLY_CONVOY", ["20260719T132308Z-convoy-terminal-history-producer-consume-c00d80a7"], "depth-1 child in the cited terminal-history producer/consumer export"),
    ("CONVOY-002", "0x14195ba40", 1, False, "A6_A7_SUPPLY_CONVOY", ["20260719T140201Z-convoy-client-handle-constructor-source-ba39dcc9"], "depth-1 child in the cited client-handle constructor/source export"),
    # XN14Q consumes the frozen XN15L named-registry/factory/definition chain.
    ("WEATHER-001", "0x14227e5f0", 1, True, "WEATHER_ENTITY_REGISTRY", ["20260718T141909Z-xn15l-authority-entity-named-factory-sea-884fb864"], "depth-1 child and exact case-sensitive named-entity registry lookup"),
    ("WEATHER-002", "0x142278710", 1, True, "WEATHER_ENTITY_REGISTRY", ["20260718T141909Z-xn15l-authority-entity-named-factory-sea-884fb864"], "direct named-factory seed"),
    ("WEATHER-003", "0x142277df0", 1, True, "WEATHER_ENTITY_REGISTRY", ["20260718T141909Z-xn15l-authority-entity-named-factory-sea-884fb864", "20260718T141757Z-xn15l-authority-entity-definition-source-ebb05129"], "depth-1 child in the named-factory export and direct definition-backed instance-factory seed"),
    ("WEATHER-004", "0x142273fb0", 1, True, "WEATHER_ENTITY_REGISTRY", ["20260718T141757Z-xn15l-authority-entity-definition-source-ebb05129"], "depth-1 child proving entity construction retains the resolved definition"),
    # GH04 per-state topology body and naval activity discriminator.
    ("GH04-001", "0x1411b2420", 1, True, "A30_GH04", ["20260719T075226Z-a30-gh04-network-strength-coverage-produ-5617feb4"], "missing per-state subnetwork-index traversal body named by GH04; use the lane's depth-1 p-code contract"),
    ("NAVAL-001", "0x140d5a690", 1, True, "A8_A9_B6_NAVAL_COMBAT", ["20260719T140112Z-wave2-naval-mission-state-and-taskforce--4c8e0425"], "depth-1 child and exact direct-caller discriminator named by the naval closeout"),
]


CONSUMERS = {
    "D81_EXACT_RAIL_REPLAY_STATE": [
        "extractor/documents/dossiers/D81_EXACT_RAIL_REPLAY_STATE.json",
        "extractor/documents/dossiers/D81_EXACT_RAIL_REPLAY_STATE.md",
    ],
    "X2_LAND_COMBAT_RESIDUALS": [
        "extractor/re_evidence/X2_LAND_COMBAT_RESIDUALS/REPORT.md",
        "extractor/re_evidence/X2_LAND_COMBAT_RESIDUALS/HUMAN_INPUT_REQUIREMENTS.json",
    ],
    "A6_A7_SUPPLY_CONVOY": [
        "extractor/re_evidence/A6_A7_SUPPLY_CONVOY/TERMINAL_ROUTE_ATTRIBUTION_CLOSEOUT.md",
    ],
    "WEATHER_ENTITY_REGISTRY": [
        "replay_interface/documents/lane_reports/rendering_laws/XN14Q_RUNTIME_WIRING_AUDIT.md",
        "replay_interface/re_evidence/rendering_laws/XN15L/entity_selector_native_contract.json",
    ],
    "A30_GH04": ["extractor/documents/dossiers/A30/GH04.md"],
    "A8_A9_B6_NAVAL_COMBAT": [
        "extractor/native/lanes/naval_combat_live/A8_A9_B6_CLOSEOUT.md",
        "extractor/native/lanes/naval_combat_live/evidence/A8_A9_B6_ARTIFACT_MANIFEST.json",
    ],
}


ROOT_GROUPS = [
    ("SUP-01-D0-P1", 0, True),
    ("SUP-02-D1-P1", 1, True),
    ("SUP-03-D1-P0", 1, False),
    ("SUP-04-D2-P1", 2, True),
]


EXPECTED_FAILURES = [
    {
        "root_call_id": "EF-01-D0-P0",
        "source_sequence": 18,
        "depth": 0,
        "pcode": False,
        "job_id": "20260719T220039Z-wave2-r10-addr-01-02-562658b0",
        "superseded_root": "ADDR-01-02",
    },
    {
        "root_call_id": "EF-02-D1-P1",
        "source_sequence": 17,
        "depth": 1,
        "pcode": True,
        "job_id": "20260719T220127Z-wave2-r10-addr-03-01-631965a4",
        "superseded_root": "ADDR-03-01",
    },
]


def action_metadata(action: dict) -> tuple[list, list, list, list]:
    return (
        action.get("source_job_ids", []),
        action.get("originating_lanes", []),
        action.get("promotion_consumers", []),
        [action["sequence"]],
    )


def refresh_root(root: dict) -> None:
    request_hash = semantic_hash(root["operation"], root["request"])
    root["expected_request_sha256"] = request_hash
    root["request_hash_preimage"] = semantic_preimage(root["operation"], root["request"])
    if root.get("replacement_result", {}).get("expected_request_sha256") != request_hash:
        root["replacement_result"] = pending_result(request_hash)


def make_failure_root(spec: dict, action: dict) -> dict:
    request = {
        "label": "wave2-r10-" + spec["root_call_id"].lower() + "-isolated-known-failure",
        "addresses": ["0x140082cb0"],
        "depth": spec["depth"],
        "pcode": spec["pcode"],
    }
    expected = semantic_hash("export_functions", request)
    job_path = EVIDENCE_ROOT / spec["job_id"] / "job.json"
    job = json.loads(job_path.read_text(encoding="utf-8"))
    failed_preimage = semantic_preimage("export_functions", job["request"])
    return {
        "root_sequence": 0,
        "root_call_id": spec["root_call_id"],
        "target_shard_id": "shard-01",
        "operation": "export_functions",
        "compatibility_key": f"export_functions|depth={spec['depth']}|pcode={str(spec['pcode']).lower()}|expected-failure",
        "broker_address_limit": 32,
        "request": request,
        "source_action_sequences": [action["sequence"]],
        "supplemental_action_ids": [],
        "source_job_ids": action["source_job_ids"],
        "originating_lanes": action["originating_lanes"],
        "external_promotion_consumers": action["promotion_consumers"],
        "replacement_result": {
            **pending_result(expected),
            "status": "expected_failure_closed_no_retry",
            "expected_failure": True,
            "retry_required": False,
        },
        "known_failure_receipt": {
            "status": "failed",
            "receipt_authoritative_for_expected_failure_only": True,
            "actual_job_id": spec["job_id"],
            "actual_shard_id": job["shard"]["id"],
            "job_json_path": str(job_path),
            "job_json_sha256": file_sha256(job_path),
            "actual_batched_request_sha256": hashlib.sha256(failed_preimage.encode("utf-8")).hexdigest(),
            "actual_batched_request_preimage": failed_preimage,
            "isolated_expected_request_sha256": expected,
            "membership": {"addresses": ["0x140082cb0"], "depth": spec["depth"], "pcode": spec["pcode"]},
            "error": job["error"],
            "disposition": "The exact-ten retry required by the source action was attempted once and failed deterministically at this member. Do not retry; the surrounding batched members moved to a clean root.",
            "superseded_batched_root_call_id": spec["superseded_root"],
        },
        "target_shard_id_authority": "advisory_only_broker_may_choose_another_ready_shard",
        "expected_request_sha256": expected,
        "request_hash_preimage": semantic_preimage("export_functions", request),
        "external_consumer_claim_count": len(action["promotion_consumers"]),
    }


def make_supplemental_root(root_id: str, depth: int, pcode: bool, actions: list[dict]) -> dict:
    addresses = sorted(a["address"].lower() for a in actions)
    request = {
        "label": "wave2-r10-" + root_id.lower(),
        "addresses": addresses,
        "depth": depth,
        "pcode": pcode,
    }
    expected = semantic_hash("export_functions", request)
    return {
        "root_sequence": 0,
        "root_call_id": root_id,
        "target_shard_id": "shard-01",
        "operation": "export_functions",
        "compatibility_key": f"export_functions|depth={depth}|pcode={str(pcode).lower()}|supplemental",
        "broker_address_limit": 32,
        "request": request,
        "source_action_sequences": [],
        "supplemental_action_ids": [a["supplemental_action_id"] for a in actions],
        "source_job_ids": sorted({j for a in actions for j in a["source_job_ids"]}),
        "originating_lanes": sorted({a["originating_lane"] for a in actions}),
        "external_promotion_consumers": sorted({c for a in actions for c in a["promotion_consumers"]}),
        "replacement_result": pending_result(expected),
        "target_shard_id_authority": "advisory_only_broker_may_choose_another_ready_shard",
        "expected_request_sha256": expected,
        "request_hash_preimage": semantic_preimage("export_functions", request),
        "external_consumer_claim_count": len(actions),
    }


def main() -> None:
    manifest = json.loads(MANIFEST_PATH.read_text(encoding="utf-8"))
    plan = json.loads(PLAN_PATH.read_text(encoding="utf-8"))
    prior_roots = plan["consolidated_execution_plan"]["root_calls"]
    prior_results_by_hash = {
        root.get("expected_request_sha256"): copy.deepcopy(root.get("replacement_result"))
        for root in prior_roots
        if isinstance(root.get("expected_request_sha256"), str)
        and isinstance(root.get("replacement_result"), dict)
    }
    prior_failures_by_id = {
        root.get("root_call_id"): copy.deepcopy(root.get("known_failure_receipt"))
        for root in prior_roots
        if isinstance(root.get("known_failure_receipt"), dict)
    }
    manifest["execution_contract"].pop("no_discovery_expansion", None)
    manifest["execution_contract"]["canonical_replay_immutability"] = (
        "The 122 canonical actions remain byte-for-byte semantic replays of the discovery requests. "
        "Dossier-required exact function entries that were only transitive children are tracked solely "
        "as separately hashed supplemental_exact10_actions."
    )
    actions_by_sequence = {a["sequence"]: a for a in manifest["reexport_actions"]}
    roots = plan["consolidated_execution_plan"]["root_calls"]

    # Remove the deterministic failure member from both mixed batches.  The
    # existing root IDs stay with the clean remainders so every other queued
    # or completed semantic request remains stable.
    for root_id, sequence in (("ADDR-01-02", 18), ("ADDR-03-01", 17)):
        root = next(r for r in roots if r["root_call_id"] == root_id)
        root["request"]["addresses"] = [a for a in root["request"]["addresses"] if a.lower() != "0x140082cb0"]
        if not root["request"]["label"].endswith("-clean"):
            root["request"]["label"] += "-clean"
        root["source_action_sequences"] = [s for s in root["source_action_sequences"] if s != sequence]
        action = actions_by_sequence[sequence]
        other_actions = [actions_by_sequence[s] for s in root["source_action_sequences"]]
        root["source_job_ids"] = sorted({j for a in other_actions for j in a.get("source_job_ids", [])})
        root["originating_lanes"] = sorted({x for a in other_actions for x in a.get("originating_lanes", [])})
        root["external_promotion_consumers"] = sorted({x for a in other_actions for x in a.get("promotion_consumers", [])})
        root["external_consumer_claim_count"] = sum(bool(a.get("promotion_consumers")) for a in other_actions)
        refresh_root(root)

    roots[:] = [r for r in roots if not r["root_call_id"].startswith(("EF-", "SUP-0"))]
    for spec in EXPECTED_FAILURES:
        roots.append(make_failure_root(spec, actions_by_sequence[spec["source_sequence"]]))

    existing_supplemental = {
        a["supplemental_action_id"]: a for a in manifest.get("supplemental_exact10_actions", [])
        if a["supplemental_action_id"].startswith("SP-")
    }
    new_supplemental = []
    for action_id, address, depth, pcode, lane, source_jobs, resolution in SUPPLEMENTAL_ACTIONS:
        request = {"addresses": [address.lower()], "depth": depth, "pcode": pcode}
        new_supplemental.append({
            "supplemental_action_id": action_id,
            "address": address.lower(),
            "operation": "export_functions",
            "depth": depth,
            "pcode": pcode,
            "originating_lane": lane,
            "source_job_ids": source_jobs,
            "source_resolution": resolution,
            "promotion_consumers": CONSUMERS[lane],
            "semantic_request_sha256": semantic_hash("export_functions", request),
        })
    all_supplemental = list(existing_supplemental.values()) + new_supplemental
    manifest["supplemental_exact10_actions"] = all_supplemental
    manifest["counts"]["supplemental_exact10_actions"] = len(all_supplemental)

    for root_id, depth, pcode in ROOT_GROUPS:
        members = [a for a in new_supplemental if a["depth"] == depth and a["pcode"] is pcode]
        roots.append(make_supplemental_root(root_id, depth, pcode, members))

    # Assign each supplemental action to its compatibility root.
    root_for_supplemental = {
        action_id: root["root_call_id"]
        for root in roots
        for action_id in root.get("supplemental_action_ids", [])
    }
    for action in all_supplemental:
        if action["supplemental_action_id"].startswith("SP-"):
            action["root_call_id"] = "SP-01"
        else:
            action["root_call_id"] = root_for_supplemental[action["supplemental_action_id"]]

    # Stable sequence for old roots; appended failure/supplement roots follow.
    for index, root in enumerate(roots, 1):
        root["root_sequence"] = index
        root["target_shard_id"] = f"shard-{((index - 1) % 10) + 1:02d}"
        prior_result = prior_results_by_hash.get(root["expected_request_sha256"])
        if isinstance(prior_result, dict):
            root["replacement_result"] = prior_result
        prior_failure = prior_failures_by_id.get(root["root_call_id"])
        if isinstance(prior_failure, dict):
            root["known_failure_receipt"] = prior_failure
    root_by_id = {r["root_call_id"]: r for r in roots}
    index_by_id = {r["root_call_id"]: i for i, r in enumerate(roots)}

    for claim in plan["claim_reconciliation"]:
        if claim["source_action_sequence"] == 17:
            claim["replacement_root_call_id"] = "EF-02-D1-P1"
        elif claim["source_action_sequence"] == 18:
            claim["replacement_root_call_id"] = "EF-01-D0-P0"
        root = root_by_id[claim["replacement_root_call_id"]]
        claim["replacement_target_shard_id"] = root["target_shard_id"]
        claim["replacement_result_json_pointer"] = f"/consolidated_execution_plan/root_calls/{index_by_id[root['root_call_id']]}/replacement_result"
        claim["expected_root_request_sha256"] = root["expected_request_sha256"]
        claim["receipt_authority_json_pointer"] = claim["replacement_result_json_pointer"]
        claim["request_hash_mapping"]["root_expected_request_sha256"] = root["expected_request_sha256"]
        contract = claim["consumer_rewrite_contract"]
        pointer = claim["replacement_result_json_pointer"]
        contract["new_job_id_source"] = pointer + "/actual_job_id"
        contract["new_actual_shard_source"] = pointer + "/actual_shard_id"
        contract["new_artifact_source"] = pointer + "/artifact"
        contract["new_artifact_sha256_source"] = pointer + "/artifact_sha256"
        contract["new_identity_manifest_sha256_source"] = pointer + "/identity_manifest_sha256"
        if root["root_call_id"].startswith("EF-"):
            claim["promotion_rule"] = "The one required exact-ten retry failed deterministically. Retain the isolated expected-failure receipt; do not retry or fabricate an artifact."
            contract["rewrite_required"] = False
            contract["expected_failure_receipt_source"] = f"/consolidated_execution_plan/root_calls/{index_by_id[root['root_call_id']]}/known_failure_receipt"

    plan["supplemental_reconciliation"] = {
        "root_call_ids": ["SP-01"] + [x[0] for x in ROOT_GROUPS],
        "actions": all_supplemental,
        "coverage_audit": {
            "status": "complete_static_seed_census",
            "d81": {"required_functions": 15, "required_data": 13, "new_function_seeds": 7, "missing_after_plan": 0},
            "x2_land_terminal_width": {"new_function_seeds": 4, "missing_after_plan": 0},
            "convoy_terminal_route": {"required_functions": 12, "required_data": 2, "new_function_seeds": 2, "missing_after_plan": 0},
            "weather_entity_registry": {"new_function_seeds": 4, "missing_after_plan": 0},
            "a30_gh04": {"new_function_seeds": 1, "missing_after_plan": 0},
            "special_project_lifecycle_reward": {"required_functions": 13, "required_data": 2, "new_function_seeds": 0, "missing_after_plan": 0},
            "cap6": {"required_discovery_jobs": 6, "represented_discovery_jobs": 6, "new_seeds": 0},
            "s4": {"ghidra_seed_required": False, "new_seeds": 0, "reason": "S4 is the offline publication/repack integrity gate, not an executable RE lane."},
            "naval": {"minimum_required_functions": 4, "new_function_seeds": 1, "missing_after_plan": 0},
        },
        "consumer_rewrite_rule": "Every supplemental consumer must cite its actual broker job/shard, normalized request hash, identity-manifest hash, and artifact hash before promotion. Expected-failure roots instead cite their one deterministic exact-ten failure receipt.",
    }

    expected_failures = sum(r["root_call_id"].startswith("EF-") for r in roots)
    plan["counts"].update({
        "supplemental_exact10_actions": len(all_supplemental),
        "minimal_root_calls": len(roots),
        "address_root_calls": sum(r["operation"] != "search_strings" for r in roots),
        "search_root_calls": sum(r["operation"] == "search_strings" for r in roots),
        "expected_failure_root_calls": expected_failures,
        "pending_success_root_calls": len(roots) - expected_failures,
    })
    plan["promotion_admission"]["open_result_slots"] = len(roots) - expected_failures
    plan["promotion_admission"]["reason"] = "All successful root-call receipt slots and external consumer rewrites remain promotion-gated; two deterministic exact-ten decompile failures are isolated and closed without retry."
    plan["promotion_consumer_audit"]["supplemental_external_actions"] = len(all_supplemental)
    consumer_paths = sorted({p for a in all_supplemental for p in a["promotion_consumers"]} | set(plan["promotion_consumer_audit"]["consumer_paths"]))
    plan["promotion_consumer_audit"]["consumer_paths"] = consumer_paths
    plan["promotion_consumer_audit"]["distinct_external_consumer_paths"] = len(consumer_paths)
    plan["promotion_consumer_audit"]["open_receipts"] = len(roots) - expected_failures
    plan["promotion_consumer_audit"]["open_consumer_rewrites"] = plan["promotion_consumer_audit"]["external_claims"] + len(all_supplemental)
    plan["promotion_consumer_audit"]["root_calls_with_external_consumers"] = sum(bool(r["external_promotion_consumers"]) for r in roots)
    plan["failed_prejob_attempts"] = [x for x in plan["failed_prejob_attempts"] if not x.get("root_call_id", "").startswith("EF-")]
    for spec in EXPECTED_FAILURES:
        root = root_by_id[spec["root_call_id"]]
        plan["failed_prejob_attempts"].append({
            "root_call_id": spec["root_call_id"],
            "reason": root["known_failure_receipt"]["error"],
            "job_created": True,
            "receipt_authority": "expected_failure_only",
            "job_id": spec["job_id"],
            "disposition": root["known_failure_receipt"]["disposition"],
        })

    MANIFEST_PATH.write_text(json.dumps(manifest, separators=(",", ":"), ensure_ascii=False) + "\n", encoding="utf-8")
    PLAN_PATH.write_text(json.dumps(plan, separators=(",", ":"), ensure_ascii=False) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
