#!/usr/bin/env python3
"""the game save -> per-country STORED-BUT-UNPARSED fields (``country_extra.json``)
plus the lobby player names (``players.json``).

The 76-field per-country record (enumerated from a real melt) carries several
fields the audit flagged as STORED yet never parsed. This module reaches them —
they are plain reads, not models or walls:

  - ``nukes``               : the nuclear stockpile list ({amount, nukes_ready}
                              per bomb type) -> nukes_total / nukes_ready_total.
                              (Area 25: the stockpile IS in the save, not just
                              the reactor BUILDING.)
  - ``dynamic_modifier``    : active named country modifiers
                              [{modifier, enabled, value[]}] (Area 20).
  - ``ace``                 : the country's flying ace(s) (name/surname/callsign,
                              modifier, alive, kill_type, killer) (Areas 13/15).
  - ``reinforcement``       : {priority} (deployment-priority setting, Area 11).
  - ``deployment``          : summary — names of deployment sub-blocks present
                              (carrier-wing defaults, hq template, unit_modifiers,
                              unlocked_subunits) (Area 11 conveyor).
  - ``program_status``      : special-projects summary — projects started /
                              completed, active programs (facilities w/ building +
                              province), and the per-specialization breakthrough
                              points (Area 8).
  - ``experience_status``   : {army_experience, navy_experience, air_experience}.
  - ``script_flags``        : per-country script flags from
                              ``countries.<TAG>.flags`` as
                              {flag: {value, date}}. Only emitted for countries
                              whose save block actually has ``flags``.
  - ``strategic_navy``      : shape-preserving read of the save-persisted
                              strategic_navy block (dockyard caps, convoy
                              region/naval-transport maps, naval-base records).
                              This is save-backed only; no live offset is
                              claimed.
  - ``convoys_destroyed`` / ``accidents_score`` / ``cosmetic_tag``.
  - ``pride_of_the_fleet``  : {id,type} reference (Area 14).
  - ``collaboration``       : present/empty flag (exile/collab block, Area 2).

``players.json`` parses ``player_countries.<TAG>.user`` — the lobby persona /
display name (W4 demotion: the chosen name IS in the save; only the true Steam
ID stays a wall). Sparse in SP saves; populated in MP.

stdlib only. Reuses the shared tokenizer via ``deep_common``.
"""

from __future__ import annotations

import deep_common as dc
from deep_common import (
    LBRACE, RBRACE, EQUALS, atom_value, iter_block_entries, _skip_block_body,
    read_id_block, read_scalar_list, to_int, to_float,
)


# ---------------------------------------------------------------------------
# player_countries.<TAG>.user  (players.json)
# ---------------------------------------------------------------------------

def _parse_player_countries(tz, players):
    """player_countries={ <TAG>={ user="name" country_leader=yes id=N } ... }."""
    for tag, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        rec = {"user": None, "country_leader": None, "id": None}
        for k, op in iter_block_entries(tz):
            if op is LBRACE:
                _skip_block_body(tz)
                continue
            v = atom_value(op)
            if v is None:
                continue
            if k == "user":
                rec["user"] = v
            elif k == "country_leader":
                rec["country_leader"] = (v == "yes")
            elif k == "id":
                rec["id"] = to_int(v)
        players[tag] = rec


# ---------------------------------------------------------------------------
# per-country sub-blocks (country_extra.json)
# ---------------------------------------------------------------------------

def _wrap_i64(value):
    value &= (1 << 64) - 1
    return value - (1 << 64) if value >= (1 << 63) else value


def _trunc_div(value, divisor):
    quotient = abs(value) // abs(divisor)
    return -quotient if (value < 0) != (divisor < 0) else quotient


def _a729_nuke_built_count(amount_raw):
    """Byte-equivalent ordinary/sentinel interpretation from FUN_1410828a0."""
    value_u64 = amount_raw & ((1 << 64) - 1)
    condition = ((value_u64 + 0x7ffffffffffffffe) & ((1 << 64) - 1)) < 0xfffffffffffffffd
    if not condition:
        return 0xa7c5
    return _trunc_div(_wrap_i64(amount_raw * 100_000), 10_000_000_000_000)


