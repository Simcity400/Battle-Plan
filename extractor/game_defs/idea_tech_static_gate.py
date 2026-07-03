#!/usr/bin/env python3
"""Build and verify idea/technology token -> localisation cross-reference.

The compact replay stream carries idea and technology tokens. This artifact is
the definition-side join from those tokens to loc keys and resolved display
names when the R3 localisation catalog contains the key.
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
import parse_idea_tech_loc  # noqa: E402
import parse_localisation  # noqa: E402
import localisation_gate  # noqa: E402
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
    verify_store,
    write_static_store,
)

GAME_ID = "idea_tech_loc_xref_static"
DEFAULT_OUT = _HERE / "out" / "idea_tech_loc_xref"
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


def _resolution_for_item(item: dict[str, Any], loc_keys: dict[str, Any]) -> dict[str, Any]:
    for candidate in localisation_gate._loc_candidates(item):
        key = candidate["key"]
        name = localisation_name(loc_keys, key)
        if name is not None:
            return {
                "localisation_status": "resolved",
                "resolved_loc_key": key,
                "resolution_rule": candidate["rule"],
                "localised_name": name,
            }
    literal_name = item.get("literal_name")
    if literal_name:
        return {
            "localisation_status": "resolved_literal",
            "resolved_loc_key": None,
            "resolution_rule": item.get("literal_rule") or "quoted_literal_name",
            "localised_name": literal_name,
        }
    return {
        "localisation_status": "missing_localisation_key" if item.get("loc_key") else "no_derivable_loc_key",
        "resolved_loc_key": None,
        "resolution_rule": None,
        "localised_name": None,
    }


def _enrich_records(records: dict[str, dict[str, Any]], loc_keys: dict[str, Any]) -> dict[str, dict[str, Any]]:
    items = {
        item["id"]: item
        for item in localisation_gate._items_from_records(records)
    }
    out = {}
    for token, rec in records.items():
        row = dict(rec)
        item = items[token]
        row.update(_resolution_for_item(item, loc_keys))
        candidates = localisation_gate._loc_candidates(item)
        if candidates:
            row["loc_candidates"] = candidates
        out[token] = row
    return dict(sorted(out.items()))


def _items_with_omitted(records: dict[str, dict[str, Any]], omitted: list[dict[str, Any]]) -> list[dict[str, Any]]:
    items = localisation_gate._items_from_records(records)
    items.extend(
        {
            "id": rec["id"],
            "loc_key": None,
            "reason": rec["reason"],
            "source_layer": rec.get("source_layer"),
            "source_file": rec.get("source_file"),
        }
        for rec in omitted
    )
    return items


def build_catalog(args: argparse.Namespace) -> tuple[dict[str, Any], dict[str, Any]]:
    layers = R.build_layers(
        vanilla=args.vanilla,
        mods=mods_from_arg(args.mods),
        userdata=args.userdata,
    )
    loc_keys, loc_source = _loc_catalog(args.localisation_catalog, layers)
    parsed = parse_idea_tech_loc.parse_idea_tech_loc_keys(layers, strict=True)
    idea_items = _items_with_omitted(parsed["tokens"]["ideas"], parsed["omitted"]["ideas"])
    tech_items = _items_with_omitted(parsed["tokens"]["technologies"], parsed["omitted"]["technologies"])
    coverage = {
        "schema": "idea_tech_loc_xref_static.localisation_coverage.v1",
        "categories": {
            "idea": localisation_gate._coverage_entry("idea", idea_items, set(loc_keys)),
            "tech": localisation_gate._coverage_entry("tech", tech_items, set(loc_keys)),
        },
    }
    catalog = {
        "schema": "idea_tech_loc_xref_static.static_artifact.v1",
        "artifact_kind": "deterministic_static_replay_enrichment",
        "scope": {
            "closed_rows": [
                "idea token -> loc key/localised name when derivable",
                "technology token -> loc key/localised name when derivable",
                "technology fallback candidates from enabled equipment/module loc keys",
            ],
            "not_closed_rows": [
                "runtime active idea ownership and completed research state",
                "script execution semantics for technology effects",
            ],
        },
        "join_keys": {
            "idea_live_token": "idea id string emitted by politics/idea live rows",
            "tech_live_token": "technology id string emitted by research live rows",
            "static_idea_key": "tokens.ideas.<id>",
            "static_technology_key": "tokens.technologies.<id>",
            "compatibility_status": "static token shape documented; no real decode-row sample verified by this static gate",
        },
        "layers": layer_summary(layers),
        "localisation_source": loc_source,
        "meta": parsed["meta"],
        "source_files": parsed["source_files"],
        "coverage": coverage,
        "tokens": {
            "ideas": _enrich_records(parsed["tokens"]["ideas"], loc_keys),
            "technologies": _enrich_records(parsed["tokens"]["technologies"], loc_keys),
        },
        "omitted": parsed["omitted"],
    }
    catalog["artifact_sha256"] = sha256_bytes(canonical_json(catalog))
    return catalog, coverage


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    add_layer_args(parser)
    parser.add_argument("--out-dir", type=Path, default=DEFAULT_OUT)
    parser.add_argument("--localisation-catalog", type=Path, default=DEFAULT_LOCALISATION_CATALOG)
    parser.add_argument("--min-ideas", type=int, default=1)
    parser.add_argument("--min-techs", type=int, default=1)
    args = parser.parse_args()

    catalog, coverage = build_catalog(args)
    paths = write_static_store(
        args.out_dir,
        game_id=GAME_ID,
        catalog_name="idea_tech_loc_xref_catalog.json",
        catalog=catalog,
        snapshot_schema="idea_tech_loc_xref_static.static_snapshot.v1",
        index_schema="idea_tech_loc_xref_static.static_index.v1",
        manifest_role="idea_tech_loc_xref_static_manifest",
        payload_role="idea_tech_loc_xref_static_payload",
        snapshot_extra={"counts": catalog["meta"], "artifact_sha256": catalog["artifact_sha256"]},
    )
    coverage_path = args.out_dir / "idea_tech_localisation_coverage.json"
    coverage_path.write_bytes(pretty_json({
        "localisation_source": catalog["localisation_source"],
        "coverage": coverage,
    }))
    store_verify = verify_store(paths)
    passed = bool(
        catalog["meta"]["idea_mapped"] >= args.min_ideas
        and catalog["meta"]["tech_mapped"] >= args.min_techs
        and catalog["meta"]["idea_files_parsed"] == catalog["meta"]["idea_files_total"]
        and catalog["meta"]["tech_files_parsed"] == catalog["meta"]["tech_files_total"]
        and not catalog["meta"].get("warnings")
        and store_verify["passed"]
    )
    report = {
        "schema": "idea_tech_loc_xref_static.static_gate_report.v1",
        "passed": passed,
        "paths": {key: repo_relative(path) for key, path in paths.items()} | {"coverage": repo_relative(coverage_path)},
        "counts": {
            "ideas": catalog["meta"]["idea_mapped"],
            "idea_omitted": catalog["meta"]["idea_omitted_no_loc_key"],
            "technologies": catalog["meta"]["tech_mapped"],
            "technology_omitted": catalog["meta"]["tech_omitted_no_loc_key"],
            "idea_files": catalog["meta"]["idea_files_total"],
            "idea_files_parsed": catalog["meta"]["idea_files_parsed"],
            "technology_files": catalog["meta"]["tech_files_total"],
            "technology_files_parsed": catalog["meta"]["tech_files_parsed"],
        },
        "coverage": coverage,
        "artifact_sha256": catalog["artifact_sha256"],
        "store_verify": store_verify,
        "warnings": catalog["meta"].get("warnings", []),
        "syntax_allowlist_hits": catalog["meta"].get("syntax_allowlist_hits", []),
        "wrapper_allowlist_hits": catalog["meta"].get("wrapper_allowlist_hits", []),
    }
    report_path = args.out_dir / "idea_tech_static_gate_report.json"
    report_path.write_bytes(pretty_json(report))
    print(json.dumps(report, ensure_ascii=False, sort_keys=True, indent=2))
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
