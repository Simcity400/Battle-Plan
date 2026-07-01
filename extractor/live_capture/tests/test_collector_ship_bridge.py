import json
import os
from pathlib import Path
from types import SimpleNamespace
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))

import live_capture.collector_ship_bridge as csb  # noqa: E402
from live_capture.collector_ship_bridge import (  # noqa: E402
    encode_ship_body_bridge,
    resolve_ship_body_bridge_preflight,
)


class FakeShipReader:
    def __init__(
        self,
        *,
        pid=111,
        build_ok=True,
        build_error=None,
        cached_valid=True,
    ):
        import oob_full_live as _oob  # noqa: E402

        self.pid = pid
        self.build_ok = build_ok
        self.build_error = build_error
        self.cached_valid = cached_valid
        self.closed = False
        self.module_base = 0x140000000
        self.exe_path = None
        self.cship_vt = self.module_base + _oob.CSHIP_VT_OFF
        self._handle_off = _oob.OFF_HANDLE
        self._ship_type = _oob.SHIP_TYPE_TAG

    def verify_build(self):
        if self.build_error is not None:
            raise self.build_error
        return self.build_ok, "a" * 64

    def fingerprint(self):
        return {"sha256": "a" * 64}

    def u64(self, addr):
        if not self.cached_valid:
            return None
        if addr in (0x8000, 0x9000):
            return self.cship_vt
        values = {
            0x8000 + self._handle_off: (3458 << 32) | self._ship_type,
            0x9000 + self._handle_off: (3459 << 32) | self._ship_type,
        }
        return values.get(addr)

    def close(self):
        self.closed = True


class FakeShipRegistry:
    build_calls = 0

    def __init__(self, reader):
        self.reader = reader

    def build_index(self):
        FakeShipRegistry.build_calls += 1
        return {3459: 0x9000, 3458: 0x8000}


