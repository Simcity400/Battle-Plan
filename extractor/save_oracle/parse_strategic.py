#!/usr/bin/env python3
"""the game save -> world-scoped STRATEGIC layers (3 JSONs).

Closes three top-level domains the older oracle never touched:

  ``strategic_air.json``        -- the global strategic-air structure
      (top-level ``strategic_air``). Distinct from per-country air OOB
      (``oob.json``). Holds the air-theatre / air-group index counters and the
      world ``air_base`` table: per base { base id, state, capacity, base/level,
      and the list of countries using it with their capacity_penalty }. Per
      country, the block also carries strategic-region arrays
      (naval_strike_remaining, etc.) — summarized as a presence count only
      (high-cardinality region vectors, low replay value).

  ``strategic_operatives.json`` -- the WORLD operative pool
      (top-level ``strategic_operatives.strategic_operative``, index-keyed).
      Per index: the subversive-activity / danger PID-controller state and, when
      a network exists, the ``intel_network.target`` country + vertices/edges
      sizes. Separate from the per-country agency in ``intel.json``.

  ``raids.json``                -- naval/air RAID target tracking
      (top-level ``raids``). ``targets`` = the global raid-target table
      ({existing_target_num, detectable_target_num, valid, dynamic, and the
      target province OR building}) + the index counters; ``countries`` = the
      per-country raid priority list.

stdlib only. Reuses the shared tokenizer via ``deep_common``.
"""

from __future__ import annotations

import deep_common as dc
from deep_common import (
    LBRACE, RBRACE, EQUALS, atom_value, iter_block_entries, _skip_block_body,
    read_id_block, read_scalar_list, to_int, to_float,
)



def _parse_air_base(tz):
    rec = {"id": None, "type": None, "state": None, "capacity": None,
           "base": None, "level": None, "countries": []}
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "id":
                rec["id"], rec["type"] = read_id_block(tz)
            elif key == "countries":
                tag = None
                pen = None
                for k, op in iter_block_entries(tz):
                    if op is LBRACE:
                        _skip_block_body(tz)
                        continue
                    v = atom_value(op)
                    if v is None:
                        continue
                    if k == "country":
                        tag = v
                    elif k == "capacity_penalty":
                        pen = to_float(v)
                if tag is not None:
                    rec["countries"].append(
                        {"country": tag, "capacity_penalty": pen})
            else:
                _skip_block_body(tz)
            continue
        v = atom_value(opener)
        if v is None:
            continue
        if key == "state":
            rec["state"] = to_int(v)
        elif key == "capacity":
            rec["capacity"] = to_int(v)
        elif key == "base":
            rec["base"] = to_int(v)
        elif key == "level":
            rec["level"] = to_int(v)
    return rec


def _parse_strategic_air(tz, result):
    air_bases = []
    per_country = {}
    indices = {}
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "air_base":
                air_bases.append(_parse_air_base(tz))
            elif key in ("air_theatre_index", "air_group_index"):
                v = None
                for k, op in iter_block_entries(tz):
                    if op is LBRACE:
                        _skip_block_body(tz)
                    elif k == "id":
                        v = to_int(atom_value(op))
                indices[key] = v
            elif (len(key) == 3 and key.isupper()) or (
                    len(key) == 3 and key[0] == "D" and key[1:].isdigit()):
                subkeys = 0
                for k, op in iter_block_entries(tz):
                    if op is LBRACE:
                        _skip_block_body(tz)
                    subkeys += 1
                per_country[key] = subkeys
            else:
                _skip_block_body(tz)
            continue
    result["air_bases"] = air_bases
    result["per_country_region_blocks"] = per_country
    result["indices"] = indices



def _parse_pid(tz):
    val = None
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            _skip_block_body(tz)
            continue
        if k == "value":
            val = to_float(atom_value(op))
    return val


