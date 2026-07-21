"""Tests for the market (market.json) + occupation (occupation.json) wiring:

  * schema fields/source-map/domains/event-types present,
  * store ingestion (synthetic + real oracle) of trade contracts -> per-country
    lend_lease (both directions) + world-scoped trade_contracts, market stockpiles,
    and occupier-side occupation-law fold onto occupied states,
  * the new diff events (lend_lease_started/ended, resistance_changed,
    compliance_changed, occupation_law_changed) fire ONLY on a real change and are
    gated on the source domain (no fabrication when the domain wasn't captured),
  * no-fabrication: a missing market/occupation file -> those fields null/empty,
    domain omitted from provenance.
"""
import json
import os
import shutil
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))

from replay import schema, store, diff_engine  # noqa: E402

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
ORACLE_OUT = os.path.join(os.path.dirname(REPO), "extractor", "save_oracle", "output")
HAVE_ORACLE = os.path.isfile(os.path.join(ORACLE_OUT, "market.json"))


def _scratch_root():
    base = os.path.join(tempfile.gettempdir(), "re_scratch")
    os.makedirs(base, exist_ok=True)
    return tempfile.mkdtemp(prefix="integ_mkocc_", dir=base)


def _dump(root, name, obj):
    with open(os.path.join(root, name), "w", encoding="utf-8") as fh:
        json.dump(obj, fh)


def _make_oracle_dir(root, *, with_market=True, with_occ=True):
    os.makedirs(root, exist_ok=True)
    _dump(root, "meta.json",
          {"source_filename": "/secret/path/SYN_melted.txt",
           "save_date": "1937.6.1.12", "save_version": "syn"})
    _dump(root, "country_snapshot.json",
          {"GER": {"stability": 0.6}, "SAF": {"stability": 0.5},
           "ENG": {"stability": 0.55}})
    # a state owned/occupied so the occupation fold has a target
    _dump(root, "state_detail.json", {
        "states": {
            "1": {"owner": "GER", "state_category": "town",
                  "resistance": {"resistance": 22.3, "resistance_speed": -0.1,
                                 "compliance": 26.0, "compliance_speed": 0.04,
                                 "occupied_country_tag": "FRA",
                                 "operational_status": True}},
            "2": {"owner": "GER", "state_category": "town",
                  "resistance": {"resistance": 0.0, "resistance_speed": 0.0,
                                 "compliance": 100.0, "compliance_speed": 0.0,
                                 "occupied_country_tag": "FRA",
                                 "operational_status": True}}}})
    _dump(root, "designs.json", {
        "equipment_variants": {
            "90:70": {"id": 90, "type": 70,
                      "archetype": "infantry_equipment_0"},
            "438:70": {"id": 438, "type": 70,
                       "archetype": "support_equipment_1"}},
        "division_templates": [],
        "countries": {}})
    if with_market:
        _dump(root, "market.json", {
            "meta": {"source_path_redacted": "secret"},
            "contracts": [
                {"contract_id": [465, 81], "seller": "SAF", "buyer": "ENG",
                 "speed": 1,
                 "equipments": [{"equip_id": 3220, "equip_type": 70,
                                 "archetype": "medium_tank_chassis_2",
                                 "amount": 32.0}],
                 "prices": [], "delivery_country": "ENG",
                 "convoys": {"assigned": 3, "total": 3},
                 "delivery_efficiency": 4.0, "days": 4,
                 "delivery_state": {"collected": 1070.0}, "completed": 7}],
            "countries": {
                "SAF": {"market_stockpile": [{"equip_id": 90, "equip_type": 70,
                                              "archetype": "infantry_equipment_0",
                                              "amount": 767.0}],
                        "market_stockpile_entries": 1,
                        "automation": {"auto_accept_purchase": True}}}})
    if with_occ:
        _dump(root, "occupation.json", {
            "meta": {"source_path_redacted": "secret"},
            "countries": {
                "GER": {"default_law": "foreign_civilian_oversight",
                        "garrison_template_id": [4183, 4713],
                        "occupied_count": 1,
                        "occupied": [{"states": [1, 2], "resistance": 22.3,
                                      "compliance": 26.0,
                                      "occupation_law_by_state":
                                          {"1": "military_governor_occupation"},
                                      "state_garrison_data": {
                                          "1": {
                                              "strength_ratio": 0.9375,
                                              "garrison_required": 0.55477,
                                              "army_manpower_need": 13869,
                                              "manpower_pool": [{
                                                  "source_ordinal": 0,
                                                  "tag": "GER", "value": 13869.0}],
                                              "equipment": [{
                                                  "source_ordinal": 0,
                                                  "equipment_id": 90,
                                                  "equipment_type": 70,
                                                  "amount": 1664.31}],
                                              "equipment_meta": {
                                                  "present": True,
                                                  "rows_complete": True,
                                                  "allow_zero_entries": False},
                                              "equipment_need": {
                                                  "infantry_equipment": 1664.31},
                                              "reinforcement_requests": [{
                                                  "source_ordinal": 0,
                                                  "total_progress": 7.57643,
                                                  "need": {
                                                      "infantry_equipment": 661.31},
                                                  "date": "1942.8.18.2"}]},
                                          "2": {
                                              "strength_ratio": 1.0,
                                              "garrison_required": 0.0,
                                              "army_manpower_need": 0,
                                              "manpower_pool": [],
                                              "equipment": [],
                                              "equipment_meta": {
                                                  "present": True,
                                                  "rows_complete": True,
                                                  "allow_zero_entries": False},
                                              "equipment_need": {},
                                              "reinforcement_requests": []}},
                                      "occupied_tag": "FRA"}],
                        "resistance_attack_log": [{
                            "stable_event_id": "GER|1942.8.15.1|1|0",
                            "source_ordinal": 0,
                            "state_id": 1,
                            "occupied_country_tag": "FRA",
                            "event_hour_or_engine_date": "1942.8.15.1",
                            "attack_blocked": True,
                            "resistance_activity": None,
                            "lost_manpower": 33,
                            "equipment_loss_rows": [{
                                "source_ordinal": 0,
                                "equipment_id": 438,
                                "equipment_type": 70,
                                "amount": 1.0}],
                            "equipment_loss_meta": {
                                "present": True,
                                "rows_complete": True,
                                "allow_zero_entries": False},
                            "row_complete": True}],
                        "resistance_attack_log_complete": True}}})
    return root


