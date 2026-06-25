"""Tests for models/equipment_stats.py.

stdlib ``unittest`` (repo convention; runs with
``python -m unittest models.tests.test_equipment_stats``) and pytest-discoverable.
Pure-unit tests on synthetic in-memory defs (no installed content needed) cover the
exact aggregation order of operations; integration tests over the real game
install + save_oracle/output are skipped gracefully when those are absent so a
clean checkout still passes.

Headline invariants (mirror the task VALIDATE clause + the Codex-confirmed
order of operations in re_scratch/WIRE_equipment_stats.md):

  * base archetype resolves through the parent/archetype chain (child wins);
  * module add_stats are flat additive; multiply_stats are SUMMED then applied
    once to (base + adds);
  * add_average_stats (ship AP) average across PROVIDING modules, not slots;
  * upgrade add_stats blocks + reliability/carrier_size bare keys are FLAT per
    level; every other bare upgrade key is a per-level PERCENT factor;
  * sanity directions: a heavy battery raises naval attack; a bigger engine
    raises speed and (for tanks) lowers reliability.
"""
import json
import os
import sys
import unittest

sys.path.insert(
    0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
)

from models import equipment_stats as es           # noqa: E402
from models import division_stats as ds             # noqa: E402
from game_assets.paradox_parser import parse        # noqa: E402

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
ORACLE_OUT = os.path.join(REPO, "save_oracle", "output")
ORACLE_1940 = os.path.join(REPO, "save_oracle", "output_1940")
# Set BATTLE_PLAN_VANILLA to your game install dir to run the game-files
# integration tests; absent it they skip (no machine-specific path in the repo).
VANILLA = os.environ.get("BATTLE_PLAN_VANILLA", os.environ.get("HOI4_VANILLA", ""))
MOD = os.environ.get("BATTLE_PLAN_MOD", os.path.join(os.path.dirname(REPO), "test"))

HAVE_GAME = os.path.isdir(os.path.join(VANILLA, "common", "units", "equipment"))
HAVE_ORACLE = os.path.isfile(os.path.join(ORACLE_OUT, "designs.json"))


# ---------------------------------------------------------------------------
# Synthetic defs (no disk): a tank chassis, a ship hull, a plane airframe, the
# modules they use, and the upgrades, all built in memory.
# ---------------------------------------------------------------------------
_EQ_TEXT = """
equipments = {
    light_tank_chassis = {
        is_archetype = yes
        maximum_speed = 4 build_cost_ic = 4 reliability = 0.8
        armor_value = 10 defense = 4 hardness = 0.8 fuel_consumption = 0
    }
    light_tank_chassis_1 = {
        archetype = light_tank_chassis
        maximum_speed = 5 build_cost_ic = 4.35 reliability = 0.95 armor_value = 15
    }
    ship_hull_light = {
        is_archetype = yes
        naval_speed = 30 max_strength = 50 armor_value = 0 reliability = 0.8
        hg_attack = 0 lg_attack = 0 hg_armor_piercing = 0 anti_air_attack = 0
        torpedo_attack = 0 naval_range = 3000 build_cost_ic = 250
    }
    ship_hull_light_1 = {
        archetype = ship_hull_light
        max_strength = 60 naval_speed = 32 build_cost_ic = 300
    }
    medium_plane_airframe = {
        is_archetype = yes
        reliability = 0.8 air_superiority = 1 naval_strike_attack = 1
    }
    medium_plane_airframe_1 = {
        archetype = medium_plane_airframe
        maximum_speed = 325 air_agility = 30 air_defence = 18 air_range = 900
        build_cost_ic = 26
    }
    mechanized_equipment = {
        is_archetype = yes
        maximum_speed = 8 armor_value = 10 reliability = 0.8 build_cost_ic = 8
    }
    mechanized_equipment_1 = { archetype = mechanized_equipment }
}
"""

