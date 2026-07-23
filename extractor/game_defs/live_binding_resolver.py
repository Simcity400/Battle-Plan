#!/usr/bin/env python3
"""Resolve the launched the game mod/DLC/load-order state into strict v1 bindings.

The resolver is deliberately fail-closed.  ``system.log`` is accepted only when
its first timestamp matches the queried process creation time and its reported
active counts agree with the enumerated active names.  The runtime list proves
which installed products the engine actually activated.  ``dlc_load.json`` is
retained separately as the requested profile selection; it must never remove a
product from the effective runtime roster when the engine reports that product
active.
Missing descriptors, duplicate names, mutable files, or catalog/live layer
mismatches stop bundle creation.
"""

from __future__ import annotations

import argparse
import ctypes
import ctypes.wintypes
import datetime as dt
import json
import re
import struct
import sys
import zipfile
from pathlib import Path, PurePosixPath
from typing import Any, Iterable

import static_bundle as SB


RESOLUTION_SCHEMA = "extractor.live_binding_resolution.v1"
UI14_ENGINE_VFS_SCHEMA = "extractor.ui14_engine_vfs.v1"
SOURCE_INVENTORY_SCHEMA = "extractor.live_binding_source_inventory.v3"
EXPECTED_EXE_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"


def _load_ui14_engine_vfs(
    path: Path, exe_sha: str, userdir: Path, enabled_mods: list[str]
) -> dict[str, Any]:
    receipt = SB.load_object(path, "UI14 engine VFS receipt")
    if set(receipt) != {"schema", "executable_sha256", "layers", "effective_winners"}:
        raise ResolutionError("UI14 engine VFS receipt has unknown or missing fields")
    if receipt["schema"] != UI14_ENGINE_VFS_SCHEMA or receipt["executable_sha256"] != exe_sha:
        raise ResolutionError("UI14 engine VFS receipt executable/schema mismatch")
    ids = [
        _normalize_vfs_relpath(token, "launcher mod descriptor")
        for token in enabled_mods
    ]
    ids_by_key = {_ascii_fold(layer_id): layer_id for layer_id in ids}
    if len(ids) != len(ids_by_key):
        raise ResolutionError("launcher repeats an enabled mod descriptor")
    layers = receipt.get("layers")
    if not isinstance(layers, list) or len(layers) != len(ids):
        raise ResolutionError("UI14 engine VFS layer count does not match enabled mods")
    descriptor_ordinals: set[int] = set()
    mount_ordinals: set[int] = set()
    seen_ids: set[str] = set()
    for row in layers:
        if not isinstance(row, dict) or set(row) != {
            "id", "descriptor_sha256", "engine_descriptor_ordinal", "engine_mount_ordinal"
        }:
            raise ResolutionError("UI14 engine VFS layer row is malformed")
        try:
            layer_id = _normalize_vfs_relpath(
                row["id"], "UI14 mod descriptor"
            ) if isinstance(row["id"], str) else ""
        except ResolutionError:
            layer_id = ""
        layer_key = _ascii_fold(layer_id)
        descriptor_ordinal = row["engine_descriptor_ordinal"]
        mount_ordinal = row["engine_mount_ordinal"]
        if (
            layer_key not in ids_by_key or layer_key in seen_ids
            or not isinstance(descriptor_ordinal, int) or descriptor_ordinal < 0
            or not isinstance(mount_ordinal, int) or mount_ordinal < 0
            or descriptor_ordinal in descriptor_ordinals or mount_ordinal in mount_ordinals
        ):
            raise ResolutionError("UI14 engine VFS layer identity/ordinal is invalid")
        layer_id = ids_by_key[layer_key]
        descriptor = (userdir / layer_id).resolve()
        descriptor_sha, _ = _hash_file_stable(descriptor)
        if descriptor_sha != row["descriptor_sha256"]:
            raise ResolutionError(f"UI14 engine VFS descriptor hash mismatch: {layer_id}")
        seen_ids.add(layer_key)
        descriptor_ordinals.add(descriptor_ordinal)
        mount_ordinals.add(mount_ordinal)
        row["id"] = layer_id
    expected_ordinals = set(range(len(ids)))
    if descriptor_ordinals != expected_ordinals or mount_ordinals != expected_ordinals:
        raise ResolutionError("UI14 engine VFS layer ordinals are not contiguous")
    winners = receipt.get("effective_winners")
    if not isinstance(winners, list):
        raise ResolutionError("UI14 engine VFS effective_winners is not an array")
    winner_keys: set[str] = set()
    for row in winners:
        if not isinstance(row, dict) or set(row) != {"relpath", "winner_id", "sha256"}:
            raise ResolutionError("UI14 engine VFS winner row is malformed")
        try:
            relpath = _normalize_vfs_relpath(
                row["relpath"], "UI14 winner path"
            ) if isinstance(row["relpath"], str) else ""
            winner_id = _normalize_vfs_relpath(
                row["winner_id"], "UI14 winner mod descriptor"
            ) if isinstance(row["winner_id"], str) else ""
        except ResolutionError:
            relpath = ""
            winner_id = ""
        key = _ascii_fold(relpath)
        winner_key = _ascii_fold(winner_id)
        if (
            not relpath
            or key in winner_keys or winner_key not in seen_ids
            or not isinstance(row["sha256"], str)
            or re.fullmatch(r"[0-9a-f]{64}", row["sha256"]) is None
        ):
            raise ResolutionError("UI14 engine VFS winner identity/path/hash is invalid")
        winner_keys.add(key)
        row["relpath"] = relpath
        row["winner_id"] = ids_by_key[winner_key]
    return receipt
