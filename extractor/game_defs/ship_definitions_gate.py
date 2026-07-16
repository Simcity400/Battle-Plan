#!/usr/bin/env python3
"""Build and verify the ship hull/module static-definition artifact."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any

_HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(_HERE))

import resolver as R  # noqa: E402
import source_inventory_layers as SIL  # noqa: E402
import parse_ship_definitions  # noqa: E402
import parse_localisation  # noqa: E402
from static_artifact_common import (  # noqa: E402
    add_layer_args,
    canonical_json,
    layer_summary,
    load_localisation_catalog,
    localisation_name,
    mods_from_arg,
    pretty_json,
    repo_relative,
    sha256_bytes,
    simple_localisation_coverage,
    verify_store,
    write_static_store,
)

GAME_ID = "ship_definitions_static"
DEFAULT_OUT = _HERE / "out" / "ship_definitions_static"
DEFAULT_LOCALISATION_CATALOG = (
    _HERE / "out" / "localisation_gate" / "static_localisation_join_r3" / "localisation_catalog.json"
)


def _loc_catalog(path: Path, layers: list[Any]) -> tuple[dict[str, Any], dict[str, Any]]:
    if path.is_file():
        data = load_localisation_catalog(path)
        return data["keys"], {
            "source": "static_localisation_join_r3",
            "path": repo_relative(path),
            "key_count": data.get("meta", {}).get("key_count", len(data["keys"])),
            "sha256": sha256_bytes(path.read_bytes()),
        }
    built = parse_localisation.parse_localisation(layers)
    return built["keys"], {
        "source": "built_in_gate_missing_r3_catalog",
        "path": None,
        "key_count": built["meta"]["key_count"],
        "sha256": sha256_bytes(canonical_json(built)),
    }


def _enrich(records: dict[str, dict[str, Any]], loc_keys: dict[str, Any]) -> dict[str, dict[str, Any]]:
    out = {}
    for token, rec in records.items():
        row = dict(rec)
        name = localisation_name(loc_keys, row.get("loc_key"))
        row["localised_name"] = name
        row["localisation_status"] = "resolved" if name else "missing_localisation_key"
        out[token] = row
    return dict(sorted(out.items()))


def _coverage_items(records: dict[str, dict[str, Any]]) -> list[dict[str, Any]]:
    return [
        {
            "id": token,
            "loc_key": rec.get("loc_key"),
            "source_file": rec.get("source_file"),
            "source_layer": rec.get("source_layer"),
        }
        for token, rec in records.items()
    ]


def build_catalog(args: argparse.Namespace) -> tuple[dict[str, Any], dict[str, Any]]:
    layers = R.build_layers(
        vanilla=args.vanilla,
        mods=mods_from_arg(args.mods),
        userdata=args.userdata,
        strict_mods=getattr(args, "strict_mods", False),
    )
    layers, source_namespace_proof = SIL.definition_layers(
        layers, getattr(args, "source_inventory", None)
    )
    loc_keys, loc_source = _loc_catalog(args.localisation_catalog, layers)
    parsed = parse_ship_definitions.parse_ship_definitions(layers, strict=True)
    coverage = {
        "schema": "ship_definitions_static.localisation_coverage.v1",
        "categories": {
            "ship_hulls": simple_localisation_coverage(
                "ship_hulls",
                _coverage_items(parsed["ship_hulls"]),
                loc_keys,
            ),
            "ship_modules": simple_localisation_coverage(
                "ship_modules",
                _coverage_items(parsed["ship_modules"]),
                loc_keys,
            ),
        },
    }
    catalog = {
        "schema": "ship_definitions_static.static_artifact.v1",
        "artifact_kind": "deterministic_static_replay_enrichment",
        "scope": {
            "closed_rows": [
                "ship hull/equipment definition token -> static hull/module-slot/stats metadata",
                "ship equipment module token -> category/stat/convert metadata",
                "loc key/localised name where R3 localisation contains the key",
            ],
            "not_closed_rows": [
                "runtime ship rows, task-force membership, and strength/org/XP (domain 29)",
                "runtime ship module composition if not encoded in the definition token",
            ],
        },
        "join_keys": {
            "domain29_live_ship_definition": "order_of_battle.ships[*].definition",
            "static_ship_hull_key": "ship_hulls.<id>",
            "static_ship_module_key": "ship_modules.<id>",
            "compatibility_status": "decoder shape documented; no real decode-row sample verified by this static gate",
        },
        "layers": layer_summary(layers),
        "source_namespace_proof": source_namespace_proof,
        "localisation_source": loc_source,
        "meta": parsed["meta"],
        "source_files": parsed["source_files"],
        "coverage": coverage,
        "ship_hulls": _enrich(parsed["ship_hulls"], loc_keys),
        "ship_modules": _enrich(parsed["ship_modules"], loc_keys),
    }
    catalog["artifact_sha256"] = sha256_bytes(canonical_json(catalog))
    return catalog, coverage


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    add_layer_args(parser)
    parser.add_argument("--out-dir", type=Path, default=DEFAULT_OUT)
    parser.add_argument("--localisation-catalog", type=Path, default=DEFAULT_LOCALISATION_CATALOG)
    parser.add_argument("--min-hulls", type=int, default=1)
    parser.add_argument("--min-modules", type=int, default=1)
    args = parser.parse_args()

    catalog, coverage = build_catalog(args)
    paths = write_static_store(
        args.out_dir,
        game_id=GAME_ID,
        catalog_name="ship_definitions_catalog.json",
        catalog=catalog,
        snapshot_schema="ship_definitions_static.static_snapshot.v1",
        index_schema="ship_definitions_static.static_index.v1",
        manifest_role="ship_definitions_static_manifest",
        payload_role="ship_definitions_static_payload",
        snapshot_extra={"counts": catalog["meta"], "artifact_sha256": catalog["artifact_sha256"]},
    )
    coverage_path = args.out_dir / "ship_definitions_localisation_coverage.json"
    coverage_path.write_bytes(pretty_json({
        "localisation_source": catalog["localisation_source"],
        "coverage": coverage,
    }))
    store_verify = verify_store(paths)
    passed = bool(
        catalog["meta"]["ship_hull_count"] >= args.min_hulls
        and catalog["meta"]["ship_module_count"] >= args.min_modules
        and catalog["meta"]["equipment_files_parsed"] == catalog["meta"]["equipment_files_total"]
        and catalog["meta"]["module_files_parsed"] == catalog["meta"]["module_files_total"]
        and not catalog["meta"].get("warnings")
        and store_verify["passed"]
    )
    report = {
        "schema": "ship_definitions_static.static_gate_report.v1",
        "passed": passed,
        "paths": {key: repo_relative(path) for key, path in paths.items()} | {"coverage": repo_relative(coverage_path)},
        "counts": {
            "ship_hulls": catalog["meta"]["ship_hull_count"],
            "ship_modules": catalog["meta"]["ship_module_count"],
            "equipment_files": catalog["meta"]["equipment_files_total"],
            "equipment_files_parsed": catalog["meta"]["equipment_files_parsed"],
            "module_files": catalog["meta"]["module_files_total"],
            "module_files_parsed": catalog["meta"]["module_files_parsed"],
            "ship_hull_source_files": catalog["meta"]["ship_hull_files_total"],
            "ship_module_source_files": catalog["meta"]["ship_module_files_total"],
        },
        "skipped_files": catalog["meta"].get("skipped_files", []),
        "coverage": coverage,
        "artifact_sha256": catalog["artifact_sha256"],
        "store_verify": store_verify,
        "warnings": catalog["meta"].get("warnings", []),
    }
    report_path = args.out_dir / "ship_definitions_gate_report.json"
    report_path.write_bytes(pretty_json(report))
    print(json.dumps(report, ensure_ascii=False, sort_keys=True, indent=2))
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
