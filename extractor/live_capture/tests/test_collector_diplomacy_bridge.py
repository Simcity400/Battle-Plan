import json
import os
from types import SimpleNamespace
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))

import live_capture.collector_diplomacy_bridge as cdb  # noqa: E402
from live_capture.collector_diplomacy_bridge import (  # noqa: E402
    encode_war_relation_bridge,
    resolve_war_relation_bridge_preflight,
)


class FakeReader:
    def __init__(self, *, pid=111, build_ok=True):
        self.pid = pid
        self.module_base = 0x140000000
        self.module_path = None
        self.closed = False
        self.build_ok = build_ok

    def fingerprint(self):
        return {
            "sha256": (
                "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120"
                if self.build_ok else "0" * 64
            )
        }

    def close(self):
        self.closed = True


class FakeIdentity:
    pass


class FakeTagTable:
    def __init__(self):
        self._tags = {0: "---"}
        for idx in range(1, 50):
            self._tags[idx] = f"D{idx % 100:02d}"
        self._tags[1] = "GER"
        self._tags[3] = "SOV"

    def as_map(self):
        return dict(self._tags)

    def tag(self, tag_db_id):
        return self._tags.get(tag_db_id)


class IndexInt:
    def __init__(self, value):
        self.value = value

    def __index__(self):
        return self.value


class TestCollectorDiplomacyBridge(unittest.TestCase):
    def test_encode_war_relation_bridge_sorts_and_sanitizes_rows(self):
        encoded = encode_war_relation_bridge({
            (3, 1): IndexInt(0x10000000004),
            (1, 3): 0x10000000000,
        })

        self.assertEqual(encoded, "1:3:0x10000000000;3:1:0x10000000004")
        self.assertEqual(encode_war_relation_bridge(encoded), encoded)

    def test_encode_war_relation_bridge_rejects_bad_rows(self):
        with self.assertRaises(ValueError):
            encode_war_relation_bridge([((1, 1), 0x10000000000)])
        with self.assertRaises(ValueError):
            encode_war_relation_bridge([((1, 3), 0x10000000002)])
        with self.assertRaises(ValueError):
            encode_war_relation_bridge([((1, 3), 0x10000000000), ((1, 3), 0x10000000004)])
        with self.assertRaises(ValueError):
            encode_war_relation_bridge([((1, 3), 0x10000000000), ((3, 1), 0x10000000000)])

    def test_preflight_resolves_encoded_bridge_with_sanitized_diagnostics(self):
        reader = FakeReader()
        fake_dl = SimpleNamespace(
            CountryIdentity=SimpleNamespace(resolve=lambda _reader: FakeIdentity()),
            read_war_relations=lambda _reader, _ident, want_addrs=False: (
                [{
                    "first": "GER",
                    "second": "SOV",
                    "first_casualties": 100,
                    "second_casualties": 200,
                }],
                {},
                {(1, 3): 0x10000000000},
            ),
            at_war_sets=lambda _relations: {"GER": ["SOV"], "SOV": ["GER"]},
            country_total_casualties=lambda _relations: {"GER": 100, "SOV": 200},
        )

        with _patched_module("diplomacy_live", fake_dl):
            result = resolve_war_relation_bridge_preflight(
                111,
                reader_factory=lambda: reader,
            )

        self.assertTrue(result.usable)
        self.assertEqual(result.encoded, "1:3:0x10000000000")
        self.assertEqual(result.row_count, 1)
        self.assertEqual(result.diagnostics["relation_count"], 1)
        self.assertEqual(result.diagnostics["at_war_country_count"], 2)
        self.assertEqual(result.diagnostics["address_manifest_policy"], "omitted")
        self.assertFalse(result.diagnostics["hot_loop_heap_scan"])
        self.assertNotIn("0x10000000000", json.dumps(result.diagnostics, sort_keys=True))
        self.assertTrue(reader.closed)

    def test_preflight_uses_tag_table_identity_fallback(self):
        reader = FakeReader()
        fake_dl = SimpleNamespace(
            CountryIdentity=SimpleNamespace(resolve=lambda _reader: None),
            tag_table=SimpleNamespace(
                TagTable=SimpleNamespace(locate=lambda _reader: FakeTagTable())
            ),
            read_war_relations=lambda _reader, _ident, want_addrs=False: (
                [{
                    "first": "GER",
                    "second": "SOV",
                    "first_casualties": 100,
                    "second_casualties": 200,
                }],
                {},
                {(1, 3): 0x10000000000},
            ),
            at_war_sets=lambda _relations: {"GER": ["SOV"], "SOV": ["GER"]},
            country_total_casualties=lambda _relations: {"GER": 100, "SOV": 200},
        )

        with _patched_module("diplomacy_live", fake_dl):
            result = resolve_war_relation_bridge_preflight(
                111,
                reader_factory=lambda: reader,
            )

        self.assertTrue(result.usable)
        self.assertEqual(result.diagnostics["identity_source"], "tag_table")
        self.assertEqual(result.encoded, "1:3:0x10000000000")

    def test_preflight_fails_closed_on_subtype_warning(self):
        reader = FakeReader()
        fake_dl = SimpleNamespace(
            CountryIdentity=SimpleNamespace(resolve=lambda _reader: FakeIdentity()),
            read_war_relations=lambda _reader, _ident, want_addrs=False: (
                [],
                {0xDEADBEEF: 1},
                {},
            ),
            at_war_sets=lambda _relations: {},
            country_total_casualties=lambda _relations: {},
        )

        with _patched_module("diplomacy_live", fake_dl):
            result = resolve_war_relation_bridge_preflight(
                111,
                reader_factory=lambda: reader,
            )

        self.assertFalse(result.usable)
        self.assertEqual(result.status, "subtype_warning")
        self.assertEqual(result.diagnostics["subtype_warning_count"], 1)
        self.assertIsNone(result.encoded)

    def test_preflight_resolves_bridge_with_advisory_warning(self):
        reader = FakeReader()
        fake_dl = SimpleNamespace(
            CountryIdentity=SimpleNamespace(resolve=lambda _reader: FakeIdentity()),
            read_war_relations=lambda _reader, _ident, want_addrs=False: (
                [{
                    "first": "GER",
                    "second": "SOV",
                    "first_casualties": 100,
                    "second_casualties": 200,
                }],
                {0xDEADBEEF: 1},
                {(1, 3): 0x10000000000},
            ),
            at_war_sets=lambda _relations: {"GER": ["SOV"], "SOV": ["GER"]},
            country_total_casualties=lambda _relations: {"GER": 100, "SOV": 200},
        )

        with _patched_module("diplomacy_live", fake_dl):
            result = resolve_war_relation_bridge_preflight(
                111,
                reader_factory=lambda: reader,
            )

        self.assertTrue(result.usable)
        self.assertEqual(result.encoded, "1:3:0x10000000000")
        self.assertEqual(result.diagnostics["subtype_warning_count"], 1)
        self.assertTrue(result.diagnostics["advisory_subtype_warning"])


class _patched_module:
    def __init__(self, name, module):
        self.name = name
        self.module = module
        self.previous = None

    def __enter__(self):
        self.previous = sys.modules.get(self.name)
        sys.modules[self.name] = self.module

    def __exit__(self, *exc):
        if self.previous is None:
            sys.modules.pop(self.name, None)
        else:
            sys.modules[self.name] = self.previous


if __name__ == "__main__":
    unittest.main()
