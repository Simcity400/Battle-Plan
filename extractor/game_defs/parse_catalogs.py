#!/usr/bin/env python3
"""CATALOGS parser  (bucket-(c): the id->fields/modifiers/costs/loc catalogs).

One module, several catalogs — each a flat ``{id: {fields...}}`` table the UI
binds (icon, loc, modifiers, costs). Mod-over-vanilla via the effective file set.
Covers, per the bucket-(c) ledger:

  occupation_laws : common/occupation_laws/*  -> tooltip, icon, gui_order,
                    state_modifier{}, suppressed_state_modifier{}, fallback_law,
                    main_fallback_law, visible/available presence.
  leader_traits   : common/unit_leader/*  `leader_traits={ <id>={...} }` ->
                    type, trait_type, modifier{}, non_shared_modifier{},
                    prerequisites, gain_xp / cost, mutually_exclusive.
  unit_modifiers  : common/unit_leader/*  `unit_modifiers={...}` (where present).
  combat_tactics  : common/combat_tactics.txt `tactic_*` -> is_attacker,
                    countered_by, picture, the attacker/defender bonus rows.
  aces            : common/aces/*  `modifiers={ <id>={ type chance effect{} } }`.
  operations      : common/operations/*  -> icon, cost/risk/network req fields.
  operation_tokens: common/operation_tokens/* -> the persistent infiltration-token
                    dictionary (name/description/icons, intel lane and exact gain).
  synchronized_dynamic_tokens: common/synchronized_dynamic_tokens/* -> the
                    engine's modset-bound numeric-token spelling registry. This
                    is dictionary input only, never a second world-state source.
  opinion_mods    : common/opinion_modifiers/*  `opinion_modifiers={ <id>={...} }`.
  equipment_modules: common/units/equipment/modules/*
                    `equipment_modules={ <id>={ category abbreviation add_stats{} } }`.
  ideas           : common/ideas/*  (national spirits + advisors + designers) ->
                    per category->idea: modifier{}, allowed{}, available presence,
                    cost/removal_cost, ledger, the advisor `traits`+`slot`/`ledger`.
  state_categories: common/state_category/* -> local_building_slots (panel chrome).
  power_balances  : common/bop/* -> balance id, left/right side ids, side icons,
                    and per-range min/max threshold + modifier blocks.

stdlib only.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import resolver as R
from paradox_parser import Node


def _scalar(v):
    return v if isinstance(v, str) else None


def _plain(node):
    if isinstance(node, str):
        return node
    if isinstance(node, Node):
        out, anon = {}, []
        for k, v in node.entries:
            (anon.append(_plain(v)) if k == "" else out.__setitem__(k, _plain(v)))
        if anon and not out:
            return anon
        if anon:
            out["_list"] = anon
        return out
    return node


def _plain_preserve_duplicate_keys(node):
    """Convert a Clausewitz node without dropping repeated sibling keys.

    Most catalog consumers rely on the historical single-value object shape, so
    this converter keeps that shape when a key occurs once and promotes only a
    repeated key's values to a source-ordered JSON array. Power-balance
    ``on_activate``/``on_deactivate`` blocks need this lossless form because the
    shipped CZE and FIN definitions author more than one
    ``set_power_balance_gfx`` command in the same block.
    """
    if isinstance(node, str):
        return node
    if not isinstance(node, Node):
        return node

    keyed: dict[str, list] = {}
    key_order: list[str] = []
    anonymous = []
    for key, value in node.entries:
        converted = _plain_preserve_duplicate_keys(value)
        if key == "":
            anonymous.append(converted)
            continue
        if key not in keyed:
            keyed[key] = []
            key_order.append(key)
        keyed[key].append(converted)

    out = {
        key: values[0] if len(values) == 1 else values
        for key in key_order
        for values in (keyed[key],)
    }
    if anonymous and not out:
        return anonymous
    if anonymous:
        out["_list"] = anonymous
    return out


def _atoms(node):
    if not isinstance(node, Node):
        return []
    return [v for k, v in node.entries if k == "" and isinstance(v, str)]


def _ordered_files(layers, subdir):
    return R.ordered_files(layers, subdir, (".txt",))


def _walk(layers, subdir, wrapper, per_item, dest, collisions):
    """Generic catalog walker. ``wrapper`` is the outer key whose children are the
    items (None = items are top-level). ``per_item(id,node,rel)->dict``."""
    for rel, abspath in _ordered_files(layers, subdir):
        try:
            root = R.parse_file(abspath)
        except Exception as e:
            print(f"[catalogs] WARN {rel}: {e}", file=sys.stderr)
            continue
        containers = ([root.get(wrapper)] if wrapper else [root])
        if wrapper:
            containers = [v for k, v in root.entries
                          if k.lower() == wrapper.lower() and isinstance(v, Node)]
        for cont in containers:
            if not isinstance(cont, Node):
                continue
            for iid, body in cont.entries:
                if not isinstance(body, Node) or not iid:
                    continue
                rec = per_item(iid, body, rel)
                if rec is None:
                    continue
                if iid in dest and dest[iid].get("source_file") != rel:
                    collisions.append({"id": iid, "first": dest[iid]["source_file"],
                                       "override": rel})
                dest[iid] = rec


def _law(iid, n, rel):
    return {
        "id": iid, "tooltip": _scalar(n.get("tooltip")),
        "icon": _plain(n.get("icon")), "gui_order": n.get("gui_order"),
        "fallback_law": _scalar(n.get("fallback_law")),
        "main_fallback_law": _scalar(n.get("main_fallback_law")) == "yes",
        "state_modifier": _plain(n.get("state_modifier")) or {},
        "suppressed_state_modifier": _plain(n.get("suppressed_state_modifier")) or {},
        "has_visible": n.get("visible") is not None,
        "has_available": n.get("available") is not None,
        "source_file": rel,
    }


def _trait(iid, n, rel):
    return {
        "id": iid, "type": _scalar(n.get("type")),
        "trait_type": _scalar(n.get("trait_type")),
        "modifier": _plain(n.get("modifier")) or {},
        "non_shared_modifier": _plain(n.get("non_shared_modifier")) or {},
        "cost": n.get("cost"), "gain_xp": _plain(n.get("gain_xp")),
        "prerequisites": _atoms(n.get("prerequisites")),
        "mutually_exclusive": _atoms(n.get("mutually_exclusive")),
        "source_file": rel,
    }


def _tactic(iid, n, rel):
    if not iid.startswith("tactic_"):
        return None
    fixed = {"is_attacker", "is_defender", "trigger", "active", "base",
             "picture", "countered_by", "sound", "group", "chance",
             "combatant", "phase"}
    bonuses = {k: _plain(v) for k, v in n.entries if k and k.lower() not in fixed}
    return {
        "id": iid, "is_attacker": _scalar(n.get("is_attacker")) == "yes",
        "countered_by": _scalar(n.get("countered_by")),
        "picture": _scalar(n.get("picture")), "bonuses": bonuses,
        "source_file": rel,
    }


def _ace(iid, n, rel):
    return {
        "id": iid, "type": _atoms(n.get("type")), "chance": n.get("chance"),
        "effect": _plain(n.get("effect")) or {}, "source_file": rel,
    }


def _operation(iid, n, rel):
    if not iid.startswith("operation_"):
        return None
    fixed_blocks = {"icon", "name", "desc"}
    fields = {k: _plain(v) for k, v in n.entries
              if k and k.lower() not in fixed_blocks}
    return {"id": iid, "icon": _scalar(n.get("icon")),
            "fields": fields, "source_file": rel}


def _operation_token(iid, n, rel):
    """One authored operation-token definition.

    Country save state stores only the token id and target-country membership.
    These fields are the modset-bound dictionary needed to render that identity;
    scripted effects and availability remain owned by the operation definitions.
    """
    return {
        "id": iid,
        "name": _scalar(n.get("name")),
        "desc": _scalar(n.get("desc")),
        "icon": _scalar(n.get("icon")),
        "text_icon": _scalar(n.get("text_icon")),
        "intel_source": _scalar(n.get("intel_source")),
        "intel_gain": n.get("intel_gain"),
        "source_file": rel,
    }


def _opinion(iid, n, rel):
    return {"id": iid, "value": n.get("value"),
            "fields": {k: _plain(v) for k, v in n.entries if k and k.lower() != "value"},
            "source_file": rel}


def _module(iid, n, rel):
    if iid.lower() in ("has_dlc",):
        return None
    return {
        "id": iid, "category": _scalar(n.get("category")),
        "abbreviation": _scalar(n.get("abbreviation")),
        "add_stats": _plain(n.get("add_stats")) or {},
        "multiply_stats": _plain(n.get("multiply_stats")) or {},
        "add_average_stats": _plain(n.get("add_average_stats")) or {},
        "can_convert_from": _atoms(n.get("can_convert_from")),
        "source_file": rel,
    }


def _state_cat(iid, n, rel):
    return {"id": iid, "local_building_slots": n.get("local_building_slots"),
            "color": _plain(n.get("color")), "source_file": rel}


def _bop_range(n, rel, side_id=None, order=0):
    return {
        "id": _scalar(n.get("id")),
        "side_id": side_id,
        "order": int(order),
        "min": n.get("min"),
        "max": n.get("max"),
        "modifier": _plain(n.get("modifier")) or {},
        "has_on_activate": n.get("on_activate") is not None,
        "has_on_deactivate": n.get("on_deactivate") is not None,
        "on_activate": _plain_preserve_duplicate_keys(n.get("on_activate")) or {},
        "on_deactivate": _plain_preserve_duplicate_keys(n.get("on_deactivate")) or {},
        "source_file": rel,
    }


def _bop_side(n, rel, order=0):
    side_id = _scalar(n.get("id"))
    ranges = []
    for i, (_k, v) in enumerate(
            (entry for entry in n.entries
             if entry[0].lower() == "range" and isinstance(entry[1], Node))):
        ranges.append(_bop_range(v, rel, side_id=side_id, order=i))
    return {
        "id": side_id,
        "order": int(order),
        "icon": _scalar(n.get("icon")),
        "ranges": ranges,
        "range_count": len(ranges),
        "source_file": rel,
    }


def _power_balance(iid, n, rel):
    neutral_ranges = []
    sides = []
    for k, v in n.entries:
        kl = k.lower()
        if kl == "range" and isinstance(v, Node):
            neutral_ranges.append(
                _bop_range(v, rel, side_id=None, order=len(neutral_ranges)))
        elif kl == "side" and isinstance(v, Node):
            sides.append(_bop_side(v, rel, order=len(sides)))
    return {
        "id": iid,
        "initial_value": n.get("initial_value"),
        "left_side": _scalar(n.get("left_side")),
        "right_side": _scalar(n.get("right_side")),
        "decision_category": _scalar(n.get("decision_category")),
        "neutral_ranges": neutral_ranges,
        "sides": sides,
        "range_count": len(neutral_ranges)
        + sum(side["range_count"] for side in sides),
        "source_file": rel,
    }


def _archetype(iid, n, rel):
    """An equipment ARCHETYPE/variant entry from common/units/equipment/*.txt
    `equipments={ <id>={ is_archetype= type{} <base stats> build_cost_ic= } }`.
    These carry the BASE DEF stats + cost the designers show (distinct from the
    module add_stats catalog)."""
    stat_keys = {
        "year", "is_archetype", "is_buildable", "picture", "type",
        "upgrades", "group_by", "interface_category", "resources",
        "archetype", "parent", "priority", "visual_level", "active",
        "can_license", "module_slots", "default_modules", "sprite",
        "allowed_modules", "module_count_limit", "one_use_only", "name",
    }
    stats = {k: _plain(v) for k, v in n.entries
             if k and k.lower() not in stat_keys}
    return {
        "id": iid,
        "is_archetype": _scalar(n.get("is_archetype")) == "yes",
        "archetype": _scalar(n.get("archetype")) or _scalar(n.get("parent")),
        "year": n.get("year"),
        "type": _atoms(n.get("type")) or _scalar(n.get("type")),
        "build_cost_ic": n.get("build_cost_ic"),
        "upgrades": _atoms(n.get("upgrades")),
        "resources": _plain(n.get("resources")) or {},
        "stats": stats,
        "source_file": rel,
    }


def _parse_ideas(layers, collisions):
    """ideas = { <category> = { <idea> = {...} } }. Categories include
    national spirits, advisor slots (political_advisor, etc.), tank/ship/plane
    designers. Per idea we keep modifier{}, allowed/available presence, cost,
    and the advisor sub-block (slot/ledger/traits/cost)."""
    out: dict[str, dict] = {}
    for rel, abspath in _ordered_files(layers, "common/ideas"):
        try:
            root = R.parse_file(abspath)
        except Exception as e:
            print(f"[catalogs] WARN ideas {rel}: {e}", file=sys.stderr)
            continue
        ideas_block = root.get("ideas")
        if not isinstance(ideas_block, Node):
            continue
        for cat, catbody in ideas_block.entries:
            if not isinstance(catbody, Node):
                continue
            for iid, body in catbody.entries:
                if not isinstance(body, Node) or not iid:
                    continue
                if iid.lower() in ("designer", "law", "use_list_view", "ledger"):
                    continue
                adv = body.get("allowed_civil_war")
                rec = {
                    "id": iid, "category": cat,
                    "modifier": _plain(body.get("modifier")) or {},
                    "research_bonus": _plain(body.get("research_bonus")) or {},
                    "cost": body.get("cost"),
                    "removal_cost": body.get("removal_cost"),
                    "has_allowed": body.get("allowed") is not None,
                    "has_available": body.get("available") is not None,
                    "has_on_add": body.get("on_add") is not None,
                    "is_advisor": False,
                    "source_file": rel,
                }
                ah = body.get("allowed")
                advisor = None
                if body.get("ledger") is not None or body.get("slot") is not None \
                        or body.get("traits") is not None:
                    rec["is_advisor"] = True
                    rec["advisor_slot"] = _scalar(body.get("slot"))
                    rec["advisor_traits"] = _atoms(body.get("traits"))
                    rec["advisor_ledger"] = _scalar(body.get("ledger"))
                if iid in out and out[iid].get("source_file") != rel:
                    collisions.append({"id": iid, "first": out[iid]["source_file"],
                                       "override": rel})
                out[iid] = rec
    return out


def _parse_synchronized_dynamic_tokens(layers, collisions):
    """Return the effective ordered dynamic-token spelling registry.

    Files in this directory are anonymous scalar lists rather than keyed
    Clausewitz blocks.  Duplicate spellings are one identity; their first and
    last effective sources are retained so a mod collision is auditable.
    """
    out: dict[str, dict] = {}
    ordinal = 0
    for rel, abspath in _ordered_files(layers, "common/synchronized_dynamic_tokens"):
        try:
            root = R.parse_file(abspath)
        except Exception as e:
            print(f"[catalogs] WARN synchronized tokens {rel}: {e}", file=sys.stderr)
            continue
        for key, value in root.entries:
            if key or not isinstance(value, str) or not value:
                continue
            if value in out:
                collisions.append({
                    "id": value,
                    "first": out[value]["source_file"],
                    "override": rel,
                })
                out[value]["last_source_file"] = rel
                continue
            out[value] = {
                "id": value,
                "ordinal": ordinal,
                "source_file": rel,
                "last_source_file": rel,
            }
            ordinal += 1
    return out


def parse_catalogs(layers) -> dict:
    catalogs = {}
    coll = {}
    spec = [
        ("occupation_laws", "common/occupation_laws", None, _law),
        ("leader_traits", "common/unit_leader", "leader_traits", _trait),
        ("aces", "common/aces", "modifiers", _ace),
        ("operations", "common/operations", None, _operation),
        ("operation_tokens", "common/operation_tokens", None, _operation_token),
        ("opinion_modifiers", "common/opinion_modifiers", "opinion_modifiers", _opinion),
        ("equipment_modules", "common/units/equipment/modules", "equipment_modules", _module),
        ("state_categories", "common/state_category", "state_categories", _state_cat),
        ("power_balances", "common/bop", None, _power_balance),
    ]
    for name, subdir, wrapper, fn in spec:
        dest, c = {}, []
        _walk(layers, subdir, wrapper, fn, dest, c)
        catalogs[name] = dest
        coll[name] = c
    dest, c = {}, []
    tactic_path = None
    for ly in layers:
        cand = ly.root / "common" / "combat_tactics.txt"
        if cand.is_file():
            tactic_path = cand
    if tactic_path is not None:
        rel = "common/combat_tactics.txt"
        try:
            root = R.parse_file(tactic_path)
            for iid, body in root.entries:
                if isinstance(body, Node) and iid:
                    rec = _tactic(iid, body, rel)
                    if rec:
                        dest[iid] = rec
        except Exception as e:
            print(f"[catalogs] WARN tactics: {e}", file=sys.stderr)
    catalogs["combat_tactics"] = dest
    coll["combat_tactics"] = c
    dest, c = {}, []
    for rel, abspath in _ordered_files(layers, "common/units/equipment"):
        if Path(rel).parent.as_posix() != "common/units/equipment":
            continue
        try:
            root = R.parse_file(abspath)
        except Exception as e:
            print(f"[catalogs] WARN equipment {rel}: {e}", file=sys.stderr)
            continue
        for blk in root.get_all("equipments"):
            if not isinstance(blk, Node):
                continue
            for eid, body in blk.entries:
                if not isinstance(body, Node) or not eid:
                    continue
                rec = _archetype(eid, body, rel)
                if eid in dest and dest[eid].get("source_file") != rel:
                    c.append({"id": eid, "first": dest[eid]["source_file"],
                              "override": rel})
                dest[eid] = rec
    catalogs["equipment_archetypes"] = dest
    coll["equipment_archetypes"] = c

    dest, c = {}, []
    for rel, abspath in _ordered_files(layers, "common/units/equipment/upgrades"):
        try:
            root = R.parse_file(abspath)
        except Exception:
            continue
        for blk in root.get_all("upgrades"):
            if not isinstance(blk, Node):
                continue
            for uid, body in blk.entries:
                if not isinstance(body, Node) or not uid:
                    continue
                dest[uid] = {"id": uid,
                             "fields": {k: _plain(v) for k, v in body.entries if k},
                             "source_file": rel}
    catalogs["equipment_upgrades"] = dest
    coll["equipment_upgrades"] = c

    ci = []
    catalogs["ideas"] = _parse_ideas(layers, ci)
    coll["ideas"] = ci

    ct = []
    catalogs["synchronized_dynamic_tokens"] = (
        _parse_synchronized_dynamic_tokens(layers, ct)
    )
    coll["synchronized_dynamic_tokens"] = ct

    meta = {name: {"count": len(tbl),
                   "collisions": len(coll.get(name, []))}
            for name, tbl in catalogs.items()}
    return {"meta": meta, "catalogs": {k: dict(sorted(v.items()))
                                       for k, v in catalogs.items()},
            "collisions": coll}


def build(vanilla=None, mods=None, modset_json=None, userdata=None):
    layers = R.build_layers(vanilla=vanilla, mods=mods,
                            modset_json=modset_json, userdata=userdata)
    return parse_catalogs(layers)


if __name__ == "__main__":
    cli = argparse.ArgumentParser()
    cli.add_argument("--vanilla")
    cli.add_argument("--mod", dest="mods", action="append")
    cli.add_argument("--modset-json")
    cli.add_argument("--userdata")
    cli.add_argument("--out", type=Path,
                     default=Path(__file__).resolve().parent / "out" / "catalogs.json")
    args = cli.parse_args()
    out = build(vanilla=args.vanilla, mods=args.mods,
                modset_json=args.modset_json, userdata=args.userdata)
    outp = args.out
    outp.parent.mkdir(parents=True, exist_ok=True)
    outp.write_text(json.dumps(out, ensure_ascii=False, indent=1), encoding="utf-8")
    for name, m in out["meta"].items():
        print(f"  {name:20s}: {m['count']:5d}  (collisions {m['collisions']})")
    print(f"-> {outp.name}")
