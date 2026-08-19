#!/usr/bin/env python3
"""the game save -> the FULL per-division record (``divisions_full.json``).

The existing ``parse_units.py`` (``divisions.json``) extracts 9 of the ~33
fields a division stores. The completeness audit flagged the rest as STORED but
unparsed — and several as wrongly labelled DERIVED. This module enumerates the
WHOLE division body so the per-division record is complete. It is additive: it
does NOT touch ``parse_units.py`` / ``divisions.json`` (the validated OOB join
pipeline), it produces a sibling ``divisions_full.json``.

Per-division fields extracted (verified against a real 1.19 melt; absent =>
absent, never fabricated):
  identity/owner/location : id(+type), logical_country, location, script_id
  template/name           : division_template_id(+type), division_name{type,
                            name_order}
  state (live, STORED)    : organisation, strength, experience, max_supply,
                            army_current_supply_ratio, supply_gain,
                            motorization_level, move_priority, last_combat_date,
                            dig_in, dig_in_cap
  acclimatization         : {hot_climate, cold_climate, max_acclimatization}
  army_manpower           : value / need (from army_manpower_value/_need.value)
  combat history          : killed, killer_definition
  equipment               : count of distinct equipment archetypes held
  officer                 : has_officer flag (officer/held_officer present)

These are the fields the audit said were mislabelled DERIVED (dig_in,
acclimatization, army_manpower, experience, max_supply, supply_gain): they are
literal save fields and are read here.

stdlib only. Reuses ``parse_save``'s tokenizer (never modifies it).
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import parse_save as ps
from parse_save import (
    Tokenizer, LBRACE, RBRACE, atom_value, iter_block_entries,
    _skip_block_body,
)


def _to_int(v):
    try:
        return int(float(v))
    except (TypeError, ValueError):
        return None


def _to_float(v):
    try:
        return float(v)
    except (TypeError, ValueError):
        return None


def _read_id_block(tz):
    _id = _type = None
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            _skip_block_body(tz)
            continue
        v = atom_value(opener)
        if v is None:
            continue
        if key == "id":
            _id = _to_int(v)
        elif key == "type":
            _type = _to_int(v)
    return _id, _type


def _parse_division_name(tz):
    name_type = name_order = None
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            _skip_block_body(tz)
            continue
        v = atom_value(op)
        if v is None:
            continue
        if k == "type":
            name_type = _to_int(v)
        elif k == "name_order":
            name_order = _to_int(v)
    return {"type": name_type, "name_order": name_order}


def _parse_acclimatization(tz):
    out = {}
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            _skip_block_body(tz)
            continue
        v = atom_value(op)
        if v is None:
            continue
        if k in ("hot_climate", "cold_climate", "max_acclimatization"):
            out[k] = _to_float(v)
    return out


def _parse_army_manpower(tz):
    """army_manpower={ army_manpower_value={ value={ tag value } }
       army_manpower_need={ value={ tag value } } }."""
    res = {"value": None, "need": None}

    def _read_value(tz):
        val = None
        for k, op in iter_block_entries(tz):
            if op is LBRACE:
                if k == "value":
                    for kk, oo in iter_block_entries(tz):
                        if oo is LBRACE:
                            _skip_block_body(tz)
                            continue
                        vv = atom_value(oo)
                        if vv is not None and kk == "value":
                            val = _to_int(vv)
                else:
                    _skip_block_body(tz)
                continue
        return val

    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "army_manpower_value":
                res["value"] = _read_value(tz)
            elif k == "army_manpower_need":
                res["need"] = _read_value(tz)
            else:
                _skip_block_body(tz)
    return res


def _parse_equipment_count(tz):
    """equipment={ equipment={ id amount } ... } -> distinct archetype count."""
    n = 0
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "equipment":
                n += 1
                _skip_block_body(tz)
            else:
                _skip_block_body(tz)
    return n


def _parse_division_block(tz):
    d = {
        "div_id": None, "div_type": None, "owner": None,
        "location_province": None, "script_id": None,
        "template_id": None, "template_type": None,
        "division_name": None,
        "organisation": None, "strength": None, "experience": None,
        "max_supply": None, "army_current_supply_ratio": None,
        "supply_gain": None, "motorization_level": None,
        "move_priority": None, "last_combat_date": None,
        "dig_in": None, "dig_in_cap": None,
        "acclimatization": None, "army_manpower": None,
        "killed": None, "killer_definition": None,
        "equipment_archetypes": 0, "has_officer": False,
    }
    first_id_done = False
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "id":
                _id, _type = _read_id_block(tz)
                if not first_id_done:
                    d["div_id"] = _id
                    d["div_type"] = _type
                    first_id_done = True
            elif k == "division_template_id":
                tid, ttype = _read_id_block(tz)
                d["template_id"] = tid
                d["template_type"] = ttype
            elif k == "division_name":
                d["division_name"] = _parse_division_name(tz)
            elif k == "acclimatization":
                d["acclimatization"] = _parse_acclimatization(tz)
            elif k == "army_manpower":
                d["army_manpower"] = _parse_army_manpower(tz)
            elif k == "equipment":
                d["equipment_archetypes"] = _parse_equipment_count(tz)
            elif k in ("officer", "held_officer"):
                d["has_officer"] = True
                _skip_block_body(tz)
            else:
                _skip_block_body(tz)
            continue
        v = atom_value(op)
        if v is None:
            continue
        if k == "logical_country":
            d["owner"] = v
        elif k == "location":
            d["location_province"] = _to_int(v)
        elif k == "script_id":
            d["script_id"] = _to_int(v)
        elif k == "organisation":
            d["organisation"] = _to_float(v)
        elif k == "strength":
            d["strength"] = _to_float(v)
        elif k == "experience":
            d["experience"] = _to_float(v)
        elif k == "max_supply":
            d["max_supply"] = _to_float(v)
        elif k == "army_current_supply_ratio":
            d["army_current_supply_ratio"] = _to_float(v)
        elif k == "supply_gain":
            d["supply_gain"] = _to_float(v)
        elif k == "motorization_level":
            d["motorization_level"] = _to_int(v)
        elif k == "move_priority":
            d["move_priority"] = v
        elif k == "last_combat_date":
            d["last_combat_date"] = v
        elif k == "dig_in":
            d["dig_in"] = _to_float(v)
        elif k == "dig_in_cap":
            d["dig_in_cap"] = _to_float(v)
        elif k == "killed":
            d["killed"] = _to_int(v)
        elif k == "killer_definition":
            d["killer_definition"] = v
    return d


def _parse_units_block(tz, out):
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "division":
                d = _parse_division_block(tz)
                if d.get("owner"):
                    out.append(d)
            else:
                _skip_block_body(tz)


def _parse_countries(tz, out):
    for tag, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        for k, op in iter_block_entries(tz):
            if op is LBRACE:
                if k == "units":
                    _parse_units_block(tz, out)
                else:
                    _skip_block_body(tz)


def parse_text(text):
    tz = Tokenizer(text)
    divisions = []
    meta = {"date": None, "version": None, "player": None}
    while True:
        tok = tz.next()
        if tok is None:
            break
        if tok is RBRACE:
            continue
        if tok is LBRACE:
            _skip_block_body(tz)
            continue
        key = atom_value(tok)
        if key is None:
            continue
        if tz.peek() is not ps.EQUALS:
            continue
        tz.next()
        val = tz.next()
        if val is LBRACE:
            if key == "countries":
                _parse_countries(tz, divisions)
            else:
                _skip_block_body(tz)
        else:
            sval = atom_value(val)
            if sval is not None and key in meta:
                meta[key] = sval
    return divisions, meta


DIVISION_FIELD_SET = [
    "div_id", "div_type", "owner", "location_province", "script_id",
    "template_id", "template_type", "division_name",
    "organisation", "strength", "experience", "max_supply",
    "army_current_supply_ratio", "supply_gain", "motorization_level",
    "move_priority", "last_combat_date", "dig_in", "dig_in_cap",
    "acclimatization", "army_manpower", "killed", "killer_definition",
    "equipment_archetypes", "has_officer",
]


def build(text, source, was_melted):
    divisions, meta = parse_text(text)
    n = len(divisions)

    presence = {f: 0 for f in DIVISION_FIELD_SET}
    for d in divisions:
        for f in DIVISION_FIELD_SET:
            val = d.get(f)
            if val not in (None, False, 0, "", []):
                presence[f] += 1

    by_owner = {}
    for d in divisions:
        by_owner[d["owner"]] = by_owner.get(d["owner"], 0) + 1

    out = {
        "meta": {
            "source_filename": source.name,
            "source_path_redacted": ps._redact(source),
            "was_melted": was_melted,
            "save_date": meta.get("date"),
            "save_version": meta.get("version"),
            "player": meta.get("player"),
            "notes": {
                "scope": "FULL per-division body (additive to divisions.json). "
                "dig_in/acclimatization/army_manpower/experience/max_supply/"
                "supply_gain are LITERAL save fields (audit had them mislabelled "
                "DERIVED).",
                "optional_fields": "move/fuel/bonus/previous/requests vary "
                "per-division; absent => the division did not store it.",
            },
            "stats": {
                "divisions": n,
                "distinct_owners": len(by_owner),
                "field_presence": presence,
            },
        },
        "by_owner": dict(sorted(by_owner.items(), key=lambda kv: -kv[1])),
        "divisions": divisions,
    }

    lines = ["=== FULL DIVISION RECORD ORACLE VALIDATION ==="]
    lines.append(f"source        : {source.name} (melted={was_melted})")
    lines.append(f"save date     : {meta.get('date')}")
    lines.append(f"divisions     : {n}  (expect ~1654; matches divisions.json)")
    lines.append("field presence (non-empty / total):")
    for f in DIVISION_FIELD_SET:
        pct = round(100.0 * presence[f] / n, 1) if n else 0.0
        lines.append(f"   {f:<28} {presence[f]:>5} ({pct}%)")
    sample = next((d for d in divisions if d.get("dig_in") is not None
                   and d.get("army_manpower")), divisions[0] if divisions else None)
    if sample:
        lines.append("sample division (combat-active):")
        lines.append(f"   #{sample['div_id']} owner={sample['owner']} "
                     f"org={sample['organisation']} str={sample['strength']} "
                     f"exp={sample['experience']} max_supply={sample['max_supply']}")
        lines.append(f"   dig_in={sample['dig_in']}/{sample['dig_in_cap']} "
                     f"acclim={sample['acclimatization']} "
                     f"supply_ratio={sample['army_current_supply_ratio']} "
                     f"supply_gain={sample['supply_gain']}")
        lines.append(f"   army_manpower={sample['army_manpower']} "
                     f"killed={sample['killed']} "
                     f"last_combat={sample['last_combat_date']}")
    return {"divisions_full.json": out}, "\n".join(lines)


def main(argv=None):
    ap = argparse.ArgumentParser(
        description="the game save -> full per-division record (divisions_full.json).")
    ap.add_argument("save", nargs="?", default=None)
    ap.add_argument("--rakaly", default=None)
    ap.add_argument("--out-dir", default=None)
    args = ap.parse_args(argv)

    script_dir = Path(__file__).resolve().parent
    out_dir = Path(args.out_dir) if args.out_dir else script_dir / "output"
    out_dir.mkdir(parents=True, exist_ok=True)

    if args.save:
        save_path = Path(args.save)
        if not save_path.exists():
            print(f"ERROR: save not found: {save_path}", file=sys.stderr)
            return 2
    else:
        save_path = ps.find_latest_save()
        if save_path is None:
            print("ERROR: no candidate save found.", file=sys.stderr)
            return 2

    print(f"[divfull] resolved save : {ps._scrub(str(save_path))}")
    rakaly = ps.find_rakaly(args.rakaly)
    print(f"[divfull] rakaly        : "
          f"{ps._scrub(str(rakaly)) if rakaly else 'NOT FOUND'}")
    try:
        text, was_melted = ps.melt_if_needed(save_path, rakaly, out_dir)
    except RuntimeError as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        return 3
    print(f"[divfull] melted        : {was_melted}  ({len(text):,} chars)")

    outputs, report = build(text, save_path, was_melted)
    for fname, obj in outputs.items():
        (out_dir / fname).write_text(
            json.dumps(obj, indent=1, ensure_ascii=False), encoding="utf-8")
    print()
    print(report)
    print()
    for fname in outputs:
        print(f"[divfull] wrote: {ps._scrub(str(out_dir / fname))}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