def _parse_operative_entry(tz):
    rec = {"subversive_activity_level": None, "danger_level": None,
           "networks": []}
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "subversive_activity_level":
                rec["subversive_activity_level"] = _parse_pid(tz)
            elif key == "danger_level":
                rec["danger_level"] = _parse_pid(tz)
            elif key == "intel_network":
                target = None
                vertices = None
                for k, op in iter_block_entries(tz):
                    if op is LBRACE:
                        if k == "intel_network":
                            for kk, oo in iter_block_entries(tz):
                                if oo is LBRACE:
                                    _skip_block_body(tz)
                                elif kk == "vertices":
                                    vertices = to_int(atom_value(oo))
                        else:
                            _skip_block_body(tz)
                        continue
                    v = atom_value(op)
                    if v is not None and k == "target":
                        target = v
                if target is not None:
                    rec["networks"].append(
                        {"target": target, "vertices": vertices})
            else:
                _skip_block_body(tz)
            continue
    return rec


def _parse_strategic_operatives(tz, result):
    entries = {}
    for key, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        if key != "strategic_operative":
            _skip_block_body(tz)
            continue
        for idx, op in iter_block_entries(tz):
            if op is not LBRACE:
                continue
            entries[idx] = _parse_operative_entry(tz)
    result["operatives"] = entries



def _parse_raid_target(tz):
    rec = {"existing_target_num": None, "detectable_target_num": None,
           "valid": None, "dynamic": None,
           "target_province": None, "target_building": None,
           "target_location": None}
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "target":
                for k, op in iter_block_entries(tz):
                    if op is LBRACE:
                        if k == "building":
                            for kk, oo in iter_block_entries(tz):
                                if oo is LBRACE:
                                    _skip_block_body(tz)
                                    continue
                                vv = atom_value(oo)
                                if vv is None:
                                    continue
                                if kk == "template":
                                    rec["target_building"] = vv
                                elif kk == "location":
                                    rec["target_location"] = to_int(vv)
                        else:
                            _skip_block_body(tz)
                        continue
                    v = atom_value(op)
                    if v is not None and k == "province":
                        rec["target_province"] = to_int(v)
            else:
                _skip_block_body(tz)
            continue
        v = atom_value(opener)
        if v is None:
            continue
        if key == "existing_target_num":
            rec["existing_target_num"] = to_int(v)
        elif key == "detectable_target_num":
            rec["detectable_target_num"] = to_int(v)
        elif key == "valid":
            rec["valid"] = (v == "yes")
        elif key == "dynamic":
            rec["dynamic"] = (v == "yes")
    return rec


def _parse_raids(tz, result):
    targets = []
    counters = {}
    countries = []
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "targets":
                for k, op in iter_block_entries(tz):
                    if op is LBRACE:
                        if k == "target":
                            targets.append(_parse_raid_target(tz))
                        else:
                            _skip_block_body(tz)
                        continue
                    v = atom_value(op)
                    if v is None:
                        continue
                    if k in ("next_state", "existing_target_num",
                             "detectable_target_num", "next_target"):
                        counters[k] = to_int(v)
            elif key == "countries":
                countries = _parse_raid_countries(tz)
            else:
                _skip_block_body(tz)
            continue
    result["targets"] = targets
    result["counters"] = counters
    result["countries"] = countries


def _parse_raid_countries(tz):
    """countries={ { tag="GER" priority=1 dummy={..} } ... } — anon blocks."""
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
            tag = None
            prio = None
            for k, op in iter_block_entries(tz):
                if op is LBRACE:
                    _skip_block_body(tz)
                    continue
                v = atom_value(op)
                if v is None:
                    continue
                if k == "tag":
                    tag = v
                elif k == "priority":
                    prio = to_int(v)
            if tag is not None:
                out.append({"tag": tag, "priority": prio})
    return out



