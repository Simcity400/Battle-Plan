import json
import os
from pathlib import Path
from types import SimpleNamespace
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))

import live_capture.collector_oob_command_bridge as ocb  # noqa: E402
from live_capture.collector_oob_command_bridge import (  # noqa: E402
    encode_oob_command_bridge,
    resolve_oob_command_bridge_preflight,
)


class FakeOobReader:
    def __init__(self, *, pid=111, build_ok=True, cached_valid=True, build_error=None):
        self.pid = pid
        self.build_ok = build_ok
        self.cached_valid = cached_valid
        self.build_error = build_error
        self.closed = False
        self.module_base = 0x140000000
        self.exe_path = None

    def verify_build(self):
        if self.build_error is not None:
            raise self.build_error
        return self.build_ok, "a" * 64

    def fingerprint(self):
        return {"sha256": "a" * 64}

    def enumerate_command_objs(self):
        return {
            "_arena": (1, 2),
            "army": [(0x8000, 17)],
            "ag": [(0x8100, 18)],
            "fleet": [(0x9000, 61500)],
            "tf": [(0xA000, 61700)],
            "theatre": [],
            "div": [],
            "ship": [(0xB000, 5001)],
        }

    def validate_row(self, kind, object_id, body_ptr):
        if not self.cached_valid:
            return False
        return (kind, object_id, body_ptr) in {
            ("fleet", 61500, 0x9000),
            ("tf", 61700, 0xA000),
        }

    def close(self):
        self.closed = True


class TestCollectorOobCommandBridge(unittest.TestCase):
    def test_encode_oob_command_bridge_sorts_and_sanitizes_rows(self):
        encoded = encode_oob_command_bridge([
            {"kind": "tf", "id": 61700, "body_ptr": "0xA000"},
            {"kind": "fleet", "id": "61500", "body_ptr": 0x9000},
            {"kind": "army", "id": 17, "body_ptr": 0x8000},
        ])

        self.assertEqual(
            encoded,
            "army:17:0x8000;fleet:61500:0x9000;tf:61700:0xA000",
        )

    def test_encode_oob_command_bridge_rejects_bad_rows(self):
        with self.assertRaises(ValueError):
            encode_oob_command_bridge([("fleet", 61500, 0x9001)])
        with self.assertRaises(ValueError):
            encode_oob_command_bridge([
                ("fleet", 61500, 0x9000),
                ("fleet", 61500, 0xA000),
            ])
        with self.assertRaises(ValueError):
            encode_oob_command_bridge([("unknown", 1, 0x9000)])
        with self.assertRaises(ValueError):
            encode_oob_command_bridge([
                ("fleet", 61500, 0x9000),
                ("tf", 61700, 0x9000),
            ])

    def test_preflight_resolves_cold_scan_without_address_diagnostics(self):
        reader = FakeOobReader()

        result = resolve_oob_command_bridge_preflight(
            111,
            reader_factory=lambda: reader,
            allow_cold_scan=True,
        )

        self.assertTrue(result.usable)
        self.assertIn("fleet:61500:0x9000", result.encoded)
        self.assertIn("tf:61700:0xA000", result.encoded)
        self.assertEqual(result.diagnostics["status"], "resolved")
        self.assertEqual(result.diagnostics["counts"]["fleet"], 1)
        self.assertEqual(result.diagnostics["counts"]["tf"], 1)
        self.assertFalse(result.diagnostics["hot_loop_heap_scan"])
        self.assertNotIn("0x9000", json.dumps(result.diagnostics, sort_keys=True))
        self.assertTrue(reader.closed)

    def test_cache_hit_validates_rows_without_cold_scan(self):
        reader = FakeOobReader()
        identity = _identity()
        with tempfile.TemporaryDirectory() as td:
            cache_path = Path(td) / "oob_command_roots.json"
            self.assertTrue(ocb._write_oob_command_cache(
                cache_path,
                identity,
                [
                    {"kind": "fleet", "id": 61500, "body_ptr": 0x9000},
                    {"kind": "tf", "id": 61700, "body_ptr": 0xA000},
                ],
            ))
            with _patched_identity(identity):
                result = resolve_oob_command_bridge_preflight(
                    111,
                    reader_factory=lambda: reader,
                    cache_path=cache_path,
                    allow_cold_scan=False,
                )

        self.assertTrue(result.usable)
        self.assertEqual(result.diagnostics["source"], "oob_command_cache")
        self.assertEqual(result.diagnostics["cache"]["status"], "hit")
        self.assertFalse(result.diagnostics["allow_cold_scan"])

    def test_preflight_error_redacts_addresses(self):
        reader = FakeOobReader(build_error=OSError("bad read at 0xDEADBEEF"))

        result = resolve_oob_command_bridge_preflight(
            111,
            reader_factory=lambda: reader,
        )

        self.assertFalse(result.usable)
        self.assertIn("0x<redacted>", result.diagnostics["error"])
        self.assertNotIn("0xDEADBEEF", json.dumps(result.diagnostics, sort_keys=True))


def _identity():
    return SimpleNamespace(
        pid=111,
        create_time_100ns=222,
        exe_sha256="a" * 64,
        module_base=0x140000000,
    )


class _patched_identity:
    def __init__(self, identity):
        self.identity = identity
        self.original = ocb._process_identity.identify

    def __enter__(self):
        ocb._process_identity.identify = lambda _reader: self.identity

    def __exit__(self, exc_type, exc, tb):
        ocb._process_identity.identify = self.original


if __name__ == "__main__":
    unittest.main()
