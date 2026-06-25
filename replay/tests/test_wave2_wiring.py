"""Tests for the WAVE-2 wiring folded into the replay contract.

NEW save deep domains (parser -> store ingest -> schema field -> diff event):
  * abilities (abilities.json)     -> countries.*.abilities (aces/cooldowns/officer corps)
  * projects (projects.json)       -> countries.*.projects + countries.*.scientists
  * market_extra (market_extra.json) -> countries.*.economy.market_* + diplomacy_world.*
  * deployment (deployment.json)   -> countries.*.military.deployment_queue + expeditionary

NEW derived/live first-class fields (source-badged in FIELD_SOURCES):
  * designs.template_stats / equipment_variant_stats (D)
  * countries.*.scalars / countries.*.score / intel_ledger (D)
  * naval_air.air_superiority_model (D)
  * weather.* (M / D)  + models.supply_estimate (D)  + combats.naval.* (M / M*)

Coverage:
  * schema presence (domains / event types / source map / empty-skeleton validates),
  * store ingestion from the real oracle output (fixture),
  * diff events fire only on a real change, gated on the source domain,
  * ACE DEDUP: ace_gained fires ONCE when both abilities + country_extra carry aces,
  * no-fabrication: missing files -> fields null/empty, domain omitted from provenance.

Existing replay tests must still pass alongside this file.
"""
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))

from replay import schema, store, diff_engine  # noqa: E402

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
ORACLE_OUT = os.path.join(REPO, "save_oracle", "output")
_WAVE2_FILES = ("abilities.json", "projects.json", "market_extra.json",
                "deployment.json")
HAVE_FIXTURES = all(os.path.isfile(os.path.join(ORACLE_OUT, f)) for f in _WAVE2_FILES)


def _snap(date, *, save_domains=None):
    s = schema.empty_snapshot("g", date)
    if save_domains is not None:
        s["provenance"]["save"] = {"captured": True, "domains": save_domains,
                                   "source_filename": None, "save_date": date,
                                   "fills_canonical_domains": True}
    return s


def _types(evs):
    return [e["type"] for e in evs]


class TestSchemaPresence(unittest.TestCase):
    def test_new_domains_registered(self):
        for dom in ("weather", "abilities", "projects", "market_extra",
                    "deployment"):
            self.assertIn(dom, schema.DOMAINS)

    def test_new_event_types_registered(self):
        for t in ("ace_lost", "commander_cooldown_started", "commander_freed",
                  "general_captured", "general_freed", "officer_spirit_changed",
                  "project_finished", "project_started", "facility_built",
                  "facility_dismantled", "scientist_specialization_up",
                  "unit_deployment_started", "unit_deployment_completed",
                  "expeditionary_force_sent", "expeditionary_force_recalled",
                  "market_subsidy_changed", "market_price_level_changed"):
            self.assertIn(t, schema.EVENT_TYPES)
            self.assertIn(t, diff_engine._EVENT_DOMAIN)

    def test_empty_snapshot_has_new_containers_and_validates(self):
        s = schema.empty_snapshot("g", "1936.1.1.12")
        for key in ("weather", "models", "intel_ledger"):
            self.assertIn(key, s)
        self.assertEqual(schema.validate_snapshot(s), [])
        # new country sub-fields present-but-null
        c = schema.empty_country()
        self.assertIsNone(c.get("abilities"))
        self.assertIsNone(c.get("projects"))
        self.assertIsNone(c.get("scientists"))
        self.assertIsNone(c.get("score"))
        self.assertIn("scalars", c)

    def test_source_tags(self):
        # derived (D)
        for p in ("designs.template_stats",
                  "designs.equipment_variant_stats",
                  "countries.GER.scalars.research_speed.pct",
                  "countries.GER.score.army",
                  "naval_air.air_superiority_model.5.leader",
                  "models.supply_estimate.provinces.3.supply",
                  "intel_ledger.GER.SOV.cipher_progress",
                  "weather.regions.20.phenom_name"):
            self.assertEqual(schema.source_of(p), "D", p)
        # live memory (M)
        for p in ("weather.regions.20.temp_c", "weather.regions.20.phenom",
                  "combats.naval.5.attacker.positioning"):
            self.assertEqual(schema.source_of(p), "M", p)
        # save (S)
        for p in ("countries.GER.abilities.aces",
                  "countries.GER.projects",
                  "countries.GER.economy.market_subsidies",
                  "countries.GER.military.deployment_queue",
                  "diplomacy_world.market_requests"):
            self.assertEqual(schema.source_of(p), "S", p)
        # M* tick-transient
        self.assertEqual(
            schema.source_of("combats.naval.5.attacker.damage_history"), "M*")


