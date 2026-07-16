#!/usr/bin/env python3
"""Build and verify the decisions static-definition artifact.

This is a definition-side companion to live decision domains 16 and 45. It
does not register a compact domain or touch recorder packing; it emits a
deterministic snapshot-store payload ready for orchestrator packing as a static
artifact.
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
import source_inventory_layers as SIL  # noqa: E402
import parse_decisions  # noqa: E402
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

GAME_ID = "decisions_static_definitions"
DEFAULT_OUT = _HERE / "out" / "decisions_static"
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


def _enrich_decisions(decisions: dict[str, dict[str, Any]], loc_keys: dict[str, Any]) -> dict[str, dict[str, Any]]:
    out = {}
    for token, rec in decisions.items():
        row = dict(rec)
        name = localisation_name(loc_keys, row.get("loc_key"))
        row["localised_name"] = name
        row["localisation_status"] = "resolved" if name else "missing_localisation_key"
        out[token] = row
    return dict(sorted(out.items()))


def _decision_items(parsed: dict[str, Any]) -> list[dict[str, Any]]:
    items = [
        {
            "id": token,
            "loc_key": rec.get("loc_key"),
            "source_file": rec.get("source_file"),
            "source_layer": rec.get("source_layer"),
        }
        for token, rec in parsed["decisions"].items()
    ]
    items.extend(
        {
            "id": rec["id"],
            "loc_key": None,
            "reason": rec["reason"],
            "source_file": rec.get("source_file"),
            "source_layer": rec.get("source_layer"),
        }
        for rec in parsed["omitted"]
    )
    return items


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
    parsed = parse_decisions.parse_decisions(layers, strict=True)
    coverage = simple_localisation_coverage("decision", _decision_items(parsed), loc_keys)
    catalog = {
        "schema": "decisions_static_definitions.static_artifact.v1",
        "artifact_kind": "deterministic_static_replay_enrichment",
        "scope": {
            "closed_rows": [
                "decision id -> category, loc key, localised name when present",
                "decision static cost/target/cooldown/visibility/fire-only-once metadata",
                "decision category icon/picture metadata",
            ],
            "not_closed_rows": [
                "runtime active/timed decision state (domains 16 and 45)",
                "engine execution of scripted visible/available triggers",
            ],
        },
        "join_keys": {
            "live_decision_token": "decision id string emitted by domains 16/45",
            "static_decision_key": "decisions.<id>",
            "localisation_key": "decisions.<id>.loc_key",
            "compatibility_status": "decoder shape documented; no real decode-row sample verified by this static gate",
        },
        "layers": layer_summary(layers),
        "source_namespace_proof": source_namespace_proof,
        "localisation_source": loc_source,
        "meta": parsed["meta"],
        "source_files": parsed["source_files"],
        "coverage": coverage,
        "categories": parsed["categories"],
        "decisions": _enrich_decisions(parsed["decisions"], loc_keys),
        "omitted": parsed["omitted"],
    }
    catalog["artifact_sha256"] = sha256_bytes(canonical_json(catalog))
    return catalog, coverage


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    add_layer_args(parser)
    parser.add_argument("--out-dir", type=Path, default=DEFAULT_OUT)
    parser.add_argument("--localisation-catalog", type=Path, default=DEFAULT_LOCALISATION_CATALOG)
    parser.add_argument("--min-decisions", type=int, default=1)
    args = parser.parse_args()

    catalog, coverage = build_catalog(args)
    paths = write_static_store(
        args.out_dir,
        game_id=GAME_ID,
        catalog_name="decision_definitions_catalog.json",
        catalog=catalog,
        snapshot_schema="decisions_static_definitions.static_snapshot.v1",
        index_schema="decisions_static_definitions.static_index.v1",
        manifest_role="decisions_static_definitions_manifest",
        payload_role="decisions_static_definitions_payload",
        snapshot_extra={"counts": catalog["meta"], "artifact_sha256": catalog["artifact_sha256"]},
    )
    coverage_path = args.out_dir / "decision_localisation_coverage.json"
    coverage_path.write_bytes(pretty_json({
        "schema": "decisions_static_definitions.localisation_coverage.v1",
        "localisation_source": catalog["localisation_source"],
        "coverage": coverage,
    }))
    store_verify = verify_store(paths)
    passed = bool(
        catalog["meta"]["decision_mapped"] >= args.min_decisions
        and catalog["meta"]["category_count"] > 0
        and catalog["meta"]["decision_files_parsed"] == catalog["meta"]["decision_files_total"]
        and catalog["meta"]["category_files_parsed"] == catalog["meta"]["category_files_total"]
        and not catalog["meta"].get("warnings")
        and store_verify["passed"]
    )
    report = {
        "schema": "decisions_static_definitions.static_gate_report.v1",
        "passed": passed,
        "paths": {key: repo_relative(path) for key, path in paths.items()} | {"coverage": repo_relative(coverage_path)},
        "counts": {
            "decisions": catalog["meta"]["decision_mapped"],
            "omitted": catalog["meta"]["decision_omitted_no_loc_key"],
            "categories": catalog["meta"]["category_count"],
            "decision_files": catalog["meta"]["decision_files_total"],
            "decision_files_parsed": catalog["meta"]["decision_files_parsed"],
            "category_files": catalog["meta"]["category_files_total"],
            "category_files_parsed": catalog["meta"]["category_files_parsed"],
        },
        "coverage": coverage,
        "artifact_sha256": catalog["artifact_sha256"],
        "store_verify": store_verify,
        "warnings": catalog["meta"].get("warnings", []),
    }
    report_path = args.out_dir / "decisions_static_gate_report.json"
    report_path.write_bytes(pretty_json(report))
    print(json.dumps(report, ensure_ascii=False, sort_keys=True, indent=2))
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
