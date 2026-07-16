#!/usr/bin/env python3
"""Resolve the launched the game mod/DLC/load-order state into strict v1 bindings.

The resolver is deliberately fail-closed.  ``system.log`` is accepted only when
its first timestamp matches the queried process creation time and its reported
active counts agree with the enumerated active names.  ``dlc_load.json`` is
retained as launcher evidence, but the launched runtime list wins when the two
conflict.  Missing descriptors, duplicate names, mutable files, or catalog/live
layer mismatches stop bundle creation.
"""

from __future__ import annotations

import argparse
import ctypes
import ctypes.wintypes
import datetime as dt
import json
import re
import sys
import zipfile
from pathlib import Path, PurePosixPath
from typing import Any, Iterable

import static_bundle as SB


RESOLUTION_SCHEMA = "extractor.live_binding_resolution.v1"
SOURCE_INVENTORY_SCHEMA = "extractor.live_binding_source_inventory.v2"
EXPECTED_EXE_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
ACTIVE_COUNT_RE = re.compile(r"Active (DLC|Mod) Count:\s*(\d+)\s*$")
ACTIVE_NAME_RE = re.compile(r"Active (DLC|Mod):\s*(.+?)\s*$")
VERSION_RE = re.compile(r"Version:\s*(.+?)\s*$")
LOG_TIME_RE = re.compile(r"^\[(\d{2}):(\d{2}):(\d{2})\]")
DESCRIPTOR_RE = re.compile(
    r"^\s*([A-Za-z0-9_]+)\s*=\s*(?:\"([^\"]*)\"|([^#\s]+))", re.MULTILINE
)


class ResolutionError(SB.BundleError):
    pass


def parse_descriptor(path: Path) -> dict[str, Any]:
    if not path.is_file():
        raise ResolutionError(f"missing descriptor: {path}")
    text = path.read_text(encoding="utf-8-sig", errors="strict")
    matches: dict[str, list[str]] = {}
    for key, quoted, bare in DESCRIPTOR_RE.findall(text):
        matches.setdefault(key.lower(), []).append(quoted or bare)
    for key in ("name", "path", "archive"):
        if len(matches.get(key, [])) > 1:
            raise ResolutionError(f"descriptor contains duplicate {key}: {path}")
    content_keys = [key for key in ("path", "archive") if matches.get(key)]
    if len(matches.get("name", [])) != 1 or len(content_keys) != 1:
        raise ResolutionError(
            f"descriptor requires one name and exactly one path/archive: {path}"
        )
    fields: dict[str, Any] = {
        key: values[0] for key, values in matches.items()
        if values and key != "replace_path"
    }
    fields["replace_path"] = [
        value.replace("\\", "/").strip("/")
        for value in matches.get("replace_path", [])
        if value.replace("\\", "/").strip("/")
    ]
    return fields


def parse_runtime_log(path: Path) -> dict[str, Any]:
    if not path.is_file():
        raise ResolutionError(f"missing runtime system log: {path}")
    lines = path.read_text(encoding="utf-8", errors="strict").splitlines()
    if not lines:
        raise ResolutionError("runtime system log is empty")
    first_time = LOG_TIME_RE.match(lines[0])
    if not first_time:
        raise ResolutionError("runtime system log has no launch timestamp")
    version = None
    counts: dict[str, int] = {}
    names: dict[str, list[str]] = {"DLC": [], "Mod": []}
    for line in lines:
        if match := VERSION_RE.search(line):
            version = match.group(1)
        if match := ACTIVE_COUNT_RE.search(line):
            counts[match.group(1)] = int(match.group(2))
        if match := ACTIVE_NAME_RE.search(line):
            names[match.group(1)].append(match.group(2))
    if version is None or set(counts) != {"DLC", "Mod"}:
        raise ResolutionError("runtime log lacks version or active DLC/mod counts")
    for kind in ("DLC", "Mod"):
        if counts[kind] != len(names[kind]):
            raise ResolutionError(
                f"runtime log {kind} count mismatch: {counts[kind]} != {len(names[kind])} names"
            )
        if len(names[kind]) != len(set(names[kind])):
            raise ResolutionError(f"runtime log contains duplicate active {kind} names")
    return {
        "first_time": ":".join(first_time.groups()),
        "version": version,
        "active_dlc": names["DLC"],
        "active_mods": names["Mod"],
    }


def _filetime_to_datetime(low: int, high: int) -> dt.datetime:
    ticks = (high << 32) | low
    return dt.datetime(1601, 1, 1, tzinfo=dt.timezone.utc) + dt.timedelta(microseconds=ticks / 10)


def _datetime_to_filetime(value: dt.datetime) -> int:
    if value.tzinfo is None:
        raise ResolutionError("process creation time must be timezone-aware")
    delta = value.astimezone(dt.timezone.utc) - dt.datetime(
        1601, 1, 1, tzinfo=dt.timezone.utc
    )
    return (
        delta.days * 864_000_000_000
        + delta.seconds * 10_000_000
        + delta.microseconds * 10
    )


