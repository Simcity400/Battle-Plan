#!/usr/bin/env python3
"""the game save -> per-country ECONOMY / RESOURCES / LAWS oracle (``economy.json``).

Deep-layer extension of the save oracle. Per country it extracts:
  - ``factories``       : {civ, mil, dockyard} summed from OWNED state buildings
                          PLUS the country's off-map ``buildings`` block, and a
                          ``under_construction`` count from the build queue.
                          (Cross-check: civ/mil/dock match country_snapshot.json,
                          which sums only state buildings.)
  - ``construction_queue`` : list of {template, location_state, active_factories,
                          produced, cost, priority, created_date} from
                          ``production.general_lines.building`` (civil builds).
  - ``resources``       : {produced{<res>:n}, to_use_total{<res>:n}, exported,
                          available{<res>:n}}. ``available`` = produced - exported
                          (what the country can consume); see REALITY NOTE.
  - ``fuel``            : {current, max} from fuel_status.
  - ``laws``            : conscription / economy / trade-laws tokens, picked out
                          of the flat ``politics.ideas`` list by a known-prefix
                          classifier. Anything not a recognized law stays in
                          ``ideas_other`` (spirits + advisors + traits).
  - ``ideas_all``       : the full flat ideas token list (laws + spirits +
                          advisors), as stored.
  - ``stability`` / ``war_support`` / ``political_power`` / ``command_power``
  - ``available_manpower`` : countries.<TAG>.manpower.ratio (the available pool).

Key paths (verified against a real 1.19 melt):
  states.<id>.owner / .buildings.{industrial_complex,arms_factory,dockyard}.level
  countries.<TAG>.buildings.<bld>.level        (off-map / shared factories)
  countries.<TAG>.production.general_lines.building = {
      active_factories produced cost priority created_date
      building={ template=arms_factory location=75 } }
  countries.<TAG>.resources.produced = { steel=1204 oil=14 ... }
  countries.<TAG>.resources.exported = { ... }   (anonymous to_use maps summed)
  countries.<TAG>.fuel_status.{fuel,max_fuel}
  countries.<TAG>.politics.ideas = { war_economy extensive_conscription ... }
  countries.<TAG>.{stability,war_support,command_power}
  countries.<TAG>.politics.political_power
  countries.<TAG>.manpower.ratio

REALITY NOTE — resources: the melt stores gross ``produced`` per resource and an
``exported`` map, plus several anonymous ``to_use`` sub-maps (consumption by
category, including negatives). There is NO single literal ``available`` key. We
report ``produced`` and ``exported`` verbatim and compute a derived
``available = produced - exported`` (resources kept for own use). The per-line
resource *needs* are in production.json.

stdlib only. Reuses the shared tokenizer via ``deep_common``.
"""

from __future__ import annotations

import deep_common as dc
from deep_common import (
    LBRACE, atom_value, iter_block_entries, _skip_block_body,
    to_int, to_float, collect_block,
)

FACTORY_BLD = {
    "industrial_complex": "civ",
    "arms_factory": "mil",
    "dockyard": "dockyard",
}

# Law tokens are mod-specific, but the engine's law IDEAS reliably contain these
# substrings. We classify a flat idea token as a law if it matches; everything
# else (spirits, advisors, traits) is kept separately.
LAW_PATTERNS = {
    "economy": ("_economy", "war_economy", "civilian_economy",
                "early_mobilization", "partial_mobilization",
                "total_mobilization", "tot_economic_mobilisation"),
    "trade": ("free_trade", "export_focus", "limited_exports", "closed_economy"),
    "conscription": ("conscription", "service_by_requirement",
                     "all_adults_serve", "scraping_the_barrel",
                     "disarmed_nation", "volunteer_only", "limited_conscription",
                     "extensive_conscription"),
}


def _classify_idea(token):
    low = token.lower()
    for law_kind, pats in LAW_PATTERNS.items():
        for p in pats:
            if p in low:
                return law_kind
    return None


# ---------------------------------------------------------------------------
# states pass: per-owner factory totals from state buildings
# ---------------------------------------------------------------------------

def _parse_state_buildings(tz):
    f = {"civ": 0, "mil": 0, "dockyard": 0}
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            bucket = FACTORY_BLD.get(key)
            if bucket is not None:
                lvl = 0
                for k, op in iter_block_entries(tz):
                    if op is LBRACE:
                        _skip_block_body(tz)
                    elif k == "level":
                        lvl = to_int(atom_value(op)) or 0
                f[bucket] += lvl
            else:
                _skip_block_body(tz)
    return f


