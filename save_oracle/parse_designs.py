#!/usr/bin/env python3
"""the game save -> DESIGNS oracle (``designs.json``).

Deep-layer extension of the save oracle covering the three "designer" systems
that the live memory reader will be validated against:

  1. DIVISION TEMPLATES (full) — the regiment grid, support companies and
     ``regimental_support``, plus role / names-group / obsolete / priority /
     division_cap / override_model. (NOTE: the engine does NOT serialise the
     template's *derived* combat stats — soft/hard attack, defense, org, HP,
     combat width are computed at runtime and are absent from the save. See the
     ``notes`` block in the output meta.)
  2. EQUIPMENT VARIANTS / DESIGNS — land/ship/plane designer output. Two places
     in the save expose this, and BOTH are captured:
       (a) top-level ``equipments={ <name>={ id={id type=70} modules={slot=mod}
           upgrades={ upgrades={ {"name" lvl} } } design_team={id type=79}
           name version parent_id creator obsolete ... } }`` — the per-(id,type)
           variant CATALOG that production lines / divisions join against.
       (b) per country ``production.scheduled_equipment_variants={ <archetype>={
           name="..." modules={slot=mod} upgrades={ name=lvl } industrial_org... } }``
           — the named designer PRESETS the player saved.
  3. MIOs (military industrial organisations) — per country
     ``production.industrial_organisations={ <key>={ id={id type=79} name icon
     research_bonus task_capacity funds size points allowed_policies={..} } }``
     plus ``production.policies={ <policy>={cost cooldown} }`` (the active /
     queued MIO policies). ``size`` = the org level, ``points`` = progress to the
     next level. A design's ``design_team={id type=79}`` joins to an MIO by the
     full (id,type) key (MIO ids are GLOBALLY unique in the save — verified).

Joins use the full Clausewitz object key ``(id, type)`` (not bare id). Equipment
variant ids (type 70) and MIO ids (type 79) are both globally unique and dense
in a real melt, so global indexes are correct for cross-country joins.

stdlib only. Reuses the shared tolerant tokenizer via ``deep_common`` (which
re-exports ``parse_save``'s primitives). Adds nothing to the tokenizer and edits
no other module.

Usage:
    python parse_designs.py [save_or_melted_path]
    python parse_designs.py --rakaly /path/to/rakaly.exe --out-dir <dir> [save]
"""

from __future__ import annotations

import deep_common as dc
from deep_common import (
    LBRACE, RBRACE, atom_value, iter_block_entries, _skip_block_body,
    to_int, to_float, read_id_block,
)


# ---------------------------------------------------------------------------
# Generic small readers (built only on shared primitives)
# ---------------------------------------------------------------------------

def _read_flat_scalar_map(tz):
    """Read a flat ``{ k=v k=v ... }`` block -> dict (LBRACE consumed).

    Nested blocks are skipped; bare atoms ignored. Repeated keys keep the LAST
    value. Values are returned as raw strings (caller coerces if needed).
    """
    out = {}
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            _skip_block_body(tz)
            continue
        if opener is None:
            continue
        v = atom_value(opener)
        if v is not None:
            out[key] = v
    return out


def _read_modules(tz):
    """Read ``modules={ <slot>=<module> ... }`` -> {slot: module} (LBRACE eaten).

    Empty ``modules={ }`` yields {}. Same shape for tanks, ships and planes.
    """
    return _read_flat_scalar_map(tz)


def _read_upgrades_flat(tz):
    """Read FLAT ``upgrades={ name=level ... }`` -> {name: int} (LBRACE eaten).

    Used by ``scheduled_equipment_variants``.
    """
    out = {}
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            _skip_block_body(tz)
            continue
        if opener is None:
            continue
        v = atom_value(opener)
        if v is not None:
            iv = to_int(v)
            out[key] = iv if iv is not None else v
    return out


def _read_upgrades_nested(tz):
    """Read NESTED ``upgrades={ upgrades={ {"name" lvl} {"name" lvl} } }``.

    Returns {name: int}. The inner list elements are anonymous 2-atom blocks
    ``{ "upgrade_name" level }``. LBRACE of the OUTER ``upgrades`` already eaten.
    """
    out = {}
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "upgrades":
                # inner anonymous list of {name level} pairs
                _read_upgrade_pair_list(tz, out)
            else:
                _skip_block_body(tz)
        # scalars/None ignored
    return out


