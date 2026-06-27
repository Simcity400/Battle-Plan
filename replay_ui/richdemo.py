"""RICH synthetic snapshot store for the replay UI (offline, schema-valid).

The bundled ``demo.py`` (3 tiny snapshots) exercises the *no-fabrication* edge
cases but is deliberately sparse. The real ``campaign`` store is rich but PREDATES
several schema domains (``extra``/``special_projects``, ``human_name``, top-level
``naval_air``/``supply``/``world``/…), so it cannot demonstrate that the UI surfaces
those *new* domains.

This module builds a **rich, schema-valid, multi-frame** store under
``replay_ui/demo_snapshots/richdemo/`` by starting from the canonical
``schema.empty_snapshot()`` / ``empty_country()`` (the latest shape — so
``validate_snapshot()`` returns **zero** errors) and populating realistic values
across **every** domain a screen touches:

  - economy incl. ``fuel.current/max`` and a construction queue,
  - politics: laws triple, ruling_party, party_popularities, ideas, faction,
  - research: ``in_progress`` with raw tokens (the UI humanizes them) + bonuses,
  - focus: tree / current / progress / completed,
  - military: num_ships, division_count, casualties (a real one + an empty one),
  - intel.agency (name / upgrades / capacity),
  - commanders (leader + generals/admirals/advisors, with ``portrait_gfx`` handles
    that resolve to the real extracted PNGs under ``game_assets/assets/portraits``),
  - ``extra.special_projects`` / nukes / aces (the *new* domain),
  - ``human_name`` (the MP persona, a *new* domain),
  - top-level ``order_of_battle`` (army groups → armies → fleets → air wings),
  - top-level ``designs`` (templates + equipment-variant catalog),
  - ``diplomacy_world.wars`` with casualties + factions,
  - the PROXY sidecars ``naval_air`` / ``supply`` with their own
    ``source``/``confidence`` (``region_proxy`` / ``division_province_proxy`` — never
    ``exact``),
  - ``world`` scenario meta, and a handful of ``combats``.

It deliberately keeps the no-fabrication contract visible: one country's
``casualties`` is a real measured object, another's stays ``{}`` (not captured);
``party_popularities`` is present for the majors and ``null`` for a minor; the
PROXY domains keep a non-``exact`` confidence label.

Pure stdlib, READ-ONLY on ``replay/schema.py`` and ``game_assets/``; writes only
under ``replay_ui/demo_snapshots/`` (gitignored). The timeline is produced by the
**real** diff engine over the generated frames (no faked events).
"""
from __future__ import annotations

import json
import os
import sys
from typing import Any, Dict, List, Optional

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
if REPO not in sys.path:
    sys.path.insert(0, REPO)

from replay.schema import (  # noqa: E402
    empty_snapshot, empty_country, empty_order_of_battle, validate_snapshot,
)

RICH_GAME_ID = "richdemo"
OUT_DIR = os.path.join(HERE, "demo_snapshots", RICH_GAME_ID)

# Portrait handles that exist in the real extracted set (game_assets/assets/portraits)
# so the UI's portrait wiring lights up on real PNGs, not 404s. These are vanilla
# leaders present at >99% coverage in portraits.json.
_HITLER = "GFX_portrait_GER_adolf_hitler"
_STALIN = "GFX_portrait_SOV_joseph_stalin"


