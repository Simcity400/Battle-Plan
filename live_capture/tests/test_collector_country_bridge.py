import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))

from live_capture.collector_country_bridge import (  # noqa: E402
    country_object_bridge_rows_from_bridge,
    country_object_bridge_rows_from_country_array,
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


if __name__ == "__main__":
    unittest.main()
