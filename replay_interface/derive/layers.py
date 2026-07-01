"""Derived ("D"-source) replay layers, computed offline from a snapshot.

Every function here takes a snapshot dict (the canonical shape in
``replay/SCHEMA.md``) plus static map assets and returns ONE derived layer.
``derive_layers`` overlays them onto a (copied) snapshot.

Honesty rules (DESIGN no-fabrication; reinforced by the Codex model review):
  * A field stays ``null`` when its inputs are absent. Aggregates skip null
    members and report coverage; they never impute.
  * Proxy values are NOT laundered into human-count fields. The deployed
    manpower proxy lives in ``deployed_strength_proxy`` (NOT ``deployed``);
    war losses live in ``casualties.total`` (NOT ``killed``); ship tonnage,
    which the save does not carry, stays ``null``.
  * The encirclement layer is an ESTIMATE over a same-controller supply graph,
    not the engine's authoritative encirclement. See ``encirclement`` below for
    the exact model and where it returns ``null`` rather than a confident value.
"""

from __future__ import annotations

import copy
from collections import defaultdict, deque
from typing import Any, Dict, List, Optional, Set, Tuple

from . import adjacency as _adj

# ---------------------------------------------------------------------------
# tunables / documented constants
# ---------------------------------------------------------------------------
# building tokens that count as "industry" for the controlled-industry share
_INDUSTRY_BUILDINGS = ("industrial_complex", "arms_factory", "dockyard")
# province-folded buildings that act as a LAND supply source in a state
_SUPPLY_SOURCE_BUILDINGS = ("supply_node", "naval_supply_hub")
# province-folded buildings that *might* allow sea supply (uncertain -> null)
_PORT_BUILDINGS = ("naval_base", "naval_supply_hub")


# ===========================================================================
# 1. front_lines  -- control-adjacency edges
# ===========================================================================
def front_lines(snapshot: Dict[str, Any],
                adj: Optional[Dict[int, List[int]]] = None,
                assets_dir: Optional[str] = None) -> Optional[List[Dict[str, Any]]]:
    """List of control-adjacency edges (province pairs whose controllers differ).

    Returns ``None`` if no province control is present in the snapshot (input
    absent -> no fabrication). Returns ``[]`` if control exists but no two
    adjacent controlled provinces differ (a fully-uniform front, e.g. peace).

    Each edge: ``{a, b, controller_a, controller_b}`` with ``a < b`` (province
    ids). Edges are unordered-unique by the (a,b) pair.
    """
    ctrl = _province_controller_map(snapshot)
    if not ctrl:
        return None
    if adj is None:
        adj = _adj.load_or_build_adjacency(assets_dir=assets_dir)

    edges: List[Dict[str, Any]] = []
    for a, b in _adj.iter_edges(adj):
        ca = ctrl.get(a)
        cb = ctrl.get(b)
        if ca is None or cb is None:
            continue  # an endpoint has no controller in this snapshot
        if ca != cb:
            edges.append({"a": a, "b": b, "controller_a": ca, "controller_b": cb})
    edges.sort(key=lambda e: (e["a"], e["b"]))
    return edges


# ===========================================================================
# 2. encirclement  -- division `encircled` over a same-controller supply graph
# ===========================================================================
#
# MODEL (estimate; documented, not authoritative):
#   A controlled land province belongs to a CONNECTED COMPONENT of provinces
#   that (a) are land-adjacent in the raster and (b) share the same controller.
#   A component is SUPPLIED iff it contains at least one LAND SUPPLY SOURCE for
#   that controller -- a province whose state the controller controls and whose
#   state is the controller's capital OR carries a supply_node / naval_supply_hub
#   building (province-folded to the state in this oracle).
#
#   * component supplied  -> every division in it: encircled = False
#   * component NOT supplied, but it touches a PORT (naval_base / naval_supply_hub
#     the controller controls) -> UNCERTAIN: we cannot see convoys / sea routes /
#     blockade, so we return encircled = None (honest), NOT a confident False/True.
#   * component NOT supplied and NO port -> encircled = True (a land pocket).
#   * a division whose location has no controller, or is absent from the
#     adjacency graph, or whose snapshot lacks province control -> None.
#
# This deliberately uses the controller-folded state buildings (the oracle does
# not split province_buildings back to the exact province). A multi-state
# component is supplied if ANY of its provinces sits in a supplied state the
# controller controls. Same-controller only: allied / subject / military-access
# supply is NOT modelled (would need diplomacy graph + access) -> the estimate
# can OVER-flag a pocket that is in fact allied-supplied. Stated, not hidden.

