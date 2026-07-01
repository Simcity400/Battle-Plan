"""Offline fold tests for the WAVE live-extraction domains added in the
backlog-closing pass:

  * country_static   -> countries.*.identity.{original_capital, original_tag,
                        reserved_dynamic_country, templates_locked}
  * country_variables -> countries.*.script_variables
  * state_fields     -> states.*.{resources, dynamic_modifier, extra_shared_slots}

NO game required: stub DomainResults are folded by the assembler and the
resulting snapshot is asserted. Proves the measured-empty / measured-None
discipline (a real empty container survives; an unmeasured field stays null;
the snapshot still validates against the schema).

Run:  python -m pytest live_capture/tests/test_new_live_domains.py -q
"""
from __future__ import annotations

import os
import sys
import unittest
import json

_HERE = os.path.dirname(os.path.abspath(__file__))
_REPO = os.path.normpath(os.path.join(_HERE, os.pardir, os.pardir))
if _REPO not in sys.path:
    sys.path.insert(0, _REPO)
_MR = os.path.join(_REPO, "memory_reader")
if _MR not in sys.path:
    sys.path.insert(0, _MR)

from replay import schema  # noqa: E402
from live_capture import registry as reg  # noqa: E402
from live_capture import assembler as asm  # noqa: E402


class FakeIdentity:
    instance_id = "inst-A"
    pid = 1234
    exe_sha256 = "deadbeef"

    def build_ok(self):
        return True


def _ok(name, data):
    return reg.DomainResult(name, available=True, captured=True, data=data)


def _assemble(results):
    return asm.assemble_snapshot(results, game_id="g", date="1936.2.7.6",
                                 exe_sha256="abc", identity=FakeIdentity())


class TestCountryStaticFold(unittest.TestCase):
    def _snap(self):
        res = {
            "country_static": _ok("country_static", {
                "countries": {
                    "USA": {"original_capital": 361, "original_tag": None,
                            "reserved_dynamic_country": False,
                            "templates_locked": False},
                    "D01": {"original_capital": 41, "original_tag": "SPR",
                            "reserved_dynamic_country": True,
                            "templates_locked": True},
                },
                "count": 2,
            })
        }
        return _assemble(res)

    def test_static_identity_folds(self):
        snap = self._snap()
        ident = snap["countries"]["USA"]["identity"]
        self.assertEqual(ident["original_capital"], 361)
        self.assertIsNone(ident["original_tag"])               # measured absent -> None
        self.assertFalse(ident["reserved_dynamic_country"])
        self.assertFalse(ident["templates_locked"])
        d01 = snap["countries"]["D01"]["identity"]
        self.assertEqual(d01["original_tag"], "SPR")
        self.assertTrue(d01["reserved_dynamic_country"])
        self.assertTrue(d01["templates_locked"])

    def test_static_provenance_and_schema(self):
        snap = self._snap()
        self.assertIn("country_static", snap["provenance"]["memory"]["domains"])
        self.assertEqual(schema.validate_snapshot(snap), [])

    def test_static_absent_country_has_null_identity(self):
        # a country the static plug did NOT measure keeps identity null (no fabrication)
        res = {
            "country_static": _ok("country_static", {"countries": {
                "USA": {"original_capital": 361, "original_tag": None,
                        "reserved_dynamic_country": False, "templates_locked": False}},
                "count": 1}),
            "country_scalars": _ok("country_scalars", {"countries": {
                "GER": {"stability": 0.6}}, "count": 1}),
        }
        snap = _assemble(res)
        self.assertIsNone(snap["countries"]["GER"]["identity"])


