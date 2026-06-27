"""Dry-run tests for live_capture -- NO game required.

Proves, with STUB readers and a fake process identity:

  * snapshot ASSEMBLY from plug results conforms to replay/SCHEMA.md (validates),
  * NULL-SAFETY: an absent/failed plug omits its domain from provenance AND leaves
    its fields null (no fabrication; 0 != null),
  * CONTINUITY tagging: first capture, normal forward step, hour ROLLBACK -> new
    segment, PID/process change -> new segment, large forward jump -> degraded +
    new segment, UNKNOWN time -> never segments and never overwrites last hour,
    campaign_id stable across a (simulated) tool restart,
  * DERIVE overlay runs and the _capture tags survive it,
  * DURABLE store write: snapshot on disk + index updated + atomic temp cleanup,
    unknown-hour captures go to _noncanonical and stay out of the index,
  * the LOCK is exclusive.

Run:
    python -m live_capture.tests.test_live_capture
    (or: python -m unittest discover live_capture)
"""
from __future__ import annotations

import os
import shutil
import sys
import tempfile
import unittest

_HERE = os.path.dirname(os.path.abspath(__file__))
_REPO = os.path.normpath(os.path.join(_HERE, os.pardir, os.pardir))
if _REPO not in sys.path:
    sys.path.insert(0, _REPO)

from replay import schema  # noqa: E402
from live_capture import registry as reg  # noqa: E402
from live_capture import assembler as asm  # noqa: E402
from live_capture import continuity as cont  # noqa: E402
from live_capture.capture import CaptureLoop, DateInfo  # noqa: E402
from live_capture.durable_store import (  # noqa: E402
    DurableCaptureStore, CaptureLock, atomic_write_json, read_json,
)


# --------------------------------------------------------------------------
# Fakes
# --------------------------------------------------------------------------
class FakeIdentity:
    def __init__(self, instance_id="inst-A", pid=1234, exe_sha="deadbeef"):
        self.instance_id = instance_id
        self.pid = pid
        self.exe_sha256 = exe_sha

    def build_ok(self):
        return True


def _stub_results(*, include=("state_control", "province_control",
                              "divisions", "country_scalars"),
                  fail=(), unavailable=()):
    """Build a {name: DomainResult} map of canned plug outputs.

    `fail` names plugs that "raised" (captured=False, data discarded).
    `unavailable` names plugs whose module "didn't import".
    """
    out = {}

    def mk(name, data):
        if name in unavailable:
            return reg.DomainResult(name, available=False, captured=False,
                                    error="ImportError(stub)")
        if name in fail:
            return reg.DomainResult(name, available=True, captured=False,
                                    error="boom")
        return reg.DomainResult(name, available=True, captured=True, data=data)

    if "state_control" in include or "state_control" in fail or "state_control" in unavailable:
        out["state_control"] = mk("state_control", {
            "controllers": {"1": "GER", "2": "GER", "3": "SOV",
                            "4": None},  # 4 = un-nameable id kept as None
            "count": 4, "unresolved_ids": 1,
        })
    if "province_control" in include or "province_control" in fail or "province_control" in unavailable:
        out["province_control"] = mk("province_control", {
            "controllers": {"11": "GER", "12": "SOV"},
            "count": 2, "with_controller": 2,
        })
    if "divisions" in include or "divisions" in fail or "divisions" in unavailable:
        out["divisions"] = mk("divisions", {
            "divisions": [
                {"div_id": 100, "owner": "GER", "location_province": 11,
                 "organisation": 30.0, "strength": 0.9, "moving": False,
                 "in_combat": True, "supply_status": 5.0},
                {"div_id": 101, "owner": "SOV", "location_province": 12,
                 "organisation": 0.0, "strength": 0.0, "moving": True,
                 "move_target_province": 13, "in_combat": False},
                # un-nameable owner kept as a division with owner=None (Codex D)
                {"div_id": 102, "owner": None, "location_province": 12},
            ],
            "count": 3, "unresolved_owner": 1,
        })
    if "country_scalars" in include or "country_scalars" in fail or "country_scalars" in unavailable:
        out["country_scalars"] = mk("country_scalars", {
            "countries": {
                "GER": {"stability": 0.6, "war_support": 0.4,
                        "political_power": 120.0, "num_ships": 50,
                        "command_power": 30.0},
                "SOV": {"stability": 0.0},  # measured ZERO, must survive as 0.0
            },
            "count": 2,
        })
    return out


def _make_loop(tmp, *, game_id="dry_run", **kw):
    return CaptureLoop(game_id=game_id, store_root=tmp,
                       open_proc=lambda: None, **kw)


# --------------------------------------------------------------------------
# Tests
# --------------------------------------------------------------------------
class TestAssembly(unittest.TestCase):
    def test_assembles_and_validates(self):
        res = _stub_results()
        snap = asm.assemble_snapshot(res, game_id="g", date="1940.9.10.13",
                                     exe_sha256="abc", identity=FakeIdentity())
        self.assertEqual(schema.validate_snapshot(snap), [])
        self.assertEqual(snap["states"]["1"]["controller"], "GER")
        # un-nameable controller kept as a state with controller None (not dropped)
        self.assertIn("4", snap["states"])
        self.assertIsNone(snap["states"]["4"]["controller"])
        self.assertEqual(snap["provinces"]["11"]["controller"], "GER")
        # 3 divisions including the owner=None one (kept, Codex D)
        self.assertEqual(len(snap["divisions"]), 3)
        ger_div = [d for d in snap["divisions"] if d["owner"] == "GER"]
        self.assertEqual(len(ger_div), 1)

    def test_measured_zero_is_not_null(self):
        res = _stub_results()
        snap = asm.assemble_snapshot(res, game_id="g", date="1940.9.10.13",
                                     identity=FakeIdentity())
        # SOV stability measured 0.0 -> 0.0, NOT null
        self.assertEqual(snap["countries"]["SOV"]["politics"]["stability"], 0.0)
        # SOV did not report war_support -> stays null
        self.assertIsNone(snap["countries"]["SOV"]["politics"]["war_support"])

    def test_provenance_lists_only_captured_domains(self):
        res = _stub_results(include=("state_control", "divisions"),
                            fail=("country_scalars",),
                            unavailable=("province_control",))
        snap = asm.assemble_snapshot(res, game_id="g", date="1940.9.10.13",
                                     identity=FakeIdentity())
        doms = snap["provenance"]["memory"]["domains"]
        self.assertIn("state_control", doms)
        self.assertIn("divisions", doms)
        # a FAILED plug is not captured -> omitted
        self.assertNotIn("country_scalars", doms)
        # an UNAVAILABLE plug is omitted
        self.assertNotIn("province_control", doms)
        # and its fields stay null: no provinces captured
        self.assertEqual(snap["provinces"], {})
        # GER exists only as a shell created by the (captured) divisions plug to
        # hold the measured division_count; the FAILED country_scalars plug filled
        # nothing -> every politics scalar stays null (no fabrication of 0).
        self.assertEqual(snap["countries"]["GER"]["military"]["division_count"], 1)
        self.assertIsNone(snap["countries"]["GER"]["politics"]["stability"])
        self.assertIsNone(snap["countries"]["GER"]["military"]["num_ships"])

    def test_known_country_zero_divisions_is_measured_zero(self):
        # Codex code-review #4: when divisions captured, a known country with no
        # division is a MEASURED zero (0), not unmeasured (null). SOV appears via
        # country_scalars but has a division here; add a 3rd country with scalars
        # and no division to prove the zero-fill.
        res = _stub_results()
        res["country_scalars"].data["countries"]["FRA"] = {"stability": 0.5}
        snap = asm.assemble_snapshot(res, game_id="g", date="1940.9.10.13",
                                     identity=FakeIdentity())
        # FRA has scalars but no division -> division_count measured 0, not null
        self.assertEqual(snap["countries"]["FRA"]["military"]["division_count"], 0)
        # GER has 1 division
        self.assertEqual(snap["countries"]["GER"]["military"]["division_count"], 1)

    def test_no_division_count_when_divisions_not_captured(self):
        # if divisions did NOT run, division_count stays null (not 0): unmeasured.
        res = _stub_results(include=("country_scalars",),
                            unavailable=("divisions", "state_control",
                                         "province_control"))
        snap = asm.assemble_snapshot(res, game_id="g", date="1940.9.10.13",
                                     identity=FakeIdentity())
        self.assertIsNone(snap["countries"]["GER"]["military"]["division_count"])

    def test_numeric_owner_controller_sanitized_to_null(self):
        # Codex code-review #6: a misbehaving plug emitting a numeric id must not
        # land it in a *_tag field; it becomes null.
        res = _stub_results()
        res["state_control"].data["controllers"]["9"] = 42  # numeric, not a TAG
        res["divisions"].data["divisions"].append(
            {"div_id": 200, "owner": 7, "location_province": 5})
        snap = asm.assemble_snapshot(res, game_id="g", date="1940.9.10.13",
                                     identity=FakeIdentity())
        self.assertIsNone(snap["states"]["9"]["controller"])
        d200 = [d for d in snap["divisions"] if d["div_id"] == 200][0]
        self.assertIsNone(d200["owner"])

    def test_no_domains_means_empty_but_valid(self):
        snap = asm.assemble_snapshot({}, game_id="g", date=None,
                                     identity=FakeIdentity())
        self.assertEqual(schema.validate_snapshot(snap), [])
        self.assertIsNone(snap["hour_index"])
        self.assertEqual(snap["provenance"]["memory"]["domains"], [])
        self.assertFalse(snap["provenance"]["memory"]["captured"])


class TestNewDomains(unittest.TestCase):
    """MOCK-based coverage for the newly wired domains: land_combats (typed
    combats[] + sidecar), and the opt-in SAVE backstops (buildings / intel).
    NO live memory -- every reader output is a canned DomainResult."""

    def _res(self, name, data, *, available=True, captured=True, error=None):
        return reg.DomainResult(name, available=available, captured=captured,
                                data=data, error=error)

    # -- land combat -> typed combats[] + sidecar -----------------------
    def test_land_combat_promoted_to_typed_combats(self):
        res = {"land_combats": self._res("land_combats", {
            "land": [
                {"combat_id": 7, "location": 1234, "day": 3, "duration": 3,
                 "attacker_units": [100, 101], "defender_units": [200]},
                # a fail-closed side is None: typed view KEEPS None (not measured),
                # distinct from a measured empty []; sidecar keeps None too.
                {"combat_id": 8, "location": 50, "day": 1, "duration": 1,
                 "attacker_units": [300], "defender_units": None},
            ],
            "count": 2,
        })}
        snap = asm.assemble_snapshot(res, game_id="g", date="1940.9.10.13",
                                     identity=FakeIdentity())
        self.assertEqual(schema.validate_snapshot(snap), [])
        self.assertIn("land_combats", snap["provenance"]["memory"]["domains"])
        # canonical typed combats[] populated, kind=land, province + div-id ints
        self.assertEqual(len(snap["combats"]), 2)
        c7 = [c for c in snap["combats"] if c["combat_id"] == 7][0]
        self.assertEqual(c7["kind"], "land")
        self.assertEqual(c7["location_province"], 1234)
        self.assertEqual(sorted(c7["attackers"]), [100, 101])
        self.assertEqual(c7["defenders"], [200])
        # None defender side -> stays None in the typed view (NOT coerced to [];
        # adversarial review #2: None = not measured, [] = measured empty)
        c8 = [c for c in snap["combats"] if c["combat_id"] == 8][0]
        self.assertIsNone(c8["defenders"])
        self.assertEqual(c8["attackers"], [300])
        # raw sidecar present and preserves the None distinction
        side = snap["_live_land_combats"]["land"]
        c8raw = [r for r in side if r["combat_id"] == 8][0]
        self.assertIsNone(c8raw["defender_units"])

    def test_land_combat_measured_empty_side_stays_empty(self):
        # a MEASURED empty side (reader returned []) must stay [] (0 != null).
        res = {"land_combats": reg.DomainResult("land_combats", True, True, {
            "land": [{"combat_id": 9, "location": 1, "day": 1, "duration": 1,
                      "attacker_units": [], "defender_units": [5]}]})}
        snap = asm.assemble_snapshot(res, game_id="g", date="1940.9.10.13",
                                     identity=FakeIdentity())
        c9 = snap["combats"][0]
        self.assertEqual(c9["attackers"], [])      # measured empty, not None
        self.assertEqual(c9["defenders"], [5])

    def test_land_combat_absent_means_empty_combats_and_no_provenance(self):
        # domain not captured -> combats[] stays empty, sidecar absent, omitted
        snap = asm.assemble_snapshot({}, game_id="g", date="1940.9.10.13",
                                     identity=FakeIdentity())
        self.assertEqual(snap["combats"], [])
        self.assertNotIn("_live_land_combats", snap)
        self.assertNotIn("land_combats", snap["provenance"]["memory"]["domains"])

    def test_failed_land_combat_omitted(self):
        res = {"land_combats": self._res("land_combats", None, captured=False,
                                         error="boom")}
        snap = asm.assemble_snapshot(res, game_id="g", date="1940.9.10.13",
                                     identity=FakeIdentity())
        self.assertEqual(snap["combats"], [])
        self.assertNotIn("land_combats", snap["provenance"]["memory"]["domains"])

    def test_naval_air_combats_empty_detector_not_claimed_captured(self):
        # adversarial review #1: an EMPTY naval/air detector run is an UNPROVEN
        # absence, not a measured "no combats" -> not listed in memory.domains,
        # but the (empty) detector output is still kept in the sidecar + flagged.
        res = {"combats": self._res("combats", {"naval": [], "air": []})}
        snap = asm.assemble_snapshot(res, game_id="g", date="1940.9.10.13",
                                     identity=FakeIdentity())
        self.assertNotIn("combats", snap["provenance"]["memory"]["domains"])
        self.assertIn("_live_combats", snap)
        self.assertTrue(snap["_live_combats"]["_detector_only_unproven_absence"])

    def test_naval_air_combats_nonempty_uses_distinct_domain_id(self):
        # adversarial review #1 (refined): non-empty naval/air data is sidecar-ONLY
        # (never folded into canonical combats[]), so it must NOT claim the
        # canonical "combats" domain -- it uses a distinct "naval_air_combats" id.
        res = {"combats": self._res("combats", {
            "naval": [{"combat_id": 1}], "air": []})}
        snap = asm.assemble_snapshot(res, game_id="g", date="1940.9.10.13",
                                     identity=FakeIdentity())
        doms = snap["provenance"]["memory"]["domains"]
        self.assertIn("naval_air_combats", doms)
        self.assertNotIn("combats", doms)  # canonical domain NOT overclaimed
        self.assertEqual(snap["combats"], [])  # canonical list stays empty

    # -- save backstops -> _save_* sidecars, present only when captured --
    def test_save_buildings_sidecar_and_separate_provenance(self):
        res = {"buildings": self._res("buildings", {
            "source": "save", "from": "buildings_save",
            "rail_by_province": {"2068": 5}, "melt_path": "X.hoi4"})}
        snap = asm.assemble_snapshot(res, game_id="g", date="1940.9.10.13",
                                     identity=FakeIdentity())
        self.assertEqual(schema.validate_snapshot(snap), [])
        # adversarial review #5: a SAVE backstop is NOT a memory domain.
        self.assertNotIn("buildings", snap["provenance"]["memory"]["domains"])
        # it is surfaced in the SAVE provenance channel instead.
        sav = snap["provenance"]["save"]
        self.assertIsNotNone(sav)
        self.assertIn("buildings", sav["backstops"])
        self.assertFalse(sav["is_live"])
        # adversarial review #2: a save backstop fills NO canonical domain, so
        # provenance.save must NOT advertise a canonical "domains" list (that would
        # let the replay diff engine fabricate domain-presence events).
        self.assertNotIn("domains", sav)
        self.assertFalse(sav["fills_canonical_domains"])
        self.assertEqual(snap["_save_buildings"]["rail_by_province"]["2068"], 5)
        # naming: _save_*, NOT _live_* (must not imply freshness)
        self.assertNotIn("_live_buildings", snap)

    def test_assembler_redacts_path_even_if_reader_bypassed(self):
        # adversarial review FIX3: defense in depth -- if a caller fed a RAW path
        # past the readers, the assembler boundary must basename-redact it in BOTH
        # the persisted sidecar AND provenance. The WHOLE snapshot is written to
        # disk, so the check is over repr(snap), not just provenance.
        res = {"buildings": self._res("buildings", {
            "source": "save", "from": "buildings_save",
            "rail_by_province": {"1": 1},
            # both the redacted-key form AND a raw *_path alias, bypassing the reader
            "melt_file": "C:" + "\\Users\\someone\\saves\\autosave.hoi4",
            "state_detail_path": "/home/user/secret/state_detail.json"})}
        snap = asm.assemble_snapshot(res, game_id="g", date="1940.9.10.13",
                                     identity=FakeIdentity())
        # sidecar payload itself is scrubbed to basenames; raw *_path alias dropped
        sb = snap["_save_buildings"]
        self.assertEqual(sb["melt_file"], "autosave.hoi4")
        self.assertEqual(sb["state_detail_file"], "state_detail.json")
        self.assertNotIn("state_detail_path", sb)
        # provenance mirrors the basenames
        srcs = snap["provenance"]["save"]["source_filenames"]["buildings"]
        self.assertEqual(srcs["melt_file"], "autosave.hoi4")
        # the HARD guarantee: NO user path anywhere in the persisted snapshot
        blob = repr(snap)
        self.assertNotIn("Users", blob)
        self.assertNotIn("someone", blob)
        self.assertNotIn("/home/user", blob)
        self.assertNotIn("\\", blob)

    def test_pure_live_capture_has_null_save_provenance(self):
        # no save backstop ran -> provenance.save stays None (no fabrication).
        res = {"land_combats": self._res("land_combats", {"land": []})}
        snap = asm.assemble_snapshot(res, game_id="g", date="1940.9.10.13",
                                     identity=FakeIdentity())
        self.assertIsNone(snap["provenance"]["save"])

    def test_buildings_full_field_set_via_reader_fn(self):
        # adversarial review #6: the buildings reader emits the full per-province
        # set (rail/supply_node/naval_base) + full per-state buildings.
        class _FakeLayer:
            prov = {"rail_way": {2068: 5}, "supply_node": {2068: 2},
                    "naval_base": {3000: 4}}
            states = {2: {"infrastructure": 3, "arms_factory": 1}}
            def rail_map(self):
                return dict(self.prov["rail_way"])
            def infrastructure(self, sid):
                return self.states.get(sid, {}).get("infrastructure", 0)
        class _FakeBS:
            class BuildingsSave:
                @staticmethod
                def from_melt(p):
                    return _FakeLayer()
            class StateBuildings:
                @staticmethod
                def from_state_detail(p):
                    return _FakeLayer()
        # feed FULL user-directory paths; the output must store BASENAMES ONLY
        # (HARD redaction rule -- no user path may leak into tracked snapshot data).
        ctx = reg.CaptureContext(extras={
            "melt_path": "C:" + "\\Users\\someone\\saves\\autosave.hoi4",
            "state_detail_path": "/home/user/out/state_detail.json"})
        plugs = {p.name: p for p in reg.default_registry()}
        out = plugs["buildings"]._reader(ctx, _FakeBS)
        self.assertEqual(out["supply_node_by_province"]["2068"], 2)
        self.assertEqual(out["naval_base_by_province"]["3000"], 4)
        self.assertEqual(out["state_buildings_by_state"]["2"]["arms_factory"], 1)
        # redaction: basenames only, no separators / drive letters / user dirs
        self.assertEqual(out["melt_file"], "autosave.hoi4")
        self.assertEqual(out["state_detail_file"], "state_detail.json")
        blob = repr(out)
        self.assertNotIn("Users", blob)
        self.assertNotIn("\\", blob)
        self.assertNotIn("/home", blob)

    def test_save_buildings_absent_by_default(self):
        snap = asm.assemble_snapshot({}, game_id="g", date="1940.9.10.13",
                                     identity=FakeIdentity())
        self.assertNotIn("_save_buildings", snap)
        self.assertNotIn("buildings", snap["provenance"]["memory"]["domains"])

    def test_save_intel_keeps_indices_not_tags(self):
        # Codex Q3: target indices are raw ints, NEVER fabricated TAGs.
        res = {"intel": self._res("intel", {
            "source": "save", "from": "intel_save", "melt_path": "X.hoi4",
            "by_observer": {"GER": {"5": {"civ": 1.0}}},
            "target_index_is_tag": False})}
        snap = asm.assemble_snapshot(res, game_id="g", date="1940.9.10.13",
                                     identity=FakeIdentity())
        self.assertEqual(schema.validate_snapshot(snap), [])
        # save backstop -> NOT a memory domain (review #5); in the save channel.
        self.assertNotIn("intel", snap["provenance"]["memory"]["domains"])
        self.assertIn("intel", snap["provenance"]["save"]["backstops"])
        si = snap["_save_intel"]
        self.assertFalse(si["target_index_is_tag"])
        # observer is a real TAG; target key stays the raw index "5", not a TAG
        self.assertIn("5", si["by_observer"]["GER"])


    def test_naval_sidecar_strips_raw_tagid(self):
        # adversarial review #4: a raw tag-db-id left on a naval entry by the
        # reader must be stripped before it reaches the sidecar (no numeric id
        # masquerading as identity).
        from live_capture import registry as _r
        ents = [{"controller": "GER", "controller_tagid": 42, "zone": 1}]
        out = _r._sanitize_naval_entries(ents)
        self.assertEqual(out[0]["controller"], "GER")
        self.assertNotIn("controller_tagid", out[0])
        # an un-nameable "?<id>" controller becomes None and the raw id is gone
        ents2 = [{"controller": "?42", "controller_id": 42}]
        out2 = _r._sanitize_naval_entries(ents2)
        self.assertIsNone(out2[0]["controller"])
        self.assertNotIn("controller_id", out2[0])


class TestWiredLiveDomains(unittest.TestCase):
    """MOCK-based coverage for the 6 LIVE domains wired into the capture path:
    country_scalars EXTRAS (manpower_pool + convoys), country_misc, state_detail,
    designs (templates), mio (mio_live), and ships (order_of_battle.ships). NO live
    memory -- every reader output is a canned DomainResult. Asserts the new fields
    land where expected, measured 0 survives, and validate_snapshot still passes."""

    def _res(self, name, data):
        return reg.DomainResult(name, available=True, captured=True, data=data)

    def _full(self):
        return {
            "country_scalars": self._res("country_scalars", {"countries": {
                "GER": {"stability": 0.6, "num_ships": 30, "command_power": 25.0,
                        "manpower_pool": 1320000, "convoys": 70.0},
                "SOV": {"manpower_pool": 0, "convoys": 0.0},  # measured zeros
            }, "count": 2}),
            "country_misc": self._res("country_misc", {"countries": {
                "GER": {"convoys_destroyed": 12, "accidents_score": 0.5,
                        "ace_count": 3, "cosmetic_tag": None},
                "SOV": {"convoys_destroyed": 0, "accidents_score": 0.0,
                        "ace_count": 0, "cosmetic_tag": "SOV_communist"},
            }, "count": 2}),
            "state_detail": self._res("state_detail", {"states": {
                "64": {"manpower_available": 100, "manpower_locked": 5,
                       "manpower_total": 105, "resistance": 0.0, "compliance": 0.0,
                       "state_category": "rural"},
                "200": {"manpower_available": 0, "manpower_locked": 0,
                        "manpower_total": 0, "resistance": 0.42, "compliance": 0.1,
                        "state_category": "town"},
            }, "count": 2}),
            "designs": self._res("designs", {"division_templates": [
                {"id": 1, "name": "Infanterie", "priority": 1, "division_cap": 0,
                 "n_regiments": 6, "n_support": 1, "regiments": ["infantry"] * 6,
                 "support": ["engineer"]}], "count": 1, "variants_available": False}),
            "mio": self._res("mio", {"by_tag": {"GER": [
                {"id": 10, "owner_tag_db_id": 1, "owner": "GER", "level": 3}]},
                "by_id": {"10": {"id": 10, "owner_tag_db_id": 1, "owner": "GER",
                                 "level": 3}}, "count": 1}),
            "ships": self._res("ships", {"ships": [
                {"instance_id": 5001, "definition": "ger_battleship",
                 "category": "battleship", "strength_hp": 95.0,
                 "organisation": 100.0, "experience": 0.0, "sunk_convoys": 3}],
                "count": 1}),
        }

    def test_all_six_domains_fold_and_validate(self):
        snap = asm.assemble_snapshot(self._full(), game_id="g",
                                     date="1940.9.10.13", identity=FakeIdentity())
        self.assertEqual(schema.validate_snapshot(snap), [])
        doms = snap["provenance"]["memory"]["domains"]
        for d in ("country_scalars", "country_misc", "state_detail", "designs",
                  "mio", "ships"):
            self.assertIn(d, doms)

    def test_country_scalars_extras_manpower_and_convoys(self):
        snap = asm.assemble_snapshot(self._full(), game_id="g",
                                     date="1940.9.10.13", identity=FakeIdentity())
        ger = snap["countries"]["GER"]; sov = snap["countries"]["SOV"]
        self.assertEqual(ger["military"]["manpower"]["available"], 1320000)
        self.assertEqual(ger["military"]["convoys_count"], 70)
        # measured ZERO survives (not coerced to null)
        self.assertEqual(sov["military"]["manpower"]["available"], 0)
        self.assertEqual(sov["military"]["convoys_count"], 0)

    def test_country_misc_folds_to_country_misc(self):
        snap = asm.assemble_snapshot(self._full(), game_id="g",
                                     date="1940.9.10.13", identity=FakeIdentity())
        ger = snap["countries"]["GER"]; sov = snap["countries"]["SOV"]
        self.assertEqual(ger["misc"]["convoys_destroyed"], 12)
        self.assertEqual(ger["misc"]["ace_count"], 3)
        # cosmetic_tag is MEASURED by the reader; None == "no cosmetic tag set"
        # (a measured absence) and is written through, not omitted (Codex review #2)
        self.assertIn("cosmetic_tag", ger["misc"])
        self.assertIsNone(ger["misc"]["cosmetic_tag"])
        self.assertEqual(sov["misc"]["convoys_destroyed"], 0)
        self.assertEqual(sov["misc"]["cosmetic_tag"], "SOV_communist")

    def test_state_detail_folds_manpower_occupation_category(self):
        snap = asm.assemble_snapshot(self._full(), game_id="g",
                                     date="1940.9.10.13", identity=FakeIdentity())
        s64 = snap["states"]["64"]; s200 = snap["states"]["200"]
        self.assertEqual(s64["manpower_pool"],
                         {"available": 100, "locked": 5, "total": 105})
        self.assertEqual(s64["category"], "rural")
        # measured-zero resistance/compliance kept
        self.assertEqual(s64["occupation"],
                         {"resistance": 0.0, "compliance": 0.0})
        self.assertEqual(s200["occupation"]["resistance"], 0.42)

    def test_designs_templates_live_variants_left_to_save(self):
        snap = asm.assemble_snapshot(self._full(), game_id="g",
                                     date="1940.9.10.13", identity=FakeIdentity())
        self.assertEqual(len(snap["designs"]["division_templates"]), 1)
        self.assertEqual(snap["designs"]["division_templates_source"], "M")
        self.assertFalse(snap["designs"]["live_variants_available"])
        # variants NOT fabricated by the live reader -> still the empty save home
        self.assertEqual(snap["designs"]["equipment_variants"], {})

    def test_mio_folds_to_top_level_not_country_mios(self):
        snap = asm.assemble_snapshot(self._full(), game_id="g",
                                     date="1940.9.10.13", identity=FakeIdentity())
        self.assertEqual(snap["mio_live"]["source"], "M")
        self.assertIn("GER", snap["mio_live"]["by_tag"])
        self.assertEqual(snap["mio_live"]["by_id"]["10"]["owner"], "GER")
        # the save-sourced per-country Country.mios is NOT polluted by live MIOs
        self.assertEqual(snap["countries"]["GER"]["mios"], [])

    def test_ships_fold_to_order_of_battle_ships(self):
        snap = asm.assemble_snapshot(self._full(), game_id="g",
                                     date="1940.9.10.13", identity=FakeIdentity())
        ships = snap["order_of_battle"]["ships"]
        self.assertEqual(len(ships), 1)
        self.assertEqual(ships[0]["instance_id"], 5001)
        self.assertEqual(ships[0]["definition"], "ger_battleship")
        self.assertEqual(snap["order_of_battle"]["ships_source"], "M")

    def test_absent_wired_domains_stay_null_and_unclaimed(self):
        snap = asm.assemble_snapshot({}, game_id="g", date="1940.9.10.13",
                                     identity=FakeIdentity())
        self.assertEqual(schema.validate_snapshot(snap), [])
        self.assertEqual(snap["order_of_battle"]["ships"], [])
        self.assertIsNone(snap["mio_live"]["source"])
        self.assertEqual(snap["designs"]["division_templates"], [])
        doms = snap["provenance"]["memory"]["domains"]
        for d in ("country_misc", "state_detail", "designs", "mio", "ships"):
            self.assertNotIn(d, doms)


class _FakeProjectsModule:
    """A stand-in for the projects_live MODULE exposing the PINNED *_from_addrs API the registry
    reader now consumes (enumerate_<pool>_addrs -> the sweep; read_<pool>_from_addrs -> decode;
    validate_<pool>_addrs -> the fail-closed gate). Counts enumerate/validate calls so a test can
    prove the warm walk SKIPS the sweep. ``valid`` toggles the validate gate; ``sci_raises``
    makes the scientist decode blow up (to exercise the partial-failure -> whole-domain-omit)."""

    def __init__(self, fac_rows, proj_rows, sci_rows, valid=True, sci_raises=False):
        self._fac = list(fac_rows or [])
        self._proj = list(proj_rows or [])
        self._sci = list(sci_rows or [])
        self.valid = valid
        self._sci_raises = sci_raises
        self.enum_calls = 0
        self.validate_calls = 0

    # addresses are just the row indices here (the real ones are (addr, fp) tuples; the reader
    # treats them opaquely, so any stable token works for the mock).
    def enumerate_facility_addrs(self, p):
        self.enum_calls += 1
        return list(range(len(self._fac)))

    def enumerate_project_addrs(self, p):
        self.enum_calls += 1
        return list(range(len(self._proj)))

    def enumerate_scientist_addrs(self, p):
        self.enum_calls += 1
        return list(range(len(self._sci)))

    def read_facilities_from_addrs(self, p, addrs):
        return [dict(self._fac[i]) for i in addrs]

    def read_projects_from_addrs(self, p, addrs, active_only=False):
        return [dict(self._proj[i]) for i in addrs]

    def read_scientists_from_addrs(self, p, addrs):
        if self._sci_raises:
            raise RuntimeError("scientist read failed")
        return [dict(self._sci[i]) for i in addrs]

    def validate_facility_addrs(self, p, addrs):
        self.validate_calls += 1
        return self.valid

    def validate_project_addrs(self, p, addrs):
        self.validate_calls += 1
        return self.valid

    def validate_scientist_addrs(self, p, addrs):
        self.validate_calls += 1
        return self.valid


