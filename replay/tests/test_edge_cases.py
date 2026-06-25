"""Edge-case tests for the issues Codex flagged (P1-P3): country lifecycle,
removed entities, division id-reuse, tech downgrade, subject/autonomy, TAG->null,
interval-aware marker dedupe, same-hour coalescing, and strict date validation."""
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))

from replay import schema, diff_engine  # noqa: E402


def _snap(date, *, memory_domains=None, save_domains=None):
    s = schema.empty_snapshot("g", date)
    if memory_domains is not None:
        s["provenance"]["memory"] = {"captured": True, "domains": memory_domains}
    if save_domains is not None:
        s["provenance"]["save"] = {"captured": True, "domains": save_domains,
                                   "source_filename": None, "save_date": date}
    return s


def _country(snap, tag, **politics):
    c = schema.empty_country()
    c["politics"].update(politics)
    snap["countries"][tag] = c
    return c


def _state(snap, sid, owner=None, controller=None):
    st = schema.empty_state()
    st["owner"] = owner
    st["controller"] = controller
    snap["states"][str(sid)] = st
    return st


def _div(snap, did, dtype, owner, prov, tpl):
    d = schema.empty_division()
    d.update(div_id=did, div_type=dtype, owner=owner, location_province=prov,
             template_id=tpl)
    snap["divisions"].append(d)
    return d


def _types(evs):
    return [e["type"] for e in evs]


class TestDateValidationStrict(unittest.TestCase):
    def test_invalid_day_rejected(self):
        self.assertIsNone(schema.date_to_hour_index("1936.2.30.0"))  # Feb 30
        self.assertIsNone(schema.date_to_hour_index("1936.4.31.0"))  # Apr 31
        self.assertIsNone(schema.date_to_hour_index("1936.2.29.0"))  # no leap

    def test_invalid_hour_rejected(self):
        self.assertIsNone(schema.date_to_hour_index("1936.1.1.24"))
        self.assertIsNone(schema.date_to_hour_index("1936.1.1.99"))

    def test_no_aliasing_between_distinct_dates(self):
        # Feb 28 last valid day, Mar 1 next — must be 24h apart, no overlap
        a = schema.date_to_hour_index("1936.2.28.0")
        b = schema.date_to_hour_index("1936.3.1.0")
        self.assertEqual(b - a, 24)


class TestCountryLifecycle(unittest.TestCase):
    def test_country_created_and_removed_when_domain_covered(self):
        a = _snap("1940.1.1.0", save_domains=["country_snapshot"])
        b = _snap("1940.1.2.0", save_domains=["country_snapshot"])
        _country(a, "POL")          # POL annexed -> gone in b
        _country(b, "ISR")          # ISR released -> new in b
        evs = diff_engine.diff_snapshots(a, b)
        types = _types(evs)
        self.assertIn("country_removed", types)
        self.assertIn("country_created", types)
        removed = next(e for e in evs if e["type"] == "country_removed")
        self.assertEqual(removed["key"], "POL")
        created = next(e for e in evs if e["type"] == "country_created")
        self.assertEqual(created["key"], "ISR")

    def test_no_lifecycle_event_when_domain_not_covered(self):
        # When provenance is RECORDED but the country_snapshot domain is NOT in it,
        # a missing country is "not captured this snapshot", NOT "ceased to exist".
        # (e.g. only divisions were captured — a partial country map from that must
        # not fire false country_removed.)
        a = _snap("1940.1.1.0", save_domains=["divisions"])
        b = _snap("1940.1.2.0", save_domains=["divisions"])
        _country(a, "POL")  # present in a only, but country_snapshot NOT captured
        evs = diff_engine.diff_snapshots(a, b)
        self.assertNotIn("country_removed", _types(evs))


