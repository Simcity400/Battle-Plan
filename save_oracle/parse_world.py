#!/usr/bin/env python3
"""the game save -> WORLD systems + SCENARIO metadata (``world.json``).

Closes a cluster of top-level world/meta domains the older oracle never parsed:

  - scenario / session meta : ``start_date``, ``gameplaysettings``
      ({difficulty, historical}), ``difficulty``, ``tutorial``, ``dlcs``,
      ``mods``, ``session``, ``speed``, ``save_version``, ``ideology``,
      explicit empty-block state for ``game_rules`` / ``difficulty_settings``.
      (Area 30: ``game_rules`` is present but EMPTY in this save — recorded as
      such; the populated scenario meta beside it IS readable.)
  - world-tension scaling inputs : ``average_major_ic``,
      ``tension_scaling_base_country`` (the threat/tension scaling inputs; the
      per-action threat ledger is in events.json).
  - ``tech_sharing_group``   : every tech-sharing group {id, bonuses, countries}
      (6 in the sample — repeated top-level key, accumulated).
  - ``faction_system``       : per-faction influence internals (index-keyed)
      {initiative, contribution, war_score_breakdown, member_upgrades} —
      distinct from the faction MEMBERSHIP list (diplomacy.json).
  - ``power_balance``        : balance-of-power instances {template, value,
      left/right/trending side, countries}.
  - ``variables``            : the global script variables map (numeric).
  - ``flags``                : the global flags map ({flag: {value, date}}).
  - ``saved_event_target``   : the global saved scopes ({country/state, name})
      (repeated top-level key, accumulated).

stdlib only. Reuses the shared tokenizer via ``deep_common``.
"""

from __future__ import annotations

import deep_common as dc
from deep_common import (
    LBRACE, RBRACE, EQUALS, atom_value, iter_block_entries, _skip_block_body,
    read_scalar_list, to_int, to_float,
)


# ---------------------------------------------------------------------------
# tech_sharing_group  (repeated top-level key)
# ---------------------------------------------------------------------------

def _parse_tech_sharing(tz, groups):
    rec = {"id": None, "bonuses": [], "countries": []}
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "bonuses":
                rec["bonuses"] = [to_float(a) for a in read_scalar_list(tz)]
            elif key == "countries":
                rec["countries"] = read_scalar_list(tz)
            else:
                _skip_block_body(tz)
            continue
        v = atom_value(opener)
        if v is not None and key == "id":
            rec["id"] = v
    groups.append(rec)


# ---------------------------------------------------------------------------
# faction_system
# ---------------------------------------------------------------------------

def _parse_faction_system(tz, factions):
    for key, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        if key != "countries":
            _skip_block_body(tz)
            continue
        # countries={ <num>={ index data={...} } { index data={...} } ... }
        # entries are a mix of `<key>={...}` and anonymous `{...}` blocks.
        _parse_faction_entries(tz, factions)


def _parse_faction_entries(tz, factions):
    depth = 1  # countries '{' consumed
    while depth > 0:
        tok = tz.next()
        if tok is None:
            return
        if tok is RBRACE:
            depth -= 1
            continue
        if tok is LBRACE:
            # anonymous faction entry
            factions.append(_parse_faction_member(tz))
            continue
        key = atom_value(tok)
        if key is None:
            continue
        nxt = tz.peek()
        if nxt is EQUALS:
            tz.next()
            v = tz.next()
            if v is LBRACE:
                rec = _parse_faction_member(tz)
                rec["key"] = key
                factions.append(rec)
            # scalar: ignore


def _parse_faction_member(tz):
    rec = {"key": None, "index": None, "initiative": None,
           "contribution": None, "war_score_breakdown": None,
           "member_upgrades": []}
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "data":
                for k, op in iter_block_entries(tz):
                    if op is LBRACE:
                        if k == "member_upgrades":
                            rec["member_upgrades"] = read_scalar_list(tz)
                        else:
                            _skip_block_body(tz)
                        continue
                    v = atom_value(op)
                    if v is None:
                        continue
                    if k == "initiative":
                        rec["initiative"] = to_float(v)
                    elif k == "contribution":
                        rec["contribution"] = to_float(v)
                    elif k == "war_score_breakdown":
                        rec["war_score_breakdown"] = to_float(v)
            else:
                _skip_block_body(tz)
            continue
        v = atom_value(opener)
        if v is not None and key == "index":
            rec["index"] = to_int(v)
    return rec


