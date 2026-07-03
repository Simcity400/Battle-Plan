#!/usr/bin/env python3
"""Ship hull and ship-module static definition parser.

Domain 29 emits a ship ``definition`` string when available; this parser builds
the static definition-side catalog keyed by those equipment tokens. It reads
effective mod-over-vanilla ship hull files plus ship module files only; broader
equipment catalogs remain owned by ``parse_catalogs.py`` and domain 59.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path
from typing import Any

import resolver as R
from paradox_parser import Node, parse
from static_artifact_common import (
    atoms,
    boolish,
    first_scalar,
    node_to_data,
    put_present,
    scalar,
    source_inventory,
    validate_clausewitz_text,
)


SHIP_HULL_PREFIX = "ship_hull_"
SHIP_TYPES = {
    "screen_ship",
    "capital_ship",
    "carrier",
    "submarine",
}
HULL_FIXED_KEYS = {
    "active",
    "alias",
    "archetype",
    "build_cost_ic",
    "can_be_produced",
    "can_license",
    "default_modules",
    "group_by",
    "interface_category",
    "is_archetype",
    "is_buildable",
    "module_count_limit",
    "module_slots",
    "name",
    "parent",
    "picture",
    "priority",
    "resources",
    "sprite",
    "type",
    "upgrades",
    "visual_level",
    "year",
}
MODULE_FIXED_KEYS = {
    "abbreviation",
    "add_average_stats",
    "add_equipment_type",
    "add_stats",
    "can_convert_from",
    "category",
    "critical_parts",
    "dismantle_cost_ic",
    "forbid_equipment_type",
    "gui_category",
    "manpower",
    "module_type_icon",
    "multiply_stats",
    "parent",
    "sfx",
    "xp_cost",
}


def _source_label(layers, abspath) -> str:
    return layers[R._layer_prio_fn(layers)(abspath)].label


def _read(rel: str, path: Path, *, strict: bool, warnings: list[str]) -> Node | None:
    try:
        text = R.read_text(path)
        validate_clausewitz_text(text, rel)
        return parse(text)
    except Exception as e:
        msg = f"{rel}: {e}"
        if strict:
            raise RuntimeError(f"failed to parse ship definition file {msg}") from e
        warnings.append(msg)
        print(f"[ship_definitions] WARN {msg}", file=sys.stderr)
        return None


def _equipment_files(layers) -> list[tuple[str, Path]]:
    return [
        (rel, path)
        for rel, path in R.ordered_files(layers, "common/units/equipment", (".txt",))
        if Path(rel).parent.as_posix() == "common/units/equipment"
    ]


def _module_files(layers) -> list[tuple[str, Path]]:
    return R.ordered_files(layers, "common/units/equipment/modules", (".txt",))


def _blocks(root: Node, wrapper: str) -> list[Node]:
    return [
        value
        for key, value in root.entries
        if key.lower() == wrapper.lower() and isinstance(value, Node)
    ]


def _stats(body: Node, fixed_keys: set[str]) -> dict[str, Any]:
    return {
        key: node_to_data(value)
        for key, value in body.entries
        if key and key.lower() not in fixed_keys
    }


def _slot_catalog(node: Any) -> dict[str, Any] | None:
    if isinstance(node, str):
        return {"inherits": node == "inherit", "raw": node, "slots": []}
    if not isinstance(node, Node):
        return None
    slots = []
    for slot_id, body in node.entries:
        if not slot_id or not isinstance(body, Node):
            continue
        slot = {"id": slot_id, "raw": node_to_data(body)}
        put_present(slot, "required", boolish(first_scalar(body, "required")))
        if body.get("allowed_module_categories") is not None:
            slot["allowed_module_categories"] = atoms(body.get("allowed_module_categories"))
        slots.append(slot)
    slots.sort(key=lambda row: row["id"])
    return {"inherits": False, "raw": node_to_data(node), "slots": slots}


def _default_modules(node: Any) -> dict[str, str] | str | None:
    if isinstance(node, str):
        return node
    if not isinstance(node, Node):
        return None
    out = {
        key: value
        for key, value in node.entries
        if key and isinstance(value, str)
    }
    return dict(sorted(out.items()))


def _repeated_blocks(body: Node, key_name: str) -> list[Any] | None:
    wanted = key_name.lower()
    blocks = [
        node_to_data(value)
        for key, value in body.entries
        if key.lower() == wanted and isinstance(value, Node)
    ]
    return blocks if body.get(key_name) is not None else None


def _type_field(body: Node) -> str | list[str] | None:
    value = body.get("type")
    listed = atoms(value)
    if listed:
        return listed
    return scalar(value)


def _field_present(node: Node, field: str) -> bool:
    fl = field.lower()
    return any(k.lower() == fl for k, _v in node.entries if k)


def _ship_loc_key(token: str, body: Node) -> tuple[str, str]:
    """Return (loc_key, loc_rule), fail-safe on ambiguous names.

    A duplicated or non-scalar ``name`` is NOT silently first-valued: it falls
    back to the id token with a visible ``id`` rule so the ambiguity is not
    masked as a resolved name-based key.
    """
    names = body.get_all("name") if _field_present(body, "name") else []
    if len(names) == 1:
        name = scalar(names[0])
        if name:
            return name, "name"
    return token, "id"


def _ship_hull_record(token: str, body: Node, rel: str, source_layer: str) -> dict[str, Any]:
    loc_key, loc_rule = _ship_loc_key(token, body)
    row = {
        "id": token,
        "loc_key": loc_key,
        "loc_rule": loc_rule,
        "source_layer": source_layer,
        "source_file": rel,
    }
    put_present(row, "stats", _stats(body, HULL_FIXED_KEYS) or None)
    put_present(row, "year", first_scalar(body, "year"))
    put_present(row, "is_archetype", boolish(first_scalar(body, "is_archetype")))
    put_present(row, "is_buildable", boolish(first_scalar(body, "is_buildable")))
    put_present(row, "archetype", first_scalar(body, "archetype") or first_scalar(body, "parent"))
    put_present(row, "type", _type_field(body))
    put_present(row, "interface_category", first_scalar(body, "interface_category"))
    put_present(row, "alias", first_scalar(body, "alias"))
    put_present(row, "priority", first_scalar(body, "priority"))
    if body.get("upgrades") is not None:
        row["upgrades"] = atoms(body.get("upgrades"))
    put_present(row, "module_slots", _slot_catalog(body.get("module_slots")))
    put_present(row, "default_modules", _default_modules(body.get("default_modules")))
    put_present(row, "module_count_limit", _repeated_blocks(body, "module_count_limit"))
    if body.get("resources") is not None:
        row["resources"] = node_to_data(body.get("resources"))
    if body.get("can_be_produced") is not None:
        row["can_be_produced"] = node_to_data(body.get("can_be_produced"))
    return row


def _ship_module_record(token: str, body: Node, rel: str, source_layer: str) -> dict[str, Any]:
    loc_key, loc_rule = _ship_loc_key(token, body)
    row = {
        "id": token,
        "loc_key": loc_key,
        "loc_rule": loc_rule,
        "source_layer": source_layer,
        "source_file": rel,
    }
    put_present(row, "extra_fields", _stats(body, MODULE_FIXED_KEYS) or None)
    put_present(row, "category", first_scalar(body, "category"))
    put_present(row, "gui_category", first_scalar(body, "gui_category"))
    put_present(row, "abbreviation", first_scalar(body, "abbreviation"))
    put_present(row, "parent", first_scalar(body, "parent"))
    put_present(row, "sfx", first_scalar(body, "sfx"))
    put_present(row, "module_type_icon", first_scalar(body, "module_type_icon"))
    if body.get("add_stats") is not None:
        row["add_stats"] = node_to_data(body.get("add_stats"))
    if body.get("multiply_stats") is not None:
        row["multiply_stats"] = node_to_data(body.get("multiply_stats"))
    if body.get("add_average_stats") is not None:
        row["add_average_stats"] = node_to_data(body.get("add_average_stats"))
    if body.get("add_equipment_type") is not None:
        row["add_equipment_type"] = atoms(body.get("add_equipment_type"))
    if body.get("forbid_equipment_type") is not None:
        row["forbid_equipment_type"] = atoms(body.get("forbid_equipment_type"))
    put_present(row, "can_convert_from", _repeated_blocks(body, "can_convert_from"))
    if body.get("critical_parts") is not None:
        row["critical_parts"] = atoms(body.get("critical_parts"))
    return row


def _ship_type_value(body: Node) -> set[str]:
    value = _type_field(body)
    if isinstance(value, list):
        return set(value)
    if isinstance(value, str):
        return {value}
    return set()


def _hull_seed(token: str, body: Node) -> bool:
    archetype = first_scalar(body, "archetype") or first_scalar(body, "parent")
    return (
        token.startswith(SHIP_HULL_PREFIX)
        or (archetype is not None and archetype.startswith(SHIP_HULL_PREFIX))
        or bool(_ship_type_value(body) & SHIP_TYPES)
    )


def _allowed_module_categories(hulls: dict[str, dict[str, Any]]) -> set[str]:
    out: set[str] = set()
    for hull in hulls.values():
        slots = (hull.get("module_slots") or {}).get("slots") or []
        for slot in slots:
            for category in slot.get("allowed_module_categories") or []:
                out.add(category)
    return out


def _module_selected(token: str, body: Node, allowed_categories: set[str]) -> bool:
    category = first_scalar(body, "category")
    gui_category = first_scalar(body, "gui_category")
    # Select by CATEGORY signals only. The prior `"ship" in token.lower()`
    # token-substring fallback over-included non-ship modules whose id merely
    # contained "ship" (e.g. "airship_*"). Category membership in a real
    # ship-hull slot category, or a category that explicitly names ship, are
    # legitimate signals; the module token itself is not.
    return bool(
        (category and category in allowed_categories)
        or (gui_category and gui_category in allowed_categories)
        or (category and "ship" in category.lower())
    )


def parse_ship_definitions(layers, *, strict: bool = True) -> dict[str, Any]:
    warnings: list[str] = []
    hulls: dict[str, dict[str, Any]] = {}
    modules: dict[str, dict[str, Any]] = {}
    hull_collisions: list[dict[str, str]] = []
    module_collisions: list[dict[str, str]] = []
    equipment_files = _equipment_files(layers)
    module_files = _module_files(layers)
    equipment_files_parsed = 0
    module_files_parsed = 0
    skipped_files: list[dict[str, str]] = []
    equipment_entries: list[tuple[str, Node, str, str]] = []
    module_entries: list[tuple[str, Node, str, str]] = []

    for rel, path in equipment_files:
        root = _read(rel, path, strict=strict, warnings=warnings)
        if root is None:
            continue
        equipment_files_parsed += 1
        source_layer = _source_label(layers, path)
        containers = _blocks(root, "equipments")
        if not containers:
            skipped_files.append({"relpath": rel, "reason": "no equipments wrapper"})
            continue
        found = False
        for container in containers:
            for token, body in container.entries:
                if token and isinstance(body, Node):
                    equipment_entries.append((token, body, rel, source_layer))
                    found = True
        if not found:
            skipped_files.append({"relpath": rel, "reason": "equipments wrapper contained no node entries"})

    hull_tokens: set[str] = set()
    changed = True
    while changed:
        changed = False
        for token, body, _rel, _source_layer in equipment_entries:
            archetype = first_scalar(body, "archetype") or first_scalar(body, "parent")
            if token not in hull_tokens and (_hull_seed(token, body) or archetype in hull_tokens):
                hull_tokens.add(token)
                changed = True

    hull_source_files: set[str] = set()
    for token, body, rel, source_layer in equipment_entries:
        if token not in hull_tokens:
            continue
        hull_source_files.add(rel)
        rec = _ship_hull_record(token, body, rel, source_layer)
        if token in hulls and hulls[token]["source_file"] != rel:
            hull_collisions.append(
                {"id": token, "first": hulls[token]["source_file"], "override": rel}
            )
        hulls[token] = rec
    for rel, _path in equipment_files:
        if rel not in hull_source_files and not any(row["relpath"] == rel for row in skipped_files):
            skipped_files.append({"relpath": rel, "reason": "no ship hull equipment tokens selected"})

    for rel, path in module_files:
        root = _read(rel, path, strict=strict, warnings=warnings)
        if root is None:
            continue
        module_files_parsed += 1
        source_layer = _source_label(layers, path)
        containers = _blocks(root, "equipment_modules")
        if not containers:
            skipped_files.append({"relpath": rel, "reason": "no equipment_modules wrapper"})
            continue
        found = False
        for container in containers:
            for token, body in container.entries:
                if not token or not isinstance(body, Node):
                    continue
                if token.lower() in {"limit", "has_dlc"}:
                    continue
                module_entries.append((token, body, rel, source_layer))
                found = True
        if not found:
            skipped_files.append({"relpath": rel, "reason": "equipment_modules wrapper contained no node entries"})

    allowed_categories = _allowed_module_categories(hulls)
    module_source_files: set[str] = set()
    for token, body, rel, source_layer in module_entries:
        if not _module_selected(token, body, allowed_categories):
            continue
        module_source_files.add(rel)
        rec = _ship_module_record(token, body, rel, source_layer)
        if token in modules and modules[token]["source_file"] != rel:
            module_collisions.append(
                {"id": token, "first": modules[token]["source_file"], "override": rel}
            )
        modules[token] = rec
    for rel, _path in module_files:
        if rel not in module_source_files and not any(row["relpath"] == rel for row in skipped_files):
            skipped_files.append({"relpath": rel, "reason": "no ship module tokens selected"})

    return {
        "meta": {
            "ship_hull_count": len(hulls),
            "ship_module_count": len(modules),
            "equipment_files_parsed": equipment_files_parsed,
            "equipment_files_total": len(equipment_files),
            "module_files_parsed": module_files_parsed,
            "module_files_total": len(module_files),
            "ship_hull_files_total": len(hull_source_files),
            "ship_module_files_total": len(module_source_files),
            "skipped_files": sorted(skipped_files, key=lambda row: row["relpath"]),
            "warnings": warnings,
            "id_collisions_last_wins": {
                "ship_hulls": hull_collisions,
                "ship_modules": module_collisions,
            },
        },
        "source_files": {
            "equipment_files_scanned": source_inventory(equipment_files),
            "module_files_scanned": source_inventory(module_files),
            "ship_hull_files": source_inventory([
                (rel, path) for rel, path in equipment_files if rel in hull_source_files
            ]),
            "ship_module_files": source_inventory([
                (rel, path) for rel, path in module_files if rel in module_source_files
            ]),
        },
        "ship_hulls": dict(sorted(hulls.items())),
        "ship_modules": dict(sorted(modules.items())),
    }


def build(vanilla=None, mods=None):
    layers = R.build_layers(vanilla=vanilla, mods=mods)
    return parse_ship_definitions(layers)


if __name__ == "__main__":
    out = build()
    outp = Path(__file__).resolve().parent / "out" / "ship_definitions.json"
    outp.parent.mkdir(exist_ok=True)
    outp.write_text(json.dumps(out, ensure_ascii=False, indent=1), encoding="utf-8")
    m = out["meta"]
    print(
        f"ship_definitions: {m['ship_hull_count']} hulls, "
        f"{m['ship_module_count']} modules -> {outp.name}"
    )
