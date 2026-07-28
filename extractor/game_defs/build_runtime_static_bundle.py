#!/usr/bin/env python3
"""Rebuild all eight static catalogs for an already-resolved live binding set."""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import re
import shutil
import subprocess
import sys
import zipfile
from pathlib import Path
from typing import Any

import live_binding_resolver as LR
import static_bundle as SB


BUILD_SCHEMA = "extractor.runtime_static_catalog_build.v1"
FINGERPRINT_SCHEMA = "extractor.runtime_static_catalog_inputs.v1"
STAGING_SCHEMA = "extractor.runtime_static_mod_staging.v2"
RENDERER_HANDOFF_SCHEMA = "extractor.renderer_game_files_handoff.v1"
HERE = Path(__file__).resolve().parent
REPO_ROOT = HERE.parents[1]
TEMPLATES_GATE = (
    REPO_ROOT
    / "extractor/native/lanes/templates_equipment_catalog_live/templates_equipment_catalog_gate.py"
)


MOD_SOURCE_FIELDS = {
    "id", "ordinal", "name", "descriptor_path", "descriptor_sha256",
    "content_kind", "content_path", "content_sha256",
}
UI14_ENGINE_VFS_FILE = "ui14_engine_vfs.json"


def _runtime_ui14_receipt(
    bindings_dir: Path, destination: Path, staged_rows: list[dict[str, Any]]
) -> dict[str, Any]:
    source = SB.load_object(bindings_dir / UI14_ENGINE_VFS_FILE, "UI14 engine VFS receipt")
    if (
        source.get("schema") != LR.UI14_ENGINE_VFS_SCHEMA
        or set(source) != {"schema", "executable_sha256", "layers", "effective_winners"}
        or not isinstance(source.get("layers"), list)
        or not isinstance(source.get("effective_winners"), list)
    ):
        raise LR.ResolutionError("UI14 engine VFS receipt is malformed")
    source_by_id = {row.get("id"): row for row in source["layers"] if isinstance(row, dict)}
    staged_ids = {row["id"] for row in staged_rows}
    if len(source_by_id) != len(source["layers"]) or set(source_by_id) != staged_ids:
        raise LR.ResolutionError("UI14 engine VFS receipt layer set changed during staging")
    inventory = LR.validate_source_inventory_document(
        SB.load_object(bindings_dir / "source_inventory.json", "live source inventory")
    )
    mod_state = SB.load_object(bindings_dir / "mod_state.json", "resolved mod state")
    SB._validate_binding_document(mod_state, "mod_state")
    resolved_source_by_id = {
        row["id"]: row for row in _load_resolved_mod_sources(bindings_dir, mod_state)
    }
    mod_inventory_by_id = {layer_id: [] for layer_id in staged_ids}
    for row in inventory["rows"]:
        if row["layer_kind"] == "mod" and row["layer_id"] in mod_inventory_by_id:
            mod_inventory_by_id[row["layer_id"]].append(row)
    LR.validate_ui14_mod_collision_winners(
        mod_inventory_by_id, source["effective_winners"]
    )
    layers = []
    descriptor_ordinals: set[int] = set()
    mount_ordinals: set[int] = set()
    for row in staged_rows:
        source_row = source_by_id[row["id"]]
        if set(source_row) != {
            "id", "descriptor_sha256", "engine_descriptor_ordinal", "engine_mount_ordinal"
        }:
            raise LR.ResolutionError("UI14 engine VFS layer row is malformed")
        if source_row["descriptor_sha256"] != resolved_source_by_id[row["id"]]["descriptor_sha256"]:
            raise LR.ResolutionError("UI14 engine VFS source descriptor hash changed")
        descriptor_ordinal = source_row["engine_descriptor_ordinal"]
        mount_ordinal = source_row["engine_mount_ordinal"]
        if (
            not isinstance(descriptor_ordinal, int)
            or isinstance(descriptor_ordinal, bool)
            or descriptor_ordinal < 0
            or descriptor_ordinal in descriptor_ordinals
            or not isinstance(mount_ordinal, int)
            or isinstance(mount_ordinal, bool)
            or mount_ordinal < 0
            or mount_ordinal in mount_ordinals
        ):
            raise LR.ResolutionError("UI14 engine VFS layer ordinal is invalid")
        descriptor_ordinals.add(descriptor_ordinal)
        mount_ordinals.add(mount_ordinal)
        descriptor = destination / row["descriptor_relpath"]
        layers.append({
            "id": row["id"],
            "descriptor_sha256": SB.sha256_file(descriptor),
            "engine_descriptor_ordinal": descriptor_ordinal,
            "engine_mount_ordinal": mount_ordinal,
        })
    return {
        "schema": LR.UI14_ENGINE_VFS_SCHEMA,
        "executable_sha256": source["executable_sha256"],
        "layers": layers,
        "effective_winners": source["effective_winners"],
    }
CONTENT_LINE_RE = re.compile(r"^\s*(?:path|archive)\s*=.*$", re.MULTILINE | re.IGNORECASE)
SAFE_DLC_ID_RE = re.compile(r"^[A-Za-z0-9_.-]+$")


def _source_namespace_index(bindings_dir: Path) -> dict[tuple[str, str], dict[str, Any]]:
    inventory = LR.validate_source_inventory_document(
        SB.load_object(bindings_dir / "source_inventory.json", "live source inventory")
    )
    return {
        (row["kind"], row["id"]): row
        for row in inventory["namespaces"]
    }