class _FakeTechModule:
    """A stand-in for the tech_live MODULE exposing the PINNED API (enumerate_tech_addrs /
    read_country_tech_from_addrs / validate_tech_addrs) on top of an OLD-style read_country_tech
    fn. Lets the existing tech tests (which provide a read_country_tech(p, bridge, strict) fn)
    drive the now-pinned tech reader. TECH_PROGRESS_SCALE mirrors the real module constant."""

    TECH_PROGRESS_SCALE = 1000.0

    def __init__(self, fn, valid=True):
        self._fn = fn
        self.valid = valid
        self.enum_calls = 0

    def enumerate_tech_addrs(self, p):
        self.enum_calls += 1
        # one sentinel status address so the pin treats the pool as non-empty (the fake fn
        # ignores the addrs and returns canned data).
        return {"status_addrs": [(0x1, 0x1)], "slot_addrs": []}

    def read_country_tech_from_addrs(self, p, status_addrs, slot_addrs, bridge=None,
                                     strict=True, rec0=None, count=None):
        return self._fn(p, bridge=bridge, strict=strict)

    def validate_tech_addrs(self, p, status_addrs, slot_addrs):
        return self.valid


def _wrap_tech(fn):
    """Wrap an old-style read_country_tech fn as a fake tech_live module for the pinned reader."""
    return _FakeTechModule(fn)


class _FakeOobModule:
    """A stand-in for the oob_command_live MODULE exposing the PINNED locate API
    (enumerate_command_objs / read_command_tree_from_objs / validate_command_objs) on top of an
    OLD-style read_command_tree(p) fn."""

    def __init__(self, fn, valid=True):
        self._fn = fn
        self.valid = valid
        self.enum_calls = 0

    def enumerate_command_objs(self, p):
        self.enum_calls += 1
        # a non-empty sentinel set so the pin treats the locate as resolved (the fake fn
        # ignores objs and returns the canned tree).
        return {"_arena": (0x1, 0x2), "army": [(0x10, (1, 53))], "ag": [], "fleet": [],
                "tf": [], "theatre": [], "div": [], "ship": []}

    def read_command_tree_from_objs(self, p, objs):
        return self._fn(p)

    def validate_command_objs(self, p, objs):
        return self.valid


def _wrap_oob(fn):
    """Wrap an old-style read_command_tree fn as a fake oob_command_live module."""
    return _FakeOobModule(fn)


class TestProjectsLiveDomain(unittest.TestCase):
    """MOCK-based coverage for the LIVE special-projects domain wired into the
    capture path: projects_live -> top-level snap["special_projects"] (facilities /
    projects / scientists global pools, source="M"). NO live memory -- the reader
    output is a canned DomainResult. Asserts the lists land world-scoped (NOT keyed
    by TAG), raw handles are kept as ints (never resolved to a name), measured-empty
    is preserved, the save-sourced Country.projects is NOT polluted, and
    validate_snapshot still passes."""

    def _res(self, name, data, *, available=True, captured=True, error=None):
        return reg.DomainResult(name, available=available, captured=captured,
                                data=data, error=error)

    def _full(self):
        return {"projects_live": self._res("projects_live", {
            "facilities": [
                {"id": 3, "province": 64, "building": "nuclear_facility",
                 "active_project_handle": 0x5612, "active": True,
                 "scientist_char_id": 12000, "unread_prototype_rewards": 2},
                # idle facility: no scientist, handle 0 -> measured idle (kept)
                {"id": 4, "province": 200, "building": "land_facility",
                 "active_project_handle": 0, "active": False,
                 "scientist_char_id": None, "unread_prototype_rewards": 0},
            ],
            "projects": [
                {"id": 800, "static_def_handle": 0x5612, "iterations": 5,
                 "worked_at_facility": 3},
                {"id": 801, "static_def_handle": 0x56A0, "iterations": 0,
                 "worked_at_facility": None},
            ],
            "scientists": [
                {"char_id": 12000, "name_token": "GER_heisenberg",
                 "display_name": "Werner Heisenberg",
                 "skills": {"specialization_nuclear": 5, "specialization_naval": 0,
                            "specialization_air": 1, "specialization_land": 0},
                 "trait_count": 2},
            ],
            "count": {"facilities": 2, "projects": 2, "scientists": 1},
        })}

    def test_projects_live_folds_and_validates(self):
        snap = asm.assemble_snapshot(self._full(), game_id="g",
                                     date="1940.9.10.13", identity=FakeIdentity())
        self.assertEqual(schema.validate_snapshot(snap), [])
        self.assertIn("projects_live", snap["provenance"]["memory"]["domains"])
        sp = snap["special_projects"]
        self.assertEqual(sp["source"], "M")
        self.assertEqual(len(sp["facilities"]), 2)
        self.assertEqual(len(sp["projects"]), 2)
        self.assertEqual(len(sp["scientists"]), 1)
        self.assertEqual(sp["count"],
                         {"facilities": 2, "projects": 2, "scientists": 1})

    def test_raw_handles_kept_as_ints_not_resolved(self):
        # static-def db-id HANDLES stay raw ints (the token-name registry is
        # unlocated); they are NEVER fabricated into a name/token.
        snap = asm.assemble_snapshot(self._full(), game_id="g",
                                     date="1940.9.10.13", identity=FakeIdentity())
        sp = snap["special_projects"]
        p800 = [p for p in sp["projects"] if p["id"] == 800][0]
        self.assertEqual(p800["static_def_handle"], 0x5612)
        f3 = [f for f in sp["facilities"] if f["id"] == 3][0]
        self.assertEqual(f3["active_project_handle"], 0x5612)
        # idle facility's measured zeros survive (handle 0, unread 0, no scientist)
        f4 = [f for f in sp["facilities"] if f["id"] == 4][0]
        self.assertEqual(f4["active_project_handle"], 0)
        self.assertEqual(f4["unread_prototype_rewards"], 0)
        self.assertIsNone(f4["scientist_char_id"])

    def test_global_lists_not_keyed_by_tag_and_country_projects_untouched(self):
        # the live pools are world-scoped global lists (no owner TAG); the
        # save-sourced per-country Country.projects / Country.scientists stay null
        # (no live attribution fabricated).
        snap = asm.assemble_snapshot(self._full(), game_id="g",
                                     date="1940.9.10.13", identity=FakeIdentity())
        # no country shells were created by the projects_live fold
        self.assertEqual(snap["countries"], {})

    def test_measured_empty_pools_stay_empty_and_source_m(self):
        # a MEASURED empty pool (reader returned []) is a measured zero -> [] + M,
        # never null/absent.
        res = {"projects_live": self._res("projects_live", {
            "facilities": [], "projects": [], "scientists": [],
            "count": {"facilities": 0, "projects": 0, "scientists": 0}})}
        snap = asm.assemble_snapshot(res, game_id="g", date="1940.9.10.13",
                                     identity=FakeIdentity())
        self.assertIn("projects_live", snap["provenance"]["memory"]["domains"])
        sp = snap["special_projects"]
        self.assertEqual(sp["facilities"], [])
        self.assertEqual(sp["scientists"], [])
        self.assertEqual(sp["source"], "M")

    def test_absent_projects_live_stays_null_and_unclaimed(self):
        # domain not captured -> special_projects stays the null-but-present skeleton
        # (lists None, source None) and is NOT in provenance (no fabrication).
        snap = asm.assemble_snapshot({}, game_id="g", date="1940.9.10.13",
                                     identity=FakeIdentity())
        self.assertEqual(schema.validate_snapshot(snap), [])
        sp = snap["special_projects"]
        self.assertIsNone(sp["facilities"])
        self.assertIsNone(sp["projects"])
        self.assertIsNone(sp["scientists"])
        self.assertIsNone(sp["source"])
        self.assertNotIn("projects_live",
                         snap["provenance"]["memory"]["domains"])

    def test_failed_projects_live_omitted(self):
        # a plug that raised (captured=False) -> domain omitted, block stays null.
        res = {"projects_live": self._res("projects_live", None, captured=False,
                                          error="boom")}
        snap = asm.assemble_snapshot(res, game_id="g", date="1940.9.10.13",
                                     identity=FakeIdentity())
        self.assertNotIn("projects_live",
                         snap["provenance"]["memory"]["domains"])
        self.assertIsNone(snap["special_projects"]["facilities"])

    def test_reader_strips_addr(self):
        # the registry reader must strip the diagnostic _addr from every row so the
        # heap address never reaches the snapshot. Drive the reader fn directly with
        # a fake projects-module impl (no live memory). The reader now consumes the PINNED
        # *_from_addrs API (Stage 1.5), so the fake module exposes that surface.
        rows_fac = [{"id": 1, "province": 5, "_addr": 0x123}]
        rows_proj = [{"id": 9, "static_def_handle": 0x56AA, "_addr": 0x456}]
        rows_sci = [{"char_id": 7, "_addr": 0x789}]
        fake_impl = _FakeProjectsModule(rows_fac, rows_proj, rows_sci)

        class _FakeProc:
            pass
        ctx = reg.CaptureContext(proc=_FakeProc(), extras={"pins": {}})
        plugs = {p.name: p for p in reg.default_registry()}
        out = plugs["projects_live"]._reader(ctx, fake_impl)
        self.assertNotIn("_addr", out["facilities"][0])
        self.assertNotIn("_addr", out["projects"][0])
        self.assertNotIn("_addr", out["scientists"][0])
        self.assertEqual(out["count"],
                         {"facilities": 1, "projects": 1, "scientists": 1})
        blob = repr(out)
        self.assertNotIn("_addr", blob)

    def test_reader_warm_pin_skips_rescan(self):
        # Stage 1.5: a warm walk (validate passes) re-reads the pools WITHOUT re-running the
        # enumerate_*_addrs sweeps, and the output is identical.
        rows_fac = [{"id": 1, "province": 5, "_addr": 0x123}]
        rows_proj = [{"id": 9, "static_def_handle": 0x56AA, "_addr": 0x456}]
        rows_sci = [{"char_id": 7, "_addr": 0x789}]
        fake_impl = _FakeProjectsModule(rows_fac, rows_proj, rows_sci)

        class _FakeProc:
            pass
        ctx = reg.CaptureContext(proc=_FakeProc(), identity=_FpIdentity(),
                                 extras={"pins": {}})
        out1 = reg._r_projects(ctx, fake_impl)
        self.assertEqual(fake_impl.enum_calls, 3)        # cold: 3 sweeps (fac/proj/sci)
        out2 = reg._r_projects(ctx, fake_impl)
        self.assertEqual(fake_impl.enum_calls, 3)        # warm: NO new sweep
        self.assertEqual(out1, out2)                     # output-identical
        self.assertGreaterEqual(fake_impl.validate_calls, 3)  # validate ran on the warm walk

    def test_reader_warm_pin_invalidates_on_validate_miss(self):
        # a validate MISS (an object moved/freed) -> the warm walk re-scans (fail-closed).
        rows_fac = [{"id": 1, "province": 5, "_addr": 0x123}]
        fake_impl = _FakeProjectsModule(rows_fac, [], [{"char_id": 7, "_addr": 0x789}])

        class _FakeProc:
            pass
        ctx = reg.CaptureContext(proc=_FakeProc(), identity=_FpIdentity(),
                                 extras={"pins": {}})
        reg._r_projects(ctx, fake_impl)
        self.assertEqual(fake_impl.enum_calls, 3)
        fake_impl.valid = False                          # the pinned pool no longer validates
        reg._r_projects(ctx, fake_impl)
        self.assertEqual(fake_impl.enum_calls, 6)        # full re-scan (3 more sweeps)

    def test_reader_raises_without_proc(self):
        # offline (ctx.proc is None) -> the reader raises so the domain is honestly
        # omitted rather than fabricating an empty live read.
        ctx = reg.CaptureContext(proc=None)
        plugs = {p.name: p for p in reg.default_registry()}
        fake_impl = _FakeProjectsModule([], [], [])
        with self.assertRaises(Exception):
            plugs["projects_live"]._reader(ctx, fake_impl)

    def test_partial_read_failure_propagates(self):
        # if one sub-read raises, the WHOLE domain is omitted (no partial block).
        fake_impl = _FakeProjectsModule([{"id": 1, "province": 2}], [], [],
                                        sci_raises=True)

        class _FakeProc:
            pass
        ctx = reg.CaptureContext(proc=_FakeProc(), extras={"pins": {}})
        plug = {p.name: p for p in reg.default_registry()}["projects_live"]
        # the reader fn raises (the scientist sub-read blows up inside _pin_projects._resolve);
        # the plug's read() wraps it transactionally.
        with self.assertRaises(Exception):
            plug._reader(ctx, fake_impl)
        # via the transactional plug.read(): captured False, data discarded.
        plug._impl = fake_impl
        plug._available = True
        dr = plug.read(reg.CaptureContext(proc=_FakeProc(), extras={"pins": {}}))
        self.assertFalse(dr.captured)
        self.assertIsNone(dr.data)


class _FakeTagTable:
    """Minimal tag table for the reader-level oob_command tests: maps a tag-db-id to a
    TAG (mirrors live_layers tag_table.tag(id))."""

    def __init__(self, mapping):
        self._m = dict(mapping)

    def tag(self, tag_db_id):
        return self._m.get(int(tag_db_id))


class TestOobCommandLiveDomain(unittest.TestCase):
    """MOCK-based coverage for the LIVE OOB command-hierarchy domain (oob_command):
    folds the command TREE into top-level order_of_battle.{armies,army_groups,fleets,
    theatres} (NOT by_country), coexists with the ships-owned order_of_battle.ships leaf,
    keeps membership/leader ids as raw ints (only owner -> TAG), preserves measured-empty,
    and is protected from the save backstop. NO live memory -- canned DomainResult / a
    fake reader fn drives the registry reader directly."""

    def _res(self, name, data, *, available=True, captured=True, error=None):
        return reg.DomainResult(name, available=available, captured=captured,
                                data=data, error=error)

    def _data(self):
        # ASSEMBLER-SHAPE data (already plug-processed): top-level lists, owner is a TAG.
        return {
            "armies": [
                {"id": 53001, "owner": "GER", "member_div_ids": [100, 101, 102],
                 "member_count_engine": 4, "live_member_count": 3, "missing_count": 1,
                 "leader_id": 9001, "army_group_id": 53900},
                {"id": 53002, "owner": "SOV", "member_div_ids": [],  # measured-empty
                 "member_count_engine": 0, "live_member_count": 0, "missing_count": 0,
                 "leader_id": None, "army_group_id": None},
            ],
            "army_groups": [
                {"id": 53900, "owner": "GER", "member_army_ids": [53001],
                 "live_member_count": 1},
            ],
            "fleets": [
                {"id": 61500, "owner": "GER", "task_force_ids": [61700],
                 "live_task_force_count": 1,
                 "task_forces": [{"id": 61700, "fleet_id": 61500,
                                  "ship_ids": [5001, 5002], "ship_count": 2}]},
            ],
            "theatres": [{"id": 67001}],
            "orphan_task_forces": [],
            "counts": {"armies": 2, "army_groups": 1, "fleets": 1, "theatres": 1,
                       "task_forces": 1},
        }

    def _full(self):
        return {"oob_command": self._res("oob_command", self._data())}

    def test_command_tree_folds_to_top_level_order_of_battle(self):
        snap = asm.assemble_snapshot(self._full(), game_id="g",
                                     date="1940.9.10.13", identity=FakeIdentity())
        self.assertEqual(schema.validate_snapshot(snap), [])
        self.assertIn("oob_command", snap["provenance"]["memory"]["domains"])
        oob = snap["order_of_battle"]
        self.assertEqual(len(oob["armies"]), 2)
        self.assertEqual(len(oob["army_groups"]), 1)
        self.assertEqual(len(oob["fleets"]), 1)
        self.assertEqual(len(oob["theatres"]), 1)
        # per-list M source flags set
        for k in ("armies", "army_groups", "fleets", "theatres"):
            self.assertEqual(oob[k + "_source"], "M")
        # owner is a resolved TAG; membership ids are RAW ints (not tags)
        a = [x for x in oob["armies"] if x["id"] == 53001][0]
        self.assertEqual(a["owner"], "GER")
        self.assertEqual(a["member_div_ids"], [100, 101, 102])
        self.assertEqual(a["leader_id"], 9001)         # unit_leader id (raw int)
        self.assertEqual(a["army_group_id"], 53900)    # raw army-group id
        self.assertEqual(a["missing_count"], 1)        # measured frame gap surfaced
        # fleet nests its task forces, each with raw ship_ids
        f = oob["fleets"][0]
        self.assertEqual(f["task_forces"][0]["ship_ids"], [5001, 5002])

    def test_measured_empty_army_members_survive(self):
        snap = asm.assemble_snapshot(self._full(), game_id="g",
                                     date="1940.9.10.13", identity=FakeIdentity())
        sov = [x for x in snap["order_of_battle"]["armies"] if x["id"] == 53002][0]
        self.assertEqual(sov["member_div_ids"], [])    # measured empty, not None
        self.assertEqual(sov["missing_count"], 0)      # measured zero kept
        self.assertIsNone(sov["leader_id"])            # leaderless -> None

    def test_oob_command_coexists_with_ships_leaf(self):
        # the ships domain owns order_of_battle.ships; oob_command must NOT clobber it.
        res = self._full()
        res["ships"] = self._res("ships", {"ships": [
            {"instance_id": 5001, "definition": "ger_bb", "category": "battleship",
             "strength_hp": 95.0, "organisation": 100.0, "experience": 0.0,
             "sunk_convoys": 3}], "count": 1})
        snap = asm.assemble_snapshot(res, game_id="g", date="1940.9.10.13",
                                     identity=FakeIdentity())
        self.assertEqual(schema.validate_snapshot(snap), [])
        oob = snap["order_of_battle"]
        # both live structures present on the SAME dict
        self.assertEqual(len(oob["ships"]), 1)
        self.assertEqual(oob["ships_source"], "M")
        self.assertEqual(len(oob["armies"]), 2)
        self.assertEqual(oob["armies_source"], "M")
        for d in ("ships", "oob_command"):
            self.assertIn(d, snap["provenance"]["memory"]["domains"])

    def test_absent_oob_command_stays_null_and_unclaimed(self):
        snap = asm.assemble_snapshot({}, game_id="g", date="1940.9.10.13",
                                     identity=FakeIdentity())
        self.assertEqual(schema.validate_snapshot(snap), [])
        oob = snap["order_of_battle"]
        # skeleton: lists empty-but-present, no M source flags, domain not claimed
        self.assertEqual(oob["armies"], [])
        self.assertEqual(oob["fleets"], [])
        self.assertNotIn("armies_source", oob)
        self.assertNotIn("oob_command", snap["provenance"]["memory"]["domains"])

    def test_failed_oob_command_omitted(self):
        res = {"oob_command": self._res("oob_command", None, captured=False,
                                        error="boom")}
        snap = asm.assemble_snapshot(res, game_id="g", date="1940.9.10.13",
                                     identity=FakeIdentity())
        self.assertNotIn("oob_command", snap["provenance"]["memory"]["domains"])
        self.assertEqual(snap["order_of_battle"]["armies"], [])

    def test_orphan_task_forces_surfaced_not_dropped(self):
        # a live TF whose fleet parent missed this frame is a MEASURED observation; the
        # assembler keeps it in an order_of_battle sidecar rather than silently dropping it.
        data = self._data()
        data["orphan_task_forces"] = [{"id": 61999, "fleet_id": None,
                                       "ship_ids": [7001], "ship_count": 1}]
        res = {"oob_command": self._res("oob_command", data)}
        snap = asm.assemble_snapshot(res, game_id="g", date="1940.9.10.13",
                                     identity=FakeIdentity())
        self.assertEqual(schema.validate_snapshot(snap), [])
        self.assertEqual(snap["order_of_battle"]["orphan_task_forces"],
                         [{"id": 61999, "fleet_id": None, "ship_ids": [7001],
                           "ship_count": 1}])

    def test_malformed_oob_command_fails_closed(self):
        # FAIL-CLOSED: a CAPTURED result missing a required command list (or non-list)
        # is a contract violation, NOT a measured-empty -> fold nothing, domain unclaimed.
        bad = self._data()
        del bad["fleets"]                       # required leaf absent -> malformed
        res = {"oob_command": self._res("oob_command", bad)}
        snap = asm.assemble_snapshot(res, game_id="g", date="1940.9.10.13",
                                     identity=FakeIdentity())
        self.assertEqual(schema.validate_snapshot(snap), [])
        # nothing folded (skeleton empty), no M source flags, domain NOT claimed
        oob = snap["order_of_battle"]
        self.assertEqual(oob["armies"], [])
        self.assertNotIn("armies_source", oob)
        self.assertNotIn("oob_command", snap["provenance"]["memory"]["domains"])

    # -- reader-level: tag-vs-int discipline + obj strip, driven by a fake reader fn --
    def test_reader_resolves_owner_keeps_ids_raw_strips_obj(self):
        # RAW reader-shape (read_command_tree output): owner_tag_db_id + obj heap addr.
        def _fake_tree(p):
            return {
                "armies": {53001: {
                    "id": 53001, "type": 53, "member_div_ids": [100, 101],
                    "member_count_engine": 2, "live_member_count": 2,
                    "missing_count": 0, "owner_tag_db_id": 1, "leader_id": 9001,
                    "army_group_id": 53900, "obj": 0xDEADBEEF}},
                "army_groups": {53900: {
                    "id": 53900, "type": 53, "member_army_ids": [53001],
                    "live_member_count": 1, "owner_tag_db_id": 1, "obj": 0xCAFE}},
                "fleets": {61500: {
                    "id": 61500, "type": 61, "task_force_ids": [61700],
                    "live_task_force_count": 1, "owner_tag_db_id": 2, "obj": 0xF00D}},
                "task_forces": {61700: {
                    "id": 61700, "type": 61, "fleet_id": 61500,
                    "ship_ids": [5001], "ship_count": 1, "obj": 0xBEEF},
                    # a task force whose live fleet parent missed -> orphan sidecar
                    61701: {"id": 61701, "type": 61, "fleet_id": None,
                            "ship_ids": [5002], "ship_count": 1, "obj": 0xBAD}},
                "theatres": {67001: {"id": 67001, "type": 67, "obj": 0x1234}},
                "_meta": {"counts": {}},
            }
        ctx = reg.CaptureContext(proc=object(),
                                 tag_table=_FakeTagTable({1: "GER", 2: "SOV"}))
        plug = {p.name: p for p in reg.default_registry()}["oob_command"]
        out = plug._reader(ctx, _wrap_oob(_fake_tree))
        # owner_tag_db_id RESOLVED to a TAG; the raw obj heap addr STRIPPED everywhere
        a = out["armies"][0]
        self.assertEqual(a["owner"], "GER")
        self.assertNotIn("obj", a)
        self.assertNotIn("owner_tag_db_id", a)
        # membership / leader / group ids stay RAW ints (NOT tag-resolved)
        self.assertEqual(a["member_div_ids"], [100, 101])
        self.assertEqual(a["leader_id"], 9001)
        self.assertEqual(a["army_group_id"], 53900)
        self.assertEqual(out["army_groups"][0]["owner"], "GER")
        self.assertEqual(out["army_groups"][0]["member_army_ids"], [53001])
        # fleet owner resolved; its task force nested with raw ship ids
        f = out["fleets"][0]
        self.assertEqual(f["owner"], "SOV")
        self.assertEqual(f["task_forces"][0]["ship_ids"], [5001])
        # orphan TF (no live fleet parent) kept in the sidecar, not silently dropped
        self.assertEqual(len(out["orphan_task_forces"]), 1)
        self.assertEqual(out["orphan_task_forces"][0]["id"], 61701)
        # the heap addr must NEVER survive anywhere in the emitted payload
        self.assertNotIn("obj", repr(out))

    def test_reader_unresolvable_owner_becomes_none(self):
        # an owner_tag_db_id the tag table can't name -> owner None (never a numeric id)
        def _fake_tree(p):
            return {"armies": {1: {"id": 1, "member_div_ids": [], "owner_tag_db_id": 999,
                                   "member_count_engine": 0, "live_member_count": 0,
                                   "missing_count": 0, "leader_id": None,
                                   "army_group_id": None, "obj": 0x1}},
                    "army_groups": {}, "fleets": {}, "task_forces": {}, "theatres": {}}
        ctx = reg.CaptureContext(proc=object(),
                                 tag_table=_FakeTagTable({1: "GER"}))
        plug = {p.name: p for p in reg.default_registry()}["oob_command"]
        out = plug._reader(ctx, _wrap_oob(_fake_tree))
        self.assertIsNone(out["armies"][0]["owner"])

    def test_reader_raises_without_proc(self):
        ctx = reg.CaptureContext(proc=None)
        plug = {p.name: p for p in reg.default_registry()}["oob_command"]
        with self.assertRaises(Exception):
            plug._reader(ctx, lambda p: {})

    # -- save backstop: live command lists WIN over a stale save oob (leaf-protected) --
    def test_save_backstop_cannot_overwrite_live_command_lists(self):
        # mirror fold_save_bundle's _protect_top re-assert: the live armies/army_groups/
        # fleets/theatres must survive a (stale) save order_of_battle merge.
        snap = asm.assemble_snapshot(self._full(), game_id="g",
                                     date="1940.9.10.13", identity=FakeIdentity())
        live_mem = snap["provenance"]["memory"]["domains"]
        self.assertIn("oob_command", live_mem)
        # capture the live leaves (as fold_save_bundle does), then merge a stale save OOB
        import copy as _copy
        protect = [("order_of_battle", leaf) for leaf in
                   ("armies", "army_groups", "fleets", "theatres")]
        live_top = {(k, leaf): _copy.deepcopy(snap[k][leaf]) for (k, leaf) in protect}
        save_oob = schema.empty_order_of_battle()
        save_oob["armies"] = [{"id": 99999, "owner": "STALE"}]       # stale save roster
        save_oob["fleets"] = [{"id": 88888, "owner": "STALE"}]
        save_oob["navy_theaters"] = [{"id": 7}]                      # save-only -> fills
        snap["order_of_battle"] = asm._backstop_merge(snap["order_of_battle"], save_oob)
        for (k, leaf), lv in live_top.items():
            if lv is not None and isinstance(snap.get(k), dict):
                snap[k][leaf] = lv
        oob = snap["order_of_battle"]
        # live command lists WON (stale save roster did not overwrite them)
        self.assertEqual([a["id"] for a in oob["armies"]], [53001, 53002])
        self.assertEqual([f["id"] for f in oob["fleets"]], [61500])
        # navy_theaters (NOT live-measured) was filled by the save backstop
        self.assertEqual(oob["navy_theaters"], [{"id": 7}])


