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
    # against the right channel (schema P1: was country_scalars -> a save-derived
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
    # --- WAVE-2 events ---
    # abilities.json (aces richer source + commander cooldowns + officer corps)
    "ace_lost": "abilities",
    "commander_cooldown_started": "abilities",
    "commander_freed": "abilities",
    "general_captured": "abilities",
    "general_freed": "abilities",
    "officer_spirit_changed": "abilities",
    # projects.json (special projects / facilities / scientists)
    "project_finished": "projects",
    "project_started": "projects",
    "facility_built": "projects",
    "facility_dismantled": "projects",
    "scientist_specialization_up": "projects",
    # market_extra.json + deployment.json
    "unit_deployment_started": "deployment",
    "unit_deployment_completed": "deployment",
    "expeditionary_force_sent": "deployment",
    "expeditionary_force_recalled": "deployment",
    "market_subsidy_changed": "market_extra",
    "market_price_level_changed": "market_extra",
}


# NO-FABRICATION GATE (schema-verified 2026-06-27). The LIVE memory channel records
# DISTINCT provenance ids (live_capture/assembler.py): the `<canonical>_live` suffix
# convention (tech_live/commanders_live/intel_live/focus_live/production_live/
# diplomacy_live/projects_live/...) plus a few non-suffix ids (mio, oob_command), and a
# handful of BARE canonical ids the live reader also emits (state_control / province_
# control / divisions / designs). A canonical event-GATE may only treat one of these as
# "captured" when the live channel actually writes THIS reader's SAVE-SHAPED leaves --
# otherwise the save-shaped reader runs against empty leaves and FABRICATES events
# (tech_lost / peace / operation_completed / design_created / ...). We therefore FAIL
# CLOSED: a live id satisfies a save-semantic gate ONLY when it is proven leaf-compatible.
#
# Proven SAFE live aliases (audited leaf-by-leaf + reader-direction):
#   * focus_live -> focus  : writes focus.completed in the same focus-script-id namespace
#     the save channel uses, and _diff_focus is ADDITIONS-ONLY (n_done - o_done, never
#     removals) -> a live-empty completed[] can neither fabricate nor drop a focus_completed.
# Deliberately NOT safe (each writes leaves the reader does NOT read):
#   * tech_live (writes research.tech_count/slots, NOT research.technologies[] -> tech_lost)
#   * diplomacy_live (writes diplomacy.at_war[], NOT diplomacy.wars[] -> peace/war_declared;
#     the `diplomacy` gate also drives _diff_faction_autonomy on politics.* it never writes)
#   * intel_live (writes intel.agency.*, NOT intel.operations.running[] -> operation_completed)
#   * production_live (writes production.*, NOT economy.factories/_production_lines)
#   * commanders_live (writes commanders.army_leaders/navy_leaders, NOT generals/field_marshals/admirals)
#   * projects_live (writes top-level special_projects, NOT country.projects)
#   * mio (writes top-level mio_live, NOT designs.equipment_variants / country.mios)
#   * oob_command (writes order_of_battle.armies, but the army-id namespace vs the save
#     OOB is unproven AND _diff_order_of_battle emits army_disbanded for a-only ids -> a
#     mixed save->live-empty pair could fabricate mass disbands; left UNPROVEN -> fail closed)
_LIVE_DOMAIN_ALIASES = {
    "focus": ("focus_live",),  # the ONLY leaf-compatible, additions-only-safe live alias
}

# BARE canonical ids the live channel records that are NOT leaf-compatible with their
# gated reader, so the MEMORY channel must NOT let them satisfy that gate (the SAVE
# channel still can, via its own exact-membership check). Today only "designs": the live
# `designs` reader writes designs.division_templates ONLY -- it explicitly does NOT write
# designs.equipment_variants (-> design_created) nor country.mios (-> mio_leveled), both
# of which the "designs" gate drives. (state_control/province_control/divisions ARE
# leaf-compatible -- they write the controller / divisions[] leaves their readers read.)
_MEMORY_UNSAFE_CANONICAL = frozenset({"designs"})


def _matches_domain(domain_list, domain: str) -> bool:
    """True if a canonical event-gate `domain` is satisfied by a MEMORY-channel
    `domain_list`. (Called ONLY from _domain_is_memory; the save channel uses an exact
    membership check.) No-fabrication gate: a bare canonical id satisfies the gate unless
    it is a known leaf-INcompatible live id (_MEMORY_UNSAFE_CANONICAL); otherwise only a
    PROVEN-safe live alias (_LIVE_DOMAIN_ALIASES) counts. Any other `<domain>_live` /
    live id FAILS CLOSED -- the save-shaped reader runs only where its data really is."""
    if domain in domain_list:
        return domain not in _MEMORY_UNSAFE_CANONICAL
    return any(a in domain_list for a in _LIVE_DOMAIN_ALIASES.get(domain, ()))


# ---------------------------------------------------------------------------
# Provenance / channel-domains helpers (single source of truth)
#
# Two channel primitives — `_domain_is_memory` and `_domain_is_save` — answer
# "did the <memory|save> channel capture <domain> in this snapshot?". Everything
# else derives from them: `_domain_present` = memory OR save; `_provenance_recorded`
# = is ANY channel block present at all. Kept as one cluster so the channel logic
# lives in exactly one place.
# ---------------------------------------------------------------------------
def _domain_is_collector(snap: Dict[str, Any], domain: str) -> bool:
    """True if the in-process collector captured `domain` as schema-compatible live data."""
    prov = (snap or {}).get("provenance") or {}
    col = prov.get("collector") or {}
    return bool(col.get("captured")) and domain in (col.get("domains") or [])


def _domain_is_memory(snap: Dict[str, Any], domain: str) -> bool:
    """True if the MEMORY channel captured `domain` (honors the `<domain>_live`
    alias + live-id map so a pure-live capture's diff still runs)."""
    prov = (snap or {}).get("provenance") or {}
    mem = prov.get("memory") or {}
    return bool(mem.get("captured")) and _matches_domain(mem.get("domains") or [], domain)


def _domain_is_save(snap: Dict[str, Any], domain: str) -> bool:
    """True if the SAVE channel captured `domain` (canonical ids only)."""
    prov = (snap or {}).get("provenance") or {}
    sav = prov.get("save") or {}
    return bool(sav.get("captured")) and domain in (sav.get("domains") or [])


