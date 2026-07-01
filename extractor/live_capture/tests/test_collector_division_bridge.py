import json
import os
from pathlib import Path
from types import SimpleNamespace
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))

import live_capture.collector_division_bridge as cdb  # noqa: E402
from live_capture.collector_division_bridge import (  # noqa: E402
    encode_division_body_bridge,
    resolve_division_body_bridge_preflight,
)


class FakeDivisionReader:
    def __init__(
        self,
        *,
        pid=111,
        build_ok=True,
        bodies=None,
        build_error=None,
        valid_cached_ids=None,
    ):
        self.pid = pid
        self.build_ok = build_ok
        self.bodies = bodies if bodies is not None else {18: 0x9000, 17: 0x8000}
        self.build_error = build_error
        self.valid_cached_ids = valid_cached_ids
        self.locate_calls = 0
        self.read_cached_calls = 0
        self.closed = False

    def verify_build(self):
        if self.build_error is not None:
            raise self.build_error
        return self.build_ok, "a" * 64

    def locate_division_bodies(self):
        self.locate_calls += 1
        return self.bodies

    def read_divisions_from_bodies(self, bodies):
        self.read_cached_calls += 1
        valid_ids = (
            set(bodies)
            if self.valid_cached_ids is None
            else set(self.valid_cached_ids)
        )
        return [
            {"id": div_id}
            for div_id in sorted(bodies)
            if div_id in valid_ids
        ]

    def close(self):
        self.closed = True


