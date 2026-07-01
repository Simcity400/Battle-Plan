"""Reject module-token-only archetype/base-airframe closure for active air variants.

This proof consumes an existing DLL compact stream and checks a tempting route:
"the air variant module-token multiset is enough to classify active aircraft."
The route is invalid when the same module fingerprint maps to multiple live
equipment/stat profiles, or when an active variant has no module/name payload.
"""

from __future__ import annotations

import argparse
import json
from collections import defaultdict
from dataclasses import asdict
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

import proof_variant_archetype_join as live_join


def stat_profile(stats: live_join.VariantStats | None) -> dict[str, int | None]:
    if stats is None:
        return {
            "equipment_token_raw": None,
            "equipment_type_token_raw": None,
            "interface_type_raw": None,
            "speed_cache_raw": None,
            "range_raw": None,
            "air_attack_raw": None,
            "agility_raw": None,
            "max_speed_raw": None,
            "reliability_raw": None,
            "air_defence_raw": None,
            "thrust_raw": None,
            "fuel_consumption_raw": None,
            "build_cost_ic_raw": None,
        }
    return {
        "equipment_token_raw": stats.equipment_token_raw,
        "equipment_type_token_raw": stats.equipment_type_token_raw,
        "interface_type_raw": stats.interface_type_raw,
        "speed_cache_raw": stats.speed_cache_raw,
        "range_raw": stats.range_raw,
        "air_attack_raw": stats.air_attack_raw,
        "agility_raw": stats.agility_raw,
        "max_speed_raw": stats.max_speed_raw,
        "reliability_raw": stats.reliability_raw,
        "air_defence_raw": stats.air_defence_raw,
        "thrust_raw": stats.thrust_raw,
        "fuel_consumption_raw": stats.fuel_consumption_raw,
        "build_cost_ic_raw": stats.build_cost_ic_raw,
    }


def compact_profile_key(stats: live_join.VariantStats | None) -> tuple[int | None, ...]:
    profile = stat_profile(stats)
    return (
        profile["equipment_token_raw"],
        profile["equipment_type_token_raw"],
        profile["interface_type_raw"],
        profile["speed_cache_raw"],
        profile["range_raw"],
        profile["air_attack_raw"],
        profile["agility_raw"],
        profile["max_speed_raw"],
        profile["reliability_raw"],
        profile["air_defence_raw"],
        profile["thrust_raw"],
        profile["fuel_consumption_raw"],
        profile["build_cost_ic_raw"],
    )


def active_superiority_index(rows: list[live_join.SuperiorityRow]) -> dict[int, dict[str, Any]]:
    out: dict[int, dict[str, Any]] = {}
    for row in rows:
        if row.deleted or row.variant_id in out:
            continue
        out[row.variant_id] = {
            "region_id": row.region_id,
            "owner_tag_db_id": row.owner_tag_db_id,
            "mission_mask": row.mission_mask,
            "wing_count": row.wing_count,
            "amount_raw": row.amount_raw,
        }
    return out


def variant_row(
    variant_id: int,
    definitions: dict[int, live_join.VariantDef],
    stats: dict[int, live_join.VariantStats],
    superiority_index: dict[int, dict[str, Any]],
) -> dict[str, Any]:
    definition = definitions.get(variant_id)
    return {
        "variant_id": variant_id,
        "display_name": definition.display_name if definition else None,
        "variant_name": definition.variant_name if definition else None,
        "module_tokens": list(definition.modules) if definition else [],
        "stats": stat_profile(stats.get(variant_id)),
        "superiority_input": superiority_index.get(variant_id),
    }