def _frame(date: str, hour_index: int, *, with_memory: bool) -> Dict[str, Any]:
    """One fully-populated, schema-valid snapshot."""
    snap = empty_snapshot(RICH_GAME_ID, date)
    snap["hour_index"] = hour_index
    snap["save_version"] = "Operation Postern v1.19.1.0.b49d (d4f6)"

    # ---- provenance: save always, memory on the later frame (so the UI shows
    # the channel flips and the M*/proxy domains light up) -------------------
    save_domains = [
        "country_snapshot", "economy", "production", "tech", "focus", "diplomacy",
        "state_ownership", "state_control", "state_detail", "province_control",
        "divisions", "combats", "commanders", "designs", "order_of_battle",
        "intel", "special_projects", "world", "world_events",
    ]
    snap["provenance"]["save"] = {
        "captured": True,
        "source_filename": f"richdemo_{date.split('.')[0]}.txt",  # basename only
        "save_date": date,
        "domains": save_domains,
    }
    if with_memory:
        snap["provenance"]["memory"] = {
            "captured": True,
            "exe_sha256": "16022851deadbeef",
            "domains": ["country_scalars", "state_control", "divisions",
                        "naval_air", "supply"],
            "save_date_validated_against": date,
        }
        # PROXY sidecars are MEMORY-channel and carry their OWN confidence label.
        snap["naval_air"] = {
            "region_control": {"5": [{"controller": "GER"}], "12": [{"controller": "ENG"}]},
            "air_superiority": {"5": {"controller": "GER"}, "12": {"controller": "ENG"}},
            "source": "M", "confidence": "region_proxy",
            "air_superiority_model": None, "air_superiority_model_meta": None,
        }
        snap["supply"] = {
            "by_province": {
                "11596": {"current": 18.0, "capacity": 24.0, "gain": 2.1, "divisions": 6},
                "9851": {"current": 7.5, "capacity": 30.0, "gain": 1.2, "divisions": 11},
            },
            "source": "M", "confidence": "division_province_proxy",
            "solver_efficiency_by_province": None, "solver_draw_by_province": None,
            "solver_inputs_source": None, "solver_inputs_confidence": None,
        }

    # progress the game forward a bit on the later frame
    advanced = with_memory

    # ---- countries ---------------------------------------------------------
    snap["countries"] = {
        "GER": _germany(advanced),
        "SOV": _soviet(advanced),
        "ENG": _britain(advanced),
        "BEL": _minor_belgium(advanced),  # a minor: sparse, shows "not captured"
    }

    # ---- states (a handful; owner/controller drive the derived states-owned)
    snap["states"] = _states(advanced)

    # ---- divisions ---------------------------------------------------------
    snap["divisions"] = _divisions(advanced)

    # ---- combats (active engagements at the instant; M*) -------------------
    if advanced:
        snap["combats"] = [
            {"combat_id": "5457:62", "kind": "land", "location_province": 12740,
             "attackers": [[7989, 4713]],
             "defenders": [[2528, 4713], [4030, 4713], [2730, 4713]],
             "combat_width": 84.0, "terrain": "hills", "progress": 14.55877,
             "modifiers": {"day": 44, "duration": 44}},
            {"combat_id": "5460:62", "kind": "naval", "location_province": 8021,
             "attackers": [[12, 4990]], "defenders": [[44, 4990]],
             "combat_width": None, "terrain": None, "progress": -3.2,
             "modifiers": {"day": 2, "duration": 2}},
        ]
    else:
        snap["combats"] = []

    # ---- diplomacy_world (wars w/ casualties + factions) -------------------
    snap["diplomacy_world"] = {
        "wars": {
            "GER-ENG@1939.9.3.1": {
                "a": "GER", "b": "ENG", "start_date": "1939.9.3.1",
                "a_casualties": 220140 if advanced else 41200,
                "b_casualties": 180300 if advanced else 30100,
                "war_score_a_vs_b": 61.4 if advanced else 22.8,
                "war_score_b_vs_a": 14.2 if advanced else 9.1,
            },
            "GER-SOV@1941.6.22.5": ({
                "a": "GER", "b": "SOV", "start_date": "1941.6.22.5",
                "a_casualties": 1840220, "b_casualties": 3120880,
                "war_score_a_vs_b": 33.1, "war_score_b_vs_a": 40.6,
            } if advanced else None),
        },
        "factions": {
            "axis": {"id": "axis", "ideology": "fascism",
                     "members": ["GER", "ITA", "HUN"], "leader": "GER"},
            "allies": {"id": "allies", "ideology": "democratic",
                       "members": ["ENG", "FRA"], "leader": "ENG"},
            "comintern": {"id": "comintern", "ideology": "communism",
                          "members": ["SOV"], "leader": "SOV"},
        },
        "subjects": [],
    }
    # strip the null SOV war key if early frame (keep the map clean)
    snap["diplomacy_world"]["wars"] = {
        k: v for k, v in snap["diplomacy_world"]["wars"].items() if v is not None}

    # ---- designs (templates + variant catalog) -----------------------------
    snap["designs"] = {
        "division_templates": [
            {"id": 1, "type": 52, "name": "Infanterie-Division", "country": "GER",
             "role": "infantry", "regiments": [{"type": "infantry", "x": 0, "y": 0}],
             "support": [], "priority": 1, "division_cap": 0, "obsolete": False,
             "n_regiments": 9, "n_support": 3},
            {"id": 2, "type": 52, "name": "Panzer-Division", "country": "GER",
             "role": "armor", "regiments": [{"type": "light_armor", "x": 0, "y": 0}],
             "support": [], "priority": 2, "division_cap": 0, "obsolete": False,
             "n_regiments": 6, "n_support": 4},
        ],
        "equipment_variants": {
            "70:1": {"id": 70, "type": 1, "archetype": "infantry_equipment_2",
                     "family": "land_other", "name": "Karabiner 98k", "version": 1,
                     "creator": "GER", "obsolete": False, "modules": {}, "upgrades": {}},
            "120:1": {"id": 120, "type": 1, "archetype": "medium_tank_chassis",
                      "family": "land_vehicle", "name": "Panzer IV Ausf. F",
                      "version": 2, "creator": "GER", "obsolete": False,
                      "modules": {}, "upgrades": {"reliability": 1}},
        },
    }

    # ---- order_of_battle (the command tree screen 3 surfaces) --------------
    snap["order_of_battle"] = _oob(advanced)

    # ---- world scenario meta ----------------------------------------------
    snap["world"]["scenario"] = {
        "start_date": "1936.1.1.12", "difficulty": "veteran", "historical": True,
        "speed": 3, "ideology": "fascism", "save_version": snap["save_version"],
        "dlcs": ["Together for Victory", "Waking the Tiger", "No Step Back"],
        "mods": ["AMSB"],
    }
    snap["world_events"]["fired_events"] = (
        ["germany.1", "germany.22", "war.13"] if advanced else ["germany.1"])

    return snap