class TestTechCommandersIntelLiveDomains(unittest.TestCase):
    """MOCK-based coverage for the 3 LIVE per-country domains wired into the same
    canonical save HOMES (research / commanders / intel) under DISTINCT *_live
    provenance ids: tech_live, commanders_live, intel_live. NO live memory -- every
    reader output is a canned DomainResult. Asserts the new fields land where Codex
    blessed them, measured 0 survives, the save-shaped research.in_progress is NOT
    polluted, the role-split save commander leaves are not clobbered, the nested
    agency capacity shape matches the save, diagnostic addresses never leak, and the
    save backstop cannot overwrite a live-measured value (leaf protection)."""

    def _res(self, name, data, *, available=True, captured=True, error=None):
        return reg.DomainResult(name, available=available, captured=captured,
                                data=data, error=error)

    def _full(self):
        return {
            "tech_live": self._res("tech_live", {"countries": {
                "GER": {"completed_tech_count": 120, "research_slot_count": 4,
                        "active_research": [
                            {"slot_index": 0, "tech_handle_raw": 0x1234,
                             "tech_def_index_raw": 5, "progress_raw": 9000}]},
                # measured zeros: 0 techs, 3 slots, no active research ([])
                "SOV": {"completed_tech_count": 0, "research_slot_count": 3,
                        "active_research": []},
            }, "count": 2}),
            "commanders_live": self._res("commanders_live", {
                "army_by_tag": {"GER": [
                    {"char_id": 1, "name": "Guderian", "country": "GER",
                     "attack": 5, "experience": 1.2, "level": None,
                     "trait_xp": [{"trait_id": 7, "trait_xp": 100}]}]},
                "navy_by_tag": {"GER": [
                    {"char_id": 2, "name": "Doenitz", "country": "GER",
                     "attack": 4, "experience": 0.0, "level": None}]},
                "counts_by_tag": {"GER": {"army_leaders": 1, "navy_leaders": 1,
                                          "characters": 3}},
                "count": 1}),
            "intel_live": self._res("intel_live", {"countries": {
                "GER": {"name": "Abwehr", "icon": "GFX_x",
                        "upgrade_progress": 24.0, "max_operative_count": 5,
                        "usable_operative_slots": 5, "defense": 6.625},
                # measured zeros
                "SOV": {"name": "NKVD", "icon": None, "upgrade_progress": 0.0,
                        "max_operative_count": 0, "usable_operative_slots": 0,
                        "defense": 0.0},
            }, "count": 2}),
        }

    def test_all_three_domains_fold_and_validate(self):
        snap = asm.assemble_snapshot(self._full(), game_id="g",
                                     date="1940.9.10.13", identity=FakeIdentity())
        self.assertEqual(schema.validate_snapshot(snap), [])
        doms = snap["provenance"]["memory"]["domains"]
        for d in ("tech_live", "commanders_live", "intel_live"):
            self.assertIn(d, doms)

    def test_tech_live_folds_counts_and_separate_active_leaf(self):
        snap = asm.assemble_snapshot(self._full(), game_id="g",
                                     date="1940.9.10.13", identity=FakeIdentity())
        ger = snap["countries"]["GER"]; sov = snap["countries"]["SOV"]
        self.assertEqual(ger["research"]["tech_count"], 120)
        self.assertEqual(ger["research"]["research_slots"], 4)
        # active_research lands in the live-only leaf, raw-handle shape preserved
        ipl = ger["research"]["in_progress_live"]
        self.assertEqual(ipl[0]["tech_handle_raw"], 0x1234)
        self.assertEqual(ipl[0]["progress_raw"], 9000)
        # the save-shaped in_progress leaf is NOT polluted by the live read
        self.assertEqual(ger["research"]["in_progress"], [])
        # measured zeros survive (0 techs; measured-empty active list stays [])
        self.assertEqual(sov["research"]["tech_count"], 0)
        self.assertEqual(sov["research"]["research_slots"], 3)
        self.assertEqual(sov["research"]["in_progress_live"], [])

    def test_commanders_live_unsplit_leaves_and_count_subkeys(self):
        snap = asm.assemble_snapshot(self._full(), game_id="g",
                                     date="1940.9.10.13", identity=FakeIdentity())
        cmd = snap["countries"]["GER"]["commanders"]
        self.assertEqual(len(cmd["army_leaders"]), 1)
        self.assertEqual(cmd["army_leaders"][0]["name"], "Guderian")
        self.assertEqual(len(cmd["navy_leaders"]), 1)
        # overall level stays RESISTED (null), trait_xp ids carried (not names)
        self.assertIsNone(cmd["army_leaders"][0]["level"])
        self.assertEqual(cmd["army_leaders"][0]["trait_xp"][0]["trait_id"], 7)
        # the role-split save leaves are NOT fabricated by the live fold
        self.assertEqual(cmd["generals"], [])
        self.assertEqual(cmd["field_marshals"], [])
        self.assertEqual(cmd["admirals"], [])
        # live count sub-keys written, save-only count keys absent (no fabrication)
        self.assertEqual(cmd["counts"],
                         {"army_leaders": 1, "navy_leaders": 1, "characters": 3})

    def test_intel_live_folds_nested_agency_capacity_shape(self):
        snap = asm.assemble_snapshot(self._full(), game_id="g",
                                     date="1940.9.10.13", identity=FakeIdentity())
        ger = snap["countries"]["GER"]["intel"]["agency"]
        sov = snap["countries"]["SOV"]["intel"]["agency"]
        self.assertEqual(ger["name"], "Abwehr")
        self.assertEqual(ger["upgrade_progress"], 24.0)
        # capacity nests under the SAME save shape (max_operatives/usable_slots/defense)
        self.assertEqual(ger["capacity"],
                         {"max_operatives": 5, "usable_slots": 5, "defense": 6.625})
        # measured zeros survive; icon None (unmeasured) is omitted, not written
        self.assertEqual(sov["capacity"]["max_operatives"], 0)
        self.assertEqual(sov["capacity"]["defense"], 0.0)
        self.assertNotIn("icon", sov)

    def test_diagnostic_addresses_never_leak(self):
        # _status_addr / agency_ptr / leader addr must never reach the snapshot.
        # Drive the readers directly with fake impls (no live memory).
        plugs = {p.name: p for p in reg.default_registry()}

        class _FakeProc:
            pass
        ctx = reg.CaptureContext(proc=_FakeProc())
        # tech: reader returns _status_addr; the plug reader must drop it
        def _fake_tech(p, bridge=None, strict=True, **kw):
            return {"GER": {"completed_tech_count": 1, "research_slot_count": 1,
                            "active_research": [], "_status_addr": 0xDEAD}}
        out_t = plugs["tech_live"]._reader(ctx, _wrap_tech(_fake_tech))
        self.assertNotIn("_status_addr", repr(out_t))
        # intel: reader returns agency_ptr; dropped
        class _FakeIntel:
            @staticmethod
            def read_all(p, bridge=None):
                return {"GER": {"name": "X", "icon": None, "upgrade_progress": 1.0,
                                "max_operative_count": 1, "usable_operative_slots": 1,
                                "defense": 1.0, "agency_ptr": 0xBEEF}}
        out_i = plugs["intel_live"]._reader(ctx, _FakeIntel)
        self.assertNotIn("agency_ptr", repr(out_i))
        self.assertNotIn("0xBEEF", repr(out_i))
        # commanders: the reader strips each leader's diagnostic 'addr' + the
        # reader-internal '_unpinned' marker at the strip point (the reader, not the
        # assembler -- mirrors the projects_live/_addr convention).
        class _FakeCmdReader:
            pid = None  # _verify_same_process returns early (no identity)
            def verify_build(self):
                return True, "sha"
            def close(self):
                pass
            _OUT = {
                "army_leaders": [{"char_id": 1, "name": "G", "country": "GER",
                                  "addr": "0xCAFE", "level": None,
                                  "_unpinned": ["level"]}],
                "navy_leaders": [],
                "characters": {1: {"country": "GER"}},
                "counts": {"army_leaders": 1, "navy_leaders": 0,
                           "characters": 1}}
            # Stage 1.5 pinned API: enumerate_addrs -> located sets; enumerate_from_addrs ->
            # the decode; validate_addrs -> fail-closed gate.
            def enumerate_addrs(self):
                return {"army": [0xCAFE], "navy": [], "char": [1], "country_leader": [],
                        "_ids": {0xCAFE: 1, 1: 1}}
            def enumerate_from_addrs(self, addr_sets, resolve_country=True, read_traits=True):
                return {k: (v if not isinstance(v, dict) else dict(v))
                        for k, v in self._OUT.items()}
            def validate_addrs(self, addr_sets):
                return True

        class _CmdImpl:
            def __call__(self):
                return _FakeCmdReader()
        out_c = plugs["commanders_live"]._reader(ctx, _CmdImpl())
        blob_c = repr(out_c)
        self.assertNotIn("addr", blob_c)
        self.assertNotIn("0xCAFE", blob_c)
        self.assertNotIn("_unpinned", blob_c)

    def test_intel_live_rejects_non_tag_keys(self):
        # intel_live.read_all can fall back to localized country NAMES when its bridge
        # fails; those are not TAGs and must be dropped (never create a non-TAG country).
        plugs = {p.name: p for p in reg.default_registry()}

        class _FakeProc:
            pass
        ctx = reg.CaptureContext(proc=_FakeProc())

        class _FakeIntel:
            @staticmethod
            def read_all(p, bridge=None):
                return {"GER": {"name": "Abwehr", "defense": 1.0,
                                "icon": None, "upgrade_progress": 0.0,
                                "max_operative_count": 1, "usable_operative_slots": 1},
                        "Germany": {"name": "loc-name-fallback", "defense": 2.0}}
        out = plugs["intel_live"]._reader(ctx, _FakeIntel)
        self.assertIn("GER", out["countries"])
        self.assertNotIn("Germany", out["countries"])

    def test_intel_live_bridge_known_tag_crosscheck(self):
        # Codex final review: a 3-letter localized name (e.g. "MI6") would pass
        # schema.valid_tag. With a bridge available, only keys the bridge KNOWS are
        # accepted, closing the no-fabrication hole.
        plugs = {p.name: p for p in reg.default_registry()}

        class _FakeProc:
            pass

        class _FakeBridge:
            # bridge knows GER (db-id 1) only; resolve maps 1 -> GER.
            def as_map(self):
                return {1: 0}

        def _resolve_tag(self, tdid):
            return "GER" if tdid == 1 else None
        # bind a custom resolve_tag onto the ctx
        ctx = reg.CaptureContext(proc=_FakeProc(), bridge=_FakeBridge())
        ctx.resolve_tag = lambda tdid: "GER" if tdid == 1 else None

        class _FakeIntel:
            @staticmethod
            def read_all(p, bridge=None):
                return {"GER": {"name": "Abwehr", "defense": 1.0, "icon": None,
                                "upgrade_progress": 0.0, "max_operative_count": 1,
                                "usable_operative_slots": 1},
                        # 3-letter, passes valid_tag, but bridge does NOT know it
                        "MI6": {"name": "fake", "defense": 9.0, "icon": None,
                                "upgrade_progress": 0.0, "max_operative_count": 9,
                                "usable_operative_slots": 9}}
        out = plugs["intel_live"]._reader(ctx, _FakeIntel)
        self.assertIn("GER", out["countries"])
        self.assertNotIn("MI6", out["countries"])  # bridge-unknown 3-letter -> dropped

    def test_save_backstop_cannot_overwrite_live_measured_leaves(self):
        # The save backstop (_merge_country) must re-assert every live-measured leaf
        # for these 3 domains AFTER the merge -- and preserve save-only sub-keys.
        prot = []
        for dom in ("tech_live", "commanders_live", "intel_live"):
            prot += list(asm._LIVE_COUNTRY_DOMAIN_LEAVES[dom])
        live_c = schema.empty_country()
        live_c["research"]["tech_count"] = 120
        live_c["research"]["research_slots"] = 4
        live_c["research"]["in_progress_live"] = [{"slot_index": 0}]
        live_c["commanders"]["army_leaders"] = [{"char_id": 1}]
        live_c["commanders"]["navy_leaders"] = [{"char_id": 2}]
        live_c["commanders"]["counts"] = {"army_leaders": 1, "navy_leaders": 1,
                                          "characters": 3}
        live_c["intel"]["agency"] = {"name": "Abwehr", "upgrade_progress": 24.0,
                                     "capacity": {"max_operatives": 5,
                                                  "usable_slots": 5, "defense": 6.6}}
        save_c = schema.empty_country()
        save_c["research"]["tech_count"] = 999           # stale -> live must win
        save_c["research"]["research_slots"] = 99
        save_c["research"]["in_progress"] = [{"name": "infantry_weapons"}]  # save-only
        save_c["commanders"]["generals"] = [{"char_id": 1}]                 # save-only
        save_c["commanders"]["admirals"] = [{"char_id": 2}]
        save_c["commanders"]["counts"] = {"generals": 1, "advisors": 5,
                                          "roster_size": 3, "army_leaders": 77}
        save_c["intel"]["agency"] = {"name": "STALE", "is_created": True,
                                     "upgrades": {"b": 1}, "upgrade_progress": 1.0,
                                     "capacity": {"max_operatives": 1,
                                                  "usable_slots": 1, "defense": 1.0,
                                                  "extra_save_key": 42}}
        merged = asm._merge_country(live_c, save_c, prot)
        # live WINS on every measured leaf
        self.assertEqual(merged["research"]["tech_count"], 120)
        self.assertEqual(merged["research"]["research_slots"], 4)
        self.assertEqual(merged["commanders"]["counts"]["army_leaders"], 1)
        self.assertEqual(merged["intel"]["agency"]["name"], "Abwehr")
        self.assertEqual(merged["intel"]["agency"]["capacity"]["max_operatives"], 5)
        # save-only sub-keys preserved (live did not measure them)
        self.assertEqual(merged["research"]["in_progress"], [{"name": "infantry_weapons"}])
        self.assertEqual(merged["commanders"]["generals"], [{"char_id": 1}])
        self.assertEqual(merged["commanders"]["counts"]["advisors"], 5)
        self.assertEqual(merged["commanders"]["counts"]["roster_size"], 3)
        self.assertTrue(merged["intel"]["agency"]["is_created"])
        self.assertEqual(merged["intel"]["agency"]["upgrades"], {"b": 1})
        self.assertEqual(merged["intel"]["agency"]["capacity"]["extra_save_key"], 42)

    def test_absent_domains_stay_null_and_unclaimed(self):
        snap = asm.assemble_snapshot({}, game_id="g", date="1940.9.10.13",
                                     identity=FakeIdentity())
        self.assertEqual(schema.validate_snapshot(snap), [])
        doms = snap["provenance"]["memory"]["domains"]
        for d in ("tech_live", "commanders_live", "intel_live"):
            self.assertNotIn(d, doms)

    def test_failed_domain_omitted_and_block_null(self):
        res = {"tech_live": self._res("tech_live", None, captured=False,
                                      error="boom")}
        snap = asm.assemble_snapshot(res, game_id="g", date="1940.9.10.13",
                                     identity=FakeIdentity())
        self.assertNotIn("tech_live", snap["provenance"]["memory"]["domains"])
        # no country shell fabricated, blocks stay null skeleton
        self.assertEqual(snap["countries"], {})

    def test_tech_reader_drops_non_tag_keys(self):
        # the registry reader keeps ONLY schema-valid TAG keys (e.g. a numeric-string
        # key from a misbehaving bridge is dropped, never written as a country).
        plugs = {p.name: p for p in reg.default_registry()}

        class _FakeProc:
            pass
        ctx = reg.CaptureContext(proc=_FakeProc())
        def _fake_tech(p, bridge=None, strict=True, **kw):
            return {"GER": {"completed_tech_count": 1, "research_slot_count": 1,
                            "active_research": [], "_status_addr": 1},
                    "42": {"completed_tech_count": 9, "research_slot_count": 9,
                           "active_research": []}}
        out = plugs["tech_live"]._reader(ctx, _wrap_tech(_fake_tech))
        self.assertIn("GER", out["countries"])
        self.assertNotIn("42", out["countries"])

    def test_tech_reader_preserves_unmeasured_active_research_none(self):
        # Codex final review: a missing/None active_research is UNMEASURED and must
        # stay None (NOT coerced to []), so the assembler leaves in_progress_live null
        # and the save can fill it. A measured [] stays [] (measured-empty).
        plugs = {p.name: p for p in reg.default_registry()}

        class _FakeProc:
            pass
        ctx = reg.CaptureContext(proc=_FakeProc())
        def _fake_tech(p, bridge=None, strict=True, **kw):
            return {"GER": {"completed_tech_count": 5, "research_slot_count": 3},
                    # active_research missing entirely -> None (unmeasured)
                    "SOV": {"completed_tech_count": 1, "research_slot_count": 1,
                            "active_research": []}}  # measured-empty
        out = plugs["tech_live"]._reader(ctx, _wrap_tech(_fake_tech))
        self.assertIsNone(out["countries"]["GER"]["active_research"])  # unmeasured
        self.assertEqual(out["countries"]["SOV"]["active_research"], [])  # measured 0
        # fold it: GER in_progress_live stays null; SOV is measured-empty []
        snap = asm.assemble_snapshot(
            {"tech_live": reg.DomainResult("tech_live", True, True, out)},
            game_id="g", date=None, identity=FakeIdentity())
        self.assertIsNone(snap["countries"]["GER"]["research"]["in_progress_live"])
        self.assertEqual(snap["countries"]["SOV"]["research"]["in_progress_live"], [])

    def test_offline_readers_raise(self):
        # ctx.proc None -> tech_live / intel_live raise (domain honestly omitted).
        plugs = {p.name: p for p in reg.default_registry()}
        ctx = reg.CaptureContext(proc=None)
        with self.assertRaises(Exception):
            plugs["tech_live"]._reader(ctx, _wrap_tech(lambda p, bridge=None, strict=True: {}))
        with self.assertRaises(Exception):
            plugs["intel_live"]._reader(ctx, object())


class TestFocusProductionLiveDomains(unittest.TestCase):
    """MOCK-based coverage for the 2 LIVE per-country domains wired this pass:
      * focus_live  -> the SAME canonical save home (country.focus) under a DISTINCT
        provenance id; current_continuous is a NEW live-only leaf; progress folds onto
        the save-shared focus.progress (same days unit); paused stays RESISTED (None).
      * production_live -> a NEW live-only home (country.production), distinct from the
        save "production" domain (which folds into economy/military).
    NO live memory -- every reader output is a canned DomainResult. Asserts the fields
    land where Codex blessed them, measured 0 / measured-empty survive, the save-shared
    focus.paused is not fabricated, the save backstop cannot overwrite a live-measured
    leaf (incl. a measured-empty completed []), diagnostic addrs never leak, and a
    bridge-name fallback (no TAG resolved) is honestly omitted."""

    def _res(self, name, data, *, available=True, captured=True, error=None):
        return reg.DomainResult(name, available=available, captured=captured,
                                data=data, error=error)

    def _full(self):
        return {
            "focus_live": self._res("focus_live", {"countries": {
                "GER": {"focus_tree": "german_focus", "completed": ["a", "b"],
                        "completed_count": 2, "current": "ger_x",
                        "current_continuous": None, "progress_days": 12.5,
                        "paused": None},
                # measured zeros: 0 completed (measured-empty list), no active national
                # focus (continuous active instead), progress 0.0 (just started)
                "SOV": {"focus_tree": "soviet_focus", "completed": [],
                        "completed_count": 0, "current": None,
                        "current_continuous": "sov_cont", "progress_days": 0.0,
                        "paused": None},
            }, "count": 2}),
            "production_live": self._res("production_live", {"countries": {
                "GER": {"stockpile": {"70": 123.5, "71": 0.0},
                        "stockpile_total": 123.5,
                        "military_lines": [{"id": 349, "active_factories": 3,
                                            "cost": 105.0, "speed": 31.05,
                                            "produced": 34.4, "amount": -1}],
                        "naval_lines": [], "military_factories": 3,
                        "naval_factories": 0},
                # measured-empty world: 0 of everything
                "SOV": {"stockpile": {}, "stockpile_total": 0.0,
                        "military_lines": [], "naval_lines": [],
                        "military_factories": 0, "naval_factories": 0},
            }, "count": 2}),
        }

    def test_all_two_domains_fold_and_validate(self):
        snap = asm.assemble_snapshot(self._full(), game_id="g",
                                     date="1940.9.10.13", identity=FakeIdentity())
        self.assertEqual(schema.validate_snapshot(snap), [])
        doms = snap["provenance"]["memory"]["domains"]
        self.assertIn("focus_live", doms)
        self.assertIn("production_live", doms)

    def test_focus_live_folds_onto_save_home_with_continuous_leaf(self):
        snap = asm.assemble_snapshot(self._full(), game_id="g",
                                     date="1940.9.10.13", identity=FakeIdentity())
        ger = snap["countries"]["GER"]["focus"]
        sov = snap["countries"]["SOV"]["focus"]
        self.assertEqual(ger["focus_tree"], "german_focus")
        self.assertEqual(ger["completed"], ["a", "b"])
        self.assertEqual(ger["completed_count"], 2)
        self.assertEqual(ger["current"], "ger_x")
        # progress_days folds onto the save-shared focus.progress (same days unit)
        self.assertEqual(ger["progress"], 12.5)
        # paused is RESISTED -> never written by the live fold (stays skeleton None)
        self.assertIsNone(ger["paused"])
        # measured zeros survive: 0 completed (measured-empty list), continuous active
        self.assertEqual(sov["completed"], [])
        self.assertEqual(sov["completed_count"], 0)
        self.assertEqual(sov["current_continuous"], "sov_cont")
        self.assertEqual(sov["progress"], 0.0)   # measured 0.0 days survives
        self.assertIsNone(sov["current"])        # no active national focus this frame

    def test_production_live_folds_to_new_country_home(self):
        snap = asm.assemble_snapshot(self._full(), game_id="g",
                                     date="1940.9.10.13", identity=FakeIdentity())
        ger = snap["countries"]["GER"]["production"]
        sov = snap["countries"]["SOV"]["production"]
        self.assertEqual(ger["stockpile"], {"70": 123.5, "71": 0.0})
        self.assertEqual(ger["stockpile_total"], 123.5)
        self.assertEqual(ger["military_lines"][0]["id"], 349)
        self.assertEqual(ger["military_factories"], 3)
        self.assertEqual(ger["naval_factories"], 0)   # measured 0 survives
        # measured-empty world is preserved as measured zeros, not nulls
        self.assertEqual(sov["stockpile"], {})
        self.assertEqual(sov["stockpile_total"], 0.0)
        self.assertEqual(sov["military_lines"], [])
        self.assertEqual(sov["military_factories"], 0)
        # the save-side production homes are NOT polluted by the live fold (live has its
        # own dedicated country.production home).
        self.assertEqual(snap["countries"]["GER"]["military"]["equipment_stockpile"], [])

    def test_save_backstop_cannot_overwrite_live_measured_leaves(self):
        # _merge_country must re-assert every live-measured leaf AFTER the merge, incl. a
        # measured-empty completed [] (beats a stale save list) and a measured 0; and an
        # UNMEASURED live None leaf must let the save fill it (focus.current here).
        prot = (list(asm._LIVE_COUNTRY_DOMAIN_LEAVES["focus_live"])
                + list(asm._LIVE_COUNTRY_DOMAIN_LEAVES["production_live"]))
        live_c = schema.empty_country()
        live_c["focus"]["focus_tree"] = "soviet_focus"
        live_c["focus"]["completed"] = []            # measured-empty -> must WIN
        live_c["focus"]["completed_count"] = 0
        live_c["focus"]["current"] = None            # UNMEASURED -> save must fill
        live_c["focus"]["current_continuous"] = "sov_cont"
        live_c["focus"]["progress"] = 3.0
        live_c["production"]["stockpile"] = {}        # measured-empty -> must WIN
        live_c["production"]["military_factories"] = 0
        save_c = schema.empty_country()
        save_c["focus"]["focus_tree"] = "STALE_TREE"
        save_c["focus"]["completed"] = ["x", "y", "z"]    # stale -> live [] wins
        save_c["focus"]["completed_count"] = 3
        save_c["focus"]["current"] = "save_current"        # live unmeasured -> save fills
        save_c["focus"]["continuous_focus_palette"] = "pal"  # save-only -> preserved
        save_c["focus"]["paused"] = True                    # save-only (RESISTED) -> kept
        save_c["production"]["stockpile"] = {"99": 5.0}     # live {} must win
        # the real fold path always records the measured-TAG set for focus.completed (the
        # ONLY protected save-shared leaf whose skeleton default is a non-None []); supply
        # it here so the measured-empty [] for this measured TAG is protected (fail-closed
        # without it, matching the cross-domain masquerade guard).
        merged = asm._merge_country(live_c, save_c, prot, tag="SOV",
                                    measured_leaf_tags={"focus.completed": {"SOV"}})
        f = merged["focus"]; p = merged["production"]
        self.assertEqual(f["focus_tree"], "soviet_focus")
        self.assertEqual(f["completed"], [])               # measured-empty won
        self.assertEqual(f["completed_count"], 0)
        self.assertEqual(f["current"], "save_current")     # unmeasured -> save filled
        self.assertEqual(f["current_continuous"], "sov_cont")
        self.assertEqual(f["continuous_focus_palette"], "pal")  # save-only preserved
        self.assertTrue(f["paused"])                       # save-only RESISTED preserved
        self.assertEqual(p["stockpile"], {})               # live measured-empty won
        self.assertEqual(p["military_factories"], 0)

    def test_focus_completed_cross_domain_masquerade_guard(self):
        # CROSS-DOMAIN no-fabrication: country.focus.completed is the ONLY protected
        # save-shared leaf whose skeleton default is a non-None [] . A TAG created by
        # ANOTHER live domain (here country_scalars) but NOT measured by focus_live
        # carries the skeleton [] -- which must NOT overwrite the save's real completed
        # list during the backstop merge. A TAG focus_live DID measure (genuine [] or a
        # real list) still wins.
        res = {
            "focus_live": reg.DomainResult("focus_live", True, True, {"countries": {
                "GER": {"focus_tree": "german_focus", "completed": ["a"],
                        "completed_count": 1, "current": None,
                        "current_continuous": None, "progress_days": None,
                        "paused": None},
            }, "count": 1}),
            "country_scalars": reg.DomainResult("country_scalars", True, True,
                                                {"countries": {"SOV": {"stability": 0.5}},
                                                 "count": 1}),
        }
        snap = asm.assemble_snapshot(res, game_id="g", date=None,
                                     identity=FakeIdentity())
        # SOV exists (country_scalars) with the skeleton focus.completed []
        self.assertEqual(snap["countries"]["SOV"]["focus"]["completed"], [])
        # the assembler recorded ONLY GER as a focus.completed-measured TAG
        self.assertEqual(snap["_live_measured_leaf_tags"]["focus.completed"], ["GER"])
        self.assertEqual(schema.validate_snapshot(snap), [])  # sidecar ignored by validator

        # mirror fold_save_bundle's per-country merge with the measured-tag guard
        prot = []
        for dom in snap["provenance"]["memory"]["domains"]:
            prot.extend(asm._LIVE_COUNTRY_DOMAIN_LEAVES.get(dom, ()))
        measured = {p: set(t) for p, t in snap["_live_measured_leaf_tags"].items()}
        save_ger = schema.empty_country(); save_ger["focus"]["completed"] = ["STALE"]
        save_sov = schema.empty_country()
        save_sov["focus"]["completed"] = ["sov_real1", "sov_real2"]
        m_ger = asm._merge_country(snap["countries"]["GER"], save_ger, prot,
                                   tag="GER", measured_leaf_tags=measured)
        m_sov = asm._merge_country(snap["countries"]["SOV"], save_sov, prot,
                                   tag="SOV", measured_leaf_tags=measured)
        # GER measured -> live wins; SOV un-measured by focus_live -> save preserved
        self.assertEqual(m_ger["focus"]["completed"], ["a"])
        self.assertEqual(m_sov["focus"]["completed"], ["sov_real1", "sov_real2"])

    def test_focus_completed_measured_empty_for_measured_tag_still_wins(self):
        # a GENUINE measured-empty [] on a focus_live-MEASURED tag must still beat the save.
        measured = {"focus.completed": {"GER"}}
        live = schema.empty_country(); live["focus"]["completed"] = []
        save = schema.empty_country(); save["focus"]["completed"] = ["stale"]
        m = asm._merge_country(live, save, ["focus.completed"], tag="GER",
                               measured_leaf_tags=measured)
        self.assertEqual(m["focus"]["completed"], [])

    def test_merge_country_legacy_three_arg_call_unchanged(self):
        # the 3-arg call (no tag / measured map) preserves the original value-guard for a
        # None-defaulting protected leaf, and an INFORMATIVE (non-empty) focus.completed
        # still wins via _backstop_merge (it is non-empty -> live beats save regardless of
        # the re-assertion). A None-defaulting leaf (research.tech_count) is re-asserted.
        live = schema.empty_country()
        live["focus"]["completed"] = ["x"]            # informative -> wins via merge
        live["research"]["tech_count"] = 7            # None-default leaf -> re-asserted
        save = schema.empty_country()
        save["focus"]["completed"] = ["old"]
        save["research"]["tech_count"] = 999
        m = asm._merge_country(live, save,
                               ["focus.completed", "research.tech_count"])
        self.assertEqual(m["focus"]["completed"], ["x"])
        self.assertEqual(m["research"]["tech_count"], 7)

    def test_merge_country_legacy_measured_empty_completed_fails_closed(self):
        # WITHOUT the measured-TAG map, a measured-empty focus.completed [] CANNOT be
        # proven measured (it is indistinguishable from the skeleton []) -> fail-closed:
        # the save value is kept rather than risk a fabricated zero. The real fold path
        # always supplies the map, so a genuinely-measured [] is still protected there.
        live = schema.empty_country(); live["focus"]["completed"] = []
        save = schema.empty_country(); save["focus"]["completed"] = ["s1", "s2"]
        m = asm._merge_country(live, save, ["focus.completed"])  # no map -> fail closed
        self.assertEqual(m["focus"]["completed"], ["s1", "s2"])

    def test_focus_live_strips_submgr_addr(self):
        # the registry reader must strip the diagnostic _submgr_addr so the heap address
        # never reaches the snapshot. Drive the reader fn directly with a fake impl.
        plugs = {p.name: p for p in reg.default_registry()}

        class _FakeProc:
            pass
        ctx = reg.CaptureContext(proc=_FakeProc())

        def _fake_focus(p, bridge=None, include_candidate=False, strict=True, **kw):
            return {"GER": {"focus_tree": "german_focus", "completed": [],
                            "completed_count": 0, "current": None,
                            "current_continuous": None, "progress_days": None,
                            "paused": None, "_submgr_addr": 0xDEAD,
                            "available_focuses_candidate": ["junk"]}}
        out = plugs["focus_live"]._reader(ctx, _fake_focus)
        blob = repr(out)
        self.assertNotIn("_submgr_addr", blob)
        self.assertNotIn("0xDEAD", blob)
        self.assertNotIn("available_focuses_candidate", blob)
        self.assertIn("GER", out["countries"])

    def test_focus_reader_drops_country_with_non_list_completed(self):
        # FAIL-CLOSED: country.focus.completed is a protected, save-shared leaf whose
        # skeleton default is [] -- so an unmeasured completed (None) would masquerade as
        # a measured-empty [] and the save backstop would protect it (fabricating "zero
        # completed"). The reader DROPS any country whose completed is not a list, so the
        # protected leaf is a measured list IFF the country is present.
        plugs = {p.name: p for p in reg.default_registry()}

        class _FakeProc:
            pass
        ctx = reg.CaptureContext(proc=_FakeProc())

        def _fake_focus(p, bridge=None, include_candidate=False, strict=True, **kw):
            return {"GER": {"focus_tree": "german_focus", "completed": ["a"],
                            "completed_count": 1, "current": None,
                            "current_continuous": None, "progress_days": None,
                            "paused": None},
                    # malformed: completed is None (would be the skeleton [] hole)
                    "SOV": {"focus_tree": "soviet_focus", "completed": None,
                            "completed_count": None, "current": None,
                            "current_continuous": None, "progress_days": None,
                            "paused": None}}
        out = plugs["focus_live"]._reader(ctx, _fake_focus)
        self.assertIn("GER", out["countries"])
        self.assertNotIn("SOV", out["countries"])   # dropped (completed not a list)

    def test_focus_reader_drops_non_tag_keys(self):
        plugs = {p.name: p for p in reg.default_registry()}

        class _FakeProc:
            pass
        ctx = reg.CaptureContext(proc=_FakeProc())

        def _fake_focus(p, bridge=None, include_candidate=False, strict=True, **kw):
            return {"GER": {"focus_tree": "german_focus", "completed": [],
                            "completed_count": 0, "current": None,
                            "current_continuous": None, "progress_days": None,
                            "paused": None},
                    "42": {"focus_tree": "x", "completed": [], "completed_count": 0,
                           "current": None, "current_continuous": None,
                           "progress_days": None, "paused": None}}
        out = plugs["focus_live"]._reader(ctx, _fake_focus)
        self.assertIn("GER", out["countries"])
        self.assertNotIn("42", out["countries"])

    def test_production_reader_drops_non_tag_keys(self):
        plugs = {p.name: p for p in reg.default_registry()}

        class _FakeProc:
            pass
        ctx = reg.CaptureContext(proc=_FakeProc())

        def _fake_prod(p, bridge=None):
            return {"GER": {"stockpile": {70: 1.0}, "stockpile_total": 1.0,
                            "military_lines": [], "naval_lines": [],
                            "military_factories": 0, "naval_factories": 0},
                    # a localized country NAME key (bridge resolved SOME tags) -> dropped
                    "Germany": {"stockpile": {}, "stockpile_total": 0.0,
                                "military_lines": [], "naval_lines": [],
                                "military_factories": 0, "naval_factories": 0}}
        out = plugs["production_live"]._reader(ctx, _fake_prod)
        self.assertIn("GER", out["countries"])
        self.assertNotIn("Germany", out["countries"])
        # stockpile keys are stringified for JSON (the values are not fabricated)
        self.assertEqual(out["countries"]["GER"]["stockpile"], {"70": 1.0})

    def test_production_reader_raises_when_bridge_falls_back_to_names(self):
        # if the reader returned countries but NONE resolved to a TAG (the bridge fell
        # back to localized names), the read is broken -> raise so the domain is honestly
        # omitted rather than claimed captured-empty.
        plugs = {p.name: p for p in reg.default_registry()}

        class _FakeProc:
            pass
        ctx = reg.CaptureContext(proc=_FakeProc())

        def _fake_prod(p, bridge=None):
            return {"Germany": {"stockpile": {}, "stockpile_total": 0.0,
                                "military_lines": [], "naval_lines": [],
                                "military_factories": 0, "naval_factories": 0},
                    "Soviet Union": {"stockpile": {}, "stockpile_total": 0.0,
                                     "military_lines": [], "naval_lines": [],
                                     "military_factories": 0, "naval_factories": 0}}
        with self.assertRaises(Exception):
            plugs["production_live"]._reader(ctx, _fake_prod)

    def test_offline_readers_raise(self):
        # ctx.proc None -> focus_live / production_live raise (domain honestly omitted).
        plugs = {p.name: p for p in reg.default_registry()}
        ctx = reg.CaptureContext(proc=None)
        with self.assertRaises(Exception):
            plugs["focus_live"]._reader(
                ctx, lambda p, bridge=None, include_candidate=False, strict=True: {})
        with self.assertRaises(Exception):
            plugs["production_live"]._reader(ctx, lambda p, bridge=None: {})

    def test_absent_domains_stay_null_and_unclaimed(self):
        snap = asm.assemble_snapshot({}, game_id="g", date="1940.9.10.13",
                                     identity=FakeIdentity())
        self.assertEqual(schema.validate_snapshot(snap), [])
        doms = snap["provenance"]["memory"]["domains"]
        self.assertNotIn("focus_live", doms)
        self.assertNotIn("production_live", doms)

    def test_failed_domain_omitted_and_block_null(self):
        res = {"production_live": self._res("production_live", None, captured=False,
                                            error="boom")}
        snap = asm.assemble_snapshot(res, game_id="g", date="1940.9.10.13",
                                     identity=FakeIdentity())
        self.assertNotIn("production_live",
                         snap["provenance"]["memory"]["domains"])
        self.assertEqual(snap["countries"], {})