class TestRemovedEntities(unittest.TestCase):
    def test_state_present_only_in_old_still_diffs(self):
        a = _snap("1940.1.1.0"); b = _snap("1940.1.2.0")
        _state(a, 1, owner="POL", controller="POL")  # disappears in b
        evs = diff_engine.diff_snapshots(a, b)
        # owner POL -> null surfaces as a state_owner_change with subject=POL
        owner = next(e for e in evs if e["type"] == "state_owner_change")
        self.assertEqual(owner["from"], "POL")
        self.assertIsNone(owner["to"])
        self.assertEqual(owner["subject"], "POL")  # losing tag, filterable

    def test_tag_to_null_subject_is_losing_tag(self):
        a = _snap("1940.1.1.0"); b = _snap("1940.1.2.0")
        _state(a, 1, owner="FRA"); _state(b, 1, owner=None)
        ev = next(e for e in diff_engine.diff_snapshots(a, b)
                  if e["type"] == "state_owner_change")
        self.assertEqual(ev["subject"], "FRA")


class TestDivisionIdReuse(unittest.TestCase):
    def test_id_reuse_is_destroy_plus_create_not_move(self):
        a = _snap("1940.1.1.0"); b = _snap("1940.1.2.0")
        _div(a, 7, 100, "GER", 10, 1)   # div (7:100)
        _div(b, 7, 200, "SOV", 99, 2)   # SAME bare id 7, DIFFERENT type -> new div
        evs = diff_engine.diff_snapshots(a, b)
        types = set(_types(evs))
        self.assertIn("division_destroyed", types)
        self.assertIn("division_created", types)
        self.assertNotIn("division_moved", types)
        self.assertNotIn("division_template_changed", types)

    def test_same_uid_move_still_detected(self):
        a = _snap("1940.1.1.0"); b = _snap("1940.1.2.0")
        _div(a, 7, 100, "GER", 10, 1)
        _div(b, 7, 100, "GER", 11, 1)  # same uid, moved
        evs = diff_engine.diff_snapshots(a, b)
        self.assertEqual(_types(evs), ["division_moved"])


class TestTechRollback(unittest.TestCase):
    def test_level_decrease_is_tech_lost_not_researched(self):
        a = _snap("1940.1.1.0"); b = _snap("1940.1.2.0")
        ca = _country(a, "GER"); cb = _country(b, "GER")
        ca["research"]["technologies"] = [{"name": "t", "level": 3}]
        cb["research"]["technologies"] = [{"name": "t", "level": 1}]
        evs = diff_engine.diff_snapshots(a, b)
        self.assertEqual(_types(evs), ["tech_lost"])
        self.assertEqual(evs[0]["to"], 1)

    def test_tech_disappears_is_tech_lost(self):
        a = _snap("1940.1.1.0"); b = _snap("1940.1.2.0")
        ca = _country(a, "GER"); cb = _country(b, "GER")
        ca["research"]["technologies"] = [{"name": "t", "level": 1}]
        cb["research"]["technologies"] = []
        evs = diff_engine.diff_snapshots(a, b)
        self.assertEqual(_types(evs), ["tech_lost"])

    def test_level_increase_still_researched(self):
        a = _snap("1940.1.1.0"); b = _snap("1940.1.2.0")
        ca = _country(a, "GER"); cb = _country(b, "GER")
        ca["research"]["technologies"] = [{"name": "t", "level": 1}]
        cb["research"]["technologies"] = [{"name": "t", "level": 2}]
        evs = diff_engine.diff_snapshots(a, b)
        self.assertEqual(_types(evs), ["tech_researched"])


class TestSubjectAutonomy(unittest.TestCase):
    def test_subject_created_freed_and_autonomy_changed(self):
        a = _snap("1940.1.1.0"); b = _snap("1940.1.2.0")
        # IRE becomes a subject of ENG
        ca = _country(a, "IRE"); cb = _country(b, "IRE")
        cb["politics"]["autonomy"] = {"level": "puppet", "overlord": "ENG"}
        evs = diff_engine.diff_snapshots(a, b)
        self.assertIn("subject_created", _types(evs))

        # then autonomy changes
        c = _snap("1940.1.3.0")
        cc = _country(c, "IRE")
        cc["politics"]["autonomy"] = {"level": "dominion", "overlord": "ENG"}
        evs2 = diff_engine.diff_snapshots(b, c)
        self.assertIn("autonomy_changed", _types(evs2))

        # then freed
        d = _snap("1940.1.4.0")
        _country(d, "IRE")  # no autonomy
        evs3 = diff_engine.diff_snapshots(c, d)
        self.assertIn("subject_freed", _types(evs3))