def _domain_present(snap: Dict[str, Any], domain: str) -> bool:
    """True if EITHER channel captured `domain`."""
    return (_domain_is_collector(snap, domain)
            or _domain_is_memory(snap, domain)
            or _domain_is_save(snap, domain))


def _provenance_recorded(snap: Dict[str, Any]) -> bool:
    """True if this snapshot states its provenance at all (memory or save block)."""
    prov = (snap or {}).get("provenance") or {}
    return (prov.get("collector") is not None
            or prov.get("memory") is not None
            or prov.get("save") is not None)


def _confidence_for(domain: str, a: Dict[str, Any], b: Dict[str, Any]) -> str:
    """exact if BOTH snapshots had the domain from memory (tick resolution)."""
    if (_domain_is_collector(a, domain) and _domain_is_collector(b, domain)
            or _domain_is_memory(a, domain) and _domain_is_memory(b, domain)):
        return "exact"
    return "interval"


def _source_for(domain: str, a: Dict[str, Any], b: Dict[str, Any]) -> str:
    """Honest event source (schema P2): a memory-exact diff is "M"; a diff that BOTH
    snapshots backed with the SAVE channel is "S" (it really IS save-sourced, not a
    generic derivation); everything else (mixed / pure-synthetic) is "D"."""
    if _domain_is_collector(a, domain) and _domain_is_collector(b, domain):
        return "C"
    if _domain_is_memory(a, domain) and _domain_is_memory(b, domain):
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
            # (schema P3 #15). interval_start_hour kept for back-compat.
            det.setdefault("interval_start_hour", hi_a)
            det.setdefault("interval", {"start_hour": hi_a, "start_date": date_a,
                                        "end_hour": hi_b, "end_date": date_b})
        if det:
            kw["detail"] = det
        events.append(schema.make_event(type, **kw))

    if a is None:
        # No prior state: emit nothing (baseline). Real timelines start at pair 2.
        return []

    # No-fabrication gate (schema 2nd-pass P1): only diff a domain when BOTH snapshots
    # actually captured it. Otherwise "uncaptured" would masquerade as "everything
    # changed" (all states null->TAG, all divisions created/destroyed, tech lost, ...).
    if _both_captured(a, b, "state_ownership"):
        _diff_states(a, b, emit, do_owner=True,
                     do_controller=_both_captured(a, b, "state_control"))
    elif _both_captured(a, b, "state_control"):
        _diff_states(a, b, emit, do_owner=False, do_controller=True)
    if _both_captured(a, b, "province_control"):
        _diff_provinces(a, b, emit)
    # Occupation governance is sourced from TWO domains with DISTINCT gates (schema
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
    # Union of keys so a state present in only one snapshot still diffs (schema P1 #2).
    for sid in set(sa) | set(sb):
        old = sa.get(sid) or {}
        new = sb.get(sid) or {}
        if do_owner:
            o_owner, n_owner = old.get("owner"), new.get("owner")
            if o_owner != n_owner and (o_owner is not None or n_owner is not None):
                # subject = new owner, or (on TAG->null) the losing tag (schema P2 #9)
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
    # destroy+create, not a spurious move+retemplate (schema P1 #3).
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
        # template identity is also (id, type) (schema 2nd-pass P2).
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
    # Gate STRICTLY on provenance (schema 2nd-pass P1): partial country maps built
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
    # but not the other can't fabricate events from empty defaults (schema 3rd-pass).
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
    # WAVE-2 deep domains, each gated on its OWN domain.
    do_abilities = _both_captured(a, b, "abilities")
    do_projects = _both_captured(a, b, "projects")
    do_deployment = _both_captured(a, b, "deployment")
    do_market_extra = _both_captured(a, b, "market_extra")
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
            # ACE DEDUP (schema-reviewed): within a single (a,b) diff, aces come from
            # exactly ONE source — abilities (id-keyed) when `do_abilities`, else
            # country_extra (name-keyed). `emit_aces=not do_abilities` guarantees the
            # two paths never BOTH emit ace_gained for the same pair, so no per-pair
            # double-count. `_diff_*` each read a UNIFORM source on both snapshots, so
            # no id-vs-name key mismatch can arise WITHIN a pair. The only residual is
            # cross-pair: a timeline that flips abilities-capture between adjacent pairs
            # uses id-keys for one pair and name-keys for another — that is consistent
            # per-pair, fires each ace once at its own gain hour, and is NOT a double
            # count (real ingested snapshots capture abilities uniformly anyway).
            _diff_country_extra(tag, old, new, emit, emit_aces=not do_abilities)
        if do_abilities:
            _diff_abilities(tag, old, new, emit)
        if do_projects:
            _diff_projects(tag, old, new, emit)
        if do_deployment:
            _diff_deployment(tag, old, new, emit)
        if do_market_extra:
            _diff_market_extra(tag, old, new, emit)


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
            # (schema 2nd-pass P2 — was missed when only the level was compared).
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
            # only an INCREASE is "researched"; a decrease is a rollback (schema P1 #4)
            if lvl > o_tech[name]:
                emit("tech_researched", subject=tag, key=name,
                     **{"from": o_tech[name]}, to=lvl, detail={"date": t.get("date")})
            elif lvl < o_tech[name]:
                emit("tech_lost", subject=tag, key=name,
                     **{"from": o_tech[name]}, to=lvl,
                     detail={"note": "level decreased (save rollback / civil war?)"})
    # tech that disappeared entirely = tech_lost.
    # NOTE: this assumes the captured technologies list is COMPLETE in both
    # snapshots. A truncated/partial tech list in `new` would make still-known
    # techs look "lost"; the _both_captured("designs"/tech) provenance gate is
    # what guarantees we only diff a fully-captured pair.
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
            # Old baseline (schema P2, refined): if the OLD archetype had a line whose
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
    UNMEASURED, not zero (schema P2): it is excluded from the gross sum so a later
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
        # occupation law (occupation-domain-gated ONLY — schema P1): a state_detail-
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
def _fired_event_identity(row: Any) -> Optional[Tuple]:
    """Hashable identity for one world_events.fired_events row.

    The validator (schema._validate_fired_event_history) admits THREE row shapes:
    legacy str ids, legacy int ids, and rich XUI21 dict rows. Legacy scalars key on
    the id text (int normalized to str — keys elsewhere are strings). Rich rows key
    on (event_id, recipient_tag, event_sequence): stable across snapshots because
    event_sequence is validated strictly increasing, and deliberately EXCLUDING the
    mutable resolution fields (resolution_reason, chosen_option_ordinal,
    resolved_tick) so a popup resolving between two snapshots does not re-emit
    event_fired. Any other row type -> None (the validator already flags it; the
    diff must not crash on it)."""
    if isinstance(row, str) or (isinstance(row, int) and not isinstance(row, bool)):
        return ("legacy", str(row))
    if isinstance(row, dict):
        return ("rich", str(row.get("event_id")), row.get("recipient_tag"),
                row.get("event_sequence"))
    return None