def _parse_states(tz, owner_factories):
    for sid, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        owner = None
        f = {"civ": 0, "mil": 0, "dockyard": 0}
        for key, op in iter_block_entries(tz):
            if op is LBRACE:
                if key == "buildings":
                    f = _parse_state_buildings(tz)
                else:
                    _skip_block_body(tz)
                continue
            v = atom_value(op)
            if v is None:
                continue
            if key == "owner":
                owner = v
        if owner:
            agg = owner_factories.setdefault(
                owner, {"civ": 0, "mil": 0, "dockyard": 0})
            agg["civ"] += f["civ"]
            agg["mil"] += f["mil"]
            agg["dockyard"] += f["dockyard"]


# ---------------------------------------------------------------------------
# countries pass
# ---------------------------------------------------------------------------

def _parse_offmap_buildings(tz):
    f = {"civ": 0, "mil": 0, "dockyard": 0}
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            bucket = FACTORY_BLD.get(key)
            if bucket is not None:
                lvl = 0
                for k, op in iter_block_entries(tz):
                    if op is LBRACE:
                        _skip_block_body(tz)
                    elif k == "level":
                        lvl = to_int(atom_value(op)) or 0
                f[bucket] += lvl
            else:
                _skip_block_body(tz)
    return f


def _parse_general_lines(tz, queue):
    """general_lines={ building={ ... building={template location} } ... }."""
    for key, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        if key != "building":
            _skip_block_body(tz)
            continue
        rec = {"template": None, "location_state": None,
               "active_factories": None, "produced": None, "cost": None,
               "priority": None, "created_date": None}
        for k, op in iter_block_entries(tz):
            if op is LBRACE:
                if k == "building":  # inner { template location }
                    for kk, oo in iter_block_entries(tz):
                        if oo is LBRACE:
                            _skip_block_body(tz)
                            continue
                        vv = atom_value(oo)
                        if vv is None:
                            continue
                        if kk == "template":
                            rec["template"] = vv
                        elif kk == "location":
                            rec["location_state"] = to_int(vv)
                else:
                    _skip_block_body(tz)
                continue
            v = atom_value(op)
            if v is None:
                continue
            if k == "active_factories":
                rec["active_factories"] = to_int(v)
            elif k == "produced":
                rec["produced"] = to_float(v)
            elif k == "cost":
                rec["cost"] = to_float(v)
            elif k == "priority":
                rec["priority"] = to_int(v)
            elif k == "created_date":
                rec["created_date"] = v
        queue.append(rec)


def _parse_production_for_queue(tz, rec):
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "general_lines":
                _parse_general_lines(tz, rec["construction_queue"])
            else:
                _skip_block_body(tz)


def _parse_resources(tz, rec):
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "produced":
                rec["res_produced"] = {
                    k: to_float(v) for k, v in collect_block(tz).items()}
            elif key == "exported":
                rec["res_exported"] = {
                    k: to_float(v) for k, v in collect_block(tz).items()}
            elif key == "to_export":
                rec["res_to_export"] = {
                    k: to_float(v) for k, v in collect_block(tz).items()}
            else:
                _skip_block_body(tz)


def _parse_politics(tz, rec):
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "ideas":
                rec["ideas_all"] = dc.read_scalar_list(tz)
            else:
                _skip_block_body(tz)
            continue
        if opener is None:
            continue
        v = atom_value(opener)
        if v is None:
            continue
        if key == "political_power":
            rec["political_power"] = to_float(v)
        elif key == "ruling_party":
            rec["ruling_party"] = v


def _parse_fuel(tz, rec):
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            _skip_block_body(tz)
            continue
        v = atom_value(opener)
        if v is None:
            continue
        if key == "fuel":
            rec["fuel"] = to_float(v)
        elif key == "max_fuel":
            rec["max_fuel"] = to_float(v)


def _parse_countries(tz, by_tag):
    for tag, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        rec = {
            "offmap_factories": {"civ": 0, "mil": 0, "dockyard": 0},
            "construction_queue": [],
            "res_produced": {}, "res_exported": {}, "res_to_export": {},
            "ideas_all": [],
            "fuel": None, "max_fuel": None,
            "stability": None, "war_support": None,
            "political_power": None, "command_power": None,
            "ruling_party": None,
            "available_manpower": None,
        }
        for key, op in iter_block_entries(tz):
            if op is LBRACE:
                if key == "buildings":
                    rec["offmap_factories"] = _parse_offmap_buildings(tz)
                elif key == "production":
                    _parse_production_for_queue(tz, rec)
                elif key == "resources":
                    _parse_resources(tz, rec)
                elif key == "politics":
                    _parse_politics(tz, rec)
                elif key == "fuel_status":
                    _parse_fuel(tz, rec)
                elif key == "manpower":
                    for k, o in iter_block_entries(tz):
                        if o is LBRACE:
                            _skip_block_body(tz)
                        elif k == "ratio":
                            rec["available_manpower"] = to_int(atom_value(o))
                else:
                    _skip_block_body(tz)
                continue
            v = atom_value(op)
            if v is None:
                continue
            if key == "stability":
                rec["stability"] = to_float(v)
            elif key == "war_support":
                rec["war_support"] = to_float(v)
            elif key == "command_power":
                rec["command_power"] = to_float(v)
        by_tag[tag] = rec


