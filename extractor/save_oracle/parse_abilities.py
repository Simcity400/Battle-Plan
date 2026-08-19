#!/usr/bin/env python3
"""the game save -> COMMANDER-ABILITIES + ACES + OFFICER-CORPS oracle.

Deep-layer extension of the save oracle. Emits ``abilities.json`` with three
sections, all keyed per country:

  (a) ``commander_abilities`` : the cooldown-gated state carried on each military
      leader (general / field marshal / admiral). See the IMPORTANT note below on
      what the game actually persists here.
  (b) ``aces``                : the per-country flying-ace roster (the air-combat
      ace pilots), read from the country's ``ace={ ... }`` blocks.
  (c) ``officer_corps``       : the country's officer-corps national spirits
      (academy / army / division-command "_spirit" ideas) plus a count of its
      captured generals.

WHY this layer (vs parse_characters.py which already reads commanders):
  parse_characters builds the *roster* (who leads what, skills, traits). This
  layer captures the **time-gated combat state** that sits alongside the roster
  -- the activatable-ability / cooldown picture the in-game ability bar shows
  (interface/abilitylist.gui) -- plus the air aces and the officer-corps spirits,
  none of which parse_characters emits.

------------------------------------------------------------------------------
(a) COMMANDER ABILITIES -- what the save ACTUALLY stores  (verified, two melts)
------------------------------------------------------------------------------
The activatable general abilities (``force_attack`` / ``last_stand`` / the
offensive-defensive stances, defined in common/abilities/*.txt with cost +
``duration``) are a TRANSIENT runtime effect: the engine only keeps an ability
"active" while its duration counts down, and it is NOT serialised as a named
token in the save. Grepping both a 1940 and a 1946 melt for every ability token
(force_attack, last_stand, staff_office_plan, siege_artillery, ...) returns
ZERO hits -- so a static save snapshot carries no "ability X active, N hours
left" rows. (Recovering live ability activations is therefore a MEMORY-READER
concern, not a save-parse one -- flagged to the orchestrator.)

What IS persisted, on each ``corps_commander`` / ``field_marshal`` /
``navy_leader`` role block, is the leader's **cooldown gate** -- the same kind of
cooldown the ability bar respects -- after a reassignment::

    field_marshal = {
        id = { id=3 type=4713 }
        skill = 3  experience = 1.70341  script_id = 517
        cooldown_reason = "reassigned"
        leader_modifier_enable_date  = 1940.9.17.20   # when the penalty lifts
        leader_cooldown_start_date   = 1940.9.2.20     # when it began
        traits = { ... }  ...
    }

So ``commander_abilities`` reports, per military leader that is currently gated:
the unit-leader id, the role, the cooldown reason, the start date and the
enable/expire date (when the leader is fully effective again). Leaders with no
cooldown fields are not gated and are omitted from the per-country list (a count
of total vs gated is kept in stats). ``captured`` leaders are also surfaced here
since capture is the other "leader unavailable" gate.

------------------------------------------------------------------------------
(b) ACES
------------------------------------------------------------------------------
Each country block carries 0..N direct ``ace={ ... }`` children::

    ace = {
        id = { id=12212 type=4713 }
        modifier  = "fighter_good"     # plane TYPE + tier (see below)
        name      = "Gunther"
        surname   = "Strohmann"
        callsign  = "Bubi"
        portrait  = 73
        alive     = no                 # (absent => alive)
        handled   = yes                # death has been processed
        kill_type = 1
        killer_name    = { id type }   # ace that shot this ace down (if any)
        killer_country = "ENG"
    }

KILLS / SPECIALTY -- reality note: the game aces do NOT store a numeric kill count in
the save. An ace's strength is the ``modifier`` token, which encodes BOTH the
plane category and the bonus tier:
    <category>_<tier>   category in {fighter, bomber, support(=CAS/other)}
                        tier     in {good, unique, genius}
e.g. "fighter_genius" = top-tier fighter ace, "support_good" = entry CAS ace.
We split ``modifier`` (on the LAST underscore) into ``ace_category`` + ``tier``
and keep the raw token. ``ace_category`` is the ace's ROLE bucket, NOT a literal
equipment type (``support`` is not an equipment id). The
"abilities" of an ace = this modifier (the bonus it confers to its wing). The
parser records ``alive`` (default True when the field is absent) and, for dead
aces, who/what killed them.

------------------------------------------------------------------------------
(c) OFFICER CORPS
------------------------------------------------------------------------------
The officer corps in the game 1.19 is three idea slots -- academy, army, and
division-command spirits -- all stored in the country's ``politics.ideas={...}``
flat token list (the same list that holds national spirits). They are
recognisable by the ``_spirit`` suffix; ``timed_idea={ idea= days= }`` entries in
politics give any remaining duration. We surface the spirit tokens (split into
the "_spirit" officer-corps subset) plus their timed-duration if any, and a
``captured_generals`` count (military leaders with ``captured=yes`` whose
``captured_by`` is this country -- i.e. enemy generals THIS country holds -- and,
separately, this country's OWN generals that are captured elsewhere).

stdlib only. Reuses the shared tokenizer via ``deep_common`` (read-only). Does
NOT touch parse_characters / parse_save.
"""