# --------------------------------------------------------------------------
# country builders
# --------------------------------------------------------------------------
def _germany(adv: bool) -> Dict[str, Any]:
    c = empty_country()
    c["human_name"] = "Manstein"  # MP persona (new domain)
    eco = c["economy"]
    eco["factories"] = {"civ": 118 if adv else 110, "mil": 156 if adv else 131,
                        "dockyard": 18 if adv else 16,
                        "civ_from_states": 116 if adv else 108,
                        "mil_from_states": 156 if adv else 131,
                        "dockyard_from_states": 18 if adv else 16,
                        "offmap": {"civ": 2, "mil": 0, "dockyard": 0},
                        "under_construction": 9 if adv else 12}
    eco["fuel"] = {"current": 612400.0 if adv else 849000.0, "max": 849000.0}
    eco["resources"] = {"produced": {"steel": 412.0, "oil": 36.0, "aluminium": 88.0}}
    eco["construction_queue"] = [
        {"template": "industrial_complex", "location_state": 23,
         "active_factories": 5, "produced": 0.42, "cost": 7200.0, "priority": 1,
         "created_date": "1940.8.1.12"}]
    mil = c["military"]
    mil["num_ships"] = 38 if adv else 32
    mil["division_count"] = 184 if adv else 126
    mil["manpower"] = {"available": 480000 if adv else 525000}
    mil["xp"] = {"army": 120.0, "navy": 40.0, "air": 80.0}
    mil["casualties"] = {"killed": 240500, "captured": 4100} if adv else {}  # late: real; early: not captured
    mil["command_power"] = 180.0
    mil["equipment_stockpile"] = [
        {"archetype": "infantry_equipment_2", "equip_id": 1951, "equip_type": 70, "amount": 18420.0},
        {"archetype": "artillery_equipment_2", "equip_id": 1912, "equip_type": 70, "amount": 2410.0},
        {"archetype": "medium_tank_chassis", "equip_id": 120, "equip_type": 1, "amount": 612.0},
    ]
    pol = c["politics"]
    pol["stability"] = 0.74 if adv else 0.81
    pol["war_support"] = 0.88 if adv else 0.62
    pol["political_power"] = 142.0
    pol["command_power"] = 180.0
    pol["ruling_party"] = "fascism"
    pol["party_popularities"] = {"fascism": 0.62, "democratic": 0.10,
                                 "communism": 0.18, "neutrality": 0.10}
    pol["laws"] = {"economy": "war_economy" if adv else "partial_mobilisation",
                   "trade": "limited_exports", "conscription": "extensive_conscription"}
    pol["ideas_all"] = ["war_economy", "extensive_conscription", "limited_exports",
                        "GER_general_staff", "GER_autobahn"]
    pol["ideas_other"] = ["GER_general_staff", "GER_autobahn", "GER_mefo_bills"]
    pol["is_human"] = True
    pol["is_major"] = True
    pol["capital_state"] = 64
    pol["faction"] = {"name": "axis", "leader": "GER", "is_leader": True}
    res = c["research"]
    res["tech_count"] = 148 if adv else 122
    res["research_slots"] = 4
    res["original_research_slots"] = 3
    res["in_progress"] = [
        {"name": "excavation3", "points_factor": 1.345},
        {"name": "medium_tank_chassis_2", "points_factor": 1.0},
        {"name": "infantry_at_upgrade_3", "points_factor": 0.9},
    ]
    res["research_bonuses"] = [
        {"name": "GER_panzer_doctrine", "categories": ["armor"], "bonus": 0.15, "kind": "doctrine"}]
    fc = c["focus"]
    fc["focus_tree"] = "germany_focus"
    fc["completed_count"] = 41 if adv else 28
    fc["completed"] = ["GER_rhineland", "GER_four_year_plan", "GER_anschluss",
                       "GER_sudetenland", "GER_danzig_or_war"]
    fc["current"] = "GER_invade_britain" if adv else "GER_war_economy"
    fc["progress"] = 4.2 if adv else 1.0
    fc["paused"] = False
    c["decisions"] = {
        "active": ["GER_continuous_war_economy"],
        "timed": [{"id": "GER_invite_to_axis", "cooldown_days": 12}],
        "missions": ["GER_prepare_sealion"]}
    # intel agency (rich)
    c["intel"] = {
        "agency": {"name": "Abwehr", "icon": "GFX_intelligence_agency_logo_ger",
                   "is_created": True, "upgrade_progress": 22.0,
                   "current_upgrade": "upgrade_decryption_boost_2",
                   "upgrades": {"upgrade_passive_defense": 2, "upgrade_anti_partisan": 2,
                                "upgrade_economy_civilian": 1},
                   "capacity": {"max_operatives": 6, "usable_slots": 5, "defense": 40}},
        "operatives": [
            {"op_id": 1, "name": "Otto Skorzeny", "gfx": "GFX_operative_ger",
             "skill": 3, "experience": 12.0, "traits": ["commando"],
             "nationalities": ["GER"], "state": "on_operation", "captured": False}],
        "recruitable_operatives": [], "intel_networks": [],
        "operations": {"priority": 1, "running": [], "finished": []},
        "decryption": {"targets": [{"target": "ENG", "decryption": True, "amount": 41.0}],
                       "intel_source": None},
        "peace_conference": None}
    # commanders w/ real portrait handles
    c["commanders"] = {
        "leader": {"name": "GER_adolf_hitler", "token": "GER_adolf_hitler",
                   "ideology": "nazism", "traits": ["GER_der_fuhrer"],
                   "expire": "1965.1.1.1", "portrait_gfx": _HITLER, "char_id": 1},
        "party_leaders": [], "ruling_party": "fascism",
        "generals": [
            {"name": "GER_heinz_guderian", "token": "GER_heinz_guderian",
             "role": "general", "level": 5, "experience": 220.0, "attack": 4,
             "defense": 3, "planning": 4, "logistics": 3, "traits": ["panzer_leader"],
             "assignment": {"kind": "army", "name": "2. Panzerarmee"},
             "portrait_gfx": "GFX_portrait_GER_heinz_guderian", "unit_leader_id": 11}],
        "field_marshals": [
            {"name": "GER_erich_von_manstein", "token": "GER_erich_von_manstein",
             "role": "field_marshal", "level": 6, "experience": 410.0, "attack": 5,
             "defense": 4, "planning": 5, "logistics": 4, "traits": ["panzer_expert"],
             "assignment": {"kind": "army_group", "name": "Heeresgruppe Süd"},
             "portrait_gfx": "GFX_portrait_GER_erich_von_manstein", "unit_leader_id": 12}],
        "admirals": [
            {"name": "GER_karl_donitz", "token": "GER_karl_donitz", "role": "admiral",
             "level": 4, "experience": 130.0, "maneuvering": 4, "coordination": 3,
             "traits": ["seawolf"], "assignment": {"kind": "fleet", "name": "U-Boot-Flotte"},
             "portrait_gfx": "GFX_portrait_GER_karl_donitz", "unit_leader_id": 13}],
        "advisors": [
            {"name": "GER_hermann_goring", "token": "GER_hermann_goring",
             "slot": "air_chief", "ledger": "air", "idea_token": "GER_hermann_goring",
             "political_power": 100.0, "traits": ["air_chief_ground_support_2"],
             "portrait_gfx": "GFX_portrait_GER_hermann_goring_small"}],
        "counts": {"generals": 1, "field_marshals": 1, "admirals": 1, "advisors": 1,
                   "roster_size": 4}}
    c["mios"] = [
        {"key": "GER_rheinmetall", "id": 1, "type": 79, "name": "Rheinmetall",
         "icon": "GFX_idea_GER_rheinmetall", "level": 3, "funds": 1200.0, "points": 40.0,
         "research_bonus": 0.10, "task_capacity": 2, "unlocked_traits": ["mass_production"],
         "allowed_policies": ["mio_policy_general_ruthless_contracts"],
         "assigned_equipment_archetype": "artillery_equipment"}]
    # extra (the NEW domain)
    c["extra"] = {
        "nukes_total": 0 if not adv else 2, "nukes_ready_total": 0 if not adv else 1,
        "nukes": [{"amount": 2, "nukes_ready": 1}] if adv else [{"amount": 0, "nukes_ready": 0}],
        "ace_count": 2, "aces": [
            {"name": "Erich", "surname": "Hartmann", "callsign": "Bubi",
             "modifier": "ace_bonus", "alive": True, "kill_type": None,
             "killer_country": None}],
        "special_projects": {"projects_started": 5, "projects_completed": 3 if adv else 1,
                             "active_programs": [
                                 {"building": "nuclear_reactor", "province": 11596, "country": "GER"}],
                             "breakthrough": {"specialization_nuclear": 420.0}},
        "experience_status": {"army_experience": 320.0, "navy_experience": 110.0,
                              "air_experience": 240.0},
        "convoys_destroyed": 84 if adv else 12, "cosmetic_tag": "",
        "collaboration_present": adv}
    return c