def _parse_nukes(tz):
    """nukes={ { amount nukes_ready } { amount nukes_ready } ... } anon blocks."""
    rows = []
    depth = 1
    while depth > 0:
        tok = tz.next()
        if tok is None:
            break
        if tok is RBRACE:
            depth -= 1
            continue
        if tok is LBRACE:
            amount = ready = None
            for k, op in iter_block_entries(tz):
                if op is LBRACE:
                    _skip_block_body(tz)
                    continue
                v = atom_value(op)
                if v is None:
                    continue
                if k == "amount":
                    amount = to_int(v)
                elif k == "nukes_ready":
                    ready = to_int(v)
            rows.append({
                "amount": None if amount is None else _a729_nuke_built_count(amount),
                "amount_raw": amount,
                "nukes_ready": ready,
            })
    return rows


def _parse_dynamic_modifier(tz):
    """dynamic_modifier={ modifier={ modifier="x" value={..} enabled=yes } ... }."""
    out = []
    for key, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        if key != "modifier":
            _skip_block_body(tz)
            continue
        # `days` is the save-persisted remaining-duration leaf for timed
        # instances. `tag` is the optional source/target tag emitted beside it.
        # Keep both literal values; absence is distinct from a permanent
        # modifier whose value vector happens to be empty.
        rec = {"modifier": None, "enabled": None, "value": [],
               "days": None, "tag": None}
        for k, op in iter_block_entries(tz):
            if op is LBRACE:
                if k == "value":
                    rec["value"] = [to_float(a) for a in read_scalar_list(tz)]
                else:
                    _skip_block_body(tz)
                continue
            v = atom_value(op)
            if v is None:
                continue
            if k == "modifier":
                rec["modifier"] = v
            elif k == "enabled":
                rec["enabled"] = (v == "yes")
            elif k == "days":
                rec["days"] = to_int(v)
            elif k == "tag":
                rec["tag"] = v
        out.append(rec)
    return out


def _parse_flags(tz):
    """flags={ <flag>={ value date } ... } -> {flag: {value, date}}.

    Country flags are causal scripted state, so keep only the literal flags that
    exist in the save. A country with no ``flags`` block does not get an inferred
    empty bag.
    """
    flags = {}
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            value = date = None
            for k, op in iter_block_entries(tz):
                if op is LBRACE:
                    _skip_block_body(tz)
                    continue
                vv = atom_value(op)
                if vv is None:
                    continue
                if k == "value":
                    value = to_int(vv)
                elif k == "date":
                    date = vv
            flags[key] = {"value": value, "date": date}
    return flags


def _coerce_raw_atom(value):
    """Coerce generic Clausewitz atoms without turning the game dates into floats."""
    if value == "yes":
        return True
    if value == "no":
        return False
    if value and value.lstrip("-").isdigit():
        return int(value)
    try:
        # Dates like 1940.9.10.13 must remain strings; float() rejects them.
        return float(value)
    except (TypeError, ValueError):
        return value


def _put_repeated(out, repeated, key, value):
    if key not in out:
        out[key] = value
        return
    if key not in repeated:
        out[key] = [out[key], value]
        repeated.add(key)
        return
    out[key].append(value)


def _parse_raw_block(tz):
    """Shape-preserving reader for mixed Clausewitz blocks.

    ``iter_block_entries`` intentionally skips anonymous child blocks; strategic
    navy uses keyed/hash-map blocks and can also carry anonymous lists, so read
    this one with the raw tokenizer and preserve repeated keys as lists.
    """
    named = {}
    repeated = set()
    items = []
    while True:
        tok = tz.next()
        if tok is None or tok is RBRACE:
            break
        if tok is LBRACE:
            items.append(_parse_raw_block(tz))
            continue
        key = atom_value(tok)
        if key is None:
            continue
        if tz.peek() is EQUALS:
            tz.next()
            valtok = tz.next()
            if valtok is LBRACE:
                value = _parse_raw_block(tz)
            else:
                value = _coerce_raw_atom(atom_value(valtok))
            _put_repeated(named, repeated, key, value)
        else:
            items.append(_coerce_raw_atom(key))
    if named and items:
        named["_items"] = items
        return named
    if named:
        return named
    if (items and any(isinstance(item, dict) for item in items)
            and not all(isinstance(item, dict) for item in items)):
        return {"_items": items}
    return items