# ---------------------------------------------------------------------------
# Schema presence
# ---------------------------------------------------------------------------
class TestSchemaPresence(unittest.TestCase):
    def test_market_domain_and_event_types(self):
        self.assertIn("market", schema.DOMAINS)
        self.assertIn("occupation", schema.DOMAINS)
        for t in ("lend_lease_started", "lend_lease_ended", "resistance_changed",
                  "compliance_changed", "occupation_law_changed"):
            self.assertIn(t, schema.EVENT_TYPES)

    def test_field_sources(self):
        self.assertEqual(schema.source_of("countries.GER.diplomacy.lend_lease"), "S/M/D")
        self.assertEqual(schema.source_of("countries.GER.economy.market_stockpile"), "C/S")
        self.assertEqual(schema.source_of("states.5.occupation_law"), "S")
        self.assertEqual(schema.source_of("states.5.occupation.garrison_equipment"), "S")
        self.assertEqual(schema.source_of("occupation_history"), "S")
        self.assertEqual(schema.source_of("diplomacy_world.trade_contracts"), "S")
        # dynamic tag resolves too
        self.assertEqual(schema.source_of("countries.D05.diplomacy.lend_lease"), "S/M/D")

    def test_naval_air_supply_first_class(self):
        s = schema.empty_snapshot("g", "1940.1.1.0")
        self.assertEqual(schema.validate_snapshot(s), [])
        self.assertIn("naval_air", s)
        self.assertIn("supply", s)
        self.assertIsNone(s["naval_air"]["region_control"])
        self.assertIsNone(s["supply"]["by_province"])
        self.assertEqual(schema.source_of("naval_air.region_control"), "M")
        self.assertEqual(schema.source_of("supply.by_province"), "M")