from __future__ import annotations

import deep_common as dc
from deep_common import (
    LBRACE, RBRACE, atom_value, iter_block_entries, _skip_block_body,
    to_int, read_scalar_list,
)

MIL_ROLE_KEYS = {
    "corps_commander": "general",
    "field_marshal": "field_marshal",
    "navy_leader": "admiral",
}

ACE_CATEGORIES = ("fighter", "bomber", "support", "cas", "naval")



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



ACE_TIERS = ("good", "unique", "genius")


def _split_modifier(mod):
    """'fighter_genius' -> ('fighter','genius'); tolerant of unknown shapes.

    The modifier token is ``<category>_<tier>``. We split on the LAST underscore
    (Codex-recommended: robust to multi-word categories like a hypothetical
    'heavy_fighter_genius'). ``category`` is the ace's role bucket
    (fighter/bomber/support/...) -- NOT a literal equipment type -- so it is
    surfaced as ``ace_category`` downstream.
    """
    if not mod:
        return None, None
    if "_" in mod:
        cat, tier = mod.rsplit("_", 1)
        if tier.lower() not in ACE_TIERS:
            for c in ACE_CATEGORIES:
                if mod.lower().startswith(c + "_"):
                    return c, mod[len(c) + 1:]
        return cat, tier
    return mod, None


def _parse_ace(tz):
    """One ``ace={...}`` block (LBRACE consumed)."""
    rec = {
        "ace_id": None, "ace_type": None,
        "name": None, "surname": None, "callsign": None,
        "modifier": None, "ace_category": None, "tier": None,
        "portrait": None,
        "alive": True,
        "handled": None,
        "kill_type": None,
        "killer_id": None, "killer_type": None, "killer_country": None,
    }
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "id":
                rec["ace_id"], rec["ace_type"] = _read_id(tz)
            elif k == "killer_name":
                rec["killer_id"], rec["killer_type"] = _read_id(tz)
            else:
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
            rec["ace_category"], rec["tier"] = _split_modifier(v)
        elif k == "portrait":
            rec["portrait"] = to_int(v)
        elif k == "alive":
            rec["alive"] = (v == "yes")
        elif k == "handled":
            rec["handled"] = (v == "yes")
        elif k == "kill_type":
            rec["kill_type"] = to_int(v)
        elif k == "killer_country":
            rec["killer_country"] = v
    return rec



def _parse_mil_cooldown(tz, role_label):
    """A corps_commander / field_marshal / navy_leader block: harvest the
    cooldown-gate + capture state (skip the heavy skill/trait detail that
    parse_characters already owns)."""
    rec = {
        "role": role_label,
        "unit_leader_id": None,
        "cooldown_reason": None,
        "cooldown_start_date": None,
        "modifier_enable_date": None,
        "captured": False, "captured_by": None, "location": None,
    }
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "id":
                rec["unit_leader_id"], _t = _read_id(tz)
            else:
                _skip_block_body(tz)
            continue
        v = atom_value(op)
        if v is None:
            continue
        if k == "cooldown_reason":
            rec["cooldown_reason"] = v
        elif k == "leader_cooldown_start_date":
            rec["cooldown_start_date"] = v
        elif k == "leader_modifier_enable_date":
            rec["modifier_enable_date"] = v
        elif k == "captured":
            rec["captured"] = (v == "yes")
        elif k == "captured_by":
            rec["captured_by"] = v
        elif k == "location":
            rec["location"] = to_int(v)
    return rec


def _is_gated(rec):
    return bool(rec["cooldown_reason"] or rec["captured"])



def _parse_character(tz, cooldowns, captures):
    """One ``character={...}`` def. Records any mil-role cooldown/capture and the
    character's home country (so a captured general can be attributed)."""
    country = None
    mils = []
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k in MIL_ROLE_KEYS:
                mils.append(_parse_mil_cooldown(tz, MIL_ROLE_KEYS[k]))
            else:
                _skip_block_body(tz)
            continue
        v = atom_value(op)
        if v is None:
            continue
        if k == "country":
            country = v
    for m in mils:
        m["country"] = country
        if _is_gated(m):
            cooldowns.append(m)
        if m["captured"]:
            captures.append(m)


def _parse_character_manager(tz, cooldowns, captures):
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k in ("historical", "dynamic"):
                for kk, oo in iter_block_entries(tz):
                    if oo is LBRACE:
                        if kk == "character":
                            _parse_character(tz, cooldowns, captures)
                        else:
                            _skip_block_body(tz)
            else:
                _skip_block_body(tz)



