"""Tests for models/victory_score.py.

Written against stdlib ``unittest`` (the repo convention; runs with
``python -m unittest models.tests.test_victory_score``) but also fully
discoverable/runnable by pytest. Mixes pure-unit tests on hand-built fixtures
with optional integration tests over the real save_oracle/output dir (skipped
gracefully when that dir is absent so CI on a clean checkout still passes).
"""
import os
import sys
import unittest

sys.path.insert(
    0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
)

from models import victory_score as vs  # noqa: E402

_REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
_OUT = os.path.join(os.path.dirname(_REPO), "extractor", "save_oracle", "output")
_OUT_1940 = os.path.join(os.path.dirname(_REPO), "extractor", "save_oracle", "output_1940")


# ---------------------------------------------------------------------------
# Synthetic fixtures (no installed content needed)
# ---------------------------------------------------------------------------
def _divisions_fixture():
    return {
        "meta": {"save_date": "1942.1.1.12", "save_version": "test"},
        "divisions": [
            # AAA: 2 armor divisions, full men + full org
            {"owner": "AAA", "template_role": "armor", "strength": 625.0, "organisation": 100.0},
            {"owner": "AAA", "template_role": "armor", "strength": 625.0, "organisation": 100.0},
            # BBB: 2 infantry divisions, full men + full org
            {"owner": "BBB", "template_role": "infantry", "strength": 625.0, "organisation": 100.0},
            {"owner": "BBB", "template_role": "infantry", "strength": 625.0, "organisation": 100.0},
            # CCC: 1 garrison (should count for little)
            {"owner": "CCC", "template_role": "garrison", "strength": 625.0, "organisation": 100.0},
            # missing-owner / missing-strength rows are skipped
            {"owner": None, "template_role": "infantry", "strength": 625.0, "organisation": 100.0},
            {"owner": "BBB", "template_role": "infantry", "strength": None, "organisation": 100.0},
        ],
    }


def _oob_fixture():
    return {
        "fleets": [
            {
                "owner": "AAA",
                "task_forces": [
                    {
                        "owner": "AAA",
                        "ships": [
                            {"definition": "battleship", "strength": 100.0},
                            {"definition": "destroyer", "strength": 100.0},
                        ],
                    }
                ],
            },
            {
                "owner": "BBB",
                "task_forces": [
                    {
                        "owner": "BBB",
                        "ships": [
                            {"definition": "destroyer", "strength": 100.0},
                            {"definition": "destroyer", "strength": 50.0},  # half hull
                        ],
                    }
                ],
            },
        ],
        "air_wings": [
            {"owner": "AAA", "plane_definition": "fighter", "count": 100, "effective_planes": 80.0},
            {"owner": "BBB", "plane_definition": "fighter", "count": 100, "effective_planes": None},
            {"owner": "CCC", "plane_definition": "strat_bomber", "count": 50, "effective_planes": 50.0},
        ],
    }


def _economy_fixture():
    return {
        "countries": {
            "AAA": {"factories": {"civ": 10, "mil": 20, "dockyard": 5}},
            "BBB": {"factories": {"civ": 30, "mil": 5, "dockyard": 0}},
            "CCC": {"factories": {"civ": 0, "mil": 0, "dockyard": 0}},
            "WST": {"factories": {"civ": 999, "mil": 999, "dockyard": 999}},  # synthetic
        }
    }


def _state_detail_fixture():
    return {
        "states": {
            "1": {"controller": "AAA", "owner": "AAA", "victory_points": 50},
            "2": {"controller": "AAA", "owner": "BBB", "victory_points": 10},  # AAA controls
            "3": {"controller": None, "owner": "BBB", "victory_points": 5},  # fallback owner
            "4": {"controller": "CCC", "owner": "CCC", "victory_points": 0},
        }
    }


