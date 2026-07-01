#!/usr/bin/env python3
"""the game save -> per-state DETAIL & OCCUPATION oracle (``state_detail.json``).

Deep-layer extension of the save oracle. Per STATE it extracts everything the
``states`` section carries, and folds in the province-level data (forts, naval
bases, victory points) that actually lives in the ``provinces`` section.

Per state (``states.<id>``):
  - ``owner`` / ``controller``
  - ``buildings``      : {infrastructure, industrial_complex, arms_factory,
                         dockyard, air_base, radar_station, anti_air_building,
                         synthetic_refinery, fuel_silo, ...} -> level (state-level
                         buildings).
  - ``province_buildings`` : {bunker, coastal_bunker, naval_base, supply_node,
                         rail_way, ...} -> summed level across the state's
                         provinces (forts / naval base / supply live per-PROVINCE).
  - ``victory_points`` : sum of province victory_points in the state.
  - ``manpower``       : {available, locked, total}.
  - ``resistance``     : {resistance, compliance, resistance_speed,
                         compliance_speed, occupied_country_tag,
                         operational_status} when the state is occupied.
  - ``state_category`` : e.g. "town", "city", "rural".
  - ``demilitarized``  : bool.

Key paths (verified against a real 1.19 melt):
  states.<id>.owner / .controller
  states.<id>.buildings.<bld>.{level,partial_health,healthy_levels}
  states.<id>.manpower_pool.{available,locked,total}
  states.<id>.resistance.{resistance,compliance,resistance_speed,
      compliance_speed,occupied_country_tag,operational_status}
  states.<id>.state_category / .demilitarized
  provinces.<pid>.victory_points
  provinces.<pid>.buildings.<bld>.level        (bunker / naval_base / supply_node)

The province->state join (which province belongs to which state) is loaded from
``map_pipeline/assets/province_to_state.json`` exactly like parse_units /
parse_control. Without it, province-level buildings + VP cannot be attributed to
a state and stay empty (reported in the validation line).

REALITY NOTE — supply: per-state SUPPLY values are computed at runtime from
supply nodes + infrastructure and are not stored as a per-state scalar in the
save. We expose the supply-node levels (province building) as the in-save proxy;
the actual supply throughput is a live-memory figure.

stdlib only. Reuses the shared tokenizer via ``deep_common``.
"""

from __future__ import annotations

import json
from pathlib import Path

import deep_common as dc
from deep_common import (
    LBRACE, atom_value, iter_block_entries, _skip_block_body,
    to_int, to_float,
)


def _read_building_level(tz):
    lvl = 0
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            _skip_block_body(tz)
        elif k == "level":
            lvl = to_int(atom_value(op)) or 0
    return lvl


def _parse_buildings(tz):
    out = {}
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            out[key] = out.get(key, 0) + _read_building_level(tz)
        # bare/scalar building entries ignored
    return out


def _parse_state(tz):
    rec = {
        "owner": None, "controller": None,
        "buildings": {}, "province_buildings": {}, "victory_points": 0,
        "manpower": {"available": None, "locked": None, "total": None},
        "resistance": None, "state_category": None, "demilitarized": None,
    }
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "buildings":
                rec["buildings"] = _parse_buildings(tz)
            elif key == "manpower_pool":
                for k, op in iter_block_entries(tz):
                    if op is LBRACE:
                        _skip_block_body(tz)
                        continue
                    v = atom_value(op)
                    if v is None:
                        continue
                    if k in ("available", "locked", "total"):
                        rec["manpower"][k] = to_int(v)
            elif key == "resistance":
                r = {}
                for k, op in iter_block_entries(tz):
                    if op is LBRACE:
                        _skip_block_body(tz)
                        continue
                    v = atom_value(op)
                    if v is None:
                        continue
                    if k in ("resistance", "compliance", "resistance_speed",
                             "compliance_speed"):
                        r[k] = to_float(v)
                    elif k == "occupied_country_tag":
                        r["occupied_country_tag"] = v
                    elif k == "operational_status":
                        r["operational_status"] = (v == "yes")
                rec["resistance"] = r if r else None
            else:
                _skip_block_body(tz)
            continue
        v = atom_value(opener)
        if v is None:
            continue
        if key == "owner":
            rec["owner"] = v
        elif key == "controller":
            rec["controller"] = v
        elif key == "state_category":
            rec["state_category"] = v
        elif key == "demilitarized":
            rec["demilitarized"] = (v == "yes")
    return rec