def _supply_source_provinces(snapshot: Dict[str, Any],
                             assets_dir: Optional[str] = None
                             ) -> Tuple[Dict[int, str], Dict[int, str]]:
    """Return (land_source_prov->controller, port_prov->controller) maps.

    A province is a LAND supply source for ``T`` if its state is controlled by
    ``T`` and is T's capital or has a supply_node / naval_supply_hub. A province
    is a PORT (sea-supply *maybe*) if its state is controlled by ``T`` and has a
    naval_base / naval_supply_hub. Provinces are resolved via the static
    province->state map joined to the snapshot's state objects (``assets_dir``
    threads through so adjacency and supply use the SAME map -- Codex M1).
    """
    states = snapshot.get("states") or {}
    p2s = _province_to_state(snapshot, assets_dir=assets_dir)
    # capital state per controller-tag is not 1:1 with control; capital credit
    # only applies when the controller actually controls its capital state.
    capital_of: Dict[str, int] = {}
    for tag, c in (snapshot.get("countries") or {}).items():
        cs = ((c.get("politics") or {}).get("capital_state"))
        if isinstance(cs, int):
            capital_of[tag] = cs

    # which states are land-supply / port sources, and who controls them
    land_state_ctrl: Dict[int, str] = {}
    port_state_ctrl: Dict[int, str] = {}
    for sid_str, st in states.items():
        try:
            sid = int(sid_str)
        except (TypeError, ValueError):
            continue
        controller = st.get("controller")
        if not controller:
            continue
        pb = st.get("province_buildings") or {}
        is_land_src = (capital_of.get(controller) == sid
                       or any((pb.get(b) or 0) for b in _SUPPLY_SOURCE_BUILDINGS))
        is_port = any((pb.get(b) or 0) for b in _PORT_BUILDINGS)
        if is_land_src:
            land_state_ctrl[sid] = controller
        if is_port:
            port_state_ctrl[sid] = controller

    land_src_prov: Dict[int, str] = {}
    port_prov: Dict[int, str] = {}
    # invert province->state to state->provinces once
    state_provs: Dict[int, List[int]] = defaultdict(list)
    for pid, sid in p2s.items():
        state_provs[sid].append(pid)
    for sid, tag in land_state_ctrl.items():
        for pid in state_provs.get(sid, ()):
            land_src_prov[pid] = tag
    for sid, tag in port_state_ctrl.items():
        for pid in state_provs.get(sid, ()):
            port_prov[pid] = tag
    return land_src_prov, port_prov


def _components(ctrl: Dict[int, str],
                adj: Dict[int, List[int]]) -> Tuple[Dict[int, int], List[Set[int]]]:
    """Connected components over controlled provinces with same controller.

    Returns (prov->component_index, [component_province_sets]).
    """
    comp_of: Dict[int, int] = {}
    comps: List[Set[int]] = []
    for start in ctrl:
        if start in comp_of:
            continue
        cidx = len(comps)
        bucket: Set[int] = set()
        dq = deque([start])
        comp_of[start] = cidx
        my_ctrl = ctrl[start]
        while dq:
            p = dq.popleft()
            bucket.add(p)
            for nb in adj.get(p, ()):  # raster land neighbours
                if nb in comp_of:
                    continue
                if ctrl.get(nb) == my_ctrl:  # same-controller bridge
                    comp_of[nb] = cidx
                    dq.append(nb)
        comps.append(bucket)
    return comp_of, comps