def query_process(pid: int) -> dict[str, Any]:
    if sys.platform != "win32":
        raise ResolutionError("live process identity is Windows-only")
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    handle = kernel32.OpenProcess(0x1000, False, pid)
    if not handle:
        raise ResolutionError(f"cannot open process {pid}: Win32 {ctypes.get_last_error()}")
    try:
        size = ctypes.c_uint32(32768)
        buffer = ctypes.create_unicode_buffer(size.value)
        if not kernel32.QueryFullProcessImageNameW(handle, 0, buffer, ctypes.byref(size)):
            raise ResolutionError(f"cannot query process image: Win32 {ctypes.get_last_error()}")
        creation = ctypes.wintypes.FILETIME()
        exit_time = ctypes.wintypes.FILETIME()
        kernel = ctypes.wintypes.FILETIME()
        user = ctypes.wintypes.FILETIME()
        if not kernel32.GetProcessTimes(
            handle,
            ctypes.byref(creation),
            ctypes.byref(exit_time),
            ctypes.byref(kernel),
            ctypes.byref(user),
        ):
            raise ResolutionError(f"cannot query process creation time: Win32 {ctypes.get_last_error()}")
        created_utc = _filetime_to_datetime(creation.dwLowDateTime, creation.dwHighDateTime)
        return {
            "pid": pid,
            "exe": str(Path(buffer.value).resolve()),
            "created_utc": created_utc,
            "creation_filetime_utc": (creation.dwHighDateTime << 32) | creation.dwLowDateTime,
        }
    finally:
        kernel32.CloseHandle(handle)


def _hash_file_stable(path: Path) -> tuple[str, int]:
    before = path.stat()
    digest = SB.sha256_file(path)
    after = path.stat()
    if (before.st_size, before.st_mtime_ns) != (after.st_size, after.st_mtime_ns):
        raise ResolutionError(f"file changed while hashing: {path}")
    return digest, after.st_size


def hash_tree(
    root: Path,
    *,
    layer_kind: str,
    layer_id: str,
    exclude_top: set[str] | None = None,
    allow_empty: bool = False,
) -> tuple[str, list[dict[str, Any]]]:
    if not root.is_dir():
        raise ResolutionError(f"missing content root: {root}")
    exclude_top = {name.casefold() for name in (exclude_top or set())}
    def enumerate_files() -> list[tuple[str, Path, tuple[int, int]]]:
        found: list[tuple[str, Path, tuple[int, int]]] = []
        seen: set[str] = set()
        for path in sorted(root.rglob("*"), key=lambda item: item.as_posix().casefold()):
            if not path.is_file():
                continue
            rel = path.relative_to(root).as_posix()
            if rel.split("/", 1)[0].casefold() in exclude_top:
                continue
            if (
                "\\" in rel
                or (len(rel) >= 2 and rel[0].isalpha() and rel[1] == ":")
                or PurePosixPath(rel).is_absolute()
                or any(
                part in {"", ".", ".."} for part in PurePosixPath(rel).parts
                )
            ):
                raise ResolutionError(f"unsafe content namespace path: {root} / {rel}")
            folded = rel.casefold()
            if folded in seen:
                raise ResolutionError(f"case-fold collision in content namespace: {root} / {rel}")
            seen.add(folded)
            stat = path.stat()
            found.append((rel, path, (stat.st_size, stat.st_mtime_ns)))
        return found

    before = enumerate_files()
    rows: list[dict[str, Any]] = []
    for rel, path, _identity in before:
        digest, byte_count = _hash_file_stable(path)
        rows.append(
            {
                "layer_kind": layer_kind,
                "layer_id": layer_id,
                "relpath": rel,
                "bytes": byte_count,
                "sha256": digest,
            }
        )
    after = enumerate_files()
    if [(rel, identity) for rel, _path, identity in before] != [
        (rel, identity) for rel, _path, identity in after
    ]:
        raise ResolutionError(f"content namespace changed while hashing: {root}")
    if not rows and not allow_empty:
        raise ResolutionError(f"empty content root: {root}")
    content_rows = [
        {"relpath": row["relpath"], "bytes": row["bytes"], "sha256": row["sha256"]}
        for row in rows
    ]
    return SB.sha256_bytes(SB.canonical_json(content_rows)), rows


