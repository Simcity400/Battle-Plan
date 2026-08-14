#!/usr/bin/env python3
"""Provenance-derived product support policy for the game modsets.

Integrity and support are separate decisions.  The static-bundle stack proves
that a product-modset key describes the exact executable, mod/DLC/load order,
and effective VFS.  This module decides whether that content address was
produced by a complete, self-consistent LIVE RESOLUTION -- the fail-closed
chain in ``live_binding_resolver`` that reads the engine's own activated
product roster, the engine-authenticated VFS layer receipt, and the reproduced
a729 mount plan -- and admits it on that provenance alone.

The policy file therefore pins the RESOLUTION MECHANISM (which document
schemas, which engine build, which proof obligations) and never a specific
content address.  Any player's modset admits when its own resolution proves
itself; nothing about one machine's installed products, DLC entitlements or
mod selection is baked into this repository.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import tempfile
from pathlib import Path
from typing import Any

import live_binding_resolver as LR
import static_bundle as SB


POLICY_SCHEMA = "extractor.product_modset_admission_policy.v2"
ADMISSION_SCHEMA = "extractor.product_modset_policy_admission.v2"
POLICY_NAME = "live_resolution_provenance"
DEFAULT_POLICY = Path(__file__).resolve().with_name("product_modset_admission_policy.json")
CANONICAL_POLICY_SHA256 = (
    "3554f6840bc8bdd32a96f234bf49102f1652f2eeafc79ac935ae590d0cfa0e37"
)

# The exact obligations this module enforces.  A policy that names a different
# set -- more, fewer, or reordered -- is refused, so admission can never be
# quietly weakened by editing only one of the two files.
IMPLEMENTED_PROVENANCE: tuple[str, ...] = (
    "resolution_mechanism_domain",
    "engine_activated_dlc_roster",
    "engine_activated_mod_roster",
    "engine_mount_order_reproduced",
    "effective_vfs_namespace_coverage",
    "binding_identity_agreement",
    "product_modset_key_recomputed",
)
CLASSIFICATIONS = ("vanilla", "modded")
# The engine build whose mount planner live_binding_resolver reproduces.  A
# policy naming a different build is refused: the planner reproduction, and
# therefore every engine-order obligation below, is only proved for this build.
IMPLEMENTED_ENGINE_BUILD_ID = "a729"
WIRE_COLLECTION = {"mod_state": "mods", "dlc_state": "dlc", "load_order": "entries"}


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


def _sorted_ids(values: Any) -> list[str]:
    return sorted(values, key=LR._engine_byte_key)


def _string_list(value: Any, field: str) -> list[str]:
    if not isinstance(value, list) or not all(
        isinstance(item, str) and item.strip() for item in value
    ):
        raise SB.BundleError(f"{field} must be an array of non-empty strings")
    if len(value) != len(set(value)):
        raise SB.BundleError(f"{field} contains duplicates")
    return list(value)


def normalize_policy(value: dict[str, Any]) -> dict[str, Any]:
    """Structurally validate one mechanism policy document.

    Every declared schema must equal the constant this stack actually
    implements, so a mechanism change (a new source-inventory version, a new
    binding document) cannot be admitted until the policy is deliberately
    re-authored.
    """
    if not isinstance(value, dict) or set(value) != {
        "schema", "policy", "resolution_mechanism", "required_provenance"
    }:
        raise SB.BundleError(
            "product-modset admission policy contains unknown or missing fields"
        )
    if value.get("schema") != POLICY_SCHEMA or value.get("policy") != POLICY_NAME:
        raise SB.BundleError("unsupported product-modset admission policy")
    mechanism = value.get("resolution_mechanism")
    if not isinstance(mechanism, dict) or set(mechanism) != {
        "engine_build_id",
        "resolution_report_schema",
        "source_inventory_schema",
        "product_modset_key_schema",
        "binding_schemas",
    }:
        raise SB.BundleError(
            "product-modset admission policy resolution mechanism is malformed"
        )
    engine_build_id = SB._required_string(
        mechanism.get("engine_build_id"), "policy resolution_mechanism.engine_build_id"
    )
    if engine_build_id != IMPLEMENTED_ENGINE_BUILD_ID:
        raise SB.BundleError(
            "policy resolution_mechanism.engine_build_id does not name the implemented "
            "engine build"
        )
    expected_schemas = {
        "resolution_report_schema": LR.RESOLUTION_SCHEMA,
        "source_inventory_schema": SB.SOURCE_INVENTORY_SCHEMA,
        "product_modset_key_schema": SB.PRODUCT_MODSET_SCHEMA,
    }
    for field, expected in expected_schemas.items():
        if mechanism.get(field) != expected:
            raise SB.BundleError(
                f"policy resolution_mechanism.{field} does not name the implemented schema"
            )
    binding_schemas = mechanism.get("binding_schemas")
    implemented_bindings = {
        "mod_state": SB.MOD_STATE_SCHEMA,
        "dlc_state": SB.DLC_STATE_SCHEMA,
        "load_order": SB.LOAD_ORDER_SCHEMA,
    }
    if binding_schemas != implemented_bindings:
        raise SB.BundleError(
            "policy resolution_mechanism.binding_schemas does not name the implemented schemas"
        )
    if value.get("required_provenance") != list(IMPLEMENTED_PROVENANCE):
        raise SB.BundleError(
            "policy required_provenance is not the exact implemented obligation list"
        )
    return {
        "policy": POLICY_NAME,
        "engine_build_id": engine_build_id,
        "binding_schemas": dict(implemented_bindings),
        "required_provenance": list(IMPLEMENTED_PROVENANCE),
    }


def load_policy(path: Path = DEFAULT_POLICY) -> dict[str, Any]:
    """Load, digest-pin and structurally validate the mechanism policy."""
    resolved = path.resolve()
    policy_sha256 = SB.sha256_file(resolved)
    if policy_sha256 != CANONICAL_POLICY_SHA256:
        raise SB.BundleError(
            "product-modset admission policy is not the canonical repository digest"
        )
    value = SB.load_object(resolved, "product-modset admission policy")
    return {
        "path": resolved,
        "sha256": policy_sha256,
        **normalize_policy(value),
    }


def _require_normalized_binding(
    bindings: dict[str, Any], role: str, schema: str
) -> dict[str, Any]:
    """Re-derive one normalized binding document from its own rows.

    Callers hand admission the output of ``static_bundle._validate_binding_document``.
    Accepting that on trust would let a caller present rows whose declared
    ``effective_content_sha256`` -- the value that feeds the product-modset key --
    describes something else, so the whole document is validated again from its
    own item rows and required to come back identical.
    """
    document = bindings.get(role)
    if not isinstance(document, dict) or set(document) != {
        "schema", "identity", "effective_content_sha256", "item_count", "items"
    }:
        raise SB.BundleError(f"admission requires a validated {role} document")
    if document["schema"] != schema:
        raise SB.BundleError(f"{role} document schema is not the policy mechanism schema")
    if not isinstance(document["items"], list):
        raise SB.BundleError(f"{role} document items must be an array")
    rederived = SB._validate_binding_document(
        {
            "schema": document["schema"],
            "identity": document["identity"],
            WIRE_COLLECTION[role]: document["items"],
            "effective_content_sha256": document["effective_content_sha256"],
        },
        role,
    )
    if rederived != document:
        raise SB.BundleError(
            f"{role} document does not re-derive from its own content rows"
        )
    return rederived


def _mechanism_domain(
    product: dict[str, Any],
    source_inventory: dict[str, Any],
    report: dict[str, Any],
) -> dict[str, Any]:
    """Prove the resolution came from the RE'd engine build and passed."""
    if not isinstance(report, dict):
        raise SB.BundleError("live resolution report must be an object")
    if report.get("schema") != LR.RESOLUTION_SCHEMA:
        raise SB.BundleError("live resolution report schema is not the policy mechanism schema")
    if report.get("passed") is not True:
        raise SB.BundleError("live resolution report did not pass its own fail-closed gate")
    if source_inventory.get("schema") != SB.SOURCE_INVENTORY_SCHEMA:
        raise SB.BundleError("source inventory schema is not the policy mechanism schema")
    process = report.get("process")
    if (
        not isinstance(process, dict)
        or not isinstance(process.get("pid"), int)
        or isinstance(process.get("pid"), bool)
        or process["pid"] <= 0
        or not isinstance(process.get("creation_filetime_utc"), int)
        or isinstance(process.get("creation_filetime_utc"), bool)
        or process["creation_filetime_utc"] <= 0
    ):
        raise SB.BundleError("live resolution report lacks an exact process identity")
    exe_sha256 = SB._required_sha(process.get("exe_sha256"), "resolution process.exe_sha256")
    if exe_sha256 != LR.EXPECTED_EXE_SHA256:
        raise SB.BundleError(
            "live resolution is not against the reverse-engineered engine build"
        )
    if product["executable_sha256"] != exe_sha256:
        raise SB.BundleError("product-modset executable differs from the resolved process")
    runtime = report.get("runtime")
    if not isinstance(runtime, dict) or set(runtime) != {
        "first_time", "version", "active_dlc", "active_mods"
    }:
        raise SB.BundleError("live resolution report has no engine runtime roster")
    version = SB._required_string(runtime.get("version"), "resolution runtime.version")
    active_dlc = _string_list(runtime.get("active_dlc"), "resolution runtime.active_dlc")
    active_mods = _string_list(runtime.get("active_mods"), "resolution runtime.active_mods")
    return {
        "exe_sha256": exe_sha256,
        "version": version,
        "active_dlc": active_dlc,
        "active_mods": active_mods,
    }