class TestDiffEvents(unittest.TestCase):
    DOMS = ["abilities", "projects", "deployment", "market_extra", "country_extra"]

    def _pair(self):
        a = _snap("1936.1.1.12", save_domains=self.DOMS)
        b = _snap("1936.1.2.12", save_domains=self.DOMS)
        for s in (a, b):
            s["countries"]["GER"] = schema.empty_country()
        return a, b

    def test_ace_dedup_single_fire(self):
        a, b = self._pair()
        # same ace in BOTH abilities and country_extra; must fire ace_gained ONCE.
        b["countries"]["GER"]["abilities"] = {
            "aces": [{"ace_id": 99, "name": "X", "callsign": "Bubi", "alive": True}],
            "commander_cooldowns": [], "officer_corps": None}
        a["countries"]["GER"]["abilities"] = {
            "aces": [], "commander_cooldowns": [], "officer_corps": None}
        a["countries"]["GER"]["extra"] = {"aces": []}
        b["countries"]["GER"]["extra"] = {"aces": [
            {"name": "X", "surname": None, "callsign": "Bubi"}]}
        evs = diff_engine.diff_snapshots(a, b)
        self.assertEqual(_types(evs).count("ace_gained"), 1)

    def test_country_extra_aces_when_no_abilities_domain(self):
        # abilities NOT captured -> country_extra owns ace_gained (fallback path).
        doms = ["country_extra"]
        a = _snap("1936.1.1.12", save_domains=doms)
        b = _snap("1936.1.2.12", save_domains=doms)
        for s in (a, b):
            s["countries"]["GER"] = schema.empty_country()
        a["countries"]["GER"]["extra"] = {"aces": []}
        b["countries"]["GER"]["extra"] = {"aces": [
            {"name": "Y", "surname": "Z", "callsign": "Ace"}]}
        evs = diff_engine.diff_snapshots(a, b)
        self.assertEqual(_types(evs).count("ace_gained"), 1)

    def test_ace_dedup_one_sided_abilities_capture(self):
        # Codex concern: abilities captured in only ONE snapshot -> _both_captured is
        # AND -> do_abilities False -> country_extra fallback owns aces on BOTH sides
        # (uniform name-keys), so still exactly one ace_gained, no churn within the pair.
        a = _snap("1936.1.1.12", save_domains=["country_extra"])  # no abilities
        b = _snap("1936.1.2.12", save_domains=["country_extra", "abilities"])
        for s in (a, b):
            s["countries"]["GER"] = schema.empty_country()
        a["countries"]["GER"]["extra"] = {"aces": []}
        b["countries"]["GER"]["extra"] = {"aces": [
            {"name": "P", "surname": "Q", "callsign": "Red"}]}
        # b also has the richer abilities copy, but abilities is NOT both-captured.
        b["countries"]["GER"]["abilities"] = {
            "aces": [{"ace_id": 5, "name": "P", "surname": "Q", "callsign": "Red"}],
            "commander_cooldowns": [], "officer_corps": None}
        evs = diff_engine.diff_snapshots(a, b)
        self.assertEqual(_types(evs).count("ace_gained"), 1)

    def test_project_completed_and_finished_are_distinct_types(self):
        # Keeping BOTH the count-based project_completed AND the token-based
        # project_finished is enrichment, not a double-fire of one event (distinct
        # types, distinct keys). Both may fire for the same underlying completion.
        doms = ["country_extra", "projects"]
        a = _snap("1936.1.1.12", save_domains=doms)
        b = _snap("1936.1.2.12", save_domains=doms)
        for s in (a, b):
            s["countries"]["GER"] = schema.empty_country()
        a["countries"]["GER"]["extra"] = {"special_projects": {"projects_completed": 0}}
        b["countries"]["GER"]["extra"] = {"special_projects": {"projects_completed": 1}}
        a["countries"]["GER"]["projects"] = {"completed_projects": []}
        b["countries"]["GER"]["projects"] = {"completed_projects": ["sp_nuke"]}
        t = _types(diff_engine.diff_snapshots(a, b))
        self.assertEqual(t.count("project_completed"), 1)  # the count event
        self.assertEqual(t.count("project_finished"), 1)   # the named-token event

    def test_ace_lost_on_death(self):
        a, b = self._pair()
        a["countries"]["GER"]["abilities"] = {
            "aces": [{"ace_id": 7, "name": "A", "callsign": "C", "alive": True}],
            "commander_cooldowns": [], "officer_corps": None}
        b["countries"]["GER"]["abilities"] = {
            "aces": [{"ace_id": 7, "name": "A", "callsign": "C", "alive": False,
                      "killer_country": "ENG"}],
            "commander_cooldowns": [], "officer_corps": None}
        evs = diff_engine.diff_snapshots(a, b)
        self.assertIn("ace_lost", _types(evs))

    def test_commander_cooldown_and_capture(self):
        a, b = self._pair()
        a["countries"]["GER"]["abilities"] = {
            "aces": [], "officer_corps": None,
            "commander_cooldowns": [{"unit_leader_id": 1, "cooldown_reason": None,
                                     "captured": False}]}
        b["countries"]["GER"]["abilities"] = {
            "aces": [], "officer_corps": None,
            "commander_cooldowns": [{"unit_leader_id": 1,
                                     "cooldown_reason": "reassigned",
                                     "captured": True, "captured_by": "SOV"}]}
        evs = diff_engine.diff_snapshots(a, b)
        self.assertIn("commander_cooldown_started", _types(evs))
        self.assertIn("general_captured", _types(evs))

    def test_project_and_facility_events(self):
        a, b = self._pair()
        a["countries"]["GER"]["projects"] = {"completed_projects": [],
                                             "active_projects": [], "facilities": []}
        b["countries"]["GER"]["projects"] = {
            "completed_projects": ["sp_nuke"],
            "active_projects": [{"token": "sp_rocket"}],
            "facilities": [{"id": 17, "type": "naval_facility", "province": 241}]}
        evs = diff_engine.diff_snapshots(a, b)
        self.assertIn("project_finished", _types(evs))
        self.assertIn("project_started", _types(evs))
        self.assertIn("facility_built", _types(evs))

    def test_scientist_specialization_up(self):
        a, b = self._pair()
        a["countries"]["GER"]["projects"] = {"completed_projects": []}
        b["countries"]["GER"]["projects"] = {"completed_projects": []}
        a["countries"]["GER"]["scientists"] = [
            {"char_id": 5, "name_token": "Dr", "skills": {"specialization_nuclear": 1}}]
        b["countries"]["GER"]["scientists"] = [
            {"char_id": 5, "name_token": "Dr", "skills": {"specialization_nuclear": 3}}]
        evs = diff_engine.diff_snapshots(a, b)
        self.assertIn("scientist_specialization_up", _types(evs))

    def test_deployment_and_subsidy_events(self):
        a, b = self._pair()
        b["countries"]["GER"]["military"]["deployment_queue"] = [
            {"template_id": [5, 1], "location": 64, "name": "Inf"}]
        a["countries"]["GER"]["economy"]["market_subsidies"] = []
        b["countries"]["GER"]["economy"]["market_subsidies"] = [
            {"archetype": "small_plane", "cic": 1000}]
        evs = diff_engine.diff_snapshots(a, b)
        self.assertIn("unit_deployment_started", _types(evs))
        self.assertIn("market_subsidy_changed", _types(evs))

    def test_no_event_when_domain_uncaptured(self):
        # abilities domain NOT in either provenance -> no abilities events fire even
        # if the field data differs (no fabrication).
        a = _snap("1936.1.1.12", save_domains=["country_snapshot"])
        b = _snap("1936.1.2.12", save_domains=["country_snapshot"])
        for s in (a, b):
            s["countries"]["GER"] = schema.empty_country()
        b["countries"]["GER"]["abilities"] = {
            "aces": [{"ace_id": 1, "name": "N", "callsign": "C"}],
            "commander_cooldowns": [], "officer_corps": None}
        evs = diff_engine.diff_snapshots(a, b)
        self.assertNotIn("ace_gained", _types(evs))
        self.assertNotIn("ace_lost", _types(evs))

    def test_all_emitted_events_validate(self):
        a, b = self._pair()
        b["countries"]["GER"]["projects"] = {"completed_projects": ["sp_x"]}
        a["countries"]["GER"]["projects"] = {"completed_projects": []}
        for e in diff_engine.diff_snapshots(a, b):
            self.assertEqual(schema.validate_event(e), [], e)