# ---------------------------------------------------------------------------
# Store ingestion (synthetic)
# ---------------------------------------------------------------------------
class TestIngestSynthetic(unittest.TestCase):
    def setUp(self):
        self.scratch = _scratch_root()
        self.oracle = _make_oracle_dir(os.path.join(self.scratch, "oracle"))

    def tearDown(self):
        shutil.rmtree(self.scratch, ignore_errors=True)

    def test_validates(self):
        snap = store.build_snapshot_from_oracle(self.oracle, "syn")
        self.assertEqual(schema.validate_snapshot(snap), [])

    def test_market_attaches_to_both_parties(self):
        snap = store.build_snapshot_from_oracle(self.oracle, "syn")
        saf = snap["countries"]["SAF"]["diplomacy"]["lend_lease"]
        eng = snap["countries"]["ENG"]["diplomacy"]["lend_lease"]
        self.assertEqual(len(saf), 1)
        self.assertEqual(len(eng), 1)
        self.assertEqual(saf[0]["direction"], "outgoing")
        self.assertEqual(saf[0]["counterpart"], "ENG")
        self.assertEqual(eng[0]["direction"], "incoming")
        self.assertEqual(eng[0]["counterpart"], "SAF")
        # canonical world list present once
        self.assertEqual(len(snap["diplomacy_world"]["trade_contracts"]), 1)

    def test_market_stockpile_on_country(self):
        snap = store.build_snapshot_from_oracle(self.oracle, "syn")
        saf_eco = snap["countries"]["SAF"]["economy"]
        self.assertEqual(len(saf_eco["market_stockpile"]), 1)
        self.assertTrue(saf_eco["market_automation"]["auto_accept_purchase"])

    def test_occupation_law_folded_onto_state(self):
        snap = store.build_snapshot_from_oracle(self.oracle, "syn")
        st = snap["states"]["1"]
        self.assertEqual(st["occupation_law"], "military_governor_occupation")
        # resistance object kept (from state_detail), enriched with occupier tag
        self.assertEqual(st["occupation"]["resistance"], 22.3)
        self.assertEqual(st["occupation"]["occupier_tag"], "GER")
        self.assertEqual(st["occupation"]["occupied_country_tag"], "FRA")

    def test_exact_garrison_equipment_history_and_default_law_are_joined(self):
        snap = store.build_snapshot_from_oracle(self.oracle, "syn")
        first = snap["states"]["1"]["occupation"]
        self.assertEqual(first["garrison_template_id"], 4183)
        self.assertEqual(first["garrison_army_manpower_need"], 13869)
        self.assertEqual(first["garrison_manpower_pool"], [{
            "source_ordinal": 0, "tag": "GER", "value": 13869.0}])
        self.assertEqual(first["garrison_equipment"], [{
            "source_ordinal": 0,
            "equipment_id": 90,
            "equipment_type": 70,
            "equipment_token": "infantry_equipment_0",
            "amount": 1664.31}])
        self.assertTrue(first["garrison_equipment_complete"])
        self.assertEqual(first["garrison_reinforcement_requests"][0]["need"],
                         {"infantry_equipment": 661.31})

        # No explicit state assignment: exact controller default is folded with
        # a distinct provenance label, not silently presented as per-state data.
        second = snap["states"]["2"]["occupation"]
        self.assertEqual(snap["states"]["2"]["occupation_law"],
                         "foreign_civilian_oversight")
        self.assertEqual(second["occupation_law_token_source"],
                         "save_controller_default_law")
        self.assertEqual(second["garrison_equipment"], [])
        self.assertTrue(second["garrison_equipment_complete"])

        self.assertTrue(snap["occupation_history_complete"])
        history = snap["occupation_history"]
        self.assertEqual(len(history), 1)
        self.assertEqual(history[0]["stable_event_id"], "GER|1942.8.15.1|1|0")
        self.assertTrue(history[0]["attack_blocked"])
        self.assertEqual(history[0]["lost_manpower"], 33)
        self.assertEqual(history[0]["equipment_loss_rows"][0]["equipment_token"],
                         "support_equipment_1")
        self.assertTrue(history[0]["equipment_loss_children_complete"])

    def test_provenance_lists_market_and_occupation(self):
        snap = store.build_snapshot_from_oracle(self.oracle, "syn")
        doms = snap["provenance"]["save"]["domains"]
        self.assertIn("market", doms)
        self.assertIn("occupation", doms)

    def test_no_personal_path(self):
        snap = store.build_snapshot_from_oracle(self.oracle, "syn")
        blob = json.dumps(snap)
        for n in ("/secret/path", "source_path_redacted"):
            self.assertNotIn(n, blob, n)


