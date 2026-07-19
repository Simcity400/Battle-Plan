#!/usr/bin/env python3
"""Reproduce the frozen A27 same-tick membership and field audit."""

from __future__ import annotations

import argparse
import json
from collections import Counter
from pathlib import Path


def _load(path: Path):
    with path.open("r", encoding="utf-8") as handle:
        return json.load(handle)


def _oracle_variant(variants: dict, variant_id: str) -> dict | None:
    return variants.get(f"{variant_id}:70")


def _live_tokens(row: dict) -> list[str]:
    return sorted(module["token"] for module in row.get("modules", []))


def _oracle_tokens(row: dict) -> list[str]:
    return sorted(row.get("modules", {}).values())


def audit(oracle: dict, snapshot: dict) -> dict:
    variants = oracle["equipment_variants"]
    designs = snapshot["designs"]
    stockpile = designs["_r2_stockpile_variant_defs"]
    air = designs["equipment_variants_live"]

    missing_stockpile: list[str] = []
    missing_air: list[str] = []
    stockpile_module_mismatches: list[str] = []
    air_module_mismatches: list[str] = []
    name_exceptions: list[dict] = []

    def compare(variant_id: str, live: dict, missing: list[str], module_mismatches: list[str]):
        saved = _oracle_variant(variants, variant_id)
        if saved is None:
            missing.append(variant_id)
            return
        if _live_tokens(live) != _oracle_tokens(saved):
            module_mismatches.append(variant_id)
        if live.get("display_name") != saved.get("name"):
            name_exceptions.append(
                {
                    "variant_id": int(variant_id),
                    "creator": saved.get("creator"),
                    "family": saved.get("family"),
                    "oracle_text": saved.get("name"),
                    "live_text": live.get("display_name"),
                }
            )

    for variant_id, live in stockpile.items():
        compare(variant_id, live, missing_stockpile, stockpile_module_mismatches)
    for variant_id, row in air.items():
        compare(variant_id, row["live_definition"], missing_air, air_module_mismatches)

    observed_ids = set(stockpile) | set(air)
    oracle_families = Counter(row["family"] for row in variants.values())
    observed_families = Counter(
        _oracle_variant(variants, variant_id)["family"]
        for variant_id in observed_ids
        if _oracle_variant(variants, variant_id) is not None
    )
    family_rows = {
        family: {
            "observed": observed_families[family],
            "oracle": total,
            "missing": total - observed_families[family],
            "coverage_percent": round(100.0 * observed_families[family] / total, 3),
        }
        for family, total in sorted(oracle_families.items())
    }
    origin_rows = sorted(
        (
            {
                "variant_id": int(row["id"]),
                "creator": row.get("creator"),
                "origin": row.get("origin"),
            }
            for row in variants.values()
            if row.get("origin") not in (None, "", "---")
        ),
        key=lambda row: row["variant_id"],
    )

    return {
        "oracle_division_templates": len(oracle["division_templates"]),
        "oracle_variants": len(variants),
        "stockpile_rows": len(stockpile),
        "air_rows": len(air),
        "live_union": len(observed_ids),
        "missing_variants": len(variants) - len(observed_ids),
        "union_coverage_percent": round(100.0 * len(observed_ids) / len(variants), 3),
        "families": family_rows,
        "missing_stockpile_ids": sorted(missing_stockpile, key=int),
        "missing_air_ids": sorted(missing_air, key=int),
        "stockpile_module_mismatches": sorted(stockpile_module_mismatches, key=int),
        "air_module_mismatches": sorted(air_module_mismatches, key=int),
        "name_exceptions": sorted(name_exceptions, key=lambda row: row["variant_id"]),
        "origin_rows": origin_rows,
    }


def verify_frozen_report(result: dict, report: dict) -> list[str]:
    errors: list[str] = []
    authority = report.get("authority", {})
    if authority.get("program") != "hoi4.exe 1.19.2.0.a729":
        errors.append("authority.program: wrong executable/build")
    if authority.get("image_base") != "0x140000000":
        errors.append("authority.image_base: wrong image base")
    if authority.get("program_sha256") != "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad":
        errors.append("authority.program_sha256: wrong executable hash")
    provenance = report.get("ghidra_provenance")
    if not isinstance(provenance, list) or not provenance:
        errors.append("ghidra_provenance: missing")
    else:
        required = {"function_address", "job_id", "artifact_path", "artifact_sha256", "manifest_sha256"}
        for index, row in enumerate(provenance):
            missing = sorted(name for name in required if not row.get(name))
            if missing:
                errors.append(f"ghidra_provenance[{index}]: missing {','.join(missing)}")
            for hash_name in ("artifact_sha256", "manifest_sha256"):
                value = row.get(hash_name)
                if not isinstance(value, str) or len(value) != 64:
                    errors.append(f"ghidra_provenance[{index}].{hash_name}: invalid SHA-256")
    oracle = report["oracle_counts"]
    live = report["existing_live_carriers"]
    expected = {
        "oracle_division_templates": oracle["division_templates"],
        "oracle_variants": oracle["equipment_variants"],
        "stockpile_rows": live["stockpile_variant_defs"],
        "air_rows": live["air_variant_defs"],
        "live_union": live["union"],
        "missing_variants": live["missing_variants"],
        "union_coverage_percent": live["union_coverage_percent"],
        "families": live["families"],
        "name_exceptions": report["same_tick_field_validation"]["name_exceptions"],
        "origin_rows": report["same_tick_field_validation"]["creator_origin_semantic_correlation"]["origin_rows"],
    }
    for key, expected_value in expected.items():
        if result[key] != expected_value:
            errors.append(f"{key}: computed={result[key]!r} frozen={expected_value!r}")
    for key in (
        "missing_stockpile_ids",
        "missing_air_ids",
        "stockpile_module_mismatches",
        "air_module_mismatches",
    ):
        if result[key]:
            errors.append(f"{key}: {result[key]!r}")
    return errors


def main(argv=None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--oracle", required=True, type=Path)
    parser.add_argument("--snapshot", required=True, type=Path)
    parser.add_argument("--report", required=True, type=Path)
    args = parser.parse_args(argv)

    result = audit(_load(args.oracle), _load(args.snapshot))
    report = _load(args.report)
    errors = verify_frozen_report(result, report)
    print(json.dumps(result, ensure_ascii=True, sort_keys=True, indent=2))
    if errors:
        for error in errors:
            print(f"ERROR: {error}")
        return 1
    print("PASS: frozen same-tick A27 audit reproduced; promotion remains fail-closed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