def _active_dlc_archive_sources(
    bindings_dir: Path, vanilla: Path, dlc_state: dict[str, Any]
) -> list[dict[str, Any]]:
    load_order = SB._validate_binding_document(
        SB.load_object(bindings_dir / "load_order.json", "resolved load order"),
        "load_order",
    )
    enabled = {row["id"]: row for row in dlc_state["dlc"] if row["enabled"]}
    ordered_ids = [row["id"] for row in load_order["items"] if row["kind"] == "dlc"]
    if set(ordered_ids) != set(enabled) or len(ordered_ids) != len(enabled):
        raise LR.ResolutionError("load order DLC identities do not match enabled DLC state")
    namespaces = _source_namespace_index(bindings_dir)
    sources: list[dict[str, Any]] = []
    for ordinal, dlc_id in enumerate(ordered_ids):
        if not SAFE_DLC_ID_RE.fullmatch(dlc_id):
            raise LR.ResolutionError(f"unsafe DLC identity for staging: {dlc_id!r}")
        descriptors = sorted((vanilla / "dlc" / dlc_id).glob("*.dlc"))
        if len(descriptors) != 1:
            raise LR.ResolutionError(
                f"enabled DLC requires exactly one installed descriptor: {dlc_id}"
            )
        descriptor = descriptors[0]
        fields = LR.parse_descriptor(descriptor)
        if "archive" not in fields:
            continue
        archive_ref = Path(fields["archive"].replace("\\", "/"))
        if archive_ref.is_absolute() or any(
            part in {"", ".", ".."} for part in archive_ref.parts
        ):
            raise LR.ResolutionError(
                f"unsafe descriptor-backed DLC archive path: {dlc_id}"
            )
        archive = (vanilla / archive_ref).resolve()
        try:
            archive.relative_to(vanilla.resolve())
        except ValueError as exc:
            raise LR.ResolutionError(
                f"descriptor-backed DLC archive escapes install root: {dlc_id}"
            ) from exc
        binding_sha, _ = LR.hash_tree(
            descriptor.parent, layer_kind="dlc", layer_id=dlc_id, allow_empty=True
        )
        if binding_sha != enabled[dlc_id]["content_sha256"]:
            raise LR.ResolutionError(f"resolved DLC content changed: {dlc_id}")
        archive_sha, _ = LR._hash_file_stable(archive)
        namespace = namespaces.get(("dlc", dlc_id))
        if not namespace or namespace.get("binding") != "active_product":
            raise LR.ResolutionError(
                f"source inventory lacks active descriptor-backed DLC namespace: {dlc_id}"
            )
        source_namespace_sha, _rows, source_container_sha, source_container_bytes = LR._zip_namespace_rows(
            archive, dlc_id, layer_kind="dlc"
        )
        if source_namespace_sha != namespace.get("content_sha256"):
            raise LR.ResolutionError(
                f"source inventory DLC namespace changed: {dlc_id}"
            )
        if (
            namespace.get("storage_kind") != "zip_archive"
            or namespace.get("source_container_sha256") != source_container_sha
            or namespace.get("source_container_bytes") != source_container_bytes
        ):
            raise LR.ResolutionError(
                f"source inventory DLC archive identity changed: {dlc_id}"
            )
        sources.append({
            "id": f"_ui15_dlc/{ordinal:04d}-{dlc_id}.mod",
            "dlc_id": dlc_id,
            "ordinal": ordinal,
            "name": fields["name"],
            "descriptor_path": str(descriptor.resolve()),
            "descriptor_sha256": SB.sha256_file(descriptor),
            "content_path": str(archive),
            "source_archive_sha256": archive_sha,
            "source_namespace_sha256": source_namespace_sha,
        })
    return sources


def _renderer_handoff(
    vanilla: Path, destination: Path, launcher: dict[str, Any], staging: dict[str, Any]
) -> dict[str, Any]:
    core = {
        "schema": RENDERER_HANDOFF_SCHEMA,
        "game_dir": str(vanilla.resolve()),
        "userdata": str(destination.resolve()),
        "use_dlc_load": True,
        "launcher_sha256": SB.sha256_bytes(SB.canonical_json(launcher)),
        "staging_manifest_sha256": staging["sha256"],
        "cli_args": [
            "--game", str(vanilla.resolve()),
            "--use-dlc-load", "--userdata", str(destination.resolve()),
        ],
    }
    return {**core, "sha256": SB.sha256_bytes(SB.canonical_json(core))}


def _safe_launcher_token(token: str, destination: Path) -> Path:
    normalized = token.replace("\\", "/")
    candidate = Path(normalized)
    if (
        not normalized
        or "," in normalized
        or candidate.is_absolute()
        or candidate.suffix.casefold() != ".mod"
        or any(part in {"", ".", ".."} for part in candidate.parts)
    ):
        raise LR.ResolutionError(f"unsafe or ambiguous launcher mod token: {token!r}")
    resolved = (destination / candidate).resolve()
    try:
        resolved.relative_to(destination.resolve())
    except ValueError as exc:
        raise LR.ResolutionError(f"launcher mod token escapes staging userdir: {token!r}") from exc
    return resolved


