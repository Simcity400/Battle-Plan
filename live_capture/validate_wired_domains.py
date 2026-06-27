"""validate_wired_domains.py -- LIVE validation harness for the 6 newly-wired
memory domains (country_scalars EXTRAS, country_misc, state_detail, designs, mio,
ships). READ-ONLY. Run ONLY when a memory-scan slot is free (it does the heap
scans the readers need); do NOT run it while other agents are saturating the box.

What it proves, against a RUNNING hoi4.exe (build-gated):
  * each wired plug runs to completion (its domain lands in
    provenance.memory.domains),
  * the snapshot still passes replay.schema.validate_snapshot,
  * the new fields are populated from LIVE memory:
      country.military.manpower.available  (manpower_pool, M)
      country.military.convoys_count       (convoys, M)
      country.misc.{convoys_destroyed,accidents_score,ace_count,cosmetic_tag} (M)
      states.*.manpower_pool / .occupation / .category   (state_detail, M)
      designs.division_templates (+ _source="M")          (designs, M)
      mio_live.by_tag / .by_id (+ source="M")             (mio, M)
      order_of_battle.ships (+ ships_source="M")          (ships, M)
      special_projects.facilities/.projects/.scientists (+ source="M") (projects_live, M)
      country.focus.{focus_tree,completed,current,current_continuous,progress} (focus_live, M)
      country.production.{stockpile,military_lines,military_factories,...}     (production_live, M)
      country.diplomacy.{at_war,casualties,faction_group} + diplomacy_live.{war_relations,
                                                            faction_members}    (diplomacy_live, M)
      country.politics.{party_popularities,ruling_party,ideas,election_frequency,...} (politics_live, M)
      order_of_battle.{armies,army_groups,fleets,theatres} (+ <list>_source="M")  (oob_command, M)

It assembles a snapshot from the LIVE plug results ONLY (no save), so it is a pure
LIVE-channel check. It does NOT write to the durable store and NEVER writes memory.

Usage (from the repo root):
    python -m live_capture.validate_wired_domains
    python -m live_capture.validate_wired_domains --force   # skip build gate (UNVERIFIED)

Exit 0 iff every wired domain captured and validate_snapshot passed.
"""
from __future__ import annotations

import argparse
import os
import sys

_HERE = os.path.dirname(os.path.abspath(__file__))
_REPO = os.path.normpath(os.path.join(_HERE, os.pardir))
_MR = os.path.join(_REPO, "memory_reader")
for _p in (_REPO, _MR):
    if _p not in sys.path:
        sys.path.insert(0, _p)

from replay import schema                       # noqa: E402
from live_capture import registry as _registry  # noqa: E402
from live_capture import assembler as _assembler  # noqa: E402
from live_capture import process_identity as _pi  # noqa: E402

