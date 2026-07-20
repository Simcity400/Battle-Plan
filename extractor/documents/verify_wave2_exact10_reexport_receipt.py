#!/usr/bin/env python3
"""Fail-closed admission check for the Wave 2 exact-10 Ghidra re-export.

This verifier is intentionally standard-library-only.  It does not invoke
Ghidra and it never mutates the manifest, reconciliation, jobs, artifacts, or
consumers.

Each populated ``replacement_result.pool_status_receipt`` must point to a JSON
document with this normalized shape::

    {
      "schema": "hoi4-ghidra-pool-status-receipt-v1",
      "binary_id": "hoi4.exe 1.19.2.0.a729",
      "image_base": "0x140000000",
      "sha256": "870e...65ad",
      "identity_manifest_sha256": "<64 lowercase hex>",
      "shard_count": 10,
      "ready_shard_ids": ["shard-01", ..., "shard-10"],
      "read_only": true,
      "analysis_enabled": false,
      "per_shard_serialization": true,
      "persistent_error": null
    }

The broker owns scheduling.  ``target_shard_id`` is checked only as a valid
advisory hint.  The actual job and shard come exclusively from the hashed
``job.json`` receipt and must name a shard present in the receipt's ready set.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from pathlib import Path
from typing import Any, Iterable


EXPECTED_BINARY_ID = "hoi4.exe 1.19.2.0.a729"
EXPECTED_IMAGE_BASE = "0x140000000"
EXPECTED_BINARY_SHA256 = (
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
)
EXPECTED_READY_SHARDS = tuple(f"shard-{number:02d}" for number in range(1, 11))
EXPECTED_DISCOVERY_ATTEMPTS = 123
EXPECTED_CANONICAL_ACTIONS = 122
EXPECTED_ROOT_CALLS = 45
EXPECTED_SUCCESS_ROOT_CALLS = 43
EXPORT_FUNCTIONS_MAX_ADDRESSES = 32
DUMP_DATA_MAX_ADDRESSES = 128
EXPECTED_SUPPLEMENTAL_ACTIONS = {
    "SP-001": "0x140fd02e0",
    "SP-002": "0x140fcf450",
    "SP-003": "0x140fcec50",
    "SP-004": "0x140fcbb50",
    "SP-005": "0x141a24a30",
    "D81-001": "0x140b626f0",
    "D81-002": "0x140b629c0",
    "D81-003": "0x140b63880",
    "D81-004": "0x140b64520",
    "D81-005": "0x14127afb0",
    "D81-006": "0x14162d360",
    "D81-007": "0x1418fd210",
    "X2-001": "0x141299610",
    "X2-002": "0x1416dcdc0",
    "X2-003": "0x140cfd210",
    "X2-004": "0x140c6e270",
    "CONVOY-001": "0x141961dd0",
    "CONVOY-002": "0x14195ba40",
    "WEATHER-001": "0x14227e5f0",
    "WEATHER-002": "0x142278710",
    "WEATHER-003": "0x142277df0",
    "WEATHER-004": "0x142273fb0",
    "GH04-001": "0x1411b2420",
    "NAVAL-001": "0x140d5a690",
}
EXPECTED_SUPPLEMENTAL_SEMANTICS = {
    **{item: (1, True, "SP-01") for item in ("SP-001", "SP-002", "SP-003", "SP-004", "SP-005")},
    "D81-001": (1, False, "SUP-03-D1-P0"),
    "D81-002": (1, False, "SUP-03-D1-P0"),
    "D81-003": (1, True, "SUP-02-D1-P1"),
    "D81-004": (1, False, "SUP-03-D1-P0"),
    "D81-005": (1, True, "SUP-02-D1-P1"),
    "D81-006": (1, True, "SUP-02-D1-P1"),
    "D81-007": (1, True, "SUP-02-D1-P1"),
    "X2-001": (0, True, "SUP-01-D0-P1"),
    "X2-002": (0, True, "SUP-01-D0-P1"),
    "X2-003": (0, True, "SUP-01-D0-P1"),
    "X2-004": (2, True, "SUP-04-D2-P1"),
    "CONVOY-001": (1, True, "SUP-02-D1-P1"),
    "CONVOY-002": (1, False, "SUP-03-D1-P0"),
    "WEATHER-001": (1, True, "SUP-02-D1-P1"),
    "WEATHER-002": (1, True, "SUP-02-D1-P1"),
    "WEATHER-003": (1, True, "SUP-02-D1-P1"),
    "WEATHER-004": (1, True, "SUP-02-D1-P1"),
    "GH04-001": (1, True, "SUP-02-D1-P1"),
    "NAVAL-001": (1, True, "SUP-02-D1-P1"),
}
EXPECTED_SUPPLEMENTAL_CONSUMERS = {
    "extractor/native/project_progress_live_lane/SPECIAL_PROJECT_LIFECYCLE_REWARD_RE_20260719.json",
    "extractor/native/project_progress_live_lane/SPECIAL_PROJECT_LIFECYCLE_REWARD_RE_20260719.md",
}
EXPECTED_FAILURE_ROOTS = {
    "EF-01-D0-P0": (18, "20260719T220039Z-wave2-r10-addr-01-02-562658b0"),
    "EF-02-D1-P1": (17, "20260719T220127Z-wave2-r10-addr-03-01-631965a4"),
}
EXPECTED_PRIOR_FAILURES = {
    "20260719T132149Z-convoy-save-schema-serializer-326c710f",
    "20260719T132213Z-convoy-save-schema-serializer-no-pcode-9de6e1a6",
    "20260719T133604Z-occupation-garrison-serializer-root-2026-7584d456",
}
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
ADDRESS_RE = re.compile(r"^0x[0-9a-f]+$")
SHARD_RE = re.compile(r"^shard-(0[1-9]|10)$")


class DuplicateKeyError(ValueError):
    pass


def _reject_duplicate_keys(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise DuplicateKeyError(f"duplicate JSON key: {key}")
        result[key] = value
    return result


def load_json(path: Path) -> dict[str, Any]:
    with path.open("r", encoding="utf-8") as handle:
        value = json.load(handle, object_pairs_hook=_reject_duplicate_keys)
    if not isinstance(value, dict):
        raise ValueError(f"{path}: top-level JSON value is not an object")
    return value


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        while chunk := handle.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def powershell_compact_json(value: Any) -> str:
    """Match the compact serialization used to create source request hashes."""

    text = json.dumps(value, ensure_ascii=True, separators=(",", ":"))
    return (
        text.replace("<", r"\u003c")
        .replace(">", r"\u003e")
        .replace("&", r"\u0026")
        .replace("'", r"\u0027")
    )


def semantic_request(operation: str, request: dict[str, Any]) -> dict[str, Any]:
    if not isinstance(request, dict):
        raise ValueError("request is not an object")
    label = request.get("label")
    if label is not None and not isinstance(label, str):
        raise ValueError("request label is not a string")

    if operation == "search_strings":
        allowed = {"label", "terms"}
        terms = request.get("terms")
        if not isinstance(terms, list) or not terms or not all(
            isinstance(term, str) and term for term in terms
        ):
            raise ValueError("search_strings terms must be a non-empty string array")
        semantic = {"terms": list(terms)}
    elif operation == "dump_data":
        allowed = {"label", "addresses"}
        addresses = request.get("addresses")
        if not isinstance(addresses, list) or not addresses or not all(
            isinstance(address, str) and ADDRESS_RE.fullmatch(address)
            for address in addresses
        ):
            raise ValueError("dump_data addresses are invalid")
        semantic = {"addresses": list(addresses)}
    elif operation == "export_functions":
        allowed = {"label", "addresses", "depth", "pcode"}
        addresses = request.get("addresses")
        depth = request.get("depth")
        pcode = request.get("pcode")
        if not isinstance(addresses, list) or not addresses or not all(
            isinstance(address, str) and ADDRESS_RE.fullmatch(address)
            for address in addresses
        ):
            raise ValueError("export_functions addresses are invalid")
        if isinstance(depth, bool) or not isinstance(depth, int) or depth < 0:
            raise ValueError("export_functions depth is invalid")
        if not isinstance(pcode, bool):
            raise ValueError("export_functions pcode is invalid")
        semantic = {"addresses": list(addresses), "depth": depth, "pcode": pcode}
    else:
        raise ValueError(f"unsupported operation: {operation!r}")

    extras = set(request) - allowed
    if extras:
        raise ValueError(f"unexpected request keys: {sorted(extras)!r}")
    return semantic


def semantic_preimage(operation: str, request: dict[str, Any]) -> str:
    return powershell_compact_json(
        {"operation": operation, "request": semantic_request(operation, request)}
    )


def resolve_under(root: Path, raw_path: Any, description: str) -> Path:
    if not isinstance(raw_path, str) or not raw_path:
        raise ValueError(f"{description} path is missing")
    candidate = Path(raw_path)
    if not candidate.is_absolute():
        candidate = root / candidate
    resolved_root = root.resolve()
    resolved = candidate.resolve()
    try:
        resolved.relative_to(resolved_root)
    except ValueError as error:
        raise ValueError(f"{description} escapes {resolved_root}: {resolved}") from error
    return resolved


def flatten(values: Iterable[Iterable[Any]]) -> list[Any]:
    return [item for group in values for item in group]


class Gate:
    def __init__(self) -> None:
        self.errors: list[str] = []
        self.actual_results: dict[str, dict[str, Any]] = {}
        self.failed_replacements: list[str] = []

    def require(self, condition: bool, message: str) -> None:
        if not condition:
            self.errors.append(message)

    def guarded(self, context: str, callback: Any) -> Any:
        try:
            return callback()
        except Exception as error:  # fail closed while retaining other findings
            self.errors.append(f"{context}: {error}")
            return None

    def audit_structure(
        self,
        manifest: dict[str, Any],
        reconciliation: dict[str, Any],
    ) -> tuple[dict[int, dict[str, Any]], dict[str, dict[str, Any]], dict[str, dict[str, Any]]]:
        inventory = manifest.get("discovery_inventory")
        actions = manifest.get("reexport_actions")
        plan = reconciliation.get("consolidated_execution_plan")
        claims = reconciliation.get("claim_reconciliation")
        self.require(isinstance(inventory, list), "manifest discovery_inventory is absent")
        self.require(isinstance(actions, list), "manifest reexport_actions is absent")
        self.require(isinstance(plan, dict), "reconciliation execution plan is absent")
        self.require(isinstance(claims, list), "reconciliation claims are absent")
        if not all(
            (isinstance(inventory, list), isinstance(actions, list), isinstance(plan, dict), isinstance(claims, list))
        ):
            return {}, {}, {}

        calls = plan.get("root_calls")
        self.require(isinstance(calls, list), "root_calls is absent")
        if not isinstance(calls, list):
            return {}, {}, {}

        self.require(len(inventory) == EXPECTED_DISCOVERY_ATTEMPTS, "discovery count is not 123")
        self.require(len(actions) == EXPECTED_CANONICAL_ACTIONS, "canonical action count is not 122")
        self.require(len(claims) == EXPECTED_DISCOVERY_ATTEMPTS, "claim count is not 123")
        self.require(len(calls) == EXPECTED_ROOT_CALLS, "root-call count is not 45")

        inventory_by_id = {
            item.get("discovery_job_id"): item
            for item in inventory
            if isinstance(item, dict) and isinstance(item.get("discovery_job_id"), str)
        }
        action_by_sequence = {
            item.get("sequence"): item
            for item in actions
            if isinstance(item, dict) and isinstance(item.get("sequence"), int)
        }
        call_by_id = {
            item.get("root_call_id"): item
            for item in calls
            if isinstance(item, dict) and isinstance(item.get("root_call_id"), str)
        }
        claim_by_id = {
            item.get("discovery_job_id"): item
            for item in claims
            if isinstance(item, dict) and isinstance(item.get("discovery_job_id"), str)
        }
        self.require(len(inventory_by_id) == len(inventory), "duplicate or missing discovery IDs")
        self.require(len(action_by_sequence) == len(actions), "duplicate or missing action sequences")
        self.require(len(call_by_id) == len(calls), "duplicate or missing root-call IDs")
        self.require(len(claim_by_id) == len(claims), "duplicate or missing claim IDs")
        self.require(
            set(action_by_sequence) == set(range(1, EXPECTED_CANONICAL_ACTIONS + 1)),
            "action sequences are not exactly 1..122",
        )
        self.require(
            set(inventory_by_id) == set(claim_by_id),
            "inventory and claim discovery IDs differ",
        )

        prior_failures = {
            job_id
            for job_id, item in inventory_by_id.items()
            if item.get("status") == "failed"
        }
        self.require(prior_failures == EXPECTED_PRIOR_FAILURES, "prior failure set changed")
        for job_id in EXPECTED_PRIOR_FAILURES:
            item = inventory_by_id.get(job_id, {})
            self.require(bool(item.get("error")), f"{job_id}: prior failure has no error")

        action_owners: dict[int, list[str]] = {sequence: [] for sequence in action_by_sequence}
        for job_id, claim in claim_by_id.items():
            sequence = claim.get("source_action_sequence")
            action = action_by_sequence.get(sequence)
            self.require(action is not None, f"{job_id}: missing source action {sequence!r}")
            if action is None:
                continue
            action_owners[sequence].append(job_id)
            self.require(
                job_id in action.get("source_job_ids", []),
                f"{job_id}: action does not retain the claim as a source alias",
            )
            self.require(
                claim.get("source_semantic_request_sha256") == action.get("semantic_request_sha256"),
                f"{job_id}: source request hash differs from action hash",
            )
            self.require(
                claim.get("replacement_target_shard_id_authority") == "advisory_only",
                f"{job_id}: planned replacement shard is not marked advisory",
            )

        self.require(
            all(action_owners.values()),
            "one or more canonical actions have no discovery claim",
        )
        duplicate_owners = {sequence: owners for sequence, owners in action_owners.items() if len(owners) > 1}
        self.require(
            len(duplicate_owners) == 1 and sorted(flatten(duplicate_owners.values())) == sorted(
                EXPECTED_PRIOR_FAILURES - {
                    "20260719T132213Z-convoy-save-schema-serializer-no-pcode-9de6e1a6"
                }
            ),
            "the sole canonical duplicate is not the depth-1 0x140082cb0 retry",
        )

        supplemental = manifest.get("supplemental_exact10_actions")
        self.require(isinstance(supplemental, list), "supplemental action list is absent")
        supplemental_by_id = {
            item.get("supplemental_action_id"): item
            for item in (supplemental or [])
            if isinstance(item, dict) and isinstance(item.get("supplemental_action_id"), str)
        }
        self.require(
            {
                supplemental_id: item.get("address")
                for supplemental_id, item in supplemental_by_id.items()
            }
            == EXPECTED_SUPPLEMENTAL_ACTIONS,
            "supplemental action IDs/addresses differ",
        )
        for supplemental_id, item in supplemental_by_id.items():
            expected_depth, expected_pcode, expected_root = EXPECTED_SUPPLEMENTAL_SEMANTICS.get(
                supplemental_id, (None, None, None)
            )
            self.require(
                item.get("operation") == "export_functions"
                and item.get("depth") == expected_depth
                and item.get("pcode") is expected_pcode
                and item.get("root_call_id") == expected_root,
                f"{supplemental_id}: supplemental depth/pcode semantics differ",
            )
            if supplemental_id.startswith("SP-"):
                self.require(
                    set(item.get("promotion_consumers", [])) == EXPECTED_SUPPLEMENTAL_CONSUMERS,
                    f"{supplemental_id}: special-project consumers differ",
                )
            else:
                self.require(bool(item.get("promotion_consumers")), f"{supplemental_id}: consumers are absent")
            supplemental_preimage = semantic_preimage(
                "export_functions",
                {
                    "addresses": [item.get("address")],
                    "depth": item.get("depth"),
                    "pcode": item.get("pcode"),
                },
            )
            self.require(
                sha256_bytes(supplemental_preimage.encode("utf-8"))
                == item.get("semantic_request_sha256"),
                f"{supplemental_id}: supplemental semantic request hash differs",
            )

        call_action_sequences: list[int] = []
        call_supplemental_ids: list[str] = []
        for call_id, call in call_by_id.items():
            sequences = call.get("source_action_sequences")
            supplemental_ids = call.get("supplemental_action_ids")
            self.require(isinstance(sequences, list), f"{call_id}: source actions are not an array")
            self.require(
                isinstance(supplemental_ids, list),
                f"{call_id}: supplemental actions are not an array",
            )
            if isinstance(sequences, list):
                call_action_sequences.extend(sequences)
            if isinstance(supplemental_ids, list):
                call_supplemental_ids.extend(supplemental_ids)
            self.require(
                bool(sequences) != bool(supplemental_ids),
                f"{call_id}: call must contain canonical actions or supplemental actions, not both/neither",
            )
            self.require(
                call.get("target_shard_id_authority")
                == "advisory_only_broker_may_choose_another_ready_shard",
                f"{call_id}: target shard is not advisory",
            )
            self.require(
                isinstance(call.get("target_shard_id"), str)
                and SHARD_RE.fullmatch(call["target_shard_id"]) is not None,
                f"{call_id}: advisory shard ID is invalid",
            )
        self.require(
            sorted(call_action_sequences) == list(range(1, EXPECTED_CANONICAL_ACTIONS + 1)),
            "root calls do not partition canonical actions exactly once",
        )
        self.require(
            sorted(call_supplemental_ids) == sorted(EXPECTED_SUPPLEMENTAL_ACTIONS),
            "root calls do not partition supplemental actions exactly once",
        )
        failed_prejobs = reconciliation.get("failed_prejob_attempts")
        self.require(isinstance(failed_prejobs, list), "failed-attempt ledger is absent")
        failed_by_root = {
            item.get("root_call_id"): item for item in (failed_prejobs or []) if isinstance(item, dict)
        }
        self.require(
            failed_by_root.get("ADDR-01", {}).get("job_created") is False
            and failed_by_root.get("ADDR-01", {}).get("receipt_authority") is False,
            "oversized ADDR-01 pre-job rejection is not reconciled as non-authority",
        )
        for failure_root, (_, expected_job_id) in EXPECTED_FAILURE_ROOTS.items():
            entry = failed_by_root.get(failure_root, {})
            self.require(
                entry.get("job_created") is True
                and entry.get("receipt_authority") == "expected_failure_only"
                and entry.get("job_id") == expected_job_id,
                f"{failure_root}: deterministic exact-ten failure ledger differs",
            )
        return action_by_sequence, call_by_id, claim_by_id

    def audit_requests(
        self,
        manifest: dict[str, Any],
        reconciliation: dict[str, Any],
        action_by_sequence: dict[int, dict[str, Any]],
        call_by_id: dict[str, dict[str, Any]],
        claim_by_id: dict[str, dict[str, Any]],
    ) -> None:
        inventory_by_id = {
            item["discovery_job_id"]: item for item in manifest["discovery_inventory"]
        }
        supplemental_by_id = {
            item["supplemental_action_id"]: item
            for item in manifest.get("supplemental_exact10_actions", [])
        }
        for sequence, action in action_by_sequence.items():
            context = f"action {sequence}"
            preimage = self.guarded(
                context,
                lambda action=action: semantic_preimage(action["operation"], action["request"]),
            )
            if preimage is not None:
                expected = sha256_bytes(preimage.encode("utf-8"))
                self.require(
                    expected == action.get("semantic_request_sha256"),
                    f"{context}: semantic request SHA-256 mismatch",
                )
            self.require(
                action.get("target_shard_id_authority") == "advisory_only",
                f"{context}: target shard is not advisory",
            )

        for call_id, call in call_by_id.items():
            context = f"root call {call_id}"
            preimage = call.get("request_hash_preimage")
            expected_hash = call.get("expected_request_sha256")
            self.require(isinstance(preimage, str), f"{context}: request preimage missing")
            self.require(
                isinstance(expected_hash, str) and SHA256_RE.fullmatch(expected_hash) is not None,
                f"{context}: expected request hash invalid",
            )
            if isinstance(preimage, str) and isinstance(expected_hash, str):
                self.require(
                    sha256_bytes(preimage.encode("utf-8")) == expected_hash,
                    f"{context}: stored request preimage hash mismatch",
                )
                parsed = self.guarded(context, lambda: json.loads(preimage))
                expected_semantic = self.guarded(
                    context,
                    lambda call=call: {
                        "operation": call["operation"],
                        "request": semantic_request(call["operation"], call["request"]),
                    },
                )
                if parsed is not None and expected_semantic is not None:
                    self.require(parsed == expected_semantic, f"{context}: preimage/request mismatch")

            source_actions = [
                action_by_sequence[sequence]
                for sequence in call.get("source_action_sequences", [])
                if sequence in action_by_sequence
            ]
            supplemental_actions = [
                supplemental_by_id[supplemental_id]
                for supplemental_id in call.get("supplemental_action_ids", [])
                if supplemental_id in supplemental_by_id
            ]
            call_addresses = call.get("request", {}).get("addresses", [])
            address_count = len(call_addresses)
            if call.get("operation") == "export_functions":
                self.require(
                    address_count <= EXPORT_FUNCTIONS_MAX_ADDRESSES,
                    f"{context}: exceeds export_functions 32-address broker limit",
                )
            elif call.get("operation") == "dump_data":
                self.require(
                    address_count <= DUMP_DATA_MAX_ADDRESSES,
                    f"{context}: exceeds dump_data 128-address broker limit",
                )
            if call.get("operation") in {"export_functions", "dump_data"}:
                if supplemental_actions:
                    # Supplemental root calls deliberately preserve the manifest's
                    # action order because address-array order is part of the
                    # canonical broker request hash.  Re-sorting here would reject
                    # an authenticated request even when its membership is exact.
                    union = [item["address"] for item in supplemental_actions]
                    self.require(
                        len(union) == len(set(union)),
                        f"{context}: supplemental address union contains duplicates",
                    )
                    expected_roots = {
                        EXPECTED_SUPPLEMENTAL_SEMANTICS[item["supplemental_action_id"]][2]
                        for item in supplemental_actions
                    }
                    self.require(
                        expected_roots == {call_id}
                        and call.get("operation") == "export_functions"
                        and all(
                            item.get("depth") == call.get("request", {}).get("depth")
                            and item.get("pcode") == call.get("request", {}).get("pcode")
                            for item in supplemental_actions
                        ),
                        f"{context}: supplemental root semantics differ",
                    )
                else:
                    union = sorted(
                        {
                            address
                            for action in source_actions
                            for address in action.get("request", {}).get("addresses", [])
                        }
                    )
                self.require(
                    len(call_addresses) == len(set(call_addresses))
                    and len(call_addresses) == len(union)
                    and set(call_addresses) == set(union),
                    f"{context}: consolidated address union is not exact",
                )
                for action in source_actions:
                    self.require(
                        action.get("operation") == call.get("operation"),
                        f"{context}: operation-incompatible source action",
                    )
                    if call.get("operation") == "export_functions":
                        self.require(
                            action["request"].get("depth") == call["request"].get("depth")
                            and action["request"].get("pcode") == call["request"].get("pcode"),
                            f"{context}: depth/pcode-incompatible source action",
                        )
            else:
                self.require(len(source_actions) == 1, f"{context}: search request was consolidated")
                if len(source_actions) == 1:
                    self.require(
                        semantic_request("search_strings", source_actions[0]["request"])
                        == semantic_request("search_strings", call["request"]),
                        f"{context}: search terms changed",
                    )

        for job_id, claim in claim_by_id.items():
            discovery = inventory_by_id[job_id]
            action = action_by_sequence.get(claim.get("source_action_sequence"))
            call = call_by_id.get(claim.get("replacement_root_call_id"))
            if action is None or call is None:
                continue
            discovery_semantic = self.guarded(
                job_id,
                lambda: semantic_request(discovery["operation"], discovery["exact_request"]),
            )
            action_semantic = self.guarded(
                job_id,
                lambda: semantic_request(action["operation"], action["request"]),
            )
            if discovery_semantic is not None and action_semantic is not None:
                self.require(
                    discovery["operation"] == action["operation"]
                    and discovery_semantic == action_semantic,
                    f"{job_id}: discovery-to-action request changed",
                )
            membership = claim.get("request_membership")
            self.require(isinstance(membership, dict), f"{job_id}: request membership missing")
            if isinstance(membership, dict) and discovery_semantic is not None:
                expected_membership = dict(discovery_semantic)
                if discovery["operation"] == "dump_data":
                    expected_membership.update({"depth": None, "pcode": None})
                self.require(
                    membership == expected_membership,
                    f"{job_id}: claim membership differs from discovery request",
                )
            self.require(
                claim.get("expected_root_request_sha256") == call.get("expected_request_sha256"),
                f"{job_id}: claim-to-root request hash mismatch",
            )

    def audit_pool_receipt(self, receipt_path: Path, expected_sha256: str) -> dict[str, Any] | None:
        context = f"pool receipt {receipt_path}"
        self.require(receipt_path.is_file(), f"{context}: file is absent")
        if not receipt_path.is_file():
            return None
        actual_sha256 = sha256_file(receipt_path)
        self.require(actual_sha256 == expected_sha256, f"{context}: file hash mismatch")
        receipt = self.guarded(context, lambda: load_json(receipt_path))
        if receipt is None:
            return None
        self.require(
            receipt.get("schema") == "hoi4-ghidra-pool-status-receipt-v1",
            f"{context}: schema mismatch",
        )
        self.require(receipt.get("binary_id") == EXPECTED_BINARY_ID, f"{context}: binary mismatch")
        self.require(receipt.get("image_base") == EXPECTED_IMAGE_BASE, f"{context}: image base mismatch")
        self.require(receipt.get("sha256") == EXPECTED_BINARY_SHA256, f"{context}: binary hash mismatch")
        self.require(receipt.get("shard_count") == 10, f"{context}: shard count is not 10")
        ready = receipt.get("ready_shard_ids")
        self.require(
            isinstance(ready, list)
            and len(ready) == 10
            and set(ready) == set(EXPECTED_READY_SHARDS),
            f"{context}: ready shard IDs are not exactly shard-01..shard-10",
        )
        self.require(receipt.get("read_only") is True, f"{context}: read_only is not true")
        self.require(receipt.get("analysis_enabled") is False, f"{context}: analysis is not disabled")
        self.require(
            receipt.get("per_shard_serialization") is True,
            f"{context}: per-shard serialization is not true",
        )
        self.require(receipt.get("persistent_error") is None, f"{context}: persistent error is present")
        identity_hash = receipt.get("identity_manifest_sha256")
        self.require(
            isinstance(identity_hash, str) and SHA256_RE.fullmatch(identity_hash) is not None,
            f"{context}: identity manifest hash is invalid",
        )
        return receipt

    def audit_receipts(
        self,
        reconciliation: dict[str, Any],
        call_by_id: dict[str, dict[str, Any]],
        job_root: Path,
        evidence_root: Path,
    ) -> None:
        pool_cache: dict[tuple[Path, str], dict[str, Any] | None] = {}
        for call_id, call in call_by_id.items():
            context = f"root call {call_id}"
            replacement = call.get("replacement_result")
            self.require(isinstance(replacement, dict), f"{context}: replacement_result missing")
            if not isinstance(replacement, dict):
                continue
            self.require(
                replacement.get("planned_target_shard_must_match_actual") is False,
                f"{context}: planned shard was made authoritative",
            )
            if call_id in EXPECTED_FAILURE_ROOTS:
                expected_sequence, expected_job_id = EXPECTED_FAILURE_ROOTS[call_id]
                known = call.get("known_failure_receipt")
                self.require(isinstance(known, dict), f"{context}: known-failure receipt missing")
                if not isinstance(known, dict):
                    continue
                self.require(
                    replacement.get("status") == "expected_failure_closed_no_retry"
                    and replacement.get("expected_failure") is True
                    and replacement.get("retry_required") is False
                    and replacement.get("receipt_authoritative") is False,
                    f"{context}: expected-failure disposition differs",
                )
                self.require(
                    call.get("source_action_sequences") == [expected_sequence]
                    and known.get("actual_job_id") == expected_job_id
                    and known.get("receipt_authoritative_for_expected_failure_only") is True,
                    f"{context}: expected-failure source/job binding differs",
                )
                failed_job_path = self.guarded(
                    context,
                    lambda: resolve_under(job_root, known.get("job_json_path"), "failed job JSON"),
                )
                if failed_job_path is None:
                    continue
                self.require(
                    failed_job_path == (job_root / expected_job_id / "job.json").resolve()
                    and failed_job_path.is_file(),
                    f"{context}: failed job path differs or is absent",
                )
                if not failed_job_path.is_file():
                    continue
                self.require(
                    sha256_file(failed_job_path) == known.get("job_json_sha256"),
                    f"{context}: failed job hash mismatch",
                )
                failed_job = self.guarded(context, lambda: load_json(failed_job_path))
                if failed_job is None:
                    continue
                failed_preimage = self.guarded(
                    context,
                    lambda: semantic_preimage(failed_job["operation"], failed_job["request"]),
                )
                if failed_preimage is not None:
                    self.require(
                        sha256_bytes(failed_preimage.encode("utf-8"))
                        == known.get("actual_batched_request_sha256"),
                        f"{context}: failed batched request hash mismatch",
                    )
                membership = known.get("membership") or {}
                failed_addresses = (failed_job.get("request") or {}).get("addresses") or []
                self.require(
                    failed_job.get("status") == "failed"
                    and bool(failed_job.get("error"))
                    and "decompile failed for 140082cb0" in failed_job.get("error", "")
                    and known.get("error") == failed_job.get("error")
                    and (failed_job.get("shard") or {}).get("id") == known.get("actual_shard_id")
                    and membership.get("addresses") == ["0x140082cb0"]
                    and "0x140082cb0" in failed_addresses
                    and membership.get("depth") == (failed_job.get("request") or {}).get("depth")
                    and membership.get("pcode") == (failed_job.get("request") or {}).get("pcode"),
                    f"{context}: deterministic decompile-failure membership differs",
                )
                self.require(
                    failed_job.get("identity_contract")
                    == {
                        "binary_id": EXPECTED_BINARY_ID,
                        "image_base": EXPECTED_IMAGE_BASE,
                        "sha256": EXPECTED_BINARY_SHA256,
                    },
                    f"{context}: failed job identity contract mismatch",
                )
                self.failed_replacements.append(call_id)
                self.actual_results[call_id] = known
                continue
            self.require(
                replacement.get("receipt_authoritative") is True,
                f"{context}: receipt is not authoritative",
            )

            actual_job_id = replacement.get("actual_job_id")
            actual_shard_id = replacement.get("actual_shard_id")
            self.require(isinstance(actual_job_id, str) and actual_job_id, f"{context}: actual job ID missing")
            self.require(
                isinstance(actual_shard_id, str) and SHARD_RE.fullmatch(actual_shard_id) is not None,
                f"{context}: actual shard ID invalid",
            )
            if not isinstance(actual_job_id, str) or not actual_job_id:
                continue

            expected_job_path = (job_root / actual_job_id / "job.json").resolve()
            job_path = self.guarded(
                context,
                lambda: resolve_under(job_root, replacement.get("job_json_path"), "job JSON"),
            )
            if job_path is None:
                continue
            self.require(job_path == expected_job_path, f"{context}: job path/job ID mismatch")
            self.require(job_path.is_file(), f"{context}: job JSON is absent")
            if not job_path.is_file():
                continue
            expected_job_sha = replacement.get("job_json_sha256")
            self.require(
                isinstance(expected_job_sha, str) and SHA256_RE.fullmatch(expected_job_sha) is not None,
                f"{context}: job JSON SHA-256 missing or invalid",
            )
            if isinstance(expected_job_sha, str):
                self.require(sha256_file(job_path) == expected_job_sha, f"{context}: job JSON hash mismatch")
            job = self.guarded(context, lambda: load_json(job_path))
            if job is None:
                continue

            self.require(job.get("job_id") == actual_job_id, f"{context}: returned job ID mismatch")
            self.require(job.get("operation") == call.get("operation"), f"{context}: operation mismatch")
            job_shard = (job.get("result") or {}).get("shard", {}).get("id") or (job.get("shard") or {}).get("id")
            self.require(job_shard == actual_shard_id, f"{context}: receipt/job shard mismatch")
            identity = job.get("identity_contract")
            self.require(
                identity
                == {
                    "binary_id": EXPECTED_BINARY_ID,
                    "image_base": EXPECTED_IMAGE_BASE,
                    "sha256": EXPECTED_BINARY_SHA256,
                },
                f"{context}: job identity contract mismatch",
            )

            actual_preimage = self.guarded(
                context,
                lambda: semantic_preimage(job["operation"], job["request"]),
            )
            expected_request_hash = call.get("expected_request_sha256")
            if actual_preimage is not None:
                actual_request_hash = sha256_bytes(actual_preimage.encode("utf-8"))
                self.require(
                    actual_request_hash == expected_request_hash,
                    f"{context}: actual job request differs from normalized root request",
                )
                self.require(
                    replacement.get("actual_request_sha256") == actual_request_hash,
                    f"{context}: receipt request hash mismatch",
                )
                self.require(replacement.get("request_hash_match") is True, f"{context}: request_hash_match is not true")

            pool_pointer = replacement.get("pool_status_receipt")
            self.require(isinstance(pool_pointer, dict), f"{context}: pool receipt pointer missing")
            pool_receipt = None
            if isinstance(pool_pointer, dict):
                pool_sha = pool_pointer.get("status_snapshot_sha256")
                self.require(
                    isinstance(pool_sha, str) and SHA256_RE.fullmatch(pool_sha) is not None,
                    f"{context}: pool receipt SHA-256 invalid",
                )
                pool_path = self.guarded(
                    context,
                    lambda: resolve_under(
                        evidence_root,
                        pool_pointer.get("status_snapshot_path"),
                        "pool receipt",
                    ),
                )
                if pool_path is not None and isinstance(pool_sha, str):
                    cache_key = (pool_path, pool_sha)
                    if cache_key not in pool_cache:
                        pool_cache[cache_key] = self.audit_pool_receipt(pool_path, pool_sha)
                    pool_receipt = pool_cache[cache_key]
            if pool_receipt is not None:
                self.require(
                    actual_shard_id in pool_receipt.get("ready_shard_ids", []),
                    f"{context}: actual shard was not ready in the exact-10 receipt",
                )
                self.require(
                    replacement.get("identity_manifest_sha256")
                    == pool_receipt.get("identity_manifest_sha256"),
                    f"{context}: identity manifest hash differs from pool receipt",
                )
                self.require(
                    pool_pointer.get("observed_shard_count") == 10,
                    f"{context}: observed shard count is not 10",
                )
                self.require(
                    set(pool_pointer.get("observed_ready_shards") or []) == set(EXPECTED_READY_SHARDS),
                    f"{context}: observed ready shards mismatch",
                )

            status = job.get("status")
            self.require(status in {"complete", "failed"}, f"{context}: job status is not terminal")
            self.require(replacement.get("status") == status, f"{context}: receipt/job status mismatch")
            if status == "failed":
                self.failed_replacements.append(call_id)
                self.require(bool(job.get("error")), f"{context}: failed job has no error")
                self.require(
                    replacement.get("artifact") is None and replacement.get("artifact_sha256") is None,
                    f"{context}: failed receipt claims an artifact",
                )
                self.errors.append(f"{context}: authoritative replacement failed; promotion remains closed")
                continue
            if status != "complete":
                continue

            result = job.get("result")
            self.require(isinstance(result, dict), f"{context}: complete job result missing")
            if not isinstance(result, dict):
                continue
            if call.get("operation") == "export_functions":
                job_artifact = result.get("manifest")
                job_artifact_sha = result.get("manifest_sha256")
            else:
                job_artifact = result.get("artifact")
                job_artifact_sha = result.get("artifact_sha256")
            self.require(replacement.get("artifact") == job_artifact, f"{context}: artifact path mismatch")
            self.require(
                replacement.get("artifact_sha256") == job_artifact_sha,
                f"{context}: artifact hash differs from job result",
            )
            self.require(
                isinstance(job_artifact_sha, str)
                and SHA256_RE.fullmatch(job_artifact_sha) is not None,
                f"{context}: artifact SHA-256 is invalid",
            )
            result_identity_hash = result.get("identity_manifest_sha256")
            if result_identity_hash is not None:
                self.require(
                    result_identity_hash == replacement.get("identity_manifest_sha256"),
                    f"{context}: job/pool identity manifest hashes differ",
                )
            artifact_path = self.guarded(
                context,
                lambda: resolve_under(job_root, job_artifact, "artifact"),
            )
            if artifact_path is not None:
                self.require(artifact_path.is_file(), f"{context}: artifact is absent")
                if artifact_path.is_file() and isinstance(job_artifact_sha, str):
                    self.require(
                        sha256_file(artifact_path) == job_artifact_sha,
                        f"{context}: artifact file hash mismatch",
                    )
            self.actual_results[call_id] = replacement

        self.require(
            len(self.actual_results) == EXPECTED_ROOT_CALLS,
            "not all 43 successful roots and two expected-failure roots have terminal receipts",
        )

    def audit_consumers(
        self,
        manifest: dict[str, Any],
        reconciliation: dict[str, Any],
        call_by_id: dict[str, dict[str, Any]],
        claim_by_id: dict[str, dict[str, Any]],
        repo_root: Path,
    ) -> None:
        inventory_by_id = {
            item["discovery_job_id"]: item for item in manifest["discovery_inventory"]
        }
        authority_names = {
            "extractor/documents/WAVE2_INVALID_20_SHARD_REEXPORT_MANIFEST_20260720.json",
            "extractor/documents/WAVE2_10_SHARD_REEXPORT_RECONCILIATION_20260720.json",
            "extractor/documents/WAVE2_INVALID_20_SHARD_REEXPORT_REPORT_20260720.md",
        }
        external_claims = 0
        external_paths: set[str] = set()
        calls_with_external_consumers: set[str] = set()
        for job_id, claim in claim_by_id.items():
            discovery_consumers = sorted(inventory_by_id[job_id].get("promotion_consumers", []))
            claim_consumers = sorted(claim.get("external_promotion_consumers", []))
            self.require(
                discovery_consumers == claim_consumers,
                f"{job_id}: discovery/claim consumer mapping differs",
            )
            effective = set(claim.get("effective_repo_consumers", []))
            self.require(
                authority_names.issubset(effective),
                f"{job_id}: quarantine authority consumers are incomplete",
            )
            if not claim_consumers:
                self.require(
                    claim.get("consumer_class") == "quarantine_ledger_only",
                    f"{job_id}: consumer class mismatch",
                )
                continue

            external_claims += 1
            external_paths.update(claim_consumers)
            call_id = claim.get("replacement_root_call_id")
            calls_with_external_consumers.add(call_id)
            replacement = self.actual_results.get(call_id)
            self.require(replacement is not None, f"{job_id}: consumer has no terminal receipt")
            if replacement is None:
                continue
            if call_id in EXPECTED_FAILURE_ROOTS:
                self.require(
                    claim.get("consumer_rewrite_contract", {}).get("rewrite_required") is False
                    and claim.get("consumer_rewrite_contract", {}).get("expected_failure_receipt_source"),
                    f"{job_id}: deterministic failure was incorrectly made a consumer rewrite",
                )
                continue
            for relative_path in claim_consumers:
                consumer_path = self.guarded(
                    job_id,
                    lambda relative_path=relative_path: resolve_under(
                        repo_root, relative_path, "consumer"
                    ),
                )
                if consumer_path is None:
                    continue
                self.require(consumer_path.is_file(), f"{job_id}: consumer is absent: {relative_path}")

        supplemental = manifest.get("supplemental_exact10_actions", [])
        supplemental_by_root: dict[str, list[dict[str, Any]]] = {}
        for item in supplemental:
            supplemental_by_root.setdefault(item.get("root_call_id"), []).append(item)
        supplemental_consumers: set[str] = set()
        for call_id, items in supplemental_by_root.items():
            root_consumers = {
                path for item in items for path in item.get("promotion_consumers", [])
            }
            supplemental_consumers.update(root_consumers)
            external_paths.update(root_consumers)
            calls_with_external_consumers.add(call_id)
            supplemental_result = self.actual_results.get(call_id)
            self.require(supplemental_result is not None, f"{call_id}: no complete receipt")
            if supplemental_result is None:
                continue
            for relative_path in root_consumers:
                consumer_path = self.guarded(
                    call_id,
                    lambda relative_path=relative_path: resolve_under(
                        repo_root, relative_path, "supplemental consumer"
                    ),
                )
                if consumer_path is None:
                    continue
                self.require(consumer_path.is_file(), f"{call_id}: consumer is absent: {relative_path}")

        audit = reconciliation.get("promotion_consumer_audit") or {}
        self.require(external_claims == 64, "external consumer claim count is not 64")
        self.require(len(supplemental) == 24, "supplemental consumer action count is not 24")
        self.require(audit.get("external_claims") == external_claims, "consumer audit claim count differs")
        self.require(
            audit.get("supplemental_external_actions") == len(supplemental),
            "consumer audit supplemental count differs",
        )
        self.require(
            audit.get("distinct_external_consumer_paths") == len(external_paths),
            "consumer audit path count differs",
        )
        self.require(
            audit.get("root_calls_with_external_consumers") == len(calls_with_external_consumers),
            "consumer audit root-call count differs",
        )
        ledger_authority = audit.get("consumer_rewrite_authority") or {}
        self.require(
            audit.get("status") == "central_consumer_rewrite_ledger_complete"
            and audit.get("open_receipts") == 0
            and audit.get("open_consumer_rewrites") == 0
            and ledger_authority.get("central_binding_is_authority") is True
            and ledger_authority.get("inline_consumer_duplication_required") is True,
            "central consumer-rewrite disposition is incomplete",
        )
        ledger_path = self.guarded(
            "consumer rewrite ledger",
            lambda: resolve_under(repo_root, ledger_authority.get("json_path"), "consumer rewrite ledger"),
        )
        ledger = None
        if ledger_path is not None:
            self.require(ledger_path.is_file(), "consumer rewrite ledger is absent")
            if ledger_path.is_file():
                self.require(
                    sha256_file(ledger_path) == ledger_authority.get("json_sha256"),
                    "consumer rewrite ledger hash mismatch",
                )
                ledger = self.guarded("consumer rewrite ledger", lambda: load_json(ledger_path))
        report_path = self.guarded(
            "consumer rewrite report",
            lambda: resolve_under(repo_root, ledger_authority.get("report_path"), "consumer rewrite report"),
        )
        if report_path is not None:
            self.require(
                report_path.is_file()
                and sha256_file(report_path) == ledger_authority.get("report_sha256"),
                "consumer rewrite report is absent or hash-mismatched",
            )
        if isinstance(ledger, dict):
            self.require(
                ledger.get("schema") == "hoi4-wave2-exact10-consumer-rewrite-ledger-v1",
                "consumer rewrite ledger schema differs",
            )
            ledger_rows = ledger.get("consumers") or []
            rows_by_path = {
                row.get("consumer_path"): row
                for row in ledger_rows
                if isinstance(row, dict) and isinstance(row.get("consumer_path"), str)
            }
            self.require(
                set(rows_by_path) == external_paths
                and len(rows_by_path) == len(ledger_rows),
                "consumer rewrite ledger does not cover every mapped consumer exactly once",
            )
            dispositions = {"enabled", "pinned", "deliberately_still_false"}
            for relative_path, row in rows_by_path.items():
                context = f"consumer ledger {relative_path}"
                consumer_path = self.guarded(
                    context,
                    lambda relative_path=relative_path: resolve_under(repo_root, relative_path, "consumer"),
                )
                if consumer_path is None:
                    continue
                self.require(
                    consumer_path.is_file()
                    and sha256_file(consumer_path) == row.get("consumer_sha256_at_audit"),
                    f"{context}: consumer changed after the static audit",
                )
                self.require(
                    row.get("disposition") in dispositions and bool(row.get("reason")),
                    f"{context}: disposition or reason is invalid",
                )
                expected_roots = {
                    call_id
                    for call_id, call in call_by_id.items()
                    if relative_path in call.get("external_promotion_consumers", [])
                }
                bindings = row.get("root_bindings") or []
                binding_by_root = {
                    item.get("root_call_id"): item
                    for item in bindings
                    if isinstance(item, dict) and isinstance(item.get("root_call_id"), str)
                }
                self.require(
                    set(binding_by_root) == expected_roots
                    and len(binding_by_root) == len(bindings),
                    f"{context}: root bindings differ from reconciliation",
                )
                for root_id, binding in binding_by_root.items():
                    self.require(
                        binding.get("inline_citation", {}).get("complete") is True
                        and not binding.get("inline_citation", {}).get("missing_fields"),
                        f"{context}: {root_id} inline exact-ten citation is incomplete",
                    )
                    root = call_by_id[root_id]
                    if root_id in EXPECTED_FAILURE_ROOTS:
                        receipt = root.get("known_failure_receipt") or {}
                        self.require(
                            binding.get("binding_kind") == "expected_failure"
                            and binding.get("actual_job_id") == receipt.get("actual_job_id")
                            and binding.get("job_json_sha256") == receipt.get("job_json_sha256")
                            and binding.get("actual_request_sha256") == receipt.get("actual_batched_request_sha256")
                            and binding.get("isolated_expected_request_sha256") == receipt.get("isolated_expected_request_sha256")
                            and binding.get("identity_manifest_sha256") == receipt.get("identity_manifest_sha256")
                            and binding.get("pool_status_sha256") == (receipt.get("pool_status_receipt") or {}).get("status_snapshot_sha256")
                            and binding.get("artifact") is None
                            and binding.get("artifact_sha256") is None,
                            f"{context}: {root_id} expected-failure binding differs",
                        )
                    else:
                        receipt = root.get("replacement_result") or {}
                        self.require(
                            binding.get("binding_kind") == "successful_artifact"
                            and binding.get("actual_job_id") == receipt.get("actual_job_id")
                            and binding.get("actual_shard_id") == receipt.get("actual_shard_id")
                            and binding.get("job_json_sha256") == receipt.get("job_json_sha256")
                            and binding.get("actual_request_sha256") == receipt.get("actual_request_sha256")
                            and binding.get("identity_manifest_sha256") == receipt.get("identity_manifest_sha256")
                            and binding.get("artifact") == receipt.get("artifact")
                            and binding.get("artifact_sha256") == receipt.get("artifact_sha256")
                            and binding.get("pool_status_sha256") == (receipt.get("pool_status_receipt") or {}).get("status_snapshot_sha256"),
                            f"{context}: {root_id} successful binding differs",
                        )
        for call_id, call in call_by_id.items():
            if call_id in supplemental_by_root:
                expected = sorted(
                    {
                        path
                        for item in supplemental_by_root[call_id]
                        for path in item.get("promotion_consumers", [])
                    }
                )
            else:
                expected = sorted(
                    {
                        path
                        for claim in claim_by_id.values()
                        if claim.get("replacement_root_call_id") == call_id
                        for path in claim.get("external_promotion_consumers", [])
                    }
                )
            self.require(
                sorted(call.get("external_promotion_consumers", [])) == expected,
                f"{call_id}: root-call consumer union differs from claims",
            )


def parse_args() -> argparse.Namespace:
    here = Path(__file__).resolve().parent
    repo_root = here.parent.parent
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--manifest",
        type=Path,
        default=here / "WAVE2_INVALID_20_SHARD_REEXPORT_MANIFEST_20260720.json",
    )
    parser.add_argument(
        "--reconciliation",
        type=Path,
        default=here / "WAVE2_10_SHARD_REEXPORT_RECONCILIATION_20260720.json",
    )
    parser.add_argument("--repo-root", type=Path, default=repo_root)
    parser.add_argument(
        "--job-root",
        type=Path,
        default=Path(r"C:\hoi4_evidence\ghidra_agent_exports"),
    )
    parser.add_argument(
        "--evidence-root",
        type=Path,
        default=Path(r"C:\hoi4_evidence"),
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    gate = Gate()
    try:
        manifest = load_json(args.manifest.resolve())
        reconciliation = load_json(args.reconciliation.resolve())
    except Exception as error:
        print(f"FAIL: authority input could not be loaded: {error}", file=sys.stderr)
        return 1

    action_by_sequence, call_by_id, claim_by_id = gate.audit_structure(
        manifest, reconciliation
    )
    if action_by_sequence and call_by_id and claim_by_id:
        gate.audit_requests(
            manifest,
            reconciliation,
            action_by_sequence,
            call_by_id,
            claim_by_id,
        )
        gate.audit_receipts(
            reconciliation,
            call_by_id,
            args.job_root.resolve(),
            args.evidence_root.resolve(),
        )
        gate.audit_consumers(
            manifest,
            reconciliation,
            call_by_id,
            claim_by_id,
            args.repo_root.resolve(),
        )

    if gate.errors:
        print(f"FAIL: {len(gate.errors)} exact-10 promotion gate error(s)", file=sys.stderr)
        for error in gate.errors:
            print(f"- {error}", file=sys.stderr)
        return 1
    print("PASS: exact-10 replacement receipts and all promotion consumers reconcile")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
