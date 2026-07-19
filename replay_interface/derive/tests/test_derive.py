"""Unit tests for the derive package.

Two layers of testing:
  * SYNTHETIC mini-world (no assets): deterministic adjacency injected, exercises
    front_lines / encirclement / industry / casualties / manpower / gross_ic gating.
  * REAL 1946 snapshot (skipped if absent): validation invariants -- shares ~1.0,
    ships_by_hull totals == raw OOB, encircled components are small, no fabrication.

Run: python -m derive.tests.test_derive   (or: python -m unittest discover derive)
"""

from __future__ import annotations

import json
import os
import sys
import unittest

_HERE = os.path.dirname(os.path.abspath(__file__))
_REPO = os.path.normpath(os.path.join(_HERE, os.pardir, os.pardir))
if _REPO not in sys.path:
    sys.path.insert(0, _REPO)

from derive import layers, overlay  # noqa: E402

_REAL = os.path.join(_REPO, "replay", "snapshots", "ger_ddr_1946",
                     "000017050260.snapshot.json")
_CAMPAIGN = os.path.join(_REPO, "replay", "snapshots", "campaign")


def _mini_snapshot():
    """A 4-province / 2-controller toy world.

    Provinces 1-2 controlled by AAA (with a supply source state), province 3 by
    BBB (no supply source -> pocket), province 4 by BBB but island-isolated.
    Adjacency: 1-2 (AAA contiguous), 2-3 (front), 3 isolated otherwise, 4 alone.
    """
    snap = {
        "schema_version": "1", "game_id": "mini", "date": "1940.1.1.12",
        "hour_index": 0, "provenance": {"notes": []},
        "countries": {
            "AAA": {"politics": {"capital_state": 10}, "economy": {}, "military": {}},
            "BBB": {"politics": {"capital_state": 20}, "economy": {}, "military": {}},
        },
        "states": {
            "10": {"owner": "AAA", "controller": "AAA",
                   "buildings": {"industrial_complex": 5, "arms_factory": 3, "dockyard": 0},
                   "province_buildings": {"supply_node": 1}},
            "30": {"owner": "BBB", "controller": "BBB",
                   "buildings": {"industrial_complex": 2, "arms_factory": 0, "dockyard": 0},
                   "province_buildings": {}},
        },
        "provinces": {
            "1": {"controller": "AAA", "state_id": 10},
            "2": {"controller": "AAA", "state_id": 10},
            "3": {"controller": "BBB", "state_id": 30},
            "4": {"controller": "BBB", "state_id": 30},
        },
        "divisions": [
            {"div_id": 1, "div_type": 1, "owner": "AAA", "location_province": 1, "strength": 100.0},
            {"div_id": 2, "div_type": 1, "owner": "BBB", "location_province": 3, "strength": 50.0},
            {"div_id": 3, "div_type": 1, "owner": "BBB", "location_province": 4, "strength": 25.0},
        ],
        "diplomacy_world": {"wars": {
            "AAA-BBB@1939.1.1": {"a": "AAA", "b": "BBB", "start_date": "1939.1.1",
                                 "a_casualties": 1000, "b_casualties": 2500},
        }},
        "map": {}, "order_of_battle": {"fleets": [
            {"owner": "AAA", "task_forces": [{"owner": "AAA", "ships": [
                {"definition": "destroyer"}, {"definition": "destroyer"},
                {"definition": "submarine"}]}]},
        ]},
        "designs": {}, "combats": [], "events": [],
    }
    return snap


# province 1-2 same controller adjacent; 2-3 cross-controller front; 4 isolated
_MINI_ADJ = {1: [2], 2: [1, 3], 3: [2], 4: []}


