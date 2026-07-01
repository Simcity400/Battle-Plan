"""Tests for the SAVE-bundle fold (backlog #1) + first-class naval_air/supply
sidecars (backlog #3), all MOCK/fixture-based -- NO live memory, NO live game.

Covers:
  * fold_save_bundle backstops the canonical national domains from a melted-save
    oracle dir, with the LIVE channel WINNING wherever it measured a value and the
    save filling only the gaps (no-fabrication: a measured 0/0.0 from live is kept;
    a field stays null iff NEITHER channel measured it),
  * provenance separation: save-derived data lands under provenance.save with
    fills_canonical_domains=True and a `domains` list -- the memory channel is NOT
    touched and save data is NOT mislabeled as a memory domain,
  * the assembler promotes naval_air (region control + air superiority) and supply
    into FIRST-CLASS typed schema fields carrying a source/confidence badge, while
    keeping the raw _live_* sidecar for back-compat,
  * NO user path leaks into the folded snapshot.
"""
from __future__ import annotations

import json
import os
import shutil
import sys
import tempfile
import unittest

_HERE = os.path.dirname(os.path.abspath(__file__))
_REPO = os.path.normpath(os.path.join(_HERE, os.pardir, os.pardir))
if _REPO not in sys.path:
    sys.path.insert(0, _REPO)

from replay import schema  # noqa: E402
from live_capture import registry as reg  # noqa: E402
from live_capture import assembler as asm  # noqa: E402


class FakeIdentity:
    instance_id = "inst-A"
    pid = 1234
    exe_sha256 = "deadbeef"

    def build_ok(self):
        return True


def _scratch_root():
    base = os.path.join(tempfile.gettempdir(), "re_scratch")
    os.makedirs(base, exist_ok=True)
    return tempfile.mkdtemp(prefix="integ_fold_", dir=base)


def _dump(root, name, obj):
    with open(os.path.join(root, name), "w", encoding="utf-8") as fh:
        json.dump(obj, fh)


def _make_oracle_dir(root):
    """A small synthetic oracle output with politics scalars, laws, focus, market,
    and an occupied state -- enough to prove the backstop fold + live-wins rule."""
    os.makedirs(root, exist_ok=True)
    # a user-directory path on meta, so the no-leak guard has something to scrub.
    _dump(root, "meta.json",
          {"source_filename": "/home/u/secret/SYN_melted.txt",
           "source_path_redacted": "/home/u/secret/SYN_melted.txt",
           "save_date": "1937.6.1.12", "save_version": "syn"})
    _dump(root, "country_snapshot.json", {
        "GER": {"stability": 0.75, "ruling_party": "communism",
                "factories": {"civ": 20, "mil": 30, "dockyard": 5}},
        "SOV": {"stability": 0.5, "ruling_party": "communism"}})
    _dump(root, "economy.json", {
        "countries": {
            "GER": {"laws": {"economy": "war_economy", "trade": "free_trade",
                             "conscription": "extensive_conscription"},
                    "ideas_all": ["idea_a", "idea_b"]}}})
    _dump(root, "focus.json", {
        "countries": {"GER": {"focus_tree": "german_focus",
                              "completed": ["f1", "f2"], "completed_count": 2}}})
    _dump(root, "country_extra.json", {
        "countries": {
            "GER": {
                "script_flags": {
                    "GER_has_started_war": {
                        "value": 1, "date": "1939.9.2.1"}}}}})
    _dump(root, "market.json", {
        "meta": {},
        "contracts": [{"contract_id": [5, 81], "seller": "SOV", "buyer": "GER",
                       "speed": 1, "equipments": [], "prices": [],
                       "convoys": {"assigned": 1, "total": 1}, "days": 2,
                       "delivery_state": {}, "completed": 0}],
        "countries": {}})
    return root