def _diff_world_events(a: Dict[str, Any], b: Dict[str, Any], emit) -> None:
    """event_fired: a scripted event newly appears in the persisted fired list.
    convoy_sunk: a per-month sunk-convoy ledger ENTRY newly appears.

    fired_events is an append-mostly persisted list; we set-diff it BY IDENTITY
    (_fired_event_identity — legacy scalar ids and rich XUI21 dict rows both
    normalize to a hashable key) so only genuinely new firings emit and a rich
    row's resolution-state change never re-fires. sunk_convoys entries are keyed
    by (month, killer, owner) — a new key, or a higher convoy count for an
    existing key, emits (the per-month tally can grow within the same month
    between two saves)."""
    wea = a.get("world_events") or {}
    web = b.get("world_events") or {}

    o_fired = set()
    for row in (wea.get("fired_events") or []):
        ident = _fired_event_identity(row)
        if ident is not None:
            o_fired.add(ident)
    n_fired: Dict[Tuple, Any] = {}
    for row in (web.get("fired_events") or []):
        ident = _fired_event_identity(row)
        if ident is not None and ident not in n_fired:
            n_fired[ident] = row
    for ident in sorted(set(n_fired) - o_fired, key=str):
        if ident[0] == "legacy":
            emit("event_fired", key=ident[1], to=ident[1])
        else:
            _, event_id, recipient, sequence = ident
            row = n_fired[ident]
            emit("event_fired",
                 key="%s@%s#%s" % (event_id, recipient, sequence),
                 to=event_id,
                 detail={"recipient": recipient,
                         "fire_date": row.get("fire_date")})

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
    """Stable per-ace identity. PREFER the save's ace_id (the abilities parser exposes
    `id={id type}` as ace_id — the old 'save has no ace id' note was wrong, WIRE_abilities
    §4); fall back to name+surname+callsign for the legacy country_extra ace shape
    (which carries no ace_id)."""
    aid = ac.get("ace_id")
    if aid is not None:
        return "id:%s" % aid
    return "%s|%s|%s" % (ac.get("name"), ac.get("surname"), ac.get("callsign"))


def _diff_country_extra(tag, old, new, emit, *, emit_aces: bool = True) -> None:
    """nuke_count_changed: countries.*.extra.nukes_total changed.
    ace_gained: a new ace (by identity) appears — ONLY when ``emit_aces`` (the richer
    abilities domain is not the ace source; ACE DEDUP, WIRE_abilities §4).
    project_completed: special_projects.projects_completed increased (by the delta)."""
    o_ext = old.get("extra") if isinstance(old.get("extra"), dict) else {}
    n_ext = new.get("extra") if isinstance(new.get("extra"), dict) else {}
    if not o_ext and not n_ext:
        return

    # nuke total (a real 0 is a measured value — only emit on an actual change)
    o_n, n_n = o_ext.get("nukes_total"), n_ext.get("nukes_total")
    if (isinstance(o_n, int) and isinstance(n_n, int) and o_n != n_n):
        emit("nuke_count_changed", subject=tag, **{"from": o_n}, to=n_n)

    # newly-gained aces (set-diff by ace identity). Suppressed when the abilities
    # domain owns aces, so the same ace never fires twice.
    if emit_aces:
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
# Abilities (abilities.json): ace lifecycle (richer source), commander cooldowns,
# general captures, officer-corps spirits (per-country, from countries.*.abilities).
# ---------------------------------------------------------------------------
def _diff_abilities(tag, old, new, emit) -> None:
    """ace_gained / ace_lost (richer abilities source, keyed by ace_id);
    commander_cooldown_started / commander_freed (keyed by unit_leader_id);
    general_captured / general_freed (captured flag flips);
    officer_spirit_changed (set-diff of officer_corps_spirits tokens).

    Owns the ace_gained event when the abilities domain is captured (DEDUP: the
    country_extra emission is suppressed in that case)."""
    o_ab = old.get("abilities") if isinstance(old.get("abilities"), dict) else {}
    n_ab = new.get("abilities") if isinstance(new.get("abilities"), dict) else {}
    if not o_ab and not n_ab:
        return

    # aces — gained/lost by ace identity (ace_id preferred).
    o_aces = {_ace_key(x): x for x in (o_ab.get("aces") or []) if isinstance(x, dict)}
    n_aces = {_ace_key(x): x for x in (n_ab.get("aces") or []) if isinstance(x, dict)}
    for k, ac in n_aces.items():
        if k not in o_aces:
            emit("ace_gained", subject=tag, key=k,
                 to=ac.get("callsign") or ac.get("surname"),
                 detail={"name": ac.get("name"), "surname": ac.get("surname"),
                         "callsign": ac.get("callsign"), "modifier": ac.get("modifier"),
                         "ace_category": ac.get("ace_category"), "tier": ac.get("tier")})
    for k, ac in o_aces.items():
        # lost = gone from the roster, OR alive flipped true->false.
        n_ac = n_aces.get(k)
        was_alive = ac.get("alive") is not False
        now_dead = (n_ac is None) or (n_ac.get("alive") is False)
        if was_alive and now_dead:
            killer = (n_ac or {}).get("killer_country") or ac.get("killer_country")
            emit("ace_lost", subject=tag, key=k,
                 **{"from": ac.get("callsign") or ac.get("surname")},
                 object=killer,
                 detail={"name": ac.get("name"), "callsign": ac.get("callsign"),
                         "killer_country": killer})

    # commander cooldowns + captures (keyed by unit_leader_id).
    def _cd_index(ab):
        out = {}
        for r in (ab.get("commander_cooldowns") or []):
            if isinstance(r, dict) and r.get("unit_leader_id") is not None:
                out[str(r["unit_leader_id"])] = r
        return out

    oc, nc = _cd_index(o_ab), _cd_index(n_ab)
    for ident in set(oc) | set(nc):
        o_r, n_r = oc.get(ident), nc.get(ident)
        o_reason = (o_r or {}).get("cooldown_reason")
        n_reason = (n_r or {}).get("cooldown_reason")
        if not o_reason and n_reason:
            emit("commander_cooldown_started", subject=tag, key=ident, to=n_reason,
                 detail={"role": (n_r or {}).get("role"),
                         "enable_date": (n_r or {}).get("modifier_enable_date")})
        elif o_reason and not n_reason:
            emit("commander_freed", subject=tag, key=ident, **{"from": o_reason},
                 detail={"role": (o_r or {}).get("role")})
        # capture flag flip (only meaningful when the leader exists both sides).
        if o_r is not None and n_r is not None:
            o_cap = bool(o_r.get("captured"))
            n_cap = bool(n_r.get("captured"))
            if not o_cap and n_cap:
                emit("general_captured", subject=tag, key=ident,
                     object=n_r.get("captured_by"),
                     detail={"role": n_r.get("role"),
                             "location": n_r.get("location")})
            elif o_cap and not n_cap:
                emit("general_freed", subject=tag, key=ident,
                     **{"from": o_r.get("captured_by")},
                     detail={"role": o_r.get("role")})

    # officer-corps spirits (set-diff of tokens).
    def _spirits(ab):
        oc_blk = ab.get("officer_corps") if isinstance(ab.get("officer_corps"), dict) else {}
        return {s.get("token") for s in (oc_blk.get("officer_corps_spirits") or [])
                if isinstance(s, dict) and s.get("token")}

    o_sp, n_sp = _spirits(o_ab), _spirits(n_ab)
    for tok in sorted((n_sp - o_sp) | (o_sp - n_sp)):
        emit("officer_spirit_changed", subject=tag, key=tok,
             to=tok if tok in n_sp else None,
             **({} if tok in n_sp else {"from": tok}))