class TestCountryVariablesFold(unittest.TestCase):
    def test_variables_bag_folds(self):
        res = {"country_variables": _ok("country_variables", {
            "countries": {
                "USA": {"current_house_icon_frame": 11.00673,
                        "fighters_milestone_var": 1.0},
                "MEX": {},   # MEASURED-EMPTY: a real read of an empty container
            },
            "count": 2, "countries_with_variables": 1,
        })}
        snap = _assemble(res)
        self.assertEqual(snap["countries"]["USA"]["script_variables"]
                         ["current_house_icon_frame"], 11.00673)
        # a measured-empty {} is written through, NOT left null
        self.assertEqual(snap["countries"]["MEX"]["script_variables"], {})
        self.assertIn("country_variables", snap["provenance"]["memory"]["domains"])
        self.assertEqual(schema.validate_snapshot(snap), [])

    def test_unmeasured_country_variables_null(self):
        res = {
            "country_variables": _ok("country_variables", {"countries": {
                "USA": {"x": 1.0}}, "count": 1}),
            "country_scalars": _ok("country_scalars", {"countries": {
                "GER": {"stability": 0.6}}, "count": 1}),
        }
        snap = _assemble(res)
        self.assertIsNone(snap["countries"]["GER"]["script_variables"])

    def test_variables_tristate_distinguishes_empty_from_unmeasured(self):
        # Codex wave-1 fix: the tri-state reader returns (None, False) for an
        # UNVERIFIABLE container (so the leaf stays null) and ({}, True) for a real
        # size==0 container (a measured-empty written through).
        import variables_live as v

        class _FakeP:
            def __init__(self, layout):
                self.layout = layout

            def u64(self, a):
                if a not in self.layout:
                    raise OSError("bad")
                return self.layout[a]

            def i64(self, a):
                return self.layout.get(a, 0)

        # sub pointer not in the heap window -> UNMEASURED (None, False)
        fp = _FakeP({0x1000 + v.OBJ_VARIABLES: 0x5})
        self.assertEqual(v.read_variables_tristate(fp, 0x1000), (None, False))
        # OSError reading the sub pointer -> UNMEASURED
        self.assertEqual(v.read_variables_tristate(_FakeP({}), 0x1000), (None, False))


class TestStateFieldsFold(unittest.TestCase):
    def test_state_fields_fold(self):
        res = {"state_fields": _ok("state_fields", {
            "states": {
                "9": {"resources": {"steel": 5.0, "coal": 6.0},
                      "dynamic_modifier": ["CZE_czech_state_modifier"],
                      "extra_shared_slots": 2},
                "361": {"resources": {}, "dynamic_modifier": [],
                        "extra_shared_slots": 0},  # all measured-empty / zero
            },
            "count": 2,
        })}
        snap = _assemble(res)
        s9 = snap["states"]["9"]
        self.assertEqual(s9["resources"], {"steel": 5.0, "coal": 6.0})
        self.assertEqual(s9["dynamic_modifier"], ["CZE_czech_state_modifier"])
        self.assertEqual(s9["extra_shared_slots"], 2)
        s361 = snap["states"]["361"]
        self.assertEqual(s361["resources"], {})         # measured-empty -> {}
        self.assertEqual(s361["dynamic_modifier"], [])   # measured-empty -> []
        self.assertEqual(s361["extra_shared_slots"], 0)  # measured 0 -> 0, NOT null
        self.assertIn("state_fields", snap["provenance"]["memory"]["domains"])
        self.assertEqual(schema.validate_snapshot(snap), [])

    def test_state_fields_measured_set_recorded(self):
        # the SAVE-SHARED leaf (resources) must record its measured state set so the
        # save backstop only re-asserts on measured states.
        res = {"state_fields": _ok("state_fields", {
            "states": {"9": {"resources": {"steel": 5.0},
                             "dynamic_modifier": [], "extra_shared_slots": 0}},
            "count": 1})}
        snap = _assemble(res)
        measured = snap.get("_live_measured_leaf_tags", {})
        self.assertIn("states.resources", measured)
        self.assertIn("9", measured["states.resources"])


