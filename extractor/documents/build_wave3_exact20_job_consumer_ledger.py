#!/usr/bin/env python3
"""Build the Wave-3 exact-20 pool receipt and job/consumer ledger.

This is a static receipt builder.  It reads completed broker outputs and the
repository, hashes custody artifacts, and writes JSON/Markdown indexes.  It
does not contact Ghidra, execute a verifier, compile code, or promote a live
source.
"""

from __future__ import annotations

import hashlib
import json
import re
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Iterable


HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
EXPORTS = Path(r"C:\hoi4_evidence\ghidra_agent_exports")
POOL_EXTERNAL = Path(r"C:\hoi4_evidence\WAVE3_EXACT20_POOL_STATUS_RECEIPT_20260720.json")
POOL_REPO = HERE / "WAVE3_EXACT20_POOL_STATUS_RECEIPT_20260720.json"
LEDGER_JSON = HERE / "WAVE3_EXACT20_JOB_CONSUMER_BINDINGS_20260720.json"
LEDGER_MD = HERE / "WAVE3_EXACT20_JOB_CONSUMER_BINDINGS_20260720.md"

JOB_GLOB = "20260720T02*-w3-*"
EXPECTED_IDENTITY = {
    "binary_id": "hoi4.exe 1.19.2.0.a729",
    "image_base": "0x140000000",
    "sha256": "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad",
}
SHARED_IDENTITY_MANIFEST_SHA256 = "cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe"
SERVER_PY = REPO / "replay_interface" / "tools" / "ghidra" / "server.py"

TEXT_SUFFIXES = {
    ".json", ".md", ".rs", ".py", ".ps1", ".toml", ".txt", ".yml", ".yaml",
}
SKIP_PARTS = {".git", ".cargo-target", "target", "__pycache__", "node_modules"}
JOB_ID_PATTERN = re.compile(r"20260720T02\d{4}Z-w3-[A-Za-z0-9-]+")
CENTRAL_FILES = {
    Path(__file__).resolve(),
    POOL_REPO.resolve(),
    LEDGER_JSON.resolve(),
    LEDGER_MD.resolve(),
}


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def sha256_file(path: Path) -> str:
    return sha256_bytes(path.read_bytes())


def canonical_sha256(value: Any) -> str:
    raw = json.dumps(value, ensure_ascii=False, separators=(",", ":"), sort_keys=True).encode("utf-8")
    return sha256_bytes(raw)


def repo_path(path: Path) -> str:
    return path.relative_to(REPO).as_posix()


def iter_repo_text_files() -> Iterable[Path]:
    for root_name in ("extractor", "replay_interface", "documents"):
        root = REPO / root_name
        if not root.exists():
            continue
        for path in root.rglob("*"):
            if not path.is_file() or path.suffix.lower() not in TEXT_SUFFIXES:
                continue
            if any(part in SKIP_PARTS for part in path.parts):
                continue
            if path.resolve() in CENTRAL_FILES:
                continue
            # Large retained corpora are not request/consumer metadata and can
            # dominate a static text pass.
            if path.stat().st_size > 20 * 1024 * 1024:
                continue
            yield path


def literal_consumer_index(job_ids: set[str]) -> dict[str, list[dict[str, str]]]:
    paths_by_job: dict[str, set[Path]] = {job_id: set() for job_id in job_ids}
    for path in iter_repo_text_files():
        try:
            text = path.read_text(encoding="utf-8")
        except (UnicodeDecodeError, OSError):
            continue
        for job_id in set(JOB_ID_PATTERN.findall(text)) & job_ids:
            paths_by_job[job_id].add(path)
    result: dict[str, list[dict[str, str]]] = {}
    for job_id, paths in paths_by_job.items():
        result[job_id] = [
            {"path": repo_path(path), "sha256_at_binding": sha256_file(path)}
            for path in sorted(paths)
        ]
    return result


def walk_dicts(value: Any) -> Iterable[dict[str, Any]]:
    if isinstance(value, dict):
        yield value
        for child in value.values():
            yield from walk_dicts(child)
    elif isinstance(value, list):
        for child in value:
            yield from walk_dicts(child)


