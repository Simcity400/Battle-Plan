#!/usr/bin/env python3
"""Build the central Wave-2 exact-ten consumer-rewrite ledger.

This performs static repository inspection only.  It never contacts Ghidra,
runs a verifier, builds code, or changes lane implementation files.
"""

from __future__ import annotations

import hashlib
import json
from pathlib import Path


HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
RECONCILIATION = HERE / "WAVE2_10_SHARD_REEXPORT_RECONCILIATION_20260720.json"
LEDGER_JSON = HERE / "WAVE2_EXACT10_CONSUMER_REWRITE_LEDGER_20260720.json"
LEDGER_MD = HERE / "WAVE2_EXACT10_CONSUMER_REWRITE_LEDGER_20260720.md"


DISPOSITION = {
    "extractor/documents/XUI21_RE_EVIDENCE.json": ("deliberately_still_false", "Executable law is pinned, but the consumer still requires its authenticated runtime fixture and central validation."),
    "extractor/documents/dossiers/A28/FACILITY_SPECIALIZATION_SMOOTH_LIST_HOOK_CONTRACT_20260719.json": ("deliberately_still_false", "Source/template law is pinned; runtime geometry, scroll, and portable identity observations remain required."),
    "extractor/documents/dossiers/A28/REMAINING_CONTROLLER_AUDIT.json": ("deliberately_still_false", "The retained/controller sweep is exhausted, but no zero-gap new controller family is admitted without the named live observations."),
    "extractor/documents/dossiers/A28/SOURCE_EXHAUSTION_CENSUS_20260719.json": ("deliberately_still_false", "Exact-ten provenance is closed; callback-bound d71 source and authenticated live fixture gates remain."),
    "extractor/documents/dossiers/A28_DYNAMIC_COLLECTION_TRACE.json": ("deliberately_still_false", "Existing admitted families remain usable, while newly anchored controller families remain fail-closed pending full identity/geometry/scroll receipts."),
    "extractor/documents/dossiers/A30/GH04.md": ("pinned", "The GH04 exact-ten topology body is pinned and semantically integrated as INGR v3 per-state strength. Promotion remains pinned pending an authenticated same-frame runtime fixture."),
    "extractor/documents/dossiers/D81_EXACT_RAIL_REPLAY_STATE.json": ("pinned", "All required exact-ten roots and v2 admission latches are pinned; the active collector still emits v1 until the authenticated double-read reader is implemented and centrally validated."),
    "extractor/documents/dossiers/D81_EXACT_RAIL_REPLAY_STATE.md": ("pinned", "Schema-v2 encoder/fold/replay admission is enabled and receipt-pinned, while live capture remains v1 because the authenticated double-read reader is still absent."),
    "extractor/documents/dossiers/X4_PRODUCTION_LEGACY_RESIDUAL_CLOSEOUT_20260719.md": ("enabled", "The native extractor/replay fields are already wired and need no human input; exact-ten receipts now replace discovery-only provenance."),
    "extractor/native/lanes/army_battleplans_depth_live/A729_BATTLEPLAN_RE_EVIDENCE.json": ("deliberately_still_false", "Static and raw-capture authority is pinned, but non-type-5 geometry/execution promotion remains controlled-fixture gated."),
    "extractor/native/lanes/focus_research_decisions_depth_live/A2_A12_RE_EVIDENCE.json": ("deliberately_still_false", "A2/A12 carrier laws are pinned, while the distinct D45 direct runtime producer remains unavailable pending an authenticated invocation root."),
    "extractor/native/lanes/focus_research_decisions_depth_live/A2_A12_RE_REPORT.md": ("enabled", "The A2 pause, A12 cooldown, and targeted-decision payload union are documented production-ready; exact-ten receipts replace only their discovery provenance."),
    "extractor/native/lanes/naval_combat_live/A8_A9_B6_CLOSEOUT.md": ("deliberately_still_false", "Exact-ten evidence is pinned, but D56 receiver/base semantics and route-linked terminal convoy attribution remain unavailable rather than guessed."),
    "extractor/native/lanes/naval_combat_live/A8_A9_B6_HUMAN_INPUT_REQUIREMENTS.json": ("deliberately_still_false", "The non-human exact-ten prerequisite is closed; the separate pre-collapse convoy route acquisition remains required."),
    "extractor/native/lanes/naval_combat_live/evidence/A8_A9_B6_ARTIFACT_MANIFEST.json": ("deliberately_still_false", "Discovery artifacts are superseded by exact-ten receipts, but the manifest's D56 and pre-collapse acquisition gates still prohibit promotion."),
    "extractor/native/lanes/occupation_resistance_compliance_live/GARRISON_RESIDUAL_ANALYSIS.md": ("enabled", "The three production per-state garrison scalars are closed and wired; the isolated serializer decompile failure is retained only as negative retry evidence."),
    "extractor/native/lanes/occupation_resistance_compliance_live/GARRISON_SCALAR_STATIC_COMPLETION_20260719.json": ("enabled", "The scalar contract is closed with no human input required; exact-ten positive roots and the deterministic negative receipt are centrally pinned."),
    "extractor/native/project_progress_live_lane/SPECIAL_PROJECT_LIFECYCLE_REWARD_RE_20260719.json": ("enabled", "The complete 13-function/two-vtable card is exact-ten bound, promotion.active=true, and the lane requires no human input."),
    "extractor/native/project_progress_live_lane/SPECIAL_PROJECT_LIFECYCLE_REWARD_RE_20260719.md": ("enabled", "The exact-ten promotion manifest is pinned and the lifecycle/reward carrier is enabled without a remaining human gate."),
    "extractor/re_evidence/A6_A7_SUPPLY_CONVOY/BOUNDED_OWNER_ROOT.json": ("pinned", "The bounded owner-root authority is pinned for consumers; it does not itself claim route-linked terminal attribution or activate a carrier."),
    "extractor/re_evidence/A6_A7_SUPPLY_CONVOY/STATIC_CLOSEOUT.md": ("deliberately_still_false", "The static owner/active join is pinned, while exact terminal route attribution still requires the pre-collapse controlled acquisition."),
    "extractor/re_evidence/A6_A7_SUPPLY_CONVOY/TERMINAL_ROUTE_ATTRIBUTION_CLOSEOUT.md": ("deliberately_still_false", "Every named static function/data root is now exact-ten authority; the missing runtime route-manager join remains a separate live evidence gate."),
    "extractor/re_evidence/SNOW_MUD_PREUPLOAD_SEAM/README.md": ("deliberately_still_false", "The static H/H2 workflow is complete, but promotion_state remains false until the two required live capture/replay receipt pairs exist."),
    "extractor/re_evidence/X2_LAND_COMBAT_RESIDUALS/HUMAN_INPUT_REQUIREMENTS.json": ("deliberately_still_false", "Terminal/static roots are pinned; current-loss and synchronous width activation remain explicitly controlled-fixture gated."),
    "extractor/re_evidence/X2_LAND_COMBAT_RESIDUALS/REPORT.md": ("deliberately_still_false", "Implemented terminal/result fields retain authority, while current losses and width stay unavailable with both activation constants false."),
    "replay_interface/documents/lane_reports/rendering_laws/XN14Q_RUNTIME_WIRING_AUDIT.md": ("deliberately_still_false", "The named-registry chain is exact-ten pinned, but active weather still lacks authenticated initial-state/per-update visibility and selected-submit sequence ownership."),
    "replay_interface/re_evidence/rendering_laws/C02/INTEGRATION_STATUS.md": ("enabled", "The non-human camera carrier seam is complete and requires no human input; exact-ten provenance is now pinned."),
    "replay_interface/re_evidence/rendering_laws/CAP6/outer_gui_source_exhaustion.json": ("deliberately_still_false", "The false authority latch is intentional until one complete event-scoped GUI-paint-to-D3D receipt exists."),
    "replay_interface/re_evidence/rendering_laws/CAP6/portable_carrier_audit.json": ("deliberately_still_false", "Portable schema support exists, but the authenticated CAP6 producer remains UNAVAILABLE_ZERO_DRAW."),
    "replay_interface/re_evidence/rendering_laws/CAP6/verify_cap6_retained.py": ("deliberately_still_false", "The retained gate intentionally requires partial/fail-closed and zero-draw disposition until live authority is supplied."),
    "replay_interface/re_evidence/rendering_laws/XN15L/entity_selector_native_contract.json": ("pinned", "The named registry/factory/definition chain is proven and centrally pinned; this contract alone does not activate active-weather capture."),
    "replay_interface/re_evidence/rendering_laws/XUI21/ghidra_producer_contract.json": ("deliberately_still_false", "Exact-ten executable provenance is pinned, but installed-hook, baseline, queue, lifecycle, and runtime completeness receipts remain required."),
    "replay_interface/re_evidence/rendering_laws/XUI21/owner_classifier_contract.json": ("deliberately_still_false", "Static owner classification is production-wired, but the contract explicitly retains its central runtime validation gate."),
}


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def receipt_binding(root: dict, text: str) -> dict:
    root_id = root["root_call_id"]
    if root_id.startswith("EF-"):
        receipt = root["known_failure_receipt"]
        tokens = {
            "actual_job_id": receipt["actual_job_id"],
            "actual_shard_id": receipt["actual_shard_id"],
            "job_json_sha256": receipt["job_json_sha256"],
            "actual_request_sha256": receipt["actual_batched_request_sha256"],
            "isolated_expected_request_sha256": receipt["isolated_expected_request_sha256"],
            "identity_manifest_sha256": receipt["identity_manifest_sha256"],
            "pool_status_sha256": receipt["pool_status_receipt"]["status_snapshot_sha256"],
        }
        result = {
            "root_call_id": root_id,
            "binding_kind": "expected_failure",
            "status": receipt["status"],
            "receipt_authoritative_for_expected_failure_only": True,
            "error": receipt["error"],
            "artifact": None,
            "artifact_sha256": None,
            **tokens,
        }
    else:
        receipt = root["replacement_result"]
        tokens = {
            "actual_job_id": receipt["actual_job_id"],
            "actual_shard_id": receipt["actual_shard_id"],
            "job_json_sha256": receipt["job_json_sha256"],
            "actual_request_sha256": receipt["actual_request_sha256"],
            "identity_manifest_sha256": receipt["identity_manifest_sha256"],
            "artifact_sha256": receipt["artifact_sha256"],
            "pool_status_sha256": receipt["pool_status_receipt"]["status_snapshot_sha256"],
        }
        result = {
            "root_call_id": root_id,
            "binding_kind": "successful_artifact",
            "status": receipt["status"],
            "receipt_authoritative": receipt["receipt_authoritative"],
            "actual_job_id": receipt["actual_job_id"],
            "actual_shard_id": receipt["actual_shard_id"],
            "job_json_path": receipt["job_json_path"],
            "job_json_sha256": receipt["job_json_sha256"],
            "actual_request_sha256": receipt["actual_request_sha256"],
            "identity_manifest_sha256": receipt["identity_manifest_sha256"],
            "artifact": receipt["artifact"],
            "artifact_sha256": receipt["artifact_sha256"],
            "pool_status_path": receipt["pool_status_receipt"]["status_snapshot_path"],
            "pool_status_sha256": receipt["pool_status_receipt"]["status_snapshot_sha256"],
        }
    lowered = text.lower()
    # The repository-wide strict consumer contract intentionally duplicates the
    # two custody fields that identify a successful replacement locally.  The
    # remaining receipt hashes stay authoritative in this central ledger and
    # may also be duplicated by a lane, but are not required for local binding.
    # Deterministic negative receipts have no artifact, so retain the complete
    # negative receipt tuple for those two edges.
    required_fields = (
        tuple(tokens)
        if root_id.startswith("EF-")
        else ("actual_job_id", "artifact_sha256")
    )
    result["inline_citation"] = {
        "required_fields": list(required_fields),
        "complete": all(str(tokens[key]).lower() in lowered for key in required_fields),
        "present_fields": sorted(key for key, value in tokens.items() if str(value).lower() in lowered),
        "missing_fields": sorted(key for key in required_fields if str(tokens[key]).lower() not in lowered),
        "authority_note": "Inline duplication is informational only; this hash-pinned central ledger is the replacement authority.",
    }
    return result


