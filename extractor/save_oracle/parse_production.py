#!/usr/bin/env python3
"""the game save -> per-country PRODUCTION & EQUIPMENT oracle (``production.json``).

Deep-layer extension of the save oracle. Per country it extracts:
  - ``equipment_lines``  : military + naval + ship-refit production lines, each
                           {kind, line_id, equipment_variant=(id,type),
                           industrial_manufacturer=(id,type=79), mio_id,
                           archetype(joined), produced, active_factories,
                           requested_factories, priority, amount, speed, cost}.
                           The ordered ``factory_efficiencies`` vector is also
                           preserved exactly when present.
  - ``stockpile``        : equipment on hand, list of {equip_id, equip_type,
                           archetype, amount} from production.equipments.
  - ``convoys``          : convoy/transport equipment on hand (same shape).
  - ``factory_totals``   : {mil_factories_assigned, naval_factories_assigned}
                           summed from the lines (a cheap activity read; the
                           authoritative civ/mil/dock COUNTS come from economy).

Top-level (not per country):
  - ``equipment_archetypes`` : {("id:type"): {name, creator, obsolete, is_frame}}
                               the variant/archetype index everything joins to.

Key paths (verified against a real 1.19 melt):
  equipments.<archetype_name> = { id={id type=70} creator obsolete is_frame }
      -> the top-level archetype/variant index (block KEY = archetype name).
  countries.<TAG>.production.military_lines = {
      id={id type} equipment_variant_index={id type}
      industrial_manufacturer={id type=79}
      produced active_factories requested_factories priority amount speed cost }
      factory_efficiencies={ <ordered per-factory percentages> }
  countries.<TAG>.production.naval_lines / ship_refit_lines  (same shape)
  countries.<TAG>.production.equipments.equipment = { id={id type} amount }
  countries.<TAG>.convoys.equipment.equipment     = { id={id type} amount }

REALITY NOTE — "gross produced" counters: each production line carries a
``produced`` accumulator (total equipment ever produced on that line, a float).
That is the closest in-save signal to a gross-output counter and is included
per-line. There is no separate per-country lifetime "equipment produced" total
in the melt; the mil-stats tool's gross-IC metric is derived elsewhere.
The CONSTRUCTION queue (general_lines / buildings) is handled by parse_economy.

stdlib only. Reuses the shared tokenizer via ``deep_common``.
"""

from __future__ import annotations

import deep_common as dc
from deep_common import (
    LBRACE, atom_value, iter_block_entries, _skip_block_body,
    to_int, to_float, read_id_block, read_scalar_list,
)


# ---------------------------------------------------------------------------
# Top-level equipment archetype/variant index
# ---------------------------------------------------------------------------

def _parse_equipments_index(tz, index):
    """equipments={ <archetype>={ id={id type} creator obsolete is_frame } }."""
    for arch_name, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        eid = etype = creator = None
        obsolete = is_frame = None
        for k, op in iter_block_entries(tz):
            if op is LBRACE:
                if k == "id":
                    eid, etype = read_id_block(tz)
                else:
                    _skip_block_body(tz)
                continue
            v = atom_value(op)
            if v is None:
                continue
            if k == "creator":
                creator = v
            elif k == "obsolete":
                obsolete = (v == "yes")
            elif k == "is_frame":
                is_frame = (v == "yes")
        if eid is not None:
            index[(eid, etype)] = {
                "name": arch_name, "creator": creator,
                "obsolete": obsolete, "is_frame": is_frame,
            }


# ---------------------------------------------------------------------------
# Per-country production
# ---------------------------------------------------------------------------