class TestCountryContainersFold(unittest.TestCase):
    def test_containers_fold(self):
        res = {"country_containers": _ok("country_containers", {
            "countries": {
                "USA": {"cores": [361, 357], "claims": [],
                        "naval_headquarter_status": None,
                        "decision_status_present": {"active_timed_decision": 1},
                        "country_reports": {"days": 9, "index": 2, "date_raw": 60760032}},
                "GER": {"cores": [64], "claims": [],
                        "naval_headquarter_status": 1,
                        "decision_status_present": None,
                        "country_reports": {"days": 9, "index": 2, "date_raw": 60760032}},
            },
            "count": 2,
        })}
        snap = _assemble(res)
        usa = snap["countries"]["USA"]
        self.assertEqual(usa["identity"]["cores"], [361, 357])
        self.assertEqual(usa["identity"]["claims"], [])             # measured-empty list
        self.assertEqual(usa["decisions"]["status_present"],
                         {"active_timed_decision": 1})
        self.assertEqual(usa["country_reports"]["days"], 9)
        ger = snap["countries"]["GER"]
        self.assertEqual(ger["identity"]["naval_headquarter_status"], 1)
        # GER measured no decision status -> status_present not written (stays absent)
        self.assertNotIn("status_present", ger.get("decisions", {}))
        self.assertIn("country_containers", snap["provenance"]["memory"]["domains"])
        self.assertEqual(schema.validate_snapshot(snap), [])


class TestWeatherFieldsWiring(unittest.TestCase):
    def test_weather_fields_fn_folds_regions_and_provinces(self):
        # exercise _wire_weather_fields directly with a fake reader (no game needed).
        import memory_reader.weather.weather_fields_live as wf

        orig_r = wf.read_region_fields
        orig_p = wf.read_province_fields
        orig_l = wf.locate_weather
        wf.locate_weather = lambda pid=None: (None, None)   # offline: no real locate
        wf.read_region_fields = lambda pid=None, _located=None: {"manager": 1, "size": 2, "regions": {
            5: {"temperature": 22.0, "phenomenon": ["rain_light"],
                "next_weather_change_hours": 16960256, "active_modifier_count": 0},
            6: {"temperature": -1.5, "phenomenon": [],
                "next_weather_change_hours": None, "active_modifier_count": 1}}}
        wf.read_province_fields = lambda pid=None, _located=None: {"manager": 1, "size": 2, "provinces": {
            11: {"mud": True, "custom_modifier_count": 0},
            12: {"mud": False, "custom_modifier_count": 1}}}
        try:
            snap = schema.empty_snapshot("g")
            ok = asm._wire_weather_fields(snap, pid=999)
            self.assertTrue(ok)
            regs = snap["weather"]["regions"]
            self.assertEqual(regs["5"]["temperature"], 22.0)
            self.assertEqual(regs["5"]["phenomenon"], ["rain_light"])
            self.assertEqual(regs["6"]["phenomenon"], [])      # measured-empty list
            self.assertIsNone(regs["6"]["next_weather_change_hours"])
            prov = snap["weather"]["provinces"]
            self.assertTrue(prov["11"]["mud"])
            self.assertEqual(prov["12"]["custom_modifier_count"], 1)
        finally:
            wf.read_region_fields = orig_r
            wf.read_province_fields = orig_p
            wf.locate_weather = orig_l

    def test_weather_fields_offline_returns_false(self):
        snap = schema.empty_snapshot("g")
        self.assertFalse(asm._wire_weather_fields(snap, pid=None))


class TestRegistryWiring(unittest.TestCase):
    def test_new_plugs_registered(self):
        names = {p.name for p in reg.default_registry()}
        self.assertIn("country_static", names)
        self.assertIn("country_variables", names)
        self.assertIn("state_fields", names)
        self.assertIn("country_containers", names)


