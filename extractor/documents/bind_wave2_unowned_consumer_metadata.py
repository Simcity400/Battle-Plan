#!/usr/bin/env python3
"""Insert exact-ten receipt metadata into central/unowned Wave-2 consumers.

This is a deterministic documentation/authority rewrite.  It does not touch
lane implementation code, contact Ghidra, or execute any verifier.
"""

from __future__ import annotations

import json
from pathlib import Path


HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
RECONCILIATION_PATH = HERE / "WAVE2_10_SHARD_REEXPORT_RECONCILIATION_20260720.json"

CONSUMERS = [
    "extractor/documents/XUI21_RE_EVIDENCE.json",
    "extractor/documents/dossiers/A28/FACILITY_SPECIALIZATION_SMOOTH_LIST_HOOK_CONTRACT_20260719.json",
    "extractor/documents/dossiers/A28/REMAINING_CONTROLLER_AUDIT.json",
    "extractor/documents/dossiers/A28/SOURCE_EXHAUSTION_CENSUS_20260719.json",
    "extractor/documents/dossiers/A28_DYNAMIC_COLLECTION_TRACE.json",
    "extractor/documents/dossiers/A30/GH04.md",
    "extractor/documents/dossiers/X4_PRODUCTION_LEGACY_RESIDUAL_CLOSEOUT_20260719.md",
    "extractor/native/lanes/occupation_resistance_compliance_live/GARRISON_RESIDUAL_ANALYSIS.md",
    "extractor/native/lanes/occupation_resistance_compliance_live/GARRISON_SCALAR_STATIC_COMPLETION_20260719.json",
    "replay_interface/re_evidence/rendering_laws/C02/INTEGRATION_STATUS.md",
    "replay_interface/re_evidence/rendering_laws/XUI21/ghidra_producer_contract.json",
    "replay_interface/re_evidence/rendering_laws/XUI21/owner_classifier_contract.json",
]

BEGIN = "<!-- WAVE2_EXACT10_CONSUMER_BINDING_BEGIN -->"
END = "<!-- WAVE2_EXACT10_CONSUMER_BINDING_END -->"


def receipt(root: dict) -> dict:
    root_id = root["root_call_id"]
    if root_id.startswith("EF-"):
        value = root["known_failure_receipt"]
        return {
            "binding_kind": "expected_failure",
            "status": value["status"],
            "actual_job_id": value["actual_job_id"],
            "actual_shard_id": value["actual_shard_id"],
            "job_json_path": value["job_json_path"],
            "job_json_sha256": value["job_json_sha256"],
            "actual_batched_request_sha256": value["actual_batched_request_sha256"],
            "isolated_expected_request_sha256": value["isolated_expected_request_sha256"],
            "identity_manifest_sha256": value["identity_manifest_sha256"],
            "pool_status_path": value["pool_status_receipt"]["status_snapshot_path"],
            "pool_status_sha256": value["pool_status_receipt"]["status_snapshot_sha256"],
            "error": value["error"],
            "artifact": None,
            "artifact_sha256": None,
            "authority_scope": "required one-retry deterministic negative outcome only",
        }
    value = root["replacement_result"]
    return {
        "binding_kind": "successful_artifact",
        "status": value["status"],
        "receipt_authoritative": value["receipt_authoritative"],
        "actual_job_id": value["actual_job_id"],
        "actual_shard_id": value["actual_shard_id"],
        "job_json_path": value["job_json_path"],
        "job_json_sha256": value["job_json_sha256"],
        "actual_request_sha256": value["actual_request_sha256"],
        "identity_manifest_sha256": value["identity_manifest_sha256"],
        "artifact": value["artifact"],
        "artifact_sha256": value["artifact_sha256"],
        "pool_status_path": value["pool_status_receipt"]["status_snapshot_path"],
        "pool_status_sha256": value["pool_status_receipt"]["status_snapshot_sha256"],
    }


def binding_for(path: str, reconciliation: dict) -> dict:
    roots = reconciliation["consolidated_execution_plan"]["root_calls"]
    root_by_id = {root["root_call_id"]: root for root in roots}
    claims = [
        claim for claim in reconciliation["claim_reconciliation"]
        if path in claim.get("external_promotion_consumers", [])
    ]
    supplemental = [
        action for action in reconciliation["supplemental_reconciliation"]["actions"]
        if path in action.get("promotion_consumers", [])
    ]
    root_ids = sorted(
        {claim["replacement_root_call_id"] for claim in claims}
        | {action["root_call_id"] for action in supplemental}
    )
    return {
        "schema": "hoi4-wave2-exact10-inline-consumer-binding-v1",
        "status": "exact10_consumer_rewrite_complete",
        "consumer_path": path,
        "reconciliation_path": "extractor/documents/WAVE2_10_SHARD_REEXPORT_RECONCILIATION_20260720.json",
        "binary_id": "hoi4.exe 1.19.2.0.a729",
        "image_base": "0x140000000",
        "sha256": "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad",
        "old_discovery_jobs_are_non_promotable_aliases_only": True,
        "lane_activation_unchanged": True,
        "roots": [
            {
                "root_call_id": root_id,
                "canonical_claims": [
                    {
                        "discovery_job_id": claim["discovery_job_id"],
                        "source_action_sequence": claim["source_action_sequence"],
                        "source_semantic_request_sha256": claim["source_semantic_request_sha256"],
                        "request_membership": claim["request_membership"],
                    }
                    for claim in claims
                    if claim["replacement_root_call_id"] == root_id
                ],
                "supplemental_action_ids": sorted(
                    action["supplemental_action_id"]
                    for action in supplemental
                    if action["root_call_id"] == root_id
                ),
                "receipt": receipt(root_by_id[root_id]),
            }
            for root_id in root_ids
        ],
    }


def main() -> None:
    reconciliation = json.loads(RECONCILIATION_PATH.read_text(encoding="utf-8"))
    mapped = set(reconciliation["promotion_consumer_audit"]["consumer_paths"])
    for relative in CONSUMERS:
        if relative not in mapped:
            raise SystemExit(f"unmapped consumer in rewrite list: {relative}")
        path = REPO / relative
        binding = binding_for(relative, reconciliation)
        if path.suffix.lower() == ".json":
            data = json.loads(path.read_text(encoding="utf-8"))
            data["wave2_exact10_consumer_binding"] = binding
            path.write_text(json.dumps(data, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
        elif path.suffix.lower() == ".md":
            text = path.read_text(encoding="utf-8")
            if BEGIN in text:
                text = text.split(BEGIN, 1)[0].rstrip() + "\n"
            block = (
                f"\n{BEGIN}\n"
                "## Exact-ten replacement authority\n\n"
                "This machine-readable block replaces discovery-only custody with exact-ten receipts. "
                "It does not alter the lane's independent runtime or human activation gates.\n\n"
                "```json\n"
                + json.dumps(binding, indent=2, ensure_ascii=False)
                + "\n```\n"
                + END
                + "\n"
            )
            path.write_text(text + block, encoding="utf-8")
        else:
            raise SystemExit(f"unsupported consumer type: {relative}")


if __name__ == "__main__":
    main()
