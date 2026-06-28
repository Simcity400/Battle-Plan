from contextlib import contextmanager
import os
from pathlib import Path
import sys
import struct
import types
from types import SimpleNamespace
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))

from live_capture import collector_country_bridge as ccb  # noqa: E402
from live_capture.collector_country_bridge import (  # noqa: E402
    _encoded_preflight,
    _load_preflight_cache,
    _write_preflight_cache,
    country_object_bridge_rows_from_country_database_root,
    country_object_bridge_rows_from_vtable_scan,
    country_object_bridge_rows_from_bridge,
    country_object_bridge_rows_from_country_array,
    locate_country_array_from_object_rows,
    resolve_country_object_bridge_preflight,
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
    count = 3

    def tag(self, tag_db_id):
        return {1: "GER", 2: "ENG"}.get(tag_db_id)


class FakeMemoryProc:
    module_base = 0x70000000

    def __init__(self):
        self.base = 0x8000
        self.blob = bytearray(0x5000)
        self.country_vtable = self.module_base + 0x27C0E80
        self.country_database = 0x8200
        self.object_array = 0x8500
        self.rec0 = 0xA000
        self.ger_obj = 0x9000
        self.eng_obj = 0x9100
        self._write_country_object(self.ger_obj, 1, "Germany")
        self._write_country_object(self.eng_obj, 2, "Britain")
        self._write_u64(self.country_database + 0x80, self.object_array)
        self._write_u64(self.object_array + 1 * 8, self.ger_obj)
        self._write_u64(self.object_array + 2 * 8, self.eng_obj)
        for i in range(24):
            self._write_u16(self.rec0 + i * 0x120 + 0x5D, i)
        self._write_u64(self.rec0 + 0x08, self.ger_obj)
        self._write_u64(self.rec0 + 0x120 + 0x08, self.eng_obj)

    def _offset(self, addr):
        off = addr - self.base
        if off < 0 or off >= len(self.blob):
            raise OSError("address outside fake region")
        return off

    def _write(self, addr, data):
        off = self._offset(addr)
        self.blob[off:off + len(data)] = data

    def _write_u16(self, addr, value):
        self._write(addr, struct.pack("<H", value))

    def _write_u32(self, addr, value):
        self._write(addr, struct.pack("<I", value))

    def _write_u64(self, addr, value):
        self._write(addr, struct.pack("<Q", value))

    def _write_country_object(self, addr, tag_db_id, name):
        self._write_u64(addr, self.country_vtable)
        self._write_u32(addr + 0x08, tag_db_id)
        raw = name.encode("latin-1")
        self._write(addr + 0x10, raw + b"\x00" * (16 - len(raw)))
        self._write_u64(addr + 0x20, len(raw))
        self._write_u64(addr + 0x28, 15)

    def iter_regions(self, lo, hi, want_exec=None):
        start = max(lo, self.base)
        stop = min(hi, self.base + len(self.blob))
        if stop > start:
            yield start, stop - start

    def try_read(self, addr, size):
        try:
            off = self._offset(addr)
        except OSError:
            return b""
        return bytes(self.blob[off:off + min(size, len(self.blob) - off)])

    def read(self, addr, size):
        data = self.try_read(addr, size)
        if len(data) != size:
            raise OSError("short fake read")
        return data

    def u16(self, addr):
        return struct.unpack("<H", self.read(addr, 2))[0]

    def u32(self, addr):
        return struct.unpack("<I", self.read(addr, 4))[0]

    def u64(self, addr):
        if addr == self.module_base + 0x33063F8:
            return self.country_database
        return struct.unpack("<Q", self.read(addr, 8))[0]


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


class FakePreflightProc(FakeMemoryProc):
    def __init__(self):
        super().__init__()
        self.table_base = 0xB000
        self.tags = ["---", "GER", "ENG", "SOV", "SWE", "FRA", "LUX", "BEL",
                     "HOL", "CZE", "POL"] + [f"T{i:02d}" for i in range(40)]
        self._write_u64(self.country_database, self.table_base)
        self._write_u32(self.country_database + 8, len(self.tags) + 5)
        self._write_u32(self.country_database + 12, len(self.tags))


@contextmanager
def patched_preflight_environment(
    proc,
    *,
    identity=None,
    resolve_cached=None,
    locate=None,
    tag_table_module=None,
    bridge_resolve=None,
):
    def forbidden_cold_scan(*_args, **_kwargs):
        raise AssertionError("unexpected cold scan")

    identity = identity or SimpleNamespace(
        pid=123,
        create_time_100ns=456,
        exe_sha256="b" * 64,
        module_base=getattr(proc, "module_base", None),
    )

    class ProcessContext:
        def __enter__(self):
            return proc

        def __exit__(self, exc_type, exc, tb):
            return False

    winmem_module = types.ModuleType("winmem")
    winmem_module.Process = lambda _pid, _name: ProcessContext()

    locate_module = types.ModuleType("locate")
    locate_module.resolve_cached = resolve_cached or (lambda _proc, _token: None)
    locate_module.locate = locate or forbidden_cold_scan

    tt_source = tag_table_module or FakeTagTableModule
    tag_table_mod = types.ModuleType("live_layers.tag_table")
    tag_table_mod.STRIDE = getattr(tt_source, "STRIDE", 0x20)
    tag_table_mod.TagTable = tt_source.TagTable

    class CountryIndexBridge:
        resolve = staticmethod(bridge_resolve or forbidden_cold_scan)

    bridge_mod = types.ModuleType("live_layers.country_index_bridge")
    bridge_mod.CountryIndexBridge = CountryIndexBridge

    live_layers_mod = types.ModuleType("live_layers")
    live_layers_mod.tag_table = tag_table_mod
    live_layers_mod.country_index_bridge = bridge_mod

    names = [
        "winmem",
        "locate",
        "live_layers",
        "live_layers.tag_table",
        "live_layers.country_index_bridge",
    ]
    old_modules = {name: sys.modules.get(name) for name in names}
    old_identify = ccb._process_identity.identify
    try:
        sys.modules["winmem"] = winmem_module
        sys.modules["locate"] = locate_module
        sys.modules["live_layers"] = live_layers_mod
        sys.modules["live_layers.tag_table"] = tag_table_mod
        sys.modules["live_layers.country_index_bridge"] = bridge_mod
        ccb._process_identity.identify = lambda _proc: identity
        yield identity
    finally:
        ccb._process_identity.identify = old_identify
        for name in names:
            old = old_modules[name]
            if old is None:
                sys.modules.pop(name, None)
            else:
                sys.modules[name] = old


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

    def test_rows_from_vtable_scan_use_object_tag_db_id(self):
        proc = FakeMemoryProc()
        rows = country_object_bridge_rows_from_vtable_scan(
            proc,
            FakeTable(),
            min_rows=2,
            scan_range=(proc.base, proc.base + len(proc.blob)),
        )

        self.assertEqual(
            rows,
            [
                {"tag_db_id": 1, "tag": "GER", "object_ptr": proc.ger_obj},
                {"tag_db_id": 2, "tag": "ENG", "object_ptr": proc.eng_obj},
            ],
        )

    def test_rows_from_country_database_root_use_tag_indexed_array(self):
        proc = FakeMemoryProc()
        rows = country_object_bridge_rows_from_country_database_root(
            proc,
            FakeTable(),
            min_rows=2,
        )

        self.assertEqual(
            rows,
            [
                {"tag_db_id": 1, "tag": "GER", "object_ptr": proc.ger_obj},
                {"tag_db_id": 2, "tag": "ENG", "object_ptr": proc.eng_obj},
            ],
        )

    def test_locate_country_array_from_object_rows_recovers_record_rows(self):
        proc = FakeMemoryProc()
        object_rows = country_object_bridge_rows_from_vtable_scan(
            proc,
            FakeTable(),
            min_rows=2,
            scan_range=(proc.base, proc.base + len(proc.blob)),
        )
        located = locate_country_array_from_object_rows(
            proc,
            FakeTable(),
            object_rows,
            min_rows=2,
            scan_range=(proc.base, proc.base + len(proc.blob)),
        )

        self.assertEqual(located, (proc.rec0, proc.rec0, 24))
        record_rows = country_object_bridge_rows_from_country_array(
            proc,
            FakeTable(),
            located,
        )
        self.assertEqual(encode_country_record_bridge(record_rows), "1:0xA000;2:0xA120")

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

    def test_preflight_cache_hit_is_bounded(self):
        proc = FakePreflightProc()
        with tempfile.TemporaryDirectory() as td:
            path = Path(td) / "country_array.json"
            with patched_preflight_environment(
                proc,
                resolve_cached=lambda _proc, _token: (proc.rec0, 24),
            ) as identity:
                self.assertTrue(
                    _write_preflight_cache(path, identity, (proc.rec0, proc.rec0, 227))
                )
                old_scan = ccb._scan_aligned_qword_hits
                ccb._scan_aligned_qword_hits = lambda *_args, **_kwargs: (
                    _ for _ in ()
                ).throw(AssertionError("unexpected reverse scan"))
                try:
                    result = resolve_country_object_bridge_preflight(
                        123,
                        min_rows=2,
                        cache_path=path,
                    )
                finally:
                    ccb._scan_aligned_qword_hits = old_scan

        self.assertTrue(result.usable)
        self.assertEqual(result.diagnostics["source"], "live_country_array_cache")
        self.assertEqual(result.diagnostics["cache"]["status"], "hit")
        self.assertFalse(result.diagnostics["allow_cold_scan"])

    def test_preflight_cache_miss_returns_root_rows_without_cold_scan(self):
        proc = FakePreflightProc()
        with patched_preflight_environment(proc):
            old_scan = ccb._scan_aligned_qword_hits
            ccb._scan_aligned_qword_hits = lambda *_args, **_kwargs: (
                _ for _ in ()
            ).throw(AssertionError("unexpected reverse scan"))
            try:
                result = resolve_country_object_bridge_preflight(123, min_rows=2)
            finally:
                ccb._scan_aligned_qword_hits = old_scan

        self.assertTrue(result.usable)
        self.assertEqual(result.diagnostics["source"], "country_database_root")
        self.assertEqual(result.record_encoded, None)
        self.assertFalse(result.diagnostics["allow_cold_scan"])

    def test_preflight_stale_cache_and_insufficient_roots_is_bounded(self):
        proc = FakePreflightProc()
        with tempfile.TemporaryDirectory() as td:
            path = Path(td) / "country_array.json"
            with patched_preflight_environment(
                proc,
                resolve_cached=lambda _proc, _token: None,
            ) as identity:
                self.assertTrue(
                    _write_preflight_cache(path, identity, (proc.rec0, proc.rec0, 227))
                )
                result = resolve_country_object_bridge_preflight(
                    123,
                    min_rows=3,
                    cache_path=path,
                )

        self.assertFalse(result.usable)
        self.assertEqual(result.status, "cold_scan_disabled")
        self.assertEqual(result.row_count, 0)
        self.assertEqual(result.diagnostics["cache"]["status"], "stale")

    def test_preflight_cold_scan_is_explicit(self):
        proc = FakePreflightProc()
        scan_hits = []

        def fake_scan(_proc, needle, *, scan_range=None):
            scan_hits.append((needle, scan_range))
            if needle == proc.ger_obj:
                yield proc.rec0 + 0x08
            elif needle == proc.eng_obj:
                yield proc.rec0 + 0x120 + 0x08

        with patched_preflight_environment(proc):
            old_scan = ccb._scan_aligned_qword_hits
            ccb._scan_aligned_qword_hits = fake_scan
            try:
                result = resolve_country_object_bridge_preflight(
                    123,
                    min_rows=2,
                    allow_cold_scan=True,
                )
            finally:
                ccb._scan_aligned_qword_hits = old_scan

        self.assertTrue(scan_hits)
        self.assertTrue(result.usable)
        self.assertEqual(result.diagnostics["source"], "country_database_root_record_scan")
        self.assertTrue(result.diagnostics["allow_cold_scan"])
        self.assertEqual(result.record_encoded, "1:0xA000;2:0xA120")

    def test_preflight_allow_cold_scan_can_use_locate_after_root_miss(self):
        proc = FakePreflightProc()
        locate_calls = []

        def fake_locate(_proc, *, scan_range=None):
            locate_calls.append(scan_range)
            return proc.rec0, proc.rec0, 24

        with patched_preflight_environment(proc, locate=fake_locate):
            old_root = ccb.country_object_bridge_rows_from_country_database_root
            ccb.country_object_bridge_rows_from_country_database_root = (
                lambda *_args, **_kwargs: []
            )
            try:
                result = resolve_country_object_bridge_preflight(
                    123,
                    min_rows=2,
                    allow_cold_scan=True,
                )
            finally:
                ccb.country_object_bridge_rows_from_country_database_root = old_root

        self.assertEqual(locate_calls, [None])
        self.assertTrue(result.usable)
        self.assertEqual(result.diagnostics["source"], "live_country_array")

    def test_preflight_root_tag_table_unavailable_is_bounded(self):
        proc = SimpleNamespace(module_base=0)

        class NoHeapTagTableModule:
            class TagTable:
                @staticmethod
                def locate(_proc):
                    raise AssertionError("unexpected tag table heap scan")

        with patched_preflight_environment(
            proc,
            tag_table_module=NoHeapTagTableModule,
        ):
            result = resolve_country_object_bridge_preflight(123, min_rows=2)

        self.assertFalse(result.usable)
        self.assertEqual(result.status, "tag_table_unavailable")
        self.assertEqual(result.diagnostics["source"], "country_database_root")

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
