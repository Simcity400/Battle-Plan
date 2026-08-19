#!/usr/bin/env python3
"""the game save -> CHARACTERS & COMMANDERS oracle (``characters.json``).

Deep-layer extension of the save oracle. Reconstructs the game 1.19 CHARACTER
system: every country's leader, military leaders (generals = corps_commander,
field marshals = field_marshal, admirals = navy_leader), advisors/cabinet/high
command/theorists, and which army-group / army / fleet each military leader is
ASSIGNED to command.

WHY characters, not legacy leaders:
  the game 1.19 unified leaders + advisors + ministers + scientists into a single
  ``character`` object. Full definitions live ONCE, top-level, in::

      character_manager = { historical = { character = { ... } ... } }

  and each country carries a *roster* of (id,type) handles + active role flags::

      countries.<TAG>.characters = { character_status = {
          character={id type} country_leader=yes/no advisor=yes/no
          unit_leader=yes/no scientist=yes/no } ... }

Character definition (``character_manager.historical.character``):
  id={id type=73}  token  template  name  country  gender  portraits
  -- plus ONE OR MORE role sub-blocks (a character can be several things) --
  country_leaders = { country_leader = { ideology traits expire } }
  advisors        = { advisor = { slot ledger idea_token political_power
                                  traits cost ... } ... }
  field_marshal / corps_commander / navy_leader = {
      id={id type=4713}   <- the UNIT-LEADER id (assignment key)
      name gfx female skill script_id traits in_progress
      attack_skill defense_skill planning_skill logistics_skill   (army)
      attack_skill defense_skill maneuvering_skill coordination_skill (navy)
      preferred_tactic penalty trait_xp_factor }

  ``skill`` is the leader's overall LEVEL; the four *_skill values are the
  attack/defense/planning/logistics (or naval) stat lines shown in-game.

Active country leader (which of the 4 ideology leaders is in charge):
  countries.<TAG>.politics = {
      parties = { <group> = { popularity country_leader = { {
          ideology character={id type=73} } } } ... }
      ruling_party = <group> }
  The ACTIVE leader is parties[ruling_party].country_leader.character; the other
  three groups' leaders are kept as ``party_leaders`` for the political picture.
  (A character carries one ``country_leaders`` entry per ideology it can lead, so
  the per-country roster flags ALL of them country_leader=yes — politics.parties
  is the authoritative disambiguator, NOT the roster.)

Command assignment (general/FM/admiral -> the formation they lead):
  countries.<TAG>.theatres.theatre.field_marshal_group = {
      id name leader={id type=4713} ... }          (FM  -> army group)
  countries.<TAG>.theatres.theatre.orders_group     = {
      id name leader={id type=4713} ... }          (general -> army)
  countries.<TAG>.units.fleet                       = {
      id leader={id type=4713} name ... }           (admiral -> fleet)
  The ``leader.id`` (type 4713) matches the military role sub-block's own
  ``id.id`` in character_manager, so we join unit-leader-id -> character.

PORTRAITS: a character's ``gfx`` / ``portraits={civilian|army={small|large=
  "GFX_portrait_..."}}`` are GFX sprite *names*, not image data. Rendering the
  actual portrait art is a SEPARATE asset-extraction step (the game's
  gfx/leaders + gfx/interface portrait sprites must be unpacked and mapped from
  these GFX_ keys). This parser records the sprite NAME so the replay can later
  resolve it; FLAGGED to the orchestrator as an asset-extraction need.

stdlib only. Reuses the shared tokenizer via ``deep_common`` (read-only).
"""

from __future__ import annotations

import deep_common as dc
from deep_common import (
    LBRACE, RBRACE, atom_value, iter_block_entries, _skip_block_body,
    to_int, to_float, read_scalar_list,
)

MIL_ROLE_KEYS = {
    "corps_commander": "general",
    "field_marshal": "field_marshal",
    "navy_leader": "admiral",
}



def _read_id(tz):
    _id = _type = None
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            _skip_block_body(tz)
            continue
        v = atom_value(op)
        if v is None:
            continue
        if k == "id":
            _id = to_int(v)
        elif k == "type":
            _type = to_int(v)
    return _id, _type



