#!/usr/bin/env python3
"""Build same-save economy/production oracle JSON for domain 58 probes.

This lane-local wrapper reuses the repository's save-oracle parsers and adds
derived factory-use/resource fields that the country economy probe can compare
across multiple live countries. It never guesses through binary saves: if the
requested save is HOI4bin and rakaly is unavailable, it writes only
oracle_status.json and exits non-zero.
"""

from __future__ import annotations

import argparse
import json
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parents[4]
SAVE_ORACLE_DIR = REPO_ROOT / "extractor" / "save_oracle"
DEFAULT_OUTPUT_DIR = SAVE_ORACLE_DIR / "output"
DEFAULT_SAVE_DIR = (
    Path.home()
    / "OneDrive"
    / "Documents"
    / "Paradox Interactive"
    / "the game"
    / "save games"
)

if str(SAVE_ORACLE_DIR) not in sys.path:
    sys.path.insert(0, str(SAVE_ORACLE_DIR))

import deep_common as dc  # type: ignore  # noqa: E402
import parse_economy  # type: ignore  # noqa: E402
import parse_production  # type: ignore  # noqa: E402
import parse_save as ps  # type: ignore  # noqa: E402


RESOURCE_BLOCK_KEYS = (
    "produced",
    "exported",
    "to_export",
    "imported",
    "to_use",
    "to_use_total",
    "base_export",
    "export",
    "available",
)

RAW_FACTORY_FIELDS = (
    "civilian_total",
    "military_total",
    "dockyard_total",
    "civilian_from_state_building_levels",
    "military_from_state_building_levels",
    "dockyard_from_state_building_levels",
    "civilian_offmap_building_levels",
    "military_offmap_building_levels",
    "dockyard_offmap_building_levels",
    "construction_line_count",
    "construction_active_factories_sum",
    "military_assigned_active_factories_sum",
    "dockyard_assigned_active_factories_sum",
)

RAW_CONSUMER_FIELDS = (
    "consumer_goods_factories",
    "consumer_goods_factor",
    "consumer_goods_factor_ppm",
)


def write_json(path: Path, obj: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(obj, indent=1, sort_keys=True), encoding="utf-8")


def coerce_resource_map(raw: dict[str, str]) -> dict[str, float | int]:
    out: dict[str, float | int] = {}
    for key, value in raw.items():
        parsed = dc.to_float(value)
        if parsed is None:
            continue
        if abs(parsed - round(parsed)) < 0.000001:
            out[key] = int(round(parsed))
        else:
            out[key] = parsed
    return out


def parse_resource_extensions(text: str) -> dict[str, dict[str, dict[str, float | int]]]:
    by_tag: dict[str, dict[str, dict[str, float | int]]] = {}

    def parse_resources(tz: dc.Tokenizer, rec: dict[str, dict[str, float | int]]) -> None:
        for key, opener in dc.iter_block_entries(tz):
            if opener is dc.LBRACE:
                if key in RESOURCE_BLOCK_KEYS:
                    rec[key] = coerce_resource_map(dc.collect_block(tz))
                else:
                    dc._skip_block_body(tz)

    def parse_countries(tz: dc.Tokenizer) -> None:
        for tag, opener in dc.iter_block_entries(tz):
            if opener is not dc.LBRACE:
                continue
            resources: dict[str, dict[str, float | int]] = {}
            for key, op in dc.iter_block_entries(tz):
                if op is dc.LBRACE:
                    if key == "resources":
                        parse_resources(tz, resources)
                    else:
                        dc._skip_block_body(tz)
            if resources:
                by_tag[tag] = resources

    dc.top_level_pass(
        text,
        {"countries": parse_countries},
        scalar_keys=("date", "version", "player"),
    )
    return by_tag


def numeric_sum(rows: list[dict[str, Any]], key: str) -> int:
    total = 0
    for row in rows:
        value = row.get(key)
        if isinstance(value, bool):
            continue
        if isinstance(value, (int, float)):
            total += int(value)
    return total


def free_or_none(total: Any, used: Any) -> int | None:
    if not isinstance(total, int) or not isinstance(used, int):
        return None
    if used < 0 or used > total:
        return None
    return total - used


