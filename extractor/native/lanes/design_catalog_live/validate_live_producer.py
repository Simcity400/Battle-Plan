#!/usr/bin/env python3
"""Fail-closed promotion gate for the exact a729 A27 native catalog."""

from __future__ import annotations

import argparse
import json
from collections import defaultdict
from pathlib import Path

from a729_layout import PROGRAM_SHA256, TYPE_TEMPLATE, TYPE_VARIANT

SCHEMA = "extractor.a27.live_heap_catalog.a729.v2"
COMPLETENESS = {"membership", "owner", "definitions", "lifecycle", "ui_occurrences"}
STAT_FIELDS = {
    "speed_cache", "range", "air_attack", "agility", "max_speed",
    "reliability", "air_defence", "thrust", "fuel_consumption", "build_cost_ic",
}


def _load(path: Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def _key(row: dict) -> str:
    return f"{int(row['id'])}:{int(row['type'])}"


def _oracle_utf8(value):
    if not isinstance(value, str):
        return value
    try:
        return value.encode("latin-1").decode("utf-8")
    except (UnicodeEncodeError, UnicodeDecodeError):
        return value


def _require(ok: bool, message: str, errors: list[str]) -> None:
    if not ok:
        errors.append(message)


def _unique(rows, label: str, errors: list[str]) -> dict[str, dict]:
    if not isinstance(rows, list):
        errors.append(f"{label}: missing array")
        return {}
    out = {}
    for row in rows:
        try:
            key = _key(row)
        except (KeyError, TypeError, ValueError):
            errors.append(f"{label}: invalid stable identity")
            continue
        if key in out:
            errors.append(f"{label}: duplicate stable identity {key}")
        out[key] = row
    return out


def _complete_ordinals(values, count: int) -> bool:
    return sorted(values) == list(range(count))


def _validate_variant_definition(key: str, row: dict, saved: dict, errors: list[str]) -> None:
    definition = row.get("definition")
    if not isinstance(definition, dict):
        errors.append(f"variant {key}: definition payload missing")
        return
    _require(definition.get("archetype_token") == saved.get("archetype"), f"variant {key}: archetype mismatch", errors)
    _require(definition.get("display_name") == _oracle_utf8(saved.get("name")), f"variant {key}: display name mismatch", errors)

    modules = definition.get("modules")
    _require(definition.get("modules_complete") is True and isinstance(modules, list), f"variant {key}: modules incomplete", errors)
    if isinstance(modules, list):
        _require(_complete_ordinals([m.get("source_ordinal") for m in modules], len(modules)), f"variant {key}: module ordinals invalid", errors)
        for module in modules:
            _require(isinstance(module.get("slot_handle"), int), f"variant {key}: module slot handle missing", errors)
            _require(isinstance(module.get("module_definition_handle"), int), f"variant {key}: module definition handle missing", errors)
            _require(isinstance(module.get("module_token"), str) and bool(module.get("module_token")), f"variant {key}: module token missing", errors)
        _require(sorted(m.get("module_token") for m in modules) == sorted((saved.get("modules") or {}).values()), f"variant {key}: module token multiset mismatch", errors)

    upgrades = definition.get("upgrades")
    _require(definition.get("upgrades_complete") is True and isinstance(upgrades, list), f"variant {key}: upgrades incomplete", errors)
    if isinstance(upgrades, list):
        _require(_complete_ordinals([u.get("source_ordinal") for u in upgrades], len(upgrades)), f"variant {key}: upgrade ordinals invalid", errors)
        for upgrade in upgrades:
            _require(isinstance(upgrade.get("upgrade_handle"), int), f"variant {key}: upgrade handle missing", errors)
            _require(isinstance(upgrade.get("level"), int), f"variant {key}: upgrade level missing", errors)
            _require(isinstance(upgrade.get("level_raw_u64"), int), f"variant {key}: raw upgrade level missing", errors)
        _require(sorted(u.get("level") for u in upgrades) == sorted((saved.get("upgrades") or {}).values()), f"variant {key}: upgrade level multiset mismatch", errors)

    stats = definition.get("cached_stats")
    _require(definition.get("cached_stats_complete") is True and isinstance(stats, list), f"variant {key}: cached stats incomplete", errors)
    if isinstance(stats, list):
        _require(_complete_ordinals([s.get("source_ordinal") for s in stats], len(stats)), f"variant {key}: cached stat ordinals invalid", errors)
        _require({s.get("stat_id") for s in stats} == STAT_FIELDS, f"variant {key}: cached stat set mismatch", errors)
        _require(all(isinstance(s.get("raw_i32"), int) and isinstance(s.get("scale"), int) and s.get("scale") > 0 for s in stats), f"variant {key}: cached stat raw representation invalid", errors)


def _positions(rows):
    return [{field: row.get(field) for field in ("type", "x", "y")} for row in rows]


def _validate_template_definition(key: str, row: dict, saved: dict, errors: list[str]) -> None:
    definition = row.get("definition")
    if not isinstance(definition, dict):
        errors.append(f"template {key}: definition payload missing")
        return
    _require(definition.get("name") == _oracle_utf8(saved.get("raw_name")), f"template {key}: name mismatch", errors)
    _require(definition.get("priority") == saved.get("priority"), f"template {key}: priority mismatch", errors)
    _require(definition.get("is_locked") == bool(saved.get("is_locked")), f"template {key}: is_locked mismatch", errors)
    _require(definition.get("obsolete") == bool(saved.get("obsolete")), f"template {key}: definition obsolete mismatch", errors)
    _require(isinstance(definition.get("lock_regime_snapshot_raw"), int), f"template {key}: lock-regime raw byte missing", errors)
    for field in ("regiments", "support", "regimental_support"):
        rows = definition.get(field)
        _require(isinstance(rows, list), f"template {key}: {field} payload missing", errors)
        if isinstance(rows, list):
            _require(_positions(rows) == saved.get(field), f"template {key}: {field} mismatch", errors)
            _require(_complete_ordinals([r.get("source_ordinal") for r in rows], len(rows)) or field in ("regiments", "regimental_support"), f"template {key}: {field} ordinals invalid", errors)
            _require(all(isinstance(r.get("subunit_definition_handle"), int) and isinstance(r.get("type"), str) for r in rows), f"template {key}: {field} native definition identity missing", errors)


def validate(snapshot: dict, oracle: dict) -> list[str]:
    errors: list[str] = []
    _require(snapshot.get("schema") == SCHEMA, "schema mismatch", errors)
    authority = snapshot.get("authority", {})
    _require(authority.get("program") == "hoi4.exe 1.19.2.0.a729", "program authority mismatch", errors)
    _require(authority.get("program_sha256") == PROGRAM_SHA256, "program SHA-256 mismatch", errors)
    _require(snapshot.get("full_replacement") is True, "snapshot is not a full replacement", errors)
    completeness = snapshot.get("completeness", {})
    _require(set(completeness) == COMPLETENESS and all(completeness.get(k) is True for k in COMPLETENESS), "completeness contract mismatch", errors)

    variants = _unique(snapshot.get("variants"), "variants", errors)
    templates = _unique(snapshot.get("division_templates"), "division_templates", errors)
    saved_variants = oracle.get("equipment_variants", {})
    saved_templates = {_key(row): row for row in oracle.get("division_templates", [])}
    _require(set(variants) == set(saved_variants), "variants: full catalog ID set differs from oracle", errors)
    _require(set(templates) == set(saved_templates), "templates: full catalog ID set differs from oracle", errors)

    catalog_ordinals = []
    ui_ordinals = []
    global_occurrences = set()
    pointer_to_key = {}
    for key, row in variants.items():
        _require(row.get("type") == TYPE_VARIANT, f"variant {key}: wrong type", errors)
        saved = saved_variants.get(key)
        if saved:
            _require(row.get("owner_tag") == saved.get("creator"), f"variant {key}: owner mismatch", errors)
            _require((row.get("origin_override_tag") or "---") == (saved.get("origin") or "---"), f"variant {key}: origin mismatch", errors)
            _require(row.get("obsolete") == bool(saved.get("obsolete")), f"variant {key}: obsolete mismatch", errors)
            _validate_variant_definition(key, row, saved, errors)
        catalog_ordinals.append(row.get("catalog_scan_ordinal"))
        occurrences = row.get("source_occurrences")
        _require(isinstance(occurrences, list), f"variant {key}: source occurrences missing", errors)
        first = row.get("first_ui_discovery_ordinal")
        if isinstance(occurrences, list) and occurrences:
            _require(isinstance(first, int), f"variant {key}: first UI ordinal missing", errors)
            if isinstance(first, int):
                ui_ordinals.append(first)
        else:
            _require(first is None, f"variant {key}: UI ordinal exists without occurrence", errors)
        for occurrence in occurrences or []:
            occurrence_key = (occurrence.get("source_country_tag"), occurrence.get("source_list"), occurrence.get("source_list_ordinal"))
            _require(occurrence.get("source_list") in ("A", "B") and isinstance(occurrence.get("source_list_ordinal"), int), f"variant {key}: invalid UI occurrence", errors)
            _require(occurrence_key not in global_occurrences, f"variant {key}: duplicate global UI occurrence", errors)
            global_occurrences.add(occurrence_key)
            pointer = occurrence.get("pointer_identity")
            _require(pointer == row.get("pointer_identity"), f"variant {key}: occurrence pointer mismatch", errors)
            if isinstance(pointer, str):
                _require(pointer_to_key.setdefault(pointer, key) == key, f"variant pointer {pointer}: conflicting IDs", errors)
    _require(_complete_ordinals(catalog_ordinals, len(variants)), "variants: catalog scan ordinals invalid", errors)
    _require(_complete_ordinals(ui_ordinals, len(ui_ordinals)), "variants: first UI ordinals invalid", errors)

    template_catalog_ordinals = []
    per_country_ordinals = defaultdict(list)
    for key, row in templates.items():
        _require(row.get("type") == TYPE_TEMPLATE, f"template {key}: wrong type", errors)
        saved = saved_templates.get(key)
        if saved:
            _require(row.get("owner_tag") == saved.get("country"), f"template {key}: owner mismatch", errors)
            _require(row.get("obsolete") == bool(saved.get("obsolete")), f"template {key}: obsolete mismatch", errors)
            _validate_template_definition(key, row, saved, errors)
        template_catalog_ordinals.append(row.get("catalog_scan_ordinal"))
        country = row.get("source_country_tag")
        ordinal = row.get("country_vector_ordinal")
        _require(country == row.get("owner_tag") and isinstance(ordinal, int), f"template {key}: invalid country-vector source", errors)
        if isinstance(country, str) and isinstance(ordinal, int):
            per_country_ordinals[country].append(ordinal)
    _require(_complete_ordinals(template_catalog_ordinals, len(templates)), "templates: catalog scan ordinals invalid", errors)
    for country, ordinals in per_country_ordinals.items():
        _require(_complete_ordinals(ordinals, len(ordinals)), f"templates: {country} country-vector ordinals invalid", errors)

    counts = snapshot.get("ui_occurrence_counts", {})
    _require(counts.get("unique_variants") == len(ui_ordinals), "UI occurrence unique-variant count mismatch", errors)
    _require(counts.get("catalog_variants_without_occurrence") == len(variants) - len(ui_ordinals), "UI occurrence absent-variant count mismatch", errors)
    return errors


def main(argv=None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--oracle", required=True, type=Path)
    parser.add_argument("--snapshot", required=True, type=Path)
    args = parser.parse_args(argv)
    errors = validate(_load(args.snapshot), _load(args.oracle))
    if errors:
        for error in errors:
            print(f"ERROR: {error}")
        return 1
    print("PASS: A27 native full catalog, definitions, lifecycle, and optional UI occurrences are exact")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