ACTIVE_COUNT_RE = re.compile(r"Active (DLC|Mod) Count:\s*(\d+)\s*$")
ACTIVE_NAME_RE = re.compile(r"Active (DLC|Mod):\s*(.+?)\s*$")
VERSION_RE = re.compile(r"Version:\s*(.+?)\s*$")
LOG_TIME_RE = re.compile(r"^\[(\d{2}):(\d{2}):(\d{2})\]")
DESCRIPTOR_RE = re.compile(
    r"^\s*([A-Za-z0-9_]+)\s*=\s*(?:\"([^\"]*)\"|([^#\s]+))", re.MULTILINE
)
QUOTED_VALUE_RE = re.compile(r'"([^"]*)"')


class ResolutionError(SB.BundleError):
    pass


_ASCII_FOLD = str.maketrans(
    "ABCDEFGHIJKLMNOPQRSTUVWXYZ", "abcdefghijklmnopqrstuvwxyz"
)


def _ascii_fold(value: str) -> str:
    return value.translate(_ASCII_FOLD)


def _normalize_vfs_relpath(raw_name: str, label: str) -> str:
    slash = raw_name.replace("\\", "/")
    if (
        not slash
        or slash.startswith("/")
        or "\x00" in slash
        or (len(slash) >= 2 and slash[0].isalpha() and slash[1] == ":")
    ):
        raise ResolutionError(f"unsafe {label}: {raw_name!r}")
    parts: list[str] = []
    for part in slash.split("/"):
        if part in {"", "."}:
            continue
        if part == ".." or ":" in part:
            raise ResolutionError(f"unsafe {label}: {raw_name!r}")
        parts.append(part)
    if not parts:
        raise ResolutionError(f"empty {label}: {raw_name!r}")
    return "/".join(parts)


def parse_descriptor(path: Path) -> dict[str, Any]:
    if not path.is_file():
        raise ResolutionError(f"missing descriptor: {path}")
    # Clausewitz descriptors are byte strings.  Latin-1 is the lossless
    # one-byte mapping also used by the production replay resolver.
    text = path.read_bytes().decode("latin-1")
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
        _normalize_vfs_relpath(value, "descriptor replace_path")
        for value in matches.get("replace_path", [])
    ]
    if len(fields["replace_path"]) != len(
        {_ascii_fold(value) for value in fields["replace_path"]}
    ):
        raise ResolutionError(f"descriptor repeats normalized replace_path: {path}")
    fields["dependencies"] = _descriptor_quoted_list(text, "dependencies")
    return fields


def _descriptor_quoted_list(text: str, wanted: str) -> list[str]:
    """Parse one descriptor ``key = { "value" ... }`` list, fail closed.

    The a729 planner compares dependency entries to exact descriptor display
    names.  Multiple dependency blocks or an unterminated block are therefore
    ambiguous and rejected instead of being guessed.
    """
    starts = list(
        re.finditer(
            rf"(?im)^\s*{re.escape(wanted)}\s*=\s*\{{",
            text,
        )
    )
    if not starts:
        return []
    if len(starts) != 1:
        raise ResolutionError(f"descriptor contains duplicate {wanted}")
    body_start = starts[0].end()
    body_end = text.find("}", body_start)
    if body_end < 0:
        raise ResolutionError(f"descriptor contains unterminated {wanted}")
    body = text[body_start:body_end]
    scrubbed = QUOTED_VALUE_RE.sub("", body)
    if scrubbed.strip():
        raise ResolutionError(f"descriptor contains malformed {wanted}")
    values = [value.strip() for value in QUOTED_VALUE_RE.findall(body) if value.strip()]
    if len(values) != len(set(values)):
        raise ResolutionError(f"descriptor repeats {wanted} entry")
    return values


def _engine_byte_key(value: str) -> bytes:
    """Exact a729 narrow-string ordering key (case-sensitive byte order)."""
    return value.encode("utf-8")


def _relative_descriptor_id(root: Path, descriptor: Path) -> str:
    try:
        value = descriptor.resolve().relative_to(root.resolve()).as_posix()
    except ValueError as exc:
        raise ResolutionError(f"descriptor escapes content root: {descriptor}") from exc
    if not value or any(part in {"", ".", ".."} for part in PurePosixPath(value).parts):
        raise ResolutionError(f"unsafe descriptor identity: {value!r}")
    return value


