"""Tests for replay/schema.py — date math, source map, empty skeletons, validator."""
import copy
import hashlib
import json
import os
from pathlib import Path
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))

from replay import schema  # noqa: E402


class TestDateMath(unittest.TestCase):
    def test_known_epoch_anchor(self):
        # memory_reader note: 1940.9.2 hour21 -> 17000277
        self.assertEqual(schema.date_to_hour_index("1940.9.2.21"), 17000277)
        self.assertEqual(schema.date_to_hour_index("1940.9.2.22"), 17000278)

    def test_one_day_is_24h(self):
        a = schema.date_to_hour_index("1940.9.10.13")
        b = schema.date_to_hour_index("1940.9.11.13")
        self.assertEqual(b - a, 24)

    def test_month_rollover(self):
        # 1936.1.31.0 + 1 day -> 1936.2.1.0, 24h apart
        a = schema.date_to_hour_index("1936.1.31.0")
        b = schema.date_to_hour_index("1936.2.1.0")
        self.assertEqual(b - a, 24)

    def test_missing_hour_defaults_zero(self):
        self.assertEqual(schema.date_to_hour_index("1936.1.1"),
                         schema.date_to_hour_index("1936.1.1.0"))

    def test_malformed_returns_none(self):
        for bad in ("", None, "garbage", "1936", "1936.13.1.0", "1936.1.0.0"):
            self.assertIsNone(schema.date_to_hour_index(bad), bad)

    def test_ordering_monotone(self):
        dates = ["1936.1.1.12", "1936.6.1.0", "1940.9.10.13", "1941.4.13.11"]
        his = [schema.date_to_hour_index(d) for d in dates]
        self.assertEqual(his, sorted(his))


class TestSourceMap(unittest.TestCase):
    def test_concrete_tag_resolves(self):
        self.assertEqual(schema.source_of("countries.GER.politics.stability"), "M")
        self.assertEqual(schema.source_of("countries.SOV.economy.factories.civ"), "S")

    def test_prefix_fallback_for_list_subfield(self):
        # divisions.*.equipment is mapped; a deeper subfield inherits it
        self.assertEqual(schema.source_of("divisions.5.equipment"), "M/S")

    def test_orders_group_manager_source(self):
        self.assertEqual(
            schema.source_of(
                "order_of_battle.extra.army_battleplans_depth_live."
                "orders_group_manager.armies.0.execution_type"
            ),
            "C",
        )

    def test_state_and_province(self):
        self.assertEqual(schema.source_of("states.23.owner"), "C/M/S")
        # live collector province_control measures the province controller; legacy
        # memory/save and the save-side state-controller fallback remain possible.
        self.assertEqual(schema.source_of("provinces.11596.controller"), "C/M/S/D")
        self.assertEqual(schema.source_of("map.control_by_province"), "C/M/S/D")

    def test_d48_building_sidecar_sources(self):
        self.assertEqual(
            schema.source_of(
                "states.42.construction_buildings_state_damage_live."
                "state_buildings.type_0x00002fab.partial_health_raw"
            ),
            "C",
        )
        self.assertEqual(
            schema.source_of(
                "provinces.2068.construction_buildings_state_damage_live."
                "buildings.type_0x00004cc1.damaged"
            ),
            "C/D",
        )

    def test_collector_fuel_source(self):
        self.assertEqual(schema.source_of("countries.POR.economy.fuel.current"), "C/M/S")
        self.assertEqual(schema.source_of("countries.POR.economy.fuel.max"), "C/M/S")
        self.assertEqual(schema.source_of("countries.POR.economy.fuel.gain"), "C")
        self.assertEqual(schema.source_of("countries.POR.economy.fuel.cost"), "C")

    def test_collector_production_source(self):
        self.assertEqual(schema.source_of("countries.POR.production.stockpile"), "C/M")
        self.assertEqual(schema.source_of("countries.POR.production.stockpile_total"), "C/M/D")
        self.assertEqual(schema.source_of("countries.POR.production.military_lines"), "C/M")
        self.assertEqual(schema.source_of("countries.POR.production.naval_factories"), "C/M")
        self.assertEqual(
            schema.source_of("countries.POR.production.stockpile_complete"), "C/M")
        self.assertEqual(
            schema.source_of("countries.POR.production.military_lines.0.daily_output"),
            "C/M")

    def test_collector_focus_source(self):
        self.assertEqual(schema.source_of("countries.POR.focus.focus_tree"), "C/M/S")
        self.assertEqual(schema.source_of("countries.POR.focus.completed"), "C/M/S")
        self.assertEqual(schema.source_of("countries.POR.focus.current"), "C/M/S")
        self.assertEqual(
            schema.source_of("countries.POR.focus.current_continuous"), "C/M")
        self.assertEqual(schema.source_of("countries.POR.focus.progress"), "C/M/S")

    def test_fired_event_stream_source(self):
        self.assertEqual(schema.source_of("world_events.fired_events"), "C/S")
        self.assertEqual(schema.source_of("world_events.fired_events_complete"), "C")

    def test_active_ability_source(self):
        self.assertEqual(
            schema.source_of("countries.GER.active_abilities.snapshot.rows.0.ability_id"),
            "C/M",
        )
        self.assertEqual(
            schema.source_of("countries.GER.active_abilities.snapshot.rows.0.disabled_reason"),
            "C/M",
        )
        self.assertEqual(
            schema.source_of("countries.GER.active_abilities.snapshot.rows.0.cooldown_total.kind"),
            "C/M",
        )
        self.assertEqual(
            schema.source_of("countries.GER.active_abilities.snapshot.rows.0.cost_command_power_fixed"),
            "C/M",
        )

    def test_collector_research_source(self):
        self.assertEqual(schema.source_of("countries.POR.research.tech_count"), "C/M/S")
        self.assertEqual(
            schema.source_of("countries.POR.research.research_slots"), "C/M/S")
        self.assertEqual(
            schema.source_of("countries.POR.research.in_progress_live"), "C/M")

    def test_collector_special_projects_source(self):
        self.assertEqual(schema.source_of("special_projects.facilities"), "C/M")
        self.assertEqual(schema.source_of("special_projects.projects"), "C/M")
        self.assertEqual(schema.source_of("special_projects.scientists"), "C/M")
        self.assertEqual(schema.source_of("special_projects._r2_breakthroughs"), "C")
        self.assertEqual(schema.source_of("special_projects.project_progress"), "C")
        self.assertEqual(
            schema.source_of("special_projects.scientists.0.traits"), "M")

    def test_collector_divisions_contract_source(self):
        contract = schema.COLLECTOR_DOMAIN_CONTRACTS["divisions"]
        self.assertEqual(contract["cadence_class"], "scheduled_hourly_snapshot")
        self.assertIn("move_progress", contract["first_slice_fields"])
        self.assertEqual(schema.source_of("divisions.0.div_id"), "C/M/S")
        self.assertEqual(schema.source_of("divisions.0.div_type"), "C/M/S")
        self.assertEqual(schema.source_of("divisions.0.owner"), "C/M/S")
        self.assertEqual(schema.source_of("divisions.0.location_province"), "C/M/S")
        self.assertEqual(schema.source_of("divisions.0.organisation"), "C/M")
        self.assertEqual(schema.source_of("divisions.0.move_progress"), "C/M*")
        # Equipment remains separate from the first collector `divisions` slice.
        self.assertEqual(schema.source_of("divisions.0.equipment"), "M/S")

    def test_collector_land_combats_raw_contract_is_not_snapshot_promoted(self):
        contract = schema.COLLECTOR_DOMAIN_CONTRACTS["land_combats"]
        self.assertEqual(contract["cadence_class"], "high_frequency_raw_stream")
        self.assertIsNone(contract["snapshot_path"])
        self.assertEqual(
            contract["root_contract_status"],
            "pinned_live_raw_stream",
        )
        self.assertEqual(contract["identity_rule"], "land_combat:<combat_id>:<generation>")
        self.assertIn("attacker_divisions", contract["nullable_side_fields"])
        self.assertEqual(schema.source_of("combats.0.combat_id"), "M*")
        self.assertEqual(
            schema.source_of("combats.0.modifiers.save_combat_handle"), "S"
        )
        self.assertEqual(schema.source_of("combats.0.modifiers.day"), "S")
        self.assertEqual(schema.source_of("combats.0.modifiers.duration"), "S")
        self.assertEqual(
            schema.source_of("combats.0.modifiers.save_air_owner_join_complete"), "S"
        )

    def test_unknown_returns_none(self):
        self.assertIsNone(schema.source_of("countries.GER.nonsense.field"))

    def test_wave4_exact_carrier_sources(self):
        expected = {
            "countries.GER.extra.nukes_total": "C/S",
            "_r2_nuke_stockpile_live.countries.0.amount_raw": "C",
            "countries.GER.dynamic_modifier_instances.0.remaining_days": "C",
            "_r2_dynamic_modifier_lifecycle_live.transitions.0.kind": "C",
            "countries.GER.special_project_programs.0.program_id": "C",
            "runtime_text.contexts.0.final_text.sha256": "C",
            "diplomacy_world.domain53_extra.operation_instance_rows.0.status": "C",
            "_r2_sunk_ship_history_live.events.0.ship_name": "C",
            "_r2_decision_panel_live.children.0.identity": "C",
            "_r2_scripted_gui_runtime_live.manager_order.0.gui_id": "C",
            "countries.ENG.extra.scripted_gui_runtime.panel.visible.value": "C",
            "_r2_national_focus_presentation_live.items.0.focus_id": "C",
            "_r2_character_presentation_live.trait_trees.0.nodes.0.state": "C",
            "order_of_battle.extra.naval_missions_status_live.task_forces.0.activity_key": "C/D",
            "combats.0.land_combat_detail_events_live.attacker.width_observations.0.source": "C",
            "map._r2_weather_live.provinces.1.snow_raw": "C",
        }
        for path, source in expected.items():
            self.assertEqual(schema.source_of(path), source, path)

    def test_every_mapped_value_is_a_legal_tag(self):
        legal = set()
        for combo in ("M", "M*", "S", "D"):
            legal.add(combo)
        # allow compound "A/B" tags
        for v in schema.FIELD_SOURCES.values():
            parts = v.split("/")
            for p in parts:
                self.assertIn(p, {"C", "M", "M*", "S", "D", "marker"}, v)


