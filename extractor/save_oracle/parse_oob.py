#!/usr/bin/env python3
"""the game save ORDER-OF-BATTLE oracle (command structure + battle plans).

Extends the save-oracle family (``parse_save.py`` tokenizer, ``deep_common.py``
helpers) to extract the persisted **order of battle** and **battle-plan
structure** from a melted the game save:

  1. ARMY:  ``countries.<TAG>.theatres.theatre[]`` ->
       - ``orders_group``  = an ARMY (id, name, commanding-general ``leader`` id,
         ``member`` unit refs = its assigned divisions, and its battle-plan
         ``order_instance`` orders).
       - ``field_marshal_group`` = an ARMY GROUP. It does NOT nest armies; it
         carries ``orders_group={id type}`` *references* to the member armies,
         plus its own id/name/leader. (Verified in a real melt + Codex.)
  2. NAVY:  ``countries.<TAG>.units.fleet[]`` -> ``task_force[]`` -> ``ship[]``.
       fleet.leader = commanding ADMIRAL id. ship.definition = hull/class;
       ship.equipment id={id,type=70} joins the equipment-variant index.
       Current task forces persist a nested mission block, movement path,
       repair/activity links and a save-tick retreat boolean; these are emitted
       exactly and are not relabeled as live mission/repair/retreat enums.
       ``countries.<TAG>.navy_theater.theater_group[]`` = naval theatre grouping
       (fleet id refs).
  3. AIR:   top-level ``strategic_air.<TAG>.air_wing_pool[].air_wings[]`` = the
       wings (count, plane ``definition``, mission + strategic_region, equipment
       variant refs, name). ``strategic_air.air_base[]`` maps an air-base id
       (type 65) -> state, so a wing -> air_base -> state join is attempted.
       NOTE: carrier/mobile-based wings may NOT resolve to a land air-base; that
       is an expected variant, not an error (Codex).
  4. BATTLE PLANS: ``order_instance`` is POLYMORPHIC (front / offensive /
       spearhead / area-defense / naval-invasion / paradrop / fallback ...). We
       capture the common fields (type, instance_id, dates, operation, path,
       states, scheduled_member refs, front ref) and keep counts; we do not
       pretend every order shares one schema.

Joins use the full Clausewitz ``(id, type)`` object key (never a bare id), per
the engine's reference semantics. ``member.unit`` ids are resolved against the
division ids that ``parse_units`` reads (the SAME handle space), across ALL
countries (expeditionary/lend-lease divisions can be controlled by another tag).

Runtime-only (NOT authoritative in the save -> defer to the live memory readers,
do not fake): ``plan_value`` / ``our_power`` / ``enemy_power`` and mission
``effectiveness`` are cached snapshots; live combat targeting, supremacy,
detection and in-flight pathfinding are runtime. We surface the cached values
verbatim and flag them as cached.

Leader (general / admiral) ids are emitted as raw ``(id, type)`` only; their
human names are resolved by the separate characters parser, not here.

stdlib only. Reuses the shared tokenizer/helpers; adds nothing to them.

Usage:
    python parse_oob.py [save_or_melted_path]
    python parse_oob.py --rakaly /path/to/rakaly.exe --out-dir <dir> [save]
"""

from __future__ import annotations

import json
from pathlib import Path

# Shared, read-only scaffolding (tokenizer + melt/discovery + typed readers).
import deep_common as dc
from deep_common import (
    ps,
    Tokenizer,
    LBRACE,
    RBRACE,
    atom_value,
    iter_block_entries,
    _skip_block_body,
    read_id_block,
    read_scalar_list,
    to_int,
    to_float,
)


# ---------------------------------------------------------------------------
# Small helpers
# ---------------------------------------------------------------------------

def _ascii(obj):
    """Recursively make every string ASCII-only (latin1 names -> \\xNN escapes).

    Save text is latin1; ship/wing/army names can carry accented bytes
    (e.g. "Goteborgs", "Molders"). The shared CLI writes JSON with
    ensure_ascii=False, so we fold non-ASCII characters to a readable
    backslash-escape here to honour the ASCII-output requirement without losing
    the name. Reversible enough for display/debug.
    """
    if isinstance(obj, str):
        return obj.encode("ascii", "backslashreplace").decode("ascii")
    if isinstance(obj, dict):
        return {_ascii(k): _ascii(v) for k, v in obj.items()}
    if isinstance(obj, list):
        return [_ascii(v) for v in obj]
    return obj


def _idkey(idt):
    """Render an (id,type) tuple as a stable JSON-friendly string "id:type"."""
    if idt is None:
        return None
    _id, _type = idt
    if _id is None:
        return None
    return f"{_id}:{_type}"