def _parse_strategic_navy(tz):
    """Read countries.<TAG>.strategic_navy as save-persisted data.

    This intentionally does not infer live memory offsets or recompute dockyard
    caps. It only exposes what the melted save serialized.
    """
    return _parse_raw_block(tz)


def _parse_ace(tz):
    rec = {"name": None, "surname": None, "callsign": None,
           "modifier": None, "alive": None, "kill_type": None,
           "killer_country": None}
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            _skip_block_body(tz)
            continue
        v = atom_value(op)
        if v is None:
            continue
        if k == "name":
            rec["name"] = v
        elif k == "surname":
            rec["surname"] = v
        elif k == "callsign":
            rec["callsign"] = v
        elif k == "modifier":
            rec["modifier"] = v
        elif k == "alive":
            rec["alive"] = (v == "yes")
        elif k == "kill_type":
            rec["kill_type"] = to_int(v)
        elif k == "killer_country":
            rec["killer_country"] = v
    return rec


def _parse_program_status(tz):
    """program_status={ project_pool={project={<name>={start completed}}}
       program={...} breakthrough={specialization_*=N} }."""
    started = completed = 0
    programs = []
    breakthrough = {}
    for key, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        if key == "project_pool":
            s, c = _count_projects(tz)
            started += s
            completed += c
        elif key == "program":
            programs.append(_parse_program(tz))
        elif key == "breakthrough":
            for k, op in iter_block_entries(tz):
                if op is LBRACE:
                    _skip_block_body(tz)
                    continue
                v = atom_value(op)
                if v is not None:
                    breakthrough[k] = to_int(v)
        else:
            _skip_block_body(tz)
    return {"projects_started": started, "projects_completed": completed,
            "active_programs": programs, "breakthrough": breakthrough}


def _count_projects(tz):
    """project_pool={ project={ <name>={ ... start=yes completed=yes } ... } }."""
    started = completed = 0
    for key, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        if key != "project":
            _skip_block_body(tz)
            continue
        # project={ <proj_name>={ ... } ... }
        for pname, op in iter_block_entries(tz):
            if op is not LBRACE:
                continue
            s = c = False
            for k, o2 in iter_block_entries(tz):
                if o2 is LBRACE:
                    _skip_block_body(tz)
                    continue
                v = atom_value(o2)
                if v is None:
                    continue
                if k == "start" and v == "yes":
                    s = True
                elif k == "completed" and v == "yes":
                    c = True
            started += s
            completed += c
    return started, completed


def _parse_program(tz):
    rec = {"building": None, "province": None, "country": None}
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            _skip_block_body(tz)
            continue
        v = atom_value(op)
        if v is None:
            continue
        if k == "building":
            rec["building"] = v
        elif k == "province":
            rec["province"] = to_int(v)
        elif k == "country":
            rec["country"] = v
    return rec