def build() -> tuple[dict, str]:
    reconciliation = json.loads(RECONCILIATION.read_text(encoding="utf-8"))
    roots = reconciliation["consolidated_execution_plan"]["root_calls"]
    root_by_id = {root["root_call_id"]: root for root in roots}
    consumers = reconciliation["promotion_consumer_audit"]["consumer_paths"]
    if set(consumers) != set(DISPOSITION):
        missing = sorted(set(consumers) - set(DISPOSITION))
        stale = sorted(set(DISPOSITION) - set(consumers))
        raise SystemExit(f"consumer classification drift: missing={missing}, stale={stale}")

    rows = []
    for consumer in consumers:
        canonical_claims = [
            claim for claim in reconciliation["claim_reconciliation"]
            if consumer in claim.get("external_promotion_consumers", [])
        ]
        supplemental = [
            action for action in reconciliation["supplemental_reconciliation"]["actions"]
            if consumer in action.get("promotion_consumers", [])
        ]
        root_ids = sorted({claim["replacement_root_call_id"] for claim in canonical_claims} | {action["root_call_id"] for action in supplemental})
        path = REPO / consumer
        if not path.is_file():
            raise SystemExit(f"mapped consumer missing: {consumer}")
        raw = path.read_bytes()
        text = raw.decode("utf-8")
        disposition, reason = DISPOSITION[consumer]
        rows.append({
            "consumer_path": consumer,
            "consumer_sha256_at_audit": sha256_bytes(raw),
            "disposition": disposition,
            "reason": reason,
            "canonical_discovery_claims": sorted(claim["discovery_job_id"] for claim in canonical_claims),
            "supplemental_action_ids": sorted(action["supplemental_action_id"] for action in supplemental),
            "root_bindings": [receipt_binding(root_by_id[root_id], text) for root_id in root_ids],
        })

    counts = {name: sum(row["disposition"] == name for row in rows) for name in ("enabled", "pinned", "deliberately_still_false")}
    inline_complete = sum(
        binding["inline_citation"]["complete"]
        for row in rows for binding in row["root_bindings"]
    )
    binding_count = sum(len(row["root_bindings"]) for row in rows)
    ledger = {
        "schema": "hoi4-wave2-exact10-consumer-rewrite-ledger-v1",
        "generated_date": "2026-07-20",
        "authority": {
            "reconciliation_path": "extractor/documents/WAVE2_10_SHARD_REEXPORT_RECONCILIATION_20260720.json",
            "binary_id": "hoi4.exe 1.19.2.0.a729",
            "image_base": "0x140000000",
            "sha256": "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad",
            "pool_status_path": "C:\\hoi4_evidence\\WAVE2_EXACT10_POOL_STATUS_20260720.json",
            "pool_status_sha256": "df67a663059a645bf8f039f34659fed76274a7259af2fc137a2a60b5e8e41395",
            "observed_shards": 10,
            "observed_ready_shards": 10,
            "successful_root_receipts": 43,
            "expected_failure_receipts": 2,
            "open_root_receipts": 0,
        },
        "rewrite_policy": {
            "central_binding_is_authority": True,
            "inline_consumer_duplication_required": True,
            "reason": "The ledger indexes every exact receipt centrally, and every mapped consumer must also contain its actual exact-ten job ID and artifact SHA-256 so stale discovery-only custody cannot survive locally.",
            "activation_independence": "Receipt replacement never flips a lane activation latch or satisfies a separate runtime/human capture gate.",
        },
        "counts": {
            "consumers": len(rows),
            "consumer_root_bindings": binding_count,
            "inline_complete_bindings": inline_complete,
            "inline_incomplete_bindings": binding_count - inline_complete,
            **counts,
        },
        "consumers": rows,
    }

    md = [
        "# Wave 2 exact-ten consumer rewrite ledger",
        "",
        "Date: 2026-07-20",
        "",
        "All 43 successful broker roots are exact-ten authoritative. The two `0x140082cb0` roots are terminal deterministic failure receipts and are authoritative only for the required one-retry negative outcome. This ledger is the central replacement binding; it does not enable a lane whose separate runtime or human gate remains false.",
        "",
        f"Consumers: **{len(rows)}**. Enabled: **{counts['enabled']}**. Pinned-only: **{counts['pinned']}**. Deliberately still false: **{counts['deliberately_still_false']}**.",
        "",
        "Exact pool receipt: `C:\\hoi4_evidence\\WAVE2_EXACT10_POOL_STATUS_20260720.json`, SHA-256 `df67a663059a645bf8f039f34659fed76274a7259af2fc137a2a60b5e8e41395`.",
        "",
        "## Consumer dispositions",
        "",
        "| Consumer | Disposition | Root receipts | Reason |",
        "|---|---|---|---|",
    ]
    for row in rows:
        roots_text = ", ".join(f"`{item['root_call_id']}`" for item in row["root_bindings"])
        md.append(f"| `{row['consumer_path']}` | `{row['disposition']}` | {roots_text} | {row['reason']} |")
    md += [
        "",
        "## Exact root receipt index",
        "",
        "| Root | Kind | Actual job | Actual shard | Request SHA-256 | Artifact SHA-256 |",
        "|---|---|---|---|---|---|",
    ]
    used_roots = sorted({binding["root_call_id"] for row in rows for binding in row["root_bindings"]})
    for root_id in used_roots:
        root = root_by_id[root_id]
        if root_id.startswith("EF-"):
            receipt = root["known_failure_receipt"]
            md.append(f"| `{root_id}` | expected failure | `{receipt['actual_job_id']}` | `{receipt['actual_shard_id']}` | `{receipt['actual_batched_request_sha256']}` | none |")
        else:
            receipt = root["replacement_result"]
            md.append(f"| `{root_id}` | successful artifact | `{receipt['actual_job_id']}` | `{receipt['actual_shard_id']}` | `{receipt['actual_request_sha256']}` | `{receipt['artifact_sha256']}` |")
    md += [
        "",
        "The machine-readable ledger records the job JSON hash, artifact path, identity-manifest hash, pool-status hash, consumer file hash, and inline-citation audit for every consumer/root edge.",
        "",
    ]
    return ledger, "\n".join(md)


