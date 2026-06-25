"""Diff / event engine for the replay backbone.

Given two adjacent SNAPSHOTs (a < b by hour_index), derive the EVENTS that occurred
between them (SCHEMA §9). Then merge a whole ordered series of snapshots into one
chronological event timeline, folding in any per-snapshot marker events and applying
marker-supersedes-diff dedupe (SCHEMA §9.3).

Pure stdlib, no I/O. Every emitted event is a dict shaped per schema.Event.

Confidence model (SCHEMA §9.3):
  * a diff on a layer whose source is memory (M) would be "exact"; today the
    ingested layers are save-backed, so cross-snapshot diffs are "interval"
    (the change happened somewhere in (a, b]; we record interval_start_hour = a).
  * marker events are "exact"; combat lifecycle is "inferred".
We pick the confidence from the *domain's* source so this auto-sharpens to "exact"
when the live memory layers come online and the writer records memory provenance.
"""
from __future__ import annotations

from typing import Any, Dict, List, Optional, Tuple

try:
    from . import schema
except ImportError:  # pragma: no cover
    import schema  # type: ignore


# Map an event-bearing domain to the snapshot provenance domain that feeds it, so
# we can decide exact-vs-interval per snapshot pair.
_EVENT_DOMAIN = {
    "state_owner_change": "state_ownership",
    "state_controller_change": "state_control",
    "province_controller_change": "province_control",
    "country_created": "country_snapshot",
    "country_removed": "country_snapshot",
    "division_created": "divisions",
    "division_destroyed": "divisions",
    "division_moved": "divisions",
    "division_template_changed": "divisions",
    "focus_completed": "focus",
    "law_changed": "economy",
    "idea_gained": "economy",
    "idea_lost": "economy",
    # ruling_party is sourced from country_snapshot (see _diff_government); the
    # event-domain MUST match the gate + the data source so confidence is computed
    # against the right channel (Codex P1: was country_scalars -> a save-derived
    # govt change could be mislabeled memory-exact when live captured country_scalars).
    "government_changed": "country_snapshot",
    "tech_researched": "tech",
    "tech_lost": "tech",
    "research_slot_gained": "tech",
    "war_declared": "diplomacy",
    "peace": "diplomacy",
    "faction_joined": "diplomacy",
    "faction_left": "diplomacy",
    "subject_created": "diplomacy",
    "subject_freed": "diplomacy",
    "autonomy_changed": "diplomacy",
    "production_milestone": "production",
    "factory_milestone": "economy",
    "commander_assigned": "commanders",
    "army_formed": "order_of_battle",
    "army_disbanded": "order_of_battle",
    "design_created": "designs",
    "mio_leveled": "designs",
    "operation_started": "intel",
    "operation_completed": "intel",
    "lend_lease_started": "market",
    "lend_lease_ended": "market",
    "resistance_changed": "state_detail",
    "compliance_changed": "state_detail",
    "occupation_law_changed": "occupation",
    # Tier-1 EXTRACT save domains
    "event_fired": "world_events",
    "convoy_sunk": "world_events",
    "nuke_count_changed": "country_extra",
    "ace_gained": "country_extra",
    "project_completed": "country_extra",
}


def _domain_is_memory(snap: Dict[str, Any], domain: str) -> bool:
    prov = (snap or {}).get("provenance") or {}
    mem = prov.get("memory") or {}
    return bool(mem.get("captured")) and domain in (mem.get("domains") or [])


def _confidence_for(domain: str, a: Dict[str, Any], b: Dict[str, Any]) -> str:
    """exact if BOTH snapshots had the domain from memory (tick resolution)."""
    if _domain_is_memory(a, domain) and _domain_is_memory(b, domain):
        return "exact"
    return "interval"


def _domain_is_save(snap: Dict[str, Any], domain: str) -> bool:
    prov = (snap or {}).get("provenance") or {}
    sav = prov.get("save") or {}
    return bool(sav.get("captured")) and domain in (sav.get("domains") or [])


def _source_for(domain: str, a: Dict[str, Any], b: Dict[str, Any]) -> str:
    """Honest event source (Codex P2): a memory-exact diff is "M"; a diff that BOTH
    snapshots backed with the SAVE channel is "S" (it really IS save-sourced, not a
    generic derivation); everything else (mixed / pure-synthetic) is "D"."""
    if _confidence_for(domain, a, b) == "exact":
        return "M"
    if _domain_is_save(a, domain) and _domain_is_save(b, domain):
        return "S"
    return "D"


def diff_snapshots(a: Dict[str, Any], b: Dict[str, Any],
                   *, milestone_steps: Optional[Dict[str, float]] = None
                   ) -> List[Dict[str, Any]]:
    """Derive events for the transition a -> b. a may be None (first snapshot).

    With a None, only "appearance" baselines are skipped (we don't fabricate a
    create event for everything that exists at game start); the first real diff
    starts at the second snapshot. Returns events sorted per SCHEMA §9.3.
    """
    if b is None:
        return []
    events: List[Dict[str, Any]] = []
    hi_b = b.get("hour_index")
    date_b = b.get("date")
    hi_a = (a or {}).get("hour_index")

    date_a = (a or {}).get("date")

    def emit(type: str, **kw: Any) -> None:
        domain = _EVENT_DOMAIN.get(type, "")
        kw.setdefault("hour_index", hi_b)
        kw.setdefault("date", date_b)
        kw.setdefault("confidence", _confidence_for(domain, a or {}, b))
        kw.setdefault("source", _source_for(domain, a or {}, b))
        det = kw.get("detail") or {}
        if kw["confidence"] == "interval" and hi_a is not None:
            det = dict(det)
            # explicit interval range so the UI shows a range, not a point at b
            # (Codex P3 #15). interval_start_hour kept for back-compat.
            det.setdefault("interval_start_hour", hi_a)
            det.setdefault("interval", {"start_hour": hi_a, "start_date": date_a,
                                        "end_hour": hi_b, "end_date": date_b})
        if det:
            kw["detail"] = det
        events.append(schema.make_event(type, **kw))

    if a is None:
        # No prior state: emit nothing (baseline). Real timelines start at pair 2.
        return []

    # No-fabrication gate (Codex 2nd-pass P1): only diff a domain when BOTH snapshots
    # actually captured it. Otherwise "uncaptured" would masquerade as "everything
    # changed" (all states null->TAG, all divisions created/destroyed, tech lost, ...).
    if _both_captured(a, b, "state_ownership"):
        _diff_states(a, b, emit, do_owner=True,
                     do_controller=_both_captured(a, b, "state_control"))
    elif _both_captured(a, b, "state_control"):
        _diff_states(a, b, emit, do_owner=False, do_controller=True)
    if _both_captured(a, b, "province_control"):
        _diff_provinces(a, b, emit)
    # Occupation governance is sourced from TWO domains with DISTINCT gates (Codex
    # P1): resistance/compliance come from state_detail; the occupation LAW comes
    # from occupation.json. Gate each on its OWN domain so a pair that captured only
    # state_detail (no occupation) never fabricates a law disappearance, and vice
    # versa. Run the diff if EITHER domain is captured; pass each sub-gate through.
    do_occ_levels = _both_captured(a, b, "state_detail")
    do_occ_law = _both_captured(a, b, "occupation")
    if do_occ_levels or do_occ_law:
        _diff_occupation(a, b, emit, do_levels=do_occ_levels, do_law=do_occ_law)
    if _both_captured(a, b, "divisions"):
        _diff_divisions(a, b, emit)
    if _both_captured(a, b, "order_of_battle"):
        _diff_order_of_battle(a, b, emit)
    if _both_captured(a, b, "designs"):
        _diff_designs(a, b, emit)
    if _both_captured(a, b, "world_events"):
        _diff_world_events(a, b, emit)
    _diff_countries(a, b, emit, milestone_steps or {})

    events.sort(key=_event_sort_key)
    return events


