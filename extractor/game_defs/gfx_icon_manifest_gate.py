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
    layer_summary,
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


def _source_label(layers, abspath) -> str:
    return layers[R._layer_prio_fn(layers)(abspath)].label


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
    return value.replace("\\", "/").lstrip("/")


def _resolve_asset(layers, relpath: str | None) -> tuple[str | None, Path | None]:
    if not relpath:
        return None, None
    rel = Path(relpath)
    for layer in reversed(layers):
        candidate = Path(layer.root) / rel
        if candidate.is_file():
            return layer.label, candidate
    return None, None


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


def _load_missing_texture_allowlist(path: Path) -> dict[tuple[str, str, str], str]:
    if not path.is_file():
        return {}
    data = json.loads(path.read_text(encoding="utf-8"))
    entries = data.get("entries", data if isinstance(data, list) else [])
    out: dict[tuple[str, str, str], str] = {}
    for entry in entries:
        if not isinstance(entry, dict):
            continue
        name = entry.get("sprite")
        texture = entry.get("texturefile")
        source = entry.get("source_file")
        reason = entry.get("reason")
        if not all(isinstance(v, str) and v for v in (name, texture, source, reason)):
            continue
        out[(name, texture, source)] = reason
    return out


def build_catalog(args: argparse.Namespace) -> dict[str, Any]:
    layers = R.build_layers(
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
    syntax_allowlist_hits: list[dict[str, str]] = []
    interface_files_parsed = 0

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
            texture = _normalise_texture_path(scalar(block.get("texturefile")))
            asset_layer, asset_path = _resolve_asset(layers, texture)
            row = {
                "name": name,
                "category": _classify(name, texture),
                "texturefile": texture,
                "texture_layer": asset_layer,
                "texture_bytes": asset_path.stat().st_size if asset_path else None,
                "texture_sha256": sha256_bytes(asset_path.read_bytes()) if asset_path else None,
                "frames": scalar(block.get("noOfFrames")),
                "source_layer": _source_label(layers, path),
                "source_file": rel,
                "raw": node_to_data(block),
            }
            if name in sprites and sprites[name]["source_file"] != rel:
                collisions.append({"id": name, "first": sprites[name]["source_file"], "override": rel})
            sprites[name] = row

    flags = []
    for rel, path in _flag_files(layers):
        flags.append(
            {
                "relpath": rel,
                "source_layer": _source_label(layers, path),
                "bytes": path.stat().st_size,
                "sha256": sha256_bytes(path.read_bytes()),
            }
        )

    by_category: dict[str, int] = {}
    missing_textures = 0
    for row in sprites.values():
        by_category[row["category"]] = by_category.get(row["category"], 0) + 1
        if row["texturefile"] and not row["texture_sha256"]:
            missing_textures += 1
            missing = {
                "sprite": row["name"],
                "texturefile": row["texturefile"],
                "source_file": row["source_file"],
                "source_layer": row["source_layer"],
                "category": row["category"],
            }
            reason = allowlist.get((row["name"], row["texturefile"], row["source_file"]))
            if reason:
                row["missing_texture_allowlist_reason"] = reason
                allowed_missing_textures.append(missing | {"reason": reason})
            else:
                unresolved_textures.append(missing)

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
        "layers": layer_summary(layers),
        "meta": {
            "interface_gfx_files": len(interface_files),
            "interface_gfx_files_parsed": interface_files_parsed,
            "sprites": len(sprites),
            "flag_assets": len(flags),
            "missing_textures": missing_textures,
            "allowed_missing_textures": len(allowed_missing_textures),
            "unresolved_textures": len(unresolved_textures),
            "sprites_by_category": dict(sorted(by_category.items())),
            "warnings": warnings,
            "id_collisions_last_wins": collisions,
            "missing_texture_allowlist": {
                "path": repo_relative(args.missing_texture_allowlist),
                "entries": len(allowlist),
            },
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
    passed = bool(
        catalog["meta"]["sprites"] >= args.min_sprites
        and catalog["meta"]["interface_gfx_files_parsed"] == catalog["meta"]["interface_gfx_files"]
        and catalog["meta"]["unresolved_textures"] == 0
        and not catalog["meta"]["warnings"]
        and store_verify["passed"]
    )
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
        },
        "sprites_by_category": catalog["meta"]["sprites_by_category"],
        "artifact_sha256": catalog["artifact_sha256"],
        "store_verify": store_verify,
        "warnings": catalog["meta"]["warnings"],
        "warning_count": len(catalog["meta"]["warnings"]),
        "missing_texture_allowlist": catalog["meta"]["missing_texture_allowlist"],
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