def _read_leader(tz: Tokenizer):
    """A ``leader={ id=.. type=.. }`` reference -> (id,type)."""
    return read_id_block(tz)


# ---------------------------------------------------------------------------
# ARMY: theatres -> orders_group (army) + field_marshal_group (army group)
# ---------------------------------------------------------------------------

def _parse_order_instance(tz: Tokenizer):
    """Parse one ``order_instance={...}`` (a battle-plan order). Polymorphic.

    We capture the fields that recur across order subtypes and keep a list of
    every other (non-block) key seen so nothing is silently dropped, without
    pretending all subtypes share one schema.
    """
    o = {
        "type": None,            # numeric subtype (front/offensive/defense/...)
        "instance_id": None,
        "creation_date": None,
        "starting_date": None,
        "operation": None,       # named operation, e.g. "o_orkan"
        "path": [],              # province route
        "states": [],            # assigned states (defensive lines etc.)
        "scheduled_members": [], # unit (id,type) refs scheduled to this order
        "root_front": None,      # (id,type) of the front this order hangs off
        "extra_keys": [],        # other scalar keys present (subtype-specific)
    }
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "path":
                o["path"] = [to_int(x) for x in read_scalar_list(tz)
                             if to_int(x) is not None]
            elif k == "states":
                o["states"] = [to_int(x) for x in read_scalar_list(tz)
                               if to_int(x) is not None]
            elif k == "scheduled_member":
                o["scheduled_members"].append(_idkey(read_id_block(tz)))
            elif k == "root_front":
                o["root_front"] = _idkey(read_id_block(tz))
            else:
                # area_defense_state_assignment, sorted_pairs, etc. -> skip body
                _skip_block_body(tz)
            continue
        val = atom_value(op)
        if val is None:
            continue
        if k == "type":
            o["type"] = to_int(val)
        elif k == "instance_id":
            o["instance_id"] = to_int(val)
        elif k == "creation_date":
            o["creation_date"] = val
        elif k == "starting_date":
            o["starting_date"] = val
        elif k == "operation":
            o["operation"] = val
        elif k not in ("prefix",):
            # Preserve the presence of subtype-specific scalar keys.
            if k not in o["extra_keys"]:
                o["extra_keys"].append(k)
    return o


def _parse_orders_group(tz: Tokenizer):
    """Parse one ``orders_group={...}`` = an ARMY (full object, with members)."""
    army = {
        "id": None,                 # (id,type) of this army order group
        "name": None,
        "leader": None,             # commanding general (id,type) -> characters
        "members": [],              # division (id,type) refs assigned to army
        "orders": [],               # battle-plan order_instance list
        "field_marshal_group": False,
        "cached": {},               # plan_value/our_power/enemy_power (RUNTIME)
        "icon": None,
    }
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "id":
                army["id"] = _idkey(read_id_block(tz))
            elif k == "leader":
                army["leader"] = _idkey(_read_leader(tz))
            elif k == "member":
                # member={ unit={ id type } } -> the assigned division ref.
                u = None
                for kk, oo in iter_block_entries(tz):
                    if oo is LBRACE:
                        if kk == "unit":
                            u = _idkey(read_id_block(tz))
                        else:
                            _skip_block_body(tz)
                if u is not None:
                    army["members"].append(u)
            elif k == "order_instance":
                army["orders"].append(_parse_order_instance(tz))
            else:
                # color, etc.
                _skip_block_body(tz)
            continue
        val = atom_value(op)
        if val is None:
            continue
        if k == "name":
            army["name"] = val
        elif k == "icon":
            army["icon"] = to_int(val)
        elif k == "field_marshal_group":
            army["field_marshal_group"] = (val == "yes")
        elif k in ("plan_value", "our_power", "enemy_power"):
            army["cached"][k] = to_float(val)
    return army


def _parse_field_marshal_group(tz: Tokenizer):
    """Parse one ``field_marshal_group={...}`` = an ARMY GROUP (refs armies)."""
    ag = {
        "id": None,
        "name": None,
        "leader": None,            # field marshal (id,type) -> characters
        "army_refs": [],           # orders_group (id,type) refs = member armies
        "cached": {},
        "icon": None,
    }
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "id":
                ag["id"] = _idkey(read_id_block(tz))
            elif k == "leader":
                ag["leader"] = _idkey(_read_leader(tz))
            elif k == "orders_group":
                # Inside a field_marshal_group these are id-only REFERENCES to
                # the real armies (no name/members here).
                ag["army_refs"].append(_idkey(read_id_block(tz)))
            else:
                _skip_block_body(tz)
            continue
        val = atom_value(op)
        if val is None:
            continue
        if k == "name":
            ag["name"] = val
        elif k == "icon":
            ag["icon"] = to_int(val)
        elif k in ("plan_value", "our_power", "enemy_power"):
            ag["cached"][k] = to_float(val)
    return ag


