import unittest

from live_capture.collector_proof_gate import (
    collector_session_proof_errors,
    collector_session_proof_summary,
)


def _manifest(**overrides):
    manifest = {
        "session_errors": [],
        "replay_verified": True,
        "game_hours_advanced": 12,
        "stored_snapshot_count": 13,
        "hourly_requirement": {
            "met": True,
            "elapsed_wall_s": 11.5,
            "problems": [],
        },
        "collector_frame_payload_summary": {
            "domains_seen": [
                "clock",
                "country_identity",
                "country_scalars",
                "fuel_live",
                "production_live",
                "state_control",
            ],
            "scheduled_focus_domain_frames": 0,
            "scheduled_decisions_domain_frames": 0,
            "scheduled_projects_domain_frames": 0,
            "raw_delta_domains_seen": ["clock", "divisions"],
            "raw_event_types_seen": ["division_org_strength_delta"],
            "raw_event_type_counts": {"division_org_strength_delta": 3},
            "heavy_payload_on_lightweight_frames": 0,
        },
    }
    manifest.update(overrides)
    return manifest


class TestCollectorProofGate(unittest.TestCase):
    def test_accepts_speed_five_session_with_required_domains(self):
        proof = collector_session_proof_summary(
            _manifest(),
            required_snapshot_domains=["fuel_live", "state_control"],
            required_raw_delta_domains=["divisions"],
            required_raw_event_types=["division_org_strength_delta"],
            min_game_hours=12,
        )

        self.assertTrue(proof["ok"])
        self.assertEqual(proof["errors"], [])
        self.assertEqual(proof["hourly_elapsed_wall_s"], 11.5)
        self.assertEqual(
            proof["raw_event_type_counts"],
            {"division_org_strength_delta": 3},
        )

    def test_accepts_focus_decision_project_scheduled_domain_proof(self):
        proof = collector_session_proof_summary(
            _manifest(collector_frame_payload_summary={
                "domains_seen": [
                    "clock",
                    "focus_live",
                    "decisions_live",
                    "projects_live",
                ],
                "scheduled_focus_domain_frames": 3,
                "scheduled_decisions_domain_frames": 3,
                "scheduled_projects_domain_frames": 3,
                "heavy_payload_on_lightweight_frames": 0,
            }),
            required_scheduled_snapshot_domains=[
                "focus_live",
                "decisions_live",
                "projects_live",
            ],
            min_game_hours=2,
        )

        self.assertTrue(proof["ok"])
        self.assertEqual(proof["errors"], [])
        self.assertEqual(
            proof["scheduled_snapshot_domain_frames"]["decisions_live"],
            3,
        )

    def test_accepts_buildings_live_scheduled_domain_proof(self):
        proof = collector_session_proof_summary(
            _manifest(collector_frame_payload_summary={
                "domains_seen": ["clock", "buildings_live"],
                "scheduled_buildings_domain_frames": 3,
                "heavy_payload_on_lightweight_frames": 0,
            }),
            required_scheduled_snapshot_domains=["buildings_live"],
            min_game_hours=2,
        )

        self.assertTrue(proof["ok"])
        self.assertEqual(proof["errors"], [])
        self.assertEqual(
            proof["scheduled_snapshot_domain_frames"]["buildings_live"],
            3,
        )

    def test_accepts_diplomacy_live_scheduled_domain_proof(self):
        proof = collector_session_proof_summary(
            _manifest(collector_frame_payload_summary={
                "domains_seen": ["clock", "country_identity", "diplomacy_live"],
                "scheduled_diplomacy_domain_frames": 3,
                "heavy_payload_on_lightweight_frames": 0,
            }),
            required_scheduled_snapshot_domains=["diplomacy_live"],
            min_game_hours=2,
        )

        self.assertTrue(proof["ok"])
        self.assertEqual(proof["errors"], [])
        self.assertEqual(
            proof["scheduled_snapshot_domain_frames"]["diplomacy_live"],
            3,
        )

    def test_accepts_diplomacy_war_relation_proof(self):
        proof = collector_session_proof_summary(
            _manifest(
                war_relation_bridge_supplied=True,
                war_relation_bridge_rows=144,
                collector_frame_payload_summary={
                    "domains_seen": ["clock", "country_identity", "diplomacy_live"],
                    "scheduled_diplomacy_domain_frames": 12,
                    "diplomacy_war_relation_frames": 12,
                    "max_diplomacy_war_relations_per_frame": 144,
                    "max_diplomacy_casualty_sum_per_frame": 300,
                    "country_rows_with_diplomacy_at_war": 30,
                    "country_rows_with_diplomacy_casualties": 30,
                    "diplomacy_war_subtype_warning_frames": 0,
                    "diplomacy_on_lightweight_frames": 0,
                    "heavy_payload_on_lightweight_frames": 0,
                },
            ),
            required_scheduled_snapshot_domains=["diplomacy_live"],
            min_game_hours=12,
            require_diplomacy_war_relations=True,
        )

        self.assertTrue(proof["ok"])
        self.assertEqual(proof["errors"], [])
        self.assertEqual(proof["max_diplomacy_war_relations_per_frame"], 144)

    def test_accepts_politics_leader_proof(self):
        proof = collector_session_proof_summary(
            _manifest(
                collector_frame_payload_summary={
                    "domains_seen": ["clock", "country_identity", "politics_live"],
                    "scheduled_politics_domain_frames": 12,
                    "politics_leader_frames": 12,
                    "country_rows_with_politics_leader": 4260,
                    "max_politics_leader_rows_per_frame": 355,
                    "politics_on_lightweight_frames": 0,
                    "heavy_payload_on_lightweight_frames": 0,
                },
            ),
            required_scheduled_snapshot_domains=["politics_live"],
            min_game_hours=12,
            require_politics_leaders=True,
        )

        self.assertTrue(proof["ok"])
        self.assertEqual(proof["errors"], [])
        self.assertEqual(proof["politics_leader_frames"], 12)
        self.assertEqual(proof["max_politics_leader_rows_per_frame"], 355)

    def test_rejects_missing_politics_leaders(self):
        errors = collector_session_proof_errors(
            _manifest(
                collector_frame_payload_summary={
                    "domains_seen": ["clock", "country_identity", "politics_live"],
                    "scheduled_politics_domain_frames": 12,
                    "politics_leader_frames": 0,
                    "country_rows_with_politics_leader": 0,
                    "max_politics_leader_rows_per_frame": 0,
                    "politics_on_lightweight_frames": 0,
                    "heavy_payload_on_lightweight_frames": 0,
                },
            ),
            min_game_hours=12,
            require_politics_leaders=True,
        )

        self.assertIn("politics leader frames below required cadence", errors)
        self.assertIn("country politics leader rows were not observed", errors)

    def test_accepts_production_construction_proof(self):
        proof = collector_session_proof_summary(
            _manifest(
                collector_frame_payload_summary={
                    "domains_seen": ["clock", "country_identity", "production_live"],
                    "scheduled_production_domain_frames": 12,
                    "construction_queue_frames": 12,
                    "country_rows_with_construction_queue": 504,
                    "construction_queue_rows": 812,
                    "max_construction_queue_rows_per_frame": 72,
                    "max_construction_queue_rows_per_country": 9,
                    "production_on_lightweight_frames": 0,
                    "heavy_payload_on_lightweight_frames": 0,
                },
            ),
            required_scheduled_snapshot_domains=["production_live"],
            min_game_hours=12,
            require_production_construction=True,
        )

        self.assertTrue(proof["ok"])
        self.assertEqual(proof["errors"], [])
        self.assertEqual(proof["construction_queue_frames"], 12)
        self.assertEqual(proof["construction_queue_rows"], 812)

    def test_rejects_missing_production_construction_rows(self):
        errors = collector_session_proof_errors(
            _manifest(
                collector_frame_payload_summary={
                    "domains_seen": ["clock", "country_identity", "production_live"],
                    "scheduled_production_domain_frames": 12,
                    "construction_queue_frames": 0,
                    "country_rows_with_construction_queue": 0,
                    "construction_queue_rows": 0,
                    "max_construction_queue_rows_per_frame": 0,
                    "production_on_lightweight_frames": 0,
                    "heavy_payload_on_lightweight_frames": 0,
                },
            ),
            min_game_hours=12,
            require_production_construction=True,
        )

        self.assertIn("construction queue frames below required cadence", errors)
        self.assertIn("construction queue line rows were not observed", errors)

    def test_accepts_tech_live_names_proof(self):
        proof = collector_session_proof_summary(
            _manifest(
                collector_frame_payload_summary={
                    "domains_seen": ["clock", "country_identity", "tech_live"],
                    "scheduled_tech_domain_frames": 12,
                    "frames_with_research_payloads": 12,
                    "country_rows_with_research_payloads": 4272,
                    "max_research_active_rows_per_country": 5,
                    "research_named_slot_frames": 12,
                    "research_named_slot_rows": 600,
                    "country_rows_with_research_named_slots": 480,
                    "max_research_named_slot_rows_per_frame": 88,
                    "max_research_named_slot_rows_per_country": 5,
                    "tech_on_lightweight_frames": 0,
                    "heavy_payload_on_lightweight_frames": 0,
                },
            ),
            required_scheduled_snapshot_domains=["tech_live"],
            min_game_hours=12,
            require_tech_live_names=True,
        )

        self.assertTrue(proof["ok"])
        self.assertEqual(proof["errors"], [])
        self.assertEqual(proof["research_named_slot_rows"], 600)

    def test_rejects_missing_tech_live_named_rows(self):
        errors = collector_session_proof_errors(
            _manifest(
                collector_frame_payload_summary={
                    "domains_seen": ["clock", "country_identity", "tech_live"],
                    "scheduled_tech_domain_frames": 12,
                    "frames_with_research_payloads": 12,
                    "country_rows_with_research_payloads": 4272,
                    "max_research_active_rows_per_country": 5,
                    "research_named_slot_frames": 0,
                    "research_named_slot_rows": 0,
                    "country_rows_with_research_named_slots": 0,
                    "max_research_named_slot_rows_per_frame": 0,
                    "tech_on_lightweight_frames": 0,
                    "heavy_payload_on_lightweight_frames": 0,
                },
            ),
            min_game_hours=12,
            require_tech_live_names=True,
        )

        self.assertIn("research named-slot frames below required cadence", errors)
        self.assertIn("research named-slot rows were not observed", errors)

    def test_accepts_tech_live_progress_proof(self):
        proof = collector_session_proof_summary(
            _manifest(
                game_hours_advanced=36,
                collector_frame_payload_summary={
                    "domains_seen": ["clock", "country_identity", "tech_live"],
                    "scheduled_tech_domain_frames": 36,
                    "frames_with_research_payloads": 36,
                    "country_rows_with_research_payloads": 12816,
                    "max_research_active_rows_per_country": 5,
                    "research_live_progress_slot_frames": 36,
                    "research_live_progress_slot_rows": 15384,
                    "country_rows_with_research_live_progress_slots": 4248,
                    "max_research_live_progress_rows_per_frame": 428,
                    "max_research_live_progress_rows_per_country": 5,
                    "tech_on_lightweight_frames": 0,
                    "heavy_payload_on_lightweight_frames": 0,
                },
                recording_diagnostics={
                    "collector": {
                        "research_live_progress": {
                            "live_progress_frames": 36,
                            "live_progress_rows": 15384,
                            "speed_frames": 35,
                            "speed_rows": 12000,
                            "eta_frames": 0,
                            "eta_rows": 0,
                            "max_live_progress_rows_per_frame": 428,
                            "max_speed_rows_per_frame": 400,
                            "max_eta_rows_per_frame": 0,
                            "max_points_factor_milli": 2500,
                        },
                    },
                },
            ),
            required_scheduled_snapshot_domains=["tech_live"],
            min_game_hours=36,
            require_tech_live_progress=True,
        )

        self.assertTrue(proof["ok"])
        self.assertEqual(proof["errors"], [])
        self.assertEqual(proof["research_live_progress"]["speed_rows"], 12000)

    def test_rejects_tech_live_progress_without_moving_rows(self):
        errors = collector_session_proof_errors(
            _manifest(
                game_hours_advanced=36,
                collector_frame_payload_summary={
                    "domains_seen": ["clock", "country_identity", "tech_live"],
                    "scheduled_tech_domain_frames": 36,
                    "frames_with_research_payloads": 36,
                    "country_rows_with_research_payloads": 12816,
                    "max_research_active_rows_per_country": 5,
                    "research_live_progress_slot_frames": 36,
                    "research_live_progress_slot_rows": 15384,
                    "country_rows_with_research_live_progress_slots": 4248,
                    "max_research_live_progress_rows_per_frame": 428,
                    "tech_on_lightweight_frames": 0,
                    "heavy_payload_on_lightweight_frames": 0,
                },
                recording_diagnostics={
                    "collector": {
                        "research_live_progress": {
                            "live_progress_frames": 36,
                            "live_progress_rows": 15384,
                            "speed_frames": 0,
                            "speed_rows": 0,
                        },
                    },
                },
            ),
            min_game_hours=36,
            require_tech_live_progress=True,
        )

        self.assertIn("research live progress did not move between frames", errors)
        self.assertIn("research live progress moving rows were not observed", errors)

    def test_accepts_tech_live_static_cost_progress_proof(self):
        proof = collector_session_proof_summary(
            _manifest(
                game_hours_advanced=36,
                collector_frame_payload_summary={
                    "domains_seen": ["clock", "country_identity", "tech_live"],
                    "scheduled_tech_domain_frames": 36,
                    "frames_with_research_payloads": 36,
                    "country_rows_with_research_payloads": 12816,
                    "max_research_active_rows_per_country": 5,
                    "tech_on_lightweight_frames": 0,
                    "heavy_payload_on_lightweight_frames": 0,
                },
                recording_diagnostics={
                    "collector": {
                        "research_live_progress": {
                            "static_cost_progress_frames": 36,
                            "static_cost_progress_rows": 12000,
                            "max_static_cost_progress_rows_per_frame": 400,
                            "progress_unavailable_reasons": {
                                "ahead_of_time_effective_cost_unavailable": 20,
                            },
                        },
                    },
                },
            ),
            required_scheduled_snapshot_domains=["tech_live"],
            min_game_hours=36,
            require_tech_live_static_cost_progress=True,
        )

        self.assertTrue(proof["ok"])
        self.assertEqual(proof["errors"], [])
        self.assertEqual(
            proof["research_live_progress"]["static_cost_progress_rows"],
            12000,
        )

    def test_rejects_missing_tech_live_static_cost_progress_rows(self):
        errors = collector_session_proof_errors(
            _manifest(
                collector_frame_payload_summary={
                    "domains_seen": ["clock", "country_identity", "tech_live"],
                    "scheduled_tech_domain_frames": 36,
                    "frames_with_research_payloads": 36,
                    "country_rows_with_research_payloads": 12816,
                    "max_research_active_rows_per_country": 5,
                    "tech_on_lightweight_frames": 0,
                    "heavy_payload_on_lightweight_frames": 0,
                },
                recording_diagnostics={
                    "collector": {
                        "research_live_progress": {
                            "static_cost_progress_frames": 0,
                            "static_cost_progress_rows": 0,
                            "max_static_cost_progress_rows_per_frame": 0,
                        },
                    },
                },
            ),
            min_game_hours=36,
            require_tech_live_static_cost_progress=True,
        )

        self.assertIn(
            "research static-cost progress frames below required cadence",
            errors,
        )
        self.assertIn(
            "research static-cost progress rows were not observed",
            errors,
        )

    def test_accepts_tech_live_dll_base_progress_proof(self):
        proof = collector_session_proof_summary(
            _manifest(
                game_hours_advanced=36,
                collector_frame_payload_summary={
                    "domains_seen": ["clock", "country_identity", "tech_live"],
                    "scheduled_tech_domain_frames": 36,
                    "frames_with_research_payloads": 36,
                    "country_rows_with_research_payloads": 12816,
                    "max_research_active_rows_per_country": 5,
                    "tech_on_lightweight_frames": 0,
                    "heavy_payload_on_lightweight_frames": 0,
                },
                recording_diagnostics={
                    "collector": {
                        "research_live_progress": {
                            "dll_base_required_progress_frames": 36,
                            "dll_base_required_progress_rows": 12000,
                            "max_dll_base_required_progress_rows_per_frame": 400,
                        },
                    },
                },
            ),
            required_scheduled_snapshot_domains=["tech_live"],
            min_game_hours=36,
            require_tech_live_dll_base_progress=True,
        )

        self.assertTrue(proof["ok"])
        self.assertEqual(proof["errors"], [])
        self.assertEqual(
            proof["research_live_progress"]["dll_base_required_progress_rows"],
            12000,
        )

    def test_rejects_missing_tech_live_dll_base_progress_rows(self):
        errors = collector_session_proof_errors(
            _manifest(
                collector_frame_payload_summary={
                    "domains_seen": ["clock", "country_identity", "tech_live"],
                    "scheduled_tech_domain_frames": 36,
                    "frames_with_research_payloads": 36,
                    "country_rows_with_research_payloads": 12816,
                    "max_research_active_rows_per_country": 5,
                    "tech_on_lightweight_frames": 0,
                    "heavy_payload_on_lightweight_frames": 0,
                },
                recording_diagnostics={
                    "collector": {
                        "research_live_progress": {
                            "dll_base_required_progress_frames": 0,
                            "dll_base_required_progress_rows": 0,
                            "max_dll_base_required_progress_rows_per_frame": 0,
                        },
                    },
                },
            ),
            min_game_hours=36,
            require_tech_live_dll_base_progress=True,
        )

        self.assertIn(
            "research DLL-base progress frames below required cadence",
            errors,
        )
        self.assertIn(
            "research DLL-base progress rows were not observed",
            errors,
        )

    def test_accepts_tech_live_eta_proof(self):
        proof = collector_session_proof_summary(
            _manifest(
                game_hours_advanced=36,
                collector_frame_payload_summary={
                    "domains_seen": ["clock", "country_identity", "tech_live"],
                    "scheduled_tech_domain_frames": 36,
                    "frames_with_research_payloads": 36,
                    "country_rows_with_research_payloads": 12816,
                    "max_research_active_rows_per_country": 5,
                    "tech_on_lightweight_frames": 0,
                    "heavy_payload_on_lightweight_frames": 0,
                },
                recording_diagnostics={
                    "collector": {
                        "research_live_progress": {
                            "speed_frames": 4,
                            "speed_rows": 900,
                            "eta_frames": 4,
                            "eta_rows": 700,
                            "max_speed_rows_per_frame": 300,
                            "max_eta_rows_per_frame": 240,
                            "max_points_factor_milli": 2500,
                        },
                    },
                },
            ),
            required_scheduled_snapshot_domains=["tech_live"],
            min_game_hours=36,
            require_tech_live_eta=True,
        )

        self.assertTrue(proof["ok"])
        self.assertEqual(proof["errors"], [])
        self.assertEqual(proof["research_live_progress"]["eta_rows"], 700)

    def test_rejects_missing_tech_live_eta_rows(self):
        errors = collector_session_proof_errors(
            _manifest(
                collector_frame_payload_summary={
                    "domains_seen": ["clock", "country_identity", "tech_live"],
                    "scheduled_tech_domain_frames": 36,
                    "frames_with_research_payloads": 36,
                    "country_rows_with_research_payloads": 12816,
                    "max_research_active_rows_per_country": 5,
                    "tech_on_lightweight_frames": 0,
                    "heavy_payload_on_lightweight_frames": 0,
                },
                recording_diagnostics={
                    "collector": {
                        "research_live_progress": {
                            "speed_frames": 0,
                            "speed_rows": 0,
                            "eta_frames": 0,
                            "eta_rows": 0,
                        },
                    },
                },
            ),
            min_game_hours=36,
            require_tech_live_eta=True,
        )

        self.assertIn("research live speed rows were not observed", errors)
        self.assertIn("research live ETA rows were not observed", errors)

    def test_rejects_faction_only_diplomacy_for_war_relation_proof(self):
        errors = collector_session_proof_errors(
            _manifest(
                war_relation_bridge_supplied=False,
                war_relation_bridge_rows=0,
                collector_frame_payload_summary={
                    "domains_seen": ["clock", "country_identity", "diplomacy_live"],
                    "scheduled_diplomacy_domain_frames": 12,
                    "diplomacy_war_relation_frames": 0,
                    "max_diplomacy_war_relations_per_frame": 0,
                    "max_diplomacy_casualty_sum_per_frame": 0,
                    "country_rows_with_diplomacy_at_war": 0,
                    "country_rows_with_diplomacy_casualties": 0,
                    "diplomacy_war_subtype_warning_frames": 0,
                    "diplomacy_on_lightweight_frames": 0,
                    "heavy_payload_on_lightweight_frames": 0,
                },
            ),
            min_game_hours=12,
            require_diplomacy_war_relations=True,
        )

        self.assertIn("war_relation_bridge was not supplied", errors)
        self.assertIn("diplomacy war relations were not observed", errors)

    def test_domains_seen_does_not_satisfy_scheduled_domain_gate(self):
        errors = collector_session_proof_errors(
            _manifest(collector_frame_payload_summary={
                "domains_seen": ["clock", "decisions_live"],
                "scheduled_decisions_domain_frames": 0,
                "raw_delta_domains_seen": [],
                "heavy_payload_on_lightweight_frames": 0,
            }),
            required_snapshot_domains=["decisions_live"],
            required_scheduled_snapshot_domains=["decisions_live"],
        )

        self.assertEqual(
            errors,
            ["scheduled snapshot domain 'decisions_live' not observed"],
        )

    def test_reports_session_hourly_replay_and_domain_failures(self):
        errors = collector_session_proof_errors(
            _manifest(
                session_errors=["attach_errors: pipe closed"],
                replay_verified=False,
                game_hours_advanced=3,
                hourly_requirement={
                    "met": False,
                    "problems": ["skipped_game_hours"],
                },
                collector_frame_payload_summary={
                    "domains_seen": ["clock"],
                    "raw_delta_domains_seen": [],
                    "raw_event_types_seen": [],
                    "raw_event_type_counts": {},
                    "heavy_payload_on_lightweight_frames": 2,
                },
            ),
            required_snapshot_domains=["production_live"],
            required_raw_delta_domains=["land_combats"],
            required_raw_event_types=["army_order_updated"],
            min_game_hours=12,
        )

        joined = "\n".join(errors)
        self.assertIn("session_errors present", joined)
        self.assertIn("replay_verified is not true", joined)
        self.assertIn("game_hours_advanced 3 below required 12", joined)
        self.assertIn("hourly_requirement not met", joined)
        self.assertIn("snapshot domain 'production_live' not observed", joined)
        self.assertIn("raw delta domain 'land_combats' not observed", joined)
        self.assertIn("raw event type 'army_order_updated' not observed", joined)
        self.assertIn("heavy payloads appeared on lightweight frames: 2", joined)

    def test_missing_summary_fails_required_domains(self):
        errors = collector_session_proof_errors(
            _manifest(collector_frame_payload_summary=None),
            required_snapshot_domains=["country_scalars"],
            required_raw_delta_domains=["divisions"],
        )

        self.assertEqual(
            errors,
            [
                "snapshot domain 'country_scalars' not observed",
                "raw delta domain 'divisions' not observed",
            ],
        )


if __name__ == "__main__":
    unittest.main()