def _soviet(adv: bool) -> Dict[str, Any]:
    c = empty_country()
    c["human_name"] = "Zhukov"
    eco = c["economy"]
    eco["factories"] = {"civ": 210, "mil": 240 if adv else 180, "dockyard": 8,
                        "offmap": {"civ": 0, "mil": 0, "dockyard": 0},
                        "under_construction": 18}
    eco["fuel"] = {"current": 1240000.0, "max": 1500000.0}
    mil = c["military"]
    mil["num_ships"] = 22
    mil["division_count"] = 360 if adv else 210
    mil["manpower"] = {"available": 2400000}
    mil["casualties"] = {"killed": 3120880, "captured": 980000} if adv else {}
    mil["command_power"] = 150.0
    mil["equipment_stockpile"] = [
        {"archetype": "infantry_equipment_1", "equip_id": 438, "equip_type": 70, "amount": 41000.0}]
    pol = c["politics"]
    pol["stability"] = 0.55
    pol["war_support"] = 0.91 if adv else 0.40
    pol["political_power"] = 88.0
    pol["ruling_party"] = "communism"
    pol["party_popularities"] = {"communism": 0.80, "fascism": 0.05,
                                 "democratic": 0.05, "neutrality": 0.10}
    pol["laws"] = {"economy": "war_economy" if adv else "civilian_economy",
                   "trade": "closed_economy", "conscription": "scraping_the_barrel" if adv else "limited_conscription"}
    pol["ideas_all"] = ["closed_economy", "limited_conscription", "trotskyite_plot"]
    pol["ideas_other"] = ["trotskyite_plot", "the_great_purge"]
    pol["is_human"] = True
    pol["is_major"] = True
    pol["capital_state"] = 219
    pol["faction"] = {"name": "comintern", "leader": "SOV", "is_leader": True}
    res = c["research"]
    res["tech_count"] = 130 if adv else 98
    res["research_slots"] = 5
    res["in_progress"] = [{"name": "heavy_tank_chassis_2", "points_factor": 1.0}]
    res["research_bonuses"] = []
    fc = c["focus"]
    fc["focus_tree"] = "soviet_focus"
    fc["completed_count"] = 33 if adv else 20
    fc["current"] = "SOV_five_year_plan_industry"
    fc["progress"] = 2.0
    c["intel"] = {"agency": {"name": "NKVD", "icon": "GFX_intelligence_agency_logo_sov",
                             "is_created": True, "upgrade_progress": 5.0,
                             "current_upgrade": None, "upgrades": {},
                             "capacity": {"max_operatives": 4, "usable_slots": 4, "defense": 30}},
                  "operatives": [], "recruitable_operatives": [], "intel_networks": [],
                  "operations": None, "decryption": None, "peace_conference": None}
    c["commanders"] = {
        "leader": {"name": "SOV_joseph_stalin", "token": "SOV_joseph_stalin",
                   "ideology": "stalinism", "traits": [], "portrait_gfx": _STALIN, "char_id": 2},
        "party_leaders": [], "ruling_party": "communism",
        "generals": [{"name": "SOV_georgy_zhukov", "token": "SOV_georgy_zhukov",
                      "role": "general", "level": 5, "experience": 300.0, "attack": 4,
                      "defense": 4, "planning": 3, "logistics": 3, "traits": ["defensive_doctrine"],
                      "assignment": None, "portrait_gfx": "GFX_portrait_SOV_georgy_zhukov",
                      "unit_leader_id": 21}],
        "field_marshals": [], "admirals": [], "advisors": [],
        "counts": {"generals": 1, "field_marshals": 0, "admirals": 0, "advisors": 0,
                   "roster_size": 1}}
    c["extra"] = {"nukes_total": 0, "nukes_ready_total": 0, "ace_count": 0, "aces": [],
                  "special_projects": {"projects_started": 1, "projects_completed": 0,
                                       "active_programs": [], "breakthrough": {}},
                  "convoys_destroyed": 4, "cosmetic_tag": "", "collaboration_present": False}
    return c


