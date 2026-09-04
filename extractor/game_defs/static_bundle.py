#!/usr/bin/env python3
"""Create and verify the fail-closed eight-artifact extractor static bundle.

The individual static gates remain responsible for parsing Clausewitz data.  This
module is the production composition boundary: it admits exactly the eight
contract-required catalogs, verifies their own proofs and store hashes, binds the
set to the executable/mod/DLC/load-order inputs, and optionally closes the
emitted-token join sweep.

No path is trusted merely because it exists.  JSON must parse, embedded and
store hashes must match, every gate report must say ``passed=true``, and all
localisation-dependent catalogs must name the exact localisation catalog bytes.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path, PurePosixPath
from typing import Any, Iterable


HERE = Path(__file__).resolve().parent
REPO_ROOT = HERE.parents[1]
BUNDLE_SCHEMA = "extractor.static_bundle.v3"
SOURCE_INVENTORY_SCHEMA = "extractor.live_binding_source_inventory.v3"
SOURCE_JOIN_SCHEMA = "extractor.static_catalog_source_join.v1"
JOIN_SCHEMA = "extractor.static_token_join.v1"
FINAL_JOIN_SCHEMA = "extractor.static_token_join_finalized.v1"
EMITTED_INVENTORY_SCHEMA = "extractor.emitted_token_inventory.v1"
OMISSION_INVENTORY_SCHEMA = "extractor.static_token_omissions.v1"
DEFER_PROVENANCE_SCHEMA = "extractor.static_token_join_deferred.v1"
MOD_STATE_SCHEMA = "extractor.mod_state.v1"
DLC_STATE_SCHEMA = "extractor.dlc_state.v1"
LOAD_ORDER_SCHEMA = "extractor.load_order.v2"
PRODUCT_MODSET_SCHEMA = "extractor.product_modset_key.v1"
HEX256 = set("0123456789abcdef")
TECH_SHARING_UNJOINED_STATUS = (
    "definition_id_candidate_only_runtime_token_is_intern_ordinal_mapping_unproven"
)
TECH_SHARING_COMPATIBILITY_STATUS = (
    "definition id is a string candidate only; the numeric runtime token is a "
    "load-order-dependent intern ordinal, not a string hash, and its mapping is unproven"
)
TECH_SHARING_SOURCE_PROOF_SCHEMA = "technology_sharing_static.source_namespace_proof.v1"
DEFINITION_SOURCE_PROOF_ROLES = frozenset({
    "templates_equipment_catalog",
    "localisation_join",
    "decisions_static_definitions",
    "idea_tech_loc_xref_static",
    "ship_definitions_static",
    "technology_sharing_static",
    "focus_defs_static",
})


@dataclass(frozen=True)
class ArtifactSpec:
    role: str
    catalog: Path
    gate_report: Path
    schema: str
    bundle_name: str
    localisation_dependent: bool = False


DEFAULT_SPECS = (
    ArtifactSpec(
        "templates_equipment_catalog",
        HERE / "out/templates_equipment_catalog/templates_equipment_catalog_live_domain59/domain59_static_catalog.json",
        HERE / "out/templates_equipment_catalog/domain59_static_gate_report.json",
        "templates_equipment_catalog_live.static_artifact.v1",
        "templates_equipment_catalog.json",
    ),
    ArtifactSpec(
        "localisation_join",
        HERE / "out/localisation_gate/static_localisation_join_r3/localisation_catalog.json",
        HERE / "out/localisation_gate/localisation_gate_report.json",
        "static_localisation_join_r3.static_artifact.v1",
        "localisation_join.json",
    ),
    ArtifactSpec(
        "idea_tech_loc_xref_static",
        HERE / "out/idea_tech_loc_xref/idea_tech_loc_xref_static/idea_tech_loc_xref_catalog.json",
        HERE / "out/idea_tech_loc_xref/idea_tech_static_gate_report.json",
        "idea_tech_loc_xref_static.static_artifact.v2",
        "idea_tech_loc_xref_static.json",
        True,
    ),
    ArtifactSpec(
        "decisions_static_definitions",
        HERE / "out/decisions_static/decisions_static_definitions/decision_definitions_catalog.json",
        HERE / "out/decisions_static/decisions_static_gate_report.json",
        "decisions_static_definitions.static_artifact.v1",
        "decisions_static_definitions.json",
        True,
    ),
    ArtifactSpec(
        "ship_definitions_static",
        HERE / "out/ship_definitions_static/ship_definitions_static/ship_definitions_catalog.json",
        HERE / "out/ship_definitions_static/ship_definitions_gate_report.json",
        "ship_definitions_static.static_artifact.v1",
        "ship_definitions_static.json",
        True,
    ),
    ArtifactSpec(
        "gfx_icon_asset_manifest_static",
        HERE / "out/gfx_icon_manifest/gfx_icon_asset_manifest_static/gfx_icon_manifest_catalog.json",
        HERE / "out/gfx_icon_manifest/gfx_icon_manifest_gate_report.json",
        "gfx_icon_asset_manifest_static.static_artifact.v2",
        "gfx_icon_asset_manifest_static.json",
    ),
    ArtifactSpec(
        "technology_sharing_static",
        HERE / "out/technology_sharing_static/technology_sharing_static/technology_sharing_catalog.json",
        HERE / "out/technology_sharing_static/technology_sharing_gate_report.json",
        "technology_sharing_static.static_artifact.v1",
        "technology_sharing_static.json",
        True,
    ),
    ArtifactSpec(
        "focus_defs_static",
        HERE / "out/focus_defs_static/focus_defs_static/focus_defs_catalog.json",
        HERE / "out/focus_defs_static/focus_defs_static_gate_report.json",
        "focus_defs_static.static_artifact.v1",
        "focus_defs_static.json",
        True,
    ),
)
EXPECTED_SPECS = {spec.role: spec for spec in DEFAULT_SPECS}


class BundleError(ValueError):
    """A production admission check failed."""


def canonical_json(value: Any) -> bytes:
    return json.dumps(
        value,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
        allow_nan=False,
    ).encode("utf-8")


def pretty_json(value: Any) -> bytes:
    return json.dumps(
        value,
        ensure_ascii=False,
        sort_keys=True,
        indent=2,
        allow_nan=False,
    ).encode("utf-8") + b"\n"


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        while block := handle.read(1024 * 1024):
            digest.update(block)
    return digest.hexdigest()


def _reject_duplicate_json_members(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    """Reject ambiguous JSON at every object nesting level.

    Python's default decoder silently keeps the last value for duplicate object
    members.  Static-bundle inputs are later consumed by independent Rust and
    container readers, so accepting that grammar would create two possible
    authorities for the same bytes.
    """
    value: dict[str, Any] = {}
    for key, item in pairs:
        if key in value:
            raise BundleError(f"duplicate JSON object member {key!r}")
        value[key] = item
    return value


def _reject_nonfinite_json_constant(value: str) -> Any:
    raise BundleError(f"non-finite JSON constant {value!r}")


def _decode_json(raw: bytes, role: str, path: Path) -> Any:
    try:
        return json.loads(
            raw.decode("utf-8"),
            object_pairs_hook=_reject_duplicate_json_members,
            parse_constant=_reject_nonfinite_json_constant,
        )
    except BundleError as exc:
        raise BundleError(f"corrupt {role}: {path}: {exc}") from exc
    except (UnicodeError, json.JSONDecodeError) as exc:
        raise BundleError(f"corrupt {role}: {path}: {exc}") from exc


def load_object(path: Path, role: str) -> dict[str, Any]:
    if not path.is_file():
        raise BundleError(f"missing {role}: {path}")
    try:
        raw = path.read_bytes()
    except OSError as exc:
        raise BundleError(f"corrupt {role}: {path}: {exc}") from exc
    value = _decode_json(raw, role, path)
    if not isinstance(value, dict):
        raise BundleError(f"invalid {role}: expected JSON object: {path}")
    return value


def _valid_sha(value: Any) -> bool:
    return (
        isinstance(value, str)
        and value == value.lower()
        and len(value) == 64
        and not (set(value) - HEX256)
    )


def _required_string(value: Any, field: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise BundleError(f"{field} must be a non-empty string")
    return value


def _required_sha(value: Any, field: str) -> str:
    if not _valid_sha(value):
        raise BundleError(f"{field} must be a lowercase SHA-256")
    return value


def _verify_catalog_self_hash(catalog: dict[str, Any], role: str) -> str:
    claimed = catalog.get("artifact_sha256")
    if not _valid_sha(claimed):
        raise BundleError(f"{role}: missing/invalid artifact_sha256")
    unhashed = dict(catalog)
    del unhashed["artifact_sha256"]
    actual = sha256_bytes(canonical_json(unhashed))
    if actual != claimed:
        raise BundleError(f"{role}: artifact self-hash mismatch: {actual} != {claimed}")
    return claimed


def _verify_static_store(catalog_path: Path, role: str) -> None:
    index_path = catalog_path.parent / "index.json"
    snapshot_path = catalog_path.parent / "snapshot_000000.json"
    index = load_object(index_path, f"{role} index")
    snapshot = load_object(snapshot_path, f"{role} snapshot")
    entries = index.get("snapshots")
    if not isinstance(entries, list):
        raise BundleError(f"{role}: index missing snapshots array")
    by_name = {entry.get("file"): entry for entry in entries if isinstance(entry, dict)}
    for path in (catalog_path, snapshot_path):
        entry = by_name.get(path.name)
        if not isinstance(entry, dict) or not _valid_sha(entry.get("sha256")):
            raise BundleError(f"{role}: index missing hashed entry for {path.name}")
        actual = sha256_file(path)
        if actual != entry["sha256"]:
            raise BundleError(f"{role}: store hash mismatch for {path.name}")
    catalog_sha = snapshot.get("catalog_sha256")
    if catalog_sha != sha256_file(catalog_path):
        raise BundleError(f"{role}: snapshot catalog_sha256 mismatch")


def validate_artifact(spec: ArtifactSpec) -> dict[str, Any]:
    catalog = load_object(spec.catalog, f"{spec.role} catalog")
    report = load_object(spec.gate_report, f"{spec.role} gate report")
    if catalog.get("schema") != spec.schema:
        raise BundleError(
            f"{spec.role}: schema mismatch: {catalog.get('schema')!r} != {spec.schema!r}"
        )
    embedded_sha = _verify_catalog_self_hash(catalog, spec.role)
    _verify_static_store(spec.catalog, spec.role)
    if report.get("passed") is not True:
        raise BundleError(f"{spec.role}: source gate did not pass")
    if report.get("artifact_sha256") != embedded_sha:
        raise BundleError(f"{spec.role}: gate report/catalog artifact hash mismatch")
    return {
        "catalog": catalog,
        "catalog_file_sha256": sha256_file(spec.catalog),
        "catalog_artifact_sha256": embedded_sha,
        "gate_report_sha256": sha256_file(spec.gate_report),
    }


def _content_digest(rows: list[dict[str, Any]]) -> str:
    return sha256_bytes(canonical_json(rows))


def _validate_binding_document(value: dict[str, Any], role: str) -> dict[str, Any]:
    schemas = {
        "mod_state": MOD_STATE_SCHEMA,
        "dlc_state": DLC_STATE_SCHEMA,
        "load_order": LOAD_ORDER_SCHEMA,
    }
    if value.get("schema") != schemas[role]:
        raise BundleError(f"{role}: schema must be {schemas[role]!r}")
    collection_name = {"mod_state": "mods", "dlc_state": "dlc", "load_order": "entries"}[role]
    if set(value) != {"schema", "identity", collection_name, "effective_content_sha256"}:
        raise BundleError(f"{role}: document contains unknown or missing fields")
    identity = value.get("identity")
    if not isinstance(identity, dict):
        raise BundleError(f"{role}: identity must be an object")
    if set(identity) != {"playset_id", "profile_id"}:
        raise BundleError(f"{role}.identity contains unknown or missing fields")
    playset_id = _required_string(identity.get("playset_id"), f"{role}.identity.playset_id")
    profile_id = _required_string(identity.get("profile_id"), f"{role}.identity.profile_id")

    collection = value.get(collection_name)
    if not isinstance(collection, list):
        raise BundleError(f"{role}.{collection_name} must be an array")
    normalized: list[dict[str, Any]] = []
    seen_ids: set[str] = set()
    for index, row in enumerate(collection):
        if not isinstance(row, dict):
            raise BundleError(f"{role}.{collection_name}[{index}] must be an object")
        item_id = _required_string(row.get("id"), f"{role}.{collection_name}[{index}].id")
        if item_id in seen_ids:
            raise BundleError(f"{role}.{collection_name}: duplicate id {item_id!r}")
        seen_ids.add(item_id)
        content_sha = _required_sha(
            row.get("content_sha256"), f"{role}.{collection_name}[{index}].content_sha256"
        )
        if role == "mod_state":
            if set(row) != {"id", "name", "descriptor_sha256", "content_sha256", "enabled"}:
                raise BundleError(f"mod_state.mods[{index}] contains unknown or missing fields")
            normalized_row = {
                "id": item_id,
                "name": _required_string(row.get("name"), f"mod_state.mods[{index}].name"),
                "descriptor_sha256": _required_sha(
                    row.get("descriptor_sha256"), f"mod_state.mods[{index}].descriptor_sha256"
                ),
                "content_sha256": content_sha,
                "enabled": row.get("enabled"),
            }
            if not isinstance(normalized_row["enabled"], bool):
                raise BundleError(f"mod_state.mods[{index}].enabled must be boolean")
        elif role == "dlc_state":
            if set(row) != {"id", "name", "content_sha256", "enabled"}:
                raise BundleError(f"dlc_state.dlc[{index}] contains unknown or missing fields")
            normalized_row = {
                "id": item_id,
                "name": _required_string(row.get("name"), f"dlc_state.dlc[{index}].name"),
                "content_sha256": content_sha,
                "enabled": row.get("enabled"),
            }
            if not isinstance(normalized_row["enabled"], bool):
                raise BundleError(f"dlc_state.dlc[{index}].enabled must be boolean")
        else:
            if set(row) != {"id", "kind", "ordinal", "content_sha256"}:
                raise BundleError(f"load_order.entries[{index}] contains unknown or missing fields")
            kind = row.get("kind")
            if kind not in {"vanilla", "dlc", "mod"}:
                raise BundleError(f"load_order.entries[{index}].kind is invalid")
            ordinal = row.get("ordinal")
            if not isinstance(ordinal, int) or isinstance(ordinal, bool) or ordinal < 0:
                raise BundleError(f"load_order.entries[{index}].ordinal must be a non-negative integer")
            normalized_row = {
                "id": item_id,
                "kind": kind,
                "ordinal": ordinal,
                "content_sha256": content_sha,
            }
        normalized.append(normalized_row)
    if role == "load_order":
        if not normalized:
            raise BundleError("load_order.entries must include at least the vanilla layer")
        ordinals = sorted(row["ordinal"] for row in normalized)
        if ordinals != list(range(len(normalized))):
            raise BundleError("load_order.entries ordinals must be unique and contiguous from zero")
        normalized.sort(key=lambda row: row["ordinal"])
    else:
        normalized.sort(key=lambda row: row["id"])
    declared = _required_sha(value.get("effective_content_sha256"), f"{role}.effective_content_sha256")
    actual = _content_digest(normalized)
    if declared != actual:
        raise BundleError(f"{role}: effective_content_sha256 does not match normalized content rows")
    return {
        "schema": schemas[role],
        "identity": {"playset_id": playset_id, "profile_id": profile_id},
        "effective_content_sha256": actual,
        "item_count": len(normalized),
        "items": normalized,
    }


def _load_binding(path: Path, role: str) -> dict[str, Any]:
    value = load_object(path, role)
    validated = _validate_binding_document(value, role)
    return {
        "role": role,
        **validated,
        "sha256": sha256_bytes(canonical_json(value)),
        "bytes_sha256": sha256_file(path),
    }


def _validate_binding_identity(bindings: dict[str, dict[str, Any]]) -> None:
    identities = [bindings[role]["identity"] for role in ("mod_state", "dlc_state", "load_order")]
    if len({identity["playset_id"] for identity in identities}) != 1:
        raise BundleError("mod/DLC/load-order playset identities do not match")
    if len({identity["profile_id"] for identity in identities}) != 1:
        raise BundleError("mod/DLC/load-order profile identities do not match")
    load_items = bindings["load_order"]["items"]
    if sum(item["kind"] == "vanilla" for item in load_items) != 1:
        raise BundleError("load_order must contain exactly one vanilla layer")
    expected = {
        ("mod", item["id"]): item["content_sha256"]
        for item in bindings["mod_state"]["items"]
        if item["enabled"]
    }
    expected.update(
        {
            ("dlc", item["id"]): item["content_sha256"]
            for item in bindings["dlc_state"]["items"]
            if item["enabled"]
        }
    )
    actual = {
        (item["kind"], item["id"]): item["content_sha256"]
        for item in load_items
        if item["kind"] != "vanilla"
    }
    if actual != expected:
        raise BundleError("load_order enabled mod/DLC identities or content hashes do not match state manifests")


def product_modset_binding(
    executable_sha256: str,
    bindings: dict[str, dict[str, Any]],
    source_inventory: dict[str, Any],
) -> dict[str, Any]:
    """Return the portable identity of one effective product modset.

    Run-local profile/playset labels and filesystem paths are deliberately not
    identity inputs.  The key is the executable plus normalized mod, DLC and
    load-order content and the complete effective-VFS namespace/row digests.
    Therefore the same selected product content has one key across machines and
    process launches, while any executable, layer, order, replacement, file or
    archive-range change produces a different key.
    """
    _validate_binding_identity(bindings)
    core = {
        "schema": PRODUCT_MODSET_SCHEMA,
        "executable_sha256": _required_sha(
            executable_sha256, "product modset executable_sha256"
        ),
        "mod_state_effective_content_sha256": _required_sha(
            bindings["mod_state"].get("effective_content_sha256"),
            "product modset mod_state_effective_content_sha256",
        ),
        "dlc_state_effective_content_sha256": _required_sha(
            bindings["dlc_state"].get("effective_content_sha256"),
            "product modset dlc_state_effective_content_sha256",
        ),
        "load_order_effective_content_sha256": _required_sha(
            bindings["load_order"].get("effective_content_sha256"),
            "product modset load_order_effective_content_sha256",
        ),
        "effective_vfs_namespace_sha256": _required_sha(
            source_inventory.get("namespace_sha256"),
            "product modset effective_vfs_namespace_sha256",
        ),
        "effective_vfs_rows_sha256": _required_sha(
            source_inventory.get("rows_sha256"),
            "product modset effective_vfs_rows_sha256",
        ),
    }
    return {
        **core,
        "product_modset_sha256": sha256_bytes(canonical_json(core)),
    }


def validate_product_modset_binding(value: Any) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise BundleError("product_modset binding must be an object")
    expected_fields = {
        "schema",
        "executable_sha256",
        "mod_state_effective_content_sha256",
        "dlc_state_effective_content_sha256",
        "load_order_effective_content_sha256",
        "effective_vfs_namespace_sha256",
        "effective_vfs_rows_sha256",
        "product_modset_sha256",
    }
    if set(value) != expected_fields or value.get("schema") != PRODUCT_MODSET_SCHEMA:
        raise BundleError("product_modset binding has unsupported schema or fields")
    core = {
        field: value[field]
        for field in expected_fields
        if field != "product_modset_sha256"
    }
    for field in expected_fields - {"schema"}:
        _required_sha(value.get(field), f"product_modset.{field}")
    if value["product_modset_sha256"] != sha256_bytes(canonical_json(core)):
        raise BundleError("product_modset binding digest mismatch")
    return value


def _iter_source_rows(value: Any) -> Iterable[dict[str, Any]]:
    if isinstance(value, dict):
        if "sha256" in value or "relpath" in value:
            relpath = _required_string(value.get("relpath"), "source inventory relpath")
            sha256 = _required_sha(value.get("sha256"), f"source inventory {relpath} sha256")
            byte_count = value.get("bytes")
            if not isinstance(byte_count, int) or isinstance(byte_count, bool) or byte_count < 0:
                raise BundleError(f"source inventory {relpath} bytes must be a non-negative integer")
            yield {
                "relpath": relpath,
                "bytes": byte_count,
                "sha256": sha256,
            }
        for child in value.values():
            yield from _iter_source_rows(child)
    elif isinstance(value, list):
        for child in value:
            yield from _iter_source_rows(child)


def _source_inventory_digest(catalogs: dict[str, dict[str, Any]]) -> tuple[str, int]:
    rows: list[dict[str, Any]] = []
    for role, catalog in catalogs.items():
        for row in _iter_source_rows(catalog.get("source_files", {})):
            rows.append({"artifact_role": role, **row})
    rows.sort(key=lambda row: (row["artifact_role"], row["relpath"], row["sha256"]))
    return sha256_bytes(canonical_json(rows)), len(rows)


def load_source_inventory(path: Path) -> dict[str, Any]:
    value = load_object(path, "live source inventory")
    required = {
        "schema", "identity", "namespaces", "rows", "namespace_sha256",
        "rows_sha256", "inventory_sha256",
    }
    if set(value) != required or value.get("schema") != SOURCE_INVENTORY_SCHEMA:
        raise BundleError("live source inventory has unsupported schema or fields")
    identity = value.get("identity")
    if not isinstance(identity, dict) or set(identity) != {"playset_id", "profile_id"}:
        raise BundleError("live source inventory identity is malformed")
    identity = {
        "playset_id": _required_string(identity.get("playset_id"), "source inventory playset_id"),
        "profile_id": _required_string(identity.get("profile_id"), "source inventory profile_id"),
    }
    namespaces = value.get("namespaces")
    rows = value.get("rows")
    if not isinstance(namespaces, list) or not namespaces or not isinstance(rows, list) or not rows:
        raise BundleError("live source inventory namespaces/rows must be non-empty arrays")
    normalized_namespaces: list[dict[str, Any]] = []
    seen_labels: set[str] = set()
    for ordinal, namespace in enumerate(namespaces):
        if not isinstance(namespace, dict) or set(namespace) != {
            "ordinal", "kind", "binding", "id", "label", "replace_paths", "file_count",
            "content_sha256", "binding_content_sha256", "storage_kind",
            "source_container_sha256", "source_container_bytes"
        }:
            raise BundleError(f"live source namespace {ordinal} is malformed")
        if namespace.get("ordinal") != ordinal or namespace.get("kind") not in {
            "vanilla", "integrated_dlc", "dlc", "mod"
        }:
            raise BundleError("live source namespace order/kind is invalid")
        if namespace.get("binding") not in {
            "base", "integrated", "active_product", "asset_only", "active_mod"
        }:
            raise BundleError("live source namespace binding class is invalid")
        label = _required_string(namespace.get("label"), f"source namespace {ordinal}.label")
        if label in seen_labels:
            raise BundleError(f"duplicate live source namespace label: {label}")
        seen_labels.add(label)
        replace_paths = namespace.get("replace_paths")
        if not isinstance(replace_paths, list) or not all(isinstance(item, str) and item for item in replace_paths):
            raise BundleError(f"source namespace {ordinal}.replace_paths is malformed")
        file_count = namespace.get("file_count")
        if not isinstance(file_count, int) or isinstance(file_count, bool) or file_count < 0:
            raise BundleError(f"source namespace {ordinal}.file_count is invalid")
        storage_kind = namespace.get("storage_kind")
        source_container_sha256 = namespace.get("source_container_sha256")
        source_container_bytes = namespace.get("source_container_bytes")
        if storage_kind == "directory":
            if source_container_sha256 is not None or source_container_bytes is not None:
                raise BundleError(f"source namespace {ordinal} directory carries container identity")
        elif storage_kind == "zip_archive":
            source_container_sha256 = _required_sha(
                source_container_sha256,
                f"source namespace {ordinal}.source_container_sha256",
            )
            if (
                not isinstance(source_container_bytes, int)
                or isinstance(source_container_bytes, bool)
                or source_container_bytes <= 0
            ):
                raise BundleError(f"source namespace {ordinal}.source_container_bytes is invalid")
        else:
            raise BundleError(f"source namespace {ordinal}.storage_kind is invalid")
        normalized_namespaces.append({
            "ordinal": ordinal,
            "kind": namespace["kind"],
            "binding": namespace["binding"],
            "id": _required_string(namespace.get("id"), f"source namespace {ordinal}.id"),
            "label": label,
            "replace_paths": replace_paths,
            "storage_kind": storage_kind,
            "source_container_sha256": source_container_sha256,
            "source_container_bytes": source_container_bytes,
            "file_count": file_count,
            "content_sha256": _required_sha(namespace.get("content_sha256"), f"source namespace {ordinal}.content_sha256"),
            "binding_content_sha256": _required_sha(
                namespace.get("binding_content_sha256"),
                f"source namespace {ordinal}.binding_content_sha256",
            ),
        })
    normalized_rows: list[dict[str, Any]] = []
    seen_rows: set[tuple[int, str]] = set()
    for index, row in enumerate(rows):
        if not isinstance(row, dict) or set(row) != {
            "namespace_ordinal", "layer_kind", "layer_id", "relpath", "bytes", "sha256",
            "storage_kind", "source_container_sha256", "source_byte_offset",
            "source_byte_count", "source_range_sha256"
        }:
            raise BundleError(f"live source inventory row {index} is malformed")
        ordinal = row.get("namespace_ordinal")
        if not isinstance(ordinal, int) or isinstance(ordinal, bool) or not 0 <= ordinal < len(namespaces):
            raise BundleError(f"live source row {index}.namespace_ordinal is invalid")
        relpath = _required_string(row.get("relpath"), f"live source row {index}.relpath")
        if (
            "\\" in relpath
            or (len(relpath) >= 2 and relpath[0].isalpha() and relpath[1] == ":")
            or PurePosixPath(relpath).is_absolute()
            or any(
            part in {"", ".", ".."} for part in PurePosixPath(relpath).parts
            )
        ):
            raise BundleError(f"live source row {index}.relpath is unsafe")
        key = (ordinal, relpath.casefold())
        if key in seen_rows:
            raise BundleError(f"duplicate/case-colliding live source row: {key}")
        seen_rows.add(key)
        namespace = normalized_namespaces[ordinal]
        if row.get("layer_kind") != namespace["kind"] or row.get("layer_id") != namespace["id"]:
            raise BundleError(f"live source row {index} namespace identity mismatch")
        byte_count = row.get("bytes")
        if not isinstance(byte_count, int) or isinstance(byte_count, bool) or byte_count < 0:
            raise BundleError(f"live source row {index}.bytes is invalid")
        payload_sha256 = _required_sha(row.get("sha256"), f"live source row {index}.sha256")
        source_byte_offset = row.get("source_byte_offset")
        source_byte_count = row.get("source_byte_count")
        if (
            not isinstance(source_byte_offset, int)
            or isinstance(source_byte_offset, bool)
            or source_byte_offset < 0
            or not isinstance(source_byte_count, int)
            or isinstance(source_byte_count, bool)
            or source_byte_count < 0
        ):
            raise BundleError(f"live source row {index} physical range is invalid")
        source_range_sha256 = _required_sha(
            row.get("source_range_sha256"),
            f"live source row {index}.source_range_sha256",
        )
        if row.get("storage_kind") != namespace["storage_kind"]:
            raise BundleError(f"live source row {index} storage kind mismatch")
        if namespace["storage_kind"] == "directory":
            if (
                row.get("source_container_sha256") is not None
                or source_byte_offset != 0
                or source_byte_count != byte_count
                or source_range_sha256 != payload_sha256
            ):
                raise BundleError(f"live source row {index} loose-file range is invalid")
        else:
            if (
                row.get("source_container_sha256") != namespace["source_container_sha256"]
                or source_byte_offset + source_byte_count > namespace["source_container_bytes"]
            ):
                raise BundleError(f"live source row {index} archive range is invalid")
        normalized_rows.append({
            "namespace_ordinal": ordinal,
            "layer_kind": namespace["kind"],
            "layer_id": namespace["id"],
            "relpath": relpath,
            "bytes": byte_count,
            "sha256": payload_sha256,
            "storage_kind": namespace["storage_kind"],
            "source_container_sha256": namespace["source_container_sha256"],
            "source_byte_offset": source_byte_offset,
            "source_byte_count": source_byte_count,
            "source_range_sha256": source_range_sha256,
        })
    for namespace in normalized_namespaces:
        ns_rows = [row for row in normalized_rows if row["namespace_ordinal"] == namespace["ordinal"]]
        if len(ns_rows) != namespace["file_count"]:
            raise BundleError("live source namespace file count mismatch")
        content_rows = [
            {"relpath": row["relpath"], "bytes": row["bytes"], "sha256": row["sha256"]}
            for row in ns_rows
        ]
        if namespace["content_sha256"] != sha256_bytes(canonical_json(content_rows)):
            raise BundleError("live source namespace content digest mismatch")
    namespace_sha = sha256_bytes(canonical_json(normalized_namespaces))
    rows_sha = sha256_bytes(canonical_json(normalized_rows))
    core = {
        "schema": SOURCE_INVENTORY_SCHEMA,
        "identity": identity,
        "namespaces": normalized_namespaces,
        "rows": normalized_rows,
        "namespace_sha256": namespace_sha,
        "rows_sha256": rows_sha,
    }
    inventory_sha = sha256_bytes(canonical_json(core))
    if (
        value.get("namespace_sha256") != namespace_sha
        or value.get("rows_sha256") != rows_sha
        or value.get("inventory_sha256") != inventory_sha
    ):
        raise BundleError("live source inventory digest mismatch")
    return {**core, "inventory_sha256": inventory_sha, "file_sha256": sha256_file(path), "bytes": path.stat().st_size}


def bind_catalog_sources(
    catalogs: dict[str, dict[str, Any]], inventory: dict[str, Any]
) -> dict[str, Any]:
    namespaces = inventory["namespaces"]
    by_label = {row["label"]: row for row in namespaces}
    by_path = {
        (row["namespace_ordinal"], row["relpath"].casefold()): row
        for row in inventory["rows"]
    }
    bindings: list[dict[str, Any]] = []
    roles: list[dict[str, Any]] = []
    for role in sorted(catalogs):
        catalog = catalogs[role]
        layers = catalog.get("layers")
        if not isinstance(layers, list) or not layers:
            raise BundleError(f"{role}: catalog layers are missing")
        ordered_namespaces: list[dict[str, Any]] = []
        for layer in layers:
            if not isinstance(layer, dict) or not isinstance(layer.get("label"), str):
                raise BundleError(f"{role}: catalog layer is malformed")
            namespace = by_label.get(layer["label"])
            if namespace is None:
                raise BundleError(f"{role}: catalog layer is absent from live source inventory: {layer['label']}")
            replace_paths = list(layer.get("replace_path") or [])
            if replace_paths != namespace["replace_paths"]:
                raise BundleError(f"{role}: replace_path mismatch for layer {layer['label']}")
            if ordered_namespaces and namespace["ordinal"] <= ordered_namespaces[-1]["ordinal"]:
                raise BundleError(f"{role}: catalog layer precedence does not match live inventory")
            ordered_namespaces.append(namespace)
        expected_namespaces = (
            namespaces
            if role == "gfx_icon_asset_manifest_static"
            else [row for row in namespaces if row["binding"] != "asset_only"]
        )
        if [row["ordinal"] for row in ordered_namespaces] != [
            row["ordinal"] for row in expected_namespaces
        ]:
            raise BundleError(
                f"{role}: catalog layers do not match the exact live definition namespace roster"
            )
        if role in DEFINITION_SOURCE_PROOF_ROLES:
            proof = catalog.get("source_namespace_proof")
            expected_selected = [
                row["label"] for row in namespaces if row["binding"] != "asset_only"
            ]
            expected_excluded = [
                row["label"] for row in namespaces if row["binding"] == "asset_only"
            ]
            if (
                not isinstance(proof, dict)
                or set(proof) != {
                    "schema",
                    "status",
                    "source_inventory_sha256",
                    "selected_namespace_labels",
                    "excluded_asset_only_namespace_labels",
                }
                or proof.get("schema") != TECH_SHARING_SOURCE_PROOF_SCHEMA
                or proof.get("status") != "bound_to_live_source_inventory"
                or proof.get("source_inventory_sha256") != inventory["inventory_sha256"]
                or proof.get("selected_namespace_labels") != expected_selected
                or proof.get("excluded_asset_only_namespace_labels") != expected_excluded
            ):
                raise BundleError(
                    f"{role}: live definition source namespace proof mismatch"
                )
        occurrences = list(_iter_source_rows(catalog.get("source_files", {})))
        if not occurrences:
            raise BundleError(f"{role}: catalog source inventory is empty")
        role_bindings = []
        for occurrence in occurrences:
            relpath = occurrence["relpath"]
            parent = Path(relpath).parent.as_posix()
            parent = "" if parent == "." else parent
            winner = None
            for namespace in ordered_namespaces:
                if parent.casefold() in {item.casefold() for item in namespace["replace_paths"]}:
                    winner = None
                candidate = by_path.get((namespace["ordinal"], relpath.casefold()))
                if candidate is not None:
                    winner = candidate
            if winner is None:
                raise BundleError(f"{role}: catalog source has no effective live namespace row: {relpath}")
            if occurrence["bytes"] != winner["bytes"] or occurrence["sha256"] != winner["sha256"]:
                raise BundleError(f"{role}: catalog source bytes do not match live namespace winner: {relpath}")
            bound = {
                "artifact_role": role,
                "relpath": relpath,
                "bytes": occurrence["bytes"],
                "sha256": occurrence["sha256"],
                "namespace_ordinal": winner["namespace_ordinal"],
                "layer_kind": winner["layer_kind"],
                "layer_id": winner["layer_id"],
            }
            bindings.append(bound)
            role_bindings.append(bound)
        roles.append({
            "role": role,
            "source_occurrence_count": len(role_bindings),
            "unique_source_count": len({
                (row["namespace_ordinal"], row["relpath"].casefold()) for row in role_bindings
            }),
            "bindings_sha256": sha256_bytes(canonical_json(role_bindings)),
        })
    report = {
        "schema": SOURCE_JOIN_SCHEMA,
        "passed": True,
        "inventory_sha256": inventory["inventory_sha256"],
        "roles": roles,
        "bindings": bindings,
    }
    report["bindings_sha256"] = sha256_bytes(canonical_json(bindings))
    return report


def join_base_layer_to_inventory(
    load_items: list[dict[str, Any]], inventory: dict[str, Any]
) -> dict[str, Any]:
    """Join the vanilla load-order row to the effective-VFS base namespace.

    IDENTITY and CONTENT are both joined.  The live resolver records the base
    namespace's own digest in the vanilla row, and
    ``live_binding_resolver.validate_source_inventory_document`` recomputes every
    namespace digest from that namespace's own inventory rows, so this row is
    reconstructible offline from the inventory alone -- it is not a second,
    differently-scoped tree hash that only the resolving machine could check.
    """
    base = [
        namespace
        for namespace in inventory["namespaces"]
        if namespace["binding"] == "base"
    ]
    if len(base) != 1:
        raise BundleError("live source inventory must declare exactly one base namespace")
    vanilla = [row for row in load_items if row["kind"] == "vanilla"]
    if len(vanilla) != 1:
        raise BundleError("load_order must contain exactly one vanilla layer")
    if vanilla[0]["id"] != base[0]["id"]:
        raise BundleError(
            "load order base layer does not name the effective-VFS base namespace"
        )
    if vanilla[0]["content_sha256"] != base[0]["content_sha256"]:
        raise BundleError(
            "load order base layer content digest is not the effective-VFS base "
            "namespace digest"
        )
    return base[0]


def validate_source_inventory_bindings(
    inventory: dict[str, Any], state_bindings: dict[str, dict[str, Any]]
) -> None:
    namespaces = inventory["namespaces"]
    if namespaces[0]["kind"] != "vanilla" or namespaces[0]["binding"] != "base":
        raise BundleError("live source inventory must begin with the base vanilla namespace")
    integrated_ordinals = [
        row["ordinal"] for row in namespaces if row["kind"] == "integrated_dlc"
    ]
    if integrated_ordinals != list(range(1, 1 + len(integrated_ordinals))):
        raise BundleError("live source inventory integrated DLC precedence is invalid")
    if any(
        row["kind"] == "integrated_dlc" and row["binding"] != "integrated"
        for row in namespaces
    ):
        raise BundleError("live source inventory integrated DLC binding is invalid")
    load_items = state_bindings["load_order"]["items"]
    join_base_layer_to_inventory(load_items, inventory)
    wanted_mod_ids = [row["id"] for row in load_items if row["kind"] == "mod"]
    wanted_dlc_ids = [row["id"] for row in load_items if row["kind"] == "dlc"]
    mod_by_id = {
        row["id"]: row for row in state_bindings["mod_state"]["items"] if row["enabled"]
    }
    dlc_by_id = {
        row["id"]: row for row in state_bindings["dlc_state"]["items"] if row["enabled"]
    }
    mod_namespaces = [row for row in namespaces if row["binding"] == "active_mod"]
    product_namespaces = [row for row in namespaces if row["binding"] == "active_product"]
    planned_namespaces = [
        (row["kind"], row["id"])
        for row in namespaces
        if row["binding"] in {"active_product", "active_mod"}
    ]
    planned_load_items = [
        (row["kind"], row["id"])
        for row in load_items
        if row["kind"] in {"dlc", "mod"}
    ]
    if planned_namespaces != planned_load_items:
        raise BundleError(
            "live source inventory combined DLC/mod order does not match load order"
        )
    if [row["id"] for row in mod_namespaces] != wanted_mod_ids:
        raise BundleError("live source inventory active mod order does not match load order")
    if [row["id"] for row in product_namespaces] != wanted_dlc_ids:
        raise BundleError("live source inventory active DLC order does not match load order")
    for namespace in mod_namespaces:
        state = mod_by_id.get(namespace["id"])
        if (
            state is None
            or namespace["label"] != state["name"]
            or namespace["binding_content_sha256"] != state["content_sha256"]
        ):
            raise BundleError(f"live source inventory mod binding mismatch: {namespace['id']}")
    for namespace in product_namespaces:
        state = dlc_by_id.get(namespace["id"])
        if state is None or namespace["binding_content_sha256"] != state["content_sha256"]:
            raise BundleError(f"live source inventory DLC binding mismatch: {namespace['id']}")
    known_dlc_ids = {row["id"] for row in state_bindings["dlc_state"]["items"]}
    for namespace in namespaces:
        expected_binding = {
            "vanilla": "base",
            "integrated_dlc": "integrated",
            "mod": "active_mod",
        }.get(namespace["kind"])
        if expected_binding is not None and namespace["binding"] != expected_binding:
            raise BundleError("live source inventory namespace binding class mismatch")
        if namespace["binding"] == "asset_only" and namespace["id"] in known_dlc_ids:
            raise BundleError(f"descriptor-backed DLC cannot be classified asset-only: {namespace['id']}")


def verify_current_source_state(
    exe: Path,
    inventory: dict[str, Any],
    live_resolution_report: Path,
) -> None:
    report = load_object(live_resolution_report, "live resolution report")
    if report.get("schema") != "extractor.live_binding_resolution.v1":
        raise BundleError("live resolution report schema is unsupported")
    if report.get("binding_identity") != inventory["identity"]:
        raise BundleError("live resolution report identity does not match source inventory")
    process = report.get("process")
    if not isinstance(process, dict) or process.get("exe_sha256") != sha256_file(exe):
        raise BundleError("live resolution report executable does not match current executable")
    if (
        not isinstance(process.get("pid"), int)
        or isinstance(process.get("pid"), bool)
        or process["pid"] <= 0
        or not isinstance(process.get("creation_filetime_utc"), int)
        or isinstance(process.get("creation_filetime_utc"), bool)
        or process["creation_filetime_utc"] <= 0
    ):
        raise BundleError("live resolution report lacks an exact process identity")
    sources = report.get("resolved_mod_sources")
    if not isinstance(sources, list) or not all(isinstance(row, dict) for row in sources):
        raise BundleError("live resolution report lacks resolved mod sources")
    active_dlc_ids = [
        row["id"] for row in inventory["namespaces"]
        if row["kind"] == "dlc" and row["binding"] == "active_product"
    ]
    extra_asset_ids = [
        row["id"] for row in inventory["namespaces"]
        if row["kind"] == "dlc" and row["binding"] == "asset_only"
    ]
    try:
        import live_binding_resolver as live_resolver

        recomputed = live_resolver.recompute_source_inventory(
            exe.resolve().parent,
            sources,
            active_dlc_ids,
            extra_asset_ids,
            inventory["identity"],
        )
    except (OSError, ValueError, KeyError) as exc:
        raise BundleError(f"current live source rehash failed: {exc}") from exc
    comparable = {
        key: value for key, value in inventory.items()
        if key not in {"file_sha256", "bytes"}
    }
    if recomputed != comparable:
        raise BundleError("current live source namespace differs from bound inventory")
    if (
        report.get("source_inventory_rows") != len(inventory["rows"])
        or report.get("source_inventory_sha256") != inventory["inventory_sha256"]
    ):
        raise BundleError("live resolution report source inventory claim mismatch")


def _lookup_sets(catalogs: dict[str, dict[str, Any]]) -> dict[str, set[str]]:
    def keys(obj: Any) -> set[str]:
        return set(obj) if isinstance(obj, dict) else set()

    templates = catalogs["templates_equipment_catalog"].get("catalog", {})
    idea_tech = catalogs["idea_tech_loc_xref_static"].get("tokens", {})
    decisions = catalogs["decisions_static_definitions"]
    ships = catalogs["ship_definitions_static"]
    gfx = catalogs["gfx_icon_asset_manifest_static"]
    focus_defs = catalogs["focus_defs_static"]
    template_rows = templates.get("division_templates", []) if isinstance(templates, dict) else []
    return {
        "localisation": keys(catalogs["localisation_join"].get("keys")),
        "idea": keys(idea_tech.get("ideas") if isinstance(idea_tech, dict) else None),
        "technology": keys(idea_tech.get("technologies") if isinstance(idea_tech, dict) else None),
        "decision": keys(decisions.get("decisions")),
        "decision_category": keys(decisions.get("categories")),
        "ship_hull": keys(ships.get("ship_hulls")),
        "ship_module": keys(ships.get("ship_modules")),
        "equipment": keys(templates.get("equipment_definitions") if isinstance(templates, dict) else None),
        "unit_definition": keys(templates.get("unit_definitions") if isinstance(templates, dict) else None),
        "division_template": {
            str(row.get("name"))
            for row in template_rows
            if isinstance(row, dict) and row.get("name") is not None
        },
        "gfx_sprite": keys(gfx.get("sprites")),
        # a recorded focus id comes from either engine database, so both count as packaged
        "focus": keys(focus_defs.get("focuses")) | keys(focus_defs.get("continuous_focuses")),
        "continuous_focus": keys(focus_defs.get("continuous_focuses")),
        "continuous_focus_palette": keys(focus_defs.get("continuous_focus_palettes")),
        "focus_tree": keys(focus_defs.get("trees")),
    }


def _validate_unjoined_technology_sharing(
    catalog: dict[str, Any], inventory: dict[str, Any] | None = None
) -> None:
    """Keep the runtime intern ordinal outside all static string-token joins."""

    join_keys = catalog.get("join_keys")
    definitions = catalog.get("definitions")
    if not isinstance(join_keys, dict) or set(join_keys) != {
        "definition_token_candidate", "runtime_hash_candidate", "compatibility_status"
    }:
        raise BundleError("technology_sharing_static: join metadata is malformed")
    if (
        join_keys.get("definition_token_candidate")
        != "definitions[*].runtime_token_candidate"
        or join_keys.get("runtime_hash_candidate") is not None
        or join_keys.get("compatibility_status") != TECH_SHARING_COMPATIBILITY_STATUS
    ):
        raise BundleError("technology_sharing_static: runtime join metadata drifted")
    if not isinstance(definitions, list) or not definitions:
        raise BundleError("technology_sharing_static: definitions must be a non-empty array")
    for index, row in enumerate(definitions):
        if not isinstance(row, dict):
            raise BundleError(f"technology_sharing_static: definition {index} is malformed")
        if (
            not isinstance(row.get("id"), str)
            or not row["id"]
            or row.get("runtime_token_candidate") != row["id"]
            or row.get("runtime_hash_candidate") is not None
            or row.get("runtime_join_status") != TECH_SHARING_UNJOINED_STATUS
        ):
            raise BundleError(
                "technology_sharing_static: numeric runtime token must remain an unjoined intern ordinal"
            )
    proof = catalog.get("source_namespace_proof")
    if not isinstance(proof, dict) or set(proof) != {
        "schema", "status", "source_inventory_sha256", "selected_namespace_labels",
        "excluded_asset_only_namespace_labels",
    }:
        raise BundleError("technology_sharing_static: source namespace proof is malformed")
    if (
        proof.get("schema") != TECH_SHARING_SOURCE_PROOF_SCHEMA
        or proof.get("status") != "bound_to_live_source_inventory"
        or not _valid_sha(proof.get("source_inventory_sha256"))
    ):
        raise BundleError("technology_sharing_static: live source namespace proof is not authoritative")
    selected = proof.get("selected_namespace_labels")
    excluded = proof.get("excluded_asset_only_namespace_labels")
    layer_labels = [
        row.get("label") for row in catalog.get("layers", []) if isinstance(row, dict)
    ]
    if (
        not isinstance(selected, list)
        or not all(isinstance(item, str) and item for item in selected)
        or not isinstance(excluded, list)
        or not all(isinstance(item, str) and item for item in excluded)
        or selected != layer_labels
    ):
        raise BundleError("technology_sharing_static: source namespace selection is malformed")
    if inventory is not None:
        expected_selected = [
            row["label"] for row in inventory["namespaces"] if row["binding"] != "asset_only"
        ]
        expected_excluded = [
            row["label"] for row in inventory["namespaces"] if row["binding"] == "asset_only"
        ]
        if (
            proof["source_inventory_sha256"] != inventory["inventory_sha256"]
            or selected != expected_selected
            or excluded != expected_excluded
        ):
            raise BundleError("technology_sharing_static: source namespace proof binding mismatch")


def _capture_provenance(value: Any, role: str) -> dict[str, str]:
    if not isinstance(value, dict):
        raise BundleError(f"{role}.provenance must be an object")
    if set(value) != {"capture_id", "session_id", "stream_sha256"}:
        raise BundleError(f"{role}.provenance contains unknown or missing fields")
    return {
        "capture_id": _required_string(value.get("capture_id"), f"{role}.provenance.capture_id"),
        "session_id": _required_string(value.get("session_id"), f"{role}.provenance.session_id"),
        "stream_sha256": _required_sha(
            value.get("stream_sha256"), f"{role}.provenance.stream_sha256"
        ),
    }


def _load_token_inventory(
    path: Path,
    role: str,
    expected_schema: str,
    *,
    require_capture_provenance: bool,
) -> tuple[list[dict[str, Any]], dict[str, Any] | None, str]:
    if not path.is_file():
        raise BundleError(f"missing {role}: {path}")
    try:
        raw = path.read_bytes()
    except OSError as exc:
        raise BundleError(f"corrupt {role}: {path}: {exc}") from exc
    value = _decode_json(raw, role, path)
    if not isinstance(value, dict):
        raise BundleError(f"invalid {role}: expected JSON object")
    if value.get("schema") != expected_schema:
        raise BundleError(f"{role}: schema must be {expected_schema!r}")
    rows = value.get("tokens")
    if not isinstance(rows, list) or not all(isinstance(row, dict) for row in rows):
        raise BundleError(f"{role}.tokens must be an array of objects")
    provenance = (
        _capture_provenance(value.get("provenance"), role)
        if require_capture_provenance
        else None
    )
    return rows, provenance, sha256_bytes(raw)


def _load_defer_provenance(path: Path) -> dict[str, str]:
    value = load_object(path, "deferred token gate provenance")
    if value.get("schema") != DEFER_PROVENANCE_SCHEMA:
        raise BundleError(
            f"deferred token gate provenance: schema must be {DEFER_PROVENANCE_SCHEMA!r}"
        )
    if set(value) != {"schema", "requested_by", "reason", "session_plan_id"}:
        raise BundleError("deferred token gate provenance contains unknown or missing fields")
    normalized = {
        "schema": DEFER_PROVENANCE_SCHEMA,
        "requested_by": _required_string(value.get("requested_by"), "defer_provenance.requested_by"),
        "reason": _required_string(value.get("reason"), "defer_provenance.reason"),
        "session_plan_id": _required_string(
            value.get("session_plan_id"), "defer_provenance.session_plan_id"
        ),
    }
    return {
        **normalized,
        "provenance_sha256": sha256_bytes(canonical_json(normalized)),
        "bytes_sha256": sha256_file(path),
    }


def account_emitted_tokens(
    catalogs: dict[str, dict[str, Any]],
    emitted_path: Path,
    omissions_path: Path | None = None,
) -> dict[str, Any]:
    emitted, provenance, emitted_sha = _load_token_inventory(
        emitted_path,
        "emitted token inventory",
        EMITTED_INVENTORY_SCHEMA,
        require_capture_provenance=True,
    )
    if not emitted:
        raise BundleError("emitted token inventory is empty; use an explicit deferred gate instead")
    if omissions_path:
        omissions, _, omissions_sha = _load_token_inventory(
            omissions_path,
            "explicit omission inventory",
            OMISSION_INVENTORY_SCHEMA,
            require_capture_provenance=False,
        )
    else:
        omissions = []
        omissions_sha = None
    omitted: dict[tuple[str, str], str] = {}
    for row in omissions:
        kind, token, reason = row.get("kind"), row.get("token"), row.get("reason")
        if not all(isinstance(value, str) and value for value in (kind, token, reason)):
            raise BundleError("explicit omission rows require non-empty kind, token, and reason")
        key = (kind, token)
        if key in omitted:
            raise BundleError(f"duplicate explicit omission: {kind}:{token}")
        omitted[key] = reason

    lookups = _lookup_sets(catalogs)
    results: list[dict[str, Any]] = []
    seen: set[tuple[str, str]] = set()
    for row in emitted:
        kind, token = row.get("kind"), row.get("token")
        if not isinstance(kind, str) or not kind or not isinstance(token, str) or not token:
            raise BundleError("emitted token rows require non-empty string kind and token")
        key = (kind, token)
        if key in seen:
            continue
        seen.add(key)
        if token in lookups.get(kind, set()):
            disposition, reason = "resolved", None
        elif key in omitted:
            disposition, reason = "explicitly_omitted", omitted[key]
        else:
            disposition, reason = "unknown", None
        result = {"kind": kind, "token": token, "disposition": disposition}
        if reason:
            result["reason"] = reason
        results.append(result)
    results.sort(key=lambda row: (row["kind"], row["token"]))
    counts = {
        disposition: sum(row["disposition"] == disposition for row in results)
        for disposition in ("resolved", "explicitly_omitted", "unknown")
    }
    return {
        "schema": JOIN_SCHEMA,
        "passed": counts["unknown"] == 0,
        "provenance": provenance,
        "counts": counts,
        "results": results,
        "emitted_inventory_sha256": emitted_sha,
        "omissions_inventory_sha256": omissions_sha,
    }


def _verify_localisation_binding(
    validated: dict[str, dict[str, Any]], specs: Iterable[ArtifactSpec]
) -> None:
    loc_file_sha = validated["localisation_join"]["catalog_file_sha256"]
    for spec in specs:
        if not spec.localisation_dependent:
            continue
        source = validated[spec.role]["catalog"].get("localisation_source")
        if not isinstance(source, dict) or source.get("sha256") != loc_file_sha:
            raise BundleError(f"{spec.role}: localisation source hash does not match bundled localisation")


def create_bundle(
    out_dir: Path,
    *,
    exe: Path,
    mod_state: Path,
    dlc_state: Path,
    load_order: Path,
    source_inventory: Path,
    live_resolution_report: Path | None = None,
    require_current_sources: bool = True,
    specs: Iterable[ArtifactSpec] = DEFAULT_SPECS,
    emitted_tokens: Path | None = None,
    omissions: Path | None = None,
    defer_token_gate: bool = False,
    defer_provenance: Path | None = None,
) -> dict[str, Any]:
    specs = tuple(specs)
    if len(specs) != 8 or {spec.role for spec in specs} != {spec.role for spec in DEFAULT_SPECS}:
        raise BundleError("bundle must contain exactly the eight contract-required artifact roles")
    for spec in specs:
        expected = EXPECTED_SPECS[spec.role]
        if (
            spec.schema != expected.schema
            or spec.bundle_name != expected.bundle_name
            or spec.localisation_dependent != expected.localisation_dependent
        ):
            raise BundleError(f"{spec.role}: artifact contract does not match canonical role specification")
    if not exe.is_file():
        raise BundleError(f"missing executable: {exe}")
    if emitted_tokens is not None and defer_token_gate:
        raise BundleError("emitted token inventory and deferred token gate are mutually exclusive")
    if omissions is not None and emitted_tokens is None:
        raise BundleError("explicit omissions require an emitted token inventory")
    if emitted_tokens is None and not defer_token_gate:
        raise BundleError("emitted token inventory is required (or explicitly use --defer-token-gate)")
    if defer_token_gate and defer_provenance is None:
        raise BundleError("deferred token gate requires --defer-provenance")
    if not defer_token_gate and defer_provenance is not None:
        raise BundleError("--defer-provenance is only valid with --defer-token-gate")

    validated = {spec.role: validate_artifact(spec) for spec in specs}
    _verify_localisation_binding(validated, specs)
    catalogs = {role: info["catalog"] for role, info in validated.items()}
    live_source_inventory = load_source_inventory(source_inventory)
    _validate_unjoined_technology_sharing(
        catalogs["technology_sharing_static"], live_source_inventory
    )
    if require_current_sources:
        if live_resolution_report is None:
            raise BundleError("live resolution report is required for current-source verification")
        verify_current_source_state(exe, live_source_inventory, live_resolution_report)
    source_join = bind_catalog_sources(catalogs, live_source_inventory)
    join_report = (
        account_emitted_tokens(catalogs, emitted_tokens, omissions) if emitted_tokens else None
    )
    if join_report is not None and not join_report["passed"]:
        raise BundleError(f"emitted-token static join has {join_report['counts']['unknown']} unknown tokens")
    deferred = _load_defer_provenance(defer_provenance) if defer_token_gate else None

    parent = out_dir.resolve().parent
    parent.mkdir(parents=True, exist_ok=True)
    stage = Path(tempfile.mkdtemp(prefix=f".{out_dir.name}-", dir=parent))
    try:
        artifact_entries = []
        artifacts_dir = stage / "artifacts"
        proofs_dir = stage / "proofs"
        bindings_dir = stage / "bindings"
        artifacts_dir.mkdir()
        proofs_dir.mkdir()
        bindings_dir.mkdir()
        for spec in specs:
            destination = artifacts_dir / spec.bundle_name
            proof_destination = proofs_dir / f"{spec.role}.gate.json"
            shutil.copyfile(spec.catalog, destination)
            shutil.copyfile(spec.gate_report, proof_destination)
            artifact_entries.append(
                {
                    "role": spec.role,
                    "file": f"artifacts/{spec.bundle_name}",
                    "bytes": destination.stat().st_size,
                    "sha256": sha256_file(destination),
                    "artifact_sha256": validated[spec.role]["catalog_artifact_sha256"],
                    "schema": spec.schema,
                    "source_gate_report": f"proofs/{spec.role}.gate.json",
                    "source_gate_report_sha256": validated[spec.role]["gate_report_sha256"],
                }
            )
        source_digest, source_count = _source_inventory_digest(catalogs)
        if source_count == 0:
            raise BundleError("effective source inventory is empty; content binding is not provable")
        state_bindings = {
            "mod_state": _load_binding(mod_state, "mod_state"),
            "dlc_state": _load_binding(dlc_state, "dlc_state"),
            "load_order": _load_binding(load_order, "load_order"),
        }
        _validate_binding_identity(state_bindings)
        if (
            deferred is not None
            and deferred["session_plan_id"]
            != state_bindings["mod_state"]["identity"]["profile_id"]
        ):
            raise BundleError(
                "deferred token gate session plan does not match current binding profile"
            )
        if live_source_inventory["identity"] != state_bindings["mod_state"]["identity"]:
            raise BundleError("live source inventory identity does not match state bindings")
        validate_source_inventory_bindings(live_source_inventory, state_bindings)
        embedded_source = bindings_dir / "source_inventory.json"
        shutil.copyfile(source_inventory, embedded_source)
        embedded_source_inventory = load_source_inventory(embedded_source)
        if embedded_source_inventory["inventory_sha256"] != live_source_inventory["inventory_sha256"]:
            raise BundleError("embedded live source inventory changed during copy")
        source_join_path = proofs_dir / "catalog_source_join.json"
        source_join_path.write_bytes(pretty_json(source_join))
        executable_binding = {
            "name": exe.name,
            "bytes": exe.stat().st_size,
            "sha256": sha256_file(exe),
        }
        bindings = {
            "executable": executable_binding,
            **state_bindings,
            "product_modset": product_modset_binding(
                executable_binding["sha256"], state_bindings, live_source_inventory
            ),
            "catalog_source_occurrences": {
                "rows": source_count,
                "sha256": source_digest,
            },
            "live_source_inventory": {
                "file": "bindings/source_inventory.json",
                "schema": SOURCE_INVENTORY_SCHEMA,
                "bytes": embedded_source.stat().st_size,
                "file_sha256": sha256_file(embedded_source),
                "inventory_sha256": live_source_inventory["inventory_sha256"],
                "namespace_sha256": live_source_inventory["namespace_sha256"],
                "rows_sha256": live_source_inventory["rows_sha256"],
                "row_count": len(live_source_inventory["rows"]),
            },
        }
        if join_report is not None:
            (stage / "token_join_report.json").write_bytes(pretty_json(join_report))
        manifest = {
            "schema": BUNDLE_SCHEMA,
            "artifacts": sorted(artifact_entries, key=lambda row: row["role"]),
            "bindings": bindings,
            "catalog_source_join": {
                "file": "proofs/catalog_source_join.json",
                "schema": SOURCE_JOIN_SCHEMA,
                "sha256": sha256_file(source_join_path),
            },
            "token_join": {
                "status": "passed" if join_report else "deferred",
                "report": "token_join_report.json" if join_report else None,
                "sha256": sha256_bytes(pretty_json(join_report)) if join_report else None,
                "provenance": join_report["provenance"] if join_report else deferred,
            },
        }
        manifest["bundle_id"] = sha256_bytes(canonical_json(manifest))
        (stage / "static_bundle_manifest.json").write_bytes(pretty_json(manifest))
        if out_dir.exists():
            raise BundleError(f"output already exists: {out_dir}")
        os.replace(stage, out_dir)
        return manifest
    except Exception:
        shutil.rmtree(stage, ignore_errors=True)
        raise



DLC_ROSTER_BOUND_ARTIFACT = "gfx_icon_asset_manifest_static"


def require_declared_asset_layer_roster(
    catalogs: dict[str, Any], state_bindings: dict[str, Any]
) -> None:
    """F5: refuse a bundled catalog generated against another DLC roster.

    A catalog that declares `meta.asset_layer_model` states which DLCs were
    mounted when it was generated. `out/` is gitignored, so the artifact on disk
    is a machine-local build product that no commit carries; comparing its own
    declaration against the DLC state this capture will launch with is what
    makes the check run per capture instead of by hand.
    """

    enabled = {
        row["id"] for row in state_bindings["dlc_state"]["items"] if row["enabled"]
    }
    for role, catalog in sorted(catalogs.items()):
        model = catalog.get("meta", {}).get("asset_layer_model")
        if model is None:
            if role == DLC_ROSTER_BOUND_ARTIFACT:
                raise BundleError(
                    f"{role}: declares no meta.asset_layer_model, so nothing states which DLCs it "
                    f"was generated against"
                )
            continue
        if not isinstance(model, dict):
            raise BundleError(f"{role}: meta.asset_layer_model is not an object")
        declared_enabled = model.get("enabled_dlcs")
        if not isinstance(declared_enabled, list) or not all(
            isinstance(row, str) for row in declared_enabled
        ):
            raise BundleError(f"{role}: meta.asset_layer_model.enabled_dlcs is malformed")
        excluded = model.get("excluded_asset_only_dlcs", [])
        if not isinstance(excluded, list) or not all(isinstance(row, str) for row in excluded):
            raise BundleError(
                f"{role}: meta.asset_layer_model.excluded_asset_only_dlcs is malformed"
            )
        if model.get("dlc_load_json_present") is not True:
            raise BundleError(
                f"{role}: was generated without reading dlc_load.json, so it names a DLC roster "
                f"no capture is known to run"
            )
        declared = set(declared_enabled) - set(excluded)
        if declared == enabled:
            continue
        raise BundleError(
            f"{role}: generated against a different DLC roster than this capture mounts; "
            f"built with but not enabled here {sorted(declared - enabled)}; "
            f"enabled here but not built with {sorted(enabled - declared)}"
        )


def verify_bundle(
    bundle_dir: Path,
    *,
    exe: Path,
    mod_state: Path,
    dlc_state: Path,
    load_order: Path,
    source_inventory: Path,
    live_resolution_report: Path | None = None,
    require_current_sources: bool = True,
    require_token_gate: bool = True,
    expected_capture_id: str | None = None,
    expected_session_id: str | None = None,
    expected_stream_sha256: str | None = None,
) -> dict[str, Any]:
    manifest = load_object(bundle_dir / "static_bundle_manifest.json", "bundle manifest")
    if manifest.get("schema") != BUNDLE_SCHEMA:
        raise BundleError("unsupported static bundle schema")
    if set(manifest) != {
        "schema", "artifacts", "bindings", "catalog_source_join", "token_join", "bundle_id"
    }:
        raise BundleError("bundle manifest contains unknown or missing fields")
    claimed_id = manifest.get("bundle_id")
    unhashed = dict(manifest)
    unhashed.pop("bundle_id", None)
    if claimed_id != sha256_bytes(canonical_json(unhashed)):
        raise BundleError("bundle manifest id mismatch")
    roles = set()
    entries = manifest.get("artifacts")
    if not isinstance(entries, list):
        raise BundleError("bundle artifacts must be an array")
    bundled_catalogs: dict[str, dict[str, Any]] = {}
    for entry in entries:
        if not isinstance(entry, dict):
            raise BundleError("malformed artifact manifest entry")
        role, rel = entry.get("role"), entry.get("file")
        if role in roles or role not in EXPECTED_SPECS:
            raise BundleError(f"duplicate/unknown artifact role: {role!r}")
        roles.add(role)
        path = (bundle_dir / str(rel)).resolve()
        try:
            path.relative_to(bundle_dir.resolve())
        except ValueError as exc:
            raise BundleError(f"artifact path escapes bundle: {rel}") from exc
        if not path.is_file() or sha256_file(path) != entry.get("sha256"):
            raise BundleError(f"missing/corrupt bundled artifact: {role}")
        catalog = load_object(path, f"bundled {role}")
        expected_schema = EXPECTED_SPECS[role].schema
        if entry.get("schema") != expected_schema or catalog.get("schema") != expected_schema:
            raise BundleError(f"bundled artifact schema mismatch: {role}")
        if _verify_catalog_self_hash(catalog, role) != entry.get("artifact_sha256"):
            raise BundleError(f"bundled artifact self-hash mismatch: {role}")
        bundled_catalogs[role] = catalog
        proof_path = (bundle_dir / str(entry.get("source_gate_report"))).resolve()
        try:
            proof_path.relative_to(bundle_dir.resolve())
        except ValueError as exc:
            raise BundleError(f"gate report path escapes bundle: {role}") from exc
        proof = load_object(proof_path, f"bundled {role} gate report")
        if sha256_file(proof_path) != entry.get("source_gate_report_sha256"):
            raise BundleError(f"bundled gate report hash mismatch: {role}")
        if proof.get("passed") is not True or proof.get("artifact_sha256") != entry.get("artifact_sha256"):
            raise BundleError(f"bundled gate report failed or names another artifact: {role}")
    if roles != {spec.role for spec in DEFAULT_SPECS}:
        raise BundleError("bundle is missing one or more required artifact roles")

    _validate_unjoined_technology_sharing(bundled_catalogs["technology_sharing_static"])

    loc_file_sha = next(
        entry["sha256"] for entry in entries if entry.get("role") == "localisation_join"
    )
    for spec in DEFAULT_SPECS:
        if spec.localisation_dependent:
            source = bundled_catalogs[spec.role].get("localisation_source")
            if not isinstance(source, dict) or source.get("sha256") != loc_file_sha:
                raise BundleError(
                    f"bundled {spec.role}: localisation source hash does not match bundled localisation"
                )

    source_digest, source_count = _source_inventory_digest(bundled_catalogs)
    if source_count == 0:
        raise BundleError("bundled effective source inventory is empty")
    recorded_bindings = manifest.get("bindings")
    if not isinstance(recorded_bindings, dict):
        raise BundleError("bundle bindings must be an object")
    recorded_product_modset = validate_product_modset_binding(
        recorded_bindings.get("product_modset")
    )
    recorded_source = recorded_bindings.get("catalog_source_occurrences")
    expected_source = {"rows": source_count, "sha256": source_digest}
    if recorded_source != expected_source:
        raise BundleError("effective source inventory binding mismatch")

    external_source = load_source_inventory(source_inventory)
    _validate_unjoined_technology_sharing(
        bundled_catalogs["technology_sharing_static"], external_source
    )
    if require_current_sources:
        if live_resolution_report is None:
            raise BundleError("live resolution report is required for current-source verification")
        verify_current_source_state(exe, external_source, live_resolution_report)
    source_binding = recorded_bindings.get("live_source_inventory")
    if not isinstance(source_binding, dict) or set(source_binding) != {
        "file", "schema", "bytes", "file_sha256", "inventory_sha256",
        "namespace_sha256", "rows_sha256", "row_count",
    }:
        raise BundleError("live source inventory binding is malformed")
    embedded_source_path = (bundle_dir / str(source_binding.get("file"))).resolve()
    try:
        embedded_source_path.relative_to(bundle_dir.resolve())
    except ValueError as exc:
        raise BundleError("live source inventory path escapes bundle") from exc
    embedded_source = load_source_inventory(embedded_source_path)
    expected_source_binding = {
        "file": source_binding["file"],
        "schema": SOURCE_INVENTORY_SCHEMA,
        "bytes": embedded_source_path.stat().st_size,
        "file_sha256": sha256_file(embedded_source_path),
        "inventory_sha256": embedded_source["inventory_sha256"],
        "namespace_sha256": embedded_source["namespace_sha256"],
        "rows_sha256": embedded_source["rows_sha256"],
        "row_count": len(embedded_source["rows"]),
    }
    if source_binding != expected_source_binding:
        raise BundleError("embedded live source inventory binding mismatch")
    if (
        external_source != embedded_source
        or external_source["file_sha256"] != embedded_source["file_sha256"]
    ):
        raise BundleError("external live source inventory does not match bundled inventory")

    join_binding = manifest.get("catalog_source_join")
    if not isinstance(join_binding, dict) or set(join_binding) != {"file", "schema", "sha256"}:
        raise BundleError("catalog source join binding is malformed")
    join_path = (bundle_dir / str(join_binding.get("file"))).resolve()
    try:
        join_path.relative_to(bundle_dir.resolve())
    except ValueError as exc:
        raise BundleError("catalog source join path escapes bundle") from exc
    recorded_join = load_object(join_path, "catalog source join")
    if (
        join_binding.get("schema") != SOURCE_JOIN_SCHEMA
        or join_binding.get("sha256") != sha256_file(join_path)
        or recorded_join != bind_catalog_sources(bundled_catalogs, embedded_source)
    ):
        raise BundleError("catalog source join proof mismatch")

    state_bindings = {
        "mod_state": _load_binding(mod_state, "mod_state"),
        "dlc_state": _load_binding(dlc_state, "dlc_state"),
        "load_order": _load_binding(load_order, "load_order"),
    }
    _validate_binding_identity(state_bindings)
    require_declared_asset_layer_roster(bundled_catalogs, state_bindings)
    if external_source["identity"] != state_bindings["mod_state"]["identity"]:
        raise BundleError("live source inventory identity does not match current state bindings")
    validate_source_inventory_bindings(external_source, state_bindings)
    executable_binding = {
        "name": exe.name,
        "bytes": exe.stat().st_size,
        "sha256": sha256_file(exe),
    }
    current_bindings = {
        "executable": executable_binding,
        **state_bindings,
        "product_modset": product_modset_binding(
            executable_binding["sha256"], state_bindings, external_source
        ),
    }
    for role, value in current_bindings.items():
        if recorded_bindings.get(role) != value:
            raise BundleError(f"bundle binding mismatch: {role}")

    token_gate = manifest.get("token_join")
    if not isinstance(token_gate, dict):
        raise BundleError("token_join must be an object")
    if set(token_gate) != {"status", "report", "sha256", "provenance"}:
        raise BundleError("token_join contains unknown or missing fields")
    status = token_gate.get("status")
    if status not in {"passed", "deferred"}:
        raise BundleError("token_join.status must be exactly 'passed' or 'deferred'")
    if require_token_gate and status != "passed":
        raise BundleError("emitted-token join gate was deferred")
    if status == "passed":
        report_rel = token_gate.get("report")
        if not isinstance(report_rel, str) or not report_rel or not _valid_sha(token_gate.get("sha256")):
            raise BundleError("passed token_join requires report path and SHA-256")
        report_path = (bundle_dir / report_rel).resolve()
        try:
            report_path.relative_to(bundle_dir.resolve())
        except ValueError as exc:
            raise BundleError("token join report path escapes bundle") from exc
        report = load_object(report_path, "token join report")
        if sha256_file(report_path) != token_gate["sha256"] or report.get("passed") is not True:
            raise BundleError("token join report missing, corrupt, or failed")
        if report.get("schema") not in {JOIN_SCHEMA, FINAL_JOIN_SCHEMA}:
            raise BundleError("token join report schema mismatch")
        _required_sha(
            report.get("emitted_inventory_sha256"),
            "token join report.emitted_inventory_sha256",
        )
        omission_sha = report.get("omissions_inventory_sha256")
        if omission_sha is not None:
            _required_sha(omission_sha, "token join report.omissions_inventory_sha256")
        provenance = _capture_provenance(report.get("provenance"), "token join report")
        if token_gate.get("provenance") != provenance:
            raise BundleError("token join manifest/report provenance mismatch")
        counts = report.get("counts")
        if not isinstance(counts, dict) or set(counts) != {
            "resolved", "explicitly_omitted", "unknown"
        }:
            raise BundleError("token join report counts are malformed")
        if any(not isinstance(count, int) or isinstance(count, bool) or count < 0 for count in counts.values()):
            raise BundleError("token join report counts must be non-negative integers")
        results = report.get("results")
        if not isinstance(results, list) or not results:
            raise BundleError("passed token join report requires non-empty results")
        if sum(counts.values()) != len(results):
            raise BundleError("token join report counts do not match results")
        dispositions = {"resolved", "explicitly_omitted", "unknown"}
        actual_counts = {disposition: 0 for disposition in dispositions}
        seen: set[tuple[str, str]] = set()
        for index, row in enumerate(results):
            if not isinstance(row, dict) or row.get("disposition") not in dispositions:
                raise BundleError(f"token join result {index} is malformed")
            kind = _required_string(row.get("kind"), f"token join result {index}.kind")
            token = _required_string(row.get("token"), f"token join result {index}.token")
            if (kind, token) in seen:
                raise BundleError("token join report contains duplicate token rows")
            seen.add((kind, token))
            actual_counts[row["disposition"]] += 1
            if row["disposition"] == "explicitly_omitted":
                _required_string(row.get("reason"), f"token join result {index}.reason")
        if actual_counts != counts:
            raise BundleError("token join report disposition counts are false")
        if counts["unknown"] != 0:
            raise BundleError("token join report contains unknown tokens")
        if require_token_gate:
            expected_values = (expected_capture_id, expected_session_id, expected_stream_sha256)
            if any(value is None for value in expected_values):
                raise BundleError("final production verification requires expected capture/session/stream provenance")
            expected_provenance = {
                "capture_id": _required_string(expected_capture_id, "expected_capture_id"),
                "session_id": _required_string(expected_session_id, "expected_session_id"),
                "stream_sha256": _required_sha(expected_stream_sha256, "expected_stream_sha256"),
            }
            if report.get("schema") != FINAL_JOIN_SCHEMA:
                raise BundleError("final production verification requires a finalized token join")
            if provenance != expected_provenance:
                raise BundleError("final token join provenance does not match current capture")
        if report.get("schema") == FINAL_JOIN_SCHEMA:
            emitted_rel = report.get("emitted_inventory_file")
            if not isinstance(emitted_rel, str):
                raise BundleError("finalized token join lacks bundled emitted inventory")
            emitted_path = (bundle_dir / emitted_rel).resolve()
            try:
                emitted_path.relative_to(bundle_dir.resolve())
            except ValueError as exc:
                raise BundleError("bundled emitted inventory path escapes bundle") from exc
            omission_rel = report.get("omissions_inventory_file")
            omission_path = (bundle_dir / omission_rel).resolve() if isinstance(omission_rel, str) else None
            if omission_path is not None:
                try:
                    omission_path.relative_to(bundle_dir.resolve())
                except ValueError as exc:
                    raise BundleError("bundled omission inventory path escapes bundle") from exc
            recomputed_join = account_emitted_tokens(
                bundled_catalogs, emitted_path, omission_path
            )
            for field in (
                "passed", "provenance", "counts", "results",
                "emitted_inventory_sha256", "omissions_inventory_sha256",
            ):
                if report.get(field) != recomputed_join.get(field):
                    raise BundleError(f"finalized token join semantic mismatch: {field}")
            lineage = report.get("deferred_lineage")
            if not isinstance(lineage, dict) or set(lineage) != {
                "bundle_id", "manifest_file", "manifest_sha256"
            }:
                raise BundleError("finalized token join deferred lineage is malformed")
            lineage_path = (bundle_dir / str(lineage["manifest_file"])).resolve()
            try:
                lineage_path.relative_to(bundle_dir.resolve())
            except ValueError as exc:
                raise BundleError("deferred lineage manifest path escapes bundle") from exc
            deferred_manifest = load_object(lineage_path, "deferred lineage manifest")
            deferred_unhashed = dict(deferred_manifest)
            deferred_id = deferred_unhashed.pop("bundle_id", None)
            if (
                sha256_file(lineage_path) != lineage["manifest_sha256"]
                or deferred_id != lineage["bundle_id"]
                or deferred_id != sha256_bytes(canonical_json(deferred_unhashed))
                or deferred_manifest.get("token_join", {}).get("status") != "deferred"
                or deferred_manifest.get("artifacts") != manifest.get("artifacts")
                or deferred_manifest.get("bindings") != manifest.get("bindings")
            ):
                raise BundleError("finalized token join deferred lineage mismatch")
    else:
        if token_gate.get("report") is not None or token_gate.get("sha256") is not None:
            raise BundleError("deferred token_join must not name a report or SHA-256")
        provenance = token_gate.get("provenance")
        if not isinstance(provenance, dict) or provenance.get("schema") != DEFER_PROVENANCE_SCHEMA:
            raise BundleError("deferred token_join requires valid provenance")
        if set(provenance) != {
            "schema", "requested_by", "reason", "session_plan_id", "provenance_sha256", "bytes_sha256"
        }:
            raise BundleError("deferred token_join provenance contains unknown or missing fields")
        for field in ("requested_by", "reason", "session_plan_id"):
            _required_string(provenance.get(field), f"deferred token_join.provenance.{field}")
        if (
            provenance["session_plan_id"]
            != state_bindings["mod_state"]["identity"]["profile_id"]
        ):
            raise BundleError(
                "deferred token_join session plan does not match current binding profile"
            )
        normalized_deferred = {
            "schema": DEFER_PROVENANCE_SCHEMA,
            "requested_by": provenance["requested_by"],
            "reason": provenance["reason"],
            "session_plan_id": provenance["session_plan_id"],
        }
        expected_deferred_sha = sha256_bytes(canonical_json(normalized_deferred))
        if provenance.get("provenance_sha256") != expected_deferred_sha:
            raise BundleError("deferred token_join provenance hash mismatch")
        _required_sha(
            provenance.get("bytes_sha256"),
            "deferred token_join.provenance.bytes_sha256",
        )
    return {
        "passed": True,
        "bundle_id": claimed_id,
        "product_modset_sha256": recorded_product_modset["product_modset_sha256"],
        "artifact_count": len(roles),
        "token_join_status": status,
        "verification_phase": (
            "pre_injection_deferred_allowed"
            if status == "deferred"
            else "post_capture_finalized"
            if require_token_gate
            else "nonproduction_passed_join"
        ),
    }


def finalize_deferred_bundle(
    source_bundle: Path,
    out_dir: Path,
    *,
    exe: Path,
    mod_state: Path,
    dlc_state: Path,
    load_order: Path,
    source_inventory: Path,
    live_resolution_report: Path,
    emitted_tokens: Path,
    expected_capture_id: str,
    expected_session_id: str,
    expected_stream_sha256: str,
    omissions: Path | None = None,
    require_current_sources: bool = True,
) -> dict[str, Any]:
    source_bundle = source_bundle.resolve()
    out_dir = out_dir.resolve()
    if out_dir.exists():
        raise BundleError(f"output already exists: {out_dir}")
    for left, right in ((source_bundle, out_dir), (out_dir, source_bundle)):
        try:
            right.relative_to(left)
        except ValueError:
            continue
        raise BundleError("source and finalized bundle paths must not overlap")
    expected_provenance = {
        "capture_id": _required_string(expected_capture_id, "expected_capture_id"),
        "session_id": _required_string(expected_session_id, "expected_session_id"),
        "stream_sha256": _required_sha(expected_stream_sha256, "expected_stream_sha256"),
    }
    verify_bundle(
        source_bundle,
        exe=exe,
        mod_state=mod_state,
        dlc_state=dlc_state,
        load_order=load_order,
        source_inventory=source_inventory,
        live_resolution_report=live_resolution_report,
        require_current_sources=require_current_sources,
        require_token_gate=False,
    )
    manifest_path = source_bundle / "static_bundle_manifest.json"
    deferred_manifest_bytes = manifest_path.read_bytes()
    deferred_manifest = _decode_json(
        deferred_manifest_bytes, "deferred bundle manifest", manifest_path
    )
    if not isinstance(deferred_manifest, dict):
        raise BundleError("deferred bundle manifest must be a JSON object")
    if deferred_manifest.get("token_join", {}).get("status") != "deferred":
        raise BundleError("token finalization requires an explicitly deferred source bundle")

    parent = out_dir.parent
    parent.mkdir(parents=True, exist_ok=True)
    stage = Path(tempfile.mkdtemp(prefix=f".{out_dir.name}-", dir=parent))
    try:
        shutil.copytree(source_bundle, stage, dirs_exist_ok=True)
        verify_bundle(
            stage,
            exe=exe,
            mod_state=mod_state,
            dlc_state=dlc_state,
            load_order=load_order,
            source_inventory=source_inventory,
            live_resolution_report=live_resolution_report,
            require_current_sources=require_current_sources,
            require_token_gate=False,
        )
        inputs_dir = stage / "inputs"
        inputs_dir.mkdir(exist_ok=False)
        emitted_copy = inputs_dir / "emitted_tokens.json"
        shutil.copyfile(emitted_tokens, emitted_copy)
        omissions_copy = None
        if omissions is not None:
            omissions_copy = inputs_dir / "token_omissions.json"
            shutil.copyfile(omissions, omissions_copy)
        catalogs = {}
        for entry in deferred_manifest["artifacts"]:
            catalogs[entry["role"]] = load_object(
                stage / entry["file"], f"bundled {entry['role']}"
            )
        join = account_emitted_tokens(catalogs, emitted_copy, omissions_copy)
        if join["provenance"] != expected_provenance:
            raise BundleError("emitted inventory provenance does not match expected current capture")
        if not join["passed"]:
            raise BundleError(
                f"emitted-token static join has {join['counts']['unknown']} unknown tokens"
            )
        lineage_path = stage / "proofs/deferred_static_bundle_manifest.json"
        lineage_path.write_bytes(deferred_manifest_bytes)
        final_report = {
            **join,
            "schema": FINAL_JOIN_SCHEMA,
            "emitted_inventory_file": "inputs/emitted_tokens.json",
            "omissions_inventory_file": (
                "inputs/token_omissions.json" if omissions_copy is not None else None
            ),
            "deferred_lineage": {
                "bundle_id": deferred_manifest["bundle_id"],
                "manifest_file": "proofs/deferred_static_bundle_manifest.json",
                "manifest_sha256": sha256_file(lineage_path),
            },
        }
        report_path = stage / "token_join_report.json"
        report_path.write_bytes(pretty_json(final_report))
        final_manifest = dict(deferred_manifest)
        final_manifest["token_join"] = {
            "status": "passed",
            "report": "token_join_report.json",
            "sha256": sha256_file(report_path),
            "provenance": expected_provenance,
        }
        final_manifest.pop("bundle_id", None)
        final_manifest["bundle_id"] = sha256_bytes(canonical_json(final_manifest))
        (stage / "static_bundle_manifest.json").write_bytes(pretty_json(final_manifest))
        result = verify_bundle(
            stage,
            exe=exe,
            mod_state=mod_state,
            dlc_state=dlc_state,
            load_order=load_order,
            source_inventory=source_inventory,
            live_resolution_report=live_resolution_report,
            require_current_sources=require_current_sources,
            require_token_gate=True,
            expected_capture_id=expected_capture_id,
            expected_session_id=expected_session_id,
            expected_stream_sha256=expected_stream_sha256,
        )
        os.replace(stage, out_dir)
        return result
    except Exception:
        shutil.rmtree(stage, ignore_errors=True)
        raise


def _path(value: str) -> Path:
    return Path(value).expanduser().resolve()


def _binding_args(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--exe", type=_path, required=True)
    parser.add_argument("--mod-state", type=_path, required=True)
    parser.add_argument("--dlc-state", type=_path, required=True)
    parser.add_argument("--load-order", type=_path, required=True)
    parser.add_argument("--source-inventory", type=_path, required=True)
    parser.add_argument("--live-resolution-report", type=_path, required=True)


def _expected_capture_args(parser: argparse.ArgumentParser, *, required: bool) -> None:
    parser.add_argument("--expected-capture-id", required=required)
    parser.add_argument("--expected-session-id", required=required)
    parser.add_argument("--expected-stream-sha256", required=required)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    create = sub.add_parser("create", help="validate and compose the eight static artifacts")
    create.add_argument("--out", type=_path, required=True)
    _binding_args(create)
    create.add_argument("--emitted-tokens", type=_path)
    create.add_argument("--omissions", type=_path)
    create.add_argument("--defer-token-gate", action="store_true")
    create.add_argument("--defer-provenance", type=_path)
    verify = sub.add_parser("verify", help="verify bundle bytes and current bindings")
    verify.add_argument("--bundle", type=_path, required=True)
    _binding_args(verify)
    verify.add_argument("--allow-deferred-token-gate", action="store_true")
    _expected_capture_args(verify, required=False)
    finalize = sub.add_parser("finalize", help="finalize a deferred bundle for one completed capture")
    finalize.add_argument("--bundle", type=_path, required=True)
    finalize.add_argument("--out", type=_path, required=True)
    _binding_args(finalize)
    finalize.add_argument("--emitted-tokens", type=_path, required=True)
    finalize.add_argument("--omissions", type=_path)
    _expected_capture_args(finalize, required=True)
    args = parser.parse_args(argv)
    try:
        if args.command == "create":
            result = create_bundle(
                args.out,
                exe=args.exe,
                mod_state=args.mod_state,
                dlc_state=args.dlc_state,
                load_order=args.load_order,
                source_inventory=args.source_inventory,
                live_resolution_report=args.live_resolution_report,
                emitted_tokens=args.emitted_tokens,
                omissions=args.omissions,
                defer_token_gate=args.defer_token_gate,
                defer_provenance=args.defer_provenance,
            )
        elif args.command == "verify":
            if args.allow_deferred_token_gate and any(
                value is not None
                for value in (
                    args.expected_capture_id,
                    args.expected_session_id,
                    args.expected_stream_sha256,
                )
            ):
                raise BundleError(
                    "pre-injection deferred admission cannot accept final capture provenance"
                )
            result = verify_bundle(
                args.bundle,
                exe=args.exe,
                mod_state=args.mod_state,
                dlc_state=args.dlc_state,
                load_order=args.load_order,
                source_inventory=args.source_inventory,
                live_resolution_report=args.live_resolution_report,
                require_token_gate=not args.allow_deferred_token_gate,
                expected_capture_id=args.expected_capture_id,
                expected_session_id=args.expected_session_id,
                expected_stream_sha256=args.expected_stream_sha256,
            )
            if (
                args.allow_deferred_token_gate
                and result.get("token_join_status") != "deferred"
            ):
                raise BundleError(
                    "--allow-deferred-token-gate is only valid for a deferred pre-injection bundle"
                )
        else:
            result = finalize_deferred_bundle(
                args.bundle,
                args.out,
                exe=args.exe,
                mod_state=args.mod_state,
                dlc_state=args.dlc_state,
                load_order=args.load_order,
                source_inventory=args.source_inventory,
                live_resolution_report=args.live_resolution_report,
                emitted_tokens=args.emitted_tokens,
                omissions=args.omissions,
                expected_capture_id=args.expected_capture_id,
                expected_session_id=args.expected_session_id,
                expected_stream_sha256=args.expected_stream_sha256,
            )
    except (BundleError, OSError, json.JSONDecodeError) as exc:
        print(f"static bundle gate: FAIL: {exc}", file=sys.stderr)
        return 1
    print(json.dumps(result, ensure_ascii=False, sort_keys=True, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