def _read_upgrade_pair_list(tz, out):
    """Inside ``{ {"name" lvl} {"name" lvl} ... }`` (LBRACE eaten) -> fill out.

    iter_block_entries skips anonymous nested blocks wholesale, so we hand-parse
    the brace stream to capture each ``{name level}`` pair. Depth-aware so a
    malformed pair (missing inner ``}``) cannot make us mistake the list's close
    for a pair's close and over-run into sibling blocks.
    """
    while True:
        tok = tz.next()
        if tok is None or tok is RBRACE:
            return
        if tok is LBRACE:
            # one pair: atom(name) atom(level); guard against malformed nesting
            # by counting brace depth rather than trusting the first RBRACE.
            name = None
            level = None
            depth = 1
            while depth > 0:
                t2 = tz.next()
                if t2 is None:
                    return  # truncated; stop gracefully
                if t2 is LBRACE:
                    depth += 1
                    continue
                if t2 is RBRACE:
                    depth -= 1
                    continue
                if depth != 1:
                    continue  # ignore atoms inside an unexpected nested block
                v = atom_value(t2)
                if v is None:
                    continue
                if name is None:
                    name = v
                elif level is None:
                    level = to_int(v)
            if name is not None:
                out[name] = level
        # bare atoms / '=' between pairs ignored


def _read_grid_blocks(tz):
    """Read ``{ <type>={x=N y=N} <type>={x y} ... }`` -> list of dicts.

    The block KEY (battalion / company type) CAN REPEAT, so this returns an
    ordered LIST of ``{"type","x","y"}`` rather than a dict (a dict would drop
    duplicate columns). LBRACE already consumed.
    """
    out = []
    for key, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        x = y = None
        for k, op in iter_block_entries(tz):
            if op is LBRACE:
                _skip_block_body(tz)
                continue
            v = atom_value(op)
            if v is None:
                continue
            if k == "x":
                x = to_int(v)
            elif k == "y":
                y = to_int(v)
        out.append({"type": key, "x": x, "y": y})
    return out


# ---------------------------------------------------------------------------
# 1) Division templates
# ---------------------------------------------------------------------------

def _parse_division_templates(tz, templates):
    """division_templates={ division_template={ ... } ... } -> fill list."""
    for key, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        if key != "division_template":
            _skip_block_body(tz)
            continue
        rec = {
            "id": None, "type": None,
            "name": None, "localization_key": None,
            "country": None, "original_tag": None,
            "role": None, "division_names_group": None,
            "priority": None, "division_cap": None,
            "obsolete": False, "is_locked": False,
            "is_army_hq": False, "override_model": None,
            "regiments": [], "support": [], "regimental_support": [],
        }
        for k, op in iter_block_entries(tz):
            if op is LBRACE:
                if k == "id":
                    rec["id"], rec["type"] = read_id_block(tz)
                elif k == "regiments":
                    rec["regiments"] = _read_grid_blocks(tz)
                elif k == "support":
                    rec["support"] = _read_grid_blocks(tz)
                elif k == "regimental_support":
                    rec["regimental_support"] = _read_grid_blocks(tz)
                else:
                    _skip_block_body(tz)
                continue
            v = atom_value(op)
            if v is None:
                continue
            if k == "name":
                rec["name"] = v
            elif k == "localization_key":
                rec["localization_key"] = v
            elif k == "country":
                rec["country"] = v
            elif k == "original_tag":
                rec["original_tag"] = v
            elif k == "role":
                rec["role"] = v
            elif k == "division_names_group":
                rec["division_names_group"] = v
            elif k == "priority":
                rec["priority"] = to_int(v)
            elif k == "division_cap":
                rec["division_cap"] = to_int(v)
            elif k == "obsolete":
                rec["obsolete"] = (v == "yes")
            elif k == "is_locked":
                rec["is_locked"] = (v == "yes")
            elif k == "is_army_hq":
                rec["is_army_hq"] = (v == "yes")
            elif k == "override_model":
                rec["override_model"] = v
        # Display name: prefer name=, else loc key (army-HQ templates carry no
        # name= but a localization_key).
        rec["display_name"] = rec["name"] or rec["localization_key"]
        templates.append(rec)


# ---------------------------------------------------------------------------
# 2a) Top-level equipment variant CATALOG (equipments={})
# ---------------------------------------------------------------------------

