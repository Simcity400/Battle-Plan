"""Tests for the Tier-1 EXTRACT save-domain wiring into the replay snapshot:

Domains wired (schema field + store ingestion + assembler fold + diff event):
  * supply (supply.json)            -> countries.*.economy.logistics
  * strategic_air (strategic_air.json)         -> snap["strategic_air"]
  * strategic_operatives (...)      -> snap["strategic_operatives"].pools
  * raids (raids.json)              -> snap["raids"]
  * events (events.json)            -> snap["world_events"]  (+ event_fired/convoy_sunk)
  * world (world.json)              -> snap["world"]
  * country_extra (country_extra.json) -> countries.*.extra (+ nuke_count_changed/
                                          ace_gained/project_completed)
  * players (players.json)          -> countries.*.human_name
  * divisions_full (divisions_full.json) -> additive fields merged onto divisions[]

Coverage:
  * schema presence (domains / event types / source map / empty-skeleton validates),
  * store ingestion from BOTH the real oracle output (fixture) and synthetic dirs,
  * diff events fire only on a real change, gated on the source domain (no fabrication),
  * no-fabrication: missing files -> fields null/empty, domain omitted from provenance,
  * provenance honesty: save-derived data under provenance.save (never memory),
  * no personal-path leak.

Existing replay tests must still pass alongside this file.
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
# The real EXTRACT fixtures live in save_oracle/output (read-only).
HAVE_FIXTURES = all(
    os.path.isfile(os.path.join(ORACLE_OUT, f))
    for f in ("supply.json", "strategic_air.json", "strategic_operatives.json",
              "raids.json", "events.json", "world.json", "country_extra.json",
              "players.json", "divisions_full.json"))


def _scratch_root():
    base = os.path.join(tempfile.gettempdir(), "re_scratch")
    os.makedirs(base, exist_ok=True)
    return tempfile.mkdtemp(prefix="integ_extract_", dir=base)


def _dump(root, name, obj):
    with open(os.path.join(root, name), "w", encoding="utf-8") as fh:
        json.dump(obj, fh)


def _types(evs):
    return [e["type"] for e in evs]


def _snap(date, *, save_domains=None):
    s = schema.empty_snapshot("g", date)
    if save_domains is not None:
        s["provenance"]["save"] = {"captured": True, "domains": save_domains,
                                   "source_filename": None, "save_date": date,
                                   "fills_canonical_domains": True}
    return s


# ---------------------------------------------------------------------------
# Synthetic oracle dir builder (small but exercises every new domain).
# ---------------------------------------------------------------------------
def _make_oracle_dir(root, *, with_extract=True):
    os.makedirs(root, exist_ok=True)
    _dump(root, "meta.json",
          {"source_filename": "/secret/path/SYN_melted.txt",
           "source_path_redacted": "/secret/path/SYN_melted.txt",
           "save_date": "1937.6.1.12", "save_version": "syn"})
    _dump(root, "country_snapshot.json",
          {"GER": {"stability": 0.6}, "ENG": {"stability": 0.55}})
    # a base divisions.json so divisions_full has something to MERGE onto
    _dump(root, "divisions.json", {
        "divisions": [
            {"div_id": 100, "div_type": 4713, "owner": "GER",
             "location_province": 5, "template_id": 1, "template_type": 2,
             "template_name": "Inf", "organisation": 50.0, "strength": 1.0,
             "equipment": [{"archetype": "infantry_equipment_0", "amount": 100}]}]})
    if with_extract:
        _dump(root, "supply.json", {
            "meta": {"source_path_redacted": "secret"},
            "countries": {
                "GER": {"priority": 2, "buffer": 1.5, "node_count": 3,
                        "truck_count": 0,  # genuine measured zero -> must survive
                        "trucks": [{"equip_id": 1, "equip_type": 70,
                                    "count": 0, "damage": None}],
                        "node_motorization": [], "disrupted_supply": [],
                        "foreign_homebase_nodes": [],
                        "lost_trains_total": 9, "lost_railways": [0, 0]}}})
        _dump(root, "strategic_air.json", {
            "meta": {},
            "air_bases": [{"id": 98, "type": 65, "state": 1, "capacity": 200,
                           "level": 1, "countries": [
                               {"country": "GER", "capacity_penalty": 1.0}]}],
            "indices": {"air_theatre_index": 0, "air_group_index": 0},
            "per_country_region_blocks": {"GER": 4, "ENG": 60}})
        _dump(root, "strategic_operatives.json", {
            "meta": {},
            "operatives": {
                "0": {"subversive_activity_level": 0.0, "danger_level": 0.0,
                      "networks": []},
                "1": {"subversive_activity_level": 41.88, "danger_level": 0.0,
                      "networks": [{"target": "ENG", "vertices": 9}]}}})
        _dump(root, "raids.json", {
            "meta": {},
            "counters": {"next_state": 977, "existing_target_num": 2400},
            "targets": [{"existing_target_num": 0, "valid": True, "dynamic": False,
                         "target_province": 11420, "target_building": None,
                         "target_location": None}],
            "countries": [{"tag": "GER", "priority": 1}]})
        _dump(root, "events.json", {
            "meta": {},
            "fired_event_names": ["AUS_political_events.11", "evt.foo"],
            "pending_events": [],
            "threat": [{"threat": 9.47, "final_threat": 9.47, "daily": 0.0,
                        "tag": "GER", "date": "1938.4.14.3", "label": "Anschluss"}],
            "sunk_convoys_history": [{"month": 23334, "convoys": 2,
                                      "killer_country": "FRA", "owner": "JAP"}],
            "convoy_losses_by_killer": {"FRA": 409},
            "convoy_losses_by_owner": {"JAP": 467}})
        _dump(root, "world.json", {
            "meta": {},
            "scenario": {"start_date": "1936.1.1.12", "difficulty": "normal",
                         "dlcs": "257535", "mods": ["AMSB - DEV"],
                         "ideology": "communism", "game_rules_empty": True,
                         "difficulty_settings_empty": True},
            "world_tension_inputs": {"average_major_ic": 456.14,
                                     "tension_scaling_base_country": "ENG"},
            "tech_sharing_groups": [{"id": "commonwealth_research",
                                     "bonuses": [0.0, 0.1],
                                     "countries": ["ENG", "AST"]}],
            "faction_system": [{"key": None, "index": 1, "initiative": 0.17,
                                "contribution": -32.76, "member_upgrades": []}],
            "power_balances": [{"template": "DEN_balance_of_power", "value": -0.35,
                                "left_side": "DEN_welfare_side",
                                "right_side": "DEN_warfare_side",
                                "countries": ["DEN"]}],
            "variables": {"allies_score": 11.0, "axis_score": 2.0},
            "flags": {"GER_has_started_war": {"value": 1, "date": "1939.9.2.1"}},
            "saved_event_targets": [{"country": "CHI", "state": None,
                                     "name": "WTT_current_china_leader"}]})
        _dump(root, "country_extra.json", {
            "meta": {},
            "countries": {
                "GER": {"nukes_total": 0, "nukes_ready_total": 0,
                        "nukes": [{"amount": 0, "nukes_ready": 0}],
                        "dynamic_modifiers": [
                            {"modifier": "GER_x", "enabled": True,
                             "value": [0.1]}],
                        "ace_count": 1,
                        "aces": [{"name": "Gunther", "surname": "Strohmann",
                                  "callsign": "Bubi", "modifier": "fighter_good",
                                  "alive": False, "kill_type": 1,
                                  "killer_country": "ENG"}],
                        "reinforcement_priority": 2,
                        "deployment_blocks": ["default_hq_template"],
                        "special_projects": {"projects_started": 5,
                                             "projects_completed": 5,
                                             "active_programs": [
                                                 {"building": "air_facility",
                                                  "province": 9388,
                                                  "country": "GER"}],
                                             "breakthrough": {
                                                 "specialization_land": 38528}},
                        "experience_status": {"army_experience": 999.0,
                                              "navy_experience": 751.79,
                                              "air_experience": 999.0},
                        "script_flags": {
                            "GER_has_started_war": {
                                "value": 1, "date": "1939.9.2.1"}},
                        "convoys_destroyed": 57, "accidents_score": 0.84,
                        "cosmetic_tag": "", "pride_of_the_fleet": {"id": 2,
                                                                   "type": 51},
                        "collaboration_present": False}}})
        _dump(root, "players.json", {
            "meta": {},
            "players": {"GER": {"user": "WooooWoooo", "country_leader": True,
                                "id": 1}}})
        _dump(root, "divisions_full.json", {
            "meta": {},
            "divisions": [
                {"div_id": 100, "div_type": 4713, "owner": "GER",
                 "location_province": 5, "script_id": -1,
                 "template_id": 1, "template_type": 2,
                 "division_name": {"type": 0, "name_order": 1029},
                 "organisation": 50.0, "strength": 1.0,
                 "experience": 356.55, "max_supply": 108.0,
                 "army_current_supply_ratio": 108.0,
                 "supply_gain": 0.0,  # GENUINE zero -> must survive
                 "motorization_level": 0, "move_priority": "front_order",
                 "last_combat_date": "1941.6.18.8",
                 "dig_in": 12.18, "dig_in_cap": 12.18,
                 "acclimatization": {"hot_climate": 6300.0,
                                     "max_acclimatization": 6300.0},
                 "army_manpower": {"value": 6300, "need": 6300},
                 "killed": 5091, "killer_definition": "no",
                 "equipment_archetypes": 3, "has_officer": True},
                # a division present ONLY in divisions_full (no divisions.json row)
                {"div_id": 200, "div_type": 4713, "owner": "ENG",
                 "location_province": 9, "template_id": 7, "template_type": 8,
                 "organisation": 30.0, "strength": 0.5,
                 "max_supply": 80.0, "dig_in": 0.0}]})
    return root


# ---------------------------------------------------------------------------
# Schema presence
# ---------------------------------------------------------------------------
class TestSchemaPresence(unittest.TestCase):
    def test_domains_and_event_types(self):
        for d in ("supply_logistics", "strategic_air", "strategic_operatives",
                  "raids", "world_events", "world", "country_extra", "players"):
            self.assertIn(d, schema.DOMAINS, d)
        for t in ("event_fired", "convoy_sunk", "nuke_count_changed",
                  "ace_gained", "project_completed"):
            self.assertIn(t, schema.EVENT_TYPES, t)

    def test_empty_snapshot_has_new_containers_and_validates(self):
        s = schema.empty_snapshot("g", "1940.1.1.0")
        self.assertEqual(schema.validate_snapshot(s), [])
        for k in ("strategic_air", "strategic_operatives", "raids",
                  "world_events", "world"):
            self.assertIn(k, s)
            self.assertIsInstance(s[k], dict)
        # null-but-present leaves
        self.assertIsNone(s["world"]["scenario"])
        self.assertEqual(s["strategic_air"]["air_bases"], [])
        self.assertEqual(s["world_events"]["fired_events"], [])
        # per-country additive fields exist on the empty skeleton
        c = schema.empty_country()
        self.assertIn("extra", c)
        self.assertIn("human_name", c)
        self.assertIsNone(c["extra"])
        self.assertIsNone(c["human_name"])
        self.assertIn("logistics", c["economy"])
        # division additive fields exist
        d = schema.empty_division()
        for f in ("dig_in", "supply_gain", "killed", "acclimatization",
                  "army_manpower", "division_name", "max_supply"):
            self.assertIn(f, d)

    def test_field_sources_all_S(self):
        self.assertEqual(schema.source_of("countries.GER.economy.logistics"), "S")
        self.assertEqual(schema.source_of("countries.GER.extra"), "S")
        self.assertEqual(schema.source_of("countries.GER.extra.script_flags"), "S")
        self.assertEqual(schema.source_of("countries.GER.human_name"), "S")
        self.assertEqual(schema.source_of("countries.GER.research.doctrine"), "S/D")
        self.assertEqual(schema.source_of("strategic_air.air_bases"), "S")
        self.assertEqual(schema.source_of("strategic_operatives.pools"), "S")
        self.assertEqual(schema.source_of("raids.targets"), "S")
        self.assertEqual(schema.source_of("world_events.fired_events"), "S")
        self.assertEqual(schema.source_of("world.scenario"), "S")
        self.assertEqual(schema.source_of(
            "world.scenario.difficulty_settings_empty"), "S")
        self.assertEqual(schema.source_of("divisions.5.dig_in"), "S")
        # dynamic released tag resolves too
        self.assertEqual(schema.source_of("countries.D05.extra"), "S")


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

    def test_supply_logistics_on_country(self):
        snap = store.build_snapshot_from_oracle(self.oracle, "syn")
        log = snap["countries"]["GER"]["economy"]["logistics"]
        self.assertEqual(log["node_count"], 3)
        self.assertEqual(log["lost_trains_total"], 9)
        # genuine measured zero preserved (no-fabrication)
        self.assertEqual(log["truck_count"], 0)

    def test_strategic_air(self):
        snap = store.build_snapshot_from_oracle(self.oracle, "syn")
        sa = snap["strategic_air"]
        self.assertEqual(len(sa["air_bases"]), 1)
        self.assertEqual(sa["indices"]["air_theatre_index"], 0)
        self.assertEqual(sa["per_country_region_blocks"]["GER"], 4)

    def test_strategic_operatives(self):
        snap = store.build_snapshot_from_oracle(self.oracle, "syn")
        pools = snap["strategic_operatives"]["pools"]
        self.assertEqual(len(pools), 2)
        self.assertEqual(pools["1"]["networks"][0]["target"], "ENG")

    def test_raids(self):
        snap = store.build_snapshot_from_oracle(self.oracle, "syn")
        rd = snap["raids"]
        self.assertEqual(rd["counters"]["next_state"], 977)
        self.assertEqual(len(rd["targets"]), 1)
        self.assertEqual(rd["countries"][0]["tag"], "GER")

    def test_world_events(self):
        snap = store.build_snapshot_from_oracle(self.oracle, "syn")
        we = snap["world_events"]
        self.assertIn("evt.foo", we["fired_events"])
        self.assertEqual(len(we["threat"]), 1)
        self.assertEqual(we["sunk_convoys"][0]["convoys"], 2)
        self.assertEqual(we["convoy_losses_by_killer"]["FRA"], 409)

    def test_world(self):
        snap = store.build_snapshot_from_oracle(self.oracle, "syn")
        w = snap["world"]
        self.assertEqual(w["scenario"]["start_date"], "1936.1.1.12")
        self.assertTrue(w["scenario"]["difficulty_settings_empty"])
        self.assertEqual(w["variables"]["axis_score"], 2.0)
        self.assertEqual(w["flags"]["GER_has_started_war"]["value"], 1)
        self.assertEqual(len(w["tech_sharing_groups"]), 1)
        self.assertEqual(len(w["power_balances"]), 1)
        # parser key faction_system surfaced under faction_influence
        self.assertEqual(len(w["faction_influence"]), 1)
        self.assertEqual(w["world_tension_inputs"]["tension_scaling_base_country"],
                         "ENG")

    def test_country_extra(self):
        snap = store.build_snapshot_from_oracle(self.oracle, "syn")
        ext = snap["countries"]["GER"]["extra"]
        self.assertEqual(ext["ace_count"], 1)
        self.assertEqual(ext["nukes_total"], 0)  # genuine zero
        self.assertEqual(ext["special_projects"]["projects_completed"], 5)
        self.assertEqual(ext["convoys_destroyed"], 57)
        self.assertEqual(ext["script_flags"]["GER_has_started_war"]["value"], 1)
        self.assertIn("dynamic_modifiers", ext)

    def test_players_human_name(self):
        snap = store.build_snapshot_from_oracle(self.oracle, "syn")
        self.assertEqual(snap["countries"]["GER"]["human_name"], "WooooWoooo")
        # ENG is not a player -> human_name stays null (no fabrication)
        self.assertIsNone(snap["countries"]["ENG"]["human_name"])

    def test_divisions_full_merge(self):
        snap = store.build_snapshot_from_oracle(self.oracle, "syn")
        by_id = {d["div_id"]: d for d in snap["divisions"]}
        d100 = by_id[100]
        # merged additive fields present...
        self.assertEqual(d100["dig_in"], 12.18)
        self.assertEqual(d100["killed"], 5091)
        self.assertEqual(d100["division_name"]["name_order"], 1029)
        # ...while the original divisions.json equipment is NOT lost
        self.assertTrue(d100["equipment"])
        self.assertEqual(d100["equipment"][0]["archetype"], "infantry_equipment_0")
        # genuine zero (supply_gain=0.0) survives the merge
        self.assertEqual(d100["supply_gain"], 0.0)
        # a division present only in divisions_full is added
        self.assertIn(200, by_id)
        self.assertEqual(by_id[200]["owner"], "ENG")
        self.assertEqual(by_id[200]["max_supply"], 80.0)
        # division_count recounts to include the appended one
        self.assertEqual(snap["countries"]["ENG"]["military"]["division_count"], 1)

    def test_provenance_lists_all_new_domains(self):
        snap = store.build_snapshot_from_oracle(self.oracle, "syn")
        sv = snap["provenance"]["save"]
        doms = sv["domains"]
        for d in ("supply_logistics", "strategic_air", "strategic_operatives",
                  "raids", "world_events", "world", "country_extra", "players"):
            self.assertIn(d, doms, d)
        # provenance lives in the SAVE channel, memory untouched
        self.assertIsNone(snap["provenance"]["memory"])

    def test_no_personal_path(self):
        snap = store.build_snapshot_from_oracle(self.oracle, "syn")
        blob = json.dumps(snap)
        for n in ("/secret/path", "source_path_redacted"):
            self.assertNotIn(n, blob, n)


# ---------------------------------------------------------------------------
# No-fabrication: missing EXTRACT files
# ---------------------------------------------------------------------------
class TestMissingFiles(unittest.TestCase):
    def setUp(self):
        self.scratch = _scratch_root()
        self.oracle = _make_oracle_dir(os.path.join(self.scratch, "oracle"),
                                       with_extract=False)

    def tearDown(self):
        shutil.rmtree(self.scratch, ignore_errors=True)

    def test_missing_extract_null_safe(self):
        snap = store.build_snapshot_from_oracle(self.oracle, "syn")
        self.assertEqual(schema.validate_snapshot(snap), [])
        # new top-level domains stay at the empty skeleton
        self.assertIsNone(snap["world"]["scenario"])
        self.assertEqual(snap["strategic_air"]["air_bases"], [])
        self.assertIsNone(snap["strategic_operatives"]["pools"])
        # per-country additive fields stay null
        self.assertIsNone(snap["countries"]["GER"]["economy"]["logistics"])
        self.assertIsNone(snap["countries"]["GER"]["extra"])
        self.assertIsNone(snap["countries"]["GER"]["human_name"])
        # divisions.json division still present but WITHOUT the divisions_full body
        d = next(d for d in snap["divisions"] if d["div_id"] == 100)
        self.assertIsNone(d["dig_in"])
        self.assertIsNone(d["max_supply"])
        # provenance omits the EXTRACT domains
        doms = snap["provenance"]["save"]["domains"]
        for d in ("supply_logistics", "strategic_air", "world", "country_extra",
                  "players"):
            self.assertNotIn(d, doms, d)


# ---------------------------------------------------------------------------
# Diff events: world_events (event_fired / convoy_sunk)
# ---------------------------------------------------------------------------
class TestWorldEventsDiff(unittest.TestCase):
    def test_event_fired_and_convoy_sunk(self):
        a = _snap("1940.1.1.0", save_domains=["world_events"])
        b = _snap("1940.1.2.0", save_domains=["world_events"])
        a["world_events"]["fired_events"] = ["e1", "e2"]
        b["world_events"]["fired_events"] = ["e1", "e2", "e3"]
        a["world_events"]["sunk_convoys"] = [
            {"month": 100, "convoys": 2, "killer_country": "GER", "owner": "ENG"}]
        b["world_events"]["sunk_convoys"] = [
            {"month": 100, "convoys": 5, "killer_country": "GER", "owner": "ENG"},
            {"month": 101, "convoys": 1, "killer_country": "GER", "owner": "USA"}]
        evs = diff_engine.diff_snapshots(a, b)
        fired = next(e for e in evs if e["type"] == "event_fired")
        self.assertEqual(fired["key"], "e3")
        convs = [e for e in evs if e["type"] == "convoy_sunk"]
        # one for the grown tally, one for the new entry
        self.assertEqual(len(convs), 2)
        grew = next(e for e in convs if e["key"] == "100:GER:ENG")
        self.assertEqual((grew["from"], grew["to"]), (2, 5))
        new = next(e for e in convs if e["key"] == "101:GER:USA")
        self.assertIsNone(new["from"])
        self.assertEqual(new["to"], 1)
        self.assertEqual(new["subject"], "GER")
        self.assertEqual(new["object"], "USA")

    def test_no_event_when_unchanged(self):
        a = _snap("1940.1.1.0", save_domains=["world_events"])
        b = _snap("1940.1.2.0", save_domains=["world_events"])
        for s in (a, b):
            s["world_events"]["fired_events"] = ["e1"]
            s["world_events"]["sunk_convoys"] = [
                {"month": 100, "convoys": 2, "killer_country": "GER",
                 "owner": "ENG"}]
        self.assertEqual([e for e in _types(diff_engine.diff_snapshots(a, b))
                          if e in ("event_fired", "convoy_sunk")], [])

    def test_gated_when_not_captured(self):
        a = _snap("1940.1.1.0", save_domains=["world_events"])
        b = _snap("1940.1.2.0", save_domains=["focus"])  # b lacks world_events
        a["world_events"]["fired_events"] = ["e1"]
        b["world_events"]["fired_events"] = ["e1", "e2"]
        self.assertNotIn("event_fired", _types(diff_engine.diff_snapshots(a, b)))

    def test_event_source_is_S(self):
        a = _snap("1940.1.1.0", save_domains=["world_events"])
        b = _snap("1940.1.2.0", save_domains=["world_events"])
        a["world_events"]["fired_events"] = []
        b["world_events"]["fired_events"] = ["e1"]
        ev = next(e for e in diff_engine.diff_snapshots(a, b)
                  if e["type"] == "event_fired")
        self.assertEqual(ev["source"], "S")
        self.assertEqual(ev["confidence"], "interval")


# ---------------------------------------------------------------------------
# Diff events: country_extra (nuke_count_changed / ace_gained / project_completed)
# ---------------------------------------------------------------------------
class TestCountryExtraDiff(unittest.TestCase):
    def _with_extra(self, snap, tag, extra):
        c = schema.empty_country()
        c["extra"] = extra
        snap["countries"][tag] = c

    def test_all_three_events(self):
        a = _snap("1940.1.1.0", save_domains=["country_extra"])
        b = _snap("1940.1.2.0", save_domains=["country_extra"])
        self._with_extra(a, "GER", {
            "nukes_total": 0,
            "aces": [{"name": "A", "surname": "B", "callsign": "X"}],
            "special_projects": {"projects_completed": 3}})
        self._with_extra(b, "GER", {
            "nukes_total": 2,
            "aces": [{"name": "A", "surname": "B", "callsign": "X"},
                     {"name": "C", "surname": "D", "callsign": "Y",
                      "modifier": "fighter_unique"}],
            "special_projects": {"projects_completed": 5}})
        evs = diff_engine.diff_snapshots(a, b)
        types = _types(evs)
        self.assertIn("nuke_count_changed", types)
        self.assertIn("ace_gained", types)
        self.assertIn("project_completed", types)
        nuke = next(e for e in evs if e["type"] == "nuke_count_changed")
        self.assertEqual((nuke["from"], nuke["to"]), (0, 2))
        ace = next(e for e in evs if e["type"] == "ace_gained")
        self.assertEqual(ace["key"], "C|D|Y")
        self.assertEqual(ace["detail"]["modifier"], "fighter_unique")
        proj = next(e for e in evs if e["type"] == "project_completed")
        self.assertEqual(proj["detail"]["completed_delta"], 2)

    def test_no_event_when_unchanged(self):
        a = _snap("1940.1.1.0", save_domains=["country_extra"])
        b = _snap("1940.1.2.0", save_domains=["country_extra"])
        ext = {"nukes_total": 0,
               "aces": [{"name": "A", "surname": "B", "callsign": "X"}],
               "special_projects": {"projects_completed": 3}}
        self._with_extra(a, "GER", dict(ext))
        self._with_extra(b, "GER", dict(ext))
        self.assertEqual([t for t in _types(diff_engine.diff_snapshots(a, b))
                          if t in ("nuke_count_changed", "ace_gained",
                                   "project_completed")], [])

    def test_project_completed_only_on_increase(self):
        # a DECREASE (rollback / civil war) must not emit a completion
        a = _snap("1940.1.1.0", save_domains=["country_extra"])
        b = _snap("1940.1.2.0", save_domains=["country_extra"])
        self._with_extra(a, "GER", {"special_projects": {"projects_completed": 5}})
        self._with_extra(b, "GER", {"special_projects": {"projects_completed": 3}})
        self.assertNotIn("project_completed",
                         _types(diff_engine.diff_snapshots(a, b)))

    def test_gated_when_not_captured(self):
        a = _snap("1940.1.1.0", save_domains=["country_extra"])
        b = _snap("1940.1.2.0", save_domains=["focus"])  # b lacks country_extra
        self._with_extra(a, "GER", {"nukes_total": 0})
        self._with_extra(b, "GER", {"nukes_total": 5})
        self.assertNotIn("nuke_count_changed",
                         _types(diff_engine.diff_snapshots(a, b)))

    def test_all_events_validate(self):
        a = _snap("1940.1.1.0", save_domains=["country_extra", "world_events"])
        b = _snap("1940.1.2.0", save_domains=["country_extra", "world_events"])
        self._with_extra(a, "GER", {"nukes_total": 0, "aces": [],
                                    "special_projects": {"projects_completed": 0}})
        self._with_extra(b, "GER", {"nukes_total": 3,
                                    "aces": [{"name": "C", "surname": "D",
                                              "callsign": "Y"}],
                                    "special_projects": {"projects_completed": 2}})
        b["world_events"]["fired_events"] = ["evt.1"]
        b["world_events"]["sunk_convoys"] = [{"month": 1, "convoys": 1,
                                              "killer_country": "GER",
                                              "owner": "ENG"}]
        for ev in diff_engine.diff_snapshots(a, b):
            self.assertEqual(schema.validate_event(ev), [], ev)


# ---------------------------------------------------------------------------
# No-fabrication (synthetic, channel-level): a real 0 stays 0; null iff unmeasured.
# ---------------------------------------------------------------------------
class TestNoFabrication(unittest.TestCase):
    def setUp(self):
        self.scratch = _scratch_root()

    def tearDown(self):
        shutil.rmtree(self.scratch, ignore_errors=True)

    def test_genuine_zero_preserved_null_for_unmeasured(self):
        oracle = _make_oracle_dir(os.path.join(self.scratch, "oracle"))
        snap = store.build_snapshot_from_oracle(oracle, "syn")
        # genuine zeros from the EXTRACT data survive (not coerced to null)
        self.assertEqual(
            snap["countries"]["GER"]["economy"]["logistics"]["truck_count"], 0)
        self.assertEqual(snap["countries"]["GER"]["extra"]["nukes_total"], 0)
        d100 = next(d for d in snap["divisions"] if d["div_id"] == 100)
        self.assertEqual(d100["supply_gain"], 0.0)
        # a field the parser never emitted (e.g. 'capital' absent here) is absent in
        # logistics, NOT fabricated to 0/None inside the block
        self.assertNotIn("capital", snap["countries"]["GER"]["economy"]["logistics"])


# ---------------------------------------------------------------------------
# Fixture-based tests against the REAL save_oracle/output EXTRACT files.
# ---------------------------------------------------------------------------
@unittest.skipUnless(HAVE_FIXTURES, "EXTRACT fixtures not present in save_oracle/output")
class TestRealFixtures(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.snap = store.build_snapshot_from_oracle(ORACLE_OUT, "ger1946_fixture")

    def test_validates(self):
        self.assertEqual(schema.validate_snapshot(self.snap), [])

    def test_real_counts_match_known_extract(self):
        s = self.snap
        # numbers the EXTRACT verifier confirmed (ger1946_melt.txt)
        self.assertEqual(len(s["strategic_air"]["air_bases"]), 337)
        self.assertEqual(len(s["strategic_operatives"]["pools"]), 144)
        self.assertEqual(len(s["raids"]["targets"]), 2404)
        self.assertEqual(len(s["world_events"]["fired_events"]), 109)
        self.assertEqual(len(s["world_events"]["threat"]), 43)
        self.assertEqual(len(s["world_events"]["sunk_convoys"]), 199)
        self.assertEqual(len(s["world"]["variables"]), 260)
        self.assertEqual(len(s["world"]["flags"]), 74)
        self.assertEqual(len(s["world"]["tech_sharing_groups"]), 6)

    def test_real_ger_extra_and_player(self):
        ger = self.snap["countries"]["GER"]
        self.assertEqual(ger["human_name"], "WooooWoooo")
        self.assertEqual(ger["extra"]["ace_count"], 30)
        # GER 2 nuke RECORDS both amount=0 -> nukes_total=0 (genuine zero, verifier)
        self.assertEqual(ger["extra"]["nukes_total"], 0)
        self.assertEqual(ger["economy"]["logistics"]["node_count"], 87)

    def test_real_divisions_full_merged_all(self):
        divs = self.snap["divisions"]
        # every division got the additive body merged (3316 in this save)
        with_full = sum(1 for d in divs if d.get("max_supply") is not None)
        self.assertEqual(with_full, len(divs))
        # equipment (from divisions.json) is NOT lost by the merge
        self.assertTrue(any(d.get("equipment") for d in divs))
        # genuine supply_gain=0.0 zeros exist and survive (verifier: 788 in melt)
        self.assertTrue(any(d.get("supply_gain") == 0.0 for d in divs))

    def test_real_provenance_save_only(self):
        sv = self.snap["provenance"]["save"]
        for d in ("supply_logistics", "strategic_air", "strategic_operatives",
                  "raids", "world_events", "world", "country_extra", "players"):
            self.assertIn(d, sv["domains"], d)
        self.assertIsNone(self.snap["provenance"]["memory"])

    def test_real_no_personal_path(self):
        blob = json.dumps(self.snap)
        for n in ("redacted", "OneDrive", "source_path_redacted", "AppData"):
            self.assertNotIn(n, blob, n)


if __name__ == "__main__":
    unittest.main()