# ---------------------------------------------------------------------------
# No-fabrication: missing files
# ---------------------------------------------------------------------------
class TestMissingFiles(unittest.TestCase):
    def setUp(self):
        self.scratch = _scratch_root()
        self.oracle = _make_oracle_dir(os.path.join(self.scratch, "oracle"),
                                       with_market=False, with_occ=False)

    def tearDown(self):
        shutil.rmtree(self.scratch, ignore_errors=True)

    def test_missing_market_occupation_null_safe(self):
        snap = store.build_snapshot_from_oracle(self.oracle, "syn")
        self.assertEqual(schema.validate_snapshot(snap), [])
        self.assertEqual(snap["countries"]["SAF"]["diplomacy"]["lend_lease"], [])
        self.assertNotIn("trade_contracts", snap["diplomacy_world"])
        # state_detail still gives the resistance object, but no occupation_law
        self.assertIsNone(snap["states"]["1"]["occupation_law"])
        doms = snap["provenance"]["save"]["domains"]
        self.assertNotIn("market", doms)
        self.assertNotIn("occupation", doms)


# ---------------------------------------------------------------------------
# Diff events
# ---------------------------------------------------------------------------
def _snap(date, *, save_domains=None):
    s = schema.empty_snapshot("g", date)
    if save_domains is not None:
        s["provenance"]["save"] = {"captured": True, "domains": save_domains,
                                   "source_filename": None, "save_date": date,
                                   "fills_canonical_domains": True}
    return s


def _types(evs):
    return [e["type"] for e in evs]


class TestMarketDiffEvents(unittest.TestCase):
    def _with_contracts(self, snap, tag, contracts):
        c = schema.empty_country()
        c["diplomacy"]["lend_lease"] = contracts
        snap["countries"][tag] = c

    def test_lend_lease_started_and_ended(self):
        a = _snap("1940.1.1.0", save_domains=["market"])
        b = _snap("1940.1.2.0", save_domains=["market"])
        self._with_contracts(a, "SAF", [
            {"contract_id": "1:81", "direction": "outgoing", "counterpart": "ENG",
             "equipments": [{"archetype": "infantry_equipment_1"}]}])
        self._with_contracts(b, "SAF", [
            {"contract_id": "2:81", "direction": "outgoing", "counterpart": "SOV",
             "equipments": [{"archetype": "artillery_equipment_1"}]}])
        evs = diff_engine.diff_snapshots(a, b)
        types = _types(evs)
        self.assertIn("lend_lease_started", types)
        self.assertIn("lend_lease_ended", types)
        started = next(e for e in evs if e["type"] == "lend_lease_started")
        self.assertEqual(started["key"], "2:81")
        self.assertEqual(started["object"], "SOV")
        self.assertEqual(started["to"], "artillery_equipment_1")
        ended = next(e for e in evs if e["type"] == "lend_lease_ended")
        self.assertEqual(ended["key"], "1:81")

    def test_no_event_when_contract_unchanged(self):
        a = _snap("1940.1.1.0", save_domains=["market"])
        b = _snap("1940.1.2.0", save_domains=["market"])
        c = [{"contract_id": "1:81", "direction": "outgoing", "counterpart": "ENG",
              "equipments": []}]
        self._with_contracts(a, "SAF", list(c))
        self._with_contracts(b, "SAF", list(c))
        self.assertEqual(_types(diff_engine.diff_snapshots(a, b)), [])

    def test_market_gated_when_not_captured(self):
        a = _snap("1940.1.1.0", save_domains=["market"])
        b = _snap("1940.1.2.0", save_domains=["focus"])  # b did NOT capture market
        self._with_contracts(a, "SAF", [
            {"contract_id": "1:81", "direction": "outgoing", "counterpart": "ENG"}])
        self.assertNotIn("lend_lease_ended",
                         _types(diff_engine.diff_snapshots(a, b)))


