"""Unit tests for derive.adjacency raster resolution, validation, and cache.

All tests run against SYNTHETIC mini-rasters written into a temp assets dir
(no real map assets needed), with the cache dir patched into the temp dir so
nothing leaks into the package's real ``_cache``.

Covers the fail-closed contracts:
  * requested raster resolution missing -> typed refusal, never a silent
    substitution of the other resolution;
  * raster/meta validation (dimensions, land-id completeness) -> typed
    ProvinceRasterMismatch refusals;
  * cache keyed by assets dir + content fingerprint -> a different assets dir
    or a regenerated raster never serves a stale/foreign cache; legacy flat
    cache files are treated as a miss and rebuilt.

Run: python -m unittest derive.tests.test_adjacency
"""

from __future__ import annotations

import json
import os
import sys
import tempfile
import unittest
from unittest import mock

import numpy as np

_HERE = os.path.dirname(os.path.abspath(__file__))
_REPO = os.path.normpath(os.path.join(_HERE, os.pardir, os.pardir))
if _REPO not in sys.path:
    sys.path.insert(0, _REPO)

from derive import adjacency  # noqa: E402


# ---------------------------------------------------------------------------
# synthetic asset helpers
# ---------------------------------------------------------------------------
# Map A: four 2x2 blocks; id 4 is sea (absent from province_to_state).
_A_FULL = [[1, 1, 2, 2],
           [1, 1, 2, 2],
           [3, 3, 4, 4],
           [3, 3, 4, 4]]
_A_HALF = [[1, 2],
           [3, 4]]
_A_P2S = {"1": 10, "2": 10, "3": 20}
_A_ADJ = {1: [2, 3], 2: [1], 3: [1]}

# Map B: different topology (all three land provinces mutually adjacent).
_B_FULL = [[1, 1, 1, 1],
           [1, 1, 1, 1],
           [2, 2, 3, 3],
           [2, 2, 3, 3]]
_B_HALF = [[1, 1],
           [2, 3]]
_B_P2S = {"1": 10, "2": 20, "3": 20}
_B_ADJ = {1: [2, 3], 2: [1, 3], 3: [1, 2]}


def _write_id_png(path, rows):
    """Write an id grid as an RGB PNG using the id = R + G*256 + B*65536 encoding."""
    from PIL import Image

    ids = np.asarray(rows, dtype=np.uint32)
    rgb = np.zeros(ids.shape + (3,), dtype=np.uint8)
    rgb[:, :, 0] = ids & 0xFF
    rgb[:, :, 1] = (ids >> 8) & 0xFF
    rgb[:, :, 2] = (ids >> 16) & 0xFF
    Image.fromarray(rgb, "RGB").save(path)


def _write_assets(assets_dir, full_rows, half_rows, p2s, meta=None):
    """Populate ``assets_dir`` with rasters + mapping + meta (any part optional)."""
    os.makedirs(assets_dir, exist_ok=True)
    if full_rows is not None:
        _write_id_png(os.path.join(assets_dir, "province_ids.png"), full_rows)
    if half_rows is not None:
        _write_id_png(os.path.join(assets_dir, "province_ids_half.png"),
                      half_rows)
    if p2s is not None:
        with open(os.path.join(assets_dir, "province_to_state.json"), "w",
                  encoding="utf-8") as fh:
            json.dump(p2s, fh)
    if meta is None and full_rows is not None:
        fh_arr = np.asarray(full_rows)
        hh_arr = np.asarray(half_rows if half_rows is not None else full_rows)
        meta = {
            "source_width": int(fh_arr.shape[1]),
            "source_height": int(fh_arr.shape[0]),
            "downscaled_display": {"width": int(hh_arr.shape[1]),
                                   "height": int(hh_arr.shape[0])},
        }
    if meta is not None:
        with open(os.path.join(assets_dir, "province_ids_meta.json"), "w",
                  encoding="utf-8") as fh:
            json.dump(meta, fh)