class TestDiplomacyPoliticsLiveDomains(unittest.TestCase):
    """MOCK-based coverage for the 2 LIVE per-country domains wired this pass:
      * diplomacy_live -> country.diplomacy.{at_war, casualties, faction_group} (M) +
        the NEW top-level sidecar snap["diplomacy_live"] {war_relations, faction_members,
        subtype_warning}. The world data is NOT folded into the SAVE-shaped diplomacy_world
        (a different identity model). at_war/ideas default None (NOT []), so the simple
        None-guard avoids the cross-domain skeleton-[] masquerade (no _SKELETON_EMPTY_
        CONTAINER_LEAVES needed).
      * politics_live -> country.politics (party_popularities/ruling_party folded onto the
        save-shared leaves; ideas/idea_count/election_frequency/party_count/low_confidence
        live leaves). political_power is RESISTED here (country_scalars owns the protected
        leaf -- the PP-overlap reconciliation).
    NO live memory -- every reader output is a canned DomainResult. Asserts the fields land
    where Codex blessed them, measured 0 / measured-empty survive, every TAG is valid_tag-
    checked, political_power is NOT double-written, the save backstop cannot overwrite a
    live-measured leaf, diagnostic addrs never leak, and validate_snapshot still passes."""

    def _res(self, name, data, *, available=True, captured=True, error=None):
        return reg.DomainResult(name, available=available, captured=captured,
                                data=data, error=error)

    def _full(self):
        return {
            "diplomacy_live": self._res("diplomacy_live", {
                "countries": {
                    "GER": {"at_war": ["SOV", "ENG"], "casualties": 300,
                            "faction_group": 7},
                    # measured zeros: at peace (measured-empty []), 0 casualties
                    "SOV": {"at_war": [], "casualties": 0, "faction_group": 9},
                },
                "war_relations": [
                    {"first": "GER", "second": "SOV", "first_casualties": 100,
                     "second_casualties": 200}],
                "faction_members": {"7": ["GER", "ITA"], "9": ["SOV"]},
                "subtype_warning": {"0xdead": 2},
                "count": 2,
            }),
            "politics_live": self._res("politics_live", {
                "countries": {
                    "GER": {"popularities": {"democratic": 10.0, "communism": 5.0,
                                             "fascism": 80.0, "neutrality": 5.0},
                            "ruling_party": "fascism", "ideas": ["a", "b"],
                            "idea_count": 2, "election_frequency": 48,
                            "party_count": 4, "low_confidence": False},
                    # measured zeros: 0 ideas (measured-empty), no elections (freq None)
                    "SOV": {"popularities": {"democratic": 0.0, "communism": 100.0,
                                             "fascism": 0.0, "neutrality": 0.0},
                            "ruling_party": "communism", "ideas": [],
                            "idea_count": 0, "election_frequency": None,
                            "party_count": 4, "low_confidence": True},
                },
                "count": 2,
            }),
        }

    def test_all_two_domains_fold_and_validate(self):
        snap = asm.assemble_snapshot(self._full(), game_id="g",
                                     date="1940.9.10.13", identity=FakeIdentity())
        self.assertEqual(schema.validate_snapshot(snap), [])
        doms = snap["provenance"]["memory"]["domains"]
        self.assertIn("diplomacy_live", doms)
        self.assertIn("politics_live", doms)

    def test_diplomacy_live_per_country_and_world_split(self):
        snap = asm.assemble_snapshot(self._full(), game_id="g",
                                     date="1940.9.10.13", identity=FakeIdentity())
        ger = snap["countries"]["GER"]["diplomacy"]
        sov = snap["countries"]["SOV"]["diplomacy"]
        self.assertEqual(ger["at_war"], ["SOV", "ENG"])
        self.assertEqual(ger["casualties"], 300)
        self.assertEqual(ger["faction_group"], 7)
        # measured zeros survive: at peace (measured-empty []), 0 casualties
        self.assertEqual(sov["at_war"], [])
        self.assertEqual(sov["casualties"], 0)
        self.assertEqual(sov["faction_group"], 9)
        # the save-derived diplomacy.wars leaf is NOT touched by the live fold (distinct leaf)
        self.assertEqual(ger["wars"], [])
        # world data lands in the NEW sidecar, NOT diplomacy_world (different identity model)
        dl = snap["diplomacy_live"]
        self.assertEqual(dl["source"], "M")
        self.assertEqual(dl["war_relations"][0]["first"], "GER")
        self.assertEqual(dl["faction_members"], {"7": ["GER", "ITA"], "9": ["SOV"]})
        self.assertEqual(dl["subtype_warning"], {"0xdead": 2})
        # diplomacy_world stays the empty save-shaped skeleton (not polluted)
        self.assertEqual(snap["diplomacy_world"],
                         {"wars": {}, "factions": {}, "subjects": []})

    def test_politics_live_folds_onto_save_home_pp_resisted(self):
        snap = asm.assemble_snapshot(self._full(), game_id="g",
                                     date="1940.9.10.13", identity=FakeIdentity())
        ger = snap["countries"]["GER"]["politics"]
        sov = snap["countries"]["SOV"]["politics"]
        self.assertEqual(ger["party_popularities"],
                         {"democratic": 10.0, "communism": 5.0,
                          "fascism": 80.0, "neutrality": 5.0})
        self.assertEqual(ger["ruling_party"], "fascism")
        self.assertEqual(ger["ideas"], ["a", "b"])
        self.assertEqual(ger["idea_count"], 2)
        self.assertEqual(ger["election_frequency"], 48)
        self.assertEqual(ger["party_count"], 4)
        self.assertFalse(ger["low_confidence"])
        # PP-OVERLAP: politics_live does NOT write political_power (country_scalars owns it).
        # country_scalars did not run in this fixture -> PP stays the skeleton null.
        self.assertIsNone(ger["political_power"])
        # measured zeros survive: 0 ideas (measured-empty []), low_confidence True, no freq
        self.assertEqual(sov["ideas"], [])
        self.assertEqual(sov["idea_count"], 0)
        self.assertTrue(sov["low_confidence"])
        self.assertIsNone(sov["election_frequency"])
        # the save-side ideas_all / ideas_other leaves are NOT polluted by the live fold
        self.assertEqual(ger["ideas_all"], [])
        self.assertEqual(ger["ideas_other"], [])

    def test_pp_reconciliation_country_scalars_still_owns_pp(self):
        # When BOTH country_scalars AND politics_live run, country_scalars writes PP and
        # politics_live writes the rest -- the single PP owner stays country_scalars.
        res = self._full()
        res["country_scalars"] = self._res("country_scalars", {"countries": {
            "GER": {"political_power": 120.5, "stability": 0.6},
        }, "count": 1})
        snap = asm.assemble_snapshot(res, game_id="g", date="1940.9.10.13",
                                     identity=FakeIdentity())
        ger = snap["countries"]["GER"]["politics"]
        # country_scalars set PP; politics_live set ruling/ideas/etc. (no PP clobber)
        self.assertEqual(ger["political_power"], 120.5)
        self.assertEqual(ger["ruling_party"], "fascism")
        self.assertEqual(ger["ideas"], ["a", "b"])

    def test_save_backstop_cannot_overwrite_live_measured_leaves(self):
        # _merge_country must re-assert every live-measured leaf AFTER the merge, incl. a
        # measured-empty at_war [] (at peace) + a measured 0 casualties + a measured-empty
        # ideas []; save-only sibling leaves (diplomacy.wars/opinion, politics.ideas_all,
        # laws) are preserved; PP (live politics_live did NOT write it) is filled by the save.
        prot = (list(asm._LIVE_COUNTRY_DOMAIN_LEAVES["diplomacy_live"])
                + list(asm._LIVE_COUNTRY_DOMAIN_LEAVES["politics_live"]))
        live_c = schema.empty_country()
        live_c["diplomacy"]["at_war"] = []        # measured-empty (at peace) -> must WIN
        live_c["diplomacy"]["casualties"] = 0     # measured zero -> must WIN
        live_c["diplomacy"]["faction_group"] = 9
        live_c["politics"]["ruling_party"] = "communism"
        live_c["politics"]["party_popularities"] = {"communism": 100.0}
        live_c["politics"]["ideas"] = []          # measured-empty -> must WIN
        live_c["politics"]["idea_count"] = 0
        # political_power left None (politics_live does not measure it -> save must fill)
        save_c = schema.empty_country()
        save_c["diplomacy"]["at_war"] = ["GER", "ENG"]    # stale -> live [] wins
        save_c["diplomacy"]["casualties"] = 999
        save_c["diplomacy"]["wars"] = [{"against": "SOV"}]  # save-only -> preserved
        save_c["diplomacy"]["opinion"] = {"SOV": -50}        # save-only -> preserved
        save_c["politics"]["ruling_party"] = "STALE"
        save_c["politics"]["party_popularities"] = {"fascism": 99.0}
        save_c["politics"]["ideas"] = ["sov_idea"]           # stale -> live [] wins
        save_c["politics"]["ideas_all"] = ["save_idea"]      # save-only sibling -> preserved
        save_c["politics"]["political_power"] = 55.0          # live unmeasured -> save fills
        save_c["politics"]["laws"] = {"economy": "war_economy"}  # save-only -> preserved
        merged = asm._merge_country(live_c, save_c, prot, tag="GER",
                                    measured_leaf_tags={})
        d = merged["diplomacy"]; p = merged["politics"]
        self.assertEqual(d["at_war"], [])              # measured-empty won
        self.assertEqual(d["casualties"], 0)           # measured zero won
        self.assertEqual(d["faction_group"], 9)
        self.assertEqual(d["wars"], [{"against": "SOV"}])    # save-only preserved
        self.assertEqual(d["opinion"], {"SOV": -50})          # save-only preserved
        self.assertEqual(p["ruling_party"], "communism")     # live won
        self.assertEqual(p["party_popularities"], {"communism": 100.0})
        self.assertEqual(p["ideas"], [])               # live measured-empty won
        self.assertEqual(p["ideas_all"], ["save_idea"])      # save-only sibling preserved
        self.assertEqual(p["political_power"], 55.0)         # live unmeasured -> save filled
        self.assertEqual(p["laws"], {"economy": "war_economy"})  # save-only preserved

    def test_unmeasured_country_keeps_none_no_skeleton_masquerade(self):
        # CROSS-DOMAIN no-fabrication: at_war + ideas default None (NOT []), so a TAG created
        # by ANOTHER live domain (country_scalars) but NOT measured by diplomacy_live /
        # politics_live carries None there -- the `lv is not None` guard in _merge_country
        # skips it, so a real save at_war / ideas list is preserved (no skeleton-[] overwrite).
        res = {
            "diplomacy_live": reg.DomainResult("diplomacy_live", True, True, {
                "countries": {"GER": {"at_war": ["SOV"], "casualties": 5,
                                      "faction_group": 7}},
                "war_relations": [], "faction_members": {}, "subtype_warning": {},
                "count": 1}),
            "country_scalars": reg.DomainResult("country_scalars", True, True,
                                                {"countries": {"SOV": {"stability": 0.5}},
                                                 "count": 1}),
        }
        snap = asm.assemble_snapshot(res, game_id="g", date=None,
                                     identity=FakeIdentity())
        # SOV exists (country_scalars) with the skeleton at_war / ideas None (NOT [])
        self.assertIsNone(snap["countries"]["SOV"]["diplomacy"]["at_war"])
        self.assertIsNone(snap["countries"]["SOV"]["politics"]["ideas"])
        self.assertEqual(schema.validate_snapshot(snap), [])
        # mirror fold_save_bundle's per-country merge: SOV un-measured -> save preserved
        prot = []
        for dom in snap["provenance"]["memory"]["domains"]:
            prot.extend(asm._LIVE_COUNTRY_DOMAIN_LEAVES.get(dom, ()))
        save_sov = schema.empty_country()
        save_sov["diplomacy"]["at_war"] = ["GER", "ENG"]
        save_sov["politics"]["ideas"] = ["sov_real"]
        m_sov = asm._merge_country(snap["countries"]["SOV"], save_sov, prot,
                                   tag="SOV", measured_leaf_tags={})
        # SOV un-measured by the live domains -> save at_war / ideas preserved (no fabricated [])
        self.assertEqual(m_sov["diplomacy"]["at_war"], ["GER", "ENG"])
        self.assertEqual(m_sov["politics"]["ideas"], ["sov_real"])
        # GER IS measured -> live wins
        save_ger = schema.empty_country()
        save_ger["diplomacy"]["at_war"] = ["STALE"]
        m_ger = asm._merge_country(snap["countries"]["GER"], save_ger, prot,
                                   tag="GER", measured_leaf_tags={})
        self.assertEqual(m_ger["diplomacy"]["at_war"], ["SOV"])

    def test_diplomacy_reader_drops_non_tag_keys_and_enemies(self):
        # the registry reader keeps ONLY schema-valid TAG keys + enemies + members + pairs.
        plugs = {p.name: p for p in reg.default_registry()}

        class _FakeProc:
            pass
        ctx = reg.CaptureContext(proc=_FakeProc())

        def _fake_dip(p, ident=None, war_addrs=None, want_war_addrs=False):
            # accepts the Stage-1.5 war-relation-pin kwargs (war_addrs / want_war_addrs); a fake
            # with no records returns no war_addrs (warm-pin cache stays empty -> sweep next time).
            return {
                "at_war": {"GER": ["SOV", "42"], "SOV": ["GER"], "42": ["GER"]},
                "casualties": {"GER": 10, "42": 99},
                "faction_group": {"GER": 7, "42": 1},
                "war_relations": [
                    {"first": "GER", "second": "SOV", "first_casualties": 1,
                     "second_casualties": 2},
                    {"first": "GER", "second": "42", "first_casualties": 3,
                     "second_casualties": 4}],
                "faction_members": {7: ["GER", "ITA", "42"], 1: ["42"]},
                "war_relation_subtype_warning": {"0xdead": 1},
                "war_addrs": None,
            }
        out = plugs["diplomacy_live"]._reader(ctx, _fake_dip)
        # numeric "42" dropped from country keys
        self.assertIn("GER", out["countries"])
        self.assertNotIn("42", out["countries"])
        # non-TAG enemy "42" filtered from GER's at_war list
        self.assertEqual(out["countries"]["GER"]["at_war"], ["SOV"])
        # war_relation with a non-TAG end dropped entirely
        self.assertEqual(len(out["war_relations"]), 1)
        self.assertEqual(out["war_relations"][0]["second"], "SOV")
        # non-TAG member "42" filtered from faction_members; group keys stringified
        self.assertEqual(out["faction_members"]["7"], ["GER", "ITA"])
        self.assertEqual(out["faction_members"]["1"], [])

    def test_politics_reader_drops_addr_and_pp_and_non_tags(self):
        # the reader strips _status_addr, does NOT carry political_power, drops non-TAG keys.
        plugs = {p.name: p for p in reg.default_registry()}

        class _FakeProc:
            pass
        ctx = reg.CaptureContext(proc=_FakeProc())

        def _fake_pol(p, bridge=None, strict=True, **kw):
            return {
                "GER": {"popularities": {"fascism": 80.0}, "ruling_party": "fascism",
                        "ideas": ["a"], "idea_count": 1, "political_power": 120.5,
                        "election_frequency": 48, "party_count": 4,
                        "low_confidence": False, "_status_addr": 0xCAFE},
                "42": {"popularities": {}, "ruling_party": None, "ideas": [],
                       "idea_count": 0, "political_power": 1.0,
                       "election_frequency": None, "party_count": 4,
                       "low_confidence": False, "_status_addr": 0x1},
            }
        out = plugs["politics_live"]._reader(ctx, _fake_pol)
        self.assertIn("GER", out["countries"])
        self.assertNotIn("42", out["countries"])        # numeric key dropped
        blob = repr(out)
        self.assertNotIn("_status_addr", blob)           # diagnostic addr stripped
        self.assertNotIn("0xCAFE", blob)
        self.assertNotIn("political_power", blob)         # PP not carried (country_scalars owns)

    def test_offline_readers_raise(self):
        # ctx.proc None -> diplomacy_live / politics_live raise (domain honestly omitted).
        plugs = {p.name: p for p in reg.default_registry()}
        ctx = reg.CaptureContext(proc=None)
        with self.assertRaises(Exception):
            plugs["diplomacy_live"]._reader(ctx, lambda p: {})
        with self.assertRaises(Exception):
            plugs["politics_live"]._reader(
                ctx, lambda p, bridge=None, strict=True: {})

    def test_absent_domains_stay_null_and_unclaimed(self):
        snap = asm.assemble_snapshot({}, game_id="g", date="1940.9.10.13",
                                     identity=FakeIdentity())
        self.assertEqual(schema.validate_snapshot(snap), [])
        doms = snap["provenance"]["memory"]["domains"]
        self.assertNotIn("diplomacy_live", doms)
        self.assertNotIn("politics_live", doms)
        # the world sidecar stays the null-but-present skeleton
        dl = snap["diplomacy_live"]
        self.assertIsNone(dl["war_relations"])
        self.assertIsNone(dl["faction_members"])
        self.assertIsNone(dl["source"])

    def test_failed_domain_omitted_and_block_null(self):
        res = {"diplomacy_live": self._res("diplomacy_live", None, captured=False,
                                           error="boom")}
        snap = asm.assemble_snapshot(res, game_id="g", date="1940.9.10.13",
                                     identity=FakeIdentity())
        self.assertNotIn("diplomacy_live",
                         snap["provenance"]["memory"]["domains"])
        self.assertEqual(snap["countries"], {})
        self.assertIsNone(snap["diplomacy_live"]["source"])


class TestDiploRelationsLiveDomain(unittest.TestCase):
    """MOCK-based coverage for the LIVE bilateral-relations domain (diplo_relations):
    folds PER-COUNTRY onto countries.*.diplomacy.{opinion (live-first onto the save-shared
    leaf), relation_flags, overlord, autonomy_state, autonomy_ratio}. NO live memory --
    the reader output is a canned DomainResult / a fake reader fn drives the registry reader
    directly. Asserts the "SRC|TGT" matrix is transposed per-country, every TAG is valid_tag-
    checked, measured values land, the diplomacy_live sidecar is NOT polluted, the save
    backstop cannot overwrite a live-measured opinion matrix, and validate_snapshot passes."""

    def _res(self, name, data, *, available=True, captured=True, error=None):
        return reg.DomainResult(name, available=available, captured=captured,
                                data=data, error=error)

    def _full(self):
        # ASSEMBLER-SHAPE data (already plug-transposed: per-SRC {TGT: ...} maps).
        return {"diplo_relations": self._res("diplo_relations", {
            "countries": {
                "GER": {"opinion": {"SOV": -120, "ITA": 80},
                        "relation_flags": {"ITA": ["non_aggression_pact", "guarantee"],
                                           "SOV": ["embargo"]}},
                # ITA is GER's subject (overlord GER) with a measured autonomy state/ratio
                "ITA": {"opinion": {"GER": 75},
                        "overlord": "GER", "autonomy_state": "autonomy_puppet",
                        "autonomy_ratio": 0.5},
                # SOV has only an opinion row (no flags, not a subject)
                "SOV": {"opinion": {"GER": -120}},
            },
            "count": 3,
        })}

    def test_diplo_relations_folds_per_country_and_validates(self):
        snap = asm.assemble_snapshot(self._full(), game_id="g",
                                     date="1940.9.10.13", identity=FakeIdentity())
        self.assertEqual(schema.validate_snapshot(snap), [])
        self.assertIn("diplo_relations", snap["provenance"]["memory"]["domains"])
        ger = snap["countries"]["GER"]["diplomacy"]
        ita = snap["countries"]["ITA"]["diplomacy"]
        # opinion transposed per-country (folds onto the save-shared opinion leaf)
        self.assertEqual(ger["opinion"], {"SOV": -120, "ITA": 80})
        self.assertEqual(ger["relation_flags"]["ITA"],
                         ["non_aggression_pact", "guarantee"])
        # subject/autonomy landed on the subject's per-country record
        self.assertEqual(ita["overlord"], "GER")
        self.assertEqual(ita["autonomy_state"], "autonomy_puppet")
        self.assertEqual(ita["autonomy_ratio"], 0.5)
        # GER is not a subject -> overlord stays the skeleton None
        self.assertIsNone(ger["overlord"])

    def test_diplo_relations_does_not_pollute_diplomacy_live_sidecar(self):
        snap = asm.assemble_snapshot(self._full(), game_id="g",
                                     date="1940.9.10.13", identity=FakeIdentity())
        # the opinion/flags matrix is PER-COUNTRY only; the world sidecar stays its skeleton
        dl = snap["diplomacy_live"]
        self.assertNotIn("opinion", dl)
        self.assertNotIn("relation_flags", dl)
        self.assertIsNone(dl["war_relations"])
        # the save-derived diplomacy.wars leaf is untouched (distinct leaf)
        self.assertEqual(snap["countries"]["GER"]["diplomacy"]["wars"], [])

    def test_save_backstop_cannot_overwrite_live_opinion_matrix(self):
        # _merge_country must re-assert the live opinion matrix AFTER the merge -- the LIVE
        # matrix is the complete measured truth, so it REPLACES (never key-merges) a stale save
        # opinion map; the save-only diplomacy.wars/at_war leaves are preserved.
        prot = list(asm._LIVE_COUNTRY_DOMAIN_LEAVES["diplo_relations"])
        live_c = schema.empty_country()
        live_c["diplomacy"]["opinion"] = {"SOV": -120}      # measured matrix -> must WIN whole
        live_c["diplomacy"]["relation_flags"] = {"SOV": ["embargo"]}
        live_c["diplomacy"]["overlord"] = "GER"
        save_c = schema.empty_country()
        # stale save opinion has an EXTRA target the live read did not include -> live replaces
        save_c["diplomacy"]["opinion"] = {"SOV": 5, "ENG": -200}
        save_c["diplomacy"]["wars"] = [{"against": "SOV"}]   # save-only -> preserved
        merged = asm._merge_country(live_c, save_c, prot, tag="ITA",
                                    measured_leaf_tags={})
        # live opinion matrix WON whole (the stale ENG key did NOT survive -- replace, not merge)
        self.assertEqual(merged["diplomacy"]["opinion"], {"SOV": -120})
        self.assertEqual(merged["diplomacy"]["relation_flags"], {"SOV": ["embargo"]})
        self.assertEqual(merged["diplomacy"]["overlord"], "GER")
        # save-only sibling leaf preserved
        self.assertEqual(merged["diplomacy"]["wars"], [{"against": "SOV"}])

    def test_unmeasured_country_keeps_save_opinion(self):
        # a country NOT measured by diplo_relations carries the skeleton None opinion; the
        # `lv is not None` guard in _merge_country lets a real save opinion map survive.
        prot = list(asm._LIVE_COUNTRY_DOMAIN_LEAVES["diplo_relations"])
        live_c = schema.empty_country()                      # opinion None (unmeasured)
        save_c = schema.empty_country()
        save_c["diplomacy"]["opinion"] = {"SOV": -50}
        merged = asm._merge_country(live_c, save_c, prot, tag="FRA",
                                    measured_leaf_tags={})
        self.assertEqual(merged["diplomacy"]["opinion"], {"SOV": -50})  # save preserved

    def test_absent_diplo_relations_stays_null_and_unclaimed(self):
        snap = asm.assemble_snapshot({}, game_id="g", date="1940.9.10.13",
                                     identity=FakeIdentity())
        self.assertEqual(schema.validate_snapshot(snap), [])
        self.assertNotIn("diplo_relations",
                         snap["provenance"]["memory"]["domains"])

    def test_failed_diplo_relations_omitted(self):
        res = {"diplo_relations": self._res("diplo_relations", None, captured=False,
                                            error="boom")}
        snap = asm.assemble_snapshot(res, game_id="g", date="1940.9.10.13",
                                     identity=FakeIdentity())
        self.assertNotIn("diplo_relations",
                         snap["provenance"]["memory"]["domains"])
        self.assertEqual(snap["countries"], {})

    # -- reader-level: transpose "SRC|TGT" + valid_tag discipline, driven by a fake reader fn --
    def test_reader_transposes_and_drops_non_tags(self):
        plugs = {p.name: p for p in reg.default_registry()}

        class _FakeProc:
            pass
        ctx = reg.CaptureContext(proc=_FakeProc())

        def _fake_rel(p, ident=None):
            return {
                "opinion": {
                    "GER|SOV": -120, "GER|ITA": 80,
                    "GER|WST": 5,          # WST pseudo-tag end -> dropped
                    "42|GER": 9,           # non-TAG src -> dropped
                    "GER|99": 7,           # non-TAG tgt -> dropped
                },
                "flags": {
                    "GER|ITA": ["non_aggression_pact", "guarantee"],
                    "ITA|GER": ["non_aggression_pact"],
                    "GER|WST": ["embargo"],   # WST end -> dropped
                },
                "undirected_flags": {"GER|ITA": ["non_aggression_pact"]},
                "subjects": [
                    {"overlord": "GER", "subject": "ITA",
                     "autonomy_state": "autonomy_puppet", "value": 0.5},
                    # a subject with a non-TAG end -> dropped
                    {"overlord": "GER", "subject": "WST",
                     "autonomy_state": "x", "value": 0.1},
                ],
            }
        out = plugs["diplo_relations"]._reader(ctx, _fake_rel)
        ger = out["countries"]["GER"]
        # transposed per-SRC; WST + non-TAG ends dropped
        self.assertEqual(ger["opinion"], {"SOV": -120, "ITA": 80})
        self.assertEqual(ger["relation_flags"]["ITA"],
                         ["non_aggression_pact", "guarantee"])
        self.assertNotIn("WST", ger["relation_flags"])
        # ITA is the subject -> overlord/autonomy on ITA's record
        ita = out["countries"]["ITA"]
        self.assertEqual(ita["overlord"], "GER")
        self.assertEqual(ita["autonomy_state"], "autonomy_puppet")
        self.assertEqual(ita["autonomy_ratio"], 0.5)
        # no numeric/WST country keys leaked
        for bad in ("42", "99", "WST"):
            self.assertNotIn(bad, out["countries"])

    def test_reader_raises_without_proc(self):
        ctx = reg.CaptureContext(proc=None)
        plug = {p.name: p for p in reg.default_registry()}["diplo_relations"]
        with self.assertRaises(Exception):
            plug._reader(ctx, lambda p: {})


