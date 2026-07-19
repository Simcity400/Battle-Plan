"""Tests for replay/schema.py — date math, source map, empty skeletons, validator."""
import os
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

    def test_state_and_province(self):
        self.assertEqual(schema.source_of("states.23.owner"), "C/M/S")
        # live collector province_control measures the province controller; legacy
        # memory/save and the save-side state-controller fallback remain possible.
        self.assertEqual(schema.source_of("provinces.11596.controller"), "C/M/S/D")
        self.assertEqual(schema.source_of("map.control_by_province"), "C/M/S/D")

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
        self.assertEqual(schema.source_of("world_events.fired_events"), "C/M/S")
        self.assertEqual(schema.source_of("world_events.fired_events_complete"), "C/M")

    def test_active_ability_source(self):
        self.assertEqual(
            schema.source_of("countries.GER.active_abilities.rows.0.ability_id"),
            "C/M",
        )
        self.assertEqual(
            schema.source_of("countries.GER.active_abilities.rows.0.disabled_reason"),
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

    def test_unknown_returns_none(self):
        self.assertIsNone(schema.source_of("countries.GER.nonsense.field"))

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
        snap = schema.empty_snapshot("g1", "1940.9.10.13")
        snap["countries"]["GER"] = schema.empty_country()
        snap["countries"]["GER"]["active_abilities"] = {
            "complete": True,
            "rows": [{
                "ability_id": "force_attack",
                "available": False,
                "active": True,
                "activation_hour": 17,
                "cooldown_hours": 504,
                "cooldown_remaining_hours": 336,
                "cost_command_power": 22.0,
                "target_kind": "army_leader",
                "target_id": "73",
                "duration_hours": 168,
                "duration_remaining_hours": 120,
                "display_order": 0,
                "icon": "GFX_ability_force_attack",
                "tooltip": "Force Attack",
                "disabled_reason": "Already active",
            }],
        }
        return snap

    def test_xui23_active_ability_contract_validates(self):
        self.assertEqual(schema.validate_snapshot(self._active_ability_snapshot()), [])

    def test_xui23_active_ability_contract_rejects_inference_shapes(self):
        snap = self._active_ability_snapshot()
        row = snap["countries"]["GER"]["active_abilities"]["rows"][0]
        row["available"] = "yes"
        row["display_order"] = 2
        row["cooldown_remaining_hours"] = 505
        row["tooltip"] = None
        probs = schema.validate_snapshot(snap)
        for token in (
            ".available", "display_order", "cooldown remaining exceeds total", ".tooltip"
        ):
            self.assertTrue(any(token in problem for problem in probs), (token, probs))

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

    def test_event_validator(self):
        good = schema.make_event("focus_completed", subject="GER", key="X", to="X",
                                 source="D", confidence="interval")
        self.assertEqual(schema.validate_event(good), [])
        bad = schema.make_event("not_a_type", confidence="weird", source="??")
        self.assertTrue(schema.validate_event(bad))


if __name__ == "__main__":
    unittest.main()