def _parse_politics(tz):
    """politics={ ideas={tok tok ...} timed_idea={idea= days=} ... } -> dict."""
    out = {"ideas": [], "timed": {}}
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "ideas":
                out["ideas"] = read_scalar_list(tz)
            elif k == "timed_idea":
                idea = None
                days = None
                for kk, oo in iter_block_entries(tz):
                    if oo is LBRACE:
                        _skip_block_body(tz)
                        continue
                    v = atom_value(oo)
                    if v is None:
                        continue
                    if kk == "idea":
                        idea = v
                    elif kk == "days":
                        days = to_int(v)
                if idea is not None:
                    out["timed"][idea] = days
            else:
                _skip_block_body(tz)
    return out



def _parse_one_country(tz, tag, aces_by_tag, politics_by_tag):
    aces = []
    politics = {"ideas": [], "timed": {}}
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "ace":
                aces.append(_parse_ace(tz))
            elif k == "politics":
                politics = _parse_politics(tz)
            else:
                _skip_block_body(tz)
    if aces:
        aces_by_tag[tag] = aces
    politics_by_tag[tag] = politics


def _parse_countries(tz, aces_by_tag, politics_by_tag):
    for tag, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        _parse_one_country(tz, tag, aces_by_tag, politics_by_tag)



def parse_abilities_text(text: str):
    aces_by_tag: dict = {}
    politics_by_tag: dict = {}
    cooldowns: list = []
    captures: list = []
    meta = dc.top_level_pass(
        text,
        {
            "character_manager":
                lambda tz: _parse_character_manager(tz, cooldowns, captures),
            "countries":
                lambda tz: _parse_countries(tz, aces_by_tag, politics_by_tag),
        },
        scalar_keys=("date", "version", "player"),
    )
    return aces_by_tag, politics_by_tag, cooldowns, captures, meta


def _officer_corps_spirits(ideas, timed):
    """The officer-corps subset of a country's idea list = the '_spirit' ideas.

    Returns a list of {token, days_remaining} (days_remaining=None => permanent).
    """
    out = []
    for tok in ideas:
        if tok.endswith("_spirit"):
            out.append({"token": tok, "days_remaining": timed.get(tok)})
    return out