class TestLeaderDetailEnrichment(unittest.TestCase):
    """MOCK-based coverage for the OPT-IN leader_detail enrichment folded INSIDE the
    commanders_live plug: it joins overall SKILL + assigned-TRAIT token set onto each
    army/navy leader by unit_leader_id (leader_skill_traits_live), gated by
    ctx.extras["resolve_leader_detail"] (default OFF). NO live memory -- a fake commander
    reader + a monkeypatched _leader_detail_map drive the plug. Asserts the join lands,
    skill/traits are absent when OFF, fail-soft when the reader is broken, the
    "leader_detail" provenance id is claimed iff the enrichment ran, and no fabrication."""

    def _res(self, name, data, *, available=True, captured=True, error=None):
        return reg.DomainResult(name, available=available, captured=captured,
                                data=data, error=error)

    class _FakeCmdReader:
        pid = None  # _verify_same_process returns early (no identity)

        def verify_build(self):
            return True, "sha"

        def close(self):
            pass

        _OUT = {
            "army_leaders": [
                {"char_id": 11, "unit_leader_id": 101, "name": "Guderian",
                 "country": "GER", "level": None,
                 "trait_xp": [{"trait_id": 7, "trait_xp": 100}]},
                # a leader the detail scan did NOT measure -> no skill/traits keys
                {"char_id": 12, "unit_leader_id": 102, "name": "Rommel",
                 "country": "GER", "level": None}],
            "navy_leaders": [
                {"char_id": 13, "unit_leader_id": 201, "name": "Doenitz",
                 "country": "GER", "level": None}],
            "characters": {11: {"country": "GER"}},
            "counts": {"army_leaders": 2, "navy_leaders": 1, "characters": 1}}

        # Stage 1.5 pinned API (the registry reader consumes these, not enumerate()).
        def enumerate_addrs(self):
            return {"army": [101, 102], "navy": [201], "char": [11],
                    "country_leader": [], "_ids": {101: 101, 102: 102, 201: 201, 11: 11}}

        def enumerate_from_addrs(self, addr_sets, resolve_country=True, read_traits=True):
            import copy
            return copy.deepcopy(self._OUT)

        def validate_addrs(self, addr_sets):
            return True

    class _CmdImpl:
        def __call__(self):
            return TestLeaderDetailEnrichment._FakeCmdReader()

    def _run_plug(self, ctx, detail_map=None, detail_ran=False):
        """Drive the commanders_live plug reader with the fake commander reader, monkeypatching
        _leader_detail_map so no live leader_skill_traits read happens."""
        plug = {p.name: p for p in reg.default_registry()}["commanders_live"]
        orig = reg._leader_detail_map
        reg._leader_detail_map = lambda c: (detail_map or {}, detail_ran)
        try:
            return plug._reader(ctx, self._CmdImpl())
        finally:
            reg._leader_detail_map = orig

    def test_enrichment_off_by_default_no_skill_traits(self):
        # resolve_leader_detail not set -> the real _leader_detail_map returns ({}, False),
        # so leaders carry NO skill/traits and the flag is False (domain not claimed).
        class _FakeProc:
            pass
        ctx = reg.CaptureContext(proc=_FakeProc())   # no resolve_leader_detail
        plug = {p.name: p for p in reg.default_registry()}["commanders_live"]
        out = plug._reader(ctx, self._CmdImpl())
        self.assertFalse(out["leader_detail_enriched"])
        for ld in out["army_by_tag"]["GER"] + out["navy_by_tag"]["GER"]:
            self.assertNotIn("skill", ld)
            self.assertNotIn("traits", ld)

    def test_enrichment_joins_skill_and_traits_by_ulid(self):
        class _FakeProc:
            pass
        ctx = reg.CaptureContext(proc=_FakeProc(),
                                 extras={"resolve_leader_detail": True})
        detail = {
            101: {"skill": 8, "traits": ["panzer_leader", "fast_planner"]},
            201: {"skill": 5, "traits": []},   # measured-empty traits (real "no traits")
            # 102 deliberately absent -> Rommel gets no skill/traits
        }
        out = self._run_plug(ctx, detail_map=detail, detail_ran=True)
        self.assertTrue(out["leader_detail_enriched"])
        gud = [l for l in out["army_by_tag"]["GER"]
               if l["unit_leader_id"] == 101][0]
        self.assertEqual(gud["skill"], 8)
        self.assertEqual(gud["traits"], ["panzer_leader", "fast_planner"])
        # measured-empty traits list survives (0 != null)
        doen = [l for l in out["navy_by_tag"]["GER"]
                if l["unit_leader_id"] == 201][0]
        self.assertEqual(doen["traits"], [])
        self.assertEqual(doen["skill"], 5)
        # the un-measured leader (102) carries NO skill/traits keys (no fabrication)
        rommel = [l for l in out["army_by_tag"]["GER"]
                  if l["unit_leader_id"] == 102][0]
        self.assertNotIn("skill", rommel)
        self.assertNotIn("traits", rommel)

    def test_assembler_claims_leader_detail_iff_enriched(self):
        # the enriched leaders + the leader_detail_enriched flag drive the assembler to
        # claim the distinct "leader_detail" provenance id; skill/traits land on the snapshot.
        data = {
            "army_by_tag": {"GER": [
                {"char_id": 11, "unit_leader_id": 101, "name": "Guderian",
                 "country": "GER", "level": None, "skill": 8,
                 "traits": ["panzer_leader"]}]},
            "navy_by_tag": {"GER": [
                {"char_id": 13, "unit_leader_id": 201, "name": "Doenitz",
                 "country": "GER", "level": None, "skill": 5, "traits": []}]},
            "counts_by_tag": {"GER": {"army_leaders": 1, "navy_leaders": 1,
                                      "characters": 1}},
            "leader_detail_enriched": True, "count": 1}
        snap = asm.assemble_snapshot(
            {"commanders_live": self._res("commanders_live", data)},
            game_id="g", date="1940.9.10.13", identity=FakeIdentity())
        self.assertEqual(schema.validate_snapshot(snap), [])
        doms = snap["provenance"]["memory"]["domains"]
        self.assertIn("commanders_live", doms)
        self.assertIn("leader_detail", doms)        # claimed because enriched
        cmd = snap["countries"]["GER"]["commanders"]
        self.assertEqual(cmd["army_leaders"][0]["skill"], 8)
        self.assertEqual(cmd["army_leaders"][0]["traits"], ["panzer_leader"])
        self.assertEqual(cmd["navy_leaders"][0]["traits"], [])

    def test_assembler_does_not_claim_leader_detail_when_off(self):
        # commanders_live ran but the enrichment did NOT (flag False / absent) -> leader_detail
        # is NOT in provenance, and leaders carry no skill/traits (no fabrication).
        data = {
            "army_by_tag": {"GER": [
                {"char_id": 11, "unit_leader_id": 101, "name": "Guderian",
                 "country": "GER", "level": None}]},
            "navy_by_tag": {}, "counts_by_tag": {"GER": {"army_leaders": 1,
                                                         "navy_leaders": 0,
                                                         "characters": 0}},
            "leader_detail_enriched": False, "count": 1}
        snap = asm.assemble_snapshot(
            {"commanders_live": self._res("commanders_live", data)},
            game_id="g", date="1940.9.10.13", identity=FakeIdentity())
        doms = snap["provenance"]["memory"]["domains"]
        self.assertIn("commanders_live", doms)
        self.assertNotIn("leader_detail", doms)
        ld = snap["countries"]["GER"]["commanders"]["army_leaders"][0]
        self.assertNotIn("skill", ld)
        self.assertNotIn("traits", ld)

    def test_enrichment_fail_soft_leaves_leaders_intact(self):
        # if _leader_detail_map raised internally it returns ({}, False) (fail-soft); the plug
        # must still emit leaders, just without skill/traits and with the flag False.
        class _FakeProc:
            pass
        ctx = reg.CaptureContext(proc=_FakeProc(),
                                 extras={"resolve_leader_detail": True})
        out = self._run_plug(ctx, detail_map={}, detail_ran=False)
        self.assertFalse(out["leader_detail_enriched"])
        self.assertEqual(len(out["army_by_tag"]["GER"]), 2)   # leaders still emitted
        for ld in out["army_by_tag"]["GER"]:
            self.assertNotIn("skill", ld)


class TestNewPlugsRegistered(unittest.TestCase):
    """The default registry exposes the new plugs, and the save backstops are
    OPT-IN: they RAISE (captured=False -> omitted) when no save path is supplied,
    so a pure-live capture omits them by default (no fabrication). No live memory:
    we drive each plug's reader directly with a fake context."""

    def test_default_registry_includes_new_plugs(self):
        names = {p.name for p in reg.default_registry()}
        for n in ("land_combats", "buildings", "intel"):
            self.assertIn(n, names)

    def test_default_registry_includes_wired_live_plugs(self):
        names = {p.name for p in reg.default_registry()}
        for n in ("state_detail", "country_misc", "designs", "mio", "ships",
                  "projects_live", "tech_live", "commanders_live", "intel_live",
                  "focus_live", "production_live", "diplomacy_live", "politics_live",
                  "diplo_relations"):
            self.assertIn(n, names)
        # leader_detail is NOT a standalone plug -- it is the OPT-IN enrichment folded
        # inside commanders_live, so it must NOT appear as a registry plug name.
        self.assertNotIn("leader_detail", names)

    def test_save_backstops_raise_without_path(self):
        ctx = reg.CaptureContext(extras={})  # no melt_path / state_detail_path
        plugs = {p.name: p for p in reg.default_registry()}
        # force the readers to run with an in-memory stub impl (no real import/proc)
        for name in ("buildings", "intel"):
            plug = plugs[name]
            # bypass the lazy module import; call the reader fn directly with a
            # dummy impl so we exercise the no-path RAISE branch (no live memory).
            with self.assertRaises(Exception):
                plug._reader(ctx, object())

    def test_save_backstops_run_with_path_via_reader_fn(self):
        # supply a melt_path; the reader fn must build a result WITHOUT touching
        # ctx.proc. We feed a fake `bs`/`isv` impl module so nothing reads a file.
        class _FakeLayer:
            prov = {"rail_way": {2068: 5}, "supply_node": {}, "naval_base": {}}
            states = {2: {}}
            def rail_map(self):
                return dict(self.prov["rail_way"])
            def infrastructure(self, sid):
                return 3
        class _FakeBS:
            class BuildingsSave:
                @staticmethod
                def from_melt(p):
                    return _FakeLayer()
            class StateBuildings:
                @staticmethod
                def from_state_detail(p):
                    return _FakeLayer()
        ctx = reg.CaptureContext(extras={"melt_path": "X.hoi4",
                                         "state_detail_path": "Y.json"})
        plugs = {p.name: p for p in reg.default_registry()}
        out = plugs["buildings"]._reader(ctx, _FakeBS)
        self.assertEqual(out["source"], "save")
        self.assertEqual(out["rail_by_province"]["2068"], 5)

        class _FakeIntelLayer:
            def observers(self):
                return ["GER"]
            def observer_map(self, t):
                return {5: {"civ": 1.0, "army": 0.0, "navy": 0.0, "air": 0.0}}
            def stats(self):
                return {"observers": 1, "total_rows": 1}
        class _FakeISV:
            class IntelSave:
                @staticmethod
                def from_melt(p):
                    return _FakeIntelLayer()
        out2 = plugs["intel"]._reader(ctx, _FakeISV)
        self.assertFalse(out2["target_index_is_tag"])
        self.assertIn("5", out2["by_observer"]["GER"])
        # the source identifier is the redacted basename key (not a raw path)
        self.assertEqual(out2["melt_file"], "X.hoi4")
        self.assertNotIn("melt_path", out2)


class TestContinuity(unittest.TestCase):
    def _tracker(self, **kw):
        return cont.ContinuityTracker(cont.ContinuityState(), **kw)

    def test_first_capture_segment_1(self):
        t = self._tracker()
        tags = t.tag(process_instance_id="A", pid=1, hour_index=100)
        self.assertEqual(tags.segment_id, 1)
        self.assertEqual(tags.capture_seq, 1)
        self.assertEqual(tags.confidence, "exact")
        self.assertIsNone(tags.continuity_break)
        self.assertEqual(tags.campaign_id_source, "process_instance")

    def test_normal_forward_step_same_segment(self):
        t = self._tracker()
        t.tag(process_instance_id="A", pid=1, hour_index=100)
        tags = t.tag(process_instance_id="A", pid=1, hour_index=124)
        self.assertEqual(tags.segment_id, 1)
        self.assertIsNone(tags.continuity_break)

    def test_same_hour_is_not_rollback(self):
        t = self._tracker()
        t.tag(process_instance_id="A", pid=1, hour_index=100)
        tags = t.tag(process_instance_id="A", pid=1, hour_index=100)
        self.assertEqual(tags.segment_id, 1)
        self.assertIsNone(tags.continuity_break)

    def test_rollback_new_segment(self):
        t = self._tracker()
        t.tag(process_instance_id="A", pid=1, hour_index=100)
        tags = t.tag(process_instance_id="A", pid=1, hour_index=90)
        self.assertEqual(tags.segment_id, 2)
        self.assertEqual(tags.continuity_break, "time_rollback")

    def test_process_change_new_segment(self):
        t = self._tracker()
        t.tag(process_instance_id="A", pid=1, hour_index=100)
        tags = t.tag(process_instance_id="B", pid=2, hour_index=200)
        self.assertEqual(tags.segment_id, 2)
        self.assertEqual(tags.continuity_break, "process_changed")

    def test_large_forward_jump_degraded_new_segment(self):
        t = self._tracker()
        t.tag(process_instance_id="A", pid=1, hour_index=100)
        far = 100 + cont.SUSPICIOUS_FORWARD_JUMP_HOURS + 1
        tags = t.tag(process_instance_id="A", pid=1, hour_index=far)
        self.assertEqual(tags.segment_id, 2)
        self.assertEqual(tags.continuity_break, "time_jump_forward")
        self.assertEqual(tags.confidence, "degraded")

    def test_unknown_time_never_segments_or_overwrites(self):
        t = self._tracker()
        t.tag(process_instance_id="A", pid=1, hour_index=100)
        # unknown time in the middle: degraded, no segment, no overwrite
        u = t.tag(process_instance_id="A", pid=1, hour_index=None)
        self.assertEqual(u.segment_id, 1)
        self.assertEqual(u.continuity_break, "unknown_time")
        self.assertEqual(u.confidence, "degraded")
        # a later KNOWN hour LOWER than the pre-gap known hour is still a rollback
        # (compared across the unknown gap, Codex A)
        r = t.tag(process_instance_id="A", pid=1, hour_index=90)
        self.assertEqual(r.segment_id, 2)
        self.assertEqual(r.continuity_break, "time_rollback")

    def test_campaign_id_stable_across_tool_restart(self):
        # same process instance -> same campaign_id even with a fresh tracker
        # (simulated tool restart): the campaign_id is process-derived, not
        # wallclock-derived (Codex C).
        t1 = self._tracker()
        a = t1.tag(process_instance_id="A", pid=1, hour_index=100)
        t2 = self._tracker()  # fresh state == restart
        b = t2.tag(process_instance_id="A", pid=1, hour_index=130)
        self.assertEqual(a.campaign_id, b.campaign_id)

    def test_save_seed_overrides_campaign_id(self):
        t = self._tracker(save_seed="XYZ")
        tags = t.tag(process_instance_id="A", pid=1, hour_index=100)
        self.assertEqual(tags.campaign_id, "seed-XYZ")
        self.assertEqual(tags.campaign_id_source, "save_seed")

    def test_process_change_then_unknown_then_known_no_false_rollback(self):
        # Codex code-review #3: a process change must DROP last_known_hour so the
        # next known hour (even if < the OLD process's last hour) is NOT a rollback.
        t = self._tracker()
        t.tag(process_instance_id="A", pid=1, hour_index=5000)  # old proc, late
        # new process, first capture unknown-time (no overwrite of last_known_hour)
        u = t.tag(process_instance_id="B", pid=2, hour_index=None)
        self.assertEqual(u.continuity_break, "process_changed")
        self.assertEqual(u.segment_id, 2)
        # new process now reports an EARLY hour: this is the new campaign's start,
        # NOT a rollback of the old one -> same (new) segment, no rollback.
        k = t.tag(process_instance_id="B", pid=2, hour_index=100)
        self.assertEqual(k.segment_id, 2)
        self.assertIsNone(k.continuity_break)

    def test_compute_does_not_mutate_until_commit(self):
        # Codex code-review #2: compute() is pure; the live state only advances on
        # commit(). A failed write (no commit) must not advance continuity.
        t = self._tracker()
        t.tag(process_instance_id="A", pid=1, hour_index=100)  # seq=1 committed
        prop = t.compute(process_instance_id="A", pid=1, hour_index=124)
        # the proposal says seq 2, but the live state is still at seq 1
        self.assertEqual(prop.tags.capture_seq, 2)
        self.assertEqual(t.state.capture_seq, 1)
        self.assertEqual(t.state.last_known_hour, 100)
        # recomputing yields the same seq (no drift) until we commit
        prop2 = t.compute(process_instance_id="A", pid=1, hour_index=124)
        self.assertEqual(prop2.tags.capture_seq, 2)
        t.commit(prop2)
        self.assertEqual(t.state.capture_seq, 2)
        self.assertEqual(t.state.last_known_hour, 124)

    def test_first_capture_unknown_time_is_degraded(self):
        # Adversarial review #2: a FIRST capture with no known in-game time must be
        # stamped unknown_time/degraded, not exact/no-break. The segment still
        # opens (it's the first) but the time is honestly flagged.
        t = self._tracker()
        tags = t.tag(process_instance_id="A", pid=1, hour_index=None)
        self.assertEqual(tags.segment_id, 1)
        self.assertEqual(tags.continuity_break, "unknown_time")
        self.assertEqual(tags.confidence, "degraded")
        # last_known_hour stays None (an unknown time never sets it), so the next
        # KNOWN hour is the first real one in the segment (no false rollback).
        k = t.tag(process_instance_id="A", pid=1, hour_index=50)
        self.assertEqual(k.segment_id, 1)
        self.assertIsNone(k.continuity_break)

    def test_resume_from_persisted_state_is_not_first_capture(self):
        # a state loaded from disk (campaign + process set) must not be treated as
        # the first capture.
        st = cont.ContinuityState(
            last_process_instance="A", last_pid=1, last_known_hour=100,
            campaign_id="proc-A", campaign_id_source="process_instance",
            segment_id=3, capture_seq=9)
        t = cont.ContinuityTracker(st)
        tags = t.tag(process_instance_id="A", pid=1, hour_index=124)
        self.assertEqual(tags.segment_id, 3)       # same segment, not reset to 1
        self.assertEqual(tags.capture_seq, 10)


class TestDurableStore(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="lc_store_")

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def test_write_and_index(self):
        store = DurableCaptureStore(self.tmp)
        snap = schema.empty_snapshot("g", "1940.9.10.13")
        snap["_capture"] = {"campaign_id": "c1", "segment_id": 1,
                            "capture_seq": 1}
        path = store.write_snapshot(snap)
        self.assertTrue(os.path.isfile(path))
        idx = read_json(store.store.index_path("g"))
        self.assertEqual(len(idx["snapshots"]), 1)
        self.assertEqual(idx["snapshots"][0]["campaign_id"], "c1")
        # no leftover temp files
        leftovers = [f for f in os.listdir(os.path.dirname(path))
                     if ".tmp." in f]
        self.assertEqual(leftovers, [])

    def test_unknown_hour_goes_noncanonical(self):
        store = DurableCaptureStore(self.tmp)
        snap = schema.empty_snapshot("g", None)  # hour_index None
        snap["_capture"] = {"capture_seq": 7}
        path = store.write_snapshot(snap)
        self.assertIn("_noncanonical", path)
        # index NOT created/updated for a non-canonical write
        self.assertIsNone(read_json(store.store.index_path("g")))

    def test_atomic_write_replaces(self):
        p = os.path.join(self.tmp, "x.json")
        atomic_write_json(p, {"a": 1})
        atomic_write_json(p, {"a": 2})
        self.assertEqual(read_json(p), {"a": 2})

    def test_rollback_does_not_overwrite_prior_segment(self):
        # Codex code-review #1: a rollback to an already-captured hour in a NEW
        # segment must produce a DISTINCT file, not overwrite segment 1's snapshot.
        store = DurableCaptureStore(self.tmp)
        s1 = schema.empty_snapshot("g", "1940.9.10.13")
        s1["_capture"] = {"campaign_id": "c1", "segment_id": 1, "capture_seq": 1}
        p1 = store.write_snapshot(s1)
        # same hour, new segment (rollback re-captured the same tick)
        s2 = schema.empty_snapshot("g", "1940.9.10.13")
        s2["_capture"] = {"campaign_id": "c1", "segment_id": 2, "capture_seq": 5}
        p2 = store.write_snapshot(s2)
        self.assertNotEqual(p1, p2)
        self.assertTrue(os.path.isfile(p1) and os.path.isfile(p2))
        idx = read_json(store.store.index_path("g"))
        self.assertEqual(len(idx["snapshots"]), 2)
        segs = sorted(e["segment_id"] for e in idx["snapshots"])
        self.assertEqual(segs, [1, 2])

    def test_noncanonical_same_seg_seq_does_not_overwrite(self):
        # Adversarial review #1: a crash between snapshot-write and continuity-save
        # can recompute the SAME (segment, capture_seq) on restart. A non-canonical
        # capture is "kept, never deleted", so two writes at the same (seg, seq)
        # must produce DISTINCT files (time_ns tiebreaker), never overwrite.
        store = DurableCaptureStore(self.tmp)
        paths = []
        # write several back-to-back at the SAME (seg, seq) -- on Windows these
        # share a time_ns() (16 ms granularity), so the monotonic counter is what
        # keeps them distinct. All must survive (kept, never deleted).
        for _ in range(4):
            s = schema.empty_snapshot("g", None)  # unknown hour -> noncanonical
            s["_capture"] = {"segment_id": 1, "capture_seq": 6}
            paths.append(store.write_snapshot(s))
        self.assertEqual(len(set(paths)), 4)  # all distinct
        for p in paths:
            self.assertTrue(os.path.isfile(p))
            self.assertIn("_noncanonical", p)

    def test_unsafe_game_id_rejected(self):
        # Codex code-review #8: a game_id with a path separator / .. is refused.
        store = DurableCaptureStore(self.tmp)
        for bad in ("../escape", "a/b", "..", "x" * 200):
            snap = schema.empty_snapshot(bad, "1940.9.10.13")
            snap["_capture"] = {"segment_id": 1, "capture_seq": 1}
            with self.assertRaises(ValueError):
                store.write_snapshot(snap)

    def test_lock_is_exclusive(self):
        lp = os.path.join(self.tmp, "x.lock")
        a = CaptureLock(lp).acquire()
        try:
            with self.assertRaises(RuntimeError):
                CaptureLock(lp).acquire()
        finally:
            a.release()
        # released -> reacquire works
        CaptureLock(lp).acquire().release()


class TestEndToEndDryRun(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="lc_e2e_")

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def test_capture_from_results_writes_durably_with_derive(self):
        loop = _make_loop(self.tmp)
        ident = FakeIdentity()
        res1 = loop.capture_from_results(
            _stub_results(), identity=ident,
            date_info=DateInfo("1940.9.10.13", trusted=True, source="mod_log"))
        self.assertTrue(res1.canonical)
        self.assertIsNotNone(res1.written_path)
        self.assertTrue(os.path.isfile(res1.written_path))
        # continuity tags present + survived derive
        self.assertIn("_capture", res1.snapshot)
        self.assertEqual(res1.snapshot["_capture"]["segment_id"], 1)
        # derive overlay ran (it adds _derived)
        self.assertIn("_derived", res1.snapshot)
        # snapshot on disk is valid
        from live_capture.durable_store import read_json as _rj
        disk = _rj(res1.written_path)
        self.assertEqual(schema.validate_snapshot(disk), [])

        # a second capture forward in time -> same segment, derive gets prev
        res2 = loop.capture_from_results(
            _stub_results(), identity=ident,
            date_info=DateInfo("1940.9.11.13", trusted=True, source="mod_log"),
            prev_snapshot=res1.snapshot)
        self.assertEqual(res2.snapshot["_capture"]["segment_id"], 1)
        self.assertGreater(res2.snapshot["hour_index"],
                           res1.snapshot["hour_index"])

        # continuity state persisted
        st = loop.store.load_continuity_state("dry_run")
        self.assertEqual(st["capture_seq"], 2)
        self.assertEqual(st["last_known_hour"], res2.snapshot["hour_index"])

    def test_measured_zero_division_count_survives_derive(self):
        # Codex reconcile #4: derive's deployed-manpower pass re-nulls
        # division_count for tags absent from divisions[]; the loop must restore
        # the MEASURED zero for a known country that simply has no division.
        loop = _make_loop(self.tmp, game_id="zerodiv")
        res = _stub_results()
        res["country_scalars"].data["countries"]["FRA"] = {"stability": 0.5}
        out = loop.capture_from_results(
            res, identity=FakeIdentity(),
            date_info=DateInfo("1940.9.10.13", trusted=True, source="mod_log"))
        snap = out.snapshot
        # derive ran (adds _derived); division_count is still a measured 0 for FRA
        self.assertIn("_derived", snap)
        self.assertEqual(snap["countries"]["FRA"]["military"]["division_count"], 0)
        self.assertEqual(snap["countries"]["GER"]["military"]["division_count"], 1)

    def test_untrusted_date_is_noncanonical(self):
        loop = _make_loop(self.tmp, game_id="untrusted")
        res = loop.capture_from_results(
            _stub_results(), identity=FakeIdentity(),
            date_info=DateInfo("1940.9.10.13", trusted=False,
                               source="save_meta"))
        self.assertFalse(res.canonical)
        self.assertIsNone(res.snapshot["hour_index"])
        self.assertIn("_noncanonical", res.written_path)
        # a note explains why the date was withheld
        notes = " ".join(res.snapshot["provenance"]["notes"])
        self.assertIn("UNTRUSTED", notes)

    def test_untrusted_date_source_is_redacted_in_notes(self):
        # Codex follow-up: a caller could pass an arbitrary string (even a path) as
        # DateInfo.source; it must NEVER leak into a persisted note. An unknown
        # source is reported as 'other', not echoed.
        loop = _make_loop(self.tmp, game_id="srcredact")
        res = loop.capture_from_results(
            _stub_results(), identity=FakeIdentity(),
            date_info=DateInfo("1940.9.10.13", trusted=False,
                               source="C:" + "\\Users\\someone\\private\\path.log"))
        notes = " ".join(res.snapshot["provenance"]["notes"])
        self.assertIn("other", notes)
        self.assertNotIn("Users", notes)
        self.assertNotIn("someone", notes)
        # a KNOWN source is still echoed verbatim (it is safe)
        res2 = loop.capture_from_results(
            _stub_results(), identity=FakeIdentity(),
            date_info=DateInfo("1940.9.11.13", trusted=False, source="save_meta"))
        self.assertIn("save_meta", " ".join(
            res2.snapshot["provenance"]["notes"]))

    def test_pid_change_between_captures_increments_segment(self):
        loop = _make_loop(self.tmp, game_id="pidchange")
        a = loop.capture_from_results(
            _stub_results(), identity=FakeIdentity("inst-A", 1),
            date_info=DateInfo("1940.9.10.13", trusted=True, source="mod_log"))
        b = loop.capture_from_results(
            _stub_results(), identity=FakeIdentity("inst-B", 2),
            date_info=DateInfo("1941.1.1.12", trusted=True, source="mod_log"))
        self.assertEqual(a.snapshot["_capture"]["segment_id"], 1)
        self.assertEqual(b.snapshot["_capture"]["segment_id"], 2)
        self.assertEqual(b.snapshot["_capture"]["continuity_break"],
                         "process_changed")


