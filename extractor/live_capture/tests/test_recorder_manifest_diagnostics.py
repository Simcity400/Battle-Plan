"""Tests for recorder-local .battleplan diagnostics patching."""
from __future__ import annotations

import json
import os
import shutil
import sys
import tempfile
import unittest

_HERE = os.path.dirname(os.path.abspath(__file__))
_REPO = os.path.normpath(os.path.join(_HERE, os.pardir, os.pardir))
if _REPO not in sys.path:
    sys.path.insert(0, _REPO)

from live_capture import replay_writer as rw  # noqa: E402
from live_capture.durable_store import DurableCaptureStore  # noqa: E402
from live_capture.recorder.manifest_diagnostics import (  # noqa: E402
    patch_manifest_diagnostics,
    verify_recording_cadence,
)
from replay import schema  # noqa: E402


class TestRecorderManifestDiagnostics(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="rec_diag_")
        self.store_root = os.path.join(self.tmp, "store")
        self.out = os.path.join(self.tmp, "rec.battleplan")

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def test_patch_manifest_diagnostics_preserves_valid_container(self):
        dates = ["1936.1.1.12", "1936.1.1.15"]
        self._write_container(dates)
        h12 = schema.date_to_hour_index(dates[0])
        h15 = schema.date_to_hour_index(dates[1])

        diagnostics = {"cadence": {
            "mode": "ingame_hour_dedup_v1",
            "target_step_hours": 1,
            "status": "lagging",
            "observations": 4,
            "trusted_observations": 4,
            "untrusted_observations": 0,
            "captures": 2,
            "duplicate_observations": 1,
            "skipped_hours": 2,
            "skip_events": 1,
            "rollback_observations": 0,
            "startup_untrusted_observations": 0,
            "last_observed_hour": h15,
            "last_captured_hour": h15,
            "last_capture_reason": "skipped_hours",
            "timing": {
                "status": "not_keeping_up",
                "last_capture_duration_ms": 42000,
                "observed_game_hour_interval_ms": 30000,
                "keeping_up": False,
                "warning_count": 1,
                "last_warning": {
                    "code": "capture_slower_than_game_hour",
                    "capture_seq": 3,
                    "last_capture_duration_ms": 42000,
                    "observed_game_hour_interval_ms": 30000,
                },
            },
            "skip_ranges": [{
                "start_hour": h12 + 1,
                "end_hour": h12 + 2,
                "count": 2,
            }],
            "coverage_warnings": [{
                "code": "capture_slower_than_game_hour",
                "capture_seq": 3,
                "last_capture_duration_ms": 42000,
                "observed_game_hour_interval_ms": 30000,
            }],
        }}
        self.assertTrue(patch_manifest_diagnostics(self.out, diagnostics))
        self.assertTrue(rw._verify_container(self.out))

        man = self._read_manifest()
        self.assertEqual(man["recording_diagnostics"]["cadence"]["status"],
                         "lagging")
        self.assertEqual(man["recording_diagnostics"]["cadence"]["skipped_hours"],
                         2)
        self.assertEqual(
            man["recording_diagnostics"]["cadence"]["timing"]["status"],
            "not_keeping_up")
        self.assertEqual(
            man["recording_diagnostics"]["cadence"]["timing"][
                "last_capture_duration_ms"],
            42000)
        self.assertEqual(
            man["recording_diagnostics"]["cadence"]["coverage_warnings"][0][
                "code"],
            "capture_slower_than_game_hour")
        report = verify_recording_cadence(self.out)
        self.assertEqual(report["snapshot_count"], 2)
        self.assertEqual(report["expected_skipped_hours"], 2)

    def test_patch_manifest_diagnostics_persists_coverage_warning(self):
        self._write_one_snapshot_container()
        h0 = schema.date_to_hour_index("1936.1.1.0")

        diagnostics = {"cadence": {
            "mode": "ingame_hour_dedup_v1",
            "target_step_hours": 1,
            "status": "coverage_warning",
            "observations": 2,
            "trusted_observations": 1,
            "untrusted_observations": 1,
            "captures": 1,
            "duplicate_observations": 0,
            "skipped_hours": 0,
            "skip_events": 0,
            "rollback_observations": 0,
            "startup_untrusted_observations": 1,
            "last_observed_hour": h0,
            "last_captured_hour": h0,
            "last_capture_reason": "first_hour",
            "skip_ranges": [],
            "coverage_warnings": [{
                "code": "startup_clock_untrusted",
                "untrusted_observations": 1,
            }],
        }}
        self.assertTrue(patch_manifest_diagnostics(self.out, diagnostics))
        self.assertTrue(rw._verify_container(self.out))

        cadence = self._read_manifest()["recording_diagnostics"]["cadence"]
        self.assertEqual(cadence["status"], "coverage_warning")
        self.assertEqual(cadence["coverage_warnings"][0]["code"],
                         "startup_clock_untrusted")
        self.assertEqual(verify_recording_cadence(self.out)["snapshot_count"],
                         1)

    def test_patch_manifest_diagnostics_rejects_wrong_skipped_hours(self):
        dates = ["1936.1.1.0", "1936.1.1.3"]
        self._write_container(dates)
        h3 = schema.date_to_hour_index(dates[1])

        diagnostics = {"cadence": {
            "mode": "ingame_hour_dedup_v1",
            "target_step_hours": 1,
            "status": "ok",
            "observations": 2,
            "trusted_observations": 2,
            "untrusted_observations": 0,
            "captures": 2,
            "duplicate_observations": 0,
            "skipped_hours": 0,
            "skip_events": 0,
            "rollback_observations": 0,
            "startup_untrusted_observations": 0,
            "last_observed_hour": h3,
            "last_captured_hour": h3,
            "last_capture_reason": "sequential_hour",
            "skip_ranges": [],
            "coverage_warnings": [],
        }}

        with self.assertRaisesRegex(ValueError, "skipped_hours"):
            patch_manifest_diagnostics(self.out, diagnostics)
        self.assertNotIn("recording_diagnostics", self._read_manifest())

    def test_every_two_hours_rejects_missing_scheduled_skip_range(self):
        dates = ["1936.1.1.0", "1936.1.1.6"]
        self._write_container(dates)
        h6 = schema.date_to_hour_index(dates[1])

        diagnostics = {"cadence": {
            "mode": "ingame_hour_dedup_v1",
            "target_step_hours": 2,
            "status": "lagging",
            "observations": 2,
            "trusted_observations": 2,
            "untrusted_observations": 0,
            "captures": 2,
            "duplicate_observations": 0,
            "skipped_hours": 1,
            "skip_events": 1,
            "rollback_observations": 0,
            "startup_untrusted_observations": 0,
            "last_observed_hour": h6,
            "last_captured_hour": h6,
            "last_capture_reason": "skipped_hours",
            "skip_ranges": [{
                "start_hour": h6 - 2,
                "end_hour": h6 - 2,
                "count": 1,
                "step_hours": 2,
            }],
            "coverage_warnings": [],
        }}

        with self.assertRaisesRegex(ValueError, "skipped_hours"):
            patch_manifest_diagnostics(self.out, diagnostics)

    def test_manual_one_shot_cadence_verifies_offline(self):
        self._write_one_snapshot_container()
        h0 = schema.date_to_hour_index("1936.1.1.0")

        diagnostics = {"cadence": {
            "mode": "ingame_hour_dedup_v1",
            "target_step_hours": 1,
            "status": "ok",
            "observations": 1,
            "trusted_observations": 1,
            "untrusted_observations": 0,
            "captures": 1,
            "duplicate_observations": 0,
            "skipped_hours": 0,
            "skip_events": 0,
            "rollback_observations": 0,
            "startup_untrusted_observations": 0,
            "last_observed_hour": h0,
            "last_captured_hour": h0,
            "last_capture_reason": "first_hour",
            "recorder_mode_label": "Manual/paused snapshot",
            "manual_once": True,
            "skip_ranges": [],
            "coverage_warnings": [],
        }}

        self.assertTrue(patch_manifest_diagnostics(self.out, diagnostics))
        report = verify_recording_cadence(self.out)
        self.assertTrue(report["manual_once"])
        self.assertEqual(report["snapshot_count"], 1)

    def test_manual_one_shot_rejects_multiple_indexed_snapshots(self):
        dates = ["1936.1.1.0", "1936.1.1.1"]
        self._write_container(dates)
        h1 = schema.date_to_hour_index(dates[1])

        diagnostics = {"cadence": {
            "mode": "ingame_hour_dedup_v1",
            "target_step_hours": 1,
            "status": "ok",
            "observations": 2,
            "trusted_observations": 2,
            "untrusted_observations": 0,
            "captures": 2,
            "duplicate_observations": 0,
            "skipped_hours": 0,
            "skip_events": 0,
            "rollback_observations": 0,
            "startup_untrusted_observations": 0,
            "last_observed_hour": h1,
            "last_captured_hour": h1,
            "last_capture_reason": "sequential_hour",
            "recorder_mode_label": "Manual/paused snapshot",
            "manual_once": True,
            "skip_ranges": [],
            "coverage_warnings": [],
        }}

        with self.assertRaisesRegex(ValueError, "manual/paused"):
            patch_manifest_diagnostics(self.out, diagnostics)

    def _write_one_snapshot_container(self):
        self._write_container(["1936.1.1.0"])

    def _write_container(self, dates):
        store = DurableCaptureStore(self.store_root)
        rec = rw.ReplayRecorder(
            store_root=self.store_root,
            game_id="rec_diag",
            out_path=self.out,
            modset=None,
            build_exe_sha256="abc",
            build_pinned=True,
            schema_version=schema.SCHEMA_VERSION,
        )
        for i, date in enumerate(dates, start=1):
            snap = schema.empty_snapshot("rec_diag", date)
            snap["_capture"] = {"segment_id": 0, "campaign_id": "c0",
                                "capture_seq": i}
            path = store.write_snapshot(snap, allow_unknown_hour=False)
            rec.on_snapshot_written(path)
        rec.finalize()

    def _read_manifest(self):
        with open(self.out, "rb") as fh:
            by_name = rw._zip_read_all(fh.read())
        return json.loads(by_name["manifest.json"].decode("utf-8"))


if __name__ == "__main__":
    unittest.main()
