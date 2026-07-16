#!/usr/bin/env python3
"""Build and verify the technology-sharing static-definition artifact."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any

_HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(_HERE))

import parse_localisation  # noqa: E402
import parse_technology_sharing  # noqa: E402
import resolver as R  # noqa: E402
import live_binding_resolver as LR  # noqa: E402
import source_inventory_layers as SIL  # noqa: E402
from static_artifact_common import (  # noqa: E402
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


GAME_ID = "technology_sharing_static"
DEFAULT_OUT = _HERE / "out" / "technology_sharing_static"
DEFAULT_LOCALISATION_CATALOG = (
    _HERE / "out" / "localisation_gate" / "static_localisation_join_r3" / "localisation_catalog.json"
)
SOURCE_NAMESPACE_PROOF_SCHEMA = "technology_sharing_static.source_namespace_proof.v1"


def _safe_namespace_root(vanilla: Path, kind: str, layer_id: str) -> Path:
    if Path(layer_id).name != layer_id or "/" in layer_id or "\\" in layer_id:
        raise RuntimeError(f"unsafe source inventory namespace id: {layer_id!r}")
    parent = "integrated_dlc" if kind == "integrated_dlc" else "dlc"
    root = (vanilla / parent / layer_id).resolve()
    try:
        root.relative_to((vanilla / parent).resolve())
    except ValueError as exc:
        raise RuntimeError(f"source inventory namespace escapes {parent}: {layer_id!r}") from exc
    if not root.is_dir():
        raise RuntimeError(f"source inventory namespace root is missing: {parent}/{layer_id}")
    return root


def _definition_layers(
    base_layers: list[Any], source_inventory_path: Path | None
) -> tuple[list[Any], dict[str, Any]]:
    if source_inventory_path is None:
        return base_layers, {
            "schema": SOURCE_NAMESPACE_PROOF_SCHEMA,
            "status": "standalone_unbound_no_live_source_inventory",
            "source_inventory_sha256": None,
            "selected_namespace_labels": [layer.label for layer in base_layers],
            "excluded_asset_only_namespace_labels": [],
        }
    try:
        raw = json.loads(source_inventory_path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        raise RuntimeError(f"failed to read live source inventory: {exc}") from exc
    inventory = LR.validate_source_inventory_document(raw)
    vanilla = Path(base_layers[0].root).resolve()
    mod_layers = iter(base_layers[1:])
    selected: list[Any] = []
    excluded_asset_only: list[str] = []
    for namespace in inventory["namespaces"]:
        binding = namespace["binding"]
        if binding == "asset_only":
            excluded_asset_only.append(namespace["label"])
            continue
        if binding == "base":
            if selected or namespace["label"] != base_layers[0].label:
                raise RuntimeError("live source inventory base namespace does not match resolver base")
            layer = base_layers[0]
        elif binding in {"integrated", "active_product"}:
            root = _safe_namespace_root(vanilla, namespace["kind"], namespace["id"])
            layer = R._ex.Layer(
                namespace["label"],
                root,
                namespace["replace_paths"],
                {
                    "kind": namespace["kind"],
                    "dlc_name": namespace["id"],
                    "binding": binding,
                },
            )
        elif binding == "active_mod":
            try:
                layer = next(mod_layers)
            except StopIteration as exc:
                raise RuntimeError("live source inventory has more active mods than resolver layers") from exc
            if (
                layer.label != namespace["label"]
                or list(getattr(layer, "replace_paths", []) or []) != namespace["replace_paths"]
            ):
                raise RuntimeError("live source inventory mod order does not match resolver layers")
        else:
            raise RuntimeError(f"unsupported source namespace binding: {binding!r}")
        selected.append(layer)
    try:
        next(mod_layers)
    except StopIteration:
        pass
    else:
        raise RuntimeError("resolver has more active mods than live source inventory")
    return selected, {
        "schema": SOURCE_NAMESPACE_PROOF_SCHEMA,
        "status": "bound_to_live_source_inventory",
        "source_inventory_sha256": inventory["inventory_sha256"],
        "selected_namespace_labels": [layer.label for layer in selected],
        "excluded_asset_only_namespace_labels": excluded_asset_only,
    }


def _effective_localisation_inventory(layers: list[Any]) -> list[dict[str, Any]]:
    file_map = R._ex.resolve_loc(layers, "english")
    return source_inventory(list(file_map.items()))


def _layer_binding(layers: list[Any]) -> list[dict[str, Any]]:
    return [
        {
            "label": layer.label,
            "root_name": Path(layer.root).name,
            "replace_path": list(getattr(layer, "replace_paths", []) or []),
        }
        for layer in layers
    ]


def _catalog_layer_binding(data: dict[str, Any]) -> list[dict[str, Any]]:
    rows = data.get("layers")
    if not isinstance(rows, list) or not rows:
        raise RuntimeError("localisation catalog is missing layer provenance")
    normalized = []
    for index, row in enumerate(rows):
        if not isinstance(row, dict):
            raise RuntimeError(f"localisation catalog layer {index} is malformed")
        label = row.get("label")
        root_name = row.get("root_name", row.get("root"))
        replace_path = row.get("replace_path")
        if (
            not isinstance(label, str)
            or not label
            or not isinstance(root_name, str)
            or not root_name
            or not isinstance(replace_path, list)
            or not all(isinstance(item, str) for item in replace_path)
        ):
            raise RuntimeError(f"localisation catalog layer {index} is malformed")
        normalized.append(
            {"label": label, "root_name": Path(root_name).name, "replace_path": replace_path}
        )
    return normalized


def _verify_external_localisation_catalog(
    data: dict[str, Any], layers: list[Any]
) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    if data.get("schema") != "static_localisation_join_r3.static_artifact.v1":
        raise RuntimeError("localisation catalog schema is unsupported")
    claimed = data.get("artifact_sha256")
    unhashed = dict(data)
    unhashed.pop("artifact_sha256", None)
    if not isinstance(claimed, str) or claimed != sha256_bytes(canonical_json(unhashed)):
        raise RuntimeError("localisation catalog artifact self-hash mismatch")
    meta = data.get("meta")
    if (
        not isinstance(meta, dict)
        or not isinstance(meta.get("files_read"), int)
        or not isinstance(meta.get("files_total"), int)
        or not (0 <= meta["files_read"] <= meta["files_total"])
        or not isinstance(meta.get("key_count"), int)
        or meta.get("key_count") != len(data.get("keys", {}))
    ):
        raise RuntimeError("localisation catalog parse accounting is incomplete or inconsistent")

    current_layers = _layer_binding(layers)
    if _catalog_layer_binding(data) != current_layers:
        raise RuntimeError("localisation catalog layers do not match current effective layers")
    expected_inventory = _effective_localisation_inventory(layers)
    source_files = data.get("source_files")
    actual_inventory = (
        source_files.get("localisation_files") if isinstance(source_files, dict) else None
    )
    if actual_inventory != expected_inventory:
        raise RuntimeError(
            "localisation catalog source inventory does not match current effective files"
        )

    # `files_read` counts files that yielded at least one key, so legitimate
    # empty files mean it need not equal `files_total`.  Rebuild the effective
    # semantic projection instead of treating that counter as completeness
    # authority.  This prevents a self-hashed catalog from carrying the right
    # source inventory while silently omitting keys from those source bytes.
    rebuilt = parse_localisation.parse_localisation(layers)
    rebuilt_meta = rebuilt["meta"]
    for field in ("files_read", "files_total", "key_count"):
        if meta.get(field) != rebuilt_meta.get(field):
            raise RuntimeError(
                f"localisation catalog {field} does not match current effective files"
            )

    def names(keys: Any) -> dict[str, str | None]:
        if not isinstance(keys, dict):
            raise RuntimeError("localisation catalog keys table is malformed")
        return {
            key: row.get("name") if isinstance(row, dict) else None
            for key, row in keys.items()
        }

    if names(data.get("keys")) != names(rebuilt.get("keys")):
        raise RuntimeError(
            "localisation catalog key contents do not match current effective files"
        )
    return current_layers, expected_inventory


def _loc_catalog(path: Path, layers: list[Any]) -> tuple[dict[str, Any], dict[str, Any]]:
    if path.is_file():
        data = load_localisation_catalog(path)
        layer_binding, inventory = _verify_external_localisation_catalog(data, layers)
        return data["keys"], {
            "source": "static_localisation_join_r3",
            "path": repo_relative(path),
            "key_count": data.get("meta", {}).get("key_count", len(data["keys"])),
            "sha256": sha256_bytes(path.read_bytes()),
            "compatibility": "verified_current_effective_layers_and_files",
            "layer_binding_sha256": sha256_bytes(canonical_json(layer_binding)),
            "localisation_source_inventory_sha256": sha256_bytes(canonical_json(inventory)),
            "localisation_source_file_count": len(inventory),
        }
    built = parse_localisation.parse_localisation(layers)
    layer_binding = _layer_binding(layers)
    inventory = _effective_localisation_inventory(layers)
    return built["keys"], {
        "source": "built_in_gate_missing_r3_catalog",
        "path": None,
        "key_count": built["meta"]["key_count"],
        "sha256": sha256_bytes(canonical_json(built)),
        "compatibility": "verified_current_effective_layers_and_files",
        "layer_binding_sha256": sha256_bytes(canonical_json(layer_binding)),
        "localisation_source_inventory_sha256": sha256_bytes(canonical_json(inventory)),
        "localisation_source_file_count": len(inventory),
    }


def _localised_value(keys: dict[str, Any], key: str) -> str | None:
    row = keys.get(key)
    if not isinstance(row, dict):
        return None
    value = row.get("name")
    return value if isinstance(value, str) else None


def _enrich(row: dict[str, Any], loc_keys: dict[str, Any]) -> dict[str, Any]:
    out = dict(row)
    name = _localised_value(loc_keys, row["name_loc_key"])
    description = _localised_value(loc_keys, row["description_loc_key"])
    out["localised_name"] = name
    out["localised_description"] = description
    out["name_localisation_status"] = "resolved" if name is not None else "missing"
    out["description_localisation_status"] = (
        "resolved" if description is not None else "missing"
    )
    return out


def build_catalog(args: argparse.Namespace) -> dict[str, Any]:
    requested_mods = mods_from_arg(args.mods)
    parse_technology_sharing.validate_implicit_active_modset(requested_mods)
    localisation_layers = R.build_layers(
        vanilla=args.vanilla,
        mods=requested_mods,
        userdata=args.userdata,
        # Gate inputs are authoritative whether --mods is explicit or sourced
        # from active_modset.json.  Warning-and-skip would certify a truncated
        # effective catalog, so this gate never permits it.
        strict_mods=True,
    )
    layers, source_namespace_proof = SIL.definition_layers(
        localisation_layers, getattr(args, "source_inventory", None)
    )
    loc_keys, loc_source = _loc_catalog(args.localisation_catalog, layers)
    parsed = parse_technology_sharing.parse_technology_sharing(layers, strict=True)
    definitions = [_enrich(row, loc_keys) for row in parsed["definitions"]]
    catalog = {
        "schema": "technology_sharing_static.static_artifact.v1",
        "artifact_kind": "deterministic_static_replay_enrichment",
        "scope": {
            "closed_rows": [
                "technology-sharing definition id, picture, base per-country bonus, and category declaration",
                "name/description localisation when the localisation catalog contains each key",
                "effective source file, layer, mod/DLC kind, and load-order provenance",
            ],
            "not_closed_rows": [
                "live technology-sharing group membership or accumulated runtime bonus",
                "runtime intern-ordinal -> definition id mapping until a safe engine lookup or direct template string path is proven",
                "meaning of an explicitly empty categories block until engine/runtime proof exists",
            ],
        },
        "join_keys": {
            "definition_token_candidate": "definitions[*].runtime_token_candidate",
            "runtime_hash_candidate": None,
            "compatibility_status": (
                "definition id is a string candidate only; the numeric runtime token is a load-order-dependent intern ordinal, not a string hash, and its mapping is unproven"
            ),
        },
        "layers": layer_summary(layers),
        "localisation_source": loc_source,
        "source_namespace_proof": source_namespace_proof,
        "meta": parsed["meta"],
        "source_files": parsed["source_files"],
        "definitions": definitions,
        "definitions_by_id": {row["id"]: row for row in definitions},
    }
    catalog["artifact_sha256"] = sha256_bytes(canonical_json(catalog))
    return catalog


def catalog_gate_checks(
    catalog: dict[str, Any],
    *,
    min_definitions: int = 1,
    store_verify_result: dict[str, Any] | None = None,
) -> dict[str, bool]:
    definitions = catalog.get("definitions")
    meta = catalog.get("meta") if isinstance(catalog.get("meta"), dict) else {}
    rows = definitions if isinstance(definitions, list) else []
    return {
        "min_definitions": len(rows) >= min_definitions,
        "all_files_parsed": meta.get("source_files_parsed") == meta.get("source_files_total"),
        "no_parse_warnings": not meta.get("warnings"),
        "unique_definition_ids": len({row.get("id") for row in rows}) == len(rows),
        "source_order_contiguous": [
            row.get("source_provenance", {}).get("catalog_order") for row in rows
        ]
        == list(range(len(rows))),
        "runtime_hash_not_fabricated": all(
            row.get("runtime_hash_candidate") is None
            and row.get("runtime_join_status")
            == "definition_id_candidate_only_runtime_token_is_intern_ordinal_mapping_unproven"
            for row in rows
        ),
        "localisation_compatible": catalog.get("localisation_source", {}).get("compatibility")
        == "verified_current_effective_layers_and_files",
        "store_verified": store_verify_result is None or store_verify_result.get("passed") is True,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    add_layer_args(parser)
    parser.add_argument("--out-dir", type=Path, default=DEFAULT_OUT)
    parser.add_argument("--localisation-catalog", type=Path, default=DEFAULT_LOCALISATION_CATALOG)
    parser.add_argument("--min-definitions", type=int, default=1)
    args = parser.parse_args()

    catalog = build_catalog(args)
    paths = write_static_store(
        args.out_dir,
        game_id=GAME_ID,
        catalog_name="technology_sharing_catalog.json",
        catalog=catalog,
        snapshot_schema="technology_sharing_static.static_snapshot.v1",
        index_schema="technology_sharing_static.static_index.v1",
        manifest_role="technology_sharing_static_manifest",
        payload_role="technology_sharing_static_payload",
        snapshot_extra={"counts": catalog["meta"], "artifact_sha256": catalog["artifact_sha256"]},
    )
    store_verify = verify_store(paths)
    checks = catalog_gate_checks(
        catalog,
        min_definitions=args.min_definitions,
        store_verify_result=store_verify,
    )
    report = {
        "schema": "technology_sharing_static.static_gate_report.v1",
        "passed": all(checks.values()),
        "checks": checks,
        "counts": catalog["meta"],
        "artifact_sha256": catalog["artifact_sha256"],
        "store_verify": store_verify,
        "paths": {key: repo_relative(path) for key, path in paths.items()},
    }
    report_path = args.out_dir / "technology_sharing_gate_report.json"
    report_path.write_bytes(pretty_json(report))
    print(json.dumps(report, ensure_ascii=False, sort_keys=True, indent=2))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