class TestOccupationDiffEvents(unittest.TestCase):
    def _state(self, snap, sid, *, resistance=None, compliance=None, law=None,
               occupier="GER", occupied="FRA"):
        st = schema.empty_state()
        occ = {"occupier_tag": occupier, "occupied_country_tag": occupied}
        if resistance is not None:
            occ["resistance"] = resistance
        if compliance is not None:
            occ["compliance"] = compliance
        st["occupation"] = occ
        if law is not None:
            st["occupation_law"] = law
        snap["states"][sid] = st

    def test_resistance_band_crossing_emits(self):
        a = _snap("1940.1.1.0", save_domains=["state_detail"])
        b = _snap("1940.1.2.0", save_domains=["state_detail"])
        self._state(a, "1", resistance=22.3)   # band: >=15 (idx 1)
        self._state(b, "1", resistance=50.0)   # band: >=40 (idx 3) -> crossing
        evs = diff_engine.diff_snapshots(a, b)
        ev = next(e for e in evs if e["type"] == "resistance_changed")
        self.assertEqual(ev["key"], "1")
        self.assertEqual(ev["subject"], "GER")
        self.assertEqual(ev["object"], "FRA")
        self.assertEqual((ev["from"], ev["to"]), (22.3, 50.0))

    def test_resistance_subband_drift_does_not_emit(self):
        # a small drift WITHIN the same band must NOT fire (avoid daily-tick noise)
        a = _snap("1940.1.1.0", save_domains=["state_detail"])
        b = _snap("1940.1.2.0", save_domains=["state_detail"])
        self._state(a, "1", resistance=16.0)   # band idx 1
        self._state(b, "1", resistance=18.0)   # still band idx 1
        self.assertNotIn("resistance_changed",
                         _types(diff_engine.diff_snapshots(a, b)))

    def test_compliance_band_crossing_emits(self):
        a = _snap("1940.1.1.0", save_domains=["state_detail"])
        b = _snap("1940.1.2.0", save_domains=["state_detail"])
        self._state(a, "1", compliance=26.0)   # band idx 2 (>=25)
        self._state(b, "1", compliance=65.0)   # band idx 4 (>=60)
        evs = diff_engine.diff_snapshots(a, b)
        self.assertIn("compliance_changed", _types(evs))

    def test_occupation_law_change_emits(self):
        a = _snap("1940.1.1.0", save_domains=["occupation"])
        b = _snap("1940.1.2.0", save_domains=["occupation"])
        self._state(a, "1", law="military_governor_occupation")
        self._state(b, "1", law="harsh_quotas")
        evs = diff_engine.diff_snapshots(a, b)
        ev = next(e for e in evs if e["type"] == "occupation_law_changed")
        self.assertEqual((ev["from"], ev["to"]),
                         ("military_governor_occupation", "harsh_quotas"))

    def test_occupation_gated_when_not_captured(self):
        # neither state_detail nor occupation captured by b -> no fabricated events
        a = _snap("1940.1.1.0", save_domains=["state_detail", "occupation"])
        b = _snap("1940.1.2.0", save_domains=["focus"])
        self._state(a, "1", resistance=22.3, law="military_governor_occupation")
        for t in ("resistance_changed", "occupation_law_changed"):
            self.assertNotIn(t, _types(diff_engine.diff_snapshots(a, b)))

    def test_all_new_events_validate(self):
        a = _snap("1940.1.1.0", save_domains=["state_detail", "occupation"])
        b = _snap("1940.1.2.0", save_domains=["state_detail", "occupation"])
        self._state(a, "1", resistance=10.0, compliance=10.0, law="law_a")
        self._state(b, "1", resistance=50.0, compliance=70.0, law="law_b")
        for ev in diff_engine.diff_snapshots(a, b):
            self.assertEqual(schema.validate_event(ev), [], ev)

    def test_law_not_diffed_when_occupation_domain_absent(self):
        # Codex P1: a state_detail-only pair must NOT diff occupation_law (the law is
        # unmeasured there) -> no fabricated law disappearance. Resistance (which IS
        # state_detail-sourced) still diffs.
        a = _snap("1940.1.1.0", save_domains=["state_detail", "occupation"])
        b = _snap("1940.1.2.0", save_domains=["state_detail"])  # NO occupation
        self._state(a, "1", resistance=10.0, law="military_governor")
        self._state(b, "1", resistance=50.0, law=None)
        types = _types(diff_engine.diff_snapshots(a, b))
        self.assertNotIn("occupation_law_changed", types)
        self.assertIn("resistance_changed", types)  # state_detail-gated, still fires

    def test_resistance_not_diffed_when_state_detail_absent(self):
        # the inverse: an occupation-only pair must NOT diff resistance/compliance
        # (those are state_detail-sourced) but CAN diff the law.
        a = _snap("1940.1.1.0", save_domains=["state_detail", "occupation"])
        b = _snap("1940.1.2.0", save_domains=["occupation"])  # NO state_detail
        self._state(a, "1", resistance=10.0, law="law_a")
        self._state(b, "1", resistance=90.0, law="law_b")
        types = _types(diff_engine.diff_snapshots(a, b))
        self.assertNotIn("resistance_changed", types)
        self.assertIn("occupation_law_changed", types)

    def test_save_backed_event_source_is_S(self):
        # Codex P2: a diff both snapshots backed with the SAVE channel is honestly
        # source="S" (save-sourced), confidence="interval" -- never "M"/"exact".
        a = _snap("1940.1.1.0", save_domains=["occupation"])
        b = _snap("1940.1.2.0", save_domains=["occupation"])
        self._state(a, "1", law="law_a")
        self._state(b, "1", law="law_b")
        ev = next(e for e in diff_engine.diff_snapshots(a, b)
                  if e["type"] == "occupation_law_changed")
        self.assertEqual(ev["source"], "S")
        self.assertEqual(ev["confidence"], "interval")