def _engine_activated_dlc(
    dlc_state: dict[str, Any], report: dict[str, Any], runtime: dict[str, Any]
) -> tuple[list[str], list[str]]:
    """Every admitted DLC layer is one the engine itself reported active."""
    installed_ids = {row["id"] for row in dlc_state["items"]}
    enabled = [row for row in dlc_state["items"] if row["enabled"]]
    enabled_ids = {row["id"] for row in enabled}
    if sorted(row["name"] for row in enabled) != sorted(runtime["active_dlc"]):
        raise SB.BundleError(
            "enabled DLC names are not the engine-reported active DLC roster"
        )
    effective = _string_list(
        report.get("effective_launch_dlc_ids"), "resolution effective_launch_dlc_ids"
    )
    if set(effective) != enabled_ids:
        raise SB.BundleError(
            "enabled DLC identities are not the engine-effective launch roster"
        )
    requested = _string_list(
        report.get("requested_profile_dlc_ids"), "resolution requested_profile_dlc_ids"
    )
    if not set(requested) <= installed_ids:
        raise SB.BundleError("requested DLC profile names a product that is not installed")
    conflicts = _string_list(
        report.get("launcher_dlc_conflicts"), "resolution launcher_dlc_conflicts"
    )
    # The engine's own active tree is authoritative; a launcher profile that
    # asked for less is retained as a named conflict, never as a reason to drop
    # an activated product from the recorded modset.
    if set(conflicts) != enabled_ids - set(requested):
        raise SB.BundleError(
            "recorded launcher DLC conflicts are not the engine-active products the "
            "requested profile omitted"
        )
    return _sorted_ids(enabled_ids), _sorted_ids(conflicts)