def _load_resolved_mod_sources(
    bindings_dir: Path, mod_state: dict[str, Any]
) -> list[dict[str, Any]]:
    resolution = SB.load_object(bindings_dir / "resolution_report.json", "live resolution report")
    sources = resolution.get("resolved_mod_sources")
    if not isinstance(sources, list) or not all(isinstance(row, dict) for row in sources):
        raise LR.ResolutionError("live resolution report lacks resolved_mod_sources")
    load_order = SB._validate_binding_document(
        SB.load_object(bindings_dir / "load_order.json", "resolved load order"),
        "load_order",
    )
    enabled_by_id = {row["id"]: row for row in mod_state["mods"] if row["enabled"]}
    ordered_ids = [row["id"] for row in load_order["items"] if row["kind"] == "mod"]
    if set(ordered_ids) != set(enabled_by_id) or len(ordered_ids) != len(enabled_by_id):
        raise LR.ResolutionError("load order mod identities do not match enabled mod state")
    enabled = [enabled_by_id[item_id] for item_id in ordered_ids]
    if len(sources) != len(enabled):
        raise LR.ResolutionError("resolved mod source count does not match enabled mod count")
    normalized: list[dict[str, Any]] = []
    for ordinal, (source, binding) in enumerate(zip(sources, enabled, strict=True)):
        if set(source) != MOD_SOURCE_FIELDS:
            raise LR.ResolutionError(f"resolved mod source {ordinal} contains unknown or missing fields")
        if source.get("ordinal") != ordinal or source.get("id") != binding["id"]:
            raise LR.ResolutionError("resolved mod sources do not preserve launcher order")
        if source.get("name") != binding["name"]:
            raise LR.ResolutionError(f"resolved mod source identity mismatch: {binding['id']}")
        if source.get("descriptor_sha256") != binding["descriptor_sha256"]:
            raise LR.ResolutionError(f"resolved mod descriptor hash mismatch: {binding['id']}")
        if source.get("content_sha256") != binding["content_sha256"]:
            raise LR.ResolutionError(f"resolved mod content hash mismatch: {binding['id']}")
        if source.get("content_kind") not in {"directory", "archive"}:
            raise LR.ResolutionError(f"resolved mod content kind is invalid: {binding['id']}")
        descriptor = Path(SB._required_string(source.get("descriptor_path"), "descriptor_path")).resolve()
        content = Path(SB._required_string(source.get("content_path"), "content_path")).resolve()
        if SB.sha256_file(descriptor) != binding["descriptor_sha256"]:
            raise LR.ResolutionError(f"resolved mod descriptor changed: {binding['id']}")
        fields = LR.parse_descriptor(descriptor)
        if fields["name"] != binding["name"]:
            raise LR.ResolutionError(f"resolved mod descriptor name changed: {binding['id']}")
        content_ref = fields.get("path") or fields["archive"]
        derived = Path(content_ref)
        if not derived.is_absolute():
            derived = descriptor.parents[1] / derived
        if derived.resolve() != content:
            raise LR.ResolutionError(f"resolved mod content path is ambiguous: {binding['id']}")
        if source["content_kind"] == "directory":
            actual_sha, _ = LR.hash_tree(content, layer_kind="mod", layer_id=binding["id"])
        else:
            if not content.is_file():
                raise LR.ResolutionError(f"resolved mod archive is missing: {binding['id']}")
            actual_sha, _ = LR._hash_file_stable(content)
        if actual_sha != binding["content_sha256"]:
            raise LR.ResolutionError(f"resolved mod content changed: {binding['id']}")
        normalized.append({**source, "descriptor_path": str(descriptor), "content_path": str(content)})
    return normalized


def _extract_archive(source: Path, destination: Path) -> None:
    destination.mkdir(parents=True, exist_ok=False)
    seen: set[str] = set()
    try:
        archive = zipfile.ZipFile(source)
    except (OSError, zipfile.BadZipFile) as exc:
        raise LR.ResolutionError(f"invalid resolved mod archive: {source}") from exc
    with archive:
        for info in archive.infolist():
            if info.is_dir():
                continue
            name = LR._normalize_archive_member(info.filename)
            path = Path(name)
            key = LR._ascii_fold(name)
            if key in seen:
                raise LR.ResolutionError(f"duplicate member in resolved mod archive: {name!r}")
            seen.add(key)
            mode = (info.external_attr >> 16) & 0o170000
            if mode == 0o120000:
                raise LR.ResolutionError(f"symlink member in resolved mod archive: {name!r}")
            if info.flag_bits & 0x1:
                raise LR.ResolutionError(f"encrypted member in resolved mod archive: {name!r}")
            target = destination.joinpath(*path.parts)
            target.parent.mkdir(parents=True, exist_ok=True)
            with archive.open(info) as src, target.open("wb") as dst:
                shutil.copyfileobj(src, dst)
    if not seen:
        raise LR.ResolutionError(f"resolved mod archive has no regular files: {source}")


def _stage_mod_descriptor(source: dict[str, Any], destination: Path, content_root: Path) -> Path:
    target = _safe_launcher_token(source["id"], destination)
    # Preserve every source descriptor byte losslessly. Clausewitz descriptors
    # are narrow byte strings, not guaranteed UTF-8; only the physical content
    # field is rewritten for the private staging userdir.
    text = Path(source["descriptor_path"]).read_bytes().decode("latin-1")
    content_bytes = content_root.resolve().as_posix().encode("utf-8")
    if any(char in content_bytes for char in (b'"', b"\r", b"\n")):
        raise LR.ResolutionError(f"resolved mod content path cannot be staged: {source['id']}")
    content_text = content_bytes.decode("latin-1")
    rendered, count = CONTENT_LINE_RE.subn(f'path="{content_text}"', text)
    if count != 1:
        raise LR.ResolutionError(f"resolved mod descriptor content field is ambiguous: {source['id']}")
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(rendered.encode("latin-1"))
    return target


