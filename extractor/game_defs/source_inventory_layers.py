"""Construct canonical definition layers from a validated live source inventory."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import live_binding_resolver as LR
import resolver as R


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


def definition_layers(
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
        raw = json.loads(Path(source_inventory_path).read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        raise RuntimeError(f"failed to read live source inventory: {exc}") from exc
    inventory = LR.validate_source_inventory_document(raw)
    if not base_layers:
        raise RuntimeError("resolver returned no base definition layer")
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