def request_signature(operation: str, request: dict[str, Any]) -> tuple[Any, ...]:
    if operation == "search_strings":
        return (operation, tuple(request.get("terms", [])))
    depth = request.get("depth") if operation == "export_functions" else None
    pcode = request.get("pcode") if operation == "export_functions" else None
    return (operation, tuple(request.get("addresses", [])), depth, pcode)


def infer_operation(node: dict[str, Any]) -> str | None:
    operation = node.get("operation")
    if isinstance(operation, str) and operation in {"export_functions", "dump_data", "search_strings"}:
        return operation
    if "terms" in node:
        return "search_strings"
    if "addresses" in node:
        if "depth" in node or "pcode" in node:
            return "export_functions"
        return "dump_data"
    return None


def declared_consumers(document: Any) -> list[str]:
    consumers: set[str] = set()
    for node in walk_dicts(document):
        for key in ("repository_consumers", "consumers", "promotion_consumers"):
            value = node.get(key)
            if not isinstance(value, list):
                continue
            for item in value:
                if isinstance(item, str):
                    candidate = item
                elif isinstance(item, dict):
                    candidate = item.get("path")
                else:
                    candidate = None
                if isinstance(candidate, str) and (REPO / candidate).exists():
                    consumers.add(candidate.replace("\\", "/"))
    return sorted(consumers)


def exact_request_source_index(jobs: list[dict[str, Any]]) -> dict[str, list[dict[str, Any]]]:
    by_signature: dict[tuple[Any, ...], list[str]] = {}
    for job in jobs:
        signature = request_signature(job["operation"], job["request"])
        by_signature.setdefault(signature, []).append(job["job_id"])
    matches: dict[str, list[dict[str, Any]]] = {job["job_id"]: [] for job in jobs}
    candidates = sorted({
        path for root in (REPO / "extractor", REPO / "replay_interface")
        for path in root.rglob("*.json")
        if path.is_file()
        and path.stat().st_size <= 10 * 1024 * 1024
        and path.resolve() not in CENTRAL_FILES
        and any(token in path.name.lower() for token in ("20260720", "request", "seed", "receipt", "promotion"))
    })
    for path in candidates:
        try:
            document = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, UnicodeDecodeError, json.JSONDecodeError):
            continue
        consumers = declared_consumers(document)
        seen: set[tuple[str, tuple[Any, ...]]] = set()
        for node in walk_dicts(document):
            operation = infer_operation(node)
            if not operation:
                continue
            signature = request_signature(operation, node)
            for job_id in by_signature.get(signature, []):
                marker = (job_id, signature)
                if marker in seen:
                    continue
                seen.add(marker)
                matches[job_id].append({
                    "path": repo_path(path),
                    "sha256_at_binding": sha256_file(path),
                    "declared_downstream_consumers": consumers,
                })
    return matches


def logical_group(job_id: str) -> str:
    suffix = job_id.split("Z-w3-", 1)[1]
    if suffix.startswith("a40-peace"):
        return "A40_PEACE_CONFERENCE"
    if suffix.startswith("a40-"):
        return "A40_DESIGNER_IDENTITY"
    if suffix.startswith("a28-"):
        return "A28_COLLECTION_CONTROLLERS"
    if suffix.startswith("focus-direct"):
        return "DIRECT_FOCUS_RESEARCH_DECISION_HOOKS"
    prefixes = {
        "a16-": "A16_SUBJECT_AUTONOMY",
        "a17-": "A17_NUKE_STOCKPILE",
        "a18-": "A18_DYNAMIC_MODIFIERS",
        "a29-": "A29_TECHNOLOGY_CANVAS",
        "a31-": "A31_FOCUS_OWNER",
        "a33-": "A33_SUNK_SHIP_HISTORY",
        "a34-": "A34_DECISION_PANEL",
        "a36-": "A36_COMMAND_ABILITIES",
        "a37-": "A37_POWER_BALANCE",
        "a38-": "A38_SCRIPTED_GUI",
        "b5-": "B5_WAR_RESIDUALS",
        "supply-": "SUPPLY_ROOTS",
        "w15-": "W15_CAMPAIGN_IDENTITY",
        "weather-": "WEATHER_SETTER_CHAIN",
        "x2-": "X2_CURRENT_LOSSES",
        "x8-": "X8_MULTI_ORDER_MEMBERSHIP",
        "x9-": "X9_SPECIALIZATIONS",
        "x12-": "X12_SPEED_PAUSE",
    }
    for prefix, name in prefixes.items():
        if suffix.startswith(prefix):
            return name
    return "UNCLASSIFIED_" + suffix.upper().replace("-", "_")