def encirclement(snapshot: Dict[str, Any],
                 adj: Optional[Dict[int, List[int]]] = None,
                 assets_dir: Optional[str] = None
                 ) -> Optional[Dict[int, Optional[bool]]]:
    """Per-province encircled estimate ``{province_id: True/False/None}``.

    Keyed by province id (a division reads its ``location_province``). Returns
    ``None`` if the snapshot carries no province control at all. Per the model
    above, a province maps to True (land pocket), False (supplied), or None
    (uncertain: unsupplied but port-adjacent component).
    """
    ctrl = _province_controller_map(snapshot)
    if not ctrl:
        return None
    if adj is None:
        adj = _adj.load_or_build_adjacency(assets_dir=assets_dir)

    land_src, port = _supply_source_provinces(snapshot, assets_dir=assets_dir)
    comp_of, comps = _components(ctrl, adj)

    # per component: supplied? port-adjacent?
    comp_supplied = [False] * len(comps)
    comp_has_port = [False] * len(comps)
    for cidx, bucket in enumerate(comps):
        if not bucket:
            continue
        ctag = ctrl[next(iter(bucket))]
        for p in bucket:
            if land_src.get(p) == ctag:
                comp_supplied[cidx] = True
                break  # supplied wins; classification can't change
            if port.get(p) == ctag:
                comp_has_port[cidx] = True

    out: Dict[int, Optional[bool]] = {}
    for pid in ctrl:
        if pid not in adj:
            # province has no land neighbour in the raster (island / off-map /
            # adjacency asset gap). We cannot reason about its component -> None,
            # never a confident "encircled" (Codex C1: do not flag absent-graph
            # singletons True).
            out[pid] = None
            continue
        cidx = comp_of[pid]
        if comp_supplied[cidx]:
            out[pid] = False
        elif comp_has_port[cidx]:
            out[pid] = None       # unsupplied by land but maybe sea -> uncertain
        else:
            out[pid] = True       # land pocket: cut off
    return out


# ===========================================================================
# 3. controlled_industry_share  -- factory share by CONTROLLER
# ===========================================================================
def controlled_industry_share(snapshot: Dict[str, Any]
                              ) -> Optional[Dict[str, Any]]:
    """Share of total factory levels held by each controller.

    Weight per state = industrial_complex + arms_factory + dockyard building
    levels (state.buildings). Attributed to the state's CONTROLLER (who holds
    the industry now), not the owner. Returns:

        {"by_tag": {TAG: fraction}, "by_tag_factories": {TAG: int},
         "total_factory_levels": int, "by_type": {TAG: {civ,mil,dockyard}},
         "uncontrolled_factory_levels": int}

    ``None`` if no state has building data (input absent).
    """
    states = snapshot.get("states") or {}
    by_tag_f: Dict[str, int] = defaultdict(int)
    by_type: Dict[str, Dict[str, int]] = defaultdict(
        lambda: {"civ": 0, "mil": 0, "dockyard": 0})
    total = 0
    uncontrolled = 0
    saw_buildings = False
    for st in states.values():
        b = st.get("buildings") or {}
        if not b:
            continue
        civ = int(b.get("industrial_complex") or 0)
        mil = int(b.get("arms_factory") or 0)
        dock = int(b.get("dockyard") or 0)
        w = civ + mil + dock
        if w == 0:
            continue
        saw_buildings = True
        total += w
        controller = st.get("controller")
        if controller:
            by_tag_f[controller] += w
            bt = by_type[controller]
            bt["civ"] += civ
            bt["mil"] += mil
            bt["dockyard"] += dock
        else:
            uncontrolled += w
    if not saw_buildings or total == 0:
        return None
    by_tag = {t: v / total for t, v in by_tag_f.items()}
    return {
        "by_tag": by_tag,
        "by_tag_factories": dict(by_tag_f),
        "by_type": {t: dict(v) for t, v in by_type.items()},
        "total_factory_levels": total,
        "uncontrolled_factory_levels": uncontrolled,
    }


# ===========================================================================
# 4. ships_by_hull  -- per-country hull census from the OOB
# ===========================================================================
def ships_by_hull(snapshot: Dict[str, Any]) -> Dict[str, Optional[Dict[str, Any]]]:
    """``{TAG: {hull: {count, tonnage}}}`` from order_of_battle fleets.

    ``definition`` is already a hull token (destroyer / submarine / light_cruiser
    / heavy_cruiser / battleship / battle_cruiser / carrier / ...). ``tonnage``
    is ``null`` per hull -- the save does NOT carry displacement and we will not
    fabricate a nominal table here (Codex C). A country present in the OOB but
    with zero ships gets ``{}``; a country never seen in any fleet is absent from
    the result (caller leaves its field null).
    """
    oob = snapshot.get("order_of_battle") or {}
    fleets = oob.get("fleets") or []
    per_tag: Dict[str, Dict[str, Dict[str, Any]]] = defaultdict(dict)
    seen_tags: Set[str] = set()
    for fl in fleets:
        owner = fl.get("owner")
        if not owner:
            continue
        seen_tags.add(owner)
        for tf in (fl.get("task_forces") or []):
            tf_owner = tf.get("owner") or owner  # lend-lease tf can differ
            seen_tags.add(tf_owner)
            for sh in (tf.get("ships") or []):
                hull = sh.get("definition")
                if not hull:
                    continue
                bucket = per_tag[tf_owner].setdefault(
                    hull, {"count": 0, "tonnage": None})
                bucket["count"] += 1
    out: Dict[str, Optional[Dict[str, Any]]] = {}
    for tag in seen_tags:
        out[tag] = per_tag.get(tag, {})
    return out


