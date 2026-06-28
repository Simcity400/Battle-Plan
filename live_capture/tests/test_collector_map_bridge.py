import os
from pathlib import Path
import sys
from types import SimpleNamespace
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))

from live_capture import collector_map_bridge as cmb  # noqa: E402


class FakeStateModule:
    STATE_VTABLE_REL = 0x100
    OFF_SID = 0x58
    OFF_OWNER = 0xC8
    OFF_CTRL = 0xCC


class FakeProc:
    module_base = 0x100000

    def __init__(self):
        self.mem = {}
        self._add_state(1, 0x2000, owner=1, controller=1)
        self._add_state(2, 0x3000, owner=2, controller=0)

    def _add_state(self, sid, obj, *, owner, controller):
        self.mem[obj] = self.module_base + FakeStateModule.STATE_VTABLE_REL
        self.mem[obj + FakeStateModule.OFF_SID] = sid
        self.mem[obj + FakeStateModule.OFF_OWNER] = owner
        self.mem[obj + FakeStateModule.OFF_CTRL] = controller

    def u64(self, addr):
        if addr not in self.mem:
            raise OSError("bad u64")
        return self.mem[addr]

    def u32(self, addr):
        if addr not in self.mem:
            raise OSError("bad u32")
        return self.mem[addr]


class FakeTable:
    def tag(self, tag_db_id):
        return {1: "GER", 2: "ENG"}.get(tag_db_id)


class TestCollectorMapBridge(unittest.TestCase):
    def test_state_rows_from_state_roots_rereads_owner_and_controller(self):
        rows = cmb.state_rows_from_state_roots(
            FakeProc(),
            FakeTable(),
            {1: 0x2000, 2: 0x3000},
            FakeStateModule,
        )

        self.assertEqual(rows["1"], {"owner": "GER", "controller": "GER"})
        self.assertEqual(rows["2"], {"owner": "ENG", "controller": None})

    def test_state_roots_validation_fails_on_bad_identity(self):
        original_min = cmb.MIN_USABLE_STATE_ROWS
        try:
            cmb.MIN_USABLE_STATE_ROWS = 2
            proc = FakeProc()
            self.assertTrue(cmb._state_roots_valid(
                proc,
                {1: 0x2000, 2: 0x3000},
                FakeStateModule,
            ))

            proc.mem[0x2000 + FakeStateModule.OFF_SID] = 99

            self.assertFalse(cmb._state_roots_valid(
                proc,
                {1: 0x2000, 2: 0x3000},
                FakeStateModule,
            ))
        finally:
            cmb.MIN_USABLE_STATE_ROWS = original_min

    def test_state_roots_cache_loads_only_matching_process_identity(self):
        original_min = cmb.MIN_USABLE_STATE_ROWS
        try:
            cmb.MIN_USABLE_STATE_ROWS = 2
            identity = SimpleNamespace(
                pid=111,
                create_time_100ns=222,
                exe_sha256="a" * 64,
                module_base=0x100000,
            )
            other_identity = SimpleNamespace(
                pid=111,
                create_time_100ns=333,
                exe_sha256="a" * 64,
                module_base=0x100000,
            )
            with tempfile.TemporaryDirectory() as td:
                path = Path(td) / "state_roots.json"

                self.assertTrue(cmb._write_state_roots_cache(
                    path,
                    identity,
                    {1: 0x2000, 2: 0x3000},
                ))
                loaded = cmb._load_state_roots_cache(path, identity)

                self.assertIsNotNone(loaded)
                self.assertEqual(loaded["state_count"], 2)
                self.assertIsNone(cmb._load_state_roots_cache(path, other_identity))
        finally:
            cmb.MIN_USABLE_STATE_ROWS = original_min

    def test_state_diagnostics_omit_addresses(self):
        diag = cmb._state_diagnostics(
            "resolved",
            {
                "1": {"owner": "GER", "controller": "GER"},
                "2": {"owner": "ENG", "controller": None},
            },
            source="state_roots_cache",
            cache={
                "enabled": True,
                "status": "hit",
                "tag_table_source": "country_database_root",
            },
        )

        self.assertEqual(diag["row_count"], 2)
        self.assertEqual(diag["owner_rows"], 2)
        self.assertEqual(diag["controller_rows"], 1)
        self.assertEqual(diag["cache"]["status"], "hit")
        self.assertNotIn("0x2000", str(diag))


if __name__ == "__main__":
    unittest.main()