# ---------------------------------------------------------------------------
# Deliverable A: per-province SNOW (+0x40) reader/validator + the FROZEN proof.
# These are offline tests (NO game): the reader's fixed-point decode is exercised
# on a hand-built per-province struct; the snow validator's raw-integer + negative-
# control logic is exercised on a tiny synthetic GT/buffer; the FROZEN proof parses
# the USA-1936 melt to assert no per-province `frozen` token and that arctic_water IS
# a per-region phenomenon the region reader emits.
# ---------------------------------------------------------------------------
class TestSnowReaderDecode(unittest.TestCase):
    """The +0x40 i32/1e5 decode: a fake per-province struct with a known raw snow
    word must decode to raw/1e5 (and water at +0x38 likewise)."""

    def _fake_struct(self, *, pid, water_raw, snow_raw):
        import struct as _s
        import memory_reader.weather.weather_temp_model as wt
        body = bytearray(wt.PERPROV_STRIDE)
        _s.pack_into("<H", body, wt.PROV_ID_OFF, pid)
        _s.pack_into("<i", body, wt.WATER_OFF, water_raw)
        _s.pack_into("<i", body, wt.SNOW_OFF, snow_raw)
        return bytes(body)

    def test_snow_decode_raw_over_1e5(self):
        import struct as _s
        import memory_reader.weather.weather_temp_model as wt
        body = self._fake_struct(pid=2, water_raw=12345, snow_raw=7976023)
        # mirror the reader's decode (raw i32 at +0x40, /1e5)
        sraw = _s.unpack_from("<i", body, wt.SNOW_OFF)[0]
        self.assertEqual(sraw, 7976023)
        self.assertAlmostEqual(sraw / wt.FIXED_POINT_SCALE, 79.76023, places=5)
        wraw = _s.unpack_from("<i", body, wt.WATER_OFF)[0]
        self.assertAlmostEqual(wraw / wt.FIXED_POINT_SCALE, 0.12345, places=5)

    def test_zero_snow_collapses_to_none(self):
        # the reader collapses a <=0 slot to None (measured-0 indistinguishable from
        # unmeasured); a positive raw is kept as measured.
        import struct as _s
        import memory_reader.weather.weather_temp_model as wt
        body = self._fake_struct(pid=5, water_raw=0, snow_raw=0)
        sraw = _s.unpack_from("<i", body, wt.SNOW_OFF)[0]
        snow = None if sraw <= 0 else sraw / wt.FIXED_POINT_SCALE
        self.assertIsNone(snow)


class TestSnowValidatorLogic(unittest.TestCase):
    """The strict snow validator's raw-integer compare + negative controls, exercised
    on a SYNTHETIC tiny GT + a SYNTHETIC live per-province buffer (no game)."""

    def _build(self):
        import struct as _s
        import memory_reader.weather.weather_temp_model as wt
        import memory_reader.weather.validate_snow_usa1936 as v
        # synthetic GT: 4 provinces, snow decimals (pid 0 has zero snow).
        gt_decimal = {0: None, 1: 0.50000, 2: 79.76023, 3: 0.00003}
        gt_raw = {pid: v._save_raw(d) for pid, d in gt_decimal.items()}
        # synthetic live buffer: index==pid, +0x40 raw == round(gt*1e5), distinct
        # neighbour words so the negative controls genuinely differ.
        bodies = {}
        for pid, raw in gt_raw.items():
            body = bytearray(wt.PERPROV_STRIDE)
            _s.pack_into("<H", body, wt.PROV_ID_OFF, pid)
            _s.pack_into("<i", body, wt.SNOW_OFF, raw)            # the real snow
            _s.pack_into("<i", body, wt.SNOW_OFF + 4, raw + 9999) # +0x44 neighbour
            _s.pack_into("<i", body, wt.SNOW_OFF - 4, raw + 7777) # +0x3C neighbour
            _s.pack_into("<i", body, wt.WATER_OFF, raw + 5555)    # water +0x38
            _s.pack_into("<i", body, wt.TEMP_OFF, 2000000)        # temp +0x118
            bodies[pid] = bytes(body)
        buf = {"manager": 0x1, "begin": 0x1000, "size": len(bodies), "bodies": bodies}
        return v, gt_raw, buf

    def test_primary_exact_match_passes(self):
        v, gt_raw, buf = self._build()
        res = v.validate(gt_raw, buf)
        p = res["primary"]
        self.assertEqual(p["FP"], 0)
        self.assertEqual(p["FN"], 0)
        self.assertEqual(p["value_mismatch"], 0)
        self.assertEqual(p["matched_nonzero"], p["n_save_nonzero"])
        self.assertEqual(p["n_save_nonzero"], 3)        # pids 1,2,3 nonzero
        self.assertEqual(p["max_abs_raw_delta"], 0)
        self.assertEqual(res["index_is_province_id"]["bad"], 0)

    def test_negative_controls_fail_to_match(self):
        v, gt_raw, buf = self._build()
        res = v.validate(gt_raw, buf)
        neg = res["negative_controls"]
        # the wrong-offset controls carry a distinct word -> 0 matches.
        for label in ("offset+0x44", "offset+0x3C", "water+0x38", "temp+0x118"):
            self.assertEqual(neg[label]["match"], 0, label)
        # overall verdict PASS on the synthetic exact-match case.
        ok, primary_pass, neg_pass = v.verdict(res)
        self.assertTrue(primary_pass)
        self.assertTrue(neg_pass)
        self.assertTrue(ok)

    def test_wrong_oracle_control_fails_to_match(self):
        # ANTI-CONTAMINATION (Codex Q1): the SAME live buffer compared against a
        # DIFFERENT save's snow GT must NOT match (proves the live read is not aliasing
        # the oracle). Here the wrong GT carries different raw values at the same pids.
        v, gt_raw, buf = self._build()
        wrong_gt = {1: v._save_raw(11.111), 2: v._save_raw(22.222),
                    3: v._save_raw(33.333)}
        wo = v.wrong_oracle_check(wrong_gt, buf)
        self.assertEqual(wo["match"], 0)       # none of the wrong values match
        self.assertEqual(wo["n"], 3)
        # and the CORRECT oracle (the same gt_raw the buffer was built from) matches all.
        right = v.wrong_oracle_check(gt_raw, buf)
        self.assertEqual(right["match"], right["n"])
        self.assertEqual(right["rate"], 1.0)

    def test_injected_fp_fn_mismatch_breaks_primary(self):
        # corrupt the live buffer: drop one nonzero (FN), zero->nonzero one (FP), and
        # perturb one value (mismatch) -> primary must FAIL.
        import struct as _s
        import memory_reader.weather.weather_temp_model as wt
        v, gt_raw, buf = self._build()
        bodies = {k: bytearray(b) for k, b in buf["bodies"].items()}
        _s.pack_into("<i", bodies[1], wt.SNOW_OFF, 0)        # FN: save nonzero, live 0
        _s.pack_into("<i", bodies[0], wt.SNOW_OFF, 123456)   # FP: save 0, live nonzero
        _s.pack_into("<i", bodies[2], wt.SNOW_OFF,
                     v._save_raw(79.76023) + 50)             # value mismatch
        buf2 = dict(buf, bodies={k: bytes(b) for k, b in bodies.items()})
        res = v.validate(gt_raw, buf2)
        p = res["primary"]
        self.assertEqual(p["FN"], 1)
        self.assertEqual(p["FP"], 1)
        self.assertEqual(p["value_mismatch"], 1)
        ok, primary_pass, neg_pass = v.verdict(res)
        self.assertFalse(primary_pass)
        self.assertFalse(ok)