_MODULE_TEXT = """
equipment_modules = {
    tank_gasoline_engine = {
        category = tank_engine_type
        add_stats = { build_cost_ic = 1 fuel_consumption = 2 maximum_speed = 0.5 }
        multiply_stats = { maximum_speed = 0.15 }
    }
    tank_big_engine = {
        category = tank_engine_type
        add_stats = { build_cost_ic = 3 maximum_speed = 0.5 reliability = -0.1 }
        multiply_stats = { maximum_speed = 0.25 }
    }
    tank_high_velocity_cannon = {
        category = tank_small_main_armament
        add_stats = { soft_attack = 10 hard_attack = 26 ap_attack = 68
                      build_cost_ic = 3 maximum_speed = -0.1 reliability = -0.05 }
    }
    ship_heavy_battery_2 = {
        category = ship_heavy_battery
        add_stats = { hg_attack = 18 build_cost_ic = 1650 }
        multiply_stats = { naval_speed = -0.06 }
        add_average_stats = { hg_armor_piercing = 36 }
    }
    ship_anti_air_2 = {
        category = ship_anti_air
        add_stats = { anti_air_attack = 6 build_cost_ic = 60 }
    }
    light_ship_engine_2 = {
        category = light_ship_engine
        multiply_stats = { naval_speed = 0.20 build_cost_ic = 0.10 }
    }
    plane_big_engine = {
        category = medium_plane_engine_type
        add_stats = { build_cost_ic = 48 }
        multiply_stats = { maximum_speed = 0.30 }
    }
    plane_defense_turret = {
        category = plane_special_module_defense_turret
        add_stats = { air_defence = 6 build_cost_ic = 6 }
    }
}
"""

_UPGRADE_TEXT = """
upgrades = {
    ship_gun_upgrade = {
        max_level = 5
        cost = naval
        hg_attack = 0.04
        hg_armor_piercing = 0.05
        naval_speed = -0.01
        reliability = -0.005
    }
    tank_nsb_engine_upgrade = {
        max_level = 20
        cost = land
        reliability = -0.015
        add_stats = { fuel_consumption = 0.05 build_cost_ic = 0.1 maximum_speed = 0.1 }
    }
    tank_nsb_armor_upgrade = {
        max_level = 20
        cost = land
        reliability = -0.015
        add_stats = { maximum_speed = -0.1 breakthrough = 1.25 build_cost_ic = 0.2 armor_value = 2.5 }
    }
    tank_reliability_upgrade = {
        max_level = 5
        cost = land
        reliability = 0.1
        build_cost_ic = 0.05
    }
    ship_deckspace_upgrade = {
        max_level = 5
        cost = naval
        carrier_size = 1
        armor_value = -0.05
    }
}
"""


def _synthetic_defs():
    """Build an EquipmentDefs from in-memory text, bypassing all disk loading."""
    # GameDefs (equipments + modules), via __new__ to skip _load().
    gd = ds.GameDefs.__new__(ds.GameDefs)
    gd.vanilla_dir = None
    gd.mod_dir = None
    gd.sub_units = {}
    gd.equipments = {}
    gd.modules = {}
    eroot = parse(_EQ_TEXT)
    for _, blk in eroot.children_named("equipments"):
        for name, body in blk.entries:
            gd.equipments[name] = ds.Equip.from_node(name, body)
    gd._collect_modules(parse(_MODULE_TEXT))

    # EquipmentDefs wrapper, via __new__ to skip its disk loader too.
    ed = es.EquipmentDefs.__new__(es.EquipmentDefs)
    ed.defs = gd
    ed.upgrades = {}
    uroot = parse(_UPGRADE_TEXT)
    for _, blk in uroot.children_named("upgrades"):
        for name, body in blk.entries:
            ed.upgrades[name] = es.Upgrade.from_node(name, body)
    return ed


