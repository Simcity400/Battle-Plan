#!/usr/bin/env python3
"""the game save -> DEPLOYMENT CONVEYOR + LEGACY EXPEDITIONARY oracle
(``deployment.json``).

Deep-layer extension capturing two unit-movement layers no existing module owns:

  (A) DEPLOYMENT CONVEYOR / REINFORCEMENT QUEUE — per country, the divisions
      being raised in the deployment queue (the "conveyor"). Stored at
      ``countries.<TAG>.deployment.military_deployment_conveyor[]``. Per conveyor:
        - ``name``                : division name being deployed (e.g. "Infantry
                                    1936 V").
        - ``template_id``         : division_template_id (id,type) being raised.
        - ``amount``              : how many divisions this conveyor is raising.
        - ``location``            : province id where they deploy.
        - ``priority``            : the conveyor's deployment priority.
        - ``role``                : deployment role (e.g. ``garrison``).
        - ``closed``              : whether the conveyor is paused.
        - ``lines``               : the per-unit ``military_deployment_line[]``,
                                    each = one unit in progress:
            * ``id``              : the deployment-line id (id,type).
            * ``name_order``      : division_name.name_order (name slot).
            * ``amount``          : units on this line.
            * ``training``        : current training level (0..1 fraction).
            * ``max_training``    : training cap.
            * ``manpower_have`` / ``manpower_need`` / ``max_manpower`` : manpower
              progress (army_manpower_value / army_manpower_need / max_manpower).
            * ``equipment``       : EQUIPMENT-IN-TRANSIT — [{equip_id,equip_type,
                                    archetype,amount}] already allocated to this
                                    deploying unit (joined to the equipments index).
      The per-country country-level ``deployment`` block ALSO carries deployment
      *settings* (``default_hq_template``, ``initial_carrier_air_wing_deployment``,
      ``unit_modifiers``, ``unlocked_subunits``) — those are doctrine/template
      defaults, NOT the conveyor, so we skip them (out of scope here).

      The companion country-level ``reinforcement={ priority=N }`` block is the
      country's global reinforcement-priority setting; captured as
      ``reinforcement_priority``.

  (B) LEGACY EXPEDITIONARY FORCE LIST — land divisions one country OWNS but
      another country CONTROLS (lent expeditionary forces). The save lists each
      division under the CONTROLLER's ``countries.<TAG>.units.division[]`` but
      stamps the division's true owner in ``logical_country``. When
      ``logical_country`` != the enclosing controller tag, that division is a lent
      / expeditionary unit. We emit ``expeditionary[]`` as
      {owner, controller, divisions:[{div_id, div_type, name_order, location}]}.
      DISTINCT from parse_oob (which records ARMY/theatre control + only the
      division (id,type) key, never the land owner≠controller pairing) and from
      parse_divisions_full / parse_units (which record a division's owner =
      logical_country but NOT the controller it is listed under). In THIS melt
      every land division's logical_country equals its controller (no active
      expeditionary forces), so ``expeditionary`` is [] with an explicit
      ``divisions_scanned`` count — never fabricated.

WHY a rehost backstop (DESIGN 11.3): the deployment conveyor (half-raised
divisions, their training %, and the equipment already poured into them) and the
expeditionary lend graph are live-only state that the daily memory layer does not
surface; after a host crash + rehost the save is the only record of what was
mid-deployment and who was holding whose troops.

stdlib only. Reuses the parse_save tokenizer via deep_common. NEW file: does not
edit oob.json / divisions.json / parse_oob.py / parse_divisions_full.py.

Verified against re_scratch/campaign_melt_1940.txt (paused 1940, 143-country
world) and refuted with an adversarial Codex pass.
"""

from __future__ import annotations

import deep_common as dc
from deep_common import (
    LBRACE, RBRACE, atom_value, iter_block_entries, _skip_block_body,
    read_id_block, to_int, to_float, run_cli, top_level_pass,
)


# ---------------------------------------------------------------------------
# Top-level equipments archetype index (id,type) -> archetype name
# ---------------------------------------------------------------------------

def _parse_equipments_index(tz, index):
    """equipments={ <archetype>={ id={id type} ... } } -> {(id,type): archetype}."""
    for arch_name, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        eid = etype = None
        for k, op in iter_block_entries(tz):
            if op is LBRACE:
                if k == "id":
                    eid, etype = read_id_block(tz)
                else:
                    _skip_block_body(tz)
                continue
            atom_value(op)
        if eid is not None:
            index[(eid, etype)] = arch_name