def build_proof(args: argparse.Namespace) -> dict[str, Any]:
    stream = live_join.load_live_stream(Path(args.stream))
    definitions: dict[int, live_join.VariantDef] = stream["combined_defs"]
    stats: dict[int, live_join.VariantStats] = stream["stats"]
    active_variant_ids: list[int] = stream["active_superiority_variant_ids"]
    superiority_rows: list[live_join.SuperiorityRow] = stream["superiority_rows"]
    superiority_index = active_superiority_index(superiority_rows)

    groups: dict[tuple[str, ...], list[int]] = defaultdict(list)
    for variant_id in active_variant_ids:
        definition = definitions.get(variant_id)
        module_fingerprint = tuple(sorted(definition.modules)) if definition else ()
        groups[module_fingerprint].append(variant_id)

    collisions: list[dict[str, Any]] = []
    for module_fingerprint, variant_ids in sorted(
        groups.items(), key=lambda item: (-len(item[1]), item[0])
    ):
        profiles = {compact_profile_key(stats.get(variant_id)) for variant_id in variant_ids}
        if len(variant_ids) <= 1 or len(profiles) <= 1:
            continue
        collisions.append(
            {
                "module_fingerprint": list(module_fingerprint),
                "active_variant_count": len(variant_ids),
                "distinct_live_stat_profile_count": len(profiles),
                "examples": [
                    variant_row(variant_id, definitions, stats, superiority_index)
                    for variant_id in variant_ids[: args.examples_per_collision]
                ],
            }
        )

    unresolved_moduleless = [
        variant_row(variant_id, definitions, stats, superiority_index)
        for variant_id in active_variant_ids
        if (
            variant_id not in definitions
            or (
                not definitions[variant_id].display_name
                and not definitions[variant_id].variant_name
                and not definitions[variant_id].modules
            )
        )
    ]

    rejected = bool(collisions) and bool(unresolved_moduleless)
    proof = {
        "schema": "air_wings_variants_lifecycle_live_module_only_rejection_v1",
        "utc": datetime.now(timezone.utc).isoformat(),
        "stream_path": args.stream,
        "candidate": {
            "name": "module_token_multiset_only_archetype_base_airframe_closure",
            "status": "rejected" if rejected else "not_rejected",
            "reason": (
                "Active variants with identical module-token fingerprints have different "
                "current-live equipment/stat profiles, and at least one active variant has "
                "no module/name payload. Module tokens are useful evidence but cannot be the "
                "sole production key for exact archetype/base-airframe/stat closure."
            ),
        },
        "live_compact_coverage": {
            "active_superiority_variant_count": len(active_variant_ids),
            "active_variants_with_defs": sum(1 for vid in active_variant_ids if vid in definitions),
            "active_variants_with_stats": sum(1 for vid in active_variant_ids if vid in stats),
            "active_module_fingerprint_count": len(groups),
            "collision_group_count": len(collisions),
            "moduleless_or_nameless_active_count": len(unresolved_moduleless),
            "decode_error_count": len(stream["decode_errors"]),
            "decoded_domain_data_counts": stream["domain_data_counts"],
        },
        "collision_groups": collisions[: args.max_collisions],
        "moduleless_or_nameless_active_examples": unresolved_moduleless[: args.max_moduleless],
        "route_narrowing": {
            "required_live_inputs": [
                "air_variant_defs domain 33 for display/variant/module tokens",
                "stockpile_variant_defs domain 37 for stockpile-rooted variant definitions",
                "air_variant_stats domain 35 for current-live equipment/stat profile fields",
                "air_superiority_inputs domain 38 for active mission/variant coverage",
                "a same-session equipment-token/name registry or bounded static-catalog solver for final token names",
            ],
            "rejected_input": "module tokens alone",
            "shared_entrypoints_touched": False,
        },
        "gate": {
            "ok": rejected and len(stream["decode_errors"]) == 0,
            "problems": ([] if rejected else ["candidate_not_rejected"]) + (
                ["compact_stream_decode_errors"] if stream["decode_errors"] else []
            ),
            "decode_errors_sample": stream["decode_errors"][:12],
        },
    }
    return proof


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--stream", required=True)
    parser.add_argument("--out-json")
    parser.add_argument("--max-collisions", type=int, default=12)
    parser.add_argument("--examples-per-collision", type=int, default=8)
    parser.add_argument("--max-moduleless", type=int, default=8)
    args = parser.parse_args()
    proof = build_proof(args)
    if args.out_json:
        out_path = Path(args.out_json)
        out_path.parent.mkdir(parents=True, exist_ok=True)
        out_path.write_text(json.dumps(proof, indent=2, sort_keys=True), encoding="utf-8")
    print(json.dumps(proof["gate"], indent=2, sort_keys=True))
    return 0 if proof["gate"]["ok"] else 2


if __name__ == "__main__":
    raise SystemExit(main())