def _parse_portraits(tz):
    """portraits={ civilian|army|navy={ small|large="GFX_..." } ... } -> dict."""
    out = {}
    for category, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        for size, op in iter_block_entries(tz):
            if op is LBRACE:
                _skip_block_body(tz)
                continue
            v = atom_value(op)
            if v is not None:
                out[f"{category}_{size}"] = v
    return out


def _parse_country_leader(tz):
    rec = {"ideology": None, "traits": [], "expire": None}
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "traits":
                rec["traits"] = read_scalar_list(tz)
            else:
                _skip_block_body(tz)
            continue
        v = atom_value(op)
        if v is None:
            continue
        if k == "ideology":
            rec["ideology"] = v
        elif k == "expire":
            rec["expire"] = v
    return rec


def _parse_country_leaders(tz):
    """country_leaders={ country_leader={...} ... } -> list (often 1)."""
    out = []
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "country_leader":
                out.append(_parse_country_leader(tz))
            else:
                _skip_block_body(tz)
    return out


def _parse_advisor(tz):
    rec = {"slot": None, "ledger": None, "idea_token": None,
           "political_power": None, "cost": None, "traits": []}
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "traits":
                rec["traits"] = read_scalar_list(tz)
            else:
                _skip_block_body(tz)
            continue
        v = atom_value(op)
        if v is None:
            continue
        if k == "slot":
            rec["slot"] = v
        elif k == "ledger":
            rec["ledger"] = v
        elif k == "idea_token":
            rec["idea_token"] = v
        elif k == "political_power":
            rec["political_power"] = to_float(v)
        elif k == "cost":
            rec["cost"] = to_float(v)
    return rec


def _parse_advisors(tz):
    out = []
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "advisor":
                out.append(_parse_advisor(tz))
            else:
                _skip_block_body(tz)
    return out


def _parse_mil_role(tz, role_label):
    """A corps_commander / field_marshal / navy_leader sub-block."""
    rec = {
        "role": role_label,
        "unit_leader_id": None, "unit_leader_type": None,
        "level": None,
        "experience": None,
        "attack": None, "defense": None,
        "planning": None, "logistics": None,
        "maneuvering": None, "coordination": None,
        "traits": [], "script_id": None,
        "preferred_tactic": None, "penalty": None,
        "captured": False, "captured_by": None, "location": None,
        "assignment": None,
    }
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "id":
                rec["unit_leader_id"], rec["unit_leader_type"] = _read_id(tz)
            elif k == "traits":
                rec["traits"] = read_scalar_list(tz)
            else:
                _skip_block_body(tz)
            continue
        v = atom_value(op)
        if v is None:
            continue
        if k == "skill":
            rec["level"] = to_int(v)
        elif k == "experience":
            rec["experience"] = to_float(v)
        elif k == "attack_skill":
            rec["attack"] = to_int(v)
        elif k == "defense_skill":
            rec["defense"] = to_int(v)
        elif k == "planning_skill":
            rec["planning"] = to_int(v)
        elif k == "logistics_skill":
            rec["logistics"] = to_int(v)
        elif k == "maneuvering_skill":
            rec["maneuvering"] = to_int(v)
        elif k == "coordination_skill":
            rec["coordination"] = to_int(v)
        elif k == "script_id":
            rec["script_id"] = to_int(v)
        elif k == "preferred_tactic":
            rec["preferred_tactic"] = to_int(v)
        elif k == "penalty":
            rec["penalty"] = to_int(v)
        elif k == "captured":
            rec["captured"] = (v == "yes")
        elif k == "captured_by":
            rec["captured_by"] = v
        elif k == "location":
            rec["location"] = to_int(v)
    return rec


def _parse_character(tz):
    """One ``character={...}`` def. Returns the record (or None if no id)."""
    rec = {
        "char_id": None, "char_type": None,
        "token": None, "name": None, "country": None, "gender": None,
        "gfx": None, "portraits": {},
        "country_leader": None,
        "advisor_roles": [],
        "military": None,
    }
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "id":
                rec["char_id"], rec["char_type"] = _read_id(tz)
            elif k == "portraits":
                rec["portraits"] = _parse_portraits(tz)
            elif k == "country_leaders":
                cls = _parse_country_leaders(tz)
                if cls:
                    rec["country_leader"] = cls
            elif k == "advisors":
                rec["advisor_roles"] = _parse_advisors(tz)
            elif k in MIL_ROLE_KEYS:
                rec["military"] = _parse_mil_role(tz, MIL_ROLE_KEYS[k])
            else:
                _skip_block_body(tz)
            continue
        v = atom_value(op)
        if v is None:
            continue
        if k == "token":
            rec["token"] = v
        elif k == "name":
            rec["name"] = v
        elif k == "country":
            rec["country"] = v
        elif k == "gender":
            rec["gender"] = v
        elif k == "gfx":
            rec["gfx"] = v
    return rec