# ---------------------------------------------------------------------------
# (A) deployment conveyor
# ---------------------------------------------------------------------------

def _parse_equipment_amounts(tz):
    """equipment={ id={id type} amount=N } list -> [(id,type,amount)]."""
    out = []
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "equipment":
                eid = etype = None
                amount = None
                for kk, oo in iter_block_entries(tz):
                    if oo is LBRACE:
                        if kk == "id":
                            eid, etype = read_id_block(tz)
                        else:
                            _skip_block_body(tz)
                        continue
                    v = atom_value(oo)
                    if v is None:
                        continue
                    if kk == "amount":
                        amount = to_float(v)
                if eid is not None:
                    out.append((eid, etype, amount))
            else:
                _skip_block_body(tz)
    return out


def _parse_manpower_value(tz):
    """army_manpower_value={ value={ tag=".." value=N } } -> int (or None)."""
    total = None
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "value":
                for kk, oo in iter_block_entries(tz):
                    if oo is LBRACE:
                        _skip_block_body(tz)
                        continue
                    v = atom_value(oo)
                    if v is None:
                        continue
                    if kk == "value":
                        iv = to_int(v)
                        if iv is not None:
                            total = (total or 0) + iv
            else:
                _skip_block_body(tz)
    return total


def _parse_military_deployment(tz, line):
    """military_deployment={ training max_training army_manpower_value/need
       equipment{...} max_manpower }."""
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "army_manpower_value":
                line["manpower_have"] = _parse_manpower_value(tz)
            elif key == "army_manpower_need":
                line["manpower_need"] = _parse_manpower_value(tz)
            elif key == "equipment":
                line["equipment_raw"] = _parse_equipment_amounts(tz)
            else:
                _skip_block_body(tz)
            continue
        v = atom_value(opener)
        if v is None:
            continue
        if key == "training":
            line["training"] = to_float(v)
        elif key == "max_training":
            line["max_training"] = to_float(v)
        elif key == "max_manpower":
            line["max_manpower"] = to_int(v)


def _parse_deployment_line(tz):
    """military_deployment_line={ id division_name{type name_order} amount
       military_deployment{...} }."""
    line = {
        "id": None, "name_order": None, "amount": None,
        "training": None, "max_training": None,
        "manpower_have": None, "manpower_need": None, "max_manpower": None,
        "equipment_raw": [],
    }
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "id":
                line["id"] = read_id_block(tz)
            elif key == "division_name":
                for k, op in iter_block_entries(tz):
                    if op is LBRACE:
                        _skip_block_body(tz)
                        continue
                    v = atom_value(op)
                    if v is None:
                        continue
                    if k == "name_order":
                        line["name_order"] = to_int(v)
            elif key == "military_deployment":
                _parse_military_deployment(tz, line)
            else:
                _skip_block_body(tz)
            continue
        v = atom_value(opener)
        if v is None:
            continue
        if key == "amount":
            line["amount"] = to_int(v)
    return line


def _parse_conveyor(tz):
    """military_deployment_conveyor={ id division_template_id name amount location
       priority role closed military_deployment_line[] }."""
    rec = {
        "name": None, "template_id": None, "amount": None, "location": None,
        "priority": None, "role": None, "closed": None, "lines": [],
    }
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "division_template_id":
                rec["template_id"] = read_id_block(tz)
            elif key == "military_deployment_line":
                rec["lines"].append(_parse_deployment_line(tz))
            else:
                # id={..} and any other block we don't surface
                _skip_block_body(tz)
            continue
        v = atom_value(opener)
        if v is None:
            continue
        if key == "name":
            rec["name"] = v
        elif key == "amount":
            rec["amount"] = to_int(v)
        elif key == "location":
            rec["location"] = to_int(v)
        elif key == "priority":
            rec["priority"] = to_int(v)
        elif key == "role":
            rec["role"] = v
        elif key == "closed":
            rec["closed"] = (v == "yes")
    return rec


def _parse_deployment_block(tz, rec):
    """countries.<TAG>.deployment={ military_deployment_conveyor[] + SETTINGS }.

    The conveyors are the queue; the sibling settings blocks
    (default_hq_template / initial_carrier_air_wing_deployment / unit_modifiers /
    unlocked_subunits) are deployment DEFAULTS and out of scope -> skipped.
    """
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "military_deployment_conveyor":
                rec["conveyors"].append(_parse_conveyor(tz))
            else:
                _skip_block_body(tz)