# ===========================================================================
# 5. casualties  -- per-country war losses from diplomacy_world.wars
# ===========================================================================
def casualties(snapshot: Dict[str, Any]) -> Dict[str, Dict[str, Any]]:
    """``{TAG: {total, killed, captured, war_count}}`` from world wars.

    ``total`` = sum of this tag's casualties across every war it belongs to
    (a_casualties when it is side a, b_casualties when side b). Engine "war
    casualties" lump killed+captured+(equipment-loss-derived) together, so we
    expose them as ``total`` and keep ``killed``/``captured`` NULL -- the save
    does not split them (Codex 5). Returns ``{}`` if no war data is present.
    """
    dw = snapshot.get("diplomacy_world") or {}
    wars = dw.get("wars") or {}
    tot: Dict[str, int] = defaultdict(int)
    wc: Dict[str, int] = defaultdict(int)
    for w in wars.values():
        a = w.get("a")
        b = w.get("b")
        ac = w.get("a_casualties")
        bc = w.get("b_casualties")
        if a and isinstance(ac, (int, float)):
            tot[a] += int(ac)
            wc[a] += 1
        if b and isinstance(bc, (int, float)):
            tot[b] += int(bc)
            wc[b] += 1
    out: Dict[str, Dict[str, Any]] = {}
    for tag in set(tot) | set(wc):
        out[tag] = {
            "total": tot.get(tag, 0),
            "killed": None,      # not split by the save -> honest null
            "captured": None,
            "war_count": wc.get(tag, 0),
        }
    return out


# ===========================================================================
# 6. manpower deployed proxy  -- sum of a tag's divisions' strength
# ===========================================================================
def deployed_manpower(snapshot: Dict[str, Any]) -> Dict[str, Dict[str, Any]]:
    """``{TAG: {deployed_strength_proxy, division_count}}``.

    Sums each tag's divisions' ``strength`` (a manpower-scaled save figure, NOT
    a 0..1 fraction in this oracle -- mean ~164, range 6..625). We do NOT claim
    this is a people-count; it is exposed as ``deployed_strength_proxy`` and the
    schema's ``manpower.deployed`` (true people) stays null unless a proven scale
    exists (Codex 6). ``None`` strengths are skipped.
    """
    divs = snapshot.get("divisions") or []
    s: Dict[str, float] = defaultdict(float)
    n: Dict[str, int] = defaultdict(int)
    for d in divs:
        owner = d.get("owner")
        if not owner:
            continue
        n[owner] += 1
        st = d.get("strength")
        if isinstance(st, (int, float)):
            s[owner] += float(st)
    out: Dict[str, Dict[str, Any]] = {}
    for tag in set(s) | set(n):
        out[tag] = {
            "deployed_strength_proxy": round(s.get(tag, 0.0), 3),
            "division_count": n.get(tag, 0),
        }
    return out