class TestAdjacencySynthetic(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = self._tmp.name
        cache_dir = os.path.join(self.root, "_cache")
        os.makedirs(cache_dir, exist_ok=True)
        patcher = mock.patch.object(adjacency, "_cache_dir",
                                    return_value=cache_dir)
        patcher.start()
        self.addCleanup(patcher.stop)

    def _assets(self, name, full_rows, half_rows, p2s, meta=None):
        d = os.path.join(self.root, name)
        _write_assets(d, full_rows, half_rows, p2s, meta=meta)
        return d

    # -- build correctness -------------------------------------------------
    def test_build_full_and_half(self):
        a = self._assets("a", _A_FULL, _A_HALF, _A_P2S)
        self.assertEqual(adjacency.build_adjacency(a, half_res=False), _A_ADJ)
        self.assertEqual(adjacency.build_adjacency(a, half_res=True), _A_ADJ)

    # -- fail-closed raster resolution ------------------------------------
    def test_missing_full_res_refuses_even_if_half_exists(self):
        a = self._assets("half_only", None, _A_HALF, _A_P2S,
                         meta={"source_width": 4, "source_height": 4,
                               "downscaled_display": {"width": 2,
                                                      "height": 2}})
        with self.assertRaises(FileNotFoundError) as ctx:
            adjacency.build_adjacency(a, half_res=False)
        self.assertIn("refusing to substitute", str(ctx.exception))
        self.assertIn("exists", str(ctx.exception))

    def test_missing_half_res_refuses_even_if_full_exists(self):
        a = self._assets("full_only", _A_FULL, None, _A_P2S)
        with self.assertRaises(FileNotFoundError) as ctx:
            adjacency.build_adjacency(a, half_res=True)
        self.assertIn("refusing to substitute", str(ctx.exception))

    def test_both_rasters_missing_names_both_candidates(self):
        a = self._assets("no_rasters", None, None, _A_P2S,
                         meta={"source_width": 4, "source_height": 4,
                               "downscaled_display": {"width": 2,
                                                      "height": 2}})
        with self.assertRaises(FileNotFoundError) as ctx:
            adjacency.build_adjacency(a, half_res=True)
        msg = str(ctx.exception)
        self.assertIn("province_ids_half.png", msg)
        self.assertIn("province_ids.png", msg)
        self.assertIn("also missing", msg)

    # -- fail-closed meta validation ---------------------------------------
    def test_missing_meta_refuses(self):
        a = self._assets("no_meta", _A_FULL, _A_HALF, _A_P2S)
        os.remove(os.path.join(a, "province_ids_meta.json"))
        with self.assertRaises(adjacency.ProvinceRasterMismatch):
            adjacency.build_adjacency(a, half_res=False)

    def test_dimension_mismatch_refuses(self):
        a = self._assets("bad_dims", _A_FULL, _A_HALF, _A_P2S,
                         meta={"source_width": 5632, "source_height": 2048,
                               "downscaled_display": {"width": 2816,
                                                      "height": 1024}})
        with self.assertRaises(adjacency.ProvinceRasterMismatch) as ctx:
            adjacency.build_adjacency(a, half_res=False)
        self.assertIn("shape", str(ctx.exception))

    def test_missing_land_id_refuses(self):
        p2s = dict(_A_P2S)
        p2s["99"] = 30  # land id absent from the raster
        a = self._assets("lost_prov", _A_FULL, _A_HALF, p2s)
        with self.assertRaises(adjacency.ProvinceRasterMismatch) as ctx:
            adjacency.build_adjacency(a, half_res=False)
        self.assertIn("99", str(ctx.exception))

    # -- cache keying + staleness ------------------------------------------
    def test_cache_keyed_by_assets_dir(self):
        a = self._assets("map_a", _A_FULL, _A_HALF, _A_P2S)
        b = self._assets("map_b", _B_FULL, _B_HALF, _B_P2S)
        self.assertEqual(adjacency.load_or_build_adjacency(a, half_res=False),
                         _A_ADJ)
        # a different assets dir must NOT be served map A's cache
        self.assertEqual(adjacency.load_or_build_adjacency(b, half_res=False),
                         _B_ADJ)
        # and A's cache still round-trips correctly afterwards
        self.assertEqual(adjacency.load_or_build_adjacency(a, half_res=False),
                         _A_ADJ)
        self.assertNotEqual(adjacency._cache_path(a, False),
                            adjacency._cache_path(b, False))

    def test_cache_path_normalizes_dir_spelling(self):
        a = self._assets("map_a2", _A_FULL, _A_HALF, _A_P2S)
        relative = os.path.relpath(a)
        self.assertEqual(adjacency._cache_path(a, True),
                         adjacency._cache_path(relative, True))

    def test_stale_cache_rebuilt_when_raster_changes(self):
        a = self._assets("mutating", _A_FULL, _A_HALF, _A_P2S)
        self.assertEqual(adjacency.load_or_build_adjacency(a, half_res=False),
                         _A_ADJ)
        # regenerate the raster in place (same dir, different map content)
        _write_id_png(os.path.join(a, "province_ids.png"), _B_FULL)
        with open(os.path.join(a, "province_to_state.json"), "w",
                  encoding="utf-8") as fh:
            json.dump(_B_P2S, fh)
        self.assertEqual(adjacency.load_or_build_adjacency(a, half_res=False),
                         _B_ADJ)

    def test_legacy_flat_cache_is_a_miss(self):
        a = self._assets("legacy", _A_FULL, _A_HALF, _A_P2S)
        cache = adjacency._cache_path(a, False)
        with open(cache, "w", encoding="utf-8") as fh:
            json.dump({"1": [999]}, fh)  # old flat format, wrong data
        self.assertEqual(adjacency.load_or_build_adjacency(a, half_res=False),
                         _A_ADJ)
        with open(cache, "r", encoding="utf-8") as fh:
            raw = json.load(fh)
        self.assertEqual(raw.get("schema"), adjacency._CACHE_SCHEMA)
        self.assertIn("source_fingerprint", raw)

    def test_use_cache_false_bypasses_cache(self):
        a = self._assets("nocache", _A_FULL, _A_HALF, _A_P2S)
        self.assertEqual(
            adjacency.load_or_build_adjacency(a, half_res=False,
                                              use_cache=False),
            _A_ADJ)
        self.assertFalse(os.path.exists(adjacency._cache_path(a, False)))


if __name__ == "__main__":
    unittest.main()
