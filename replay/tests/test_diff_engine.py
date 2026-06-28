"""Tests for replay/diff_engine.py — synthetic before/after snapshots covering
every event type, plus the merge/dedupe + confidence rules."""
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))

from replay import schema, diff_engine  # noqa: E402


def _snap(date, *, memory_domains=None):
    s = schema.empty_snapshot("g", date)
    if memory_domains is not None:
        s["provenance"]["memory"] = {"captured": True, "domains": memory_domains}
    return s


def _add_country(snap, tag, **politics):
    c = schema.empty_country()
    c["politics"].update(politics)
    snap["countries"][tag] = c
    return c


def _add_state(snap, sid, owner=None, controller=None):
    st = schema.empty_state()
    st["owner"] = owner
    st["controller"] = controller
    snap["states"][str(sid)] = st
    return st


def _add_div(snap, did, owner, prov, tpl, name="T", state_id=None):
    d = schema.empty_division()
    d.update(div_id=did, owner=owner, location_province=prov, template_id=tpl,
             template_name=name, state_id=state_id)
    snap["divisions"].append(d)
    return d


def _types(events):
    return [e["type"] for e in events]


class TestStateProvinceEvents(unittest.TestCase):
    def test_owner_and_controller_change(self):
        a = _snap("1940.1.1.0"); b = _snap("1940.1.2.0")
        _add_state(a, 1, owner="POL", controller="POL")
        _add_state(b, 1, owner="GER", controller="GER")
        evs = diff_engine.diff_snapshots(a, b)
        types = _types(evs)
        self.assertIn("state_owner_change", types)
        self.assertIn("state_controller_change", types)
        owner_ev = next(e for e in evs if e["type"] == "state_owner_change")
        self.assertEqual(owner_ev["from"], "POL")
        self.assertEqual(owner_ev["to"], "GER")
        self.assertEqual(owner_ev["key"], "1")

    def test_province_controller_change(self):
        a = _snap("1940.1.1.0"); b = _snap("1940.1.2.0")
        a["provinces"]["5"] = {"controller": "FRA", "state_id": None}
        b["provinces"]["5"] = {"controller": "GER", "state_id": None}
        evs = diff_engine.diff_snapshots(a, b)
        self.assertEqual(_types(evs), ["province_controller_change"])
        self.assertEqual(evs[0]["to"], "GER")

    def test_no_event_when_unchanged(self):
        a = _snap("1940.1.1.0"); b = _snap("1940.1.2.0")
        _add_state(a, 1, owner="GER", controller="GER")
        _add_state(b, 1, owner="GER", controller="GER")
        self.assertEqual(diff_engine.diff_snapshots(a, b), [])


class TestDivisionEvents(unittest.TestCase):
    def test_created_destroyed_moved_retemplate(self):
        a = _snap("1940.1.1.0"); b = _snap("1940.1.2.0")
        _add_div(a, 1, "GER", 100, 50, state_id=10)   # will move + retemplate
        _add_div(a, 2, "GER", 200, 51)                # will be destroyed
        _add_div(b, 1, "GER", 101, 99, state_id=11)   # moved + new template
        _add_div(b, 3, "GER", 300, 52)                # created
        evs = diff_engine.diff_snapshots(a, b)
        types = set(_types(evs))
        self.assertEqual(types, {"division_created", "division_destroyed",
                                 "division_moved", "division_template_changed"})
        moved = next(e for e in evs if e["type"] == "division_moved")
        self.assertEqual((moved["from"], moved["to"]), (100, 101))
        self.assertEqual(moved["detail"]["from_state"], 10)
        self.assertEqual(moved["detail"]["to_state"], 11)
        created = next(e for e in evs if e["type"] == "division_created")
        self.assertEqual(created["key"], "3")
        destroyed = next(e for e in evs if e["type"] == "division_destroyed")
        self.assertEqual(destroyed["key"], "2")