def _parse_line(tz, kind):
    rec = {
        "kind": kind, "line_id": None,
        "equipment_variant": None,  # (id,type)
        "industrial_manufacturer": None,  # assigned MIO (id,type=79), or null
        "produced": None, "active_factories": None,
        "requested_factories": None, "priority": None,
        "amount": None, "speed": None, "cost": None,
        # Ordered, authoritative per-factory save values. This is intentionally
        # not replaced by a value reconstructed from speed/factory counts.
        "factory_efficiencies": None,
    }
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "id":
                _id, _t = read_id_block(tz)
                rec["line_id"] = _id
            elif k == "equipment_variant_index":
                rec["equipment_variant"] = read_id_block(tz)
            elif k == "industrial_manufacturer":
                rec["industrial_manufacturer"] = read_id_block(tz)
            elif k == "factory_efficiencies":
                raw_values = read_scalar_list(tz)
                values = [to_float(value) for value in raw_values]
                rec["factory_efficiencies"] = (
                    values if all(value is not None for value in values) else None
                )
            else:
                _skip_block_body(tz)
            continue
        v = atom_value(op)
        if v is None:
            continue
        if k == "produced":
            rec["produced"] = to_float(v)
        elif k == "active_factories":
            rec["active_factories"] = to_int(v)
        elif k == "requested_factories":
            rec["requested_factories"] = to_int(v)
        elif k == "priority":
            rec["priority"] = to_int(v)
        elif k == "amount":
            rec["amount"] = to_int(v)
        elif k == "speed":
            rec["speed"] = to_float(v)
        elif k == "cost":
            rec["cost"] = to_float(v)
    return rec


def _parse_equipment_amounts(tz):
    """Parse an ``{ equipment={ id={id type} amount=N } ... }`` list."""
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


def _parse_production_block(tz, rec):
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key in ("military_lines", "naval_lines", "ship_refit_lines"):
                rec["lines"].append(_parse_line(tz, key))
            elif key == "equipments":
                rec["stockpile_raw"] = _parse_equipment_amounts(tz)
            else:
                _skip_block_body(tz)
        # scalars (dirty, max_factories_for_repair, ...) ignored


def _parse_convoys_block(tz, rec):
    """convoys={ equipment={ equipment={ id amount } ... } }."""
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "equipment":
                rec["convoys_raw"] = _parse_equipment_amounts(tz)
            else:
                _skip_block_body(tz)


def _parse_countries(tz, by_tag):
    for tag, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        rec = {
            "lines": [],
            "stockpile_raw": [],   # (id,type,amount) joined later
            "convoys_raw": [],
        }
        for key, op in iter_block_entries(tz):
            if op is LBRACE:
                if key == "production":
                    _parse_production_block(tz, rec)
                elif key == "convoys":
                    _parse_convoys_block(tz, rec)
                else:
                    _skip_block_body(tz)
            # scalar country fields ignored
        if rec["lines"] or rec["stockpile_raw"] or rec["convoys_raw"]:
            by_tag[tag] = rec


def parse_production_text(text: str):
    by_tag: dict = {}
    arch_index: dict = {}
    meta = dc.top_level_pass(
        text,
        {
            "equipments": lambda tz: _parse_equipments_index(tz, arch_index),
            "countries": lambda tz: _parse_countries(tz, by_tag),
        },
        scalar_keys=("date", "version", "player"),
    )
    return by_tag, arch_index, meta


def _join_amounts(raw, arch_index, unknown):
    out = []
    for (eid, etype, amount) in raw:
        info = arch_index.get((eid, etype))
        if info is None:
            unknown.add((eid, etype))
        out.append({
            "equip_id": eid, "equip_type": etype,
            "archetype": info["name"] if info else None,
            "amount": amount,
        })
    return out