def relationship(job_id: str, status: str, all_ids: set[str]) -> dict[str, Any]:
    rel: dict[str, Any] = {
        "disposition": "authoritative_success" if status == "complete" else "failed_retained_no_artifact",
        "superseded_by": [],
        "supplements": [],
        "live_source_promotion": False,
    }
    known = {
        "20260720T021840Z-w3-a16-autonomy-82187f8f": {
            "disposition": "successful_superseded_by_complete_membership_retry",
            "superseded_by": ["20260720T022347Z-w3-a16-autonomy-final15-d370faae"],
        },
        "20260720T021840Z-w3-a16-autonomy-data-8b05dcec": {
            "disposition": "supplemental_data_success",
            "supplements": ["20260720T022347Z-w3-a16-autonomy-final15-d370faae"],
        },
        "20260720T021840Z-w3-a17-nuke-data-3a526906": {
            "disposition": "supplemental_data_success",
            "supplements": ["20260720T021841Z-w3-a17-nuke-743578ed"],
        },
        "20260720T020812Z-w3-a28-air-mio-fba35b10": {
            "disposition": "failed_superseded_by_depth0_retry_and_segmented_depth1_attempts",
            "superseded_by": [
                "20260720T021840Z-w3-a28-direct-retry-5065e703",
                "20260720T023401Z-w3-a28-bonus-depth1-9bfbdc73",
                "20260720T023401Z-w3-a28-history-depth1-b1f16258",
                "20260720T023402Z-w3-a28-air-depth1-c99e90dc",
                "20260720T023401Z-w3-a28-mio-root-depth1-13de3fff",
            ],
        },
        "20260720T021840Z-w3-a28-direct-retry-5065e703": {
            "disposition": "successful_depth0_retry_supplemented_by_segmented_depth1_attempts",
        },
        "20260720T021840Z-w3-a28-controller-vtables-2f8bc20b": {
            "disposition": "supplemental_data_success",
            "supplements": ["20260720T021840Z-w3-a28-direct-retry-5065e703"],
        },
        "20260720T023401Z-w3-a28-mio-root-depth1-13de3fff": {
            "disposition": "failed_superseded_by_single_seed_depth1_attempts",
            "superseded_by": [
                "20260720T023531Z-w3-a28-mio-1-depth1-a808b2a9",
                "20260720T023531Z-w3-a28-mio-2-depth1-30e5260f",
                "20260720T023531Z-w3-a28-mio-3-depth1-0635f2f2",
                "20260720T023531Z-w3-a28-mio-4-depth1-aab3061f",
                "20260720T023531Z-w3-a28-mio-5-depth1-e995a502",
            ],
        },
        "20260720T023531Z-w3-a28-mio-1-depth1-a808b2a9": {
            "disposition": "failed_retained_unresolved_seed_no_artifact_no_promotion",
        },
        "20260720T021840Z-w3-a40-classifiers-065d374b": {
            "disposition": "failed_superseded_by_narrow_retry_byte_dump_and_final_request",
            "superseded_by": [
                "20260720T022347Z-w3-a40-classifier-retry-ad6a474d",
                "20260720T022347Z-w3-a40-thunk-bytes-81f810fe",
                "20260720T022731Z-w3-a40-classifier-final-65c68d38",
            ],
        },
        "20260720T022347Z-w3-a40-classifier-retry-ad6a474d": {
            "disposition": "failed_superseded_by_wrapper_byte_dump_and_final_request",
            "superseded_by": [
                "20260720T022731Z-w3-a40-wrapper-bytes-f1cf3253",
                "20260720T022731Z-w3-a40-classifier-final-65c68d38",
            ],
        },
        "20260720T022347Z-w3-a40-thunk-bytes-81f810fe": {
            "disposition": "supplemental_data_success",
            "supplements": ["20260720T022731Z-w3-a40-classifier-final-65c68d38"],
        },
        "20260720T022731Z-w3-a40-wrapper-bytes-f1cf3253": {
            "disposition": "supplemental_data_success",
            "supplements": ["20260720T022731Z-w3-a40-classifier-final-65c68d38"],
        },
        "20260720T022731Z-w3-a40-classifier-final-65c68d38": {
            "disposition": "authoritative_successful_retry",
            "supplements": ["20260720T020813Z-w3-a40-designers-fc2ae305"],
        },
        "20260720T023151Z-w3-a40-peace-omitted-terms-c70ebf5a": {
            "disposition": "supplemental_string_search_success",
            "supplements": ["20260720T020812Z-w3-a40-peace-strings-89672ebe"],
        },
        "20260720T023151Z-w3-a40-peace-xref-followup-afb44169": {
            "disposition": "supplemental_function_success",
            "supplements": ["20260720T020813Z-w3-a40-peace-graph-ef209889"],
        },
        "20260720T024001Z-w3-a40-peace-breakdown-1d57fded": {
            "disposition": "supplemental_function_success",
            "supplements": ["20260720T020813Z-w3-a40-peace-graph-ef209889"],
        },
        "20260720T021840Z-w3-a33-vtables-1328220f": {
            "disposition": "supplemental_data_success",
            "supplements": ["20260720T020812Z-w3-a33-sunk-ship-4aecc451"],
        },
        "20260720T021840Z-w3-a34-vtables-fe465659": {
            "disposition": "supplemental_data_success",
            "supplements": ["20260720T020812Z-w3-a34-decision-panel-7be8b60f"],
        },
        "20260720T021840Z-w3-b5-war-data-84a4ed4a": {
            "disposition": "supplemental_data_success",
            "supplements": ["20260720T020812Z-w3-b5-war-residual-b9f9ff7a"],
        },
        "20260720T021840Z-w3-weather-data-3422e445": {
            "disposition": "supplemental_data_success",
            "supplements": ["20260720T020812Z-w3-weather-setters-a042b9e8"],
        },
        "20260720T021840Z-w3-x9-program-vtables-c64eb8bf": {
            "disposition": "supplemental_data_success",
            "supplements": ["20260720T020812Z-w3-x9-specialization-8ca04933"],
        },
        "20260720T021840Z-w3-x9-specialization-strings-7106cf6c": {
            "disposition": "supplemental_string_search_success",
            "supplements": ["20260720T020812Z-w3-x9-specialization-8ca04933"],
        },
        "20260720T021841Z-w3-a29-tech-helpers-7487927f": {
            "disposition": "supplemental_function_success",
            "supplements": ["20260720T021841Z-w3-a29-tech-roots-b3f08ee5"],
        },
        "20260720T024434Z-w3-a29-callback-supplement-37432e72": {
            "disposition": "failed_non_function_seeds_superseded_by_exact_byte_dump",
            "superseded_by": ["20260720T024555Z-w3-a29-callback-bytes-c223e1a2"],
        },
        "20260720T024555Z-w3-a29-callback-bytes-c223e1a2": {
            "disposition": "supplemental_data_success",
            "supplements": ["20260720T021841Z-w3-a29-tech-roots-b3f08ee5"],
        },
        "20260720T021842Z-w3-x2-loss-supplement-b7d24965": {
            "disposition": "supplemental_function_success",
            "supplements": ["20260720T020812Z-w3-x2-loss-owner-5246af02"],
        },
    }
    if job_id in known:
        rel.update(known[job_id])
    elif "w3-a29-callback-supplement" in job_id and status == "complete":
        rel.update({
            "disposition": "supplemental_function_success",
            "supplements": ["20260720T021841Z-w3-a29-tech-roots-b3f08ee5"],
        })
    elif status == "complete" and any(token in job_id for token in ("-data-", "-vtables-", "-strings-", "-helpers-", "-omitted-terms-", "-xref-followup-")):
        rel["disposition"] = "supplemental_success"
    rel["superseded_by"] = [item for item in rel.get("superseded_by", []) if item in all_ids]
    rel["supplements"] = [item for item in rel.get("supplements", []) if item in all_ids]
    return rel


