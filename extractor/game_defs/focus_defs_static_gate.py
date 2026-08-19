#!/usr/bin/env python3
"""Build and verify the production national-focus definition artifact."""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Any

_HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(_HERE))

import parse_focus_def
import parse_localisation
import resolver as R
import source_inventory_layers as SIL
from static_artifact_common import (
    add_layer_args,
    canonical_json,
    layer_summary,
    load_localisation_catalog,
    mods_from_arg,
    pretty_json,
    repo_relative,
    sha256_bytes,
    source_inventory,
    verify_store,
    write_static_store,
)

GAME_ID = "focus_defs_static"
SCHEMA = "focus_defs_static.static_artifact.v1"
DEFAULT_OUT = _HERE / "out" / "focus_defs_static"
DEFAULT_LOCALISATION_CATALOG = (
    _HERE / "out" / "localisation_gate" / "static_localisation_join_r3"
    / "localisation_catalog.json"
)
_PORTABLE_ID = re.compile(r"^[A-Za-z0-9_][A-Za-z0-9_.:-]*$")


class _CatalogLoc:
    def __init__(self, keys: dict[str, Any]):
        self._keys = keys

    def resolve(self, key: str) -> str | None:
        row = self._keys.get(key)
        if not isinstance(row, dict):
            return None
        value = row.get("name")
        return value if isinstance(value, str) else None


def _localisation(
    path: Path, layers: list[Any]
) -> tuple[_CatalogLoc, dict[str, Any]]:
    if path.is_file():
        data = load_localisation_catalog(path)
        return _CatalogLoc(data["keys"]), {
            "source": "static_localisation_join_r3",
            "path": repo_relative(path),
            "key_count": data.get("meta", {}).get("key_count", len(data["keys"])),
            "sha256": sha256_bytes(path.read_bytes()),
        }
    built = parse_localisation.parse_localisation(layers)
    return _CatalogLoc(built["keys"]), {
        "source": "built_in_gate_missing_r3_catalog",
        "path": None,
        "key_count": built["meta"]["key_count"],
        "sha256": sha256_bytes(canonical_json(built)),
    }


def _validate_catalog(parsed: dict[str, Any]) -> None:
    meta = parsed["meta"]
    trees = parsed["trees"]
    focuses = parsed["focuses"]
    styles = parsed["focus_background_styles"]
    if (
        meta["tree_count"] != len(trees)
        or meta["focus_count"] != len(focuses)
        or meta["focus_background_style_count"] != len(styles)
        or meta["files_parsed"] <= 0
    ):
        raise RuntimeError("focus definition declared counts disagree with parsed objects")
    if not trees or not focuses or not styles:
        raise RuntimeError("focus definition catalog is empty")
    if meta["id_collisions_last_wins"]:
        raise RuntimeError("focus definition IDs collide across effective source files")
    if meta["unresolved_focus_background_styles"]:
        raise RuntimeError("focus definitions reference unresolved background styles")
    if len(meta["default_focus_background_styles"]) != 1:
        raise RuntimeError("focus background registry must have exactly one default style")
    for style_id, style in styles.items():
        if (
            style.get("name") != style_id
            or not style.get("source_file")
            or not style.get("source_layer")
            or any(not style.get(state) for state in ("unavailable", "available", "current", "completed"))
        ):
            raise RuntimeError(f"focus background style {style_id!r} is incomplete")
    kind_counts = {"focus": 0, "shared_focus": 0, "joint_focus": 0}
    for focus_id, focus in focuses.items():
        kind = focus.get("kind")
        if (
            focus.get("id") != focus_id
            or not _PORTABLE_ID.fullmatch(focus_id)
            or kind not in kind_counts
            or focus.get("title_loc_key") != focus_id
            or focus.get("description_loc_key") != f"{focus_id}_desc"
            or not focus.get("source_file")
            or not focus.get("source_layer")
        ):
            raise RuntimeError(f"focus definition {focus_id!r} has inconsistent identity")
        for scripted_key in ("scripted_position_offsets", "allow_branch_blocks"):
            value = focus.get(scripted_key)
            if not isinstance(value, int) or isinstance(value, bool) or value < 0:
                raise RuntimeError(
                    f"focus definition {focus_id!r} has no valid {scripted_key} count"
                )
        exclusions = focus.get("mutually_exclusive")
        if (
            not isinstance(exclusions, list)
            or any(not _PORTABLE_ID.fullmatch(other) for other in exclusions if isinstance(other, str))
            or any(not isinstance(other, str) for other in exclusions)
            or len(set(exclusions)) != len(exclusions)
            or focus_id in exclusions
        ):
            raise RuntimeError(
                f"focus definition {focus_id!r} has a malformed mutually_exclusive set"
            )
        kind_counts[kind] += 1
        style = focus.get("background_style")
        style_name = focus.get("background_style_name")
        if style_name not in styles or style != styles[style_name]:
            raise RuntimeError(f"focus definition {focus_id!r} has inconsistent style binding")
        if kind == "joint_focus" and not focus.get("text_icon"):
            raise RuntimeError(f"joint focus {focus_id!r} has no authored style family")
    if kind_counts["shared_focus"] != meta["shared_focus_count"]:
        raise RuntimeError("shared focus count disagrees with parsed definition kinds")
    if kind_counts["joint_focus"] != meta["joint_focus_count"]:
        raise RuntimeError("joint focus count disagrees with parsed definition kinds")
    for tree_id, tree in trees.items():
        if (
            tree.get("id") != tree_id
            or not _PORTABLE_ID.fullmatch(tree_id)
            or tree.get("focus_count") != len(tree.get("focuses", []))
            or not tree.get("source_file")
            or not tree.get("source_layer")
            or len(tree.get("focuses", [])) != len(set(tree.get("focuses", [])))
            or len(tree.get("shared_focus_refs", []))
            != len(set(tree.get("shared_focus_refs", [])))
        ):
            raise RuntimeError(f"focus tree {tree_id!r} has inconsistent topology")
        for focus_id in tree["focuses"]:
            definition = focuses.get(focus_id)
            if not isinstance(definition, dict) or definition.get("kind") != "focus":
                raise RuntimeError(
                    f"focus tree {tree_id!r} references absent/non-local node {focus_id!r}"
                )
        for focus_id in tree["shared_focus_refs"]:
            definition = focuses.get(focus_id)
            if not isinstance(definition, dict) or definition.get("kind") not in {
                "shared_focus", "joint_focus"
            }:
                raise RuntimeError(
                    f"focus tree {tree_id!r} references absent shared/joint root {focus_id!r}"
                )
    local_ids = {
        focus_id
        for focus_id, focus in focuses.items()
        if focus["kind"] == "focus"
    }
    owned_local_ids = [
        focus_id
        for tree in trees.values()
        for focus_id in tree["focuses"]
    ]
    if (
        len(owned_local_ids) != len(set(owned_local_ids))
        or set(owned_local_ids) != local_ids
    ):
        raise RuntimeError(
            "focus tree topology must own every local focus definition exactly once"
        )