class TestFuelSupplyInputsLiveDomains(unittest.TestCase):
    """MOCK-based coverage for the 2 FINAL live domains wired this pass:
      * fuel_live -> the SAME canonical save home (country.economy.fuel.{current,max}) under a
        DISTINCT provenance id (live-first, protected). A measured 0.0 (empty tank) survives;
        current fuel is NOT range-gated (it legitimately reads below the save).
      * supply_inputs -> ADDITIONAL distinctly-named, separately-badged signals on the supply
        block (solver_efficiency_by_province / solver_draw_by_province), without touching the
        existing division-province proxy and never claiming the exact per-province supply.
    NO live memory -- every reader output is a canned DomainResult."""

    def _res(self, name, data, *, available=True, captured=True, error=None):
        return reg.DomainResult(name, available=available, captured=captured,
                                data=data, error=error)

    def _full(self):
        return {
            "fuel_live": self._res("fuel_live", {"countries": {
                "GER": {"fuel": 45000.0, "max_fuel": 71000.0},
                # measured zero current fuel (empty tank) survives; max measured too
                "SOV": {"fuel": 0.0, "max_fuel": 120000.0},
            }, "count": 2}),
            "supply_inputs": self._res("supply_inputs", {
                "efficiency_by_province": {"3324": 0.95, "3544": 1.0},
                "draw_by_province": {"3324": 12.0},
                "meta": {"csupplysystem": "0xABC", "efficiency_count": 2},
            }),
        }

    def test_both_domains_fold_and_validate(self):
        snap = asm.assemble_snapshot(self._full(), game_id="g",
                                     date="1940.9.10.13", identity=FakeIdentity())
        self.assertEqual(schema.validate_snapshot(snap), [])
        doms = snap["provenance"]["memory"]["domains"]
        self.assertIn("fuel_live", doms)
        self.assertIn("supply_inputs", doms)

    def test_fuel_folds_onto_economy_fuel_home(self):
        snap = asm.assemble_snapshot(self._full(), game_id="g",
                                     date="1940.9.10.13", identity=FakeIdentity())
        ger = snap["countries"]["GER"]["economy"]["fuel"]
        sov = snap["countries"]["SOV"]["economy"]["fuel"]
        self.assertEqual(ger["current"], 45000.0)
        self.assertEqual(ger["max"], 71000.0)
        # measured 0.0 current fuel survives (empty tank), NOT coerced to null
        self.assertEqual(sov["current"], 0.0)
        self.assertEqual(sov["max"], 120000.0)

    def test_fuel_measured_value_protected_from_save_backstop(self):
        # the save backstop must NOT overwrite a live-measured economy.fuel leaf (incl. 0.0).
        prot = list(asm._LIVE_COUNTRY_DOMAIN_LEAVES["fuel_live"])
        live_c = schema.empty_country()
        live_c["economy"]["fuel"] = {"current": 0.0, "max": 120000.0}  # measured empty tank
        save_c = schema.empty_country()
        save_c["economy"]["fuel"] = {"current": 99999.0, "max": 999.0}  # stale -> live wins
        merged = asm._merge_country(live_c, save_c, prot)
        self.assertEqual(merged["economy"]["fuel"]["current"], 0.0)   # measured 0 won
        self.assertEqual(merged["economy"]["fuel"]["max"], 120000.0)

    def test_fuel_unmeasured_leaf_lets_save_fill(self):
        # an UNMEASURED fuel leaf (None / absent) must let the save backstop fill it.
        prot = list(asm._LIVE_COUNTRY_DOMAIN_LEAVES["fuel_live"])
        live_c = schema.empty_country()        # economy.fuel == {} (nothing measured)
        save_c = schema.empty_country()
        save_c["economy"]["fuel"] = {"current": 500.0, "max": 1000.0}
        merged = asm._merge_country(live_c, save_c, prot)
        self.assertEqual(merged["economy"]["fuel"]["current"], 500.0)  # save filled
        self.assertEqual(merged["economy"]["fuel"]["max"], 1000.0)

    def test_fuel_reader_drops_non_tag_and_strips_addr(self):
        # the registry reader keeps only schema-valid TAG keys and strips fuel_status_addr.
        plugs = {p.name: p for p in reg.default_registry()}

        class _FakeProc:
            pass
        ctx = reg.CaptureContext(proc=_FakeProc())

        def _fake_fuel(p, bridge=None):
            return {"GER": {"fuel": 1.0, "max_fuel": 2.0, "fuel_status_addr": 0xDEAD},
                    "42": {"fuel": 9.0, "max_fuel": 9.0, "fuel_status_addr": 0xBEEF}}
        out = plugs["fuel_live"]._reader(ctx, _fake_fuel)
        self.assertIn("GER", out["countries"])
        self.assertNotIn("42", out["countries"])
        self.assertNotIn("fuel_status_addr", repr(out))

    def test_fuel_reader_raises_without_proc(self):
        ctx = reg.CaptureContext(proc=None)
        plug = {p.name: p for p in reg.default_registry()}["fuel_live"]
        with self.assertRaises(Exception):
            plug._reader(ctx, lambda p: {})

    def test_fuel_reader_raises_when_no_country_resolves(self):
        # read_fuel_live can resolve objects but have every fuel_status read fail (-> {}); that
        # is a broken read, not a measured-empty world -> the reader RAISES (domain omitted).
        plugs = {p.name: p for p in reg.default_registry()}

        class _FakeProc:
            pass
        ctx = reg.CaptureContext(proc=_FakeProc())
        with self.assertRaises(Exception):
            plugs["fuel_live"]._reader(ctx, lambda p: {})
        # also raises when only NON-TAG keys survive (nothing schema-valid resolved)
        with self.assertRaises(Exception):
            plugs["fuel_live"]._reader(
                ctx, lambda p: {"42": {"fuel": 1.0, "max_fuel": 2.0}})

    def test_supply_inputs_badged_alongside_proxy_untouched(self):
        # supply_inputs adds distinctly-named badged fields; if the proxy ("supply") also
        # captured, its by_province + confidence are NOT overwritten.
        res = self._full()
        res["supply"] = self._res("supply", {
            "by_province": {"100": {"current": 5.0, "capacity": 9.0}}, "count": 1})
        snap = asm.assemble_snapshot(res, game_id="g", date="1940.9.10.13",
                                     identity=FakeIdentity())
        self.assertEqual(schema.validate_snapshot(snap), [])
        sblk = snap["supply"]
        # proxy preserved with its own confidence
        self.assertEqual(sblk["by_province"], {"100": {"current": 5.0, "capacity": 9.0}})
        self.assertEqual(sblk["confidence"], "division_province_proxy")
        # solver inputs added with their OWN badge (not the proxy confidence)
        self.assertEqual(sblk["solver_efficiency_by_province"], {"3324": 0.95, "3544": 1.0})
        self.assertEqual(sblk["solver_draw_by_province"], {"3324": 12.0})
        self.assertEqual(sblk["solver_inputs_source"], "M")
        self.assertEqual(sblk["solver_inputs_confidence"],
                         "live_solver_input_not_exact_supply")
        for d in ("supply", "supply_inputs"):
            self.assertIn(d, snap["provenance"]["memory"]["domains"])

    def test_supply_inputs_without_proxy_stands_alone(self):
        # supply_inputs can fold even when the proxy "supply" domain did not capture.
        res = {"supply_inputs": self._full()["supply_inputs"]}
        snap = asm.assemble_snapshot(res, game_id="g", date="1940.9.10.13",
                                     identity=FakeIdentity())
        self.assertEqual(schema.validate_snapshot(snap), [])
        sblk = snap["supply"]
        self.assertEqual(sblk["solver_efficiency_by_province"], {"3324": 0.95, "3544": 1.0})
        self.assertEqual(sblk["solver_inputs_source"], "M")
        # the proxy by_province stays the null skeleton (no proxy captured)
        self.assertIsNone(sblk["by_province"])
        self.assertIn("supply_inputs", snap["provenance"]["memory"]["domains"])
        self.assertNotIn("supply", snap["provenance"]["memory"]["domains"])

    def test_supply_inputs_reader_raises_on_empty(self):
        # read_supply_inputs returns {} on a build mismatch / not-run -> the reader RAISES so
        # the domain is honestly omitted (not claimed captured-empty).
        plugs = {p.name: p for p in reg.default_registry()}

        class _FakeProc:
            pass
        ctx = reg.CaptureContext(proc=_FakeProc())
        with self.assertRaises(Exception):
            plugs["supply_inputs"]._reader(ctx, lambda p: {})

    def test_supply_inputs_reader_raises_on_meta_only(self):
        # a truthy result with ONLY meta (neither efficiency NOR draw a dict) is a broken read,
        # not a measured world -> the reader RAISES so the domain is honestly omitted (#3).
        plugs = {p.name: p for p in reg.default_registry()}

        class _FakeProc:
            pass
        ctx = reg.CaptureContext(proc=_FakeProc())
        with self.assertRaises(Exception):
            plugs["supply_inputs"]._reader(ctx, lambda p: {"meta": {"build": "sha"}})

    def test_supply_inputs_reader_stringifies_pids(self):
        plugs = {p.name: p for p in reg.default_registry()}

        class _FakeProc:
            pass
        ctx = reg.CaptureContext(proc=_FakeProc())

        def _fake_inputs(p):
            return {"efficiency": {3324: 0.95}, "draw": {3324: 1.0}, "meta": {}}
        out = plugs["supply_inputs"]._reader(ctx, _fake_inputs)
        self.assertEqual(out["efficiency_by_province"], {"3324": 0.95})
        self.assertEqual(out["draw_by_province"], {"3324": 1.0})

    def test_supply_inputs_reader_strips_heap_addr_from_meta(self):
        # the reader's meta carries a diagnostic heap addr (`csupplysystem` "0x..."); it must
        # be stripped before the data enters the snapshot (mirrors fuel_status_addr stripping).
        plugs = {p.name: p for p in reg.default_registry()}

        class _FakeProc:
            pass
        ctx = reg.CaptureContext(proc=_FakeProc())

        def _fake_inputs(p):
            return {"efficiency": {3324: 0.95}, "draw": {3324: 1.0},
                    "meta": {"csupplysystem": "0xDEADBEEF", "efficiency_count": 1,
                             "build": "sha"}}
        out = plugs["supply_inputs"]._reader(ctx, _fake_inputs)
        self.assertNotIn("csupplysystem", out["meta"])
        self.assertEqual(out["meta"]["efficiency_count"], 1)   # non-addr meta kept
        self.assertNotIn("0xDEADBEEF", repr(out))

    def test_absent_domains_stay_null_and_unclaimed(self):
        snap = asm.assemble_snapshot({}, game_id="g", date="1940.9.10.13",
                                     identity=FakeIdentity())
        self.assertEqual(schema.validate_snapshot(snap), [])
        doms = snap["provenance"]["memory"]["domains"]
        self.assertNotIn("fuel_live", doms)
        self.assertNotIn("supply_inputs", doms)
        # economy.fuel stays the {} skeleton; supply solver fields stay null
        self.assertEqual(snap.get("supply", {}).get("solver_efficiency_by_province"), None)


class TestDefnamesEnrichmentFailSoft(unittest.TestCase):
    """The in-plug defnames name-resolution is OPT-IN (ctx.extras['resolve_defnames']) and
    STRICTLY fail-soft: with resolution OFF (the default) the plugs return ONLY their raw
    handles -- identical to before this pass -- and never crash. NO live memory; drives the
    registry reader fns directly with fake impls."""

    def _ctx(self, **extras):
        class _FakeProc:
            pass
        return reg.CaptureContext(proc=_FakeProc(), extras=dict(extras))

    def test_resolution_off_by_default_tech_keeps_raw_handles_only(self):
        plugs = {p.name: p for p in reg.default_registry()}
        ctx = self._ctx()  # resolve_defnames not set -> OFF

        def _fake_tech(p, bridge=None, strict=True, **kw):
            return {"GER": {"completed_tech_count": 1, "research_slot_count": 1,
                            "active_research": [{"slot_index": 0, "tech_handle_raw": 0x1,
                                                 "tech_def_index_raw": 5,
                                                 "progress_raw": 10}]}}
        out = plugs["tech_live"]._reader(ctx, _wrap_tech(_fake_tech))
        slot = out["countries"]["GER"]["active_research"][0]
        self.assertEqual(slot["tech_def_index_raw"], 5)   # raw handle survives
        self.assertNotIn("tech_name", slot)               # no resolution attempted

    def test_resolution_off_by_default_politics_no_laws(self):
        plugs = {p.name: p for p in reg.default_registry()}
        ctx = self._ctx()

        def _fake_pol(p, bridge=None, strict=True, **kw):
            return {"GER": {"popularities": {"a": 1.0}, "ruling_party": "x",
                            "ideas": ["war_economy"], "idea_count": 1,
                            "election_frequency": 48, "party_count": 4,
                            "low_confidence": False}}
        out = plugs["politics_live"]._reader(ctx, _fake_pol)
        self.assertEqual(out["countries"]["GER"]["ideas"], ["war_economy"])
        self.assertNotIn("laws", out["countries"]["GER"])  # no classification attempted

    def test_defnames_enabled_but_import_fails_soft(self):
        # with resolution ON but defnames unimportable/failing, the plug STILL returns its
        # raw handles (names absent), never raising. We force the failure by making the
        # cached registry None (the import-guard path) and confirm no crash + no fake name.
        plugs = {p.name: p for p in reg.default_registry()}
        ctx = self._ctx(resolve_defnames=True)
        # pre-seed the per-capture cache so _defnames_registry returns None (simulates a
        # failed/missing build) without importing the real defnames module.
        ctx.extras["_defnames_cache"] = {"tech": None}

        def _fake_tech(p, bridge=None, strict=True, **kw):
            return {"GER": {"completed_tech_count": 1, "research_slot_count": 1,
                            "active_research": [{"slot_index": 0, "tech_handle_raw": 0x1,
                                                 "tech_def_index_raw": 5,
                                                 "progress_raw": 10}]}}
        out = plugs["tech_live"]._reader(ctx, _wrap_tech(_fake_tech))   # must NOT raise
        slot = out["countries"]["GER"]["active_research"][0]
        self.assertNotIn("tech_name", slot)   # unresolved -> no fabricated name

    def test_defnames_registry_cached_once_per_capture(self):
        # _defnames_registry builds at most once per capture (caches on ctx.extras). We
        # count builds via a fake defnames module installed in sys.modules.
        import sys as _sys
        import types as _types
        calls = {"n": 0}
        fake = _types.ModuleType("defnames_live")
        def _build(p):
            calls["n"] += 1
            return {0x1: "war_economy"}
        fake.build_tech_registry = _build
        old = _sys.modules.get("defnames_live")
        _sys.modules["defnames_live"] = fake
        try:
            ctx = self._ctx(resolve_defnames=True)
            r1 = reg._defnames_registry(ctx, "tech")
            r2 = reg._defnames_registry(ctx, "tech")
            self.assertIs(r1, r2)
            self.assertEqual(calls["n"], 1)   # built ONCE, second call hit the cache
        finally:
            if old is not None:
                _sys.modules["defnames_live"] = old
            else:
                _sys.modules.pop("defnames_live", None)

    def test_defnames_enabled_resolves_when_registry_known(self):
        # with a fake defnames installed + a known handle, the tech plug ADDS tech_name
        # ALONGSIDE the raw handles (additive, never replacing them).
        import sys as _sys
        import types as _types
        fake = _types.ModuleType("defnames_live")
        fake.build_tech_registry = lambda p: {5: "infantry_weapons"}
        fake.resolve_tech = lambda p, h, reg=None: (reg or {}).get(h)
        old = _sys.modules.get("defnames_live")
        _sys.modules["defnames_live"] = fake
        try:
            plugs = {p.name: p for p in reg.default_registry()}
            ctx = self._ctx(resolve_defnames=True)

            def _fake_tech(p, bridge=None, strict=True, **kw):
                return {"GER": {"completed_tech_count": 1, "research_slot_count": 1,
                                "active_research": [{"slot_index": 0,
                                                     "tech_handle_raw": 0x99,
                                                     "tech_def_index_raw": 5,
                                                     "progress_raw": 10}]}}
            out = plugs["tech_live"]._reader(ctx, _wrap_tech(_fake_tech))
            slot = out["countries"]["GER"]["active_research"][0]
            # raw handles preserved AND the resolved token added
            self.assertEqual(slot["tech_def_index_raw"], 5)
            self.assertEqual(slot["tech_name"], "infantry_weapons")
        finally:
            if old is not None:
                _sys.modules["defnames_live"] = old
            else:
                _sys.modules.pop("defnames_live", None)

    def test_mio_policies_partial_resolution_omitted(self):
        # a MIO with allowed_policies_count>0 whose handles resolve to a SHORTER list (some
        # unknown) -> `policies` OMITTED (incomplete; the count still conveys the real number).
        # A complete resolution (len == count) is emitted; a genuine count==0 emits [].
        import sys as _sys
        import types as _types
        fake = _types.ModuleType("defnames_live")
        fake.build_mio_policy_registry = lambda p: {1: "GFX_mio_policy_a"}
        # full record at id 10: count 2 but only 1 handle resolves -> partial -> omit
        # record at id 11: count 1 and 1 resolves -> complete -> emit
        # record at id 12: count 0 -> measured-empty -> emit []
        def _resolve(p, addr, reg=None):
            return {0xA10: ["GFX_mio_policy_a"],          # 1 of 2 -> partial
                    0xA11: ["GFX_mio_policy_a"],          # 1 of 1 -> complete
                    0xA12: []}.get(addr, [])
        fake.resolve_mio_policies_live = _resolve
        old = _sys.modules.get("defnames_live")
        _sys.modules["defnames_live"] = fake
        try:
            plugs = {p.name: p for p in reg.default_registry()}
            ctx = self._ctx(resolve_defnames=True)
            ctx.tag_table = _FakeTagTable({1: "GER"})

            def _fake_enum(p):
                return {10: {"id": 10, "owner_tag_db_id": 1, "allowed_policies_count": 2,
                             "addr": 0xA10},
                        11: {"id": 11, "owner_tag_db_id": 1, "allowed_policies_count": 1,
                             "addr": 0xA11},
                        12: {"id": 12, "owner_tag_db_id": 1, "allowed_policies_count": 0,
                             "addr": 0xA12}}
            out = plugs["mio"]._reader(ctx, (None, _fake_enum))
            by_id = out["by_id"]
            self.assertNotIn("policies", by_id["10"])        # partial -> omitted
            self.assertEqual(by_id["10"]["allowed_policies_count"], 2)  # count kept
            self.assertEqual(by_id["11"]["policies"], ["GFX_mio_policy_a"])  # complete
            self.assertEqual(by_id["12"]["policies"], [])    # measured-empty
            # addr never leaks
            self.assertNotIn("addr", repr(out))
        finally:
            if old is not None:
                _sys.modules["defnames_live"] = old
            else:
                _sys.modules.pop("defnames_live", None)

    def test_defnames_unknown_handle_no_fake_name(self):
        # a handle the registry does not know -> NO tech_name (never a synthetic name).
        import sys as _sys
        import types as _types
        fake = _types.ModuleType("defnames_live")
        fake.build_tech_registry = lambda p: {5: "infantry_weapons"}
        fake.resolve_tech = lambda p, h, reg=None: (reg or {}).get(h)
        old = _sys.modules.get("defnames_live")
        _sys.modules["defnames_live"] = fake
        try:
            plugs = {p.name: p for p in reg.default_registry()}
            ctx = self._ctx(resolve_defnames=True)

            def _fake_tech(p, bridge=None, strict=True, **kw):
                return {"GER": {"completed_tech_count": 1, "research_slot_count": 1,
                                "active_research": [{"slot_index": 0,
                                                     "tech_handle_raw": 0xDEAD,
                                                     "tech_def_index_raw": 0xBEEF,
                                                     "progress_raw": 10}]}}
            out = plugs["tech_live"]._reader(ctx, _wrap_tech(_fake_tech))
            slot = out["countries"]["GER"]["active_research"][0]
            self.assertNotIn("tech_name", slot)   # unknown handle -> no fabricated name
        finally:
            if old is not None:
                _sys.modules["defnames_live"] = old
            else:
                _sys.modules.pop("defnames_live", None)


class TestStateVPDomain(unittest.TestCase):
    """MOCK-based coverage for the LIVE per-state VICTORY-POINTS domain (state_vp ->
    states.*.victory_points_total + .victory_points_by_province, save-shared homes under
    the DISTINCT "state_vp" id). NO live memory -- the plug reader is driven with a fake
    StateVPLive + a stubbed state asset; the fold is exercised through assemble_snapshot
    with canned DomainResults. Asserts the measured ZERO survives, by_province REPLACES the
    save map (never key-merges), and the save backstop never overwrites a measured VP."""

    def _res(self, name, data, *, available=True, captured=True, error=None):
        return reg.DomainResult(name, available=available, captured=captured,
                                data=data, error=error)

    def _full(self):
        # ASSEMBLER-SHAPE data (already plug-processed): every enumerated state carries a
        # measured victory_points_total (incl. a measured 0) + a by_province map.
        return {"state_vp": self._res("state_vp", {"states": {
            "64": {"victory_points_total": 50,
                   "victory_points_by_province": {"1182": 50}},
            # measured ZERO state: total 0, empty by_province {} (measured, no VP province)
            "200": {"victory_points_total": 0,
                    "victory_points_by_province": {}},
            "7": {"victory_points_total": 90,
                  "victory_points_by_province": {"391": 40, "392": 50}},
        }, "count": 3, "n_with_vp": 2})}

    def test_state_vp_folds_and_validates(self):
        snap = asm.assemble_snapshot(self._full(), game_id="g",
                                     date="1940.9.10.13", identity=FakeIdentity())
        self.assertEqual(schema.validate_snapshot(snap), [])
        self.assertIn("state_vp", snap["provenance"]["memory"]["domains"])
        self.assertEqual(snap["states"]["64"]["victory_points_total"], 50)
        self.assertEqual(snap["states"]["64"]["victory_points_by_province"],
                         {"1182": 50})
        self.assertEqual(snap["states"]["7"]["victory_points_by_province"],
                         {"391": 40, "392": 50})

    def test_measured_zero_vp_survives_as_zero(self):
        snap = asm.assemble_snapshot(self._full(), game_id="g",
                                     date="1940.9.10.13", identity=FakeIdentity())
        # state 200 measured 0 -> 0, NOT null; by_province measured-empty {} kept
        self.assertEqual(snap["states"]["200"]["victory_points_total"], 0)
        self.assertEqual(snap["states"]["200"]["victory_points_by_province"], {})

    def test_measured_state_set_recorded_for_protection(self):
        snap = asm.assemble_snapshot(self._full(), game_id="g",
                                     date="1940.9.10.13", identity=FakeIdentity())
        m = snap.get("_live_measured_leaf_tags") or {}
        self.assertEqual(set(m.get("states.victory_points_total") or []),
                         {"64", "200", "7"})
        self.assertEqual(set(m.get("states.victory_points_by_province") or []),
                         {"64", "200", "7"})

    def test_absent_state_vp_stays_null_and_unclaimed(self):
        snap = asm.assemble_snapshot({}, game_id="g", date="1940.9.10.13",
                                     identity=FakeIdentity())
        self.assertEqual(schema.validate_snapshot(snap), [])
        self.assertNotIn("state_vp", snap["provenance"]["memory"]["domains"])
        # no states created by an absent VP fold
        self.assertEqual(snap["states"], {})

    def test_failed_state_vp_omitted(self):
        res = {"state_vp": self._res("state_vp", None, captured=False, error="boom")}
        snap = asm.assemble_snapshot(res, game_id="g", date="1940.9.10.13",
                                     identity=FakeIdentity())
        self.assertNotIn("state_vp", snap["provenance"]["memory"]["domains"])
        self.assertEqual(snap["states"], {})

    # -- save backstop: live VP WINS over a stale save (leaf-protected, replace-not-merge) --
    def test_save_backstop_cannot_overwrite_measured_vp(self):
        # mirror fold_save_bundle's _merge_state re-assert: a stale save VP total + a stale
        # save by_province for a state the live channel MEASURED must lose to live, and the
        # by_province must REPLACE (not key-merge) the save map.
        snap = asm.assemble_snapshot(self._full(), game_id="g",
                                     date="1940.9.10.13", identity=FakeIdentity())
        live_mem = snap["provenance"]["memory"]["domains"]
        self.assertIn("state_vp", live_mem)
        protected = []
        for dom in live_mem:
            protected.extend(asm._LIVE_STATE_DOMAIN_LEAVES.get(dom, ()))
        measured = {}
        for path, ids in (snap.get("_live_measured_leaf_tags") or {}).items():
            if path.startswith("states."):
                measured[path[len("states."):]] = set(ids or ())
        # a STALE save state: total 999, an extra stale province VP key
        save_state = schema.empty_state()
        save_state["victory_points_total"] = 999
        save_state["victory_points_by_province"] = {"1182": 999, "9999": 7}
        save_state["category"] = "city"  # save-only leaf -> should FILL (live didn't measure)
        merged = asm._merge_state(snap["states"]["64"], save_state, protected,
                                  sid="64", measured_leaf_tags=measured)
        # live total + live by_province WON (replace, not key-merge -> no stale 9999 key)
        self.assertEqual(merged["victory_points_total"], 50)
        self.assertEqual(merged["victory_points_by_province"], {"1182": 50})
        # the save-only category filled the gap (live left it null)
        self.assertEqual(merged["category"], "city")

    def test_save_only_state_keeps_save_vp(self):
        # a state the VP plug did NOT measure (save-only id) keeps the save VP: the live
        # leaf is None for it, so _merge_state does not re-assert.
        protected = list(asm._LIVE_STATE_DOMAIN_LEAVES["state_vp"])
        measured = {"victory_points_total": {"64", "200", "7"},
                    "victory_points_by_province": {"64", "200", "7"}}
        # state "500" was never measured live -> a fresh skeleton (all None)
        live_unmeasured = schema.empty_state()
        save_state = schema.empty_state()
        save_state["victory_points_total"] = 25
        save_state["victory_points_by_province"] = {"700": 25}
        merged = asm._merge_state(live_unmeasured, save_state, protected,
                                  sid="500", measured_leaf_tags=measured)
        # save VP kept (live never measured this state)
        self.assertEqual(merged["victory_points_total"], 25)
        self.assertEqual(merged["victory_points_by_province"], {"700": 25})

    # -- reader-level: full-asset enumeration emits a measured 0 for zero-VP states --
    def test_reader_enumerates_full_asset_emits_measured_zero(self):
        class _FakeVP:
            @staticmethod
            def resolve(p):
                return _FakeVP()

            def as_province_map(self):
                return {1182: 50, 391: 40}

            def as_state_map(self):
                # as_state_map is the per-province SUM (reader invariant): state 64 -> 50
                # (prov 1182), state 7 -> 40 (prov 391); state 200 has no VP province -> 0.
                return {64: 50, 7: 40}

        fake_p2s = {64: [1182, 9999], 7: [391], 200: [5000, 5001]}
        old = reg._load_state_provinces
        reg._load_state_provinces = lambda: dict(fake_p2s)
        try:
            ctx = reg.CaptureContext(proc=object())
            out = reg._r_state_vp(ctx, _FakeVP)
        finally:
            reg._load_state_provinces = old
        st = out["states"]
        # EVERY enumerated state carries a measured total (a 0 for the zero-VP ones)
        self.assertEqual(st["64"]["victory_points_total"], 50)
        self.assertEqual(st["64"]["victory_points_by_province"], {"1182": 50})
        self.assertEqual(st["7"]["victory_points_total"], 40)
        self.assertEqual(st["7"]["victory_points_by_province"], {"391": 40})
        # state 200 has provinces but none carry VP -> measured 0 + empty {} map
        self.assertEqual(st["200"]["victory_points_total"], 0)
        self.assertEqual(st["200"]["victory_points_by_province"], {})
        self.assertEqual(out["n_with_vp"], 2)

    def test_reader_raises_without_proc(self):
        ctx = reg.CaptureContext(proc=None)

        class _FakeVP:
            @staticmethod
            def resolve(p):
                return object()
        with self.assertRaises(Exception):
            reg._r_state_vp(ctx, _FakeVP)

    def test_reader_raises_when_resolve_fails(self):
        # fail-closed: a None resolve (build mismatch / ambiguous relocation) -> raise so
        # the domain is honestly omitted, never a fabricated zero-VP world.
        class _FailVP:
            @staticmethod
            def resolve(p):
                return None
        ctx = reg.CaptureContext(proc=object())
        with self.assertRaises(Exception):
            reg._r_state_vp(ctx, _FailVP)

    def test_reader_raises_without_state_asset(self):
        # no state asset -> cannot emit a measured zero per state -> raise (no partial).
        class _FakeVP:
            @staticmethod
            def resolve(p):
                return _FakeVP()

            def as_province_map(self):
                return {1182: 50}

            def as_state_map(self):
                return {64: 50}
        old = reg._load_state_provinces
        reg._load_state_provinces = lambda: {}
        try:
            ctx = reg.CaptureContext(proc=object())
            with self.assertRaises(Exception):
                reg._r_state_vp(ctx, _FakeVP)
        finally:
            reg._load_state_provinces = old


class TestWeatherPrecipWire(unittest.TestCase):
    """MOCK-based coverage for the LIVE per-PROVINCE weather fold (_wire_weather ->
    weather.provinces.{pid}.{temp_c,precip,snow}, M). NO live memory -- the
    memory_reader.weather.weather_temp_model module is stubbed via sys.modules so
    assemble_snapshot's _wire_weather runs offline. Asserts temp+precip+snow fold at
    per-province granularity, a None precip stays None (no fabricated 0.0), the region
    block stays null (retracted reader), and validate_snapshot passes."""

    def _install_fake_weather(self, tprov, pprov):
        import types as _types
        import sys as _sys
        fake = _types.ModuleType("memory_reader.weather.weather_temp_model")
        fake.read_province_temperatures = lambda pid=None: {
            "provinces": dict(tprov), "n_with_temp": sum(
                1 for r in tprov.values() if r.get("temp_c") is not None),
            "temp_min_c": 5.0, "temp_max_c": 25.0, "temp_mean_c": 15.0}
        fake.read_province_precipitation = lambda pid=None: ({
            "provinces": dict(pprov),
            "n_with_water": sum(1 for r in pprov.values()
                                if r.get("water") is not None),
            "n_with_snow": sum(1 for r in pprov.values()
                               if r.get("snow") is not None),
            "water_max": 2.5, "water_mean": 1.0} if pprov is not None else None)
        # ensure the parent packages exist so the dotted import resolves
        for name in ("memory_reader", "memory_reader.weather"):
            if name not in _sys.modules:
                pkg = _types.ModuleType(name)
                pkg.__path__ = []  # mark as package
                _sys.modules[name] = pkg
        # `from memory_reader.weather import weather_temp_model` binds the submodule via
        # BOTH sys.modules AND the parent package's attribute -> stub both so the import
        # resolves to the fake whether the real package was already imported or not.
        self._old_mod = _sys.modules.get(
            "memory_reader.weather.weather_temp_model")
        _pkg = _sys.modules["memory_reader.weather"]
        self._had_attr = hasattr(_pkg, "weather_temp_model")
        self._old_attr = getattr(_pkg, "weather_temp_model", None)
        _sys.modules["memory_reader.weather.weather_temp_model"] = fake
        setattr(_pkg, "weather_temp_model", fake)

    def _restore(self):
        import sys as _sys
        if getattr(self, "_old_mod", None) is not None:
            _sys.modules["memory_reader.weather.weather_temp_model"] = self._old_mod
        else:
            _sys.modules.pop("memory_reader.weather.weather_temp_model", None)
        _pkg = _sys.modules.get("memory_reader.weather")
        if _pkg is not None:
            if self._had_attr:
                setattr(_pkg, "weather_temp_model", self._old_attr)
            elif hasattr(_pkg, "weather_temp_model"):
                delattr(_pkg, "weather_temp_model")

    def test_weather_province_temp_precip_snow_fold(self):
        tprov = {
            1896: {"temp_c": 20.10, "temp_change_c": 1.48},
            3544: {"temp_c": 18.17, "temp_change_c": -0.12},
            # sea/inactive province: temp_c None -> kept None (not a fab 0 C)
            9999: {"temp_c": None, "temp_change_c": None},
        }
        pprov = {
            1896: {"water": 1.25, "snow": None},     # measured precip, no snow
            3544: {"water": None, "snow": 0.40},     # no precip, measured snow
            9999: {"water": None, "snow": None},     # neither
        }
        self._install_fake_weather(tprov, pprov)
        try:
            snap = asm.assemble_snapshot({}, game_id="g", date="1940.9.10.13",
                                         identity=FakeIdentity())
        finally:
            self._restore()
        self.assertEqual(schema.validate_snapshot(snap), [])
        self.assertIn("weather", snap["provenance"]["memory"]["domains"])
        w = snap["weather"]
        self.assertEqual(w["source"], "M")
        # province block populated at per-province granularity
        self.assertEqual(w["provinces"]["1896"]["temp_c"], 20.10)
        self.assertEqual(w["provinces"]["1896"]["precip"], 1.25)
        # snow None stays None (NOT fabricated to 0.0)
        self.assertIsNone(w["provinces"]["1896"]["snow"])
        self.assertIsNone(w["provinces"]["3544"]["precip"])
        self.assertEqual(w["provinces"]["3544"]["snow"], 0.40)
        # inactive province: temp/precip/snow all None (measured-absent, never fab 0)
        self.assertIsNone(w["provinces"]["9999"]["temp_c"])
        self.assertIsNone(w["provinces"]["9999"]["precip"])
        # the per-province province_id is the int id
        self.assertEqual(w["provinces"]["1896"]["province_id"], 1896)
        # rollup carries the precip tallies
        self.assertEqual(w["global_rollup"]["n_with_precip"], 1)
        self.assertEqual(w["global_rollup"]["n_with_snow"], 1)
        # region block left null (retracted per-region reader; no stale merge)
        self.assertIsNone(w["regions"])

    def test_weather_precip_read_failure_still_claims_on_temp(self):
        # precip is ADDITIVE: a precip-read failure (read_province_precipitation -> None)
        # must NOT sink the temperature evidence; precip/snow fold as None.
        tprov = {1896: {"temp_c": 20.10, "temp_change_c": 1.48}}
        self._install_fake_weather(tprov, None)   # precip reader returns None
        try:
            snap = asm.assemble_snapshot({}, game_id="g", date="1940.9.10.13",
                                         identity=FakeIdentity())
        finally:
            self._restore()
        self.assertEqual(schema.validate_snapshot(snap), [])
        self.assertIn("weather", snap["provenance"]["memory"]["domains"])
        w = snap["weather"]
        self.assertEqual(w["provinces"]["1896"]["temp_c"], 20.10)
        self.assertIsNone(w["provinces"]["1896"]["precip"])
        self.assertIsNone(w["provinces"]["1896"]["snow"])
        # rollup precip tallies are UNMEASURED -> None (not a fabricated 0 implying
        # "measured, none found"); temp tallies still concrete (Codex final review).
        self.assertIsNone(w["global_rollup"]["n_with_precip"])
        self.assertIsNone(w["global_rollup"]["n_with_snow"])
        self.assertNotIn("precip_max", w["global_rollup"])
        self.assertIsNotNone(w["global_rollup"]["n_with_temp"])

    def test_weather_absent_when_temp_read_empty(self):
        # no per-province temp array -> weather not claimed (no fabrication).
        self._install_fake_weather({}, {})
        try:
            snap = asm.assemble_snapshot({}, game_id="g", date="1940.9.10.13",
                                         identity=FakeIdentity())
        finally:
            self._restore()
        self.assertEqual(schema.validate_snapshot(snap), [])
        self.assertNotIn("weather", snap["provenance"]["memory"]["domains"])
        self.assertIsNone(snap["weather"]["provinces"])
        self.assertIsNone(snap["weather"]["source"])