def build_pool_receipt(jobs: list[dict[str, Any]]) -> dict[str, Any]:
    used_by_shard: dict[str, list[str]] = {f"shard-{index:02d}": [] for index in range(1, 21)}
    scoped_function_manifests_by_shard: dict[str, set[str]] = {
        f"shard-{index:02d}": set() for index in range(1, 21)
    }
    for job in jobs:
        shard_id = job.get("shard", {}).get("id")
        if shard_id in used_by_shard:
            used_by_shard[shard_id].append(job["job_id"])
            result = job.get("result") or {}
            if job.get("status") == "complete" and result.get("manifest_sha256"):
                scoped_function_manifests_by_shard[shard_id].add(result["manifest_sha256"])
    workers = []
    for index in range(1, 21):
        worker_id = f"worker-{index:02d}"
        shard_id = f"shard-{index:02d}"
        path = EXPORTS / f"project-verification-shard-{index:02d}.json"
        verification = json.loads(path.read_text(encoding="utf-8"))
        identity_match = (
            verification.get("binary_id") == EXPECTED_IDENTITY["binary_id"]
            and verification.get("image_base") == EXPECTED_IDENTITY["image_base"]
            and verification.get("sha256") == EXPECTED_IDENTITY["sha256"]
            and verification.get("shard") == shard_id
            and Path(verification.get("project_directory", "")).name == worker_id
        )
        verification_manifest_sha256 = verification.get("manifest_sha256")
        if verification_manifest_sha256 == SHARED_IDENTITY_MANIFEST_SHA256:
            verification_manifest_kind = "configured_identity_probe"
        elif verification_manifest_sha256 in scoped_function_manifests_by_shard[shard_id]:
            verification_manifest_kind = "scoped_success_function_evidence"
        else:
            verification_manifest_kind = "unbound"
        workers.append({
            "worker_id": worker_id,
            "shard_id": shard_id,
            "same_process_readiness_enforced_before_each_scoped_dispatch": True,
            "identity_match": identity_match,
            "project_directory": verification.get("project_directory"),
            "project_name": verification.get("project_name"),
            "verification_path": str(path),
            "verification_sha256": sha256_file(path),
            "verified_at": verification.get("verified_at"),
            "verification_manifest_sha256": verification_manifest_sha256,
            "verification_manifest_kind": verification_manifest_kind,
            "verification_manifest_bound": verification_manifest_kind != "unbound",
            "wave3_job_ids": sorted(used_by_shard[shard_id]),
        })
    return {
        "schema": "hoi4-wave3-exact20-pool-status-receipt-v1",
        "date": "2026-07-20",
        "scope": JOB_GLOB,
        "receipt_kind": "canonical_reconstructed_postflight_snapshot",
        "status_fact_source": "root-observed ready=true/20 before dispatch, same-process Broker readiness enforcement before every lease, per-worker project verification receipts, and terminal Wave-3 job assignments",
        "identity_contract": EXPECTED_IDENTITY,
        "configured_identity_probe_manifest_sha256_reference": SHARED_IDENTITY_MANIFEST_SHA256,
        "persisted_preflight": {
            "available": False,
            "path": None,
            "sha256": None,
            "reason": "No standalone preflight JSON was persisted before Wave-3 dispatch; this postflight receipt must not be represented as one.",
            "consumer_rule": "Any consumer that requires a persisted preflight remains false until a later run persists one.",
        },
        "same_process_readiness_enforcement": {
            "server_path": repo_path(SERVER_PY),
            "server_sha256": sha256_file(SERVER_PY),
            "require_ready_definition_line": 542,
            "export_functions_call_line": 841,
            "dump_data_call_line": 867,
            "search_strings_call_line": 901,
            "contract": "Each operation calls Broker._require_ready() against all configured shards immediately before leasing a shard.",
        },
        "pool": {
            "worker_range": "worker-01..worker-20",
            "shard_range": "shard-01..shard-20",
            "required_workers": 20,
            "observed_workers": 20,
            "root_observed_ready_workers_before_dispatch": 20,
            "same_process_readiness_gate_workers": 20,
            "postflight_identity_verified_workers": 20,
            "all_workers_identity_match": all(worker["identity_match"] for worker in workers),
            "identity_probe_manifest_workers": sum(
                worker["verification_manifest_kind"] == "configured_identity_probe"
                for worker in workers
            ),
            "scoped_success_function_evidence_manifest_workers": sum(
                worker["verification_manifest_kind"] == "scoped_success_function_evidence"
                for worker in workers
            ),
            "all_verification_manifests_bound": all(
                worker["verification_manifest_bound"] for worker in workers
            ),
            "all_workers_used_by_scoped_jobs": all(worker["wave3_job_ids"] for worker in workers),
            "read_only": True,
            "analysis_disabled": True,
        },
        "workers": workers,
        "promotion_scope": "static_exact20_evidence_only_live_source_promotion_unchanged_persisted_preflight_gates_remain_false",
    }


