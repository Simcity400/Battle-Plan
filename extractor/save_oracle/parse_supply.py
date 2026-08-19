#!/usr/bin/env python3
"""the game save -> the LOGISTICS / SUPPLY layer (``supply.json``).

Parses the top-level ``supply_system_2={ countries={ <TAG>={...} } }`` block —
the full per-country truck/rail/node logistics state the older oracle never
touched. This is the STORED logistics layer (NOT the rendered per-province
"20/20" supply number, which remains the W2 wall — that is a tooltip render, not
a stored slot).

Per country it extracts (verified against a real 1.19 melt):
  - ``priority``                : supply priority (0-3)
  - ``buffer``                  : supply buffer setting
  - ``wanted_supply_trucks``    : desired truck count
  - ``capital``                 : supply-capital province id (when present)
  - ``last_supply_capital_move``
  - ``daily_losses_index``      : ring-buffer write index for the loss arrays
  - ``last_lost_train_province``
  - ``trucks``                  : list of {equip_id, equip_type, count, damage}
                                  (motorization equipment held), + ``truck_count``
                                  total and ``truck_damage_avg``.
  - ``nodes``                   : count of supply-node settings, + a
                                  ``node_motorization`` list of the nodes that
                                  carry an explicit per-country motorization
                                  override {node_province, levels={TAG:lvl}}.
  - ``disrupted_supply``        : list of {node_province, value} (active supply
                                  disruption per node) + ``disrupted_count``.
  - ``foreign_homebase_nodes``  : list of {province, supply, start, penalty,
                                  source, duration, hours, decay, base}
                                  (docking-rights / foreign basing supply).
  - loss ring-buffers (last ~30 days, as stored):
      ``lost_railways`` / ``lost_trains`` (int arrays) +
      ``lost_railways_total`` / ``lost_trains_total`` sums,
      ``lost_trucks_attrition`` / ``lost_trucks_killed`` (float arrays) +
      their rounded sums.

Key paths:
  supply_system_2.countries.<TAG>.{priority,buffer,wanted_supply_trucks,...}
  ...truck={ { id={id type} count=N damage=f } ... }
  ...settings.node={ id={ <prov> <area> } data={ disabled motorization_level={TAG=lvl} } }
  ...disrupted_supply.node={ id={ <prov> <area> } value=f }
  ...foreign_homebase_nodes={ { <prov> { supply penalty province ... } } }
  ...lost_railways / lost_trains / lost_trucks_attrition / lost_trucks_killed = { ... }

stdlib only. Reuses the shared tokenizer via ``deep_common``.
"""

from __future__ import annotations

import deep_common as dc
from deep_common import (
    LBRACE, RBRACE, EQUALS, atom_value, iter_block_entries, _skip_block_body,
    read_id_block, read_scalar_list, to_int, to_float,
)


def _parse_truck_record(tz):
    """Read one ``{ id={id type} count=N damage=f }`` block (LBRACE consumed)."""
    rec = {"equip_id": None, "equip_type": None, "count": None, "damage": None}
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "id":
                rec["equip_id"], rec["equip_type"] = read_id_block(tz)
            else:
                _skip_block_body(tz)
            continue
        v = atom_value(op)
        if v is None:
            continue
        if k == "count":
            rec["count"] = to_int(v)
        elif k == "damage":
            rec["damage"] = to_float(v)
    return rec


def _parse_trucks(tz):
    """truck={ { id={id type} count=N damage=f } ... } — list of ANONYMOUS blocks.

    ``iter_block_entries`` auto-skips anonymous blocks, so the records are read
    with the raw tokenizer (each entry is a top-level ``{ ... }`` with no key).
    """
    out = []
    depth = 1
    while depth > 0:
        tok = tz.next()
        if tok is None:
            return out
        if tok is RBRACE:
            depth -= 1
            continue
        if tok is LBRACE:
            out.append(_parse_truck_record(tz))
    return out


def _parse_node_id(tz):
    """A node id is ``id={ <prov> <area> }`` (a bare 2-atom list)."""
    atoms = read_scalar_list(tz)
    prov = to_int(atoms[0]) if atoms else None
    return prov


def _parse_settings(tz):
    """settings={ node={ id={prov area} data={ disabled motorization_level={..}}} ...}."""
    node_count = 0
    motorized = []
    for key, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        if key != "node":
            _skip_block_body(tz)
            continue
        node_count += 1
        prov = None
        levels = {}
        for k, op in iter_block_entries(tz):
            if op is LBRACE:
                if k == "id":
                    prov = _parse_node_id(tz)
                elif k == "data":
                    for kk, oo in iter_block_entries(tz):
                        if oo is LBRACE:
                            if kk == "motorization_level":
                                for kt, ot in iter_block_entries(tz):
                                    if ot is LBRACE:
                                        _skip_block_body(tz)
                                        continue
                                    lv = to_int(atom_value(ot))
                                    if lv is not None:
                                        levels[kt] = lv
                            else:
                                _skip_block_body(tz)
                else:
                    _skip_block_body(tz)
        if levels:
            motorized.append({"node_province": prov, "levels": levels})
    return node_count, motorized


