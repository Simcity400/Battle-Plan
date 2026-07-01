"""Tests for models/air_superiority.py.

stdlib ``unittest`` (repo convention; runs with
``python -m unittest models.tests.test_air_superiority``) and pytest-discoverable.
Mixes pure-unit tests on hand-built fixtures (no installed content) with optional
integration tests over the real save_oracle/output dir (skipped when absent).

Invariants under test mirror the task's VALIDATE clause:
  * per-wing EAS == effective_planes * air_superiority_stat (the save oracle);
  * a region with ONLY friendly fighters -> ~100% for that side;
  * a contested region -> strictly between 0 and 100;
  * stat weighting beats the raw effective_planes proxy where stats differ
    (heavy_fighter 1.25, strat_bomber 0.01), yet AGREES on single-stat regions
    (so leader ordering matches the proxy when every plane has stat 1);
  * no fabrication: idle wings (no region) score None, never a guess;
  * a zero-score region -> percent None, never divide-by-zero.
"""
import json
import os
import sys
import tempfile
import unittest

sys.path.insert(
    0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
)

from models import air_superiority as asup  # noqa: E402

_REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
_OUT = os.path.join(os.path.dirname(_REPO), "extractor", "save_oracle", "output")


def _wing(wid, owner, pdef, region, eff, count=None, mission="x"):
    return {
        "id": f"{wid}:69",
        "owner": owner,
        "plane_definition": pdef,
        "strategic_region": region,
        "effective_planes": eff,
        "count": count if count is not None else eff,
        "mission": mission,
    }


# ---------------------------------------------------------------------------
# Unit: per-plane stat resolution
# ---------------------------------------------------------------------------
class TestPlaneStat(unittest.TestCase):
    def test_vanilla_calibrated_stats(self):
        # The save-confirmed ratios.
        self.assertEqual(asup.plane_air_sup_stat("fighter"), 1.0)
        self.assertEqual(asup.plane_air_sup_stat("cv_fighter"), 1.0)
        self.assertEqual(asup.plane_air_sup_stat("heavy_fighter"), 1.25)
        self.assertEqual(asup.plane_air_sup_stat("strat_bomber"), 0.01)
        self.assertEqual(asup.plane_air_sup_stat("cas"), 1.0)
        self.assertEqual(asup.plane_air_sup_stat("tac_bomber"), 1.0)

    def test_override_wins(self):
        ov = {"fighter": 2.0}
        self.assertEqual(asup.plane_air_sup_stat("fighter", ov), 2.0)
        # class not in override -> falls back to table
        self.assertEqual(asup.plane_air_sup_stat("heavy_fighter", ov), 1.25)

    def test_unknown_plane_falls_back(self):
        self.assertEqual(
            asup.plane_air_sup_stat("some_modded_thing"), asup.DEFAULT_AIR_SUP_STAT
        )

    def test_none_plane(self):
        self.assertEqual(
            asup.plane_air_sup_stat(None), asup.DEFAULT_AIR_SUP_STAT
        )


# ---------------------------------------------------------------------------
# Unit: per-wing score (EAS == effective_planes * stat)
# ---------------------------------------------------------------------------
class TestWingScore(unittest.TestCase):
    def test_fighter_eas_equals_planes(self):
        # save oracle: fighter EAS/effective_planes == 1.000
        self.assertEqual(
            asup.wing_air_sup_score(_wing(1, "GER", "fighter", 10, 43)), 43.0
        )

    def test_heavy_fighter_125(self):
        # heavy_fighter measured ratio 1.250
        self.assertAlmostEqual(
            asup.wing_air_sup_score(_wing(2, "GER", "heavy_fighter", 10, 40)),
            50.0,
        )

    def test_strat_bomber_near_zero(self):
        # strat_bomber measured ratio 0.010
        self.assertAlmostEqual(
            asup.wing_air_sup_score(_wing(3, "USA", "strat_bomber", 10, 100)),
            1.0,
        )

    def test_idle_wing_scores_none(self):
        # No strategic_region -> not in the pool -> None (no fabrication).
        self.assertIsNone(
            asup.wing_air_sup_score(_wing(4, "GER", "fighter", None, 50))
        )

    def test_missing_effplanes_falls_back_to_count(self):
        w = _wing(5, "GER", "fighter", 10, None, count=30)
        self.assertEqual(asup.wing_air_sup_score(w), 30.0)