def _parse_character_manager(tz, chars_by_key):
    """character_manager={ historical={ character={...} } dynamic={ ... } }.

    BOTH the ``historical`` (scenario-authored) and ``dynamic`` (runtime-
    generated, e.g. promoted officers / event leaders) character pools must be
    read — per-country rosters reference characters from either.
    """
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k in ("historical", "dynamic"):
                for kk, oo in iter_block_entries(tz):
                    if oo is LBRACE:
                        if kk == "character":
                            rec = _parse_character(tz)
                            if rec["char_id"] is not None:
                                chars_by_key[(rec["char_id"], rec["char_type"])] = rec
                        else:
                            _skip_block_body(tz)
            else:
                _skip_block_body(tz)



def _parse_characters_roster(tz):
    """countries.<TAG>.characters={ character_status={...} ... } -> list.

    Each status = {char_key:(id,type), country_leader, advisor, unit_leader,
    scientist}.
    """
    out = []
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "character_status":
                st = {"char_id": None, "char_type": None,
                      "is_country_leader": False, "is_advisor": False,
                      "is_unit_leader": False, "is_scientist": False}
                for kk, oo in iter_block_entries(tz):
                    if oo is LBRACE:
                        if kk == "character":
                            st["char_id"], st["char_type"] = _read_id(tz)
                        else:
                            _skip_block_body(tz)
                        continue
                    v = atom_value(oo)
                    if v is None:
                        continue
                    if kk == "country_leader":
                        st["is_country_leader"] = (v == "yes")
                    elif kk == "advisor":
                        st["is_advisor"] = (v == "yes")
                    elif kk == "unit_leader":
                        st["is_unit_leader"] = (v == "yes")
                    elif kk == "scientist":
                        st["is_scientist"] = (v == "yes")
                if st["char_id"] is not None:
                    out.append(st)
            else:
                _skip_block_body(tz)
    return out


def _read_group_leader_name(tz):
    """Inside a group block ({ already consumed): return (name, leader_ul_id).

    Used for field_marshal_group / orders_group / fleet. We read the block's
    direct ``name`` and ``leader={id type}`` and skip everything else.
    """
    name = None
    leader_id = None
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "leader":
                lid, _lt = _read_id(tz)
                if leader_id is None:
                    leader_id = lid
            else:
                _skip_block_body(tz)
            continue
        v = atom_value(op)
        if v is None:
            continue
        if k == "name" and name is None:
            name = v
    return name, leader_id


def _parse_theatre(tz, assignments):
    """One theatre block: harvest field_marshal_group / orders_group leaders."""
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "field_marshal_group":
                name, lid = _read_group_leader_name(tz)
                if lid is not None:
                    assignments[lid] = {"kind": "army_group", "name": name}
            elif k == "orders_group":
                name, lid = _read_group_leader_name(tz)
                if lid is not None:
                    assignments.setdefault(
                        lid, {"kind": "army", "name": name})
            else:
                _skip_block_body(tz)


def _parse_theatres(tz, assignments):
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "theatre":
                _parse_theatre(tz, assignments)
            else:
                _skip_block_body(tz)


def _parse_units_for_fleets(tz, assignments):
    """The country ``units={ division={..} fleet={..} }`` block.

    Divisions are handled by parse_units.py; here we only harvest each
    ``fleet={ id leader={id type} name ... }`` -> admiral command.
    """
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "fleet":
                name, lid = _read_group_leader_name(tz)
                if lid is not None:
                    assignments.setdefault(
                        lid, {"kind": "fleet", "name": name})
            else:
                _skip_block_body(tz)