def prepare_runtime_userdir(bindings_dir: Path, vanilla: Path, destination: Path) -> dict[str, Any]:
    mod_state = SB.load_object(bindings_dir / "mod_state.json", "resolved mod state")
    dlc_state = SB.load_object(bindings_dir / "dlc_state.json", "resolved DLC state")
    SB._validate_binding_document(mod_state, "mod_state")
    SB._validate_binding_document(dlc_state, "dlc_state")
    mod_sources = _load_resolved_mod_sources(bindings_dir, mod_state)
    source_namespaces = _source_namespace_index(bindings_dir)
    dlc_archive_sources = _active_dlc_archive_sources(
        bindings_dir, vanilla, dlc_state
    )
    descriptors = {
        path.parent.name: path.relative_to(vanilla).as_posix()
        for path in vanilla.glob("dlc/*/*.dlc")
    }
    disabled = []
    for row in dlc_state["dlc"]:
        descriptor = descriptors.get(row["id"])
        if descriptor is None:
            raise LR.ResolutionError(f"resolved DLC lacks installed descriptor: {row['id']}")
        if not row["enabled"]:
            disabled.append(descriptor)
    destination.mkdir(parents=True, exist_ok=False)
    staged_dlc_rows = []
    for source in dlc_archive_sources:
        content_root = destination / "_materialized_dlc" / f"{source['ordinal']:04d}"
        _extract_archive(Path(source["content_path"]), content_root)
        staged_descriptor = _stage_mod_descriptor(source, destination, content_root)
        staged_content_sha, staged_inventory = LR.hash_tree(
            content_root, layer_kind="staged_dlc", layer_id=source["dlc_id"]
        )
        if staged_content_sha != source["source_namespace_sha256"]:
            raise LR.ResolutionError(
                f"source/staged DLC namespace mismatch: {source['dlc_id']}"
            )
        staged_dlc_rows.append({
            "id": source["id"],
            "dlc_id": source["dlc_id"],
            "ordinal": source["ordinal"],
            "descriptor_relpath": staged_descriptor.relative_to(destination).as_posix(),
            "descriptor_sha256": SB.sha256_file(staged_descriptor),
            "content_path": str(content_root.resolve()),
            "content_sha256": staged_content_sha,
            "content_files": len(staged_inventory),
            "source_archive_sha256": source["source_archive_sha256"],
            "source_namespace_sha256": source["source_namespace_sha256"],
        })
    staged_rows = []
    for source in mod_sources:
        content_root = Path(source["content_path"])
        if source["content_kind"] == "archive":
            content_root = destination / "_materialized_mods" / f"{source['ordinal']:04d}"
            _extract_archive(Path(source["content_path"]), content_root)
        staged_descriptor = _stage_mod_descriptor(source, destination, content_root)
        staged_content_sha, staged_inventory = LR.hash_tree(
            content_root, layer_kind="staged_mod", layer_id=source["id"]
        )
        source_namespace = source_namespaces.get(("mod", source["id"]))
        if not source_namespace or staged_content_sha != source_namespace.get("content_sha256"):
            raise LR.ResolutionError(
                f"source/staged mod namespace mismatch: {source['id']}"
            )
        staged_rows.append({
            "id": source["id"],
            "ordinal": source["ordinal"],
            "descriptor_relpath": staged_descriptor.relative_to(destination).as_posix(),
            "descriptor_sha256": SB.sha256_file(staged_descriptor),
            "content_path": str(content_root.resolve()),
            "content_sha256": staged_content_sha,
            "content_files": len(staged_inventory),
            "source_content_sha256": source["content_sha256"],
            "source_namespace_sha256": source_namespace["content_sha256"],
        })
    launcher = {
        "enabled_mods": [
            *[source["id"] for source in dlc_archive_sources],
            *[source["id"] for source in mod_sources],
        ],
        "disabled_dlcs": sorted(disabled),
    }
    (destination / "dlc_load.json").write_bytes(SB.pretty_json(launcher))
    ui14_receipt = _runtime_ui14_receipt(bindings_dir, destination, staged_rows)
    (destination / UI14_ENGINE_VFS_FILE).write_bytes(SB.pretty_json(ui14_receipt))
    staging = {
        "schema": STAGING_SCHEMA,
        "dlc_archives": staged_dlc_rows,
        "mods": staged_rows,
        "ui14_engine_vfs_sha256": SB.sha256_bytes(SB.canonical_json(ui14_receipt)),
    }
    staging["sha256"] = SB.sha256_bytes(SB.canonical_json(staging))
    (destination / "static_mod_staging_manifest.json").write_bytes(SB.pretty_json(staging))
    handoff = _renderer_handoff(vanilla, destination, launcher, staging)
    (destination / "renderer_game_file_handoff.json").write_bytes(
        SB.pretty_json(handoff)
    )
    return launcher