# ---------------------------------------------------------------------------
# Unit: region aggregation + displayed percent
# ---------------------------------------------------------------------------
class TestRegionPercent(unittest.TestCase):
    def test_friendly_only_is_100(self):
        wings = [
            _wing(1, "GER", "fighter", 1, 50),
            _wing(2, "GER", "fighter", 1, 30),
        ]
        regions = asup.aggregate_regions(wings)
        r = regions[1]
        self.assertEqual(r.total_score, 80.0)
        self.assertEqual(r.leader, "GER")
        self.assertEqual(r.percent_for(["GER"]), 100.0)

    def test_contested_is_strictly_between(self):
        wings = [
            _wing(1, "GER", "fighter", 7, 80),
            _wing(2, "ENG", "fighter", 7, 20),
        ]
        regions = asup.aggregate_regions(wings)
        r = regions[7]
        pct = r.percent_for(["GER"])
        self.assertTrue(0.0 < pct < 100.0)
        self.assertAlmostEqual(pct, 80.0)  # 80/(80+20)
        # The opposing side is the complement.
        self.assertAlmostEqual(r.percent_for(["ENG"]), 20.0)

    def test_worked_example_region_164(self):
        # The README/Codex worked example: Allies vs JAP+SOV. All stat 1.
        owners = {"USA": 1387, "JAP": 261, "ENG": 193, "SOV": 26, "FRA": 25, "MEX": 0}
        wings = [
            _wing(i, t, "fighter", 164, v) for i, (t, v) in enumerate(owners.items())
        ]
        regions = asup.aggregate_regions(wings)
        r = regions[164]
        allies = ["USA", "ENG", "FRA", "MEX"]
        pct = r.percent_for(allies)
        # 1605 / (1605 + 287) = 84.83%
        self.assertAlmostEqual(pct, 100.0 * 1605 / 1892, places=4)

    def test_zero_score_region_percent_none(self):
        # A region whose only wing is a stat-0 rocket -> total 0 -> None.
        wings = [_wing(1, "GER", "rocket", 9, 100)]
        regions = asup.aggregate_regions(wings)
        r = regions[9]
        self.assertEqual(r.total_score, 0.0)
        self.assertIsNone(r.percent_for(["GER"]))

    def test_stat_weighting_changes_contest_vs_proxy(self):
        # Region where the raw effective_planes PROXY would say ENG leads, but the
        # stat-weighted MODEL says GER leads (ENG flies strat bombers, stat 0.01).
        wings = [
            _wing(1, "GER", "fighter", 5, 100),        # score 100
            _wing(2, "ENG", "strat_bomber", 5, 500),   # score 5  (proxy: 500)
        ]
        regions = asup.aggregate_regions(wings)
        r = regions[5]
        # Proxy (effective_planes) would crown ENG (500 > 100); model crowns GER.
        self.assertEqual(r.leader, "GER")
        self.assertGreater(
            r.by_owner["ENG"]["effective_planes"],
            r.by_owner["GER"]["effective_planes"],
        )  # proxy ordering is the opposite
        self.assertGreater(
            r.by_owner["GER"]["score"], r.by_owner["ENG"]["score"]
        )  # stat-weighted ordering

    def test_single_stat_region_matches_proxy_ordering(self):
        # When every plane has stat 1, model leader == proxy (effective_planes) leader.
        wings = [
            _wing(1, "GER", "fighter", 3, 60),
            _wing(2, "ITA", "cas", 3, 40),
            _wing(3, "ENG", "tac_bomber", 3, 90),
        ]
        regions = asup.aggregate_regions(wings)
        r = regions[3]
        proxy_leader = max(
            r.by_owner, key=lambda t: r.by_owner[t]["effective_planes"]
        )
        self.assertEqual(r.leader, proxy_leader)  # both -> ENG


# ---------------------------------------------------------------------------
# Unit: defs parsing (synthetic equipment dir)
# ---------------------------------------------------------------------------
class TestDefsParsing(unittest.TestCase):
    def test_parse_air_superiority_from_equipment_dir(self):
        with tempfile.TemporaryDirectory() as d:
            with open(os.path.join(d, "single_engine_airframe.txt"), "w") as fh:
                fh.write(
                    "equipments = {\n"
                    "  small_plane_airframe = {\n"
                    "    air_superiority = 1\n"
                    "  }\n"
                    "}\n"
                )
            with open(os.path.join(d, "quad_engine_airframe.txt"), "w") as fh:
                fh.write(
                    "equipments = {\n"
                    "  large_plane_airframe = {\n"
                    "    air_superiority = 0.01\n"
                    "  }\n"
                    "  # commented_out_airframe = {\n"
                    "  #   air_superiority = 99\n"
                    "  # }\n"
                    "}\n"
                )
            stats = asup.load_air_superiority_stats(d)
            self.assertEqual(stats.get("fighter"), 1.0)
            self.assertEqual(stats.get("strat_bomber"), 0.01)
            self.assertNotIn(99.0, stats.values())  # commented stat ignored

    def test_missing_dir_returns_empty(self):
        self.assertEqual(asup.load_air_superiority_stats("/no/such/dir"), {})
        self.assertEqual(asup.load_air_superiority_stats(""), {})