def parse_economy_text(text: str):
    by_tag: dict = {}
    owner_factories: dict = {}
    meta = dc.top_level_pass(
        text,
        {
            "states": lambda tz: _parse_states(tz, owner_factories),
            "countries": lambda tz: _parse_countries(tz, by_tag),
        },
        scalar_keys=("date", "version", "player"),
    )
    return by_tag, owner_factories, meta


def build(text, source, was_melted):
    by_tag, owner_factories, meta = parse_economy_text(text)

    countries = {}
    for tag, rec in by_tag.items():
        state_f = owner_factories.get(
            tag, {"civ": 0, "mil": 0, "dockyard": 0})
        off = rec["offmap_factories"]
        factories = {
            "civ": state_f["civ"] + off["civ"],
            "mil": state_f["mil"] + off["mil"],
            "dockyard": state_f["dockyard"] + off["dockyard"],
            "civ_from_states": state_f["civ"],
            "mil_from_states": state_f["mil"],
            "dockyard_from_states": state_f["dockyard"],
            "offmap": off,
            "under_construction": len(rec["construction_queue"]),
        }

        # Laws vs other ideas.
        laws = {"economy": None, "trade": None, "conscription": None}
        ideas_other = []
        for tok in rec["ideas_all"]:
            kind = _classify_idea(tok)
            if kind and laws[kind] is None:
                laws[kind] = tok
            elif kind:
                # multiple matched the same bucket; keep first, push rest to other
                ideas_other.append(tok)
            else:
                ideas_other.append(tok)

        # Derived available resources.
        avail = {}
        for r, amt in rec["res_produced"].items():
            exp = rec["res_exported"].get(r, 0) or 0
            avail[r] = round((amt or 0) - exp, 3)

        countries[tag] = {
            "factories": factories,
            "construction_queue": rec["construction_queue"],
            "resources": {
                "produced": rec["res_produced"],
                "exported": rec["res_exported"],
                "to_export": rec["res_to_export"],
                "available_derived": avail,
            },
            "fuel": {"current": rec["fuel"], "max": rec["max_fuel"]},
            "laws": laws,
            "ideas_all": rec["ideas_all"],
            "ideas_other": ideas_other,
            "stability": rec["stability"],
            "war_support": rec["war_support"],
            "political_power": rec["political_power"],
            "command_power": rec["command_power"],
            "ruling_party": rec["ruling_party"],
            "available_manpower": rec["available_manpower"],
        }

    total_builds = sum(len(c["construction_queue"]) for c in countries.values())
    out = {
        "meta": {
            "source_filename": source.name,
            "source_path_redacted": dc.ps._redact(source),
            "was_melted": was_melted,
            "save_date": meta.get("date"),
            "save_version": meta.get("version"),
            "player": meta.get("player"),
            "notes": {
                "available_resources": "no literal 'available' key; "
                "available_derived = produced - exported.",
                "laws": "laws are flat tokens in politics.ideas, classified by "
                "substring; ideas_other holds spirits/advisors/traits.",
                "factories_cross_check": "factories.civ/mil/dockyard = states + "
                "offmap; *_from_states matches country_snapshot.json.",
            },
            "stats": {
                "countries": len(countries),
                "total_construction_lines": total_builds,
            },
        },
        "countries": dict(sorted(countries.items())),
    }

    ger = countries.get("GER", {})
    gf = ger.get("factories", {})
    lines = ["=== ECONOMY ORACLE VALIDATION ==="]
    lines.append(f"source        : {source.name} (melted={was_melted})")
    lines.append(f"save date     : {meta.get('date')}")
    lines.append(f"countries     : {len(countries)}  build lines={total_builds}")
    lines.append(f"GER factories : civ={gf.get('civ')} mil={gf.get('mil')} "
                 f"dock={gf.get('dockyard')} (states-only "
                 f"{gf.get('civ_from_states')}/{gf.get('mil_from_states')}/"
                 f"{gf.get('dockyard_from_states')}, offmap={gf.get('offmap')})")
    lines.append(f"GER under construction: {gf.get('under_construction')}")
    lines.append(f"GER laws      : {ger.get('laws')}")
    lines.append(f"GER fuel      : {ger.get('fuel')}")
    lines.append(f"GER stab/ws/pp: {ger.get('stability')}/"
                 f"{ger.get('war_support')}/{ger.get('political_power')}")
    lines.append(f"GER steel prod: {ger.get('resources', {}).get('produced', {}).get('steel')}")
    return {"economy.json": out}, "\n".join(lines)


if __name__ == "__main__":
    raise SystemExit(dc.run_cli(
        "economy", "the game save -> per-country economy/resources/laws oracle.",
        build, ["economy.json"]))