def _parse_theatre(tz: Tokenizer, tag: str, armies: list, army_groups: list,
                   theatres_meta: list):
    """Parse one ``theatre={...}`` block; append armies/army-groups for TAG.

    A theatre directly contains ``orders_group`` (armies) and
    ``field_marshal_group`` (army groups) as siblings, plus an ``area`` (state
    ids) and an ``id``.
    """
    theatre = {"owner": tag, "id": None, "area_states": [],
               "n_armies": 0, "n_army_groups": 0}
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "id":
                theatre["id"] = _idkey(read_id_block(tz))
            elif k == "area":
                theatre["area_states"] = [
                    to_int(x) for x in read_scalar_list(tz)
                    if to_int(x) is not None]
            elif k == "orders_group":
                army = _parse_orders_group(tz)
                army["owner"] = tag
                armies.append(army)
                theatre["n_armies"] += 1
            elif k == "field_marshal_group":
                ag = _parse_field_marshal_group(tz)
                ag["owner"] = tag
                army_groups.append(ag)
                theatre["n_army_groups"] += 1
            else:
                _skip_block_body(tz)
        # scalars at theatre level ignored
    theatres_meta.append(theatre)


def _parse_theatres(tz: Tokenizer, tag: str, armies: list, army_groups: list,
                    theatres_meta: list):
    """Parse ``theatres={ theatre={...} ... }`` for one country."""
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "theatre":
                _parse_theatre(tz, tag, armies, army_groups, theatres_meta)
            else:
                _skip_block_body(tz)


def _parse_navy_theater(tz: Tokenizer, tag: str, navy_theaters: list):
    """Parse ``navy_theater={ theater_group={ id name fleet={refs} } ... }``."""
    for k, op in iter_block_entries(tz):
        if op is not LBRACE:
            continue
        if k != "theater_group":
            _skip_block_body(tz)
            continue
        grp = {"owner": tag, "id": None, "name": None, "fleet_refs": []}
        for kk, oo in iter_block_entries(tz):
            if oo is LBRACE:
                if kk == "id":
                    grp["id"] = _idkey(read_id_block(tz))
                elif kk == "fleet":
                    grp["fleet_refs"].append(_idkey(read_id_block(tz)))
                else:
                    _skip_block_body(tz)
                continue
            v = atom_value(oo)
            if v is not None and kk == "name":
                grp["name"] = v
        navy_theaters.append(grp)


# ---------------------------------------------------------------------------
# NAVY: units.fleet -> task_force -> ship
# ---------------------------------------------------------------------------

def _parse_ship(tz: Tokenizer):
    ship = {
        "id": None,
        "definition": None,    # hull/class, e.g. light_cruiser
        "organisation": None,
        "strength": None,
        "experience": None,
        "name": None,          # override name if persisted
        "equipment": [],       # [(id,type,amount)] -> variant index later
    }
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "id":
                ship["id"] = _idkey(read_id_block(tz))
            elif k == "equipment":
                ship["equipment"] = _parse_equipment_list(tz)
            elif k == "ship_name":
                # ship_name carries an override string sometimes.
                for kk, oo in iter_block_entries(tz):
                    if oo is LBRACE:
                        _skip_block_body(tz)
                    elif kk == "override":
                        ship["name"] = atom_value(oo)
            else:
                _skip_block_body(tz)
            continue
        val = atom_value(op)
        if val is None:
            continue
        if k == "definition":
            ship["definition"] = val
        elif k == "organisation":
            ship["organisation"] = to_float(val)
        elif k == "strength":
            ship["strength"] = to_float(val)
        elif k == "experience":
            ship["experience"] = to_float(val)
    return ship