def _parse_disrupted(tz):
    """disrupted_supply={ node={ id={prov area} value=f } ... }."""
    out = []
    for key, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        if key != "node":
            _skip_block_body(tz)
            continue
        prov = None
        val = None
        for k, op in iter_block_entries(tz):
            if op is LBRACE:
                if k == "id":
                    prov = _parse_node_id(tz)
                else:
                    _skip_block_body(tz)
                continue
            v = atom_value(op)
            if v is None:
                continue
            if k == "value":
                val = to_float(v)
        out.append({"node_province": prov, "value": val})
    return out


def _parse_homebase_fields(tz):
    """Read one exact serialized foreign-homebase value block.

    The a729 serializer ``FUN_141200e10`` writes all nine fields.  Keep the
    numeric lifecycle fields instead of reducing this to the four values used
    by the original pressure-candidate probe.
    """
    rec = {
        "province": None,
        "supply": None,
        "start": None,
        "penalty": None,
        "source": None,
        "duration": None,
        "hours": None,
        "decay": None,
        "base": None,
    }
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            _skip_block_body(tz)
            continue
        v = atom_value(op)
        if v is None:
            continue
        if k == "supply":
            rec["supply"] = to_float(v)
        elif k == "start":
            rec["start"] = to_float(v)
        elif k == "penalty":
            rec["penalty"] = to_float(v)
        elif k == "province":
            rec["province"] = to_int(v)
        elif k == "add":
            rec["source"] = v
        elif k == "duration":
            rec["duration"] = to_float(v)
        elif k == "hours":
            rec["hours"] = to_float(v)
        elif k == "decay":
            rec["decay"] = to_float(v)
        elif k == "base":
            rec["base"] = (v == "yes")
    return rec


def _parse_foreign_homebase(tz):
    """foreign_homebase_nodes={ { <prov> { supply penalty province add ... } } ...}.

    The entries are ANONYMOUS blocks wrapping a single ``<prov> = { ... }``.
    ``iter_block_entries`` auto-skips anonymous blocks, so this layer is walked
    with the raw tokenizer: outer ``{`` -> read ``prov`` atom -> ``=`` -> inner
    ``{ fields }`` -> closing ``}`` of the wrapper.
    """
    out = []
    depth = 1
    while depth > 0:
        tok = tz.next()
        if tok is None:
            return out
        if tok is RBRACE:
            depth -= 1
            continue
        if tok is LBRACE:
            while True:
                t2 = tz.next()
                if t2 is None or t2 is RBRACE:
                    break
                if t2 is LBRACE:
                    rec = _parse_homebase_fields(tz)
                    out.append(rec)
                    continue
                kp = atom_value(t2)
                if kp is None:
                    continue
                nxt = tz.peek()
                if nxt is EQUALS:
                    tz.next()
                    nxt = tz.peek()
                if nxt is LBRACE:
                    tz.next()
                    rec = _parse_homebase_fields(tz)
                    if rec["province"] is None:
                        rec["province"] = to_int(kp)
                    out.append(rec)
    return out


def _int_list(atoms):
    out = []
    for a in atoms:
        n = to_int(a)
        if n is not None:
            out.append(n)
    return out


def _float_list(atoms):
    out = []
    for a in atoms:
        f = to_float(a)
        if f is not None:
            out.append(f)
    return out


def _parse_country_supply(tz):
    rec = {
        "priority": None, "buffer": None, "wanted_supply_trucks": None,
        "capital": None, "last_supply_capital_move": None,
        "daily_losses_index": None, "last_lost_train_province": None,
        "trucks": [], "nodes": 0, "node_motorization": [],
        "disrupted_supply": [], "foreign_homebase_nodes": [],
        "lost_railways": [], "lost_trains": [],
        "lost_trucks_attrition": [], "lost_trucks_killed": [],
    }
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "truck":
                rec["trucks"] = _parse_trucks(tz)
            elif key == "settings":
                rec["nodes"], rec["node_motorization"] = _parse_settings(tz)
            elif key == "disrupted_supply":
                rec["disrupted_supply"] = _parse_disrupted(tz)
            elif key == "foreign_homebase_nodes":
                rec["foreign_homebase_nodes"] = _parse_foreign_homebase(tz)
            elif key == "lost_railways":
                rec["lost_railways"] = _int_list(read_scalar_list(tz))
            elif key == "lost_trains":
                rec["lost_trains"] = _int_list(read_scalar_list(tz))
            elif key == "lost_trucks_attrition":
                rec["lost_trucks_attrition"] = _float_list(read_scalar_list(tz))
            elif key == "lost_trucks_killed":
                rec["lost_trucks_killed"] = _float_list(read_scalar_list(tz))
            else:
                _skip_block_body(tz)
            continue
        v = atom_value(opener)
        if v is None:
            continue
        if key == "priority":
            rec["priority"] = to_int(v)
        elif key == "buffer":
            rec["buffer"] = to_float(v)
        elif key == "wanted_supply_trucks":
            rec["wanted_supply_trucks"] = to_int(v)
        elif key == "capital":
            rec["capital"] = to_int(v)
        elif key == "last_supply_capital_move":
            rec["last_supply_capital_move"] = to_int(v)
        elif key == "daily_losses_index":
            rec["daily_losses_index"] = to_int(v)
        elif key == "last_lost_train_province":
            rec["last_lost_train_province"] = to_int(v)
    return rec


