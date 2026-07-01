import json
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))

from live_capture.collector_naval_combat_bridge import (  # noqa: E402
    encode_naval_combat_body_bridge,
    resolve_naval_combat_body_bridge_preflight,
)


class FakeCombatReader:
    def __init__(
        self,
        *,
        pid=111,
        build_ok=True,
        combats=None,
        build_error=None,
        enumerate_error=None,
        reject_stats=None,
    ):
        self.pid = pid
        self.build_ok = build_ok
        self.combats = combats if combats is not None else [
            {"combat_id": 42, "body_addr": 0x9000},
            {"combat_id": 41, "body_addr": 0x8000},
        ]
        self.build_error = build_error
        self.enumerate_error = enumerate_error
        self.reject_stats = reject_stats or {}
        self.enumerate_calls = 0
        self.closed = False

    def verify_build(self):
        if self.build_error is not None:
            raise self.build_error
        return self.build_ok, "a" * 64

    def enumerate_naval_combats(self, *, with_defenders=True):
        self.enumerate_calls += 1
        if not with_defenders:
            raise AssertionError("with_defenders must stay enabled")
        if self.enumerate_error is not None:
            raise self.enumerate_error
        return self.combats

    def close(self):
        self.closed = True


class TestCollectorNavalCombatBridge(unittest.TestCase):
    def test_encode_naval_combat_body_bridge_sorts_and_sanitizes_rows(self):
        encoded = encode_naval_combat_body_bridge([
            {"combat_id": 42, "body_addr": "0x9000"},
            {"combat_id": "41", "body_ptr": 0x8000},
        ])

        self.assertEqual(encoded, "41:0x8000;42:0x9000")

    def test_encode_naval_combat_body_bridge_rejects_bad_rows(self):
        with self.assertRaises(ValueError):
            encode_naval_combat_body_bridge([(41, 0x8001)])
        with self.assertRaises(ValueError):
            encode_naval_combat_body_bridge([(41, 0x8000), (41, 0x9000)])
        with self.assertRaises(ValueError):
            encode_naval_combat_body_bridge([(41, 0x8000), (42, 0x8000)])

    def test_preflight_resolves_encoded_bridge_with_sanitized_diagnostics(self):
        reader = FakeCombatReader(reject_stats={"naval_bad_identity": 3})

        result = resolve_naval_combat_body_bridge_preflight(
            111,
            reader_factory=lambda: reader,
        )

        self.assertTrue(result.usable)
        self.assertEqual(result.encoded, "41:0x8000;42:0x9000")
        self.assertEqual(result.row_count, 2)
        self.assertTrue(result.diagnostics["build_ok"])
        self.assertTrue(result.diagnostics["pid_match"])
        self.assertEqual(result.diagnostics["address_manifest_policy"], "omitted")
        self.assertEqual(
            result.diagnostics["discovery_mode"],
            "preflight_vtable_enumeration",
        )
        self.assertFalse(result.diagnostics["hot_loop_heap_scan"])
        self.assertEqual(result.diagnostics["reject_stats"], {"naval_bad_identity": 3})
        self.assertNotIn("0x8000", json.dumps(result.diagnostics, sort_keys=True))
        self.assertEqual(reader.enumerate_calls, 1)
        self.assertTrue(reader.closed)

    def test_preflight_reports_no_active_combats_without_error(self):
        reader = FakeCombatReader(combats=[])

        result = resolve_naval_combat_body_bridge_preflight(
            111,
            reader_factory=lambda: reader,
        )

        self.assertFalse(result.usable)
        self.assertEqual(result.status, "no_active_combats")
        self.assertEqual(result.row_count, 0)
        self.assertTrue(result.diagnostics["build_ok"])
        self.assertEqual(reader.enumerate_calls, 1)

    def test_preflight_fails_closed_on_pid_mismatch(self):
        reader = FakeCombatReader(pid=222)

        result = resolve_naval_combat_body_bridge_preflight(
            111,
            reader_factory=lambda: reader,
        )

        self.assertFalse(result.usable)
        self.assertEqual(result.status, "pid_mismatch")
        self.assertFalse(result.diagnostics["pid_match"])
        self.assertEqual(reader.enumerate_calls, 0)

    def test_preflight_fails_closed_on_build_mismatch(self):
        reader = FakeCombatReader(build_ok=False)

        result = resolve_naval_combat_body_bridge_preflight(
            111,
            reader_factory=lambda: reader,
        )

        self.assertFalse(result.usable)
        self.assertEqual(result.status, "build_mismatch")
        self.assertFalse(result.diagnostics["build_ok"])
        self.assertEqual(reader.enumerate_calls, 0)

    def test_preflight_error_redacts_addresses(self):
        reader = FakeCombatReader(
            enumerate_error=OSError("bad read at 0xDEADBEEF"),
        )

        result = resolve_naval_combat_body_bridge_preflight(
            111,
            reader_factory=lambda: reader,
        )

        self.assertFalse(result.usable)
        self.assertEqual(result.status, "resolve_failed")
        self.assertIn("0x<redacted>", result.diagnostics["error"])
        self.assertNotIn("0xDEADBEEF", json.dumps(result.diagnostics, sort_keys=True))


if __name__ == "__main__":
    unittest.main()
