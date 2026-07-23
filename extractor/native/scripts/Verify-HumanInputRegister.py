#!/usr/bin/env python3
"""Fail closed on drift in the production human-input register.

This is an offline metadata verifier.  It deliberately does not launch the game,
read a live process, mutate evidence, or choose values for an operator.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from pathlib import Path
from typing import Any


REGISTER = Path("extractor/documents/PRODUCTION_PROMOTION_HUMAN_INPUT_REGISTER_20260719.json")
SCHEMA = "hoi4-production-promotion-human-input-register-v4"
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
PATH_FIELDS = {"authority", "validation", "source", "evidence"}
SOURCE_MARKERS = ("human_input", "human_fixture", "human_capture")
SOURCE_EXACT_NAMES = {
    "artifact_gap_request.json",
    "data_blocked.md",
    "data_blocked_contract.json",
}
EXCLUDED_PARTS = {
    "archive",
    "runs",
    "final_integration",
    "fixtures",
}


class VerificationError(RuntimeError):
    pass


def fail(message: str) -> None:
    raise VerificationError(message)


def reject_duplicate_keys(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            fail(f"duplicate JSON object member: {key!r}")
        result[key] = value
    return result


def reject_constant(value: str) -> None:
    fail(f"non-finite JSON number is forbidden: {value}")


def load_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(
            path.read_text(encoding="utf-8"),
            object_pairs_hook=reject_duplicate_keys,
            parse_constant=reject_constant,
        )
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        fail(f"cannot load strict JSON {path}: {exc}")
    if not isinstance(value, dict):
        fail(f"top-level JSON value must be an object: {path}")
    return value


def require_keys(value: dict[str, Any], required: set[str], context: str) -> None:
    missing = sorted(required - value.keys())
    if missing:
        fail(f"{context} missing keys: {missing}")


def require_repo_path(repo: Path, raw: Any, context: str) -> Path:
    if not isinstance(raw, str) or not raw or "\\" in raw:
        fail(f"{context} must be a nonempty repo-relative forward-slash path")
    candidate = Path(raw)
    if candidate.is_absolute() or ".." in candidate.parts:
        fail(f"{context} escapes the repository: {raw!r}")
    resolved = (repo / candidate).resolve()
    try:
        resolved.relative_to(repo)
    except ValueError:
        fail(f"{context} resolves outside the repository: {raw!r}")
    if not resolved.is_file():
        fail(f"{context} does not exist as a file: {raw!r}")
    return resolved


def discover_human_source_docs(repo: Path) -> set[str]:
    found: set[str] = set()
    for top in (repo / "extractor", repo / "replay_interface"):
        for path in top.rglob("*"):
            if not path.is_file():
                continue
            relative = path.relative_to(repo)
            lowered_parts = {part.lower() for part in relative.parts}
            if lowered_parts & EXCLUDED_PARTS:
                continue
            if any(part.lower().startswith(("target", "scratch")) for part in relative.parts):
                continue
            if relative.as_posix() == REGISTER.as_posix():
                continue
            name = path.name.lower()
            if any(marker in name for marker in SOURCE_MARKERS) or name in SOURCE_EXACT_NAMES:
                if path.suffix.lower() in {".json", ".md"}:
                    found.add(relative.as_posix())
    return found


def validate_paths(repo: Path, value: Any, context: str = "register") -> None:
    if isinstance(value, dict):
        for key, child in value.items():
            child_context = f"{context}.{key}"
            if key in PATH_FIELDS:
                require_repo_path(repo, child, child_context)
            else:
                validate_paths(repo, child, child_context)
    elif isinstance(value, list):
        for index, child in enumerate(value):
            validate_paths(repo, child, f"{context}[{index}]")


def validate_exact10_authority(repo: Path, value: Any) -> None:
    context = "exact10_authority"
    if not isinstance(value, dict):
        fail(f"{context} must be an object")
    expected_keys = {
        "receipt_path",
        "receipt_sha256",
        "reconciliation_path",
        "reconciliation_sha256",
        "consumer_ledger_path",
        "consumer_ledger_sha256",
        "successful_root_receipts",
        "expected_failure_receipts",
        "unexpected_failures",
        "successful_claim_consumer_rows_bound",
        "consumer_root_bindings_inline_complete",
        "expected_failure_policy",
    }
    require_keys(value, expected_keys, context)
    if set(value) != expected_keys:
        fail(f"{context} has unexpected keys: {sorted(set(value) - expected_keys)}")

    for stem in ("receipt", "reconciliation", "consumer_ledger"):
        path_key = f"{stem}_path"
        sha_key = f"{stem}_sha256"
        path = require_repo_path(repo, value[path_key], f"{context}.{path_key}")
        expected_sha = value[sha_key]
        if not isinstance(expected_sha, str) or not SHA256_RE.fullmatch(expected_sha):
            fail(f"{context}.{sha_key} is invalid")
        actual_sha = hashlib.sha256(path.read_bytes()).hexdigest()
        if actual_sha != expected_sha:
            fail(
                f"{context}.{sha_key} does not match {value[path_key]!r}: "
                f"expected {expected_sha}, got {actual_sha}"
            )

    expected_scalars = {
        "successful_root_receipts": 43,
        "expected_failure_receipts": 2,
        "unexpected_failures": 0,
        "successful_claim_consumer_rows_bound": "110/110",
        "consumer_root_bindings_inline_complete": "107/107",
    }
    for key, expected in expected_scalars.items():
        if value[key] != expected:
            fail(f"{context}.{key} drift: expected {expected!r}, got {value[key]!r}")
    policy = value["expected_failure_policy"]
    for token in ("EF-01-D0-P0", "EF-02-D1-P1", "0x140082cb0", "no artifact exists or is claimed"):
        if not isinstance(policy, str) or token not in policy:
            fail(f"{context}.expected_failure_policy is missing {token!r}")


def validate_closure_evidence(repo: Path) -> None:
    m03 = load_json(repo / "replay_interface/re_evidence/rendering_laws/M03/static_semantic_completion.json")
    if m03.get("operator_input_required") is not False:
        fail("M03 static completion no longer proves operator_input_required=false")
    a48 = load_json(repo / "replay_interface/re_evidence/rendering_laws/M03/a48_production_contract.json")
    if a48.get("only_external_input_remaining", object()) is not None:
        fail("M03 A48 contract again declares an external input")
    if a48.get("completion_proof", {}).get("operator_input_required") is not False:
        fail("M03 A48 completion proof again requires operator input")

    r01 = load_json(repo / "replay_interface/re_evidence/rendering_laws/R01/artifact_gap_request.json")
    if r01.get("installed_topology_status") != "SATISFIED_PRODUCTION_READY":
        fail("R01 installed topology is no longer closed")
    if r01.get("open_installed_topology_questions") != []:
        fail("R01 again contains open installed-topology questions")

    t00 = load_json(repo / "replay_interface/re_evidence/rendering_laws/T00/DATA_BLOCKED_CONTRACT.json")
    if t00.get("disposition") != "SUPERSEDED_BY_SYNCHRONIZED_CAPTURE":
        fail("T00 historical blocker is no longer marked superseded")
    t01 = load_json(repo / "replay_interface/re_evidence/rendering_laws/T01/DATA_BLOCKED_CONTRACT.json")
    if t01.get("disposition") != "RESOLVED" or t01.get("promotion") != "PROVEN_PROMOTION_READY":
        fail("T01 former blocker is no longer resolved and promotion-ready")

    program = (repo / "replay_interface/documents/lane_reports/rendering_laws/PROGRAM_STATUS.md").read_text(encoding="utf-8")
    if "| R03 | `r03_strait_mesh` | PROVEN |" not in program:
        fail("PROGRAM_STATUS no longer marks R03 PROVEN")
    if "| O04 | `cont_o06` | PROVEN |" not in program:
        fail("PROGRAM_STATUS no longer marks O04 PROVEN")

    w20_receipt = load_json(repo / "replay_interface/documents/lane_reports/evidence/w20-rdc-integrity/w20-rdc-corpus-integrity.recovery.json")
    w20_path = require_repo_path(repo, w20_receipt.get("restored_repo_path"), "W20 recovery restored_repo_path")
    w20_bytes = w20_path.read_bytes()
    if w20_receipt.get("all_identities_verified_after_restore") is not True:
        fail("W20 recovery receipt no longer verifies all restored identities")
    w20_sha256 = hashlib.sha256(w20_bytes).hexdigest()
    frozen_bytes = 392_964
    frozen_sha256 = "9715a81dababc7c6d888a957ec102e73f2d5cc01b3cd93d2f995facd56d23e4d"
    checkout_bytes = 384_598
    checkout_sha256 = "4804b6618cdc73d97b9871bb86328218ed4181e2322259db79c3b29979025e70"
    checkout_lf_count = 8_366
    if (
        w20_receipt.get("restored_bytes") != frozen_bytes
        or w20_receipt.get("restored_sha256") != frozen_sha256
    ):
        fail("W20 recovery receipt frozen identity drift")
    if not (
        len(w20_bytes) == checkout_bytes
        and w20_sha256 == checkout_sha256
        and b"\r" not in w20_bytes
        and w20_bytes.count(b"\n") == checkout_lf_count
        and w20_receipt.get("source_bytes") == checkout_bytes
        and w20_receipt.get("source_sha256") == checkout_sha256
        and w20_receipt.get("source_lf_count") == checkout_lf_count
        and w20_receipt.get("current_checkout_bytes") == checkout_bytes
        and w20_receipt.get("current_checkout_sha256") == checkout_sha256
        and w20_receipt.get("current_checkout_lf_count") == checkout_lf_count
        and w20_receipt.get("current_checkout_representation")
        == "Git-normalized LF bytes; the receipt-bound transform reproduces the frozen CRLF identity"
        and w20_receipt.get("checkout_transform")
        == "replace each LF with CRLF; source contains zero CRLF pairs"
    ):
        fail(
            "W20 index is not the exact receipt-bound Git-normalized LF checkout: "
            f"bytes={len(w20_bytes)}, sha256={w20_sha256}"
        )
    restored = w20_bytes.replace(b"\n", b"\r\n")
    if len(restored) != frozen_bytes or hashlib.sha256(restored).hexdigest() != frozen_sha256:
        fail("W20 authenticated LF checkout does not reproduce the frozen CRLF identity")

    anomaly = (repo / "replay_interface/native/replay_app/src/anomaly.rs").read_text(encoding="utf-8")
    timeline = (repo / "replay_interface/native/replay_app/src/timeline.rs").read_text(encoding="utf-8")
    validation = (repo / "replay_interface/native/replay_app/src/validate.rs").read_text(encoding="utf-8")
    # F4 moved from an eager whole-recording scan at provider construction to
    # ordered incremental validation through the requested hour.  Verify the
    # semantic boundary rather than pinning one obsolete call-site spelling:
    # every newly crossed frame is loaded before observe(), scan progress moves
    # only afterwards, playback maps a load failure to the typed terminal error,
    # and explicit certification still exhaustively validates every payload.
    required_f4_patterns = {
        "anomaly.rs": [
            ("stateful anomaly policy", r"pub\s+struct\s+AnomalyPolicy\b"),
            ("frame observation API", r"pub\s+fn\s+observe\s*\(\s*&mut\s+self\s*,\s*frame:\s*&WorldFrame"),
        ],
        "timeline.rs": [
            ("incremental scan state", r"struct\s+IncrementalAnomalyScan\b"),
            ("ordered prefix boundary", r"partition_point\s*\(\s*\|hour\|\s*\*hour\s*<=\s*target_hour\s*\)"),
            (
                "authenticate then observe then advance",
                r"\.frame\s*\(\s*hour\s*\)[\s\S]{0,700}?"
                r"scan\.policy\.observe\s*\(\s*&frame\s*\)[\s\S]{0,300}?"
                r"scan\.next_index\s*\+=\s*1",
            ),
            (
                "terminal incremental frame-load mapping",
                r"F4\s+fail-closed\s+incremental\s+validation\s+failed\s+at\s+hour",
            ),
            (
                "provider boundary validates before frame return",
                r"fn\s+selected_frame\s*\([^)]*\)[\s\S]{0,500}?"
                r"scan_anomalies_through\s*\(\s*canonical_hour\s*\)\?",
            ),
            (
                "incremental ordering regression test",
                r"fn\s+anomaly_validation_is_lazy_ordered_and_does_not_scan_at_open\s*\(",
            ),
            (
                "anomaly-note failure regression test",
                r"fn\s+anomaly_note_validates_the_frame_before_reporting_no_hold\s*\(",
            ),
        ],
        "validate.rs": [
            ("exhaustive certification function", r"fn\s+validate_all_frame_payloads\s*\("),
            (
                "every indexed payload is decoded",
                r"for\s+&hour\s+in\s+timeline\.hours\s*\(\s*\)\s*\{"
                r"[\s\S]{0,300}?timeline\.frame\s*\(\s*hour\s*\)\.map_err",
            ),
            (
                "lazy integrity regression test",
                r"fn\s+exhaustive_validation_rejects_a_lazy_midstream_sha_mismatch\s*\(",
            ),
        ],
    }
    texts = {"anomaly.rs": anomaly, "timeline.rs": timeline, "validate.rs": validation}
    for filename, patterns in required_f4_patterns.items():
        for description, pattern in patterns:
            if re.search(pattern, texts[filename]) is None:
                fail(f"F4 closure semantic missing from {filename}: {description}")

    main = (repo / "replay_interface/native/replay_app/src/main.rs").read_text(encoding="utf-8")
    blockers = (repo / "replay_interface/native/replay_app/src/blockers.rs").read_text(encoding="utf-8")
    for token in (
        "registry_covers_every_product_path_gate",
        "read_dir",
        "product_path_test",
    ):
        if token not in main:
            fail(f"blocker-registry coverage token missing from main.rs: {token!r}")
    if "pub const REGISTRY" not in blockers:
        fail("blocker registry authority is missing from blockers.rs")


def verify(repo: Path) -> dict[str, Any]:
    repo = repo.resolve()
    register_path = repo / REGISTER
    register = load_json(register_path)
    required_top = {
        "schema",
        "date",
        "policy",
        "authority",
        "validation",
        "exact10_authority",
        "dependency_reclassification",
        "optional_evidence",
        "genuine_external_dependencies",
        "feature_local_omissions",
        "canonical_human_inputs",
        "aliases",
        "missing_authenticated_artifacts",
        "product_decisions",
        "non_human_residuals",
        "source_dispositions",
    }
    require_keys(register, required_top, "register")
    if register["schema"] != SCHEMA:
        fail(f"unexpected register schema: {register['schema']!r}")
    if set(register) != required_top:
        fail(f"unexpected top-level keys: {sorted(set(register) - required_top)}")

    validate_exact10_authority(repo, register["exact10_authority"])

    canonical = register["canonical_human_inputs"]
    if not isinstance(canonical, list) or not canonical:
        fail("canonical_human_inputs must be a nonempty array")
    canonical_ids: set[str] = set()
    allowed_classes = {
        "optional_state_capture",
        "optional_same_tick_pair",
        "optional_temporal_capture",
        "optional_lifecycle_capture",
        "optional_early_launch_capture",
        "optional_environment_fixture",
        "optional_live_acceptance",
        "optional_deployment_observation",
    }
    allowed_states = {"optional_nonblocking_regression_evidence"}
    for index, item in enumerate(canonical):
        context = f"canonical_human_inputs[{index}]"
        if not isinstance(item, dict):
            fail(f"{context} must be an object")
        require_keys(item, {"id", "class", "category", "state", "action", "source"}, context)
        if set(item) != {"id", "class", "category", "state", "action", "source"}:
            fail(f"{context} has unexpected keys")
        item_id = item["id"]
        if not isinstance(item_id, str) or not re.fullmatch(r"H-[A-Z0-9-]+", item_id):
            fail(f"{context}.id is not an atomic canonical H-ID: {item_id!r}")
        if item_id in canonical_ids:
            fail(f"duplicate canonical human-input id: {item_id}")
        canonical_ids.add(item_id)
        if item["class"] not in allowed_classes:
            fail(f"{context}.class is invalid: {item['class']!r}")
        if item["state"] not in allowed_states:
            fail(f"{context}.state is invalid: {item['state']!r}")
        for key in ("category", "action"):
            if not isinstance(item[key], str) or not item[key].strip():
                fail(f"{context}.{key} must be nonempty")
        forbidden_gate_claims = (
            r"(?:activation|scheduling|promotion) remains false until",
            r"remains unscheduled until",
            r"zero rows remain promoted until",
            r"missing evidence keeps only this feature",
        )
        for pattern in forbidden_gate_claims:
            if re.search(pattern, item["action"], flags=re.IGNORECASE):
                fail(f"{context}.action incorrectly makes optional evidence a product gate")

    if any(item_id.startswith("H-M03") for item_id in canonical_ids):
        fail("retired M03 work reappeared as a canonical acquisition")

    reclassification = register["dependency_reclassification"]
    if not isinstance(reclassification, dict):
        fail("dependency_reclassification must be an object")
    expected_reclassification_keys = {
        "as_of",
        "user_action_required",
        "optional_nonblocking_regression_evidence",
        "h_card_unresolved_re_or_engineering",
        "retired_h_cards",
        "authorized_defaults",
        "classification_rule",
    }
    require_keys(reclassification, expected_reclassification_keys, "dependency_reclassification")
    if set(reclassification) != expected_reclassification_keys:
        fail("dependency_reclassification has unexpected keys")
    classified: set[str] = set()
    for key in (
        "user_action_required",
        "optional_nonblocking_regression_evidence",
        "h_card_unresolved_re_or_engineering",
        "retired_h_cards",
    ):
        values = reclassification[key]
        if not isinstance(values, list) or len(values) != len(set(values)):
            fail(f"dependency_reclassification.{key} must be a unique array")
        overlap = classified & set(values)
        if overlap:
            fail(f"dependency_reclassification arrays overlap: {sorted(overlap)}")
        classified.update(values)
    if classified != canonical_ids:
        fail(
            "dependency_reclassification must exhaust canonical ids: "
            f"missing={sorted(canonical_ids - classified)}, "
            f"unknown={sorted(classified - canonical_ids)}"
        )
    canonical_by_id = {item["id"]: item for item in canonical}
    if reclassification["h_card_unresolved_re_or_engineering"]:
        fail("H cards cannot conceal unresolved RE or engineering")
    if reclassification["retired_h_cards"]:
        fail("retired H cards must remain outside the canonical set")
    for item_id in reclassification["optional_nonblocking_regression_evidence"]:
        if canonical_by_id[item_id]["state"] != "optional_nonblocking_regression_evidence":
            fail(f"optional evidence card has a blocking state: {item_id}")
    defaults = reclassification["authorized_defaults"]
    expected_default_keys = {
        "save_and_scene",
        "profile_modset_and_target",
        "candidate_review",
        "boundary",
    }
    if not isinstance(defaults, dict) or set(defaults) != expected_default_keys:
        fail("dependency_reclassification.authorized_defaults has unexpected keys")
    if reclassification["user_action_required"]:
        fail("current user authorization leaves no canonical card user-blocking")

    optional_evidence = register["optional_evidence"]
    expected_optional_keys = {
        "pending_regression_receipts",
        "accepted_regression_receipts",
        "product_activation_dependencies",
        "rule",
    }
    if not isinstance(optional_evidence, dict) or set(optional_evidence) != expected_optional_keys:
        fail("optional_evidence has unexpected keys")
    pending = optional_evidence["pending_regression_receipts"]
    accepted = optional_evidence["accepted_regression_receipts"]
    activation_dependencies = optional_evidence["product_activation_dependencies"]
    for name, values in (("pending", pending), ("accepted", accepted)):
        if not isinstance(values, list) or len(values) != len(set(values)):
            fail(f"optional_evidence {name} receipts must be a unique array")
    if set(pending) & set(accepted):
        fail("optional evidence pending and accepted sets overlap")
    if set(pending) | set(accepted) != canonical_ids:
        fail("optional evidence status must exhaust canonical H IDs")
    if activation_dependencies != []:
        fail("H evidence must never appear as a product activation dependency")
    if accepted:
        fail("no optional regression receipt is accepted in this authority refreeze")

    external_dependencies = register["genuine_external_dependencies"]
    if not isinstance(external_dependencies, list) or len(external_dependencies) != 1:
        fail("exactly one genuine external dependency must be listed")
    dependency = external_dependencies[0]
    require_keys(dependency, {"id", "state", "detail", "source"}, "genuine_external_dependencies[0]")
    if set(dependency) != {"id", "state", "detail", "source"}:
        fail("genuine external dependency has unexpected keys")
    if dependency.get("id") != "P3-M3-SECOND-MACHINE" or dependency.get("state") != "external_endpoint_required":
        fail("the sole genuine external dependency is not P3 M3")

    omissions = register["feature_local_omissions"]
    expected_omission_ids = {
        "B-A24-POLITICS-TOOLTIP-PRODUCER-TAPS",
        "B-A25-GUI-PAINT-OWNER-PATH",
        "B-A28-TECHNOLOGY-CANVAS",
        "B-A28-MIO-AGGREGATE-BONUS",
        "B-D74-POWER-BALANCE-SEMANTIC-SEALER",
        "B-D87-RESOLVED-DECISION-PANEL-SUPPLEMENT-PRODUCER",
    }
    if not isinstance(omissions, list):
        fail("feature_local_omissions must be an array")
    omission_ids: set[str] = set()
    for index, omission in enumerate(omissions):
        context = f"feature_local_omissions[{index}]"
        if not isinstance(omission, dict):
            fail(f"{context} must be an object")
        expected_keys = {"id", "feature", "state", "reason", "source"}
        if set(omission) != expected_keys:
            fail(f"{context} has unexpected keys")
        omission_id = omission["id"]
        if not isinstance(omission_id, str) or omission_id in omission_ids:
            fail(f"{context}.id must be a unique string")
        omission_ids.add(omission_id)
        if omission["state"] != "omitted_fail_closed":
            fail(f"{context} is not feature-local fail-closed")
        for key in ("feature", "reason"):
            if not isinstance(omission[key], str) or not omission[key].strip():
                fail(f"{context}.{key} must be nonempty")
    if omission_ids != expected_omission_ids:
        fail(
            "feature-local omission inventory drift: "
            f"missing={sorted(expected_omission_ids - omission_ids)}, "
            f"unknown={sorted(omission_ids - expected_omission_ids)}"
        )

    aliases = register["aliases"]
    alias_ids: set[str] = set()
    for index, alias in enumerate(aliases):
        context = f"aliases[{index}]"
        if not isinstance(alias, dict):
            fail(f"{context} must be an object")
        require_keys(alias, {"alias", "canonical_ids"}, context)
        alias_id = alias["alias"]
        if alias_id in alias_ids or alias_id in canonical_ids:
            fail(f"duplicate or colliding alias: {alias_id!r}")
        alias_ids.add(alias_id)
        targets = alias["canonical_ids"]
        if not isinstance(targets, list) or not targets or len(targets) != len(set(targets)):
            fail(f"{context}.canonical_ids must be a nonempty unique list")
        unknown = sorted(set(targets) - canonical_ids)
        if unknown:
            fail(f"{context} points at unknown canonical ids: {unknown}")

    all_ids = set(canonical_ids)
    for section, prefix in (
        ("missing_authenticated_artifacts", None),
        ("product_decisions", "D-"),
        ("non_human_residuals", "NH-"),
    ):
        values = register[section]
        if not isinstance(values, list):
            fail(f"{section} must be an array")
        for index, item in enumerate(values):
            if not isinstance(item, dict) or not isinstance(item.get("id"), str):
                fail(f"{section}[{index}] must have a string id")
            item_id = item["id"]
            if prefix and not item_id.startswith(prefix):
                fail(f"{section}[{index}].id must start with {prefix}")
            if item_id in all_ids:
                fail(f"duplicate id across register sections: {item_id}")
            all_ids.add(item_id)

    for index, artifact in enumerate(register["missing_authenticated_artifacts"]):
        require_keys(artifact, {"id", "state", "path_hint", "byte_len", "sha256", "effect", "source"}, f"missing_authenticated_artifacts[{index}]")
        if not isinstance(artifact["byte_len"], int) or artifact["byte_len"] <= 0:
            fail(f"missing_authenticated_artifacts[{index}].byte_len must be positive")
        if not isinstance(artifact["sha256"], str) or not SHA256_RE.fullmatch(artifact["sha256"]):
            fail(f"missing_authenticated_artifacts[{index}].sha256 is invalid")

    for index, decision in enumerate(register["product_decisions"]):
        require_keys(decision, {"id", "decision"}, f"product_decisions[{index}]")
        if set(decision) != {"id", "decision"}:
            fail(f"product_decisions[{index}] has unexpected keys")
        if not isinstance(decision["decision"], str) or not decision["decision"].strip():
            fail(f"product_decisions[{index}].decision must be nonempty")

    package_decision_ids = {
        "D-SIGNATURE",
        "D-TRUST-LIFECYCLE",
        "D-PLATFORM-SIGNING",
        "D-INSTALL-SCOPE-ACL",
        "D-TRANSPORT-STAGING",
        "D-RESIDUE-RECOVERY",
        "D-RELEASE-POLICY",
        "D-WINDOWS-MITIGATIONS",
        "D-INSTALL-TOPOLOGY",
        "D-PRODUCT-SURFACE",
        "D-MODSET",
        "D-WEAK-PC",
    }
    register_decisions = {item["id"]: item["decision"] for item in register["product_decisions"]}
    if set(register_decisions) != package_decision_ids:
        fail(
            "resolved product-policy inventory drift: "
            f"missing={sorted(package_decision_ids - set(register_decisions))}, "
            f"unknown={sorted(set(register_decisions) - package_decision_ids)}"
        )
    unresolved_decisions = sorted(
        decision_id
        for decision_id, decision in register_decisions.items()
        if not decision.startswith("RESOLVED:")
    )
    if unresolved_decisions:
        fail(f"product-policy decisions are not resolved: {unresolved_decisions}")

    install_matrix = load_json(repo / "replay_interface/documents/PACKAGE_INSTALL_ACCEPTANCE_MATRIX.json")
    expected_matrix_keys = {
        "schema",
        "product_id",
        "status",
        "acceptance_state",
        "selected_distribution",
        "resolved_product_policies",
        "unresolved_human_product_policy_inputs",
        "publication_authority",
        "mechanical_cases",
        "legacy_package_runtime_disposition",
        "explicitly_out_of_scope",
    }
    if set(install_matrix) != expected_matrix_keys:
        fail("package install matrix has unexpected keys")
    if install_matrix.get("schema") != "hoi4.replay.package-install-matrix.v2":
        fail("package install matrix schema drift")
    if install_matrix.get("product_id") != "battle-plan":
        fail("package install matrix product identity drift")
    if install_matrix.get("status") != "policy_resolved_portable_zip_acceptance_spec":
        fail("package install matrix status drift")
    if install_matrix.get("acceptance_state") != "specified_not_asserted_by_this_document":
        fail("package install matrix overclaims acceptance")
    if install_matrix.get("unresolved_human_product_policy_inputs") != []:
        fail("resolved package policy cannot retain pending human product-policy inputs")

    matrix_policy_rows = install_matrix.get("resolved_product_policies")
    if not isinstance(matrix_policy_rows, list):
        fail("package install matrix resolved_product_policies must be an array")
    matrix_decisions: dict[str, str] = {}
    for index, decision in enumerate(matrix_policy_rows):
        context = f"package install matrix resolved_product_policies[{index}]"
        if not isinstance(decision, dict) or set(decision) != {"id", "decision"}:
            fail(f"{context} has unexpected shape")
        decision_id = decision.get("id")
        decision_text = decision.get("decision")
        if not isinstance(decision_id, str) or decision_id in matrix_decisions:
            fail(f"{context}.id must be a unique string")
        if not isinstance(decision_text, str) or not decision_text.startswith("RESOLVED:"):
            fail(f"{context}.decision is not resolved")
        matrix_decisions[decision_id] = decision_text
    if matrix_decisions != register_decisions:
        fail("package install matrix does not preserve the exact resolved product-policy values")

    selected = install_matrix.get("selected_distribution")
    if not isinstance(selected, dict):
        fail("package install matrix selected_distribution must be an object")
    selected_contract = {
        "install_kind": "portable_zip",
        "delivery": "offline_manual_download",
        "release_channel": "single_stable_channel",
        "automatic_updater": False,
        "publisher_signature_gate": False,
        "authenticode_required": False,
        "administrator_required": False,
        "desktop_surface": "normal_window",
        "package_entries": [
            "Battle Plan Recorder.exe",
            "Battle Plan.exe",
            "collector.dll",
            "README.txt",
            "SHA256SUMS.txt",
        ],
        "checksum_semantics": "accidental_corruption_detection_only_not_publisher_authentication",
        "writable_state": "current_user_application_data",
        "recorder_payload_rule": (
            "collector.dll_must_be_a_nonempty_non_alias_sibling_of_the_recorder_executable"
        ),
    }
    if selected != selected_contract:
        fail("package install matrix selected portable distribution drift")

    publication = install_matrix.get("publication_authority")
    if not isinstance(publication, dict):
        fail("package install matrix publication_authority must be an object")
    required_publication_values = {
        "builder": "extractor/native/scripts/New-PortableProductPackage.ps1",
        "verifier": "extractor/native/scripts/Test-PortableProductPackageReceipt.ps1",
        "required_validation": "unfiltered centralized Release -Workspace All",
        "required_result": "PASS_CENTRAL_RELEASE_PORTABLE_PACKAGE",
        "required_receipt": ".cargo-target/release/hoi4-portable-package-receipt.json",
        "canonical_receipt_required": True,
    }
    for key, expected in required_publication_values.items():
        if publication.get(key) != expected:
            fail(f"package install matrix publication authority drift: {key}")

    legacy_runtime = install_matrix.get("legacy_package_runtime_disposition")
    if not isinstance(legacy_runtime, dict) or legacy_runtime.get("state") != "not_selected_for_the_portable_zip_product":
        fail("legacy signed package runtime reappeared as a portable-product prerequisite")

    for index, residual in enumerate(register["non_human_residuals"]):
        require_keys(residual, {"id", "state", "detail", "evidence"}, f"non_human_residuals[{index}]")
        if set(residual) != {"id", "state", "detail", "evidence"}:
            fail(f"non_human_residuals[{index}] has unexpected keys")
        for key in ("state", "detail"):
            if not isinstance(residual[key], str) or not residual[key].strip():
                fail(f"non_human_residuals[{index}].{key} must be nonempty")

    dispositions = register["source_dispositions"]
    disposition_paths: set[str] = set()
    for index, disposition in enumerate(dispositions):
        if not isinstance(disposition, dict):
            fail(f"source_dispositions[{index}] must be an object")
        require_keys(disposition, {"path", "disposition"}, f"source_dispositions[{index}]")
        path = disposition["path"]
        if path in disposition_paths:
            fail(f"duplicate source disposition path: {path!r}")
        disposition_paths.add(path)
        require_repo_path(repo, path, f"source_dispositions[{index}].path")

    discovered = discover_human_source_docs(repo)
    if discovered != disposition_paths:
        missing = sorted(discovered - disposition_paths)
        stale = sorted(disposition_paths - discovered)
        fail(f"human-source census drift: unclassified={missing}, no_longer_discovered={stale}")

    validate_paths(repo, register)
    validate_closure_evidence(repo)
    return {
        "schema": "hoi4-production-promotion-human-input-register-verification-v1",
        "result": "PASS",
        "canonical_human_input_count": len(canonical_ids),
        "user_action_required_count": len(reclassification["user_action_required"]),
        "optional_nonblocking_regression_evidence_count": len(
            reclassification["optional_nonblocking_regression_evidence"]
        ),
        "h_card_unresolved_re_or_engineering_count": len(
            reclassification["h_card_unresolved_re_or_engineering"]
        ),
        "retired_h_card_count": len(reclassification["retired_h_cards"]),
        "pending_regression_receipt_count": len(pending),
        "accepted_regression_receipt_count": len(accepted),
        "product_activation_dependency_count": len(activation_dependencies),
        "genuine_external_dependency_count": len(external_dependencies),
        "feature_local_omission_count": len(omission_ids),
        "alias_count": len(alias_ids),
        "missing_authenticated_artifact_count": len(register["missing_authenticated_artifacts"]),
        "product_decision_count": len(register["product_decisions"]),
        "non_human_residual_count": len(register["non_human_residuals"]),
        "source_disposition_count": len(disposition_paths),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[3])
    args = parser.parse_args()
    try:
        result = verify(args.repo)
    except VerificationError as exc:
        print(json.dumps({"result": "FAIL", "error": str(exc)}, sort_keys=True))
        return 1
    print(json.dumps(result, indent=2, sort_keys=True, allow_nan=False))
    return 0


if __name__ == "__main__":
    sys.exit(main())
