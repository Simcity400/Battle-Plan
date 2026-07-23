#!/usr/bin/env python3
"""Strict product support policy for content-addressed the game modsets.

Integrity and support are separate decisions.  The static-bundle stack proves
that a product-modset key describes the exact executable, mod/DLC/load order,
and effective VFS.  This module additionally requires that exact key and every
binding component to appear in the repository-owned support registry.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import tempfile
from pathlib import Path
from typing import Any

import static_bundle as SB


POLICY_SCHEMA = "extractor.supported_product_modsets.v1"
ADMISSION_SCHEMA = "extractor.product_modset_policy_admission.v1"
DEFAULT_POLICY = Path(__file__).resolve().with_name("supported_product_modsets.json")
CANONICAL_POLICY_SHA256 = (
    "33ab5499109236d23fbdef656b66a74dd8eb820ccc12dd7ec046f0b51a71adb3"
)
CLASSIFICATIONS = {"vanilla", "validated_modset"}


def _write_new_atomic(path: Path, payload: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.exists():
        raise SB.BundleError("product-modset policy output already exists")
    handle = tempfile.NamedTemporaryFile(
        mode="wb", prefix=f".{path.name}.", suffix=".tmp", dir=path.parent, delete=False
    )
    temporary = Path(handle.name)
    try:
        with handle:
            handle.write(payload)
            handle.flush()
            os.fsync(handle.fileno())
        # Hard-link publication is create-if-absent on every supported host;
        # it cannot replace a planted or concurrently published receipt.
        os.link(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def _enabled_mod_ids(bindings: dict[str, dict[str, Any]]) -> list[str]:
    return sorted(
        row["id"]
        for row in bindings["mod_state"]["items"]
        if row["enabled"]
    )


def load_policy(
    path: Path = DEFAULT_POLICY, *, allow_dev_policy_override: bool = False
) -> dict[str, Any]:
    """Load and structurally validate the exact-match support registry."""
    resolved = path.resolve()
    policy_sha256 = SB.sha256_file(resolved)
    if not allow_dev_policy_override and policy_sha256 != CANONICAL_POLICY_SHA256:
        raise SB.BundleError(
            "product-modset policy is not the canonical repository registry digest; "
            "use --allow-dev-policy-override only for explicit tests/development"
        )
    value = SB.load_object(resolved, "supported product-modset policy")
    if set(value) != {"schema", "policy", "entries"}:
        raise SB.BundleError(
            "supported product-modset policy contains unknown or missing fields"
        )
    if value.get("schema") != POLICY_SCHEMA or value.get("policy") != "exact_registry_match":
        raise SB.BundleError("unsupported product-modset admission policy")
    entries = value.get("entries")
    if not isinstance(entries, list) or not entries:
        raise SB.BundleError("supported product-modset registry must not be empty")

    seen_ids: set[str] = set()
    seen_keys: set[str] = set()
    normalized = []
    for index, row in enumerate(entries):
        if not isinstance(row, dict) or set(row) != {
            "id", "classification", "product_modset"
        }:
            raise SB.BundleError(
                f"supported product-modset entry {index} is malformed"
            )
        entry_id = SB._required_string(row.get("id"), f"policy entries[{index}].id")
        classification = row.get("classification")
        if classification not in CLASSIFICATIONS:
            raise SB.BundleError(
                f"supported product-modset entry {entry_id!r} has invalid classification"
            )
        product = SB.validate_product_modset_binding(row.get("product_modset"))
        key = product["product_modset_sha256"]
        if entry_id in seen_ids or key in seen_keys:
            raise SB.BundleError("supported product-modset registry has duplicate id or key")
        seen_ids.add(entry_id)
        seen_keys.add(key)
        normalized.append(
            {"id": entry_id, "classification": classification, "product_modset": product}
        )
    return {
        "path": resolved,
        "sha256": policy_sha256,
        "entries": normalized,
    }


def admit(
    product_modset: dict[str, Any],
    bindings: dict[str, dict[str, Any]],
    policy_path: Path = DEFAULT_POLICY,
    *,
    allow_dev_policy_override: bool = False,
) -> dict[str, Any]:
    """Admit only an exact registry entry and enforce its vanilla/modded class."""
    product = SB.validate_product_modset_binding(product_modset)
    policy = load_policy(
        policy_path, allow_dev_policy_override=allow_dev_policy_override
    )
    matches = [
        row for row in policy["entries"] if row["product_modset"] == product
    ]
    if len(matches) != 1:
        raise SB.BundleError(
            "product modset is not an explicitly supported exact content address"
        )
    row = matches[0]
    enabled_mods = _enabled_mod_ids(bindings)
    load_order_mods = sorted(
        item["id"] for item in bindings["load_order"]["items"] if item["kind"] == "mod"
    )
    if load_order_mods != enabled_mods:
        raise SB.BundleError("supported product-modset mod roster is internally inconsistent")
    if row["classification"] == "vanilla" and enabled_mods:
        raise SB.BundleError("vanilla product policy cannot admit enabled mods")
    if row["classification"] == "validated_modset" and not enabled_mods:
        raise SB.BundleError("validated-modset policy entry has no enabled mods")
    return {
        "schema": ADMISSION_SCHEMA,
        "policy": "exact_registry_match",
        "policy_sha256": policy["sha256"],
        "entry_id": row["id"],
        "classification": row["classification"],
        "product_modset_sha256": product["product_modset_sha256"],
        "enabled_mod_ids": enabled_mods,
    }


def admit_static_bundle(
    manifest_path: Path,
    mod_state_path: Path,
    dlc_state_path: Path,
    load_order_path: Path,
    source_inventory_path: Path,
    policy_path: Path = DEFAULT_POLICY,
    *,
    allow_dev_policy_override: bool = False,
) -> dict[str, Any]:
    """Reconstruct the support decision directly from static-bundle inputs."""
    manifest = SB.load_object(manifest_path, "static bundle manifest")
    if manifest.get("schema") != SB.BUNDLE_SCHEMA:
        raise SB.BundleError("unsupported static bundle schema for product policy")
    bindings = {
        role: SB._validate_binding_document(SB.load_object(path, role), role)
        for role, path in (
            ("mod_state", mod_state_path),
            ("dlc_state", dlc_state_path),
            ("load_order", load_order_path),
        )
    }
    SB._validate_binding_identity(bindings)
    source_inventory = SB.load_source_inventory(source_inventory_path)
    if source_inventory["identity"] != bindings["mod_state"]["identity"]:
        raise SB.BundleError("product policy source inventory identity mismatch")
    SB.validate_source_inventory_bindings(source_inventory, bindings)
    manifest_bindings = manifest.get("bindings")
    if not isinstance(manifest_bindings, dict):
        raise SB.BundleError("static bundle has no product-modset binding")
    product = SB.validate_product_modset_binding(manifest_bindings.get("product_modset"))
    reconstructed = SB.product_modset_binding(
        product["executable_sha256"], bindings, source_inventory
    )
    if product != reconstructed:
        raise SB.BundleError(
            "static bundle product-modset binding differs from live static inputs"
        )
    return admit(
        product, bindings, policy_path,
        allow_dev_policy_override=allow_dev_policy_override,
    )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--mod-state", required=True, type=Path)
    parser.add_argument("--dlc-state", required=True, type=Path)
    parser.add_argument("--load-order", required=True, type=Path)
    parser.add_argument("--source-inventory", required=True, type=Path)
    parser.add_argument("--policy", type=Path, default=DEFAULT_POLICY)
    parser.add_argument(
        "--allow-dev-policy-override", action="store_true",
        help="allow a noncanonical policy digest for explicit tests/development",
    )
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args(argv)
    try:
        receipt = admit_static_bundle(
            args.manifest, args.mod_state, args.dlc_state, args.load_order,
            args.source_inventory, args.policy,
            allow_dev_policy_override=args.allow_dev_policy_override,
        )
        encoded = json.dumps(receipt, ensure_ascii=False, sort_keys=True, indent=2) + "\n"
        _write_new_atomic(args.out, encoded.encode("utf-8"))
    except (OSError, json.JSONDecodeError, SB.BundleError) as exc:
        print(f"product-modset policy admission: FAIL: {exc}", file=sys.stderr)
        return 1
    print(encoded, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