# ---------------------------------------------------------------------------
# Projects (projects.json): special-project lifecycle, facilities, scientists
# (per-country, from countries.*.projects + countries.*.scientists).
# ---------------------------------------------------------------------------
def _diff_projects(tag, old, new, emit) -> None:
    """project_finished: a NAMED token enters completed_projects.
    project_started: a token enters active_projects.
    facility_built / facility_dismantled: a facility id appears / disappears (or
    dismantle_in_progress flips true). Diffs token SETS + flags, never raw floats.

    DISTINCT from the existing `project_completed` (schema-reviewed): that event is a
    COUNT-delta from country_extra.special_projects.projects_completed (sourced from a
    different parser, fired in _diff_country_extra) — a coarse "N more finished" signal.
    `project_finished` is a per-TOKEN event naming WHICH project finished. They are
    different event TYPES with different keys/semantics; a consumer must not sum across
    both names as if they were the same metric (they describe the same underlying fact
    at two granularities). This is enrichment, NOT a double-count of a single event."""
    o_pj = old.get("projects") if isinstance(old.get("projects"), dict) else {}
    n_pj = new.get("projects") if isinstance(new.get("projects"), dict) else {}
    if not o_pj and not n_pj:
        return

    # completed-project token SET (names which project finished — enriches the
    # existing count-based project_completed, no double-fire: different event type).
    o_done = set(o_pj.get("completed_projects") or [])
    n_done = set(n_pj.get("completed_projects") or [])
    for tok in sorted(n_done - o_done):
        emit("project_finished", subject=tag, key=tok, to=tok)

    # active-project token SET (started).
    def _active_tokens(pj):
        return {p.get("token") for p in (pj.get("active_projects") or [])
                if isinstance(p, dict) and p.get("token")}

    o_act, n_act = _active_tokens(o_pj), _active_tokens(n_pj)
    for tok in sorted(n_act - o_act):
        emit("project_started", subject=tag, key=tok, to=tok)

    # facilities (keyed by id) — built / dismantled.
    def _fac_index(pj):
        out = {}
        for f in (pj.get("facilities") or []):
            if isinstance(f, dict) and f.get("id") is not None:
                out[str(f["id"])] = f
        return out

    o_fac, n_fac = _fac_index(o_pj), _fac_index(n_pj)
    for fid in set(n_fac) - set(o_fac):
        f = n_fac[fid]
        emit("facility_built", subject=tag, key=fid, to=f.get("type"),
             detail={"type": f.get("type"), "province": f.get("province")})
    for fid in set(o_fac) - set(n_fac):
        f = o_fac[fid]
        emit("facility_dismantled", subject=tag, key=fid, **{"from": f.get("type")},
             detail={"type": f.get("type"), "province": f.get("province")})
    # a facility entering dismantle while still present (flag flip false->true).
    for fid in set(o_fac) & set(n_fac):
        o_dis = bool(o_fac[fid].get("dismantle_in_progress"))
        n_dis = bool(n_fac[fid].get("dismantle_in_progress"))
        if not o_dis and n_dis:
            emit("facility_dismantled", subject=tag, key=fid,
                 to=n_fac[fid].get("type"),
                 detail={"type": n_fac[fid].get("type"), "in_progress": True})

    # scientist specialization increases (per char_id; from countries.*.scientists).
    _diff_scientists(tag, old, new, emit)


def _diff_scientists(tag, old, new, emit) -> None:
    """scientist_specialization_up: a scientist's skills.specialization_* increases."""
    def _index(country):
        out = {}
        for s in (country.get("scientists") or []):
            if isinstance(s, dict) and s.get("char_id") is not None:
                out[str(s["char_id"])] = s
        return out

    oi, ni = _index(old), _index(new)
    for cid in set(oi) & set(ni):
        o_sk = oi[cid].get("skills") or {}
        n_sk = ni[cid].get("skills") or {}
        for spec in set(n_sk) | set(o_sk):
            ov, nv = o_sk.get(spec), n_sk.get(spec)
            if isinstance(ov, int) and isinstance(nv, int) and nv > ov:
                emit("scientist_specialization_up", subject=tag, key="%s:%s" % (cid, spec),
                     **{"from": ov}, to=nv,
                     detail={"scientist": ni[cid].get("name_token"),
                             "specialization": spec})