def _parse_task_force_mission(tz: Tokenizer):
    """Parse the persisted ``task_force.mission`` state block.

    This is a block in current a729 saves, not the scalar that the original
    oracle assumed.  The retained-corpus census found the fields below in
    20,212 task forces across 257 saves.  Lists and typed object references are
    kept structurally distinct so a movement/region list cannot be mistaken
    for a stable convoy-route identity.
    """
    mission = {
        "mission": None,
        "is_in_regions": None,
        "hours_in_mission": None,
        "navy_engagement_rule": None,
        "accessible_regions_range": None,
        "accessible_regions": [],
        "accessible_regions_in_fleet": [],
        "extra": {},
        "extra_blocks": [],
    }
    integer_fields = {
        "hours", "hours_in_mission", "hours_in_spotting_region",
        "hours_to_wait_for_base_check", "mission", "navy_engagement_rule",
        "num_convoys_in_regions", "spotting_region", "convoy_spotting_region",
        "strike_force_base", "strike_force_target", "bombardment_region",
        "move",
    }
    float_fields = {
        "accessible_regions_range", "air_superiority", "radar",
        "spotting_process", "spotting_speed",
    }
    bool_fields = {
        "already_spotted", "is_in_regions", "stop_training_at_max_xp",
    }
    integer_list_fields = {"accessible_regions"}
    boolean_list_fields = {"accessible_regions_in_fleet"}
    reference_fields = {
        "group_to_escort", "spotting_convoy_client", "spotting_target",
    }

    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k in integer_list_fields:
                mission[k] = [to_int(x) for x in read_scalar_list(tz)
                              if to_int(x) is not None]
            elif k in boolean_list_fields:
                mission[k] = [x == "yes" for x in read_scalar_list(tz)]
            elif k in reference_fields:
                mission["extra"][k] = _idkey(read_id_block(tz))
            else:
                mission["extra_blocks"].append(k)
                _skip_block_body(tz)
            continue

        val = atom_value(op)
        if val is None:
            continue
        if k in integer_fields:
            parsed = to_int(val)
        elif k in float_fields:
            parsed = to_float(val)
        elif k in bool_fields:
            parsed = (val == "yes")
        else:
            parsed = val

        if k in mission:
            mission[k] = parsed
        else:
            mission["extra"][k] = parsed
    return mission


def _parse_task_force(tz: Tokenizer):
    tf = {
        "id": None,
        "name": None,
        "location_province": None,
        "owner": None,            # logical_country
        "ships": [],
        "mission": None,          # numeric enum from nested mission.mission
        "mission_details": None,  # exact persisted task-force mission block
        "path": [],               # current movement path (province ids)
        "movement_progress": None,
        "repair_mode": None,
        "repair_split": None,
        "repair_target": None,
        "repair_parent": None,
        "repair_child": None,
        "repair_last_mission": None,
        "detached_activity": None,
        "hours_to_wait_for_repair_check": None,
        "hours_waited_for_repairs": None,
        "retreat": None,          # save boolean; not a live retreat-state enum
        "sortie_efficiency": None,
        "underway_replenishment": None,
        "extra_keys": [],
    }
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "id":
                tf["id"] = _idkey(read_id_block(tz))
            elif k == "ship":
                tf["ships"].append(_parse_ship(tz))
            elif k == "mission":
                tf["mission_details"] = _parse_task_force_mission(tz)
                tf["mission"] = tf["mission_details"]["mission"]
            elif k == "path":
                tf["path"] = [to_int(x) for x in read_scalar_list(tz)
                              if to_int(x) is not None]
            elif k in ("repair_parent", "repair_child"):
                tf[k] = _idkey(read_id_block(tz))
            else:
                # officer, AI composition, refit variant, HQ and cosmetic data.
                _skip_block_body(tz)
            continue
        val = atom_value(op)
        if val is None:
            continue
        if k == "name":
            tf["name"] = val
        elif k == "location":
            tf["location_province"] = to_int(val)
        elif k == "logical_country":
            tf["owner"] = val
        elif k == "mission":
            # Legacy saves may persist a scalar.  Current a729 saves use the
            # block handled above; retain scalar compatibility without
            # inventing mission_details.
            tf["mission"] = to_int(val)
        elif k in ("repair_mode", "repair_target", "repair_last_mission",
                   "hours_to_wait_for_repair_check",
                   "hours_waited_for_repairs", "sortie_efficiency"):
            tf[k] = to_int(val)
        elif k == "movement_progress":
            tf[k] = to_float(val)
        elif k in ("repair_split", "retreat", "underway_replenishment"):
            tf[k] = (val == "yes")
        elif k == "detached_activity":
            tf[k] = val
        elif k not in ("previous", "last_combat_date", "move_priority", "seed"):
            if k not in tf["extra_keys"]:
                tf["extra_keys"].append(k)
    return tf


def _parse_fleet(tz: Tokenizer, tag: str, fleets: list):
    fleet = {
        "owner": tag,
        "id": None,
        "name": None,
        "leader": None,           # commanding ADMIRAL (id,type) -> characters
        "icon": None,
        "task_forces": [],
    }
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "id":
                fleet["id"] = _idkey(read_id_block(tz))
            elif k == "leader":
                fleet["leader"] = _idkey(_read_leader(tz))
            elif k == "task_force":
                fleet["task_forces"].append(_parse_task_force(tz))
            else:
                _skip_block_body(tz)
            continue
        val = atom_value(op)
        if val is None:
            continue
        if k == "name":
            fleet["name"] = val
        elif k == "icon":
            fleet["icon"] = to_int(val)
    fleets.append(fleet)