def merge_domain58_oracle_fields(
    economy: dict[str, Any],
    production: dict[str, Any],
    resource_extensions: dict[str, dict[str, dict[str, float | int]]],
) -> None:
    countries = economy.get("countries", {})
    prod_countries = production.get("countries", {})
    if not isinstance(countries, dict) or not isinstance(prod_countries, dict):
        return

    for tag, country in countries.items():
        if not isinstance(country, dict):
            continue
        factories = country.get("factories")
        if not isinstance(factories, dict):
            continue

        construction_queue = country.get("construction_queue", [])
        if not isinstance(construction_queue, list):
            construction_queue = []
        civ_used = numeric_sum(
            [row for row in construction_queue if isinstance(row, dict)],
            "active_factories",
        )
        prod_country = prod_countries.get(tag, {})
        factory_totals = (
            prod_country.get("factory_totals", {})
            if isinstance(prod_country, dict)
            else {}
        )
        mil_used = factory_totals.get("mil_factories_assigned")
        naval_used = factory_totals.get("naval_factories_assigned")

        if isinstance(civ_used, int):
            factories["civilian_used_derived"] = civ_used
            free = free_or_none(factories.get("civ"), civ_used)
            if free is not None:
                factories["civilian_free_derived"] = free
        if isinstance(mil_used, int):
            factories["military_used_assigned"] = mil_used
            free = free_or_none(factories.get("mil"), mil_used)
            if free is not None:
                factories["military_free_derived"] = free
        if isinstance(naval_used, int):
            factories["dockyard_used_assigned"] = naval_used
            free = free_or_none(factories.get("dockyard"), naval_used)
            if free is not None:
                factories["dockyard_free_derived"] = free

        resources = country.setdefault("resources", {})
        if isinstance(resources, dict):
            for key, values in resource_extensions.get(tag, {}).items():
                resources.setdefault(key, values)

    meta = economy.setdefault("meta", {})
    notes = meta.setdefault("notes", {})
    if isinstance(notes, dict):
        notes["domain58_factory_usage"] = (
            "civilian_used_derived sums construction_queue.active_factories; "
            "military_used_assigned/naval_used_assigned come from production.json "
            "factory_totals. Free fields are total minus those assigned counts."
        )
        notes["domain58_damaged_factories"] = (
            "No fail-closed damaged-factory scalar is exposed by this save parser; "
            "validate from live/UI oracle before emitting."
        )
        notes["domain58_overlaps"] = (
            "fuel remains domain 10; stability and war_support remain domain 14."
        )


def count_non_null_fields(countries: dict[str, Any]) -> dict[str, int]:
    coverage: dict[str, int] = {
        **{f"factories_raw.{key}": 0 for key in RAW_FACTORY_FIELDS},
        **{f"consumer_goods_raw.{key}": 0 for key in RAW_CONSUMER_FIELDS},
    }
    for row in countries.values():
        if not isinstance(row, dict):
            continue
        for group_name in ("factories_raw", "consumer_goods_raw"):
            group = row.get(group_name, {})
            if not isinstance(group, dict):
                continue
            for key, value in group.items():
                if value is not None:
                    coverage[f"{group_name}.{key}"] = coverage.get(f"{group_name}.{key}", 0) + 1
    return dict(sorted(coverage.items()))


