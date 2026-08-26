#!/usr/bin/env python3
"""Build and verify an interface sprite/icon asset manifest.

The manifest maps ``interface/*.gfx`` sprite names to texture paths and hashes.
It also inventories country flag image files by path because flags are normally
path-convention assets rather than ``SpriteType`` entries.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any

_HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(_HERE))

import resolver as R
import live_binding_resolver as LR
from paradox_parser import Node, parse
from static_artifact_common import (
    CLAUSEWITZ_SYNTAX_QUIRK_ALLOWLIST,
    add_layer_args,
    canonical_json,
    mods_from_arg,
    node_to_data,
    pretty_json,
    repo_relative,
    scalar,
    sha256_bytes,
    source_inventory,
    validate_clausewitz_text,
    verify_store,
    write_static_store,
)

GAME_ID = "gfx_icon_asset_manifest_static"
DEFAULT_OUT = _HERE / "out" / "gfx_icon_manifest"
DEFAULT_MISSING_TEXTURE_ALLOWLIST = _HERE / "gfx_icon_missing_texture_allowlist.json"
TEXTURE_EXTS = (".dds", ".tga", ".png")
EXTENSION_FALLBACK_EXTS = (".dds", ".tga")
MISSING_TEXTURE_REASON = (
    "absent in vanilla root, integrated_dlc/*, enabled dlc/*, and as extension "
    "variant; engine renders placeholder at runtime"
)
MissingTextureKey = tuple[str, str, str, str]


def _new_layer(label: str, root: Path, replace_paths=None, meta=None):
    return R._ex.Layer(label, Path(root).resolve(), replace_paths, meta)


def _clone_layer(layer, *, meta_extra: dict[str, Any] | None = None):
    meta = dict(getattr(layer, "meta", {}) or {})
    if meta_extra:
        meta.update(meta_extra)
    return _new_layer(layer.label, Path(layer.root), getattr(layer, "replace_paths", []), meta)


def _installed_dlc_dirs(vanilla: Path, bucket: str) -> list[Path]:
    root = vanilla / bucket
    if not root.is_dir():
        return []
    return sorted((p.resolve() for p in root.iterdir() if p.is_dir()), key=lambda p: p.name.casefold())


def _staged_dlc_roots(userdata: Path) -> dict[str, dict[str, Any]]:
    manifest_path = userdata / "static_mod_staging_manifest.json"
    if not manifest_path.is_file():
        return {}
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        raise RuntimeError(f"failed to read runtime DLC staging manifest: {exc}") from exc
    if (
        manifest.get("schema") != "extractor.runtime_static_mod_staging.v2"
        or not isinstance(manifest.get("dlc_archives"), list)
    ):
        raise RuntimeError("runtime DLC staging manifest has an unsupported schema")
    staging_root = (userdata / "_materialized_dlc").resolve()
    rows: dict[str, dict[str, Any]] = {}
    for row in manifest["dlc_archives"]:
        if not isinstance(row, dict) or not isinstance(row.get("dlc_id"), str):
            raise RuntimeError("runtime DLC staging manifest contains a malformed row")
        dlc_id = row["dlc_id"]
        if Path(dlc_id).name != dlc_id or "/" in dlc_id or "\\" in dlc_id:
            raise RuntimeError(f"unsafe staged DLC id: {dlc_id!r}")
        root = Path(row.get("content_path", "")).resolve()
        try:
            root.relative_to(staging_root)
        except ValueError as exc:
            raise RuntimeError(f"staged DLC root escapes runtime staging: {dlc_id}") from exc
        if not root.is_dir():
            raise RuntimeError(f"staged DLC root is missing: {root}")
        if dlc_id in rows:
            raise RuntimeError(f"runtime DLC staging repeats id: {dlc_id}")
        rows[dlc_id] = {**row, "root": root}
    return rows


def _inventory_dlc_root(
    vanilla: Path,
    namespace: dict[str, Any],
    staged_dlc: dict[str, dict[str, Any]],
) -> Path:
    layer_id = namespace["id"]
    if Path(layer_id).name != layer_id or "/" in layer_id or "\\" in layer_id:
        raise RuntimeError(f"unsafe source inventory namespace id: {layer_id!r}")
    staged = staged_dlc.get(layer_id)
    if staged is not None:
        if staged.get("source_namespace_sha256") != namespace.get("content_sha256"):
            raise RuntimeError(f"staged DLC content hash disagrees with live inventory: {layer_id}")
        return staged["root"]
    parent = "integrated_dlc" if namespace["kind"] == "integrated_dlc" else "dlc"
    parent_root = (vanilla / parent).resolve()
    root = (parent_root / layer_id).resolve()
    try:
        root.relative_to(parent_root)
    except ValueError as exc:
        raise RuntimeError(f"source inventory namespace escapes {parent}: {layer_id}") from exc
    if not root.is_dir():
        raise RuntimeError(f"source inventory namespace root is missing: {parent}/{layer_id}")
    return root


def _dlc_name_from_token(token: str) -> str | None:
    norm = str(token).replace("\\", "/").strip().strip('"')
    if not norm:
        return None
    parts = [part for part in norm.split("/") if part]
    for idx, part in enumerate(parts):
        if part.lower() == "dlc" and idx + 1 < len(parts):
            return parts[idx + 1]
    if len(parts) >= 2 and parts[-1].lower().endswith(".dlc"):
        return parts[-2]
    return Path(norm).stem or None


def _load_dlc_selection(userdata: Path, installed_dlc: list[Path]) -> dict[str, Any]:
    dlc_load = userdata / "dlc_load.json"
    installed_names = [path.name for path in installed_dlc]
    # "No dlc_load.json means every installed DLC is enabled" is the engine's
    # rule, and only an OBSERVATION when this really is the user-data dir.
    if not R.looks_like_userdata(userdata):
        raise RuntimeError(
            f"{userdata} is not a the game user-data dir (no settings.txt / "
            f"launcher-v2.sqlite / dlc_load.json), so the absence of dlc_load.json there is not "
            f"evidence that every installed DLC is enabled. Pass --userdata or set BATTLE_PLAN_USERDATA."
        )
    if not dlc_load.is_file():
        return {
            "dlc_load_json_present": False,
            "dlc_load_json_name": "dlc_load.json",
            "missing_dlc_load_means_all_installed_enabled": True,
            "installed_dlcs": installed_names,
            "enabled_dlcs": installed_names,
            "disabled_dlcs": [],
            "disabled_dlc_paths": [],
        }

    data = json.loads(dlc_load.read_text(encoding="utf-8"))
    raw_disabled = data.get("disabled_dlcs", [])
    if not isinstance(raw_disabled, list):
        raise RuntimeError(f"dlc_load.json disabled_dlcs is not a list: {dlc_load}")
    disabled_paths = [str(item).replace("\\", "/") for item in raw_disabled if isinstance(item, str)]
    disabled_names = sorted(
        {name for token in disabled_paths for name in [_dlc_name_from_token(token)] if name},
        key=str.casefold,
    )
    disabled_set = {name.casefold() for name in disabled_names}
    enabled_names = [name for name in installed_names if name.casefold() not in disabled_set]
    return {
        "dlc_load_json_present": True,
        "dlc_load_json_name": "dlc_load.json",
        "missing_dlc_load_means_all_installed_enabled": False,
        "installed_dlcs": installed_names,
        "enabled_dlcs": enabled_names,
        "disabled_dlcs": disabled_names,
        "disabled_dlc_paths": sorted(disabled_paths, key=str.casefold),
    }


def build_gfx_asset_layers(
    vanilla: str | None = None,
    mods: list[str] | None = None,
    userdata: str | Path | None = None,
    strict_mods: bool = False,
    source_inventory_path: Path | None = None,
) -> tuple[list[Any], dict[str, Any]]:
    """Build the GFX-only effective asset layer stack.

    Definition parsers keep using ``resolver.build_layers``. This opt-in model
    adds the game asset DLC roots between vanilla and mods because interface sprites,
    textures, and flags are shipped there even though common/localisation data is
    not.
    """

    vanilla_root = R.vanilla_root(vanilla)
    # `userdata_root()`, not `DEFAULT_USERDATA`: this gate reads launcher state
    # and must refuse a directory it cannot identify.
    userdata_root = R.userdata_root(userdata)
    mod_layers = R.build_layers(
        vanilla=str(vanilla_root), mods=mods, userdata=userdata_root, strict_mods=strict_mods
    )
    inventory = None
    if source_inventory_path is not None:
        inventory = LR.validate_source_inventory_document(
            json.loads(Path(source_inventory_path).read_text(encoding="utf-8"))
        )
        active_mod_labels = [
            row["label"]
            for row in inventory["namespaces"]
            if row["binding"] == "active_mod"
        ]
        if len(active_mod_labels) != len(set(active_mod_labels)):
            raise RuntimeError("live source inventory repeats an active-mod label")
        by_label = {layer.label: layer for layer in mod_layers[1:]}
        if len(by_label) != len(mod_layers) - 1:
            raise RuntimeError("resolver returned duplicate non-base layer labels")
        try:
            selected_mod_layers = [by_label[label] for label in active_mod_labels]
        except KeyError as exc:
            raise RuntimeError(
                "resolver active-mod layers do not match live source inventory"
            ) from exc
        for namespace, layer in zip(
            (row for row in inventory["namespaces"] if row["binding"] == "active_mod"),
            selected_mod_layers,
            strict=True,
        ):
            if list(getattr(layer, "replace_paths", []) or []) != namespace["replace_paths"]:
                raise RuntimeError("resolver active-mod replace_path differs from live inventory")
    else:
        selected_mod_layers = mod_layers[1:]

    integrated_dirs = _installed_dlc_dirs(vanilla_root, "integrated_dlc")
    installed_dlc_dirs = _installed_dlc_dirs(vanilla_root, "dlc")
    dlc_selection = _load_dlc_selection(userdata_root, installed_dlc_dirs)
    enabled_set = {name.casefold() for name in dlc_selection["enabled_dlcs"]}
    enabled_dlc_dirs = [path for path in installed_dlc_dirs if path.name.casefold() in enabled_set]

    inventory_enabled_dlcs = (
        [
            row["id"]
            for row in inventory["namespaces"]
            if row["kind"] == "dlc" and row["binding"] == "active_product"
        ]
        if inventory is not None
        else dlc_selection["enabled_dlcs"]
    )
    inventory_integrated_dlcs = (
        [
            row["id"]
            for row in inventory["namespaces"]
            if row["kind"] == "integrated_dlc" and row["binding"] == "integrated"
        ]
        if inventory is not None
        else [path.name for path in integrated_dirs]
    )
    excluded_asset_only_dlcs = (
        [
            row["id"]
            for row in inventory["namespaces"]
            if row["kind"] == "dlc" and row["binding"] == "asset_only"
        ]
        if inventory is not None
        else []
    )

    selection_meta = {
        "dlc_load_json_present": dlc_selection["dlc_load_json_present"],
        "missing_dlc_load_means_all_installed_enabled": dlc_selection[
            "missing_dlc_load_means_all_installed_enabled"
        ],
        "enabled_dlcs": inventory_enabled_dlcs,
        "disabled_dlcs": dlc_selection["disabled_dlcs"],
    }
    if inventory is None:
        layers = [
            _clone_layer(
                mod_layers[0],
                meta_extra={
                    "kind": "vanilla",
                    "asset_layer_model": "vanilla, integrated_dlc/*, enabled dlc/*, mods",
                    **selection_meta,
                },
            )
        ]
        for root in integrated_dirs:
            layers.append(
                _new_layer(
                    f"integrated_dlc/{root.name}",
                    root,
                    meta={
                        "kind": "integrated_dlc",
                        "dlc_name": root.name,
                        "enabled": True,
                        **selection_meta,
                    },
                )
            )
        for root in enabled_dlc_dirs:
            layers.append(
                _new_layer(
                    f"dlc/{root.name}",
                    root,
                    meta={
                        "kind": "dlc",
                        "dlc_name": root.name,
                        "enabled": True,
                        **selection_meta,
                    },
                )
            )
        for layer in selected_mod_layers:
            layers.append(_clone_layer(layer, meta_extra={"kind": "mod"}))
    else:
        staged_dlc = _staged_dlc_roots(userdata_root)
        mods_by_label = {layer.label: layer for layer in selected_mod_layers}
        layers = []
        for namespace in inventory["namespaces"]:
            binding = namespace["binding"]
            if binding == "base":
                if layers or namespace["label"] != mod_layers[0].label:
                    raise RuntimeError("live source inventory base does not match resolver base")
                layer = _clone_layer(
                    mod_layers[0],
                    meta_extra={
                        "kind": "vanilla",
                        "binding": binding,
                        "asset_layer_model": "exact live source_inventory namespace order",
                        **selection_meta,
                    },
                )
            elif binding == "asset_only":
                continue
            elif binding in {"integrated", "active_product"}:
                layer = _new_layer(
                    namespace["label"],
                    _inventory_dlc_root(vanilla_root, namespace, staged_dlc),
                    replace_paths=namespace["replace_paths"],
                    meta={
                        "kind": namespace["kind"],
                        "binding": binding,
                        "dlc_name": namespace["id"],
                        "enabled": True,
                        **selection_meta,
                    },
                )
            elif binding == "active_mod":
                layer = _clone_layer(
                    mods_by_label[namespace["label"]],
                    meta_extra={"kind": "mod", "binding": binding},
                )
            else:
                raise RuntimeError(f"unsupported source namespace binding: {binding!r}")
            layers.append(layer)

    meta = {
        "model": (
            "exact live source_inventory namespace order"
            if inventory is not None
            else "vanilla root -> integrated_dlc/* -> enabled dlc/* -> mods"
        ),
        "source_inventory_sha256": (
            inventory["inventory_sha256"] if inventory is not None else None
        ),
        "integrated_dlcs": inventory_integrated_dlcs,
        "installed_dlcs": dlc_selection["installed_dlcs"],
        "enabled_dlcs": inventory_enabled_dlcs,
        "excluded_asset_only_dlcs": excluded_asset_only_dlcs,
        "disabled_dlcs": dlc_selection["disabled_dlcs"],
        "disabled_dlc_paths": dlc_selection["disabled_dlc_paths"],
        "dlc_load_json_present": dlc_selection["dlc_load_json_present"],
        "dlc_load_json_name": dlc_selection["dlc_load_json_name"],
        "missing_dlc_load_means_all_installed_enabled": dlc_selection[
            "missing_dlc_load_means_all_installed_enabled"
        ],
    }
    return layers, meta


def _source_label(layers, abspath) -> str:
    return layers[R._layer_prio_fn(layers)(abspath)].label


def _source_layer_kind(layers, abspath) -> str:
    layer = layers[R._layer_prio_fn(layers)(abspath)]
    meta = getattr(layer, "meta", {}) or {}
    return meta.get("kind") or ("vanilla" if layer.label == "vanilla" else "mod")


def _gfx_layer_summary(layers) -> list[dict[str, Any]]:
    allowed_meta = {
        "kind",
        "asset_layer_model",
        "dlc_name",
        "enabled",
        "dlc_load_json_present",
        "missing_dlc_load_means_all_installed_enabled",
        "enabled_dlcs",
        "disabled_dlcs",
    }
    out = []
    for layer in layers:
        meta = getattr(layer, "meta", {}) or {}
        row = {
            "label": layer.label,
            "root_name": Path(layer.root).name,
            "replace_path": list(getattr(layer, "replace_paths", []) or []),
        }
        filtered = {key: meta[key] for key in sorted(allowed_meta) if key in meta}
        if filtered:
            row["meta"] = filtered
        out.append(row)
    return out


def _direct_interface_files(layers) -> list[tuple[str, Path]]:
    return [
        (rel, path)
        for rel, path in R.ordered_files(layers, "interface", (".gfx",))
        if Path(rel).parent.as_posix() == "interface"
    ]


def _direct_gui_files(layers) -> list[tuple[str, Path]]:
    return [
        (rel, path)
        for rel, path in R.ordered_files(layers, "interface", (".gui",))
        if Path(rel).parent.as_posix() == "interface"
    ]


_POWER_BALANCE_GUI_ROLES = (
    ("panel_background", "interface/powerbalanceview.gui", ("powerbalanceview", "background"), "background", "authored"),
    ("panel_artwork", "interface/powerbalanceview.gui", ("powerbalanceview", "bop_bg"), "iconType", "authored"),
    ("decision_container_frame", "interface/powerbalanceview.gui", ("powerbalanceview", "decision_container", "Background"), "background", "authored"),
    ("header_background", "interface/powerbalanceview.gui", ("powerbalanceview", "power_balance_header_bg"), "iconType", "authored"),
    ("close_button", "interface/powerbalanceview.gui", ("powerbalanceview", "close"), "buttonType", "authored"),
    ("decision_background", "interface/powerbalanceview.gui", ("powerbalanceview", "Background"), "background", "authored"),
    ("balance_frame", "interface/powerbalanceview.gui", ("powerbalanceview", "power_balance_frame"), "iconType", "authored"),
    ("balance_value", "interface/powerbalanceview.gui", ("powerbalanceview", "power_balance_value"), "iconType", "engine"),
    ("position_marker", "interface/powerbalanceview.gui", ("powerbalanceview", "position_marker"), "iconType", "authored"),
    ("range_indicator", "interface/powerbalanceview.gui", ("range_indicator",), "iconType", "authored"),
    ("range_splitter", "interface/powerbalanceview.gui", ("range_bar",), "iconType", "authored"),
    ("left_side_icon", "interface/powerbalanceview.gui", ("powerbalanceview", "left_power_icon"), "iconType", "engine"),
    ("right_side_icon", "interface/powerbalanceview.gui", ("powerbalanceview", "right_power_icon"), "iconType", "engine"),
    ("launcher_button", "interface/countrypoliticsview.gui", ("power_balance_button", "show_power_balance"), "buttonType", "authored"),
    ("launcher_glow", "interface/countrypoliticsview.gui", ("power_balance_button", "show_power_balance_glow"), "iconType", "authored_animation"),
    ("launcher_icon", "interface/countrypoliticsview.gui", ("power_balance_button", "power_balance_ICON"), "iconType", "engine"),
)


def _named_gui_elements(node: Node, ancestors: tuple[str, ...] = ()):
    sibling_type_ordinals: dict[str, int] = {}
    for element_type, value in node.entries:
        if not isinstance(value, Node):
            continue
        name = scalar(value.get("name"))
        next_ancestors = ancestors
        if name:
            folded_type = element_type.casefold()
            type_ordinal = sibling_type_ordinals.get(folded_type, 0)
            sibling_type_ordinals[folded_type] = type_ordinal + 1
            path = (*ancestors, name)
            yield path, element_type, value, ancestors, type_ordinal
            next_ancestors = path
        yield from _named_gui_elements(value, next_ancestors)


def _gui_sprite_fields(node: Node) -> list[tuple[str, str]]:
    rows = []
    for key, value in node.entries:
        if isinstance(value, Node) or not isinstance(value, str):
            continue
        folded = key.casefold()
        if folded in {"spritetype", "quadtexturesprite"} or folded.endswith("spritetype"):
            if value.strip():
                rows.append((key, value))
    return rows


def _power_balance_gui_manifest(layers) -> dict[str, Any]:
    effective = {rel.casefold(): (rel, path) for rel, path in _direct_gui_files(layers)}
    parsed: dict[
        str,
        tuple[
            str,
            Path,
            list[tuple[tuple[str, ...], str, Node, tuple[str, ...], int]],
        ],
    ] = {}
    for _, rel, _, _, _ in _POWER_BALANCE_GUI_ROLES:
        folded = rel.casefold()
        if folded in parsed:
            continue
        selected = effective.get(folded)
        if selected is None:
            raise RuntimeError(f"effective GUI manifest is missing {rel}")
        actual_rel, path = selected
        try:
            text = R.read_text(path)
            validate_clausewitz_text(text, actual_rel)
            root = parse(text)
        except Exception as exc:
            raise RuntimeError(f"failed to parse required GUI file {actual_rel}: {exc}") from exc
        parsed[folded] = (actual_rel, path, list(_named_gui_elements(root)))

    sources = {}
    roles = {}
    for role, rel, suffix, widget_type, binding_source in _POWER_BALANCE_GUI_ROLES:
        actual_rel, path, elements = parsed[rel.casefold()]
        matches = [
            row for row in elements
            if len(row[0]) >= len(suffix)
            and row[0][-len(suffix):] == suffix
            and row[1].casefold() == widget_type.casefold()
        ]
        if len(matches) != 1:
            raise RuntimeError(
                f"effective GUI role {role} has {len(matches)} matches for "
                f"{actual_rel}#{'/'.join(suffix)}"
            )
        hierarchy, element_type, node, parent_hierarchy, type_ordinal = matches[0]
        sprite_fields = _gui_sprite_fields(node)
        if binding_source in {"authored", "authored_animation"} and len(sprite_fields) != 1:
            raise RuntimeError(
                f"effective GUI role {role} requires exactly one authored sprite, "
                f"found {len(sprite_fields)}"
            )
        if binding_source == "engine" and role != "launcher_icon" and sprite_fields:
            raise RuntimeError(f"engine-selected GUI role {role} unexpectedly authors a sprite")
        row = {
            "source_file": actual_rel,
            "source_layer": _source_label(layers, path),
            "source_layer_kind": _source_layer_kind(layers, path),
            "hierarchy_path": "/".join(hierarchy),
            "parent_hierarchy_path": "/".join(parent_hierarchy),
            "widget_type": element_type,
            "widget_type_ordinal": type_ordinal,
            "widget_name": hierarchy[-1],
            "hierarchy_match": "exact_case_sensitive",
            "binding_source": binding_source,
            "authored_sprite_fields": [
                {"field": field, "sprite_name": sprite}
                for field, sprite in sprite_fields
            ],
        }
        if binding_source in {"authored", "authored_animation"}:
            row["sprite_name"] = sprite_fields[0][1]
        roles[role] = row
        if actual_rel not in sources:
            payload = path.read_bytes()
            sources[actual_rel] = {
                "source_layer": _source_label(layers, path),
                "source_layer_kind": _source_layer_kind(layers, path),
                "bytes": len(payload),
                "sha256": sha256_bytes(payload),
            }
    return {
        "schema": "gfx_icon_asset_manifest_static.power_balance_gui_roles.v2",
        "sources": dict(sorted(sources.items())),
        "roles": dict(sorted(roles.items())),
    }


def _flag_files(layers) -> list[tuple[str, Path]]:
    return R.ordered_files(layers, "gfx/flags", (".dds", ".tga"))


def _renderable_blocks(node: Node):
    """Yield every named drawable declared below ``spriteTypes``.

    Clausewitz does not limit that container to the literal ``spriteType``
    key.  In particular, the a729 power-balance file uses
    ``progressBarType`` for all four value bars and
    ``frameAnimatedSpriteType`` for the launcher glow.  Treating only the
    plain key as an asset definition made those production sprites impossible
    to bind to portable texture identities.
    """
    for key, value in node.entries:
        if not isinstance(value, Node):
            continue
        if key.lower() == "spritetypes":
            for drawable_kind, block in value.entries:
                if isinstance(block, Node) and scalar(block.get("name")):
                    yield drawable_kind, block
        else:
            yield from _renderable_blocks(value)


def _sprite_blocks(node: Node):
    """Compatibility iterator used by focused parser tests and callers."""
    for _kind, block in _renderable_blocks(node):
        yield block


def _texture_slot_rows(
    block: Node,
    layers,
    asset_lookup: dict[str, tuple[str, Path]],
) -> list[dict[str, Any]]:
    rows = []
    for key, raw_value in block.entries:
        if not key.lower().startswith("texturefile"):
            continue
        raw_texture = scalar(raw_value)
        if not raw_texture or not raw_texture.strip():
            continue
        texture = _normalise_texture_path(raw_texture)
        raw_texture_redacted = _redact_raw_texture_path(raw_texture)
        invalid_texture_ref = not texture
        resolved_asset = _resolve_asset(layers, asset_lookup, texture)
        asset_path = resolved_asset["path"] if resolved_asset else None
        row = {
            "slot": key,
            "texturefile": texture,
            "raw_texturefile": raw_texture_redacted
            if (invalid_texture_ref or _is_absolute_texture_path(raw_texture))
            else None,
            "invalid_texturefile": invalid_texture_ref,
            "resolved_texturefile": resolved_asset["resolved_relpath"]
            if resolved_asset
            else None,
            "texture_layer": _source_label(layers, asset_path) if asset_path else None,
            "texture_layer_kind": _source_layer_kind(layers, asset_path)
            if asset_path
            else None,
            "texture_bytes": asset_path.stat().st_size if asset_path else None,
            "texture_sha256": sha256_bytes(asset_path.read_bytes()) if asset_path else None,
        }
        if resolved_asset and resolved_asset["resolved_via"] != "exact":
            row["resolved_via"] = resolved_asset["resolved_via"]
        rows.append(row)
    return rows


def _normalise_texture_path(value: str | None) -> str | None:
    if not value:
        return None
    text = value.replace("\\", "/").strip()
    if text[1:3] == ":/" or text.startswith("/"):
        idx = text.lower().find("gfx/")
        text = text[idx:] if idx != -1 else ""
    while "//" in text:
        text = text.replace("//", "/")
    return text.lstrip("/")


def _is_absolute_texture_path(value: str | None) -> bool:
    if not value:
        return False
    text = value.replace("\\", "/").strip()
    return text[1:3] == ":/" or text.startswith("/")


def _redact_raw_texture_path(value: str | None) -> str | None:
    if not value:
        return None
    text = value.replace("\\", "/").strip()
    if _is_absolute_texture_path(text):
        name = Path(text).name
        return f"<absolute>/{name}" if name else "<absolute>"
    return text


def _asset_lookup(layers) -> dict[str, tuple[str, Path]]:
    files = R.effective_files(layers, "gfx", TEXTURE_EXTS)
    return {rel.casefold(): (rel, path) for rel, path in files.items()}


def _resolve_asset(
    layers,
    lookup: dict[str, tuple[str, Path]],
    relpath: str | None,
) -> dict[str, Any] | None:
    if not relpath:
        return None
    key = relpath.casefold()
    exact = lookup.get(key)
    if exact:
        actual_rel, path = exact
        return {"resolved_via": "exact", "resolved_relpath": actual_rel, "path": path}

    base, dot, _ext = relpath.rpartition(".")
    if not dot:
        return None
    prio = R._layer_prio_fn(layers)
    candidates = []
    for order, ext in enumerate(EXTENSION_FALLBACK_EXTS):
        alt = f"{base}{ext}"
        if alt.casefold() == key:
            continue
        hit = lookup.get(alt.casefold())
        if hit:
            actual_rel, path = hit
            candidates.append((prio(path), -order, actual_rel, path))
    if not candidates:
        return None
    _layer_index, _order, actual_rel, path = max(candidates, key=lambda item: (item[0], item[1]))
    return {"resolved_via": "extension_fallback", "resolved_relpath": actual_rel, "path": path}


def _classify(sprite_name: str, texture_path: str | None) -> str:
    text = f"{sprite_name} {texture_path or ''}".lower()
    if "decision" in text:
        return "decision_icon"
    if "goal" in text or "focus" in text:
        return "focus_icon"
    if "idea" in text:
        return "idea_icon"
    if "equipment" in text or "equipments" in text:
        return "equipment_icon"
    if "unit" in text or "division" in text:
        return "unit_icon"
    if "ideolog" in text or "party" in text:
        return "ideology_icon"
    if "flag" in text:
        return "flag_related_sprite"
    return "other_sprite"


def _load_missing_texture_allowlist(path: Path) -> dict[MissingTextureKey, str]:
    if not path.is_file():
        return {}
    data = json.loads(path.read_text(encoding="utf-8"))
    entries = data.get("entries", data if isinstance(data, list) else [])
    out: dict[MissingTextureKey, str] = {}
    for index, entry in enumerate(entries):
        if not isinstance(entry, dict):
            raise RuntimeError(f"malformed missing texture allowlist entry #{index}: expected object")
        name = entry.get("sprite")
        texture = entry.get("texturefile")
        source = entry.get("source_file")
        source_layer = entry.get("source_layer")
        reason = entry.get("reason")
        if not all(isinstance(v, str) and v for v in (name, texture, source, source_layer, reason)):
            raise RuntimeError(
                f"malformed missing texture allowlist entry #{index}: "
                "sprite, texturefile, source_file, source_layer, and reason are required"
            )
        key = (name, texture, source, source_layer)
        if key in out:
            raise RuntimeError(
                "duplicate missing texture allowlist identity: "
                f"sprite={name!r} texturefile={texture!r} "
                f"source_file={source!r} source_layer={source_layer!r}"
            )
        out[key] = reason
    return out


def build_catalog(args: argparse.Namespace) -> dict[str, Any]:
    layers, asset_layer_meta = build_gfx_asset_layers(
        vanilla=args.vanilla,
        mods=mods_from_arg(args.mods),
        userdata=args.userdata,
        strict_mods=getattr(args, "strict_mods", False),
        source_inventory_path=getattr(args, "source_inventory", None),
    )
    warnings = []
    sprites: dict[str, dict[str, Any]] = {}
    sprite_keys_by_casefold: dict[str, str] = {}
    collisions = []
    interface_files = _direct_interface_files(layers)
    allowlist = _load_missing_texture_allowlist(args.missing_texture_allowlist)
    unresolved_textures: list[dict[str, Any]] = []
    allowed_missing_textures: list[dict[str, Any]] = []
    used_allowlist_keys: set[MissingTextureKey] = set()
    syntax_allowlist_hits: list[dict[str, str]] = []
    interface_files_parsed = 0
    asset_lookup = _asset_lookup(layers)

    for rel, path in interface_files:
        try:
            text = R.read_text(path)
            allowlist_hit = validate_clausewitz_text(
                text,
                rel,
                allowlist=CLAUSEWITZ_SYNTAX_QUIRK_ALLOWLIST,
            )
            if allowlist_hit:
                syntax_allowlist_hits.append({"kind": "gfx", **allowlist_hit})
            root = parse(text)
        except Exception as e:
            msg = f"{rel}: {e}"
            if args.strict:
                raise RuntimeError(f"failed to parse gfx file {msg}") from e
            warnings.append(msg)
            continue
        interface_files_parsed += 1
        for drawable_kind, block in _renderable_blocks(root):
            name = scalar(block.get("name"))
            if not name:
                continue
            texture_slots = _texture_slot_rows(block, layers, asset_lookup)
            primary = texture_slots[0] if texture_slots else {}
            raw_texture = scalar(block.get("texturefile"))
            if raw_texture is None and texture_slots:
                raw_texture = scalar(next(
                    (value for key, value in block.entries
                     if key.lower().startswith("texturefile")),
                    None,
                ))
            texture = primary.get("texturefile")
            raw_texture_redacted = primary.get("raw_texturefile")
            invalid_texture_ref = bool(primary.get("invalid_texturefile"))
            raw_data = node_to_data(block)
            raw_is_absolute = _is_absolute_texture_path(raw_texture)
            if isinstance(raw_data, dict):
                for raw_key in list(raw_data):
                    if not raw_key.lower().startswith("texturefile"):
                        continue
                    raw_value = raw_data[raw_key]
                    if isinstance(raw_value, str) and _is_absolute_texture_path(raw_value):
                        raw_data[raw_key] = _redact_raw_texture_path(raw_value)
                    elif isinstance(raw_value, list):
                        raw_data[raw_key] = [
                            _redact_raw_texture_path(value)
                            if isinstance(value, str) and _is_absolute_texture_path(value)
                            else value
                            for value in raw_value
                        ]
            row = {
                "name": name,
                "drawable_kind": drawable_kind,
                "category": _classify(name, texture or raw_texture_redacted),
                "texturefile": texture,
                "raw_texturefile": raw_texture_redacted
                if (invalid_texture_ref or raw_is_absolute)
                else None,
                "invalid_texturefile": invalid_texture_ref,
                "resolved_texturefile": primary.get("resolved_texturefile"),
                "texture_layer": primary.get("texture_layer"),
                "texture_layer_kind": primary.get("texture_layer_kind"),
                "texture_bytes": primary.get("texture_bytes"),
                "texture_sha256": primary.get("texture_sha256"),
                "texture_slots": texture_slots,
                "frames": scalar(block.get("noOfFrames")),
                "source_layer": _source_label(layers, path),
                "source_layer_kind": _source_layer_kind(layers, path),
                "source_file": rel,
                "raw": raw_data,
            }
            if primary.get("resolved_via"):
                row["resolved_via"] = primary["resolved_via"]
            folded_name = name.casefold()
            prior_key = sprite_keys_by_casefold.get(folded_name)
            if prior_key is not None:
                prior = sprites.pop(prior_key)
                collisions.append({
                    "id": name,
                    "first_id": prior_key,
                    "first": prior["source_file"],
                    "override": rel,
                    "case_insensitive": prior_key != name,
                })
            sprites[name] = row
            sprite_keys_by_casefold[folded_name] = name

    flags = []
    for rel, path in _flag_files(layers):
        flags.append(
            {
                "relpath": rel,
                "source_layer": _source_label(layers, path),
                "source_layer_kind": _source_layer_kind(layers, path),
                "bytes": path.stat().st_size,
                "sha256": sha256_bytes(path.read_bytes()),
            }
        )

    by_category: dict[str, int] = {}
    texture_resolution_by_kind: dict[str, int] = {}
    texture_resolution_by_method: dict[str, int] = {}
    missing_textures = 0
    for row in sprites.values():
        by_category[row["category"]] = by_category.get(row["category"], 0) + 1
        for slot in row.get("texture_slots", []):
            if slot["texturefile"] and slot["texture_sha256"]:
                layer_kind = slot.get("texture_layer_kind") or "unknown"
                texture_resolution_by_kind[layer_kind] = (
                    texture_resolution_by_kind.get(layer_kind, 0) + 1
                )
                method = slot.get("resolved_via") or "exact"
                texture_resolution_by_method[method] = (
                    texture_resolution_by_method.get(method, 0) + 1
                )
            texture_identity = slot["texturefile"] or slot.get("raw_texturefile")
            if not (slot["texturefile"] or slot.get("invalid_texturefile")) or slot["texture_sha256"]:
                continue
            missing_textures += 1
            missing = {
                "sprite": row["name"],
                "texturefile": texture_identity,
                "texture_slot": slot["slot"],
                "source_file": row["source_file"],
                "source_layer": row["source_layer"],
                "category": row["category"],
            }
            if slot.get("invalid_texturefile"):
                missing["raw_texturefile"] = slot.get("raw_texturefile")
                missing["invalid_texturefile"] = True
            allowlist_key = (row["name"], texture_identity, row["source_file"], row["source_layer"])
            reason = allowlist.get(allowlist_key)
            if reason:
                used_allowlist_keys.add(allowlist_key)
                row["missing_texture_allowlist_reason"] = reason
                allowed_missing_textures.append(missing | {"reason": reason})
            else:
                unresolved_textures.append(missing)
    unused_allowlist_keys = sorted(
        set(allowlist) - used_allowlist_keys,
        key=lambda item: (item[2], item[3], item[0], item[1]),
    )

    power_balance_gui = _power_balance_gui_manifest(layers)
    catalog = {
        "schema": "gfx_icon_asset_manifest_static.static_artifact.v2",
        "artifact_kind": "deterministic_static_replay_enrichment",
        "scope": {
            "closed_rows": [
                "interface/*.gfx named spriteTypes drawable -> every texturefile slot path/hash",
                "effective powerbalanceview/countrypoliticsview GUI semantic role -> authored or engine-selected sprite binding",
                "gfx/flags file path/hash inventory for country flags",
            ],
            "not_closed_rows": [
                "image decoding, atlas extraction, or generated thumbnails",
                "runtime selection of ideology/cosmetic flag variant",
            ],
        },
        "join_keys": {
            "sprite_name": "sprites.<GFX_name>",
            "texture_path": "sprites.<GFX_name>.texturefile",
            "texture_slots": "sprites.<GFX_name>.texture_slots[*].slot",
            "power_balance_gui_role": "power_balance_gui.roles.<semantic_role>",
            "flag_path": "flags[*].relpath",
        },
        "layers": _gfx_layer_summary(layers),
        "meta": {
            "asset_layer_model": asset_layer_meta,
            "interface_gfx_files": len(interface_files),
            "interface_gfx_files_parsed": interface_files_parsed,
            "sprites": len(sprites),
            "flag_assets": len(flags),
            "missing_textures": missing_textures,
            "allowed_missing_textures": len(allowed_missing_textures),
            "unresolved_textures": len(unresolved_textures),
            "unused_missing_texture_allowlist_entries": len(unused_allowlist_keys),
            "texture_resolution_by_layer_kind": dict(sorted(texture_resolution_by_kind.items())),
            "texture_resolution_by_method": dict(sorted(texture_resolution_by_method.items())),
            "sprites_by_category": dict(sorted(by_category.items())),
            "warnings": warnings,
            "id_collisions_last_wins": collisions,
            "missing_texture_allowlist": {
                "path": repo_relative(args.missing_texture_allowlist),
                "entries": len(allowlist),
            },
            "unused_missing_texture_allowlist": [
                {
                    "sprite": sprite,
                    "texturefile": texture,
                    "source_file": source,
                    "source_layer": source_layer,
                }
                for sprite, texture, source, source_layer in unused_allowlist_keys
            ],
            "syntax_allowlist_hits": sorted(
                syntax_allowlist_hits,
                key=lambda row: (row["kind"], row["source_file"], row["matched_error"]),
            ),
        },
        "source_files": {
            "interface_gfx_files": source_inventory(interface_files),
            "flag_files": source_inventory(_flag_files(layers)),
        },
        "sprites": dict(sorted(sprites.items())),
        "power_balance_gui": power_balance_gui,
        "flags": sorted(flags, key=lambda row: row["relpath"]),
        "allowed_missing_textures": sorted(
            allowed_missing_textures,
            key=lambda row: (row["source_file"], row["sprite"], row["texturefile"]),
        ),
        "unresolved_textures": sorted(
            unresolved_textures,
            key=lambda row: (row["source_file"], row["sprite"], row["texturefile"]),
        ),
    }
    catalog["artifact_sha256"] = sha256_bytes(canonical_json(catalog))
    return catalog


def catalog_gate_checks(
    catalog: dict[str, Any],
    *,
    min_sprites: int = 1,
    store_verify_result: dict[str, Any] | None = None,
) -> dict[str, bool]:
    """Shared GFX manifest pass criteria.

    Store verification is only available in the standalone gate after writing
    the static store. Build-all callers pass no store result and still get the
    catalog-level closure checks.
    """

    meta = catalog["meta"]
    checks = {
        "min_sprites": meta["sprites"] >= min_sprites,
        "interface_gfx_files_parsed": meta["interface_gfx_files_parsed"] == meta["interface_gfx_files"],
        "unresolved_textures": meta["unresolved_textures"] == 0,
        "allowed_missing_textures": meta["allowed_missing_textures"]
        == meta["missing_textures"],
        "warnings": not meta["warnings"],
    }
    if store_verify_result is not None:
        checks["store_verify"] = bool(store_verify_result.get("passed"))
    return checks


def catalog_gate_passed(
    catalog: dict[str, Any],
    *,
    min_sprites: int = 1,
    store_verify_result: dict[str, Any] | None = None,
) -> bool:
    return all(catalog_gate_checks(catalog, min_sprites=min_sprites, store_verify_result=store_verify_result).values())


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    add_layer_args(parser)
    parser.add_argument("--out-dir", type=Path, default=DEFAULT_OUT)
    parser.add_argument("--min-sprites", type=int, default=1)
    parser.add_argument("--missing-texture-allowlist", type=Path, default=DEFAULT_MISSING_TEXTURE_ALLOWLIST)
    parser.add_argument("--strict", action="store_true", default=True)
    args = parser.parse_args()

    catalog = build_catalog(args)
    paths = write_static_store(
        args.out_dir,
        game_id=GAME_ID,
        catalog_name="gfx_icon_manifest_catalog.json",
        catalog=catalog,
        snapshot_schema="gfx_icon_asset_manifest_static.static_snapshot.v1",
        index_schema="gfx_icon_asset_manifest_static.static_index.v1",
        manifest_role="gfx_icon_asset_manifest_static_manifest",
        payload_role="gfx_icon_asset_manifest_static_payload",
        snapshot_extra={"counts": catalog["meta"], "artifact_sha256": catalog["artifact_sha256"]},
    )
    store_verify = verify_store(paths)
    gate_checks = catalog_gate_checks(catalog, min_sprites=args.min_sprites, store_verify_result=store_verify)
    passed = all(gate_checks.values())
    report = {
        "schema": "gfx_icon_asset_manifest_static.static_gate_report.v1",
        "passed": passed,
        "paths": {key: repo_relative(path) for key, path in paths.items()},
        "counts": {
            "interface_gfx_files": catalog["meta"]["interface_gfx_files"],
            "interface_gfx_files_parsed": catalog["meta"]["interface_gfx_files_parsed"],
            "sprites": catalog["meta"]["sprites"],
            "flag_assets": catalog["meta"]["flag_assets"],
            "missing_textures": catalog["meta"]["missing_textures"],
            "allowed_missing_textures": catalog["meta"]["allowed_missing_textures"],
            "unresolved_textures": catalog["meta"]["unresolved_textures"],
            "unused_missing_texture_allowlist_entries": catalog["meta"][
                "unused_missing_texture_allowlist_entries"
            ],
        },
        "asset_layer_model": catalog["meta"]["asset_layer_model"],
        "texture_resolution_by_layer_kind": catalog["meta"]["texture_resolution_by_layer_kind"],
        "texture_resolution_by_method": catalog["meta"]["texture_resolution_by_method"],
        "sprites_by_category": catalog["meta"]["sprites_by_category"],
        "artifact_sha256": catalog["artifact_sha256"],
        "store_verify": store_verify,
        "gate_checks": gate_checks,
        "warnings": catalog["meta"]["warnings"],
        "warning_count": len(catalog["meta"]["warnings"]),
        "missing_texture_allowlist": catalog["meta"]["missing_texture_allowlist"],
        "unused_missing_texture_allowlist": catalog["meta"]["unused_missing_texture_allowlist"],
        "syntax_allowlist_hits": catalog["meta"]["syntax_allowlist_hits"],
        "unresolved_textures": catalog["unresolved_textures"],
        "allowed_missing_textures": catalog["allowed_missing_textures"],
    }
    report_path = args.out_dir / "gfx_icon_manifest_gate_report.json"
    report_path.write_bytes(pretty_json(report))
    sys.stdout.buffer.write(
        (json.dumps(report, ensure_ascii=False, sort_keys=True, indent=2) + "\n").encode("utf-8")
    )
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