def _parse_equipment_list(tz: Tokenizer):
    """Parse ``equipment={ equipment={ id={id type} amount=N } ... }``.

    Returns ``[(id, type, amount)]``. (Same shape parse_units uses.)
    """
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
                    if v is not None and kk == "amount":
                        amount = to_float(v)
                if eid is not None:
                    out.append((eid, etype, amount))
            else:
                _skip_block_body(tz)
    return out


def _parse_units_block(tz: Tokenizer, tag: str, fleets: list,
                       division_ids: set):
    """Walk a country's ``units={ division={..} ... fleet={..} ... }`` block.

    Records division (id,type) keys (for the member->division join) and parses
    each fleet. Divisions themselves are owned by parse_units; we only need their
    ids here for membership resolution.
    """
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "fleet":
                _parse_fleet(tz, tag, fleets)
            elif k == "division":
                # We only need the division's (id,type) key for the join.
                _id = _type = None
                for kk, oo in iter_block_entries(tz):
                    if oo is LBRACE:
                        if kk == "id" and _id is None:
                            _id, _type = read_id_block(tz)
                        else:
                            _skip_block_body(tz)
                if _id is not None:
                    division_ids.add(f"{_id}:{_type}")
            else:
                _skip_block_body(tz)


# ---------------------------------------------------------------------------
# Countries pass: theatres + navy_theater + units(fleet/division-ids)
# ---------------------------------------------------------------------------

def _parse_countries(tz: Tokenizer, store: dict) -> None:
    """Walk ``countries={ <TAG>={ ... } ... }`` collecting OOB sub-sections."""
    for tag, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        for k, op in iter_block_entries(tz):
            if op is LBRACE:
                if k == "theatres":
                    _parse_theatres(tz, tag, store["armies"],
                                    store["army_groups"], store["theatres"])
                elif k == "navy_theater":
                    _parse_navy_theater(tz, tag, store["navy_theaters"])
                elif k == "units":
                    _parse_units_block(tz, tag, store["fleets"],
                                       store["division_ids"])
                else:
                    _skip_block_body(tz)
            # scalar country fields ignored


# ---------------------------------------------------------------------------
# AIR: strategic_air -> air_base map + <TAG>.air_wing_pool.air_wings
# ---------------------------------------------------------------------------

def _parse_air_base(tz: Tokenizer, airbase_to_state: dict):
    """Parse ``air_base={ id={id type=65} state=<n> ... }`` -> base->state map."""
    base_key = None
    state = None
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "id":
                base_key = _idkey(read_id_block(tz))
            else:
                _skip_block_body(tz)
            continue
        val = atom_value(op)
        if val is not None and k == "state":
            state = to_int(val)
    if base_key is not None:
        airbase_to_state[base_key] = state


def _parse_air_wing(tz: Tokenizer):
    """Parse one ``air_wings={...}`` record = an air WING."""
    wing = {
        "id": None,
        "name": None,
        "owner": None,           # tag= inside the wing
        "count": None,           # number of planes
        "experience": None,
        "manpower": None,
        "priority": None,
        "reinforcement_setting": None,
        "mission": None,         # executing_mission name (cached/runtime)
        "mission_type": None,
        "strategic_region": None,
        "effective_planes": None,  # RUNTIME cached
        "equipment": [],         # [(id,type,amount)] -> plane variant index
    }
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "id":
                wing["id"] = _idkey(read_id_block(tz))
            elif k == "equipment":
                wing["equipment"] = _parse_equipment_list(tz)
            elif k == "mission":
                for kk, oo in iter_block_entries(tz):
                    if oo is LBRACE:
                        _skip_block_body(tz)
                        continue
                    v = atom_value(oo)
                    if v is None:
                        continue
                    if kk == "executing_mission":
                        wing["mission"] = v
                    elif kk == "type":
                        wing["mission_type"] = to_int(v)
                    elif kk == "strategic_region":
                        wing["strategic_region"] = to_int(v)
                    elif kk == "effective_planes_count":
                        wing["effective_planes"] = to_float(v)
            else:
                _skip_block_body(tz)
            continue
        val = atom_value(op)
        if val is None:
            continue
        if k == "name":
            wing["name"] = val
        elif k == "tag":
            wing["owner"] = val
        elif k == "count":
            wing["count"] = to_int(val)
        elif k == "experience":
            wing["experience"] = to_float(val)
        elif k == "manpower":
            wing["manpower"] = to_int(val)
        elif k == "priority":
            wing["priority"] = to_int(val)
        elif k == "reinforcement_setting":
            wing["reinforcement_setting"] = to_int(val)
    return wing