def _parse_equipments_catalog(tz, catalog):
    """equipments={ <name>={ id={id type=70} ... modules upgrades ... } } -> dict.

    catalog[(id,type)] = {archetype, name(custom or None), version, parent_id,
    creator, origin, obsolete, is_frame, max_version, manpower, modules,
    upgrades, design_team(id,type or None), design_team_bonus}.
    """
    for arch_name, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        rec = {
            "archetype": arch_name,
            "name": None, "version": None, "parent_id": None,
            "creator": None, "origin": None,
            "obsolete": False, "is_frame": None,
            "max_version": None, "manpower": None,
            "modules": {}, "upgrades": {},
            "design_team": None, "design_team_bonus": None,
        }
        eid = etype = None
        for k, op in iter_block_entries(tz):
            if op is LBRACE:
                if k == "id":
                    eid, etype = read_id_block(tz)
                elif k == "parent_id":
                    rec["parent_id"] = list(read_id_block(tz))
                elif k == "modules":
                    rec["modules"] = _read_modules(tz)
                elif k == "upgrades":
                    rec["upgrades"] = _read_upgrades_nested(tz)
                elif k == "design_team":
                    rec["design_team"] = list(read_id_block(tz))
                elif k == "design_team_bonus":
                    rec["design_team_bonus"] = _read_flat_scalar_map(tz)
                else:
                    _skip_block_body(tz)
                continue
            v = atom_value(op)
            if v is None:
                continue
            if k == "name":
                rec["name"] = v
            elif k == "version":
                rec["version"] = to_int(v)
            elif k == "creator":
                rec["creator"] = v
            elif k == "origin":
                rec["origin"] = v
            elif k == "obsolete":
                rec["obsolete"] = (v == "yes")
            elif k == "is_frame":
                rec["is_frame"] = (v == "yes")
            elif k == "max_version":
                rec["max_version"] = to_int(v)
            elif k == "manpower":
                rec["manpower"] = to_int(v)
        if eid is not None:
            rec["id"], rec["type"] = eid, etype
            catalog[(eid, etype)] = rec


# ---------------------------------------------------------------------------
# 2b/3) Per-country production: MIOs + scheduled variants + policies
# ---------------------------------------------------------------------------

def _parse_mio(tz):
    """Read one MIO definition block (LBRACE eaten) -> dict.

    Captures (per Codex review of the 1.19 MIO system):
      - ``size`` = the org LEVEL; ``funds`` = accumulated development currency
        (progress toward the next level); ``points`` = a secondary counter (left
        as-is, not interpreted as level progress).
      - ``unlocked={ trait="..." }`` (REPEATS) = the traits actually TAKEN. This
        is distinct from ``allowed_policies`` (mere eligibility) and from the
        ``upgrades=yes`` scalar flag.
      - ``history.equipment={id type}`` = the assigned equipment variant the org
        is developing/producing (the authoritative forward MIO->variant link).
    """
    rec = {
        "id": None, "type": None,
        "name": None, "icon": None,
        "research_bonus": None, "task_capacity": None,
        "funds": None, "size": None, "points": None,
        "allowed_policies": [], "unlocked_traits": [],
        "assigned_equipment": None,  # (id,type) from history.equipment
    }
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "id":
                rec["id"], rec["type"] = read_id_block(tz)
            elif k == "allowed_policies":
                rec["allowed_policies"] = dc.read_scalar_list(tz)
            elif k == "unlocked":
                # unlocked={ trait="..." } — one taken trait; block repeats.
                t = _read_scalar_field(tz, "trait")
                if t is not None:
                    rec["unlocked_traits"].append(t)
            elif k == "history":
                rec["assigned_equipment"] = _read_history_equipment(tz)
            else:
                _skip_block_body(tz)
            continue
        v = atom_value(op)
        if v is None:
            continue
        if k == "name":
            rec["name"] = v
        elif k == "icon":
            rec["icon"] = v
        elif k == "research_bonus":
            rec["research_bonus"] = to_float(v)
        elif k == "task_capacity":
            rec["task_capacity"] = to_int(v)
        elif k == "funds":
            rec["funds"] = to_float(v)
        elif k == "size":
            rec["size"] = to_int(v)
        elif k == "points":
            rec["points"] = to_float(v)
    return rec