def classify_descriptorless_asset_layers(
    vanilla: Path,
    descriptor_parent_ids: set[str],
) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    dlc_root = vanilla / "dlc"
    classifications: list[dict[str, Any]] = []
    prefixed_rows: list[dict[str, Any]] = []
    for asset_layer in sorted(
        (path for path in dlc_root.iterdir() if path.is_dir() and path.name not in descriptor_parent_ids),
        key=lambda path: path.name.casefold(),
    ):
        asset_id = asset_layer.name
        content_sha, inventory = hash_tree(
            asset_layer,
            layer_kind="descriptorless_asset_layer",
            layer_id=asset_id,
            allow_empty=True,
        )
        for row in inventory:
            prefixed = dict(row)
            prefixed["relpath"] = f"dlc/{asset_id}/{row['relpath']}"
            prefixed_rows.append(prefixed)
        integrated_counterpart = (vanilla / "integrated_dlc" / asset_id).is_dir()
        classifications.append(
            {
                "id": asset_id,
                "classification": (
                    "integrated_shadow_asset_layer"
                    if integrated_counterpart
                    else "descriptorless_asset_layer"
                ),
                "descriptor_present": False,
                "integrated_counterpart_present": integrated_counterpart,
                "content_sha256": content_sha,
            }
        )
    return classifications, prefixed_rows


def _zip_namespace_rows(path: Path, layer_id: str) -> tuple[str, list[dict[str, Any]]]:
    before_sha, _ = _hash_file_stable(path)
    rows: list[dict[str, Any]] = []
    seen: set[str] = set()
    try:
        archive = zipfile.ZipFile(path)
    except (OSError, zipfile.BadZipFile) as exc:
        raise ResolutionError(f"invalid mod archive: {path}") from exc
    with archive:
        for info in sorted(archive.infolist(), key=lambda item: item.filename.casefold()):
            if info.is_dir():
                continue
            rel = info.filename.replace("\\", "/")
            parts = PurePosixPath(rel).parts
            if (
                PurePosixPath(rel).is_absolute()
                or "\\" in rel
                or (len(rel) >= 2 and rel[0].isalpha() and rel[1] == ":")
                or any(part in {"", ".", ".."} for part in parts)
            ):
                raise ResolutionError(f"unsafe mod archive member: {rel!r}")
            folded = rel.casefold()
            if folded in seen:
                raise ResolutionError(f"case-fold/duplicate mod archive member: {rel!r}")
            seen.add(folded)
            mode = (info.external_attr >> 16) & 0o170000
            if mode == 0o120000:
                raise ResolutionError(f"symlink mod archive member: {rel!r}")
            data = archive.read(info)
            rows.append({
                "layer_kind": "mod",
                "layer_id": layer_id,
                "relpath": rel,
                "bytes": len(data),
                "sha256": SB.sha256_bytes(data),
            })
    after_sha, _ = _hash_file_stable(path)
    if before_sha != after_sha:
        raise ResolutionError(f"mod archive changed while hashing: {path}")
    if not rows:
        raise ResolutionError(f"empty mod archive: {path}")
    content_rows = [
        {"relpath": row["relpath"], "bytes": row["bytes"], "sha256": row["sha256"]}
        for row in rows
    ]
    return SB.sha256_bytes(SB.canonical_json(content_rows)), rows


def recompute_source_inventory(
    vanilla: Path,
    resolved_mod_sources: list[dict[str, Any]],
    enabled_dlc_ids: list[str],
    extra_asset_dlc_ids: list[str],
    identity: dict[str, str],
) -> dict[str, Any]:
    """Re-enumerate the ordered effective source namespaces."""
    vanilla = vanilla.resolve()
    descriptor_by_id = {
        path.parent.name: (path, parse_descriptor(path))
        for path in sorted((vanilla / "dlc").glob("*/*.dlc"), key=lambda item: item.as_posix().casefold())
    }
    namespaces: list[dict[str, Any]] = []
    rows: list[dict[str, Any]] = []

    def add_namespace(
        kind: str,
        layer_id: str,
        label: str,
        root: Path,
        replace_paths: list[str] | None = None,
        allow_empty: bool = False,
        archive: bool = False,
        exclude_top: set[str] | None = None,
        binding: str = "asset_only",
        binding_content_sha256: str | None = None,
    ) -> None:
        ordinal = len(namespaces)
        if archive:
            content_sha, inventory = _zip_namespace_rows(root, layer_id)
        else:
            content_sha, inventory = hash_tree(
                root,
                layer_kind=kind,
                layer_id=layer_id,
                allow_empty=allow_empty,
                exclude_top=exclude_top,
            )
        namespaces.append({
            "ordinal": ordinal,
            "kind": kind,
            "binding": binding,
            "id": layer_id,
            "label": label,
            "replace_paths": list(replace_paths or []),
            "file_count": len(inventory),
            "content_sha256": content_sha,
            "binding_content_sha256": binding_content_sha256 or content_sha,
        })
        rows.extend({"namespace_ordinal": ordinal, **row} for row in inventory)

    add_namespace(
        "vanilla", "vanilla", "vanilla", vanilla,
        allow_empty=False, exclude_top={"dlc", "integrated_dlc"}, binding="base"
    )

    integrated_root = vanilla / "integrated_dlc"
    if integrated_root.is_dir():
        for path in sorted((item for item in integrated_root.iterdir() if item.is_dir()), key=lambda item: item.name.casefold()):
            add_namespace(
                "integrated_dlc", path.name, f"integrated_dlc/{path.name}", path,
                allow_empty=True, binding="integrated"
            )
    active_products = set(enabled_dlc_ids)
    asset_only = set(extra_asset_dlc_ids)
    for dlc_id in sorted(active_products | asset_only, key=str.casefold):
        root = vanilla / "dlc" / dlc_id
        descriptor = descriptor_by_id.get(dlc_id)
        replace_paths = descriptor[1].get("replace_path", []) if descriptor else []
        add_namespace(
            "dlc", dlc_id, f"dlc/{dlc_id}", root, replace_paths,
            allow_empty=True,
            binding="active_product" if dlc_id in active_products else "asset_only",
        )
    for source in resolved_mod_sources:
        descriptor = parse_descriptor(Path(source["descriptor_path"]))
        add_namespace(
            "mod",
            source["id"],
            source["name"],
            Path(source["content_path"]),
            descriptor.get("replace_path", []),
            archive=source["content_kind"] == "archive",
            binding="active_mod",
            binding_content_sha256=source["content_sha256"],
        )
    namespace_sha = SB.sha256_bytes(SB.canonical_json(namespaces))
    rows_sha = SB.sha256_bytes(SB.canonical_json(rows))
    core = {
        "schema": SOURCE_INVENTORY_SCHEMA,
        "identity": identity,
        "namespaces": namespaces,
        "rows": rows,
        "namespace_sha256": namespace_sha,
        "rows_sha256": rows_sha,
    }
    return {**core, "inventory_sha256": SB.sha256_bytes(SB.canonical_json(core))}