def _provenance_recorded(snap: Dict[str, Any]) -> bool:
    """True if this snapshot states its provenance at all (memory or save block)."""
    prov = (snap or {}).get("provenance") or {}
    return prov.get("memory") is not None or prov.get("save") is not None


def _both_captured(a: Dict[str, Any], b: Dict[str, Any], domain: str) -> bool:
    """Was `domain` captured in BOTH snapshots? When NEITHER snapshot records any
    provenance (pure synthetic data), fall back to True so the diff still runs —
    real ingested snapshots always carry provenance, so the no-fabrication gate
    is fully enforced there."""
    a_rec, b_rec = _provenance_recorded(a), _provenance_recorded(b)
    if not a_rec and not b_rec:
        return True
    return _domain_present(a, domain) and _domain_present(b, domain)


# ---------------------------------------------------------------------------
# State owner / controller
# ---------------------------------------------------------------------------
def _diff_states(a: Dict[str, Any], b: Dict[str, Any], emit,
                 *, do_owner: bool = True, do_controller: bool = True) -> None:
    sa, sb = a.get("states") or {}, b.get("states") or {}
    # Union of keys so a state present in only one snapshot still diffs (Codex P1 #2).
    for sid in set(sa) | set(sb):
        old = sa.get(sid) or {}
        new = sb.get(sid) or {}
        if do_owner:
            o_owner, n_owner = old.get("owner"), new.get("owner")
            if o_owner != n_owner and (o_owner is not None or n_owner is not None):
                # subject = new owner, or (on TAG->null) the losing tag (Codex P2 #9)
                emit("state_owner_change", key=sid, **{"from": o_owner}, to=n_owner,
                     subject=(n_owner or o_owner), object=o_owner)
        if do_controller:
            o_ctl, n_ctl = old.get("controller"), new.get("controller")
            if o_ctl != n_ctl and (o_ctl is not None or n_ctl is not None):
                emit("state_controller_change", key=sid, **{"from": o_ctl}, to=n_ctl,
                     subject=(n_ctl or o_ctl), object=o_ctl)


def _diff_provinces(a: Dict[str, Any], b: Dict[str, Any], emit) -> None:
    pa, pb = a.get("provinces") or {}, b.get("provinces") or {}
    for pid in set(pa) | set(pb):
        old = (pa.get(pid) or {}).get("controller")
        new = (pb.get(pid) or {}).get("controller")
        if old != new and (old is not None or new is not None):
            emit("province_controller_change", key=pid, **{"from": old}, to=new,
                 subject=(new or old), object=old)


# ---------------------------------------------------------------------------
# Divisions: created / destroyed / moved / template-changed (keyed by div_id)
# ---------------------------------------------------------------------------
def _div_index(snap: Dict[str, Any]) -> Dict[str, Dict[str, Any]]:
    # Key by full Clausewitz identity (id:type) so id reuse after destruction is a
    # destroy+create, not a spurious move+retemplate (Codex P1 #3).
    out = {}
    for d in snap.get("divisions") or []:
        uid = schema.div_uid(d)
        if uid is not None:
            out[uid] = d
    return out


def _reconcile_uids(da: Dict[str, Dict[str, Any]], db: Dict[str, Dict[str, Any]]
                    ) -> List[Tuple[str, str]]:
    """Pair up uids that don't match exactly but are the SAME division seen through
    channels that disagree on div_type presence (one has 'id:type', the other bare
    'id'). Only reconcile when the bare id is unambiguous on BOTH sides (exactly one
    unmatched uid with that bare id), so we never merge two distinct (id,type) divs.

    RESIDUAL (unavoidable without div_type on both sides): a bare-id-only channel
    cannot distinguish "same division" from "id reused after destruction". When the
    bare id is unique on both sides we PREFER pairing (treat as the same div → a
    move/retemplate) over emitting a false destroy+create on a mere channel mismatch.
    This only bites when one channel lacks div_type (today: a future memory-only
    feed); the all-save path always has div_type, so it never triggers there.
    Returns [(uid_a, uid_b)] extra pairings."""
    unmatched_a = [u for u in da if u not in db]
    unmatched_b = [u for u in db if u not in da]
    if not unmatched_a or not unmatched_b:
        return []

    def bare(u: str) -> str:
        return u.split(":", 1)[0]

    from collections import Counter
    a_by_bare = Counter(bare(u) for u in unmatched_a)
    b_by_bare = Counter(bare(u) for u in unmatched_b)
    a_lookup = {bare(u): u for u in unmatched_a}
    b_lookup = {bare(u): u for u in unmatched_b}
    pairs = []
    for bid in set(a_by_bare) & set(b_by_bare):
        if a_by_bare[bid] == 1 and b_by_bare[bid] == 1:
            ua, ub = a_lookup[bid], b_lookup[bid]
            # only reconcile if at least one side actually lacked the type half
            if (":" not in ua) or (":" not in ub):
                pairs.append((ua, ub))
    return pairs