class TestSnowWiring(unittest.TestCase):
    """_wire_weather carries the per-province snow into weather.provinces.*.snow and
    NEVER adds a per-province `frozen` field (fail-closed; arctic_water is per-region)."""

    def test_wire_weather_carries_snow_no_frozen(self):
        import memory_reader.weather.weather_temp_model as wt
        orig_t = wt.read_province_temperatures
        orig_p = wt.read_province_precipitation
        wt.read_province_temperatures = lambda pid=None: {
            "manager": 1, "array": 2, "size": 2, "n_with_temp": 2,
            "temp_min_c": -1.0, "temp_max_c": 5.0, "temp_mean_c": 2.0,
            "provinces": {
                2: {"temp_c": -1.0, "temp_change_c": 0.1, "temp2_c": None,
                    "province_id_read": 2},
                3: {"temp_c": 5.0, "temp_change_c": -0.2, "temp2_c": None,
                    "province_id_read": 3}}}
        wt.read_province_precipitation = lambda pid=None: {
            "manager": 1, "array": 2, "size": 2, "n_with_water": 1, "n_with_snow": 1,
            "water_max": 1.0, "water_mean": 0.5,
            "provinces": {
                2: {"water": 0.123, "snow": 79.76023,
                    "water_raw": 12300, "snow_raw": 7976023},
                3: {"water": None, "snow": None,
                    "water_raw": 0, "snow_raw": 0}}}
        try:
            snap = schema.empty_snapshot("g")
            ok = asm._wire_weather(snap, pid=999)
            self.assertTrue(ok)
            prov = snap["weather"]["provinces"]
            # snow folded onto province 2 exactly; province 3 has a measured-None snow.
            self.assertAlmostEqual(prov["2"]["snow"], 79.76023, places=5)
            self.assertIsNone(prov["3"]["snow"])
            self.assertEqual(prov["2"]["precip"], 0.123)
            # CRITICAL: no per-province `frozen` field is ever emitted (fail-closed).
            self.assertNotIn("frozen", prov["2"])
            self.assertNotIn("frozen", prov["3"])
            self.assertEqual(snap["weather"]["source"], "M")
            self.assertEqual(schema.validate_snapshot(snap), [])
        finally:
            wt.read_province_temperatures = orig_t
            wt.read_province_precipitation = orig_p

    def test_schema_has_no_per_province_frozen_field(self):
        # the schema must NOT define a weather.provinces.*.frozen leaf (no fabrication).
        froz = [k for k in schema.FIELD_SOURCES
                if k.startswith("weather.provinces.") and k.endswith(".frozen")]
        self.assertEqual(froz, [])
        # the snow leaf IS present and badged M.
        self.assertEqual(schema.source_of("weather.provinces.2.snow"), "M")