# the 6 domains this pass wired (country_scalars was extended, not new) + the
# special-projects LIVE pools wired in the follow-on pass (projects_live) + the 3
# per-country LIVE domains folded into the canonical save homes under *_live ids
# (tech_live / commanders_live / intel_live).
WIRED = ("country_scalars", "country_misc", "state_detail", "state_vp",
         "designs", "mio", "ships",
         "projects_live", "tech_live", "commanders_live", "intel_live",
         # this pass: LIVE focus (-> country.focus, save-shared home) + LIVE production
         # (-> country.production, a new live-only home).
         "focus_live", "production_live",
         # this pass: LIVE diplomacy (-> country.diplomacy + the top-level diplomacy_live
         # sidecar) + LIVE politics (-> country.politics, save-shared home; PP stays
         # country_scalars-owned).
         "diplomacy_live", "politics_live",
         # this pass: LIVE OOB command hierarchy (-> top-level order_of_battle.{armies,
         # army_groups,fleets,theatres}, save-shared home under a DISTINCT live id; the
         # per-SHIP order_of_battle.ships leaf stays "ships"-owned).
         "oob_command",
         # FINAL pass: LIVE national fuel (-> country.economy.fuel.{current,max}, save-shared
         # home under a DISTINCT live id) + LIVE supply solver INPUTS (-> badged signals on
         # snap["supply"], NOT the exact per-province supply). The defnames name-resolution
         # enrichments (tech_name / laws / mio policies / scientist traits) ride ON the
         # tech_live / politics_live / mio / projects_live plugs when resolve_defnames is set.
         "fuel_live", "supply_inputs",
         # FINAL-2 pass: LIVE bilateral RELATIONS (-> country.diplomacy.{opinion live-first,
         # relation_flags, overlord, autonomy_state, autonomy_ratio}) + LIVE leader DETAIL (the
         # OPT-IN skill/traits enrichment ON the commanders_live army/navy leaders; claimed under
         # the distinct "leader_detail" id when resolve_leader_detail is set).
         "diplo_relations", "leader_detail",
         # DEEP-CRACK pass: the LIVE EXTRACT-residual domains. world scenario DATES
         # (-> world.scenario.{current_date_live,start_date_live}); construction queue + battle-
         # plan orders + opinion-modifier decomposition (-> country.production.construction_queue
         # + order_of_battle.battle_plan_orders + country.diplomacy.opinion_modifiers); active
         # timed DECISIONS + equipment-MARKET (-> country.decisions.timed_live + snap["market_live"]);
         # operative SKILL/XP (corrected body base L=ident-0x8) + tech in-progress UNITS
         # (-> country.intel.operative_skill_xp + research.in_progress_live progress_units);
         # election residuals (-> country.politics.{last_election,elections_allowed,
         # appointed_advisors}); operative roster + cryptology (-> country.intel.{operatives_live,
         # cryptology_live}); per-state/province BUILDING levels (-> states/provinces.*.buildings);
         # equipment-VARIANT catalog (-> designs.equipment_variants); per-division equipment
         # inventory (-> divisions[].equipment).
         "world_meta", "construction_orders", "decisions_market", "operative_xp",
         "politics_residuals", "intel_deep", "buildings_live", "designs_variants",
         "division_equipment")


def _open_proc():
    import winmem
    p = winmem.open_process("hoi4.exe")
    if p is None:
        raise SystemExit("hoi4.exe not running")
    return p


def _load_economy_offsets():
    import json
    path = os.path.join(_MR, "economy_offsets.draft.json")
    try:
        with open(path, "r", encoding="utf-8") as fh:
            return json.load(fh)
    except (OSError, ValueError):
        return {}


