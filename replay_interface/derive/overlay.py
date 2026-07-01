"""Overlay the derived layers onto a snapshot.

``derive_layers(snapshot, prev_snapshot=None)`` returns a DEEP COPY of the
snapshot with the D-source fields filled in (the original is never mutated, so a
caller importing the live snapshot is unaffected). It writes:

  * ``map.front_lines``                 (schema field; D)
  * ``map.controlled_industry_share``   (schema field; D)
  * ``divisions[*].encircled``          (schema field; D estimate)
  * ``countries[*].military.ships_by_hull``   (schema field; tonnage stays null)
  * ``countries[*].military.casualties``      (schema field; total filled, killed/captured null)
  * ``countries[*].military.manpower.deployed_strength_proxy``  (proxy; deployed stays null)
  * ``countries[*].military.division_count``  (schema field; D)
  * ``countries[*].economy.gross_ic_by_equipment``  (only with a 2-snapshot series; else null)

It also records what it filled under ``snapshot['_derived']`` (provenance for the
derive pass) and appends ``"derived_layers"`` to ``provenance.notes``.
"""

from __future__ import annotations

import copy
from typing import Any, Dict, List, Optional

from . import layers as _layers


def _as_int(v: Any) -> Optional[int]:
    """Coerce a province id (int or str) to int; None on failure."""
    if isinstance(v, int):
        return v
    try:
        return int(v)
    except (TypeError, ValueError):
        return None


def derive_layers(snapshot: Dict[str, Any],
                  prev_snapshot: Optional[Dict[str, Any]] = None,
                  adj: Optional[Dict[int, List[int]]] = None,
                  assets_dir: Optional[str] = None) -> Dict[str, Any]:
    """Return a copy of ``snapshot`` with derived layers overlaid (no mutation)."""
    snap = copy.deepcopy(snapshot)
    filled: Dict[str, Any] = {}

    # The overlay is IDEMPOTENT: every D field is written to its honest value on
    # every run -- including back to null when inputs are absent -- so a stale
    # non-null value from a prior derive can never survive a recompute (Codex
    # C2/C3). A `null` input therefore yields `null`/`None`, never `[]` or a kept
    # old value.

    # --- map.front_lines -----------------------------------------------------
    fl = _layers.front_lines(snap, adj=adj, assets_dir=assets_dir)
    snap.setdefault("map", {})["front_lines"] = fl  # None when no control input
    filled["front_lines"] = (len(fl) if fl is not None else None)

    # --- map.controlled_industry_share --------------------------------------
    cis = _layers.controlled_industry_share(snap)
    snap["map"]["controlled_industry_share"] = (
        cis["by_tag"] if cis else None)
    # Keep the richer breakdown alongside (non-schema, useful for native diagnostics).
    snap["map"]["controlled_industry_detail"] = cis
    filled["controlled_industry_share"] = (len(cis["by_tag"]) if cis else None)

    # --- divisions[*].encircled ---------------------------------------------
    enc = _layers.encirclement(snap, adj=adj, assets_dir=assets_dir)
    n_true = n_false = n_none = 0
    for d in snap.get("divisions", []):
        pid = _as_int(d.get("location_province"))  # int-normalize (Codex M3)
        val = enc.get(pid) if (enc is not None and pid is not None) else None
        d["encircled"] = val          # always written (null when uncomputable)
        if val is True:
            n_true += 1
        elif val is False:
            n_false += 1
        else:
            n_none += 1
    filled["encircled"] = {"true": n_true, "false": n_false,
                           "uncertain_or_unknown": n_none}

    # --- ships_by_hull (always overwritten; null when tag never in a fleet) --
    sbh = _layers.ships_by_hull(snap)
    n_hull = 0
    for tag, c in snap.get("countries", {}).items():
        mil = c.setdefault("military", {})
        val = sbh.get(tag)            # dict (maybe empty) or None
        mil["ships_by_hull"] = val if val else None
        if val:
            n_hull += 1
    filled["ships_by_hull"] = n_hull

    # --- casualties (always overwritten; null when no war data for the tag) --
    cas = _layers.casualties(snap)
    n_cas = 0
    for tag, c in snap.get("countries", {}).items():
        mil = c.setdefault("military", {})
        if tag in cas:
            mil["casualties"] = cas[tag]
            n_cas += 1
        else:
            mil["casualties"] = None
    filled["casualties"] = n_cas

    # --- deployed manpower proxy + division_count ---------------------------
    dep = _layers.deployed_manpower(snap)
    n_dep = 0
    for tag, c in snap.get("countries", {}).items():
        mil = c.setdefault("military", {})
        mp = mil.get("manpower")
        if not isinstance(mp, dict):
            mp = {}
            mil["manpower"] = mp
        if tag in dep:
            mp["deployed_strength_proxy"] = dep[tag]["deployed_strength_proxy"]
            mil["division_count"] = dep[tag]["division_count"]
            n_dep += 1
        else:
            mp["deployed_strength_proxy"] = None
            mil["division_count"] = None
        # the real people-count `deployed` is never fabricated; keep/init null.
        mp.setdefault("deployed", None)
    filled["deployed_manpower"] = n_dep

    # --- gross_ic_by_equipment (2-snapshot delta or null) -------------------
    gic = _layers.gross_ic_by_equipment(prev_snapshot, snap)
    n_gic = 0
    for tag, c in snap.get("countries", {}).items():
        val = gic.get(tag)            # dict or None
        c.setdefault("economy", {})["gross_ic_by_equipment"] = val
        if val:
            n_gic += 1
    filled["gross_ic_by_equipment"] = (n_gic if prev_snapshot is not None else None)

    # provenance of the derive pass
    snap["_derived"] = {
        "layers": filled,
        "model": {
            "front_lines": "control-adjacency edges; raster 4-neighbour land adjacency",
            "encircled": ("same-controller supply-graph ESTIMATE; True=land pocket, "
                          "False=supplied, None=unsupplied-but-port-adjacent (uncertain) "
                          "or unknown; allied/access supply not modelled"),
            "controlled_industry_share": "factory-level share by CONTROLLER (civ+mil+dockyard)",
            "ships_by_hull": "OOB hull census; tonnage null (save lacks displacement)",
            "casualties": "war casualties summed per tag as total; killed/captured null",
            "manpower": "deployed_strength_proxy = sum of division strength (proxy, not people)",
            "gross_ic_by_equipment": "two-snapshot monotone produced-delta; null on single snapshot",
        },
    }
    prov = snap.setdefault("provenance", {})
    notes = prov.setdefault("notes", [])
    if "derived_layers overlaid (front_lines, encircled, industry_share, ships_by_hull, casualties, deployed_proxy)" not in notes:
        notes.append("derived_layers overlaid (front_lines, encircled, industry_share, "
                     "ships_by_hull, casualties, deployed_proxy)")
    return snap