def _read_anon_block(tz):
    """Read one anonymous ``{ ... }`` block (LBRACE already consumed).

    ``iter_block_entries`` auto-SKIPS anonymous nested blocks, so for the
    ``country_leader={ { ideology character={..} } }`` shape (the leader entry is
    an anonymous inner object) we descend manually. Returns ``(ideology,
    char_id, char_type)`` for that inner object.
    """
    ideology = char_id = char_type = None
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "character":
                char_id, char_type = _read_id(tz)
            else:
                _skip_block_body(tz)
            continue
        v = atom_value(op)
        if k == "ideology" and v is not None:
            ideology = v
    return ideology, char_id, char_type


def _parse_country_leader_ref(tz):
    """country_leader={ { ideology character={id type} } } (LBRACE consumed).

    The leadership entry is an ANONYMOUS inner block, so we hand-roll the
    descent (iter_block_entries would skip it). Returns (ideology, id, type).
    """
    ideology = char_id = char_type = None
    while True:
        tok = tz.next()
        if tok is None or tok is RBRACE:
            break
        if tok is LBRACE:
            ideology, char_id, char_type = _read_anon_block(tz)
    return ideology, char_id, char_type


def _parse_politics(tz):
    """politics={ parties={ <group>={ popularity country_leader={ {ideology
    character={id type}} } } } ruling_party=<group> ... }.

    Returns ``{"ruling_party": grp, "parties": {grp: {ideology, char_id,
    char_type, popularity}}}``.
    """
    out = {"ruling_party": None, "parties": {}}
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "parties":
                for grp, gop in iter_block_entries(tz):
                    if gop is not LBRACE:
                        continue
                    p = {"ideology": None, "char_id": None,
                         "char_type": None, "popularity": None}
                    for kk, oo in iter_block_entries(tz):
                        if oo is LBRACE:
                            if kk == "country_leader":
                                ideo, cid, ct = _parse_country_leader_ref(tz)
                                p["ideology"] = ideo
                                p["char_id"] = cid
                                p["char_type"] = ct
                            else:
                                _skip_block_body(tz)
                            continue
                        v = atom_value(oo)
                        if kk == "popularity" and v is not None:
                            p["popularity"] = to_float(v)
                    out["parties"][grp] = p
            else:
                _skip_block_body(tz)
            continue
        v = atom_value(op)
        if v is None:
            continue
        if k == "ruling_party":
            out["ruling_party"] = v
    return out


def _parse_one_country(tz, tag, rosters, assignments_by_tag, politics_by_tag):
    """Walk a single country block's direct children we care about."""
    roster = []
    assignments = {}
    politics = {"ruling_party": None, "parties": {}}
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "characters":
                roster = _parse_characters_roster(tz)
            elif k == "theatres":
                _parse_theatres(tz, assignments)
            elif k == "units":
                _parse_units_for_fleets(tz, assignments)
            elif k == "politics":
                politics = _parse_politics(tz)
            else:
                _skip_block_body(tz)
    rosters[tag] = roster
    assignments_by_tag[tag] = assignments
    politics_by_tag[tag] = politics


def _parse_countries(tz, rosters, assignments_by_tag, politics_by_tag):
    for tag, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        _parse_one_country(tz, tag, rosters, assignments_by_tag,
                           politics_by_tag)



def parse_characters_text(text: str):
    chars_by_key: dict = {}
    rosters: dict = {}
    assignments_by_tag: dict = {}
    politics_by_tag: dict = {}
    meta = dc.top_level_pass(
        text,
        {
            "character_manager":
                lambda tz: _parse_character_manager(tz, chars_by_key),
            "countries":
                lambda tz: _parse_countries(tz, rosters, assignments_by_tag,
                                            politics_by_tag),
        },
        scalar_keys=("date", "version", "player"),
    )
    return chars_by_key, rosters, assignments_by_tag, politics_by_tag, meta



def _mil_to_out(mil, assignment):
    return {
        "role": mil["role"],
        "level": mil["level"],
        "experience": mil["experience"],
        "attack": mil["attack"],
        "defense": mil["defense"],
        "planning": mil["planning"],
        "logistics": mil["logistics"],
        "maneuvering": mil["maneuvering"],
        "coordination": mil["coordination"],
        "traits": mil["traits"],
        "preferred_tactic": mil["preferred_tactic"],
        "penalty": mil["penalty"],
        "captured": mil["captured"],
        "captured_by": mil["captured_by"],
        "location": mil["location"],
        "unit_leader_id": mil["unit_leader_id"],
        "assignment": assignment,
    }


