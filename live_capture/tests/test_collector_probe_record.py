import os
import sys
import json
from pathlib import Path
from types import SimpleNamespace
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))

from live_capture.collector_probe_record import (  # noqa: E402
    hourly_requirement_diagnostics,
    record_probes,
    summarize_probe_samples,
)
import live_capture.collector_probe_record as cpr  # noqa: E402


class TestCollectorProbeRecord(unittest.TestCase):
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
            control_path, pipe_token = cpr._write_worker_control(dll, 3.0, 25)
            payload = json.loads(control_path.read_text(encoding="utf-8"))

        self.assertEqual(payload["mode"], "hourly_poll")
        self.assertEqual(payload["pipe_token"], pipe_token)
        self.assertFalse(payload["sidecar_log"])
        self.assertEqual(payload["sample_ms"], 25)

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


def _sample(date, raw, *, sequence, stored=False, skip_reason=None):
    return {
        "current_game_date": date,
        "current_game_date_raw": raw,
        "sequence": sequence,
        "store_path": "snapshot.json" if stored else None,
        "store_skipped_reason": skip_reason,
    }


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


if __name__ == "__main__":
    unittest.main()