@unittest.skipUnless(HAVE_FIXTURES, "WAVE-2 oracle fixtures not present")
class TestRealFixtureIngest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.snap = store.build_snapshot_from_oracle(ORACLE_OUT, "fixture")

    def test_validates(self):
        self.assertEqual(schema.validate_snapshot(self.snap), [])

    def test_new_save_domains_in_provenance(self):
        doms = set(self.snap["provenance"]["save"]["domains"])
        for d in ("abilities", "projects", "market_extra", "deployment"):
            self.assertIn(d, doms)

    def test_abilities_ingested(self):
        n = sum(1 for c in self.snap["countries"].values() if c.get("abilities"))
        self.assertGreater(n, 0)

    def test_projects_and_scientists_ingested(self):
        n_pj = sum(1 for c in self.snap["countries"].values() if c.get("projects"))
        n_sci = sum(1 for c in self.snap["countries"].values() if c.get("scientists"))
        self.assertGreater(n_pj, 0)
        self.assertGreater(n_sci, 0)

    def test_market_extra_ingested(self):
        # WIRE_market_extra validation: 4 subsidy countries, request id 144.
        n_sub = sum(1 for c in self.snap["countries"].values()
                    if (c.get("economy") or {}).get("market_subsidies"))
        self.assertGreaterEqual(n_sub, 1)
        self.assertIn(144, self.snap["diplomacy_world"].get("market_requests") or [])

    def test_deployment_ingested(self):
        n_conv = sum(1 for c in self.snap["countries"].values()
                     if (c.get("military") or {}).get("deployment_queue"))
        self.assertGreater(n_conv, 0)


if __name__ == "__main__":
    unittest.main()