class TestIntervalAwareDedupe(unittest.TestCase):
    def _series_with_distant_marker(self):
        # monthly-ish saves: change between s1 (hour 0) and s2 (hour ~720); a
        # marker fires at hour 300 (inside the interval, >24h from s2's bound).
        s1 = _snap("1936.1.1.0", save_domains=["focus"])
        s2 = _snap("1936.2.1.0", save_domains=["focus"])  # ~744h later
        c1 = _country(s1, "GER"); c2 = _country(s2, "GER")
        c1["focus"]["completed"] = []
        c2["focus"]["completed"] = ["fA"]
        marker_hour = schema.date_to_hour_index("1936.1.13.12")  # mid-interval
        s2["events"] = [schema.make_event("focus_completed", subject="GER", key="fA",
                                          to="fA", source="marker", confidence="exact",
                                          hour_index=marker_hour, date="1936.1.13.12")]
        return s1, s2

    def test_distant_marker_supersedes_interval_diff(self):
        s1, s2 = self._series_with_distant_marker()
        tl = diff_engine.build_timeline([s1, s2])
        focus = [e for e in tl if e["type"] == "focus_completed" and e["key"] == "fA"]
        self.assertEqual(len(focus), 1)            # exactly one, not duplicated
        self.assertEqual(focus[0]["source"], "marker")
        self.assertEqual(focus[0]["confidence"], "exact")

    def test_marker_outside_interval_does_not_supersede(self):
        s1, s2 = self._series_with_distant_marker()
        # move the marker BEFORE the interval start -> must not dedupe the diff
        s2["events"][0]["hour_index"] = schema.date_to_hour_index("1935.12.1.0")
        tl = diff_engine.build_timeline([s1, s2])
        focus = [e for e in tl if e["type"] == "focus_completed" and e["key"] == "fA"]
        # both survive (marker out of range can't claim the diff)
        self.assertEqual(len(focus), 2)

    def test_from_in_dedupe_key_keeps_opposite_flips(self):
        # A->B then B->A on the same state must both survive (not collapse).
        s1 = _snap("1940.1.1.0"); s2 = _snap("1940.1.2.0"); s3 = _snap("1940.1.3.0")
        _state(s1, 1, owner="A"); _state(s2, 1, owner="B"); _state(s3, 1, owner="A")
        tl = diff_engine.build_timeline([s1, s2, s3])
        owner_evs = [e for e in tl if e["type"] == "state_owner_change"]
        self.assertEqual(len(owner_evs), 2)
        self.assertEqual((owner_evs[0]["from"], owner_evs[0]["to"]), ("A", "B"))
        self.assertEqual((owner_evs[1]["from"], owner_evs[1]["to"]), ("B", "A"))


class TestSameHourCoalesce(unittest.TestCase):
    def test_complementary_same_hour_snapshots_do_not_fabricate(self):
        # s_mem has owner (memory), s_save has focus (save); same hour. Diffing
        # them naively would see focus null->X and owner X->null. Coalescing
        # merges them into one instant so neither fabricated transition appears
        # against the PRIOR real snapshot.
        prev = _snap("1940.1.1.0", save_domains=["state_ownership", "focus"])
        _state(prev, 1, owner="GER")
        cprev = _country(prev, "GER"); cprev["focus"]["completed"] = []

        s_mem = _snap("1940.1.2.0", memory_domains=["state_ownership"])
        _state(s_mem, 1, owner="GER")
        s_save = _snap("1940.1.2.0", save_domains=["focus"])
        c = _country(s_save, "GER"); c["focus"]["completed"] = ["fA"]

        tl = diff_engine.build_timeline([prev, s_mem, s_save])
        # the only real change is focus fA completed; owner unchanged -> no owner event
        self.assertNotIn("state_owner_change", _types(tl))
        self.assertIn("focus_completed", _types(tl))