def _engine_mount_plan(
    dlc_layers: list[dict[str, Any]],
    mod_layers: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    """Reproduce a729's selected-DLC/mod dependency priority planner.

    DLC starts at priority 0 and mods at 10000.  For every dependency display
    name, the matching layer is repeatedly lowered until it precedes the
    dependent.  The final ascending-priority tie break is the exact portable
    descriptor ID.  A dependency cycle has no engine fixed point and is
    rejected rather than hanging the extractor.
    """
    plan = [
        {**row, "priority": 0, "planner_kind": "dlc"} for row in dlc_layers
    ] + [
        {**row, "priority": 10000, "planner_kind": "mod"} for row in mod_layers
    ]
    ids = [row["descriptor_id"] for row in plan]
    if len(ids) != len(set(ids)):
        raise ResolutionError("engine mount plan repeats a descriptor identity")
    if not plan:
        return []

    # Bellman-Ford form of the engine's repeated relaxation.  A stable plan
    # exists after at most N-1 propagation rounds; a change in round N proves
    # an active dependency cycle.
    changed = False
    for round_index in range(len(plan)):
        changed = False
        for dependent in plan:
            dependent_priority = dependent["priority"]
            for dependency_name in dependent.get("dependencies", []):
                for dependency in plan:
                    if dependency["name"] != dependency_name:
                        continue
                    if dependent_priority <= dependency["priority"]:
                        dependency["priority"] -= 1
                        changed = True
        if not changed:
            break
        if round_index == len(plan) - 1:
            raise ResolutionError("active DLC/mod dependencies contain a priority cycle")
    return sorted(
        plan,
        key=lambda row: (row["priority"], _engine_byte_key(row["descriptor_id"])),
    )


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
    exclude_top = {_ascii_fold(name) for name in (exclude_top or set())}
    def enumerate_files() -> list[tuple[str, Path, tuple[int, int]]]:
        found: list[tuple[str, Path, tuple[int, int]]] = []
        seen: set[str] = set()
        for path in sorted(root.rglob("*"), key=lambda item: _ascii_fold(item.as_posix())):
            if not path.is_file():
                continue
            rel = path.relative_to(root).as_posix()
            if _ascii_fold(rel.split("/", 1)[0]) in exclude_top:
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
            folded = _ascii_fold(rel)
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
                "storage_kind": "directory",
                "source_container_sha256": None,
                "source_byte_offset": 0,
                "source_byte_count": byte_count,
                "source_range_sha256": digest,
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
        key=lambda path: _ascii_fold(path.name),
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


def _normalize_archive_member(raw_name: str) -> str:
    """Return the engine-equivalent canonical virtual path for one ZIP member."""
    return _normalize_vfs_relpath(raw_name, "mod archive member")


def _zip_namespace_rows(
    path: Path, layer_id: str, *, layer_kind: str = "mod"
) -> tuple[str, list[dict[str, Any]], str, int]:
    before_sha, before_bytes = _hash_file_stable(path)
    rows: list[dict[str, Any]] = []
    seen: set[str] = set()
    try:
        archive = zipfile.ZipFile(path)
    except (OSError, zipfile.BadZipFile) as exc:
        raise ResolutionError(f"invalid mod archive: {path}") from exc
    with archive, path.open("rb") as raw_archive:
        for info in sorted(
            archive.infolist(), key=lambda item: _ascii_fold(item.filename)
        ):
            if info.is_dir():
                continue
            rel = _normalize_archive_member(info.filename)
            folded = _ascii_fold(rel)
            if folded in seen:
                raise ResolutionError(f"case-fold/duplicate mod archive member: {rel!r}")
            seen.add(folded)
            mode = (info.external_attr >> 16) & 0o170000
            if mode == 0o120000:
                raise ResolutionError(f"symlink mod archive member: {rel!r}")
            if info.flag_bits & 0x1:
                raise ResolutionError(f"encrypted mod archive member: {rel!r}")
            raw_archive.seek(info.header_offset)
            local_header = raw_archive.read(30)
            if len(local_header) != 30:
                raise ResolutionError(f"truncated archive local header: {rel!r}")
            signature, *_, filename_len, extra_len = struct.unpack(
                "<IHHHHHIIIHH", local_header
            )
            if signature != 0x04034B50:
                raise ResolutionError(f"invalid archive local header: {rel!r}")
            source_byte_offset = info.header_offset + 30 + filename_len + extra_len
            raw_archive.seek(source_byte_offset)
            compressed = raw_archive.read(info.compress_size)
            if len(compressed) != info.compress_size:
                raise ResolutionError(f"truncated archive member range: {rel!r}")
            data = archive.read(info)
            rows.append({
                "layer_kind": layer_kind,
                "layer_id": layer_id,
                "relpath": rel,
                "bytes": len(data),
                "sha256": SB.sha256_bytes(data),
                "storage_kind": "zip_archive",
                "source_container_sha256": before_sha,
                "source_byte_offset": source_byte_offset,
                "source_byte_count": info.compress_size,
                "source_range_sha256": SB.sha256_bytes(compressed),
            })
    if not rows:
        raise ResolutionError(f"mod archive has no regular files: {path}")
    rows.sort(key=lambda row: _ascii_fold(row["relpath"]))
    after_sha, _ = _hash_file_stable(path)
    if before_sha != after_sha:
        raise ResolutionError(f"mod archive changed while hashing: {path}")
    if not rows:
        raise ResolutionError(f"empty mod archive: {path}")
    content_rows = [
        {"relpath": row["relpath"], "bytes": row["bytes"], "sha256": row["sha256"]}
        for row in rows
    ]
    return SB.sha256_bytes(SB.canonical_json(content_rows)), rows, before_sha, before_bytes


def recompute_source_inventory(
    vanilla: Path,
    resolved_mod_sources: list[dict[str, Any]],
    enabled_dlc_ids: list[str],
    extra_asset_dlc_ids: list[str],
    identity: dict[str, str],
) -> dict[str, Any]:
    """Re-enumerate the ordered effective source namespaces."""
    vanilla = vanilla.resolve()
    descriptor_by_id: dict[str, tuple[Path, dict[str, Any]]] = {}
    loose_descriptors = sorted(
        (vanilla / "dlc").rglob("*.dlc") if (vanilla / "dlc").is_dir() else [],
        key=lambda item: _engine_byte_key(_relative_descriptor_id(vanilla, item)),
    )
    for path in loose_descriptors:
        dlc_id = path.parent.name
        if dlc_id in descriptor_by_id:
            raise ResolutionError(f"multiple DLC descriptors share product id: {dlc_id}")
        descriptor_by_id[dlc_id] = (path, parse_descriptor(path))
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
            content_sha, inventory, source_container_sha256, source_container_bytes = _zip_namespace_rows(
                root, layer_id, layer_kind=kind
            )
        else:
            content_sha, inventory = hash_tree(
                root,
                layer_kind=kind,
                layer_id=layer_id,
                allow_empty=allow_empty,
                exclude_top=exclude_top,
            )
            source_container_sha256 = None
            source_container_bytes = None
        namespaces.append({
            "ordinal": ordinal,
            "kind": kind,
            "binding": binding,
            "id": layer_id,
            "label": label,
            "replace_paths": list(replace_paths or []),
            "storage_kind": "zip_archive" if archive else "directory",
            "source_container_sha256": source_container_sha256,
            "source_container_bytes": source_container_bytes,
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
        integrated_descriptors = sorted(
            integrated_root.rglob("*.dlc"),
            key=lambda item: _engine_byte_key(_relative_descriptor_id(vanilla, item)),
        )
        seen_integrated_ids: set[str] = set()
        for descriptor_path in integrated_descriptors:
            path = descriptor_path.parent
            if path.name in seen_integrated_ids:
                raise ResolutionError(
                    f"multiple integrated DLC descriptors share product id: {path.name}"
                )
            seen_integrated_ids.add(path.name)
            add_namespace(
                "integrated_dlc", path.name, f"integrated_dlc/{path.name}", path,
                allow_empty=True, binding="integrated"
            )
    active_products = set(enabled_dlc_ids)
    asset_only = set(extra_asset_dlc_ids)
    unknown_products = active_products - set(descriptor_by_id)
    if unknown_products:
        raise ResolutionError(
            f"active DLC product ids lack descriptors: {sorted(unknown_products)}"
        )
    if asset_only & set(descriptor_by_id):
        raise ResolutionError("descriptor-backed DLC cannot be classified asset-only")

    # Descriptorless asset namespaces are deliberately excluded by production
    # interpreters.  Keep their deterministic evidence rows ahead of mounted
    # selected content without letting their names influence the engine plan.
    for dlc_id in sorted(asset_only, key=_engine_byte_key):
        root = vanilla / "dlc" / dlc_id
        add_namespace(
            "dlc", dlc_id, f"dlc/{dlc_id}", root,
            allow_empty=True, binding="asset_only",
        )

    dlc_plan_rows = [
        {
            "id": dlc_id,
            "descriptor_id": _relative_descriptor_id(vanilla, descriptor_by_id[dlc_id][0]),
            "name": descriptor_by_id[dlc_id][1]["name"],
            "dependencies": descriptor_by_id[dlc_id][1]["dependencies"],
        }
        for dlc_id in active_products
    ]
    mod_plan_rows: list[dict[str, Any]] = []
    source_by_id: dict[str, dict[str, Any]] = {}
    for source in resolved_mod_sources:
        descriptor_id = str(source["id"]).replace("\\", "/")
        if descriptor_id in source_by_id:
            raise ResolutionError(f"resolved mod source repeats id: {descriptor_id}")
        descriptor_fields = parse_descriptor(Path(source["descriptor_path"]))
        if descriptor_fields["name"] != source["name"]:
            raise ResolutionError(f"resolved mod descriptor name changed: {descriptor_id}")
        source_by_id[descriptor_id] = source
        mod_plan_rows.append(
            {
                "id": descriptor_id,
                "descriptor_id": descriptor_id,
                "name": descriptor_fields["name"],
                "dependencies": descriptor_fields["dependencies"],
            }
        )
    mount_plan = _engine_mount_plan(dlc_plan_rows, mod_plan_rows)
    planned_mod_ids = [
        row["descriptor_id"] for row in mount_plan if row["planner_kind"] == "mod"
    ]
    ordered_recorded_sources = sorted(
        resolved_mod_sources,
        key=lambda source: source.get("ordinal", -1),
    )
    if [source.get("ordinal") for source in ordered_recorded_sources] != list(
        range(len(ordered_recorded_sources))
    ):
        raise ResolutionError("authenticated UI14 mod ordinals are not contiguous from zero")
    recorded_mod_ids = [
        str(source["id"]).replace("\\", "/")
        for source in ordered_recorded_sources
    ]
    if planned_mod_ids != recorded_mod_ids:
        raise ResolutionError(
            "authenticated UI14 mod order disagrees with the a729 dependency/stable-id planner"
        )

    for planned in mount_plan:
        if planned["planner_kind"] == "mod":
            source = source_by_id[planned["descriptor_id"]]
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
            continue

        dlc_id = planned["id"]
        root = vanilla / "dlc" / dlc_id
        descriptor = descriptor_by_id[dlc_id]
        replace_paths = descriptor[1].get("replace_path", [])
        if descriptor[1].get("archive"):
            archive_ref = Path(descriptor[1]["archive"].replace("\\", "/"))
            if archive_ref.is_absolute() or any(
                part in {"", ".", ".."} for part in archive_ref.parts
            ):
                raise ResolutionError(
                    f"unsafe descriptor-backed DLC archive path: {dlc_id}"
                )
            archive_path = (vanilla / archive_ref).resolve()
            try:
                archive_path.relative_to(vanilla)
            except ValueError as exc:
                raise ResolutionError(
                    f"descriptor-backed DLC archive escapes install root: {dlc_id}"
                ) from exc
            binding_sha, _ = hash_tree(
                root, layer_kind="dlc", layer_id=dlc_id, allow_empty=True
            )
            add_namespace(
                "dlc", dlc_id, f"dlc/{dlc_id}", archive_path, replace_paths,
                archive=True, binding="active_product",
                binding_content_sha256=binding_sha,
            )
        else:
            add_namespace(
                "dlc", dlc_id, f"dlc/{dlc_id}", root, replace_paths,
                allow_empty=True, binding="active_product",
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
            "content_sha256", "binding_content_sha256", "storage_kind",
            "source_container_sha256", "source_container_bytes"
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
        try:
            canonical_replacement = [
                _normalize_vfs_relpath(item, "source namespace replace_path")
                for item in replacement
            ]
        except ResolutionError as exc:
            raise ResolutionError(
                f"namespace {ordinal}.replace_paths is malformed"
            ) from exc
        if canonical_replacement != replacement or len(canonical_replacement) != len(
            {_ascii_fold(item) for item in canonical_replacement}
        ):
            raise ResolutionError(
                f"namespace {ordinal}.replace_paths is non-canonical or repeated"
            )
        file_count = namespace.get("file_count")
        if not isinstance(file_count, int) or isinstance(file_count, bool) or file_count < 0:
            raise ResolutionError(f"namespace {ordinal}.file_count is invalid")
        storage_kind = namespace.get("storage_kind")
        source_container_sha256 = namespace.get("source_container_sha256")
        source_container_bytes = namespace.get("source_container_bytes")
        if storage_kind == "directory":
            if source_container_sha256 is not None or source_container_bytes is not None:
                raise ResolutionError(f"namespace {ordinal} directory carries container identity")
        elif storage_kind == "zip_archive":
            source_container_sha256 = SB._required_sha(
                source_container_sha256,
                f"namespace {ordinal}.source_container_sha256",
            )
            if (
                not isinstance(source_container_bytes, int)
                or isinstance(source_container_bytes, bool)
                or source_container_bytes <= 0
            ):
                raise ResolutionError(f"namespace {ordinal}.source_container_bytes is invalid")
        else:
            raise ResolutionError(f"namespace {ordinal}.storage_kind is invalid")
        normalized_namespaces.append({
            "ordinal": ordinal,
            "kind": namespace["kind"],
            "binding": namespace["binding"],
            "id": SB._required_string(namespace.get("id"), f"namespace {ordinal}.id"),
            "label": label,
            "replace_paths": canonical_replacement,
            "storage_kind": storage_kind,
            "source_container_sha256": source_container_sha256,
            "source_container_bytes": source_container_bytes,
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
            "namespace_ordinal", "layer_kind", "layer_id", "relpath", "bytes", "sha256",
            "storage_kind", "source_container_sha256", "source_byte_offset",
            "source_byte_count", "source_range_sha256"
        }:
            raise ResolutionError(f"source inventory row {index} is malformed")
        ordinal = row.get("namespace_ordinal")
        if not isinstance(ordinal, int) or isinstance(ordinal, bool) or not 0 <= ordinal < len(namespaces):
            raise ResolutionError(f"source row {index}.namespace_ordinal is invalid")
        relpath = SB._required_string(row.get("relpath"), f"source row {index}.relpath")
        try:
            canonical_relpath = _normalize_archive_member(relpath)
        except ResolutionError:
            canonical_relpath = ""
        if canonical_relpath != relpath:
            raise ResolutionError(f"source row {index}.relpath is unsafe")
        key = (ordinal, _ascii_fold(relpath))
        if key in seen_rows:
            raise ResolutionError(f"duplicate/case-colliding source row: {key}")
        seen_rows.add(key)
        namespace = normalized_namespaces[ordinal]
        if row.get("layer_kind") != namespace["kind"] or row.get("layer_id") != namespace["id"]:
            raise ResolutionError(f"source row {index} namespace identity mismatch")
        byte_count = row.get("bytes")
        if not isinstance(byte_count, int) or isinstance(byte_count, bool) or byte_count < 0:
            raise ResolutionError(f"source row {index}.bytes is invalid")
        payload_sha256 = SB._required_sha(row.get("sha256"), f"source row {index}.sha256")
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
            raise ResolutionError(f"source row {index} physical range is invalid")
        source_range_sha256 = SB._required_sha(
            row.get("source_range_sha256"), f"source row {index}.source_range_sha256"
        )
        if row.get("storage_kind") != namespace["storage_kind"]:
            raise ResolutionError(f"source row {index} storage kind mismatch")
        if namespace["storage_kind"] == "directory":
            if (
                row.get("source_container_sha256") is not None
                or source_byte_offset != 0
                or source_byte_count != byte_count
                or source_range_sha256 != payload_sha256
            ):
                raise ResolutionError(f"source row {index} loose-file range is invalid")
        else:
            if (
                row.get("source_container_sha256") != namespace["source_container_sha256"]
                or source_byte_offset + source_byte_count > namespace["source_container_bytes"]
            ):
                raise ResolutionError(f"source row {index} archive range is invalid")
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
    state["enabled_mods"] = [
        _normalize_vfs_relpath(token, "launcher mod descriptor")
        for token in state["enabled_mods"]
    ]
    if len(state["enabled_mods"]) != len(
        {_ascii_fold(token) for token in state["enabled_mods"]}
    ):
        raise ResolutionError("dlc_load.json contains duplicate enabled mods")
    return state, before


def _disabled_dlc_layer_ids(
    disabled_tokens: list[str],
    installed_descriptor_ids: set[str],
) -> list[str]:
    """Normalize launcher-disabled descriptor tokens to installed layer IDs."""
    disabled_ids: set[str] = set()
    for token in disabled_tokens:
        normalized = token.replace("\\", "/")
        parts = PurePosixPath(normalized).parts
        if (
            len(parts) != 3
            or _ascii_fold(parts[0]) != "dlc"
            or not parts[1]
            or not _ascii_fold(parts[2]).endswith(".dlc")
        ):
            raise ResolutionError(f"invalid disabled DLC descriptor token: {token!r}")
        if parts[1] not in installed_descriptor_ids:
            raise ResolutionError(
                f"disabled DLC descriptor token has no installed layer: {token!r}"
            )
        disabled_ids.add(parts[1])
    return sorted(disabled_ids, key=_engine_byte_key)


def _runtime_and_requested_dlc_selection(
    runtime_active_names: list[str],
    descriptor_ids_by_name: dict[str, str],
    disabled_tokens: list[str],
) -> tuple[list[str], list[str], list[str], list[str]]:
    """Return effective engine names/IDs, requested IDs, and their conflicts.

    The a729 engine-owned Active DLC producer enumerates the manager's active
    tree directly.  Launcher-disabled membership is therefore retained only as
    requested-profile evidence and never intersects the effective roster.
    """
    missing = sorted(set(runtime_active_names) - set(descriptor_ids_by_name))
    if missing:
        raise ResolutionError(
            f"runtime active DLC names lack installed descriptors: {missing}"
        )
    if len(runtime_active_names) != len(set(runtime_active_names)):
        raise ResolutionError("runtime active DLC names contain duplicates")
    installed_ids = set(descriptor_ids_by_name.values())
    if len(installed_ids) != len(descriptor_ids_by_name):
        raise ResolutionError("installed DLC descriptors repeat a layer ID")
    disabled_ids = _disabled_dlc_layer_ids(disabled_tokens, installed_ids)
    effective_ids = [descriptor_ids_by_name[name] for name in runtime_active_names]
    requested_ids = sorted(installed_ids - set(disabled_ids), key=_engine_byte_key)
    conflicts = sorted(set(effective_ids) & set(disabled_ids), key=_engine_byte_key)
    return runtime_active_names, effective_ids, requested_ids, conflicts


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


def validate_ui14_mod_collision_winners(
    mod_inventory_by_id: dict[str, list[dict[str, Any]]],
    effective_winners: list[dict[str, Any]],
) -> None:
    """Join engine winner receipts to the complete loose/archive namespace."""
    collision_candidates: dict[str, list[tuple[str, dict[str, Any]]]] = {}
    for layer_id, inventory in mod_inventory_by_id.items():
        for row in inventory:
            collision_candidates.setdefault(
                _ascii_fold(row["relpath"].replace("\\", "/")), []
            ).append((layer_id, row))
    collisions = {
        key: rows for key, rows in collision_candidates.items() if len(rows) > 1
    }
    winner_rows: dict[str, dict[str, Any]] = {}
    for index, row in enumerate(effective_winners):
        if not isinstance(row, dict) or set(row) != {"relpath", "winner_id", "sha256"}:
            raise ResolutionError(f"UI14 engine VFS winner row {index} is malformed")
        relpath = row.get("relpath")
        winner_id = row.get("winner_id")
        payload_sha = row.get("sha256")
        if not isinstance(relpath, str):
            raise ResolutionError(f"UI14 engine VFS winner row {index} path is invalid")
        try:
            normalized = _normalize_vfs_relpath(relpath, "UI14 winner path")
        except ResolutionError as exc:
            raise ResolutionError(
                f"UI14 engine VFS winner row {index} identity is invalid"
            ) from exc
        key = _ascii_fold(normalized)
        if (
            winner_id not in mod_inventory_by_id
            or not isinstance(payload_sha, str)
            or re.fullmatch(r"[0-9a-f]{64}", payload_sha) is None
            or key in winner_rows
        ):
            raise ResolutionError(f"UI14 engine VFS winner row {index} identity is invalid")
        winner_rows[key] = {**row, "relpath": normalized}
    if set(winner_rows) != set(collisions):
        raise ResolutionError(
            "UI14 engine VFS winner rows do not exactly cover mod collisions"
        )
    for key, winner in winner_rows.items():
        matches = [
            row
            for layer_id, row in collisions[key]
            if layer_id == winner["winner_id"]
        ]
        if len(matches) != 1 or matches[0]["sha256"] != winner["sha256"]:
            raise ResolutionError(
                f"UI14 engine VFS effective winner mismatch: {winner['relpath']}"
            )


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
    ui14_engine_vfs: Path,
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
    ui14_receipt = _load_ui14_engine_vfs(
        ui14_engine_vfs, exe_sha, userdir, launcher["enabled_mods"]
    )
    engine_layers = sorted(
        ui14_receipt["layers"],
        key=lambda row: (row["engine_mount_ordinal"], row["engine_descriptor_ordinal"]),
    )

    vanilla = exe.parent
    dlc_root = vanilla / "dlc"
    descriptors = sorted(
        dlc_root.rglob("*.dlc"),
        key=lambda path: _engine_byte_key(_relative_descriptor_id(vanilla, path)),
    )
    if not descriptors:
        raise ResolutionError("no installed DLC descriptors found")
    descriptor_by_name: dict[str, tuple[Path, dict[str, Any]]] = {}
    for descriptor in descriptors:
        fields = parse_descriptor(descriptor)
        if fields["name"] in descriptor_by_name:
            raise ResolutionError(f"duplicate installed DLC name: {fields['name']}")
        descriptor_by_name[fields["name"]] = (descriptor, fields)
    descriptor_ids_by_name = {
        name: descriptor.parent.name
        for name, (descriptor, _fields) in descriptor_by_name.items()
    }
    (
        selected_dlc_names,
        selected_dlc_ids,
        requested_dlc_ids,
        launched_disabled_conflicts,
    ) = (
        _runtime_and_requested_dlc_selection(
            runtime["active_dlc"],
            descriptor_ids_by_name,
            launcher["disabled_dlcs"],
        )
    )

    mod_rows: list[dict[str, Any]] = []
    mod_inventory: list[dict[str, Any]] = []
    mod_names: list[str] = []
    resolved_mod_sources: list[dict[str, Any]] = []
    mod_inventory_by_id: dict[str, list[dict[str, Any]]] = {}
    for ordinal, layer_receipt in enumerate(engine_layers):
        token = layer_receipt["id"]
        descriptor = (userdir / token).resolve()
        fields = parse_descriptor(descriptor)
        content_ref = fields.get("path") or fields["archive"]
        content_root = (userdir / content_ref).resolve() if not Path(content_ref).is_absolute() else Path(content_ref).resolve()
        if content_root.is_file():
            content_sha, content_bytes = _hash_file_stable(content_root)
            (
                _namespace_sha,
                inventory,
                archive_sha,
                archive_bytes,
            ) = _zip_namespace_rows(content_root, token, layer_kind="mod")
            if archive_sha != content_sha or archive_bytes != content_bytes:
                raise ResolutionError(f"mod archive changed while resolving: {token}")
        else:
            content_sha, inventory = hash_tree(content_root, layer_kind="mod", layer_id=token)
        mod_inventory.extend(inventory)
        mod_names.append(fields["name"])
        descriptor_sha, _ = _hash_file_stable(descriptor)
        normalized_id = token.replace("\\", "/")
        mod_inventory_by_id[normalized_id] = inventory
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
    if sorted(mod_names) != sorted(runtime["active_mods"]):
        raise ResolutionError(
            f"engine-receipt mod names {mod_names} != runtime active mods {runtime['active_mods']}"
        )

    validate_ui14_mod_collision_winners(
        mod_inventory_by_id, ui14_receipt["effective_winners"]
    )

    source_rows: list[dict[str, Any]] = []
    vanilla_sha, vanilla_inventory = hash_tree(
        vanilla, layer_kind="vanilla", layer_id="vanilla", exclude_top={"dlc"}
    )
    descriptor_parent_ids = {descriptor.parent.name for descriptor in descriptors}
    excluded_descriptorless_asset_layers, _descriptorless_inventory = (
        classify_descriptorless_asset_layers(vanilla, descriptor_parent_ids)
    )
    source_rows.extend(vanilla_inventory)
    source_rows.extend(mod_inventory)
    dlc_rows: list[dict[str, Any]] = []
    enabled_dlc_ids: list[str] = []
    dlc_content: dict[str, str] = {}
    for name, (descriptor, fields) in sorted(descriptor_by_name.items()):
        dlc_id = descriptor.parent.name
        content_sha, inventory = hash_tree(descriptor.parent, layer_kind="dlc", layer_id=dlc_id)
        source_rows.extend(inventory)
        dlc_content[dlc_id] = content_sha
        enabled = dlc_id in selected_dlc_ids
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
        "active_dlcs": selected_dlc_names,
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
    mod_rows.sort(key=lambda row: _engine_byte_key(row["id"]))
    dlc_rows.sort(key=lambda row: _engine_byte_key(row["id"]))
    descriptor_by_product = {
        descriptor.parent.name: (descriptor, fields)
        for descriptor, fields in descriptor_by_name.values()
    }
    planned = _engine_mount_plan(
        [
            {
                "id": dlc_id,
                "descriptor_id": _relative_descriptor_id(
                    vanilla, descriptor_by_product[dlc_id][0]
                ),
                "name": descriptor_by_product[dlc_id][1]["name"],
                "dependencies": descriptor_by_product[dlc_id][1]["dependencies"],
            }
            for dlc_id in enabled_dlc_ids
        ],
        [
            {
                "id": source["id"],
                "descriptor_id": source["id"],
                "name": source["name"],
                "dependencies": parse_descriptor(
                    Path(source["descriptor_path"])
                )["dependencies"],
            }
            for source in resolved_mod_sources
        ],
    )
    planned_mod_ids = [
        row["descriptor_id"] for row in planned if row["planner_kind"] == "mod"
    ]
    authenticated_mod_ids = [row["id"] for row in engine_layers]
    if planned_mod_ids != authenticated_mod_ids:
        raise ResolutionError(
            "UI14 engine mount order disagrees with the a729 dependency/stable-id planner"
        )
    dlc_by_id = {row["id"]: row for row in dlc_rows}
    mod_by_id = {row["id"]: row for row in mod_rows}
    load_rows = [{"id": "vanilla", "kind": "vanilla", "ordinal": 0, "content_sha256": vanilla_sha}]
    for ordinal, planned_row in enumerate(planned, start=1):
        row = (
            dlc_by_id[planned_row["id"]]
            if planned_row["planner_kind"] == "dlc"
            else mod_by_id[planned_row["id"]]
        )
        load_rows.append(
            {
                "id": row["id"],
                "kind": planned_row["planner_kind"],
                "ordinal": ordinal,
                "content_sha256": row["content_sha256"],
            }
        )
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
        [],
        identity,
    )
    validate_source_inventory_document(source_inventory)
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
        "requested_profile_dlc_ids": requested_dlc_ids,
        "effective_launch_dlc_ids": enabled_dlc_ids,
        "descriptorless_asset_layers": [],
        "excluded_descriptorless_asset_layers": excluded_descriptorless_asset_layers,
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
    (out_dir / "ui14_engine_vfs.json").write_bytes(SB.pretty_json(ui14_receipt))
    return {
        "documents": documents,
        "report": report,
        "active_mod_names": mod_names,
        "active_dlc_ids": enabled_dlc_ids,
        "paths": {
            **{role: out_dir / f"{role}.json" for role in documents},
            "source_inventory": out_dir / "source_inventory.json",
            "resolution_report": out_dir / "resolution_report.json",
            "ui14_engine_vfs": out_dir / "ui14_engine_vfs.json",
        },
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True)
    parser.add_argument("--userdir", type=Path, required=True)
    parser.add_argument("--pid", type=int, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--ui14-engine-vfs", type=Path, required=True)
    parser.add_argument("--bundle-out", type=Path)
    parser.add_argument("--defer-provenance", type=Path)
    args = parser.parse_args(argv)
    try:
        resolved = resolve_live_bindings(
            exe=args.exe, userdir=args.userdir, pid=args.pid, out_dir=args.out,
            ui14_engine_vfs=args.ui14_engine_vfs,
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