def _validate_runtime_userdir(
    bindings_dir: Path, vanilla: Path, destination: Path
) -> dict[str, Any]:
    mod_state = SB.load_object(bindings_dir / "mod_state.json", "resolved mod state")
    dlc_state = SB.load_object(bindings_dir / "dlc_state.json", "resolved DLC state")
    SB._validate_binding_document(mod_state, "mod_state")
    SB._validate_binding_document(dlc_state, "dlc_state")
    sources = _load_resolved_mod_sources(bindings_dir, mod_state)
    source_namespaces = _source_namespace_index(bindings_dir)
    dlc_archive_sources = _active_dlc_archive_sources(
        bindings_dir, vanilla, dlc_state
    )
    descriptors = {
        path.parent.name: path.relative_to(vanilla).as_posix()
        for path in vanilla.glob("dlc/*/*.dlc")
    }
    disabled = []
    for row in dlc_state["dlc"]:
        descriptor = descriptors.get(row["id"])
        if descriptor is None:
            raise LR.ResolutionError(f"resolved DLC lacks installed descriptor: {row['id']}")
        if not row["enabled"]:
            disabled.append(descriptor)
    expected_launcher = {
        "enabled_mods": [
            *[row["id"] for row in dlc_archive_sources],
            *[row["id"] for row in sources],
        ],
        "disabled_dlcs": sorted(disabled),
    }
    actual_launcher = SB.load_object(destination / "dlc_load.json", "runtime staging launcher state")
    if actual_launcher != expected_launcher:
        raise LR.ResolutionError("runtime staging launcher state does not match resolved bindings")
    expected_ui14 = _runtime_ui14_receipt(bindings_dir, destination, [
        {
            **source,
            "descriptor_relpath": source["id"],
        }
        for source in sources
    ])
    actual_ui14 = SB.load_object(
        destination / UI14_ENGINE_VFS_FILE, "runtime UI14 engine VFS receipt"
    )
    if actual_ui14 != expected_ui14:
        raise LR.ResolutionError("runtime UI14 engine VFS receipt changed")
    manifest = SB.load_object(
        destination / "static_mod_staging_manifest.json", "runtime mod staging manifest"
    )
    claimed = manifest.get("sha256")
    unhashed = dict(manifest)
    unhashed.pop("sha256", None)
    if (
        manifest.get("schema") != STAGING_SCHEMA
        or claimed != SB.sha256_bytes(SB.canonical_json(unhashed))
        or not isinstance(manifest.get("dlc_archives"), list)
        or len(manifest["dlc_archives"]) != len(dlc_archive_sources)
        or not isinstance(manifest.get("mods"), list)
        or len(manifest["mods"]) != len(sources)
        or manifest.get("ui14_engine_vfs_sha256")
        != SB.sha256_bytes(SB.canonical_json(actual_ui14))
    ):
        raise LR.ResolutionError("runtime mod staging manifest is malformed")
    expected_descriptor_relpaths = {
        *[row["id"] for row in dlc_archive_sources],
        *[row["id"] for row in sources],
    }
    actual_descriptor_relpaths = {
        path.relative_to(destination).as_posix()
        for path in destination.rglob("*.mod")
    }
    if actual_descriptor_relpaths != expected_descriptor_relpaths:
        raise LR.ResolutionError("runtime mod staging descriptor namespace changed")
    for source, row in zip(
        dlc_archive_sources, manifest["dlc_archives"], strict=True
    ):
        if (
            row.get("id") != source["id"]
            or row.get("dlc_id") != source["dlc_id"]
            or row.get("ordinal") != source["ordinal"]
        ):
            raise LR.ResolutionError("runtime DLC archive staging order changed")
        descriptor = (destination / str(row.get("descriptor_relpath"))).resolve()
        if SB.sha256_file(descriptor) != row.get("descriptor_sha256"):
            raise LR.ResolutionError(
                f"runtime staged DLC descriptor changed: {source['dlc_id']}"
            )
        fields = LR.parse_descriptor(descriptor)
        content = Path(fields["path"]).resolve()
        if str(content) != row.get("content_path"):
            raise LR.ResolutionError(
                f"runtime staged DLC content path changed: {source['dlc_id']}"
            )
        digest, inventory = LR.hash_tree(
            content, layer_kind="staged_dlc", layer_id=source["dlc_id"]
        )
        expected_namespace = source["source_namespace_sha256"]
        if (
            digest != row.get("content_sha256")
            or digest != row.get("source_namespace_sha256")
            or digest != expected_namespace
            or len(inventory) != row.get("content_files")
            or row.get("source_archive_sha256") != source["source_archive_sha256"]
        ):
            raise LR.ResolutionError(
                f"runtime staged DLC namespace changed: {source['dlc_id']}"
            )
    for source, row in zip(sources, manifest["mods"], strict=True):
        if row.get("id") != source["id"] or row.get("ordinal") != source["ordinal"]:
            raise LR.ResolutionError("runtime mod staging order changed")
        descriptor = (destination / str(row.get("descriptor_relpath"))).resolve()
        if SB.sha256_file(descriptor) != row.get("descriptor_sha256"):
            raise LR.ResolutionError(f"runtime staged descriptor changed: {source['id']}")
        fields = LR.parse_descriptor(descriptor)
        content = Path(fields["path"]).resolve()
        if str(content) != row.get("content_path"):
            raise LR.ResolutionError(f"runtime staged mod content path changed: {source['id']}")
        digest, inventory = LR.hash_tree(content, layer_kind="staged_mod", layer_id=source["id"])
        source_namespace = source_namespaces.get(("mod", source["id"]))
        if (
            not source_namespace
            or digest != row.get("content_sha256")
            or digest != row.get("source_namespace_sha256")
            or digest != source_namespace.get("content_sha256")
            or len(inventory) != row.get("content_files")
        ):
            raise LR.ResolutionError(f"runtime staged mod content changed: {source['id']}")
    expected_handoff = _renderer_handoff(
        vanilla, destination, expected_launcher, manifest
    )
    actual_handoff = SB.load_object(
        destination / "renderer_game_file_handoff.json",
        "renderer game-file handoff",
    )
    if actual_handoff != expected_handoff:
        raise LR.ResolutionError("renderer staged-root handoff changed")
    return expected_launcher


def _implementation_inventory() -> list[dict[str, Any]]:
    paths = sorted(HERE.glob("*.py"), key=lambda path: path.name.casefold())
    paths.extend([
        TEMPLATES_GATE,
        REPO_ROOT / "replay_interface/game_assets/extract.py",
        REPO_ROOT / "replay_interface/game_assets/paradox_parser.py",
        HERE / "gfx_icon_missing_texture_allowlist.json",
    ])
    return [
        {"path": str(path.resolve()), "sha256": SB.sha256_file(path)}
        for path in paths
    ]