def _diff_divisions(a: Dict[str, Any], b: Dict[str, Any], emit) -> None:
    da, db = _div_index(a), _div_index(b)
    # reconcile typed-vs-bare uid mismatches into a shared key map
    extra_pairs = _reconcile_uids(da, db)
    # remap b's reconciled uids onto a's key so the matched loop sees them as same
    remap_b = {ub: ua for ua, ub in extra_pairs}
    matched_a = {ua for ua, _ in extra_pairs}
    matched_b = {ub for _, ub in extra_pairs}

    for uid in db:
        if uid not in da and uid not in matched_b:
            d = db[uid]
            emit("division_created", key=uid, to=d.get("owner"),
                 subject=d.get("owner"),
                 detail={"template_id": d.get("template_id"),
                         "template_name": d.get("template_name"),
                         "location_province": d.get("location_province")})
    for uid in da:
        if uid not in db and uid not in matched_a:
            d = da[uid]
            emit("division_destroyed", key=uid, **{"from": d.get("owner")},
                 subject=d.get("owner"),
                 detail={"template_id": d.get("template_id"),
                         "last_location": d.get("location_province")})
    # exact-uid matches plus reconciled pairs
    paired = [(uid, uid) for uid in db if uid in da]
    paired += [(ua, ub) for ua, ub in extra_pairs]
    for uid_a, uid_b in paired:
        old, new = da[uid_a], db[uid_b]
        uid = uid_a
        o_loc, n_loc = old.get("location_province"), new.get("location_province")
        if o_loc != n_loc and (o_loc is not None or n_loc is not None):
            emit("division_moved", key=uid, **{"from": o_loc}, to=n_loc,
                 subject=new.get("owner"),
                 detail={"from_state": old.get("state_id"),
                         "to_state": new.get("state_id")})
        # template identity is also (id, type) (Codex 2nd-pass P2).
        o_tpl = (old.get("template_id"), old.get("template_type"))
        n_tpl = (new.get("template_id"), new.get("template_type"))
        if o_tpl != n_tpl and (o_tpl[0] is not None or n_tpl[0] is not None):
            emit("division_template_changed", key=uid,
                 **{"from": old.get("template_id")}, to=new.get("template_id"),
                 subject=new.get("owner"),
                 detail={"from_name": old.get("template_name"),
                         "to_name": new.get("template_name"),
                         "from_type": old.get("template_type"),
                         "to_type": new.get("template_type")})


# ---------------------------------------------------------------------------
# Country-scoped: focus, laws, ideas, government, tech, slots, diplomacy, prod
# ---------------------------------------------------------------------------
def _diff_countries(a: Dict[str, Any], b: Dict[str, Any], emit,
                    milestone_steps: Dict[str, float]) -> None:
    ca, cb = a.get("countries") or {}, b.get("countries") or {}
    # Lifecycle: a country present in only one snapshot appeared / disappeared.
    # Gate STRICTLY on provenance (Codex 2nd-pass P1): partial country maps built
    # from diplomacy/divisions must NOT trigger false created/removed. Only the
    # country_snapshot domain authoritatively lists every living country.
    if _both_captured(a, b, "country_snapshot"):
        for tag in set(cb) - set(ca):
            emit("country_created", subject=tag, key=tag, to=tag)
        for tag in set(ca) - set(cb):
            emit("country_removed", subject=tag, key=tag, **{"from": tag})
    # Per-domain sub-diffs, each gated on both snapshots capturing that domain.
    # Laws/ideas are sourced from economy.json; government (ruling_party) from
    # country_snapshot — gate them SEPARATELY so a snapshot pair that captured one
    # but not the other can't fabricate events from empty defaults (Codex 3rd-pass).
    do_focus = _both_captured(a, b, "focus")
    do_laws_ideas = _both_captured(a, b, "economy")
    do_government = _both_captured(a, b, "country_snapshot")
    do_tech = _both_captured(a, b, "tech")
    do_dip = _both_captured(a, b, "diplomacy")
    do_prod = _both_captured(a, b, "production") or _both_captured(a, b, "economy")
    do_commanders = _both_captured(a, b, "commanders")
    do_intel = _both_captured(a, b, "intel")
    do_mios = _both_captured(a, b, "designs")
    do_market = _both_captured(a, b, "market")
    do_country_extra = _both_captured(a, b, "country_extra")
    for tag in set(ca) | set(cb):
        old = ca.get(tag) or {}
        new = cb.get(tag) or {}
        if do_focus:
            _diff_focus(tag, old, new, emit)
        if do_laws_ideas:
            _diff_laws_ideas(tag, old, new, emit)
        if do_government:
            _diff_government(tag, old, new, emit)
        if do_tech:
            _diff_research(tag, old, new, emit)
        if do_dip:
            _diff_diplomacy(tag, old, new, emit)
            _diff_faction_autonomy(tag, old, new, emit)
        if do_prod:
            _diff_production(tag, old, new, emit, milestone_steps)
        if do_commanders:
            _diff_commanders(tag, old, new, emit)
        if do_intel:
            _diff_intel(tag, old, new, emit)
        if do_mios:
            _diff_mios(tag, old, new, emit)
        if do_market:
            _diff_market(tag, old, new, emit)
        if do_country_extra:
            _diff_country_extra(tag, old, new, emit)


def _domain_present(snap: Dict[str, Any], domain: str) -> bool:
    prov = (snap or {}).get("provenance") or {}
    for ch in ("memory", "save"):
        block = prov.get(ch) or {}
        if block.get("captured") and domain in (block.get("domains") or []):
            return True
    return False


def _diff_focus(tag, old, new, emit) -> None:
    o_done = set((old.get("focus") or {}).get("completed") or [])
    n_done = set((new.get("focus") or {}).get("completed") or [])
    for f in sorted(n_done - o_done):
        emit("focus_completed", subject=tag, key=f, to=f)


def _diff_laws_ideas(tag, old, new, emit) -> None:
    """Laws + national ideas/spirits (sourced from economy.json)."""
    o_pol, n_pol = old.get("politics") or {}, new.get("politics") or {}
    o_laws, n_laws = (o_pol.get("laws") or {}), (n_pol.get("laws") or {})
    for cat in set(o_laws) | set(n_laws):
        ov, nv = o_laws.get(cat), n_laws.get(cat)
        if ov != nv and (ov is not None or nv is not None):
            emit("law_changed", subject=tag, key=cat, **{"from": ov}, to=nv)
    o_ideas = set(o_pol.get("ideas_all") or [])
    n_ideas = set(n_pol.get("ideas_all") or [])
    for idea in sorted(n_ideas - o_ideas):
        emit("idea_gained", subject=tag, key=idea, to=idea)
    for idea in sorted(o_ideas - n_ideas):
        emit("idea_lost", subject=tag, key=idea, **{"from": idea})


def _diff_government(tag, old, new, emit) -> None:
    """Ruling party (sourced from country_snapshot)."""
    o_pol, n_pol = old.get("politics") or {}, new.get("politics") or {}
    ov, nv = o_pol.get("ruling_party"), n_pol.get("ruling_party")
    if ov != nv and (ov is not None or nv is not None):
        emit("government_changed", subject=tag, **{"from": ov}, to=nv)


