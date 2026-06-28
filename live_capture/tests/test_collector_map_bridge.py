import os
from pathlib import Path
import sys
from types import ModuleType, SimpleNamespace
import tempfile
import unittest
from unittest.mock import patch

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


class FakeProcessContext:
    def __init__(self, proc):
        self.proc = proc

    def __enter__(self):
        return self.proc

    def __exit__(self, exc_type, exc, tb):
        return False


class FakeTable:
    def tag(self, tag_db_id):
        return {1: "GER", 2: "ENG", 3: "SOV"}.get(tag_db_id)

    def covers(self, tag_db_ids):
        return all(self.tag(tag_db_id) for tag_db_id in tag_db_ids)


class ShortTable(FakeTable):
    def tag(self, tag_db_id):
        return {1: "GER", 2: "ENG"}.get(tag_db_id)


class HighIdTable(FakeTable):
    def tag(self, tag_db_id):
        if tag_db_id == 250:
            return "HIG"
        return super().tag(tag_db_id)


class FakeProvinceModule:
    PROV_STRIDE = 8
    HEAP_LO = 0x7000
    HEAP_HI = 0xB000
    MIN_CONTROLLERS = 2
    MAX_CONTROLLERS = 250


def _fake_resolve_modules(proc, prov_to_state, locate_array):
    winmem = ModuleType("winmem")
    winmem.Process = lambda pid, name: FakeProcessContext(proc)

    state_detail_live = ModuleType("state_detail_live")
    state_detail_live.STATE_VTABLE_REL = FakeStateModule.STATE_VTABLE_REL
    state_detail_live.OFF_SID = FakeStateModule.OFF_SID
    state_detail_live.OFF_OWNER = FakeStateModule.OFF_OWNER
    state_detail_live.OFF_CTRL = FakeStateModule.OFF_CTRL
    state_detail_live.StateDetailLive = SimpleNamespace(
        resolve=lambda proc: SimpleNamespace(states={1: 0x2000, 2: 0x3000})
    )

    province_control = ModuleType("live_layers.province_control")
    province_control.PROV_STRIDE = FakeProvinceModule.PROV_STRIDE
    province_control.HEAP_LO = FakeProvinceModule.HEAP_LO
    province_control.HEAP_HI = FakeProvinceModule.HEAP_HI
    province_control.MIN_CONTROLLERS = FakeProvinceModule.MIN_CONTROLLERS
    province_control.MAX_CONTROLLERS = FakeProvinceModule.MAX_CONTROLLERS
    province_control.load_province_to_state = lambda: dict(prov_to_state)
    province_control.locate_array = locate_array
    province_control._build_expected = (
        lambda state_ctrl, prov_to_state: {
            prov: state_ctrl[st]
            for prov, st in prov_to_state.items()
            if state_ctrl.get(st)
        }
    )

    live_layers = ModuleType("live_layers")
    live_layers.province_control = province_control
    live_layers.tag_table = ModuleType("live_layers.tag_table")

    return {
        "winmem": winmem,
        "state_detail_live": state_detail_live,
        "live_layers": live_layers,
        "live_layers.province_control": province_control,
        "live_layers.tag_table": live_layers.tag_table,
    }


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

    def test_map_control_state_roots_encoder_sorts_rows(self):
        encoded = cmb.encode_map_control_state_roots({2: 0x3000, "1": 0x2000})

        self.assertEqual(encoded, "1:0x2000;2:0x3000")

    def test_state_controller_ids_from_roots_rereads_live_values(self):
        proc = FakeProc()
        roots = {1: 0x2000, 2: 0x3000}

        first = cmb.state_controller_ids_from_state_roots(
            proc,
            roots,
            FakeStateModule,
        )
        self.assertEqual(first, {1: 1, 2: 0})

        proc.mem[0x2000 + FakeStateModule.OFF_CTRL] = 2
        second = cmb.state_controller_ids_from_state_roots(
            proc,
            roots,
            FakeStateModule,
        )
        self.assertEqual(second, {1: 2, 2: 0})

    def test_state_controller_ids_accept_table_covered_high_tag_ids(self):
        proc = FakeProc()
        roots = {1: 0x2000, 2: 0x3000}
        proc.mem[0x2000 + FakeStateModule.OFF_CTRL] = 250

        self.assertEqual(
            cmb.state_controller_ids_from_state_roots(
                proc,
                roots,
                FakeStateModule,
                table=HighIdTable(),
            ),
            {1: 250, 2: 0},
        )
        self.assertEqual(
            cmb.state_controller_ids_from_state_roots(
                proc,
                roots,
                FakeStateModule,
                table=ShortTable(),
            ),
            {2: 0},
        )

    def test_province_rows_from_cached_base_rereads_live_slots(self):
        proc = FakeProc()
        base = 0x4000
        prov_to_state = {11: 1, 12: 2}
        proc.mem[base + 11 * FakeProvinceModule.PROV_STRIDE] = 0x8000
        proc.mem[base + 12 * FakeProvinceModule.PROV_STRIDE] = 0x9000

        rows = cmb.province_rows_from_cached_base(
            proc,
            FakeTable(),
            base,
            20,
            {0x8000: 1, 0x9000: 2},
            prov_to_state,
            FakeProvinceModule,
        )
        self.assertEqual(rows["11"], {"controller": "GER", "state_id": 1})
        self.assertEqual(rows["12"], {"controller": "ENG", "state_id": 2})

        proc.mem[base + 11 * FakeProvinceModule.PROV_STRIDE] = 0x9000
        rows = cmb.province_rows_from_cached_base(
            proc,
            FakeTable(),
            base,
            20,
            {0x8000: 1, 0x9000: 2},
            prov_to_state,
            FakeProvinceModule,
        )
        self.assertEqual(rows["11"], {"controller": "ENG", "state_id": 1})

    def test_province_ptr_bridge_from_cached_base_uses_state_controller_votes(self):
        proc = FakeProc()
        base = 0x4000
        prov_to_state = {11: 1, 12: 1, 13: 2}
        state_ctrl = {1: 1, 2: 2}
        proc.mem[base + 11 * FakeProvinceModule.PROV_STRIDE] = 0x8000
        proc.mem[base + 12 * FakeProvinceModule.PROV_STRIDE] = 0x8000
        proc.mem[base + 13 * FakeProvinceModule.PROV_STRIDE] = 0x9000

        bridge = cmb.province_ptr_bridge_from_cached_base(
            proc,
            base,
            20,
            state_ctrl,
            prov_to_state,
            FakeProvinceModule,
        )

        self.assertEqual(bridge, {0x8000: 1, 0x9000: 2})

    def test_province_array_cache_validation_accepts_cached_override_bridge(self):
        proc = FakeProc()
        base = 0x4000
        prov_to_state = {11: 1, 12: 1, 13: 2}
        state_ctrl = {1: 1, 2: 2}
        proc.mem[base + 11 * FakeProvinceModule.PROV_STRIDE] = 0x8000
        proc.mem[base + 12 * FakeProvinceModule.PROV_STRIDE] = 0x8000
        proc.mem[base + 13 * FakeProvinceModule.PROV_STRIDE] = 0xA000

        ptr2tagid = {0x8000: 1, 0xA000: 3}

        self.assertIsNone(cmb.province_array_cache_validation_error(
            proc,
            base,
            20,
            ptr2tagid,
            FakeTable(),
            prov_to_state,
            FakeProvinceModule,
            state_ctrl=state_ctrl,
        ))
        rows = cmb.province_rows_from_cached_base(
            proc,
            FakeTable(),
            base,
            20,
            ptr2tagid,
            prov_to_state,
            FakeProvinceModule,
        )
        self.assertEqual(rows["13"], {"controller": "SOV", "state_id": 2})

    def test_province_array_cache_validation_rejects_short_tag_table(self):
        proc = FakeProc()
        base = 0x4000
        prov_to_state = {11: 1, 12: 2}
        proc.mem[base + 11 * FakeProvinceModule.PROV_STRIDE] = 0x8000
        proc.mem[base + 12 * FakeProvinceModule.PROV_STRIDE] = 0xA000

        reason = cmb.province_array_cache_validation_error(
            proc,
            base,
            20,
            {0x8000: 1, 0xA000: 3},
            ShortTable(),
            prov_to_state,
            FakeProvinceModule,
        )

        self.assertEqual(reason, "tag_table_uncovered")
        self.assertFalse(cmb._province_array_cache_valid(
            proc,
            base,
            20,
            {0x8000: 1, 0xA000: 3},
            ShortTable(),
            prov_to_state,
            FakeProvinceModule,
        ))

    def test_province_array_cache_validation_accepts_stale_unused_bridge_entry(self):
        proc = FakeProc()
        base = 0x4000
        prov_to_state = {11: 1, 12: 2}
        proc.mem[base + 11 * FakeProvinceModule.PROV_STRIDE] = 0x8000
        proc.mem[base + 12 * FakeProvinceModule.PROV_STRIDE] = 0x9000

        reason = cmb.province_array_cache_validation_error(
            proc,
            base,
            20,
            {0x8000: 1, 0x9000: 2, 0xA000: 3},
            FakeTable(),
            prov_to_state,
            FakeProvinceModule,
        )

        self.assertIsNone(reason)

    def test_province_preflight_cache_miss_without_cold_scan_fails_closed(self):
        proc = FakeProc()
        proc.mem[0x3000 + FakeStateModule.OFF_CTRL] = 2
        prov_to_state = {11: 1, 12: 2}
        identity = SimpleNamespace(
            pid=111,
            create_time_100ns=222,
            exe_sha256="a" * 64,
            module_base=proc.module_base,
        )

        def locate_array(unused_proc, unused_expected):
            raise AssertionError("cold locator should not run")

        original_state_min = cmb.MIN_USABLE_STATE_ROWS
        try:
            cmb.MIN_USABLE_STATE_ROWS = 2
            with tempfile.TemporaryDirectory() as td:
                root = Path(td)
                self.assertTrue(cmb._write_state_roots_cache(
                    root / "state_roots.json",
                    identity,
                    {1: 0x2000, 2: 0x3000},
                ))
                with patch.dict(
                    sys.modules,
                    _fake_resolve_modules(proc, prov_to_state, locate_array),
                ), patch.object(
                    cmb._process_identity,
                    "identify",
                    return_value=identity,
                ), patch.object(
                    cmb,
                    "tag_table_from_country_database_root",
                    return_value=FakeTable(),
                ):
                    result = cmb.resolve_province_control_preflight(
                        111,
                        min_rows=2,
                        cache_path=root / "province_control.json",
                        state_cache_path=root / "state_roots.json",
                        allow_cold_scan=False,
                    )
        finally:
            cmb.MIN_USABLE_STATE_ROWS = original_state_min

        self.assertEqual(result.status, "province_array_cache_unavailable")
        self.assertEqual(result.row_count, 0)
        self.assertIsNone(result.province_bridge_encoded)
        self.assertEqual(result.diagnostics["cache"]["status"], "miss")
        self.assertEqual(result.diagnostics["state_bridge"]["status"], "hit")

    def test_province_preflight_cold_scan_resolves_and_stores_cache(self):
        proc = FakeProc()
        base = 0x4000
        proc.mem[0x3000 + FakeStateModule.OFF_CTRL] = 2
        proc.mem[base + 11 * FakeProvinceModule.PROV_STRIDE] = 0x8000
        proc.mem[base + 12 * FakeProvinceModule.PROV_STRIDE] = 0x9000
        prov_to_state = {11: 1, 12: 2}
        identity = SimpleNamespace(
            pid=111,
            create_time_100ns=222,
            exe_sha256="a" * 64,
            module_base=proc.module_base,
        )
        locate_calls = []

        def locate_array(unused_proc, expected):
            locate_calls.append(dict(expected))
            return base, 20

        original_state_min = cmb.MIN_USABLE_STATE_ROWS
        try:
            cmb.MIN_USABLE_STATE_ROWS = 2
            with tempfile.TemporaryDirectory() as td:
                root = Path(td)
                cache_path = root / "province_control.json"
                self.assertTrue(cmb._write_state_roots_cache(
                    root / "state_roots.json",
                    identity,
                    {1: 0x2000, 2: 0x3000},
                ))
                with patch.dict(
                    sys.modules,
                    _fake_resolve_modules(proc, prov_to_state, locate_array),
                ), patch.object(
                    cmb._process_identity,
                    "identify",
                    return_value=identity,
                ), patch.object(
                    cmb,
                    "tag_table_from_country_database_root",
                    return_value=FakeTable(),
                ):
                    result = cmb.resolve_province_control_preflight(
                        111,
                        min_rows=2,
                        cache_path=cache_path,
                        state_cache_path=root / "state_roots.json",
                        allow_cold_scan=True,
                    )
                    loaded = cmb._load_province_array_cache(cache_path, identity)
        finally:
            cmb.MIN_USABLE_STATE_ROWS = original_state_min

        self.assertEqual(result.status, "resolved")
        self.assertEqual(result.row_count, 2)
        self.assertEqual(result.provinces["11"]["controller"], "GER")
        self.assertEqual(result.provinces["12"]["controller"], "ENG")
        self.assertEqual(locate_calls, [{11: 1, 12: 2}])
        self.assertIn("province_array=0x4000", result.province_bridge_encoded)
        self.assertEqual(result.controller_bridge_encoded, "1:0x8000;2:0x9000")
        self.assertEqual(result.diagnostics["cache"]["status"], "stored")
        self.assertIsNotNone(loaded)

    def test_province_preflight_stale_cache_without_cold_scan_fails_closed(self):
        proc = FakeProc()
        base = 0x4000
        proc.mem[0x3000 + FakeStateModule.OFF_CTRL] = 2
        proc.mem[base + 11 * FakeProvinceModule.PROV_STRIDE] = 0x8000
        proc.mem[base + 12 * FakeProvinceModule.PROV_STRIDE] = 0x9000
        prov_to_state = {11: 1, 12: 2}
        identity = SimpleNamespace(
            pid=111,
            create_time_100ns=222,
            exe_sha256="a" * 64,
            module_base=proc.module_base,
        )

        def locate_array(unused_proc, unused_expected):
            raise AssertionError("cold locator should not run")

        original_state_min = cmb.MIN_USABLE_STATE_ROWS
        try:
            cmb.MIN_USABLE_STATE_ROWS = 2
            with tempfile.TemporaryDirectory() as td:
                root = Path(td)
                self.assertTrue(cmb._write_state_roots_cache(
                    root / "state_roots.json",
                    identity,
                    {1: 0x2000, 2: 0x3000},
                ))
                self.assertTrue(cmb._write_province_array_cache(
                    root / "province_control.json",
                    identity,
                    base,
                    20,
                    {0x8000: 1, 0xA000: 2},
                    2,
                ))
                with patch.dict(
                    sys.modules,
                    _fake_resolve_modules(proc, prov_to_state, locate_array),
                ), patch.object(
                    cmb._process_identity,
                    "identify",
                    return_value=identity,
                ), patch.object(
                    cmb,
                    "tag_table_from_country_database_root",
                    return_value=FakeTable(),
                ):
                    result = cmb.resolve_province_control_preflight(
                        111,
                        min_rows=2,
                        cache_path=root / "province_control.json",
                        state_cache_path=root / "state_roots.json",
                        allow_cold_scan=False,
                    )
        finally:
            cmb.MIN_USABLE_STATE_ROWS = original_state_min

        self.assertEqual(result.status, "province_array_cache_unavailable")
        self.assertEqual(result.row_count, 0)
        self.assertIsNone(result.province_bridge_encoded)
        self.assertEqual(result.diagnostics["cache"]["status"], "stale")
        self.assertEqual(
            result.diagnostics["cache"]["validation"],
            "pointer_bridge_mismatch",
        )

    def test_province_array_cache_loads_only_matching_process_identity(self):
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
            path = Path(td) / "province_control.json"

            self.assertTrue(cmb._write_province_array_cache(
                path,
                identity,
                0x4000,
                20,
                {0x8000: 1, 0x9000: 2},
                2,
            ))
            loaded = cmb._load_province_array_cache(path, identity)

            self.assertIsNotNone(loaded)
            self.assertEqual(loaded["slots"], 20)
            self.assertEqual(cmb._coerce_ptr_bridge(loaded["ptr2tagid"]),
                             {0x8000: 1, 0x9000: 2})
            self.assertIsNone(cmb._load_province_array_cache(path, other_identity))

    def test_province_diagnostics_and_errors_omit_addresses(self):
        diag = cmb._province_diagnostics(
            "resolved",
            {
                "11": {"controller": "GER", "state_id": 1},
                "12": {"controller": "ENG", "state_id": 2},
            },
            source="province_array_cache",
            cache={
                "enabled": True,
                "status": "stale",
                "validation": "tag_table_uncovered",
                "tag_table_source": "country_database_root",
            },
            state_bridge={"cache_enabled": True, "status": "hit", "row_count": 2},
            slots=20,
            pointer_bridge_count=2,
        )
        self.assertEqual(diag["row_count"], 2)
        self.assertEqual(diag["cache"]["validation"], "tag_table_uncovered")
        self.assertEqual(diag["state_bridge"]["status"], "hit")
        self.assertNotIn("0x8000", str(diag))

        msg = cmb._safe_error_message(OSError("bad read at 0x8000"))
        self.assertNotIn("0x8000", msg)

    def test_map_control_province_encoders_match_rust_contract(self):
        province_bridge = cmb.encode_map_control_province_bridge(
            0x4000,
            20,
            FakeProvinceModule,
        )
        controller_bridge = cmb.encode_map_control_controller_bridge({
            0x9000: 2,
            0x8000: 1,
        })

        self.assertEqual(
            province_bridge,
            "province_array=0x4000;province_count=20;"
            "province_stride=8;controller_ptr_offset=0",
        )
        self.assertEqual(controller_bridge, "1:0x8000;2:0x9000")


if __name__ == "__main__":
    unittest.main()