def _britain(adv: bool) -> Dict[str, Any]:
    c = empty_country()
    c["human_name"] = None  # AI: no persona
    eco = c["economy"]
    eco["factories"] = {"civ": 95, "mil": 110, "dockyard": 40,
                        "offmap": {"civ": 0, "mil": 0, "dockyard": 0},
                        "under_construction": 6}
    eco["fuel"] = {"current": 980000.0, "max": 1200000.0}
    mil = c["military"]
    mil["num_ships"] = 210
    mil["division_count"] = 95 if adv else 60
    mil["manpower"] = {"available": 410000}
    mil["casualties"] = {"killed": 180300, "captured": 60200} if adv else {}
    pol = c["politics"]
    pol["stability"] = 0.70
    pol["war_support"] = 0.75 if adv else 0.50
    pol["political_power"] = 120.0
    pol["ruling_party"] = "democratic"
    pol["party_popularities"] = {"democratic": 0.78, "fascism": 0.04,
                                 "communism": 0.08, "neutrality": 0.10}
    pol["laws"] = {"economy": "war_economy" if adv else "early_mobilisation",
                   "trade": "free_trade", "conscription": "extensive_conscription" if adv else "limited_conscription"}
    pol["ideas_all"] = ["free_trade", "limited_conscription"]
    pol["ideas_other"] = ["ENG_the_war_cabinet"]
    pol["is_human"] = False  # measured AI
    pol["is_major"] = True
    pol["capital_state"] = 126
    pol["faction"] = {"name": "allies", "leader": "ENG", "is_leader": True}
    res = c["research"]
    res["tech_count"] = 140 if adv else 110
    res["research_slots"] = 4
    res["in_progress"] = [{"name": "radar_tech_2", "points_factor": 1.0}]
    fc = c["focus"]
    fc["focus_tree"] = "britain_focus"
    fc["completed_count"] = 30 if adv else 22
    fc["current"] = "ENG_war_economy"
    fc["progress"] = 1.5
    c["intel"] = {"agency": {"name": "MI6", "icon": "GFX_intelligence_agency_logo_eng",
                             "is_created": True, "upgrade_progress": 60.0,
                             "current_upgrade": "upgrade_network_node", "upgrades": {"upgrade_network": 3},
                             "capacity": {"max_operatives": 8, "usable_slots": 7, "defense": 50}},
                  "operatives": [], "recruitable_operatives": [], "intel_networks": [],
                  "operations": None, "decryption": None, "peace_conference": None}
    c["commanders"] = {
        "leader": {"name": "ENG_winston_churchill", "token": "ENG_winston_churchill",
                   "ideology": "conservatism", "traits": [],
                   "portrait_gfx": "GFX_portrait_ENG_winston_churchill", "char_id": 3},
        "party_leaders": [], "ruling_party": "democratic",
        "generals": [], "field_marshals": [], "admirals": [
            {"name": "ENG_andrew_cunningham", "token": "ENG_andrew_cunningham",
             "role": "admiral", "level": 5, "experience": 200.0, "maneuvering": 4,
             "coordination": 4, "traits": ["fleet_protector"], "assignment": None,
             "portrait_gfx": "GFX_portrait_ENG_andrew_cunningham", "unit_leader_id": 31}],
        "advisors": [],
        "counts": {"generals": 0, "field_marshals": 0, "admirals": 1, "advisors": 0,
                   "roster_size": 1}}
    c["extra"] = {"nukes_total": 0, "nukes_ready_total": 0, "ace_count": 1,
                  "aces": [{"name": "Douglas", "surname": "Bader", "callsign": None,
                            "modifier": "ace_bonus", "alive": True}],
                  "special_projects": {"projects_started": 2, "projects_completed": 1,
                                       "active_programs": [], "breakthrough": {}},
                  "convoys_destroyed": 12, "cosmetic_tag": "", "collaboration_present": False}
    return c