def _diff_faction_autonomy(tag, old, new, emit) -> None:
    """Faction membership + subject/autonomy. Gated on the diplomacy domain (these
    fields are sourced from diplomacy.json), so split out of _diff_politics."""
    o_pol, n_pol = old.get("politics") or {}, new.get("politics") or {}
    # faction membership
    o_fac = (o_pol.get("faction") or {}).get("name") if o_pol.get("faction") else None
    n_fac = (n_pol.get("faction") or {}).get("name") if n_pol.get("faction") else None
    if o_fac != n_fac:
        if n_fac and not o_fac:
            emit("faction_joined", subject=tag, object=n_fac, to=n_fac)
        elif o_fac and not n_fac:
            emit("faction_left", subject=tag, object=o_fac, **{"from": o_fac})
        else:  # switched factions
            emit("faction_left", subject=tag, object=o_fac, **{"from": o_fac})
            emit("faction_joined", subject=tag, object=n_fac, to=n_fac)
    # subject / autonomy (politics.autonomy = {level, overlord})
    o_aut = o_pol.get("autonomy") or {}
    n_aut = n_pol.get("autonomy") or {}
    o_over, n_over = o_aut.get("overlord"), n_aut.get("overlord")
    if not o_over and n_over:
        emit("subject_created", subject=tag, object=n_over, to=n_aut.get("level"),
             detail={"overlord": n_over})
    elif o_over and not n_over:
        emit("subject_freed", subject=tag, object=o_over, **{"from": o_aut.get("level")})
    elif o_over and n_over:
        if o_over != n_over:
            # overlord switched (e.g. A -> B): freed from A, then created under B
            # (Codex 2nd-pass P2 — was missed when only the level was compared).
            emit("subject_freed", subject=tag, object=o_over,
                 **{"from": o_aut.get("level")})
            emit("subject_created", subject=tag, object=n_over, to=n_aut.get("level"),
                 detail={"overlord": n_over})
        elif o_aut.get("level") != n_aut.get("level"):
            emit("autonomy_changed", subject=tag, object=n_over,
                 **{"from": o_aut.get("level")}, to=n_aut.get("level"))


def _diff_research(tag, old, new, emit) -> None:
    o_res, n_res = old.get("research") or {}, new.get("research") or {}
    o_tech = {t.get("name"): t.get("level") for t in (o_res.get("technologies") or [])
              if isinstance(t, dict)}
    n_tech = {t.get("name"): t for t in (n_res.get("technologies") or [])
              if isinstance(t, dict)}
    for name, t in n_tech.items():
        lvl = t.get("level")
        if name not in o_tech:
            emit("tech_researched", subject=tag, key=name, to=lvl,
                 detail={"date": t.get("date")})
        elif lvl is not None and o_tech.get(name) is not None:
            # only an INCREASE is "researched"; a decrease is a rollback (Codex P1 #4)
            if lvl > o_tech[name]:
                emit("tech_researched", subject=tag, key=name,
                     **{"from": o_tech[name]}, to=lvl, detail={"date": t.get("date")})
            elif lvl < o_tech[name]:
                emit("tech_lost", subject=tag, key=name,
                     **{"from": o_tech[name]}, to=lvl,
                     detail={"note": "level decreased (save rollback / civil war?)"})
    # tech that disappeared entirely = tech_lost
    for name in set(o_tech) - set(n_tech):
        emit("tech_lost", subject=tag, key=name, **{"from": o_tech[name]}, to=None)
    o_slots, n_slots = o_res.get("research_slots"), n_res.get("research_slots")
    if isinstance(o_slots, int) and isinstance(n_slots, int) and n_slots > o_slots:
        emit("research_slot_gained", subject=tag, **{"from": o_slots}, to=n_slots)


def _diff_diplomacy(tag, old, new, emit) -> None:
    o_dip, n_dip = old.get("diplomacy") or {}, new.get("diplomacy") or {}
    o_wars = _war_targets(o_dip.get("wars"))
    n_wars = _war_targets(n_dip.get("wars"))
    for enemy in sorted(n_wars - o_wars):
        emit("war_declared", subject=tag, object=enemy, to=enemy)
    for enemy in sorted(o_wars - n_wars):
        emit("peace", subject=tag, object=enemy, **{"from": enemy})


def _war_targets(wars: Any) -> set:
    """Extract opponent TAGs from a wars[] list (tolerant of shapes)."""
    out = set()
    if not isinstance(wars, list):
        return out
    for w in wars:
        if isinstance(w, dict):
            ag = w.get("against")
            if isinstance(ag, str):
                out.add(ag)
            elif isinstance(ag, list):
                out.update(x for x in ag if isinstance(x, str))
        elif isinstance(w, str):
            out.add(w)
    return out


