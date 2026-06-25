"""Tests for the SAVE-side deep domains folded into the snapshot contract:
commanders (characters.json), designs (designs.json), order_of_battle (oob.json),
and the deepened intel (intel.json).

Covers: schema presence/null-default, store ingestion (synthetic + real oracle),
the cross-ref joins are carried through, the new diff/event types, and the
no-fabrication gating (a missing file -> those fields null, domain omitted; a
self-diff emits nothing).
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
ORACLE_OUT = os.path.join(REPO, "save_oracle", "output")
HAVE_ORACLE = os.path.isfile(os.path.join(ORACLE_OUT, "characters.json"))


def _scratch_root():
    base = os.path.join(tempfile.gettempdir(), "re_scratch")
    os.makedirs(base, exist_ok=True)
    return tempfile.mkdtemp(prefix="integ_deep_", dir=base)


def _dump(root, name, obj):
    with open(os.path.join(root, name), "w", encoding="utf-8") as fh:
        json.dump(obj, fh)


def _make_oracle_dir(root, *, with_deep=True):
    """A tiny synthetic oracle output, optionally incl. the 4 deep files."""
    os.makedirs(root, exist_ok=True)
    _dump(root, "meta.json",
          {"source_filename": "/secret/path/SYN_melted.txt", "save_date": "1937.6.1.12",
           "save_version": "syn"})
    _dump(root, "country_snapshot.json",
          {"GER": {"stability": 0.6, "war_support": 0.5, "is_human": True,
                   "factories": {"civ": 20, "mil": 30, "dockyard": 5}}})
    if not with_deep:
        return root
    _dump(root, "characters.json", {
        "meta": {"source_path_redacted": "secret"},
        "countries": {"GER": {
            "leader": {"name": "GER_leader", "ideology": "nazism", "traits": ["t"],
                       "char_id": 1},
            "party_leaders": [{"party": "fascism", "name": "GER_leader"}],
            "ruling_party": "fascism",
            "generals": [{"name": "Gen A", "token": "GER_gen_a", "role": "general",
                          "level": 3, "unit_leader_id": 27,
                          "assignment": {"kind": "army", "name": "Army 1"}}],
            "field_marshals": [{"name": "FM B", "token": "GER_fm_b",
                                "role": "field_marshal", "level": 5,
                                "unit_leader_id": 9, "assignment": None}],
            "admirals": [{"name": "Adm C", "token": "GER_adm_c", "role": "admiral",
                          "level": 4, "unit_leader_id": 12, "assignment": None}],
            "advisors": [{"name": "Adv D", "slot": "army_chief", "ledger": "army"}],
            "counts": {"generals": 1, "field_marshals": 1, "admirals": 1,
                       "advisors": 1, "roster_size": 4},
        }}})
    _dump(root, "designs.json", {
        "meta": {"source_path_redacted": "secret"},
        "division_templates": [
            {"id": 2, "type": 52, "name": "Inf Div", "country": "GER",
             "role": None, "regiments": [{"type": "infantry", "x": 0, "y": 0}],
             "support": [], "n_regiments": 1, "n_support": 0}],
        "equipment_variants": {
            "100:70": {"id": 100, "type": 70, "archetype": "medium_tank_chassis",
                       "family": "land_vehicle", "name": "Panzer III",
                       "creator": "GER", "modules": {"main_armament": "gun"},
                       "design_team": [1, 79],
                       "design_team_mio": {"tag": "GER", "key": "krupp",
                                           "name": "Krupp", "size": 3}}},
        "countries": {"GER": {
            "mios": [{"key": "krupp", "id": 1, "type": 79, "name": "Krupp",
                      "level": 3, "funds": 10.0,
                      "assigned_equipment": [100, 70],
                      "assigned_equipment_archetype": "medium_tank_chassis"}],
            "scheduled_variants": [{"archetype": "light_cruiser_1",
                                    "name": "Konigsberg", "modules": {}}],
            "policies": {"mio_policy_x": {"cost": 25.0, "cooldown": 180}},
        }}})
    _dump(root, "oob.json", {
        "meta": {"source_path_redacted": "secret",
                 "runtime_only_note": "cached values"},
        "by_country": {"GER": {"armies": 1, "army_groups": 1, "fleets": 1,
                               "ships": 2, "air_wings": 1, "planes": 30,
                               "divisions_in_armies": 5}},
        "army_groups": [{"id": "49:53", "owner": "GER", "name": "AG 1",
                         "leader": "5:4713", "army_refs": ["46:53"],
                         "army_refs_resolved": 1}],
        "armies": [{"id": "46:53", "owner": "GER", "name": "Army 1",
                    "leader": "282:4713", "members": ["10:4713"],
                    "member_count": 1, "members_resolved": 1, "orders": []}],
        "fleets": [{"id": "4897:61", "owner": "GER", "name": "Fleet 1",
                    "leader": "319:4713",
                    "task_forces": [{"id": "218:61", "name": "TF 1",
                                     "ships": [{"id": "4:51",
                                                "definition": "light_cruiser",
                                                "equipment": [[1569, 70, 1.0]]}]}]}],
        "navy_theaters": [{"owner": "GER", "id": "1:68", "name": "Theater 1",
                           "fleet_refs": ["4897:61"]}],
        "air_wings": [{"id": "3:69", "owner": "GER", "name": "KG 1", "count": 30,
                       "plane_definition": "tac_bomber", "air_base": "100:65",
                       "state_id": 15, "equipment": [[1573, 70, 30.0]]}],
        "theatres": [{"owner": "GER", "id": "1:67", "area_states": [1, 2],
                      "n_armies": 1, "n_army_groups": 1}]})
    _dump(root, "intel.json", {
        "meta": {"source_path_redacted": "secret",
                 "peace_conference": {"active": False}},
        "countries": {"GER": {
            "agency_name": "Abwehr", "icon": "GFX_x", "is_created": True,
            "upgrade_progress": 0.5, "current_upgrade": "up_x",
            "upgrades": {"upgrade_passive_defense": 2},
            "capacity": {"max_operatives": 8, "usable_slots": 8, "defense": 2.85},
            "cryptology": {"targets": [{"target": "ENG", "decryption": True,
                                        "amount": 7175.0}],
                           "intel_source": {"country": "GER"}},
            "operatives": [{"op_id": 11387, "name": "Johann", "skill": 1,
                            "state": "on_operation", "operation_id": 29104,
                            "traits": ["operative_safe_cracker"],
                            "captured": False}],
            "recruitable_operatives": [{"op_id": 841, "name": "Ritter"}],
            "operations": {"priority": 1,
                           "running": [{"type": "operation_infiltrate", "op_id": 29104,
                                        "date": "1940.9.8.24", "duration": 146,
                                        "target": "ENG", "phases": [], "operatives": []}],
                           "finished": [{"type": "operation_done", "values": ["FRA", "1"]}]},
            "counts": {"operatives": 1, "recruitable": 1, "running_operations": 1,
                       "finished_operations": 1, "cryptology_targets": 1},
        }}})
    return root


# ---------------------------------------------------------------------------
# Schema presence / null defaults
# ---------------------------------------------------------------------------
class TestSchemaPresence(unittest.TestCase):
    def test_new_domains_present_and_empty(self):
        snap = schema.empty_snapshot("g", "1940.9.10.13")
        self.assertEqual(schema.validate_snapshot(snap), [])
        # top-level containers present
        self.assertIn("designs", snap)
        self.assertIn("order_of_battle", snap)
        self.assertEqual(snap["designs"]["division_templates"], [])
        self.assertEqual(snap["designs"]["equipment_variants"], {})
        for k in ("army_groups", "armies", "fleets", "air_wings"):
            self.assertEqual(snap["order_of_battle"][k], [])

    def test_country_commanders_and_mios_present(self):
        c = schema.empty_country()
        self.assertIn("commanders", c)
        self.assertIsNone(c["commanders"]["leader"])      # null until fed
        self.assertEqual(c["commanders"]["generals"], [])
        self.assertEqual(c["mios"], [])
        self.assertIsNone(c["order_of_battle"])           # null until fed

    def test_intel_deepened(self):
        c = schema.empty_country()
        for k in ("agency", "operatives", "recruitable_operatives", "operations",
                  "decryption", "peace_conference"):
            self.assertIn(k, c["intel"])
        self.assertIsNone(c["intel"]["agency"])

    def test_field_sources_added(self):
        self.assertEqual(schema.source_of("countries.GER.commanders.generals"), "S")
        self.assertEqual(schema.source_of("designs.equipment_variants"), "S")
        self.assertEqual(schema.source_of("order_of_battle.armies"), "S")
        self.assertEqual(schema.source_of("countries.GER.intel.agency"), "S")
        self.assertEqual(schema.source_of("countries.GER.mios"), "S")

    def test_new_event_types_in_enum(self):
        for t in ("commander_assigned", "army_formed", "army_disbanded",
                  "design_created", "mio_leveled", "operation_started",
                  "operation_completed"):
            self.assertIn(t, schema.EVENT_TYPES)

    def test_dynamic_tag_field_sources_resolve(self):
        # Codex integ-review finding 2: dynamic/released tags (D01, D02, ...) are
        # real in the save and must resolve through FIELD_SOURCES.
        self.assertEqual(schema.source_of("countries.D01.mios"), "S")
        self.assertEqual(schema.source_of("countries.D01.intel.agency"), "S")
        self.assertEqual(schema.source_of("countries.D75.commanders.generals"), "S")
        # static tags still resolve; a lowercase field name does NOT get normalized
        self.assertEqual(schema.source_of("countries.GER.politics.stability"), "M")
        self.assertIsNone(schema.source_of("countries.GER.nonsense.field"))

    def test_validator_enforces_new_country_fields(self):
        s = schema.empty_snapshot("g", "1940.1.1.0")
        s["countries"]["GER"] = schema.empty_country()
        del s["countries"]["GER"]["mios"]
        self.assertTrue(any("mios" in p for p in schema.validate_snapshot(s)))

    def test_validator_enforces_top_level_subkeys(self):
        s = schema.empty_snapshot("g", "1940.1.1.0")
        del s["designs"]["equipment_variants"]
        self.assertTrue(any("equipment_variants" in p
                            for p in schema.validate_snapshot(s)))
        s2 = schema.empty_snapshot("g", "1940.1.1.0")
        del s2["order_of_battle"]["armies"]
        self.assertTrue(any("order_of_battle.armies" in p
                            for p in schema.validate_snapshot(s2)))


class TestSameHourCoalesceNewDomains(unittest.TestCase):
    """Codex integ-review finding 1: same-hour merge must carry designs/oob, not drop
    one sibling while provenance unions both domains."""

    def test_complementary_designs_and_oob_both_survive(self):
        a = schema.empty_snapshot("g", "1940.1.2.0")
        a["designs"]["equipment_variants"] = {"1:70": {"name": "X"}}
        a["designs"]["division_templates"] = [{"id": 1}]
        b = schema.empty_snapshot("g", "1940.1.2.0")
        b["order_of_battle"]["armies"] = [{"id": "1:53", "owner": "GER"}]
        m = diff_engine._merge_two_snapshots(a, b)
        self.assertIn("1:70", m["designs"]["equipment_variants"])
        self.assertEqual(len(m["designs"]["division_templates"]), 1)
        self.assertEqual(len(m["order_of_battle"]["armies"]), 1)
        # reversed order keeps both too
        m2 = diff_engine._merge_two_snapshots(b, a)
        self.assertIn("1:70", m2["designs"]["equipment_variants"])
        self.assertEqual(len(m2["order_of_battle"]["armies"]), 1)

    def test_empty_later_does_not_wipe_real_designs_or_oob(self):
        a = schema.empty_snapshot("g", "1940.1.2.0")
        a["order_of_battle"]["armies"] = [{"id": "9:53"}]
        a["designs"]["equipment_variants"] = {"5:70": {"name": "Y"}}
        b = schema.empty_snapshot("g", "1940.1.2.0")  # empty skeleton
        m = diff_engine._merge_two_snapshots(a, b)
        self.assertEqual(len(m["order_of_battle"]["armies"]), 1)
        self.assertIn("5:70", m["designs"]["equipment_variants"])


# ---------------------------------------------------------------------------
# Store ingestion (synthetic)
# ---------------------------------------------------------------------------
class TestIngestSyntheticDeep(unittest.TestCase):
    def setUp(self):
        self.scratch = _scratch_root()
        self.oracle = _make_oracle_dir(os.path.join(self.scratch, "oracle"))

    def tearDown(self):
        shutil.rmtree(self.scratch, ignore_errors=True)

    def test_ingest_validates_with_deep(self):
        snap = store.build_snapshot_from_oracle(self.oracle, "syn")
        self.assertEqual(schema.validate_snapshot(snap), [])

    def test_commanders_ingested(self):
        snap = store.build_snapshot_from_oracle(self.oracle, "syn")
        cmd = snap["countries"]["GER"]["commanders"]
        self.assertEqual(cmd["leader"]["name"], "GER_leader")
        self.assertEqual(len(cmd["generals"]), 1)
        self.assertEqual(cmd["generals"][0]["assignment"]["name"], "Army 1")
        self.assertEqual(cmd["counts"]["generals"], 1)

    def test_designs_ingested_world_and_country(self):
        snap = store.build_snapshot_from_oracle(self.oracle, "syn")
        self.assertEqual(len(snap["designs"]["division_templates"]), 1)
        self.assertIn("100:70", snap["designs"]["equipment_variants"])
        ger = snap["countries"]["GER"]
        self.assertEqual(len(ger["mios"]), 1)
        self.assertEqual(ger["mios"][0]["level"], 3)
        self.assertEqual(len(ger["scheduled_variants"]), 1)
        self.assertIn("mio_policy_x", ger["mio_policies"])

    def test_design_team_join_carried_through(self):
        # the parser's design_team_mio cross-ref join must survive ingestion
        snap = store.build_snapshot_from_oracle(self.oracle, "syn")
        v = snap["designs"]["equipment_variants"]["100:70"]
        self.assertEqual(v["design_team_mio"]["key"], "krupp")

    def test_oob_ingested_with_joins(self):
        snap = store.build_snapshot_from_oracle(self.oracle, "syn")
        oob = snap["order_of_battle"]
        self.assertEqual(len(oob["armies"]), 1)
        self.assertEqual(oob["armies"][0]["members_resolved"], 1)
        self.assertEqual(oob["army_groups"][0]["army_refs_resolved"], 1)
        # per-country rollup mirrored onto the Country
        self.assertEqual(snap["countries"]["GER"]["order_of_battle"]["armies"], 1)

    def test_intel_ingested_and_normalized(self):
        snap = store.build_snapshot_from_oracle(self.oracle, "syn")
        intel = snap["countries"]["GER"]["intel"]
        self.assertEqual(intel["agency"]["name"], "Abwehr")
        self.assertEqual(intel["agency"]["capacity"]["max_operatives"], 8)
        self.assertEqual(len(intel["operatives"]), 1)
        self.assertEqual(intel["operations"]["running"][0]["op_id"], 29104)
        # cryptology -> decryption field
        self.assertEqual(intel["decryption"]["targets"][0]["target"], "ENG")
        self.assertEqual(intel["peace_conference"], {"active": False})

    def test_provenance_lists_new_domains(self):
        snap = store.build_snapshot_from_oracle(self.oracle, "syn")
        doms = snap["provenance"]["save"]["domains"]
        for d in ("commanders", "designs", "order_of_battle", "intel"):
            self.assertIn(d, doms)


# ---------------------------------------------------------------------------
# No-fabrication: a missing deep file -> those fields null, domain omitted
# ---------------------------------------------------------------------------
class TestMissingFilesNullSafe(unittest.TestCase):
    def setUp(self):
        self.scratch = _scratch_root()
        self.oracle = _make_oracle_dir(os.path.join(self.scratch, "oracle"),
                                       with_deep=False)

    def tearDown(self):
        shutil.rmtree(self.scratch, ignore_errors=True)

    def test_no_deep_files_stay_null(self):
        snap = store.build_snapshot_from_oracle(self.oracle, "syn")
        self.assertEqual(schema.validate_snapshot(snap), [])
        # containers still present (always-present rule), but empty/null
        self.assertEqual(snap["designs"]["division_templates"], [])
        self.assertEqual(snap["order_of_battle"]["armies"], [])
        ger = snap["countries"]["GER"]
        self.assertIsNone(ger["commanders"]["leader"])
        self.assertEqual(ger["mios"], [])
        self.assertIsNone(ger["intel"]["agency"])
        # provenance omits the deep domains
        doms = snap["provenance"]["save"]["domains"]
        for d in ("commanders", "designs", "order_of_battle", "intel"):
            self.assertNotIn(d, doms)


# ---------------------------------------------------------------------------
# Diff / event types
# ---------------------------------------------------------------------------
def _snap(date, *, save_domains=None):
    s = schema.empty_snapshot("g", date)
    if save_domains is not None:
        s["provenance"]["save"] = {"captured": True, "domains": save_domains,
                                   "source_filename": None, "save_date": date}
    return s


def _country(snap, tag):
    c = schema.empty_country()
    snap["countries"][tag] = c
    return c


def _types(evs):
    return [e["type"] for e in evs]


class TestDeepDiffEvents(unittest.TestCase):
    def test_commander_assigned(self):
        a = _snap("1940.1.1.0", save_domains=["commanders"])
        b = _snap("1940.1.2.0", save_domains=["commanders"])
        ca = _country(a, "GER"); cb = _country(b, "GER")
        ca["commanders"]["generals"] = [
            {"name": "Gen A", "token": "GER_a", "role": "general", "assignment": None}]
        cb["commanders"]["generals"] = [
            {"name": "Gen A", "token": "GER_a", "role": "general",
             "assignment": {"kind": "army", "name": "Army 3"}}]
        evs = diff_engine.diff_snapshots(a, b)
        self.assertIn("commander_assigned", _types(evs))
        ev = next(e for e in evs if e["type"] == "commander_assigned")
        self.assertEqual(ev["to"], "Army 3")
        self.assertEqual(ev["key"], "GER_a")

    def test_commander_to_reserve_not_emitted(self):
        # formation -> null (reserve) is intentionally not a commander_assigned event
        a = _snap("1940.1.1.0", save_domains=["commanders"])
        b = _snap("1940.1.2.0", save_domains=["commanders"])
        ca = _country(a, "GER"); cb = _country(b, "GER")
        ca["commanders"]["generals"] = [
            {"name": "G", "token": "GER_a", "assignment": {"kind": "army", "name": "A1"}}]
        cb["commanders"]["generals"] = [
            {"name": "G", "token": "GER_a", "assignment": None}]
        self.assertNotIn("commander_assigned", _types(diff_engine.diff_snapshots(a, b)))

    def test_army_formed_and_disbanded(self):
        a = _snap("1940.1.1.0", save_domains=["order_of_battle"])
        b = _snap("1940.1.2.0", save_domains=["order_of_battle"])
        a["order_of_battle"]["armies"] = [
            {"id": "1:53", "owner": "GER", "name": "Old Army"}]
        b["order_of_battle"]["armies"] = [
            {"id": "2:53", "owner": "GER", "name": "New Army", "member_count": 3}]
        evs = diff_engine.diff_snapshots(a, b)
        types = _types(evs)
        self.assertIn("army_formed", types)
        self.assertIn("army_disbanded", types)
        formed = next(e for e in evs if e["type"] == "army_formed")
        self.assertEqual(formed["key"], "2:53")
        self.assertEqual(formed["subject"], "GER")

    def test_mio_leveled(self):
        a = _snap("1940.1.1.0", save_domains=["designs"])
        b = _snap("1940.1.2.0", save_domains=["designs"])
        ca = _country(a, "GER"); cb = _country(b, "GER")
        ca["mios"] = [{"key": "krupp", "name": "Krupp", "level": 2}]
        cb["mios"] = [{"key": "krupp", "name": "Krupp", "level": 4}]
        evs = diff_engine.diff_snapshots(a, b)
        ev = next(e for e in evs if e["type"] == "mio_leveled")
        self.assertEqual((ev["from"], ev["to"]), (2, 4))
        self.assertEqual(ev["key"], "krupp")

    def test_design_created(self):
        a = _snap("1940.1.1.0", save_domains=["designs"])
        b = _snap("1940.1.2.0", save_domains=["designs"])
        a["designs"]["equipment_variants"] = {}
        b["designs"]["equipment_variants"] = {
            "5:70": {"id": 5, "type": 70, "archetype": "medium_tank_chassis",
                     "family": "land_vehicle", "name": "Panzer IV", "creator": "GER",
                     "modules": {"x": "y"}}}
        evs = diff_engine.diff_snapshots(a, b)
        ev = next(e for e in evs if e["type"] == "design_created")
        self.assertEqual(ev["subject"], "GER")
        self.assertEqual(ev["to"], "Panzer IV")

    def test_design_created_skips_stock_archetypes(self):
        # a new catalog entry with no custom name and no modules = stock baseline
        a = _snap("1940.1.1.0", save_domains=["designs"])
        b = _snap("1940.1.2.0", save_domains=["designs"])
        a["designs"]["equipment_variants"] = {}
        b["designs"]["equipment_variants"] = {
            "1:70": {"id": 1, "type": 70, "archetype": "convoy_1", "name": None,
                     "modules": {}, "creator": "GER"}}
        self.assertNotIn("design_created", _types(diff_engine.diff_snapshots(a, b)))

    def test_operation_started_and_completed(self):
        a = _snap("1940.1.1.0", save_domains=["intel"])
        b = _snap("1940.1.2.0", save_domains=["intel"])
        ca = _country(a, "GER"); cb = _country(b, "GER")
        ca["intel"]["operations"] = {"running": [
            {"op_id": 100, "type": "op_old", "target": "FRA"}]}
        cb["intel"]["operations"] = {"running": [
            {"op_id": 200, "type": "op_new", "target": "ENG", "date": "1940.1.2.0",
             "duration": 30}]}
        evs = diff_engine.diff_snapshots(a, b)
        types = _types(evs)
        self.assertIn("operation_started", types)
        self.assertIn("operation_completed", types)
        started = next(e for e in evs if e["type"] == "operation_started")
        self.assertEqual(started["key"], "200")
        self.assertEqual(started["object"], "ENG")
        done = next(e for e in evs if e["type"] == "operation_completed")
        self.assertEqual(done["key"], "100")

    def test_deep_domains_gated_when_not_captured(self):
        # b did NOT capture these domains -> no fabricated events from a's data.
        a = _snap("1940.1.1.0", save_domains=["commanders", "designs",
                                              "order_of_battle", "intel"])
        b = _snap("1940.1.2.0", save_domains=["focus"])
        ca = _country(a, "GER")
        ca["commanders"]["generals"] = [
            {"name": "G", "token": "GER_a", "assignment": {"name": "A1"}}]
        ca["mios"] = [{"key": "k", "level": 5}]
        ca["intel"]["operations"] = {"running": [{"op_id": 1, "type": "op"}]}
        a["order_of_battle"]["armies"] = [{"id": "1:53", "owner": "GER"}]
        a["designs"]["equipment_variants"] = {
            "1:70": {"name": "X", "modules": {"a": "b"}, "creator": "GER"}}
        evs = diff_engine.diff_snapshots(a, b)
        for t in ("commander_assigned", "army_formed", "army_disbanded",
                  "mio_leveled", "design_created", "operation_completed"):
            self.assertNotIn(t, _types(evs), t)

    def test_all_deep_events_validate(self):
        a = _snap("1940.1.1.0", save_domains=["commanders", "designs",
                                              "order_of_battle", "intel"])
        b = _snap("1940.1.2.0", save_domains=["commanders", "designs",
                                              "order_of_battle", "intel"])
        ca = _country(a, "GER"); cb = _country(b, "GER")
        ca["commanders"]["generals"] = [
            {"name": "G", "token": "GER_a", "assignment": None}]
        cb["commanders"]["generals"] = [
            {"name": "G", "token": "GER_a", "assignment": {"kind": "army", "name": "A1"}}]
        ca["mios"] = [{"key": "k", "name": "K", "level": 1}]
        cb["mios"] = [{"key": "k", "name": "K", "level": 2}]
        ca["intel"]["operations"] = {"running": []}
        cb["intel"]["operations"] = {"running": [{"op_id": 9, "type": "op", "target": "X"}]}
        b["order_of_battle"]["armies"] = [{"id": "1:53", "owner": "GER", "name": "A"}]
        b["designs"]["equipment_variants"] = {
            "1:70": {"name": "X", "modules": {"a": "b"}, "creator": "GER",
                     "archetype": "x", "family": "land_other"}}
        for ev in diff_engine.diff_snapshots(a, b):
            self.assertEqual(schema.validate_event(ev), [], ev)


# ---------------------------------------------------------------------------
# Real-oracle ingest (skipped when output not present)
# ---------------------------------------------------------------------------
@unittest.skipUnless(HAVE_ORACLE, "real save_oracle/output (deep files) not present")
class TestRealDeepIngest(unittest.TestCase):
    def setUp(self):
        self.snap = store.build_snapshot_from_oracle(ORACLE_OUT, "real")

    def test_validates(self):
        self.assertEqual(schema.validate_snapshot(self.snap), [])

    def test_domains_populated(self):
        self.assertGreater(len(self.snap["designs"]["division_templates"]), 100)
        self.assertGreater(len(self.snap["designs"]["equipment_variants"]), 100)
        self.assertGreater(len(self.snap["order_of_battle"]["armies"]), 10)
        self.assertGreater(len(self.snap["order_of_battle"]["air_wings"]), 10)
        ger = self.snap["countries"]["GER"]
        self.assertIsNotNone(ger["commanders"]["leader"])
        self.assertGreater(len(ger["commanders"]["generals"]), 0)
        self.assertIsNotNone(ger["intel"]["agency"]["name"])
        self.assertGreater(len(ger["mios"]), 0)

    def test_no_personal_path(self):
        blob = json.dumps(self.snap)
        for needle in ("Example", "OneDrive", "re_scratch", "Downloads",
                       "source_path_redacted"):
            self.assertNotIn(needle, blob, needle)
        self.assertNotIn("C:" + chr(92) + "Users", blob)

    def test_self_diff_is_empty(self):
        self.assertEqual(
            diff_engine.diff_snapshots(self.snap, json.loads(json.dumps(self.snap))),
            [])


if __name__ == "__main__":
    unittest.main()
