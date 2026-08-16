#!/usr/bin/env python3
"""Fail-closed passive-recorder sink reconciliation.

The verifier grades immutable output files only.  A successful production run
must have been attached to a user-owned the game process, must declare zero client
commands/state writes, and must account for every scheduled domain in each
FRAME_END as either an accepted PERF outcome or an authenticated H4DG outcome.
Evidence-only sink fault receipts remain non-production regression inputs.
"""

from __future__ import annotations

import argparse
import hashlib
import hmac
import json
import math
import os
import re
import struct
import sys
from pathlib import Path
from typing import Any

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import gap_ledger  # noqa: E402  (path is set immediately above)


HEX64 = set("0123456789abcdef")
EXACT_A729_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
COLLECTOR_BUILD_PROFILE_ID = "vanilla_latest_2026_06_30"
CANONICAL_COMPACT_FAILURE_MASK = 0x13CF
FORBIDDEN_RECORDER_ARGUMENTS = {
    "--early-launch-exe",
    "--early-launch-arg",
    "--early-launch-keep-window",
    "--hour-latch",
}
ALLOWED_GAP_PARTS = {"state", "event", "both"}
# The COMPLETE H4DG reason vocabulary, and it must stay complete.
#
# It stopped at `worker_panic` for as long as those were the only five reasons,
# and then three were assigned without it: a manifest carrying
# `source_unavailable`, `reader_silent` or `hour_label_mismatch` was reported
# `domain_gap_N_reason_invalid` -- a capture failed for naming a loss the
# producer is entitled to name. `hour_label_mismatch` is the one that made it
# reachable in bulk, since d81's split now publishes it directly.
#
# Kept honest from the other side: compact_contract's
# `verify_sink_reconciliation_admits_exactly_the_contract_gap_reasons` rebuilds
# this set from `DomainGapReason::ALL` and requires an exact match, so a reason
# added to the wire without being added here fails that test rather than a
# capture.
ALLOWED_GAP_REASONS = {
    "torn",
    "overrun",
    "backpressure",
    "clock_unreadable",
    "worker_panic",
    "source_unavailable",
    "reader_silent",
    "hour_label_mismatch",
}
PRODUCTION_DOMAIN_STATUSES = {
    "existing",
    "production_complete",
    "production_complete_base",
}