class TestDomainGatingNoFabrication(unittest.TestCase):
    def test_uncaptured_state_domain_emits_nothing(self):
        # b did NOT capture state_ownership -> a's states must not appear as "lost".
        a = _snap("1940.1.1.0", save_domains=["state_ownership"])
        b = _snap("1940.1.2.0", save_domains=["focus"])  # no state domain
        _state(a, 1, owner="GER")
        evs = diff_engine.diff_snapshots(a, b)
        self.assertNotIn("state_owner_change", _types(evs))

    def test_uncaptured_division_domain_emits_nothing(self):
        a = _snap("1940.1.1.0", save_domains=["divisions"])
        b = _snap("1940.1.2.0", save_domains=["focus"])  # divisions not captured in b
        _div(a, 1, 100, "GER", 10, 1)
        evs = diff_engine.diff_snapshots(a, b)
        self.assertEqual(evs, [])  # no spurious division_destroyed

    def test_both_captured_state_domain_diffs_normally(self):
        a = _snap("1940.1.1.0", save_domains=["state_ownership"])
        b = _snap("1940.1.2.0", save_domains=["state_ownership"])
        _state(a, 1, owner="POL"); _state(b, 1, owner="GER")
        self.assertIn("state_owner_change", _types(diff_engine.diff_snapshots(a, b)))


class TestSameHourMergeNoDataLoss(unittest.TestCase):
    def test_complementary_domains_both_survive(self):
        mem = _snap("1940.1.2.0", memory_domains=["state_ownership"])
        _state(mem, 1, owner="GER")
        sav = _snap("1940.1.2.0", save_domains=["focus"])
        c = _country(sav, "GER"); c["focus"]["completed"] = ["fA"]
        merged = diff_engine._merge_two_snapshots(mem, sav)
        self.assertEqual(merged["states"]["1"]["owner"], "GER")
        self.assertEqual(merged["countries"]["GER"]["focus"]["completed"], ["fA"])

    def test_empty_later_does_not_wipe_real_earlier(self):
        e1 = schema.empty_snapshot("g", "1940.1.2.0")
        e1["diplomacy_world"]["wars"] = {"GER-POL@x": {"a": "GER", "b": "POL"}}
        e2 = schema.empty_snapshot("g", "1940.1.2.0")  # empty skeleton
        m = diff_engine._merge_two_snapshots(e1, e2)
        self.assertIn("GER-POL@x", m["diplomacy_world"]["wars"])

    def test_merged_memory_domains_exclude_save_overlap(self):
        # If save covered a domain in either input, the merged memory channel must
        # not still claim that domain as memory-exact.
        mem = _snap("1940.1.2.0", memory_domains=["state_ownership", "focus"])
        sav = _snap("1940.1.2.0", save_domains=["focus"])
        m = diff_engine._merge_two_snapshots(mem, sav)
        self.assertNotIn("focus", m["provenance"]["memory"]["domains"])
        self.assertIn("state_ownership", m["provenance"]["memory"]["domains"])

    def test_empty_list_default_does_not_wipe_real_list(self):
        # later skeleton with ideas_all=[]/focus.completed=[] must not erase earlier
        # real lists (Codex 3rd-pass P1).
        a = _snap("1940.1.2.0")
        ca = _country(a, "GER")
        ca["politics"]["ideas_all"] = ["idea_a"]
        ca["focus"]["completed"] = ["focus_a"]
        b = _snap("1940.1.2.0")
        _country(b, "GER")  # all-empty skeleton
        m = diff_engine._merge_two_snapshots(a, b)
        self.assertEqual(m["countries"]["GER"]["politics"]["ideas_all"], ["idea_a"])
        self.assertEqual(m["countries"]["GER"]["focus"]["completed"], ["focus_a"])

    def test_division_equipment_list_not_wiped_on_merge(self):
        a = _snap("1940.1.2.0")
        da = schema.empty_division()
        da.update(div_id=1, div_type=4713, equipment=[{"archetype": "x", "amount": 5}])
        a["divisions"] = [da]
        b = _snap("1940.1.2.0")
        db = schema.empty_division(); db.update(div_id=1, div_type=4713)  # no equipment
        b["divisions"] = [db]
        m = diff_engine._merge_two_snapshots(a, b)
        self.assertEqual(m["divisions"][0]["equipment"], [{"archetype": "x", "amount": 5}])


