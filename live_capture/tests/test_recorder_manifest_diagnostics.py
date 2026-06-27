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
        self._write_one_snapshot_container()

        diagnostics = {"cadence": {
            "mode": "ingame_hour_dedup_v1",
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
            "last_observed_hour": 17,
            "last_captured_hour": 17,
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
            "skip_ranges": [{"start_hour": 15, "end_hour": 16, "count": 2}],
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

    def test_patch_manifest_diagnostics_persists_coverage_warning(self):
        self._write_one_snapshot_container()

        diagnostics = {"cadence": {
            "mode": "ingame_hour_dedup_v1",
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
            "last_observed_hour": 0,
            "last_captured_hour": 0,
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

    def _write_one_snapshot_container(self):
        store = DurableCaptureStore(self.store_root)
        snap = schema.empty_snapshot("rec_diag", "1936.1.1.0")
        snap["_capture"] = {"segment_id": 0, "campaign_id": "c0",
                            "capture_seq": 1}
        path = store.write_snapshot(snap, allow_unknown_hour=False)
        rec = rw.ReplayRecorder(
            store_root=self.store_root,
            game_id="rec_diag",
            out_path=self.out,
            modset=None,
            build_exe_sha256="abc",
            build_pinned=True,
            schema_version=schema.SCHEMA_VERSION,
        )
        rec.on_snapshot_written(path)
        rec.finalize()

    def _read_manifest(self):
        with open(self.out, "rb") as fh:
            by_name = rw._zip_read_all(fh.read())
        return json.loads(by_name["manifest.json"].decode("utf-8"))


if __name__ == "__main__":
    unittest.main()