class TestCollectorDivisionBridge(unittest.TestCase):
    def test_encode_division_body_bridge_sorts_and_sanitizes_rows(self):
        encoded = encode_division_body_bridge({
            18: "0x9000",
            "17": 0x8000,
        })

        self.assertEqual(encoded, "17:0x8000;18:0x9000")

    def test_encode_division_body_bridge_rejects_bad_rows(self):
        with self.assertRaises(ValueError):
            encode_division_body_bridge([(17, 0x8001)])
        with self.assertRaises(ValueError):
            encode_division_body_bridge([(17, 0x8000), (17, 0x9000)])
        with self.assertRaises(ValueError):
            encode_division_body_bridge([(17, 0x8000), (18, 0x8000)])

    def test_preflight_resolves_encoded_bridge_with_sanitized_diagnostics(self):
        reader = FakeDivisionReader()

        result = resolve_division_body_bridge_preflight(
            111,
            reader_factory=lambda: reader,
            allow_cold_scan=True,
        )

        self.assertTrue(result.usable)
        self.assertEqual(result.encoded, "17:0x8000;18:0x9000")
        self.assertEqual(result.row_count, 2)
        self.assertTrue(result.diagnostics["build_ok"])
        self.assertTrue(result.diagnostics["pid_match"])
        self.assertEqual(result.diagnostics["address_manifest_policy"], "omitted")
        self.assertTrue(result.diagnostics["bridge_supplied"])
        self.assertTrue(result.diagnostics["allow_cold_scan"])
        self.assertNotIn("0x8000", json.dumps(result.diagnostics, sort_keys=True))
        self.assertEqual(reader.locate_calls, 1)
        self.assertTrue(reader.closed)

    def test_preflight_fails_closed_on_pid_mismatch(self):
        reader = FakeDivisionReader(pid=222)

        result = resolve_division_body_bridge_preflight(
            111,
            reader_factory=lambda: reader,
        )

        self.assertFalse(result.usable)
        self.assertEqual(result.status, "pid_mismatch")
        self.assertEqual(result.row_count, 0)
        self.assertFalse(result.diagnostics["pid_match"])
        self.assertEqual(reader.locate_calls, 0)

    def test_preflight_fails_closed_on_build_mismatch(self):
        reader = FakeDivisionReader(build_ok=False)

        result = resolve_division_body_bridge_preflight(
            111,
            reader_factory=lambda: reader,
        )

        self.assertFalse(result.usable)
        self.assertEqual(result.status, "build_mismatch")
        self.assertFalse(result.diagnostics["build_ok"])
        self.assertEqual(reader.locate_calls, 0)

    def test_preflight_error_redacts_addresses(self):
        reader = FakeDivisionReader(build_error=OSError("bad read at 0xDEADBEEF"))

        result = resolve_division_body_bridge_preflight(
            111,
            reader_factory=lambda: reader,
        )

        self.assertFalse(result.usable)
        self.assertEqual(result.status, "build_gate_failed")
        self.assertIn("0x<redacted>", result.diagnostics["error"])
        self.assertNotIn("0xDEADBEEF", json.dumps(result.diagnostics, sort_keys=True))

    def test_cache_hit_uses_warm_body_validation_without_locator(self):
        reader = FakeDivisionReader()
        identity = _identity()
        with tempfile.TemporaryDirectory() as td:
            cache_path = Path(td) / "division_bodies.json"
            self.assertTrue(cdb._write_division_body_cache(
                cache_path,
                identity,
                [
                    {"div_id": 17, "body_ptr": 0x8000},
                    {"div_id": 18, "body_ptr": 0x9000},
                ],
            ))

            with _patched_identity(identity):
                result = resolve_division_body_bridge_preflight(
                    111,
                    reader_factory=lambda: reader,
                    cache_path=cache_path,
                    allow_cold_scan=False,
                )

        self.assertTrue(result.usable)
        self.assertEqual(result.encoded, "17:0x8000;18:0x9000")
        self.assertEqual(result.diagnostics["source"], "division_body_cache")
        self.assertEqual(result.diagnostics["cache"]["status"], "hit")
        self.assertFalse(result.diagnostics["allow_cold_scan"])
        self.assertIsNone(result.diagnostics["cold_locate_s"])
        self.assertEqual(reader.read_cached_calls, 1)
        self.assertEqual(reader.locate_calls, 0)
        self.assertNotIn("0x8000", json.dumps(result.diagnostics, sort_keys=True))

    def test_cache_miss_without_cold_scan_fails_closed(self):
        reader = FakeDivisionReader()
        with tempfile.TemporaryDirectory() as td:
            with _patched_identity(_identity()):
                result = resolve_division_body_bridge_preflight(
                    111,
                    reader_factory=lambda: reader,
                    cache_path=Path(td) / "division_bodies.json",
                    allow_cold_scan=False,
                )

        self.assertFalse(result.usable)
        self.assertEqual(result.status, "cold_scan_disabled")
        self.assertEqual(result.row_count, 0)
        self.assertEqual(result.diagnostics["cache"]["status"], "miss")
        self.assertEqual(result.diagnostics["source"], "division_body_cache")
        self.assertEqual(reader.locate_calls, 0)

    def test_stale_cache_without_cold_scan_fails_closed(self):
        reader = FakeDivisionReader(valid_cached_ids={17})
        identity = _identity()
        with tempfile.TemporaryDirectory() as td:
            cache_path = Path(td) / "division_bodies.json"
            self.assertTrue(cdb._write_division_body_cache(
                cache_path,
                identity,
                [
                    {"div_id": 17, "body_ptr": 0x8000},
                    {"div_id": 18, "body_ptr": 0x9000},
                ],
            ))

            with _patched_identity(identity):
                result = resolve_division_body_bridge_preflight(
                    111,
                    reader_factory=lambda: reader,
                    min_rows=2,
                    cache_path=cache_path,
                    allow_cold_scan=False,
                )

        self.assertFalse(result.usable)
        self.assertEqual(result.status, "cold_scan_disabled")
        self.assertEqual(result.diagnostics["cache"]["status"], "stale")
        self.assertEqual(reader.read_cached_calls, 1)
        self.assertEqual(reader.locate_calls, 0)

    def test_opt_in_cold_scan_stores_body_cache(self):
        reader = FakeDivisionReader()
        identity = _identity()
        with tempfile.TemporaryDirectory() as td:
            cache_path = Path(td) / "division_bodies.json"
            with _patched_identity(identity):
                result = resolve_division_body_bridge_preflight(
                    111,
                    reader_factory=lambda: reader,
                    cache_path=cache_path,
                    allow_cold_scan=True,
                )
                loaded = cdb._load_division_body_cache(
                    cache_path,
                    identity,
                    min_rows=2,
                )

        self.assertTrue(result.usable)
        self.assertEqual(result.diagnostics["cache"]["status"], "stored")
        self.assertEqual(reader.locate_calls, 1)
        self.assertIsNotNone(loaded)
        self.assertEqual(loaded["row_count"], 2)
        self.assertEqual(loaded["bodies"], {"17": 0x8000, "18": 0x9000})

def _identity(**overrides):
    values = {
        "pid": 111,
        "create_time_100ns": 222,
        "exe_sha256": "a" * 64,
        "module_base": 0x100000,
    }
    values.update(overrides)
    return SimpleNamespace(**values)


class _patched_identity:
    def __init__(self, identity):
        self.identity = identity
        self.original = cdb._process_identity.identify

    def __enter__(self):
        cdb._process_identity.identify = lambda _reader: self.identity
        return self.identity

    def __exit__(self, exc_type, exc, tb):
        cdb._process_identity.identify = self.original
        return False


if __name__ == "__main__":
    unittest.main()
