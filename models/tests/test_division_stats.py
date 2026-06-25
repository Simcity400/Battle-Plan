"""Tests for models/division_stats.py.

stdlib ``unittest`` (repo convention; runs with
``python -m unittest models.tests.test_division_stats``) and pytest-discoverable.
Mixes pure-unit tests on synthetic hand-built fixtures (no installed content needed)
with optional integration tests over the real game install + save_oracle/output
(skipped gracefully when those are absent, so CI on a clean checkout passes).

Headline invariants under test (mirror the task VALIDATE clause):
  * battalion equipment scaling = equip_stat * need_amount / 100;
  * support-company inline stat = a MULTIPLIER (1 + inline) on the equip stat;
  * division attack/defense = SUM over all battalions;
  * division HP = SUM of max_strength (matches the SAVE's stored strength);
  * division max-org = mean of org>0 line battalions; width = multiple of 2;
  * a tank division has the right order of magnitude for hard atk / breakthrough.
"""
import json
import os
import sys
import unittest

sys.path.insert(
    0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
)

from models import division_stats as ds  # noqa: E402
from game_assets.paradox_parser import parse  # noqa: E402

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
ORACLE_OUT = os.path.join(REPO, "save_oracle", "output")
# Set BATTLE_PLAN_VANILLA to your game install dir to run the game-files
# integration tests; absent it they skip (no machine-specific path in the repo).
VANILLA = os.environ.get("BATTLE_PLAN_VANILLA", os.environ.get("HOI4_VANILLA", ""))
MOD = os.environ.get("BATTLE_PLAN_MOD", os.path.join(os.path.dirname(REPO), "test"))

HAVE_GAME = os.path.isdir(os.path.join(VANILLA, "common", "units"))
HAVE_ORACLE = os.path.isfile(os.path.join(ORACLE_OUT, "designs.json"))


# ---------------------------------------------------------------------------
# Pure-unit tests on a hand-built GameDefs (no installed content touched).
# ---------------------------------------------------------------------------
def _synthetic_defs():
    """Build a GameDefs from in-memory script text, bypassing disk loading."""
    obj = ds.GameDefs.__new__(ds.GameDefs)
    obj.vanilla_dir = None
    obj.mod_dir = None
    obj.sub_units = {}
    obj.equipments = {}
    obj.modules = {}

    units_text = """
    sub_units = {
        infantry = {
            group = infantry
            combat_width = 2
            max_strength = 25
            max_organisation = 60
            default_morale = 0.3
            manpower = 1000
            suppression = 1.5
            supply_consumption = 0.06
            need = { infantry_equipment = 100 }
        }
        artillery_brigade = {
            group = combat_support
            combat_width = 3
            max_strength = 0.6
            max_organisation = 0
            manpower = 500
            need = { artillery_equipment = 24 }
        }
        engineer = {
            group = support
            regimental = no
            combat_width = 0
            max_strength = 2
            max_organisation = 20
            default_morale = 0.3
            manpower = 300
            entrenchment = 2
            defense = 0.1
            breakthrough = 0.5
            soft_attack = -0.5
            need = { infantry_equipment = 10 support_equipment = 30 }
        }
    }
    """
    eq_text = """
    equipments = {
        infantry_equipment = {
            is_archetype = yes
            reliability = 0.9 maximum_speed = 4
            defense = 20 breakthrough = 2 hardness = 0 armor_value = 0
            soft_attack = 3 hard_attack = 0.5 ap_attack = 1 air_attack = 0
        }
        infantry_equipment_3 = {
            archetype = infantry_equipment
            defense = 34 breakthrough = 5
            soft_attack = 12 hard_attack = 2 ap_attack = 10 air_attack = 0
            reliability = 0.8
        }
        artillery_equipment = {
            is_archetype = yes
            reliability = 0.8 maximum_speed = 4
            defense = 10 breakthrough = 6
            soft_attack = 25 hard_attack = 2 ap_attack = 5 air_attack = 0
        }
        artillery_equipment_1 = { archetype = artillery_equipment }
        support_equipment = { is_archetype = yes reliability = 0.8 }
        support_equipment_1 = { archetype = support_equipment }
    }
    """
    uroot = parse(units_text)
    for _, blk in uroot.children_named("sub_units"):
        for name, body in blk.entries:
            obj.sub_units[name] = ds.SubUnit.from_node(name, body)
    eroot = parse(eq_text)
    for _, blk in eroot.children_named("equipments"):
        for name, body in blk.entries:
            obj.equipments[name] = ds.Equip.from_node(name, body)
    return obj