def build_economy_raw(
    economy: dict[str, Any],
    production: dict[str, Any],
) -> dict[str, Any]:
    economy_countries = economy.get("countries", {})
    production_countries = production.get("countries", {})
    out_countries: dict[str, Any] = {}
    if not isinstance(economy_countries, dict):
        economy_countries = {}
    if not isinstance(production_countries, dict):
        production_countries = {}

    for tag in sorted(economy_countries):
        row = economy_countries.get(tag, {})
        factories = row.get("factories", {}) if isinstance(row, dict) else {}
        construction_queue = row.get("construction_queue", []) if isinstance(row, dict) else []
        if not isinstance(factories, dict):
            factories = {}
        if not isinstance(construction_queue, list):
            construction_queue = []
        offmap = factories.get("offmap", {})
        if not isinstance(offmap, dict):
            offmap = {}
        prod_row = production_countries.get(tag, {})
        factory_totals = (
            prod_row.get("factory_totals", {})
            if isinstance(prod_row, dict)
            else {}
        )
        construction_rows = [
            item for item in construction_queue if isinstance(item, dict)
        ]

        factories_raw = {
            "civilian_total": factories.get("civ"),
            "military_total": factories.get("mil"),
            "dockyard_total": factories.get("dockyard"),
            "civilian_from_state_building_levels": factories.get("civ_from_states"),
            "military_from_state_building_levels": factories.get("mil_from_states"),
            "dockyard_from_state_building_levels": factories.get("dockyard_from_states"),
            "civilian_offmap_building_levels": offmap.get("civ"),
            "military_offmap_building_levels": offmap.get("mil"),
            "dockyard_offmap_building_levels": offmap.get("dockyard"),
            "construction_line_count": factories.get("under_construction"),
            "construction_active_factories_sum": numeric_sum(
                construction_rows, "active_factories"
            ),
            "military_assigned_active_factories_sum": factory_totals.get(
                "mil_factories_assigned"
            ),
            "dockyard_assigned_active_factories_sum": factory_totals.get(
                "naval_factories_assigned"
            ),
        }
        for key in RAW_FACTORY_FIELDS:
            factories_raw.setdefault(key, None)
        consumer_goods_raw = {key: None for key in RAW_CONSUMER_FIELDS}
        out_countries[tag] = {
            "factories_raw": factories_raw,
            "consumer_goods_raw": consumer_goods_raw,
        }

    coverage = count_non_null_fields(out_countries)
    return {
        "schema": "battle-plan.country_economy_counters_live.economy_raw_oracle.v1",
        "meta": {
            "source_filename": economy.get("meta", {}).get("source_filename"),
            "save_date": economy.get("meta", {}).get("save_date"),
            "save_version": economy.get("meta", {}).get("save_version"),
            "player": economy.get("meta", {}).get("player"),
            "countries": len(out_countries),
            "raw_field_coverage": coverage,
            "notes": {
                "no_free_or_used_derived_fields": (
                    "This raw oracle intentionally omits civilian_free_derived, "
                    "civilian_used_derived, military_free_derived, and "
                    "dockyard_free_derived from economy.json."
                ),
                "factory_totals": (
                    "Factory total fields are sums of stored state building levels "
                    "plus stored country offmap building levels; the save does not "
                    "expose a direct country scalar for these totals."
                ),
                "assigned_active_factories": (
                    "Assigned fields are sums of stored production-line active "
                    "factories, not free/available calculations."
                ),
                "consumer_goods": (
                    "No direct consumer-goods factory or factor scalar was found "
                    "in the save country blocks; fields are explicit null."
                ),
            },
        },
        "countries": out_countries,
    }


def resolve_save(arg: str | None) -> Path | None:
    if arg:
        path = Path(arg)
        return path if path.exists() else None
    latest = ps.find_latest_save()
    if latest is not None:
        return latest
    candidates = []
    try:
        candidates = [
            p
            for p in DEFAULT_SAVE_DIR.glob("*.hoi4")
            if p.is_file()
        ]
    except OSError:
        candidates = []
    return max(candidates, key=lambda p: p.stat().st_mtime) if candidates else None


def output_status(
    out_dir: Path,
    status: str,
    save_path: Path | None,
    details: dict[str, Any] | None = None,
) -> None:
    body: dict[str, Any] = {
        "schema": "battle-plan.country_economy_counters_live.oracle_status.v1",
        "status": status,
        "generated_utc": datetime.now(timezone.utc).isoformat(timespec="seconds"),
        "save_path_redacted": ps._redact(save_path) if save_path else None,
        "details": details or {},
    }
    write_json(out_dir / "oracle_status.json", body)