class TestCollectorShipBridge(unittest.TestCase):
    def setUp(self):
        FakeShipRegistry.build_calls = 0

    def test_encode_ship_body_bridge_sorts_and_sanitizes_rows(self):
        encoded = encode_ship_body_bridge({
            3459: "0x9000",
            "3458": 0x8000,
        })

        self.assertEqual(encoded, "3458:0x8000;3459:0x9000")

    def test_encode_ship_body_bridge_rejects_bad_rows(self):
        with self.assertRaises(ValueError):
            encode_ship_body_bridge([(3458, 0x8001)])
        with self.assertRaises(ValueError):
            encode_ship_body_bridge([(3458, 0x8000), (3458, 0x9000)])
        with self.assertRaises(ValueError):
            encode_ship_body_bridge([(3458, 0x8000), (3459, 0x8000)])
        with self.assertRaises(ValueError):
            encode_ship_body_bridge([(0, 0x8000)])

    def test_preflight_resolves_encoded_bridge_with_sanitized_diagnostics(self):
        reader = FakeShipReader()

        result = resolve_ship_body_bridge_preflight(
            111,
            reader_factory=lambda: reader,
            registry_factory=FakeShipRegistry,
            allow_cold_scan=True,
        )

        self.assertTrue(result.usable)
        self.assertEqual(result.encoded, "3458:0x8000;3459:0x9000")
        self.assertEqual(result.row_count, 2)
        self.assertTrue(result.diagnostics["build_ok"])
        self.assertTrue(result.diagnostics["pid_match"])
        self.assertEqual(result.diagnostics["address_manifest_policy"], "omitted")
        self.assertTrue(result.diagnostics["bridge_supplied"])
        self.assertTrue(result.diagnostics["allow_cold_scan"])
        self.assertFalse(result.diagnostics["hot_loop_heap_scan"])
        self.assertEqual(
            result.diagnostics["discovery_mode"],
            "cache_or_explicit_cship_vtable_scan",
        )
        self.assertNotIn("0x8000", json.dumps(result.diagnostics, sort_keys=True))
        self.assertEqual(FakeShipRegistry.build_calls, 1)
        self.assertTrue(reader.closed)

    def test_preflight_fails_closed_on_pid_mismatch(self):
        reader = FakeShipReader(pid=222)

        result = resolve_ship_body_bridge_preflight(
            111,
            reader_factory=lambda: reader,
            registry_factory=FakeShipRegistry,
        )

        self.assertFalse(result.usable)
        self.assertEqual(result.status, "pid_mismatch")
        self.assertFalse(result.diagnostics["pid_match"])
        self.assertEqual(FakeShipRegistry.build_calls, 0)

    def test_preflight_fails_closed_on_build_mismatch(self):
        reader = FakeShipReader(build_ok=False)

        result = resolve_ship_body_bridge_preflight(
            111,
            reader_factory=lambda: reader,
            registry_factory=FakeShipRegistry,
        )

        self.assertFalse(result.usable)
        self.assertEqual(result.status, "build_mismatch")
        self.assertFalse(result.diagnostics["build_ok"])
        self.assertEqual(FakeShipRegistry.build_calls, 0)

    def test_preflight_error_redacts_addresses(self):
        reader = FakeShipReader(build_error=OSError("bad read at 0xDEADBEEF"))

        result = resolve_ship_body_bridge_preflight(
            111,
            reader_factory=lambda: reader,
            registry_factory=FakeShipRegistry,
        )

        self.assertFalse(result.usable)
        self.assertEqual(result.status, "build_gate_failed")
        self.assertIn("0x<redacted>", result.diagnostics["error"])
        self.assertNotIn("0xDEADBEEF", json.dumps(result.diagnostics, sort_keys=True))

    def test_cache_hit_uses_warm_ship_validation_without_locator(self):
        reader = FakeShipReader()
        identity = _identity()
        with tempfile.TemporaryDirectory() as td:
            cache_path = Path(td) / "ship_bodies.json"
            self.assertTrue(csb._write_ship_body_cache(
                cache_path,
                identity,
                [
                    {"instance_id": 3458, "body_ptr": 0x8000},
                    {"instance_id": 3459, "body_ptr": 0x9000},
                ],
            ))

            with _patched_identity(identity):
                result = resolve_ship_body_bridge_preflight(
                    111,
                    reader_factory=lambda: reader,
                    registry_factory=FakeShipRegistry,
                    cache_path=cache_path,
                    allow_cold_scan=False,
                )

        self.assertTrue(result.usable)
        self.assertEqual(result.encoded, "3458:0x8000;3459:0x9000")
        self.assertEqual(result.diagnostics["source"], "ship_body_cache")
        self.assertEqual(result.diagnostics["cache"]["status"], "hit")
        self.assertFalse(result.diagnostics["allow_cold_scan"])
        self.assertIsNone(result.diagnostics["cold_locate_s"])
        self.assertEqual(FakeShipRegistry.build_calls, 0)
        self.assertNotIn("0x8000", json.dumps(result.diagnostics, sort_keys=True))

    def test_stale_cache_without_cold_scan_fails_closed(self):
        reader = FakeShipReader(cached_valid=False)
        identity = _identity()
        with tempfile.TemporaryDirectory() as td:
            cache_path = Path(td) / "ship_bodies.json"
            self.assertTrue(csb._write_ship_body_cache(
                cache_path,
                identity,
                [{"instance_id": 3458, "body_ptr": 0x8000}],
            ))

            with _patched_identity(identity):
                result = resolve_ship_body_bridge_preflight(
                    111,
                    reader_factory=lambda: reader,
                    registry_factory=FakeShipRegistry,
                    min_rows=1,
                    cache_path=cache_path,
                    allow_cold_scan=False,
                )

        self.assertFalse(result.usable)
        self.assertEqual(result.status, "cold_scan_disabled")
        self.assertEqual(result.diagnostics["cache"]["status"], "stale")
        self.assertEqual(FakeShipRegistry.build_calls, 0)

    def test_opt_in_cold_scan_stores_body_cache(self):
        reader = FakeShipReader()
        identity = _identity()
        with tempfile.TemporaryDirectory() as td:
            cache_path = Path(td) / "ship_bodies.json"
            with _patched_identity(identity):
                result = resolve_ship_body_bridge_preflight(
                    111,
                    reader_factory=lambda: reader,
                    registry_factory=FakeShipRegistry,
                    cache_path=cache_path,
                    allow_cold_scan=True,
                )
                loaded = csb._load_ship_body_cache(
                    cache_path,
                    identity,
                    min_rows=2,
                )

        self.assertTrue(result.usable)
        self.assertEqual(result.diagnostics["cache"]["status"], "stored")
        self.assertEqual(FakeShipRegistry.build_calls, 1)
        self.assertIsNotNone(loaded)
        self.assertEqual(loaded["row_count"], 2)
        self.assertEqual(
            loaded["ships"],
            [
                {"instance_id": 3458, "body_ptr": 0x8000},
                {"instance_id": 3459, "body_ptr": 0x9000},
            ],
        )


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
        self.original = csb._process_identity.identify

    def __enter__(self):
        csb._process_identity.identify = lambda _reader: self.identity
        return self.identity

    def __exit__(self, exc_type, exc, tb):
        csb._process_identity.identify = self.original
        return False


if __name__ == "__main__":
    unittest.main()
