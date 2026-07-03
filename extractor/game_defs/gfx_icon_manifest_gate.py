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

import resolver as R  # noqa: E402
from paradox_parser import Node, parse  # noqa: E402
from static_artifact_common import (  # noqa: E402
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
) -> tuple[list[Any], dict[str, Any]]:
    """Build the GFX-only effective asset layer stack.

    Definition parsers keep using ``resolver.build_layers``. This opt-in model
    adds the game asset DLC roots between vanilla and mods because interface sprites,
    textures, and flags are shipped there even though common/localisation data is
    not.
    """

    vanilla_root = R.vanilla_root(vanilla)
    userdata_root = Path(userdata).resolve() if userdata else R.DEFAULT_USERDATA
    mod_layers = R.build_layers(vanilla=str(vanilla_root), mods=mods, userdata=userdata_root)

    integrated_dirs = _installed_dlc_dirs(vanilla_root, "integrated_dlc")
    installed_dlc_dirs = _installed_dlc_dirs(vanilla_root, "dlc")
    dlc_selection = _load_dlc_selection(userdata_root, installed_dlc_dirs)
    enabled_set = {name.casefold() for name in dlc_selection["enabled_dlcs"]}
    enabled_dlc_dirs = [path for path in installed_dlc_dirs if path.name.casefold() in enabled_set]

    selection_meta = {
        "dlc_load_json_present": dlc_selection["dlc_load_json_present"],
        "missing_dlc_load_means_all_installed_enabled": dlc_selection[
            "missing_dlc_load_means_all_installed_enabled"
        ],
        "enabled_dlcs": dlc_selection["enabled_dlcs"],
        "disabled_dlcs": dlc_selection["disabled_dlcs"],
    }
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
    for layer in mod_layers[1:]:
        layers.append(_clone_layer(layer, meta_extra={"kind": "mod"}))

    meta = {
        "model": "vanilla root -> integrated_dlc/* -> enabled dlc/* -> mods",
        "integrated_dlcs": [path.name for path in integrated_dirs],
        "installed_dlcs": dlc_selection["installed_dlcs"],
        "enabled_dlcs": dlc_selection["enabled_dlcs"],
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


def _flag_files(layers) -> list[tuple[str, Path]]:
    return R.ordered_files(layers, "gfx/flags", (".dds", ".tga"))


def _sprite_blocks(node: Node):
    for key, value in node.entries:
        if not isinstance(value, Node):
            continue
        if key.lower() == "spritetype":
            yield value
        else:
            yield from _sprite_blocks(value)


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
    )
    warnings = []
    sprites: dict[str, dict[str, Any]] = {}
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
        for block in _sprite_blocks(root):
            name = scalar(block.get("name"))
            if not name:
                continue
            raw_texture = scalar(block.get("texturefile"))
            texture = _normalise_texture_path(raw_texture)
            raw_texture_redacted = _redact_raw_texture_path(raw_texture)
            has_texture_ref = bool(raw_texture and raw_texture.strip())
            invalid_texture_ref = bool(has_texture_ref and not texture)
            resolved_asset = _resolve_asset(layers, asset_lookup, texture)
            asset_path = resolved_asset["path"] if resolved_asset else None
            asset_layer = _source_label(layers, asset_path) if asset_path else None
            asset_layer_kind = _source_layer_kind(layers, asset_path) if asset_path else None
            raw_data = node_to_data(block)
            # An absolute-looking ref is ALWAYS redacted in the persisted raw
            # block, even when a game-relative path could be recovered from it
            # (containing "gfx/") — no personal/install path may reach the
            # packed catalog through any branch.
            raw_is_absolute = _is_absolute_texture_path(raw_texture)
            if (invalid_texture_ref or raw_is_absolute) and isinstance(raw_data, dict):
                raw_data["texturefile"] = raw_texture_redacted
            row = {
                "name": name,
                "category": _classify(name, texture or raw_texture_redacted),
                "texturefile": texture,
                "raw_texturefile": raw_texture_redacted
                if (invalid_texture_ref or raw_is_absolute)
                else None,
                "invalid_texturefile": invalid_texture_ref,
                "resolved_texturefile": resolved_asset["resolved_relpath"] if resolved_asset else None,
                "texture_layer": asset_layer,
                "texture_layer_kind": asset_layer_kind,
                "texture_bytes": asset_path.stat().st_size if asset_path else None,
                "texture_sha256": sha256_bytes(asset_path.read_bytes()) if asset_path else None,
                "frames": scalar(block.get("noOfFrames")),
                "source_layer": _source_label(layers, path),
                "source_layer_kind": _source_layer_kind(layers, path),
                "source_file": rel,
                "raw": raw_data,
            }
            if resolved_asset and resolved_asset["resolved_via"] != "exact":
                row["resolved_via"] = resolved_asset["resolved_via"]
            if name in sprites and sprites[name]["source_file"] != rel:
                collisions.append({"id": name, "first": sprites[name]["source_file"], "override": rel})
            sprites[name] = row

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
        if row["texturefile"] and row["texture_sha256"]:
            layer_kind = row.get("texture_layer_kind") or "unknown"
            texture_resolution_by_kind[layer_kind] = texture_resolution_by_kind.get(layer_kind, 0) + 1
            method = row.get("resolved_via") or "exact"
            texture_resolution_by_method[method] = texture_resolution_by_method.get(method, 0) + 1
        texture_identity = row["texturefile"] or row.get("raw_texturefile")
        if (row["texturefile"] or row.get("invalid_texturefile")) and not row["texture_sha256"]:
            missing_textures += 1
            missing = {
                "sprite": row["name"],
                "texturefile": texture_identity,
                "source_file": row["source_file"],
                "source_layer": row["source_layer"],
                "category": row["category"],
            }
            if row.get("invalid_texturefile"):
                missing["raw_texturefile"] = row.get("raw_texturefile")
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

    catalog = {
        "schema": "gfx_icon_asset_manifest_static.static_artifact.v1",
        "artifact_kind": "deterministic_static_replay_enrichment",
        "scope": {
            "closed_rows": [
                "interface/*.gfx SpriteType name -> texturefile path/hash",
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
        == meta["missing_texture_allowlist"]["entries"],
        "unused_missing_texture_allowlist_entries": meta["unused_missing_texture_allowlist_entries"] == 0,
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