def build_oracle(save_path: Path, out_dir: Path, rakaly_arg: str | None) -> int:
    out_dir.mkdir(parents=True, exist_ok=True)
    rakaly = ps.find_rakaly(rakaly_arg)
    print(f"[domain58-oracle] save   : {ps._scrub(str(save_path))}")
    print(
        "[domain58-oracle] rakaly : "
        f"{ps._scrub(str(rakaly)) if rakaly else 'NOT FOUND'}"
    )

    try:
        text, was_melted = ps.melt_if_needed(save_path, rakaly, out_dir)
    except RuntimeError as exc:
        output_status(
            out_dir,
            "blocked_binary_save_needs_rakaly_or_uncompressed_text_save",
            save_path,
            {"error": str(exc)},
        )
        print(f"ERROR: {exc}", file=sys.stderr)
        print(
            "[domain58-oracle] wrote blocked status only: "
            f"{ps._scrub(str(out_dir / 'oracle_status.json'))}"
        )
        return 3

    economy_outputs, economy_report = parse_economy.build(text, save_path, was_melted)
    production_outputs, production_report = parse_production.build(text, save_path, was_melted)
    economy = economy_outputs["economy.json"]
    production = production_outputs["production.json"]
    resource_extensions = parse_resource_extensions(text)
    merge_domain58_oracle_fields(economy, production, resource_extensions)
    economy_raw = build_economy_raw(economy, production)

    status = {
        "schema": "battle-plan.country_economy_counters_live.oracle_status.v1",
        "status": "ok",
        "generated_utc": datetime.now(timezone.utc).isoformat(timespec="seconds"),
        "save_path_redacted": ps._redact(save_path),
        "was_melted": was_melted,
        "economy_stats": economy.get("meta", {}).get("stats", {}),
        "production_stats": production.get("meta", {}).get("stats", {}),
        "economy_raw_stats": {
            "countries": economy_raw.get("meta", {}).get("countries"),
            "raw_field_coverage": economy_raw.get("meta", {}).get("raw_field_coverage", {}),
        },
        "resource_extension_country_count": len(resource_extensions),
        "same_tick_requirement": (
            "Use these files only while hoi4.exe is loaded on this exact save "
            "before unpausing or advancing a tick."
        ),
    }

    write_json(out_dir / "economy.json", economy)
    write_json(out_dir / "economy_raw.json", economy_raw)
    write_json(out_dir / "production.json", production)
    write_json(out_dir / "oracle_status.json", status)

    print()
    print(economy_report)
    print()
    print(production_report)
    print()
    print("=== RAW ECONOMY ORACLE COVERAGE ===")
    print(f"countries     : {economy_raw.get('meta', {}).get('countries')}")
    coverage = economy_raw.get("meta", {}).get("raw_field_coverage", {})
    for key, count in coverage.items():
        print(f"{key:52s}: {count}")
    print()
    print(
        "[domain58-oracle] wrote: "
        f"{ps._scrub(str(out_dir / 'economy.json'))}"
    )
    print(
        "[domain58-oracle] wrote: "
        f"{ps._scrub(str(out_dir / 'economy_raw.json'))}"
    )
    print(
        "[domain58-oracle] wrote: "
        f"{ps._scrub(str(out_dir / 'production.json'))}"
    )
    print(
        "[domain58-oracle] wrote: "
        f"{ps._scrub(str(out_dir / 'oracle_status.json'))}"
    )
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("save", nargs="?", help="Path to a .hoi4 save or HOI4txt melt.")
    parser.add_argument("--save", dest="save_override", help="Path to a .hoi4 save or HOI4txt melt.")
    parser.add_argument("--rakaly", help="Path to rakaly.exe for HOI4bin saves.")
    parser.add_argument(
        "--out-dir",
        default=str(DEFAULT_OUTPUT_DIR),
        help="Output directory for economy.json and production.json.",
    )
    args = parser.parse_args(argv)

    save_path = resolve_save(args.save_override or args.save)
    out_dir = Path(args.out_dir)
    if save_path is None:
        output_status(out_dir, "blocked_no_save_found", None)
        print("ERROR: no .hoi4 save or HOI4txt melt found.", file=sys.stderr)
        return 2
    return build_oracle(save_path, out_dir, args.rakaly)


if __name__ == "__main__":
    raise SystemExit(main())