def _read_scalar_field(tz, field):
    """Inside a block whose '{' was just consumed, return value of `field`."""
    out = None
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            _skip_block_body(tz)
        elif key == field:
            out = atom_value(opener)
    return out


def _read_history_equipment(tz):
    """history={ equipment={id type} data={...} } -> [id,type] (LBRACE eaten)."""
    eq = None
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "equipment":
                eid, etype = read_id_block(tz)
                if eid is not None:
                    eq = [eid, etype]
            else:
                _skip_block_body(tz)
    return eq


def _parse_industrial_organisations(tz, mios):
    """industrial_organisations={ <key>={ id={id type=79} ... } } -> append."""
    for key, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        rec = _parse_mio(tz)
        rec["key"] = key
        mios.append(rec)


def _parse_policies(tz, policies):
    """policies={ <policy_name>={ cost cooldown } } -> {name:{cost,cooldown}}."""
    for key, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        info = {"cost": None, "cooldown": None}
        for k, op in iter_block_entries(tz):
            if op is LBRACE:
                _skip_block_body(tz)
                continue
            v = atom_value(op)
            if v is None:
                continue
            if k == "cost":
                info["cost"] = to_float(v)
            elif k == "cooldown":
                info["cooldown"] = to_int(v)
        policies[key] = info


def _parse_scheduled_variant(tz):
    """One ``scheduled_equipment_variants.<archetype>={ ... }`` (LBRACE eaten)."""
    rec = {
        "name": None, "icon": None, "obsolete": False,
        "modules": {}, "upgrades": {}, "industrial_organisation": None,
    }
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "modules":
                rec["modules"] = _read_modules(tz)
            elif k == "upgrades":
                rec["upgrades"] = _read_upgrades_flat(tz)
            elif k == "industrial_organisation":
                rec["industrial_organisation"] = list(read_id_block(tz))
            else:
                _skip_block_body(tz)
            continue
        v = atom_value(op)
        if v is None:
            continue
        if k == "name":
            rec["name"] = v
        elif k == "icon":
            rec["icon"] = v
        elif k == "obsolete":
            rec["obsolete"] = (v == "yes")
    return rec


def _parse_scheduled_variants(tz, presets):
    """scheduled_equipment_variants={ <archetype>={...} } -> list of dicts."""
    for key, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        rec = _parse_scheduled_variant(tz)
        rec["archetype"] = key
        presets.append(rec)


def _parse_production_block(tz, rec):
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "industrial_organisations":
                _parse_industrial_organisations(tz, rec["mios"])
            elif key == "scheduled_equipment_variants":
                _parse_scheduled_variants(tz, rec["presets"])
            elif key == "policies":
                _parse_policies(tz, rec["policies"])
            else:
                _skip_block_body(tz)
        # scalars ignored


def _parse_countries(tz, by_tag):
    for tag, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        rec = {"mios": [], "presets": [], "policies": {}}
        for key, op in iter_block_entries(tz):
            if op is LBRACE:
                if key == "production":
                    _parse_production_block(tz, rec)
                else:
                    _skip_block_body(tz)
            # scalar country fields ignored
        if rec["mios"] or rec["presets"] or rec["policies"]:
            by_tag[tag] = rec


# ---------------------------------------------------------------------------
# Top-level streaming pass
# ---------------------------------------------------------------------------

def parse_designs_text(text):
    templates = []          # list of division-template dicts
    catalog = {}            # (id,type) -> equipment-variant dict
    by_tag = {}             # TAG -> {mios, presets, policies}
    meta = dc.top_level_pass(
        text,
        {
            "division_templates":
                lambda tz: _parse_division_templates(tz, templates),
            "equipments":
                lambda tz: _parse_equipments_catalog(tz, catalog),
            "countries":
                lambda tz: _parse_countries(tz, by_tag),
        },
        scalar_keys=("date", "version", "player"),
    )
    return templates, catalog, by_tag, meta


# ---------------------------------------------------------------------------
# Output build + join + validation
# ---------------------------------------------------------------------------