# ---------------------------------------------------------------------------
# (B) legacy expeditionary: divisions whose logical_country != controller tag
# ---------------------------------------------------------------------------

def _parse_units_for_expeditionary(tz, controller, store):
    """Walk ``units={ division={..} fleet={..} }`` for the CONTROLLER tag.

    For each land division, read its ``logical_country`` (owner), id and
    location. A division whose owner != controller is a lent / expeditionary unit.
    """
    for k, op in iter_block_entries(tz):
        if op is not LBRACE:
            continue
        if k != "division":
            _skip_block_body(tz)
            continue
        store["divisions_scanned"] += 1
        div = {"div_id": None, "div_type": None, "owner": None,
               "location": None, "name_order": None}
        for kk, oo in iter_block_entries(tz):
            if oo is LBRACE:
                if kk == "id" and div["div_id"] is None:
                    div["div_id"], div["div_type"] = read_id_block(tz)
                elif kk == "division_name":
                    for k3, o3 in iter_block_entries(tz):
                        if o3 is LBRACE:
                            _skip_block_body(tz)
                            continue
                        v = atom_value(o3)
                        if v is not None and k3 == "name_order":
                            div["name_order"] = to_int(v)
                else:
                    _skip_block_body(tz)
                continue
            v = atom_value(oo)
            if v is None:
                continue
            if kk == "logical_country":
                div["owner"] = v
            elif kk == "location":
                div["location"] = to_int(v)
        owner = div["owner"]
        if owner is not None and owner != controller:
            key = (owner, controller)
            store["pairs"].setdefault(key, []).append({
                "div_id": div["div_id"], "div_type": div["div_type"],
                "name_order": div["name_order"], "location": div["location"],
            })


# ---------------------------------------------------------------------------
# Countries pass
# ---------------------------------------------------------------------------

def _parse_countries(tz, conv_by_tag, rein_by_tag, exped):
    for tag, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        crec = {"conveyors": []}
        for key, op in iter_block_entries(tz):
            if op is LBRACE:
                if key == "deployment":
                    _parse_deployment_block(tz, crec)
                elif key == "reinforcement":
                    for k, o in iter_block_entries(tz):
                        if o is LBRACE:
                            _skip_block_body(tz)
                            continue
                        v = atom_value(o)
                        if v is not None and k == "priority":
                            rein_by_tag[tag] = to_int(v)
                elif key == "units":
                    _parse_units_for_expeditionary(tz, tag, exped)
                else:
                    _skip_block_body(tz)
        if crec["conveyors"]:
            conv_by_tag[tag] = crec["conveyors"]


# ---------------------------------------------------------------------------
# Build
# ---------------------------------------------------------------------------

def _join_equipment(raw, arch_index, unknown):
    out = []
    for eid, etype, amount in raw:
        name = arch_index.get((eid, etype))
        if name is None:
            unknown.add((eid, etype))
        out.append({"equip_id": eid, "equip_type": etype,
                    "archetype": name, "amount": amount})
    return out


