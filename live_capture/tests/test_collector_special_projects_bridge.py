import json
import os
from pathlib import Path
from types import SimpleNamespace
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))

import live_capture.collector_special_projects_bridge as spb  # noqa: E402
from live_capture.collector_special_projects_bridge import (  # noqa: E402
    encode_special_projects_bridge,
    resolve_special_projects_bridge_preflight,
)


HEAP = 0x10000000000


class FakeSpecialProjectsReader:
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

    def enumerate_special_project_objs(self):
        return {
            "facility": [(HEAP, (1, 118))],
            "project": [(HEAP + 0x100, (77, 9001, 83, 1))],
            "scientist": [(HEAP + 0x200, (44,))],
        }

    def validate_row(self, kind, object_id, aux_id, body_ptr):
        if not self.cached_valid:
            return False
        return (kind, object_id, aux_id, body_ptr) in {
            ("facility", 1, 118, HEAP),
            ("project", 77, 9001, HEAP + 0x100),
            ("scientist", 44, 0, HEAP + 0x200),
        }

    def close(self):
        self.closed = True


class TestCollectorSpecialProjectsBridge(unittest.TestCase):
    def test_encode_special_projects_bridge_sorts_and_sanitizes_rows(self):
        encoded = encode_special_projects_bridge([
            {"kind": "scientist", "id": 44, "aux_id": 0, "body_ptr": HEAP + 0x200},
            {"kind": "project", "id": "77", "aux_id": "9001", "body_ptr": HEAP + 0x100},
            {"kind": "facility", "id": 1, "aux_id": 118, "body_ptr": f"0x{HEAP:X}"},
        ])

        self.assertEqual(
            encoded,
            "facility:1:118:0x10000000000;"
            "project:77:9001:0x10000000100;"
            "scientist:44:0x10000000200",
        )

    def test_encode_special_projects_bridge_accepts_projects_live_enumeration_shape(self):
        encoded = encode_special_projects_bridge({
            "facility": [(HEAP, (1, 118))],
            "project": [(HEAP + 0x100, (77, 9001, 83, 1))],
            "scientist": [(HEAP + 0x200, (44,))],
        })

        self.assertIn("facility:1:118:0x10000000000", encoded)
        self.assertIn("project:77:9001:0x10000000100", encoded)
        self.assertIn("scientist:44:0x10000000200", encoded)

    def test_encode_special_projects_bridge_rejects_bad_rows(self):
        with self.assertRaises(ValueError):
            encode_special_projects_bridge([("facility", 1, 118, HEAP + 1)])
        with self.assertRaises(ValueError):
            encode_special_projects_bridge([
                ("facility", 1, 118, HEAP),
                ("facility", 1, 118, HEAP + 8),
            ])
        with self.assertRaises(ValueError):
            encode_special_projects_bridge([
                ("facility", 1, 118, HEAP),
                ("project", 77, 9001, HEAP),
            ])
        with self.assertRaises(ValueError):
            encode_special_projects_bridge([("project", 77, 0, HEAP)])
        with self.assertRaises(ValueError):
            encode_special_projects_bridge([("unknown", 1, 0, HEAP)])

    def test_preflight_resolves_cold_scan_without_address_diagnostics(self):
        reader = FakeSpecialProjectsReader()

        result = resolve_special_projects_bridge_preflight(
            111,
            reader_factory=lambda: reader,
            allow_cold_scan=True,
        )

        self.assertTrue(result.usable)
        self.assertIn("facility:1:118:0x10000000000", result.encoded)
        self.assertIn("project:77:9001:0x10000000100", result.encoded)
        self.assertIn("scientist:44:0x10000000200", result.encoded)
        self.assertEqual(result.diagnostics["status"], "resolved")
        self.assertEqual(result.diagnostics["counts"]["facility"], 1)
        self.assertEqual(result.diagnostics["counts"]["project"], 1)
        self.assertEqual(result.diagnostics["counts"]["scientist"], 1)
        self.assertFalse(result.diagnostics["hot_loop_heap_scan"])
        self.assertNotIn("0x10000000000", json.dumps(result.diagnostics, sort_keys=True))
        self.assertTrue(reader.closed)

    def test_cache_hit_validates_rows_without_cold_scan(self):
        reader = FakeSpecialProjectsReader()
        identity = _identity()
        with tempfile.TemporaryDirectory() as td:
            cache_path = Path(td) / "special_projects_roots.json"
            self.assertTrue(spb._write_special_projects_cache(
                cache_path,
                identity,
                [
                    {"kind": "facility", "id": 1, "aux_id": 118, "body_ptr": HEAP},
                    {"kind": "project", "id": 77, "aux_id": 9001, "body_ptr": HEAP + 0x100},
                    {"kind": "scientist", "id": 44, "aux_id": 0, "body_ptr": HEAP + 0x200},
                ],
            ))
            with _patched_identity(identity):
                result = resolve_special_projects_bridge_preflight(
                    111,
                    reader_factory=lambda: reader,
                    cache_path=cache_path,
                    allow_cold_scan=False,
                )

        self.assertTrue(result.usable)
        self.assertEqual(result.diagnostics["source"], "special_projects_cache")
        self.assertEqual(result.diagnostics["cache"]["status"], "hit")
        self.assertFalse(result.diagnostics["allow_cold_scan"])

    def test_preflight_error_redacts_addresses(self):
        reader = FakeSpecialProjectsReader(build_error=OSError("bad read at 0xDEADBEEF"))

        result = resolve_special_projects_bridge_preflight(
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
        self.original = spb._process_identity.identify

    def __enter__(self):
        spb._process_identity.identify = lambda _reader: self.identity

    def __exit__(self, exc_type, exc, tb):
        spb._process_identity.identify = self.original


if __name__ == "__main__":
    unittest.main()