class TestSyntheticLayers(unittest.TestCase):
    def setUp(self):
        self.snap = _mini_snapshot()

    def test_front_lines(self):
        fl = layers.front_lines(self.snap, adj=_MINI_ADJ)
        # only 2-3 is a cross-controller edge
        self.assertEqual(len(fl), 1)
        self.assertEqual((fl[0]["a"], fl[0]["b"]), (2, 3))
        self.assertEqual({fl[0]["controller_a"], fl[0]["controller_b"]}, {"AAA", "BBB"})

    def test_front_lines_null_when_no_control(self):
        s = _mini_snapshot()
        for p in s["provinces"].values():
            p["controller"] = None
        self.assertIsNone(layers.front_lines(s, adj=_MINI_ADJ))

    def test_encirclement(self):
        enc = layers.encirclement(self.snap, adj=_MINI_ADJ)
        # AAA provinces supplied (capital+supply_node in state 10)
        self.assertIs(enc[1], False)
        self.assertIs(enc[2], False)
        # BBB province 3: no supply source, no port -> land pocket (True)
        self.assertIs(enc[3], True)
        # BBB province 4: isolated, no supply source, no port -> True
        self.assertIs(enc[4], True)

    def test_encirclement_port_is_uncertain(self):
        s = _mini_snapshot()
        # give state 30 a naval_base (port) but still no land supply source
        s["states"]["30"]["province_buildings"]["naval_base"] = 2
        enc = layers.encirclement(s, adj=_MINI_ADJ)
        # port-adjacent unsupplied -> None (honest uncertainty), not True/False
        self.assertIsNone(enc[3])
        self.assertIsNone(enc[4])

    def test_industry_share(self):
        cis = layers.controlled_industry_share(self.snap)
        # AAA: 5+3 = 8 ; BBB: 2 ; total 10
        self.assertEqual(cis["total_factory_levels"], 10)
        self.assertAlmostEqual(cis["by_tag"]["AAA"], 0.8)
        self.assertAlmostEqual(cis["by_tag"]["BBB"], 0.2)
        self.assertAlmostEqual(sum(cis["by_tag"].values()), 1.0)
        self.assertEqual(cis["uncontrolled_factory_levels"], 0)

    def test_industry_by_controller_not_owner(self):
        s = _mini_snapshot()
        # flip state 30 controller to AAA while owner stays BBB
        s["states"]["30"]["controller"] = "AAA"
        cis = layers.controlled_industry_share(s)
        # all 10 factory-levels now attributed to AAA (the controller)
        self.assertAlmostEqual(cis["by_tag"]["AAA"], 1.0)
        self.assertNotIn("BBB", cis["by_tag"])

    def test_ships_by_hull(self):
        sbh = layers.ships_by_hull(self.snap)
        self.assertEqual(sbh["AAA"]["destroyer"]["count"], 2)
        self.assertEqual(sbh["AAA"]["submarine"]["count"], 1)
        # tonnage is honestly null (save lacks displacement)
        self.assertIsNone(sbh["AAA"]["destroyer"]["tonnage"])

    def test_ships_by_hull_measured_empty_is_empty_dict_not_null(self):
        # A tag present in the OOB but with zero ships is MEASURED-empty ({}),
        # distinct from a tag never seen in any fleet (null). The overlay must
        # preserve that distinction (honesty rule: 0 means measured zero).
        s = _mini_snapshot()
        s["order_of_battle"]["fleets"].append(
            {"owner": "BBB", "task_forces": [{"owner": "BBB", "ships": []}]})
        out = overlay.derive_layers(s, adj=_MINI_ADJ)
        self.assertEqual(out["countries"]["BBB"]["military"]["ships_by_hull"], {})

    def test_casualties(self):
        cas = layers.casualties(self.snap)
        self.assertEqual(cas["AAA"]["total"], 1000)
        self.assertEqual(cas["BBB"]["total"], 2500)
        # killed/captured not split -> null
        self.assertIsNone(cas["AAA"]["killed"])
        self.assertIsNone(cas["AAA"]["captured"])

    def test_deployed_manpower_proxy(self):
        dep = layers.deployed_manpower(self.snap)
        self.assertEqual(dep["AAA"]["deployed_strength_proxy"], 100.0)
        self.assertEqual(dep["BBB"]["deployed_strength_proxy"], 75.0)
        self.assertEqual(dep["BBB"]["division_count"], 2)

    def test_gross_ic_null_single(self):
        gic = layers.gross_ic_by_equipment(None, self.snap)
        self.assertTrue(all(v is None for v in gic.values()))

    def test_gross_ic_delta(self):
        prev = _mini_snapshot()
        prev["countries"]["AAA"]["economy"]["_production_lines"] = [
            {"archetype": "infantry_equipment", "produced": 10.0}]
        curr = _mini_snapshot()
        curr["countries"]["AAA"]["economy"]["_production_lines"] = [
            {"archetype": "infantry_equipment", "produced": 25.0}]
        gic = layers.gross_ic_by_equipment(prev, curr)
        self.assertAlmostEqual(gic["AAA"]["infantry_equipment"], 15.0)

    def test_overlay_no_mutation(self):
        before = json.dumps(self.snap, sort_keys=True)
        overlay.derive_layers(self.snap, adj=_MINI_ADJ)
        after = json.dumps(self.snap, sort_keys=True)
        self.assertEqual(before, after, "derive_layers must not mutate its input")

    def test_encirclement_absent_from_graph_is_none(self):
        # Codex C1: a controlled province with NO adjacency entry must be None,
        # never a confident True pocket.
        enc = layers.encirclement(self.snap, adj={})  # empty graph
        self.assertIsNone(enc[1])
        self.assertIsNone(enc[3])

    def test_front_lines_none_not_empty_in_overlay(self):
        # Codex C2: when control input is absent, overlay must keep None, not [].
        s = _mini_snapshot()
        for p in s["provinces"].values():
            p["controller"] = None
        out = overlay.derive_layers(s, adj=_MINI_ADJ)
        self.assertIsNone(out["map"]["front_lines"])

    def test_recompute_clears_stale_derived(self):
        # Codex C3: a recompute on a snapshot whose inputs vanished must null out
        # previously-derived D fields, not preserve them.
        s = _mini_snapshot()
        out1 = overlay.derive_layers(s, adj=_MINI_ADJ)
        # now strip the inputs and recompute on the already-derived snapshot
        out1["order_of_battle"]["fleets"] = []
        out1["diplomacy_world"]["wars"] = {}
        out1["divisions"] = []
        out2 = overlay.derive_layers(out1, adj=_MINI_ADJ)
        self.assertIsNone(out2["countries"]["AAA"]["military"]["ships_by_hull"])
        self.assertIsNone(out2["countries"]["AAA"]["military"]["casualties"])
        self.assertIsNone(
            out2["countries"]["AAA"]["military"]["manpower"]["deployed_strength_proxy"])
        self.assertIsNone(out2["countries"]["AAA"]["military"]["division_count"])

    def test_gross_ic_no_baseline_is_null(self):
        # Codex C4: a tag/archetype absent from the prior snapshot has no baseline
        # -> must NOT report current produced as a delta.
        prev = {"countries": {}, "schema_version": "1", "game_id": "m",
                "date": "1940.1.1.12", "hour_index": 0}
        curr = _mini_snapshot()
        curr["countries"]["AAA"]["economy"]["_production_lines"] = [
            {"archetype": "rifle", "produced": 10.0}]
        gic = layers.gross_ic_by_equipment(prev, curr)
        self.assertIsNone(gic["AAA"])  # no prior tag -> no fabricated delta

    def test_encircled_int_normalizes_string_province(self):
        # Codex M3: a division whose location_province is a string still resolves.
        s = _mini_snapshot()
        s["divisions"][0]["location_province"] = "1"  # string, supplied AAA prov
        out = overlay.derive_layers(s, adj=_MINI_ADJ)
        d = [d for d in out["divisions"] if d["owner"] == "AAA"][0]
        self.assertIs(d["encircled"], False)

    def test_overlay_fills_schema_fields(self):
        out = overlay.derive_layers(self.snap, adj=_MINI_ADJ)
        self.assertIn("front_lines", out["map"])
        self.assertIn("controlled_industry_share", out["map"])
        self.assertIsNotNone(out["countries"]["AAA"]["military"]["ships_by_hull"])
        # deployed (people) stays null; proxy is set
        mp = out["countries"]["AAA"]["military"]["manpower"]
        self.assertIsNone(mp["deployed"])
        self.assertEqual(mp["deployed_strength_proxy"], 100.0)
        # division encircled values land on the divisions
        d_aaa = [d for d in out["divisions"] if d["owner"] == "AAA"][0]
        self.assertIs(d_aaa["encircled"], False)