# ===========================================================================
# 7. gross_ic_by_equipment  -- TWO-snapshot delta (else null)
# ===========================================================================
def gross_ic_by_equipment(prev_snapshot: Optional[Dict[str, Any]],
                          curr_snapshot: Dict[str, Any]
                          ) -> Dict[str, Optional[Dict[str, float]]]:
    """Per-country ``{archetype: produced_delta}`` BETWEEN two snapshots.

    Returns ``{TAG: None}`` for every country when ``prev_snapshot`` is absent
    -- a single snapshot cannot yield a production-over-interval figure, and the
    per-line ``produced`` accumulator is NOT a reliable lifetime cumulative
    (it is per-run and resets per finished unit), so we will NOT fabricate a
    delta from one snapshot (Codex 7).

    With two snapshots we still only emit a delta when the per-archetype
    accumulator is plausibly MONOTONE (curr >= prev); a negative delta (a line
    reset / line removed) is dropped for that archetype rather than reported as
    negative output. This stays an APPROXIMATION and is gated, never guessed.
    """
    countries = curr_snapshot.get("countries") or {}
    if prev_snapshot is None:
        return {tag: None for tag in countries}

    def _archsum(snap: Dict[str, Any]) -> Dict[str, Dict[str, float]]:
        res: Dict[str, Dict[str, float]] = {}
        for tag, c in (snap.get("countries") or {}).items():
            lines = ((c.get("economy") or {}).get("_production_lines")) or []
            acc: Dict[str, float] = defaultdict(float)
            for ln in lines:
                arch = ln.get("archetype")
                prod = ln.get("produced")
                if arch and isinstance(prod, (int, float)):
                    acc[arch] += float(prod)
            res[tag] = dict(acc)
        return res

    prev = _archsum(prev_snapshot)
    curr = _archsum(curr_snapshot)
    prev_countries = prev_snapshot.get("countries") or {}
    out: Dict[str, Optional[Dict[str, float]]] = {}
    for tag in countries:
        # A tag absent from the prior snapshot has NO baseline -> we cannot
        # derive a delta (treating absent baseline as 0 would fabricate the
        # whole `produced` as new output -- Codex C4). Leave null.
        if tag not in prev_countries:
            out[tag] = None
            continue
        pc = curr.get(tag, {})
        pp = prev.get(tag, {})
        deltas: Dict[str, float] = {}
        for arch, cv in pc.items():
            # only archetypes with a KNOWN prior value yield a delta; a brand-new
            # line (absent from prev) has no baseline -> skip, do not assume 0.
            if arch not in pp:
                continue
            d = cv - pp[arch]
            if d > 0:               # monotone-only; drop resets/negatives
                deltas[arch] = round(d, 5)
        out[tag] = deltas or None
    return out


# ===========================================================================
# helpers
# ===========================================================================
def _province_controller_map(snapshot: Dict[str, Any]) -> Dict[int, str]:
    """``{province_id:int -> controller TAG}`` from the snapshot provinces.

    Prefers ``map.control_by_province`` (already a flat mirror); falls back to
    walking ``provinces[*].controller``. Empty if neither carries control.
    """
    out: Dict[int, str] = {}
    m = (snapshot.get("map") or {}).get("control_by_province")
    if isinstance(m, dict) and m:
        for k, v in m.items():
            if v:
                try:
                    out[int(k)] = v
                except (TypeError, ValueError):
                    pass
        if out:
            return out
    for k, prv in (snapshot.get("provinces") or {}).items():
        c = prv.get("controller")
        if c:
            try:
                out[int(k)] = c
            except (TypeError, ValueError):
                pass
    return out


_P2S_CACHE: Dict[str, Dict[int, int]] = {}


def _static_province_to_state(assets_dir: Optional[str] = None) -> Dict[int, int]:
    """The static ``province_to_state.json`` asset (cached per assets_dir)."""
    a = assets_dir or _adj.default_assets_dir()
    if a not in _P2S_CACHE:
        import json
        import os
        path = os.path.join(a, "province_to_state.json")
        with open(path, "r", encoding="utf-8") as fh:
            raw = json.load(fh)
        _P2S_CACHE[a] = {int(k): int(v) for k, v in raw.items()}
    return _P2S_CACHE[a]


def _province_to_state(snapshot: Dict[str, Any],
                       assets_dir: Optional[str] = None) -> Dict[int, int]:
    """``{province_id -> state_id}``.

    Starts from the STATIC asset (authoritative, complete) and OVERLAYS any
    inline snapshot ``state_id`` on top. This avoids treating a PARTIAL inline
    map as complete (Codex M2): a province with no inline ``state_id`` still
    resolves via the static map, so supply/port evidence is never lost.
    """
    out: Dict[int, int] = dict(_static_province_to_state(assets_dir))
    for k, p in (snapshot.get("provinces") or {}).items():
        sid = p.get("state_id")
        if sid is None:
            continue
        try:
            out[int(k)] = int(sid)
        except (TypeError, ValueError):
            pass
    return out
