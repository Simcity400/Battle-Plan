import json
import os
from pathlib import Path
from types import SimpleNamespace
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))

import live_capture.collector_army_order_bridge as cab  # noqa: E402
from live_capture.collector_army_order_bridge import (  # noqa: E402
    encode_army_body_bridge,
    resolve_army_body_bridge_preflight,
)


class FakeArmyReader:
    def __init__(
        self,
        *,
        pid=111,
        build_ok=True,
        armies=None,
        build_error=None,
        validate_cached=True,
        invalid_cached_ids=None,
    ):
        self.pid = pid
        self.build_ok = build_ok
        self.armies = armies if armies is not None else {18: 0x9000, 17: 0x8000}
        self.build_error = build_error
        self.validate_cached = validate_cached
        self.invalid_cached_ids = set(invalid_cached_ids or [])
        self.enumerate_calls = 0
        self.validate_calls = 0
        self.closed = False

    def verify_build(self):
        if self.build_error is not None:
            raise self.build_error
        return self.build_ok, "a" * 64

    def enumerate_armies(self):
        self.enumerate_calls += 1
        return self.armies

    def validate_armies(self, armies):
        self.validate_calls += 1
        if not armies or not self.validate_cached:
            return False
        return not any(int(army_id) in self.invalid_cached_ids
                       for army_id in armies)

    def close(self):
        self.closed = True