# ==========================================================================
# PERF FIX: single identity resolution (Stage 1) + the pin extension (Stage 2)
# + arena-restrict range selection (Stage 3) + the locate-abort bugfix.
# All mockable WITHOUT a live game.
# ==========================================================================
class _FpIdentity:
    """Minimal ProcessIdentity for ctx.proc_fp() (the pin epoch key)."""
    def __init__(self, pid=4321, ct=111, sha="sha-A"):
        self.pid = pid
        self.create_time_100ns = ct
        self.exe_sha256 = sha


class _MemProc:
    """A tiny fake process: a {addr: qword} memory map + module_base/pid, with u64/u32/i32 reads.
    Lets the pin's validate/reread paths run deterministically with no live game."""
    def __init__(self, mem=None, module_base=0x1000, pid=4321, module_size=0x4000000):
        self.mem = dict(mem or {})
        self.module_base = module_base
        self.module_size = module_size
        self.pid = pid

    def u64(self, a):
        if a not in self.mem:
            raise OSError("unmapped 0x%X" % a)
        return self.mem[a] & 0xFFFFFFFFFFFFFFFF

    def u32(self, a):
        if a not in self.mem:
            raise OSError("unmapped 0x%X" % a)
        return self.mem[a] & 0xFFFFFFFF

    def i32(self, a):
        v = self.u32(a)
        return v - 0x100000000 if v >= 0x80000000 else v


class _FakeBridgeObj:
    """A CountryIndexBridge stand-in: stable as_map + tag + country_object + rec0/count +
    revalidate, all driven by a plain dict so tests control agreement/liveness."""
    def __init__(self, id_to_tag, id_to_obj, rec0=0xAA00, count=88, live=True):
        self._id_to_tag = dict(id_to_tag)
        self._id_to_obj = dict(id_to_obj)
        self.rec0 = rec0
        self.count = count
        self._live = live

    def as_map(self):
        return {tid: i for i, tid in enumerate(sorted(self._id_to_tag))}

    def tag(self, tid):
        return self._id_to_tag.get(tid)

    def country_object(self, tid):
        return self._id_to_obj.get(tid)

    def revalidate(self):
        return self._live


class _FakeIdentityObj:
    """A CountryIdentity stand-in mirroring the real one's surface (tag/tag_db_id/object_of/
    tags/population_token/revalidate)."""
    def __init__(self, id_to_tag, id_to_obj, live=True):
        self._id_to_tag = dict(id_to_tag)
        self._tag_to_id = {v: k for k, v in id_to_tag.items()}
        self._id_to_obj = dict(id_to_obj)
        self._tag_to_obj = {id_to_tag[t]: o for t, o in id_to_obj.items() if t in id_to_tag}
        self._live = live

    def tag(self, tid):
        return self._id_to_tag.get(tid)

    def tag_db_id(self, tag):
        return self._tag_to_id.get(tag)

    def object_of(self, tag):
        return self._tag_to_obj.get(tag)

    def tags(self):
        return list(self._tag_to_id)

    def valid_ids(self):
        return set(self._id_to_tag)

    def population_token(self):
        return frozenset((t, v) for t, v in self._id_to_tag.items())

    def revalidate(self, p):
        return self._live


class TestStage1SingleIdentity(unittest.TestCase):
    """Stage 1: the THREADED shared identity/bridge resolves the SAME identities/tags as the
    INDEPENDENT per-reader path, and a BAD/DISAGREEING identity is never threaded (fail-closed)."""

    def _ctx(self, bridge=None, proc=None):
        return reg.CaptureContext(proc=proc or _MemProc(), identity=_FpIdentity(),
                                  bridge=bridge, extras={"pins": {}})

    def test_identity_agrees_with_bridge_accepts_consistent(self):
        idmap = {i: "T%02d" % i for i in range(1, 9)}
        objmap = {i: 0x10000 + i for i in range(1, 9)}
        br = _FakeBridgeObj(idmap, objmap)
        idn = _FakeIdentityObj(idmap, objmap)
        self.assertTrue(reg._identity_agrees_with_bridge(_MemProc(), idn, br))

    def test_identity_agrees_with_bridge_rejects_tag_disagreement(self):
        idmap = {i: "T%02d" % i for i in range(1, 9)}
        objmap = {i: 0x10000 + i for i in range(1, 9)}
        br = _FakeBridgeObj(idmap, objmap)
        bad = dict(idmap); bad[3] = "XXX"            # same id, different TAG -> disagree
        idn = _FakeIdentityObj(bad, objmap)
        self.assertFalse(reg._identity_agrees_with_bridge(_MemProc(), idn, br))

    def test_identity_agrees_with_bridge_rejects_object_disagreement(self):
        idmap = {i: "T%02d" % i for i in range(1, 9)}
        objmap = {i: 0x10000 + i for i in range(1, 9)}
        br = _FakeBridgeObj(idmap, objmap)
        badobj = dict(objmap); badobj[5] = 0x99999   # same id+TAG, different object -> disagree
        idn = _FakeIdentityObj(idmap, badobj)
        self.assertFalse(reg._identity_agrees_with_bridge(_MemProc(), idn, br))

    def test_country_identity_caches_and_fails_closed_on_disagreement(self):
        # monkeypatch diplomacy_live.CountryIdentity.resolve to return a DISAGREEING identity ->
        # ctx.country_identity() must cache None (never thread a bad identity).
        import diplomacy_live as dl
        idmap = {i: "T%02d" % i for i in range(1, 9)}
        objmap = {i: 0x10000 + i for i in range(1, 9)}
        br = _FakeBridgeObj(idmap, objmap)
        bad = dict(idmap); bad[2] = "ZZZ"
        orig = dl.CountryIdentity.resolve
        dl.CountryIdentity.resolve = staticmethod(
            lambda p, bridge=None: _FakeIdentityObj(bad, objmap))
        try:
            ctx = self._ctx(bridge=br)
            self.assertIsNone(ctx.country_identity())     # disagreement -> None (fail closed)
            self.assertTrue(ctx._identity_full_done)       # remembered (not retried)
        finally:
            dl.CountryIdentity.resolve = orig

    def test_country_identity_threads_agreeing_identity_once(self):
        # an AGREEING identity is cached + returned; a second call does NOT re-resolve.
        import diplomacy_live as dl
        idmap = {i: "T%02d" % i for i in range(1, 9)}
        objmap = {i: 0x10000 + i for i in range(1, 9)}
        br = _FakeBridgeObj(idmap, objmap)
        calls = {"n": 0}
        def _res(p, bridge=None):
            calls["n"] += 1
            return _FakeIdentityObj(idmap, objmap)
        orig = dl.CountryIdentity.resolve
        dl.CountryIdentity.resolve = staticmethod(_res)
        try:
            ctx = self._ctx(bridge=br)
            a = ctx.country_identity()
            b = ctx.country_identity()
            self.assertIsNotNone(a)
            self.assertIs(a, b)                            # cached: same object
            self.assertEqual(calls["n"], 1)                # resolved exactly ONCE (single resolution)
            # the threaded identity exposes the SAME tag set as an independent resolve would
            indep = _res(ctx.proc)
            self.assertEqual(set(a.tags()), set(indep.tags()))
        finally:
            dl.CountryIdentity.resolve = orig

    def test_bridge_idx_tag_supplies_rec0_count_together(self):
        idmap = {i: "T%02d" % i for i in range(1, 9)}
        objmap = {i: 0x10000 + i for i in range(1, 9)}
        br = _FakeBridgeObj(idmap, objmap, rec0=0xBEEF00, count=77)
        # _bridge_idx_tag names via ctx.resolve_tag (the tag table); supply a matching fake table.
        class _Tbl:
            def __init__(self, m): self._m = m
            def tag(self, tid): return self._m.get(tid)
        ctx = reg.CaptureContext(proc=_MemProc(), identity=_FpIdentity(), bridge=br,
                                 tag_table=_Tbl(idmap), extras={"pins": {}})
        idx_tag, rec0, count = reg._bridge_idx_tag(ctx)
        self.assertEqual(rec0, 0xBEEF00)
        self.assertEqual(count, 77)
        # idx_tag maps running_index -> TAG (the form the PP-bridge readers expect)
        self.assertTrue(all(isinstance(k, int) and isinstance(v, str)
                            for k, v in idx_tag.items()))
        # no bridge -> all None (reader self-locates)
        ctx2 = self._ctx(bridge=None)
        self.assertEqual(reg._bridge_idx_tag(ctx2), (None, None, None))


class TestStage1OffenderThreading(unittest.TestCase):
    """Stage 1: the offender readers (fuel/construction/decisions) FORWARD the shared bridge and
    self-resolve (bridge=None) when it is absent -- output-identical, fail-closed."""

    def test_fuel_reader_forwards_bridge_and_falls_back(self):
        seen = []
        def _fake_fuel(p, bridge=None):
            seen.append(bridge)
            return {"GER": {"fuel": 1.0, "max_fuel": 2.0}}
        # with a bridge present, it is threaded
        idmap = {i: "T%02d" % i for i in range(1, 9)}
        objmap = {i: 0x10000 + i for i in range(1, 9)}
        br = _FakeBridgeObj(idmap, objmap)
        ctx = reg.CaptureContext(proc=_MemProc(), identity=_FpIdentity(), bridge=br,
                                 extras={"pins": {}})
        plugs = {p.name: p for p in reg.default_registry()}
        out = plugs["fuel_live"]._reader(ctx, _fake_fuel)
        self.assertIn("GER", out["countries"])
        self.assertIs(seen[-1], br)                        # the SHARED bridge was threaded
        # with NO bridge, the reader is called with bridge=None (self-resolve, no fabrication)
        ctx2 = reg.CaptureContext(proc=_MemProc(), identity=_FpIdentity(), bridge=None,
                                  extras={"pins": {}})
        plugs["fuel_live"]._reader(ctx2, _fake_fuel)
        self.assertIsNone(seen[-1])


class TestStage2Pin(unittest.TestCase):
    """Stage 2: _pin_get hit / miss / invalidate, and the newly-pinned readers' resolve/validate/
    reread callbacks -- all with mocks. Proves the pin is a CACHE that never serves a stale root."""

    def _ctx(self):
        return reg.CaptureContext(proc=_MemProc(), identity=_FpIdentity(), extras={"pins": {}})

    def test_pin_get_hit_skips_rescan(self):
        ctx = self._ctx()
        scans = {"n": 0}
        rereads = {"n": 0}
        def _resolve():
            scans["n"] += 1
            return {"root": 1}, "DATA0", 1
        def _validate(roots, pop):
            return True
        def _reread(roots):
            rereads["n"] += 1
            return "DATA_WARM"
        self.assertEqual(reg._pin_get(ctx, "k", _resolve, _validate, _reread), "DATA0")
        self.assertEqual(scans["n"], 1)
        # warm walk: validate passes -> reread, NO rescan
        self.assertEqual(reg._pin_get(ctx, "k", _resolve, _validate, _reread), "DATA_WARM")
        self.assertEqual(scans["n"], 1)
        self.assertEqual(rereads["n"], 1)

    def test_pin_get_miss_rescans(self):
        ctx = self._ctx()
        scans = {"n": 0}
        def _resolve():
            scans["n"] += 1
            return {"root": 1}, "DATA%d" % scans["n"], 1
        def _reread(roots):
            return "WARM"
        # validate ALWAYS fails -> every walk rescans (fail-closed)
        reg._pin_get(ctx, "k", _resolve, lambda r, p: False, _reread)
        reg._pin_get(ctx, "k", _resolve, lambda r, p: False, _reread)
        self.assertEqual(scans["n"], 2)

    def test_pin_get_invalidates_on_fp_change(self):
        ctx = self._ctx()
        scans = {"n": 0}
        def _resolve():
            scans["n"] += 1
            return {"root": 1}, "D", 1
        reg._pin_get(ctx, "k", _resolve, lambda r, p: True, lambda r: "W")
        self.assertEqual(scans["n"], 1)
        # change the process fingerprint (new pid) -> the WHOLE pin cache is cleared -> rescan
        ctx.identity = _FpIdentity(pid=9999)
        reg._pin_get(ctx, "k", _resolve, lambda r, p: True, lambda r: "W")
        self.assertEqual(scans["n"], 2)

    def test_pin_get_reread_exception_falls_back_to_rescan(self):
        # a reread that raises AFTER validate passed must DROP the pin + rescan (never omit/stale).
        ctx = self._ctx()
        scans = {"n": 0}
        def _resolve():
            scans["n"] += 1
            return {"root": 1}, "FRESH", 1
        def _reread(roots):
            raise RuntimeError("root became unreadable mid-walk")
        reg._pin_get(ctx, "k", _resolve, lambda r, p: True, _reread)
        reg._pin_get(ctx, "k", _resolve, lambda r, p: True, _reread)
        self.assertEqual(scans["n"], 2)                    # second walk fell back to a full scan

    def test_pin_ships_validate_catches_aba_and_move(self):
        # build a fake CombatReader + OobShipRegistry; the pin's validate must reject a moved
        # vtable AND an ABA instance-id swap at a pinned body, forcing a rescan.
        import oob_full_live as _oob
        base = 0x1000
        cship_vt = base + _oob.CSHIP_VT_OFF
        # one ship: body @ 0x5000, handle = (iid<<32 | 51)
        body = 0x5000
        iid = 7
        good_mem = {body: cship_vt, body + _oob.OFF_HANDLE: (iid << 32) | _oob.SHIP_TYPE_TAG}

        class _Reader:
            module_base = base
            def __init__(self, mem):
                self.mem = mem
            def u64(self, a):
                return self.mem.get(a)
            def verify_build(self):
                return True, "sha"
            def close(self):
                pass

        class _Reg:
            def __init__(self, reader):
                self.r = reader
                self._index = None
            def build_index(self):
                self._index = {iid: body}
                return self._index
            def read_ship(self, i):
                return {"instance_id": i, "definition": "x", "category": "destroyer",
                        "strength_hp": 1.0, "organisation": 1.0, "experience": 0.0,
                        "sunk_convoys": 0}

        mem_holder = {"mem": dict(good_mem)}
        def _CombatReader():
            return _Reader(mem_holder["mem"])
        # patch _verify_same_process to a no-op (the fake reader has no pid)
        orig_vsp = reg._verify_same_process
        reg._verify_same_process = lambda ctx, reader: None
        try:
            ctx = reg.CaptureContext(proc=_MemProc(module_base=base), identity=_FpIdentity(),
                                     extras={"pins": {}})
            out = reg._pin_ships(ctx, _CombatReader, _Reg)
            self.assertIn(iid, out)
            # ABA: same vtable but the instance id at the handle changes -> validate False -> rescan
            mem_holder["mem"][body + _oob.OFF_HANDLE] = (999 << 32) | _oob.SHIP_TYPE_TAG
            out2 = reg._pin_ships(ctx, _CombatReader, _Reg)   # rescans (build_index re-pins iid=7)
            self.assertIn(iid, out2)
        finally:
            reg._verify_same_process = orig_vsp

    def test_pin_country_reader_revalidate_miss_reresolves_identity(self):
        # Codex finding 1: after a validate MISS the shared identity must be RE-RESOLVED fresh,
        # not served from the ctx cache. Drive a country-set pin whose identity goes "dead".
        import diplomacy_live as dl
        idmap = {i: "T%02d" % i for i in range(1, 9)}
        objmap = {i: 0x10000 + i for i in range(1, 9)}
        br = _FakeBridgeObj(idmap, objmap)
        state = {"live": True, "resolves": 0}
        def _res(p, bridge=None):
            state["resolves"] += 1
            idn = _FakeIdentityObj(idmap, objmap)
            idn.revalidate = lambda p: state["live"]   # liveness tracks the MUTABLE flag
            return idn
        orig = dl.CountryIdentity.resolve
        dl.CountryIdentity.resolve = staticmethod(_res)
        try:
            ctx = reg.CaptureContext(proc=_MemProc(), identity=_FpIdentity(), bridge=br,
                                     extras={"pins": {}})
            reads = {"n": 0}
            out1 = reg._pin_country_reader(
                ctx, "diplomacy",
                lambda roots: (reads.__setitem__("n", reads["n"] + 1) or {"ok": True}),
                need_identity=True)
            self.assertEqual(out1, {"ok": True})
            self.assertEqual(state["resolves"], 1)         # resolved ONCE (cold)
            # warm walk with the identity still LIVE -> validate passes -> reread, NO re-resolve
            reg._pin_country_reader(
                ctx, "diplomacy", lambda roots: {"ok": True}, need_identity=True)
            self.assertEqual(state["resolves"], 1)
            # now the pinned identity goes DEAD -> validate fails -> cache cleared -> re-resolve
            state["live"] = False
            reg._pin_country_reader(
                ctx, "diplomacy", lambda roots: {"ok": True}, need_identity=True)
            self.assertGreaterEqual(state["resolves"], 2)  # genuinely re-resolved (not stale cache)
        finally:
            dl.CountryIdentity.resolve = orig

    def test_pin_supply_inputs_singleton_validate(self):
        # the css singleton pin: validate re-confirms the double-vtable + province-count gate.
        import supply_solver_live as _ss
        base = 0x2000
        css = 0x8000
        vt = base + _ss.CSS_VT_RVA[0]
        effb = _ss.HEAP_LO + 0x100
        mem = {
            css: vt, css + 8: vt,
            css + _ss.CSS_PROV_EFF: effb, css + _ss.CSS_PROV_EFF + 8: 13414,
            css + _ss.CSS_PROV_DRAW + 8: 13414,
        }
        proc = _MemProc(mem, module_base=base)
        ctx = reg.CaptureContext(proc=proc, identity=_FpIdentity(), extras={"pins": {}})
        calls = {"n": 0}
        def _read_inputs(p, css=None):
            calls["n"] += 1
            return {"efficiency": {3324: 0.95}, "meta": {"csupplysystem": "0x%X" % (css or 0)}}
        orig_find = _ss._find_csupplysystem
        _ss._find_csupplysystem = lambda p: css
        try:
            r1 = reg._pin_supply_inputs(ctx, _read_inputs)
            self.assertIn("efficiency", r1)
            # warm walk: validate passes (mem unchanged) -> reread off pinned css, NO new locate
            find_calls = {"n": 0}
            def _find2(p):
                find_calls["n"] += 1
                return css
            _ss._find_csupplysystem = _find2
            reg._pin_supply_inputs(ctx, _read_inputs)
            self.assertEqual(find_calls["n"], 0)            # css scan SKIPPED on the warm walk
            # break the double-vtable IN THE PROC's view -> validate fails -> rescan (find again)
            proc.mem[css + 8] = 0xDEAD
            reg._pin_supply_inputs(ctx, _read_inputs)
            self.assertEqual(find_calls["n"], 1)
        finally:
            _ss._find_csupplysystem = orig_find


class TestStage2DivisionsSplit(unittest.TestCase):
    """Stage 2: the division reader's locate/read split is OUTPUT-IDENTICAL to the monolithic
    enumerate (same dedup, order, decode)."""

    def test_split_matches_monolithic(self):
        import struct
        from division_live.division_live_reader import DivisionLiveReader, DIV_VT_OFF
        # craft a tiny in-memory CDivision pool with 2 distinct ids + 1 duplicate (first wins).
        base = 0x10000
        div_vt = base + DIV_VT_OFF
        BODY = 0x620
        bodies = {0x40000: 1, 0x41000: 2, 0x42000: 1}   # addr -> div_id (0x42000 dup of id 1)

        class _R(DivisionLiveReader):
            def __init__(self):
                self.module_base = base
                self.div_vt = div_vt
                self.move_order_vt = base + 0x1
            def _enum_regions(self):
                return [(0x40000, 0x3000)]
            def _rpm(self, addr, n):
                # one region blob holding the three vtable needles at their offsets
                out = bytearray(0x3000)
                for a in bodies:
                    struct.pack_into("<Q", out, a - 0x40000, div_vt)
                return bytes(out[: n]) if addr == 0x40000 else b""
            def _read_clamped(self, addr, n):
                did = bodies.get(addr)
                if did is None:
                    return b""
                b = bytearray(max(n, BODY))
                struct.pack_into("<I", b, 0x08, 4713)      # type
                struct.pack_into("<I", b, 0x0C, did)       # id
                return bytes(b[:n])

        r = _R()
        mono = r.enumerate_divisions()
        split = r.read_divisions_from_bodies(r.locate_division_bodies())
        self.assertEqual([d["id"] for d in mono], [d["id"] for d in split])
        self.assertEqual([d["id"] for d in mono], [1, 2])  # first-wins dedup, scan order preserved


class TestStage3ArenaRestrict(unittest.TestCase):
    """Stage 3: the range-selection logic, now DEFAULT ON (flag default ON -> arena bounds;
    explicit-disable -> wide; fail-open to wide on discover_arena failure). Mockable, no game."""

    def test_default_on_uses_arena_bounds(self):
        # Stage 3 FLIP: with NO override, arena-restrict is ON and scan_range uses the discovered
        # arena bounds (the live coverage-diff proved the SAME roots are found in the tight arena).
        import fastscan
        old = os.environ.pop("CAPTURE_ARENA_RESTRICT", None)
        ctx = reg.CaptureContext(proc=_MemProc(pid=4321), identity=_FpIdentity(), extras={})
        orig = fastscan.discover_arena
        fastscan.discover_arena = lambda pid, gap=0x100000000: (0xAAAA0000, 0xBBBB0000)
        try:
            self.assertTrue(ctx.arena_restrict_enabled())
            self.assertEqual(ctx.scan_range(), (0xAAAA0000, 0xBBBB0000))
            self.assertEqual(ctx.extras["_arena_bounds"], (0xAAAA0000, 0xBBBB0000))
        finally:
            fastscan.discover_arena = orig
            if old is not None:
                os.environ["CAPTURE_ARENA_RESTRICT"] = old

    def test_explicit_disable_via_extras_is_wide(self):
        # arena_restrict=False in extras forces OFF -> the wide 47-bit window (no discover call).
        ctx = reg.CaptureContext(proc=_MemProc(), identity=_FpIdentity(),
                                 extras={"arena_restrict": False})
        old = os.environ.pop("CAPTURE_ARENA_RESTRICT", None)
        try:
            self.assertFalse(ctx.arena_restrict_enabled())
            self.assertEqual(ctx.scan_range(), (0x10000, 0x7FFFFFFFFFFF))
        finally:
            if old is not None:
                os.environ["CAPTURE_ARENA_RESTRICT"] = old

    def test_flag_on_uses_arena_bounds(self):
        import fastscan
        ctx = reg.CaptureContext(proc=_MemProc(pid=4321), identity=_FpIdentity(),
                                 extras={"arena_restrict": True})
        orig = fastscan.discover_arena
        fastscan.discover_arena = lambda pid, gap=0x100000000: (0xAAAA0000, 0xBBBB0000)
        try:
            self.assertTrue(ctx.arena_restrict_enabled())
            self.assertEqual(ctx.scan_range(), (0xAAAA0000, 0xBBBB0000))
            # discovered once + cached
            self.assertEqual(ctx.extras["_arena_bounds"], (0xAAAA0000, 0xBBBB0000))
        finally:
            fastscan.discover_arena = orig

    def test_flag_on_but_discover_fails_falls_back_to_wide(self):
        import fastscan
        ctx = reg.CaptureContext(proc=_MemProc(pid=4321), identity=_FpIdentity(),
                                 extras={"arena_restrict": True})
        orig = fastscan.discover_arena
        fastscan.discover_arena = lambda pid, gap=0x100000000: None
        try:
            # fail-OPEN to the wide window (a wider scan only finds MORE roots, never a fabrication)
            self.assertEqual(ctx.scan_range(), (0x10000, 0x7FFFFFFFFFFF))
        finally:
            fastscan.discover_arena = orig

    def test_env_var_toggles_flag(self):
        # an explicit disable token via the env var forces OFF even though the default is ON.
        ctx = reg.CaptureContext(proc=_MemProc(), identity=_FpIdentity(), extras={})
        os.environ["CAPTURE_ARENA_RESTRICT"] = "1"
        try:
            self.assertTrue(ctx.arena_restrict_enabled())
        finally:
            os.environ.pop("CAPTURE_ARENA_RESTRICT", None)
        os.environ["CAPTURE_ARENA_RESTRICT"] = "0"
        try:
            self.assertFalse(ctx.arena_restrict_enabled())   # explicit disable token
        finally:
            os.environ.pop("CAPTURE_ARENA_RESTRICT", None)
        # with NO env var at all, the Stage-3 default is ON
        self.assertTrue(ctx.arena_restrict_enabled())

    def test_capture_arena_scan_range_default_on(self):
        # capture._arena_scan_range mirrors ctx: DEFAULT ON -> returns the discovered arena bounds
        # and shares them via extras['_arena_bounds']; explicit-disable -> None (wide).
        import fastscan
        from live_capture import capture as _cap
        old = os.environ.pop("CAPTURE_ARENA_RESTRICT", None)
        orig = fastscan.discover_arena
        fastscan.discover_arena = lambda pid, gap=0x100000000: (0x12340000, 0x56780000)
        try:
            extras = {}
            rng = _cap._arena_scan_range(_MemProc(pid=4321), extras)
            self.assertEqual(rng, (0x12340000, 0x56780000))
            self.assertEqual(extras["_arena_bounds"], (0x12340000, 0x56780000))
            # explicit disable -> None (wide), no discover dependency
            self.assertIsNone(_cap._arena_scan_range(_MemProc(pid=4321),
                                                     {"arena_restrict": False}))
        finally:
            fastscan.discover_arena = orig
            if old is not None:
                os.environ["CAPTURE_ARENA_RESTRICT"] = old


class TestPinBranchTrace(unittest.TestCase):
    """The READ-ONLY pin branch-trace seam (ctx.extras['pin_trace']) records, per pinned read,
    exactly which _pin_get branch fired -- the diagnostic the perf harness uses to see why a warm
    walk re-scans. It must never alter read behaviour and must be a no-op when not enabled."""

    def _ctx(self, trace=None):
        extras = {"pins": {}}
        if trace is not None:
            extras["pin_trace"] = trace
        return reg.CaptureContext(proc=_MemProc(), identity=_FpIdentity(), extras=extras)

    def test_trace_records_cold_then_warm_hit(self):
        trace = []
        ctx = self._ctx(trace)
        reg._pin_get(ctx, "k", lambda: ({"r": 1}, "D", 1),
                     lambda r, p: True, lambda r: "W")
        self.assertEqual(trace, [("k", "miss_no_entry"), ("k", "resolve_pinned")])
        trace.clear()
        reg._pin_get(ctx, "k", lambda: ({"r": 1}, "D", 1),
                     lambda r, p: True, lambda r: "W")
        self.assertEqual(trace, [("k", "hit")])

    def test_trace_records_validate_false(self):
        trace = []
        ctx = self._ctx(trace)
        reg._pin_get(ctx, "k", lambda: ({"r": 1}, "D", 1), lambda r, p: True, lambda r: "W")
        trace.clear()
        reg._pin_get(ctx, "k", lambda: ({"r": 1}, "D2", 1), lambda r, p: False, lambda r: "W")
        self.assertEqual(trace, [("k", "miss_validate_false"), ("k", "resolve_pinned")])

    def test_trace_records_validate_raise(self):
        trace = []
        ctx = self._ctx(trace)
        reg._pin_get(ctx, "k", lambda: ({"r": 1}, "D", 1), lambda r, p: True, lambda r: "W")
        trace.clear()
        def _boom(r, p):
            raise RuntimeError("ships-style NameError surrogate")
        reg._pin_get(ctx, "k", lambda: ({"r": 1}, "D2", 1), _boom, lambda r: "W")
        # the raise is recorded ONCE (not also as validate_false), then a re-scan
        self.assertEqual(trace, [("k", "miss_validate_raise"), ("k", "resolve_pinned")])

    def test_trace_records_reread_raise(self):
        trace = []
        ctx = self._ctx(trace)
        reg._pin_get(ctx, "k", lambda: ({"r": 1}, "D", 1), lambda r, p: True, lambda r: "W")
        trace.clear()
        def _reread_boom(r):
            raise RuntimeError("root unreadable mid-walk")
        reg._pin_get(ctx, "k", lambda: ({"r": 1}, "D2", 1), lambda r, p: True, _reread_boom)
        self.assertEqual(trace, [("k", "miss_reread_raise"), ("k", "resolve_pinned")])

    def test_trace_records_resolve_unpinned(self):
        trace = []
        ctx = self._ctx(trace)
        reg._pin_get(ctx, "k", lambda: (None, "HONEST", None),
                     lambda r, p: True, lambda r: "W")
        self.assertEqual(trace, [("k", "miss_no_entry"), ("k", "resolve_unpinned")])

    def test_trace_off_is_noop(self):
        # no pin_trace in extras -> recording is a no-op, reads unaffected.
        ctx = self._ctx()  # no trace
        self.assertEqual(
            reg._pin_get(ctx, "k", lambda: ({"r": 1}, "D", 1),
                         lambda r, p: True, lambda r: "W"), "D")
        self.assertNotIn("pin_trace", ctx.extras)