class TestCountryEvents(unittest.TestCase):
    def test_focus_completed(self):
        a = _snap("1940.1.1.0"); b = _snap("1940.1.2.0")
        ca = _add_country(a, "GER"); cb = _add_country(b, "GER")
        ca["focus"]["completed"] = ["f1"]
        cb["focus"]["completed"] = ["f1", "f2", "f3"]
        evs = diff_engine.diff_snapshots(a, b)
        keys = sorted(e["key"] for e in evs if e["type"] == "focus_completed")
        self.assertEqual(keys, ["f2", "f3"])

    def test_law_and_government_change(self):
        a = _snap("1940.1.1.0"); b = _snap("1940.1.2.0")
        ca = _add_country(a, "GER", ruling_party="democratic")
        cb = _add_country(b, "GER", ruling_party="fascism")
        ca["politics"]["laws"] = {"economy": "civilian_economy", "trade": None,
                                  "conscription": "volunteer_only"}
        cb["politics"]["laws"] = {"economy": "war_economy", "trade": None,
                                  "conscription": "volunteer_only"}
        evs = diff_engine.diff_snapshots(a, b)
        types = _types(evs)
        self.assertIn("government_changed", types)
        law = next(e for e in evs if e["type"] == "law_changed")
        self.assertEqual(law["key"], "economy")
        self.assertEqual((law["from"], law["to"]), ("civilian_economy", "war_economy"))

    def test_idea_gained_lost(self):
        a = _snap("1940.1.1.0"); b = _snap("1940.1.2.0")
        ca = _add_country(a, "GER"); cb = _add_country(b, "GER")
        ca["politics"]["ideas_all"] = ["x", "y"]
        cb["politics"]["ideas_all"] = ["y", "z"]
        evs = diff_engine.diff_snapshots(a, b)
        gained = [e["key"] for e in evs if e["type"] == "idea_gained"]
        lost = [e["key"] for e in evs if e["type"] == "idea_lost"]
        self.assertEqual(gained, ["z"])
        self.assertEqual(lost, ["x"])

    def test_tech_researched_and_slot(self):
        a = _snap("1940.1.1.0"); b = _snap("1940.1.2.0")
        ca = _add_country(a, "GER"); cb = _add_country(b, "GER")
        ca["research"]["technologies"] = [{"name": "t1", "level": 1}]
        ca["research"]["research_slots"] = 3
        cb["research"]["technologies"] = [{"name": "t1", "level": 1},
                                          {"name": "t2", "level": 1}]
        cb["research"]["research_slots"] = 4
        evs = diff_engine.diff_snapshots(a, b)
        tech = [e for e in evs if e["type"] == "tech_researched"]
        self.assertEqual([e["key"] for e in tech], ["t2"])
        self.assertTrue(any(e["type"] == "research_slot_gained" for e in evs))

    def test_tech_level_upgrade_emits(self):
        a = _snap("1940.1.1.0"); b = _snap("1940.1.2.0")
        ca = _add_country(a, "GER"); cb = _add_country(b, "GER")
        ca["research"]["technologies"] = [{"name": "t1", "level": 1}]
        cb["research"]["technologies"] = [{"name": "t1", "level": 2}]
        evs = diff_engine.diff_snapshots(a, b)
        tech = [e for e in evs if e["type"] == "tech_researched"]
        self.assertEqual(len(tech), 1)
        self.assertEqual(tech[0]["to"], 2)

    def test_war_and_peace_and_faction(self):
        a = _snap("1940.1.1.0"); b = _snap("1940.1.2.0")
        ca = _add_country(a, "GER"); cb = _add_country(b, "GER")
        ca["diplomacy"]["wars"] = [{"against": "FRA"}]
        cb["diplomacy"]["wars"] = [{"against": "SOV"}]  # FRA peace, SOV war
        ca["politics"]["faction"] = None
        cb["politics"]["faction"] = {"name": "axis"}
        evs = diff_engine.diff_snapshots(a, b)
        types = _types(evs)
        self.assertIn("war_declared", types)
        self.assertIn("peace", types)
        self.assertIn("faction_joined", types)
        war = next(e for e in evs if e["type"] == "war_declared")
        self.assertEqual(war["object"], "SOV")
        peace = next(e for e in evs if e["type"] == "peace")
        self.assertEqual(peace["object"], "FRA")

    def test_faction_switch_emits_leave_and_join(self):
        a = _snap("1940.1.1.0"); b = _snap("1940.1.2.0")
        ca = _add_country(a, "ITA"); cb = _add_country(b, "ITA")
        ca["politics"]["faction"] = {"name": "axis"}
        cb["politics"]["faction"] = {"name": "allies"}
        evs = diff_engine.diff_snapshots(a, b)
        types = _types(evs)
        self.assertIn("faction_left", types)
        self.assertIn("faction_joined", types)

    def test_factory_and_production_milestones(self):
        a = _snap("1940.1.1.0"); b = _snap("1940.1.2.0")
        ca = _add_country(a, "GER"); cb = _add_country(b, "GER")
        ca["economy"]["factories"] = {"civ": 48, "mil": 95, "dockyard": 9}
        cb["economy"]["factories"] = {"civ": 52, "mil": 105, "dockyard": 11}
        ca["economy"]["_production_lines"] = [{"archetype": "inf_eq", "produced": 90.0}]
        cb["economy"]["_production_lines"] = [{"archetype": "inf_eq", "produced": 160.0}]
        evs = diff_engine.diff_snapshots(a, b, milestone_steps={"factory": 50,
                                                                "production": 100})
        types = _types(evs)
        # civ 48->52 crosses 50; mil 95->105 crosses 100; production 90->160 crosses 100
        fac = [e for e in evs if e["type"] == "factory_milestone"]
        self.assertEqual({e["key"] for e in fac}, {"civ", "mil"})
        self.assertIn("production_milestone", types)


