import os
import sys
import json
from pathlib import Path
from types import SimpleNamespace
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))

from live_capture.collector_probe_record import (  # noqa: E402
    encode_country_object_bridge,
    encode_country_record_bridge,
    encode_scheduled_domains,
    hourly_requirement_diagnostics,
    record_probes,
    summarize_probe_samples,
)
from live_capture.collector_division_bridge import (  # noqa: E402
    encode_division_body_bridge,
)
from live_capture.collector_army_order_bridge import (  # noqa: E402
    encode_army_body_bridge,
)
from live_capture.collector_naval_combat_bridge import (  # noqa: E402
    encode_naval_combat_body_bridge,
)
from live_capture.collector_air_wing_bridge import (  # noqa: E402
    encode_air_wing_body_bridge,
)
from live_capture.collector_ship_bridge import (  # noqa: E402
    encode_ship_body_bridge,
)
from live_capture.collector_advisor_tokens import (  # noqa: E402
    encode_advisor_token_filter,
)
from live_capture.collector_idea_groups import (  # noqa: E402
    encode_idea_group_filter,
)
import live_capture.collector_probe_record as cpr  # noqa: E402


class TestCollectorProbeRecord(unittest.TestCase):
    def test_encode_scheduled_domains_sorts_nothing_and_deduplicates(self):
        self.assertEqual(
            encode_scheduled_domains([
                "focus_live",
                "projects_live",
                "focus_live",
                "decisions_live",
            ]),
            "focus_live;projects_live;decisions_live",
        )
        self.assertIsNone(encode_scheduled_domains(["", "  "]))
        with self.assertRaises(ValueError):
            encode_scheduled_domains(["projects-live"])

    def test_hourly_summary_counts_no_skips_for_consecutive_hours(self):
        summary = summarize_probe_samples([
            _sample("1936.1.1.1", 100, sequence=10, stored=True),
            _sample("1936.1.1.2", 101, sequence=11, stored=True),
            _sample("1936.1.1.3", 102, sequence=12, stored=True),
        ])

        self.assertEqual(summary["game_hours_advanced"], 2)
        self.assertEqual(summary["positive_game_hour_deltas"], [1, 1])
        self.assertEqual(summary["skipped_game_hours"], 0)
        self.assertTrue(summary["hourly_cadence_ok"])
        self.assertEqual(summary["stored_snapshot_count"], 3)

    def test_hourly_summary_counts_duplicate_samples_and_skips(self):
        summary = summarize_probe_samples([
            _sample("1936.1.1.1", 100, sequence=10, stored=True),
            _sample("1936.1.1.1", 100, sequence=11, skip_reason="duplicate_hour"),
            _sample("1936.1.1.4", 103, sequence=12, stored=True),
        ])

        self.assertEqual(summary["game_hours_advanced"], 3)
        self.assertEqual(summary["positive_game_hour_deltas"], [3])
        self.assertEqual(summary["skipped_game_hours"], 2)
        self.assertFalse(summary["hourly_cadence_ok"])
        self.assertEqual(summary["duplicate_game_hour_samples"], 1)
        self.assertEqual(summary["duplicate_hour_store_skips"], 1)

    def test_summary_counts_country_object_identity_status(self):
        first = _sample("1936.1.1.1", 100, sequence=10, stored=True)
        first.update({
            "domains": ["clock", "country_identity"],
            "country_object_identity_status": "country_database_root_insufficient",
            "country_object_identity_object_count": 0,
            "country_object_identity_scan_ms": 15,
            "country_scalars_domain_present": False,
            "politics_live_domain_present": False,
            "politics_residuals_domain_present": False,
        })
        second = _sample("1936.1.1.2", 101, sequence=11, stored=True)
        second.update({
            "domains": [
                "clock",
                "country_identity",
                "country_scalars",
                "politics_live",
                "politics_residuals",
            ],
            "country_object_identity_status": "country_database_root_verified",
            "country_object_identity_object_count": 88,
            "country_object_identity_scan_ms": 3,
            "country_scalars_domain_present": True,
            "politics_live_domain_present": True,
            "politics_residuals_domain_present": True,
        })

        summary = summarize_probe_samples([first, second])

        self.assertEqual(
            summary["domains_seen"],
            [
                "clock",
                "country_identity",
                "country_scalars",
                "politics_live",
                "politics_residuals",
            ],
        )
        self.assertEqual(summary["country_object_identity_statuses"], {
            "country_database_root_insufficient": 1,
            "country_database_root_verified": 1,
        })
        self.assertEqual(summary["country_object_identity_max_object_count"], 88)
        self.assertEqual(summary["country_object_identity_max_scan_ms"], 15)
        self.assertEqual(summary["country_scalars_samples"], 1)
        self.assertEqual(summary["politics_live_samples"], 1)
        self.assertEqual(summary["politics_residuals_samples"], 1)

    def test_hourly_requirement_passes_for_requested_consecutive_hours(self):
        summary = summarize_probe_samples([
            _sample("1936.1.1.1", 100, sequence=10, stored=True),
            _sample("1936.1.1.2", 101, sequence=11, stored=True),
            _sample("1936.1.1.3", 102, sequence=12, stored=True),
        ])

        requirement = hourly_requirement_diagnostics(
            summary, required=True, requested_game_hours=2
        )

        self.assertTrue(requirement["met"])
        self.assertEqual(requirement["problems"], [])

    def test_hourly_requirement_reports_skipped_and_short_runs(self):
        summary = summarize_probe_samples([
            _sample("1936.1.1.1", 100, sequence=10, stored=True),
            _sample("1936.1.1.4", 103, sequence=11, stored=True),
        ])

        requirement = hourly_requirement_diagnostics(
            summary, required=True, requested_game_hours=4
        )

        self.assertFalse(requirement["met"])
        self.assertIn("skipped_game_hours", requirement["problems"])
        self.assertIn("requested_game_hours_not_reached", requirement["problems"])

    def test_hourly_requirement_reports_wall_time_overrun(self):
        summary = summarize_probe_samples([
            _sample("1936.1.1.1", 100, sequence=10, stored=True),
            _sample("1936.1.1.2", 101, sequence=11, stored=True),
            _sample("1936.1.1.3", 102, sequence=12, stored=True),
        ])

        requirement = hourly_requirement_diagnostics(
            summary,
            required=True,
            requested_game_hours=2,
            elapsed_wall_s=36.0,
            max_wall_s=15.0,
        )

        self.assertFalse(requirement["met"])
        self.assertIn("max_wall_s_exceeded", requirement["problems"])

    def test_hourly_requirement_can_be_diagnostic_only(self):
        summary = summarize_probe_samples([])

        requirement = hourly_requirement_diagnostics(
            summary, required=False, requested_game_hours=4
        )

        self.assertTrue(requirement["met"])
        self.assertEqual(requirement["problems"], [])

    def test_record_probes_writes_hourly_requirement_failure(self):
        original_attach = cpr._attach_with_retries
        frames = [
            _frame("1936.1.1.1", 100, sequence=10),
            _frame("1936.1.1.3", 102, sequence=11),
        ]
        cpr._attach_with_retries = lambda *a, **kw: SimpleNamespace(
            pipe_frames=frames, pipe_frame=frames[-1], thread_id=123
        )
        try:
            with tempfile.TemporaryDirectory() as td:
                run_dir = record_probes(
                    111,
                    Path("collector.dll"),
                    Path(td),
                    until_game_hours=2,
                    require_hourly_cadence=True,
                )
                manifest = json.loads((run_dir / "manifest.json").read_text(encoding="utf-8"))
        finally:
            cpr._attach_with_retries = original_attach

        requirement = manifest["hourly_requirement"]
        self.assertFalse(requirement["met"])
        self.assertEqual(requirement["requested_game_hours"], 2)
        self.assertIn("skipped_game_hours", requirement["problems"])

    def test_worker_control_uses_unique_pipe_and_disables_sidecar(self):
        with tempfile.TemporaryDirectory() as td:
            dll = Path(td) / "collector.dll"
            control_path, pipe_token = cpr._write_worker_control(
                dll,
                3.0,
                25,
                session_id="session-1",
                army_body_bridge="17:0x7000",
                naval_combat_body_bridge="41:0x8000",
                air_wing_body_bridge="1:3:0x9000",
                ship_body_bridge="3458:0xA000",
                special_projects_bridge="facility:1:118:0x10000000000",
                tech_def_filter="4660=basic_machine_tools",
            )
            payload = json.loads(control_path.read_text(encoding="utf-8"))

        self.assertEqual(payload["mode"], "hourly_poll")
        self.assertEqual(payload["pipe_token"], pipe_token)
        self.assertEqual(payload["session_id"], "session-1")
        self.assertEqual(payload["army_body_bridge"], "17:0x7000")
        self.assertEqual(payload["naval_combat_body_bridge"], "41:0x8000")
        self.assertEqual(payload["air_wing_body_bridge"], "1:3:0x9000")
        self.assertEqual(payload["ship_body_bridge"], "3458:0xA000")
        self.assertEqual(
            payload["special_projects_bridge"], "facility:1:118:0x10000000000")
        self.assertEqual(payload["tech_def_filter"], "4660=basic_machine_tools")
        self.assertFalse(payload["country_object_scan"])
        self.assertFalse(payload["special_projects_scan"])
        self.assertFalse(payload["sidecar_log"])
        self.assertEqual(payload["sample_ms"], 25)
        self.assertEqual(payload["scheduled_snapshot_step_hours"], 1)

    def test_encode_country_object_bridge_sorts_and_sanitizes_rows(self):
        encoded = encode_country_object_bridge({
            2: "0x2000",
            "1": 0x1000,
        })

        self.assertEqual(encoded, "1:0x1000;2:0x2000")

    def test_encode_country_record_bridge_uses_record_ptr(self):
        encoded = encode_country_record_bridge([
            {"tag_db_id": 2, "record_ptr": "0x2200"},
            {"tag_db_id": 1, "record_ptr": 0x1200},
        ])

        self.assertEqual(encoded, "1:0x1200;2:0x2200")

    def test_encode_division_body_bridge_sorts_and_sanitizes_rows(self):
        encoded = encode_division_body_bridge({
            18: "0x9000",
            "17": 0x8000,
        })

        self.assertEqual(encoded, "17:0x8000;18:0x9000")

    def test_encode_army_body_bridge_sorts_and_sanitizes_rows(self):
        encoded = encode_army_body_bridge({
            18: "0x9000",
            "17": 0x8000,
        })

        self.assertEqual(encoded, "17:0x8000;18:0x9000")

    def test_encode_naval_combat_body_bridge_sorts_and_sanitizes_rows(self):
        encoded = encode_naval_combat_body_bridge([
            {"combat_id": 42, "body_addr": "0x9000"},
            {"combat_id": "41", "body_ptr": 0x8000},
        ])

        self.assertEqual(encoded, "41:0x8000;42:0x9000")

    def test_encode_air_wing_body_bridge_sorts_and_sanitizes_rows(self):
        encoded = encode_air_wing_body_bridge([
            {"owner_tagid": 2, "wing_id": "4", "body_addr": "0x9000"},
            {"owner_tag_db_id": "1", "wing_id": 3, "body_ptr": 0x8000},
        ])

        self.assertEqual(encoded, "1:3:0x8000;2:4:0x9000")

    def test_encode_ship_body_bridge_sorts_and_sanitizes_rows(self):
        encoded = encode_ship_body_bridge({
            3459: "0x9000",
            "3458": 0x8000,
        })

        self.assertEqual(encoded, "3458:0x8000;3459:0x9000")

    def test_encode_advisor_token_filter_sorts_dedupes_and_rejects_bad_tokens(self):
        encoded = encode_advisor_token_filter([
            "POR_salazar",
            "ENG_maxwell_aitken",
            "POR_salazar",
        ])

        self.assertEqual(encoded, "ENG_maxwell_aitken;POR_salazar")
        with self.assertRaises(ValueError):
            encode_advisor_token_filter(["POR_salazar", "bad token!"])

    def test_encode_advisor_token_filter_preserves_static_metadata(self):
        encoded = encode_advisor_token_filter({
            "POR_salazar": {
                "slot": "political_advisor",
                "ledger": "political",
                "character": "POR_antonio_de_oliveira_salazar",
            },
            "ENG_maxwell_aitken": {
                "slot": "high_command",
                "ledger": None,
                "character": "ENG_maxwell_aitken",
            },
        })

        self.assertEqual(
            encoded,
            (
                "ENG_maxwell_aitken=high_command||ENG_maxwell_aitken;"
                "POR_salazar=political_advisor|political|"
                "POR_antonio_de_oliveira_salazar"
            ),
        )
        with self.assertRaises(ValueError):
            encode_advisor_token_filter({
                "POR_salazar": {"slot": "bad slot!"},
            })

    def test_encode_idea_group_filter_sorts_dedupes_and_rejects_bad_rows(self):
        encoded = encode_idea_group_filter([
            ("free_trade", "trade_laws"),
            ("war_economy", "economy"),
            ("free_trade", "trade_laws"),
        ])

        self.assertEqual(encoded, "free_trade=trade_laws;war_economy=economy")
        with self.assertRaises(ValueError):
            encode_idea_group_filter([("war_economy", "bad group!")])
        with self.assertRaises(ValueError):
            encode_idea_group_filter([
                ("war_economy", "economy"),
                ("war_economy", "trade_laws"),
            ])

    def test_encode_country_object_bridge_rejects_bad_rows(self):
        with self.assertRaises(ValueError):
            encode_country_object_bridge([(1, 0x1001)])
        with self.assertRaises(ValueError):
            encode_country_object_bridge([(1, 0x1000), (1, 0x2000)])
        with self.assertRaises(ValueError):
            encode_country_object_bridge([(1, 0x1000), (2, 0x1000)])

    def test_worker_control_writes_bridge_without_enabling_heap_scan(self):
        with tempfile.TemporaryDirectory() as td:
            dll = Path(td) / "collector.dll"
            control_path, _pipe_token = cpr._write_worker_control(
                dll,
                3.0,
                25,
                session_id="session-1",
                country_object_bridge="1:0x1000;2:0x2000",
                country_record_bridge="1:0x1200;2:0x2200",
                division_body_bridge="17:0x8000;18:0x9000",
                ship_body_bridge="3458:0xA000;3459:0xB000",
                map_control_state_roots="1:0x3000;2:0x4000",
                map_control_province_bridge=(
                    "province_array=0x5000;province_count=13232;"
                    "province_stride=8;controller_ptr_offset=0"
                ),
                map_control_controller_bridge="1:0x6000;2:0x7000",
                scheduled_snapshot_step_hours=1,
                advisor_token_filter="ENG_maxwell_aitken;POR_salazar",
                idea_group_filter="free_trade=trade_laws;war_economy=economy",
            )
            payload = json.loads(control_path.read_text(encoding="utf-8"))

        self.assertEqual(payload["country_object_bridge"], "1:0x1000;2:0x2000")
        self.assertEqual(payload["country_record_bridge"], "1:0x1200;2:0x2200")
        self.assertEqual(payload["division_body_bridge"], "17:0x8000;18:0x9000")
        self.assertEqual(payload["ship_body_bridge"], "3458:0xA000;3459:0xB000")
        self.assertEqual(payload["map_control_state_roots"], "1:0x3000;2:0x4000")
        self.assertEqual(
            payload["map_control_province_bridge"],
            "province_array=0x5000;province_count=13232;"
            "province_stride=8;controller_ptr_offset=0",
        )
        self.assertEqual(payload["map_control_controller_bridge"], "1:0x6000;2:0x7000")
        self.assertEqual(
            payload["advisor_token_filter"],
            "ENG_maxwell_aitken;POR_salazar",
        )
        self.assertEqual(
            payload["idea_group_filter"],
            "free_trade=trade_laws;war_economy=economy",
        )
        self.assertFalse(payload["country_object_scan"])

    def test_record_probes_manifest_tracks_worker_session_id(self):
        original_attach = cpr._attach_with_retries
        frames = [_frame("1936.1.1.1", 100, sequence=10)]
        cpr._attach_with_retries = lambda *a, **kw: SimpleNamespace(
            pipe_frames=frames, pipe_frame=frames[-1], thread_id=123
        )
        try:
            with tempfile.TemporaryDirectory() as td:
                run_dir = record_probes(
                    111,
                    Path("collector.dll"),
                    Path(td),
                    worker_duration_s=1.0,
                    worker_sample_ms=25,
                )
                manifest = json.loads((run_dir / "manifest.json").read_text(encoding="utf-8"))
        finally:
            cpr._attach_with_retries = original_attach

        self.assertEqual(manifest["session_id"], run_dir.name)
        self.assertEqual(manifest["dll"], "collector.dll")
        self.assertFalse(manifest["country_object_scan"])
        self.assertFalse(manifest["special_projects_scan"])
        self.assertFalse(manifest["country_object_bridge_supplied"])
        self.assertEqual(manifest["country_object_bridge_rows"], 0)
        self.assertFalse(manifest["country_record_bridge_supplied"])
        self.assertEqual(manifest["country_record_bridge_rows"], 0)
        self.assertFalse(manifest["division_body_bridge_supplied"])
        self.assertEqual(manifest["division_body_bridge_rows"], 0)
        self.assertFalse(manifest["air_wing_body_bridge_supplied"])
        self.assertEqual(manifest["air_wing_body_bridge_rows"], 0)
        self.assertFalse(manifest["ship_body_bridge_supplied"])
        self.assertEqual(manifest["ship_body_bridge_rows"], 0)
        self.assertFalse(manifest["advisor_token_filter_supplied"])
        self.assertEqual(manifest["advisor_token_filter_tokens"], 0)
        self.assertIsNone(manifest["advisor_token_filter_sha256"])
        self.assertFalse(manifest["idea_group_filter_supplied"])
        self.assertEqual(manifest["idea_group_filter_rows"], 0)
        self.assertIsNone(manifest["idea_group_filter_sha256"])
        self.assertNotIn(str(Path("collector.dll").resolve().parent),
                         json.dumps(manifest, sort_keys=True))

    def test_record_probes_manifest_tracks_bridge_count_without_addresses(self):
        original_attach = cpr._attach_with_retries
        frames = [_frame("1936.1.1.1", 100, sequence=10)]
        cpr._attach_with_retries = lambda *a, **kw: SimpleNamespace(
            pipe_frames=frames, pipe_frame=frames[-1], thread_id=123
        )
        try:
            with tempfile.TemporaryDirectory() as td:
                run_dir = record_probes(
                    111,
                    Path("collector.dll"),
                    Path(td),
                    worker_duration_s=1.0,
                    worker_sample_ms=25,
                    country_object_bridge={1: 0x1000, 2: 0x2000},
                    country_record_bridge={1: 0x1200, 2: 0x2200},
                    division_body_bridge={17: 0x8000, 18: 0x9000},
                    air_wing_body_bridge=[
                        {"owner_tag_db_id": 1, "wing_id": 3, "body_ptr": 0xA000},
                        {"owner_tag_db_id": 2, "wing_id": 4, "body_ptr": 0xB000},
                    ],
                    ship_body_bridge={
                        3458: 0xC000,
                        3459: 0xD000,
                    },
                    map_control_state_roots="1:0x3000;2:0x4000",
                    map_control_province_bridge=(
                        "province_array=0x5000;province_count=13232;"
                        "province_stride=8;controller_ptr_offset=0"
                    ),
                    map_control_controller_bridge="1:0x6000;2:0x7000",
                    advisor_token_filter=[
                        "POR_salazar",
                        "ENG_maxwell_aitken",
                        "POR_salazar",
                    ],
                    idea_group_filter={
                        "war_economy": "economy",
                        "free_trade": "trade_laws",
                    },
                )
                manifest = json.loads((run_dir / "manifest.json").read_text(encoding="utf-8"))
        finally:
            cpr._attach_with_retries = original_attach

        self.assertTrue(manifest["country_object_bridge_supplied"])
        self.assertEqual(manifest["country_object_bridge_rows"], 2)
        self.assertTrue(manifest["country_record_bridge_supplied"])
        self.assertEqual(manifest["country_record_bridge_rows"], 2)
        self.assertTrue(manifest["division_body_bridge_supplied"])
        self.assertEqual(manifest["division_body_bridge_rows"], 2)
        self.assertTrue(manifest["air_wing_body_bridge_supplied"])
        self.assertEqual(manifest["air_wing_body_bridge_rows"], 2)
        self.assertTrue(manifest["ship_body_bridge_supplied"])
        self.assertEqual(manifest["ship_body_bridge_rows"], 2)
        self.assertTrue(manifest["map_control_state_roots_supplied"])
        self.assertEqual(manifest["map_control_state_root_rows"], 2)
        self.assertTrue(manifest["map_control_province_bridge_supplied"])
        self.assertTrue(manifest["map_control_controller_bridge_supplied"])
        self.assertEqual(manifest["map_control_controller_bridge_rows"], 2)
        self.assertTrue(manifest["advisor_token_filter_supplied"])
        self.assertEqual(manifest["advisor_token_filter_tokens"], 2)
        self.assertEqual(len(manifest["advisor_token_filter_sha256"]), 64)
        self.assertTrue(manifest["idea_group_filter_supplied"])
        self.assertEqual(manifest["idea_group_filter_rows"], 2)
        self.assertEqual(len(manifest["idea_group_filter_sha256"]), 64)
        self.assertNotIn("0x1000", json.dumps(manifest, sort_keys=True))
        self.assertNotIn("0x1200", json.dumps(manifest, sort_keys=True))
        self.assertNotIn("0x8000", json.dumps(manifest, sort_keys=True))
        self.assertNotIn("0x9000", json.dumps(manifest, sort_keys=True))
        self.assertNotIn("0xA000", json.dumps(manifest, sort_keys=True))
        self.assertNotIn("0xB000", json.dumps(manifest, sort_keys=True))
        self.assertNotIn("0xC000", json.dumps(manifest, sort_keys=True))
        self.assertNotIn("0xD000", json.dumps(manifest, sort_keys=True))
        self.assertNotIn("0x3000", json.dumps(manifest, sort_keys=True))
        self.assertNotIn("0x5000", json.dumps(manifest, sort_keys=True))
        self.assertNotIn("0x6000", json.dumps(manifest, sort_keys=True))
        self.assertNotIn("POR_salazar", json.dumps(manifest, sort_keys=True))
        self.assertNotIn("ENG_maxwell_aitken", json.dumps(manifest, sort_keys=True))
        self.assertNotIn("war_economy", json.dumps(manifest, sort_keys=True))
        self.assertNotIn("free_trade", json.dumps(manifest, sort_keys=True))

    def test_record_probes_can_skip_snapshot_sidecar_artifacts(self):
        original_attach = cpr._attach_with_retries
        frames = [
            _frame("1936.1.1.1", 100, sequence=10),
            _frame("1936.1.1.2", 101, sequence=11),
        ]
        cpr._attach_with_retries = lambda *a, **kw: SimpleNamespace(
            pipe_frames=frames, pipe_frame=frames[-1], thread_id=123
        )
        try:
            with tempfile.TemporaryDirectory() as td:
                run_dir = record_probes(
                    111,
                    Path("collector.dll"),
                    Path(td),
                    write_snapshot_artifacts=False,
                )
                manifest = json.loads((run_dir / "manifest.json").read_text(encoding="utf-8"))
        finally:
            cpr._attach_with_retries = original_attach

        self.assertIsNone(manifest["first_snapshot"])
        self.assertFalse((run_dir / "snapshot.json").exists())
        self.assertFalse((run_dir / "snapshot_0000.json").exists())
        self.assertEqual(manifest["samples"][0]["snapshot"], None)

    def test_duplicate_hour_skips_large_snapshot_artifact(self):
        original_attach = cpr._attach_with_retries
        frames = [
            _frame("1936.1.1.1", 100, sequence=10),
            _frame("1936.1.1.1", 100, sequence=11),
            _frame("1936.1.1.2", 101, sequence=12),
        ]
        cpr._attach_with_retries = lambda *a, **kw: SimpleNamespace(
            pipe_frames=frames, pipe_frame=frames[-1], thread_id=123
        )
        try:
            with tempfile.TemporaryDirectory() as td:
                root = Path(td)
                run_dir = record_probes(
                    111,
                    Path("collector.dll"),
                    root / "out",
                    store_root=root / "store",
                    unique_store_hours_only=True,
                )
                manifest = json.loads((run_dir / "manifest.json").read_text(encoding="utf-8"))
        finally:
            cpr._attach_with_retries = original_attach

        self.assertIsNone(manifest["samples"][1]["snapshot"])
        self.assertEqual(manifest["samples"][1]["store_skipped_reason"], "duplicate_hour")
        self.assertFalse((run_dir / "snapshot_0001.json").exists())

    def test_restarted_worker_sequences_are_diagnosed_per_burst(self):
        summary = summarize_probe_samples([
            _sample("1936.1.1.1", 100, sequence=1, burst=0),
            _sample("1936.1.1.2", 101, sequence=2, burst=0),
            _sample("1936.1.1.3", 102, sequence=1, burst=1),
            _sample("1936.1.1.4", 103, sequence=2, burst=1),
        ])

        self.assertEqual(summary["transport_sequence_mode"], "burst_local")
        self.assertFalse(summary["monotonic_transport_sequence"])
        self.assertEqual(summary["duplicate_transport_sequences"], 2)
        self.assertTrue(summary["burst_local_monotonic_transport_sequence"])
        self.assertEqual(summary["burst_local_duplicate_transport_sequences"], 0)
        self.assertEqual(len(summary["transport_bursts"]), 2)

    def test_record_probes_writes_partial_manifest_after_later_attach_error(self):
        original_attach = cpr._attach_with_retries
        calls = {"count": 0}
        frames = [_frame("1936.1.1.1", 100, sequence=1)]

        def fake_attach(*args, **kwargs):
            del args, kwargs
            calls["count"] += 1
            if calls["count"] == 1:
                return SimpleNamespace(
                    pipe_frames=frames, pipe_frame=frames[-1], thread_id=123
                )
            fake_path = "C:" + "\\temp\\collector.dll"
            raise cpr.AttachError("pipe timed out at " + fake_path)

        cpr._attach_with_retries = fake_attach
        try:
            with tempfile.TemporaryDirectory() as td:
                root = Path(td)
                dll = root / "collector.dll"
                run_dir = record_probes(
                    111,
                    dll,
                    root / "out",
                    repeat=2,
                    worker_duration_s=1.0,
                    worker_sample_ms=25,
                )
                manifest = json.loads(
                    (run_dir / "manifest.json").read_text(encoding="utf-8"))
                raw_file_exists = (run_dir / "collector_raw_0000.json").exists()
                second_snapshot_exists = (run_dir / "snapshot_0001.json").exists()
        finally:
            cpr._attach_with_retries = original_attach

        self.assertEqual(manifest["stop_reason"], "attach_error")
        self.assertEqual(manifest["sample_count"], 1)
        self.assertTrue(manifest["partial_session"])
        self.assertEqual(len(manifest["attach_errors"]), 1)
        self.assertNotIn(str(root), json.dumps(manifest, sort_keys=True))

    def test_record_probes_persists_raw_pipe_frames_without_counting_samples(self):
        original_attach = cpr._attach_with_retries
        frames = [
            _frame("1936.1.1.1", 100, sequence=1),
            _raw_clock_delta("1936.1.1.1", sequence=2),
        ]
        cpr._attach_with_retries = lambda *a, **kw: SimpleNamespace(
            pipe_frames=frames, pipe_frame=frames[-1], thread_id=123
        )
        try:
            with tempfile.TemporaryDirectory() as td:
                root = Path(td)
                run_dir = record_probes(
                    111,
                    root / "collector.dll",
                    root / "out",
                    worker_duration_s=1.0,
                    worker_sample_ms=10,
                )
                manifest = json.loads(
                    (run_dir / "manifest.json").read_text(encoding="utf-8"))
                raw_file_exists = (run_dir / "collector_raw_0000.json").exists()
                second_snapshot_exists = (run_dir / "snapshot_0001.json").exists()
        finally:
            cpr._attach_with_retries = original_attach

        self.assertEqual(manifest["sample_count"], 1)
        self.assertEqual(manifest["raw_pipe_frame_count"], 1)
        self.assertEqual(manifest["raw_frames"][0]["frame"], "collector_raw_0000.json")
        self.assertEqual(manifest["raw_frames"][0]["type"], "domain_delta")
        self.assertEqual(manifest["raw_frames"][0]["domain"], "clock")
        self.assertEqual(manifest["raw_frames"][0]["sequence"], 2)
        self.assertTrue(raw_file_exists)
        self.assertFalse(second_snapshot_exists)


def _sample(date, raw, *, sequence, stored=False, skip_reason=None, burst=None):
    sample = {
        "current_game_date": date,
        "current_game_date_raw": raw,
        "sequence": sequence,
        "store_path": "snapshot.json" if stored else None,
        "store_skipped_reason": skip_reason,
    }
    if burst is not None:
        sample["worker_burst_index"] = burst
    return sample


def _frame(date, raw, *, sequence):
    return {
        "type": "collector_started",
        "backend": "in_process_collector",
        "collector_version": "0.1.0",
        "abi_version": 1,
        "process_id": 111,
        "tick_ms": sequence,
        "sequence": sequence,
        "quality_status": "root_probe_verified",
        "root_probe": {
            "status": "verified",
            "profile": "test",
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


def _hour_index(date):
    year, month, day, hour = [int(part) for part in date.split(".")]
    month_days = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    return (year * 365 + sum(month_days[:month - 1]) + (day - 1)) * 24 + hour


def _raw_clock_delta(date, *, sequence):
    hour_index = _hour_index(date)
    return {
        "type": "domain_delta",
        "backend": "in_process_collector",
        "collector_version": "0.1.0",
        "abi_version": 1,
        "process_id": 111,
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