# ---------------------------------------------------------------------------
# power_balance
# ---------------------------------------------------------------------------

def _parse_power_balance(tz, bops):
    for key, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        if key != "power_balances":
            _skip_block_body(tz)
            continue
        _parse_bop_list(tz, bops)


def _parse_bop_list(tz, bops):
    depth = 1
    while depth > 0:
        tok = tz.next()
        if tok is None:
            return
        if tok is RBRACE:
            depth -= 1
            continue
        if tok is LBRACE:
            bops.append(_parse_bop_entry(tz))


def _parse_bop_entry(tz):
    rec = {"template": None, "value": None, "left_side": None,
           "right_side": None, "trending_side": None, "countries": []}
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "countries":
                rec["countries"] = read_scalar_list(tz)
            else:
                _skip_block_body(tz)
            continue
        v = atom_value(opener)
        if v is None:
            continue
        if key == "template":
            rec["template"] = v
        elif key == "value":
            rec["value"] = to_float(v)
        elif key == "left_side":
            rec["left_side"] = v
        elif key == "right_side":
            rec["right_side"] = v
        elif key == "trending_side":
            rec["trending_side"] = v
    return rec


# ---------------------------------------------------------------------------
# variables / flags / saved_event_target
# ---------------------------------------------------------------------------

def _parse_variables(tz, variables):
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            _skip_block_body(tz)  # nested (e.g. random={...}) ignored
            continue
        v = atom_value(opener)
        if v is None:
            continue
        f = to_float(v)
        variables[key] = f if f is not None else v


def _parse_flags(tz, flags):
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
        # scalar flags (rare) ignored


def _parse_saved_event_target(tz, targets):
    rec = {"country": None, "state": None, "name": None}
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            _skip_block_body(tz)
            continue
        v = atom_value(opener)
        if v is None:
            continue
        if key == "country":
            rec["country"] = v
        elif key == "state":
            rec["state"] = to_int(v)
        elif key == "name":
            rec["name"] = v
    targets.append(rec)


# ---------------------------------------------------------------------------
# build
# ---------------------------------------------------------------------------