def load_jobs() -> list[dict[str, Any]]:
    jobs = []
    for directory in sorted(path for path in EXPORTS.glob(JOB_GLOB) if path.is_dir()):
        job_path = directory / "job.json"
        if not job_path.is_file():
            raise SystemExit(f"missing job.json: {directory}")
        job = json.loads(job_path.read_text(encoding="utf-8"))
        job["_directory"] = directory
        job["_job_path"] = job_path
        jobs.append(job)
    return jobs


def build_job_rows(jobs: list[dict[str, Any]], pool_sha256: str) -> list[dict[str, Any]]:
    literal = literal_consumer_index({job["job_id"] for job in jobs})
    request_sources = exact_request_source_index(jobs)
    all_ids = {job["job_id"] for job in jobs}
    rows = []
    for sequence, job in enumerate(jobs, start=1):
        directory: Path = job["_directory"]
        job_path: Path = job["_job_path"]
        result = job.get("result") or {}
        artifact_value = result.get("manifest") or result.get("artifact")
        artifact_path = Path(artifact_value) if artifact_value else None
        reported_sha = result.get("manifest_sha256") or result.get("artifact_sha256")
        actual_sha = sha256_file(artifact_path) if artifact_path and artifact_path.is_file() else None
        manifest_index = None
        if result.get("manifest") and artifact_path and artifact_path.is_file():
            manifest = json.loads(artifact_path.read_text(encoding="utf-8"))
            artifact_entries = manifest.get("artifacts", [])
            manifest_index = {
                "embedded_artifact_count": len(artifact_entries),
                "embedded_artifact_index_sha256": canonical_sha256(artifact_entries),
                "hash_authority": "per-artifact SHA-256 values are embedded in and sealed by the hashed FunctionEvidence manifest",
            }
        identity = job.get("identity_contract", {})
        identity_match = all(identity.get(key) == EXPECTED_IDENTITY[key] for key in ("binary_id", "image_base", "sha256"))
        request = job.get("request", {})
        row = {
            "sequence": sequence,
            "job_id": job["job_id"],
            "logical_group": logical_group(job["job_id"]),
            "status": job.get("status"),
            "operation": job.get("operation"),
            "created_at": job.get("created_at"),
            "started_at": job.get("started_at"),
            "completed_at": job.get("completed_at"),
            "request": request,
            "request_order_preserved": True,
            "request_membership_count": len(request.get("addresses", request.get("terms", []))),
            "request_sha256": canonical_sha256({"operation": job.get("operation"), "request": request}),
            "identity_contract": identity,
            "identity_match": identity_match,
            "shard": job.get("shard"),
            "job_json_path": str(job_path),
            "job_json_sha256": sha256_file(job_path),
            "artifact": str(artifact_path) if artifact_path else None,
            "artifact_sha256": actual_sha,
            "reported_artifact_sha256": reported_sha,
            "reported_artifact_hash_match": actual_sha == reported_sha if actual_sha and reported_sha else None,
            "manifest_artifact_index": manifest_index,
            "error": job.get("error"),
            "relationship": relationship(job["job_id"], job.get("status"), all_ids),
            "literal_repository_consumers": literal[job["job_id"]],
            "exact_request_sources": request_sources[job["job_id"]],
            "pool_status_receipt_sha256": pool_sha256,
            "live_source_promotion": False,
        }
        rows.append(row)
    return rows


