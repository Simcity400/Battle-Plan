import json
import os
from pathlib import Path
from types import SimpleNamespace
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))

import live_capture.collector_tech_defs as tdf  # noqa: E402
from live_capture.collector_tech_defs import (  # noqa: E402
    encode_tech_def_filter,
    resolve_tech_def_filter_preflight,
    tech_def_filter_metadata,
)


class FakeTechDefReader:
    def __init__(self, *, pid=111, build_ok=True, build_error=None):
        self.pid = pid
        self.build_ok = build_ok
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

    def build_tech_defs(self):
        return {
            0x1234: "basic_machine_tools",
            0x1235: "construction1",
        }

    def close(self):
        self.closed = True


class TestCollectorTechDefs(unittest.TestCase):
    def test_encode_tech_def_filter_sorts_and_sanitizes_rows(self):
        encoded = encode_tech_def_filter({
            "0x1235": "construction1",
            0x1234: "basic_machine_tools",
        })

        self.assertEqual(encoded, "4660=basic_machine_tools;4661=construction1")
        meta = tech_def_filter_metadata(encoded)
        self.assertTrue(meta["tech_def_filter_supplied"])
        self.assertEqual(meta["tech_def_filter_rows"], 2)
        self.assertIsInstance(meta["tech_def_filter_sha256"], str)

    def test_encode_tech_def_filter_rejects_bad_rows(self):
        with self.assertRaises(ValueError):
            encode_tech_def_filter({0: "basic_machine_tools"})
        with self.assertRaises(ValueError):
            encode_tech_def_filter({1: "bad token!"})
        with self.assertRaises(ValueError):
            encode_tech_def_filter([(1, "a"), (1, "b")])

    def test_preflight_resolves_cold_scan_without_tokens_in_diagnostics(self):
        reader = FakeTechDefReader()

        result = resolve_tech_def_filter_preflight(
            111,
            reader_factory=lambda: reader,
            allow_cold_scan=True,
        )

        self.assertTrue(result.usable)
        self.assertEqual(result.encoded, "4660=basic_machine_tools;4661=construction1")
        self.assertEqual(result.diagnostics["status"], "resolved")
        self.assertEqual(result.diagnostics["row_count"], 2)
        self.assertFalse(result.diagnostics["hot_loop_heap_scan"])
        self.assertNotIn("basic_machine_tools", json.dumps(result.diagnostics, sort_keys=True))
        self.assertTrue(reader.closed)

    def test_cache_hit_uses_cached_rows_without_cold_scan(self):
        reader = FakeTechDefReader()
        identity = _identity()
        with tempfile.TemporaryDirectory() as td:
            cache_path = Path(td) / "tech_defs.json"
            self.assertTrue(tdf._write_tech_def_cache(
                cache_path,
                identity,
                {
                    0x1234: "basic_machine_tools",
                    0x1235: "construction1",
                },
            ))
            with _patched_identity(identity):
                result = resolve_tech_def_filter_preflight(
                    111,
                    reader_factory=lambda: reader,
                    cache_path=cache_path,
                    allow_cold_scan=False,
                )

        self.assertTrue(result.usable)
        self.assertEqual(result.diagnostics["source"], "tech_def_cache")
        self.assertEqual(result.diagnostics["cache"]["status"], "hit")
        self.assertFalse(result.diagnostics["allow_cold_scan"])

    def test_preflight_error_redacts_addresses(self):
        reader = FakeTechDefReader(build_error=OSError("bad read at 0xDEADBEEF"))

        result = resolve_tech_def_filter_preflight(
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
        self.original = tdf._process_identity.identify

    def __enter__(self):
        tdf._process_identity.identify = lambda _reader: self.identity

    def __exit__(self, exc_type, exc, tb):
        tdf._process_identity.identify = self.original


if __name__ == "__main__":
    unittest.main()