def build(text, source, was_melted):
    tech_groups = []
    factions = []
    bops = []
    variables = {}
    flags = {}
    saved_targets = []
    game_rules_present = {"empty": None}
    difficulty_settings_present = {"empty": None}

    def _parse_empty_block(tz, state):
        n = 0
        for key, opener in iter_block_entries(tz):
            if opener is LBRACE:
                _skip_block_body(tz)
            n += 1
        state["empty"] = (n == 0)

    def _parse_game_rules(tz):
        _parse_empty_block(tz, game_rules_present)

    def _parse_difficulty_settings(tz):
        _parse_empty_block(tz, difficulty_settings_present)

    def _parse_mods(tz):
        meta_extra["mods"] = read_scalar_list(tz)

    def _parse_gameplaysettings(tz):
        for key, opener in iter_block_entries(tz):
            if opener is LBRACE:
                _skip_block_body(tz)
                continue
            v = atom_value(opener)
            if v is not None:
                meta_extra.setdefault("gameplaysettings", {})[key] = v

    meta_extra = {}

    meta = dc.top_level_pass(
        text,
        {
            "tech_sharing_group": lambda tz: _parse_tech_sharing(tz, tech_groups),
            "faction_system": lambda tz: _parse_faction_system(tz, factions),
            "power_balance": lambda tz: _parse_power_balance(tz, bops),
            "variables": lambda tz: _parse_variables(tz, variables),
            "flags": lambda tz: _parse_flags(tz, flags),
            "saved_event_target":
                lambda tz: _parse_saved_event_target(tz, saved_targets),
            "game_rules": _parse_game_rules,
            "difficulty_settings": _parse_difficulty_settings,
            "mods": _parse_mods,
            "gameplaysettings": _parse_gameplaysettings,
        },
        scalar_keys=("date", "version", "player", "ideology", "difficulty",
                     "tutorial", "start_date", "dlcs", "session", "speed",
                     "save_version", "average_major_ic",
                     "tension_scaling_base_country"),
    )

    scenario = {
        "start_date": meta.get("start_date"),
        "difficulty": meta.get("difficulty"),
        "historical": (meta_extra.get("gameplaysettings", {})
                       .get("historical")),
        "tutorial": meta.get("tutorial"),
        "dlcs": meta.get("dlcs"),
        "mods": meta_extra.get("mods", []),
        "session": meta.get("session"),
        "speed": meta.get("speed"),
        "save_version": meta.get("save_version"),
        "ideology": meta.get("ideology"),
        "gameplaysettings": meta_extra.get("gameplaysettings", {}),
        "game_rules_empty": game_rules_present["empty"],
        "difficulty_settings_empty": difficulty_settings_present["empty"],
    }
    world_tension_inputs = {
        "average_major_ic": to_float(meta.get("average_major_ic")),
        "tension_scaling_base_country": meta.get("tension_scaling_base_country"),
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
                "game_rules": "game_rules is present but EMPTY in this save; "
                "scenario meta beside it is populated and captured.",
                "difficulty_settings": "difficulty_settings is present but EMPTY "
                "in this save; the actual selected difficulty is the separate "
                "top-level difficulty scalar.",
                "world_tension": "the aggregate world_tension value is not a "
                "stored top-level key here; these are the scaling INPUTS. The "
                "per-action threat ledger is in events.json.",
                "faction_system": "per-faction INFLUENCE internals, distinct "
                "from the faction membership list in diplomacy.json.",
            },
            "stats": {
                "tech_sharing_groups": len(tech_groups),
                "faction_entries": len(factions),
                "power_balances": len(bops),
                "global_variables": len(variables),
                "global_flags": len(flags),
                "saved_event_targets": len(saved_targets),
            },
        },
        "scenario": scenario,
        "world_tension_inputs": world_tension_inputs,
        "tech_sharing_groups": tech_groups,
        "faction_system": factions,
        "power_balances": bops,
        "variables": dict(sorted(variables.items())),
        "flags": dict(sorted(flags.items())),
        "saved_event_targets": saved_targets,
    }

    lines = ["=== WORLD / SCENARIO ORACLE VALIDATION ==="]
    lines.append(f"source        : {source.name} (melted={was_melted})")
    lines.append(f"save date     : {meta.get('date')}")
    lines.append(f"scenario      : start={scenario['start_date']} "
                 f"diff={scenario['difficulty']} hist={scenario['historical']} "
                 f"dlcs={scenario['dlcs']} mods={scenario['mods']}")
    lines.append(f"game_rules    : empty={scenario['game_rules_empty']}")
    lines.append("difficulty_settings: "
                 f"empty={scenario['difficulty_settings_empty']}")
    lines.append(f"WT inputs     : avg_major_ic="
                 f"{world_tension_inputs['average_major_ic']} "
                 f"base={world_tension_inputs['tension_scaling_base_country']}")
    lines.append(f"tech-sharing  : {len(tech_groups)} groups "
                 f"{[g['id'] for g in tech_groups][:3]}")
    lines.append(f"faction infl. : {len(factions)} entries")
    lines.append(f"power balances: {len(bops)} "
                 f"{[b['template'] for b in bops][:3]}")
    lines.append(f"global vars   : {len(variables)}  global flags: {len(flags)}")
    lines.append(f"saved targets : {len(saved_targets)} {saved_targets[:2]}")
    return {"world.json": out}, "\n".join(lines)


if __name__ == "__main__":
    raise SystemExit(dc.run_cli(
        "world",
        "the game save -> world systems + scenario metadata oracle.",
        build, ["world.json"]))