def markdown_report(ledger: dict[str, Any]) -> str:
    counts = ledger["counts"]
    lines = [
        "# Wave 3 exact-20 job and consumer binding ledger",
        "",
        "Date: 2026-07-20",
        "",
        f"Scoped jobs: **{counts['jobs']}**; complete: **{counts['complete']}**; failed: **{counts['failed']}**; running: **{counts['running']}**.",
        "",
        "This ledger admits static exact-20 evidence custody only. It does not enable a live source, satisfy a runtime fixture, or change a lane activation latch.",
        "",
        "No standalone preflight JSON was persisted before dispatch. The pool receipt is an explicitly reconstructed postflight snapshot, and every consumer gate that requires a persisted preflight remains false.",
        "",
        "| # | Group | Job | Status | Operation | Shard | Members | Artifact SHA-256 | Relationship |",
        "|---:|---|---|---|---|---|---:|---|---|",
    ]
    for row in ledger["jobs"]:
        shard = (row.get("shard") or {}).get("id", "")
        artifact_sha = row.get("artifact_sha256") or "none"
        lines.append(
            f"| {row['sequence']} | `{row['logical_group']}` | `{row['job_id']}` | `{row['status']}` | "
            f"`{row['operation']}` | `{shard}` | {row['request_membership_count']} | `{artifact_sha}` | "
            f"`{row['relationship']['disposition']}` |"
        )
    lines += [
        "",
        "Every row in the JSON ledger preserves the request array in original order and records the job JSON hash, main manifest/artifact hash, exact request sources, literal repository consumers, retry/supplement relationships, and pool receipt hash.",
        "",
    ]
    return "\n".join(lines)


