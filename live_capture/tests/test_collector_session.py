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

            frame["session_id"] = "other_session"
            _write_json(run_dir / "collector_started_0000.json", frame)
            manifest["session_id"] = "session_x"
            with self.assertRaisesRegex(RuntimeError, "session_id"):
                cs._frames_from_probe_manifest(run_dir, manifest)

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


def _frame(date, raw, *, sequence, pid):
    return {
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


if __name__ == "__main__":
    unittest.main()