def validate_source_inventory_document(value: dict[str, Any]) -> dict[str, Any]:
    required = {
        "schema", "identity", "namespaces", "rows", "namespace_sha256",
        "rows_sha256", "inventory_sha256",
    }
    if set(value) != required or value.get("schema") != SOURCE_INVENTORY_SCHEMA:
        raise ResolutionError("source inventory has unsupported schema or fields")
    identity = value.get("identity")
    if not isinstance(identity, dict) or set(identity) != {"playset_id", "profile_id"}:
        raise ResolutionError("source inventory identity is malformed")
    identity = {
        "playset_id": SB._required_string(identity.get("playset_id"), "source identity.playset_id"),
        "profile_id": SB._required_string(identity.get("profile_id"), "source identity.profile_id"),
    }
    namespaces = value.get("namespaces")
    rows = value.get("rows")
    if not isinstance(namespaces, list) or not namespaces or not isinstance(rows, list) or not rows:
        raise ResolutionError("source inventory namespaces/rows must be non-empty arrays")
    normalized_namespaces: list[dict[str, Any]] = []
    seen_labels: set[str] = set()
    for ordinal, namespace in enumerate(namespaces):
        if not isinstance(namespace, dict) or set(namespace) != {
            "ordinal", "kind", "binding", "id", "label", "replace_paths", "file_count",
            "content_sha256", "binding_content_sha256"
        }:
            raise ResolutionError(f"source namespace {ordinal} is malformed")
        if namespace.get("ordinal") != ordinal or namespace.get("kind") not in {
            "vanilla", "integrated_dlc", "dlc", "mod"
        }:
            raise ResolutionError("source namespace order/kind is invalid")
        if namespace.get("binding") not in {
            "base", "integrated", "active_product", "asset_only", "active_mod"
        }:
            raise ResolutionError("source namespace binding class is invalid")
        label = SB._required_string(namespace.get("label"), f"namespace {ordinal}.label")
        if label in seen_labels:
            raise ResolutionError(f"duplicate source namespace label: {label}")
        seen_labels.add(label)
        replacement = namespace.get("replace_paths")
        if not isinstance(replacement, list) or not all(isinstance(item, str) and item for item in replacement):
            raise ResolutionError(f"namespace {ordinal}.replace_paths is malformed")
        file_count = namespace.get("file_count")
        if not isinstance(file_count, int) or isinstance(file_count, bool) or file_count < 0:
            raise ResolutionError(f"namespace {ordinal}.file_count is invalid")
        normalized_namespaces.append({
            "ordinal": ordinal,
            "kind": namespace["kind"],
            "binding": namespace["binding"],
            "id": SB._required_string(namespace.get("id"), f"namespace {ordinal}.id"),
            "label": label,
            "replace_paths": replacement,
            "file_count": file_count,
            "content_sha256": SB._required_sha(namespace.get("content_sha256"), f"namespace {ordinal}.content_sha256"),
            "binding_content_sha256": SB._required_sha(
                namespace.get("binding_content_sha256"),
                f"namespace {ordinal}.binding_content_sha256",
            ),
        })
    normalized_rows: list[dict[str, Any]] = []
    seen_rows: set[tuple[int, str]] = set()
    for index, row in enumerate(rows):
        if not isinstance(row, dict) or set(row) != {
            "namespace_ordinal", "layer_kind", "layer_id", "relpath", "bytes", "sha256"
        }:
            raise ResolutionError(f"source inventory row {index} is malformed")
        ordinal = row.get("namespace_ordinal")
        if not isinstance(ordinal, int) or isinstance(ordinal, bool) or not 0 <= ordinal < len(namespaces):
            raise ResolutionError(f"source row {index}.namespace_ordinal is invalid")
        relpath = SB._required_string(row.get("relpath"), f"source row {index}.relpath")
        if (
            "\\" in relpath
            or (len(relpath) >= 2 and relpath[0].isalpha() and relpath[1] == ":")
            or PurePosixPath(relpath).is_absolute()
            or any(
            part in {"", ".", ".."} for part in PurePosixPath(relpath).parts
            )
        ):
            raise ResolutionError(f"source row {index}.relpath is unsafe")
        key = (ordinal, relpath.casefold())
        if key in seen_rows:
            raise ResolutionError(f"duplicate/case-colliding source row: {key}")
        seen_rows.add(key)
        namespace = normalized_namespaces[ordinal]
        if row.get("layer_kind") != namespace["kind"] or row.get("layer_id") != namespace["id"]:
            raise ResolutionError(f"source row {index} namespace identity mismatch")
        byte_count = row.get("bytes")
        if not isinstance(byte_count, int) or isinstance(byte_count, bool) or byte_count < 0:
            raise ResolutionError(f"source row {index}.bytes is invalid")
        normalized_rows.append({
            "namespace_ordinal": ordinal,
            "layer_kind": namespace["kind"],
            "layer_id": namespace["id"],
            "relpath": relpath,
            "bytes": byte_count,
            "sha256": SB._required_sha(row.get("sha256"), f"source row {index}.sha256"),
        })
    for namespace in normalized_namespaces:
        ns_rows = [row for row in normalized_rows if row["namespace_ordinal"] == namespace["ordinal"]]
        if len(ns_rows) != namespace["file_count"]:
            raise ResolutionError("source namespace file_count mismatch")
        content_rows = [
            {"relpath": row["relpath"], "bytes": row["bytes"], "sha256": row["sha256"]}
            for row in ns_rows
        ]
        if namespace["content_sha256"] != SB.sha256_bytes(SB.canonical_json(content_rows)):
            raise ResolutionError("source namespace content SHA-256 mismatch")
    namespace_sha = SB.sha256_bytes(SB.canonical_json(normalized_namespaces))
    rows_sha = SB.sha256_bytes(SB.canonical_json(normalized_rows))
    core = {
        "schema": SOURCE_INVENTORY_SCHEMA,
        "identity": identity,
        "namespaces": normalized_namespaces,
        "rows": normalized_rows,
        "namespace_sha256": namespace_sha,
        "rows_sha256": rows_sha,
    }
    expected_inventory = SB.sha256_bytes(SB.canonical_json(core))
    if (
        value.get("namespace_sha256") != namespace_sha
        or value.get("rows_sha256") != rows_sha
        or value.get("inventory_sha256") != expected_inventory
    ):
        raise ResolutionError("source inventory digest mismatch")
    return {**core, "inventory_sha256": expected_inventory}