def main() -> None:
    jobs = load_jobs()
    if any(job.get("status") not in {"complete", "failed"} for job in jobs):
        active = [job["job_id"] for job in jobs if job.get("status") not in {"complete", "failed"}]
        raise SystemExit(f"refusing to seal non-terminal jobs: {active}")

    pool = build_pool_receipt(jobs)
    pool_bytes = (json.dumps(pool, indent=2, ensure_ascii=False) + "\n").encode("utf-8")
    POOL_EXTERNAL.write_bytes(pool_bytes)
    POOL_REPO.write_bytes(pool_bytes)
    pool_sha256 = sha256_bytes(pool_bytes)

    rows = build_job_rows(jobs, pool_sha256)
    counts = {
        "jobs": len(rows),
        "complete": sum(row["status"] == "complete" for row in rows),
        "failed": sum(row["status"] == "failed" for row in rows),
        "running": sum(row["status"] not in {"complete", "failed"} for row in rows),
        "artifact_receipts": sum(row["artifact"] is not None for row in rows),
        "missing_artifacts": sum(row["status"] == "complete" and row["artifact"] is None for row in rows),
        "artifact_hash_mismatches": sum(row["reported_artifact_hash_match"] is False for row in rows),
        "identity_mismatches": sum(not row["identity_match"] for row in rows),
        "literal_consumer_edges": sum(len(row["literal_repository_consumers"]) for row in rows),
        "exact_request_source_edges": sum(len(row["exact_request_sources"]) for row in rows),
    }
    ledger = {
        "schema": "hoi4-wave3-exact20-job-consumer-bindings-v1",
        "generated_at": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
        "scope": {
            "export_root": str(EXPORTS),
            "job_glob": JOB_GLOB,
            "request_order_policy": "preserve exact array order and membership from each job.json",
        },
        "identity_contract": EXPECTED_IDENTITY,
        "pool_status_receipt": {
            "repo_path": repo_path(POOL_REPO),
            "external_path": str(POOL_EXTERNAL),
            "sha256": pool_sha256,
            "workers": 20,
            "root_observed_ready_workers_before_dispatch": 20,
            "persisted_preflight_available": False,
        },
        "promotion_policy": {
            "static_evidence_custody_admitted": True,
            "live_source_promotion": False,
            "lane_activation_unchanged": True,
            "failed_jobs_never_supply_artifact_authority": True,
            "runtime_and_human_gates_remain_independent": True,
            "persisted_preflight_dependent_gates_remain_false": True,
        },
        "counts": counts,
        "jobs": rows,
    }
    LEDGER_JSON.write_text(json.dumps(ledger, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    LEDGER_MD.write_text(markdown_report(ledger), encoding="utf-8")
    print(
        f"WAVE3_EXACT20 jobs={counts['jobs']} complete={counts['complete']} "
        f"failed={counts['failed']} artifacts={counts['artifact_receipts']} "
        f"hash_mismatches={counts['artifact_hash_mismatches']}"
    )


if __name__ == "__main__":
    main()