def reject_duplicate_members(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    value: dict[str, Any] = {}
    for key, child in pairs:
        if key in value:
            raise ValueError(f"duplicate JSON member: {key}")
        value[key] = child
    return value


def load_json(path: Path) -> Any:
    return json.loads(
        path.read_bytes().decode("utf-8"),
        object_pairs_hook=reject_duplicate_members,
    )


def load_object(path: Path) -> dict[str, Any]:
    value = load_json(path)
    if not isinstance(value, dict):
        raise ValueError(f"{path} is not a JSON object")
    return value


def decode_object(raw: bytes, label: str) -> dict[str, Any]:
    value = json.loads(
        raw.decode("utf-8"), object_pairs_hook=reject_duplicate_members
    )
    if not isinstance(value, dict):
        raise ValueError(f"{label} is not a JSON object")
    return value


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def is_hex64(value: Any) -> bool:
    return isinstance(value, str) and len(value) == 64 and set(value) <= HEX64


def is_int(value: Any) -> bool:
    return type(value) is int


def is_finite_number(value: Any) -> bool:
    return type(value) is int or (type(value) is float and math.isfinite(value))


def require(condition: bool, problem: str, problems: list[str]) -> None:
    if not condition:
        problems.append(problem)


def nested(root: dict[str, Any], *keys: str) -> Any:
    value: Any = root
    for key in keys:
        if not isinstance(value, dict):
            return None
        value = value.get(key)
    return value


def utf8_sha256(value: str) -> str:
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def labelled_preimage(label: str) -> bytes:
    raw = label.encode("utf-8")
    return struct.pack("<H", len(raw)) + raw + struct.pack("<H", 1)


def canonical_uuid_bytes(value: Any) -> bytes | None:
    if (
        not isinstance(value, str)
        or len(value) != 36
        or any(value[offset] != "-" for offset in (8, 13, 18, 23))
    ):
        return None
    compact = value.replace("-", "")
    if len(compact) != 32 or any(char not in "0123456789abcdef" for char in compact):
        return None
    decoded = bytes.fromhex(compact)
    return decoded if decoded != bytes(16) else None


def capture_ready_payload_sha256(ready: dict[str, Any], pipe_token: str) -> str | None:
    digest_fields = (
        "target_executable_sha256",
        "collector_dll_sha256",
        "hook_allowlist_sha256",
        "hook_outcomes_sha256",
        "validated_domain_roster_sha256",
        "clock_witness_sha256",
        "scheduler_config_sha256",
        "output_pipe_identity_sha256",
        "pipe_token_sha256",
        "zero_mutation_policy_sha256",
        "session_id_sha256",
        "capture_id_sha256",
        "control_sha256",
    )
    if not all(is_hex64(ready.get(field)) for field in digest_fields):
        return None
    try:
        body = (
            b"H4RD"
            + struct.pack("<HHI", 2, 512, 512)
            + struct.pack(
                "<IIIQIIIHHHHiQ",
                ready["readiness_flags"],
                ready["pid"],
                # Offsets 20 and 50 were both reserved-must-be-zero once and both
                # became real fields (the collector's compile-time feature word,
                # then its install-freeze count). This re-derivation kept the
                # constants, so it disagreed with every receipt written after
                # either change.
                #
                # Offset 20 keeps a zero default because a pre-declaration
                # receipt legitimately carries no such property and really did
                # authenticate a zero there -- the PowerShell mirror's
                # Get-PassiveCaptureReadyFeatureBits says the same. Offset 50 has
                # NO default: `ready["install_freeze_count"]` raises KeyError,
                # this function returns None, and the caller reports
                # `capture_ready_authenticated_payload_or_ack_digest_invalid`
                # instead of quietly rebuilding the wrong bytes. That silent zero
                # is what refused six captures with nothing but two hashes to
                # look at.
                ready.get("collector_feature_bits", 0),
                ready["process_creation_filetime_utc"],
                ready["requested_duration_ms"],
                ready["validated_domain_count"],
                ready["scheduler_queue_capacity"],
                ready["scheduler_worker_count"],
                ready["allowed_hook_count"],
                ready["active_hook_count"],
                ready["install_freeze_count"],
                ready["clock_hour_raw"],
                ready["clock_observation_ordinal"],
            )
            + b"".join(bytes.fromhex(ready[field]) for field in digest_fields)
        )
    except (KeyError, TypeError, ValueError, struct.error):
        return None
    if len(body) != 480:
        return None
    authenticated = body + hmac.new(
        pipe_token.encode("utf-8"), body, hashlib.sha256
    ).digest()
    return hashlib.sha256(authenticated).hexdigest()


def capture_ready_ack_sha256(ready: dict[str, Any], pipe_token: str) -> str | None:
    digest_fields = (
        "ready_payload_sha256",
        "public_receipt_core_sha256",
        "session_id_sha256",
    )
    if not all(is_hex64(ready.get(field)) for field in digest_fields):
        return None
    try:
        body = (
            b"H4RA"
            + struct.pack("<HHI", 2, 168, 168)
            + struct.pack(
                "<IIIQII",
                1,
                ready["pid"],
                0,
                ready["process_creation_filetime_utc"],
                ready["requested_duration_ms"],
                ready["validated_domain_count"],
            )
            + b"".join(bytes.fromhex(ready[field]) for field in digest_fields)
        )
    except (KeyError, TypeError, ValueError, struct.error):
        return None
    if len(body) != 136:
        return None
    authenticated = body + hmac.new(
        pipe_token.encode("utf-8"), body, hashlib.sha256
    ).digest()
    return hashlib.sha256(authenticated).hexdigest()


def canonical_hook_authority(problems: list[str]) -> tuple[int, int, str | None]:
    """Re-derive (site count, import-slot count, allowlist digest) from the roster.

    The import-slot count rides the teardown receipt as
    ``pinned_hook_import_slot_count`` and is the count of the ONLY thing the
    ImportExport primitive mutates. It is re-derived here from the same roster
    text the digest is built from, for the same reason the site count is: a
    count is only a check while something independent knows what it should be.
    """
    source_path = (
        Path(__file__).resolve().parent.parent
        / "compact_contract"
        / "src"
        / "passive_ready.rs"
    )
    require(source_path.is_file(), "canonical_hook_authority_source_missing", problems)
    if not source_path.is_file():
        return 0, 0, None
    source = source_path.read_text(encoding="utf-8")
    count_match = re.search(
        r"pub const PINNED_PRODUCTION_HOOK_COUNT_V1:\s*u16\s*=\s*"
        r"PINNED_PRODUCTION_HOOK_SPECS_V1\.len\(\)\s+as\s+u16\s*;",
        source,
    )
    roster_match = re.search(
        r"pub const PINNED_PRODUCTION_HOOK_SPECS_V1:.*?=\s*&?\[(.*?)\n\];",
        source,
        re.DOTALL,
    )
    require(
        count_match is not None and roster_match is not None,
        "canonical_hook_authority_declaration_invalid",
        problems,
    )
    if count_match is None or roster_match is None:
        return 0, 0, None
    constants = {
        name: value
        for name, value in re.findall(
            r'const\s+([A-Z][A-Z0-9_]+):\s*&str\s*=\s*"([^"]+)"\s*;',
            source,
        )
    }
    # The import-slot patch width, parsed rather than assumed. This encoder is a
    # deliberately INDEPENDENT re-derivation of the Rust roster digest, which is
    # why it is allowed to disagree -- but independence is only worth something
    # while it tracks the contract, and it did not. When the two bootstrap
    # entries were corrected from the retired inline primitive
    # (minhook-0.9.0.normal-jmp-rel5, 5 bytes) to the one the provider actually
    # installs (pe64.import-address-slot.atomic-cas.v1, 8 bytes), this file and
    # its PowerShell twin were left on the old encoding and went on silently
    # producing the pre-correction digest
    # 1eed3cf79a86d69cb79f3d55ca95e5d8817cb3846100e96fcb103e0c21d2a3a6.
    import_width_match = re.search(
        r"pub const PE64_IMPORT_ADDRESS_SLOT_ATOMIC_WIDTH:\s*u16\s*=\s*(\d+)\s*;",
        source,
    )

    def macro_mandatory_byte(macro_name: str) -> bytes | None:
        """Parse one macro's `mandatory:` literal, bounded to its own body.

        The bit is part of the Rust allowlist preimage
        (``out.push(u8::from(entry.mandatory))``), so it moves the digest. This
        encoder hardcoded ``\\x01`` for BOTH macros; when the gameplay producer
        detours became non-mandatory it kept emitting the superseded digest
        b100b582f946788d45d66a4ee9192ac506a5498148582815476642c2010ce2dd.

        The body is bounded at the macro's own closing brace -- a ``}`` in
        column 0, since every brace inside a macro body is indented. Scanning to
        the next ``macro_rules!`` instead leaves the file's LAST macro reading to
        EOF, where struct definitions, doc comments and tests declare
        ``mandatory:`` too.
        """
        body = re.search(
            r"^macro_rules!\s+" + re.escape(macro_name) + r"\s*\{.*?^\}",
            source,
            re.DOTALL | re.MULTILINE,
        )
        if body is None:
            return None
        literals = re.findall(r"mandatory:\s*(true|false)\s*,", body.group(0))
        if len(literals) != 1:
            return None
        return b"\x01" if literals[0] == "true" else b"\x00"

    import_mandatory = macro_mandatory_byte("pinned_import")
    rva_mandatory = macro_mandatory_byte("pinned_rva")
    require(
        import_mandatory is not None and rva_mandatory is not None,
        "canonical_hook_authority_macro_mandatory_missing",
        problems,
    )
    if import_mandatory is None or rva_mandatory is None:
        return 0, 0, None
    require(
        import_width_match is not None,
        "canonical_hook_authority_import_width_missing",
        problems,
    )
    if import_width_match is None:
        return 0, 0, None
    import_patch_len = int(import_width_match.group(1))
    roster_source = roster_match.group(1)
    import_rows = re.findall(
        r'pinned_import!\(\s*"([^"]+)"\s*,\s*"([^"]+)"\s*,\s*'
        r'"([^"]+)"\s*,\s*"([0-9a-f]{64})"\s*\)',
        roster_source,
        re.DOTALL,
    )
    rva_rows = re.findall(
        r'pinned_rva!\(\s*"([^"]+)"\s*,\s*(0x[0-9A-Fa-f]+|\d+)\s*,\s*'
        r"(\d+)\s*,\s*([A-Z][A-Z0-9_]+|\"[^\"]+\")\s*,\s*"
        r'"([0-9a-f]{64})"\s*\)',
        roster_source,
        re.DOTALL,
    )
    expected_import_slot_count = len(import_rows)
    expected_count = len(import_rows) + len(rva_rows)
    declared_macro_count = len(
        re.findall(r"\bpinned_(?:import|rva)!\s*\(", roster_source)
    )
    require(
        0 < expected_count <= 0xFFFF
        and declared_macro_count == expected_count
        and expected_import_slot_count > 0,
        "canonical_hook_authority_roster_count_mismatch",
        problems,
    )
    if not (
        0 < expected_count <= 0xFFFF
        and declared_macro_count == expected_count
        and expected_import_slot_count > 0
    ):
        return 0, 0, None

    def named_digest(label: str, name: str) -> bytes:
        raw = name.encode("utf-8")
        return hashlib.sha256(
            labelled_preimage(label) + struct.pack("<H", len(raw)) + raw
        ).digest()

    entries: list[tuple[bytes, bytes]] = []
    logical_names: list[str] = []
    for logical_name, module_name, symbol_name, proof in import_rows:
        identity = named_digest(
            "battle-plan.capture_ready.hook_identity.v1", logical_name
        )
        primitive = constants.get("PE64_IMPORT_ADDRESS_SLOT_ATOMIC_CAS")
        if primitive is None:
            problems.append("canonical_hook_authority_import_primitive_missing")
            continue
        body = (
            identity
            + struct.pack("<HH", 1, import_patch_len)
            + named_digest(
                "battle-plan.capture_ready.hook_target_name.v1", module_name
            )
            + named_digest(
                "battle-plan.capture_ready.hook_target_name.v1", symbol_name
            )
            + named_digest(
                "battle-plan.capture_ready.hook_primitive_id.v1", primitive
            )
            + bytes.fromhex(proof)
            + import_mandatory
            + bytes(7)
        )
        entries.append((identity, body))
        logical_names.append(logical_name)
    for logical_name, raw_rva, raw_patch_len, primitive_expr, proof in rva_rows:
        primitive = (
            primitive_expr[1:-1]
            if primitive_expr.startswith('"')
            else constants.get(primitive_expr)
        )
        if primitive is None:
            problems.append(
                f"canonical_hook_authority_primitive_missing:{primitive_expr}"
            )
            continue
        identity = named_digest(
            "battle-plan.capture_ready.hook_identity.v1", logical_name
        )
        body = (
            identity
            + struct.pack("<HH", 2, int(raw_patch_len))
            + named_digest(
                "battle-plan.capture_ready.hook_target_name.v1", "hoi4.exe"
            )
            + struct.pack("<Q", int(raw_rva, 0))
            + named_digest(
                "battle-plan.capture_ready.hook_primitive_id.v1", primitive
            )
            + bytes(24)
            + bytes.fromhex(proof)
            + rva_mandatory
            + bytes(7)
        )
        entries.append((identity, body))
        logical_names.append(logical_name)
    require(
        len(entries) == expected_count
        and len(set(logical_names)) == expected_count
        and logical_names == sorted(logical_names),
        "canonical_hook_authority_roster_invalid",
        problems,
    )
    bootstrap_names = {
        "bootstrap.d3d11_create_device",
        "bootstrap.d3d_compile",
    }
    production_names = [
        name for name in logical_names if name not in bootstrap_names
    ]
    require(
        bootstrap_names.issubset(logical_names)
        and all(
            re.fullmatch(
                r"(?:gameplay\.d\d+|presentation\.d82)\.[a-z0-9_.-]+",
                name,
            )
            is not None
            for name in production_names
        )
        and all(
            not any(
                forbidden in name
                for forbidden in (
                    "a25",
                    "a28",
                    "a30",
                    "render",
                    "evidence",
                )
            )
            for name in logical_names
        ),
        "canonical_hook_authority_name_policy_invalid",
        problems,
    )
    entries.sort(key=lambda row: row[0])
    if len(entries) != expected_count:
        return expected_count, expected_import_slot_count, None
    preimage = (
        labelled_preimage("battle-plan.capture_ready.hook_allowlist.v1")
        + struct.pack("<H", expected_count)
        + b"".join(row[1] for row in entries)
    )
    return (
        expected_count,
        expected_import_slot_count,
        hashlib.sha256(preimage).hexdigest(),
    )


def canonical_production_domain_roster(problems: list[str]) -> list[tuple[int, str]]:
    registry_path = Path(__file__).resolve().parent.parent / "domain_registry.json"
    require(registry_path.is_file(), "canonical_domain_registry_missing", problems)
    if not registry_path.is_file():
        return []
    registry = load_object(registry_path)
    require(
        registry.get("schema") == "battle-plan.compact_domain_registry.v1",
        "canonical_domain_registry_schema_invalid",
        problems,
    )
    raw_domains = registry.get("domains")
    require(isinstance(raw_domains, list), "canonical_domain_registry_domains_invalid", problems)
    if not isinstance(raw_domains, list):
        return []
    roster: list[tuple[int, str]] = []
    for index, row in enumerate(raw_domains):
        if not isinstance(row, dict):
            problems.append(f"canonical_domain_registry_row_{index}_invalid")
            continue
        # `capture_scheduled: false` marks a domain the owner ruled out of
        # capture scope; the key is absent for every domain that is still
        # scheduled. The recorder never schedules these, so reconciling a sink
        # against them would report a permanent shortfall. Owned by
        # compact_contract::DESCHEDULED_CAPTURE_DOMAIN_IDS.
        if (
            row.get("schedulable") is True
            and row.get("capture_scheduled") is not False
            and row.get("status") in PRODUCTION_DOMAIN_STATUSES
        ):
            domain_id = row.get("id")
            name = row.get("name")
            require(
                is_int(domain_id) and 0 < domain_id <= 0xFFFF,
                f"canonical_domain_registry_row_{index}_id_invalid",
                problems,
            )
            require(
                isinstance(name, str) and bool(name),
                f"canonical_domain_registry_row_{index}_name_invalid",
                problems,
            )
            if is_int(domain_id) and 0 < domain_id <= 0xFFFF and isinstance(name, str) and name:
                roster.append((domain_id, name))
    roster.sort(key=lambda row: row[0])
    require(bool(roster), "canonical_production_domain_roster_empty", problems)
    require(
        len({row[0] for row in roster}) == len(roster)
        and len({row[1] for row in roster}) == len(roster),
        "canonical_production_domain_roster_not_unique",
        problems,
    )
    return roster


def grade_passive_lifecycle(
    run_dir: Path,
    plan: dict[str, Any],
    expected_domain_names: list[str],
    problems: list[str],
) -> None:
    (
        expected_hook_count,
        expected_import_slot_count,
        expected_hook_allowlist_sha256,
    ) = canonical_hook_authority(problems)
    ready_path = run_dir / "CAPTURE_READY_V2.json"
    teardown_path = run_dir / "CAPTURE_TEARDOWN_V1.json"
    require(ready_path.is_file(), "capture_ready_receipt_missing", problems)
    require(teardown_path.is_file(), "capture_teardown_receipt_missing", problems)
    if not ready_path.is_file() or not teardown_path.is_file():
        return
    ready = load_object(ready_path)
    teardown = load_object(teardown_path)
    ready_keys = {
        "schema", "status", "capture_policy", "pid",
        "process_creation_filetime_utc", "session_id", "capture_id",
        "requested_duration_ms", "readiness_flags", "validated_domain_count",
        "scheduler_queue_capacity", "scheduler_worker_count",
        "allowed_hook_count", "active_hook_count", "install_freeze_count",
        "clock_hour_raw",
        "clock_observation_ordinal", "target_executable_sha256", "collector_dll_sha256",
        "hook_allowlist_sha256", "hook_outcomes_sha256",
        "validated_domain_roster_sha256", "clock_witness_sha256",
        "scheduler_config_sha256", "output_pipe_identity_sha256",
        "zero_mutation_policy_sha256", "session_id_sha256",
        "capture_id_sha256", "control_sha256", "pipe_token_sha256",
        "ready_payload_sha256", "host_binding_sha256", "host_validated",
        "public_receipt_core_sha256", "ready_ack_sha256",
    }
    # `install_freeze_count` is REQUIRED above, in step with the PowerShell
    # mirror's roster; only the build declaration is an optional axis, kept in
    # step with Get-PassiveCaptureReadyDeclaredProperties in
    # PassiveCapture.Lifecycle.ps1. This set was left on the pre-declaration
    # roster, so it rejected every receipt the recorder actually writes as
    # `capture_ready_property_set_invalid` before it ever reached the digest
    # check below -- the same failure lane AS is recorded as having caused on
    # the teardown set.
    declaration_pair = {"collector_feature_bits", "collector_build"}
    declaration_present = declaration_pair & set(ready)
    require(
        len(declaration_present) in (0, len(declaration_pair)),
        "capture_ready_partial_build_declaration",
        problems,
    )
    ready_keys |= declaration_present
    # NAMED SEPARATELY, ahead of the generic set and digest checks below, for the
    # same reason the PowerShell mirror reads it before comparing property sets:
    # `capture_ready_property_set_invalid` and
    # `capture_ready_authenticated_payload_or_ack_digest_invalid` are both true
    # when this field is missing and neither says which field, which is exactly
    # how a producer skew read as an unexplained hash mismatch for a week.
    require(
        "install_freeze_count" in ready,
        "capture_ready_install_freeze_count_missing",
        problems,
    )
    teardown_keys = {
        "schema", "status", "capture_policy", "pid",
        "process_creation_filetime_utc", "session_id", "capture_id",
        "session_id_sha256", "capture_id_sha256", "pipe_token_sha256",
        "collector_dll_sha256", "capture_ready_payload_sha256",
        "capture_ready_receipt_core_sha256", "capture_ready_ack_sha256",
        "capture_ready_receipt_sha256", "host_binding_sha256",
        "data_pipe_eof_observed", "hook_lease_released_after_eof",
        "collector_module_disposition",
        "collector_module_retention_policy",
        "target_process_epoch_revalidated",
        "target_code_temporarily_instrumented",
        "pinned_hook_target_code_restored",
        "semantic_game_state_mutated", "pinned_hook_sites_restored",
        "pinned_hook_site_count", "pinned_hook_site_baseline_sha256",
        "pinned_hook_site_restored_sha256",
        # Lane AS added this to the recorder's teardown record and to the
        # PowerShell verifier's exact-property pin; this independent
        # re-derivation was left on the 28-key set, so it rejected every
        # receipt the recorder actually writes as
        # `capture_teardown_property_set_invalid`.
        "pinned_hook_import_slot_count",
        # Lane LU 2026-08-16: the collector's own terminal verdicts, or null
        # when it published none. The host's byte evidence above cannot see
        # that the collector's teardown sequence did not run.
        "collector_terminal_report",
    }
    require(set(ready) == ready_keys, "capture_ready_property_set_invalid", problems)
    require(
        set(teardown) == teardown_keys,
        "capture_teardown_property_set_invalid",
        problems,
    )
    # ADMITTING A PROPERTY IS NOT CHECKING IT. Null means the collector
    # published no terminal verdict; anything else must be a non-empty string.
    _terminal_report = teardown.get("collector_terminal_report")
    require(
        _terminal_report is None
        or (isinstance(_terminal_report, str) and _terminal_report.strip() != ""),
        "capture_teardown_collector_terminal_report_invalid",
        problems,
    )
    digest_fields = {
        "target_executable_sha256", "collector_dll_sha256",
        "hook_allowlist_sha256", "hook_outcomes_sha256",
        "validated_domain_roster_sha256", "clock_witness_sha256",
        "scheduler_config_sha256", "output_pipe_identity_sha256",
        "zero_mutation_policy_sha256", "session_id_sha256",
        "capture_id_sha256", "control_sha256", "pipe_token_sha256",
        "ready_payload_sha256", "host_binding_sha256",
        "public_receipt_core_sha256", "ready_ack_sha256",
    }
    require(
        all(is_hex64(ready.get(field)) and ready[field] != "0" * 64
            for field in digest_fields),
        "capture_ready_digest_set_invalid",
        problems,
    )
    expected_pid = plan.get("pid")
    expected_creation = plan.get("process_creation_filetime_utc")
    expected_duration = plan.get("requested_duration_ms")
    expected_queue = plan.get("read_only_scheduler_queue_cap")
    expected_workers = plan.get("scheduler_worker_count")
    expected_poll = plan.get("read_only_scheduler_poll_ms")
    expected_session = plan.get("session_id")
    expected_capture = plan.get("capture_id")
    require(
        is_int(expected_pid)
        and 0 < expected_pid <= 0xFFFFFFFF
        and is_int(expected_creation)
        and 0 < expected_creation <= 0xFFFFFFFFFFFFFFFF
        and is_int(expected_duration)
        and 0 < expected_duration <= 0xFFFFFFFF
        and is_int(expected_queue)
        and expected_queue == 1
        and is_int(expected_workers)
        and 0 < expected_workers <= 0xFFFF
        and is_int(expected_poll)
        and 0 < expected_poll <= 0xFFFFFFFF // 1000,
        "capture_plan_ready_configuration_invalid",
        problems,
    )
    require(
        ready.get("schema") == "hoi4-capture-ready-v2"
        and ready.get("status") == "ready"
        and ready.get("capture_policy") == "passive-observer-only"
        and is_int(ready.get("pid"))
        and 0 < ready.get("pid") <= 0xFFFFFFFF
        and ready.get("pid") == expected_pid
        and is_int(ready.get("process_creation_filetime_utc"))
        and 0 < ready.get("process_creation_filetime_utc") <= 0xFFFFFFFFFFFFFFFF
        and ready.get("process_creation_filetime_utc") == expected_creation
        and ready.get("session_id") == expected_session
        and ready.get("capture_id") == expected_capture
        and is_int(ready.get("requested_duration_ms"))
        and 0 < ready.get("requested_duration_ms") <= 0xFFFFFFFF
        and ready.get("requested_duration_ms") == expected_duration
        and is_int(ready.get("readiness_flags"))
        and 0 <= ready.get("readiness_flags") <= 0xFFFFFFFF
        and ready.get("readiness_flags") == 255
        and is_int(ready.get("validated_domain_count"))
        and 0 < ready.get("validated_domain_count") <= 0xFFFFFFFF
        and ready.get("validated_domain_count") == len(expected_domain_names)
        and is_int(ready.get("scheduler_queue_capacity"))
        and 0 < ready.get("scheduler_queue_capacity") <= 0xFFFFFFFF
        and ready.get("scheduler_queue_capacity") == expected_queue == 1
        and is_int(ready.get("scheduler_worker_count"))
        and 0 < ready.get("scheduler_worker_count") <= 0xFFFF
        and ready.get("scheduler_worker_count") == expected_workers
        and is_int(ready.get("allowed_hook_count"))
        and 0 < ready.get("allowed_hook_count") <= 0xFFFF
        and ready.get("allowed_hook_count") == expected_hook_count
        and is_int(ready.get("active_hook_count"))
        and 0 < ready.get("active_hook_count") <= 0xFFFF
        and ready.get("active_hook_count") == expected_hook_count
        and is_int(ready.get("clock_hour_raw"))
        and 60005999 <= ready.get("clock_hour_raw") <= 63071999
        and is_int(ready.get("clock_observation_ordinal"))
        and 0 <= ready.get("clock_observation_ordinal") < 0xFFFFFFFFFFFFFFFF
        and ready.get("host_validated") is True,
        "capture_ready_identity_or_configuration_invalid",
        problems,
    )
    require(
        ready.get("target_executable_sha256") == plan.get("exe_sha256")
        == EXACT_A729_SHA256
        and ready.get("collector_dll_sha256") == plan.get("staged_dll_sha256")
        and ready.get("hook_allowlist_sha256")
        == expected_hook_allowlist_sha256,
        "capture_ready_binary_binding_invalid",
        problems,
    )
    expected_session_sha = (
        utf8_sha256(expected_session) if isinstance(expected_session, str) else None
    )
    expected_capture_sha = (
        utf8_sha256(expected_capture) if isinstance(expected_capture, str) else None
    )
    expected_roster_sha = utf8_sha256("\n".join(expected_domain_names))
    if (
        is_int(expected_poll)
        and 0 < expected_poll <= 0xFFFFFFFF // 1000
        and is_int(expected_workers)
        and 0 < expected_workers <= 0xFFFF
    ):
        scheduler_preimage = (
            labelled_preimage("battle-plan.capture_ready.scheduler_config.v1")
            + struct.pack(
                "<IIIHHIQ",
                expected_poll * 1000,
                1,
                2,
                expected_workers,
                0,
                0,
                256 * 1024 * 1024,
            )
        )
        expected_scheduler_sha = hashlib.sha256(scheduler_preimage).hexdigest()
    else:
        expected_scheduler_sha = None
    expected_mutation_sha = hashlib.sha256(
        labelled_preimage("battle-plan.capture_ready.zero_mutation_policy.v1")
        + struct.pack("<H", 1)
    ).hexdigest()
    if (
        is_int(ready.get("clock_hour_raw"))
        and 60005999 <= ready.get("clock_hour_raw") <= 63071999
        and is_int(ready.get("clock_observation_ordinal"))
        and 0 <= ready.get("clock_observation_ordinal") < 0xFFFFFFFFFFFFFFFF
        and is_hex64(ready.get("target_executable_sha256"))
    ):
        clock_preimage = (
            labelled_preimage(
                "battle-plan.capture_ready.read_only_clock_witness.v1"
            )
            + struct.pack(
                "<iiQ",
                ready["clock_hour_raw"],
                ready["clock_hour_raw"],
                ready["clock_observation_ordinal"],
            )
            + bytes.fromhex(ready["target_executable_sha256"])
            + hashlib.sha256(COLLECTOR_BUILD_PROFILE_ID.encode("utf-8")).digest()
        )
        expected_clock_sha = hashlib.sha256(clock_preimage).hexdigest()
    else:
        expected_clock_sha = None
    staged_dll_value = plan.get("staged_dll")
    control_path = (
        Path(staged_dll_value).resolve().parent / "collector_control.json"
        if isinstance(staged_dll_value, str)
        else run_dir / "__missing_control__"
    )
    try:
        control_path.relative_to(run_dir)
        control_contained = True
    except ValueError:
        control_contained = False
    require(
        control_contained and control_path.is_file(),
        "capture_ready_control_file_missing_or_uncontained",
        problems,
    )
    control_bytes = (
        control_path.read_bytes()
        if control_contained and control_path.is_file()
        else None
    )
    expected_control_sha = (
        hashlib.sha256(control_bytes).hexdigest()
        if isinstance(control_bytes, bytes)
        else None
    )
    control = (
        decode_object(control_bytes, str(control_path))
        if isinstance(control_bytes, bytes)
        else {}
    )
    pipe_token = control.get("pipe_token")
    expected_pipe_name = (
        f"\\\\.\\pipe\\battle-plan_control_{expected_pid}_{pipe_token}"
        if is_int(expected_pid) and isinstance(pipe_token, str) and pipe_token
        else None
    )
    session_uuid = canonical_uuid_bytes(expected_session)
    pipe_name_bytes = (
        expected_pipe_name.encode("utf-8")
        if isinstance(expected_pipe_name, str)
        else b""
    )
    pipe_suffix = (
        expected_pipe_name[9:] if isinstance(expected_pipe_name, str) else ""
    )
    pipe_name_valid = (
        isinstance(expected_pipe_name, str)
        and expected_pipe_name.startswith("\\\\.\\pipe\\")
        and 0 < len(pipe_name_bytes) <= 256
        and all(0x21 <= byte <= 0x7E for byte in pipe_name_bytes)
        and bool(pipe_suffix)
        and "\\" not in pipe_suffix
        and "/" not in pipe_suffix
    )
    require(
        isinstance(pipe_token, str)
        and bool(pipe_token)
        and control.get("capture_ready_pipe_name_v1") == expected_pipe_name
        and pipe_name_valid
        and session_uuid is not None,
        "capture_ready_output_pipe_control_identity_invalid",
        problems,
    )
    expected_pipe_token_sha = (
        utf8_sha256(pipe_token) if isinstance(pipe_token, str) and pipe_token else None
    )
    if (
        isinstance(expected_pipe_name, str)
        and is_int(expected_pid)
        and 0 < expected_pid <= 0xFFFFFFFF
        and is_int(expected_creation)
        and 0 < expected_creation <= 0xFFFFFFFFFFFFFFFF
        and session_uuid is not None
        and pipe_name_valid
    ):
        output_pipe_preimage = (
            labelled_preimage(
                "battle-plan.capture_ready.output_pipe_identity.v1"
            )
            + struct.pack("<H", len(pipe_name_bytes))
            + pipe_name_bytes
            + struct.pack("<IQ", expected_pid, expected_creation)
            + session_uuid
        )
        expected_output_pipe_sha = hashlib.sha256(output_pipe_preimage).hexdigest()
    else:
        expected_output_pipe_sha = None
    expected_ready_payload_sha = (
        capture_ready_payload_sha256(ready, pipe_token)
        if isinstance(pipe_token, str) and pipe_token
        else None
    )
    expected_ready_ack_sha = (
        capture_ready_ack_sha256(ready, pipe_token)
        if isinstance(pipe_token, str) and pipe_token
        else None
    )
    require(
        ready.get("session_id_sha256") == expected_session_sha
        and ready.get("capture_id_sha256") == expected_capture_sha
        and ready.get("validated_domain_roster_sha256") == expected_roster_sha
        and ready.get("scheduler_config_sha256") == expected_scheduler_sha
        and ready.get("zero_mutation_policy_sha256") == expected_mutation_sha
        and ready.get("clock_witness_sha256") == expected_clock_sha
        and ready.get("output_pipe_identity_sha256") == expected_output_pipe_sha
        and ready.get("pipe_token_sha256") == expected_pipe_token_sha
        and ready.get("control_sha256") == expected_control_sha,
        "capture_ready_derived_digest_invalid",
        problems,
    )
    require(
        ready.get("ready_payload_sha256") == expected_ready_payload_sha
        and ready.get("ready_ack_sha256") == expected_ready_ack_sha,
        "capture_ready_authenticated_payload_or_ack_digest_invalid",
        problems,
    )
    host_digest_inputs = (
        ready.get("ready_payload_sha256"),
        ready.get("target_executable_sha256"),
        ready.get("collector_dll_sha256"),
        expected_session_sha,
        expected_capture_sha,
        ready.get("pipe_token_sha256"),
    )
    if (
        is_int(expected_pid) and 0 <= expected_pid <= 0xFFFFFFFF
        and is_int(expected_creation) and 0 <= expected_creation <= 0xFFFFFFFFFFFFFFFF
        and is_int(expected_duration) and 0 <= expected_duration <= 0xFFFFFFFF
        and all(is_hex64(value) for value in host_digest_inputs)
    ):
        host_bytes = (
            b"battle-plan.capture_ready.host_binding.v2\0"
            + bytes.fromhex(host_digest_inputs[0])
            + struct.pack("<IQI", expected_pid, expected_creation, expected_duration)
            + b"".join(bytes.fromhex(value) for value in host_digest_inputs[1:])
        )
        expected_host_sha = hashlib.sha256(host_bytes).hexdigest()
    else:
        expected_host_sha = None
    require(
        ready.get("host_binding_sha256") == expected_host_sha,
        "capture_ready_host_binding_invalid",
        problems,
    )
    require(
        teardown.get("schema") == "hoi4-capture-teardown-v2"
        and teardown.get("status")
        in ("collector-retained-pinned", "collector-unloaded")
        and teardown.get("capture_policy") == "passive-observer-only"
        and teardown.get("pid") == expected_pid
        and teardown.get("process_creation_filetime_utc") == expected_creation
        and teardown.get("session_id") == expected_session
        and teardown.get("capture_id") == expected_capture
        and teardown.get("data_pipe_eof_observed") is True
        and teardown.get("hook_lease_released_after_eof") is True
        and teardown.get("collector_module_disposition")
        in ("retained-pinned", "unloaded")
        and (teardown.get("collector_module_disposition") == "retained-pinned")
        == (teardown.get("status") == "collector-retained-pinned")
        and teardown.get("collector_module_retention_policy")
        == "inline-detour.module.process-lifetime.v1"
        and teardown.get("target_process_epoch_revalidated") is True
        and teardown.get("target_code_temporarily_instrumented") is True
        and teardown.get("pinned_hook_target_code_restored") is True
        and teardown.get("semantic_game_state_mutated") is False
        and teardown.get("pinned_hook_sites_restored") is True
        and teardown.get("pinned_hook_site_count") == expected_hook_count
        # A zero expectation means the roster re-derivation failed above (it
        # already recorded its own problem); never let that degrade into
        # accepting a receipt that claims zero verified import slots.
        and expected_import_slot_count > 0
        and teardown.get("pinned_hook_import_slot_count")
            == expected_import_slot_count
        and is_hex64(teardown.get("pinned_hook_site_baseline_sha256"))
        and teardown.get("pinned_hook_site_baseline_sha256") != "0" * 64
        and teardown.get("pinned_hook_site_restored_sha256")
            == teardown.get("pinned_hook_site_baseline_sha256"),
        "capture_teardown_identity_or_state_invalid",
        problems,
    )
    require(
        teardown.get("session_id_sha256") == ready.get("session_id_sha256")
        and teardown.get("capture_id_sha256") == ready.get("capture_id_sha256")
        and teardown.get("pipe_token_sha256") == ready.get("pipe_token_sha256")
        and teardown.get("collector_dll_sha256") == ready.get("collector_dll_sha256")
        and teardown.get("capture_ready_payload_sha256")
            == ready.get("ready_payload_sha256")
        and teardown.get("capture_ready_receipt_core_sha256")
            == ready.get("public_receipt_core_sha256")
        and teardown.get("capture_ready_ack_sha256")
            == ready.get("ready_ack_sha256")
        and teardown.get("capture_ready_receipt_sha256") == sha256(ready_path)
        and teardown.get("host_binding_sha256")
            == ready.get("host_binding_sha256"),
        "capture_teardown_ready_binding_invalid",
        problems,
    )


def grade_passive_domain_outcomes(
    compact: dict[str, Any],
    expected_roster: list[tuple[int, str]],
    problems: list[str],
) -> None:
    expected_domain_ids = {row[0] for row in expected_roster}
    expected_domain_names = [row[1] for row in expected_roster]
    frame_count = compact.get("frame_end_record_count")
    require(is_int(frame_count) and frame_count > 0, "frame_end_count_invalid", problems)
    if not is_int(frame_count) or frame_count <= 0:
        return

    # The sealed manifest carries a RUN-ENCODED gap ledger, expanded here into
    # exactly the rows the flat `domain_gap_outcomes` array used to hold, so the
    # per-row grading below is unchanged. A manifest sealed with the flat array
    # expands through the same call, which is why archived captures still grade.
    #
    # A ledger this build cannot read is a MISSING ledger, not an empty one: the
    # rows below would otherwise all pass vacuously on a manifest nobody read.
    try:
        gaps = gap_ledger.expand_domain_gap_outcomes(compact)
        readable = True
    except gap_ledger.GapLedgerError:
        gaps = []
        readable = False
    require(readable, "domain_gap_outcomes_missing", problems)
    require(
        compact.get("domain_gap_record_count") == len(gaps),
        "domain_gap_record_count_mismatch",
        problems,
    )
    require(len(gaps) == 0, "production_zero_loss_domain_gaps_present", problems)

    gap_counts: dict[int, int] = {}
    next_ordinals: dict[tuple[int, str], int] = {}
    previous_ranges: dict[tuple[int, str], tuple[int, int]] = {}
    gap_targets: set[tuple[int, int]] = set()
    for index, row in enumerate(gaps):
        prefix = f"domain_gap_{index}"
        if not isinstance(row, dict):
            problems.append(f"{prefix}_not_object")
            continue
        domain_id = row.get("domain_id")
        part = row.get("part")
        reason = row.get("reason")
        start = row.get("range_start_hour_raw")
        end = row.get("range_end_hour_raw")
        ordinal = row.get("outcome_ordinal")
        target = row.get("target_hour_raw")
        require(is_int(domain_id) and 0 < domain_id <= 0xFFFF,
                f"{prefix}_domain_invalid", problems)
        require(part in ALLOWED_GAP_PARTS, f"{prefix}_part_invalid", problems)
        require(reason in ALLOWED_GAP_REASONS, f"{prefix}_reason_invalid", problems)
        require(is_int(start) and is_int(end) and start <= end,
                f"{prefix}_range_invalid", problems)
        require(is_int(target) and is_int(start) and is_int(end)
                and start <= target <= end,
                f"{prefix}_target_outside_range", problems)
        require(is_int(ordinal) and ordinal >= 0,
                f"{prefix}_ordinal_invalid", problems)
        if not (is_int(domain_id) and part in ALLOWED_GAP_PARTS):
            continue
        require(domain_id in expected_domain_ids,
                f"{prefix}_domain_not_scheduled", problems)
        if is_int(target):
            target_key = (domain_id, target)
            require(target_key not in gap_targets,
                    f"{prefix}_duplicate_domain_frame_terminal", problems)
            gap_targets.add(target_key)
        key = (domain_id, part)
        expected = next_ordinals.get(key, 0)
        require(ordinal == expected, f"{prefix}_ordinal_discontinuous", problems)
        next_ordinals[key] = expected + 1
        if is_int(start) and is_int(end) and start <= end:
            prior = previous_ranges.get(key)
            require(prior is None or start > prior[1],
                    f"{prefix}_range_overlap_or_regression", problems)
            previous_ranges[key] = (start, end)
        gap_counts[domain_id] = gap_counts.get(domain_id, 0) + 1

    per_domain = compact.get("per_domain")
    require(isinstance(per_domain, dict), "per_domain_missing", problems)
    if not isinstance(per_domain, dict):
        per_domain = {}
    expected = compact.get("expected_domains")
    require(
        expected == expected_domain_names,
        "expected_domain_roster_not_canonical_or_ordered",
        problems,
    )
    require(
        nested(compact, "domain_coverage", "ok") is True
        and nested(compact, "domain_coverage", "errors") == [],
        "domain_coverage_not_closed",
        problems,
    )

    seen_ids: set[int] = set()
    observed_roster: list[tuple[int, str]] = []
    for name, row in per_domain.items():
        if not isinstance(row, dict):
            problems.append(f"per_domain_{name}_not_object")
            continue
        domain_id = row.get("domain_id")
        accepted = row.get("record_count")
        data_count = row.get("data_record_count")
        raw_flags = row.get("status_flags_or")
        effective_flags = row.get("effective_failure_status_flags_or")
        require(is_int(domain_id) and 0 < domain_id <= 0xFFFF,
                f"per_domain_{name}_id_invalid", problems)
        require(is_int(accepted) and accepted >= 0,
                f"per_domain_{name}_record_count_invalid", problems)
        require(is_int(data_count) and data_count == accepted,
                f"per_domain_{name}_data_record_count_mismatch", problems)
        require(
            is_int(raw_flags) and raw_flags >= 0
            and raw_flags & CANONICAL_COMPACT_FAILURE_MASK == 0,
            f"per_domain_{name}_raw_failure_status",
            problems,
        )
        require(
            is_int(effective_flags) and effective_flags >= 0
            and effective_flags & CANONICAL_COMPACT_FAILURE_MASK == 0,
            f"per_domain_{name}_effective_failure_status",
            problems,
        )
        if is_int(domain_id) and is_int(accepted):
            seen_ids.add(domain_id)
            observed_roster.append((domain_id, name))
            require(
                accepted + gap_counts.get(domain_id, 0) == frame_count,
                f"domain_{domain_id}_terminal_outcome_count_mismatch",
                problems,
            )
    for domain_id, gap_count in gap_counts.items():
        if domain_id not in seen_ids:
            require(
                gap_count == frame_count,
                f"domain_{domain_id}_gap_only_terminal_outcome_count_mismatch",
                problems,
            )
    require(
        seen_ids | set(gap_counts) == expected_domain_ids,
        "scheduled_domain_id_roster_mismatch",
        problems,
    )
    require(
        sorted(observed_roster, key=lambda row: row[0]) == expected_roster,
        "per_domain_id_name_pair_roster_not_canonical",
        problems,
    )


def grade_success(run_dir: Path, problems: list[str]) -> dict[str, Any]:
    manifest_path = run_dir / "native_session_manifest.json"
    plan_path = run_dir / "production_capture_plan.json"
    speed_path = run_dir / "speed5_proof.json"
    require(manifest_path.is_file(), "native_session_manifest_missing", problems)
    require(plan_path.is_file(), "production_capture_plan_missing", problems)
    require(speed_path.is_file(), "speed5_proof_missing", problems)
    if not (manifest_path.is_file() and plan_path.is_file() and speed_path.is_file()):
        return {"manifest": None}

    manifest = load_object(manifest_path)
    plan = load_object(plan_path)
    speed = load_object(speed_path)
    require(plan.get("schema") == "battle-plan.production_capture_plan.v1",
            "production_capture_plan_schema_invalid", problems)
    require(plan.get("unpaused_required") is True,
            "production_capture_did_not_require_unpaused", problems)
    require(plan.get("read_only_scheduler_required") is True,
            "read_only_scheduler_not_required", problems)
    closure = plan.get("interaction_closure")
    closure = closure if isinstance(closure, dict) else {}
    expected_closure = {
        "policy": "passive_injected_observer",
        "injected_collector_allowed": True,
        "evidence_backed_hooks_allowed": True,
        "client_commands_during_capture": 0,
        "semantic_engine_state_writes": 0,
        "game_process_lifecycle_commands": 0,
        "game_ui_commands": 0,
    }
    require(
        all(closure.get(key) == value for key, value in expected_closure.items()),
        "interaction_closure_invalid",
        problems,
    )
    policy_inputs = closure.get("policy_inputs")
    policy_inputs = policy_inputs if isinstance(policy_inputs, dict) else {}
    require(
        set(policy_inputs) == {
            "read_only_game_state_witness_sha256",
            "passive_capture_policy_test_sha256",
            "passive_capture_lifecycle_sha256",
        }
        and all(is_hex64(value) for value in policy_inputs.values()),
        "interaction_closure_policy_inputs_invalid",
        problems,
    )
    current_policy_inputs = {
        "read_only_game_state_witness_sha256": sha256(
            Path(__file__).with_name("Read-Hoi4GameState.ps1")
        ),
        "passive_capture_policy_test_sha256": sha256(
            Path(__file__).with_name("Test-PassiveCapturePolicy.ps1")
        ),
        "passive_capture_lifecycle_sha256": sha256(
            Path(__file__).with_name("PassiveCapture.Lifecycle.ps1")
        ),
    }
    require(
        policy_inputs == current_policy_inputs,
        "interaction_closure_policy_inputs_stale",
        problems,
    )

    arguments = plan.get("recorder_arguments")
    require(isinstance(arguments, list) and all(isinstance(v, str) for v in arguments),
            "recorder_arguments_invalid", problems)
    arguments = arguments if isinstance(arguments, list) else []
    require("--read-only-scheduler" in arguments,
            "read_only_scheduler_argument_missing", problems)
    require("--read-only-scheduler-poll-ms" in arguments,
            "read_only_scheduler_poll_argument_missing", problems)
    require("--read-only-scheduler-queue-cap" in arguments,
            "read_only_scheduler_capacity_argument_missing", problems)
    for forbidden in FORBIDDEN_RECORDER_ARGUMENTS:
        require(forbidden not in arguments, f"forbidden_recorder_argument:{forbidden}", problems)

    require(plan.get("session_id") == manifest.get("session_id"),
            "session_id_mismatch", problems)
    require(plan.get("capture_id") == manifest.get("capture_id"),
            "capture_id_mismatch", problems)
    require(plan.get("pid") == manifest.get("pid"), "process_id_mismatch", problems)
    require(plan.get("speed_required_raw") == 4, "speed5_not_required", problems)
    require(speed.get("schema") == "battle-plan.speed5_proof.v2",
            "speed5_proof_schema_invalid", problems)
    require(speed.get("pid") == plan.get("pid"), "speed5_process_id_mismatch", problems)
    require(speed.get("speed_raw") == 4 and speed.get("paused") is False,
            "speed5_unpaused_not_proven", problems)
    require(is_int(speed.get("sample_count")) and speed["sample_count"] >= 3,
            "speed5_samples_insufficient", problems)
    require(speed.get("capture_plan_sha256") == sha256(plan_path),
            "speed5_capture_plan_digest_mismatch", problems)
    require(speed.get("exe_sha256") == EXACT_A729_SHA256,
            "speed5_executable_not_exact_a729", problems)

    # The passive product must never publish the retired client-owning path.
    for name in (
        "early_launch_authorization_plan.json",
        "early_launch_resolved_attempt.json",
        "early_render_capture_receipt.json",
    ):
        require(not (run_dir / name).exists(), f"forbidden_early_launch_artifact:{name}", problems)
    require(manifest.get("early_render_launch_manifest") is None,
            "forbidden_early_launch_manifest_present", problems)
    require(manifest.get("early_render_capture_receipt") is None,
            "forbidden_early_capture_receipt_present", problems)

    expected_domain_roster = canonical_production_domain_roster(problems)
    expected_domain_ids = [row[0] for row in expected_domain_roster]
    expected_domain_names = [row[1] for row in expected_domain_roster]
    expected_domain_roster_json = [
        {"id": domain_id, "name": name}
        for domain_id, name in expected_domain_roster
    ]
    raw_expected_ids = plan.get("production_profile_domain_ids")
    raw_expected_names = plan.get("production_profile_domain_names")
    require(
        raw_expected_ids == expected_domain_ids
        and raw_expected_names == expected_domain_names
        and list(zip(raw_expected_ids, raw_expected_names)) == expected_domain_roster,
        "production_profile_domain_roster_not_canonical_or_pairwise_ordered",
        problems,
    )
    require(
        plan.get("production_profile_domain_roster")
        == expected_domain_roster_json,
        "production_profile_domain_pair_roster_not_canonical_or_ordered",
        problems,
    )
    scheduled_domain_names = plan.get("scheduled_domain_names")
    scheduled_domain_roster_json = plan.get("scheduled_domain_roster")
    scheduled_domain_names = (
        scheduled_domain_names if isinstance(scheduled_domain_names, list) else []
    )
    scheduled_domain_roster_json = (
        scheduled_domain_roster_json
        if isinstance(scheduled_domain_roster_json, list)
        else []
    )
    scheduled_domain_roster: list[tuple[int, str]] = []
    if (
        len(scheduled_domain_names) == len(expected_domain_roster)
        and len(scheduled_domain_roster_json) == len(expected_domain_roster)
    ):
        for index, ((expected_id, _), scheduled_name, row) in enumerate(
            zip(
                expected_domain_roster,
                scheduled_domain_names,
                scheduled_domain_roster_json,
            )
        ):
            if (
                isinstance(scheduled_name, str)
                and bool(scheduled_name)
                and isinstance(row, dict)
                and set(row) == {"id", "wire_name"}
                and row.get("id") == expected_id
                and row.get("wire_name") == scheduled_name
            ):
                scheduled_domain_roster.append((expected_id, scheduled_name))
            else:
                problems.append(f"scheduled_domain_roster_row_{index}_invalid")
    require(
        len(scheduled_domain_roster) == len(expected_domain_roster)
        and len({row[1] for row in scheduled_domain_roster})
        == len(scheduled_domain_roster)
        and is_hex64(plan.get("domain_roster_tool_sha256")),
        "scheduled_domain_roster_not_authoritative_pairwise_or_ordered",
        problems,
    )
    roster_authority_path = run_dir / "production_domain_roster_authority.json"
    raw_roster_authority_path = plan.get("domain_roster_authority_path")
    try:
        planned_roster_authority_path = (
            Path(raw_roster_authority_path).resolve()
            if isinstance(raw_roster_authority_path, str)
            else run_dir / "__missing_roster_authority__"
        )
    except (OSError, RuntimeError):
        planned_roster_authority_path = run_dir / "__invalid_roster_authority__"
    require(
        planned_roster_authority_path == roster_authority_path
        and roster_authority_path.is_file()
        and is_hex64(plan.get("domain_roster_authority_sha256"))
        and (
            sha256(roster_authority_path)
            == plan.get("domain_roster_authority_sha256")
            if roster_authority_path.is_file()
            else False
        ),
        "domain_roster_authority_file_not_digest_bound",
        problems,
    )
    authority_roster: Any = []
    if roster_authority_path.is_file():
        try:
            authority_roster = load_json(roster_authority_path)
        except (OSError, UnicodeDecodeError, json.JSONDecodeError, ValueError):
            problems.append("domain_roster_authority_not_strict_json")
    expected_authority_roster = [
        {
            "id": domain_id,
            "name": canonical_name,
            "wire_name": wire_name,
        }
        for (domain_id, canonical_name), (_, wire_name) in zip(
            expected_domain_roster,
            scheduled_domain_roster,
        )
    ]
    require(
        authority_roster == expected_authority_roster,
        "domain_roster_authority_not_exact_canonical_wire_join",
        problems,
    )
    require(
        manifest.get("scheduled_domains") == scheduled_domain_names,
        "manifest_scheduled_domain_roster_not_canonical_or_ordered",
        problems,
    )
    if scheduled_domain_roster:
        grade_passive_lifecycle(
            run_dir,
            plan,
            scheduled_domain_names,
            problems,
        )
    compact = manifest.get("compact_stream")
    require(isinstance(compact, dict), "compact_stream_missing", problems)
    if isinstance(compact, dict):
        require("hour_latch" not in compact, "forbidden_hour_latch_summary_present", problems)
        require(compact.get("decodable") is True, "compact_stream_not_decodable", problems)
        require(compact.get("ordered") is True, "compact_stream_not_ordered", problems)
        grade_passive_domain_outcomes(compact, scheduled_domain_roster, problems)

    raw_stream_name = manifest.get("raw_stream")
    raw_stream = (run_dir / (
        raw_stream_name if isinstance(raw_stream_name, str) else "collector_stream.bin"
    )).resolve()
    try:
        raw_stream.relative_to(run_dir)
    except ValueError:
        problems.append("collector_stream_path_escapes_run")
        raw_stream = run_dir / "collector_stream.bin"
    require(raw_stream.is_file(), "collector_stream_missing", problems)
    raw_stream_sha256 = sha256(raw_stream) if raw_stream.is_file() else None
    raw_stream_bytes = raw_stream.stat().st_size if raw_stream.is_file() else None
    manifest_storage = manifest.get("stream_storage")
    manifest_storage = manifest_storage if isinstance(manifest_storage, dict) else {}
    require(bool(manifest_storage), "manifest_stream_storage_missing", problems)
    require(
        manifest_storage.get("compression") in {"none", "zstd"}
        and is_int(manifest_storage.get("stored_bytes"))
        and manifest_storage.get("stored_bytes") == raw_stream_bytes
        and is_int(manifest_storage.get("decoded_bytes"))
        and manifest_storage.get("decoded_bytes") >= 0
        and is_hex64(manifest_storage.get("decoded_sha256")),
        "manifest_stream_storage_invalid_or_unbound",
        problems,
    )
    if "stored_sha256" in manifest_storage:
        require(
            manifest_storage.get("stored_sha256") == raw_stream_sha256,
            "manifest_stream_storage_stored_digest_mismatch",
            problems,
        )
    if manifest_storage.get("compression") == "none":
        require(
            manifest_storage.get("decoded_bytes") == raw_stream_bytes
            and manifest_storage.get("decoded_sha256") == raw_stream_sha256,
            "uncompressed_stream_storage_decoded_identity_mismatch",
            problems,
        )
    if raw_stream.is_file():
        require(
            raw_stream_sha256 == nested(manifest, "host_finalize", "stream_sha256"),
            "collector_stream_digest_mismatch",
            problems,
        )
    validation_path = run_dir / "production_validation_report.json"
    require(validation_path.is_file(), "production_validation_report_missing", problems)
    if validation_path.is_file():
        validation = load_object(validation_path)
        require(validation.get("schema") == "battle-plan.production_capture_validation.v2"
                and validation.get("ok") is True
                and validation.get("problems") == [],
                "production_validation_report_invalid", problems)
        require(
            validation.get("manifest_sha256") == sha256(manifest_path),
            "production_validation_manifest_digest_mismatch",
            problems,
        )
        require(
            validation.get("capture_id") == plan.get("capture_id")
            and validation.get("session_id") == plan.get("session_id"),
            "production_validation_identity_mismatch",
            problems,
        )
        require(
            validation.get("stream_sha256")
            == nested(manifest, "host_finalize", "stream_sha256"),
            "production_validation_stream_digest_mismatch",
            problems,
        )
        require(
            validation.get("expected_domain_count") == len(expected_domain_roster)
            and validation.get("expected_domain_roster")
            == expected_domain_roster_json
            and validation.get("scheduled_domain_roster")
            == scheduled_domain_roster_json
            and isinstance(validation.get("domain_roster_authority_path"), str)
            and Path(validation["domain_roster_authority_path"]).resolve()
            == roster_authority_path
            and validation.get("domain_roster_authority_sha256")
            == plan.get("domain_roster_authority_sha256"),
            "production_validation_domain_roster_not_canonical_or_ordered",
            problems,
        )
        independent = validation.get("independent_gates")
        independent = independent if isinstance(independent, list) else []
        compact_results = [
            row for row in independent
            if isinstance(row, dict) and row.get("name") == "compact_gate"
        ]
        require(
            len(compact_results) == 1 and compact_results[0].get("passed") is True,
            "independent_compact_gate_not_proven",
            problems,
        )
        if len(compact_results) == 1:
            compact_result = compact_results[0]
            compact_args = compact_result.get("arguments")
            compact_args = compact_args if isinstance(compact_args, list) else []
            try:
                compact_run_arg = compact_args[compact_args.index("--run-dir") + 1]
                compact_run_matches = (
                    Path(compact_run_arg).resolve() == run_dir
                )
            except (ValueError, IndexError, TypeError):
                compact_run_matches = False
            require(
                all(isinstance(value, str) for value in compact_args)
                and compact_args.count("--run-dir") == 1
                and compact_run_matches
                and compact_args.count("--require-zero-loss") == 1
                and compact_args.count("--write-report") == 1
                and is_hex64(compact_result.get("executable_sha256")),
                "independent_compact_gate_invocation_unbound",
                problems,
            )
            compact_report_value = compact_result.get("report_path")
            compact_report = Path(compact_report_value).resolve() if isinstance(
                compact_report_value, str
            ) else run_dir / "__missing_compact_report__"
            expected_report_parent = (run_dir / "independent_validation").resolve()
            try:
                compact_report.relative_to(expected_report_parent)
                compact_report_contained = True
            except ValueError:
                compact_report_contained = False
            require(
                compact_report_contained and compact_report.is_file(),
                "independent_compact_gate_report_missing_or_uncontained",
                problems,
            )
            if compact_report_contained and compact_report.is_file():
                try:
                    compact_write_arg = compact_args[
                        compact_args.index("--write-report") + 1
                    ]
                    compact_write_matches = (
                        Path(compact_write_arg).resolve() == compact_report
                    )
                except (ValueError, IndexError, TypeError):
                    compact_write_matches = False
                require(
                    compact_write_matches,
                    "independent_compact_gate_report_argument_unbound",
                    problems,
                )
                require(
                    compact_result.get("report_sha256") == sha256(compact_report),
                    "independent_compact_gate_report_digest_mismatch",
                    problems,
                )
                compact_report_json = load_object(compact_report)
                require(
                    compact_report_json.get("gate")
                    == "compact_replay_verification_gates"
                    and compact_report_json.get("passed") is True
                    and compact_report_json.get("errors") == [],
                    "independent_compact_gate_report_invalid",
                    problems,
                )
                require(
                    nested(
                        compact_report_json,
                        "details",
                        "verification_profile",
                        "gap_acceptance",
                    )
                    == "require_zero_loss"
                    and nested(
                        compact_report_json,
                        "details",
                        "verification_profile",
                        "zero_loss_claimed",
                    )
                    is True,
                    "independent_compact_gate_zero_loss_not_proven",
                    problems,
                )
                compact_details = compact_report_json.get("details")
                compact_details = (
                    compact_details if isinstance(compact_details, dict) else {}
                )
                compact_manifest_value = compact_details.get("manifest")
                compact_stream_value = compact_details.get("stream_file")
                require(
                    isinstance(compact_details.get("run_dir"), str)
                    and Path(compact_details["run_dir"]).resolve() == run_dir
                    and isinstance(compact_manifest_value, str)
                    and Path(compact_manifest_value).resolve() == manifest_path,
                    "independent_compact_gate_run_or_manifest_path_unbound",
                    problems,
                )
                require(
                    isinstance(compact_stream_value, str)
                    and Path(compact_stream_value).resolve() == raw_stream,
                    "independent_compact_gate_stream_path_unbound",
                    problems,
                )
                require(
                    compact_details.get("scheduled_domains")
                    == scheduled_domain_names
                    and compact_details.get("required_domains") == [],
                    "independent_compact_gate_domain_roster_not_canonical_or_ordered",
                    problems,
                )
                compact_storage = compact_details.get("stream_storage")
                compact_storage = (
                    compact_storage if isinstance(compact_storage, dict) else {}
                )
                require(
                    set(compact_storage)
                    == {
                        "compression",
                        "stored_bytes",
                        "stored_sha256",
                        "decoded_bytes",
                        "decoded_sha256",
                    }
                    and compact_storage.get("compression")
                    == manifest_storage.get("compression")
                    and compact_storage.get("stored_bytes") == raw_stream_bytes
                    and compact_storage.get("stored_sha256") == raw_stream_sha256
                    and compact_storage.get("decoded_bytes")
                    == manifest_storage.get("decoded_bytes")
                    and compact_storage.get("decoded_sha256")
                    == manifest_storage.get("decoded_sha256"),
                    "independent_compact_gate_stream_storage_unbound",
                    problems,
                )
                replay_verify = compact_details.get("replay_verify")
                replay_verify = (
                    replay_verify if isinstance(replay_verify, dict) else {}
                )
                replay_name_for_binding = manifest.get("replay_file")
                replay_path_for_binding = (
                    (run_dir / replay_name_for_binding).resolve()
                    if isinstance(replay_name_for_binding, str)
                    else run_dir / "__missing_replay__"
                )
                replay_entries = manifest.get("replay_entries")
                replay_entries = (
                    replay_entries if isinstance(replay_entries, dict) else {}
                )
                replay_game_id = manifest.get("replay_game_id")
                expected_container_stream = (
                    f"payload/{replay_game_id}/{replay_entries.get('stream')}"
                    if isinstance(replay_game_id, str)
                    and isinstance(replay_entries.get("stream"), str)
                    else None
                )
                expected_container_manifest = (
                    f"payload/{replay_game_id}/"
                    f"{replay_entries.get('session_manifest')}"
                    if isinstance(replay_game_id, str)
                    and isinstance(replay_entries.get("session_manifest"), str)
                    else None
                )
                replay_file_value = replay_verify.get("file")
                # The gate states WHICH proof certified the recording. A
                # packaged container is byte-compared against its own stream
                # entry; a cache-only container carries none, so its source
                # identity is the one the recorder manifest, H4IX and H4CP all
                # agree on. Absence means "written before the gate stated its
                # path", and every such report was byte-compared.
                certification_path = replay_verify.get(
                    "stream_certification_path",
                    "byte_compared_against_packaged_stream",
                )
                if certification_path == "byte_compared_against_packaged_stream":
                    source_binding = (
                        replay_verify.get("collector_compact_stream")
                        == expected_container_stream
                        and replay_verify.get("h4cr_stream_digest_bound") is True
                        and replay_verify.get("collector_compact_stream_bytes")
                        == manifest_storage.get("decoded_bytes")
                        and replay_verify.get("collector_compact_stream_sha256")
                        == manifest_storage.get("decoded_sha256")
                        and replay_verify.get("independently_observed_stream_sha256")
                        == manifest_storage.get("decoded_sha256")
                    )
                elif certification_path == "manifest_attested_through_delta_cache":
                    source_binding = (
                        # A cache-only recording names no packaged stream, in
                        # the manifest or in the gate report.
                        expected_container_stream is None
                        and replay_verify.get("collector_compact_stream") is None
                        and replay_verify.get("h4cr_stream_bytes_compared") is False
                        and replay_verify.get("h4cr_identity_is_manifest_attested")
                        is True
                        and replay_verify.get("attested_source_bytes")
                        == manifest_storage.get("decoded_bytes")
                        and replay_verify.get("attested_source_sha256")
                        == manifest_storage.get("decoded_sha256")
                    )
                else:
                    # Fail closed on a spelling this reader cannot describe.
                    source_binding = False
                require(
                    isinstance(replay_file_value, str)
                    and Path(replay_file_value).resolve() == replay_path_for_binding
                    and replay_verify.get("game_id") == replay_game_id
                    and replay_entries.get("stream_role")
                    == "collector_compact_stream"
                    and replay_entries.get("session_manifest_role")
                    == "recorder_manifest"
                    and replay_verify.get("recorder_manifest")
                    == expected_container_manifest
                    # `recorder_manifest_bytes`/`_sha256` are the identity of
                    # the SEALED copy inside the container. They are NOT the
                    # identity of the run-dir file: the recorder redacts
                    # absolute paths out of the sealed copy and writes finalize
                    # telemetry back into the run-dir copy after the container
                    # exists, so the two files are deliberately different (t31:
                    # 77 bytes apart). Comparing the sealed identity against
                    # the local file was only ever satisfiable because the gate
                    # demanded byte-identity, which no live capture met. The
                    # run-dir file is checked against the field that actually
                    # describes it.
                    and replay_verify.get("external_manifest_bytes")
                    == manifest_path.stat().st_size
                    and replay_verify.get("external_manifest_sha256")
                    == sha256(manifest_path)
                    and replay_verify.get(
                        "external_manifest_bound_to_sealed_authority"
                    )
                    is True
                    and source_binding,
                    "independent_compact_gate_replay_authority_unbound",
                    problems,
                )

    acceptance = manifest.get("acceptance_telemetry")
    acceptance = acceptance if isinstance(acceptance, dict) else {}
    require(acceptance.get("schema") == "battle-plan.acceptance_telemetry.v1",
            "acceptance_schema_invalid", problems)
    require(acceptance.get("complete") is True, "acceptance_not_complete", problems)
    require(acceptance.get("problems") == [], "acceptance_problems_present", problems)
    sink = acceptance.get("sink_write_us")
    sink = sink if isinstance(sink, dict) else {}
    require(is_int(sink.get("count")) and sink["count"] > 0,
            "host_sink_write_samples_empty", problems)
    require(is_finite_number(sink.get("avg")) and sink["avg"] >= 0,
            "host_sink_write_average_invalid", problems)
    require(is_int(sink.get("max")) and sink["max"] >= 0,
            "host_sink_write_max_invalid", problems)
    clock = acceptance.get("clock_skew")
    clock = clock if isinstance(clock, dict) else {}
    require(clock.get("wall_monotonic_ok") is True,
            "wall_clock_non_monotonic_or_unmeasured", problems)
    require(nested(manifest, "hourly_requirement", "met") is True,
            "hourly_requirement_not_met", problems)

    replay_name = manifest.get("replay_file")
    require(isinstance(replay_name, str) and Path(replay_name).name == replay_name,
            "replay_file_name_invalid", problems)
    if isinstance(replay_name, str) and Path(replay_name).name == replay_name:
        require((run_dir / replay_name).is_file(), "replay_file_missing", problems)
    finalize = manifest.get("host_finalize")
    finalize = finalize if isinstance(finalize, dict) else {}
    require(bool(finalize), "host_finalize_missing", problems)
    require(nested(finalize, "fold", "skipped") is not True,
            "host_fold_skipped", problems)
    require(nested(finalize, "fold", "truncated_tail_bytes") == 0,
            "host_fold_truncated_tail", problems)
    require(nested(finalize, "verify", "stream_sha256_matches_capture") is True,
            "host_verify_stream_hash_mismatch", problems)
    require(not (run_dir / "collector_sink_failure.json").exists(),
            "failure_receipt_present_in_success_run", problems)
    return {
        "manifest": str(manifest_path),
        "manifest_sha256": sha256(manifest_path),
        "production_capture_plan": str(plan_path),
        "production_capture_plan_sha256": sha256(plan_path),
        "speed5_proof_sha256": sha256(speed_path),
    }


def grade_failure(run_dir: Path, expected: str, problems: list[str]) -> dict[str, Any]:
    receipt_path = run_dir / "collector_sink_failure.json"
    plan_path = run_dir / "sink_failure_acceptance_plan.json"
    require(receipt_path.is_file(), "sink_failure_receipt_missing", problems)
    require(plan_path.is_file(), "sink_failure_acceptance_plan_missing", problems)
    if not receipt_path.is_file() or not plan_path.is_file():
        return {"receipt": None, "plan": None}
    receipt = load_object(receipt_path)
    plan = load_object(plan_path)
    require(receipt.get("schema") == "extractor.authenticated-sink-failure.v2",
            "sink_failure_schema_invalid", problems)
    require(plan.get("schema") == "extractor.sink-failure-acceptance-plan.v1",
            "sink_failure_plan_schema_invalid", problems)
    require(plan.get("admissible_for_production") is False,
            "failure_plan_claimed_production", problems)
    require(receipt.get("admissible_for_production") is False,
            "failure_receipt_claimed_production", problems)
    require(receipt.get("attempt_terminal") is True, "failure_not_terminal", problems)
    require(receipt.get("authenticated_h4ct_session") is True,
            "failure_h4ct_not_authenticated", problems)
    require(receipt.get("replay_created") is False,
            "failure_claimed_replay", problems)
    acceptance = receipt.get("acceptance_test")
    acceptance = acceptance if isinstance(acceptance, dict) else {}
    require(acceptance.get("mode") == expected, "failure_mode_mismatch", problems)
    require(acceptance.get("report_matches_armed_fault") is True,
            "armed_fault_report_mismatch", problems)
    require(is_hex64(acceptance.get("nonce")), "failure_nonce_invalid", problems)
    require(acceptance.get("acceptance_plan_sha256") == sha256(plan_path),
            "failure_plan_hash_mismatch", problems)
    require(plan.get("mode") == expected, "plan_mode_mismatch", problems)
    for key in ("session_id", "capture_id", "process_id"):
        require(plan.get(key) == receipt.get(key), f"plan_{key}_mismatch", problems)
    require(plan.get("game_executable_sha256") == EXACT_A729_SHA256,
            "plan_game_executable_not_exact_a729", problems)
    require(receipt.get("requested_bits") == 2, "failure_not_sole_pipe", problems)
    require(receipt.get("failed_bits") == 2, "failed_sink_not_pipe", problems)
    require(receipt.get("file_bytes_committed") == 0,
            "failure_file_bytes_nonzero", problems)
    if expected == "sink-open-fault":
        require(receipt.get("kind") == "Setup", "open_fault_kind_invalid", problems)
        require(receipt.get("opened_bits") == 0, "open_fault_claimed_open", problems)
    else:
        require(receipt.get("kind") == "Terminal", "write_fault_kind_invalid", problems)
        require(receipt.get("opened_bits") == 2, "write_fault_pipe_not_open", problems)
        last = receipt.get("last_complete_sequence")
        failed = receipt.get("failed_sequence")
        require(is_int(last) and is_int(failed) and last + 1 == failed,
                "write_fault_sequence_progress_invalid", problems)
    require(not (run_dir / "native_session_manifest.json").exists(),
            "failure_run_published_session_manifest", problems)
    require(not any(run_dir.glob("*.battleplan")),
            "failure_run_published_replay", problems)
    return {
        "receipt": str(receipt_path),
        "receipt_sha256": sha256(receipt_path),
        "plan": str(plan_path),
        "plan_sha256": sha256(plan_path),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--run-dir", required=True, type=Path)
    parser.add_argument(
        "--expect",
        required=True,
        choices=("success", "sink-open-fault", "sink-write-fault"),
    )
    parser.add_argument("--out", type=Path)
    args = parser.parse_args()
    run_dir = args.run_dir.resolve()
    problems: list[str] = []
    evidence = (
        grade_success(run_dir, problems)
        if args.expect == "success"
        else grade_failure(run_dir, args.expect, problems)
    )
    result = {
        "schema": "extractor.sink-reconciliation-verification.v2",
        "run_dir": str(run_dir),
        "expected_outcome": args.expect,
        "verdict": "passed" if not problems else "failed",
        "problems": problems,
        "evidence": evidence,
    }
    encoded = json.dumps(result, indent=2, sort_keys=True) + "\n"
    if args.out:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(encoded, encoding="utf-8")
    print(encoded, end="")
    return 0 if not problems else 1


if __name__ == "__main__":
    raise SystemExit(main())