def build(text, source, was_melted):
    (aces_by_tag, politics_by_tag,
     cooldowns, captures, meta) = parse_abilities_text(text)

    commander_abilities = {}
    for m in cooldowns:
        tag = m.get("country") or "?"
        commander_abilities.setdefault(tag, []).append({
            "unit_leader_id": m["unit_leader_id"],
            "role": m["role"],
            "cooldown_reason": m["cooldown_reason"],
            "cooldown_start_date": m["cooldown_start_date"],
            "modifier_enable_date": m["modifier_enable_date"],
            "captured": m["captured"],
            "captured_by": m["captured_by"],
            "location": m["location"],
        })

    held_by = {}
    own_lost = {}
    for m in captures:
        if m.get("captured_by"):
            held_by[m["captured_by"]] = held_by.get(m["captured_by"], 0) + 1
        if m.get("country"):
            own_lost[m["country"]] = own_lost.get(m["country"], 0) + 1

    officer_corps = {}
    all_tags = set(politics_by_tag) | set(held_by) | set(own_lost)
    for tag in all_tags:
        pol = politics_by_tag.get(tag, {"ideas": [], "timed": {}})
        spirits = _officer_corps_spirits(pol["ideas"], pol["timed"])
        rec = {
            "officer_corps_spirits": spirits,
            "spirit_count": len(spirits),
            "captured_enemy_generals": held_by.get(tag, 0),
            "own_generals_captured": own_lost.get(tag, 0),
        }
        if spirits or rec["captured_enemy_generals"] or rec["own_generals_captured"]:
            officer_corps[tag] = rec

    aces_out = {}
    total_aces = 0
    aces_alive = 0
    for tag, aces in aces_by_tag.items():
        lst = []
        for a in aces:
            lst.append({
                "ace_id": a["ace_id"],
                "name": a["name"], "surname": a["surname"],
                "callsign": a["callsign"],
                "modifier": a["modifier"],
                "ace_category": a["ace_category"], "tier": a["tier"],
                "portrait": a["portrait"],
                "alive": a["alive"],
                "handled": a["handled"],
                "kill_type": a["kill_type"],
                "killer_id": a["killer_id"],
                "killer_type": a["killer_type"],
                "killer_country": a["killer_country"],
            })
            total_aces += 1
            if a["alive"]:
                aces_alive += 1
        aces_out[tag] = {"aces": lst, "count": len(lst)}

    total_gated = sum(len(v) for v in commander_abilities.values())
    total_captured = len(captures)

    out = {
        "meta": {
            "source_filename": source.name,
            "source_path_redacted": dc.ps._redact(source),
            "was_melted": was_melted,
            "save_date": meta.get("date"),
            "save_version": meta.get("version"),
            "player": meta.get("player"),
            "notes": {
                "commander_abilities": "Activatable general abilities "
                "(force_attack/last_stand/offensive-defensive stances, defined in "
                "common/abilities/*.txt) are a TRANSIENT runtime effect and are "
                "NOT serialised as named tokens in the save (verified: zero ability "
                "tokens across a 1940 and a 1946 melt). What the save DOES persist "
                "on each military-leader role block is the cooldown GATE "
                "(cooldown_reason / leader_cooldown_start_date / "
                "leader_modifier_enable_date) plus capture state. This section "
                "reports those gated leaders. Live ability-bar activations are a "
                "MEMORY-READER concern (FLAGGED).",
                "aces": "the game aces store no numeric kill count; an ace's strength "
                "is its `modifier` token = <ace_category>_<tier> (e.g. "
                "fighter_genius), split on the LAST underscore. ace_category in "
                "{fighter,bomber,support,...} is the ace's ROLE bucket, NOT a "
                "literal equipment type; tier in {good,unique,genius}. alive "
                "defaults True when the field is absent (do NOT infer death from "
                "handled). killer_name is an object ref -> killer_id + killer_type. "
                "kill_type is a death-cause enum, not a kill count.",
                "officer_corps": "Officer-corps = the academy/army/division-command "
                "spirit ideas, stored in politics.ideas (flat token list) and "
                "recognised by the '_spirit' suffix; timed_idea gives any remaining "
                "duration. captured_enemy_generals = leaders this country holds "
                "(captured_by==TAG); own_generals_captured = this country's leaders "
                "held elsewhere.",
            },
            "stats": {
                "countries_with_aces": len(aces_out),
                "total_aces": total_aces,
                "aces_alive": aces_alive,
                "aces_dead": total_aces - aces_alive,
                "gated_commanders": total_gated,
                "countries_with_gated_commanders": len(commander_abilities),
                "captured_commanders": total_captured,
                "countries_with_officer_corps": len(officer_corps),
            },
        },
        "aces": dict(sorted(aces_out.items())),
        "commander_abilities": dict(sorted(commander_abilities.items())),
        "officer_corps": dict(sorted(officer_corps.items())),
    }

    st = out["meta"]["stats"]
    lines = ["=== COMMANDER-ABILITIES / ACES / OFFICER-CORPS ORACLE VALIDATION ==="]
    lines.append(f"source        : {source.name} (melted={was_melted})")
    lines.append(f"save date     : {meta.get('date')}  "
                 f"version={meta.get('version')}")
    lines.append(f"aces          : {st['total_aces']} total "
                 f"({st['aces_alive']} alive / {st['aces_dead']} dead) across "
                 f"{st['countries_with_aces']} countries")
    lines.append(f"gated commanders: {st['gated_commanders']} (cooldown/captured) "
                 f"in {st['countries_with_gated_commanders']} countries; "
                 f"{st['captured_commanders']} captured")
    lines.append(f"officer corps : {st['countries_with_officer_corps']} countries "
                 f"with spirits/captures")
    if aces_out:
        brk = ", ".join(f"{t}:{v['count']}" for t, v in
                        sorted(aces_out.items(), key=lambda x: -x[1]['count']))
        lines.append(f" aces by tag  : {brk}")
    ger = aces_out.get("GER")
    if ger and ger["aces"]:
        a0 = ger["aces"][0]
        lines.append("-- GER ace spot-check --")
        lines.append(f"   {a0['name']!r} {a0['surname']!r} "
                     f"callsign={a0['callsign']!r} modifier={a0['modifier']!r} "
                     f"({a0['ace_category']}/{a0['tier']}) alive={a0['alive']}")
    gca = None
    if commander_abilities:
        gca_tag = "GER" if "GER" in commander_abilities else next(
            iter(commander_abilities))
        gca = (gca_tag, commander_abilities[gca_tag][0])
    if gca:
        tag0, c0 = gca
        lines.append(f"-- commander cooldown spot-check ({tag0}) --")
        lines.append(f"   role={c0['role']} ul_id={c0['unit_leader_id']} "
                     f"reason={c0['cooldown_reason']!r} "
                     f"start={c0['cooldown_start_date']} "
                     f"enable={c0['modifier_enable_date']}")
    return {"abilities.json": out}, "\n".join(lines)


if __name__ == "__main__":
    raise SystemExit(dc.run_cli(
        "abilities",
        "the game save -> commander-abilities, aces & officer-corps oracle.",
        build, ["abilities.json"]))