def _engine_activated_mods(
    mod_state: dict[str, Any], report: dict[str, Any], runtime: dict[str, Any]
) -> list[dict[str, Any]]:
    """Every admitted mod layer traces to an engine-activated descriptor."""
    items = mod_state["items"]
    if not all(row["enabled"] for row in items):
        raise SB.BundleError(
            "mod_state records a mod the engine did not activate; the live resolver "
            "only records engine-activated mods"
        )
    if sorted(row["name"] for row in items) != sorted(runtime["active_mods"]):
        raise SB.BundleError(
            "enabled mod names are not the engine-reported active mod roster"
        )
    sources = report.get("resolved_mod_sources")
    if not isinstance(sources, list) or not all(isinstance(row, dict) for row in sources):
        raise SB.BundleError("resolution report has no resolved mod source roster")
    if len(sources) != len(items):
        raise SB.BundleError("resolved mod source count differs from the enabled mod roster")
    ordinals = [row.get("ordinal") for row in sources]
    if any(
        not isinstance(ordinal, int) or isinstance(ordinal, bool) for ordinal in ordinals
    ) or sorted(ordinals) != list(range(len(sources))):
        raise SB.BundleError("resolved mod source engine ordinals are not contiguous from zero")
    by_id = {row["id"]: row for row in items}
    ordered = sorted(sources, key=lambda row: row["ordinal"])
    for source in ordered:
        state = by_id.get(str(source.get("id", "")).replace("\\", "/"))
        if (
            state is None
            or source.get("name") != state["name"]
            or source.get("descriptor_sha256") != state["descriptor_sha256"]
            or source.get("content_sha256") != state["content_sha256"]
        ):
            raise SB.BundleError(
                "resolved mod source identity/content differs from the mod state roster"
            )
    return ordered


