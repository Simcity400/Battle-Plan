from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import residual_acceptance_gate as gate


def write_json(path: Path, data: dict) -> None:
    path.write_text(json.dumps(data), encoding="utf-8")


def minimal_proof() -> dict:
    return {
        "gates": {
            "compact_stream_decoded": True,
            "live_root_validated": True,
            "live_active_variant_token_offsets_validated": True,
            "active_compact_variant_defs_stats_closed": True,
            "same_session_token_dictionary_validated": True,
            "compact_active_base_airframe_names_closed": True,
            "same_session_direct_name_token_neighborhood_candidate_rejected": True,
            "wing_create_after_initial_observed": True,
            "wing_delete_observed": True,
            "wing_rebase_observed": True,
            "reserves_stockpile_variant_ids_cross_checked": True,
            "ace_join_current_compact_route_rejected": True,
            "ace_join_field_present": False,
            "compact_active_missing_catalog_tokens_badged_as_source_enum": True,
        },
        "ace_join_rejection_evidence": {
            "current_compact_air_domains": {
                "30_air_wings": "owner_tag_db_id, wing_id",
                "32_air_wing_loadouts": "owner_tag_db_id, wing_id, variant_id",
            },
            "rejection_scope": "current compact schemas only",
            "shared_collector_existing_ace_surface": "ace_count only",
        },
        "lifecycle_matrix": {
            "observed": {
                "air_wing_create_after_initial": True,
                "air_wing_delete": True,
                "air_wing_rebase": True,
            },
            "totals": {
                "air_wing_create_after_initial_rows": 2,
                "air_wing_deleted_rows": 1,
                "loadout_rebase_events": 3,
            },
            "delete_path_evidence": "deleted row tombstone",
        },
        "badges": {
            "direct_name_token_neighborhood_candidate": "rejected_bounded",
            "equipment_archetypes_catalog": "source_enum_badged",
        },
        "validated_roots_offsets_functions": {
            "StrategicAirManager": "CGameState+0x690",
        },
    }


def minimal_manifest() -> dict:
    return {
        "replay_file": "native_session.battleplan",
        "skipped_game_hours": 0,
        "hourly_requirement": {"met": True},
        "compact_stream": {
            "decodable": True,
            "ordered": True,
            "domain_coverage": {"ok": True},
            "source_unavailable_seen": False,
            "partial_data_seen": False,
            "bridge_unavailable_seen": False,
            "worker_panic_seen": False,
            "per_domain": {
                "air_variant_archetypes": {
                    "record_count": 10,
                    "data_record_count": 10,
                    "data_rows_total": 12,
                    "row_count_total": 120,
                    "data_units_total": {
                        "air_variant_archetype_rows": 12,
                    },
                },
            },
        },
    }


class ResidualAcceptanceGateTests(unittest.TestCase):
    def test_candidate_gate_passes_without_marking_lane_complete(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            proof_path = root / "proof.json"
            manifest_path = root / "native_session_manifest.json"
            (root / "native_session.battleplan").write_bytes(b"replay")
            write_json(proof_path, minimal_proof())
            write_json(manifest_path, minimal_manifest())

            acceptance = gate.build_acceptance(proof_path, manifest_path)

        self.assertEqual(acceptance["completion_status"], "NOT_COMPLETE")
        self.assertEqual(acceptance["candidate_rejection_gate_status"], "COMPLETE")
        self.assertEqual(acceptance["lane_full_production_status"], "NOT_COMPLETE")
        self.assertTrue(acceptance["backlog_reduced"])
        self.assertFalse(acceptance["partial_source_worker_bridge_failures"])
        self.assertEqual(
            acceptance["candidate_rejections"]["ace_join_from_current_compact_air_domains"]["status"],
            "rejected",
        )

    def test_not_complete_when_ace_field_is_not_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            proof = minimal_proof()
            proof["gates"]["ace_join_current_compact_route_rejected"] = False
            proof_path = root / "proof.json"
            manifest_path = root / "native_session_manifest.json"
            (root / "native_session.battleplan").write_bytes(b"replay")
            write_json(proof_path, proof)
            write_json(manifest_path, minimal_manifest())

            acceptance = gate.build_acceptance(proof_path, manifest_path)

        self.assertEqual(acceptance["completion_status"], "NOT_COMPLETE")
        self.assertIn("proof_gate_failed:ace_join_current_compact_route_rejected", acceptance["problems"])


if __name__ == "__main__":
    unittest.main()