class TestEmptySkeletons(unittest.TestCase):
    def test_empty_country_has_all_domains(self):
        c = schema.empty_country()
        for dom in ("economy", "military", "politics", "research", "focus",
                    "decisions", "diplomacy", "intel"):
            self.assertIn(dom, c)

    def test_empty_snapshot_validates(self):
        snap = schema.empty_snapshot("g1", "1940.9.10.13")
        self.assertEqual(schema.validate_snapshot(snap), [])
        self.assertEqual(snap["hour_index"], schema.date_to_hour_index("1940.9.10.13"))

    def test_collector_provenance_validates(self):
        snap = schema.empty_snapshot("g1", "1940.9.10.13")
        snap["provenance"]["collector"] = {
            "captured": True,
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "exe_sha256": "0" * 64,
            "mod_checksum": "vanilla",
            "profile_id": "hoi4-1.19.1-test",
            "quality_status": "synthetic",
            "capture_boundary": "daily_update",
            "transport": "synthetic",
            "transport_crc32": "1234abcd",
            "transport_bytes": 512,
            "sequence": 1,
            "domains": ["clock", "country_identity"],
        }
        self.assertEqual(schema.validate_snapshot(snap), [])

    def test_bad_collector_transport_metadata_flagged(self):
        snap = schema.empty_snapshot("g1", "1940.9.10.13")
        snap["provenance"]["collector"] = {
            "captured": True,
            "transport_crc32": 1234,
            "transport_bytes": "512",
        }
        probs = schema.validate_snapshot(snap)
        self.assertTrue(any("transport_crc32" in p for p in probs), probs)
        self.assertTrue(any("transport_bytes" in p for p in probs), probs)

    def test_bad_collector_domain_flagged(self):
        snap = schema.empty_snapshot("g1", "1940.9.10.13")
        snap["provenance"]["collector"] = {
            "captured": True,
            "domains": ["nope"],
        }
        probs = schema.validate_snapshot(snap)
        self.assertTrue(any("provenance.collector.domains" in p for p in probs), probs)

    def test_containers_always_present(self):
        snap = schema.empty_snapshot("g1")
        for k in ("countries", "states", "provinces", "divisions", "combats",
                  "map", "events", "provenance"):
            self.assertIn(k, snap)

    def test_no_fabrication_nulls(self):
        # an unobtained scalar is None, not a guessed default
        c = schema.empty_country()
        self.assertIsNone(c["politics"]["stability"])
        self.assertIsNone(c["military"]["num_ships"])

    def test_per_entity_skeletons_match_dataclasses(self):
        # empty_state/province/division copy precomputed templates for speed;
        # they must remain exactly the dataclass shape.
        from dataclasses import asdict
        self.assertEqual(schema.empty_state(), asdict(schema.State()))
        self.assertEqual(schema.empty_province(), asdict(schema.Province()))
        self.assertEqual(schema.empty_division(), asdict(schema.Division()))

    def test_per_entity_skeletons_share_no_mutable_state(self):
        # the template copy must splice in FRESH lists — mutating one skeleton
        # can never leak into another (or into the template itself).
        s1, s2 = schema.empty_state(), schema.empty_state()
        s1["is_core_of"].append("GER")
        s1["is_claimed_by"].append("POL")
        self.assertEqual(s2["is_core_of"], [])
        self.assertEqual(s2["is_claimed_by"], [])
        self.assertEqual(schema.empty_state()["is_core_of"], [])
        d1, d2 = schema.empty_division(), schema.empty_division()
        d1["equipment"].append({"variant_id": "1", "count": 2})
        self.assertEqual(d2["equipment"], [])
        self.assertEqual(schema.empty_division()["equipment"], [])