def _resolve_live_layers(proc):
    out = {"tag_table": None, "bridge": None, "province_control": None}
    try:
        from live_layers import tag_table as _tt
        out["tag_table"] = _tt.TagTable.locate(proc)
    except Exception as e:
        print("  (tag_table resolve failed: %s)" % e)
    try:
        from live_layers import country_index_bridge as _cib
        out["bridge"] = _cib.CountryIndexBridge.resolve(proc, tbl=out["tag_table"])
    except Exception as e:
        print("  (bridge resolve failed: %s)" % e)
    try:
        from live_layers import province_control as _pc
        out["province_control"] = _pc.ProvinceControl.resolve(proc, tbl=out["tag_table"])
    except Exception as e:
        print("  (province_control resolve failed: %s)" % e)
    return out


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--force", action="store_true",
                    help="skip the build gate (results UNVERIFIED on a non-pinned build)")
    args = ap.parse_args(argv)

    proc = _open_proc()
    try:
        identity = _pi.identify(proc)
        if not identity.build_ok() and not args.force:
            raise SystemExit(
                "build gate: live exe sha256 %s != pinned %s (pass --force to override)."
                % (getattr(identity, "exe_sha256", "?"), _pi.PINNED_EXE_SHA256))

        layers = _resolve_live_layers(proc)
        ctx = _registry.CaptureContext(
            proc=proc, identity=identity,
            tag_table=layers["tag_table"], bridge=layers["bridge"],
            province_control=layers["province_control"],
            # resolve_defnames=True so the final pass also exercises the in-plug name
            # resolution (tech_name / laws / mio policies / scientist traits); it is OFF in
            # the base capture path (the per-registry heap scans are opt-in).
            # resolve_leader_detail=True exercises the OPT-IN commanders_live skill/traits
            # enrichment (a second full leader-pool scan; OFF in the base path) so the
            # "leader_detail" provenance id is claimed.
            extras={"economy_offsets": _load_economy_offsets(),
                    "resolve_defnames": True,
                    "resolve_leader_detail": True},
        )
        # run ONLY the wired plugs (+ their dependencies are resolved already).
        plugs = [p for p in _registry.default_registry() if p.name in WIRED]
        results = _registry.read_all(ctx, plugs)
    finally:
        try:
            proc.close()
        except Exception:
            pass

    # report each plug
    print("\n=== wired-domain plug results ===")
    # leader_detail is NOT its own plug -- it is the OPT-IN enrichment folded INSIDE
    # commanders_live (it sets leader_detail_enriched=True), so it has no DomainResult here.
    _NON_PLUG = {"leader_detail"}
    for name in WIRED:
        if name in _NON_PLUG:
            cl = results.get("commanders_live")
            ran = bool((cl.data or {}).get("leader_detail_enriched")) if (
                cl is not None and cl.domain_captured) else False
            print("  %-16s %s (in commanders_live)" % (name, "enriched" if ran else "OFF"))
            continue
        r = results.get(name)
        if r is None:
            print("  %-16s MISSING" % name)
            continue
        status = ("captured" if r.domain_captured else
                  ("FAILED: %s" % (r.error or "").splitlines()[-1:] if r.available
                   else "unavailable"))
        n = len((r.data or {}).get("countries") or (r.data or {}).get("states")
                or (r.data or {}).get("division_templates")
                or (r.data or {}).get("by_id") or (r.data or {}).get("ships")
                or (r.data or {}).get("facilities") or {})
        print("  %-16s %-10s n=%s" % (name, "captured" if r.domain_captured else "MISS", n))

    snap = _assembler.assemble_snapshot(
        results, game_id="wired_validate", date=None,
        exe_sha256=getattr(identity, "exe_sha256", None), identity=identity)

    probs = schema.validate_snapshot(snap)
    doms = snap["provenance"]["memory"]["domains"]

    print("\n=== snapshot checks ===")
    print("  validate_snapshot:", "OK" if not probs else probs)
    print("  domains captured  :", sorted(doms))

    # spot-check the new fields landed (only assert when their domain captured)
    checks = []
    if "country_scalars" in doms:
        any_mp = any(c["military"]["manpower"].get("available") is not None
                     for c in snap["countries"].values())
        any_cv = any(c["military"].get("convoys_count") is not None
                     for c in snap["countries"].values())
        checks.append(("country.military.manpower.available", any_mp))
        checks.append(("country.military.convoys_count", any_cv))
    if "country_misc" in doms:
        any_misc = any(isinstance(c.get("misc"), dict) and c["misc"]
                       for c in snap["countries"].values())
        checks.append(("country.misc", any_misc))
    if "state_detail" in doms:
        any_sd = any(s.get("manpower_pool") or s.get("category")
                     for s in snap["states"].values())
        checks.append(("states.*.manpower_pool/category", any_sd))
    if "state_vp" in doms:
        # at least one state reports a live VP total > 0 (a 1946 game always has
        # capitals/VP). victory_points_total is MEASURED-present for EVERY enumerated
        # state (a 0 is a measured zero), so the proof is "some state has VP>0".
        any_vp = any(isinstance(s.get("victory_points_total"), int)
                     and s["victory_points_total"] > 0
                     for s in snap["states"].values())
        # the per-province VP map is MEASURED-present (a dict, incl. measured-empty {})
        # on every enumerated state.
        any_byp = any(isinstance(s.get("victory_points_by_province"), dict)
                      for s in snap["states"].values())
        # measured-ZERO is recorded as 0, not null, for at least one zero-VP state.
        any_zero = any(s.get("victory_points_total") == 0
                       for s in snap["states"].values())
        checks.append(("states.*.victory_points_total>0 (M)", any_vp))
        checks.append(("states.*.victory_points_by_province (M, dict)", any_byp))
        checks.append(("states.*.victory_points_total measured-0 kept", any_zero))
    if "designs" in doms:
        checks.append(("designs.division_templates",
                       bool(snap["designs"]["division_templates"])))
    if "mio" in doms:
        checks.append(("mio_live.by_id", bool(snap["mio_live"].get("by_id"))))
    if "ships" in doms:
        checks.append(("order_of_battle.ships",
                       bool(snap["order_of_battle"].get("ships"))))
    if "projects_live" in doms:
        sp = snap.get("special_projects") or {}
        # MEASURED-present, not non-empty: a measured-empty pool ([] + source="M")
        # is a valid live read (a paused world with 0 facilities), so the honest
        # check is `is not None` + source=="M", NOT bool(...) (Codex: don't fail a
        # measured zero). A live 1946 game has 25 facilities / 311 scientists, but
        # the harness must not lie about measured-empty.
        checks.append(("special_projects.facilities measured",
                       sp.get("facilities") is not None))
        checks.append(("special_projects.scientists measured",
                       sp.get("scientists") is not None))
        checks.append(("special_projects.source==M", sp.get("source") == "M"))
    if "tech_live" in doms:
        # at least one country reports a live completed-tech count + slot capacity,
        # and the LIVE active-research leaf is distinct from the save-shaped one.
        any_tc = any(c["research"].get("tech_count") is not None
                     for c in snap["countries"].values())
        any_rs = any(c["research"].get("research_slots") is not None
                     for c in snap["countries"].values())
        any_ipl = any(c["research"].get("in_progress_live") is not None
                      for c in snap["countries"].values())
        checks.append(("country.research.tech_count (M)", any_tc))
        checks.append(("country.research.research_slots (M)", any_rs))
        checks.append(("country.research.in_progress_live (M, distinct leaf)", any_ipl))
    if "commanders_live" in doms:
        any_army = any(c["commanders"].get("army_leaders")
                       for c in snap["countries"].values())
        any_navy = any(c["commanders"].get("navy_leaders")
                       for c in snap["countries"].values())
        # role-split save leaves must NOT be fabricated by the live fold
        no_fab_roles = all(c["commanders"].get("generals") == []
                           and c["commanders"].get("field_marshals") == []
                           for c in snap["countries"].values())
        checks.append(("country.commanders.army_leaders (M, unsplit)", any_army))
        checks.append(("country.commanders.navy_leaders (M)", any_navy))
        checks.append(("role-split save leaves NOT fabricated", no_fab_roles))
    if "intel_live" in doms:
        # at least one agency reports a live name or capacity (nested save shape).
        any_ag = any(isinstance(c["intel"].get("agency"), dict)
                     and (c["intel"]["agency"].get("name") is not None
                          or isinstance(c["intel"]["agency"].get("capacity"), dict))
                     for c in snap["countries"].values())
        checks.append(("country.intel.agency.{name,capacity} (M)", any_ag))
    if "focus_live" in doms:
        # at least one country reports a live focus tree, and the live-only
        # current_continuous leaf folded onto the save-shared country.focus home.
        any_tree = any(c["focus"].get("focus_tree") is not None
                       for c in snap["countries"].values())
        # current_continuous is a NEW live-only leaf; at least one major running a
        # continuous focus should report it (a 1946 game always has some).
        any_cont = any(c["focus"].get("current_continuous") is not None
                       for c in snap["countries"].values())
        # paused must stay RESISTED (the live fold never fabricates a bool here).
        no_fab_paused = all(c["focus"].get("paused") is None
                            for c in snap["countries"].values())
        checks.append(("country.focus.focus_tree (M)", any_tree))
        checks.append(("country.focus.current_continuous (M, live-only leaf)", any_cont))
        checks.append(("country.focus.paused stays RESISTED (None)", no_fab_paused))
    if "production_live" in doms:
        # the NEW live-only home country.production is populated: at least one country
        # reports a measured stockpile + production lines + factory totals.
        any_stock = any(isinstance(c["production"].get("stockpile"), dict)
                        and c["production"]["stockpile"]
                        for c in snap["countries"].values())
        any_lines = any(c["production"].get("military_lines")
                        for c in snap["countries"].values())
        # factory totals are MEASURED-present (a measured 0 is valid), so the honest
        # check is `is not None` for at least one country, not bool(...).
        any_fac = any(c["production"].get("military_factories") is not None
                      for c in snap["countries"].values())
        # the save-side production home (military.equipment_stockpile) is NOT polluted by
        # the live fold (live writes its own country.production home only).
        no_pollute = all(c["military"].get("equipment_stockpile") == []
                         for c in snap["countries"].values())
        checks.append(("country.production.stockpile (M)", any_stock))
        checks.append(("country.production.military_lines (M)", any_lines))
        checks.append(("country.production.military_factories measured", any_fac))
        checks.append(("save military.equipment_stockpile NOT polluted", no_pollute))
    if "diplomacy_live" in doms:
        # at least one country reports a live at_war list (a 1946 game has ~30 belligerents)
        # and a cumulative casualties total; the world sidecar carries the war_relations list
        # + faction_members + source M. faction_group is an OPAQUE int (kept, not resisted).
        any_atwar = any(isinstance(c["diplomacy"].get("at_war"), list)
                        for c in snap["countries"].values())
        any_cas = any(c["diplomacy"].get("casualties") is not None
                      for c in snap["countries"].values())
        any_fg = any(c["diplomacy"].get("faction_group") is not None
                     for c in snap["countries"].values())
        dls = snap.get("diplomacy_live") or {}
        # measured-PRESENT, not non-empty: a measured-empty war_relations [] is a valid live
        # read (a world at total peace), so the honest check is `is not None` + source=="M".
        sidecar_ok = (dls.get("war_relations") is not None
                      and dls.get("faction_members") is not None
                      and dls.get("source") == "M")
        # the save-shaped diplomacy_world must NOT be polluted by the live fold (different
        # identity model -> the live data lives in the diplomacy_live sidecar only).
        dw = snap.get("diplomacy_world") or {}
        dw_clean = (dw.get("wars") == {} and dw.get("factions") == {})
        checks.append(("country.diplomacy.at_war (M)", any_atwar))
        checks.append(("country.diplomacy.casualties (M)", any_cas))
        checks.append(("country.diplomacy.faction_group (M, opaque int)", any_fg))
        checks.append(("diplomacy_live sidecar measured (source==M)", sidecar_ok))
        checks.append(("diplomacy_world NOT polluted by live fold", dw_clean))
    if "politics_live" in doms:
        # at least one country reports a live popularity 4-vector + ruling party + ideas
        # list; election_frequency is EXACT for election-holding governments.
        any_pops = any(isinstance(c["politics"].get("party_popularities"), dict)
                       and c["politics"]["party_popularities"]
                       for c in snap["countries"].values())
        any_ruling = any(c["politics"].get("ruling_party") is not None
                         for c in snap["countries"].values())
        any_ideas = any(isinstance(c["politics"].get("ideas"), list)
                        for c in snap["countries"].values())
        any_freq = any(c["politics"].get("election_frequency") is not None
                       for c in snap["countries"].values())
        # PP-OVERLAP reconciliation: politics_live must NOT have written political_power
        # itself -- but country_scalars (also wired) DOES, so where BOTH ran the leaf is set
        # by country_scalars. The honest check: politics_live did not BREAK PP. We confirm PP
        # is consistent — if country_scalars ran, at least one PP is non-None; the two
        # readers share the same Rosetta so they cannot disagree on a captured country.
        checks.append(("country.politics.party_popularities (M)", any_pops))
        checks.append(("country.politics.ruling_party (M/S)", any_ruling))
        checks.append(("country.politics.ideas (M)", any_ideas))
        checks.append(("country.politics.election_frequency (M)", any_freq))
        # cross-reader PP consistency (only when country_scalars also captured): the live
        # politics PP (ps+0xE0) and the country_scalars PP (record+0x20) share the PP Rosetta,
        # so a captured country must have a non-null PP from country_scalars (politics_live
        # intentionally does not write it). This proves the reconciliation did not leave a hole.
        if "country_scalars" in doms:
            pp_ok = any(c["politics"].get("political_power") is not None
                        for c in snap["countries"].values())
            checks.append(("political_power present (country_scalars-owned)", pp_ok))
    if "oob_command" in doms:
        oob = snap.get("order_of_battle") or {}
        # MEASURED-present, not non-empty: a measured-empty list ([] + <list>_source="M")
        # is a valid live read, so the honest check is `is not None` + source=="M". A live
        # 1946 game has 343 armies / 83 fleets, but the harness must not lie about empty.
        for leaf in ("armies", "army_groups", "fleets", "theatres"):
            checks.append(("order_of_battle.%s measured" % leaf,
                           isinstance(oob.get(leaf), list)))
            checks.append(("order_of_battle.%s_source==M" % leaf,
                           oob.get(leaf + "_source") == "M"))
        # owner is a resolved TAG (or None); membership ids stay RAW ints (NOT tags). Verify
        # at least one army resolved an owner TAG and that member_div_ids are ints.
        armies = oob.get("armies") or []
        any_owner = any(schema.valid_tag(a.get("owner")) for a in armies)
        ids_raw_int = all(all(isinstance(d, int) for d in (a.get("member_div_ids") or []))
                          for a in armies)
        checks.append(("order_of_battle army owner resolved to TAG", any_owner))
        checks.append(("army member_div_ids are raw ints (not tags)", ids_raw_int))
        # missing_count is a MEASURED frame gap surfaced honestly (an int or None per army).
        miss_ok = all((a.get("missing_count") is None
                       or isinstance(a.get("missing_count"), int)) for a in armies)
        checks.append(("army missing_count surfaced (int|None)", miss_ok))
        # the per-SHIP order_of_battle.ships leaf is NOT clobbered by oob_command: if ships
        # also captured it stays a list; either way oob_command did not write a non-list there.
        checks.append(("order_of_battle.ships coexists (list|skeleton)",
                       isinstance(oob.get("ships"), list)))
        # the heap addr "obj" must NEVER reach the snapshot OOB lists.
        no_obj = ("obj" not in repr(armies)
                  and "obj" not in repr(oob.get("fleets") or []))
        checks.append(("no diagnostic 'obj' heap addr leaked", no_obj))
        # orphan task forces (live TF whose fleet parent missed this frame) are surfaced
        # in a sidecar, not silently dropped (measured frame gap). list|absent both OK.
        checks.append(("orphan_task_forces surfaced (list|absent)",
                       oob.get("orphan_task_forces") is None
                       or isinstance(oob.get("orphan_task_forces"), list)))
    if "fuel_live" in doms:
        # at least one country reports a measured max fuel (the proof field). current fuel
        # may legitimately read at-or-below the save; both fold onto economy.fuel.{max,current}.
        any_max = any(isinstance(c["economy"].get("fuel"), dict)
                      and c["economy"]["fuel"].get("max") is not None
                      for c in snap["countries"].values())
        any_cur = any(isinstance(c["economy"].get("fuel"), dict)
                      and c["economy"]["fuel"].get("current") is not None
                      for c in snap["countries"].values())
        checks.append(("country.economy.fuel.max (M, proof field)", any_max))
        checks.append(("country.economy.fuel.current (M)", any_cur))
    if "supply_inputs" in doms:
        sblk = snap.get("supply") or {}
        # the solver-input signals are DISTINCTLY named + SEPARATELY badged (never the proxy
        # confidence). measured-present (a sparse dict) + source M + the not-exact confidence.
        eff_ok = isinstance(sblk.get("solver_efficiency_by_province"), dict)
        draw_ok = isinstance(sblk.get("solver_draw_by_province"), dict)
        badge_ok = (sblk.get("solver_inputs_source") == "M"
                    and sblk.get("solver_inputs_confidence")
                    == "live_solver_input_not_exact_supply")
        checks.append(("supply.solver_efficiency_by_province (M)", eff_ok))
        checks.append(("supply.solver_draw_by_province (M)", draw_ok))
        checks.append(("supply solver-input badge (source M + not-exact confidence)", badge_ok))
    if "diplo_relations" in doms:
        # at least one country reports a live opinion matrix (a 1946 game has dense opinion)
        # + a relation-flag map; subjects/autonomy present for at least one subject (17 in 1946).
        any_op = any(isinstance(c["diplomacy"].get("opinion"), dict)
                     and c["diplomacy"]["opinion"]
                     for c in snap["countries"].values())
        any_rf = any(isinstance(c["diplomacy"].get("relation_flags"), dict)
                     and c["diplomacy"]["relation_flags"]
                     for c in snap["countries"].values())
        any_subj = any(c["diplomacy"].get("overlord") is not None
                       for c in snap["countries"].values())
        # opinion folds onto the SAVE-SHARED leaf; the live-only diplomacy_live sidecar (wars/
        # factions) must NOT have absorbed the opinion matrix (different identity model).
        dls = snap.get("diplomacy_live") or {}
        no_pollute = "opinion" not in dls and "relation_flags" not in dls
        checks.append(("country.diplomacy.opinion (M, bilateral)", any_op))
        checks.append(("country.diplomacy.relation_flags (M)", any_rf))
        checks.append(("country.diplomacy.overlord/autonomy (M, subjects)", any_subj))
        checks.append(("diplomacy_live sidecar NOT polluted by opinion matrix", no_pollute))
    if "leader_detail" in doms:
        # the OPT-IN enrichment joined skill + traits onto the commanders_live leaders. At least
        # one army/navy leader should carry a measured skill (953/953 in 1946) + a traits list.
        leaders = [ld for c in snap["countries"].values()
                   for ld in (c["commanders"].get("army_leaders") or [])
                   + (c["commanders"].get("navy_leaders") or [])]
        any_skill = any(isinstance(ld, dict) and ld.get("skill") is not None
                        for ld in leaders)
        any_traits = any(isinstance(ld, dict) and isinstance(ld.get("traits"), list)
                         for ld in leaders)
        checks.append(("commanders leader skill (M, leader_detail)", any_skill))
        checks.append(("commanders leader traits (M, leader_detail)", any_traits))
    # --- DEEP-CRACK residual domains ---
    if "world_meta" in doms:
        scen = (snap.get("world") or {}).get("scenario") or {}
        # the live current date should be present + EXACT (a 1946 game). start_date_live too.
        checks.append(("world.scenario.current_date_live (M)",
                       isinstance(scen.get("current_date_live"), str)))
        checks.append(("world.scenario.start_date_live (M)",
                       isinstance(scen.get("start_date_live"), str)))
        # world_tension is COMPUTED-not-stored -> NEVER fabricated onto the scenario block.
        checks.append(("world_tension NOT fabricated (resisted)",
                       "world_tension" not in scen))
    if "construction_orders" in doms:
        # at least one country reports a live construction queue (a 1946 game always builds);
        # the battle-plan orders list is measured-present on the OOB dict.
        any_cq = any(isinstance(c["production"].get("construction_queue"), list)
                     and c["production"]["construction_queue"]
                     for c in snap["countries"].values())
        oob = snap.get("order_of_battle") or {}
        bpo_ok = isinstance(oob.get("battle_plan_orders"), list)
        any_om = any(isinstance(c["diplomacy"].get("opinion_modifiers"), dict)
                     and c["diplomacy"]["opinion_modifiers"]
                     for c in snap["countries"].values())
        # non-type-5 order geometry stays RESISTED: any order with order_type != 5 must carry
        # target_states=None (never a fabricated []).
        no_fab_geo = all(o.get("target_states") is None
                         for o in (oob.get("battle_plan_orders") or [])
                         if o.get("order_type") != 5)
        checks.append(("country.production.construction_queue (M)", any_cq))
        checks.append(("order_of_battle.battle_plan_orders measured (list)", bpo_ok))
        checks.append(("country.diplomacy.opinion_modifiers (M)", any_om))
        checks.append(("non-type-5 order geometry RESISTED (None)", no_fab_geo))
    if "decisions_market" in doms:
        any_dec = any(isinstance(c["decisions"].get("timed_live"), list)
                      for c in snap["countries"].values())
        mkl = snap.get("market_live") or {}
        market_ok = (isinstance(mkl.get("contracts"), list) and mkl.get("source") == "M")
        # contracts carry resolved seller/buyer TAGs (or None) -- NEVER a raw tag-db-id / ptr.
        no_raw = all(("seller_tag_db_id" not in ct and "buyer_ptr" not in ct)
                     and (ct.get("seller") is None or schema.valid_tag(ct.get("seller")))
                     for ct in (mkl.get("contracts") or []))
        checks.append(("country.decisions.timed_live measured (list)", any_dec))
        checks.append(("market_live.contracts measured (source==M)", market_ok))
        checks.append(("market contract seller/buyer resolved to TAG (no raw id)", no_raw))
    if "operative_xp" in doms:
        # at least one country reports a live operative skill/xp map.
        any_osx = any(isinstance(c["intel"].get("operative_skill_xp"), dict)
                      and c["intel"]["operative_skill_xp"]
                      for c in snap["countries"].values())
        # tech progress_units merged onto an in_progress_live slot (NOT replacing the raw shape).
        any_pu = any(isinstance(slot, dict) and slot.get("progress_units") is not None
                     and slot.get("progress_raw") is not None  # raw handle shape PRESERVED
                     for c in snap["countries"].values()
                     for slot in (c["research"].get("in_progress_live") or []))
        checks.append(("country.intel.operative_skill_xp (M)", any_osx))
        checks.append(("research.in_progress_live progress_units merged (raw preserved)", any_pu))
    if "politics_residuals" in doms:
        any_le = any(c["politics"].get("last_election") is not None
                     for c in snap["countries"].values())
        any_ea = any(c["politics"].get("elections_allowed") is not None
                     for c in snap["countries"].values())
        checks.append(("country.politics.last_election (M)", any_le))
        checks.append(("country.politics.elections_allowed (M)", any_ea))
    if "intel_deep" in doms:
        any_ol = any(isinstance(c["intel"].get("operatives_live"), list)
                     and c["intel"]["operatives_live"]
                     for c in snap["countries"].values())
        # operatives_live carries no diagnostic heap addr / raw tag-db-id.
        no_diag = all(("leader_addr" not in op and "identity_addr" not in op
                       and "owner_tag_db_id" not in op)
                      for c in snap["countries"].values()
                      for op in (c["intel"].get("operatives_live") or []))
        checks.append(("country.intel.operatives_live (M)", any_ol))
        checks.append(("operatives_live no diagnostic addr / raw id leaked", no_diag))
    if "buildings_live" in doms:
        any_sb = any(isinstance(s.get("buildings"), dict) and s["buildings"]
                     for s in snap["states"].values())
        any_pb = any(isinstance(pv.get("buildings"), dict) and pv["buildings"]
                     for pv in snap["provinces"].values())
        checks.append(("states.*.buildings (M)", any_sb))
        checks.append(("provinces.*.buildings (M)", any_pb))
    if "designs_variants" in doms:
        ev = (snap.get("designs") or {}).get("equipment_variants_live")
        checks.append(("designs.equipment_variants_live (M)",
                       isinstance(ev, dict) and bool(ev)))
        checks.append(("designs.equipment_variants_live_source==M",
                       (snap.get("designs") or {}).get("equipment_variants_live_source") == "M"))
    # NOTE: division_equipment FOLDS onto divisions[].equipment, but the "divisions" domain is
    # NOT in this LIVE harness's WIRED set, so snap["divisions"] is empty here and the join has
    # nothing to enrich. The DOMAIN still captures (the plug ran to completion); the per-division
    # join is exercised by the reader's own _validate (validated vs the melt). So this is an
    # INFORMATIONAL line, NOT a PASS/FAIL gate (mirrors the defnames-enrichment convention) --
    # gating it would falsely fail on a harness that intentionally skips the divisions domain.
    if "division_equipment" in doms:
        de = results.get("division_equipment")
        n_div = len((de.data or {}).get("by_div") or {}) if (
            de is not None and de.domain_captured) else 0
        print("  (info) division_equipment live divisions read:", n_div)
    # --- defnames name-resolution enrichments (present only when resolve_defnames + a known
    # handle). These are ADDITIVE -- their absence is not a domain failure, so they are
    # reported informationally, NOT folded into the PASS/FAIL gate (a handle the registry
    # does not know is honestly left unresolved).
    if "tech_live" in doms:
        any_tn = any(isinstance(slot, dict) and slot.get("tech_name")
                     for c in snap["countries"].values()
                     for slot in (c["research"].get("in_progress_live") or []))
        print("  (info) tech in-progress tech_name resolved:", any_tn)
    if "politics_live" in doms:
        any_laws = any(isinstance(c["politics"].get("laws"), dict)
                       and c["politics"]["laws"]
                       for c in snap["countries"].values())
        print("  (info) politics laws triple classified:", any_laws)
    if "mio" in doms:
        any_pol = any(isinstance(r, dict) and r.get("policies")
                      for r in (snap.get("mio_live", {}).get("by_id") or {}).values())
        print("  (info) mio policies resolved:", any_pol)
    if "projects_live" in doms:
        any_tr = any(isinstance(s, dict) and "traits" in s
                     for s in (snap.get("special_projects", {}).get("scientists") or []))
        print("  (info) scientist traits resolved:", any_tr)

    print("\n=== field-population spot checks ===")
    for label, ok in checks:
        print("  %-40s %s" % (label, "OK" if ok else "EMPTY"))

    all_captured = all(d in doms for d in WIRED)
    field_ok = all(ok for _, ok in checks)
    ok = (not probs) and all_captured and field_ok
    print("\nRESULT:", "PASS" if ok else "FAIL",
          "(all wired domains captured, validate clean, fields populated)"
          if ok else "(see above)")
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