def _minor_belgium(adv: bool) -> Dict[str, Any]:
    """A minor: deliberately sparse so the no-fabrication 'not captured' shows."""
    c = empty_country()
    eco = c["economy"]
    eco["factories"] = {"civ": 12, "mil": 6, "dockyard": 1,
                        "offmap": {"civ": 0, "mil": 0, "dockyard": 0},
                        "under_construction": 0}
    eco["fuel"] = {}  # NOT captured (stays {})
    mil = c["military"]
    mil["num_ships"] = 4
    mil["division_count"] = 12
    mil["manpower"] = {"available": 40000}
    mil["casualties"] = {}  # not captured
    pol = c["politics"]
    pol["stability"] = 0.60
    pol["war_support"] = 0.20
    pol["ruling_party"] = "democratic"
    pol["party_popularities"] = None  # NOT captured for the minor
    pol["laws"] = {"economy": "civilian_economy", "trade": "free_trade",
                   "conscription": "volunteer_only"}
    pol["ideas_all"] = ["volunteer_only", "free_trade"]
    pol["is_human"] = False
    pol["is_major"] = False
    pol["capital_state"] = 7
    pol["faction"] = None
    res = c["research"]
    res["tech_count"] = 60
    res["research_slots"] = 3
    res["in_progress"] = []
    fc = c["focus"]
    fc["focus_tree"] = "generic_focus"
    fc["completed_count"] = 8
    # intel/commanders/extra left mostly empty -> the UI shows "not captured" honestly
    c["commanders"] = {"leader": None, "party_leaders": [], "ruling_party": "democratic",
                       "generals": [], "field_marshals": [], "admirals": [], "advisors": [],
                       "counts": None}
    return c