# ---------------------------------------------------------------------------
# Deployment (deployment.json): deploying-division conveyor + expeditionary
# (per-country, from countries.*.military + diplomacy.expeditionary).
# ---------------------------------------------------------------------------
def _diff_deployment(tag, old, new, emit) -> None:
    """unit_deployment_started / unit_deployment_completed: a conveyor (keyed by
    its exact serialized ``id={id type}``) appears / disappears.
    expeditionary_force_sent / expeditionary_force_recalled: an (owner,controller)
    lent-division pairing appears/grows / disappears/shrinks."""
    def _conv_index(country):
        out = {}
        queue = (country.get("military") or {}).get("deployment_queue")
        if not isinstance(queue, list):
            return None
        for c in queue:
            if not isinstance(c, dict):
                return None
            cid = c.get("id")
            if (not isinstance(cid, list) or len(cid) != 2
                    or any(type(value) is not int for value in cid)):
                return None
            key = "%d:%d" % (cid[0], cid[1])
            if key in out:
                return None
            out[key] = c
        return out

    oc, nc = _conv_index(old), _conv_index(new)
    # Any malformed/missing/duplicate identity invalidates the complete vector;
    # emitting from a filtered subset could fabricate starts or completions.
    if oc is not None and nc is not None:
        for key in sorted(set(nc) - set(oc)):
            c = nc[key]
            emit("unit_deployment_started", subject=tag, key=key, to=c.get("location"),
                 detail={"id": c.get("id"), "ordinal": c.get("ordinal"),
                         "name": c.get("name"),
                         "template_id": c.get("template_id"),
                         "amount": c.get("amount"), "priority": c.get("priority"),
                         "role": c.get("role"), "closed": c.get("closed"),
                         "line_count": len(c.get("lines") or [])})
        for key in sorted(set(oc) - set(nc)):
            c = oc[key]
            emit("unit_deployment_completed", subject=tag, key=key,
                 **{"from": c.get("location")},
                 detail={"id": c.get("id"), "ordinal": c.get("ordinal"),
                         "name": c.get("name"),
                         "template_id": c.get("template_id"),
                         "amount": c.get("amount"), "priority": c.get("priority"),
                         "role": c.get("role"), "closed": c.get("closed"),
                         "line_count": len(c.get("lines") or [])})

    # expeditionary (per-owner; keyed by controller). count = number of divisions.
    def _exp_index(country):
        out = {}
        for e in ((country.get("diplomacy") or {}).get("expeditionary") or []):
            if isinstance(e, dict) and e.get("controller"):
                out[str(e["controller"])] = len(e.get("divisions") or [])
        return out

    oe, ne = _exp_index(old), _exp_index(new)
    for ctrl in set(ne) | set(oe):
        ov, nv = oe.get(ctrl, 0), ne.get(ctrl, 0)
        if nv > ov:
            emit("expeditionary_force_sent", subject=tag, object=ctrl,
                 **{"from": ov}, to=nv)
        elif nv < ov:
            emit("expeditionary_force_recalled", subject=tag, object=ctrl,
                 **{"from": ov}, to=nv)


# ---------------------------------------------------------------------------
# Market extra (market_extra.json): equipment-market subsidies + manual price bands
# (per-country, from countries.*.economy.market_*).
# ---------------------------------------------------------------------------
def _diff_market_extra(tag, old, new, emit) -> None:
    """market_subsidy_changed: a country's subsidies[] set changes (add/remove/cic).
    market_price_level_changed: a manual price band changes (dormant on auto-price)."""
    o_eco = old.get("economy") if isinstance(old.get("economy"), dict) else {}
    n_eco = new.get("economy") if isinstance(new.get("economy"), dict) else {}

    def _subs_index(eco):
        out = {}
        for s in (eco.get("market_subsidies") or []):
            if isinstance(s, dict) and s.get("archetype") is not None:
                out[str(s["archetype"])] = s.get("cic")
        return out

    os_, ns_ = _subs_index(o_eco), _subs_index(n_eco)
    for arch in set(os_) | set(ns_):
        ov, nv = os_.get(arch), ns_.get(arch)
        if ov != nv:
            emit("market_subsidy_changed", subject=tag, key=str(arch),
                 **{"from": ov}, to=nv)

    # price-level set (dormant until a manual band is set; harmless no-op otherwise).
    o_pl = o_eco.get("market_price_levels") or []
    n_pl = n_eco.get("market_price_levels") or []
    if o_pl != n_pl and (o_pl or n_pl):
        emit("market_price_level_changed", subject=tag,
             detail={"from_count": len(o_pl), "to_count": len(n_pl)})


# ---------------------------------------------------------------------------
# Marker events + merge + dedupe (SCHEMA §9.3)
# ---------------------------------------------------------------------------
def _event_sort_key(ev: Dict[str, Any]) -> Tuple:
    return (ev.get("hour_index") if ev.get("hour_index") is not None else 1 << 62,
            ev.get("type") or "", str(ev.get("key") or ""))


# A distinct sentinel for a genuinely-absent/None field so it can never collide
# with the literal STRING "None" once stringified (schema: `str(None) == "None"` let
# a marker carrying the literal "None" supersede a real `from=None` diff).
_NONE = object()


def _dedupe_atom(v: Any) -> Any:
    """Identity atom for a dedupe field: None maps to a unique sentinel (NOT the
    string "None"); everything else stringifies as before so a numeric `1` and the
    string `"1"` still match (the established behavior)."""
    return _NONE if v is None else str(v)


def _dedupe_key(ev: Dict[str, Any]) -> Tuple:
    # Include 'from' so an A->B flip and a B->A flip are NOT collapsed (P2 #8), and
    # 'object' so e.g. two subject_created with different overlords don't collide
    # (schema 2nd-pass P2). None != the literal "None" (schema: _dedupe_atom sentinel).
    return (ev.get("type"), _dedupe_atom(ev.get("key")), _dedupe_atom(ev.get("from")),
            _dedupe_atom(ev.get("to")), _dedupe_atom(ev.get("subject")),
            _dedupe_atom(ev.get("object")))