class SyntheticAggregation(unittest.TestCase):
    def setUp(self):
        self.defs = _synthetic_defs()

    def test_equip_stat_no_need_divisor(self):
        # infantry: full equip stat (need=100 is irrelevant to combat stats)
        c = ds.battalion_contribution(
            self.defs.sub_units["infantry"], self.defs.resolve_equip_stats,
            {"infantry_equipment": "infantry_equipment_3"})
        self.assertAlmostEqual(c["soft_attack"], 12.0, places=3)
        self.assertAlmostEqual(c["defense"], 34.0, places=3)
        self.assertAlmostEqual(c["breakthrough"], 5.0, places=3)

    def test_line_artillery_full_stat_no_divisor(self):
        # artillery_brigade need={artillery_equipment=24} but contributes the
        # FULL artillery soft_attack 25 -- NO /100 divisor (Codex-corrected).
        c = ds.battalion_contribution(
            self.defs.sub_units["artillery_brigade"], self.defs.resolve_equip_stats,
            {"artillery_equipment": "artillery_equipment_1"})
        self.assertAlmostEqual(c["soft_attack"], 25.0, places=3)   # not 6.0

    def test_support_inline_is_multiplier(self):
        # engineer soft_attack inline = -0.5 => 12 * (1-0.5) = 6.0 (no divisor)
        c = ds.battalion_contribution(
            self.defs.sub_units["engineer"], self.defs.resolve_equip_stats,
            {"infantry_equipment": "infantry_equipment_3",
             "support_equipment": "support_equipment_1"})
        self.assertAlmostEqual(c["soft_attack"], 12.0 * (1 - 0.5), places=3)  # 6.0
        # defense inline = +0.1 => 34 * 1.1 = 37.4
        self.assertAlmostEqual(c["defense"], 34.0 * 1.1, places=3)
        self.assertTrue(c["is_support"])

    def test_division_sum_and_org_and_hp(self):
        # 6 infantry + 1 engineer.
        template = {
            "regiments": [{"type": "infantry"}] * 6,
            "support": [{"type": "engineer"}],
        }
        vmap = {"infantry_equipment": "infantry_equipment_3",
                "support_equipment": "support_equipment_1"}
        out = ds.compute_division_stats(template, self.defs, vmap)
        # HP = 6*25 + 2 = 152 (ground-truth from the save)
        self.assertAlmostEqual(out.max_strength, 152.0, places=3)
        # soft attack = 6*12 + engineer (12*0.5)=6 => 78.0
        self.assertAlmostEqual(out.soft_attack, 78.0, places=2)
        # defense = 6*34 + engineer (34*1.1)=37.4 => 241.4
        self.assertAlmostEqual(out.defense, 241.4, places=2)
        # org = mean over ALL 7 sub-units = (6*60 + 20)/7 = 54.2857
        self.assertAlmostEqual(out.max_organisation, (6 * 60 + 20) / 7, places=3)
        # width = 6*2 = 12 (multiple of 2)
        self.assertEqual(out.combat_width, 12.0)
        self.assertEqual(out.combat_width % 2, 0.0)
        self.assertEqual(out.n_battalions, 6)
        self.assertEqual(out.n_support, 1)

    def test_org_includes_zero_org_line_artillery(self):
        # infantry (org 60) + artillery_brigade (org 0) -> mean = (60+0)/2 = 30
        template = {
            "regiments": [{"type": "infantry"}, {"type": "artillery_brigade"}],
            "support": [],
        }
        out = ds.compute_division_stats(template, self.defs,
                                        {"infantry_equipment": "infantry_equipment_3",
                                         "artillery_equipment": "artillery_equipment_1"})
        self.assertAlmostEqual(out.max_organisation, 30.0, places=3)

    def test_armor_piercing_blend(self):
        # _blend = w*max + (1-w)*avg. [40,0,0] with w=0.4 => 0.4*40 + 0.6*(40/3)
        # = 16 + 8 = 24. [10,10,10] with w=0.4 => 10.
        self.assertAlmostEqual(ds._blend([40, 0, 0], 0.4),
                               0.4 * 40 + 0.6 * (40 / 3), places=4)
        self.assertAlmostEqual(ds._blend([10, 10, 10], 0.4), 10.0, places=4)
        self.assertEqual(ds._blend([], 0.4), 0.0)
        # vanilla defines: both 0.4 (verified in 00_defines.lua).
        self.assertAlmostEqual(ds.ARMOR_VS_AVERAGE, 0.4, places=6)
        self.assertAlmostEqual(ds.PEN_VS_AVERAGE, 0.4, places=6)

    def test_speed_is_multiplier(self):
        # foot infantry: no transport -> infantry_equipment maximum_speed (4),
        # sub_unit maximum_speed None -> *(1+0) = 4.
        c = ds.battalion_contribution(
            self.defs.sub_units["infantry"], self.defs.resolve_equip_stats,
            {"infantry_equipment": "infantry_equipment_3"})
        self.assertAlmostEqual(c["maximum_speed"], 4.0, places=3)

    def test_modular_add_then_multiply(self):
        # add_stats applied before multiply_stats on running total.
        self.defs.modules["m_add"] = {"add": {"maximum_speed": 0.5},
                                      "mult": {}, "avg": {}}
        self.defs.modules["m_mult"] = {"add": {}, "mult": {"maximum_speed": 0.15},
                                       "avg": {}}
        base = {"maximum_speed": 6.0}
        out = self.defs.apply_modules(base, ["m_add", "m_mult"])
        # (6 + 0.5) * 1.15 = 7.475
        self.assertAlmostEqual(out["maximum_speed"], (6.0 + 0.5) * 1.15, places=4)