class TestValidator(unittest.TestCase):
    def _a6_disrupted_supply_snapshot(self):
        snap = schema.empty_snapshot("a6", "1940.9.10.13")
        country = schema.empty_country()
        country["_r2_tag_db_id"] = 47
        snap["countries"] = {"GER": country}
        snap["tag_db_id_index"] = {"resolved": True, "key": "tag"}
        snap["measured_countries"] = {
            "known": True, "count": 1, "tags": ["GER"], "tag_db_ids": [47],
        }
        snap["supply"] = {
            "source": "M",
            "confidence": "division_province_proxy",
            "by_province": {
                "7081": {"disrupted_supply": [{
                    "owner": "GER", "owner_tag_db_id": 47, "node_type": 3,
                    "value": 0.04017, "value_raw": 4017, "exact": True,
                    "source": "CCountrySupplySystem.disrupted_supply",
                }],
            },
            },
            "r2_logistics_attrition": {
                "country_disrupted_supply_available": True,
                "country_disrupted_supply_rows": 1,
                "country_disrupted_supply_ordering":
                    "owner_tag_db_id_then_province_id_then_node_type",
                "country_disrupted_supply_authority": {
                    "schema_version": 1,
                    "authority": "exact_a729_bounded_owner_robin_hood",
                    "executable_sha256":
                        "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad",
                    "function_manifest_sha256":
                        "3833350a60768765d3a56b80c49be49ed5e3d188a4b1810698c006bf1cb34061",
                    "complete_roster_validation": True,
                    "scale_raw": 100_000,
                },
            },
        }
        return snap

    def test_a6_disrupted_supply_exact_contract_and_fail_closed_mutations(self):
        snap = self._a6_disrupted_supply_snapshot()
        self.assertEqual(schema.validate_snapshot(snap), [])

        wrong_owner = copy.deepcopy(snap)
        wrong_owner["supply"]["by_province"]["7081"]["disrupted_supply"][0][
            "owner_tag_db_id"
        ] = 48
        self.assertTrue(any(
            "owner does not join current D18" in problem
            for problem in schema.validate_snapshot(wrong_owner)
        ))

        wrong_count = copy.deepcopy(snap)
        wrong_count["supply"]["r2_logistics_attrition"][
            "country_disrupted_supply_rows"
        ] = 2
        self.assertTrue(any(
            "row count" in problem
            for problem in schema.validate_snapshot(wrong_count)
        ))

        stale_d18 = copy.deepcopy(snap)
        stale_d18["measured_countries"]["known"] = False
        self.assertTrue(any(
            "current resolved D18" in problem
            for problem in schema.validate_snapshot(stale_d18)
        ))

    def test_a6_disrupted_supply_proven_empty_is_distinct_from_unavailable(self):
        snap = self._a6_disrupted_supply_snapshot()
        snap["supply"]["by_province"] = {}
        snap["supply"]["r2_logistics_attrition"][
            "country_disrupted_supply_rows"
        ] = 0
        self.assertEqual(schema.validate_snapshot(snap), [])

        snap["supply"]["r2_logistics_attrition"].update({
            "country_disrupted_supply_available": False,
            "country_disrupted_supply_rows": None,
            "country_disrupted_supply_ordering": None,
            "country_disrupted_supply_authority": None,
        })
        self.assertEqual(schema.validate_snapshot(snap), [])

    def _a5_air_ace_snapshot(self):
        snap = schema.empty_snapshot("a5", "1940.9.10.13")
        country = schema.empty_country()
        country["_r2_tag_db_id"] = 47
        country["extra"] = {"_r2_air_ace_roster_live": {
            "schema_version": 1,
            "authority": "exact_a729_cace_owner_roster",
            "executable_sha256":
                "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad",
            "function_manifest_sha256":
                "f85162e5b2a531962e1080fb3add1867f4522a2aba7811d013a9f43c8e708720",
            "carrier_version": 2,
            "owner_tag_db_id": 47,
            "observed_country_rows": 1,
            "observed_ace_rows": 1,
            "owner_row_count": 1,
            "ordering": "country_ace_index_then_ace_id",
            "complete_roster_validation": True,
            "aces": [{
                "owner_tag_db_id": 47,
                "ace_id": 900,
                "country_ace_index": 0,
                "source_mask": 115,
                "name": "Max",
                "surname": "Mustermann",
                "callsign": "Falke",
                "alive": True,
                "handled": False,
                "kill_type_raw": 0,
                "lifecycle": "alive",
                "wing_id": 700,
                "killer_ace_id": None,
                "killer_country_tag_db_id": None,
            }],
        }}
        snap["countries"] = {"GER": country}
        snap["tag_db_id_index"] = {"resolved": True, "key": "tag"}
        snap["measured_countries"] = {
            "known": True, "count": 1, "tags": ["GER"], "tag_db_ids": [47],
        }
        return snap

    def test_a5_air_ace_authority_and_measured_empty_contract(self):
        snap = self._a5_air_ace_snapshot()
        self.assertEqual(schema.validate_snapshot(snap), [])
        sidecar = snap["countries"]["GER"]["extra"]["_r2_air_ace_roster_live"]
        sidecar["observed_ace_rows"] = 0
        sidecar["owner_row_count"] = 0
        sidecar["aces"] = []
        self.assertEqual(schema.validate_snapshot(snap), [])

    def test_a5_air_ace_rejects_owner_source_and_count_tampering(self):
        for field, value in (
            ("owner_tag_db_id", 48),
            ("observed_ace_rows", 2),
            ("authority", "fabricated"),
        ):
            snap = self._a5_air_ace_snapshot()
            snap["countries"]["GER"]["extra"]["_r2_air_ace_roster_live"][field] = value
            self.assertTrue(schema.validate_snapshot(snap))
        snap = self._a5_air_ace_snapshot()
        snap["countries"]["GER"]["extra"]["_r2_air_ace_roster_live"]["aces"][0][
            "source_mask"
        ] = 1
        self.assertTrue(schema.validate_snapshot(snap))

    def _x9_breakthrough_snapshot(self):
        snap = schema.empty_snapshot("g1", "1940.9.10.13")
        country = schema.empty_country()
        country["_r2_tag_db_id"] = 47
        snap["countries"] = {"GER": country}
        snap["special_projects"]["_r2_breakthroughs"] = [{
            "country": "GER",
            "tag_db_id": 47,
            "specialization": "specialization_air",
            "points_raw": 7932,
            "_r2_specialization_authority":
                "exact_a729_serializer_token_enum_join",
            "_r2_country_authority": "exact_country_identity",
        }]
        return snap

    def test_x9_portable_specialization_enum_validates_without_engine_handle(self):
        self.assertEqual(schema.validate_snapshot(self._x9_breakthrough_snapshot()), [])

    def test_x9_rejects_raw_handle_and_swapped_d18_identity(self):
        leaked = self._x9_breakthrough_snapshot()
        leaked["special_projects"]["_r2_breakthroughs"][0][
            "_r2_specialization_handle"
        ] = 22216
        self.assertTrue(any(
            "nonportable specialization handle" in problem
            for problem in schema.validate_snapshot(leaked)
        ))

        swapped = self._x9_breakthrough_snapshot()
        swapped["special_projects"]["_r2_breakthroughs"][0]["tag_db_id"] = 99
        self.assertTrue(any(
            "country/D18 identity invalid" in problem
            for problem in schema.validate_snapshot(swapped)
        ))

    def _d48_building_snapshot(self):
        snap = schema.empty_snapshot("g1", "1940.9.10.13")
        snap["states"]["42"] = {
            "construction_buildings_state_damage_live": {
                "source": "C",
                "state_buildings": {
                    "type_0x00002fab": {
                        "level": 5,
                        "healthy_level": 4,
                        "partial_health_raw": 8_184_501,
                        "damaged": True,
                        "token": "industrial_complex",
                    }
                },
            }
        }
        snap["provinces"]["2068"] = {
            "construction_buildings_state_damage_live": {
                "source": "C",
                "state_id": 42,
                "buildings": {
                    "type_0x00004cc1": {
                        "level": 2,
                        "healthy_level": 2,
                        "partial_health_raw": 0,
                        "damaged": False,
                        "token": "rail_way",
                    }
                },
            }
        }
        return snap

    def test_d48_building_sidecars_validate(self):
        self.assertEqual(schema.validate_snapshot(self._d48_building_snapshot()), [])

    def _d48_railway_snapshot(self):
        snap = schema.empty_snapshot("g1", "1940.9.10.13")
        country = schema.empty_country()
        country["production"]["construction_buildings_state_damage_live"] = {
            "source": "C",
            "railway_construction_authority": {
                "schema": "RLC1",
                "binary": "hoi4.exe 1.19.2.0.a729",
                "executable_sha256": schema.RAILWAY_CONSTRUCTION_EXECUTABLE_SHA256,
                "line_type": 4713,
                "vtable_rva": "0x02A15130",
                "fixed_point_divisor": 100000,
                "pause_state":
                    "derived_from_current_edge_access_and_combat_not_stored_cooldown",
            },
            "railway_construction_lines": [{
                "line_id": 135764,
                "queue_index": 1,
                "active_factories": 15,
                "cost_raw": 448000000,
                "produced_raw": 231350000,
                "created_date_raw": 60827637,
                "base_level": 2,
                "target_level": 3,
                "completion_flag": False,
                "current_segment_index": 3,
                "remaining_hours_raw": 216650000,
                "remaining_hours": 2166.5,
                "route_province_ids": [
                    9422, 11408, 9564, 3539, 11485, 9436, 3582, 9504, 3529,
                ],
            }],
        }
        snap["countries"] = {"GER": country}
        return snap

    def test_wave6_d48_rlc1_railway_sidecar_validates_exact_authority(self):
        self.assertEqual(schema.validate_snapshot(self._d48_railway_snapshot()), [])

    def test_wave6_d48_rlc1_rejects_torn_index_or_authority(self):
        snap = self._d48_railway_snapshot()
        sidecar = snap["countries"]["GER"]["production"][
            "construction_buildings_state_damage_live"
        ]
        sidecar["railway_construction_authority"]["vtable_rva"] = "0x0"
        sidecar["railway_construction_lines"][0]["current_segment_index"] = 8
        problems = schema.validate_snapshot(snap)
        self.assertTrue(any("RLC1 authority mismatch" in problem for problem in problems))
        self.assertTrue(any("current_segment_index outside route" in problem for problem in problems))

    def test_d48_building_sidecars_reject_incoherent_or_invented_rows(self):
        snap = self._d48_building_snapshot()
        state_buildings = snap["states"]["42"][
            "construction_buildings_state_damage_live"
        ]["state_buildings"]
        state_buildings["TYPE_0x00002fab"] = state_buildings.pop("type_0x00002fab")
        province_row = snap["provinces"]["2068"][
            "construction_buildings_state_damage_live"
        ]["buildings"]["type_0x00004cc1"]
        province_row["damaged"] = True
        province_row["partial_health_raw"] = 20_000_001

        probs = schema.validate_snapshot(snap)

        for token in (
            "invalid building type key",
            "partial_health_raw should be an int in 0..20000000",
            "damaged disagrees",
        ):
            self.assertTrue(any(token in problem for problem in probs), (token, probs))

    def _fired_event_snapshot(self):
        snap = schema.empty_snapshot("g1", "1940.9.10.13")
        snap["world_events"]["fired_events_complete"] = True
        snap["world_events"]["fired_events"] = [{
            "event_id": "test.1",
            "recipient_tag": "GER",
            "fire_date": "1940.9.10.13",
            "fire_hour": schema.date_to_hour_index("1940.9.10.13"),
            "fire_tick": 4001,
            "event_sequence": 7,
            "visible_option_ordinals": [0, 2],
            "chosen_option_ordinal": 2,
            "selected_desc_key": "test.1.d2",
            "shown_picture_gfx": "GFX_report_event_test",
            "popup_subtype": "country",
            "effective_timeout_hours": 312,
            "timeout_tick": 5000,
            "resolved_tick": 4010,
            "resolution_reason": "selected",
            "runtime_title_text_context_id": "event:title:7",
            "runtime_description_text_context_id": "event:description:7",
            "runtime_option_text_contexts": [
                {"definition_ordinal": 0, "context_id": "event:option:7:0"},
                {"definition_ordinal": 2, "context_id": "event:option:7:2"},
            ],
        }]
        return snap

    def test_xui21_fired_event_contract_validates(self):
        self.assertEqual(schema.validate_snapshot(self._fired_event_snapshot()), [])

    def test_xui21_fired_event_contract_rejects_inference_shapes(self):
        snap = self._fired_event_snapshot()
        row = snap["world_events"]["fired_events"][0]
        row["visible_option_ordinals"] = [2, 0]
        row["chosen_option_ordinal"] = 1
        row["popup_subtype"] = "guessed"
        row["timeout_tick"] = row["fire_tick"]
        row["resolution_reason"] = "pending"
        probs = schema.validate_snapshot(snap)
        for token in (
            "visible_option_ordinals", "chosen_option_ordinal", "popup_subtype",
            "timeout_tick", "resolution state inconsistent",
        ):
            self.assertTrue(any(token in problem for problem in probs), (token, probs))

    def test_xui21_fired_event_contract_rejects_popup_wide_or_misbound_text_contexts(self):
        snap = self._fired_event_snapshot()
        row = snap["world_events"]["fired_events"][0]
        row["runtime_text_context_id"] = "forbidden:popup-wide"
        row["runtime_option_text_contexts"] = [
            {"definition_ordinal": 1, "context_id": "event:option:7:1"},
            {"definition_ordinal": 1, "context_id": "event:option:7:duplicate"},
        ]
        probs = schema.validate_snapshot(snap)
        self.assertTrue(any("singular popup-wide join is forbidden" in p for p in probs), probs)
        self.assertTrue(any("definition_ordinal not visible" in p for p in probs), probs)
        self.assertTrue(any("not strictly ordered" in p for p in probs), probs)

    def test_xui21_lifecycle_only_and_timeout_default_are_valid(self):
        snap = self._fired_event_snapshot()
        ai = snap["world_events"]["fired_events"][0]
        ai["visible_option_ordinals"] = []
        ai["selected_desc_key"] = None
        ai["shown_picture_gfx"] = None
        ai["popup_subtype"] = None
        ai["runtime_title_text_context_id"] = None
        ai["runtime_description_text_context_id"] = None
        ai["runtime_option_text_contexts"] = []
        self.assertEqual(schema.validate_snapshot(snap), [])

        timeout = self._fired_event_snapshot()
        row = timeout["world_events"]["fired_events"][0]
        row["chosen_option_ordinal"] = 0
        row["visible_option_ordinals"] = [0, 2]
        row["resolution_reason"] = "timeout"
        self.assertEqual(schema.validate_snapshot(timeout), [])

    def test_xui21_complete_history_rejects_legacy_identity_rows(self):
        snap = self._fired_event_snapshot()
        snap["world_events"]["fired_events"] = ["test.1"]
        probs = schema.validate_snapshot(snap)
        self.assertTrue(
            any("legacy identity cannot satisfy fired_events_complete" in p for p in probs),
            probs,
        )

    def _active_ability_snapshot(self):
        fixture = (Path(__file__).parents[2] / "re_evidence" / "rendering_laws" /
                   "XUI23" / "canonical_active_abilities_envelope.json")
        raw = fixture.read_bytes()
        self.assertEqual(
            hashlib.sha256(raw).hexdigest(),
            fixture.with_suffix(".sha256").read_text(encoding="utf-8").split()[0],
        )
        snap = schema.empty_snapshot("g1", "1936.1.1.0")
        snap["countries"]["GER"] = schema.empty_country()
        snap["countries"]["GER"]["active_abilities"] = json.loads(raw)
        return snap

    def test_xui23_active_ability_contract_validates(self):
        self.assertEqual(schema.validate_snapshot(self._active_ability_snapshot()), [])

    def test_xui23_active_ability_contract_rejects_inference_shapes(self):
        snap = self._active_ability_snapshot()
        row = snap["countries"]["GER"]["active_abilities"]["snapshot"]["rows"][0]
        row["available"] = "yes"
        row["display_order"] = 2
        row["cooldown_remaining"] = {"kind": "hours", "hours": 505}
        row["tooltip"] = None
        row["affected_division_ids"] = ["division:9", "division:9"]
        probs = schema.validate_snapshot(snap)
        for token in (
            ".available", "display_order", "cooldown remaining exceeds total", ".tooltip",
            "affected_division_ids",
        ):
            self.assertTrue(any(token in problem for problem in probs), (token, probs))

    def test_xui23_active_ability_authority_mutations_reject(self):
        base = self._active_ability_snapshot()
        mutations = (
            lambda envelope: envelope["carrier"].__setitem__("executable_sha256", "0" * 64),
            lambda envelope: envelope["snapshot"].__setitem__(
                "source_record_id", "a729:a36:16959360:GER:1:leader:99"),
            lambda envelope: envelope["carrier"].__setitem__("frame_hour", 16959361),
            lambda envelope: envelope["snapshot"]["rows"][0].__setitem__(
                "cooldown_remaining", {"kind": "indefinite", "hours": 0}),
            lambda envelope: envelope["snapshot"]["rows"][0].__setitem__(
                "cost_command_power_scale", 1),
        )
        for mutate in mutations:
            snap = copy.deepcopy(base)
            mutate(snap["countries"]["GER"]["active_abilities"])
            self.assertTrue(schema.validate_snapshot(snap))

    def _trade_snapshot(self):
        snap = schema.empty_snapshot("g1", "1940.9.10.13")
        snap["countries"]["GER"] = schema.empty_country()
        resource_order = (
            "oil", "aluminium", "rubber", "tungsten", "steel", "chromium", "coal")
        snap["countries"]["GER"]["economy"]["resources"] = {
            "complete": True,
            "produced": {resource: 10.0 if resource == "oil" else 0.0 for resource in resource_order},
            "imported": {resource: 4.0 if resource == "oil" else 0.0 for resource in resource_order},
            "used": {resource: 8.0 if resource == "oil" else 0.0 for resource in resource_order},
            "to_export": {resource: 2.0 if resource == "oil" else 0.0 for resource in resource_order},
            "base_export": {resource: 3.0 if resource == "oil" else 0.0 for resource in resource_order},
            "exported": {resource: 1.0 if resource == "oil" else 0.0 for resource in resource_order},
            "available_derived": {"oil": 9.0},
        }
        resource_rows = []
        for visible_order, resource in enumerate(resource_order):
            resource_rows.append({
                "resource": resource, "visible_order": visible_order,
                "balance": 5.0 if resource == "oil" else 0.0,
                "produced": 10.0 if resource == "oil" else 0.0,
                "imported": 4.0 if resource == "oil" else 0.0,
                "exported": 1.0 if resource == "oil" else 0.0,
                "special_project": 0.0, "production_line": 8.0 if resource == "oil" else 0.0,
                "needed": -12.0 if resource == "oil" else 0.0,
                "needed_current": -12.0 if resource == "oil" else None,
                "needed_max": -20.0 if resource == "oil" else None,
                "needed_type": "current_max" if resource == "oil" else "surplus",
            })
        snap["countries"]["GER"]["economy"]["resource_trade"] = {
            "resource_rows_complete": True,
            "resource_rows": resource_rows,
            "deals_complete": True,
            "deals": [{
                "deal_id": 41, "counterpart": "SWE", "direction": "import",
                "resource": "oil", "requested": 8.0, "delivered": 6.0,
                "available_export": 12.0, "civilian_factories": 1,
                "factory_cost_per_resource": 0.125,
                "convoys": {"assigned": 2, "required": 3},
                "route_kind": "sea", "delivery_efficiency": 0.75,
                "no_trade_influence": False, "no_resources": False,
                "visible_order": 0,
            }],
            "factories": {"total": 25, "used": 11, "available": 14},
        }
        snap["countries"]["SWE"] = schema.empty_country()
        snap["countries"]["SWE"]["economy"]["resource_trade"] = {
            "deals_complete": True,
            "deals": [{
                "deal_id": 41, "counterpart": "GER", "direction": "export",
                "resource": "oil", "requested": 8.0, "delivered": 6.0,
                "available_export": 12.0, "civilian_factories": 1,
                "factory_cost_per_resource": 0.125,
                "convoys": {"assigned": 2, "required": 3},
                "route_kind": "sea", "delivery_efficiency": 0.75,
                "no_trade_influence": False, "no_resources": False,
                "visible_order": 0,
            }],
        }
        return snap

    def test_xui15_trade_contract_validates(self):
        self.assertEqual(schema.validate_snapshot(self._trade_snapshot()), [])
        self.assertEqual(
            schema.source_of("countries.GER.economy.resource_trade.deals.0.resource"),
            "M")

    def test_xui15_resource_pool_completeness_is_typed(self):
        snap = self._trade_snapshot()
        snap["countries"]["GER"]["economy"]["resources"]["complete"] = 1
        self.assertIn(
            "country 'GER'.economy.resources.complete 1 not bool|null",
            schema.validate_snapshot(snap))

    def test_xui15_trade_order_and_completeness_fail_closed(self):
        snap = self._trade_snapshot()
        trade = snap["countries"]["GER"]["economy"]["resource_trade"]
        trade["resource_rows"][0]["visible_order"] = 2
        trade["deals"] = None
        probs = schema.validate_snapshot(snap)
        self.assertTrue(any("resource_rows[0].visible_order" in p for p in probs), probs)
        self.assertTrue(any("deals_complete true" in p for p in probs), probs)

    def test_xui15_complete_trade_requires_exact_resources_and_typed_route_flags(self):
        snap = self._trade_snapshot()
        trade = snap["countries"]["GER"]["economy"]["resource_trade"]
        trade["resource_rows"][1]["resource"] = "steel"
        trade["deals"][0]["no_resources"] = 0
        del snap["countries"]["GER"]["economy"]["resources"]["used"]["coal"]
        probs = schema.validate_snapshot(snap)
        for token in (
            "exact seven-resource order", ".no_resources", "used complete without exactly seven",
        ):
            self.assertTrue(any(token in problem for problem in probs), (token, probs))

    def test_xui15_deal_requires_exact_reciprocal_half(self):
        snap = self._trade_snapshot()
        snap["countries"]["SWE"]["economy"]["resource_trade"]["deals"] = []
        probs = schema.validate_snapshot(snap)
        self.assertTrue(
            any("exactly two reciprocal halves" in problem for problem in probs),
            probs,
        )

    def test_xui15_mtrd_route_status_schema_is_world_atomic(self):
        snap = self._trade_snapshot()
        export = snap["countries"]["SWE"]["economy"]["resource_trade"]["deals"][0]
        del export["no_trade_influence"]
        del export["no_resources"]
        probs = schema.validate_snapshot(snap)
        self.assertTrue(any("reciprocal halves disagree" in problem for problem in probs), probs)

        snap = self._trade_snapshot()
        legacy_pair = copy.deepcopy(
            snap["countries"]["GER"]["economy"]["resource_trade"]["deals"][0]
        )
        legacy_pair["deal_id"] = 42
        legacy_pair["visible_order"] = 1
        legacy_pair["counterpart"] = "SOV"
        del legacy_pair["no_trade_influence"]
        del legacy_pair["no_resources"]
        snap["countries"]["GER"]["economy"]["resource_trade"]["deals"].append(legacy_pair)
        snap["countries"]["SOV"] = schema.empty_country()
        reciprocal = copy.deepcopy(legacy_pair)
        reciprocal["counterpart"] = "GER"
        reciprocal["direction"] = "export"
        reciprocal["visible_order"] = 0
        snap["countries"]["SOV"]["economy"]["resource_trade"] = {
            "deals_complete": True,
            "deals": [reciprocal],
        }
        probs = schema.validate_snapshot(snap)
        self.assertTrue(any("mixes legacy MTRD v1" in problem for problem in probs), probs)

    def test_xui15_factory_header_rejects_non_i32_values(self):
        snap = self._trade_snapshot()
        snap["countries"]["GER"]["economy"]["resource_trade"]["factories"]["used"] = 2 ** 31
        probs = schema.validate_snapshot(snap)
        self.assertTrue(any("factories.used" in problem and "not i32" in problem for problem in probs), probs)

    def test_hour_index_mismatch_flagged(self):
        snap = schema.empty_snapshot("g1", "1940.9.10.13")
        snap["hour_index"] = 12345  # wrong
        probs = schema.validate_snapshot(snap)
        self.assertTrue(any("hour_index" in p for p in probs))

    def test_bad_stability_flagged(self):
        snap = schema.empty_snapshot("g1", "1940.9.10.13")
        snap["countries"]["GER"] = schema.empty_country()
        snap["countries"]["GER"]["politics"]["stability"] = 5.0
        probs = schema.validate_snapshot(snap)
        self.assertTrue(any("stability" in p for p in probs))

    def test_duplicate_div_id_flagged(self):
        snap = schema.empty_snapshot("g1", "1940.9.10.13")
        d1 = schema.empty_division(); d1["div_id"] = 7
        d2 = schema.empty_division(); d2["div_id"] = 7
        snap["divisions"] = [d1, d2]
        probs = schema.validate_snapshot(snap)
        self.assertTrue(any("duplicate division" in p for p in probs))

    def test_collector_division_contract_accepts_typed_measured_row(self):
        snap = schema.empty_snapshot("g1", "1940.9.10.13")
        snap["provenance"]["collector"] = {"captured": True, "domains": ["divisions"]}
        div = schema.empty_division()
        div.update({
            "div_id": 7,
            "div_type": 4713,
            "owner": "GER",
            "location_province": 11596,
            "state_id": 64,
            "template_id": 22,
            "template_type": 52,
            "template_name": "Infantry",
            "organisation": 42.5,
            "strength": 0.97,
            "experience": 1.0,
            "moving": True,
            "move_target_province": 11597,
            "move_progress": 123.0,
            "in_combat": False,
            "supply_status": 0.8,
        })
        snap["divisions"] = [div]
        self.assertEqual(schema.validate_snapshot(snap), [])

    def test_collector_division_contract_rejects_raw_keys_and_bad_types(self):
        self.assertIn("divisions", schema.COLLECTOR_DOMAIN_CONTRACTS)
        snap = schema.empty_snapshot("g1", "1940.9.10.13")
        snap["provenance"]["collector"] = {
            "captured": True,
            "domains": ["divisions"],
        }
        div = schema.empty_division()
        div.update({
            "div_id": 7,
            "div_type": 1,
            "owner": "GER",
            "location_province": 11596,
            "organisation": 0.8,
            "strength": 0.9,
            "moving": False,
            "body_addr": "0x1234",
        })
        snap["divisions"] = [div]

        probs = schema.validate_snapshot(snap)

        self.assertTrue(any("raw pointer/tag-db" in p for p in probs), probs)

        div.pop("body_addr")
        div["move_progress"] = float("nan")
        div["moving"] = 1
        div["owner"] = "Germany"
        probs = schema.validate_snapshot(snap)
        self.assertTrue(any("move_progress" in p for p in probs), probs)
        self.assertTrue(any("moving" in p for p in probs), probs)
        self.assertTrue(any("owner" in p for p in probs), probs)

    def test_collector_division_contract_requires_div_type(self):
        snap = schema.empty_snapshot("g1", "1940.9.10.13")
        snap["provenance"]["collector"] = {"captured": True, "domains": ["divisions"]}
        div = schema.empty_division()
        div["div_id"] = 7
        snap["divisions"] = [div]
        probs = schema.validate_snapshot(snap)
        self.assertTrue(any("div_type" in p and "collector" in p for p in probs), probs)

    def test_legacy_division_without_type_still_validates(self):
        snap = schema.empty_snapshot("g1", "1940.9.10.13")
        div = schema.empty_division()
        div["div_id"] = 7
        snap["divisions"] = [div]
        self.assertEqual(schema.validate_snapshot(snap), [])

    def test_missing_container_flagged(self):
        snap = schema.empty_snapshot("g1", "1940.9.10.13")
        del snap["states"]
        probs = schema.validate_snapshot(snap)
        self.assertTrue(any("states" in p for p in probs))

    def test_bad_country_extra_flagged(self):
        snap = schema.empty_snapshot("g1", "1940.9.10.13")
        snap["countries"]["GER"] = schema.empty_country()
        snap["countries"]["GER"]["extra"] = "bad"
        probs = schema.validate_snapshot(snap)
        self.assertTrue(any("extra" in p and "dict|null" in p for p in probs))

    def test_bad_country_fuel_shape_flagged(self):
        snap = schema.empty_snapshot("g1", "1940.9.10.13")
        snap["countries"]["POR"] = schema.empty_country()
        snap["countries"]["POR"]["economy"]["fuel"] = "bad"
        probs = schema.validate_snapshot(snap)
        self.assertTrue(any("economy.fuel" in p and "dict|null" in p for p in probs))

    def test_bad_country_fuel_values_flagged(self):
        snap = schema.empty_snapshot("g1", "1940.9.10.13")
        snap["countries"]["POR"] = schema.empty_country()
        snap["countries"]["POR"]["economy"]["fuel"] = {
            "current": -1.0,
            "max": "bad",
            "gain": float("inf"),
            "cost": -42.0,
        }
        probs = schema.validate_snapshot(snap)
        self.assertTrue(any("economy.fuel.current" in p and "out of range" in p
                            for p in probs), probs)
        self.assertTrue(any("economy.fuel.max" in p and "not number|null" in p
                            for p in probs), probs)
        self.assertTrue(any("economy.fuel.gain" in p and "not number|null" in p
                            for p in probs), probs)
        self.assertFalse(any("economy.fuel.cost" in p for p in probs), probs)

    def test_bad_country_production_values_flagged(self):
        snap = schema.empty_snapshot("g1", "1940.9.10.13")
        snap["countries"]["POR"] = schema.empty_country()
        snap["countries"]["POR"]["production"].update({
            "stockpile": {"70": float("inf"), "bad": 1.0},
            "stockpile_total": "bad",
            "stockpile_complete": 1,
            "military_lines_complete": "yes",
            "military_lines": [{
                "id": True,
                "active_factories": -1,
                "cost": -1.0,
                "speed": 1.0,
                "produced": 2.0,
                "amount": -2,
                "priority": -1,
                "requested_factories": 1.5,
                "equipment_variant_id": "bad",
                "line_subtype": "unknown",
                "efficiency": -1.0,
                "progress_ratio": 2.0,
                "daily_output": float("inf"),
                "mio_id": {},
            }],
            "naval_lines": ["bad"],
            "military_factories": -1,
            "naval_factories": True,
        })
        probs = schema.validate_snapshot(snap)
        self.assertTrue(any("production.stockpile['70']" in p for p in probs), probs)
        self.assertTrue(any("production.stockpile key 'bad'" in p for p in probs), probs)
        self.assertTrue(any("production.stockpile_total" in p for p in probs), probs)
        self.assertTrue(any("stockpile_complete" in p for p in probs), probs)
        self.assertTrue(any("military_lines_complete" in p for p in probs), probs)
        self.assertTrue(any("military_lines[0].id" in p for p in probs), probs)
        self.assertTrue(any("military_lines[0].active_factories" in p
                            for p in probs), probs)
        self.assertTrue(any("military_lines[0].cost" in p for p in probs), probs)
        self.assertTrue(any("military_lines[0].amount" in p for p in probs), probs)
        for field in (
            "priority", "requested_factories", "equipment_variant_id", "line_subtype",
            "efficiency", "progress_ratio", "daily_output", "mio_id",
        ):
            self.assertTrue(any(f"military_lines[0].{field}" in p for p in probs), probs)
        self.assertTrue(any("naval_lines[0] not a dict" in p for p in probs), probs)
        self.assertTrue(any("military_factories" in p and "nonnegative int" in p
                            for p in probs), probs)
        self.assertTrue(any("naval_factories" in p and "nonnegative int" in p
                            for p in probs), probs)

    def test_complete_future_production_contract_validates(self):
        snap = schema.empty_snapshot("g1", "1940.9.10.13")
        snap["countries"]["POR"] = schema.empty_country()
        snap["countries"]["POR"]["production"].update({
            "stockpile": {},
            "stockpile_total": 0.0,
            "stockpile_complete": True,
            "military_lines_complete": True,
            "naval_lines_complete": True,
            "military_lines": [{
                "id": 7,
                "priority": 0,
                "line_subtype": "military",
                "equipment_variant_id": 7316,
                "active_factories": 4,
                "requested_factories": 6,
                "efficiency": 0.7,
                "efficiency_cap": 0.9,
                "efficiency_growth": 0.004,
                "cost": 4.0,
                "speed": 12.0,
                "produced": 2.0,
                "amount": -1,
                "progress_ratio": 0.5,
                "daily_output": 3.0,
                "days_to_next_unit": 0.1666666667,
                "mio_id": "por_mio",
            }],
            "naval_lines": [],
        })
        self.assertEqual(schema.validate_snapshot(snap), [])

    def test_bad_map_control_tags_flagged(self):
        snap = schema.empty_snapshot("g1", "1940.9.10.13")
        snap["states"]["1"] = schema.empty_state()
        snap["states"]["1"]["owner"] = "123"       # raw tag-db id, not a TAG
        snap["states"]["1"]["controller"] = "ger"  # lowercase is not a TAG
        snap["provinces"]["10"] = schema.empty_province()
        snap["provinces"]["10"]["controller"] = "??"
        probs = schema.validate_snapshot(snap)
        self.assertTrue(any("state '1'.owner" in p and "TAG|null" in p for p in probs),
                        probs)
        self.assertTrue(any("state '1'.controller" in p and "TAG|null" in p
                            for p in probs), probs)
        self.assertTrue(any("province '10'.controller" in p and "TAG|null" in p
                            for p in probs), probs)

    def test_bad_strategic_navy_shape_flagged(self):
        snap = schema.empty_snapshot("g1", "1940.9.10.13")
        snap["countries"]["GER"] = schema.empty_country()
        snap["countries"]["GER"]["extra"] = {
            "strategic_navy": {"max_allowed_dockyards_to_repair": "22"}}
        probs = schema.validate_snapshot(snap)
        self.assertTrue(any("strategic_navy.max_allowed" in p for p in probs))

    def test_complete_orders_group_manager_validates(self):
        snap = schema.empty_snapshot("g1", "1940.9.10.13")
        snap["order_of_battle"]["extra"] = {
            "army_battleplans_depth_live": {
                "known": True,
                "orders": [],
                "orders_group_manager": {
                    "known": True,
                    "complete": True,
                    "source": "registered_reference_type_53",
                    "registered_entry_count": 3,
                    "army_group_count": 1,
                    "armies": [
                        {"army_id": 17, "execution_type": 0,
                         "cohesion_type": 1, "leader_proximity": 2},
                        {"army_id": 18, "execution_type": 2,
                         "cohesion_type": 3, "leader_proximity": 4},
                    ],
                },
            },
        }
        self.assertEqual(schema.validate_snapshot(snap), [])

    def test_malformed_orders_group_manager_is_rejected(self):
        snap = schema.empty_snapshot("g1", "1940.9.10.13")
        snap["order_of_battle"]["extra"] = {
            "army_battleplans_depth_live": {
                "known": True,
                "orders": [],
                "orders_group_manager": {
                    "known": True,
                    "complete": True,
                    "source": "registered_reference_type_53",
                    "registered_entry_count": 1,
                    "army_group_count": 0,
                    "armies": [
                        {"army_id": 17, "execution_type": 3,
                         "cohesion_type": 1, "leader_proximity": 2},
                    ],
                },
            },
        }
        probs = schema.validate_snapshot(snap)
        self.assertTrue(any("execution_type" in problem for problem in probs), probs)

        manager = snap["order_of_battle"]["extra"]["army_battleplans_depth_live"][
            "orders_group_manager"
        ]
        manager["armies"][0]["execution_type"] = 2
        manager["object_ptr"] = "0x200000000"
        probs = schema.validate_snapshot(snap)
        self.assertTrue(any("wrong exact field set" in problem for problem in probs), probs)
        del manager["object_ptr"]
        manager.update({
            "known": False,
            "complete": False,
            "source": None,
            "registered_entry_count": None,
            "army_group_count": None,
        })
        probs = schema.validate_snapshot(snap)
        self.assertTrue(any("armies should be empty" in problem for problem in probs), probs)

    def test_v2_orders_group_manager_authenticates_typed_army_group_order(self):
        snap = schema.empty_snapshot("g1", "1940.9.10.13")
        snap["order_of_battle"]["extra"] = {
            "army_battleplans_depth_live": {
                "known": True,
                "orders": [{
                    "owner_kind": "army_group", "owner_id": 23, "army_id": None,
                }],
                "orders_group_manager": {
                    "known": True,
                    "complete": True,
                    "source": "registered_reference_type_53",
                    "registered_entry_count": 2,
                    "army_group_count": 1,
                    "armies": [
                        {"army_id": 17, "execution_type": 0,
                         "cohesion_type": 1, "leader_proximity": 2},
                    ],
                    "army_group_ids": [23],
                },
            },
        }
        self.assertEqual(schema.validate_snapshot(snap), [])

        manager = snap["order_of_battle"]["extra"]["army_battleplans_depth_live"][
            "orders_group_manager"
        ]
        manager["army_group_ids"] = [24]
        probs = schema.validate_snapshot(snap)
        self.assertTrue(any("lacks complete manager identity" in p for p in probs), probs)

        manager["army_group_ids"] = [17]
        probs = schema.validate_snapshot(snap)
        self.assertTrue(any("across owner kinds" in p for p in probs), probs)

    def test_typed_army_group_order_requires_known_complete_manager_census(self):
        snap = schema.empty_snapshot("g1", "1940.9.10.13")
        domain = {
            "known": True,
            "orders": [{
                "owner_kind": "army_group", "owner_id": 23, "army_id": None,
            }],
        }
        snap["order_of_battle"]["extra"] = {
            "army_battleplans_depth_live": domain,
        }
        probs = schema.validate_snapshot(snap)
        self.assertTrue(any("required for an army-group owner" in p for p in probs), probs)

        domain["orders_group_manager"] = {
            "known": False,
            "complete": False,
            "source": None,
            "registered_entry_count": None,
            "army_group_count": None,
            "armies": [],
            "army_group_ids": [],
        }
        probs = schema.validate_snapshot(snap)
        self.assertTrue(any("cannot be unavailable" in p for p in probs), probs)

    def test_unavailable_battleplan_domain_rejects_stale_orders(self):
        snap = schema.empty_snapshot("g1", "1940.9.10.13")
        snap["order_of_battle"]["extra"] = {
            "army_battleplans_depth_live": {
                "known": False,
                "orders": [{"army_id": 17}],
                "orders_group_manager": {
                    "known": False,
                    "complete": False,
                    "source": None,
                    "registered_entry_count": None,
                    "army_group_count": None,
                    "armies": [],
                },
            },
        }
        probs = schema.validate_snapshot(snap)
        self.assertTrue(any("orders should be empty" in problem for problem in probs), probs)

    def test_event_validator(self):
        good = schema.make_event("focus_completed", subject="GER", key="X", to="X",
                                 source="D", confidence="interval")
        self.assertEqual(schema.validate_event(good), [])
        bad = schema.make_event("not_a_type", confidence="weird", source="??")
        self.assertTrue(schema.validate_event(bad))