def _classify_archetype(arch):
    """Rough land/ship/plane class from an archetype name (designer family).

    Modular designer families are identified by their hull / airframe / chassis
    marker. Legacy non-modular equipment (infantry/artillery/anti_tank/anti_air/
    convoy/etc.) is ``land_other``. NOTE: ``anti_tank_equipment_*`` contains the
    substring "tank_" but is a GUN, not a vehicle, so the chassis check (the only
    reliable vehicle marker) must NOT fall back to a bare "tank_" match.
    """
    if arch is None:
        return "unknown"
    a = arch.lower()
    if "ship_hull" in a or a.startswith("ship_") or "_ship_" in a:
        return "ship"
    if "airframe" in a or "_plane_" in a:
        return "plane"
    # Vehicles are modular tank/SPG chassis. Plain "*_equipment_*" entries (incl.
    # anti_tank_equipment / anti_air_equipment) are legacy guns -> land_other.
    if "chassis" in a:
        return "land_vehicle"
    return "land_other"


def build(text, source, was_melted):
    templates, catalog, by_tag, meta = parse_designs_text(text)

    # --- Global MIO index (id,type=79) -> {tag, key, name, size}. MIO ids are
    #     globally unique, so a design's design_team / industrial_organisation
    #     handle resolves here regardless of which country owns it. -----------
    mio_index = {}
    for tag, rec in by_tag.items():
        for m in rec["mios"]:
            if m["id"] is not None:
                mio_index[(m["id"], m["type"])] = {
                    "tag": tag, "key": m["key"], "name": m["name"],
                    "size": m["size"],
                }

    # --- Equipment variant CATALOG output (keyed "id:type"), with derived
    #     family class + resolved design-team MIO. --------------------------
    variants_out = {}
    n_custom = 0
    n_with_modules = 0
    n_design_team = 0
    by_family = {}
    for (eid, etype), v in catalog.items():
        dt = v["design_team"]
        dt_resolved = mio_index.get(tuple(dt)) if dt else None
        fam = _classify_archetype(v["archetype"])
        by_family[fam] = by_family.get(fam, 0) + 1
        if v["name"]:
            n_custom += 1
        if v["modules"]:
            n_with_modules += 1
        if dt:
            n_design_team += 1
        variants_out[f"{eid}:{etype}"] = {
            "id": eid, "type": etype,
            "archetype": v["archetype"],
            "family": fam,
            "name": v["name"],
            "version": v["version"],
            "parent_id": v["parent_id"],
            "creator": v["creator"],
            "origin": v["origin"],
            "obsolete": v["obsolete"],
            "is_frame": v["is_frame"],
            "max_version": v["max_version"],
            "manpower": v["manpower"],
            "modules": v["modules"],
            "upgrades": v["upgrades"],
            "design_team": dt,
            "design_team_mio": dt_resolved,
            "design_team_bonus": v["design_team_bonus"],
        }

    # --- Per-country block: MIOs + scheduled presets + policies ------------
    countries_out = {}
    n_mios = n_presets = n_policies = 0
    n_preset_with_mio = n_preset_mio_resolved = 0
    n_mio_traits = n_mio_assigned = n_mio_developed = 0
    for tag, rec in sorted(by_tag.items()):
        mios = []
        for m in rec["mios"]:
            ae = m["assigned_equipment"]
            ae_info = catalog.get(tuple(ae)) if ae else None
            mios.append({
                "key": m["key"], "id": m["id"], "type": m["type"],
                "name": m["name"], "icon": m["icon"],
                "level": m["size"],            # `size` = org level
                "funds": m["funds"],           # development currency (progress)
                "points": m["points"],         # secondary counter (uninterpreted)
                "research_bonus": m["research_bonus"],
                "task_capacity": m["task_capacity"],
                "unlocked_traits": m["unlocked_traits"],  # traits TAKEN
                "allowed_policies": m["allowed_policies"],  # eligibility only
                "assigned_equipment": ae,                   # (id,type) or None
                "assigned_equipment_archetype":
                    ae_info["archetype"] if ae_info else None,
            })
        presets = []
        for p in rec["presets"]:
            io = p["industrial_organisation"]
            io_resolved = mio_index.get(tuple(io)) if io else None
            if io:
                n_preset_with_mio += 1
                if io_resolved:
                    n_preset_mio_resolved += 1
            presets.append({
                "archetype": p["archetype"],
                "family": _classify_archetype(p["archetype"]),
                "name": p["name"], "icon": p["icon"],
                "obsolete": p["obsolete"],
                "modules": p["modules"],
                "upgrades": p["upgrades"],
                "industrial_organisation": io,
                "industrial_organisation_mio": io_resolved,
            })
        countries_out[tag] = {
            "mios": mios,
            "scheduled_variants": presets,
            "policies": rec["policies"],
        }
        n_mios += len(mios)
        n_presets += len(presets)
        n_policies += len(rec["policies"])
        for m in mios:
            n_mio_traits += len(m["unlocked_traits"])
            if m["assigned_equipment"]:
                n_mio_assigned += 1
            if m["level"] and m["level"] > 0:
                n_mio_developed += 1

    # --- Division templates output -----------------------------------------
    templates_out = []
    tmpl_by_country = {}
    for t in templates:
        templates_out.append({
            "id": t["id"], "type": t["type"],
            "name": t["display_name"],
            "raw_name": t["name"],
            "localization_key": t["localization_key"],
            "country": t["country"],
            "original_tag": t["original_tag"],
            "role": t["role"],
            "division_names_group": t["division_names_group"],
            "priority": t["priority"],
            "division_cap": t["division_cap"],
            "obsolete": t["obsolete"],
            "is_locked": t["is_locked"],
            "is_army_hq": t["is_army_hq"],
            "override_model": t["override_model"],
            "regiments": t["regiments"],
            "support": t["support"],
            "regimental_support": t["regimental_support"],
            "n_regiments": len(t["regiments"]),
            "n_support": len(t["support"]),
        })
        c = t["country"] or "??"
        tmpl_by_country[c] = tmpl_by_country.get(c, 0) + 1

    # Catalog variant counts per creator (designer activity per country).
    variants_by_creator = {}
    for v in variants_out.values():
        c = v["creator"] or "??"
        variants_by_creator[c] = variants_by_creator.get(c, 0) + 1

    n_tmpl = len(templates_out)
    n_named_tmpl = sum(1 for t in templates_out if t["name"])
    n_with_regiments = sum(1 for t in templates_out if t["n_regiments"] > 0)

    out = {
        "meta": {
            "source_filename": source.name,
            "source_path_redacted": dc.ps._redact(source),
            "was_melted": was_melted,
            "save_date": meta.get("date"),
            "save_version": meta.get("version"),
            "player": meta.get("player"),
            "notes": {
                "division_template_stats": "The save does NOT store a template's "
                "DERIVED combat stats (soft/hard attack, defense, org, HP, combat "
                "width). Those are computed at runtime by the engine from the "
                "regiment + support composition; only the grid layout is here.",
                "variant_locations": "Equipment designs appear in TWO places: the "
                "top-level 'equipments' CATALOG (per-(id,type), what production & "
                "divisions join to) and per-country "
                "'production.scheduled_equipment_variants' (named designer "
                "presets). Both are captured; the catalog 'upgrades' encoding is "
                "nested {\"name\" lvl} pairs, the preset encoding is flat name=lvl.",
                "mio_level": "MIO 'size' = org level. 'funds' = the accumulated "
                "development currency (progress toward the next level); 'points' "
                "is a secondary counter left uninterpreted. MIO ids (type 79) are "
                "globally unique, so design_team / industrial_organisation handles "
                "resolve via a global index.",
                "mio_traits": "'unlocked_traits' = traits the org has actually "
                "TAKEN (from repeated 'unlocked={trait=..}' blocks). "
                "'allowed_policies' is mere eligibility, NOT taken. The country "
                "'policies' block holds cost/cooldown state of MIO policies.",
                "mio_assigned_equipment": "An MIO's 'assigned_equipment' is the "
                "(id,type) equipment variant it is developing, read from its "
                "history.equipment block and joined to the catalog archetype.",
            },
            "stats": {
                "division_templates": n_tmpl,
                "division_templates_named": n_named_tmpl,
                "division_templates_with_regiments": n_with_regiments,
                "equipment_variants_catalog": len(variants_out),
                "equipment_variants_custom_named": n_custom,
                "equipment_variants_with_modules": n_with_modules,
                "equipment_variants_with_design_team": n_design_team,
                "variant_family_breakdown": by_family,
                "countries_with_production": len(countries_out),
                "mios_total": n_mios,
                "mios_developed_level_gt0": n_mio_developed,
                "mio_traits_taken_total": n_mio_traits,
                "mios_with_assigned_equipment": n_mio_assigned,
                "scheduled_variants_total": n_presets,
                "scheduled_variants_with_mio_link": n_preset_with_mio,
                "scheduled_variants_mio_resolved": n_preset_mio_resolved,
                "policies_total": n_policies,
            },
        },
        "division_templates": templates_out,
        "equipment_variants": variants_out,
        "countries": countries_out,
    }

    report = _report(out, tmpl_by_country, variants_by_creator)
    return {"designs.json": out}, report


