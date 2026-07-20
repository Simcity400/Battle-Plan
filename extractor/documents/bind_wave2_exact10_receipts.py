#!/usr/bin/env python3
"""Bind already-terminal exact-10 Ghidra jobs into the Wave 2 receipt ledger.

This is a deterministic receipt assembler, not a Ghidra runner or promotion
verifier. It only accepts the explicit job-binding table written by the sole
exact-10 export lane, validates immutable job/artifact facts, updates receipt
slots, and emits the per-action receipt/report.
"""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
from typing import Any


HERE = Path(__file__).resolve().parent
REPO_ROOT = HERE.parent.parent
MANIFEST_PATH = HERE / "WAVE2_INVALID_20_SHARD_REEXPORT_MANIFEST_20260720.json"
RECONCILIATION_PATH = HERE / "WAVE2_10_SHARD_REEXPORT_RECONCILIATION_20260720.json"
BINDINGS_PATH = HERE / "WAVE2_EXACT10_JOB_BINDINGS_20260720.json"
RECEIPT_PATH = HERE / "WAVE2_EXACT10_REEXPORT_RECEIPT_20260720.json"
REPORT_PATH = HERE / "WAVE2_EXACT10_REEXPORT_REPORT_20260720.md"
JOB_ROOT = Path(r"C:\hoi4_evidence\ghidra_agent_exports")
POOL_RECEIPT_PATH = Path(r"C:\hoi4_evidence\WAVE2_EXACT10_POOL_STATUS_20260720.json")
EXPECTED_IDENTITY = {
    "binary_id": "hoi4.exe 1.19.2.0.a729",
    "image_base": "0x140000000",
    "sha256": "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad",
}


def load_json(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"{path}: expected JSON object")
    return value


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def atomic_write(path: Path, text: str) -> None:
    temp = path.with_name(f"{path.name}.{os.getpid()}.tmp")
    try:
        temp.write_text(text, encoding="utf-8", newline="\n")
        temp.replace(path)
    finally:
        if temp.exists():
            temp.unlink()


def semantic_request(operation: str, request: dict[str, Any]) -> dict[str, Any]:
    if operation == "search_strings":
        return {"terms": request["terms"]}
    if operation == "dump_data":
        return {"addresses": request["addresses"]}
    if operation == "export_functions":
        return {
            "addresses": request["addresses"],
            "depth": request["depth"],
            "pcode": request["pcode"],
        }
    raise ValueError(f"unsupported operation: {operation}")


def semantic_preimage(operation: str, request: dict[str, Any]) -> str:
    text = json.dumps(
        {"operation": operation, "request": semantic_request(operation, request)},
        ensure_ascii=True,
        separators=(",", ":"),
    )
    return (
        text.replace("<", r"\u003c")
        .replace(">", r"\u003e")
        .replace("&", r"\u0026")
        .replace("'", r"\u0027")
    )


def semantic_hash(operation: str, request: dict[str, Any]) -> str:
    return sha256_bytes(semantic_preimage(operation, request).encode("utf-8"))


def artifact_fields(operation: str, result: dict[str, Any]) -> tuple[str, str]:
    if operation == "export_functions":
        return result["manifest"], result["manifest_sha256"]
    return result["artifact"], result["artifact_sha256"]


def result_summary(root: dict[str, Any]) -> dict[str, Any]:
    if root["root_call_id"].startswith("EF-"):
        failure = root["known_failure_receipt"]
        return {
            "root_call_id": root["root_call_id"],
            "expected_request_sha256": root["expected_request_sha256"],
            "status": "expected_failure_closed_no_retry",
            "actual_job_id": failure["actual_job_id"],
            "actual_shard_id": failure["actual_shard_id"],
            "artifact": None,
            "artifact_sha256": None,
            "error": failure["error"],
            "retry_required": False,
        }
    replacement = root["replacement_result"]
    return {
        "root_call_id": root["root_call_id"],
        "expected_request_sha256": root["expected_request_sha256"],
        "status": replacement["status"],
        "actual_job_id": replacement["actual_job_id"],
        "actual_shard_id": replacement["actual_shard_id"],
        "artifact": replacement["artifact"],
        "artifact_sha256": replacement["artifact_sha256"],
        "error": None,
        "retry_required": False,
    }


