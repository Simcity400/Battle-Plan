#!/usr/bin/env python3
"""Shared deterministic I/O and central-validation checks for production seals."""

from __future__ import annotations

import hashlib
import json
import os
import uuid
from pathlib import Path
from typing import Any, Callable


TEXT_SUFFIXES = {
    ".css",
    ".csv",
    ".glsl",
    ".h",
    ".hpp",
    ".html",
    ".hlsl",
    ".js",
    ".json",
    ".jsx",
    ".md",
    ".ps1",
    ".py",
    ".ron",
    ".rs",
    ".toml",
    ".ts",
    ".tsx",
    ".txt",
    ".wgsl",
    ".yaml",
    ".yml",
}

CENTRAL_ATTESTATION_REL = (
    "replay_interface/re_evidence/rendering_laws/M01/runtime_admission/"
    "central_validation_attestation.json"
)
CENTRAL_SCHEMA = "hoi4.m01.runtime-admission.central-rust-validation-attestation.v1"
CENTRAL_RUNNER_REL = "extractor/native/scripts/Invoke-CentralRustValidation.ps1"


def canonical_bytes(path: Path) -> bytes:
    """Hash repository text with canonical LF; preserve binary bytes exactly."""
    raw = path.read_bytes()
    if path.suffix.lower() not in TEXT_SUFFIXES:
        return raw
    text = raw.decode("utf-8")
    return text.replace("\r\n", "\n").replace("\r", "\n").encode("utf-8")


def repo_sha256(path: Path) -> str:
    return hashlib.sha256(canonical_bytes(path)).hexdigest()


def repo_identity(path: Path) -> dict[str, Any]:
    payload = canonical_bytes(path)
    return {"bytes": len(payload), "sha256": hashlib.sha256(payload).hexdigest()}


def canonical_json_bytes(value: Any) -> bytes:
    return (json.dumps(value, indent=2, sort_keys=True) + "\n").encode("utf-8")


def atomic_write_bytes(path: Path, payload: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.tmp-{os.getpid()}-{uuid.uuid4().hex}")
    try:
        with temporary.open("xb") as handle:
            handle.write(payload)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def atomic_write_json(path: Path, value: Any) -> None:
    atomic_write_bytes(path, canonical_json_bytes(value))


def write_or_check_json(path: Path, value: Any, *, refresh: bool) -> None:
    expected = canonical_json_bytes(value)
    if refresh:
        atomic_write_bytes(path, expected)
        return
    if not path.is_file() or canonical_bytes(path) != expected:
        raise AssertionError(f"generated artifact drift: {path}")


def canonical_sha256(value: object) -> bool:
    return (
        isinstance(value, str)
        and len(value) == 64
        and set(value) <= set("0123456789abcdef")
    )


def central_binding(root: Path) -> dict[str, Any]:
    path = root / CENTRAL_ATTESTATION_REL
    attestation = json.loads(path.read_text(encoding="utf-8"))
    return {
        "path": CENTRAL_ATTESTATION_REL,
        "sha256": repo_sha256(path),
        "result": "PASS_FINAL_ALL",
        "output_sha256": attestation["output_sha256"],
    }


def verify_central_attestation(
    root: Path,
    binding: dict[str, Any],
    checks: list[dict[str, str]],
    require: Callable[[bool, str, list[dict[str, str]]], None],
) -> str:
    require(binding.get("path") == CENTRAL_ATTESTATION_REL, "central attestation path", checks)
    path = root / CENTRAL_ATTESTATION_REL
    actual_hash = repo_sha256(path)
    require(binding.get("sha256") == actual_hash, "central attestation canonical hash", checks)
    attestation = json.loads(path.read_text(encoding="utf-8"))
    require(attestation.get("schema") == CENTRAL_SCHEMA, "central attestation schema", checks)
    require(attestation.get("status") == "PASS", "central validation passed", checks)
    invocation = attestation.get("invocation", {})
    require(
        invocation.get("mode") == "Final" and invocation.get("workspace") == "All",
        "central validation is Final/All",
        checks,
    )
    require(
        invocation.get("package_selection") == []
        and invocation.get("workspace_package_selection") == ["--workspace"],
        "central validation covers complete workspaces",
        checks,
    )
    require(
        invocation.get("target_directory_owner") == "central_runner"
        and invocation.get("direct_cargo_invocation") is False
        and invocation.get("target_directory_override") is False,
        "central runner exclusively owns Cargo execution",
        checks,
    )
    require(
        attestation.get("exit_code") == 0
        and attestation.get("inputs_stable_across_validation") is True,
        "central validation completed against stable inputs",
        checks,
    )
    require(
        type(attestation.get("output_bytes")) is int
        and attestation["output_bytes"] > 0
        and canonical_sha256(attestation.get("output_sha256"))
        and binding.get("output_sha256") == attestation.get("output_sha256")
        and binding.get("result") == "PASS_FINAL_ALL",
        "central validation output identity",
        checks,
    )
    runner = attestation.get("runner", {})
    runner_identity = repo_identity(root / CENTRAL_RUNNER_REL)
    require(
        runner.get("path") == CENTRAL_RUNNER_REL
        and runner.get("bytes") == runner_identity["bytes"]
        and runner.get("sha256") == runner_identity["sha256"],
        "central runner canonical identity",
        checks,
    )
    m01 = attestation.get("m01_runtime_admission", {})
    expected = {
        "source": "replay_interface/native/map_viewer/src/m01_runtime_admission.rs",
        "tests": "replay_interface/native/map_viewer/tests/m01_runtime_admission.rs",
        "contract": "replay_interface/re_evidence/rendering_laws/M01/runtime_admission/promotion_contract.json",
        "frozen_discriminator": "replay_interface/re_evidence/rendering_laws/M01/assist_unit_pass_identity/minimal_discriminator.json",
    }
    for key, relative in expected.items():
        observed = m01.get(key, {})
        identity = repo_identity(root / relative)
        require(
            observed.get("path") == relative
            and observed.get("bytes") == identity["bytes"]
            and observed.get("sha256") == identity["sha256"],
            f"central attestation {key} canonical identity",
            checks,
        )
    return actual_hash
