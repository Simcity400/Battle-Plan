#!/usr/bin/env python3
"""Parse effective technology-sharing definitions with source provenance.

The definition ``id`` is retained as a possible string-side join candidate,
but no numeric mapping is manufactured: build-870ebf static RE of
``FUN_1424A6500``/``FUN_1424A65A0`` proves the engine token is a
load-order-dependent intern-table ordinal, not the string's FNV lookup hash.
Source order remains part of the artifact so alike definitions originating at
different load-order positions do not become indistinguishable.
"""

from __future__ import annotations

import argparse
import json
import math
import sys
from pathlib import Path
from typing import Any

import resolver as R
from paradox_parser import Node, parse, tokenize
from static_artifact_common import node_to_data, source_inventory, validate_clausewitz_text


SOURCE_DIR = "common/technology_sharing"
WRAPPER = "technology_sharing_group"
REQUIRED_SCALARS = (
    "id",
    "name",
    "desc",
    "picture",
    "research_sharing_per_country_bonus",
)


def validate_implicit_active_modset(mods: list[str] | None) -> None:
    """Reject malformed implicit active-mod rows before resolver filtering.

    The shared resolver intentionally treats a missing manifest as vanilla
    only, but its manifest loader skips a ``kind=mod`` row whose descriptor is
    absent.  For an authoritative static catalog that omission is not a warning:
    strict layer-count checks cannot see a request that was discarded first.
    """

    if mods is not None:
        return
    manifest = Path(R._DEFAULT_MODSET)
    if not manifest.is_file():
        return
    try:
        data = json.loads(manifest.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise RuntimeError(f"failed to read active modset manifest {manifest.name}: {exc}") from exc
    load_order = data.get("load_order") if isinstance(data, dict) else None
    if not isinstance(load_order, list):
        raise RuntimeError("active modset manifest requires a load_order list")
    if not load_order:
        raise RuntimeError("active modset load_order must include exactly one vanilla row")
    vanilla_rows = 0
    for index, entry in enumerate(load_order):
        if not isinstance(entry, dict):
            raise RuntimeError(f"active modset load_order row {index} is malformed")
        kind = entry.get("kind")
        if kind not in {"vanilla", "mod"}:
            raise RuntimeError(
                f"active modset load_order row {index} has invalid or missing kind"
            )
        load_index = entry.get("load_index")
        if (
            isinstance(load_index, bool)
            or not isinstance(load_index, int)
            or load_index != index
        ):
            raise RuntimeError(
                f"active modset load_order row {index} has non-contiguous load_index"
            )
        descriptor = entry.get("descriptor")
        if kind == "vanilla":
            vanilla_rows += 1
            if index != 0 or (descriptor is not None and descriptor != ""):
                raise RuntimeError(
                    "active modset vanilla row must be first and must not name a descriptor"
                )
            continue
        if not isinstance(descriptor, str) or not descriptor.strip():
            raise RuntimeError(
                f"active modset load_order mod row {index} requires a descriptor"
            )
    if vanilla_rows != 1:
        raise RuntimeError("active modset load_order must include exactly one vanilla row")
    mod_count = data.get("mod_count")
    actual_mod_count = len(load_order) - 1
    if mod_count is not None and (
        isinstance(mod_count, bool)
        or not isinstance(mod_count, int)
        or mod_count != actual_mod_count
    ):
        raise RuntimeError("active modset mod_count does not match load_order")


def _validate_categories_token_grammar(text: str, source: str) -> None:
    """Reject category-block tokens the tolerant parser would discard."""

    tokens = tokenize(text)
    depth = 0
    for index, (kind, value) in enumerate(tokens):
        if kind == "lbrace":
            depth += 1
            continue
        if kind == "rbrace":
            depth -= 1
            continue
        # Direct fields of a top-level technology_sharing_group are at depth
        # one.  A legitimate category whose literal ID is "categories" is at
        # depth two and must not be mistaken for another field declaration.
        if depth != 1:
            continue
        if kind not in {"bare", "str"} or value.lower() != "categories":
            continue
        # A scalar value literally named "categories" is not a field key.
        if index > 0 and tokens[index - 1][0] == "eq":
            continue
        if (
            index + 2 >= len(tokens)
            or tokens[index + 1][0] != "eq"
            or tokens[index + 2][0] != "lbrace"
        ):
            raise ValueError(f"{source}: categories must use categories = {{ ... }}")
        category_depth = 1
        cursor = index + 3
        while cursor < len(tokens) and category_depth:
            token_kind, token_value = tokens[cursor]
            if token_kind == "lbrace":
                raise ValueError(
                    f"{source}: categories must contain only anonymous scalar tokens"
                )
            if token_kind == "rbrace":
                category_depth -= 1
                cursor += 1
                continue
            if token_kind not in {"bare", "str"}:
                raise ValueError(
                    f"{source}: categories must contain only anonymous scalar tokens"
                )
            if any(operator in token_value for operator in ("<", ">", "!")):
                raise ValueError(
                    f"{source}: categories contain an invalid operator-like token"
                )
            cursor += 1


def _source_provenance(
    layers: list[Any],
    path: Path,
    rel: str,
    *,
    file_order: int,
    block_order: int,
    catalog_order: int,
) -> dict[str, Any]:
    layer_ordinal = R._layer_prio_fn(layers)(path)
    layer = layers[layer_ordinal]
    meta = dict(getattr(layer, "meta", {}) or {})
    # Layer position is the authority when older resolver layers do not carry a
    # kind.  A mod is allowed to call itself "vanilla"; a display label must
    # never be treated as a source-type discriminator.
    kind = meta.get("kind") or ("vanilla" if layer_ordinal == 0 else "mod")
    dlc_name = meta.get("dlc_name") if kind in {"dlc", "integrated_dlc"} else None
    mod_name = layer.label if kind == "mod" else None
    return {
        "layer": layer.label,
        "layer_kind": kind,
        "layer_ordinal": layer_ordinal,
        "mod_name": mod_name,
        "dlc_name": dlc_name,
        "file": rel,
        "file_order": file_order,
        "block_order": block_order,
        "catalog_order": catalog_order,
    }


def _one_scalar(body: Node, field: str, source: str) -> str:
    values = body.get_all(field)
    if len(values) != 1 or not isinstance(values[0], str) or not values[0].strip():
        raise ValueError(
            f"{source}: {WRAPPER} requires exactly one non-empty scalar {field!r}"
        )
    return values[0]


def _bonus(raw: str, source: str) -> float:
    try:
        value = float(raw)
    except ValueError as exc:
        raise ValueError(
            f"{source}: research_sharing_per_country_bonus is not numeric: {raw!r}"
        ) from exc
    if not math.isfinite(value):
        raise ValueError(
            f"{source}: research_sharing_per_country_bonus must be finite: {raw!r}"
        )
    return value


def _definition(
    body: Node,
    provenance: dict[str, Any],
) -> dict[str, Any]:
    source = f"{provenance['file']} block {provenance['block_order']}"
    values = {field: _one_scalar(body, field, source) for field in REQUIRED_SCALARS}
    categories_present = bool(body.get_all("categories"))
    if len(body.get_all("categories")) > 1:
        raise ValueError(f"{source}: duplicate categories fields are ambiguous")
    categories_node = body.get("categories")
    if categories_present and not isinstance(categories_node, Node):
        raise ValueError(f"{source}: categories must be a block")
    categories: list[str] = []
    if categories_present:
        for key, value in categories_node.entries:
            if key != "" or not isinstance(value, str) or not value.strip():
                raise ValueError(
                    f"{source}: categories must contain only anonymous scalar tokens"
                )
            categories.append(value)
    if len(categories) != len(set(categories)):
        raise ValueError(f"{source}: duplicate categories are ambiguous")

    if not categories_present:
        category_scope = "unrestricted_by_absent_categories"
    elif categories:
        category_scope = "explicit_category_list"
    else:
        # The installed corpus contains an explicit empty block.  Preserve it,
        # but do not claim engine semantics without runtime/static RE proof.
        category_scope = "explicit_empty_categories_semantics_unproven"

    row = {
        "id": values["id"],
        "runtime_token_candidate": values["id"],
        "runtime_hash_candidate": None,
        "runtime_join_status": (
            "definition_id_candidate_only_runtime_token_is_intern_ordinal_mapping_unproven"
        ),
        "name_loc_key": values["name"],
        "description_loc_key": values["desc"],
        "picture": values["picture"],
        "base_research_sharing_per_country_bonus": _bonus(
            values["research_sharing_per_country_bonus"], source
        ),
        "base_research_sharing_per_country_bonus_raw": values[
            "research_sharing_per_country_bonus"
        ],
        "categories": categories,
        "categories_present": categories_present,
        "category_scope": category_scope,
        "source_layer": provenance["layer"],
        "source_file": provenance["file"],
        "source_provenance": provenance,
        "raw": node_to_data(body),
    }
    return row


def parse_technology_sharing(layers: list[Any], *, strict: bool = True) -> dict[str, Any]:
    files = R.ordered_files(layers, SOURCE_DIR, (".txt",))
    definitions: list[dict[str, Any]] = []
    warnings: list[str] = []
    parsed_files = 0

    for file_order, (rel, path) in enumerate(files):
        try:
            text = R.read_text(path)
            validate_clausewitz_text(text, rel)
            _validate_categories_token_grammar(text, rel)
            root = parse(text)
            block_order = 0
            for key, value in root.entries:
                if key != WRAPPER or not isinstance(value, Node):
                    raise ValueError(
                        f"{rel}: unexpected top-level entry {key!r}; expected {WRAPPER} block"
                    )
                provenance = _source_provenance(
                    layers,
                    path,
                    rel,
                    file_order=file_order,
                    block_order=block_order,
                    catalog_order=len(definitions),
                )
                definitions.append(_definition(value, provenance))
                block_order += 1
            parsed_files += 1
        except Exception as exc:
            if strict:
                raise RuntimeError(f"failed to parse technology-sharing file {rel}: {exc}") from exc
            warnings.append(f"{rel}: {exc}")
            print(f"[technology_sharing] WARN {rel}: {exc}", file=sys.stderr)

    by_id: dict[str, dict[str, Any]] = {}
    for row in definitions:
        token = row["id"]
        if token in by_id:
            first = by_id[token]
            raise RuntimeError(
                "duplicate technology-sharing id "
                f"{token!r}: {first['source_file']} and {row['source_file']}"
            )
        by_id[token] = row

    return {
        "meta": {
            "schema": "technology_sharing.catalog.v1",
            "definition_count": len(definitions),
            "source_files_total": len(files),
            "source_files_parsed": parsed_files,
            "categories_absent_unrestricted_count": sum(
                row["category_scope"] == "unrestricted_by_absent_categories"
                for row in definitions
            ),
            "categories_explicit_nonempty_count": sum(
                row["category_scope"] == "explicit_category_list" for row in definitions
            ),
            "categories_explicit_empty_unproven_count": sum(
                row["category_scope"] == "explicit_empty_categories_semantics_unproven"
                for row in definitions
            ),
            "runtime_token_proof": "engine_runtime_intern_ordinal_not_string_hash",
            "runtime_hash_proof": "fnv1a_only_indexes_intern_table_not_runtime_token",
            "runtime_token_static_re_evidence": {
                "build_sha256": "870EBF1F615262FACCA48A3B947775C41F0A5338F3068C40958209F5C41C65AD",
                "modify_effect_parser": "FUN_140323D40",
                "string_intern_entry": "FUN_1424A6500",
                "intern_lookup_insert": "FUN_1424A65A0",
                "runtime_group_lookup": "FUN_140EBFF80",
            },
            "warnings": warnings,
        },
        "source_files": source_inventory(files),
        "definitions": definitions,
        "definitions_by_id": dict(sorted(by_id.items())),
    }


def build(
    vanilla: str | None = None,
    mods: list[str] | None = None,
    userdata: str | Path | None = None,
) -> dict[str, Any]:
    validate_implicit_active_modset(mods)
    return parse_technology_sharing(
        R.build_layers(
            vanilla=vanilla,
            mods=mods,
            userdata=userdata,
            # A missing explicitly requested mod or an unresolved entry in the
            # implicit active_modset would make the catalog incomplete.  This
            # standalone producer is an extraction authority, so it must fail
            # closed in both cases.
            strict_mods=True,
        ),
        strict=True,
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--vanilla", default=None)
    parser.add_argument("--mods", default=None, help="comma-separated .mod descriptors")
    parser.add_argument("--userdata", default=None)
    parser.add_argument("--out", type=Path, default=None)
    args = parser.parse_args()
    parsed = build(
        vanilla=args.vanilla,
        mods=[part.strip() for part in args.mods.split(",") if part.strip()]
        if args.mods is not None
        else None,
        userdata=args.userdata,
    )
    rendered = json.dumps(parsed, ensure_ascii=False, sort_keys=True, indent=2)
    if args.out:
        args.out.write_text(rendered + "\n", encoding="utf-8")
    else:
        print(rendered)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