def build(text, source, was_melted):
    sa = {}
    so = {}
    rd = {}
    meta = dc.top_level_pass(
        text,
        {
            "strategic_air": lambda tz: _parse_strategic_air(tz, sa),
            "strategic_operatives": lambda tz: _parse_strategic_operatives(tz, so),
            "raids": lambda tz: _parse_raids(tz, rd),
        },
        scalar_keys=("date", "version", "player"),
    )

    base_meta = {
        "source_filename": source.name,
        "source_path_redacted": dc.ps._redact(source),
        "was_melted": was_melted,
        "save_date": meta.get("date"),
        "save_version": meta.get("version"),
        "player": meta.get("player"),
    }

    air_bases = sa.get("air_bases", [])
    air_out = {
        "meta": {**base_meta, "stats": {
            "air_bases": len(air_bases),
            "per_country_region_blocks": len(
                sa.get("per_country_region_blocks", {})),
            "indices": sa.get("indices", {}),
        }},
        "air_bases": air_bases,
        "indices": sa.get("indices", {}),
        "per_country_region_blocks": sa.get("per_country_region_blocks", {}),
    }

    op_entries = so.get("operatives", {})
    with_net = {k: v for k, v in op_entries.items() if v.get("networks")}
    total_networks = sum(len(v.get("networks", []))
                         for v in op_entries.values())
    op_out = {
        "meta": {**base_meta, "stats": {
            "operative_entries": len(op_entries),
            "entries_with_network": len(with_net),
            "total_networks": total_networks,
        }},
        "operatives": op_entries,
    }

    targets = rd.get("targets", [])
    raids_out = {
        "meta": {**base_meta, "stats": {
            "raid_targets": len(targets),
            "dynamic_targets": sum(1 for t in targets if t.get("dynamic")),
            "raid_countries": len(rd.get("countries", [])),
            "counters": rd.get("counters", {}),
        }},
        "counters": rd.get("counters", {}),
        "targets": targets,
        "countries": rd.get("countries", []),
    }

    lines = ["=== STRATEGIC (air / operatives / raids) ORACLE VALIDATION ==="]
    lines.append(f"source        : {source.name} (melted={was_melted})")
    lines.append(f"save date     : {meta.get('date')}")
    lines.append(f"air_bases     : {len(air_bases)}  "
                 f"indices={sa.get('indices')}  "
                 f"per-country region blocks={len(sa.get('per_country_region_blocks', {}))}")
    if air_bases:
        ab0 = air_bases[0]
        lines.append(f"  air_base[0] : id={ab0['id']} state={ab0['state']} "
                     f"cap={ab0['capacity']} lvl={ab0['level']} "
                     f"users={len(ab0['countries'])}")
    lines.append(f"operatives    : {len(op_entries)} pool entries, "
                 f"{len(with_net)} with an intel network "
                 f"({total_networks} networks total)")
    if with_net:
        k0 = next(iter(with_net))
        nets = with_net[k0]["networks"]
        lines.append(f"  op[{k0}] networks={[(n['target'], n['vertices']) for n in nets]}")
    lines.append(f"raid targets  : {len(targets)} "
                 f"({sum(1 for t in targets if t.get('dynamic'))} dynamic), "
                 f"raid countries={len(rd.get('countries', []))}")
    lines.append(f"  counters    : {rd.get('counters')}")
    if targets:
        t0 = next((t for t in targets if t.get("target_province")), targets[0])
        lines.append(f"  target sample: prov={t0.get('target_province')} "
                     f"bld={t0.get('target_building')} valid={t0.get('valid')}")

    return {
        "strategic_air.json": air_out,
        "strategic_operatives.json": op_out,
        "raids.json": raids_out,
    }, "\n".join(lines)


if __name__ == "__main__":
    raise SystemExit(dc.run_cli(
        "strategic",
        "the game save -> strategic_air / strategic_operatives / raids oracle.",
        build,
        ["strategic_air.json", "strategic_operatives.json", "raids.json"]))