def _make_doctrine_oracle_dir(root):
    os.makedirs(root, exist_ok=True)
    _dump(root, "doctrine.json", {
        "meta": {
            "source_filename": "/home/u/secret/SYN_melted.txt",
            "save_date": "1937.6.1.12",
            "save_version": "syn",
        },
        "countries": {
            "GER": {
                "raw": {"completed": ["superior_firepower"]},
                "completed": [{"id": "superior_firepower", "level": 1,
                               "date": None, "research_points": None}],
                "completed_count": 1,
                "in_progress": [],
            },
        },
        "countries_by_key": {
            "39": {
                "raw": {"completed": ["mass_assault"]},
                "completed": [{"id": "mass_assault", "level": 1,
                               "date": None, "research_points": None}],
                "completed_count": 1,
                "in_progress": [],
            },
        },
    })
    return root


# ---------------------------------------------------------------------------
# fold_save_bundle: backstop + live-wins
# ---------------------------------------------------------------------------
class TestSaveFold(unittest.TestCase):
    def setUp(self):
        self.scratch = _scratch_root()
        self.oracle = _make_oracle_dir(os.path.join(self.scratch, "oracle"))

    def tearDown(self):
        shutil.rmtree(self.scratch, ignore_errors=True)

    def _live_snap_with_ger_stability(self, stab):
        res = {"country_scalars": reg.DomainResult("country_scalars", True, True, {
            "countries": {"GER": {"stability": stab}}, "count": 1})}
        return asm.assemble_snapshot(res, game_id="g", date="1937.6.1.12",
                                     identity=FakeIdentity())

    def test_live_value_wins_over_save(self):
        snap = self._live_snap_with_ger_stability(0.99)
        asm.fold_save_bundle(snap, self.oracle)
        # live stability 0.99 WINS over save 0.75
        self.assertEqual(snap["countries"]["GER"]["politics"]["stability"], 0.99)
        # save fills the gaps the live channel left null
        self.assertEqual(snap["countries"]["GER"]["politics"]["ruling_party"],
                         "communism")
        self.assertEqual(snap["countries"]["GER"]["politics"]["laws"]["economy"],
                         "war_economy")
        self.assertEqual(snap["countries"]["GER"]["focus"]["focus_tree"],
                         "german_focus")
        self.assertEqual(
            snap["countries"]["GER"]["extra"]["script_flags"]
            ["GER_has_started_war"]["date"],
            "1939.9.2.1")
        self.assertEqual(schema.validate_snapshot(snap), [])

    def test_measured_zero_from_live_is_kept(self):
        # a measured 0.0 from the live channel must NOT be overwritten by a save value
        snap = self._live_snap_with_ger_stability(0.0)
        asm.fold_save_bundle(snap, self.oracle)
        self.assertEqual(snap["countries"]["GER"]["politics"]["stability"], 0.0)

    def test_field_stays_null_when_neither_channel_measured(self):
        # SOV has no political_power in either channel -> stays null (no fabrication)
        snap = self._live_snap_with_ger_stability(0.6)
        asm.fold_save_bundle(snap, self.oracle)
        self.assertIsNone(
            snap["countries"]["SOV"]["politics"]["political_power"])

    def test_save_only_tag_adopted_wholesale(self):
        # SOV appears only in the save -> fully adopted from the save
        snap = self._live_snap_with_ger_stability(0.6)
        asm.fold_save_bundle(snap, self.oracle)
        self.assertIn("SOV", snap["countries"])
        self.assertEqual(snap["countries"]["SOV"]["politics"]["stability"], 0.5)

    def test_market_folds_lend_lease(self):
        snap = self._live_snap_with_ger_stability(0.6)
        asm.fold_save_bundle(snap, self.oracle)
        ger_ll = snap["countries"]["GER"]["diplomacy"]["lend_lease"]
        sov_ll = snap["countries"]["SOV"]["diplomacy"]["lend_lease"]
        self.assertEqual(len(ger_ll), 1)
        self.assertEqual(ger_ll[0]["direction"], "incoming")
        self.assertEqual(sov_ll[0]["direction"], "outgoing")
        self.assertEqual(len(snap["diplomacy_world"]["trade_contracts"]), 1)

    def test_provenance_separation_save_not_memory(self):
        snap = self._live_snap_with_ger_stability(0.6)
        mem_domains_before = list(snap["provenance"]["memory"]["domains"])
        asm.fold_save_bundle(snap, self.oracle)
        # memory channel is UNTOUCHED -- save domains are NOT mislabeled as memory
        self.assertEqual(snap["provenance"]["memory"]["domains"],
                         mem_domains_before)
        sav = snap["provenance"]["save"]
        self.assertTrue(sav["fills_canonical_domains"])
        self.assertFalse(sav["is_live"])
        self.assertIn("country_snapshot", sav["domains"])
        self.assertIn("country_extra", sav["domains"])
        self.assertIn("market", sav["domains"])
        # source filename is a BASENAME only
        self.assertEqual(sav["source_filename"], "SYN_melted.txt")
        # and the save domains are NOT in the memory channel
        for d in sav["domains"]:
            self.assertNotIn(d, snap["provenance"]["memory"]["domains"])

    def test_no_user_path_leak_after_fold(self):
        snap = self._live_snap_with_ger_stability(0.6)
        asm.fold_save_bundle(snap, self.oracle)
        blob = json.dumps(snap)
        for needle in ("/home/u", "secret", "source_path_redacted"):
            self.assertNotIn(needle, blob, needle)

    def test_empty_oracle_dir_is_noop(self):
        empty = os.path.join(self.scratch, "empty")
        os.makedirs(empty, exist_ok=True)
        snap = self._live_snap_with_ger_stability(0.6)
        before_save = snap["provenance"]["save"]
        asm.fold_save_bundle(snap, empty)
        # no domains to fold -> provenance.save unchanged, no fabricated save block
        self.assertEqual(snap["provenance"]["save"], before_save)

    def test_doctrine_only_oracle_folds_as_save_backstop_not_canonical_domain(self):
        doctrine = _make_doctrine_oracle_dir(os.path.join(self.scratch, "doctrine"))
        snap = asm.assemble_snapshot({}, game_id="g", date="1937.6.1.12",
                                     identity=FakeIdentity())
        asm.fold_save_bundle(snap, doctrine)
        self.assertEqual(schema.validate_snapshot(snap), [])
        d = snap["countries"]["GER"]["research"]["doctrine"]
        self.assertEqual(d["source"], "S")
        self.assertEqual(d["completed"][0]["id"], "superior_firepower")
        # Numeric/non-TAG save keys are preserved but not mapped onto countries.
        self.assertNotIn("39", snap["countries"])
        self.assertEqual(
            snap["doctrine_save"]["countries_by_key"]["39"]["completed"][0]["id"],
            "mass_assault")
        sav = snap["provenance"]["save"]
        self.assertTrue(sav["captured"])
        self.assertFalse(sav["fills_canonical_domains"])
        self.assertNotIn("domains", sav)
        self.assertIn("doctrine", sav["backstops"])
        self.assertEqual(sav["save_date"], "1937.6.1.12")
        self.assertEqual(sav["source_filename"], "SYN_melted.txt")
        self.assertEqual(sav["source_filenames"]["doctrine"]["source_filename"],
                         "SYN_melted.txt")
        blob = json.dumps(snap)
        for needle in ("/home/u", "secret"):
            self.assertNotIn(needle, blob, needle)

    def test_measured_empty_live_leaf_preserved_over_save(self):
        # Codex P1: a LIVE-measured empty []/{} at a registered live leaf is a
        # MEASURED zero-count and must NOT be overwritten by stale save data. We
        # register a synthetic protected leaf to exercise the mechanism without
        # depending on which live readers exist today.
        snap = self._live_snap_with_ger_stability(0.6)
        # simulate a future live reader having measured GER's lend_lease as empty
        snap["countries"]["GER"]["diplomacy"]["lend_lease"] = []
        orig = asm._LIVE_COUNTRY_DOMAIN_LEAVES.get("country_scalars", ())
        asm._LIVE_COUNTRY_DOMAIN_LEAVES["country_scalars"] = \
            tuple(orig) + ("diplomacy.lend_lease",)
        try:
            asm.fold_save_bundle(snap, self.oracle)
        finally:
            asm._LIVE_COUNTRY_DOMAIN_LEAVES["country_scalars"] = orig
        # GER measured zero contracts -> the save's GER contract must NOT overwrite it
        self.assertEqual(snap["countries"]["GER"]["diplomacy"]["lend_lease"], [])

    def test_unregistered_empty_leaf_still_filled_by_save(self):
        # the protection is LEAF-PRECISE: an empty list the live channel did NOT
        # measure (not registered) is still a skeleton default -> save fills it.
        snap = self._live_snap_with_ger_stability(0.6)
        # GER focus.completed is the skeleton [] (country_scalars never measures it)
        self.assertEqual(snap["countries"]["GER"]["focus"]["completed"], [])
        asm.fold_save_bundle(snap, self.oracle)
        # save's focus.completed (["f1","f2"]) fills it
        self.assertEqual(snap["countries"]["GER"]["focus"]["completed"],
                         ["f1", "f2"])

    def test_fold_coexists_with_buildings_backstop(self):
        # a buildings SAVE backstop (fills_canonical_domains=False) must survive the
        # full-bundle fold; the fold's source_filenames preserve the sidecar's.
        res = {
            "country_scalars": reg.DomainResult("country_scalars", True, True, {
                "countries": {"GER": {"stability": 0.6}}}),
            "buildings": reg.DomainResult("buildings", True, True, {
                "source": "save", "rail_by_province": {"1": 5},
                "melt_file": "autosave.hoi4"}),
        }
        snap = asm.assemble_snapshot(res, game_id="g", date="1937.6.1.12",
                                     identity=FakeIdentity())
        # before fold: save block is the buildings backstop (fills none canonical)
        self.assertFalse(snap["provenance"]["save"]["fills_canonical_domains"])
        asm.fold_save_bundle(snap, self.oracle)
        sav = snap["provenance"]["save"]
        self.assertTrue(sav["fills_canonical_domains"])
        # the buildings sidecar + its backstop entry survive
        self.assertIn("buildings", sav.get("backstops", []))
        self.assertIn("autosave.hoi4", json.dumps(sav["source_filenames"]))