def _build_input_fingerprint(
    bindings_dir: Path, vanilla: Path, exe: Path, source_inventory: dict[str, Any]
) -> dict[str, Any]:
    binding_files = [
        "mod_state.json", "dlc_state.json", "load_order.json", UI14_ENGINE_VFS_FILE,
        "resolution_report.json", "source_inventory.json",
    ]
    value = {
        "schema": FINGERPRINT_SCHEMA,
        "vanilla_root": str(vanilla.resolve()),
        "executable_path": str(exe.resolve()),
        "executable_sha256": SB.sha256_file(exe),
        "binding_files": [
            {"name": name, "sha256": SB.sha256_file(bindings_dir / name)}
            for name in binding_files
        ],
        "source_inventory_sha256": source_inventory["inventory_sha256"],
        "implementation_files": _implementation_inventory(),
    }
    value["fingerprint_sha256"] = SB.sha256_bytes(SB.canonical_json(value))
    return value


def _require_build_input_fingerprint_unchanged(
    expected: dict[str, Any],
    bindings_dir: Path,
    vanilla: Path,
    exe: Path,
    source_inventory: dict[str, Any],
) -> None:
    current = _build_input_fingerprint(
        bindings_dir, vanilla, exe, source_inventory
    )
    if current != expected:
        raise LR.ResolutionError(
            "build input fingerprint changed during static generation"
        )


def _run(name: str, command: list[str], cwd: Path, logs: Path) -> dict[str, Any]:
    proc = subprocess.run(
        command,
        cwd=cwd,
        text=True,
        capture_output=True,
        encoding="utf-8",
        errors="replace",
    )
    (logs / f"{name}.stdout.txt").write_text(proc.stdout, encoding="utf-8")
    (logs / f"{name}.stderr.txt").write_text(proc.stderr, encoding="utf-8")
    result = {
        "name": name,
        "command": command,
        "returncode": proc.returncode,
        "stdout_sha256": SB.sha256_bytes(proc.stdout.encode("utf-8")),
        "stderr_sha256": SB.sha256_bytes(proc.stderr.encode("utf-8")),
    }
    if proc.returncode != 0:
        tail = "\n".join((proc.stderr or proc.stdout).splitlines()[-30:])
        raise LR.ResolutionError(f"static generator {name} failed ({proc.returncode}):\n{tail}")
    return result


def runtime_specs(root: Path) -> tuple[SB.ArtifactSpec, ...]:
    return (
        SB.ArtifactSpec(
            "templates_equipment_catalog",
            root / "templates/templates_equipment_catalog_live_domain59/domain59_static_catalog.json",
            root / "templates/domain59_static_gate_report.json",
            "templates_equipment_catalog_live.static_artifact.v1",
            "templates_equipment_catalog.json",
        ),
        SB.ArtifactSpec(
            "localisation_join",
            root / "localisation/static_localisation_join_r3/localisation_catalog.json",
            root / "localisation/localisation_gate_report.json",
            "static_localisation_join_r3.static_artifact.v1",
            "localisation_join.json",
        ),
        SB.ArtifactSpec(
            "idea_tech_loc_xref_static",
            root / "idea_tech/idea_tech_loc_xref_static/idea_tech_loc_xref_catalog.json",
            root / "idea_tech/idea_tech_static_gate_report.json",
            "idea_tech_loc_xref_static.static_artifact.v1",
            "idea_tech_loc_xref_static.json",
            True,
        ),
        SB.ArtifactSpec(
            "decisions_static_definitions",
            root / "decisions/decisions_static_definitions/decision_definitions_catalog.json",
            root / "decisions/decisions_static_gate_report.json",
            "decisions_static_definitions.static_artifact.v1",
            "decisions_static_definitions.json",
            True,
        ),
        SB.ArtifactSpec(
            "ship_definitions_static",
            root / "ships/ship_definitions_static/ship_definitions_catalog.json",
            root / "ships/ship_definitions_gate_report.json",
            "ship_definitions_static.static_artifact.v1",
            "ship_definitions_static.json",
            True,
        ),
        SB.ArtifactSpec(
            "gfx_icon_asset_manifest_static",
            root / "gfx/gfx_icon_asset_manifest_static/gfx_icon_manifest_catalog.json",
            root / "gfx/gfx_icon_manifest_gate_report.json",
            "gfx_icon_asset_manifest_static.static_artifact.v2",
            "gfx_icon_asset_manifest_static.json",
        ),
        SB.ArtifactSpec(
            "technology_sharing_static",
            root / "technology_sharing/technology_sharing_static/technology_sharing_catalog.json",
            root / "technology_sharing/technology_sharing_gate_report.json",
            "technology_sharing_static.static_artifact.v1",
            "technology_sharing_static.json",
            True,
        ),
        SB.ArtifactSpec(
            "focus_defs_static",
            root / "focus_defs/focus_defs_static/focus_defs_catalog.json",
            root / "focus_defs/focus_defs_static_gate_report.json",
            "focus_defs_static.static_artifact.v1",
            "focus_defs_static.json",
            True,
        ),
    )


def _technology_sharing_command(
    common: list[str], loc_catalog: Path, catalogs: Path
) -> list[str]:
    return [
        sys.executable,
        str(HERE / "technology_sharing_static_gate.py"),
        *common,
        "--localisation-catalog", str(loc_catalog),
        "--out-dir", str(catalogs / "technology_sharing"),
    ]