class TestWave4ExactSchema(unittest.TestCase):
    EXECUTABLE = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"

    def _base(self):
        return schema.empty_snapshot("wave4", "1936.1.1.0")

    def _b4_candidate_snapshot(self):
        snap = self._base()
        ger = schema.empty_country()
        eng = schema.empty_country()
        ger["_r2_tag_db_id"] = 4
        eng["_r2_tag_db_id"] = 9
        snap["countries"] = {"GER": ger, "ENG": eng}
        snap["measured_countries"] = {
            "known": True,
            "count": 2,
            "tags": ["GER", "ENG"],
            "tag_db_ids": [4, 9],
        }
        snap["_r2_sunk_convoy_history_live_candidate"] = {
            "schema_version": 3,
            "authority": "static_exact20_evidence_candidate",
            "production_activation_authorized": False,
            "frame_hour": snap["hour_index"],
            "source_sequence": 2,
            "executable_sha256": self.EXECUTABLE,
            "owner_serializer_manifest_sha256":
                "2fd64c0a861d38e252fe262d52e62cbb4a5efa7f94ee7ab374c33e8ac368827e",
            "owner_serializer_artifact_sha256":
                "bc679b4a175e75253a1fea1bfde625ccf8594ec06c562699aeaac75b9eaddf3a",
            "row_mutation_manifest_sha256":
                "b36682b2f942e7e4e7c8ae9dbfc05e2b52ee46dcf388fc9ad6e1fee24fdb802e",
            "row_serializer_artifact_sha256":
                "005235886a2d847b111df481ae948395828ec999e3cebe39612d36d364fefe98",
            "mutation_artifact_sha256":
                "2fa2e8a326081f82735b3824dc8b693130605a956964a170f1b5538e4ac1ef14",
            "live_fixture_receipt_sha256": "0" * 64,
            "owner_root": "CGameState+0x5A8",
            "capacity_count_word": "CGameState+0x5B0",
            "row_vtable_rva": "0x295D008",
            "row_extent": 0x18,
            "observed_rows": 1,
            "iterator_rows": 1,
            "row_count": 1,
            "ordering": "engine_first_seen_aggregate_identity",
            "d18_identity_complete": True,
            "rows": [{
                "source_ordinal": 0,
                "month": 10,
                "convoys": 2,
                "killer_country_tag_db_id": 4,
                "owner_tag_db_id": 9,
                "killer_country": "GER",
                "owner": "ENG",
            }],
        }
        snap["_r2_sunk_convoy_history_live"] = None
        snap["world_events"]["sunk_convoys"] = None
        snap["world_events"]["sunk_convoys_ordering"] = None
        snap["world_events"]["sunk_convoys_route_attribution"] = None
        return snap

    def test_b4_candidate_requires_current_frame_unique_d18_and_stays_dormant(self):
        snap = self._b4_candidate_snapshot()
        self.assertEqual(schema.validate_wave4_snapshot(snap), [])

        stale = copy.deepcopy(snap)
        stale["measured_countries"]["known"] = False
        self.assertTrue(any(
            "sunk_convoy_history_live_candidate" in problem
            for problem in schema.validate_wave4_snapshot(stale)
        ))

        duplicate = copy.deepcopy(snap)
        duplicate["countries"]["ENG"]["_r2_tag_db_id"] = 4
        duplicate["_r2_sunk_convoy_history_live_candidate"]["rows"][0][
            "owner_tag_db_id"
        ] = 4
        self.assertTrue(any(
            "D18 join invalid" in problem
            for problem in schema.validate_wave4_snapshot(duplicate)
        ))

    def _project_authority(self, owner_rows, breakthrough_rows, walked):
        return {
            "schema": "a729_d41_project_authority_v1", "status": "exact",
            "record_sequence": 1, "game_hour_raw": 0, "source_status_flags": 0,
            "executable_sha256": self.EXECUTABLE,
            "a20_static_receipt_sha256": "f10a9cad9e53c660b50364f5a242891013be38f77cb873b870aede44da1d8f3a",
            "x9_producer_manifest_sha256": "86f3aa3a916a203ea657cda01a84fb297e1812e03df59b481b004b16e0ad7ff6",
            "d18_identity_sha256": "0f0afc254220cdfac660083a6e7225b7e1d1b6eed3ae9e6bd935f89c97359ec1",
            "d18_country_count": 1, "owner_progress_row_count": owner_rows,
            "breakthrough_row_count": breakthrough_rows, "walked_countries": walked,
            "aborted_countries": 0, "authority_flags": 15,
            "carrier_payload_sha256": "11" * 32,
        }

    def test_native_special_projects_null_and_id_map_variants(self):
        unavailable = self._base()
        unavailable["special_projects"] = None
        self.assertEqual(schema.validate_snapshot(unavailable), [])

        native = self._base()
        native["special_projects"] = {
            "facilities": {}, "projects": {}, "scientists": {},
            "project_lifecycle": None, "project_progress": None,
            "reward_history": None, "_r2_header_extension": 5,
            "_r2_project_states": {}, "_r2_breakthroughs": [],
            "count": 0, "source": "scan",
        }
        self.assertEqual(schema.validate_snapshot(native), [])

    def test_a17_exact_projection_and_d18_join(self):
        snap = self._base()
        country = schema.empty_country()
        country["_r2_tag_db_id"] = 17
        country["extra"] = {
            "nuke_stockpile_tag_db_id": 17,
            "nukes_total": 1,
            "nukes_ready_total": 2,
            "nukes": [
                {"amount": 1, "nukes_ready": 2},
                {"amount": 0, "nukes_ready": 0},
            ],
        }
        snap["countries"]["GER"] = country
        snap["_r2_nuke_stockpile_live"] = {
            "schema_version": 2,
            "authority": "exact_a729_nuke_stockpile",
            "executable_sha256": self.EXECUTABLE,
            "producer_consumer_manifest_sha256": "0968ed85cd6103149ebd51415d6b64d1959bb05d1aff4cbfce9d31e437a6995c",
            "container_constructor_manifest_sha256": "233962bab3a8ca79150977f1f7e77312d1bbd262e89e582d1c01caaf01a89032",
            "cnuke_vtable_artifact_sha256": "cfd9d9e4088b4998398e579a5ebee222534868bad8a4529240b370faf467adbb",
            "elapsed_wall_ms": 5,
            "frame_hour": snap["hour_index"],
            "source_sequence": 1,
            "countries": [{
                "tag_db_id": 17, "container_capacity": 2,
                "nukes_total": 1, "nukes_ready_total": 2,
                "nukes": [
                    {"nuke_type_raw": 0, "amount_raw": 100_000_000, "amount": 1, "nukes_ready": 2},
                    {"nuke_type_raw": 1, "amount_raw": 0, "amount": 0, "nukes_ready": 0},
                ],
            }],
        }
        self.assertEqual(schema.validate_snapshot(snap), [])
        wrong_capacity = copy.deepcopy(snap)
        wrong_capacity["_r2_nuke_stockpile_live"]["countries"][0]["container_capacity"] = 1
        self.assertTrue(any("container_capacity" in p for p in schema.validate_snapshot(wrong_capacity)))
        wrong_type = copy.deepcopy(snap)
        wrong_type["_r2_nuke_stockpile_live"]["countries"][0]["nukes"][1]["nuke_type_raw"] = 0
        self.assertTrue(any("type order" in p for p in schema.validate_snapshot(wrong_type)))
        snap["countries"]["GER"]["extra"]["nukes_total"] = 2
        self.assertTrue(any("canonical country projection" in p for p in schema.validate_snapshot(snap)))

    def test_a18_known_empty_and_transition_join(self):
        snap = self._base()
        snap["measured_countries"] = {
            "known": True, "source": "country_identity", "count": 0,
            "tag_db_ids": [], "tags": [], "note": "measured empty",
        }
        snap["_r2_dynamic_modifier_lifecycle_live"] = {
            "schema_version": 3,
            "authority": "exact_a729_dynamic_modifier_instance",
            "source_binary_sha256": self.EXECUTABLE,
            "static_authority_receipt_sha256": "bdbac3cb1158c971b408586611d6aba618ecfe85b5265e3b9e8c2c0bf545e415",
            "elapsed_wall_ms": 1, "frame_hour": snap["hour_index"],
            "continuity": "baseline", "transitions": [],
        }
        self.assertEqual(schema.validate_snapshot(snap), [])

        country = schema.empty_country()
        country["dynamic_modifier_instances"] = [{
            "source_ordinal": 0, "modifier": "test_modifier", "enabled": True,
            "values_raw": [10], "value_scale": 100_000, "remaining_days": 0,
        }]
        snap["countries"]["GER"] = country
        snap["measured_countries"].update({"count": 1, "tag_db_ids": [17], "tags": ["GER"]})
        snap["_r2_dynamic_modifier_lifecycle_live"]["transitions"] = [{
            "owner_tag_db_id": 17, "owner_tag": "GER", "modifier": "test_modifier",
            "kind": "baseline_present", "previous_remaining_days": None,
            "current_remaining_days": 0, "expiry_classification": None,
        }]
        self.assertEqual(schema.validate_snapshot(snap), [])
        snap["countries"]["GER"]["dynamic_modifier_instances"][0]["remaining_days"] = None
        self.assertTrue(any("current instance projection" in p for p in schema.validate_snapshot(snap)))

    def test_a18_v3_synchronous_removal_stays_absent_from_current_set(self):
        snap = self._base()
        country = schema.empty_country()
        country["dynamic_modifier_instances"] = []
        snap["countries"]["GER"] = country
        snap["measured_countries"] = {
            "known": True, "source": "country_identity", "count": 1,
            "tag_db_ids": [17], "tags": ["GER"], "note": "current roster",
        }
        snap["_r2_dynamic_modifier_lifecycle_live"] = {
            "schema_version": 3, "authority": "exact_a729_dynamic_modifier_instance",
            "source_binary_sha256": self.EXECUTABLE,
            "static_authority_receipt_sha256": "bdbac3cb1158c971b408586611d6aba618ecfe85b5265e3b9e8c2c0bf545e415",
            "elapsed_wall_ms": 1, "frame_hour": snap["hour_index"],
            "continuity": "continuous", "transitions": [{
                "owner_tag_db_id": 17, "owner_tag": "GER", "modifier": "timed_modifier",
                "kind": "removed", "previous_remaining_days": 0,
                "current_remaining_days": None,
                "expiry_classification": "removed_after_observed_zero",
            }],
        }
        self.assertEqual(schema.validate_snapshot(snap), [])
        snap["countries"]["GER"]["dynamic_modifier_instances"] = [{
            "source_ordinal": 0, "modifier": "timed_modifier", "enabled": True,
            "values_raw": [], "value_scale": 100_000, "remaining_days": 0,
        }]
        self.assertTrue(any("current instance projection" in p for p in schema.validate_snapshot(snap)))

    def test_a20_project_progress_bounds_and_owner_join(self):
        snap = self._base()
        country = schema.empty_country(); country["_r2_tag_db_id"] = 17
        snap["countries"]["GER"] = country
        snap["special_projects"]["project_progress"] = {
            "authority": "exact_a729_owner_joined_project_progress",
            "countries": {"GER": [{
                "local_id": 1, "static_def_handle": 2, "current_state_offset": 0x18,
                "current_state_enum": 1, "prototype_state_enum": 2,
                "simple_state_1_enum": 3, "simple_state_2_enum": 4,
                "simple_state_3_enum": 5, "prototype_iterations": 6,
                "phase_progress_raw": 7, "phase_target_raw": 8, "completed": False,
            }]},
            "row_count": 1, "walked_countries": 1, "aborted_countries": 0,
        }
        # PPA1 seals both exact inner carriers.  A measured-empty X9 list is
        # therefore required; null means the breakthrough carrier was absent.
        snap["special_projects"]["_r2_breakthroughs"] = []
        snap["special_projects"]["_r2_project_authority"] = self._project_authority(1, 0, 1)
        self.assertEqual(schema.validate_snapshot(snap), [])
        snap["special_projects"]["project_progress"]["countries"]["GER"][0]["phase_target_raw"] = 0
        self.assertTrue(any("exact bounds" in p for p in schema.validate_snapshot(snap)))

    def test_a23_x9_and_runtime_text_semantics(self):
        snap = self._base()
        country = schema.empty_country(); country["_r2_tag_db_id"] = 17
        country["special_project_programs"] = [{
            "program_id": 4, "province": 12, "unread_prototype_rewards": 0,
        }]
        snap["countries"]["GER"] = country
        snap["special_projects"]["_r2_breakthroughs"] = [{
            "country": "GER", "tag_db_id": 17, "specialization": "specialization_air",
            "points_raw": 10,
            "_r2_specialization_authority": "exact_a729_serializer_token_enum_join",
            "_r2_country_authority": "exact_country_identity",
        }]
        snap["special_projects"]["project_progress"] = {
            "authority": "exact_a729_owner_joined_project_progress",
            "countries": {}, "row_count": 0, "walked_countries": 0,
            "aborted_countries": 0,
        }
        snap["special_projects"]["_r2_project_authority"] = self._project_authority(0, 1, 0)
        empty_hash = hashlib.sha256(b"").hexdigest()
        snap["runtime_text"] = {
            "schema": "hoi4-runtime-text-v1", "state": "measured",
            "contexts_complete": True, "contexts": [{
                "context_id": "test:reason", "frame_hour": snap["hour_index"],
                "pov_country_or_global": "GER", "view_id": "leader_traits",
                "element_id": "trait_tooltip", "source": {"kind": "authored_token", "identity": "trait_reason"},
                "final_text": {"bytes_hex": "", "utf8": "", "sha256": empty_hash},
                "variables": {}, "data_functions": {}, "dynamic_colors": {},
                "layout": {"mode": "authored_reproducible"},
                "producer": {"setter_entry_rva": 0x022B59E0, "setter_caller_rva": None,
                             "formatter_entry_rva": None, "identity_source": "authored_widget_registration"},
            }],
        }
        self.assertEqual(schema.validate_snapshot(snap), [])
        snap["special_projects"]["_r2_breakthroughs"][0]["specialization"] = None
        snap["special_projects"]["_r2_breakthroughs"][0]["_r2_specialization_authority"] = None
        self.assertTrue(any("project_authority" in p for p in schema.validate_snapshot(snap)))

    def test_a25_captured_runtime_mixed_order_and_exact_authority(self):
        snap = self._base()
        final_bytes = b"A\x13command_power"
        snap["runtime_text"] = {
            "schema": "hoi4-runtime-text-v1", "state": "measured",
            "contexts_complete": True, "contexts": [{
                "context_id": "ability:cost", "frame_hour": snap["hour_index"],
                "pov_country_or_global": "global", "view_id": "ability_tooltip",
                "element_id": "daily_cost", "source": {
                    "kind": "authored_token", "identity": "ABILITY_TOOLTIP_DAILY_COST",
                },
                "final_text": {
                    "bytes_hex": final_bytes.hex(),
                    "utf8": final_bytes.decode("utf-8"),
                    "sha256": hashlib.sha256(final_bytes).hexdigest(),
                },
                "variables": {}, "data_functions": {}, "dynamic_colors": {},
                "layout": {
                    "mode": "captured_runtime", "clip": [0, 0, 200, 24],
                    "glyphs": [{
                        "run_ordinal": 0, "font": "hoi_16mbs", "glyph_id": 65,
                        "x": 0, "y": 0, "rgba": [255, 255, 255, 255],
                    }],
                    "inline_icons": [{
                        "run_ordinal": 1, "sprite": "GFX_command_power", "frame": 0,
                        "x": 12, "y": 0, "baseline": 0, "advance": 20,
                    }],
                },
                "producer": {
                    "setter_entry_rva": 0x022B59E0, "setter_caller_rva": None,
                    "formatter_entry_rva": 0x02232AB0,
                    "identity_source": "authored_widget_registration",
                },
            }],
        }
        self.assertEqual(schema.validate_snapshot(snap), [])

        snap["runtime_text"]["contexts"][0]["layout"]["inline_icons"][0]["run_ordinal"] = 0
        self.assertTrue(any("mixed glyph/icon run order" in p for p in schema.validate_snapshot(snap)))
        snap["runtime_text"]["contexts"][0]["layout"]["inline_icons"][0]["run_ordinal"] = 1
        snap["runtime_text"]["contexts"][0]["layout"]["clip"] = [0, 0, 0, 24]
        self.assertTrue(any("layout.clip invalid" in p for p in schema.validate_snapshot(snap)))
        snap["runtime_text"]["contexts"][0]["layout"]["clip"] = [0, 0, 200, 24]
        snap["runtime_text"]["contexts"][0]["producer"]["setter_entry_rva"] = 0x1732DAD0
        self.assertTrue(any("producer exact authority invalid" in p for p in schema.validate_snapshot(snap)))

    def test_a30_exact_intel_level_and_d18_swap(self):
        snap = self._base()
        for tag, ident in (("GER", 17), ("ENG", 18)):
            country = schema.empty_country(); country["_r2_tag_db_id"] = ident
            snap["countries"][tag] = country
        snap["diplomacy_world"]["domain53_extra"] = {"intel_level_rows": [{
            "owner": "GER", "target": "ENG", "owner_tag_db_id": 17,
            "target_tag_db_id": 18, "civilian_raw": 1, "army_raw": 2,
            "navy_raw": 3, "air_raw": 4, "scale": 100_000,
            "lane_order": ["civilian", "army", "navy", "air"],
            "producer": "FUN_141417F60",
        }]}
        self.assertEqual(schema.validate_snapshot(snap), [])
        snap["diplomacy_world"]["domain53_extra"]["intel_level_rows"][0]["target_tag_db_id"] = 17
        self.assertTrue(any("intel level" in p for p in schema.validate_snapshot(snap)))

    def test_a33_sunk_ship_v2_empty_sequence_and_tail(self):
        snap = self._base()
        snap["_r2_sunk_ship_history_live"] = {
            "schema_version": 2, "authority": "exact_a729_sunk_ship_writer",
            "executable_sha256": self.EXECUTABLE,
            "function_manifest_sha256": "466880cc290e8961dab48c8323346d9d862089d85bbe0f4ba285b924cc959c82",
            "data_artifact_sha256": "5e0ed3e76724032f3e2f4cc5bcc9e5c38347ed862e0489821be8b7bbdd45ca21",
            "writer_sentinel_artifact_sha256": "dccd61e7573317458980fc0796fcba2ba945a434fbacbf3303a588c620307a4a",
            "all_writer_routes_manifest_sha256": "d02f0f1aaf2369db9dc9529062f5e8f59f3f6f751fb9e4c0ab81d47dea1e1de0",
            "csunk_ship_vtable_artifact_sha256": "61140fad7a24c150b60935580374129321719555bb5a4e1db88bd2bcd296937e",
            "csunk_ship_virtuals_manifest_sha256": "755a0011376d06bbd38456a2bb009236d99f4481a9b076789d9d57729c5673fd",
            "attacker_getters_manifest_sha256": "946eff0fb077863ce2dc1f1607ac2c0b9fcfee8f3377662ff14315101c30cbd6",
            "elapsed_wall_ms": 1, "frame_hour": snap["hour_index"],
            "source_sequence": 1, "producer_complete": True, "events": [],
        }
        self.assertEqual(schema.validate_snapshot(snap), [])
        snap["_r2_sunk_ship_history_live"]["source_sequence"] = 2
        self.assertTrue(any("history tail" in p for p in schema.validate_snapshot(snap)))

        history = snap["_r2_sunk_ship_history_live"]
        history["source_sequence"] = 2
        history["events"] = [{
            "event_sequence": 2, "combat_id": 9, "combat_generation": 3,
            "victim_country_id_raw": 2, "attacker_country_id_raw": 101,
            "ui_source_country_id_raw": 1, "sink_date_raw": snap["hour_index"],
            "victim_member_battle_slot_index": 4,
            "killed": True, "assisted_kill": False,
            "victim_ship_reference_raw": [7, 8],
            "damage_source_reference_raw": [0, 0],
            "task_force_id_raw": 0, "terminal_context_raw": [12, 13],
            "cause_record_flag": False,
            "ship_name": "HMS Exact", "ship_class_name": "Destroyer",
            "attribution_source_name": "",
            "attribution_source_class_name": "",
        }]
        self.assertEqual(schema.validate_snapshot(snap), [])
        history["events"][0]["victim_ship_reference_raw"] = [0, 0]
        self.assertTrue(any("writer event" in p for p in schema.validate_snapshot(snap)))

    def test_a34_decision_panel_v2_receipt_metadata_and_empty_bijection(self):
        snap = self._base()
        snap["_r2_decision_panel_live"] = {
            "schema_version": 2, "authority": "exact_a729_decision_panel",
            "executable_sha256": self.EXECUTABLE,
            "function_manifest_sha256": "712233abfbd1352f0c84ede6267a99f4de8f5406cabfd20d76046a094b5fec0a",
            "data_artifact_sha256": "ccca7fbfe0378821f22ed82d1a371a58ea12c24a9601fd3dd36d97bef093f0ea",
            "country_view_owner_tag_manifest_sha256":
                "a31b9fa4d886d509ceceef46819b6048331ef65065c2f07a8cc086e0868dd07a",
            "elapsed_wall_ms": 1, "frame_hour": snap["hour_index"], "source_sequence": 1,
            "producer_complete": True, "country_tag": "ENG",
            "decision_catalog_sha256": "c" * 64, "materialization_qpc_ticks": 90,
            "selected_present": {
                "frame_hour": snap["hour_index"], "selected_present_ordinal": 0,
                "selected_present_qpc_ticks": 100, "qpc_frequency_hz": 10_000_000,
                "selected_present_receipt_sha256": "d" * 64,
            },
            "grid": {
                "clip_left": 0, "clip_top": 0, "clip_right": 502, "clip_bottom": 600,
                "content_origin_x": 0, "content_origin_y": 0,
                "content_extent_width": 502, "content_extent_height": 0,
                "scroll_min": 0, "scroll_max": 0, "scroll_target": 0, "scroll_current": 0,
            },
            "children": [], "available_alert_count": 0, "timeout_alert_count": 0,
            "rows": [],
        }
        self.assertEqual(schema.validate_snapshot(snap), [])
        snap["_r2_decision_panel_live"]["materialization_qpc_ticks"] = 101
        self.assertTrue(any("receipt metadata" in p for p in schema.validate_snapshot(snap)))
        panel = snap["_r2_decision_panel_live"]
        panel["materialization_qpc_ticks"] = 90
        panel["grid"]["content_extent_height"] = 58
        panel["children"] = [{
            "presentation_index": 0, "kind": "category", "decision_row_index": None,
            "identity": "political", "name_text": "Political Actions",
            "secondary_text": "Exact category description", "expanded": True,
            "x": 0, "y": 0, "width": 502, "height": 58,
        }]
        self.assertEqual(schema.validate_snapshot(snap), [])
        panel["children"][0]["expanded"] = None
        self.assertTrue(any("heterogeneous child" in p for p in schema.validate_snapshot(snap)))

    def test_a38_committed_empty_authority_and_ordered_tombstones(self):
        snap = self._base()
        snap["_r2_scripted_gui_runtime_live"] = {
            "schema_version": 1, "authority": "exact20_a38_engine_materialized",
            "elapsed_wall_ms": 1, "frame_hour": snap["hour_index"],
            "producer_record_identity": "a38:committed-empty",
            "executable_sha256": self.EXECUTABLE,
            "ui14_engine_vfs_sha256": "4" * 64,
            "static_authority_job_id": "20260720T020812Z-w3-a38-controller-owner-41ee5208",
            "static_authority_receipt_sha256": "7d0a36b1822faf85a949d628a79bc4a09f95dc9c9aa3ed4aecb715b72b0a9082",
            "producer_manifest_sha256": "6" * 64,
            "manager_order": [],
            "tombstones": [{"country_tag": "USA", "gui_id": "usa_congress"}],
        }
        self.assertEqual(schema.validate_snapshot(snap), [])
        snap["_r2_scripted_gui_runtime_live"]["tombstones"].append(
            {"country_tag": "USA", "gui_id": "usa_congress"}
        )
        self.assertTrue(any("ordered tombstone" in p for p in schema.validate_snapshot(snap)))

    def test_a31_a39_exact_headers_and_runtime_reason_join(self):
        snap = self._base()
        country = schema.empty_country(); country["_r2_tag_db_id"] = 17
        snap["countries"]["GER"] = country
        snap["_r2_national_focus_presentation_live"] = {
            "schema_version": 2, "authority": "exact_a729_national_focus_view_items",
            "executable_sha256": self.EXECUTABLE,
            "function_manifest_sha256": "b319dd1b721ed6a13e47088b7def034111f9882631080bffbf049fbb79265a2f",
            "job_receipt_sha256": "b1c9d8c0678956f17bea2e7f9e06e7ff4baf31254c11b3c983a9a5ca9ae2d04b",
            "elapsed_wall_ms": 1, "frame_hour": snap["hour_index"],
            "materialization_qpc_ticks": 90,
            "selected_present": {
                "frame_hour": snap["hour_index"], "selected_present_global_order": 7,
                "selected_present_qpc_ticks": 100, "qpc_frequency_hz": 10_000_000,
                "selected_present_receipt_sha256": "d" * 64,
            },
            "source_sequence": 9,
            "producer_complete": True, "country_tag": "GER", "view_id": "nationalfocusview:GER",
            "producer_record_identity": f"a729:a31:{snap['hour_index']}:GER:9",
            "expected_visible_item_count": 0, "items": [],
        }
        character_hashes = {
            "trait_collection_manifest_sha256": "837c8b4db6a6cbc03fa5c950dc24c814f5156d6cf0de08f5041bfcccab7d78c9",
            "trait_state_manifest_sha256": "63ca92c9e5238a0a342e24209417188b7ceeabd7f01a21c45475f58bb3e29e8c",
            "trait_progress_manifest_sha256": "134d8fc1d5056e05023ebd46e33bda913ba3dde6c1f0945d423297d464dae547",
            "ace_refresh_manifest_sha256": "4be836cc67b9f59d748a05a73fa4c5d9c46db357d283d25b756c7ad18fcb2674",
            "ace_bind_sort_manifest_sha256": "ea545ce9457982590bd42741d1f573a7874594b7c3f836bf290ae8a276f83396",
            "ace_modifier_setter_artifact_sha256": "3b49c9eb735a03fc711f7322b74a5b86ad64b25ffdbcaa73a390b64a2edfd552",
            "ace_modifier_formatter_manifest_sha256": "79fd3a1e1b9eafb09566b2721260936c8430acffcf0ba408cc866c6292ac8f0b",
            "ace_selected_context_manifest_sha256": "b593bfc287648ac2bdced54567db5241f2acfe4032f8aed2a77632e29b468c5c",
        }
        empty_hash = hashlib.sha256(b"").hexdigest()
        snap["runtime_text"] = {
            "schema": "hoi4-runtime-text-v1", "state": "measured",
            "contexts_complete": True, "contexts": [{
                "context_id": "a39:trait:reason", "frame_hour": snap["hour_index"],
                "pov_country_or_global": "GER", "view_id": "leader_traits",
                "element_id": "trait_reason", "source": {
                    "kind": "authored_token", "identity": "trait_reason",
                },
                "final_text": {"bytes_hex": "", "utf8": "", "sha256": empty_hash},
                "variables": {}, "data_functions": {}, "dynamic_colors": {},
                "layout": {"mode": "authored_reproducible"},
                "producer": {
                    "setter_entry_rva": 0x022B59E0, "setter_caller_rva": None,
                    "formatter_entry_rva": None,
                    "identity_source": "authored_widget_registration",
                },
            }],
        }
        snap["_r2_character_presentation_live"] = {
            "schema_version": 3, "authority": "exact_a729_character_presentation",
            "executable_sha256": self.EXECUTABLE, **character_hashes,
            "elapsed_wall_ms": 1, "frame_hour": snap["hour_index"],
            "completeness_record_identity": "a39:complete", "source_sequence": 1,
            "trait_trees": [{
                "owner_tag_db_id": 17, "unit_leader_id": 33,
                "tree_root_offset": 0x120, "nodes": [{
                    "source_ordinal": 0, "trait_token": "trait_test",
                    "state_raw": 0, "state": "blocked", "progress_raw": None,
                    "reason_context_id": "a39:trait:reason",
                }],
            }, {
                "owner_tag_db_id": 17, "unit_leader_id": 33,
                "tree_root_offset": 0x130, "nodes": [],
            }],
            "ace_context": {"kind": "plane_type_mask", "owner_tag_db_id": None,
                            "wing_id": None, "plane_type_mask_raw": 0x400},
            "ace_portraits": [],
        }
        self.assertEqual(schema.validate_snapshot(snap), [])
        snap["_r2_national_focus_presentation_live"]["expected_visible_item_count"] = 1
        self.assertTrue(any("completeness" in p for p in schema.validate_snapshot(snap)))
        snap["_r2_national_focus_presentation_live"]["expected_visible_item_count"] = 0
        snap["_r2_national_focus_presentation_live"]["materialization_qpc_ticks"] = 101
        self.assertTrue(any("receipt metadata" in p for p in schema.validate_snapshot(snap)))
        snap["_r2_national_focus_presentation_live"]["materialization_qpc_ticks"] = 90
        snap["runtime_text"]["contexts"][0]["pov_country_or_global"] = "ENG"
        self.assertTrue(any("runtime-text join" in p for p in schema.validate_snapshot(snap)))

    def test_d56_and_x2_fail_closed_mutations(self):
        snap = self._base()
        snap["order_of_battle"]["extra"] = {"naval_missions_status_live": {
            "source": "C", "confidence": "validated_slice",
            "projection": "R2 order_of_battle.extra annotation",
            "task_force_existence_owner_domain": "oob_command_live",
            "ship_existence_owner_domain": "ships", "task_force_count": 0,
            "validated_location_rows": 0, "derived_region_rows": 0,
            "ship_summary_rows": 0, "dominant_hull_rows": 0,
            "fixture_required_rows": 0, "convoy_sunk_count_rows": 0,
            "oob_command_join_checked": True, "oob_command_orphan_rows": 0,
            "task_forces": [],
        }}
        snap["combats"].append({"land_combat_detail_events_live": {"attacker": {
            "committed": 1, "counts_incomplete": False,
            "losses_status": "unavailable_no_authenticated_live_carrier",
            "width_status": "exact_synchronous_call",
            "width_observations": [{"capture_sequence": 1, "final_width_raw": 5,
                                    "wrappers": [{"ordinal": 0, "division_id": 9, "width_raw": 5}],
                                    "source": "authenticated_a729_141299610_call"}],
        }}})
        self.assertEqual(schema.validate_snapshot(snap), [])
        snap["combats"][0]["land_combat_detail_events_live"]["attacker"]["width_observations"][0]["final_width_raw"] = 4
        self.assertTrue(any("width decomposition" in p for p in schema.validate_snapshot(snap)))

    def test_native_weather_counts_and_null_semantics(self):
        snap = self._base()
        regions = {}
        for region_id in range(305):
            regions[str(region_id)] = {
                "region_id": region_id, "temperature": 0.0, "temperature_raw": 0,
                "phenomenon": 0, "weather_kind": "no_phenomenon",
                "next_weather_change_raw": 0,
                "active_modifier_count": 0, "intensity": 0,
                "rain_light": False, "rain_heavy": False, "snow": False,
                "blizzard": False, "arctic_water": False, "active_signal": False,
                "phenomenon_active": False, "sandstorm": False,
            }
        snap["map"]["_r2_weather_live"] = {
            "source": "weather_live", "active_region_count": 0,
            "observed_region_rows": 305, "observed_province_rows": 0,
            "scale_raw": 100_000, "regions": regions, "provinces": {},
        }
        self.assertEqual(schema.validate_snapshot(snap), [])
        snap["map"]["_r2_weather_live"]["regions"]["42"]["weather_kind"] = "rain_heavy"
        self.assertTrue(any("exact region row" in p for p in schema.validate_snapshot(snap)))
        snap["map"]["_r2_weather_live"]["regions"]["42"]["weather_kind"] = "no_phenomenon"
        snap["map"]["_r2_weather_live"]["regions"]["42"]["rain_light"] = True
        snap["map"]["_r2_weather_live"]["regions"]["42"]["rain_heavy"] = True
        self.assertTrue(any("exact region row" in p for p in schema.validate_snapshot(snap)))
        snap["map"]["_r2_weather_live"]["regions"]["42"]["rain_light"] = False
        snap["map"]["_r2_weather_live"]["regions"]["42"]["rain_heavy"] = False
        snap["map"]["_r2_weather_live"]["observed_region_rows"] = 304
        self.assertTrue(any("completeness counts" in p for p in schema.validate_snapshot(snap)))


if __name__ == "__main__":
    unittest.main()