class TestConfidenceAndMerge(unittest.TestCase):
    def test_save_diff_is_interval_with_start_hour(self):
        a = _snap("1940.1.1.0"); b = _snap("1940.1.2.0")
        _add_state(a, 1, owner="POL"); _add_state(b, 1, owner="GER")
        ev = diff_engine.diff_snapshots(a, b)[0]
        self.assertEqual(ev["confidence"], "interval")
        self.assertEqual(ev["source"], "D")
        self.assertEqual(ev["detail"]["interval_start_hour"], a["hour_index"])

    def test_memory_backed_diff_is_exact(self):
        a = _snap("1940.1.1.0", memory_domains=["state_ownership"])
        b = _snap("1940.1.2.0", memory_domains=["state_ownership"])
        _add_state(a, 1, owner="POL"); _add_state(b, 1, owner="GER")
        ev = diff_engine.diff_snapshots(a, b)[0]
        self.assertEqual(ev["confidence"], "exact")
        self.assertEqual(ev["source"], "M")

    def test_collector_backed_diff_is_exact_source_c(self):
        a = _snap("1940.1.1.0"); b = _snap("1940.1.2.0")
        a["provenance"]["collector"] = {"captured": True, "domains": ["state_ownership"]}
        b["provenance"]["collector"] = {"captured": True, "domains": ["state_ownership"]}
        _add_state(a, 1, owner="POL"); _add_state(b, 1, owner="GER")
        ev = diff_engine.diff_snapshots(a, b)[0]
        self.assertEqual(ev["confidence"], "exact")
        self.assertEqual(ev["source"], "C")

    def test_collector_backed_state_and_province_control_are_exact_source_c(self):
        a = _snap("1940.1.1.0"); b = _snap("1940.1.2.0")
        domains = ["state_control", "province_control"]
        a["provenance"]["collector"] = {"captured": True, "domains": domains}
        b["provenance"]["collector"] = {"captured": True, "domains": domains}
        _add_state(a, 1, controller="POL")
        _add_state(b, 1, controller="GER")
        a["provinces"]["5"] = {"controller": "POL", "state_id": 1}
        b["provinces"]["5"] = {"controller": "GER", "state_id": 1}

        evs = diff_engine.diff_snapshots(a, b)

        self.assertEqual(_types(evs),
                         ["province_controller_change", "state_controller_change"])
        for ev in evs:
            self.assertEqual(ev["confidence"], "exact")
            self.assertEqual(ev["source"], "C")

    def test_first_snapshot_no_baseline_events(self):
        b = _snap("1940.1.2.0")
        self.assertEqual(diff_engine.diff_snapshots(None, b), [])

    def test_build_timeline_orders_and_dedupes(self):
        s1 = _snap("1940.1.1.0")
        s2 = _snap("1940.1.2.0")
        s3 = _snap("1940.1.3.0")
        c1 = _add_country(s1, "GER"); c2 = _add_country(s2, "GER"); c3 = _add_country(s3, "GER")
        c1["focus"]["completed"] = []
        c2["focus"]["completed"] = ["fA"]   # diff: focus fA between s1,s2
        c3["focus"]["completed"] = ["fA"]
        # exact marker for the same focus on s2
        s2["events"] = [schema.make_event("focus_completed", subject="GER", key="fA",
                                          to="fA", source="marker", confidence="exact",
                                          hour_index=s2["hour_index"], date=s2["date"])]
        tl = diff_engine.build_timeline([s3, s1, s2])  # deliberately unordered input
        focus_evs = [e for e in tl if e["type"] == "focus_completed" and e["key"] == "fA"]
        # marker supersedes the diff-derived duplicate -> exactly one, and it's exact
        self.assertEqual(len(focus_evs), 1)
        self.assertEqual(focus_evs[0]["confidence"], "exact")
        self.assertEqual(focus_evs[0]["source"], "marker")
        # timeline sorted by hour_index
        his = [e["hour_index"] for e in tl]
        self.assertEqual(his, sorted(his))

    def test_marker_with_literal_none_string_does_not_eat_real_diff(self):
        # BUG (Codex): _dedupe_key used str(field), so a diff event with a real
        # `from=None` (owner null -> GER) collided with a marker carrying the literal
        # STRING "None" -> the marker wrongly superseded a genuinely-distinct diff.
        # None and the literal "None" must NEVER share a dedupe identity.
        s1 = _snap("1940.1.1.0")
        s2 = _snap("1940.1.2.0")
        _add_state(s1, 1, owner=None)          # state owner truly null at s1
        _add_state(s2, 1, owner="GER")          # -> diff: from None to GER
        # a marker at s2 with the SAME (type,key,to,subject) but a literal string
        # "None" for `from`/`object` (a different, distinct transition). It must NOT
        # supersede the real None->GER diff.
        s2["events"] = [schema.make_event(
            "state_owner_change", key="1", subject="GER", object="None",
            to="GER", source="marker", confidence="exact",
            hour_index=s2["hour_index"], date=s2["date"], **{"from": "None"})]
        tl = diff_engine.build_timeline([s1, s2])
        owner_evs = [e for e in tl if e["type"] == "state_owner_change" and e["key"] == "1"]
        # both must survive: the marker ("from"=="None") and the real diff (from=None)
        self.assertEqual(
            len(owner_evs), 2,
            "a literal 'None' marker must not supersede a real None->X diff: %r"
            % [e.get("from") for e in owner_evs])

    def test_all_emitted_events_validate(self):
        a = _snap("1940.1.1.0"); b = _snap("1940.1.2.0")
        ca = _add_country(a, "GER"); cb = _add_country(b, "GER")
        ca["focus"]["completed"] = []; cb["focus"]["completed"] = ["f1"]
        _add_state(a, 1, owner="POL"); _add_state(b, 1, owner="GER")
        _add_div(b, 9, "GER", 1, 1)
        for ev in diff_engine.diff_snapshots(a, b):
            self.assertEqual(schema.validate_event(ev), [], ev)