def _parse_supply_system(tz, by_tag):
    """supply_system_2={ countries={ <TAG>={...} } ... }."""
    for key, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        if key != "countries":
            _skip_block_body(tz)
            continue
        for tag, op in iter_block_entries(tz):
            if op is not LBRACE:
                continue
            by_tag[tag] = _parse_country_supply(tz)


def build(text, source, was_melted):
    by_tag = {}
    meta = dc.top_level_pass(
        text,
        {"supply_system_2": lambda tz: _parse_supply_system(tz, by_tag)},
        scalar_keys=("date", "version", "player"),
    )

    countries = {}
    for tag, rec in by_tag.items():
        trucks = rec["trucks"]
        truck_count = sum((t["count"] or 0) for t in trucks)
        dmg = [t["damage"] for t in trucks if t["damage"] is not None]
        truck_damage_avg = round(sum(dmg) / len(dmg), 4) if dmg else None
        countries[tag] = {
            "priority": rec["priority"],
            "buffer": rec["buffer"],
            "wanted_supply_trucks": rec["wanted_supply_trucks"],
            "capital": rec["capital"],
            "last_supply_capital_move": rec["last_supply_capital_move"],
            "daily_losses_index": rec["daily_losses_index"],
            "last_lost_train_province": rec["last_lost_train_province"],
            "truck_count": truck_count,
            "truck_damage_avg": truck_damage_avg,
            "trucks": trucks,
            "node_count": rec["nodes"],
            "node_motorization": rec["node_motorization"],
            "disrupted_count": len(rec["disrupted_supply"]),
            "disrupted_supply": rec["disrupted_supply"],
            "foreign_homebase_nodes": rec["foreign_homebase_nodes"],
            "lost_railways_total": sum(rec["lost_railways"]),
            "lost_trains_total": sum(rec["lost_trains"]),
            "lost_trucks_attrition_total": round(
                sum(rec["lost_trucks_attrition"]), 3),
            "lost_trucks_killed_total": round(
                sum(rec["lost_trucks_killed"]), 3),
            "lost_railways": rec["lost_railways"],
            "lost_trains": rec["lost_trains"],
            "lost_trucks_attrition": rec["lost_trucks_attrition"],
            "lost_trucks_killed": rec["lost_trucks_killed"],
        }

    out = {
        "meta": {
            "source_filename": source.name,
            "source_path_redacted": dc.ps._redact(source),
            "was_melted": was_melted,
            "save_date": meta.get("date"),
            "save_version": meta.get("version"),
            "player": meta.get("player"),
            "notes": {
                "scope": "supply_system_2 top-level logistics layer (trucks, "
                "rail, nodes, supply disruption, losses). NOT the rendered "
                "per-province supply number (W2 wall — tooltip render).",
                "loss_arrays": "lost_* are ~30-day ring buffers; *_total is the "
                "sum over the buffer window, not lifetime.",
            },
            "stats": {
                "countries": len(countries),
                "countries_with_trucks": sum(
                    1 for c in countries.values() if c["trucks"]),
                "countries_with_disruption": sum(
                    1 for c in countries.values() if c["disrupted_count"]),
            },
        },
        "countries": dict(sorted(countries.items())),
    }

    ger = countries.get("GER", {})
    lines = ["=== SUPPLY (supply_system_2) ORACLE VALIDATION ==="]
    lines.append(f"source        : {source.name} (melted={was_melted})")
    lines.append(f"save date     : {meta.get('date')}")
    lines.append(f"countries     : {len(countries)} with supply state")
    lines.append(f"GER priority  : {ger.get('priority')}  buffer={ger.get('buffer')}")
    lines.append(f"GER trucks    : count={ger.get('truck_count')} "
                 f"(records={len(ger.get('trucks', []))}, "
                 f"dmg_avg={ger.get('truck_damage_avg')})")
    lines.append(f"GER nodes     : {ger.get('node_count')} "
                 f"(motorization overrides={len(ger.get('node_motorization', []))})")
    lines.append(f"GER disrupted : {ger.get('disrupted_count')} nodes")
    lines.append(f"GER foreign hb: {len(ger.get('foreign_homebase_nodes', []))}")
    lines.append(f"GER losses    : rail={ger.get('lost_railways_total')} "
                 f"trains={ger.get('lost_trains_total')} "
                 f"trucks_attr={ger.get('lost_trucks_attrition_total')} "
                 f"trucks_killed={ger.get('lost_trucks_killed_total')}")
    return {"supply.json": out}, "\n".join(lines)


if __name__ == "__main__":
    raise SystemExit(dc.run_cli(
        "supply", "the game save -> supply_system_2 logistics oracle.",
        build, ["supply.json"]))