def _engine_mount_order(
    load_order: dict[str, Any],
    ordered_sources: list[dict[str, Any]],
    source_inventory: dict[str, Any],
) -> list[str]:
    """The a729 planner's order must be the order the engine actually mounted."""
    entries = load_order["items"]
    if not entries or entries[0]["kind"] != "vanilla" or entries[0]["ordinal"] != 0:
        raise SB.BundleError("load order does not begin with the base vanilla layer")
    base = [
        namespace
        for namespace in source_inventory["namespaces"]
        if namespace["binding"] == "base"
    ]
    if len(base) != 1 or entries[0]["id"] != base[0]["id"]:
        raise SB.BundleError(
            "load order base layer does not name the effective-VFS base namespace"
        )
    # The base row's content_sha256 is deliberately NOT joined here.  The live
    # resolver digests it over `vanilla` minus `dlc/` only
    # (live_binding_resolver.py::resolve_live_bindings), while the base
    # namespace digests `vanilla` minus `dlc/` AND `integrated_dlc/`
    # (::recompute_source_inventory), so the two cover different file sets by
    # construction and no offline join between them exists.  It is still bound
    # into the product-modset key through load_order.effective_content_sha256.
    # See runs/ja_modset_design/RESIDUALS.txt (g) for the one-step closure.
    engine_order = [
        str(source["id"]).replace("\\", "/") for source in ordered_sources
    ]
    recorded_order = [row["id"] for row in entries if row["kind"] == "mod"]
    if recorded_order != engine_order:
        raise SB.BundleError(
            "recorded mod load order is not the engine mount order the resolver authenticated"
        )
    return engine_order


