import json
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))

from live_capture.collector_division_bridge import (  # noqa: E402
    encode_division_body_bridge,
    resolve_division_body_bridge_preflight,
)


class FakeDivisionReader:
    def __init__(self, *, pid=111, build_ok=True, bodies=None, build_error=None):
        self.pid = pid
        self.build_ok = build_ok
        self.bodies = bodies if bodies is not None else {18: 0x9000, 17: 0x8000}
        self.build_error = build_error
        self.locate_called = False
        self.closed = False

    def verify_build(self):
        if self.build_error is not None:
            raise self.build_error
        return self.build_ok, "a" * 64

    def locate_division_bodies(self):
        self.locate_called = True
        return self.bodies

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
        )

        self.assertTrue(result.usable)
        self.assertEqual(result.encoded, "17:0x8000;18:0x9000")
        self.assertEqual(result.row_count, 2)
        self.assertTrue(result.diagnostics["build_ok"])
        self.assertTrue(result.diagnostics["pid_match"])
        self.assertEqual(result.diagnostics["address_manifest_policy"], "omitted")
        self.assertTrue(result.diagnostics["bridge_supplied"])
        self.assertNotIn("0x8000", json.dumps(result.diagnostics, sort_keys=True))
        self.assertTrue(reader.locate_called)
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
        self.assertFalse(reader.locate_called)

    def test_preflight_fails_closed_on_build_mismatch(self):
        reader = FakeDivisionReader(build_ok=False)

        result = resolve_division_body_bridge_preflight(
            111,
            reader_factory=lambda: reader,
        )

        self.assertFalse(result.usable)
        self.assertEqual(result.status, "build_mismatch")
        self.assertFalse(result.diagnostics["build_ok"])
        self.assertFalse(reader.locate_called)

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


if __name__ == "__main__":
    unittest.main()