def _report(out, tmpl_by_country, variants_by_creator):
    st = out["meta"]["stats"]
    L = ["=== DESIGNS ORACLE VALIDATION ==="]
    L.append(f"source        : {out['meta']['source_filename']} "
             f"(melted={out['meta']['was_melted']})")
    L.append(f"save date     : {out['meta']['save_date']}  "
             f"version={out['meta']['save_version']}")
    L.append("")
    L.append(f"DIV TEMPLATES : {st['division_templates']} "
             f"({st['division_templates_named']} named, "
             f"{st['division_templates_with_regiments']} have regiments)")
    top_tc = sorted(tmpl_by_country.items(), key=lambda kv: kv[1],
                    reverse=True)[:6]
    L.append("   top by country: " +
             ", ".join(f"{c}={n}" for c, n in top_tc))

    L.append("")
    L.append(f"EQUIP VARIANTS: {st['equipment_variants_catalog']} in catalog "
             f"({st['equipment_variants_custom_named']} custom-named, "
             f"{st['equipment_variants_with_modules']} have modules, "
             f"{st['equipment_variants_with_design_team']} link a design-team)")
    L.append(f"   family split : {st['variant_family_breakdown']}")
    top_vc = sorted(variants_by_creator.items(), key=lambda kv: kv[1],
                    reverse=True)[:6]
    L.append("   top by creator: " +
             ", ".join(f"{c}={n}" for c, n in top_vc))

    L.append("")
    L.append(f"MIOs          : {st['mios_total']} across "
             f"{st['countries_with_production']} countries "
             f"({st['mios_developed_level_gt0']} developed >L0, "
             f"{st['mio_traits_taken_total']} traits taken, "
             f"{st['mios_with_assigned_equipment']} have assigned equipment)")
    L.append(f"PRESETS       : {st['scheduled_variants_total']} scheduled "
             f"variants ({st['scheduled_variants_with_mio_link']} link a MIO, "
             f"{st['scheduled_variants_mio_resolved']} resolved)")
    L.append(f"POLICIES      : {st['policies_total']} MIO-policy entries")

    # --- One sample of each system -----------------------------------------
    L.append("")
    L.append("sample division template:")
    # Prefer a named combat template with regiments.
    samp_t = next((t for t in out["division_templates"]
                   if t["name"] and t["n_regiments"] >= 2
                   and not t["is_army_hq"]), None) \
        or (out["division_templates"][0] if out["division_templates"] else None)
    if samp_t:
        regs = ", ".join(f"{r['type']}@({r['x']},{r['y']})"
                         for r in samp_t["regiments"][:6])
        sups = ", ".join(r["type"] for r in samp_t["support"][:6])
        L.append(f"   id={samp_t['id']}:{samp_t['type']} "
                 f"name={samp_t['name']!r} country={samp_t['country']} "
                 f"role={samp_t['role']}")
        L.append(f"     regiments[{samp_t['n_regiments']}]: {regs}")
        L.append(f"     support[{samp_t['n_support']}]: {sups}")

    L.append("sample equipment variant (custom-named w/ modules):")
    samp_v = next((v for v in out["equipment_variants"].values()
                   if v["name"] and v["modules"]), None) \
        or next(iter(out["equipment_variants"].values()), None)
    if samp_v:
        L.append(f"   {samp_v['id']}:{samp_v['type']} arch={samp_v['archetype']} "
                 f"family={samp_v['family']} name={samp_v['name']!r} "
                 f"creator={samp_v['creator']}")
        L.append(f"     modules: {samp_v['modules']}")
        L.append(f"     upgrades: {samp_v['upgrades']}  "
                 f"design_team_mio={samp_v['design_team_mio']}")

    L.append("sample MIO:")
    samp_mio = None
    samp_mio_tag = None
    for tag, c in out["countries"].items():
        for m in c["mios"]:
            if m["level"] and m["level"] > 0:
                samp_mio, samp_mio_tag = m, tag
                break
        if samp_mio:
            break
    if samp_mio is None:
        for tag, c in out["countries"].items():
            if c["mios"]:
                samp_mio, samp_mio_tag = c["mios"][0], tag
                break
    if samp_mio:
        L.append(f"   [{samp_mio_tag}] {samp_mio['key']} "
                 f"id={samp_mio['id']}:{samp_mio['type']} "
                 f"name={samp_mio['name']!r} level={samp_mio['level']} "
                 f"funds={samp_mio['funds']} "
                 f"research_bonus={samp_mio['research_bonus']}")
        L.append(f"     traits_taken[{len(samp_mio['unlocked_traits'])}]: "
                 f"{samp_mio['unlocked_traits'][:3]}")
        L.append(f"     assigned_equipment={samp_mio['assigned_equipment']} "
                 f"({samp_mio['assigned_equipment_archetype']})")

    L.append("sample scheduled variant (designer preset):")
    samp_p = None
    samp_p_tag = None
    for tag, c in out["countries"].items():
        for p in c["scheduled_variants"]:
            if p["name"] and (p["modules"] or p["upgrades"]):
                samp_p, samp_p_tag = p, tag
                break
        if samp_p:
            break
    if samp_p:
        L.append(f"   [{samp_p_tag}] arch={samp_p['archetype']} "
                 f"name={samp_p['name']!r} family={samp_p['family']}")
        L.append(f"     modules: {samp_p['modules']}")
        L.append(f"     upgrades: {samp_p['upgrades']}  "
                 f"mio={samp_p['industrial_organisation_mio']}")
    return "\n".join(L)


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------
# A self-contained CLI (mirrors deep_common.run_cli) is used instead of the
# shared one for ONE reason: run_cli writes with ensure_ascii=False, but this
# tool must emit pure-ASCII JSON (non-ASCII designer names like "Panzerjager
# Elefant" / "Konigsberg" are \u-escaped). All other behaviour — save
# discovery, rakaly melt, path redaction — is delegated to the shared helpers
# in parse_save / deep_common (no duplication of the tokenizer or melt logic).