def _parse_air_wing_pool(tz: Tokenizer, tag: str, wings: list):
    """Parse ``air_wing_pool={ id definition air_base={id type=65} air_wings={..} }``.

    A pool groups one plane ``definition`` based at one ``air_base``; it holds
    one or more ``air_wings`` records (the actual wings).
    """
    pool_def = None
    pool_base = None
    pool_wings = []
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "air_base":
                pool_base = _idkey(read_id_block(tz))
            elif k == "air_wings":
                pool_wings.append(_parse_air_wing(tz))
            elif k == "id":
                _skip_block_body(tz)  # pool id (type 4713 handle); not needed
            else:
                _skip_block_body(tz)
            continue
        val = atom_value(op)
        if val is not None and k == "definition":
            pool_def = val
    for w in pool_wings:
        w["plane_definition"] = pool_def
        w["air_base"] = pool_base
        if not w.get("owner"):
            w["owner"] = tag
        wings.append(w)


def _parse_strategic_air(tz: Tokenizer, wings: list, airbase_to_state: dict):
    """Parse top-level ``strategic_air={ air_base={..} <TAG>={ air_wing_pool } }``."""
    for k, op in iter_block_entries(tz):
        if op is not LBRACE:
            continue
        if k == "air_base":
            _parse_air_base(tz, airbase_to_state)
        elif k in ("air_theatre_index", "air_group_index"):
            _skip_block_body(tz)
        elif len(k) <= 4 and k.isupper():
            # A country tag block -> its air_wing_pool list.
            for kk, oo in iter_block_entries(tz):
                if oo is LBRACE:
                    if kk == "air_wing_pool":
                        _parse_air_wing_pool(tz, k, wings)
                    else:
                        _skip_block_body(tz)
        else:
            _skip_block_body(tz)


# ---------------------------------------------------------------------------
# Driver: one streaming top-level pass, then join
# ---------------------------------------------------------------------------

def parse_oob_text(text: str) -> dict:
    store = {
        "armies": [],
        "army_groups": [],
        "theatres": [],
        "navy_theaters": [],
        "fleets": [],
        "division_ids": set(),
        "air_wings": [],
        "airbase_to_state": {},
        "meta": {"date": None, "version": None, "player": None,
                 "player_countries": []},
    }

    def h_countries(tz):
        _parse_countries(tz, store)

    def h_strategic_air(tz):
        _parse_strategic_air(tz, store["air_wings"], store["airbase_to_state"])

    block_handlers = {
        "countries": h_countries,
        "strategic_air": h_strategic_air,
    }
    meta = dc.top_level_pass(
        text, block_handlers,
        scalar_keys=("date", "version", "player"))
    store["meta"].update(meta)
    return store


# ---------------------------------------------------------------------------
# Output assembly + validation
# ---------------------------------------------------------------------------

def _load_province_to_state(script_dir: Path):
    p = (script_dir.parent / "map_pipeline" / "assets"
         / "province_to_state.json")
    if not p.is_file():
        return {}, None
    raw = json.loads(p.read_text(encoding="utf-8"))
    mapping = {}
    for k, v in raw.items():
        try:
            mapping[int(k)] = int(v)
        except (TypeError, ValueError):
            continue
    return mapping, p