def _diff_production(tag, old, new, emit, milestone_steps: Dict[str, float]) -> None:
    # factory milestones (civ/mil/dockyard cross an integer threshold step)
    o_f = ((old.get("economy") or {}).get("factories") or {})
    n_f = ((new.get("economy") or {}).get("factories") or {})
    for kind in ("civ", "mil", "dockyard"):
        ov, nv = o_f.get(kind), n_f.get(kind)
        step = milestone_steps.get("factory_" + kind) or milestone_steps.get("factory")
        if step and isinstance(ov, int) and isinstance(nv, int) and nv > ov:
            if (nv // step) > (ov // step):
                emit("factory_milestone", subject=tag, key=kind,
                     **{"from": ov}, to=nv,
                     detail={"step": step, "crossed": (nv // step) * step})
    # production milestones (gross produced of an archetype crossing a step)
    step = milestone_steps.get("production")
    if step:
        o_lines = (old.get("economy") or {}).get("_production_lines")
        o_prod, o_unmeasured = _gross_by_archetype(o_lines, with_unmeasured=True)
        n_prod = _gross_by_archetype(
            (new.get("economy") or {}).get("_production_lines"))
        for arch, nv in n_prod.items():
            # Old baseline (Codex P2, refined): if the OLD archetype had a line whose
            # `produced` was UNMEASURED (None), we cannot baseline it at 0.0 -> skip
            # (no fabricated milestone). But an archetype GENUINELY ABSENT from a
            # measured old production set is a real 0 baseline (a new line that has
            # since produced N) -> eligible. So: skip only the unmeasured case.
            if arch in o_unmeasured and arch not in o_prod:
                continue  # old line present but produced unknown -> not a real 0
            ov = o_prod.get(arch, 0.0)
            if nv > ov and int(nv // step) > int(ov // step):
                emit("production_milestone", subject=tag, key=arch,
                     **{"from": ov}, to=nv,
                     detail={"step": step, "crossed": int(nv // step) * step})


def _gross_by_archetype(lines: Any, *, with_unmeasured: bool = False):
    """Sum measured `produced` per archetype. A line with `produced=None` is
    UNMEASURED, not zero (Codex P2): it is excluded from the gross sum so a later
    measured value does not diff against a fabricated 0.0. With ``with_unmeasured``,
    also return the SET of archetypes that had at least one unmeasured line, so the
    caller can distinguish "old line present but produced unknown" (skip) from
    "archetype genuinely absent from a measured set" (a real 0 baseline)."""
    out: Dict[str, float] = {}
    unmeasured = set()
    if isinstance(lines, list):
        for ln in lines:
            if not (isinstance(ln, dict) and ln.get("archetype") is not None):
                continue
            arch = ln["archetype"]
            produced = ln.get("produced")
            if produced is None:
                unmeasured.add(arch)
                continue  # unmeasured -> do not coerce to 0.0
            out[arch] = out.get(arch, 0.0) + produced
    if with_unmeasured:
        return out, unmeasured
    return out


# ---------------------------------------------------------------------------
# Commanders (characters.json): commander -> formation assignment changes
# ---------------------------------------------------------------------------
def _commander_index(country: Dict[str, Any]) -> Dict[str, Dict[str, Any]]:
    """Index a country's military leaders by a stable per-tag identity.

    Identity = the character `token` (script id, stable across snapshots) falling
    back to `unit_leader_id`. Generals + field marshals + admirals share one space.
    """
    cmd = country.get("commanders") or {}
    out: Dict[str, Dict[str, Any]] = {}
    for role_list in ("generals", "field_marshals", "admirals"):
        for g in (cmd.get(role_list) or []):
            if not isinstance(g, dict):
                continue
            ident = g.get("token") or g.get("unit_leader_id")
            if ident is not None:
                out[str(ident)] = g
    return out


def _assignment_name(g: Dict[str, Any]) -> Optional[str]:
    a = g.get("assignment")
    return a.get("name") if isinstance(a, dict) else None


def _diff_commanders(tag, old, new, emit) -> None:
    """commander_assigned: a leader moves to / between formations.

    Emitted only for an assignment that gains a formation or switches formation —
    a leader going to reserve (formation -> null) is not an "assignment" and is
    skipped (avoids noise on every dissolved army). Keyed by the commander ident.
    """
    oi, ni = _commander_index(old), _commander_index(new)
    for ident in set(oi) & set(ni):
        o_name = _assignment_name(oi[ident])
        n_name = _assignment_name(ni[ident])
        if n_name and n_name != o_name:
            g = ni[ident]
            a = g.get("assignment") or {}
            emit("commander_assigned", subject=tag, key=str(ident),
                 **{"from": o_name}, to=n_name,
                 detail={"commander": g.get("name"), "role": g.get("role"),
                         "kind": a.get("kind")})


# ---------------------------------------------------------------------------
# Intel (intel.json): operations started / completed
# ---------------------------------------------------------------------------
def _running_ops(country: Dict[str, Any]) -> Dict[str, Dict[str, Any]]:
    ops = ((country.get("intel") or {}).get("operations") or {})
    out: Dict[str, Dict[str, Any]] = {}
    for op in (ops.get("running") or []):
        if isinstance(op, dict) and op.get("op_id") is not None:
            out[str(op["op_id"])] = op
    return out


def _diff_intel(tag, old, new, emit) -> None:
    """operation_started / operation_completed by tracking running-operation ids.

    A running-op id present only in `new` -> started; present only in `old` ->
    completed/ended (it left the running set — finished, aborted, or expired).
    """
    oo, no = _running_ops(old), _running_ops(new)
    for oid in set(no) - set(oo):
        op = no[oid]
        emit("operation_started", subject=tag, object=op.get("target"), key=oid,
             to=op.get("type"),
             detail={"target": op.get("target"), "date": op.get("date"),
                     "duration": op.get("duration")})
    for oid in set(oo) - set(no):
        op = oo[oid]
        emit("operation_completed", subject=tag, object=op.get("target"), key=oid,
             **{"from": op.get("type")},
             detail={"target": op.get("target")})


# ---------------------------------------------------------------------------
# MIOs (designs.json per-country): org level increases
# ---------------------------------------------------------------------------
def _mio_index(country: Dict[str, Any]) -> Dict[str, Dict[str, Any]]:
    out: Dict[str, Dict[str, Any]] = {}
    for m in (country.get("mios") or []):
        if isinstance(m, dict) and m.get("key"):
            out[str(m["key"])] = m
    return out


def _diff_mios(tag, old, new, emit) -> None:
    """mio_leveled: a military-industrial org's level (`level`) increased."""
    oi, ni = _mio_index(old), _mio_index(new)
    for key in set(oi) & set(ni):
        ov, nv = oi[key].get("level"), ni[key].get("level")
        if isinstance(ov, int) and isinstance(nv, int) and nv > ov:
            emit("mio_leveled", subject=tag, key=key, **{"from": ov}, to=nv,
                 detail={"name": ni[key].get("name")})


# ---------------------------------------------------------------------------
# Market / lend-lease (market.json): trade contracts started / ended (per-country)
# ---------------------------------------------------------------------------
def _ll_index(country: Dict[str, Any]) -> Dict[str, Dict[str, Any]]:
    """Index a country's lend-lease/market contracts by a stable per-side key.

    A directed contract appears on both parties, so the key combines the contract
    id with this side's direction ("465:81|outgoing") — so an outgoing and an
    incoming contract with the same id on the same tag (a self-trade edge case) do
    not collide, and the SAME contract on the other party indexes under that party.
    """
    out: Dict[str, Dict[str, Any]] = {}
    for ll in ((country.get("diplomacy") or {}).get("lend_lease") or []):
        if not isinstance(ll, dict):
            continue
        cid = ll.get("contract_id")
        if cid is None:
            continue
        out["%s|%s" % (cid, ll.get("direction"))] = ll
    return out


def _diff_market(tag, old, new, emit) -> None:
    """lend_lease_started / lend_lease_ended: an equipment-market (lend-lease)
    trade contract appears / disappears for this country.

    Keyed by the contract id + direction. ``object`` = the counterpart TAG, ``to`` =
    the lead equipment archetype being transferred (informational). Started = a
    contract present only in ``new``; ended = present only in ``old``.
    """
    oi, ni = _ll_index(old), _ll_index(new)
    for key in set(ni) - set(oi):
        ll = ni[key]
        emit("lend_lease_started", subject=tag, object=ll.get("counterpart"),
             key=str(ll.get("contract_id")), to=_lead_archetype(ll),
             detail={"direction": ll.get("direction"),
                     "counterpart": ll.get("counterpart"),
                     "seller": ll.get("seller"), "buyer": ll.get("buyer")})
    for key in set(oi) - set(ni):
        ll = oi[key]
        emit("lend_lease_ended", subject=tag, object=ll.get("counterpart"),
             key=str(ll.get("contract_id")), **{"from": _lead_archetype(ll)},
             detail={"direction": ll.get("direction"),
                     "counterpart": ll.get("counterpart")})


def _lead_archetype(ll: Dict[str, Any]) -> Optional[str]:
    eqs = ll.get("equipments")
    if isinstance(eqs, list) and eqs and isinstance(eqs[0], dict):
        return eqs[0].get("archetype")
    return None


# ---------------------------------------------------------------------------
# Occupation governance (state_detail + occupation.json): resistance / compliance /
# occupation-law shifts on occupied states (world-scoped, keyed by state id)
# ---------------------------------------------------------------------------
# Only emit when a resistance/compliance level crosses a meaningful step, so a
# snapshot pair doesn't fire an event on every sub-percent drift (these tick every
# day). The bands match the in-game resistance/compliance milestone thresholds.
_OCC_BANDS = (0.0, 15.0, 25.0, 40.0, 60.0, 80.0, 100.0)


def _band(v: Any) -> Optional[int]:
    """Return the index of the highest band threshold v has reached, or None."""
    if not isinstance(v, (int, float)):
        return None
    idx = None
    for i, thr in enumerate(_OCC_BANDS):
        if v >= thr:
            idx = i
        else:
            break
    return idx


def _diff_occupation(a: Dict[str, Any], b: Dict[str, Any], emit,
                     *, do_levels: bool = True, do_law: bool = True) -> None:
    sa, sb = a.get("states") or {}, b.get("states") or {}
    for sid in set(sa) | set(sb):
        old = sa.get(sid) or {}
        new = sb.get(sid) or {}
        o_occ = old.get("occupation") if isinstance(old.get("occupation"), dict) else {}
        n_occ = new.get("occupation") if isinstance(new.get("occupation"), dict) else {}
        # resistance / compliance (state_detail-gated): emit only on a band crossing.
        if do_levels:
            for field, etype in (("resistance", "resistance_changed"),
                                 ("compliance", "compliance_changed")):
                ov, nv = o_occ.get(field), n_occ.get(field)
                ob, nb = _band(ov), _band(nv)
                if ob is not None and nb is not None and ob != nb:
                    emit(etype, key=sid,
                         subject=(n_occ.get("occupier_tag")
                                  or o_occ.get("occupier_tag")),
                         object=(n_occ.get("occupied_country_tag")
                                 or o_occ.get("occupied_country_tag")),
                         **{"from": ov}, to=nv,
                         detail={"from_band": ob, "to_band": nb})
        # occupation law (occupation-domain-gated ONLY — Codex P1): a state_detail-
        # only pair must NOT diff the law (it is unmeasured there, so a measured-vs-
        # null comparison would fabricate a law disappearance).
        if do_law:
            o_law, n_law = old.get("occupation_law"), new.get("occupation_law")
            if o_law != n_law and (o_law is not None or n_law is not None):
                emit("occupation_law_changed", key=sid,
                     subject=(n_occ.get("occupier_tag") or o_occ.get("occupier_tag")),
                     object=(n_occ.get("occupied_country_tag")
                             or o_occ.get("occupied_country_tag")),
                     **{"from": o_law}, to=n_law)


# ---------------------------------------------------------------------------
# Order of battle (oob.json): armies formed / disbanded (world-scoped)
# ---------------------------------------------------------------------------
def _army_index(snap: Dict[str, Any]) -> Dict[str, Dict[str, Any]]:
    out: Dict[str, Dict[str, Any]] = {}
    for army in ((snap.get("order_of_battle") or {}).get("armies") or []):
        if isinstance(army, dict) and army.get("id"):
            out[str(army["id"])] = army
    return out


def _diff_order_of_battle(a: Dict[str, Any], b: Dict[str, Any], emit) -> None:
    """army_formed / army_disbanded keyed by the army's (id:type) handle.

    An army id (Clausewitz "id:type") present only in b -> formed; only in a ->
    disbanded. Subject = the owning TAG.
    """
    aa, ab = _army_index(a), _army_index(b)
    for aid in set(ab) - set(aa):
        army = ab[aid]
        emit("army_formed", subject=army.get("owner"), key=aid, to=army.get("name"),
             detail={"name": army.get("name"), "members": army.get("member_count")})
    for aid in set(aa) - set(ab):
        army = aa[aid]
        emit("army_disbanded", subject=army.get("owner"), key=aid,
             **{"from": army.get("name")}, detail={"name": army.get("name")})


# ---------------------------------------------------------------------------
# Designs (designs.json): new equipment-variant designs created
# ---------------------------------------------------------------------------
def _diff_designs(a: Dict[str, Any], b: Dict[str, Any], emit) -> None:
    """design_created: a new equipment variant appears in the catalog.

    Keyed by the catalog key ("id:type"). Subject = the variant's `creator` TAG.
    Only newly-appearing keys are emitted (a removed design is not a player-facing
    event — obsolete designs persist in the catalog, so removals are noise).
    """
    va = ((a.get("designs") or {}).get("equipment_variants") or {})
    vb = ((b.get("designs") or {}).get("equipment_variants") or {})
    if not isinstance(va, dict) or not isinstance(vb, dict):
        return
    for key in set(vb) - set(va):
        v = vb[key]
        if not isinstance(v, dict):
            continue
        # Only count player-authored designs (a custom name OR modules), so the
        # game-start baseline of stock archetypes doesn't fire on first appearance.
        if not (v.get("name") or v.get("modules")):
            continue
        emit("design_created", subject=v.get("creator"), key=key,
             to=v.get("name") or v.get("archetype"),
             detail={"archetype": v.get("archetype"), "family": v.get("family"),
                     "name": v.get("name")})


# ---------------------------------------------------------------------------
# World events (events.json): scripted events newly fired + convoys newly sunk
# (world-scoped, from snap["world_events"]).
# ---------------------------------------------------------------------------
def _diff_world_events(a: Dict[str, Any], b: Dict[str, Any], emit) -> None:
    """event_fired: a scripted event id newly appears in the persisted fired list.
    convoy_sunk: a per-month sunk-convoy ledger ENTRY newly appears.

    fired_events is an append-mostly persisted list; we set-diff it so only genuinely
    new ids fire. sunk_convoys entries are keyed by (month, killer, owner) — a new key,
    or a higher convoy count for an existing key, emits (the per-month tally can grow
    within the same month between two saves)."""
    wea = a.get("world_events") or {}
    web = b.get("world_events") or {}

    o_fired = set(wea.get("fired_events") or [])
    n_fired = set(web.get("fired_events") or [])
    for eid in sorted(n_fired - o_fired):
        emit("event_fired", key=eid, to=eid)

    def _conv_index(we):
        out = {}
        for e in (we.get("sunk_convoys") or []):
            if not isinstance(e, dict):
                continue
            k = (e.get("month"), e.get("killer_country"), e.get("owner"))
            out[k] = e
        return out

    oc, nc = _conv_index(wea), _conv_index(web)
    for k, e in nc.items():
        ov = (oc.get(k) or {}).get("convoys")
        nv = e.get("convoys")
        # new ledger entry, OR an existing month-killer-owner tally that grew.
        is_new = k not in oc
        grew = (isinstance(ov, (int, float)) and isinstance(nv, (int, float))
                and nv > ov)
        if is_new or grew:
            emit("convoy_sunk", subject=e.get("killer_country"),
                 object=e.get("owner"),
                 key="%s:%s:%s" % (e.get("month"), e.get("killer_country"),
                                   e.get("owner")),
                 **{"from": ov if grew else None}, to=nv,
                 detail={"month": e.get("month"), "owner": e.get("owner"),
                         "killer": e.get("killer_country")})


# ---------------------------------------------------------------------------
# Country extra (country_extra.json): nuke count, new aces, projects completed
# (per-country, from countries.*.extra).
# ---------------------------------------------------------------------------
def _ace_key(ac: Dict[str, Any]) -> str:
    """Stable per-ace identity = name+surname+callsign (the save has no ace id)."""
    return "%s|%s|%s" % (ac.get("name"), ac.get("surname"), ac.get("callsign"))


def _diff_country_extra(tag, old, new, emit) -> None:
    """nuke_count_changed: countries.*.extra.nukes_total changed.
    ace_gained: a new ace (by name/surname/callsign) appears.
    project_completed: special_projects.projects_completed increased (by the delta)."""
    o_ext = old.get("extra") if isinstance(old.get("extra"), dict) else {}
    n_ext = new.get("extra") if isinstance(new.get("extra"), dict) else {}
    if not o_ext and not n_ext:
        return

    # nuke total (a real 0 is a measured value — only emit on an actual change)
    o_n, n_n = o_ext.get("nukes_total"), n_ext.get("nukes_total")
    if (isinstance(o_n, int) and isinstance(n_n, int) and o_n != n_n):
        emit("nuke_count_changed", subject=tag, **{"from": o_n}, to=n_n)

    # newly-gained aces (set-diff by ace identity)
    o_aces = {_ace_key(x) for x in (o_ext.get("aces") or []) if isinstance(x, dict)}
    for ac in (n_ext.get("aces") or []):
        if not isinstance(ac, dict):
            continue
        if _ace_key(ac) not in o_aces:
            emit("ace_gained", subject=tag, key=_ace_key(ac),
                 to=ac.get("callsign") or ac.get("surname"),
                 detail={"name": ac.get("name"), "surname": ac.get("surname"),
                         "callsign": ac.get("callsign"),
                         "modifier": ac.get("modifier")})

    # special-project completions (count INCREASE only)
    o_done = (o_ext.get("special_projects") or {}).get("projects_completed")
    n_done = (n_ext.get("special_projects") or {}).get("projects_completed")
    if (isinstance(o_done, int) and isinstance(n_done, int) and n_done > o_done):
        emit("project_completed", subject=tag, **{"from": o_done}, to=n_done,
             detail={"completed_delta": n_done - o_done})


# ---------------------------------------------------------------------------
# Marker events + merge + dedupe (SCHEMA §9.3)
# ---------------------------------------------------------------------------
def _event_sort_key(ev: Dict[str, Any]) -> Tuple:
    return (ev.get("hour_index") if ev.get("hour_index") is not None else 1 << 62,
            ev.get("type") or "", str(ev.get("key") or ""))


def _dedupe_key(ev: Dict[str, Any]) -> Tuple:
    # Include 'from' so an A->B flip and a B->A flip are NOT collapsed (P2 #8), and
    # 'object' so e.g. two subject_created with different overlords don't collide
    # (Codex 2nd-pass P2).
    return (ev.get("type"), str(ev.get("key")), str(ev.get("from")),
            str(ev.get("to")), str(ev.get("subject")), str(ev.get("object")))


def build_timeline(snapshots: List[Dict[str, Any]],
                   *, milestone_steps: Optional[Dict[str, float]] = None
                   ) -> List[Dict[str, Any]]:
    """Merge a whole ordered snapshot series into one chronological event feed.

    1. collect per-snapshot marker events (snap['events']),
    2. diff each adjacent pair,
    3. merge, sort, and apply marker-supersedes-diff dedupe.
    """
    snaps = sorted(
        [s for s in snapshots if isinstance(s, dict)],
        key=lambda s: (s.get("hour_index") is None, s.get("hour_index") or 0),
    )
    # Merge snapshots sharing the same hour_index (e.g. a memory snapshot + a save
    # snapshot at the same tick carrying complementary domains) so we don't diff
    # two partial views of one instant into fabricated transitions (Codex P3 #12).
    snaps = _coalesce_same_hour(snaps)
    all_events: List[Dict[str, Any]] = []

    # markers carried inside snapshots
    for s in snaps:
        for ev in (s.get("events") or []):
            m = dict(ev)
            m.setdefault("hour_index", s.get("hour_index"))
            m.setdefault("date", s.get("date"))
            m.setdefault("source", "marker")
            m.setdefault("confidence", "exact")
            all_events.append(m)

    # diff-derived
    prev = None
    for s in snaps:
        if prev is not None:
            all_events.extend(diff_snapshots(prev, s, milestone_steps=milestone_steps))
        prev = s

    return _merge_dedupe(all_events)


def _merge_dedupe(events: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
    """Sort, then drop a diff-derived event when an exact marker covers the same
    (type,key,from,to,subject). The dedupe window is INTERVAL-AWARE (Codex P2 #8):

      * for an interval diff, a marker supersedes it if the marker hour lies inside
        the diff's interval (interval_start_hour, hour_index] — exactly the window
        the change is known to have occurred in (handles monthly saves where the
        marker can be >24h before the later bound);
      * for an exact (memory) diff, only a marker at the same tick supersedes it
        (no fuzzy window — hourly memory must not eat a real repeated flip).
    """
    events = sorted(events, key=_event_sort_key)
    marker_hours: Dict[Tuple, List[int]] = {}
    for ev in events:
        if ev.get("source") == "marker" and ev.get("confidence") == "exact":
            hi = ev.get("hour_index")
            if hi is not None:
                marker_hours.setdefault(_dedupe_key(ev), []).append(hi)

    kept: List[Dict[str, Any]] = []
    for ev in events:
        if ev.get("source") == "marker":
            kept.append(ev)
            continue
        k = _dedupe_key(ev)
        hi = ev.get("hour_index")
        superseded = False
        if hi is not None and k in marker_hours:
            if ev.get("confidence") == "interval":
                det = ev.get("detail") or {}
                start = det.get("interval_start_hour")
                if not isinstance(start, int):
                    start = (det.get("interval") or {}).get("start_hour")
                lo = start if isinstance(start, int) else hi - 1
                superseded = any(lo < mh <= hi for mh in marker_hours[k])
            else:  # exact: same-tick only
                superseded = any(mh == hi for mh in marker_hours[k])
        if not superseded:
            kept.append(ev)
    return kept


def _coalesce_same_hour(snaps: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
    """Deep-merge consecutive snapshots that share a (non-None) hour_index.

    Later-listed snapshot wins on scalar conflicts; container domains are merged
    shallowly per top-level key. Marker events from both are concatenated. This
    yields one canonical snapshot per instant so adjacent-pair diffing is clean.
    """
    out: List[Dict[str, Any]] = []
    for s in snaps:
        hi = s.get("hour_index")
        if out and hi is not None and out[-1].get("hour_index") == hi:
            out[-1] = _merge_two_snapshots(out[-1], s)
        else:
            out.append(s)
    return out


def _is_empty(v: Any) -> bool:
    """A value that carries no information: None, or an empty list/dict (the
    no-fabrication skeleton default). Such a value must NOT overwrite earlier real
    data during a same-hour merge (Codex 3rd-pass P1 — list/dict defaults were
    wiping ideas_all/focus.completed/equipment)."""
    return v is None or v == [] or v == {}


def _deep_null_aware_merge(av: Any, bv: Any) -> Any:
    """Recursively merge b into a: b wins ONLY where it carries information (non-empty),
    so a later partial/skeleton snapshot never wipes earlier complementary real data.
    Dicts merge key-wise recursively; non-dicts: b if b is informative else a."""
    if isinstance(av, dict) and isinstance(bv, dict):
        out = dict(av)
        for k, v in bv.items():
            out[k] = _deep_null_aware_merge(av.get(k), v) if k in av else v
        return out
    if isinstance(av, dict) and _is_empty(bv):
        return av
    return bv if not _is_empty(bv) else av


def _merge_two_snapshots(a: Dict[str, Any], b: Dict[str, Any]) -> Dict[str, Any]:
    merged = dict(a)
    # Keyed containers: merge per entity (TAG / state id / province id / war id),
    # recursively + null-aware so neither side clobbers the other's real data.
    for key in ("countries", "states", "provinces", "map", "diplomacy_world"):
        av, bv = a.get(key) or {}, b.get(key) or {}
        if isinstance(av, dict) and isinstance(bv, dict):
            m = dict(av)
            for ent, bval in bv.items():
                m[ent] = _deep_null_aware_merge(av.get(ent), bval)
            merged[key] = m
    # Top-level world catalogs (designs / order_of_battle + Tier-1 EXTRACT world
    # domains): null-aware deep merge so a same-hour partial snapshot (e.g. one
    # carrying only designs, the other only world) doesn't drop the sibling's data
    # while provenance still unions both domains (Codex integ-review finding 1).
    # Empty lists/dicts don't clobber real data.
    for key in ("designs", "order_of_battle", "strategic_air",
                "strategic_operatives", "raids", "world_events", "world"):
        av, bv = a.get(key), b.get(key)
        if av is not None or bv is not None:
            merged[key] = _deep_null_aware_merge(av, bv)
    # divisions: merge by identity (uid) so complementary fields combine and neither
    # side's roster is dropped.
    merged["divisions"] = _merge_div_lists(a.get("divisions") or [],
                                           b.get("divisions") or [])
    # combats: merge by combat_id (b wins on conflict; union the set).
    merged["combats"] = _merge_by_key(a.get("combats") or [], b.get("combats") or [],
                                      "combat_id")
    merged["events"] = (a.get("events") or []) + (b.get("events") or [])
    merged["provenance"] = _merge_provenance(a.get("provenance"), b.get("provenance"))
    return merged


def _merge_div_lists(la: List[Dict[str, Any]], lb: List[Dict[str, Any]]
                     ) -> List[Dict[str, Any]]:
    by_uid: Dict[str, Dict[str, Any]] = {}
    order: List[str] = []
    for d in la:
        uid = schema.div_uid(d) or id(d)
        if uid not in by_uid:
            order.append(uid)
        by_uid[uid] = d
    for d in lb:
        uid = schema.div_uid(d) or id(d)
        if uid in by_uid:
            by_uid[uid] = _deep_null_aware_merge(by_uid[uid], d)
        else:
            order.append(uid)
            by_uid[uid] = d
    return [by_uid[u] for u in order]


def _merge_by_key(la: List[Dict[str, Any]], lb: List[Dict[str, Any]], key: str
                  ) -> List[Dict[str, Any]]:
    out: Dict[Any, Dict[str, Any]] = {}
    order: List[Any] = []
    for d in la + lb:
        k = d.get(key) if isinstance(d, dict) else None
        kk = k if k is not None else id(d)
        if kk not in out:
            order.append(kk)
        out[kk] = d  # later (b) wins
    return [out[k] for k in order]


def _merge_provenance(pa: Any, pb: Any) -> Dict[str, Any]:
    """Union the captured domains per channel, but DOWNGRADE a domain to save when
    it was captured by save in EITHER input — a merged value that save touched must
    not be reported as memory-exact (Codex 2nd-pass P1)."""
    pa, pb = pa or {}, pb or {}
    out: Dict[str, Any] = {"notes": (pa.get("notes") or []) + (pb.get("notes") or [])}
    save_domains = set()
    for p in (pa, pb):
        s = p.get("save")
        if s and s.get("captured"):
            save_domains |= set(s.get("domains") or [])
    for ch in ("memory", "save"):
        a_ch, b_ch = pa.get(ch), pb.get(ch)
        if not a_ch and not b_ch:
            out[ch] = None
            continue
        a_ch, b_ch = a_ch or {}, b_ch or {}
        out[ch] = dict(a_ch)
        out[ch].update({k: v for k, v in b_ch.items() if k != "domains"})
        out[ch]["captured"] = bool(a_ch.get("captured") or b_ch.get("captured"))
        domains = set(a_ch.get("domains") or []) | set(b_ch.get("domains") or [])
        if ch == "memory":
            # a domain that save also covered here is no longer purely memory-exact
            domains -= save_domains
        out[ch]["domains"] = sorted(domains)
    return out


__all__ = ["diff_snapshots", "build_timeline"]