def admit(
    product_modset: dict[str, Any],
    bindings: dict[str, dict[str, Any]],
    source_inventory: dict[str, Any],
    resolution_report: dict[str, Any],
    policy_path: Path = DEFAULT_POLICY,
) -> dict[str, Any]:
    """Admit any modset whose own live resolution proves its provenance."""
    product = SB.validate_product_modset_binding(product_modset)
    policy = load_policy(policy_path)
    schemas = policy["binding_schemas"]
    documents = {
        role: _require_normalized_binding(bindings, role, schema)
        for role, schema in schemas.items()
    }
    # Re-derive the effective-VFS inventory from its own namespace and row
    # documents rather than trusting the caller's declared digests: those two
    # digests are direct inputs to the product-modset key recomputed below.
    if not isinstance(source_inventory, dict):
        raise SB.BundleError("source inventory must be an object")
    inventory = LR.validate_source_inventory_document(source_inventory)

    runtime = _mechanism_domain(product, inventory, resolution_report)
    enabled_dlc_ids, conflict_dlc_ids = _engine_activated_dlc(
        documents["dlc_state"], resolution_report, runtime
    )
    ordered_sources = _engine_activated_mods(
        documents["mod_state"], resolution_report, runtime
    )
    engine_mount_order = _engine_mount_order(
        documents["load_order"], ordered_sources, inventory
    )

    SB._validate_binding_identity(documents)
    identity = documents["mod_state"]["identity"]
    if inventory["identity"] != identity:
        raise SB.BundleError("source inventory identity does not match the bindings")
    if resolution_report.get("binding_identity") != identity:
        raise SB.BundleError("resolution report identity does not match the bindings")

    SB.validate_source_inventory_bindings(inventory, documents)
    # Descriptorless asset layers are the only mounted namespaces no product
    # roster covers, so the resolution must have named every one of them.
    declared_asset_layers = resolution_report.get("descriptorless_asset_layers")
    if not isinstance(declared_asset_layers, list) or not all(
        isinstance(row, dict) and isinstance(row.get("id"), str)
        for row in declared_asset_layers
    ):
        raise SB.BundleError("resolution report descriptorless asset roster is malformed")
    if {
        namespace["id"]
        for namespace in inventory["namespaces"]
        if namespace["binding"] == "asset_only"
    } != {row["id"] for row in declared_asset_layers}:
        raise SB.BundleError(
            "effective VFS mounts an asset layer the resolution did not declare"
        )
    if (
        resolution_report.get("source_inventory_rows") != len(inventory["rows"])
        or resolution_report.get("source_inventory_sha256")
        != inventory["inventory_sha256"]
    ):
        raise SB.BundleError(
            "resolution report does not bind the effective-VFS inventory it admitted"
        )

    recomputed = SB.product_modset_binding(
        runtime["exe_sha256"], documents, inventory
    )
    if recomputed != product:
        raise SB.BundleError(
            "product modset key is not the key of the resolved bindings and effective VFS"
        )

    enabled_mods = documents["mod_state"]["items"]
    return {
        "schema": ADMISSION_SCHEMA,
        "policy": POLICY_NAME,
        "policy_sha256": policy["sha256"],
        "engine_build_id": policy["engine_build_id"],
        "classification": "modded" if enabled_mods else "vanilla",
        "product_modset_sha256": product["product_modset_sha256"],
        "binding_identity": identity,
        # Mod DISPLAY NAMES are arbitrary player-authored text and are proved
        # against the engine roster above but deliberately never echoed here:
        # this receipt is embedded in the pointer-free capture handoff, and a
        # mod named like a path would otherwise refuse an honest player's
        # preparation.  Descriptor ids are normalized relative VFS paths.
        "enabled_mod_ids": _sorted_ids(row["id"] for row in enabled_mods),
        "engine_mount_order": engine_mount_order,
        "enabled_dlc_ids": enabled_dlc_ids,
        "launcher_profile_conflict_dlc_ids": conflict_dlc_ids,
        "satisfied_provenance": list(IMPLEMENTED_PROVENANCE),
    }


def admit_static_bundle(
    manifest_path: Path,
    mod_state_path: Path,
    dlc_state_path: Path,
    load_order_path: Path,
    source_inventory_path: Path,
    resolution_report_path: Path,
    policy_path: Path = DEFAULT_POLICY,
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
    source_inventory = LR.validate_source_inventory_document(
        SB.load_object(source_inventory_path, "live source inventory")
    )
    if source_inventory["identity"] != bindings["mod_state"]["identity"]:
        raise SB.BundleError("product policy source inventory identity mismatch")
    SB.validate_source_inventory_bindings(source_inventory, bindings)
    report = SB.load_object(resolution_report_path, "live resolution report")
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
    return admit(product, bindings, source_inventory, report, policy_path)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--mod-state", required=True, type=Path)
    parser.add_argument("--dlc-state", required=True, type=Path)
    parser.add_argument("--load-order", required=True, type=Path)
    parser.add_argument("--source-inventory", required=True, type=Path)
    parser.add_argument("--resolution-report", required=True, type=Path)
    parser.add_argument("--policy", type=Path, default=DEFAULT_POLICY)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args(argv)
    try:
        receipt = admit_static_bundle(
            args.manifest, args.mod_state, args.dlc_state, args.load_order,
            args.source_inventory, args.resolution_report, args.policy,
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