def build_oob_outputs(store: dict, prov_to_state: dict,
                      source: Path, was_melted: bool) -> dict:
    armies = store["armies"]
    army_groups = store["army_groups"]
    fleets = store["fleets"]
    wings = store["air_wings"]
    division_ids = store["division_ids"]
    airbase_to_state = store["airbase_to_state"]

    # --- Army membership join: member.unit -> a real division id -------------
    members_total = 0
    members_resolved = 0
    for army in armies:
        resolved = 0
        for m in army["members"]:
            members_total += 1
            if m in division_ids:
                resolved += 1
                members_resolved += 1
        army["members_resolved"] = resolved
        army["member_count"] = len(army["members"])

    # --- Army group -> army ref join -----------------------------------------
    army_id_set = {a["id"] for a in armies if a["id"]}
    ag_refs_total = 0
    ag_refs_resolved = 0
    for ag in army_groups:
        r = 0
        for ref in ag["army_refs"]:
            ag_refs_total += 1
            if ref in army_id_set:
                r += 1
                ag_refs_resolved += 1
        ag["army_refs_resolved"] = r

    # --- Navy theatre fleet-ref join -----------------------------------------
    fleet_id_set = {f["id"] for f in fleets if f["id"]}
    navy_refs_total = 0
    navy_refs_resolved = 0
    for grp in store["navy_theaters"]:
        for ref in grp["fleet_refs"]:
            navy_refs_total += 1
            if ref in fleet_id_set:
                navy_refs_resolved += 1

    # --- Air wing -> air_base -> state join ----------------------------------
    wings_with_base = 0
    wings_base_resolved = 0   # base id present in the air_base->state map
    wings_with_state = 0
    for w in wings:
        base = w.get("air_base")
        if base is not None:
            wings_with_base += 1
            if base in airbase_to_state:
                wings_base_resolved += 1
                st = airbase_to_state[base]
                w["state_id"] = st
                if st is not None:
                    wings_with_state += 1
            else:
                w["state_id"] = None
        else:
            w["state_id"] = None  # carrier/mobile-based: expected variant

    # --- Counts --------------------------------------------------------------
    n_ships = sum(len(tf["ships"])
                  for f in fleets for tf in f["task_forces"])
    n_task_forces = sum(len(f["task_forces"]) for f in fleets)
    n_orders = sum(len(a["orders"]) for a in armies)

    # Per-country rollup.
    by_country: dict = {}

    def _ctry(tag):
        return by_country.setdefault(tag, {
            "armies": 0, "army_groups": 0, "divisions_in_armies": 0,
            "fleets": 0, "task_forces": 0, "ships": 0, "air_wings": 0,
            "planes": 0})

    for a in armies:
        c = _ctry(a["owner"])
        c["armies"] += 1
        c["divisions_in_armies"] += a["member_count"]
    for ag in army_groups:
        _ctry(ag["owner"])["army_groups"] += 1
    for f in fleets:
        c = _ctry(f["owner"])
        c["fleets"] += 1
        c["task_forces"] += len(f["task_forces"])
        c["ships"] += sum(len(tf["ships"]) for tf in f["task_forces"])
    for w in wings:
        c = _ctry(w.get("owner") or "??")
        c["air_wings"] += 1
        c["planes"] += w.get("count") or 0

    # Equipment-variant join coverage is reported as raw presence of an
    # (id,type) key; the variant *names* come from the equipments index, which
    # the equipment/production parsers own. We surface the keys so a downstream
    # join is trivial, and count how many ship/wing equipment refs carry a key.
    ship_eq_refs = sum(len(s["equipment"])
                       for f in fleets for tf in f["task_forces"]
                       for s in tf["ships"])
    wing_eq_refs = sum(len(w["equipment"]) for w in wings)

    meta = {
        "source_filename": source.name,
        "source_path_redacted": ps._redact(source),
        "was_melted": was_melted,
        "save_date": store["meta"].get("date"),
        "save_version": store["meta"].get("version"),
        "player": store["meta"].get("player"),
        "stats": {
            "theatres": len(store["theatres"]),
            "armies": len(armies),
            "army_groups": len(army_groups),
            "battle_plan_orders": n_orders,
            "army_members_total": members_total,
            "army_members_resolved_to_division": members_resolved,
            "member_resolve_pct": round(
                100.0 * members_resolved / members_total, 1)
            if members_total else 0.0,
            "army_group_refs_total": ag_refs_total,
            "army_group_refs_resolved": ag_refs_resolved,
            "fleets": len(fleets),
            "task_forces": n_task_forces,
            "ships": n_ships,
            "ship_equipment_refs": ship_eq_refs,
            "navy_theater_groups": len(store["navy_theaters"]),
            "navy_theater_fleet_refs_total": navy_refs_total,
            "navy_theater_fleet_refs_resolved": navy_refs_resolved,
            "air_wings": len(wings),
            "air_bases_mapped": len(airbase_to_state),
            "air_wing_equipment_refs": wing_eq_refs,
            "wings_with_air_base": wings_with_base,
            "wings_base_resolved": wings_base_resolved,
            "wings_with_state": wings_with_state,
            "distinct_owners": len(by_country),
            "division_ids_indexed": len(division_ids),
        },
        "runtime_only_note": (
            "plan_value/our_power/enemy_power (armies & army groups) and air "
            "mission effectiveness/effective_planes are CACHED runtime snapshots, "
            "surfaced verbatim but NOT authoritative. Live combat targeting, "
            "fronts evaluation, naval/air supremacy & detection, and in-flight "
            "pathfinding are runtime-only and deferred to the memory readers."),
    }

    return {
        "meta": meta,
        "by_country": dict(sorted(by_country.items())),
        "army_groups": army_groups,
        "armies": armies,
        "fleets": fleets,
        "navy_theaters": store["navy_theaters"],
        "air_wings": wings,
        "theatres": store["theatres"],
    }


