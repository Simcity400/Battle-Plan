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
        self.assertEqual(schema.source_of("states.23.owner"), "M/S")
        self.assertEqual(schema.source_of("provinces.11596.controller"), "S/D")

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
                self.assertIn(p, {"M", "M*", "S", "D", "marker"}, v)


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

    def test_event_validator(self):
        good = schema.make_event("focus_completed", subject="GER", key="X", to="X",
                                 source="D", confidence="interval")
        self.assertEqual(schema.validate_event(good), [])
        bad = schema.make_event("not_a_type", confidence="weird", source="??")
        self.assertTrue(schema.validate_event(bad))


if __name__ == "__main__":
    unittest.main()