class TestComponentScorers(unittest.TestCase):
    def test_army_role_weighting(self):
        army = vs.army_strength(_divisions_fixture())
        # armor (weight 2.2) beats equal infantry (1.0); garrison (0.3) tiny.
        self.assertGreater(army["AAA"], army["BBB"])
        self.assertGreater(army["BBB"], army["CCC"])
        # None-owner and None-strength rows skipped -> BBB has exactly 2 counted divs.
        # full org -> org_mult = 1.0; per div = weight * men.
        self.assertAlmostEqual(army["BBB"], 2 * 1.0 * 625.0, places=3)
        self.assertAlmostEqual(army["AAA"], 2 * 2.2 * 625.0, places=3)

    def test_army_org_modulates(self):
        divs = {
            "divisions": [
                {"owner": "X", "template_role": "infantry", "strength": 625.0, "organisation": 100.0},
                {"owner": "Y", "template_role": "infantry", "strength": 625.0, "organisation": 0.0},
            ]
        }
        army = vs.army_strength(divs)
        # org >= ORG_REF -> mult 1.0; org 0 -> mult ORG_MULT_FLOOR (narrow band)
        self.assertAlmostEqual(army["X"], 625.0, places=3)
        self.assertAlmostEqual(army["Y"], vs.ORG_MULT_FLOOR * 625.0, places=3)

    def test_navy_class_and_hull(self):
        navy = vs.navy_strength(_oob_fixture())
        # AAA: battleship(11) + destroyer(1) at full hull = 12
        self.assertAlmostEqual(navy["AAA"], 11.0 + 1.0, places=3)
        # BBB: destroyer full (1.0) + destroyer half hull (0.5) = 1.5
        self.assertAlmostEqual(navy["BBB"], 1.5, places=3)

    def test_air_effective_fallback(self):
        air = vs.air_strength(_oob_fixture())
        # AAA fighter effective=80 -> 80; BBB fighter effective=None -> count 100
        self.assertAlmostEqual(air["AAA"], 80.0, places=3)
        self.assertAlmostEqual(air["BBB"], 100.0, places=3)
        # CCC strat_bomber weight 2.2 * 50 = 110
        self.assertAlmostEqual(air["CCC"], 2.2 * 50.0, places=3)

    def test_industry_weighting(self):
        ind = vs.industry_score(_economy_fixture(), production_json=None)
        # AAA: 1.0*10 + 1.6*20 + 1.3*5 = 10+32+6.5 = 48.5
        self.assertAlmostEqual(ind["AAA"], 48.5, places=3)
        # BBB: 30 + 8 + 0 = 38
        self.assertAlmostEqual(ind["BBB"], 38.0, places=3)
        self.assertAlmostEqual(ind["CCC"], 0.0, places=3)

    def test_industry_output_disabled_by_default(self):
        # OUTPUT_W is 0 by default -> a production_json must NOT change the score.
        prod = {
            "countries": {
                "AAA": {"equipment_lines": [{"speed": 100.0}, {"speed": 50.0}]}
            }
        }
        base = vs.industry_score(_economy_fixture(), production_json=None)["AAA"]
        with_prod = vs.industry_score(_economy_fixture(), production_json=prod)["AAA"]
        self.assertEqual(base, with_prod)
        self.assertEqual(vs.OUTPUT_W, 0.0)

    def test_industry_output_bonus_when_enabled(self):
        # When OUTPUT_W is temporarily enabled, the `speed` (units/day) term applies.
        prod = {
            "countries": {
                "AAA": {"equipment_lines": [{"speed": 100.0}, {"speed": 50.0}]}
            }
        }
        orig = vs.OUTPUT_W
        vs.OUTPUT_W = 0.15
        try:
            base = vs.industry_score(_economy_fixture(), production_json=None)["AAA"]
            boosted = vs.industry_score(_economy_fixture(), production_json=prod)["AAA"]
            self.assertAlmostEqual(boosted - base, 0.15 * 150.0, places=3)
        finally:
            vs.OUTPUT_W = orig

    def test_territory_controller_then_owner(self):
        ter = vs.territory_score(_state_detail_fixture())
        # AAA controls states 1 (50) + 2 (10) = 60
        self.assertAlmostEqual(ter["AAA"], 60.0, places=3)
        # state 3 controller None -> falls back to owner BBB (5)
        self.assertAlmostEqual(ter["BBB"], 5.0, places=3)