# ---------------------------------------------------------------------------
# Unit: full compute over a synthetic save_oracle dir
# ---------------------------------------------------------------------------
class TestCompute(unittest.TestCase):
    def test_compute_over_synthetic_dir(self):
        with tempfile.TemporaryDirectory() as d:
            oob = {
                "meta": {"save_date": "1940.1.1.12"},
                "air_wings": [
                    _wing(1, "GER", "fighter", 1, 50),
                    _wing(2, "ENG", "fighter", 1, 50),
                    _wing(3, "GER", "fighter", None, 99),  # idle -> excluded
                ],
            }
            with open(os.path.join(d, "oob.json"), "w", encoding="utf-8") as fh:
                json.dump(oob, fh)
            res = asup.compute_air_superiority(d)
            self.assertEqual(res.source, "MODEL")
            self.assertEqual(res.meta["n_wings_total"], 3)
            self.assertEqual(res.meta["n_wings_scored"], 2)  # idle excluded
            r = res.region(1)
            self.assertIsNotNone(r)
            self.assertAlmostEqual(r.percent_for(["GER"]), 50.0)
            # serializable + source-badged
            blob = json.dumps(res.as_dict())
            self.assertIn('"source": "MODEL"', blob)

    def test_missing_oob_raises(self):
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaises(FileNotFoundError):
                asup.compute_air_superiority(d)


# ---------------------------------------------------------------------------
# Integration: real save_oracle/output (skipped if absent)
# ---------------------------------------------------------------------------
@unittest.skipUnless(
    os.path.exists(os.path.join(_OUT, "oob.json")), "no real oob.json present"
)
class TestRealSnapshot(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.res = asup.compute_air_superiority(_OUT)
        with open(os.path.join(_OUT, "oob.json"), "r", encoding="utf-8") as fh:
            cls.oob = json.load(fh)

    def test_only_region_wings_scored(self):
        region_wings = [
            w for w in self.oob["air_wings"] if w.get("strategic_region") is not None
        ]
        self.assertEqual(self.res.meta["n_wings_scored"], len(region_wings))

    def test_percent_in_range_and_leader_present(self):
        for r in self.res.regions.values():
            self.assertTrue(r.by_owner)
            self.assertIsNotNone(r.leader)
            lead_pct = r.percent_for([r.leader])
            if lead_pct is not None:
                self.assertGreaterEqual(lead_pct, 0.0)
                self.assertLessEqual(lead_pct, 100.0)
                # leader holds >= 50% by definition of "leader" in a 2-way split
                # (with >2 owners this need not hold, so only assert the bound).

    def test_model_leader_matches_proxy_on_single_stat_regions(self):
        # On regions where every present plane is a stat-1 class, the stat-weighted
        # leader MUST equal the raw effective_planes (proxy) leader.
        stat1 = {
            "fighter", "cv_fighter", "cas", "cv_cas", "tac_bomber",
            "nav_bomber", "cv_nav_bomber", "scout_plane", "suicide",
        }
        pdef_by_region = {}
        for w in self.oob["air_wings"]:
            rid = w.get("strategic_region")
            if rid is None:
                continue
            pdef_by_region.setdefault(rid, set()).add(w.get("plane_definition"))
        checked = 0
        for rid, r in self.res.regions.items():
            if not pdef_by_region.get(rid, set()) <= stat1:
                continue  # region has a non-stat-1 plane -> ordering may differ
            proxy_leader = max(
                r.by_owner, key=lambda t: r.by_owner[t]["effective_planes"]
            )
            self.assertEqual(r.leader, proxy_leader, f"region {rid}")
            checked += 1
        # Sanity: we actually exercised at least one real region.
        self.assertGreater(checked, 0)


if __name__ == "__main__":
    unittest.main(verbosity=2)