# Paired inverse event TYPES where the DIRECTION of the change lives in the type name,
# not in from/to (schema-verified 2026-06-27). The two members of each pair describe the
# SAME underlying identity in opposite directions, so for the marker-oscillation
# "last marker wins" logic they must share ONE identity family — otherwise a later
# reversing marker of the OPPOSITE type is invisible to the diff and the (now-reversed)
# earlier marker still supersedes a survivor.
_PAIR_FAMILIES = {
    "faction_joined": "faction_membership", "faction_left": "faction_membership",
    "idea_gained": "idea_presence", "idea_lost": "idea_presence",
    "tech_researched": "tech_state", "tech_lost": "tech_state",
    "subject_created": "subject_relation", "subject_freed": "subject_relation",
    "war_declared": "war_state", "peace": "war_state",
}

# Event types whose subject/object are DIRECTION-bearing (subject = the NEW owner, object
# = the OLD owner), so they must be EXCLUDED from the identity — leaving (type, key) — or
# an A->B and a B->A flip land in different buckets and the oscillation guard fails.
_DIRECTIONAL_SUBJECT_OBJECT = frozenset({
    "state_owner_change", "state_controller_change", "province_controller_change",
})


def _dedupe_identity(ev: Dict[str, Any]) -> Tuple:
    """Direction-AGNOSTIC identity of the changing thing (schema-verified). Used to group
    a diff with the markers that touch the SAME identity so the LAST (net) marker decides
    supersession. Strips the value/direction (from/to is never part of identity); folds
    paired inverse TYPES into one family; and drops subject/object for the owner/controller
    types where those fields encode direction. The FULL _dedupe_key (with type + from/to +
    subject/object) is still what the last marker is COMPARED against."""
    typ = ev.get("type")
    fam = _PAIR_FAMILIES.get(typ, typ)
    if typ in _DIRECTIONAL_SUBJECT_OBJECT:
        return (fam, _dedupe_atom(ev.get("key")))
    return (fam, _dedupe_atom(ev.get("key")),
            _dedupe_atom(ev.get("subject")), _dedupe_atom(ev.get("object")))


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
    # two partial views of one instant into fabricated transitions (schema P3 #12).
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


_RAW_METADATA_FIELDS = (
    "sequence", "session_sequence", "tick_ms", "sample_ms", "capture_boundary",
    "domain", "entity_key", "diagnostics",
)


def build_timeline_with_raw_events(snapshots: List[Dict[str, Any]],
                                   raw_events: List[Dict[str, Any]],
                                   *,
                                   milestone_steps: Optional[Dict[str, float]] = None
                                   ) -> List[Dict[str, Any]]:
    """Build the normal snapshot timeline, then merge already-validated raw frames.

    Exact collector raw events supersede only interval snapshot diffs with the same
    identity inside that diff's interval. Sampled/proxy raw observations and gap
    markers remain visible but never delete snapshot-derived fallback events.
    """
    snapshot_events = build_timeline(snapshots, milestone_steps=milestone_steps)
    normalized_raw: List[Dict[str, Any]] = []
    for frame in raw_events or []:
        normalized = _normalize_raw_frame(frame)
        if normalized is not None:
            normalized_raw.append(normalized)
    return _merge_raw_events(snapshot_events, normalized_raw)


def _normalize_raw_frame(frame: Any) -> Optional[Dict[str, Any]]:
    if not isinstance(frame, dict):
        return None
    frame_type = frame.get("type")
    if frame_type == "event":
        return _normalize_raw_event(frame)
    if frame_type == "gap":
        return _normalize_raw_gap(frame)
    return None


def _raw_payload(frame: Dict[str, Any]) -> Dict[str, Any]:
    payload = frame.get("payload")
    return payload if isinstance(payload, dict) else {}


def _normalize_raw_event(frame: Dict[str, Any]) -> Optional[Dict[str, Any]]:
    payload = _raw_payload(frame)
    raw_event_type = payload.get("event_type") or frame.get("event_type")
    event_type = _canonical_raw_event_type(frame, payload, raw_event_type)
    if event_type is None or event_type not in schema.EVENT_TYPES:
        return None

    raw_confidence = frame.get("confidence") or payload.get("confidence")
    confidence = (raw_confidence if raw_confidence in schema.CONFIDENCE_LEVELS
                  else "inferred")
    source = frame.get("source") or "C"
    if source not in schema.EVENT_SOURCES:
        source = "D"

    detail = payload.get("detail") if isinstance(payload.get("detail"), dict) else {}
    detail = dict(detail)
    if raw_event_type != event_type and _contains_forbidden_raw_key(detail):
        return None
    if raw_event_type != event_type:
        detail.setdefault("raw_event_type", raw_event_type)
    if raw_confidence is not None and raw_confidence not in schema.CONFIDENCE_LEVELS:
        detail.setdefault("raw_confidence", raw_confidence)
    raw_detail = {field: frame[field] for field in _RAW_METADATA_FIELDS
                  if field in frame and frame.get(field) is not None}
    if raw_detail:
        detail.setdefault("raw", raw_detail)

    frm = payload.get("from")
    to = payload.get("to")
    subject = payload.get("subject")
    if subject is None and event_type in _DIRECTIONAL_SUBJECT_OBJECT:
        subject = to if to is not None else frm
    obj = payload.get("object")
    if obj is None and event_type in _DIRECTIONAL_SUBJECT_OBJECT:
        obj = frm

    ev = schema.make_event(
        event_type,
        hour_index=frame.get("hour_index"),
        date=frame.get("date"),
        subject=subject,
        object=obj,
        key=payload.get("key", frame.get("entity_key")),
        source=source,
        confidence=confidence,
        detail=detail or None,
        **{"from": frm},
        to=to,
    )
    _copy_raw_metadata(frame, ev)
    return ev


def _canonical_raw_event_type(
    frame: Dict[str, Any],
    payload: Dict[str, Any],
    raw_event_type: Any,
) -> Optional[str]:
    if raw_event_type in schema.EVENT_TYPES:
        return raw_event_type
    if frame.get("domain") != "land_combats" and payload.get("domain") != "land_combats":
        return None
    if raw_event_type == "land_combat_started":
        return "combat_started"
    if raw_event_type == "land_combat_ended":
        return "combat_ended"
    return None


def _contains_forbidden_raw_key(value: Any) -> bool:
    return schema._contains_forbidden_raw_key(value)