class TestCollectorArmyOrderBridge(unittest.TestCase):
    def test_encode_army_body_bridge_sorts_and_sanitizes_rows(self):
        encoded = encode_army_body_bridge({
            18: "0x9000",
            "17": 0x8000,
        })

        self.assertEqual(encoded, "17:0x8000;18:0x9000")

    def test_encode_army_body_bridge_rejects_bad_rows(self):
        with self.assertRaises(ValueError):
            encode_army_body_bridge([(17, 0x8001)])
        with self.assertRaises(ValueError):
            encode_army_body_bridge([(17, 0x8000), (17, 0x9000)])
        with self.assertRaises(ValueError):
            encode_army_body_bridge([(17, 0x8000), (18, 0x8000)])

    def test_preflight_resolves_encoded_bridge_with_sanitized_diagnostics(self):
        reader = FakeArmyReader()

        result = resolve_army_body_bridge_preflight(
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
        self.assertEqual(reader.enumerate_calls, 1)
        self.assertTrue(reader.closed)

    def test_preflight_fails_closed_on_pid_mismatch(self):
        reader = FakeArmyReader(pid=222)

        result = resolve_army_body_bridge_preflight(
            111,
            reader_factory=lambda: reader,
        )

        self.assertFalse(result.usable)
        self.assertEqual(result.status, "pid_mismatch")
        self.assertFalse(result.diagnostics["pid_match"])
        self.assertEqual(reader.enumerate_calls, 0)

    def test_preflight_fails_closed_on_build_mismatch(self):
        reader = FakeArmyReader(build_ok=False)

        result = resolve_army_body_bridge_preflight(
            111,
            reader_factory=lambda: reader,
        )

        self.assertFalse(result.usable)
        self.assertEqual(result.status, "build_mismatch")
        self.assertFalse(result.diagnostics["build_ok"])
        self.assertEqual(reader.enumerate_calls, 0)

    def test_preflight_error_redacts_addresses(self):
        reader = FakeArmyReader(build_error=OSError("bad read at 0xDEADBEEF"))

        result = resolve_army_body_bridge_preflight(
            111,
            reader_factory=lambda: reader,
        )

        self.assertFalse(result.usable)
        self.assertEqual(result.status, "build_gate_failed")
        self.assertIn("0x<redacted>", result.diagnostics["error"])
        self.assertNotIn("0xDEADBEEF", json.dumps(result.diagnostics, sort_keys=True))

    def test_cache_hit_uses_warm_army_validation_without_locator(self):
        reader = FakeArmyReader()
        identity = _identity()
        with tempfile.TemporaryDirectory() as td:
            cache_path = Path(td) / "army_bodies.json"
            self.assertTrue(cab._write_army_body_cache(
                cache_path,
                identity,
                [
                    {"army_id": 17, "body_ptr": 0x8000},
                    {"army_id": 18, "body_ptr": 0x9000},
                ],
            ))

            with _patched_identity(identity):
                result = resolve_army_body_bridge_preflight(
                    111,
                    reader_factory=lambda: reader,
                    cache_path=cache_path,
                    allow_cold_scan=False,
                )

        self.assertTrue(result.usable)
        self.assertEqual(result.encoded, "17:0x8000;18:0x9000")
        self.assertEqual(result.diagnostics["source"], "army_body_cache")
        self.assertEqual(result.diagnostics["cache"]["status"], "hit")
        self.assertFalse(result.diagnostics["allow_cold_scan"])
        self.assertIsNone(result.diagnostics["cold_locate_s"])
        self.assertEqual(reader.validate_calls, 1)
        self.assertEqual(reader.enumerate_calls, 0)
        self.assertNotIn("0x8000", json.dumps(result.diagnostics, sort_keys=True))

    def test_cache_hit_salvages_valid_army_rows_when_some_cached_rows_stale(self):
        reader = FakeArmyReader(invalid_cached_ids={18})
        identity = _identity()
        with tempfile.TemporaryDirectory() as td:
            cache_path = Path(td) / "army_bodies.json"
            self.assertTrue(cab._write_army_body_cache(
                cache_path,
                identity,
                [
                    {"army_id": 17, "body_ptr": 0x8000},
                    {"army_id": 18, "body_ptr": 0x9000},
                    {"army_id": 19, "body_ptr": 0xA000},
                ],
            ))

            with _patched_identity(identity):
                result = resolve_army_body_bridge_preflight(
                    111,
                    reader_factory=lambda: reader,
                    min_rows=1,
                    cache_path=cache_path,
                    allow_cold_scan=False,
                )

        self.assertTrue(result.usable)
        self.assertEqual(result.encoded, "17:0x8000;19:0xA000")
        self.assertEqual(result.row_count, 2)
        self.assertEqual(result.diagnostics["source"], "army_body_cache")
        self.assertEqual(result.diagnostics["cache"]["status"], "hit")
        self.assertEqual(result.diagnostics["cache"]["cached_row_count"], 3)
        self.assertEqual(result.diagnostics["cache"]["valid_row_count"], 2)
        self.assertEqual(result.diagnostics["cache"]["stale_row_count"], 1)
        self.assertEqual(reader.validate_calls, 4)
        self.assertEqual(reader.enumerate_calls, 0)
        self.assertNotIn("0x8000", json.dumps(result.diagnostics, sort_keys=True))

    def test_stale_cache_without_cold_scan_fails_closed(self):
        reader = FakeArmyReader(validate_cached=False)
        identity = _identity()
        with tempfile.TemporaryDirectory() as td:
            cache_path = Path(td) / "army_bodies.json"
            self.assertTrue(cab._write_army_body_cache(
                cache_path,
                identity,
                [{"army_id": 17, "body_ptr": 0x8000}],
            ))

            with _patched_identity(identity):
                result = resolve_army_body_bridge_preflight(
                    111,
                    reader_factory=lambda: reader,
                    min_rows=1,
                    cache_path=cache_path,
                    allow_cold_scan=False,
                )

        self.assertFalse(result.usable)
        self.assertEqual(result.status, "cold_scan_disabled")
        self.assertEqual(result.diagnostics["cache"]["status"], "stale")
        self.assertEqual(result.diagnostics["cache"]["cached_row_count"], 1)
        self.assertEqual(result.diagnostics["cache"]["valid_row_count"], 0)
        self.assertEqual(result.diagnostics["cache"]["stale_row_count"], 1)
        self.assertEqual(reader.validate_calls, 2)
        self.assertEqual(reader.enumerate_calls, 0)


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
        self.original = cab._process_identity.identify

    def __enter__(self):
        cab._process_identity.identify = lambda _reader: self.identity
        return self.identity

    def __exit__(self, exc_type, exc, tb):
        cab._process_identity.identify = self.original
        return False


if __name__ == "__main__":
    unittest.main()