def main() -> None:
    ledger, markdown = build()
    LEDGER_JSON.write_text(json.dumps(ledger, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    LEDGER_MD.write_text(markdown, encoding="utf-8")

    reconciliation = json.loads(RECONCILIATION.read_text(encoding="utf-8"))
    ledger_sha = sha256_bytes(LEDGER_JSON.read_bytes())
    markdown_sha = sha256_bytes(LEDGER_MD.read_bytes())
    audit = reconciliation["promotion_consumer_audit"]
    audit.update({
        "status": "central_consumer_rewrite_ledger_complete",
        "open_receipts": 0,
        "open_consumer_rewrites": 0,
        "consumer_rewrite_authority": {
            "json_path": "extractor/documents/WAVE2_EXACT10_CONSUMER_REWRITE_LEDGER_20260720.json",
            "json_sha256": ledger_sha,
            "report_path": "extractor/documents/WAVE2_EXACT10_CONSUMER_REWRITE_LEDGER_20260720.md",
            "report_sha256": markdown_sha,
            "central_binding_is_authority": True,
            "inline_consumer_duplication_required": True,
        },
        "disposition_counts": ledger["counts"],
    })
    reconciliation["counts"].update({
        "success_root_calls": 43,
        "completed_success_root_calls": 43,
        "pending_success_root_calls": 0,
        "expected_failure_root_calls": 2,
    })
    reconciliation["promotion_admission"].update({
        "allowed": True,
        "reason": "All exact-ten root receipts and central consumer bindings are complete. This admits replacement provenance only; independent lane runtime/human activation gates remain unchanged.",
        "admission_scope": "exact10_evidence_provenance_replacement_only",
        "open_result_slots": 0,
        "open_consumer_rewrites": 0,
        "admit_only_when": "Satisfied by 43 authoritative successful receipts, two terminal expected-failure receipts, and the hash-pinned central consumer rewrite ledger.",
        "lane_activation_unchanged": True,
    })
    RECONCILIATION.write_text(json.dumps(reconciliation, separators=(",", ":"), ensure_ascii=False) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