def _normalize_raw_gap(frame: Dict[str, Any]) -> Dict[str, Any]:
    payload = _raw_payload(frame)
    detail = payload.get("detail") if isinstance(payload.get("detail"), dict) else {}
    detail = dict(detail)
    raw_detail = {field: frame[field] for field in _RAW_METADATA_FIELDS
                  if field in frame and frame.get(field) is not None}
    if raw_detail:
        detail.setdefault("raw", raw_detail)
    marker: Dict[str, Any] = {
        "type": "gap",
        "hour_index": frame.get("hour_index"),
        "date": frame.get("date"),
        "source": frame.get("source") or "C",
        "confidence": "gap",
        "domain": payload.get("domain", frame.get("domain")),
        "gap_kind": payload.get("gap_kind", frame.get("gap_kind")),
        "start_sequence": payload.get("start_sequence", frame.get("start_sequence")),
        "end_sequence": payload.get("end_sequence", frame.get("end_sequence")),
        "reason": payload.get("reason", frame.get("reason")),
    }
    if detail:
        marker["detail"] = detail
    _copy_raw_metadata(frame, marker)
    return marker


def _copy_raw_metadata(frame: Dict[str, Any], target: Dict[str, Any]) -> None:
    for field in _RAW_METADATA_FIELDS:
        if field in frame and frame.get(field) is not None:
            target[field] = frame[field]