# ---------------------------------------------------------------------------
# naval_air / supply first-class fields (backlog #3)
# ---------------------------------------------------------------------------
class TestNavalAirSupplyFirstClass(unittest.TestCase):
    def test_naval_air_promoted_to_typed_field_with_badge(self):
        res = {"naval_air": reg.DomainResult("naval_air", True, True, {
            "sea_zone_control": {"5": [{"controller": "GER", "share": 0.8}]},
            "air_superiority": {"5": {"controller": "GER", "value": 0.6}}})}
        snap = asm.assemble_snapshot(res, game_id="g", date="1940.9.10.13",
                                     identity=FakeIdentity())
        self.assertEqual(schema.validate_snapshot(snap), [])
        na = snap["naval_air"]
        self.assertEqual(na["region_control"]["5"][0]["controller"], "GER")
        self.assertEqual(na["air_superiority"]["5"]["controller"], "GER")
        # badge present: source M, confidence is a PROXY label, never "exact"
        self.assertEqual(na["source"], "M")
        self.assertEqual(na["confidence"], "region_proxy")
        self.assertNotEqual(na["confidence"], "exact")
        # raw sidecar still present for back-compat
        self.assertIn("_live_naval_air", snap)
        self.assertIn("naval_air", snap["provenance"]["memory"]["domains"])

    def test_supply_promoted_to_typed_field_with_badge(self):
        res = {"supply": reg.DomainResult("supply", True, True, {
            "by_province": {"100": {"current": 5.0, "capacity": 10.0,
                                    "gain": 1.0, "divisions": 2}}, "count": 1})}
        snap = asm.assemble_snapshot(res, game_id="g", date="1940.9.10.13",
                                     identity=FakeIdentity())
        self.assertEqual(schema.validate_snapshot(snap), [])
        sup = snap["supply"]
        self.assertEqual(sup["by_province"]["100"]["current"], 5.0)
        self.assertEqual(sup["source"], "M")
        self.assertEqual(sup["confidence"], "division_province_proxy")
        self.assertIn("_live_supply", snap)

    def test_naval_air_supply_absent_stay_null(self):
        snap = asm.assemble_snapshot({}, game_id="g", date="1940.9.10.13",
                                     identity=FakeIdentity())
        self.assertIsNone(snap["naval_air"]["region_control"])
        self.assertIsNone(snap["naval_air"]["source"])
        self.assertIsNone(snap["supply"]["by_province"])
        self.assertNotIn("naval_air", snap["provenance"]["memory"]["domains"])
        self.assertNotIn("supply", snap["provenance"]["memory"]["domains"])