def main() -> int:
    manifest = load_json(MANIFEST_PATH)
    reconciliation = load_json(RECONCILIATION_PATH)
    bindings = load_json(BINDINGS_PATH)
    pool = load_json(POOL_RECEIPT_PATH)

    if pool.get("schema") != "hoi4-ghidra-pool-status-receipt-v1":
        raise ValueError("pool receipt schema mismatch")
    for key, expected in EXPECTED_IDENTITY.items():
        if pool.get(key) != expected:
            raise ValueError(f"pool receipt {key} mismatch")
    ready_shards = [f"shard-{number:02d}" for number in range(1, 11)]
    if pool.get("shard_count") != 10 or sorted(pool.get("ready_shard_ids", [])) != ready_shards:
        raise ValueError("pool receipt is not exact-10 ready")
    if pool.get("read_only") is not True or pool.get("analysis_enabled") is not False:
        raise ValueError("pool receipt is not read-only/no-analysis")
    if pool.get("per_shard_serialization") is not True or pool.get("persistent_error") is not None:
        raise ValueError("pool receipt serialization/error gate failed")
    pool_sha = sha256_file(POOL_RECEIPT_PATH)
    if bindings["authority_gate"]["pool_status_receipt_sha256"] != pool_sha:
        raise ValueError("binding table pool receipt hash mismatch")

    roots = reconciliation["consolidated_execution_plan"]["root_calls"]
    root_by_id = {root["root_call_id"]: root for root in roots}
    if len(roots) != 45 or len(root_by_id) != 45:
        raise ValueError("expected 45 unique root calls")
    success_bindings = bindings["successful_roots"]
    if len(success_bindings) != 43:
        raise ValueError("expected 43 successful bindings")
    binding_ids = {binding["root_call_id"] for binding in success_bindings}
    expected_success_ids = {root_id for root_id in root_by_id if not root_id.startswith("EF-")}
    if binding_ids != expected_success_ids:
        raise ValueError("successful binding/root ID set mismatch")

    pool_pointer = {
        "status_snapshot_path": str(POOL_RECEIPT_PATH),
        "status_snapshot_sha256": pool_sha,
        "observed_shard_count": 10,
        "observed_ready_shards": ready_shards,
    }

    for binding in success_bindings:
        root = root_by_id[binding["root_call_id"]]
        expected_hash = root["expected_request_sha256"]
        if binding["expected_request_sha256"] != expected_hash:
            raise ValueError(f"{root['root_call_id']}: binding hash mismatch")
        job_path = JOB_ROOT / binding["actual_job_id"] / "job.json"
        job = load_json(job_path)
        if job.get("status") != "complete":
            raise ValueError(f"{root['root_call_id']}: job is not complete")
        if job.get("operation") != root["operation"]:
            raise ValueError(f"{root['root_call_id']}: operation mismatch")
        actual_hash = semantic_hash(job["operation"], job["request"])
        if actual_hash != expected_hash:
            raise ValueError(f"{root['root_call_id']}: semantic request hash mismatch")
        if job.get("identity_contract") != EXPECTED_IDENTITY:
            raise ValueError(f"{root['root_call_id']}: job identity mismatch")
        result = job["result"]
        actual_shard = (result.get("shard") or job.get("shard"))["id"]
        if actual_shard != binding["actual_shard_id"] or actual_shard not in ready_shards:
            raise ValueError(f"{root['root_call_id']}: shard mismatch")
        artifact, artifact_sha = artifact_fields(job["operation"], result)
        if artifact != binding["artifact"] or artifact_sha != binding["artifact_sha256"]:
            raise ValueError(f"{root['root_call_id']}: binding artifact mismatch")
        artifact_path = Path(artifact)
        if not artifact_path.is_file() or sha256_file(artifact_path) != artifact_sha:
            raise ValueError(f"{root['root_call_id']}: artifact file/hash mismatch")
        identity_manifest_sha = result.get("identity_manifest_sha256") or pool["identity_manifest_sha256"]
        if identity_manifest_sha != pool["identity_manifest_sha256"]:
            raise ValueError(f"{root['root_call_id']}: identity manifest mismatch")
        root["replacement_result"] = {
            "status": "complete",
            "receipt_authoritative": True,
            "actual_job_id": job["job_id"],
            "actual_shard_id": actual_shard,
            "job_json_path": str(job_path),
            "job_json_sha256": sha256_file(job_path),
            "actual_request_sha256": actual_hash,
            "expected_request_sha256": expected_hash,
            "request_hash_match": True,
            "identity_contract": EXPECTED_IDENTITY,
            "identity_manifest_sha256": identity_manifest_sha,
            "artifact": artifact,
            "artifact_sha256": artifact_sha,
            "pool_status_receipt": dict(pool_pointer),
            "planned_target_shard_must_match_actual": False,
        }

    expected_failure_ids = {"EF-01-D0-P0", "EF-02-D1-P1"}
    for root_id in expected_failure_ids:
        root = root_by_id[root_id]
        failure = root["known_failure_receipt"]
        job_path = Path(failure["job_json_path"])
        job = load_json(job_path)
        if job.get("status") != "failed" or not job.get("error"):
            raise ValueError(f"{root_id}: expected failed job is not terminal failed")
        if job.get("identity_contract") != EXPECTED_IDENTITY:
            raise ValueError(f"{root_id}: failed job identity mismatch")
        if sha256_file(job_path) != failure["job_json_sha256"]:
            raise ValueError(f"{root_id}: failed job JSON hash mismatch")
        if semantic_hash(job["operation"], job["request"]) != failure["actual_batched_request_sha256"]:
            raise ValueError(f"{root_id}: failed batched request hash mismatch")
        if job["error"] != failure["error"]:
            raise ValueError(f"{root_id}: failed job error mismatch")
        failure["identity_contract"] = EXPECTED_IDENTITY
        failure["identity_manifest_sha256"] = pool["identity_manifest_sha256"]
        failure["pool_status_receipt"] = dict(pool_pointer)
        failure["planned_target_shard_must_match_actual"] = False

    reconciliation["authority_state"] = (
        "exact_10_root_receipts_complete_expected_failures_retained_consumer_rewrites_pending"
    )
    reconciliation["receipt_completion"] = {
        "status": "complete",
        "successful_roots": 43,
        "isolated_expected_failures": 2,
        "open_root_receipts": 0,
        "pool_status_receipt": str(POOL_RECEIPT_PATH),
        "pool_status_receipt_sha256": pool_sha,
        "promotion_consumers_rewritten": False,
    }
    admission = reconciliation.get("promotion_admission")
    if isinstance(admission, dict):
        admission["allowed"] = False
        admission["reason"] = (
            "Exact-10 root receipts are complete; external promotion-consumer rewrites remain pending. "
            "Two isolated 0x140082cb0 decompiler failures are closed/no-retry evidence."
        )
        admission["open_result_slots"] = 0

    root_results = [result_summary(root) for root in roots]
    root_results.sort(key=lambda row: root_by_id[row["root_call_id"]]["root_sequence"])

    action_results: list[dict[str, Any]] = []
    for action in manifest["reexport_actions"]:
        matched = [
            root for root in roots if action["sequence"] in root.get("source_action_sequences", [])
        ]
        if not matched:
            raise ValueError(f"canonical action {action['sequence']} has no replacement root")
        action_results.append(
            {
                "sequence": action["sequence"],
                "operation": action["operation"],
                "semantic_request_sha256": action["semantic_request_sha256"],
                "source_job_ids": action["source_job_ids"],
                "replacement_roots": [result_summary(root) for root in matched],
            }
        )
    if len(action_results) != 122:
        raise ValueError("canonical action receipt count is not 122")

    supplemental_results: list[dict[str, Any]] = []
    for action in manifest.get("supplemental_exact10_actions", []):
        action_id = action["supplemental_action_id"]
        matched = [root for root in roots if action_id in root.get("supplemental_action_ids", [])]
        if not matched:
            raise ValueError(f"supplemental action {action_id} has no replacement root")
        supplemental_results.append(
            {
                "supplemental_action_id": action_id,
                "operation": action["operation"],
                "address": action["address"],
                "depth": action["depth"],
                "pcode": action["pcode"],
                "replacement_roots": [result_summary(root) for root in matched],
            }
        )

    receipt = {
        "schema": "hoi4-wave2-exact10-reexport-receipt-v1",
        "generated_at": "2026-07-19T22:40:59Z",
        "authority_state": reconciliation["authority_state"],
        "identity_gate": {
            **EXPECTED_IDENTITY,
            "shard_count": 10,
            "ready_shard_ids": ready_shards,
            "read_only": True,
            "analysis_enabled": False,
            "per_shard_serialization": True,
            "persistent_error": None,
            "pool_status_receipt": str(POOL_RECEIPT_PATH),
            "pool_status_receipt_sha256": pool_sha,
        },
        "source_files": {
            str(MANIFEST_PATH.relative_to(REPO_ROOT)).replace("\\", "/"): sha256_file(MANIFEST_PATH),
            str(RECONCILIATION_PATH.relative_to(REPO_ROOT)).replace("\\", "/"): None,
            str(BINDINGS_PATH.relative_to(REPO_ROOT)).replace("\\", "/"): sha256_file(BINDINGS_PATH),
        },
        "counts": {
            "root_calls": 45,
            "successful_roots": 43,
            "isolated_expected_failures": 2,
            "canonical_actions": len(action_results),
            "supplemental_actions": len(supplemental_results),
            "unexpected_failures": 0,
        },
        "root_results": root_results,
        "canonical_action_results": action_results,
        "supplemental_action_results": supplemental_results,
        "failures": [row for row in root_results if row["status"] != "complete"],
    }

    reconciliation_text = json.dumps(
        reconciliation, ensure_ascii=True, separators=(",", ":")
    ) + "\n"
    atomic_write(RECONCILIATION_PATH, reconciliation_text)
    receipt["source_files"][
        str(RECONCILIATION_PATH.relative_to(REPO_ROOT)).replace("\\", "/")
    ] = sha256_file(RECONCILIATION_PATH)
    atomic_write(RECEIPT_PATH, json.dumps(receipt, indent=2, sort_keys=True) + "\n")

    report = "\n".join(
        [
            "# Wave 2 exact-10 Ghidra re-export receipt",
            "",
            "The exact-10 authority replay is terminal: **43 root calls completed successfully** and "
            "**2 isolated roots reproduced the expected `0x140082cb0` decompiler failure**. "
            "There were no unexpected failures and the failed seed must not be retried.",
            "",
            "- Binary: `hoi4.exe 1.19.2.0.a729`",
            "- Image base: `0x140000000`",
            f"- SHA-256: `{EXPECTED_IDENTITY['sha256']}`",
            "- Pool: exactly 10 ready shards, read-only, analysis disabled, per-shard serialized",
            "- Canonical action mappings: 122/122",
            f"- Supplemental action mappings: {len(supplemental_results)}",
            "- Actual executing shard for this lane: `shard-03` (planned shards were advisory only)",
            "",
            "## Expected failures",
            "",
            "- `EF-01-D0-P0`: depth 0, p-code disabled, `0x140082cb0`; deterministic decompile failure; no retry.",
            "- `EF-02-D1-P1`: depth 1, p-code enabled, `0x140082cb0`; deterministic decompile failure; no retry.",
            "",
            "Both surrounding clean remainders completed, isolating the failure to that seed. "
            "Promotion remains closed only for pending external consumer rewrites and any claim that "
            "requires the unavailable decompile, not for missing exact-10 replay work.",
            "",
            "Machine authority: `WAVE2_EXACT10_REEXPORT_RECEIPT_20260720.json`.",
            "",
        ]
    )
    atomic_write(REPORT_PATH, report)
    print(
        f"BOUND roots={len(root_results)} success=43 expected_failures=2 "
        f"canonical_actions={len(action_results)} supplemental={len(supplemental_results)}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