def _runtime_log_matches_process(runtime: dict[str, Any], process: dict[str, Any]) -> None:
    local_created = process["created_utc"].astimezone()
    hh, mm, ss = (int(part) for part in runtime["first_time"].split(":"))
    logged = local_created.replace(hour=hh, minute=mm, second=ss, microsecond=0)
    if abs((logged - local_created).total_seconds()) > 5:
        raise ResolutionError(
            f"system.log launch time {runtime['first_time']} does not match process creation "
            f"{local_created.strftime('%H:%M:%S')}"
        )


def _launcher_state(userdir: Path) -> tuple[dict[str, Any], bytes]:
    path = userdir / "dlc_load.json"
    before = path.read_bytes()
    try:
        state = json.loads(before.decode("utf-8"))
    except (UnicodeError, json.JSONDecodeError) as exc:
        raise ResolutionError(f"invalid dlc_load.json: {exc}") from exc
    if not isinstance(state, dict) or set(state) != {"enabled_mods", "disabled_dlcs"}:
        raise ResolutionError("dlc_load.json has an unsupported schema")
    if not all(isinstance(value, list) and all(isinstance(item, str) for item in value) for value in state.values()):
        raise ResolutionError("dlc_load.json lists must contain strings")
    if len(state["enabled_mods"]) != len(set(state["enabled_mods"])):
        raise ResolutionError("dlc_load.json contains duplicate enabled mods")
    return state, before