def _merge_raw_events(snapshot_events: List[Dict[str, Any]],
                      raw_events: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
    raw_superseders: Dict[Tuple, List[Dict[str, Any]]] = {}
    for ev in raw_events:
        if not _is_exact_collector_raw_event(ev):
            continue
        identity = _raw_merge_identity(ev)
        if identity is not None:
            raw_superseders.setdefault(identity, []).append(ev)

    kept: List[Dict[str, Any]] = []
    for ev in snapshot_events:
        if not _interval_diff_superseded_by_raw(ev, raw_superseders):
            kept.append(ev)
    return sorted(kept + raw_events, key=_raw_timeline_sort_key)


def _is_exact_collector_raw_event(ev: Dict[str, Any]) -> bool:
    return (ev.get("type") != "gap"
            and ev.get("source") == "C"
            and ev.get("confidence") == "exact"
            and isinstance(ev.get("hour_index"), int))


def _interval_diff_superseded_by_raw(
    ev: Dict[str, Any],
    raw_superseders: Dict[Tuple, List[Dict[str, Any]]],
) -> bool:
    if ev.get("confidence") != "interval":
        return False
    hi = ev.get("hour_index")
    if not isinstance(hi, int):
        return False
    identity = _raw_merge_identity(ev)
    if identity is None:
        return False
    candidates = raw_superseders.get(identity) or []
    if not candidates:
        return False
    lo = _event_interval_start(ev, hi)
    return any(lo < raw_ev["hour_index"] <= hi for raw_ev in candidates)


def _event_interval_start(ev: Dict[str, Any], fallback_end: int) -> int:
    detail = ev.get("detail") if isinstance(ev.get("detail"), dict) else {}
    start = detail.get("interval_start_hour")
    if not isinstance(start, int):
        start = (detail.get("interval") or {}).get("start_hour")
    return start if isinstance(start, int) else fallback_end - 1


def _raw_merge_identity(ev: Dict[str, Any]) -> Optional[Tuple]:
    typ = ev.get("type")
    if typ is None or typ == "gap":
        return None
    if typ in _DIRECTIONAL_SUBJECT_OBJECT:
        return (typ, _dedupe_atom(ev.get("key")), _dedupe_atom(ev.get("from")),
                _dedupe_atom(ev.get("to")))
    return _dedupe_key(ev)


def _raw_timeline_sort_key(ev: Dict[str, Any]) -> Tuple:
    big = 1 << 62

    def int_or_big(value: Any) -> int:
        return value if isinstance(value, int) else big

    return (int_or_big(ev.get("hour_index")),
            int_or_big(ev.get("tick_ms")),
            int_or_big(ev.get("sequence")),
            ev.get("type") or "",
            str(ev.get("key") or ev.get("entity_key") or ""))


def _merge_dedupe(events: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
    """Sort, then drop a diff-derived event when an exact marker reports the SAME net
    transition over the diff's window. The dedupe window is INTERVAL-AWARE (schema P2 #8):

      * for an interval diff, the window is (interval_start_hour, hour_index] — exactly
        the span the change is known to have occurred in (handles monthly saves where
        the marker can be >24h before the later bound);
      * for an exact (memory) diff, only a marker at the same tick supersedes it
        (no fuzzy window — hourly memory must not eat a real repeated flip).

    NET-MARKER RULE (schema-verified 2026-06-27, fixes the oscillation drop): a diff is
    superseded only when the LAST marker for that diff's IDENTITY in the window agrees
    with the diff's full (type,from,to,subject,object) key. Identity is direction-
    agnostic (`_dedupe_identity`: strips from/to, folds paired inverse types into one
    family, drops the directional subject/object of owner/controller events). So an
    A->B->A->B oscillation no longer lets an EARLIER, already-reversed A->B marker eat the
    net A->B endpoint diff — if the last marker reversed the change, the marker's key
    differs from the diff's and the endpoint diff SURVIVES. Ties at the max hour with
    >1 distinct key FAIL OPEN (keep the diff): never delete a real endpoint on ambiguity.
    """
    events = sorted(events, key=_event_sort_key)
    # identity -> chronological list of (hour, full_dedupe_key) for exact markers.
    markers_by_identity: Dict[Tuple, List[Tuple[int, Tuple]]] = {}
    for ev in events:
        if ev.get("source") == "marker" and ev.get("confidence") == "exact":
            hi = ev.get("hour_index")
            if hi is not None:
                markers_by_identity.setdefault(_dedupe_identity(ev), []).append(
                    (hi, _dedupe_key(ev)))

    kept: List[Dict[str, Any]] = []
    for ev in events:
        if ev.get("source") == "marker":
            kept.append(ev)
            continue
        k = _dedupe_key(ev)
        hi = ev.get("hour_index")
        superseded = False
        marks = markers_by_identity.get(_dedupe_identity(ev)) if hi is not None else None
        if marks:
            if ev.get("confidence") == "interval":
                det = ev.get("detail") or {}
                start = det.get("interval_start_hour")
                if not isinstance(start, int):
                    start = (det.get("interval") or {}).get("start_hour")
                lo = start if isinstance(start, int) else hi - 1
                in_window = [(mh, fk) for mh, fk in marks if lo < mh <= hi]
            else:  # exact: same-tick only
                in_window = [(mh, fk) for mh, fk in marks if mh == hi]
            if in_window:
                # The LAST (net) marker for this identity decides. Ties at the latest
                # hour with conflicting keys are ambiguous -> fail open (keep the diff).
                max_h = max(mh for mh, _ in in_window)
                last_fks = [fk for mh, fk in in_window if mh == max_h]
                if len(set(last_fks)) == 1:
                    superseded = (last_fks[-1] == k)
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
    data during a same-hour merge (schema 3rd-pass P1 — list/dict defaults were
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
    # Generic pass: EVERY top-level key present in either input merges null-aware.
    # Dict-dict recursion in _deep_null_aware_merge gives keyed containers
    # (countries / states / provinces / map / diplomacy_world) their per-entity
    # merge, world catalogs their domain merge, and scalars the documented
    # later-wins-only-when-informative policy. Enumerating merge keys by hand had
    # silently dropped the later snapshot's live sidecar domains (mio_live /
    # market_live / diplomacy_live / special_projects / naval_air / supply /
    # weather / models / intel_ledger / occupation_history) while
    # _merge_provenance still unioned their domains — captured-without-data, a
    # no-fabrication violation in reverse. The identity/lifecycle collections and
    # completeness claims below need per-row or claim semantics, not a deep merge.
    _special = ("divisions", "combats", "ended_combats", "events", "provenance",
                "occupation_history_complete")
    for key in list(a) + [k for k in b if k not in a]:
        if key in _special:
            continue
        if key not in a:
            merged[key] = b[key]
        elif key not in b:
            merged[key] = a[key]
        else:
            merged[key] = _deep_null_aware_merge(a[key], b[key])
    # occupation_history_complete is a completeness CLAIM, not data: None is
    # no-claim, and a False (incomplete) from EITHER same-hour view must never be
    # fail-opened into True by the other view (a plain informative-later-wins
    # merge would do exactly that).
    if "occupation_history_complete" in a or "occupation_history_complete" in b:
        oa = a.get("occupation_history_complete")
        ob = b.get("occupation_history_complete")
        if oa is False or ob is False:
            merged["occupation_history_complete"] = False
        elif oa is True or ob is True:
            merged["occupation_history_complete"] = True
        else:
            merged["occupation_history_complete"] = None
    # divisions: merge by identity (uid) so complementary fields combine and neither
    # side's roster is dropped.
    merged["divisions"] = _merge_div_lists(a.get("divisions") or [],
                                           b.get("divisions") or [])
    # combats: merge by combat_id (b wins on conflict; union the set).
    merged["combats"] = _merge_by_key(a.get("combats") or [], b.get("combats") or [],
                                      "combat_id")
    # Terminal combat lifecycle is a distinct collection. Combat ids are reused,
    # so the only safe identity is the complete (combat_id, generation) pair.
    # Same-key rows merge null-aware to preserve complementary terminal detail;
    # a measured-present detail row outranks an unavailable observation in
    # either stitching order.
    if "ended_combats" in a or "ended_combats" in b:
        av = a.get("ended_combats")
        bv = b.get("ended_combats")
        lists = [value for value in (av, bv) if isinstance(value, list)]
        if lists:
            merged["ended_combats"] = _merge_ended_combat_rows(
                lists[0], lists[1] if len(lists) > 1 else []
            )
        elif av is None or bv is None:
            # Preserve the producer's unavailable state when neither input
            # contributes authenticated rows. Do not manufacture known-empty.
            merged["ended_combats"] = None
        else:
            merged["ended_combats"] = _deep_null_aware_merge(av, bv)
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


def _merge_ended_combat_rows(
    la: List[Dict[str, Any]],
    lb: List[Dict[str, Any]],
) -> List[Dict[str, Any]]:
    """Union terminal combat observations without degrading retained detail.

    Terminal combat ids are reusable, so `(combat_id, generation)` is the
    identity.  A same-hour row badged `present` owns more information than an
    `unavailable` observation of the same terminal event.  Merge the less
    informative row first so the present row wins scalar conflicts while the
    null-aware merge can still retain complementary non-empty leaves.
    """
    out: Dict[Any, Dict[str, Any]] = {}
    order: List[Any] = []
    for row in la + lb:
        if isinstance(row, dict) and all(
            row.get(key) is not None for key in ("combat_id", "generation")
        ):
            identity: Any = (row["combat_id"], row["generation"])
        else:
            # Preserve malformed producer data for schema validation to reject;
            # coalescing must not silently discard it.
            identity = id(row)
        if identity not in out:
            order.append(identity)
            out[identity] = row
            continue

        previous = out[identity]
        previous_present = (
            isinstance(previous, dict)
            and previous.get("side_detail_availability") == "present"
        )
        row_present = (
            isinstance(row, dict)
            and row.get("side_detail_availability") == "present"
        )
        if previous_present and not row_present:
            out[identity] = _deep_null_aware_merge(row, previous)
        else:
            out[identity] = _deep_null_aware_merge(previous, row)
    return [out[identity] for identity in order]


def _merge_provenance(pa: Any, pb: Any) -> Dict[str, Any]:
    """Union the captured domains per channel, but DOWNGRADE a domain to save when
    it was captured by save in EITHER input — a merged value that save touched must
    not be reported as memory-exact (schema 2nd-pass P1)."""
    pa, pb = pa or {}, pb or {}
    out: Dict[str, Any] = {"notes": (pa.get("notes") or []) + (pb.get("notes") or [])}
    save_domains = set()
    for p in (pa, pb):
        s = p.get("save")
        if s and s.get("captured"):
            save_domains |= set(s.get("domains") or [])
    for ch in ("collector", "memory", "save"):
        a_ch, b_ch = pa.get(ch), pb.get(ch)
        if not a_ch and not b_ch:
            out[ch] = None
            continue
        a_ch, b_ch = a_ch or {}, b_ch or {}
        out[ch] = dict(a_ch)
        out[ch].update({k: v for k, v in b_ch.items() if k != "domains"})
        out[ch]["captured"] = bool(a_ch.get("captured") or b_ch.get("captured"))
        domains = set(a_ch.get("domains") or []) | set(b_ch.get("domains") or [])
        if ch in ("collector", "memory"):
            # a domain that save also covered here is no longer purely live-exact
            domains -= save_domains
        out[ch]["domains"] = sorted(domains)
    return out


__all__ = ["diff_snapshots", "build_timeline", "build_timeline_with_raw_events"]