def _parse_country_block(tz):
    rec = {
        "nukes": [], "dynamic_modifiers": [], "aces": [],
        "reinforcement_priority": None, "deployment_blocks": [],
        "program_status": None,
        "experience_status": None,
        "script_flags": {}, "_script_flags_present": False,
        "strategic_navy": None, "_strategic_navy_present": False,
        "convoys_destroyed": None, "accidents_score": None,
        "preferred_tactic": None,
        "cosmetic_tag": None, "pride_of_the_fleet": None,
        "collaboration_present": False,
    }
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "nukes":
                rec["nukes"] = _parse_nukes(tz)
            elif key == "dynamic_modifier":
                rec["dynamic_modifiers"] = _parse_dynamic_modifier(tz)
            elif key == "ace":
                # a country can carry MANY ace blocks (repeated key)
                rec["aces"].append(_parse_ace(tz))
            elif key == "reinforcement":
                for k, op in iter_block_entries(tz):
                    if op is LBRACE:
                        _skip_block_body(tz)
                    elif k == "priority":
                        rec["reinforcement_priority"] = to_int(atom_value(op))
            elif key == "deployment":
                blocks = []
                for k, op in iter_block_entries(tz):
                    if op is LBRACE:
                        blocks.append(k)
                        _skip_block_body(tz)
                rec["deployment_blocks"] = blocks
            elif key == "program_status":
                rec["program_status"] = _parse_program_status(tz)
            elif key == "experience_status":
                es = {}
                for k, op in iter_block_entries(tz):
                    if op is LBRACE:
                        _skip_block_body(tz)
                        continue
                    v = atom_value(op)
                    if v is not None and k in (
                            "army_experience", "navy_experience",
                            "air_experience"):
                        es[k] = to_float(v)
                rec["experience_status"] = es
            elif key == "flags":
                rec["script_flags"] = _parse_flags(tz)
                rec["_script_flags_present"] = True
            elif key == "strategic_navy":
                rec["strategic_navy"] = _parse_strategic_navy(tz)
                rec["_strategic_navy_present"] = True
            elif key == "pride_of_the_fleet":
                pid, ptype = read_id_block(tz)
                rec["pride_of_the_fleet"] = {"id": pid, "type": ptype}
            elif key == "collaboration":
                # collaboration={ } when empty; mark present if it has children
                n = 0
                for k, op in iter_block_entries(tz):
                    if op is LBRACE:
                        _skip_block_body(tz)
                    n += 1
                rec["collaboration_present"] = n > 0
            else:
                _skip_block_body(tz)
            continue
        v = atom_value(opener)
        if v is None:
            continue
        if key == "convoys_destroyed":
            rec["convoys_destroyed"] = to_int(v)
        elif key == "accidents_score":
            rec["accidents_score"] = to_float(v)
        elif key == "cosmetic_tag":
            rec["cosmetic_tag"] = v
        elif key == "preferred_tactic":
            rec["preferred_tactic"] = to_int(v)
    return rec


def _parse_countries(tz, by_tag):
    for tag, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        by_tag[tag] = _parse_country_block(tz)


# ---------------------------------------------------------------------------
# build
# ---------------------------------------------------------------------------