@unittest.skipUnless(os.path.exists(_REAL), "real 1946 snapshot not present")
class TestRealSnapshot(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        # Province adjacency needs either a province_ids raster (gitignored,
        # unregenerable since the generator moved to Rust) or the prebuilt
        # _cache adjacency JSON. Without both, skip the whole class instead of
        # dying inside PIL with a raw FileNotFoundError.
        from derive import adjacency as _adjmod
        assets = _adjmod.default_assets_dir()
        cache = os.path.join(os.path.dirname(os.path.abspath(_adjmod.__file__)),
                             "_cache", "province_adjacency_half.json")
        if not (os.path.exists(os.path.join(assets, "province_ids.png"))
                or os.path.exists(os.path.join(assets, "province_ids_half.png"))
                or os.path.exists(cache)):
            raise unittest.SkipTest(
                "province adjacency unavailable: province_ids raster absent "
                "(gitignored) and no _cache/province_adjacency_half.json")
        with open(_REAL, "r", encoding="utf-8") as fh:
            cls.snap = json.load(fh)
        cls.out = overlay.derive_layers(cls.snap)

    def test_industry_shares_sum_to_one(self):
        cis = self.out["map"]["controlled_industry_share"]
        det = self.out["map"]["controlled_industry_detail"]
        s = sum(cis.values())
        uncontrolled = det["uncontrolled_factory_levels"] / det["total_factory_levels"]
        self.assertAlmostEqual(s + uncontrolled, 1.0, places=4)

    def test_ships_by_hull_total_matches_oob(self):
        raw = 0
        for f in self.snap["order_of_battle"]["fleets"]:
            for tf in (f.get("task_forces") or []):
                raw += len(tf.get("ships") or [])
        der = 0
        for c in self.out["countries"].values():
            sbh = c["military"].get("ships_by_hull")
            if sbh:
                for info in sbh.values():
                    der += info["count"]
        self.assertEqual(der, raw)

    def test_encircled_components_are_small(self):
        # no whole-nation false positive: every encircled-True component is small.
        from derive import adjacency
        adj = adjacency.load_or_build_adjacency()
        ctrl = layers._province_controller_map(self.snap)
        enc = layers.encirclement(self.snap, adj=adj)
        comp_of, comps = layers._components(ctrl, adj)
        sizes = {comp_of[p] for p, v in enc.items() if v is True}
        for cidx in sizes:
            self.assertLess(len(comps[cidx]), 50,
                            "an encircled component is implausibly large")

    def test_no_fabrication_tonnage_and_killed(self):
        for c in self.out["countries"].values():
            sbh = c["military"].get("ships_by_hull")
            if sbh:
                for info in sbh.values():
                    self.assertIsNone(info["tonnage"])
            cas = c["military"].get("casualties")
            if cas and isinstance(cas, dict) and "killed" in cas:
                self.assertIsNone(cas["killed"])

    def test_gross_ic_null_on_single_snapshot(self):
        for c in self.out["countries"].values():
            self.assertIsNone(c["economy"].get("gross_ic_by_equipment"))


@unittest.skipUnless(os.path.isdir(_CAMPAIGN), "campaign series not present")
class TestCampaignSeries(unittest.TestCase):
    def test_gross_ic_delta_on_series(self):
        files = sorted(f for f in os.listdir(_CAMPAIGN) if f.endswith(".snapshot.json"))
        if len(files) < 2:
            self.skipTest("need >=2 campaign snapshots")
        with open(os.path.join(_CAMPAIGN, files[0]), encoding="utf-8") as fh:
            prev = json.load(fh)
        with open(os.path.join(_CAMPAIGN, files[1]), encoding="utf-8") as fh:
            curr = json.load(fh)
        out = overlay.derive_layers(curr, prev_snapshot=prev)
        n = sum(1 for c in out["countries"].values()
                if c["economy"].get("gross_ic_by_equipment"))
        self.assertGreater(n, 0, "a multi-snapshot series should yield some gross_ic deltas")


if __name__ == "__main__":
    unittest.main(verbosity=2)