def _variant(archetype, family, modules=None, upgrades=None, name=None):
    return {
        "archetype": archetype, "family": family, "name": name,
        "modules": modules or {}, "upgrades": upgrades or {},
    }


class SyntheticAggregation(unittest.TestCase):
    def setUp(self):
        self.defs = _synthetic_defs()

    # --- base / inheritance ------------------------------------------------
    def test_base_inherits_parent_chain(self):
        base = self.defs.base_stats("light_tank_chassis_1")
        # child overrides maximum_speed/armor; inherits hardness/defense.
        self.assertAlmostEqual(base["maximum_speed"], 5.0)
        self.assertAlmostEqual(base["armor_value"], 15.0)
        self.assertAlmostEqual(base["hardness"], 0.8)   # inherited
        self.assertAlmostEqual(base["defense"], 4.0)    # inherited

    # --- module flat add + multiply ---------------------------------------
    def test_module_add_and_multiply(self):
        v = _variant("light_tank_chassis_1", "land_vehicle",
                     modules={"engine": "tank_gasoline_engine",
                              "gun": "tank_high_velocity_cannon"})
        vs = es.compute_variant_stats(v, self.defs)
        # soft_attack: base 0 + cannon 10 = 10 (no multiply)
        self.assertAlmostEqual(vs.stats["soft_attack"], 10.0)
        self.assertAlmostEqual(vs.stats["ap_attack"], 68.0)
        # maximum_speed: (base 5 + gas 0.5 + cannon -0.1) * (1 + 0.15) = 5.4*1.15
        self.assertAlmostEqual(vs.stats["maximum_speed"], (5 + 0.5 - 0.1) * 1.15)
        self.assertEqual(vs.unknown_modules, [])

    def test_bigger_engine_raises_speed_lowers_reliability(self):
        small = es.compute_variant_stats(
            _variant("light_tank_chassis_1", "land_vehicle",
                     modules={"engine": "tank_gasoline_engine"}), self.defs)
        big = es.compute_variant_stats(
            _variant("light_tank_chassis_1", "land_vehicle",
                     modules={"engine": "tank_big_engine"}), self.defs)
        self.assertGreater(big.stats["maximum_speed"], small.stats["maximum_speed"])
        # tank_big_engine add_stats reliability = -0.1 (flat); small has none
        self.assertLess(big.stats["reliability"], small.stats["reliability"])

    # --- multiply_stats summed, applied to base+adds ----------------------
    def test_multiply_summed_not_compounded(self):
        # two heavy batteries (each naval_speed mult -0.06) + engine (+0.20)
        v = _variant("ship_hull_light_1", "ship",
                     modules={"b1": "ship_heavy_battery_2",
                              "b2": "ship_heavy_battery_2",
                              "eng": "light_ship_engine_2"})
        vs = es.compute_variant_stats(v, self.defs)
        # naval_speed mult sum = -0.06 -0.06 +0.20 = 0.08; base 32, no adds
        self.assertAlmostEqual(vs.stats["naval_speed"], 32 * (1 + 0.08))

    # --- ship heavy battery raises attack; AP averaged --------------------
    def test_heavy_battery_raises_attack_ap_averaged(self):
        bare = es.compute_variant_stats(
            _variant("ship_hull_light_1", "ship",
                     modules={"eng": "light_ship_engine_2"}), self.defs)
        armed = es.compute_variant_stats(
            _variant("ship_hull_light_1", "ship",
                     modules={"b1": "ship_heavy_battery_2",
                              "b2": "ship_heavy_battery_2",
                              "eng": "light_ship_engine_2"}), self.defs)
        # attack rises from 0
        self.assertAlmostEqual(bare.stats["hg_attack"], 0.0)
        self.assertAlmostEqual(armed.stats["hg_attack"], 36.0)   # 2 * 18
        # AP averaged across the 2 providing modules -> 36, NOT 72
        self.assertAlmostEqual(armed.stats["hg_armor_piercing"], 36.0)

    # --- upgrades: percent vs flat ----------------------------------------
    def test_naval_gun_upgrade_is_percent_on_postmodule(self):
        # ship_gun_upgrade hg_attack=0.04/level multiplies the 36 from batteries
        v = _variant("ship_hull_light_1", "ship",
                     modules={"b1": "ship_heavy_battery_2",
                              "b2": "ship_heavy_battery_2"},
                     upgrades={"ship_gun_upgrade": 5})
        vs = es.compute_variant_stats(v, self.defs)
        # hg_attack 36 * (1 + 0.04*5) = 36 * 1.2 = 43.2
        self.assertAlmostEqual(vs.stats["hg_attack"], 36.0 * 1.2)
        # AP 36 * (1 + 0.05*5) = 36 * 1.25 = 45
        self.assertAlmostEqual(vs.stats["hg_armor_piercing"], 36.0 * 1.25)
        # reliability bare upgrade = FLAT: base 0.8 + (-0.005*5) = 0.775
        self.assertAlmostEqual(vs.stats["reliability"], 0.8 - 0.005 * 5)

    def test_tank_nsb_upgrade_is_flat_per_level(self):
        v = _variant("light_tank_chassis_1", "land_vehicle",
                     modules={"engine": "tank_gasoline_engine"},
                     upgrades={"tank_nsb_armor_upgrade": 4,
                               "tank_nsb_engine_upgrade": 5})
        vs = es.compute_variant_stats(v, self.defs)
        # armor: base 15 + 2.5*4 (flat) = 25
        self.assertAlmostEqual(vs.stats["armor_value"], 15 + 2.5 * 4)
        # reliability: 0.95 + (-0.015*4) + (-0.015*5) = 0.95 - 0.135 = 0.815
        self.assertAlmostEqual(vs.stats["reliability"], 0.95 - 0.015 * 4 - 0.015 * 5)

    def test_carrier_size_upgrade_is_flat(self):
        up = self.defs.upgrades["ship_deckspace_upgrade"]
        # carrier_size is in the FLAT bucket; armor_value stays percent
        self.assertIn("carrier_size", up.flat)
        self.assertAlmostEqual(up.flat["carrier_size"], 1.0)
        self.assertIn("armor_value", up.pct)
        self.assertAlmostEqual(up.pct["armor_value"], -0.05)

    def test_max_level_clamp(self):
        v = _variant("ship_hull_light_1", "ship",
                     modules={"b1": "ship_heavy_battery_2"},
                     upgrades={"ship_gun_upgrade": 99})   # over max_level 5
        vs = es.compute_variant_stats(v, self.defs)
        # clamps to 5: 18 * (1 + 0.04*5) = 18 * 1.2 = 21.6
        self.assertAlmostEqual(vs.stats["hg_attack"], 18.0 * 1.2)

    # --- plane: bigger engine raises speed; turret raises defence ----------
    def test_plane_engine_and_turret(self):
        v = _variant("medium_plane_airframe_1", "plane",
                     modules={"eng": "plane_big_engine",
                              "t1": "plane_defense_turret"})
        vs = es.compute_variant_stats(v, self.defs)
        # speed: 325 * (1 + 0.30) = 422.5
        self.assertAlmostEqual(vs.stats["maximum_speed"], 325 * 1.30)
        # defence: base 18 + turret 6 = 24
        self.assertAlmostEqual(vs.stats["air_defence"], 24.0)

    # --- upgrade-only (no modules) variant --------------------------------
    def test_upgrade_only_variant(self):
        v = _variant("mechanized_equipment_1", "land_other",
                     upgrades={"tank_reliability_upgrade": 5})
        vs = es.compute_variant_stats(v, self.defs)
        # reliability flat: 0.8 + 0.1*5 = 1.3
        self.assertAlmostEqual(vs.stats["reliability"], 0.8 + 0.1 * 5)
        self.assertEqual(vs.modules, {})

    # --- robustness: unknown names recorded, not crashing ------------------
    def test_unknown_module_and_upgrade_recorded(self):
        v = _variant("light_tank_chassis_1", "land_vehicle",
                     modules={"x": "no_such_module", "y": "empty"},
                     upgrades={"no_such_upgrade": 3})
        vs = es.compute_variant_stats(v, self.defs)
        self.assertIn("no_such_module", vs.unknown_modules)
        self.assertNotIn("empty", vs.unknown_modules)        # empty is benign
        self.assertIn("no_such_upgrade", vs.unknown_upgrades)