def _catalog_mod_names(catalog: dict[str, Any]) -> list[str]:
    layers = catalog.get("layers")
    if not isinstance(layers, list) or not layers:
        raise ResolutionError("catalog layers must be a non-empty array")
    names: list[str] = []
    for layer in layers:
        if not isinstance(layer, dict):
            raise ResolutionError("catalog layer must be an object")
        label = layer.get("label")
        if not isinstance(label, str) or not label:
            raise ResolutionError("catalog layer label must be a non-empty string")
        meta = layer.get("meta") if isinstance(layer.get("meta"), dict) else {}
        if meta.get("kind") == "mod":
            names.append(label)
        elif label != "vanilla" and not label.startswith(("dlc/", "integrated_dlc/")):
            names.append(label)
    if len(names) != len(set(names)):
        raise ResolutionError("catalog contains duplicate mod layer labels")
    return names


def catalog_compatibility(
    validated: dict[str, dict[str, Any]],
    active_mod_names: list[str],
    active_dlc_ids: list[str],
    allowed_extra_asset_layers: set[str] | None = None,
) -> list[str]:
    blockers: list[str] = []
    wanted_mods = list(active_mod_names)
    for role, info in validated.items():
        try:
            actual = _catalog_mod_names(info["catalog"])
        except ResolutionError as exc:
            blockers.append(f"{role}: {exc}")
            continue
        if actual != wanted_mods:
            blockers.append(
                f"{role}: catalog mod layers {actual} != runtime active mods {wanted_mods}"
            )
    gfx = validated["gfx_icon_asset_manifest_static"]["catalog"]
    layers = gfx.get("layers")
    vanilla_meta = None
    if isinstance(layers, list):
        vanilla = next((row for row in layers if isinstance(row, dict) and row.get("label") == "vanilla"), None)
        if isinstance(vanilla, dict) and isinstance(vanilla.get("meta"), dict):
            vanilla_meta = vanilla["meta"]
    recorded_dlcs = set(vanilla_meta.get("enabled_dlcs", [])) if vanilla_meta else set()
    active_dlcs = set(active_dlc_ids)
    allowed_extra_asset_layers = allowed_extra_asset_layers or set()
    missing_active_dlcs = active_dlcs - recorded_dlcs
    unexpected_asset_layers = (recorded_dlcs - active_dlcs) - allowed_extra_asset_layers
    if missing_active_dlcs:
        blockers.append(
            "gfx_icon_asset_manifest_static: catalog is missing runtime active DLC ids "
            f"{sorted(missing_active_dlcs)}"
        )
    if unexpected_asset_layers:
        blockers.append(
            "gfx_icon_asset_manifest_static: catalog has unclassified extra DLC asset layers "
            f"{sorted(unexpected_asset_layers)}"
        )
    return blockers


