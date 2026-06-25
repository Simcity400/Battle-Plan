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
            "melt_file": r"C:\Users\Example\saves\autosave.hoi4",
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
            "melt_path": r"C:\Users\Example\saves\autosave.hoi4",
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


class TestNewPlugsRegistered(unittest.TestCase):
    """The default registry exposes the new plugs, and the save backstops are
    OPT-IN: they RAISE (captured=False -> omitted) when no save path is supplied,
    so a pure-live capture omits them by default (no fabrication). No live memory:
    we drive each plug's reader directly with a fake context."""

    def test_default_registry_includes_new_plugs(self):
        names = {p.name for p in reg.default_registry()}
        for n in ("land_combats", "buildings", "intel"):
            self.assertIn(n, names)

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
                               source=r"C:\Users\Example\private\path.log"))
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


if __name__ == "__main__":
    unittest.main(verbosity=2)
