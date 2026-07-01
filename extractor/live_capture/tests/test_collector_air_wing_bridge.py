import json
import os
from pathlib import Path
from types import SimpleNamespace
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))

import live_capture.collector_air_wing_bridge as cawb  # noqa: E402
from live_capture.collector_air_wing_bridge import (  # noqa: E402
    encode_air_wing_body_bridge,
    resolve_air_wing_body_bridge_preflight,
)


class FakeAirWingReader:
    def __init__(
        self,
        *,
        pid=111,
        build_ok=True,
        wings=None,
        build_error=None,
        cached_valid=True,
    ):
        self.pid = pid
        self.build_ok = build_ok
        self.wings = wings if wings is not None else [
            {"owner_tagid": 1, "wing_id": 3, "body_addr": 0x8000},
            {"owner_tagid": 2, "wing_id": 4, "body_addr": 0x9000},
        ]
        self.build_error = build_error
        self.cached_valid = cached_valid
        self.enumerate_calls = 0
        self.closed = False
        self.module_base = 0x140000000
        self.exe_path = None
        self.wing_vt = self.module_base + 0x29540C0

    def verify_build(self):
        if self.build_error is not None:
            raise self.build_error
        return self.build_ok, "a" * 64

    def enumerate_air_wings(self):
        self.enumerate_calls += 1
        return self.wings

    def u64(self, addr):
        if not self.cached_valid:
            return None
        if addr in (0x8000, 0x9000):
            return self.wing_vt
        return None

    def u32(self, addr):
        if not self.cached_valid:
            return None
        values = {
            0x8000 + 0x08: 69,
            0x8000 + 0x0C: 3,
            0x9000 + 0x08: 69,
            0x9000 + 0x0C: 4,
        }
        return values.get(addr)

    def wing_owner_tagid(self, body_ptr):
        if not self.cached_valid:
            return None
        return {0x8000: 1, 0x9000: 2}.get(body_ptr)

    def close(self):
        self.closed = True


class IndexInt:
    def __init__(self, value):
        self.value = value

    def __index__(self):
        return self.value