class TestStage15WarmPins(unittest.TestCase):
    """Stage 1.5: the pin-MISS fixes (ships _oob NameError, designs_variants result-pin) and the
    newly-pinned residual readers each take the WARM fast-path and fail closed on a moved/freed
    root. Driven by mock-heap _MemProc + fake reader modules -- no live game."""

    # ---- ships pin-miss fix: validate now RUNS (was a NameError every warm walk) -------------
    def test_ships_pin_validate_runs_and_hits_warm(self):
        # before the fix, _pin_ships._validate referenced an undefined `_oob` -> NameError every
        # warm walk -> re-scan. Now it binds oob_full_live and validate genuinely runs -> warm hit.
        import oob_full_live as _oob
        base = 0x1000
        cship_vt = base + _oob.CSHIP_VT_OFF
        body, iid = 0x5000, 7
        mem = {body: cship_vt, body + _oob.OFF_HANDLE: (iid << 32) | _oob.SHIP_TYPE_TAG}

        class _Reader:
            module_base = base
            def __init__(self, m): self.mem = m
            def u64(self, a): return self.mem.get(a)
            def verify_build(self): return True, "sha"
            def close(self): pass

        class _Reg:
            scans = {"n": 0}
            def __init__(self, reader): self.r = reader; self._index = None
            def build_index(self):
                _Reg.scans["n"] += 1
                self._index = {iid: body}; return self._index
            def read_ship(self, i):
                return {"instance_id": i, "definition": "x", "category": "destroyer",
                        "strength_hp": 1.0, "organisation": 1.0, "experience": 0.0,
                        "sunk_convoys": 0}

        orig = reg._verify_same_process
        reg._verify_same_process = lambda ctx, reader: None
        try:
            trace = []
            ctx = reg.CaptureContext(proc=_MemProc(module_base=base), identity=_FpIdentity(),
                                     extras={"pins": {}, "pin_trace": trace})
            reg._pin_ships(ctx, lambda: _Reader(mem), _Reg)
            self.assertEqual(_Reg.scans["n"], 1)            # cold scan
            trace.clear()
            reg._pin_ships(ctx, lambda: _Reader(mem), _Reg)
            self.assertEqual(_Reg.scans["n"], 1)            # warm: NO re-scan (validate ran -> hit)
            self.assertEqual(trace, [("ships", "hit")])     # the fix: a genuine hit, not a re-scan
            # ABA: the instance id at the handle changes -> validate False -> re-scan
            mem[body + _oob.OFF_HANDLE] = (999 << 32) | _oob.SHIP_TYPE_TAG
            trace.clear()
            reg._pin_ships(ctx, lambda: _Reader(mem), _Reg)
            self.assertEqual(_Reg.scans["n"], 2)            # re-scan on the ABA miss
            self.assertEqual(trace[0], ("ships", "miss_validate_false"))
        finally:
            reg._verify_same_process = orig

    # ---- designs_variants result-pin: warm walk re-decodes NOTHING, fails closed -------------
    def _designs_module(self):
        # a fake designs_mio_deep_live module: a small variant pool with module/upgrade arrays,
        # exposing exactly the surface _pin_designs_variants + _decode_variant_catalog use.
        class _DMD:
            EQ_VARIANT_VT_RVA = 0x292B218
            VARIANT_TYPE = 70
            VAR_ID_OFF = 0x0C
            MOD_DATA_OFF = 0xB8
            MOD_SIZE_OFF = 0xC4
            UPG_DATA_OFF = 0x70
            UPG_SIZE_OFF = 0x7C

            def __init__(self):
                self.decode_calls = 0

            def enumerate_variant_addrs(self, p):
                return {41: 0x9000, 42: 0x9100}

            def read_variant_modules(self, p, addr):
                self.decode_calls += 1
                return {0x9000: [(1, "engine"), (2, "armor")],
                        0x9100: [(1, "engine")]}.get(addr, [])

            def read_variant_upgrades(self, p, addr):
                return {0x9000: [3, 1], 0x9100: [0]}.get(addr, [])

            def _load_variant_gt(self):
                return {}      # no GT -> slot_table empty -> slot_<hex> names (deterministic)

        return _DMD()

    def _designs_mem(self, dmd, base=0x1000):
        vt = base + dmd.EQ_VARIANT_VT_RVA
        mem = {}
        for vid, a in dmd.enumerate_variant_addrs(None).items():
            mem[a] = vt
            mem[a + 0x08] = dmd.VARIANT_TYPE
            mem[a + dmd.VAR_ID_OFF] = vid
            mod_data = 0xD000 + vid * 0x100
            upg_data = 0xE000 + vid * 0x100
            mem[a + dmd.MOD_DATA_OFF] = mod_data           # module-array data ptr
            mem[a + dmd.MOD_SIZE_OFF] = 2
            mem[a + dmd.UPG_DATA_OFF] = upg_data           # upgrade-array data ptr
            mem[a + dmd.UPG_SIZE_OFF] = 1
            # populate boundary module/upgrade entries so _variant_payload_fp resolves a real
            # fingerprint (entry stride 0x10: module key u32 @+0, val u64 @+8; upgrade val @+8).
            for i in range(2):
                mem[mod_data + i * 0x10 + 0x00] = vid * 10 + i
                mem[mod_data + i * 0x10 + 0x08] = 0xABCD00 + vid * 0x10 + i
            mem[upg_data + 0x08] = 3 + vid
        return mem

    def test_designs_variants_pin_caches_decode_and_hits_warm(self):
        dmd = self._designs_module()
        mem = self._designs_mem(dmd)
        ctx = reg.CaptureContext(proc=_MemProc(mem=mem, module_base=0x1000),
                                 identity=_FpIdentity(),
                                 extras={"pins": {}})
        cat1 = reg._pin_designs_variants(ctx, dmd)
        self.assertEqual(set(cat1), {"41", "42"})
        cold_decodes = dmd.decode_calls
        self.assertGreater(cold_decodes, 0)                # cold decoded the modules
        cat2 = reg._pin_designs_variants(ctx, dmd)
        self.assertEqual(cat1, cat2)                       # output-identical
        self.assertEqual(dmd.decode_calls, cold_decodes)   # warm: NO re-decode (cached catalog)

    def test_designs_variants_pin_invalidates_on_payload_change(self):
        # the per-variant PAYLOAD fingerprint (module/upgrade array data ptr+size) is a tripwire:
        # an in-place realloc of a variant's module array -> validate False -> full re-decode.
        dmd = self._designs_module()
        mem = self._designs_mem(dmd)
        proc = _MemProc(mem=mem, module_base=0x1000)
        ctx = reg.CaptureContext(proc=proc, identity=_FpIdentity(), extras={"pins": {}})
        reg._pin_designs_variants(ctx, dmd)
        cold = dmd.decode_calls
        # mutate a variant's module-array DATA POINTER (a realloc) -> payload fp changes. NOTE:
        # _MemProc copies the mem dict, so mutate proc.mem (the live map the validate reads).
        proc.mem[0x9000 + dmd.MOD_DATA_OFF] = 0xDEAD
        reg._pin_designs_variants(ctx, dmd)
        self.assertGreater(dmd.decode_calls, cold)         # re-decoded (fail-closed)

    def test_designs_variants_pin_invalidates_on_vtable_move(self):
        dmd = self._designs_module()
        mem = self._designs_mem(dmd)
        proc = _MemProc(mem=mem, module_base=0x1000)
        ctx = reg.CaptureContext(proc=proc, identity=_FpIdentity(), extras={"pins": {}})
        reg._pin_designs_variants(ctx, dmd)
        cold = dmd.decode_calls
        proc.mem[0x9000] = 0xBADC0DE                       # vtable clobbered (object moved/freed)
        reg._pin_designs_variants(ctx, dmd)
        self.assertGreater(dmd.decode_calls, cold)

    def test_designs_variants_pin_invalidates_on_boundary_entry_edit(self):
        # the payload fingerprint now also captures FIRST/LAST module+upgrade entries, so an
        # in-place edit of a boundary entry (same ptr+size) is caught -> re-decode (fail-closed).
        dmd = self._designs_module()
        mem = self._designs_mem(dmd)
        proc = _MemProc(mem=mem, module_base=0x1000)
        ctx = reg.CaptureContext(proc=proc, identity=_FpIdentity(), extras={"pins": {}})
        reg._pin_designs_variants(ctx, dmd)
        cold = dmd.decode_calls
        # edit the LAST module entry's value qword of variant 41 (same array ptr + size)
        last_entry_val = (0xD000 + 41 * 0x100) + 1 * 0x10 + 0x08
        proc.mem[last_entry_val] = 0xFEEDFACE
        reg._pin_designs_variants(ctx, dmd)
        self.assertGreater(dmd.decode_calls, cold)         # caught the interior edit

    def _designs_module_n(self, mod_size=3):
        # a designs module whose ONE variant (id 41) has mod_size module entries -- so an
        # INTERIOR (non-boundary) entry exists when mod_size >= 3. read_variant_modules returns
        # a decode derived from the live module-array bytes, so an in-place element swap MUST
        # change the decoded output (the catalog).
        class _DMD:
            EQ_VARIANT_VT_RVA = 0x292B218
            VARIANT_TYPE = 70
            VAR_ID_OFF = 0x0C
            MOD_DATA_OFF = 0xB8
            MOD_SIZE_OFF = 0xC4
            UPG_DATA_OFF = 0x70
            UPG_SIZE_OFF = 0x7C

            def __init__(self, n):
                self.decode_calls = 0
                self._n = n

            def enumerate_variant_addrs(self, p):
                return {41: 0x9000}

            def read_variant_modules(self, p, addr):
                # DECODE FROM LIVE BYTES: read each entry's (slot_handle, module_val) so an
                # interior element swap changes the decoded module list.
                self.decode_calls += 1
                mod_data = 0xD000 + 41 * 0x100
                out = []
                for i in range(self._n):
                    h = p.u32(mod_data + i * 0x10 + 0x00)
                    v = p.u64(mod_data + i * 0x10 + 0x08)
                    out.append((h, "mod_%X" % v))
                return out

            def read_variant_upgrades(self, p, addr):
                return [0]

            def _load_variant_gt(self):
                return {}

        return _DMD(mod_size)

    def _designs_mem_n(self, dmd, n=3, base=0x1000):
        vt = base + dmd.EQ_VARIANT_VT_RVA
        a = 0x9000
        mod_data = 0xD000 + 41 * 0x100
        upg_data = 0xE000 + 41 * 0x100
        mem = {
            a: vt, a + 0x08: dmd.VARIANT_TYPE, a + dmd.VAR_ID_OFF: 41,
            a + dmd.MOD_DATA_OFF: mod_data, a + dmd.MOD_SIZE_OFF: n,
            a + dmd.UPG_DATA_OFF: upg_data, a + dmd.UPG_SIZE_OFF: 1,
            upg_data + 0x08: 7,
        }
        for i in range(n):
            mem[mod_data + i * 0x10 + 0x00] = 100 + i           # slot handle
            mem[mod_data + i * 0x10 + 0x08] = 0x1000 + i        # module value
        return mem

    def test_designs_variants_pin_invalidates_on_interior_entry_swap(self):
        # NO-STALE-DATA: an in-place, SAME-PTR/SAME-SIZE swap of an INTERIOR module entry
        # changes the decoded catalog but is NOT a boundary entry -- a (ptr,size,first,last)
        # fingerprint MISSES it and a warm hit would serve the stale decode. The fingerprint
        # MUST cover every decoded element -> re-decode (fail-closed).
        dmd = self._designs_module_n(mod_size=3)
        mem = self._designs_mem_n(dmd, n=3)
        proc = _MemProc(mem=mem, module_base=0x1000)
        ctx = reg.CaptureContext(proc=proc, identity=_FpIdentity(), extras={"pins": {}})
        cat1 = reg._pin_designs_variants(ctx, dmd)
        cold = dmd.decode_calls
        # the cold catalog reflects the middle module value 0x1001 -> "mod_1001"
        self.assertIn("mod_1001", set(cat1["41"]["modules"].values()))
        # swap the MIDDLE entry's module value IN PLACE (same array ptr + same size=3).
        proc.mem[(0xD000 + 41 * 0x100) + 1 * 0x10 + 0x08] = 0x2002
        cat2 = reg._pin_designs_variants(ctx, dmd)
        self.assertGreater(dmd.decode_calls, cold)         # MUST re-decode (caught the swap)
        self.assertIn("mod_2002", set(cat2["41"]["modules"].values()))  # LIVE, not stale
        self.assertNotIn("mod_1001", set(cat2["41"]["modules"].values()))

    def test_designs_variants_pin_fails_closed_on_none_fingerprint(self):
        # FAIL-CLOSED (Codex Q2 residual): when the per-variant payload fp is UNREADABLE
        # (a read fault / insane size -> _variant_payload_fp returns None), the variant's
        # payload is UNVERIFIABLE. A warm hit MUST re-decode (fail closed), NOT serve the
        # cached catalog behind a None==None compare. We force None by making the module
        # size exceed the _VAR_FP_MAX_ENTRIES cap.
        dmd = self._designs_module_n(mod_size=1)           # decode reads 1 entry (small)
        mem = self._designs_mem_n(dmd, n=1)
        # but advertise an INSANE module size in the header so the fp read returns None
        # (the decode mock only reads self._n entries, so the catalog still builds).
        a = 0x9000
        mem[a + dmd.MOD_SIZE_OFF] = reg._VAR_FP_MAX_ENTRIES + 1
        proc = _MemProc(mem=mem, module_base=0x1000)
        ctx = reg.CaptureContext(proc=proc, identity=_FpIdentity(), extras={"pins": {}})
        reg._pin_designs_variants(ctx, dmd)
        cold = dmd.decode_calls
        # nothing changed in memory, but the fp is None (unverifiable) -> a warm walk MUST
        # NOT serve the cached catalog; it must re-decode (fail-closed).
        reg._pin_designs_variants(ctx, dmd)
        self.assertGreater(dmd.decode_calls, cold,
                           "an unverifiable (None) payload fp must force a re-decode, "
                           "never serve the cached catalog behind None==None")

    # ---- diplomacy war-relation pin: output-identical warm path + fail-closed ----------------
    def _diplo_setup(self):
        import diplomacy_live as dl
        base = 0x1000
        typeptr = base + dl.WR_TYPEPTR_RVA
        # one war_relation record A(=3) vs B(=5) at addr 0x7000; casualties 11/22.
        rec = 0x7000
        mem = {
            rec + dl.WR_FIRST_TAGID: 3, rec + dl.WR_SECOND_TAGID: 5,
            rec + dl.WR_TYPEPTR: typeptr,
            rec + dl.WR_ECHO: 3, rec + dl.WR_ECHO + 4: 5,
            rec + dl.WR_FIRST_CAS: 11, rec + dl.WR_SECOND_CAS: 22,
        }
        proc = _MemProc(mem=mem, module_base=base)
        addr_by_pair = {(3, 5): rec}

        class _Ident:
            def tag(self, tid): return {3: "GER", 5: "SOV"}.get(tid)
        return dl, proc, addr_by_pair, _Ident(), rec

    def test_diplomacy_war_addrs_validate_and_reread_output_identical(self):
        dl, proc, addr_by_pair, ident, rec = self._diplo_setup()
        # the warm validate passes (signature intact) and the reread reproduces the SAME record.
        self.assertTrue(dl.validate_war_relation_addrs(proc, ident, addr_by_pair))
        wr = dl.read_war_relations_from_addrs(proc, ident, addr_by_pair)
        self.assertEqual(wr, [{"first": "GER", "second": "SOV",
                               "first_casualties": 11, "second_casualties": 22}])

    def test_diplomacy_war_addrs_fail_closed_on_typeptr_change(self):
        dl, proc, addr_by_pair, ident, rec = self._diplo_setup()
        proc.mem[rec + dl.WR_TYPEPTR] = 0xBADBAD          # the type ptr no longer matches
        self.assertFalse(dl.validate_war_relation_addrs(proc, ident, addr_by_pair))

    def test_diplomacy_war_addrs_fail_closed_on_pair_change(self):
        dl, proc, addr_by_pair, ident, rec = self._diplo_setup()
        proc.mem[rec + dl.WR_SECOND_TAGID] = 9            # ABA: the slot now holds a different pair
        self.assertFalse(dl.validate_war_relation_addrs(proc, ident, addr_by_pair))

    def test_diplomacy_read_all_warm_skips_sweep(self):
        # read_all(p, ident, war_addrs=...) takes the warm path (re-read from pinned addrs) and
        # does NOT call read_war_relations (the sweep). Patch read_war_relations to a tripwire.
        dl, proc, addr_by_pair, ident, rec = self._diplo_setup()
        # faction groups: ident must expose tags()/object_of for read_faction_groups; extend it.
        ident.tags = lambda: ["GER", "SOV"]
        ident.object_of = lambda t: None                  # no faction object -> empty groups
        swept = {"n": 0}
        orig = dl.read_war_relations
        dl.read_war_relations = lambda *a, **k: (swept.__setitem__("n", swept["n"] + 1) or ([], {}))
        try:
            out = dl.read_all(proc, ident=ident, war_addrs=addr_by_pair, want_war_addrs=True)
        finally:
            dl.read_war_relations = orig
        self.assertEqual(swept["n"], 0)                   # the SWEEP was skipped on the warm path
        self.assertEqual(out["war_relations"],
                         [{"first": "GER", "second": "SOV",
                           "first_casualties": 11, "second_casualties": 22}])
        self.assertEqual(out["at_war"], {"GER": ["SOV"], "SOV": ["GER"]})
        self.assertEqual(out["war_addrs"], addr_by_pair)  # addrs threaded back for the next walk

    def test_diplomacy_read_all_falls_back_to_sweep_on_bad_pin(self):
        # a STALE war_addrs (validate fails) -> read_all re-sweeps (fail-closed), never serves the
        # stale pinned record.
        dl, proc, addr_by_pair, ident, rec = self._diplo_setup()
        ident.tags = lambda: []
        ident.object_of = lambda t: None
        proc.mem[rec + dl.WR_TYPEPTR] = 0xBADBAD          # the pinned record is now invalid
        swept = {"n": 0}
        orig = dl.read_war_relations
        def _sweep(p, idn, want_addrs=False):
            swept["n"] += 1
            return ([], {}, {}) if want_addrs else ([], {})
        dl.read_war_relations = _sweep
        try:
            out = dl.read_all(proc, ident=ident, war_addrs=addr_by_pair, want_war_addrs=True)
        finally:
            dl.read_war_relations = orig
        self.assertEqual(swept["n"], 1)                   # fell back to the full sweep


class TestStage15ResidualPins(unittest.TestCase):
    """Stage 1.5: each newly-pinned RESIDUAL reader (commanders / projects / tech / operatives /
    oob_command) takes the warm fast-path and re-scans on a validate miss -- via the registry pin
    helpers driven by fake reader modules (no live game)."""

    def _ctx(self):
        return reg.CaptureContext(proc=_MemProc(), identity=_FpIdentity(), extras={"pins": {}})

    def test_pin_commanders_warm_then_invalidate(self):
        calls = {"enum": 0, "validate": 0}
        out = {"army_leaders": [], "navy_leaders": [], "characters": {},
               "counts": {"army_leaders": 0, "navy_leaders": 0, "characters": 0}}

        class _Reader:
            pid = None
            valid = True
            def verify_build(self): return True, "sha"
            def close(self): pass
            def enumerate_addrs(self):
                calls["enum"] += 1
                return {"army": [0x1], "navy": [], "char": [0x2], "country_leader": [],
                        "_ids": {0x1: 1, 0x2: 2}}
            def enumerate_from_addrs(self, a, resolve_country=True, read_traits=True):
                return dict(out)
            def validate_addrs(self, a):
                calls["validate"] += 1
                return _Reader.valid

        orig = reg._verify_same_process
        reg._verify_same_process = lambda ctx, reader: None
        try:
            ctx = self._ctx()
            reg._pin_commanders(ctx, _Reader)
            self.assertEqual(calls["enum"], 1)             # cold sweep
            reg._pin_commanders(ctx, _Reader)
            self.assertEqual(calls["enum"], 1)             # warm: NO new sweep
            self.assertGreaterEqual(calls["validate"], 1)
            _Reader.valid = False                          # a body moved/freed -> re-scan
            reg._pin_commanders(ctx, _Reader)
            self.assertEqual(calls["enum"], 2)
        finally:
            reg._verify_same_process = orig

    def test_pin_tech_warm_then_invalidate(self):
        mod = _FakeTechModule(
            lambda p, bridge=None, strict=True: {"GER": {"completed_tech_count": 1,
                                                         "research_slot_count": 1,
                                                         "active_research": []}})
        ctx = self._ctx()
        reg._pin_tech(ctx, mod, {0: "GER"}, 0xAA00, 88)
        self.assertEqual(mod.enum_calls, 1)
        reg._pin_tech(ctx, mod, {0: "GER"}, 0xAA00, 88)
        self.assertEqual(mod.enum_calls, 1)                # warm: NO re-sweep
        mod.valid = False
        reg._pin_tech(ctx, mod, {0: "GER"}, 0xAA00, 88)
        self.assertEqual(mod.enum_calls, 2)                # re-sweep on validate miss

    def test_pin_operatives_warm_then_invalidate(self):
        calls = {"enum": 0, "validate": 0}

        class _FR:
            valid = True
            def enumerate_operatives(self, p, bridge=None):
                calls["enum"] += 1
                return {7: {"op_id": 7, "owner_tag": "GER", "name": "x",
                            "identity_addr": 0x100}}
            def read_operative_skill_xp_from_operatives(self, p, operatives, bridge=None):
                return {oid: {"owner_tag": o["owner_tag"], "name": o["name"],
                              "skill": 3, "experience": 1.0} for oid, o in operatives.items()}
            def validate_operatives(self, p, operatives):
                calls["validate"] += 1
                return _FR.valid

        fr = _FR()
        ctx = self._ctx()
        reg._pin_operatives(ctx, fr)
        self.assertEqual(calls["enum"], 1)
        reg._pin_operatives(ctx, fr)
        self.assertEqual(calls["enum"], 1)                 # warm: NO re-sweep
        _FR.valid = False
        reg._pin_operatives(ctx, fr)
        self.assertEqual(calls["enum"], 2)

    def test_operative_xp_reader_derives_tech_units_from_shared_tech_pin(self):
        # _r_operative_xp pins operatives AND derives tech_progress_units from the SHARED "tech"
        # pin (no second tech sweep). The tech_units output == progress_raw/scale per active slot,
        # identical to the old read_tech_progress transform.
        class _FR:
            TECH_PROGRESS_SCALE = 1000.0
            def enumerate_operatives(self, p, bridge=None):
                return {7: {"op_id": 7, "owner_tag": "GER", "name": "spy",
                            "identity_addr": 0x100}}
            def read_operative_skill_xp_from_operatives(self, p, operatives, bridge=None):
                return {7: {"owner_tag": "GER", "name": "spy", "skill": 4, "experience": 2.0}}
            def validate_operatives(self, p, operatives):
                return True

        tl = _FakeTechModule(lambda p, bridge=None, strict=True: {
            "GER": {"completed_tech_count": 5, "research_slot_count": 2,
                    "active_research": [{"slot_index": 0, "progress_raw": 2500,
                                         "tech_handle_raw": 9, "tech_def_index_raw": 9},
                                        {"slot_index": 1, "progress_raw": 0,
                                         "tech_handle_raw": 3, "tech_def_index_raw": 3}]}})
        ctx = self._ctx()
        out = reg._r_operative_xp(ctx, (_FR(), tl))
        self.assertEqual(out["operative_skill_xp"], {"GER": {"7": {
            "name": "spy", "skill": 4, "experience": 2.0}}})
        # progress_units = progress_raw/1000; the 0-progress slot is KEPT as 0.0 (not None)
        self.assertEqual(out["tech_progress_units"], {"GER": {0: 2.5, 1: 0.0}})

    def test_pin_oob_command_warm_then_invalidate(self):
        calls = {"enum": 0, "validate": 0}
        tree = {"armies": {}, "army_groups": {}, "fleets": {}, "task_forces": {},
                "theatres": {}, "_meta": {}}

        import oob_command_live as _ocl_real

        class _OCL:
            valid = True
            # _pin_oob_command's struct fingerprint (_oob_struct_fp, Codex-C MUST-FIX) reads the
            # mutable inline edge offsets off the module -- surface them so the fake is complete.
            DIV_ARMY_PARENT = _ocl_real.DIV_ARMY_PARENT
            DIV_OWNER = _ocl_real.DIV_OWNER
            ARMY_AG_PARENT = _ocl_real.ARMY_AG_PARENT
            ARMY_LEADER_PTR = _ocl_real.ARMY_LEADER_PTR
            ARMY_MEMBER_COUNT = _ocl_real.ARMY_MEMBER_COUNT
            TF_FLEET_PARENT = _ocl_real.TF_FLEET_PARENT
            TF_SHIP_BEGIN = _ocl_real.TF_SHIP_BEGIN
            TF_SHIP_COUNT = _ocl_real.TF_SHIP_COUNT
            def enumerate_command_objs(self, p):
                calls["enum"] += 1
                return {"_arena": (1, 2), "army": [(0x10, (1, 53))], "ag": [], "fleet": [],
                        "tf": [], "theatre": [], "div": [], "ship": []}
            def read_command_tree_from_objs(self, p, objs):
                return dict(tree)
            def validate_command_objs(self, p, objs):
                calls["validate"] += 1
                return _OCL.valid

        ocl = _OCL()
        ctx = self._ctx()
        reg._pin_oob_command(ctx, ocl)
        self.assertEqual(calls["enum"], 1)
        reg._pin_oob_command(ctx, ocl)
        self.assertEqual(calls["enum"], 1)                 # warm: locate NOT re-run
        _OCL.valid = False
        reg._pin_oob_command(ctx, ocl)
        self.assertEqual(calls["enum"], 2)


class TestLocateAbortBugfix(unittest.TestCase):
    """The correctness bug: the PP-bridge readers called _locate_country_array UNCONDITIONALLY
    and its failure raised SystemExit (a BaseException) that killed the WHOLE walk. The fix:
    (a) supplying rec0/count from the live bridge SKIPS _locate_country_array entirely;
    (b) a genuine locate failure raises LocateError (a plain Exception) -> the plug omits ONE
    domain honestly and the walk continues."""

    def test_locate_error_is_a_plain_exception(self):
        import country_misc_live as cm
        self.assertTrue(issubclass(cm.LocateError, Exception))
        self.assertFalse(issubclass(cm.LocateError, SystemExit))   # NOT a BaseException-only abort

    def test_reader_skips_locate_when_rec0_count_supplied(self):
        # focus_live.read_country_focus must NOT call _locate_country_array when rec0/count given.
        import focus_live as fl
        called = {"n": 0}
        orig = fl._locate_country_array
        fl._locate_country_array = lambda p, save_pp: (_ for _ in ()).throw(
            AssertionError("_locate_country_array must NOT be called on the bridge path"))
        try:
            # bridge dict + rec0/count supplied -> the locate branch is skipped; the per-record
            # walk runs against our fake proc (which returns nothing) -> empty dict, no raise.
            class _P:
                def u16(self, a): raise OSError("x")
                def u64(self, a): raise OSError("x")
            out = fl.read_country_focus(_P(), bridge={0: "GER"}, rec0=0x1000, count=10)
            self.assertEqual(out, {})                       # no countries, but NO locate + NO raise
        finally:
            fl._locate_country_array = orig

    def test_reader_rejects_partial_rec0_count(self):
        import focus_live as fl
        with self.assertRaises(ValueError):
            fl.read_country_focus(_MemProc(), rec0=0x1000)   # count missing -> caller bug, fail closed

    def test_plug_wrapper_omits_domain_on_locate_error_not_abort(self):
        # a reader raising LocateError -> the plug records-and-continues (domain absent), the walk
        # is NOT killed (LocateError is an Exception, caught by ReaderPlug.read).
        import country_misc_live as cm
        def _factory():
            def _impl():  # the plug factory returns the reader callable
                return None
            return _impl
        def _reader(ctx, impl):
            raise cm.LocateError("no clean-index array base resolved from the PP anchors")
        plug = reg.ReaderPlug("x", _factory, _reader)
        plug._impl = object()
        plug._available = True
        dr = plug.read(reg.CaptureContext(proc=_MemProc()))
        self.assertFalse(dr.captured)                       # honest omission
        self.assertIsNone(dr.data)                          # no fabricated data
        self.assertIn("LocateError", dr.error or "")


if __name__ == "__main__":
    unittest.main(verbosity=2)