def validation_report(out: dict) -> str:
    st = out["meta"]["stats"]
    L = []
    L.append("=== ORDER-OF-BATTLE ORACLE VALIDATION ===")
    L.append(f"source        : {out['meta']['source_filename']} "
             f"(melted={out['meta']['was_melted']})")
    L.append(f"save date     : {out['meta']['save_date']}  "
             f"version={out['meta']['save_version']}")
    L.append("-- ARMY --")
    L.append(f"theatres      : {st['theatres']}")
    L.append(f"armies        : {st['armies']}  "
             f"army_groups: {st['army_groups']}")
    L.append(f"battle orders : {st['battle_plan_orders']} order_instances")
    L.append(f"army members  : {st['army_members_total']} unit refs, "
             f"{st['army_members_resolved_to_division']} resolve to a division "
             f"({st['member_resolve_pct']}%)")
    L.append(f"AG->army refs : {st['army_group_refs_resolved']}/"
             f"{st['army_group_refs_total']} resolved")
    L.append("-- NAVY --")
    L.append(f"fleets        : {st['fleets']}  task_forces: {st['task_forces']}"
             f"  ships: {st['ships']}")
    L.append(f"ship equip    : {st['ship_equipment_refs']} (id,type) variant "
             f"refs")
    L.append(f"navy theaters : {st['navy_theater_groups']} groups, fleet refs "
             f"{st['navy_theater_fleet_refs_resolved']}/"
             f"{st['navy_theater_fleet_refs_total']} resolved")
    L.append("-- AIR --")
    L.append(f"air wings     : {st['air_wings']}  "
             f"equip refs: {st['air_wing_equipment_refs']}")
    L.append(f"air bases     : {st['air_bases_mapped']} mapped (base->state)")
    L.append(f"wing basing   : {st['wings_with_air_base']} have an air_base, "
             f"{st['wings_base_resolved']} base resolved, "
             f"{st['wings_with_state']} -> a state "
             f"(rest = carrier/mobile, expected)")
    L.append(f"distinct owners (any branch): {st['distinct_owners']}")

    # Top owners by army count.
    bc = sorted(out["by_country"].items(),
                key=lambda kv: (kv[1]["armies"], kv[1]["ships"]), reverse=True)
    L.append("top owners (armies / divs-in-armies / fleets / ships / wings):")
    for tag, c in bc[:8]:
        L.append(f"   {tag}: {c['armies']}a {c['divisions_in_armies']}d "
                 f"{c['fleets']}f {c['ships']}sh {c['air_wings']}w "
                 f"({c['planes']} planes)")

    # Sample army.
    L.append("sample army:")
    for a in out["armies"][:1]:
        L.append(f"   {a['owner']} {a['name']!r} id={a['id']} "
                 f"leader={a['leader']} members={a['member_count']}"
                 f"(res {a['members_resolved']}) orders={len(a['orders'])} "
                 f"FMgrp={a['field_marshal_group']} cached={a['cached']}")
        for o in a["orders"][:2]:
            L.append(f"      order type={o['type']} id={o['instance_id']} "
                     f"op={o['operation']} states={len(o['states'])} "
                     f"path={len(o['path'])} sched={len(o['scheduled_members'])}"
                     f" extra={o['extra_keys'][:5]}")
    # Sample fleet.
    L.append("sample fleet:")
    for f in out["fleets"][:1]:
        tf0 = f["task_forces"][0] if f["task_forces"] else None
        sh0 = tf0["ships"][0] if (tf0 and tf0["ships"]) else None
        L.append(f"   {f['owner']} {f['name']!r} id={f['id']} "
                 f"admiral={f['leader']} TFs={len(f['task_forces'])}")
        if sh0:
            L.append(f"      tf {tf0['name']!r} loc={tf0['location_province']} "
                     f"ship0 def={sh0['definition']} str={sh0['strength']} "
                     f"eq={len(sh0['equipment'])}")
    # Sample wing.
    L.append("sample air wing:")
    for w in out["air_wings"][:2]:
        L.append(f"   {w.get('owner')} {w['name']!r} id={w['id']} "
                 f"plane={w.get('plane_definition')} count={w['count']} "
                 f"mission={w['mission']} region={w['strategic_region']} "
                 f"base={w.get('air_base')} state={w.get('state_id')} "
                 f"eq={len(w['equipment'])}")
    return "\n".join(L)


# ---------------------------------------------------------------------------
# build_fn for the shared CLI runner
# ---------------------------------------------------------------------------

def _build_fn(text: str, save_path: Path, was_melted: bool):
    store = parse_oob_text(text)
    script_dir = Path(__file__).resolve().parent
    prov_to_state, ptos_path = _load_province_to_state(script_dir)
    if ptos_path is None:
        print("[oob] note: province_to_state.json not found "
              "(theatre area / order states stay province/state ids as-is).")
    out = build_oob_outputs(store, prov_to_state, save_path, was_melted)
    report = validation_report(out)
    # Honour the ASCII-output requirement (names may carry latin1 accents).
    out = _ascii(out)
    return {"oob.json": out}, report


def main(argv=None) -> int:
    return dc.run_cli(
        tag="oob",
        description="the game save -> oob.json "
                    "(army/navy/air command structure + battle plans).",
        build_fn=_build_fn,
        out_files=["oob.json"],
    )


if __name__ == "__main__":
    raise SystemExit(main())