class TestCollectorAirWingBridge(unittest.TestCase):
    def test_encode_air_wing_body_bridge_sorts_and_sanitizes_rows(self):
        encoded = encode_air_wing_body_bridge([
            {"owner_tagid": 2, "wing_id": "4", "body_addr": IndexInt(0x9000)},
            {"owner_tag_db_id": "1", "wing_id": 3, "body_ptr": 0x8000},
        ])

        self.assertEqual(encoded, "1:3:0x8000;2:4:0x9000")

    def test_encode_air_wing_body_bridge_rejects_bad_rows(self):
        with self.assertRaises(ValueError):
            encode_air_wing_body_bridge([(1, 3, 0x8001)])
        with self.assertRaises(ValueError):
            encode_air_wing_body_bridge([(1, 3, 0x8000), (1, 3, 0x9000)])
        with self.assertRaises(ValueError):
            encode_air_wing_body_bridge([(1, 3, 0x8000), (2, 4, 0x8000)])
        with self.assertRaises(ValueError):
            encode_air_wing_body_bridge([(0, 3, 0x8000)])

    def test_preflight_resolves_encoded_bridge_with_sanitized_diagnostics(self):
        reader = FakeAirWingReader()

        with _patched_arena_discovery({"status": "updated", "range_bytes": 0x4000}):
            result = resolve_air_wing_body_bridge_preflight(
                111,
                reader_factory=lambda: reader,
                allow_cold_scan=True,
            )

        self.assertTrue(result.usable)
        self.assertEqual(result.encoded, "1:3:0x8000;2:4:0x9000")
        self.assertEqual(result.row_count, 2)
        self.assertTrue(result.diagnostics["build_ok"])
        self.assertTrue(result.diagnostics["pid_match"])
        self.assertEqual(result.diagnostics["address_manifest_policy"], "omitted")
        self.assertTrue(result.diagnostics["bridge_supplied"])
        self.assertTrue(result.diagnostics["allow_cold_scan"])
        self.assertEqual(result.diagnostics["arena_discovery"]["status"], "updated")
        self.assertFalse(result.diagnostics["hot_loop_heap_scan"])
        self.assertNotIn("0x8000", json.dumps(result.diagnostics, sort_keys=True))
        self.assertEqual(reader.enumerate_calls, 1)
        self.assertTrue(reader.closed)

    def test_preflight_fails_closed_on_pid_mismatch(self):
        reader = FakeAirWingReader(pid=222)

        result = resolve_air_wing_body_bridge_preflight(
            111,
            reader_factory=lambda: reader,
        )

        self.assertFalse(result.usable)
        self.assertEqual(result.status, "pid_mismatch")
        self.assertFalse(result.diagnostics["pid_match"])
        self.assertEqual(reader.enumerate_calls, 0)

    def test_preflight_fails_closed_on_build_mismatch(self):
        reader = FakeAirWingReader(build_ok=False)

        result = resolve_air_wing_body_bridge_preflight(
            111,
            reader_factory=lambda: reader,
        )

        self.assertFalse(result.usable)
        self.assertEqual(result.status, "build_mismatch")
        self.assertFalse(result.diagnostics["build_ok"])
        self.assertEqual(reader.enumerate_calls, 0)

    def test_preflight_error_redacts_addresses(self):
        reader = FakeAirWingReader(build_error=OSError("bad read at 0xDEADBEEF"))

        result = resolve_air_wing_body_bridge_preflight(
            111,
            reader_factory=lambda: reader,
        )

        self.assertFalse(result.usable)
        self.assertEqual(result.status, "build_gate_failed")
        self.assertIn("0x<redacted>", result.diagnostics["error"])
        self.assertNotIn("0xDEADBEEF", json.dumps(result.diagnostics, sort_keys=True))

    def test_cache_hit_uses_warm_air_wing_validation_without_locator(self):
        reader = FakeAirWingReader()
        identity = _identity()
        with tempfile.TemporaryDirectory() as td:
            cache_path = Path(td) / "air_wing_bodies.json"
            self.assertTrue(cawb._write_air_wing_body_cache(
                cache_path,
                identity,
                [
                    {"owner_tag_db_id": 1, "wing_id": 3, "body_ptr": 0x8000},
                    {"owner_tag_db_id": 2, "wing_id": 4, "body_ptr": 0x9000},
                ],
            ))

            with _patched_identity(identity):
                result = resolve_air_wing_body_bridge_preflight(
                    111,
                    reader_factory=lambda: reader,
                    cache_path=cache_path,
                    allow_cold_scan=False,
                )

        self.assertTrue(result.usable)
        self.assertEqual(result.encoded, "1:3:0x8000;2:4:0x9000")
        self.assertEqual(result.diagnostics["source"], "air_wing_body_cache")
        self.assertEqual(result.diagnostics["cache"]["status"], "hit")
        self.assertFalse(result.diagnostics["allow_cold_scan"])
        self.assertIsNone(result.diagnostics["cold_locate_s"])
        self.assertEqual(reader.enumerate_calls, 0)
        self.assertNotIn("0x8000", json.dumps(result.diagnostics, sort_keys=True))

    def test_stale_cache_without_cold_scan_fails_closed(self):
        reader = FakeAirWingReader(cached_valid=False)
        identity = _identity()
        with tempfile.TemporaryDirectory() as td:
            cache_path = Path(td) / "air_wing_bodies.json"
            self.assertTrue(cawb._write_air_wing_body_cache(
                cache_path,
                identity,
                [{"owner_tag_db_id": 1, "wing_id": 3, "body_ptr": 0x8000}],
            ))

            with _patched_identity(identity):
                result = resolve_air_wing_body_bridge_preflight(
                    111,
                    reader_factory=lambda: reader,
                    min_rows=1,
                    cache_path=cache_path,
                    allow_cold_scan=False,
                )

        self.assertFalse(result.usable)
        self.assertEqual(result.status, "cold_scan_disabled")
        self.assertEqual(result.diagnostics["cache"]["status"], "stale")
        self.assertEqual(reader.enumerate_calls, 0)


def _identity(**overrides):
    values = {
        "pid": 111,
        "create_time_100ns": 222,
        "exe_sha256": "a" * 64,
        "module_base": 0x140000000,
    }
    values.update(overrides)
    return SimpleNamespace(**values)


class _patched_identity:
    def __init__(self, identity):
        self.identity = identity
        self.original = cawb._process_identity.identify

    def __enter__(self):
        cawb._process_identity.identify = lambda _reader: self.identity
        return self.identity

    def __exit__(self, exc_type, exc, tb):
        cawb._process_identity.identify = self.original
        return False


class _patched_arena_discovery:
    def __init__(self, value):
        self.value = value
        self.original = cawb._discover_live_arena_for_reader

    def __enter__(self):
        cawb._discover_live_arena_for_reader = lambda _reader: dict(self.value)

    def __exit__(self, exc_type, exc, tb):
        cawb._discover_live_arena_for_reader = self.original
        return False


if __name__ == "__main__":
    unittest.main()