class TestFrozenAbsenceAndArcticWater(unittest.TestCase):
    """FROZEN proof: the per-province weather output has NO `frozen` token, and the
    measured frozen-water signal is the per-REGION arctic_water phenomenon flag."""

    @classmethod
    def setUpClass(cls):
        # Use tracked save-oracle output. The old raw-melt helper lived in ignored
        # re_scratch/ and made this test depend on local cleanup state.
        cls.weather_json = os.path.join(_REPO, "save_oracle", "output", "weather.json")
        cls.have_weather_json = os.path.isfile(cls.weather_json)

    def test_no_per_province_frozen_token_in_melt(self):
        if not self.have_weather_json:
            self.skipTest("save_oracle/output/weather.json not present")
        with open(self.weather_json, "r", encoding="utf-8") as fh:
            recs = json.load(fh)
        self.assertGreater(len(recs), 1000)
        toplevel = set().union(*(set(v.keys()) for v in recs.values()))
        self.assertNotIn("frozen", toplevel)
        self.assertTrue({"temp_c", "water", "snow"} <= toplevel)

    def test_frozen_token_absent_whole_melt(self):
        if not self.have_weather_json:
            self.skipTest("save_oracle/output/weather.json not present")
        with open(self.weather_json, "r", encoding="utf-8") as fh:
            text = fh.read()
        self.assertNotIn('"frozen"', text)

    def test_arctic_water_is_a_region_phenomenon_the_reader_emits(self):
        # arctic_water IS a per-REGION phenomenon flag (the measured frozen-water signal),
        # read at +0x130 by weather_fields_live and folded into regions.*.phenomenon.
        import memory_reader.weather.weather_fields_live as wf
        self.assertIn("arctic_water", wf.R_PHENOM)
        self.assertEqual(wf.R_PHENOM["arctic_water"], 0x130)
        # exercise the wiring with a fake region carrying arctic_water -> it reaches
        # snap.weather.regions.*.phenomenon (no game needed).
        orig_r = wf.read_region_fields
        orig_p = wf.read_province_fields
        orig_l = wf.locate_weather
        wf.locate_weather = lambda pid=None: (None, None)
        wf.read_region_fields = lambda pid=None, _located=None: {
            "manager": 1, "size": 1, "regions": {
                7: {"temperature": -20.0, "phenomenon": ["arctic_water"],
                    "next_weather_change_hours": None, "active_modifier_count": 0}}}
        wf.read_province_fields = lambda pid=None, _located=None: {
            "manager": 1, "size": 0, "provinces": {}}
        try:
            snap = schema.empty_snapshot("g")
            ok = asm._wire_weather_fields(snap, pid=999)
            self.assertTrue(ok)
            self.assertIn("arctic_water",
                          snap["weather"]["regions"]["7"]["phenomenon"])
        finally:
            wf.read_region_fields = orig_r
            wf.read_province_fields = orig_p
            wf.locate_weather = orig_l


if __name__ == "__main__":
    unittest.main()