class TestComposite(unittest.TestCase):
    def _make(self, **kw):
        # build a ladder from fixtures by monkeypatching load_inputs
        data = {
            "divisions": _divisions_fixture(),
            "oob": _oob_fixture(),
            "economy": _economy_fixture(),
            "state_detail": _state_detail_fixture(),
            "production": None,
            "country_snapshot": None,
        }
        orig = vs.load_inputs
        vs.load_inputs = lambda _d: data  # type: ignore
        try:
            return vs.compute_victory_score("ignored", **kw)
        finally:
            vs.load_inputs = orig

    def test_synthetic_excluded_by_default(self):
        ladder = self._make()
        self.assertNotIn("WST", ladder.countries)
        ladder2 = self._make(include_synthetic=True)
        self.assertIn("WST", ladder2.countries)

    def test_normalization_bounds(self):
        ladder = self._make()
        for cs in ladder.countries.values():
            for n in (cs.army_n, cs.navy_n, cs.air_n, cs.industry_n, cs.territory_n):
                self.assertGreaterEqual(n, 0.0)
                self.assertLessEqual(n, 100.0)
            self.assertGreaterEqual(cs.overall, 0.0)
            self.assertLessEqual(cs.overall, 100.0)

    def test_ranks_are_dense_and_ordered(self):
        ladder = self._make()
        ranks = [ladder.countries[t].rank for t in ladder.ranking]
        self.assertEqual(ranks, list(range(1, len(ranks) + 1)))
        overalls = [ladder.countries[t].overall for t in ladder.ranking]
        self.assertEqual(overalls, sorted(overalls, reverse=True))

    def test_aaa_dominant(self):
        # AAA has top army (armor), top navy (battleship), top territory -> rank 1.
        ladder = self._make()
        self.assertEqual(ladder.ranking[0], "AAA")

    def test_branch_weights_sum_documented(self):
        s = sum(vs.BRANCH_WEIGHTS.values())
        self.assertAlmostEqual(s, 1.0, places=6)


@unittest.skipUnless(os.path.isdir(_OUT), "save_oracle/output not present")
class TestIntegrationOutput(unittest.TestCase):
    def setUp(self):
        self.ladder = vs.compute_victory_score(_OUT)

    def test_major_powers_dominate_top(self):
        # Late-game (1946) save: the surviving industrial/military giants lead.
        top = set(self.ladder.ranking[:5])
        # SOV/ENG/USA are unambiguous endgame leaders here.
        self.assertTrue({"SOV", "ENG", "USA"}.issubset(top), top)

    def test_no_synthetic_in_ladder(self):
        for t in self.ladder.ranking:
            self.assertFalse(vs._is_synthetic(t), t)

    def test_every_country_has_rank(self):
        for cs in self.ladder.countries.values():
            self.assertIsNotNone(cs.rank)


@unittest.skipUnless(os.path.isdir(_OUT_1940), "save_oracle/output_1940 not present")
class TestIntegration1940(unittest.TestCase):
    def test_classic_majors_in_top6(self):
        ladder = vs.compute_victory_score(_OUT_1940)
        top6 = set(ladder.ranking[:6])
        # 1940 mid-game: the canonical great powers should populate the top.
        majors = {"ENG", "SOV", "JAP", "ITA", "USA", "GER", "FRA"}
        overlap = majors & top6
        self.assertGreaterEqual(len(overlap), 5, f"top6={top6}")


if __name__ == "__main__":
    unittest.main()