class TestLiveAliasGating(unittest.TestCase):
    """NO-FABRICATION gate (Codex-verified): a `<domain>_live` channel may satisfy a
    save-shaped canonical gate ONLY when it actually writes that reader's save-shaped
    leaves. The live channels write DIFFERENT leaves than the save-shaped diff readers
    consume (tech_live -> counts not technologies[]; diplomacy_live -> at_war[] not
    wars[]; intel_live -> agency not running ops; ...), so they MUST fail closed — else a
    pure/mixed live snapshot fabricates tech_lost / peace / operation_completed. Only
    focus_live -> focus is leaf-compatible + additions-only-safe."""

    def _mem(self, *domains):
        return {"provenance": {"memory": {"captured": True, "domains": list(domains)},
                               "save": None}}

    def test_unsafe_live_aliases_do_not_satisfy_gate(self):
        # Every live channel whose shape differs from its save-shaped reader: the live id
        # must NOT satisfy the canonical gate (fail closed -> no fabrication).
        cases = {
            "tech": "tech_live", "commanders": "commanders_live", "intel": "intel_live",
            "production": "production_live", "projects": "projects_live",
            "diplomacy": "diplomacy_live", "designs": "mio",
            "order_of_battle": "oob_command",
        }
        for canon, live_id in cases.items():
            live = self._mem(live_id)
            self.assertFalse(diff_engine._domain_is_memory(live, canon),
                             "%s must not be satisfied by %s" % (canon, live_id))
            self.assertFalse(diff_engine._domain_present(live, canon), canon)
            self.assertFalse(diff_engine._both_captured(live, live, canon), canon)

    def test_bare_designs_canonical_id_fails_closed(self):
        # The live channel ALSO records the BARE "designs" id, but writes only
        # division_templates (not equipment_variants / country.mios) -> the "designs"
        # gate (design_created / mio_leveled) must NOT treat memory "designs" as captured.
        live = self._mem("designs")
        self.assertFalse(diff_engine._domain_is_memory(live, "designs"))
        self.assertFalse(diff_engine._both_captured(live, live, "designs"))

    def test_focus_live_is_the_one_safe_alias(self):
        # focus_live writes focus.completed in the same id namespace; _diff_focus is
        # additions-only -> safe. It MUST still satisfy the focus gate, memory-exact.
        live = self._mem("focus_live")
        self.assertTrue(diff_engine._domain_is_memory(live, "focus"))
        self.assertTrue(diff_engine._both_captured(live, live, "focus"))
        self.assertEqual(diff_engine._confidence_for("focus", live, live), "exact")
        self.assertEqual(diff_engine._source_for("focus", live, live), "M")

    def test_shape_compatible_bare_canonical_ids_still_work(self):
        # state_control / province_control / divisions write the SAVE-shaped leaves their
        # readers read, so the bare canonical id must KEEP satisfying the gate.
        for canon in ("state_control", "province_control", "divisions"):
            live = self._mem(canon)
            self.assertTrue(diff_engine._domain_is_memory(live, canon), canon)
            self.assertTrue(diff_engine._both_captured(live, live, canon), canon)

    def test_save_only_is_present_but_not_memory(self):
        sv = {"provenance": {"memory": {"captured": True, "domains": []},
                             "save": {"captured": True, "domains": ["tech"]}}}
        self.assertFalse(diff_engine._domain_is_memory(sv, "tech"))
        self.assertTrue(diff_engine._domain_present(sv, "tech"))   # gate still runs via save
        self.assertEqual(diff_engine._source_for("tech", sv, sv), "S")

    def test_no_false_positive(self):
        live = self._mem("tech_live")
        self.assertFalse(diff_engine._domain_is_memory(live, "diplomacy"))
        self.assertFalse(diff_engine._domain_is_memory(live, "commanders"))
        self.assertFalse(diff_engine._domain_present(live, "intel"))