class TestGovernmentChangedProvenance(unittest.TestCase):
    """Codex P1: government_changed is sourced from country_snapshot. Its event
    domain + gate + confidence must all agree, so a save-derived ruling-party change
    is never mislabeled memory-exact even when the live channel captured
    country_scalars (the OLD, wrong event-domain)."""

    def test_save_ruling_party_change_not_exact(self):
        a = schema.empty_snapshot("g", "1940.1.1.0")
        b = schema.empty_snapshot("g", "1940.1.2.0")
        for s in (a, b):
            # live captured country_scalars; ruling_party comes from save's
            # country_snapshot domain.
            s["provenance"]["memory"] = {"captured": True,
                                         "domains": ["country_scalars"]}
            s["provenance"]["save"] = {"captured": True,
                                       "domains": ["country_snapshot"],
                                       "fills_canonical_domains": True}
        ca = schema.empty_country(); ca["politics"]["ruling_party"] = "fascism"
        cb = schema.empty_country(); cb["politics"]["ruling_party"] = "communism"
        a["countries"]["GER"] = ca
        b["countries"]["GER"] = cb
        ev = next(e for e in diff_engine.diff_snapshots(a, b)
                  if e["type"] == "government_changed")
        # must NOT be mislabeled as memory-exact (the bug); it is save-interval.
        self.assertEqual(ev["confidence"], "interval")
        self.assertEqual(ev["source"], "S")


