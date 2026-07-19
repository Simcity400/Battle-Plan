"""Tests for derive/asset_provenance.py.

Two layers:
  * Hash gate (always on): the frozen map_pipeline tables on disk must match
    the committed FROZEN_TABLE_HASHES record. The tables are unregenerable
    (their generators were deleted when the map checkpoint moved to Rust), so
    any drift is silent corruption or an undeclared edit and must fail loudly.
  * Bundle parity gate (opt-in, BATTLE_PLAN_BUNDLE_DIR): compares the frozen tables
    to a locally generated Rust map bundle. SKIPS with an explicit reason when
    the env var is unset -- never a silent pass.

Pure-unit tests cover the field-aware comparison logic with synthetic data.
"""

from __future__ import annotations

import os
import sys
import unittest

_HERE = os.path.dirname(os.path.abspath(__file__))
_REPO = os.path.normpath(os.path.join(_HERE, os.pardir, os.pardir))
if _REPO not in sys.path:
    sys.path.insert(0, _REPO)

from derive import asset_provenance as ap  # noqa: E402


class TestFrozenTableHashes(unittest.TestCase):
    def test_manifest_covers_every_frozen_table(self):
        self.assertEqual(set(ap.FROZEN_TABLE_HASHES), set(ap.FROZEN_TABLES))

    def test_source_provenance_is_honestly_unrecorded(self):
        # The original game version / mod identity were lost with the deleted
        # generator; the record must say so, never invent a version.
        self.assertEqual(ap.SOURCE_PROVENANCE["game_version"], "UNRECORDED")
        self.assertFalse(ap.SOURCE_PROVENANCE["regenerable"])

    def test_frozen_tables_match_committed_hashes(self):
        problems = ap.verify_frozen_tables()
        self.assertEqual(
            problems, [],
            "frozen map_pipeline tables drifted from their committed hashes "
            "(unregenerable data -- restore it, or re-freeze deliberately and "
            "update FROZEN_TABLE_HASHES): " + "; ".join(problems))

    def test_fnv1a64_matches_rust_reference(self):
        # FNV-1a 64 test vectors (same algorithm as map_viewer's fnv1a64).
        self.assertEqual(ap.fnv1a64_bytes(b""), "cbf29ce484222325")
        self.assertEqual(ap.fnv1a64_bytes(b"a"), "af63dc4c8601ec8c")


class TestComparisonLogic(unittest.TestCase):
    def test_province_to_state_exact(self):
        frozen = {"1": 10, "2": 20}
        self.assertEqual(ap.compare_province_to_state(frozen, dict(frozen)), [])
        diffs = ap.compare_province_to_state(frozen, {"1": 10, "2": 21, "3": 30})
        self.assertEqual(len(diffs), 2)  # extra province + remap
        self.assertTrue(any("only in bundle" in d for d in diffs))
        self.assertTrue(any("different" in d for d in diffs))

    def test_state_meta_shared_fields_and_impassable_tolerance(self):
        frozen = {"1": {"name_key": "STATE_1", "provinces": [1, 2],
                        "victory_points": {"1": 5}, "vp_total": 5,
                        "state_category": "rural", "manpower": 1000}}
        # Bundle side: identical shared fields + the known 'impassable' extra.
        bundle = {"1": dict(frozen["1"], impassable=False)}
        self.assertEqual(ap.compare_state_meta(frozen, bundle), [])

    def test_state_meta_unknown_extra_field_is_schema_drift(self):
        frozen = {"1": {"name_key": "S", "provinces": [], "victory_points": {},
                        "vp_total": 0, "state_category": None, "manpower": 0}}
        bundle = {"1": dict(frozen["1"], impassable=True, surprise=1)}
        diffs = ap.compare_state_meta(frozen, bundle)
        self.assertEqual(len(diffs), 1)
        self.assertIn("surprise", diffs[0])

    def test_state_meta_shared_field_difference_fails(self):
        frozen = {"1": {"name_key": "S", "provinces": [1], "victory_points": {},
                        "vp_total": 0, "state_category": None, "manpower": 5}}
        bundle = {"1": {"name_key": "S", "provinces": [1], "victory_points": {},
                        "vp_total": 0, "state_category": None, "manpower": 6,
                        "impassable": False}}
        diffs = ap.compare_state_meta(frozen, bundle)
        self.assertEqual(len(diffs), 1)
        self.assertIn("manpower", diffs[0])

    def test_victory_points_key_normalization(self):
        # int-keyed vs str-keyed victory_points maps must compare equal.
        frozen = {"1": {"name_key": "S", "provinces": [], "vp_total": 3,
                        "victory_points": {1: 3}, "state_category": None,
                        "manpower": 0}}
        bundle = {"1": {"name_key": "S", "provinces": [], "vp_total": 3,
                        "victory_points": {"1": 3}, "state_category": None,
                        "manpower": 0, "impassable": False}}
        self.assertEqual(ap.compare_state_meta(frozen, bundle), [])


class TestBundleParityGate(unittest.TestCase):
    """Opt-in staleness gate vs a locally generated Rust map bundle."""

    def setUp(self):
        self.bundle_dir = ap.bundle_dir_from_env()
        if self.bundle_dir is None:
            self.skipTest(
                "BATTLE_PLAN_BUNDLE_DIR unset: bundle parity gate is opt-in (needs a "
                "locally generated map bundle); frozen-table staleness NOT "
                "verified in this run")
        if not os.path.isdir(self.bundle_dir):
            self.fail(f"BATTLE_PLAN_BUNDLE_DIR={self.bundle_dir!r} is not a directory")

    def test_tables_match_bundle(self):
        report = ap.check_bundle_parity(self.bundle_dir)
        self.assertEqual(
            report["status"], "ok",
            "frozen map tables drifted from the generated bundle (the frozen "
            "assets are a vanilla snapshot of an UNRECORDED game version; a "
            "diff after a the game patch indicates data-version drift requiring a "
            "deliberate re-freeze, not necessarily a code bug):\n  "
            + "\n  ".join(report["differences"]))

    def test_raster_planes_match_bundle(self):
        report = ap.check_raster_parity(self.bundle_dir)
        if report["status"] == "skipped":
            self.skipTest("raster parity skipped: " + report["reason"])
        self.assertEqual(
            report["status"], "ok",
            "province id plane drifted between the frozen PNG raster and the "
            "bundle r32 plane:\n  " + "\n  ".join(report["differences"]))


if __name__ == "__main__":
    unittest.main(verbosity=2)