def build(text, source, was_melted):
    by_tag = {}
    players = {}
    meta = dc.top_level_pass(
        text,
        {
            "countries": lambda tz: _parse_countries(tz, by_tag),
            "player_countries": lambda tz: _parse_player_countries(tz, players),
        },
        scalar_keys=("date", "version", "player"),
    )

    # Roll up convenient scalars.
    countries = {}
    for tag, rec in sorted(by_tag.items()):
        nukes_total = sum((n["amount"] or 0) for n in rec["nukes"])
        nukes_ready = sum((n["nukes_ready"] or 0) for n in rec["nukes"])
        ps = rec["program_status"] or {}
        country = {
            "nukes_total": nukes_total,
            "nukes_ready_total": nukes_ready,
            "nukes": rec["nukes"],
            "dynamic_modifiers": rec["dynamic_modifiers"],
            "ace_count": len(rec["aces"]),
            "aces": rec["aces"],
            "reinforcement_priority": rec["reinforcement_priority"],
            "deployment_blocks": rec["deployment_blocks"],
            "special_projects": {
                "projects_started": ps.get("projects_started"),
                "projects_completed": ps.get("projects_completed"),
                "active_programs": ps.get("active_programs", []),
                "breakthrough": ps.get("breakthrough", {}),
            },
            "experience_status": rec["experience_status"],
            "convoys_destroyed": rec["convoys_destroyed"],
            "accidents_score": rec["accidents_score"],
            "cosmetic_tag": rec["cosmetic_tag"],
            "preferred_tactic": rec["preferred_tactic"],
            "pride_of_the_fleet": rec["pride_of_the_fleet"],
            "collaboration_present": rec["collaboration_present"],
        }
        if rec["_script_flags_present"]:
            country["script_flags"] = dict(sorted(rec["script_flags"].items()))
        if rec["_strategic_navy_present"]:
            country["strategic_navy"] = rec["strategic_navy"]
        countries[tag] = country

    base_meta = {
        "source_filename": source.name,
        "source_path_redacted": dc.ps._redact(source),
        "was_melted": was_melted,
        "save_date": meta.get("date"),
        "save_version": meta.get("version"),
        "player": meta.get("player"),
    }

    extra_out = {
        "meta": {**base_meta, "stats": {
            "countries": len(countries),
            "countries_with_nukes": sum(
                1 for c in countries.values() if c["nukes_total"]),
            "countries_with_ace": sum(
                1 for c in countries.values() if c["aces"]),
            "countries_with_dynamic_modifiers": sum(
                1 for c in countries.values() if c["dynamic_modifiers"]),
            "countries_with_active_programs": sum(
                1 for c in countries.values()
                if c["special_projects"]["active_programs"]),
            "countries_with_script_flags_block": sum(
                1 for c in countries.values() if "script_flags" in c),
            "script_flags": sum(
                len(c.get("script_flags") or {}) for c in countries.values()),
            "countries_with_strategic_navy": sum(
                1 for c in countries.values() if "strategic_navy" in c),
        }},
        "countries": countries,
    }

    players_out = {
        "meta": {**base_meta, "stats": {
            "player_countries": len(players),
            "named_players": sum(1 for p in players.values() if p["user"]),
        }, "notes": {
            "user": "player_countries.<TAG>.user = lobby/persona display name "
            "(sparse in SP saves, populated in MP). True Steam ID stays a wall.",
        }},
        "players": players,
    }

    # Validation report.
    ger = countries.get("GER", {})
    lines = ["=== COUNTRY-EXTRA + PLAYERS ORACLE VALIDATION ==="]
    lines.append(f"source        : {source.name} (melted={was_melted})")
    lines.append(f"save date     : {meta.get('date')}")
    lines.append(f"countries     : {len(countries)}")
    lines.append(f"players       : {len(players)} player_countries, "
                 f"named={extra_out['meta']['stats'] and players_out['meta']['stats']['named_players']}")
    for tag, p in players.items():
        lines.append(f"  player {tag}: user={p['user']!r} "
                     f"leader={p['country_leader']} id={p['id']}")
    lines.append(f"GER nukes     : total={ger.get('nukes_total')} "
                 f"ready={ger.get('nukes_ready_total')} "
                 f"(records={len(ger.get('nukes', []))})")
    lines.append(f"GER dyn-mods  : {len(ger.get('dynamic_modifiers', []))} "
                 f"(e.g. {[m['modifier'] for m in ger.get('dynamic_modifiers', [])][:2]})")
    aces = ger.get("aces", [])
    a0 = aces[0] if aces else None
    lines.append(f"GER aces      : {len(aces)} "
                 f"(first={a0.get('name') if a0 else None} "
                 f"{a0.get('callsign') if a0 else ''} "
                 f"alive={a0.get('alive') if a0 else None})")
    lines.append(f"GER reinforce : priority={ger.get('reinforcement_priority')}")
    lines.append(f"GER deploy    : {ger.get('deployment_blocks')}")
    sp = ger.get("special_projects", {})
    lines.append(f"GER projects  : started={sp.get('projects_started')} "
                 f"completed={sp.get('projects_completed')} "
                 f"programs={len(sp.get('active_programs', []))} "
                 f"breakthrough={sp.get('breakthrough')}")
    lines.append(f"GER exp       : {ger.get('experience_status')}")
    lines.append(f"GER flags     : {len(ger.get('script_flags', {}))} "
                 f"(e.g. {list(ger.get('script_flags', {}))[:3]})")
    sn = ger.get("strategic_navy") or {}
    lines.append(f"GER strat-navy: present={'strategic_navy' in ger} "
                 f"dockyards={sn.get('max_allowed_dockyards_to_repair')}")
    lines.append(f"GER convoys_destroyed={ger.get('convoys_destroyed')} "
                 f"accidents={ger.get('accidents_score')} "
                 f"pride_of_fleet={ger.get('pride_of_the_fleet')}")
    nuke_tags = [t for t, c in countries.items() if c["nukes_total"]]
    lines.append(f"tags w/ nukes : {nuke_tags[:10]}")
    return {"country_extra.json": extra_out, "players.json": players_out}, \
        "\n".join(lines)


if __name__ == "__main__":
    raise SystemExit(dc.run_cli(
        "country_extra",
        "the game save -> per-country nukes/dyn-mod/ace/deploy/projects + players.",
        build, ["country_extra.json", "players.json"]))