# --------------------------------------------------------------------------
# states / divisions / oob
# --------------------------------------------------------------------------
def _states(adv: bool) -> Dict[str, Any]:
    def st(owner, ctrl, vp=0):
        return {"owner": owner, "controller": ctrl, "is_core_of": [owner] if owner else [],
                "victory_points_total": vp, "category": "town"}
    s = {
        "23": st("GER", "GER", 5), "64": st("GER", "GER", 20),
        "219": st("SOV", "SOV", 50), "126": st("ENG", "ENG", 30),
        "7": st("BEL", "BEL", 3),
        "9851": st("ENG", "GER" if adv else "ENG", 8),  # contested → owner flip in control
    }
    return s


def _divisions(adv: bool) -> List[Dict[str, Any]]:
    base = [
        {"div_id": 2279, "div_type": 4713, "owner": "GER", "location_province": 11596,
         "state_id": 23, "template_id": 1, "template_type": 52,
         "template_name": "Infanterie-Division", "template_role": "infantry",
         "organisation": 99.8, "strength": 1.0,
         "equipment": [{"archetype": "infantry_equipment_2", "equip_id": 1951,
                        "equip_type": 70, "amount": 980.0}],
         "experience": 0.4, "moving": False, "in_combat": False},
        {"div_id": 2280, "div_type": 4713, "owner": "GER", "location_province": 12740,
         "state_id": 23, "template_id": 2, "template_type": 52,
         "template_name": "Panzer-Division", "template_role": "armor",
         "organisation": 88.2, "strength": 0.92,
         "equipment": [{"archetype": "medium_tank_chassis", "equip_id": 120,
                        "equip_type": 1, "amount": 120.0}],
         "experience": 1.2, "moving": True, "in_combat": True},
        {"div_id": 4101, "div_type": 4713, "owner": "SOV", "location_province": 9851,
         "state_id": 219, "template_id": 5, "template_type": 52,
         "template_name": "Strelkovaya Diviziya", "template_role": "infantry",
         "organisation": 60.0, "strength": 0.7,
         "equipment": [], "experience": None, "moving": None, "in_combat": None},
        {"div_id": 5500, "div_type": 4713, "owner": "ENG", "location_province": 126,
         "state_id": 126, "template_id": 9, "template_type": 52,
         "template_name": "Infantry Division", "template_role": "infantry",
         "organisation": 70.0, "strength": 0.85, "equipment": []},
    ]
    if adv:
        # more SOV divisions later (so the chart/count moves)
        for i in range(6):
            base.append({"div_id": 6000 + i, "div_type": 4713, "owner": "SOV",
                         "location_province": 219, "state_id": 219, "template_id": 5,
                         "template_type": 52, "template_name": "Strelkovaya Diviziya",
                         "template_role": "infantry", "organisation": 55.0 + i,
                         "strength": 0.6, "equipment": []})
    return base