def _parse_states(tz, states):
    for sid, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        states[sid] = _parse_state(tz)


def _parse_province(tz):
    rec = {"victory_points": 0, "buildings": {}}
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "buildings":
                rec["buildings"] = _parse_buildings(tz)
            else:
                _skip_block_body(tz)
            continue
        v = atom_value(opener)
        if v is None:
            continue
        if key == "victory_points":
            rec["victory_points"] = to_int(v) or 0
    return rec


def _parse_provinces(tz, provinces):
    for pid, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        rec = _parse_province(tz)
        if rec["victory_points"] or rec["buildings"]:
            provinces[pid] = rec


def parse_state_detail_text(text: str):
    states: dict = {}
    provinces: dict = {}
    meta = dc.top_level_pass(
        text,
        {
            "states": lambda tz: _parse_states(tz, states),
            "provinces": lambda tz: _parse_provinces(tz, provinces),
        },
        scalar_keys=("date", "version", "player"),
    )
    return states, provinces, meta


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


def build(text, source, was_melted):
    states, provinces, meta = parse_state_detail_text(text)

    script_dir = Path(__file__).resolve().parent
    prov_to_state, ptos_path = _load_province_to_state(script_dir)

    # Fold province buildings + VP into their state.
    folded = 0
    for pid_str, prec in provinces.items():
        try:
            pid = int(pid_str)
        except (TypeError, ValueError):
            continue
        sid = prov_to_state.get(pid)
        if sid is None:
            continue
        sid_str = str(sid)
        st = states.get(sid_str)
        if st is None:
            continue
        folded += 1
        st["victory_points"] += prec["victory_points"]
        for b, lvl in prec["buildings"].items():
            st["province_buildings"][b] = st["province_buildings"].get(b, 0) + lvl

    occupied = sum(1 for s in states.values() if s["resistance"])
    out = {
        "meta": {
            "source_filename": source.name,
            "source_path_redacted": dc.ps._redact(source),
            "was_melted": was_melted,
            "save_date": meta.get("date"),
            "save_version": meta.get("version"),
            "player": meta.get("player"),
            "province_to_state_loaded": ptos_path is not None,
            "notes": {
                "supply": "per-state supply is runtime-computed; supply_node "
                "levels (province_buildings) are the in-save proxy.",
                "victory_points": "VP live on provinces; summed into the state.",
                "forts_naval": "forts/naval_base/coastal_bunker are province "
                "buildings, summed into province_buildings.",
            },
            "stats": {
                "states": len(states),
                "states_with_owner": sum(1 for s in states.values() if s["owner"]),
                "occupied_states": occupied,
                "provinces_with_detail": len(provinces),
                "provinces_folded_into_states": folded,
            },
        },
        "states": dict(sorted(states.items(), key=lambda kv: int(kv[0]))),
    }

    lines = ["=== STATE DETAIL ORACLE VALIDATION ==="]
    lines.append(f"source        : {source.name} (melted={was_melted})")
    lines.append(f"save date     : {meta.get('date')}")
    lines.append(f"states        : {len(states)} "
                 f"({out['meta']['stats']['states_with_owner']} owned)")
    lines.append(f"occupied      : {occupied} states have resistance data")
    lines.append(f"prov detail   : {len(provinces)} provinces w/ VP or buildings, "
                 f"{folded} folded (ptos loaded={ptos_path is not None})")
    # spot-check a known industrial state and an occupied one.
    s64 = states.get("64")  # GER core (Berlin area)
    if s64:
        lines.append(f"state 64      : owner={s64['owner']} "
                     f"cat={s64['state_category']} "
                     f"bld={s64['buildings']} "
                     f"vp={s64['victory_points']} "
                     f"provbld={s64['province_buildings']}")
    occ = next((sid for sid, s in states.items() if s["resistance"]), None)
    if occ:
        s = states[occ]
        lines.append(f"occupied {occ}: owner={s['owner']} "
                     f"resist={s['resistance']}")
    return {"state_detail.json": out}, "\n".join(lines)


if __name__ == "__main__":
    raise SystemExit(dc.run_cli(
        "state_detail", "the game save -> per-state detail & occupation oracle.",
        build, ["state_detail.json"]))
