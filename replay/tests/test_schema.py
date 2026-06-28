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

    def test_collector_production_source(self):
        self.assertEqual(schema.source_of("countries.POR.production.stockpile"), "C/M")
        self.assertEqual(schema.source_of("countries.POR.production.stockpile_total"), "C/M/D")
        self.assertEqual(schema.source_of("countries.POR.production.military_lines"), "C/M")
        self.assertEqual(schema.source_of("countries.POR.production.naval_factories"), "C/M")

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


class TestValidator(unittest.TestCase):
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
        }
        probs = schema.validate_snapshot(snap)
        self.assertTrue(any("economy.fuel.current" in p and "out of range" in p
                            for p in probs), probs)
        self.assertTrue(any("economy.fuel.max" in p and "not number|null" in p
                            for p in probs), probs)

    def test_bad_country_production_values_flagged(self):
        snap = schema.empty_snapshot("g1", "1940.9.10.13")
        snap["countries"]["POR"] = schema.empty_country()
        snap["countries"]["POR"]["production"].update({
            "stockpile": {"70": float("inf"), "bad": 1.0},
            "stockpile_total": "bad",
            "military_lines": [{
                "id": True,
                "active_factories": -1,
                "cost": -1.0,
                "speed": 1.0,
                "produced": 2.0,
                "amount": -2,
            }],
            "naval_lines": ["bad"],
            "military_factories": -1,
            "naval_factories": True,
        })
        probs = schema.validate_snapshot(snap)
        self.assertTrue(any("production.stockpile['70']" in p for p in probs), probs)
        self.assertTrue(any("production.stockpile key 'bad'" in p for p in probs), probs)
        self.assertTrue(any("production.stockpile_total" in p for p in probs), probs)
        self.assertTrue(any("military_lines[0].id" in p for p in probs), probs)
        self.assertTrue(any("military_lines[0].active_factories" in p
                            for p in probs), probs)
        self.assertTrue(any("military_lines[0].cost" in p for p in probs), probs)
        self.assertTrue(any("military_lines[0].amount" in p for p in probs), probs)
        self.assertTrue(any("naval_lines[0] not a dict" in p for p in probs), probs)
        self.assertTrue(any("military_factories" in p and "nonnegative int" in p
                            for p in probs), probs)
        self.assertTrue(any("naval_factories" in p and "nonnegative int" in p
                            for p in probs), probs)

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