class TestSplitPoliticsGating(unittest.TestCase):
    def test_no_false_law_idea_events_when_economy_uncaptured(self):
        # both captured country_snapshot (so government CAN diff) but NOT economy ->
        # laws/ideas must NOT diff (would fabricate from empty defaults).
        a = _snap("1940.1.1.0", save_domains=["country_snapshot"])
        b = _snap("1940.1.2.0", save_domains=["country_snapshot"])
        ca = _country(a, "GER", ruling_party="democratic")
        cb = _country(b, "GER", ruling_party="fascism")
        ca["politics"]["ideas_all"] = ["x"]   # present only in a's data
        cb["politics"]["laws"] = {"economy": "war_economy"}
        evs = diff_engine.diff_snapshots(a, b)
        types = _types(evs)
        self.assertIn("government_changed", types)   # country_snapshot domain -> OK
        self.assertNotIn("law_changed", types)       # economy not captured -> gated
        self.assertNotIn("idea_lost", types)

    def test_law_idea_diff_when_economy_captured(self):
        a = _snap("1940.1.1.0", save_domains=["economy"])
        b = _snap("1940.1.2.0", save_domains=["economy"])
        ca = _country(a, "GER"); cb = _country(b, "GER")
        ca["politics"]["laws"] = {"economy": "civilian_economy"}
        cb["politics"]["laws"] = {"economy": "war_economy"}
        self.assertIn("law_changed", _types(diff_engine.diff_snapshots(a, b)))


class TestDivUidReconciliation(unittest.TestCase):
    def test_typed_vs_bare_same_div_is_move_not_recreate(self):
        a = _snap("1940.1.1.0"); b = _snap("1940.1.2.0")
        da = schema.empty_division()
        da.update(div_id=7, div_type=4713, owner="GER", location_province=10)
        db = schema.empty_division()
        db.update(div_id=7, owner="GER", location_province=11)  # memory lacks type
        a["divisions"] = [da]; b["divisions"] = [db]
        self.assertEqual(_types(diff_engine.diff_snapshots(a, b)), ["division_moved"])

    def test_ambiguous_bare_id_not_reconciled(self):
        # two distinct typed divs share bare id 7 -> do NOT merge with a bare-id 7.
        a = _snap("1940.1.1.0"); b = _snap("1940.1.2.0")
        d1 = schema.empty_division(); d1.update(div_id=7, div_type=100, owner="GER")
        d2 = schema.empty_division(); d2.update(div_id=7, div_type=200, owner="GER")
        a["divisions"] = [d1, d2]
        db = schema.empty_division(); db.update(div_id=7, owner="GER")  # ambiguous
        b["divisions"] = [db]
        evs = diff_engine.diff_snapshots(a, b)
        # ambiguous -> falls through to destroy(s)+create, no false move
        self.assertNotIn("division_moved", _types(evs))


class TestOverlordSwitch(unittest.TestCase):
    def test_overlord_change_emits_freed_and_created(self):
        a = _snap("1940.1.1.0"); b = _snap("1940.1.2.0")
        ca = _country(a, "MAN"); cb = _country(b, "MAN")
        ca["politics"]["autonomy"] = {"level": "puppet", "overlord": "JAP"}
        cb["politics"]["autonomy"] = {"level": "puppet", "overlord": "SOV"}
        types = _types(diff_engine.diff_snapshots(a, b))
        self.assertIn("subject_freed", types)
        self.assertIn("subject_created", types)


class TestAllEdgeEventsValidate(unittest.TestCase):
    def test_emitted_events_pass_schema(self):
        a = _snap("1940.1.1.0", save_domains=["country_snapshot", "tech", "diplomacy"])
        b = _snap("1940.1.2.0", save_domains=["country_snapshot", "tech", "diplomacy"])
        ca = _country(a, "GER"); cb = _country(b, "GER")
        ca["research"]["technologies"] = [{"name": "t", "level": 2}]
        cb["research"]["technologies"] = [{"name": "t", "level": 1}]  # tech_lost
        cb["politics"]["autonomy"] = {"level": "puppet", "overlord": "SOV"}  # subject_created
        _country(a, "POL")  # country_removed
        for ev in diff_engine.diff_snapshots(a, b):
            self.assertEqual(schema.validate_event(ev), [], ev)


if __name__ == "__main__":
    unittest.main()