def _oob(adv: bool) -> Dict[str, Any]:
    oob = empty_order_of_battle()  # keeps the canonical `ships`/`navy_theaters` lists
    oob.update({
        "army_groups": [
            {"id": "49:53", "owner": "GER", "name": "Heeresgruppe Süd",
             "leader": "12:4713", "army_refs": ["46:53", "71:53"],
             "army_refs_resolved": 2, "icon": 1,
             "cached": {"plan_value": 0.7, "our_power": 20826.8, "enemy_power": 14200.0}}],
        "armies": [
            {"id": "46:53", "owner": "GER", "name": "2. Panzerarmee", "leader": "11:4713",
             "members": ["2280:4713"], "member_count": 1, "members_resolved": 1,
             "orders": [], "field_marshal_group": "49:53", "cached": {}},
            {"id": "71:53", "owner": "GER", "name": "6. Armee", "leader": None,
             "members": ["2279:4713"], "member_count": 1, "members_resolved": 1,
             "orders": [], "field_marshal_group": "49:53", "cached": {}},
            {"id": "210:53", "owner": "SOV", "name": "Western Front", "leader": "21:4713",
             "members": ["4101:4713"], "member_count": 1, "members_resolved": 1,
             "orders": [], "field_marshal_group": None, "cached": {}}],
        "fleets": [
            {"id": "12:54", "owner": "GER", "name": "U-Boot-Flotte", "leader": "13:4713",
             "task_forces": [
                 {"id": "1:55", "name": "1. U-Flottille", "location_province": 8021,
                  "owner": "GER", "mission": "convoy_raiding",
                  "ships": [{"id": 12, "definition": "submarine", "organisation": 100.0,
                             "strength": 1.0, "experience": 0.2, "name": "U-47",
                             "equipment": [[55, 1, 1]]}]}]}],
        "navy_theaters": [],
        "air_wings": [
            {"id": "5:56", "owner": "GER", "name": "Jagdgeschwader 1", "count": 120,
             "experience": 12.0, "manpower": 1200, "mission": "air_superiority",
             "mission_type": "air_superiority", "strategic_region": 5,
             "plane_definition": "fighter", "air_base": "23:1", "state_id": 23,
             "equipment": [[200, 1, 120]], "effective_planes": 118}],
        "theatres": [
            {"owner": "GER", "id": "1:57", "area_states": [23, 64], "n_armies": 2,
             "n_army_groups": 1}],
        "by_country": {
            "GER": {"armies": 2, "army_groups": 1, "divisions_in_armies": 2,
                    "fleets": 1, "task_forces": 1, "ships": 1, "air_wings": 1, "planes": 120},
            "SOV": {"armies": 1, "army_groups": 0, "divisions_in_armies": 1,
                    "fleets": 0, "task_forces": 0, "ships": 0, "air_wings": 0, "planes": 0}},
    })
    return oob


# --------------------------------------------------------------------------
# driver
# --------------------------------------------------------------------------
def generate(out_dir: Optional[str] = None) -> str:
    """Build the rich store, validate every frame, write it + a real timeline."""
    out_dir = out_dir or OUT_DIR
    os.makedirs(out_dir, exist_ok=True)

    frames = [
        _frame("1940.6.1.12", _hour("1940.6.1.12"), with_memory=False),
        _frame("1942.11.19.6", _hour("1942.11.19.6"), with_memory=True),
    ]
    # validate — the synthetic store MUST be schema-valid (acceptance criterion 1)
    for f in frames:
        errs = validate_snapshot(f)
        if errs:
            raise AssertionError(f"richdemo frame {f['date']} invalid:\n  " +
                                 "\n  ".join(errs[:10]))

    index = {"game_id": RICH_GAME_ID, "schema_version": "1", "snapshots": []}
    for f in frames:
        fname = f"{f['hour_index']:012d}.snapshot.json"
        with open(os.path.join(out_dir, fname), "w", encoding="utf-8") as fh:
            json.dump(f, fh, separators=(",", ":"))
        index["snapshots"].append({
            "hour_index": f["hour_index"], "date": f["date"], "file": fname,
            "save_version": f["save_version"],
            "counts": {"countries": len(f["countries"]), "states": len(f["states"]),
                       "provinces": len(f["provinces"]), "divisions": len(f["divisions"])}})
    with open(os.path.join(out_dir, "index.json"), "w", encoding="utf-8") as fh:
        json.dump(index, fh, indent=2)

    # real diff-engine timeline over the two frames (no faked events)
    timeline = _build_timeline(frames)
    with open(os.path.join(out_dir, "timeline.json"), "w", encoding="utf-8") as fh:
        json.dump({"game_id": RICH_GAME_ID, "events": timeline}, fh, indent=1)

    return out_dir


def _hour(date: str) -> int:
    from replay.schema import date_to_hour_index
    return date_to_hour_index(date)


def _build_timeline(frames: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
    """Use the REAL diff engine so timeline events are not fabricated."""
    try:
        from replay.diff_engine import build_timeline
        return build_timeline(frames)
    except Exception:
        # diff engine API drift must not break the demo; fall back to empty.
        return []


if __name__ == "__main__":
    d = generate()
    print(f"[richdemo] wrote schema-valid rich store -> {d}")