def build_catalog(args: argparse.Namespace) -> dict[str, Any]:
    layers = R.build_layers(
        vanilla=args.vanilla,
        mods=mods_from_arg(args.mods),
        userdata=args.userdata,
        strict_mods=getattr(args, "strict_mods", False),
    )
    layers, source_namespace_proof = SIL.definition_layers(
        layers, getattr(args, "source_inventory", None)
    )
    loc, loc_source = _localisation(args.localisation_catalog, layers)
    parsed = parse_focus_def.parse_focus_defs(layers, loc, strict=True)
    _validate_catalog(parsed)
    definition_files = R.ordered_files(
        layers, "common/national_focus", (".txt",)
    )
    catalog = {
        "schema": SCHEMA,
        "artifact_kind": "deterministic_static_replay_enrichment",
        "scope": {
            "closed_rows": [
                "focus tree local node ids and imported shared/joint roots",
                "local/shared/joint definition kind and authored geometry",
                "exact icon, overlay, title/description localisation keys",
                "authored focus state background style families",
            ],
            "not_closed_rows": [
                "runtime focus visibility, availability, progress, state, and order",
                "runtime joint-focus participant selection",
                "scripted offset trigger evaluation",
            ],
        },
        "join_keys": {
            "runtime_tree_id": "trees.<id>",
            "runtime_visible_focus_id": "focuses.<id>",
            "localisation_title_key": "focuses.<id>.title_loc_key",
            "compatibility_status": (
                "exact string ids; runtime visible order selects joint nodes and "
                "the replay consumer rejects non-joint topology extras"
            ),
        },
        "layers": layer_summary(layers),
        "source_namespace_proof": source_namespace_proof,
        "localisation_source": loc_source,
        "source_files": source_inventory(definition_files),
        **parsed,
    }
    catalog["artifact_sha256"] = sha256_bytes(canonical_json(catalog))
    return catalog


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    add_layer_args(parser)
    parser.add_argument("--out-dir", type=Path, default=DEFAULT_OUT)
    parser.add_argument(
        "--localisation-catalog", type=Path, default=DEFAULT_LOCALISATION_CATALOG
    )
    parser.add_argument("--min-trees", type=int, default=1)
    parser.add_argument("--min-focuses", type=int, default=1)
    args = parser.parse_args()

    catalog = build_catalog(args)
    paths = write_static_store(
        args.out_dir,
        game_id=GAME_ID,
        catalog_name="focus_defs_catalog.json",
        catalog=catalog,
        snapshot_schema="focus_defs_static.static_snapshot.v1",
        index_schema="focus_defs_static.static_index.v1",
        manifest_role="focus_defs_static_manifest",
        payload_role="focus_defs_static_payload",
        snapshot_extra={
            "counts": catalog["meta"],
            "artifact_sha256": catalog["artifact_sha256"],
        },
    )
    store_verify = verify_store(paths)
    passed = bool(
        catalog["meta"]["tree_count"] >= args.min_trees
        and catalog["meta"]["focus_count"] >= args.min_focuses
        and store_verify["passed"]
    )
    report = {
        "schema": "focus_defs_static.static_gate_report.v1",
        "passed": passed,
        "paths": {key: repo_relative(path) for key, path in paths.items()},
        "counts": catalog["meta"],
        "artifact_sha256": catalog["artifact_sha256"],
        "store_verify": store_verify,
    }
    report_path = args.out_dir / "focus_defs_static_gate_report.json"
    report_path.write_bytes(pretty_json(report))
    print(json.dumps(report, ensure_ascii=False, sort_keys=True, indent=2))
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
