import json
import hashlib
import os
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))

from live_capture.collector_frames import validate_collector_frame  # noqa: E402
from live_capture.collector_session import (  # noqa: E402
    CollectorFrameSession,
    record_collector_session,
)
import live_capture.collector_session as cs  # noqa: E402
from live_capture.replay_writer import _verify_container, _zip_read_all  # noqa: E402


class TestCollectorSession(unittest.TestCase):
    def test_process_instance_id_uses_exe_hash_pid_and_create_time(self):
        original_create_time = cs._process_identity._process_create_time_100ns
        try:
            cs._process_identity._process_create_time_100ns = lambda pid: 123456789
            instance_id = cs._collector_process_instance_id(
                777,
                [{"exe_sha256": "a" * 64}],
                None,
            )
        finally:
            cs._process_identity._process_create_time_100ns = original_create_time

        expected = hashlib.sha256(
            (("a" * 64) + "|777|123456789").encode("utf-8")
        ).hexdigest()[:16]
        self.assertEqual(instance_id, expected)

    def test_collector_frame_payload_summary_counts_map_rows(self):
        summary = cs._collector_frame_payload_summary([
            {
                "domains": ["clock", "state_control", "province_control"],
                "countries": {"GER": {}, "ENG": {}},
                "states": {"1": {"controller": "GER"}},
                "provinces": {
                    "11": {"controller": "GER"},
                    "12": {"controller": "ENG"},
                },
            },
            {
                "domains": ["clock", "state_ownership"],
                "countries": {"GER": {}},
                "states": {
                    "1": {"owner": "GER"},
                    "2": {"owner": "ENG"},
                },
            },
        ])

        self.assertEqual(
            summary["domains_seen"],
            ["clock", "province_control", "state_control", "state_ownership"],
        )
        self.assertEqual(summary["frames_with_countries"], 2)
        self.assertEqual(summary["frames_with_states"], 2)
        self.assertEqual(summary["frames_with_provinces"], 1)
        self.assertEqual(summary["max_country_rows"], 2)
        self.assertEqual(summary["max_state_rows"], 2)
        self.assertEqual(summary["max_province_rows"], 2)
        self.assertEqual(summary["state_control_frames"], 1)
        self.assertEqual(summary["province_control_frames"], 1)
        self.assertEqual(summary["state_ownership_frames"], 1)
        self.assertEqual(summary["country_scalars_frames"], 0)
        self.assertEqual(summary["country_rows_with_scalar_payloads"], 0)

    def test_collector_frame_payload_summary_counts_country_scalar_source(self):
        summary = cs._collector_frame_payload_summary([
            {
                "domains": ["clock", "country_identity"],
                "root_probe": {
                    "country_object_identity": {
                        "status": "country_database_root_insufficient",
                        "object_count": 0,
                        "scan_ms": 15,
                    },
                },
                "countries": {
                    "GER": {"present": True, "identity": {"tag_db_id": 1}},
                },
            },
            {
                "domains": [
                    "clock",
                    "country_identity",
                    "country_scalars",
                    "country_misc",
                    "fuel_live",
                ],
                "root_probe": {
                    "country_object_identity": {
                        "status": "country_database_root_verified",
                        "object_count": 88,
                        "scan_ms": 3,
                    },
                },
                "countries": {
                    "POR": {
                        "collector_object": "0x10000001000",
                        "identity": {"tag_db_id": 47},
                        "scalars": {
                            "stability": 0.61,
                            "war_support": None,
                            "political_power": None,
                        },
                        "misc": {"ace_count": 1},
                        "fuel": {"current": 10.0, "max": 20.0},
                    },
                    "ENG": {
                        "collector_object": "0x10000002000",
                        "identity": {"tag_db_id": 2},
                        "scalars": {
                            "stability": None,
                            "war_support": None,
                        },
                    },
                },
            },
        ])

        self.assertEqual(summary["country_scalars_frames"], 1)
        self.assertEqual(summary["country_misc_frames"], 1)
        self.assertEqual(summary["fuel_live_frames"], 1)
        self.assertEqual(summary["country_rows_with_scalar_payloads"], 2)
        self.assertEqual(summary["country_rows_with_non_null_scalar_values"], 1)
        self.assertEqual(summary["max_country_scalar_rows_per_frame"], 2)
        self.assertEqual(summary["country_object_identity_statuses"], {
            "country_database_root_insufficient": 1,
            "country_database_root_verified": 1,
        })
        self.assertEqual(summary["country_object_identity_max_object_count"], 88)
        self.assertEqual(summary["country_object_identity_max_scan_ms"], 15)

    def test_map_control_quality_counts_static_province_frames_without_preflight(self):
        quality = cs._map_control_quality_summary(
            state_control_preflight={"enabled": False},
            province_control_preflight={"enabled": False},
            probe_manifest={},
            payload_summary={
                "scheduled_snapshot_frame_count": 3,
                "state_ownership_frames": 0,
                "state_control_frames": 0,
                "province_control_frames": 3,
                "heavy_payload_on_lightweight_frames": 0,
            },
        )

        self.assertEqual(quality["requested_domains"], [])
        self.assertEqual(quality["captured_domains"], ["province_control"])
        self.assertEqual(quality["omitted_domains"], {})
        self.assertFalse(quality["fail_closed"])

    def test_collector_frame_payload_summary_counts_production_live_coverage(self):
        summary = cs._collector_frame_payload_summary([
            {
                "capture_boundary": "scheduled_hour_snapshot",
                "domains": ["clock", "production_live"],
                "countries": {
                    "GER": {
                        "production": {
                            "stockpile": {"70": 12.0, "71": 0.0},
                            "military_lines": [{"id": 1}],
                            "naval_lines": [{"id": 2}, {"id": 3}],
                        },
                    },
                    "ENG": {
                        "production": {
                            "stockpile": {},
                            "military_lines": [],
                            "naval_lines": [],
                        },
                    },
                    "FRA": {},
                },
            },
            {
                "capture_boundary": "scheduled_hour_snapshot",
                "domains": ["clock", "production_live"],
                "countries": {
                    "SOV": {
                        "production": {
                            "stockpile": {"80": 1.0, "81": 2.0, "82": 3.0},
                            "military_lines": [{"id": 4}, {"id": 5}],
                            "naval_lines": [{"id": 6}],
                        },
                    },
                },
            },
            {
                "capture_boundary": "scheduled_hour_snapshot",
                "domains": ["clock", "production_live"],
                "countries": {"USA": {}},
            },
            {
                "capture_boundary": "worker_poll",
                "domains": ["clock"],
                "countries": {
                    "ITA": {
                        "production": {
                            "stockpile": {"90": 4.0},
                            "military_lines": [],
                            "naval_lines": [],
                        },
                    },
                },
            },
        ])

        self.assertEqual(summary["domains_seen"], ["clock", "production_live"])
        self.assertEqual(summary["scheduled_snapshot_frame_count"], 3)
        self.assertEqual(summary["lightweight_poll_frame_count"], 1)
        self.assertEqual(summary["scheduled_production_domain_frames"], 3)
        self.assertEqual(summary["frames_with_production_payloads"], 3)
        self.assertEqual(summary["country_rows_with_production_payloads"], 4)
        self.assertEqual(summary["max_production_country_rows_per_frame"], 2)
        self.assertEqual(summary["max_production_stockpile_entries"], 3)
        self.assertEqual(summary["max_production_military_line_rows"], 2)
        self.assertEqual(summary["max_production_naval_line_rows"], 2)
        self.assertEqual(summary["production_on_lightweight_frames"], 1)
        self.assertEqual(summary["heavy_payload_on_lightweight_frames"], 1)

    def test_collector_frame_payload_summary_counts_focus_live_coverage(self):
        summary = cs._collector_frame_payload_summary([
            {
                "capture_boundary": "scheduled_hour_snapshot",
                "domains": ["clock", "focus_live"],
                "countries": {
                    "GER": {
                        "focus": {
                            "focus_tree": "german_focus",
                            "completed": ["GER_autarky", "GER_west_wall"],
                            "completed_count": 2,
                            "current": "GER_air_innovation",
                        },
                    },
                    "SOV": {
                        "focus": {
                            "focus_tree": "soviet_focus",
                            "completed": [],
                            "completed_count": 0,
                            "current_continuous": "continuous_repair",
                        },
                    },
                },
            },
            {
                "capture_boundary": "worker_poll",
                "domains": ["clock"],
                "countries": {
                    "ITA": {
                        "focus": {
                            "focus_tree": "italy_focus",
                            "completed": ["ITA_army_effort"],
                            "completed_count": 1,
                        },
                    },
                },
            },
        ])

        self.assertEqual(summary["domains_seen"], ["clock", "focus_live"])
        self.assertEqual(summary["focus_live_frames"], 1)
        self.assertEqual(summary["scheduled_focus_domain_frames"], 1)
        self.assertEqual(summary["frames_with_focus_payloads"], 2)
        self.assertEqual(summary["country_rows_with_focus_payloads"], 3)
        self.assertEqual(summary["max_focus_country_rows_per_frame"], 2)
        self.assertEqual(summary["max_focus_completed_rows_per_country"], 2)
        self.assertEqual(summary["focus_on_lightweight_frames"], 1)
        self.assertEqual(summary["heavy_payload_on_lightweight_frames"], 1)

    def test_collector_frame_payload_summary_counts_tech_live_coverage(self):
        summary = cs._collector_frame_payload_summary([
            {
                "capture_boundary": "scheduled_hour_snapshot",
                "domains": ["clock", "tech_live"],
                "countries": {
                    "GER": {
                        "research": {
                            "completed_tech_count": 120,
                            "research_slot_count": 4,
                            "active_research": [
                                {"slot_index": 0, "tech_handle_raw": 1},
                                {"slot_index": 1, "tech_handle_raw": 2},
                            ],
                        },
                    },
                    "SOV": {
                        "research": {
                            "completed_tech_count": 0,
                            "research_slot_count": 3,
                            "active_research": [],
                        },
                    },
                },
            },
            {
                "capture_boundary": "worker_poll",
                "domains": ["clock"],
                "countries": {
                    "ITA": {
                        "research": {
                            "completed_tech_count": 18,
                            "research_slot_count": 4,
                        },
                    },
                },
            },
        ])

        self.assertEqual(summary["domains_seen"], ["clock", "tech_live"])
        self.assertEqual(summary["tech_live_frames"], 1)
        self.assertEqual(summary["scheduled_tech_domain_frames"], 1)
        self.assertEqual(summary["frames_with_research_payloads"], 2)
        self.assertEqual(summary["country_rows_with_research_payloads"], 3)
        self.assertEqual(summary["max_research_country_rows_per_frame"], 2)
        self.assertEqual(summary["max_research_active_rows_per_country"], 2)
        self.assertEqual(summary["tech_on_lightweight_frames"], 1)
        self.assertEqual(summary["heavy_payload_on_lightweight_frames"], 1)

    def test_collector_frame_payload_summary_counts_projects_live_coverage(self):
        summary = cs._collector_frame_payload_summary([
            {
                "capture_boundary": "scheduled_hour_snapshot",
                "domains": ["clock", "projects_live"],
                "special_projects": {
                    "facilities": [
                        {"id": 1},
                        {"id": 2},
                    ],
                    "projects": [
                        {"id": 10},
                    ],
                    "scientists": [
                        {"char_id": 100},
                        {"char_id": 101},
                        {"char_id": 102},
                    ],
                    "source": "C",
                },
            },
            {
                "capture_boundary": "scheduled_hour_snapshot",
                "domains": ["clock", "projects_live"],
                "special_projects": {
                    "facilities": [],
                    "projects": [],
                    "scientists": [],
                    "source": "C",
                },
            },
            {
                "capture_boundary": "worker_poll",
                "domains": ["clock"],
                "special_projects": {
                    "facilities": [{"id": 3}],
                    "projects": [],
                    "scientists": [],
                    "source": "C",
                },
            },
        ])

        self.assertEqual(summary["domains_seen"], ["clock", "projects_live"])
        self.assertEqual(summary["projects_live_frames"], 2)
        self.assertEqual(summary["scheduled_projects_domain_frames"], 2)
        self.assertEqual(summary["frames_with_special_projects_payloads"], 3)
        self.assertEqual(summary["special_project_facility_rows"], 3)
        self.assertEqual(summary["special_project_rows"], 1)
        self.assertEqual(summary["special_project_scientist_rows"], 3)
        self.assertEqual(summary["max_special_project_facilities_per_frame"], 2)
        self.assertEqual(summary["max_special_project_rows_per_frame"], 1)
        self.assertEqual(summary["max_special_project_scientists_per_frame"], 3)
        self.assertEqual(summary["projects_on_lightweight_frames"], 1)
        self.assertEqual(summary["heavy_payload_on_lightweight_frames"], 1)

    def test_collector_frame_payload_summary_counts_politics_live_coverage(self):
        summary = cs._collector_frame_payload_summary([
            {
                "capture_boundary": "scheduled_hour_snapshot",
                "domains": ["clock", "politics_live"],
                "countries": {
                    "POR": {
                        "politics": {
                            "popularities": {"neutrality": 100.0},
                            "ruling_party": "neutrality",
                            "ideas": ["war_economy", "sour_loser"],
                        },
                    },
                    "GER": {
                        "politics": {
                            "popularities": {"fascism": 80.0},
                            "ruling_party": "fascism",
                            "ideas": ["autarky"],
                        },
                    },
                },
            },
            {
                "capture_boundary": "worker_poll",
                "domains": ["clock"],
                "countries": {
                    "ITA": {
                        "politics": {
                            "popularities": {"fascism": 60.0},
                            "ruling_party": "fascism",
                            "ideas": ["idea_a", "idea_b", "idea_c"],
                        },
                    },
                },
            },
        ])

        self.assertEqual(summary["domains_seen"], ["clock", "politics_live"])
        self.assertEqual(summary["politics_live_frames"], 1)
        self.assertEqual(summary["scheduled_politics_domain_frames"], 1)
        self.assertEqual(summary["frames_with_politics_payloads"], 2)
        self.assertEqual(summary["country_rows_with_politics_payloads"], 3)
        self.assertEqual(summary["max_politics_country_rows_per_frame"], 2)
        self.assertEqual(summary["max_politics_idea_rows_per_country"], 3)
        self.assertEqual(summary["politics_on_lightweight_frames"], 1)
        self.assertEqual(summary["heavy_payload_on_lightweight_frames"], 1)

    def test_collector_frame_payload_summary_counts_politics_residuals_coverage(self):
        summary = cs._collector_frame_payload_summary([
            {
                "capture_boundary": "scheduled_hour_snapshot",
                "domains": ["clock", "politics_residuals"],
                "countries": {
                    "POR": {
                        "politics": {
                            "last_election": "1933.7.1.1",
                            "elections_allowed": False,
                        },
                    },
                    "GER": {
                        "politics": {
                            "last_election": "1933.3.5.1",
                            "elections_allowed": True,
                        },
                    },
                },
            },
            {
                "capture_boundary": "worker_poll",
                "domains": ["clock", "politics_residuals"],
                "countries": {
                    "ITA": {
                        "politics": {
                            "elections_allowed": False,
                        },
                    },
                },
            },
        ])

        self.assertEqual(summary["domains_seen"], ["clock", "politics_residuals"])
        self.assertEqual(summary["politics_residuals_frames"], 2)
        self.assertEqual(summary["scheduled_politics_residuals_domain_frames"], 1)
        self.assertEqual(summary["frames_with_politics_residual_payloads"], 2)
        self.assertEqual(summary["country_rows_with_politics_residual_payloads"], 3)
        self.assertEqual(summary["max_politics_residual_country_rows_per_frame"], 2)
        self.assertEqual(summary["politics_residuals_on_lightweight_frames"], 1)
        self.assertEqual(summary["heavy_payload_on_lightweight_frames"], 1)

    def test_collector_frame_payload_summary_counts_raw_stream_frames(self):
        summary = cs._collector_frame_payload_summary([
            {
                "type": "event",
                "capture_boundary": "worker_poll",
                "domain": "province_control",
                "payload": {
                    "event_type": "province_controller_change",
                },
            },
            {
                "type": "domain_delta",
                "capture_boundary": "worker_poll",
                "domain": "divisions",
                "op": "sample",
            },
            {
                "type": "gap",
                "capture_boundary": "worker_poll",
                "domain": "land_combats",
                "gap_kind": "dropped_frames",
            },
        ])

        self.assertEqual(summary["frames_by_type"], {
            "domain_delta": 1,
            "event": 1,
            "gap": 1,
        })
        self.assertEqual(summary["raw_event_frame_count"], 1)
        self.assertEqual(summary["raw_domain_delta_frame_count"], 1)
        self.assertEqual(summary["raw_gap_frame_count"], 1)
        self.assertEqual(
            summary["raw_event_types_seen"],
            ["province_controller_change"],
        )
        self.assertEqual(summary["raw_delta_domains_seen"], ["divisions"])
        self.assertEqual(summary["raw_gap_domains_seen"], ["land_combats"])
        self.assertEqual(summary["lightweight_poll_frame_count"], 3)
        self.assertEqual(summary["heavy_payload_on_lightweight_frames"], 0)

    def test_collector_stream_transport_summary_counts_raw_frame_sequences(self):
        summary = cs._collector_stream_transport_summary([
            {
                "type": "collector_started",
                "sequence": 1,
                "worker_burst_index": 0,
            },
            {
                "type": "domain_delta",
                "sequence": 2,
                "domain": "clock",
                "worker_burst_index": 0,
            },
            {
                "type": "domain_delta",
                "sequence": 4,
                "domain": "divisions",
                "worker_burst_index": 0,
            },
            {
                "type": "collector_started",
                "sequence": 1,
                "worker_burst_index": 1,
            },
            {
                "type": "domain_delta",
                "sequence": 2,
                "domain": "clock",
                "worker_burst_index": 1,
            },
        ])

        self.assertEqual(summary["frame_count"], 5)
        self.assertEqual(summary["sequence_count"], 5)
        self.assertEqual(summary["mode"], "burst_local")
        self.assertFalse(summary["monotonic_sequence"])
        self.assertEqual(summary["duplicate_sequences"], 2)
        self.assertEqual(summary["collector_started_frame_count"], 2)
        self.assertEqual(summary["average_frames_per_collector_started"], 2.5)
        self.assertTrue(summary["burst_local_monotonic_sequence"])
        self.assertEqual(summary["burst_local_duplicate_sequences"], 0)
        self.assertEqual(summary["burst_local_missing_sequences"], 1)
        self.assertEqual(summary["burst_local_max_sequence_gap"], 1)

    def test_session_can_record_state_control_preflight_diagnostics(self):
        original_record_probes = cs.record_probes
        original_preflight = cs.resolve_state_control_preflight
        try:
            with tempfile.TemporaryDirectory() as td:
                root = Path(td)

                def fake_preflight(pid, *, cache_path):
                    self.assertEqual(pid, 555)
                    self.assertEqual(cache_path.name, "state_roots.json")
                    return _PreflightResult({
                        "enabled": True,
                        "status": "resolved",
                        "row_count": 1081,
                        "owner_rows": 1081,
                        "controller_rows": 1081,
                        "source": "state_roots_cache",
                        "address_manifest_policy": "omitted",
                    }, state_roots_encoded="1:0x1000;2:0x2000")

                def fake_record_probes(pid, dll, out_root, **kwargs):
                    del dll
                    self.assertIs(kwargs.get("write_snapshot_artifacts"), False)
                    self.assertEqual(
                        kwargs.get("map_control_state_roots"),
                        "1:0x1000;2:0x2000",
                    )
                    run_dir = Path(out_root) / ("20260628T030000Z_pid%d" % pid)
                    run_dir.mkdir(parents=True)
                    frame = _frame(
                        "1936.1.1.1",
                        100,
                        sequence=1,
                        pid=pid,
                        capture_boundary="scheduled_hour_snapshot",
                    )
                    frame["domains"] = ["clock", "state_ownership", "state_control"]
                    frame["root_probe"]["domains"] = [
                        "clock",
                        "state_ownership",
                        "state_control",
                    ]
                    frame["states"] = {
                        "1": {"owner": "GER", "controller": "GER"},
                        "2": {"owner": "ENG", "controller": "ENG"},
                    }
                    frame_name = "collector_started_0000.json"
                    _write_json(run_dir / frame_name, frame)
                    _write_json(run_dir / "manifest.json", {
                        "type": "collector_probe_manifest",
                        "session_id": run_dir.name,
                        "pid": pid,
                        "game_id": "state_preflight",
                        "stop_reason": "repeat_exhausted",
                        "sample_count": 1,
                        "unique_game_date_count": 1,
                        "game_hours_advanced": 0,
                        "skipped_game_hours": 0,
                        "map_control_state_roots_supplied": True,
                        "map_control_state_root_rows": 2,
                        "map_control_province_bridge_supplied": False,
                        "map_control_controller_bridge_supplied": False,
                        "map_control_controller_bridge_rows": 0,
                        "worker_pipe_tokens": ["token"],
                        "monotonic_transport_sequence": True,
                        "duplicate_transport_sequences": 0,
                        "first_sequence": 1,
                        "last_sequence": 1,
                        "samples": [{
                            "frame": frame_name,
                            "current_game_date": frame["root_probe"]["current_game_date"],
                            "current_game_date_raw": frame["root_probe"]["current_game_date_raw"],
                            "sequence": frame["sequence"],
                        }],
                        "hourly_requirement": {
                            "required": False,
                            "requested_game_hours": None,
                            "met": True,
                            "problems": [],
                        },
                    })
                    return run_dir

                cs.resolve_state_control_preflight = fake_preflight
                cs.record_probes = fake_record_probes

                run_dir = record_collector_session(
                    555,
                    Path("collector.dll"),
                    root / "out",
                    game_id="state_preflight",
                    duration_s=1.0,
                    preflight_state_control=True,
                )
                session_manifest = _read_json(run_dir / "session_manifest.json")
        finally:
            cs.record_probes = original_record_probes
            cs.resolve_state_control_preflight = original_preflight

        self.assertEqual(
            session_manifest["state_control_preflight"]["status"],
            "resolved",
        )
        self.assertEqual(session_manifest["state_control_preflight"]["row_count"], 1081)
        self.assertEqual(
            session_manifest["state_control_preflight"]["source"],
            "state_roots_cache",
        )
        self.assertNotIn(str(root), json.dumps(session_manifest, sort_keys=True))
        self.assertTrue(session_manifest["map_control_state_roots_supplied"])
        self.assertEqual(session_manifest["map_control_state_root_rows"], 2)
        self.assertEqual(
            session_manifest["map_control_quality"]["requested_domains"],
            ["state_control", "state_ownership"],
        )
        self.assertEqual(
            session_manifest["map_control_quality"]["captured_domains"],
            ["state_control", "state_ownership"],
        )
        self.assertEqual(
            session_manifest["map_control_quality"]["omitted_domains"], {})
        self.assertFalse(session_manifest["map_control_quality"]["fail_closed"])

    def test_session_passes_advisor_token_filter_preflight_to_probe(self):
        original_record_probes = cs.record_probes
        original_advisor_preflight = cs.resolve_advisor_token_filter_preflight
        original_idea_group_preflight = cs.resolve_idea_group_filter_preflight
        try:
            with tempfile.TemporaryDirectory() as td:
                root = Path(td)

                def fake_advisor_preflight(pid):
                    self.assertEqual(pid, 557)
                    return _AdvisorPreflightResult(
                        {
                            "enabled": True,
                            "status": "resolved",
                            "source": "game_defs.common_characters.vanilla",
                            "token_count": 2,
                            "sha256": "a" * 64,
                        },
                        encoded="ENG_maxwell_aitken;POR_salazar",
                    )

                def fake_idea_group_preflight(pid):
                    self.assertEqual(pid, 557)
                    return _AdvisorPreflightResult(
                        {
                            "enabled": True,
                            "status": "resolved",
                            "source": "game_defs.common_ideas.vanilla",
                            "row_count": 2,
                            "sha256": "b" * 64,
                        },
                        encoded="free_trade=trade_laws;war_economy=economy",
                    )

                def fake_record_probes(pid, dll, out_root, **kwargs):
                    del dll
                    self.assertEqual(pid, 557)
                    self.assertEqual(
                        kwargs.get("advisor_token_filter"),
                        "ENG_maxwell_aitken;POR_salazar",
                    )
                    self.assertEqual(
                        kwargs.get("idea_group_filter"),
                        "free_trade=trade_laws;war_economy=economy",
                    )
                    run_dir = Path(out_root) / ("20260628T031000Z_pid%d" % pid)
                    run_dir.mkdir(parents=True)
                    frame = _frame(
                        "1936.1.1.1",
                        100,
                        sequence=1,
                        pid=pid,
                        capture_boundary="scheduled_hour_snapshot",
                    )
                    frame_name = "collector_started_0000.json"
                    _write_json(run_dir / frame_name, frame)
                    _write_json(run_dir / "manifest.json", {
                        "type": "collector_probe_manifest",
                        "session_id": run_dir.name,
                        "pid": pid,
                        "game_id": "advisor_preflight",
                        "stop_reason": "repeat_exhausted",
                        "sample_count": 1,
                        "unique_game_date_count": 1,
                        "game_hours_advanced": 0,
                        "skipped_game_hours": 0,
                        "advisor_token_filter_supplied": True,
                        "advisor_token_filter_tokens": 2,
                        "advisor_token_filter_sha256": "a" * 64,
                        "idea_group_filter_supplied": True,
                        "idea_group_filter_rows": 2,
                        "idea_group_filter_sha256": "b" * 64,
                        "worker_pipe_tokens": ["token"],
                        "monotonic_transport_sequence": True,
                        "duplicate_transport_sequences": 0,
                        "first_sequence": 1,
                        "last_sequence": 1,
                        "samples": [{
                            "frame": frame_name,
                            "current_game_date": frame["root_probe"]["current_game_date"],
                            "current_game_date_raw": frame["root_probe"]["current_game_date_raw"],
                            "sequence": frame["sequence"],
                        }],
                        "hourly_requirement": {
                            "required": False,
                            "requested_game_hours": None,
                            "met": True,
                            "problems": [],
                        },
                    })
                    return run_dir

                cs.resolve_advisor_token_filter_preflight = fake_advisor_preflight
                cs.resolve_idea_group_filter_preflight = fake_idea_group_preflight
                cs.record_probes = fake_record_probes

                run_dir = record_collector_session(
                    557,
                    Path("collector.dll"),
                    root / "out",
                    game_id="advisor_preflight",
                    duration_s=1.0,
                )
                session_manifest = _read_json(run_dir / "session_manifest.json")
        finally:
            cs.record_probes = original_record_probes
            cs.resolve_advisor_token_filter_preflight = original_advisor_preflight
            cs.resolve_idea_group_filter_preflight = original_idea_group_preflight

        self.assertEqual(
            session_manifest["advisor_token_filter_preflight"]["status"],
            "resolved",
        )
        self.assertTrue(session_manifest["advisor_token_filter_supplied"])
        self.assertEqual(session_manifest["advisor_token_filter_tokens"], 2)
        self.assertEqual(len(session_manifest["advisor_token_filter_sha256"]), 64)
        self.assertEqual(
            session_manifest["idea_group_filter_preflight"]["status"],
            "resolved",
        )
        self.assertTrue(session_manifest["idea_group_filter_supplied"])
        self.assertEqual(session_manifest["idea_group_filter_rows"], 2)
        self.assertEqual(len(session_manifest["idea_group_filter_sha256"]), 64)
        self.assertNotIn(
            "POR_salazar",
            json.dumps(session_manifest, sort_keys=True),
        )
        self.assertNotIn(
            "war_economy",
            json.dumps(session_manifest, sort_keys=True),
        )

    def test_session_fails_closed_when_requested_state_control_is_omitted(self):
        original_record_probes = cs.record_probes
        original_preflight = cs.resolve_state_control_preflight
        try:
            with tempfile.TemporaryDirectory() as td:
                root = Path(td)

                def fake_preflight(pid, *, cache_path):
                    del cache_path
                    self.assertEqual(pid, 556)
                    return _PreflightResult({
                        "enabled": True,
                        "status": "resolved",
                        "row_count": 1081,
                        "source": "state_roots_cache",
                        "address_manifest_policy": "omitted",
                    }, state_roots_encoded="1:0x1000;2:0x2000")

                def fake_record_probes(pid, dll, out_root, **kwargs):
                    del dll
                    self.assertEqual(
                        kwargs.get("map_control_state_roots"),
                        "1:0x1000;2:0x2000",
                    )
                    run_dir = Path(out_root) / ("20260628T031500Z_pid%d" % pid)
                    run_dir.mkdir(parents=True)
                    frame = _frame(
                        "1936.1.1.1",
                        100,
                        sequence=1,
                        pid=pid,
                        capture_boundary="scheduled_hour_snapshot",
                    )
                    frame_name = "collector_started_0000.json"
                    _write_json(run_dir / frame_name, frame)
                    _write_json(run_dir / "manifest.json", {
                        "type": "collector_probe_manifest",
                        "session_id": run_dir.name,
                        "pid": pid,
                        "game_id": "state_preflight_omitted",
                        "stop_reason": "repeat_exhausted",
                        "sample_count": 1,
                        "unique_game_date_count": 1,
                        "game_hours_advanced": 0,
                        "skipped_game_hours": 0,
                        "map_control_state_roots_supplied": True,
                        "map_control_state_root_rows": 2,
                        "map_control_province_bridge_supplied": False,
                        "map_control_controller_bridge_supplied": False,
                        "map_control_controller_bridge_rows": 0,
                        "worker_pipe_tokens": ["token"],
                        "monotonic_transport_sequence": True,
                        "duplicate_transport_sequences": 0,
                        "first_sequence": 1,
                        "last_sequence": 1,
                        "samples": [{
                            "frame": frame_name,
                            "current_game_date": frame["root_probe"]["current_game_date"],
                            "current_game_date_raw": frame["root_probe"]["current_game_date_raw"],
                            "sequence": frame["sequence"],
                        }],
                        "hourly_requirement": {
                            "required": False,
                            "requested_game_hours": None,
                            "met": True,
                            "problems": [],
                        },
                    })
                    return run_dir

                cs.resolve_state_control_preflight = fake_preflight
                cs.record_probes = fake_record_probes

                with self.assertRaisesRegex(RuntimeError, "map_control_omitted"):
                    record_collector_session(
                        556,
                        Path("collector.dll"),
                        root / "out",
                        game_id="state_preflight_omitted",
                        duration_s=1.0,
                        preflight_state_control=True,
                    )
                session_manifest = _read_json(
                    root / "out" / "20260628T031500Z_pid556"
                    / "session_manifest.json")
        finally:
            cs.record_probes = original_record_probes
            cs.resolve_state_control_preflight = original_preflight

        self.assertEqual(
            session_manifest["map_control_quality"]["omitted_domains"],
            {
                "state_control": "no_scheduled_domain_frames",
                "state_ownership": "no_scheduled_domain_frames",
            },
        )
        self.assertTrue(session_manifest["map_control_quality"]["fail_closed"])
        self.assertIn(
            "map_control_omitted",
            ";".join(session_manifest["session_errors"]),
        )

    def test_session_can_preflight_division_movement_bridge(self):
        original_record_probes = cs.record_probes
        original_preflight = cs.resolve_division_body_bridge_preflight
        try:
            with tempfile.TemporaryDirectory() as td:
                root = Path(td)

                def fake_preflight(pid, *, cache_path, allow_cold_scan):
                    self.assertEqual(pid, 555)
                    self.assertEqual(cache_path.name, "division_bodies.json")
                    self.assertFalse(allow_cold_scan)
                    return _DivisionPreflightResult(
                        {
                            "enabled": True,
                            "status": "resolved",
                            "row_count": 2,
                            "build_ok": True,
                            "pid_match": True,
                            "elapsed_s": 0.25,
                            "cold_locate_s": 0.20,
                            "allow_cold_scan": False,
                            "cache": {
                                "enabled": True,
                                "status": "hit",
                            },
                            "address_manifest_policy": "omitted",
                        },
                        encoded="17:0x8000;18:0x9000",
                    )

                def fake_record_probes(pid, dll, out_root, **kwargs):
                    del dll
                    self.assertIs(kwargs.get("write_snapshot_artifacts"), False)
                    self.assertEqual(
                        kwargs.get("division_body_bridge"),
                        "17:0x8000;18:0x9000",
                    )
                    run_dir = Path(out_root) / ("20260628T031500Z_pid%d" % pid)
                    run_dir.mkdir(parents=True)
                    frame = _frame("1936.1.1.1", 100, sequence=1, pid=pid)
                    frame_name = "collector_started_0000.json"
                    _write_json(run_dir / frame_name, frame)
                    _write_json(run_dir / "manifest.json", {
                        "type": "collector_probe_manifest",
                        "session_id": run_dir.name,
                        "pid": pid,
                        "game_id": "division_preflight",
                        "stop_reason": "repeat_exhausted",
                        "sample_count": 1,
                        "unique_game_date_count": 1,
                        "game_hours_advanced": 0,
                        "skipped_game_hours": 0,
                        "division_body_bridge_supplied": True,
                        "division_body_bridge_rows": 2,
                        "worker_pipe_tokens": ["token"],
                        "monotonic_transport_sequence": True,
                        "duplicate_transport_sequences": 0,
                        "first_sequence": 1,
                        "last_sequence": 1,
                        "samples": [{
                            "frame": frame_name,
                            "current_game_date": frame["root_probe"]["current_game_date"],
                            "current_game_date_raw": frame["root_probe"]["current_game_date_raw"],
                            "sequence": frame["sequence"],
                        }],
                        "hourly_requirement": {
                            "required": False,
                            "requested_game_hours": None,
                            "met": True,
                            "problems": [],
                        },
                    })
                    return run_dir

                cs.resolve_division_body_bridge_preflight = fake_preflight
                cs.record_probes = fake_record_probes

                run_dir = record_collector_session(
                    555,
                    Path("collector.dll"),
                    root / "out",
                    game_id="division_preflight",
                    duration_s=1.0,
                    preflight_division_movement=True,
                )
                session_manifest = _read_json(run_dir / "session_manifest.json")
        finally:
            cs.record_probes = original_record_probes
            cs.resolve_division_body_bridge_preflight = original_preflight

        self.assertEqual(
            session_manifest["division_body_bridge_preflight"]["status"],
            "resolved",
        )
        self.assertEqual(
            session_manifest["division_body_bridge_preflight"]["row_count"],
            2,
        )
        self.assertEqual(
            session_manifest["division_body_bridge_preflight"]
            ["address_manifest_policy"],
            "omitted",
        )
        self.assertFalse(
            session_manifest["division_body_bridge_preflight"]
            ["allow_cold_scan"],
        )
        self.assertEqual(
            session_manifest["division_body_bridge_preflight"]["cache"]["status"],
            "hit",
        )
        self.assertTrue(session_manifest["division_body_bridge_supplied"])
        self.assertEqual(session_manifest["division_body_bridge_rows"], 2)
        self.assertNotIn("0x8000", json.dumps(session_manifest, sort_keys=True))
        self.assertNotIn("0x9000", json.dumps(session_manifest, sort_keys=True))
        self.assertNotIn(str(root), json.dumps(session_manifest, sort_keys=True))

    def test_session_can_preflight_naval_combat_bridge(self):
        original_record_probes = cs.record_probes
        original_preflight = cs.resolve_naval_combat_body_bridge_preflight
        try:
            with tempfile.TemporaryDirectory() as td:
                root = Path(td)

                def fake_preflight(pid):
                    self.assertEqual(pid, 555)
                    return _DivisionPreflightResult(
                        {
                            "enabled": True,
                            "status": "resolved",
                            "row_count": 2,
                            "build_ok": True,
                            "pid_match": True,
                            "elapsed_s": 0.12,
                            "source": "CombatReader.enumerate_naval_combats",
                            "discovery_mode": "preflight_vtable_enumeration",
                            "hot_loop_heap_scan": False,
                            "address_manifest_policy": "omitted",
                        },
                        encoded="41:0x8000;42:0x9000",
                    )

                def fake_record_probes(pid, dll, out_root, **kwargs):
                    del dll
                    self.assertIs(kwargs.get("write_snapshot_artifacts"), False)
                    self.assertEqual(
                        kwargs.get("naval_combat_body_bridge"),
                        "41:0x8000;42:0x9000",
                    )
                    run_dir = Path(out_root) / ("20260628T031500Z_pid%d" % pid)
                    run_dir.mkdir(parents=True)
                    frame = _frame("1936.1.1.1", 100, sequence=1, pid=pid)
                    frame_name = "collector_started_0000.json"
                    _write_json(run_dir / frame_name, frame)
                    _write_json(run_dir / "manifest.json", {
                        "type": "collector_probe_manifest",
                        "session_id": run_dir.name,
                        "pid": pid,
                        "game_id": "naval_preflight",
                        "stop_reason": "repeat_exhausted",
                        "sample_count": 1,
                        "unique_game_date_count": 1,
                        "game_hours_advanced": 0,
                        "skipped_game_hours": 0,
                        "naval_combat_body_bridge_supplied": True,
                        "naval_combat_body_bridge_rows": 2,
                        "worker_pipe_tokens": ["token"],
                        "monotonic_transport_sequence": True,
                        "duplicate_transport_sequences": 0,
                        "first_sequence": 1,
                        "last_sequence": 1,
                        "samples": [{
                            "frame": frame_name,
                            "current_game_date": frame["root_probe"]["current_game_date"],
                            "current_game_date_raw": frame["root_probe"]["current_game_date_raw"],
                            "sequence": frame["sequence"],
                        }],
                        "hourly_requirement": {
                            "required": False,
                            "requested_game_hours": None,
                            "met": True,
                            "problems": [],
                        },
                    })
                    return run_dir

                cs.resolve_naval_combat_body_bridge_preflight = fake_preflight
                cs.record_probes = fake_record_probes

                run_dir = record_collector_session(
                    555,
                    Path("collector.dll"),
                    root / "out",
                    game_id="naval_preflight",
                    duration_s=1.0,
                    preflight_naval_combat=True,
                )
                session_manifest = _read_json(run_dir / "session_manifest.json")
        finally:
            cs.record_probes = original_record_probes
            cs.resolve_naval_combat_body_bridge_preflight = original_preflight

        self.assertEqual(
            session_manifest["naval_combat_body_bridge_preflight"]["status"],
            "resolved",
        )
        self.assertEqual(
            session_manifest["naval_combat_body_bridge_preflight"]["row_count"],
            2,
        )
        self.assertEqual(
            session_manifest["naval_combat_body_bridge_preflight"]
            ["address_manifest_policy"],
            "omitted",
        )
        self.assertEqual(
            session_manifest["naval_combat_body_bridge_preflight"]
            ["discovery_mode"],
            "preflight_vtable_enumeration",
        )
        self.assertFalse(
            session_manifest["naval_combat_body_bridge_preflight"]
            ["hot_loop_heap_scan"],
        )
        self.assertTrue(session_manifest["naval_combat_body_bridge_supplied"])
        self.assertEqual(session_manifest["naval_combat_body_bridge_rows"], 2)
        self.assertNotIn("0x8000", json.dumps(session_manifest, sort_keys=True))
        self.assertNotIn("0x9000", json.dumps(session_manifest, sort_keys=True))
        self.assertNotIn(str(root), json.dumps(session_manifest, sort_keys=True))

    def test_session_can_preflight_army_order_bridge(self):
        original_record_probes = cs.record_probes
        original_preflight = cs.resolve_army_body_bridge_preflight
        try:
            with tempfile.TemporaryDirectory() as td:
                root = Path(td)

                def fake_preflight(pid, *, cache_path, allow_cold_scan):
                    self.assertEqual(pid, 555)
                    self.assertEqual(cache_path.name, "army_bodies.json")
                    self.assertFalse(allow_cold_scan)
                    return _DivisionPreflightResult(
                        {
                            "enabled": True,
                            "status": "resolved",
                            "row_count": 2,
                            "build_ok": True,
                            "pid_match": True,
                            "elapsed_s": 0.25,
                            "cold_locate_s": None,
                            "allow_cold_scan": False,
                            "cache": {
                                "enabled": True,
                                "status": "hit",
                            },
                            "address_manifest_policy": "omitted",
                        },
                        encoded="17:0x8000;18:0x9000",
                    )

                def fake_record_probes(pid, dll, out_root, **kwargs):
                    del dll
                    self.assertIs(kwargs.get("write_snapshot_artifacts"), False)
                    self.assertEqual(
                        kwargs.get("army_body_bridge"),
                        "17:0x8000;18:0x9000",
                    )
                    run_dir = Path(out_root) / ("20260628T031500Z_pid%d" % pid)
                    run_dir.mkdir(parents=True)
                    frame = _frame("1936.1.1.1", 100, sequence=1, pid=pid)
                    frame_name = "collector_started_0000.json"
                    _write_json(run_dir / frame_name, frame)
                    _write_json(run_dir / "manifest.json", {
                        "type": "collector_probe_manifest",
                        "session_id": run_dir.name,
                        "pid": pid,
                        "game_id": "army_preflight",
                        "stop_reason": "repeat_exhausted",
                        "sample_count": 1,
                        "unique_game_date_count": 1,
                        "game_hours_advanced": 0,
                        "skipped_game_hours": 0,
                        "army_body_bridge_supplied": True,
                        "army_body_bridge_rows": 2,
                        "worker_pipe_tokens": ["token"],
                        "monotonic_transport_sequence": True,
                        "duplicate_transport_sequences": 0,
                        "first_sequence": 1,
                        "last_sequence": 1,
                        "samples": [{
                            "frame": frame_name,
                            "current_game_date": frame["root_probe"]["current_game_date"],
                            "current_game_date_raw": frame["root_probe"]["current_game_date_raw"],
                            "sequence": frame["sequence"],
                        }],
                        "hourly_requirement": {
                            "required": False,
                            "requested_game_hours": None,
                            "met": True,
                            "problems": [],
                        },
                    })
                    return run_dir

                cs.resolve_army_body_bridge_preflight = fake_preflight
                cs.record_probes = fake_record_probes

                run_dir = record_collector_session(
                    555,
                    Path("collector.dll"),
                    root / "out",
                    game_id="army_preflight",
                    duration_s=1.0,
                    preflight_army_orders=True,
                )
                session_manifest = _read_json(run_dir / "session_manifest.json")
        finally:
            cs.record_probes = original_record_probes
            cs.resolve_army_body_bridge_preflight = original_preflight

        self.assertEqual(
            session_manifest["army_body_bridge_preflight"]["status"],
            "resolved",
        )
        self.assertEqual(
            session_manifest["army_body_bridge_preflight"]["row_count"],
            2,
        )
        self.assertEqual(
            session_manifest["army_body_bridge_preflight"]
            ["address_manifest_policy"],
            "omitted",
        )
        self.assertFalse(
            session_manifest["army_body_bridge_preflight"]
            ["allow_cold_scan"],
        )
        self.assertEqual(
            session_manifest["army_body_bridge_preflight"]["cache"]["status"],
            "hit",
        )
        self.assertTrue(session_manifest["army_body_bridge_supplied"])
        self.assertEqual(session_manifest["army_body_bridge_rows"], 2)
        self.assertNotIn("0x8000", json.dumps(session_manifest, sort_keys=True))
        self.assertNotIn("0x9000", json.dumps(session_manifest, sort_keys=True))
        self.assertNotIn(str(root), json.dumps(session_manifest, sort_keys=True))

    def test_session_can_preflight_air_wing_bridge(self):
        original_record_probes = cs.record_probes
        original_preflight = cs.resolve_air_wing_body_bridge_preflight
        try:
            with tempfile.TemporaryDirectory() as td:
                root = Path(td)

                def fake_preflight(pid, *, cache_path, allow_cold_scan):
                    self.assertEqual(pid, 555)
                    self.assertEqual(cache_path.name, "air_wing_bodies.json")
                    self.assertFalse(allow_cold_scan)
                    return _DivisionPreflightResult(
                        {
                            "enabled": True,
                            "status": "resolved",
                            "row_count": 2,
                            "build_ok": True,
                            "pid_match": True,
                            "elapsed_s": 0.25,
                            "cold_locate_s": None,
                            "allow_cold_scan": False,
                            "cache": {
                                "enabled": True,
                                "status": "hit",
                            },
                            "discovery_mode": "cache_or_preflight_vtable_enumeration",
                            "hot_loop_heap_scan": False,
                            "address_manifest_policy": "omitted",
                        },
                        encoded="1:3:0x8000;2:4:0x9000",
                    )

                def fake_record_probes(pid, dll, out_root, **kwargs):
                    del dll
                    self.assertIs(kwargs.get("write_snapshot_artifacts"), False)
                    self.assertEqual(
                        kwargs.get("air_wing_body_bridge"),
                        "1:3:0x8000;2:4:0x9000",
                    )
                    run_dir = Path(out_root) / ("20260628T031500Z_pid%d" % pid)
                    run_dir.mkdir(parents=True)
                    frame = _frame("1936.1.1.1", 100, sequence=1, pid=pid)
                    frame_name = "collector_started_0000.json"
                    _write_json(run_dir / frame_name, frame)
                    _write_json(run_dir / "manifest.json", {
                        "type": "collector_probe_manifest",
                        "session_id": run_dir.name,
                        "pid": pid,
                        "game_id": "air_wing_preflight",
                        "stop_reason": "repeat_exhausted",
                        "sample_count": 1,
                        "unique_game_date_count": 1,
                        "game_hours_advanced": 0,
                        "skipped_game_hours": 0,
                        "air_wing_body_bridge_supplied": True,
                        "air_wing_body_bridge_rows": 2,
                        "worker_pipe_tokens": ["token"],
                        "monotonic_transport_sequence": True,
                        "duplicate_transport_sequences": 0,
                        "first_sequence": 1,
                        "last_sequence": 1,
                        "samples": [{
                            "frame": frame_name,
                            "current_game_date": frame["root_probe"]["current_game_date"],
                            "current_game_date_raw": frame["root_probe"]["current_game_date_raw"],
                            "sequence": frame["sequence"],
                        }],
                        "hourly_requirement": {
                            "required": False,
                            "requested_game_hours": None,
                            "met": True,
                            "problems": [],
                        },
                    })
                    return run_dir

                cs.resolve_air_wing_body_bridge_preflight = fake_preflight
                cs.record_probes = fake_record_probes

                run_dir = record_collector_session(
                    555,
                    Path("collector.dll"),
                    root / "out",
                    game_id="air_wing_preflight",
                    duration_s=1.0,
                    preflight_air_wings=True,
                )
                session_manifest = _read_json(run_dir / "session_manifest.json")
        finally:
            cs.record_probes = original_record_probes
            cs.resolve_air_wing_body_bridge_preflight = original_preflight

        self.assertEqual(
            session_manifest["air_wing_body_bridge_preflight"]["status"],
            "resolved",
        )
        self.assertEqual(
            session_manifest["air_wing_body_bridge_preflight"]["row_count"],
            2,
        )
        self.assertEqual(
            session_manifest["air_wing_body_bridge_preflight"]
            ["address_manifest_policy"],
            "omitted",
        )
        self.assertFalse(
            session_manifest["air_wing_body_bridge_preflight"]
            ["allow_cold_scan"],
        )
        self.assertFalse(
            session_manifest["air_wing_body_bridge_preflight"]
            ["hot_loop_heap_scan"],
        )
        self.assertEqual(
            session_manifest["air_wing_body_bridge_preflight"]["cache"]["status"],
            "hit",
        )
        self.assertTrue(session_manifest["air_wing_body_bridge_supplied"])
        self.assertEqual(session_manifest["air_wing_body_bridge_rows"], 2)
        self.assertNotIn("0x8000", json.dumps(session_manifest, sort_keys=True))
        self.assertNotIn("0x9000", json.dumps(session_manifest, sort_keys=True))
        self.assertNotIn(str(root), json.dumps(session_manifest, sort_keys=True))

    def test_session_can_preflight_ship_bridge(self):
        original_record_probes = cs.record_probes
        original_preflight = cs.resolve_ship_body_bridge_preflight
        try:
            with tempfile.TemporaryDirectory() as td:
                root = Path(td)

                def fake_preflight(pid, *, cache_path, allow_cold_scan):
                    self.assertEqual(pid, 555)
                    self.assertEqual(cache_path.name, "ship_bodies.json")
                    self.assertFalse(allow_cold_scan)
                    return _DivisionPreflightResult(
                        {
                            "enabled": True,
                            "status": "resolved",
                            "row_count": 2,
                            "build_ok": True,
                            "pid_match": True,
                            "elapsed_s": 0.25,
                            "cold_locate_s": None,
                            "allow_cold_scan": False,
                            "cache": {
                                "enabled": True,
                                "status": "hit",
                            },
                            "discovery_mode": "cache_or_explicit_cship_vtable_scan",
                            "hot_loop_heap_scan": False,
                            "address_manifest_policy": "omitted",
                        },
                        encoded="3458:0x8000;3459:0x9000",
                    )

                def fake_record_probes(pid, dll, out_root, **kwargs):
                    del dll
                    self.assertIs(kwargs.get("write_snapshot_artifacts"), False)
                    self.assertEqual(
                        kwargs.get("ship_body_bridge"),
                        "3458:0x8000;3459:0x9000",
                    )
                    run_dir = Path(out_root) / ("20260628T031500Z_pid%d" % pid)
                    run_dir.mkdir(parents=True)
                    frame = _frame("1936.1.1.1", 100, sequence=1, pid=pid)
                    frame_name = "collector_started_0000.json"
                    _write_json(run_dir / frame_name, frame)
                    _write_json(run_dir / "manifest.json", {
                        "type": "collector_probe_manifest",
                        "session_id": run_dir.name,
                        "pid": pid,
                        "game_id": "ship_preflight",
                        "stop_reason": "repeat_exhausted",
                        "sample_count": 1,
                        "unique_game_date_count": 1,
                        "game_hours_advanced": 0,
                        "skipped_game_hours": 0,
                        "ship_body_bridge_supplied": True,
                        "ship_body_bridge_rows": 2,
                        "worker_pipe_tokens": ["token"],
                        "monotonic_transport_sequence": True,
                        "duplicate_transport_sequences": 0,
                        "first_sequence": 1,
                        "last_sequence": 1,
                        "samples": [{
                            "frame": frame_name,
                            "current_game_date": frame["root_probe"]["current_game_date"],
                            "current_game_date_raw": frame["root_probe"]["current_game_date_raw"],
                            "sequence": frame["sequence"],
                        }],
                        "hourly_requirement": {
                            "required": False,
                            "requested_game_hours": None,
                            "met": True,
                            "problems": [],
                        },
                    })
                    return run_dir

                cs.resolve_ship_body_bridge_preflight = fake_preflight
                cs.record_probes = fake_record_probes

                run_dir = record_collector_session(
                    555,
                    Path("collector.dll"),
                    root / "out",
                    game_id="ship_preflight",
                    duration_s=1.0,
                    preflight_ships=True,
                )
                session_manifest = _read_json(run_dir / "session_manifest.json")
        finally:
            cs.record_probes = original_record_probes
            cs.resolve_ship_body_bridge_preflight = original_preflight

        self.assertEqual(
            session_manifest["ship_body_bridge_preflight"]["status"],
            "resolved",
        )
        self.assertEqual(
            session_manifest["ship_body_bridge_preflight"]["row_count"],
            2,
        )
        self.assertEqual(
            session_manifest["ship_body_bridge_preflight"]
            ["address_manifest_policy"],
            "omitted",
        )
        self.assertFalse(
            session_manifest["ship_body_bridge_preflight"]
            ["allow_cold_scan"],
        )
        self.assertFalse(
            session_manifest["ship_body_bridge_preflight"]
            ["hot_loop_heap_scan"],
        )
        self.assertEqual(
            session_manifest["ship_body_bridge_preflight"]["cache"]["status"],
            "hit",
        )
        self.assertTrue(session_manifest["ship_body_bridge_supplied"])
        self.assertEqual(session_manifest["ship_body_bridge_rows"], 2)
        self.assertNotIn("0x8000", json.dumps(session_manifest, sort_keys=True))
        self.assertNotIn("0x9000", json.dumps(session_manifest, sort_keys=True))
        self.assertNotIn(str(root), json.dumps(session_manifest, sort_keys=True))

    def test_session_wraps_probe_manifest_with_lifecycle_frames(self):
        original_record_probes = cs.record_probes
        try:
            with tempfile.TemporaryDirectory() as td:
                root = Path(td)

                def fake_record_probes(pid, dll, out_root, **kwargs):
                    del dll
                    self.assertIs(kwargs.get("write_snapshot_artifacts"), False)
                    run_dir = Path(out_root) / ("20260628T000000Z_pid%d" % pid)
                    run_dir.mkdir(parents=True)
                    frames = [
                        _frame("1936.1.1.1", 100, sequence=10, pid=pid),
                        _frame("1936.1.1.2", 101, sequence=11, pid=pid),
                        _frame("1936.1.1.3", 102, sequence=12, pid=pid),
                    ]
                    samples = []
                    burst_indices = [0, 0, 1, 2]
                    burst_sample_indices = [0, 1, 0, 0]
                    burst_tokens = ["a", "a", "b", "c"]
                    for i, frame in enumerate(frames):
                        frame_name = "collector_started_%04d.json" % i
                        _write_json(run_dir / frame_name, frame)
                        samples.append({
                            "worker_burst_index": burst_indices[i],
                            "burst_sample_index": burst_sample_indices[i],
                            "worker_pipe_token": burst_tokens[i],
                            "frame": frame_name,
                            "current_game_date": frame["root_probe"]["current_game_date"],
                            "current_game_date_raw": frame["root_probe"]["current_game_date_raw"],
                            "sequence": frame["sequence"],
                        })
                    _write_json(run_dir / "manifest.json", {
                        "type": "collector_probe_manifest",
                        "pid": pid,
                        "game_id": "session_test",
                        "stop_reason": "until_game_hours",
                        "sample_count": 3,
                        "stored_snapshot_count": 3,
                        "unique_game_date_count": 3,
                        "game_hours_advanced": 2,
                        "skipped_game_hours": 0,
                        "worker_pipe_tokens": ["token"],
                        "monotonic_transport_sequence": True,
                        "duplicate_transport_sequences": 0,
                        "first_sequence": 10,
                        "last_sequence": 12,
                        "samples": samples,
                        "hourly_requirement": {
                            "required": True,
                            "requested_game_hours": 2,
                            "met": True,
                            "problems": [],
                        },
                    })
                    return run_dir

                cs.record_probes = fake_record_probes
                run_dir = record_collector_session(
                    111,
                    Path("collector.dll"),
                    root / "out",
                    game_id="session_test",
                    duration_s=8.0,
                    until_game_hours=2,
                )

                session_manifest = _read_json(run_dir / "session_manifest.json")
                session_start = _read_json(run_dir / "session_start.json")
                session_stop = _read_json(run_dir / "session_stop.json")
                replay_path = run_dir / "session_test.battleplan"
                replay_exists = replay_path.exists()
                replay_valid = _verify_container(str(replay_path))
                with open(replay_path, "rb") as fh:
                    replay_manifest = json.loads(
                        _zip_read_all(fh.read())["manifest.json"].decode("utf-8"))
        finally:
            cs.record_probes = original_record_probes

        self.assertEqual(session_manifest["type"], "collector_session_manifest")
        self.assertEqual(session_manifest["mode"], "bounded_worker_session_poll")
        self.assertEqual(session_manifest["runtime_source"], "in_process_collector")
        self.assertEqual(session_manifest["dll"], "collector.dll")
        self.assertEqual(session_manifest["duration_ms"], 8000)
        self.assertEqual(session_manifest["sample_count"], 3)
        self.assertEqual(session_manifest["stored_snapshot_count"], 3)
        self.assertEqual(session_manifest["hourly_requirement"]["met"], True)
        self.assertEqual(
            session_manifest["recording_diagnostics"]["collector"]["backend"],
            "in_process_collector")
        self.assertEqual(session_manifest["transport"]["worker_pipe_token_count"], 1)
        self.assertNotIn(str(root), json.dumps(session_manifest, sort_keys=True))
        self.assertTrue(replay_exists)
        self.assertTrue(replay_valid)
        self.assertEqual(
            replay_manifest["build"]["exe_sha256"],
            "16022851EAA571728AA3A0B5FD5F990A56DA57E2249A4C1B9CFB30336F59B120")
        self.assertTrue(replay_manifest["build"]["pinned"])

        self.assertEqual(validate_collector_frame(session_start), [])
        self.assertEqual(validate_collector_frame(session_stop), [])
        self.assertEqual(session_start["type"], "session_start")
        self.assertEqual(session_stop["type"], "session_stop")
        self.assertEqual(session_stop["quality_status"], "session_complete")

    def test_session_stop_quality_reports_required_hourly_gap(self):
        original_record_probes = cs.record_probes
        try:
            with tempfile.TemporaryDirectory() as td:
                root = Path(td)

                def fake_record_probes(pid, dll, out_root, **kwargs):
                    del dll
                    self.assertIs(kwargs.get("write_snapshot_artifacts"), False)
                    run_dir = Path(out_root) / ("20260628T000000Z_pid%d" % pid)
                    run_dir.mkdir(parents=True)
                    frames = [
                        _frame("1936.1.1.1", 100, sequence=10, pid=pid),
                        _frame("1936.1.1.4", 103, sequence=11, pid=pid),
                    ]
                    samples = []
                    burst_indices = [0, 0, 1, 2]
                    burst_sample_indices = [0, 1, 0, 0]
                    burst_tokens = ["a", "a", "b", "c"]
                    for i, frame in enumerate(frames):
                        frame_name = "collector_started_%04d.json" % i
                        _write_json(run_dir / frame_name, frame)
                        samples.append({
                            "worker_burst_index": burst_indices[i],
                            "burst_sample_index": burst_sample_indices[i],
                            "worker_pipe_token": burst_tokens[i],
                            "frame": frame_name,
                            "current_game_date": frame["root_probe"]["current_game_date"],
                            "current_game_date_raw": frame["root_probe"]["current_game_date_raw"],
                            "sequence": frame["sequence"],
                        })
                    _write_json(run_dir / "manifest.json", {
                        "type": "collector_probe_manifest",
                        "pid": pid,
                        "game_id": "session_gap",
                        "stop_reason": "max_wall_s",
                        "sample_count": 2,
                        "stored_snapshot_count": 2,
                        "worker_pipe_tokens": ["token"],
                        "samples": samples,
                        "hourly_requirement": {
                            "required": True,
                            "requested_game_hours": 4,
                            "met": False,
                            "problems": ["requested_game_hours_not_reached"],
                        },
                    })
                    return run_dir

                cs.record_probes = fake_record_probes
                run_dir = record_collector_session(
                    222,
                    Path("collector.dll"),
                    root / "out",
                    game_id="session_gap",
                    duration_s=8.0,
                    until_game_hours=4,
                )
                session_stop = _read_json(run_dir / "session_stop.json")
        finally:
            cs.record_probes = original_record_probes

        self.assertEqual(session_stop["quality_status"], "session_gap")

    def test_session_can_wrap_repeated_worker_bursts(self):
        original_record_probes = cs.record_probes
        try:
            with tempfile.TemporaryDirectory() as td:
                root = Path(td)

                def fake_record_probes(pid, dll, out_root, **kwargs):
                    del dll
                    self.assertEqual(kwargs.get("repeat"), 3)
                    self.assertEqual(kwargs.get("interval_s"), 0.5)
                    self.assertEqual(kwargs.get("max_wall_s"), 32.0)
                    self.assertIs(kwargs.get("write_snapshot_artifacts"), False)
                    run_dir = Path(out_root) / ("20260628T010000Z_pid%d" % pid)
                    run_dir.mkdir(parents=True)
                    frames = [
                        _frame("1936.1.1.1", 100, sequence=10, pid=pid),
                        _frame("1936.1.1.2", 101, sequence=11, pid=pid),
                        _frame("1936.1.1.2", 101, sequence=12, pid=pid),
                        _frame("1936.1.1.4", 103, sequence=13, pid=pid),
                    ]
                    samples = []
                    burst_indices = [0, 0, 1, 2]
                    burst_sample_indices = [0, 1, 0, 0]
                    burst_tokens = ["a", "a", "b", "c"]
                    for i, frame in enumerate(frames):
                        frame_name = "collector_started_%04d.json" % i
                        _write_json(run_dir / frame_name, frame)
                        samples.append({
                            "worker_burst_index": burst_indices[i],
                            "burst_sample_index": burst_sample_indices[i],
                            "worker_pipe_token": burst_tokens[i],
                            "frame": frame_name,
                            "current_game_date": frame["root_probe"]["current_game_date"],
                            "current_game_date_raw": frame["root_probe"]["current_game_date_raw"],
                            "sequence": frame["sequence"],
                        })
                    _write_json(run_dir / "manifest.json", {
                        "type": "collector_probe_manifest",
                        "pid": pid,
                        "game_id": "session_repeat",
                        "stop_reason": "until_game_hours",
                        "sample_count": 4,
                        "unique_game_date_count": 3,
                        "game_hours_advanced": 3,
                        "skipped_game_hours": 1,
                        "worker_pipe_tokens": ["a", "b", "c"],
                        "monotonic_transport_sequence": True,
                        "duplicate_transport_sequences": 0,
                        "transport_sequence_mode": "burst_local",
                        "burst_local_monotonic_transport_sequence": True,
                        "burst_local_duplicate_transport_sequences": 0,
                        "transport_bursts": [
                            {
                                "worker_burst_index": 0,
                                "sample_count": 2,
                                "first_sequence": 10,
                                "last_sequence": 11,
                                "monotonic_sequence": True,
                                "nonmonotonic_pairs": 0,
                                "duplicate_sequences": 0,
                                "worker_pipe_token": "a",
                            },
                            {
                                "worker_burst_index": 1,
                                "sample_count": 1,
                                "first_sequence": 12,
                                "last_sequence": 12,
                                "monotonic_sequence": True,
                                "nonmonotonic_pairs": 0,
                                "duplicate_sequences": 0,
                                "worker_pipe_token": "b",
                            },
                            {
                                "worker_burst_index": 2,
                                "sample_count": 1,
                                "first_sequence": 13,
                                "last_sequence": 13,
                                "monotonic_sequence": True,
                                "nonmonotonic_pairs": 0,
                                "duplicate_sequences": 0,
                                "worker_pipe_token": "c",
                            },
                        ],
                        "first_sequence": 10,
                        "last_sequence": 13,
                        "samples": samples,
                        "hourly_requirement": {
                            "required": False,
                            "requested_game_hours": 3,
                            "met": True,
                            "problems": [],
                        },
                    })
                    return run_dir

                cs.record_probes = fake_record_probes
                run_dir = record_collector_session(
                    333,
                    Path("collector.dll"),
                    root / "out",
                    game_id="session_repeat",
                    duration_s=8.0,
                    worker_bursts=3,
                    burst_interval_s=0.5,
                    until_game_hours=3,
                    require_hourly_cadence=False,
                )
                session_manifest = _read_json(run_dir / "session_manifest.json")
                replay_path = run_dir / "session_repeat.battleplan"
                replay_valid = _verify_container(str(replay_path))
                stream_path = run_dir / session_manifest["collector_frame_stream"]
                stream_frames = [
                    json.loads(line)
                    for line in stream_path.read_text(encoding="utf-8").splitlines()
                    if line
                ]
                stream_bytes = stream_path.read_bytes()
                replay_by_name = _zip_read_all(replay_path.read_bytes())
                replay_manifest = json.loads(
                    replay_by_name["manifest.json"].decode("utf-8"))
        finally:
            cs.record_probes = original_record_probes

        self.assertEqual(session_manifest["worker_bursts"], 3)
        self.assertEqual(session_manifest["burst_interval_ms"], 500)
        self.assertEqual(session_manifest["sample_count"], 4)
        self.assertEqual(session_manifest["raw_collector_frame_count"], 4)
        self.assertEqual(session_manifest["duplicate_game_hour_frame_count"], 1)
        self.assertEqual(session_manifest["canonical_snapshot_step_hours"], 1)
        self.assertEqual(
            session_manifest["frame_stream_granularity"],
            {
                "sample_ms": 10,
                "stores_every_valid_collector_frame": True,
                "canonical_snapshots_unique_game_hours_only": True,
            },
        )
        self.assertEqual([f["sequence"] for f in stream_frames], [10, 11, 12, 13])
        self.assertEqual(session_manifest["processed_frame_count"], 3)
        self.assertEqual(session_manifest["transport"]["worker_pipe_token_count"], 3)
        self.assertEqual(session_manifest["transport"]["sequence_mode"], "burst_local")
        self.assertTrue(session_manifest["transport"]["burst_local_monotonic_sequence"])
        self.assertEqual(
            session_manifest["recording_diagnostics"]["collector"]
            ["transport_sequence"]["mode"],
            "burst_local")
        self.assertEqual(session_manifest["stored_snapshot_count"], 3)
        self.assertTrue(replay_valid)
        raw_path = "payload/session_repeat/collector_raw/collector_frames.jsonl"
        self.assertEqual(replay_by_name[raw_path], stream_bytes)
        self.assertEqual(
            replay_manifest["optional_features"],
            ["collector_raw_stream_v1"])
        self.assertEqual(replay_manifest["counts"]["raw_streams"], 1)
        self.assertEqual(
            replay_manifest["counts"]["raw_collector_frames"],
            len(stream_frames))
        self.assertEqual(replay_manifest["raw_streams"][0]["path"], raw_path)
        self.assertEqual(
            replay_manifest["raw_streams"][0]["frame_count"],
            len(stream_frames))
        self.assertEqual(
            replay_manifest["raw_streams"][0]["sequence_count"],
            len(stream_frames))
        self.assertEqual(
            replay_manifest["raw_streams"][0]["domains"],
            ["clock"])
        replay_manifest_text = json.dumps(replay_manifest, sort_keys=True)
        self.assertNotIn(str(root), replay_manifest_text)
        self.assertNotIn(str(root).replace("\\", "\\\\"), replay_manifest_text)

    def test_session_writes_prefix_replay_then_fails_closed_on_later_burst_error(self):
        original_record_probes = cs.record_probes
        try:
            with tempfile.TemporaryDirectory() as td:
                root = Path(td)
                run_dir = root / "out" / "20260628T020000Z_pid444"

                def fake_record_probes(pid, dll, out_root, **kwargs):
                    del dll, kwargs
                    run_dir.mkdir(parents=True)
                    frames = [
                        _frame("1936.1.1.1", 100, sequence=1, pid=pid),
                        _frame("1936.1.1.2", 101, sequence=2, pid=pid),
                    ]
                    samples = []
                    for i, frame in enumerate(frames):
                        frame["session_id"] = run_dir.name
                        frame_name = "collector_started_%04d.json" % i
                        _write_json(run_dir / frame_name, frame)
                        samples.append({
                            "frame": frame_name,
                            "current_game_date": frame["root_probe"]["current_game_date"],
                            "current_game_date_raw": frame["root_probe"]["current_game_date_raw"],
                            "sequence": frame["sequence"],
                        })
                    _write_json(run_dir / "manifest.json", {
                        "type": "collector_probe_manifest",
                        "session_id": run_dir.name,
                        "pid": pid,
                        "game_id": "session_partial",
                        "stop_reason": "attach_error",
                        "attach_errors": ["burst_1:AttachError:pipe timed out"],
                        "partial_session": True,
                        "sample_count": 2,
                        "unique_game_date_count": 2,
                        "game_hours_advanced": 1,
                        "skipped_game_hours": 0,
                        "worker_pipe_tokens": ["a", "b"],
                        "monotonic_transport_sequence": True,
                        "duplicate_transport_sequences": 0,
                        "first_sequence": 1,
                        "last_sequence": 2,
                        "samples": samples,
                        "hourly_requirement": {
                            "required": False,
                            "requested_game_hours": None,
                            "met": True,
                            "problems": [],
                        },
                    })
                    return run_dir

                cs.record_probes = fake_record_probes
                with self.assertRaisesRegex(RuntimeError, "attach_errors"):
                    record_collector_session(
                        444,
                        Path("collector.dll"),
                        root / "out",
                        game_id="session_partial",
                        duration_s=8.0,
                        worker_bursts=2,
                        require_hourly_cadence=False,
                    )
                session_manifest = _read_json(run_dir / "session_manifest.json")
                replay_path = run_dir / "session_partial.battleplan"
                replay_exists = replay_path.exists()
                replay_valid = _verify_container(str(replay_path))
        finally:
            cs.record_probes = original_record_probes

        self.assertIn("attach_errors", ";".join(session_manifest["session_errors"]))
        self.assertEqual(session_manifest["stored_snapshot_count"], 2)
        self.assertTrue(replay_exists)
        self.assertTrue(replay_valid)

    def test_session_fails_closed_when_manifest_frame_missing(self):
        original_record_probes = cs.record_probes
        try:
            with tempfile.TemporaryDirectory() as td:
                root = Path(td)

                def fake_record_probes(pid, dll, out_root, **kwargs):
                    del pid, dll, kwargs
                    run_dir = Path(out_root) / "missing_frame_run"
                    run_dir.mkdir(parents=True)
                    _write_json(run_dir / "manifest.json", {
                        "type": "collector_probe_manifest",
                        "pid": 222,
                        "game_id": "missing_frame",
                        "stop_reason": "until_game_hours",
                        "sample_count": 1,
                        "unique_game_date_count": 1,
                        "worker_pipe_tokens": ["token"],
                        "samples": [{"frame": "collector_started_0000.json"}],
                        "hourly_requirement": {
                            "required": True,
                            "requested_game_hours": 1,
                            "met": True,
                            "problems": [],
                        },
                    })
                    return run_dir

                cs.record_probes = fake_record_probes
                with self.assertRaisesRegex(RuntimeError, "frame unreadable"):
                    record_collector_session(
                        222,
                        Path("collector.dll"),
                        root / "out",
                        game_id="missing_frame",
                        duration_s=8.0,
                        until_game_hours=1,
                    )
        finally:
            cs.record_probes = original_record_probes

    def test_manifest_frame_session_id_is_normalized_or_rejected(self):
        with tempfile.TemporaryDirectory() as td:
            run_dir = Path(td) / "session_x"
            run_dir.mkdir()
            frame = _frame("1936.1.1.1", 100, sequence=1, pid=123)
            _write_json(run_dir / "collector_started_0000.json", frame)
            manifest = {
                "type": "collector_probe_manifest",
                "samples": [{
                    "frame": "collector_started_0000.json",
                    "current_game_date_raw": 100,
                }],
            }

            frames = cs._frames_from_probe_manifest(run_dir, manifest)
            self.assertEqual(frames[0]["session_id"], "session_x")

            _write_json(
                run_dir / "collector_started_0001.json",
                _frame("1936.1.1.1", 100, sequence=2, pid=123),
            )
            manifest["samples"].append({
                "frame": "collector_started_0001.json",
                "current_game_date_raw": 100,
            })
            unique_frames = cs._frames_from_probe_manifest(run_dir, manifest)
            all_frames = cs._frames_from_probe_manifest(
                run_dir,
                manifest,
                unique_game_hours_only=False,
            )
            self.assertEqual([f["sequence"] for f in unique_frames], [1])
            self.assertEqual([f["sequence"] for f in all_frames], [1, 2])

            _write_json(
                run_dir / "collector_started_0002.json",
                _frame(
                    "1936.1.1.1",
                    100,
                    sequence=3,
                    pid=123,
                    capture_boundary="scheduled_hour_snapshot",
                ),
            )
            manifest["samples"].append({
                "frame": "collector_started_0002.json",
                "current_game_date_raw": 100,
            })
            scheduled_preferred = cs._frames_from_probe_manifest(run_dir, manifest)
            self.assertEqual([f["sequence"] for f in scheduled_preferred], [3])

            frame["session_id"] = "other_session"
            _write_json(run_dir / "collector_started_0000.json", frame)
            manifest["session_id"] = "session_x"
            with self.assertRaisesRegex(RuntimeError, "session_id"):
                cs._frames_from_probe_manifest(run_dir, manifest)

    def test_manifest_loader_preserves_raw_stream_frames_only_in_full_stream(self):
        with tempfile.TemporaryDirectory() as td:
            run_dir = Path(td) / "session_x"
            run_dir.mkdir()
            _write_json(
                run_dir / "collector_started_0000.json",
                _frame("1936.1.1.1", 100, sequence=1, pid=123),
            )
            _write_json(
                run_dir / "collector_raw_0000.json",
                _raw_clock_delta("1936.1.1.1", sequence=2, pid=123),
            )
            _write_json(
                run_dir / "collector_started_0001.json",
                _frame("1936.1.1.2", 101, sequence=3, pid=123),
            )
            manifest = {
                "type": "collector_probe_manifest",
                "samples": [
                    {
                        "frame": "collector_started_0000.json",
                        "current_game_date_raw": 100,
                        "worker_burst_index": 0,
                        "burst_sample_index": 0,
                    },
                    {
                        "frame": "collector_started_0001.json",
                        "current_game_date_raw": 101,
                        "worker_burst_index": 0,
                        "burst_sample_index": 2,
                    },
                ],
                "raw_frames": [{
                    "frame": "collector_raw_0000.json",
                    "worker_burst_index": 0,
                    "burst_sample_index": 1,
                }],
            }

            unique_frames = cs._frames_from_probe_manifest(run_dir, manifest)
            all_frames = cs._frames_from_probe_manifest(
                run_dir,
                manifest,
                unique_game_hours_only=False,
            )

        self.assertEqual([f["sequence"] for f in unique_frames], [1, 3])
        self.assertEqual([f["sequence"] for f in all_frames], [1, 2, 3])
        self.assertEqual(all_frames[1]["type"], "domain_delta")
        self.assertEqual(all_frames[1]["session_id"], "session_x")
        self.assertEqual(all_frames[1]["worker_burst_index"], 0)
        self.assertEqual(all_frames[1]["burst_sample_index"], 1)

    def test_collector_frame_session_stamps_continuity_and_replay_diagnostics(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            replay_path = root / "session.battleplan"
            session = CollectorFrameSession(
                game_id="frame_session",
                store_root=root / "store",
                replay_path=replay_path,
                pid=333,
                process_instance_id="proc-test",
                finalize_every=2,
            )
            results = session.record_frames([
                _frame("1936.1.1.1", 100, sequence=10, pid=333),
                _frame("1936.1.1.1", 100, sequence=11, pid=333),
                _frame("1936.1.1.3", 102, sequence=12, pid=333),
            ])

            self.assertEqual([r.skipped_reason for r in results],
                             [None, "duplicate_hour", None])
            self.assertEqual(sum(1 for r in results if r.written_path), 2)
            self.assertTrue(_verify_container(str(replay_path)))

            index = _read_json(root / "store" / "frame_session" / "index.json")
            self.assertEqual(len(index["snapshots"]), 2)
            first_snapshot_path = (
                root / "store" / "frame_session" / index["snapshots"][0]["file"]
            )
            first_snapshot = _read_json(first_snapshot_path)
            self.assertEqual(first_snapshot["_capture"]["pid"], 333)
            self.assertEqual(first_snapshot["_capture"]["process_instance_id"],
                             "proc-test")

            with open(replay_path, "rb") as fh:
                by_name = _zip_read_all(fh.read())
            manifest = json.loads(by_name["manifest.json"].decode("utf-8"))
            diagnostics = manifest["recording_diagnostics"]
            self.assertEqual(diagnostics["cadence"]["skipped_hours"], 1)
            self.assertEqual(
                diagnostics["collector"]["transport_sequence"]["count"], 3)
            self.assertTrue(
                diagnostics["collector"]["transport_sequence"]["monotonic"])

    def test_collector_frame_session_rejects_invalid_frame_without_write(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            session = CollectorFrameSession(
                game_id="bad_frame",
                store_root=root / "store",
                replay_path=root / "bad_frame.battleplan",
                pid=444,
            )
            results = session.record_frames([
                {"type": "snapshot", "date": "1936.1.1.1", "domains": ["bogus"]},
            ])

            self.assertEqual(len(results), 1)
            self.assertIsNotNone(results[0].error)
            self.assertEqual(session.invalid_frames, 1)
            self.assertFalse((root / "store" / "bad_frame" / "index.json").exists())


def _write_json(path, data):
    path.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n",
                    encoding="utf-8")


def _read_json(path):
    return json.loads(path.read_text(encoding="utf-8"))


class _PreflightResult:
    def __init__(
        self,
        diagnostics,
        *,
        state_roots_encoded=None,
        province_bridge_encoded=None,
        controller_bridge_encoded=None,
    ):
        self.diagnostics = diagnostics
        self.status = diagnostics.get("status")
        self.row_count = diagnostics.get("row_count", 0)
        self.state_roots_encoded = state_roots_encoded
        self.province_bridge_encoded = province_bridge_encoded
        self.controller_bridge_encoded = controller_bridge_encoded

    @property
    def usable(self):
        return self.status == "resolved" and self.row_count >= 1000


class _DivisionPreflightResult:
    def __init__(self, diagnostics, *, encoded):
        self.diagnostics = diagnostics
        self.status = diagnostics.get("status")
        self.row_count = diagnostics.get("row_count", 0)
        self.encoded = encoded

    @property
    def usable(self):
        return self.status == "resolved" and bool(self.encoded)


class _AdvisorPreflightResult:
    def __init__(self, diagnostics, *, encoded):
        self.diagnostics = diagnostics
        self.status = diagnostics.get("status")
        self.token_count = diagnostics.get("token_count", 0)
        self.encoded = encoded

    @property
    def usable(self):
        return self.status == "resolved" and bool(self.encoded)


def _frame(date, raw, *, sequence, pid, capture_boundary=None):
    frame = {
        "type": "collector_started",
        "backend": "in_process_collector",
        "collector_version": "0.1.0",
        "abi_version": 1,
        "process_id": pid,
        "tick_ms": sequence,
        "sequence": sequence,
        "quality_status": "root_probe_verified",
        "root_probe": {
            "status": "verified",
            "profile": "vanilla_latest_2026_06_28",
            "daily_update_sentinel_match": True,
            "long_string_sentinel_match": True,
            "current_game_date": date,
            "current_game_date_raw": raw,
            "start_date": "1936.1.1.1",
            "start_date_raw": 100,
            "country_tag_count": 3,
            "country_active_count": 2,
            "tag_sample": ["---", "GER", "ENG"],
            "domains": ["clock"],
        },
    }
    if capture_boundary is not None:
        frame["capture_boundary"] = capture_boundary
    return {
        **frame,
    }


def _hour_index(date):
    year, month, day, hour = [int(part) for part in date.split(".")]
    month_days = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    return (year * 365 + sum(month_days[:month - 1]) + (day - 1)) * 24 + hour


def _raw_clock_delta(date, *, sequence, pid):
    hour_index = _hour_index(date)
    return {
        "type": "domain_delta",
        "backend": "in_process_collector",
        "collector_version": "0.1.0",
        "abi_version": 1,
        "process_id": pid,
        "tick_ms": sequence,
        "sequence": sequence,
        "date": date,
        "hour_index": hour_index,
        "sample_ms": 10,
        "capture_boundary": "worker_poll",
        "domain": "clock",
        "entity_key": "clock:game_date",
        "source": "C",
        "confidence": "sampled",
        "payload": {
            "op": "sample",
            "after": {
                "date": date,
                "hour_index": hour_index,
                "raw_date": 100,
            },
            "changed_fields": ["date", "hour_index", "raw_date"],
        },
    }


if __name__ == "__main__":
    unittest.main()
