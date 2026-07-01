"""Lane-local residual acceptance gate for domain 46 air archetype work.

This gate consumes the heavier live proof plus the integrated compact-session
manifest and emits a compact acceptance artifact. It is deliberately scoped to
candidate rejection / backlog narrowing; it does not claim full air lifecycle
completion.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
from pathlib import Path
from typing import Any


REQUIRED_PROOF_GATES = (
    "compact_stream_decoded",
    "live_root_validated",
    "live_active_variant_token_offsets_validated",
    "active_compact_variant_defs_stats_closed",
    "same_session_token_dictionary_validated",
    "compact_active_base_airframe_names_closed",
    "same_session_direct_name_token_neighborhood_candidate_rejected",
    "wing_create_after_initial_observed",
    "wing_delete_observed",
    "wing_rebase_observed",
    "reserves_stockpile_variant_ids_cross_checked",
    "ace_join_current_compact_route_rejected",
)


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def nested_bool(data: dict[str, Any], *keys: str) -> bool:
    cur: Any = data
    for key in keys:
        if not isinstance(cur, dict):
            return False
        cur = cur.get(key)
    return bool(cur)


def replay_exists(manifest_path: Path, manifest: dict[str, Any]) -> bool:
    replay_file = manifest.get("replay_file")
    if not isinstance(replay_file, str) or not replay_file:
        return False
    replay_path = Path(replay_file)
    if not replay_path.is_absolute():
        replay_path = manifest_path.parent / replay_path
    return replay_path.exists()


def compact_manifest_ok(manifest_path: Path, manifest: dict[str, Any]) -> tuple[bool, dict[str, Any]]:
    compact = manifest.get("compact_stream", {})
    checks = {
        "decodable": bool(compact.get("decodable")),
        "ordered": bool(compact.get("ordered")),
        "domain_coverage_ok": nested_bool(compact, "domain_coverage", "ok"),
        "replay_file_exists": replay_exists(manifest_path, manifest),
        "hourly_requirement_met": nested_bool(manifest, "hourly_requirement", "met"),
        "zero_skipped_hours": manifest.get("skipped_game_hours") == 0,
        "no_source_unavailable": not bool(compact.get("source_unavailable_seen")),
        "no_partial_data": not bool(compact.get("partial_data_seen")),
        "no_bridge_unavailable": not bool(compact.get("bridge_unavailable_seen")),
        "no_worker_panic": not bool(compact.get("worker_panic_seen")),
    }
    return all(checks.values()), checks


def proof_gate_status(proof: dict[str, Any]) -> tuple[bool, dict[str, bool]]:
    gates = proof.get("gates", {})
    statuses = {name: bool(gates.get(name)) for name in REQUIRED_PROOF_GATES}
    statuses["ace_join_field_absent"] = not bool(gates.get("ace_join_field_present"))
    statuses["compact_active_missing_catalog_tokens_badged"] = bool(
        gates.get("compact_active_missing_catalog_tokens_badged_as_source_enum")
    )
    return all(statuses.values()), statuses


def build_acceptance(proof_path: Path, manifest_path: Path) -> dict[str, Any]:
    proof = load_json(proof_path)
    manifest = load_json(manifest_path)
    proof_ok, proof_checks = proof_gate_status(proof)
    manifest_ok, manifest_checks = compact_manifest_ok(manifest_path, manifest)
    problems: list[str] = []
    if not proof_ok:
        problems.extend([f"proof_gate_failed:{key}" for key, value in proof_checks.items() if not value])
    if not manifest_ok:
        problems.extend([f"manifest_gate_failed:{key}" for key, value in manifest_checks.items() if not value])

    current_air_domains = proof.get("ace_join_rejection_evidence", {}).get(
        "current_compact_air_domains", {}
    )
    lifecycle_observed = proof.get("lifecycle_matrix", {}).get("observed", {})
    compact_domain46 = manifest.get("compact_stream", {}).get("per_domain", {}).get(
        "air_variant_archetypes", {}
    )
    compact_domain46_units = compact_domain46.get("data_units_total", {})

    candidate_gate_status = "COMPLETE" if not problems else "NOT_COMPLETE"

    return {
        "schema": "air_variant_archetypes_live_residual_acceptance_v1",
        "generated_utc": dt.datetime.now(dt.UTC).replace(microsecond=0).isoformat(),
        "completion_status": "NOT_COMPLETE",
        "candidate_rejection_gate_status": candidate_gate_status,
        "lane_full_production_status": "NOT_COMPLETE",
        "backlog_reduced": not problems,
        "exact_backlog_item_removed_or_narrowed": (
            "Rejected ace identity/wing-assignment joins through the current compact air domains "
            "30/31/32/36/38 because those schemas expose wing/loadout/activity/reserve/superiority "
            "rows but no ace id, ace name, character pointer, or assigned wing field. The live proof "
            "also closes active variant defs/stats and observes wing create/delete/rebase through "
            "existing non-domain-42 air lifecycle domains, so these residuals should not be re-added "
            "through the current compact air schemas. Direct ace identity/wing assignment roots remain "
            "a separate residual until captured or blocked."
        ),
        "input_proof": str(proof_path),
        "input_manifest": str(manifest_path),
        "problems": problems,
        "proof_checks": proof_checks,
        "manifest_checks": manifest_checks,
        "domain": {
            "id": 46,
            "name": "air_variant_archetypes_live",
            "accepted_scope": "validated slice: air variant archetype classification plus residual candidate rejection",
            "not_full_scope": "full air lifecycle remains open",
        },
        "compact_domain46_manifest": {
            "record_count": compact_domain46.get("record_count"),
            "data_record_count": compact_domain46.get("data_record_count"),
            "data_rows_total": compact_domain46.get("data_rows_total"),
            "row_count_total": compact_domain46.get("row_count_total"),
            "data_units_total": compact_domain46_units,
            "read_time_us": compact_domain46.get("read_time_us"),
        },
        "validated_roots_offsets_functions": proof.get("validated_roots_offsets_functions", {}),
        "candidate_rejections": {
            "ace_join_from_current_compact_air_domains": {
                "status": "rejected" if proof_checks["ace_join_current_compact_route_rejected"] else "not_rejected",
                "field_present": proof.get("gates", {}).get("ace_join_field_present"),
                "schemas_checked": current_air_domains,
                "scope": proof.get("ace_join_rejection_evidence", {}).get("rejection_scope"),
                "existing_ace_surface": proof.get("ace_join_rejection_evidence", {}).get(
                    "shared_collector_existing_ace_surface"
                ),
            },
            "direct_ascii_name_near_u32_token_registry": proof.get("badges", {}).get(
                "direct_name_token_neighborhood_candidate"
            ),
            "equipment_archetypes_catalog_for_source_enum_airframe_tokens": proof.get(
                "badges", {}
            ).get("equipment_archetypes_catalog"),
        },
        "lifecycle_evidence_not_domain42": {
            "observed": lifecycle_observed,
            "create_rows": proof.get("lifecycle_matrix", {}).get("totals", {}).get(
                "air_wing_create_after_initial_rows"
            ),
            "delete_rows": proof.get("lifecycle_matrix", {}).get("totals", {}).get(
                "air_wing_deleted_rows"
            ),
            "rebase_events": proof.get("lifecycle_matrix", {}).get("totals", {}).get(
                "loadout_rebase_events"
            ),
            "delete_path_evidence": proof.get("lifecycle_matrix", {}).get(
                "delete_path_evidence"
            ),
        },
        "compact_schema_changes": (
            "No new compact schema fields in this acceptance gate. Existing domain 46 schema remains "
            "variant id, equipment token/type/interface raw fields, optional air-superiority weight, "
            "source mask, and archetype/base-airframe/role strings. Ace fields are rejected for the "
            "current compact air-domain route."
        ),
        "rust_decoder_gate_changes": (
            "No shared Rust recorder changes. Existing lane Rust decoder/gate remains the domain 46 "
            "decoder; this Python gate is lane-local acceptance evidence."
        ),
        "dll_only": False,
        "dll_only_detail": (
            "The integrated compact stream is DLL-only; this acceptance artifact also consumes a "
            "lane-local Python live/static proof."
        ),
        "compact_stream": True,
        "rust_decoded": bool(manifest_checks["decodable"]),
        "hoi4replay_verified": bool(manifest_checks["replay_file_exists"]),
        "speed_five_current_equivalent_hourly_or_lower": bool(manifest_checks["hourly_requirement_met"]),
        "zero_skipped_hours": bool(manifest_checks["zero_skipped_hours"]),
        "partial_source_worker_bridge_failures": not (
            manifest_checks["no_source_unavailable"]
            and manifest_checks["no_partial_data"]
            and manifest_checks["no_bridge_unavailable"]
            and manifest_checks["no_worker_panic"]
        ),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--proof", required=True)
    parser.add_argument("--manifest", required=True)
    parser.add_argument("--out-dir", required=True)
    args = parser.parse_args()

    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    acceptance = build_acceptance(Path(args.proof), Path(args.manifest))
    out_path = out_dir / "air_variant_archetypes_live_residual_acceptance.json"
    out_path.write_text(json.dumps(acceptance, indent=2, sort_keys=True), encoding="utf-8")
    print(
        json.dumps(
            {
                "out": str(out_path),
                "completion_status": acceptance["completion_status"],
                "candidate_rejection_gate_status": acceptance["candidate_rejection_gate_status"],
                "lane_full_production_status": acceptance["lane_full_production_status"],
                "backlog_reduced": acceptance["backlog_reduced"],
                "problems": acceptance["problems"],
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0 if acceptance["candidate_rejection_gate_status"] == "COMPLETE" else 1


if __name__ == "__main__":
    raise SystemExit(main())