class TestProductionNullNotZero(unittest.TestCase):
    """Codex P2: a production line with produced=None is UNMEASURED, not 0.0; it must
    not seed a fabricated production_milestone."""

    def test_null_produced_skipped(self):
        self.assertEqual(diff_engine._gross_by_archetype(
            [{"archetype": "inf", "produced": None}]), {})
        # a measured 0.0 IS kept (it is a measured zero)
        self.assertEqual(diff_engine._gross_by_archetype(
            [{"archetype": "inf", "produced": 0.0}]), {"inf": 0.0})

    def test_no_milestone_from_null_old_produced(self):
        a = _snap("1940.1.1.0", save_domains=["production"])
        b = _snap("1940.1.2.0", save_domains=["production"])
        ca = schema.empty_country()
        ca["economy"]["_production_lines"] = [{"archetype": "inf", "produced": None}]
        cb = schema.empty_country()
        cb["economy"]["_production_lines"] = [{"archetype": "inf", "produced": 150.0}]
        a["countries"]["GER"] = ca
        b["countries"]["GER"] = cb
        evs = diff_engine.diff_snapshots(a, b, milestone_steps={"production": 100})
        # old produced was UNKNOWN (None), so no fabricated 0->150 milestone
        self.assertNotIn("production_milestone", _types(evs))

    def test_milestone_for_genuinely_new_archetype(self):
        # Codex P2 (round 2): an archetype genuinely ABSENT from a MEASURED old set is
        # a real 0 baseline -> a new line that has produced N DOES fire its first
        # milestone (the absence is measured, not unknown).
        a = _snap("1940.1.1.0", save_domains=["production"])
        b = _snap("1940.1.2.0", save_domains=["production"])
        ca = schema.empty_country()
        # old set is measured (inf has a real produced) but has NO 'tank' line
        ca["economy"]["_production_lines"] = [{"archetype": "inf", "produced": 50.0}]
        cb = schema.empty_country()
        cb["economy"]["_production_lines"] = [
            {"archetype": "inf", "produced": 50.0},
            {"archetype": "tank", "produced": 150.0}]
        a["countries"]["GER"] = ca
        b["countries"]["GER"] = cb
        evs = diff_engine.diff_snapshots(a, b, milestone_steps={"production": 100})
        ms = [e for e in evs if e["type"] == "production_milestone"]
        self.assertEqual(len(ms), 1)
        self.assertEqual(ms[0]["key"], "tank")
        self.assertEqual(ms[0]["from"], 0.0)  # genuinely-measured-absent -> real 0


# ---------------------------------------------------------------------------
# Real-oracle ingest
# ---------------------------------------------------------------------------
@unittest.skipUnless(HAVE_ORACLE, "real save_oracle/output (market/occupation) not present")
class TestRealIngest(unittest.TestCase):
    def setUp(self):
        self.snap = store.build_snapshot_from_oracle(ORACLE_OUT, "real")

    def test_validates(self):
        self.assertEqual(schema.validate_snapshot(self.snap), [])

    def test_market_populated(self):
        self.assertGreater(len(self.snap["diplomacy_world"]["trade_contracts"]), 0)
        # at least one participating tag carries a lend_lease entry
        n = sum(1 for c in self.snap["countries"].values()
                if c["diplomacy"]["lend_lease"])
        self.assertGreater(n, 0)

    def test_occupation_populated(self):
        n = sum(1 for s in self.snap["states"].values() if s.get("occupation_law"))
        self.assertGreater(n, 0)

    def test_self_diff_empty(self):
        self.assertEqual(
            diff_engine.diff_snapshots(self.snap, json.loads(json.dumps(self.snap))),
            [])

    def test_no_personal_path(self):
        blob = json.dumps(self.snap)
        for n in ("redacted", "OneDrive", "re_scratch", "source_path_redacted"):
            self.assertNotIn(n, blob, n)


if __name__ == "__main__":
    unittest.main()