def resolve_live_bindings(
    *,
    exe: Path,
    userdir: Path,
    pid: int,
    out_dir: Path,
    process: dict[str, Any] | None = None,
) -> dict[str, Any]:
    process = process or query_process(pid)
    exe = exe.resolve()
    if Path(process["exe"]).resolve() != exe:
        raise ResolutionError(f"PID {pid} executable does not match requested hoi4.exe")
    exe_sha, exe_bytes = _hash_file_stable(exe)
    if exe_sha != EXPECTED_EXE_SHA256:
        raise ResolutionError(f"unexpected hoi4.exe SHA-256: {exe_sha}")
    runtime = parse_runtime_log(userdir / "logs/system.log")
    _runtime_log_matches_process(runtime, process)
    launcher, launcher_bytes = _launcher_state(userdir)
    if len(launcher["enabled_mods"]) != len(runtime["active_mods"]):
        raise ResolutionError("launcher enabled-mod count does not match runtime active-mod count")

    vanilla = exe.parent
    dlc_root = vanilla / "dlc"
    descriptors = sorted(dlc_root.glob("*/*.dlc"), key=lambda path: path.as_posix().casefold())
    if not descriptors:
        raise ResolutionError("no installed DLC descriptors found")
    descriptor_by_name: dict[str, tuple[Path, dict[str, str]]] = {}
    for descriptor in descriptors:
        fields = parse_descriptor(descriptor)
        if fields["name"] in descriptor_by_name:
            raise ResolutionError(f"duplicate installed DLC name: {fields['name']}")
        descriptor_by_name[fields["name"]] = (descriptor, fields)
    missing_runtime_dlcs = sorted(set(runtime["active_dlc"]) - set(descriptor_by_name))
    if missing_runtime_dlcs:
        raise ResolutionError(f"runtime active DLC names lack installed descriptors: {missing_runtime_dlcs}")

    mod_rows: list[dict[str, Any]] = []
    mod_inventory: list[dict[str, Any]] = []
    mod_names: list[str] = []
    resolved_mod_sources: list[dict[str, Any]] = []
    for ordinal, token in enumerate(launcher["enabled_mods"]):
        descriptor = (userdir / token).resolve()
        fields = parse_descriptor(descriptor)
        content_ref = fields.get("path") or fields["archive"]
        content_root = (userdir / content_ref).resolve() if not Path(content_ref).is_absolute() else Path(content_ref).resolve()
        if content_root.is_file():
            content_sha, content_bytes = _hash_file_stable(content_root)
            inventory = [{
                "layer_kind": "mod",
                "layer_id": token,
                "relpath": content_root.name,
                "bytes": content_bytes,
                "sha256": content_sha,
            }]
        else:
            content_sha, inventory = hash_tree(content_root, layer_kind="mod", layer_id=token)
        mod_inventory.extend(inventory)
        mod_names.append(fields["name"])
        descriptor_sha, _ = _hash_file_stable(descriptor)
        normalized_id = token.replace("\\", "/")
        mod_rows.append(
            {
                "id": normalized_id,
                "name": fields["name"],
                "descriptor_sha256": descriptor_sha,
                "content_sha256": content_sha,
                "enabled": True,
            }
        )
        resolved_mod_sources.append(
            {
                "id": normalized_id,
                "ordinal": ordinal,
                "name": fields["name"],
                "descriptor_path": str(descriptor),
                "descriptor_sha256": descriptor_sha,
                "content_kind": "archive" if "archive" in fields else "directory",
                "content_path": str(content_root),
                "content_sha256": content_sha,
            }
        )
    if mod_names != runtime["active_mods"]:
        raise ResolutionError(
            f"launcher mod order/names {mod_names} != runtime active mods {runtime['active_mods']}"
        )

    source_rows: list[dict[str, Any]] = []
    vanilla_sha, vanilla_inventory = hash_tree(
        vanilla, layer_kind="vanilla", layer_id="vanilla", exclude_top={"dlc"}
    )
    descriptor_parent_ids = {descriptor.parent.name for descriptor in descriptors}
    descriptorless_asset_layers, descriptorless_inventory = (
        classify_descriptorless_asset_layers(vanilla, descriptor_parent_ids)
    )
    if descriptorless_inventory:
        vanilla_content_rows = [
            {"relpath": row["relpath"], "bytes": row["bytes"], "sha256": row["sha256"]}
            for row in [*vanilla_inventory, *descriptorless_inventory]
        ]
        vanilla_content_rows.sort(key=lambda row: row["relpath"].casefold())
        vanilla_sha = SB.sha256_bytes(SB.canonical_json(vanilla_content_rows))
    source_rows.extend(vanilla_inventory)
    source_rows.extend(descriptorless_inventory)
    source_rows.extend(mod_inventory)
    dlc_rows: list[dict[str, Any]] = []
    enabled_dlc_ids: list[str] = []
    dlc_content: dict[str, str] = {}
    for name, (descriptor, fields) in sorted(descriptor_by_name.items()):
        dlc_id = descriptor.parent.name
        content_sha, inventory = hash_tree(descriptor.parent, layer_kind="dlc", layer_id=dlc_id)
        source_rows.extend(inventory)
        dlc_content[dlc_id] = content_sha
        enabled = name in runtime["active_dlc"]
        if enabled:
            enabled_dlc_ids.append(dlc_id)
        dlc_rows.append(
            {"id": dlc_id, "name": name, "content_sha256": content_sha, "enabled": enabled}
        )

    launcher_after = (userdir / "dlc_load.json").read_bytes()
    if launcher_after != launcher_bytes:
        raise ResolutionError("dlc_load.json changed during resolution")
    playset_seed = {
        "active_mods": runtime["active_mods"],
        "active_dlcs": runtime["active_dlc"],
        "launcher_sha256": SB.sha256_bytes(launcher_bytes),
    }
    profile_seed = {
        "exe_sha256": exe_sha,
        "version": runtime["version"],
        "process_created_utc": process["created_utc"].isoformat(),
    }
    identity = {
        "playset_id": "runtime-playset:" + SB.sha256_bytes(SB.canonical_json(playset_seed)),
        "profile_id": "runtime-profile:" + SB.sha256_bytes(SB.canonical_json(profile_seed)),
    }
    mod_rows.sort(key=lambda row: row["id"])
    dlc_rows.sort(key=lambda row: row["id"])
    load_rows = [{"id": "vanilla", "kind": "vanilla", "ordinal": 0, "content_sha256": vanilla_sha}]
    ordinal = 1
    for row in sorted((row for row in dlc_rows if row["enabled"]), key=lambda row: row["id"]):
        load_rows.append(
            {"id": row["id"], "kind": "dlc", "ordinal": ordinal, "content_sha256": row["content_sha256"]}
        )
        ordinal += 1
    ordered_mods = {token.replace("\\", "/"): index for index, token in enumerate(launcher["enabled_mods"])}
    for row in sorted(mod_rows, key=lambda row: ordered_mods[row["id"]]):
        load_rows.append(
            {"id": row["id"], "kind": "mod", "ordinal": ordinal, "content_sha256": row["content_sha256"]}
        )
        ordinal += 1
    documents = {
        "mod_state": {
            "schema": SB.MOD_STATE_SCHEMA,
            "identity": identity,
            "mods": mod_rows,
            "effective_content_sha256": SB._content_digest(mod_rows),
        },
        "dlc_state": {
            "schema": SB.DLC_STATE_SCHEMA,
            "identity": identity,
            "dlc": dlc_rows,
            "effective_content_sha256": SB._content_digest(dlc_rows),
        },
        "load_order": {
            "schema": SB.LOAD_ORDER_SCHEMA,
            "identity": identity,
            "entries": load_rows,
            "effective_content_sha256": SB._content_digest(load_rows),
        },
    }
    bindings = {
        role: SB._validate_binding_document(document, role) for role, document in documents.items()
    }
    SB._validate_binding_identity(bindings)
    source_inventory = recompute_source_inventory(
        vanilla,
        resolved_mod_sources,
        enabled_dlc_ids,
        [row["id"] for row in descriptorless_asset_layers],
        identity,
    )
    validate_source_inventory_document(source_inventory)
    disabled_claims = {
        Path(token).parent.name for token in launcher["disabled_dlcs"]
    }
    launched_disabled_conflicts = sorted(disabled_claims & set(enabled_dlc_ids))
    report = {
        "schema": RESOLUTION_SCHEMA,
        "passed": True,
        "process": {
            "pid": pid,
            "created_utc": process["created_utc"].isoformat(),
            "creation_filetime_utc": int(
                process.get(
                    "creation_filetime_utc",
                    _datetime_to_filetime(process["created_utc"]),
                )
            ),
            "exe_name": exe.name,
            "exe_bytes": exe_bytes,
            "exe_sha256": exe_sha,
        },
        "runtime": runtime,
        "launcher_dlc_conflicts": launched_disabled_conflicts,
        "descriptorless_asset_layers": descriptorless_asset_layers,
        "binding_identity": identity,
        "source_inventory_rows": len(source_inventory["rows"]),
        "source_inventory_sha256": source_inventory["inventory_sha256"],
        "resolved_mod_sources": resolved_mod_sources,
    }
    out_dir.mkdir(parents=True, exist_ok=False)
    for role, document in documents.items():
        (out_dir / f"{role}.json").write_bytes(SB.pretty_json(document))
    (out_dir / "source_inventory.json").write_bytes(SB.pretty_json(source_inventory))
    (out_dir / "resolution_report.json").write_bytes(SB.pretty_json(report))
    return {
        "documents": documents,
        "report": report,
        "active_mod_names": mod_names,
        "active_dlc_ids": enabled_dlc_ids,
        "paths": {
            **{role: out_dir / f"{role}.json" for role in documents},
            "source_inventory": out_dir / "source_inventory.json",
            "resolution_report": out_dir / "resolution_report.json",
        },
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True)
    parser.add_argument("--userdir", type=Path, required=True)
    parser.add_argument("--pid", type=int, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--bundle-out", type=Path)
    parser.add_argument("--defer-provenance", type=Path)
    args = parser.parse_args(argv)
    try:
        resolved = resolve_live_bindings(
            exe=args.exe, userdir=args.userdir, pid=args.pid, out_dir=args.out
        )
        validated = {spec.role: SB.validate_artifact(spec) for spec in SB.DEFAULT_SPECS}
        blockers = catalog_compatibility(
            validated,
            resolved["active_mod_names"],
            resolved["active_dlc_ids"],
            {row["id"] for row in resolved["report"]["descriptorless_asset_layers"]},
        )
        if blockers:
            report_path = args.out / "resolution_report.json"
            report = SB.load_object(report_path, "resolution report")
            report["passed"] = False
            report["catalog_blockers"] = blockers
            report_path.write_bytes(SB.pretty_json(report))
            raise ResolutionError("; ".join(blockers))
        if args.bundle_out:
            if not args.defer_provenance:
                raise ResolutionError("--bundle-out requires --defer-provenance")
            SB.create_bundle(
                args.bundle_out,
                exe=args.exe,
                mod_state=resolved["paths"]["mod_state"],
                dlc_state=resolved["paths"]["dlc_state"],
                load_order=resolved["paths"]["load_order"],
                source_inventory=resolved["paths"]["source_inventory"],
                live_resolution_report=resolved["paths"]["resolution_report"],
                defer_token_gate=True,
                defer_provenance=args.defer_provenance,
            )
        print(json.dumps(resolved["report"], ensure_ascii=False, sort_keys=True, indent=2))
        return 0
    except (ResolutionError, OSError, json.JSONDecodeError) as exc:
        print(f"live binding resolver: FAIL: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