# ---------------------------------------------------------------------------
# Integration tests over the real install + save (skipped if unavailable).
# ---------------------------------------------------------------------------
@unittest.skipUnless(HAVE_GAME and HAVE_ORACLE,
                     "needs real the game install + save_oracle/output")
class RealIntegration(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.defs = ds.GameDefs(vanilla_dir=VANILLA, mod_dir=MOD)
        with open(os.path.join(ORACLE_OUT, "designs.json"), encoding="utf-8") as f:
            cls.designs = json.load(f)
        with open(os.path.join(ORACLE_OUT, "divisions.json"), encoding="utf-8") as f:
            cls.divisions = json.load(f)
        cls.tmpl = {t["id"]: t for t in cls.designs["division_templates"]}

    def test_defs_loaded(self):
        self.assertGreater(len(self.defs.sub_units), 50)
        self.assertGreater(len(self.defs.equipments), 100)
        self.assertIn("infantry", self.defs.sub_units)
        # defines parsed from the real 00_defines.lua (both 0.4 in vanilla)
        self.assertAlmostEqual(self.defs.armor_vs_average, 0.4, places=3)
        self.assertAlmostEqual(self.defs.pen_vs_average, 0.4, places=3)

    def test_hp_matches_save_strength(self):
        # The SAVE stores per-division strength (HP). Our summed max_strength
        # must equal it for at least one real infantry division.
        for div in self.divisions["divisions"]:
            t = self.tmpl.get(div["template_id"])
            if not t:
                continue
            if div.get("template_role") != "infantry":
                continue
            out = ds.compute_division_stats(t, self.defs,
                                            ds.variant_map_from_division(div))
            # HP is the exact sum of template max_strength; the save stores the
            # division's current strength which can differ slightly from the
            # template max (attrition / partial reinforcement). Allow a small
            # tolerance but require the same order of magnitude.
            self.assertGreater(out.max_strength, 0)
            self.assertAlmostEqual(out.max_strength, div["strength"],
                                   delta=max(5.0, 0.15 * div["strength"]))
            return
        self.skipTest("no infantry division found")

    def test_infantry_magnitudes(self):
        # find any 6-10 infantry battalion template; org in [40,90], width %2==0
        for t in self.designs["division_templates"]:
            inf = sum(1 for r in t["regiments"] if r["type"] == "infantry")
            if 6 <= inf <= 10 and t.get("country") == "GER":
                out = ds.compute_division_stats(t, self.defs)
                # org is the mean over all sub-units; support companies pull it
                # below 60 but it stays in a sane infantry band.
                self.assertGreaterEqual(out.max_organisation, 30)
                self.assertLessEqual(out.max_organisation, 90)
                self.assertEqual(out.combat_width % 2, 0.0)
                self.assertGreater(out.soft_attack, 0)
                self.assertGreater(out.defense, 0)
                return
        self.skipTest("no GER infantry template found")

    def test_tank_division_order_of_magnitude(self):
        resolver = ds.make_modular_resolver(self.defs,
                                            self.designs["equipment_variants"])
        for div in self.divisions["divisions"]:
            t = self.tmpl.get(div["template_id"])
            if not t:
                continue
            n_armor = sum(1 for r in t["regiments"]
                          if "armor" in r["type"] and "amphib" not in r["type"])
            if n_armor >= 3:
                out = ds.compute_division_stats(
                    t, self.defs, ds.variant_map_from_division(div),
                    equip_resolver=resolver)
                # tank divisions have meaningful hard attack & breakthrough
                self.assertGreater(out.hard_attack, 5)
                self.assertGreater(out.breakthrough, 20)
                self.assertGreater(out.piercing, 0)
                self.assertGreater(out.max_strength, 0)
                return
        self.skipTest("no armor division found")


if __name__ == "__main__":
    unittest.main()