def build(
    *, bindings_dir: Path, vanilla: Path, output: Path, exe: Path, resume: bool = False
) -> dict[str, Any]:
    bindings_dir = bindings_dir.resolve()
    vanilla = vanilla.resolve()
    exe = exe.resolve()
    output = output.resolve()
    if not vanilla.is_dir() or not exe.is_file():
        raise LR.ResolutionError("vanilla root and executable must both exist")
    try:
        same_install = vanilla.samefile(exe.parent)
    except OSError as exc:
        raise LR.ResolutionError("cannot prove vanilla root/executable installation identity") from exc
    if not same_install:
        raise LR.ResolutionError("vanilla root must be the executable parent directory")
    ui14 = SB.load_object(bindings_dir / UI14_ENGINE_VFS_FILE, "UI14 engine VFS receipt")
    if ui14.get("executable_sha256") != SB.sha256_file(exe):
        raise LR.ResolutionError("UI14 engine VFS receipt executable mismatch")
    if output.exists() and not resume:
        raise LR.ResolutionError(f"output already exists: {output}")
    if resume and not output.is_dir():
        raise LR.ResolutionError("--resume requires an existing build output directory")
    mod_state_for_inventory = SB.load_object(bindings_dir / "mod_state.json", "resolved mod state")
    SB._validate_binding_document(mod_state_for_inventory, "mod_state")
    resolved_sources = _load_resolved_mod_sources(bindings_dir, mod_state_for_inventory)
    recorded_source_inventory = LR.validate_source_inventory_document(
        SB.load_object(bindings_dir / "source_inventory.json", "live source inventory")
    )
    active_dlc_ids_for_inventory = [
        row["id"] for row in recorded_source_inventory["namespaces"]
        if row["kind"] == "dlc" and row["binding"] == "active_product"
    ]
    extra_asset_dlc_ids = [
        row["id"] for row in recorded_source_inventory["namespaces"]
        if row["kind"] == "dlc" and row["binding"] == "asset_only"
    ]
    recomputed_before = LR.recompute_source_inventory(
        vanilla,
        resolved_sources,
        active_dlc_ids_for_inventory,
        extra_asset_dlc_ids,
        recorded_source_inventory["identity"],
    )
    if recomputed_before != recorded_source_inventory:
        raise LR.ResolutionError("live source inventory changed before static generation")
    fingerprint = _build_input_fingerprint(
        bindings_dir, vanilla, exe, recorded_source_inventory
    )
    fingerprint_path = output / "build_input_fingerprint.json"
    if resume:
        existing_fingerprint = SB.load_object(fingerprint_path, "build input fingerprint")
        if existing_fingerprint != fingerprint:
            raise LR.ResolutionError("resume input fingerprint mismatch")
    else:
        output.mkdir(parents=True, exist_ok=False)
        fingerprint_path.write_bytes(SB.pretty_json(fingerprint))
    logs = output / "logs"
    logs.mkdir(exist_ok=resume)
    staging_userdir = output / "runtime_userdir"
    if staging_userdir.exists():
        launcher = _validate_runtime_userdir(bindings_dir, vanilla, staging_userdir)
    else:
        launcher = prepare_runtime_userdir(bindings_dir, vanilla, staging_userdir)
    explicit_mods = list(launcher.get("enabled_mods", []))
    mod_arg = ",".join(explicit_mods)
    catalogs = output / "catalogs"
    catalogs.mkdir(exist_ok=resume)
    loc_out = catalogs / "localisation"
    localisation_command = [
        sys.executable,
        str(HERE / "localisation_gate.py"),
        "--vanilla", str(vanilla),
        "--userdata", str(staging_userdir),
        "--mods", mod_arg,
        "--strict-mods",
        "--source-inventory", str(bindings_dir / "source_inventory.json"),
        "--out-dir", str(loc_out),
    ]
    results = []
    loc_report = loc_out / "localisation_gate_report.json"
    loc_passed = loc_report.is_file() and SB.load_object(loc_report, "localisation report").get("passed") is True
    if not loc_passed:
        results.append(_run("localisation", localisation_command, REPO_ROOT, logs))
    template_command = [
        sys.executable,
        str(TEMPLATES_GATE),
        "--vanilla-root", str(vanilla),
        "--source-inventory", str(bindings_dir / "source_inventory.json"),
        "--out-dir", str(catalogs / "templates"),
    ]
    if explicit_mods:
        template_command.extend(
            ["--mods", mod_arg, "--userdata", str(staging_userdir), "--strict-mods"]
        )
    else:
        template_command.append("--vanilla-only")
    commands = [("templates", template_command)]
    loc_catalog = loc_out / "static_localisation_join_r3/localisation_catalog.json"
    common = [
        "--vanilla", str(vanilla),
        "--userdata", str(staging_userdir),
        "--mods", mod_arg,
        "--strict-mods",
        "--source-inventory", str(bindings_dir / "source_inventory.json"),
    ]
    commands.extend(
        [
            (
                "idea_tech",
                [sys.executable, str(HERE / "idea_tech_static_gate.py"), *common,
                 "--localisation-catalog", str(loc_catalog), "--out-dir", str(catalogs / "idea_tech")],
            ),
            (
                "decisions",
                [sys.executable, str(HERE / "decisions_static_gate.py"), *common,
                 "--localisation-catalog", str(loc_catalog), "--out-dir", str(catalogs / "decisions")],
            ),
            (
                "ships",
                [sys.executable, str(HERE / "ship_definitions_gate.py"), *common,
                 "--localisation-catalog", str(loc_catalog), "--out-dir", str(catalogs / "ships")],
            ),
            (
                "gfx",
                [sys.executable, str(HERE / "gfx_icon_manifest_gate.py"), *common,
                 "--out-dir", str(catalogs / "gfx")],
            ),
            (
                "technology_sharing",
                _technology_sharing_command(
                    common,
                    loc_catalog,
                    catalogs,
                ),
            ),
            (
                "focus_defs",
                [sys.executable, str(HERE / "focus_defs_static_gate.py"), *common,
                 "--localisation-catalog", str(loc_catalog), "--out-dir", str(catalogs / "focus_defs")],
            ),
        ]
    )
    report_by_name = {
        "templates": catalogs / "templates/domain59_static_gate_report.json",
        "idea_tech": catalogs / "idea_tech/idea_tech_static_gate_report.json",
        "decisions": catalogs / "decisions/decisions_static_gate_report.json",
        "ships": catalogs / "ships/ship_definitions_gate_report.json",
        "gfx": catalogs / "gfx/gfx_icon_manifest_gate_report.json",
        "technology_sharing": catalogs / "technology_sharing/technology_sharing_gate_report.json",
        "focus_defs": catalogs / "focus_defs/focus_defs_static_gate_report.json",
    }
    commands = [
        (name, command)
        for name, command in commands
        if not (
            report_by_name[name].is_file()
            and SB.load_object(report_by_name[name], f"{name} report").get("passed") is True
        )
    ]
    with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, len(commands))) as pool:
        futures = {
            pool.submit(_run, name, command, REPO_ROOT, logs): name
            for name, command in commands
        }
        for future in concurrent.futures.as_completed(futures):
            results.append(future.result())
    results.sort(key=lambda row: row["name"])

    _validate_runtime_userdir(bindings_dir, vanilla, staging_userdir)
    recomputed_after = LR.recompute_source_inventory(
        vanilla,
        resolved_sources,
        active_dlc_ids_for_inventory,
        extra_asset_dlc_ids,
        recorded_source_inventory["identity"],
    )
    if recomputed_after != recorded_source_inventory:
        raise LR.ResolutionError("live source inventory changed during static generation")
    _require_build_input_fingerprint_unchanged(
        fingerprint, bindings_dir, vanilla, exe, recorded_source_inventory
    )

    specs = runtime_specs(catalogs)
    validated = {spec.role: SB.validate_artifact(spec) for spec in specs}
    resolution = SB.load_object(bindings_dir / "resolution_report.json", "live resolution report")
    dlc_state = SB.load_object(bindings_dir / "dlc_state.json", "resolved DLC state")
    active_dlc_ids = [row["id"] for row in dlc_state["dlc"] if row["enabled"]]
    allowed_extra_asset_layers = {
        row["id"] for row in resolution.get("descriptorless_asset_layers", [])
    }
    mod_state = SB.load_object(bindings_dir / "mod_state.json", "resolved mod state")
    SB._validate_binding_document(mod_state, "mod_state")
    mod_sources = _load_resolved_mod_sources(bindings_dir, mod_state)
    active_mod_names = [row["name"] for row in mod_sources]
    blockers = LR.catalog_compatibility(
        validated, active_mod_names, active_dlc_ids, allowed_extra_asset_layers
    )
    if blockers:
        raise LR.ResolutionError("rebuilt catalog compatibility failed: " + "; ".join(blockers))
    deferred = output / "deferred_token_join.json"
    deferred.write_bytes(
        SB.pretty_json(
            {
                "schema": SB.DEFER_PROVENANCE_SCHEMA,
                "requested_by": "extractor-static-runtime-builder",
                "reason": "static bundle built before emitted-token inventory exists",
                "session_plan_id": resolution["binding_identity"]["profile_id"],
            }
        )
    )
    bundle = output / "bundle"
    SB.create_bundle(
        bundle,
        exe=exe,
        mod_state=bindings_dir / "mod_state.json",
        dlc_state=bindings_dir / "dlc_state.json",
        load_order=bindings_dir / "load_order.json",
        source_inventory=bindings_dir / "source_inventory.json",
        live_resolution_report=bindings_dir / "resolution_report.json",
        specs=specs,
        defer_token_gate=True,
        defer_provenance=deferred,
    )
    verified = SB.verify_bundle(
        bundle,
        exe=exe,
        mod_state=bindings_dir / "mod_state.json",
        dlc_state=bindings_dir / "dlc_state.json",
        load_order=bindings_dir / "load_order.json",
        source_inventory=bindings_dir / "source_inventory.json",
        live_resolution_report=bindings_dir / "resolution_report.json",
        require_token_gate=False,
    )
    _require_build_input_fingerprint_unchanged(
        fingerprint, bindings_dir, vanilla, exe, recorded_source_inventory
    )
    report = {
        "schema": BUILD_SCHEMA,
        "passed": True,
        "runtime_launcher_state": launcher,
        "renderer_game_files": SB.load_object(
            staging_userdir / "renderer_game_file_handoff.json",
            "renderer game-file handoff",
        ),
        "renderer_game_files_path": str(
            (staging_userdir / "renderer_game_file_handoff.json").resolve()
        ),
        "preserved_launcher_conflicts": resolution.get("launcher_dlc_conflicts", []),
        "generators": results,
        "bundle": verified,
        "bundle_path": str(bundle),
    }
    (output / "build_report.json").write_bytes(SB.pretty_json(report))
    return report


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bindings-dir", type=Path, required=True)
    parser.add_argument("--vanilla", type=Path, required=True)
    parser.add_argument("--exe", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--resume", action="store_true")
    args = parser.parse_args(argv)
    try:
        report = build(
            bindings_dir=args.bindings_dir,
            vanilla=args.vanilla,
            output=args.out,
            exe=args.exe,
            resume=args.resume,
        )
    except (LR.ResolutionError, SB.BundleError, OSError, json.JSONDecodeError) as exc:
        print(f"runtime static catalog build: FAIL: {exc}", file=sys.stderr)
        return 1
    print(json.dumps(report, ensure_ascii=False, sort_keys=True, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