def main(argv=None):
    import argparse
    import json
    from pathlib import Path

    ap = argparse.ArgumentParser(
        description="the game save -> designs.json (templates, variants, MIOs).")
    ap.add_argument("save", nargs="?", default=None,
                    help="Path to a .hoi4 save or melted plaintext. "
                         "If omitted, the most recent candidate is used.")
    ap.add_argument("--rakaly", default=None,
                    help="Path to rakaly executable (for binary saves).")
    ap.add_argument("--out-dir", default=None,
                    help="Output directory (default: <script>/output).")
    args = ap.parse_args(argv)

    script_dir = Path(__file__).resolve().parent
    out_dir = Path(args.out_dir) if args.out_dir else script_dir / "output"
    out_dir.mkdir(parents=True, exist_ok=True)

    if args.save:
        save_path = Path(args.save)
        if not save_path.exists():
            print(f"ERROR: save not found: {save_path}", file=__import__("sys").stderr)
            return 2
    else:
        save_path = dc.ps.find_latest_save()
        if save_path is None:
            print("ERROR: no candidate save found in known locations.",
                  file=__import__("sys").stderr)
            return 2

    print(f"[designs] resolved save : {dc.ps._scrub(str(save_path))}")
    rakaly = dc.ps.find_rakaly(args.rakaly)
    print(f"[designs] rakaly        : "
          f"{dc.ps._scrub(str(rakaly)) if rakaly else 'NOT FOUND'}")

    try:
        text, was_melted = dc.ps.melt_if_needed(save_path, rakaly, out_dir)
    except RuntimeError as exc:
        print(f"ERROR: {exc}", file=__import__("sys").stderr)
        return 3

    print(f"[designs] melted        : {was_melted}  "
          f"({len(text):,} chars to parse)")

    outputs, report = build(text, save_path, was_melted)

    for fname, obj in outputs.items():
        # ensure_ascii=True -> pure-ASCII JSON (non-ASCII names \u-escaped).
        (out_dir / fname).write_text(
            json.dumps(obj, indent=1, ensure_ascii=True), encoding="utf-8")

    print()
    print(report)
    print()
    for fname in outputs:
        print(f"[designs] wrote: {dc.ps._scrub(str(out_dir / fname))}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