# ---------------------------------------------------------------------------
# Integration: real install + real save_oracle/output (skipped if absent).
# ---------------------------------------------------------------------------
@unittest.skipUnless(HAVE_GAME and HAVE_ORACLE,
                     "real game install + designs.json required")
class RealDesigns(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.defs = es.EquipmentDefs(vanilla_dir=VANILLA, mod_dir=MOD)
        with open(os.path.join(ORACLE_OUT, "designs.json"), encoding="utf-8") as fh:
            cls.designs = json.load(fh)

    def test_every_module_and_upgrade_resolves(self):
        """No referenced module/upgrade is missing from the parsed defs."""
        allv = es.compute_all(self.designs, self.defs)
        self.assertGreater(len(allv), 1000)
        unk_mod, unk_up = set(), set()
        for vs in allv.values():
            unk_mod.update(vs.unknown_modules)
            unk_up.update(vs.unknown_upgrades)
        self.assertEqual(unk_mod, set(), f"unknown modules: {sorted(unk_mod)[:20]}")
        self.assertEqual(unk_up, set(), f"unknown upgrades: {sorted(unk_up)[:20]}")

    def test_panzer_iii_directions(self):
        """The real GER Panzer III (1595:70): cannon -> attack; engine -> speed
        up; engine+armor upgrades -> reliability down vs the bare chassis."""
        v = self.designs["equipment_variants"].get("1595:70")
        if not v:
            self.skipTest("variant 1595:70 absent in this save")
        vs = es.compute_variant_stats(v, self.defs)
        self.assertGreater(vs.stats["hard_attack"], 20)     # high-velocity cannon
        self.assertGreater(vs.stats["ap_attack"], 50)
        # reliability dragged below the chassis base by upgrades + cannon
        self.assertLess(vs.stats["reliability"], vs.base["reliability"])
        # armor raised above base by armor module + nsb armor upgrade
        self.assertGreater(vs.stats["armor_value"], vs.base.get("armor_value", 0))

    def test_heavy_ship_battery_raises_attack(self):
        """A heavy-ship variant with batteries has hg_attack >> 0 and AP set."""
        ev = self.designs["equipment_variants"]
        v = ev.get("2084:70")
        if not v:
            self.skipTest("variant 2084:70 absent")
        vs = es.compute_variant_stats(v, self.defs)
        self.assertGreater(vs.stats["hg_attack"], 0)
        self.assertGreater(vs.stats["hg_armor_piercing"], 0)
        # max_strength (HP) from a real heavy hull is on the order of hundreds
        self.assertGreater(vs.stats["max_strength"], 100)

    def test_1940_designs_also_resolve(self):
        if not os.path.isfile(os.path.join(ORACLE_1940, "designs.json")):
            self.skipTest("output_1940 absent")
        with open(os.path.join(ORACLE_1940, "designs.json"), encoding="utf-8") as fh:
            d = json.load(fh)
        allv = es.compute_all(d, self.defs)
        unk = set()
        for vs in allv.values():
            unk.update(vs.unknown_modules)
            unk.update(vs.unknown_upgrades)
        self.assertEqual(unk, set(), f"unknown refs in 1940 set: {sorted(unk)[:20]}")


if __name__ == "__main__":
    unittest.main()