def build(text, source, was_melted):
    arch_index: dict = {}
    conv_by_tag: dict = {}
    rein_by_tag: dict = {}
    exped = {"pairs": {}, "divisions_scanned": 0}

    meta = top_level_pass(
        text,
        {
            "equipments": lambda tz: _parse_equipments_index(tz, arch_index),
            "countries": lambda tz: _parse_countries(
                tz, conv_by_tag, rein_by_tag, exped),
        },
        scalar_keys=("date", "version", "player"),
    )

    unknown: set = set()

    # Finalize per-country conveyors (join equipment-in-transit).
    countries = {}
    n_conveyors = 0
    n_lines = 0
    n_equip = 0
    for tag in sorted(set(conv_by_tag) | set(rein_by_tag)):
        conveyors = []
        for c in conv_by_tag.get(tag, []):
            lines = []
            for ln in c["lines"]:
                eq = _join_equipment(ln["equipment_raw"], arch_index, unknown)
                n_equip += len(eq)
                lines.append({
                    "id": ln["id"], "name_order": ln["name_order"],
                    "amount": ln["amount"], "training": ln["training"],
                    "max_training": ln["max_training"],
                    "manpower_have": ln["manpower_have"],
                    "manpower_need": ln["manpower_need"],
                    "max_manpower": ln["max_manpower"],
                    "equipment_in_transit": eq,
                })
            n_lines += len(lines)
            conveyors.append({
                "name": c["name"], "template_id": c["template_id"],
                "amount": c["amount"], "location": c["location"],
                "priority": c["priority"], "role": c["role"],
                "closed": c["closed"], "lines": lines,
            })
        n_conveyors += len(conveyors)
        countries[tag] = {
            "reinforcement_priority": rein_by_tag.get(tag),
            "conveyors": conveyors,
            "conveyor_count": len(conveyors),
        }

    # Finalize expeditionary pairs.
    expeditionary = []
    for (owner, controller), divs in sorted(exped["pairs"].items()):
        expeditionary.append({
            "owner": owner, "controller": controller,
            "division_count": len(divs), "divisions": divs,
        })

    out = {
        "meta": {
            "source_filename": source.name,
            "was_melted": was_melted,
            "save_date": meta.get("date"),
            "save_version": meta.get("version"),
            "player": meta.get("player"),
            "notes": {
                "deployment_conveyor": "countries.<TAG>.deployment."
                "military_deployment_conveyor[] = the deployment queue (divisions "
                "being raised). Each conveyor has name/template/amount/location/"
                "priority/role/closed + per-unit lines carrying training %, "
                "manpower progress, and equipment-in-transit. The sibling "
                "deployment SETTINGS blocks (default_hq_template / "
                "initial_carrier_air_wing_deployment / unit_modifiers / "
                "unlocked_subunits) are deployment defaults, intentionally skipped.",
                "reinforcement": "countries.<TAG>.reinforcement={ priority=N } is "
                "the country's global reinforcement-priority setting "
                "(reinforcement_priority). The per-division reinforcement/upgrade "
                "REQUESTS live inside each division (units.division[].requests) and "
                "are out of scope here.",
                "expeditionary": "LEGACY expeditionary = land divisions whose "
                "logical_country (owner) != the country block they are listed under "
                "(controller). EMPTY in this save (every division's owner == its "
                "controller); divisions_scanned reports how many were checked. "
                "DISTINCT from oob.json (army/theatre control) and divisions.json "
                "(owner only, no controller pairing).",
                "archetype_join": "equipment-in-transit (equip_id,equip_type) joined "
                "to the top-level equipments index (same as production.json). "
                "Unresolved ids keep archetype=null; counted in unknown_equipment_ids.",
            },
            "stats": {
                "countries_with_conveyor": len(conv_by_tag),
                "countries_with_reinforcement_priority": len(rein_by_tag),
                "total_conveyors": n_conveyors,
                "total_deploying_unit_lines": n_lines,
                "total_equipment_in_transit_entries": n_equip,
                "expeditionary_pairs": len(expeditionary),
                "expeditionary_divisions": sum(
                    e["division_count"] for e in expeditionary),
                "divisions_scanned": exped["divisions_scanned"],
                "unknown_equipment_ids": len(unknown),
            },
        },
        "countries": countries,
        "expeditionary": expeditionary,
    }

    lines_out = ["=== DEPLOYMENT CONVEYOR + LEGACY EXPEDITIONARY VALIDATION ==="]
    lines_out.append(f"source                 : {source.name} (melted={was_melted})")
    lines_out.append(f"save date              : {meta.get('date')}")
    lines_out.append(f"countries w/ conveyor  : {len(conv_by_tag)}  "
                     f"(conveyors={n_conveyors}  deploying-unit lines={n_lines}  "
                     f"equip-in-transit entries={n_equip})")
    lines_out.append(f"reinforcement-priority : {len(rein_by_tag)} countries")
    lines_out.append(f"expeditionary pairs    : {len(expeditionary)}  "
                     f"(divisions scanned={exped['divisions_scanned']})  "
                     f"unknown ids={len(unknown)}")
    shown = 0
    for tag, rec in countries.items():
        for c in rec["conveyors"]:
            tr = c["lines"][0]["training"] if c["lines"] else None
            lines_out.append(
                f"   {tag} deploying {c['name']!r} x{c['amount']} "
                f"@loc {c['location']} prio={c['priority']} role={c['role']} "
                f"(lines={len(c['lines'])}, first training={tr})")
            shown += 1
            if shown >= 8:
                break
        if shown >= 8:
            break
    return {"deployment.json": out}, "\n".join(lines_out)


if __name__ == "__main__":
    raise SystemExit(run_cli(
        "deployment",
        "the game save -> deployment conveyor + legacy expeditionary oracle.",
        build, ["deployment.json"]))