class TestLiveAliasNoFabrication(unittest.TestCase):
    """End-to-end no-fabrication proof: on a MIXED save+live snapshot pair, the live
    channel's presence must NOT fabricate a save-semantic event for data it does not
    carry — but a GENUINE such event IS still emitted when the real save data is present.
    """

    def _prov(self, snap, *, save, memory):
        snap["provenance"]["save"] = {"captured": bool(save), "domains": list(save)}
        snap["provenance"]["memory"] = {"captured": bool(memory), "domains": list(memory)}
        return snap

    # ---- tech_lost ----------------------------------------------------------
    def _tech_country(self, snap, tag, techs):
        c = schema.empty_country()
        c["research"]["technologies"] = [{"name": n, "level": l} for n, l in techs]
        snap["countries"][tag] = c
        return c

    def test_mixed_tech_live_does_not_fabricate_tech_lost(self):
        # a = save tech (has techs). b = the live channel reports tech_live (counts only,
        # technologies[] empty) and did NOT capture save tech. The reader must NOT run ->
        # no fabricated tech_lost for the techs that "vanished" from the empty live shape.
        a = _snap("1940.1.1.0"); b = _snap("1940.1.2.0")
        self._prov(a, save=["tech"], memory=[])
        self._prov(b, save=[], memory=["tech_live"])  # pure-live tech, save-shape empty
        self._tech_country(a, "GER", [("infantry_weapons", 2), ("artillery", 1)])
        self._tech_country(b, "GER", [])  # live shape carries no technologies[]
        self.assertNotIn("tech_lost", _types(diff_engine.diff_snapshots(a, b)))

    def test_genuine_tech_lost_still_emitted_when_save_present(self):
        a = _snap("1940.1.1.0"); b = _snap("1940.1.2.0")
        self._prov(a, save=["tech"], memory=[])
        self._prov(b, save=["tech"], memory=[])  # both real save tech
        self._tech_country(a, "GER", [("infantry_weapons", 2), ("artillery", 1)])
        self._tech_country(b, "GER", [("infantry_weapons", 2)])  # artillery really gone
        types = _types(diff_engine.diff_snapshots(a, b))
        self.assertIn("tech_lost", types)

    # ---- peace --------------------------------------------------------------
    def _war_country(self, snap, tag, enemies):
        c = schema.empty_country()
        c["diplomacy"]["wars"] = [{"against": e} for e in enemies]
        snap["countries"][tag] = c
        return c

    def test_mixed_diplomacy_live_does_not_fabricate_peace(self):
        a = _snap("1940.1.1.0"); b = _snap("1940.1.2.0")
        self._prov(a, save=["diplomacy"], memory=[])
        self._prov(b, save=[], memory=["diplomacy_live"])  # live writes at_war[], not wars[]
        self._war_country(a, "GER", ["POL", "FRA"])
        self._war_country(b, "GER", [])  # live shape has empty wars[]
        self.assertNotIn("peace", _types(diff_engine.diff_snapshots(a, b)))

    def test_genuine_peace_still_emitted_when_save_present(self):
        a = _snap("1940.1.1.0"); b = _snap("1940.1.2.0")
        self._prov(a, save=["diplomacy"], memory=[])
        self._prov(b, save=["diplomacy"], memory=[])
        self._war_country(a, "GER", ["POL", "FRA"])
        self._war_country(b, "GER", ["FRA"])  # POL war really ended
        self.assertIn("peace", _types(diff_engine.diff_snapshots(a, b)))

    # ---- operation_completed ------------------------------------------------
    def _ops_country(self, snap, tag, op_ids):
        c = schema.empty_country()
        c["intel"]["operations"] = {"running": [{"op_id": i, "type": "t", "target": "X"}
                                                 for i in op_ids]}
        snap["countries"][tag] = c
        return c

    def test_mixed_intel_live_does_not_fabricate_operation_completed(self):
        a = _snap("1940.1.1.0"); b = _snap("1940.1.2.0")
        self._prov(a, save=["intel"], memory=[])
        self._prov(b, save=[], memory=["intel_live"])  # live writes agency.*, not running ops
        self._ops_country(a, "GER", [11, 22])
        self._ops_country(b, "GER", [])  # live shape carries no running ops
        self.assertNotIn("operation_completed", _types(diff_engine.diff_snapshots(a, b)))

    def test_genuine_operation_completed_still_emitted_when_save_present(self):
        a = _snap("1940.1.1.0"); b = _snap("1940.1.2.0")
        self._prov(a, save=["intel"], memory=[])
        self._prov(b, save=["intel"], memory=[])
        self._ops_country(a, "GER", [11, 22])
        self._ops_country(b, "GER", [11])  # op 22 really completed
        self.assertIn("operation_completed", _types(diff_engine.diff_snapshots(a, b)))


if __name__ == "__main__":
    unittest.main()