def _leader_record(chars_by_key, char_id, char_type, ideology):
    """Build a leader output record from a (id,type) char ref + active ideology."""
    cdef = chars_by_key.get((char_id, char_type))
    if cdef is None:
        return None
    traits = []
    expire = None
    for cl in (cdef.get("country_leader") or []):
        if cl["ideology"] == ideology:
            traits, expire = cl["traits"], cl["expire"]
            break
    else:
        if cdef.get("country_leader"):
            cl0 = cdef["country_leader"][0]
            traits, expire = cl0["traits"], cl0["expire"]
    return {
        "name": cdef["name"], "token": cdef["token"],
        "ideology": ideology, "traits": traits, "expire": expire,
        "portrait_gfx": (cdef["portraits"].get("civilian_large")
                         or cdef.get("gfx")),
        "char_id": char_id,
    }


def build(text, source, was_melted):
    (chars_by_key, rosters, assignments_by_tag,
     politics_by_tag, meta) = parse_characters_text(text)

    countries = {}
    total_generals = total_fms = total_admirals = 0
    total_advisors = total_leaders = 0
    assign_resolved = assign_total = 0
    missing_defs = 0

    for tag, roster in sorted(rosters.items()):
        assignments = assignments_by_tag.get(tag, {})
        politics = politics_by_tag.get(tag, {})
        c = {
            "leader": None,
            "party_leaders": [],
            "ruling_party": politics.get("ruling_party"),
            "generals": [], "field_marshals": [], "admirals": [],
            "advisors": [],
            "counts": {},
        }

        ruling = politics.get("ruling_party")
        for grp, p in (politics.get("parties") or {}).items():
            if p.get("char_id") is None:
                continue
            lr = _leader_record(chars_by_key, p["char_id"], p["char_type"],
                                p["ideology"])
            if lr is None:
                continue
            lr_entry = {
                "party": grp, "popularity": p.get("popularity"),
                **lr,
            }
            c["party_leaders"].append(lr_entry)
            if grp == ruling:
                c["leader"] = lr
                total_leaders += 1

        for st in roster:
            key = (st["char_id"], st["char_type"])
            cdef = chars_by_key.get(key)
            if cdef is None:
                missing_defs += 1
                continue

            if st["is_unit_leader"] and cdef.get("military"):
                mil = cdef["military"]
                ulid = mil["unit_leader_id"]
                assignment = assignments.get(ulid)
                assign_total += 1
                if assignment is not None:
                    assign_resolved += 1
                rec = {
                    "name": cdef["name"], "token": cdef["token"],
                    "portrait_gfx": (cdef["portraits"].get("army_large")
                                     or cdef["portraits"].get("navy_large")
                                     or cdef["gfx"]),
                }
                rec.update(_mil_to_out(mil, assignment))
                if mil["role"] == "field_marshal":
                    c["field_marshals"].append(rec); total_fms += 1
                elif mil["role"] == "admiral":
                    c["admirals"].append(rec); total_admirals += 1
                else:
                    c["generals"].append(rec); total_generals += 1

            if st["is_advisor"] and cdef.get("advisor_roles"):
                for adv in cdef["advisor_roles"]:
                    c["advisors"].append({
                        "name": cdef["name"], "token": cdef["token"],
                        "slot": adv["slot"], "ledger": adv["ledger"],
                        "idea_token": adv["idea_token"],
                        "political_power": adv["political_power"],
                        "traits": adv["traits"],
                        "portrait_gfx": (cdef["portraits"].get("civilian_large")
                                         or cdef["portraits"].get("army_small")
                                         or cdef["gfx"]),
                    })
                    total_advisors += 1

        c["counts"] = {
            "generals": len(c["generals"]),
            "field_marshals": len(c["field_marshals"]),
            "admirals": len(c["admirals"]),
            "advisors": len(c["advisors"]),
            "roster_size": len(roster),
        }
        if roster:
            countries[tag] = c

    out = {
        "meta": {
            "source_filename": source.name,
            "source_path_redacted": dc.ps._redact(source),
            "was_melted": was_melted,
            "save_date": meta.get("date"),
            "save_version": meta.get("version"),
            "player": meta.get("player"),
            "notes": {
                "system": "the game 1.19 unified CHARACTER system: full defs in "
                "top-level character_manager.historical; per-country roster in "
                "countries.<TAG>.characters (character_status role flags). "
                "Generals = corps_commander, field marshals = field_marshal, "
                "admirals = navy_leader. 'level' = the leader's `skill`.",
                "assignment": "general/FM/admiral -> formation join via the "
                "military role's unit_leader id (type 4713) matched against "
                "leader={} in theatres' field_marshal_group (army group) / "
                "orders_group (army) and navy fleets. assignment=null means the "
                "leader is unassigned/in reserve (no formation references them).",
                "portraits": "portrait_gfx is a GFX sprite NAME, not image data. "
                "Rendering the actual portrait art is a SEPARATE asset-extraction "
                "step (unpack game gfx/leaders + interface portrait sprites and "
                "map these GFX_ keys). FLAGGED as an asset-extraction need.",
            },
            "stats": {
                "character_defs": len(chars_by_key),
                "countries_with_characters": len(countries),
                "country_leaders": total_leaders,
                "generals": total_generals,
                "field_marshals": total_fms,
                "admirals": total_admirals,
                "advisors": total_advisors,
                "commands_total": assign_total,
                "commands_assigned_resolved": assign_resolved,
                "roster_handles_missing_def": missing_defs,
            },
        },
        "countries": countries,
    }

    st = out["meta"]["stats"]
    ger = countries.get("GER", {})
    lines = ["=== CHARACTERS / COMMANDERS ORACLE VALIDATION ==="]
    lines.append(f"source        : {source.name} (melted={was_melted})")
    lines.append(f"save date     : {meta.get('date')}  "
                 f"version={meta.get('version')}")
    lines.append(f"character defs: {st['character_defs']} "
                 f"(full top-level character_manager defs)")
    lines.append(f"countries     : {st['countries_with_characters']} with "
                 f"characters")
    lines.append(f"country leaders: {st['country_leaders']}")
    lines.append(f"military      : {st['generals']} generals, "
                 f"{st['field_marshals']} field marshals, "
                 f"{st['admirals']} admirals")
    lines.append(f"advisors/cabinet: {st['advisors']}")
    lines.append(f"commands      : {st['commands_assigned_resolved']}/"
                 f"{st['commands_total']} unit-leaders resolved to a formation "
                 f"(rest unassigned/reserve)")
    lines.append(f"missing defs  : {st['roster_handles_missing_def']} roster "
                 f"handles had no character_manager def (expect 0)")
    if ger:
        gc = ger["counts"]
        ldr = ger.get("leader") or {}
        lines.append(f"-- GER spot-check --")
        lines.append(f"   leader: {ldr.get('name')!r} "
                     f"ideology={ldr.get('ideology')} traits={ldr.get('traits')}")
        lines.append(f"   {gc['generals']} gen / {gc['field_marshals']} FM / "
                     f"{gc['admirals']} adm / {gc['advisors']} advisors "
                     f"(roster {gc['roster_size']})")
        for g in (ger.get("field_marshals", []) + ger.get("generals", []))[:4]:
            a = g.get("assignment")
            lines.append(
                f"   {g['role']:<13} {g['name']!r} lvl={g['level']} "
                f"a/d/p/l={g['attack']}/{g['defense']}/{g['planning']}/"
                f"{g['logistics']} cmd={a['name']!r} ({a['kind']})"
                if a else
                f"   {g['role']:<13} {g['name']!r} lvl={g['level']} "
                f"a/d/p/l={g['attack']}/{g['defense']}/{g['planning']}/"
                f"{g['logistics']} cmd=UNASSIGNED")
        for adm in ger.get("admirals", [])[:2]:
            a = adm.get("assignment")
            lines.append(
                f"   admiral       {adm['name']!r} lvl={adm['level']} "
                f"a/d/m/c={adm['attack']}/{adm['defense']}/"
                f"{adm['maneuvering']}/{adm['coordination']} "
                f"cmd={(a or {}).get('name')!r}")
    return {"characters.json": out}, "\n".join(lines)


if __name__ == "__main__":
    raise SystemExit(dc.run_cli(
        "chars", "the game save -> characters & commanders oracle.",
        build, ["characters.json"]))
