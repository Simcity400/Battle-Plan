import os
from pathlib import Path
import sys
from types import SimpleNamespace
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))

from live_capture.collector_country_bridge import (  # noqa: E402
    _encoded_preflight,
    _load_preflight_cache,
    _write_preflight_cache,
    country_object_bridge_rows_from_bridge,
    country_object_bridge_rows_from_country_array,
    tag_table_from_country_database_root,
)
from live_capture.collector_probe_record import (  # noqa: E402
    encode_country_object_bridge,
    encode_country_record_bridge,
)


class FakeBridge:
    def as_map(self):
        return {2: 54, 1: 12, 3: 61}

    def tag(self, tag_db_id):
        return {1: "GER", 2: "ENG", 3: "SOV"}.get(tag_db_id)

    def country_object(self, tag_db_id):
        return {1: 0x1000, 2: 0x2000, 3: 0}.get(tag_db_id)


class FakeProc:
    def u16(self, addr):
        if addr == 0x1000 + 0x5D:
            return 0
        if addr == 0x1000 + 0x120 + 0x5D:
            return 1
        return 999

    def u64(self, addr):
        if addr == 0x1000 + 0x08:
            return 0x4000
        if addr == 0x1000 + 0x120 + 0x08:
            return 0x5000
        return 0

    def u32(self, addr):
        if addr == 0x4000 + 0x08:
            return 2
        if addr == 0x5000 + 0x08:
            return 1
        return 0


class FakeTable:
    def tag(self, tag_db_id):
        return {1: "GER", 2: "ENG"}.get(tag_db_id)


class FakeRootTagProc:
    module_base = 0x70000000

    def __init__(self):
        self.table_base = 0x5000
        self.tags = ["---", "GER", "ENG", "SOV", "SWE", "FRA", "LUX", "BEL",
                     "HOL", "CZE", "POL"] + [f"T{i:02d}" for i in range(40)]

    def u64(self, addr):
        if addr == self.module_base + 0x33063F8:
            return 0x4000
        if addr == 0x4000:
            return self.table_base
        raise OSError("bad u64")

    def u32(self, addr):
        if addr == 0x4000 + 8:
            return len(self.tags) + 5
        if addr == 0x4000 + 12:
            return len(self.tags)
        raise OSError("bad u32")


class FakeTagTableModule:
    STRIDE = 0x20

    class TagTable:
        def __init__(self, proc, base, count, tags):
            self.proc = proc
            self.base = base
            self.count = count
            self.tags = tags

        @staticmethod
        def _read_record(proc, addr):
            idx = (addr - proc.table_base) // FakeTagTableModule.STRIDE
            if 0 <= idx < len(proc.tags):
                return proc.tags[idx]
            return None

        @staticmethod
        def _is_canonical(tags):
            prefix = ["---", "GER", "ENG", "SOV", "SWE", "FRA", "LUX", "BEL",
                      "HOL", "CZE", "POL"]
            return tags[:len(prefix)] == prefix and len(set(tags[1:])) == len(tags[1:])

        def tag(self, tag_db_id):
            if 0 <= tag_db_id < len(self.tags):
                return self.tags[tag_db_id]
            return None


class TestCollectorCountryBridge(unittest.TestCase):
    def test_rows_from_bridge_are_sorted_and_skip_missing_objects(self):
        rows = country_object_bridge_rows_from_bridge(FakeBridge())

        self.assertEqual(
            rows,
            [
                {"tag_db_id": 1, "tag": "GER", "object_ptr": 0x1000},
                {"tag_db_id": 2, "tag": "ENG", "object_ptr": 0x2000},
            ],
        )
        self.assertEqual(encode_country_object_bridge(rows), "1:0x1000;2:0x2000")

    def test_rows_from_country_array_use_object_tag_db_id(self):
        rows = country_object_bridge_rows_from_country_array(
            FakeProc(),
            FakeTable(),
            (0x1000, None, 2),
        )

        self.assertEqual(
            rows,
            [
                {
                    "tag_db_id": 1,
                    "tag": "GER",
                    "object_ptr": 0x5000,
                    "record_ptr": 0x1120,
                    "running_index": 1,
                },
                {
                    "tag_db_id": 2,
                    "tag": "ENG",
                    "object_ptr": 0x4000,
                    "record_ptr": 0x1000,
                    "running_index": 0,
                },
            ],
        )
        self.assertEqual(encode_country_object_bridge(rows), "1:0x5000;2:0x4000")
        self.assertEqual(encode_country_record_bridge(rows), "1:0x1120;2:0x1000")

    def test_tag_table_from_country_database_root(self):
        table = tag_table_from_country_database_root(
            FakeRootTagProc(),
            FakeTagTableModule,
        )

        self.assertIsNotNone(table)
        self.assertEqual(table.tag(1), "GER")
        self.assertEqual(table.tag(10), "POL")

    def test_preflight_cache_loads_only_matching_process_identity(self):
        identity = SimpleNamespace(
            pid=111,
            create_time_100ns=222,
            exe_sha256="a" * 64,
            module_base=0x70000000,
        )
        other_identity = SimpleNamespace(
            pid=111,
            create_time_100ns=333,
            exe_sha256="a" * 64,
            module_base=0x70000000,
        )
        with tempfile.TemporaryDirectory() as td:
            path = Path(td) / "country_array.json"

            self.assertTrue(_write_preflight_cache(path, identity, (0x1000, 0x1000, 227)))
            loaded = _load_preflight_cache(path, identity)

            self.assertIsNotNone(loaded)
            self.assertEqual(loaded["rec0"], 0x1000)
            self.assertIsNone(_load_preflight_cache(path, other_identity))

    def test_encoded_preflight_reports_cache_status_without_addresses(self):
        preflight = _encoded_preflight(
            "resolved",
            [{"tag_db_id": 1, "tag": "GER", "object_ptr": 0x5000, "record_ptr": 0x1000}],
            source="live_country_array_cache",
            cache={
                "enabled": True,
                "status": "hit",
                "tag_table_source": "country_database_root",
            },
        )

        self.assertEqual(preflight.diagnostics["cache"], {
            "enabled": True,
            "status": "hit",
            "tag_table_source": "country_database_root",
        })
        self.assertNotIn("0x5000", str(preflight.diagnostics))


if __name__ == "__main__":
    unittest.main()