def build(text, source, was_melted):
    by_tag, arch_index, meta = parse_production_text(text)
    unknown = set()

    countries = {}
    for tag, rec in by_tag.items():
        lines = []
        mil_fac = naval_fac = 0
        for ln in rec["lines"]:
            ev = ln["equipment_variant"]
            info = arch_index.get(ev) if ev else None
            if ev and info is None:
                unknown.add(ev)
            af = ln["active_factories"] or 0
            if ln["kind"] == "military_lines":
                mil_fac += af
            elif ln["kind"] in ("naval_lines", "ship_refit_lines"):
                naval_fac += af
            lines.append({
                "kind": ln["kind"],
                "line_id": ln["line_id"],
                "equip_id": ev[0] if ev else None,
                "equip_type": ev[1] if ev else None,
                "archetype": info["name"] if info else None,
                "industrial_manufacturer": (
                    list(ln["industrial_manufacturer"])
                    if ln["industrial_manufacturer"] else None
                ),
                "mio_id": (
                    ln["industrial_manufacturer"][0]
                    if ln["industrial_manufacturer"]
                    and ln["industrial_manufacturer"][1] == 79
                    else None
                ),
                "produced": ln["produced"],
                "active_factories": ln["active_factories"],
                "requested_factories": ln["requested_factories"],
                "priority": ln["priority"],
                "amount": ln["amount"],
                "speed": ln["speed"],
                "cost": ln["cost"],
                "factory_efficiencies": ln["factory_efficiencies"],
            })
        stockpile = _join_amounts(rec["stockpile_raw"], arch_index, unknown)
        convoys = _join_amounts(rec["convoys_raw"], arch_index, unknown)
        countries[tag] = {
            "equipment_lines": lines,
            "stockpile": stockpile,
            "convoys": convoys,
            "factory_totals": {
                "mil_factories_assigned": mil_fac,
                "naval_factories_assigned": naval_fac,
            },
        }

    # Top-level archetype index keyed "id:type".
    arch_out = {f"{k[0]}:{k[1]}": v for k, v in arch_index.items()}

    n_lines = sum(len(c["equipment_lines"]) for c in countries.values())
    n_lines_with_mio = sum(
        1 for c in countries.values() for line in c["equipment_lines"]
        if line["mio_id"] is not None
    )
    n_lines_with_invalid_mio_type = sum(
        1 for c in countries.values() for line in c["equipment_lines"]
        if line["industrial_manufacturer"] is not None
        and line["industrial_manufacturer"][1] != 79
    )
    n_stock = sum(len(c["stockpile"]) for c in countries.values())
    out = {
        "meta": {
            "source_filename": source.name,
            "source_path_redacted": dc.ps._redact(source),
            "was_melted": was_melted,
            "save_date": meta.get("date"),
            "save_version": meta.get("version"),
            "player": meta.get("player"),
            "notes": {
                "gross_produced": "per-line 'produced' is the line's lifetime "
                "accumulator; no per-country lifetime total exists in the save.",
                "construction_queue": "civil construction (general_lines) is in "
                "economy.json, not here.",
            },
            "stats": {
                "countries_with_production": len(countries),
                "equipment_archetypes": len(arch_index),
                "total_production_lines": n_lines,
                "production_lines_with_mio": n_lines_with_mio,
                "production_lines_with_invalid_mio_type":
                    n_lines_with_invalid_mio_type,
                "total_stockpile_entries": n_stock,
                "unknown_equipment_ids": len(unknown),
            },
        },
        "equipment_archetypes": arch_out,
        "countries": dict(sorted(countries.items())),
    }

    ger = countries.get("GER", {})
    ger_lines = ger.get("equipment_lines", [])
    ger_fac = ger.get("factory_totals", {})
    lines = ["=== PRODUCTION ORACLE VALIDATION ==="]
    lines.append(f"source        : {source.name} (melted={was_melted})")
    lines.append(f"save date     : {meta.get('date')}")
    lines.append(f"archetypes    : {len(arch_index)} (unknown joins: {len(unknown)})")
    lines.append(f"countries     : {len(countries)}  lines={n_lines}  "
                 f"stockpile entries={n_stock}")
    lines.append(f"GER lines     : {len(ger_lines)} "
                 f"(mil fac={ger_fac.get('mil_factories_assigned')}, "
                 f"naval fac={ger_fac.get('naval_factories_assigned')})")
    lines.append(f"GER stockpile : {len(ger.get('stockpile', []))} entries, "
                 f"convoys={len(ger.get('convoys', []))}")
    # top GER mil lines by active factories
    top = sorted([l for l in ger_lines if l["kind"] == "military_lines"],
                 key=lambda l: l["active_factories"] or 0, reverse=True)[:3]
    for l in top:
        lines.append(f"   {l['archetype']}: {l['active_factories']} fac "
                     f"produced={l['produced']}")
    return {"production.json": out}, "\n".join(lines)


if __name__ == "__main__":
    raise SystemExit(dc.run_cli(
        "production", "the game save -> per-country production & equipment oracle.",
        build, ["production.json"]))