# ---------------------------------------------------------------------------
# Real-oracle fold (skipped when output absent)
# ---------------------------------------------------------------------------
ORACLE_OUT = os.path.join(_REPO, "save_oracle", "output")
HAVE_ORACLE = os.path.isfile(os.path.join(ORACLE_OUT, "country_snapshot.json"))


@unittest.skipUnless(HAVE_ORACLE, "real save_oracle/output not present")
class TestRealFold(unittest.TestCase):
    def test_fold_real_oracle_onto_live_snap(self):
        res = {"country_scalars": reg.DomainResult("country_scalars", True, True, {
            "countries": {"GER": {"stability": 0.99}}})}
        snap = asm.assemble_snapshot(res, game_id="g", date="1946.5.18.12",
                                     identity=FakeIdentity())
        asm.fold_save_bundle(snap, ORACLE_OUT)
        self.assertEqual(schema.validate_snapshot(snap), [])
        # live wins; save backfills the national tabs
        self.assertEqual(snap["countries"]["GER"]["politics"]["stability"], 0.99)
        self.assertIsNotNone(
            snap["countries"]["GER"]["politics"]["ruling_party"])
        self.assertGreater(len(snap["countries"]), 100)
        self.assertGreater(len(snap["states"]), 1000)
        self.assertTrue(snap["provenance"]["save"]["fills_canonical_domains"])
        # the memory channel still records ONLY what the live channel measured
        self.assertEqual(snap["provenance"]["memory"]["domains"],
                         ["country_scalars"])
        blob = json.dumps(snap)
        for needle in ("aa" + "ugu", "One" + "Drive", "re_scratch", "source_path_redacted"):
            self.assertNotIn(needle, blob, needle)

    def test_derived_models_attach_via_fold(self):
        """WAVE-2: fold_save_bundle attaches the derived-model layers (provenance D)
        WITHOUT leaking a path and WITHOUT breaking validation. Models that need a
        game install degrade to None (no fabrication) — we only assert the
        output-dir-driven ones, which run from the oracle JSON alone."""
        res = {"country_scalars": reg.DomainResult("country_scalars", True, True, {
            "countries": {"GER": {"stability": 0.5}}})}
        snap = asm.assemble_snapshot(res, game_id="g", date="1946.5.18.12",
                                     identity=FakeIdentity())
        asm.fold_save_bundle(snap, ORACLE_OUT)
        self.assertEqual(schema.validate_snapshot(snap), [])
        # victory score: at least some countries got a derived score block.
        n_score = sum(1 for c in snap["countries"].values() if c.get("score"))
        self.assertGreater(n_score, 0)
        # supply estimate folded under the new models block.
        self.assertIsNotNone((snap.get("models") or {}).get("supply_estimate"))
        # no path leak survives the model meta scrub.
        blob = json.dumps(snap)
        for needle in ("aa" + "ugu", "One" + "Drive", "Steam" + "Library",
                       "source_path_redacted"):
            self.assertNotIn(needle, blob, needle)

    def test_live_readers_guarded_offline(self):
        """WAVE-2: with no live pid (identity=None) the weather + naval-combat-stat
        live readers must NOT run, NOT claim a domain, and NOT crash. The offline
        snapshot stays the null-but-present weather skeleton."""
        snap = asm.assemble_snapshot({}, game_id="g", date="1936.1.1.12")
        self.assertEqual(schema.validate_snapshot(snap), [])
        self.assertNotIn("weather", snap["provenance"]["memory"]["domains"])
        self.assertNotIn("naval_air_combat_stats",
                         snap["provenance"]["memory"]["domains"])
        self.assertIsNone((snap.get("weather") or {}).get("regions"))


if __name__ == "__main__":
    unittest.main(verbosity=2)
